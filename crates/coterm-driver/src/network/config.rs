use crate::stream::shell::{validate_prompt, Mode};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Proxy {
    #[default]
    Direct,
    Socks5 {
        host: String,
        port: u16,
    },
    Http {
        host: String,
        port: u16,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LocalEcho {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Config {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: Option<String>,
    pub connect_timeout_ms: u64,
    #[serde(default)]
    pub auto_reconnect: bool,
    pub reconnect_interval_ms: u64,
    #[serde(default)]
    pub proxy: Proxy,
    #[serde(default)]
    pub local_echo: LocalEcho,
    pub receive_newlines: String,
    pub backspace: String,
    pub terminal_type: String,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub prompt: Option<String>,
}

impl Config {
    pub(crate) fn parse(mut value: Value, telnet: bool) -> Result<Self> {
        let Some(object) = value.as_object_mut() else {
            bail!("Connection configuration must be an object")
        };
        for (key, default) in [
            ("port", json!(if telnet { 23 } else { 8080 })),
            ("connectTimeoutMs", json!(5000)),
            ("reconnectIntervalMs", json!(1000)),
            ("localEcho", json!(if telnet { "auto" } else { "on" })),
            ("receiveNewlines", json!(if telnet { "none" } else { "lf" })),
            ("backspace", json!(if telnet { "del" } else { "bs" })),
            ("terminalType", json!("xterm")),
            ("mode", json!(if telnet { "shell" } else { "stream" })),
            ("prompt", json!("# ")),
        ] {
            object.entry(key).or_insert(default);
        }
        let config: Self = serde_json::from_value(value)
            .map_err(|_| anyhow::anyhow!("Invalid connection parameters"))?;
        config.validate(telnet)?;
        Ok(config)
    }

    pub(crate) fn validate(&self, telnet: bool) -> Result<()> {
        validate_host(&self.host)?;
        if self.port == 0
            || !(100..=60_000).contains(&self.connect_timeout_ms)
            || !(250..=60_000).contains(&self.reconnect_interval_ms)
        {
            bail!("Invalid port or connection timing")
        }
        match &self.proxy {
            Proxy::Direct => {}
            Proxy::Socks5 { host, port } | Proxy::Http { host, port } => {
                validate_host(host)?;
                if *port == 0 {
                    bail!("Proxy port must be between 1 and 65535")
                }
            }
        }
        if let Some(username) = &self.username {
            if !telnet
                || username.is_empty()
                || username.len() > 128
                || username.chars().any(char::is_control)
            {
                bail!("Only Telnet supports a visible username of 1–128 bytes")
            }
        }
        if !matches!(
            self.receive_newlines.as_str(),
            "none" | "lf" | "cr" | "both"
        ) || !matches!(self.backspace.as_str(), "bs" | "del")
        {
            bail!("Invalid terminal display settings")
        }
        if !matches!(
            self.terminal_type.as_str(),
            "xterm" | "xterm-256color" | "vt100"
        ) {
            bail!("Unsupported terminal type")
        }
        if let Some(prompt) = &self.prompt {
            validate_prompt(prompt)?;
        }
        if self.mode == Mode::Shell && self.prompt.is_none() {
            bail!("Shell mode requires an explicit prompt")
        }
        Ok(())
    }

    pub(crate) fn patch(&self, patch: &Value, telnet: bool) -> Result<Self> {
        let Some(object) = patch.as_object().filter(|v| !v.is_empty()) else {
            bail!("Configuration patch is empty")
        };
        let mut value = serde_json::to_value(self)?;
        for (name, item) in object {
            value[name] = item.clone();
        }
        Self::parse(value, telnet)
    }
}

fn validate_host(host: &str) -> Result<()> {
    if host.is_empty()
        || host.len() > 253
        || !host.is_ascii()
        || host
            .bytes()
            .any(|b| !(b.is_ascii_alphanumeric() || b".-_:%".contains(&b)))
    {
        bail!("Enter a hostname or IP address without a scheme, path or port")
    }
    Ok(())
}
