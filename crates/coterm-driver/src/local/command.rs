//! Local command records and explicit PTY boundaries. Prompts are never completion signals.
use std::{
    collections::VecDeque,
    hash::{BuildHasher, Hasher},
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::local::shell::ShellFamily;

pub(crate) type History = Arc<Mutex<Records>>;
const CAPACITY: usize = 32;
const STREAM_BYTES: usize = 16 * 1024;
const RESULT_BYTES: usize = 12 * 1024;

pub(crate) fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}

struct Record {
    id: String,
    input: String,
    out: Vec<u8>,
    err: Vec<u8>,
    mode: &'static str,
    status: &'static str,
    exit: Option<i32>,
    truncated: bool,
}

/// Bounded instance-owned records; absent output streams retain their keys.
#[derive(Default)]
pub(crate) struct Records {
    records: VecDeque<Record>,
    dropped: usize,
    selected: String,
    selection_truncated: bool,
}

impl Records {
    pub fn begin(&mut self, id: &str, input: String, mode: &'static str) -> Result<()> {
        if self.records.len() == CAPACITY {
            let index = self
                .records
                .iter()
                .position(|item| item.status != "running")
                .ok_or_else(|| anyhow!("Too many running local commands"))?;
            self.records.remove(index);
            self.dropped += 1;
        }
        self.records.push_back(Record {
            id: id.into(),
            input,
            out: Vec::new(),
            err: Vec::new(),
            mode,
            status: "running",
            exit: None,
            truncated: false,
        });
        Ok(())
    }

    pub fn append(&mut self, id: &str, bytes: &[u8], stderr: bool) {
        if let Some(item) = self.records.iter_mut().find(|item| item.id == id) {
            let stream = if stderr { &mut item.err } else { &mut item.out };
            let take = bytes.len().min(STREAM_BYTES.saturating_sub(stream.len()));
            stream.extend_from_slice(&bytes[..take]);
            item.truncated |= take < bytes.len();
        }
    }

    pub fn finish(&mut self, id: &str, status: &'static str, exit: Option<i32>) {
        if let Some(item) = self.records.iter_mut().find(|item| item.id == id) {
            item.status = status;
            item.exit = exit;
        }
    }

    pub fn value(&self, id: &str) -> Result<Value> {
        let item = self
            .records
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| anyhow!("Local command record not found"))?;
        let mut value = json!({"operationId":item.id,"mode":item.mode,"stdin":item.input,
            "stdout":String::from_utf8_lossy(&item.out),"stderr":String::from_utf8_lossy(&item.err),
            "streamMode":if item.mode == "terminal" {"merged"} else {"separated"},
            "status":item.status,"exitCode":item.exit,"truncated":item.truncated,"error":null});
        bound(&mut value, RESULT_BYTES - 1024);
        Ok(value)
    }

    pub fn pairs(&self, count: usize) -> Result<Value> {
        let mut pairs = Vec::new();
        for item in self.records.iter().rev().take(count) {
            let value = self.value(&item.id)?;
            pairs.push(value);
            if serde_json::to_vec(&pairs)?.len() > RESULT_BYTES - 256 {
                pairs.pop();
                break;
            }
        }
        Ok(
            json!({"truncated":pairs.len()<count.min(self.records.len()),"pairs":pairs,"dropped":self.dropped}),
        )
    }

    pub fn select(&mut self, text: &str, truncated: bool) {
        self.selected = prefix(text, STREAM_BYTES).to_owned();
        self.selection_truncated = truncated || self.selected.len() < text.len();
    }

    pub fn selected(&self) -> Value {
        let mut value = json!({"text":self.selected,"truncated":self.selection_truncated});
        bound(&mut value, RESULT_BYTES);
        value
    }
}

