//! 有界输出缓冲：单调游标、容量淘汰与缺口报告。
//!
//! 终端回放和作业流都需要「按游标读取有界字节」这一套语义，区别只在容量、分块大小
//! 和是否同时保存 UTF-8 文本，因此由 [`OutputBuffer`] 参数化，而不是每个驱动各写一份。
use std::collections::VecDeque;

use anyhow::{bail, Result};

use crate::decode::Utf8Decoder;

struct Chunk {
    sequence: u64,
    bytes: Vec<u8>,
    /// 仅在解码模式保存，纯字节场景不必多付一份文本内存。
    text: Option<String>,
}

/// 一页读取结果。
///
/// `gap` 表示游标已经越过已淘汰的块，调用方必须按返回的 `cursor` 重新对齐，
/// 不要把缺口当成连续数据。
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub cursor: u64,
    pub data: Vec<u8>,
    pub text: String,
    pub gap: bool,
    pub more: bool,
    pub closed: bool,
}

pub struct OutputBuffer {
    chunks: VecDeque<Chunk>,
    decoder: Option<Utf8Decoder>,
    bytes: usize,
    sequence: u64,
    limit: usize,
    chunk_bytes: usize,
    total: usize,
    dropped: usize,
    closed: bool,
}

impl OutputBuffer {
    /// 只保存原始字节（终端回放、作业流）。调用方自行决定如何呈现文本。
    pub fn bytes(limit: usize, chunk_bytes: usize) -> Self {
        Self::new(limit, chunk_bytes, false)
    }

    /// 同时保存 UTF-8 文本（以文本为主的通道）。解码器会跨块保留不完整序列。
    pub fn decoded(limit: usize, chunk_bytes: usize) -> Self {
        Self::new(limit, chunk_bytes, true)
    }

    fn new(limit: usize, chunk_bytes: usize, decode: bool) -> Self {
        Self {
            chunks: VecDeque::new(),
            decoder: decode.then(Utf8Decoder::default),
            bytes: 0,
            sequence: 0,
            limit,
            chunk_bytes: chunk_bytes.max(1),
            total: 0,
            dropped: 0,
            closed: false,
        }
    }

    /// 写入一段字节并切成有界块；超出容量时从最旧的块淘汰。
    ///
    /// 返回本次写入解出的文本；未开启解码时总是空字符串。
    pub fn push(&mut self, data: &[u8]) -> String {
        let mut text = String::new();
        for part in data.chunks(self.chunk_bytes) {
            let decoded = match self.decoder.as_mut() {
                Some(decoder) => decoder.push(part),
                None => String::new(),
            };
            let decode = self.decoder.is_some();
            self.sequence += 1;
            self.total += part.len();
            self.bytes += part.len();
            text.push_str(&decoded);
            self.chunks.push_back(Chunk {
                sequence: self.sequence,
                bytes: part.to_vec(),
                text: decode.then_some(decoded),
            });
            while self.bytes > self.limit {
                let Some(chunk) = self.chunks.pop_front() else {
                    break;
                };
                self.bytes -= chunk.bytes.len();
                self.dropped += chunk.bytes.len();
            }
        }
        text
    }

    /// 结束写入：冲刷解码器残留（不完整序列按替换字符处理）并标记关闭。
    ///
    /// 冲刷出的文本没有对应原始字节，因此作为一个空字节块入队，保证游标连续。
    pub fn close(&mut self) -> String {
        let tail = self
            .decoder
            .as_mut()
            .map(Utf8Decoder::finish)
            .unwrap_or_default();
        if !tail.is_empty() {
            self.sequence += 1;
            self.chunks.push_back(Chunk {
                sequence: self.sequence,
                bytes: Vec::new(),
                text: Some(tail.clone()),
            });
        }
        self.closed = true;
        tail
    }

    /// 读取 `after` 之后、总长不超过 `limit` 的一页数据。
    pub fn collect(&self, after: u64, limit: usize) -> Result<Page> {
        if after > self.sequence {
            bail!("Output cursor is ahead of the connection");
        }
        let first = self
            .chunks
            .front()
            .map_or(self.sequence + 1, |chunk| chunk.sequence);
        let gap = after.saturating_add(1) < first;
        let mut next = if gap { first - 1 } else { after };
        let mut page = Page {
            gap,
            closed: self.closed,
            ..Page::default()
        };
        for chunk in &self.chunks {
            if chunk.sequence <= next {
                continue;
            }
            if page.data.len() + chunk.bytes.len() > limit {
                break;
            }
            page.data.extend_from_slice(&chunk.bytes);
            if let Some(text) = &chunk.text {
                page.text.push_str(text);
            }
            next = chunk.sequence;
        }
        page.cursor = next;
        page.more = next < self.sequence;
        Ok(page)
    }

