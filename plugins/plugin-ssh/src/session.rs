//! SSH owns its connection, PTY and bounded replay buffer. No host internals.
//!
//! Two execution paths share one transport:
//! - the interactive PTY (`ssh.command`) preserves `cd`/`export` for humans and agents;
//! - one-shot exec channels (`ssh.exec`, read-only queries) isolate stdout/stderr and
//!   cannot be polluted by shell job notices or kill the login shell on a syntax error.
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::command::Command;
use crate::result::tool_result;
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use coterm_proto::runtime::{message::PluginNotification, ErrorCategory, RuntimeError};
use russh::{
    client,
    keys::ssh_key,
    ChannelMsg,
    Disconnect,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::{
    sync::{mpsc, oneshot, watch, Mutex},
    task::{JoinHandle, JoinSet},
    time::{sleep_until, timeout, Instant},
};

const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const OUTPUT_CHUNK_BYTES: usize = 8 * 1024;
// Base64 plus envelope must fit the host ledger's 16 KiB result limit.
const READ_BYTES: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(7);
/// How long Ctrl+C may take to produce the completion marker before the terminal is reset.
const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// Exec output retained for `ssh.output` polling, per stream and per job.
const EXEC_OUTPUT_LIMIT: usize = 64 * 1024;
/// Exec preview returned with the operation result; larger reads use `ssh.output`.
const EXEC_RESULT_LIMIT: usize = 4 * 1024;
const MAX_EXEC_JOBS: usize = 8;
const EXEC_MAX_DURATION: Duration = Duration::from_secs(30 * 60);
const QUERY_LIMIT: usize = 8 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Builds a classified error. The host ledger keeps the code and bounded details;
/// no credential or raw network error text is included.
fn session_error(
    category: ErrorCategory,
    code: &str,
    stage: &str,
    message: &str,
) -> anyhow::Error {
    anyhow::Error::new(RuntimeError {
        category,
        code: code.to_owned(),
        message: message.to_owned(),
        details: Some(json!({ "stage": stage })),
    })
}

fn host_key_mismatch(observed: Option<String>) -> anyhow::Error {
    anyhow::Error::new(RuntimeError {
        category: ErrorCategory::Conflict,
        code: "ssh_host_key_mismatch".to_owned(),
        message: "SSH host key did not match the confirmed fingerprint".to_owned(),
        details: Some(json!({ "stage": "host_key", "observedFingerprint": observed })),
    })
}

/// Maps transport failures to a stable stage so clients can decide whether to retry,
/// ask the user to confirm a fingerprint, or reconnect an exited shell.
fn connect_error(error: russh::Error, mismatch: Option<String>) -> anyhow::Error {
    use russh::Error as E;
    let (category, code, stage, message) = match error {
        E::UnknownKey => return host_key_mismatch(mismatch),
        E::ConnectionTimeout | E::KeepaliveTimeout | E::InactivityTimeout => (
            ErrorCategory::Transport,
            "ssh_connect_timeout",
            "tcp",
            "SSH connection timed out",
        ),
        E::Version
        | E::KexInit
        | E::Kex
        | E::NoCommonAlgo { .. }
        | E::PacketAuth
        | E::WrongServerSig
        | E::StrictKeyExchangeViolation { .. }
        | E::Inconsistent
        | E::UnknownAlgo => (
            ErrorCategory::Transport,
            "ssh_handshake_failed",
            "handshake",
            "SSH handshake failed",
        ),
        E::IO(_) => (
            ErrorCategory::Transport,
            "ssh_tcp_connect_failed",
            "tcp",
            "SSH TCP connection failed",
        ),
        E::Disconnect | E::HUP | E::SendError | E::RecvError => (
            ErrorCategory::Transport,
            "ssh_connection_closed",
            "connect",
            "SSH connection closed during connect",
        ),
        E::NoAuthMethod => (
            ErrorCategory::Domain,
            "ssh_auth_unsupported",
            "auth",
            "SSH server offered no supported authentication method",
        ),
        _ => (
            ErrorCategory::Transport,
            "ssh_connect_failed",
            "connect",
            "SSH connection failed",
        ),
    };
    session_error(category, code, stage, message)
}

struct HostKey {
    expected: Option<String>,
    observed: Arc<Mutex<Option<String>>>,
    mismatch: Arc<AtomicBool>,
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
        let accepted = self
            .expected
            .as_ref()
            .is_none_or(|expected| expected == &fingerprint);
        if !accepted {
            self.mismatch.store(true, Ordering::Release);
        }
        Ok(accepted)
    }
}

async fn handshake(
    config: &Config,
    expected: Option<String>,
) -> Result<(client::Handle<HostKey>, String)> {
    let observed = Arc::new(Mutex::new(None));
    let mismatch = Arc::new(AtomicBool::new(false));
    let handler = HostKey {
        expected,
        observed: observed.clone(),
        mismatch: mismatch.clone(),
    };
    let transport = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let connection = client::connect(transport, (config.host.as_str(), config.port), handler)
        .await
        .map_err(|error| {
            let observed = mismatch
                .load(Ordering::Acquire)
                .then(|| observed.try_lock().ok().and_then(|value| value.clone()))
                .flatten();
            connect_error(error, observed)
        })?;
    let fingerprint = observed
        .lock()
        .await
        .clone()
        .ok_or_else(|| session_error(
            ErrorCategory::Transport,
            "ssh_host_key_missing",
            "host_key",
            "SSH server provided no host key",
        ))?;
    Ok((connection, fingerprint))
}