fn prefix(text: &str, bytes: usize) -> &str {
    let mut end = bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn bound(value: &mut Value, budget: usize) {
    while serde_json::to_vec(value).map_or(usize::MAX, |data| data.len()) > budget {
        let mut shortened = false;
        for name in ["stdin", "stdout", "stderr", "text"] {
            if let Some(text) = value[name].as_str().filter(|text| !text.is_empty()) {
                let next = prefix(text, text.len() / 2).to_owned();
                value[name] = json!(next);
                shortened = true;
            }
        }
        value["truncated"] = json!(true);
        if !shortened {
            break;
        }
    }
}

pub(crate) fn lines(bytes: &[u8], count: usize, dropped: bool, closed: bool) -> Value {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text
        .split('\n')
        .rev()
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut truncated = false;
    while serde_json::to_vec(&lines).map_or(usize::MAX, |data| data.len()) > RESULT_BYTES - 512 {
        truncated = true;
        if lines.len() > 1 {
            lines.remove(0);
        } else if let Some(line) = lines.first_mut() {
            *line = prefix(line, line.len() / 2).to_owned();
        } else {
            break;
        }
    }
    json!({"lines":lines,"stderrLines":[],"streamMode":"merged","dropped":dropped,"closed":closed,"truncated":truncated})
}

/// Validates a command list and produces source for its configured Shell family.
/// Unknown modes, unsupported families and source over 64 KiB are rejected before IO.
pub(crate) fn source(args: &Value, family: ShellFamily) -> Result<(String, String, bool)> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow!("Expected local command arguments"))?;
    if object
        .keys()
        .any(|key| !["commands", "mode", "driverInstanceId"].contains(&key.as_str()))
    {
        bail!("Unknown local command argument");
    }
    let mode = match args.get("mode") {
        None => "terminal",
        Some(Value::String(mode)) => mode,
        _ => bail!("Invalid local command mode"),
    };
    if !matches!(mode, "terminal" | "exec") {
        bail!("Invalid local command mode");
    }
    let commands = args["commands"]
        .as_array()
        .filter(|items| (1..=64).contains(&items.len()))
        .ok_or_else(|| anyhow!("Expected 1 to 64 local commands"))?;
    let texts = commands
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|text| !text.trim().is_empty() && !text.contains('\0'))
                .ok_or_else(|| anyhow!("Invalid local command source"))
        })
        .collect::<Result<Vec<_>>>()?;
    if texts.iter().map(|text| text.len()).sum::<usize>() > 64 * 1024 {
        bail!("Local source exceeds 64 KiB");
    }
    let quote = |text: &str| {
        text.replace(
            '\'',
            if family == ShellFamily::PowerShell {
                "''"
            } else {
                "'\\''"
            },
        )
    };
    let script = match family {
        ShellFamily::Posix => texts.iter().map(|text| format!("eval '{}'",quote(text))).collect::<Vec<_>>().join(" && "),
        ShellFamily::PowerShell => texts.iter().map(|text| format!(". ([scriptblock]::Create('{}')); if (!$?) {{ $__coterm_rc=if ($LASTEXITCODE) {{$LASTEXITCODE}} else {{1}}; throw 'Local command failed' }}",quote(text))).collect::<Vec<_>>().join("; "),
        ShellFamily::Cmd => texts.join("\r\nif errorlevel 1 exit /b %errorlevel%\r\n"),
        _ => bail!("This Shell supports terminal input but not structured foreground commands; choose a supported Shell or explicit exec"),
    };
    Ok((texts.join("\n"), script, mode == "exec"))
}

pub(crate) struct Foreground {
    pub id: String,
    pub mcp: bool,
    pub notified: bool,
    begin: Vec<u8>,
    end: Vec<u8>,
    pending: Vec<u8>,
    started: bool,
    cleanup: Vec<std::path::PathBuf>,
}

impl Foreground {
    pub async fn new(
        id: &str,
        mcp: bool,
        script: &str,
        family: ShellFamily,
    ) -> Result<(Self, Vec<u8>)> {
        let mut hash = std::collections::hash_map::RandomState::new().build_hasher();
        hash.write(id.as_bytes());
        let token = format!("{:016x}", hash.finish());
        let begin = format!("__coterm_{token}_begin__");
        let end = format!("__coterm_{token}_end_");
        // ConPTY can consume unknown OSCs. Split printable markers in source so
        // echoed wrapper text cannot be mistaken for an actual output boundary.
        let mut cleanup = Vec::new();
        let wire = match family {
            ShellFamily::Posix => {
                let quoted = script.replace('\'', "'\\''");
                format!("printf '__coterm_%s_begin__' '{token}'; __coterm_src='{quoted}'; if eval \"$__coterm_src\"; then __coterm_rc=0; else __coterm_rc=$?; fi; printf '__coterm_%s_end_%s__' '{token}' \"$__coterm_rc\"\n")
            }
            ShellFamily::PowerShell => format!("[Console]::Write(('__coterm_'+'{token}'+'_begin__')); $__coterm_rc=0; try {{ {script} }} catch {{ Write-Error $_; if ($__coterm_rc -eq 0) {{ $__coterm_rc=1 }} }}; [Console]::Write(('__coterm_'+'{token}'+'_end_'+$__coterm_rc+'__'))\r"),
            ShellFamily::Cmd => {
                let source_path = std::env::temp_dir().join(format!("coterm-{token}-source.cmd"));
                let wrapper_path = std::env::temp_dir().join(format!("coterm-{token}-wrapper.cmd"));
                let source = format!("@echo off\r\n{script}\r\n");
                let wrapper = format!("@echo off\r\necho {begin}\r\ncall \"{}\"\r\nset \"__coterm_rc=%errorlevel%\"\r\necho {end}%__coterm_rc%__\r\n@echo on\r\n",source_path.display());
                tokio::fs::write(&source_path,source).await?;
                if let Err(error) = tokio::fs::write(&wrapper_path,wrapper).await {
                    let _ = tokio::fs::remove_file(&source_path).await;
                    return Err(error.into());
                }
                cleanup = vec![source_path,wrapper_path.clone()];
                format!("@call \"{}\"\r",wrapper_path.display())
            }
            _ => bail!("Unsupported structured foreground Shell"),
        };
        Ok((
            Self {
                id: id.into(),
                mcp,
                notified: false,
                begin: begin.into_bytes(),
                end: end.into_bytes(),
                pending: Vec::new(),
                started: false,
                cleanup,
            },
            wire.into_bytes(),
        ))
    }

