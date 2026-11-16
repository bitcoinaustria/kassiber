//! OS backups and synced folders (Settings → Data).
//!
//! An encrypted book is ciphertext on disk, so Time Machine may back it up. An unencrypted book
//! is not: with "Keep unencrypted books out of Time Machine" on (the default), Kassiber marks
//! its files with `com.apple.metadata:com_apple_backup_excludeItem` (the per-item exclusion
//! `tmutil addexclusion` writes; no admin, no Time Machine settings change), one item at a time:
//!
//! - unencrypted book: the database file and its `-wal`, `-shm` and `-journal` files, the
//!   managed `attachments/` and `exports/` folders when they sit inside the state root, and the
//!   plaintext `kassiber.pre-encryption.sqlite3.bak` rollback copy;
//! - encrypted book: only that rollback copy (it is the old plaintext database).
//!
//! Only paths that exist get the mark; nothing is created. The data root itself is never marked
//! (`--data-root` may point at a folder holding other files), nor is anything outside the state
//! root, nor the user's own export destinations. Each mark Kassiber sets carries a second
//! attribute, [`MARKER`], so turning the setting off, or encrypting the book, removes only
//! Kassiber's marks and never an exclusion the user made. Marks are applied when the open book's
//! `status` arrives or changes and checked again every minute while the book is open, because
//! SQLite deletes and recreates its `-wal`/`-shm` files, which drops the mark with them.
//!
//! Time Machine marks do nothing for iCloud Drive, Dropbox, OneDrive, Google Drive or Syncthing:
//! [`synced_by`] recognizes those folders, and the Data page warns while an unencrypted book sits
//! in one. Windows (File History, OneDrive backup) and Linux backup tools have no per-file
//! exclusion attribute Kassiber can set; there the Data page says to exclude the folder in the
//! backup tool, or encrypt the book.
//!
//! Tooling runs never touch files: `poll` records the selection instead ([`recorded`]).

use std::cell::{Cell, RefCell};
use std::path::{Component, Path, PathBuf};

use rune::Tx;
use serde_json::Value;

use crate::app::App;

/// The Time Machine exclusion attribute and its value: the binary plist string
/// `com.apple.backupd`, byte for byte what `tmutil addexclusion` writes.
pub const EXCLUDE: &str = "com.apple.metadata:com_apple_backup_excludeItem";
pub const EXCLUDE_VALUE: [u8; 61] = [
    0x62, 0x70, 0x6C, 0x69, 0x73, 0x74, 0x30, 0x30, 0x5F, 0x10, 0x11, 0x63, 0x6F, 0x6D, 0x2E, 0x61,
    0x70, 0x70, 0x6C, 0x65, 0x2E, 0x62, 0x61, 0x63, 0x6B, 0x75, 0x70, 0x64, 0x08, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1C,
];
/// Kassiber's own note that it set [`EXCLUDE`] on an item.
pub const MARKER: &str = "at.bitcoinaustria.kassiber.backup-excluded";
/// The plaintext database `secrets init` keeps as a rollback (privacy-and-security.md).
pub const ROLLBACK: &str = "kassiber.pre-encryption.sqlite3.bak";
const SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];
/// How often the marks are checked again while an unencrypted book is open.
const RECHECK_SECS: f64 = 60.0;

/// The open book's paths, from `status`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book {
    pub encrypted: bool,
    pub database: PathBuf,
    pub state_root: Option<PathBuf>,
    pub data_root: Option<PathBuf>,
    pub attachments: Option<PathBuf>,
    pub exports: Option<PathBuf>,
}

impl Book {
    pub fn from_status(s: &Value) -> Option<Book> {
        let path = |k: &str| {
            s[k].as_str()
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Some(Book {
            encrypted: s["database_encrypted"].as_bool()?,
            database: path("database")?,
            state_root: path("state_root"),
            data_root: path("data_root"),
            attachments: path("attachments_root"),
            exports: path("exports_root"),
        })
    }
}

/// What to mark and what to release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub exclude: Vec<PathBuf>,
    pub release: Vec<PathBuf>,
}

