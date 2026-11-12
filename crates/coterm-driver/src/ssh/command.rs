//! POSIX shell command boundaries. A prompt or a period of silence is not completion.
//!
//! Two boundaries frame one command: a `begin` marker printed before the source runs and
//! an `end` marker that carries the exit code and whether static syntax validation failed.
//! The remote shell echoes the wrapper line; that echo is dropped before it reaches the
//! replay buffer so terminal recordings and model context stay free of protocol noise.
use crate::ssh::blocks::{BlockHandle, BlockState};
pub(crate) use crate::ssh::output::encode_output;
#[cfg(test)]
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

pub const MAX_CAPTURE: usize = 8 * 1024;
/// Upper bound for bytes withheld before the begin marker. Readline redraws a long
/// command line (continuation prompts, CRLF, bracketed-paste toggles), so the echo can
/// exceed the wrapper several times over; anything larger is not a single command echo.
const MAX_PRE_MARKER: usize = 32 * 1024;
/// Distinctive fragments of the wrapper. Readline may insert a CR and padding inside a
/// long line, so a single long literal is fragile; any short hint is enough to identify
/// the region as our echo. A prompt that happens to contain "coterm" only costs the
/// prompt line, never command output.
const ECHO_HINTS: &[&[u8]] = &[b"__coterm", b"coterm_src", b"coterm;"];

/// Result encoding. UTF-8 terminal output preserves ANSI and common terminal controls;
/// binary data stays base64 so clients can recover the exact bytes.
#[cfg(test)]
const ENCODING_TEXT: &str = "utf-8";
#[cfg(test)]
const ENCODING_BASE64: &str = "base64";

pub struct Command {
    pub operation_id: String,
    pub mcp: bool,
    begin: Vec<u8>,
    end: Vec<u8>,
    pending: Vec<u8>,
    started: bool,
    output: Vec<u8>,
    stderr: Vec<u8>,
    stdin: String,
    separated: bool,
    echo_screened: bool,
    original: usize,
    dropped: usize,
    truncated: bool,
    exit_code: Option<u8>,
    syntax_error: bool,
    started_at: tokio::time::Instant,
    pub cancelled: bool,
    block: Option<BlockHandle>,
    boundaries: Vec<(usize, &'static str)>,
}

impl Command {
    pub fn new(operation_id: String, mcp: bool, text: &str) -> (Self, Vec<u8>) {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let begin = format!("\x1b]777;coterm;{token};begin\x07").into_bytes();
        let end = format!("\x1b]777;coterm;{token};end;").into_bytes();
        let quoted = text.replace('\'', "'\\''");
        // The source is validated by the same shell that will execute it. An incomplete
        // or malformed command is reported with a syntax flag instead of reaching `eval`,
        // which keeps the shared PTY alive when a login shell would otherwise abort.
        // `eval` still runs in the existing shell, preserving cd/export and the shared PTY.
        let wire = format!(
            "printf '\\033]777;coterm;{token};begin\\007'; __coterm_src='{quoted}'; \
             __coterm_flag=; __coterm_sh=${{BASH:-${{SHELL:-/bin/sh}}}}; \
             command -v \"$__coterm_sh\" >/dev/null 2>&1 || __coterm_sh=/bin/sh; \
             if __coterm_err=$(printf '%s\\n' \"$__coterm_src\" | \"$__coterm_sh\" -n 2>&1); then \
             eval \"$__coterm_src\" && __coterm_rc=0 || __coterm_rc=$?; \
             else __coterm_rc=2; __coterm_flag=syntax; printf '%s\\n' \"$__coterm_err\"; fi; \
             printf '\\033]777;coterm;{token};end;%d;%s\\007' \"$__coterm_rc\" \"$__coterm_flag\"\r"
        );
        let wire = terminal_wire(text, &quoted, wire).into_bytes();
        (
            Self {
                operation_id,
                mcp,
                begin,
                end,
                pending: Vec::new(),
                started: false,
                output: Vec::new(),
                stderr: Vec::new(),
                stdin: text.to_owned(),
                separated: false,
                echo_screened: false,
                original: 0,
                dropped: 0,
                truncated: false,
                exit_code: None,
                syntax_error: false,
                started_at: tokio::time::Instant::now(),
                cancelled: false,
                block: None,
                boundaries: Vec::new(),
            },
            wire,
        )
    }

