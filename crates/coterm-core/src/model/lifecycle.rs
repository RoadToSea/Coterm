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
    /// 唯一实例名称，桌面显示与 MCP name 路由使用同一个值。
    #[serde(alias = "label")]
    pub name: String,
    pub status: InstanceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInstance {
    /// 由基座分配；驱动不得自行切换到另一连接。
    pub instance: InstanceInfo,
    pub config: Value,
}

/// Trim an instance name and return its canonical String.
/// Empty names, controls and names exceeding the UTF-8 byte budget return instance_name_invalid.
pub fn normalize_instance_name(name: &str) -> Result<String, super::RuntimeError> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > crate::limits::INSTANCE_NAME_MAX_BYTES
        || name.chars().any(char::is_control)
    {
        return Err(super::RuntimeError {
            category: super::ErrorCategory::InvalidRequest,
            code: "instance_name_invalid".into(),
            message: "Use a nonempty instance name within the UTF-8 byte limit, without control characters.".into(),
            details: None,
        });
    }
    Ok(name.to_owned())
}
