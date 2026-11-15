//! Old client commands must fail without opening another desktop or an adapter process.
use std::{process::Stdio, time::Duration};

#[tokio::test]
async fn retired_stdio_configuration_exits_with_an_http_migration_message() -> anyhow::Result<()> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_coterm-desktop"))
            .args(["mcp", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Streamable HTTP"));
    Ok(())
}
