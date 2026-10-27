//! Tools-only MCP MVP: explicit plugin tools and host operation queries.
use std::{net::SocketAddr, sync::Arc, time::Duration};

use coterm_core::{
    plugin_manager::ProcessPluginRuntime,
    plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
};
use coterm_proto::runtime::{
    operation::{OperationReceipt, OperationSource, OperationStatus},
    RuntimeError,
};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::{
    io::BufReader,
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time::timeout,
};

use crate::transport::{read_frame, write_frame};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionInfo {
    pub address: String,
    /// Private UI command only; never log, persist, or publish in an event.
    pub token: String,
}

pub struct BridgeServer {
    info: McpConnectionInfo,
    task: JoinHandle<()>,
}

impl BridgeServer {
    pub async fn bind(runtime: Arc<ProcessPluginRuntime>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let info = McpConnectionInfo {
            address: listener.local_addr()?.to_string(),
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        };
        let token = info.token.clone();
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, peer)) = accepted else { break };
                        if !peer.ip().is_loopback() || clients.len() >= 8 { continue; }
                        let runtime = runtime.clone();
                        let token = token.clone();
                        clients.spawn(async move { let _ = serve_client(socket, runtime, &token).await; });
                    }
                    _ = clients.join_next(), if !clients.is_empty() => {}
                }
            }
        });
        Ok(Self { info, task })
    }

    pub fn connection_info(&self) -> McpConnectionInfo {
        self.info.clone()
    }
}

impl Drop for BridgeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_client(
    socket: TcpStream,
    runtime: Arc<ProcessPluginRuntime>,
    token: &str,
) -> anyhow::Result<()> {
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    let mut auth = timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await??
        .ok_or_else(|| anyhow::anyhow!("authentication required"))?;
    let supplied: Value = serde_json::from_slice(&auth)?;
    auth.fill(0);
    let valid = supplied
        .get("token")
        .and_then(Value::as_str)
        .is_some_and(|s| {
            s.len() == token.len()
                && s.bytes()
                    .zip(token.bytes())
                    .fold(0u8, |v, (a, b)| v | (a ^ b))
                    == 0
        });
    if !valid {
        anyhow::bail!("authentication failed");
    }
    write_frame(&mut writer, &json!({"authenticated":true})).await?;
    let mut initialized = false;
    let mut ready = false;
    let mut last_tools = Value::Null;
    let mut catalog_poll = tokio::time::interval(Duration::from_secs(1));
    let (responses, mut completed) = tokio::sync::mpsc::channel::<Value>(16);
    let slots = Arc::new(tokio::sync::Semaphore::new(16));
    let mut calls = JoinSet::new();
    loop {
        // Keep the read future alive across timer ticks: cancelling a partial
        // newline frame would otherwise discard its prefix and corrupt the wire.
        let incoming = read_frame(&mut reader);
        tokio::pin!(incoming);
        let frame = loop {
            tokio::select! {
                frame = &mut incoming => break frame?,
                Some(response) = completed.recv() => { write_frame(&mut writer, &response).await?; }
                _ = calls.join_next(), if !calls.is_empty() => {}
                _ = catalog_poll.tick(), if ready => {
                    let tools = tools_list(&runtime).await?;
                    if tools != last_tools {
                        last_tools = tools;
                        write_frame(&mut writer, &json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"})).await?;
                    }
                }
            }
        };
        let Some(frame) = frame else { break };
        let value: Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(_) => {
                write_frame(&mut writer, &rpc_error(Value::Null, -32700, "Parse error")).await?;
                continue;
            }
        };
        let id = value.get("id").cloned();
        let valid_id = id
            .as_ref()
            .is_none_or(|id| id.is_string() || id.is_number());
        if !value.is_object()
            || value["jsonrpc"] != "2.0"
            || !value["method"].is_string()
            || !valid_id
        {
            write_frame(
                &mut writer,
                &rpc_error(Value::Null, -32600, "Invalid Request"),
            )
            .await?;
            continue;
        }
        let method = value["method"].as_str().unwrap_or_default();
        if id.is_none() {
            if method == "notifications/initialized" && initialized {
                ready = true;
                last_tools = tools_list(&runtime).await?;
            }
            // Waiting calls do not block reading; operation cancellation is explicit.
            continue;
        }
        let id = id.unwrap_or(Value::Null);
        let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
        if ready
            && method == "tools/call"
            && params["name"].is_string()
            && params.get("arguments").is_none_or(Value::is_object)
        {
            let Ok(permit) = slots.clone().try_acquire_owned() else {
                write_frame(
                    &mut writer,
                    &rpc_error(id, -32000, "Too many in-flight calls"),
                )
                .await?;
                continue;
            };
            let runtime = runtime.clone();
            let responses = responses.clone();
            calls.spawn(async move {
                let _permit = permit;
                let result = match call_tool(&runtime, &params).await {
                    Ok(value) => value,
                    Err(error) => {
                        json!({"content":[{"type":"text","text":error.to_string()}],"isError":true})
                    }
                };
                let _ = responses
                    .send(json!({"jsonrpc":"2.0","id":id,"result":result}))
                    .await;
            });
            continue;
        }
        let result = match method {
            "initialize" if !initialized => {
                let result = initialize_result(&params);
                initialized = result.is_ok();
                result
            }
            "ping" if initialized => Ok(json!({})),
            _ if !ready => Err((-32000, "MCP initialization required".to_owned())),
            _ => ready_request(&runtime, method, &params).await,
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, message)) => rpc_error(id, code, &message),
        };
        write_frame(&mut writer, &response).await?;
    }
    Ok(())
}

