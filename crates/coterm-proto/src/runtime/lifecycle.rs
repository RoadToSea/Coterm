//! 进程、实例和界面生命周期互相独立；实例 ready 不代表协议已连接。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::operation::OperationDescriptor;
use super::stream::FlowWindow;
use super::{ErrorCategory, PluginId, PluginInstanceId, RuntimeError};

pub const INTERFACE_MAJOR: u16 = 1;
pub const INTERFACE_MINOR: u16 = 0;
pub const REQUIRED_FEATURES: &[&str] = &["instances", "backend-calls", "operation-catalog"];
pub const OPTIONAL_FEATURES: &[&str] = &[
    "secret-delivery",
    "native-mcp-tools",
    "deferred-interactions",
    "async-operations",
    "plugin-events",
];

fn incompatible(message: &str) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Unsupported,
        code: "plugin_handshake_incompatible".to_owned(),
        message: message.to_owned(),
        details: None,
    }
}

impl RuntimeHello {
    /// 插件仅启用双方支持的特性；未来可选特性可以忽略，必需特性不能忽略。
    pub fn negotiate_features(&self, supported: &[&str]) -> Result<Vec<String>, RuntimeError> {
        if self.interface_major != INTERFACE_MAJOR {
            return Err(incompatible("Unsupported runtime interface"));
        }
        if self.limits.max_control_bytes == 0 {
            return Err(incompatible("Control limit must be positive"));
        }
        if self
            .required_features
            .iter()
            .any(|f| !supported.contains(&f.as_str()))
        {
            return Err(incompatible("Unsupported required runtime feature"));
        }
        let mut enabled = Vec::new();
        for feature in self.required_features.iter().chain(&self.optional_features) {
            if supported.contains(&feature.as_str()) && !enabled.contains(feature) {
                enabled.push(feature.clone());
            }
        }
        if !enabled.iter().any(|f| f == "secret-delivery") {
            if self
                .required_features
                .iter()
                .any(|f| f == "deferred-interactions")
            {
                return Err(incompatible(
                    "v1 secret interactions require secret-delivery",
                ));
            }
            enabled.retain(|f| f != "deferred-interactions");
        }
        Ok(enabled)
    }

    /// 与当前 JSON 适配器协商；流字段只保留原值，不宣称已支持流。
    pub fn control_limits(&self) -> RuntimeLimits {
        RuntimeLimits {
            max_control_bytes: self
                .limits
                .max_control_bytes
                .min(super::wire::MAX_CONTROL_BYTES as u32),
            ..self.limits.clone()
        }
    }

    /// 宿主与 SDK 共用结果校验；身份和操作目录由宿主另行校验。
    pub fn validate_result(&self, result: &HelloResult) -> Result<(), RuntimeError> {
        let enabled = &result.enabled_features;
        let limit = result.limits.max_control_bytes;
        let duplicates = enabled
            .iter()
            .enumerate()
            .any(|(index, f)| enabled[..index].contains(f));
        if self.interface_major != INTERFACE_MAJOR
            || result.interface_minor > self.interface_minor
            || result.interface_minor > INTERFACE_MINOR
            || !self.required_features.iter().all(|f| enabled.contains(f))
            || enabled
                .iter()
                .any(|f| !self.required_features.contains(f) && !self.optional_features.contains(f))
            || duplicates
            || limit == 0
            || limit > self.control_limits().max_control_bytes
            || result.limits.max_data_chunk_bytes != self.limits.max_data_chunk_bytes
            || result.limits.initial_window.chunks != self.limits.initial_window.chunks
            || result.limits.initial_window.bytes != self.limits.initial_window.bytes
        {
            return Err(incompatible(
                "Invalid negotiated version, features or limits",
            ));
        }
        if enabled.iter().any(|f| f == "deferred-interactions")
            && !enabled.iter().any(|f| f == "secret-delivery")
        {
            return Err(incompatible(
                "v1 secret interactions require secret-delivery",
            ));
        }
        if !result.mcp_tools.is_empty() && !enabled.iter().any(|f| f == "native-mcp-tools") {
            return Err(incompatible("MCP tools were not negotiated"));
        }
        Ok(())
    }
}

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