    fn capture(&mut self, bytes: &[u8], visible: &mut Vec<u8>) {
        visible.extend_from_slice(bytes);
        if self.started {
            if let Some(block) = &self.block {
                block.append(bytes, false);
            }
            self.original += bytes.len();
            let remaining = MAX_CAPTURE.saturating_sub(self.output.len());
            let take = bytes.len().min(remaining);
            self.output.extend_from_slice(&bytes[..take]);
            self.dropped += bytes.len() - take;
            self.truncated |= take < bytes.len();
        }
    }

    /// Drops the echoed wrapper line before boundary scanning.
    ///
    /// Readline does not echo a long line verbatim: it inserts CRLF, continuation
    /// prompts and mode toggles at the terminal width. Exact matching therefore fails on
    /// real servers. Instead the whole pre-marker region is withheld until the `begin`
    /// marker arrives, and is discarded only when it contains the wrapper hint
    /// `__coterm_src=`. A server that does not echo never produces that hint, so its
    /// prompt and output are still replayed. The region is bounded so a missing marker
    /// cannot stall the stream.
    fn screen_pre_marker(&mut self, visible: &mut Vec<u8>) -> bool {
        if self.started {
            return true;
        }
        let Some(index) = self
            .pending
            .windows(self.begin.len())
            .position(|window| window == self.begin)
        else {
            if self.pending.len() > MAX_PRE_MARKER {
                // No marker in a large stream: stop screening and replay what we have,
                // keeping a tail that could still be a split marker.
                let keep = (1..self.begin.len())
                    .rev()
                    .find(|&n| self.pending.ends_with(&self.begin[..n]))
                    .unwrap_or(0);
                let flush = self.pending.len() - keep;
                let bytes: Vec<_> = self.pending.drain(..flush).collect();
                self.echo_screened |= ECHO_HINTS
                    .iter()
                    .any(|hint| bytes.windows(hint.len()).any(|window| window == *hint));
                if !self.echo_screened {
                    visible.extend_from_slice(&bytes);
                }
                return true;
            }
            return false;
        };
        let before: Vec<_> = self.pending.drain(..index).collect();
        let echoed = self.echo_screened
            || ECHO_HINTS
                .iter()
                .any(|hint| before.windows(hint.len()).any(|window| window == *hint));
        if !echoed {
            visible.extend_from_slice(&before);
        }
        self.pending.drain(..self.begin.len());
        self.started = true;
        self.boundaries.push((visible.len(), "begin"));
        true
    }

    /// Returns visible terminal bytes and an optional final domain result.
    /// Only a possible marker prefix is withheld across arbitrary SSH packet splits.
    pub fn feed(&mut self, bytes: &[u8]) -> (Vec<u8>, Option<Value>) {
        self.pending.extend_from_slice(bytes);
        let mut visible = Vec::new();
        if !self.screen_pre_marker(&mut visible) {
            return (visible, None);
        }
        loop {
            let marker = &self.end;
            let Some(index) = self.pending.windows(marker.len()).position(|w| w == marker) else {
                let keep = (1..marker.len())
                    .rev()
                    .find(|&n| self.pending.ends_with(&marker[..n]))
                    .unwrap_or(0);
                let flush = self.pending.len() - keep;
                let bytes: Vec<_> = self.pending.drain(..flush).collect();
                self.capture(&bytes, &mut visible);
                return (visible, None);
            };
            let bytes: Vec<_> = self.pending.drain(..index).collect();
            self.capture(&bytes, &mut visible);
            let prefix = self.end.len();
            if let Some(end) = self.pending[prefix..].iter().position(|b| *b == 7) {
                let end = prefix + end;
                let fields = std::str::from_utf8(&self.pending[prefix..end])
                    .ok()
                    .map(|fields| fields.split(';').collect::<Vec<_>>());
                let code = fields
                    .as_ref()
                    .and_then(|fields| fields.first())
                    .and_then(|code| code.parse::<u8>().ok());
                if let Some(code) = code {
                    self.syntax_error = fields
                        .as_ref()
                        .is_some_and(|fields| fields.get(1) == Some(&"syntax"));
                    self.exit_code = Some(code);
                    self.pending.drain(..=end);
                    self.boundaries.push((visible.len(), "end"));
                    visible.append(&mut self.pending);
                    let result = self.finish();
                    return (visible, Some(result));
                }
            } else if self.pending.len() <= prefix + 8 {
                return (visible, None);
            }
            // Malformed lookalike: forward it and keep scanning, without growing memory.
            let byte = self.pending.remove(0);
            self.capture(&[byte], &mut visible);
        }
    }

