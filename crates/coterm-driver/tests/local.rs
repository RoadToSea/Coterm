//! 本地驱动在真实基座运行时里的端到端验证：注册、能力协商、实例、PTY 读写与重启。
use std::time::{Duration, Instant};

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use coterm_core::model::{
    lifecycle::RuntimeLimits,
    operation::{OperationSource, OperationStatus},
};
use coterm_core::{
    runtime::Runtime,
    runtime::{CallerContext, SubmitOperation},
};
use serde_json::{json, Value};

const DRIVER_ID: &str = "coterm-driver-local";

/// 端到端测试固定用一个启动快、行为确定的 Shell。
///
/// PowerShell 在无终端环境里冷启动可能超过 20 秒（加载 profile 与模块），会让断言依赖
/// 机器状态；这里只验证 PTY 管路本身，Shell 选择由 `local.status` 与手动桌面验收覆盖。
fn test_shell() -> &'static str {
    if cfg!(windows) {
        "cmd.exe"
    } else {
        "/bin/sh"
    }
}

async fn register(runtime: &Runtime) -> Result<coterm_core::model::manifest::DriverManifest> {
    Ok(runtime
        .register::<coterm_driver::local::backend::Backend>(
            serde_json::from_str(include_str!("../manifests/local.json"))?,
            Some("export function mount() { return { unmount() {} }; }"),
        )
        .await?)
}

fn runtime() -> Runtime {
    Runtime::new(RuntimeLimits {
        max_control_bytes: 1024 * 1024,
    })
}

fn operation(name: &str, instance: Option<&str>, args: Value) -> SubmitOperation {
    SubmitOperation {
        driver_id: DRIVER_ID.to_owned(),
        driver_instance_id: instance.map(str::to_owned),
        operation: name.to_owned(),
        args,
        wait_timeout_ms: None,
    }
}

async fn call(
    runtime: &Runtime,
    caller: &CallerContext,
    name: &str,
    instance: Option<&str>,
    args: Value,
) -> Result<Value> {
    let receipt = runtime
        .submit(caller.clone(), operation(name, instance, args))
        .await?;
    if receipt.status != OperationStatus::Completed {
        let detail = receipt
            .error
            .as_ref()
            .map(|error| error.message.clone())
            .unwrap_or_default();
        anyhow::bail!("{name} finished as {:?}: {detail}", receipt.status);
    }
    receipt
        .output
        .ok_or_else(|| anyhow::anyhow!("{name} returned no output"))
}

