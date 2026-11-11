use super::{config::Config, session::Client};
use crate::stream::{
    payload::{decode_hex, encode_text},
    shell::{validate_prompt, Mode},
};
use anyhow::{anyhow, bail, Result};
use coterm_core::driver::DriverNotification;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{
    sync::{mpsc, watch, OwnedMutexGuard},
    task::JoinHandle,
    time::{sleep, timeout, Instant},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    commands: Vec<String>,
    #[serde(default)]
    interval_ms: Option<u64>,
    #[serde(default)]
    line_ending: Option<String>,
    #[serde(default)]
    wait_timeout_ms: Option<u64>,
    #[serde(default)]
    wait_for_prompt: Option<String>,
}

pub(crate) struct List {
    inputs: Vec<String>,
    bytes: Vec<Vec<u8>>,
    interval: Duration,
    wait: Duration,
    prompt: Option<String>,
    hex: bool,
}
impl List {
    pub(crate) fn parse(value: Value, config: &Config, hex: bool) -> Result<Self> {
        let args: Args =
            serde_json::from_value(value).map_err(|_| anyhow!("Invalid command list"))?;
        if !(1..=32).contains(&args.commands.len())
            || args.interval_ms.unwrap_or(100) > 60000
            || !(100..=60000).contains(&args.wait_timeout_ms.unwrap_or(5000))
        {
            bail!("Invalid command count or timing")
        }
        if hex && (args.line_ending.is_some() || args.wait_for_prompt.is_some()) {
            bail!("HEX sends have no line ending or prompt interpretation")
        }
        if config.mode == Mode::Stream && args.wait_for_prompt.is_some() {
            bail!("Byte streams do not have Shell prompt boundaries")
        }
        let prompt = if config.mode == Mode::Shell && !hex {
            args.wait_for_prompt.or_else(|| config.prompt.clone())
        } else {
            None
        };
        if let Some(prompt) = &prompt {
            validate_prompt(prompt)?;
        }
        let mut bytes = Vec::new();
        let mut total = 0;
        for command in &args.commands {
            if command.is_empty() || command.len() > 4096 {
                bail!("Each command must contain 1–4096 UTF-8 bytes")
            }
            let payload = if hex {
                decode_hex(command)?
            } else {
                encode_text(command, args.line_ending.as_deref().unwrap_or("crlf"))?
            };
            total += payload.len();
            if total > 16384 {
                bail!("Command batch exceeds 16 KiB")
            }
            bytes.push(payload);
        }
        Ok(Self {
            inputs: args.commands,
            bytes,
            interval: Duration::from_millis(args.interval_ms.unwrap_or(100)),
            wait: Duration::from_millis(args.wait_timeout_ms.unwrap_or(5000)),
            prompt,
            hex,
        })
    }
}

pub(crate) struct Job {
    cancel: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Drop for Job {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        self.task.abort();
    }
}
impl Job {
    pub(crate) fn finished(&self) -> bool {
        self.task.is_finished()
    }
    pub(crate) async fn cancel(&mut self) -> Value {
        let _ = self.cancel.send(true);
        if self.task.is_finished() {
            return json!({"outcome":"stopped"});
        }
        if timeout(Duration::from_secs(3), &mut self.task)
            .await
            .is_ok()
        {
            json!({"outcome":"stopped"})
        } else {
            json!({"outcome":"unknown"})
        }
    }
}

pub(crate) async fn start(
    client: Client,
    list: List,
    operation: String,
    mcp: bool,
    events: mpsc::Sender<DriverNotification>,
) -> Result<Job> {
    let gate = client
        .busy
        .clone()
        .try_lock_owned()
        .map_err(|_| anyhow!("Command queue is busy"))?;
    let epoch = client.epoch().await?;
    let (cancel, cancelled) = watch::channel(false);
    let task = tokio::spawn(async move {
        let worker = Batch {
            client,
            list,
            epoch,
            cancelled,
            operation: operation.clone(),
            sent: 0,
            sent_commands: 0,
            _gate: gate,
        };
        let result = worker.run().await;
        let notification = match result {
            Ok(result) => DriverNotification::OperationCompleted {
                operation_id: operation,
                result: if mcp {
                    crate::stream::result::tool_result(result)
                } else {
                    result
                },
            },
            Err(error) => DriverNotification::OperationFailed {
                operation_id: operation,
                error,
            },
        };
        let _ = events.send(notification).await;
    });
    Ok(Job { cancel, task })
}

