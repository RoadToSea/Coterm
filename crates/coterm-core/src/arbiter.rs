use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbiterMode {
    /// 默认：人类优先抢占，审核模式
    HumanPreemption,
    /// 全自动运行，无审查直接送达（受熔断器保护）
    Bypass,
}

pub struct InputArbiter {
    mode: ArbiterMode,
    is_agent_running: Arc<AtomicBool>,
    interrupt_sender: mpsc::Sender<()>,
}

impl InputArbiter {
    pub fn new(mode: ArbiterMode) -> (Self, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel(16);
        let arbiter = Self {
            mode,
            is_agent_running: Arc::new(AtomicBool::new(false)),
            interrupt_sender: tx,
        };
        (arbiter, rx)
    }

    pub fn set_mode(&mut self, mode: ArbiterMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> ArbiterMode {
        self.mode
    }

    pub fn set_agent_running(&self, running: bool) {
        self.is_agent_running.store(running, Ordering::SeqCst);
    }

    /// 当人类敲下键盘按键时调用
    pub async fn on_human_input(&self) {
        if self.mode == ArbiterMode::HumanPreemption && self.is_agent_running.load(Ordering::SeqCst) {
            warn!("人类键盘输入介入，立即触发抢占挂起 Agent 序列");
            self.is_agent_running.store(false, Ordering::SeqCst);
            let _ = self.interrupt_sender.try_send(());
        }
    }
}
