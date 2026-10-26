//! 插件后端与基座之间的 stdio 传输骨架。
//!
//! 物理编码是「一行一个 JSON-RPC 2.0 消息」：基座按命令种类发不同的方法名（`initialize`、
//! `invoke`、`create_instance` …），插件回 `JsonRpcResponse`，插件主动推送走
//! `coterm.notification`。信封、读行、大小上限和输出串行化在这一层统一，插件只实现
//! [`PluginBackend`] 的领域逻辑。

use std::{sync::Arc, time::Duration};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::lifecycle::{HelloResult, RuntimeHello, INTERFACE_MAJOR};
use super::message::{HostCommand, PluginNotification};
use super::wire::{decode_request, encode_line, reply};
use super::{ErrorCategory, RuntimeError};

pub use super::backend::PluginBackend;
pub use super::wire::{MAX_CONTROL_BYTES, NOTIFICATION_METHOD};
pub const DEFAULT_TICK: Duration = Duration::from_secs(1);

/// 传输失败：协议不合法，或 stdout 已不可用。
#[derive(Debug)]
pub enum TransportError {
    Protocol(String),
    Io(std::io::Error),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(message) => write!(formatter, "{message}"),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// 读取一条控制行，并在等待期间按 `tick` 驱动插件钩子；返回 `None` 表示 stdin 已关闭。
///
/// 每次 consume 后保留已读字节；tick 只取消尚未消费的 fill_buf 等待。
async fn read_line_with_tick<B: PluginBackend, R: tokio::io::AsyncBufRead + Unpin>(
    backend: &mut B,
    reader: &mut R,
    tick: Duration,
    limit: usize,
) -> Result<Option<Vec<u8>>, TransportError> {
    let mut line = Vec::new();
    loop {
        let available = tokio::select! {
            available = reader.fill_buf() => available?,
            _ = tokio::time::sleep(tick) => {
                backend.tick();
                continue;
            }
        };
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(TransportError::Protocol(
                "plugin runtime control line is unterminated".to_owned(),
            ));
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let count = end.map_or(available.len(), |index| index + 1);
        if line.len() + count > limit {
            return Err(TransportError::Protocol(
                "plugin runtime control line exceeds limit".to_owned(),
            ));
        }
        // copy 在 consume 之前完成，两个 await 之间不持有 `available`。
        let chunk: Vec<u8> = available[..count].to_vec();
        reader.consume(count);
        line.extend_from_slice(&chunk);
        if end.is_some() {
            return Ok(Some(line));
        }
    }
}

/// 在 stdin/stdout 上运行插件后端，直到 stdin 关闭或收到 `shutdown`。
pub async fn serve<B: PluginBackend>(backend: B) -> Result<(), TransportError> {
    serve_with_tick(backend, DEFAULT_TICK).await
}

/// 与 [`serve`] 相同，但可指定 tick 间隔。
pub async fn serve_with_tick<B: PluginBackend>(
    backend: B,
    tick: Duration,
) -> Result<(), TransportError> {
    serve_with_io(backend, tokio::io::stdin(), tokio::io::stdout(), tick).await
}

/// 与 stdio 入口相同的协议循环；允许契约测试使用双向内存管道。
pub async fn serve_with_io<B, R, W>(
    mut backend: B,
    input: R,
    mut output: W,
    tick: Duration,
) -> Result<(), TransportError>
where
    B: PluginBackend,
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut notifications = backend
        .take_notifications()
        .ok_or_else(|| TransportError::Protocol("notification receiver unavailable".to_owned()))?;
    let (replies, mut responses) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let (negotiated, current) = tokio::sync::watch::channel::<Option<Arc<HelloResult>>>(None);
    let read = async move {
        let result = read_loop(
            &mut backend,
            BufReader::new(input),
            &replies,
            &negotiated,
            tick,
        )
        .await;
        // 关闭回复队列即结束输出，不等待领域任务遗留的通知发送端。
        drop(replies);
        result
    };
    let write = async move {
        let mut notifications_open = true;
        loop {
            let line = tokio::select! {
                biased;
                response = responses.recv() => match response {
                    Some(line) => line,
                    None => return Ok(()),
                },
                notification = notifications.recv(), if notifications_open => {
                    let Some(notification) = notification else { notifications_open = false; continue; };
                    let negotiated = current.borrow().clone()
                        .ok_or_else(|| TransportError::Protocol("notification before initialize".to_owned()))?;
                    let feature = match notification {
                        PluginNotification::Event { .. } => "plugin-events",
                        _ => "async-operations",
                    };
                    if !negotiated.enabled_features.iter().any(|f| f == feature) {
                        return Err(TransportError::Protocol("notification feature was not negotiated".to_owned()));
                    }
                    encode_line(&serde_json::json!({
                        "jsonrpc":"2.0", "method":NOTIFICATION_METHOD, "params":notification,
                    }), negotiated.limits.max_control_bytes as usize)
                        .map_err(|e| TransportError::Protocol(e.code))?
                }
            };
            output.write_all(&line).await?;
            output.flush().await?;
        }
    };
    // 任一端失败都会取消另一端；输出失败时不继续接收有副作用的请求。
    tokio::try_join!(read, write)?;
    Ok(())
}

