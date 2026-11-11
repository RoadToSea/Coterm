//! 进程内驱动的类型化控制请求、声明和有界通知。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::interaction::{InteractionResult, SecretDelivery};
use crate::model::lifecycle::CreateInstance;
use crate::model::operation::{ApproveProposal, InvokeRequest};
use crate::model::{DriverInstanceId, InteractionId, OperationId, ProposalId};

/// 驱动领域事件与异步操作完成通知；通过独立的有界队列投递。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DriverNotification {
    OperationCompleted {
        operation_id: OperationId,
        result: Value,
    },
    OperationFailed {
        operation_id: OperationId,
        error: RuntimeError,
    },
    Event {
        driver_instance_id: DriverInstanceId,
        name: String,
        payload: Value,
    },
}

/// A control reply accepting work whose result will arrive in a notification.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeferredOperation {
    #[serde(rename = "_cotermDeferred")]
    pub operation_id: OperationId,
}

/// 单一取消对象，避免多个可选 ID 相互冲突。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum CancelTarget {
    Operation(OperationId),
    Interaction(InteractionId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "params",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum HostCommand {
    CreateInstance(CreateInstance),
    DestroyInstance {
        driver_instance_id: DriverInstanceId,
    },
    Invoke(InvokeRequest),
    Approve(ApproveProposal),
    Reject {
        operation_id: OperationId,
        proposal_id: ProposalId,
        reason: Option<String>,
    },
    Cancel(CancelTarget),
    InteractionResult(InteractionResult),
    /// Private host-to-driver delivery, separate from operations and events.
    ProvideSecret(SecretDelivery),
    Shutdown,
}

use crate::model::{
    lifecycle::McpToolRegistration, operation::OperationDescriptor, ErrorCategory, RuntimeError,
};
use tokio::sync::mpsc;

pub const MAX_CONTROL_BYTES: usize = 1024 * 1024;
pub const INTERFACE_MAJOR: u16 = 2;
pub const REQUIRED_FEATURES: &[&str] = &["instances", "backend-calls", "operation-catalog"];
pub const OPTIONAL_FEATURES: &[&str] = &[
    "secret-delivery",
    "native-mcp-tools",
    "deferred-interactions",
    "async-operations",
    "driver-events",
];

/// 编译期驱动声明；注册时校验身份、能力和目录，不进行通信协商。
#[derive(Clone)]
pub struct DriverDefinition {
    pub driver_id: String,
    pub driver_version: String,
    pub features: Vec<String>,
    pub operations: Vec<OperationDescriptor>,
    pub mcp_tools: Vec<McpToolRegistration>,
}

/// 宿主按驱动串行处理请求；实现不得执行阻塞 IO。
pub trait DriverBackend: Send {
    /// 返回固定身份、能力及目录；不得连接设备或产生领域副作用。
    fn definition() -> DriverDefinition;
    /// 接收类型化控制请求，返回领域 JSON 或结构化错误；长操作立即返回延迟收据。
    fn request(
        &mut self,
        command: HostCommand,
    ) -> impl std::future::Future<Output = Result<Value, RuntimeError>> + Send;
    /// 空闲期间清理过期状态；不得阻塞。
    fn tick(&mut self) {}
    /// 取走有界通知接收端；重复调用返回 None，停止时释放发送端。
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<DriverNotification>>;
}

pub(crate) fn rejected(category: ErrorCategory, code: &str) -> RuntimeError {
    RuntimeError {
        category,
        code: code.into(),
        message: code.replace('_', " "),
        details: None,
    }
}

pub(crate) async fn dispatch<B: DriverBackend>(
    backend: &mut B,
    command: HostCommand,
    features: &[String],
) -> Result<Value, RuntimeError> {
    let required = match &command {
        HostCommand::ProvideSecret(_) => Some("secret-delivery"),
        HostCommand::InteractionResult(_) => Some("deferred-interactions"),
        _ => None,
    };
    if required.is_some_and(|feature| !features.iter().any(|f| f == feature)) {
        return Err(rejected(
            ErrorCategory::Unsupported,
            "runtime_feature_not_supported",
        ));
    }
    backend.request(command).await
}
