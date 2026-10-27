// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod workspace;
mod workspace_commands;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use coterm_core::{
    plugin_manager::{DiscoveredPackage, PackageOrigin, PluginLoadIssue, ProcessPluginRuntime},
    plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
    SessionManager,
};
use coterm_proto::runtime::{
    lifecycle::{InstanceInfo, RuntimeLimits},
    manifest::PluginManifest,
    operation::{ApproveProposal, OperationDescriptor, OperationSource},
    stream::FlowWindow,
};
use serde::Serialize;
use serde_json::Value;
use tauri::{Emitter, Manager};

/// 宿主待处理状态变化的全局事件名；payload 只有版本号。
const ATTENTION_CHANGED_EVENT: &str = "host://attention-changed";

#[derive(Clone, Serialize)]
struct AttentionChanged {
    version: u64,
}

type EventSubscriptions =
    Arc<tokio::sync::Mutex<std::collections::HashMap<String, EventSubscription>>>;
struct EventSubscription {
    instance_id: String,
    ack: tokio::sync::mpsc::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Serialize)]
struct InstanceEvent {
    name: String,
    payload: Value,
}

struct AppState {
    workspace: Arc<
        std::sync::Mutex<Result<workspace::WorkspaceStore, coterm_proto::runtime::RuntimeError>>,
    >,
    workspace_actions: tokio::sync::Mutex<()>,
    workspace_path: PathBuf,
    exiting: std::sync::atomic::AtomicBool,
    event_subscriptions: EventSubscriptions,
    _session_mgr: Arc<SessionManager>,
    plugin_runtime: Arc<ProcessPluginRuntime>,
    plugins_dir: PathBuf,
    mcp_server: Result<coterm_mcp::http::HttpBridgeServer, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginSummary {
    id: String,
    name: String,
    version: String,
    has_frontend: bool,
    running: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginCatalog {
    plugins: Vec<PluginSummary>,
    /// 逐包诊断：多个坏包不会让整次发现失败，也不会隐藏其余有效插件。
    issues: Vec<PluginLoadIssue>,
}

/// 列出扫描根下的子目录，只保留标准目录名；不跟随符号链接、不暴露绝对路径。
async fn scan_packages(
    root: &Path,
    origin: PackageOrigin,
    priority: u8,
    scanned: &mut Vec<DiscoveredPackage>,
    issues: &mut Vec<PluginLoadIssue>,
) {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            issues.push(PluginLoadIssue {
                directory: String::new(),
                plugin_id: None,
                code: "plugin_search_unavailable".to_owned(),
                message: match origin {
                    PackageOrigin::Development => "Development plugin directory is unavailable",
                    PackageOrigin::Installed => "Installed plugin directory is unavailable",
                }
                .to_owned(),
            });
            return;
        }
    };
    let mut names = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        // symlink 跳过：避免从插件目录外加载入口。
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        names.push(name);
    }
    // 排序保证扫描顺序与文件系统返回顺序无关。
    names.sort();
    for name in names {
        scanned.push(DiscoveredPackage {
            root: root.join(&name),
            directory: name,
            priority,
            origin,
        });
    }
}

/// JSON-event MVP: one unacknowledged delivery per subscriber, bounded replay in plugin.
#[tauri::command]
async fn subscribe_plugin_events(
    plugin_instance_id: String,
    on_event: tauri::ipc::Channel<InstanceEvent>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let mut events = state
        .plugin_runtime
        .subscribe_events(&plugin_instance_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut subscriptions = state.event_subscriptions.lock().await;
    if subscriptions.len() >= 32 {
        return Err("Too many event subscriptions".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let (ack, mut acknowledged) = tokio::sync::mpsc::channel(1);
    let instance = plugin_instance_id.clone();
    let cleanup = Arc::downgrade(&state.event_subscriptions);
    let subscription_id = id.clone();
    let task = tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(coterm_proto::runtime::message::PluginNotification::Event {
                    plugin_instance_id,
                    name,
                    payload,
                }) if plugin_instance_id == instance
                    || (plugin_instance_id.is_empty() && name == "runtime.closed") =>
                {
                    InstanceEvent { name, payload }
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => InstanceEvent {
                    name: "runtime.gap".into(),
                    payload: Value::Null,
                },
                Err(_) => break,
            };
            let closed = event.name == "runtime.closed";
            if on_event.send(event).is_err() {
                break;
            }
            // No second event enters the WebView until the consumer finishes this one.
            if !matches!(
                tokio::time::timeout(std::time::Duration::from_secs(30), acknowledged.recv()).await,
                Ok(Some(()))
            ) {
                let _ = on_event.send(InstanceEvent {
                    name: "runtime.closed".into(),
                    payload: serde_json::json!({"reason":"consumer_timeout"}),
                });
                break;
            }
            if closed {
                break;
            }
        }
        if let Some(subscriptions) = cleanup.upgrade() {
            subscriptions.lock().await.remove(&subscription_id);
        }
    });
    subscriptions.insert(
        id.clone(),
        EventSubscription {
            instance_id: plugin_instance_id,
            ack,
            task,
        },
    );
    Ok(id)
}

