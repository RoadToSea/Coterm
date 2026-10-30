//! 发布给本机 MCP 客户端的 Stdio 适配器配置。
//!
//! 适配器是桌面可执行文件自身的 `mcp stdio` 子命令，因此无需独立分发二进制。
//! 桥使用固定的本机回环地址；配置只需要桌面可执行文件自身的 Stdio 子命令。

use serde::Serialize;
use serde_json::json;

/// 基于桌面可执行文件的客户端配置。`command` 必须为绝对路径。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StdioClientConfiguration {
    pub command: String,
    /// 固定私有桥的回环地址，供面板展示。
    pub address: String,
    /// 不含秘密的完整客户端配置。
    pub client_config: String,
}

impl StdioClientConfiguration {
    pub fn new(command: String, address: &str) -> Self {
        Self {
            client_config: client_config(&command),
            command,
            address: address.to_owned(),
        }
    }
}

/// 与 MCP 客户端自身设置文件相同的文档结构。`args` 固定为 `mcp stdio`，
/// 桌面在参数模式下不初始化窗口或运行时。
pub fn client_config(command: &str) -> String {
    let document = json!({
        "mcpServers": {
            "coterm": {
                "command": command,
                "args": ["mcp", "stdio"]
            }
        }
    });
    serde_json::to_string_pretty(&document).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_config_starts_the_desktop_adapter_subcommand_without_credentials() {
        let rendered = client_config(r"C:\Program Files\Coterm\coterm-desktop.exe");
        let parsed: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
        let server = &parsed["mcpServers"]["coterm"];
        assert_eq!(
            server["command"],
            r"C:\Program Files\Coterm\coterm-desktop.exe"
        );
        assert_eq!(server["args"], json!(["mcp", "stdio"]));
        assert!(server.get("env").is_none());
    }

    #[test]
    fn configuration_exposes_the_fixed_address_without_credentials() {
        let configuration = StdioClientConfiguration::new(
            "/opt/coterm/coterm-desktop".to_owned(),
            "127.0.0.1:51234",
        );
        assert_eq!(configuration.address, "127.0.0.1:51234");
        assert!(!configuration.client_config.contains("token"));
        assert!(!configuration.client_config.contains("env"));
    }
}
