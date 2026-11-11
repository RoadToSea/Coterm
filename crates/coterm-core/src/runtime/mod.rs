//! 内置驱动生命周期、实例路由与操作审批。

mod context;
mod task;

pub use context::{CallerContext, SubmitOperation};
pub use task::DriverTask;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::driver::{DeferredOperation, DriverNotification, HostCommand};
use crate::model::{
    interaction::{
        DeferredInteraction, InteractionKind, InteractionOutcome, InteractionRequest,
        InteractionResult, InteractionValue, SecretDelivery, SecretHandle,
    },
    lifecycle::{CreateInstance, InstanceInfo, InstanceStatus, McpToolRegistration, RuntimeLimits},
    manifest::DriverManifest,
    operation::{
        ApproveProposal, CancellationOutcome, DispatchMode, InvokeRequest, OperationDescriptor,
        OperationReceipt, OperationRecord, OperationScope, OperationSource, OperationStatus,
    },
    ErrorCategory, RuntimeError,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    approval_policy::{ApprovalContext, ApprovalPolicy, InstanceApproval},
    operation_history::{OperationHistory, OperationHistoryError},
};

#[derive(Default)]
struct State {
    mcp_tools: HashMap<String, Vec<McpToolRegistration>>,
    features: HashMap<String, Vec<String>>,
    interactions: HashMap<String, (InteractionRequest, tokio::time::Instant)>,
    drivers: HashMap<String, BuiltinDriver>,
    running: HashMap<String, Arc<DriverTask>>,
    instances: HashMap<String, InstanceInfo>,
    instance_approvals: HashMap<String, InstanceApproval>,
    operation_catalog: HashMap<String, HashMap<String, OperationDescriptor>>,
    /// (driver_id, driver_instance_id, proposal_id, operation)
    pending_approvals: HashMap<String, (String, Option<String>, String, String)>,
    /// 敏感草稿仅在人工审批读取时提供，不能经普通账本或 MCP 返回。
    approval_drafts: HashMap<String, Value>,
}

impl State {
    fn prune_approval_drafts(&mut self) {
        self.approval_drafts
            .retain(|id, _| self.pending_approvals.contains_key(id));
    }

    fn with_approval_draft(&self, mut record: OperationRecord) -> OperationRecord {
        if record.status == OperationStatus::AwaitingApproval {
            if let Some(draft) = self.approval_drafts.get(&record.operation_id) {
                record.output = Some(draft.clone());
            }
        }
        record
    }
}

/// 宿主级待处理操作；只含 UI 渲染所需字段，不含操作参数或秘密值。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub operation_id: String,
    pub proposal_id: String,
    pub driver_id: String,
    /// None 表示驱动级操作。
    pub driver_instance_id: Option<String>,
    pub operation: String,
    pub target_label: String,
    pub created_at: String,
}

/// 宿主级待交互请求；秘密值永不在此出现。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInteraction {
    pub interaction_id: String,
    pub driver_instance_id: String,
    pub operation_id: Option<String>,
    pub kind: InteractionKind,
    pub title: String,
    pub prompt: String,
    pub allow_cancel: bool,
    /// 由基座按绝对期限计算，前端不自行计时。
    pub remaining_ms: u64,
    pub deadline: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionSnapshot {
    /// 单调递增；每次宿主待处理状态变化 +1。
    pub version: u64,
    pub approvals: Vec<PendingApproval>,
    pub interactions: Vec<PendingInteraction>,
}

/// 驱动目录诊断的兼容返回类型；编译期注册失败时应用直接拒绝启动。
/// 只含驱动目录名、驱动 ID（能解析时）和错误码，不含绝对路径或内部错误明文。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriverLoadIssue {
    /// 驱动目录名；用于定位是哪个包出了问题。
    pub directory: String,
    /// manifest 能被解析且 ID 合法时给出。
    pub driver_id: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Clone)]
struct BuiltinDriver {
    manifest: DriverManifest,
    definition: crate::driver::DriverDefinition,
    frontend: Option<&'static str>,
    spawn: fn() -> DriverTask,
}

/// 一个宿主运行时；每种内置驱动独享一个后端任务，各实例状态由驱动分别保存。
#[derive(Clone)]
pub struct Runtime {
    limits: RuntimeLimits,
    state: Arc<Mutex<State>>,
    control_gates: Arc<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    history: Arc<OperationHistory>,
    attention: Arc<std::sync::Mutex<u64>>,
    tasks: Arc<std::sync::Mutex<HashMap<String, std::sync::Weak<DriverTask>>>>,
    shutting_down: Arc<std::sync::atomic::AtomicBool>,
}

impl Runtime {
    /// 创建尚未注册/启动后端的运行时；limits 在驱动注册时校验，不进行 IO。
    pub fn new(limits: RuntimeLimits) -> Self {
        Self {
            limits,
            state: Arc::new(Mutex::new(State::default())),
            control_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
            history: Arc::new(OperationHistory::new(100, 16 * 1024).with_output_limit(192 * 1024)),
            attention: Arc::new(std::sync::Mutex::new(0)),
            tasks: Arc::new(std::sync::Mutex::new(HashMap::new())),
            shutting_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub async fn create_instance_with_name(
        &self,
        driver_id: &str,
        config: Value,
        name: Option<String>,
    ) -> Result<InstanceInfo, RuntimeError> {
        let name = match name {
            Some(name) => {
                let name = name.trim();
                if name.len() > 256 || name.chars().any(char::is_control) {
                    return Err(runtime_error(
                        ErrorCategory::InvalidRequest,
                        "instance_name_invalid",
                    ));
                }
                (!name.is_empty()).then(|| name.to_owned())
            }
            None => None,
        };
        let _control = self.control_gate(driver_id).lock_owned().await;
        let state = self.state.lock().await;
        self.ensure_accepting()?;
        let driver = state
            .drivers
            .get(driver_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_not_loaded"))?;
        if !driver.manifest.instances.multiple {
            return Err(runtime_error(
                ErrorCategory::Unsupported,
                "driver_multi_instance_unsupported",
            ));
        }
        let task = state
            .running
            .get(driver_id)
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "driver_not_started"))?;
        let instance_id = uuid::Uuid::new_v4().to_string();
        let instance = InstanceInfo {
            driver_id: driver_id.to_owned(),
            driver_instance_id: instance_id.clone(),
            session_id: uuid::Uuid::new_v4().to_string(),
            label: format!("{} {instance_id}", driver.manifest.name),
            status: InstanceStatus::Starting,
        };
        drop(state);
        let created = task
            .request(HostCommand::CreateInstance(CreateInstance {
                instance: instance.clone(),
                config,
            }))
            .await?;
        let mut state = self.state.lock().await;
        ensure_current_task(&state, driver_id, &task)?;
        self.ensure_accepting()?;
        let requested_label = name
            .or_else(|| {
                created
                    .get("label")
                    .and_then(Value::as_str)
                    .filter(|label| {
                        !label.trim().is_empty()
                            && label.len() <= 256
                            && !label.chars().any(char::is_control)
                    })
                    .map(str::to_owned)
            })
            .unwrap_or(instance.label);
        let label = unique_instance_label(
            &requested_label,
            state.instances.values().map(|item| item.label.as_str()),
        );
        let ready = InstanceInfo {
            label,
            status: InstanceStatus::Ready,
            ..instance
        };
        state
            .instance_approvals
            .insert(instance_id.clone(), InstanceApproval::default());
        state.instances.insert(instance_id, ready.clone());
        Ok(ready)
    }

    fn control_gate(&self, driver_id: &str) -> Arc<Mutex<()>> {
        self.control_gates
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(driver_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn operation_control(
        &self,
        operation_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, RuntimeError> {
        let record = self
            .history
            .get(operation_id)
            .await
            .map_err(map_history_error)?;
        Ok(self.control_gate(&record.driver_id).lock_owned().await)
    }

    fn ensure_accepting(&self) -> Result<(), RuntimeError> {
        if self
            .shutting_down
            .load(std::sync::atomic::Ordering::Acquire)
        {
            Err(runtime_error(
                ErrorCategory::Conflict,
                "runtime_shutting_down",
            ))
        } else {
            Ok(())
        }
    }

    /// 必须在持有 `state` 锁时调用，保证版本与快照内容一致。
    fn bump_attention(&self) {
        let mut version = self
            .attention
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *version += 1;
    }

    /// Stop every driver task within a bounded deadline, without replaying any operation.
    pub async fn shutdown(&self) {
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::Release);
        // This registry is independent of the routing lock: an unresponsive control
        // request may hold that lock, but must never prevent cancelling its backend.
        let backends = self
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter_map(std::sync::Weak::upgrade)
            .collect::<Vec<_>>();
        let mut tasks = tokio::task::JoinSet::new();
        for task in backends {
            tasks.spawn(async move {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(750),
                    task.request(HostCommand::Shutdown),
                )
                .await;
                let _ = task.close().await;
            });
        }
        while tasks.join_next().await.is_some() {}
        let mut state = self.state.lock().await;
        let ids = state.running.keys().cloned().collect::<Vec<_>>();
        state.running.clear();
        for id in ids {
            settle_closed_driver(&mut state, &self.history, &id).await;
        }
        state.instances.clear();
        self.bump_attention();
    }

    pub fn attention_version(&self) -> u64 {
        *self
            .attention
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// 一次调用覆盖全部实例级与驱动级待处理项。
    pub async fn attention_snapshot(&self) -> AttentionSnapshot {
        // 先取版本再读状态：并发变化只会让前端拿到较旧的手快照，
        // 而对应 changed 事件已经包含更高版本，前端会据此重取。
        let version = self.attention_version();
        let state = self.state.lock().await;
        let now = tokio::time::Instant::now();
        let mut approvals = state
            .pending_approvals
            .iter()
            .map(
                |(operation_id, (driver_id, instance, proposal_id, operation))| {
                    let target_label = instance
                        .as_ref()
                        .and_then(|id| state.instances.get(id))
                        .map(|instance| instance.label.clone())
                        .unwrap_or_else(|| driver_id.clone());
                    PendingApproval {
                        operation_id: operation_id.clone(),
                        proposal_id: proposal_id.clone(),
                        driver_id: driver_id.clone(),
                        driver_instance_id: instance.clone(),
                        operation: operation.clone(),
                        target_label,
                        created_at: String::new(),
                    }
                },
            )
            .collect::<Vec<_>>();
        approvals.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
        let mut interactions = state
            .interactions
            .values()
            .map(|(request, deadline)| (request, deadline))
            .filter(|(_, deadline)| **deadline > now)
            .map(|(request, deadline)| PendingInteraction {
                interaction_id: request.interaction_id.clone(),
                driver_instance_id: request.driver_instance_id.clone(),
                operation_id: request.operation_id.clone(),
                kind: request.kind.clone(),
                title: request.title.clone(),
                prompt: request.prompt.clone(),
                allow_cancel: request.allow_cancel,
                remaining_ms: deadline.duration_since(now).as_millis() as u64,
                deadline: request
                    .deadline
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        drop(state);
        for approval in &mut approvals {
            approval.created_at = self
                .history
                .get(&approval.operation_id)
                .await
                .map(|record| record.created_at.to_rfc3339())
                .unwrap_or_default();
        }
        interactions.sort_by(|left, right| left.interaction_id.cmp(&right.interaction_id));
        AttentionSnapshot {
            version,
            approvals,
            interactions,
        }
    }

    pub async fn is_running(&self, driver_id: &str) -> bool {
        self.state.lock().await.running.contains_key(driver_id)
    }

    /// 注册编译进宿主的驱动工厂和自包含前端源码；不访问文件系统或启动程序。
    /// 重复 ID、无效声明、前端缺失/超限或运行时已停止时返回错误，旧注册保持不变。
    pub async fn register<B: crate::driver::DriverBackend + Default + 'static>(
        &self,
        manifest: DriverManifest,
        frontend: Option<&'static str>,
    ) -> Result<DriverManifest, RuntimeError> {
        validate_manifest(&manifest)?;
        if self.limits.max_control_bytes == 0 {
            return Err(registration_error("driver_control_limit_invalid"));
        }
        let definition = B::definition();
        validate_definition(&manifest, &definition)?;
        if manifest.frontend.is_some() != frontend.is_some()
            || frontend.is_some_and(|source| source.is_empty() || source.len() > 2 * 1024 * 1024)
        {
            return Err(runtime_error(
                ErrorCategory::InvalidRequest,
                "driver_frontend_invalid",
            ));
        }
        let _control = self.control_gate(&manifest.id).lock_owned().await;
        let mut state = self.state.lock().await;
        self.ensure_accepting()?;
        if state.drivers.contains_key(&manifest.id) {
            return Err(runtime_error(ErrorCategory::Conflict, "driver_id_conflict"));
        }
        state.drivers.insert(
            manifest.id.clone(),
            BuiltinDriver {
                manifest: manifest.clone(),
                definition,
                frontend,
                spawn: || DriverTask::spawn(B::default()),
            },
        );
        Ok(manifest)
    }

    pub async fn loaded_manifests(&self) -> Vec<DriverManifest> {
        let state = self.state.lock().await;
        Self::sorted_manifests(state.drivers.values().map(|driver| &driver.manifest))
    }

    /// 确定性排序：先驱动名，再驱动 ID；同名不同 ID 也不会因遍历顺序抖动。
    fn sorted_manifests<'a>(
        manifests: impl Iterator<Item = &'a DriverManifest>,
    ) -> Vec<DriverManifest> {
        let mut manifests = manifests.cloned().collect::<Vec<_>>();
        manifests.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.cmp(&right.id))
        });
        manifests
    }

