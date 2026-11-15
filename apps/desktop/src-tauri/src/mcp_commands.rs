//! Desktop-owned lifecycle and configuration for the in-process MCP HTTP endpoint.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use coterm_core::runtime::Runtime;
use coterm_mcp::{config::client_config, http::HttpServer};
use serde::{Deserialize, Serialize};

use crate::mcp_settings::{self, StartupMode};

/// Current HTTP service state and copyable client configuration; no credentials.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// HTTP 服务是否正在监听。
    pub enabled: bool,
    /// Active Streamable HTTP endpoint; null while stopped.
    pub url: Option<String>,
    /// 不含秘密的完整客户端配置；未启用时为 null。
    pub client_config: Option<String>,
    /// Saved device-local port, including while the service is disabled.
    pub port: u16,
    pub startup_mode: StartupMode,
    /// A damaged settings file blocks enabling until the user saves a valid port.
    pub configuration_error: Option<String>,
}

struct ServiceState {
    bind_address: SocketAddr,
    active: Option<HttpServer>,
    startup_mode: StartupMode,
    configuration_error: Option<String>,
}

/// Desktop-owned HTTP listener; port and startup updates preserve the old service on failure.
pub struct McpService {
    runtime: Arc<Runtime>,
    settings_path: Option<PathBuf>,
    state: tokio::sync::Mutex<ServiceState>,
}

impl McpService {
    /// Load existing device-local port and startup preferences without starting a listener.
    pub fn new(runtime: Arc<Runtime>, settings_path: PathBuf) -> Self {
        let loaded = mcp_settings::load(&settings_path);
        let settings = loaded.as_ref().cloned().unwrap_or_default();
        let mut service =
            Self::with_address(runtime, SocketAddr::from(([127, 0, 0, 1], settings.port)));
        service.state.get_mut().startup_mode = settings.startup_mode;
        service.state.get_mut().configuration_error = loaded.err();
        service.settings_path = Some(settings_path);
        service
    }

    fn with_address(runtime: Arc<Runtime>, bind_address: SocketAddr) -> Self {
        Self {
            runtime,
            settings_path: None,
            state: tokio::sync::Mutex::new(ServiceState {
                bind_address,
                active: None,
                startup_mode: StartupMode::Auto,
                configuration_error: None,
            }),
        }
    }

    fn snapshot(&self, state: &ServiceState) -> McpStatus {
        let url = state.active.as_ref().map(HttpServer::url);
        McpStatus {
            enabled: url.is_some(),
            client_config: url.as_deref().map(client_config),
            url,
            port: state.bind_address.port(),
            startup_mode: state.startup_mode,
            configuration_error: state.configuration_error.clone(),
        }
    }

