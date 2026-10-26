//! Crosses the same codec and transport seam as a plugin, with literal wire fixtures.
use std::time::Duration;

use coterm_proto::runtime::{
    backend::PluginBackend,
    lifecycle::{HelloResult, RuntimeHello, OPTIONAL_FEATURES, REQUIRED_FEATURES},
    message::{HostCommand, PluginEvent, PluginNotification},
    transport::serve_with_io,
    wire::{
        decode_request, decode_response, encode_line, encode_request, reply, MAX_CONTROL_BYTES,
    },
    ErrorCategory, RuntimeError,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};

fn hello(limit: u32) -> RuntimeHello {
    serde_json::from_value(json!({
        "interfaceMajor":1,"interfaceMinor":0,
        "requiredFeatures":REQUIRED_FEATURES,"optionalFeatures":OPTIONAL_FEATURES,
        "limits":{"maxControlBytes":limit,"maxDataChunkBytes":65536,
                  "initialWindow":{"chunks":16,"bytes":1048576}}
    }))
    .unwrap()
}

fn accepted(hello: &RuntimeHello) -> HelloResult {
    let supported: Vec<_> = REQUIRED_FEATURES
        .iter()
        .chain(OPTIONAL_FEATURES)
        .copied()
        .collect();
    HelloResult {
        plugin_id: "fixture".to_owned(),
        plugin_version: "0.1.0".to_owned(),
        interface_minor: 0,
        enabled_features: hello.negotiate_features(&supported).unwrap(),
        limits: hello.control_limits(),
        operations: vec![],
        mcp_tools: vec![],
    }
}

fn request(id: &str, method: &str, params: Value) -> Vec<u8> {
    encode_line(
        &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        MAX_CONTROL_BYTES,
    )
    .unwrap()
}

#[test]
fn golden_methods_round_trip_with_camel_case_fields() {
    let handle = json!({"id":"secret","pluginInstanceId":"i","purpose":"password","expiresAt":"2026-10-26T00:00:00Z"});
    let fixtures = vec![
        ("initialize", serde_json::to_value(hello(4096)).unwrap()),
        (
            "create_instance",
            json!({"instance":{"pluginId":"p","pluginInstanceId":"i","sessionId":"s","label":"x","status":"starting"},"config":{}}),
        ),
        ("destroy_instance", json!({"pluginInstanceId":"i"})),
        (
            "invoke",
            json!({"operationId":"o","pluginInstanceId":"i","source":"agent","effect":"commit","operation":"example.write","args":{"bytes":"AA=="},"dispatch":{"mode":"prepare_proposal","proposalId":"p"}}),
        ),
        (
            "approve",
            json!({"operationId":"o","proposalId":"p","pluginInstanceId":"i","args":{}}),
        ),
        (
            "reject",
            json!({"operationId":"o","proposalId":"p","reason":null}),
        ),
        ("cancel", json!({"type":"operation","id":"o"})),
        ("cancel", json!({"type":"interaction","id":"ix"})),
        ("provide_secret", json!({"handle":handle,"value":"private"})),
        (
            "interaction_result",
            json!({"interactionId":"ix","pluginInstanceId":"i","outcome":{"outcome":"submitted","value":{"kind":"secret","value":handle}}}),
        ),
        ("shutdown", Value::Null),
    ];
    for (method, params) in fixtures {
        let expected = request("r", method, params);
        let decoded = decode_request(&expected).unwrap();
        assert_eq!(decoded.request_id, "r");
        let actual = encode_request(decoded, MAX_CONTROL_BYTES).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&actual).unwrap(),
            serde_json::from_slice::<Value>(&expected).unwrap(),
            "{method}"
        );
    }
}