    /// Changes one live instance's bypass; unknown IDs return driver_instance_not_found.
    pub async fn set_bypass(&self, instance_id: &str, enabled: bool) -> Result<(), RuntimeError> {
        let mut state = self.state.lock().await;
        let approval = state
            .instance_approvals
            .get_mut(instance_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))?;
        approval.set_bypass(enabled);
        Ok(())
    }

    /// Reads bypass for one live instance; unknown IDs return NotFound.
    pub async fn bypass_enabled(&self, instance_id: &str) -> Result<bool, RuntimeError> {
        self.state
            .lock()
            .await
            .instance_approvals
            .get(instance_id)
            .map(InstanceApproval::bypass_enabled)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))
    }

    /// Replaces one live instance's rules; invalid rules and unknown IDs leave all rules intact.
    pub async fn set_approval_policy(
        &self,
        instance_id: &str,
        policy: ApprovalPolicy,
    ) -> Result<(), RuntimeError> {
        let mut state = self.state.lock().await;
        let approval = state
            .instance_approvals
            .get_mut(instance_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))?;
        approval.set_policy(policy)
    }

    /// Returns this instance's rules; unknown IDs cannot fall back to another instance.
    pub async fn approval_policy(&self, instance_id: &str) -> Result<ApprovalPolicy, RuntimeError> {
        self.state
            .lock()
            .await
            .instance_approvals
            .get(instance_id)
            .map(InstanceApproval::policy)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))
    }

    /// Publishes one prevalidated group to all still-live bindings under one lock.
    /// Missing instances have already stopped and are skipped; bypass is preserved.
    pub async fn apply_approval_policy(
        &self,
        instance_ids: Vec<String>,
        policy: crate::approval_policy::CompiledApprovalPolicy,
    ) {
        let mut state = self.state.lock().await;
        for id in instance_ids {
            if let Some(approval) = state.instance_approvals.get_mut(&id) {
                approval.set_compiled_policy(policy.clone());
            }
        }
    }

    pub async fn operation_catalog(&self, driver_id: &str) -> Vec<OperationDescriptor> {
        let state = self.state.lock().await;
        state
            .operation_catalog
            .get(driver_id)
            .into_iter()
            .flat_map(|operations| operations.values().cloned())
            .collect()
    }

    pub async fn get_instance(&self, instance_id: &str) -> Option<InstanceInfo> {
        self.state.lock().await.instances.get(instance_id).cloned()
    }

    pub async fn subscribe_events(
        &self,
        instance_id: &str,
    ) -> Result<tokio::sync::broadcast::Receiver<DriverNotification>, RuntimeError> {
        let state = self.state.lock().await;
        let instance = state
            .instances
            .get(instance_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))?;
        let task = state
            .running
            .get(&instance.driver_id)
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "driver_not_started"))?;
        Ok(task.events.subscribe())
    }

    pub async fn pending_operation(&self, instance_id: &str, proposal_id: &str) -> Option<String> {
        self.state
            .lock()
            .await
            .pending_approvals
            .iter()
            .find(|(_, pending)| {
                pending.1.as_deref() == Some(instance_id) && pending.2 == proposal_id
            })
            .map(|(operation_id, _)| operation_id.clone())
    }

    pub async fn pending_driver_operation(
        &self,
        driver_id: &str,
        proposal_id: &str,
    ) -> Option<String> {
        self.state
            .lock()
            .await
            .pending_approvals
            .iter()
            .find(|(_, pending)| {
                pending.0 == driver_id && pending.1.is_none() && pending.2 == proposal_id
            })
            .map(|(operation_id, _)| operation_id.clone())
    }

    pub async fn recent_driver_operations(
        &self,
        driver_id: &str,
        limit: usize,
    ) -> Vec<OperationRecord> {
        // 此适配器只供桌面人工入口使用，驱动级审批也需要读取当前敏感草稿。
        let state = self.state.lock().await;
        self.history
            .recent(driver_id, None, limit)
            .await
            .into_iter()
            .map(|record| state.with_approval_draft(record))
            .collect()
    }

    /// 返回编译期注册的前端源码；未知驱动返回 NotFound，不读取外部文件。
    pub async fn frontend_source(&self, driver_id: &str) -> Result<Option<String>, RuntimeError> {
        let state = self.state.lock().await;
        state
            .drivers
            .get(driver_id)
            .map(|driver| driver.frontend.map(str::to_owned))
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_not_loaded"))
    }

    /// Human-only adapter entry: sends no ordinary operation and creates no history.
    pub async fn provide_secret(
        &self,
        instance_id: &str,
        purpose: String,
        value: String,
    ) -> Result<SecretHandle, RuntimeError> {
        if purpose.is_empty() || purpose.len() > 128 || value.len() > 16 * 1024 {
            return Err(runtime_error(
                ErrorCategory::InvalidRequest,
                "invalid_secret_delivery",
            ));
        }
        let driver_id = self
            .get_instance(instance_id)
            .await
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))?
            .driver_id;
        let _control = self.control_gate(&driver_id).lock_owned().await;
        let state = self.state.lock().await;
        let instance = state
            .instances
            .get(instance_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))?;
        let driver = state
            .drivers
            .get(&instance.driver_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_not_loaded"))?;
        if !driver
            .manifest
            .capabilities
            .iter()
            .any(|capability| capability == "secret-input")
        {
            return Err(runtime_error(
                ErrorCategory::Unsupported,
                "secret_input_not_declared",
            ));
        }
        let task = state
            .running
            .get(&instance.driver_id)
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "driver_not_started"))?;
        let handle = SecretHandle {
            id: uuid::Uuid::new_v4().to_string(),
            driver_instance_id: instance_id.to_owned(),
            purpose,
            expires_at: Utc::now() + chrono::Duration::seconds(30),
        };
        let task = task.clone();
        drop(state);
        task.request(HostCommand::ProvideSecret(SecretDelivery {
            handle: handle.clone(),
            value,
        }))
        .await?;
        Ok(handle)
    }
}