/// `path` lies strictly inside `root`, with no `..` that could climb out.
fn inside(path: &Path, root: &Path) -> bool {
    path != root
        && path.starts_with(root)
        && !path.components().any(|c| matches!(c, Component::ParentDir))
        && !root.components().any(|c| matches!(c, Component::ParentDir))
}

/// Which of the book's items get the exclusion (see the module docs).
pub fn selection(book: &Book, enabled: bool) -> Selection {
    let mut db: Vec<PathBuf> = vec![book.database.clone()];
    for s in SIDECARS {
        let mut p = book.database.clone().into_os_string();
        p.push(s);
        db.push(PathBuf::from(p));
    }
    let managed: Vec<PathBuf> = match &book.state_root {
        Some(root) => [&book.attachments, &book.exports]
            .into_iter()
            .flatten()
            .filter(|p| inside(p, root))
            .cloned()
            .collect(),
        None => vec![],
    };
    let rollback = book
        .data_root
        .clone()
        .or_else(|| book.database.parent().map(Path::to_path_buf))
        .map(|d| d.join(ROLLBACK));
    let mut out = Selection::default();
    let plaintext: Vec<PathBuf> = db.into_iter().chain(managed).collect();
    if !enabled {
        out.release = plaintext.into_iter().chain(rollback).collect();
    } else if book.encrypted {
        out.exclude = rollback.into_iter().collect();
        out.release = plaintext;
    } else {
        out.exclude = plaintext.into_iter().chain(rollback).collect();
    }
    out
}

/// What applying a selection did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Items now carrying Kassiber's exclusion (or one the user set).
    pub excluded: usize,
    /// Items that should have it and could not be marked.
    pub failed: usize,
    pub released: usize,
}

/// Marks and releases the selection's existing items. Never call it on real paths from a
/// tooling run (`poll` guards that; tests use temporary folders).
pub fn apply(sel: &Selection) -> Outcome {
    let mut out = Outcome::default();
    for p in &sel.exclude {
        if std::fs::symlink_metadata(p).is_err() {
            continue;
        }
        if xattr::has(p, EXCLUDE)
            || (xattr::set(p, EXCLUDE, &EXCLUDE_VALUE) && xattr::set(p, MARKER, b"1"))
        {
            out.excluded += 1;
        } else {
            out.failed += 1;
        }
    }
    for p in &sel.release {
        if std::fs::symlink_metadata(p).is_ok() && xattr::has(p, MARKER) {
            xattr::remove(p, EXCLUDE);
            xattr::remove(p, MARKER);
            out.released += 1;
        }
    }
    out
}

/// Extended attributes on the item itself (`XATTR_NOFOLLOW`: a symlink's target is never
/// marked).
#[cfg(target_os = "macos")]
mod xattr {
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    const NOFOLLOW: c_int = 0x0001;

    unsafe extern "C" {
        fn getxattr(
            path: *const c_char,
            name: *const c_char,
            value: *mut c_void,
            size: usize,
            position: u32,
            options: c_int,
        ) -> isize;
        fn setxattr(
            path: *const c_char,
            name: *const c_char,
            value: *const c_void,
            size: usize,
            position: u32,
            options: c_int,
        ) -> c_int;
        fn removexattr(path: *const c_char, name: *const c_char, options: c_int) -> c_int;
    }

    fn c(path: &Path, name: &str) -> Option<(CString, CString)> {
        Some((
            CString::new(path.as_os_str().as_bytes()).ok()?,
            CString::new(name).ok()?,
        ))
    }

    pub fn has(path: &Path, name: &str) -> bool {
        c(path, name).is_some_and(|(p, n)| unsafe { getxattr(p.as_ptr(), n.as_ptr(), std::ptr::null_mut(), 0, 0, NOFOLLOW) } >= 0)
    }

    pub fn set(path: &Path, name: &str, value: &[u8]) -> bool {
        c(path, name).is_some_and(|(p, n)| unsafe { setxattr(p.as_ptr(), n.as_ptr(), value.as_ptr().cast(), value.len(), 0, NOFOLLOW) } == 0)
    }

