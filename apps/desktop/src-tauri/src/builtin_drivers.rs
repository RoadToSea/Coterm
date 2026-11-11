//! 桌面应用的驱动组合入口；后端与前端都由构建时固定选择。
use coterm_core::driver_manager::InProcessDriverRuntime;
use coterm_proto::runtime::{manifest::DriverManifest, RuntimeError};

/// 注册全部内置驱动；声明或前端无效时让启动失败，避免分发残缺安装包。
pub async fn register(runtime: &InProcessDriverRuntime) -> Result<(), RuntimeError> {
    runtime
        .register::<coterm_driver_ssh::backend::Backend>(
            manifest(include_str!("../../../../drivers/driver-ssh/manifest.json"))?,
            Some(include_str!(
                "../../../../drivers/driver-ssh/dist/frontend/index.js"
            )),
        )
        .await?;
    runtime
        .register::<coterm_driver_serial::backend::Backend>(
            manifest(include_str!(
                "../../../../drivers/driver-serial/manifest.json"
            ))?,
            Some(include_str!(
                "../../../../drivers/driver-serial/dist/frontend/index.js"
            )),
        )
        .await?;
    runtime
        .register::<coterm_driver_local::backend::Backend>(
            manifest(include_str!(
                "../../../../drivers/driver-local/manifest.json"
            ))?,
            Some(include_str!(
                "../../../../drivers/driver-local/dist/frontend/index.js"
            )),
        )
        .await?;
    Ok(())
}

fn manifest(source: &str) -> Result<DriverManifest, RuntimeError> {
    serde_json::from_str(source).map_err(|_| RuntimeError {
        category: coterm_proto::runtime::ErrorCategory::Internal,
        code: "builtin_manifest_invalid".into(),
        message: "Invalid built-in driver declaration".into(),
        details: None,
    })
}
