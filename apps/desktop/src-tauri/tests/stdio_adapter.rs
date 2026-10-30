//! 发布形态的 Stdio 适配器回归：用桌面可执行文件自身的 `mcp stdio` 子命令连接私有桥。
//! 不创建窗口，也不依赖独立分发的 CLI。
use coterm_core::plugin_manager::ProcessPluginRuntime;
use coterm_mcp::{
    server::BridgeServer,
    transport::{read_frame, write_frame},
};
use coterm_proto::runtime::{lifecycle::RuntimeLimits, stream::FlowWindow};
use serde_json::{Value, json};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    time::timeout,
};

fn runtime() -> Arc<ProcessPluginRuntime> {
    Arc::new(ProcessPluginRuntime::new(
        "test",
        RuntimeLimits {
            max_control_bytes: 1024 * 1024,
            max_data_chunk_bytes: 8192,
            initial_window: FlowWindow {
                chunks: 16,
                bytes: 1024 * 1024,
            },
        },
    ))
}

async fn read_message(
    output: &mut BufReader<tokio::process::ChildStdout>,
) -> anyhow::Result<Value> {
    let frame = timeout(Duration::from_secs(10), read_frame(output))
        .await??
        .ok_or_else(|| anyhow::anyhow!("adapter closed stdout"))?;
    Ok(serde_json::from_slice(&frame)?)
}

#[tokio::test]
async fn desktop_subcommand_serves_as_the_stdio_adapter() -> anyhow::Result<()> {
    let server = BridgeServer::bind_at(runtime(), "127.0.0.1:0".parse()?).await?;
    let info = server.connection_info();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_coterm-desktop"))
        .args(["mcp", "stdio"])
        .env("COTERM_MCP_ADDR", &info.address)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut input = child.stdin.take().ok_or_else(|| anyhow::anyhow!("stdin"))?;
    let mut output = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("stdout"))?,
    );

    write_frame(&mut input, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"desktop-subcommand","version":"1"},"capabilities":{}}})).await?;
    let initialized = read_message(&mut output).await?;
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");

    write_frame(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await?;
    write_frame(&mut input, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"coterm.list_instances","arguments":{}}})).await?;
    let instances = read_message(&mut output).await?;
    assert_eq!(
        instances["result"]["structuredContent"],
        json!({"instances":[]})
    );
    input.shutdown().await?;
    drop(input);
    let status = timeout(Duration::from_secs(10), child.wait()).await??;
    assert!(status.success());
    Ok(())
}

#[tokio::test]
async fn adapter_reports_bridge_connection_failure_on_stderr_only() -> anyhow::Result<()> {
    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_coterm-desktop"))
        .args(["mcp", "stdio"])
        .env("COTERM_MCP_ADDR", "127.0.0.1:0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let output = timeout(Duration::from_secs(10), child).await??;
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stdout is reserved for MCP frames"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.trim().is_empty(),
        "stderr should explain the failure"
    );
    Ok(())
}
