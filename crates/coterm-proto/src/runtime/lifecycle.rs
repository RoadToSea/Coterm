//! 进程、实例和界面生命周期互相独立；实例 ready 不代表协议已连接。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::operation::OperationDescriptor;
use super::stream::FlowWindow;
use super::{PluginId, PluginInstanceId};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHello {
    pub interface_major: u16,
    pub interface_minor: u16,
    pub required_features: Vec<String>,
    pub optional_features: Vec<String>,
    pub limits: RuntimeLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLimits {
    pub max_control_bytes: u32,
    pub max_data_chunk_bytes: u32,
    pub initial_window: FlowWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloResult {
    pub plugin_id: PluginId,
    pub plugin_version: String,
    pub interface_minor: u16,
    pub enabled_features: Vec<String>,
    pub limits: RuntimeLimits,
    /// Operation contracts are registered by the host at process startup.
    #[serde(default)]
    pub operations: Vec<OperationDescriptor>,
    /// Plugin-authored native MCP tools; never inferred from the operation catalog.
    #[serde(default)]
    pub mcp_tools: Vec<McpToolRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolRegistration {
    /// Operation used for host source attribution, approval and history.
    pub operation: String,
    /// Native MCP Tool descriptor, including the plugin's inputSchema.
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
    pub plugin_id: PluginId,
    pub plugin_instance_id: PluginInstanceId,
    pub session_id: String,
    pub label: String,
    pub status: InstanceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInstance {
    /// 由基座分配；插件不得自行切换到另一连接。
    pub instance: InstanceInfo,
    pub config: Value,
}