    /// Filters framing and command echo across read boundaries; returns an observed exit code.
    pub fn feed(&mut self, bytes: &[u8], history: &History) -> (Vec<u8>, Option<i32>) {
        self.pending.extend_from_slice(bytes);
        let mut controls = Vec::new();
        if !self.started {
            // Readline can request cursor position before it consumes the queued line.
            // Forward terminal queries even while suppressing the wrapper echo.
            for query in [
                b"\x1b[6n".as_slice(),
                b"\x1b[?6n".as_slice(),
                b"\x1b[c".as_slice(),
            ] {
                while let Some(index) = self
                    .pending
                    .windows(query.len())
                    .position(|part| part == query)
                {
                    controls.extend(self.pending.drain(index..index + query.len()));
                }
            }
            if let Some(index) = self
                .pending
                .windows(self.begin.len())
                .position(|part| part == self.begin)
            {
                self.pending.drain(..index + self.begin.len());
                self.started = true;
            } else {
                // Echo can be much larger than source due to terminal redraws.
                if self.pending.len() > 256 * 1024 {
                    self.pending.drain(..self.pending.len() - self.begin.len());
                }
                return (controls, None);
            }
        }
        if let Some(index) = self
            .pending
            .windows(self.end.len())
            .position(|part| part == self.end)
        {
            let code_start = index + self.end.len();
            if let Some(end) = self.pending[code_start..]
                .windows(2)
                .position(|bytes| bytes == b"__")
            {
                if let Ok(code) =
                    String::from_utf8_lossy(&self.pending[code_start..code_start + end])
                        .parse::<i32>()
                {
                    let output = self.pending[..index].to_vec();
                    lock(history).append(&self.id, &output, false);
                    let mut visible = controls;
                    visible.extend(output);
                    visible.extend_from_slice(&self.pending[code_start + end + 2..]);
                    self.pending.clear();
                    return (visible, Some(code));
                }
            } else if self.pending.len() - code_start < 16 {
                return (controls, None);
            }
        }
        let keep = (1..self.end.len())
            .rev()
            .find(|length| self.pending.ends_with(&self.end[..*length]))
            .unwrap_or(0);
        let flush = self.pending.len() - keep;
        let output = self.pending.drain(..flush).collect::<Vec<_>>();
        lock(history).append(&self.id, &output, false);
        controls.extend(output);
        (controls, None)
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        for path in &self.cleanup {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn framing_handles_every_packet_split_and_never_matches_wrapper_echo() -> Result<()> {
        for split in 0..100 {
            let history: History = Default::default();
            lock(&history).begin("test", "printf reply".into(), "terminal")?;
            let (mut command, wire) =
                Foreground::new("test", false, "printf reply", ShellFamily::Posix).await?;
            assert!(!wire
                .windows(command.begin.len())
                .any(|part| part == command.begin));
            let mut bytes = command.begin.clone();
            bytes.extend_from_slice("你好\nreply".as_bytes());
            bytes.extend_from_slice(&command.end);
            bytes.extend_from_slice(b"7__prompt");
            let split = split.min(bytes.len());
            let (first, code) = command.feed(&bytes[..split], &history);
            let (second, next) = if code.is_some() {
                (bytes[split..].to_vec(), None)
            } else {
                command.feed(&bytes[split..], &history)
            };
            assert_eq!(code.or(next), Some(7));
            assert_eq!([first, second].concat(), "你好\nreplyprompt".as_bytes());
            assert_eq!(lock(&history).value("test")?["stdout"], "你好\nreply");
        }
        Ok(())
    }

    #[tokio::test]
    async fn terminal_queries_are_forwarded_while_command_echo_is_suppressed() -> Result<()> {
        let history: History = Default::default();
        let (mut command, _) =
            Foreground::new("test", false, "echo ok", ShellFamily::PowerShell).await?;
        assert_eq!(command.feed(b"echo wrapper\x1b[", &history).0, b"");
        assert_eq!(command.feed(b"6n", &history).0, b"\x1b[6n");
        Ok(())
    }

    #[test]
    fn snapshots_have_bounded_json_and_report_eviction_and_truncation() -> Result<()> {
        let mut records = Records::default();
        for number in 0..40 {
            let id = number.to_string();
            records.begin(&id, "\0".repeat(65536), "exec")?;
            records.append(&id, &vec![0; 65536], false);
            records.finish(&id, "completed", Some(0));
        }
        let pairs = records.pairs(32)?;
        assert_eq!(pairs["dropped"], 8);
        assert_eq!(pairs["truncated"], true);
        assert!(serde_json::to_vec(&pairs)?.len() <= RESULT_BYTES);
        records.select(&"\0".repeat(20000), false);
        assert!(serde_json::to_vec(&records.selected())?.len() <= RESULT_BYTES);
        assert_eq!(records.selected()["truncated"], true);
        assert!(source(&json!({"commands":[""],"mode":"exec"}), ShellFamily::Posix).is_err());
        assert!(source(
            &json!({"commands":["echo ok"],"mode":"guess"}),
            ShellFamily::Posix
        )
        .is_err());
        Ok(())
    }
}
