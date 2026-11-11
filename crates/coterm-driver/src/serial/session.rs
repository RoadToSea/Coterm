//! 串口连接、读取线程和有界回放缓冲。协议语义全部留在这里，基座不感知 UART。
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::JoinHandle as ThreadHandle,
    time::{Duration, Instant},
};

use crate::serial::command::{CommandHandle, CommandJob, CommandList};
use crate::serial::serial_io::SerialWriter;
use crate::serial::shell::validate_prompt;
pub use crate::serial::shell::Mode;
use crate::stream::buffer::OUTPUT_CHUNK_BYTES;
#[cfg(test)]
use crate::stream::{
    buffer::{MAX_OUTPUT_BYTES, READ_BYTES},
    payload::OUTPUT_TEXT_JSON_BUDGET,
};
use anyhow::{anyhow, bail, Result};
use coterm_core::driver::DriverNotification;
use coterm_core::model::{ErrorCategory, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serialport::SerialPort as _;
use tokio::{
    sync::{mpsc, oneshot, Mutex},
    task::JoinHandle,
    time::timeout,
};

const IO_TIMEOUT: Duration = Duration::from_secs(2);
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// 读取线程单次等待上限，兼顾小批接收和停止响应。
const READ_POLL: Duration = Duration::from_millis(20);
const READ_BATCH_DELAY: Duration = Duration::from_millis(4);
/// 显式断开时等待读取线程回收的上限；超时后句柄分离，不阻塞断开。
const READER_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    None,
    Odd,
    Even,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub port: String,
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    #[serde(default = "default_parity")]
    pub parity: Parity,
    /// 打开端口后立即设置的数据终端就绪电平；部分开发板会因 DTR 变化复位。
    #[serde(default = "default_line_level")]
    pub dtr: bool,
    /// 打开端口后立即设置的请求发送电平。
    #[serde(default = "default_line_level")]
    pub rts: bool,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub prompt: Option<String>,
}

fn default_data_bits() -> u8 {
    8
}

fn default_stop_bits() -> u8 {
    1
}

fn default_parity() -> Parity {
    Parity::None
}

fn default_line_level() -> bool {
    true
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if !valid_port_name(self.port.trim()) {
            bail!("Invalid serial port name");
        }
        if self.baud_rate == 0 {
            bail!("Invalid serial baud rate");
        }
        if !(5..=8).contains(&self.data_bits) {
            bail!("Invalid serial data bits");
        }
        if !matches!(self.stop_bits, 1 | 2) {
            bail!("Invalid serial stop bits");
        }
        if let Some(prompt) = &self.prompt {
            validate_prompt(prompt)?;
        }
        if self.mode == Mode::Shell && self.prompt.is_none() {
            bail!("mode=shell requires an explicit literal prompt");
        }
        Ok(())
    }

    /// 稳定的实例标签，基座只做长度校验，不解析字段含义。
    pub fn label(&self) -> String {
        format!("{}@{}", self.port.trim(), self.baud_rate)
    }
}

