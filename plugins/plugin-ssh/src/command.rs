//! POSIX shell command boundaries. A prompt or a period of silence is not completion.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

const MAX_CAPTURE: usize = 8 * 1024;

pub struct Command {
    pub operation_id: String,
    pub mcp: bool,
    begin: Vec<u8>,
    end: Vec<u8>,
    pending: Vec<u8>,
    started: bool,
    output: Vec<u8>,
    truncated: bool,
    pub deadline: tokio::time::Instant,
    pub timed_out: bool,
}

impl Command {
    pub fn new(operation_id: String, mcp: bool, text: &str) -> (Self, Vec<u8>) {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let begin = format!("\x1b]777;coterm;{token};begin\x07").into_bytes();
        let end = format!("\x1b]777;coterm;{token};end;").into_bytes();
        let quoted = text.replace('\'', "'\\''");
        // eval runs in the existing shell, preserving cd/export and the shared PTY.
        // The echoed source contains printable escapes, never the actual markers.
        let wire = format!("printf '\\033]777;coterm;{token};begin\\007'; eval '{quoted}'; __coterm_rc=$?; printf '\\033]777;coterm;{token};end;%d\\007' \"$__coterm_rc\"\r").into_bytes();
        (
            Self {
                operation_id,
                mcp,
                begin,
                end,
                pending: Vec::new(),
                started: false,
                output: Vec::new(),
                truncated: false,
                deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(120),
                timed_out: false,
            },
            wire,
        )
    }

    fn capture(&mut self, bytes: &[u8], visible: &mut Vec<u8>) {
        visible.extend_from_slice(bytes);
        if self.started {
            let remaining = MAX_CAPTURE.saturating_sub(self.output.len());
            self.output
                .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
            self.truncated |= bytes.len() > remaining;
        }
    }

    /// Returns visible terminal bytes and an optional final domain result.
    /// Only a possible marker prefix is withheld across arbitrary SSH packet splits.
    pub fn feed(&mut self, bytes: &[u8]) -> (Vec<u8>, Option<Value>) {
        self.pending.extend_from_slice(bytes);
        let mut visible = Vec::new();
        loop {
            let marker = if self.started { &self.end } else { &self.begin };
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
            if !self.started {
                self.pending.drain(..self.begin.len());
                self.started = true;
                continue;
            }
            let prefix = self.end.len();
            if let Some(end) = self.pending[prefix..].iter().position(|b| *b == 7) {
                let end = prefix + end;
                let code = std::str::from_utf8(&self.pending[prefix..end])
                    .ok()
                    .and_then(|s| s.parse::<u8>().ok());
                if let Some(code) = code {
                    self.pending.drain(..=end);
                    visible.append(&mut self.pending);
                    return (
                        visible,
                        Some(
                            json!({"data":STANDARD.encode(&self.output),"encoding":"base64","exitCode":code,"outputTruncated":self.truncated}),
                        ),
                    );
                }
            } else if self.pending.len() <= prefix + 4 {
                return (visible, None);
            }
            // Malformed lookalike: forward it and keep scanning, without growing memory.
            let byte = self.pending.remove(0);
            self.capture(&[byte], &mut visible);
        }
    }

    pub fn remaining(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn boundaries_survive_every_packet_split_and_exclude_echo() {
        let (template, wire) = Command::new("op".into(), true, "printf 'test'; false");
        assert!(!wire.contains(&27));
        let mut bytes = b"echoed command\r\n".to_vec();
        bytes.extend_from_slice(&template.begin);
        bytes.extend_from_slice(b"actual output\r\n");
        bytes.extend_from_slice(&template.end);
        bytes.extend_from_slice(b"7\x07prompt");
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
            assert_eq!(result["exitCode"], 7);
            assert_eq!(
                STANDARD
                    .decode(result["data"].as_str().expect("base64"))
                    .expect("valid base64"),
                b"actual output\r\n"
            );
            assert_eq!(visible, b"echoed command\r\nactual output\r\nprompt");
        }
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
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["exitCode"], 1);
        assert_eq!(results[1]["exitCode"], 0);
        for result in results {
            assert_eq!(
                STANDARD.decode(result["data"].as_str().unwrap_or_default())?,
                b"a b"
            );
        }
        assert_eq!(visible, b"a ba b");
        Ok(())
    }

    #[tokio::test]
    async fn capture_is_bounded_and_silence_does_not_complete() {
        let (mut command, _) = Command::new("op".into(), false, "yes");
        let begin = command.begin.clone();
        assert!(command.feed(&begin).1.is_none());
        assert!(command.feed(&vec![b'x'; MAX_CAPTURE * 3]).1.is_none());
        let mut end = command.end.clone();
        end.extend_from_slice(b"0\x07");
        let result = command.feed(&end).1.expect("complete");
        assert_eq!(result["outputTruncated"], true);
        assert_eq!(
            STANDARD
                .decode(result["data"].as_str().expect("base64"))
                .expect("valid")
                .len(),
            MAX_CAPTURE
        );
    }
}
