use serde_json::{json, Value};

/// A bounded readable preview accompanies the authoritative structured snapshot.
pub(crate) fn tool_result(value: Value) -> Value {
    let encoded = if let Some(lines) = value["lines"].as_array() {
        lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    } else if let Some(text) = value["text"].as_str().or_else(|| value["stdout"].as_str()) {
        text.to_owned()
    } else {
        value.to_string()
    };
    let (text, _) = super::payload::bounded_output_text(&encoded, 1024, false);
    let failed = value["timedOut"] == true
        || value["cancelled"] == true
        || value.get("error").is_some_and(|e| !e.is_null());
    json!({"content":[{"type":"text","text":text}],"structuredContent":value,"isError":failed})
}
