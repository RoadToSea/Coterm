// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use coterm_core::SessionManager;
use std::sync::Arc;

struct AppState {
    _session_mgr: Arc<SessionManager>,
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn main() {
    let session_mgr = Arc::new(SessionManager::new());
    let state = AppState {
        _session_mgr: session_mgr,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![get_version])
        .run(tauri::generate_context!())
        .expect("error while running Coterm desktop application");
}
