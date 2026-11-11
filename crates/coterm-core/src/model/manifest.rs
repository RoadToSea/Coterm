//! 编译期内置驱动声明，不规定协议配置和领域数据。

use serde::{Deserialize, Serialize};

use super::DriverId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverManifest {
    pub manifest_version: u16,
    pub id: DriverId,
    pub name: String,
    pub version: String,
    pub runtime: RuntimeRequirement,
    pub frontend: Option<FrontendEntry>,
    pub mcp: McpEntry,
    pub instances: InstanceDeclaration,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRequirement {
    pub interface_major: u16,
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendEntry {
    pub entry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpEntry {
    pub provider: McpProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpProvider {
    Backend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceDeclaration {
    #[serde(rename = "type")]
    pub instance_type: String,
    /// 每个内置驱动必须支持多个独立实例。
    pub multiple: bool,
}
