//! 可信调用上下文与操作提交参数。

use crate::model::{operation::OperationSource, DriverId};
use serde_json::Value;

/// source 由可信的 UI/MCP 适配层构造，不从领域 args 或 Tauri 参数反序列化。
#[derive(Clone)]
pub struct CallerContext {
    pub source: OperationSource,
    pub session_id: String,
    /// Trusted route authority for driver-scoped operations; never taken from args.
    pub driver_id: Option<DriverId>,
}

/// 外部调用不分配 operation_id，也不自行声明 effect；基座从目录读取。
pub struct SubmitOperation {
    pub driver_id: DriverId,
    pub driver_instance_id: Option<String>,
    pub operation: String,
    pub args: Value,
    /// None=默认等待，Some(0)=立即返回；仅影响本次等待。
    pub wait_timeout_ms: Option<u64>,
}
