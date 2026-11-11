use super::{catalog::prefix, config::Config, proxy};
use crate::{stream::buffer::OutputBuffer, telnet::codec::Codec};
use anyhow::{anyhow, bail, Result};
use coterm_core::driver::DriverNotification;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, oneshot, watch, Mutex},
    task::JoinHandle,
    time::{sleep, timeout},
};

const READ_BATCH_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(crate) struct State {
    pub connected: bool,
    pub reconnecting: bool,
    pub connection_id: Option<String>,
    pub remote_echo: bool,
    pub rx: u64,
    pub tx: u64,
    pub error: Option<String>,
}

enum Input {
    Write {
        epoch: String,
        bytes: Vec<u8>,
        reply: oneshot::Sender<Result<usize>>,
    },
    Resize {
        epoch: String,
        cols: u16,
        rows: u16,
        reply: oneshot::Sender<Result<()>>,
    },
}

#[derive(Clone)]
pub(crate) struct Client {
    pub state: Arc<Mutex<State>>,
    pub output: Arc<Mutex<OutputBuffer>>,
    pub busy: Arc<Mutex<()>>,
    pub interpretation: Arc<Mutex<(crate::stream::shell::Mode, Option<String>)>>,
    input: mpsc::Sender<Input>,
    stopped: watch::Receiver<bool>,
    closed: Arc<AtomicBool>,
}

impl Client {
    pub(crate) async fn epoch(&self) -> Result<String> {
        let state = self.state.lock().await;
        if self.closed.load(Ordering::Acquire) || !state.connected {
            bail!("Connection is not established")
        }
        state
            .connection_id
            .clone()
            .ok_or_else(|| anyhow!("Connection epoch is unavailable"))
    }

    pub(crate) async fn write(&self, epoch: &str, bytes: Vec<u8>) -> Result<usize> {
        if bytes.is_empty() || bytes.len() > 16 * 1024 {
            bail!("A write must contain 1–16384 bytes")
        }
        let (reply, result) = oneshot::channel();
        self.input
            .try_send(Input::Write {
                epoch: epoch.into(),
                bytes,
                reply,
            })
            .map_err(|_| anyhow!("Input queue is full or closed"))?;
        timeout(Duration::from_secs(3), result)
            .await
            .map_err(|_| anyhow!("Write result is unknown; do not replay it"))?
            .map_err(|_| anyhow!("Connection ended; write result is unknown"))?
    }

    pub(crate) async fn resize(&self, epoch: &str, cols: u16, rows: u16) -> Result<()> {
        let (reply, result) = oneshot::channel();
        self.input
            .try_send(Input::Resize {
                epoch: epoch.into(),
                cols,
                rows,
                reply,
            })
            .map_err(|_| anyhow!("Input queue is full or closed"))?;
        timeout(Duration::from_secs(3), result)
            .await
            .map_err(|_| anyhow!("Resize timed out"))?
            .map_err(|_| anyhow!("Connection ended"))?
    }

    pub(crate) fn is_stopped(&self) -> bool {
        *self.stopped.borrow() || self.closed.load(Ordering::Acquire)
    }
}

pub(crate) struct Session {
    pub client: Client,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.task.abort();
    }
}

impl Session {
    pub(crate) fn start(
        config: Config,
        telnet: bool,
        instance: String,
        operation: String,
        mcp: bool,
        events: mpsc::Sender<DriverNotification>,
    ) -> Self {
        let (input, receiver) = mpsc::channel(32);
        let (stop, stopped) = watch::channel(false);
        let client = Client {
            state: Arc::new(Mutex::new(State::default())),
            output: Arc::new(Mutex::new(OutputBuffer::with_shell(
                config.mode,
                config.prompt.as_deref(),
            ))),
            busy: Arc::new(Mutex::new(())),
            interpretation: Arc::new(Mutex::new((config.mode, config.prompt.clone()))),
            input,
            stopped: stopped.clone(),
            closed: Arc::new(AtomicBool::new(false)),
        };
        let worker = Worker {
            config,
            telnet,
            instance,
            operation: Some((operation, mcp)),
            events,
            client: client.clone(),
            receiver,
            stopped,
        };
        let task = tokio::spawn(worker.run());
        Self { client, stop, task }
    }

    pub(crate) fn active(&self) -> bool {
        !self.task.is_finished() && !*self.stop.borrow()
    }

