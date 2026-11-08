//! MCP CallToolResult construction shared by deferred and immediate plugin results.
//!
//! Structured content is the single machine-readable copy. The text block stays a
//! short digest (plus a bounded readable preview) so clients do not parse a nested
//! JSON string that repeats the whole payload.
use serde_json::{json, Value};

const TEXT_PREVIEW: usize = 2048;
const EXEC_TEXT_PREVIEW: usize = 68 * 1024;

pub fn tool_result(domain: &Value) -> Value {
    let is_error = domain.get("error").is_some_and(|value| !value.is_null())
        || domain.get("cancelled") == Some(&Value::Bool(true))
        || domain.get("timedOut") == Some(&Value::Bool(true))
        || domain
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
    let limit = if domain.get("stdout").is_some() {
        EXEC_TEXT_PREVIEW
    } else {
        TEXT_PREVIEW
    };
    preview_limit(&digest_inner(domain), limit)
}

fn digest_inner(domain: &Value) -> String {
    if domain.get("stdout").is_some_and(Value::is_string) {
        let mut text = format!(
            "mode={} status={} streamMode={} exitCode={} truncated={}\n",
            domain["mode"],
            domain["status"],
            domain["streamMode"],
            domain["exitCode"],
            domain["truncated"]
        );
        for field in ["stdout", "stderr"] {
            if domain[format!("{field}Encoding")] == "base64" {
                text.push_str(&format!("{field}: base64 data in structuredContent\n"));
            } else if let Some(stream) = domain[field].as_str().filter(|text| !text.is_empty()) {
                text.push_str(&format!("{field}:\n{}\n", preview_limit(stream, 34 * 1024)));
            }
        }
        return text;
    }
    if let Some(pairs) = domain.get("pairs").and_then(Value::as_array) {
        let mut text = format!(
            "pairs={} dropped={} truncated={}\n",
            pairs.len(),
            domain["dropped"],
            domain["truncated"]
        );
        for pair in pairs {
            text.push_str(&format!(
                "{}\n{}\n",
                preview(pair["stdin"].as_str().unwrap_or("")),
                preview(&digest_inner(pair))
            ));
            if text.len() > TEXT_PREVIEW {
                break;
            }
        }
        return text;
    }
    if let Some(lines) = domain.get("lines").and_then(Value::as_array) {
        let mut text = lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(stderr) = domain
            .get("stderrLines")
            .and_then(Value::as_array)
            .filter(|lines| !lines.is_empty())
        {
            text.push_str("\nstderr:\n");
            text.push_str(
                &stderr
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        return preview(&text);
    }
    if let Some(text) = domain.get("text").and_then(Value::as_str) {
        return preview(text);
    }
    if let Some(blocks) = domain.get("blocks").and_then(Value::as_array) {
        let mut text = format!(
            "selectedCount={} more={} nextBefore={}\n",
            domain.get("selectedCount").unwrap_or(&Value::Null),
            domain.get("more").unwrap_or(&Value::Null),
            domain.get("nextBefore").unwrap_or(&Value::Null)
        );
        for block in blocks {
            text.push_str(&format!(
                "#{} [{} / {}] {}\n{}\n{}\n",
                block.get("index").unwrap_or(&Value::Null),
                block["actor"].as_str().unwrap_or("unknown"),
                block["channel"].as_str().unwrap_or("unknown"),
                block["command"].as_str().unwrap_or(""),
                block["output"].as_str().unwrap_or(""),
                block["stderr"].as_str().unwrap_or("")
            ));
            if text.len() > TEXT_PREVIEW {
                break;
            }
        }
        return text;
    }
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
                summary.push_str(&preview_limit(text, 34 * 1024));
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
        summary.push_str(&preview_limit(stdout, 34 * 1024));
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
        assert!(serde_json::to_string(&summary).is_ok_and(|text| text.len() <= EXEC_TEXT_PREVIEW));
    }
}
