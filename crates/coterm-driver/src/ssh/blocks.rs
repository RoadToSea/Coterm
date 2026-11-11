//! Driver-owned, in-memory command previews. Raw terminal keys are never recorded here.
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};

use crate::ssh::output::{encode_output, text_prefix};
use anyhow::{anyhow, bail, Result};
use coterm_core::driver::DriverNotification;
use coterm_core::model::operation::OperationSource;
use serde::Serialize;
use serde_json::{json, Value};

const MAX_BLOCKS: usize = 32;
const COMMAND_BYTES: usize = 1024;
const OUTPUT_BYTES: usize = 2048;
const PAGE_BYTES: usize = 12 * 1024;
const SOURCE_BYTES: usize = 64 * 1024;
const STREAM_BYTES: usize = 64 * 1024;
const RECORD_BYTES: usize = 96 * 1024;
const STREAM_PREVIEW_BYTES: usize = 32 * 1024;

#[derive(Clone)]
pub(crate) struct BlockSpec {
    pub text: String,
    pub source: OperationSource,
    pub observed: bool,
}

impl BlockSpec {
    pub fn new(text: String, mcp: bool) -> Self {
        Self {
            text,
            observed: false,
            source: if mcp {
                OperationSource::Agent
            } else {
                OperationSource::Human
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BlockState {
    Running,
    Completed,
    Cancelled,
    TimedOut,
    Unknown,
}

struct Entry {
    id: String,
    index: u32,
    command: String,
    actor: &'static str,
    channel: &'static str,
    state: BlockState,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    command_truncated: bool,
    output_truncated: bool,
    stdout_truncated: bool,
    stderr_truncated: bool,
    observed: bool,
    separated: bool,
    error: Option<Value>,
    started: std::time::Instant,
    duration_ms: Option<u64>,
}

impl Entry {
    fn value(&self) -> Value {
        let command = text_prefix(&self.command, COMMAND_BYTES);
        let stdout = &self.stdout[..self.stdout.len().min(OUTPUT_BYTES)];
        let stderr = &self.stderr[..self.stderr.len().min(OUTPUT_BYTES - stdout.len())];
        json!({
            "id": self.id, "index": self.index, "command": command,
            "actor": self.actor, "channel": self.channel, "state": self.state,
            "exitCode": self.exit_code, "output": plain_text(stdout),
            "stderr": plain_text(stderr), "commandTruncated": self.command_truncated || command.len() < self.command.len(),
            "outputTruncated": self.output_truncated || stdout.len() < self.stdout.len() || stderr.len() < self.stderr.len(),
            "boundary": if self.observed { "observed" } else { "verified" },
        })
    }

    fn record(&self, budget: usize) -> Value {
        let mut limit = STREAM_PREVIEW_BYTES;
        loop {
            let input = text_prefix(&self.command, limit);
            let stdout = &self.stdout[..self.stdout.len().min(limit)];
            let stderr = &self.stderr[..self.stderr.len().min(limit)];
            let (out_encoding, _, out) = encode_output(stdout, false);
            let (err_encoding, _, err) = encode_output(stderr, false);
            let truncated = self.output_truncated
                || self.command_truncated
                || input.len() < self.command.len()
                || stdout.len() < self.stdout.len()
                || stderr.len() < self.stderr.len();
            let value = json!({
                "instanceId": null, "operationId": self.id, "index": self.index,
                "mode": if self.channel == "exec" { "exec" } else { "terminal" },
                "status": self.state, "stdin": input, "stdout": out, "stderr": err,
                "stdoutEncoding": out_encoding, "stderrEncoding": err_encoding,
                "streamMode": if self.separated { "separated" } else { "merged" },
                "exitCode": self.exit_code, "truncated": truncated, "error": self.error,
                "stdinTruncated": self.command_truncated || input.len() < self.command.len(),
                "stdoutTruncated": self.stdout_truncated || stdout.len() < self.stdout.len(),
                "stderrTruncated": self.stderr_truncated || stderr.len() < self.stderr.len(),
                "actor": self.actor, "boundary": if self.observed { "observed" } else { "verified" },
                "durationMs": self.duration_ms.unwrap_or_else(|| self.started.elapsed().as_millis() as u64),
            });
            if serde_json::to_vec(&value).is_ok_and(|bytes| bytes.len() <= budget) || limit == 0 {
                return value;
            }
            limit /= 2;
        }
    }
}

/// Logical LF-delimited text, independent of xterm wrapping and cursor movement.
pub(crate) fn line_snapshot(bytes: &[u8], count: usize) -> Value {
    let text = plain_text(bytes);
    let mut lines: Vec<_> = text.lines().rev().take(count).collect();
    lines.reverse();
    let mut truncated = false;
    while serde_json::to_vec(&lines).is_ok_and(|value| value.len() > RECORD_BYTES / 2) {
        truncated = true;
        if lines.len() > 1 {
            lines.remove(0);
        } else if let Some(line) = lines.first_mut() {
            let mut start = line.len() / 2;
            while !line.is_char_boundary(start) {
                start += 1;
            }
            *line = &line[start..];
        } else {
            break;
        }
    }
    json!({"lines": lines, "truncated": truncated})
}

struct Inner {
    epoch: String,
    revision: u64,
    next_index: u32,
    dropped: u32,
    dirty: bool,
    entries: VecDeque<Entry>,
    selected: HashSet<String>,
}

#[derive(Clone)]
pub(crate) struct BlockLog(Arc<Mutex<Inner>>);

impl Default for BlockLog {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Inner {
            epoch: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            next_index: 0,
            dropped: 0,
            dirty: false,
            entries: VecDeque::new(),
            selected: HashSet::new(),
        })))
    }
}

