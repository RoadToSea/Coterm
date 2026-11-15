//! 桌面应用的驱动组合入口；后端静态链接，前端由桌面工作台提供。
use coterm_core::model::{manifest::DriverManifest, RuntimeError};
use coterm_core::runtime::Runtime;

/// 注册全部内置驱动；声明无效时让启动失败，避免分发残缺安装包。
pub async fn register(runtime: &Runtime) -> Result<(), RuntimeError> {
    runtime
        .register::<coterm_driver::ssh::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/ssh.json"
            ))?,
            None,
        )
        .await?;
    runtime
        .register::<coterm_driver::serial::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/serial.json"
            ))?,
            None,
        )
        .await?;
    runtime
        .register::<coterm_driver::local::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/local.json"
            ))?,
            None,
        )
        .await?;
    runtime
        .register::<coterm_driver::tcp::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/tcp.json"
            ))?,
            None,
        )
        .await?;
    runtime
        .register::<coterm_driver::telnet::backend::Backend>(
            manifest(include_str!(
                "../../../../crates/coterm-driver/manifests/telnet.json"
            ))?,
            None,
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
