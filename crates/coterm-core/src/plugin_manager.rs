//! Minimal process-backed plugin lifecycle manager for control requests.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::Utc;
use coterm_proto::runtime::{
    interaction::{
        DeferredInteraction, InteractionKind, InteractionOutcome, InteractionRequest,
        InteractionResult, InteractionValue, SecretDelivery, SecretHandle,
    },
    lifecycle::{
        CreateInstance, InstanceInfo, InstanceStatus, McpToolRegistration, RuntimeHello,
        RuntimeLimits,
    },
    manifest::PluginManifest,
    message::{DeferredOperation, HostCommand, PluginNotification},
    operation::{
        ApproveProposal, CancellationOutcome, DispatchMode, InvokeRequest, OperationDescriptor,
        OperationEffect, OperationReceipt, OperationRecord, OperationScope, OperationSource,
        OperationStatus,
    },
    ErrorCategory, RuntimeError, RuntimeFuture,
};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    operation_history::{OperationHistory, OperationHistoryError},
    plugin_process::{
        resolve_backend_entry, resolve_package_file, validate_manifest, JsonRpcPluginProcess,
        MAX_MANIFEST_BYTES,
    },
    plugin_runtime::{CallerContext, Operations, PluginConnection, PluginRuntime, SubmitOperation},
};

#[derive(Default)]
struct State {
    mcp_tools: HashMap<String, Vec<McpToolRegistration>>,
    features: HashMap<String, Vec<String>>,
    interactions: HashMap<String, (InteractionRequest, tokio::time::Instant)>,
    packages: HashMap<String, super::plugin_process::PluginPackage>,
    processes: HashMap<String, Arc<JsonRpcPluginProcess>>,
    instances: HashMap<String, InstanceInfo>,
    operation_catalog: HashMap<String, HashMap<String, OperationDescriptor>>,
    /// (plugin_id, plugin_instance_id, proposal_id, operation)
    pending_approvals: HashMap<String, (String, Option<String>, String, String)>,
}

/// 宿主级待处理操作；只含 UI 渲染所需字段，不含操作参数或秘密值。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub operation_id: String,
    pub proposal_id: String,
    pub plugin_id: String,
    /// None 表示插件级操作。
    pub plugin_instance_id: Option<String>,
    pub operation: String,
    pub target_label: String,
    pub created_at: String,
}

/// 宿主级待交互请求；秘密值永不在此出现。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInteraction {
    pub interaction_id: String,
    pub plugin_instance_id: String,
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

/// 单个插件包加载失败的可序列化诊断。
/// 只含插件目录名、插件 ID（能解析时）和错误码，不含绝对路径或内部错误明文。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginLoadIssue {
    /// 插件目录名；用于定位是哪个包出了问题。
    pub directory: String,
    /// manifest 能被解析且 ID 合法时给出。
    pub plugin_id: Option<String>,
    pub code: String,
    pub message: String,
}

/// `list_plugins` 的结果：有效插件与逐包诊断分开返回，单个包失败不影响其余包。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginList {
    pub plugins: Vec<PluginManifest>,
    pub issues: Vec<PluginLoadIssue>,
}

/// 启动扫描发现的一个 `<root>/<package>` 目录。只保存目录名和包根，
/// 诊断和错误消息都不回显绝对路径。
#[derive(Debug, Clone)]
pub struct DiscoveredPackage {
    pub directory: String,
    /// 包根，通常等于扫描根下的 `directory`；开发包可能指向 `plugins/<dir>/dist`。
    pub root: PathBuf,
    /// 已安装包优先于开发包；两个来源声明同一 ID 时较矮的优先级被忽略并记诊断。
    pub priority: u8,
    pub origin: PackageOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageOrigin {
    /// 开发搜索到的 `plugins/<dir>/dist`。
    Development,
    /// 应用数据目录下的已安装包。
    Installed,
}

impl DiscoveredPackage {
    fn issue(&self, code: &str, message: &str) -> PluginLoadIssue {
        PluginLoadIssue {
            directory: self.directory.clone(),
            plugin_id: None,
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }

    /// 读取并校验 manifest 与包内入口；任何失败都转成不含绝对路径的诊断。
    async fn load_into(&self, runtime: &ProcessPluginRuntime) -> Result<PluginManifest, PluginLoadIssue> {
        let missing_manifest = match self.origin {
            PackageOrigin::Development => {
                "Package has no built manifest; run the package build to produce dist/manifest.json"
            }
            PackageOrigin::Installed => "Package has no manifest.json",
        };
        let metadata = tokio::fs::metadata(self.root.join("manifest.json"))
            .await
            .map_err(|_| self.issue("plugin_manifest_unavailable", missing_manifest))?;
        if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
            return Err(self.issue(
                "plugin_manifest_unavailable",
                missing_manifest,
            ));
        }
        let bytes = tokio::fs::read(self.root.join("manifest.json"))
            .await
            .map_err(|_| self.issue("plugin_manifest_read_failed", "Package manifest could not be read"))?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            self.issue("plugin_manifest_invalid", "Package manifest is not valid JSON")
        })?;
        // 旧格式包缺少 manifestVersion：给出可定位诊断，不静默当成有效插件。
        if value.get("manifestVersion").is_none() {
            return Err(PluginLoadIssue {
                directory: self.directory.clone(),
                plugin_id: value
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| is_manifest_id(id))
                    .map(str::to_owned),
                code: "plugin_manifest_legacy".to_owned(),
                message: issue_message("plugin_manifest_legacy").to_owned(),
            });
        }
        let manifest: PluginManifest = serde_json::from_value(value).map_err(|_| {
            self.issue(
                "plugin_manifest_invalid",
                "Package manifest does not match the manifest schema",
            )
        })?;
        let id = manifest.id.clone();
        let diagnose = |code: &str| PluginLoadIssue {
            directory: self.directory.clone(),
            plugin_id: Some(id.clone()),
            code: code.to_owned(),
            message: issue_message(code).to_owned(),
        };
        validate_manifest(&manifest).map_err(|error| diagnose(&error.code))?;
        // 同 ID 冲突：先扫描到的来源胜出，其余包得到可定位诊断而不是静默消失。
        if runtime.is_discovered(&id).await {
            return Err(PluginLoadIssue {
                directory: self.directory.clone(),
                plugin_id: Some(id),
                code: "plugin_id_conflict".to_owned(),
                message: issue_message("plugin_id_conflict").to_owned(),
            });
        }
        let root = self
            .root
            .canonicalize()
            .map_err(|_| diagnose("plugin_package_unavailable"))?;
        // 逐平台入口缺失是包的问题，不是本次平台的限制：全部校验后才允许加载。
        for relative in manifest.backend.values() {
            resolve_package_file(&root, relative, "plugin_entry_invalid")
                .map_err(|error| diagnose(&error.code))?;
        }
        if let Some(frontend) = &manifest.frontend {
            resolve_package_file(&root, &frontend.entry, "plugin_frontend_entry_invalid")
                .map_err(|error| diagnose(&error.code))?;
        }
        // 此时已确认本平台入口存在；完整包校验仍在这里跑一次，失败按稳定 code 报出。
        runtime
            .insert_validated(manifest, root, &runtime.platform)
            .await
            .map_err(|error| diagnose(&error.code))
    }
}

