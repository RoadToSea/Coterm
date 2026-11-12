use coterm_core::model::{
    lifecycle::{InstanceInfo, InstanceStatus, RuntimeLimits},
    operation::{OperationSource, OperationStatus},
};
use coterm_core::{
    runtime::Runtime,
    runtime::{CallerContext, SubmitOperation},
};
use serde_json::json;
use std::time::Duration;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ID: &str = "coterm-runtime-fixture";

fn runtime() -> Runtime {
    Runtime::new(RuntimeLimits {
        max_control_bytes: 1024 * 1024,
    })
}
async fn register(runtime: &Runtime) -> Result<()> {
    let manifest = serde_json::from_value(json!({
        "manifestVersion":1,"id":ID,"name":"Fixture","version":"0.1.0",
        "runtime":{"interfaceMajor":2,"requires":["instances","backend-calls","operation-catalog","driver-events","async-operations","secret-delivery","deferred-interactions"]},
        "mcp":{"provider":"backend"},"instances":{"type":"test","multiple":true},"capabilities":["secret-input"]
    }))?;
    runtime
        .register::<coterm_runtime_fixture::Backend>(manifest, None)
        .await?;
    Ok(())
}
async fn submit(
    runtime: &Runtime,
    instance: &InstanceInfo,
    operation: &str,
    source: OperationSource,
    driver_scope: bool,
) -> Result<coterm_core::model::operation::OperationReceipt> {
    Ok(runtime
        .submit(
            CallerContext {
                source,
                session_id: instance.session_id.clone(),
                driver_id: Some(ID.into()),
            },
            SubmitOperation {
                driver_id: ID.into(),
                driver_instance_id: if driver_scope {
                    None
                } else {
                    Some(instance.driver_instance_id.clone())
                },
                operation: operation.into(),
                args: json!({}),
                wait_timeout_ms: Some(100),
            },
        )
        .await?)
}

#[tokio::test]
async fn explicit_instance_names_are_preserved_and_duplicates_are_rejected() -> Result<()> {
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(ID).await?;

    let first = runtime
        .create_instance_with_name(
            ID,
            json!({"name":"Driver name"}),
            Some("  Lab server  ".into()),
        )
        .await?;
    assert_eq!(first.name, "Lab server");
    let second = runtime
        .create_instance_with_name(ID, json!({}), Some("Lab server".into()))
        .await;
    assert_eq!(
        second.err().map(|error| error.code),
        Some("instance_name_exists".into())
    );
    assert_eq!(runtime.list_instances(None).await?.len(), 1);

    let invalid = runtime
        .create_instance_with_name(ID, json!({}), Some("名".repeat(86)))
        .await;
    assert_eq!(
        invalid.err().map(|error| error.code),
        Some("instance_name_invalid".into())
    );
    assert_eq!(runtime.list_instances(None).await?.len(), 1);
    runtime.destroy_instance(&first.driver_instance_id).await?;
    let recreated = runtime
        .create_instance_with_name(ID, json!({}), Some("Lab server".into()))
        .await?;
    assert_eq!(recreated.name, "Lab server");
    let serialized = serde_json::to_value(&recreated)?;
    assert_eq!(serialized["name"], "Lab server");
    assert!(serialized.get("label").is_none());
    Ok(())
}

