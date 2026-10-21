//! 本地终端插件的领域后端。目前只负责实例的创建与销毁；PTY 会话尚未实现，
//! 所以所有子命令操作都由目录外的分派明确拒绝，而不是返回假的成功。
use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use coterm_proto::runtime::{
    lifecycle::{CreateInstance, HelloResult, RuntimeHello},
    message::{HostCommand, PluginNotification},
    operation::{
        ApproveProposal, DispatchMode, InvokeRequest, OperationDescriptor, OperationEffect,
        OperationScope,
    },
    ErrorCategory, RuntimeError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PLUGIN_ID: &str = "coterm-plugin-local";

const MAX_INSTANCES: usize = 32;

/// 插件声明的功能集合；必须覆盖 manifest 的 `runtime.requires`。
const ENABLED_FEATURES: &[&str] = &[
    "instances",
    "backend-calls",
    "operation-catalog",
    "plugin-events",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_shell")]
    pub shell: String,
    #[serde(default)]
    pub cwd: String,
}

fn default_shell() -> String {
    if cfg!(windows) {
        "powershell.exe".to_owned()
    } else {
        "/bin/bash".to_owned()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: default_shell(),
            cwd: String::new(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        let shell = self.shell.trim();
        if shell.is_empty() || shell.len() > 1024 || shell.chars().any(char::is_control) {
            bail!("Invalid local shell path");
        }
        if self.cwd.len() > 4096 || self.cwd.chars().any(char::is_control) {
            bail!("Invalid local working directory");
        }
        Ok(())
    }

    fn label(&self) -> String {
        let shell = self.shell.trim();
        let name = shell.rsplit(['/', '\\']).next().unwrap_or(shell);
        if self.cwd.trim().is_empty() {
            name.to_owned()
        } else {
            format!("{name} @ {}", self.cwd.trim())
        }
    }
}

pub struct Backend {
    notifications: Option<tokio::sync::mpsc::Receiver<PluginNotification>>,
    instances: HashMap<String, Config>,
    proposals: HashMap<String, Proposal>,
}

struct Proposal {
    instance_id: String,
    operation_id: String,
}

impl Default for Backend {
    fn default() -> Self {
        // 还没有 PTY 会话可以发事件；保留一个挂起的接收端以满足传输层契约。
        let (_events, notifications) = tokio::sync::mpsc::channel(128);
        Self {
            notifications: Some(notifications),
            instances: HashMap::new(),
            proposals: HashMap::new(),
        }
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| anyhow!("Invalid request parameters"))
}

pub fn catalog() -> Vec<OperationDescriptor> {
    vec![OperationDescriptor {
        name: "local.plugin_info".to_owned(),
        scope: OperationScope::Plugin,
        effect: OperationEffect::Read,
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
        output_schema: None,
        sensitive_input: false,
        transient: false,
    }]
}

/// 本地终端还没有可暴露给 Agent 的 MCP 工具：没有 PTY 会话可驱动。
pub fn mcp_tools() -> Vec<coterm_proto::runtime::lifecycle::McpToolRegistration> {
    Vec::new()
}

fn unsupported(code: &str, message: &str) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Unsupported,
        code: code.to_owned(),
        message: message.to_owned(),
        details: None,
    }
}

impl coterm_proto::runtime::transport::PluginBackend for Backend {
    async fn request(&mut self, command: HostCommand) -> Result<Value, RuntimeError> {
        let (method, params) = match command {
            HostCommand::Hello(hello) => ("initialize", serde_json::to_value(hello)),
            HostCommand::CreateInstance(create) => ("create_instance", serde_json::to_value(create)),
            HostCommand::DestroyInstance { plugin_instance_id } => (
                "destroy_instance",
                Ok(json!({ "pluginInstanceId": plugin_instance_id })),
            ),
            HostCommand::Invoke(request) => ("invoke", serde_json::to_value(request)),
            HostCommand::Approve(request) => ("approve", serde_json::to_value(request)),
            HostCommand::Reject {
                operation_id,
                proposal_id,
                reason,
            } => (
                "reject",
                Ok(json!({
                    "operationId": operation_id,
                    "proposalId": proposal_id,
                    "reason": reason,
                })),
            ),
            HostCommand::Cancel(target) => ("cancel", serde_json::to_value(target)),
            HostCommand::InteractionResult(result) => {
                ("interaction_result", serde_json::to_value(result))
            }
            HostCommand::ProvideSecret(secret) => {
                ("provide_secret", serde_json::to_value(secret))
            }
            HostCommand::Shutdown => ("shutdown", Ok(Value::Null)),
            HostCommand::StreamSubscribe(_)
            | HostCommand::StreamUnsubscribe { .. }
            | HostCommand::StreamCredit { .. } => {
                return Err(unsupported(
                    "local_stream_unsupported",
                    "Local terminal plugin does not support stream subscriptions",
                ))
            }
        };
        let params = params.map_err(|error| RuntimeError {
            category: ErrorCategory::InvalidRequest,
            code: "local_request_encode_failed".to_owned(),
            message: error.to_string(),
            details: None,
        })?;
        self.domain(method, params).await.map_err(|error| RuntimeError {
            category: ErrorCategory::Domain,
            code: "local_domain_error".to_owned(),
            message: error.to_string(),
            details: None,
        })
    }

    fn take_notifications(&mut self) -> Option<tokio::sync::mpsc::Receiver<PluginNotification>> {
        self.notifications.take()
    }
}

