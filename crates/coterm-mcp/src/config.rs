//! Client configuration for Coterm's in-process Streamable HTTP endpoint.

use serde_json::json;

/// Render a generic MCP client configuration for a validated loopback HTTP `url`.
/// Returns JSON containing no executable, credentials, or environment variables.
pub fn client_config(url: &str) -> String {
    json!({"mcpServers":{"coterm":{"type":"http","url":url}}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_connects_directly_to_the_desktop_without_a_process() -> anyhow::Result<()> {
        for url in ["http://127.0.0.1:51234/mcp", "http://[::1]:12345/mcp"] {
            let parsed: serde_json::Value = serde_json::from_str(&client_config(url))?;
            assert_eq!(
                parsed["mcpServers"]["coterm"],
                json!({"type":"http","url":url})
            );
        }
        Ok(())
    }
}
