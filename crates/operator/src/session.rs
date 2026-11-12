//! The Linux login-lifetime guard (`server.py` `_login_session_runtime_*`,
//! `_linux_logind_user_alive`).
//!
//! A broker must not outlive the login that unlocked it. Linux offers two
//! witnesses: logind's per-user state, and the volatile `/run/user/<uid>`
//! runtime directory (with its original device and inode). Python asks
//! `sd_uid_get_state` through `libsystemd`; this reads the file that call
//! parses, `/run/systemd/users/<uid>`, whose absence `sd_uid_get_state`
//! reports as `offline`. A host without a running systemd is "unknown", as
//! Python's missing library is. Other platforms end user processes at logoff
//! themselves.

use std::path::{Path, PathBuf};

/// The runtime directory the broker started under, pinned by identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginRuntime {
    pub root: PathBuf,
    pub device: u64,
    pub inode: u64,
}

/// Whether a directory is the OS-managed runtime directory
/// (`_login_session_runtime_path_is_trusted`); a seam so the integration
/// fixture can trust its private directory.
pub type TrustRuntime = dyn Fn(&Path, &std::fs::Metadata) -> bool + Send + Sync;

/// `_login_session_runtime_root()`.
pub fn login_session_runtime_root(
    configured: Option<&std::ffi::OsStr>,
    trust: &TrustRuntime,
) -> Option<LoginRuntime> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let configured = configured.filter(|value| !value.is_empty())?;
    let root = std::fs::canonicalize(configured).ok()?;
    let info = std::fs::metadata(&root).ok()?;
    let runtime = LoginRuntime {
        root,
        device: device_of(&info),
        inode: inode_of(&info),
    };
    login_session_runtime_is_valid(Some(&runtime), trust).then_some(runtime)
}

/// `_login_session_runtime_is_valid(runtime)`.
pub fn login_session_runtime_is_valid(
    runtime: Option<&LoginRuntime>,
    trust: &TrustRuntime,
) -> bool {
    let Some(runtime) = runtime else {
        return false;
    };
    let Ok(info) = std::fs::metadata(&runtime.root) else {
        return false;
    };
    trust(&runtime.root, &info)
        && device_of(&info) == runtime.device
        && inode_of(&info) == runtime.inode
}

#[cfg(unix)]
fn device_of(info: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::dev(info)
}

#[cfg(unix)]
fn inode_of(info: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::ino(info)
}

#[cfg(not(unix))]
fn device_of(_: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(not(unix))]
fn inode_of(_: &std::fs::Metadata) -> u64 {
    0
}

/// `_login_session_runtime_path_is_trusted`: `/run/user/<uid>`, a 0700
/// directory this user owns, on a memory-backed mount.
#[cfg(unix)]
pub fn runtime_path_is_trusted(uid: u32, root: &Path, info: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    root == Path::new("/run/user").join(uid.to_string())
        && info.is_dir()
        && info.uid() == uid
        && info.permissions().mode() & 0o7777 == 0o700
        && runtime_filesystem_is_volatile(
            root,
            &std::fs::read_to_string("/proc/self/mountinfo").ok(),
        )
}

/// `_linux_runtime_filesystem_is_volatile`: the deepest mount holding
/// `root` is `tmpfs` or `ramfs`.
pub fn runtime_filesystem_is_volatile(root: &Path, mountinfo: &Option<String>) -> bool {
    let Some(mountinfo) = mountinfo else {
        return false;
    };
    let mut best: Option<(usize, &str)> = None;
    for line in mountinfo.lines() {
        let Some((mount, filesystem)) = line.split_once(" - ") else {
            continue;
        };
        let mount_fields: Vec<&str> = mount.split_whitespace().collect();
        let Some(filesystem) = filesystem.split_whitespace().next() else {
            continue;
        };
        if mount_fields.len() < 5 {
            continue;
        }
        let mount_point = Path::new(mount_fields[4]);
        if root != mount_point && !root.ancestors().skip(1).any(|parent| parent == mount_point) {
            continue;
        }
        let depth = mount_point.components().count();
        if best.is_none_or(|(deepest, _)| depth > deepest) {
            best = Some((depth, filesystem));
        }
    }
    best.is_some_and(|(_, filesystem)| matches!(filesystem, "ramfs" | "tmpfs"))
}

