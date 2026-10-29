//! 标准 MCP Stdio 适配器：stdin/stdout 只承载 JSON-RPC，通过私有回环桥转发给桌面。
//!
//! 接收与发送分离到两个任务：任何一侧关闭或出错都不能丢帧，
//! 且 stdout 始终只包含完整帧。插件由桌面运行时启动，本模块不接触插件进程。

use anyhow::{Context, Result};
use serde_json::json;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::{timeout, Duration},
};

use crate::{
    server::loopback_address,
    transport::{read_frame, write_frame},
};

pub const ADDRESS_ENV: &str = "COTERM_MCP_ADDR";
pub const TOKEN_ENV: &str = "COTERM_MCP_TOKEN";

/// 读取 `COTERM_MCP_ADDR` / `COTERM_MCP_TOKEN` 并转发标准输入输出。
/// 未配置时返回可直接打印到 stderr 的错误，stdout 保持纯净。
pub async fn run_from_env() -> Result<()> {
    let address = loopback_address(
        &std::env::var(ADDRESS_ENV).with_context(|| format!("{ADDRESS_ENV} required"))?,
    )?;
    let token = std::env::var(TOKEN_ENV).with_context(|| format!("{TOKEN_ENV} required"))?;
    relay(address, &token, tokio::io::stdin(), tokio::io::stdout()).await
}

/// 认证后双向复制帧。`input` 读到 EOF 时优雅结束；`output` 断开立即返回。
pub async fn relay<R, W>(
    address: std::net::SocketAddr,
    token: &str,
    input: R,
    output: W,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let socket = timeout(Duration::from_secs(5), TcpStream::connect(address))
        .await
        .context("Coterm did not accept the connection in time")??;
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    write_frame(&mut writer, &json!({ "token": token })).await?;
    let auth = timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .context("Coterm did not answer the authentication frame")??
        .context("Coterm closed the connection during authentication")?;
    if serde_json::from_slice::<serde_json::Value>(&auth)?["authenticated"] != true {
        anyhow::bail!("Coterm rejected the bridge token");
    }
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
