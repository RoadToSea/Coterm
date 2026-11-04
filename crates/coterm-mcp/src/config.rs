//! 发布给本机 MCP 客户端的 Stdio 适配器配置。
//!
//! 适配器是桌面可执行文件自身的 `mcp stdio` 子命令，因此无需独立分发二进制。
//! 桥使用本机回环地址；非默认端口通过适配器的 `--addr` 参数交付。

use serde::Serialize;
use serde_json::json;

/// 基于桌面可执行文件的客户端配置。`command` 必须为绝对路径。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StdioClientConfiguration {
    pub command: String,
    /// 私有桥的回环地址，供面板展示。
    pub address: String,
    /// 不含秘密的完整客户端配置。
    pub client_config: String,
}

impl StdioClientConfiguration {
    pub fn new(command: String, address: &str) -> Self {
        Self {
            client_config: client_config_at(&command, address),
            command,
            address: address.to_owned(),
        }
    }
}

/// 与 MCP 客户端自身设置文件相同的文档结构。`args` 固定为 `mcp stdio`，
/// 桌面在参数模式下不初始化窗口或运行时。
pub fn client_config(command: &str) -> String {
    client_config_at(command, crate::server::DEFAULT_BRIDGE_ADDRESS)
}

pub fn client_config_at(command: &str, address: &str) -> String {
    let mut args = vec!["mcp", "stdio"];
    if address != crate::server::DEFAULT_BRIDGE_ADDRESS {
        args.extend(["--addr", address]);
    }
    let document = json!({
        "mcpServers": {
            "coterm": {
                "command": command,
                "args": args
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

    #[test]
    fn custom_address_is_delivered_to_the_adapter() -> anyhow::Result<()> {
        let configuration =
            StdioClientConfiguration::new("/opt/coterm/coterm-desktop".into(), "127.0.0.1:12345");
        let parsed: serde_json::Value = serde_json::from_str(&configuration.client_config)?;
        assert_eq!(
            parsed["mcpServers"]["coterm"]["args"],
            json!(["mcp", "stdio", "--addr", "127.0.0.1:12345"])
        );
        assert_eq!(configuration.address, "127.0.0.1:12345");
        Ok(())
    }
}
