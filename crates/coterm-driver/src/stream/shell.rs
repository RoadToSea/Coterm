//! Observed byte-stream Shell boundaries; a prompt never proves an exit status.
use std::collections::VecDeque;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::stream::payload::bounded_output_text;

const RECORDS: usize = 32;
const OUTPUT_BYTES: usize = 16 * 1024;
const LINE_BYTES: usize = 4096;
const RESULT_BYTES: usize = 12 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Stream,
    Shell,
}

/// Validates a literal visible prompt. Empty, oversized and control-bearing values fail.
pub fn validate_prompt(prompt: &str) -> Result<()> {
    if prompt.trim().is_empty() || prompt.len() > 256 || prompt.chars().any(char::is_control) {
        bail!("Shell prompt must contain 1 to 256 UTF-8 bytes without control characters");
    }
    Ok(())
}

struct Record {
    operation_id: String,
    command_index: usize,
    stdin: String,
    stdout: String,
    status: &'static str,
    prompt: String,
    truncated: bool,
    observed_input: bool,
}

impl Record {
    fn value(&self, limit: usize) -> Value {
        let (input, _) = bounded_output_text(&self.stdin, limit / 3, false);
        let (output, _) = bounded_output_text(&self.stdout, limit * 2 / 3, false);
        json!({
            "operationId": self.operation_id, "commandIndex": self.command_index,
            "stdin": input, "stdout": output, "stderr": "", "streamMode": "merged",
            "status": self.status, "exitCode": null, "boundary": "observed",
            "deviceCompletion": if self.status == "completed" { "prompt_observed" } else { "unknown" },
            "observedInput": self.observed_input,
            "truncated": self.truncated || input.len() < self.stdin.len() || output.len() < self.stdout.len(),
        })
    }
}

#[derive(Default, PartialEq)]
enum Escape {
    #[default]
    Plain,
    Start,
    Csi,
    Osc,
    OscEnd,
}

#[derive(Default)]
pub(crate) struct ShellHistory {
    prompt: Option<String>,
    line: String,
    line_overflow: bool,
    escape: Escape,
    records: VecDeque<Record>,
    active: bool,
    ready: bool,
    echo_prompt: String,
    dropped: u64,
    selected: String,
    selection_truncated: bool,
}

impl ShellHistory {
    pub fn new(mode: Mode, prompt: Option<&str>) -> Self {
        Self {
            prompt: (mode == Mode::Shell).then(|| prompt.unwrap_or("").trim_end().to_owned()),
            ..Self::default()
        }
    }

    /// Changes interpretation without replaying old bytes or discarding completed pairs.
    /// An unfinished observed command becomes unknown; selection and prompt readiness expire.
    pub fn configure(&mut self, mode: Mode, prompt: Option<&str>) {
        let prompt = (mode == Mode::Shell).then(|| prompt.unwrap_or("").trim_end().to_owned());
        if self.prompt == prompt {
            return;
        }
        self.interrupt("unknown");
        self.escape = Escape::Plain;
        self.selected.clear();
        self.selection_truncated = false;
        self.prompt = prompt;
    }

    pub fn enabled(&self) -> bool {
        self.prompt.is_some()
    }

    pub fn ready(&self) -> bool {
        self.ready && !self.active
    }

    /// Starts one explicitly submitted command. No input is captured before dispatch.
    pub fn begin(
        &mut self,
        operation: &str,
        index: usize,
        input: &str,
        prompt: &str,
    ) -> Result<()> {
        if self.enabled()
            && !self.active
            && !self.line_overflow
            && self.line.trim_end() == prompt.trim_end()
        {
            self.ready = true;
        }
        if !self.enabled() || !self.ready() {
            bail!("Shell prompt has not been observed; reach the Shell with human input before submitting commands");
        }
        self.add_record(Record {
            operation_id: operation.into(),
            command_index: index,
            stdin: input.into(),
            stdout: String::new(),
            status: "running",
            prompt: prompt.trim_end().into(),
            truncated: false,
            observed_input: false,
        });
        self.line.clear();
        self.line_overflow = false;
        self.ready = false;
        Ok(())
    }