impl BlockLog {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        match self.0.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn begin(&self, id: &str, spec: BlockSpec, channel: &'static str) -> Result<BlockHandle> {
        if id.is_empty() || id.len() > 128 {
            bail!("Invalid SSH block operation ID");
        }
        let mut inner = self.lock();
        if inner.entries.iter().any(|entry| entry.id == id) {
            bail!("Duplicate SSH block operation");
        }
        let index = inner
            .next_index
            .checked_add(1)
            .ok_or_else(|| anyhow!("SSH block numbering exhausted"))?;
        if inner.entries.len() == MAX_BLOCKS {
            let position = inner
                .entries
                .iter()
                .position(|entry| entry.state != BlockState::Running)
                .ok_or_else(|| anyhow!("Too many active SSH message blocks"))?;
            if let Some(expired) = inner.entries.remove(position) {
                inner.selected.remove(&expired.id);
            }
            inner.dropped = inner.dropped.saturating_add(1);
        }
        let mut end = spec.text.len().min(SOURCE_BYTES);
        while !spec.text.is_char_boundary(end) {
            end -= 1;
        }
        inner.entries.push_back(Entry {
            id: id.to_owned(),
            index,
            command: spec.text[..end].to_owned(),
            actor: if spec.source == OperationSource::Human {
                "human"
            } else {
                "robot"
            },
            channel,
            state: BlockState::Running,
            exit_code: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            command_truncated: end < spec.text.len(),
            output_truncated: false,
            stdout_truncated: false,
            stderr_truncated: false,
            observed: spec.observed,
            separated: channel == "exec",
            error: None,
            started: std::time::Instant::now(),
            duration_ms: None,
        });
        inner.next_index = index;
        inner.revision = inner.revision.saturating_add(1);
        inner.dirty = true;
        Ok(BlockHandle {
            log: self.clone(),
            id: id.to_owned(),
        })
    }

    /// Newest-first pages fit the host's 16 KiB operation-result budget, including escaping.
    pub fn snapshot(&self, before: Option<u32>) -> Result<Value> {
        self.read_snapshot(before, false)
    }

    pub fn selected_snapshot(&self, before: Option<u32>) -> Result<Value> {
        self.read_snapshot(before, true)
    }

