//! 秘密仅返回受实例、用途和有效期约束的引用，不进入普通领域事件。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{InteractionId, OperationId, PluginInstanceId};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum InteractionKind {
    Text,
    Secret { purpose: String },
    Confirmation,
    Selection { options: Vec<SelectionOption> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionRequest {
    pub interaction_id: InteractionId,
    pub plugin_instance_id: PluginInstanceId,
    pub operation_id: Option<OperationId>,
    pub kind: InteractionKind,
    pub title: String,
    pub prompt: String,
    /// 交互有效期，与操作调用的 waitTimeoutMs 分开。
    pub timeout_ms: Option<u64>,
    pub allow_cancel: bool,
    /// 基座受理交互时写入的绝对期限。插件发出的请求不带此字段，也不依赖它。
    #[serde(default)]
    pub deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretHandle {
    pub id: String,
    pub plugin_instance_id: PluginInstanceId,
    pub purpose: String,
    pub expires_at: DateTime<Utc>,
}

/// MVP private delivery method; never emitted, recorded, or exposed through MCP.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretDelivery {
    pub handle: SecretHandle,
    pub value: String,
}

impl std::fmt::Debug for SecretDelivery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretDelivery")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum InteractionValue {
    Text(String),
    Secret(SecretHandle),
    Confirmation(bool),
    Selection(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", content = "value", rename_all = "snake_case")]
pub enum InteractionOutcome {
    Submitted(InteractionValue),
    Rejected,
    Cancelled,
    TimedOut,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionResult {
    pub interaction_id: InteractionId,
    pub plugin_instance_id: PluginInstanceId,
    pub outcome: InteractionOutcome,
}

/// Reserved control reply from invoke/approve, negotiated via deferred-interactions.
/// This envelope is consumed by the host, never stored as ordinary domain output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferredInteraction {
    #[serde(rename = "_cotermInteraction")]
    pub request: InteractionRequest,
}

/// 不实现 Debug/Serialize/Clone；仅后端 SDK 兑换路径可以访问原始字节。
pub trait SecretMaterial: Send {
    fn expose(&self) -> &[u8];
}