fn issue_message(code: &str) -> &'static str {
    match code {
        "plugin_manifest_legacy" => {
            "Legacy manifest format is not supported; rebuild the package with the current manifest schema"
        }
        "plugin_manifest_version_unsupported" => {
            "Package manifest version is not supported by this host"
        }
        "plugin_manifest_identity_invalid" => "Package manifest has an invalid ID, name, or version",
        "plugin_runtime_version_unsupported" => {
            "Package targets an incompatible runtime interface version"
        }
        "plugin_required_feature_unsupported" => {
            "Package requires a runtime feature this host does not provide"
        }
        "plugin_instance_declaration_invalid" => {
            "Package does not declare a valid multi-instance contract"
        }
        "plugin_platform_unsupported" => "Package has no backend entry for this platform",
        "plugin_entry_invalid" => "Package backend entry is missing or outside the package",
        "plugin_frontend_entry_invalid" => "Package frontend entry is missing or outside the package",
        "plugin_package_unavailable" => "Package directory is no longer available",
        "cannot_replace_running_plugin" => {
            "A running plugin cannot be replaced by another package with the same ID"
        }
        "plugin_id_conflict" => {
            "Another package with the same plugin ID was found first; this one was ignored"
        }
        _ => "Package could not be loaded",
    }
}

fn is_manifest_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Process-backed lifecycle support for the synchronous control MVP.
#[derive(Clone)]
pub struct ProcessPluginRuntime {
    platform: String,
    limits: RuntimeLimits,
    state: Arc<Mutex<State>>,
    history: Arc<OperationHistory>,
    bypass: Arc<tokio::sync::RwLock<bool>>,
    /// 仅承载版本号；快照内容以 `state` 为准，二者在同一把锁内推进。
    attention: Arc<std::sync::Mutex<u64>>,
}

impl ProcessPluginRuntime {
    pub fn new(platform: impl Into<String>, limits: RuntimeLimits) -> Self {
        Self {
            platform: platform.into(),
            limits,
            state: Arc::new(Mutex::new(State::default())),
            history: Arc::new(OperationHistory::new(100, 16 * 1024)),
            bypass: Arc::new(tokio::sync::RwLock::new(false)),
            attention: Arc::new(std::sync::Mutex::new(0)),
        }
    }

    /// 必须在持有 `state` 锁时调用，保证版本与快照内容一致。
    fn bump_attention(&self) {
        let mut version = self.attention.lock().unwrap_or_else(|error| error.into_inner());
        *version += 1;
    }

    pub fn attention_version(&self) -> u64 {
        *self.attention.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// 一次调用覆盖全部实例级与插件级待处理项。
    pub async fn attention_snapshot(&self) -> AttentionSnapshot {
        // 先取版本再读状态：并发变化只会让前端拿到较旧的手快照，
        // 而对应 changed 事件已经包含更高版本，前端会据此重取。
        let version = self.attention_version();
        let state = self.state.lock().await;
        let now = tokio::time::Instant::now();
        let mut approvals = state
            .pending_approvals
            .iter()
            .map(|(operation_id, (plugin_id, instance, proposal_id, operation))| {
                let target_label = instance
                    .as_ref()
                    .and_then(|id| state.instances.get(id))
                    .map(|instance| instance.label.clone())
                    .unwrap_or_else(|| plugin_id.clone());
                PendingApproval {
                    operation_id: operation_id.clone(),
                    proposal_id: proposal_id.clone(),
                    plugin_id: plugin_id.clone(),
                    plugin_instance_id: instance.clone(),
                    operation: operation.clone(),
                    target_label,
                    created_at: String::new(),
                }
            })
            .collect::<Vec<_>>();
        approvals.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
        let mut interactions = state
            .interactions
            .values()
            .map(|(request, deadline)| (request, deadline))
            .filter(|(_, deadline)| **deadline > now)
            .map(|(request, deadline)| PendingInteraction {
                interaction_id: request.interaction_id.clone(),
                plugin_instance_id: request.plugin_instance_id.clone(),
                operation_id: request.operation_id.clone(),
                kind: request.kind.clone(),
                title: request.title.clone(),
                prompt: request.prompt.clone(),
                allow_cancel: request.allow_cancel,
                remaining_ms: deadline.duration_since(now).as_millis() as u64,
                deadline: request.deadline.map(|value| value.to_rfc3339()).unwrap_or_default(),
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

    pub async fn is_running(&self, plugin_id: &str) -> bool {
        self.state.lock().await.processes.contains_key(plugin_id)
    }

    /// 该 ID 是否已被某个扫描到的包占用；用于同 ID 冲突诊断。
    pub async fn is_discovered(&self, plugin_id: &str) -> bool {
        self.state.lock().await.packages.contains_key(plugin_id)
    }

    /// 发现路径已经校验过 manifest 与包内入口；这里补上目标平台的入口装配。
    async fn insert_validated(
        &self,
        manifest: PluginManifest,
        root: PathBuf,
        platform: &str,
    ) -> Result<PluginManifest, RuntimeError> {
        let backend_entry = resolve_backend_entry(&root, &manifest, platform)?;
        let frontend_entry = manifest
            .frontend
            .as_ref()
            .map(|frontend| {
                resolve_package_file(&root, &frontend.entry, "plugin_frontend_entry_invalid")
            })
            .transpose()?;
        self.insert_package(super::plugin_process::PluginPackage {
            root,
            manifest,
            backend_entry,
            frontend_entry,
        })
        .await
    }

    /// 运行中的包不能被另一个目录的同 ID 包替换；否则以目录为权威来源。
    async fn insert_package(
        &self,
        package: super::plugin_process::PluginPackage,
    ) -> Result<PluginManifest, RuntimeError> {
        let manifest = package.manifest.clone();
        let mut state = self.state.lock().await;
        if state.processes.contains_key(&manifest.id) {
            let Some(loaded) = state.packages.get(&manifest.id) else {
                return Err(runtime_error(
                    ErrorCategory::Internal,
                    "running_plugin_package_missing",
                ));
            };
            if loaded.root != package.root {
                return Err(runtime_error(
                    ErrorCategory::Conflict,
                    "cannot_replace_running_plugin",
                ));
            }
            return Ok(loaded.manifest.clone());
        }
        state.packages.insert(manifest.id.clone(), package);
        Ok(manifest)
    }

    pub async fn loaded_manifests(&self) -> Vec<PluginManifest> {
        let state = self.state.lock().await;
        Self::sorted_manifests(state.packages.values().map(|package| &package.manifest))
    }

    /// 确定性排序：先插件名，再插件 ID；同名不同 ID 也不会因遍历顺序抖动。
    fn sorted_manifests<'a>(manifests: impl Iterator<Item = &'a PluginManifest>) -> Vec<PluginManifest> {
        let mut manifests = manifests.cloned().collect::<Vec<_>>();
        manifests.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.cmp(&right.id))
        });
        manifests
    }

    /// 按稳定顺序加载启动扫描到的目录；单个包失败只产出一条诊断，不影响其余包。
    /// 每个目录都是一个 `<root>/<package>` 包根，manifest 位于包根下的 `dist/manifest.json`。
    pub async fn load_discovered(
        &self,
        scanned: Vec<DiscoveredPackage>,
    ) -> Vec<PluginLoadIssue> {
        let mut issues = Vec::new();
        for package in scanned {
            if let Err(issue) = package.load_into(self).await {
                issues.push(issue);
            }
        }
        issues.sort_by(|left, right| {
            left.directory
                .cmp(&right.directory)
                .then_with(|| left.code.cmp(&right.code))
        });
        issues
    }

