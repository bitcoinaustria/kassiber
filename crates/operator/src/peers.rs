//! Display names for the processes behind agent sessions
//! (`kassiber/operator/peers.py`).
//!
//! The label names the program that started a `kassiber mcp serve` process
//! (`claude`, `codex`) so the user can recognize it before allowing it. It is
//! best effort and not an identity; pairing binds to the process id the OS
//! reports. Linux reads `/proc`; macOS asks `/bin/ps` with a 2 s limit, as
//! Python does; Windows has no introspection here, so it reports no label and
//! the process id still shows.

use std::path::Path;
use std::time::Duration;

/// Launchers between the agent host and the MCP server itself.
const SKIPPED_NAMES: &[&str] = &[
    "kassiber", "python", "python3", "uv", "uvx", "sh", "bash", "zsh",
];
const MAX_ANCESTORS: usize = 4;
const MAX_LABEL_CHARS: usize = 32;
/// `subprocess.run(..., timeout=2)` for each `/bin/ps` question.
pub const PS_TIMEOUT: Duration = Duration::from_secs(2);

/// Where parents and names come from. `None` is any failure, which makes
/// the whole label `None` (Python catches `OSError`, `ValueError` and
/// `SubprocessError`).
pub trait ProcessTree {
    fn parent(&self, pid: u32) -> Option<u32>;
    fn name(&self, pid: u32) -> Option<String>;
}

/// `process_label(pid)` on this platform.
pub fn process_label(pid: Option<u32>) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        process_label_with(&Proc(Path::new("/proc")), pid)
    }
    #[cfg(target_os = "macos")]
    {
        process_label_with(&Ps(run_ps), pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// `process_label` over a `/proc`-shaped tree.
pub fn process_label_in(proc_root: &Path, pid: Option<u32>) -> Option<String> {
    process_label_with(&Proc(proc_root), pid)
}

/// `process_label(pid)` over any tree: the first ancestor, at most four
/// up, that is not a launcher.
pub fn process_label_with(tree: &dyn ProcessTree, pid: Option<u32>) -> Option<String> {
    let pid = pid.filter(|pid| *pid > 0)?;
    let mut current = tree.parent(pid)?;
    for _ in 0..MAX_ANCESTORS {
        if current <= 1 {
            return None;
        }
        let name = tree.name(current)?;
        if !SKIPPED_NAMES.contains(&normalized(&name).as_str()) {
            return clean(&name);
        }
        current = tree.parent(current)?;
    }
    // Only launchers found.
    None
}

/// `_normalized`: the basename, lowercased, without `.exe`/`.app`, and
/// `python3.11` as `python3`.
fn normalized(name: &str) -> String {
    let mut base = kassiber_pytext::lower(name.rsplit('/').next().unwrap_or(name));
    for suffix in [".exe", ".app"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.to_owned();
        }
    }
    if base.starts_with("python") {
        return base.split('.').next().unwrap_or_default().to_owned();
    }
    base
}

/// `_clean`: printable characters of the basename, stripped, at most 32.
fn clean(name: &str) -> Option<String> {
    let base = name.rsplit('/').next().unwrap_or(name);
    let printable: String = base
        .chars()
        .filter(|c| kassiber_pytext::is_printable(u32::from(*c)))
        .collect();
    let label: String = kassiber_pytext::strip(&printable)
        .chars()
        .take(MAX_LABEL_CHARS)
        .collect();
    (!label.is_empty()).then_some(label)
}

/// Linux: `/proc/<pid>/stat` and `/proc/<pid>/comm`.
pub struct Proc<'a>(pub &'a Path);

impl ProcessTree for Proc<'_> {
    /// The parent pid: fields resume after the name's last `)`.
    fn parent(&self, pid: u32) -> Option<u32> {
        let bytes = std::fs::read(self.0.join(pid.to_string()).join("stat")).ok()?;
        let stat = String::from_utf8_lossy(&bytes);
        let rest = stat.get(stat.rfind(')')? + 2..)?;
        rest.split_whitespace().nth(1)?.parse().ok()
    }

    fn name(&self, pid: u32) -> Option<String> {
        let bytes = std::fs::read(self.0.join(pid.to_string()).join("comm")).ok()?;
        let name = kassiber_pytext::strip(&String::from_utf8_lossy(&bytes)).to_owned();
        (!name.is_empty()).then_some(name)
    }
}

/// macOS: `/bin/ps -o <field> -p <pid>`. The runner answers the stripped
/// standard output, or `None` when the launch failed or timed out.
pub struct Ps<F: Fn(u32, &str) -> Option<String>>(pub F);

impl<F: Fn(u32, &str) -> Option<String>> ProcessTree for Ps<F> {
    /// `int(output) if output else None`; an unparsable answer is a
    /// `ValueError`, which ends the label.
    fn parent(&self, pid: u32) -> Option<u32> {
        let output = (self.0)(pid, "ppid=")?;
        if output.is_empty() {
            return None;
        }
        let parsed = kassiber_pytext::parse_int(&output).ok()?;
        // A negative parent stops the walk as Python's `current <= 1` does.
        Some(u32::try_from(parsed).unwrap_or(0))
    }

