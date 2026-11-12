use coterm_core::model::lifecycle::RuntimeLimits;
use coterm_core::runtime::Runtime;
use coterm_mcp::{
    server::BridgeServer,
    transport::{read_frame, write_frame},
};
use serde_json::{json, Value};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    process::Command,
    time::timeout,
};

#[tokio::test]
async fn real_stdio_adapter_routes_to_desktop_runtime() -> anyhow::Result<()> {
    let runtime = Arc::new(Runtime::new(RuntimeLimits {
        max_control_bytes: 1024 * 1024,
    }));
    let server = BridgeServer::bind_at(runtime, "127.0.0.1:0".parse()?).await?;
    let info = server.connection_info();

    let mut child = Command::new(env!("CARGO_BIN_EXE_coterm-mcp-stdio"))
        .args(["--addr", &info.address])
        .env("COTERM_MCP_ADDR", "invalid-address-overridden-by-cli")
        .env_remove("COTERM_MCP_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("stdin missing"))?;
    let mut output = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("stdout missing"))?,
    );
    write_frame(&mut input, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"test","version":"1"},"capabilities":{}}})).await?;
    let initialized: Value = serde_json::from_slice(
        &timeout(Duration::from_secs(5), read_frame(&mut output))
            .await??
            .ok_or_else(|| anyhow::anyhow!("No initialize response"))?,
    )?;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        initialized["result"]["capabilities"],
        json!({"tools":{"listChanged":false}})
    );
    write_frame(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await?;
    write_frame(
        &mut input,
        &json!({"jsonrpc":"2.0","id":"list","method":"tools/list"}),
    )
    .await?;
    let tools: Value = serde_json::from_slice(
        &timeout(Duration::from_secs(5), read_frame(&mut output))
            .await??
            .ok_or_else(|| anyhow::anyhow!("No tools response"))?,
    )?;
    assert_eq!(tools["id"], "list");
    assert_eq!(tools["result"]["tools"][0]["name"], "coterm_instances");
    assert_eq!(tools["result"]["tools"].as_array().map(Vec::len), Some(8));
    write_frame(&mut input, &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"coterm_instances","arguments":{}}})).await?;
    let instances: Value = serde_json::from_slice(
        &timeout(Duration::from_secs(5), read_frame(&mut output))
            .await??
            .ok_or_else(|| anyhow::anyhow!("No instances response"))?,
    )?;
    assert_eq!(
        json!({"instances":instances["result"]["structuredContent"]["instances"]}),
        json!({"instances":[]})
    );
    write_frame(&mut input, &json!({"jsonrpc":"2.0","id":"health","method":"tools/call","params":{"name":"coterm.status","arguments":{}}})).await?;
    let health: Value = serde_json::from_slice(
        &timeout(Duration::from_secs(2), read_frame(&mut output))
            .await??
            .ok_or_else(|| anyhow::anyhow!("No health response"))?,
    )?;
    assert_eq!(health["id"], "health");
    assert_eq!(health["result"]["structuredContent"]["desktopAlive"], true);
    assert_eq!(
        health["result"]["structuredContent"]["connectedInstances"],
        0
    );
    assert_eq!(health["result"]["structuredContent"]["activeInstances"], 0);
    input.shutdown().await?;
    drop(input);
    assert!(timeout(Duration::from_secs(5), child.wait())
        .await??
        .success());
    Ok(())
}

#[tokio::test]
async fn bounds_frames_even_without_a_newline() {
    let oversized = vec![b'x'; coterm_mcp::transport::MAX_FRAME_BYTES + 1];
    let mut reader = BufReader::new(oversized.as_slice());
    assert!(read_frame(&mut reader).await.is_err());
    assert!(coterm_mcp::server::loopback_address("192.0.2.1:1234").is_err());
}

#[tokio::test]
async fn invalid_adapter_options_fail_without_stdout_protocol_noise() -> anyhow::Result<()> {
    for args in [
        vec!["--addr"],
        vec!["--addr", "192.0.2.1:1234"],
        vec!["--token", "not-supported"],
    ] {
        let output = timeout(
            Duration::from_secs(5),
            Command::new(env!("CARGO_BIN_EXE_coterm-mcp-stdio"))
                .args(&args)
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
    Ok(())
}
