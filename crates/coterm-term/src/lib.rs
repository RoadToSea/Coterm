//! 终端与流式输出共享的基础设施。
//!
//! SSH、串口和本地终端都要回答同一组问题：怎么在有限内存里保留最近的输出、怎么让调用方
//! 按稳定游标续读、缺口怎么报告、结果该给文本还是 base64。这些语义只实现一次，驱动只负责
//! 与协议相关的部分。
pub mod buffer;
pub mod decode;
pub mod encode;

pub use buffer::{OutputBuffer, Page};
pub use decode::Utf8Decoder;
pub use encode::{encode, encode_output, ENCODING_BASE64, ENCODING_TEXT};

use anyhow::{anyhow, Result};

/// 解析十进制游标。空字段由调用方按 0 处理，这里只拒绝非法文本。
pub fn parse_cursor(text: &str) -> Result<u64> {
    text.parse::<u64>()
        .map_err(|_| anyhow!("Invalid output cursor"))
}
