//! 桌面应用的驱动组合入口；后端与前端都由构建时固定选择。
use coterm_core::model::{manifest::DriverManifest, RuntimeError};
use coterm_core::runtime::Runtime;

/// 注册全部内置驱动；声明或前端无效时让启动失败，避免分发残缺安装包。
pub async fn register(runtime: &Runtime) -> Result<(), RuntimeError> {
    runtime
        .register::<coterm_driver::ssh::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/ssh.json"
            ))?,
            Some(include_str!(
                "../../../../crates/coterm-driver/dist/ssh/frontend/index.js"
            )),
        )
        .await?;
    runtime
        .register::<coterm_driver::serial::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/serial.json"
            ))?,
            Some(include_str!(
                "../../../../crates/coterm-driver/dist/serial/frontend/index.js"
            )),
        )
        .await?;
    runtime
        .register::<coterm_driver::local::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/local.json"
            ))?,
            Some(include_str!(
                "../../../../crates/coterm-driver/dist/local/frontend/index.js"
            )),
        )
        .await?;
    runtime
        .register::<coterm_driver::tcp::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/tcp.json"
            ))?,
            Some(include_str!(
                "../../../../crates/coterm-driver/dist/tcp/frontend/index.js"
            )),
        )
        .await?;
    runtime
        .register::<coterm_driver::telnet::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/telnet.json"
            ))?,
            Some(include_str!(
                "../../../../crates/coterm-driver/dist/telnet/frontend/index.js"
            )),
        )
        .await?;
    Ok(())
}

fn manifest(source: &str) -> Result<DriverManifest, RuntimeError> {
    serde_json::from_str(source).map_err(|_| RuntimeError {
        category: coterm_core::model::ErrorCategory::Internal,
        code: "builtin_manifest_invalid".into(),
        message: "Invalid built-in driver declaration".into(),
        details: None,
    })
}
