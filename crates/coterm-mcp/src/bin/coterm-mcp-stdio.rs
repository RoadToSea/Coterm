//! MCP stdio client adapter. stdout is exclusively MCP; no plugin is spawned here.
use anyhow::{Context, Result};
use coterm_mcp::{
    server::loopback_address,
    transport::{read_frame, write_frame},
};
use serde_json::json;
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::TcpStream,
    time::{timeout, Duration},
};

#[tokio::main]
async fn main() -> Result<()> {
    let address =
        loopback_address(&std::env::var("COTERM_MCP_ADDR").context("COTERM_MCP_ADDR required")?)?;
    let token = std::env::var("COTERM_MCP_TOKEN").context("COTERM_MCP_TOKEN required")?;
    let socket = timeout(Duration::from_secs(5), TcpStream::connect(address)).await??;
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    write_frame(&mut writer, &json!({"token":token})).await?;
    let auth = timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await??
        .context("Coterm closed the connection")?;
    if serde_json::from_slice::<serde_json::Value>(&auth)?["authenticated"] != true {
        anyhow::bail!("Coterm authentication failed");
    }
    let input = async {
        let mut stdin = BufReader::new(tokio::io::stdin());
        while let Some(frame) = read_frame(&mut stdin).await? {
            writer.write_all(&frame).await?;
            writer.flush().await?;
        }
        writer.shutdown().await?;
        Ok::<(), anyhow::Error>(())
    };
    let output = async {
        let mut stdout = tokio::io::stdout();
        while let Some(frame) = read_frame(&mut reader).await? {
            stdout.write_all(&frame).await?;
            stdout.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::pin!(input);
    tokio::pin!(output);
    tokio::select! {
        result = &mut output => result,
        result = &mut input => { result?; output.await }
    }
}
