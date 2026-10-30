//! 标准 MCP Stdio 适配器：stdin/stdout 只承载 JSON-RPC，通过私有回环桥转发给桌面。
//!
//! 接收与发送分离到两个任务：任何一侧关闭或出错都不能丢帧，
//! 且 stdout 始终只包含完整帧。插件由桌面运行时启动，本模块不接触插件进程。

use anyhow::{bail, Context, Result};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::{timeout, Duration},
};

use crate::{
    server::{loopback_address, DEFAULT_BRIDGE_ADDRESS},
    transport::read_frame,
};

pub const ADDRESS_ENV: &str = "COTERM_MCP_ADDR";

/// Connect to the fixed loopback bridge and forward standard input/output.
/// `COTERM_MCP_ADDR` remains an optional test/development override; normal
/// client configurations use the fixed production endpoint.
pub async fn run_from_env() -> Result<()> {
    run(&[]).await
}

/// Recognize the adapter before starting the desktop, including invalid adapter flags
/// which must fail on stderr instead of accidentally opening a window.
pub fn is_adapter_command(args: &[String]) -> bool {
    args.get(..2)
        .is_some_and(|prefix| prefix == ["mcp", "stdio"])
}

/// CLI address takes precedence over the optional development environment override.
pub fn bridge_address(args: &[String], environment: Option<&str>) -> Result<std::net::SocketAddr> {
    let mut explicit = None;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if flag != "--addr" || explicit.is_some() {
            bail!("Usage: mcp stdio [--addr <127.0.0.1:port>]; unsupported or repeated option");
        }
        explicit = Some(
            args.next()
                .context("--addr requires a loopback address with a port")?
                .as_str(),
        );
    }
    let address = loopback_address(explicit.or(environment).unwrap_or(DEFAULT_BRIDGE_ADDRESS))?;
    if address.port() == 0 {
        bail!("The Coterm bridge address requires a nonzero port");
    }
    Ok(address)
}

pub async fn run(args: &[String]) -> Result<()> {
    let environment = std::env::var(ADDRESS_ENV).ok();
    let address = bridge_address(args, environment.as_deref())?;
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
        .with_context(|| format!("Coterm bridge at {address} did not accept the connection in time; start the desktop and enable MCP"))?
        .with_context(|| format!("Cannot connect to Coterm bridge at {address}; start the desktop and enable MCP, or check --addr / {ADDRESS_ENV}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_address_is_optional_and_overrides_environment() -> Result<()> {
        assert_eq!(
            bridge_address(&[], None)?.to_string(),
            DEFAULT_BRIDGE_ADDRESS
        );
        assert_eq!(bridge_address(&[], Some("127.0.0.1:12345"))?.port(), 12345);
        let args = vec!["--addr".into(), "127.0.0.1:23456".into()];
        assert_eq!(bridge_address(&args, Some("invalid"))?.port(), 23456);
        assert!(is_adapter_command(&[
            "mcp".into(),
            "stdio".into(),
            "--addr".into()
        ]));
        assert!(!is_adapter_command(&["mcp".into()]));
        for args in [
            vec!["--addr"],
            vec!["--addr", "192.0.2.1:1234"],
            vec!["--addr", "127.0.0.1:0"],
            vec!["--token", "unused"],
            vec!["--unknown"],
            vec!["--addr", "127.0.0.1:1", "--addr", "127.0.0.1:2"],
        ] {
            let args: Vec<_> = args.into_iter().map(str::to_owned).collect();
            assert!(bridge_address(&args, None).is_err(), "{args:?}");
        }
        Ok(())
    }
}