#[tokio::test]
async fn command_rules_control_real_agent_dispatch_and_preserve_pending_approvals() -> Result<()> {
    use coterm_core::approval_policy::{ApprovalMode, ApprovalPolicy};
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(ID).await?;
    let instance = runtime.create_instance(ID, json!({})).await?;
    let allow = ApprovalPolicy {
        mode: ApprovalMode::AllowMatches,
        patterns: vec![r"^ls$".into(), r"^cd /tmp$".into()],
    };
    runtime
        .set_approval_policy(&instance.driver_instance_id, allow.clone())
        .await?;
    for (mode, commands, expected) in [
        (
            ApprovalMode::AllowMatches,
            vec!["ls", "cd /tmp"],
            OperationStatus::Completed,
        ),
        (
            ApprovalMode::AllowMatches,
            vec!["ls", "rm x"],
            OperationStatus::AwaitingApproval,
        ),
        (
            ApprovalMode::AllowMatches,
            vec!["ls; rm x"],
            OperationStatus::AwaitingApproval,
        ),
        (
            ApprovalMode::ApproveMatches,
            vec!["rm x"],
            OperationStatus::Completed,
        ),
        (
            ApprovalMode::ApproveMatches,
            vec!["ls", "rm x"],
            OperationStatus::AwaitingApproval,
        ),
    ] {
        runtime
            .set_approval_policy(
                &instance.driver_instance_id,
                ApprovalPolicy {
                    mode,
                    ..allow.clone()
                },
            )
            .await?;
        let receipt = runtime
            .submit_until_approval(
                CallerContext {
                    source: OperationSource::Agent,
                    session_id: instance.session_id.clone(),
                    driver_id: Some(ID.into()),
                },
                SubmitOperation {
                    driver_id: ID.into(),
                    driver_instance_id: Some(instance.driver_instance_id.clone()),
                    operation: "fixture.commit".into(),
                    args: json!({"commands":commands}),
                    wait_timeout_ms: Some(1000),
                },
            )
            .await?;
        assert_eq!(receipt.status, expected);
    }
    // An invalid replacement cannot weaken the last valid configuration.
    let before = runtime
        .approval_policy(&instance.driver_instance_id)
        .await?;
    assert!(runtime
        .set_approval_policy(
            &instance.driver_instance_id,
            ApprovalPolicy {
                mode: ApprovalMode::AllowMatches,
                patterns: vec!["[".into()]
            }
        )
        .await
        .is_err());
    assert_eq!(
        runtime
            .approval_policy(&instance.driver_instance_id)
            .await?,
        before
    );
    assert_eq!(runtime.attention_snapshot().await.approvals.len(), 3);
    runtime.stop(ID).await?;
    Ok(())
}

