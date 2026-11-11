//! MCP 工具结果封装：结构化内容只放一份，文本块只做摘要。
use serde_json::{json, Value};

const TEXT_PREVIEW: usize = 2048;

pub fn tool_result(domain: &Value) -> Value {
    let is_error = matches!(
        domain["status"].as_str(),
        Some("unknown" | "cancelled" | "timed_out")
    ) || domain
        .get("exitCode")
        .and_then(Value::as_i64)
        .is_some_and(|code| code != 0)
        || domain.get("timedOut").and_then(Value::as_bool) == Some(true)
        || domain.get("cancelled").and_then(Value::as_bool) == Some(true)
        || domain.get("error").is_some_and(|error| !error.is_null());
    json!({
        "content": [{"type": "text", "text": digest(domain)}],
        "structuredContent": domain,
        "isError": is_error,
    })
}

pub fn digest(domain: &Value) -> String {
    if domain.get("exitCode").is_none() {
        return preview(&domain.to_string());
    }
    let mut summary = match domain.get("exitCode").and_then(Value::as_i64) {
        Some(code) => format!("exitCode={code}"),
        None => "exitCode=unknown".to_owned(),
    };
    for field in ["timedOut", "cancelled", "durationMs"] {
        if let Some(value) = domain.get(field) {
            if value.as_bool() == Some(false) {
                continue;
            }
            summary.push(' ');
            summary.push_str(field);
            summary.push('=');
            summary.push_str(&value.to_string());
        }
    }
    for field in ["stdout", "stderr"] {
        let Some(stream) = domain.get(field) else {
            continue;
        };
        if let Some(text) = stream
            .as_str()
            .or_else(|| stream.get("text").and_then(Value::as_str))
        {
            if text.is_empty() {
                continue;
            }
            summary.push('\n');
            summary.push_str(field);
            summary.push_str(":\n");
            summary.push_str(&preview(text));
        } else if stream.get("truncated").and_then(Value::as_bool) == Some(true) {
            summary.push('\n');
            summary.push_str(field);
            summary.push_str(": <omitted; see structuredContent>");
        }
    }
    summary
}

fn preview(text: &str) -> String {
    if text.len() <= TEXT_PREVIEW {
        return text.to_owned();
    }
    let mut boundary = TEXT_PREVIEW;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}…", &text[..boundary])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn non_zero_exit_marks_the_tool_result_as_error() {
        let ok = tool_result(&json!({"exitCode": 0, "stdout": {"text": "done"}}));
        assert_eq!(ok["isError"], false);
        assert!(ok["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("exitCode=0"));

        let failed = tool_result(&json!({"exitCode": 2, "stderr": {"text": "boom"}}));
        assert_eq!(failed["isError"], true);
        assert_eq!(failed["structuredContent"]["exitCode"], 2);
        for domain in [
            json!({"exitCode":null,"cancelled":true}),
            json!({"exitCode":0,"timedOut":true}),
            json!({"exitCode":0,"error":"incomplete output"}),
        ] {
            assert_eq!(tool_result(&domain)["isError"], true);
        }
    }

    #[test]
    fn digest_reports_both_streams_and_truncation() {
        let digest = digest(&json!({
            "exitCode": 0,
            "timedOut": true,
            "stdout": {"text": "out"},
            "stderr": {"data": "AA==", "truncated": true},
        }));
        assert!(digest.contains("timedOut=true"), "{digest}");
        assert!(digest.contains("stdout:\nout"), "{digest}");
        assert!(digest.contains("stderr: <omitted"), "{digest}");
    }
}
