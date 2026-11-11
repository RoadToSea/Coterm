use super::{
    catalog, commands,
    config::Config,
    session::{Client, Session},
};
use crate::stream::{
    payload::{decode_hex, encode_text},
    result::tool_result,
    shell::Mode,
};
use anyhow::{anyhow, bail, Result};
use coterm_core::{
    driver::{
        CancelTarget, DeferredOperation, DriverBackend, DriverDefinition, DriverNotification,
        HostCommand,
    },
    model::{
        operation::{DispatchMode, InvokeRequest, OperationEffect, OperationSource},
        ErrorCategory, RuntimeError,
    },
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;

struct Instance {
    config: Config,
    session: Option<Session>,
    revision: u64,
    connect_operation: Option<String>,
    jobs: HashMap<String, commands::Job>,
}
struct Proposal {
    instance: String,
    operation: String,
    operation_id: String,
    revision: u64,
    epoch: Option<String>,
}

/// TCP lifecycle implementation selected by the concrete Raw TCP / Telnet entry points.
/// Requests use the existing typed driver contract; invalid targets and stale epochs fail closed.
pub struct Backend<const TELNET: bool> {
    instances: HashMap<String, Instance>,
    proposals: HashMap<String, Proposal>,
    events: mpsc::Sender<DriverNotification>,
    receiver: Option<mpsc::Receiver<DriverNotification>>,
}

impl<const TELNET: bool> Default for Backend<TELNET> {
    fn default() -> Self {
        let (events, receiver) = mpsc::channel(128);
        Self {
            instances: HashMap::new(),
            proposals: HashMap::new(),
            events,
            receiver: Some(receiver),
        }
    }
}

impl<const TELNET: bool> DriverBackend for Backend<TELNET> {
    fn definition() -> DriverDefinition {
        DriverDefinition {
            driver_id: catalog::driver_id(TELNET).into(),
            driver_version: env!("CARGO_PKG_VERSION").into(),
            features: [
                "instances",
                "backend-calls",
                "operation-catalog",
                "native-mcp-tools",
                "async-operations",
                "driver-events",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            operations: catalog::catalog(TELNET),
            mcp_tools: catalog::mcp_tools(TELNET),
        }
    }
    async fn request(&mut self, command: HostCommand) -> Result<Value, RuntimeError> {
        self.handle(command)
            .await
            .map_err(|e| error("network_domain_error", e.to_string()))
    }
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<DriverNotification>> {
        self.receiver.take()
    }
    fn tick(&mut self) {
        for instance in self.instances.values_mut() {
            instance.jobs.retain(|_, job| !job.finished());
        }
    }
}

pub(crate) fn error(code: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Domain,
        code: code.into(),
        message: message.into(),
        details: None,
    }
}

impl<const TELNET: bool> Backend<TELNET> {
    async fn handle(&mut self, command: HostCommand) -> Result<Value> {
        match command {
            HostCommand::CreateInstance(create) => {
                if create.instance.driver_id != catalog::driver_id(TELNET)
                    || self
                        .instances
                        .contains_key(&create.instance.driver_instance_id)
                    || self.instances.len() >= 32
                {
                    bail!("Invalid or duplicate instance (limit: 32)")
                }
                let config = Config::parse(create.config, TELNET)?;
                let label = format!("{}:{}", config.host, config.port);
                self.instances.insert(
                    create.instance.driver_instance_id,
                    Instance {
                        config,
                        session: None,
                        revision: 0,
                        connect_operation: None,
                        jobs: HashMap::new(),
                    },
                );
                Ok(json!({"ok":true,"label":label}))
            }
            HostCommand::DestroyInstance { driver_instance_id } => {
                let Some(mut instance) = self.instances.remove(&driver_instance_id) else {
                    bail!("Instance not found")
                };
                stop(&mut instance).await;
                self.proposals
                    .retain(|_, p| p.instance != driver_instance_id);
                Ok(json!({"ok":true}))
            }
            HostCommand::Invoke(request) => self.invoke(request).await,
            HostCommand::Approve(approved) => {
                let proposal = self
                    .proposals
                    .remove(&approved.proposal_id)
                    .ok_or_else(|| anyhow!("Proposal not found"))?;
                if proposal.operation_id != approved.operation_id
                    || approved.driver_instance_id.as_deref() != Some(&proposal.instance)
                {
                    bail!("Proposal target mismatch")
                }
                let instance = self
                    .instances
                    .get(&proposal.instance)
                    .ok_or_else(|| anyhow!("Instance not found"))?;
                if instance.revision != proposal.revision {
                    bail!("Configuration changed; prepare a new operation")
                }
                if let Some(epoch) = proposal.epoch {
                    if client(instance)?.epoch().await? != epoch {
                        bail!("Connection changed; prepared input is stale")
                    }
                }
                self.validate(&proposal.instance, &proposal.operation, &approved.args)
                    .await?;
                self.execute(
                    &proposal.instance,
                    &proposal.operation,
                    approved.args,
                    &proposal.operation_id,
                )
                .await
            }
            HostCommand::Reject {
                operation_id,
                proposal_id,
                ..
            } => {
                if self
                    .proposals
                    .get(&proposal_id)
                    .is_none_or(|p| p.operation_id != operation_id)
                {
                    bail!("Proposal mismatch")
                }
                self.proposals.remove(&proposal_id);
                Ok(json!({"ok":true}))
            }
            HostCommand::Cancel(CancelTarget::Operation(id)) => {
                for instance in self.instances.values_mut() {
                    if let Some(job) = instance.jobs.get_mut(&id) {
                        return Ok(job.cancel().await);
                    }
                    if instance.connect_operation.as_deref() == Some(&id)
                        && if let Some(session) = &instance.session {
                            session.connecting().await
                        } else {
                            false
                        }
                    {
                        stop(instance).await;
                        return Ok(json!({"outcome":"stopped"}));
                    }
                }
                Ok(json!({"outcome":"unsupported"}))
            }
            HostCommand::Shutdown => {
                for instance in self.instances.values_mut() {
                    stop(instance).await;
                }
                self.instances.clear();
                self.proposals.clear();
                Ok(json!({"ok":true}))
            }
            _ => bail!("This driver does not accept secret delivery or interaction results"),
        }
    }

    async fn invoke(&mut self, request: InvokeRequest) -> Result<Value> {
        let id = request
            .driver_instance_id
            .as_ref()
            .ok_or_else(|| anyhow!("An explicit instance is required"))?;
        let descriptor = catalog::catalog(TELNET)
            .into_iter()
            .find(|d| d.name == request.operation)
            .ok_or_else(|| anyhow!("Unknown operation"))?;
        if descriptor.effect != request.effect {
            bail!("Operation effect mismatch")
        }
        let native = native(&request.operation, TELNET)?;
        if matches!(native, "write" | "write_hex" | "resize" | "set_selection")
            && request.source != OperationSource::Human
        {
            bail!("Raw terminal input and selection can only be changed by Human input")
        }
        validate_schema(&descriptor.input_schema, &request.args)?;
        if request.operation.starts_with("mcp.")
            && request.args["driverInstanceId"].as_str() != Some(id)
        {
            bail!("MCP instance mismatch")
        }
        self.validate(id, &request.operation, &request.args).await?;
        if let DispatchMode::PrepareProposal { proposal_id } = request.dispatch {
            if request.effect != OperationEffect::Commit || self.proposals.len() >= 64 {
                bail!("Invalid proposal or proposal capacity reached")
            }
            let instance = self
                .instances
                .get(id)
                .ok_or_else(|| anyhow!("Instance not found"))?;
            let epoch = if matches!(native, "command" | "command_hex" | "write" | "write_hex") {
                Some(client(instance)?.epoch().await?)
            } else {
                None
            };
            self.proposals.insert(
                proposal_id.clone(),
                Proposal {
                    instance: id.clone(),
                    operation: request.operation.clone(),
                    operation_id: request.operation_id,
                    revision: instance.revision,
                    epoch,
                },
            );
            return Ok(
                json!({"proposalId":proposal_id,"operation":request.operation,"args":request.args}),
            );
        }
        self.execute(id, &request.operation, request.args, &request.operation_id)
            .await
    }

    async fn validate(&self, id: &str, operation: &str, args: &Value) -> Result<()> {
        if operation.starts_with("mcp.") && args["driverInstanceId"].as_str() != Some(id) {
            bail!("MCP instance mismatch")
        }
        let instance = self
            .instances
            .get(id)
            .ok_or_else(|| anyhow!("Instance not found"))?;
        let args = arguments(args);
        match native(operation, TELNET)? {
            "command" | "command_hex" => {
                commands::List::parse(args, &instance.config, operation.ends_with("command_hex"))?;
            }
            "write" => {
                encode_text(
                    string(&args, "text")?,
                    args["lineEnding"].as_str().unwrap_or("none"),
                )?;
            }
            "write_hex" => {
                decode_hex(string(&args, "hex")?)?;
            }
            "configure" => {
                instance.config.patch(&args, TELNET)?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn execute(
        &mut self,
        id: &str,
        operation: &str,
        args: Value,
        operation_id: &str,
    ) -> Result<Value> {
        let mcp = operation.starts_with("mcp.");
        let name = native(operation, TELNET)?;
        let args = arguments(&args);
        let instance = self
            .instances
            .get_mut(id)
            .ok_or_else(|| anyhow!("Instance not found"))?;
        let value = match name {
            "status" | "probe" => status(instance).await,
            "connect" => {
                if instance.session.as_ref().is_some_and(Session::active) {
                    bail!("Already connected, connecting or reconnecting")
                }
                instance.jobs.clear();
                instance.revision += 1;
                instance.connect_operation = Some(operation_id.into());
                instance.session = Some(Session::start(
                    instance.config.clone(),
                    TELNET,
                    id.into(),
                    operation_id.into(),
                    mcp,
                    self.events.clone(),
                ));
                return Ok(serde_json::to_value(DeferredOperation {
                    operation_id: operation_id.into(),
                })?);
            }
            "disconnect" => {
                stop(instance).await;
                self.proposals.retain(|_, p| p.instance != id);
                status(instance).await
            }
            "configure" => {
                let next = instance.config.patch(&args, TELNET)?;
                let interpretation = args
                    .as_object()
                    .is_some_and(|p| p.keys().all(|k| matches!(k.as_str(), "mode" | "prompt")));
                if let Some(session) = &instance.session {
                    if !interpretation && session.active() {
                        bail!("Disconnect before changing connection parameters")
                    }
                    if interpretation {
                        if session.client.busy.try_lock().is_err() {
                            bail!("Wait for the command queue before switching modes")
                        }
                        *session.client.interpretation.lock().await =
                            (next.mode, next.prompt.clone());
                        session
                            .client
                            .output
                            .lock()
                            .await
                            .shell
                            .configure(next.mode, next.prompt.as_deref());
                    }
                }
                if !interpretation {
                    instance.session = None;
                }
                instance.config = next;
                instance.revision += 1;
                self.proposals.retain(|_, p| p.instance != id);
                status(instance).await
            }
            "read" | "read_hex" => {
                let client = client(instance)?;
                let state = client.state.lock().await;
                let mut value = if name == "read" {
                    client
                        .output
                        .lock()
                        .await
                        .read(number(&args, "cursor", 0)?)?
                } else {
                    client
                        .output
                        .lock()
                        .await
                        .read_hex(number(&args, "cursor", 0)?)?
                };
                value["connectionId"] = json!(state.connection_id);
                value
            }
            "read_lines" => {
                if let Some(session) = &instance.session {
                    session
                        .client
                        .output
                        .lock()
                        .await
                        .read_lines(number(&args, "lines", 100)? as usize)?
                } else {
                    json!({"lines":[],"stderrLines":[],"streamMode":"merged","closed":true,"truncated":false,"dropped":false})
                }
            }
            "read_selected_lines" | "read_dialogue_pairs" => {
                if instance.config.mode != Mode::Shell {
                    bail!("Byte streams only support read_lines; switch to terminal mode for Shell reads")
                }
                let client = client(instance)?;
                let buffer = client.output.lock().await;
                if name == "read_selected_lines" {
                    buffer.shell.selection()?
                } else {
                    buffer.shell.pairs(number(&args, "count", 2)? as usize)?
                }
            }
            "set_selection" => {
                let client = client(instance)?;
                check_epoch(&client, &args).await?;
                client.output.lock().await.shell.set_selection(
                    string(&args, "text")?,
                    args["truncated"].as_bool().unwrap_or(false),
                )?;
                json!({"ok":true})
            }
            "write" | "write_hex" => {
                let client = client(instance)?;
                let _guard = client
                    .busy
                    .try_lock()
                    .map_err(|_| anyhow!("Command queue is busy"))?;
                let epoch = check_epoch(&client, &args).await?;
                let bytes = if name == "write" {
                    encode_text(
                        string(&args, "text")?,
                        args["lineEnding"].as_str().unwrap_or("none"),
                    )?
                } else {
                    decode_hex(string(&args, "hex")?)?
                };
                json!({"bytes":client.write(&epoch,bytes).await?})
            }
            "resize" => {
                let client = client(instance)?;
                let epoch = check_epoch(&client, &args).await?;
                client
                    .resize(
                        &epoch,
                        number(&args, "cols", 80)? as u16,
                        number(&args, "rows", 24)? as u16,
                    )
                    .await?;
                json!({"ok":true})
            }
            "command" | "command_hex" => {
                let list = commands::List::parse(args, &instance.config, name == "command_hex")?;
                let job = commands::start(
                    client(instance)?,
                    list,
                    operation_id.into(),
                    mcp,
                    self.events.clone(),
                )
                .await?;
                instance.jobs.retain(|_, job| !job.finished());
                instance.jobs.insert(operation_id.into(), job);
                return Ok(serde_json::to_value(DeferredOperation {
                    operation_id: operation_id.into(),
                })?);
            }
            _ => bail!("Unknown operation"),
        };
        Ok(if mcp { tool_result(value) } else { value })
    }
}

fn client(instance: &Instance) -> Result<Client> {
    instance
        .session
        .as_ref()
        .map(|s| s.client.clone())
        .ok_or_else(|| anyhow!("No connection has been opened"))
}
async fn check_epoch(client: &Client, args: &Value) -> Result<String> {
    let current = client.epoch().await?;
    if args["connectionId"].as_str() != Some(&current) {
        bail!("Stale connection input or selection rejected")
    }
    Ok(current)
}
async fn stop(instance: &mut Instance) {
    if let Some(session) = instance.session.as_mut() {
        session.disconnect().await;
    }
    for job in instance.jobs.values_mut() {
        job.cancel().await;
    }
    instance.jobs.clear();
    instance.revision += 1;
}
async fn status(instance: &Instance) -> Value {
    let mut value = json!({"config":instance.config,"connected":false,"reconnecting":false,"connectionId":null,"remoteEcho":false,
        "rxBytes":0,"txBytes":0,"commandQueue":instance.jobs.values().filter(|j| !j.finished()).count(),
        "shellReady":false,"capabilities":{"readLines":true,"readSelectedLines":instance.config.mode == Mode::Shell,"readDialoguePairs":instance.config.mode == Mode::Shell}});
    if let Some(session) = &instance.session {
        let state = session.client.state.lock().await;
        value["connected"] = json!(state.connected && session.active());
        value["reconnecting"] = json!(state.reconnecting && session.active());
        value["connectionId"] = json!(state.connection_id);
        value["remoteEcho"] = json!(state.remote_echo);
        value["rxBytes"] = json!(state.rx);
        value["txBytes"] = json!(state.tx);
        value["lastError"] = json!(state.error);
        value["shellReady"] = json!(session.client.output.lock().await.shell.ready());
    }
    value
}
fn native(operation: &str, telnet: bool) -> Result<&str> {
    operation
        .strip_prefix("mcp.")
        .unwrap_or(operation)
        .strip_prefix(&format!("{}.", catalog::prefix(telnet)))
        .ok_or_else(|| anyhow!("Invalid driver namespace"))
}
fn arguments(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("driverInstanceId");
    }
    value
}
fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .ok_or_else(|| anyhow!("Missing or invalid {name}"))
}
fn number(value: &Value, name: &str, default: u64) -> Result<u64> {
    match value.get(name) {
        None => Ok(default),
        Some(v) => v.as_u64().ok_or_else(|| anyhow!("Invalid {name}")),
    }
}

fn validate_schema(schema: &Value, value: &Value) -> Result<()> {
    let Some(args) = value.as_object() else {
        bail!("Arguments must be an object")
    };
    let properties = schema["properties"]
        .as_object()
        .ok_or_else(|| anyhow!("Invalid operation schema"))?;
    for required in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !args.contains_key(required) {
            bail!("Missing {required}")
        }
    }
    for (name, value) in args {
        let property = properties
            .get(name)
            .ok_or_else(|| anyhow!("Unknown argument {name}"))?;
        let valid = match property["type"].as_str() {
            Some("string") => value.is_string(),
            Some("integer") => value.is_u64(),
            Some("boolean") => value.is_boolean(),
            Some("object") => value.is_object(),
            Some("array") => value.is_array(),
            None if property["type"].is_array() => value.is_string() || value.is_null(),
            _ => false,
        };
        if !valid {
            bail!("Invalid type for {name}")
        }
        if let Some(allowed) = property["enum"].as_array() {
            if !allowed.contains(value) {
                bail!("Invalid {name}")
            }
        }
        if let Some(number) = value.as_u64() {
            if property["minimum"].as_u64().is_some_and(|n| number < n)
                || property["maximum"].as_u64().is_some_and(|n| number > n)
            {
                bail!("Invalid range for {name}")
            }
        }
    }
    Ok(())
}
