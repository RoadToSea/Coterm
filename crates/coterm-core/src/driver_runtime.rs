//! 基座内部接口；进程内调度与路由实现在 driver_task / driver_manager。

use coterm_proto::runtime::{
    lifecycle::InstanceInfo,
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

/// 负责启动/停止已注册后端以及实例索引。卸载前端不调用 destroy_instance。
pub trait DriverRuntime: Send + Sync {
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
