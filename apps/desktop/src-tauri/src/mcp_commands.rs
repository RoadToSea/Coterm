//! 桌面对 Stdio 适配器的私有桥发布。
//!
//! 适配器是桌面可执行文件自身（`coterm-desktop mcp stdio`）。这里负责按需启动固定
//! 回环地址的本机桥，并把可直接粘贴的客户端配置交给桌面面板。

use std::{net::SocketAddr, sync::Arc};

use coterm_core::plugin_manager::ProcessPluginRuntime;
use coterm_mcp::{
    config::StdioClientConfiguration,
    server::{BridgeServer, McpConnectionInfo},
};
use serde::Serialize;

/// 启用时未找到当前可执行文件，无法给出可启动的客户端配置。
const EXECUTABLE_UNAVAILABLE: &str = "mcp_stdio_executable_unavailable";

/// 面板展示的 Stdio 状态；客户端配置不含秘密。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StdioStatus {
    /// 桥是否正在监听。
    pub enabled: bool,
    /// 适配器入口，即桌面可执行文件；解析失败时为 null。
    pub command: Option<String>,
    pub address: Option<String>,
    /// 不含秘密的完整客户端配置；未启用时为 null。
    pub client_config: Option<String>,
}

/// 按需存活的本机桥。启用时绑定固定回环端口；停用即丢弃监听。
pub struct StdioBridge {
    runtime: Arc<ProcessPluginRuntime>,
    command: Option<String>,
    bind_address: SocketAddr,
    active: tokio::sync::Mutex<Option<BridgeServer>>,
}

impl StdioBridge {
    /// `executable` 必须是不经 PATH 解析的绝对路径，客户端才能从任意工作目录启动。
    pub fn new(runtime: Arc<ProcessPluginRuntime>, executable: Option<std::path::PathBuf>) -> Self {
        Self::with_address(
            runtime,
            executable,
            SocketAddr::from(([127, 0, 0, 1], 51234)),
        )
    }

    fn with_address(
        runtime: Arc<ProcessPluginRuntime>,
        executable: Option<std::path::PathBuf>,
        bind_address: SocketAddr,
    ) -> Self {
        let command = executable
            .filter(|path| path.is_absolute())
            .map(|path| path.to_string_lossy().into_owned());
        Self {
            runtime,
            command,
            bind_address,
            active: tokio::sync::Mutex::new(None),
        }
    }

    fn snapshot(&self, info: Option<&McpConnectionInfo>) -> StdioStatus {
        // 没有适配器时不交付配置：用户无法用它启动任何进程。
        let client_config = match (&self.command, info) {
            (Some(command), Some(info)) => {
                Some(StdioClientConfiguration::new(command.clone(), &info.address).client_config)
            }
            _ => None,
        };
        StdioStatus {
            enabled: info.is_some(),
            command: self.command.clone(),
            address: info.map(|info| info.address.clone()),
            client_config,
        }
    }

    pub async fn status(&self) -> StdioStatus {
        let active = self.active.lock().await;
        self.snapshot(active.as_ref().map(BridgeServer::connection_info).as_ref())
    }

    /// 幂等：重复启用复用已监听的固定端口。停用后桥立即关闭。
    pub async fn set_enabled(&self, enabled: bool) -> Result<StdioStatus, String> {
        let mut active = self.active.lock().await;
        if !enabled {
            *active = None;
            return Ok(self.snapshot(None));
        }
        // 无法给出可启动的适配器入口时不开监听，也不交付配置。
        self.command.as_ref().ok_or(EXECUTABLE_UNAVAILABLE)?;
        if active.is_none() {
            let bridge = BridgeServer::bind_at(self.runtime.clone(), self.bind_address)
                .await
                .map_err(|error| format!("MCP Stdio bridge unavailable: {error}"))?;
            *active = Some(bridge);
        }
        Ok(self.snapshot(active.as_ref().map(BridgeServer::connection_info).as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coterm_proto::runtime::{lifecycle::RuntimeLimits, stream::FlowWindow};
    use std::path::PathBuf;

    fn runtime() -> Arc<ProcessPluginRuntime> {
        Arc::new(ProcessPluginRuntime::new(
            "test",
            RuntimeLimits {
                max_control_bytes: 1024 * 1024,
                max_data_chunk_bytes: 8192,
                initial_window: FlowWindow {
                    chunks: 16,
                    bytes: 1024 * 1024,
                },
            },
        ))
    }

    fn executable() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Program Files\Coterm\coterm-desktop.exe")
        } else {
            PathBuf::from("/opt/coterm/coterm-desktop")
        }
    }

    #[tokio::test]
    async fn enabling_publishes_a_client_configuration_and_disabling_closes_it() {
        let bridge = StdioBridge::with_address(
            runtime(),
            Some(executable()),
            "127.0.0.1:0".parse().expect("socket address"),
        );
        let idle = bridge.status().await;
        assert!(!idle.enabled);
        assert!(idle.client_config.is_none());
        assert_eq!(idle.command.as_deref(), executable().to_str());

        let enabled = bridge.set_enabled(true).await.expect("bridge");
        assert!(enabled.enabled);
        let configuration = enabled.client_config.clone().expect("configuration");
        let parsed: serde_json::Value = serde_json::from_str(&configuration).expect("JSON");
        let server = &parsed["mcpServers"]["coterm"];
        assert_eq!(server["command"], executable().to_str().unwrap_or_default());
        assert_eq!(server["args"], serde_json::json!(["mcp", "stdio"]));
        assert!(server.get("env").is_none());
        // 重复启用复用固定地址，配置可反复复制。
        let again = bridge.set_enabled(true).await.expect("bridge");
        assert_eq!(again.client_config, Some(configuration));

        let disabled = bridge.set_enabled(false).await.expect("disabled");
        assert!(!disabled.enabled);
        assert!(disabled.client_config.is_none());
        assert!(disabled.address.is_none());
        assert!(
            disabled.command.is_some(),
            "the adapter path outlives the bridge"
        );
    }

    #[tokio::test]
    async fn a_relative_or_missing_executable_is_not_published() {
        for executable in [None, Some(PathBuf::from("coterm-desktop.exe"))] {
            let bridge = StdioBridge::new(runtime(), executable);
            let result = bridge.set_enabled(true).await;
            assert!(matches!(result, Err(message) if message == EXECUTABLE_UNAVAILABLE));
            let status = bridge.status().await;
            assert!(!status.enabled);
            assert!(status.command.is_none());
            assert!(status.client_config.is_none());
        }
    }
}
