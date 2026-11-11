//! 结果编码：能读的 UTF-8 终端文本直接给 `text`，其余保持无损 base64。
use base64::{engine::general_purpose::STANDARD, Engine};

/// UTF-8 终端输出：保留 ANSI 与常见终端控制字符。
pub const ENCODING_TEXT: &str = "utf-8";
/// 二进制或控制字符过密的输出：base64 无损返回。
pub const ENCODING_BASE64: &str = "base64";

/// 选择 `text` 或 `data` 字段并给出编码名。
///
/// 判定条件包含 JSON 转义后的体积：控制字符密集的捕获经转义可能膨胀六倍，
/// 这种情况宁可退回 base64，也不要让结果超出宿主的结果预算。
pub fn encode(bytes: &[u8]) -> (&'static str, &'static str, String) {
    let readable = std::str::from_utf8(bytes).ok().filter(|text| {
        !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x1b' | '\x07' | '\x08'))
            && serde_json::to_string(text)
                .is_ok_and(|json| json.len() <= bytes.len().div_ceil(3) * 4 + 128)
    });
    match readable {
        Some(text) => (ENCODING_TEXT, "text", text.to_owned()),
        None => (ENCODING_BASE64, "data", STANDARD.encode(bytes)),
    }
}

/// 兼容既有调用点的签名；`truncated` 不影响编码选择。
pub fn encode_output(bytes: &[u8], _truncated: bool) -> (&'static str, &'static str, String) {
    encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_output_stays_readable_text() {
        let bytes = b"\x1b[32mconfig.yaml\x1b[0m\r\n";
        let (encoding, field, text) = encode(bytes);
        assert_eq!(encoding, ENCODING_TEXT);
        assert_eq!(field, "text");
        assert_eq!(text.as_bytes(), bytes);
    }

    #[test]
    fn control_heavy_and_binary_output_stay_lossless_base64() {
        for bytes in [
            b"\x00\x01\x02".to_vec(),
            b"\x1b[0m".repeat(1024),
            vec![0xff, 0xfe, 0x00],
        ] {
            let (encoding, field, value) = encode(&bytes);
            assert_eq!(encoding, ENCODING_BASE64, "{bytes:?}");
            assert_eq!(field, "data");
            let decoded = STANDARD.decode(value).unwrap();
            assert_eq!(decoded, bytes);
        }
    }

    #[test]
    fn empty_output_is_readable() {
        assert_eq!(encode(b"").1, "text");
    }
}
