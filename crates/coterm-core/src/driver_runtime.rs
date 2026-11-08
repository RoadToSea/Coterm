//! 基座内部接口；进程与路由实现在 driver_process / driver_manager。

use coterm_proto::runtime::{
    lifecycle::InstanceInfo,
    manifest::DriverManifest,
    message::{DriverEvent, HostMessage},
    operation::{ApproveProposal, OperationReceipt, OperationRecord, OperationSource},
    DriverId, RuntimeFuture,
};
use serde_json::Value;

/// source 由可信的 UI/MCP 适配层构造，不从领域 args 或 Tauri 参数反序列化。
#[derive(Clone)]
pub struct CallerContext {
    pub source: OperationSource,
    pub session_id: String,
    /// Trusted route authority for driver-scoped operations; never taken from args.
    pub driver_id: Option<DriverId>,
}

/// 外部调用不分配 operation_id，也不自行声明 effect；基座从目录读取。
pub struct SubmitOperation {
    pub driver_id: DriverId,
    pub driver_instance_id: Option<String>,
    pub operation: String,
    pub args: Value,
    /// None=默认等待，Some(0)=立即返回；仅影响本次等待。
    pub wait_timeout_ms: Option<u64>,
}

/// 负责加载/握手/停止进程以及实例索引。卸载前端不调用 destroy_instance。
pub trait DriverRuntime: Send + Sync {
    fn load<'a>(&'a self, package_path: &'a std::path::Path) -> RuntimeFuture<'a, DriverManifest>;
    fn start<'a>(&'a self, driver_id: &'a str) -> RuntimeFuture<'a, ()>;
    fn stop<'a>(&'a self, driver_id: &'a str) -> RuntimeFuture<'a, ()>;
    fn create_instance<'a>(
        &'a self,
        driver_id: &'a str,
        config: Value,
    ) -> RuntimeFuture<'a, InstanceInfo>;
    fn destroy_instance<'a>(&'a self, instance_id: &'a str) -> RuntimeFuture<'a, ()>;
    fn list_instances<'a>(
        &'a self,
        driver_id: Option<&'a str>,
    ) -> RuntimeFuture<'a, Vec<InstanceInfo>>;
}

/// 操作状态的唯一公共入口；方法必须验证 caller 与目标实例/操作的关联。
pub trait Operations: Send + Sync {
    fn submit<'a>(
        &'a self,
        caller: CallerContext,
        request: SubmitOperation,
    ) -> RuntimeFuture<'a, OperationReceipt>;
    fn get<'a>(
        &'a self,
        caller: CallerContext,
        operation_id: &'a str,
    ) -> RuntimeFuture<'a, OperationRecord>;
    fn recent<'a>(
        &'a self,
        caller: CallerContext,
        instance_id: &'a str,
        limit: usize,
    ) -> RuntimeFuture<'a, Vec<OperationRecord>>;
    /// args 是用户批准时的当前表单/草稿快照；仅 human 调用者可批准。
    fn approve<'a>(
        &'a self,
        caller: CallerContext,
        request: ApproveProposal,
    ) -> RuntimeFuture<'a, OperationReceipt>;
    fn reject<'a>(
        &'a self,
        caller: CallerContext,
        operation_id: &'a str,
        proposal_id: &'a str,
    ) -> RuntimeFuture<'a, ()>;
    /// 接受取消请求不等于底层已停止，仍通过 get 查询最终状态。
    fn cancel<'a>(
        &'a self,
        caller: CallerContext,
        operation_id: &'a str,
    ) -> RuntimeFuture<'a, OperationReceipt>;
}

/// 绑定某次握手连接；单一接收任务读取，EOF 使用 None，异常返回错误。
/// receive 不能独占整个连接，等待接收时仍须允许并发 send/close。
/// 控制帧编解码由 coterm-proto::runtime::wire 处理。
pub trait DriverConnection: Send + Sync {
    fn send<'a>(&'a self, message: HostMessage) -> RuntimeFuture<'a, ()>;
    fn receive<'a>(&'a self) -> RuntimeFuture<'a, Option<DriverEvent>>;
    fn close<'a>(&'a self) -> RuntimeFuture<'a, ()>;
}
