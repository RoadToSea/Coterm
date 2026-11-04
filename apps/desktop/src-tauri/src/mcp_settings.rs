//! Public, device-local MCP preferences. No credentials or plugin data are stored here.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 51234;
const MAX_BYTES: u64 = 4096;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupMode {
    #[default]
    Auto,
    Manual,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    version: u8,
    pub port: u16,
    #[serde(default)]
    pub startup_mode: StartupMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            port: DEFAULT_PORT,
            startup_mode: StartupMode::Auto,
        }
    }
}

pub fn load(path: &Path) -> Result<Settings, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default())
        }
        Err(_) => return Err("mcp_settings_read_failed".into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "mcp_settings_read_failed")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("mcp_settings_invalid".into());
    }
    let settings: Settings = serde_json::from_slice(&bytes).map_err(|_| "mcp_settings_invalid")?;
    if settings.version != 1 || settings.port == 0 {
        return Err("mcp_settings_invalid".into());
    }
    Ok(settings)
}

pub fn save(path: &Path, port: u16, startup_mode: StartupMode) -> Result<(), String> {
    if port == 0 {
        return Err("mcp_port_invalid".into());
    }
    let bytes = serde_json::to_vec_pretty(&Settings {
        version: 1,
        port,
        startup_mode,
    })
    .map_err(|_| "mcp_settings_write_failed")?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        crate::workspace::replace_file(&temporary, path)?;
        #[cfg(not(windows))]
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|_| "mcp_settings_write_failed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_defaults_and_atomic_updates_survive_reopening() -> anyhow::Result<()> {
        let path = std::env::temp_dir().join(format!("coterm-mcp-{}.json", uuid::Uuid::new_v4()));
        assert_eq!(load(&path).map_err(anyhow::Error::msg)?.port, DEFAULT_PORT);
        save(&path, 12345, StartupMode::Auto).map_err(anyhow::Error::msg)?;
        assert_eq!(load(&path).map_err(anyhow::Error::msg)?.port, 12345);
        save(&path, 65535, StartupMode::Manual).map_err(anyhow::Error::msg)?;
        assert_eq!(load(&path).map_err(anyhow::Error::msg)?.port, 65535);
        assert!(save(&path, 0, StartupMode::Auto).is_err());
        assert_eq!(load(&path).map_err(anyhow::Error::msg)?.port, 65535);
        assert_eq!(
            load(&path).map_err(anyhow::Error::msg)?.startup_mode,
            StartupMode::Manual
        );
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn legacy_port_only_settings_preserve_automatic_startup() -> anyhow::Result<()> {
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-legacy-{}.json", uuid::Uuid::new_v4()));
        fs::write(&path, br#"{"version":1,"port":12345}"#)?;
        let settings = load(&path).map_err(anyhow::Error::msg)?;
        assert_eq!(settings.port, 12345);
        assert_eq!(settings.startup_mode, StartupMode::Auto);
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn malformed_out_of_range_and_oversized_settings_are_rejected() -> anyhow::Result<()> {
        let path =
            std::env::temp_dir().join(format!("coterm-mcp-invalid-{}.json", uuid::Uuid::new_v4()));
        for bytes in [
            b"invalid".as_slice(),
            br#"{"version":1,"port":0}"#,
            br#"{"version":1,"port":65536}"#,
            br#"{"version":2,"port":51234}"#,
            br#"{"version":1,"port":51234,"extra":1}"#,
            br#"{"version":1,"port":51234,"startupMode":"unknown"}"#,
        ] {
            fs::write(&path, bytes)?;
            assert!(load(&path).is_err());
        }
        fs::write(&path, vec![b' '; MAX_BYTES as usize + 1])?;
        assert!(load(&path).is_err());
        fs::remove_file(path)?;
        Ok(())
    }
}
