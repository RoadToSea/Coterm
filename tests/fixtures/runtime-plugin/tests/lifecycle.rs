use coterm_core::{
    plugin_manager::ProcessPluginRuntime,
    plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
};
use coterm_proto::runtime::{
    lifecycle::{InstanceInfo, InstanceStatus, RuntimeLimits},
    operation::{OperationSource, OperationStatus},
    stream::FlowWindow,
};
use serde_json::json;
use std::{path::PathBuf, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ID: &str = "coterm-runtime-fixture";
fn runtime() -> ProcessPluginRuntime {
    ProcessPluginRuntime::new(
        format!(
            "{}-{}",
            std::env::consts::OS,
            if cfg!(target_arch = "x86_64") {
                "x64"
            } else {
                "arm64"
            }
        ),
        RuntimeLimits {
            max_control_bytes: 1024 * 1024,
            max_data_chunk_bytes: 65536,
            initial_window: FlowWindow {
                chunks: 16,
                bytes: 1024 * 1024,
            },
        },
    )
}
fn package() -> Result<PathBuf> {
    let root = std::env::temp_dir().join(format!("coterm-lifecycle-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root)?;
    let binary = std::path::Path::new(env!("CARGO_BIN_EXE_coterm-runtime-fixture"));
    let filename = binary.file_name().ok_or("binary filename")?;
    std::fs::copy(binary, root.join(filename))?;
    let platform = format!(
        "{}-{}",
        std::env::consts::OS,
        if cfg!(target_arch = "x86_64") {
            "x64"
        } else {
            "arm64"
        }
    );
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&json!({
            "manifestVersion":1,"id":ID,"name":"Fixture","version":"0.1.0",
            "runtime":{"interfaceMajor":1,"requires":["instances","backend-calls","operation-catalog","plugin-events","async-operations","secret-delivery","deferred-interactions"]},
            "backend":{platform:filename.to_string_lossy()},"mcp":{"provider":"backend"},"instances":{"type":"test","multiple":true},"capabilities":["secret-input"]
        }))?,
    )?;
    Ok(root)
}
async fn submit(
    runtime: &ProcessPluginRuntime,
    instance: &InstanceInfo,
    operation: &str,
    source: OperationSource,
    plugin_scope: bool,
) -> Result<coterm_proto::runtime::operation::OperationReceipt> {
    Ok(runtime
        .submit(
            CallerContext {
                source,
                session_id: instance.session_id.clone(),
                plugin_id: Some(ID.into()),
            },
            SubmitOperation {
                plugin_id: ID.into(),
                plugin_instance_id: if plugin_scope {
                    None
                } else {
                    Some(instance.plugin_instance_id.clone())
                },
                operation: operation.into(),
                args: json!({}),
                wait_timeout_ms: Some(100),
            },
        )
        .await?)
}

#[tokio::test]
async fn crash_converges_all_scopes_clears_attention_and_allows_restart_without_replay(
) -> Result<()> {
    let root = package()?;
    let runtime = runtime();
    runtime.load(&root).await?;
    runtime.start(ID).await?;
    let first = runtime
        .create_instance(ID, json!({"label":"First"}))
        .await?;
    let second = runtime
        .create_instance(ID, json!({"label":"Second"}))
        .await?;
    let pending = submit(
        &runtime,
        &first,
        "fixture.commit",
        OperationSource::Agent,
        false,
    )
    .await?;
    assert_eq!(pending.status, OperationStatus::AwaitingApproval);
    let running = submit(
        &runtime,
        &second,
        "fixture.hang",
        OperationSource::Human,
        false,
    )
    .await?;
    let plugin_running = submit(
        &runtime,
        &first,
        "fixture.plugin_hang",
        OperationSource::Human,
        true,
    )
    .await?;
    let secret = submit(
        &runtime,
        &first,
        "fixture.secret",
        OperationSource::Human,
        false,
    )
    .await?;
    assert_eq!(runtime.attention_snapshot().await.interactions.len(), 1);
    let _ = submit(
        &runtime,
        &second,
        "fixture.crash",
        OperationSource::Human,
        false,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime.is_running(ID).await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let instances = runtime.list_instances(None).await?;
    assert_eq!(instances.len(), 2);
    assert!(instances
        .iter()
        .all(|instance| instance.status == InstanceStatus::Failed));
    assert!(runtime.attention_snapshot().await.approvals.is_empty());
    assert!(runtime.attention_snapshot().await.interactions.is_empty());
    for (receipt, instance) in [
        (running, &second),
        (plugin_running, &first),
        (secret, &first),
    ] {
        let caller = CallerContext {
            source: OperationSource::Human,
            session_id: instance.session_id.clone(),
            plugin_id: Some(ID.into()),
        };
        assert_eq!(
            runtime.get(caller, &receipt.operation_id).await?.status,
            OperationStatus::Unknown
        );
    }
    assert_eq!(
        runtime
            .get(
                CallerContext {
                    source: OperationSource::Human,
                    session_id: first.session_id.clone(),
                    plugin_id: Some(ID.into())
                },
                &pending.operation_id
            )
            .await?
            .status,
        OperationStatus::Cancelled
    );
    runtime.destroy_instance(&first.plugin_instance_id).await?;
    runtime.start(ID).await?;
    let restored = runtime
        .create_instance(ID, json!({"label":"First"}))
        .await?;
    assert_ne!(first.plugin_instance_id, restored.plugin_instance_id);
    let info = submit(
        &runtime,
        &restored,
        "fixture.info",
        OperationSource::Human,
        false,
    )
    .await?;
    assert_eq!(info.output.ok_or("output")?["writes"], 0);
    // Deleting a failed instance must not send its obsolete ID to the new process.
    runtime.destroy_instance(&second.plugin_instance_id).await?;
    runtime.shutdown().await;
    assert!(!runtime.is_running(ID).await);
    assert!(runtime.list_instances(None).await?.is_empty());
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn shutdown_reaps_a_child_even_when_control_holds_the_routing_lock() -> Result<()> {
    let root = package()?;
    let runtime = runtime();
    runtime.load(&root).await?;
    runtime.start(ID).await?;
    let instance = runtime.create_instance(ID, json!({})).await?;
    let receipt = submit(
        &runtime,
        &instance,
        "fixture.block",
        OperationSource::Human,
        false,
    )
    .await?;
    assert!(receipt.wait_expired);
    tokio::time::timeout(Duration::from_secs(3), runtime.shutdown()).await?;
    assert!(!runtime.is_running(ID).await);
    assert!(runtime.list_instances(None).await?.is_empty());
    assert_eq!(
        runtime
            .start(ID)
            .await
            .expect_err("shutdown rejects restart")
            .code,
        "runtime_shutting_down"
    );
    std::fs::remove_dir_all(root)?;
    Ok(())
}