#[test]
fn standard_errors_do_not_echo_invalid_or_secret_input() {
    let cases = [
        (b"not-json private".to_vec(), -32700, Value::Null),
        (request("r", "missing", Value::Null), -32601, json!("r")),
        (
            request("r", "invoke", json!({"secret":"private"})),
            -32602,
            json!("r"),
        ),
        (
            request(
                "r",
                "reject",
                json!({"operationId":"o","proposalId":"p","reason":42}),
            ),
            -32602,
            json!("r"),
        ),
        (
            request("r", "reject", json!({"operation_id":"o","proposal_id":"p"})),
            -32602,
            json!("r"),
        ),
        (
            br#"{"jsonrpc":"2.0","id":4,"method":"shutdown"}"#.to_vec(),
            -32600,
            Value::Null,
        ),
        (
            br#"{"jsonrpc":"1.0","id":"r","method":"shutdown"}"#.to_vec(),
            -32600,
            json!("r"),
        ),
        (
            br#"[{"jsonrpc":"2.0","id":"r","method":"shutdown"}]"#.to_vec(),
            -32600,
            Value::Null,
        ),
        (
            request("r", "stream_subscribe", json!({})),
            -32601,
            json!("r"),
        ),
    ];
    for (line, code, id) in cases {
        let response = decode_request(&line).unwrap_err().into_response();
        assert_eq!(response.id, id);
        assert_eq!(response.error.as_ref().unwrap().code, code);
        assert!(!serde_json::to_string(&response)
            .unwrap()
            .contains("private"));
    }
}

#[test]
fn structured_error_preserves_category_code_and_safe_details() {
    let original = RuntimeError {
        category: ErrorCategory::Conflict,
        code: "instance_busy".to_owned(),
        message: "Busy".to_owned(),
        details: Some(json!({"retryable":false})),
    };
    let response = serde_json::to_string(&reply("r".to_owned(), Err(original.clone()))).unwrap();
    let PluginEvent::Reply {
        result: Err(error), ..
    } = decode_response(&response).unwrap()
    else {
        panic!("expected error");
    };
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    let success = serde_json::to_string(&reply("r".to_owned(), Ok(Value::Null))).unwrap();
    assert!(matches!(
        decode_response(&success).unwrap(),
        PluginEvent::Reply {
            result: Ok(Value::Null),
            ..
        }
    ));
}

#[test]
fn byte_limit_includes_utf8_and_line_terminator() {
    let value = json!({"text":"中文"});
    let length = serde_json::to_vec(&value).unwrap().len() + 1;
    assert_eq!(encode_line(&value, length).unwrap().len(), length);
    assert_eq!(
        encode_line(&value, length - 1).unwrap_err().code,
        "plugin_control_too_large"
    );
    assert!(encode_line(&json!("x".repeat(MAX_CONTROL_BYTES)), usize::MAX).is_err());
}

#[test]
fn negotiation_uses_intersection_and_rejects_unoffered_or_missing_features() {
    let mut host = hello(4096);
    host.optional_features = vec!["future-optional".to_owned()];
    assert_eq!(
        host.negotiate_features(REQUIRED_FEATURES).unwrap(),
        host.required_features
    );
    let result = accepted(&host);
    host.validate_result(&result).unwrap();
    let mut invalid = result.clone();
    invalid.enabled_features.push("plugin-events".to_owned());
    assert!(host.validate_result(&invalid).is_err());
    invalid = result.clone();
    invalid.enabled_features.pop();
    assert!(host.validate_result(&invalid).is_err());
    invalid = result.clone();
    invalid
        .enabled_features
        .push(invalid.enabled_features[0].clone());
    assert!(host.validate_result(&invalid).is_err());
    host.required_features.push("future-required".to_owned());
    assert!(host.negotiate_features(REQUIRED_FEATURES).is_err());
}

