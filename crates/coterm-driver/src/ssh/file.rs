//! Text-file execution owns its deadline and capacity checks, independent of host waiting.
use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Result};
use coterm_core::{
    limits::{
        DEFAULT_FILE_TIMEOUT_MS, FILE_CONTENT_MAX_BYTES, FILE_PATH_MAX_BYTES,
        OPERATION_RESULT_MAX_BYTES,
    },
    model::{ErrorCategory, RuntimeError},
};
use russh::{client, Channel, ChannelMsg};
use serde_json::{json, Value};
use tokio::{
    sync::watch,
    time::{sleep_until, timeout, Instant},
};

use super::{
    result::tool_result,
    session::{self, HostKey},
};

const METADATA_DIAGNOSTIC_MAX_BYTES: usize = 1024;

enum Kind {
    Read,
    Metadata,
    Write { content: String, mode: String },
}

pub(super) struct Request {
    path: String,
    deadline: Duration,
    kind: Kind,
}

impl Request {
    /// Validate a native or MCP file request before approval or secret collection.
    pub(super) fn parse(operation: &str, args: &Value) -> Result<Self> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow!("A file path is required"))?;
        if path.is_empty() || path.len() > FILE_PATH_MAX_BYTES || path.chars().any(char::is_control)
        {
            bail!("SSH path must contain 1 to {FILE_PATH_MAX_BYTES} UTF-8 bytes without controls");
        }
        let timeout_ms = match args.get("timeoutMs") {
            None => DEFAULT_FILE_TIMEOUT_MS,
            Some(value) => value
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or_else(|| anyhow!("timeoutMs must be a positive integer"))?,
        };
        let deadline = Duration::from_millis(timeout_ms);
        if Instant::now().checked_add(deadline).is_none() {
            bail!("timeoutMs is not representable on this platform");
        }
        let kind = match operation.trim_start_matches("mcp.") {
            "ssh.read_file" => Kind::Read,
            "ssh.file_info" => Kind::Metadata,
            "ssh.write_file" => {
                let content = args["content"]
                    .as_str()
                    .ok_or_else(|| anyhow!("File content is required"))?;
                if content.len() > FILE_CONTENT_MAX_BYTES {
                    bail!("SSH file content exceeds {FILE_CONTENT_MAX_BYTES} UTF-8 bytes");
                }
                let mode = args
                    .get("mode")
                    .map_or(Some("0644"), Value::as_str)
                    .ok_or_else(|| anyhow!("Invalid file mode"))?;
                if !matches!(mode, "0600" | "0640" | "0644" | "0700" | "0750" | "0755") {
                    bail!("Unsupported SSH file mode");
                }
                Kind::Write {
                    content: content.to_owned(),
                    mode: mode.to_owned(),
                }
            }
            _ => bail!("Unknown file operation"),
        };
        Ok(Self {
            path: path.to_owned(),
            deadline,
            kind,
        })
    }

    fn command(&self) -> String {
        match &self.kind {
            Kind::Read => format!("cat < '{}'", session::shell_quote(&self.path)),
            Kind::Metadata => session::file_info_script(&self.path),
            Kind::Write { mode, .. } => {
                let path = session::shell_quote(&shell_path(&self.path));
                let temporary = session::shell_quote(&shell_path(&format!(
                    "{}.coterm-{}",
                    self.path,
                    uuid::Uuid::new_v4().simple()
                )));
                format!("umask 077; __p='{path}'; __t='{temporary}'; cat > \"$__t\" && chmod {mode} \"$__t\" && mv -f \"$__t\" \"$__p\"; __rc=$?; if [ $__rc -ne 0 ]; then rm -f \"$__t\"; fi; exit $__rc")
            }
        }
    }

    fn result(
        &self,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: Option<i32>,
        sudo: bool,
    ) -> Result<Value> {
        if exit_code.is_none() {
            return Err(RuntimeError {
                category: ErrorCategory::Transport,
                code: "ssh_file_incomplete".into(),
                message: "SSH file channel closed without confirming completion; remote effects may be unknown".into(),
                details: None,
            }.into());
        }
        if exit_code != Some(0) {
            bail!("SSH file operation failed (exit code {exit_code:?})");
        }
        let context = json!({"channel":"exec","inheritsPtyState":false,"sudo":sudo});
        let result = match &self.kind {
            Kind::Read => {
                let text = String::from_utf8(stdout)
                    .map_err(|_| anyhow!("SSH file is not valid UTF-8 text"))?;
                json!({"path":self.path,"bytes":text.len(),"text":text,"limit":FILE_CONTENT_MAX_BYTES,"truncated":false,"executionContext":context})
            }
            Kind::Metadata => {
                let mut fields = session::parse_fields(&stdout);
                fields.insert("path".into(), json!(self.path));
                fields.insert("exitCode".into(), json!(exit_code));
                let diagnostic = String::from_utf8_lossy(&stderr);
                if !diagnostic.trim().is_empty() {
                    fields.insert("error".into(), json!(diagnostic));
                }
                Value::Object(fields)
            }
            Kind::Write { content, mode } => {
                json!({"path":self.path,"bytes":content.len(),"mode":mode,"written":true,"executionContext":context})
            }
        };
        if serde_json::to_vec(&tool_result(&result))?.len() > OPERATION_RESULT_MAX_BYTES {
            bail!("SSH file result exceeds {OPERATION_RESULT_MAX_BYTES} JSON bytes after encoding");
        }
        Ok(result)
    }
}

