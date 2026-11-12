//! Unified MCP contract tests use local fixtures and never open physical ports.
use anyhow::{anyhow, Result};
use coterm_core::{model::lifecycle::RuntimeLimits, runtime::Runtime};
use coterm_mcp::server::{call_tool, tools_list};
use serde_json::{json, Value};

async fn serial_runtime() -> Result<Runtime> {
    let runtime = Runtime::new(RuntimeLimits {
        max_control_bytes: 1024 * 1024,
    });
    runtime
        .register::<coterm_driver::serial::backend::Backend>(
            serde_json::from_str(include_str!("../manifests/serial.json"))?,
            Some("export function mount() { return { unmount() {} }; }"),
        )
        .await?;
    runtime.start("coterm-driver-serial").await?;
    for (name, port) in [("board", "COM999"), ("other", "COM998")] {
        runtime
            .create_instance_with_name(
                "coterm-driver-serial",
                json!({"port":port,"baudRate":115200,"mode":"stream"}),
                Some(name.into()),
            )
            .await?;
    }
    Ok(runtime)
}

async fn call(runtime: &Runtime, name: &str, arguments: Value) -> Result<Value> {
    call_tool(runtime, &json!({"name":name,"arguments":arguments})).await
}

fn failed(result: Result<Value>) -> bool {
    match result {
        Err(_) => true,
        Ok(value) => {
            value["isError"] == true || value["_meta"]["coterm/operation"]["status"] == "failed"
        }
    }
}

#[tokio::test]
async fn catalog_and_instance_reads_hide_driver_tools_and_follow_current_mode() -> Result<()> {
    let runtime = serial_runtime().await?;
    let tools = tools_list(&runtime).await?;
    let listed = tools["tools"]
        .as_array()
        .ok_or_else(|| anyhow!("Missing catalog"))?;
    assert_eq!(listed.len(), 8);
    assert!(listed.iter().all(|tool| !tool["name"]
        .as_str()
        .unwrap_or_default()
        .starts_with("coterm_serial_")));
    let summary = call(&runtime, "coterm_instances", json!({})).await?;
    assert_eq!(summary["structuredContent"]["totalInstances"], 2);
    assert_eq!(summary["structuredContent"]["connectedInstances"], 0);
    for instance in summary["structuredContent"]["instances"]
        .as_array()
        .ok_or_else(|| anyhow!("Missing instances"))?
    {
        assert_eq!(instance["driver"], "serial");
        assert_eq!(instance["connected"], false);
        assert!(instance["capabilities"]
            .as_array()
            .is_some_and(
                |items| items.contains(&json!("hex")) && !items.contains(&json!("dialogue"))
            ));
    }
    for alias in ["name", "instance", "instanceName", "instance_name"] {
        let detail = call(&runtime, "coterm_instances", json!({alias:"board"})).await?;
        assert_eq!(
            detail["structuredContent"]["selectedInstance"]["config"]["port"],
            "COM999"
        );
    }
    let immediate = call_tool(
        &runtime,
        &json!({"name":"coterm_instances","arguments":{"name":"board"},"_meta":{"coterm/waitTimeoutMs":0}}),
    )
    .await?;
    if immediate["_meta"]["coterm/operation"]["status"] == "completed" {
        assert!(immediate["structuredContent"]["selectedInstance"]["config"].is_object());
    } else {
        assert!(immediate["structuredContent"]
            .get("selectedInstance")
            .is_none());
        assert!(immediate["structuredContent"]["operationId"].is_string());
    }
    let lines = call(&runtime, "coterm_read_output", json!({"name":"board"})).await?;
    assert!(lines["structuredContent"]["lines"].is_array());
    for mode in ["dialogue", "selection"] {
        assert!(failed(
            call(
                &runtime,
                "coterm_read_output",
                json!({"name":"board","mode":mode})
            )
            .await
        ));
    }
    for args in [
        json!({"name":"missing"}),
        json!({"name":null}),
        json!({"name":"board","instance":"other"}),
        json!({"name":"board","password":"never-accepted"}),
        json!({"driverInstanceId":"private"}),
    ] {
        assert!(call(&runtime, "coterm_instances", args).await.is_err());
    }
    assert!(call(&runtime, "coterm_read_output", json!({}))
        .await
        .is_err());
    runtime.stop("coterm-driver-serial").await?;
    Ok(())
}

