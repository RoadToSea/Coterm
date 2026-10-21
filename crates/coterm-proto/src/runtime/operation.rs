//! 基座生成操作 ID 并保存最近 N 步。插件只报告执行事实和领域结果。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OperationId, PluginId, PluginInstanceId, ProposalId, RuntimeError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationSource {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationEffect {
    Read,
    Draft,
    Commit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationScope {
    Plugin,
    Instance,
}

/// 基座据此路由和审批；effect 从注册目录读取，不信任调用参数自报。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationDescriptor {
    pub name: String,
    pub scope: OperationScope,
    pub effect: OperationEffect,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    /// Suppress raw inputs in the public ledger (e.g. interactive terminal keys).
    #[serde(default)]
    pub sensitive_input: bool,
    /// Background reads use a separate bounded ledger, never evict user actions.
    #[serde(default)]
    pub transient: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Accepted,
    AwaitingApproval,
    Dispatched,
    Running,
    Completed,
    Failed,
    Rejected,
    Cancelled,
    Unknown,
}

/// Execute 允许执行；PrepareProposal 仅校验/填写草稿，禁止产生发送副作用。
/// 后者用于一个 MCP 工具同时包含“填写+发送”的情况。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DispatchMode {
    Execute,
    PrepareProposal { proposal_id: ProposalId },
}

/// 内部派发上下文，由基座构造。不是前端或外部 Agent 的输入类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvokeRequest {
    pub operation_id: OperationId,
    pub plugin_instance_id: Option<PluginInstanceId>,
    pub source: OperationSource,
    pub effect: OperationEffect,
    pub operation: String,
    pub args: Value,
    pub dispatch: DispatchMode,
}

/// 用户批准并发送当前草稿；args 是本次批准的快照，不能迟到地换成新草稿。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveProposal {
    pub operation_id: OperationId,
    pub proposal_id: ProposalId,
    pub plugin_instance_id: Option<PluginInstanceId>,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationReceipt {
    pub operation_id: OperationId,
    pub proposal_id: Option<ProposalId>,
    pub status: OperationStatus,
    pub wait_expired: bool,
    /// 未完成时为空；等待超时本身不是 error，也不会取消操作。
    pub output: Option<Value>,
    pub error: Option<RuntimeError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecord {
    pub operation_id: OperationId,
    pub plugin_id: PluginId,
    pub plugin_instance_id: Option<PluginInstanceId>,
    pub source: OperationSource,
    pub effect: OperationEffect,
    pub operation: String,
    pub status: OperationStatus,
    /// 脱敏后的有界摘要；原始领域数据和完整流由插件保存。
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<RuntimeError>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub proposal_id: Option<ProposalId>,
    pub wait_expired: bool,
    #[serde(default)]
    pub transient: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureOutcome {
    Failed,
    Cancelled,
    Unknown,
}

/// 收到取消请求不等于成功取消；已派发且无结果时不能推断未提交。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationOutcome {
    NotSubmitted,
    Stopped,
    Unsupported,
    Unknown,
}