    fn finish(&self) -> Value {
        let (encoding, field, value) = encode_output(&self.output, self.truncated);
        // Flat and explicit: no nested copies of the same payload.
        let recorded = self.block.as_ref().and_then(BlockHandle::record);
        let tracked = recorded.is_some();
        let mut result = recorded
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_else(|| {
                let (out_encoding, _, out) = encode_output(&self.output, self.truncated);
                let (err_encoding, _, err) = encode_output(&self.stderr, self.truncated);
                let input = crate::ssh::output::text_prefix(&self.stdin, 8 * 1024);
                serde_json::from_value(json!({"instanceId":null,"operationId":self.operation_id,
                    "mode":"terminal","stdin":input,"stdout":out,"stderr":err,
                    "stdoutEncoding":out_encoding,"stderrEncoding":err_encoding,
                    "streamMode":if self.separated {"separated"} else {"merged"},
                    "truncated":self.truncated || input.len() < self.stdin.len(),"error":null}))
                .unwrap_or_default()
            });
        result.insert(
            "status".into(),
            json!(if self.cancelled {
                "cancelled"
            } else {
                "completed"
            }),
        );
        result.insert(field.to_owned(), Value::String(value));
        result.insert("encoding".into(), Value::String(encoding.into()));
        result.insert("exitCode".into(), json!(self.exit_code));
        result.insert("bytes".into(), json!(self.output.len()));
        result.insert("originalBytes".into(), json!(self.original));
        result.insert("droppedBytes".into(), json!(self.dropped));
        result.insert("limit".into(), json!(MAX_CAPTURE));
        let truncated =
            (!tracked && self.truncated) || result.get("truncated") == Some(&Value::Bool(true));
        result.insert("truncated".into(), json!(truncated));
        result.insert("outputTruncated".into(), json!(self.truncated));
        result.insert("syntaxError".into(), json!(self.syntax_error));
        result.insert("cancelled".into(), json!(self.cancelled));
        result.insert(
            "durationMs".into(),
            json!(self.started_at.elapsed().as_millis() as u64),
        );
        result.insert(
            "executionContext".into(),
            json!({"channel":"pty","inheritsPtyState":true}),
        );
        Value::Object(result)
    }

    pub fn remaining(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }

    pub fn track(&mut self, block: BlockHandle) {
        self.block = Some(block);
    }

    pub fn take_annotations(&mut self) -> Vec<(usize, Value)> {
        std::mem::take(&mut self.boundaries)
            .into_iter()
            .filter_map(|(offset, kind)| Some((offset, self.block.as_ref()?.annotation(kind)?)))
            .collect()
    }

    pub fn finish_block(&self, state: BlockState, exit_code: Option<i32>) {
        if let Some(block) = &self.block {
            block.finish(state, exit_code);
        }
    }

    pub fn feed_stderr(&mut self, bytes: &[u8]) {
        self.separated = true;
        if let Some(block) = &self.block {
            block.append(bytes, true);
        }
        let take = bytes
            .len()
            .min(MAX_CAPTURE.saturating_sub(self.stderr.len()));
        self.stderr.extend_from_slice(&bytes[..take]);
        self.truncated |= take < bytes.len();
    }