    /// 一次调用返回可列出的插件和逐包诊断；任一包失败都不会让整次发现失败。
    pub async fn discover(&self, scanned: Vec<DiscoveredPackage>) -> PluginList {
        let issues = self.load_discovered(scanned).await;
        let plugins = self.loaded_manifests().await;
        PluginList { plugins, issues }
    }

    pub async fn set_bypass(&self, enabled: bool) {
        *self.bypass.write().await = enabled;
    }

    pub async fn bypass_enabled(&self) -> bool {
        *self.bypass.read().await
    }

    pub async fn operation_catalog(&self, plugin_id: &str) -> Vec<OperationDescriptor> {
        let state = self.state.lock().await;
        state
            .operation_catalog
            .get(plugin_id)
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
    ) -> Result<tokio::sync::broadcast::Receiver<PluginNotification>, RuntimeError> {
        let state = self.state.lock().await;
        let instance = state
            .instances
            .get(instance_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "plugin_instance_not_found"))?;
        let process = state
            .processes
            .get(&instance.plugin_id)
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
        Ok(process.events.subscribe())
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

    pub async fn pending_plugin_operation(
        &self,
        plugin_id: &str,
        proposal_id: &str,
    ) -> Option<String> {
        self.state
            .lock()
            .await
            .pending_approvals
            .iter()
            .find(|(_, pending)| {
                pending.0 == plugin_id && pending.1.is_none() && pending.2 == proposal_id
            })
            .map(|(operation_id, _)| operation_id.clone())
    }

    pub async fn recent_plugin_operations(
        &self,
        plugin_id: &str,
        limit: usize,
    ) -> Vec<OperationRecord> {
        self.history.recent(plugin_id, None, limit).await
    }

    /// Only returns the package's validated, self-contained frontend entry.
    pub async fn frontend_source(&self, plugin_id: &str) -> Result<Option<String>, RuntimeError> {
        let entry = self
            .state
            .lock()
            .await
            .packages
            .get(plugin_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "plugin_not_loaded"))?
            .frontend_entry
            .clone();
        let Some(entry) = entry else {
            return Ok(None);
        };
        let size = tokio::fs::metadata(&entry)
            .await
            .map_err(|_| runtime_error(ErrorCategory::Transport, "frontend_unavailable"))?
            .len();
        if size > 2 * 1024 * 1024 {
            return Err(runtime_error(
                ErrorCategory::InvalidRequest,
                "frontend_too_large",
            ));
        }
        tokio::fs::read_to_string(entry)
            .await
            .map(Some)
            .map_err(|_| runtime_error(ErrorCategory::Transport, "frontend_read_failed"))
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
        let state = self.state.lock().await;
        let instance = state
            .instances
            .get(instance_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "plugin_instance_not_found"))?;
        let package = state
            .packages
            .get(&instance.plugin_id)
            .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "plugin_not_loaded"))?;
        if !package
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
        let process = state
            .processes
            .get(&instance.plugin_id)
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
        let handle = SecretHandle {
            id: uuid::Uuid::new_v4().to_string(),
            plugin_instance_id: instance_id.to_owned(),
            purpose,
            expires_at: Utc::now() + chrono::Duration::seconds(30),
        };
        process
            .request(HostCommand::ProvideSecret(SecretDelivery {
                handle: handle.clone(),
                value,
            }))
            .await?;
        Ok(handle)
    }
}

