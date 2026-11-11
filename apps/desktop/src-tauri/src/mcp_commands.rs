//! 桌面对 Stdio 适配器的私有桥发布。
//!
//! 适配器是桌面可执行文件自身（`coterm-desktop mcp stdio`）。这里负责按需启动可配置端口的
//! 回环地址的本机桥，并把可直接粘贴的客户端配置交给桌面面板。

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use coterm_core::driver_manager::InProcessDriverRuntime;
use coterm_mcp::{config::StdioClientConfiguration, server::BridgeServer};
use serde::{Deserialize, Serialize};

use crate::mcp_settings::{self, StartupMode};

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
    /// Saved device-local port, including while the bridge is disabled.
    pub port: u16,
    pub startup_mode: StartupMode,
    /// A damaged settings file blocks enabling until the user saves a valid port.
    pub configuration_error: Option<String>,
}

struct BridgeState {
    bind_address: SocketAddr,
    active: Option<BridgeServer>,
    startup_mode: StartupMode,
    configuration_error: Option<String>,
}

/// 按需存活的本机桥。修改端口时先绑定并保存新设置，失败保留旧监听。
pub struct StdioBridge {
    runtime: Arc<InProcessDriverRuntime>,
    command: Option<String>,
    settings_path: Option<PathBuf>,
    state: tokio::sync::Mutex<BridgeState>,
}

impl StdioBridge {
    /// `executable` 必须是不经 PATH 解析的绝对路径，客户端才能从任意工作目录启动。
    pub fn new(
        runtime: Arc<InProcessDriverRuntime>,
        executable: Option<PathBuf>,
        settings_path: PathBuf,
    ) -> Self {
        let loaded = mcp_settings::load(&settings_path);
        let settings = loaded.as_ref().cloned().unwrap_or_default();
        let mut bridge = Self::with_address(
            runtime,
            executable,
            SocketAddr::from(([127, 0, 0, 1], settings.port)),
        );
        bridge.state.get_mut().startup_mode = settings.startup_mode;
        bridge.state.get_mut().configuration_error = loaded.err();
        bridge.settings_path = Some(settings_path);
        bridge
    }

    fn with_address(
        runtime: Arc<InProcessDriverRuntime>,
        executable: Option<std::path::PathBuf>,
        bind_address: SocketAddr,
    ) -> Self {
        let command = executable
            .filter(|path| path.is_absolute())
            .map(|path| path.to_string_lossy().into_owned());
        Self {
            runtime,
            command,
            settings_path: None,
            state: tokio::sync::Mutex::new(BridgeState {
                bind_address,
                active: None,
                startup_mode: StartupMode::Auto,
                configuration_error: None,
            }),
        }
    }

    fn snapshot(&self, state: &BridgeState) -> StdioStatus {
        let info = state.active.as_ref().map(BridgeServer::connection_info);
        // 没有适配器时不交付配置：用户无法用它启动任何进程。
        let client_config = match (&self.command, &info) {
            (Some(command), Some(info)) => {
                Some(StdioClientConfiguration::new(command.clone(), &info.address).client_config)
            }
            _ => None,
        };
        StdioStatus {
            enabled: info.is_some(),
            command: self.command.clone(),
            address: info.map(|info| info.address),
            client_config,
            port: state.bind_address.port(),
            startup_mode: state.startup_mode,
            configuration_error: state.configuration_error.clone(),
        }
    }

    pub async fn status(&self) -> StdioStatus {
        let state = self.state.lock().await;
        self.snapshot(&state)
    }

    /// Restore the device startup policy once, before the frontend can issue commands.
    pub async fn start_on_launch(&self) -> Result<(), String> {
        let automatic = self.state.lock().await.startup_mode == StartupMode::Auto;
        if automatic {
            self.set_enabled(true).await?;
        }
        Ok(())
    }

    async fn persist(&self, port: u16, startup_mode: StartupMode) -> Result<(), String> {
        if let Some(path) = self.settings_path.clone() {
            tokio::task::spawn_blocking(move || mcp_settings::save(&path, port, startup_mode))
                .await
                .map_err(|_| "mcp_settings_write_failed")??;
        }
        Ok(())
    }

