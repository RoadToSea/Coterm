//! 双向逻辑消息；不指定帧头、字节序、JSON-RPC 或 MCP 物理复用方式。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::interaction::{InteractionRequest, InteractionResult, SecretDelivery};
use super::lifecycle::{CreateInstance, HelloResult, InstanceStatus, RuntimeHello};
use super::operation::{
    ApproveProposal, CancellationOutcome, FailureOutcome, InvokeRequest, OperationDescriptor,
};
use super::stream::{FlowWindow, StreamData, StreamGap, StreamInfo, SubscribeStream};
use super::{
    InteractionId, OperationId, PluginInstanceId, ProposalId, RequestId, RuntimeError,
    SubscriptionId,
};

/// MVP JSON notifications; binary stream framing remains a separate contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PluginNotification {
    OperationCompleted {
        operation_id: OperationId,
        result: Value,
    },
    OperationFailed {
        operation_id: OperationId,
        error: RuntimeError,
    },
    Event {
        plugin_instance_id: PluginInstanceId,
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
#[serde(rename_all = "camelCase")]
pub struct HostMessage {
    pub request_id: RequestId,
    pub command: HostCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "params", rename_all = "snake_case")]
pub enum HostCommand {
    Hello(RuntimeHello),
    CreateInstance(CreateInstance),
    DestroyInstance {
        plugin_instance_id: PluginInstanceId,
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
    /// Private host-to-plugin delivery, separate from operations and events.
    ProvideSecret(SecretDelivery),
    StreamSubscribe(SubscribeStream),
    StreamUnsubscribe {
        subscription_id: SubscriptionId,
    },
    /// 消费确认后补充额度；不代表协议数据已持久化。
    StreamCredit {
        subscription_id: SubscriptionId,
        credit: FlowWindow,
    },
    Shutdown,
}

/// 操作事件按基座生成的 operation_id 关联；请求应答用于生命周期/控制确认。
#[derive(Debug, Clone)]
pub enum PluginEvent {
    HelloResult {
        request_id: RequestId,
        result: Result<HelloResult, RuntimeError>,
    },
    Ready {
        operations: Vec<OperationDescriptor>,
    },
    Reply {
        request_id: RequestId,
        result: Result<Value, RuntimeError>,
    },
    InstanceStatus {
        plugin_instance_id: PluginInstanceId,
        status: InstanceStatus,
    },
    ProposalPrepared {
        operation_id: OperationId,
        proposal_id: ProposalId,
        draft: Value,
    },
    OperationStarted {
        operation_id: OperationId,
    },
    OperationProgress {
        operation_id: OperationId,
        progress: Value,
    },
    OperationOutput {
        operation_id: OperationId,
        output: Value,
    },
    OperationCompleted {
        operation_id: OperationId,
        result: Value,
    },
    OperationFailed {
        operation_id: OperationId,
        error: RuntimeError,
        outcome: FailureOutcome,
    },
    CancellationResult {
        request_id: RequestId,
        operation_id: OperationId,
        outcome: CancellationOutcome,
    },
    StreamOpen(StreamInfo),
    StreamData(StreamData),
    StreamGap(StreamGap),
    StreamClose {
        stream_id: String,
        plugin_instance_id: PluginInstanceId,
        reason: Option<String>,
    },
    InteractionRequest(InteractionRequest),
    Log {
        level: String,
        message: String,
        fields: Value,
    },
}
