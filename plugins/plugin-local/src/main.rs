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
                    let hello: coterm_proto::runtime::lifecycle::RuntimeHello =
                        serde_json::from_value(req.params)?;
                    let resp = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id.unwrap_or(serde_json::Value::Null),
                        result: Some(serde_json::json!({
                            "pluginId": "coterm-plugin-local",
                            "pluginVersion": env!("CARGO_PKG_VERSION"),
                            "interfaceMinor": hello.interface_minor,
                            "enabledFeatures": ["instances", "backend-calls", "operation-catalog"],
                            "limits": hello.limits,
                            "operations": []
                        })),
                        error: None,
                    };
                    writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                    stdout.flush()?;
                }
                "create_instance" | "destroy_instance" | "shutdown" => {
                    if let Some(id) = req.id {
                        let resp = JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id,
                            result: Some(serde_json::json!({ "ok": true })),
                            error: None,
                        };
                        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                        stdout.flush()?;
                    }
                    if req.method == "shutdown" {
                        break;
                    }
                }
                "invoke" | "approve" | "reject" => {
                    let response = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id.unwrap_or(serde_json::Value::Null),
                        result: None,
                        error: Some(coterm_proto::JsonRpcError {
                            code: -32601,
                            message: "local plugin exposes no operations".to_owned(),
                            data: None,
                        }),
                    };
                    writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
                    stdout.flush()?;
                }
                _ => {
                    if let Some(id) = req.id {
                        let response = JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id,
                            result: None,
                            error: Some(coterm_proto::JsonRpcError {
                                code: -32601,
                                message: "method not found".to_owned(),
                                data: None,
                            }),
                        };
                        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
                        stdout.flush()?;
                    }
                }
            }
        }
    }

    Ok(())
}