impl Runtime {
    pub async fn native_mcp_tools(&self) -> Vec<(String, McpToolRegistration)> {
        let state = self.state.lock().await;
        let mut tools: Vec<_> = state
            .mcp_tools
            .iter()
            .flat_map(|(id, tools)| tools.iter().cloned().map(|t| (id.clone(), t)))
            .collect();
        tools.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.operation.cmp(&b.1.operation)));
        tools
    }

    async fn finish_reply(
        &self,
        state: &mut State,
        operation_id: &str,
        result: Result<Value, RuntimeError>,
    ) -> Result<OperationRecord, RuntimeError> {
        let record = self
            .history
            .get(operation_id)
            .await
            .map_err(map_history_error)?;
        // Shutdown can settle an operation while its control request is still returning.
        if crate::operation_history::terminal(record.status) {
            return Ok(record);
        }
        let result = match result {
            Ok(output) if output.get("_cotermDeferred").is_some() => {
                let record = self
                    .history
                    .get(operation_id)
                    .await
                    .map_err(map_history_error)?;
                let deferred = serde_json::from_value::<DeferredOperation>(output);
                if !deferred.is_ok_and(|deferred| deferred.operation_id == operation_id)
                    || !state
                        .features
                        .get(&record.driver_id)
                        .is_some_and(|f| f.iter().any(|f| f == "async-operations"))
                {
                    return self
                        .history
                        .transition(
                            operation_id,
                            OperationStatus::Unknown,
                            None,
                            Some(runtime_error(
                                ErrorCategory::Transport,
                                "invalid_deferred_operation",
                            )),
                        )
                        .await
                        .map_err(map_history_error);
                }
                // Resuming a deferred secret interaction may start an async job.
                // The operation is already Running; acknowledge that continuation
                // without attempting an invalid Running -> Running transition.
                if record.status == OperationStatus::Running {
                    return Ok(record);
                }
                return self
                    .history
                    .transition(operation_id, OperationStatus::Running, None, None)
                    .await
                    .map_err(map_history_error);
            }
            Ok(output) if output.get("_cotermInteraction").is_some() => {
                let record = self
                    .history
                    .get(operation_id)
                    .await
                    .map_err(map_history_error)?;
                let parsed = serde_json::from_value::<DeferredInteraction>(output).map_err(|_| {
                    runtime_error(ErrorCategory::Transport, "invalid_interaction_reply")
                });
                match parsed {
                    Ok(envelope) => {
                        let request = envelope.request;
                        let supported =
                            state
                                .features
                                .get(&record.driver_id)
                                .is_some_and(|features| {
                                    features.iter().any(|f| f == "deferred-interactions")
                                });
                        let allowed = state.drivers.get(&record.driver_id).is_some_and(|p| {
                            p.manifest.capabilities.iter().any(|c| c == "secret-input")
                        });
                        if !supported
                            || !allowed
                            || record.status != OperationStatus::Dispatched
                            || !matches!(&request.kind, InteractionKind::Secret { purpose } if !purpose.is_empty() && purpose.len() <= 128)
                            || request.operation_id.as_deref() != Some(operation_id)
                            || record.driver_instance_id.as_deref()
                                != Some(&request.driver_instance_id)
                            || request.interaction_id.is_empty()
                            || request.interaction_id.len() > 128
                            || request.title.len() > 256
                            || request.prompt.len() > 4096
                            || state.interactions.len() >= 32
                            || state.interactions.contains_key(&request.interaction_id)
                        {
                            Err(runtime_error(
                                ErrorCategory::Interaction,
                                "interaction_unavailable",
                            ))
                        } else {
                            let duration = std::time::Duration::from_millis(
                                request.timeout_ms.unwrap_or(60_000).clamp(1, 300_000),
                            );
                            let mut request = request;
                            request.deadline = Some(
                                Utc::now()
                                    + chrono::Duration::from_std(duration)
                                        .unwrap_or_else(|_| chrono::Duration::seconds(60)),
                            );
                            state.interactions.insert(
                                request.interaction_id.clone(),
                                (request, tokio::time::Instant::now() + duration),
                            );
                            self.bump_attention();
                            return self
                                .history
                                .transition(operation_id, OperationStatus::Running, None, None)
                                .await
                                .map_err(map_history_error);
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            result => result,
        };
        match result {
            Ok(output) => {
                let record = self
                    .history
                    .get(operation_id)
                    .await
                    .map_err(map_history_error)?;
                let schema = state
                    .operation_catalog
                    .get(&record.driver_id)
                    .and_then(|catalog| catalog.get(&record.operation))
                    .and_then(|descriptor| descriptor.output_schema.as_ref());
                if schema.is_some_and(|schema| !schema_is_valid(schema, &output)) {
                    self.history
                        .transition(
                            operation_id,
                            OperationStatus::Unknown,
                            None,
                            Some(runtime_error(
                                ErrorCategory::Transport,
                                "operation_output_invalid",
                            )),
                        )
                        .await
                        .map_err(map_history_error)
                } else {
                    self.history
                        .transition(operation_id, OperationStatus::Completed, Some(output), None)
                        .await
                        .map_err(map_history_error)
                }
            }
            Err(error) => self
                .history
                .transition(operation_id, failure_status(&error), None, Some(error))
                .await
                .map_err(map_history_error),
        }
    }

    pub async fn pending_interactions(&self) -> Vec<InteractionRequest> {
        self.state
            .lock()
            .await
            .interactions
            .values()
            .map(|(r, _)| r.clone())
            .collect()
    }

    /// Trusted human adapter only. Raw values travel exclusively via provide_secret.
    pub async fn respond_secret(
        &self,
        interaction_id: &str,
        value: Option<String>,
    ) -> Result<(), RuntimeError> {
        let mut state = self.state.lock().await;
        let (request, deadline) = state
            .interactions
            .get(interaction_id)
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "interaction_not_found"))?;
        let outcome = if deadline <= tokio::time::Instant::now() {
            InteractionOutcome::TimedOut
        } else if let Some(value) = value {
            let purpose = match &request.kind {
                InteractionKind::Secret { purpose } => purpose.clone(),
                _ => {
                    return Err(runtime_error(
                        ErrorCategory::Unsupported,
                        "secret_interaction_required",
                    ))
                }
            };
            // Drop state while using the ordinary private delivery path. The response
            // is claimed first, so competing/late submissions cannot resume twice.
            state.interactions.remove(interaction_id);
            self.bump_attention();
            drop(state);
            let delivery = self
                .provide_secret(&request.driver_instance_id, purpose, value)
                .await;
            state = self.state.lock().await;
            match delivery {
                Ok(_) if deadline <= tokio::time::Instant::now() => InteractionOutcome::TimedOut,
                Ok(handle) => InteractionOutcome::Submitted(InteractionValue::Secret(handle)),
                Err(error) => {
                    if let Some(id) = &request.operation_id {
                        let _ = self
                            .history
                            .transition(id, failure_status(&error), None, Some(error.clone()))
                            .await;
                    }
                    return Err(error);
                }
            }
        } else {
            InteractionOutcome::Cancelled
        };
        if state.interactions.remove(interaction_id).is_some() {
            self.bump_attention();
        }
        drop(state);
        let operation_id = request.operation_id.as_deref().ok_or_else(|| {
            runtime_error(
                ErrorCategory::InvalidRequest,
                "interaction_operation_required",
            )
        })?;
        let _control = self.operation_control(operation_id).await?;
        self.resume_interaction(request, outcome).await
    }

    async fn resume_interaction(
        &self,
        request: InteractionRequest,
        outcome: InteractionOutcome,
    ) -> Result<(), RuntimeError> {
        let operation_id = request.operation_id.clone().ok_or_else(|| {
            runtime_error(
                ErrorCategory::InvalidRequest,
                "interaction_operation_required",
            )
        })?;
        let record = self
            .history
            .get(&operation_id)
            .await
            .map_err(map_history_error)?;
        let state = self.state.lock().await;
        // Instance destruction wins over a late credential delivery.
        if record.status != OperationStatus::Running
            || !state.instances.contains_key(&request.driver_instance_id)
        {
            return Err(runtime_error(
                ErrorCategory::Conflict,
                "interaction_no_longer_active",
            ));
        }
        let cancelled = !matches!(outcome, InteractionOutcome::Submitted(_));
        let task = state
            .running
            .get(&record.driver_id)
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "driver_not_started"))?;
        drop(state);
        let result = task
            .request(HostCommand::InteractionResult(InteractionResult {
                interaction_id: request.interaction_id,
                driver_instance_id: request.driver_instance_id,
                outcome,
            }))
            .await;
        if cancelled {
            self.history
                .transition(&operation_id, OperationStatus::Cancelled, None, None)
                .await
                .map_err(map_history_error)?;
        } else {
            let mut state = self.state.lock().await;
            self.finish_reply(&mut state, &operation_id, result).await?;
        }
        Ok(())
    }

    pub async fn expire_interactions(&self) {
        let mut state = self.state.lock().await;
        let ids: Vec<_> = state
            .interactions
            .iter()
            .filter(|(_, (_, deadline))| *deadline <= tokio::time::Instant::now())
            .map(|(id, _)| id.clone())
            .collect();
        let mut expired = Vec::new();
        for id in ids {
            if let Some((request, _)) = state.interactions.remove(&id) {
                self.bump_attention();
                expired.push(request);
            }
        }
        drop(state);
        for request in expired {
            if let Some(id) = request.operation_id.as_deref() {
                if let Ok(_control) = self.operation_control(id).await {
                    let _ = self
                        .resume_interaction(request, InteractionOutcome::TimedOut)
                        .await;
                }
            }
        }
    }

    async fn cancel_instance_interactions(&self, state: &mut State, instances: &[String]) {
        // 无论是否命中，实例销毁/驱动停止都会改变宿主待处理视图，统一推高版本。
        self.bump_attention();
        state.pending_approvals.retain(|_, (_, instance, _, _)| {
            !instance.as_ref().is_some_and(|id| instances.contains(id))
        });
        state.prune_approval_drafts();
        for instance_id in instances {
            if let Some(instance) = state.instances.get(instance_id) {
                for record in self.history.active(&instance.driver_id, instance_id).await {
                    if matches!(
                        record.status,
                        OperationStatus::Accepted
                            | OperationStatus::AwaitingApproval
                            | OperationStatus::Dispatched
                            | OperationStatus::Running
                    ) {
                        let _ = self
                            .history
                            .transition(
                                &record.operation_id,
                                if matches!(
                                    record.status,
                                    OperationStatus::Dispatched | OperationStatus::Running
                                ) {
                                    OperationStatus::Unknown
                                } else {
                                    OperationStatus::Cancelled
                                },
                                None,
                                None,
                            )
                            .await;
                    }
                }
            }
        }
        let ids: Vec<_> = state
            .interactions
            .iter()
            .filter(|(_, (r, _))| instances.contains(&r.driver_instance_id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some((request, _)) = state.interactions.remove(&id) {
                if let Some(operation_id) = request.operation_id {
                    let _ = self
                        .history
                        .transition(&operation_id, OperationStatus::Cancelled, None, None)
                        .await;
                }
            }
        }
        // 交互清理与实例移除在同一把锁内完成，快照不会观察到半更新状态。
    }
}

