use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 终端操作发起者
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Actor {
    Human,
    Agent,
}

/// 块级命令项 (Command Block / Item)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandItem {
    pub id: String,
    pub index: u64,
    pub timestamp: DateTime<Utc>,
    pub cwd: Option<String>,
    pub command: String,
    pub exit_code: Option<i32>,
    pub output: String,
    pub actor: Actor,
    pub is_selected: bool,
}

impl CommandItem {
    pub fn new(index: u64, command: String, actor: Actor, cwd: Option<String>) -> Self {
        Self {
            id: format!("cmd_{}", Uuid::new_v4().simple()),
            index,
            timestamp: Utc::now(),
            cwd,
            command,
            exit_code: None,
            output: String::new(),
            actor,
            is_selected: false,
        }
    }
}

/// 人机审批请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub session_id: String,
    pub command: String,
    pub created_at: DateTime<Utc>,
    pub is_risky: bool,
    pub risk_reason: Option<String>,
}

/// 审批裁决结果
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Approved,
    Modified,
    Rejected,
}

/// 插件元信息配置 (manifest.json)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub protocol: String, // "ssh", "serial", "local", etc.
    pub entry_point: String,
    pub config_schema: serde_json::Value,
}

/// JSON-RPC 2.0 基础规范 (跨进程通信)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}
