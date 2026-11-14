//! Per-turn state that must not outlive the broker (`cleanup.ts`).
//!
//! The daemon ends a broker by signalling its process group. The working
//! directory and tool bridge would then stay in the temporary folder, and an
//! ACP agent's per-turn home holds its session log with the conversation and
//! tool results. The broker therefore registers both here and clears them on
//! SIGTERM, SIGINT and SIGHUP before it exits with `128 + signal`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Entries {
    directories: Vec<(u64, PathBuf)>,
    children: Vec<(u64, Arc<Mutex<Child>>)>,
}

/// A set of tracked directories and children. The process has one,
/// [`PROCESS`], which the free functions use; tests make their own so a
/// cleanup in one test cannot reach another's state.
pub struct Cleanup {
    entries: Mutex<Entries>,
    next: AtomicU64,
}

/// The broker process's registry.
pub static PROCESS: Cleanup = Cleanup::new();

impl Cleanup {
    pub const fn new() -> Self {
        Self {
            entries: Mutex::new(Entries {
                directories: Vec::new(),
                children: Vec::new(),
            }),
            next: AtomicU64::new(1),
        }
    }

    fn entries(&self) -> MutexGuard<'_, Entries> {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst)
    }

    /// `trackDirectory(path)`: remove `path` on termination until untracked.
    pub fn track_directory(&'static self, path: &Path) -> TrackedDirectory {
        let id = self.id();
        self.entries().directories.push((id, path.to_path_buf()));
        TrackedDirectory { id, registry: self }
    }

    /// `trackChild(child)`: kill `child` on termination until it has exited.
    /// Take its pipes before tracking it.
    pub fn track_child(&'static self, child: Child) -> TrackedChild {
        let id = self.id();
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        self.entries().children.push((id, Arc::clone(&child)));
        TrackedChild {
            id,
            pid,
            child,
            registry: self,
        }
    }

    /// `cleanUpNow()`: stop agents first so none can write into a directory
    /// being removed, then remove the directories (best effort, a few
    /// retries).
    pub fn clean_up_now(&self) {
        let (children, directories) = {
            let mut entries = self.entries();
            (
                std::mem::take(&mut entries.children),
                std::mem::take(&mut entries.directories),
            )
        };
        for (_, child) in children {
            let mut child = child.lock().unwrap_or_else(|poison| poison.into_inner());
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
        }
        for (_, path) in directories {
            for attempt in 0..4 {
                match std::fs::remove_dir_all(&path) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound && attempt < 3 => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    _ => break,
                }
            }
        }
    }
}

impl Default for Cleanup {
    fn default() -> Self {
        Self::new()
    }
}

/// A tracked directory; [`TrackedDirectory::untrack`] (or dropping it)
/// forgets it once its turn removed it.
#[must_use = "dropping the guard untracks the directory"]
pub struct TrackedDirectory {
    id: u64,
    registry: &'static Cleanup,
}

impl TrackedDirectory {
    /// The returned `untrack` of `trackDirectory`.
    pub fn untrack(self) {}
}

impl Drop for TrackedDirectory {
    fn drop(&mut self) {
        self.registry
            .entries()
            .directories
            .retain(|(id, _)| *id != self.id);
    }
}

/// A tracked child. Waiting never holds the lock a termination needs, so a
/// signal always reaches a running child, and every reap goes through that
/// lock, so a signal never reaches a reused pid.
pub struct TrackedChild {
    id: u64,
    pid: u32,
    child: Arc<Mutex<Child>>,
    registry: &'static Cleanup,
}

const WAIT_POLL: Duration = Duration::from_millis(5);

impl TrackedChild {
    fn lock(&self) -> MutexGuard<'_, Child> {
        self.child
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn forget(&self) {
        self.registry
            .entries()
            .children
            .retain(|(id, _)| *id != self.id);
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    /// The exit status if it has exited; an exited child is forgotten.
    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let status = self.lock().try_wait()?;
        if status.is_some() {
            self.forget();
        }
        Ok(status)
    }

    /// Wait for it to exit.
    pub fn wait(&self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(WAIT_POLL);
        }
    }

    /// Wait at most `timeout`; `None` if it is still running.
    pub fn wait_timeout(&self, timeout: Duration) -> io::Result<Option<ExitStatus>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            std::thread::sleep(left.min(WAIT_POLL));
        }
    }

    /// SIGKILL (`TerminateProcess` on Windows). An exited child is left
    /// alone.
    pub fn kill(&self) -> io::Result<()> {
        let mut child = self.lock();
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        child.kill()
    }

    /// SIGTERM, for adapters that let an agent close its session first.
    #[cfg(unix)]
    pub fn terminate(&self) -> io::Result<()> {
        let mut child = self.lock();
        // Unreaped now means the pid is still this child's while the lock
        // is held.
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        let pid = nix::unistd::Pid::from_raw(i32::try_from(self.pid).unwrap_or(0));
        nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).map_err(io::Error::from)
    }
}

impl Drop for TrackedChild {
    /// A running child stays tracked until termination or exit, as in the
    /// TypeScript broker; an exited one is forgotten.
    fn drop(&mut self) {
        if !matches!(self.lock().try_wait(), Ok(None)) {
            self.forget();
        }
    }
}

/// [`Cleanup::track_directory`] on the process registry.
pub fn track_directory(path: &Path) -> TrackedDirectory {
    PROCESS.track_directory(path)
}

/// [`Cleanup::track_child`] on the process registry.
pub fn track_child(child: Child) -> TrackedChild {
    PROCESS.track_child(child)
}

/// [`Cleanup::clean_up_now`] on the process registry.
pub fn clean_up_now() {
    PROCESS.clean_up_now();
}

/// `cleanUpOnTermination()`: on SIGTERM, SIGINT or SIGHUP clean up and exit
/// with `128 + signal`.
///
/// The handlers only write the signal number to a pipe; a dedicated thread
/// reads it and cleans up as ordinary code. Handlers reset to the default
/// action in exec'd children, so provider CLIs still stop on the daemon's
/// signals (a blocked mask would be inherited instead).
#[cfg(unix)]
pub fn clean_up_on_termination() -> io::Result<()> {
    let signals = kassiber_ipc::signal::install_termination_signals()?;
    std::thread::Builder::new()
        .name("broker-termination".into())
        .spawn(move || {
            if let Ok(signal) = signals.wait() {
                clean_up_now();
                std::process::exit(128 + signal);
            }
        })
        .map(drop)
}

/// Windows has no SIGTERM: the daemon ends the broker with
/// `TerminateProcess`, which runs no code, and its temporary root covers
/// what is left behind.
#[cfg(not(unix))]
pub fn clean_up_on_termination() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
