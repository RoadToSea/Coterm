//! 实例生命周期与运行时容量；ready 不代表协议已连接。
use super::{DriverId, DriverInstanceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 单次控制请求、回复和通知的 JSON 大小上限；队列容量独立受限。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLimits {
    pub max_control_bytes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolRegistration {
    /// Operation used for host source attribution, approval and history.
    pub operation: String,
    /// Native MCP Tool descriptor, including the driver's inputSchema.
    pub descriptor: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Starting,
    Ready,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceInfo {
    pub driver_id: DriverId,
    pub driver_instance_id: DriverInstanceId,
    pub session_id: String,
    pub label: String,
    pub status: InstanceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInstance {
    /// 由基座分配；驱动不得自行切换到另一连接。
    pub instance: InstanceInfo,
    pub config: Value,
}