    pub async fn status(&self) -> McpStatus {
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
    pub async fn set_startup_mode(&self, mode: StartupMode) -> Result<McpStatus, String> {
        let mut state = self.state.lock().await;
        if let Some(error) = &state.configuration_error {
            return Err(error.clone());
        }
        if state.startup_mode == mode && (mode == StartupMode::Manual || state.active.is_some()) {
            return Ok(self.snapshot(&state));
        }
        let replacement = if mode == StartupMode::Auto && state.active.is_none() {
            Some(
                HttpServer::bind_at(self.runtime.clone(), state.bind_address)
                    .await
                    .map_err(|_| "mcp_port_unavailable")?,
            )
        } else {
            None
        };
        self.persist(state.bind_address.port(), mode).await?;
        if let Some(server) = replacement {
            if let Some(previous) = state.active.replace(server) {
                previous.shutdown().await;
            }
        }
        state.startup_mode = mode;
        Ok(self.snapshot(&state))
    }

    /// Idempotently start or stop HTTP; returns the snapshot or a port/settings error.
    pub async fn set_enabled(&self, enabled: bool) -> Result<McpStatus, String> {
        let mut state = self.state.lock().await;
        if !enabled {
            if let Some(server) = state.active.take() {
                server.shutdown().await;
            }
            return Ok(self.snapshot(&state));
        }
        if let Some(error) = &state.configuration_error {
            return Err(error.clone());
        }
        if state.active.is_none() {
            let service = HttpServer::bind_at(self.runtime.clone(), state.bind_address)
                .await
                .map_err(|_| "mcp_port_unavailable")?;
            state.active = Some(service);
        }
        Ok(self.snapshot(&state))
    }

    /// Bind and persist nonzero `port` before retiring the old listener.
    /// Invalid/occupied ports or save failures leave the previous service and settings intact.
    pub async fn set_port(&self, port: u16) -> Result<McpStatus, String> {
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
                HttpServer::bind_at(self.runtime.clone(), address)
                    .await
                    .map_err(|_| "mcp_port_unavailable")?,
            )
        } else {
            None
        };
        // A disabled service checks availability without starting an MCP listener.
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
        let previous = std::mem::replace(&mut state.active, replacement);
        if let Some(previous) = previous {
            previous.shutdown().await;
        }
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
pub async fn set_mcp_port(
    request: SetMcpPortRequest,
    state: tauri::State<'_, crate::AppState>,
) -> Result<McpStatus, String> {
    let port = checked_port(request.port)?;
    state.mcp_service.set_port(port).await
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
) -> Result<McpStatus, String> {
    state
        .mcp_service
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
    use coterm_core::model::lifecycle::RuntimeLimits;

    fn authority(url: &str) -> &str {
        url.strip_prefix("http://")
            .and_then(|url| url.strip_suffix("/mcp"))
            .expect("local HTTP URL")
    }

    fn runtime() -> Arc<Runtime> {
        Arc::new(Runtime::new(RuntimeLimits {
            max_control_bytes: 1024 * 1024,
        }))
    }

    #[tokio::test]
    async fn enabling_publishes_a_client_configuration_and_disabling_closes_it() {
        let service =
            McpService::with_address(runtime(), "127.0.0.1:0".parse().expect("socket address"));
        let idle = service.status().await;
        assert!(!idle.enabled);
        assert!(idle.client_config.is_none());

        let enabled = service.set_enabled(true).await.expect("service");
        assert!(enabled.enabled);
        let configuration = enabled.client_config.clone().expect("configuration");
        let parsed: serde_json::Value = serde_json::from_str(&configuration).expect("JSON");
        let server = &parsed["mcpServers"]["coterm"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], serde_json::json!(enabled.url));
        assert!(server.get("env").is_none());
        // 重复启用复用固定地址，配置可反复复制。
        let again = service.set_enabled(true).await.expect("service");
        assert_eq!(again.client_config, Some(configuration));

        let disabled = service.set_enabled(false).await.expect("disabled");
        assert!(!disabled.enabled);
        assert!(disabled.client_config.is_none());
        assert!(disabled.url.is_none());
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
        let service = McpService::new(runtime(), path.clone());
        let reservation = TcpListener::bind("127.0.0.1:0").await?;
        let first = reservation.local_addr()?.port();
        drop(reservation);
        let saved = service.set_port(first).await.map_err(anyhow::Error::msg)?;
        assert!(
            !saved.enabled,
            "editing a disabled service does not enable it"
        );
        let enabled = service
            .set_enabled(true)
            .await
            .map_err(anyhow::Error::msg)?;
        let address = enabled
            .url
            .ok_or_else(|| anyhow::anyhow!("missing address"))?;
        let mut client = TcpStream::connect(authority(&address)).await?;
        // Ensure the old server has accepted the client before replacing its listener.
        tokio::io::AsyncWriteExt::write_all(
            &mut client,
            format!("GET /mcp HTTP/1.1\r\nHost: {}\r\n\r\n", authority(&address)).as_bytes(),
        )
        .await?;
        let mut reply = [0; 4096];
        assert!(timeout(Duration::from_secs(2), client.read(&mut reply)).await?? > 0);
        let occupied = TcpListener::bind("127.0.0.1:0").await?;
        let second = occupied.local_addr()?.port();
        assert_eq!(
            service.set_port(second).await.err().as_deref(),
            Some("mcp_port_unavailable")
        );
        assert_eq!(
            service.status().await.url.as_deref(),
            Some(address.as_str())
        );
        assert_eq!(
            mcp_settings::load(&path).map_err(anyhow::Error::msg)?.port,
            first
        );
        drop(occupied);
        let updated = service.set_port(second).await.map_err(anyhow::Error::msg)?;
        assert!(updated.enabled);
        assert_eq!(updated.port, second);
        assert_eq!(updated.url, Some(format!("http://127.0.0.1:{second}/mcp")));
        let config: serde_json::Value = serde_json::from_str(
            updated
                .client_config
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing config"))?,
        )?;
        assert_eq!(
            config["mcpServers"]["coterm"]["url"],
            serde_json::json!(format!("http://127.0.0.1:{second}/mcp"))
        );
        let closed = timeout(Duration::from_secs(2), client.read(&mut reply)).await?;
        assert!(
            closed.is_err() || matches!(closed, Ok(0)),
            "old MCP clients are disconnected"
        );
        let again = service.set_port(second).await.map_err(anyhow::Error::msg)?;
        assert_eq!(again.url, updated.url, "same-port save is idempotent");
        let restarted = McpService::new(runtime(), path.clone());
        assert_eq!(restarted.status().await.port, second);
        assert!(!restarted.status().await.enabled);
        assert!(service.set_port(0).await.is_err());
        service
            .set_enabled(false)
            .await
            .map_err(anyhow::Error::msg)?;
        std::fs::remove_file(&path)?;
        Ok(())
    }

