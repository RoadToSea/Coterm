//! v1.0 JSON 控制协议的唯一编解码入口；不依赖进程或平台。

use super::{
    message::{HostCommand, HostMessage, PluginEvent},
    ErrorCategory, RuntimeError,
};
use crate::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde_json::{json, Value};

pub const NOTIFICATION_METHOD: &str = "coterm.notification";
/// UTF-8 字节数，包含结尾 LF（输入也接受 CRLF）。
pub const MAX_CONTROL_BYTES: usize = 1024 * 1024;

/// 插件内部适配器可复用方法映射，避免复制线缆字段。
pub fn command_params(command: HostCommand) -> Result<(&'static str, Value), RuntimeError> {
    let (method, params) = match command {
        HostCommand::Hello(hello) => ("initialize", serde_json::to_value(hello)),
        HostCommand::CreateInstance(instance) => {
            ("create_instance", serde_json::to_value(instance))
        }
        HostCommand::DestroyInstance { plugin_instance_id } => (
            "destroy_instance",
            serde_json::to_value(json!({ "pluginInstanceId": plugin_instance_id })),
        ),
        HostCommand::Invoke(request) => ("invoke", serde_json::to_value(request)),
        HostCommand::Approve(request) => ("approve", serde_json::to_value(request)),
        HostCommand::Reject {
            operation_id,
            proposal_id,
            reason,
        } => (
            "reject",
            serde_json::to_value(json!({
                "operationId": operation_id,
                "proposalId": proposal_id,
                "reason": reason,
            })),
        ),
        HostCommand::Cancel(target) => ("cancel", serde_json::to_value(target)),
        HostCommand::InteractionResult(result) => {
            ("interaction_result", serde_json::to_value(result))
        }
        HostCommand::ProvideSecret(secret) => ("provide_secret", serde_json::to_value(secret)),
        HostCommand::StreamSubscribe(_)
        | HostCommand::StreamUnsubscribe { .. }
        | HostCommand::StreamCredit { .. } => {
            return Err(RuntimeError {
                category: ErrorCategory::Unsupported,
                code: "runtime_stream_unsupported".to_owned(),
                message: "Streams are reserved outside the v1.0 control contract".to_owned(),
                details: None,
            })
        }
        HostCommand::Shutdown => ("shutdown", Ok(Value::Null)),
    };
    let params = params.map_err(|error| protocol_error_with("request_encode_failed", error))?;
    Ok((method, params))
}

pub fn encode_request(message: HostMessage, limit: usize) -> Result<Vec<u8>, RuntimeError> {
    let (method, params) = command_params(message.command)?;
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_owned(),
        id: Some(Value::String(message.request_id)),
        method: method.to_owned(),
        params,
    };
    encode_line(&request, limit)
}

/// 返回已带 LF 的完整行；超限时尚未写入任何字节。
pub fn encode_line(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>, RuntimeError> {
    let mut line = serde_json::to_vec(value)
        .map_err(|error| protocol_error_with("request_encode_failed", error))?;
    if line.len().saturating_add(1) > limit.min(MAX_CONTROL_BYTES) {
        return Err(protocol_error("plugin_control_too_large"));
    }
    line.push(b'\n');
    Ok(line)
}

#[derive(Debug)]
pub struct RequestDecodeError {
    pub id: Value,
    pub code: i32,
    pub message: &'static str,
}

impl RequestDecodeError {
    pub fn into_response(self) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id: self.id,
            result: None,
            error: Some(JsonRpcError {
                code: self.code,
                message: self.message.to_owned(),
                data: None,
            }),
        }
    }
}

pub fn decode_request(line: &[u8]) -> Result<HostMessage, RequestDecodeError> {
    let invalid = |id, code, message| RequestDecodeError { id, code, message };
    let value: Value =
        serde_json::from_slice(line).map_err(|_| invalid(Value::Null, -32700, "Parse error"))?;
    let id = value
        .get("id")
        .filter(|id| id.is_string())
        .cloned()
        .unwrap_or(Value::Null);
    let request: JsonRpcRequest = serde_json::from_value(value)
        .map_err(|_| invalid(id.clone(), -32600, "Invalid Request"))?;
    if request.jsonrpc != "2.0" || !id.is_string() {
        return Err(invalid(id, -32600, "Invalid Request"));
    }
    let method = request.method.as_str();
    // HostCommand 的内部 serde 表示不是线缆表示。
    let kind = match method {
        "initialize" => "hello",
        "create_instance" | "destroy_instance" | "invoke" | "approve" | "reject" | "cancel"
        | "interaction_result" | "provide_secret" | "shutdown" => method,
        _ => return Err(invalid(id, -32601, "Method not found")),
    };
    let command = serde_json::from_value(json!({"type":kind, "params":request.params}))
        .map_err(|_| invalid(id.clone(), -32602, "Invalid params"))?;
    Ok(HostMessage {
        request_id: id.as_str().unwrap_or_default().to_owned(),
        command,
    })
}

pub fn reply(id: String, result: Result<Value, RuntimeError>) -> JsonRpcResponse {
    let (result, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (
            None,
            Some(JsonRpcError {
                code: -32000,
                message: error.message.clone(),
                data: Some(json!({"category":error.category, "code":error.code,
                             "message":error.message, "details":error.details})),
            }),
        ),
    };
    JsonRpcResponse {
        jsonrpc: "2.0".to_owned(),
        id: Value::String(id),
        result,
        error,
    }
}

pub fn decode_response(line: &str) -> Result<PluginEvent, RuntimeError> {
    let response: Value = serde_json::from_str(line)
        .map_err(|error| protocol_error_with("plugin_response_invalid", error))?;
    if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(protocol_error("plugin_jsonrpc_version_invalid"));
    }
    let request_id = match response.get("id") {
        Some(Value::String(id)) => id.clone(),
        _ => return Err(protocol_error("plugin_response_id_invalid")),
    };
    let result = match (response.get("result"), response.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => {
            let error: JsonRpcError = serde_json::from_value(error.clone())
                .map_err(|cause| protocol_error_with("plugin_rpc_error_invalid", cause))?;
            Err(map_rpc_error(error))
        }
        _ => return Err(protocol_error("plugin_response_shape_invalid")),
    };
    Ok(PluginEvent::Reply { request_id, result })
}

fn map_rpc_error(error: JsonRpcError) -> RuntimeError {
    if error.code == -32000 {
        if let Some(data) = &error.data {
            if let Ok(runtime) = serde_json::from_value::<RuntimeError>(data.clone()) {
                return runtime;
            }
        }
    }
    // 接受旧插件和标准 JSON-RPC 错误，但不从展示文本猜测业务错误码。
    RuntimeError {
        category: match error.code {
            -32601 => ErrorCategory::Unsupported,
            -32700 | -32600 | -32602 => ErrorCategory::InvalidRequest,
            -32603 => ErrorCategory::Internal,
            _ => ErrorCategory::Domain,
        },
        code: format!("plugin_rpc_{}", error.code),
        message: error.message,
        details: error.data,
    }
}

fn protocol_error(code: &str) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Transport,
        code: code.to_owned(),
        message: code.replace('_', " "),
        details: None,
    }
}

fn protocol_error_with(code: &str, error: impl std::fmt::Display) -> RuntimeError {
    let mut result = protocol_error(code);
    result.details = Some(json!({ "cause": error.to_string() }));
    result
}
