//! 插件后端与基座之间的 stdio 传输骨架。
//!
//! 物理编码是「一行一个 JSON-RPC 2.0 消息」：基座按命令种类发不同的方法名（`initialize`、
//! `invoke`、`create_instance` …），插件回 `JsonRpcResponse`，插件主动推送走
//! `coterm.notification`。信封、读行、大小上限和输出串行化在这一层统一，插件只实现
//! [`PluginBackend`] 的领域逻辑。

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::message::{HostCommand, PluginNotification};
use super::operation::InvokeRequest;
use super::lifecycle::CreateInstance;
use super::interaction::SecretDelivery;
use super::RuntimeError;
use crate::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};

/// 插件发往基座的通知方法名。
pub const NOTIFICATION_METHOD: &str = "coterm.notification";
/// 与基座控制行上限对齐；超出一行的请求按协议错误处理。
pub const MAX_CONTROL_BYTES: usize = 1024 * 1024;
/// 默认 tick：只驱动过期清理，不影响操作处理延迟。
pub const DEFAULT_TICK: Duration = Duration::from_secs(1);

/// 框架级失败（回复未能产生）使用的错误码。
const FRAMEWORK_ERROR: i32 = -32000;

/// 插件实现的领域逻辑。
#[allow(async_fn_in_trait)]
pub trait PluginBackend: Send {
    /// 处理带 `request_id` 的控制命令。
    async fn request(&mut self, command: HostCommand) -> Result<serde_json::Value, RuntimeError>;

    /// 周期钩子；默认不做事。用于清理过期交互与凭据。
    fn tick(&mut self) {}

    /// 通知接收端，由基座在 [`serve`] 开始时取出。
    fn take_notifications(&mut self) -> Option<tokio::sync::mpsc::Receiver<PluginNotification>>;
}

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

/// 把一行控制请求解析为 `(request_id, command, is_shutdown)`。
///
/// 方法名与 `coterm-core::plugin_process::to_json_rpc` 一一对应；两者是同一契约的两端。
fn parse_command(line: &[u8]) -> Result<(String, HostCommand, bool), TransportError> {
    let request: JsonRpcRequest = serde_json::from_slice(line)
        .map_err(|_| TransportError::Protocol("invalid plugin runtime JSON-RPC".to_owned()))?;
    if request.jsonrpc != "2.0" {
        return Err(TransportError::Protocol(
            "invalid JSON-RPC version".to_owned(),
        ));
    }
    let id = request
        .id
        .and_then(|id| id.as_str().map(str::to_owned))
        .ok_or_else(|| {
            TransportError::Protocol("plugin runtime request id must be a string".to_owned())
        })?;
    let params = request.params;
    let decode = |error: serde_json::Error| TransportError::Protocol(error.to_string());
    let command = match request.method.as_str() {
        "initialize" => HostCommand::Hello(serde_json::from_value(params).map_err(decode)?),
        "create_instance" => {
            HostCommand::CreateInstance(serde_json::from_value::<CreateInstance>(params).map_err(decode)?)
        }
        "destroy_instance" => HostCommand::DestroyInstance {
            plugin_instance_id: string_param(&params, "pluginInstanceId")?,
        },
        "invoke" => HostCommand::Invoke(serde_json::from_value::<InvokeRequest>(params).map_err(decode)?),
        "approve" => {
            HostCommand::Approve(serde_json::from_value(params).map_err(decode)?)
        }
        "reject" => HostCommand::Reject {
            operation_id: string_param(&params, "operationId")?,
            proposal_id: string_param(&params, "proposalId")?,
            reason: params
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "cancel" => HostCommand::Cancel(serde_json::from_value(params).map_err(decode)?),
        "interaction_result" => {
            HostCommand::InteractionResult(serde_json::from_value(params).map_err(decode)?)
        }
        "provide_secret" => {
            HostCommand::ProvideSecret(serde_json::from_value::<SecretDelivery>(params).map_err(decode)?)
        }
        "stream_subscribe" => {
            HostCommand::StreamSubscribe(serde_json::from_value(params).map_err(decode)?)
        }
        "stream_unsubscribe" => HostCommand::StreamUnsubscribe {
            subscription_id: string_param(&params, "subscriptionId")?,
        },
        "stream_credit" => HostCommand::StreamCredit {
            subscription_id: string_param(&params, "subscriptionId")?,
            credit: serde_json::from_value(
                params.get("credit").cloned().unwrap_or(Value::Null),
            )
            .map_err(decode)?,
        },
        "shutdown" => HostCommand::Shutdown,
        other => {
            return Err(TransportError::Protocol(format!(
                "unsupported plugin runtime method: {other}"
            )))
        }
    };
    let shutdown = matches!(command, HostCommand::Shutdown);
    Ok((id, command, shutdown))
}

fn string_param(params: &Value, field: &str) -> Result<String, TransportError> {
    params
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| TransportError::Protocol(format!("missing or invalid {field}")))
}