/// `_linux_session_lifetime_is_valid`.
pub fn session_lifetime_is_valid(
    runtime: Option<&LoginRuntime>,
    logind_observed: bool,
    logind_alive: Option<bool>,
    trust: &TrustRuntime,
) -> bool {
    if runtime.is_some() && !login_session_runtime_is_valid(runtime, trust) {
        return false;
    }
    if logind_alive == Some(false) {
        return false;
    }
    if runtime.is_some() {
        return true;
    }
    logind_observed && logind_alive == Some(true)
}

/// `_logind_user_state_is_alive`: `online` is a logged-in user without a
/// foreground session; `closing`, `lingering` and `offline` are not.
pub fn logind_user_state_is_alive(state: &str) -> bool {
    matches!(state, "online" | "active")
}

/// `_linux_logind_user_alive()`, from systemd's own files: `None` when
/// systemd is not running or the record cannot be read as `sd_uid_get_state`
/// would fail.
pub fn logind_user_alive(systemd_root: &Path, uid: u32) -> Option<bool> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    // sd_booted(): /run/systemd/system exists while systemd runs.
    if !systemd_root.join("system").is_dir() {
        return None;
    }
    let state = sd_uid_get_state(&systemd_root.join("users").join(uid.to_string()))?;
    Some(logind_user_state_is_alive(&state))
}

/// `sd_uid_get_state`: `STATE=` of the user record; a missing record is
/// `offline`, an empty state an error.
pub fn sd_uid_get_state(record: &Path) -> Option<String> {
    let text = match std::fs::read(record) {
        Ok(bytes) => String::from_utf8(bytes).ok()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some("offline".into())
        }
        Err(_) => return None,
    };
    let state = env_file_value(&text, "STATE")?;
    (!state.is_empty()).then_some(state)
}

/// `parse_env_file(NULL, path, key, &value)` for one key: `KEY=value` lines,
/// `#`/`;` comments, quotes and backslash escapes; the last assignment wins.
/// `None` when the key is absent (systemd leaves the value unset).
fn env_file_value(text: &str, key: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        found = Some(unquote(value.trim()));
    }
    found
}

fn unquote(value: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(open), c) if c == open => quote = None,
            (Some('"') | None, '\\') => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            (_, c) => out.push(c),
        }
    }
    out
}

