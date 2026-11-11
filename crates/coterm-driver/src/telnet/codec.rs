//! Incremental NVT decoding. Negotiation bytes never enter terminal history.
use anyhow::{bail, Result};

const IAC: u8 = 255;
const SE: u8 = 240;
const SB: u8 = 250;
const WILL: u8 = 251;
const WONT: u8 = 252;
const DO: u8 = 253;
const DONT: u8 = 254;
const BINARY: u8 = 0;
const ECHO: u8 = 1;
const SGA: u8 = 3;
const TERMINAL_TYPE: u8 = 24;
const NAWS: u8 = 31;

#[derive(Default)]
enum State {
    #[default]
    Data,
    Iac,
    Option(u8),
    Sub,
    SubIac,
}

pub(crate) struct Decoded {
    pub data: Vec<u8>,
    pub reply: Vec<u8>,
}

pub(crate) struct Codec {
    state: State,
    local: [bool; 256],
    remote: [bool; 256],
    sub: Vec<u8>,
    previous_cr: bool,
    terminal_type: String,
    cols: u16,
    rows: u16,
}

impl Codec {
    pub(crate) fn new(terminal_type: &str, cols: u16, rows: u16) -> Self {
        Self {
            state: State::Data,
            local: [false; 256],
            remote: [false; 256],
            sub: Vec::new(),
            previous_cr: false,
            terminal_type: terminal_type.into(),
            cols,
            rows,
        }
    }

    pub(crate) fn remote_echo(&self) -> bool {
        self.remote[ECHO as usize]
    }

    pub(crate) fn decode(&mut self, bytes: &[u8]) -> Result<Decoded> {
        let mut decoded = Decoded {
            data: Vec::with_capacity(bytes.len()),
            reply: Vec::new(),
        };
        for &byte in bytes {
            match self.state {
                State::Data if byte == IAC => self.state = State::Iac,
                State::Data => self.data(byte, &mut decoded.data),
                State::Iac => match byte {
                    IAC => {
                        self.data(byte, &mut decoded.data);
                        self.state = State::Data;
                    }
                    WILL | WONT | DO | DONT => self.state = State::Option(byte),
                    SB => {
                        self.sub.clear();
                        self.state = State::Sub;
                    }
                    _ => self.state = State::Data,
                },
                State::Option(verb) => {
                    self.negotiate(verb, byte, &mut decoded.reply);
                    self.state = State::Data;
                }
                State::Sub if byte == IAC => self.state = State::SubIac,
                State::Sub => self.sub.push(byte),
                State::SubIac => match byte {
                    IAC => {
                        self.sub.push(IAC);
                        self.state = State::Sub;
                    }
                    SE => {
                        self.subnegotiation(&mut decoded.reply);
                        self.sub.clear();
                        self.state = State::Data;
                    }
                    _ => {
                        self.sub.clear();
                        self.state = State::Data;
                    }
                },
            }
            if self.sub.len() > 512 {
                bail!("Telnet subnegotiation exceeds the bounded option size")
            }
        }
        Ok(decoded)
    }

    fn data(&mut self, byte: u8, output: &mut Vec<u8>) {
        if !self.remote[BINARY as usize] && self.previous_cr && byte == 0 {
            self.previous_cr = false;
            return;
        }
        output.push(byte);
        self.previous_cr = byte == b'\r';
    }

    fn negotiate(&mut self, verb: u8, option: u8, reply: &mut Vec<u8>) {
        let index = option as usize;
        match verb {
            WILL if matches!(option, BINARY | ECHO | SGA) => {
                if !self.remote[index] {
                    reply.extend([IAC, DO, option]);
                    self.remote[index] = true;
                }
            }
            WILL => reply.extend([IAC, DONT, option]),
            WONT => {
                if self.remote[index] {
                    reply.extend([IAC, DONT, option]);
                    self.remote[index] = false;
                }
            }
            DO if matches!(option, BINARY | SGA | NAWS | TERMINAL_TYPE) => {
                if !self.local[index] {
                    self.local[index] = true;
                    reply.extend([IAC, WILL, option]);
                    if option == NAWS {
                        reply.extend(self.window());
                    }
                }
            }
            DO => reply.extend([IAC, WONT, option]),
            DONT if self.local[index] => {
                reply.extend([IAC, WONT, option]);
                self.local[index] = false;
            }
            _ => {}
        }
    }

    fn subnegotiation(&self, reply: &mut Vec<u8>) {
        if self.sub == [TERMINAL_TYPE, 1] && self.local[TERMINAL_TYPE as usize] {
            let mut payload = vec![TERMINAL_TYPE, 0];
            payload.extend(self.terminal_type.as_bytes());
            reply.extend(subframe(&payload));
        }
    }

