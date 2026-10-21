//! Single host-owned stdio connection: bounded JSON-RPC replies and notifications.
use anyhow::Result;
use coterm_plugin_ssh::backend::Backend;
use coterm_proto::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() -> Result<()> {
    let mut backend = Backend::default();
    let mut notifications = backend
        .take_notifications()
        .ok_or_else(|| anyhow::anyhow!("Notification receiver unavailable"))?;
    let (replies, mut responses) = tokio::sync::mpsc::channel::<JsonRpcResponse>(16);
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        let mut responses_open = true;
        let mut notifications_open = true;
        while responses_open || notifications_open {
            let value = tokio::select! {
                biased;
                response = responses.recv(), if responses_open => match response {
                    Some(response) => serde_json::to_value(response)?,
                    None => { responses_open = false; continue; }
                },
                notification = notifications.recv(), if notifications_open => match notification {
                    Some(notification) => serde_json::json!({"jsonrpc":"2.0","method":"coterm.notification","params":notification}),
                    None => { notifications_open = false; continue; }
                }
            };
            stdout.write_all(&serde_json::to_vec(&value)?).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    });
    let mut reader = BufReader::new(tokio::io::stdin());
    let result: Result<()> = async {
        loop {
            let mut line = Vec::new();
            loop {
                let available = tokio::select! {
                    available = reader.fill_buf() => available?,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        backend.expire_secrets();
                        continue;
                    }
                };
                if available.is_empty() {
                    return Ok(());
                }
                let end = available.iter().position(|byte| *byte == b'\n');
                let count = end.map_or(available.len(), |index| index + 1);
                if line.len() + count > 1024 * 1024 {
                    anyhow::bail!("SSH runtime control line exceeds limit");
                }
                line.extend_from_slice(&available[..count]);
                reader.consume(count);
                if end.is_some() {
                    break;
                }
            }
            let request: JsonRpcRequest = match serde_json::from_slice(&line) {
                Ok(request) => request,
                Err(_) => anyhow::bail!("Invalid SSH runtime JSON-RPC"),
            };
            if request.jsonrpc != "2.0" {
                anyhow::bail!("Invalid JSON-RPC version");
            }
            // The raw line may contain a private credential delivery.
            zeroize::Zeroize::zeroize(&mut line);
            let Some(id) = request.id else {
                continue;
            };
            let shutdown = request.method == "shutdown";
            let result = backend.request(&request.method, request.params).await;
            let response = match result {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: "2.0".to_owned(),
                    id,
                    result: Some(result),
                    error: None,
                },
                Err(error) => JsonRpcResponse {
                    jsonrpc: "2.0".to_owned(),
                    id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32000,
                        message: error.to_string(),
                        data: None,
                    }),
                },
            };
            replies
                .send(response)
                .await
                .map_err(|_| anyhow::anyhow!("SSH stdout writer closed"))?;
            if shutdown {
                return Ok(());
            }
        }
    }
    .await;
    drop(backend);
    drop(replies);
    writer.await??;
    result
}
