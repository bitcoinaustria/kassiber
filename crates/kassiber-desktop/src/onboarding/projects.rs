//! Opening an existing book folder, as the Tauri shell does it (`lib.rs`
//! `select_import_project_directory` / `activate_import_project` / `clear_import_project`):
//!
//! - the folder comes only from the native folder picker; it is inspected (no symlinks, a
//!   Kassiber database directly in it or under `data/`, plaintext databases must carry
//!   Kassiber's tables), and its data root is recorded in the approved-projects store;
//! - only an approved data root is ever opened again (a launch restoring the last book);
//! - opening restarts the daemon against it (`crate::app::switch_root`), closing returns the
//!   daemon to the launch's own root.
//!
//! The approved-projects store is Tauri's `approved-import-projects.json`
//! (`{"data_roots": [...]}`, sorted) in the app config directory, beside the preferences file:
//! owner-only, written atomically, never logged. It is the only place a book path is kept.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub const APPROVED_FILE_NAME: &str = "approved-import-projects.json";
/// Overrides the store's location (tests, scripts).
pub const APPROVED_ENV: &str = "KASSIBER_APPROVED_IMPORT_PROJECTS_FILE";
const DEFAULT_STATE_DIR: &str = ".kassiber";
const DEFAULT_DATA_DIR: &str = "data";
const DB_FILENAMES: [&str; 2] = ["kassiber.sqlite3", "satbooks.sqlite3"];
const MAX_STORE_BYTES: u64 = 1024 * 1024;

/// `ImportProjectSelection`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub state_root: String,
    pub data_root: String,
    pub database: String,
    pub encrypted: bool,
}

/// Where the approved-projects store lives: the env override, else beside the preferences
/// file (its env override's folder, or the app config directory). Tooling runs without an
/// override keep approvals in memory only (`None`), so a scripted run never writes the
/// account's real config.
pub fn store_location() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(p) = env(APPROVED_ENV) {
        return p.is_absolute().then_some(p);
    }
    if let Some(prefs) = env(crate::persist::file::ENV) {
        return prefs
            .is_absolute()
            .then(|| prefs.parent().map(|d| d.join(APPROVED_FILE_NAME)))
            .flatten();
    }
    if crate::persist::file::tooling_run() {
        return None;
    }
    let home = crate::daemon::home_dir()?;
    crate::persist::file::default_dir(crate::persist::file::Os::current(), &home, |k| {
        std::env::var_os(k).filter(|v| !v.is_empty())
    })
    .map(|d| d.join(APPROVED_FILE_NAME))
}

/// The approved data roots (in memory for tooling runs, the file otherwise).
#[derive(Default)]
pub struct Approved {
    pub path: Option<PathBuf>,
    /// The roots approved this run when there is no file.
    pub memory: Vec<String>,
}

impl Approved {
    pub fn load() -> Approved {
        Approved {
            path: store_location(),
            memory: vec![],
        }
    }

    pub fn roots(&self) -> Result<Vec<String>, String> {
        match &self.path {
            Some(p) => read_store(p),
            None => Ok(self.memory.clone()),
        }
    }

    /// `approve_import_project_data_root`.
    pub fn approve(&mut self, data_root: &str) -> Result<(), String> {
        match &self.path {
            Some(p) => approve_at(p, data_root),
            None => {
                if !self.memory.iter().any(|r| r == data_root) {
                    self.memory.push(data_root.to_owned());
                    self.memory.sort();
                }
                Ok(())
            }
        }
    }

    /// `require_approved_import_project_data_root`.
    pub fn require(&self, requested: &str) -> Result<String, String> {
        self.roots()?
            .into_iter()
            .find(|r| r == requested)
            .ok_or_else(|| {
                "Choose this Kassiber project with the native folder picker before opening it."
                    .to_owned()
            })
    }

    /// The approved data root whose book key is `key` (the identity keeps only the key).
    pub fn by_key(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self
            .roots()?
            .into_iter()
            .find(|r| crate::persist::model::book_key(Some(Path::new(r))) == key))
    }
}

/// `read_approved_import_projects`: a missing file is an empty store; a bad one is an error.
pub fn read_store(path: &Path) -> Result<Vec<String>, String> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("Approved project store could not be read: {e}")),
    };
    if meta.len() > MAX_STORE_BYTES {
        return Err("Approved project store is invalid: too large".into());
    }
    let raw = std::fs::read(path)
        .map_err(|e| format!("Approved project store could not be read: {e}"))?;
    let v: Value = serde_json::from_slice(&raw)
        .map_err(|e| format!("Approved project store is invalid: {e}"))?;
    let roots = v
        .get("data_roots")
        .and_then(Value::as_array)
        .ok_or("Approved project store is invalid: missing field `data_roots`")?;
    roots
        .iter()
        .map(|r| {
            r.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "Approved project store is invalid: invalid type".to_owned())
        })
        .collect()
}

