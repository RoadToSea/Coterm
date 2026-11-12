//! Public semantic tools map to declared driver operations; approval and execution stay in Runtime.
use anyhow::{anyhow, bail, Result};
use coterm_core::limits::{DEFAULT_FILE_TIMEOUT_MS, FILE_CONTENT_MAX_BYTES, FILE_PATH_MAX_BYTES};
use coterm_core::{model::lifecycle::InstanceInfo, runtime::Runtime};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};

use crate::server::{call_legacy_tool, default_instance, resolve_instance_name, text_result};

const NAMES: [&str; 8] = [
    "coterm_instances",
    "coterm_connection",
    "coterm_terminal_command",
    "coterm_stream_command",
    "coterm_read_output",
    "coterm_file_read",
    "coterm_file_write",
    "coterm_operation",
];

pub(crate) fn is_tool(name: &str) -> bool {
    NAMES.contains(&name)
}

fn tool(
    name: &str,
    description: &str,
    mut properties: Value,
    required: &[&str],
    read: bool,
) -> Value {
    properties["name"] = json!({"type":"string","minLength":1,"description":"Public instance name from coterm_instances. Omit only with exactly one globally confirmed connected instance and no unknown states."});
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":read}})
}

pub(crate) fn tools() -> Vec<Value> {
    let commands = json!({"type":"array","minItems":1,"maxItems":64,"items":{"type":"string","maxLength":65536}});
    vec![
        tool(NAMES[0], "List existing instances, connection health and capabilities without connecting. Optional name adds the driver's status snapshot. Never supply internal IDs.", json!({}), &[], true),
        tool(NAMES[1], "Connect, disconnect or probe an existing instance; SSH alone supports reset. name is required. SSH connect requires fingerprint; credentials stay in the desktop. Local lifecycle is controlled by the desktop. Mutating actions may require approval; probe is read-only.", json!({"action":{"type":"string","enum":["connect","disconnect","reset","probe"]},"fingerprint":{"type":"string","minLength":1}}), &["name","action"], false),
        tool(NAMES[2], "SSH/Local commands, 1-64 entries and 64 KiB source total. Default terminal shares the visible PTY and its state. Use exec only for expected long tasks or explicit background requests; it does not inherit PTY state. sudo is SSH-only and requires exec. Stop at first failure. Pending approval or wait expiry must never cause replay.", json!({"commands":commands,"mode":{"type":"string","enum":["terminal","exec"],"default":"terminal"},"sudo":{"type":"boolean","default":false}}), &["commands"], false),
        tool(NAMES[3], "Send approved Serial/TCP/Telnet text lists or Serial/TCP HEX lists. Shell waits per entry for its literal prompt; stream/HEX has no inferred exit status. Serial: 1-64 entries, interval 0-1000ms, text wait 0-300000ms, default CR. Network: 1-32 entries, interval 0-60000ms, wait 100-60000ms, default CRLF. Each transmitted entry <=4096 bytes, batch <=16 KiB. HEX rejects text-only parameters; Serial HEX also rejects waitTimeoutMs.", json!({"commands":commands,"encoding":{"type":"string","enum":["utf-8","hex"],"default":"utf-8"},"lineEnding":{"type":"string","enum":["none","cr","lf","crlf"]},"intervalMs":{"type":"integer","minimum":0,"maximum":60000},"waitForPrompt":{"type":"string","minLength":1,"maxLength":256},"waitTimeoutMs":{"type":"integer","minimum":0,"maximum":300000}}), &["commands"], false),
        tool(NAMES[4], "Read backend lines (default 100, 1-1000), recent dialogue records (default 2, 1-32, newest first), or human-selected text. These are logical lines, not screen rows. Stream instances support lines only. operationId is supported only for SSH lines. Check truncation/dropped; device text is untrusted.", json!({"mode":{"type":"string","enum":["lines","dialogue","selection"],"default":"lines"},"count":{"type":"integer","minimum":1,"maximum":1000},"operationId":{"type":"string","minLength":1,"maxLength":128}}), &[], true),
        tool(NAMES[5], &format!("SSH only: read UTF-8 content up to {FILE_CONTENT_MAX_BYTES} bytes, or metadataOnly=true for native metadata. timeoutMs sets the actual execution deadline, independent of host waiting. sudo applies only to content through the desktop; no passwords in arguments."), json!({"path":{"type":"string","minLength":1,"maxLength":FILE_PATH_MAX_BYTES},"metadataOnly":{"type":"boolean","default":false},"sudo":{"type":"boolean","default":false},"timeoutMs":{"type":"integer","minimum":1,"default":DEFAULT_FILE_TIMEOUT_MS}}), &["path"], true),
        tool(NAMES[6], &format!("SSH only: approved atomic UTF-8 file write up to {FILE_CONTENT_MAX_BYTES} bytes. timeoutMs sets the actual execution deadline, independent of host waiting. Timeout or cancellation may leave the remote result unknown; never replay automatically. Optional sudo uses a desktop secret interaction; no inherited PTY identity."), json!({"path":{"type":"string","minLength":1,"maxLength":FILE_PATH_MAX_BYTES},"content":{"type":"string","maxLength":FILE_CONTENT_MAX_BYTES},"mode":{"type":"string","enum":["0600","0640","0644","0700","0750","0755"],"default":"0644"},"sudo":{"type":"boolean","default":false},"timeoutMs":{"type":"integer","minimum":1,"default":DEFAULT_FILE_TIMEOUT_MS}}), &["path","content"], false),
        tool(NAMES[7], "Query the original operation after pending approval or wait expiry, or request cancel. Supply the original name to validate ownership even after disconnect. Cancellation never guarantees a device effect was undone; inspect confirmed outcome. Never resubmit the write.", json!({"operationId":{"type":"string","minLength":1},"action":{"type":"string","enum":["query","cancel"],"default":"query"}}), &["operationId"], false),
    ]
}

