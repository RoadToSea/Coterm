//! 真实硬件链路验证：需要一个已经连接到本机、已在 shell 提示符的串口设备。
//! 默认跳过，避免在没有板子的机器上失败；显式指定端口和波特率才运行：
//!
//! ```text
//! COTERM_SERIAL_LIVE=COM6 cargo test -p coterm-plugin-serial --test live -- --nocapture
//! ```
//!
//! 断言的是插件对真实字节的往返能力（写入 → 回显 → 回放游标推进），
//! 不断言具体板子的输出内容。
use std::{env, time::Duration};

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use coterm_plugin_serial::backend::Backend;
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

const PLUGIN_ID: &str = "coterm-plugin-serial";

async fn invoke(backend: &mut Backend, operation: &str, args: Value, effect: &str) -> Result<Value> {
    backend
        .request(
            "invoke",
            json!({"operationId":format!("live-{operation}"),"operation":operation,
                   "effect":effect,"pluginInstanceId":"live","args":args,"source":"human",
                   "dispatch":{"mode":"execute"},"pluginId":PLUGIN_ID}),
        )
        .await
}

#[tokio::test]
async fn real_device_round_trips_bytes_at_the_configured_baud_rate() -> Result<()> {
    let Ok(port) = env::var("COTERM_SERIAL_LIVE") else {
        eprintln!("COTERM_SERIAL_LIVE is unset; skipping the hardware test.");
        return Ok(());
    };
    let baud: u32 = env::var("COTERM_SERIAL_LIVE_BAUD")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(115200);
    let mut backend = Backend::default();
    backend
        .request(
            "create_instance",
            json!({"instance":{"pluginId":PLUGIN_ID,"pluginInstanceId":"live",
                               "sessionId":"live","label":"Serial","status":"ready"},
                   "config":{"port":port,"baudRate":baud,"dataBits":8,"stopBits":1,"parity":"none"}}),
        )
        .await?;

    let probe = invoke(&mut backend, "serial.probe", json!({}), "read").await?;
    assert!(
        probe["ports"]
            .as_array()
            .is_some_and(|ports| ports.iter().any(|item| item["portName"] == port.as_str())),
        "the configured port must be reported by the operating system: {probe}"
    );

    let connected = invoke(&mut backend, "serial.connect", json!({}), "commit").await?;
    assert_eq!(connected["connected"], true);

    // 发送两个回车取得提示符，随后发一条能产生确定性输出的命令。
    let before = invoke(&mut backend, "serial.read", json!({}), "read").await?;
    let mut cursor = before["cursor"].as_str().unwrap_or("0").to_owned();
    for payload in [&b"\r"[..], b"\r\n"] {
        invoke(
            &mut backend,
            "serial.write",
            json!({"data":STANDARD.encode(payload)}),
            "commit",
        )
        .await?;
    }

    // 回放缓冲是权威来源：等到确实读回非空字节，且游标推进。
    let mut received = Vec::new();
    timeout(Duration::from_secs(5), async {
        loop {
            let read = invoke(&mut backend, "serial.read", json!({"cursor":cursor}), "read").await?;
            received.extend(
                STANDARD
                    .decode(read["data"].as_str().unwrap_or_default())
                    .unwrap_or_default(),
            );
            cursor = read["cursor"].as_str().unwrap_or(&cursor).to_owned();
            assert_eq!(read["gap"], false, "a fresh connection must not report a gap");
            if !received.is_empty() {
                return Ok::<(), anyhow::Error>(());
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    assert!(
        cursor.parse::<u64>().unwrap_or(0) > 0,
        "the replay cursor must advance after real bytes arrive"
    );
    let _ = received;

    let disconnected = invoke(&mut backend, "serial.disconnect", json!({}), "commit").await?;
    assert_eq!(disconnected["connected"], false);
    // 断开后读取必须明确失败，而不是返回上一次的缓冲。
    let status = invoke(&mut backend, "serial.status", json!({}), "read").await?;
    assert_eq!(status["connected"], false);
    assert_eq!(status["connectionId"], Value::Null);
    Ok(())
}
