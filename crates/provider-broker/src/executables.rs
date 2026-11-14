//! Finding and running provider CLIs (`executables.ts`).
//!
//! [`provider_command`] is the broker's one process launch: every provider
//! CLI, short probe or long-lived agent, starts from it with the environment
//! [`crate::env`] allows and nothing on stdin, stdout or stderr unless the
//! caller asks for a pipe.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::cleanup::track_child;
use crate::env::provider_environment;
use crate::protocol::{BrokerFailure, ProviderId};

/// Where installers put CLIs when the daemon's `PATH` lacks them.
pub fn fallback_dirs() -> Vec<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local").join("bin"),
        home.join(".opencode").join("bin"),
        PathBuf::from("/opt/local/bin"),
    ]
}

/// `/\.[^\\/.]+$/`: the command already names an extension.
fn has_extension(command: &str) -> bool {
    command.rfind('.').is_some_and(|dot| {
        let tail = &command[dot + 1..];
        !tail.is_empty() && !tail.contains(['\\', '/'])
    })
}

/// `executableNames(command, platform)`: file names a bare command can
/// have. Windows installs CLIs as `name.exe`; only `.exe` and `.com` count,
/// because a `.cmd` or `.bat` script needs a shell, and the broker never
/// starts one. `pathext` is `PATHEXT`, `None` when unset.
pub fn executable_names(command: &str, windows: bool, pathext: Option<&str>) -> Vec<String> {
    if !windows || has_extension(command) {
        return vec![command.to_owned()];
    }
    let extensions: Vec<String> = pathext
        .unwrap_or(".COM;.EXE")
        .split(';')
        .map(|ext| ext.trim().to_lowercase())
        .filter(|ext| ext == ".exe" || ext == ".com")
        .collect();
    let extensions = if extensions.is_empty() {
        vec![".exe".to_owned()]
    } else {
        extensions
    };
    extensions
        .into_iter()
        .map(|ext| format!("{command}{ext}"))
        .collect()
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    nix::unistd::access(path, nix::unistd::AccessFlags::X_OK).is_ok()
}

/// Node's `access(path, X_OK)` on Windows only asks whether it exists.
#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.exists()
}

