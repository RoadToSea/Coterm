use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use coterm_proto::runtime::{
    interaction::{
        DeferredInteraction, InteractionKind, InteractionOutcome, InteractionRequest,
        InteractionResult, InteractionValue, SecretDelivery, SecretHandle,
    },
    lifecycle::{CreateInstance, HelloResult, McpToolRegistration, RuntimeHello},
    message::{CancelTarget, DeferredOperation, PluginNotification},
    operation::{
        ApproveProposal, DispatchMode, InvokeRequest, OperationDescriptor, OperationEffect,
        OperationScope, OperationSource,
    },
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::session::{self, Config, Connection};

pub const PLUGIN_ID: &str = "coterm-plugin-ssh";

struct Instance {
    config: Config,
    connection: Option<Connection>,
    secret: Option<(SecretHandle, Zeroizing<String>)>,
}

struct PendingConnection {
    instance_id: String,
    operation_id: String,
    args: Value,
    expires: chrono::DateTime<Utc>,
}

struct Proposal {
    instance_id: String,
    operation_id: String,
    operation: String,
}

pub struct Backend {
    events: tokio::sync::mpsc::Sender<PluginNotification>,
    notifications: Option<tokio::sync::mpsc::Receiver<PluginNotification>>,
    instances: HashMap<String, Instance>,
    proposals: HashMap<String, Proposal>,
    interactions: HashMap<String, PendingConnection>,
}

impl Default for Backend {
    fn default() -> Self {
        let (events, notifications) = tokio::sync::mpsc::channel(128);
        Self {
            events,
            notifications: Some(notifications),
            instances: HashMap::new(),
            proposals: HashMap::new(),
            interactions: HashMap::new(),
        }
    }
}

pub fn decode<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| anyhow!("Invalid request parameters"))
}

