//! `local.exec` 的后台作业：独立子进程、两条有界流、超时与取消。
//!
//! 共享 PTY 只服务人工输入；Agent 的命令走独立子进程，退出码和两条流都精确，
//! 不受提示符、回显和 shell 重绘影响。
use std::{
    collections::{HashMap, VecDeque},
    process::Stdio,
    sync::{atomic::AtomicU64, Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Result};
use coterm_proto::runtime::message::DriverNotification;
use coterm_term::{encode, OutputBuffer};
use serde_json::{json, Value};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    sync::{mpsc, watch},
};

use crate::shell::ShellFamily;

/// 每个作业每条流的保留上限。
pub const STREAM_LIMIT: usize = 64 * 1024;
pub const STREAM_CHUNK: usize = 4 * 1024;
/// 单次 `local.output` 每流返回的字节上限。
pub const OUT_BYTES: usize = 4 * 1024;
/// 结果里每流的预览上限。
pub const PREVIEW_BYTES: usize = 4 * 1024;
/// 同时保留的作业数。
pub const MAX_JOBS: usize = 8;
/// 脚本上限，与 SSH 的 exec 保持一致。
pub const MAX_SCRIPT: usize = 64 * 1024;
pub const MIN_TIMEOUT_MS: u64 = 1_000;
pub const MAX_TIMEOUT_MS: u64 = 1_800_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[derive(Default)]
struct JobState {
    done: bool,
    exit_code: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    error: Option<String>,
    duration_ms: u64,
}

pub struct Job {
    pub(crate) instance_id: String,
    record: Option<(crate::command::History, String)>,
    family: ShellFamily,
    stdout: Arc<Mutex<OutputBuffer>>,
    stderr: Arc<Mutex<OutputBuffer>>,
    state: Arc<Mutex<JobState>>,
    cancel: watch::Sender<bool>,
    started: Instant,
}

#[derive(Default)]
pub struct Jobs {
    jobs: HashMap<String, Arc<Job>>,
    order: VecDeque<String>,
}

impl Jobs {
    /// 登记新作业；容量满时先淘汰已完成的作业，全部在跑就拒绝。
    fn insert(&mut self, operation_id: &str, job: Arc<Job>) -> Result<()> {
        while self.jobs.len() >= MAX_JOBS {
            let evicted = self
                .order
                .iter()
                .find(|id| self.jobs.get(*id).is_some_and(|job| lock(&job.state).done))
                .cloned();
            let Some(evicted) = evicted else {
                bail!("Too many running local jobs; cancel or wait for one to finish");
            };
            self.remove(&evicted);
        }
        self.jobs.insert(operation_id.to_owned(), job);
        self.order.push_back(operation_id.to_owned());
        Ok(())
    }

    fn remove(&mut self, operation_id: &str) {
        self.jobs.remove(operation_id);
        self.order.retain(|id| id != operation_id);
    }

    pub fn get(&self, operation_id: &str) -> Result<Arc<Job>> {
        self.jobs
            .get(operation_id)
            .cloned()
            .ok_or_else(|| anyhow!("Local job not found or already evicted"))
    }

    pub fn contains(&self, operation_id: &str) -> bool {
        self.jobs.contains_key(operation_id)
    }

