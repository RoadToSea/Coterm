//! Instance capability and cursor-free snapshot contracts without opening hardware.
use anyhow::Result;
type Backend = coterm_core::diagnostic::Backend<coterm_driver::serial::backend::Backend>;
use serde_json::{json, Value};

async fn create(backend: &mut Backend, id: &str, mode: &str, port: &str) -> Result<()> {
    backend.request("create_instance", json!({
        "instance":{"driverId":"coterm-driver-serial","driverInstanceId":id,"sessionId":"s","label":id,"status":"ready"},
        "config":{"port":port,"baudRate":115200,"mode":mode,"prompt":"# "}
    })).await?;
    Ok(())
}

async fn read(backend: &mut Backend, id: &str, operation: &str, args: Value) -> Result<Value> {
    backend
        .request(
            "invoke",
            json!({"operationId":"read-test","operation":operation,"effect":"read",
        "driverInstanceId":id,"args":args,"source":"human","dispatch":{"mode":"execute"}}),
        )
        .await
}

#[tokio::test]
async fn stream_exposes_only_line_reads_and_shell_capabilities_are_instance_owned() -> Result<()> {
    let mut backend = Backend::default();
    create(&mut backend, "stream", "stream", "COM6").await?;
    create(&mut backend, "shell", "shell", "COM7").await?;
    for id in ["stream", "shell"] {
        let status = read(&mut backend, id, "serial.status", json!({})).await?;
        assert_eq!(status["capabilities"]["readLines"], true);
        assert_eq!(status["capabilities"]["readDialoguePairs"], id == "shell");
        let lines = read(&mut backend, id, "serial.read_lines", json!({"lines":2})).await?;
        assert_eq!(lines["lines"], json!([]));
        assert!(lines.get("cursor").is_none());
    }
    for operation in ["serial.read_selected_lines", "serial.read_dialogue_pairs"] {
        assert!(read(&mut backend, "stream", operation, json!({}))
            .await
            .is_err());
    }
    assert_eq!(
        read(
            &mut backend,
            "shell",
            "serial.read_selected_lines",
            json!({})
        )
        .await?["text"],
        ""
    );
    assert_eq!(
        read(
            &mut backend,
            "shell",
            "serial.read_dialogue_pairs",
            json!({})
        )
        .await?["pairs"],
        json!([])
    );
    for count in [
        json!(0),
        json!(1001),
        json!(-1),
        json!(1.5),
        json!("2"),
        json!(null),
    ] {
        assert!(read(
            &mut backend,
            "shell",
            "serial.read_lines",
            json!({"lines":count})
        )
        .await
        .is_err());
    }
    for count in [json!(0), json!(33), json!(-1), json!(1.5), json!("2")] {
        assert!(read(
            &mut backend,
            "shell",
            "serial.read_dialogue_pairs",
            json!({"count":count})
        )
        .await
        .is_err());
    }
    Ok(())
}

#[tokio::test]
async fn stream_cannot_enable_prompt_interpretation_through_command_arguments() -> Result<()> {
    let mut backend = Backend::default();
    create(&mut backend, "stream", "stream", "COM6").await?;
    let error = backend.request("invoke", json!({"operationId":"write","operation":"serial.command","effect":"commit",
        "driverInstanceId":"stream","args":{"commands":["pwd"],"waitForPrompt":"# "},"source":"agent",
        "dispatch":{"mode":"prepare_proposal","proposalId":"p"}})).await.expect_err("stream cannot opt into Shell");
    assert!(error.to_string().contains("mode=shell"));
    Ok(())
}

#[test]
fn line_snapshots_preserve_utf8_across_packets_and_bound_escaped_output() -> Result<()> {
    let mut output = coterm_driver::serial::session::OutputBuffer::default();
    for byte in "old\r\n中文\r\n# ".as_bytes() {
        output.push(&[*byte]);
    }
    assert_eq!(output.read_lines(2)?["lines"], json!(["中文", "# "]));
    output.push(&vec![0; 20 * 1024]);
    let snapshot = output.read_lines(100)?;
    assert_eq!(snapshot["truncated"], true);
    assert!(serde_json::to_vec(&snapshot)?.len() < 12 * 1024);
    output.close();
    let snapshot = output.read_lines(2)?;
    output.close();
    assert_eq!(snapshot, output.read_lines(2)?);
    Ok(())
}

#[tokio::test]
async fn agent_cannot_forge_human_selection_even_with_direct_dispatch() -> Result<()> {
    let mut backend = Backend::default();
    create(&mut backend, "shell", "shell", "COM6").await?;
    let error = backend.request("invoke", json!({"operationId":"fake-selection","operation":"serial.set_selection","effect":"commit",
        "driverInstanceId":"shell","source":"agent","dispatch":{"mode":"execute"},
        "args":{"text":"fake human selection","connectionId":"old"}})).await.expect_err("Agent cannot forge human selection");
    assert!(error.to_string().contains("Human"));
    assert_eq!(
        read(
            &mut backend,
            "shell",
            "serial.read_selected_lines",
            json!({})
        )
        .await?["text"],
        ""
    );
    Ok(())
}
