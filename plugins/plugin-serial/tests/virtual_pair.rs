//! 显式启用的虚拟串口配对验收：启动真实插件进程，测试两端字节，不依赖设备 shell。
//! COTERM_SERIAL_PAIR_A / COTERM_SERIAL_PAIR_B 必须指向用户授权的相连端口。
use std::{env, path::PathBuf, time::Duration};

use anyhow::{anyhow, ensure, Context, Result};
use coterm_core::{
    plugin_manager::ProcessPluginRuntime,
    plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
};
use coterm_plugin_serial::payload::{decode_hex, encode_hex};
use coterm_proto::runtime::{
    lifecycle::{InstanceInfo, RuntimeLimits},
    operation::{ApproveProposal, OperationRecord, OperationSource, OperationStatus},
    stream::FlowWindow,
};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout, Instant};

const PLUGIN: &str = "coterm-plugin-serial";

fn decode_hex_page(hex: &str) -> Result<Vec<u8>> {
    if hex.is_empty() {
        Ok(Vec::new())
    } else {
        decode_hex(hex)
    }
}

struct Endpoint {
    instance: InstanceInfo,
    cursor: u64,
}

impl Endpoint {
    fn caller(&self, source: OperationSource) -> CallerContext {
        CallerContext {
            source,
            session_id: self.instance.session_id.clone(),
            plugin_id: Some(PLUGIN.into()),
        }
    }

    fn request(&self, operation: &str, args: Value, wait_ms: u64) -> SubmitOperation {
        SubmitOperation {
            plugin_id: PLUGIN.into(),
            plugin_instance_id: Some(self.instance.plugin_instance_id.clone()),
            operation: operation.into(),
            args,
            wait_timeout_ms: Some(wait_ms),
        }
    }

    async fn call(&self, runtime: &ProcessPluginRuntime, name: &str, args: Value) -> Result<Value> {
        let receipt = runtime
            .submit(
                self.caller(OperationSource::Human),
                self.request(name, args, 3000),
            )
            .await?;
        ensure!(
            receipt.status == OperationStatus::Completed,
            "{name}: {receipt:?}"
        );
        receipt
            .output
            .ok_or_else(|| anyhow!("{name}: missing output"))
    }

    async fn queue(&self, runtime: &ProcessPluginRuntime, args: Value) -> Result<String> {
        let receipt = runtime
            .submit(
                self.caller(OperationSource::Human),
                self.request(
                    "serial.command",
                    {
                        let mut args = args;
                        if args.get("lineEnding").is_none() {
                            args["lineEnding"] = json!("cr");
                        }
                        args
                    },
                    0,
                ),
            )
            .await?;
        ensure!(receipt.error.is_none(), "queue rejected: {receipt:?}");
        Ok(receipt.operation_id)
    }

