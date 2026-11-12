//! Real local Shell execution through the same backend surface used by runtime dispatch.
use std::time::Duration;

use anyhow::{anyhow, Result};
use coterm_core::driver::DriverBackend;
use coterm_core::driver::DriverNotification;
use coterm_driver::local::backend::{Backend, DRIVER_ID};
use serde_json::{json, Value};

async fn request(backend: &mut Backend, method: &str, params: Value) -> Result<Value> {
    let command = serde_json::from_value(json!({"type":method,"params":params}))?;
    Ok(backend.request(command).await?)
}

async fn call(
    backend: &mut Backend,
    id: &str,
    operation: &str,
    effect: &str,
    args: Value,
    source: &str,
    op: &str,
) -> Result<Value> {
    request(backend,"invoke",json!({"driverInstanceId":id,"operation":operation,
        "effect":effect,"args":args,"source":source,"operationId":op,"dispatch":{"mode":"execute"}})).await
}

async fn result(
    backend: &mut Backend,
    events: &mut tokio::sync::mpsc::Receiver<DriverNotification>,
    id: &str,
) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            match events.recv().await.ok_or_else(||anyhow!("Event stream closed"))? {
                DriverNotification::OperationCompleted {operation_id,result} if operation_id == id => return Ok(result),
                DriverNotification::Event {name,payload,..} if name == "local.terminal.output" => {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD.decode(payload["data"].as_str().unwrap_or(""))?;
                    if bytes.windows(4).any(|part|part==b"\x1b[6n") {
                        call(backend,"one","local.input","commit",json!({"data":base64::engine::general_purpose::STANDARD.encode(b"\x1b[1;1R")}),"human","terminal-report").await?;
                    }
                }
                _ => (),
            }
        }
    }).await?
}

#[tokio::test]
async fn foreground_preserves_state_background_is_separate_and_reads_are_isolated() -> Result<()> {
    exercise(false).await
}

#[cfg(windows)]
#[tokio::test]
async fn cmd_foreground_preserves_state_and_backend_records() -> Result<()> {
    exercise(true).await
}

async fn exercise(cmd: bool) -> Result<()> {
    let mut backend = Backend::default();
    let mut events = backend
        .take_notifications()
        .ok_or_else(|| anyhow!("Missing events"))?;
    let shell = if cmd {
        "cmd.exe"
    } else if cfg!(windows) {
        "powershell.exe"
    } else {
        "/bin/sh"
    };
    for id in ["one", "two"] {
        request(
            &mut backend,
            "create_instance",
            json!({"instance":{"driverId":DRIVER_ID,"driverInstanceId":id,
            "sessionId":id,"name":id,"status":"ready"},"config":{"shell":shell}}),
        )
        .await?;
    }
    // Choosing foreground never silently starts a separate process for an idle terminal.
    assert!(call(
        &mut backend,
        "one",
        "local.command",
        "commit",
        json!({"commands":["echo wrong"]}),
        "human",
        "idle"
    )
    .await
    .is_err());
    let status = call(
        &mut backend,
        "one",
        "local.restart",
        "commit",
        json!({}),
        "human",
        "start",
    )
    .await?;
    let shell_id = status["status"]["shellId"].clone();
    let (set, read, error) = if cmd {
        (
            "set COTERM_TEST_VALUE=retained&echo first-output",
            "echo %COTERM_TEST_VALUE%",
            "echo separate-error 1>&2 & echo separate-output",
        )
    } else if cfg!(windows) {
        (
            "$env:COTERM_TEST_VALUE='retained'; Write-Output 'first-output'",
            "Write-Output $env:COTERM_TEST_VALUE",
            "[Console]::Error.WriteLine('separate-error'); Write-Output 'separate-output'",
        )
    } else {
        (
            "export COTERM_TEST_VALUE=retained; printf 'first-output\\n'",
            "printf '%s\\n' \"$COTERM_TEST_VALUE\"",
            "printf 'separate-error\\n' >&2; printf 'separate-output\\n'",
        )
    };
    for (id, command) in [("first", set), ("second", read)] {
        let accepted = call(
            &mut backend,
            "one",
            "local.command",
            "commit",
            json!({"commands":[command]}),
            "human",
            id,
        )
        .await?;
        assert_eq!(accepted["_cotermDeferred"], id);
        let value = result(&mut backend, &mut events, id).await?;
        assert_eq!(value["mode"], "terminal", "{value}");
        assert_eq!(value["exitCode"], 0, "{value}");
        assert_eq!(value["stderr"], "");
        assert!(
            value["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains(if id == "first" {
                    "first-output"
                } else {
                    "retained"
                }),
            "{value}"
        );
    }
    let lines = call(
        &mut backend,
        "one",
        "local.read_lines",
        "read",
        json!({"lines":30}),
        "agent",
        "lines",
    )
    .await?;
    assert!(lines.to_string().contains("retained"), "{lines}");
    assert!(
        !lines.to_string().contains("__coterm_src"),
        "wrapper echo leaked: {lines}"
    );
    let selection = json!({"shellId":shell_id,"text":"human selection"});
    assert!(call(
        &mut backend,
        "one",
        "local.select_lines",
        "draft",
        selection.clone(),
        "agent",
        "forged"
    )
    .await
    .is_err());
    call(
        &mut backend,
        "one",
        "local.select_lines",
        "draft",
        selection,
        "human",
        "select",
    )
    .await?;
    assert_eq!(
        call(
            &mut backend,
            "one",
            "local.read_selected_lines",
            "read",
            json!({}),
            "agent",
            "selected"
        )
        .await?["text"],
        "human selection"
    );
    call(
        &mut backend,
        "one",
        "local.command",
        "commit",
        json!({"commands":[error],"mode":"exec"}),
        "human",
        "background",
    )
    .await?;
    let value = result(&mut backend, &mut events, "background").await?;
    assert_eq!(value["streamMode"], "separated");
    assert!(
        value["stderr"]
            .as_str()
            .unwrap_or_default()
            .contains("separate-error"),
        "{value}"
    );
    let pairs = call(
        &mut backend,
        "one",
        "local.read_dialogue_pairs",
        "read",
        json!({"count":2}),
        "agent",
        "pairs",
    )
    .await?;
    assert_eq!(pairs["pairs"].as_array().map(Vec::len), Some(2));
    assert_eq!(pairs["pairs"][0]["operationId"], "background");
    assert!(call(
        &mut backend,
        "two",
        "local.output",
        "read",
        json!({"operationId":"background"}),
        "human",
        "cross-instance"
    )
    .await
    .is_err());
    assert_eq!(
        call(
            &mut backend,
            "two",
            "local.read_dialogue_pairs",
            "read",
            json!({}),
            "agent",
            "empty"
        )
        .await?["pairs"],
        json!([])
    );
    request(&mut backend, "shutdown", Value::Null).await?;
    Ok(())
}