/// State convergence runs under the manager lock and includes driver-scoped operations.
async fn settle_closed_driver(state: &mut State, history: &OperationHistory, driver_id: &str) {
    state.operation_catalog.remove(driver_id);
    state.mcp_tools.remove(driver_id);
    state.features.remove(driver_id);
    let ids = state
        .instances
        .values_mut()
        .filter(|instance| instance.driver_id == driver_id)
        .map(|instance| {
            instance.status = InstanceStatus::Failed;
            instance.driver_instance_id.clone()
        })
        .collect::<HashSet<_>>();
    state
        .pending_approvals
        .retain(|_, pending| pending.0 != driver_id);
    state.prune_approval_drafts();
    state
        .interactions
        .retain(|_, (request, _)| !ids.contains(&request.driver_instance_id));
    for record in history.active_driver(driver_id).await {
        let status = if matches!(
            record.status,
            OperationStatus::Dispatched | OperationStatus::Running
        ) {
            OperationStatus::Unknown
        } else {
            OperationStatus::Cancelled
        };
        let _ = history
            .transition(
                &record.operation_id,
                status,
                None,
                Some(runtime_error(
                    ErrorCategory::Transport,
                    "driver_connection_closed",
                )),
            )
            .await;
    }
}

impl Runtime {
    async fn start_inner(&self, driver_id: &str) -> Result<(), RuntimeError> {
        let _control = self.control_gate(driver_id).lock_owned().await;
        let state = self.state.lock().await;
        if self
            .shutting_down
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(runtime_error(
                ErrorCategory::Conflict,
                "runtime_shutting_down",
            ));
        }
        if state.running.contains_key(driver_id) {
            return Ok(());
        }
        let driver = state
            .drivers
            .get(driver_id)
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_not_loaded"))?;
        drop(state);
        let task = Arc::new((driver.spawn)());
        self.tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(driver_id.to_owned(), Arc::downgrade(&task));
        if self
            .shutting_down
            .load(std::sync::atomic::Ordering::Acquire)
        {
            let _ = task.close().await;
            return Err(runtime_error(
                ErrorCategory::Conflict,
                "runtime_shutting_down",
            ));
        }
        let result = driver.definition;
        task.set_control_limit(
            self.limits
                .max_control_bytes
                .min(crate::driver::MAX_CONTROL_BYTES as u32),
        );
        let catalog = result
            .operations
            .into_iter()
            .map(|d| (d.name.clone(), d))
            .collect();
        let notifications = task.take_completions().await;
        let mut state = self.state.lock().await;
        if let Err(error) = self.ensure_accepting() {
            drop(state);
            let _ = task.close().await;
            return Err(error);
        }
        state.features.insert(driver_id.to_owned(), result.features);
        state
            .mcp_tools
            .insert(driver_id.to_owned(), result.mcp_tools);
        state
            .operation_catalog
            .insert(driver_id.to_owned(), catalog);
        let task_ref = Arc::downgrade(&task);
        state.running.insert(driver_id.to_owned(), task);
        if let Some(mut notifications) = notifications {
            let control_gate = self.control_gate(driver_id);
            let state_ref = Arc::downgrade(&self.state);
            let history_ref = Arc::downgrade(&self.history);
            let attention_ref = Arc::downgrade(&self.attention);
            let driver_id = driver_id.to_owned();
            tokio::spawn(async move {
                while let Some(notification) = notifications.recv().await {
                    let (Some(state), Some(history)) = (state_ref.upgrade(), history_ref.upgrade())
                    else {
                        break;
                    };
                    let _control = control_gate.lock().await;
                    let _state = state.lock().await;
                    if !task_ref.upgrade().is_some_and(|task| {
                        _state
                            .running
                            .get(&driver_id)
                            .is_some_and(|current| Arc::ptr_eq(current, &task))
                    }) {
                        continue;
                    }
                    let (id, result) = match notification {
                        DriverNotification::OperationCompleted {
                            operation_id,
                            result,
                        } => (operation_id, Ok(result)),
                        DriverNotification::OperationFailed {
                            operation_id,
                            error,
                        } => (operation_id, Err(error)),
                        _ => continue,
                    };
                    let Ok(record) = history.get(&id).await else {
                        continue;
                    };
                    if record.driver_id != driver_id
                        || !matches!(
                            record.status,
                            OperationStatus::Running | OperationStatus::Dispatched
                        )
                    {
                        continue;
                    }
                    let (status, output, error) = match result {
                        Ok(result) => {
                            let schema = _state
                                .operation_catalog
                                .get(&driver_id)
                                .and_then(|catalog| catalog.get(&record.operation))
                                .and_then(|descriptor| descriptor.output_schema.as_ref());
                            if schema.is_some_and(|schema| !schema_is_valid(schema, &result)) {
                                (
                                    OperationStatus::Unknown,
                                    None,
                                    Some(runtime_error(
                                        ErrorCategory::Transport,
                                        "operation_output_invalid",
                                    )),
                                )
                            } else {
                                (OperationStatus::Completed, Some(result), None)
                            }
                        }
                        Err(error) => (OperationStatus::Unknown, None, Some(error)),
                    };
                    let _ = history.transition(&id, status, output, error).await;
                }
                if let (Some(state), Some(history)) = (state_ref.upgrade(), history_ref.upgrade()) {
                    let _control = control_gate.lock().await;
                    let mut state = state.lock().await;
                    if !state.running.get(&driver_id).is_some_and(|current| {
                        task_ref
                            .upgrade()
                            .is_some_and(|task| Arc::ptr_eq(current, &task))
                    }) {
                        return;
                    }
                    let task = state.running.remove(&driver_id);
                    settle_closed_driver(&mut state, &history, &driver_id).await;
                    if let Some(attention) = attention_ref.upgrade() {
                        *attention.lock().unwrap_or_else(|error| error.into_inner()) += 1;
                    }
                    drop(state);
                    if let Some(task) = task {
                        let _ = task.close().await;
                    }
                }
            });
        }
        Ok(())
    }
}

impl Runtime {
    pub async fn start(&self, driver_id: &str) -> Result<(), RuntimeError> {
        self.start_inner(driver_id).await
    }

    pub async fn stop(&self, driver_id: &str) -> Result<(), RuntimeError> {
        let _control = self.control_gate(driver_id).lock_owned().await;
        let mut state = self.state.lock().await;
        let Some(task) = state.running.remove(driver_id) else {
            return Ok(());
        };
        state.operation_catalog.remove(driver_id);
        state.mcp_tools.remove(driver_id);
        state.features.remove(driver_id);
        let instance_ids = state
            .instances
            .values()
            .filter(|instance| instance.driver_id == driver_id)
            .map(|instance| instance.driver_instance_id.clone())
            .collect::<Vec<_>>();
        let pending_ids = state
            .pending_approvals
            .iter()
            .filter(|(_, pending)| pending.0 == driver_id)
            .map(|(operation_id, _)| operation_id.clone())
            .collect::<Vec<_>>();
        for operation_id in pending_ids {
            state.pending_approvals.remove(&operation_id);
            state.approval_drafts.remove(&operation_id);
            let _ = self
                .history
                .transition(&operation_id, OperationStatus::Cancelled, None, None)
                .await;
        }
        self.bump_attention();
        self.cancel_instance_interactions(&mut state, &instance_ids)
            .await;
        settle_closed_driver(&mut state, &self.history, driver_id).await;
        for instance_id in &instance_ids {
            state.instances.remove(instance_id);
            state.instance_approvals.remove(instance_id);
        }
        drop(state);
        // One deadline for the entire driver, including many instances.
        let graceful = async {
            for instance_id in &instance_ids {
                task.request(HostCommand::DestroyInstance {
                    driver_instance_id: instance_id.clone(),
                })
                .await?;
            }
            task.request(HostCommand::Shutdown).await?;
            Ok::<(), RuntimeError>(())
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), graceful).await;
        task.close().await
    }

    pub async fn create_instance(
        &self,
        driver_id: &str,
        config: Value,
    ) -> Result<InstanceInfo, RuntimeError> {
        self.create_instance_with_name(driver_id, config, None)
            .await
    }

    pub async fn destroy_instance(&self, instance_id: &str) -> Result<(), RuntimeError> {
        let driver_id = self
            .get_instance(instance_id)
            .await
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))?
            .driver_id;
        let _control = self.control_gate(&driver_id).lock_owned().await;
        let mut state = self.state.lock().await;
        let instance =
            state.instances.get(instance_id).cloned().ok_or_else(|| {
                runtime_error(ErrorCategory::NotFound, "driver_instance_not_found")
            })?;
        if instance.status != InstanceStatus::Failed {
            if let Some(task) = state.running.get(&instance.driver_id).cloned() {
                drop(state);
                task.request(HostCommand::DestroyInstance {
                    driver_instance_id: instance_id.to_owned(),
                })
                .await?;
                state = self.state.lock().await;
            }
        }
        self.cancel_instance_interactions(&mut state, &[instance_id.to_owned()])
            .await;
        state.instances.remove(instance_id);
        state.instance_approvals.remove(instance_id);
        Ok(())
    }

    pub async fn list_instances(
        &self,
        driver_id: Option<&str>,
    ) -> Result<Vec<InstanceInfo>, RuntimeError> {
        let state = self.state.lock().await;
        let mut instances = state
            .instances
            .values()
            .filter(|instance| driver_id.is_none_or(|id| instance.driver_id == id))
            .cloned()
            .collect::<Vec<_>>();
        instances.sort_by(|left, right| {
            left.driver_id
                .cmp(&right.driver_id)
                .then(left.driver_instance_id.cmp(&right.driver_instance_id))
        });
        Ok(instances)
    }
}

impl Runtime {
    /// MCP adapters may return once a validated proposal is ready for human review.
    /// Other callers retain the normal wait-until-completed behavior.
    pub async fn submit_until_approval(
        &self,
        caller: CallerContext,
        request: SubmitOperation,
    ) -> Result<OperationReceipt, RuntimeError> {
        self.submit_with_wait_policy(caller, request, true).await
    }

