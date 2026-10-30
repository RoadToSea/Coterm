//! Generic serial text and HEX I/O; device-specific protocols belong to other plugins.
use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use coterm_proto::runtime::{
    lifecycle::{CreateInstance, HelloResult, McpToolRegistration, RuntimeHello},
    message::{CancelTarget, DeferredOperation, HostCommand, PluginNotification},
    operation::{
        ApproveProposal, DispatchMode, InvokeRequest, OperationDescriptor, OperationEffect,
        OperationScope,
    },
    ErrorCategory, RuntimeError,
};
use serde_json::{json, Value};

use crate::command::{self, CommandList};
use crate::payload::{hex_payload, text_payload};
use crate::session::{same_port, Config, Connection};

pub const PLUGIN_ID: &str = "coterm-plugin-serial";

struct Instance {
    config: Config,
    connection: Option<Connection>,
}

pub struct Backend {
    events: tokio::sync::mpsc::Sender<PluginNotification>,
    notifications: Option<tokio::sync::mpsc::Receiver<PluginNotification>>,
    instances: HashMap<String, Instance>,
    proposals: HashMap<String, Proposal>,
}

struct Proposal {
    instance_id: String,
    operation_id: String,
    operation: String,
}

impl Default for Backend {
    fn default() -> Self {
        let (events, notifications) = tokio::sync::mpsc::channel(128);
        Self {
            events,
            notifications: Some(notifications),
            instances: HashMap::new(),
            proposals: HashMap::new(),
        }
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| anyhow!("Invalid request parameters"))
}

pub fn catalog() -> Vec<OperationDescriptor> {
    let definitions = [
        ("serial.status", OperationEffect::Read, false, json!({})),
        ("serial.probe", OperationEffect::Read, false, json!({})),
        ("serial.connect", OperationEffect::Commit, false, json!({})),
        (
            "serial.disconnect",
            OperationEffect::Commit,
            false,
            json!({}),
        ),
        (
            "serial.read",
            OperationEffect::Read,
            false,
            json!({"cursor":{"type":"string","default":"0"}}),
        ),
        // 人工输入可能是登录密码或密钥：账本只保留脱敏占位。
        (
            "serial.write",
            OperationEffect::Commit,
            true,
            json!({"text":{"type":"string","maxLength":16384},"lineEnding":{"type":"string","enum":["none","lf","cr","crlf"],"default":"none"}}),
        ),
        (
            "serial.read_hex",
            OperationEffect::Read,
            false,
            json!({"cursor":{"type":"string","default":"0"}}),
        ),
        (
            "serial.write_hex",
            OperationEffect::Commit,
            true,
            json!({"hex":{"type":"string","minLength":2,"maxLength":49154}}),
        ),
        (
            "serial.command",
            OperationEffect::Commit,
            true,
            command::properties(),
        ),
        (
            "serial.command_hex",
            OperationEffect::Commit,
            true,
            command::hex_properties(),
        ),
        (
            "serial.signals",
            OperationEffect::Commit,
            false,
            json!({"dtr":{"type":"boolean"},"rts":{"type":"boolean"}}),
        ),
    ];
    let mut operations: Vec<_> = definitions
        .into_iter()
        .map(
            |(name, effect, sensitive_input, properties)| OperationDescriptor {
                name: name.to_owned(),
                scope: OperationScope::Instance,
                effect,
                input_schema: json!({"type":"object","properties":properties,"additionalProperties":false}),
                output_schema: None,
                sensitive_input,
                transient: matches!(
                    name,
                    "serial.read" | "serial.read_hex" | "serial.status" | "serial.probe" | "serial.write" | "serial.write_hex"
                ),
            },
        )
        .collect();
    operations.push(OperationDescriptor {
        name: "serial.plugin_info".to_owned(),
        scope: OperationScope::Plugin,
        effect: OperationEffect::Read,
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
        output_schema: None,
        sensitive_input: false,
        transient: false,
    });
    for operation in &mut operations {
        match operation.name.as_str() {
            "serial.command" | "serial.command_hex" => {
                operation.input_schema["required"] = json!(["commands"])
            }
            "serial.write" => operation.input_schema["required"] = json!(["text"]),
            "serial.write_hex" => operation.input_schema["required"] = json!(["hex"]),
            _ => {}
        }
    }
    for registration in mcp_tools() {
        let native = registration.operation.trim_start_matches("mcp.");
        if let Some(base) = operations.iter().find(|item| item.name == native).cloned() {
            operations.push(OperationDescriptor {
                name: registration.operation,
                // MCP 参数也会进入 Agent 账本；镜像沿用领域操作本身的敏感标记。
                sensitive_input: base.sensitive_input,
                ..base
            });
        }
    }
    operations
}

