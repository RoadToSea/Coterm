//! 对真实串口链路的手工验证入口：`cargo run -p coterm-plugin-serial --example live -- COM6 115200`
//! 只做一次「连上、发回车、读回显」的往返，用于 115200 板卡验证。
use std::{env, process::exit, time::Duration};

use coterm_plugin_serial::backend::Backend;
use serde_json::{json, Value};

async fn call(backend: &mut Backend, method: &str, params: Value) -> Value {
    match backend.request(method, params).await {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{method} failed: {error}");
            exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    let port = args.next().unwrap_or_else(|| "COM6".to_owned());
    let baud: u32 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(115200);
    let mut backend = Backend::default();
    call(
        &mut backend,
        "create_instance",
        json!({"instance":{"pluginId":"coterm-plugin-serial","pluginInstanceId":"live",
                           "sessionId":"cli","label":"Serial","status":"ready"},
               "config":{"port":port,"baudRate":baud,"dataBits":8,"stopBits":1,"parity":"none"}}),
    )
    .await;
    let ports = call(
        &mut backend,
        "invoke",
        json!({"operationId":"live-0","operation":"serial.probe","effect":"read",
               "pluginInstanceId":"live","args":{},"source":"human",
               "dispatch":{"mode":"execute"},"pluginId":"coterm-plugin-serial"}),
    )
    .await;
    println!(
        "ports: {}",
        ports["ports"]
            .as_array()
            .map(|list| list
                .iter()
                .map(|item| item["portName"].as_str().unwrap_or("?").to_owned())
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default()
    );
    let connected = call(
        &mut backend,
        "invoke",
        json!({"operationId":"live-1","operation":"serial.connect","effect":"commit",
               "pluginInstanceId":"live","args":{},"source":"human",
               "dispatch":{"mode":"execute"},"pluginId":"coterm-plugin-serial"}),
    )
    .await;
    println!("connect: {connected}");

    // 先在发送前取一次游标，避免把上一次残留的输出算进本轮。
    let before = call(
        &mut backend,
        "invoke",
        json!({"operationId":"live-2","operation":"serial.read","effect":"read",
               "pluginInstanceId":"live","args":{},"source":"human",
               "dispatch":{"mode":"execute"},"pluginId":"coterm-plugin-serial"}),
    )
    .await;
    let mut cursor = before["cursor"].as_u64().unwrap_or(0);
    println!("cursor before: {cursor}");

    let script: [(usize, &[u8]); 5] = [
        (3, b"\r"),
        (4, b"root\r\n"),
        (5, b"root\r\n"),
        (6, b"uname -a\r\n"),
        (7, b"\r\n"),
    ];
    for (index, payload) in script {
        let written = call(
            &mut backend,
            "invoke",
            json!({"operationId":format!("live-{index}"),"operation":"serial.write","effect":"commit",
                   "pluginInstanceId":"live","args":{"text":String::from_utf8_lossy(payload)},"source":"human",
                   "dispatch":{"mode":"execute"},"pluginId":"coterm-plugin-serial"}),
        )
        .await;
        println!(
            "write {}: {}",
            String::from_utf8_lossy(payload).escape_debug(),
            written["bytes"]
        );
        tokio::time::sleep(Duration::from_millis(600)).await;
        for _ in 0..16 {
            let read = call(
                &mut backend,
                "invoke",
                json!({"operationId":format!("live-{index}"),"operation":"serial.read","effect":"read",
                       "pluginInstanceId":"live","args":{"cursor":cursor},"source":"human",
                       "dispatch":{"mode":"execute"},"pluginId":"coterm-plugin-serial"}),
            )
            .await;
            let bytes = read["text"].as_str().unwrap_or_default().as_bytes();
            if !bytes.is_empty() {
                print!("{}", String::from_utf8_lossy(bytes));
            }
            cursor = read["cursor"].as_u64().unwrap_or(cursor);
            if read["gap"].as_bool().unwrap_or(false) {
                eprintln!("[gap: replay buffer wrapped]");
            }
            if !read["more"].as_bool().unwrap_or(false) {
                break;
            }
        }
    }
    let disconnected = call(
        &mut backend,
        "invoke",
        json!({"operationId":"live-9","operation":"serial.disconnect","effect":"commit",
               "pluginInstanceId":"live","args":{},"source":"human",
               "dispatch":{"mode":"execute"},"pluginId":"coterm-plugin-serial"}),
    )
    .await;
    println!("\ndisconnect: {disconnected}");
}