    pub(crate) async fn connecting(&self) -> bool {
        self.active() && self.client.state.lock().await.connection_id.is_none()
    }

    pub(crate) async fn disconnect(&mut self) {
        let _ = self.stop.send(true);
        if !self.task.is_finished()
            && timeout(Duration::from_secs(2), &mut self.task)
                .await
                .is_err()
        {
            self.task.abort();
        }
        self.client.closed.store(true, Ordering::Release);
        self.client.state.lock().await.connected = false;
        self.client.state.lock().await.reconnecting = false;
        self.client.output.lock().await.close();
    }
}

struct Worker {
    config: Config,
    telnet: bool,
    instance: String,
    operation: Option<(String, bool)>,
    events: mpsc::Sender<DriverNotification>,
    client: Client,
    receiver: mpsc::Receiver<Input>,
    stopped: watch::Receiver<bool>,
}

struct Closed(Arc<AtomicBool>);
impl Drop for Closed {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl Worker {
    async fn notify(&self, name: &str, payload: Value) {
        let event = DriverNotification::Event {
            driver_instance_id: self.instance.clone(),
            name: format!("{}.{}", prefix(self.telnet), name),
            payload,
        };
        let _ = timeout(Duration::from_secs(1), self.events.send(event)).await;
    }

    async fn run(mut self) {
        let _closed = Closed(self.client.closed.clone());
        let mut established = false;
        let mut attempt = 0_u32;
        loop {
            if *self.stopped.borrow() {
                break;
            }
            let connect = timeout(
                Duration::from_millis(self.config.connect_timeout_ms),
                proxy::connect(&self.config),
            );
            let opened = tokio::select! {
                _ = self.stopped.changed() => break,
                result = connect => result.map_err(|_| anyhow!("Connection timed out")).and_then(|r| r),
            };
            match opened {
                Ok(mut stream) => {
                    attempt = 0;
                    established = true;
                    let epoch = uuid::Uuid::new_v4().to_string();
                    let (mode, prompt) = self.client.interpretation.lock().await.clone();
                    *self.client.output.lock().await =
                        OutputBuffer::with_shell(mode, prompt.as_deref());
                    {
                        let mut state = self.client.state.lock().await;
                        *state = State {
                            connected: true,
                            connection_id: Some(epoch.clone()),
                            ..State::default()
                        };
                    }
                    self.notify("status", json!({"connected":true,"connectionId":epoch}))
                        .await;
                    if let Some((operation_id, mcp)) = self.operation.take() {
                        let result = json!({"connected":true,"connectionId":epoch});
                        let _ = self
                            .events
                            .send(DriverNotification::OperationCompleted {
                                operation_id,
                                result: if mcp {
                                    crate::stream::result::tool_result(result)
                                } else {
                                    result
                                },
                            })
                            .await;
                    }
                    let result = self.connected(&mut stream, &epoch).await;
                    self.client.output.lock().await.close();
                    {
                        let mut state = self.client.state.lock().await;
                        state.connected = false;
                        state.error = result.err().map(|e| e.to_string());
                    }
                }
                Err(error) => {
                    self.client.state.lock().await.error = Some(error.to_string());
                    if let Some((operation_id, _)) = self.operation.take() {
                        let _ = self
                            .events
                            .send(DriverNotification::OperationFailed {
                                operation_id,
                                error: super::backend::error(
                                    "network_connect_failed",
                                    error.to_string(),
                                ),
                            })
                            .await;
                    }
                }
            }
            if *self.stopped.borrow() || !established || !self.config.auto_reconnect {
                break;
            }
            self.client.state.lock().await.reconnecting = true;
            self.notify("status", json!({"connected":false,"reconnecting":true}))
                .await;
            let delay = self
                .config
                .reconnect_interval_ms
                .saturating_mul(1_u64 << attempt.min(5))
                .min(60_000);
            attempt = attempt.saturating_add(1);
            tokio::select! { _ = self.stopped.changed() => break, _ = sleep(Duration::from_millis(delay)) => {} }
        }
        {
            let mut state = self.client.state.lock().await;
            state.connected = false;
            state.reconnecting = false;
        }
        self.client.output.lock().await.close();
        if let Some((operation_id, _)) = self.operation.take() {
            let _ = self
                .events
                .send(DriverNotification::OperationFailed {
                    operation_id,
                    error: super::backend::error(
                        "network_connect_cancelled",
                        "Connection attempt cancelled",
                    ),
                })
                .await;
        }
        self.notify("status", json!({"connected":false,"reconnecting":false}))
            .await;
    }