/// `approve_import_project_data_root`: add, sort, write (owner-only, atomic).
pub fn approve_at(path: &Path, data_root: &str) -> Result<(), String> {
    let mut roots = read_store(path)?;
    if roots.iter().any(|r| r == data_root) {
        return Ok(());
    }
    roots.push(data_root.to_owned());
    roots.sort();
    if path.parent().is_none() {
        return Err("Approved project store has no parent folder.".into());
    }
    crate::persist::file::write(path, &json!({"data_roots": roots}))
        .map_err(|e| format!("Approved project store could not be written: {e}"))
}

/// `ExpandUser`.
fn expanduser(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if (raw == "~" || raw.starts_with("~/"))
        && let Some(home) = crate::daemon::home_dir()
    {
        return if raw == "~" {
            home
        } else {
            home.join(&raw[2..])
        };
    }
    path.to_path_buf()
}

/// `inspect_import_project_directory`.
pub fn inspect(path: &Path) -> Result<Selection, String> {
    let canonical = expanduser(path)
        .canonicalize()
        .map_err(|e| format!("Kassiber project folder could not be opened: {e}"))?;
    let (data_root, database, encrypted) = resolve_data_root(&canonical)?.ok_or_else(|| "Choose a Kassiber project folder containing data/kassiber.sqlite3, or choose the data folder itself.".to_owned())?;
    let state_root = if data_root.file_name().and_then(|n| n.to_str()) == Some(DEFAULT_DATA_DIR) {
        data_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| data_root.clone())
    } else {
        data_root.clone()
    };
    Ok(Selection {
        state_root: lossy(&state_root),
        data_root: lossy(&data_root),
        database: lossy(&database),
        encrypted,
    })
}

fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn resolve_data_root(path: &Path) -> Result<Option<(PathBuf, PathBuf, bool)>, String> {
    let mut direct = None;
    for name in DB_FILENAMES {
        let database = path.join(name);
        if let Some(enc) = inspect_database(&database)? {
            direct = Some((path.to_path_buf(), database, enc));
            break;
        }
    }
    let mut nested = None;
    if let Some(data_root) = inspect_data_root(&path.join(DEFAULT_DATA_DIR))? {
        for name in DB_FILENAMES {
            let database = data_root.join(name);
            if let Some(enc) = inspect_database(&database)? {
                nested = Some((data_root.clone(), database, enc));
                break;
            }
        }
    }
    match (direct, nested) {
        (Some(_), Some(sel)) if is_managed_state_root(path) => Ok(Some(sel)),
        (Some(_), Some(_)) => Err("Selected folder contains Kassiber databases both directly and under data/. Choose the exact data folder to import.".into()),
        (Some(sel), None) | (None, Some(sel)) => Ok(Some(sel)),
        (None, None) => Ok(None),
    }
}

fn is_managed_state_root(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some(DEFAULT_STATE_DIR)
        || path.join("config").join("settings.json").is_file()
}

fn inspect_data_root(path: &Path) -> Result<Option<PathBuf>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "Kassiber data folder candidate could not be inspected: {e}"
            ));
        }
    };
    if meta.file_type().is_symlink() {
        return Err("Kassiber data folders must not be symlinks.".into());
    }
    if !meta.file_type().is_dir() {
        return Ok(None);
    }
    path.canonicalize()
        .map(Some)
        .map_err(|e| format!("Kassiber data folder could not be opened: {e}"))
}

fn inspect_database(path: &Path) -> Result<Option<bool>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "Kassiber database candidate could not be inspected: {e}"
            ));
        }
    };
    if meta.file_type().is_symlink() {
        return Err("Kassiber database files must not be symlinks.".into());
    }
    if !meta.file_type().is_file() {
        return Ok(None);
    }
    if meta.len() == 0 {
        return Err("Kassiber database file is empty.".into());
    }
    let encrypted =
        is_encrypted(path).map_err(|e| format!("Kassiber database could not be inspected: {e}"))?;
    if !encrypted && !looks_like_kassiber(path)? {
        return Err(
            "Selected SQLite file does not contain Kassiber workspace/profile tables.".into(),
        );
    }
    Ok(Some(encrypted))
}

fn is_encrypted(path: &Path) -> std::io::Result<bool> {
    let mut f = std::fs::File::open(path)?;
    let mut header = [0u8; 16];
    let n = f.read(&mut header)?;
    if n == 0 {
        return Ok(false);
    }
    if n < header.len() {
        return Ok(true);
    }
    Ok(&header != b"SQLite format 3\0")
}

