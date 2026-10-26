//! Bounded, in-memory host ledger for public operation status.

use std::collections::{HashMap, VecDeque};

use chrono::Utc;
use coterm_proto::runtime::operation::{OperationRecord, OperationStatus};
use thiserror::Error;
use tokio::sync::{watch, Mutex};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OperationHistoryError {
    #[error("operation already exists")]
    Duplicate,
    #[error("operation not found")]
    NotFound,
    #[error("invalid operation state transition")]
    InvalidTransition,
    #[error("operation history is full of active operations")]
    PendingCapacity,
}

#[derive(Default)]
struct State {
    buckets: HashMap<String, VecDeque<OperationRecord>>,
    locations: HashMap<String, String>,
    updates: HashMap<String, watch::Sender<OperationRecord>>,
}

/// Keeps the newest `capacity` records for each plugin instance (or plugin scope).
pub struct OperationHistory {
    capacity: usize,
    max_value_bytes: usize,
    state: Mutex<State>,
}

impl OperationHistory {
    pub fn new(capacity: usize, max_value_bytes: usize) -> Self {
        Self {
            capacity,
            max_value_bytes,
            state: Mutex::new(State::default()),
        }
    }

    pub async fn begin(&self, mut record: OperationRecord) -> Result<(), OperationHistoryError> {
        let mut state = self.state.lock().await;
        if record.status != OperationStatus::Accepted {
            return Err(OperationHistoryError::InvalidTransition);
        }
        if state.locations.contains_key(&record.operation_id) {
            return Err(OperationHistoryError::Duplicate);
        }

        if self.capacity == 0 {
            return Ok(());
        }

        record.input = redact_value(record.input);
        record.output = record.output.map(redact_value);
        record.error = record.error.map(redact_error);
        bound_value(&mut record.input, self.max_value_bytes);
        if let Some(output) = &mut record.output {
            bound_value(output, self.max_value_bytes);
        }

        let bucket = bucket_key(&record);
        let (updates, _) = watch::channel(record.clone());
        if state.buckets.get(&bucket).is_some_and(|records| {
            records.len() >= self.capacity && records.iter().all(|record| !terminal(record.status))
        }) {
            return Err(OperationHistoryError::PendingCapacity);
        }
        state.updates.insert(record.operation_id.clone(), updates);
        let operation_id = record.operation_id.clone();
        let records = state.buckets.entry(bucket.clone()).or_default();
        records.push_back(record);
        state.locations.insert(operation_id, bucket.clone());

        let mut evicted_ids = Vec::new();
        if let Some(records) = state.buckets.get_mut(&bucket) {
            while records.len() > self.capacity {
                let Some(index) = records.iter().position(|record| terminal(record.status)) else {
                    break;
                };
                if let Some(evicted) = records.remove(index) {
                    evicted_ids.push(evicted.operation_id);
                }
            }
        }
        for operation_id in evicted_ids {
            state.locations.remove(&operation_id);
            state.updates.remove(&operation_id);
        }

        Ok(())
    }

    /// Subscribe before dispatch: the final snapshot survives ledger eviction.
    pub async fn subscribe(
        &self,
        id: &str,
    ) -> Result<watch::Receiver<OperationRecord>, OperationHistoryError> {
        self.state
            .lock()
            .await
            .updates
            .get(id)
            .map(watch::Sender::subscribe)
            .ok_or(OperationHistoryError::NotFound)
    }

    pub async fn get(&self, operation_id: &str) -> Result<OperationRecord, OperationHistoryError> {
        let state = self.state.lock().await;
        let bucket = state
            .locations
            .get(operation_id)
            .ok_or(OperationHistoryError::NotFound)?;
        state
            .buckets
            .get(bucket)
            .and_then(|records| {
                records
                    .iter()
                    .find(|record| record.operation_id == operation_id)
            })
            .cloned()
            .ok_or(OperationHistoryError::NotFound)
    }

    pub async fn recent(
        &self,
        plugin_id: &str,
        plugin_instance_id: Option<&str>,
        limit: usize,
    ) -> Vec<OperationRecord> {
        let key = bucket_key_parts(plugin_id, plugin_instance_id);
        let state = self.state.lock().await;
        state
            .buckets
            .get(&key)
            .into_iter()
            .flat_map(|records| records.iter().rev())
            .take(limit.min(self.capacity))
            .cloned()
            .collect()
    }

