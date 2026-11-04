//! Public, device-local MCP preferences. No credentials or plugin data are stored here.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 51234;
const MAX_BYTES: u64 = 4096;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    version: u8,
    port: u16,
}

pub fn load_port(path: &Path) -> Result<u16, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(DEFAULT_PORT),
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
    Ok(settings.port)
}

pub fn save_port(path: &Path, port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("mcp_port_invalid".into());
    }
    let bytes = serde_json::to_vec_pretty(&Settings { version: 1, port })
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
        assert_eq!(load_port(&path).map_err(anyhow::Error::msg)?, DEFAULT_PORT);
        save_port(&path, 12345).map_err(anyhow::Error::msg)?;
        assert_eq!(load_port(&path).map_err(anyhow::Error::msg)?, 12345);
        save_port(&path, 65535).map_err(anyhow::Error::msg)?;
        assert_eq!(load_port(&path).map_err(anyhow::Error::msg)?, 65535);
        assert!(save_port(&path, 0).is_err());
        assert_eq!(load_port(&path).map_err(anyhow::Error::msg)?, 65535);
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
        ] {
            fs::write(&path, bytes)?;
            assert!(load_port(&path).is_err());
        }
        fs::write(&path, vec![b' '; MAX_BYTES as usize + 1])?;
        assert!(load_port(&path).is_err());
        fs::remove_file(path)?;
        Ok(())
    }
}
