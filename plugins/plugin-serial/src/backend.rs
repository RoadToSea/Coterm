//! 串口插件的领域后端。只暴露 UART 语义：原始字节进、原始字节出。
use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use coterm_proto::runtime::{
    lifecycle::{CreateInstance, HelloResult, McpToolRegistration, RuntimeHello},
    message::{HostCommand, PluginNotification},
    operation::{
        ApproveProposal, DispatchMode, InvokeRequest, OperationDescriptor, OperationEffect,
        OperationScope,
    },
    ErrorCategory, RuntimeError,
};
use serde_json::{json, Value};

use crate::session::{Config, Connection};

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
        ("serial.disconnect", OperationEffect::Commit, false, json!({})),
        (
            "serial.read",
            OperationEffect::Read,
            false,
            json!({"cursor":{"type":"string","default":"0"}}),
        ),
        (
            "serial.write",
            OperationEffect::Commit,
            false,
            json!({"data":{"type":"string","contentEncoding":"base64"},"lineEnding":{"type":"string","enum":["none","lf","cr","crlf"]}}),
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
                    "serial.read" | "serial.status" | "serial.probe" | "serial.write"
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
    for registration in mcp_tools() {
        let native = registration.operation.trim_start_matches("mcp.");
        if let Some(base) = operations.iter().find(|item| item.name == native).cloned() {
            operations.push(OperationDescriptor {
                name: registration.operation,
                sensitive_input: false,
                ..base
            });
        }
    }
    operations
}

/// MCP 是显式插件 API；原始字节写入会直接驱动硬件，不在此暴露。
pub fn mcp_tools() -> Vec<McpToolRegistration> {
    let mut tools: Vec<_> = [
        ("serial.status", "Read the serial port configuration and connection state.", json!({}), vec![]),
        ("serial.probe", "List serial ports visible to the operating system together with this instance's configuration.", json!({}), vec![]),
        ("serial.connect", "Open the configured serial port.", json!({}), vec![]),
        ("serial.disconnect", "Close the serial port and release it.", json!({}), vec![]),
        ("serial.read", "Read bounded base64 bytes from the device using a decimal cursor; reports gaps.", json!({"cursor":{"type":"string","default":"0"}}), vec![]),
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
    if object
        .iter()
        .any(|(name, value)| !properties.contains_key(name) || !value.is_string())
    {
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
    Ok(())
}

fn mcp_result(value: Value) -> Value {
    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
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
                    .any(|instance| instance.config.port.eq_ignore_ascii_case(config.port.trim()))
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
                let Some(instance) = self.instances.remove(id) else {
                    bail!("Serial instance not found");
                };
                if let Some(connection) = &instance.connection {
                    let _ = connection.disconnect().await;
                }
                self.proposals
                    .retain(|_, proposal| proposal.instance_id != id);
                Ok(json!({"ok":true}))
            }
            "invoke" => self.invoke(decode(params)?).await,
            "cancel" => {
                // 串口写入是短事务，没有可取消的长时间操作。
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
                self.execute(&proposal.instance_id, &proposal.operation, approve.args, &proposal.operation_id)
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
                for instance in self.instances.values() {
                    if let Some(connection) = &instance.connection {
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
            if request.operation == "serial.write" {
                write_payload(&request.args)?;
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
            let result = self.execute_domain(id, native, args, operation_id).await?;
            return Ok(mcp_result(result));
        }
        self.execute_domain(id, operation, args, operation_id).await
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
            "serial.status" => Ok(json!({
                "config": instance.config,
                "connected": instance.connection.as_ref().is_some_and(|connection| !connection.is_closed()),
                "connectionId": instance.connection.as_ref().map(|connection| &connection.id),
            })),
            "serial.probe" => Ok(json!({
                "config": instance.config,
                "connected": instance.connection.as_ref().is_some_and(|connection| !connection.is_closed()),
                "ports": available_ports(),
            })),
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
                let connection_id = instance.connection.as_ref().map(|connection| connection.id.clone());
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
                if let Some(connection) = &instance.connection {
                    let _ = connection.disconnect().await;
                }
                instance.connection = None;
                Ok(json!({"connected":false}))
            }
            "serial.read" => {
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
                let mut result = connection.output.lock().await.read(cursor)?;
                result["connectionId"] = json!(connection.id);
                Ok(result)
            }
            "serial.write" => {
                let data = write_payload(&args)?;
                let submitted = data.len();
                connection_of(instance)?.write(data).await?;
                Ok(json!({"submitted":true,"bytes":submitted}))
            }
            _ => bail!("Unknown serial operation"),
        }
    }
}

/// 写入负载：base64 数据加上可选行结束符；行结束符由插件解释，不交给基座。
fn write_payload(args: &Value) -> Result<Vec<u8>> {
    let encoded = string(args, "data")?;
    let data = STANDARD
        .decode(encoded)
        .map_err(|_| anyhow!("Serial data must use base64 encoding"))?;
    let ending: &[u8] = match args.get("lineEnding").and_then(Value::as_str) {
        None | Some("none") => b"",
        Some("lf") => b"\n",
        Some("cr") => b"\r",
        Some("crlf") => b"\r\n",
        Some(_) => bail!("Invalid serial line ending"),
    };
    let mut payload = data;
    payload.extend_from_slice(ending);
    if payload.is_empty() || payload.len() > 16 * 1024 {
        bail!("Serial input must contain 1 to 16384 bytes");
    }
    Ok(payload)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(data: &[u8], ending: Option<&str>) -> Result<Vec<u8>> {
        let mut args = json!({"data": STANDARD.encode(data)});
        if let Some(ending) = ending {
            args["lineEnding"] = json!(ending);
        }
        write_payload(&args)
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
    fn write_payload_rejects_bad_base64_and_oversized_input() {
        assert!(write_payload(&json!({"data":"not base64!"})).is_err());
        assert!(write_payload(&json!({})).is_err());
        // base64 解码后恰好 16384 字节是上限。
        let limit = STANDARD.encode(vec![b'x'; 16 * 1024]);
        assert_eq!(write_payload(&json!({"data":limit})).unwrap().len(), 16 * 1024);
        // 同样的 16384 字节再加行结束符就超限了。
        assert!(write_payload(&json!({"data":limit,"lineEnding":"crlf"})).is_err());
        let over = STANDARD.encode(vec![b'x'; 16 * 1024 + 1]);
        assert!(write_payload(&json!({"data":over})).is_err());
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
        assert!(validate_mcp_args(
            "mcp.serial.read",
            &json!({"pluginInstanceId":"one"})
        )
        .is_ok());
        assert!(validate_mcp_args("mcp.serial.read", &json!({})).is_err());
        assert!(validate_mcp_args(
            "mcp.serial.read",
            &json!({"pluginInstanceId":"one","cursor":7})
        )
        .is_err());
        assert!(validate_mcp_args("mcp.serial.nope", &json!({})).is_err());
    }
}