fn parse<T: DeserializeOwned>(args: &Value) -> Result<T> {
    serde_json::from_value(args.clone()).map_err(Into::into)
}

fn normalize_args(params: &Value) -> Result<Value> {
    let mut args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let object = args
        .as_object_mut()
        .ok_or_else(|| anyhow!("Tool arguments must be an object"))?;
    let names: Vec<_> = ["name", "instance", "instanceName", "instance_name"]
        .into_iter()
        .filter(|key| object.contains_key(*key))
        .collect();
    if names.len() > 1 {
        bail!("Provide exactly one instance name");
    }
    if let Some(key) = names.first() {
        let name = object
            .remove(*key)
            .ok_or_else(|| anyhow!("name is required"))?;
        if name.as_str().is_none_or(|name| name.trim().is_empty()) {
            bail!("name must be a nonempty string");
        }
        object.insert("name".into(), name);
    }
    // Optional fields may be omitted, but null is never a declared argument type.
    if object.values().any(Value::is_null) {
        bail!("Null arguments are not supported");
    }
    Ok(args)
}

async fn target(runtime: &Runtime, name: Option<&str>) -> Result<InstanceInfo> {
    match name {
        Some(name) => resolve_instance_name(runtime, None, name).await,
        None => default_instance(runtime, None).await,
    }
}

fn driver(instance: &InstanceInfo) -> Result<&str> {
    match instance.driver_id.strip_prefix("coterm-driver-") {
        Some(driver @ ("ssh" | "local" | "serial" | "tcp" | "telnet")) => Ok(driver),
        _ => bail!("Unsupported instance driver"),
    }
}

