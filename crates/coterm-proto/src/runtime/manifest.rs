//! 插件包声明。只描述装配信息，不规定协议配置和领域数据。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::PluginId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub manifest_version: u16,
    pub id: PluginId,
    pub name: String,
    pub version: String,
    pub runtime: RuntimeRequirement,
    /// 平台标识 -> 包内相对入口路径；不得越出插件包。
    pub backend: BTreeMap<String, String>,
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
    /// 社区插件须支持多实例；保留此字段用于校验清单。
    pub multiple: bool,
}