pub(crate) fn initialize_result(params: &Value) -> Result<Value, (i32, String)> {
    if !params["protocolVersion"].is_string()
        || !params["capabilities"].is_object()
        || !params["clientInfo"]["name"].is_string()
        || !params["clientInfo"]["version"].is_string()
    {
        return Err((-32602, "Invalid initialize parameters".into()));
    }
    let requested = params["protocolVersion"].as_str().unwrap_or_default();
    let version = if ["2025-06-18", "2025-03-26", "2024-11-05"].contains(&requested) {
        requested
    } else {
        "2025-06-18"
    };
    Ok(
        json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":true}},"serverInfo":{"name":"Coterm","version":env!("CARGO_PKG_VERSION")},"instructions":"Use explicit pluginInstanceId. Tools wait up to 60s including approval and human interaction. Set params._meta['coterm/waitTimeoutMs'] to 0 for an immediate receipt, or 1..300000 to bound waiting. Only query coterm.get_operation after a pending/expired receipt; never pass credentials or replay writes automatically."}),
    )
}

pub(crate) async fn ready_request(
    runtime: &ProcessPluginRuntime,
    method: &str,
    params: &Value,
) -> Result<Value, (i32, String)> {
    match method {
        "ping" => Ok(json!({})),
        "tools/list" => tools_list(runtime)
            .await
            .map_err(|e| (-32603, e.to_string())),
        "tools/call"
            if !params["name"].is_string()
                || params.get("arguments").is_some_and(|a| !a.is_object()) =>
        {
            Err((-32602, "Invalid tool call parameters".into()))
        }
        "tools/call" => Ok(match call_tool(runtime, params).await {
            Ok(value) => value,
            Err(error) => {
                json!({"content":[{"type":"text","text":error.to_string()}],"isError":true})
            }
        }),
        _ => Err((-32601, "Method not found".into())),
    }
}

pub(crate) fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