    #[tokio::test]
    async fn save_failure_preserves_the_old_listener_and_releases_the_new_one() -> anyhow::Result<()>
    {
        let mut service = McpService::with_address(runtime(), "127.0.0.1:0".parse()?);
        let enabled = service
            .set_enabled(true)
            .await
            .map_err(anyhow::Error::msg)?;
        service.settings_path = Some(
            std::env::temp_dir()
                .join(format!("coterm-mcp-missing-{}", uuid::Uuid::new_v4()))
                .join("settings.json"),
        );
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = reservation.local_addr()?;
        drop(reservation);
        assert_eq!(
            service.set_port(address.port()).await.err().as_deref(),
            Some("mcp_settings_write_failed")
        );
        assert_eq!(service.status().await.url, enabled.url);
        let old = enabled
            .url
            .ok_or_else(|| anyhow::anyhow!("missing address"))?;
        let _client = tokio::net::TcpStream::connect(authority(&old)).await?;
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
        let service = McpService::new(runtime(), path.clone());
        assert_eq!(
            service.status().await.configuration_error.as_deref(),
            Some("mcp_settings_invalid")
        );
        assert!(service.set_enabled(true).await.is_err());
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = reservation.local_addr()?.port();
        drop(reservation);
        let fixed = service.set_port(port).await.map_err(anyhow::Error::msg)?;
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
        let service = McpService::new(runtime(), path.clone());
        service
            .start_on_launch()
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(!service.status().await.enabled);
        assert_eq!(service.status().await.startup_mode, StartupMode::Manual);
        let running = service
            .set_enabled(true)
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(running.startup_mode, StartupMode::Manual);

        let automatic = service
            .set_startup_mode(StartupMode::Auto)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(automatic.enabled);
        assert_eq!(automatic.port, port);
        let saved = mcp_settings::load(&path).map_err(anyhow::Error::msg)?;
        assert_eq!(saved.port, port);
        assert_eq!(saved.startup_mode, StartupMode::Auto);
        let reopened = McpService::new(runtime(), path.clone());
        assert_eq!(reopened.status().await.startup_mode, StartupMode::Auto);
        // A second process cannot claim success while the first owns the port.
        assert!(reopened.start_on_launch().await.is_err());
        assert!(!reopened.status().await.enabled);

        let manual = service
            .set_startup_mode(StartupMode::Manual)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(
            manual.enabled,
            "changing launch policy must not interrupt clients"
        );
        assert_eq!(manual.url, automatic.url);
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let next_port = reservation.local_addr()?.port();
        drop(reservation);
        let moved = service
            .set_port(next_port)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(moved.enabled);
        assert_eq!(moved.startup_mode, StartupMode::Manual);
        let saved = mcp_settings::load(&path).map_err(anyhow::Error::msg)?;
        assert_eq!(saved.port, next_port);
        assert_eq!(saved.startup_mode, StartupMode::Manual);
        // Manual mode stays off on a new launch, even if the last session was running.
        let reopened = McpService::new(runtime(), path.clone());
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
        let service = McpService::new(runtime(), path.clone());
        service
            .start_on_launch()
            .await
            .map_err(anyhow::Error::msg)?;
        let status = service.status().await;
        assert!(status.enabled);
        assert_eq!(
            status.url.as_deref(),
            Some(format!("http://127.0.0.1:{port}/mcp").as_str())
        );
        let _client = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await?;
        service
            .start_on_launch()
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(
            service.status().await.url,
            status.url,
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
        let service = McpService::new(runtime(), path.clone());
        assert!(
            matches!(service.set_startup_mode(StartupMode::Auto).await, Err(error) if error == "mcp_port_unavailable")
        );
        assert_eq!(service.status().await.startup_mode, StartupMode::Manual);
        assert!(!service.status().await.enabled);
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
        let mut service = McpService::with_address(runtime(), address);
        service.state.get_mut().startup_mode = StartupMode::Manual;
        service.settings_path = Some(
            std::env::temp_dir()
                .join(uuid::Uuid::new_v4().to_string())
                .join("missing.json"),
        );
        assert!(
            matches!(service.set_startup_mode(StartupMode::Auto).await, Err(error) if error == "mcp_settings_write_failed")
        );
        assert_eq!(service.status().await.startup_mode, StartupMode::Manual);
        assert!(!service.status().await.enabled);
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
        let running = service
            .set_enabled(true)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(service.set_startup_mode(StartupMode::Auto).await.is_err());
        assert!(service.status().await.enabled);
        assert_eq!(service.status().await.url, running.url);
        assert_eq!(service.status().await.startup_mode, StartupMode::Manual);
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
    async fn a_disabled_service_checks_occupied_ports_without_saving_or_enabling(
    ) -> anyhow::Result<()> {
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-disabled-{}.json", uuid::Uuid::new_v4()));
        let service = McpService::new(runtime(), path.clone());
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        assert!(service
            .set_port(reservation.local_addr()?.port())
            .await
            .is_err());
        assert!(!service.status().await.enabled);
        assert_eq!(service.status().await.port, mcp_settings::DEFAULT_PORT);
        assert!(!path.exists());
        Ok(())
    }
}
