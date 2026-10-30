//! MCP CallToolResult construction shared by deferred and immediate plugin results.
//!
//! Structured content is the single machine-readable copy. The text block stays a
//! short digest (plus a bounded readable preview) so clients do not parse a nested
//! JSON string that repeats the whole payload.
use serde_json::{json, Value};

const TEXT_PREVIEW: usize = 2048;

pub fn tool_result(domain: &Value) -> Value {
    let is_error = domain
        .get("exitCode")
        .and_then(Value::as_i64)
        .is_some_and(|code| code != 0);
    json!({
        "content": [{"type": "text", "text": digest(domain)}],
        "structuredContent": domain,
        "isError": is_error,
    })
}

pub fn digest(domain: &Value) -> String {
    preview(&digest_inner(domain))
}

fn digest_inner(domain: &Value) -> String {
    if let Some(code) = domain.get("exitCode").and_then(Value::as_i64) {
        let mut summary = format!("exitCode={code}");
        for field in [
            "encoding",
            "bytes",
            "truncated",
            "syntaxError",
            "timedOut",
            "cancelled",
        ] {
            if let Some(value) = domain.get(field) {
                summary.push(' ');
                summary.push_str(field);
                summary.push('=');
                summary.push_str(&value.to_string());
            }
        }
        if let Some(text) = domain.get("text").and_then(Value::as_str) {
            summary.push('\n');
            summary.push_str(&preview(text));
        }
        for field in ["stdout", "stderr"] {
            if let Some(text) = domain
                .get(field)
                .and_then(|stream| stream.get("text"))
                .and_then(Value::as_str)
            {
                summary.push('\n');
                summary.push_str(field);
                summary.push_str(":\n");
                summary.push_str(&preview_limit(text, TEXT_PREVIEW / 3));
            }
        }
        return summary;
    }
    if let Some(stdout) = domain
        .get("stdout")
        .and_then(|value| value.get("text"))
        .and_then(Value::as_str)
    {
        let mut summary = format!(
            "exitCode={} stdoutBytes={}",
            domain
                .get("exitCode")
                .map_or_else(|| "null".into(), Value::to_string),
            domain
                .pointer("/stdout/bytes")
                .map_or_else(|| "0".into(), Value::to_string)
        );
        summary.push('\n');
        summary.push_str(&preview(stdout));
        return summary;
    }
    let encoded = domain.to_string();
    if encoded.len() <= TEXT_PREVIEW {
        return encoded;
    }
    format!("{} bytes of structured content", encoded.len())
}

fn preview(text: &str) -> String {
    preview_limit(text, TEXT_PREVIEW)
}

fn preview_limit(text: &str, limit: usize) -> String {
    if serde_json::to_string(text).is_ok_and(|json| json.len() <= limit) {
        return text.to_owned();
    }
    let mut end = 0;
    let mut cost = 0;
    for (index, c) in text.char_indices() {
        let size = if c.is_ascii_control() {
            6
        } else if matches!(c, '"' | '\\') {
            2
        } else {
            c.len_utf8()
        };
        if cost + size > limit.saturating_sub(80) {
            break;
        }
        cost += size;
        end = index + c.len_utf8();
    }
    format!(
        "{}\n… (preview truncated; read structuredContent)",
        &text[..end]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_summary_includes_stdout_and_stderr_even_with_an_exit_code() {
        let result = tool_result(&json!({
            "exitCode": 0,
            "stdout": {"text": "config.yaml\n"},
            "stderr": {"text": "diagnostic\n"}
        }));
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        assert!(text.contains("config.yaml"));
        assert!(text.contains("diagnostic"));
        assert_eq!(result["isError"], false);
        let summary = digest(
            &json!({"exitCode":1,"stdout":{"text":"out".repeat(2000)},"stderr":{"text":"important-error".repeat(500)}}),
        );
        assert!(summary.contains("out"));
        assert!(summary.contains("important-error"));
        assert!(serde_json::to_string(&summary).is_ok_and(|text| text.len() <= TEXT_PREVIEW));
    }
}