    pub fn record(&self, operation_id: &str) -> Result<Value> {
        self.lock()
            .entries
            .iter()
            .find(|entry| entry.id == operation_id)
            .map(|entry| entry.record(RECORD_BYTES))
            .ok_or_else(|| anyhow!("SSH command record is unavailable or expired"))
    }

    pub fn dialogue_pairs(&self, count: usize) -> Value {
        let count = count.clamp(1, MAX_BLOCKS);
        let inner = self.lock();
        let budget = (RECORD_BYTES - 8 * 1024) / count;
        let records: Vec<_> = inner
            .entries
            .iter()
            .rev()
            .take(count)
            .map(|entry| entry.record(budget))
            .collect();
        json!({"pairs": records, "dropped": inner.dropped,
            "truncated": inner.dropped > 0 || records.iter().any(|record| record["truncated"] == true)})
    }

    pub fn read_lines(&self, operation_id: &str, count: usize) -> Result<Value> {
        let inner = self.lock();
        let entry = inner
            .entries
            .iter()
            .find(|entry| entry.id == operation_id)
            .ok_or_else(|| anyhow!("SSH command record is unavailable or expired"))?;
        let mut value = line_snapshot(&entry.stdout, count);
        let stderr = line_snapshot(&entry.stderr, count);
        value["stderrLines"] = stderr["lines"].clone();
        value["truncated"] = json!(
            entry.output_truncated || value["truncated"] == true || stderr["truncated"] == true
        );
        value["operationId"] = json!(operation_id);
        value["status"] = json!(entry.state);
        value["streamMode"] = json!(if entry.separated {
            "separated"
        } else {
            "merged"
        });
        value["exitCode"] = json!(entry.exit_code);
        Ok(value)
    }

    /// Only the human frontend may call this through the backend's source gate.
    pub fn select(&self, epoch: &str, ids: Vec<String>) -> Result<Value> {
        let mut inner = self.lock();
        if epoch != inner.epoch || ids.len() > MAX_BLOCKS {
            bail!("SSH block selection is stale or exceeds its bound");
        }
        let selected: HashSet<_> = ids.into_iter().collect();
        if selected
            .iter()
            .any(|id| !inner.entries.iter().any(|entry| &entry.id == id))
        {
            bail!("Selected SSH block is unavailable");
        }
        if inner.selected != selected {
            inner.selected = selected;
            inner.revision = inner.revision.saturating_add(1);
            inner.dirty = true;
        }
        Ok(json!({"epoch": inner.epoch, "selectedCount": inner.selected.len()}))
    }

    fn read_snapshot(&self, before: Option<u32>, selected_only: bool) -> Result<Value> {
        let inner = self.lock();
        let mut result = json!({"epoch": inner.epoch, "revision": inner.revision.to_string(),
            "dropped": inner.dropped, "blocks": [], "more": false, "nextBefore": null,
            "selectedCount": inner.selected.len()});
        let mut blocks = Vec::new();
        let mut next_before = None;
        for entry in inner
            .entries
            .iter()
            .rev()
            .filter(|entry| before.is_none_or(|index| entry.index < index))
            .filter(|entry| !selected_only || inner.selected.contains(&entry.id))
        {
            blocks.push(entry.value());
            result["blocks"] = json!(blocks);
            if serde_json::to_vec(&result)?.len() > PAGE_BYTES - 128 {
                blocks.pop();
                result["blocks"] = json!(blocks);
                result["more"] = json!(true);
                result["nextBefore"] = json!(next_before);
                break;
            }
            next_before = Some(entry.index);
        }
        Ok(result)
    }

    /// Coalesce high-frequency output into a lightweight invalidation on the runtime tick.
    /// A full queue retains the dirty flag so the next tick retries without blocking IO.
    pub fn notify(&self, instance: &str, sender: &tokio::sync::mpsc::Sender<DriverNotification>) {
        let mut inner = self.lock();
        if !inner.dirty {
            return;
        }
        let event = DriverNotification::Event {
            driver_instance_id: instance.to_owned(),
            name: "ssh.blocks.changed".to_owned(),
            payload: json!({"epoch": inner.epoch, "revision": inner.revision.to_string()}),
        };
        if sender.try_send(event).is_ok() {
            inner.dirty = false;
        }
    }
}

