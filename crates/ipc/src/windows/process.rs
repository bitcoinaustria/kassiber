//! `CreateProcessW` with `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.

use super::{last_error, wide, RawHandle};
use crate::process::{list2cmdline, Child, ChildCommand, Stdio};
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;
use std::ptr;
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, SetHandleInformation, DUPLICATE_SAME_ACCESS, GENERIC_READ,
    GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, DETACHED_PROCESS,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

/// Launches through this module never overlap, so one launch's temporarily
/// inheritable handles cannot reach another's child.
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct Process {
    handle: RawHandle,
    pid: u32,
    status: Option<ExitStatus>,
}

impl Process {
    pub(crate) fn id(&self) -> u32 {
        self.pid
    }

    fn exit_code(&self) -> io::Result<ExitStatus> {
        let mut code = 0u32;
        // SAFETY: the process handle is owned and open.
        if unsafe { GetExitCodeProcess(self.handle.0, &mut code) } == 0 {
            return Err(last_error("GetExitCodeProcess failed"));
        }
        Ok(ExitStatus::from_raw(code))
    }

    pub(crate) fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        // SAFETY: the process handle is owned and open.
        if unsafe { WaitForSingleObject(self.handle.0, INFINITE) } != WAIT_OBJECT_0 {
            return Err(last_error("WaitForSingleObject failed"));
        }
        let status = self.exit_code()?;
        self.status = Some(status);
        Ok(status)
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        // SAFETY: the process handle is owned and open.
        if unsafe { WaitForSingleObject(self.handle.0, 0) } != WAIT_OBJECT_0 {
            return Ok(None);
        }
        let status = self.exit_code()?;
        self.status = Some(status);
        Ok(Some(status))
    }

    /// `Popen.terminate()`/`kill()` on Windows: `TerminateProcess(h, 1)`,
    /// ignoring a process that has already exited.
    pub(crate) fn terminate(&mut self) -> io::Result<()> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        // SAFETY: the process handle is owned and open.
        if unsafe { TerminateProcess(self.handle.0, 1) } == 0 {
            let error = last_error("TerminateProcess failed");
            if self.try_wait()?.is_some() {
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn kill(&mut self) -> io::Result<()> {
        self.terminate()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: CreateProcessW returned this handle to us.
        unsafe {
            CloseHandle(self.handle.0);
        }
    }
}

/// Handles this launch created and must close once the child has them.
struct Owned(Vec<HANDLE>);

impl Drop for Owned {
    fn drop(&mut self) {
        for handle in &self.0 {
            // SAFETY: each handle was created by this launch and is closed once.
            unsafe {
                CloseHandle(*handle);
            }
        }
    }
}

fn inheritable_attributes() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    }
}

fn open_null(owned: &mut Owned) -> io::Result<HANDLE> {
    let name = wide(OsStr::new("NUL"));
    let attributes = inheritable_attributes();
    // SAFETY: NUL-terminated name; the handle is checked and recorded.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &attributes,
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error("could not open the null device"));
    }
    owned.0.push(handle);
    Ok(handle)
}

fn inheritable_duplicate(source: HANDLE, owned: &mut Owned) -> io::Result<HANDLE> {
    let mut duplicate = ptr::null_mut();
    // SAFETY: duplicates a live handle within this process.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(last_error("DuplicateHandle failed"));
    }
    owned.0.push(duplicate);
    Ok(duplicate)
}

/// The child's end of one standard stream, plus the parent's pipe end.
fn standard(
    stdio: Stdio,
    current: Option<HANDLE>,
    child_reads: bool,
    owned: &mut Owned,
) -> io::Result<(HANDLE, Option<File>)> {
    match stdio {
        Stdio::Null => Ok((open_null(owned)?, None)),
        Stdio::Inherit => match current.filter(|h| !h.is_null() && *h != INVALID_HANDLE_VALUE) {
            Some(handle) => Ok((inheritable_duplicate(handle, owned)?, None)),
            None => Ok((open_null(owned)?, None)),
        },
        Stdio::Pipe => {
            let mut read = ptr::null_mut();
            let mut write = ptr::null_mut();
            // SAFETY: both outputs are writable; default (non-inheritable) security.
            if unsafe { CreatePipe(&mut read, &mut write, ptr::null(), 0) } == 0 {
                return Err(last_error("CreatePipe failed"));
            }
            // SAFETY: CreatePipe returned two handles we now own.
            let (read, write) = unsafe {
                (
                    OwnedHandle::from_raw_handle(read),
                    OwnedHandle::from_raw_handle(write),
                )
            };
            let (child, parent) = if child_reads {
                (read, write)
            } else {
                (write, read)
            };
            let child = inheritable_duplicate(child.as_raw_handle(), owned)?;
            Ok((child, Some(File::from(parent))))
        }
    }
}