/// 等终端初始化完成：读完当前回放并回应光标位置查询，但不判定内容。
///
/// PSReadLine 启动时会先探测光标位置，这期间到达的按键会被丢弃；真实用户也是等提示符
/// 出现后才输入，所以测试同样先空转一小段时间。
async fn settle(
    runtime: &Runtime,
    caller: &CallerContext,
    instance: &str,
    cursor: &str,
    duration: Duration,
) -> Result<String> {
    let deadline = Instant::now() + duration;
    let mut cursor = cursor.to_owned();
    let mut answered = 0;
    while Instant::now() < deadline {
        let page = call(
            runtime,
            caller,
            "local.read",
            Some(instance),
            json!({"cursor": cursor}),
        )
        .await?;
        if let Some(value) = page["cursor"].as_str() {
            cursor = value.to_owned();
        }
        let data = STANDARD
            .decode(page["data"].as_str().unwrap_or_default())
            .map_err(|_| anyhow::anyhow!("local.read must return base64"))?;
        let text = String::from_utf8_lossy(&data);
        let pending = text.matches("\u{1b}[6n").count();
        while answered < pending {
            answered += 1;
            call(
                runtime,
                caller,
                "local.input",
                Some(instance),
                json!({"data": STANDARD.encode(b"\x1b[1;1R")}),
            )
            .await?;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(cursor)
}

/// 反复读取回放缓冲，直到解出的文本包含期望内容或超时。
///
/// 同时替终端回答光标位置查询（`ESC[6n`）：真实前端由 xterm 自动回应，PowerShell 的
/// PSReadLine 会一直等这个答复，不回应就看不到提示符与回显。
async fn read_until(
    runtime: &Runtime,
    caller: &CallerContext,
    instance: &str,
    start_cursor: &str,
    needle: &str,
    minimum: usize,
) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut cursor = start_cursor.to_owned();
    let mut text = String::new();
    let mut answered = 0;
    while Instant::now() < deadline {
        let page = call(
            runtime,
            caller,
            "local.read",
            Some(instance),
            json!({"cursor": cursor}),
        )
        .await?;
        let cursor_value = page["cursor"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("cursor must be a decimal string"))?;
        cursor = cursor_value.to_owned();
        let data = STANDARD
            .decode(page["data"].as_str().unwrap_or_default())
            .map_err(|_| anyhow::anyhow!("local.read must return base64"))?;
        text.push_str(&String::from_utf8_lossy(&data));
        let pending = text.matches("\u{1b}[6n").count();
        while answered < pending {
            answered += 1;
            call(
                runtime,
                caller,
                "local.input",
                Some(instance),
                json!({"data": STANDARD.encode(b"\x1b[1;1R")}),
            )
            .await?;
        }
        if text.matches(needle).count() >= minimum {
            return Ok(text);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("shell output never contained {needle:?}; saw {text:?}")
}

/// 等一个延迟操作收敛到终态，返回它的领域结果。
async fn wait_completed(
    runtime: &Runtime,
    caller: &CallerContext,
    operation_id: &str,
) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        let record = runtime.get(caller.clone(), operation_id).await?;
        match record.status {
            OperationStatus::Completed => {
                return record
                    .output
                    .ok_or_else(|| anyhow::anyhow!("completed operation returned no output"))
            }
            OperationStatus::Failed | OperationStatus::Cancelled => {
                anyhow::bail!("operation ended as {:?}: {:?}", record.status, record.error)
            }
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    anyhow::bail!("operation {operation_id} never finished")
}

/// 与 Shell 家族无关的慢命令，用于超时与取消。
fn slow_script() -> &'static str {
    if cfg!(windows) {
        "ping -n 20 127.0.0.1 > nul"
    } else {
        "sleep 10"
    }
}

#[tokio::test]
async fn in_process_host_runs_a_local_shell_through_the_runtime() -> Result<()> {
    let runtime = runtime();
    let manifest = register(&runtime).await?;
    assert_eq!(manifest.id, DRIVER_ID);
    assert_eq!(manifest.manifest_version, 1);
    assert_eq!(manifest.instances.instance_type, "local-shell");
    assert!(manifest.instances.multiple);
    // 前端入口是必需的：基座没有配置界面就无法创建本地实例。
    let frontend = runtime
        .frontend_source(DRIVER_ID)
        .await?
        .ok_or_else(|| anyhow::anyhow!("local driver must ship a frontend entry"))?;
    assert!(frontend.contains("export function mount"));

    runtime.start(DRIVER_ID).await?;
    let catalog = runtime.operation_catalog(DRIVER_ID).await;
    let mut names: Vec<_> = catalog
        .iter()
        .map(|item| item.name.as_str())
        .filter(|name| !name.starts_with("mcp."))
        .collect();
    // 基座不保证目录顺序，按名字比较即可。
    names.sort_unstable();
    let mut expected = [
        "local.driver_info",
        "local.status",
        "local.read",
        "local.input",
        "local.resize",
        "local.restart",
        "local.exec",
        "local.output",
        "local.command",
        "local.read_dialogue_pairs",
        "local.read_lines",
        "local.read_selected_lines",
        "local.select_lines",
    ];
    expected.sort_unstable();
    assert_eq!(names, expected);

    // 指定一个确定存在的 Shell；留空时的探测另有单元测试覆盖。
    let instance = runtime
        .create_instance(DRIVER_ID, json!({"shell": test_shell()}))
        .await?;
    assert!(instance.label.contains(" @ "), "{}", instance.label);
    let caller = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };

    // 创建实例不启动进程；此时状态是空闲的。
    let idle = call(
        &runtime,
        &caller,
        "local.status",
        Some(&instance.driver_instance_id),
        json!({}),
    )
    .await?;
    assert_eq!(idle["running"], false);
    assert_eq!(idle["cursor"], "0");

    // 读取保持空闲；只有人工显式启动才创建 PTY。
    let first = call(
        &runtime,
        &caller,
        "local.read",
        Some(&instance.driver_instance_id),
        json!({"cursor": "0"}),
    )
    .await?;
    assert!(first["cursor"].as_str().is_some(), "{first:?}");
    assert!(first["shellId"].is_null());
    assert_eq!(first["data"], "");
    let idle = call(
        &runtime,
        &caller,
        "local.status",
        Some(&instance.driver_instance_id),
        json!({}),
    )
    .await?;
    assert_eq!(idle["running"], false);
    call(
        &runtime,
        &caller,
        "local.restart",
        Some(&instance.driver_instance_id),
        json!({}),
    )
    .await?;
    let status = call(
        &runtime,
        &caller,
        "local.status",
        Some(&instance.driver_instance_id),
        json!({}),
    )
    .await?;
    assert_eq!(status["running"], true);
    assert!(status["pid"].as_u64().is_some(), "{status:?}");

    // 等 Shell 初始化完（含光标位置查询的答复），再像用户一样输入命令。
    let cursor = settle(
        &runtime,
        &caller,
        &instance.driver_instance_id,
        "0",
        Duration::from_millis(2500),
    )
    .await?;

    // 人工按键：Shell 会回显输入并执行一遍，因此同一标记至少出现两次。
    let marker = format!("coterm-{}", std::process::id());
    let command = format!("echo {marker}\r");
    let acknowledged = call(
        &runtime,
        &caller,
        "local.input",
        Some(&instance.driver_instance_id),
        json!({"data": STANDARD.encode(command.as_bytes())}),
    )
    .await?;
    assert_eq!(acknowledged["bytes"], command.len());
    let text = read_until(
        &runtime,
        &caller,
        &instance.driver_instance_id,
        &cursor,
        &marker,
        2,
    )
    .await?;
    assert!(text.matches(&marker).count() >= 2, "{text:?}");

    let resized = call(
        &runtime,
        &caller,
        "local.resize",
        Some(&instance.driver_instance_id),
        json!({"rows": 40, "cols": 100}),
    )
    .await?;
    assert_eq!(resized["rows"], 40);
    assert_eq!(resized["cols"], 100);

    // 重启换掉 shellId，旧事件不会被误认为当前终端。
    let before = call(
        &runtime,
        &caller,
        "local.status",
        Some(&instance.driver_instance_id),
        json!({}),
    )
    .await?;
    let restarted = call(
        &runtime,
        &caller,
        "local.restart",
        Some(&instance.driver_instance_id),
        json!({}),
    )
    .await?;
    assert_eq!(restarted["restarted"], true);
    assert_ne!(restarted["status"]["shellId"], before["shellId"]);
    assert_eq!(restarted["status"]["running"], true);

    runtime
        .destroy_instance(&instance.driver_instance_id)
        .await?;
    assert!(runtime.list_instances(Some(DRIVER_ID)).await?.is_empty());
    runtime.stop(DRIVER_ID).await?;
    Ok(())
}

#[tokio::test]
async fn agent_commands_run_in_isolated_child_processes() -> Result<()> {
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(DRIVER_ID).await?;
    let instance = runtime
        .create_instance(DRIVER_ID, json!({"shell": test_shell()}))
        .await?;
    let caller = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let run = |name: &str, args: Value| operation(name, Some(&instance.driver_instance_id), args);

    // 正常命令：立即受理或已经完成，完成时通过通知返回结果。
    let exec = runtime
        .submit(
            caller.clone(),
            run("local.exec", json!({"script": "echo coterm-exec"})),
        )
        .await?;
    assert!(
        matches!(
            exec.status,
            OperationStatus::Running | OperationStatus::Completed
        ),
        "{exec:?}"
    );
    let output = wait_completed(&runtime, &caller, &exec.operation_id).await?;
    assert_eq!(output["exitCode"], 0, "{output:?}");
    assert!(
        output["stdout"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("coterm-exec"),
        "{output:?}"
    );
    // 独立子进程不继承共享 PTY 的状态。
    assert_eq!(output["executionContext"]["inheritsPtyState"], false);

    // 完整流可以按游标续读，且不挤占人工终端缓冲。
    let page = call(
        &runtime,
        &caller,
        "local.output",
        Some(&instance.driver_instance_id),
        json!({"operationId": exec.operation_id, "stdoutCursor": "0"}),
    )
    .await?;
    assert_eq!(page["done"], true, "{page:?}");
    assert!(page["stdout"]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("coterm-exec"));
    let terminal = call(
        &runtime,
        &caller,
        "local.read",
        Some(&instance.driver_instance_id),
        json!({"cursor": "0"}),
    )
    .await?;
    let terminal_bytes = STANDARD.decode(terminal["data"].as_str().unwrap_or_default())?;
    assert!(
        !String::from_utf8_lossy(&terminal_bytes).contains("coterm-exec"),
        "local.exec output must not appear in the interactive buffer"
    );

    // 非零退出码与 stderr 都要如实返回。
    let failure = runtime
        .submit(
            caller.clone(),
            run("local.exec", json!({"script": "echo oops 1>&2\nexit 3"})),
        )
        .await?;
    let output = wait_completed(&runtime, &caller, &failure.operation_id).await?;
    assert_eq!(output["exitCode"], 3, "{output:?}");
    assert!(
        output["stderr"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("oops"),
        "{output:?}"
    );

    // 超时必须真正结束子进程，并标记 timedOut。
    let slow = runtime
        .submit(
            caller.clone(),
            run(
                "local.exec",
                json!({"script": slow_script(), "timeoutMs": 1000}),
            ),
        )
        .await?;
    let output = wait_completed(&runtime, &caller, &slow.operation_id).await?;
    assert_eq!(output["timedOut"], true, "{output:?}");

    // 显式取消由进程级测试覆盖（tests/control.rs）：这里只确认超时能真正结束子进程。
    // 作业队列与游标缺口都按同一套有界语义实现，见 exec.rs。

    // 未登记或已淘汰的作业要明确失败，而不是返回空结果。
    let missing = call(
        &runtime,
        &caller,
        "local.output",
        Some(&instance.driver_instance_id),
        json!({"operationId": "op-does-not-exist"}),
    )
    .await
    .err()
    .ok_or_else(|| anyhow::anyhow!("unknown job must be rejected"))?;
    assert!(
        missing.to_string().contains("Local job not found"),
        "{missing:?}"
    );

    runtime
        .destroy_instance(&instance.driver_instance_id)
        .await?;
    runtime.stop(DRIVER_ID).await?;
    Ok(())
}

#[tokio::test]
async fn mcp_commands_preserve_approval_results_and_idle_terminal() -> Result<()> {
    use coterm_core::approval_policy::{ApprovalMode, ApprovalPolicy};
    use coterm_core::model::operation::ApproveProposal;

    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(DRIVER_ID).await?;
    let instance = runtime
        .create_instance(DRIVER_ID, json!({"shell":test_shell()}))
        .await?;
    let human = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(DRIVER_ID.to_owned()),
    };
    let agent = CallerContext {
        source: OperationSource::Agent,
        ..human.clone()
    };
    let id = &instance.driver_instance_id;
    for (name, target, args) in [
        (
            "mcp.local.status",
            Some(id.as_str()),
            json!({"driverInstanceId":id}),
        ),
        (
            "mcp.local.read_lines",
            Some(id.as_str()),
            json!({"driverInstanceId":id,"lines":10}),
        ),
        ("mcp.local.driver_info", None, json!({})),
    ] {
        let result = call(&runtime, &agent, name, target, args).await?;
        assert!(result["content"].is_array(), "{name}: {result}");
        assert!(result["structuredContent"].is_object());
        assert_eq!(result["isError"], false);
        if name == "mcp.local.status" {
            assert_eq!(result["structuredContent"]["connected"], false);
        }
    }
    let args = json!({"driverInstanceId":id,"mode":"exec","commands":["echo coterm-pending"]});
    let pending = runtime
        .submit_until_approval(
            agent.clone(),
            operation("mcp.local.command", Some(id), args),
        )
        .await?;
    assert_eq!(pending.status, OperationStatus::AwaitingApproval);
    let request = ApproveProposal {
        operation_id: pending.operation_id.clone(),
        proposal_id: pending
            .proposal_id
            .ok_or_else(|| anyhow::anyhow!("missing proposal"))?,
        driver_instance_id: Some(id.clone()),
        args: json!({"driverInstanceId":id,"mode":"exec","commands":["echo coterm-edited"]}),
    };
    let (first, second) = tokio::join!(
        runtime.approve(human.clone(), request.clone()),
        runtime.approve(human.clone(), request)
    );
    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "approval must execute once"
    );
    let result = wait_completed(&runtime, &human, &pending.operation_id).await?;
    assert!(result["content"].is_array());
    let stdout = result["structuredContent"]["stdout"]
        .as_str()
        .unwrap_or_default();
    assert!(stdout.contains("coterm-edited"), "{result}");
    assert!(!stdout.contains("coterm-pending"));
    let output = call(
        &runtime,
        &agent,
        "mcp.local.read_dialogue_pairs",
        Some(id),
        json!({"driverInstanceId":id,"count":2}),
    )
    .await?;
    assert!(output["content"].is_array());
    assert_eq!(
        output["structuredContent"]["pairs"][0]["status"],
        "completed"
    );

    runtime
        .set_approval_policy(
            id,
            ApprovalPolicy {
                mode: ApprovalMode::AllowMatches,
                patterns: vec!["^echo coterm-allowed$".to_owned()],
            },
        )
        .await?;
    let allowed = runtime
        .submit_until_approval(
            agent,
            operation(
                "mcp.local.command",
                Some(id),
                json!({"driverInstanceId":id,"mode":"exec","commands":["echo coterm-allowed"]}),
            ),
        )
        .await?;
    assert_eq!(allowed.status, OperationStatus::Completed);
    assert_eq!(
        allowed
            .output
            .as_ref()
            .and_then(|value| value.pointer("/structuredContent/exitCode")),
        Some(&json!(0))
    );
    let idle = call(&runtime, &human, "local.status", Some(id), json!({})).await?;
    assert!(idle["shellId"].is_null());
    assert_eq!(idle["running"], false);
    assert!(runtime.attention_snapshot().await.approvals.is_empty());
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn runtime_cancels_local_jobs_before_their_timeout() -> Result<()> {
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(DRIVER_ID).await?;
    let instance = runtime
        .create_instance(DRIVER_ID, json!({"shell":test_shell()}))
        .await?;
    let caller = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(DRIVER_ID.to_owned()),
    };
    let script = if cfg!(windows) {
        "ping -n 6 127.0.0.1 > nul"
    } else {
        "sleep 5"
    };
    let mut request = operation(
        "local.exec",
        Some(&instance.driver_instance_id),
        json!({"script":script,"timeoutMs":15000}),
    );
    request.wait_timeout_ms = Some(100);
    let running = runtime.submit(caller.clone(), request).await?;
    assert_eq!(running.status, OperationStatus::Running);
    let cancelled = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.cancel(caller.clone(), &running.operation_id),
    )
    .await??;
    assert_eq!(cancelled.status, OperationStatus::Cancelled);
    let output = call(
        &runtime,
        &caller,
        "local.output",
        Some(&instance.driver_instance_id),
        json!({"operationId":running.operation_id}),
    )
    .await?;
    assert_eq!(output["done"], true);
    assert_eq!(output["cancelled"], true);
    assert_eq!(
        runtime.get(caller, &running.operation_id).await?.status,
        OperationStatus::Cancelled
    );
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn backend_rejects_requests_that_break_the_runtime_contract() -> Result<()> {
    use coterm_core::driver::DriverBackend;
    use coterm_core::driver::HostCommand;
    use coterm_driver::local::backend::Backend;

    fn create(id: &str, driver_id: &str, config: Value) -> HostCommand {
        let value = json!({"instance":{"driverId":driver_id,"driverInstanceId":id,
                                       "sessionId":"s","label":"Local","status":"ready"},
                           "config":config});
        HostCommand::CreateInstance(serde_json::from_value(value).unwrap())
    }

    fn invoke(
        id: &str,
        operation_id: &str,
        operation: &str,
        effect: &str,
        dispatch: Value,
    ) -> HostCommand {
        let instance_id = if id.is_empty() { None } else { Some(id) };
        let value = json!({"operationId":operation_id,"operation":operation,"effect":effect,
                           "driverInstanceId":instance_id,"args":{},"source":"human","dispatch":dispatch});
        HostCommand::Invoke(serde_json::from_value(value).unwrap())
    }

    let definition = Backend::definition();
    assert_eq!(definition.driver_id, DRIVER_ID);
    assert!(!definition.features.iter().any(|f| f == "secret-delivery"));
    assert!(!definition.mcp_tools.is_empty());
    let mut backend = Backend::default();
    backend.request(create("one", DRIVER_ID, json!({}))).await?;
    // 重复实例 ID 与错误驱动 ID 都必须拒绝。
    backend
        .request(create("one", DRIVER_ID, json!({})))
        .await
        .expect_err("duplicate instance id must be rejected");
    backend
        .request(create("two", "other", json!({})))
        .await
        .expect_err("foreign driver id must be rejected");
    // 未知配置键暴露拼写错误，而不是被静默忽略。
    backend
        .request(create("typo", DRIVER_ID, json!({"workingDir":"/tmp"})))
        .await
        .expect_err("unknown config keys must be rejected");
    // 不存在的 Shell 与工作目录必须给出稳定错误码。
    let missing = backend
        .request(create(
            "missing",
            DRIVER_ID,
            json!({"shell":"coterm-no-such-shell"}),
        ))
        .await
        .expect_err("missing shell must be rejected");
    assert_eq!(missing.code, "local_shell_not_found");
    let missing = backend
        .request(create(
            "cwd",
            DRIVER_ID,
            json!({"cwd":"/coterm/no/such/directory"}),
        ))
        .await
        .expect_err("missing working directory must be rejected");
    assert_eq!(missing.code, "local_cwd_unavailable");

    // 可以并存多个实例，本地驱动不应被限制成单例。
    backend.request(create("two", DRIVER_ID, json!({}))).await?;
    let info = backend
        .request(invoke(
            "",
            "op-1",
            "local.driver_info",
            "read",
            json!({"mode":"execute"}),
        ))
        .await?;
    assert_eq!(info["instanceCount"], 2);
    // 只读驱动级操作不接受实例作用域。
    backend
        .request(invoke(
            "one",
            "op-2",
            "local.driver_info",
            "read",
            json!({"mode":"execute"}),
        ))
        .await
        .expect_err("driver-scoped operation must not take an instance");
    // 实例级操作必须定位实例。
    backend
        .request(invoke(
            "",
            "op-3",
            "local.status",
            "read",
            json!({"mode":"execute"}),
        ))
        .await
        .expect_err("instance operation must require an instance");
    // effect 由基座目录决定，驱动拒绝自报的差异。
    backend
        .request(invoke(
            "",
            "op-4",
            "local.driver_info",
            "commit",
            json!({"mode":"execute"}),
        ))
        .await
        .expect_err("effect mismatch must be rejected");
    // 没注册的操作名必须拒绝，不能落入任何成功分支。
    backend
        .request(invoke(
            "one",
            "op-5",
            "local.spawn",
            "read",
            json!({"mode":"execute"}),
        ))
        .await
        .expect_err("unknown operation must be rejected");
    // 只读操作不能被当作提案准备。
    backend
        .request(invoke(
            "",
            "op-6",
            "local.driver_info",
            "read",
            json!({"mode":"prepare_proposal","proposalId":"p-1"}),
        ))
        .await
        .expect_err("read-only operation cannot be prepared as a proposal");
    // 提交类操作可以先准备提案，批准后才执行。
    let prepared = backend
        .request(invoke(
            "one",
            "op-7",
            "local.restart",
            "commit",
            json!({"mode":"prepare_proposal","proposalId":"p-2"}),
        ))
        .await?;
    assert_eq!(prepared["proposalId"], "p-2");
    assert_eq!(prepared["operation"], "local.restart");
    backend
        .request(HostCommand::Approve(
            serde_json::from_value(
                json!({"operationId":"op-other","proposalId":"p-2","driverInstanceId":"one","args":{}}),
            )
            .unwrap(),
        ))
        .await
        .expect_err("a stale proposal target must not execute");
    backend
        .request(coterm_core::driver::HostCommand::Shutdown)
        .await?;
    Ok(())
}

#[tokio::test]
async fn health_check_within_the_handshake_deadline() -> Result<()> {
    // 基座要求控制命令 10 秒内应答；这里用更紧的 2 秒守住回归。
    let runtime = runtime();
    let started = tokio::time::Instant::now();
    register(&runtime).await?;
    runtime.start(DRIVER_ID).await?;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "startup took {:?}",
        started.elapsed()
    );
    runtime.stop(DRIVER_ID).await?;
    Ok(())
}