/// Text and HEX are separate MCP tools; both use bounded, approved FIFO lists.
pub fn mcp_tools() -> Vec<McpToolRegistration> {
    let mut tools: Vec<_> = [
        ("serial.status", "Read the serial port configuration and connection state.", json!({}), vec![]),
        ("serial.probe", "List serial ports visible to the operating system together with this instance's configuration.", json!({}), vec![]),
        ("serial.connect", "Open the configured serial port.", json!({}), vec![]),
        ("serial.disconnect", "Close the serial port and release it.", json!({}), vec![]),
        ("serial.read", "Read bounded text from the device using a decimal cursor; reports gaps. Non-text bytes are represented with replacement characters.", json!({"cursor":{"type":"string","default":"0"}}), vec![]),
        ("serial.read_hex", "Read bounded serial bytes as a HEX string using a decimal cursor. Explicit HEX mode only; no Base64 or device protocol parsing.", json!({"cursor":{"type":"string","default":"0"}}), vec![]),
        ("serial.command", "Queue 1-64 text inputs in order, exactly like human Shell input. Preserves all control characters, newlines and escape sequences. lineEnding defaults to none; include CR in the text or select an ending explicitly. intervalMs is a sending delay, not a device completion check. Requires approval unless Bypass or allowlist permits. Never replays after uncertain delivery.", command::properties(), vec!["commands"]),
        ("serial.command_hex", "Queue 1-64 HEX byte strings in order (for example 00 80 FF). Explicit HEX mode only. No Base64, automatic line endings, Modbus interpretation or firmware flashing. Requires approval unless Bypass or allowlist permits; device completion is unknown.", command::hex_properties(), vec!["commands"]),
    ]
    .into_iter()
    .map(|(name, description, mut properties, required)| {
        properties["pluginInstanceId"] = json!({"type":"string","minLength":1});
        let mut required = required;
        required.push("pluginInstanceId");
        McpToolRegistration {
            operation: format!("mcp.{name}"),
            descriptor: json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}),
        }
    })
    .collect();
    tools.push(McpToolRegistration {
        operation: "mcp.serial.plugin_info".to_owned(),
        descriptor: json!({"name":"serial.plugin_info","description":"Read plugin-level serial information without selecting an instance.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}),
    });
    tools
}

fn validate_mcp_args(operation: &str, args: &Value) -> Result<()> {
    let tool = mcp_tools()
        .into_iter()
        .find(|tool| tool.operation == operation)
        .ok_or_else(|| anyhow!("Unknown serial MCP tool"))?;
    let object = args
        .as_object()
        .ok_or_else(|| anyhow!("MCP arguments must be an object"))?;
    let schema = &tool.descriptor["inputSchema"];
    let properties = schema["properties"]
        .as_object()
        .ok_or_else(|| anyhow!("Invalid MCP schema"))?;
    if object.iter().any(|(name, value)| {
        let Some(property) = properties.get(name) else {
            return true;
        };
        match property["type"].as_str() {
            Some("string") => !value.is_string(),
            Some("array") => !value.is_array(),
            Some("integer") => !value.is_u64(),
            _ => true,
        }
    }) {
        bail!("Unknown or invalid serial MCP argument");
    }
    if schema["required"].as_array().is_some_and(|required| {
        required
            .iter()
            .filter_map(Value::as_str)
            .any(|name| !object.contains_key(name))
    }) {
        bail!("Missing required serial MCP argument");
    }
    if object
        .get("pluginInstanceId")
        .and_then(Value::as_str)
        .is_some_and(|id| id.trim().is_empty())
    {
        bail!("An explicit serial instance is required");
    }
    if matches!(operation, "mcp.serial.command" | "mcp.serial.command_hex") {
        parse_commands(operation, &command_args(args))?;
    }
    Ok(())
}

fn command_args(args: &Value) -> Value {
    let mut args = args.clone();
    if let Some(object) = args.as_object_mut() {
        object.remove("pluginInstanceId");
    }
    args
}

fn mcp_result(value: Value) -> Value {
    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
}

fn parse_commands(operation: &str, args: &Value) -> Result<CommandList> {
    if operation.ends_with("serial.command_hex") {
        CommandList::parse_hex(args)
    } else {
        CommandList::parse(args)
    }
}

impl coterm_proto::runtime::backend::PluginBackend for Backend {
    async fn request(&mut self, command: HostCommand) -> Result<Value, RuntimeError> {
        // 传输层负责 JSON-RPC 方法名，领域分派保持基座契约的形状。
        let (method, params) = coterm_proto::runtime::wire::command_params(command)?;
        self.request(method, params).await.map_err(|error| {
            error
                .downcast_ref::<RuntimeError>()
                .cloned()
                .unwrap_or_else(|| RuntimeError {
                    category: ErrorCategory::Domain,
                    code: "serial_domain_error".to_owned(),
                    message: error.to_string(),
                    details: None,
                })
        })
    }

    fn take_notifications(&mut self) -> Option<tokio::sync::mpsc::Receiver<PluginNotification>> {
        self.notifications.take()
    }
}

impl Backend {
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        match method {
            "initialize" => {
                let hello: RuntimeHello = decode(params)?;
                let enabled = [
                    "instances",
                    "backend-calls",
                    "operation-catalog",
                    "native-mcp-tools",
                    "async-operations",
                    "plugin-events",
                ];
                let enabled_features = hello.negotiate_features(&enabled)?;
                let mcp_tools = if enabled_features.iter().any(|f| f == "native-mcp-tools") {
                    mcp_tools()
                } else {
                    Vec::new()
                };
                Ok(serde_json::to_value(HelloResult {
                    plugin_id: PLUGIN_ID.to_owned(),
                    plugin_version: env!("CARGO_PKG_VERSION").to_owned(),
                    interface_minor: 0,
                    enabled_features,
                    limits: hello.control_limits(),
                    operations: catalog(),
                    mcp_tools,
                })?)
            }
            "create_instance" => {
                let create: CreateInstance = decode(params)?;
                if create.instance.plugin_id != PLUGIN_ID
                    || self
                        .instances
                        .contains_key(&create.instance.plugin_instance_id)
                    || self.instances.len() >= 32
                {
                    bail!("Invalid or duplicate serial instance (limit: 32)");
                }
                let config: Config = decode(create.config)?;
                config.validate()?;
                // 同一物理端口不能被两个实例同时打开。
                if self
                    .instances
                    .values()
                    .any(|instance| same_port(instance.config.port.trim(), config.port.trim()))
                {
                    bail!("This serial port is already used by another instance");
                }
                let label = config.label();
                self.instances.insert(
                    create.instance.plugin_instance_id,
                    Instance {
                        config,
                        connection: None,
                    },
                );
                Ok(json!({"ok":true,"label":label}))
            }
            "destroy_instance" => {
                let id = string(&params, "pluginInstanceId")?;
                let Some(mut instance) = self.instances.remove(id) else {
                    bail!("Serial instance not found");
                };
                if let Some(connection) = &mut instance.connection {
                    let _ = connection.disconnect().await;
                }
                self.proposals
                    .retain(|_, proposal| proposal.instance_id != id);
                Ok(json!({"ok":true}))
            }
            "invoke" => self.invoke(decode(params)?).await,
            "cancel" => {
                let target: CancelTarget = decode(params)?;
                if let CancelTarget::Operation(operation_id) = target {
                    for instance in self.instances.values() {
                        if let Some(connection) = &instance.connection {
                            let result = connection.cancel_commands(&operation_id).await?;
                            if result["outcome"] != "unsupported" {
                                return Ok(result);
                            }
                        }
                    }
                }
                Ok(json!({"outcome":"unsupported"}))
            }
            "approve" => {
                let approve: ApproveProposal = decode(params)?;
                let proposal = self
                    .proposals
                    .remove(&approve.proposal_id)
                    .ok_or_else(|| anyhow!("Serial proposal not found"))?;
                if Some(&proposal.instance_id) != approve.plugin_instance_id.as_ref()
                    || proposal.operation_id != approve.operation_id
                {
                    bail!("Serial proposal target mismatch");
                }
                self.execute(
                    &proposal.instance_id,
                    &proposal.operation,
                    approve.args,
                    &proposal.operation_id,
                )
                .await
            }
            "reject" => {
                let id = string(&params, "proposalId")?;
                let proposal = self
                    .proposals
                    .get(id)
                    .ok_or_else(|| anyhow!("Serial proposal not found"))?;
                if proposal.operation_id != string(&params, "operationId")? {
                    bail!("Serial proposal mismatch");
                }
                self.proposals.remove(id);
                Ok(json!({"ok":true}))
            }
            "shutdown" => {
                for instance in self.instances.values_mut() {
                    if let Some(connection) = &mut instance.connection {
                        let _ = connection.disconnect().await;
                    }
                }
                self.instances.clear();
                self.proposals.clear();
                Ok(json!({"ok":true}))
            }
            _ => bail!("Unsupported serial plugin method"),
        }
    }

    async fn invoke(&mut self, request: InvokeRequest) -> Result<Value> {
        if matches!(
            request.operation.as_str(),
            "serial.plugin_info" | "mcp.serial.plugin_info"
        ) {
            if request.plugin_instance_id.is_some() || request.effect != OperationEffect::Read {
                bail!("Serial plugin operation context mismatch");
            }
            let info = json!({"pluginId":PLUGIN_ID,"instanceCount":self.instances.len()});
            return Ok(if request.operation.starts_with("mcp.") {
                mcp_result(info)
            } else {
                info
            });
        }
        let id = request
            .plugin_instance_id
            .ok_or_else(|| anyhow!("An explicit serial instance is required"))?;
        let descriptor = catalog()
            .into_iter()
            .find(|operation| operation.name == request.operation)
            .ok_or_else(|| anyhow!("Unknown serial operation"))?;
        if descriptor.effect != request.effect || !self.instances.contains_key(&id) {
            bail!("Serial operation context mismatch");
        }
        if request.operation.starts_with("mcp.") {
            validate_mcp_args(&request.operation, &request.args)?;
            if request.args.get("pluginInstanceId").and_then(Value::as_str) != Some(&id) {
                bail!("MCP instance argument mismatch");
            }
        }
        if let DispatchMode::PrepareProposal { proposal_id } = request.dispatch {
            if self.proposals.len() >= 64 {
                bail!("Too many pending serial proposals");
            }
            // 提案不触碰硬件；只校验参数形状。
            match request.operation.as_str() {
                "serial.write" => {
                    text_payload(&request.args)?;
                }
                "serial.write_hex" => {
                    hex_payload(&request.args)?;
                }
                "serial.command"
                | "mcp.serial.command"
                | "serial.command_hex"
                | "mcp.serial.command_hex" => {
                    let args = if request.operation.starts_with("mcp.") {
                        command_args(&request.args)
                    } else {
                        request.args.clone()
                    };
                    parse_commands(&request.operation, &args)?;
                }
                "serial.signals" => {
                    let has_dtr = optional_bool(&request.args, "dtr")?.is_some();
                    let has_rts = optional_bool(&request.args, "rts")?.is_some();
                    if !has_dtr && !has_rts {
                        bail!("Provide dtr, rts, or both");
                    }
                }
                _ => {}
            }
            self.proposals.insert(
                proposal_id.clone(),
                Proposal {
                    instance_id: id,
                    operation_id: request.operation_id,
                    operation: request.operation.clone(),
                },
            );
            return Ok(
                json!({"proposalId":proposal_id,"operation":request.operation,"args":request.args}),
            );
        }
        self.execute(&id, &request.operation, request.args, &request.operation_id)
            .await
    }

    async fn execute(
        &mut self,
        id: &str,
        operation: &str,
        args: Value,
        operation_id: &str,
    ) -> Result<Value> {
        if let Some(native) = operation.strip_prefix("mcp.") {
            validate_mcp_args(operation, &args)?;
            if args.get("pluginInstanceId").and_then(Value::as_str) != Some(id) {
                bail!("MCP instance argument mismatch");
            }
            if matches!(native, "serial.command" | "serial.command_hex") {
                return self.queue_commands(id, native, command_args(&args), operation_id, true);
            }
            let result = self.execute_domain(id, native, args, operation_id).await?;
            return Ok(mcp_result(result));
        }
        if matches!(operation, "serial.command" | "serial.command_hex") {
            return self.queue_commands(id, operation, args, operation_id, false);
        }
        self.execute_domain(id, operation, args, operation_id).await
    }

    fn queue_commands(
        &mut self,
        id: &str,
        operation: &str,
        args: Value,
        operation_id: &str,
        mcp: bool,
    ) -> Result<Value> {
        let list = parse_commands(operation, &args)?;
        let connection = self
            .instances
            .get_mut(id)
            .and_then(|instance| instance.connection.as_mut())
            .ok_or_else(|| anyhow!("Serial port is not open"))?;
        connection.queue_commands(operation_id, mcp, list)?;
        Ok(serde_json::to_value(DeferredOperation {
            operation_id: operation_id.into(),
        })?)
    }

    async fn execute_domain(
        &mut self,
        id: &str,
        operation: &str,
        args: Value,
        _operation_id: &str,
    ) -> Result<Value> {
        if matches!(operation, "serial.connect" | "serial.disconnect") {
            // 物理连接换了，旧提案不再有意义。
            self.proposals
                .retain(|_, proposal| proposal.instance_id != id);
        }
        let instance = self
            .instances
            .get_mut(id)
            .ok_or_else(|| anyhow!("Serial instance not found"))?;
        match operation {
            "serial.status" => Ok(status_of(instance)),
            "serial.probe" => {
                let mut status = status_of(instance);
                status["ports"] = json!(available_ports());
                Ok(status)
            }
            "serial.connect" => {
                if instance
                    .connection
                    .as_ref()
                    .is_some_and(|connection| !connection.is_closed())
                {
                    bail!("Serial instance is already connected");
                }
                instance.connection = Some(
                    Connection::connect_with_events(
                        &instance.config,
                        Some((id.to_owned(), self.events.clone())),
                    )
                    .await?,
                );
                let connection_id = instance
                    .connection
                    .as_ref()
                    .map(|connection| connection.id.clone());
                let _ = self
                    .events
                    .send(PluginNotification::Event {
                        plugin_instance_id: id.to_owned(),
                        name: "serial.status".into(),
                        payload: json!({"connected":true,"connectionId":connection_id}),
                    })
                    .await;
                Ok(json!({"connected":true}))
            }
            "serial.disconnect" => {
                if let Some(connection) = &mut instance.connection {
                    let _ = connection.disconnect().await;
                }
                instance.connection = None;
                Ok(json!({"connected":false}))
            }
            "serial.read" | "serial.read_hex" => {
                let cursor = args
                    .get("cursor")
                    .and_then(Value::as_str)
                    .unwrap_or("0")
                    .parse::<u64>()
                    .map_err(|_| anyhow!("Invalid serial output cursor"))?;
                let connection = instance
                    .connection
                    .as_ref()
                    .ok_or_else(|| anyhow!("Serial port is not open"))?;
                let output = connection.output.lock().await;
                let mut result = if operation == "serial.read_hex" {
                    output.read_hex(cursor)?
                } else {
                    output.read(cursor)?
                };
                result["connectionId"] = json!(connection.id);
                Ok(result)
            }
            "serial.write" => {
                let data = text_payload(&args)?;
                let submitted = data.len();
                connection_of(instance)?.write(data).await?;
                Ok(json!({"submitted":true,"bytes":submitted}))
            }
            "serial.write_hex" => {
                let data = hex_payload(&args)?;
                let submitted = data.len();
                connection_of(instance)?.write(data).await?;
                Ok(json!({"submitted":true,"bytes":submitted}))
            }
            "serial.signals" => {
                let dtr = optional_bool(&args, "dtr")?;
                let rts = optional_bool(&args, "rts")?;
                if dtr.is_none() && rts.is_none() {
                    bail!("Provide dtr, rts, or both");
                }
                let connection = connection_of(instance)?;
                let (dtr, rts) = connection.set_signals(dtr, rts).await?;
                Ok(json!({"dtr":dtr,"rts":rts}))
            }
            _ => bail!("Unknown serial operation"),
        }
    }
}