fn environment_block(command: &ChildCommand) -> Vec<u16> {
    let mut environment = command.environment();
    // CPython sorts the block case-insensitively, as Windows expects.
    environment.sort_by_key(|(key, _)| key.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (key, value) in environment {
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

pub(crate) fn spawn(command: &ChildCommand) -> io::Result<Child> {
    let _serial = SPAWN_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut owned = Owned(Vec::new());
    let (stdin, parent_stdin) = standard(
        command.stdin,
        Some(io::stdin().as_raw_handle()),
        true,
        &mut owned,
    )?;
    let (stdout, parent_stdout) = standard(
        command.stdout,
        Some(io::stdout().as_raw_handle()),
        false,
        &mut owned,
    )?;
    let (stderr, parent_stderr) = standard(
        command.stderr,
        Some(io::stderr().as_raw_handle()),
        false,
        &mut owned,
    )?;

    let mut handles: Vec<HANDLE> = vec![stdin, stdout, stderr];
    for handle in &command.inherit {
        if !handles.contains(handle) {
            handles.push(*handle);
        }
    }
    // Listed handles keep their numbers: mark them inheritable for this
    // launch only and reset them afterwards.
    struct Restore<'a>(&'a [HANDLE]);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            for handle in self.0 {
                // SAFETY: the caller keeps these handles open during spawn.
                unsafe {
                    SetHandleInformation(*handle, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }
    }
    let mut marked = Vec::new();
    for handle in &command.inherit {
        // SAFETY: the caller keeps this handle open during spawn.
        if unsafe { SetHandleInformation(*handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
            let _restore = Restore(&marked);
            return Err(last_error("SetHandleInformation failed"));
        }
        marked.push(*handle);
    }
    let _restore = Restore(&marked);

    let mut size = 0usize;
    // SAFETY: the size query writes only `size`.
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
    }
    let mut storage = vec![0usize; size.div_ceil(size_of::<usize>()).max(1)];
    let list: LPPROC_THREAD_ATTRIBUTE_LIST = storage.as_mut_ptr().cast();
    // SAFETY: `storage` holds at least `size` aligned bytes.
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err(last_error("InitializeProcThreadAttributeList failed"));
    }
    struct List(LPPROC_THREAD_ATTRIBUTE_LIST);
    impl Drop for List {
        fn drop(&mut self) {
            // SAFETY: initialized above and deleted once.
            unsafe { DeleteProcThreadAttributeList(self.0) }
        }
    }
    let _list = List(list);
    // SAFETY: `handles` outlives CreateProcessW below.
    if unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            handles.len() * size_of::<HANDLE>(),
            ptr::null_mut(),
            ptr::null(),
        )
    } == 0
    {
        return Err(last_error("UpdateProcThreadAttribute failed"));
    }

    // SAFETY: STARTUPINFOEXW is plain data; zero is its initial state.
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin;
    startup.StartupInfo.hStdOutput = stdout;
    startup.StartupInfo.hStdError = stderr;
    startup.lpAttributeList = list;

    let mut arguments: Vec<Vec<u16>> = vec![command.program.encode_wide().collect()];
    arguments.extend(command.args.iter().map(|arg| arg.encode_wide().collect()));
    let mut command_line = list2cmdline(&arguments);
    command_line.push(0);
    let environment = environment_block(command);
    let directory = command.cwd.as_ref().map(|cwd| wide(cwd.as_os_str()));
    let mut flags = CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;
    if command.detach {
        flags |= DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    }
    // SAFETY: PROCESS_INFORMATION is plain data filled on success.
    let mut information: PROCESS_INFORMATION = unsafe { zeroed() };
    // SAFETY: every pointer refers to a live, NUL-terminated buffer owned
    // by this function; the handle list holds only open handles.
    let created = unsafe {
        CreateProcessW(
            ptr::null(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags,
            environment.as_ptr().cast(),
            directory.as_ref().map_or(ptr::null(), |d| d.as_ptr()),
            &startup.StartupInfo,
            &mut information,
        )
    };
    if created == 0 {
        return Err(last_error("CreateProcessW failed"));
    }
    // SAFETY: the thread handle is ours and unused.
    unsafe {
        CloseHandle(information.hThread);
    }
    drop(owned);
    Ok(Child {
        stdin: parent_stdin,
        stdout: parent_stdout,
        stderr: parent_stderr,
        inner: Process {
            handle: RawHandle(information.hProcess),
            pid: information.dwProcessId,
            status: None,
        },
    })
}