pub fn catalog() -> Vec<OperationDescriptor> {
    let definitions = [
        ("ssh.status", OperationEffect::Read, false, json!({})),
        ("ssh.probe", OperationEffect::Read, false, json!({})),
        (
            "ssh.connect",
            OperationEffect::Commit,
            true,
            json!({"fingerprint":{"type":"string"},"secret":{"type":"object"}}),
        ),
        ("ssh.disconnect", OperationEffect::Commit, false, json!({})),
        (
            "ssh.read",
            OperationEffect::Read,
            false,
            json!({"cursor":{"type":"string","default":"0"}}),
        ),
        (
            "ssh.input",
            OperationEffect::Commit,
            true,
            json!({"data":{"type":"string","contentEncoding":"base64"}}),
        ),
        (
            "ssh.command",
            OperationEffect::Commit,
            false,
            json!({"text":{"type":"string","minLength":1,"maxLength":4096}}),
        ),
        (
            "ssh.resize",
            OperationEffect::Draft,
            false,
            json!({"cols":{"type":"integer","minimum":1,"maximum":1000},"rows":{"type":"integer","minimum":1,"maximum":1000}}),
        ),
    ];
    let mut operations: Vec<_> = definitions.into_iter().map(|(name, effect, sensitive_input, properties)| OperationDescriptor {
        name: name.to_owned(),
        scope: OperationScope::Instance,
        effect,
        input_schema: json!({"type":"object","properties":properties,"additionalProperties":false}),
        output_schema: None,
        sensitive_input,
        transient: matches!(name, "ssh.read" | "ssh.status" | "ssh.resize" | "ssh.input"),
    }).collect();
    operations.push(OperationDescriptor {
        name: "ssh.plugin_info".to_owned(),
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

/// MCP is an explicit plugin API. Raw terminal input and password handles are excluded.
pub fn mcp_tools() -> Vec<McpToolRegistration> {
    let mut tools: Vec<_> = [
        ("ssh.status", "Read connection status.", json!({}), vec![]),
        ("ssh.probe", "Probe the host key without authenticating. Verify it before connect.", json!({}), vec![]),
        ("ssh.connect", "Connect the existing instance. Waits for host approval and a human password prompt; never supply a password.", json!({"fingerprint":{"type":"string"}}), vec!["fingerprint"]),
        ("ssh.disconnect", "Disconnect this instance. May return pending approval.", json!({}), vec![]),
        ("ssh.command", "Run one command in the shared POSIX shell, waiting for approval and a completion marker. Returns bounded base64 output and exitCode. exit/exec or unsupported shells may produce an unknown result; never replay automatically.", json!({"text":{"type":"string","minLength":1,"maxLength":4096}}), vec!["text"]),
        ("ssh.read", "Read bounded base64 terminal bytes using a decimal cursor; reports gaps.", json!({"cursor":{"type":"string","default":"0"}}), vec![]),
    ].into_iter().map(|(name, description, mut properties, required)| {
        properties["pluginInstanceId"] = json!({"type":"string","minLength":1});
        let mut required = required;
        required.push("pluginInstanceId");
        McpToolRegistration {
            operation: format!("mcp.{name}"),
            descriptor: json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}),
        }
    }).collect();
    tools.push(McpToolRegistration {
        operation: "mcp.ssh.plugin_info".to_owned(),
        descriptor: json!({"name":"ssh.plugin_info","description":"Read plugin-level SSH backend information without selecting an instance.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}),
    });
    tools
}

fn validate_mcp_args(operation: &str, args: &Value) -> Result<()> {
    let tool = mcp_tools()
        .into_iter()
        .find(|tool| tool.operation == operation)
        .ok_or_else(|| anyhow!("Unknown SSH MCP tool"))?;
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
        bail!("Unknown or invalid SSH MCP argument");
    }
    if schema["required"].as_array().is_some_and(|required| {
        required
            .iter()
            .filter_map(Value::as_str)
            .any(|name| !object.contains_key(name))
    }) {
        bail!("Missing required SSH MCP argument");
    }
    Ok(())
}

fn mcp_result(value: Value) -> Value {
    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
}

impl Backend {
    pub fn take_notifications(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<PluginNotification>> {
        self.notifications.take()
    }

    async fn start_command(
        &mut self,
        id: &str,
        args: &Value,
        operation_id: &str,
        mcp: bool,
    ) -> Result<Value> {
        let text = command(args)?;
        let instance = self
            .instances
            .get(id)
            .ok_or_else(|| anyhow!("SSH instance not found"))?;
        connected(instance)?
            .command(operation_id.to_owned(), mcp, text)
            .await?;
        Ok(serde_json::to_value(DeferredOperation {
            operation_id: operation_id.to_owned(),
        })?)
    }

    pub fn expire_secrets(&mut self) {
        self.interactions
            .retain(|_, pending| pending.expires > Utc::now());
        for instance in self.instances.values_mut() {
            if instance
                .secret
                .as_ref()
                .is_some_and(|secret| secret.0.expires_at <= Utc::now())
            {
                instance.secret = None;
            }
        }
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.expire_secrets();
        match method {
            "initialize" => {
                let hello: RuntimeHello = decode(params)?;
                if hello.interface_major != 1 {
                    bail!("Unsupported runtime interface");
                }
                let enabled = [
                    "instances",
                    "backend-calls",
                    "operation-catalog",
                    "secret-delivery",
                    "native-mcp-tools",
                    "deferred-interactions",
                    "async-operations",
                    "plugin-events",
                ];
                if hello
                    .required_features
                    .iter()
                    .any(|feature| !enabled.contains(&feature.as_str()))
                {
                    bail!("Unsupported required runtime feature");
                }
                Ok(serde_json::to_value(HelloResult {
                    plugin_id: PLUGIN_ID.to_owned(),
                    plugin_version: env!("CARGO_PKG_VERSION").to_owned(),
                    interface_minor: 0,
                    enabled_features: enabled
                        .iter()
                        .map(|feature| (*feature).to_owned())
                        .collect(),
                    limits: hello.limits,
                    operations: catalog(),
                    mcp_tools: mcp_tools(),
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
                    bail!("Invalid or duplicate SSH instance (limit: 32)");
                }
                let config: Config = decode(create.config)?;
                config.validate()?;
                let label = format!("{}@{}:{}", config.username, config.host, config.port);
                self.instances.insert(
                    create.instance.plugin_instance_id,
                    Instance {
                        config,
                        connection: None,
                        secret: None,
                    },
                );
                Ok(json!({"ok":true,"label":label}))
            }
            "destroy_instance" => {
                let id = string(&params, "pluginInstanceId")?;
                if self.instances.remove(id).is_none() {
                    bail!("SSH instance not found");
                }
                self.proposals
                    .retain(|_, proposal| proposal.instance_id != id);
                self.interactions
                    .retain(|_, pending| pending.instance_id != id);
                Ok(json!({"ok":true}))
            }
            "provide_secret" => {
                let delivery: SecretDelivery = decode(params)?;
                if delivery.handle.purpose != "ssh.password"
                    || delivery.handle.expires_at <= Utc::now()
                    || delivery.value.len() > 16 * 1024
                {
                    bail!("Invalid SSH credential handle");
                }
                let instance = self
                    .instances
                    .get_mut(&delivery.handle.plugin_instance_id)
                    .ok_or_else(|| anyhow!("SSH instance not found"))?;
                instance.secret = Some((delivery.handle, Zeroizing::new(delivery.value)));
                Ok(json!({"ok":true}))
            }
            "interaction_result" => {
                let result: InteractionResult = decode(params)?;
                let pending = self
                    .interactions
                    .get(&result.interaction_id)
                    .ok_or_else(|| anyhow!("SSH interaction expired or cancelled"))?;
                if result.plugin_instance_id != pending.instance_id {
                    bail!("SSH interaction target mismatch");
                }
                let mut pending = self
                    .interactions
                    .remove(&result.interaction_id)
                    .ok_or_else(|| anyhow!("SSH interaction unavailable"))?;
                match result.outcome {
                    InteractionOutcome::Submitted(InteractionValue::Secret(handle)) => {
                        pending.args["secret"] = serde_json::to_value(handle)?;
                        let result = self
                            .execute(
                                &pending.instance_id,
                                "ssh.connect",
                                pending.args,
                                &pending.operation_id,
                            )
                            .await?;
                        Ok(mcp_result(result))
                    }
                    _ => {
                        if let Some(instance) = self.instances.get_mut(&pending.instance_id) {
                            instance.secret = None;
                        }
                        bail!("SSH password interaction cancelled or unavailable")
                    }
                }
            }
            "invoke" => self.invoke(decode(params)?).await,
            "cancel" => {
                let target: CancelTarget = decode(params)?;
                let CancelTarget::Operation(operation_id) = target else {
                    return Ok(json!({"outcome":"unsupported"}));
                };
                for instance in self.instances.values() {
                    if let Some(connection) = instance.connection.as_ref() {
                        if connection
                            .cancel_command(operation_id.clone())
                            .await
                            .is_ok()
                        {
                            return Ok(json!({"outcome":"unknown"}));
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
                    .ok_or_else(|| anyhow!("SSH proposal not found"))?;
                if Some(&proposal.instance_id) != approve.plugin_instance_id.as_ref()
                    || proposal.operation_id != approve.operation_id
                {
                    bail!("SSH proposal target mismatch");
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
                    .ok_or_else(|| anyhow!("SSH proposal not found"))?;
                if proposal.operation_id != string(&params, "operationId")? {
                    bail!("SSH proposal mismatch");
                }
                self.proposals.remove(id);
                Ok(json!({"ok":true}))
            }
            "shutdown" => {
                self.instances.clear();
                self.proposals.clear();
                self.interactions.clear();
                Ok(json!({"ok":true}))
            }
            _ => bail!("Unsupported SSH plugin method"),
        }
    }

    async fn invoke(&mut self, request: InvokeRequest) -> Result<Value> {
        if matches!(
            request.operation.as_str(),
            "ssh.plugin_info" | "mcp.ssh.plugin_info"
        ) {
            if request.plugin_instance_id.is_some() || request.effect != OperationEffect::Read {
                bail!("SSH plugin operation context mismatch");
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
            .ok_or_else(|| anyhow!("An explicit SSH instance is required"))?;
        let descriptor = catalog()
            .into_iter()
            .find(|operation| operation.name == request.operation)
            .ok_or_else(|| anyhow!("Unknown SSH operation"))?;
        if descriptor.effect != request.effect || !self.instances.contains_key(&id) {
            bail!("SSH operation context mismatch");
        }
        // Raw terminal keys can contain passwords. Agent uses the reviewed command path.
        if (request.operation == "ssh.input" || request.operation == "ssh.connect")
            && request.source != OperationSource::Human
        {
            bail!("This SSH operation requires human interaction");
        }
        if request.operation.starts_with("mcp.") {
            validate_mcp_args(&request.operation, &request.args)?;
            if request.args.get("pluginInstanceId").and_then(Value::as_str) != Some(&id) {
                bail!("MCP instance argument mismatch");
            }
            if request.args.get("secret").is_some() || request.args.get("password").is_some() {
                bail!("MCP must not supply credentials");
            }
        }
        if let DispatchMode::PrepareProposal { proposal_id } = request.dispatch {
            if self.proposals.len() >= 64 {
                bail!("Too many pending SSH proposals");
            }
            if request.operation == "ssh.command" || request.operation == "mcp.ssh.command" {
                command(&request.args)?;
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
            if native == "ssh.connect" {
                let fingerprint = string(&args, "fingerprint")?;
                if !fingerprint.starts_with("SHA256:") || fingerprint.len() > 128 {
                    bail!("A verified SHA256 host fingerprint is required");
                }
                if self.interactions.len() >= 32
                    || self.interactions.values().any(|p| p.instance_id == id)
                {
                    bail!("SSH password interaction already pending or capacity reached");
                }
                let instance = self
                    .instances
                    .get(id)
                    .ok_or_else(|| anyhow!("SSH instance not found"))?;
                if instance.connection.as_ref().is_some_and(|c| !c.is_closed()) {
                    bail!("SSH instance is already connected");
                }
                let interaction_id = format!("ssh-password-{operation_id}");
                self.interactions.insert(
                    interaction_id.clone(),
                    PendingConnection {
                        instance_id: id.to_owned(),
                        operation_id: operation_id.to_owned(),
                        args,
                        expires: Utc::now() + chrono::Duration::seconds(60),
                    },
                );
                return Ok(serde_json::to_value(DeferredInteraction {
                    request: InteractionRequest {
                        interaction_id,
                        plugin_instance_id: id.to_owned(),
                        operation_id: Some(operation_id.to_owned()),
                        kind: InteractionKind::Secret {
                            purpose: "ssh.password".to_owned(),
                        },
                        title: "SSH password".to_owned(),
                        prompt: format!(
                            "Password for {}@{}:{}",
                            instance.config.username, instance.config.host, instance.config.port
                        ),
                        timeout_ms: Some(60_000),
                        allow_cancel: true,
                        deadline: None,
                    },
                })?);
            }
            if native == "ssh.command" {
                return self.start_command(id, &args, operation_id, true).await;
            }
            let result = self.execute_domain(id, native, args).await?;
            return Ok(mcp_result(result));
        }
        if operation == "ssh.command" {
            return self.start_command(id, &args, operation_id, false).await;
        }
        self.execute_domain(id, operation, args).await
    }

    async fn execute_domain(&mut self, id: &str, operation: &str, args: Value) -> Result<Value> {
        if matches!(operation, "ssh.connect" | "ssh.disconnect") {
            self.interactions
                .retain(|_, pending| pending.instance_id != id);
            // Never apply a proposal prepared for a previous physical connection.
            self.proposals
                .retain(|_, proposal| proposal.instance_id != id);
        }
        let instance = self
            .instances
            .get_mut(id)
            .ok_or_else(|| anyhow!("SSH instance not found"))?;
        match operation {
            "ssh.status" => Ok(
                json!({"config":instance.config,"connected":instance.connection.as_ref().is_some_and(|connection| !connection.is_closed()),"connectionId":instance.connection.as_ref().map(|c| &c.id)}),
            ),
            "ssh.probe" => Ok(json!({"fingerprint":session::probe(&instance.config).await?})),
            "ssh.connect" => {
                if instance
                    .connection
                    .as_ref()
                    .is_some_and(|connection| !connection.is_closed())
                {
                    bail!("SSH instance is already connected");
                }
                let fingerprint = string(&args, "fingerprint")?.to_owned();
                let supplied: SecretHandle = decode(
                    args.get("secret")
                        .cloned()
                        .ok_or_else(|| anyhow!("A password handle is required"))?,
                )?;
                let valid = instance.secret.as_ref().is_some_and(|(handle, _)| {
                    handle.id == supplied.id
                        && handle.plugin_instance_id == id
                        && supplied.plugin_instance_id == id
                        && handle.purpose == "ssh.password"
                        && supplied.purpose == handle.purpose
                        && supplied.expires_at == handle.expires_at
                        && handle.expires_at > Utc::now()
                });
                if !valid {
                    bail!("Password handle is missing, expired, already used or belongs to another instance");
                }
                let (_, password) = instance
                    .secret
                    .take()
                    .ok_or_else(|| anyhow!("Password unavailable"))?;
                // Reset the old replay buffer only once a replacement connection is ready.
                instance.connection = Some(
                    Connection::connect_with_events(
                        &instance.config,
                        fingerprint,
                        &password,
                        Some((id.to_owned(), self.events.clone())),
                    )
                    .await?,
                );
                let connection_id = instance.connection.as_ref().map(|c| c.id.clone());
                let _ = self
                    .events
                    .send(PluginNotification::Event {
                        plugin_instance_id: id.to_owned(),
                        name: "ssh.status".into(),
                        payload: json!({"connected":true,"connectionId":connection_id}),
                    })
                    .await;
                Ok(json!({"connected":true}))
            }
            "ssh.disconnect" => {
                if let Some(connection) = &instance.connection {
                    let _ = connection.disconnect().await;
                }
                instance.connection = None;
                instance.secret = None;
                Ok(json!({"connected":false}))
            }
            "ssh.read" => {
                let cursor = args
                    .get("cursor")
                    .and_then(Value::as_str)
                    .unwrap_or("0")
                    .parse::<u64>()
                    .map_err(|_| anyhow!("Invalid SSH output cursor"))?;
                let connection = instance
                    .connection
                    .as_ref()
                    .ok_or_else(|| anyhow!("SSH is not connected"))?;
                let mut result = connection.output.lock().await.read(cursor)?;
                result["connectionId"] = json!(connection.id);
                Ok(result)
            }
            "ssh.input" => {
                let data = string(&args, "data")?;
                if data.len() > 22 * 1024 {
                    bail!("SSH input is too large");
                }
                let data = STANDARD
                    .decode(data)
                    .map_err(|_| anyhow!("SSH input must use base64 encoding"))?;
                connected(instance)?.input(data).await?;
                Ok(json!({"submitted":true}))
            }
            "ssh.resize" => {
                let cols = args
                    .get("cols")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| anyhow!("Invalid terminal columns"))?;
                let rows = args
                    .get("rows")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| anyhow!("Invalid terminal rows"))?;
                connected(instance)?.resize(cols, rows).await?;
                Ok(json!({"ok":true}))
            }
            _ => bail!("Unknown SSH operation"),
        }
    }
}

fn connected(instance: &Instance) -> Result<&Connection> {
    instance
        .connection
        .as_ref()
        .filter(|connection| !connection.is_closed())
        .ok_or_else(|| anyhow!("SSH is not connected"))
}

fn string<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing or invalid {field}"))
}

fn command(args: &Value) -> Result<&str> {
    let text = string(args, "text")?;
    if text.trim().is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
        bail!("SSH command must be a nonempty single line of at most 4096 bytes");
    }
    Ok(text)
}