    async fn submit_with_wait_policy(
        &self,
        caller: CallerContext,
        request: SubmitOperation,
        return_on_approval: bool,
    ) -> Result<OperationReceipt, RuntimeError> {
        let state = self.state.lock().await;
        self.ensure_accepting()?;
        let descriptor = state
            .operation_catalog
            .get(&request.driver_id)
            .and_then(|catalog| catalog.get(&request.operation))
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "operation_not_found"))?;
        validate_scope_target(descriptor.scope, request.driver_instance_id.is_some())?;
        if descriptor.scope == OperationScope::Driver {
            validate_driver_authority(&caller, &request.driver_id)?;
        }
        if let Some(instance_id) = request.driver_instance_id.as_deref() {
            let instance = state.instances.get(instance_id).ok_or_else(|| {
                runtime_error(ErrorCategory::NotFound, "driver_instance_not_found")
            })?;
            if instance.driver_id != request.driver_id {
                return Err(runtime_error(
                    ErrorCategory::Conflict,
                    "driver_instance_mismatch",
                ));
            }
            if instance.session_id != caller.session_id {
                return Err(runtime_error(ErrorCategory::Conflict, "session_mismatch"));
            }
            if instance.status != InstanceStatus::Ready {
                return Err(runtime_error(
                    ErrorCategory::Conflict,
                    "driver_instance_not_ready",
                ));
            }
        }
        validate_operation_args(
            &state,
            &request.driver_id,
            &request.operation,
            &request.args,
        )?;
        if !state.running.contains_key(&request.driver_id) {
            return Err(runtime_error(ErrorCategory::Conflict, "driver_not_started"));
        }
        let operation_id = uuid::Uuid::new_v4().to_string();
        let context = ApprovalContext {
            source: caller.source,
            descriptor: &descriptor,
            args: &request.args,
        };
        let default_approval = InstanceApproval::default();
        let approval = request
            .driver_instance_id
            .as_deref()
            .and_then(|id| state.instance_approvals.get(id))
            .unwrap_or(&default_approval);
        let proposal_id = approval
            .requires_approval(&context)
            .then(|| uuid::Uuid::new_v4().to_string());
        let now = Utc::now();
        let record = OperationRecord {
            operation_id: operation_id.clone(),
            driver_id: request.driver_id.clone(),
            driver_instance_id: request.driver_instance_id.clone(),
            source: caller.source,
            effect: descriptor.effect,
            operation: request.operation.clone(),
            status: OperationStatus::Accepted,
            input: if descriptor.sensitive_input {
                serde_json::json!({"redacted":true})
            } else {
                request.args.clone()
            },
            output: None,
            error: None,
            created_at: now,
            updated_at: now,
            proposal_id: proposal_id.clone(),
            wait_expired: false,
            transient: descriptor.transient && proposal_id.is_none(),
        };
        self.history
            .begin(record.clone())
            .await
            .map_err(map_history_error)?;
        let mut updates = self
            .history
            .subscribe(&operation_id)
            .await
            .map_err(map_history_error)?;
        drop(state);
        let wait_ms = request.wait_timeout_ms.unwrap_or(60_000).min(300_000);
        let runtime = self.clone();
        tokio::spawn(async move {
            let _control = runtime.control_gate(&request.driver_id).lock_owned().await;
            let state = runtime.state.lock().await;
            if runtime.ensure_accepting().is_err() {
                let _ = runtime
                    .history
                    .transition(&operation_id, OperationStatus::Cancelled, None, None)
                    .await;
                return;
            }
            if !runtime
                .history
                .get(&operation_id)
                .await
                .is_ok_and(|r| r.status == OperationStatus::Accepted)
            {
                return;
            }
            if request
                .driver_instance_id
                .as_ref()
                .is_some_and(|id| !state.instances.contains_key(id))
            {
                let _ = runtime
                    .history
                    .transition(&operation_id, OperationStatus::Cancelled, None, None)
                    .await;
                return;
            }
            let Some(task) = state.running.get(&request.driver_id).cloned() else {
                let _ = runtime
                    .history
                    .transition(
                        &operation_id,
                        OperationStatus::Failed,
                        None,
                        Some(runtime_error(ErrorCategory::Conflict, "driver_not_started")),
                    )
                    .await;
                return;
            };
            let dispatch = proposal_id.as_ref().map_or(DispatchMode::Execute, |id| {
                DispatchMode::PrepareProposal {
                    proposal_id: id.clone(),
                }
            });
            if proposal_id.is_none() {
                let _ = runtime
                    .history
                    .transition(&operation_id, OperationStatus::Dispatched, None, None)
                    .await;
            }
            let mut driver_id = request.driver_id;
            let mut driver_instance_id = request.driver_instance_id;
            let operation_name = request.operation.clone();
            drop(state);
            let result = task
                .request(HostCommand::Invoke(InvokeRequest {
                    operation_id: operation_id.clone(),
                    driver_instance_id: driver_instance_id.clone(),
                    source: caller.source,
                    effect: descriptor.effect,
                    operation: request.operation,
                    args: request.args,
                    dispatch,
                }))
                .await;
            let mut state = runtime.state.lock().await;
            if ensure_current_task(&state, &driver_id, &task).is_err()
                || runtime.ensure_accepting().is_err()
            {
                return;
            }
            if let Some(proposal_id) = proposal_id {
                if !runtime
                    .history
                    .get(&operation_id)
                    .await
                    .is_ok_and(|record| record.status == OperationStatus::Accepted)
                {
                    return;
                }
                match result {
                    Ok(draft) => {
                        let draft = if descriptor.sensitive_input {
                            state.approval_drafts.insert(operation_id.clone(), draft);
                            serde_json::json!({"redacted":true})
                        } else {
                            draft
                        };
                        state.pending_approvals.insert(
                            operation_id.clone(),
                            (
                                std::mem::take(&mut driver_id),
                                driver_instance_id.take(),
                                proposal_id,
                                operation_name,
                            ),
                        );
                        runtime.bump_attention();
                        let _ = runtime
                            .history
                            .transition(
                                &operation_id,
                                OperationStatus::AwaitingApproval,
                                Some(draft),
                                None,
                            )
                            .await;
                    }
                    Err(error) => {
                        let _ = runtime
                            .history
                            .transition(&operation_id, OperationStatus::Failed, None, Some(error))
                            .await;
                    }
                }
            } else if let Err(error) = runtime
                .finish_reply(&mut state, &operation_id, result)
                .await
            {
                let _ = runtime
                    .history
                    .transition(&operation_id, OperationStatus::Failed, None, Some(error))
                    .await;
            }
        });
        if wait_ms == 0 {
            return Ok(receipt(&record));
        }
        let final_result = tokio::time::timeout(std::time::Duration::from_millis(wait_ms), async {
            loop {
                let record = updates.borrow_and_update().clone();
                if crate::operation_history::terminal(record.status)
                    || (return_on_approval && record.status == OperationStatus::AwaitingApproval)
                {
                    return record;
                }
                if updates.changed().await.is_err() {
                    return updates.borrow().clone();
                }
            }
        })
        .await;
        match final_result {
            Ok(record) => Ok(receipt(&record)),
            Err(_) => {
                let mut result = receipt(&updates.borrow().clone());
                result.wait_expired = true;
                Ok(result)
            }
        }
    }
}

impl Runtime {
    pub async fn submit(
        &self,
        caller: CallerContext,
        request: SubmitOperation,
    ) -> Result<OperationReceipt, RuntimeError> {
        self.submit_with_wait_policy(caller, request, false).await
    }

    pub async fn get(
        &self,
        caller: CallerContext,
        operation_id: &str,
    ) -> Result<OperationRecord, RuntimeError> {
        let record = self
            .history
            .get(operation_id)
            .await
            .map_err(map_history_error)?;
        verify_record_access(&self.state, &caller, &record).await?;
        if caller.source == OperationSource::Human {
            Ok(self.state.lock().await.with_approval_draft(record))
        } else {
            Ok(record)
        }
    }

