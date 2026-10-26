//! 基座与插件的 v1.0 JSON 控制契约。wire 是唯一线缆编解码入口。
//! stream 与 PluginEvent 的扩展事件为预留类型，不属于 v1.0 线缆契约。
//! 旧版根模块中的终端类型仍供原型使用，不属于本组接口。

pub mod backend;
pub mod interaction;
pub mod lifecycle;
pub mod manifest;
pub mod message;
pub mod operation;
pub mod stream;
pub mod transport;
pub mod wire;

use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type RuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RuntimeError>> + Send + 'a>>;

pub type PluginId = String;
pub type PluginInstanceId = String;
pub type RequestId = String;
pub type OperationId = String;
pub type ProposalId = String;
pub type StreamId = String;
pub type SubscriptionId = String;
pub type InteractionId = String;

/// 公共错误分类与插件领域错误码分开，调用方不解析 message。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    InvalidRequest,
    NotFound,
    Unsupported,
    Conflict,
    Interaction,
    Transport,
    Domain,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeError {
    pub category: ErrorCategory,
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RuntimeError {}
