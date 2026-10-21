//! 串口连接、读取线程和有界回放缓冲。协议语义全部留在这里，基座不感知 UART。
use std::{
    collections::VecDeque,
    io::{Read, Write},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use coterm_proto::runtime::{message::PluginNotification, ErrorCategory, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    sync::{mpsc, oneshot, Mutex},
    task::JoinHandle,
    time::timeout,
};

const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const OUTPUT_CHUNK_BYTES: usize = 8 * 1024;
// base64 加信封必须落在基座 16 KiB 结果上限内。
const READ_BYTES: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// 读取线程单次阻塞等待；只影响断开延迟，不影响数据完整性。
const READ_POLL: Duration = Duration::from_millis(20);

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

impl Config {
    pub fn validate(&self) -> Result<()> {
        let port = self.port.trim();
        // 端口名交给操作系统解析，但必须先挡掉路径穿越和控制字符。
        if port.is_empty()
            || port.len() > 256
            || port.chars().any(char::is_control)
            || port.contains("..")
            || port.contains(['/', '\\'])
        {
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
        Ok(())
    }

    /// 稳定的实例标签，基座只做长度校验，不解析字段含义。
    pub fn label(&self) -> String {
        format!("{}@{}", self.port.trim(), self.baud_rate)
    }
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

    /// 读取 `after` 之后的有界数据；游标过期时报告 gap，交由调用方重新对齐。
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
        Ok(json!({
            "data": STANDARD.encode(data),
            "cursor": next.to_string(),
            "gap": gap,
            "closed": self.closed,
            "more": next < self.sequence,
        }))
    }
}

enum Input {
    Write(Vec<u8>),
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

    pub async fn connect(config: &Config) -> Result<Self> {
        Self::connect_with_events(config, None).await
    }

    /// 打开串口并启动专用读取线程。阻塞式串口 IO 不进入异步运行时。
    pub async fn connect_with_events(
        config: &Config,
        events: Option<(String, mpsc::Sender<PluginNotification>)>,
    ) -> Result<Self> {
        config.validate()?;
        let mut port = timeout(OPEN_TIMEOUT, async {
            serialport::new(config.port.trim(), config.baud_rate)
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
                .open()
        })
        .await
        .map_err(|_| anyhow!("Opening the serial port timed out"))?
        .map_err(|error| anyhow!("Serial port could not be opened: {error}"))?;
        let mut writer = Some(
            port.try_clone()
                .map_err(|error| anyhow!("Serial port write handle failed: {error}"))?,
        );

        let output = Arc::new(Mutex::new(OutputBuffer::default()));
        let (actions, mut receiver) = mpsc::channel::<Action>(64);
        let id = uuid::Uuid::new_v4().to_string();
        let connection_id = id.clone();
        let task_output = output.clone();
        let task_events = events.clone();
        let (closed, mut reader_closed) = oneshot::channel::<()>();
        let reader_output = output.clone();
        let reader_connection_id = connection_id.clone();

        // 读取线程独占端口读端：阻塞式 IO 不能占用异步工作线程。
        std::thread::spawn(move || {
            let mut buffer = vec![0u8; OUTPUT_CHUNK_BYTES];
            loop {
                match port.read(&mut buffer) {
                    Ok(0) => continue,
                    Ok(count) => {
                        let data = buffer[..count].to_vec();
                        let pushed = {
                            let mut output = reader_output.blocking_lock();
                            output.push(&data);
                            output.sequence
                        };
                        if let Some((instance, sender)) = &task_events {
                            let notification = PluginNotification::Event {
                                plugin_instance_id: instance.clone(),
                                name: "serial.output".into(),
                                payload: json!({
                                    "connectionId": reader_connection_id,
                                    "data": STANDARD.encode(&data),
                                    "cursor": pushed.to_string(),
                                }),
                            };
                            // 有界队列已满时丢弃推送：回放缓冲才是权威来源。
                            if sender.try_send(notification).is_err() {
                                continue;
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = closed.send(());
        });

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    action = receiver.recv() => {
                        let Some(action) = action else { break; };
                        let closing = matches!(action.input, Input::Close);
                        let result = match action.input {
                            Input::Write(data) => {
                                // 写端在阻塞线程里独占使用，闭包返回时归还给本 actor。
                                let Some(handle) = writer.take() else {
                                    let _ = action.reply.send(Err(anyhow!(
                                        "Serial connection is no longer writable"
                                    )));
                                    break;
                                };
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
                            }
                            Input::Close => Ok(()),
                        };
                        let failed = result.is_err();
                        let _ = action.reply.send(result);
                        if closing || failed {
                            break;
                        }
                    }
                    _ = &mut reader_closed => break,
                }
            }
            task_output.lock().await.closed = true;
            if let Some((instance, sender)) = &events {
                let cursor = task_output.lock().await.sequence.to_string();
                let _ = sender
                    .send(PluginNotification::Event {
                        plugin_instance_id: instance.clone(),
                        name: "serial.status".into(),
                        payload: json!({"connected": false, "connectionId": connection_id, "cursor": cursor}),
                    })
                    .await;
            }
        });
        Ok(Self {
            actions,
            output,
            task,
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
        if data.is_empty() || data.len() > 16 * 1024 {
            bail!("Serial input must contain 1 to 16384 bytes");
        }
        self.submit(Input::Write(data)).await
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.submit(Input::Close).await
    }
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
    fn config_rejects_control_characters_and_paths() {
        let valid = Config {
            port: "COM6".into(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: Parity::None,
        };
        assert!(valid.validate().is_ok());
        assert_eq!(valid.label(), "COM6@115200");

        for port in ["", "  ", "..", "COM\u{7}", "/dev/ttyUSB0", "COM\\6"] {
            let mut config = valid.clone();
            config.port = port.into();
            assert!(config.validate().is_err(), "{port:?} must be rejected");
        }
        let mut config = valid.clone();
        config.baud_rate = 0;
        assert!(config.validate().is_err());
        let mut config = valid;
        config.data_bits = 9;
        assert!(config.validate().is_err());
    }

    #[test]
    fn replay_buffer_reports_gaps_and_bounds_memory() {
        let mut output = OutputBuffer::default();
        output.push(b"first");
        let read = output.read(0).unwrap();
        assert_eq!(read["cursor"], "1");
        assert_eq!(read["gap"], false);
        assert_eq!(STANDARD.decode(read["data"].as_str().unwrap()).unwrap(), b"first");

        // 塞入超过上限的数据，最老的块被淘汰。
        for _ in 0..64 {
            output.push(&vec![b'x'; OUTPUT_CHUNK_BYTES]);
        }
        let read = output.read(0).unwrap();
        assert_eq!(read["gap"], true);
        assert_eq!(read["more"], true);
        assert!(output.bytes <= MAX_OUTPUT_BYTES);

        // 超出末尾的游标必须被拒绝，而不是静默返回空。
        assert!(output.read(output.sequence + 1).is_err());
    }

    #[test]
    fn read_is_capped_per_call() {
        let mut output = OutputBuffer::default();
        output.push(&vec![b'y'; OUTPUT_CHUNK_BYTES * 4]);
        let read = output.read(0).unwrap();
        let bytes = STANDARD.decode(read["data"].as_str().unwrap()).unwrap();
        assert!(bytes.len() <= READ_BYTES);
        assert_eq!(read["more"], true);
    }
}
