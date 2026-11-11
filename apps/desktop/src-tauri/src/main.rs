// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod approval_commands;
mod builtin_drivers;
mod mcp_commands;
mod mcp_settings;
mod workspace;
mod workspace_commands;

use std::{
    path::PathBuf,
    sync::Arc,
};

use coterm_core::{
    driver_manager::{DriverLoadIssue, InProcessDriverRuntime},
    driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation},
};
use coterm_proto::runtime::{
    lifecycle::{InstanceInfo, RuntimeLimits},
    manifest::DriverManifest,
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
    driver_runtime: Arc<InProcessDriverRuntime>,
    stdio_bridge: mcp_commands::StdioBridge,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DriverSummary {
    id: String,
    name: String,
    version: String,
    has_frontend: bool,
    running: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DriverCatalog {
    drivers: Vec<DriverSummary>,
    /// 保留目录接口的诊断字段；内置驱动在启动前校验，此处为空。
    issues: Vec<DriverLoadIssue>,
}

/// JSON-event MVP: one unacknowledged delivery per subscriber, bounded replay in driver.
#[tauri::command]
async fn subscribe_driver_events(
    driver_instance_id: String,
    on_event: tauri::ipc::Channel<InstanceEvent>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let mut events = state
        .driver_runtime
        .subscribe_events(&driver_instance_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut subscriptions = state.event_subscriptions.lock().await;
    if subscriptions.len() >= 32 {
        return Err("Too many event subscriptions".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let (ack, mut acknowledged) = tokio::sync::mpsc::channel(1);
    let instance = driver_instance_id.clone();
    let cleanup = Arc::downgrade(&state.event_subscriptions);
    let subscription_id = id.clone();
    let task = tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(coterm_proto::runtime::message::DriverNotification::Event {
                    driver_instance_id,
                    name,
                    payload,
                }) if driver_instance_id == instance
                    || (driver_instance_id.is_empty() && name == "runtime.closed") =>
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
            instance_id: driver_instance_id,
            ack,
            task,
        },
    );
    Ok(id)
}

#[tauri::command]
async fn ack_driver_event(
    driver_instance_id: String,
    subscription_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let subscriptions = state.event_subscriptions.lock().await;
    let subscription = subscriptions
        .get(&subscription_id)
        .filter(|s| s.instance_id == driver_instance_id)
        .ok_or("Event subscription not found")?;
    subscription
        .ack
        .try_send(())
        .map_err(|_| "Event already acknowledged".to_owned())
}

#[tauri::command]
async fn unsubscribe_driver_events(
    driver_instance_id: String,
    subscription_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut subscriptions = state.event_subscriptions.lock().await;
    if subscriptions
        .get(&subscription_id)
        .is_some_and(|s| s.instance_id == driver_instance_id)
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
async fn list_drivers(state: tauri::State<'_, AppState>) -> Result<DriverCatalog, String> {
    let _guard = state.workspace_actions.lock().await;
    registered_driver_catalog(&state).await
}

async fn registered_driver_catalog(state: &AppState) -> Result<DriverCatalog, String> {
    let mut drivers = Vec::new();
    for manifest in state.driver_runtime.loaded_manifests().await {
        drivers.push(summary(state, manifest).await);
    }
    drivers.sort_by(|a,b| a.name.cmp(&b.name));
    Ok(DriverCatalog {drivers, issues:Vec::new()})
}

#[tauri::command]
async fn start_driver(driver_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .driver_runtime
        .start(&driver_id)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn stop_driver(driver_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _guard = state.workspace_actions.lock().await;
    state
        .driver_runtime
        .stop(&driver_id)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn list_instances(
    driver_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstanceInfo>, String> {
    state
        .driver_runtime
        .list_instances(driver_id.as_deref())
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeSnapshot {
    instances: Vec<InstanceInfo>,
    running_drivers: Vec<String>,
}

#[tauri::command]
async fn runtime_snapshot(state: tauri::State<'_, AppState>) -> Result<RuntimeSnapshot, String> {
    let instances = state
        .driver_runtime
        .list_instances(None)
        .await
        .map_err(|error| error.code)?;
    let mut running_drivers = Vec::new();
    for manifest in state.driver_runtime.loaded_manifests().await {
        if state.driver_runtime.is_running(&manifest.id).await {
            running_drivers.push(manifest.id);
        }
    }
    Ok(RuntimeSnapshot {
        instances,
        running_drivers,
    })
}

#[tauri::command]
async fn list_operations(
    driver_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<OperationDescriptor>, String> {
    Ok(state.driver_runtime.operation_catalog(&driver_id).await)
}

#[tauri::command]
async fn driver_frontend_source(
    driver_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    state
        .driver_runtime
        .frontend_source(&driver_id)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

/// Only the trusted desktop secret prompt calls this; never an MCP operation.
#[tauri::command]
async fn provide_secret(
    driver_instance_id: String,
    purpose: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::interaction::SecretHandle, String> {
    state
        .driver_runtime
        .provide_secret(&driver_instance_id, purpose, value)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn pending_interactions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<coterm_proto::runtime::interaction::InteractionRequest>, String> {
    Ok(state.driver_runtime.pending_interactions().await)
}

/// 宿主级待处理快照；事件只携带版本号，前端据此重取。
#[tauri::command]
async fn host_attention(
    state: tauri::State<'_, AppState>,
) -> Result<coterm_core::driver_manager::AttentionSnapshot, String> {
    Ok(state.driver_runtime.attention_snapshot().await)
}

#[tauri::command]
async fn respond_secret(
    interaction_id: String,
    value: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), coterm_proto::runtime::RuntimeError> {
    state
        .driver_runtime
        .respond_secret(&interaction_id, value)
        .await
}

/// Stdio 适配器状态：是否已允许本机客户端连接，以及可粘贴的客户端配置。
#[tauri::command]
async fn mcp_stdio_status(
    state: tauri::State<'_, AppState>,
) -> Result<mcp_commands::StdioStatus, String> {
    Ok(state.stdio_bridge.status().await)
}

/// 用户显式启用或停用私有 Stdio Bridge。
#[tauri::command]
async fn set_mcp_stdio_enabled(
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<mcp_commands::StdioStatus, String> {
    state.stdio_bridge.set_enabled(enabled).await
}

#[tauri::command]
async fn create_instance(
    driver_id: String,
    config: Value,
    workspace_id: Option<String>,
    name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<InstanceInfo, String> {
    let _guard = state.workspace_actions.lock().await;
    let instance = state
        .driver_runtime
        .create_instance_with_name(&driver_id, config.clone(), name)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let id = instance.driver_instance_id.clone();
    let label = instance.label.clone();
    if let Err(error) = workspace_commands::access(&state, move |store| {
        store.insert(driver_id, id, label, config, workspace_id)
    })
    .await
    {
        let _ = state
            .driver_runtime
            .destroy_instance(&instance.driver_instance_id)
            .await;
        return Err(format!("{}: {}", error.code, error.message));
    }
    Ok(instance)
}

#[tauri::command]
async fn destroy_instance(
    driver_instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _guard = state.workspace_actions.lock().await;
    state
        .driver_runtime
        .destroy_instance(&driver_instance_id)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    workspace_commands::access(&state, move |store| {
        store.remove_instance(&driver_instance_id)
    })
    .await
    .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn invoke_operation(
    driver_instance_id: String,
    operation: String,
    args: Value,
    wait_timeout_ms: Option<u64>,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    let instance = state
        .driver_runtime
        .get_instance(&driver_instance_id)
        .await
        .ok_or_else(|| "driver instance not found".to_owned())?;
    state
        .driver_runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            SubmitOperation {
                driver_id: instance.driver_id,
                driver_instance_id: Some(driver_instance_id),
                operation,
                args,
                wait_timeout_ms,
            },
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn invoke_driver_operation(
    driver_id: String,
    operation: String,
    args: Value,
    wait_timeout_ms: Option<u64>,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    state
        .driver_runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: String::new(),
                driver_id: Some(driver_id.clone()),
            },
            SubmitOperation {
                driver_id,
                driver_instance_id: None,
                operation,
                args,
                wait_timeout_ms,
            },
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn recent_driver_operations(
    driver_id: String,
    limit: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<coterm_proto::runtime::operation::OperationRecord>, String> {
    Ok(state
        .driver_runtime
        .recent_driver_operations(&driver_id, limit)
        .await)
}

#[tauri::command]
async fn approve_driver_proposal(
    driver_id: String,
    proposal_id: String,
    args: Value,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    let operation_id = state
        .driver_runtime
        .pending_driver_operation(&driver_id, &proposal_id)
        .await
        .ok_or_else(|| "pending proposal not found for this driver".to_owned())?;
    state
        .driver_runtime
        .approve(
            CallerContext {
                source: OperationSource::Human,
                session_id: String::new(),
                driver_id: Some(driver_id.clone()),
            },
            ApproveProposal {
                operation_id,
                proposal_id,
                driver_instance_id: None,
                args,
            },
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn reject_driver_proposal(
    driver_id: String,
    proposal_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let operation_id = state
        .driver_runtime
        .pending_driver_operation(&driver_id, &proposal_id)
        .await
        .ok_or_else(|| "pending proposal not found for this driver".to_owned())?;
    state
        .driver_runtime
        .reject(
            CallerContext {
                source: OperationSource::Human,
                session_id: String::new(),
                driver_id: Some(driver_id),
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
    driver_instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationRecord, String> {
    let instance = state
        .driver_runtime
        .get_instance(&driver_instance_id)
        .await
        .ok_or_else(|| "driver instance not found".to_owned())?;
    state
        .driver_runtime
        .get(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            &operation_id,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn recent_operations(
    driver_instance_id: String,
    limit: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<coterm_proto::runtime::operation::OperationRecord>, String> {
    let instance = state
        .driver_runtime
        .get_instance(&driver_instance_id)
        .await
        .ok_or_else(|| "driver instance not found".to_owned())?;
    state
        .driver_runtime
        .recent(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            &driver_instance_id,
            limit,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn approve_proposal(
    driver_instance_id: String,
    proposal_id: String,
    args: Value,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    let instance = state
        .driver_runtime
        .get_instance(&driver_instance_id)
        .await
        .ok_or_else(|| "driver instance not found".to_owned())?;
    let operation_id = state
        .driver_runtime
        .pending_operation(&driver_instance_id, &proposal_id)
        .await
        .ok_or_else(|| "pending proposal not found for this instance".to_owned())?;
    state
        .driver_runtime
        .approve(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            ApproveProposal {
                operation_id,
                proposal_id,
                driver_instance_id: Some(driver_instance_id),
                args,
            },
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn reject_proposal(
    driver_instance_id: String,
    proposal_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let instance = state
        .driver_runtime
        .get_instance(&driver_instance_id)
        .await
        .ok_or_else(|| "driver instance not found".to_owned())?;
    let operation_id = state
        .driver_runtime
        .pending_operation(&driver_instance_id, &proposal_id)
        .await
        .ok_or_else(|| "pending proposal not found for this instance".to_owned())?;
    state
        .driver_runtime
        .reject(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            &operation_id,
            &proposal_id,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn cancel_operation(
    driver_instance_id: String,
    operation_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<coterm_proto::runtime::operation::OperationReceipt, String> {
    let instance = state
        .driver_runtime
        .get_instance(&driver_instance_id)
        .await
        .ok_or_else(|| "driver instance not found".to_owned())?;
    state
        .driver_runtime
        .cancel(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            &operation_id,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

#[tauri::command]
async fn set_bypass(
    driver_instance_id: String,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), coterm_proto::runtime::RuntimeError> {
    let _guard = state.workspace_actions.lock().await;
    state
        .driver_runtime
        .bypass_enabled(&driver_instance_id)
        .await?;
    let instance_id = driver_instance_id.clone();
    workspace_commands::access(&state, move |store| {
        store.save_bypass(&instance_id, enabled)
    })
    .await?;
    state
        .driver_runtime
        .set_bypass(&driver_instance_id, enabled)
        .await
}

#[tauri::command]
async fn get_bypass(
    driver_instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, coterm_proto::runtime::RuntimeError> {
    state
        .driver_runtime
        .bypass_enabled(&driver_instance_id)
        .await
}

#[tauri::command]
fn preview_approval_policy(
    policy: coterm_core::approval_policy::ApprovalPolicy,
    text: String,
) -> Result<bool, coterm_proto::runtime::RuntimeError> {
    Ok(policy.compile()?.requires_approval(&[&text]))
}

async fn summary(state: &AppState, manifest: DriverManifest) -> DriverSummary {
    let id = manifest.id;
    DriverSummary {
        running: state.driver_runtime.is_running(&id).await,
        id,
        name: manifest.name,
        version: manifest.version,
        has_frontend: manifest.frontend.is_some(),
    }
}

/// MCP 客户端以 `mcp stdio` 启动本程序；该模式不创建窗口或驱动运行时，
/// stdout 只承载 MCP 帧。前缀匹配后由适配器校验选项，错误选项不会启动窗口。
fn is_stdio_adapter(args: &[String]) -> bool {
    coterm_mcp::stdio::is_adapter_command(args)
}

/// 适配器使用独立的单线程运行时，避免与桌面 UI 运行时共享线程池。
/// 诊断只写 stderr：stdout 是 MCP 协议通道。
fn run_stdio_adapter(args: &[String]) -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Coterm MCP adapter could not start: {error}");
            return 1;
        }
    };
    match runtime.block_on(coterm_mcp::stdio::run(args)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Coterm MCP adapter: {error:#}");
            1
        }
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if is_stdio_adapter(&arguments) {
        std::process::exit(run_stdio_adapter(&arguments[2..]));
    }
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
            std::fs::create_dir_all(&app_data)?;
            let workspace_path = app_data.join("workspace.v1.dpapi");
            let workspace = workspace::WorkspaceStore::open(workspace_path.clone());
            let driver_runtime = Arc::new(InProcessDriverRuntime::new(runtime_limits));
            tauri::async_runtime::block_on(builtin_drivers::register(&driver_runtime))?;

            // 适配器就是本可执行文件的 `mcp stdio` 子命令：不搜索 PATH，也不依赖旁路文件。
            let stdio_bridge = mcp_commands::StdioBridge::new(
                driver_runtime.clone(),
                std::env::current_exe().ok(),
                app_data.join("mcp-settings.v1.json"),
            );
            if let Err(error) = tauri::async_runtime::block_on(stdio_bridge.start_on_launch()) {
                eprintln!("Failed to auto-start MCP Stdio bridge: {error}");
            }
            let expiry_runtime = driver_runtime.clone();
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
                        let _ =
                            app_handle.emit(ATTENTION_CHANGED_EVENT, AttentionChanged { version });
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
                driver_runtime,
                stdio_bridge,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace_commands::workspace_snapshot,
            workspace_commands::save_instance_configuration,
            workspace_commands::save_workspace_layout,
            workspace_commands::create_workspace_group,
            workspace_commands::delete_workspace_group,
            workspace_commands::move_workspace_entry,
            workspace_commands::restore_saved_instance,
            workspace_commands::forget_saved_instance,
            workspace_commands::reset_saved_workspace,
            subscribe_driver_events,
            ack_driver_event,
            unsubscribe_driver_events,
            get_version,
            list_drivers,
            start_driver,
            stop_driver,
            list_instances,
            runtime_snapshot,
            list_operations,
            driver_frontend_source,
            provide_secret,
            pending_interactions,
            host_attention,
            respond_secret,
            mcp_stdio_status,
            set_mcp_stdio_enabled,
            mcp_commands::set_mcp_stdio_port,
            mcp_commands::set_mcp_startup_mode,
            create_instance,
            destroy_instance,
            invoke_operation,
            invoke_driver_operation,
            recent_driver_operations,
            approve_driver_proposal,
            reject_driver_proposal,
            get_operation,
            recent_operations,
            approve_proposal,
            reject_proposal,
            cancel_operation,
            set_bypass,
            get_bypass,
            approval_commands::approval_snapshot,
            approval_commands::save_approval_group,
            approval_commands::assign_approval_group,
            approval_commands::delete_approval_group,
            preview_approval_policy
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
            let runtime = state.driver_runtime.clone();
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