    /// Automatic mode starts immediately; manual mode preserves the current listener.
    pub async fn set_startup_mode(&self, mode: StartupMode) -> Result<StdioStatus, String> {
        let mut state = self.state.lock().await;
        if let Some(error) = &state.configuration_error {
            return Err(error.clone());
        }
        if state.startup_mode == mode && (mode == StartupMode::Manual || state.active.is_some()) {
            return Ok(self.snapshot(&state));
        }
        let replacement = if mode == StartupMode::Auto && state.active.is_none() {
            self.command.as_ref().ok_or(EXECUTABLE_UNAVAILABLE)?;
            Some(
                BridgeServer::bind_at(self.runtime.clone(), state.bind_address)
                    .await
                    .map_err(|_| "mcp_port_unavailable")?,
            )
        } else {
            None
        };
        self.persist(state.bind_address.port(), mode).await?;
        if let Some(bridge) = replacement {
            state.active = Some(bridge);
        }
        state.startup_mode = mode;
        Ok(self.snapshot(&state))
    }

    /// 幂等：重复启用复用已监听的端口。停用后桥立即关闭。
    pub async fn set_enabled(&self, enabled: bool) -> Result<StdioStatus, String> {
        let mut state = self.state.lock().await;
        if !enabled {
            state.active = None;
            return Ok(self.snapshot(&state));
        }
        if let Some(error) = &state.configuration_error {
            return Err(error.clone());
        }
        // 无法给出可启动的适配器入口时不开监听，也不交付配置。
        self.command.as_ref().ok_or(EXECUTABLE_UNAVAILABLE)?;
        if state.active.is_none() {
            let bridge = BridgeServer::bind_at(self.runtime.clone(), state.bind_address)
                .await
                .map_err(|error| format!("MCP Stdio bridge unavailable: {error}"))?;
            state.active = Some(bridge);
        }
        Ok(self.snapshot(&state))
    }