#[test]
fn negotiation_checks_version_and_limits_and_reserves_stream_fields() {
    let host = hello(4096);
    for limit in [0, 4097, u32::MAX] {
        let mut result = accepted(&host);
        result.limits.max_control_bytes = limit;
        assert!(host.validate_result(&result).is_err());
    }
    let mut result = accepted(&host);
    result.interface_minor = 1;
    assert!(host.validate_result(&result).is_err());
    result = accepted(&host);
    result.limits.initial_window.bytes = 42;
    assert!(host.validate_result(&result).is_err());
    let mut future = host.clone();
    future.interface_minor = 7;
    future.validate_result(&accepted(&future)).unwrap();
    future.interface_major = 2;
    assert!(future.negotiate_features(REQUIRED_FEATURES).is_err());
    let oversized = hello(u32::MAX);
    assert_eq!(
        oversized.control_limits().max_control_bytes as usize,
        MAX_CONTROL_BYTES
    );
}

#[test]
fn secret_interaction_dependency_is_required_and_optional_fields_can_grow() {
    let mut host = hello(4096);
    host.optional_features = vec!["deferred-interactions".to_owned()];
    assert!(!host
        .negotiate_features(&[
            "instances",
            "backend-calls",
            "operation-catalog",
            "deferred-interactions"
        ])
        .unwrap()
        .contains(&"deferred-interactions".to_owned()));
    host.required_features
        .push("deferred-interactions".to_owned());
    assert!(host
        .negotiate_features(&[
            "instances",
            "backend-calls",
            "operation-catalog",
            "deferred-interactions"
        ])
        .is_err());
    let future = request(
        "r",
        "destroy_instance",
        json!({"pluginInstanceId":"i","futureOptional":true}),
    );
    assert!(decode_request(&future).is_ok());
}

struct Backend {
    sender: mpsc::Sender<PluginNotification>,
    receiver: Option<mpsc::Receiver<PluginNotification>>,
}
impl Backend {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel(4);
        Self {
            sender,
            receiver: Some(receiver),
        }
    }
}
impl PluginBackend for Backend {
    async fn request(&mut self, command: HostCommand) -> Result<Value, RuntimeError> {
        match command {
            HostCommand::Hello(h) => Ok(serde_json::to_value(accepted(&h)).unwrap()),
            HostCommand::Invoke(request) if request.operation == "defer" => {
                self.sender
                    .send(PluginNotification::OperationCompleted {
                        operation_id: request.operation_id.clone(),
                        result: json!({"done":true}),
                    })
                    .await
                    .unwrap();
                Ok(json!({"_cotermDeferred":request.operation_id}))
            }
            HostCommand::Invoke(_) => Err(RuntimeError {
                category: ErrorCategory::Transport,
                code: "device_disconnected".to_owned(),
                message: "Device disconnected".to_owned(),
                details: None,
            }),
            HostCommand::DestroyInstance { plugin_instance_id }
                if plugin_instance_id == "large" =>
            {
                Ok(json!("x".repeat(5000)))
            }
            _ => Ok(json!({"ok":true})),
        }
    }
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<PluginNotification>> {
        self.receiver.take()
    }
}

#[tokio::test]
async fn transport_requires_handshake_and_recovers_after_request_errors() {
    let mut minimal = hello(4096);
    minimal.optional_features.clear();
    let input = [
        request("before","destroy_instance",json!({"pluginInstanceId":"i"})),
        request("hello","initialize",serde_json::to_value(&minimal).unwrap()),
        request("twice","initialize",serde_json::to_value(&minimal).unwrap()),
        request("unknown","missing",Value::Null),
        request("private","provide_secret",json!({"handle":{"id":"s","pluginInstanceId":"i","purpose":"password","expiresAt":"2026-10-26T00:00:00Z"},"value":"private"})),
        request("bye","shutdown",Value::Null),
    ].concat();
    let mut output = Vec::new();
    serve_with_io(
        Backend::new(),
        input.as_slice(),
        &mut output,
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let lines: Vec<Value> = output
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
        .map(|b| serde_json::from_slice(b).unwrap())
        .collect();
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[0]["error"]["data"]["code"], "runtime_not_initialized");
    assert!(lines[1].get("result").is_some());
    assert_eq!(
        lines[2]["error"]["data"]["code"],
        "runtime_already_initialized"
    );
    assert_eq!(lines[3]["error"]["code"], -32601);
    assert_eq!(
        lines[4]["error"]["data"]["code"],
        "runtime_feature_not_negotiated"
    );
    assert!(lines[5].get("result").is_some());
    assert!(!String::from_utf8(output)
        .unwrap()
        .contains("\"value\":\"private\""));
}