impl ProcessPluginRuntime {
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
                        .get(&record.plugin_id)
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
                                .get(&record.plugin_id)
                                .is_some_and(|features| {
                                    features.iter().any(|f| f == "deferred-interactions")
                                });
                        let allowed = state.packages.get(&record.plugin_id).is_some_and(|p| {
                            p.manifest.capabilities.iter().any(|c| c == "secret-input")
                        });
                        if !supported
                            || !allowed
                            || record.status != OperationStatus::Dispatched
                            || !matches!(&request.kind, InteractionKind::Secret { purpose } if !purpose.is_empty() && purpose.len() <= 128)
                            || request.operation_id.as_deref() != Some(operation_id)
                            || record.plugin_instance_id.as_deref()
                                != Some(&request.plugin_instance_id)
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
                    .get(&record.plugin_id)
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
                .provide_secret(&request.plugin_instance_id, purpose, value)
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
        self.resume_interaction(&mut state, request, outcome).await
    }

    async fn resume_interaction(
        &self,
        state: &mut State,
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
        // Instance destruction wins over a late credential delivery.
        if record.status != OperationStatus::Running
            || !state.instances.contains_key(&request.plugin_instance_id)
        {
            return Err(runtime_error(
                ErrorCategory::Conflict,
                "interaction_no_longer_active",
            ));
        }
        let cancelled = !matches!(outcome, InteractionOutcome::Submitted(_));
        let process = state
            .processes
            .get(&record.plugin_id)
            .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
        let result = process
            .request(HostCommand::InteractionResult(InteractionResult {
                interaction_id: request.interaction_id,
                plugin_instance_id: request.plugin_instance_id,
                outcome,
            }))
            .await;
        if cancelled {
            self.history
                .transition(&operation_id, OperationStatus::Cancelled, None, None)
                .await
                .map_err(map_history_error)?;
        } else {
            self.finish_reply(state, &operation_id, result).await?;
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
        for id in ids {
            if let Some((request, _)) = state.interactions.remove(&id) {
                self.bump_attention();
                let _ = self
                    .resume_interaction(&mut state, request, InteractionOutcome::TimedOut)
                    .await;
            }
        }
    }

    async fn cancel_instance_interactions(&self, state: &mut State, instances: &[String]) {
        // 无论是否命中，实例销毁/插件停止都会改变宿主待处理视图，统一推高版本。
        self.bump_attention();
        state.pending_approvals.retain(|_, (_, instance, _, _)| {
            !instance.as_ref().is_some_and(|id| instances.contains(id))
        });
        for instance_id in instances {
            if let Some(instance) = state.instances.get(instance_id) {
                for record in self.history.active(&instance.plugin_id, instance_id).await {
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
                                OperationStatus::Cancelled,
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
            .filter(|(_, (r, _))| instances.contains(&r.plugin_instance_id))
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

impl PluginRuntime for ProcessPluginRuntime {
    fn load<'a>(&'a self, package_path: &'a Path) -> RuntimeFuture<'a, PluginManifest> {
        Box::pin(async move {
            let package =
                super::plugin_process::load_plugin_package(package_path, &self.platform).await?;
            self.insert_package(package).await
        })
    }

    fn start<'a>(&'a self, plugin_id: &'a str) -> RuntimeFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            if state.processes.contains_key(plugin_id) {
                return Ok(());
            }
            let package = state
                .packages
                .get(plugin_id)
                .cloned()
                .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "plugin_not_loaded"))?;
            let process = Arc::new(JsonRpcPluginProcess::spawn(&package.backend_entry, &[]).await?);
            let handshake = RuntimeHello {
                interface_major: coterm_proto::runtime::lifecycle::INTERFACE_MAJOR,
                interface_minor: coterm_proto::runtime::lifecycle::INTERFACE_MINOR,
                required_features: coterm_proto::runtime::lifecycle::REQUIRED_FEATURES
                    .iter()
                    .map(|f| (*f).to_owned())
                    .collect(),
                optional_features: coterm_proto::runtime::lifecycle::OPTIONAL_FEATURES
                    .iter()
                    .map(|f| (*f).to_owned())
                    .collect(),
                limits: self.limits.clone(),
            };
            let result = process
                .request(HostCommand::Hello(handshake.clone()))
                .await
                .and_then(|value| {
                    serde_json::from_value::<coterm_proto::runtime::lifecycle::HelloResult>(value)
                        .map_err(|error| {
                            runtime_error_with(
                                ErrorCategory::Transport,
                                "plugin_handshake_invalid",
                                error.to_string(),
                            )
                        })
                });
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    let _ = process.close().await;
                    return Err(error);
                }
            };
            if result.plugin_id != package.manifest.id
                || result.plugin_version != package.manifest.version
                || !package
                    .manifest
                    .runtime
                    .requires
                    .iter()
                    .all(|f| result.enabled_features.contains(f))
                || handshake.validate_result(&result).is_err()
            {
                let _ = process.close().await;
                return Err(runtime_error(
                    ErrorCategory::Unsupported,
                    "plugin_handshake_incompatible",
                ));
            }
            process.set_control_limit(result.limits.max_control_bytes);
            let mut catalog = HashMap::new();
            let mut names = HashSet::new();
            for descriptor in result.operations {
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
                    let _ = process.close().await;
                    return Err(runtime_error(
                        ErrorCategory::InvalidRequest,
                        "plugin_operation_catalog_invalid",
                    ));
                }
                catalog.insert(descriptor.name.clone(), descriptor);
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
                    .is_some_and(|required| {
                        required.iter().any(|value| value == "pluginInstanceId")
                    });
                if name.is_empty()
                    || name.len() > 64
                    || plugin_id.len() + 2 + name.len() > 128
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                    || !tool_names.insert(name)
                    || operation_scope.is_none()
                    || !result
                        .enabled_features
                        .iter()
                        .any(|f| f == "native-mcp-tools")
                    || has_instance_param != requires_instance
                    || (requires_instance
                        && schema
                            .and_then(|s| s.pointer("/properties/pluginInstanceId/type"))
                            .and_then(Value::as_str)
                            != Some("string"))
                    || schema.and_then(|s| s.get("type")).and_then(Value::as_str) != Some("object")
                    || schema.is_none_or(|schema| jsonschema::validator_for(schema).is_err())
                {
                    let _ = process.close().await;
                    return Err(runtime_error(
                        ErrorCategory::InvalidRequest,
                        "plugin_mcp_catalog_invalid",
                    ));
                }
            }
            state
                .features
                .insert(plugin_id.to_owned(), result.enabled_features);
            state
                .mcp_tools
                .insert(plugin_id.to_owned(), result.mcp_tools);
            state
                .operation_catalog
                .insert(plugin_id.to_owned(), catalog);
            let notifications = process.take_completions().await;
            let process_ref = Arc::downgrade(&process);
            state.processes.insert(plugin_id.to_owned(), process);
            if let Some(mut notifications) = notifications {
                let state_ref = Arc::downgrade(&self.state);
                let history_ref = Arc::downgrade(&self.history);
                let plugin_id = plugin_id.to_owned();
                tokio::spawn(async move {
                    while let Some(notification) = notifications.recv().await {
                        let (Some(state), Some(history)) =
                            (state_ref.upgrade(), history_ref.upgrade())
                        else {
                            break;
                        };
                        // The control reply is committed before a completion can be applied.
                        let _state = state.lock().await;
                        let (id, result) = match notification {
                            PluginNotification::OperationCompleted {
                                operation_id,
                                result,
                            } => (operation_id, Ok(result)),
                            PluginNotification::OperationFailed {
                                operation_id,
                                error,
                            } => (operation_id, Err(error)),
                            _ => continue,
                        };
                        let Ok(record) = history.get(&id).await else {
                            continue;
                        };
                        if record.plugin_id != plugin_id
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
                                    .get(&plugin_id)
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
                    if let (Some(state), Some(history)) =
                        (state_ref.upgrade(), history_ref.upgrade())
                    {
                        let state = state.lock().await;
                        if !state.processes.get(&plugin_id).is_some_and(|current| {
                            process_ref
                                .upgrade()
                                .is_some_and(|process| Arc::ptr_eq(current, &process))
                        }) {
                            return;
                        }
                        for instance in state
                            .instances
                            .values()
                            .filter(|i| i.plugin_id == plugin_id)
                        {
                            for record in history
                                .active(&plugin_id, &instance.plugin_instance_id)
                                .await
                            {
                                if !crate::operation_history::terminal(record.status) {
                                    let _ = history
                                        .transition(
                                            &record.operation_id,
                                            OperationStatus::Unknown,
                                            None,
                                            Some(runtime_error(
                                                ErrorCategory::Transport,
                                                "plugin_connection_closed",
                                            )),
                                        )
                                        .await;
                                }
                            }
                        }
                    }
                });
            }
            Ok(())
        })
    }

    fn stop<'a>(&'a self, plugin_id: &'a str) -> RuntimeFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let Some(process) = state.processes.remove(plugin_id) else {
                return Ok(());
            };
            state.operation_catalog.remove(plugin_id);
            state.mcp_tools.remove(plugin_id);
            state.features.remove(plugin_id);
            let instance_ids = state
                .instances
                .values()
                .filter(|instance| instance.plugin_id == plugin_id)
                .map(|instance| instance.plugin_instance_id.clone())
                .collect::<Vec<_>>();
            let pending_ids = state
                .pending_approvals
                .iter()
                .filter(|(_, pending)| pending.0 == plugin_id)
                .map(|(operation_id, _)| operation_id.clone())
                .collect::<Vec<_>>();
            for operation_id in pending_ids {
                state.pending_approvals.remove(&operation_id);
                let _ = self
                    .history
                    .transition(&operation_id, OperationStatus::Cancelled, None, None)
                    .await;
            }
            self.bump_attention();
            self.cancel_instance_interactions(&mut state, &instance_ids)
                .await;
            let mut first_error = None;
            for instance_id in &instance_ids {
                if let Err(error) = process
                    .request(HostCommand::DestroyInstance {
                        plugin_instance_id: instance_id.clone(),
                    })
                    .await
                {
                    first_error.get_or_insert(error);
                }
            }
            if let Err(error) = process.request(HostCommand::Shutdown).await {
                first_error.get_or_insert(error);
            }
            if let Err(error) = process.close().await {
                first_error.get_or_insert(error);
            }
            for instance_id in instance_ids {
                state.instances.remove(&instance_id);
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }

    fn create_instance<'a>(
        &'a self,
        plugin_id: &'a str,
        config: Value,
    ) -> RuntimeFuture<'a, InstanceInfo> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let package = state
                .packages
                .get(plugin_id)
                .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "plugin_not_loaded"))?;
            if !package.manifest.instances.multiple {
                return Err(runtime_error(
                    ErrorCategory::Unsupported,
                    "plugin_multi_instance_unsupported",
                ));
            }
            let process = state
                .processes
                .get(plugin_id)
                .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
            let instance_id = uuid::Uuid::new_v4().to_string();
            let instance = InstanceInfo {
                plugin_id: plugin_id.to_owned(),
                plugin_instance_id: instance_id.clone(),
                session_id: uuid::Uuid::new_v4().to_string(),
                label: format!("{} {instance_id}", package.manifest.name),
                status: InstanceStatus::Starting,
            };
            let created = process
                .request(HostCommand::CreateInstance(CreateInstance {
                    instance: instance.clone(),
                    config,
                }))
                .await?;
            let ready = InstanceInfo {
                label: created
                    .get("label")
                    .and_then(Value::as_str)
                    .filter(|label| {
                        !label.trim().is_empty()
                            && label.len() <= 256
                            && !label.chars().any(char::is_control)
                    })
                    .map(str::to_owned)
                    .unwrap_or(instance.label),
                status: InstanceStatus::Ready,
                ..instance
            };
            state.instances.insert(instance_id, ready.clone());
            Ok(ready)
        })
    }

    fn destroy_instance<'a>(&'a self, instance_id: &'a str) -> RuntimeFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let instance = state.instances.get(instance_id).cloned().ok_or_else(|| {
                runtime_error(ErrorCategory::NotFound, "plugin_instance_not_found")
            })?;
            let process = state
                .processes
                .get(&instance.plugin_id)
                .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
            process
                .request(HostCommand::DestroyInstance {
                    plugin_instance_id: instance_id.to_owned(),
                })
                .await?;
            self.cancel_instance_interactions(&mut state, &[instance_id.to_owned()])
                .await;
            state.instances.remove(instance_id);
            Ok(())
        })
    }

    fn list_instances<'a>(
        &'a self,
        plugin_id: Option<&'a str>,
    ) -> RuntimeFuture<'a, Vec<InstanceInfo>> {
        Box::pin(async move {
            let state = self.state.lock().await;
            let mut instances = state
                .instances
                .values()
                .filter(|instance| plugin_id.is_none_or(|id| instance.plugin_id == id))
                .cloned()
                .collect::<Vec<_>>();
            instances.sort_by(|left, right| {
                left.plugin_id
                    .cmp(&right.plugin_id)
                    .then(left.plugin_instance_id.cmp(&right.plugin_instance_id))
            });
            Ok(instances)
        })
    }
}

