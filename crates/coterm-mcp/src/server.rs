//! Tools-only MCP MVP: explicit plugin tools and host operation queries.
use std::{net::SocketAddr, sync::Arc, time::Duration};

use coterm_core::{
    plugin_manager::ProcessPluginRuntime,
    plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
};
use coterm_proto::runtime::{
    lifecycle::InstanceInfo,
    operation::{OperationReceipt, OperationSource, OperationStatus},
    RuntimeError,
};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::{
    io::BufReader,
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

use crate::transport::{read_frame, write_frame};

/// Fixed loopback endpoint used by the desktop MCP bridge.
pub const DEFAULT_BRIDGE_ADDRESS: &str = "127.0.0.1:51234";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionInfo {
    pub address: String,
}

pub struct BridgeServer {
    info: McpConnectionInfo,
    task: JoinHandle<()>,
}

impl BridgeServer {
    /// Bind the production bridge to the stable local endpoint.
    pub async fn bind(runtime: Arc<ProcessPluginRuntime>) -> std::io::Result<Self> {
        let address = DEFAULT_BRIDGE_ADDRESS
            .parse()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        Self::bind_at(runtime, address).await
    }

    /// Bind a bridge at an explicit address. Tests use port `0` to avoid
    /// competing with another bridge process.
    pub async fn bind_at(
        runtime: Arc<ProcessPluginRuntime>,
        address: SocketAddr,
    ) -> std::io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Coterm MCP requires a loopback address",
            ));
        }
        let listener = TcpListener::bind(address).await?;
        let info = McpConnectionInfo {
            address: listener.local_addr()?.to_string(),
        };
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, peer)) = accepted else { break };
                        if !peer.ip().is_loopback() || clients.len() >= 8 { continue; }
                        let runtime = runtime.clone();
                        clients.spawn(async move { let _ = serve_client(socket, runtime).await; });
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

async fn serve_client(socket: TcpStream, runtime: Arc<ProcessPluginRuntime>) -> anyhow::Result<()> {
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
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
    Ok(json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": true}},
        "serverInfo": {"name": "Coterm", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Instance-scoped tools use the public instance name from coterm.list_instances; UUIDs are internal and must not be supplied. Tools wait up to 60s including approval and human interaction. Serial tools return as soon as a write enters approval, with status pending_approval; use coterm.get_operation to query its final result. Set tools/call.params._meta['coterm/waitTimeoutMs'] to 0 for an immediate receipt, or 1..300000 to bound waiting. Only query coterm.get_operation after a pending/expired receipt; never pass credentials or replay writes automatically. Agent commit operations require approval unless the user has enabled Bypass or an explicit allowlist entry."
    }))
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
        json!({"name":"coterm.list_instances","description":"List existing shared plugin instances. Each instance has a unique public name; internal UUIDs are not exposed.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}),
        json!({"name":"coterm.get_operation","description":"Query an operation using its instance name or plugin scope.","inputSchema":{"type":"object","properties":{"name":{"type":"string","minLength":1},"pluginId":{"type":"string","minLength":1},"operationId":{"type":"string","minLength":1}},"required":["operationId"],"oneOf":[{"required":["name"],"not":{"required":["pluginId"]}},{"required":["pluginId"],"not":{"required":["name"]}}],"additionalProperties":false}}),
        json!({"name":"coterm.cancel_operation","description":"Request cancellation using an instance name or plugin scope. The result can be cancelled, unknown, or unsupported depending on protocol confirmation.","inputSchema":{"type":"object","properties":{"name":{"type":"string","minLength":1},"pluginId":{"type":"string","minLength":1},"operationId":{"type":"string","minLength":1}},"required":["operationId"],"oneOf":[{"required":["name"],"not":{"required":["pluginId"]}},{"required":["pluginId"],"not":{"required":["name"]}}],"additionalProperties":false}}),
    ];
    for (plugin, registration) in runtime.native_mcp_tools().await {
        let mut descriptor = public_tool_descriptor(registration.descriptor);
        if let Some(name) = descriptor["name"].as_str() {
            descriptor["name"] = json!(format!("{plugin}__{name}"));
        }
        tools.push(descriptor);
    }
    Ok(json!({"tools":tools}))
}

