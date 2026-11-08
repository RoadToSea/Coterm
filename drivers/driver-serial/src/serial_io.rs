//! 串口写端的驱动兼容；仅在 flush 明确不受支持时改为有界等待发送队列排空。
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

pub struct SerialWriter {
    pub port: Box<dyn serialport::SerialPort>,
    pub timeout: Duration,
}

impl Write for SerialWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.port.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        finish_transmit(self.port.flush(), self.timeout, || {
            self.port.bytes_to_write().map_err(io::Error::from)
        })
    }
}

fn unsupported_flush(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    // Windows 虚拟驱动可能分别返回 ERROR_INVALID_FUNCTION / ERROR_NOT_SUPPORTED。
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(1 | 50)) {
        return true;
    }
    false
}

fn finish_transmit(
    result: io::Result<()>,
    timeout: Duration,
    mut pending: impl FnMut() -> io::Result<u32>,
) -> io::Result<()> {
    match result {
        Ok(()) => return Ok(()),
        Err(error) if unsupported_flush(&error) => {}
        Err(error) => return Err(error),
    }
    let deadline = Instant::now() + timeout;
    loop {
        if pending()? == 0 {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Serial transmit queue did not drain",
            ));
        }
        std::thread::sleep(remaining.min(Duration::from_millis(2)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_flush_does_not_query_the_queue() -> io::Result<()> {
        finish_transmit(Ok(()), Duration::from_secs(1), || {
            Err(io::Error::other("unexpected queue query"))
        })
    }

    #[test]
    fn unsupported_flush_waits_for_confirmed_empty_queue() -> io::Result<()> {
        let mut polls = 0;
        finish_transmit(
            Err(io::ErrorKind::Unsupported.into()),
            Duration::from_secs(1),
            || {
                polls += 1;
                Ok(if polls == 1 { 5 } else { 0 })
            },
        )?;
        assert_eq!(polls, 2);
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn windows_virtual_driver_invalid_function_uses_queue_confirmation() -> io::Result<()> {
        for code in [1, 50] {
            finish_transmit(
                Err(io::Error::from_raw_os_error(code)),
                Duration::from_secs(1),
                || Ok(0),
            )?;
        }
        Ok(())
    }

    #[test]
    fn real_flush_errors_and_queue_query_errors_remain_errors() {
        let error = finish_transmit(
            Err(io::ErrorKind::PermissionDenied.into()),
            Duration::from_secs(1),
            || Ok(0),
        );
        assert!(error.is_err_and(|error| error.kind() == io::ErrorKind::PermissionDenied));
        let error = finish_transmit(
            Err(io::ErrorKind::Unsupported.into()),
            Duration::from_secs(1),
            || Err(io::ErrorKind::BrokenPipe.into()),
        );
        assert!(error.is_err_and(|error| error.kind() == io::ErrorKind::BrokenPipe));
    }

    #[test]
    fn nonempty_transmit_queue_has_a_deadline() {
        let error = finish_transmit(
            Err(io::ErrorKind::Unsupported.into()),
            Duration::from_millis(3),
            || Ok(1),
        );
        assert!(error.is_err_and(|error| error.kind() == io::ErrorKind::TimedOut));
    }
}
