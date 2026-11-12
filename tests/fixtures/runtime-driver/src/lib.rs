//! Hardware-free test driver. Never included in application bundles or driver discovery.
use coterm_core::driver::DriverBackend;
use coterm_core::driver::{DeferredOperation, DriverNotification, HostCommand};
use coterm_core::model::{
    interaction::{
        DeferredInteraction, InteractionKind, InteractionOutcome, InteractionRequest,
        InteractionValue,
    },
    operation::{DispatchMode, OperationDescriptor, OperationEffect, OperationScope},
    ErrorCategory, RuntimeError,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;

const ID: &str = "coterm-runtime-fixture";

pub struct Backend {
    instances: HashMap<String, Value>,
    writes: usize,
    tx: mpsc::Sender<DriverNotification>,
    rx: Option<mpsc::Receiver<DriverNotification>>,
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
        input_schema: if name == "fixture.commit" {
            json!({"type":"object","properties":{"commands":{"type":"array","items":{"type":"string"}}},"additionalProperties":false,"x-coterm-approval-text":"/commands"})
        } else {
            json!({"type":"object","properties":{},"additionalProperties":false})
        },
        output_schema: None,
        sensitive_input: false,
        transient: false,
    }
}
impl DriverBackend for Backend {
    fn definition() -> coterm_core::driver::DriverDefinition {
        coterm_core::driver::DriverDefinition {
            driver_id: ID.into(),
            driver_version: "0.1.0".into(),
            features: [
                "instances",
                "backend-calls",
                "operation-catalog",
                "secret-delivery",
                "deferred-interactions",
                "async-operations",
                "driver-events",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
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
                    "fixture.early_completion",
                    OperationScope::Instance,
                    OperationEffect::Read,
                ),
                descriptor(
                    "fixture.driver_hang",
                    OperationScope::Driver,
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
        }
    }
    async fn request(&mut self, command: HostCommand) -> Result<Value, RuntimeError> {
        match command {
            HostCommand::CreateInstance(request) => {
                if self.instances.len() >= 128 {
                    return Err(failure("capacity"));
                }
                let name = request
                    .config
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Fixture")
                    .to_owned();
                self.instances
                    .insert(request.instance.driver_instance_id, request.config);
                Ok(json!({"name":name}))
            }
            HostCommand::DestroyInstance { driver_instance_id } => {
                self.instances.remove(&driver_instance_id);
                Ok(json!({"ok":true}))
            }
            HostCommand::Invoke(request) => {
                if let DispatchMode::PrepareProposal { proposal_id } = request.dispatch {
                    return Ok(json!({"proposalId":proposal_id,"draft":request.args}));
                }
                let instance = request.driver_instance_id.unwrap_or_default();
                if request.operation != "fixture.driver_hang"
                    && !self.instances.contains_key(&instance)
                {
                    return Err(failure("missing_instance"));
                }
                match request.operation.as_str() {
                    "fixture.block" => std::future::pending::<Result<Value, RuntimeError>>().await,
                    "fixture.info" => Ok(
                        json!({"pid":std::process::id(),"instanceId":instance,"writes":self.writes}),
                    ),
                    "fixture.crash" => panic!("fixture task panic"),
                    "fixture.early_completion" => {
                        self.tx
                            .send(DriverNotification::OperationCompleted {
                                operation_id: request.operation_id.clone(),
                                result: json!({"completedBeforeAck":true}),
                            })
                            .await
                            .map_err(|_| failure("notification_closed"))?;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        serde_json::to_value(DeferredOperation {
                            operation_id: request.operation_id,
                        })
                        .map_err(|_| failure("encode_failed"))
                    }
                    "fixture.hang" | "fixture.driver_hang" => {
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
                                    .send(DriverNotification::Event {
                                        driver_instance_id: instance.clone(),
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
                            driver_instance_id: instance,
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
        }
    }
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<DriverNotification>> {
        self.rx.take()
    }
}
impl Default for Backend {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            instances: HashMap::new(),
            writes: 0,
            tx,
            rx: Some(rx),
            secret: None,
        }
    }
}
