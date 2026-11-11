//! Windows overlapped serial IO: independent completion events keep reads and writes concurrent.
use serialport::{COMPort, Parity};
use std::{
    ffi::c_void,
    io::{self, Read},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    ptr,
};

#[repr(C)]
#[derive(Default)]
struct Dcb {
    length: u32,
    baud: u32,
    flags: u32,
    reserved: u16,
    xon_limit: u16,
    xoff_limit: u16,
    bits: u8,
    parity: u8,
    stop: u8,
    xon: i8,
    xoff: i8,
    error: i8,
    eof: i8,
    event: i8,
    reserved1: u16,
}

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: RawHandle,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "CreateFileW"]
    fn create_file(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: RawHandle,
    ) -> RawHandle;
    #[link_name = "GetCommState"]
    fn get_comm_state(port: RawHandle, dcb: *mut Dcb) -> i32;
    #[link_name = "SetCommState"]
    fn set_comm_state(port: RawHandle, dcb: *const Dcb) -> i32;
    #[link_name = "CreateEventW"]
    fn create_event(
        security: *const c_void,
        manual: i32,
        initial: i32,
        name: *const u16,
    ) -> RawHandle;
    #[link_name = "ResetEvent"]
    fn reset_event(event: RawHandle) -> i32;
    #[link_name = "ReadFile"]
    fn read_file(
        port: RawHandle,
        buffer: *mut u8,
        count: u32,
        read: *mut u32,
        state: *mut Overlapped,
    ) -> i32;
    #[link_name = "WriteFile"]
    fn write_file(
        port: RawHandle,
        buffer: *const u8,
        count: u32,
        written: *mut u32,
        state: *mut Overlapped,
    ) -> i32;
    #[link_name = "GetOverlappedResult"]
    fn get_result(port: RawHandle, state: *mut Overlapped, count: *mut u32, wait: i32) -> i32;
    #[cfg(test)]
    #[link_name = "CreateNamedPipeW"]
    fn create_pipe(
        name: *const u16,
        access: u32,
        mode: u32,
        instances: u32,
        output: u32,
        input: u32,
        timeout: u32,
        security: *const c_void,
    ) -> RawHandle;
    #[cfg(test)]
    #[link_name = "ConnectNamedPipe"]
    fn connect_pipe(port: RawHandle, state: *mut Overlapped) -> i32;
    #[cfg(test)]
    #[link_name = "CancelIoEx"]
    fn cancel_io(port: RawHandle, state: *mut Overlapped) -> i32;
}

