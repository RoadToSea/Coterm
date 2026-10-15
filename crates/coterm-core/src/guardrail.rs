use regex::Regex;

pub struct SafetyGuardrail {
    enabled: bool,
    blacklists: Vec<Regex>,
}

impl Default for SafetyGuardrail {
    fn default() -> Self {
        Self::new(true)
    }
}

impl SafetyGuardrail {
    pub fn new(enabled: bool) -> Self {
        let default_patterns = vec![
            r"rm\s+-rf\s+/\S*",
            r"rm\s+-rf\s+/\s*$",
            r"mkfs(\.\w+)?\s+",
            r":\(\)\{\s*:\|:&\s*\};:",
            r"dd\s+if=.*of=/dev/([sh]d[a-z]|nvme)",
            r">\s*/dev/([sh]d[a-z]|nvme)",
            r"chmod\s+-R\s+777\s+/\s*$",
            r"reboot(\s+|$)",
            r"shutdown(\s+|$)",
            r"poweroff(\s+|$)",
        ];

        let blacklists = default_patterns
            .into_iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();

        Self {
            enabled,
            blacklists,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 校验是否包含高危命令模式
    pub fn check_risk(&self, command: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }

        for re in &self.blacklists {
            if re.is_match(command.trim()) {
                return Some(format!("命中高危命令熔断规则: `{}`", re.as_str()));
            }
        }
        None
    }
}
