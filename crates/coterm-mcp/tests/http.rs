use coterm_core::plugin_manager::ProcessPluginRuntime;
use coterm_mcp::http::HttpBridgeServer;
use coterm_proto::runtime::{lifecycle::RuntimeLimits, stream::FlowWindow};
use futures::StreamExt;
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::{json, Value};
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
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

fn post(client: &Client, url: &str, session: Option<&str>, value: Value) -> RequestBuilder {
    let mut request = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .json(&value);
    if let Some(session) = session {
        request = request
            .header("Mcp-Session-Id", session)
            .header("MCP-Protocol-Version", "2025-06-18");
    }
    request
}

async fn initialize(client: &Client, url: &str) -> anyhow::Result<String> {
    let response = post(client, url, None, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18", "clientInfo":{"name":"http-test","version":"1"},"capabilities":{}
    }})).send().await?;
    assert_eq!(response.status(), StatusCode::OK);
    let session = response
        .headers()
        .get("Mcp-Session-Id")
        .ok_or_else(|| anyhow::anyhow!("No session"))?
        .to_str()?
        .to_owned();
    let value: Value = response.json().await?;
    assert_eq!(value["result"]["protocolVersion"], "2025-06-18");
    assert!(value["result"].get("token").is_none());
    Ok(session)
}

async fn ready(client: &Client, url: &str, session: &str) -> anyhow::Result<()> {
    let response = post(
        client,
        url,
        Some(session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .send()
    .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(response.bytes().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn url_only_handshake_tools_sessions_and_restart() -> anyhow::Result<()> {
    let runtime = runtime();
    let server =
        HttpBridgeServer::bind_at(runtime.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await?;
    let url = server.connection_info().url;
    let address: SocketAddr = url
        .trim_start_matches("http://")
        .trim_end_matches("/mcp")
        .parse()?;
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?;
    let session = initialize(&client, &url).await?;
    let list = json!({"jsonrpc":"2.0","id":"list","method":"tools/list"});
    let before: Value = post(&client, &url, Some(&session), list.clone())
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(before["error"]["code"], -32000);
    ready(&client, &url, &session).await?;
    let tools: Value = post(&client, &url, Some(&session), list.clone())
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(tools["id"], "list");
    assert_eq!(tools["result"]["tools"][0]["name"], "coterm.list_instances");
    let instances: Value = post(&client, &url, Some(&session), json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"coterm.list_instances","arguments":{}}})).send().await?.json().await?;
    assert_eq!(
        instances["result"]["structuredContent"],
        json!({"instances":[]})
    );
    let unknown: Value = post(
        &client,
        &url,
        Some(&session),
        json!({"jsonrpc":"2.0","id":4,"method":"no-such-method"}),
    )
    .send()
    .await?
    .json()
    .await?;
    assert_eq!(unknown["error"]["code"], -32601);
    let response = client
        .delete(&url)
        .header("Mcp-Session-Id", &session)
        .header("MCP-Protocol-Version", "2025-06-18")
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        post(&client, &url, Some(&session), list.clone())
            .send()
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    let old = initialize(&client, &url).await?;
    // The same address must be unavailable to a second desktop, never silently changed.
    assert!(HttpBridgeServer::bind_at(runtime.clone(), address)
        .await
        .is_err());
    tokio::time::timeout(Duration::from_secs(3), server.shutdown()).await?;
    let restarted = HttpBridgeServer::bind_at(runtime, address).await?;
    assert_eq!(restarted.connection_info().url, url);
    assert_eq!(
        post(&client, &url, Some(&old), list).send().await?.status(),
        StatusCode::NOT_FOUND
    );
    let new_session = initialize(&client, &url).await?;
    assert_ne!(new_session, old);
    ready(&client, &url, &new_session).await?;
    restarted.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn rejects_browser_origins_rebinding_invalid_versions_and_oversized_messages(
) -> anyhow::Result<()> {
    let server =
        HttpBridgeServer::bind_at(runtime(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
    let url = server.connection_info().url;
    let client = Client::builder().no_proxy().build()?;
    let ping = json!({"jsonrpc":"2.0","id":2,"method":"ping"});
    assert_eq!(
        post(&client, &url, None, ping.clone())
            .header("Origin", "https://evil.example")
            .send()
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&client, &url, None, ping.clone())
            .header("Origin", "null")
            .send()
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&client, &url, None, ping.clone())
            .header("Host", "evil.example")
            .send()
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&client, &url, None, ping.clone())
            .send()
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client.post(&url).json(&ping).send().await?.status(),
        StatusCode::NOT_ACCEPTABLE
    );
    assert_eq!(
        client
            .post(&url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "text/plain")
            .body("{}")
            .send()
            .await?
            .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        client
            .post(&url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body("x".repeat(1024 * 1024 + 1))
            .send()
            .await?
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let session = initialize(&client, &url).await?;
    ready(&client, &url, &session).await?;
    assert_eq!(
        client
            .post(&url)
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Session-Id", &session)
            .header("MCP-Protocol-Version", "invalid")
            .json(&ping)
            .send()
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(&client, &url, Some(&session), json!([ping]))
            .send()
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(HttpBridgeServer::bind_at(runtime(), "0.0.0.0:0".parse()?)
        .await
        .is_err());
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn sessions_and_sse_are_bounded_and_close_on_shutdown() -> anyhow::Result<()> {
    let server =
        HttpBridgeServer::bind_at(runtime(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
    let url = server.connection_info().url;
    let client = Client::builder().no_proxy().build()?;
    let session = initialize(&client, &url).await?;
    ready(&client, &url, &session).await?;
    for _ in 0..7 {
        initialize(&client, &url).await?;
    }
    let excess = post(&client, &url, None, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18", "clientInfo":{"name":"test","version":"1"},"capabilities":{}
    }})).send().await?;
    assert_eq!(excess.status(), StatusCode::TOO_MANY_REQUESTS);
    let get = || {
        client
            .get(&url)
            .header("Accept", "text/event-stream")
            .header("Mcp-Session-Id", &session)
            .header("MCP-Protocol-Version", "2025-06-18")
    };
    let response = get().send().await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut events = response.bytes_stream();
    let first = tokio::time::timeout(Duration::from_secs(3), events.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("No event"))??;
    assert!(String::from_utf8_lossy(&first).contains("notifications/tools/list_changed"));
    assert_eq!(get().send().await?.status(), StatusCode::CONFLICT);
    // An idle stream must not prevent an ordinary request on the same session.
    assert_eq!(
        post(
            &client,
            &url,
            Some(&session),
            json!({"jsonrpc":"2.0","id":2,"method":"ping"})
        )
        .send()
        .await?
        .status(),
        StatusCode::OK
    );
    tokio::time::timeout(Duration::from_secs(3), server.shutdown()).await?;
    assert!(tokio::time::timeout(Duration::from_secs(3), events.next())
        .await?
        .is_none());
    Ok(())
}
