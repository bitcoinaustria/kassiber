//! The operator broker's named pipe, server and client.

use super::{last_error, process_sid, wide, RawHandle};
use std::ffi::OsStr;
use std::io;
use std::mem::{size_of, zeroed};
use std::ptr;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeServerProcessId, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};

const IO_CHUNK_BYTES: usize = 64 * 1024;
const CANCEL_DRAIN_MILLISECONDS: u32 = 1000;
const PIPE_BUFFER_BYTES: u32 = 65536;

/// Why the first instance could not be created: another listener holds the
/// name (Python's `operator_broker_running`), or something else failed.
#[derive(Debug)]
pub enum CreateError {
    AlreadyRunning,
    Io(io::Error),
}

/// A buffer and its OVERLAPPED whose cancelled I/O had not drained; kept
/// alive so the kernel never writes into freed memory.
struct PendingIo {
    _buffer: Box<[u8]>,
    overlapped: Box<OVERLAPPED>,
}
// SAFETY: the boxes are only reaped under the mutex below; the kernel may
// still write to them, which is why they are never freed early.
unsafe impl Send for PendingIo {}

static PENDING_IO: Mutex<Vec<PendingIo>> = Mutex::new(Vec::new());

fn reap_pending_io() {
    let mut pending = PENDING_IO
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    pending.retain(|entry| {
        // SAFETY: the event handle is owned by the retained entry.
        if unsafe { WaitForSingleObject(entry.overlapped.hEvent, 0) } == WAIT_OBJECT_0 {
            // SAFETY: completion was signalled; the event is ours to close.
            unsafe {
                CloseHandle(entry.overlapped.hEvent);
            }
            false
        } else {
            true
        }
    });
}

fn new_overlapped() -> io::Result<Box<OVERLAPPED>> {
    // SAFETY: an unnamed manual-reset event with default security.
    let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if event.is_null() {
        return Err(last_error("CreateEventW for operator pipe failed"));
    }
    // SAFETY: OVERLAPPED is plain data; zero is its documented initial state.
    let mut overlapped: Box<OVERLAPPED> = Box::new(unsafe { zeroed() });
    overlapped.hEvent = event;
    Ok(overlapped)
}

fn close_event(overlapped: &OVERLAPPED) {
    // SAFETY: the event was created by new_overlapped and is closed once.
    unsafe {
        CloseHandle(overlapped.hEvent);
    }
}

/// One connected pipe end with Python's deadline semantics: each
/// `recv_exact` and `send_all` call as a whole must finish within
/// `io_timeout`.
pub struct PipeStream {
    handle: RawHandle,
    io_timeout: Option<Duration>,
    closed: bool,
}

impl PipeStream {
    fn new(handle: HANDLE, io_timeout: Option<Duration>) -> Self {
        PipeStream {
            handle: RawHandle(handle),
            io_timeout,
            closed: false,
        }
    }

    pub fn set_io_timeout(&mut self, timeout: Option<Duration>) {
        self.io_timeout = timeout;
    }

    fn close_handle(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        // SAFETY: the stream owns its handle and closes it exactly once.
        unsafe {
            CloseHandle(self.handle.0);
        }
    }

    /// Close and, on the server side, disconnect first, as Python does
    /// when it rejects a client.
    pub fn reject(mut self) {
        if !self.closed {
            // SAFETY: the handle is still owned and open.
            unsafe {
                DisconnectNamedPipe(self.handle.0);
            }
        }
        self.close_handle();
    }

    fn cancel_timed_out_io(&mut self, buffer: Box<[u8]>, overlapped: Box<OVERLAPPED>) {
        // SAFETY: the handle is open; the OVERLAPPED identifies our request.
        unsafe {
            CancelIoEx(self.handle.0, &*overlapped);
        }
        self.close_handle();
        // SAFETY: the event belongs to `overlapped`.
        if unsafe { WaitForSingleObject(overlapped.hEvent, CANCEL_DRAIN_MILLISECONDS) }
            == WAIT_OBJECT_0
        {
            close_event(&overlapped);
        } else {
            PENDING_IO
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(PendingIo {
                    _buffer: buffer,
                    overlapped,
                });
        }
    }