    pub async fn active(&self, plugin_id: &str, instance_id: &str) -> Vec<OperationRecord> {
        let state = self.state.lock().await;
        state
            .buckets
            .values()
            .flat_map(|records| records.iter())
            .filter(|r| {
                r.plugin_id == plugin_id
                    && r.plugin_instance_id.as_deref() == Some(instance_id)
                    && !terminal(r.status)
            })
            .cloned()
            .collect()
    }

    pub async fn active_plugin(&self, plugin_id: &str) -> Vec<OperationRecord> {
        self.state
            .lock()
            .await
            .buckets
            .values()
            .flatten()
            .filter(|record| record.plugin_id == plugin_id && !terminal(record.status))
            .cloned()
            .collect()
    }

    pub async fn set_wait_expired(
        &self,
        operation_id: &str,
    ) -> Result<OperationRecord, OperationHistoryError> {
        let mut state = self.state.lock().await;
        let bucket = state
            .locations
            .get(operation_id)
            .ok_or(OperationHistoryError::NotFound)?
            .clone();
        let record = state
            .buckets
            .get_mut(&bucket)
            .and_then(|records| {
                records
                    .iter_mut()
                    .find(|record| record.operation_id == operation_id)
            })
            .ok_or(OperationHistoryError::NotFound)?;
        record.wait_expired = true;
        record.updated_at = Utc::now();
        let snapshot = record.clone();
        if let Some(updates) = state.updates.get(operation_id) {
            updates.send_replace(snapshot.clone());
        }
        Ok(snapshot)
    }

    pub async fn set_input(
        &self,
        operation_id: &str,
        input: serde_json::Value,
    ) -> Result<OperationRecord, OperationHistoryError> {
        let mut state = self.state.lock().await;
        let bucket = state
            .locations
            .get(operation_id)
            .ok_or(OperationHistoryError::NotFound)?
            .clone();
        let record = state
            .buckets
            .get_mut(&bucket)
            .and_then(|records| {
                records
                    .iter_mut()
                    .find(|record| record.operation_id == operation_id)
            })
            .ok_or(OperationHistoryError::NotFound)?;
        record.input = redact_value(input);
        bound_value(&mut record.input, self.max_value_bytes);
        record.updated_at = Utc::now();
        let snapshot = record.clone();
        if let Some(updates) = state.updates.get(operation_id) {
            updates.send_replace(snapshot.clone());
        }
        Ok(snapshot)
    }

    pub async fn transition(
        &self,
        operation_id: &str,
        status: OperationStatus,
        output: Option<serde_json::Value>,
        error: Option<coterm_proto::runtime::RuntimeError>,
    ) -> Result<OperationRecord, OperationHistoryError> {
        let mut state = self.state.lock().await;
        let bucket = state
            .locations
            .get(operation_id)
            .ok_or(OperationHistoryError::NotFound)?
            .clone();
        let record = state
            .buckets
            .get_mut(&bucket)
            .and_then(|records| {
                records
                    .iter_mut()
                    .find(|record| record.operation_id == operation_id)
            })
            .ok_or(OperationHistoryError::NotFound)?;
        if !can_transition(record.status, status) {
            return Err(OperationHistoryError::InvalidTransition);
        }
        record.status = status;
        record.output = output.map(redact_value);
        if let Some(output) = &mut record.output {
            bound_value(output, self.max_value_bytes);
        }
        record.error = error;
        record.error = record.error.take().map(redact_error);
        record.updated_at = Utc::now();
        let snapshot = record.clone();
        if let Some(updates) = state.updates.get(operation_id) {
            updates.send_replace(snapshot.clone());
        }
        Ok(snapshot)
    }
}