pub async fn probe(config: &Config) -> Result<String> {
    let (connection, fingerprint) = timeout(CONNECT_TIMEOUT, handshake(config, None))
        .await
        .map_err(|_| {
            session_error(
                ErrorCategory::Transport,
                "ssh_connect_timeout",
                "connect",
                "SSH host key probe timed out",
            )
        })??;
    let _ = connection
        .disconnect(Disconnect::ByApplication, "Probe complete", "en")
        .await;
    Ok(fingerprint)
}

/// Bounded byte stream with a monotonic cursor. Used for terminal replay and exec jobs.
pub struct OutputBuffer {
    chunks: VecDeque<(u64, Vec<u8>)>,
    bytes: usize,
    sequence: u64,
    limit: usize,
    total: usize,
    dropped: usize,
    pub closed: bool,
}

impl Default for OutputBuffer {
    fn default() -> Self {
        Self::new(MAX_OUTPUT_BYTES)
    }
}

impl OutputBuffer {
    pub fn new(limit: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            bytes: 0,
            sequence: 0,
            limit,
            total: 0,
            dropped: 0,
            closed: false,
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        for part in data.chunks(OUTPUT_CHUNK_BYTES) {
            self.sequence += 1;
            self.total += part.len();
            self.chunks.push_back((self.sequence, part.to_vec()));
            self.bytes += part.len();
            while self.bytes > self.limit {
                if let Some((_, data)) = self.chunks.pop_front() {
                    self.bytes -= data.len();
                    self.dropped += data.len();
                }
            }
        }
    }

    fn collect(&self, after: u64, limit: usize) -> Result<(Vec<u8>, u64, bool, bool)> {
        if after > self.sequence {
            bail!("Output cursor is ahead of the connection");
        }
        let first = self
            .chunks
            .front()
            .map_or(self.sequence + 1, |chunk| chunk.0);
        let gap = after.saturating_add(1) < first;
        let mut next = if gap { first - 1 } else { after };
        let mut data = Vec::with_capacity(limit.min(READ_BYTES));
        for (sequence, part) in &self.chunks {
            if *sequence <= next {
                continue;
            }
            if data.len() + part.len() > limit {
                break;
            }
            data.extend_from_slice(part);
            next = *sequence;
        }
        Ok((data, next, gap, next < self.sequence))
    }

    pub fn read(&self, after: u64) -> Result<Value> {
        let (data, next, gap, more) = self.collect(after, READ_BYTES)?;
        Ok(json!({"data": STANDARD.encode(data), "cursor": next.to_string(), "gap": gap, "closed": self.closed, "more": more}))
    }

    /// Reads a job stream with a readable representation when the bytes are safe text.
    pub fn read_encoded(&self, after: u64) -> Result<Value> {
        self.read_encoded_limit(after, READ_BYTES)
    }

    pub fn read_encoded_limit(&self, after: u64, limit: usize) -> Result<Value> {
        let (data, next, gap, more) = self.collect(after, limit)?;
        let (encoding, field, value) = crate::command::encode_output(&data, false);
        Ok(json!({field: value, "encoding": encoding, "cursor": next.to_string(), "gap": gap, "closed": self.closed, "more": more}))
    }