    async fn state(
        &self,
        runtime: &ProcessPluginRuntime,
        id: &str,
        wanted: OperationStatus,
    ) -> Result<OperationRecord> {
        timeout(Duration::from_secs(5), async {
            loop {
                let record = runtime.get(self.caller(OperationSource::Human), id).await?;
                if record.status == wanted {
                    return Ok(record);
                }
                ensure!(
                    !matches!(
                        record.status,
                        OperationStatus::Completed
                            | OperationStatus::Failed
                            | OperationStatus::Cancelled
                            | OperationStatus::Unknown
                            | OperationStatus::Rejected
                    ),
                    "expected {wanted:?}: {record:?}"
                );
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("operation status timeout")?
    }

    async fn read(&mut self, runtime: &ProcessPluginRuntime) -> Result<Vec<u8>> {
        let output = self
            .call(runtime, "serial.read_hex", json!({"cursor":self.cursor}))
            .await?;
        ensure!(
            output["gap"] == false && output["closed"] == false,
            "unexpected read gap or closure: {output}"
        );
        self.cursor = output["cursor"]
            .as_u64()
            .ok_or_else(|| anyhow!("missing cursor"))?;
        decode_hex_page(
            output["hex"]
                .as_str()
                .ok_or_else(|| anyhow!("missing bytes"))?,
        )
    }

    async fn expect_bytes(
        &mut self,
        runtime: &ProcessPluginRuntime,
        expected: &[u8],
    ) -> Result<()> {
        timeout(Duration::from_secs(5), async {
            let mut received = Vec::new();
            while received.len() < expected.len() {
                received.extend(self.read(runtime).await?);
                ensure!(
                    expected.starts_with(&received),
                    "unexpected bytes on {}: {received:?}",
                    self.instance.label
                );
                if received.len() < expected.len() {
                    sleep(Duration::from_millis(10)).await;
                }
            }
            ensure!(received == expected, "byte count mismatch");
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("paired port receive timeout")?
    }

    async fn expect_quiet(
        &mut self,
        runtime: &ProcessPluginRuntime,
        duration: Duration,
    ) -> Result<()> {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            ensure!(
                self.read(runtime).await?.is_empty(),
                "unexpected bytes after stop or before approval"
            );
            sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }
}

fn platform() -> Result<String> {
    let os = match env::consts::OS {
        "windows" => "windows",
        "linux" => "linux",
        "macos" => "macos",
        _ => anyhow::bail!("unsupported OS"),
    };
    let arch = match env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => anyhow::bail!("unsupported architecture"),
    };
    Ok(format!("{os}-{arch}"))
}

fn package() -> Result<PathBuf> {
    let directory = env::temp_dir().join(format!("coterm-serial-pair-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(directory.join("backend"))?;
    std::fs::create_dir_all(directory.join("frontend"))?;
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_coterm-plugin-serial"));
    std::fs::copy(
        &executable,
        directory.join("backend").join(
            executable
                .file_name()
                .ok_or_else(|| anyhow!("missing executable"))?,
        ),
    )?;
    std::fs::write(
        directory.join("manifest.json"),
        include_str!("../manifest.json"),
    )?;
    std::fs::write(
        directory.join("frontend/index.js"),
        "export function mount() {}",
    )?;
    Ok(directory)
}

async fn exercise(runtime: &ProcessPluginRuntime, a: &str, b: &str, baud: u32) -> Result<()> {
    let mut endpoints = Vec::new();
    for port in [a, b] {
        let instance = runtime
            .create_instance(PLUGIN, json!({"port":port,"baudRate":baud}))
            .await?;
        let endpoint = Endpoint {
            instance,
            cursor: 0,
        };
        endpoint.call(runtime, "serial.connect", json!({})).await?;
        endpoints.push(endpoint);
    }
    let mut peer = endpoints.pop().ok_or_else(|| anyhow!("missing peer"))?;
    let mut sender = endpoints.pop().ok_or_else(|| anyhow!("missing sender"))?;

    // The user-controlled Bypass still passes through source attribution and validation.
    runtime.set_bypass(true).await;
    let controls = "\u{3}\t\u{1b}[A\u{7f}\r\n中文\0";
    for (tool, commands, expected) in [
        (
            "serial.command",
            json!([controls, "next\r"]),
            format!("{controls}next\r").into_bytes(),
        ),
        (
            "serial.command_hex",
            json!(["00 80 FF", "0d0a"]),
            vec![0, 128, 255, 13, 10],
        ),
    ] {
        let result = coterm_mcp::server::call_tool(
            runtime,
            &json!({
                "name":format!("{PLUGIN}__{tool}"),
                "arguments":{"name":sender.instance.label,"commands":commands,"intervalMs":0}
            }),
        )
        .await?;
        ensure!(result["isError"] == false, "{result}");
        peer.expect_bytes(runtime, &expected).await?;
    }
    runtime.set_bypass(false).await;
    println!("PASS public MCP name routing, exact Shell controls, HEX lists, and Bypass");

    // A -> B 的每条命令分别写入；shell 符号和 UTF-8 按字节保留。
    let started = Instant::now();
    let op = sender.queue(runtime, json!({"commands":["first 中文", "second && literal", "third"],"lineEnding":"crlf","intervalMs":100})).await?;
    let expected = "first 中文\r\nsecond && literal\r\nthird\r\n";
    if let Err(error) = peer.expect_bytes(runtime, expected.as_bytes()).await {
        let operation = runtime
            .get(sender.caller(OperationSource::Human), &op)
            .await?;
        let source = sender.call(runtime, "serial.status", json!({})).await?;
        let destination = peer.call(runtime, "serial.status", json!({})).await?;
        return Err(error.context(format!(
            "first list: {operation:?}; source: {source}; peer: {destination}"
        )));
    }
    let result = sender
        .state(runtime, &op, OperationStatus::Completed)
        .await?;
    let output = result
        .output
        .ok_or_else(|| anyhow!("missing command result"))?;
    ensure!(
        output["sent"] == 3
            && output["bytes"] == expected.len()
            && output["deviceCompletion"] == "unknown",
        "{output}"
    );
    ensure!(
        started.elapsed() >= Duration::from_millis(180),
        "inter-command delay was not applied"
    );
    println!("PASS ordered list, UTF-8, CRLF, interval, and byte accounting");

    // B -> A 回传原始二进制，确保没有把底层 UART 强制改成文本。
    let binary = [0, 1, 2, 3, 0x7f, 0x80, 0xff, b'\r', b'\n'];
    peer.call(
        runtime,
        "serial.write_hex",
        json!({"hex":encode_hex(&binary)}),
    )
    .await?;
    sender.expect_bytes(runtime, &binary).await?;
    println!("PASS reverse binary receive and replay cursor");

    // 一个活动列表占住写端，再入队一个列表；取消后两者余下字节均不得送达。
    let active = sender
        .queue(
            runtime,
            json!({"commands":["cancel-before", "cancel-never"],"intervalMs":1000}),
        )
        .await?;
    peer.expect_bytes(runtime, b"cancel-before\r").await?;
    let queued = sender
        .queue(runtime, json!({"commands":["queued-never"]}))
        .await?;
    sender
        .state(runtime, &queued, OperationStatus::Running)
        .await?;
    for (name, args) in [
        ("serial.write", json!({"text":"raw-never"})),
        ("serial.signals", json!({"dtr":false})),
    ] {
        let rejected = runtime
            .submit(
                sender.caller(OperationSource::Human),
                sender.request(name, args, 3000),
            )
            .await?;
        ensure!(
            rejected.status == OperationStatus::Failed,
            "conflicting input was accepted: {rejected:?}"
        );
    }
    ensure!(
        runtime
            .cancel(sender.caller(OperationSource::Human), &queued)
            .await?
            .status
            == OperationStatus::Cancelled,
        "queued cancellation failed"
    );
    ensure!(
        runtime
            .cancel(sender.caller(OperationSource::Human), &active)
            .await?
            .status
            == OperationStatus::Cancelled,
        "active cancellation failed"
    );
    peer.expect_quiet(runtime, Duration::from_millis(1100))
        .await?;
    println!("PASS active/queued cancellation, no delayed writes, raw/signal conflict rejection");

    // MCP 原生操作走真实宿主的 Agent 来源识别与审批，而非直接调用领域后端。
    let proposed_args = json!({"pluginInstanceId":sender.instance.plugin_instance_id,"commands":["approval-original-never", "approval-next"]});
    let proposal = runtime
        .submit(
            sender.caller(OperationSource::Agent),
            sender.request("mcp.serial.command", proposed_args.clone(), 0),
        )
        .await?;
    sender
        .state(
            runtime,
            &proposal.operation_id,
            OperationStatus::AwaitingApproval,
        )
        .await?;
    peer.expect_quiet(runtime, Duration::from_millis(150))
        .await?;
    let agent_view = runtime
        .get(
            sender.caller(OperationSource::Agent),
            &proposal.operation_id,
        )
        .await?;
    ensure!(
        !serde_json::to_string(&agent_view)?.contains("approval-original-never"),
        "sensitive draft leaked"
    );
    let human_view = runtime
        .get(
            sender.caller(OperationSource::Human),
            &proposal.operation_id,
        )
        .await?;
    ensure!(
        human_view.output.as_ref().map(|value| &value["args"]) == Some(&proposed_args),
        "approval draft mismatch"
    );
    let approved_args = json!({"pluginInstanceId":sender.instance.plugin_instance_id,"commands":["approval-approved", "approval-next"],"lineEnding":"lf","intervalMs":0});
    runtime
        .approve(
            sender.caller(OperationSource::Human),
            ApproveProposal {
                operation_id: proposal.operation_id.clone(),
                proposal_id: proposal
                    .proposal_id
                    .ok_or_else(|| anyhow!("missing proposal id"))?,
                plugin_instance_id: Some(sender.instance.plugin_instance_id.clone()),
                args: approved_args,
            },
        )
        .await?;
    peer.expect_bytes(runtime, b"approval-approved\napproval-next\n")
        .await?;
    let approved = sender
        .state(runtime, &proposal.operation_id, OperationStatus::Completed)
        .await?;
    let output = approved
        .output
        .ok_or_else(|| anyhow!("missing MCP result"))?;
    let domain = output["structuredContent"].clone();
    ensure!(
        domain["sent"] == 2 && domain["remaining"] == 0,
        "bad MCP result: {domain}"
    );
    println!(
        "PASS MCP operation approval, edited snapshot, secret redaction, and final notification"
    );

    // 实际传输 64 项与含结束符恰好 16 KiB 的边界列表。
    let commands: Vec<_> = (0..64).map(|index| format!("item-{index:02}")).collect();
    let expected = format!("{}\r", commands.join("\r"));
    let op = sender
        .queue(runtime, json!({"commands":commands,"intervalMs":0}))
        .await?;
    peer.expect_bytes(runtime, expected.as_bytes()).await?;
    sender
        .state(runtime, &op, OperationStatus::Completed)
        .await?;
    let boundary = vec!["x".repeat(4095); 4];
    let expected = format!("{}\r", boundary.join("\r"));
    let op = sender
        .queue(runtime, json!({"commands":boundary,"intervalMs":0}))
        .await?;
    peer.expect_bytes(runtime, expected.as_bytes()).await?;
    let completed = sender
        .state(runtime, &op, OperationStatus::Completed)
        .await?;
    ensure!(
        completed.output.as_ref().map(|value| &value["bytes"]) == Some(&json!(16384)),
        "boundary byte count mismatch"
    );
    let invalid = runtime
        .submit(
            sender.caller(OperationSource::Human),
            sender.request(
                "serial.command",
                json!({"commands":["invalid-first-never", ""]}),
                3000,
            ),
        )
        .await?;
    ensure!(
        invalid.status == OperationStatus::Failed,
        "invalid later entry was accepted"
    );
    peer.expect_quiet(runtime, Duration::from_millis(100))
        .await?;
    println!("PASS 64 entries, 16384-byte boundary, multi-chunk replay, and whole-list validation");

    // 真实 actor 容量：长列表保持活动，另排队 15 项，第 17 个列表必须拒绝。
    let hold: Vec<_> = (0..64).map(|index| format!("hold-{index:02}")).collect();
    let active = sender
        .queue(runtime, json!({"commands":hold,"intervalMs":1000}))
        .await?;
    peer.expect_bytes(runtime, b"hold-00\r").await?;
    let mut queued = Vec::new();
    for index in 0..15 {
        let id = sender
            .queue(
                runtime,
                json!({"commands":[format!("capacity-never-{index}")]}),
            )
            .await?;
        sender.state(runtime, &id, OperationStatus::Running).await?;
        queued.push(id);
    }
    let status = sender.call(runtime, "serial.status", json!({})).await?;
    ensure!(
        status["commandQueue"] == 16,
        "queue capacity mismatch: {status}"
    );
    let rejected = runtime
        .submit(
            sender.caller(OperationSource::Human),
            sender.request(
                "serial.command",
                json!({"commands":["overflow-never"]}),
                3000,
            ),
        )
        .await?;
    ensure!(
        rejected.status == OperationStatus::Failed,
        "seventeenth list must be rejected"
    );
    sender.call(runtime, "serial.disconnect", json!({})).await?;
    sender
        .state(runtime, &active, OperationStatus::Unknown)
        .await?;
    for id in queued {
        sender.state(runtime, &id, OperationStatus::Unknown).await?;
    }
    // 如果容量探测较慢，hold 的后续项可能已发送；它们也只能来自活动列表。
    let already_sent = peer.read(runtime).await?;
    ensure!(
        String::from_utf8(already_sent)?
            .split('\r')
            .filter(|item| !item.is_empty())
            .all(|item| item.starts_with("hold-")),
        "queued commands escaped before disconnect"
    );
    peer.expect_quiet(runtime, Duration::from_millis(1100))
        .await?;
    println!("PASS 16-list capacity, overflow rejection, disconnect settlement, and no replay");

    // 重新打开只发送新请求；多轮连接/断开结束后仍可回收端口句柄。
    for index in 0..3 {
        sender.call(runtime, "serial.connect", json!({})).await?;
        let command = format!("reconnected-{index}");
        let op = sender
            .queue(runtime, json!({"commands":[command],"intervalMs":0}))
            .await?;
        peer.expect_bytes(runtime, format!("reconnected-{index}\r").as_bytes())
            .await?;
        sender
            .state(runtime, &op, OperationStatus::Completed)
            .await?;
        sender.call(runtime, "serial.disconnect", json!({})).await?;
    }
    peer.call(runtime, "serial.disconnect", json!({})).await?;
    println!("PASS reconnect loops and clean port shutdown");
    Ok(())
}

#[tokio::test]
#[ignore = "requires an explicitly selected, connected virtual serial pair"]
async fn virtual_pair_exercises_real_process_command_queue_and_approval() -> Result<()> {
    let a = env::var("COTERM_SERIAL_PAIR_A")
        .context("set COTERM_SERIAL_PAIR_A to the authorized sender port")?;
    let b = env::var("COTERM_SERIAL_PAIR_B")
        .context("set COTERM_SERIAL_PAIR_B to its paired peer port")?;
    ensure!(
        !a.eq_ignore_ascii_case(&b),
        "the virtual pair must use distinct ports"
    );
    let baud =
        env::var("COTERM_SERIAL_PAIR_BAUD").map_or(Ok(115200), |value| value.parse::<u32>())?;
    let directory = package()?;
    let runtime = ProcessPluginRuntime::new(
        platform()?,
        RuntimeLimits {
            max_control_bytes: 1024 * 1024,
            max_data_chunk_bytes: 64 * 1024,
            initial_window: FlowWindow {
                chunks: 16,
                bytes: 1024 * 1024,
            },
        },
    );
    let result = async {
        runtime.load(&directory).await?;
        runtime.start(PLUGIN).await?;
        exercise(&runtime, &a, &b, baud).await
    }
    .await;
    runtime.shutdown().await;
    std::fs::remove_dir_all(&directory).context("remove temporary test package")?;
    result
}
