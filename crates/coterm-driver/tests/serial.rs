//! 串口驱动的领域契约与真实基座加载验证，不打开或写入物理端口。
//! 真实字节往返只在显式启用的 `live` 测试中执行。

use anyhow::Result;
type Backend = coterm_core::diagnostic::Backend<coterm_driver::serial::backend::Backend>;
use serde_json::{json, Value};

const DRIVER_ID: &str = "coterm-driver-serial";

#[tokio::test]
async fn driver_info_lists_ports_before_any_instance_exists() -> Result<()> {
    let mut backend = Backend::default();
    let info = backend
        .request(
            "invoke",
            json!({
                "operationId":"discover-ports", "operation":"serial.driver_info", "effect":"read",
                "source":"human", "dispatch":{"mode":"execute"}, "args":{}
            }),
        )
        .await?;
    assert_eq!(info["driverId"], DRIVER_ID);
    assert_eq!(info["instanceCount"], 0);
    let ports = info["ports"].as_array().ok_or_else(|| {
        anyhow::anyhow!("Driver discovery must return a ports array, including when empty")
    })?;
    for port in ports {
        assert!(port["portName"]
            .as_str()
            .is_some_and(|name| !name.is_empty()));
        assert!(matches!(
            port["portType"]["kind"].as_str(),
            Some("usb" | "pci" | "bluetooth" | "unknown")
        ));
    }
    Ok(())
}

#[tokio::test]
async fn disconnected_configuration_is_validated_and_keeps_the_instance_port() -> Result<()> {
    let mut backend = Backend::default();
    backend.request("create_instance", json!({
        "instance":{"driverId":DRIVER_ID,"driverInstanceId":"settings","sessionId":"s","label":"Serial","status":"ready"},
        "config":{"port":"COM6","baudRate":115200}
    })).await?;
    let invoke = |args: Value| {
        json!({
            "operationId":"settings-op","operation":"serial.configure","effect":"commit",
            "driverInstanceId":"settings","driverId":DRIVER_ID,"source":"human",
            "dispatch":{"mode":"execute"},"args":args
        })
    };
    for args in [
        json!({}),
        json!({"port":"COM7"}),
        json!({"baudRate":0}),
        json!({"baudRate":4294967296_u64}),
        json!({"baudRate":1.5}),
        json!({"dataBits":9}),
        json!({"stopBits":3}),
        json!({"parity":"mark"}),
        json!({"dtr":"false"}),
        json!({"flowControl":"hardware"}),
        json!(null),
    ] {
        assert!(backend.request("invoke", invoke(args)).await.is_err());
    }
    let status = backend
        .request(
            "invoke",
            invoke(json!({
                "baudRate":9600,"dataBits":7,"stopBits":2,"parity":"even","dtr":false,"rts":false
            })),
        )
        .await?;
    assert_eq!(status["config"]["port"], "COM6");
    assert_eq!(status["config"]["baudRate"], 9600);
    assert_eq!(status["config"]["dataBits"], 7);
    assert_eq!(status["config"]["stopBits"], 2);
    assert_eq!(status["config"]["parity"], "even");
    assert_eq!(status["signals"]["dtr"], false);
    assert_eq!(status["connected"], false);
    // Patches preserve unspecified settings and do not open hardware.
    let status = backend
        .request("invoke", invoke(json!({"baudRate":19200})))
        .await?;
    assert_eq!(status["config"]["dataBits"], 7);
    assert_eq!(status["config"]["rts"], false);
    assert_eq!(status["connectionId"], Value::Null);
    assert_eq!(status["txBytes"], 0);
    let mut proposal = invoke(json!({"baudRate":38400}));
    proposal["source"] = json!("agent");
    proposal["dispatch"] = json!({"mode":"prepare_proposal","proposalId":"settings-proposal"});
    backend.request("invoke", proposal).await?;
    let status = backend
        .request("invoke", invoke(json!({"rts":true})))
        .await?;
    assert_eq!(
        status["config"]["baudRate"], 19200,
        "preparing a proposal does not change settings"
    );
    assert!(!coterm_driver::serial::backend::mcp_tools()
        .iter()
        .any(|tool| tool.operation.contains("configure")));
    Ok(())
}

