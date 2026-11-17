//! Where the preferences file lives, and reading and writing it.
//!
//! - Location: `KASSIBER_DESKTOP_PREFS_FILE` (absolute path) wins. Otherwise Tauri's app config
//!   directory for `at.bitcoinaustria.kassiber` (`tauri::path::app_config_dir`):
//!   macOS `~/Library/Application Support/at.bitcoinaustria.kassiber`, Windows
//!   `%APPDATA%\at.bitcoinaustria.kassiber`, Linux `$XDG_CONFIG_HOME/at.bitcoinaustria.kassiber`
//!   (else `~/.config/…`). On macOS that directory is Kassiber's native state root, which must
//!   not gain files while a legacy `~/.kassiber` still holds state that the default-root
//!   migration will move (an existing native root blocks the move), so there the file follows
//!   the CLI's `default_state_root` rule and sits in `~/.kassiber` until the move, which carries
//!   it along.
//! - Tooling runs (`--snap`, `--tree`, `--script`, `--scene`, `--playground`, any headless run)
//!   and unit tests never touch the default file: without the env var they keep nothing.
//! - A path inside the open book's data root is refused (preferences never go into the book).
//! - Reading is tolerant: a missing, oversized, unreadable or malformed file is "no file".
//! - Writing replaces the file atomically (a fresh 0600 temp file in the same directory, synced,
//!   then renamed over it) inside a 0700 directory. Nothing is logged.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The env var tests, scripts and packagers use to pick the file.
pub const ENV: &str = "KASSIBER_DESKTOP_PREFS_FILE";
pub const FILE_NAME: &str = "desktop-ui.json";
const APP_ID: &str = "at.bitcoinaustria.kassiber";
/// A preferences file is a few KB; anything far larger is not ours.
const MAX_BYTES: u64 = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Macos,
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Whether this process is agent tooling (headless renders, scripted runs, scenes, the
/// playground): those never read or write the real preferences.
pub fn tooling_run() -> bool {
    if cfg!(test) || rune::is_headless() {
        return true;
    }
    std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--scene" | "--playground" | "--script" | "--snap" | "--tree"
        )
    })
}

/// The preferences file for this launch, or None to keep preferences in memory only.
pub fn location(data_root: Option<&Path>) -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    // Headless tooling (scenes, snapshots, scripts, the click-through and parity harnesses) keeps
    // preferences in memory even when a preferences file is configured: one run's theme, language
    // or toggles must never leak into the next. `KASSIBER_DESKTOP_PREFS_TOOLING=1` opts a scripted
    // persistence check back in.
    if tooling_run() && env("KASSIBER_DESKTOP_PREFS_TOOLING").is_none() {
        return None;
    }
    let path = match env(ENV) {
        Some(p) => Some(PathBuf::from(p)).filter(|p| p.is_absolute())?,
        None if tooling_run() => return None,
        // Unit tests never read or write the real preferences file.
        None if cfg!(test) => return None,
        None => default_dir(Os::current(), &home()?, env)?.join(FILE_NAME),
    };
    if let Some(root) = data_root
        && inside(&path, root)
    {
        return None;
    }
    Some(path)
}

fn home() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Whether `path` lies in `root` (lexically, and after resolving whatever exists of both).
fn inside(path: &Path, root: &Path) -> bool {
    if path.starts_with(root) {
        return true;
    }
    let canon = |p: &Path| -> Option<PathBuf> {
        let mut base = p.to_path_buf();
        let mut rest = Vec::new();
        while !base.exists() {
            rest.push(base.file_name()?.to_owned());
            base = base.parent()?.to_path_buf();
        }
        let mut out = std::fs::canonicalize(&base).ok()?;
        out.extend(rest.iter().rev());
        Some(out)
    };
    matches!((canon(path), canon(root)), (Some(p), Some(r)) if p.starts_with(&r))
}

/// The directory the file goes in by default (see the module docs).
pub fn default_dir(os: Os, home: &Path, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let abs = |k: &str| env(k).map(PathBuf::from).filter(|p| p.is_absolute());
    Some(match os {
        Os::Macos => {
            let legacy = home.join(".kassiber");
            if legacy_holds_state(&legacy) {
                legacy
            } else {
                home.join("Library")
                    .join("Application Support")
                    .join(APP_ID)
            }
        }
        Os::Windows => abs("APPDATA")
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join(APP_ID),
        Os::Linux => abs("XDG_CONFIG_HOME")
            .unwrap_or_else(|| home.join(".config"))
            .join(APP_ID),
    })
}

