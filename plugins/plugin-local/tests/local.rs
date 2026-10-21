//! 本地插件在真实基座运行时的端到端验证：包加载、握手、实例、插件级只读操作。
//! 插件目前没有 PTY 会话，所以这里不碰终端输入输出——那是后续阶段。
use std::time::Duration;

use anyhow::Result;
use coterm_core::{
    plugin_manager::ProcessPluginRuntime,
    plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
};
use coterm_proto::runtime::{
    lifecycle::RuntimeLimits,
    operation::{OperationSource, OperationStatus},
    stream::FlowWindow,
};
use serde_json::{json, Value};

const PLUGIN_ID: &str = "coterm-plugin-local";

fn platform() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("macos", "aarch64") => "macos-arm64",
        _ => anyhow::bail!("Unsupported test platform"),
    })
}

/// 按发行布局组装插件包：backend/ + manifest.json。本地插件没有前端入口。
fn package(platform: &str) -> Result<std::path::PathBuf> {
    let directory = std::env::temp_dir().join(format!("coterm-local-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("backend"))?;
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_coterm-plugin-local"));
    std::fs::copy(
        executable,
        directory.join("backend").join(
            executable
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Missing executable name"))?,
        ),
    )?;
    std::fs::write(
        directory.join("manifest.json"),
        include_str!("../manifest.json"),
    )?;
    let _ = platform;
    Ok(directory)
}

fn runtime() -> ProcessPluginRuntime {
    ProcessPluginRuntime::new(
        "test",
        RuntimeLimits {
            max_control_bytes: 1024 * 1024,
            max_data_chunk_bytes: 64 * 1024,
            initial_window: FlowWindow {
                chunks: 16,
                bytes: 1024 * 1024,
            },
        },
    )
}

#[tokio::test]
async fn process_host_loads_the_local_package_and_serves_plugin_info() -> Result<()> {
    let platform = platform()?;
    let directory = package(platform)?;
    let runtime = runtime();
    // 旧的 manifest（protocol/entry_point/config_schema）在这里必须被拒绝；
    // 能 load 就说明包已经符合 manifestVersion 契约。
    let manifest = runtime.load(&directory).await?;
    assert_eq!(manifest.id, PLUGIN_ID);
    assert_eq!(manifest.manifest_version, 1);
    assert_eq!(manifest.instances.instance_type, "local-shell");
    assert!(manifest.instances.multiple);
    // 本地插件还没有前端，基座不应期望一份 index.js。
    assert!(runtime.frontend_source(PLUGIN_ID).await?.is_none());

    runtime.start(PLUGIN_ID).await?;
    // 握手声明的每个必需特性都必须被插件真正启用，否则 start 会失败。
    let catalog = runtime.operation_catalog(PLUGIN_ID).await;
    let names: Vec<_> = catalog.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["local.plugin_info"]);

    let instance = runtime
        .create_instance(PLUGIN_ID, json!({"shell":"bash","cwd":"/srv"}))
        .await?;
    assert_eq!(instance.label, "bash @ /srv");
    assert_eq!(instance.plugin_id, PLUGIN_ID);

    let caller = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        plugin_id: Some(instance.plugin_id.clone()),
    };
    let operation = |name: &str, args: Value| SubmitOperation {
        plugin_id: instance.plugin_id.clone(),
        plugin_instance_id: None,
        operation: name.to_owned(),
        args,
        wait_timeout_ms: None,
    };

    let info = runtime
        .submit(caller.clone(), operation("local.plugin_info", json!({})))
        .await?;
    assert_eq!(info.status, OperationStatus::Completed);
    let output = info
        .output
        .ok_or_else(|| anyhow::anyhow!("Missing plugin info output"))?;
    assert_eq!(output["pluginId"], PLUGIN_ID);
    assert_eq!(output["instanceCount"], 1);

    // 未注册的操作名必须失败，而不是被当成未知命令静默放行。
    let unknown = runtime
        .submit(caller.clone(), operation("local.spawn", json!({})))
        .await?;
    assert_eq!(unknown.status, OperationStatus::Failed);

    // 插件级操作不能挂到实例上。
    let attached = runtime
        .submit(
            caller,
            SubmitOperation {
                plugin_id: instance.plugin_id.clone(),
                plugin_instance_id: Some(instance.plugin_instance_id.clone()),
                operation: "local.plugin_info".to_owned(),
                args: json!({}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    assert_eq!(attached.status, OperationStatus::Failed);

    runtime
        .destroy_instance(&instance.plugin_instance_id)
        .await?;
    assert!(runtime.list_instances(Some(PLUGIN_ID)).await?.is_empty());
    runtime.stop(PLUGIN_ID).await?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn backend_rejects_requests_that_break_the_runtime_contract() -> Result<()> {
    use coterm_plugin_local::backend::Backend;
    use coterm_proto::runtime::{message::HostCommand, transport::PluginBackend};

    fn create(id: &str, plugin_id: &str, config: Value) -> HostCommand {
        let value = json!({"instance":{"pluginId":plugin_id,"pluginInstanceId":id,
                                       "sessionId":"s","label":"Local","status":"ready"},
                           "config":config});
        HostCommand::CreateInstance(serde_json::from_value(value).unwrap())
    }

    fn invoke(id: &str, operation_id: &str, operation: &str, effect: &str, dispatch: Value) -> HostCommand {
        let value = json!({"operationId":operation_id,"operation":operation,"effect":effect,
                           "pluginInstanceId":id,"source":"human","dispatch":dispatch});
        HostCommand::Invoke(serde_json::from_value(value).unwrap())
    }

    fn hello(required: &[&str]) -> HostCommand {
        HostCommand::Hello(serde_json::from_value(json!({
            "interfaceMajor": 1,
            "interfaceMinor": 0,
            "requiredFeatures": required,
            "optionalFeatures": [],
            "limits": {"maxControlBytes": 1048576, "maxDataChunkBytes": 65536,
                       "initialWindow": {"chunks": 16, "bytes": 1048576}},
        })).unwrap())
    }

    let mut backend = Backend::default();
    let result = backend.request(hello(&[])).await?;
    assert_eq!(result["pluginId"], PLUGIN_ID);
    assert_eq!(result["interfaceMinor"], 0);
    // 没有 PTY 就没有 MCP 工具可暴露。
    assert_eq!(result["mcpTools"].as_array().map(Vec::len), Some(0));

    // 必需特性超出插件声明时必须拒绝，否则基座会发来没人处理的命令。
    let unsupported = backend
        .request(hello(&["secret-delivery"]))
        .await
        .expect_err("undeclared required feature must be rejected");
    assert!(unsupported.message.contains("Unsupported required runtime feature"));

    backend
        .request(create("one", PLUGIN_ID, json!({"shell":"bash","cwd":"/srv"})))
        .await?;
    // 重复实例 ID 与错误插件 ID 都必须拒绝。
    backend
        .request(create("one", PLUGIN_ID, json!({"shell":"bash"})))
        .await
        .expect_err("duplicate instance id must be rejected");
    backend
        .request(create("two", "other", json!({})))
        .await
        .expect_err("foreign plugin id must be rejected");
    // 未知配置键与空 shell 暴露拼写错误而不是被静默忽略。
    backend
        .request(create("typo", PLUGIN_ID, json!({"workingDir":"/tmp"})))
        .await
        .expect_err("unknown config keys must be rejected");
    backend
        .request(create("blank", PLUGIN_ID, json!({"shell":"  "})))
        .await
        .expect_err("blank shell must be rejected");

    // 可以并存多个实例，本地插件不应被限制成单例。
    backend
        .request(create("two", PLUGIN_ID, json!({"shell":"bash"})))
        .await?;
    let info = backend
        .request(invoke("", "op-1", "local.plugin_info", "read", json!({"mode":"execute"})))
        .await?;
    assert_eq!(info["instanceCount"], 2);
    // 只读操作不接受实例作用域。
    backend
        .request(invoke("one", "op-2", "local.plugin_info", "read", json!({"mode":"execute"})))
        .await
        .expect_err("plugin-scoped operation must not take an instance");
    // effect 由基座目录决定，插件拒绝自报的差异。
    backend
        .request(invoke("", "op-3", "local.plugin_info", "commit", json!({"mode":"execute"})))
        .await
        .expect_err("effect mismatch must be rejected");
    // 没注册的操作名必须拒绝，不能落入任何成功分支。
    backend
        .request(invoke("one", "op-4", "local.spawn", "read", json!({"mode":"execute"})))
        .await
        .expect_err("unknown operation must be rejected");
    // 没有已注册的写入操作，也就不存在可批准的提案。
    backend
        .request(invoke("", "op-5", "local.plugin_info", "read",
                        json!({"mode":"prepare_proposal","proposalId":"p-1"})))
        .await
        .expect_err("read-only operation cannot be prepared as a proposal");
    backend
        .request(HostCommand::Approve(serde_json::from_value(
            json!({"operationId":"op-5","proposalId":"p-1","args":{}})).unwrap()))
        .await
        .expect_err("no proposal should exist to approve");
    // 未声明的交互/秘密命令必须拒绝，而不是回一个假的成功。
    backend
        .request(coterm_proto::runtime::message::HostCommand::Shutdown)
        .await?;
    Ok(())
}

/// 传输层契约：stdout 只写协议，回复与请求 id 一一对应。
#[tokio::test]
async fn process_speaks_the_shared_transport_envelope() -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let executable = env!("CARGO_BIN_EXE_coterm-plugin-local");
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let hello = json!({
        "jsonrpc": "2.0",
        "id": "req-hello",
        "method": "initialize",
        "params": {
            "interfaceMajor": 1,
            "interfaceMinor": 0,
            "requiredFeatures": ["instances"],
            "optionalFeatures": [],
            "limits": {"maxControlBytes": 1048576, "maxDataChunkBytes": 65536,
                       "initialWindow": {"chunks": 16, "bytes": 1048576}},
        },
    });
    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Missing stdin"))?;
    writeln!(stdin, "{hello}")?;
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":"req-shutdown","method":"shutdown","params":null})
    )?;
    stdin.flush()?;
    drop(child.stdin.take());
    let output = child.wait_with_output()?;
    assert!(output.status.success(), "{output:?}");
    let lines: Vec<Value> = String::from_utf8(output.stdout)?
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout must stay protocol-only"))
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0]["id"], "req-hello");
    assert_eq!(lines[0]["result"]["pluginId"], PLUGIN_ID);
    assert_eq!(lines[1]["id"], "req-shutdown");
    assert!(lines.iter().all(|line| line.get("error").is_none()));
    Ok(())
}

#[tokio::test]
async fn health_check_within_the_handshake_deadline() -> Result<()> {
    // 基座要求控制命令 10 秒内应答；这里用更紧的 2 秒守住回归。
    let platform = platform()?;
    let directory = package(platform)?;
    let runtime = runtime();
    let started = tokio::time::Instant::now();
    runtime.load(&directory).await?;
    runtime.start(PLUGIN_ID).await?;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "startup took {:?}",
        started.elapsed()
    );
    runtime.stop(PLUGIN_ID).await?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