    fn overlapped_io(
        &mut self,
        write: bool,
        mut buffer: Box<[u8]>,
        deadline: Option<Instant>,
        timeout_message: &'static str,
        failure_message: &'static str,
    ) -> io::Result<(Box<[u8]>, usize)> {
        if self.closed {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "operator named pipe closed",
            ));
        }
        reap_pending_io();
        let mut overlapped = new_overlapped()?;
        let size = buffer.len() as u32;
        // SAFETY: buffer and overlapped are boxed, so their addresses stay
        // valid until the I/O completes or is retained after cancellation.
        let completed = unsafe {
            if write {
                WriteFile(
                    self.handle.0,
                    buffer.as_ptr(),
                    size,
                    ptr::null_mut(),
                    &mut *overlapped,
                )
            } else {
                ReadFile(
                    self.handle.0,
                    buffer.as_mut_ptr(),
                    size,
                    ptr::null_mut(),
                    &mut *overlapped,
                )
            }
        };
        let mut transferred = 0u32;
        if completed != 0 {
            // SAFETY: the request completed; this only reads its count.
            let ok =
                unsafe { GetOverlappedResult(self.handle.0, &*overlapped, &mut transferred, 0) };
            let result = if ok == 0 {
                Err(last_error(failure_message))
            } else {
                Ok((buffer, transferred as usize))
            };
            close_event(&overlapped);
            return result;
        }
        // SAFETY: reads this thread's last-error value.
        let error = unsafe { GetLastError() };
        if error != ERROR_IO_PENDING {
            close_event(&overlapped);
            return Err(io::Error::new(
                io::Error::from_raw_os_error(error as i32).kind(),
                format!(
                    "{failure_message}: {}",
                    io::Error::from_raw_os_error(error as i32)
                ),
            ));
        }
        let (finished, error) = match deadline {
            None => {
                // SAFETY: waits for our own request on our own handle.
                let ok = unsafe {
                    GetOverlappedResult(self.handle.0, &*overlapped, &mut transferred, 1)
                };
                // SAFETY: reads this thread's last-error value.
                (
                    ok,
                    if ok == 0 {
                        unsafe { GetLastError() }
                    } else {
                        0
                    },
                )
            }
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    // Python: a deadline already passed counts as WAIT_TIMEOUT.
                    (0, WAIT_TIMEOUT)
                } else {
                    let millis = (remaining.as_secs_f64() * 1000.0) as u64;
                    let millis = millis.clamp(1, 0xFFFF_FFFE) as u32;
                    // SAFETY: as above, bounded by `millis`.
                    let ok = unsafe {
                        GetOverlappedResultEx(
                            self.handle.0,
                            &*overlapped,
                            &mut transferred,
                            millis,
                            0,
                        )
                    };
                    // SAFETY: reads this thread's last-error value.
                    (
                        ok,
                        if ok == 0 {
                            unsafe { GetLastError() }
                        } else {
                            0
                        },
                    )
                }
            }
        };
        if finished == 0 {
            if error == WAIT_TIMEOUT {
                self.cancel_timed_out_io(buffer, overlapped);
                return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
            }
            close_event(&overlapped);
            let os = io::Error::from_raw_os_error(error as i32);
            return Err(io::Error::new(
                os.kind(),
                format!("{failure_message}: {os}"),
            ));
        }
        close_event(&overlapped);
        Ok((buffer, transferred as usize))
    }

    /// Read exactly `size` bytes. A zero-byte read is end of file
    /// (`UnexpectedEof`); a passed deadline is `TimedOut`.
    pub fn recv_exact(&mut self, size: usize) -> io::Result<Vec<u8>> {
        let deadline = self.io_timeout.map(|timeout| Instant::now() + timeout);
        let mut chunks = Vec::with_capacity(size.min(IO_CHUNK_BYTES));
        while chunks.len() < size {
            let read_size = (size - chunks.len()).min(IO_CHUNK_BYTES);
            let (buffer, read) = self.overlapped_io(
                false,
                vec![0u8; read_size].into_boxed_slice(),
                deadline,
                "operator named-pipe read timed out",
                "ReadFile from operator pipe failed",
            )?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "operator named pipe closed",
                ));
            }
            chunks.extend_from_slice(&buffer[..read]);
        }
        Ok(chunks)
    }

    /// Write all of `payload` in 64 KiB chunks before the deadline.
    pub fn send_all(&mut self, payload: &[u8]) -> io::Result<()> {
        let deadline = self.io_timeout.map(|timeout| Instant::now() + timeout);
        let mut offset = 0;
        while offset < payload.len() {
            let end = (offset + IO_CHUNK_BYTES).min(payload.len());
            let (_, written) = self.overlapped_io(
                true,
                payload[offset..end].to_vec().into_boxed_slice(),
                deadline,
                "operator named-pipe write timed out",
                "WriteFile to operator pipe failed",
            )?;
            if written == 0 {
                return Err(io::Error::other(
                    "WriteFile to operator pipe wrote zero bytes",
                ));
            }
            offset += written;
        }
        Ok(())
    }

    pub fn close(&mut self) {
        self.close_handle();
    }
}