/// `kassiber_paths::movable_state`: a legacy root holds state when it has any top-level entry
/// besides `bin`, `run` and the migration marker (an unreadable one counts, failing safe).
fn legacy_holds_state(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(entries) => entries.flatten().any(|e| {
            let n = e.file_name();
            n != "bin" && n != "run" && n != ".state-root-migration.json"
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => path.exists() || path.is_symlink(),
    }
}

/// The file's JSON, or None when it is missing, too large, unreadable or not JSON.
pub fn read(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Atomically replaces the file with `doc` (owner-only folder and file).
pub fn write(path: &Path, doc: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("no parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let mut encoded = serde_json::to_vec_pretty(doc).map_err(|e| e.to_string())?;
    encoded.push(b'\n');
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(FILE_NAME);
    let tmp = parent.join(format!(".{name}.{}.{nonce}.tmp", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
    let written = f.write_all(&encoded).and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kb-persist-{tag}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_dirs_follow_tauri_app_config_dir() {
        let home = temp_dir("home");
        let none = |_: &str| None;
        assert_eq!(
            default_dir(Os::Linux, &home, none).unwrap(),
            home.join(".config/at.bitcoinaustria.kassiber")
        );
        // Absolute on every host (`/xdg` is not absolute on Windows).
        let xdg_dir = home.join("xdg");
        let xdg = |k: &str| (k == "XDG_CONFIG_HOME").then(|| xdg_dir.clone().into_os_string());
        assert_eq!(
            default_dir(Os::Linux, &home, xdg).unwrap(),
            xdg_dir.join("at.bitcoinaustria.kassiber")
        );
        let rel = |k: &str| (k == "XDG_CONFIG_HOME").then(|| OsString::from("relative"));
        assert_eq!(
            default_dir(Os::Linux, &home, rel).unwrap(),
            home.join(".config/at.bitcoinaustria.kassiber")
        );
        let appdata_dir = home.join("appdata");
        let appdata = |k: &str| (k == "APPDATA").then(|| appdata_dir.clone().into_os_string());
        assert_eq!(
            default_dir(Os::Windows, &home, appdata).unwrap(),
            appdata_dir.join("at.bitcoinaustria.kassiber")
        );
        assert_eq!(
            default_dir(Os::Windows, &home, none).unwrap(),
            home.join("AppData/Roaming/at.bitcoinaustria.kassiber")
        );
        let native = home.join("Library/Application Support/at.bitcoinaustria.kassiber");
        assert_eq!(default_dir(Os::Macos, &home, none).unwrap(), native);
        // A legacy root with only `bin`/`run` is not state; with anything else the file waits there.
        std::fs::create_dir_all(home.join(".kassiber/bin")).unwrap();
        assert_eq!(default_dir(Os::Macos, &home, none).unwrap(), native);
        std::fs::create_dir_all(home.join(".kassiber/data")).unwrap();
        assert_eq!(
            default_dir(Os::Macos, &home, none).unwrap(),
            home.join(".kassiber")
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn tests_never_resolve_the_real_file() {
        // `location` without the env var is None under cfg(test) (tooling_run), so no test can
        // reach the account's real preferences even by accident.
        if std::env::var_os(ENV).is_none() {
            assert_eq!(location(None), None);
        }
    }

    #[test]
    fn a_path_inside_the_book_is_refused() {
        let dir = temp_dir("book");
        assert!(inside(&dir.join("data/desktop-ui.json"), &dir.join("data")));
        // Through a symlink to the book's folder too.
        std::fs::create_dir_all(dir.join("data")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("data"), dir.join("link")).unwrap();
            assert!(inside(&dir.join("link/desktop-ui.json"), &dir.join("data")));
        }
        assert!(!inside(&dir.join("desktop-ui.json"), &dir.join("data")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn writes_owner_only_and_atomically() {
        let dir = temp_dir("write");
        let path = dir.join("cfg/desktop-ui.json");
        write(
            &path,
            &json!({"schemaVersion": 1, "ui": {"theme": "light"}}),
        )
        .unwrap();
        write(&path, &json!({"schemaVersion": 1, "ui": {"theme": "dark"}})).unwrap();
        assert_eq!(read(&path).unwrap()["ui"]["theme"], "dark");
        // No temp files left behind.
        let names: Vec<_> = std::fs::read_dir(dir.join("cfg"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![OsString::from("desktop-ui.json")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(dir.join("cfg"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_files_read_as_none() {
        let dir = temp_dir("read");
        assert_eq!(read(&dir.join("missing.json")), None);
        std::fs::write(dir.join("bad.json"), b"{not json").unwrap();
        assert_eq!(read(&dir.join("bad.json")), None);
        std::fs::write(dir.join("big.json"), vec![b' '; (MAX_BYTES + 1) as usize]).unwrap();
        assert_eq!(read(&dir.join("big.json")), None);
        assert_eq!(read(&dir), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
