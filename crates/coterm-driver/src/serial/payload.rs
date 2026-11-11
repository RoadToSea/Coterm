//! Separate text and HEX contracts; both terminate in exact serial bytes.
use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use serde_json::Value;

pub const MAX_WRITE_BYTES: usize = 16 * 1024;
/// Text appears in both MCP content and structuredContent. Reserve room for the
/// envelope within the host's 16 KiB result budget, including JSON escaping.
pub(crate) const OUTPUT_TEXT_JSON_BUDGET: usize = 6 * 1024;

pub(crate) fn bounded_output_text(text: &str, budget: usize, tail: bool) -> (&str, usize) {
    let mut chars = text.char_indices();
    let mut used = 0;
    let mut boundary = if tail { text.len() } else { 0 };
    loop {
        let next = if tail {
            chars.next_back()
        } else {
            chars.next()
        };
        let Some((index, ch)) = next else { break };
        let bytes = match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            '\u{0}'..='\u{1f}' => 6,
            _ => ch.len_utf8(),
        };
        if used + bytes > budget {
            break;
        }
        used += bytes;
        boundary = if tail { index } else { index + ch.len_utf8() };
    }
    (
        if tail {
            &text[boundary..]
        } else {
            &text[..boundary]
        },
        used,
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextArgs {
    text: String,
    #[serde(default = "default_ending")]
    line_ending: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HexArgs {
    hex: String,
}

fn default_ending() -> String {
    "none".into()
}

pub fn encode_text(text: &str, ending: &str) -> Result<Vec<u8>> {
    let suffix: &[u8] = match ending {
        "none" => b"",
        "cr" => b"\r",
        "lf" => b"\n",
        "crlf" => b"\r\n",
        _ => bail!("Invalid serial line ending"),
    };
    let mut bytes = text.as_bytes().to_vec();
    bytes.extend_from_slice(suffix);
    validate_size(bytes)
}

pub fn decode_hex(hex: &str) -> Result<Vec<u8>> {
    // Bound allocation before stripping whitespace; no Base64 or implicit encoding.
    if hex.len() > MAX_WRITE_BYTES * 3 + 2 {
        bail!("Serial HEX input is too large");
    }
    let compact: String = hex.chars().filter(|ch| !ch.is_ascii_whitespace()).collect();
    let compact = compact
        .strip_prefix("0x")
        .or_else(|| compact.strip_prefix("0X"))
        .unwrap_or(&compact);
    if compact.is_empty()
        || !compact.len().is_multiple_of(2)
        || !compact.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("HEX input must contain an even number of hexadecimal digits");
    }
    let bytes = compact
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| {
                if byte <= b'9' {
                    byte - b'0'
                } else {
                    byte.to_ascii_lowercase() - b'a' + 10
                }
            };
            (digit(pair[0]) << 4) | digit(pair[1])
        })
        .collect();
    validate_size(bytes)
}

fn validate_size(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > MAX_WRITE_BYTES {
        bail!("Serial input must contain 1 to 16384 bytes");
    }
    Ok(bytes)
}

pub fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(DIGITS[(byte >> 4) as usize] as char);
        hex.push(DIGITS[(byte & 15) as usize] as char);
    }
    hex
}

pub fn text_payload(args: &Value) -> Result<Vec<u8>> {
    let args: TextArgs = serde_json::from_value(args.clone())
        .map_err(|_| anyhow!("Expected text and optional lineEnding"))?;
    encode_text(&args.text, &args.line_ending)
}

pub fn hex_payload(args: &Value) -> Result<Vec<u8>> {
    let args: HexArgs = serde_json::from_value(args.clone())
        .map_err(|_| anyhow!("Expected hex without text, data or lineEnding"))?;
    decode_hex(&args.hex)
}

/// At most three unfinished UTF-8 bytes survive a read. Control characters remain intact.
#[derive(Default)]
pub struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut offset = 0;
        while offset < self.pending.len() {
            match std::str::from_utf8(&self.pending[offset..]) {
                Ok(valid) => {
                    text.push_str(valid);
                    offset = self.pending.len();
                }
                Err(error) => {
                    let valid_end = offset + error.valid_up_to();
                    if let Ok(valid) = std::str::from_utf8(&self.pending[offset..valid_end]) {
                        text.push_str(valid);
                    }
                    offset = valid_end;
                    if let Some(invalid) = error.error_len() {
                        text.push('\u{fffd}');
                        offset += invalid;
                    } else {
                        break;
                    }
                }
            }
        }
        self.pending.drain(..offset);
        text
    }

    pub fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_is_exact_and_contracts_cannot_be_mixed() -> Result<()> {
        let text = "\u{3}\t\u{1b}[A\u{7f}\r\n中文\0";
        assert_eq!(text_payload(&json!({"text":text}))?, text.as_bytes());
        assert!(text_payload(&json!({"data":"AA=="})).is_err());
        assert!(text_payload(&json!({"text":"ok","hex":"00"})).is_err());
        assert!(hex_payload(&json!({"hex":"00","text":"ok"})).is_err());
        assert!(hex_payload(&json!({"hex":"00","lineEnding":"cr"})).is_err());
        Ok(())
    }

    #[test]
    fn hex_preserves_every_byte_and_rejects_malformed_input() -> Result<()> {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode_hex(&encode_hex(&all))?, all);
        assert_eq!(decode_hex("0x00 80 FF\n")?, [0, 128, 255]);
        for value in ["", "0", "0x", "GG", "AID/", "0x0x00"] {
            assert!(decode_hex(value).is_err());
        }
        assert!(decode_hex(&"ff".repeat(MAX_WRITE_BYTES + 1)).is_err());
        Ok(())
    }

    #[test]
    fn utf8_survives_split_reads_and_keeps_controls() {
        let mut decoder = Utf8Decoder::default();
        assert_eq!(decoder.push(&[0xe4]), "");
        assert_eq!(decoder.push(&[0xb8]), "");
        assert_eq!(decoder.push(&[0xad, 3, 27, 13, 10]), "中\u{3}\u{1b}\r\n");
        assert_eq!(decoder.push(&[0xff, 0xe6]), "\u{fffd}");
        assert_eq!(decoder.finish(), "\u{fffd}");
    }
}