#[tauri::command]
async fn ack_plugin_event(
    plugin_instance_id: String,
    subscription_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let subscriptions = state.event_subscriptions.lock().await;
    let subscription = subscriptions
        .get(&subscription_id)
        .filter(|s| s.instance_id == plugin_instance_id)
        .ok_or("Event subscription not found")?;
    subscription
        .ack
        .try_send(())
        .map_err(|_| "Event already acknowledged".to_owned())
}

#[tauri::command]
async fn unsubscribe_plugin_events(
    plugin_instance_id: String,
    subscription_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut subscriptions = state.event_subscriptions.lock().await;
    if subscriptions
        .get(&subscription_id)
        .is_some_and(|s| s.instance_id == plugin_instance_id)
    {
        if let Some(subscription) = subscriptions.remove(&subscription_id) {
            subscription.task.abort();
        }
    }
    Ok(())
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn list_plugins(state: tauri::State<'_, AppState>) -> Result<PluginCatalog, String> {
    // 已安装包优先；开发包随后扫描并记诊断，绝不覆盖已安装的同 ID 包。
    let mut scanned = Vec::new();
    let mut issues = Vec::new();
    scan_packages(
        &state.plugins_dir,
        PackageOrigin::Installed,
        0,
        &mut scanned,
        &mut issues,
    )
    .await;
    // 开发包只在 Windows/macOS/Linux 的开发构建中出现：
    // 路径来自编译期 CARGO_MANIFEST_DIR（apps/desktop/src-tauri），向上三级到仓库根的 plugins/。
    // 该路径只在本进程内用于读取，不写入任何 IPC 负载。
    #[cfg(debug_assertions)]
    {
        let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../plugins");
        scan_packages(
            &development,
            PackageOrigin::Development,
            1,
            &mut scanned,
            &mut issues,
        )
        .await;
        // 开发包位于 <repo>/plugins/<dir>/dist；已安装包直接是 <dir>。
        for package in scanned
            .iter_mut()
            .filter(|p| p.origin == PackageOrigin::Development)
        {
            package.root = package.root.join("dist");
        }
    }
    scanned.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.directory.cmp(&right.directory))
    });
    let catalog = state.plugin_runtime.discover(scanned).await;
    issues.extend(catalog.issues);
    // 只暴露目录名与稳定错误码，不暴露绝对路径。
    let mut plugins = Vec::with_capacity(catalog.plugins.len());
    for manifest in catalog.plugins {
        plugins.push(summary(&state, manifest).await);
    }
    plugins.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    issues.sort_by(|left, right| {
        left.directory
            .cmp(&right.directory)
            .then_with(|| left.code.cmp(&right.code))
    });
    Ok(PluginCatalog { plugins, issues })
}

