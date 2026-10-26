//! Rust 插件唯一后端入口；线缆编码与握手顺序由 transport 处理。

use serde_json::Value;
use tokio::sync::mpsc;

use super::{
    message::{HostCommand, PluginNotification},
    RuntimeError,
};

/// 控制请求串行处理。长操作和交互必须立即受理，稍后通过通知完成。
#[allow(async_fn_in_trait)]
pub trait PluginBackend: Send {
    async fn request(&mut self, command: HostCommand) -> Result<Value, RuntimeError>;

    /// 等待控制请求时清理过期状态；不允许阻塞。
    fn tick(&mut self) {}

    /// SDK 启动时取走有界通知队列；保留的发送端必须在停止时释放。
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<PluginNotification>>;
}