/// 端口名交给操作系统解析，但先挡掉路径穿越、控制字符和包外设备节点。
///
/// 接受 `COM6` 这类设备名，也接受 Unix 的 `/dev/...` 绝对路径；其余带分隔符的
/// 名字一律拒绝，避免把配置变成任意文件打开入口。
pub fn valid_port_name(port: &str) -> bool {
    if port.is_empty() || port.len() > 256 || port.chars().any(char::is_control) {
        return false;
    }
    if port.contains("..") || port.contains("//") || port.ends_with('/') {
        return false;
    }
    if let Some(device) = port.strip_prefix("/dev/") {
        return !device.is_empty()
            && device
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    }
    if port.contains(['/', '\\']) {
        return false;
    }
    port.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Windows 端口名不区分大小写，Unix 设备节点区分；用于判断两个配置是否指向同一端口。
pub fn same_port(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

pub use crate::stream::buffer::OutputBuffer;

impl OutputBuffer {
    pub(crate) fn configured(config: &Config) -> Self {
        Self::with_shell(config.mode, config.prompt.as_deref())
    }
}

enum Input {
    Write(Vec<u8>),
    Commands(CommandJob),
    /// 只改请求的电平；未给出的信号保持不变。
    Signals {
        dtr: Option<bool>,
        rts: Option<bool>,
    },
    Close,
}

struct Action {
    input: Input,
    reply: oneshot::Sender<Result<()>>,
}

pub struct Connection {
    actions: mpsc::Sender<Action>,
    commands: HashMap<String, CommandHandle>,
    pub output: Arc<Mutex<OutputBuffer>>,
    task: Option<JoinHandle<()>>,
    reader: Option<ThreadHandle<()>>,
    stop: Arc<AtomicBool>,
    dtr: Arc<AtomicBool>,
    rts: Arc<AtomicBool>,
    rx_bytes: Arc<AtomicU64>,
    tx_bytes: Arc<AtomicU64>,
    pub id: String,
}

impl Drop for Connection {
    fn drop(&mut self) {
        // 未显式断开时也要让读取线程退出；析构不能阻塞，句柄交给线程自身收尾。
        self.stop.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.reader.take();
    }
}

impl Connection {
    /// Returns whether this connection has stopped or its actor has exited.
    /// Reader/write failures stop the connection before final events finish sending.
    pub fn is_closed(&self) -> bool {
        self.stop.load(Ordering::SeqCst) || self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub async fn connect(config: &Config) -> Result<Self> {
        Self::connect_with_events(config, None).await
    }

    /// 打开串口并启动专用读取线程。阻塞式串口 IO 不进入异步运行时。
    pub async fn connect_with_events(
        config: &Config,
        events: Option<(String, mpsc::Sender<DriverNotification>)>,
    ) -> Result<Self> {
        config.validate()?;
        // 打开端口是阻塞系统调用：放进阻塞线程池，超时后由闭包自己丢弃句柄关闭端口。
        let (opened, opened_result) = oneshot::channel();
        let open_config = config.clone();
        tokio::task::spawn_blocking(move || {
            let _ = opened.send(open_port(&open_config));
        });
        let (port, writer) = match timeout(OPEN_TIMEOUT, opened_result).await {
            Ok(Ok(Ok(pair))) => pair,
            Ok(Ok(Err(error))) => return Err(error),
            Ok(Err(_)) => bail!("Serial port opening was abandoned"),
            Err(_) => bail!("Opening the serial port timed out"),
        };
        let mut writer = Some(writer);

        let output = Arc::new(Mutex::new(OutputBuffer::configured(config)));
        let (actions, mut receiver) = mpsc::channel::<Action>(64);
        let id = uuid::Uuid::new_v4().to_string();
        let connection_id = id.clone();
        let task_output = output.clone();
        let task_events = events.clone();
        let (closed, mut reader_closed) = oneshot::channel::<()>();
        let reader_output = output.clone();
        let reader_connection_id = connection_id.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let reader_stop = stop.clone();
        let task_stop = stop.clone();
        let dtr = Arc::new(AtomicBool::new(config.dtr));
        let rts = Arc::new(AtomicBool::new(config.rts));
        let task_dtr = dtr.clone();
        let task_rts = rts.clone();
        let rx_bytes = Arc::new(AtomicU64::new(0));
        let reader_rx = rx_bytes.clone();
        let tx_bytes = Arc::new(AtomicU64::new(0));
        let task_tx = tx_bytes.clone();

        // 读取线程独占端口读端：阻塞式 IO 不能占用异步工作线程。
        let source = ReaderSource {
            port,
            output: reader_output,
            stop: reader_stop,
            events: task_events,
            connection_id: reader_connection_id,
            rx_bytes: reader_rx,
            pending: None,
        };
        let reader = std::thread::Builder::new()
            .name("coterm-serial-reader".to_owned())
            .spawn(move || {
                read_until_stopped(source);
                let _ = closed.send(());
            })
            .map_err(|error| anyhow!("Serial reader thread could not start: {error}"))?;

        let task = tokio::spawn(async move {
            let mut reader_finished = false;
            loop {
                tokio::select! {
                    action = receiver.recv() => {
                        let Some(action) = action else { break; };
                        let closing = matches!(action.input, Input::Close);
                        let result = match action.input {
                            Input::Commands(job) => {
                                let (returned, failed) = job.run(writer.take(), &task_stop, &task_tx, &events, &connection_id, task_output.clone()).await;
                                writer = returned;
                                if failed { Err(anyhow!("Serial command delivery failed")) } else { Ok(()) }
                            }
                            Input::Write(data) => {
                                // 写端在阻塞线程里独占使用，闭包返回时归还给本 actor。
                                let Some(handle) = writer.take() else {
                                    let _ = action.reply.send(Err(anyhow!(
                                        "Serial connection is no longer writable"
                                    )));
                                    break;
                                };
                                let written = data.len() as u64;
                                timeout(
                                    IO_TIMEOUT,
                                    tokio::task::spawn_blocking(move || {
                                        let mut handle = handle;
                                        let outcome = handle
                                            .write_all(&data)
                                            .and_then(|()| handle.flush());
                                        (handle, outcome)
                                    }),
                                )
                                .await
                                .map_err(|_| anyhow!("Serial write timed out"))
                                .and_then(|joined| {
                                    joined.map_err(|error| anyhow!("Serial write task failed: {error}"))
                                })
                                .and_then(|(returned, outcome)| {
                                    writer = Some(returned);
                                    outcome.map_err(|error| anyhow!("Serial write failed: {error}"))
                                })
                                .inspect(|()| {
                                    task_tx.fetch_add(written, Ordering::Relaxed);
                                })
                            }
                            Input::Signals { dtr, rts } => {
                                let Some(handle) = writer.take() else {
                                    let _ = action.reply.send(Err(anyhow!(
                                        "Serial connection is no longer writable"
                                    )));
                                    break;
                                };
                                let (dtr_state, rts_state) = (task_dtr.clone(), task_rts.clone());
                                timeout(
                                    IO_TIMEOUT,
                                    tokio::task::spawn_blocking(move || {
                                        let mut handle = handle;
                                        let mut outcome = Ok(());
                                        if let Some(level) = dtr {
                                            if let Err(error) = handle.port.write_data_terminal_ready(level) {
                                                outcome = Err(error);
                                            } else {
                                                dtr_state.store(level, Ordering::Relaxed);
                                            }
                                        }
                                        if outcome.is_ok() {
                                            if let Some(level) = rts {
                                                if let Err(error) = handle.port.write_request_to_send(level) {
                                                    outcome = Err(error);
                                                } else {
                                                    rts_state.store(level, Ordering::Relaxed);
                                                }
                                            }
                                        }
                                        (handle, outcome)
                                    }),
                                )
                                .await
                                .map_err(|_| anyhow!("Serial signal change timed out"))
                                .and_then(|joined| {
                                    joined.map_err(|error| anyhow!("Serial signal task failed: {error}"))
                                })
                                .and_then(|(returned, outcome)| {
                                    writer = Some(returned);
                                    outcome.map_err(|error| anyhow!("Serial signal change failed: {error}"))
                                })
                            }
                            Input::Close => Ok(()),
                        };
                        let failed = result.is_err();
                        let _ = action.reply.send(result);
                        if closing || failed {
                            break;
                        }
                    }
                    _ = &mut reader_closed => { reader_finished = true; break; },
                }
            }
            // 停止标志同时约束读取线程；断开或读写失败后都不能让它继续跑。
            task_stop.store(true, Ordering::SeqCst);
            receiver.close();
            while let Some(action) = receiver.recv().await {
                if let Input::Commands(job) = action.input {
                    job.fail(&events).await;
                }
                let _ = action.reply.send(Err(anyhow!(
                    "Serial connection closed before sending input"
                )));
            }
            drop(writer);
            // The reader must publish its final partial batch before UTF-8 and Shell state close.
            if !reader_finished {
                let _ = timeout(READER_JOIN_TIMEOUT, &mut reader_closed).await;
            }
            task_output.lock().await.close();
            if let Some((instance, sender)) = &events {
                let cursor = task_output.lock().await.cursor();
                let _ = sender
                    .send(DriverNotification::Event {
                        driver_instance_id: instance.clone(),
                        name: "serial.status".into(),
                        payload: json!({"connected": false, "connectionId": connection_id, "cursor": cursor}),
                    })
                    .await;
            }
        });
        Ok(Self {
            actions,
            commands: HashMap::new(),
            output,
            task: Some(task),
            reader: Some(reader),
            stop,
            dtr,
            rts,
            rx_bytes,
            tx_bytes,
            id,
        })
    }

    async fn submit(&self, input: Input) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.actions
            .try_send(Action { input, reply })
            .map_err(|_| anyhow!("Serial input queue is full or closed"))?;
        timeout(Duration::from_secs(3), response)
            .await
            .map_err(|_| anyhow!("Serial input wait timed out; delivery may be unknown"))?
            .map_err(|_| anyhow!("Serial connection closed before input acknowledgement"))?
    }

    pub async fn write(&self, data: Vec<u8>) -> Result<()> {
        if self.pending_commands() > 0 {
            bail!("Serial command queue is busy; wait or cancel it before raw input");
        }
        if data.is_empty() || data.len() > 16 * 1024 {
            bail!("Serial input must contain 1 to 16384 bytes");
        }
        self.submit(Input::Write(data)).await
    }

    pub(crate) fn queue_commands(
        &mut self,
        operation_id: &str,
        mcp: bool,
        list: CommandList,
    ) -> Result<()> {
        self.commands
            .retain(|_, handle| !handle.progress.borrow().finished);
        if self.is_closed() {
            bail!("Serial port is not open");
        }
        if self.commands.len() >= 16 || self.commands.contains_key(operation_id) {
            bail!("Serial command queue is full or operation already exists (limit: 16 lists)");
        }
        let (job, handle) = CommandJob::new(operation_id.into(), mcp, list);
        let (reply, _) = oneshot::channel();
        self.actions
            .try_send(Action {
                input: Input::Commands(job),
                reply,
            })
            .map_err(|_| anyhow!("Serial input queue is full or closed"))?;
        self.commands.insert(operation_id.into(), handle);
        Ok(())
    }

    pub fn pending_commands(&self) -> usize {
        self.commands
            .values()
            .filter(|handle| !handle.progress.borrow().finished)
            .count()
    }

    pub async fn cancel_commands(&self, operation_id: &str) -> Result<Value> {
        let Some(handle) = self.commands.get(operation_id) else {
            return Ok(json!({"outcome":"unsupported"}));
        };
        let mut progress = handle.progress.clone();
        let _ = handle.cancel.send(true);
        if !progress.borrow().started && !progress.borrow().finished {
            return Ok(json!({"outcome":"not_submitted","sent":0,"bytes":0}));
        }
        let finished = timeout(Duration::from_secs(3), async {
            loop {
                let state = progress.borrow().clone();
                if state.finished {
                    return Some(state);
                }
                if progress.changed().await.is_err() {
                    return None;
                }
            }
        })
        .await
        .ok()
        .flatten();
        Ok(match finished {
            Some(state) if state.cancelled => {
                json!({"outcome":if state.sent == 0 {"not_submitted"} else {"stopped"},"sent":state.sent,"bytes":state.bytes})
            }
            Some(_) => json!({"outcome":"unsupported"}),
            None => json!({"outcome":"unknown"}),
        })
    }

    /// 当前 DTR/RTS 电平；只在驱动确认后更新，失败保留上一次的值。
    pub fn signals(&self) -> (bool, bool) {
        (
            self.dtr.load(Ordering::Relaxed),
            self.rts.load(Ordering::Relaxed),
        )
    }

    pub async fn set_signals(&self, dtr: Option<bool>, rts: Option<bool>) -> Result<(bool, bool)> {
        if self.pending_commands() > 0 {
            bail!("Serial command queue is busy; wait or cancel it before changing signals");
        }
        if dtr.is_none() && rts.is_none() {
            bail!("At least one serial signal must be requested");
        }
        self.submit(Input::Signals { dtr, rts }).await?;
        Ok(self.signals())
    }

    /// 本连接累计收发字节数；断开后保留，供状态查询显示。
    pub fn counters(&self) -> (u64, u64) {
        (
            self.rx_bytes.load(Ordering::Relaxed),
            self.tx_bytes.load(Ordering::Relaxed),
        )
    }

    /// 显式断开：先停写端，再等读取线程退出并回收句柄。
    pub async fn disconnect(&mut self) -> Result<()> {
        // 在 Close 到达 actor 前停止当前列表的后续项；不继续排空设备命令。
        self.stop.store(true, Ordering::SeqCst);
        let result = self.submit(Input::Close).await;
        self.stop.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take() {
            let _ = timeout(IO_TIMEOUT, task).await;
        }
        if let Some(reader) = self.reader.take() {
            // join 会阻塞，放进阻塞线程池，避免占用异步工作线程。
            let joined = tokio::task::spawn_blocking(move || {
                let _ = reader.join();
            });
            let _ = timeout(READER_JOIN_TIMEOUT, joined).await;
        }
        self.output.lock().await.close();
        result
    }
}

/// 读取线程独占的端口读端和推送目标。抽出来是为了让停止与回收逻辑可测。
struct ReaderSource<R> {
    port: R,
    output: Arc<Mutex<OutputBuffer>>,
    stop: Arc<AtomicBool>,
    events: Option<(String, mpsc::Sender<DriverNotification>)>,
    connection_id: String,
    rx_bytes: Arc<AtomicU64>,
    pending: Option<DriverNotification>,
}

impl<R> ReaderSource<R> {
    fn publish(&mut self, data: &[u8]) {
        if data.is_empty() {
            self.retry();
            return;
        }
        let (cursor, text) = {
            let mut output = self.output.blocking_lock();
            let text = output.push(data);
            (output.cursor(), text)
        };
        if let Some((instance, _)) = &self.events {
            // Keep only the latest hint; skipped cursors recover from the bounded replay buffer.
            self.pending = Some(DriverNotification::Event {
                driver_instance_id: instance.clone(),
                name: "serial.output".into(),
                payload: json!({
                    "connectionId": self.connection_id,
                    "text": text,
                    "bytes": data.len(),
                    "cursor": cursor,
                }),
            });
        }
        self.retry();
    }

    fn retry(&mut self) {
        if let (Some(event), Some((_, sender))) = (self.pending.take(), &self.events) {
            if let Err(mpsc::error::TrySendError::Full(event)) = sender.try_send(event) {
                self.pending = Some(event);
            }
        }
    }
}

/// 循环读取直到停止标志置位或端口出错；读取等待受 `READ_POLL` 限制，
/// 实际退出时间还受系统 IO 和调度影响。
fn read_until_stopped<R: Read>(mut source: ReaderSource<R>) {
    let mut incoming = vec![0u8; OUTPUT_CHUNK_BYTES];
    let mut buffer = vec![0u8; OUTPUT_CHUNK_BYTES];
    let mut used = 0;
    let mut published = Instant::now();
    while !source.stop.load(Ordering::Relaxed) {
        let mut idle = false;
        // Notification boundaries must not change the size of OS read requests.
        match source.port.read(&mut incoming) {
            Ok(0) => idle = true,
            Ok(count) => {
                source.rx_bytes.fetch_add(count as u64, Ordering::Relaxed);
                let mut data = &incoming[..count];
                while !data.is_empty() {
                    let count = (buffer.len() - used).min(data.len());
                    buffer[used..used + count].copy_from_slice(&data[..count]);
                    used += count;
                    data = &data[count..];
                    if used == buffer.len() {
                        source.publish(&buffer);
                        used = 0;
                        published = Instant::now();
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => idle = true,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
        // Small UART reads must not each allocate a replay chunk and a notification.
        if used > 0 && (idle || published.elapsed() >= READ_BATCH_DELAY) {
            source.publish(&buffer[..used]);
            used = 0;
            published = Instant::now();
        } else if idle {
            source.retry();
        }
    }
    source.publish(&buffer[..used]);
    source.stop.store(true, Ordering::SeqCst);
}

/// 阻塞打开端口并设置初始信号电平，返回读端和写端两个句柄。
fn open_port(config: &Config) -> Result<(Box<dyn Read + Send>, SerialWriter)> {
    #[cfg(windows)]
    let mut port = super::windows_io::open(
        config.port.trim(),
        config.baud_rate,
        config.data_bits,
        config.stop_bits,
        match config.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        },
    )
    .map_err(|error| anyhow!("Serial port could not be opened: {error}"))?;
    #[cfg(not(windows))]
    let mut port = serialport::new(config.port.trim(), config.baud_rate)
        .data_bits(match config.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        })
        .stop_bits(if config.stop_bits == 2 {
            serialport::StopBits::Two
        } else {
            serialport::StopBits::One
        })
        .parity(match config.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        })
        .timeout(READ_POLL)
        .open_native()
        .map_err(|error| anyhow!("Serial port could not be opened: {error}"))?;
    port.write_data_terminal_ready(config.dtr)
        .map_err(|error| anyhow!("Serial DTR could not be set: {error}"))?;
    port.write_request_to_send(config.rts)
        .map_err(|error| anyhow!("Serial RTS could not be set: {error}"))?;
    #[cfg(not(windows))]
    let writer = port
        .try_clone()
        .map_err(|error| anyhow!("Serial port write handle failed: {error}"))?;
    #[cfg(windows)]
    let writer = port
        .try_clone_native()
        .map_err(|error| anyhow!("Serial port write handle failed: {error}"))?;
    #[cfg(windows)]
    crate::serial::serial_io::configure_io_timeouts(&port, READ_POLL, IO_TIMEOUT)
        .map_err(|error| anyhow!("Serial I/O timeouts could not be set: {error}"))?;
    #[cfg(windows)]
    let reader = super::windows_io::Reader::new(port)?;
    #[cfg(not(windows))]
    let reader = port;
    Ok((
        Box::new(reader),
        SerialWriter {
            port: writer,
            timeout: IO_TIMEOUT,
            #[cfg(windows)]
            operation: super::windows_io::Operation::new()?,
        },
    ))
}

/// 错误码与 `ErrorCategory` 的稳定映射，供后端在失败通知中使用。
pub fn domain_error(code: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Domain,
        code: code.to_owned(),
        message: message.into(),
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_text_returns_bounded_tail_lines_without_a_cursor() -> Result<()> {
        let mut output = OutputBuffer::default();
        output.push("old\r\nmiddle\r\nnew".as_bytes());

        let recent = output.recent_text(2)?;
        assert_eq!(recent["text"], "middle\r\nnew");
        assert_eq!(recent["lines"], 2);
        assert_eq!(recent["cursor"], output.cursor());
        assert_eq!(recent["gap"], false);
        assert_eq!(recent["truncated"], false);
        assert!(output.recent_text(0).is_err());
        assert!(output.recent_text(201).is_err());
        Ok(())
    }

    #[test]
    fn recent_text_caps_a_single_long_line() -> Result<()> {
        let mut output = OutputBuffer::default();
        output.push("x".repeat(OUTPUT_TEXT_JSON_BUDGET * 2).as_bytes());

        let recent = output.recent_text(1)?;
        assert_eq!(
            recent["text"].as_str().map(str::len),
            Some(OUTPUT_TEXT_JSON_BUDGET)
        );
        assert_eq!(recent["truncated"], true);
        Ok(())
    }

    #[test]
    fn recent_text_reports_when_the_bounded_buffer_evicted_earlier_output() -> Result<()> {
        let mut output = OutputBuffer::default();
        output.push(&vec![b'x'; MAX_OUTPUT_BYTES + 1]);

        let recent = output.recent_text(1)?;
        assert_eq!(recent["gap"], true);
        assert_eq!(recent["truncated"], true);
        Ok(())
    }

    /// 只替换物理连接，仍使用 Connection 的真实入队、限流和取消接口。
    fn queued_connection() -> (Connection, mpsc::Receiver<Action>) {
        let (actions, receiver) = mpsc::channel(64);
        (
            Connection {
                actions,
                commands: HashMap::new(),
                output: Arc::new(Mutex::new(OutputBuffer::default())),
                task: Some(tokio::spawn(std::future::pending())),
                reader: None,
                stop: Arc::new(AtomicBool::new(false)),
                dtr: Arc::new(AtomicBool::new(true)),
                rts: Arc::new(AtomicBool::new(true)),
                rx_bytes: Arc::new(AtomicU64::new(0)),
                tx_bytes: Arc::new(AtomicU64::new(0)),
                id: "queue-test".into(),
            },
            receiver,
        )
    }

    #[tokio::test]
    async fn reader_failure_reports_closed_before_actor_finishes() -> Result<()> {
        struct RemovedPort;
        impl Read for RemovedPort {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }
        let (connection, _receiver) = queued_connection();
        assert!(!connection.is_closed());
        let source = ReaderSource {
            port: RemovedPort,
            output: connection.output.clone(),
            stop: connection.stop.clone(),
            events: None,
            connection_id: connection.id.clone(),
            rx_bytes: connection.rx_bytes.clone(),
            pending: None,
        };
        tokio::task::spawn_blocking(move || read_until_stopped(source)).await?;
        assert!(
            connection.is_closed(),
            "a removed port must not remain connected while the actor is still finishing"
        );
        Ok(())
    }

    #[tokio::test]
    async fn command_queue_is_bounded_fifo_and_blocks_conflicting_raw_input() -> Result<()> {
        let (mut connection, mut receiver) = queued_connection();
        for index in 0..16 {
            connection.queue_commands(&format!("op-{index}"), false,
                CommandList::parse(&json!({"commands":[format!("first-{index}"), format!("second-{index}")],"lineEnding":"cr","intervalMs":0}))?)?;
        }
        assert_eq!(connection.pending_commands(), 16);
        assert!(connection
            .queue_commands(
                "overflow",
                false,
                CommandList::parse(&json!({"commands":["overflow"]}))?
            )
            .is_err());
        assert!(connection.write(b"raw".to_vec()).await.is_err());
        assert!(connection.set_signals(Some(false), None).await.is_err());
        let mut writer = Some(std::io::Cursor::new(Vec::<u8>::new()));
        let mut expected = String::new();
        for index in 0..16 {
            let action = receiver.try_recv()?;
            let Input::Commands(job) = action.input else {
                bail!("expected queued command list");
            };
            assert_eq!(job.operation_id, format!("op-{index}"));
            let (returned, failed) = job
                .run(
                    writer.take(),
                    &connection.stop,
                    &connection.tx_bytes,
                    &None,
                    &connection.id,
                    connection.output.clone(),
                )
                .await;
            assert!(!failed);
            writer = returned;
            expected.push_str(&format!("first-{index}\rsecond-{index}\r"));
        }
        assert_eq!(
            writer.ok_or_else(|| anyhow!("writer lost"))?.into_inner(),
            expected.as_bytes()
        );
        assert_eq!(connection.pending_commands(), 0);
        assert_eq!(connection.counters().1, expected.len() as u64);
        // 完成后的句柄会回收，下一批可以立即入队。
        connection.queue_commands(
            "next",
            false,
            CommandList::parse(&json!({"commands":["next"]}))?,
        )?;
        assert_eq!(connection.commands.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_a_queued_list_confirms_not_submitted_and_sends_no_bytes() -> Result<()> {
        let (mut connection, mut receiver) = queued_connection();
        connection.queue_commands(
            "cancel-queued",
            false,
            CommandList::parse(&json!({"commands":["must-not-send"]}))?,
        )?;
        let outcome = connection.cancel_commands("cancel-queued").await?;
        assert_eq!(outcome["outcome"], "not_submitted");
        let Input::Commands(job) = receiver.try_recv()?.input else {
            bail!("expected queued command list");
        };
        let (writer, failed) = job
            .run(
                Some(std::io::Cursor::new(Vec::<u8>::new())),
                &connection.stop,
                &connection.tx_bytes,
                &None,
                &connection.id,
                connection.output.clone(),
            )
            .await;
        assert!(!failed);
        assert!(writer
            .ok_or_else(|| anyhow!("writer lost"))?
            .into_inner()
            .is_empty());
        assert_eq!(connection.pending_commands(), 0);
        Ok(())
    }

    fn config(port: &str) -> Config {
        Config {
            port: port.into(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: Parity::None,
            dtr: true,
            rts: true,
            mode: Mode::Stream,
            prompt: None,
        }
    }

    #[test]
    fn config_accepts_device_names_and_unix_nodes_only() {
        let valid = config("COM6");
        assert!(valid.validate().is_ok());
        assert_eq!(valid.label(), "COM6@115200");
        assert!(config("/dev/ttyUSB0").validate().is_ok());
        assert!(config("/dev/serial/by-id/usb-CH340-if00")
            .validate()
            .is_ok());
        assert!(config("tty.usbserial-1420").validate().is_ok());

        for port in [
            "",
            "  ",
            "..",
            "COM\u{7}",
            "/etc/passwd",
            "/dev/../etc/shadow",
            "/dev/",
            "/dev//ttyUSB0",
            "COM\\6",
            "COM..6",
            "COM 6",
        ] {
            assert!(
                config(port).validate().is_err(),
                "{port:?} must be rejected"
            );
        }
        let mut invalid = config("COM6");
        invalid.baud_rate = 0;
        assert!(invalid.validate().is_err());
        invalid.baud_rate = 115200;
        invalid.data_bits = 9;
        assert!(invalid.validate().is_err());
        invalid.data_bits = 8;
        invalid.stop_bits = 3;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn port_names_compare_per_platform() {
        assert!(same_port("COM6", "COM6"));
        #[cfg(windows)]
        assert!(same_port("COM6", "com6"));
        #[cfg(not(windows))]
        assert!(!same_port("/dev/ttyUSB0", "/dev/ttyusb0"));
    }

    #[test]
    fn replay_buffer_reports_gaps_and_bounds_memory() {
        let mut output = OutputBuffer::default();
        output.push(b"first");
        let read = output.read(0).unwrap();
        assert_eq!(read["cursor"], 1);
        assert_eq!(read["gap"], false);
        assert_eq!(read["text"], "first");

        // 塞入超过上限的数据，最老的块被淘汰。
        for _ in 0..300 {
            output.push(&vec![b'x'; OUTPUT_CHUNK_BYTES]);
        }
        let read = output.read(0).unwrap();
        assert_eq!(read["gap"], true);
        assert_eq!(read["more"], true);
        assert!(output.bytes <= MAX_OUTPUT_BYTES);

        // 超出末尾的游标必须被拒绝，而不是静默返回空。
        assert!(output.read(output.cursor() + 1).is_err());
    }

    /// 不打开硬件的回收契约：停止标志必须让真实读取循环退出，数据也不会丢。
    /// 这条用例防止「读取线程缺少停止信号与 join」的回归。
    #[test]
    fn read_loop_stops_on_the_flag_and_keeps_every_byte() {
        /// 每次读返回一个字节，模拟持续的串口数据流。
        struct Endless {
            reads: Arc<AtomicU64>,
        }
        impl Read for Endless {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                buffer[0] = b'z';
                Ok(1)
            }
        }

        let reads = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let output = Arc::new(Mutex::new(OutputBuffer::default()));
        let rx_bytes = Arc::new(AtomicU64::new(0));
        let source = ReaderSource {
            port: Endless {
                reads: reads.clone(),
            },
            output: output.clone(),
            stop: stop.clone(),
            events: None,
            connection_id: "test".into(),
            rx_bytes: rx_bytes.clone(),
            pending: None,
        };
        let reader = std::thread::spawn(move || read_until_stopped(source));

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while reads.load(Ordering::SeqCst) < 32 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(reads.load(Ordering::SeqCst) >= 32, "the loop must read");

        stop.store(true, Ordering::SeqCst);
        reader
            .join()
            .expect("the stop flag must end the reader thread");
        let after_stop = reads.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            reads.load(Ordering::SeqCst),
            after_stop,
            "no reads may happen after the stop flag"
        );
        // 每个字节都进入回放缓冲并计入接收计数，退出不能丢数据。
        assert_eq!(rx_bytes.load(Ordering::SeqCst), after_stop);
        assert_eq!(output.blocking_lock().bytes as u64, after_stop);
    }

    #[test]
    fn fragmented_reads_do_not_exhaust_output_notifications_and_keep_the_tail() -> Result<()> {
        struct Fragmented(std::io::Cursor<Vec<u8>>);
        impl Read for Fragmented {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                let count = self.0.read(&mut bytes[..1])?;
                if count == 0 {
                    return Err(std::io::ErrorKind::BrokenPipe.into());
                }
                Ok(count)
            }
        }
        let expected = vec![b'x'; OUTPUT_CHUNK_BYTES * 8 + 17];
        let output = Arc::new(Mutex::new(OutputBuffer::default()));
        let rx = Arc::new(AtomicU64::new(0));
        let (events, mut notifications) = mpsc::channel(128);
        read_until_stopped(ReaderSource {
            port: Fragmented(std::io::Cursor::new(expected.clone())),
            output: output.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            events: Some(("fragmented".into(), events)),
            connection_id: "epoch".into(),
            rx_bytes: rx.clone(),
            pending: None,
        });
        let mut received = Vec::new();
        while let Ok(DriverNotification::Event { payload, .. }) = notifications.try_recv() {
            let text = payload["text"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing output"))?;
            received.extend_from_slice(text.as_bytes());
        }
        assert_eq!(rx.load(Ordering::Relaxed), expected.len() as u64);
        assert_eq!(output.blocking_lock().bytes, expected.len());
        assert_eq!(
            received.len(),
            expected.len(),
            "Small system reads must not lose notifications"
        );
        assert_eq!(received, expected);
        Ok(())
    }

    #[test]
    fn idle_reader_retries_the_last_full_queue_notification() {
        struct Intermittent {
            step: u8,
            events: mpsc::Receiver<DriverNotification>,
            observed: Arc<AtomicU64>,
        }
        impl Read for Intermittent {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                self.step += 1;
                if self.step <= 2 {
                    let count = if self.step == 1 {
                        OUTPUT_CHUNK_BYTES
                    } else {
                        17
                    };
                    bytes[..count].fill(b'x');
                    return Ok(count);
                }
                if self.step >= 4 {
                    while let Ok(DriverNotification::Event { payload, .. }) = self.events.try_recv()
                    {
                        let text = payload["text"]
                            .as_str()
                            .ok_or_else(|| std::io::Error::other("Missing output"))?;
                        assert!(text.bytes().all(|byte| byte == b'x'));
                        self.observed
                            .fetch_add(text.len() as u64, Ordering::Relaxed);
                    }
                }
                Err(if self.step < 5 {
                    std::io::ErrorKind::TimedOut
                } else {
                    std::io::ErrorKind::BrokenPipe
                }
                .into())
            }
        }
        let (events, notifications) = mpsc::channel(1);
        let observed = Arc::new(AtomicU64::new(0));
        read_until_stopped(ReaderSource {
            port: Intermittent {
                step: 0,
                events: notifications,
                observed: observed.clone(),
            },
            output: Arc::new(Mutex::new(OutputBuffer::default())),
            stop: Arc::new(AtomicBool::new(false)),
            events: Some(("idle".into(), events)),
            connection_id: "epoch".into(),
            rx_bytes: Arc::new(AtomicU64::new(0)),
            pending: None,
        });
        assert_eq!(
            observed.load(Ordering::Relaxed),
            (OUTPUT_CHUNK_BYTES + 17) as u64,
            "The final output must be retried even when no later device data arrives"
        );
    }

    #[test]
    fn read_is_capped_per_call() {
        let mut output = OutputBuffer::default();
        output.push(&vec![b'y'; OUTPUT_CHUNK_BYTES * 4]);
        let read = output.read(0).unwrap();
        assert!(read["bytes"].as_u64().unwrap() <= READ_BYTES as u64);
        assert_eq!(read["more"], true);
    }

    #[test]
    fn text_and_hex_share_cursors_and_utf8_survives_fragmentation() -> Result<()> {
        let mut output = OutputBuffer::default();
        output.push(&[0xe4]);
        assert_eq!(output.read(0)?["text"], "");
        output.push(&[0xb8, 0xad, 3, 27, 13, 10, 0xff]);
        let read = output.read(1)?;
        assert_eq!(read["text"], "中\u{3}\u{1b}\r\n\u{fffd}");
        let hex = output.read_hex(1)?;
        assert_eq!(hex["hex"], "b8ad031b0d0aff");
        assert!(hex.get("text").is_none());
        assert!(read.get("hex").is_none());
        assert_eq!(hex["cursor"], read["cursor"]);
        output.push(&[0xe4]);
        output.close();
        assert_eq!(output.read(2)?["text"], "\u{fffd}");
        Ok(())
    }

    #[test]
    fn worst_case_text_pages_fit_mcp_result_envelopes() -> Result<()> {
        let mut output = OutputBuffer::default();
        output.push(&vec![0; READ_BYTES * 2]);
        let page = output.read(0)?;
        let result = json!({"content":[{"type":"text","text":page.to_string()}],"isError":false});
        assert!(serde_json::to_vec(&result)?.len() < 16 * 1024);
        assert_eq!(page["more"], true);
        Ok(())
    }
}
