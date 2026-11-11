//! 串口系统超时和写端兼容；仅在 flush 不受支持时有界等待发送队列排空。
#[cfg(windows)]
use serialport::SerialPort as _;
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

pub struct SerialWriter {
    #[cfg(not(windows))]
    pub port: Box<dyn serialport::SerialPort>,
    #[cfg(windows)]
    pub port: serialport::COMPort,
    pub timeout: Duration,
    #[cfg(windows)]
    pub(super) operation: super::windows_io::Operation,
}

/// 为 Windows 端口设置独立读写期限，避免短读轮询截断正常的高速写入。
///
/// `port` 必须是仍打开的原生串口；两个 Duration 均须为 1..u32::MAX 毫秒。
/// 成功返回 `Ok(())`；非法期限或系统设置失败返回 `io::Error`。
#[cfg(windows)]
pub(super) fn configure_io_timeouts(
    port: &serialport::COMPort,
    read_poll: Duration,
    write_timeout: Duration,
) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    let timeouts = CommTimeouts::new(read_poll, write_timeout)?;
    // serialport::set_timeout 同时修改读写期限；克隆句柄共享系统设置，不能在写端重设它。
    // SAFETY: COMPort 持有有效句柄；repr(C) 对应 COMMTIMEOUTS，调用期间指针保持有效。
    if unsafe { set_comm_timeouts(port.as_raw_handle(), &timeouts) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
#[repr(C)]
struct CommTimeouts {
    read_interval: u32,
    read_multiplier: u32,
    read_constant: u32,
    write_multiplier: u32,
    write_constant: u32,
}

#[cfg(windows)]
impl CommTimeouts {
    fn new(read_poll: Duration, write_timeout: Duration) -> io::Result<Self> {
        let millis = |duration: Duration| {
            u32::try_from(duration.as_millis())
                .ok()
                .filter(|&value| value > 0 && value < u32::MAX)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Invalid serial timeout")
                })
        };
        Ok(Self {
            // Complete a bounded batch instead of issuing a fresh request for every arriving byte.
            read_interval: 1,
            read_multiplier: 0,
            read_constant: millis(read_poll)?,
            write_multiplier: 0,
            write_constant: millis(write_timeout)?,
        })
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "SetCommTimeouts"]
    fn set_comm_timeouts(
        handle: std::os::windows::io::RawHandle,
        timeouts: *const CommTimeouts,
    ) -> i32;
}

impl Write for SerialWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        #[cfg(windows)]
        {
            self.operation.write(&self.port, bytes)
        }
        #[cfg(not(windows))]
        {
            self.port.write(bytes)
        }
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
    #[cfg(windows)]
    fn windows_read_poll_does_not_shorten_write_deadline() -> io::Result<()> {
        let timeouts = CommTimeouts::new(Duration::from_millis(20), Duration::from_secs(2))?;
        assert_eq!(timeouts.read_interval, 1);
        assert_eq!(timeouts.read_multiplier, 0);
        assert_eq!(timeouts.read_constant, 20);
        assert_eq!(timeouts.write_multiplier, 0);
        assert_eq!(timeouts.write_constant, 2000);
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn windows_timeout_constants_reject_infinite_or_reserved_values() {
        for invalid in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(u64::from(u32::MAX)),
            Duration::MAX,
        ] {
            let valid = Duration::from_millis(20);
            assert!(CommTimeouts::new(invalid, valid)
                .is_err_and(|error| error.kind() == io::ErrorKind::InvalidInput));
            assert!(CommTimeouts::new(valid, invalid)
                .is_err_and(|error| error.kind() == io::ErrorKind::InvalidInput));
        }
    }

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