impl Backend {
    async fn domain(&mut self, method: &str, params: Value) -> Result<Value> {
        match method {
            "initialize" => {
                let hello: RuntimeHello = decode(params)?;
                if hello.interface_major != 1 {
                    bail!("Unsupported runtime interface");
                }
                if hello
                    .required_features
                    .iter()
                    .any(|feature| !ENABLED_FEATURES.contains(&feature.as_str()))
                {
                    bail!("Unsupported required runtime feature");
                }
                Ok(serde_json::to_value(HelloResult {
                    plugin_id: PLUGIN_ID.to_owned(),
                    plugin_version: env!("CARGO_PKG_VERSION").to_owned(),
                    interface_minor: 0,
                    enabled_features: ENABLED_FEATURES
                        .iter()
                        .map(|feature| (*feature).to_owned())
                        .collect(),
                    limits: hello.limits,
                    operations: catalog(),
                    mcp_tools: mcp_tools(),
                })?)
            }
            "create_instance" => {
                let create: CreateInstance = decode(params)?;
                if create.instance.plugin_id != PLUGIN_ID
                    || self
                        .instances
                        .contains_key(&create.instance.plugin_instance_id)
                    || self.instances.len() >= MAX_INSTANCES
                {
                    bail!("Invalid or duplicate local instance (limit: {MAX_INSTANCES})");
                }
                let config: Config = decode(create.config)?;
                config.validate()?;
                let label = config.label();
                self.instances
                    .insert(create.instance.plugin_instance_id, config);
                Ok(json!({"ok":true,"label":label}))
            }
            "destroy_instance" => {
                let id = string(&params, "pluginInstanceId")?;
                if self.instances.remove(id).is_none() {
                    bail!("Local instance not found");
                }
                self.proposals
                    .retain(|_, proposal| proposal.instance_id != id);
                Ok(json!({"ok":true}))
            }
            "invoke" => self.invoke(decode(params)?).await,
            "cancel" => {
                // 没有长事务可取消；明确回报而不是假装已经停止。
                Ok(json!({"outcome":"unsupported"}))
            }
            "approve" => {
                let approve: ApproveProposal = decode(params)?;
                let proposal = self
                    .proposals
                    .remove(&approve.proposal_id)
                    .ok_or_else(|| anyhow!("Local proposal not found"))?;
                if Some(&proposal.instance_id) != approve.plugin_instance_id.as_ref()
                    || proposal.operation_id != approve.operation_id
                {
                    bail!("Local proposal target mismatch");
                }
                bail!("Local terminal has no approved operations")
            }
            "reject" => {
                let id = string(&params, "proposalId")?;
                let proposal = self
                    .proposals
                    .get(id)
                    .ok_or_else(|| anyhow!("Local proposal not found"))?;
                if proposal.operation_id != string(&params, "operationId")? {
                    bail!("Local proposal mismatch");
                }
                self.proposals.remove(id);
                Ok(json!({"ok":true}))
            }
            "shutdown" => {
                self.instances.clear();
                self.proposals.clear();
                Ok(json!({"ok":true}))
            }
            // 插件未声明 secret-delivery / deferred-interactions，基座不会发送这两类命令。
            "interaction_result" => bail!("Local terminal accepts no interactions"),
            "provide_secret" => bail!("Local terminal accepts no secrets"),
            _ => bail!("Unsupported local plugin method"),
        }
    }

    async fn invoke(&mut self, request: InvokeRequest) -> Result<Value> {
        if request.operation != "local.plugin_info" {
            bail!("Unknown local operation");
        }
        if request.plugin_instance_id.is_some() || request.effect != OperationEffect::Read {
            bail!("Local plugin operation context mismatch");
        }
        if let DispatchMode::PrepareProposal { .. } = request.dispatch {
            bail!("Read-only local operation cannot be prepared as a proposal");
        }
        Ok(json!({"pluginId":PLUGIN_ID,"instanceCount":self.instances.len()}))
    }
}

fn string<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing or invalid {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_rejects_broken_paths() {
        let config: Config = serde_json::from_value(json!({})).unwrap();
        assert!(!config.shell.trim().is_empty());
        assert!(config.validate().is_ok());
        // 未知键暴露拼写错误，而不是被静默忽略。
        assert!(serde_json::from_value::<Config>(json!({"shell":"bash","workingDir":"/tmp"})).is_err());
        for shell in ["", "  ", "bash\nrm -rf /"] {
            let config = Config {
                shell: shell.to_owned(),
                cwd: String::new(),
            };
            assert!(config.validate().is_err(), "{shell:?} must be rejected");
        }
        let config = Config {
            shell: "bash".to_owned(),
            cwd: "/tmp\n".to_owned(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn labels_use_the_shell_file_name_and_working_directory() {
        let config = Config {
            shell: "/bin/bash".to_owned(),
            cwd: "/srv".to_owned(),
        };
        assert_eq!(config.label(), "bash @ /srv");
        let config = Config {
            shell: "powershell.exe".to_owned(),
            cwd: "  ".to_owned(),
        };
        assert_eq!(config.label(), "powershell.exe");
    }

    #[test]
    fn catalog_only_exposes_plugin_scoped_info() {
        let catalog = catalog();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].name, "local.plugin_info");
        assert_eq!(catalog[0].scope, OperationScope::Plugin);
        assert_eq!(catalog[0].effect, OperationEffect::Read);
        assert!(mcp_tools().is_empty());
    }
}
