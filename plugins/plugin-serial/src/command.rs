//! Text / HEX lists define send order, not device completion or protocol semantics.
use crate::payload::{decode_hex, encode_text};
use anyhow::{anyhow, bail, Result};
use coterm_proto::runtime::{message::PluginNotification, ErrorCategory, RuntimeError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Write,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    time::{sleep, timeout},
};

pub const MAX_COMMANDS: usize = 64;
pub const MAX_COMMAND_BYTES: usize = 4096;
pub const MAX_BATCH_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandArgs {
    commands: Vec<String>,
    #[serde(default = "default_ending")]
    line_ending: String,
    #[serde(default = "default_interval")]
    interval_ms: u64,
}

fn default_ending() -> String {
    "none".into()
}
fn default_interval() -> u64 {
    100
}

pub struct CommandList {
    pub payloads: Vec<Vec<u8>>,
    pub interval: Duration,
}

impl CommandList {
    pub fn parse(args: &Value) -> Result<Self> {
        let args: CommandArgs = serde_json::from_value(args.clone())
            .map_err(|_| anyhow!("Invalid serial command list parameters"))?;
        Self::from_entries(args.commands, args.interval_ms, |entry| {
            encode_text(entry, &args.line_ending)
        })
    }

    pub fn parse_hex(args: &Value) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct HexListArgs {
            commands: Vec<String>,
            #[serde(default = "default_interval")]
            interval_ms: u64,
        }
        let args: HexListArgs = serde_json::from_value(args.clone())
            .map_err(|_| anyhow!("Invalid serial HEX list parameters"))?;
        Self::from_entries(args.commands, args.interval_ms, decode_hex)
    }

    fn from_entries(
        commands: Vec<String>,
        interval_ms: u64,
        encode: impl Fn(&str) -> Result<Vec<u8>>,
    ) -> Result<Self> {
        if commands.is_empty() || commands.len() > MAX_COMMANDS {
            bail!("Serial commands must contain 1 to 64 entries");
        }
        if interval_ms > 1000 {
            bail!("Serial command interval must be between 0 and 1000 ms");
        }
        let mut bytes = 0;
        let mut payloads = Vec::with_capacity(commands.len());
        for command in commands {
            let payload = encode(&command)?;
            if command.is_empty() || payload.len() > MAX_COMMAND_BYTES {
                bail!("Each serial entry must contain 1 to 4096 bytes including optional line endings");
            }
            bytes += payload.len();
            if bytes > MAX_BATCH_BYTES {
                bail!("Serial command list must not exceed 16384 bytes including line endings");
            }
            payloads.push(payload);
        }
        Ok(Self {
            payloads,
            interval: Duration::from_millis(interval_ms),
        })
    }
}

pub fn properties() -> Value {
    json!({
        "commands":{"type":"array","minItems":1,"maxItems":64,
            "items":{"type":"string","minLength":1,"maxLength":4096}},
        "lineEnding":{"type":"string","enum":["none","cr","lf","crlf"],"default":"none"},
        "intervalMs":{"type":"integer","minimum":0,"maximum":1000,"default":100}
    })
}

pub fn hex_properties() -> Value {
    json!({
        "commands":{"type":"array","minItems":1,"maxItems":64,
            "items":{"type":"string","minLength":1,"maxLength":12290}},
        "intervalMs":{"type":"integer","minimum":0,"maximum":1000,"default":100}
    })
}

#[derive(Clone, Default)]
pub struct Progress {
    pub started: bool,
    pub sent: usize,
    pub bytes: usize,
    pub finished: bool,
    pub cancelled: bool,
}

pub struct CommandHandle {
    pub cancel: watch::Sender<bool>,
    pub progress: watch::Receiver<Progress>,
}

pub struct CommandJob {
    pub operation_id: String,
    pub mcp: bool,
    pub list: CommandList,
    cancel: watch::Receiver<bool>,
    progress: watch::Sender<Progress>,
}

