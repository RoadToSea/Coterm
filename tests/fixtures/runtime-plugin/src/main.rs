//! Hardware-free test plugin. Never included in application bundles or plugin discovery.
use coterm_proto::runtime::{
    backend::PluginBackend,
    interaction::{
        DeferredInteraction, InteractionKind, InteractionOutcome, InteractionRequest,
        InteractionValue,
    },
    lifecycle::HelloResult,
    message::{DeferredOperation, HostCommand, PluginNotification},
    operation::{DispatchMode, OperationDescriptor, OperationEffect, OperationScope},
    ErrorCategory, RuntimeError,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;

const ID: &str = "coterm-runtime-fixture";

fn console_attached() -> Option<bool> {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetConsoleWindow() -> *mut std::ffi::c_void;
        }
        // SAFETY: This parameterless Win32 query only reads this process's console handle.
        Some(unsafe { !GetConsoleWindow().is_null() })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

struct Backend {
    instances: HashMap<String, Value>,
    writes: usize,
    tx: mpsc::Sender<PluginNotification>,
    rx: Option<mpsc::Receiver<PluginNotification>>,
    secret: Option<String>,
}
fn failure(code: &str) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Domain,
        code: code.into(),
        message: code.into(),
        details: None,
    }
}
fn descriptor(name: &str, scope: OperationScope, effect: OperationEffect) -> OperationDescriptor {
    OperationDescriptor {
        name: name.into(),
        scope,
        effect,
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
        output_schema: None,
        sensitive_input: false,
        transient: false,
    }
}
impl PluginBackend for Backend {
    async fn request(&mut self, command: HostCommand) -> Result<Value, RuntimeError> {
        match command {
            HostCommand::Hello(hello) => {
                let features = hello.negotiate_features(&[
                    "instances",
                    "backend-calls",
                    "operation-catalog",
                    "secret-delivery",
                    "deferred-interactions",
                    "async-operations",
                    "plugin-events",
                ])?;
                serde_json::to_value(HelloResult {
                    plugin_id: ID.into(),
                    plugin_version: "0.1.0".into(),
                    interface_minor: 0,
                    enabled_features: features,
                    limits: hello.control_limits(),
                    operations: vec![
                        descriptor(
                            "fixture.info",
                            OperationScope::Instance,
                            OperationEffect::Read,
                        ),
                        descriptor(
                            "fixture.flood",
                            OperationScope::Instance,
                            OperationEffect::Read,
                        ),
                        descriptor(
                            "fixture.secret",
                            OperationScope::Instance,
                            OperationEffect::Read,
                        ),
                        descriptor(
                            "fixture.block",
                            OperationScope::Instance,
                            OperationEffect::Read,
                        ),
                        descriptor(
                            "fixture.hang",
                            OperationScope::Instance,
                            OperationEffect::Read,
                        ),
                        descriptor(
                            "fixture.plugin_hang",
                            OperationScope::Plugin,
                            OperationEffect::Read,
                        ),
                        descriptor(
                            "fixture.commit",
                            OperationScope::Instance,
                            OperationEffect::Commit,
                        ),
                        descriptor(
                            "fixture.crash",
                            OperationScope::Instance,
                            OperationEffect::Read,
                        ),
                    ],
                    mcp_tools: vec![],
                })
                .map_err(|_| failure("encode_failed"))
            }
            HostCommand::CreateInstance(request) => {
                if self.instances.len() >= 128 {
                    return Err(failure("capacity"));
                }
                let label = request
                    .config
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("Fixture")
                    .to_owned();
                self.instances
                    .insert(request.instance.plugin_instance_id, request.config);
                Ok(json!({"label":label}))
            }
            HostCommand::DestroyInstance { plugin_instance_id } => {
                self.instances.remove(&plugin_instance_id);
                Ok(json!({"ok":true}))
            }
            HostCommand::Invoke(request) => {
                if let DispatchMode::PrepareProposal { proposal_id } = request.dispatch {
                    return Ok(json!({"proposalId":proposal_id,"draft":request.args}));
                }
                let instance = request.plugin_instance_id.unwrap_or_default();
                if request.operation != "fixture.plugin_hang"
                    && !self.instances.contains_key(&instance)
                {
                    return Err(failure("missing_instance"));
                }
                match request.operation.as_str() {
                    "fixture.block" => std::future::pending::<Result<Value, RuntimeError>>().await,
                    "fixture.info" => Ok(
                        json!({"pid":std::process::id(),"instanceId":instance,"writes":self.writes,"consoleAttached":console_attached()}),
                    ),
                    "fixture.crash" => std::process::exit(23),
                    "fixture.hang" | "fixture.plugin_hang" => {
                        serde_json::to_value(DeferredOperation {
                            operation_id: request.operation_id,
                        })
                        .map_err(|_| failure("encode_failed"))
                    }
                    "fixture.commit" => {
                        self.writes += 1;
                        Ok(json!({"committed":true}))
                    }
                    "fixture.flood" => {
                        let tx = self.tx.clone();
                        tokio::spawn(async move {
                            for index in 0..4096 {
                                if tx
                                    .send(PluginNotification::Event {
                                        plugin_instance_id: instance.clone(),
                                        name: "fixture.data".into(),
                                        payload: json!({"index":index}),
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        });
                        Ok(json!({"accepted":true}))
                    }
                    "fixture.secret" => serde_json::to_value(DeferredInteraction {
                        request: InteractionRequest {
                            interaction_id: uuid::Uuid::new_v4().to_string(),
                            plugin_instance_id: instance,
                            operation_id: Some(request.operation_id),
                            kind: InteractionKind::Secret {
                                purpose: "fixture.password".into(),
                            },
                            title: "Headless fixture credential".into(),
                            prompt: "Enter a test credential".into(),
                            timeout_ms: Some(60_000),
                            allow_cancel: true,
                            deadline: None,
                        },
                    })
                    .map_err(|_| failure("encode_failed")),
                    _ => Err(failure("unsupported_operation")),
                }
            }
            HostCommand::ProvideSecret(delivery) => {
                self.secret = Some(delivery.handle.id);
                Ok(json!({"ok":true}))
            }
            HostCommand::InteractionResult(result) => {
                let accepted = matches!(result.outcome, InteractionOutcome::Submitted(InteractionValue::Secret(ref handle))
                    if self.secret.take().as_ref() == Some(&handle.id));
                Ok(json!({"accepted":accepted}))
            }
            HostCommand::Approve(_) => {
                self.writes += 1;
                Ok(json!({"committed":true}))
            }
            HostCommand::Reject { .. } => Ok(json!({"ok":true})),
            HostCommand::Cancel(_) => Ok(json!({"outcome":"stopped"})),
            HostCommand::Shutdown => {
                self.instances.clear();
                Ok(json!({"ok":true}))
            }
            _ => Err(failure("unsupported")),
        }
    }
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<PluginNotification>> {
        self.rx.take()
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::channel(64);
    coterm_proto::runtime::transport::serve(Backend {
        instances: HashMap::new(),
        writes: 0,
        tx,
        rx: Some(rx),
        secret: None,
    })
    .await?;
    Ok(())
}
