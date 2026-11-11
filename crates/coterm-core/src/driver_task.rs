//! 进程内驱动任务。请求串行执行，通知独立消费，所有队列都有上限。

use coterm_proto::runtime::{
    backend::{dispatch, DriverBackend},
    lifecycle::HelloResult,
    message::{DriverNotification, HostCommand},
    wire::MAX_CONTROL_BYTES,
    ErrorCategory, RuntimeError,
};
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch, Mutex},
    task::AbortHandle,
};

const QUEUE_CAPACITY: usize = 64;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

struct Request {
    command: HostCommand,
    reply: oneshot::Sender<Result<Value, RuntimeError>>,
}

/// 一个驱动后端的运行句柄；丢弃最后一个句柄会取消其任务并释放后端。
pub struct DriverTask {
    requests: mpsc::Sender<Request>,
    completions: Mutex<Option<mpsc::Receiver<DriverNotification>>>,
    pub events: broadcast::Sender<DriverNotification>,
    limit: Arc<AtomicUsize>,
    abort: AbortHandle,
    closed: watch::Receiver<bool>,
}

impl DriverTask {
    /// 在当前 Tokio 运行时调度后端；后端构造和请求不得执行阻塞 IO。
    /// 无可用运行时属于调用错误；异步 panic、通知溢出会关闭此驱动任务。
    pub fn spawn<B: DriverBackend + 'static>(mut backend: B) -> Self {
        let (requests, input) = mpsc::channel::<Request>(QUEUE_CAPACITY);
        let (completed, completions) = mpsc::channel(QUEUE_CAPACITY);
        let (events, _) = broadcast::channel(QUEUE_CAPACITY);
        let (closed_tx, closed) = watch::channel(false);
        let limit = Arc::new(AtomicUsize::new(MAX_CONTROL_BYTES));
        let notifications = backend.take_notifications();
        let (negotiated_tx, negotiated_rx) = watch::channel::<Option<Arc<HelloResult>>>(None);
        let worker = tokio::spawn(run_backend(backend, input, negotiated_tx));
        let abort = worker.abort_handle();
        let event_tx = events.clone();
        let notification_limit = limit.clone();
        tokio::spawn(forward_notifications(
            worker,
            notifications,
            completed,
            event_tx,
            notification_limit,
            negotiated_rx,
            closed_tx,
        ));
        Self {
            requests,
            completions: Mutex::new(Some(completions)),
            events,
            limit,
            abort,
            closed,
        }
    }

    /// 提交一次类型化请求；队列满、任务关闭、超时或数据超限均返回结构化错误。
    /// 超时会终止整个驱动任务，避免未知执行结果被重试。
    pub async fn request(&self, command: HostCommand) -> Result<Value, RuntimeError> {
        if !within_limit(&command, self.limit.load(Ordering::Acquire)) {
            return Err(failure("driver_control_too_large"));
        }
        let (reply, result) = oneshot::channel();
        self.requests
            .try_send(Request { command, reply })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => failure("driver_request_queue_full"),
                mpsc::error::TrySendError::Closed(_) => failure("driver_connection_closed"),
            })?;
        match tokio::time::timeout(REQUEST_TIMEOUT, result).await {
            Ok(Ok(result)) => {
                if !within_limit(&result, self.limit.load(Ordering::Acquire)) {
                    self.close().await?;
                    return Err(failure("driver_control_too_large"));
                }
                result
            }
            Ok(Err(_)) => Err(failure("driver_connection_closed")),
            Err(_) => {
                self.close().await?;
                Err(failure("driver_request_timed_out"))
            }
        }
    }

    /// 设置能力协商后的单次数据上限（字节）；不改变队列容量。
    pub fn set_control_limit(&self, bytes: u32) {
        self.limit.store(bytes as usize, Ordering::Release);
    }

    /// 完成通知只能由一个宿主任务消费；重复调用返回 None。
    pub async fn take_completions(&self) -> Option<mpsc::Receiver<DriverNotification>> {
        self.completions.lock().await.take()
    }

    /// 取消后端并等待其析构完成；可重复调用。优雅停机由宿主先提交 Shutdown。
    pub async fn close(&self) -> Result<(), RuntimeError> {
        self.abort.abort();
        let mut closed = self.closed.clone();
        let _ = closed.wait_for(|value| *value).await;
        Ok(())
    }
}

async fn run_backend<B: DriverBackend>(
    mut backend: B,
    mut input: mpsc::Receiver<Request>,
    negotiated_tx: watch::Sender<Option<Arc<HelloResult>>>,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut state: Option<Arc<HelloResult>> = None;
    loop {
        tokio::select! {
            request = input.recv() => {
                let Some(request) = request else { break; };
                if request.reply.is_closed() { continue; }
                let shutdown = matches!(request.command, HostCommand::Shutdown);
                let hello = match &request.command { HostCommand::Hello(hello) => Some(hello.clone()), _ => None };
                let mut result = dispatch(&mut backend, request.command, state.as_deref()).await;
                if let (Some(hello), Ok(value)) = (hello, &result) {
                    match serde_json::from_value::<HelloResult>(value.clone()).map_err(|_| failure("driver_handshake_invalid"))
                        .and_then(|accepted| { hello.validate_result(&accepted)?; Ok(accepted) }) {
                        Ok(accepted) => { state = Some(Arc::new(accepted)); negotiated_tx.send_replace(state.clone()); }
                        Err(error) => result = Err(error),
                    }
                }
                let _ = request.reply.send(result);
                if shutdown { break; }
            }
            _ = tick.tick() => backend.tick(),
        }
    }
}