    pub async fn recent(
        &self,
        caller: CallerContext,
        instance_id: &str,
        limit: usize,
    ) -> Result<Vec<OperationRecord>, RuntimeError> {
        let state = self.state.lock().await;
        let instance = state
            .instances
            .get(instance_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))?;
        if instance.session_id != caller.session_id {
            return Err(runtime_error(ErrorCategory::Conflict, "session_mismatch"));
        }
        Ok(self
            .history
            .recent(&instance.driver_id, Some(instance_id), limit)
            .await)
    }

    pub async fn approve(
        &self,
        caller: CallerContext,
        request: ApproveProposal,
    ) -> Result<OperationReceipt, RuntimeError> {
        if caller.source != OperationSource::Human {
            return Err(runtime_error(
                ErrorCategory::Conflict,
                "human_approval_required",
            ));
        }
        let _control = self.operation_control(&request.operation_id).await?;
        let mut state = self.state.lock().await;
        self.ensure_accepting()?;
        let pending = state
            .pending_approvals
            .get(&request.operation_id)
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "proposal_not_found"))?;
        if pending.2 != request.proposal_id || pending.1 != request.driver_instance_id {
            return Err(runtime_error(ErrorCategory::Conflict, "proposal_mismatch"));
        }
        if let Some(instance_id) = pending.1.as_deref() {
            if state
                .instances
                .get(instance_id)
                .is_none_or(|instance| instance.session_id != caller.session_id)
            {
                return Err(runtime_error(ErrorCategory::Conflict, "session_mismatch"));
            }
        } else {
            validate_driver_authority(&caller, &pending.0)?;
        }
        let record = self
            .history
            .get(&request.operation_id)
            .await
            .map_err(map_history_error)?;
        validate_operation_args(&state, &pending.0, &record.operation, &request.args)?;
        let task = state
            .running
            .get(&pending.0)
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "driver_not_started"))?;
        self.history
            .transition(
                &request.operation_id,
                OperationStatus::Dispatched,
                None,
                None,
            )
            .await
            .map_err(map_history_error)?;
        let sensitive = state
            .operation_catalog
            .get(&pending.0)
            .and_then(|catalog| catalog.get(&record.operation))
            .is_some_and(|descriptor| descriptor.sensitive_input);
        self.history
            .set_input(
                &request.operation_id,
                if sensitive {
                    serde_json::json!({"redacted": true})
                } else {
                    request.args.clone()
                },
            )
            .await
            .map_err(map_history_error)?;
        state.approval_drafts.remove(&request.operation_id);
        if state
            .pending_approvals
            .remove(&request.operation_id)
            .is_some()
        {
            self.bump_attention();
        }
        drop(state);
        let result = task.request(HostCommand::Approve(request.clone())).await;
        let mut state = self.state.lock().await;
        let record = self
            .finish_reply(&mut state, &request.operation_id, result)
            .await?;
        Ok(receipt(&record))
    }

    pub async fn reject(
        &self,
        caller: CallerContext,
        operation_id: &str,
        proposal_id: &str,
    ) -> Result<(), RuntimeError> {
        if caller.source != OperationSource::Human {
            return Err(runtime_error(
                ErrorCategory::Conflict,
                "human_rejection_required",
            ));
        }
        let _control = self.operation_control(operation_id).await?;
        let state = self.state.lock().await;
        let pending = state
            .pending_approvals
            .get(operation_id)
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "proposal_not_found"))?;
        if pending.2 != proposal_id {
            return Err(runtime_error(ErrorCategory::Conflict, "proposal_mismatch"));
        }
        if let Some(instance_id) = pending.1.as_deref() {
            if state
                .instances
                .get(instance_id)
                .is_none_or(|instance| instance.session_id != caller.session_id)
            {
                return Err(runtime_error(ErrorCategory::Conflict, "session_mismatch"));
            }
        } else {
            validate_driver_authority(&caller, &pending.0)?;
        }
        let task = state
            .running
            .get(&pending.0)
            .cloned()
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "driver_not_started"))?;
        drop(state);
        task.request(HostCommand::Reject {
            operation_id: operation_id.to_owned(),
            proposal_id: proposal_id.to_owned(),
            reason: None,
        })
        .await?;
        let mut state = self.state.lock().await;
        state.pending_approvals.remove(operation_id);
        state.approval_drafts.remove(operation_id);
        self.bump_attention();
        self.history
            .transition(operation_id, OperationStatus::Rejected, None, None)
            .await
            .map_err(map_history_error)?;
        Ok(())
    }

    pub async fn cancel(
        &self,
        caller: CallerContext,
        operation_id: &str,
    ) -> Result<OperationReceipt, RuntimeError> {
        let record = self.get(caller, operation_id).await?;
        let _control = self.control_gate(&record.driver_id).lock_owned().await;
        let mut state = self.state.lock().await;
        let record = self
            .history
            .get(&record.operation_id)
            .await
            .map_err(map_history_error)?;
        if record.status == OperationStatus::Accepted {
            self.history
                .transition(operation_id, OperationStatus::Cancelled, None, None)
                .await
                .map_err(map_history_error)?;
        } else if let Some(id) = state
            .interactions
            .iter()
            .find(|(_, (r, _))| r.operation_id.as_deref() == Some(operation_id))
            .map(|(id, _)| id.clone())
        {
            if let Some((request, _)) = state.interactions.remove(&id) {
                self.bump_attention();
                drop(state);
                self.resume_interaction(request, InteractionOutcome::Cancelled)
                    .await?;
            }
        } else if let Some((driver_id, _, proposal_id, _)) =
            state.pending_approvals.get(operation_id).cloned()
        {
            let task = state
                .running
                .get(&driver_id)
                .cloned()
                .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "driver_not_started"))?;
            drop(state);
            let result = task
                .request(HostCommand::Reject {
                    operation_id: operation_id.to_owned(),
                    proposal_id,
                    reason: Some("Cancelled by caller".to_owned()),
                })
                .await;
            let mut state = self.state.lock().await;
            state.pending_approvals.remove(operation_id);
            state.approval_drafts.remove(operation_id);
            self.bump_attention();
            let (status, error) = match result {
                Ok(_) => (OperationStatus::Cancelled, None),
                Err(error) => (failure_status(&error), Some(error)),
            };
            self.history
                .transition(operation_id, status, None, error)
                .await
                .map_err(map_history_error)?;
        } else if matches!(
            record.status,
            OperationStatus::Accepted | OperationStatus::Dispatched | OperationStatus::Running
        ) {
            let task = state
                .running
                .get(&record.driver_id)
                .cloned()
                .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "driver_not_started"))?;
            drop(state);
            let response = task
                .request(HostCommand::Cancel(crate::driver::CancelTarget::Operation(
                    operation_id.to_owned(),
                )))
                .await?;
            let outcome: CancellationOutcome =
                serde_json::from_value(response.get("outcome").cloned().unwrap_or(Value::Null))
                    .map_err(|_| {
                        runtime_error(ErrorCategory::Transport, "cancel_result_invalid")
                    })?;
            let (status, error) = match outcome {
                CancellationOutcome::NotSubmitted | CancellationOutcome::Stopped => {
                    (OperationStatus::Cancelled, None)
                }
                CancellationOutcome::Unknown => (
                    OperationStatus::Unknown,
                    Some(runtime_error(
                        ErrorCategory::Transport,
                        "operation_cancellation_unconfirmed",
                    )),
                ),
                CancellationOutcome::Unsupported => {
                    return Err(runtime_error(
                        ErrorCategory::Unsupported,
                        "operation_cancellation_unsupported",
                    ))
                }
            };
            self.history
                .transition(operation_id, status, None, error)
                .await
                .map_err(map_history_error)?;
        }
        Ok(receipt(
            &self
                .history
                .get(operation_id)
                .await
                .map_err(map_history_error)?,
        ))
    }
}

fn ensure_current_task(
    state: &State,
    driver_id: &str,
    task: &Arc<DriverTask>,
) -> Result<(), RuntimeError> {
    if state
        .running
        .get(driver_id)
        .is_some_and(|current| Arc::ptr_eq(current, task))
    {
        Ok(())
    } else {
        Err(runtime_error(
            ErrorCategory::Conflict,
            "driver_task_changed",
        ))
    }
}

fn receipt(record: &OperationRecord) -> OperationReceipt {
    OperationReceipt {
        operation_id: record.operation_id.clone(),
        proposal_id: record.proposal_id.clone(),
        status: record.status,
        wait_expired: record.wait_expired,
        output: record.output.clone(),
        error: record.error.clone(),
    }
}

fn map_history_error(error: OperationHistoryError) -> RuntimeError {
    let category = match error {
        OperationHistoryError::NotFound => ErrorCategory::NotFound,
        OperationHistoryError::Duplicate
        | OperationHistoryError::InvalidTransition
        | OperationHistoryError::PendingCapacity => ErrorCategory::Conflict,
    };
    runtime_error(category, &error.to_string().replace(' ', "_"))
}

async fn verify_record_access(
    state: &Mutex<State>,
    caller: &CallerContext,
    record: &OperationRecord,
) -> Result<(), RuntimeError> {
    let Some(instance_id) = record.driver_instance_id.as_deref() else {
        return if caller.driver_id.as_deref() == Some(record.driver_id.as_str()) {
            Ok(())
        } else {
            Err(runtime_error(
                ErrorCategory::Conflict,
                "driver_scope_access_denied",
            ))
        };
    };
    let state = state.lock().await;
    let instance = state
        .instances
        .get(instance_id)
        .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "driver_instance_not_found"))?;
    if instance.session_id != caller.session_id {
        return Err(runtime_error(ErrorCategory::Conflict, "session_mismatch"));
    }
    Ok(())
}

fn validate_driver_authority(caller: &CallerContext, driver_id: &str) -> Result<(), RuntimeError> {
    if caller.driver_id.as_deref() == Some(driver_id) {
        Ok(())
    } else {
        Err(runtime_error(
            ErrorCategory::Conflict,
            "driver_scope_access_denied",
        ))
    }
}

fn validate_scope_target(scope: OperationScope, has_instance: bool) -> Result<(), RuntimeError> {
    match scope {
        OperationScope::Instance if !has_instance => Err(runtime_error(
            ErrorCategory::InvalidRequest,
            "driver_instance_required",
        )),
        OperationScope::Driver if has_instance => Err(runtime_error(
            ErrorCategory::InvalidRequest,
            "driver_operation_does_not_accept_instance",
        )),
        _ => Ok(()),
    }
}

fn validate_operation_args(
    state: &State,
    driver_id: &str,
    operation: &str,
    args: &Value,
) -> Result<(), RuntimeError> {
    let schema = state
        .mcp_tools
        .get(driver_id)
        .and_then(|tools| tools.iter().find(|tool| tool.operation == operation))
        .and_then(|tool| tool.descriptor.get("inputSchema"))
        .or_else(|| {
            state
                .operation_catalog
                .get(driver_id)
                .and_then(|catalog| catalog.get(operation))
                .map(|descriptor| &descriptor.input_schema)
        })
        .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "operation_not_found"))?;
    validate_schema(schema, args).map_err(|_| {
        runtime_error_with(
            ErrorCategory::InvalidRequest,
            "operation_args_invalid",
            operation_args_error(schema, args),
        )
    })
}