fn reply(id: String, result: Result<serde_json::Value, RuntimeError>) -> JsonRpcResponse {
    match result {
        Ok(result) => JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id: serde_json::Value::String(id),
            result: Some(result),
            error: None,
        },
        Err(error) => JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id: serde_json::Value::String(id),
            result: None,
            error: Some(JsonRpcError {
                code: FRAMEWORK_ERROR,
                message: error.to_string(),
                data: None,
            }),
        },
    }
}

/// 读取一条控制行，并在等待期间按 `tick` 驱动插件钩子；返回 `None` 表示 stdin 已关闭。
///
/// tick 与读取共用一个循环，而不是在 `select!` 里竞争：`fill_buf` 被取消会丢掉已经
/// 从内核取出但尚未消费的字节，因此读取本身不能被取消，只能在其内部让出等待。
async fn read_line_with_tick<B: PluginBackend, R: tokio::io::AsyncBufRead + Unpin>(
    backend: &mut B,
    reader: &mut R,
    tick: Duration,
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
        if line.len() + count > MAX_CONTROL_BYTES {
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

/// 与 [`serve`] 相同，但可指定 tick 间隔；测试用以加速过期路径。
pub async fn serve_with_tick<B: PluginBackend>(
    mut backend: B,
    tick: Duration,
) -> Result<(), TransportError> {
    let mut notifications = backend
        .take_notifications()
        .ok_or_else(|| TransportError::Protocol("notification receiver unavailable".to_owned()))?;
    let (replies, mut responses) = tokio::sync::mpsc::channel::<JsonRpcResponse>(16);
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        let mut responses_open = true;
        let mut notifications_open = true;
        while responses_open || notifications_open {
            let value = tokio::select! {
                biased;
                response = responses.recv(), if responses_open => match response {
                    Some(response) => serde_json::to_value(response)?,
                    None => { responses_open = false; continue; }
                },
                notification = notifications.recv(), if notifications_open => match notification {
                    // `id` 必须缺省而非 null：基座用 `get("id").is_none()` 区分通知与回复。
                    Some(notification) => serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": NOTIFICATION_METHOD,
                        "params": serde_json::to_value(notification)?,
                    }),
                    None => { notifications_open = false; continue; }
                }
            };
            stdout.write_all(&serde_json::to_vec(&value)?).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let outcome = read_loop(&mut backend, &replies, tick).await;
    drop(backend);
    drop(replies);
    match writer.await {
        Ok(written) => written.map_err(|error| TransportError::Protocol(error.to_string()))?,
        Err(error) => {
            return Err(TransportError::Protocol(format!(
                "stdout writer failed: {error}"
            )))
        }
    }
    outcome
}

async fn read_loop<B: PluginBackend>(
    backend: &mut B,
    replies: &tokio::sync::mpsc::Sender<JsonRpcResponse>,
    tick: Duration,
) -> Result<(), TransportError> {
    let mut reader = BufReader::new(tokio::io::stdin());
    loop {
        let Some(mut line) = read_line_with_tick(backend, &mut reader, tick).await? else {
            return Ok(());
        };
        let parsed = parse_command(&line);
        // 原始行可能包含私密凭据投递。
        zeroize::Zeroize::zeroize(&mut line);
        let (id, command, shutdown) = parsed?;
        let result = backend.request(command).await;
        replies
            .send(reply(id, result))
            .await
            .map_err(|_| TransportError::Protocol("plugin stdout writer closed".to_owned()))?;
        if shutdown {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_command, HostCommand, MAX_CONTROL_BYTES};
    use crate::runtime::lifecycle::RuntimeHello;
    use crate::runtime::operation::{DispatchMode, InvokeRequest, OperationEffect, OperationSource};
    use serde_json::json;

    fn line(method: &str, params: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": method,
            "params": params,
        }))
        .unwrap()
    }

    fn hello() -> serde_json::Value {
        serde_json::to_value(RuntimeHello {
            interface_major: 1,
            interface_minor: 0,
            required_features: vec!["instances".to_owned()],
            optional_features: vec![],
            limits: crate::runtime::lifecycle::RuntimeLimits {
                max_control_bytes: 1024,
                max_data_chunk_bytes: 1024,
                initial_window: crate::runtime::stream::FlowWindow {
                    chunks: 1,
                    bytes: 1024,
                },
            },
        })
        .unwrap()
    }

    // 方法名与参数形状必须与 coterm-core::plugin_process::to_json_rpc 保持一致。
    #[test]
    fn decodes_every_host_command_method() {
        let (id, command, shutdown) = parse_command(&line("initialize", hello())).unwrap();
        assert_eq!(id, "req-1");
        assert!(matches!(command, HostCommand::Hello(_)));
        assert!(!shutdown);

        let (_, command, _) = parse_command(&line(
            "create_instance",
            json!({
                "instance": {
                    "pluginId": "p",
                    "pluginInstanceId": "i",
                    "sessionId": "s",
                    "label": "l",
                    "status": "starting",
                },
                "config": {},
            }),
        ))
        .unwrap();
        assert!(matches!(command, HostCommand::CreateInstance(_)));

        let (_, command, _) =
            parse_command(&line("destroy_instance", json!({"pluginInstanceId": "i"}))).unwrap();
        assert!(matches!(
            command,
            HostCommand::DestroyInstance { plugin_instance_id } if plugin_instance_id == "i"
        ));

        let (_, command, _) = parse_command(&line(
            "invoke",
            serde_json::to_value(InvokeRequest {
                operation_id: "op".to_owned(),
                plugin_instance_id: Some("i".to_owned()),
                source: OperationSource::Agent,
                effect: OperationEffect::Commit,
                operation: "serial.write".to_owned(),
                args: json!({}),
                dispatch: DispatchMode::Execute,
            })
            .unwrap(),
        ))
        .unwrap();
        assert!(matches!(command, HostCommand::Invoke(_)));

        let (_, command, _) = parse_command(&line(
            "reject",
            json!({"operationId": "op", "proposalId": "pr", "reason": "no"}),
        ))
        .unwrap();
        assert!(matches!(
            command,
            HostCommand::Reject { proposal_id, .. } if proposal_id == "pr"
        ));

        let (_, command, _) = parse_command(&line(
            "stream_unsubscribe",
            json!({"subscriptionId": "sub"}),
        ))
        .unwrap();
        assert!(matches!(
            command,
            HostCommand::StreamUnsubscribe { subscription_id } if subscription_id == "sub"
        ));

        let (_, command, shutdown) = parse_command(&line("shutdown", json!(null))).unwrap();
        assert!(matches!(command, HostCommand::Shutdown));
        assert!(shutdown);
    }

    #[test]
    fn rejects_malformed_control_lines() {
        assert!(parse_command(b"not json").is_err());
        assert!(parse_command(&serde_json::to_vec(&json!({"method": "x"})).unwrap()).is_err());
        assert!(parse_command(&line("not_a_method", json!({}))).is_err());
        // id 必须是字符串，数字会让基座无法关联回复。
        assert!(parse_command(
            &serde_json::to_vec(&json!({"jsonrpc":"2.0","id":7,"method":"shutdown","params":null}))
                .unwrap()
        )
        .is_err());
        // params 与声明的方法不匹配时按协议错误处理，不静默降级。
        assert!(parse_command(&line("invoke", json!({"nonsense": true}))).is_err());
        assert!(parse_command(&line("destroy_instance", json!({}))).is_err());
    }

    // 基座用 `get("id").is_none()` 区分通知与回复；出现 "id":null 会被判为非法回复。
    #[test]
    fn notification_envelope_omits_id() {
        let value = serde_json::json!({
            "jsonrpc": "2.0",
            "method": super::NOTIFICATION_METHOD,
            "params": {"type": "event"},
        });
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["method"], "coterm.notification");
        assert!(value.get("id").is_none());
    }

    #[test]
    fn control_line_limit_matches_host() {
        assert_eq!(MAX_CONTROL_BYTES, 1024 * 1024);
    }
}
