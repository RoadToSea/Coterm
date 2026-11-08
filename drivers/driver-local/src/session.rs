//! 本地 PTY 会话：启动 Shell、搬运字节、维护有界回放缓冲。
//!
//! 这里不解析 Shell 语义，只保证三件事：子进程的字节按块进入有界缓冲、前端按游标能续读、
//! 进程退出时缓冲关闭且事件只发一次。命令边界与退出码属于上层。
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};

use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use coterm_proto::runtime::message::DriverNotification;
use coterm_term::OutputBuffer;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde_json::{json, Value};
use tokio::sync::mpsc::{self, error::TrySendError};

use crate::command::{lock as record_lock, Foreground, History};
use crate::shell::{ResolvedShell, ShellFamily};

#[derive(Clone)]
struct Commands {
    active: Arc<Mutex<Option<Foreground>>>,
    history: History,
}

impl Commands {
    fn complete(&self, code: Option<i32>) -> Option<DriverNotification> {
        let mut active = record_lock(&self.active);
        let command = active.take()?;
        let mut history = record_lock(&self.history);
        history.finish(
            &command.id,
            if code.is_some() {
                "completed"
            } else {
                "unknown"
            },
            code,
        );
        if command.notified {
            return None;
        }
        let value = history.value(&command.id).ok()?;
        Some(DriverNotification::OperationCompleted {
            operation_id: command.id.clone(),
            result: if command.mcp {
                crate::result::tool_result(&value)
            } else {
                value
            },
        })
    }
}

/// 终端回放缓冲上限。
pub const OUTPUT_LIMIT: usize = 256 * 1024;
/// 每块字节数：前端按块游标续读，块越小缺口越细。
pub const OUTPUT_CHUNK: usize = 4 * 1024;
/// 单次 `local.read` 返回的字节上限。
pub const READ_BYTES: usize = 8 * 1024;
/// 单次 `local.input` 接收的字节上限。
pub const MAX_INPUT: usize = 4 * 1024;
/// 输入队列容量：4 × 4 KiB，保证排队中的输入不超过 16 KiB。
pub const INPUT_QUEUE: usize = 4;
/// 最小与最大的终端尺寸，避免异常值把 PTY 压成 0 行列。
pub const MIN_DIMENSION: u16 = 10;
pub const MAX_DIMENSION: u16 = 1000;