/// Hide the runtime routing key from the MCP contract. The bridge resolves `name`
/// before forwarding the request, while plugins continue to receive their internal ID.
fn public_tool_descriptor(mut descriptor: Value) -> Value {
    let Some(schema) = descriptor.get_mut("inputSchema") else {
        return descriptor;
    };
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return descriptor;
    };
    if properties.remove("pluginInstanceId").is_none() {
        return descriptor;
    }
    properties.insert(
        "name".to_owned(),
        json!({
            "type":"string",
            "minLength":1,
            "description":"Required public instance name returned by coterm.list_instances. The bridge also accepts instance, instanceName, or instance_name as compatibility aliases."
        }),
    );
    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        for entry in required {
            if entry.as_str() == Some("pluginInstanceId") {
                *entry = json!("name");
            }
        }
    }
    descriptor
}

fn public_instance(instance: &InstanceInfo) -> Value {
    json!({
        "name": instance.label,
        "pluginId": instance.plugin_id,
        "status": instance.status,
    })
}

fn hide_internal_instance_ids(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("pluginInstanceId");
            for child in object.values_mut() {
                hide_internal_instance_ids(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                hide_internal_instance_ids(item);
            }
        }
        _ => {}
    }
}

async fn resolve_instance_name(
    runtime: &ProcessPluginRuntime,
    plugin_id: Option<&str>,
    name: &str,
) -> anyhow::Result<InstanceInfo> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("Instance name must not be empty");
    }
    let matches: Vec<_> = runtime
        .list_instances(plugin_id)
        .await?
        .into_iter()
        .filter(|instance| instance.label == name)
        .collect();
    match matches.as_slice() {
        [instance] => Ok(instance.clone()),
        [] => anyhow::bail!("Instance name not found: {name}"),
        _ => anyhow::bail!("Instance name is not unique: {name}"),
    }
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
    let mut args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !args.is_object() {
        anyhow::bail!("Tool arguments must be an object");
    }
    if name == "coterm.list_instances" {
        let instances = runtime
            .list_instances(None)
            .await?
            .iter()
            .map(public_instance)
            .collect::<Vec<_>>();
        return Ok(text_result(json!({"instances":instances})));
    }
    if name == "coterm.get_operation" || name == "coterm.cancel_operation" {
        let instance_name = args["name"].as_str();
        let plugin_id = args["pluginId"].as_str();
        if instance_name.is_some() == plugin_id.is_some() {
            anyhow::bail!("Specify exactly one name or pluginId");
        }
        let (authorized_plugin, session_id, instance_id) =
            if let Some(instance_name) = instance_name {
                let instance = resolve_instance_name(runtime, None, instance_name).await?;
                (
                    instance.plugin_id,
                    instance.session_id,
                    Some(instance.plugin_instance_id),
                )
            } else {
                (
                    plugin_id.unwrap_or_default().to_owned(),
                    String::new(),
                    None,
                )
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
        if record.plugin_instance_id.as_deref() != instance_id.as_deref() {
            anyhow::bail!("Operation target mismatch");
        }
        return if name == "coterm.get_operation" {
            let mut value = serde_json::to_value(record)?;
            hide_internal_instance_ids(&mut value);
            Ok(text_result(value))
        } else {
            let mut value = serde_json::to_value(runtime.cancel(caller, id).await?)?;
            hide_internal_instance_ids(&mut value);
            Ok(text_result(value))
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
    let instance_id = if descriptor.scope
        == coterm_proto::runtime::operation::OperationScope::Instance
    {
        let target_fields = ["name", "instance", "instanceName", "instance_name"]
            .into_iter()
            .filter(|field| args.get(*field).is_some())
            .collect::<Vec<_>>();
        if target_fields.len() != 1 {
            anyhow::bail!(
                "Provide exactly one instance name using name, instance, instanceName, or instance_name"
            );
        }
        let public_name = args[target_fields[0]]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Instance name must be a string"))?;
        let instance = resolve_instance_name(runtime, Some(&plugin), public_name).await?;
        let instance_id = instance.plugin_instance_id.clone();
        let object = args
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Tool arguments must be an object"))?;
        object.remove("name");
        object.remove("instance");
        object.remove("instanceName");
        object.remove("instance_name");
        object.insert("pluginInstanceId".to_owned(), json!(&instance_id));
        Some(instance_id)
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
    let return_on_approval = tool.descriptor["_meta"]["coterm/returnOnApproval"] == true;
    let request = SubmitOperation {
        plugin_id: plugin,
        plugin_instance_id: instance_id,
        operation: tool.operation,
        args,
        wait_timeout_ms,
    };
    let receipt = if return_on_approval {
        runtime.submit_until_approval(caller, request).await?
    } else {
        runtime.submit(caller, request).await?
    };
    let public_status = if return_on_approval && receipt.status == OperationStatus::AwaitingApproval {
        json!("pending_approval")
    } else {
        serde_json::to_value(receipt.status)?
    };
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
            json!({"operationId":receipt.operation_id,"status":public_status,"proposalId":receipt.proposal_id,"error":receipt.error}),
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

    #[tokio::test]
    async fn bridge_rejects_non_loopback_bind_addresses() {
        let runtime = Arc::new(ProcessPluginRuntime::new(
            "test",
            coterm_proto::runtime::lifecycle::RuntimeLimits {
                max_control_bytes: 1024,
                max_data_chunk_bytes: 1024,
                initial_window: coterm_proto::runtime::stream::FlowWindow {
                    chunks: 1,
                    bytes: 1024,
                },
            },
        ));
        let address: SocketAddr = "192.0.2.1:51234".parse().expect("address");
        let result = BridgeServer::bind_at(runtime, address).await;
        assert!(
            matches!(result, Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
    }

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
        assert!(
            text.len() <= DIGEST_LIMIT,
            "digest was {} bytes",
            text.len()
        );
        assert!(text.contains("omitted"));
        assert!(!text.contains(&"y".repeat(1024)));
    }

    #[test]
    fn small_values_keep_their_full_structured_form_in_text() {
        let value = json!({"instances": []});
        assert_eq!(digest(&value), value.to_string());
    }

    #[test]
    fn public_values_do_not_expose_internal_instance_ids() {
        let mut value = json!({
            "pluginInstanceId": "private",
            "output": {"pluginInstanceId": "also-private", "text": "ok"},
        });
        hide_internal_instance_ids(&mut value);
        assert_eq!(value, json!({"output": {"text": "ok"}}));
    }

    #[test]
    fn public_tool_schema_uses_name_for_instance_routing() {
        let descriptor = public_tool_descriptor(json!({
            "name": "serial.status",
            "inputSchema": {
                "type": "object",
                "properties": {"pluginInstanceId": {"type": "string"}},
                "required": ["pluginInstanceId"],
            }
        }));
        assert!(descriptor["inputSchema"]["properties"]
            .get("pluginInstanceId")
            .is_none());
        assert_eq!(descriptor["inputSchema"]["required"], json!(["name"]));
        assert_eq!(
            descriptor["inputSchema"]["properties"]["name"]["type"],
            "string"
        );
        assert!(
            descriptor["inputSchema"]["properties"]["name"]["description"]
                .as_str()
                .is_some_and(|description| {
                    description.contains("coterm.list_instances")
                        && description.contains("instanceName")
                })
        );
    }
}
