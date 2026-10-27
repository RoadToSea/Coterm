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
    if let Some(code) = domain.get("exitCode").and_then(Value::as_i64) {
        let mut summary = format!("exitCode={code}");
        for field in ["encoding", "bytes", "truncated", "syntaxError", "timedOut", "cancelled"] {
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
        return summary;
    }
    if let Some(stdout) = domain.get("stdout").and_then(|value| value.get("text")).and_then(Value::as_str) {
        let mut summary = format!(
            "exitCode={} stdoutBytes={}",
            domain.get("exitCode").map_or_else(|| "null".into(), Value::to_string),
            domain.pointer("/stdout/bytes").map_or_else(|| "0".into(), Value::to_string)
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
    if text.len() <= TEXT_PREVIEW {
        return text.to_owned();
    }
    let mut end = TEXT_PREVIEW;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (preview truncated; read structuredContent)", &text[..end])
}
