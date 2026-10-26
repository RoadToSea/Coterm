//! 串口插件的领域契约与真实基座加载验证，不打开或写入物理端口。
//! 真实字节往返只在显式启用的 `live` 测试中执行。

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use coterm_plugin_serial::backend::Backend;
use serde_json::{json, Value};

const PLUGIN_ID: &str = "coterm-plugin-serial";

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

fn package(platform: &str) -> Result<std::path::PathBuf> {
    let directory = std::env::temp_dir().join(format!("coterm-serial-test-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("backend"))?;
    std::fs::create_dir_all(directory.join("frontend"))?;
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_coterm-plugin-serial"));
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
    std::fs::write(
        directory.join("frontend/index.js"),
        "export function mount() {}",
    )?;
    let _ = platform;
    Ok(directory)
}

/// 后端直接驱动的契约测试：不需要打开物理端口。
#[tokio::test]
async fn backend_rejects_requests_that_break_the_runtime_contract() -> Result<()> {
    let mut backend = Backend::default();
    let hello = json!({
        "interfaceMajor": 1,
        "interfaceMinor": 0,
        "requiredFeatures": [],
        "optionalFeatures": ["instances", "native-mcp-tools"],
        "limits": {"maxControlBytes": 1048576, "maxDataChunkBytes": 65536,
                   "initialWindow": {"chunks": 16, "bytes": 1048576}},
    });
    let result = backend.request("initialize", hello).await?;
    assert_eq!(result["pluginId"], PLUGIN_ID);
    let operations = result["operations"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing operation catalog"))?;
    let names: Vec<_> = operations
        .iter()
        .filter_map(|operation| operation["name"].as_str())
        .collect();
    assert!(names.contains(&"serial.connect"));
    assert!(!names.contains(&"mcp.serial.write"));
    // MCP 工具必须声明实例作用域参数，否则基座在握手期就会拒绝。
    for tool in result["mcpTools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing MCP tools"))?
    {
        let name = tool["descriptor"]["name"].as_str().unwrap_or_default();
        if name.starts_with("serial.") && name != "serial.plugin_info" {
            assert!(
                tool["descriptor"]["inputSchema"]["required"]
                    .as_array()
                    .is_some_and(|required| required.iter().any(|item| item == "pluginInstanceId")),
                "{name} must require pluginInstanceId"
            );
        }
    }

    backend
        .request(
            "create_instance",
            json!({"instance":{"pluginId":PLUGIN_ID,"pluginInstanceId":"one",
                               "sessionId":"s","label":"Serial","status":"ready"},
                   "config":{"port":"COM6","baudRate":115200}}),
        )
        .await?;
    // 同一物理端口不能被两个实例同时占用。
    let duplicate = backend
        .request(
            "create_instance",
            json!({"instance":{"pluginId":PLUGIN_ID,"pluginInstanceId":"two",
                               "sessionId":"s","label":"Serial","status":"ready"},
                   "config":{"port":"com6","baudRate":115200}}),
        )
        .await
        .expect_err("duplicate port must be rejected");
    assert!(duplicate.to_string().contains("already used"));
    // 路径穿越和控制字符一律拒绝。
    for port in ["", "/dev/ttyUSB0", "COM..6"] {
        backend
            .request(
                "create_instance",
                json!({"instance":{"pluginId":PLUGIN_ID,"pluginInstanceId":format!("bad{port}"),
                                   "sessionId":"s","label":"Serial","status":"ready"},
                       "config":{"port":port,"baudRate":115200}}),
            )
            .await
            .expect_err("invalid port must be rejected");
    }
    // 未知键暴露拼写错误而不是被静默忽略。
    backend
        .request(
            "create_instance",
            json!({"instance":{"pluginId":PLUGIN_ID,"pluginInstanceId":"typo",
                               "sessionId":"s","label":"Serial","status":"ready"},
                   "config":{"port":"COM7","baudRate":115200,"databits":8}}),
        )
        .await
        .expect_err("unknown config keys must be rejected");

    let status = backend
        .request(
            "invoke",
            json!({"operationId":"op-1","operation":"serial.status","effect":"read",
                   "pluginInstanceId":"one","args":{},"source":"human","dispatch":{"mode":"execute"},
                   "pluginId":PLUGIN_ID}),
        )
        .await?;
    assert_eq!(status["connected"], false);
    assert_eq!(status["config"]["baudRate"], 115200);
    // 未打开端口时写入与读取必须失败，而不是静默成功。
    backend
        .request(
            "invoke",
            json!({"operationId":"op-2","operation":"serial.write","effect":"commit",
                   "pluginInstanceId":"one","args":{"data":STANDARD.encode(b"ls")},
                   "source":"human","dispatch":{"mode":"execute"},"pluginId":PLUGIN_ID}),
        )
        .await
        .expect_err("writing to a closed port must fail");
    backend
        .request(
            "invoke",
            json!({"operationId":"op-3","operation":"serial.read","effect":"read",
                   "pluginInstanceId":"one","args":{"cursor":"0"},
                   "source":"human","dispatch":{"mode":"execute"},"pluginId":PLUGIN_ID}),
        )
        .await
        .expect_err("reading a closed port must fail");
    // 未知游标不是静默空结果。
    backend
        .request(
            "invoke",
            json!({"operationId":"op-4","operation":"serial.read","effect":"read",
                   "pluginInstanceId":"one","args":{"cursor":"nope"},
                   "source":"human","dispatch":{"mode":"execute"},"pluginId":PLUGIN_ID}),
        )
        .await
        .expect_err("invalid cursor must fail");

    // 提交草稿只校验参数形状，不触碰硬件。
    let proposal = backend
        .request(
            "invoke",
            json!({"operationId":"op-5","operation":"serial.write","effect":"commit",
                   "pluginInstanceId":"one","args":{"data":STANDARD.encode(b"ls"),"lineEnding":"crlf"},
                   "source":"agent","dispatch":{"mode":"prepare_proposal","proposalId":"p-1"},"pluginId":PLUGIN_ID}),
        )
        .await?;
    assert_eq!(proposal["proposalId"], "p-1");
    let rejected = backend
        .request(
            "reject",
            json!({"operationId":"op-5","proposalId":"p-1","reason":"changed my mind"}),
        )
        .await?;
    assert_eq!(rejected["ok"], true);
    // 提案被移除后不能再次审批。
    backend
        .request(
            "approve",
            json!({"operationId":"op-5","proposalId":"p-1","pluginInstanceId":"one",
                   "args":{"data":STANDARD.encode(b"ls")}}),
        )
        .await
        .expect_err("a rejected proposal must not be approvable");

    backend
        .request("destroy_instance", json!({"pluginInstanceId":"one"}))
        .await?;
    backend.request("shutdown", Value::Null).await?;
    Ok(())
}

#[tokio::test]
async fn process_host_loads_the_serial_package_and_validates_mcp_tools() -> Result<()> {
    use coterm_core::{
        plugin_manager::ProcessPluginRuntime,
        plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
    };
    use coterm_proto::runtime::{
        lifecycle::RuntimeLimits,
        operation::{OperationSource, OperationStatus},
        stream::FlowWindow,
    };
    let platform = platform()?;
    let directory = package(platform)?;
    let runtime = ProcessPluginRuntime::new(
        platform,
        RuntimeLimits {
            max_control_bytes: 1024 * 1024,
            max_data_chunk_bytes: 64 * 1024,
            initial_window: FlowWindow {
                chunks: 16,
                bytes: 1024 * 1024,
            },
        },
    );
    // 基座握手会校验实例作用域的 MCP 工具必须要求 pluginInstanceId。
    runtime.load(&directory).await?;
    runtime.start(PLUGIN_ID).await?;
    assert!(runtime.frontend_source(PLUGIN_ID).await?.is_some());
    let instance = runtime
        .create_instance(
            PLUGIN_ID,
            json!({"port":"COM6","baudRate":115200,"dataBits":8,"stopBits":1,"parity":"none"}),
        )
        .await?;
    assert_eq!(instance.label, "COM6@115200");
    let caller = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        plugin_id: Some(instance.plugin_id.clone()),
    };
    let operation = |name: &str, args: Value| SubmitOperation {
        plugin_id: instance.plugin_id.clone(),
        plugin_instance_id: Some(instance.plugin_instance_id.clone()),
        operation: name.to_owned(),
        args,
        wait_timeout_ms: None,
    };
    // probe 只读枚举，不需要端口存在。
    let probe = runtime
        .submit(caller.clone(), operation("serial.probe", json!({})))
        .await?;
    assert_eq!(probe.status, OperationStatus::Completed);
    let output = probe
        .output
        .ok_or_else(|| anyhow::anyhow!("Missing probe output"))?;
    assert_eq!(output["config"]["port"], "COM6");
    assert!(output["ports"].is_array());

    // MCP 实例绑定与物理设备是否存在无关，所有测试机都必须执行。
    let mcp = runtime
        .submit(
            caller.clone(),
            SubmitOperation {
                plugin_id: instance.plugin_id.clone(),
                plugin_instance_id: Some(instance.plugin_instance_id.clone()),
                operation: "mcp.serial.status".to_owned(),
                args: json!({"pluginInstanceId":instance.plugin_instance_id}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    assert_eq!(mcp.status, OperationStatus::Completed);
    let mismatch = runtime
        .submit(
            caller.clone(),
            SubmitOperation {
                plugin_id: instance.plugin_id.clone(),
                plugin_instance_id: Some(instance.plugin_instance_id.clone()),
                operation: "mcp.serial.status".to_owned(),
                args: json!({"pluginInstanceId":"other-instance"}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    assert_eq!(mismatch.status, OperationStatus::Failed);
    assert!(
        mismatch
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("mismatch")),
        "{mismatch:?}"
    );

    // 实例从未连接；验证宿主传递写入失败，不尝试打开固定 COM6。
    let failed = runtime
        .submit(
            caller.clone(),
            operation("serial.write", json!({"data":STANDARD.encode(b"\r")})),
        )
        .await?;
    assert_eq!(failed.status, OperationStatus::Failed);
    assert!(failed.error.is_some());

    runtime.stop(PLUGIN_ID).await?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