async fn forward_notifications(
    mut worker: tokio::task::JoinHandle<()>,
    mut notifications: Option<mpsc::Receiver<DriverNotification>>,
    completed: mpsc::Sender<DriverNotification>,
    event_tx: broadcast::Sender<DriverNotification>,
    notification_limit: Arc<AtomicUsize>,
    negotiated_rx: watch::Receiver<Option<Arc<HelloResult>>>,
    closed_tx: watch::Sender<bool>,
) {
    loop {
        tokio::select! {
            _ = &mut worker => break,
            event = async {
                match notifications.as_mut() {
                    Some(receiver) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(event) = event else { notifications = None; continue; };
                let feature = match &event { DriverNotification::Event { .. } => "driver-events", _ => "async-operations" };
                let permitted = negotiated_rx.borrow().as_ref().is_some_and(|state| state.enabled_features.iter().any(|enabled| enabled == feature));
                if !permitted || !within_limit(&event, notification_limit.load(Ordering::Acquire)) {
                    worker.abort();
                    let _ = worker.await;
                    break;
                }
                match event {
                    event @ DriverNotification::Event { .. } => { let _ = event_tx.send(event); }
                    event => if completed.try_send(event).is_err() {
                        worker.abort();
                        let _ = worker.await;
                        break;
                    },
                }
            }
        }
    }
    drop(completed);
    let _ = event_tx.send(DriverNotification::Event {
        driver_instance_id: String::new(),
        name: "runtime.closed".into(),
        payload: Value::Null,
    });
    let _ = closed_tx.send(true);
}

impl Drop for DriverTask {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

fn failure(code: &str) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Transport,
        code: code.into(),
        message: code.replace('_', " "),
        details: None,
    }
}

// Count without allocating a second copy of potentially sensitive request/output data.
fn within_limit(value: &impl serde::Serialize, limit: usize) -> bool {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.bytes = self.bytes.saturating_add(data.len());
            if self.bytes > self.limit {
                return Err(std::io::Error::other("data too large"));
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter { bytes: 0, limit }, value).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coterm_proto::runtime::lifecycle::{HelloResult, RuntimeHello};
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    struct BlockingBackend(Arc<AtomicBool>);
    impl Drop for BlockingBackend {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    impl DriverBackend for BlockingBackend {
        async fn request(&mut self, command: HostCommand) -> Result<Value, RuntimeError> {
            match command {
                HostCommand::Hello(hello) => Ok(serde_json::to_value(HelloResult {
                    driver_id: "test".into(),
                    driver_version: "0.1.0".into(),
                    interface_minor: 0,
                    enabled_features: vec![],
                    limits: hello.control_limits(),
                    operations: vec![],
                    mcp_tools: vec![],
                })
                .map_err(|_| failure("encode_failed"))?),
                _ => std::future::pending().await,
            }
        }
        fn take_notifications(&mut self) -> Option<mpsc::Receiver<DriverNotification>> {
            None
        }
    }

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    async fn started() -> TestResult<(Arc<DriverTask>, Arc<AtomicBool>)> {
        let dropped = Arc::new(AtomicBool::new(false));
        let task = Arc::new(DriverTask::spawn(BlockingBackend(dropped.clone())));
        let hello: RuntimeHello = serde_json::from_value(json!({
            "interfaceMajor":2,"interfaceMinor":0,"requiredFeatures":[],"optionalFeatures":[],
            "limits":{"maxControlBytes":1048576,"maxDataChunkBytes":65536,"initialWindow":{"chunks":16,"bytes":1048576}}
        }))?;
        assert_eq!(
            task.request(HostCommand::DestroyInstance {
                driver_instance_id: "early".into()
            })
            .await
            .unwrap_err()
            .code,
            "runtime_not_initialized"
        );
        task.request(HostCommand::Hello(hello.clone())).await?;
        assert_eq!(
            task.request(HostCommand::Hello(hello))
                .await
                .unwrap_err()
                .code,
            "runtime_already_initialized"
        );
        Ok((task, dropped))
    }

    #[tokio::test]
    async fn saturated_requests_are_bounded_and_close_releases_a_blocked_backend() -> TestResult {
        let (task, dropped) = started().await?;
        let mut calls = Vec::new();
        for _ in 0..QUEUE_CAPACITY + 16 {
            let task = task.clone();
            calls.push(tokio::spawn(async move {
                task.request(HostCommand::DestroyInstance {
                    driver_instance_id: "blocked".into(),
                })
                .await
            }));
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while !calls.iter().any(|call| call.is_finished()) {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        tokio::time::timeout(Duration::from_secs(1), task.close()).await??;
        assert!(dropped.load(Ordering::SeqCst));
        let mut full = 0;
        for call in calls {
            let error = call.await?.unwrap_err();
            if error.code == "driver_request_queue_full" {
                full += 1;
            } else {
                assert_eq!(error.code, "driver_connection_closed");
            }
        }
        assert!(full > 0);
        task.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn dropping_last_handle_cancels_its_backend() -> TestResult {
        let (task, dropped) = started().await?;
        drop(task);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }
}
