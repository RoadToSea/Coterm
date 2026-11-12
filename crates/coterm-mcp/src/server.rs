//! Semantic MCP tools backed by driver operations and host approval.
use std::{net::SocketAddr, sync::Arc, time::Duration};

use coterm_core::model::{
    lifecycle::{InstanceInfo, InstanceStatus},
    operation::{
        OperationEffect, OperationReceipt, OperationScope, OperationSource, OperationStatus,
    },
    RuntimeError,
};
use coterm_core::{
    runtime::Runtime,
    runtime::{CallerContext, SubmitOperation},
};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::{
    io::BufReader,
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

use crate::transport::{read_frame, write_frame};

/// Default loopback endpoint used by the desktop MCP bridge.
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
    pub async fn bind(runtime: Arc<Runtime>) -> std::io::Result<Self> {
        let address = DEFAULT_BRIDGE_ADDRESS
            .parse()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        Self::bind_at(runtime, address).await
    }

    /// Bind a bridge at an explicit address. Tests use port `0` to avoid
    /// competing with another bridge process.
    pub async fn bind_at(runtime: Arc<Runtime>, address: SocketAddr) -> std::io::Result<Self> {
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

async fn serve_client(socket: TcpStream, runtime: Arc<Runtime>) -> anyhow::Result<()> {
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    let mut initialized = false;
    let mut ready = false;
    let (responses, mut completed) = tokio::sync::mpsc::channel::<Value>(16);
    let slots = Arc::new(tokio::sync::Semaphore::new(16));
    let mut calls = JoinSet::new();
    loop {
        // Keep the read future alive across responses: cancelling a partial
        // newline frame would otherwise discard its prefix and corrupt the wire.
        let incoming = read_frame(&mut reader);
        tokio::pin!(incoming);
        let frame = loop {
            tokio::select! {
                frame = &mut incoming => break frame?,
                Some(response) = completed.recv() => { write_frame(&mut writer, &response).await?; }
                _ = calls.join_next(), if !calls.is_empty() => {}
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
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "Coterm", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Start with coterm_instances for connection state and capabilities. Use coterm_terminal_command for SSH/Local, coterm_stream_command for Serial/TCP/Telnet, and coterm_read_output for lines, dialogues or human selections. Omit name only with one globally confirmed connected instance and no unknown states. Credentials stay in the desktop. Writes may return pending_approval; query the same operation with coterm_operation instead of resubmitting. Waiting expires without cancellation or replay. Choose exec only for expected long tasks or an explicit background request."
    }))
}

pub(crate) async fn ready_request(
    runtime: &Runtime,
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

/// Return the eight public semantic tool schemas; no device IO or credentials are required.
/// The Runtime parameter is retained for callers; this fixed catalog currently cannot fail.
pub async fn tools_list(_runtime: &Runtime) -> Result<Value, RuntimeError> {
    Ok(json!({"tools":crate::unified::tools()}))
}

fn public_tool_name(driver: &str, native: &str) -> String {
    let driver = driver.strip_prefix("coterm-driver-").unwrap_or(driver);
    let native = native.strip_prefix(&format!("{driver}.")).unwrap_or(native);
    let normalize = |value: &str| {
        value
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>()
    };
    format!("coterm_{}_{}", normalize(driver), normalize(native))
}

fn public_instance(instance: &InstanceInfo) -> Value {
    json!({
        "name": instance.label,
        "driverId": instance.driver_id,
        "status": instance.status,
    })
}

pub(crate) struct ConnectionSnapshot {
    pub(crate) instances: Vec<(InstanceInfo, Option<bool>)>,
    pub(crate) details: std::collections::HashMap<String, Value>,
    pub(crate) total: usize,
}

/// Connection knowledge is opt-in driver metadata, never guessed from a ready UI
/// instance or a protocol-specific operation name. Probes cannot commit or prompt.
pub(crate) async fn connection_snapshot(
    runtime: &Runtime,
    driver: Option<&str>,
) -> anyhow::Result<ConnectionSnapshot> {
    let all = runtime.list_instances(driver).await?;
    let total = all.len();
    let mut instances: Vec<_> = all
        .into_iter()
        .take(64)
        .map(|instance| {
            let connected = if instance.status == InstanceStatus::Ready {
                None
            } else {
                Some(false)
            };
            (instance, connected)
        })
        .collect();
    let tools = runtime.native_mcp_tools().await;
    let slots = Arc::new(tokio::sync::Semaphore::new(8));
    let mut probes = JoinSet::new();
    for (index, (instance, connected)) in instances.iter().enumerate() {
        if connected.is_some() {
            continue;
        }
        let declared: Vec<_> = tools
            .iter()
            .filter(|(driver, tool)| {
                driver == &instance.driver_id
                    && tool.descriptor["_meta"]["coterm/connectionStatus"] == true
            })
            .collect();
        let [(_, registration)] = declared.as_slice() else {
            continue;
        };
        let operation = runtime
            .operation_catalog(&instance.driver_id)
            .await
            .into_iter()
            .find(|op| op.name == registration.operation);
        let Some(operation) = operation.filter(|op| {
            op.effect == OperationEffect::Read
                && op.scope == OperationScope::Instance
                && !op.sensitive_input
                && op.transient
        }) else {
            continue;
        };
        let runtime = runtime.clone();
        let instance = instance.clone();
        let slots = slots.clone();
        probes.spawn(async move {
            let Ok(_permit) = slots.acquire_owned().await else {
                return (index, None, None);
            };
            let receipt = runtime
                .submit(
                    CallerContext {
                        source: OperationSource::Agent,
                        session_id: instance.session_id,
                        driver_id: Some(instance.driver_id.clone()),
                    },
                    SubmitOperation {
                        driver_id: instance.driver_id,
                        driver_instance_id: Some(instance.driver_instance_id.clone()),
                        operation: operation.name,
                        args: json!({"driverInstanceId":instance.driver_instance_id}),
                        wait_timeout_ms: Some(500),
                    },
                )
                .await;
            let detail = receipt
                .ok()
                .filter(|receipt| receipt.status == OperationStatus::Completed)
                .and_then(|receipt| receipt.output)
                .and_then(|mut output| output.as_object_mut()?.remove("structuredContent"));
            let connected = detail
                .as_ref()
                .and_then(|detail| detail["connected"].as_bool());
            (index, connected, detail)
        });
    }
    let mut details = std::collections::HashMap::new();
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        while let Some(result) = probes.join_next().await {
            if let Ok((index, connected, detail)) = result {
                instances[index].1 = connected;
                if let Some(detail) = detail {
                    details.insert(instances[index].0.driver_instance_id.clone(), detail);
                }
            }
        }
    })
    .await;
    // Dropping the JoinSet cancels only our read waiters, never user operations.
    Ok(ConnectionSnapshot {
        instances,
        details,
        total,
    })
}

pub(crate) async fn default_instance(
    runtime: &Runtime,
    driver: Option<&str>,
) -> anyhow::Result<InstanceInfo> {
    let snapshot = connection_snapshot(runtime, driver).await?;
    if snapshot.total != snapshot.instances.len()
        || snapshot
            .instances
            .iter()
            .any(|(_, connected)| connected.is_none())
    {
        anyhow::bail!("Connection state is unavailable; specify name from coterm_instances");
    }
    let mut connected = snapshot
        .instances
        .into_iter()
        .filter(|(_, connected)| *connected == Some(true))
        .map(|(instance, _)| instance);
    let Some(instance) = connected.next() else {
        anyhow::bail!("No connected default instance; specify name from coterm_instances and connect it first");
    };
    if connected.next().is_some() {
        anyhow::bail!("Multiple connected instances; specify name from coterm_instances");
    }
    Ok(instance)
}

fn hide_internal_instance_ids(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("driverInstanceId");
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

pub(crate) async fn resolve_instance_name(
    runtime: &Runtime,
    driver_id: Option<&str>,
    name: &str,
) -> anyhow::Result<InstanceInfo> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("Instance name must not be empty");
    }
    let matches: Vec<_> = runtime
        .list_instances(driver_id)
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
pub(crate) fn text_result(value: Value) -> Value {
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

/// Dispatch a tools/call parameter object to public tools or hidden compatibility names.
/// Returns a CallToolResult; malformed arguments and unavailable targets return an error.
/// Caller identity, approval, waiting and cancellation are enforced by the existing Runtime.
pub async fn call_tool(runtime: &Runtime, params: &Value) -> anyhow::Result<Value> {
    validate_wait(params)?;
    if crate::unified::is_tool(params["name"].as_str().unwrap_or_default()) {
        return crate::unified::call(runtime, params).await;
    }
    call_legacy_tool(runtime, params).await
}

fn validate_wait(params: &Value) -> anyhow::Result<Option<u64>> {
    params
        .pointer("/_meta/coterm~1waitTimeoutMs")
        .map(|value| {
            value
                .as_u64()
                .filter(|n| *n <= 300_000)
                .ok_or_else(|| anyhow::anyhow!("Invalid coterm/waitTimeoutMs (expected 0..300000)"))
        })
        .transpose()
}

pub(crate) async fn call_legacy_tool(runtime: &Runtime, params: &Value) -> anyhow::Result<Value> {
    let wait_timeout_ms = validate_wait(params)?;
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
    if name == "coterm.status" {
        if args.as_object().is_none_or(|args| !args.is_empty()) {
            anyhow::bail!("coterm.status takes no arguments");
        }
        let snapshot = connection_snapshot(runtime, None).await?;
        let active = snapshot
            .instances
            .iter()
            .filter(|(instance, _)| instance.status == InstanceStatus::Ready)
            .count();
        let connected = snapshot
            .instances
            .iter()
            .filter(|(_, connected)| *connected == Some(true))
            .count();
        let instances: Vec<_> = snapshot
            .instances
            .iter()
            .map(|(instance, connected)| {
                let mut value = public_instance(instance);
                value["connected"] = json!(connected);
                value
            })
            .collect();
        return Ok(text_result(
            json!({"desktopAlive":true,"bridgeEnabled":true,"totalInstances":snapshot.total,"activeInstances":active,"connectedInstances":connected,"instances":instances,"truncated":snapshot.total > snapshot.instances.len()}),
        ));
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
        if let Some(field) = args.as_object().and_then(|args| {
            args.keys()
                .find(|field| !matches!(field.as_str(), "name" | "driverId" | "operationId"))
        }) {
            anyhow::bail!("Unknown argument {field}; use name or driverId and operationId");
        }
        let instance_name = args
            .get("name")
            .map(|name| {
                name.as_str()
                    .ok_or_else(|| anyhow::anyhow!("name must be a string"))
            })
            .transpose()?;
        let driver_id = args
            .get("driverId")
            .map(|id| {
                id.as_str()
                    .ok_or_else(|| anyhow::anyhow!("driverId must be a string"))
            })
            .transpose()?;
        if instance_name.is_some() && driver_id.is_some() {
            anyhow::bail!("Specify at most one name or driverId");
        }
        let (authorized_driver, session_id, instance_id) =
            if let Some(instance_name) = instance_name {
                let instance = resolve_instance_name(runtime, None, instance_name).await?;
                (
                    instance.driver_id,
                    instance.session_id,
                    Some(instance.driver_instance_id),
                )
            } else if let Some(driver_id) = driver_id {
                (driver_id.to_owned(), String::new(), None)
            } else {
                let instance = default_instance(runtime, None).await?;
                (
                    instance.driver_id,
                    instance.session_id,
                    Some(instance.driver_instance_id),
                )
            };
        let id = args["operationId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("operationId required"))?;
        let caller = CallerContext {
            source: OperationSource::Agent,
            session_id,
            driver_id: Some(authorized_driver),
        };
        let record = runtime.get(caller.clone(), id).await?;
        if record.driver_instance_id.as_deref() != instance_id.as_deref() {
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
    let matching: Vec<_> = native
        .into_iter()
        .filter(|(driver, tool)| {
            tool.descriptor["name"].as_str().is_some_and(|native| {
                public_tool_name(driver, native) == name || format!("{driver}__{native}") == name
            })
        })
        .collect();
    let (driver, tool) = match matching.len() {
        0 => anyhow::bail!("Tool not found"),
        1 => matching
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Tool not found"))?,
        _ => anyhow::bail!("Tool name is ambiguous; conflicting driver registrations"),
    };
    let descriptor = runtime
        .operation_catalog(&driver)
        .await
        .into_iter()
        .find(|op| op.name == tool.operation)
        .ok_or_else(|| anyhow::anyhow!("Operation descriptor not found"))?;
    let instance_id = if descriptor.scope == coterm_core::model::operation::OperationScope::Instance
    {
        let target_fields = ["name", "instance", "instanceName", "instance_name"]
            .into_iter()
            .filter(|field| args.get(*field).is_some())
            .collect::<Vec<_>>();
        if target_fields.len() > 1 {
            anyhow::bail!(
                "Provide exactly one instance name using name, instance, instanceName, or instance_name"
            );
        }
        if args.get("driverInstanceId").is_some() {
            anyhow::bail!("driverInstanceId is internal; use name from coterm.list_instances");
        }
        let instance = if let Some(field) = target_fields.first() {
            let public_name = args[*field]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Instance name must be a string"))?;
            resolve_instance_name(runtime, Some(&driver), public_name).await?
        } else {
            default_instance(runtime, Some(&driver)).await?
        };
        let instance_id = instance.driver_instance_id.clone();
        let object = args
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Tool arguments must be an object"))?;
        object.remove("name");
        object.remove("instance");
        object.remove("instanceName");
        object.remove("instance_name");
        object.insert("driverInstanceId".to_owned(), json!(&instance_id));
        Some(instance_id)
    } else {
        None
    };
    let session_id = if let Some(instance_id) = instance_id.as_deref() {
        let instance = runtime
            .get_instance(instance_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Instance not found"))?;
        if instance.driver_id != driver {
            anyhow::bail!("Tool instance mismatch");
        }
        instance.session_id
    } else {
        String::new()
    };
    // Source and driver authorization are derived from the trusted tool directory.
    let caller = CallerContext {
        source: OperationSource::Agent,
        session_id,
        driver_id: Some(driver.clone()),
    };
    let return_on_approval = tool.descriptor["_meta"]["coterm/returnOnApproval"] == true;
    let request = SubmitOperation {
        driver_id: driver,
        driver_instance_id: instance_id,
        operation: tool.operation,
        args,
        wait_timeout_ms,
    };
    let receipt = if return_on_approval {
        runtime.submit_until_approval(caller, request).await?
    } else {
        runtime.submit(caller, request).await?
    };
    let public_status = if return_on_approval && receipt.status == OperationStatus::AwaitingApproval
    {
        json!("pending_approval")
    } else {
        serde_json::to_value(receipt.status)?
    };
    let mut result = if receipt.status == OperationStatus::Completed {
        let output = receipt
            .output
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Driver MCP result missing"))?;
        if !output["content"].is_array() {
            anyhow::bail!("Driver returned invalid MCP CallToolResult");
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

    #[test]
    fn flat_names_do_not_repeat_driver_namespaces() {
        assert_eq!(
            public_tool_name("coterm-driver-ssh", "ssh.command"),
            "coterm_ssh_command"
        );
        assert_eq!(
            public_tool_name("coterm-driver-serial", "serial.command_hex"),
            "coterm_serial_command_hex"
        );
        assert_ne!(
            public_tool_name("coterm-driver-ssh", "ssh.command"),
            public_tool_name("coterm-driver-local", "local.command")
        );
        // The directory and dispatcher detect collisions rather than routing the
        // normalized name to whichever driver happens to have been registered first.
        assert_eq!(
            public_tool_name("a-b", "read"),
            public_tool_name("a_b", "read")
        );
    }

    #[tokio::test]
    async fn bridge_rejects_non_loopback_bind_addresses() {
        let runtime = Arc::new(Runtime::new(coterm_core::model::lifecycle::RuntimeLimits {
            max_control_bytes: 1024,
        }));
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
            "driverInstanceId": "private",
            "output": {"driverInstanceId": "also-private", "text": "ok"},
        });
        hide_internal_instance_ids(&mut value);
        assert_eq!(value, json!({"output": {"text": "ok"}}));
    }

    #[test]
    fn public_catalog_has_eight_unique_tools_and_no_internal_routing_keys() {
        let tools = crate::unified::tools();
        assert_eq!(tools.len(), 8);
        let names: std::collections::HashSet<_> = tools
            .iter()
            .map(|tool| tool["name"].clone().to_string())
            .collect();
        assert_eq!(names.len(), 8);
        for tool in tools {
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
            assert_eq!(tool["inputSchema"]["properties"]["name"]["type"], "string");
            for key in [
                "driverInstanceId",
                "driverId",
                "source",
                "bypass",
                "password",
                "cursor",
            ] {
                assert!(tool["inputSchema"]["properties"].get(key).is_none());
            }
        }
    }
}
