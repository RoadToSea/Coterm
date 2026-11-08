//! Explicit hardware acceptance. Requires an already logged-in Shell and a released port.
use std::{env, time::Duration};

use anyhow::{anyhow, ensure, Result};
use coterm_driver_serial::backend::Backend;
use coterm_proto::runtime::{backend::DriverBackend, message::DriverNotification};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

async fn invoke(
    backend: &mut Backend,
    id: &str,
    operation: &str,
    args: Value,
    effect: &str,
    operation_id: &str,
) -> Result<Value> {
    backend.request("invoke", json!({"operationId":operation_id,"operation":operation,"effect":effect,
        "driverInstanceId":id,"driverId":"coterm-driver-serial","args":args,"source":"human","dispatch":{"mode":"execute"}})).await
}

async fn create(
    backend: &mut Backend,
    port: &str,
    baud: u32,
    prompt: &str,
    mode: &str,
) -> Result<()> {
    backend.request("create_instance", json!({
        "instance":{"driverId":"coterm-driver-serial","driverInstanceId":"hardware","sessionId":"hardware","label":"Hardware acceptance","status":"ready"},
        "config":{"port":port,"baudRate":baud,"mode":mode,"prompt":prompt,"dtr":false,"rts":false}
    })).await?;
    Ok(())
}

async fn exercise(backend: &mut Backend, port: &str, baud: u32, prompt: &str) -> Result<()> {
    create(backend, port, baud, prompt, "shell").await?;
    let mut events = backend
        .take_notifications()
        .ok_or_else(|| anyhow!("Missing notifications"))?;
    invoke(
        backend,
        "hardware",
        "serial.connect",
        json!({}),
        "commit",
        "connect",
    )
    .await?;
    invoke(
        backend,
        "hardware",
        "serial.write",
        json!({"text":"\r"}),
        "commit",
        "prompt",
    )
    .await?;
    timeout(Duration::from_secs(5), async {
        loop {
            let status = invoke(
                backend,
                "hardware",
                "serial.status",
                json!({}),
                "read",
                "status",
            )
            .await?;
            if status["shellReady"] == true {
                return Ok::<(), anyhow::Error>(());
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| {
        anyhow!("Configured prompt not observed; log in manually and verify the literal prompt")
    })??;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let commands = [
        format!("printf '%s%s\\n' 'COTERM_SERIAL_ONE_' '{token}'"),
        format!("printf '%s%s\\n' 'COTERM_SERIAL_TWO_' '{token}' >&2; uname -s"),
    ];
    invoke(
        backend,
        "hardware",
        "serial.command",
        json!({"commands":commands,"waitTimeoutMs":5000}),
        "commit",
        "two-dialogues",
    )
    .await?;
    let result = timeout(Duration::from_secs(12), async {
        while let Some(event) = events.recv().await {
            match event {
                DriverNotification::OperationCompleted {
                    operation_id,
                    result,
                } if operation_id == "two-dialogues" => return Ok(result),
                DriverNotification::OperationFailed {
                    operation_id,
                    error,
                } if operation_id == "two-dialogues" => return Err(anyhow!(error.message)),
                _ => {}
            }
        }
        Err(anyhow!("Notification stream closed"))
    })
    .await??;
    ensure!(
        result["sent"] == 2 && result["timedOut"] == false,
        "Both commands must reach their prompt: {result}"
    );
    ensure!(
        result["stderr"] == "" && result["exitCode"].is_null(),
        "Serial output must remain merged with unknown exit status"
    );
    let pairs = invoke(
        backend,
        "hardware",
        "serial.read_dialogue_pairs",
        json!({"count":2}),
        "read",
        "pairs",
    )
    .await?;
    for (index, marker) in [
        (0, format!("COTERM_SERIAL_TWO_{token}")),
        (1, format!("COTERM_SERIAL_ONE_{token}")),
    ] {
        ensure!(
            pairs["pairs"][index]["status"] == "completed",
            "Prompt must frame each pair"
        );
        let output = pairs["pairs"][index]["stdout"].as_str().unwrap_or_default();
        ensure!(
            output.lines().any(|line| line.trim() == marker),
            "Expected standalone device output, not only the command echo"
        );
        ensure!(
            !output.contains(if index == 0 {
                "COTERM_SERIAL_ONE_"
            } else {
                "COTERM_SERIAL_TWO_"
            }),
            "Command outputs must not overlap"
        );
    }
    let lines = invoke(
        backend,
        "hardware",
        "serial.read_lines",
        json!({"lines":20}),
        "read",
        "lines",
    )
    .await?;
    ensure!(
        lines.get("cursor").is_none(),
        "Agent reads must not expose cursors"
    );
    invoke(
        backend,
        "hardware",
        "serial.disconnect",
        json!({}),
        "commit",
        "disconnect",
    )
    .await?;
    let retained = invoke(
        backend,
        "hardware",
        "serial.read_dialogue_pairs",
        json!({"count":2}),
        "read",
        "retained",
    )
    .await?;
    ensure!(
        retained == pairs,
        "Disconnect must retain the bounded backend dialogue snapshot"
    );
    backend
        .request("destroy_instance", json!({"driverInstanceId":"hardware"}))
        .await?;
    create(backend, port, baud, prompt, "stream").await?;
    ensure!(
        invoke(
            backend,
            "hardware",
            "serial.read_dialogue_pairs",
            json!({}),
            "read",
            "stream-dialogues"
        )
        .await
        .is_err(),
        "Stream instances must reject dialogue reads"
    );
    let status = invoke(
        backend,
        "hardware",
        "serial.status",
        json!({}),
        "read",
        "stream-status",
    )
    .await?;
    ensure!(
        status["capabilities"]["readDialoguePairs"] == false,
        "Stream capability must remain disabled"
    );
    println!("hardware acceptance: two distinct pairs, stderr merged, unknown exit status, cursor-free lines, retained snapshot and stream rejection verified");
    Ok(())
}

#[tokio::test]
#[ignore = "requires an explicitly selected, released port and an already logged-in Shell"]
async fn physical_shell_splits_pairs_and_stream_target_rejects_dialogues() -> Result<()> {
    let port = env::var("COTERM_SERIAL_SHELL_LIVE")?;
    let prompt = env::var("COTERM_SERIAL_SHELL_PROMPT")?;
    let baud = env::var("COTERM_SERIAL_LIVE_BAUD")
        .unwrap_or_else(|_| "115200".into())
        .parse()?;
    let mut backend = Backend::default();
    let result = exercise(&mut backend, &port, baud, &prompt).await;
    let cleanup = backend.request("shutdown", Value::Null).await;
    result?;
    cleanup?;
    Ok(())
}
