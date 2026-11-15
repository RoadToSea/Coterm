//! Test-only independent HTTP/1.1 client; production HTTP parsing belongs to Hyper.
#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

pub struct HttpReply {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpReply {
    pub fn json(&self) -> Result<Value> {
        Ok(serde_json::from_slice(&self.body)?)
    }
}

pub async fn send(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<TcpStream> {
    let mut socket = TcpStream::connect(address).await?;
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    socket.write_all(request.as_bytes()).await?;
    socket.write_all(body).await?;
    Ok(socket)
}

pub async fn receive(mut socket: TcpStream) -> Result<HttpReply> {
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(35), socket.read_to_end(&mut bytes)).await??;
    let split = bytes
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .context("HTTP headers missing")?;
    let mut lines = std::str::from_utf8(&bytes[..split])?.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .context("HTTP status missing")?
        .parse()?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<HashMap<_, _>>();
    let body = bytes[split + 4..].to_vec();
    if let Some(length) = headers.get("content-length") {
        if body.len() != length.parse::<usize>()? {
            bail!("Incomplete HTTP response")
        }
    }
    Ok(HttpReply {
        status,
        headers,
        body,
    })
}

pub async fn post(address: SocketAddr, value: &Value) -> Result<HttpReply> {
    receive(start(address, value).await?).await
}

pub async fn start(address: SocketAddr, value: &Value) -> Result<TcpStream> {
    send(
        address,
        "POST",
        "/mcp",
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", "2025-06-18"),
        ],
        &serde_json::to_vec(value)?,
    )
    .await
}

pub fn initialize(id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"http-test","version":"1"}
    }})
}
