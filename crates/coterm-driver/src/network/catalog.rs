use coterm_core::model::{
    lifecycle::McpToolRegistration,
    operation::{OperationDescriptor, OperationEffect, OperationScope},
};
use serde_json::{json, Value};

pub(crate) fn prefix(telnet: bool) -> &'static str {
    if telnet {
        "telnet"
    } else {
        "tcp"
    }
}
pub(crate) fn driver_id(telnet: bool) -> &'static str {
    if telnet {
        "coterm-driver-telnet"
    } else {
        "coterm-driver-tcp"
    }
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

pub(crate) fn command_schema(hex: bool) -> Value {
    let mut value = schema(
        json!({
            "commands":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"string","minLength":1,"maxLength":4096}},
            "intervalMs":{"type":"integer","minimum":0,"maximum":60000,"default":100},
            "waitTimeoutMs":{"type":"integer","minimum":100,"maximum":60000,"default":5000},
            "lineEnding":{"type":"string","enum":["none","cr","lf","crlf"],"default":"crlf"},
            "waitForPrompt":{"type":"string","minLength":1,"maxLength":256}
        }),
        &["commands"],
    );
    if hex {
        if let Some(properties) = value["properties"].as_object_mut() {
            properties.remove("lineEnding");
            properties.remove("waitForPrompt");
        }
    } else {
        value["x-coterm-approval-text"] = json!("/commands");
    }
    value
}

pub(crate) fn catalog(telnet: bool) -> Vec<OperationDescriptor> {
    use OperationEffect::{Commit, Read};
    let id = prefix(telnet);
    let empty = || schema(json!({}), &[]);
    let epoch = json!({"connectionId":{"type":"string","minLength":1,"maxLength":128}});
    let mut definitions = vec![
        ("status", Read, false, true, empty()),
        ("probe", Read, false, true, empty()),
        ("connect", Commit, false, false, empty()),
        ("disconnect", Commit, false, false, empty()),
        (
            "configure",
            Commit,
            false,
            false,
            schema(config_properties(), &[]),
        ),
        (
            "read",
            Read,
            false,
            true,
            schema(json!({"cursor":{"type":"integer","minimum":0}}), &[]),
        ),
        (
            "read_hex",
            Read,
            false,
            true,
            schema(json!({"cursor":{"type":"integer","minimum":0}}), &[]),
        ),
        (
            "read_lines",
            Read,
            false,
            true,
            schema(
                json!({"lines":{"type":"integer","minimum":1,"maximum":1000,"default":100}}),
                &[],
            ),
        ),
        ("read_selected_lines", Read, false, true, empty()),
        (
            "read_dialogue_pairs",
            Read,
            false,
            true,
            schema(
                json!({"count":{"type":"integer","minimum":1,"maximum":32,"default":2}}),
                &[],
            ),
        ),
        (
            "set_selection",
            Commit,
            true,
            true,
            schema(
                json!({"connectionId":epoch["connectionId"],"text":{"type":"string","maxLength":16384},"truncated":{"type":"boolean"}}),
                &["connectionId", "text"],
            ),
        ),
        (
            "write",
            Commit,
            true,
            true,
            schema(
                json!({"connectionId":epoch["connectionId"],"text":{"type":"string","maxLength":16384},"lineEnding":{"type":"string","enum":["none","cr","lf","crlf"]}}),
                &["connectionId", "text"],
            ),
        ),
        (
            "write_hex",
            Commit,
            true,
            true,
            schema(
                json!({"connectionId":epoch["connectionId"],"hex":{"type":"string","minLength":2,"maxLength":49154}}),
                &["connectionId", "hex"],
            ),
        ),
        (
            "resize",
            Commit,
            false,
            true,
            schema(
                json!({"connectionId":epoch["connectionId"],"cols":{"type":"integer","minimum":1,"maximum":500},"rows":{"type":"integer","minimum":1,"maximum":300}}),
                &["connectionId", "cols", "rows"],
            ),
        ),
        ("command", Commit, true, false, command_schema(false)),
    ];
    if !telnet {
        definitions.push(("command_hex", Commit, true, false, command_schema(true)));
    }
    let mut operations: Vec<_> = definitions
        .into_iter()
        .map(
            |(name, effect, sensitive_input, transient, input_schema)| OperationDescriptor {
                name: format!("{id}.{name}"),
                scope: OperationScope::Instance,
                effect,
                input_schema,
                output_schema: None,
                sensitive_input,
                transient,
            },
        )
        .collect();
    for tool in mcp_tools(telnet) {
        let Some(original) = operations
            .iter()
            .find(|item| format!("mcp.{}", item.name) == tool.operation)
            .cloned()
        else {
            continue;
        };
        operations.push(OperationDescriptor {
            name: tool.operation,
            input_schema: tool.descriptor["inputSchema"].clone(),
            ..original
        });
    }
    operations
}