    /// 请求停止一个作业；返回是否在宽限期内确认结束。
    pub async fn cancel(&self, operation_id: &str) -> Result<bool> {
        let job = self.get(operation_id)?;
        let _ = job.cancel.send(true);
        for _ in 0..20 {
            if lock(&job.state).done {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let done = lock(&job.state).done;
        Ok(done)
    }

    /// 实例销毁或驱动停止时取消全部作业。
    pub fn cancel_instance(&mut self, instance_id: &str) {
        for job in self.jobs.values() {
            if job.instance_id == instance_id {
                let _ = job.cancel.send(true);
            }
        }
        self.jobs.retain(|_, job| job.instance_id != instance_id);
        let present: Vec<String> = self.jobs.keys().cloned().collect();
        self.order.retain(|id| present.contains(id));
    }

    pub fn cancel_all(&mut self) {
        for job in self.jobs.values() {
            let _ = job.cancel.send(true);
        }
        self.jobs.clear();
        self.order.clear();
    }

    /// 作业是否已结束，用于实例销毁前的收尾。
    #[allow(dead_code)]
    pub fn is_done(&self, operation_id: &str) -> bool {
        self.jobs
            .get(operation_id)
            .is_some_and(|job| lock(&job.state).done)
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

pub struct ExecRequest {
    pub instance_id: String,
    pub shell: std::path::PathBuf,
    pub family: ShellFamily,
    pub cwd: std::path::PathBuf,
    pub script: String,
    pub timeout: Duration,
    pub operation_id: String,
    pub mcp: bool,
    pub(crate) history: Option<crate::command::History>,
}

/// 启动一个作业，立即返回；完成时通过 `operations` 通知发送结果。
pub async fn spawn(
    jobs: &mut Jobs,
    events: mpsc::Sender<DriverNotification>,
    request: ExecRequest,
) -> Result<Value> {
    if request.family == ShellFamily::Unknown {
        bail!("This local shell family cannot run Agent commands");
    }
    let (cancel, cancel_rx) = watch::channel(false);
    let job = Arc::new(Job {
        instance_id: request.instance_id.clone(),
        record: request
            .history
            .clone()
            .map(|history| (history, request.operation_id.clone())),
        family: request.family,
        stdout: Arc::new(Mutex::new(OutputBuffer::bytes(STREAM_LIMIT, STREAM_CHUNK))),
        stderr: Arc::new(Mutex::new(OutputBuffer::bytes(STREAM_LIMIT, STREAM_CHUNK))),
        state: Arc::new(Mutex::new(JobState::default())),
        cancel,
        started: Instant::now(),
    });
    jobs.insert(&request.operation_id, job.clone())?;

    let (child, temp_script) = match prepare(&request).await {
        Ok(prepared) => prepared,
        Err(error) => {
            jobs.remove(&request.operation_id);
            return Err(error);
        }
    };
    let mut child = child;
    // 读取线程必须拿到两个流；取不到就说明启动失败，别留一个半死的作业。
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let runner = Runner {
        job: job.clone(),
        operation_id: request.operation_id.clone(),
        mcp: request.mcp,
        events,
        timeout: request.timeout,
        temp_script,
    };
    match (stdout, stderr) {
        (Some(stdout), Some(stderr)) => runner.start(child, stdout, stderr, cancel_rx),
        _ => {
            jobs.remove(&request.operation_id);
            let _ = child.kill().await;
            bail!("Local job streams could not be opened");
        }
    }
    Ok(json!({"_cotermDeferred": request.operation_id}))
}

/// 按 Shell 家族构造入口：脚本一律经 stdin 或临时文件交付，不进命令行参数。
///
/// 返回子进程和（仅 cmd 家族）需要事后删除的临时脚本路径。
async fn prepare(request: &ExecRequest) -> Result<(Child, Option<std::path::PathBuf>)> {
    let mut temp_script = None;
    let mut command = match request.family {
        ShellFamily::Posix => {
            let mut command = Command::new(&request.shell);
            command.arg("-s");
            command
        }
        ShellFamily::PowerShell => {
            let mut command = Command::new(&request.shell);
            command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "-"]);
            command
        }
        ShellFamily::Cmd => {
            // cmd 没有从 stdin 读脚本的方式，只能落临时文件。批处理默认回显每条命令，
            // 这里在文件头关掉，结果里只留脚本自己的输出。
            let path = std::env::temp_dir().join(format!(
                "coterm-{}-{}.cmd",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::write(&path, format!("@echo off\r\n{}", request.script))?;
            temp_script = Some(path.clone());
            let mut command = Command::new(&request.shell);
            command.args(["/D", "/S", "/C"]).arg(&path);
            command
        }
        ShellFamily::Unknown => bail!("This local shell family cannot run Agent commands"),
    };
    // cmd 走临时文件，不接 stdin：否则批处理结束后它会回到交互模式，把提示符和回显
    // 一起写进结果。
    let pipe_stdin = request.family != ShellFamily::Cmd;
    command
        .current_dir(&request.cwd)
        .env("COTERM_INSTANCE", &request.instance_id)
        .stdin(if pipe_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        // 独立进程组，取消时能连同子进程一起收掉。
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| anyhow!("Local job could not be started"))?;
    if pipe_stdin {
        if let Some(mut stdin) = child.stdin.take() {
            let script = request.script.clone();
            // 写入与读取分离：脚本较大时不能让写阻塞等待进程消费。
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(script.as_bytes()).await;
                let _ = stdin.shutdown().await;
            });
        }
    }
    Ok((child, temp_script))
}

struct Runner {
    job: Arc<Job>,
    operation_id: String,
    mcp: bool,
    events: mpsc::Sender<DriverNotification>,
    timeout: Duration,
    temp_script: Option<std::path::PathBuf>,
}

impl Runner {
    fn start(
        self,
        mut child: Child,
        stdout: tokio::process::ChildStdout,
        stderr: tokio::process::ChildStderr,
        mut cancel: watch::Receiver<bool>,
    ) {
        let out_buffer = self.job.stdout.clone();
        let err_buffer = self.job.stderr.clone();
        tokio::spawn(async move {
            let out_task = tokio::spawn(pump(stdout, out_buffer, self.job.record.clone(), false));
            let err_task = tokio::spawn(pump(stderr, err_buffer, self.job.record.clone(), true));

            let mut timed_out = false;
            let mut cancelled = false;
            let mut error = None;
            let status = tokio::select! {
                status = child.wait() => status.ok(),
                _ = tokio::time::sleep(self.timeout) => {
                    timed_out = true;
                    let _ = child.start_kill();
                    child.wait().await.ok()
                }
                _ = cancel.changed() => {
                    cancelled = true;
                    let _ = child.start_kill();
                    child.wait().await.ok()
                }
            };
            if status.is_none() {
                error = Some("Local job ended without an exit status".to_owned());
            }
            // Descendants can inherit the pipes after the direct child exits.
            // Bound draining so cancellation and execution deadlines remain effective.
            let (out_closed, err_closed) =
                tokio::join!(drain_stream(out_task), drain_stream(err_task));
            if !out_closed || !err_closed {
                error = Some(
                    "Output streams did not close after shell exit; output may be incomplete"
                        .to_owned(),
                );
            }
            lock(&self.job.stdout).close();
            lock(&self.job.stderr).close();
            if let Some(path) = &self.temp_script {
                cleanup_script(path);
            }

            let exit_code = status.as_ref().and_then(|status| status.code());
            {
                let mut state = lock(&self.job.state);
                state.done = true;
                state.exit_code = exit_code;
                state.timed_out = timed_out;
                state.cancelled = cancelled;
                state.error = error;
                state.duration_ms = self.job.started.elapsed().as_millis() as u64;
            }
            if let Some((history, id)) = &self.job.record {
                crate::command::lock(history).finish(
                    id,
                    if cancelled {
                        "cancelled"
                    } else if timed_out {
                        "timed_out"
                    } else if status.is_none() {
                        "unknown"
                    } else {
                        "completed"
                    },
                    exit_code,
                );
            }
            let domain = result(&self.job);
            let payload = if self.mcp {
                crate::result::tool_result(&domain)
            } else {
                domain
            };
            let _ = self
                .events
                .send(DriverNotification::OperationCompleted {
                    operation_id: self.operation_id,
                    result: payload,
                })
                .await;
        });
    }
}

async fn drain_stream(mut task: tokio::task::JoinHandle<()>) -> bool {
    if let Ok(result) = tokio::time::timeout(Duration::from_millis(500), &mut task).await {
        return result.is_ok();
    }
    task.abort();
    let _ = task.await;
    false
}

async fn pump<R>(
    mut reader: R,
    buffer: Arc<Mutex<OutputBuffer>>,
    record: Option<(crate::command::History, String)>,
    stderr: bool,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut chunk = [0u8; STREAM_CHUNK];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                lock(&buffer).push(&chunk[..read]);
                if let Some((history, id)) = &record {
                    crate::command::lock(history).append(id, &chunk[..read], stderr);
                }
            }
            Err(_) => break,
        }
    }
}