fn operation_args_error(schema: &Value, args: &Value) -> String {
    let Some(arguments) = args.as_object() else {
        return "Operation arguments must be an object".to_owned();
    };
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return "Operation arguments do not match the registered schema".to_owned();
    };

    let mut unknown = if schema["additionalProperties"] == false {
        arguments
            .keys()
            .filter(|name| !properties.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    unknown.sort();

    let mut missing = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|name| !arguments.contains_key(*name))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    missing.sort();

    let format_fields = |fields: &[String]| {
        fields
            .iter()
            .map(|field| format!("{field:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (unknown.is_empty(), missing.is_empty()) {
        (false, false) => format!(
            "Unknown field(s): {}. Missing required field(s): {}.",
            format_fields(&unknown),
            format_fields(&missing)
        ),
        (false, true) => format!("Unknown field(s): {}.", format_fields(&unknown)),
        (true, false) => format!("Missing required field(s): {}.", format_fields(&missing)),
        (true, true) => "Operation arguments do not match the registered schema".to_owned(),
    }
}

fn schema_is_valid(schema: &Value, value: &Value) -> bool {
    jsonschema::validator_for(schema).is_ok_and(|validator| validator.is_valid(value))
}

fn validate_schema(schema: &Value, value: &Value) -> Result<(), ()> {
    let validator = jsonschema::validator_for(schema).map_err(|_| ())?;
    validator.validate(value).map_err(|_| ())
}

fn failure_status(error: &RuntimeError) -> OperationStatus {
    if error.category == ErrorCategory::Transport {
        OperationStatus::Unknown
    } else {
        OperationStatus::Failed
    }
}

fn runtime_error(category: ErrorCategory, code: &str) -> RuntimeError {
    runtime_error_with(category, code, code.replace('_', " "))
}

fn unique_instance_label<'a>(requested: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let existing = existing.collect::<std::collections::HashSet<_>>();
    if !existing.contains(requested) {
        return requested.to_owned();
    }
    for suffix in 2u64.. {
        let suffix = format!(" ({suffix})");
        let max_prefix_bytes = 256usize.saturating_sub(suffix.len());
        let mut prefix = String::new();
        for character in requested.chars() {
            if prefix.len() + character.len_utf8() > max_prefix_bytes {
                break;
            }
            prefix.push(character);
        }
        let candidate = format!("{prefix}{suffix}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    requested.to_owned()
}

fn runtime_error_with(category: ErrorCategory, code: &str, message: String) -> RuntimeError {
    RuntimeError {
        category,
        code: code.to_owned(),
        message,
        details: None,
    }
}

pub(crate) fn validate_manifest(manifest: &DriverManifest) -> Result<(), RuntimeError> {
    if manifest.manifest_version != 1 {
        return Err(registration_error("driver_manifest_version_unsupported"));
    }
    if !is_valid_driver_id(&manifest.id)
        || manifest.name.trim().is_empty()
        || manifest.version.trim().is_empty()
    {
        return Err(registration_error("driver_manifest_identity_invalid"));
    }
    if manifest.runtime.interface_major != crate::driver::INTERFACE_MAJOR {
        return Err(registration_error("driver_runtime_version_unsupported"));
    }
    if manifest.runtime.requires.iter().any(|feature| {
        !crate::driver::REQUIRED_FEATURES.contains(&feature.as_str())
            && !crate::driver::OPTIONAL_FEATURES.contains(&feature.as_str())
    }) {
        return Err(registration_error("driver_required_feature_unsupported"));
    }
    if manifest.instances.instance_type.trim().is_empty() || !manifest.instances.multiple {
        return Err(registration_error("driver_instance_declaration_invalid"));
    }
    Ok(())
}

fn validate_definition(
    manifest: &DriverManifest,
    result: &crate::driver::DriverDefinition,
) -> Result<(), RuntimeError> {
    if result.driver_id != manifest.id
        || result.driver_version != manifest.version
        || !manifest
            .runtime
            .requires
            .iter()
            .all(|f| result.features.contains(f))
        || !crate::driver::REQUIRED_FEATURES
            .iter()
            .all(|f| result.features.iter().any(|enabled| enabled == f))
        || result.features.iter().enumerate().any(|(index, f)| {
            result.features[..index].contains(f)
                || !crate::driver::REQUIRED_FEATURES.contains(&f.as_str())
                    && !crate::driver::OPTIONAL_FEATURES.contains(&f.as_str())
        })
        || result.features.iter().any(|f| f == "deferred-interactions")
            && !result.features.iter().any(|f| f == "secret-delivery")
        || !result.mcp_tools.is_empty() && !result.features.iter().any(|f| f == "native-mcp-tools")
    {
        return Err(registration_error("driver_definition_invalid"));
    }
    let mut catalog = HashMap::new();
    let mut names = HashSet::new();
    for descriptor in &result.operations {
        if descriptor.name.trim().is_empty()
            || descriptor.name.len() > 128
            || !names.insert(descriptor.name.clone())
            || !descriptor.input_schema.is_object()
            || jsonschema::validator_for(&descriptor.input_schema).is_err()
            || descriptor
                .output_schema
                .as_ref()
                .is_some_and(|schema| jsonschema::validator_for(schema).is_err())
        {
            return Err(runtime_error(
                ErrorCategory::InvalidRequest,
                "driver_operation_catalog_invalid",
            ));
        }
        catalog.insert(descriptor.name.clone(), descriptor.clone());
    }
    let mut tool_names = HashSet::new();
    for tool in &result.mcp_tools {
        let name = tool
            .descriptor
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let schema = tool.descriptor.get("inputSchema");
        let operation_scope = catalog.get(&tool.operation).map(|op| op.scope);
        let requires_instance = operation_scope == Some(OperationScope::Instance);
        let has_instance_param = schema
            .and_then(|s| s.get("required"))
            .and_then(Value::as_array)
            .is_some_and(|required| required.iter().any(|value| value == "driverInstanceId"));
        if name.is_empty()
            || name.len() > 64
            || manifest.id.len() + 2 + name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || !tool_names.insert(name)
            || operation_scope.is_none()
            || !result.features.iter().any(|f| f == "native-mcp-tools")
            || has_instance_param != requires_instance
            || (requires_instance
                && schema
                    .and_then(|s| s.pointer("/properties/driverInstanceId/type"))
                    .and_then(Value::as_str)
                    != Some("string"))
            || schema.and_then(|s| s.get("type")).and_then(Value::as_str) != Some("object")
            || schema.is_none_or(|schema| jsonschema::validator_for(schema).is_err())
        {
            return Err(runtime_error(
                ErrorCategory::InvalidRequest,
                "driver_mcp_catalog_invalid",
            ));
        }
    }
    Ok(())
}

fn is_valid_driver_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn registration_error(code: &str) -> RuntimeError {
    runtime_error(ErrorCategory::InvalidRequest, code)
}

#[cfg(test)]
mod tests {
    use crate::model::operation::{
        OperationDescriptor, OperationEffect, OperationScope, OperationSource,
    };

    use super::{
        failure_status, operation_args_error, schema_is_valid, unique_instance_label,
        validate_driver_authority, validate_scope_target, Runtime,
    };
    use crate::approval_policy::{ApprovalContext, InstanceApproval};
    use crate::model::interaction::{InteractionKind, InteractionRequest};
    use crate::model::lifecycle::{InstanceInfo, InstanceStatus, RuntimeLimits};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn fixed_definitions_validate_identity_capabilities_and_catalog_before_start() {
        let manifest: crate::model::manifest::DriverManifest = serde_json::from_value(json!({
            "manifestVersion":1,"id":"coterm-test","name":"test","version":"0.1.0",
            "runtime":{"interfaceMajor":2,"requires":["instances","backend-calls","operation-catalog"]},
            "frontend":null,"mcp":{"provider":"backend"},
            "instances":{"type":"test","multiple":true},"capabilities":[]
        })).unwrap();
        let valid = crate::driver::DriverDefinition {
            driver_id: "coterm-test".into(),
            driver_version: "0.1.0".into(),
            features: crate::driver::REQUIRED_FEATURES
                .iter()
                .map(|f| (*f).to_owned())
                .collect(),
            operations: vec![],
            mcp_tools: vec![],
        };
        super::validate_definition(&manifest, &valid).unwrap();
        for invalid in [
            crate::driver::DriverDefinition {
                driver_id: "other".into(),
                ..valid.clone()
            },
            crate::driver::DriverDefinition {
                driver_version: "2.0.0".into(),
                ..valid.clone()
            },
            crate::driver::DriverDefinition {
                features: vec![],
                ..valid.clone()
            },
            crate::driver::DriverDefinition {
                features: [valid.features.clone(), vec!["instances".into()]].concat(),
                ..valid.clone()
            },
            crate::driver::DriverDefinition {
                features: [valid.features.clone(), vec!["unknown".into()]].concat(),
                ..valid.clone()
            },
            crate::driver::DriverDefinition {
                features: [valid.features.clone(), vec!["deferred-interactions".into()]].concat(),
                ..valid.clone()
            },
        ] {
            assert_eq!(
                super::validate_definition(&manifest, &invalid)
                    .unwrap_err()
                    .code,
                "driver_definition_invalid"
            );
        }
        let invalid_schema = crate::driver::DriverDefinition {
            operations: vec![OperationDescriptor {
                name: "test.read".into(),
                scope: OperationScope::Instance,
                effect: OperationEffect::Read,
                input_schema: json!({"type":"not-a-schema-type"}),
                output_schema: None,
                sensitive_input: false,
                transient: true,
            }],
            ..valid
        };
        assert_eq!(
            super::validate_definition(&manifest, &invalid_schema)
                .unwrap_err()
                .code,
            "driver_operation_catalog_invalid"
        );
    }

    #[test]
    fn duplicate_instance_label_suffix_stays_within_utf8_limit() {
        let long = "名".repeat(85);
        let label = unique_instance_label(&long, [long.as_str()].into_iter());
        assert!(label.ends_with(" (2)"));
        assert!(label.len() <= 256);
        assert!(label.is_char_boundary(label.len()));
    }

    fn runtime() -> Runtime {
        Runtime::new(RuntimeLimits {
            max_control_bytes: 1024 * 1024,
        })
    }

    #[tokio::test]
    async fn invalid_deferred_acceptance_is_unknown_after_dispatch() {
        use crate::model::operation::{OperationRecord, OperationStatus};
        for (reply, negotiated) in [
            (json!({"_cotermDeferred":"wrong-operation"}), true),
            (json!({"_cotermDeferred":42}), true),
            (json!({"_cotermDeferred":"op"}), false),
        ] {
            let runtime = runtime();
            let now = chrono::Utc::now();
            runtime
                .history
                .begin(OperationRecord {
                    operation_id: "op".to_owned(),
                    driver_id: "driver".to_owned(),
                    driver_instance_id: Some("instance".to_owned()),
                    source: OperationSource::Human,
                    effect: OperationEffect::Commit,
                    operation: "write".to_owned(),
                    status: OperationStatus::Accepted,
                    input: json!({}),
                    output: None,
                    error: None,
                    created_at: now,
                    updated_at: now,
                    proposal_id: None,
                    wait_expired: false,
                    transient: false,
                })
                .await
                .unwrap();
            runtime
                .history
                .transition("op", OperationStatus::Dispatched, None, None)
                .await
                .unwrap();
            let mut state = runtime.state.lock().await;
            if negotiated {
                state
                    .features
                    .insert("driver".to_owned(), vec!["async-operations".to_owned()]);
            }
            let record = runtime
                .finish_reply(&mut state, "op", Ok(reply))
                .await
                .unwrap();
            assert_eq!(record.status, OperationStatus::Unknown);
            assert_eq!(record.error.unwrap().code, "invalid_deferred_operation");
        }
    }

    fn instance(driver_id: &str, instance_id: &str) -> InstanceInfo {
        InstanceInfo {
            driver_id: driver_id.to_owned(),
            driver_instance_id: instance_id.to_owned(),
            session_id: format!("session-{instance_id}"),
            label: format!("{driver_id} {instance_id}"),
            status: InstanceStatus::Ready,
        }
    }

    fn secret_request(id: &str, instance_id: &str, timeout_ms: Option<u64>) -> InteractionRequest {
        InteractionRequest {
            interaction_id: id.to_owned(),
            driver_instance_id: instance_id.to_owned(),
            operation_id: Some(format!("operation-{id}")),
            kind: InteractionKind::Secret {
                purpose: "ssh.password".to_owned(),
            },
            title: "SSH password".to_owned(),
            prompt: "Password".to_owned(),
            timeout_ms,
            allow_cancel: true,
            deadline: None,
        }
    }

    #[tokio::test]
    async fn snapshot_lists_every_pending_approval_and_interaction() {
        let runtime = runtime();
        let state = Arc::clone(&runtime.state);
        {
            let mut state = state.lock().await;
            state.instances.insert(
                "instance-a".to_owned(),
                instance("com.example.alpha", "instance-a"),
            );
            state.instances.insert(
                "instance-b".to_owned(),
                instance("com.example.alpha", "instance-b"),
            );
            for (operation_id, instance_id) in [("op-1", "instance-a"), ("op-2", "instance-b")] {
                state.pending_approvals.insert(
                    operation_id.to_owned(),
                    (
                        "com.example.alpha".to_owned(),
                        Some(instance_id.to_owned()),
                        format!("proposal-{operation_id}"),
                        "session.write".to_owned(),
                    ),
                );
            }
            // 驱动级审批：没有实例目标，目标标签回落到驱动 ID。
            state.pending_approvals.insert(
                "op-3".to_owned(),
                (
                    "com.example.beta".to_owned(),
                    None,
                    "proposal-op-3".to_owned(),
                    "driver.configure".to_owned(),
                ),
            );
            state.interactions.insert(
                "interaction-1".to_owned(),
                (
                    secret_request("interaction-1", "instance-a", Some(60_000)),
                    tokio::time::Instant::now() + Duration::from_secs(60),
                ),
            );
            runtime.bump_attention();
        }

        let snapshot = runtime.attention_snapshot().await;
        assert_eq!(snapshot.approvals.len(), 3);
        assert_eq!(snapshot.interactions.len(), 1);
        // 稳定排序：前端以 operationId / interactionId 作为渲染 key。
        let ids = snapshot
            .approvals
            .iter()
            .map(|approval| approval.operation_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["op-1", "op-2", "op-3"]);
        assert_eq!(
            snapshot.approvals[0].target_label,
            "com.example.alpha instance-a"
        );
        assert_eq!(snapshot.approvals[2].driver_instance_id, None);
        assert_eq!(snapshot.approvals[2].target_label, "com.example.beta");
        assert_eq!(snapshot.approvals[0].operation, "session.write");
        assert_eq!(snapshot.interactions[0].interaction_id, "interaction-1");
        assert!(snapshot.interactions[0].remaining_ms > 0);
        assert!(snapshot.interactions[0].remaining_ms <= 60_000);
        assert_eq!(snapshot.version, 1);
    }

    #[tokio::test]
    async fn snapshot_omits_expired_interactions_and_never_exposes_secret_values() {
        let runtime = runtime();
        let state = Arc::clone(&runtime.state);
        {
            let mut state = state.lock().await;
            state.interactions.insert(
                "expired".to_owned(),
                (
                    secret_request("expired", "instance-a", Some(1)),
                    tokio::time::Instant::now() - Duration::from_secs(1),
                ),
            );
            state.interactions.insert(
                "live".to_owned(),
                (
                    secret_request("live", "instance-a", Some(60_000)),
                    tokio::time::Instant::now() + Duration::from_secs(60),
                ),
            );
        }
        let snapshot = runtime.attention_snapshot().await;
        assert_eq!(snapshot.interactions.len(), 1);
        assert_eq!(snapshot.interactions[0].interaction_id, "live");
    }

    #[tokio::test]
    async fn version_advances_on_every_attention_change_and_only_on_change() {
        let runtime = runtime();
        let state = Arc::clone(&runtime.state);
        assert_eq!(runtime.attention_version(), 0);
        {
            let mut state = state.lock().await;
            state.interactions.insert(
                "interaction-1".to_owned(),
                (
                    secret_request("interaction-1", "instance-a", Some(60_000)),
                    tokio::time::Instant::now() + Duration::from_secs(60),
                ),
            );
            runtime.bump_attention();
        }
        assert_eq!(runtime.attention_version(), 1);
        // 快照是只读的，读取不改变版本。
        let first = runtime.attention_snapshot().await;
        let second = runtime.attention_snapshot().await;
        assert_eq!(first.version, second.version);
        assert_eq!(second.version, 1);

        // 拆除实例时，即使没有命中任何待处理项也会推进版本，
        // 这样实例标签变化不会让前端停留在旧快照。
        let mut state = state.lock().await;
        runtime
            .cancel_instance_interactions(&mut state, &["instance-unused".to_owned()])
            .await;
        drop(state);
        assert_eq!(runtime.attention_version(), 2);
        assert_eq!(runtime.attention_snapshot().await.version, 2);
    }

    #[test]
    fn approval_entries_carry_display_fields_without_operation_args() {
        // 快照结构本身不含 args/input/draft；这里显式固定字段清单，
        // 防止后续有人把操作参数塞进 changed 路径。
        let approval = super::PendingApproval {
            operation_id: "op-1".to_owned(),
            proposal_id: "proposal-1".to_owned(),
            driver_id: "com.example.alpha".to_owned(),
            driver_instance_id: Some("instance-a".to_owned()),
            operation: "session.write".to_owned(),
            target_label: "Alpha".to_owned(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let value = serde_json::to_value(&approval).expect("approval serializes");
        let keys = value
            .as_object()
            .expect("object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "createdAt",
                "driverId",
                "driverInstanceId",
                "operation",
                "operationId",
                "proposalId",
                "targetLabel",
            ]
        );
    }

    #[test]
    fn interaction_deadline_is_optional_on_the_wire() {
        // 驱动发来的请求不含 deadline；基座在受理时补齐。
        let request: InteractionRequest = serde_json::from_value(json!({
            "interactionId": "interaction-1",
            "driverInstanceId": "instance-a",
            "operationId": "op-1",
            "kind": {"kind": "secret", "purpose": "ssh.password"},
            "title": "SSH password",
            "prompt": "Password",
            "timeoutMs": 60000,
            "allowCancel": true
        }))
        .expect("legacy request parses");
        assert!(request.deadline.is_none());
    }

    fn descriptor(effect: OperationEffect, sensitive_input: bool) -> OperationDescriptor {
        OperationDescriptor {
            name: "driver.operation".into(),
            scope: OperationScope::Instance,
            effect,
            input_schema: serde_json::json!({"type":"object"}),
            output_schema: None,
            sensitive_input,
            transient: false,
        }
    }

    #[test]
    fn instance_approval_checks_source_and_effect() {
        let mut approval = InstanceApproval::default();
        let args = serde_json::json!({});
        for source in [
            OperationSource::Human,
            OperationSource::System,
            OperationSource::Agent,
        ] {
            for effect in [
                OperationEffect::Read,
                OperationEffect::Draft,
                OperationEffect::Commit,
            ] {
                let descriptor = descriptor(effect, true);
                let context = ApprovalContext {
                    source,
                    descriptor: &descriptor,
                    args: &args,
                };
                assert_eq!(
                    approval.requires_approval(&context),
                    source == OperationSource::Agent && effect == OperationEffect::Commit
                );
            }
        }
        approval.set_bypass(true);
        let descriptor = descriptor(OperationEffect::Commit, true);
        assert!(!approval.requires_approval(&ApprovalContext {
            source: OperationSource::Agent,
            descriptor: &descriptor,
            args: &args
        }));
    }

    #[test]
    fn transport_failure_after_dispatch_is_unknown_not_failed() {
        let transport = super::runtime_error(
            crate::model::ErrorCategory::Transport,
            "driver_disconnected",
        );
        let domain =
            super::runtime_error(crate::model::ErrorCategory::Domain, "device_rejected_write");
        assert_eq!(
            failure_status(&transport),
            crate::model::operation::OperationStatus::Unknown
        );
        assert_eq!(
            failure_status(&domain),
            crate::model::operation::OperationStatus::Failed
        );
    }

    #[test]
    fn driver_scope_has_no_instance_target_and_instance_scope_requires_one() {
        use crate::model::operation::OperationScope;

        assert!(validate_scope_target(OperationScope::Driver, false).is_ok());
        assert_eq!(
            validate_scope_target(OperationScope::Driver, true)
                .expect_err("driver operation cannot accept an instance target")
                .code,
            "driver_operation_does_not_accept_instance"
        );
        assert_eq!(
            validate_scope_target(OperationScope::Instance, false)
                .expect_err("instance operation requires a target")
                .code,
            "driver_instance_required"
        );
    }

    #[test]
    fn driver_scope_authority_is_bound_to_the_routed_driver() {
        use crate::model::operation::OperationSource;
        use crate::runtime::CallerContext;

        let caller = CallerContext {
            source: OperationSource::Agent,
            session_id: String::new(),
            driver_id: Some("com.example.alpha".to_owned()),
        };
        assert!(validate_driver_authority(&caller, "com.example.alpha").is_ok());
        assert_eq!(
            validate_driver_authority(&caller, "com.example.beta")
                .expect_err("cross-driver routing must be denied")
                .code,
            "driver_scope_access_denied"
        );
    }

    #[test]
    fn schemas_validate_types_bounds_required_fields_and_outputs() {
        let schema = json!({
            "type":"object",
            "properties":{"count":{"type":"integer","minimum":1,"maximum":4}},
            "required":["count"],
            "additionalProperties":false
        });
        assert!(schema_is_valid(&schema, &json!({"count":2})));
        assert!(!schema_is_valid(&schema, &json!({"count":"2"})));
        assert!(!schema_is_valid(&schema, &json!({"count":5})));
        assert!(!schema_is_valid(&schema, &json!({"extra":true})));
        assert!(!schema_is_valid(&schema, &json!({})));
    }

    #[test]
    fn schema_errors_name_unknown_and_missing_fields_without_echoing_values() {
        let schema = json!({
            "type": "object",
            "properties": {"commands": {"type": "array"}},
            "required": ["commands"],
            "additionalProperties": false,
        });
        let message = operation_args_error(&schema, &json!({"line_ending":"private-value"}));
        assert!(message.contains("line_ending"));
        assert!(message.contains("commands"));
        assert!(!message.contains("private-value"));
    }
}