/// 连接状态与收发计数；断开后仍回报配置和上一次的字节数。
fn status_of(instance: &Instance) -> Value {
    let connection = instance
        .connection
        .as_ref()
        .filter(|connection| !connection.is_closed());
    let (dtr, rts) = connection.map_or((instance.config.dtr, instance.config.rts), |connection| {
        connection.signals()
    });
    let (rx, tx) = connection.map_or((0, 0), |connection| connection.counters());
    json!({
        "config": instance.config,
        "connected": connection.is_some(),
        "connectionId": connection.map(|connection| &connection.id),
        "signals": {"dtr": dtr, "rts": rts},
        "rxBytes": rx,
        "txBytes": tx,
        "commandQueue": connection.map_or(0, Connection::pending_commands),
    })
}

fn available_ports() -> Vec<Value> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|port| {
            let kind = match &port.port_type {
                serialport::SerialPortType::UsbPort(usb) => json!({
                    "kind":"usb",
                    "vid":format!("{:04X}", usb.vid),
                    "pid":format!("{:04X}", usb.pid),
                    "manufacturer":usb.manufacturer,
                    "product":usb.product,
                    "serialNumber":usb.serial_number,
                }),
                serialport::SerialPortType::PciPort => json!({"kind":"pci"}),
                serialport::SerialPortType::BluetoothPort => json!({"kind":"bluetooth"}),
                serialport::SerialPortType::Unknown => json!({"kind":"unknown"}),
            };
            json!({"portName":port.port_name,"portType":kind})
        })
        .collect()
}