    /// First `limit` retained bytes with explicit loss accounting.
    pub fn preview(&self, limit: usize) -> Value {
        let mut data = Vec::new();
        for (_, part) in &self.chunks {
            if data.len() >= limit {
                break;
            }
            let take = (limit - data.len()).min(part.len());
            data.extend_from_slice(&part[..take]);
        }
        let (encoding, field, value) = crate::command::encode_output(&data, false);
        let dropped = self.dropped + self.bytes.saturating_sub(data.len());
        json!({
            field: value,
            "encoding": encoding,
            "bytes": data.len(),
            "retainedBytes": self.bytes,
            "originalBytes": self.total,
            "droppedBytes": dropped,
            "truncated": dropped > 0,
            "limit": limit,
        })
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

struct Job {
    stdout: OutputBuffer,
    stderr: OutputBuffer,
    done: bool,
    exit_code: Option<i32>,
    signal: Option<String>,
    error: Option<RuntimeError>,
    cancelled: bool,
    timed_out: bool,
    started: Instant,
    cancel: watch::Sender<bool>,
    cancel_reply: Option<oneshot::Sender<Result<Value>>>,
}

/// Terminal facts about one exec job, kept separately from the cancel reply.
struct JobResult {
    stdout: OutputBuffer,
    stderr: OutputBuffer,
    exit_code: Option<i32>,
    signal: Option<String>,
    error: Option<RuntimeError>,
    cancelled: bool,
    timed_out: bool,
}

#[derive(Default)]
pub struct JobRegistry {
    jobs: HashMap<String, Job>,
    order: VecDeque<String>,
}

impl JobRegistry {
    fn insert(&mut self, operation_id: &str, cancel: watch::Sender<bool>) -> Result<()> {
        while self.jobs.len() >= MAX_EXEC_JOBS {
            let evicted = self
                .order
                .iter()
                .find(|id| self.jobs.get(*id).is_some_and(|job| job.done))
                .cloned();
            let Some(evicted) = evicted else {
                bail!("Too many running SSH jobs; cancel or wait for one to finish");
            };
            self.remove(&evicted);
        }
        self.jobs.insert(
            operation_id.to_owned(),
            Job {
                stdout: OutputBuffer::new(EXEC_OUTPUT_LIMIT),
                stderr: OutputBuffer::new(EXEC_OUTPUT_LIMIT),
                done: false,
                exit_code: None,
                signal: None,
                error: None,
                cancelled: false,
                timed_out: false,
                started: Instant::now(),
                cancel,
                cancel_reply: None,
            },
        );
        self.order.push_back(operation_id.to_owned());
        Ok(())
    }

    fn remove(&mut self, operation_id: &str) {
        self.jobs.remove(operation_id);
        self.order.retain(|id| id != operation_id);
    }

    fn finish(
        &mut self,
        operation_id: &str,
        result: JobResult,
        cancel_reply: Option<oneshot::Sender<Result<Value>>>,
    ) -> Option<oneshot::Sender<Result<Value>>> {
        let job = self.jobs.get_mut(operation_id)?;
        job.stdout = result.stdout;
        job.stderr = result.stderr;
        job.done = true;
        job.exit_code = result.exit_code;
        job.signal = result.signal;
        job.error = result.error;
        job.cancelled = result.cancelled;
        job.timed_out = result.timed_out;
        let _ = cancel_reply;
        job.cancel_reply.take()
    }

    fn knows(&self, operation_id: &str) -> bool {
        self.jobs.contains_key(operation_id)
    }

    fn request_cancel(
        &mut self,
        operation_id: &str,
        reply: oneshot::Sender<Result<Value>>,
    ) -> bool {
        let Some(job) = self.jobs.get_mut(operation_id) else {
            return false;
        };
        if job.done {
            let _ = reply.send(Ok(json!({"outcome":"not_submitted"})));
            return true;
        }
        job.cancel_reply = Some(reply);
        let _ = job.cancel.send(true);
        true
    }

    pub fn read(
        &self,
        operation_id: &str,
        stdout_cursor: u64,
        stderr_cursor: u64,
    ) -> Result<Value> {
        let job = self.jobs.get(operation_id).ok_or_else(|| {
            anyhow!("SSH output is unavailable for this operation; use ssh.read for terminal commands")
        })?;
        let mut result = json!({
            "operationId": operation_id,
            "done": job.done,
            "exitCode": job.exit_code,
            "signal": job.signal,
            "cancelled": job.cancelled,
            "timedOut": job.timed_out,
            "elapsedMs": job.started.elapsed().as_millis() as u64,
            "stdout": job.stdout.read_encoded(stdout_cursor)?,
            "stderr": job.stderr.read_encoded(stderr_cursor)?,
        });
        if let Some(error) = &job.error {
            result["error"] = json!(error);
        }
        Ok(result)
    }

    /// Bounded preview embedded in the completed operation result; full streams stay
    /// available through `ssh.output`.
    pub fn read_preview(&self, operation_id: &str) -> Option<Value> {
        let job = self.jobs.get(operation_id)?;
        Some(json!({
            "stdout": job.stdout.preview(EXEC_RESULT_LIMIT),
            "stderr": job.stderr.preview(EXEC_RESULT_LIMIT),
        }))
    }
}

enum Input {
    Data(Vec<u8>),
    Resize(u32, u32),
    Command(Command, Vec<u8>),
    Exec {
        operation_id: String,
        mcp: bool,
        text: String,
    },
    Cancel(String),
    Reset,
    Close,
}

struct Action {
    input: Input,
    reply: oneshot::Sender<Result<Value>>,
}

struct PendingCancel {
    operation_id: String,
    reply: oneshot::Sender<Result<Value>>,
}

pub struct Connection {
    actions: mpsc::Sender<Action>,
    pub output: Arc<Mutex<OutputBuffer>>,
    pub jobs: Arc<Mutex<JobRegistry>>,
    handle: Arc<client::Handle<HostKey>>,
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
        let (handle, read, write, output, jobs, id) = timeout(CONNECT_TIMEOUT, async {
            let (mut handle, _) = handshake(config, Some(fingerprint)).await?;
            let auth = handle
                .authenticate_password(config.username.clone(), password)
                .await
                .map_err(|_| {
                    session_error(
                        ErrorCategory::Transport,
                        "ssh_auth_failed",
                        "auth",
                        "SSH authentication failed",
                    )
                })?;
            if !auth.success() {
                return Err(session_error(
                    ErrorCategory::Domain,
                    "ssh_auth_rejected",
                    "auth",
                    "SSH password authentication was rejected",
                ));
            }
            let handle = Arc::new(handle);
            let output = Arc::new(Mutex::new(OutputBuffer::default()));
            let (read, write) = open_shell(&handle, &output).await?;
            Ok::<_, anyhow::Error>((handle, read, write, output, Arc::new(Mutex::new(JobRegistry::default())), uuid::Uuid::new_v4().to_string()))
        })
        .await
        .map_err(|_| {
            session_error(
                ErrorCategory::Transport,
                "ssh_connect_timeout",
                "connect",
                "SSH connect, authentication or terminal setup timed out",
            )
        })??;
        let (actions, mut receiver) = mpsc::channel::<Action>(64);
        let task_output = output.clone();
        let task_jobs = jobs.clone();
        let task_handle = handle.clone();
        let connection_id = id.clone();
        let task = tokio::spawn(async move {
            let mut read = read;
            let mut write = write;
            let mut active: Option<Command> = None;
            let mut pending_cancel: Option<PendingCancel> = None;
            let mut cancel_deadline: Option<Instant> = None;
            let mut shell_exit: Option<u32> = None;
            let mut shell_signal: Option<String> = None;
            let mut exec_tasks = JoinSet::new();
            loop {
                let command_deadline = active
                    .as_ref()
                    .filter(|command| !command.timed_out)
                    .map(|command| command.deadline);
                tokio::select! {
                    action = receiver.recv() => {
                        let Some(action) = action else { break; };
                        match action.input {
                            Input::Command(command, data) => {
                                if active.is_some() {
                                    let _ = action.reply.send(Err(anyhow!(
                                        "A command is still running or its completion is unknown; reset the terminal to recover"
                                    )));
                                    continue;
                                }
                                let result = timeout(IO_TIMEOUT, write.data(&data[..])).await;
                                match result {
                                    Ok(Ok(())) => {
                                        active = Some(command);
                                        let _ = action.reply.send(Ok(json!({})));
                                    }
                                    _ => {
                                        let _ = action.reply.send(Err(anyhow!(
                                            "SSH input was not confirmed; connection closed"
                                        )));
                                        break;
                                    }
                                }
                            }
                            Input::Exec { operation_id, mcp, text } => {
                                let (cancel, cancel_rx) = watch::channel(false);
                                {
                                    let mut jobs = task_jobs.lock().await;
                                    if let Err(error) = jobs.insert(&operation_id, cancel) {
                                        let _ = action.reply.send(Err(error));
                                        continue;
                                    }
                                }
                                let command_text = text.clone();
                                exec_tasks.spawn(run_exec(
                                    task_handle.clone(),
                                    task_jobs.clone(),
                                    events.clone(),
                                    connection_id.clone(),
                                    operation_id,
                                    text,
                                    mcp,
                                    cancel_rx,
                                    command_text,
                                ));
                                let _ = action.reply.send(Ok(json!({})));
                            }
                            Input::Cancel(operation_id) => {
                                let matches_active = active
                                    .as_ref()
                                    .is_some_and(|command| command.operation_id == operation_id);
                                if matches_active {
                                    if pending_cancel.is_some() {
                                        let _ = action.reply.send(Err(anyhow!(
                                            "Another SSH cancellation is already pending"
                                        )));
                                        continue;
                                    }
                                    match timeout(IO_TIMEOUT, write.data(&[3][..])).await {
                                        Ok(Ok(())) => {
                                            pending_cancel = Some(PendingCancel {
                                                operation_id,
                                                reply: action.reply,
                                            });
                                            cancel_deadline = Some(Instant::now() + CANCEL_GRACE);
                                        }
                                        _ => {
                                            let _ = action.reply.send(Err(anyhow!(
                                                "SSH interrupt delivery failed"
                                            )));
                                        }
                                    }
                                    continue;
                                }
                                let mut jobs = task_jobs.lock().await;
                                if jobs.knows(&operation_id) {
                                    jobs.request_cancel(&operation_id, action.reply);
                                    continue;
                                }
                                drop(jobs);
                                let _ = action
                                    .reply
                                    .send(Ok(json!({"outcome":"unsupported"})));
                            }
                            Input::Reset => {
                                let reply = action.reply;
                                match reset_terminal(
                                    &task_handle,
                                    &task_output,
                                    &events,
                                    &connection_id,
                                    &mut read,
                                    &mut write,
                                    &mut active,
                                    &mut pending_cancel,
                                    &mut cancel_deadline,
                                )
                                .await
                                {
                                    Ok(value) => {
                                        shell_exit = None;
                                        shell_signal = None;
                                        let _ = reply.send(Ok(value));
                                    }
                                    Err(error) => {
                                        let _ = reply.send(Err(error));
                                        break;
                                    }
                                }
                            }
                            Input::Close => {
                                if let Some(command) = active.take() {
                                    if let Some(pending) = pending_cancel.take() {
                                        let _ = pending
                                            .reply
                                            .send(Ok(json!({"outcome":"unknown"})));
                                    }
                                    fail_command(
                                        &events,
                                        &command,
                                        "ssh_disconnected_before_completion",
                                        "SSH disconnected before the command completed",
                                        None,
                                    )
                                    .await;
                                }
                                let _ = action.reply.send(Ok(json!({"connected":false})));
                                break;
                            }
                            Input::Data(data) => {
                                let result = timeout(IO_TIMEOUT, write.data(&data[..])).await;
                                let failed = !matches!(result, Ok(Ok(())));
                                let _ = action.reply.send(if failed {
                                    Err(anyhow!("SSH input was not confirmed; connection closed"))
                                } else {
                                    Ok(json!({}))
                                });
                                if failed {
                                    break;
                                }
                            }
                            Input::Resize(cols, rows) => {
                                let result =
                                    timeout(IO_TIMEOUT, write.window_change(cols, rows, 0, 0)).await;
                                let failed = !matches!(result, Ok(Ok(())));
                                let _ = action.reply.send(if failed {
                                    Err(anyhow!("SSH resize was not confirmed; connection closed"))
                                } else {
                                    Ok(json!({}))
                                });
                                if failed {
                                    break;
                                }
                            }
                        }
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
                                    let cancelled = pending_cancel
                                        .as_ref()
                                        .is_some_and(|pending| pending.operation_id == command.operation_id);
                                    if let Some(pending) = pending_cancel.take() {
                                        if pending.operation_id == command.operation_id {
                                            cancel_deadline = None;
                                            let _ = pending
                                                .reply
                                                .send(Ok(json!({"outcome":"stopped"})));
                                        } else {
                                            let _ = pending.reply.send(Ok(json!({"outcome":"unknown"})));
                                        }
                                    }
                                    if !command.timed_out && !cancelled {
                                        emit_completion(&events, &command, result).await;
                                    }
                                }
                            }
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => shell_exit = Some(exit_status),
                        Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                            shell_signal = Some(format!("{signal_name:?}"));
                        }
                        Some(ChannelMsg::Close) | None => break,
                        _ => {}
                    },
                    _ = sleep_until(command_deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(86400))), if command_deadline.is_some() => {
                        if let Some(command) = active.take() {
                            fail_command(
                                &events,
                                &command,
                                "ssh_command_completion_unknown",
                                "SSH command did not complete within 120 seconds; the terminal was reset",
                                Some(json!({"timeoutMs": 120_000, "terminalReset": true})),
                            )
                            .await;
                            match timeout(
                                CONNECT_TIMEOUT,
                                reopen_shell(&task_handle, &task_output, &mut read, &mut write),
                            )
                            .await
                            {
                                Ok(Ok(())) => {
                                    shell_exit = None;
                                    shell_signal = None;
                                    emit_status(
                                        &events,
                                        &connection_id,
                                        true,
                                        true,
                                        &task_output,
                                    )
                                    .await;
                                }
                                _ => break,
                            }
                        }
                    },
                    _ = exec_tasks.join_next(), if !exec_tasks.is_empty() => {}
                    _ = sleep_until(cancel_deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(86400))), if cancel_deadline.is_some() => {
                        cancel_deadline = None;
                        if let Some(pending) = pending_cancel.take() {
                            let _ = pending
                                .reply
                                .send(Ok(json!({"outcome":"unknown","terminalReset":true})));
                        }
                        if let Some(command) = active.take() {
                            fail_command(
                                &events,
                                &command,
                                "ssh_command_aborted_by_cancel_timeout",
                                "Cancellation was not confirmed; the terminal was reset",
                                None,
                            )
                            .await;
                        }
                        match timeout(
                            CONNECT_TIMEOUT,
                            reopen_shell(&task_handle, &task_output, &mut read, &mut write),
                        )
                        .await
                        {
                            Ok(Ok(())) => {
                                shell_exit = None;
                                shell_signal = None;
                                emit_status(&events, &connection_id, true, true, &task_output).await;
                            }
                            _ => break,
                        }
                    }
                }
            }
            exec_tasks.abort_all();
            let closed_by_transport = task_handle.is_closed();
            if let Some(mut command) = active {
                push_output(&task_output, &events, &connection_id, &command.remaining()).await;
                if let Some(pending) = pending_cancel.take() {
                    let _ = pending.reply.send(Ok(json!({"outcome":"unknown"})));
                }
                if !command.timed_out {
                    let (code, message, details) = if closed_by_transport {
                        (
                            "ssh_connection_lost_before_completion",
                            "SSH connection was lost before the command completed; no command was replayed",
                            None,
                        )
                    } else {
                        (
                            "ssh_remote_shell_exited",
                            "Remote shell exited before the command completed",
                            Some(json!({"exitStatus": shell_exit, "signal": shell_signal})),
                        )
                    };
                    fail_command(&events, &command, code, message, details).await;
                }
            }
            task_output.lock().await.closed = true;
            if let Some((instance, sender)) = &events {
                let cursor = task_output.lock().await.sequence().to_string();
                let _ = sender.send(PluginNotification::Event {
                    plugin_instance_id: instance.clone(),
                    name: "ssh.status".into(),
                    payload: json!({"connected":false,"connectionId":connection_id,"cursor":cursor}),
                }).await;
            }
            let _ = timeout(
                IO_TIMEOUT,
                task_handle.disconnect(Disconnect::ByApplication, "Session closed", "en"),
            )
            .await;
        });
        Ok(Self {
            actions,
            output,
            jobs,
            handle,
            task,
            id,
        })
    }

    async fn submit(&self, input: Input) -> Result<Value> {
        let (reply, response) = oneshot::channel();
        self.actions
            .try_send(Action { input, reply })
            .map_err(|_| anyhow!("SSH input queue is full or closed"))?;
        timeout(Duration::from_secs(10), response)
            .await
            .map_err(|_| anyhow!("SSH input wait timed out; delivery may be unknown"))?
            .map_err(|_| anyhow!("SSH connection closed before input acknowledgement"))?
    }

    pub async fn input(&self, data: Vec<u8>) -> Result<()> {
        if data.is_empty() || data.len() > 16 * 1024 {
            bail!("SSH input must contain 1 to 16384 bytes");
        }
        self.submit(Input::Data(data)).await?;
        Ok(())
    }

    pub async fn command(&self, operation_id: String, mcp: bool, text: &str) -> Result<()> {
        let (command, wire) = Command::new(operation_id, mcp, text);
        if wire.len() > 4000 {
            bail!("Encoded command exceeds the shared PTY line limit; shorten the command");
        }
        self.submit(Input::Command(command, wire)).await?;
        Ok(())
    }

    /// Starts an isolated one-shot exec channel. The operation completes by notification.
    pub async fn exec(&self, operation_id: String, mcp: bool, text: String) -> Result<()> {
        self.submit(Input::Exec {
            operation_id,
            mcp,
            text,
        })
        .await?;
        Ok(())
    }

    /// Cancels a PTY command or exec job. `outcome` is `stopped`, `unknown`, or
    /// `not_submitted`; an unconfirmed PTY cancel resets the terminal instead of blocking it.
    pub async fn cancel(&self, operation_id: String) -> Result<Value> {
        self.submit(Input::Cancel(operation_id)).await
    }

    /// Replaces the shared PTY shell channel. The SSH transport and exec jobs survive.
    pub async fn reset(&self) -> Result<Value> {
        self.submit(Input::Reset).await
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.submit(Input::Close).await?;
        Ok(())
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        if cols == 0 || rows == 0 || cols > 1000 || rows > 1000 {
            bail!("Invalid terminal dimensions");
        }
        self.submit(Input::Resize(cols, rows)).await?;
        Ok(())
    }

    /// Runs a fixed read-only script on an isolated exec channel, outside the shared PTY.
    pub async fn query(&self, script: &str) -> Result<QueryResult> {
        if self.is_closed() {
            bail!("SSH is not connected");
        }
        timeout(QUERY_TIMEOUT, query_inner(self.handle.clone(), script))
            .await
            .map_err(|_| {
                session_error(
                    ErrorCategory::Transport,
                    "ssh_query_timeout",
                    "exec",
                    "SSH read-only query timed out",
                )
            })?
    }
}