#[tokio::test]
async fn output_failure_and_shutdown_do_not_wait_for_notification_sender() {
    let backend = Backend::new();
    let _held_sender = backend.sender.clone();
    let input = request("bye", "shutdown", Value::Null);
    let mut output = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(1),
        serve_with_io(
            backend,
            input.as_slice(),
            &mut output,
            Duration::from_secs(1),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let (closed, peer) = tokio::io::duplex(64);
    drop(peer);
    tokio::time::timeout(
        Duration::from_secs(1),
        serve_with_io(
            Backend::new(),
            input.as_slice(),
            closed,
            Duration::from_secs(1),
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
}

#[tokio::test]
async fn transport_enforces_negotiated_limits_in_both_directions() {
    for second in [
        request(
            "large",
            "destroy_instance",
            json!({"pluginInstanceId":"large"}),
        ),
        request(
            "large",
            "destroy_instance",
            json!({"pluginInstanceId":"x".repeat(5000)}),
        ),
    ] {
        let input = [
            request(
                "hello",
                "initialize",
                serde_json::to_value(hello(1024)).unwrap(),
            ),
            second,
        ]
        .concat();
        let mut output = Vec::new();
        assert!(serve_with_io(
            Backend::new(),
            input.as_slice(),
            &mut output,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        assert!(!output
            .windows(1100)
            .any(|bytes| bytes.iter().all(|b| *b == b'x')));
    }
    let mut output = Vec::new();
    assert!(serve_with_io(
        Backend::new(),
        &b"{\"jsonrpc\":\"2.0\"}"[..],
        &mut output,
        Duration::from_secs(1)
    )
    .await
    .is_err());
}

#[tokio::test]
async fn deferred_completion_uses_notification_without_id() {
    let (client, server) = tokio::io::duplex(16384);
    let (server_read, server_write) = tokio::io::split(server);
    let (client_read, mut client_write) = tokio::io::split(client);
    let exchange = async move {
        let mut reader = BufReader::new(client_read);
        client_write
            .write_all(&request(
                "hello",
                "initialize",
                serde_json::to_value(hello(4096)).unwrap(),
            ))
            .await
            .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(serde_json::from_str::<Value>(&line)
            .unwrap()
            .get("result")
            .is_some());
        client_write.write_all(&request("invoke","invoke",json!({"operationId":"op","pluginInstanceId":"i","source":"human","effect":"read","operation":"defer","args":{},"dispatch":{"mode":"execute"}}))).await.unwrap();
        let mut responses = Vec::new();
        for _ in 0..2 {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            responses.push(serde_json::from_str::<Value>(&line).unwrap());
        }
        assert!(responses
            .iter()
            .any(|v| v["result"]["_cotermDeferred"] == "op"));
        let notification = responses
            .iter()
            .find(|v| v["method"] == "coterm.notification")
            .unwrap();
        assert!(notification.get("id").is_none());
        assert_eq!(notification["params"]["operationId"], "op");
        client_write
            .write_all(&request("bye", "shutdown", Value::Null))
            .await
            .unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        let (result, ()) = tokio::join!(
            serve_with_io(
                Backend::new(),
                server_read,
                server_write,
                Duration::from_millis(10)
            ),
            exchange
        );
        result.unwrap();
    })
    .await
    .unwrap();
}
