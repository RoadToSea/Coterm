//! The host adapter and a real plugin process must preserve the shared contract.
use anyhow::Result;
use coterm_core::{plugin_process::JsonRpcPluginProcess, plugin_runtime::PluginConnection};
use coterm_proto::runtime::{
    lifecycle::{HelloResult, RuntimeHello},
    message::HostCommand,
    ErrorCategory,
};
use serde_json::json;

#[tokio::test]
async fn structured_errors_and_rejected_oversize_requests_keep_process_usable() -> Result<()> {
    let process = JsonRpcPluginProcess::spawn(
        std::path::Path::new(env!("CARGO_BIN_EXE_coterm-plugin-local")),
        &[],
    )
    .await?;
    let hello: RuntimeHello = serde_json::from_value(json!({
        "interfaceMajor":1,"interfaceMinor":0,
        "requiredFeatures":["instances","backend-calls","operation-catalog"],"optionalFeatures":[],
        "limits":{"maxControlBytes":1024,"maxDataChunkBytes":65536,"initialWindow":{"chunks":16,"bytes":1048576}}
    }))?;
    let accepted: HelloResult =
        serde_json::from_value(process.request(HostCommand::Hello(hello.clone())).await?)?;
    hello.validate_result(&accepted)?;
    process.set_control_limit(accepted.limits.max_control_bytes);
    let error = process
        .request(HostCommand::DestroyInstance {
            plugin_instance_id: "missing".to_owned(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Domain);
    assert_eq!(error.code, "local_domain_error");
    let error = process
        .request(HostCommand::DestroyInstance {
            plugin_instance_id: "x".repeat(4096),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "plugin_control_too_large");
    assert_eq!(process.request(HostCommand::Shutdown).await?["ok"], true);
    process.close().await?;
    Ok(())
}