pub(crate) struct BlockHandle {
    log: BlockLog,
    id: String,
}

impl BlockHandle {
    pub fn annotation(&self, kind: &str) -> Option<Value> {
        let inner = self.log.lock();
        let entry = inner.entries.iter().find(|entry| entry.id == self.id)?;
        Some(
            json!({"kind": kind, "id": entry.id, "index": entry.index, "actor": entry.actor,
            "boundary": if entry.observed { "observed" } else { "verified" }}),
        )
    }

    pub fn append(&self, bytes: &[u8], stderr: bool) {
        if bytes.is_empty() {
            return;
        }
        let mut inner = self.log.lock();
        let Some(entry) = inner.entries.iter_mut().find(|entry| entry.id == self.id) else {
            return;
        };
        if entry.state != BlockState::Running {
            return;
        }
        let was_truncated = entry.output_truncated;
        if stderr {
            entry.separated = true;
        }
        let target = if stderr {
            &mut entry.stderr
        } else {
            &mut entry.stdout
        };
        let keep = bytes.len().min(STREAM_BYTES);
        let drop = target
            .len()
            .saturating_add(keep)
            .saturating_sub(STREAM_BYTES);
        target.drain(..drop);
        target.extend_from_slice(&bytes[bytes.len() - keep..]);
        entry.output_truncated |= drop > 0 || keep < bytes.len();
        if stderr {
            entry.stderr_truncated |= drop > 0 || keep < bytes.len();
        } else {
            entry.stdout_truncated |= drop > 0 || keep < bytes.len();
        }
        if keep > 0 || was_truncated != entry.output_truncated {
            inner.revision = inner.revision.saturating_add(1);
            inner.dirty = true;
        }
    }

    pub fn record(&self) -> Option<Value> {
        self.log.record(&self.id).ok()
    }

    pub fn set_error(&self, code: &str, message: &str) {
        let mut inner = self.log.lock();
        if let Some(entry) = inner.entries.iter_mut().find(|entry| entry.id == self.id) {
            entry.error = Some(json!({"code":code,"message":message}));
        }
        inner.dirty = true;
    }

    pub fn finish(&self, state: BlockState, exit_code: Option<i32>) {
        let mut inner = self.log.lock();
        let Some(entry) = inner.entries.iter_mut().find(|entry| entry.id == self.id) else {
            return;
        };
        if entry.state != BlockState::Running {
            return;
        }
        entry.state = state;
        entry.exit_code = exit_code;
        entry.duration_ms = Some(entry.started.elapsed().as_millis() as u64);
        inner.revision = inner.revision.saturating_add(1);
        inner.dirty = true;
    }
}

impl Drop for BlockHandle {
    fn drop(&mut self) {
        self.finish(BlockState::Unknown, None);
    }
}

