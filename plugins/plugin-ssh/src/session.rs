//! SSH owns its connection, PTY and bounded replay buffer. No host internals.
use std::{collections::VecDeque, sync::Arc, time::Duration};

use crate::command::Command;
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use coterm_proto::runtime::{message::PluginNotification, ErrorCategory, RuntimeError};
use russh::{client, keys::ssh_key, ChannelMsg, Disconnect};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    sync::{mpsc, oneshot, Mutex},
    task::JoinHandle,
    time::timeout,
};

const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const OUTPUT_CHUNK_BYTES: usize = 8 * 1024;
// Base64 plus envelope must fit the host ledger's 16 KiB result limit.
const READ_BYTES: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(7);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
}

fn default_port() -> u16 {
    22
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.host.trim().is_empty()
            || self.host.len() > 255
            || self.host.chars().any(char::is_control)
            || self.username.trim().is_empty()
            || self.username.len() > 128
            || self.username.chars().any(char::is_control)
            || self.port == 0
        {
            bail!("Invalid SSH host, port or username");
        }
        Ok(())
    }
}

struct HostKey {
    expected: Option<String>,
    observed: Arc<Mutex<Option<String>>>,
}

impl client::Handler for HostKey {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = key
            .public_key()
            .fingerprint(ssh_key::HashAlg::Sha256)
            .to_string();
        *self.observed.lock().await = Some(fingerprint.clone());
        // A probe authenticates nobody. An authenticated connection MUST match a pin.
        Ok(self
            .expected
            .as_ref()
            .is_none_or(|expected| expected == &fingerprint))
    }
}

async fn handshake(
    config: &Config,
    expected: Option<String>,
) -> Result<(client::Handle<HostKey>, String)> {
    let observed = Arc::new(Mutex::new(None));
    let handler = HostKey {
        expected,
        observed: observed.clone(),
    };
    let transport = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let connection = client::connect(transport, (config.host.as_str(), config.port), handler)
        .await
        .map_err(|_| anyhow!("SSH connection failed or host key did not match"))?;
    let fingerprint = observed
        .lock()
        .await
        .clone()
        .ok_or_else(|| anyhow!("Server provided no host key"))?;
    Ok((connection, fingerprint))
}

pub async fn probe(config: &Config) -> Result<String> {
    let (connection, fingerprint) = timeout(CONNECT_TIMEOUT, handshake(config, None))
        .await
        .map_err(|_| anyhow!("SSH host key probe timed out"))??;
    let _ = connection
        .disconnect(Disconnect::ByApplication, "Probe complete", "en")
        .await;
    Ok(fingerprint)
}

#[derive(Default)]
pub struct OutputBuffer {
    chunks: VecDeque<(u64, Vec<u8>)>,
    bytes: usize,
    sequence: u64,
    pub closed: bool,
}

impl OutputBuffer {
    pub fn push(&mut self, data: &[u8]) {
        for part in data.chunks(OUTPUT_CHUNK_BYTES) {
            self.sequence += 1;
            self.chunks.push_back((self.sequence, part.to_vec()));
            self.bytes += part.len();
            while self.bytes > MAX_OUTPUT_BYTES {
                if let Some((_, data)) = self.chunks.pop_front() {
                    self.bytes -= data.len();
                }
            }
        }
    }

    pub fn read(&self, after: u64) -> Result<Value> {
        if after > self.sequence {
            bail!("Output cursor is ahead of the connection");
        }
        let first = self
            .chunks
            .front()
            .map_or(self.sequence + 1, |chunk| chunk.0);
        let gap = after.saturating_add(1) < first;
        let mut next = if gap { first - 1 } else { after };
        let mut data = Vec::with_capacity(READ_BYTES);
        for (sequence, part) in &self.chunks {
            if *sequence <= next {
                continue;
            }
            if data.len() + part.len() > READ_BYTES {
                break;
            }
            data.extend_from_slice(part);
            next = *sequence;
        }
        Ok(
            json!({"data": STANDARD.encode(data), "cursor": next.to_string(), "gap": gap, "closed": self.closed, "more": next < self.sequence}),
        )
    }
}