    pub fn record_error(&self, code: &str, message: &str) {
        if let Some(block) = &self.block {
            block.set_error(code, message);
        }
    }
}

// A brace group keeps cd/export in the login shell. Each assignment fits the
// canonical PTY line limit, and no source executes until the entire group parses.
fn terminal_wire(text: &str, quoted: &str, wire: String) -> String {
    if wire.len() <= 4000 && !text.contains(['\n', '\r']) {
        return wire;
    }
    let assignment = format!("__coterm_src='{quoted}'; ");
    let body = wire.replacen(&assignment, "", 1);
    let mut result = String::from("{ __coterm_src=\r");
    let mut chunk = String::new();
    for c in text.chars() {
        let escaped = if c == '\'' {
            "'\\''".to_owned()
        } else {
            c.to_string()
        };
        if chunk.len() + escaped.len() > 1024 {
            result.push_str(&format!("__coterm_src=\"$__coterm_src\"'{chunk}'\r"));
            chunk.clear();
        }
        chunk.push_str(&escaped);
    }
    result.push_str(&format!("__coterm_src=\"$__coterm_src\"'{chunk}'\r"));
    result.push_str(body.trim_end_matches('\r'));
    result.push_str("\r}\r");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn long_multiline_terminal_source_executes_once_and_preserves_shell_state(
    ) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;
        let source = format!(
            "export COTERM_LONG='{}'\nprintf '%s' \"$COTERM_LONG\"",
            "'中".repeat(4000).replace('\'', "'\\''")
        );
        let (mut first, mut wire) = Command::new("long".into(), false, &source);
        assert!(wire
            .split(|byte| *byte == b'\r' || *byte == b'\n')
            .all(|line| line.len() < 4000));
        let (mut second, next) =
            Command::new("next".into(), false, "printf '%s' \"${#COTERM_LONG}\"");
        wire.extend(next);
        for byte in &mut wire {
            if *byte == b'\r' {
                *byte = b'\n';
            }
        }
        let mut child = match tokio::process::Command::new("sh")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) if cfg!(windows) && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(())
            }
            Err(error) => return Err(error.into()),
        };
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing stdin"))?;
        // Reading concurrently avoids filling a pipe while the shell echoes large output.
        let sender = tokio::spawn(async move { stdin.write_all(&wire).await });
        let output =
            tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
                .await??;
        sender.await??;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut results = Vec::new();
        for byte in output.stdout {
            let command = if results.is_empty() {
                &mut first
            } else {
                &mut second
            };
            if let Some(result) = command.feed(&[byte]).1 {
                results.push(result);
            }
        }
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["exitCode"], 0);
        assert_eq!(results[1]["exitCode"], 0);
        assert!(results[1]["text"]
            .as_str()
            .is_some_and(|text| text != "0" && !text.is_empty()));
        Ok(())
    }

    #[test]
    fn long_wrapper_echo_is_hidden_with_bounded_pending_bytes() {
        let source = format!("printf done\n#{}", "'".repeat(65_000));
        let (mut command, wire) = Command::new("long".into(), true, &source);
        for part in wire.chunks(4096) {
            let (visible, result) = command.feed(part);
            assert!(visible.is_empty());
            assert!(result.is_none());
            assert!(command.pending.len() <= MAX_PRE_MARKER);
        }
        let mut output = command.begin.clone();
        output.extend_from_slice(b"done");
        output.extend_from_slice(&command.end);
        output.extend_from_slice(b"0;\x07");
        let (visible, result) = command.feed(&output);
        assert_eq!(visible, b"done");
        assert_eq!(
            result.as_ref().map(|value| &value["stdout"]),
            Some(&json!("done"))
        );
    }

    #[test]
    fn distinct_terminal_stderr_is_recorded_without_becoming_visible_stdout() {
        let (mut command, _) = Command::new("op".into(), false, "true");
        let begin = command.begin.clone();
        command.feed(&begin);
        command.feed_stderr(b"diagnostic");
        let mut end = command.end.clone();
        end.extend_from_slice(b"0;\x07");
        let (visible, result) = command.feed(&end);
        assert!(visible.is_empty());
        let result = result.unwrap_or_default();
        assert_eq!(result["stdout"], "");
        assert_eq!(result["stderr"], "diagnostic");
        assert_eq!(result["streamMode"], "separated");
    }

    #[test]
    fn trusted_ui_annotations_survive_byte_splits_without_changing_output() -> anyhow::Result<()> {
        let log = crate::ssh::blocks::BlockLog::default();
        let block = log.begin(
            "tracked",
            crate::ssh::blocks::BlockSpec::new("echo real".into(), true),
            "pty",
        )?;
        let (mut command, _) = Command::new("tracked".into(), true, "echo real");
        command.track(block);
        let bytes = [
            command.begin.clone(),
            b"real\r\n".to_vec(),
            command.end.clone(),
            b"0;\x07".to_vec(),
        ]
        .concat();
        let mut visible = Vec::new();
        let mut boundaries = Vec::new();
        for byte in bytes {
            let (part, _) = command.feed(&[byte]);
            for (offset, value) in command.take_annotations() {
                boundaries.push((visible.len() + offset, value));
            }
            visible.extend(part);
        }
        assert_eq!(visible, b"real\r\n");
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[0].0, 0);
        assert_eq!(boundaries[1].0, 6);
        assert_eq!(boundaries[0].1["actor"], "robot");
        assert!(boundaries[0].1.get("command").is_none());
        assert!(boundaries[0].1.get("output").is_none());
        assert_eq!(log.snapshot(None)?["blocks"][0]["output"], "real\n");
        Ok(())
    }

    fn echo_of(wire: &[u8]) -> Vec<u8> {
        let mut echo = wire.to_vec();
        if echo.last() == Some(&b'\r') {
            echo.pop();
            echo.extend_from_slice(b"\r\n");
        }
        echo
    }

    #[test]
    fn ansi_output_is_readable_without_decoding_and_keeps_control_characters() {
        let bytes = b"\x1b[32mconfig.yaml\x1b[0m\r\n";
        let (encoding, field, text) = encode_output(bytes, false);
        assert_eq!(encoding, ENCODING_TEXT);
        assert_eq!(field, "text");
        assert_eq!(text.as_bytes(), bytes);
    }

    #[tokio::test]
    async fn control_heavy_output_stays_lossless_within_the_host_record_limit() {
        let payload = b"\x1b[0m".repeat(MAX_CAPTURE / 4);
        let (mut command, _) = Command::new("op".into(), true, "printf output");
        let bytes = [&command.begin[..], &payload, &command.end, b"0;\x07"].concat();
        let result = command.feed(&bytes).1.expect("completion");
        assert!(
            serde_json::to_vec(&crate::ssh::result::tool_result(&result))
                .expect("JSON")
                .len()
                <= coterm_core::limits::OPERATION_RESULT_MAX_BYTES
        );
        assert_eq!(
            STANDARD
                .decode(result["data"].as_str().unwrap_or_default())
                .expect("base64"),
            payload
        );
        assert_eq!(result["truncated"], false);
    }

    #[tokio::test]
    async fn compound_command_output_survives_prompt_changes_and_every_packet_split() {
        let source = "cd /tmp && export PS1='changed> ' && ls -la";
        let (template, wire) = Command::new("op".into(), true, source);
        let output = b"total 4\r\n\x1b[34mconfig\x1b[0m\r\n";
        let bytes = [
            b"old$ ".as_slice(),
            &echo_of(&wire),
            &template.begin,
            output.as_slice(),
            &template.end,
            b"0;\x07changed> ",
        ]
        .concat();
        for split in 0..=bytes.len() {
            let mut command = Command {
                begin: template.begin.clone(),
                end: template.end.clone(),
                ..Command::new("op".into(), true, source).0
            };
            let (mut visible, first) = command.feed(&bytes[..split]);
            let (rest, second) = if first.is_none() {
                command.feed(&bytes[split..])
            } else {
                (bytes[split..].to_vec(), None)
            };
            visible.extend(rest);
            let result = first.or(second).expect("completion marker");
            assert_eq!(result["exitCode"], 0, "split={split}");
            assert_eq!(
                result["text"].as_str().map(str::as_bytes),
                Some(output.as_slice()),
                "split={split}"
            );
            assert_eq!(
                visible,
                [output.as_slice(), b"changed> "].concat(),
                "split={split}"
            );
        }
    }

    #[tokio::test]
    async fn boundaries_survive_every_packet_split_and_hide_the_echo() {
        let (template, wire) = Command::new("op".into(), true, "printf 'test'; false");
        assert!(!wire.contains(&27));
        // Readline wraps a long line with CRLF and a continuation prompt; the hint
        // still marks the region as our echo.
        let mut bytes = b"prompt$ ".to_vec();
        bytes.extend_from_slice(&echo_of(&wire));
        bytes.extend_from_slice(&[13, 10, 62, 32]);

        bytes.extend_from_slice(&template.begin);
        bytes.extend_from_slice(b"actual output\r\n");
        bytes.extend_from_slice(&template.end);
        bytes.extend_from_slice(b"7\x07prompt$ ");
        for split in 0..=bytes.len() {
            let mut command = Command {
                begin: template.begin.clone(),
                end: template.end.clone(),
                ..Command::new("op".into(), true, "true").0
            };
            let (mut visible, first) = command.feed(&bytes[..split]);
            let (second_visible, second) = if first.is_none() {
                command.feed(&bytes[split..])
            } else {
                (bytes[split..].to_vec(), None)
            };
            visible.extend(second_visible);
            let result = first.or(second).expect("completion marker");
            assert_eq!(result["exitCode"], 7, "split={split}");
            assert_eq!(result["text"], "actual output\r\n", "split={split}");
            // The echoed region (prompt line plus wrapper redraws) is dropped;
            // command output and the next prompt stay visible.
            assert_eq!(
                visible,
                &[
                    97, 99, 116, 117, 97, 108, 32, 111, 117, 116, 112, 117, 116, 13, 10, 112, 114,
                    111, 109, 112, 116, 36, 32
                ][..]
            );
        }
    }

    #[tokio::test]
    async fn readline_redraws_are_dropped_and_partial_markers_survive() {
        // Real interactive bash redraws a long line with CRLF and a continuation prompt.
        // Only the wrapper hint marks the region as our echo.
        let (mut command, wire) = Command::new("op".into(), false, "echo hi");
        let begin = command.begin.clone();
        let crlf_prompt: &[u8] = &[13, 10, 62, 32];
        let mut region = b"root@host:/srv# printf '\\033]777;coterm;".to_vec();
        region.extend_from_slice(&wire[..40]);
        region.extend_from_slice(crlf_prompt);
        region.extend_from_slice(&wire[40..120]);
        region.extend_from_slice(crlf_prompt);
        region.extend_from_slice(&wire[120..]);
        region.extend_from_slice(&[13, 10]);
        region.extend_from_slice(&begin);
        // Deliver the region one byte at a time so the marker is never contiguous.
        let mut visible = Vec::new();
        for byte in &region {
            visible.extend(command.feed(&[*byte]).0);
        }
        assert!(visible.is_empty(), "wrapper region leaked: {visible:?}");
        let (visible, result) = command.feed(&[104, 105, 13, 10]);
        assert!(result.is_none());
        assert_eq!(visible, &[104, 105, 13, 10][..]);
        let mut end = command.end.clone();
        end.extend_from_slice(&[48, 59, 7]);
        let (_, result) = command.feed(&end);
        assert_eq!(
            result.expect("completion")["text"]
                .as_str()
                .map(str::as_bytes),
            Some(&[104, 105, 13, 10][..])
        );
    }

    #[tokio::test]
    async fn a_different_echo_is_released_instead_of_swallowed() {
        let (mut command, wire) = Command::new("op".into(), false, "printf 'x'");
        let begin = command.begin.clone();
        let mut bytes = b"prompt$ printf 'x' extra\r\n".to_vec();
        bytes.extend_from_slice(&begin);
        let (visible, result) = command.feed(&bytes);
        assert!(result.is_none());
        // Only the trailing partial marker prefix may be withheld, never user output.
        assert!(visible.starts_with(b"prompt$ printf 'x' extra\r\n"));
        assert!(visible.len() >= b"prompt$ printf 'x' extra\r\n".len());
        let _ = wire;
    }

    #[tokio::test]
    async fn a_syntax_error_is_flagged_instead_of_escaping_the_session() {
        let (mut command, _) = Command::new("op".into(), false, "if");
        let begin = command.begin.clone();
        let mut bytes = b"prompt$ ".to_vec();
        bytes.extend_from_slice(&begin);
        bytes.extend_from_slice(b"bash: syntax error: unexpected end of file\r\n");
        bytes.extend_from_slice(&command.end);
        bytes.extend_from_slice(b"2;syntax\x07");
        let (_, result) = command.feed(&bytes);
        let result = result.expect("completion");
        assert_eq!(result["exitCode"], 2);
        assert_eq!(result["syntaxError"], true);
        assert_eq!(
            result["text"],
            "bash: syntax error: unexpected end of file\r\n"
        );
    }

    #[tokio::test]
    async fn binary_and_control_bytes_stay_base64_and_report_truncation() {
        let (mut command, _) = Command::new("op".into(), false, "yes");
        assert!(command.feed(&command.begin.clone()).1.is_none());
        assert!(command.feed(&vec![b'x'; MAX_CAPTURE * 3]).1.is_none());
        let mut end = command.end.clone();
        end.extend_from_slice(b"0;\x07");
        let result = command.feed(&end).1.expect("complete");
        assert_eq!(result["truncated"], true);
        assert_eq!(result["outputTruncated"], true);
        assert_eq!(result["bytes"], MAX_CAPTURE);
        assert_eq!(result["originalBytes"], MAX_CAPTURE * 3);
        assert_eq!(
            result["droppedBytes"].as_u64(),
            Some((MAX_CAPTURE * 2) as u64)
        );
        assert_eq!(result["encoding"], ENCODING_TEXT);
        // NUL and C0 control bytes are never presented as text.
        let (mut command, _) = Command::new("op".into(), false, "true");
        let begin = command.begin.clone();
        command.feed(&begin);
        let mut end = command.end.clone();
        end.extend_from_slice(b"0;\x07");
        let payload = [0u8, 1, 2, 0x1b, 0x7f];
        let (_, result) = command.feed(&[&payload[..], &end[..]].concat());
        let result = result.expect("complete");
        assert_eq!(result["encoding"], ENCODING_BASE64);
        assert_eq!(
            STANDARD
                .decode(result["data"].as_str().unwrap_or_default())
                .expect("base64"),
            payload
        );
    }

    #[tokio::test]
    async fn real_posix_shell_preserves_state_quotes_and_exit_codes() -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;
        let (mut first, mut wire) = Command::new(
            "first".into(),
            false,
            "export COTERM_WRAPPER_TEST='a b'; printf '%s' \"$COTERM_WRAPPER_TEST\"; false",
        );
        let (mut second, next) = Command::new(
            "second".into(),
            false,
            "printf '%s' \"$COTERM_WRAPPER_TEST\"",
        );
        wire.extend(next);
        // A PTY translates CR to newline before the shell reads it.
        for byte in &mut wire {
            if *byte == b'\r' {
                *byte = b'\n';
            }
        }
        let mut child = match tokio::process::Command::new("sh")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) if cfg!(windows) && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(())
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&wire).await?;
        }
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(3), child.wait_with_output())
                .await??;
        assert!(result.status.success());
        let mut results = Vec::new();
        let mut visible = Vec::new();
        for byte in result.stdout {
            let command = if results.is_empty() {
                &mut first
            } else {
                &mut second
            };
            let (bytes, result) = command.feed(&[byte]);
            visible.extend(bytes);
            if let Some(result) = result {
                results.push(result);
            }
        }
        assert_eq!(results.len(), 2, "results={results:?} visible={visible:?}");
        assert_eq!(results[0]["exitCode"], 1, "results={results:?}");
        assert_eq!(results[1]["exitCode"], 0, "{results:?}");
        for result in results {
            assert_eq!(result["text"], "a b", "{result:?}");
        }
        assert_eq!(visible, b"a ba b");
        Ok(())
    }

    #[tokio::test]
    async fn syntax_validation_runs_before_eval_on_a_real_shell() -> anyhow::Result<()> {
        let (mut command, wire) = Command::new("op".into(), false, "printf 'BEFORE'; if");
        let mut child = match tokio::process::Command::new("sh")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) if cfg!(windows) && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(())
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(mut stdin) = child.stdin.take() {
            let mut wire = wire;
            for byte in &mut wire {
                if *byte == b'\r' {
                    *byte = b'\n';
                }
            }
            tokio::io::AsyncWriteExt::write_all(&mut stdin, &wire).await?;
        }
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(3), child.wait_with_output())
                .await??;
        let mut completion = None;
        for byte in result.stdout {
            let (_, result) = command.feed(&[byte]);
            if result.is_some() {
                completion = result;
            }
        }
        let completion = completion.expect("completion marker");
        assert_eq!(completion["exitCode"], 2);
        assert_eq!(completion["syntaxError"], true);
        assert!(completion["text"]
            .as_str()
            .is_some_and(|text| text.contains("syntax error")));
        Ok(())
    }
}