async fn register(
    runtime: &coterm_core::runtime::Runtime,
) -> Result<coterm_core::model::manifest::DriverManifest> {
    Ok(runtime
        .register::<coterm_driver::serial::backend::Backend>(
            serde_json::from_str(include_str!("../manifests/serial.json"))?,
            Some("export function mount() { return { unmount() {} }; }"),
        )
        .await?)
}

/// 后端直接驱动的契约测试：不需要打开物理端口。
#[tokio::test]
async fn backend_rejects_requests_that_break_the_runtime_contract() -> Result<()> {
    let mut backend = Backend::default();
    let hello = json!({
        "interfaceMajor": 2,
        "interfaceMinor": 0,
        "requiredFeatures": [],
        "optionalFeatures": ["instances", "native-mcp-tools"],
        "limits": {"maxControlBytes": 1048576, "maxDataChunkBytes": 65536,
                   "initialWindow": {"chunks": 16, "bytes": 1048576}},
    });
    let result = backend.request("initialize", hello).await?;
    assert_eq!(result["driverId"], DRIVER_ID);
    let operations = result["operations"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing operation catalog"))?;
    let names: Vec<_> = operations
        .iter()
        .filter_map(|operation| operation["name"].as_str())
        .collect();
    assert!(names.contains(&"serial.connect"));
    assert!(names.contains(&"serial.command"));
    assert!(names.contains(&"mcp.serial.command"));
    assert!(!names.contains(&"mcp.serial.write"));
    // MCP 工具必须声明实例作用域参数，否则基座在握手期就会拒绝。
    for tool in result["mcpTools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing MCP tools"))?
    {
        let name = tool["descriptor"]["name"].as_str().unwrap_or_default();
        if name.starts_with("serial.") && name != "serial.driver_info" {
            assert!(
                tool["descriptor"]["inputSchema"]["required"]
                    .as_array()
                    .is_some_and(|required| required.iter().any(|item| item == "driverInstanceId")),
                "{name} must require driverInstanceId"
            );
        }
    }

    backend
        .request(
            "create_instance",
            json!({"instance":{"driverId":DRIVER_ID,"driverInstanceId":"one",
                               "sessionId":"s","label":"Serial","status":"ready"},
                   "config":{"port":"COM6","baudRate":115200}}),
        )
        .await?;
    // 同一物理端口不能被两个实例同时占用。
    let duplicate = backend
        .request(
            "create_instance",
            json!({"instance":{"driverId":DRIVER_ID,"driverInstanceId":"two",
                               "sessionId":"s","label":"Serial","status":"ready"},
                   "config":{"port":"com6","baudRate":115200}}),
        )
        .await
        .expect_err("duplicate port must be rejected");
    assert!(duplicate.to_string().contains("already used"));
    // 路径穿越、控制字符和包外路径一律拒绝；Unix 设备节点是合法端口名。
    for port in ["", "/etc/passwd", "/dev/../etc/shadow", "COM..6"] {
        backend
            .request(
                "create_instance",
                json!({"instance":{"driverId":DRIVER_ID,"driverInstanceId":format!("bad{port}"),
                                   "sessionId":"s","label":"Serial","status":"ready"},
                       "config":{"port":port,"baudRate":115200}}),
            )
            .await
            .expect_err("invalid port must be rejected");
    }
    // 设备节点与设备名都按平台规则接受，manifest 声明的平台因此保持一致。
    for (index, port) in ["COM7", "/dev/ttyUSB0"].into_iter().enumerate() {
        backend
            .request(
                "create_instance",
                json!({"instance":{"driverId":DRIVER_ID,"driverInstanceId":format!("ok{index}"),
                                   "sessionId":"s","label":"Serial","status":"ready"},
                       "config":{"port":port,"baudRate":115200}}),
            )
            .await
            .unwrap_or_else(|error| panic!("{port} must be accepted: {error}"));
        backend
            .request(
                "destroy_instance",
                json!({"driverInstanceId":format!("ok{index}")}),
            )
            .await?;
    }
    // 未知键暴露拼写错误而不是被静默忽略。
    backend
        .request(
            "create_instance",
            json!({"instance":{"driverId":DRIVER_ID,"driverInstanceId":"typo",
                               "sessionId":"s","label":"Serial","status":"ready"},
                   "config":{"port":"COM7","baudRate":115200,"databits":8}}),
        )
        .await
        .expect_err("unknown config keys must be rejected");

    let status = backend
        .request(
            "invoke",
            json!({"operationId":"op-1","operation":"serial.status","effect":"read",
                   "driverInstanceId":"one","args":{},"source":"human","dispatch":{"mode":"execute"},
                   "driverId":DRIVER_ID}),
        )
        .await?;
    assert_eq!(status["connected"], false);
    assert_eq!(status["config"]["baudRate"], 115200);
    // 状态同时给出信号电平与本次启动内的收发计数，供调试界面显示。
    assert_eq!(status["signals"]["dtr"], true);
    assert_eq!(status["signals"]["rts"], true);
    assert_eq!(status["rxBytes"], 0);
    assert_eq!(status["txBytes"], 0);
    assert_eq!(status["commandQueue"], 0);
    // 列表在审批准备阶段完成整批校验，不打开端口、不提前发送第一项。
    for operation in ["serial.command", "mcp.serial.command"] {
        let mut args = json!({"commands":["pwd", "ls -la"],"intervalMs":100});
        if operation.starts_with("mcp.") {
            args["driverInstanceId"] = json!("one");
        }
        let proposal = backend.request("invoke", json!({
            "operationId":"list-proposal","operation":operation,"effect":"commit",
            "driverInstanceId":"one","args":args,"source":"agent",
            "dispatch":{"mode":"prepare_proposal","proposalId":"p-list"},"driverId":DRIVER_ID
        })).await?;
        assert_eq!(proposal["args"]["commands"], json!(["pwd", "ls -la"]));
        backend
            .request(
                "approve",
                json!({"operationId":"list-proposal","proposalId":"p-list",
            "driverInstanceId":"one","args":args}),
            )
            .await
            .expect_err("approval on a closed port must fail");
        args["commands"] = json!(["valid-first-entry", ""]);
        args["lineEnding"] = json!("none");
        backend.request("invoke", json!({
            "operationId":"invalid-list","operation":operation,"effect":"commit",
            "driverInstanceId":"one","args":args,"source":"agent",
            "dispatch":{"mode":"prepare_proposal","proposalId":"p-invalid"},"driverId":DRIVER_ID
        })).await.expect_err("invalid later entry must reject the whole proposal");
    }
    // 未打开端口时写入与读取必须失败，而不是静默成功。
    backend
        .request(
            "invoke",
            json!({"operationId":"op-2","operation":"serial.write","effect":"commit",
                   "driverInstanceId":"one","args":{"text":"ls"},
                   "source":"human","dispatch":{"mode":"execute"},"driverId":DRIVER_ID}),
        )
        .await
        .expect_err("writing to a closed port must fail");
    // 信号改动同样要连接；空请求在提案阶段就被拒绝。
    backend
        .request(
            "invoke",
            json!({"operationId":"op-2b","operation":"serial.signals","effect":"commit",
                   "driverInstanceId":"one","args":{"dtr":false},
                   "source":"human","dispatch":{"mode":"execute"},"driverId":DRIVER_ID}),
        )
        .await
        .expect_err("changing signals on a closed port must fail");
    backend
        .request(
            "invoke",
            json!({"operationId":"op-2c","operation":"serial.signals","effect":"commit",
                   "driverInstanceId":"one","args":{},
                   "source":"agent","dispatch":{"mode":"prepare_proposal","proposalId":"p-signals"},
                   "driverId":DRIVER_ID}),
        )
        .await
        .expect_err("a signal proposal without a target level must fail");
    backend
        .request(
            "invoke",
            json!({"operationId":"op-3","operation":"serial.read","effect":"read",
                   "driverInstanceId":"one","args":{"cursor":0},
                   "source":"human","dispatch":{"mode":"execute"},"driverId":DRIVER_ID}),
        )
        .await
        .expect_err("reading a closed port must fail");
    // 未知游标不是静默空结果。
    backend
        .request(
            "invoke",
            json!({"operationId":"op-4","operation":"serial.read","effect":"read",
                   "driverInstanceId":"one","args":{"cursor":"nope"},
                   "source":"human","dispatch":{"mode":"execute"},"driverId":DRIVER_ID}),
        )
        .await
        .expect_err("invalid cursor must fail");

    // 提交草稿只校验参数形状，不触碰硬件。
    let proposal = backend
        .request(
            "invoke",
            json!({"operationId":"op-5","operation":"serial.write","effect":"commit",
                   "driverInstanceId":"one","args":{"text":"ls","lineEnding":"crlf"},
                   "source":"agent","dispatch":{"mode":"prepare_proposal","proposalId":"p-1"},"driverId":DRIVER_ID}),
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
            json!({"operationId":"op-5","proposalId":"p-1","driverInstanceId":"one",
                   "args":{"text":"ls"}}),
        )
        .await
        .expect_err("a rejected proposal must not be approvable");

    backend
        .request("destroy_instance", json!({"driverInstanceId":"one"}))
        .await?;
    backend.request("shutdown", Value::Null).await?;
    Ok(())
}