/// Open an exclusive overlapped port with the specified name, baud and 5–8N/O/E1/2 framing.
/// Callers validate framing first. OS/open/configuration errors are returned; no IO starts here.
pub(super) fn open(
    name: &str,
    baud: u32,
    bits: u8,
    stop: u8,
    parity: Parity,
) -> io::Result<COMPort> {
    let name: Vec<_> = format!("\\\\.\\{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: name is terminated; no security/template pointers are supplied. COMPort owns success.
    let handle = unsafe {
        create_file(
            name.as_ptr(),
            0xc0000000,
            0,
            ptr::null(),
            3,
            0x40000080,
            ptr::null_mut(),
        )
    };
    if handle == -1_isize as RawHandle {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned a new owned handle; COMPort closes it on every error path.
    let port = unsafe { COMPort::from_raw_handle(handle) };
    let mut dcb = Dcb {
        length: std::mem::size_of::<Dcb>() as u32,
        ..Dcb::default()
    };
    // SAFETY: repr(C) matches DCB; the port and writable structure remain alive during this call.
    if unsafe { get_comm_state(handle, &mut dcb) } == 0 {
        return Err(io::Error::last_os_error());
    }
    dcb.baud = baud;
    dcb.bits = bits;
    dcb.stop = if stop == 2 { 2 } else { 0 };
    dcb.parity = match parity {
        Parity::None => 0,
        Parity::Odd => 1,
        Parity::Even => 2,
    };
    // Binary, no flow control, no null stripping/error substitution, DTR/RTS initially disabled.
    dcb.flags = 1 | if parity == Parity::None { 0 } else { 2 };
    dcb.xon = 17;
    dcb.xoff = 19;
    dcb.error = 0;
    dcb.eof = 26;
    // SAFETY: same owned port and initialized DCB; no asynchronous operation has started.
    if unsafe { set_comm_state(handle, &dcb) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(port)
}

pub(super) struct Operation {
    event: OwnedHandle,
}

impl Operation {
    /// Allocate a per-direction completion event; event allocation errors are returned.
    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: unnamed, manual-reset event with no security attributes.
        let event = unsafe { create_event(ptr::null(), 1, 0, ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a fresh event handle is transferred exactly once to OwnedHandle.
        Ok(Self {
            event: unsafe { OwnedHandle::from_raw_handle(event) },
        })
    }

    fn prepare(&self) -> io::Result<Overlapped> {
        // SAFETY: this event is owned and no previous operation remains pending.
        if unsafe { reset_event(self.event.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Overlapped {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event: self.event.as_raw_handle(),
        })
    }

    fn complete(port: &COMPort, state: &mut Overlapped, started: i32) -> io::Result<usize> {
        if started == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(997) {
                return Err(error);
            }
        }
        let mut count = 0;
        // SAFETY: the port, OVERLAPPED and caller's buffer stay alive until completion;
        // both immediate and pending IO use the completion result, not ReadFile's synchronous count.
        if unsafe { get_result(port.as_raw_handle(), state, &mut count, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(count as usize)
    }

    /// Read into a borrowed buffer, waiting for this operation only; propagate native IO errors.
    pub(super) fn read(&mut self, port: &COMPort, bytes: &mut [u8]) -> io::Result<usize> {
        let length = u32::try_from(bytes.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
        let mut state = self.prepare()?;
        // SAFETY: buffer bounds are checked; complete() waits before these locals can be dropped.
        let started = unsafe {
            read_file(
                port.as_raw_handle(),
                bytes.as_mut_ptr(),
                length,
                ptr::null_mut(),
                &mut state,
            )
        };
        Self::complete(port, &mut state, started)
    }

    /// Write borrowed bytes, returning the actual count or native error; never retry implicitly.
    pub(super) fn write(&mut self, port: &COMPort, bytes: &[u8]) -> io::Result<usize> {
        let length = u32::try_from(bytes.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
        let mut state = self.prepare()?;
        // SAFETY: buffer bounds are checked; complete() waits before these locals can be dropped.
        let started = unsafe {
            write_file(
                port.as_raw_handle(),
                bytes.as_ptr(),
                length,
                ptr::null_mut(),
                &mut state,
            )
        };
        Self::complete(port, &mut state, started)
    }
}

pub(super) struct Reader {
    port: COMPort,
    operation: Operation,
}

impl Reader {
    /// Own an already configured overlapped port; return an error if its read event cannot allocate.
    pub(super) fn new(port: COMPort) -> io::Result<Self> {
        Ok(Self {
            port,
            operation: Operation::new()?,
        })
    }
}

impl Read for Reader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        match self.operation.read(&self.port, bytes)? {
            0 => Err(io::ErrorKind::TimedOut.into()),
            count => Ok(count),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipe_pair() -> io::Result<(COMPort, COMPort)> {
        let name: Vec<_> = format!("\\\\.\\pipe\\coterm-io-{}", uuid::Uuid::new_v4())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: unique byte-mode pipe; both successful handles have one owning COMPort.
        let raw = unsafe {
            create_pipe(
                name.as_ptr(),
                0x40000003,
                0,
                1,
                1024,
                1024,
                1000,
                ptr::null(),
            )
        };
        if raw == -1_isize as RawHandle {
            return Err(io::Error::last_os_error());
        }
        let server = unsafe { COMPort::from_raw_handle(raw) };
        let raw = unsafe {
            create_file(
                name.as_ptr(),
                0xc0000000,
                0,
                ptr::null(),
                3,
                0x40000080,
                ptr::null_mut(),
            )
        };
        if raw == -1_isize as RawHandle {
            return Err(io::Error::last_os_error());
        }
        let client = unsafe { COMPort::from_raw_handle(raw) };
        let connect = Operation::new()?;
        let mut state = connect.prepare()?;
        let started = unsafe { connect_pipe(server.as_raw_handle(), &mut state) };
        if started != 0 || io::Error::last_os_error().raw_os_error() != Some(535) {
            Operation::complete(&server, &mut state, started)?;
        }
        Ok((server, client))
    }

    fn exchange(server: &COMPort, client: &COMPort) -> io::Result<u8> {
        let mut write = Operation::new()?;
        let mut peer = Operation::new()?;
        if write.write(server, b"x")? != 1 {
            return Err(io::Error::other("Incomplete pipe write"));
        }
        let mut observed = [0];
        if peer.read(client, &mut observed)? != 1 || peer.write(client, b"y")? != 1 {
            return Err(io::Error::other("Incomplete pipe exchange"));
        }
        Ok(observed[0])
    }

    #[test]
    fn pending_read_does_not_block_a_write_on_the_same_file_object() -> io::Result<()> {
        let (server, client) = pipe_pair()?;
        let read = Operation::new()?;
        let mut state = read.prepare()?;
        let mut buffer = [0];
        let mut count = 0;
        // SAFETY: retain these locals and handles until get_result below, including on exchange errors.
        let started = unsafe {
            read_file(
                server.as_raw_handle(),
                buffer.as_mut_ptr(),
                1,
                &mut count,
                &mut state,
            )
        };
        if started != 0 || io::Error::last_os_error().raw_os_error() != Some(997) {
            return Err(io::Error::other("Expected an outstanding empty-pipe read"));
        }
        let result = exchange(&server, &client);
        if result.is_err() {
            // No early return may leave the kernel borrowing this stack buffer or OVERLAPPED.
            unsafe {
                cancel_io(server.as_raw_handle(), &mut state);
            }
        }
        let completed = unsafe { get_result(server.as_raw_handle(), &mut state, &mut count, 1) };
        if completed == 0 {
            return Err(io::Error::last_os_error());
        }
        assert_eq!(result?, b'x');
        assert_eq!(count, 1);
        assert_eq!(buffer, *b"y");
        Ok(())
    }

    #[test]
    fn native_layouts_and_independent_events_match_windows() -> io::Result<()> {
        assert_eq!(std::mem::size_of::<Dcb>(), 28);
        assert_eq!(
            std::mem::size_of::<Overlapped>(),
            if cfg!(target_pointer_width = "64") {
                32
            } else {
                20
            }
        );
        let read = Operation::new()?;
        let write = Operation::new()?;
        assert_ne!(read.event.as_raw_handle(), write.event.as_raw_handle());
        assert_eq!(read.prepare()?.event, read.event.as_raw_handle());
        Ok(())
    }
}