fn looks_like_kassiber(path: &Path) -> Result<bool, String> {
    let f = std::fs::File::open(path)
        .map_err(|e| format!("Kassiber database could not be read: {e}"))?;
    let mut bytes = Vec::new();
    f.take(1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Kassiber database could not be read: {e}"))?;
    let has = |needle: &[u8]| {
        bytes
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
    };
    Ok([
        &b"create table"[..],
        b"settings",
        b"workspaces",
        b"profiles",
        b"workspace_id",
        b"fiat_currency",
    ]
    .iter()
    .all(|n| has(n)))
}

/// `default_import_picker_root`: the default state root when it exists, else home.
pub fn picker_root() -> Option<PathBuf> {
    let state = crate::daemon::default_state_root();
    match state {
        Some(s) if s.exists() => Some(s),
        other => crate::daemon::home_dir().or(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("kb-onb-{tag}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn plaintext_db(path: &Path) {
        let mut b = b"SQLite format 3\0".to_vec();
        b.extend_from_slice(b"CREATE TABLE settings; CREATE TABLE workspaces; CREATE TABLE profiles (workspace_id, fiat_currency)");
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn inspection_follows_the_tauri_rules() {
        let d = dir("inspect");
        // A project folder with data/kassiber.sqlite3.
        std::fs::create_dir_all(d.join("p/data")).unwrap();
        plaintext_db(&d.join("p/data/kassiber.sqlite3"));
        let s = inspect(&d.join("p")).unwrap();
        assert_eq!(s.data_root, lossy(&d.join("p/data")));
        assert_eq!(s.state_root, lossy(&d.join("p")));
        assert!(!s.encrypted);
        // The data folder itself.
        assert_eq!(inspect(&d.join("p/data")).unwrap().data_root, s.data_root);
        // Encrypted: any header but SQLite's.
        std::fs::create_dir_all(d.join("e")).unwrap();
        std::fs::write(d.join("e/kassiber.sqlite3"), vec![7u8; 64]).unwrap();
        assert!(inspect(&d.join("e")).unwrap().encrypted);
        // Not Kassiber.
        std::fs::create_dir_all(d.join("n")).unwrap();
        std::fs::write(
            d.join("n/kassiber.sqlite3"),
            b"SQLite format 3\0CREATE TABLE other",
        )
        .unwrap();
        assert!(
            inspect(&d.join("n"))
                .unwrap_err()
                .contains("does not contain Kassiber")
        );
        // Empty folder.
        std::fs::create_dir_all(d.join("x")).unwrap();
        assert!(
            inspect(&d.join("x"))
                .unwrap_err()
                .starts_with("Choose a Kassiber project folder")
        );
        // Ambiguous unless managed.
        std::fs::create_dir_all(d.join("a/data")).unwrap();
        plaintext_db(&d.join("a/kassiber.sqlite3"));
        plaintext_db(&d.join("a/data/kassiber.sqlite3"));
        assert!(
            inspect(&d.join("a"))
                .unwrap_err()
                .contains("both directly and under data/")
        );
        std::fs::create_dir_all(d.join("a/config")).unwrap();
        std::fs::write(d.join("a/config/settings.json"), "{}").unwrap();
        assert_eq!(
            inspect(&d.join("a")).unwrap().data_root,
            lossy(&d.join("a/data"))
        );
        // Empty database file.
        std::fs::create_dir_all(d.join("z")).unwrap();
        std::fs::write(d.join("z/kassiber.sqlite3"), b"").unwrap();
        assert_eq!(
            inspect(&d.join("z")).unwrap_err(),
            "Kassiber database file is empty."
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.join("p/data"), d.join("l")).unwrap();
            std::fs::create_dir_all(d.join("s")).unwrap();
            std::os::unix::fs::symlink(d.join("p/data"), d.join("s/data")).unwrap();
            assert_eq!(
                inspect(&d.join("s")).unwrap_err(),
                "Kassiber data folders must not be symlinks."
            );
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn approved_store_is_sorted_owner_only_and_required() {
        let d = dir("approved");
        let path = d.join("cfg").join(APPROVED_FILE_NAME);
        let mut a = Approved {
            path: Some(path.clone()),
            memory: vec![],
        };
        assert_eq!(a.roots().unwrap(), Vec::<String>::new());
        a.approve("/b/data").unwrap();
        a.approve("/a/data").unwrap();
        a.approve("/b/data").unwrap();
        assert_eq!(
            read_store(&path).unwrap(),
            vec!["/a/data".to_owned(), "/b/data".to_owned()]
        );
        assert_eq!(a.require("/a/data").unwrap(), "/a/data");
        assert!(
            a.require("/c/data")
                .unwrap_err()
                .contains("native folder picker")
        );
        let key = crate::persist::model::book_key(Some(Path::new("/b/data")));
        assert_eq!(a.by_key(&key).unwrap().as_deref(), Some("/b/data"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, b"{nope").unwrap();
        assert!(
            a.roots()
                .unwrap_err()
                .starts_with("Approved project store is invalid")
        );
        // In memory when there is no file.
        let mut m = Approved::default();
        m.approve("/x").unwrap();
        assert_eq!(m.require("/x").unwrap(), "/x");
        let _ = std::fs::remove_dir_all(d);
    }
}