#[tokio::test]
async fn task_panic_converges_all_scopes_clears_attention_and_allows_restart_without_replay(
) -> Result<()> {
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(ID).await?;
    let first = runtime.create_instance(ID, json!({"name":"First"})).await?;
    let second = runtime
        .create_instance(ID, json!({"name":"Second"}))
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
    let driver_running = submit(
        &runtime,
        &first,
        "fixture.driver_hang",
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
        (driver_running, &first),
        (secret, &first),
    ] {
        let caller = CallerContext {
            source: OperationSource::Human,
            session_id: instance.session_id.clone(),
            driver_id: Some(ID.into()),
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
                    driver_id: Some(ID.into())
                },
                &pending.operation_id
            )
            .await?
            .status,
        OperationStatus::Cancelled
    );
    runtime.destroy_instance(&first.driver_instance_id).await?;
    runtime.start(ID).await?;
    let restored = runtime.create_instance(ID, json!({"name":"First"})).await?;
    assert_ne!(first.driver_instance_id, restored.driver_instance_id);
    let info = submit(
        &runtime,
        &restored,
        "fixture.info",
        OperationSource::Human,
        false,
    )
    .await?;
    assert_eq!(info.output.ok_or("output")?["writes"], 0);
    // Deleting a failed instance must not send its obsolete ID to the new backend.
    runtime.destroy_instance(&second.driver_instance_id).await?;
    runtime.shutdown().await;
    assert!(!runtime.is_running(ID).await);
    assert!(runtime.list_instances(None).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn early_completion_and_duplicate_approvals_preserve_operation_order() -> Result<()> {
    use coterm_core::model::operation::ApproveProposal;
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(ID).await?;
    let instance = runtime.create_instance(ID, json!({})).await?;
    let caller = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(ID.to_owned()),
    };
    let completed = runtime
        .submit(
            caller.clone(),
            SubmitOperation {
                driver_id: ID.to_owned(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "fixture.early_completion".to_owned(),
                args: json!({}),
                wait_timeout_ms: Some(2000),
            },
        )
        .await?;
    assert_eq!(completed.status, OperationStatus::Completed);
    assert_eq!(
        completed.output.ok_or("missing completion")?["completedBeforeAck"],
        true
    );
    let pending = submit(
        &runtime,
        &instance,
        "fixture.commit",
        OperationSource::Agent,
        false,
    )
    .await?;
    let request = ApproveProposal {
        operation_id: pending.operation_id,
        proposal_id: pending.proposal_id.ok_or("missing proposal")?,
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        args: json!({}),
    };
    let (first, second) = tokio::join!(
        runtime.approve(caller.clone(), request.clone()),
        runtime.approve(caller, request)
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let info = submit(
        &runtime,
        &instance,
        "fixture.info",
        OperationSource::Human,
        false,
    )
    .await?;
    assert_eq!(info.output.ok_or("missing info")?["writes"], 1);
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn blocked_control_keeps_runtime_queries_responsive() -> Result<()> {
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(ID).await?;
    let instance = runtime.create_instance(ID, json!({})).await?;
    let blocked = submit(
        &runtime,
        &instance,
        "fixture.block",
        OperationSource::Human,
        false,
    )
    .await?;
    assert_eq!(blocked.status, OperationStatus::Dispatched);
    let queried =
        tokio::time::timeout(Duration::from_millis(250), runtime.list_instances(None)).await;
    tokio::time::timeout(Duration::from_secs(3), runtime.shutdown()).await?;
    assert_eq!(
        queried
            .map_err(|_| "Blocked control held the runtime state lock")??
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn shutdown_drops_backend_even_when_control_holds_the_routing_lock() -> Result<()> {
    let runtime = runtime();
    register(&runtime).await?;
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
    Ok(())
}

#[tokio::test]
async fn instance_approvals_isolate_rules_bypass_and_lifetime() -> Result<()> {
    use coterm_core::approval_policy::{ApprovalMode, ApprovalPolicy};
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(ID).await?;
    let first = runtime.create_instance(ID, json!({})).await?;
    let second = runtime.create_instance(ID, json!({})).await?;
    let caller = |instance: &InstanceInfo| CallerContext {
        source: OperationSource::Agent,
        session_id: instance.session_id.clone(),
        driver_id: Some(ID.into()),
    };
    let request = |instance: &InstanceInfo, command: &str| SubmitOperation {
        driver_id: ID.into(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: "fixture.commit".into(),
        args: json!({"commands":[command]}),
        wait_timeout_ms: Some(1000),
    };
    runtime
        .set_approval_policy(
            &first.driver_instance_id,
            ApprovalPolicy {
                mode: ApprovalMode::AllowMatches,
                patterns: vec!["^ls$".into()],
            },
        )
        .await?;
    assert_eq!(
        runtime
            .submit_until_approval(caller(&first), request(&first, "ls"))
            .await?
            .status,
        OperationStatus::Completed
    );
    let pending = runtime
        .submit_until_approval(caller(&second), request(&second, "ls"))
        .await?;
    assert_eq!(pending.status, OperationStatus::AwaitingApproval);
    runtime.set_bypass(&second.driver_instance_id, true).await?;
    assert!(!runtime.bypass_enabled(&first.driver_instance_id).await?);
    assert!(runtime.bypass_enabled(&second.driver_instance_id).await?);
    assert_eq!(
        runtime
            .get(caller(&second), &pending.operation_id)
            .await?
            .status,
        OperationStatus::AwaitingApproval
    );
    assert_eq!(
        runtime
            .submit_until_approval(caller(&first), request(&first, "pwd"))
            .await?
            .status,
        OperationStatus::AwaitingApproval
    );
    assert_eq!(
        runtime
            .submit_until_approval(caller(&second), request(&second, "pwd"))
            .await?
            .status,
        OperationStatus::Completed
    );
    assert!(runtime
        .submit_until_approval(caller(&first), request(&second, "ls"))
        .await
        .is_err());
    assert!(runtime.set_bypass("missing", true).await.is_err());
    assert!(runtime.approval_policy("missing").await.is_err());
    runtime.destroy_instance(&second.driver_instance_id).await?;
    assert!(runtime
        .bypass_enabled(&second.driver_instance_id)
        .await
        .is_err());
    let third = runtime.create_instance(ID, json!({})).await?;
    assert!(!runtime.bypass_enabled(&third.driver_instance_id).await?);
    assert_eq!(
        runtime
            .submit_until_approval(caller(&third), request(&third, "ls"))
            .await?
            .status,
        OperationStatus::AwaitingApproval
    );
    runtime.stop(ID).await?;
    assert!(runtime
        .approval_policy(&first.driver_instance_id)
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn shared_group_publication_updates_new_submissions_and_preserves_bypass_and_proposals(
) -> Result<()> {
    use coterm_core::approval_policy::ApprovalPolicy;
    let runtime = runtime();
    register(&runtime).await?;
    runtime.start(ID).await?;
    let one = runtime.create_instance(ID, json!({})).await?;
    let two = runtime.create_instance(ID, json!({})).await?;
    let caller = |instance: &InstanceInfo| CallerContext {
        source: OperationSource::Agent,
        session_id: instance.session_id.clone(),
        driver_id: Some(ID.into()),
    };
    let request = |instance: &InstanceInfo| SubmitOperation {
        driver_id: ID.into(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: "fixture.commit".into(),
        args: json!({"commands":["pwd"]}),
        wait_timeout_ms: Some(1000),
    };
    let pending = runtime
        .submit_until_approval(caller(&one), request(&one))
        .await?;
    assert_eq!(pending.status, OperationStatus::AwaitingApproval);
    runtime.set_bypass(&two.driver_instance_id, true).await?;
    let allow = ApprovalPolicy {
        patterns: vec!["^pwd$".into()],
        ..ApprovalPolicy::default()
    };
    runtime
        .apply_approval_policy(
            vec![
                one.driver_instance_id.clone(),
                two.driver_instance_id.clone(),
                "already-stopped".into(),
            ],
            allow.compile()?,
        )
        .await;
    assert_eq!(
        runtime.approval_policy(&one.driver_instance_id).await?,
        allow
    );
    assert_eq!(
        runtime.approval_policy(&two.driver_instance_id).await?,
        allow
    );
    assert!(runtime.bypass_enabled(&two.driver_instance_id).await?);
    assert_eq!(
        runtime
            .get(caller(&one), &pending.operation_id)
            .await?
            .status,
        OperationStatus::AwaitingApproval
    );
    assert_eq!(
        runtime
            .submit_until_approval(caller(&one), request(&one))
            .await?
            .status,
        OperationStatus::Completed
    );
    runtime
        .apply_approval_policy(
            vec![
                one.driver_instance_id.clone(),
                two.driver_instance_id.clone(),
            ],
            ApprovalPolicy::default().compile()?,
        )
        .await;
    assert_eq!(
        runtime
            .submit_until_approval(caller(&one), request(&one))
            .await?
            .status,
        OperationStatus::AwaitingApproval
    );
    assert_eq!(
        runtime
            .submit_until_approval(caller(&two), request(&two))
            .await?
            .status,
        OperationStatus::Completed
    );
    runtime.stop(ID).await?;
    Ok(())
}

#[tokio::test]
async fn backends_share_the_host_pid_but_not_state_and_registration_is_explicit() -> Result<()> {
    let first = runtime();
    let second = runtime();
    register(&first).await?;
    register(&second).await?;
    assert!(register(&first).await.is_err());
    assert_eq!(first.loaded_manifests().await.len(), 1);
    assert_eq!(
        first.start("missing").await.unwrap_err().code,
        "driver_not_loaded"
    );
    first.start(ID).await?;
    second.start(ID).await?;
    let one = first.create_instance(ID, json!({})).await?;
    let two = second.create_instance(ID, json!({})).await?;
    submit(
        &first,
        &one,
        "fixture.commit",
        OperationSource::Human,
        false,
    )
    .await?;
    let info = submit(&first, &one, "fixture.info", OperationSource::Human, false)
        .await?
        .output
        .ok_or("info")?;
    assert_eq!(info["pid"], std::process::id());
    assert_eq!(info["writes"], 1);
    let info = submit(&second, &two, "fixture.info", OperationSource::Human, false)
        .await?
        .output
        .ok_or("info")?;
    assert_eq!(info["pid"], std::process::id());
    assert_eq!(info["writes"], 0);
    first.shutdown().await;
    assert!(second.is_running(ID).await);
    assert!(
        submit(&second, &two, "fixture.info", OperationSource::Human, false)
            .await
            .is_ok()
    );
    assert!(register(&first).await.is_err());
    second.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn registration_rejects_invalid_metadata_without_mutating_catalog() -> Result<()> {
    let runtime = runtime();
    let valid = serde_json::json!({
        "manifestVersion":1,"id":ID,"name":"Fixture","version":"0.1.0",
        "runtime":{"interfaceMajor":2,"requires":["instances"]},
        "mcp":{"provider":"backend"},"instances":{"type":"test","multiple":true},"capabilities":[]
    });
    for (pointer, value, expected) in [
        (
            "/id",
            json!("invalid/id"),
            "driver_manifest_identity_invalid",
        ),
        (
            "/manifestVersion",
            json!(99),
            "driver_manifest_version_unsupported",
        ),
        (
            "/runtime/interfaceMajor",
            json!(99),
            "driver_runtime_version_unsupported",
        ),
        (
            "/runtime/requires",
            json!(["unknown-feature"]),
            "driver_required_feature_unsupported",
        ),
        (
            "/instances/multiple",
            json!(false),
            "driver_instance_declaration_invalid",
        ),
        (
            "/frontend",
            json!({"entry":"frontend/index.js"}),
            "driver_frontend_invalid",
        ),
    ] {
        let mut manifest = valid.clone();
        if pointer == "/frontend" {
            manifest["frontend"] = value;
        } else {
            *manifest.pointer_mut(pointer).ok_or("pointer")? = value;
        }
        let error = runtime
            .register::<coterm_runtime_fixture::Backend>(serde_json::from_value(manifest)?, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(runtime.loaded_manifests().await.is_empty());
    }
    Ok(())
}

#[derive(Default)]
struct OtherBackend(coterm_runtime_fixture::Backend);

impl coterm_core::driver::DriverBackend for OtherBackend {
    fn definition() -> coterm_core::driver::DriverDefinition {
        let mut definition =
            <coterm_runtime_fixture::Backend as coterm_core::driver::DriverBackend>::definition();
        definition.driver_id = "other-fixture".into();
        definition
    }
    async fn request(
        &mut self,
        command: coterm_core::driver::HostCommand,
    ) -> std::result::Result<serde_json::Value, coterm_core::model::RuntimeError> {
        coterm_core::driver::DriverBackend::request(&mut self.0, command).await
    }
    fn take_notifications(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<coterm_core::driver::DriverNotification>> {
        coterm_core::driver::DriverBackend::take_notifications(&mut self.0)
    }
}

#[tokio::test]
async fn a_panicking_driver_does_not_stop_other_drivers_in_the_same_runtime() -> Result<()> {
    let runtime = runtime();
    register(&runtime).await?;
    let mut other = runtime.loaded_manifests().await.remove(0);
    other.id = "other-fixture".into();
    runtime.register::<OtherBackend>(other, None).await?;
    runtime.start(ID).await?;
    runtime.start("other-fixture").await?;
    let first = runtime.create_instance(ID, json!({})).await?;
    let second = runtime.create_instance("other-fixture", json!({})).await?;
    let _ = submit(
        &runtime,
        &first,
        "fixture.crash",
        OperationSource::Human,
        false,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.is_running(ID).await {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert!(runtime.is_running("other-fixture").await);
    let result = runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: second.session_id.clone(),
                driver_id: Some("other-fixture".into()),
            },
            SubmitOperation {
                driver_id: "other-fixture".into(),
                driver_instance_id: Some(second.driver_instance_id),
                operation: "fixture.info".into(),
                args: json!({}),
                wait_timeout_ms: Some(1000),
            },
        )
        .await?;
    assert_eq!(result.status, OperationStatus::Completed);
    assert_eq!(result.output.ok_or("info")?["pid"], std::process::id());
    runtime.shutdown().await;
    Ok(())
}