fn rejected(category: ErrorCategory, code: &str) -> RuntimeError {
    RuntimeError {
        category,
        code: code.to_owned(),
        message: code.replace('_', " "),
        details: None,
    }
}

async fn dispatch<B: PluginBackend>(
    backend: &mut B,
    command: HostCommand,
    state: Option<&HelloResult>,
) -> Result<Value, RuntimeError> {
    if let HostCommand::Hello(ref hello) = command {
        if state.is_some() {
            return Err(rejected(
                ErrorCategory::Conflict,
                "runtime_already_initialized",
            ));
        }
        if hello.interface_major != INTERFACE_MAJOR || hello.limits.max_control_bytes == 0 {
            return Err(rejected(
                ErrorCategory::Unsupported,
                "plugin_handshake_incompatible",
            ));
        }
    } else if !matches!(command, HostCommand::Shutdown) {
        let state =
            state.ok_or_else(|| rejected(ErrorCategory::Conflict, "runtime_not_initialized"))?;
        let required = match command {
            HostCommand::ProvideSecret(_) => Some("secret-delivery"),
            HostCommand::InteractionResult(_) => Some("deferred-interactions"),
            _ => None,
        };
        if required.is_some_and(|feature| !state.enabled_features.iter().any(|f| f == feature)) {
            return Err(rejected(
                ErrorCategory::Unsupported,
                "runtime_feature_not_negotiated",
            ));
        }
    }
    backend.request(command).await
}

async fn read_loop<B: PluginBackend, R: tokio::io::AsyncBufRead + Unpin>(
    backend: &mut B,
    mut reader: R,
    replies: &tokio::sync::mpsc::Sender<Vec<u8>>,
    negotiated: &tokio::sync::watch::Sender<Option<Arc<HelloResult>>>,
    tick: Duration,
) -> Result<(), TransportError> {
    let mut state: Option<Arc<HelloResult>> = None;
    loop {
        let limit = state
            .as_ref()
            .map_or(MAX_CONTROL_BYTES, |s| s.limits.max_control_bytes as usize);
        let Some(line) = read_line_with_tick(backend, &mut reader, tick, limit).await? else {
            return Ok(());
        };
        let line = zeroize::Zeroizing::new(line);
        let parsed = decode_request(&line);
        let mut next_state = None;
        let mut shutdown = false;
        let response = match parsed {
            Err(error) => error.into_response(),
            Ok(message) => {
                shutdown = matches!(message.command, HostCommand::Shutdown);
                let hello: Option<RuntimeHello> = match &message.command {
                    HostCommand::Hello(h) => Some(h.clone()),
                    _ => None,
                };
                let mut result = dispatch(backend, message.command, state.as_deref()).await;
                if let (Some(hello), Ok(value)) = (hello, &result) {
                    let validated = serde_json::from_value::<HelloResult>(value.clone())
                        .map_err(|_| {
                            rejected(ErrorCategory::InvalidRequest, "plugin_handshake_invalid")
                        })
                        .and_then(|negotiated| {
                            hello.validate_result(&negotiated)?;
                            Ok(negotiated)
                        });
                    match validated {
                        Ok(value) => next_state = Some(value),
                        Err(error) => result = Err(error),
                    }
                }
                reply(message.request_id, result)
            }
        };
        let encoded =
            encode_line(&response, limit).map_err(|e| TransportError::Protocol(e.code))?;
        replies
            .send(encoded)
            .await
            .map_err(|_| TransportError::Protocol("output closed".to_owned()))?;
        if let Some(next_state) = next_state {
            state = Some(Arc::new(next_state));
            negotiated.send_replace(state.clone());
        }
        if shutdown {
            return Ok(());
        }
    }
}