pub async fn tools_list(runtime: &ProcessPluginRuntime) -> Result<Value, RuntimeError> {
    let mut tools = vec![
        json!({"name":"coterm.list_instances","description":"List existing shared plugin instances. Create and configure instances in Coterm.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}),
        json!({"name":"coterm.get_operation","description":"Query an operation using its explicit pluginInstanceId or pluginId scope.","inputSchema":{"type":"object","properties":{"pluginInstanceId":{"type":"string"},"pluginId":{"type":"string"},"operationId":{"type":"string"}},"required":["operationId"],"oneOf":[{"required":["pluginInstanceId"],"not":{"required":["pluginId"]}},{"required":["pluginId"],"not":{"required":["pluginInstanceId"]}}],"additionalProperties":false}}),
        json!({"name":"coterm.cancel_operation","description":"Request cancellation. The result can be cancelled, unknown, or unsupported depending on protocol confirmation.","inputSchema":{"type":"object","properties":{"pluginInstanceId":{"type":"string"},"pluginId":{"type":"string"},"operationId":{"type":"string"}},"required":["operationId"],"oneOf":[{"required":["pluginInstanceId"],"not":{"required":["pluginId"]}},{"required":["pluginId"],"not":{"required":["pluginInstanceId"]}}],"additionalProperties":false}}),
    ];
    for (plugin, registration) in runtime.native_mcp_tools().await {
        let mut descriptor = registration.descriptor;
        if let Some(name) = descriptor["name"].as_str() {
            descriptor["name"] = json!(format!("{plugin}__{name}"));
        }
        tools.push(descriptor);
    }
    Ok(json!({"tools":tools}))
}

/// Structured content is the machine-readable copy. The text block is a bounded digest,
/// so a large operation result is not serialized twice in one response.
fn text_result(value: Value) -> Value {
    let text = digest(&value);
    json!({"content":[{"type":"text","text":text}],"structuredContent":value,"isError":false})
}

const DIGEST_LIMIT: usize = 2048;

fn digest(value: &Value) -> String {
    let encoded = value.to_string();
    if encoded.len() <= DIGEST_LIMIT {
        return encoded;
    }
    // Operation records can embed both the request and its result. Replace each large
    // field with a size marker so the text block stays a digest of the same response.
    let mut compact = value.clone();
    if let Some(object) = compact.as_object_mut() {
        for field in ["output", "input", "result"] {
            let Some(full) = object.get(field) else {
                continue;
            };
            let bytes = serde_json::to_vec(full).map_or(0, |bytes| bytes.len());
            if bytes > 256 {
                object.insert(field.into(), json!({"omitted": true, "bytes": bytes}));
            }
        }
    }
    let encoded = compact.to_string();
    if encoded.len() <= DIGEST_LIMIT {
        return encoded;
    }
    format!(
        "structuredContent has {len} bytes; read structuredContent for the full value",
        len = encoded.len()
    )
}

/// The operation receipt attached to every tool result. Output already lives in the tool
/// result, so the receipt only reports its size.
fn receipt_meta(receipt: &OperationReceipt) -> Value {
    let mut value = serde_json::to_value(receipt).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        if let Some(output) = object.remove("output") {
            object.insert(
                "output".into(),
                json!({"omitted": true, "bytes": serde_json::to_vec(&output).map_or(0, |bytes| bytes.len())}),
            );
        }
    }
    value
}

pub async fn call_tool(runtime: &ProcessPluginRuntime, params: &Value) -> anyhow::Result<Value> {
    let wait_timeout_ms = params
        .pointer("/_meta/coterm~1waitTimeoutMs")
        .map(|value| {
            value
                .as_u64()
                .filter(|n| *n <= 300_000)
                .ok_or_else(|| anyhow::anyhow!("Invalid coterm/waitTimeoutMs (expected 0..300000)"))
        })
        .transpose()?;
    let name = params["name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Tool name required"))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !args.is_object() {
        anyhow::bail!("Tool arguments must be an object");
    }
    if name == "coterm.list_instances" {
        return Ok(text_result(
            json!({"instances":runtime.list_instances(None).await?}),
        ));
    }
    if name == "coterm.get_operation" || name == "coterm.cancel_operation" {
        let instance_id = args["pluginInstanceId"].as_str();
        let plugin_id = args["pluginId"].as_str();
        if instance_id.is_some() == plugin_id.is_some() {
            anyhow::bail!("Specify exactly one pluginInstanceId or pluginId");
        }
        let (authorized_plugin, session_id) = if let Some(instance_id) = instance_id {
            let instance = runtime
                .get_instance(instance_id)
                .await
                .ok_or_else(|| anyhow::anyhow!("Instance not found"))?;
            (instance.plugin_id, instance.session_id)
        } else {
            (plugin_id.unwrap_or_default().to_owned(), String::new())
        };
        let id = args["operationId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("operationId required"))?;
        let caller = CallerContext {
            source: OperationSource::Agent,
            session_id,
            plugin_id: Some(authorized_plugin),
        };
        let record = runtime.get(caller.clone(), id).await?;
        if record.plugin_instance_id.as_deref() != instance_id {
            anyhow::bail!("Operation target mismatch");
        }
        return if name == "coterm.get_operation" {
            Ok(text_result(serde_json::to_value(record)?))
        } else {
            Ok(text_result(serde_json::to_value(
                runtime.cancel(caller, id).await?,
            )?))
        };
    }
    let native = runtime.native_mcp_tools().await;
    let (plugin, tool) = native
        .into_iter()
        .find(|(plugin, tool)| {
            tool.descriptor["name"]
                .as_str()
                .is_some_and(|native| format!("{plugin}__{native}") == name)
        })
        .ok_or_else(|| anyhow::anyhow!("Tool not found"))?;
    let descriptor = runtime
        .operation_catalog(&plugin)
        .await
        .into_iter()
        .find(|op| op.name == tool.operation)
        .ok_or_else(|| anyhow::anyhow!("Operation descriptor not found"))?;
    let instance_id =
        if descriptor.scope == coterm_proto::runtime::operation::OperationScope::Instance {
            Some(
                args["pluginInstanceId"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Explicit pluginInstanceId required"))?
                    .to_owned(),
            )
        } else {
            None
        };
    let session_id = if let Some(instance_id) = instance_id.as_deref() {
        let instance = runtime
            .get_instance(instance_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Instance not found"))?;
        if instance.plugin_id != plugin {
            anyhow::bail!("Tool instance mismatch");
        }
        instance.session_id
    } else {
        String::new()
    };
    // Source and plugin authorization are derived from the trusted tool directory.
    let caller = CallerContext {
        source: OperationSource::Agent,
        session_id,
        plugin_id: Some(plugin.clone()),
    };
    let receipt = runtime
        .submit(
            caller,
            SubmitOperation {
                plugin_id: plugin,
                plugin_instance_id: instance_id,
                operation: tool.operation,
                args,
                wait_timeout_ms,
            },
        )
        .await?;
    let mut result = if receipt.status == OperationStatus::Completed {
        let output = receipt
            .output
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Plugin MCP result missing"))?;
        if !output["content"].is_array() {
            anyhow::bail!("Plugin returned invalid MCP CallToolResult");
        }
        output
    } else {
        text_result(
            json!({"operationId":receipt.operation_id,"status":receipt.status,"proposalId":receipt.proposal_id,"error":receipt.error}),
        )
    };
    if receipt.error.is_some() {
        result["isError"] = json!(true);
    }
    if !result.get("_meta").is_some_and(Value::is_object) {
        result["_meta"] = json!({});
    }
    result["_meta"]["coterm/operation"] = receipt_meta(&receipt);
    Ok(result)
}

pub fn loopback_address(value: &str) -> anyhow::Result<SocketAddr> {
    let address: SocketAddr = value.parse()?;
    if !address.ip().is_loopback() {
        anyhow::bail!("Coterm MCP requires a loopback address");
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_report_output_size_instead_of_copying_it() {
        let receipt = OperationReceipt {
            operation_id: "op".into(),
            proposal_id: None,
            status: OperationStatus::Completed,
            wait_expired: false,
            output: Some(json!({"text":"x".repeat(4096)})),
            error: None,
        };
        let meta = receipt_meta(&receipt);
        assert_eq!(meta["output"]["omitted"], true);
        assert!(meta["output"].get("text").is_none());
        assert!(
            serde_json::to_vec(&meta).expect("encode").len() < 1024,
            "receipt must not repeat the payload"
        );
    }

    #[test]
    fn digests_stay_bounded_for_large_records() {
        let record = json!({
            "operationId": "op",
            "status": "completed",
            "operation": "ssh.command",
            "input": {"text": "x".repeat(4096)},
            "output": {"text": "y".repeat(8192)},
        });
        let text = digest(&record);
        assert!(text.len() <= DIGEST_LIMIT, "digest was {} bytes", text.len());
        assert!(text.contains("omitted"));
        assert!(!text.contains(&"y".repeat(1024)));
    }

    #[test]
    fn small_values_keep_their_full_structured_form_in_text() {
        let value = json!({"instances": []});
        assert_eq!(digest(&value), value.to_string());
    }
}