impl Operations for ProcessPluginRuntime {
    fn submit<'a>(
        &'a self,
        caller: CallerContext,
        request: SubmitOperation,
    ) -> RuntimeFuture<'a, OperationReceipt> {
        Box::pin(async move {
            let bypass = *self.bypass.read().await;
            let state = self.state.lock().await;
            let descriptor = state
                .operation_catalog
                .get(&request.plugin_id)
                .and_then(|catalog| catalog.get(&request.operation))
                .cloned()
                .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "operation_not_found"))?;
            validate_scope_target(descriptor.scope, request.plugin_instance_id.is_some())?;
            if descriptor.scope == OperationScope::Plugin {
                validate_plugin_authority(&caller, &request.plugin_id)?;
            }
            if let Some(instance_id) = request.plugin_instance_id.as_deref() {
                let instance = state.instances.get(instance_id).ok_or_else(|| {
                    runtime_error(ErrorCategory::NotFound, "plugin_instance_not_found")
                })?;
                if instance.plugin_id != request.plugin_id {
                    return Err(runtime_error(
                        ErrorCategory::Conflict,
                        "plugin_instance_mismatch",
                    ));
                }
                if instance.session_id != caller.session_id {
                    return Err(runtime_error(ErrorCategory::Conflict, "session_mismatch"));
                }
            }
            validate_operation_args(
                &state,
                &request.plugin_id,
                &request.operation,
                &request.args,
            )?;
            if !state.processes.contains_key(&request.plugin_id) {
                return Err(runtime_error(ErrorCategory::Conflict, "plugin_not_started"));
            }
            let operation_id = uuid::Uuid::new_v4().to_string();
            let proposal_id = requires_proposal(caller.source, descriptor.effect, bypass)
                .then(|| uuid::Uuid::new_v4().to_string());
            let now = Utc::now();
            let record = OperationRecord {
                operation_id: operation_id.clone(),
                plugin_id: request.plugin_id.clone(),
                plugin_instance_id: request.plugin_instance_id.clone(),
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
                let mut state = runtime.state.lock().await;
                if !runtime
                    .history
                    .get(&operation_id)
                    .await
                    .is_ok_and(|r| r.status == OperationStatus::Accepted)
                {
                    return;
                }
                if request
                    .plugin_instance_id
                    .as_ref()
                    .is_some_and(|id| !state.instances.contains_key(id))
                {
                    let _ = runtime
                        .history
                        .transition(&operation_id, OperationStatus::Cancelled, None, None)
                        .await;
                    return;
                }
                let Some(process) = state.processes.get(&request.plugin_id).cloned() else {
                    let _ = runtime
                        .history
                        .transition(
                            &operation_id,
                            OperationStatus::Failed,
                            None,
                            Some(runtime_error(ErrorCategory::Conflict, "plugin_not_started")),
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
                let mut plugin_id = request.plugin_id;
                let mut plugin_instance_id = request.plugin_instance_id;
                let operation_name = request.operation.clone();
                let result = process
                    .request(HostCommand::Invoke(InvokeRequest {
                        operation_id: operation_id.clone(),
                        plugin_instance_id: plugin_instance_id.clone(),
                        source: caller.source,
                        effect: descriptor.effect,
                        operation: request.operation,
                        args: request.args,
                        dispatch,
                    }))
                    .await;
                if let Some(proposal_id) = proposal_id {
                    match result {
                        Ok(draft) => {
                            state.pending_approvals.insert(
                                operation_id.clone(),
                                (
                                    std::mem::take(&mut plugin_id),
                                    plugin_instance_id.take(),
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
                                .transition(
                                    &operation_id,
                                    OperationStatus::Failed,
                                    None,
                                    Some(error),
                                )
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
            let final_result =
                tokio::time::timeout(std::time::Duration::from_millis(wait_ms), async {
                    loop {
                        let record = updates.borrow_and_update().clone();
                        if crate::operation_history::terminal(record.status) {
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
        })
    }

    fn get<'a>(
        &'a self,
        caller: CallerContext,
        operation_id: &'a str,
    ) -> RuntimeFuture<'a, OperationRecord> {
        Box::pin(async move {
            let record = self
                .history
                .get(operation_id)
                .await
                .map_err(map_history_error)?;
            verify_record_access(&self.state, &caller, &record).await?;
            Ok(record)
        })
    }

    fn recent<'a>(
        &'a self,
        caller: CallerContext,
        instance_id: &'a str,
        limit: usize,
    ) -> RuntimeFuture<'a, Vec<OperationRecord>> {
        Box::pin(async move {
            let state = self.state.lock().await;
            let instance = state.instances.get(instance_id).ok_or_else(|| {
                runtime_error(ErrorCategory::NotFound, "plugin_instance_not_found")
            })?;
            if instance.session_id != caller.session_id {
                return Err(runtime_error(ErrorCategory::Conflict, "session_mismatch"));
            }
            Ok(self
                .history
                .recent(&instance.plugin_id, Some(instance_id), limit)
                .await)
        })
    }

    fn approve<'a>(
        &'a self,
        caller: CallerContext,
        request: ApproveProposal,
    ) -> RuntimeFuture<'a, OperationReceipt> {
        Box::pin(async move {
            if caller.source != OperationSource::Human {
                return Err(runtime_error(
                    ErrorCategory::Conflict,
                    "human_approval_required",
                ));
            }
            let mut state = self.state.lock().await;
            let pending = state
                .pending_approvals
                .get(&request.operation_id)
                .cloned()
                .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "proposal_not_found"))?;
            if pending.2 != request.proposal_id || pending.1 != request.plugin_instance_id {
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
                validate_plugin_authority(&caller, &pending.0)?;
            }
            let record = self
                .history
                .get(&request.operation_id)
                .await
                .map_err(map_history_error)?;
            validate_operation_args(&state, &pending.0, &record.operation, &request.args)?;
            let process = state
                .processes
                .get(&pending.0)
                .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
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
            let result = process.request(HostCommand::Approve(request.clone())).await;
            if state.pending_approvals.remove(&request.operation_id).is_some() {
                self.bump_attention();
            }
            let record = self
                .finish_reply(&mut state, &request.operation_id, result)
                .await?;
            Ok(receipt(&record))
        })
    }

    fn reject<'a>(
        &'a self,
        caller: CallerContext,
        operation_id: &'a str,
        proposal_id: &'a str,
    ) -> RuntimeFuture<'a, ()> {
        Box::pin(async move {
            if caller.source != OperationSource::Human {
                return Err(runtime_error(
                    ErrorCategory::Conflict,
                    "human_rejection_required",
                ));
            }
            let mut state = self.state.lock().await;
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
                validate_plugin_authority(&caller, &pending.0)?;
            }
            let process = state
                .processes
                .get(&pending.0)
                .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
            process
                .request(HostCommand::Reject {
                    operation_id: operation_id.to_owned(),
                    proposal_id: proposal_id.to_owned(),
                    reason: None,
                })
                .await?;
            state.pending_approvals.remove(operation_id);
            self.bump_attention();
            self.history
                .transition(operation_id, OperationStatus::Rejected, None, None)
                .await
                .map_err(map_history_error)?;
            Ok(())
        })
    }

    fn cancel<'a>(
        &'a self,
        caller: CallerContext,
        operation_id: &'a str,
    ) -> RuntimeFuture<'a, OperationReceipt> {
        Box::pin(async move {
            let record = self.get(caller, operation_id).await?;
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
                    self.resume_interaction(&mut state, request, InteractionOutcome::Cancelled)
                        .await?;
                }
            } else if let Some((plugin_id, _, proposal_id, _)) =
                state.pending_approvals.get(operation_id).cloned()
            {
                let process = state
                    .processes
                    .get(&plugin_id)
                    .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
                let result = process
                    .request(HostCommand::Reject {
                        operation_id: operation_id.to_owned(),
                        proposal_id,
                        reason: Some("Cancelled by caller".to_owned()),
                    })
                    .await;
                state.pending_approvals.remove(operation_id);
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
                let process = state
                    .processes
                    .get(&record.plugin_id)
                    .ok_or_else(|| runtime_error(ErrorCategory::Conflict, "plugin_not_started"))?;
                let response = process
                    .request(HostCommand::Cancel(
                        coterm_proto::runtime::message::CancelTarget::Operation(
                            operation_id.to_owned(),
                        ),
                    ))
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
        })
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
    let Some(instance_id) = record.plugin_instance_id.as_deref() else {
        return if caller.plugin_id.as_deref() == Some(record.plugin_id.as_str()) {
            Ok(())
        } else {
            Err(runtime_error(
                ErrorCategory::Conflict,
                "plugin_scope_access_denied",
            ))
        };
    };
    let state = state.lock().await;
    let instance = state
        .instances
        .get(instance_id)
        .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "plugin_instance_not_found"))?;
    if instance.session_id != caller.session_id {
        return Err(runtime_error(ErrorCategory::Conflict, "session_mismatch"));
    }
    Ok(())
}

