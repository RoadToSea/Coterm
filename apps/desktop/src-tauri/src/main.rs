// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::PathBuf, sync::Arc};

use coterm_core::{
    plugin_manager::ProcessPluginRuntime,
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
use tauri::Manager;

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
async fn list_plugins(state: tauri::State<'_, AppState>) -> Result<Vec<PluginSummary>, String> {
    // Development packages remain inside the workspace; installed packages win by ID.
    #[cfg(debug_assertions)]
    {
        let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../plugins");
        if let Ok(mut entries) = tokio::fs::read_dir(development).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let package = entry.path().join("dist");
                if package.join("manifest.json").is_file() {
                    state
                        .plugin_runtime
                        .load(&package)
                        .await
                        .map_err(|error| format!("{}: {}", error.code, error.message))?;
                }
            }
        }
    }
    let mut entries = tokio::fs::read_dir(&state.plugins_dir)
        .await
        .map_err(|error| format!("plugin directory unavailable: {error}"))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("plugin directory read failed: {error}"))?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| format!("plugin entry read failed: {error}"))?;
        if !file_type.is_dir() {
            continue;
        }
        let package_path = entry.path();
        state
            .plugin_runtime
            .load(&package_path)
            .await
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
    }
    let manifests = state.plugin_runtime.loaded_manifests().await;
    let mut plugins = Vec::with_capacity(manifests.len());
    for manifest in manifests {
        plugins.push(summary(&state, manifest).await);
    }
    plugins.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(plugins)
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

#[tauri::command]
async fn create_instance(
    plugin_id: String,
    config: Value,
    state: tauri::State<'_, AppState>,
) -> Result<InstanceInfo, String> {
    state
        .plugin_runtime
        .create_instance(&plugin_id, config)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn destroy_instance(
    plugin_instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_runtime
        .destroy_instance(&plugin_instance_id)
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
                session_id: instance.session_id,
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
                session_id: instance.session_id,
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
                session_id: instance.session_id,
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
                session_id: instance.session_id,
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
                session_id: instance.session_id,
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
                session_id: instance.session_id,
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

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let plugins_dir = app_data.join("plugins");
            std::fs::create_dir_all(&plugins_dir)?;
            let plugin_runtime = Arc::new(ProcessPluginRuntime::new(
                current_platform(),
                runtime_limits,
            ));
            let mcp_server = tauri::async_runtime::block_on(
                coterm_mcp::http::HttpBridgeServer::bind(plugin_runtime.clone()),
            ).map_err(|error| format!("MCP unavailable at http://127.0.0.1:{}/mcp: {error}. Close the application using this port and restart Coterm.", coterm_mcp::http::DEFAULT_PORT));
            let expiry_runtime = plugin_runtime.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    expiry_runtime.expire_interactions().await;
                }
            });
            app.manage(AppState {
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
            subscribe_plugin_events,
            ack_plugin_event,
            unsubscribe_plugin_events,
            get_version,
            list_plugins,
            start_plugin,
            stop_plugin,
            list_instances,
            list_operations,
            plugin_frontend_source,
            provide_secret,
            pending_interactions,
            respond_secret,
            mcp_connection_info,
            create_instance,
            destroy_instance,
            invoke_operation,
            get_operation,
            recent_operations,
            approve_proposal,
            reject_proposal,
            cancel_operation,
            set_bypass,
            get_bypass
        ])
        .run(tauri::generate_context!())
        .expect("error while running Coterm desktop application");
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