enum Input {
    Data(Vec<u8>),
    Resize(u32, u32),
    Command(Command, Vec<u8>),
    Close,
}

struct Action {
    input: Input,
    reply: oneshot::Sender<Result<()>>,
}

pub struct Connection {
    actions: mpsc::Sender<Action>,
    pub output: Arc<Mutex<OutputBuffer>>,
    task: JoinHandle<()>,
    pub id: String,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Connection {
    pub fn is_closed(&self) -> bool {
        self.task.is_finished()
    }

    pub async fn connect(config: &Config, fingerprint: String, password: &str) -> Result<Self> {
        Self::connect_with_events(config, fingerprint, password, None).await
    }

    pub async fn connect_with_events(
        config: &Config,
        fingerprint: String,
        password: &str,
        events: Option<(String, mpsc::Sender<PluginNotification>)>,
    ) -> Result<Self> {
        if !fingerprint.starts_with("SHA256:") || fingerprint.len() > 128 {
            bail!("A confirmed SHA256 host fingerprint is required");
        }
        timeout(CONNECT_TIMEOUT, async {
            let (mut handle, _) = handshake(config, Some(fingerprint)).await?;
            if !handle.authenticate_password(config.username.clone(), password).await
                .map_err(|_| anyhow!("SSH authentication failed"))?.success() {
                bail!("SSH password authentication was rejected");
            }
            let mut channel = handle.channel_open_session().await.map_err(|_| anyhow!("SSH session channel failed"))?;
            let output = Arc::new(Mutex::new(OutputBuffer::default()));
            channel.request_pty(true, "xterm-256color", 80, 24, 0, 0, &[]).await?;
            wait_success(&mut channel, &output).await?;
            channel.request_shell(true).await?;
            wait_success(&mut channel, &output).await?;
            let (mut read, write) = channel.split();
            let (actions, mut receiver) = mpsc::channel::<Action>(64);
            let task_output = output.clone();
            let id = uuid::Uuid::new_v4().to_string();
            let connection_id = id.clone();
            let task = tokio::spawn(async move {
                let mut active: Option<Command> = None;
                loop {
                    let deadline = active.as_ref().filter(|c| !c.timed_out).map(|c| c.deadline)
                        .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(86400));
                    tokio::select! {
                        action = receiver.recv() => {
                            let Some(action) = action else { break; };
                            if matches!(&action.input, Input::Command(_, _)) && active.is_some() {
                                let _ = action.reply.send(Err(anyhow!("A command is still running or its completion is unknown")));
                                continue;
                            }
                            if matches!(&action.input, Input::Close) {
                                if let Some(command) = active.take() { fail_command(&events, &command, "ssh_disconnected_before_completion").await; }
                                let _ = action.reply.send(Ok(()));
                                break;
                            }
                            let result = timeout(IO_TIMEOUT, async {
                                match action.input {
                                    Input::Data(data) => write.data(&data[..]).await,
                                    Input::Resize(cols, rows) => write.window_change(cols, rows, 0, 0).await,
                                    Input::Command(command, data) => { active = Some(command); write.data(&data[..]).await },
                                    Input::Close => Ok(()),
                                }
                            }).await;
                            let failed = !matches!(result, Ok(Ok(())));
                            let _ = action.reply.send(if failed { Err(anyhow!("SSH input was not confirmed; connection closed")) } else { Ok(()) });
                            if failed { break; }
                        }
                        message = read.wait() => match message {
                            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                                let (visible, completed) = match active.as_mut() {
                                    Some(command) => command.feed(&data),
                                    None => (data.to_vec(), None),
                                };
                                push_output(&task_output, &events, &connection_id, &visible).await;
                                if let Some(result) = completed {
                                    if let Some(command) = active.take() {
                                        if !command.timed_out {
                                            if let Some((_, sender)) = &events {
                                                let result = if command.mcp { json!({"content":[{"type":"text","text":result.to_string()}],"isError":result["exitCode"] != 0}) } else { result };
                                                let _ = sender.send(PluginNotification::OperationCompleted { operation_id:command.operation_id, result }).await;
                                            }
                                        }
                                    }
                                }
                            }
                            Some(ChannelMsg::Close) | None => break,
                            _ => {}
                        },
                        _ = tokio::time::sleep_until(deadline), if active.as_ref().is_some_and(|c| !c.timed_out) => {
                            if let Some(command) = active.as_mut() {
                                command.timed_out = true;
                                fail_command(&events, command, "ssh_command_completion_unknown").await;
                            }
                        }
                    }
                }
                if let Some(mut command) = active {
                    push_output(&task_output, &events, &connection_id, &command.remaining()).await;
                    if !command.timed_out { fail_command(&events, &command, "ssh_connection_lost_before_completion").await; }
                }
                task_output.lock().await.closed = true;
                if let Some((instance, sender)) = &events {
                    let cursor = task_output.lock().await.sequence.to_string();
                    let _ = sender.send(PluginNotification::Event { plugin_instance_id:instance.clone(), name:"ssh.status".into(), payload:json!({"connected":false,"connectionId":connection_id,"cursor":cursor}) }).await;
                }
                let _ = timeout(IO_TIMEOUT, handle.disconnect(Disconnect::ByApplication, "Session closed", "en")).await;
            });
            Ok(Self { actions, output, task, id })
        }).await.map_err(|_| anyhow!("SSH connect/authentication timed out"))?
    }

    async fn submit(&self, input: Input) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.actions
            .try_send(Action { input, reply })
            .map_err(|_| anyhow!("SSH input queue is full or closed"))?;
        timeout(Duration::from_secs(3), response)
            .await
            .map_err(|_| anyhow!("SSH input wait timed out; delivery may be unknown"))?
            .map_err(|_| anyhow!("SSH connection closed before input acknowledgement"))?
    }

    pub async fn input(&self, data: Vec<u8>) -> Result<()> {
        if data.is_empty() || data.len() > 16 * 1024 {
            bail!("SSH input must contain 1 to 16384 bytes");
        }
        self.submit(Input::Data(data)).await
    }

    pub async fn command(&self, operation_id: String, mcp: bool, text: &str) -> Result<()> {
        let (command, wire) = Command::new(operation_id, mcp, text);
        if wire.len() > 4000 {
            bail!("Encoded command exceeds the shared PTY line limit; shorten the command");
        }
        self.submit(Input::Command(command, wire)).await
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.submit(Input::Close).await
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        if cols == 0 || rows == 0 || cols > 1000 || rows > 1000 {
            bail!("Invalid terminal dimensions");
        }
        self.submit(Input::Resize(cols, rows)).await
    }
}

