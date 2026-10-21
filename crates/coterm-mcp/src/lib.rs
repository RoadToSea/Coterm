use serde::{Deserialize, Serialize};

pub mod bridge;
pub mod http;
pub mod server;
pub mod transport;

/// 获取命令块的 MCP 工具入参
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetCommandBlocksParams {
    pub session_id: String,
    pub target: CommandBlockTarget,
    pub item_ids: Option<Vec<String>>,
    #[serde(default = "default_max_lines")]
    pub max_output_lines: usize,
}

fn default_max_lines() -> usize {
    100
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandBlockTarget {
    Selected,
    LastFailed,
    LastN,
    ById,
}

/// 发送按键/命令的 MCP 工具入参
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendKeysParams {
    pub session_id: String,
    pub keys: String,
    pub timeout_ms: Option<u64>,
    pub prompt_regex: Option<String>,
}

pub struct McpGateway {
    // 待后续挂载动态插件 Tools 与会话转发
}

impl Default for McpGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl McpGateway {
    pub fn new() -> Self {
        Self {}
    }
}
