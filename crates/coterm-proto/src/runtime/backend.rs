//! 插件后端 SDK 接入面：开发者处理领域逻辑，SDK 隐藏帧和公共操作状态。

use super::interaction::{InteractionRequest, InteractionResult, SecretHandle, SecretMaterial};
use super::message::{HostMessage, PluginEvent};
use super::{RuntimeError, RuntimeFuture};

/// 由插件实现。可并发处理，不能因等待密码而阻塞整个收件循环。
pub trait PluginBackend: Send + Sync {
    /// 返回只表示已处理控制消息；领域结果通过 host.emit 报告。
    /// SDK 消费内部交互应答/额度后再派发，不要求插件重复实现请求关联。
    fn handle<'a>(
        &'a self,
        message: HostMessage,
        host: &'a dyn BackendHost,
    ) -> RuntimeFuture<'a, ()>;
}

/// 由基座 SDK 提供，绑定已握手的插件进程；事件中的实例须由本进程拥有。
pub trait BackendHost: Send + Sync {
    fn emit<'a>(&'a self, event: PluginEvent) -> RuntimeFuture<'a, ()>;
    fn interact<'a>(&'a self, request: InteractionRequest) -> RuntimeFuture<'a, InteractionResult>;
    /// 校验实例、用途、有效期；原始秘密不得通过 emit 或 MCP 输出。
    fn resolve_secret<'a>(
        &'a self,
        handle: SecretHandle,
    ) -> RuntimeFuture<'a, Box<dyn SecretMaterial>>;
    /// 通知 SDK 插件不可继续服务，结束在途请求；不自行伪造领域操作成功。
    fn unavailable<'a>(&'a self, error: RuntimeError) -> RuntimeFuture<'a, ()>;
}