async fn wait_success(
    channel: &mut russh::Channel<client::Msg>,
    output: &Arc<Mutex<OutputBuffer>>,
) -> Result<()> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Data { data }) => output.lock().await.push(&data),
            Some(ChannelMsg::Failure | ChannelMsg::Close) | None => {
                bail!("Server refused an interactive SSH terminal")
            }
            _ => {}
        }
    }
}

async fn push_output(
    output: &Arc<Mutex<OutputBuffer>>,
    events: &Option<(String, mpsc::Sender<PluginNotification>)>,
    connection_id: &str,
    bytes: &[u8],
) {
    for part in bytes.chunks(OUTPUT_CHUNK_BYTES) {
        let cursor = {
            let mut output = output.lock().await;
            output.push(part);
            output.sequence.to_string()
        };
        if let Some((instance, sender)) = events {
            if sender.send(PluginNotification::Event { plugin_instance_id:instance.clone(), name:"ssh.output".into(),
                payload:json!({"connectionId":connection_id,"data":STANDARD.encode(part),"cursor":cursor}) }).await.is_err() { break; }
        }
    }
}

async fn fail_command(
    events: &Option<(String, mpsc::Sender<PluginNotification>)>,
    command: &Command,
    code: &str,
) {
    if let Some((_, sender)) = events {
        let _ = sender
            .send(PluginNotification::OperationFailed {
                operation_id: command.operation_id.clone(),
                error: RuntimeError {
                    category: ErrorCategory::Domain,
                    code: code.to_owned(),
                    message: "SSH command completion is unknown; no command was replayed".into(),
                    details: None,
                },
            })
            .await;
    }
}
