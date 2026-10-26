//! 基座内部接口；进程与路由实现在 plugin_process / plugin_manager。
//! StreamRouter 等预留接口不属于对外 v1.0 JSON 控制契约。

use coterm_proto::runtime::{
    interaction::{InteractionRequest, InteractionResult},
    lifecycle::{InstanceInfo, RuntimeLimits},
    manifest::PluginManifest,
    message::{HostMessage, PluginEvent},
    operation::{ApproveProposal, OperationReceipt, OperationRecord, OperationSource},
    stream::{FlowWindow, StreamData, StreamGap, StreamInfo, SubscribeStream},
    PluginId, RuntimeFuture,
};
use serde_json::Value;

/// source 由可信的 UI/MCP 适配层构造，不从领域 args 或 Tauri 参数反序列化。
#[derive(Clone)]
pub struct CallerContext {
    pub source: OperationSource,
    pub session_id: String,
    /// Trusted route authority for plugin-scoped operations; never taken from args.
    pub plugin_id: Option<PluginId>,
}

/// 外部调用不分配 operation_id，也不自行声明 effect；基座从目录读取。
pub struct SubmitOperation {
    pub plugin_id: PluginId,
    pub plugin_instance_id: Option<String>,
    pub operation: String,
    pub args: Value,
    /// None=默认等待，Some(0)=立即返回；仅影响本次等待。
    pub wait_timeout_ms: Option<u64>,
}

pub struct RuntimeConfig {
    pub limits: RuntimeLimits,
    pub recent_operations_per_instance: usize,
    pub max_record_bytes: usize,
    pub default_wait_timeout_ms: u64,
    /// 用户主动开启后不再执行基座的 Agent 提交审批。
    pub bypass: bool,
}

/// 负责加载/握手/停止进程以及实例索引。卸载前端不调用 destroy_instance。
pub trait PluginRuntime: Send + Sync {
    fn load<'a>(&'a self, package_path: &'a std::path::Path) -> RuntimeFuture<'a, PluginManifest>;
    fn start<'a>(&'a self, plugin_id: &'a str) -> RuntimeFuture<'a, ()>;
    fn stop<'a>(&'a self, plugin_id: &'a str) -> RuntimeFuture<'a, ()>;
    fn create_instance<'a>(
        &'a self,
        plugin_id: &'a str,
        config: Value,
    ) -> RuntimeFuture<'a, InstanceInfo>;
    fn destroy_instance<'a>(&'a self, instance_id: &'a str) -> RuntimeFuture<'a, ()>;
    fn list_instances<'a>(
        &'a self,
        plugin_id: Option<&'a str>,
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
/// 帧编码/部分读写和控制优先级由未来适配器处理。
pub trait PluginConnection: Send + Sync {
    fn send<'a>(&'a self, message: HostMessage) -> RuntimeFuture<'a, ()>;
    fn receive<'a>(&'a self) -> RuntimeFuture<'a, Option<PluginEvent>>;
    fn close<'a>(&'a self) -> RuntimeFuture<'a, ()>;
}

pub trait InteractionBroker: Send + Sync {
    fn request<'a>(&'a self, request: InteractionRequest) -> RuntimeFuture<'a, InteractionResult>;
    fn respond<'a>(
        &'a self,
        caller: CallerContext,
        result: InteractionResult,
    ) -> RuntimeFuture<'a, ()>;
}

pub enum StreamDelivery {
    Open(StreamInfo),
    Data(StreamData),
    Gap(StreamGap),
    Closed { reason: Option<String> },
}

/// 每个消费者独立订阅；next 的读取不自动表示用户已消费。
pub trait StreamSubscription: Send {
    fn id(&self) -> &str;
    fn next<'a>(&'a mut self) -> RuntimeFuture<'a, Option<StreamDelivery>>;
    fn grant<'a>(&'a self, credit: FlowWindow) -> RuntimeFuture<'a, ()>;
    fn cancel<'a>(&'a self) -> RuntimeFuture<'a, ()>;
}

pub trait StreamRouter: Send + Sync {
    fn subscribe<'a>(
        &'a self,
        caller: CallerContext,
        request: SubscribeStream,
    ) -> RuntimeFuture<'a, Box<dyn StreamSubscription>>;
}

/// 插件前端不能设置 Bypass。桌面设置入口只有可信的人类操作可调用。
pub trait RuntimeSettings: Send + Sync {
    fn set_bypass<'a>(&'a self, caller: CallerContext, enabled: bool) -> RuntimeFuture<'a, ()>;
}