static NEXT_SHELL: AtomicU64 = AtomicU64::new(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // 某个线程 panic 不该让整个驱动卡死；继续使用已有状态并让上层观察结果。
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

pub struct ShellSession {
    /// 每次启动都新建，用于丢弃重启前的迟到事件。
    id: String,
    shell: PathBuf,
    family: ShellFamily,
    cwd: PathBuf,
    buffer: Arc<Mutex<OutputBuffer>>,
    input: mpsc::Sender<Vec<u8>>,
    commands: Commands,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    size: Arc<Mutex<(u16, u16)>>,
    closed: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<u32>>>,
}

impl ShellSession {
    pub(crate) fn spawn(
        instance_id: &str,
        resolved: &ResolvedShell,
        events: mpsc::Sender<DriverNotification>,
        rows: u16,
        cols: u16,
        history: History,
    ) -> Result<Self> {
        let size = (clamp_dimension(rows), clamp_dimension(cols));
        let pty_size = PtySize {
            rows: size.0,
            cols: size.1,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(pty_size)
            .map_err(|_| anyhow!("Local pseudo terminal is unavailable"))?;
        let mut command = CommandBuilder::new(&resolved.path);
        command.cwd(&resolved.cwd);
        command.env("COTERM_INSTANCE", instance_id);
        if !cfg!(windows) {
            command.env("TERM", "xterm-256color");
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|_| anyhow!("Local shell could not be started"))?;
        // 关掉从端句柄，读端才能在子进程退出后看到 EOF。
        drop(pair.slave);
        let master = pair.master;
        let reader = master
            .try_clone_reader()
            .map_err(|_| anyhow!("Local shell output could not be read"))?;
        let writer = master
            .take_writer()
            .map_err(|_| anyhow!("Local shell input could not be opened"))?;

        let id = format!("shell-{}", NEXT_SHELL.fetch_add(1, Ordering::Relaxed));
        let buffer = Arc::new(Mutex::new(OutputBuffer::bytes(OUTPUT_LIMIT, OUTPUT_CHUNK)));
        let closed = Arc::new(AtomicBool::new(false));
        let exit_code = Arc::new(Mutex::new(None));
        let child = Arc::new(Mutex::new(child));
        let (input, queue) = mpsc::channel::<Vec<u8>>(INPUT_QUEUE);

        let commands = Commands {
            active: Arc::new(Mutex::new(None)),
            history,
        };
        spawn_reader(
            instance_id,
            &id,
            reader,
            buffer.clone(),
            closed.clone(),
            events.clone(),
            commands.clone(),
        );
        spawn_writer(writer, queue);
        ExitWatcher {
            instance_id: instance_id.to_owned(),
            shell_id: id.clone(),
            child: child.clone(),
            buffer: buffer.clone(),
            closed: closed.clone(),
            exit_code: exit_code.clone(),
            events,
        }
        .start();

        Ok(Self {
            id,
            shell: resolved.path.clone(),
            family: resolved.family,
            cwd: resolved.cwd.clone(),
            buffer,
            input,
            commands,
            master: Arc::new(Mutex::new(master)),
            child,
            size: Arc::new(Mutex::new(size)),
            closed,
            exit_code,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn read(&self, after: u64) -> Result<Value> {
        let page = lock(&self.buffer).collect(after, READ_BYTES)?;
        Ok(json!({
            "shellId": self.id,
            "data": STANDARD.encode(&page.data),
            "cursor": page.cursor.to_string(),
            "gap": page.gap,
            "more": page.more,
            "closed": page.closed,
        }))
    }

    pub(crate) fn read_lines(&self, count: usize) -> Value {
        let buffer = lock(&self.buffer);
        crate::command::lines(
            &buffer.preview(OUTPUT_LIMIT),
            count,
            buffer.dropped() > 0,
            self.is_closed(),
        )
    }

    pub(crate) async fn command(
        &self,
        id: &str,
        mcp: bool,
        source: String,
        script: &str,
    ) -> Result<Value> {
        if self.is_closed() {
            bail!("Local shell has exited; restart it explicitly");
        }
        if record_lock(&self.commands.active).is_some() {
            bail!("A foreground command is still unresolved; wait or restart the Shell");
        }
        let (command, wire) = Foreground::new(id, mcp, script, self.family).await?;
        let mut active = record_lock(&self.commands.active);
        record_lock(&self.commands.history).begin(id, source, "terminal")?;
        *active = Some(command);
        if self.input.try_send(wire).is_err() {
            *active = None;
            record_lock(&self.commands.history).finish(id, "unknown", None);
            bail!("Local command input queue is unavailable");
        }
        Ok(json!({"_cotermDeferred":id}))
    }

    pub(crate) fn cancel_command(&self, id: &str) -> Result<bool> {
        let mut active = record_lock(&self.commands.active);
        let Some(command) = active.as_mut().filter(|command| command.id == id) else {
            return Ok(false);
        };
        self.input
            .try_send(vec![3])
            .map_err(|_| anyhow!("Local input queue is unavailable"))?;
        // Ctrl-C does not prove the Shell stopped; hold the lane until a boundary or restart.
        record_lock(&self.commands.history).finish(id, "unknown", None);
        command.notified = true;
        Ok(true)
    }

    /// 人工按键。排队满了就明确失败，不阻塞控制消息循环。
    pub fn input(&self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            bail!("Local input must contain at least one byte");
        }
        if data.len() > MAX_INPUT {
            bail!("Local input must not exceed {MAX_INPUT} bytes");
        }
        if self.is_closed() {
            bail!("Local shell has exited");
        }
        self.input
            .try_send(data.to_vec())
            .map_err(|error| match error {
                TrySendError::Full(_) => anyhow!("Local input queue is full"),
                TrySendError::Closed(_) => anyhow!("Local shell has exited"),
            })
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<Value> {
        if self.is_closed() {
            bail!("Local shell has exited");
        }
        for value in [rows, cols] {
            if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&value) {
                bail!("Local terminal size must be between {MIN_DIMENSION} and {MAX_DIMENSION}");
            }
        }
        lock(&self.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| anyhow!("Local terminal could not be resized"))?;
        *lock(&self.size) = (rows, cols);
        Ok(json!({"ok": true, "rows": rows, "cols": cols}))
    }

    pub fn status(&self) -> Value {
        let (rows, cols) = *lock(&self.size);
        let pid = lock(&self.child).process_id();
        let running = !self.is_closed();
        json!({
            "shellId": self.id,
            "shell": self.shell.to_string_lossy(),
            "family": self.family.as_str(),
            "cwd": self.cwd.to_string_lossy(),
            "rows": rows,
            "cols": cols,
            "pid": pid,
            "running": running,
            "connected": running,
            "exitCode": *lock(&self.exit_code),
            "cursor": lock(&self.buffer).sequence().to_string(),
        })
    }
}

impl Drop for ShellSession {
    fn drop(&mut self) {
        // 断开输出与输入，再结束子进程；顺序反过来会让读取线程收不到 EOF。
        let _ = lock(&self.child).kill();
    }
}

fn clamp_dimension(value: u16) -> u16 {
    value.clamp(MIN_DIMENSION, MAX_DIMENSION)
}

fn spawn_reader(
    instance_id: &str,
    shell_id: &str,
    mut reader: Box<dyn Read + Send>,
    buffer: Arc<Mutex<OutputBuffer>>,
    closed: Arc<AtomicBool>,
    events: mpsc::Sender<DriverNotification>,
    commands: Commands,
) {
    let instance_id = instance_id.to_owned();
    let shell_id = shell_id.to_owned();
    std::thread::spawn(move || {
        let mut chunk = [0u8; OUTPUT_CHUNK];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    let (visible, done) = {
                        let mut active = record_lock(&commands.active);
                        match active.as_mut() {
                            Some(command) => command.feed(&chunk[..read], &commands.history),
                            None => (chunk[..read].to_vec(), None),
                        }
                    };
                    if let Some(code) = done {
                        if let Some(notification) = commands.complete(Some(code)) {
                            let _ = events.blocking_send(notification);
                        }
                    }
                    if visible.is_empty() {
                        continue;
                    }
                    let (data, cursor) = {
                        let mut buffer = lock(&buffer);
                        buffer.push(&visible);
                        (STANDARD.encode(&visible), buffer.sequence())
                    };
                    let notification = DriverNotification::Event {
                        driver_instance_id: instance_id.clone(),
                        name: "local.terminal.output".to_owned(),
                        payload: json!({"shellId": shell_id, "data": data, "cursor": cursor.to_string()}),
                    };
                    // 队列满时阻塞读取线程，形成自然背压；连接断开就结束。
                    if events.blocking_send(notification).is_err() {
                        break;
                    }
                }
                // Windows ConPTY 在子进程退出后以错误结束读取，而不是返回 0。
                Err(_) => break,
            }
        }
        if let Some(notification) = commands.complete(None) {
            let _ = events.blocking_send(notification);
        }
        // 关闭缓冲；读取线程与退出轮询谁先发现都由这里统一收口。
        if end_stream(&buffer, &closed) {
            let _ = events.blocking_send(exit_event(&shell_id, &instance_id, None));
        }
    });
}

fn spawn_writer(mut writer: Box<dyn Write + Send>, mut queue: mpsc::Receiver<Vec<u8>>) {
    std::thread::spawn(move || {
        while let Some(bytes) = queue.blocking_recv() {
            if writer.write_all(&bytes).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });
}

/// 关闭缓冲，并报告「是不是本次调用真正结束了流」，避免读取线程与退出轮询重复通知。
fn end_stream(buffer: &Mutex<OutputBuffer>, closed: &AtomicBool) -> bool {
    lock(buffer).close();
    !closed.swap(true, Ordering::SeqCst)
}

fn exit_event(shell_id: &str, instance_id: &str, exit_code: Option<u32>) -> DriverNotification {
    DriverNotification::Event {
        driver_instance_id: instance_id.to_owned(),
        name: "local.status".to_owned(),
        payload: json!({"shellId": shell_id, "running": false, "exitCode": exit_code}),
    }
}

/// 轮询子进程退出。PTY 读取看不到退出码，只有 `try_wait` 能给出。
struct ExitWatcher {
    instance_id: String,
    shell_id: String,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    buffer: Arc<Mutex<OutputBuffer>>,
    closed: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<u32>>>,
    events: mpsc::Sender<DriverNotification>,
}

impl ExitWatcher {
    fn start(self) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(150));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let observed = lock(&self.child).try_wait();
                let exit_code = match observed {
                    Ok(Some(status)) => Some(status.exit_code()),
                    Ok(None) => continue,
                    // 进程句柄失效时按未知结束处理，不假装成功。
                    Err(_) => None,
                };
                *lock(&self.exit_code) = exit_code;
                if end_stream(&self.buffer, &self.closed) {
                    let event = exit_event(&self.shell_id, &self.instance_id, exit_code);
                    let _ = self.events.send(event).await;
                }
                return;
            }
        });
    }
}