/// `resolveExecutable(command)` over explicit inputs: `path` is `PATH`.
pub fn resolve_in(command: &str, path: Option<&OsStr>, fallbacks: &[PathBuf]) -> Option<PathBuf> {
    let mut directories: Vec<PathBuf> = path
        .map(|path| {
            std::env::split_paths(path)
                .filter(|dir| !dir.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    directories.extend(fallbacks.iter().cloned());
    let names = executable_names(
        command,
        cfg!(windows),
        std::env::var("PATHEXT").ok().as_deref(),
    );
    let mut seen: Vec<&PathBuf> = Vec::new();
    for directory in &directories {
        if seen.contains(&directory) {
            continue;
        }
        seen.push(directory);
        for name in &names {
            let candidate = directory.join(name);
            if executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// `resolveExecutable(command)`: `PATH`, then the fallback directories,
/// each directory once.
pub fn resolve_executable(command: &str) -> Option<PathBuf> {
    resolve_in(
        command,
        std::env::var_os("PATH").as_deref(),
        &fallback_dirs(),
    )
}

/// The libuv code Node names a failed spawn by (`spawn <file> ENOENT`).
pub(crate) fn node_code(error: &io::Error) -> String {
    #[cfg(unix)]
    if let Some(code) = error.raw_os_error() {
        return format!("{:?}", nix::errno::Errno::from_raw(code));
    }
    match error.kind() {
        io::ErrorKind::NotFound => "ENOENT".into(),
        io::ErrorKind::PermissionDenied => "EACCES".into(),
        _ => error.to_string(),
    }
}

/// Node's message for a failed spawn, which `errorCode` maps (`ENOENT` is a
/// missing executable).
pub fn spawn_failure(executable: &Path, error: &io::Error) -> BrokerFailure {
    BrokerFailure::new(format!(
        "spawn {} {}",
        executable.display(),
        node_code(error)
    ))
}

/// Node's message for a failed file-system call (`ENOENT: no such file or
/// directory, mkdtemp '<path>'`).
pub fn fs_failure(operation: &str, path: &Path, error: &io::Error) -> BrokerFailure {
    let code = node_code(error);
    let reason = error.to_string();
    let reason = match reason.find(" (os error") {
        Some(end) => reason[..end].to_lowercase(),
        None => reason.to_lowercase(),
    };
    BrokerFailure::new(format!(
        "{code}: {reason}, {operation} '{}'",
        path.display()
    ))
}

/// A provider CLI with the provider's environment, its arguments and
/// working directory, and no standard streams; callers ask for the pipes
/// they read. `env` replaces the provider environment when given.
pub fn provider_command(
    provider: ProviderId,
    executable: &Path,
    args: &[impl AsRef<OsStr>],
    cwd: Option<&Path>,
    env: Option<&[(OsString, OsString)]>,
) -> Command {
    let mut command = Command::new(executable);
    command.args(args);
    command.env_clear();
    match env {
        Some(env) => command.envs(env.iter().map(|(name, value)| (name, value))),
        None => command.envs(provider_environment(provider)),
    };
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Start a provider command, retrying briefly while exec reports `ETXTBSY`.
///
/// `ETXTBSY` means some process still holds the executable open for
/// writing: a CLI update finishing, or a thread elsewhere in this process
/// that forked while a file was being written and has not exec'd yet. Both
/// pass within moments, and a failed exec starts nothing, so the launch is
/// retried for up to two seconds rather than reported as a failure.
pub fn spawn_provider(command: &mut Command) -> io::Result<Child> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match command.spawn() {
            Err(error) if text_file_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            spawned => return spawned,
        }
    }
}

#[cfg(unix)]
fn text_file_busy(error: &io::Error) -> bool {
    error.raw_os_error() == Some(nix::errno::Errno::ETXTBSY as i32)
}

#[cfg(not(unix))]
fn text_file_busy(_: &io::Error) -> bool {
    false
}

/// Options for [`run_provider`].
#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions<'a> {
    pub cwd: Option<&'a Path>,
    /// Stop keeping stdout after this many bytes (default 512,000).
    pub limit: Option<usize>,
    pub env: Option<&'a [(OsString, OsString)]>,
}

/// What a finished provider CLI printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderOutput {
    /// The exit code; `None` when a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
}

/// Default for [`RunOptions::limit`].
pub const RUN_LIMIT: usize = 512_000;

/// `runProvider`: run a provider CLI to completion and return its stdout.
///
/// Settles when stdout has ended *and* the process has exited (Node's
/// `close`, not `exit`): with several probes running concurrently, settling
/// on exit alone routinely lost stdout still queued behind it, and a probe
/// that succeeded read as "provider not configured".
///
/// stderr is discarded deliberately; provider CLIs put credentials and
/// machine detail there, and `safe_error_message` is the only sanctioned path
/// for surfacing provider failures. Output past the limit is read and
/// dropped, so the CLI never blocks on a full pipe.
pub fn run_provider(
    provider: ProviderId,
    executable: &Path,
    args: &[impl AsRef<OsStr>],
    options: RunOptions<'_>,
) -> Result<ProviderOutput, BrokerFailure> {
    let limit = options.limit.unwrap_or(RUN_LIMIT);
    let mut command = provider_command(provider, executable, args, options.cwd, options.env);
    command.stdout(Stdio::piped());
    let mut child =
        spawn_provider(&mut command).map_err(|error| spawn_failure(executable, &error))?;
    let stdout = child.stdout.take();
    let child = track_child(child);
    let mut kept = Vec::new();
    if let Some(mut stdout) = stdout {
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    if kept.len() < limit {
                        kept.extend_from_slice(&chunk[..read]);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }
    let status = child
        .wait()
        .map_err(|error| BrokerFailure::new(error.to_string()))?;
    Ok(ProviderOutput {
        code: status.code(),
        stdout: String::from_utf8_lossy(&kept).into_owned(),
    })
}

#[cfg(test)]
mod tests;