#[tokio::test]
async fn in_process_host_registers_serial_and_validates_mcp_tools() -> Result<()> {
    use coterm_core::model::{
        lifecycle::RuntimeLimits,
        operation::{OperationSource, OperationStatus},
    };
    use coterm_core::{
        runtime::Runtime,
        runtime::{CallerContext, SubmitOperation},
    };
    let runtime = Runtime::new(RuntimeLimits {
        max_control_bytes: 1024 * 1024,
    });
    // 基座握手会校验实例作用域的 MCP 工具必须要求 driverInstanceId。
    register(&runtime).await?;
    runtime.start(DRIVER_ID).await?;
    assert!(runtime.frontend_source(DRIVER_ID).await?.is_some());
    let instance = runtime
        .create_instance(
            DRIVER_ID,
            json!({"port":"COM6","baudRate":115200,"dataBits":8,"stopBits":1,"parity":"none"}),
        )
        .await?;
    assert_eq!(instance.label, "COM6@115200");
    // Exercise the public bridge contract, not only the internal mirror operations.
    let tools = coterm_mcp::server::tools_list(&runtime).await?;
    let tool = tools["tools"]
        .as_array()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["name"] == "coterm_stream_command")
        })
        .ok_or_else(|| anyhow::anyhow!("Missing public HEX tool"))?;
    assert!(tool["inputSchema"]["properties"]
        .get("driverInstanceId")
        .is_none());
    assert!(tool["inputSchema"]["required"]
        .as_array()
        .is_some_and(|required| !required.contains(&json!("name"))));
    let command_tool = tools["tools"]
        .as_array()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["name"] == "coterm_stream_command")
        })
        .ok_or_else(|| anyhow::anyhow!("Missing public text tool"))?;
    assert_eq!(
        command_tool["inputSchema"]["properties"]["commands"]["items"]["type"],
        "string"
    );
    assert_eq!(
        command_tool["inputSchema"]["properties"]["waitForPrompt"]["type"],
        "string"
    );
    assert_eq!(
        command_tool["inputSchema"]["properties"]["encoding"]["default"],
        "utf-8"
    );
    assert!(
        command_tool["inputSchema"]["properties"]["name"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("coterm_instances"))
    );
    let listed = tools["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing tools"))?;
    let read_tool = listed
        .iter()
        .find(|tool| tool["name"] == "coterm_read_output")
        .ok_or_else(|| anyhow::anyhow!("Missing public line-read tool"))?;
    assert_eq!(
        read_tool["inputSchema"]["properties"]["mode"]["default"],
        "lines"
    );
    assert!(read_tool["inputSchema"]["properties"]
        .get("cursor")
        .is_none());
    for removed in [
        "coterm_serial_read",
        "coterm_serial_read_hex",
        "coterm_serial_get_recent",
    ] {
        assert!(!listed.iter().any(|tool| tool["name"] == removed));
    }
    for name in ["coterm_instances", "coterm_connection", "coterm_operation"] {
        assert!(listed.iter().any(|tool| tool["name"] == name));
    }
    let missing_name = coterm_mcp::server::call_tool(
        &runtime,
        &json!({"name":"coterm_serial_status","arguments":{}}),
    )
    .await
    .expect_err("instance-scoped tools must explain their routing argument");
    assert!(missing_name.to_string().contains("name"));
    let health =
        coterm_mcp::server::call_tool(&runtime, &json!({"name":"coterm.status","arguments":{}}))
            .await?;
    assert_eq!(health["structuredContent"]["connectedInstances"], 0);
    assert_eq!(
        health["structuredContent"]["instances"][0]["connected"],
        false
    );
    assert!(!health.to_string().contains(&instance.driver_instance_id));
    let invalid_field = coterm_mcp::server::call_tool(
        &runtime,
        &json!({
            "name":"coterm_serial_command",
            "arguments":{
                "name":instance.label,
                "commands":["secret-command-value"],
                "line_ending":"cr"
            }
        }),
    )
    .await
    .expect_err("unknown argument fields must fail schema validation");
    assert!(invalid_field.to_string().contains("line_ending"));
    assert!(!invalid_field.to_string().contains("secret-command-value"));
    for field in ["name", "instance", "instanceName", "instance_name"] {
        let status = coterm_mcp::server::call_tool(
            &runtime,
            &json!({
                "name":"coterm_serial_status",
                "arguments":{field:instance.label}
            }),
        )
        .await?;
        assert_eq!(status["isError"], false, "{field}");
        assert_eq!(status["structuredContent"]["connected"], false);
    }
    let duplicate_names = coterm_mcp::server::call_tool(
        &runtime,
        &json!({
            "name":"coterm_serial_status",
            "arguments":{"name":instance.label,"instance":"different"}
        }),
    )
    .await
    .expect_err("ambiguous instance aliases must not silently pick a target");
    assert!(duplicate_names.to_string().contains("exactly one"));
    let proposal = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        coterm_mcp::server::call_tool(
            &runtime,
            &json!({
                "name":"coterm_serial_connect",
                "arguments":{"instanceName":instance.label}
            }),
        ),
    )
    .await??;
    assert_eq!(proposal["structuredContent"]["status"], "pending_approval");
    assert_eq!(
        proposal["_meta"]["coterm/operation"]["status"],
        "awaiting_approval"
    );
    let proposal_id = proposal["_meta"]["coterm/operation"]["operationId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing pending operation ID"))?;
    let proposal = runtime
        .get(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            proposal_id,
        )
        .await?;
    runtime
        .reject(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            proposal_id,
            proposal
                .proposal_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Missing proposal identifier"))?,
        )
        .await?;
    let listed = coterm_mcp::server::call_tool(
        &runtime,
        &json!({"name":"coterm.list_instances","arguments":{}}),
    )
    .await?;
    assert!(!listed.to_string().contains(&instance.driver_instance_id));
    for (tool, commands) in [
        ("serial.command", json!(["\u{3}\r\n\u{1b}[A"])),
        ("serial.command_hex", json!(["00 80 FF"])),
    ] {
        let result = coterm_mcp::server::call_tool(
            &runtime,
            &json!({
                "name":format!("coterm-driver-serial__{tool}"),
                "arguments":{"name":instance.label,"commands":commands},
                "_meta":{"coterm/waitTimeoutMs":0}
            }),
        )
        .await?;
        let operation_id = result["_meta"]["coterm/operation"]["operationId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing MCP operation"))?;
        let caller = CallerContext {
            source: OperationSource::Human,
            session_id: instance.session_id.clone(),
            driver_id: Some(instance.driver_id.clone()),
        };
        let record = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let record = runtime.get(caller.clone(), operation_id).await?;
                if record.status == OperationStatus::AwaitingApproval {
                    return Ok::<_, anyhow::Error>(record);
                }
                anyhow::ensure!(record.error.is_none(), "{record:?}");
                tokio::task::yield_now().await;
            }
        })
        .await??;
        runtime
            .reject(
                caller,
                operation_id,
                record
                    .proposal_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Missing approval"))?,
            )
            .await?;
    }
    let caller = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let operation = |name: &str, args: Value| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
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
                driver_id: instance.driver_id.clone(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "mcp.serial.status".to_owned(),
                args: json!({"driverInstanceId":instance.driver_instance_id}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    assert_eq!(mcp.status, OperationStatus::Completed);
    let mismatch = runtime
        .submit(
            caller.clone(),
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "mcp.serial.status".to_owned(),
                args: json!({"driverInstanceId":"other-instance"}),
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
            operation("serial.write", json!({"text":"\r"})),
        )
        .await?;
    assert_eq!(failed.status, OperationStatus::Failed);
    assert!(failed.error.is_some());

    // 真宿主来源识别、审批和脱敏：Agent 的列表不能直接进入设备队列。
    let secret_command = "echo serial-test-sensitive-token";
    let agent = CallerContext {
        source: OperationSource::Agent,
        session_id: instance.session_id.clone(),
        driver_id: None,
    };
    let proposal = runtime
        .submit(
            agent.clone(),
            SubmitOperation { wait_timeout_ms: Some(1000), ..operation(
                "mcp.serial.command",
                json!({
                    "driverInstanceId":instance.driver_instance_id,"commands":[secret_command,"pwd"]
                }),
            ) },
        )
        .await?;
    assert_eq!(proposal.status, OperationStatus::AwaitingApproval);
    let history = runtime
        .recent(caller.clone(), &instance.driver_instance_id, 100)
        .await?;
    assert!(!serde_json::to_string(&history)?.contains(secret_command));
    let agent_view = runtime.get(agent, &proposal.operation_id).await?;
    assert!(!serde_json::to_string(&agent_view)?.contains(secret_command));
    let human_view = runtime.get(caller.clone(), &proposal.operation_id).await?;
    assert_eq!(
        human_view
            .output
            .as_ref()
            .map(|output| &output["args"]["commands"]),
        Some(&json!([secret_command, "pwd"])),
        "human must see the full list being approved"
    );
    let proposal_id = proposal
        .proposal_id
        .ok_or_else(|| anyhow::anyhow!("Missing command approval"))?;
    let approved = runtime.approve(caller.clone(), coterm_core::model::operation::ApproveProposal {
        operation_id: proposal.operation_id, proposal_id,
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        args: json!({"driverInstanceId":instance.driver_instance_id,"commands":[secret_command,"pwd"]}),
    }).await?;
    assert_eq!(
        approved.status,
        OperationStatus::Failed,
        "approved list still cannot write to a closed port"
    );

    runtime.stop(DRIVER_ID).await?;
    Ok(())
}