/// Plain-text preview, not terminal emulation. Parsing the retained prefix as a whole
/// handles split UTF-8/CSI/OSC without an unbounded partial-sequence buffer.
fn plain_text(bytes: &[u8]) -> String {
    enum State {
        Text,
        Escape,
        Csi,
        String,
        StringEscape,
    }
    let mut state = State::Text;
    let mut clean = Vec::new();
    for &byte in bytes {
        state = match state {
            State::Text => match byte {
                0x1b => State::Escape,
                b'\n' | b'\t' | 0x20..=0x7e | 0x80..=0xff => {
                    clean.push(byte);
                    State::Text
                }
                _ => State::Text,
            },
            State::Escape => match byte {
                b'[' => State::Csi,
                b']' | b'P' | b'_' | b'^' => State::String,
                _ => State::Text,
            },
            State::Csi => {
                if (0x40..=0x7e).contains(&byte) {
                    State::Text
                } else {
                    State::Csi
                }
            }
            State::String => match byte {
                7 => State::Text,
                0x1b => State::StringEscape,
                _ => State::String,
            },
            State::StringEscape => match byte {
                b'\\' | 7 => State::Text,
                0x1b => State::StringEscape,
                _ => State::String,
            },
        };
    }
    // Do not display a replacement character for a still-in-flight UTF-8 suffix.
    if let Err(error) = std::str::from_utf8(&clean) {
        if error.error_len().is_none() {
            clean.truncate(error.valid_up_to());
        }
    }
    String::from_utf8_lossy(&clean).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_records_keep_empty_keys_and_actual_input_in_both_modes() -> Result<()> {
        let log = BlockLog::default();
        for (id, channel, stream_mode) in [
            ("terminal", "pty", "merged"),
            ("background", "exec", "separated"),
        ] {
            let block = log.begin(
                id,
                BlockSpec::new("printf actual\nprintf input".into(), true),
                channel,
            )?;
            let record = log.record(id)?;
            assert_eq!(record["stdin"], "printf actual\nprintf input");
            assert_eq!(record["stdout"], "");
            assert_eq!(record["stderr"], "");
            assert_eq!(record["streamMode"], stream_mode);
            assert_eq!(record["status"], "running");
            assert!(record["exitCode"].is_null());
            assert!(record.get("error").is_some_and(Value::is_null));
            block.append(b"ordinary\n", false);
            block.append(b"diagnostic\n", true);
            block.finish(BlockState::Completed, Some(1));
            let record = log.record(id)?;
            assert_eq!(record["stdout"], "ordinary\n");
            assert_eq!(record["stderr"], "diagnostic\n");
            assert_eq!(record["streamMode"], "separated");
            assert_eq!(record["exitCode"], 1);
        }
        let snapshot = log.dialogue_pairs(2);
        assert_eq!(snapshot["pairs"].as_array().map(Vec::len), Some(2));
        assert_eq!(snapshot["pairs"][0]["operationId"], "background");
        assert_eq!(snapshot, log.dialogue_pairs(2));
        assert_eq!(BlockLog::default().dialogue_pairs(2)["pairs"], json!([]));
        Ok(())
    }

    #[test]
    fn recent_lines_include_running_output_and_the_tail_after_retention_loss() -> Result<()> {
        let log = BlockLog::default();
        let block = log.begin("live", BlockSpec::new("long-task".into(), true), "exec")?;
        block.append(&vec![b'x'; STREAM_BYTES + 4096], false);
        block.append(b"\nfirst\nsecond\nthird\n", false);
        block.append(b"failure\n", true);
        let lines = log.read_lines("live", 2)?;
        assert_eq!(lines["lines"], json!(["second", "third"]));
        assert_eq!(lines["stderrLines"], json!(["failure"]));
        assert_eq!(lines["status"], "running");
        assert_eq!(lines["truncated"], true);
        assert_eq!(lines, log.read_lines("live", 2)?);
        assert!(log.read_lines("missing", 2).is_err());
        Ok(())
    }

    #[test]
    fn record_snapshots_fit_json_budget_without_consuming_history() -> Result<()> {
        let log = BlockLog::default();
        for index in 0..MAX_BLOCKS {
            let block = log.begin(
                &index.to_string(),
                BlockSpec::new("'\n中".repeat(10_000), true),
                "exec",
            )?;
            block.append(&vec![0; STREAM_BYTES], false);
            block.append(&vec![255; STREAM_BYTES], true);
            block.finish(BlockState::Completed, Some(0));
        }
        let snapshot = log.dialogue_pairs(32);
        assert!(serde_json::to_vec(&snapshot)?.len() <= RECORD_BYTES);
        assert_eq!(snapshot["pairs"].as_array().map(Vec::len), Some(32));
        assert_eq!(snapshot["truncated"], true);
        assert_eq!(snapshot, log.dialogue_pairs(32));
        assert!(
            serde_json::to_vec(&crate::ssh::result::tool_result(&snapshot))?.len() < 192 * 1024
        );
        Ok(())
    }

    #[test]
    fn logical_lines_strip_ansi_without_using_screen_wraps() {
        assert_eq!(
            line_snapshot("\x1b[31m设备\x1b[0m\r\nnext\nlast".as_bytes(), 2)["lines"],
            json!(["next", "last"])
        );
        assert_eq!(line_snapshot(b"", 2)["lines"], json!([]));
        let long = line_snapshot(&vec![b'"'; 256 * 1024], 1);
        assert_eq!(long["truncated"], true);
        assert!(serde_json::to_vec(&long).is_ok_and(|bytes| bytes.len() < RECORD_BYTES));
    }

    #[test]
    fn record_streams_round_trip_text_and_binary_within_the_host_budget() -> Result<()> {
        use base64::Engine;
        for bytes in [
            vec![b'x'; 32 * 1024],
            "中\"\\\n".repeat(5000).into_bytes(),
            vec![255; 32 * 1024],
        ] {
            let log = BlockLog::default();
            let block = log.begin("op", BlockSpec::new("actual input".into(), true), "exec")?;
            block.append(&bytes, false);
            block.append(&bytes, true);
            block.finish(BlockState::Completed, Some(0));
            let record = log.record("op")?;
            assert_eq!(record["truncated"], false);
            for field in ["stdout", "stderr"] {
                let value = record[field]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing stream"))?;
                let decoded = if record[format!("{field}Encoding")] == "base64" {
                    base64::engine::general_purpose::STANDARD.decode(value)?
                } else {
                    value.as_bytes().to_vec()
                };
                assert_eq!(decoded, bytes);
            }
            assert!(
                serde_json::to_vec(&crate::ssh::result::tool_result(&record))?.len() < 192 * 1024
            );
        }
        Ok(())
    }

    #[test]
    fn selection_is_epoch_bound_validated_paged_and_pruned_on_eviction() -> Result<()> {
        let log = BlockLog::default();
        for index in 0..32 {
            let block = log.begin(
                &format!("chosen-{index}"),
                BlockSpec::new("中\"".repeat(200), false),
                "exec",
            )?;
            block.append(&vec![b'"'; OUTPUT_BYTES], false);
            block.finish(BlockState::Completed, Some(0));
        }
        let epoch = log.snapshot(None)?["epoch"]
            .as_str()
            .ok_or_else(|| anyhow!("missing epoch"))?
            .to_owned();
        assert!(log.select("stale", vec![]).is_err());
        assert!(log.select(&epoch, vec!["missing".into()]).is_err());
        assert_eq!(log.selected_snapshot(None)?["blocks"], json!([]));
        log.select(
            &epoch,
            (0..32).map(|index| format!("chosen-{index}")).collect(),
        )?;
        let mut before = None;
        let mut count = 0;
        loop {
            let page = log.selected_snapshot(before)?;
            assert!(serde_json::to_vec(&page)?.len() <= PAGE_BYTES);
            count += page["blocks"].as_array().map_or(0, Vec::len);
            if page["more"] == false {
                break;
            }
            before = page["nextBefore"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok());
            assert!(before.is_some());
        }
        assert_eq!(count, 32);
        let _new = log.begin("new", BlockSpec::new("true".into(), false), "pty")?;
        assert_eq!(log.selected_snapshot(None)?["selectedCount"], 31);
        log.select(&epoch, vec![])?;
        assert_eq!(log.selected_snapshot(None)?["blocks"], json!([]));
        Ok(())
    }

    #[test]
    fn previews_handle_split_utf8_and_terminal_controls() -> Result<()> {
        let log = BlockLog::default();
        let block = log.begin(
            "agent-op",
            BlockSpec::new("printf device".into(), true),
            "pty",
        )?;
        for byte in "\x1b[32m设备\x1b[0m\r\n\x1b]0;private title\x07ready".as_bytes() {
            block.append(&[*byte], false);
        }
        let live = log.snapshot(None)?;
        assert_eq!(live["blocks"][0]["output"], "设备\nready");
        assert_eq!(live["blocks"][0]["state"], "running");
        block.finish(BlockState::Completed, Some(3));
        drop(block);
        let done = log.snapshot(None)?;
        assert_eq!(done["blocks"][0]["id"], "agent-op");
        assert_eq!(done["blocks"][0]["actor"], "robot");
        assert_eq!(done["blocks"][0]["state"], "completed");
        assert_eq!(done["blocks"][0]["exitCode"], 3);
        assert!(done["blocks"][0].get("selected").is_none());
        Ok(())
    }

    #[test]
    fn history_is_bounded_without_evicting_active_commands() -> Result<()> {
        let log = BlockLog::default();
        let active = log.begin("active", BlockSpec::new("sleep 60".into(), false), "exec")?;
        for index in 0..40 {
            let block = log.begin(
                &format!("op-{index}"),
                BlockSpec::new("true".into(), false),
                "pty",
            )?;
            block.finish(BlockState::Completed, Some(0));
        }
        let snapshot = log.snapshot(None)?;
        let blocks = snapshot["blocks"]
            .as_array()
            .ok_or_else(|| anyhow!("missing blocks"))?;
        assert_eq!(blocks.len(), MAX_BLOCKS);
        assert_eq!(snapshot["dropped"], 9);
        assert_eq!(
            blocks.last().map(|block| &block["id"]),
            Some(&json!("active"))
        );
        drop(active);
        assert_eq!(log.snapshot(None)?["blocks"][31]["state"], "unknown");
        Ok(())
    }

    #[test]
    fn binary_previews_and_long_commands_page_within_the_wire_budget() -> Result<()> {
        let log = BlockLog::default();
        for index in 0..32 {
            let block = log.begin(
                &format!("op-{index}"),
                BlockSpec::new("'中\n".repeat(1000), false),
                "exec",
            )?;
            block.append(&vec![255; OUTPUT_BYTES + 20], false);
            block.append(b"more stderr", true);
            block.finish(BlockState::Completed, Some(1));
        }
        let mut before = None;
        let mut ids = std::collections::HashSet::new();
        loop {
            let snapshot = log.snapshot(before)?;
            assert!(serde_json::to_vec(&snapshot)?.len() <= PAGE_BYTES);
            let blocks = snapshot["blocks"]
                .as_array()
                .ok_or_else(|| anyhow!("missing blocks"))?;
            assert!(!blocks.is_empty());
            for block in blocks {
                assert!(ids.insert(
                    block["id"]
                        .as_str()
                        .ok_or_else(|| anyhow!("missing id"))?
                        .to_owned()
                ));
                assert_eq!(block["commandTruncated"], true);
                assert_eq!(block["outputTruncated"], true);
            }
            if snapshot["more"] == false {
                break;
            }
            let next = snapshot["nextBefore"]
                .as_u64()
                .ok_or_else(|| anyhow!("missing cursor"))? as u32;
            assert!(before.is_none_or(|previous| next < previous));
            before = Some(next);
        }
        assert_eq!(ids.len(), MAX_BLOCKS);
        Ok(())
    }

    #[test]
    fn notifications_coalesce_and_retry_when_consumers_are_slow() -> Result<()> {
        let log = BlockLog::default();
        let block = log.begin("op", BlockSpec::new("echo".into(), false), "exec")?;
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        log.notify("instance-one", &sender);
        block.append(b"first", false);
        block.append(b"second", false);
        log.notify("instance-one", &sender);
        let _ = receiver.try_recv()?;
        log.notify("instance-one", &sender);
        match receiver.try_recv()? {
            DriverNotification::Event {
                driver_instance_id,
                name,
                payload,
            } => {
                assert_eq!(driver_instance_id, "instance-one");
                assert_eq!(name, "ssh.blocks.changed");
                assert_eq!(payload["revision"], "3");
                assert!(payload.get("output").is_none());
            }
            _ => bail!("wrong notification"),
        }
        log.notify("instance-one", &sender);
        assert!(receiver.try_recv().is_err());
        Ok(())
    }
}
