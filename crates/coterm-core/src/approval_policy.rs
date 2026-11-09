//! User-owned command approval rules. Drivers declare the text field in their schema.
use coterm_proto::runtime::{
    operation::{OperationDescriptor, OperationEffect, OperationSource},
    ErrorCategory, RuntimeError,
};
use regex::{RegexSet, RegexSetBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TEXT_POINTER: &str = "x-coterm-approval-text";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    #[default]
    AllowMatches,
    ApproveMatches,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalPolicy {
    pub mode: ApprovalMode,
    pub patterns: Vec<String>,
}

/// Independently saved rules referenced by stable ID; host storage validates names
/// and references, and `policy().compile()` validates matcher limits and syntax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalRuleGroup {
    pub id: String,
    pub name: String,
    pub mode: ApprovalMode,
    pub patterns: Vec<String>,
}

impl ApprovalRuleGroup {
    /// Returns this group's matcher input; validation is performed by `compile`.
    pub fn policy(&self) -> ApprovalPolicy {
        ApprovalPolicy {
            mode: self.mode,
            patterns: self.patterns.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CompiledApprovalPolicy {
    policy: ApprovalPolicy,
    patterns: RegexSet,
}

/// Trusted submission data after instance routing and schema validation.
/// The descriptor comes from the driver catalog, never from submitted arguments.
pub struct ApprovalContext<'a> {
    pub source: OperationSource,
    pub descriptor: &'a OperationDescriptor,
    pub args: &'a Value,
}

/// One live instance's rules and user-controlled bypass, restored by the host when saved.
/// Default instances require approval for every Agent commit.
#[derive(Default)]
pub struct InstanceApproval {
    policy: Option<CompiledApprovalPolicy>,
    bypass: bool,
}

impl InstanceApproval {
    /// Returns whether this validated submission needs a proposal; never dispatches it.
    pub fn requires_approval(&self, context: &ApprovalContext<'_>) -> bool {
        context.source == OperationSource::Agent
            && context.descriptor.effect == OperationEffect::Commit
            && !self.bypass
            && !self
                .policy
                .as_ref()
                .is_some_and(|policy| policy.allows(context.descriptor, context.args))
    }

    /// Replaces this instance's rules; invalid/oversized rules leave them unchanged.
    pub fn set_policy(&mut self, policy: ApprovalPolicy) -> Result<(), RuntimeError> {
        self.policy = Some(policy.compile()?);
        Ok(())
    }

    pub(crate) fn set_compiled_policy(&mut self, policy: CompiledApprovalPolicy) {
        self.policy = Some(policy);
    }

    /// Returns this instance's configured rules, or the approval-required default.
    pub fn policy(&self) -> ApprovalPolicy {
        self.policy
            .as_ref()
            .map(CompiledApprovalPolicy::policy)
            .unwrap_or_default()
    }

    /// Changes only this instance; callers must be trusted human settings handlers.
    pub fn set_bypass(&mut self, enabled: bool) {
        self.bypass = enabled;
    }

    /// Returns the current instance's bypass state.
    pub fn bypass_enabled(&self) -> bool {
        self.bypass
    }
}

impl ApprovalPolicy {
    pub fn compile(&self) -> Result<CompiledApprovalPolicy, RuntimeError> {
        if self.patterns.len() > 128
            || self
                .patterns
                .iter()
                .any(|pattern| pattern.trim().is_empty() || pattern.len() > 2048)
            || self.patterns.iter().map(String::len).sum::<usize>() > 16 * 1024
        {
            return Err(invalid(
                "Use up to 128 nonempty rules, 2048 bytes each and 16384 bytes total.",
            ));
        }
        let patterns = RegexSetBuilder::new(&self.patterns)
            .size_limit(2 * 1024 * 1024)
            .dfa_size_limit(2 * 1024 * 1024)
            .build()
            .map_err(|error| invalid(&format!("Invalid regular expression: {error}")))?;
        Ok(CompiledApprovalPolicy {
            policy: self.clone(),
            patterns,
        })
    }
}

impl CompiledApprovalPolicy {
    pub fn policy(&self) -> ApprovalPolicy {
        self.policy.clone()
    }

    /// Uses the same matcher for the settings preview and real dispatch.
    pub fn requires_approval(&self, commands: &[&str]) -> bool {
        if commands.is_empty()
            || commands.len() > 128
            || commands.iter().any(|text| text.is_empty())
            || commands.iter().map(|text| text.len()).sum::<usize>() > 1024 * 1024
        {
            return true;
        }
        match self.policy.mode {
            ApprovalMode::AllowMatches => !commands.iter().all(|text| self.patterns.is_match(text)),
            ApprovalMode::ApproveMatches => {
                commands.iter().any(|text| self.patterns.is_match(text))
            }
        }
    }

    /// Only the declared command field is inspected; it is never recorded or exposed here.
    pub fn allows(&self, descriptor: &OperationDescriptor, args: &Value) -> bool {
        let Some(pointer) = descriptor
            .input_schema
            .get(TEXT_POINTER)
            .and_then(Value::as_str)
            .filter(|pointer| pointer.starts_with('/') && pointer.len() <= 256)
        else {
            return false;
        };
        let Some(value) = args.pointer(pointer) else {
            return false;
        };
        let commands = match value {
            Value::String(text) => vec![text.as_str()],
            Value::Array(texts) => {
                if texts.is_empty() || texts.len() > 128 {
                    return false;
                }
                let Some(texts) = texts.iter().map(Value::as_str).collect::<Option<Vec<_>>>()
                else {
                    return false;
                };
                texts
            }
            _ => return false,
        };
        !self.requires_approval(&commands)
    }
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::InvalidRequest,
        code: "approval_rule_invalid".into(),
        message: message.into(),
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_modes_check_each_whole_payload_without_splitting_shell_syntax(
    ) -> Result<(), RuntimeError> {
        let allow = ApprovalPolicy {
            mode: ApprovalMode::AllowMatches,
            patterns: vec![
                r"\Als(?:[ \t]+[^;&|`$<>\r\n]+)?\z".into(),
                r"\Acd /tmp\z".into(),
            ],
        }
        .compile()?;
        assert!(!allow.requires_approval(&["ls -la", "cd /tmp"]));
        for script in ["ls; rm -rf /tmp/x", "ls\nrm x", "ls $(rm x)", "pwd", ""] {
            assert!(allow.requires_approval(&[script]), "{script:?}");
        }
        assert!(allow.requires_approval(&["ls", "rm x"]));
        let reverse = ApprovalPolicy {
            mode: ApprovalMode::ApproveMatches,
            patterns: vec![r"\brm\b".into()],
        }
        .compile()?;
        assert!(!reverse.requires_approval(&["ls", "cd /tmp"]));
        assert!(reverse.requires_approval(&["ls", "sudo rm -rf /tmp/x"]));
        assert!(reverse.requires_approval(&[]));
        Ok(())
    }

    #[test]
    fn invalid_or_excessive_rules_are_rejected() {
        for patterns in [
            vec!["[".into()],
            vec!["".into()],
            vec!["a".repeat(2049)],
            vec!["a".into(); 129],
        ] {
            assert!(ApprovalPolicy {
                mode: ApprovalMode::AllowMatches,
                patterns
            }
            .compile()
            .is_err());
        }
    }

    #[test]
    fn explicit_command_opt_in_is_required_even_for_redacted_operations() -> Result<(), RuntimeError>
    {
        use coterm_proto::runtime::operation::{OperationEffect, OperationScope};
        let policy = ApprovalPolicy {
            mode: ApprovalMode::ApproveMatches,
            patterns: vec![],
        }
        .compile()?;
        let mut descriptor = OperationDescriptor {
            name: "test.command".into(),
            scope: OperationScope::Instance,
            effect: OperationEffect::Commit,
            input_schema: serde_json::json!({"type":"object"}),
            output_schema: None,
            sensitive_input: false,
            transient: false,
        };
        let args = serde_json::json!({"commands":["ls"]});
        assert!(!policy.allows(&descriptor, &args));
        descriptor.input_schema[TEXT_POINTER] = Value::String("/commands".into());
        assert!(policy.allows(&descriptor, &args));
        for args in [
            serde_json::json!({}),
            serde_json::json!({"commands":[]}),
            serde_json::json!({"commands":["ls", 3]}),
        ] {
            assert!(!policy.allows(&descriptor, &args));
        }
        descriptor.sensitive_input = true;
        assert!(policy.allows(&descriptor, &args));
        descriptor.input_schema[TEXT_POINTER] = Value::Null;
        assert!(!policy.allows(&descriptor, &args));
        Ok(())
    }
}
