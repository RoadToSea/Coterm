//! 内置驱动后端入口；宿主按驱动串行调度请求。

use super::lifecycle::{HelloResult, INTERFACE_MAJOR};
use super::ErrorCategory;
use serde_json::Value;
use tokio::sync::mpsc;

use super::{
    message::{DriverNotification, HostCommand},
    RuntimeError,
};

/// 控制请求串行处理。长操作和交互必须立即受理，稍后通过通知完成。
pub trait DriverBackend: Send {
    /// 接收 HostCommand，异步返回领域 JSON 或 RuntimeError；Future 必须可跨任务调度。
    fn request(
        &mut self,
        command: HostCommand,
    ) -> impl std::future::Future<Output = Result<Value, RuntimeError>> + Send;

    /// 等待控制请求时清理过期状态；不允许阻塞。
    fn tick(&mut self) {}

    /// 运行时启动时取走有界通知队列；保留的发送端必须在停止时释放。
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<DriverNotification>>;
}

pub(crate) fn rejected(category: ErrorCategory, code: &str) -> RuntimeError {
    RuntimeError {
        category,
        code: code.to_owned(),
        message: code.replace('_', " "),
        details: None,
    }
}

/// 调用一个后端请求，校验初始化顺序与已协商的秘密交互能力。
/// `state` 是本次后端初始化的结果；未初始化、重复初始化和未启用能力均返回结构化错误。
pub async fn dispatch<B: DriverBackend>(
    backend: &mut B,
    command: HostCommand,
    state: Option<&HelloResult>,
) -> Result<Value, RuntimeError> {
    if let HostCommand::Hello(ref hello) = command {
        if state.is_some() {
            return Err(rejected(
                ErrorCategory::Conflict,
                "runtime_already_initialized",
            ));
        }
        if hello.interface_major != INTERFACE_MAJOR || hello.limits.max_control_bytes == 0 {
            return Err(rejected(
                ErrorCategory::Unsupported,
                "driver_handshake_incompatible",
            ));
        }
    } else if !matches!(command, HostCommand::Shutdown) {
        let state =
            state.ok_or_else(|| rejected(ErrorCategory::Conflict, "runtime_not_initialized"))?;
        let required = match command {
            HostCommand::ProvideSecret(_) => Some("secret-delivery"),
            HostCommand::InteractionResult(_) => Some("deferred-interactions"),
            _ => None,
        };
        if required.is_some_and(|feature| !state.enabled_features.iter().any(|f| f == feature)) {
            return Err(rejected(
                ErrorCategory::Unsupported,
                "runtime_feature_not_negotiated",
            ));
        }
    }
    backend.request(command).await
}