pub struct QueryResult {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

async fn query_inner(handle: Arc<client::Handle<HostKey>>, script: &str) -> Result<QueryResult> {
    let mut channel = handle.channel_open_session().await.map_err(|_| {
        session_error(
            ErrorCategory::Transport,
            "ssh_channel_failed",
            "exec",
            "SSH could not open an exec channel",
        )
    })?;
    channel.exec(true, script.as_bytes()).await.map_err(|_| {
        session_error(
            ErrorCategory::Transport,
            "ssh_exec_failed",
            "exec",
            "SSH could not start the query",
        )
    })?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;
    let mut timed_out = false;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    loop {
        tokio::select! {
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data }) => {
                    if stdout.len() < QUERY_LIMIT {
                        let take = (QUERY_LIMIT - stdout.len()).min(data.len());
                        stdout.extend_from_slice(&data[..take]);
                    }
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if stderr.len() < QUERY_LIMIT {
                        let take = (QUERY_LIMIT - stderr.len()).min(data.len());
                        stderr.extend_from_slice(&data[..take]);
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => exit_code = Some(exit_status as i32),
                Some(ChannelMsg::Failure) => {
                    bail!("SSH server refused the read-only query");
                }
                Some(ChannelMsg::Eof) => {}
                Some(ChannelMsg::Close) | None => break,
                _ => {}
            },
            _ = sleep_until(deadline) => {
                timed_out = true;
                let _ = channel.close().await;
                break;
            }
        }
    }
    Ok(QueryResult {
        exit_code,
        stdout,
        stderr,
        timed_out,
    })
}