fn validate_plugin_authority(caller: &CallerContext, plugin_id: &str) -> Result<(), RuntimeError> {
    if caller.plugin_id.as_deref() == Some(plugin_id) {
        Ok(())
    } else {
        Err(runtime_error(
            ErrorCategory::Conflict,
            "plugin_scope_access_denied",
        ))
    }
}

fn validate_scope_target(scope: OperationScope, has_instance: bool) -> Result<(), RuntimeError> {
    match scope {
        OperationScope::Instance if !has_instance => Err(runtime_error(
            ErrorCategory::InvalidRequest,
            "plugin_instance_required",
        )),
        OperationScope::Plugin if has_instance => Err(runtime_error(
            ErrorCategory::InvalidRequest,
            "plugin_operation_does_not_accept_instance",
        )),
        _ => Ok(()),
    }
}

fn validate_operation_args(
    state: &State,
    plugin_id: &str,
    operation: &str,
    args: &Value,
) -> Result<(), RuntimeError> {
    let schema = state
        .mcp_tools
        .get(plugin_id)
        .and_then(|tools| tools.iter().find(|tool| tool.operation == operation))
        .and_then(|tool| tool.descriptor.get("inputSchema"))
        .or_else(|| {
            state
                .operation_catalog
                .get(plugin_id)
                .and_then(|catalog| catalog.get(operation))
                .map(|descriptor| &descriptor.input_schema)
        })
        .ok_or_else(|| runtime_error(ErrorCategory::NotFound, "operation_not_found"))?;
    validate_schema(schema, args).map_err(|_| {
        runtime_error_with(
            ErrorCategory::InvalidRequest,
            "operation_args_invalid",
            "Operation arguments do not match the registered schema".to_owned(),
        )
    })
}

fn schema_is_valid(schema: &Value, value: &Value) -> bool {
    jsonschema::validator_for(schema).is_ok_and(|validator| validator.is_valid(value))
}

fn validate_schema(schema: &Value, value: &Value) -> Result<(), ()> {
    let validator = jsonschema::validator_for(schema).map_err(|_| ())?;
    validator.validate(value).map_err(|_| ())
}