impl CommandJob {
    pub fn new(operation_id: String, mcp: bool, list: CommandList) -> (Self, CommandHandle) {
        let (cancel_tx, cancel) = watch::channel(false);
        let (progress, progress_rx) = watch::channel(Progress::default());
        (
            Self {
                operation_id,
                mcp,
                list,
                cancel,
                progress,
            },
            CommandHandle {
                cancel: cancel_tx,
                progress: progress_rx,
            },
        )
    }

    fn summary(&self, state: &Progress) -> Value {
        json!({"operationId":self.operation_id,"total":self.list.payloads.len(),
            "sent":state.sent,"bytes":state.bytes,"remaining":self.list.payloads.len()-state.sent,
            "finished":state.finished,"cancelled":state.cancelled,"deviceCompletion":"unknown"})
    }

    fn publish(
        &self,
        state: &Progress,
        events: &Option<(String, mpsc::Sender<PluginNotification>)>,
        connection_id: &str,
        tx_bytes: u64,
    ) {
        self.progress.send_replace(state.clone());
        if let Some((instance, sender)) = events {
            let mut payload = self.summary(state);
            payload["connectionId"] = json!(connection_id);
            payload["txBytes"] = json!(tx_bytes);
            let _ = sender.try_send(PluginNotification::Event {
                plugin_instance_id: instance.clone(),
                name: "serial.command".into(),
                payload,
            });
        }
    }

    /// 关闭时让排队但尚未发送的操作得到终态，不静默丢失 DeferredOperation。
    pub async fn fail(self, events: &Option<(String, mpsc::Sender<PluginNotification>)>) {
        let mut state = self.progress.borrow().clone();
        state.finished = true;
        if *self.cancel.borrow() && !state.started {
            state.cancelled = true;
            self.progress.send_replace(state);
            return;
        }
        self.progress.send_replace(state.clone());
        if let Some((_, sender)) = events {
            let _ = sender.send(PluginNotification::OperationFailed {
                operation_id: self.operation_id.clone(),
                error: RuntimeError { category: ErrorCategory::Transport, code: "serial_command_interrupted".into(),
                    message: "Serial connection closed; remaining commands were discarded and will not be replayed".into(),
                    details: Some(self.summary(&state)) },
            }).await;
        }
    }