    /// `_ps(pid, "comm=") or None`.
    fn name(&self, pid: u32) -> Option<String> {
        (self.0)(pid, "comm=").filter(|name| !name.is_empty())
    }
}

/// `_ps(pid, field)`: `/bin/ps` in the C locale with a fixed `PATH`, its
/// output decoded strictly as UTF-8 (`text=True`) and stripped. A launch
/// that outlives [`PS_TIMEOUT`] is killed and answers `None`.
#[cfg(unix)]
pub fn run_ps(pid: u32, field: &str) -> Option<String> {
    use kassiber_ipc::process::{ChildCommand, Stdio};
    use std::io::Read;
    use std::time::Instant;

    let mut command = ChildCommand::new("/bin/ps");
    command
        .args(["-o", field, "-p", &pid.to_string()])
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::Null)
        .stdout(Stdio::Pipe)
        .stderr(Stdio::Null);
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + PS_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
        }
    }
    let bytes = reader.join().ok()?.ok()?;
    let text = String::from_utf8(bytes).ok()?;
    // Universal newlines turn CR into LF; both are stripped or dropped as
    // unprintable later, so the label is the same.
    Some(kassiber_pytext::strip(&text).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[cfg(target_os = "linux")]
    fn process(root: &Path, pid: u32, ppid: u32, comm: &str) {
        let directory = root.join(pid.to_string());
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("stat"),
            format!("{pid} ({comm} (odd)) S {ppid} 1 1 0 -1"),
        )
        .unwrap();
        std::fs::write(directory.join("comm"), format!("{comm}\n")).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn launchers_are_skipped_up_to_four_ancestors() {
        let root = tempfile::tempdir().unwrap();
        process(root.path(), 50, 40, "kassiber");
        process(root.path(), 40, 30, "python3.11");
        process(root.path(), 30, 20, "uv");
        process(root.path(), 20, 10, "claude");
        process(root.path(), 10, 1, "systemd");
        assert_eq!(
            process_label_in(root.path(), Some(50)).as_deref(),
            Some("claude")
        );
        // Only launchers within reach: no label.
        process(root.path(), 20, 15, "bash");
        process(root.path(), 15, 10, "zsh");
        assert_eq!(process_label_in(root.path(), Some(50)), None);
        assert_eq!(process_label_in(root.path(), None), None);
        assert_eq!(process_label_in(root.path(), Some(0)), None);
        // A label is at most 32 printable characters.
        process(root.path(), 40, 30, &"x".repeat(40));
        assert_eq!(
            process_label_in(root.path(), Some(50)).as_deref(),
            Some("x".repeat(32).as_str())
        );
        // The parent chain reaching init has no label.
        process(root.path(), 60, 1, "kassiber");
        assert_eq!(process_label_in(root.path(), Some(60)), None);
        // A missing process has no label.
        assert_eq!(process_label_in(root.path(), Some(12345)), None);
    }

    /// `ProcessLabelTest.test_the_label_names_the_program_above_launchers`
    /// over `/bin/ps` answers, as macOS sees them.
    #[test]
    fn ps_labels_name_the_program_above_launchers() {
        let answers: HashMap<(u32, &str), Option<&str>> = HashMap::from([
            ((11, "ppid="), Some("10")),
            ((10, "comm="), Some("/opt/homebrew/bin/kassiber")),
            ((10, "ppid="), Some("   9")),
            ((9, "comm="), Some("uv")),
            ((9, "ppid="), Some("8")),
            (
                (8, "comm="),
                Some("/Applications/Claude.app/Contents/MacOS/Claude"),
            ),
            ((8, "ppid="), Some("1")),
            ((20, "ppid="), Some("")),
            ((21, "ppid="), Some("ten")),
            ((22, "ppid="), Some("23")),
            ((23, "comm="), Some("")),
            ((24, "ppid="), None),
        ]);
        let asked = RefCell::new(Vec::new());
        let tree = Ps(|pid: u32, field: &str| {
            asked.borrow_mut().push((pid, field.to_owned()));
            answers
                .get(&(pid, field))
                .copied()
                .flatten()
                .map(str::to_owned)
        });
        assert_eq!(
            process_label_with(&tree, Some(11)).as_deref(),
            Some("Claude")
        );
        assert_eq!(
            *asked.borrow(),
            [
                (11, "ppid=".to_owned()),
                (10, "comm=".to_owned()),
                (10, "ppid=".to_owned()),
                (9, "comm=".to_owned()),
                (9, "ppid=".to_owned()),
                (8, "comm=".to_owned()),
            ]
        );
        // An empty, unparsable or failed answer, or an empty name: no label.
        for pid in [20, 21, 22, 24] {
            assert_eq!(process_label_with(&tree, Some(pid)), None, "{pid}");
        }
        assert_eq!(process_label_with(&tree, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn ps_runs_bin_ps_or_answers_none() {
        if !Path::new("/bin/ps").exists() {
            return;
        }
        let own = std::process::id();
        let parent = run_ps(own, "ppid=").expect("ps answers for this process");
        assert!(kassiber_pytext::parse_int(&parent).is_ok(), "{parent:?}");
        // A process that does not exist prints nothing.
        assert_eq!(run_ps(u32::MAX >> 1, "ppid=").as_deref(), Some(""));
    }
}
