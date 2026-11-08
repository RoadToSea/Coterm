//! 有界终端输出缓存、UTF-8 解码和结果编码工具，当前由 Local 驱动使用。
//!
//! 游标用于前端内部回放；Agent 通过驱动的无游标快照接口读取数据。
//! SSH 与 Serial 保留各自的领域缓冲实现。
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