struct Batch {
    client: Client,
    list: List,
    epoch: String,
    cancelled: watch::Receiver<bool>,
    operation: String,
    sent: usize,
    sent_commands: usize,
    _gate: OwnedMutexGuard<()>,
}
impl Batch {
    async fn check(&self) -> Result<()> {
        if *self.cancelled.borrow() || self.client.is_stopped() {
            bail!("Command cancelled or disconnected; sent input must not be replayed")
        }
        if self.client.epoch().await? != self.epoch {
            bail!("Connection changed; the remaining commands were not sent")
        }
        Ok(())
    }

    async fn run(mut self) -> std::result::Result<Value, coterm_core::model::RuntimeError> {
        let result = self.send().await;
        if let Err(reason) = result {
            let mut buffer = self.client.output.lock().await;
            buffer.shell.interrupt_operation(&self.operation, "unknown");
            let mut details = if self.list.prompt.is_some() {
                buffer.shell.operation_result(&self.operation)
            } else {
                stream_result(&self.list.inputs[..self.sent_commands])
            };
            details["bytes"] = json!(self.sent);
            details["sentCommands"] = json!(self.sent_commands);
            details["connectionId"] = json!(self.epoch);
            details["deviceCompletion"] = json!("unknown");
            details["inputFormat"] = json!(if self.list.hex { "hex" } else { "text" });
            let mut error = super::backend::error("network_command_uncertain", reason.to_string());
            error.category = coterm_core::model::ErrorCategory::Transport;
            error.details = Some(details);
            return Err(error);
        }
        result.map_err(|reason| {
            super::backend::error("network_command_uncertain", reason.to_string())
        })
    }

    async fn send(&mut self) -> Result<Value> {
        for index in 0..self.list.bytes.len() {
            self.check().await?;
            if let Some(prompt) = &self.list.prompt {
                self.client.output.lock().await.shell.begin(
                    &self.operation,
                    index,
                    &self.list.inputs[index],
                    prompt,
                )?;
            }
            self.sent += self
                .client
                .write(&self.epoch, self.list.bytes[index].clone())
                .await?;
            self.sent_commands += 1;
            if self.list.prompt.is_some() {
                let deadline = Instant::now() + self.list.wait;
                loop {
                    self.check().await?;
                    if self
                        .client
                        .output
                        .lock()
                        .await
                        .shell
                        .completed(&self.operation, index)
                    {
                        break;
                    }
                    if Instant::now() >= deadline {
                        bail!("Shell prompt wait timed out; remaining commands were not sent")
                    }
                    tokio::select! { _ = self.cancelled.changed() => {}, _ = sleep(Duration::from_millis(10)) => {} }
                }
            }
            if index + 1 < self.list.bytes.len() {
                tokio::select! { _ = self.cancelled.changed() => {}, _ = sleep(self.list.interval) => {} }
            }
        }
        self.check().await?;
        let mut value = if self.list.prompt.is_some() {
            self.client
                .output
                .lock()
                .await
                .shell
                .operation_result(&self.operation)
        } else {
            stream_result(&self.list.inputs)
        };
        if let Some(object) = value.as_object_mut() {
            object.insert("bytes".into(), json!(self.sent));
            object.insert("status".into(), json!("completed"));
            object.insert("connectionId".into(), json!(self.epoch));
            object.insert(
                "inputFormat".into(),
                json!(if self.list.hex { "hex" } else { "text" }),
            );
            object.insert(
                "deviceCompletion".into(),
                json!(if self.list.prompt.is_some() {
                    "prompt_observed"
                } else {
                    "unknown"
                }),
            );
        }
        Ok(value)
    }
}

fn stream_result(inputs: &[String]) -> Value {
    let input = inputs.join("\n");
    let (stdin, _) = crate::stream::payload::bounded_output_text(&input, 2048, false);
    json!({"stdin":stdin,"stdout":"","stderr":"","streamMode":"merged","exitCode":null,"truncated":stdin.len()<input.len()})
}
