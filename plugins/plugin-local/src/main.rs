use coterm_proto::{JsonRpcRequest, JsonRpcResponse};
use std::io::{self, BufRead, Write};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_writer(io::stderr) // 日志只打到 stderr，stdout 纯净留给 JSON-RPC
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Coterm Local PTY Plugin starting up...");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(req) = serde_json::from_str::<JsonRpcRequest>(&line) {
            match req.method.as_str() {
                "initialize" => {
                    let resp = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id.unwrap_or(serde_json::Value::Null),
                        result: Some(serde_json::json!({
                            "status": "ok",
                            "protocol": "local",
                            "capabilities": {
                                "pty": true
                            }
                        })),
                        error: None,
                    };
                    writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                    stdout.flush()?;
                }
                _ => {
                    // 其他方法待后续完善接入
                }
            }
        }
    }

    Ok(())
}