    pub fn remove(path: &Path, name: &str) {
        if let Some((p, n)) = c(path, name) {
            unsafe { removexattr(p.as_ptr(), n.as_ptr(), NOFOLLOW) };
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod xattr {
    use std::path::Path;
    pub fn has(_: &Path, _: &str) -> bool {
        false
    }
    pub fn set(_: &Path, _: &str, _: &[u8]) -> bool {
        false
    }
    pub fn remove(_: &Path, _: &str) {}
}

// ---- Synced folders -----------------------------------------------------------------------------

/// The sync service whose folder holds `path`, if any: a product name, or "" for an
/// unrecognized cloud-storage provider. `exists` answers for marker files (`.stfolder`,
/// `.dropbox`) and iCloud's Desktop & Documents folders; `home` is the user's home folder.
pub fn synced_by(
    path: &Path,
    home: Option<&Path>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<&'static str> {
    let names: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    for (i, n) in names.iter().enumerate() {
        let after_library = i > 0 && names[i - 1] == "Library";
        if after_library && n == "Mobile Documents" {
            return Some("iCloud Drive");
        }
        if after_library && n == "CloudStorage" {
            let provider = names.get(i + 1).map(String::as_str).unwrap_or("");
            return Some(provider_name(provider));
        }
    }
    for n in &names {
        if n == "Dropbox" || n.starts_with("Dropbox (") {
            return Some("Dropbox");
        }
        if n.starts_with("OneDrive") {
            return Some("OneDrive");
        }
        if n == "Google Drive" || n == "My Drive" {
            return Some("Google Drive");
        }
    }
    // iCloud's "Desktop & Documents Folders": the user's Desktop and Documents are iCloud Drive
    // folders when iCloud Drive holds a Desktop / Documents folder of its own.
    if let Some(home) = home {
        let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
        for d in ["Desktop", "Documents"] {
            if path.starts_with(home.join(d)) && exists(&icloud.join(d)) {
                return Some("iCloud Drive");
            }
        }
    }
    for a in path.ancestors().skip(1) {
        if exists(&a.join(".stfolder")) {
            return Some("Syncthing");
        }
        if exists(&a.join(".dropbox")) {
            return Some("Dropbox");
        }
    }
    None
}

/// `~/Library/CloudStorage/<Provider>-<account>` folder names.
fn provider_name(dir: &str) -> &'static str {
    for (prefix, name) in [
        ("Dropbox", "Dropbox"),
        ("OneDrive", "OneDrive"),
        ("GoogleDrive", "Google Drive"),
        ("Box", "Box"),
        ("iCloud", "iCloud Drive"),
    ] {
        if dir.starts_with(prefix) {
            return name;
        }
    }
    ""
}

// ---- The open book ------------------------------------------------------------------------------

/// What the Data page shows for the open book.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub book: Option<Book>,
    pub outcome: Option<Outcome>,
    pub synced: Option<&'static str>,
}

#[derive(Default)]
struct State {
    /// The status and setting the marks were last applied for.
    key: Option<(Book, bool)>,
    checked_at: f64,
    view: View,
    recorded: Vec<Selection>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// Scenes: a simulated sync service for the open book.
    static SIMULATED_SYNC: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// The Data page's view of the open book.
pub fn view() -> View {
    STATE.with(|s| s.borrow().view.clone())
}

/// The selections a tooling run would have applied.
#[allow(dead_code)]
pub fn recorded() -> Vec<Selection> {
    STATE.with(|s| s.borrow().recorded.clone())
}

/// Scenes: pretend the open book sits in `service`'s folder.
pub fn simulate_sync(service: &'static str) {
    SIMULATED_SYNC.with(|s| s.set(Some(service)));
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

fn sync_service(book: &Book) -> Option<&'static str> {
    if let Some(s) = SIMULATED_SYNC.with(Cell::get) {
        return Some(s);
    }
    let exists = |p: &Path| std::fs::symlink_metadata(p).is_ok();
    let home = home();
    let raw = synced_by(&book.database, home.as_deref(), &exists);
    raw.or_else(|| {
        std::fs::canonicalize(&book.database)
            .ok()
            .and_then(|real| synced_by(&real, home.as_deref(), &exists))
    })
}

/// Every frame: (re)applies the marks when the open book or the setting changes, and every
/// minute while there is something to mark. True when the Data page's view changed.
pub fn poll(app: &App, tx: &mut Tx) -> bool {
    let book = app.read("status").and_then(Book::from_status);
    let enabled = app.settings.prefs.exclude_unencrypted_backups;
    let tooling = crate::persist::file::tooling_run();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(book) = book else {
            let changed = s.view.book.is_some();
            s.key = None;
            s.view = View::default();
            return changed;
        };
        let key = Some((book.clone(), enabled));
        let due = app.clock - s.checked_at >= RECHECK_SECS && !book.encrypted && enabled;
        if s.key == key && !due {
            return false;
        }
        let fresh = s.key != key;
        s.key = key;
        s.checked_at = app.clock;
        let sel = selection(&book, enabled);
        let outcome = if tooling {
            s.recorded.push(sel);
            None
        } else if cfg!(target_os = "macos") {
            Some(apply(&sel))
        } else {
            None
        };
        if !tooling && enabled && !book.encrypted {
            tx.wake_after(std::time::Duration::from_secs_f64(RECHECK_SECS));
        }
        let synced = if fresh {
            sync_service(&book)
        } else {
            s.view.synced
        };
        let next = View {
            book: Some(book),
            outcome,
            synced,
        };
        let changed = next != s.view;
        s.view = next;
        changed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(encrypted: bool) -> Book {
        Book {
            encrypted,
            database: "/s/projects/main/data/kassiber.sqlite3".into(),
            state_root: Some("/s/projects/main".into()),
            data_root: Some("/s/projects/main/data".into()),
            attachments: Some("/s/projects/main/attachments".into()),
            exports: Some("/elsewhere/exports".into()),
        }
    }

    fn paths(v: &[PathBuf]) -> Vec<&str> {
        v.iter().map(|p| p.to_str().unwrap()).collect()
    }

    #[test]
    fn an_unencrypted_book_excludes_its_database_sidecars_and_managed_folders() {
        let sel = selection(&book(false), true);
        assert_eq!(
            paths(&sel.exclude),
            [
                "/s/projects/main/data/kassiber.sqlite3",
                "/s/projects/main/data/kassiber.sqlite3-wal",
                "/s/projects/main/data/kassiber.sqlite3-shm",
                "/s/projects/main/data/kassiber.sqlite3-journal",
                "/s/projects/main/attachments",
                "/s/projects/main/data/kassiber.pre-encryption.sqlite3.bak",
            ]
        );
        // Never the data root itself, never exports outside the state root.
        assert!(
            !sel.exclude
                .iter()
                .any(|p| p.ends_with("data") || p.starts_with("/elsewhere"))
        );
        assert!(sel.release.is_empty());
    }

    #[test]
    fn an_encrypted_book_is_backed_up_but_its_plaintext_rollback_is_not() {
        let sel = selection(&book(true), true);
        assert_eq!(
            paths(&sel.exclude),
            ["/s/projects/main/data/kassiber.pre-encryption.sqlite3.bak"]
        );
        assert!(
            sel.release
                .iter()
                .any(|p| p.ends_with("kassiber.sqlite3-wal"))
        );
        let off = selection(&book(false), false);
        assert!(off.exclude.is_empty());
        assert_eq!(off.release.len(), 6);
    }

    #[test]
    fn managed_folders_must_be_strictly_inside_the_state_root() {
        let root = Path::new("/s/p");
        assert!(inside(Path::new("/s/p/exports"), root));
        assert!(!inside(Path::new("/s/p"), root));
        assert!(!inside(Path::new("/s/p/../x"), root));
        assert!(!inside(Path::new("/s/px/exports"), root));
        let mut b = book(false);
        b.state_root = None;
        assert!(
            !selection(&b, true)
                .exclude
                .iter()
                .any(|p| p.ends_with("attachments"))
        );
    }

    #[test]
    fn status_without_paths_or_encryption_flag_is_ignored() {
        assert_eq!(
            Book::from_status(&serde_json::json!({"database": "/d/k.sqlite3"})),
            None
        );
        assert_eq!(
            Book::from_status(
                &serde_json::json!({"database_encrypted": false, "database": "relative.sqlite3"})
            ),
            None
        );
        let b = Book::from_status(&serde_json::json!({"database_encrypted": false, "database": "/d/k.sqlite3", "exports_root": ""})).unwrap();
        assert_eq!(b.exports, None);
    }

    #[test]
    fn synced_folders_are_recognized() {
        let none = |_: &Path| false;
        let home = Path::new("/Users/u");
        let s = |p: &str| synced_by(Path::new(p), Some(home), &none);
        assert_eq!(
            s("/Users/u/Library/Mobile Documents/com~apple~CloudDocs/k/data/kassiber.sqlite3"),
            Some("iCloud Drive")
        );
        assert_eq!(
            s("/Users/u/Library/CloudStorage/OneDrive-Personal/k/kassiber.sqlite3"),
            Some("OneDrive")
        );
        assert_eq!(
            s("/Users/u/Library/CloudStorage/GoogleDrive-u@example.org/My Drive/k.sqlite3"),
            Some("Google Drive")
        );
        assert_eq!(
            s("/Users/u/Library/CloudStorage/SomethingElse/k.sqlite3"),
            Some("")
        );
        assert_eq!(s("/Users/u/Dropbox/k/kassiber.sqlite3"), Some("Dropbox"));
        assert_eq!(
            s("/Users/u/Library/Application Support/Kassiber/projects/main/data/kassiber.sqlite3"),
            None
        );
        assert_eq!(s("/Users/u/Documents/k/kassiber.sqlite3"), None);
        let icloud_docs = |p: &Path| {
            p == Path::new("/Users/u/Library/Mobile Documents/com~apple~CloudDocs/Documents")
        };
        assert_eq!(
            synced_by(
                Path::new("/Users/u/Documents/k/kassiber.sqlite3"),
                Some(home),
                &icloud_docs
            ),
            Some("iCloud Drive")
        );
        let syncthing = |p: &Path| p == Path::new("/Users/u/Sync/.stfolder");
        assert_eq!(
            synced_by(
                Path::new("/Users/u/Sync/k/kassiber.sqlite3"),
                Some(home),
                &syncthing
            ),
            Some("Syncthing")
        );
    }

    /// Real attributes, on a temporary folder only.
    #[cfg(target_os = "macos")]
    #[test]
    fn marks_are_set_on_existing_items_and_only_kassibers_are_released() {
        let dir =
            std::env::temp_dir().join(format!("kassiber-osleaks-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(dir.join("attachments")).unwrap();
        let db = data.join("kassiber.sqlite3");
        std::fs::write(&db, b"plain").unwrap();
        std::fs::write(data.join("kassiber.sqlite3-wal"), b"wal").unwrap();
        let mine = data.join("kassiber.sqlite3-shm");
        std::fs::write(&mine, b"shm").unwrap();
        // The user excluded the -shm file themselves: Kassiber leaves that mark alone.
        assert!(xattr::set(&mine, EXCLUDE, &EXCLUDE_VALUE));
        let b = Book {
            encrypted: false,
            database: db.clone(),
            state_root: Some(dir.clone()),
            data_root: Some(data.clone()),
            attachments: Some(dir.join("attachments")),
            exports: Some(dir.join("exports")),
        };
        let out = apply(&selection(&b, true));
        assert_eq!(
            out,
            Outcome {
                excluded: 4,
                failed: 0,
                released: 0
            }
        );
        assert!(xattr::has(&db, EXCLUDE) && xattr::has(&db, MARKER));
        assert!(xattr::has(&dir.join("attachments"), EXCLUDE));
        assert!(!xattr::has(&mine, MARKER));
        assert!(
            !data.join("kassiber.sqlite3-journal").exists(),
            "nothing is created"
        );
        assert!(
            !xattr::has(&data, EXCLUDE),
            "the data root itself is never marked"
        );
        // tmutil agrees the database is excluded.
        let tm = std::process::Command::new("/usr/bin/tmutil")
            .arg("isexcluded")
            .arg(&db)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&tm.stdout).contains("[Excluded]"));
        // Encrypting the book (or turning the setting off) releases Kassiber's marks only.
        let out = apply(&selection(
            &Book {
                encrypted: true,
                ..b.clone()
            },
            true,
        ));
        assert_eq!(out.released, 3);
        assert!(!xattr::has(&db, EXCLUDE) && !xattr::has(&db, MARKER));
        assert!(xattr::has(&mine, EXCLUDE), "the user's own exclusion stays");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
