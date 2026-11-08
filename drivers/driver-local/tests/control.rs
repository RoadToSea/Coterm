//! The host adapter and a real driver process must preserve the shared contract.
use anyhow::Result;
use coterm_core::{driver_process::JsonRpcDriverProcess, driver_runtime::DriverConnection};
use coterm_proto::runtime::{
    lifecycle::{HelloResult, RuntimeHello},
    message::HostCommand,
    ErrorCategory,
};
use serde_json::json;

#[tokio::test]
async fn structured_errors_and_rejected_oversize_requests_keep_process_usable() -> Result<()> {
    let process = JsonRpcDriverProcess::spawn(
        std::path::Path::new(env!("CARGO_BIN_EXE_coterm-driver-local")),
        &[],
    )
    .await?;
    let hello: RuntimeHello = serde_json::from_value(json!({
        "interfaceMajor":2,"interfaceMinor":0,
        "requiredFeatures":["instances","backend-calls","operation-catalog"],"optionalFeatures":[],
        "limits":{"maxControlBytes":1024,"maxDataChunkBytes":65536,"initialWindow":{"chunks":16,"bytes":1048576}}
    }))?;
    let accepted: HelloResult =
        serde_json::from_value(process.request(HostCommand::Hello(hello.clone())).await?)?;
    hello.validate_result(&accepted)?;
    process.set_control_limit(accepted.limits.max_control_bytes);
    let error = process
        .request(HostCommand::DestroyInstance {
            driver_instance_id: "missing".to_owned(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Domain);
    assert_eq!(error.code, "local_domain_error");
    let error = process
        .request(HostCommand::DestroyInstance {
            driver_instance_id: "x".repeat(4096),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "driver_control_too_large");
    assert_eq!(process.request(HostCommand::Shutdown).await?["ok"], true);
    process.close().await?;
    Ok(())
}

/// 延迟作业必须在收到取消后真正结束子进程，而不是只把状态改掉。
#[tokio::test]
async fn a_running_job_stops_when_cancelled() -> Result<()> {
    use coterm_proto::runtime::message::CancelTarget;

    let process = JsonRpcDriverProcess::spawn(
        std::path::Path::new(env!("CARGO_BIN_EXE_coterm-driver-local")),
        &[],
    )
    .await?;
    let hello: RuntimeHello = serde_json::from_value(json!({
        "interfaceMajor":2,"interfaceMinor":0,
        "requiredFeatures":["instances","backend-calls","operation-catalog","async-operations"],
        "optionalFeatures":[],
        "limits":{"maxControlBytes":1048576,"maxDataChunkBytes":65536,"initialWindow":{"chunks":16,"bytes":1048576}}
    }))?;
    let accepted: HelloResult =
        serde_json::from_value(process.request(HostCommand::Hello(hello.clone())).await?)?;
    hello.validate_result(&accepted)?;
    process.set_control_limit(accepted.limits.max_control_bytes);

    process
        .request(HostCommand::CreateInstance(serde_json::from_value(
            json!({
                "instance":{"driverId":"coterm-driver-local","driverInstanceId":"i1",
                            "sessionId":"s1","label":"Local","status":"ready"},
                "config":{"shell": if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }},
            }),
        )?))
        .await?;

    let script = if cfg!(windows) {
        "ping -n 20 127.0.0.1 > nul"
    } else {
        "sleep 10"
    };
    let deferred = process
        .request(HostCommand::Invoke(serde_json::from_value(json!({
            "operationId":"op-slow","operation":"local.exec","effect":"commit",
            "driverInstanceId":"i1","args":{"script":script,"timeoutMs":30000},
            "source":"human","dispatch":{"mode":"execute"},
        }))?))
        .await?;
    assert_eq!(deferred["_cotermDeferred"], "op-slow");

    let cancelled = process
        .request(HostCommand::Cancel(CancelTarget::Operation(
            "op-slow".to_owned(),
        )))
        .await?;
    assert_eq!(cancelled["outcome"], "stopped", "{cancelled:?}");

    let output = process
        .request(HostCommand::Invoke(serde_json::from_value(json!({
            "operationId":"op-out","operation":"local.output","effect":"read",
            "driverInstanceId":"i1","args":{"operationId":"op-slow"},
            "source":"human","dispatch":{"mode":"execute"},
        }))?))
        .await?;
    assert_eq!(output["done"], true, "{output:?}");
    assert_eq!(output["cancelled"], true, "{output:?}");

    // 不存在的作业要明确失败，不能返回空的成功结果。
    let error = process
        .request(HostCommand::Cancel(CancelTarget::Operation(
            "op-missing".to_owned(),
        )))
        .await?;
    assert_eq!(error["outcome"], "not_submitted");

    assert_eq!(process.request(HostCommand::Shutdown).await?["ok"], true);
    process.close().await?;
    Ok(())
}