#[tauri::command]
async fn start_plugin(plugin_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .plugin_runtime
        .start(&plugin_id)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn stop_plugin(plugin_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _guard = state.workspace_actions.lock().await;
    state
        .plugin_runtime
        .stop(&plugin_id)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn list_instances(
    plugin_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstanceInfo>, String> {
    state
        .plugin_runtime
        .list_instances(plugin_id.as_deref())
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeSnapshot {
    instances: Vec<InstanceInfo>,
    running_plugins: Vec<String>,
}

#[tauri::command]
async fn runtime_snapshot(state: tauri::State<'_, AppState>) -> Result<RuntimeSnapshot, String> {
    let instances = state
        .plugin_runtime
        .list_instances(None)
        .await
        .map_err(|error| error.code)?;
    let mut running_plugins = Vec::new();
    for manifest in state.plugin_runtime.loaded_manifests().await {
        if state.plugin_runtime.is_running(&manifest.id).await {
            running_plugins.push(manifest.id);
        }
    }
    Ok(RuntimeSnapshot {
        instances,
        running_plugins,
    })
}

#[tauri::command]
async fn list_operations(
    plugin_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<OperationDescriptor>, String> {
    Ok(state.plugin_runtime.operation_catalog(&plugin_id).await)
}

#[tauri::command]
async fn plugin_frontend_source(
    plugin_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    state
        .plugin_runtime
        .frontend_source(&plugin_id)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

/// Only the trusted desktop secret prompt calls this; never an MCP operation.
#[tauri::command]
async fn provide_secret(
    plugin_instance_id: String,
    purpose: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::interaction::SecretHandle, String> {
    state
        .plugin_runtime
        .provide_secret(&plugin_instance_id, purpose, value)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn pending_interactions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<coterm_proto::runtime::interaction::InteractionRequest>, String> {
    Ok(state.plugin_runtime.pending_interactions().await)
}

/// 宿主级待处理快照；事件只携带版本号，前端据此重取。
#[tauri::command]
async fn host_attention(
    state: tauri::State<'_, AppState>,
) -> Result<coterm_core::plugin_manager::AttentionSnapshot, String> {
    Ok(state.plugin_runtime.attention_snapshot().await)
}

#[tauri::command]
async fn respond_secret(
    interaction_id: String,
    value: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), coterm_proto::runtime::RuntimeError> {
    state
        .plugin_runtime
        .respond_secret(&interaction_id, value)
        .await
}

#[tauri::command]
fn mcp_connection_info(
    state: tauri::State<'_, AppState>,
) -> Result<coterm_mcp::http::HttpConnectionInfo, String> {
    state
        .mcp_server
        .as_ref()
        .map(|server| server.connection_info())
        .map_err(Clone::clone)
}

/// Live MCP sessions for the desktop panel. IDs are prefixes, never full session tokens.
#[tauri::command]
async fn mcp_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<coterm_mcp::http::SessionInfo>, String> {
    match state.mcp_server.as_ref() {
        Ok(server) => Ok(server.sessions().await),
        Err(error) => Err(error.clone()),
    }
}

/// Closes one client session by its prefix so a new client can initialize.
#[tauri::command]
async fn close_mcp_session(
    session_prefix: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    match state.mcp_server.as_ref() {
        Ok(server) if server.close_session(&session_prefix).await => Ok(true),
        Ok(_) => Err("MCP session not found or ambiguous".to_owned()),
        Err(error) => Err(error.clone()),
    }
}

#[tauri::command]
async fn create_instance(
    plugin_id: String,
    config: Value,
    state: tauri::State<'_, AppState>,
) -> Result<InstanceInfo, String> {
    let _guard = state.workspace_actions.lock().await;
    let instance = state
        .plugin_runtime
        .create_instance(&plugin_id, config.clone())
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let id = instance.plugin_instance_id.clone();
    let label = instance.label.clone();
    if let Err(error) = workspace_commands::access(&state, move |store| {
        store.insert(plugin_id, id, label, config)
    })
    .await
    {
        let _ = state
            .plugin_runtime
            .destroy_instance(&instance.plugin_instance_id)
            .await;
        return Err(format!("{}: {}", error.code, error.message));
    }
    Ok(instance)
}

#[tauri::command]
async fn destroy_instance(
    plugin_instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _guard = state.workspace_actions.lock().await;
    state
        .plugin_runtime
        .destroy_instance(&plugin_instance_id)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    workspace_commands::access(&state, move |store| {
        store.remove_instance(&plugin_instance_id)
    })
    .await
    .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn invoke_operation(
    plugin_instance_id: String,
    operation: String,
    args: Value,
    wait_timeout_ms: Option<u64>,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    let instance = state
        .plugin_runtime
        .get_instance(&plugin_instance_id)
        .await
        .ok_or_else(|| "plugin instance not found".to_owned())?;
    state
        .plugin_runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                plugin_id: Some(instance.plugin_id.clone()),
            },
            SubmitOperation {
                plugin_id: instance.plugin_id,
                plugin_instance_id: Some(plugin_instance_id),
                operation,
                args,
                wait_timeout_ms,
            },
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn invoke_plugin_operation(
    plugin_id: String,
    operation: String,
    args: Value,
    wait_timeout_ms: Option<u64>,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    state
        .plugin_runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: String::new(),
                plugin_id: Some(plugin_id.clone()),
            },
            SubmitOperation {
                plugin_id,
                plugin_instance_id: None,
                operation,
                args,
                wait_timeout_ms,
            },
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn recent_plugin_operations(
    plugin_id: String,
    limit: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<coterm_proto::runtime::operation::OperationRecord>, String> {
    Ok(state
        .plugin_runtime
        .recent_plugin_operations(&plugin_id, limit)
        .await)
}

#[tauri::command]
async fn approve_plugin_proposal(
    plugin_id: String,
    proposal_id: String,
    args: Value,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    let operation_id = state
        .plugin_runtime
        .pending_plugin_operation(&plugin_id, &proposal_id)
        .await
        .ok_or_else(|| "pending proposal not found for this plugin".to_owned())?;
    state
        .plugin_runtime
        .approve(
            CallerContext {
                source: OperationSource::Human,
                session_id: String::new(),
                plugin_id: Some(plugin_id.clone()),
            },
            ApproveProposal {
                operation_id,
                proposal_id,
                plugin_instance_id: None,
                args,
            },
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn reject_plugin_proposal(
    plugin_id: String,
    proposal_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let operation_id = state
        .plugin_runtime
        .pending_plugin_operation(&plugin_id, &proposal_id)
        .await
        .ok_or_else(|| "pending proposal not found for this plugin".to_owned())?;
    state
        .plugin_runtime
        .reject(
            CallerContext {
                source: OperationSource::Human,
                session_id: String::new(),
                plugin_id: Some(plugin_id),
            },
            &operation_id,
            &proposal_id,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn get_operation(
    operation_id: String,
    plugin_instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationRecord, String> {
    let instance = state
        .plugin_runtime
        .get_instance(&plugin_instance_id)
        .await
        .ok_or_else(|| "plugin instance not found".to_owned())?;
    state
        .plugin_runtime
        .get(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                plugin_id: Some(instance.plugin_id.clone()),
            },
            &operation_id,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn recent_operations(
    plugin_instance_id: String,
    limit: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<coterm_proto::runtime::operation::OperationRecord>, String> {
    let instance = state
        .plugin_runtime
        .get_instance(&plugin_instance_id)
        .await
        .ok_or_else(|| "plugin instance not found".to_owned())?;
    state
        .plugin_runtime
        .recent(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                plugin_id: Some(instance.plugin_id.clone()),
            },
            &plugin_instance_id,
            limit,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn approve_proposal(
    plugin_instance_id: String,
    proposal_id: String,
    args: Value,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    let instance = state
        .plugin_runtime
        .get_instance(&plugin_instance_id)
        .await
        .ok_or_else(|| "plugin instance not found".to_owned())?;
    let operation_id = state
        .plugin_runtime
        .pending_operation(&plugin_instance_id, &proposal_id)
        .await
        .ok_or_else(|| "pending proposal not found for this instance".to_owned())?;
    state
        .plugin_runtime
        .approve(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                plugin_id: Some(instance.plugin_id.clone()),
            },
            ApproveProposal {
                operation_id,
                proposal_id,
                plugin_instance_id: Some(plugin_instance_id),
                args,
            },
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn reject_proposal(
    plugin_instance_id: String,
    proposal_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let instance = state
        .plugin_runtime
        .get_instance(&plugin_instance_id)
        .await
        .ok_or_else(|| "plugin instance not found".to_owned())?;
    let operation_id = state
        .plugin_runtime
        .pending_operation(&plugin_instance_id, &proposal_id)
        .await
        .ok_or_else(|| "pending proposal not found for this instance".to_owned())?;
    state
        .plugin_runtime
        .reject(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                plugin_id: Some(instance.plugin_id.clone()),
            },
            &operation_id,
            &proposal_id,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn cancel_operation(
    plugin_instance_id: String,
    operation_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    let instance = state
        .plugin_runtime
        .get_instance(&plugin_instance_id)
        .await
        .ok_or_else(|| "plugin instance not found".to_owned())?;
    state
        .plugin_runtime
        .cancel(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                plugin_id: Some(instance.plugin_id.clone()),
            },
            &operation_id,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn set_bypass(enabled: bool, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.plugin_runtime.set_bypass(enabled).await;
    Ok(())
}

#[tauri::command]
async fn get_bypass(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.plugin_runtime.bypass_enabled().await)
}

/// 放行列表只在人工操作下修改；它跳过审批，但不改变 effect、不绕过插件校验。
#[tauri::command]
async fn set_allowed_operations(
    operations: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let _guard = state.workspace_actions.lock().await;
    state
        .plugin_runtime
        .set_allowed_operations(operations)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let allowed = state.plugin_runtime.allowed_operations().await;
    if let Err(error) = workspace_commands::store_allowed_operations(&state, allowed.clone()).await {
        return Err(format!("{}: {}", error.code, error.message));
    }
    Ok(allowed)
}

#[tauri::command]
async fn get_allowed_operations(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.plugin_runtime.allowed_operations().await)
}

async fn summary(state: &AppState, manifest: PluginManifest) -> PluginSummary {
    let id = manifest.id;
    PluginSummary {
        running: state.plugin_runtime.is_running(&id).await,
        id,
        name: manifest.name,
        version: manifest.version,
        has_frontend: manifest.frontend.is_some(),
    }
}

fn main() {
    let runtime_limits = RuntimeLimits {
        max_control_bytes: 1024 * 1024,
        max_data_chunk_bytes: 64 * 1024,
        initial_window: FlowWindow {
            chunks: 16,
            bytes: 1024 * 1024,
        },
    };

    let application = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let plugins_dir = app_data.join("plugins");
            std::fs::create_dir_all(&plugins_dir)?;
            let workspace_path = app_data.join("workspace.v1.dpapi");
            let workspace = workspace::WorkspaceStore::open(workspace_path.clone());
            let plugin_runtime = Arc::new(ProcessPluginRuntime::new(
                current_platform(),
                runtime_limits,
            ));
            // Restore the approval allowlist before any client can submit work.
            if let Ok(store) = &workspace {
                tauri::async_runtime::block_on(
                    plugin_runtime.set_allowed_operations(store.snapshot().allowed_operations),
                )
                .ok();
            }
            let mcp_server = tauri::async_runtime::block_on(
                coterm_mcp::http::HttpBridgeServer::bind(plugin_runtime.clone()),
            ).map_err(|error| format!("MCP unavailable at http://127.0.0.1:{}/mcp: {error}. Close the application using this port and restart Coterm.", coterm_mcp::http::DEFAULT_PORT));
            let expiry_runtime = plugin_runtime.clone();
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut expiry = tokio::time::interval(std::time::Duration::from_secs(1));
                let mut watcher = tokio::time::interval(std::time::Duration::from_millis(100));
                let mut published = expiry_runtime.attention_version();
                loop {
                    tokio::select! {
                        _ = expiry.tick() => expiry_runtime.expire_interactions().await,
                        // 统一观察宿主待处理状态：任何进入/离开变化都只发送版本号。
                        _ = watcher.tick() => {}
                    }
                    let version = expiry_runtime.attention_version();
                    if version != published {
                        published = version;
                        let _ = app_handle.emit(ATTENTION_CHANGED_EVENT, AttentionChanged { version });
                    }
                }
            });
            app.manage(AppState {
                workspace: Arc::new(std::sync::Mutex::new(workspace)),
                workspace_actions: tokio::sync::Mutex::new(()),
                workspace_path,
                exiting: std::sync::atomic::AtomicBool::new(false),
                event_subscriptions: Arc::new(tokio::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                _session_mgr: Arc::new(SessionManager::new()),
                plugin_runtime,
                mcp_server,
                plugins_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace_commands::workspace_snapshot,
            workspace_commands::save_workspace_layout,
            workspace_commands::restore_saved_instance,
            workspace_commands::forget_saved_instance,
            workspace_commands::reset_saved_workspace,
            subscribe_plugin_events,
            ack_plugin_event,
            unsubscribe_plugin_events,
            get_version,
            list_plugins,
            start_plugin,
            stop_plugin,
            list_instances,
            runtime_snapshot,
            list_operations,
            plugin_frontend_source,
            provide_secret,
            pending_interactions,
            host_attention,
            respond_secret,
            mcp_connection_info,
            mcp_sessions,
            close_mcp_session,
            create_instance,
            destroy_instance,
            invoke_operation,
            invoke_plugin_operation,
            recent_plugin_operations,
            approve_plugin_proposal,
            reject_plugin_proposal,
            get_operation,
            recent_operations,
            approve_proposal,
            reject_proposal,
            cancel_operation,
            set_bypass,
            get_bypass,
            set_allowed_operations,
            get_allowed_operations
        ])
        .build(tauri::generate_context!());
    let application = match application {
        Ok(application) => application,
        Err(error) => {
            eprintln!("Coterm startup failed: {error}");
            return;
        }
    };
    application.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            let state = app.state::<AppState>();
            if state
                .exiting
                .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                if code.is_none() {
                    api.prevent_exit();
                }
                return;
            }
            api.prevent_exit();
            let handle = app.clone();
            let runtime = state.plugin_runtime.clone();
            let subscriptions = state.event_subscriptions.clone();
            tauri::async_runtime::spawn(async move {
                for (_, subscription) in subscriptions.lock().await.drain() {
                    subscription.task.abort();
                }
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), runtime.shutdown())
                    .await;
                handle.exit(code.unwrap_or(0));
            });
        }
    });
}

fn current_platform() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x64"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "windows-arm64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-arm64"
    }
}
