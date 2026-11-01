//! Plugin-owned, in-memory command previews. Raw terminal input is never recorded here.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::{anyhow, bail, Result};
use coterm_proto::runtime::{message::PluginNotification, operation::OperationSource};
use serde::Serialize;
use serde_json::{json, Value};

const MAX_BLOCKS: usize = 32;
const COMMAND_BYTES: usize = 1024;
const OUTPUT_BYTES: usize = 2048;
const PAGE_BYTES: usize = 12 * 1024;

#[derive(Clone)]
pub(crate) struct BlockSpec {
    pub text: String,
    pub source: OperationSource,
}

impl BlockSpec {
    pub fn new(text: String, mcp: bool) -> Self {
        Self {
            text,
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
}

impl Entry {
    fn value(&self) -> Value {
        json!({
            "id": self.id, "index": self.index, "command": self.command,
            "actor": self.actor, "channel": self.channel, "state": self.state,
            "exitCode": self.exit_code, "output": plain_text(&self.stdout),
            "stderr": plain_text(&self.stderr), "commandTruncated": self.command_truncated,
            "outputTruncated": self.output_truncated,
        })
    }
}

struct Inner {
    epoch: String,
    revision: u64,
    next_index: u32,
    dropped: u32,
    dirty: bool,
    entries: VecDeque<Entry>,
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
            inner.entries.remove(position);
            inner.dropped = inner.dropped.saturating_add(1);
        }
        let mut end = spec.text.len().min(COMMAND_BYTES);
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
        let inner = self.lock();
        let mut result = json!({"epoch": inner.epoch, "revision": inner.revision.to_string(),
            "dropped": inner.dropped, "blocks": [], "more": false, "nextBefore": null});
        let mut blocks = Vec::new();
        let mut next_before = None;
        for entry in inner
            .entries
            .iter()
            .rev()
            .filter(|entry| before.is_none_or(|index| entry.index < index))
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
    pub fn notify(&self, instance: &str, sender: &tokio::sync::mpsc::Sender<PluginNotification>) {
        let mut inner = self.lock();
        if !inner.dirty {
            return;
        }
        let event = PluginNotification::Event {
            plugin_instance_id: instance.to_owned(),
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
        let take = bytes
            .len()
            .min(OUTPUT_BYTES.saturating_sub(entry.stdout.len() + entry.stderr.len()));
        let was_truncated = entry.output_truncated;
        if stderr {
            entry.stderr.extend_from_slice(&bytes[..take]);
        } else {
            entry.stdout.extend_from_slice(&bytes[..take]);
        }
        entry.output_truncated |= take < bytes.len();
        if take > 0 || was_truncated != entry.output_truncated {
            inner.revision = inner.revision.saturating_add(1);
            inner.dirty = true;
        }
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
            PluginNotification::Event {
                plugin_instance_id,
                name,
                payload,
            } => {
                assert_eq!(plugin_instance_id, "instance-one");
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