impl Drop for PipeStream {
    fn drop(&mut self) {
        self.close_handle();
    }
}

/// Connect to `endpoint`, retrying a busy pipe until `timeout`, and report
/// the server process's SID. A failed SID lookup closes the pipe.
pub fn connect(
    endpoint: &str,
    timeout: Duration,
    io_timeout: Option<Duration>,
) -> io::Result<(PipeStream, String)> {
    let name = wide(OsStr::new(endpoint));
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: a NUL-terminated name; the handle is checked below.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            let stream = PipeStream::new(handle, io_timeout);
            let mut pid = 0u32;
            // SAFETY: the pipe handle is open; pid is writable.
            if unsafe { GetNamedPipeServerProcessId(handle, &mut pid) } == 0 {
                return Err(last_error("GetNamedPipeServerProcessId failed"));
            }
            let sid = process_sid(pid)?;
            return Ok((stream, sid));
        }
        // SAFETY: reads this thread's last-error value.
        let error = unsafe { GetLastError() };
        if error != ERROR_PIPE_BUSY || Instant::now() >= deadline {
            let os = io::Error::from_raw_os_error(error as i32);
            return Err(io::Error::new(
                os.kind(),
                format!("could not connect to operator named pipe: {os}"),
            ));
        }
        // SAFETY: a NUL-terminated name.
        unsafe {
            WaitNamedPipeW(name.as_ptr(), 100);
        }
    }
}

struct ListenerState {
    closed: bool,
    pending: Option<RawHandle>,
}

/// The broker's pipe: the first instance wins the per-user name, and each
/// accept keeps one further instance waiting.
pub struct PipeListener {
    name: Vec<u16>,
    state: Mutex<ListenerState>,
}

/// A connected client with what the OS reports about it. The caller
/// compares `sid` and calls [`PipeStream::reject`] on a mismatch.
pub struct Accepted {
    pub stream: PipeStream,
    pub sid: String,
    /// `GetNamedPipeClientProcessId`, when above zero.
    pub pid: Option<u32>,
}