/// The account's uid.
#[cfg(unix)]
pub fn current_uid() -> u32 {
    kassiber_ipc::unix::current_uid()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trust_all() -> Box<TrustRuntime> {
        Box::new(|_, _| true)
    }

    #[test]
    fn logind_user_state_distinguishes_login_from_linger() {
        assert!(logind_user_state_is_alive("online"));
        assert!(logind_user_state_is_alive("active"));
        for state in ["closing", "lingering", "offline", ""] {
            assert!(!logind_user_state_is_alive(state));
        }
    }

    #[test]
    fn user_records_parse_as_sd_uid_get_state() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join("1000");
        // A missing record is offline, not an error.
        assert_eq!(sd_uid_get_state(&record).as_deref(), Some("offline"));
        // The vector systemd writes (src/login/logind-user.c, user_save).
        std::fs::write(
            &record,
            "# This is private data. Do not parse.\nNAME=alice\nSTATE=active\nSTOPPING=no\nRUNTIME=/run/user/1000\n",
        )
        .unwrap();
        assert_eq!(sd_uid_get_state(&record).as_deref(), Some("active"));
        std::fs::write(&record, "STATE=\"lingering\"\n").unwrap();
        assert_eq!(sd_uid_get_state(&record).as_deref(), Some("lingering"));
        std::fs::write(&record, "STATE=online\nSTATE=closing\n").unwrap();
        assert_eq!(sd_uid_get_state(&record).as_deref(), Some("closing"));
        // No state, or an empty one, is an error (-EIO / unset).
        std::fs::write(&record, "NAME=alice\n").unwrap();
        assert_eq!(sd_uid_get_state(&record), None);
        std::fs::write(&record, "STATE=\n").unwrap();
        assert_eq!(sd_uid_get_state(&record), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn logind_is_unknown_without_a_running_systemd() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(logind_user_alive(root.path(), 1000), None);
        std::fs::create_dir(root.path().join("system")).unwrap();
        std::fs::create_dir(root.path().join("users")).unwrap();
        // A missing user record is offline: logged out.
        assert_eq!(logind_user_alive(root.path(), 1000), Some(false));
        std::fs::write(root.path().join("users/1000"), "STATE=online\n").unwrap();
        assert_eq!(logind_user_alive(root.path(), 1000), Some(true));
        std::fs::write(root.path().join("users/1000"), "STATE=\n").unwrap();
        assert_eq!(logind_user_alive(root.path(), 1000), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn session_runtime_disappearance_invalidates_session() {
        let root = tempfile::tempdir().unwrap();
        let runtime =
            login_session_runtime_root(Some(root.path().as_os_str()), &*trust_all()).unwrap();
        assert_eq!(runtime.root, std::fs::canonicalize(root.path()).unwrap());
        assert!(login_session_runtime_is_valid(
            Some(&runtime),
            &*trust_all()
        ));
        drop(root);
        assert!(!login_session_runtime_is_valid(
            Some(&runtime),
            &*trust_all()
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn session_runtime_replacement_invalidates_session() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("runtime");
        std::fs::create_dir(&root).unwrap();
        let runtime = login_session_runtime_root(Some(root.as_os_str()), &*trust_all()).unwrap();
        std::fs::rename(&root, parent.path().join("displaced-runtime")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(!login_session_runtime_is_valid(
            Some(&runtime),
            &*trust_all()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_fallback_requires_the_canonical_volatile_owner_directory() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let info = std::fs::metadata(directory.path()).unwrap();
        let uid = current_uid();
        // Only /run/user/<uid> is trusted; a caller-selected directory is not.
        assert!(!runtime_path_is_trusted(uid, directory.path(), &info));
        let mountinfo = Some(
            "22 1 0:21 / / rw - ext4 /dev/sda1 rw\n30 22 0:25 / /run rw - tmpfs tmpfs rw\n31 30 0:26 / /run/user/1000 rw - tmpfs tmpfs rw\n40 22 0:30 / /srv rw - ext4 /dev/sdb1 rw\n"
                .to_owned(),
        );
        assert!(runtime_filesystem_is_volatile(
            Path::new("/run/user/1000"),
            &mountinfo
        ));
        assert!(runtime_filesystem_is_volatile(
            Path::new("/run/other"),
            &mountinfo
        ));
        assert!(!runtime_filesystem_is_volatile(
            Path::new("/srv/user"),
            &mountinfo
        ));
        assert!(!runtime_filesystem_is_volatile(
            Path::new("/home/user"),
            &mountinfo
        ));
        assert!(!runtime_filesystem_is_volatile(
            Path::new("/run/user/1000"),
            &None
        ));
        // A prefix that is not a path ancestor does not count.
        assert!(!runtime_filesystem_is_volatile(
            Path::new("/srvx"),
            &Some("1 1 0:1 / /srv rw - tmpfs t rw\n".into())
        ));
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let info = std::fs::metadata(directory.path()).unwrap();
        assert!(!runtime_path_is_trusted(uid, directory.path(), &info));
    }

    #[test]
    fn logind_guard_fails_closed_after_query_loss() {
        assert!(!session_lifetime_is_valid(None, true, None, &*trust_all()));
        assert!(session_lifetime_is_valid(
            None,
            true,
            Some(true),
            &*trust_all()
        ));
        assert!(!session_lifetime_is_valid(None, false, None, &*trust_all()));
        assert!(!session_lifetime_is_valid(
            None,
            true,
            Some(false),
            &*trust_all()
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_guard_can_cover_transient_logind_query_loss() {
        let root = tempfile::tempdir().unwrap();
        let runtime =
            login_session_runtime_root(Some(root.path().as_os_str()), &*trust_all()).unwrap();
        assert!(session_lifetime_is_valid(
            Some(&runtime),
            true,
            None,
            &*trust_all()
        ));
        // A logout logind reports still ends it.
        assert!(!session_lifetime_is_valid(
            Some(&runtime),
            true,
            Some(false),
            &*trust_all()
        ));
    }

    #[test]
    fn non_linux_has_no_guards() {
        if cfg!(target_os = "linux") {
            return;
        }
        assert_eq!(
            login_session_runtime_root(Some("/tmp".as_ref()), &*trust_all()),
            None
        );
        assert_eq!(logind_user_alive(Path::new("/run/systemd"), 0), None);
    }
}