    fn window(&self) -> Vec<u8> {
        let mut data = vec![NAWS];
        data.extend(self.cols.to_be_bytes());
        data.extend(self.rows.to_be_bytes());
        subframe(&data)
    }

    pub(crate) fn resize(&mut self, cols: u16, rows: u16) -> Vec<u8> {
        self.cols = cols;
        self.rows = rows;
        if self.local[NAWS as usize] {
            self.window()
        } else {
            Vec::new()
        }
    }

    pub(crate) fn encode(&self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len() * 2);
        for (index, &byte) in bytes.iter().enumerate() {
            if !self.local[BINARY as usize] {
                if byte == b'\n' && (index == 0 || bytes[index - 1] != b'\r') {
                    output.push(b'\r');
                }
                output.push(byte);
                if byte == b'\r' && bytes.get(index + 1) != Some(&b'\n') {
                    output.push(0);
                }
            } else {
                output.push(byte);
            }
            if byte == IAC {
                output.push(IAC);
            }
        }
        output
    }
}

fn subframe(data: &[u8]) -> Vec<u8> {
    let mut output = vec![IAC, SB];
    for &byte in data {
        output.push(byte);
        if byte == IAC {
            output.push(IAC);
        }
    }
    output.extend([IAC, SE]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_options_subnegotiation_and_nvt_bytes_are_separated() -> Result<()> {
        let wire = [
            b'h',
            IAC,
            WILL,
            ECHO,
            IAC,
            DO,
            NAWS,
            IAC,
            DO,
            TERMINAL_TYPE,
            IAC,
            SB,
            TERMINAL_TYPE,
            1,
            IAC,
            SE,
            b'i',
            b'\r',
            0,
            IAC,
            IAC,
        ];
        for size in 1..=wire.len() {
            let mut codec = Codec::new("vt100", 255, 24);
            let mut data = Vec::new();
            let mut reply = Vec::new();
            for part in wire.chunks(size) {
                let decoded = codec.decode(part)?;
                data.extend(decoded.data);
                reply.extend(decoded.reply);
            }
            assert_eq!(data, [b'h', b'i', b'\r', IAC]);
            assert!(codec.remote_echo());
            let mut expected = vec![IAC, DO, ECHO, IAC, WILL, NAWS];
            expected.extend(subframe(&[NAWS, 0, 255, 0, 24]));
            expected.extend([IAC, WILL, TERMINAL_TYPE]);
            expected.extend(subframe(&[TERMINAL_TYPE, 0, b'v', b't', b'1', b'0', b'0']));
            assert_eq!(reply, expected);
        }
        Ok(())
    }

    #[test]
    fn duplicate_enables_do_not_loop_and_disable_is_acknowledged_once() -> Result<()> {
        let mut codec = Codec::new("xterm", 80, 24);
        assert_eq!(codec.decode(&[IAC, WILL, ECHO])?.reply, [IAC, DO, ECHO]);
        assert!(codec.decode(&[IAC, WILL, ECHO])?.reply.is_empty());
        assert_eq!(codec.decode(&[IAC, WONT, ECHO])?.reply, [IAC, DONT, ECHO]);
        assert!(codec.decode(&[IAC, WONT, ECHO])?.reply.is_empty());
        assert!(!codec.remote_echo());
        assert_eq!(
            codec.decode(&[IAC, DO, 99, IAC, WILL, 99])?.reply,
            [IAC, WONT, 99, IAC, DONT, 99]
        );
        Ok(())
    }

    #[test]
    fn binary_negotiation_preserves_bytes_and_naws_escapes_ff() -> Result<()> {
        let mut codec = Codec::new("xterm", 80, 24);
        assert_eq!(codec.encode(b"a\rb\n\r\n\xff"), b"a\r\0b\r\n\r\n\xff\xff");
        codec.decode(&[IAC, DO, BINARY, IAC, WILL, BINARY, IAC, DO, NAWS])?;
        assert_eq!(codec.encode(b"\r\0\xff"), b"\r\0\xff\xff");
        assert_eq!(codec.decode(b"\r\0")?.data, b"\r\0");
        assert_eq!(codec.resize(255, 255), subframe(&[NAWS, 0, 255, 0, 255]));
        assert!(codec
            .decode(&[IAC, SB].into_iter().chain([42; 514]).collect::<Vec<_>>())
            .is_err());
        Ok(())
    }
}
