//! 实例、操作、审批交互和结构化错误的共享数据模型。

pub mod interaction;
pub mod lifecycle;
pub mod manifest;
pub mod operation;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type DriverId = String;
pub type DriverInstanceId = String;
pub type OperationId = String;
pub type ProposalId = String;
pub type InteractionId = String;

/// 公共错误分类与驱动领域错误码分开，调用方不解析 message。
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