    async fn connected(&mut self, stream: &mut TcpStream, epoch: &str) -> Result<()> {
        let mut codec = self
            .telnet
            .then(|| Codec::new(&self.config.terminal_type, 80, 24));
        // Bound each notification by bytes while amortizing serialization and broadcast work.
        let mut bytes = vec![0_u8; READ_BATCH_BYTES];
        let mut login_tail = String::new();
        let mut username_sent = false;
        loop {
            tokio::select! {
                _ = self.stopped.changed() => return Ok(()),
                action = self.receiver.recv() => {
                    let Some(action) = action else { return Ok(()) };
                    self.input(stream, &mut codec, epoch, action).await?;
                },
                read = stream.read(&mut bytes) => {
                    let count = read?;
                    if count == 0 { return Ok(()) }
                    let data = if let Some(codec) = codec.as_mut() {
                        let decoded = codec.decode(&bytes[..count])?;
                        self.send(stream, &decoded.reply).await?;
                        let changed = { let mut state = self.client.state.lock().await;
                            let changed = state.remote_echo != codec.remote_echo(); state.remote_echo = codec.remote_echo(); changed };
                        if changed { self.notify("status", json!({"connected":true})).await; }
                        decoded.data
                    } else { bytes[..count].to_vec() };
                    if !data.is_empty() {
                        let (text,cursor) = { let mut output = self.client.output.lock().await;
                            let text = output.push(&data); (text,output.cursor()) };
                        self.client.state.lock().await.rx += data.len() as u64;
                        let _ = self.events.try_send(DriverNotification::Event { driver_instance_id:self.instance.clone(),
                            name:format!("{}.output", prefix(self.telnet)), payload:json!({"connectionId":epoch,"text":text,"bytes":data.len(),"cursor":cursor}) });
                        if let Some(username) = self.config.username.as_ref().filter(|_| !username_sent) {
                            login_tail.push_str(&String::from_utf8_lossy(&data));
                            if login_tail.len() > 256 { let mut start = login_tail.len() - 256;
                                while !login_tail.is_char_boundary(start) { start += 1; } login_tail = login_tail[start..].into(); }
                            let line = login_tail.trim_end().to_ascii_lowercase();
                            if line.ends_with("login:") || line.ends_with("username:") {
                                let input = format!("{username}\r\n").into_bytes();
                                let encoded = codec.as_ref().map_or_else(|| input.clone(), |c| c.encode(&input));
                                self.send(stream, &encoded).await?; username_sent = true;
                            }
                        }
                    }
                }
            }
        }
    }

    async fn send(&self, stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
        if !bytes.is_empty() {
            timeout(Duration::from_secs(2), stream.write_all(bytes))
                .await
                .map_err(|_| anyhow!("Socket write timed out; result is unknown"))??;
        }
        Ok(())
    }

    async fn input(
        &mut self,
        stream: &mut TcpStream,
        codec: &mut Option<Codec>,
        epoch: &str,
        action: Input,
    ) -> Result<()> {
        match action {
            Input::Write {
                epoch: requested,
                bytes,
                reply,
            } => {
                if requested != epoch || reply.is_closed() {
                    let _ = reply.send(Err(anyhow!("Stale connection input rejected")));
                    return Ok(());
                }
                let encoded = codec
                    .as_ref()
                    .map_or_else(|| bytes.clone(), |c| c.encode(&bytes));
                if let Err(error) = self.send(stream, &encoded).await {
                    let _ = reply.send(Err(anyhow!("Socket write failed; result is unknown")));
                    return Err(error);
                }
                self.client.state.lock().await.tx += bytes.len() as u64;
                let _ = reply.send(Ok(bytes.len()));
            }
            Input::Resize {
                epoch: requested,
                cols,
                rows,
                reply,
            } => {
                if requested != epoch {
                    let _ = reply.send(Err(anyhow!("Stale connection resize rejected")));
                    return Ok(());
                }
                if let Some(codec) = codec {
                    self.send(stream, &codec.resize(cols, rows)).await?;
                }
                let _ = reply.send(Ok(()));
            }
        }
        Ok(())
    }
}
