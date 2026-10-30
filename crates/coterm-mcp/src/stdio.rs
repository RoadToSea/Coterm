//! 标准 MCP Stdio 适配器：stdin/stdout 只承载 JSON-RPC，通过私有回环桥转发给桌面。
//!
//! 接收与发送分离到两个任务：任何一侧关闭或出错都不能丢帧，
//! 且 stdout 始终只包含完整帧。插件由桌面运行时启动，本模块不接触插件进程。

use anyhow::{Context, Result};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::{Duration, timeout},
};

use crate::{
    server::{DEFAULT_BRIDGE_ADDRESS, loopback_address},
    transport::read_frame,
};

pub const ADDRESS_ENV: &str = "COTERM_MCP_ADDR";

/// Connect to the fixed loopback bridge and forward standard input/output.
/// `COTERM_MCP_ADDR` remains an optional test/development override; normal
/// client configurations use the fixed production endpoint.
pub async fn run_from_env() -> Result<()> {
    let address = loopback_address(
        &std::env::var(ADDRESS_ENV).unwrap_or_else(|_| DEFAULT_BRIDGE_ADDRESS.to_owned()),
    )?;
    relay(address, tokio::io::stdin(), tokio::io::stdout()).await
}

/// Bidirectionally copy MCP frames. `input` EOF ends gracefully; a broken
/// bridge connection returns immediately.
pub async fn relay<R, W>(address: std::net::SocketAddr, input: R, output: W) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let socket = timeout(Duration::from_secs(5), TcpStream::connect(address))
        .await
        .context("Coterm did not accept the connection in time")??;
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    let mut input = BufReader::new(input);
    let mut output = output;
    let forward = async {
        while let Some(frame) = read_frame(&mut input).await? {
            writer.write_all(&frame).await?;
            writer.flush().await?;
        }
        writer.shutdown().await?;
        Ok::<(), anyhow::Error>(())
    };
    let back = async {
        while let Some(frame) = read_frame(&mut reader).await? {
            output.write_all(&frame).await?;
            output.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::pin!(forward);
    tokio::pin!(back);
    tokio::select! {
        result = &mut back => result,
        result = &mut forward => { result?; back.await }
    }
}
