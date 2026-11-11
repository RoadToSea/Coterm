use base64::{engine::general_purpose::STANDARD, Engine};

pub(crate) fn text_prefix(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Lossless UTF-8 terminal text, or base64 for binary and excessive JSON expansion.
pub(crate) fn encode_output(
    bytes: &[u8],
    _truncated: bool,
) -> (&'static str, &'static str, String) {
    let readable = std::str::from_utf8(bytes).ok().filter(|text| {
        !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x1b' | '\x07' | '\x08'))
            && serde_json::to_string(text)
                .is_ok_and(|json| json.len() <= bytes.len().div_ceil(3) * 4 + 128)
    });
    match readable {
        Some(text) => ("utf-8", "text", text.to_owned()),
        None => ("base64", "data", STANDARD.encode(bytes)),
    }
}
