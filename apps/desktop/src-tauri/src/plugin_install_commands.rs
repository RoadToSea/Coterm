//! File selection stays in the native host; IPC accepts tokens, never arbitrary paths.
use coterm_core::plugin_install::PreparedPluginPackage;
use coterm_proto::runtime::{ErrorCategory, RuntimeError};
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use crate::{AppState, PluginSummary};

pub struct PendingInstall {
    token: String,
    package: PreparedPluginPackage,
    created: std::time::Instant,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPreview {
    token: String,
    id: String,
    name: String,
    version: String,
    platform: String,
}

fn error(code: &str) -> RuntimeError {
    RuntimeError {
        category: match code {
            "plugin_install_conflict" => ErrorCategory::Conflict,
            "plugin_install_io" => ErrorCategory::Transport,
            "plugin_install_unavailable" => ErrorCategory::NotFound,
            _ => ErrorCategory::InvalidRequest,
        },
        code: code.to_owned(),
        message: code.to_owned(),
        details: None,
    }
}

#[tauri::command]
pub async fn prepare_plugin_install(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    locale: String,
) -> Result<Option<InstallPreview>, RuntimeError> {
    // One bounded staging slot per app, including simultaneous/repeated IPC calls.
    let mut pending = state.plugin_install.lock().await;
    let previous = pending.take();
    tokio::task::spawn_blocking(move || drop(previous))
        .await
        .map_err(|_| error("plugin_install_io"))?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(if locale == "zh-CN" {
            "选择离线插件包"
        } else {
            "Choose offline plugin package"
        })
        .add_filter("Coterm plugin", &["coterm", "zip"])
        .pick_file(move |file| {
            let _ = sender.send(file);
        });
    let Some(selected) = receiver
        .await
        .map_err(|_| error("plugin_install_unavailable"))?
    else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|_| error("plugin_install_io"))?;
    let staging = app
        .path()
        .app_data_dir()
        .map_err(|_| error("plugin_install_io"))?
        .join("plugin-staging");
    let platform = crate::current_platform();
    let package = PreparedPluginPackage::prepare(path, staging, platform).await?;
    if state
        .plugin_runtime
        .is_discovered(&package.manifest.id)
        .await
    {
        return Err(error("plugin_install_conflict"));
    }
    let token = uuid::Uuid::new_v4().to_string();
    let preview = InstallPreview {
        token: token.clone(),
        id: package.manifest.id.clone(),
        name: package.manifest.name.clone(),
        version: package.manifest.version.clone(),
        platform: platform.to_owned(),
    };
    *pending = Some(PendingInstall {
        token,
        package,
        created: std::time::Instant::now(),
    });
    Ok(Some(preview))
}

#[tauri::command]
pub async fn install_plugin_package(
    token: String,
    state: tauri::State<'_, AppState>,
) -> Result<PluginSummary, RuntimeError> {
    let mut pending = state.plugin_install.lock().await;
    if !pending.as_ref().is_some_and(|item| item.token == token) {
        return Err(error("plugin_install_unavailable"));
    }
    let item = pending
        .take()
        .ok_or_else(|| error("plugin_install_unavailable"))?;
    if item.created.elapsed() > std::time::Duration::from_secs(600) {
        return Err(error("plugin_install_unavailable"));
    }
    let manifest = item
        .package
        .install(state.plugins_dir.clone(), &state.plugin_runtime)
        .await?;
    Ok(crate::summary(&state, manifest).await)
}

#[tauri::command]
pub async fn discard_plugin_install(
    token: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), RuntimeError> {
    let mut pending = state.plugin_install.lock().await;
    if pending.as_ref().is_some_and(|item| item.token == token) {
        let previous = pending.take();
        tokio::task::spawn_blocking(move || drop(previous))
            .await
            .map_err(|_| error("plugin_install_io"))?;
    }
    Ok(())
}