fn requires_proposal(source: OperationSource, effect: OperationEffect, bypass: bool) -> bool {
    source == OperationSource::Agent && effect == OperationEffect::Commit && !bypass
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

fn runtime_error_with(category: ErrorCategory, code: &str, message: String) -> RuntimeError {
    RuntimeError {
        category,
        code: code.to_owned(),
        message,
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use coterm_proto::runtime::operation::{OperationEffect, OperationSource};

    use super::{
        failure_status, requires_proposal, schema_is_valid, validate_plugin_authority,
        validate_scope_target, ProcessPluginRuntime,
    };
    use coterm_proto::runtime::interaction::{InteractionKind, InteractionRequest};
    use coterm_proto::runtime::lifecycle::{InstanceInfo, InstanceStatus, RuntimeLimits};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;

    fn runtime() -> ProcessPluginRuntime {
        ProcessPluginRuntime::new(
            "windows-x64",
            RuntimeLimits {
                max_control_bytes: 1024 * 1024,
                max_data_chunk_bytes: 64 * 1024,
                initial_window: coterm_proto::runtime::stream::FlowWindow {
                    chunks: 16,
                    bytes: 1024 * 1024,
                },
            },
        )
    }

    #[tokio::test]
    async fn invalid_deferred_acceptance_is_unknown_after_dispatch() {
        use coterm_proto::runtime::operation::{OperationRecord, OperationStatus};
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
                    plugin_id: "plugin".to_owned(),
                    plugin_instance_id: Some("instance".to_owned()),
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
                    .insert("plugin".to_owned(), vec!["async-operations".to_owned()]);
            }
            let record = runtime
                .finish_reply(&mut state, "op", Ok(reply))
                .await
                .unwrap();
            assert_eq!(record.status, OperationStatus::Unknown);
            assert_eq!(record.error.unwrap().code, "invalid_deferred_operation");
        }
    }

    fn instance(plugin_id: &str, instance_id: &str) -> InstanceInfo {
        InstanceInfo {
            plugin_id: plugin_id.to_owned(),
            plugin_instance_id: instance_id.to_owned(),
            session_id: format!("session-{instance_id}"),
            label: format!("{plugin_id} {instance_id}"),
            status: InstanceStatus::Ready,
        }
    }

    fn secret_request(id: &str, instance_id: &str, timeout_ms: Option<u64>) -> InteractionRequest {
        InteractionRequest {
            interaction_id: id.to_owned(),
            plugin_instance_id: instance_id.to_owned(),
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
            state
                .instances
                .insert("instance-a".to_owned(), instance("com.example.alpha", "instance-a"));
            state
                .instances
                .insert("instance-b".to_owned(), instance("com.example.alpha", "instance-b"));
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
            // 插件级审批：没有实例目标，目标标签回落到插件 ID。
            state.pending_approvals.insert(
                "op-3".to_owned(),
                (
                    "com.example.beta".to_owned(),
                    None,
                    "proposal-op-3".to_owned(),
                    "plugin.configure".to_owned(),
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
        assert_eq!(snapshot.approvals[0].target_label, "com.example.alpha instance-a");
        assert_eq!(snapshot.approvals[2].plugin_instance_id, None);
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
            plugin_id: "com.example.alpha".to_owned(),
            plugin_instance_id: Some("instance-a".to_owned()),
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
                "operation",
                "operationId",
                "pluginId",
                "pluginInstanceId",
                "proposalId",
                "targetLabel",
            ]
        );
    }

    #[test]
    fn interaction_deadline_is_optional_on_the_wire() {
        // 插件发来的请求不含 deadline；基座在受理时补齐。
        let request: InteractionRequest = serde_json::from_value(json!({
            "interactionId": "interaction-1",
            "pluginInstanceId": "instance-a",
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

    #[test]
    fn agent_commit_requires_approval_unless_bypass_is_enabled() {
        assert!(requires_proposal(
            OperationSource::Agent,
            OperationEffect::Commit,
            false
        ));
        assert!(!requires_proposal(
            OperationSource::Agent,
            OperationEffect::Commit,
            true
        ));
        assert!(!requires_proposal(
            OperationSource::Human,
            OperationEffect::Commit,
            false
        ));
        assert!(!requires_proposal(
            OperationSource::Agent,
            OperationEffect::Draft,
            false
        ));
    }

    #[test]
    fn transport_failure_after_dispatch_is_unknown_not_failed() {
        let transport = super::runtime_error(
            coterm_proto::runtime::ErrorCategory::Transport,
            "plugin_disconnected",
        );
        let domain = super::runtime_error(
            coterm_proto::runtime::ErrorCategory::Domain,
            "device_rejected_write",
        );
        assert_eq!(
            failure_status(&transport),
            coterm_proto::runtime::operation::OperationStatus::Unknown
        );
        assert_eq!(
            failure_status(&domain),
            coterm_proto::runtime::operation::OperationStatus::Failed
        );
    }

    #[test]
    fn plugin_scope_has_no_instance_target_and_instance_scope_requires_one() {
        use coterm_proto::runtime::operation::OperationScope;

        assert!(validate_scope_target(OperationScope::Plugin, false).is_ok());
        assert_eq!(
            validate_scope_target(OperationScope::Plugin, true)
                .expect_err("plugin operation cannot accept an instance target")
                .code,
            "plugin_operation_does_not_accept_instance"
        );
        assert_eq!(
            validate_scope_target(OperationScope::Instance, false)
                .expect_err("instance operation requires a target")
                .code,
            "plugin_instance_required"
        );
    }

    #[test]
    fn plugin_scope_authority_is_bound_to_the_routed_plugin() {
        use crate::plugin_runtime::CallerContext;
        use coterm_proto::runtime::operation::OperationSource;

        let caller = CallerContext {
            source: OperationSource::Agent,
            session_id: String::new(),
            plugin_id: Some("com.example.alpha".to_owned()),
        };
        assert!(validate_plugin_authority(&caller, "com.example.alpha").is_ok());
        assert_eq!(
            validate_plugin_authority(&caller, "com.example.beta")
                .expect_err("cross-plugin routing must be denied")
                .code,
            "plugin_scope_access_denied"
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

    mod discovery {
        use super::super::{DiscoveredPackage, PackageOrigin, PluginLoadIssue};
        use super::runtime;
        use std::path::{Path, PathBuf};
        use std::time::{SystemTime, UNIX_EPOCH};
        use tokio::fs;

        /// 每个测试一个独立目录；扫描根就是该目录。
        fn scratch(name: &str) -> PathBuf {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("coterm-discovery-{name}-{unique}"));
            std::fs::create_dir_all(&root).expect("scratch root");
            root
        }

        fn valid_manifest(id: &str, name: &str) -> String {
            serde_json::json!({
                "manifestVersion": 1,
                "id": id,
                "name": name,
                "version": "0.1.0",
                "runtime": {"interfaceMajor": 1, "requires": ["instances"]},
                // 入口文件在这些测试里都真实存在，只做声明校验。
                "backend": {"windows-x64": "backend/agent", "linux-x64": "backend/agent"},
                "mcp": {"provider": "backend"},
                "instances": {"type": "agent", "multiple": true},
                "capabilities": []
            })
            .to_string()
        }

        async fn write_package(root: &Path, directory: &str, manifest: Option<&str>) {
            let package = root.join(directory);
            fs::create_dir_all(package.join("backend"))
                .await
                .expect("package dir");
            // 两个平台入口都写成普通文件，避免测试依赖宿主机平台。
            fs::write(package.join("backend/agent"), b"")
                .await
                .expect("entry");
            if let Some(manifest) = manifest {
                fs::write(package.join("manifest.json"), manifest)
                    .await
                    .expect("manifest");
            }
        }

        fn discovered(root: &Path, directory: &str, priority: u8) -> DiscoveredPackage {
            DiscoveredPackage {
                directory: directory.to_owned(),
                root: root.join(directory),
                priority,
                origin: PackageOrigin::Installed,
            }
        }

        fn codes(issues: &[PluginLoadIssue]) -> Vec<(&str, &str)> {
            issues
                .iter()
                .map(|issue| (issue.directory.as_str(), issue.code.as_str()))
                .collect()
        }

        #[tokio::test]
        async fn one_broken_package_does_not_hide_healthy_plugins() {
            let root = scratch("mixed");
            write_package(&root, "alpha", Some(&valid_manifest("com.example.alpha", "Alpha"))).await;
            write_package(&root, "broken", Some("{ not json")).await;
            write_package(&root, "omega", Some(&valid_manifest("com.example.omega", "Omega"))).await;
            // 缺少 manifest.json 的包应有诊断，而不是静默当成有效插件。
            write_package(&root, "unbuilt", None).await;

            let runtime = runtime();
            let catalog = runtime
                .discover(vec![
                    discovered(&root, "omega", 0),
                    discovered(&root, "broken", 0),
                    discovered(&root, "unbuilt", 0),
                    discovered(&root, "alpha", 0),
                ])
                .await;

            assert_eq!(
                catalog
                    .plugins
                    .iter()
                    .map(|plugin| plugin.id.as_str())
                    .collect::<Vec<_>>(),
                ["com.example.alpha", "com.example.omega"]
            );
            assert_eq!(
                codes(&catalog.issues),
                [
                    ("broken", "plugin_manifest_invalid"),
                    ("unbuilt", "plugin_manifest_unavailable"),
                ]
            );
            // 诊断消息不含绝对路径或原始解析错误。
            for issue in &catalog.issues {
                assert!(!issue.message.contains(&root.to_string_lossy().to_string()));
                assert!(!issue.message.contains('{'));
            }
            fs::remove_dir_all(&root).await.ok();
        }

        #[tokio::test]
        async fn legacy_manifest_is_diagnosed_not_listed() {
            let root = scratch("legacy");
            // 旧格式：有 protocol/entry_point，没有 manifestVersion。
            let legacy = serde_json::json!({
                "id": "coterm-plugin-serial",
                "name": "Serial Port",
                "version": "0.1.0",
                "protocol": "serial",
                "entry_point": "coterm-plugin-serial"
            })
            .to_string();
            write_package(&root, "serial", Some(&legacy)).await;

            let catalog = runtime()
                .discover(vec![discovered(&root, "serial", 0)])
                .await;
            assert!(catalog.plugins.is_empty());
            assert_eq!(catalog.issues.len(), 1);
            assert_eq!(catalog.issues[0].code, "plugin_manifest_legacy");
            // 能解析出 ID 时给出，便于用户定位是哪个插件。
            assert_eq!(
                catalog.issues[0].plugin_id.as_deref(),
                Some("coterm-plugin-serial")
            );
            fs::remove_dir_all(&root).await.ok();
        }

        #[tokio::test]
        async fn unsupported_schema_is_isolated_with_a_stable_code() {
            let root = scratch("schema");
            let mismatched = serde_json::json!({
                "manifestVersion": 2,
                "id": "com.example.future",
                "name": "Future",
                "version": "0.1.0",
                "runtime": {"interfaceMajor": 1, "requires": []},
                "backend": {"linux-x64": "backend/agent"},
                "mcp": {"provider": "backend"},
                "instances": {"type": "future", "multiple": true},
                "capabilities": []
            })
            .to_string();
            write_package(&root, "future", Some(&mismatched)).await;
            write_package(&root, "alpha", Some(&valid_manifest("com.example.alpha", "Alpha"))).await;

            let catalog = runtime()
                .discover(vec![
                    discovered(&root, "future", 0),
                    discovered(&root, "alpha", 0),
                ])
                .await;
            assert_eq!(catalog.plugins.len(), 1);
            assert_eq!(catalog.plugins[0].id, "com.example.alpha");
            assert_eq!(
                codes(&catalog.issues),
                [("future", "plugin_manifest_version_unsupported")]
            );
            assert_eq!(catalog.issues[0].plugin_id.as_deref(), Some("com.example.future"));
            fs::remove_dir_all(&root).await.ok();
        }

        #[tokio::test]
        async fn duplicate_ids_keep_the_first_package_and_diagnose_the_rest() {
            let root = scratch("duplicate");
            // 同一个 ID 出现在两个目录：两者都有效，后者必须被诊断为冲突。
            write_package(&root, "alpha-a", Some(&valid_manifest("com.example.alpha", "Alpha"))).await;
            write_package(&root, "alpha-b", Some(&valid_manifest("com.example.alpha", "Alpha Again"))).await;

            let catalog = runtime()
                .discover(vec![
                    discovered(&root, "alpha-a", 0),
                    discovered(&root, "alpha-b", 0),
                ])
                .await;
            assert_eq!(catalog.plugins.len(), 1);
            assert_eq!(catalog.plugins[0].name, "Alpha");
            assert_eq!(
                codes(&catalog.issues),
                [("alpha-b", "plugin_id_conflict")]
            );
            fs::remove_dir_all(&root).await.ok();
        }

        #[tokio::test]
        async fn listing_is_deterministic_regardless_of_scan_order() {
            let root = scratch("order");
            for (directory, id, name) in [
                ("zeta", "com.example.zeta", "Zeta"),
                ("alpha", "com.example.alpha", "Alpha"),
                ("mid", "com.example.beta", "Beta"),
            ] {
                write_package(&root, directory, Some(&valid_manifest(id, name))).await;
            }
            let forward = runtime()
                .discover(vec![
                    discovered(&root, "zeta", 0),
                    discovered(&root, "alpha", 0),
                    discovered(&root, "mid", 0),
                ])
                .await;
            let reverse = {
                let runtime = runtime();
                runtime
                    .discover(vec![
                        discovered(&root, "mid", 0),
                        discovered(&root, "alpha", 0),
                        discovered(&root, "zeta", 0),
                    ])
                    .await
            };
            let names = |catalog: &super::super::PluginList| {
                catalog
                    .plugins
                    .iter()
                    .map(|plugin| plugin.name.clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(names(&forward), ["Alpha", "Beta", "Zeta"]);
            assert_eq!(names(&forward), names(&reverse));
            fs::remove_dir_all(&root).await.ok();
        }

        #[tokio::test]
        async fn empty_directory_lists_nothing_and_reports_no_errors() {
            let root = scratch("empty");
            let catalog = runtime().discover(Vec::new()).await;
            assert!(catalog.plugins.is_empty());
            assert!(catalog.issues.is_empty());
            fs::remove_dir_all(&root).await.ok();
        }

        #[tokio::test]
        async fn entry_outside_the_package_is_rejected() {
            let root = scratch("escape");
            let escaping = serde_json::json!({
                "manifestVersion": 1,
                "id": "com.example.escape",
                "name": "Escape",
                "version": "0.1.0",
                "runtime": {"interfaceMajor": 1, "requires": []},
                "backend": {"windows-x64": "../../outside/agent"},
                "mcp": {"provider": "backend"},
                "instances": {"type": "escape", "multiple": true},
                "capabilities": []
            })
            .to_string();
            write_package(&root, "escape", Some(&escaping)).await;

            let catalog = runtime()
                .discover(vec![discovered(&root, "escape", 0)])
                .await;
            assert!(catalog.plugins.is_empty());
            assert_eq!(codes(&catalog.issues), [("escape", "plugin_entry_invalid")]);
            fs::remove_dir_all(&root).await.ok();
        }

        #[test]
        fn diagnostics_serialize_without_paths() {
            let issue = PluginLoadIssue {
                directory: "broken".to_owned(),
                plugin_id: None,
                code: "plugin_manifest_invalid".to_owned(),
                message: "Package manifest is not valid JSON".to_owned(),
            };
            let value = serde_json::to_value(&issue).expect("issue serializes");
            let keys = value
                .as_object()
                .expect("object")
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(keys, ["code", "directory", "message", "pluginId"]);
            assert_eq!(value["pluginId"], serde_json::Value::Null);
        }
    }
}
