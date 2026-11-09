use crate::{workspace_commands::access, AppState};
use coterm_core::approval_policy::{ApprovalMode, ApprovalPolicy, ApprovalRuleGroup};
use coterm_proto::runtime::RuntimeError;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalInstance {
    config_ref: String,
    label: String,
    driver_id: String,
    instance_id: Option<String>,
    rule_group_id: Option<String>,
    bypass: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalSnapshot {
    groups: Vec<ApprovalRuleGroup>,
    instances: Vec<ApprovalInstance>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveApprovalGroup {
    id: Option<String>,
    name: String,
    mode: ApprovalMode,
    patterns: Vec<String>,
}

/// Returns the user's group catalog and saved bindings, including offline entries.
/// Does not expose connection config; workspace read failures are returned unchanged.
#[tauri::command]
pub async fn approval_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<ApprovalSnapshot, RuntimeError> {
    let _guard = state.workspace_actions.lock().await;
    access(&state, |store| {
        let snapshot = store.snapshot();
        Ok(ApprovalSnapshot {
            groups: snapshot.approval_rule_groups,
            instances: snapshot
                .entries
                .into_iter()
                .map(|entry| ApprovalInstance {
                    config_ref: entry.config_ref,
                    label: entry.label,
                    driver_id: entry.driver_id,
                    instance_id: entry.instance_id,
                    rule_group_id: entry.approval_rule_group_id,
                    bypass: entry.bypass,
                })
                .collect(),
        })
    })
    .await
}

/// Saves a new (null id) or existing group, then atomically publishes its matcher
/// to all live bindings. Invalid rules/IDs or write failures change no active rules.
#[tauri::command]
pub async fn save_approval_group(
    group: SaveApprovalGroup,
    state: tauri::State<'_, AppState>,
) -> Result<ApprovalRuleGroup, RuntimeError> {
    let _guard = state.workspace_actions.lock().await;
    let policy = ApprovalPolicy {
        mode: group.mode,
        patterns: group.patterns,
    };
    let compiled = policy.compile()?;
    let (saved, ids) = access(&state, move |store| {
        let saved = store.save_approval_group(group.id, group.name, policy)?;
        let ids = store
            .snapshot()
            .entries
            .into_iter()
            .filter(|entry| entry.approval_rule_group_id.as_deref() == Some(&saved.id))
            .filter_map(|entry| entry.instance_id)
            .collect();
        Ok((saved, ids))
    })
    .await?;
    state
        .driver_runtime
        .apply_approval_policy(ids, compiled)
        .await;
    Ok(saved)
}

/// Assigns one existing group (or null for approval-required) to a saved configRef.
/// Offline targets are supported; missing targets/groups and save failures preserve
/// the old binding, matcher and bypass. No connection or operation is started.
#[tauri::command]
pub async fn assign_approval_group(
    config_ref: String,
    group_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), RuntimeError> {
    let _guard = state.workspace_actions.lock().await;
    let (ids, compiled) = access(&state, move |store| {
        let entry = store.find(&config_ref)?;
        let policy = match &group_id {
            None => ApprovalPolicy::default(),
            Some(id) => store
                .snapshot()
                .approval_rule_groups
                .iter()
                .find(|group| &group.id == id)
                .map(ApprovalRuleGroup::policy)
                .ok_or_else(|| crate::workspace::error("approval_group_not_found"))?,
        };
        let compiled = policy.compile()?;
        store.assign_approval_group(&config_ref, group_id)?;
        Ok((entry.instance_id.into_iter().collect(), compiled))
    })
    .await?;
    state
        .driver_runtime
        .apply_approval_policy(ids, compiled)
        .await;
    Ok(())
}

/// Deletes an unbound group. Unknown IDs, saved/live references and persistence
/// failures are errors; no instance binding or running approval state is changed.
#[tauri::command]
pub async fn delete_approval_group(
    group_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), RuntimeError> {
    let _guard = state.workspace_actions.lock().await;
    access(&state, move |store| store.delete_approval_group(&group_id)).await
}