fn connection_of(instance: &Instance) -> Result<&Connection> {
    instance
        .connection
        .as_ref()
        .filter(|connection| !connection.is_closed())
        .ok_or_else(|| anyhow!("Serial port is not open"))
}

fn string<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing or invalid {field}"))
}

/// 未提供的布尔字段返回 `None`；类型不对则报错，不静默当成 false。
fn optional_bool(args: &Value, field: &str) -> Result<Option<bool>> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => bail!("Invalid {field}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(data: &[u8], ending: Option<&str>) -> Result<Vec<u8>> {
        let mut args = json!({"text": std::str::from_utf8(data)?});
        if let Some(ending) = ending {
            args["lineEnding"] = json!(ending);
        }
        text_payload(&args)
    }

    #[test]
    fn write_payload_appends_the_selected_line_ending() {
        assert_eq!(payload(b"ls", None).unwrap(), b"ls");
        assert_eq!(payload(b"ls", Some("none")).unwrap(), b"ls");
        assert_eq!(payload(b"ls", Some("lf")).unwrap(), b"ls\n");
        assert_eq!(payload(b"ls", Some("cr")).unwrap(), b"ls\r");
        assert_eq!(payload(b"ls", Some("crlf")).unwrap(), b"ls\r\n");
        // 行结束符单独发送时也必须非空。
        assert_eq!(payload(b"", Some("cr")).unwrap(), b"\r");
        assert!(payload(b"", None).is_err());
        assert!(payload(b"ls", Some("nope")).is_err());
        assert!(payload(b"ls", Some("lf")).is_ok());
    }

    #[test]
    fn text_write_rejects_binary_arguments_and_oversized_input() {
        assert!(text_payload(&json!({"data":"AA=="})).is_err());
        assert!(text_payload(&json!({})).is_err());
        let limit = "x".repeat(16 * 1024);
        assert_eq!(
            text_payload(&json!({"text":limit})).unwrap().len(),
            16 * 1024
        );
        // 同样的 16384 字节再加行结束符就超限了。
        assert!(text_payload(&json!({"text":limit,"lineEnding":"crlf"})).is_err());
        assert!(text_payload(&json!({"text":"x".repeat(16 * 1024 + 1)})).is_err());
    }

    #[test]
    fn mcp_read_returns_text_without_binary_payload() {
        let value = json!({
            "text": "COM2_OK\r\n",
            "cursor": "7",
            "gap": false,
            "closed": false,
            "more": false,
        });
        let result = mcp_result(value);
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text["text"], "COM2_OK\r\n");
        assert!(text.get("data").is_none());
        assert_eq!(text["cursor"], "7");
    }

    #[test]
    fn catalog_marks_human_input_as_sensitive_and_keeps_write_off_mcp() {
        let operations = catalog();
        let write = operations
            .iter()
            .find(|item| item.name == "serial.write")
            .expect("serial.write must stay available to the UI");
        // 人工输入可能是密码或密钥，账本只能保留脱敏占位。
        assert!(write.sensitive_input);
        // MCP 不暴露原始写入，也不暴露信号控制。
        let mcp: Vec<_> = operations
            .iter()
            .filter(|item| item.name.starts_with("mcp."))
            .map(|item| item.name.as_str())
            .collect();
        assert!(!mcp.iter().any(|name| name.ends_with("serial.write")));
        assert!(!mcp.iter().any(|name| name.ends_with("serial.signals")));
        // 镜像操作沿用领域操作的敏感标记，不能自报降级。
        for operation in operations
            .iter()
            .filter(|item| item.name.starts_with("mcp."))
        {
            let native = operation.name.trim_start_matches("mcp.");
            let base = operations
                .iter()
                .find(|item| item.name == native)
                .expect("mirrors must have a native operation");
            assert_eq!(operation.sensitive_input, base.sensitive_input);
        }
    }

    #[test]
    fn catalog_exposes_mcp_mirrors_without_raw_write() {
        let names: Vec<_> = catalog().into_iter().map(|item| item.name).collect();
        for expected in [
            "serial.status",
            "serial.probe",
            "serial.connect",
            "serial.disconnect",
            "serial.read",
            "serial.write",
            "serial.plugin_info",
            "mcp.serial.status",
            "mcp.serial.plugin_info",
        ] {
            assert!(names.contains(&expected.to_owned()), "missing {expected}");
        }
        assert!(
            !names.contains(&"mcp.serial.write".to_owned()),
            "raw byte writes must stay off the MCP surface"
        );
    }

    #[test]
    fn mcp_arguments_require_an_explicit_instance() {
        assert!(validate_mcp_args("mcp.serial.read", &json!({"pluginInstanceId":"one"})).is_ok());
        assert!(validate_mcp_args("mcp.serial.read", &json!({})).is_err());
        assert!(validate_mcp_args(
            "mcp.serial.read",
            &json!({"pluginInstanceId":"one","cursor":7})
        )
        .is_err());
        assert!(validate_mcp_args("mcp.serial.nope", &json!({})).is_err());
    }

    #[test]
    fn command_mcp_is_sensitive_commit_and_validates_full_lists() -> Result<()> {
        for name in ["serial.command", "mcp.serial.command"] {
            let descriptor = catalog()
                .into_iter()
                .find(|item| item.name == name)
                .ok_or_else(|| anyhow!("Missing command operation"))?;
            assert_eq!(descriptor.effect, OperationEffect::Commit);
            assert!(descriptor.sensitive_input);
            assert!(!descriptor.transient);
            assert_eq!(descriptor.input_schema["required"], json!(["commands"]));
        }
        assert!(validate_mcp_args(
            "mcp.serial.command",
            &json!({"pluginInstanceId":"one","commands":["pwd","ls"],"intervalMs":0})
        )
        .is_ok());
        for args in [
            json!({"commands":["pwd"]}),
            json!({"pluginInstanceId":"","commands":["pwd"]}),
            json!({"pluginInstanceId":"one","commands":[]}),
            json!({"pluginInstanceId":"one","commands":["pwd", ""]}),
            json!({"pluginInstanceId":"one","commands":["pwd"],"intervalMs":true}),
            json!({"pluginInstanceId":"one","commands":["pwd"],"extra":true}),
            json!({"pluginInstanceId":"one","text":"pwd"}),
        ] {
            assert!(
                validate_mcp_args("mcp.serial.command", &args).is_err(),
                "{args}"
            );
        }
        Ok(())
    }

    #[test]
    fn mcp_text_and_hex_are_separate_approved_list_contracts() -> Result<()> {
        let controls = "\u{3}\t\u{1b}[A\u{7f}\r\n中文";
        let text = CommandList::parse(&json!({"commands":[controls]}))?;
        assert_eq!(text.payloads[0], text_payload(&json!({"text":controls}))?);
        assert!(validate_mcp_args(
            "mcp.serial.command",
            &json!({"pluginInstanceId":"one","commands":[controls]})
        )
        .is_ok());
        assert!(validate_mcp_args(
            "mcp.serial.command_hex",
            &json!({"pluginInstanceId":"one","commands":["00 80 FF"]})
        )
        .is_ok());
        for args in [
            json!({"pluginInstanceId":"one","commands":["AID/"]}),
            json!({"pluginInstanceId":"one","commands":["00"],"lineEnding":"cr"}),
            json!({"pluginInstanceId":"one","data":"AA=="}),
        ] {
            assert!(validate_mcp_args("mcp.serial.command_hex", &args).is_err());
        }
        for name in ["serial.command_hex", "mcp.serial.command_hex"] {
            let descriptor = catalog()
                .into_iter()
                .find(|op| op.name == name)
                .ok_or_else(|| anyhow!("Missing HEX operation"))?;
            assert!(descriptor.sensitive_input);
            assert_eq!(descriptor.effect, OperationEffect::Commit);
            assert!(!descriptor.transient);
        }
        Ok(())
    }
}