impl PipeListener {
    /// Create the first instance; `AlreadyRunning` when another process
    /// holds the name. `sid` is the only account the DACL admits.
    pub fn bind(endpoint: &str, sid: &str) -> Result<Self, CreateError> {
        let name = wide(OsStr::new(endpoint));
        let first = create_instance(&name, sid, true)?;
        Ok(PipeListener {
            name,
            state: Mutex::new(ListenerState {
                closed: false,
                pending: Some(first),
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ListenerState> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Wait for one client, then put a fresh instance in its place. The
    /// connected instance is closed exactly once even if [`close`] races
    /// the wait (`test_windows_listener_close_does_not_double_close_connected_handle`).
    ///
    /// [`close`]: PipeListener::close
    pub fn accept(&self, sid: &str, io_timeout: Option<Duration>) -> io::Result<Accepted> {
        let handle = {
            let mut state = self.lock();
            if state.closed {
                return Err(io::Error::other("operator named-pipe listener is closed"));
            }
            if state.pending.is_none() {
                state.pending = Some(create_instance(&self.name, sid, false).map_err(into_io)?);
            }
            state.pending.expect("pending instance")
        };
        let connected = (|| {
            let overlapped = new_overlapped()?;
            // SAFETY: `overlapped` is boxed and outlives the wait below.
            let connected =
                unsafe { ConnectNamedPipe(handle.0, &*overlapped as *const _ as *mut _) };
            let result = if connected == 0 {
                // SAFETY: reads this thread's last-error value.
                let error = unsafe { GetLastError() };
                if error == ERROR_IO_PENDING {
                    let mut transferred = 0u32;
                    // SAFETY: waits for our own connect request.
                    if unsafe { GetOverlappedResult(handle.0, &*overlapped, &mut transferred, 1) }
                        == 0
                    {
                        Err(last_error("ConnectNamedPipe failed"))
                    } else {
                        Ok(())
                    }
                } else if error != ERROR_PIPE_CONNECTED {
                    let os = io::Error::from_raw_os_error(error as i32);
                    Err(io::Error::new(
                        os.kind(),
                        format!("ConnectNamedPipe failed: {os}"),
                    ))
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            };
            close_event(&overlapped);
            result
        })();
        if let Err(error) = connected {
            let mut state = self.lock();
            if state.pending == Some(handle) {
                state.pending = None;
                // SAFETY: the listener still owned this instance.
                unsafe {
                    CloseHandle(handle.0);
                }
            }
            return Err(error);
        }
        {
            let mut state = self.lock();
            if state.closed {
                if state.pending == Some(handle) {
                    state.pending = None;
                    // SAFETY: as above.
                    unsafe {
                        CloseHandle(handle.0);
                    }
                }
                return Err(io::Error::other("operator named-pipe listener is closed"));
            }
            if state.pending != Some(handle) {
                return Err(io::Error::other(
                    "operator named-pipe listener ownership changed",
                ));
            }
            state.pending = None;
        }
        let replacement = match create_instance(&self.name, sid, false) {
            Ok(replacement) => replacement,
            Err(error) => {
                // SAFETY: this accept now owns the connected instance.
                unsafe {
                    DisconnectNamedPipe(handle.0);
                    CloseHandle(handle.0);
                }
                return Err(into_io(error));
            }
        };
        {
            let mut state = self.lock();
            if state.closed {
                // SAFETY: both handles are owned here.
                unsafe {
                    CloseHandle(replacement.0);
                    CloseHandle(handle.0);
                }
                return Err(io::Error::other("operator named-pipe listener is closed"));
            }
            state.pending = Some(replacement);
        }
        let stream = PipeStream::new(handle.0, io_timeout);
        let mut pid = 0u32;
        // SAFETY: the connected handle is open; pid is writable.
        if unsafe { GetNamedPipeClientProcessId(handle.0, &mut pid) } == 0 {
            let error = last_error("GetNamedPipeClientProcessId failed");
            stream.reject();
            return Err(error);
        }
        let client_sid = match process_sid(pid) {
            Ok(client_sid) => client_sid,
            Err(error) => {
                stream.reject();
                return Err(error);
            }
        };
        Ok(Accepted {
            stream,
            sid: client_sid,
            pid: (pid > 0).then_some(pid),
        })
    }

    /// Stop accepting; a blocked [`PipeListener::accept`] returns an error.
    pub fn close(&self) {
        let mut state = self.lock();
        if state.closed {
            return;
        }
        state.closed = true;
        if let Some(pending) = state.pending.take() {
            // SAFETY: the listener owned this waiting instance.
            unsafe {
                CloseHandle(pending.0);
            }
        }
    }
}

impl Drop for PipeListener {
    fn drop(&mut self) {
        self.close();
    }
}

fn into_io(error: CreateError) -> io::Error {
    match error {
        CreateError::AlreadyRunning => io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32),
        CreateError::Io(error) => error,
    }
}

fn create_instance(name: &[u16], sid: &str, first: bool) -> Result<RawHandle, CreateError> {
    let sddl = wide(OsStr::new(&format!("D:P(A;;GA;;;{sid})")));
    let mut descriptor = ptr::null_mut();
    // SAFETY: a NUL-terminated SDDL string; the descriptor is freed below.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(CreateError::Io(last_error(
            "could not construct operator pipe DACL",
        )));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let mut access = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        access |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: valid name and attributes; the descriptor outlives the call.
    let handle: HANDLE = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            access,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            &attributes,
        )
    };
    // SAFETY: reads this thread's last-error value before LocalFree.
    let creation_error = if handle == INVALID_HANDLE_VALUE {
        unsafe { GetLastError() }
    } else {
        0
    };
    // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe {
        LocalFree(descriptor);
    }
    if handle == INVALID_HANDLE_VALUE {
        if first && creation_error == ERROR_ACCESS_DENIED {
            return Err(CreateError::AlreadyRunning);
        }
        let os = io::Error::from_raw_os_error(creation_error as i32);
        return Err(CreateError::Io(io::Error::new(
            os.kind(),
            format!("could not create operator named pipe: {os}"),
        )));
    }
    Ok(RawHandle(handle))
}