/// Run one file request. Deadline/cancellation closes its own channel; write effects may be unknown.
pub(super) async fn run(
    handle: Arc<client::Handle<HostKey>>,
    request: Request,
    password: Option<&str>,
    mut cancel: Option<watch::Receiver<bool>>,
) -> Result<Value> {
    let deadline = Instant::now()
        .checked_add(request.deadline)
        .ok_or_else(|| anyhow!("timeoutMs is not representable on this platform"))?;
    let mut channel = None;
    let result = tokio::select! {
        result = async {
            channel = Some(handle.channel_open_session().await?);
            let channel = channel.as_mut().ok_or_else(|| anyhow!("SSH file channel unavailable"))?;
            let input = match &request.kind { Kind::Write { content, .. } => Some(content.as_bytes()), _ => None };
            session::start_channel(channel, &request.command(), input, password).await?;
            let (stdout, stderr, exit_code) = collect(channel).await?;
            request.result(stdout, stderr, exit_code, password.is_some())
        } => result,
        _ = sleep_until(deadline) => Err(interrupted("ssh_file_timeout", "SSH file execution deadline expired; remote effects may be unknown", request.deadline)),
        _ = async {
            match cancel.as_mut() {
                Some(signal) => { let _ = signal.changed().await; },
                None => std::future::pending().await,
            }
        } => Err(interrupted("ssh_file_cancelled", "SSH file cancellation requested; remote effects may be unknown", request.deadline)),
    };
    if let Some(channel) = channel.as_mut() {
        let _ = timeout(Duration::from_secs(2), channel.close()).await;
    }
    result
}

async fn collect(channel: &mut Channel<client::Msg>) -> Result<(Vec<u8>, Vec<u8>, Option<i32>)> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { data }) => {
                if stdout.len().saturating_add(data.len()) > FILE_CONTENT_MAX_BYTES {
                    bail!("SSH file exceeds {FILE_CONTENT_MAX_BYTES} UTF-8 bytes");
                }
                stdout.extend_from_slice(&data);
            }
            Some(ChannelMsg::ExtendedData { data, .. }) => {
                let take = METADATA_DIAGNOSTIC_MAX_BYTES
                    .saturating_sub(stderr.len())
                    .min(data.len());
                stderr.extend_from_slice(&data[..take]);
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => exit_code = Some(exit_status as i32),
            Some(ChannelMsg::Failure) => bail!("SSH server refused the file operation"),
            Some(ChannelMsg::Close) | None => break,
            _ => {}
        }
    }
    Ok((stdout, stderr, exit_code))
}

fn interrupted(code: &str, message: &str, deadline: Duration) -> anyhow::Error {
    RuntimeError {
        category: ErrorCategory::Transport,
        code: code.into(),
        message: message.into(),
        details: Some(json!({"timeoutMs":deadline.as_millis() as u64})),
    }
    .into()
}

fn shell_path(path: &str) -> String {
    if path.starts_with('-') {
        format!("./{path}")
    } else {
        path.to_owned()
    }
}