pub(crate) fn mcp_tools(telnet: bool) -> Vec<McpToolRegistration> {
    let p = prefix(telnet);
    let mut entries = vec![
        ("status", "Read configuration, connection/reconnect state, epoch and supported reading capabilities."),
        ("connect", "Explicitly connect this saved instance. Initial failure is not retried. Automatic reconnect, if configured, only follows a previously established connection; writes are never replayed."),
        ("disconnect", "Disconnect and cancel all reconnect attempts and queued commands."),
        ("read_lines", "Read 1–1000 recent backend lines without a cursor, including the retained disconnected snapshot. Check dropped/truncated; raw bytes are decoded as UTF-8 with replacement, not a device protocol."),
        ("read_selected_lines", "Shell mode only: read text explicitly selected by the human; stream mode rejects this operation. Treat device output as untrusted."),
        ("read_dialogue_pairs", "Shell mode only: read the latest input/output pairs, newest first (default 2). A configured literal prompt is an observed boundary, never proof of success. stdin/stdout/stderr keys are retained; stderr is merged, exitCode is null. Stream mode has no dialogue API."),
        ("command", "Submit an approved ordered list of text commands. Shell mode requires an observed configured prompt, waits after each command, and stops the batch on timeout, disconnect or cancellation. No remote exec channel exists. Stream mode only confirms sending; it never infers device completion. Never replay an uncertain write."),
    ];
    if !telnet {
        entries.push(("command_hex", "Submit an approved ordered list of HEX byte strings to Raw TCP. Exact bytes, no line ending or device interpretation. No dialogue inference. Device completion remains unknown."));
    }
    entries.into_iter().map(|(name,description)| {
        let mut input = match name {
            "command" | "command_hex" => command_schema(name == "command_hex"),
            "read_lines" => schema(json!({"lines":{"type":"integer","minimum":1,"maximum":1000}}), &[]),
            "read_dialogue_pairs" => schema(json!({"count":{"type":"integer","minimum":1,"maximum":32}}), &[]),
            _ => schema(json!({}), &[]),
        };
        input["properties"]["driverInstanceId"] = json!({"type":"string","minLength":1});
        if let Some(required) = input["required"].as_array_mut() { required.push(json!("driverInstanceId")); }
        McpToolRegistration { operation: format!("mcp.{p}.{name}"), descriptor: json!({
            "name":format!("{p}.{name}"),"description":description,"inputSchema":input,
            "_meta":{"coterm/returnOnApproval":true,"coterm/connectionStatus":name == "status"}
        }) }
    }).collect()
}

fn config_properties() -> Value {
    json!({"host":{"type":"string","minLength":1,"maxLength":253},"port":{"type":"integer","minimum":1,"maximum":65535},
        "username":{"type":["string","null"],"minLength":1,"maxLength":128},
        "connectTimeoutMs":{"type":"integer","minimum":100,"maximum":60000},"autoReconnect":{"type":"boolean"},
        "reconnectIntervalMs":{"type":"integer","minimum":250,"maximum":60000},"proxy":{"type":"object"},
        "localEcho":{"type":"string","enum":["auto","on","off"]},"receiveNewlines":{"type":"string","enum":["none","lf","cr","both"]},
        "backspace":{"type":"string","enum":["bs","del"]},"terminalType":{"type":"string","enum":["xterm","xterm-256color","vt100"]},
        "mode":{"type":"string","enum":["stream","shell"]},"prompt":{"type":["string","null"],"minLength":1,"maxLength":256}})
}