    /// 保留区最前面的 `limit` 字节，用于结果预览。
    ///
    /// 缺口不在这里计算：调用方用 [`dropped`](Self::dropped) 与 [`retained`](Self::retained)
    /// 组合出「原始字节 / 丢失字节」的完整统计。
    pub fn preview(&self, limit: usize) -> Vec<u8> {
        let mut data = Vec::new();
        for chunk in &self.chunks {
            if data.len() >= limit {
                break;
            }
            let take = (limit - data.len()).min(chunk.bytes.len());
            data.extend_from_slice(&chunk.bytes[..take]);
        }
        data
    }

    /// 已淘汰（无法再回放）的字节数。
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// 当前保留的字节数。
    pub fn retained(&self) -> usize {
        self.bytes
    }

    /// 累计写入的字节数，含已淘汰部分。
    pub fn total(&self) -> usize {
        self.total
    }

    /// 最新块序号，等价于「当前游标」。
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// 保留区全部文本；未开启解码时为空。
    pub fn text(&self) -> String {
        self.chunks
            .iter()
            .filter_map(|chunk| chunk.text.as_deref())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_follow_the_configured_size_and_report_more() {
        let mut buffer = OutputBuffer::bytes(64, 4);
        buffer.push(b"abcdefghij");
        assert_eq!(buffer.sequence(), 3);
        assert_eq!(buffer.total(), 10);
        let page = buffer.collect(0, 8).unwrap();
        assert_eq!(page.data, b"abcdefgh");
        assert_eq!(page.cursor, 2);
        assert!(page.more);
        assert!(!page.gap);
        let page = buffer.collect(2, 8).unwrap();
        assert_eq!(page.data, b"ij");
        assert!(!page.more);
    }

    #[test]
    fn a_cursor_ahead_of_the_stream_is_an_error() {
        let mut buffer = OutputBuffer::bytes(64, 4);
        buffer.push(b"ab");
        assert!(buffer.collect(2, 8).is_err());
    }

    #[test]
    fn eviction_reports_the_gap_and_keeps_byte_accounting() {
        let mut buffer = OutputBuffer::bytes(4, 4);
        buffer.push(b"abcd");
        buffer.push(b"efgh");
        // 只剩最后一块；从 0 读取必须明确报告缺口，而不是把 efgh 当成开头。
        let page = buffer.collect(0, 16).unwrap();
        assert!(page.gap);
        assert_eq!(page.data, b"efgh");
        assert_eq!(page.cursor, 2);
        assert_eq!(buffer.dropped(), 4);
        assert_eq!(buffer.retained(), 4);
        assert_eq!(buffer.total(), 8);
        assert_eq!(buffer.preview(16), b"efgh");
        assert_eq!(buffer.preview(2), b"ef");
    }

    #[test]
    fn text_mode_decodes_across_chunk_boundaries_and_flushes_the_tail() {
        let mut buffer = OutputBuffer::decoded(64, 1);
        assert_eq!(buffer.push(&[0xe4]), "");
        assert_eq!(buffer.push(&[0xb8]), "");
        assert_eq!(buffer.push(&[0xad, b'a']), "中a");
        assert_eq!(buffer.text(), "中a");
        let page = buffer.collect(0, 64).unwrap();
        assert_eq!(page.text, "中a");
        assert_eq!(page.data.len(), 4);

        assert_eq!(buffer.push(&[0xff]), "\u{fffd}");
        assert_eq!(buffer.close(), "");
        assert!(buffer.is_closed());
    }

    #[test]
    fn closing_flushes_an_incomplete_sequence_as_one_more_chunk() {
        let mut buffer = OutputBuffer::decoded(64, 4);
        buffer.push(&[0xe4, 0xb8]);
        let cursor = buffer.sequence();
        assert_eq!(buffer.close(), "\u{fffd}");
        assert_eq!(buffer.sequence(), cursor + 1);
        let page = buffer.collect(cursor, 64).unwrap();
        assert!(page.data.is_empty());
        assert_eq!(page.text, "\u{fffd}");
        assert!(page.closed);
    }

    #[test]
    fn byte_mode_never_accrues_text() {
        let mut buffer = OutputBuffer::bytes(64, 2);
        assert_eq!(buffer.push(b"hello"), "");
        assert_eq!(buffer.text(), "");
        assert_eq!(buffer.close(), "");
        assert_eq!(buffer.sequence(), 3);
    }

    #[test]
    fn empty_writes_do_not_advance_the_cursor() {
        let mut buffer = OutputBuffer::bytes(64, 4);
        buffer.push(b"");
        assert_eq!(buffer.sequence(), 0);
        let page = buffer.collect(0, 64).unwrap();
        assert!(page.data.is_empty());
        assert!(!page.more);
    }
}