    fn add_record(&mut self, record: Record) {
        if self.records.len() == RECORDS {
            self.records.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.records.push_back(record);
        self.active = true;
    }

    pub fn feed(&mut self, text: &str) {
        if !self.enabled() {
            return;
        }
        for ch in text.chars() {
            if self.active {
                if let Some(record) = self.records.back_mut() {
                    if record.stdout.len() + ch.len_utf8() <= OUTPUT_BYTES {
                        record.stdout.push(ch);
                    } else {
                        record.truncated = true;
                    }
                }
            }
            if self.consume_escape(ch) {
                continue;
            }
            match ch {
                '\r' | '\n' => {
                    self.observe_line();
                    self.line.clear();
                    self.line_overflow = false;
                }
                '\u{8}' | '\u{7f}' => {
                    self.line.pop();
                }
                ch if !ch.is_control() => {
                    if self.line.len() + ch.len_utf8() <= LINE_BYTES {
                        self.line.push(ch);
                    } else {
                        self.line_overflow = true;
                    }
                }
                _ => {}
            }
        }
        if self.escape == Escape::Plain {
            self.observe_prompt();
        }
    }

    fn consume_escape(&mut self, ch: char) -> bool {
        match self.escape {
            Escape::Plain if ch == '\u{1b}' => self.escape = Escape::Start,
            Escape::Plain => return false,
            Escape::Start => {
                self.escape = match ch {
                    '[' => Escape::Csi,
                    ']' => Escape::Osc,
                    _ => Escape::Plain,
                }
            }
            Escape::Csi if ('@'..='~').contains(&ch) => self.escape = Escape::Plain,
            Escape::Csi => {}
            Escape::Osc if ch == '\u{7}' => self.escape = Escape::Plain,
            Escape::Osc if ch == '\u{1b}' => self.escape = Escape::OscEnd,
            Escape::Osc => {}
            Escape::OscEnd => {
                self.escape = if ch == '\\' {
                    Escape::Plain
                } else {
                    Escape::Osc
                }
            }
        }
        true
    }

    fn observe_line(&mut self) {
        if self.observe_prompt() || self.active || !self.ready || self.line_overflow {
            return;
        }
        // Only the device's echo after a confirmed prompt can identify human input.
        // Raw keys can be passwords and must never be recorded as command sources.
        let Some(input) = self.line.strip_prefix(&self.echo_prompt).map(str::trim) else {
            return;
        };
        if input.is_empty() {
            return;
        }
        self.add_record(Record {
            operation_id: format!("observed-{}", uuid::Uuid::new_v4()),
            command_index: 0,
            stdin: input.to_owned(),
            stdout: String::new(),
            status: "running",
            prompt: self.prompt.clone().unwrap_or_default(),
            truncated: false,
            observed_input: true,
        });
        self.ready = false;
    }

    fn observe_prompt(&mut self) -> bool {
        let expected = if self.active {
            self.records.back().map(|record| record.prompt.as_str())
        } else {
            self.prompt.as_deref()
        };
        if self.line_overflow
            || !expected.is_some_and(|prompt| !prompt.is_empty() && self.line.trim_end() == prompt)
        {
            return false;
        }
        if self.active {
            if let Some(record) = self.records.back_mut() {
                record.status = "completed";
            }
        }
        self.active = false;
        self.ready = true;
        self.echo_prompt = self.line.trim_end().to_owned();
        true
    }

    /// Marks a dispatched command uncertain; future output remains in the raw buffer.
    pub fn interrupt(&mut self, status: &'static str) {
        if self.active {
            if let Some(record) = self.records.back_mut() {
                record.status = status;
            }
        }
        self.active = false;
        self.ready = false;
        self.echo_prompt.clear();
        self.line.clear();
        self.line_overflow = false;
    }

    pub fn interrupt_operation(&mut self, operation: &str, status: &'static str) {
        if self.active
            && self
                .records
                .back()
                .is_some_and(|record| record.operation_id == operation)
        {
            self.interrupt(status);
        }
    }

    pub fn completed(&self, operation: &str, index: usize) -> bool {
        self.records.iter().rev().any(|record| {
            record.operation_id == operation
                && record.command_index == index
                && record.status == "completed"
        })
    }

    pub fn operation_result(&self, operation: &str) -> Value {
        let records: Vec<_> = self
            .records
            .iter()
            .filter(|record| record.operation_id == operation)
            .collect();
        let input = records
            .iter()
            .map(|record| record.stdin.as_str())
            .collect::<String>();
        let output = records
            .iter()
            .map(|record| record.stdout.as_str())
            .collect::<String>();
        let (stdin, _) = bounded_output_text(&input, 2048, false);
        let (stdout, _) = bounded_output_text(&output, 6 * 1024, false);
        json!({"stdin":stdin,"stdout":stdout,"stderr":"","streamMode":"merged","exitCode":null,
            "truncated":stdin.len()<input.len() || stdout.len()<output.len() || records.iter().any(|record| record.truncated) || records.first().is_some_and(|record| record.command_index > 0)})
    }

    pub fn pairs(&self, count: usize) -> Result<Value> {
        if !self.enabled() {
            bail!("Dialogue reads require mode=shell; use serial.read_lines for byte streams");
        }
        if !(1..=32).contains(&count) {
            bail!("Dialogue count must be between 1 and 32");
        }
        let mut limit = 8192;
        loop {
            let mut pairs: Vec<_> = self
                .records
                .iter()
                .rev()
                .take(count)
                .map(|record| record.value(limit))
                .collect();
            let mut omitted = false;
            if limit == 0 {
                while serde_json::to_vec(&pairs)?.len() > RESULT_BYTES - 256 {
                    if pairs.pop().is_none() {
                        break;
                    }
                    omitted = true;
                }
            }
            let truncated = pairs.iter().any(|record| record["truncated"] == true);
            let value =
                json!({"pairs":pairs,"dropped":self.dropped,"truncated":truncated || omitted});
            if serde_json::to_vec(&value)?.len() <= RESULT_BYTES || limit == 0 {
                return Ok(value);
            }
            limit /= 2;
        }
    }

    pub fn set_selection(&mut self, text: &str, truncated: bool) -> Result<()> {
        if !self.enabled() {
            bail!("Selected-line reads require mode=shell");
        }
        if text.len() > 16 * 1024 {
            bail!("Serial selection must not exceed 16384 UTF-8 bytes");
        }
        self.selected = text.into();
        self.selection_truncated = truncated;
        Ok(())
    }

    pub fn selection(&self) -> Result<Value> {
        if !self.enabled() {
            bail!("Selected-line reads require mode=shell; use serial.read_lines for byte streams");
        }
        let (text, _) = bounded_output_text(&self.selected, RESULT_BYTES - 512, false);
        Ok(
            json!({"text":text,"truncated":self.selection_truncated || text.len()<self.selected.len()}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_views_retains_pairs_but_expires_selection_and_readiness() -> Result<()> {
        let mut history = ShellHistory::new(Mode::Shell, Some("root# "));
        history.feed("root# ");
        history.begin("one", 0, "pwd", "root# ")?;
        history.feed("pwd\r\n/root\r\nroot# ");
        assert!(history.completed("one", 0));
        history.set_selection("/root", false)?;
        history.configure(Mode::Stream, Some("root# "));
        assert!(history.pairs(1).is_err());
        assert!(history.selection().is_err());
        history.feed("UART data\r\nroot# ");
        history.configure(Mode::Shell, Some("root# "));
        assert!(!history.ready());
        assert_eq!(history.selection()?["text"], "");
        assert_eq!(history.pairs(2)?["pairs"].as_array().map(Vec::len), Some(1));
        assert!(history.begin("two", 0, "ls", "root# ").is_err());
        history.feed("root# ");
        history.begin("two", 0, "ls", "root# ")?;
        history.configure(Mode::Stream, None);
        history.configure(Mode::Shell, Some("root# "));
        assert_eq!(history.pairs(1)?["pairs"][0]["status"], "unknown");
        Ok(())
    }

    fn history() -> ShellHistory {
        ShellHistory::new(Mode::Shell, Some("root# "))
    }

    #[test]
    fn prompt_matches_whole_visible_line_across_every_packet_split() -> Result<()> {
        let bytes = "echo ok\r\nok 中文\r\n\u{1b}[32mroot#\u{1b}[0m ";
        for split in bytes.char_indices().map(|(index, _)| index) {
            let mut history = history();
            history.feed("root# ");
            history.begin("one", 0, "echo ok", "root# ")?;
            history.feed(&bytes[..split]);
            history.feed(&bytes[split..]);
            assert!(history.completed("one", 0), "split {split}");
            let pair = &history.pairs(2)?["pairs"][0];
            assert_eq!(pair["stdin"], "echo ok");
            assert_eq!(pair["exitCode"], Value::Null);
            assert_eq!(pair["stderr"], "");
            assert_eq!(pair["streamMode"], "merged");
            assert!(pair["stdout"]
                .as_str()
                .is_some_and(|text| text.contains("中文")));
        }
        Ok(())
    }

    #[test]
    fn output_substrings_and_echoed_prompt_arguments_are_not_boundaries() -> Result<()> {
        let mut history = history();
        history.feed("root# ");
        history.begin("one", 0, "echo root#", "root# ")?;
        history.feed("echo root#\r\nlog root#\r\nroot# extra\r\n");
        assert!(!history.completed("one", 0));
        history.feed("root# ");
        assert!(history.completed("one", 0));
        Ok(())
    }

    #[test]
    fn human_input_requires_device_echo_after_a_confirmed_prompt() -> Result<()> {
        let mut history = history();
        history.feed("login: \r\nPassword: \r\n");
        assert!(history.pairs(2)?["pairs"]
            .as_array()
            .is_some_and(Vec::is_empty));
        history.feed("root# ");
        history.feed("pwX\u{8}d\r\n/tmp\r\nroot# ");
        let result = history.pairs(2)?;
        assert_eq!(result["pairs"][0]["stdin"], "pwd");
        assert_eq!(result["pairs"][0]["observedInput"], true);
        assert_eq!(result["pairs"][0]["status"], "completed");
        Ok(())
    }

    #[test]
    fn timeout_disconnect_and_initial_login_never_claim_command_completion() -> Result<()> {
        let mut history = history();
        assert!(history.begin("one", 0, "pwd", "root#").is_err());
        history.feed("root# ");
        history.begin("one", 0, "sleep 10", "root#")?;
        history.feed("partial");
        history.interrupt("timed_out");
        assert!(!history.ready());
        assert_eq!(history.pairs(2)?["pairs"][0]["status"], "timed_out");
        history.feed("\r\nroot# ");
        assert!(history.ready());
        assert_eq!(history.pairs(2)?["pairs"][0]["status"], "timed_out");
        Ok(())
    }

    #[test]
    fn stream_has_no_dialogue_or_selection_state() {
        let mut history = ShellHistory::default();
        history.feed("# command\nresult\n# ");
        assert!(history.pairs(2).is_err());
        assert!(history.selection().is_err());
        assert!(history.set_selection("output", false).is_err());
    }

    #[test]
    fn records_outputs_and_json_pages_remain_bounded() -> Result<()> {
        let mut history = history();
        for index in 0..40 {
            history.feed("root# ");
            history.begin("long", index, &"中".repeat(1000), "root# ")?;
            history.feed(&"\0\"\\".repeat(10000));
            history.feed("\r\nroot# ");
        }
        let pairs = history.pairs(32)?;
        assert_eq!(pairs["pairs"].as_array().map(Vec::len), Some(32));
        assert_eq!(pairs["dropped"], 8);
        assert_eq!(pairs["truncated"], true);
        assert!(serde_json::to_vec(&crate::serial::result::tool_result(pairs))?.len() < 16 * 1024);
        history.set_selection(&"\0".repeat(16000), false)?;
        assert!(
            serde_json::to_vec(&crate::serial::result::tool_result(history.selection()?))?.len()
                < 16 * 1024
        );
        Ok(())
    }

    #[test]
    fn prompt_validation_and_instance_isolation() -> Result<()> {
        for prompt in ["", " ", "#\r", "#\n", "\u{1b}[32m#"] {
            assert!(validate_prompt(prompt).is_err());
        }
        assert!(validate_prompt(&"x".repeat(257)).is_err());
        validate_prompt("root# ")?;
        let mut one = history();
        let mut two = history();
        one.feed("root# ");
        one.begin("one", 0, "pwd", "root#")?;
        one.feed("/tmp\r\nroot# ");
        one.set_selection("one", false)?;
        two.feed("root# ");
        assert!(two.pairs(2)?["pairs"].as_array().is_some_and(Vec::is_empty));
        assert_eq!(two.selection()?["text"], "");
        Ok(())
    }

    #[test]
    fn explicit_prompt_override_can_recognize_the_current_visible_line() -> Result<()> {
        let mut history = history();
        history.feed("other> ");
        assert!(!history.ready());
        history.begin("override", 0, "pwd", "other> ")?;
        history.feed("pwd\r\n/tmp\r\nother> ");
        assert!(history.completed("override", 0));
        history.set_selection("selected", true)?;
        assert_eq!(history.selection()?["truncated"], true);
        Ok(())
    }
}