fn redact_value(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;

    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase();
                    let sensitive = [
                        "password",
                        "passwd",
                        "token",
                        "secret",
                        "credential",
                        "private_key",
                    ]
                    .iter()
                    .any(|needle| normalized.contains(needle));
                    (
                        key,
                        if sensitive {
                            Value::String("[REDACTED]".to_owned())
                        } else {
                            redact_value(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        other => other,
    }
}

fn bound_value(value: &mut serde_json::Value, max_bytes: usize) {
    if serde_json::to_vec(value).map_or(true, |encoded| encoded.len() > max_bytes) {
        *value = serde_json::json!({"_coterm_truncated": true});
    }
}

fn redact_error(
    mut error: coterm_proto::runtime::RuntimeError,
) -> coterm_proto::runtime::RuntimeError {
    error.details = error.details.map(redact_value);
    if error.message.len() > 1024 {
        error.message.truncate(1024);
    }
    if let Some(details) = &mut error.details {
        bound_value(details, 4096);
    }
    error
}

fn bucket_key(record: &OperationRecord) -> String {
    let key = bucket_key_parts(&record.plugin_id, record.plugin_instance_id.as_deref());
    if record.transient {
        format!("{key}\0transient")
    } else {
        key
    }
}

fn bucket_key_parts(plugin_id: &str, instance_id: Option<&str>) -> String {
    match instance_id {
        Some(instance_id) => format!("{plugin_id}\0instance\0{instance_id}"),
        None => format!("{plugin_id}\0plugin"),
    }
}

pub(crate) fn terminal(status: OperationStatus) -> bool {
    matches!(
        status,
        OperationStatus::Completed
            | OperationStatus::Failed
            | OperationStatus::Rejected
            | OperationStatus::Cancelled
            | OperationStatus::Unknown
    )
}

fn can_transition(from: OperationStatus, to: OperationStatus) -> bool {
    use OperationStatus::*;
    matches!(
        (from, to),
        (
            Accepted,
            AwaitingApproval | Dispatched | Running | Rejected | Cancelled | Failed | Unknown
        ) | (
            AwaitingApproval,
            Dispatched | Running | Rejected | Cancelled | Failed | Unknown
        ) | (
            Dispatched,
            Running | Completed | Failed | Cancelled | Unknown
        ) | (Running, Completed | Failed | Cancelled | Unknown)
    )
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use coterm_proto::runtime::operation::{
        OperationEffect, OperationRecord, OperationSource, OperationStatus,
    };

    use super::{OperationHistory, OperationHistoryError};

    fn record(id: &str, instance_id: Option<&str>) -> OperationRecord {
        let now = Utc::now();
        OperationRecord {
            operation_id: id.to_owned(),
            plugin_id: "example.plugin".to_owned(),
            plugin_instance_id: instance_id.map(str::to_owned),
            source: OperationSource::Human,
            effect: OperationEffect::Read,
            operation: "example.read".to_owned(),
            status: OperationStatus::Accepted,
            input: serde_json::json!({}),
            output: None,
            error: None,
            created_at: now,
            updated_at: now,
            proposal_id: None,
            wait_expired: false,
            transient: false,
        }
    }

    #[tokio::test]
    async fn keeps_only_the_newest_n_records_per_instance() {
        let history = OperationHistory::new(2, 4096);
        history
            .begin(record("op-1", Some("instance-a")))
            .await
            .unwrap();
        history
            .begin(record("op-2", Some("instance-a")))
            .await
            .unwrap();
        for id in ["op-1", "op-2"] {
            assert!(history
                .transition(id, OperationStatus::Dispatched, None, None)
                .await
                .is_ok());
            assert!(history
                .transition(id, OperationStatus::Completed, None, None)
                .await
                .is_ok());
        }
        history
            .begin(record("op-3", Some("instance-a")))
            .await
            .unwrap();
        history
            .begin(record("op-4", Some("instance-b")))
            .await
            .unwrap();

        let recent = history
            .recent("example.plugin", Some("instance-a"), 10)
            .await;
        assert_eq!(
            recent
                .iter()
                .map(|r| r.operation_id.as_str())
                .collect::<Vec<_>>(),
            ["op-3", "op-2"]
        );
        assert!(matches!(
            history.get("op-1").await,
            Err(OperationHistoryError::NotFound)
        ));
        assert_eq!(
            history
                .get("op-4")
                .await
                .unwrap()
                .plugin_instance_id
                .as_deref(),
            Some("instance-b")
        );
    }

    #[tokio::test]
    async fn transient_reads_do_not_displace_completed_commands(
    ) -> Result<(), OperationHistoryError> {
        let history = OperationHistory::new(2, 4096);
        history.begin(record("command", Some("one"))).await?;
        history
            .transition("command", OperationStatus::Dispatched, None, None)
            .await?;
        history
            .transition(
                "command",
                OperationStatus::Completed,
                Some(serde_json::json!({"exitCode":0})),
                None,
            )
            .await?;
        for n in 0..200 {
            let mut entry = record(&format!("read-{n}"), Some("one"));
            entry.transient = true;
            let id = entry.operation_id.clone();
            history.begin(entry).await?;
            history
                .transition(&id, OperationStatus::Dispatched, None, None)
                .await?;
            history
                .transition(&id, OperationStatus::Completed, None, None)
                .await?;
        }
        assert_eq!(
            history.get("command").await?.status,
            OperationStatus::Completed
        );
        assert_eq!(
            history
                .recent("example.plugin", Some("one"), 100)
                .await
                .len(),
            1
        );
        assert!(matches!(
            history.get("read-0").await,
            Err(OperationHistoryError::NotFound)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn waiting_caller_keeps_the_final_result_after_history_eviction(
    ) -> Result<(), OperationHistoryError> {
        let history = OperationHistory::new(1, 4096);
        history.begin(record("command", Some("one"))).await?;
        let updates = history.subscribe("command").await?;
        history
            .transition("command", OperationStatus::Dispatched, None, None)
            .await?;
        history
            .transition(
                "command",
                OperationStatus::Completed,
                Some(serde_json::json!("result")),
                None,
            )
            .await?;
        history.begin(record("next", Some("one"))).await?;
        assert!(matches!(
            history.get("command").await,
            Err(OperationHistoryError::NotFound)
        ));
        assert_eq!(updates.borrow().status, OperationStatus::Completed);
        assert_eq!(updates.borrow().output, Some(serde_json::json!("result")));
        Ok(())
    }

    #[tokio::test]
    async fn polling_does_not_evict_active_operations() -> Result<(), OperationHistoryError> {
        for status in [OperationStatus::AwaitingApproval, OperationStatus::Running] {
            let history = OperationHistory::new(2, 4096);
            history.begin(record("pending", Some("one"))).await?;
            history.transition("pending", status, None, None).await?;
            for index in 0..20 {
                let id = format!("read-{index}");
                history.begin(record(&id, Some("one"))).await?;
                history
                    .transition(&id, OperationStatus::Dispatched, None, None)
                    .await?;
                history
                    .transition(&id, OperationStatus::Completed, None, None)
                    .await?;
            }
            assert_eq!(history.get("pending").await?.status, status);
            history.begin(record("second", Some("one"))).await?;
            history
                .transition("second", OperationStatus::AwaitingApproval, None, None)
                .await?;
            assert_eq!(
                history.begin(record("third", Some("one"))).await,
                Err(OperationHistoryError::PendingCapacity)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn wait_expiry_does_not_cancel_operation() {
        let history = OperationHistory::new(4, 4096);
        history
            .begin(record("op-1", Some("instance-a")))
            .await
            .unwrap();

        let updated = history.set_wait_expired("op-1").await.unwrap();
        assert!(updated.wait_expired);
        assert_eq!(updated.status, OperationStatus::Accepted);
    }

    #[tokio::test]
    async fn terminal_status_cannot_be_replaced() {
        let history = OperationHistory::new(4, 4096);
        history
            .begin(record("op-1", Some("instance-a")))
            .await
            .unwrap();
        history
            .transition("op-1", OperationStatus::Running, None, None)
            .await
            .unwrap();
        history
            .transition(
                "op-1",
                OperationStatus::Completed,
                Some(serde_json::json!(1)),
                None,
            )
            .await
            .unwrap();

        assert!(matches!(
            history
                .transition("op-1", OperationStatus::Failed, None, None)
                .await,
            Err(OperationHistoryError::InvalidTransition)
        ));
    }

    #[tokio::test]
    async fn duplicate_operation_ids_are_rejected_while_retained() {
        let history = OperationHistory::new(2, 4096);
        history
            .begin(record("op-1", Some("instance-a")))
            .await
            .unwrap();
        assert_eq!(
            history.begin(record("op-1", Some("instance-b"))).await,
            Err(OperationHistoryError::Duplicate)
        );
    }

    #[tokio::test]
    async fn secret_fields_are_redacted_and_large_values_are_bounded() {
        let history = OperationHistory::new(2, 32);
        let mut entry = record("op-1", Some("instance-a"));
        entry.input = serde_json::json!({"password": "cleartext", "public": "value"});
        history.begin(entry).await.unwrap();

        let stored = history.get("op-1").await.unwrap();
        assert_eq!(stored.input, serde_json::json!({"_coterm_truncated": true}));

        let mut small = record("op-2", Some("instance-a"));
        small.input = serde_json::json!({"password": "cleartext"});
        history.begin(small).await.unwrap();
        let stored = history.get("op-2").await.unwrap();
        assert_eq!(stored.input, serde_json::json!({"password": "[REDACTED]"}));
    }
}
