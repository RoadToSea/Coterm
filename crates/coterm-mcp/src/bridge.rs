//! 单入口 MCP 路由契约。不把插件的原生工具转换成统一 execute 工具。

use coterm_core::plugin_runtime::CallerContext;
use coterm_proto::runtime::{
    operation::{DispatchMode, OperationEffect, OperationReceipt, OperationScope},
    OperationId, PluginId, PluginInstanceId, RuntimeFuture,
};
use coterm_proto::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use serde_json::Value;

/// 保留完整 MCP 描述和扩展字段；effect/scope 是基座的路由元数据。
pub struct McpToolDescriptor {
    pub name: String,
    pub effect: OperationEffect,
    pub scope: OperationScope,
    pub descriptor: Value,
}

pub struct McpCatalog {
    pub tools: Vec<McpToolDescriptor>,
    pub resources: Vec<Value>,
    pub resource_templates: Vec<Value>,
    pub prompts: Vec<Value>,
    /// 包括 listChanged、subscribe 等 MCP 能力声明。
    pub capabilities: Value,
}

/// 双向原生消息：保留 result/error、进度、取消和目录变化通知。
pub enum McpMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

/// 只有进入插件执行路径后才产生。ID、source 和 dispatch 均由基座确定。
pub struct McpDispatchContext {
    pub operation_id: OperationId,
    pub plugin_instance_id: Option<PluginInstanceId>,
    pub caller: CallerContext,
    pub dispatch: DispatchMode,
}

pub struct McpInbound {
    pub message: McpMessage,
    /// 初始化/目录消息没有领域操作上下文。
    pub context: Option<McpDispatchContext>,
}

/// 发送到外部 MCP 客户端的路由信封，避免多个客户端之间的结果串线。
pub struct McpClientMessage {
    pub session_id: String,
    pub message: McpMessage,
}

/// 一个原生 MCP Server 的逻辑通道，与其插件后端共享领域实例。
/// 具体采用独立连接还是复用 stdio 尚待冻结；本接口不启动额外协议连接。
pub trait PluginMcpProvider: Send + Sync {
    fn catalog<'a>(&'a self) -> RuntimeFuture<'a, McpCatalog>;
    fn send<'a>(&'a self, message: McpInbound) -> RuntimeFuture<'a, ()>;
    /// 单一收件任务；等待消息不能阻止并发 send/close。
    fn receive<'a>(&'a self) -> RuntimeFuture<'a, Option<McpMessage>>;
    fn close<'a>(&'a self) -> RuntimeFuture<'a, ()>;
}

/// 路由只调整名称/URI/请求 ID；领域参数、原生结果和错误保持插件定义。
pub trait McpBridge: Send + Sync {
    fn register<'a>(
        &'a self,
        plugin_id: PluginId,
        provider: Box<dyn PluginMcpProvider>,
    ) -> RuntimeFuture<'a, ()>;
    fn unregister<'a>(&'a self, plugin_id: &'a str) -> RuntimeFuture<'a, ()>;
    fn catalog<'a>(&'a self) -> RuntimeFuture<'a, McpCatalog>;
    /// caller.source 必须为 Agent。实例级工具必须显式含 pluginInstanceId，
    /// 且与基座解析出的目标一致；不依据当前标签页补入参数。
    fn forward<'a>(&'a self, caller: CallerContext, message: McpMessage) -> RuntimeFuture<'a, ()>;
    /// 向外部客户端交付的消息，包含异步通知和结果。
    fn receive<'a>(&'a self) -> RuntimeFuture<'a, Option<McpClientMessage>>;
    /// 公共查询/取消由基座提供，不要求插件重复实现状态表。
    fn get_operation<'a>(
        &'a self,
        caller: CallerContext,
        operation_id: &'a str,
    ) -> RuntimeFuture<'a, OperationReceipt>;
    fn cancel_operation<'a>(
        &'a self,
        caller: CallerContext,
        operation_id: &'a str,
    ) -> RuntimeFuture<'a, OperationReceipt>;
}