async fn open_shell(
    handle: &Arc<client::Handle<HostKey>>,
    output: &Arc<Mutex<OutputBuffer>>,
) -> Result<(russh::ChannelReadHalf, russh::ChannelWriteHalf<client::Msg>)> {
    let mut channel = handle.channel_open_session().await.map_err(|_| {
        session_error(
            ErrorCategory::Transport,
            "ssh_channel_failed",
            "channel",
            "SSH could not open a session channel",
        )
    })?;
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .map_err(|_| {
            session_error(
                ErrorCategory::Domain,
                "ssh_pty_refused",
                "pty",
                "SSH server refused a pseudo-terminal",
            )
        })?;
    wait_success(&mut channel, output, "pty").await?;
    channel.request_shell(true).await.map_err(|_| {
        session_error(
            ErrorCategory::Domain,
            "ssh_shell_refused",
            "shell",
            "SSH server refused an interactive shell",
        )
    })?;
    wait_success(&mut channel, output, "shell").await?;
    Ok(channel.split())
}

async fn reopen_shell(
    handle: &Arc<client::Handle<HostKey>>,
    output: &Arc<Mutex<OutputBuffer>>,
    read: &mut russh::ChannelReadHalf,
    write: &mut russh::ChannelWriteHalf<client::Msg>,
) -> Result<()> {
    let (new_read, new_write) = open_shell(handle, output).await?;
    *read = new_read;
    *write = new_write;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn reset_terminal(
    handle: &Arc<client::Handle<HostKey>>,
    output: &Arc<Mutex<OutputBuffer>>,
    events: &Option<(String, mpsc::Sender<PluginNotification>)>,
    connection_id: &str,
    read: &mut russh::ChannelReadHalf,
    write: &mut russh::ChannelWriteHalf<client::Msg>,
    active: &mut Option<Command>,
    pending_cancel: &mut Option<PendingCancel>,
    cancel_deadline: &mut Option<Instant>,
) -> Result<Value> {
    if let Some(pending) = pending_cancel.take() {
        let _ = pending.reply.send(Ok(json!({"outcome":"unknown"})));
    }
    *cancel_deadline = None;
    if let Some(command) = active.take() {
        fail_command(
            events,
            &command,
            "ssh_command_aborted_by_terminal_reset",
            "SSH terminal was reset before the command completed",
            None,
        )
        .await;
    }
    timeout(CONNECT_TIMEOUT, reopen_shell(handle, output, read, write))
        .await
        .map_err(|_| {
            session_error(
                ErrorCategory::Transport,
                "ssh_terminal_reset_timeout",
                "pty",
                "SSH terminal reset timed out; reconnect the instance",
            )
        })??;
    emit_status(events, connection_id, true, true, output).await;
    Ok(json!({"connected":true,"reset":true,"connectionId":connection_id}))
}

async fn wait_success(
    channel: &mut russh::Channel<client::Msg>,
    output: &Arc<Mutex<OutputBuffer>>,
    stage: &str,
) -> Result<()> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Data { data }) => output.lock().await.push(&data),
            Some(ChannelMsg::Failure | ChannelMsg::Close) | None => {
                let (code, message) = if stage == "pty" {
                    ("ssh_pty_refused", "SSH server refused a pseudo-terminal")
                } else {
                    ("ssh_shell_refused", "SSH server refused an interactive shell")
                };
                return Err(session_error(ErrorCategory::Domain, code, stage, message));
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
            output.sequence().to_string()
        };
        if let Some((instance, sender)) = events {
            if sender
                .send(PluginNotification::Event {
                    plugin_instance_id: instance.clone(),
                    name: "ssh.terminal.output".into(),
                    payload: json!({"connectionId":connection_id,"data":STANDARD.encode(part),"cursor":cursor}),
                })
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

async fn emit_status(
    events: &Option<(String, mpsc::Sender<PluginNotification>)>,
    connection_id: &str,
    connected: bool,
    reset: bool,
    output: &Arc<Mutex<OutputBuffer>>,
) {
    if let Some((instance, sender)) = events {
        let cursor = output.lock().await.sequence().to_string();
        let _ = sender
            .send(PluginNotification::Event {
                plugin_instance_id: instance.clone(),
                name: "ssh.status".into(),
                payload: json!({
                    "connected": connected,
                    "connectionId": connection_id,
                    "cursor": cursor,
                    "reset": reset,
                }),
            })
            .await;
    }
}

async fn emit_completion(
    events: &Option<(String, mpsc::Sender<PluginNotification>)>,
    command: &Command,
    result: Value,
) {
    if let Some((_, sender)) = events {
        let result = if command.mcp {
            tool_result(&result)
        } else {
            result
        };
        let _ = sender
            .send(PluginNotification::OperationCompleted {
                operation_id: command.operation_id.clone(),
                result,
            })
            .await;
    }
}

async fn fail_command(
    events: &Option<(String, mpsc::Sender<PluginNotification>)>,
    command: &Command,
    code: &str,
    message: &str,
    details: Option<Value>,
) {
    if let Some((_, sender)) = events {
        let _ = sender
            .send(PluginNotification::OperationFailed {
                operation_id: command.operation_id.clone(),
                error: RuntimeError {
                    category: ErrorCategory::Domain,
                    code: code.to_owned(),
                    message: message.to_owned(),
                    details,
                },
            })
            .await;
    }
}

async fn fail_operation(
    events: &Option<(String, mpsc::Sender<PluginNotification>)>,
    operation_id: &str,
    code: &str,
    message: &str,
    details: Option<Value>,
) {
    if let Some((_, sender)) = events {
        let _ = sender
            .send(PluginNotification::OperationFailed {
                operation_id: operation_id.to_owned(),
                error: RuntimeError {
                    category: ErrorCategory::Domain,
                    code: code.to_owned(),
                    message: message.to_owned(),
                    details,
                },
            })
            .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_exec(
    handle: Arc<client::Handle<HostKey>>,
    jobs: Arc<Mutex<JobRegistry>>,
    events: Option<(String, mpsc::Sender<PluginNotification>)>,
    connection_id: String,
    operation_id: String,
    text: String,
    mcp: bool,
    mut cancel: watch::Receiver<bool>,
    command_text: String,
) {
    // Announce the job before it starts so the terminal can show a running tab.
    emit_job(&events, &connection_id, &operation_id, &command_text, false).await;
    let started = Instant::now();
    let mut stdout = OutputBuffer::new(EXEC_OUTPUT_LIMIT);
    let mut stderr = OutputBuffer::new(EXEC_OUTPUT_LIMIT);
    let mut exit_code = None;
    let mut signal = None;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut error: Option<RuntimeError> = None;
    let mut channel = match handle.channel_open_session().await {
        Ok(channel) => Some(channel),
        Err(_) => {
            error = Some(RuntimeError {
                category: ErrorCategory::Transport,
                code: "ssh_channel_failed".to_owned(),
                message: "SSH could not open an exec channel".to_owned(),
                details: Some(json!({"stage":"exec"})),
            });
            None
        }
    };
    if let Some(channel) = channel.as_mut() {
        if channel.exec(true, text.as_bytes()).await.is_err() {
            error = Some(RuntimeError {
                category: ErrorCategory::Transport,
                code: "ssh_exec_failed".to_owned(),
                message: "SSH could not start the command".to_owned(),
                details: Some(json!({"stage":"exec"})),
            });
        }
    }
    if let (None, Some(channel)) = (&error, channel.as_mut()) {
        let deadline = Instant::now() + EXEC_MAX_DURATION;
        loop {
            tokio::select! {
                message = channel.wait() => match message {
                    Some(ChannelMsg::Data { data }) => stdout.push(&data),
                    Some(ChannelMsg::ExtendedData { data, .. }) => stderr.push(&data),
                    Some(ChannelMsg::ExitStatus { exit_status }) => exit_code = Some(exit_status as i32),
                    Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                        signal = Some(format!("{signal_name:?}"));
                    }
                    Some(ChannelMsg::Failure) => {
                        error = Some(RuntimeError {
                            category: ErrorCategory::Domain,
                            code: "ssh_exec_refused".to_owned(),
                            message: "SSH server refused the command".to_owned(),
                            details: Some(json!({"stage":"exec"})),
                        });
                        break;
                    }
                    Some(ChannelMsg::Eof) => {}
                    Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                _ = cancel.changed() => {
                    cancelled = true;
                    let _ = channel.close().await;
                    let _ = timeout(CANCEL_GRACE, drain_until_close(channel)).await;
                    break;
                }
                _ = sleep_until(deadline) => {
                    timed_out = true;
                    let _ = channel.close().await;
                    let _ = timeout(CANCEL_GRACE, drain_until_close(channel)).await;
                    break;
                }
            }
        }
    }
    let mut cancel_reply = {
        let mut jobs = jobs.lock().await;
        jobs.finish(
            &operation_id,
            JobResult {
                stdout,
                stderr,
                exit_code,
                signal: signal.clone(),
                error: error.clone(),
                cancelled,
                timed_out,
            },
            None,
        )
    };
    if let Some(reply) = cancel_reply.take() {
        let outcome = if cancelled { "stopped" } else { "unknown" };
        let _ = reply.send(Ok(json!({"outcome": outcome})));
    }
    let job_snapshot = {
        let jobs = jobs.lock().await;
        jobs.read_preview(&operation_id)
    };
    let (stdout_preview, stderr_preview) = job_snapshot
        .as_ref()
        .map(|value| (value["stdout"].clone(), value["stderr"].clone()))
        .unwrap_or((Value::Null, Value::Null));
    let domain = json!({
        "exitCode": exit_code,
        "signal": signal,
        "timedOut": timed_out,
        "cancelled": cancelled,
        "stdout": stdout_preview,
        "stderr": stderr_preview,
        "durationMs": started.elapsed().as_millis() as u64,
        "outputOperationId": operation_id,
    });
    if cancelled {
        emit_job(&events, &connection_id, &operation_id, &command_text, true).await;
        return;
    }
    if let Some(error) = error {
        fail_operation(
            &events,
            &operation_id,
            &error.code,
            &error.message,
            error.details,
        )
        .await;
        return;
    }
    if timed_out {
        emit_job(&events, &connection_id, &operation_id, &command_text, true).await;
        fail_operation(
            &events,
            &operation_id,
            "ssh_command_timeout",
            "SSH command exceeded the job time limit and its channel was closed",
            Some(json!({"timeoutMs": EXEC_MAX_DURATION.as_millis() as u64})),
        )
        .await;
        return;
    }
    emit_job(&events, &connection_id, &operation_id, &command_text, true).await;
    if let Some((_, sender)) = events {
        let result = if mcp { tool_result(&domain) } else { domain };
        let _ = sender
            .send(PluginNotification::OperationCompleted {
                operation_id,
                result,
            })
            .await;
    }
}

/// Publishes the job list entry for an exec operation. It carries no output; the
/// terminal reads output through `ssh.output` with cursors.
async fn emit_job(
    events: &Option<(String, mpsc::Sender<PluginNotification>)>,
    connection_id: &str,
    operation_id: &str,
    text: &str,
    done: bool,
) {
    if let Some((instance, sender)) = events {
        let _ = sender
            .send(PluginNotification::Event {
                plugin_instance_id: instance.clone(),
                name: "ssh.job".into(),
                payload: json!({
                    "connectionId": connection_id,
                    "operationId": operation_id,
                    "text": text,
                    "done": done,
                }),
            })
            .await;
    }
}

async fn drain_until_close(channel: &mut russh::Channel<client::Msg>) {
    while let Some(message) = channel.wait().await {
        if matches!(message, ChannelMsg::Close) {
            break;
        }
    }
}

/// Fixed read-only system information. No user input is interpolated.
pub const SYSTEM_INFO_SCRIPT: &str = concat!(
    "printf 'user=%s\\n' \"$(id -un 2>/dev/null)\";",
    "printf 'uid=%s\\n' \"$(id -u 2>/dev/null)\";",
    "printf 'gid=%s\\n' \"$(id -g 2>/dev/null)\";",
    "printf 'groups=%s\\n' \"$(id -Gn 2>/dev/null)\";",
    "printf 'hostname=%s\\n' \"$(uname -n 2>/dev/null)\";",
    "printf 'kernel=%s\\n' \"$(uname -sr 2>/dev/null)\";",
    "printf 'arch=%s\\n' \"$(uname -m 2>/dev/null)\";",
    "printf 'shell=%s\\n' \"${SHELL:-}\";",
    "printf 'home=%s\\n' \"${HOME:-}\";",
    "printf 'cwd=%s\\n' \"$(pwd 2>/dev/null)\";",
    "printf 'os=%s\\n' \"$(. /etc/os-release 2>/dev/null && printf '%s' \"${PRETTY_NAME:-}\")\";",
    "printf 'uptime=%s\\n' \"$(uptime -p 2>/dev/null || uptime 2>/dev/null)\"",
);

/// Metadata-only file query. The path is single-quote escaped before interpolation.
pub fn file_info_script(path: &str) -> String {
    let quoted = path.replace('\'', "'\\''");
    format!(
        concat!(
            "__p='{quoted}';",
            "if [ -e \"$__p\" ] || [ -L \"$__p\" ]; then ",
            "printf 'exists=1\\n';",
            "printf 'type=%s\\n' \"$(stat -c %F -- \"$__p\" 2>/dev/null || stat -f %HT -- \"$__p\" 2>/dev/null)\";",
            "printf 'size=%s\\n' \"$(stat -c %s -- \"$__p\" 2>/dev/null || stat -f %z -- \"$__p\" 2>/dev/null)\";",
            "printf 'mode=%s\\n' \"$(stat -c %a -- \"$__p\" 2>/dev/null || stat -f %Lp -- \"$__p\" 2>/dev/null)\";",
            "printf 'owner=%s\\n' \"$(stat -c %U -- \"$__p\" 2>/dev/null || stat -f %Su -- \"$__p\" 2>/dev/null)\";",
            "printf 'group=%s\\n' \"$(stat -c %G -- \"$__p\" 2>/dev/null || stat -f %Sg -- \"$__p\" 2>/dev/null)\";",
            "printf 'mtime=%s\\n' \"$(stat -c %Y -- \"$__p\" 2>/dev/null || stat -f %m -- \"$__p\" 2>/dev/null)\";",
            "printf 'symlink=%s\\n' \"$(readlink \"$__p\" 2>/dev/null)\";",
            "else printf 'exists=0\\n'; fi"
        ),
        quoted = quoted
    )
}

pub fn parse_fields(bytes: &[u8]) -> Map<String, Value> {
    let text = String::from_utf8_lossy(bytes);
    let mut fields = Map::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(key.trim().to_owned(), Value::String(value.trim().to_owned()));
        }
    }
    fields
}
