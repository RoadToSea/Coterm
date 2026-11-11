//! Bounded byte replay, UTF-8 views and observed Shell records shared by stream transports.
use super::payload::{bounded_output_text, encode_hex, Utf8Decoder, OUTPUT_TEXT_JSON_BUDGET};
use super::shell::{Mode, ShellHistory};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::VecDeque;

pub(crate) const MAX_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const OUTPUT_CHUNK_BYTES: usize = 1024;
pub(crate) const READ_BYTES: usize = 1024;

#[derive(Default)]
pub struct OutputBuffer {
    chunks: VecDeque<(u64, Vec<u8>, String)>,
    decoder: Utf8Decoder,
    pub(crate) bytes: usize,
    sequence: u64,
    pub closed: bool,
    pub(crate) shell: ShellHistory,
}

impl OutputBuffer {
    pub(crate) fn with_shell(mode: Mode, prompt: Option<&str>) -> Self {
        Self {
            shell: ShellHistory::new(mode, prompt),
            ..Self::default()
        }
    }

    pub fn cursor(&self) -> u64 {
        self.sequence
    }

    pub fn push(&mut self, data: &[u8]) -> String {
        let mut text = String::new();
        for part in data.chunks(OUTPUT_CHUNK_BYTES) {
            let decoded = self.decoder.push(part);
            text.push_str(&decoded);
            self.sequence += 1;
            self.chunks
                .push_back((self.sequence, part.to_vec(), decoded));
            self.bytes += part.len();
            while self.bytes > MAX_OUTPUT_BYTES {
                if let Some((_, data, _)) = self.chunks.pop_front() {
                    self.bytes -= data.len();
                }
            }
        }
        self.shell.feed(&text);
        text
    }

    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        let tail = self.decoder.finish();
        self.shell.feed(&tail);
        self.shell.interrupt("unknown");
        if self.shell.enabled() {
            let _ = self.shell.set_selection("", false);
        }
        if !tail.is_empty() {
            self.sequence += 1;
            self.chunks.push_back((self.sequence, Vec::new(), tail));
        }
        self.closed = true;
    }

    /// 读取 `after` 之后的有界数据；游标过期时报告 gap，交由调用方重新对齐。
    pub fn read(&self, after: u64) -> Result<Value> {
        self.read_page(after, false)
    }

    pub fn read_hex(&self, after: u64) -> Result<Value> {
        self.read_page(after, true)
    }

    /// Return a bounded tail snapshot without requiring the caller to remember a cursor.
    pub fn recent_text(&self, lines: usize) -> Result<Value> {
        if !(1..=200).contains(&lines) {
            bail!("Recent output lines must be between 1 and 200");
        }

        let text = self
            .chunks
            .iter()
            .map(|(_, _, decoded)| decoded.as_str())
            .collect::<String>();
        let selected = text
            .split_inclusive('\n')
            .rev()
            .take(lines)
            .collect::<Vec<_>>();
        let recent = selected.iter().rev().copied().collect::<String>();
        let (bounded, _) = bounded_output_text(&recent, OUTPUT_TEXT_JSON_BUDGET, true);
        let truncated = bounded.len() < recent.len();

        Ok(json!({
            "text": bounded,
            "lines": bounded.split_inclusive('\n').count(),
            "cursor": self.sequence,
            "closed": self.closed,
            "gap": self.chunks.front().is_some_and(|chunk| chunk.0 > 1),
            "truncated": truncated,
        }))
    }

    /// Returns 1–1000 trailing LF-delimited lines, bounded by the result JSON budget.
    pub fn read_lines(&self, count: usize) -> Result<Value> {
        if !(1..=1000).contains(&count) {
            bail!("Line count must be between 1 and 1000");
        }
        let text = self
            .chunks
            .iter()
            .map(|(_, _, text)| text.as_str())
            .collect::<String>();
        let mut lines: Vec<_> = text.lines().rev().take(count).map(str::to_owned).collect();
        lines.reverse();
        let mut truncated = false;
        while serde_json::to_vec(&lines)?.len() > 10 * 1024 {
            truncated = true;
            if lines.len() > 1 {
                lines.remove(0);
            } else if let Some(line) = lines.first_mut() {
                let mut start = line.len() / 2;
                while !line.is_char_boundary(start) {
                    start += 1;
                }
                *line = line[start..].into();
            } else {
                break;
            }
        }
        Ok(
            json!({"lines":lines,"stderrLines":[],"streamMode":"merged","closed":self.closed,
            "truncated":truncated,"dropped":self.chunks.front().is_some_and(|chunk| chunk.0>1)}),
        )
    }

    fn read_page(&self, after: u64, hex: bool) -> Result<Value> {
        if after > self.sequence {
            bail!("Output cursor is ahead of the connection");
        }
        let first = self
            .chunks
            .front()
            .map_or(self.sequence + 1, |chunk| chunk.0);
        let gap = after.saturating_add(1) < first;
        let mut next = if gap { first - 1 } else { after };
        let mut data = Vec::with_capacity(READ_BYTES);
        let mut text = String::new();
        for (sequence, part, decoded) in &self.chunks {
            if *sequence <= next {
                continue;
            }
            if data.len() + part.len() > READ_BYTES {
                break;
            }
            data.extend_from_slice(part);
            text.push_str(decoded);
            next = *sequence;
        }
        let mut result = json!({
            "bytes": data.len(),
            "cursor": next,
            "gap": gap,
            "closed": self.closed,
            "more": next < self.sequence,
        });
        if hex {
            result["hex"] = json!(encode_hex(&data));
        } else {
            result["text"] = json!(text);
        }
        Ok(result)
    }
}