    /// 整个列表占一个 actor 队列槽。每项单独 write_all/flush，不拼接 shell 表达式。
    pub async fn run<W: Write + Send + 'static>(
        mut self,
        mut writer: Option<W>,
        stop: &AtomicBool,
        tx: &AtomicU64,
        events: &Option<(String, mpsc::Sender<PluginNotification>)>,
        connection_id: &str,
    ) -> (Option<W>, bool) {
        let mut state = Progress {
            started: true,
            ..Progress::default()
        };
        let mut failure = None;
        let mut io_failure = None;
        self.publish(&state, events, connection_id, tx.load(Ordering::Relaxed));
        for index in 0..self.list.payloads.len() {
            if *self.cancel.borrow() {
                state.cancelled = true;
                break;
            }
            if stop.load(Ordering::SeqCst) {
                failure = Some("Serial connection closed");
                break;
            }
            let Some(handle) = writer.take() else {
                failure = Some("Serial writer unavailable");
                break;
            };
            let payload = self.list.payloads[index].clone();
            let bytes = payload.len();
            match timeout(
                Duration::from_secs(2),
                tokio::task::spawn_blocking(move || {
                    let mut handle = handle;
                    let result = handle
                        .write_all(&payload)
                        .map_err(|error| ("write", error))
                        .and_then(|()| handle.flush().map_err(|error| ("flush", error)));
                    (handle, result)
                }),
            )
            .await
            {
                Ok(Ok((handle, result))) => {
                    writer = Some(handle);
                    if let Err((stage, error)) = result {
                        io_failure = Some(
                            json!({"stage":stage,"kind":format!("{:?}",error.kind()),"osError":error.raw_os_error()}),
                        );
                        failure =
                            Some("Serial write failed; delivery of the current entry is unknown");
                        break;
                    }
                }
                _ => {
                    failure = Some("Serial write timed out or failed; delivery of the current entry is unknown");
                    break;
                }
            }
            state.sent += 1;
            state.bytes += bytes;
            tx.fetch_add(bytes as u64, Ordering::Relaxed);
            self.publish(&state, events, connection_id, tx.load(Ordering::Relaxed));
            // 只在两项之间等待；取消会唤醒间隔等待，不打断已交给驱动的写入。
            if index + 1 < self.list.payloads.len() {
                let deadline = tokio::time::Instant::now() + self.list.interval;
                while tokio::time::Instant::now() < deadline && !stop.load(Ordering::SeqCst) {
                    tokio::select! {
                        _ = self.cancel.changed() => break,
                        _ = sleep(deadline.saturating_duration_since(tokio::time::Instant::now()).min(Duration::from_millis(20))) => {}
                    }
                }
            }
        }
        state.finished = true;
        self.publish(&state, events, connection_id, tx.load(Ordering::Relaxed));
        if let Some(message) = failure {
            let mut details = self.summary(&state);
            if let Some(error) = io_failure {
                details["ioError"] = error;
            }
            if let Some((_, sender)) = events {
                let _ = sender
                    .send(PluginNotification::OperationFailed {
                        operation_id: self.operation_id.clone(),
                        error: RuntimeError {
                            category: ErrorCategory::Transport,
                            code: "serial_command_delivery_unknown".into(),
                            message: message.into(),
                            details: Some(details),
                        },
                    })
                    .await;
            }
            return (writer, true);
        }
        // 取消的终态由宿主 cancel 响应写入；绝不把它报成 completed。
        if !state.cancelled {
            if let Some((_, sender)) = events {
                let domain = self.summary(&state);
                let result = if self.mcp {
                    json!({"content":[{"type":"text","text":domain.to_string()}],"isError":false})
                } else {
                    domain
                };
                let _ = sender
                    .send(PluginNotification::OperationCompleted {
                        operation_id: self.operation_id,
                        result,
                    })
                    .await;
            }
        }
        (writer, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Default)]
    struct RecordingWriter {
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        fail_at: Option<usize>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut writes = self
                .writes
                .lock()
                .map_err(|_| io::Error::other("test lock poisoned"))?;
            if self.fail_at == Some(writes.len()) {
                return Err(io::Error::other("injected failure"));
            }
            writes.push(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn job(interval: u64) -> Result<(CommandJob, CommandHandle)> {
        Ok(CommandJob::new(
            "op-list".into(),
            false,
            CommandList::parse(
                &json!({"commands":["first", "second", "third"],"lineEnding":"cr","intervalMs":interval}),
            )?,
        ))
    }

    #[tokio::test]
    async fn sends_separate_writes_in_order_and_reports_bytes_not_exit_codes() -> Result<()> {
        let (mut job, handle) = job(0)?;
        job.mcp = true;
        let writer = RecordingWriter::default();
        let writes = writer.writes.clone();
        let tx = AtomicU64::new(0);
        let (sender, mut events) = mpsc::channel(16);
        let (_, failed) = job
            .run(
                Some(writer),
                &AtomicBool::new(false),
                &tx,
                &Some(("one".into(), sender)),
                "conn",
            )
            .await;
        assert!(!failed);
        assert_eq!(
            *writes.lock().map_err(|_| anyhow::anyhow!("poisoned"))?,
            [
                b"first\r".to_vec(),
                b"second\r".to_vec(),
                b"third\r".to_vec()
            ]
        );
        assert_eq!(tx.load(Ordering::Relaxed), 19);
        assert_eq!(handle.progress.borrow().sent, 3);
        let mut result = None;
        while let Ok(event) = events.try_recv() {
            if let PluginNotification::OperationCompleted { result: value, .. } = event {
                result = Some(value);
            }
        }
        let result = result.ok_or_else(|| anyhow::anyhow!("missing completion"))?;
        let domain: Value = serde_json::from_str(
            result["content"][0]["text"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing MCP text"))?,
        )?;
        assert_eq!(domain["deviceCompletion"], "unknown");
        assert_eq!(domain["remaining"], 0);
        assert!(domain.get("exitCode").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn hex_and_shell_controls_reach_writer_as_exact_separate_payloads() -> Result<()> {
        let controls = "\u{3}\t\u{1b}[A\u{7f}\r\n中文\0";
        for list in [
            CommandList::parse(&json!({"commands":[controls,"next\r"],"intervalMs":0}))?,
            CommandList::parse_hex(&json!({"commands":["00 80 FF","0d0a"],"intervalMs":0}))?,
        ] {
            let expected = list.payloads.clone();
            let (job, handle) = CommandJob::new("exact-input".into(), true, list);
            let writer = RecordingWriter::default();
            let writes = writer.writes.clone();
            let (_, failed) = job
                .run(
                    Some(writer),
                    &AtomicBool::new(false),
                    &AtomicU64::new(0),
                    &None,
                    "conn",
                )
                .await;
            assert!(!failed);
            assert_eq!(*writes.lock().map_err(|_| anyhow!("poisoned"))?, expected);
            assert_eq!(handle.progress.borrow().sent, 2);
        }
        Ok(())
    }

    #[tokio::test]
    async fn write_failure_drops_remaining_entries_with_unknown_delivery() -> Result<()> {
        let (job, handle) = job(0)?;
        let writer = RecordingWriter {
            fail_at: Some(1),
            ..Default::default()
        };
        let writes = writer.writes.clone();
        let (sender, mut events) = mpsc::channel(16);
        let (_, failed) = job
            .run(
                Some(writer),
                &AtomicBool::new(false),
                &AtomicU64::new(0),
                &Some(("one".into(), sender)),
                "conn",
            )
            .await;
        assert!(failed);
        assert_eq!(
            writes
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .len(),
            1
        );
        assert_eq!(handle.progress.borrow().sent, 1);
        let mut error = None;
        while let Ok(event) = events.try_recv() {
            if let PluginNotification::OperationFailed { error: value, .. } = event {
                error = Some(value);
            }
        }
        let error = error.ok_or_else(|| anyhow::anyhow!("missing failure"))?;
        assert_eq!(error.category, ErrorCategory::Transport);
        assert_eq!(
            error.details.as_ref().map(|details| &details["sent"]),
            Some(&json!(1))
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancel_wakes_the_interval_and_discards_the_remainder() -> Result<()> {
        let (job, mut handle) = job(1000)?;
        let writer = RecordingWriter::default();
        let writes = writer.writes.clone();
        let task = tokio::spawn(async move {
            job.run(
                Some(writer),
                &AtomicBool::new(false),
                &AtomicU64::new(0),
                &None,
                "conn",
            )
            .await
        });
        timeout(Duration::from_secs(2), async {
            while handle.progress.borrow().sent == 0 {
                handle.progress.changed().await?;
            }
            Result::<()>::Ok(())
        })
        .await??;
        handle.cancel.send(true)?;
        let (_, failed) = timeout(Duration::from_millis(500), task).await??;
        assert!(!failed);
        assert!(handle.progress.borrow().cancelled);
        assert_eq!(
            writes
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .len(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_queued_list_and_disconnected_list_never_send() -> Result<()> {
        for disconnected in [false, true] {
            let (job, handle) = job(0)?;
            if !disconnected {
                handle.cancel.send(true)?;
            }
            let writer = RecordingWriter::default();
            let writes = writer.writes.clone();
            let (_, failed) = job
                .run(
                    Some(writer),
                    &AtomicBool::new(disconnected),
                    &AtomicU64::new(0),
                    &None,
                    "conn",
                )
                .await;
            assert_eq!(failed, disconnected);
            assert!(writes
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .is_empty());
            assert!(handle.progress.borrow().finished);
        }
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_waits_for_inflight_write_and_never_sends_the_next_entry() -> Result<()> {
        struct GatedWriter {
            inner: RecordingWriter,
            entered: Option<tokio::sync::oneshot::Sender<()>>,
            release: std::sync::mpsc::Receiver<()>,
        }
        impl Write for GatedWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(entered) = self.entered.take() {
                    let _ = entered.send(());
                    self.release
                        .recv_timeout(Duration::from_secs(2))
                        .map_err(io::Error::other)?;
                }
                self.inner.write(bytes)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.inner.flush()
            }
        }
        let (job, handle) = job(0)?;
        let inner = RecordingWriter::default();
        let writes = inner.writes.clone();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, receiver) = std::sync::mpsc::channel();
        let task = tokio::spawn(async move {
            job.run(
                Some(GatedWriter {
                    inner,
                    entered: Some(entered),
                    release: receiver,
                }),
                &AtomicBool::new(false),
                &AtomicU64::new(0),
                &None,
                "conn",
            )
            .await
        });
        timeout(Duration::from_secs(2), started).await??;
        handle.cancel.send(true)?;
        assert!(
            !handle.progress.borrow().finished,
            "inflight bytes cannot be cancelled retroactively"
        );
        release.send(())?;
        let (_, failed) = timeout(Duration::from_secs(2), task).await??;
        assert!(!failed);
        assert!(handle.progress.borrow().cancelled);
        assert_eq!(handle.progress.borrow().sent, 1);
        assert_eq!(writes.lock().map_err(|_| anyhow!("poisoned"))?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn disconnection_during_interval_discards_remaining_commands() -> Result<()> {
        let (job, mut handle) = job(1000)?;
        let writer = RecordingWriter::default();
        let writes = writer.writes.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let task = tokio::spawn(async move {
            job.run(
                Some(writer),
                &worker_stop,
                &AtomicU64::new(0),
                &None,
                "conn",
            )
            .await
        });
        timeout(Duration::from_secs(2), async {
            while handle.progress.borrow().sent == 0 {
                handle.progress.changed().await?;
            }
            Result::<()>::Ok(())
        })
        .await??;
        stop.store(true, Ordering::SeqCst);
        let (_, failed) = timeout(Duration::from_millis(500), task).await??;
        assert!(failed);
        assert_eq!(writes.lock().map_err(|_| anyhow!("poisoned"))?.len(), 1);
        Ok(())
    }

    #[test]
    fn list_preserves_entries_and_adds_endings_without_shell_composition() -> Result<()> {
        let list =
            CommandList::parse(&json!({"commands":["cd /tmp", "printf 'a && b'", "echo 中文"]}))?;
        assert_eq!(
            list.payloads,
            [
                b"cd /tmp".to_vec(),
                b"printf 'a && b'".to_vec(),
                "echo 中文".as_bytes().to_vec()
            ]
        );
        assert_eq!(list.interval, Duration::from_millis(100));
        for (ending, suffix) in [("cr", "\r"), ("lf", "\n"), ("crlf", "\r\n")] {
            let list = CommandList::parse(
                &json!({"commands":["pwd"],"lineEnding":ending,"intervalMs":0}),
            )?;
            assert_eq!(list.payloads[0], format!("pwd{suffix}").as_bytes());
        }
        Ok(())
    }

    #[test]
    fn validates_the_entire_list_before_any_sending() {
        for args in [
            json!({"commands":[]}),
            json!({"commands":["ok", ""]}),
            json!({"commands":[7]}),
            json!({"commands":["x".repeat(4097)]}),
            json!({"commands":vec!["x";65]}),
            json!({"commands":vec!["x".repeat(4096);5]}),
            json!({"commands":["pwd"],"lineEnding":"invalid"}),
            json!({"commands":["pwd"],"intervalMs":1001}),
            json!({"commands":["pwd"],"intervalMs":-1}),
            json!({"commands":["pwd"],"intervalMs":1.5}),
            json!({"commands":["pwd"],"text":"ignored?"}),
            json!({"text":"pwd"}),
        ] {
            assert!(CommandList::parse(&args).is_err(), "{args}");
        }
        assert!(CommandList::parse(&json!({"commands":vec!["x".repeat(4095);4]})).is_ok());
    }

    #[test]
    fn hex_lists_preserve_order_and_validate_every_entry() -> Result<()> {
        let list =
            CommandList::parse_hex(&json!({"commands":["00 80 FF", "0d0a"],"intervalMs":0}))?;
        assert_eq!(list.payloads, [vec![0, 128, 255], vec![13, 10]]);
        for args in [
            json!({"commands":["00", "GG"]}),
            json!({"commands":["00"],"lineEnding":"cr"}),
            json!({"commands":["ff".repeat(4097)]}),
        ] {
            assert!(CommandList::parse_hex(&args).is_err());
        }
        Ok(())
    }
}