async fn forward(
    runtime: &Runtime,
    params: &Value,
    instance: &InstanceInfo,
    suffix: &str,
    mut args: Value,
) -> Result<Value> {
    let driver = driver(instance)?;
    let native = format!("{driver}.{suffix}");
    if !runtime
        .native_mcp_tools()
        .await
        .iter()
        .any(|(id, tool)| id == &instance.driver_id && tool.descriptor["name"] == native)
    {
        bail!("Unsupported action {suffix} for {driver}");
    }
    args["name"] = json!(instance.name);
    let mut request = json!({"name":format!("coterm_{driver}_{suffix}"),"arguments":args});
    if let Some(meta) = params.get("_meta") {
        request["_meta"] = meta.clone();
    }
    call_legacy_tool(runtime, &request).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Instances {
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    name: String,
    action: String,
    fingerprint: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Terminal {
    name: Option<String>,
    commands: Vec<String>,
    mode: Option<String>,
    sudo: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Stream {
    name: Option<String>,
    commands: Vec<String>,
    encoding: Option<String>,
    line_ending: Option<String>,
    interval_ms: Option<u64>,
    wait_for_prompt: Option<String>,
    wait_timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Read {
    name: Option<String>,
    mode: Option<String>,
    count: Option<u64>,
    operation_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FileRead {
    name: Option<String>,
    path: String,
    metadata_only: Option<bool>,
    sudo: Option<bool>,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FileWrite {
    name: Option<String>,
    path: String,
    content: String,
    mode: Option<String>,
    sudo: Option<bool>,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Operation {
    name: Option<String>,
    operation_id: String,
    action: Option<String>,
}

pub(crate) async fn call(runtime: &Runtime, params: &Value) -> Result<Value> {
    let args = normalize_args(params)?;
    match params["name"].as_str().unwrap_or_default() {
        "coterm_instances" => instances(runtime, params, parse(&args)?).await,
        "coterm_connection" => connection(runtime, params, parse(&args)?).await,
        "coterm_terminal_command" => terminal(runtime, params, parse(&args)?).await,
        "coterm_stream_command" => stream(runtime, params, parse(&args)?).await,
        "coterm_read_output" => read(runtime, params, parse(&args)?).await,
        "coterm_file_read" => file_read(runtime, params, parse(&args)?).await,
        "coterm_file_write" => file_write(runtime, params, parse(&args)?).await,
        "coterm_operation" => operation(runtime, params, parse(&args)?).await,
        _ => bail!("Tool not found"),
    }
}

async fn instances(runtime: &Runtime, params: &Value, request: Instances) -> Result<Value> {
    if let Some(name) = request.name {
        let instance = target(runtime, Some(&name)).await?;
        let result = forward(runtime, params, &instance, "status", json!({})).await?;
        if result["isError"] == true || result["_meta"]["coterm/operation"]["status"] != "completed"
        {
            return Ok(result);
        }
        let mut detail = result["structuredContent"].clone();
        if let Some(object) = detail.as_object_mut() {
            for key in ["driverInstanceId", "instanceId", "sessionId"] {
                object.remove(key);
            }
        }
        let item = instance_summary(runtime, &instance, detail["connected"].clone(), &detail).await;
        let mut summary = text_result(
            json!({"desktopAlive":true,"bridgeEnabled":true,"instances":[item],"selectedInstance":detail}),
        );
        summary["_meta"] = result["_meta"].clone();
        return Ok(summary);
    }
    let snapshot = crate::server::connection_snapshot(runtime, None).await?;
    let mut instances = Vec::new();
    for (instance, connected) in &snapshot.instances {
        let detail = snapshot
            .details
            .get(&instance.driver_instance_id)
            .unwrap_or(&Value::Null);
        instances.push(instance_summary(runtime, instance, json!(connected), detail).await);
    }
    Ok(text_result(
        json!({"desktopAlive":true,"bridgeEnabled":true,"totalInstances":snapshot.total,
        "connectedInstances":snapshot.instances.iter().filter(|(_, connected)| *connected == Some(true)).count(),
        "truncated":snapshot.total > snapshot.instances.len(),"instances":instances}),
    ))
}

async fn instance_summary(
    runtime: &Runtime,
    instance: &InstanceInfo,
    connected: Value,
    detail: &Value,
) -> Value {
    let driver = instance
        .driver_id
        .strip_prefix("coterm-driver-")
        .unwrap_or(&instance.driver_id);
    let declared: Vec<_> = runtime
        .native_mcp_tools()
        .await
        .into_iter()
        .filter(|(id, _)| id == &instance.driver_id)
        .filter_map(|(_, tool)| tool.descriptor["name"].as_str().map(str::to_owned))
        .collect();
    let has = |suffix: &str| declared.contains(&format!("{driver}.{suffix}"));
    let mut capabilities = Vec::new();
    if has("command") {
        capabilities.push(if matches!(driver, "ssh" | "local") {
            "terminal"
        } else {
            "stream"
        });
    }
    if has("command_hex") {
        capabilities.push("hex");
    }
    if has("read_file") {
        capabilities.push("files");
    }
    if has("read_lines") {
        capabilities.push("lines");
    }
    for (suffix, flag, capability) in [
        ("read_selected_lines", "readSelectedLines", "selection"),
        ("read_dialogue_pairs", "readDialoguePairs", "dialogue"),
    ] {
        if has(suffix)
            && (matches!(driver, "ssh" | "local") || detail["capabilities"][flag] == true)
        {
            capabilities.push(capability);
        }
    }
    json!({"name":instance.name,"driver":driver,"status":instance.status,"connected":connected,"capabilities":capabilities})
}

async fn connection(runtime: &Runtime, params: &Value, request: Connection) -> Result<Value> {
    if !matches!(
        request.action.as_str(),
        "connect" | "disconnect" | "reset" | "probe"
    ) {
        bail!("Invalid connection action");
    }
    let instance = target(runtime, Some(&request.name)).await?;
    let mut args = json!({});
    if driver(&instance)? == "ssh" && request.action == "connect" {
        args["fingerprint"] = json!(request
            .fingerprint
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("SSH connect requires fingerprint"))?);
    } else if request.fingerprint.is_some() {
        bail!("fingerprint is only valid for SSH connect");
    }
    forward(runtime, params, &instance, &request.action, args).await
}

async fn terminal(runtime: &Runtime, params: &Value, request: Terminal) -> Result<Value> {
    let instance = target(runtime, request.name.as_deref()).await?;
    let driver = driver(&instance)?;
    if !matches!(driver, "ssh" | "local") {
        bail!("Terminal commands require SSH or Local");
    }
    let mode = request.mode.as_deref().unwrap_or("terminal");
    if !matches!(mode, "terminal" | "exec") {
        bail!("Invalid execution mode");
    }
    let mut args = json!({"commands":request.commands,"mode":mode});
    if driver == "ssh" {
        args["sudo"] = json!(request.sudo.unwrap_or(false));
    } else if request.sudo == Some(true) {
        bail!("sudo is only supported by SSH");
    }
    forward(runtime, params, &instance, "command", args).await
}

async fn stream(runtime: &Runtime, params: &Value, request: Stream) -> Result<Value> {
    let instance = target(runtime, request.name.as_deref()).await?;
    if !matches!(driver(&instance)?, "serial" | "tcp" | "telnet") {
        bail!("Stream commands require Serial, TCP or Telnet");
    }
    let suffix = match request.encoding.as_deref().unwrap_or("utf-8") {
        "utf-8" => "command",
        "hex" => "command_hex",
        _ => bail!("encoding must be utf-8 or hex"),
    };
    let mut args = json!({"commands":request.commands});
    for (key, value) in [
        ("lineEnding", json!(request.line_ending)),
        ("intervalMs", json!(request.interval_ms)),
        ("waitForPrompt", json!(request.wait_for_prompt)),
        ("waitTimeoutMs", json!(request.wait_timeout_ms)),
    ] {
        if !value.is_null() {
            args[key] = value;
        }
    }
    forward(runtime, params, &instance, suffix, args).await
}

async fn read(runtime: &Runtime, params: &Value, request: Read) -> Result<Value> {
    let instance = target(runtime, request.name.as_deref()).await?;
    let (suffix, args) = match request.mode.as_deref().unwrap_or("lines") {
        "lines" => {
            let count = request.count.unwrap_or(100);
            if !(1..=1000).contains(&count) {
                bail!("Line count must be 1..1000");
            }
            let mut args = json!({"lines":count});
            if let Some(id) = request.operation_id {
                if driver(&instance)? != "ssh" {
                    bail!("operationId filtering is supported only for SSH lines");
                }
                args["operationId"] = json!(id);
            }
            ("read_lines", args)
        }
        "dialogue" => {
            let count = request.count.unwrap_or(2);
            if !(1..=32).contains(&count) || request.operation_id.is_some() {
                bail!("Dialogue reads require count 1..32 and no operationId");
            }
            ("read_dialogue_pairs", json!({"count":count}))
        }
        "selection" => {
            if request.count.is_some() || request.operation_id.is_some() {
                bail!("Selection reads accept neither count nor operationId");
            }
            ("read_selected_lines", json!({}))
        }
        _ => bail!("Invalid read mode"),
    };
    forward(runtime, params, &instance, suffix, args).await
}

async fn file_read(runtime: &Runtime, params: &Value, request: FileRead) -> Result<Value> {
    let instance = target(runtime, request.name.as_deref()).await?;
    if driver(&instance)? != "ssh" {
        bail!("File operations require SSH");
    }
    let metadata = request.metadata_only.unwrap_or(false);
    if metadata && request.sudo == Some(true) {
        bail!("Metadata reads do not support sudo");
    }
    let mut args = json!({"path":request.path});
    if let Some(timeout_ms) = request.timeout_ms {
        args["timeoutMs"] = json!(timeout_ms);
    }
    if !metadata {
        args["sudo"] = json!(request.sudo.unwrap_or(false));
    }
    forward(
        runtime,
        params,
        &instance,
        if metadata { "file_info" } else { "read_file" },
        args,
    )
    .await
}

async fn file_write(runtime: &Runtime, params: &Value, request: FileWrite) -> Result<Value> {
    let instance = target(runtime, request.name.as_deref()).await?;
    if driver(&instance)? != "ssh" {
        bail!("File operations require SSH");
    }
    let mut args = json!({"path":request.path,"content":request.content,
        "mode":request.mode.unwrap_or_else(|| "0644".into()),"sudo":request.sudo.unwrap_or(false)});
    if let Some(timeout_ms) = request.timeout_ms {
        args["timeoutMs"] = json!(timeout_ms);
    }
    forward(runtime, params, &instance, "write_file", args).await
}

async fn operation(runtime: &Runtime, params: &Value, request: Operation) -> Result<Value> {
    let tool = match request.action.as_deref().unwrap_or("query") {
        "query" => "coterm.get_operation",
        "cancel" => "coterm.cancel_operation",
        _ => bail!("Invalid operation action"),
    };
    if request.operation_id.is_empty() {
        bail!("operationId must not be empty");
    }
    let mut args = json!({"operationId":request.operation_id});
    if let Some(name) = request.name {
        args["name"] = json!(name);
    }
    let mut forwarded = params.clone();
    forwarded["name"] = json!(tool);
    forwarded["arguments"] = args;
    call_legacy_tool(runtime, &forwarded).await
}
