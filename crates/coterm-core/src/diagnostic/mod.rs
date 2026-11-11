//! 显式诊断工具的 JSON-RPC 适配，不参与桌面驱动派发。

pub mod lifecycle;
pub mod message;
pub mod transport;
pub mod wire;

use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 基础规范 (跨进程通信)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// 显式方法名适配器；仅诊断工具与契约测试使用。
pub struct Backend<B>(pub B);

impl<B: Default> Default for Backend<B> {
    fn default() -> Self {
        Self(B::default())
    }
}

impl<B> std::ops::Deref for Backend<B> {
    type Target = B;
    fn deref(&self) -> &B {
        &self.0
    }
}

impl<B> std::ops::DerefMut for Backend<B> {
    fn deref_mut(&mut self) -> &mut B {
        &mut self.0
    }
}

impl<B: crate::driver::DriverBackend> Backend<B> {
    /// 解析诊断方法和参数并调用真实后端；无效方法/参数返回 RuntimeError。
    pub async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let frame = serde_json::to_vec(&serde_json::json!({"jsonrpc":"2.0", "id":"diagnostic", "method":method, "params":params}))
            .map_err(|_| crate::driver::rejected(crate::model::ErrorCategory::InvalidRequest, "diagnostic_encode_failed"))?;
        let message = crate::diagnostic::wire::decode_request(&frame).map_err(|_| {
            crate::driver::rejected(
                crate::model::ErrorCategory::InvalidRequest,
                "diagnostic_request_invalid",
            )
        })?;
        Ok(match message.command {
            message::Command::Hello(hello) => serde_json::to_value(hello.accept(B::definition())?)?,
            message::Command::Driver(command) => self.0.request(command).await?,
        })
    }
}

impl<B: crate::driver::DriverBackend> crate::driver::DriverBackend for Backend<B> {
    fn definition() -> crate::driver::DriverDefinition {
        B::definition()
    }
    async fn request(
        &mut self,
        command: crate::driver::HostCommand,
    ) -> Result<serde_json::Value, crate::model::RuntimeError> {
        self.0.request(command).await
    }
    fn tick(&mut self) {
        self.0.tick();
    }
    fn take_notifications(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<crate::driver::DriverNotification>> {
        self.0.take_notifications()
    }
}