#[tokio::test]
async fn connection_approval_query_and_cancel_keep_the_original_instance() -> Result<()> {
    let runtime = serial_runtime().await?;
    let proposed = call(
        &runtime,
        "coterm_connection",
        json!({"name":"board","action":"connect"}),
    )
    .await?;
    assert_eq!(proposed["structuredContent"]["status"], "pending_approval");
    assert_eq!(
        proposed["_meta"]["coterm/operation"]["status"],
        "awaiting_approval"
    );
    let id = proposed["structuredContent"]["operationId"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing operation"))?;
    let query = call(
        &runtime,
        "coterm_operation",
        json!({"name":"board","operationId":id}),
    )
    .await?;
    assert_eq!(query["structuredContent"]["status"], "awaiting_approval");
    for args in [
        json!({"name":"other","operationId":id}),
        json!({"operationId":id}),
        json!({"name":"board","operationId":id,"source":"human"}),
    ] {
        assert!(call(&runtime, "coterm_operation", args).await.is_err());
    }
    let cancelled = call(
        &runtime,
        "coterm_operation",
        json!({"name":"board","operationId":id,"action":"cancel"}),
    )
    .await?;
    assert_eq!(cancelled["isError"], false);
    let query = call(
        &runtime,
        "coterm_operation",
        json!({"name":"board","operationId":id}),
    )
    .await?;
    assert_eq!(query["structuredContent"]["status"], "cancelled");
    let probe = call(
        &runtime,
        "coterm_connection",
        json!({"name":"board","action":"probe"}),
    )
    .await?;
    assert!(probe["structuredContent"]["ports"].is_array());
    assert!(
        call(&runtime, "coterm_connection", json!({"action":"connect"}))
            .await
            .is_err()
    );
    assert!(call(
        &runtime,
        "coterm_connection",
        json!({"name":"board","action":"reset"})
    )
    .await
    .is_err());
    runtime.stop("coterm-driver-serial").await?;
    Ok(())
}

#[tokio::test]
async fn stream_encoding_keeps_serial_limits_and_rejects_inapplicable_parameters() -> Result<()> {
    let runtime = serial_runtime().await?;
    for encoding in ["utf-8", "hex"] {
        let result = call(
            &runtime,
            "coterm_stream_command",
            json!({"name":"board","commands":["00 FF"],"encoding":encoding}),
        )
        .await?;
        assert_eq!(result["structuredContent"]["status"], "pending_approval");
    }
    for args in [
        json!({"encoding":"hex","lineEnding":"cr"}),
        json!({"encoding":"hex","waitTimeoutMs":100}),
        json!({"encoding":"hex","waitForPrompt":"#"}),
        json!({"intervalMs":1001}),
        json!({"encoding":"base64"}),
        json!({"commands":["0"],"encoding":"hex"}),
        json!({"encoding":"utf-8","source":"human"}),
    ] {
        let mut arguments = json!({"name":"board","commands":["00 FF"]});
        arguments
            .as_object_mut()
            .ok_or_else(|| anyhow!("Missing object"))?
            .extend(
                args.as_object()
                    .ok_or_else(|| anyhow!("Missing args"))?
                    .clone(),
            );
        assert!(failed(
            call(&runtime, "coterm_stream_command", arguments).await
        ));
    }
    assert!(call(
        &runtime,
        "coterm_terminal_command",
        json!({"name":"board","commands":["pwd"]})
    )
    .await
    .is_err());
    assert!(call(
        &runtime,
        "coterm_file_read",
        json!({"name":"board","path":"/tmp/test"})
    )
    .await
    .is_err());
    assert!(call_tool(
        &runtime,
        &json!({"name":"coterm_instances","_meta":{"coterm/waitTimeoutMs":300001}})
    )
    .await
    .is_err());
    runtime.stop("coterm-driver-serial").await?;
    Ok(())
}

#[tokio::test]
async fn network_routes_keep_native_limits_and_do_not_invent_hex_or_probe_capabilities(
) -> Result<()> {
    let runtime = Runtime::new(RuntimeLimits {
        max_control_bytes: 1024 * 1024,
    });
    runtime
        .register::<coterm_driver::tcp::backend::Backend>(
            serde_json::from_str(include_str!("../manifests/tcp.json"))?,
            Some("export function mount() { return { unmount() {} }; }"),
        )
        .await?;
    runtime
        .register::<coterm_driver::telnet::backend::Backend>(
            serde_json::from_str(include_str!("../manifests/telnet.json"))?,
            Some("export function mount() { return { unmount() {} }; }"),
        )
        .await?;
    let mut peers = Vec::new();
    for protocol in ["tcp", "telnet"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let id = format!("coterm-driver-{protocol}");
        runtime.start(&id).await?;
        let instance = runtime
            .create_instance_with_name(
                &id,
                json!({"host":"127.0.0.1","port":listener.local_addr()?.port()}),
                Some(protocol.into()),
            )
            .await?;
        runtime
            .set_bypass(&instance.driver_instance_id, true)
            .await?;
        let connected = call(
            &runtime,
            "coterm_connection",
            json!({"name":protocol,"action":"connect"}),
        )
        .await?;
        assert_eq!(connected["isError"], false, "{connected}");
        peers.push(listener.accept().await?.0);
        runtime
            .set_bypass(&instance.driver_instance_id, false)
            .await?;
        let result = call(
            &runtime,
            "coterm_stream_command",
            json!({"name":protocol,"commands":["data"],"intervalMs":60000}),
        )
        .await?;
        assert_eq!(
            result["structuredContent"]["status"], "pending_approval",
            "{result}"
        );
        assert!(failed(
            call(
                &runtime,
                "coterm_stream_command",
                json!({"name":protocol,"commands":vec!["data";33]})
            )
            .await
        ));
        assert!(failed(
            call(
                &runtime,
                "coterm_stream_command",
                json!({"name":protocol,"commands":["data"],"waitTimeoutMs":0})
            )
            .await
        ));
        assert!(call(
            &runtime,
            "coterm_connection",
            json!({"name":protocol,"action":"probe"})
        )
        .await
        .is_err());
    }
    assert!(call(
        &runtime,
        "coterm_stream_command",
        json!({"name":"telnet","encoding":"hex","commands":["FF"]})
    )
    .await
    .is_err());
    let hex = call(
        &runtime,
        "coterm_stream_command",
        json!({"name":"tcp","encoding":"hex","commands":["00 FF"]}),
    )
    .await?;
    assert_eq!(hex["structuredContent"]["status"], "pending_approval");
    runtime.stop("coterm-driver-tcp").await?;
    runtime.stop("coterm-driver-telnet").await?;
    Ok(())
}

#[tokio::test]
async fn local_exec_and_global_default_routing_work_across_driver_types() -> Result<()> {
    use coterm_core::{
        model::operation::OperationSource,
        runtime::{CallerContext, SubmitOperation},
    };
    let runtime = serial_runtime().await?;
    let frontend = Some("export function mount() { return { unmount() {} }; }");
    runtime
        .register::<coterm_driver::local::backend::Backend>(
            serde_json::from_str(include_str!("../manifests/local.json"))?,
            frontend,
        )
        .await?;
    runtime
        .register::<coterm_driver::tcp::backend::Backend>(
            serde_json::from_str(include_str!("../manifests/tcp.json"))?,
            frontend,
        )
        .await?;
    runtime.start("coterm-driver-local").await?;
    runtime.start("coterm-driver-tcp").await?;
    let local = runtime
        .create_instance_with_name(
            "coterm-driver-local",
            json!({"shell":if cfg!(windows) {"cmd.exe"} else {"/bin/sh"}}),
            Some("local".into()),
        )
        .await?;
    let restart = runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: local.session_id.clone(),
                driver_id: Some(local.driver_id.clone()),
            },
            SubmitOperation {
                driver_id: local.driver_id.clone(),
                driver_instance_id: Some(local.driver_instance_id.clone()),
                operation: "local.restart".into(),
                args: json!({}),
                wait_timeout_ms: Some(5000),
            },
        )
        .await?;
    assert!(restart.error.is_none(), "{restart:?}");
    runtime.set_bypass(&local.driver_instance_id, true).await?;
    let executed = call_tool(&runtime, &json!({"name":"coterm_terminal_command","arguments":{"commands":["echo unified-local"],"mode":"exec"},"_meta":{"coterm/waitTimeoutMs":5000}})).await?;
    assert_eq!(executed["structuredContent"]["exitCode"], 0, "{executed}");
    assert!(executed["structuredContent"]["stdout"]
        .as_str()
        .is_some_and(|value| value.contains("unified-local")));
    let snapshot = call(&runtime, "coterm_instances", json!({"name":"local"})).await?;
    let detail = &snapshot["structuredContent"]["selectedInstance"];
    assert!(detail.is_object(), "{snapshot}");
    for key in ["driverInstanceId", "instanceId", "sessionId"] {
        assert!(detail.get(key).is_none(), "{snapshot}");
    }
    for args in [
        json!({"name":"local","action":"connect"}),
        json!({"name":"local","action":"reset"}),
    ] {
        assert!(call(&runtime, "coterm_connection", args).await.is_err());
    }
    assert!(call(
        &runtime,
        "coterm_terminal_command",
        json!({"name":"local","commands":["echo nope"],"sudo":true})
    )
    .await
    .is_err());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let tcp = runtime
        .create_instance_with_name(
            "coterm-driver-tcp",
            json!({"host":"127.0.0.1","port":listener.local_addr()?.port()}),
            Some("tcp".into()),
        )
        .await?;
    let connected = runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: tcp.session_id.clone(),
                driver_id: Some(tcp.driver_id.clone()),
            },
            SubmitOperation {
                driver_id: tcp.driver_id.clone(),
                driver_instance_id: Some(tcp.driver_instance_id.clone()),
                operation: "tcp.connect".into(),
                args: json!({}),
                wait_timeout_ms: Some(5000),
            },
        )
        .await?;
    assert!(connected.error.is_none(), "{connected:?}");
    assert!(call(
        &runtime,
        "coterm_terminal_command",
        json!({"commands":["echo ambiguous"],"mode":"exec"})
    )
    .await
    .is_err());
    assert!(call(
        &runtime,
        "coterm_read_output",
        json!({"name":"local","operationId":"unused"})
    )
    .await
    .is_err());
    for driver in [
        "coterm-driver-local",
        "coterm-driver-tcp",
        "coterm-driver-serial",
    ] {
        runtime.stop(driver).await?;
    }
    Ok(())
}