    pub async fn set_port(&self, port: u16) -> Result<StdioStatus, String> {
        if port == 0 {
            return Err("mcp_port_invalid".into());
        }
        let mut state = self.state.lock().await;
        if state.bind_address.port() == port && state.configuration_error.is_none() {
            return Ok(self.snapshot(&state));
        }
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        // Keep the old service alive until binding AND persistence have succeeded.
        let replacement = if state.active.is_some() {
            Some(
                BridgeServer::bind_at(self.runtime.clone(), address)
                    .await
                    .map_err(|_| "mcp_port_unavailable")?,
            )
        } else {
            None
        };
        // A disabled bridge checks availability without starting an MCP listener.
        let reservation = if state.active.is_none() {
            Some(
                tokio::net::TcpListener::bind(address)
                    .await
                    .map_err(|_| "mcp_port_unavailable")?,
            )
        } else {
            None
        };
        self.persist(port, state.startup_mode).await?;
        state.active = replacement;
        state.bind_address = address;
        state.configuration_error = None;
        drop(reservation);
        Ok(self.snapshot(&state))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetMcpPortRequest {
    pub port: u32,
}

/// Trusted desktop settings only; never registered as an MCP tool.
#[tauri::command]
pub async fn set_mcp_stdio_port(
    request: SetMcpPortRequest,
    state: tauri::State<'_, crate::AppState>,
) -> Result<StdioStatus, String> {
    let port = checked_port(request.port)?;
    state.stdio_bridge.set_port(port).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetMcpStartupModeRequest {
    pub startup_mode: StartupMode,
}

#[tauri::command]
pub async fn set_mcp_startup_mode(
    request: SetMcpStartupModeRequest,
    state: tauri::State<'_, crate::AppState>,
) -> Result<StdioStatus, String> {
    state
        .stdio_bridge
        .set_startup_mode(request.startup_mode)
        .await
}

fn checked_port(port: u32) -> Result<u16, String> {
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| "mcp_port_invalid".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use coterm_proto::runtime::{lifecycle::RuntimeLimits, stream::FlowWindow};
    use std::path::PathBuf;

    fn runtime() -> Arc<InProcessDriverRuntime> {
        Arc::new(InProcessDriverRuntime::new(
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
        assert_eq!(
            server["args"],
            serde_json::json!(["mcp", "stdio", "--addr", enabled.address])
        );
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
            let bridge = StdioBridge::with_address(
                runtime(),
                executable,
                SocketAddr::from(([127, 0, 0, 1], 0)),
            );
            let result = bridge.set_enabled(true).await;
            assert!(matches!(result, Err(message) if message == EXECUTABLE_UNAVAILABLE));
            let status = bridge.status().await;
            assert!(!status.enabled);
            assert!(status.command.is_none());
            assert!(status.client_config.is_none());
        }
    }

    #[tokio::test]
    async fn changing_ports_keeps_the_old_service_on_conflict_and_restores_saved_port(
    ) -> anyhow::Result<()> {
        use tokio::{
            io::AsyncReadExt,
            net::{TcpListener, TcpStream},
            time::{timeout, Duration},
        };
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-port-{}.json", uuid::Uuid::new_v4()));
        let bridge = StdioBridge::new(runtime(), Some(executable()), path.clone());
        let reservation = TcpListener::bind("127.0.0.1:0").await?;
        let first = reservation.local_addr()?.port();
        drop(reservation);
        let saved = bridge.set_port(first).await.map_err(anyhow::Error::msg)?;
        assert!(
            !saved.enabled,
            "editing a disabled bridge does not enable it"
        );
        let enabled = bridge.set_enabled(true).await.map_err(anyhow::Error::msg)?;
        let address = enabled
            .address
            .ok_or_else(|| anyhow::anyhow!("missing address"))?;
        let mut client = TcpStream::connect(&address).await?;
        // Ensure the old server has accepted the client before replacing its listener.
        tokio::io::AsyncWriteExt::write_all(
            &mut client,
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
        )
        .await?;
        let mut reply = [0; 4096];
        assert!(timeout(Duration::from_secs(2), client.read(&mut reply)).await?? > 0);
        let occupied = TcpListener::bind("127.0.0.1:0").await?;
        let second = occupied.local_addr()?.port();
        assert_eq!(
            bridge.set_port(second).await.err().as_deref(),
            Some("mcp_port_unavailable")
        );
        assert_eq!(
            bridge.status().await.address.as_deref(),
            Some(address.as_str())
        );
        assert_eq!(
            mcp_settings::load(&path).map_err(anyhow::Error::msg)?.port,
            first
        );
        drop(occupied);
        let updated = bridge.set_port(second).await.map_err(anyhow::Error::msg)?;
        assert!(updated.enabled);
        assert_eq!(updated.port, second);
        assert_eq!(updated.address, Some(format!("127.0.0.1:{second}")));
        let config: serde_json::Value = serde_json::from_str(
            updated
                .client_config
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing config"))?,
        )?;
        assert_eq!(
            config["mcpServers"]["coterm"]["args"],
            serde_json::json!(["mcp", "stdio", "--addr", format!("127.0.0.1:{second}")])
        );
        let closed = timeout(Duration::from_secs(2), client.read(&mut reply)).await?;
        assert!(
            closed.is_err() || matches!(closed, Ok(0)),
            "old MCP clients are disconnected"
        );
        let again = bridge.set_port(second).await.map_err(anyhow::Error::msg)?;
        assert_eq!(
            again.address, updated.address,
            "same-port save is idempotent"
        );
        let restarted = StdioBridge::new(runtime(), Some(executable()), path.clone());
        assert_eq!(restarted.status().await.port, second);
        assert!(!restarted.status().await.enabled);
        assert!(bridge.set_port(0).await.is_err());
        bridge
            .set_enabled(false)
            .await
            .map_err(anyhow::Error::msg)?;
        std::fs::remove_file(&path)?;
        Ok(())
    }

    #[tokio::test]
    async fn save_failure_preserves_the_old_listener_and_releases_the_new_one() -> anyhow::Result<()>
    {
        let mut bridge =
            StdioBridge::with_address(runtime(), Some(executable()), "127.0.0.1:0".parse()?);
        let enabled = bridge.set_enabled(true).await.map_err(anyhow::Error::msg)?;
        bridge.settings_path = Some(
            std::env::temp_dir()
                .join(format!("coterm-mcp-missing-{}", uuid::Uuid::new_v4()))
                .join("settings.json"),
        );
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = reservation.local_addr()?;
        drop(reservation);
        assert_eq!(
            bridge.set_port(address.port()).await.err().as_deref(),
            Some("mcp_settings_write_failed")
        );
        assert_eq!(bridge.status().await.address, enabled.address);
        let old = enabled
            .address
            .ok_or_else(|| anyhow::anyhow!("missing address"))?;
        let _client = tokio::net::TcpStream::connect(old).await?;
        // Dropping the failed replacement aborts its task; wait until its socket is released.
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Ok(listener) = tokio::net::TcpListener::bind(address).await {
                    break listener;
                }
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn damaged_settings_block_enabling_until_a_valid_port_is_saved() -> anyhow::Result<()> {
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-damaged-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, "damaged")?;
        let bridge = StdioBridge::new(runtime(), Some(executable()), path.clone());
        assert_eq!(
            bridge.status().await.configuration_error.as_deref(),
            Some("mcp_settings_invalid")
        );
        assert!(bridge.set_enabled(true).await.is_err());
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = reservation.local_addr()?.port();
        drop(reservation);
        let fixed = bridge.set_port(port).await.map_err(anyhow::Error::msg)?;
        assert!(fixed.configuration_error.is_none());
        assert_eq!(
            mcp_settings::load(&path).map_err(anyhow::Error::msg)?.port,
            port
        );
        std::fs::remove_file(&path)?;
        Ok(())
    }

    #[tokio::test]
    async fn startup_mode_controls_launch_and_survives_port_changes() -> anyhow::Result<()> {
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-startup-{}.json", uuid::Uuid::new_v4()));
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = reservation.local_addr()?.port();
        drop(reservation);
        mcp_settings::save(&path, port, StartupMode::Manual).map_err(anyhow::Error::msg)?;
        let bridge = StdioBridge::new(runtime(), Some(executable()), path.clone());
        bridge.start_on_launch().await.map_err(anyhow::Error::msg)?;
        assert!(!bridge.status().await.enabled);
        assert_eq!(bridge.status().await.startup_mode, StartupMode::Manual);
        let running = bridge.set_enabled(true).await.map_err(anyhow::Error::msg)?;
        assert_eq!(running.startup_mode, StartupMode::Manual);

        let automatic = bridge
            .set_startup_mode(StartupMode::Auto)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(automatic.enabled);
        assert_eq!(automatic.port, port);
        let saved = mcp_settings::load(&path).map_err(anyhow::Error::msg)?;
        assert_eq!(saved.port, port);
        assert_eq!(saved.startup_mode, StartupMode::Auto);
        let reopened = StdioBridge::new(runtime(), Some(executable()), path.clone());
        assert_eq!(reopened.status().await.startup_mode, StartupMode::Auto);
        // A second process cannot claim success while the first owns the port.
        assert!(reopened.start_on_launch().await.is_err());
        assert!(!reopened.status().await.enabled);

        let manual = bridge
            .set_startup_mode(StartupMode::Manual)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(
            manual.enabled,
            "changing launch policy must not interrupt clients"
        );
        assert_eq!(manual.address, automatic.address);
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let next_port = reservation.local_addr()?.port();
        drop(reservation);
        let moved = bridge
            .set_port(next_port)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(moved.enabled);
        assert_eq!(moved.startup_mode, StartupMode::Manual);
        let saved = mcp_settings::load(&path).map_err(anyhow::Error::msg)?;
        assert_eq!(saved.port, next_port);
        assert_eq!(saved.startup_mode, StartupMode::Manual);
        // Manual mode stays off on a new launch, even if the last session was running.
        let reopened = StdioBridge::new(runtime(), Some(executable()), path.clone());
        reopened
            .start_on_launch()
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(!reopened.status().await.enabled);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[tokio::test]
    async fn automatic_launch_uses_the_saved_port() -> anyhow::Result<()> {
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-auto-{}.json", uuid::Uuid::new_v4()));
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = reservation.local_addr()?.port();
        drop(reservation);
        mcp_settings::save(&path, port, StartupMode::Auto).map_err(anyhow::Error::msg)?;
        let bridge = StdioBridge::new(runtime(), Some(executable()), path.clone());
        bridge.start_on_launch().await.map_err(anyhow::Error::msg)?;
        let status = bridge.status().await;
        assert!(status.enabled);
        assert_eq!(
            status.address.as_deref(),
            Some(format!("127.0.0.1:{port}").as_str())
        );
        let _client = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await?;
        bridge.start_on_launch().await.map_err(anyhow::Error::msg)?;
        assert_eq!(
            bridge.status().await.address,
            status.address,
            "startup is idempotent"
        );
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[tokio::test]
    async fn failed_startup_mode_changes_preserve_policy_and_listener() -> anyhow::Result<()> {
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-mode-{}.json", uuid::Uuid::new_v4()));
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = occupied.local_addr()?;
        mcp_settings::save(&path, address.port(), StartupMode::Manual)
            .map_err(anyhow::Error::msg)?;
        let bridge = StdioBridge::new(runtime(), Some(executable()), path.clone());
        assert!(
            matches!(bridge.set_startup_mode(StartupMode::Auto).await, Err(error) if error == "mcp_port_unavailable")
        );
        assert_eq!(bridge.status().await.startup_mode, StartupMode::Manual);
        assert!(!bridge.status().await.enabled);
        assert_eq!(
            mcp_settings::load(&path)
                .map_err(anyhow::Error::msg)?
                .startup_mode,
            StartupMode::Manual
        );
        std::fs::remove_file(path)?;

        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = reservation.local_addr()?;
        drop(reservation);
        let mut bridge = StdioBridge::with_address(runtime(), Some(executable()), address);
        bridge.state.get_mut().startup_mode = StartupMode::Manual;
        bridge.settings_path = Some(
            std::env::temp_dir()
                .join(uuid::Uuid::new_v4().to_string())
                .join("missing.json"),
        );
        assert!(
            matches!(bridge.set_startup_mode(StartupMode::Auto).await, Err(error) if error == "mcp_settings_write_failed")
        );
        assert_eq!(bridge.status().await.startup_mode, StartupMode::Manual);
        assert!(!bridge.status().await.enabled);
        // The failed replacement listener must release its socket.
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Ok(listener) = tokio::net::TcpListener::bind(address).await {
                    break listener;
                }
                tokio::task::yield_now().await;
            }
        })
        .await?;
        let running = bridge.set_enabled(true).await.map_err(anyhow::Error::msg)?;
        assert!(bridge.set_startup_mode(StartupMode::Auto).await.is_err());
        assert!(bridge.status().await.enabled);
        assert_eq!(bridge.status().await.address, running.address);
        assert_eq!(bridge.status().await.startup_mode, StartupMode::Manual);
        Ok(())
    }

    #[test]
    fn startup_requests_reject_unknown_modes_and_fields() {
        assert!(
            serde_json::from_str::<SetMcpStartupModeRequest>(r#"{"startupMode":"auto"}"#).is_ok()
        );
        assert!(
            serde_json::from_str::<SetMcpStartupModeRequest>(r#"{"startupMode":"manual"}"#).is_ok()
        );
        for invalid in [
            r#"{"startupMode":"other"}"#,
            r#"{"startupMode":"auto","extra":1}"#,
            r#"{}"#,
        ] {
            assert!(serde_json::from_str::<SetMcpStartupModeRequest>(invalid).is_err());
        }
    }

    #[test]
    fn desktop_port_requests_validate_the_full_range() {
        assert_eq!(checked_port(1).ok(), Some(1));
        assert_eq!(checked_port(65535).ok(), Some(65535));
        for invalid in [0, 65536, u32::MAX] {
            assert!(checked_port(invalid).is_err());
        }
    }

    #[tokio::test]
    async fn a_disabled_bridge_checks_occupied_ports_without_saving_or_enabling(
    ) -> anyhow::Result<()> {
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-disabled-{}.json", uuid::Uuid::new_v4()));
        let bridge = StdioBridge::new(runtime(), Some(executable()), path.clone());
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        assert!(bridge
            .set_port(reservation.local_addr()?.port())
            .await
            .is_err());
        assert!(!bridge.status().await.enabled);
        assert_eq!(bridge.status().await.port, mcp_settings::DEFAULT_PORT);
        assert!(!path.exists());
        Ok(())
    }
}