fn stream_preview(buffer: &Mutex<OutputBuffer>) -> Value {
    let buffer = lock(buffer);
    let data = buffer.preview(PREVIEW_BYTES);
    let (encoding, field, value) = encode(&data);
    let dropped = buffer.dropped() + buffer.retained().saturating_sub(data.len());
    let mut preview = json!({
        field: value,
        "encoding": encoding,
        "bytes": data.len(),
        "originalBytes": buffer.total(),
        "droppedBytes": dropped,
        "truncated": dropped > 0,
        "limit": PREVIEW_BYTES,
    });
    if buffer.is_closed() {
        preview["closed"] = json!(true);
    }
    preview
}

fn result(job: &Job) -> Value {
    if let Some((history, id)) = &job.record {
        if let Ok(value) = crate::command::lock(history).value(id) {
            return value;
        }
    }
    let state = lock(&job.state);
    json!({
        "family": job.family.as_str(),
        "exitCode": state.exit_code,
        "timedOut": state.timed_out,
        "cancelled": state.cancelled,
        "durationMs": state.duration_ms,
        "error": state.error,
        "stdout": stream_preview(&job.stdout),
        "stderr": stream_preview(&job.stderr),
        "executionContext": {"channel": "exec", "inheritsPtyState": false},
    })
}

/// 读取作业的两条流。
pub fn output(job: &Job, stdout_cursor: u64, stderr_cursor: u64) -> Result<Value> {
    let state = lock(&job.state);
    let stdout = lock(&job.stdout).collect(stdout_cursor, OUT_BYTES)?;
    let stderr = lock(&job.stderr).collect(stderr_cursor, OUT_BYTES)?;
    let page = |value: &coterm_term::Page| {
        let (encoding, field, data) = encode(&value.data);
        json!({
            field: data,
            "encoding": encoding,
            "cursor": value.cursor.to_string(),
            "gap": value.gap,
            "more": value.more,
        })
    };
    Ok(json!({
        "done": state.done,
        "exitCode": state.exit_code,
        "timedOut": state.timed_out,
        "cancelled": state.cancelled,
        "stdout": page(&stdout),
        "stderr": page(&stderr),
        "error": state.error,
    }))
}

/// 清理 `ShellFamily::Cmd` 落下的临时脚本。
pub fn cleanup_script(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}
