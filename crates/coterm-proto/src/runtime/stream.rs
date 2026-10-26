//! 预留流类型，未纳入 v1.0 JSON 控制契约，不可发送到当前适配器。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OperationId, PluginInstanceId, StreamId, SubscriptionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowWindow {
    pub chunks: u32,
    pub bytes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowPolicy {
    Pause,
    DropWithGap,
    Close,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeStream {
    pub subscription_id: SubscriptionId,
    pub plugin_instance_id: PluginInstanceId,
    pub stream_id: StreamId,
    pub window: FlowWindow,
    pub overflow: OverflowPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub stream_id: StreamId,
    pub plugin_instance_id: PluginInstanceId,
    pub operation_id: Option<OperationId>,
    pub content_type: String,
    pub metadata: Value,
}

/// 仅进程内的逻辑载荷。Binary 对应独立数据帧，具体编码待评审。
#[derive(Debug, Clone)]
pub enum StreamPayload {
    Json(Value),
    Binary(Vec<u8>),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct StreamData {
    pub stream_id: StreamId,
    pub subscription_id: SubscriptionId,
    pub plugin_instance_id: PluginInstanceId,
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub payload: StreamPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamGap {
    pub stream_id: StreamId,
    pub subscription_id: SubscriptionId,
    pub plugin_instance_id: PluginInstanceId,
    pub from_sequence: u64,
    pub to_sequence: u64,
    pub reason: String,
    pub recoverable: bool,
}
