//! 跨块 UTF-8 解码。终端与串口分块写入时，一个多字节字符可能被切在两次读取之间。

/// 最多保留三个未完成的 UTF-8 字节；控制字符原样保留。
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

    /// 冲刷残留字节；调用后视作流结束。
    pub fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_survives_split_reads_and_keeps_controls() {
        let mut decoder = Utf8Decoder::default();
        assert_eq!(decoder.push(&[0xe4]), "");
        assert_eq!(decoder.push(&[0xb8]), "");
        assert_eq!(decoder.push(&[0xad, 3, 27, 13, 10]), "中\u{3}\u{1b}\r\n");
        assert_eq!(decoder.push(&[0xff, 0xe6]), "\u{fffd}");
        assert_eq!(decoder.finish(), "\u{fffd}");
        assert_eq!(decoder.finish(), "");
    }
}
