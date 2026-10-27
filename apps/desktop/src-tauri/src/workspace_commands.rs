use crate::{
    workspace::{self, Snapshot, WorkspaceStore},
    AppState,
};
use coterm_core::plugin_runtime::PluginRuntime;
use coterm_proto::runtime::{
    lifecycle::{InstanceInfo, InstanceStatus},
    RuntimeError,
};

pub async fn access<T: Send + 'static>(
    state: &AppState,
    action: impl FnOnce(&mut WorkspaceStore) -> Result<T, RuntimeError> + Send + 'static,
) -> Result<T, RuntimeError> {
    let store = state.workspace.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = store
            .lock()
            .map_err(|_| workspace::error("workspace_lock_failed"))?;
        action(guard.as_mut().map_err(|error| error.clone())?)
    })
    .await
    .map_err(|_| workspace::error("workspace_task_failed"))?
}

#[tauri::command]
pub async fn workspace_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<Snapshot, RuntimeError> {
    let mut snapshot = access(&state, |store| Ok(store.snapshot())).await?;
    let live = state.plugin_runtime.list_instances(None).await?;
    for entry in &mut snapshot.entries {
        if !live
            .iter()
            .any(|instance| Some(&instance.plugin_instance_id) == entry.instance_id.as_ref())
        {
            entry.instance_id = None;
        }
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn save_workspace_layout(
    active_instance_id: Option<String>,
    instances_collapsed: bool,
    order: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), RuntimeError> {
    if order.len() > 128 {
        return Err(workspace::error("workspace_capacity"));
    }
    access(&state, move |store| {
        store.save_layout(active_instance_id, instances_collapsed, order)
    })
    .await
}

#[tauri::command]
pub async fn restore_saved_instance(
    config_ref: String,
    state: tauri::State<'_, AppState>,
) -> Result<InstanceInfo, RuntimeError> {
    let _guard = state.workspace_actions.lock().await;
    let reference = config_ref.clone();
    let entry = access(&state, move |store| store.find(&reference)).await?;
    if let Some(old_id) = &entry.instance_id {
        if let Some(old) = state.plugin_runtime.get_instance(old_id).await {
            if old.status != InstanceStatus::Failed {
                return Err(workspace::error("workspace_instance_already_running"));
            }
            state.plugin_runtime.destroy_instance(old_id).await?;
        }
    }
    // Only create/configure. Never invoke connect, commit or a historical operation.
    if !state.plugin_runtime.is_running(&entry.plugin_id).await {
        state.plugin_runtime.start(&entry.plugin_id).await?;
    }
    let instance = state
        .plugin_runtime
        .create_instance(&entry.plugin_id, entry.config)
        .await?;
    let id = instance.plugin_instance_id.clone();
    let label = instance.label.clone();
    if let Err(error) = access(&state, move |store| store.bind(&config_ref, id, label)).await {
        let _ = state
            .plugin_runtime
            .destroy_instance(&instance.plugin_instance_id)
            .await;
        return Err(error);
    }
    Ok(instance)
}

#[tauri::command]
pub async fn forget_saved_instance(
    config_ref: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), RuntimeError> {
    let _guard = state.workspace_actions.lock().await;
    let reference = config_ref.clone();
    let entry = access(&state, move |store| store.find(&reference)).await?;
    if let Some(id) = entry.instance_id {
        if state.plugin_runtime.get_instance(&id).await.is_some() {
            return Err(workspace::error("workspace_instance_still_present"));
        }
    }
    access(&state, move |store| store.remove(&config_ref)).await
}

#[tauri::command]
pub async fn reset_saved_workspace(state: tauri::State<'_, AppState>) -> Result<(), RuntimeError> {
    let _guard = state.workspace_actions.lock().await;
    if !state.plugin_runtime.list_instances(None).await?.is_empty() {
        return Err(workspace::error("workspace_instance_still_present"));
    }
    let store = state.workspace.clone();
    let path = state.workspace_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = store
            .lock()
            .map_err(|_| workspace::error("workspace_lock_failed"))?;
        let mut empty = WorkspaceStore::empty(path);
        empty.save_layout(None, false, Vec::new())?;
        *guard = Ok(empty);
        Ok(())
    })
    .await
    .map_err(|_| workspace::error("workspace_task_failed"))?
}

/// Persists the approval allowlist without touching instance entries.
pub async fn store_allowed_operations(
    state: &tauri::State<'_, AppState>,
    allowed: Vec<String>,
) -> Result<(), coterm_proto::runtime::RuntimeError> {
    access(state, move |store| store.save_allowed_operations(allowed)).await
}
