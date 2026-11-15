#[path = "../../../tests/support/mcp_http.rs"]
mod wire;

use coterm_core::{model::lifecycle::RuntimeLimits, runtime::Runtime};
use coterm_mcp::http::{HttpServer, MAX_CONCURRENT_REQUESTS, MAX_MESSAGE_BYTES};
use serde_json::json;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{timeout, Duration},
};

async fn server() -> anyhow::Result<HttpServer> {
    Ok(HttpServer::bind_at(
        Arc::new(Runtime::new(RuntimeLimits {
            max_control_bytes: MAX_MESSAGE_BYTES as u32,
        })),
        "127.0.0.1:0".parse()?,
    )
    .await?)
}

#[tokio::test]
async fn streamable_http_negotiates_lists_tools_and_accepts_notifications() -> anyhow::Result<()> {
    let server = server().await?;
    assert_eq!(server.url(), format!("http://{}/mcp", server.address()));
    let reply = wire::post(server.address(), &wire::initialize(1)).await?;
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert!(!reply.headers.contains_key("mcp-session-id"));
    assert_eq!(reply.json()?["result"]["protocolVersion"], "2025-06-18");
    let reply = wire::post(
        server.address(),
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await?;
    assert_eq!(reply.status, 202);
    assert!(reply.body.is_empty());
    let reply = wire::post(
        server.address(),
        &json!({"jsonrpc":"2.0","id":"catalog","method":"tools/list"}),
    )
    .await?;
    let body = reply.json()?;
    assert_eq!(body["id"], "catalog");
    assert_eq!(body["result"]["tools"].as_array().map(Vec::len), Some(8));
    let result = wire::post(server.address(), &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"coterm_instances","arguments":{}}})).await?.json()?;
    assert_eq!(
        result["result"]["structuredContent"]["instances"],
        json!([])
    );
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn validates_origin_host_methods_headers_json_and_protocol_versions() -> anyhow::Result<()> {
    let server = server().await?;
    let body = serde_json::to_vec(&wire::initialize(1))?;
    let standard = [
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
    ];
    for (method, path, extra, expected) in [
        ("GET", "/mcp", vec![], 405),
        ("DELETE", "/mcp", vec![], 405),
        ("POST", "/unknown", vec![], 404),
        (
            "POST",
            "/mcp",
            vec![("Origin", "https://attacker.example")],
            403,
        ),
        ("POST", "/mcp", vec![("Origin", "null")], 403),
    ] {
        let headers = standard.into_iter().chain(extra).collect::<Vec<_>>();
        let reply =
            wire::receive(wire::send(server.address(), method, path, &headers, &body).await?)
                .await?;
        assert_eq!(reply.status, expected, "{method} {path}");
    }
    for (headers, expected) in [
        (vec![("Content-Type", "application/json")], 406),
        (
            vec![
                ("Content-Type", "text/plain"),
                ("Accept", "application/json, text/event-stream"),
            ],
            415,
        ),
        (
            vec![
                ("Content-Type", "application/json"),
                ("Accept", "application/json, text/event-stream;q=0"),
            ],
            406,
        ),
    ] {
        assert_eq!(
            wire::receive(wire::send(server.address(), "POST", "/mcp", &headers, &body).await?)
                .await?
                .status,
            expected
        );
    }
    for (body, code) in [
        (b"{".as_slice(), -32700),
        (b"[]".as_slice(), -32600),
        (
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":1}".as_slice(),
            -32600,
        ),
    ] {
        let reply =
            wire::receive(wire::send(server.address(), "POST", "/mcp", &standard, body).await?)
                .await?;
        assert_eq!(reply.status, 400);
        assert_eq!(reply.json()?["error"]["code"], code);
    }
    let headers = standard
        .into_iter()
        .chain([("MCP-Protocol-Version", "unsupported")])
        .collect::<Vec<_>>();
    let reply = wire::receive(
        wire::send(
            server.address(),
            "POST",
            "/mcp",
            &headers,
            br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
        )
        .await?,
    )
    .await?;
    assert_eq!(reply.status, 400);
    let mut socket = TcpStream::connect(server.address()).await?;
    socket
        .write_all(b"GET /mcp HTTP/1.1\r\nHost: attacker.example\r\nConnection: close\r\n\r\n")
        .await?;
    assert_eq!(wire::receive(socket).await?.status, 403);
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn oversized_bodies_are_rejected_and_another_client_remains_usable() -> anyhow::Result<()> {
    let server = server().await?;
    let headers = [
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
    ];
    let mut bytes = serde_json::to_vec(&wire::initialize(1))?;
    bytes.resize(MAX_MESSAGE_BYTES, b' ');
    let reply =
        wire::receive(wire::send(server.address(), "POST", "/mcp", &headers, &bytes).await?)
            .await?;
    assert_eq!(reply.status, 200);
    bytes.push(b' ');
    let reply =
        wire::receive(wire::send(server.address(), "POST", "/mcp", &headers, &bytes).await?)
            .await?;
    assert_eq!(reply.status, 413);
    let mut requests = tokio::task::JoinSet::new();
    for id in 0..16 {
        let address = server.address();
        requests.spawn(async move { wire::post(address, &wire::initialize(id)).await });
    }
    while let Some(reply) = requests.join_next().await {
        assert_eq!(reply??.status, 200);
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn overload_is_rejected_and_capacity_recovers_after_a_request_finishes() -> anyhow::Result<()>
{
    let server = server().await?;
    let mut waiting = Vec::new();
    for _ in 0..MAX_CONCURRENT_REQUESTS {
        let mut socket = TcpStream::connect(server.address()).await?;
        let headers = format!("POST /mcp HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: 1\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n", server.address());
        socket.write_all(headers.as_bytes()).await?;
        let mut continued = [0; 25];
        timeout(Duration::from_secs(2), socket.read_exact(&mut continued)).await??;
        assert_eq!(&continued, b"HTTP/1.1 100 Continue\r\n\r\n");
        waiting.push(socket);
    }
    assert_eq!(
        wire::post(server.address(), &wire::initialize(1))
            .await?
            .status,
        503
    );
    // Finish one body so the handler releases its permit before the next request.
    let mut socket = waiting
        .pop()
        .ok_or_else(|| anyhow::anyhow!("No pending client"))?;
    socket.write_all(b"{").await?;
    assert_eq!(wire::receive(socket).await?.status, 400);
    assert_eq!(
        wire::post(server.address(), &wire::initialize(2))
            .await?
            .status,
        200
    );
    drop(waiting);
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn accepts_same_origin_and_rejects_duplicate_origin_headers() -> anyhow::Result<()> {
    let server = server().await?;
    let origin = format!("http://{}", server.address());
    let mut headers = vec![
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
        ("Origin", origin.as_str()),
    ];
    let body = serde_json::to_vec(&wire::initialize(1))?;
    assert_eq!(
        wire::receive(wire::send(server.address(), "POST", "/mcp", &headers, &body).await?)
            .await?
            .status,
        200
    );
    headers.push(("Origin", "https://attacker.example"));
    assert_eq!(
        wire::receive(wire::send(server.address(), "POST", "/mcp", &headers, &body).await?)
            .await?
            .status,
        403
    );
    let reply = wire::post(
        server.address(),
        &json!({"jsonrpc":"2.0","id":1,"method":"unknown"}),
    )
    .await?;
    assert_eq!(reply.json()?["error"]["code"], -32601);
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn shutdown_closes_idle_connections_and_releases_the_port() -> anyhow::Result<()> {
    let server = server().await?;
    let address = server.address();
    let mut socket = TcpStream::connect(address).await?;
    socket.write_all(b"POST /mcp HTTP/1.1\r\n").await?;
    server.shutdown().await;
    let mut byte = [0];
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.read(&mut byte)).await?,
        Ok(0) | Err(_)
    ));
    let _listener = tokio::net::TcpListener::bind(address).await?;
    Ok(())
}

#[tokio::test]
async fn rejects_non_loopback_listeners() -> anyhow::Result<()> {
    let runtime = Arc::new(Runtime::new(RuntimeLimits {
        max_control_bytes: 1024,
    }));
    let result = HttpServer::bind_at(runtime, "192.0.2.1:51234".parse()?).await;
    assert!(matches!(result, Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied));
    Ok(())
}
