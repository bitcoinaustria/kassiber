//! Display names for the processes behind agent sessions
//! (`kassiber/operator/peers.py`).
//!
//! The label names the program that started a `kassiber mcp serve` process
//! (`claude`, `codex`) so the user can recognize it before allowing it. It is
//! best effort and not an identity; pairing binds to the process id the OS
//! reports. Linux reads `/proc`. macOS asks `/bin/ps` in Python; that launch
//! is not ported here (R6-3), so macOS and Windows report no label and the
//! process id still shows.

use std::path::Path;

/// Launchers between the agent host and the MCP server itself.
const SKIPPED_NAMES: &[&str] = &[
    "kassiber", "python", "python3", "uv", "uvx", "sh", "bash", "zsh",
];
const MAX_ANCESTORS: usize = 4;
const MAX_LABEL_CHARS: usize = 32;

/// `process_label(pid)`.
pub fn process_label(pid: Option<u32>) -> Option<String> {
    process_label_in(Path::new("/proc"), pid)
}

/// `process_label` over a `/proc`-shaped tree.
pub fn process_label_in(proc_root: &Path, pid: Option<u32>) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let pid = pid.filter(|pid| *pid > 0)?;
    let mut current = parent(proc_root, pid)?;
    for _ in 0..MAX_ANCESTORS {
        if current <= 1 {
            return None;
        }
        let name = name(proc_root, current)?;
        if !SKIPPED_NAMES.contains(&normalized(&name).as_str()) {
            return clean(&name);
        }
        current = parent(proc_root, current)?;
    }
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

/// `/proc/<pid>/stat`'s parent pid: fields resume after the last `)`.
fn parent(proc_root: &Path, pid: u32) -> Option<u32> {
    let bytes = std::fs::read(proc_root.join(pid.to_string()).join("stat")).ok()?;
    let stat = String::from_utf8_lossy(&bytes);
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// `/proc/<pid>/comm`.
fn name(proc_root: &Path, pid: u32) -> Option<String> {
    let bytes = std::fs::read(proc_root.join(pid.to_string()).join("comm")).ok()?;
    let name = kassiber_pytext::strip(&String::from_utf8_lossy(&bytes)).to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

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
    }
}
