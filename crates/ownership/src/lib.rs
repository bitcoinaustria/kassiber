//! Python-compatible owner exclusion (`kassiber/operator/project.py`). A
//! broker and a desktop may coexist for one file identity; duplicate roles
//! and different files at a held path cannot. Records, errors and the
//! seven-day janitor match Python's, so either implementation can hold or
//! sweep a project the other one opened.
#[cfg(windows)]
use kassiber_paths::windows;
use kassiber_paths::{canonicalize_allow_missing, resolve_database_path, Error, ErrorKind, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// `json.dumps(record, sort_keys=True)` plus a newline: the owner record
/// exactly as Python writes it (default separators, ASCII escapes).
fn record_bytes(role: OwnerKind, generation: &str, identity: &str) -> Vec<u8> {
    use kassiber_pyjson::{dumps, Key, Options, Value as Py};
    let record = Py::Dict(vec![
        (Key::from("schema_version"), Py::from(2)),
        (Key::from("owner"), Py::Str(role.as_str().into())),
        (Key::from("generation"), Py::Str(generation.into())),
        (Key::from("identity"), Py::Str(identity.into())),
        (Key::from("pid"), Py::from(std::process::id())),
    ]);
    let mut text = dumps(&record, &Options::new().sort_keys(true)).expect("owner record");
    text.push('\n');
    text.into_bytes()
}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalProject {
    pub database: PathBuf,
    pub lock_path: PathBuf,
    pub alias_lock_path: PathBuf,
    pub local_lock_path: PathBuf,
    pub identity: String,
    pub public_id: String,
}
/// Production rendezvous ignores HOME, XDG, and broker endpoint overrides.
/// An explicit isolated namespace is only for embedding/tests; it must never be
/// selected from process environment by the product shell.
#[derive(Clone, Debug)]
pub struct OwnerNamespace {
    root: PathBuf,
    harden_parent: bool,
}
impl OwnerNamespace {
    pub fn system() -> Result<Self> {
        #[cfg(unix)]
        let root = nix::unistd::User::from_uid(nix::unistd::getuid())
            .ok()
            .flatten()
            .ok_or_else(|| unavailable(None))?
            .dir
            .join(".kassiber/run/operator-owners");
        #[cfg(windows)]
        let root = windows::local_appdata()?.join("Kassiber/run/owners");
        Ok(Self {
            root,
            harden_parent: true,
        })
    }
    /// The namespace `system()` derives from an account home, for tests that
    /// stand in a disposable home for the passwd entry.
    #[doc(hidden)]
    pub fn for_account_home(home: &Path) -> Self {
        Self {
            root: home.join(".kassiber/run/operator-owners"),
            harden_parent: true,
        }
    }
    /// The root this namespace keeps its records in.
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn isolated(root: PathBuf) -> Self {
        Self {
            root,
            harden_parent: false,
        }
    }
    pub fn canonical_project(&self, data_root: &Path) -> Result<CanonicalProject> {
        let data_root = kassiber_paths::Context::from_env()?.expand_user(data_root)?;
        let database = canonicalize_allow_missing(&resolve_database_path(&data_root))?;
        let database_text = database.to_str().ok_or_else(|| {
            Error::new(
                ErrorKind::UnsafeProjectDatabase,
                "project database path is not valid Unicode",
            )
        })?;
        let parent = database.parent().ok_or_else(|| unavailable(None))?;
        let material = match fs::metadata(&database) {
            Ok(info) => {
                validate_database_metadata(&database, &info)?;
                file_identity(&database, &info)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if parent.exists() {
                    let info = fs::metadata(parent).map_err(Error::io)?;
                    if !info.is_dir() {
                        return Err(Error::new(
                            ErrorKind::UnsafeProjectDatabase,
                            "the project data directory is not a directory",
                        ));
                    }
                    require_owner(parent, &info)?;
                }
                format!("path:{}", database.display())
            }
            Err(e) => return Err(Error::io(e)),
        };
        let identity = digest(&format!(
            "kassiber-operator-v1:{}:{}:{material}",
            kassiber_paths::Platform::current().python_name(),
            principal()?
        ));
        let root = canonicalize_allow_missing(&self.root)?;
        Ok(CanonicalProject {
            lock_path: root.join(format!("identity-{identity}.lock")),
            alias_lock_path: root.join(format!("path-{}.lock", digest(database_text))),
            local_lock_path: parent.join(".operator-owner.lock"),
            public_id: identity[..16].into(),
            identity,
            database,
        })
    }
    /// Acquire before opening a long-lived book, then sweep dead records
    /// (`acquire_project_ownership`); a failed sweep never denies the owner.
    pub fn acquire(
        &self,
        project: CanonicalProject,
        role: OwnerKind,
        generation: &str,
    ) -> Result<OwnerLease> {
        if generation.len() > 1024 {
            return Err(Error::new(
                ErrorKind::Validation,
                "owner generation is too long",
            ));
        }
        if let Some(parent) = self.root.parent().filter(|_| self.harden_parent) {
            ensure_directory(parent)?;
        }
        if !self.harden_parent && !self.root.parent().is_some_and(Path::is_dir) {
            return Err(unavailable(None));
        }
        ensure_directory(&self.root)?;
        let mut lease = OwnerLease {
            pending_creation: !project.database.exists(),
            project,
            role,
            generation: generation.into(),
            handles: Vec::new(),
            paths: HashSet::new(),
        };
        let p = lease.project.clone();
        for path in [&p.lock_path, &p.alias_lock_path, &p.local_lock_path] {
            lease.add_path(path, &p)?;
        }
        // Janitorial only: a sweep that fails must never deny a valid owner.
        let _ = prune_owner_locks(p.lock_path.parent().unwrap_or(&self.root), now());
        Ok(lease)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerKind {
    Broker,
    Desktop,
}
impl OwnerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Broker => "broker",
            Self::Desktop => "desktop",
        }
    }
    fn other(self) -> Self {
        match self {
            Self::Broker => Self::Desktop,
            Self::Desktop => Self::Broker,
        }
    }
}
/// Drop closes handles, without explicitly unlocking. Inherited duplicates
/// continue to hold Unix flock / Windows share-mode reservations.
pub struct OwnerLease {
    pending_creation: bool,
    project: CanonicalProject,
    role: OwnerKind,
    generation: String,
    handles: Vec<File>,
    paths: HashSet<PathBuf>,
}
/// Parent-side duplicates of every held lock for one child
/// (`ProjectOwnerChildHandles`). They are close-on-exec and non-inheritable
/// here; `kassiber_ipc::process::ChildCommand::inherit` passes exactly these
/// numbers to the child. Closing them after the launch leaves the child's
/// copies holding the locks.
pub struct ChildHandles {
    files: Vec<File>,
}
impl ChildHandles {
    /// The numbers the child inherits (`tokens`): descriptors on POSIX,
    /// handle values on Windows.
    #[cfg(unix)]
    pub fn tokens(&self) -> Vec<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;
        self.files.iter().map(AsRawFd::as_raw_fd).collect()
    }
    /// The numbers the child inherits (`tokens`): descriptors on POSIX,
    /// handle values on Windows.
    #[cfg(windows)]
    pub fn tokens(&self) -> Vec<std::os::windows::io::RawHandle> {
        use std::os::windows::io::AsRawHandle;
        self.files.iter().map(AsRawHandle::as_raw_handle).collect()
    }
    /// Close every parent-side duplicate.
    pub fn close(self) {
        drop(self.files);
    }
}
impl OwnerLease {
    /// Bind a newly created file while retaining the path/local reservations.
    /// An existing file may never use this operation to accept replacement.
    pub fn bind_created_database(&mut self, namespace: &OwnerNamespace) -> Result<()> {
        let current = namespace.canonical_project(
            self.project
                .database
                .parent()
                .ok_or_else(|| unavailable(None))?,
        )?;
        if current.identity == self.project.identity {
            return Ok(());
        }
        if !self.pending_creation {
            return self.verify_current(namespace);
        }
        self.add_path(&current.lock_path, &current)?;
        let bytes = record_bytes(self.role, &self.generation, &current.identity);
        for file in &mut self.handles {
            let old = read_record(file);
            if old.get("generation").and_then(Value::as_str) == Some(&self.generation)
                && old.get("owner").and_then(Value::as_str) == Some(self.role.as_str())
            {
                file.set_len(0).map_err(Error::io)?;
                file.seek(SeekFrom::Start(0)).map_err(Error::io)?;
                file.write_all(&bytes).map_err(Error::io)?;
            }
        }
        self.project = current;
        self.pending_creation = false;
        Ok(())
    }
    pub fn project(&self) -> &CanonicalProject {
        &self.project
    }
    pub fn role(&self) -> OwnerKind {
        self.role
    }
    pub fn release(self) {
        drop(self);
    }
    /// `duplicate_for_child`: one duplicate of every held lock for a child.
    pub fn duplicate_for_child(&self) -> Result<ChildHandles> {
        let files = self
            .handles
            .iter()
            .map(|f| f.try_clone().map_err(Error::io))
            .collect::<Result<_>>()?;
        Ok(ChildHandles { files })
    }
    pub fn verify_current(&self, namespace: &OwnerNamespace) -> Result<()> {
        if namespace
            .canonical_project(
                self.project
                    .database
                    .parent()
                    .ok_or_else(|| unavailable(None))?,
            )?
            .identity
            != self.project.identity
        {
            return Err(Error::new(
                ErrorKind::OperatorProjectReplaced,
                "the project database was replaced",
            ));
        }
        Ok(())
    }
    pub fn add_alias(&mut self, project: &CanonicalProject) -> Result<()> {
        if project.identity != self.project.identity {
            return Err(Error::new(
                ErrorKind::ProjectOwnerMismatch,
                "the project alias resolves to a different database",
            ));
        }
        let original = self.handles.len();
        let original_paths = self.paths.clone();
        let result = self
            .add_path(&project.alias_lock_path, project)
            .and_then(|_| self.add_path(&project.local_lock_path, project));
        if result.is_err() {
            self.handles.truncate(original);
            self.paths = original_paths;
        }
        result
    }
    fn add_path(&mut self, path: &Path, project: &CanonicalProject) -> Result<()> {
        let _admission = admission(path, &project.public_id)?;
        let original = self.handles.len();
        let original_paths = self.paths.clone();
        let result = (|| {
            if !self.paths.contains(path) {
                self.add_lock(path, true, project)?;
            }
            let role_path = suffixed(path, self.role.as_str());
            if !self.paths.contains(&role_path) {
                self.add_lock(&role_path, false, project)?;
            }
            let other = suffixed(path, self.role.other().as_str());
            if let Some(record) = active_record(&other, &project.public_id)? {
                if record.get("identity").and_then(Value::as_str) != Some(&project.identity) {
                    return Err(different_project(project, &record));
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.handles.truncate(original);
            self.paths = original_paths;
        }
        result
    }
    fn add_lock(&mut self, path: &Path, shared: bool, project: &CanonicalProject) -> Result<()> {
        // A Windows share-mode holder is refused at open, as in Python,
        // without an owner label.
        let mut file = open_lock(path, &project.public_id, shared, false)?;
        if !try_lock(&file, shared)? {
            return Err(in_use(project, read_record(&mut file)));
        }
        if !shared {
            file.set_len(0).map_err(Error::io)?;
            file.seek(SeekFrom::Start(0)).map_err(Error::io)?;
            file.write_all(&record_bytes(
                self.role,
                &self.generation,
                &project.identity,
            ))
            .map_err(Error::io)?;
        }
        self.handles.push(file);
        self.paths.insert(path.into());
        Ok(())
    }
}
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    path.with_file_name(format!(
        "{}.{suffix}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}
const ADMISSION_TIMEOUT: Duration = Duration::from_secs(5);
const ADMISSION_RETRY: Duration = Duration::from_millis(10);
/// `_acquire_admission_lock`: serializes acquisitions of one lock path for
/// up to five seconds.
fn admission(path: &Path, project_id: &str) -> Result<File> {
    let path = suffixed(path, "admission");
    let deadline = Instant::now() + ADMISSION_TIMEOUT;
    #[cfg(unix)]
    {
        let file = open_lock(&path, project_id, false, false)?;
        while !try_lock(&file, false)? {
            if Instant::now() >= deadline {
                return Err(admission_busy(project_id));
            }
            std::thread::sleep(ADMISSION_RETRY);
        }
        Ok(file)
    }
    #[cfg(windows)]
    loop {
        match open_lock(&path, project_id, false, false) {
            Ok(file) => return Ok(file),
            // Python turns every AppError at this point into "busy".
            Err(error) if error.kind != ErrorKind::ProjectInUse || Instant::now() >= deadline => {
                return Err(admission_busy(project_id))
            }
            Err(_) => std::thread::sleep(ADMISSION_RETRY),
        }
    }
}
fn admission_busy(project_id: &str) -> Error {
    Error::new(ErrorKind::ProjectInUse, "project owner admission is busy")
        .hint("Retry after the other Kassiber process finishes opening.")
        .details(json!({ "project": project_id }))
        .retryable(true)
}
/// `_active_owner_record`: the record of a live holder, or `None`.
fn active_record(path: &Path, project_id: &str) -> Result<Option<Value>> {
    let mut file = match open_lock(path, project_id, false, false) {
        Ok(file) => file,
        Err(e) if cfg!(windows) && e.kind == ErrorKind::ProjectInUse => {
            return open_lock(path, project_id, false, true).map(|mut f| Some(read_record(&mut f)))
        }
        Err(e) => return Err(e),
    };
    if try_lock(&file, false)? {
        return Ok(None);
    }
    Ok(Some(read_record(&mut file)))
}
/// Windows legacy root migration uses Python's active share-mode probe rather
/// than POSIX flock. This is an explicit maintenance read, never a status probe.
#[cfg(windows)]
pub fn legacy_owner_lock_is_active(path: &Path) -> Result<bool> {
    Ok(active_record(path, "legacy")?.is_some())
}
fn read_record(file: &mut File) -> Value {
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(0)).is_err() || file.take(4096).read_to_end(&mut bytes).is_err() {
        return json!({});
    }
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(Value::is_object)
        .unwrap_or(json!({}))
}
fn record_owner(record: &Value) -> &str {
    record
        .get("owner")
        .and_then(Value::as_str)
        .filter(|s| ["desktop", "broker"].contains(s))
        .unwrap_or("unknown")
}
/// `_project_in_use_error`: name the conflicting owner and, when its record
/// says, the holder's pid. Same-user, 0700 namespace: nothing is disclosed
/// the principal cannot already read.
fn in_use(project: &CanonicalProject, record: Value) -> Error {
    let owner = record_owner(&record);
    let generation = record.get("generation").and_then(Value::as_str);
    let pid = record
        .get("pid")
        .filter(|v| v.is_i64() || v.is_u64())
        .cloned();
    let held_by = pid
        .as_ref()
        .map_or_else(String::new, |pid| format!(" It is held by pid {pid}."));
    let (message, hint) = match owner {
        "desktop" => (
            "another desktop app or preview owns this project",
            Some(format!(
                "Reuse or close the existing desktop app or preview.{held_by} \
A CLI broker can coexist, but a second desktop cannot."
            )),
        ),
        "broker" => (
            "another CLI broker owns this project",
            Some(format!(
                "Reuse or lock the existing CLI broker, then retry.{held_by}"
            )),
        ),
        _ => (
            "another long-lived process owns this project path",
            Some(held_by.trim().to_owned()).filter(|hint| !hint.is_empty()),
        ),
    };
    let error = Error::new(ErrorKind::ProjectInUse, message)
        .retryable(true)
        .details(
            json!({"project":project.public_id,"owner":owner,"generation":generation,"pid":pid}),
        );
    match hint {
        Some(hint) => error.hint_text(hint),
        None => error,
    }
}
/// `_require_compatible_other_owner`: the other role holds a different file
/// at this path.
fn different_project(project: &CanonicalProject, record: &Value) -> Error {
    Error::new(
        ErrorKind::ProjectInUse,
        "another long-lived process owns a different project at this path",
    )
    .hint("Close the process using the replaced project path, then retry.")
    .details(json!({
        "project": project.public_id,
        "owner": record_owner(record),
        "generation": record.get("generation").and_then(Value::as_str),
    }))
    .retryable(true)
}
fn unavailable(project_id: Option<&str>) -> Error {
    let error = Error::new(
        ErrorKind::ProjectOwnerLockUnavailable,
        "the project ownership lock is unavailable",
    )
    .retryable(true);
    match project_id {
        Some(project) => error.details(json!({ "project": project })),
        None => error,
    }
}
/// `Path.mkdir(mode=0o700, parents=True, exist_ok=True)`: missing parents
/// get the default mode and only the leaf gets 0700.
fn make_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(parent) = path.parent().filter(|parent| *parent != path) else {
                return Err(error);
            };
            if let Err(error) = fs::create_dir_all(parent) {
                if !parent.is_dir() {
                    return Err(error);
                }
            }
            match builder.create(path) {
                Err(_) if path.is_dir() => Ok(()),
                other => other,
            }
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(error) => Err(error),
    }
}
/// `_ensure_owner_lock_directory`.
fn ensure_directory(path: &Path) -> Result<()> {
    if path.is_symlink() {
        return Err(Error::new(
            ErrorKind::UnsafeProjectOwnerLock,
            "the project owner lock directory may not be a symlink",
        ));
    }
    make_directory(path).map_err(|_| unavailable(None))?;
    let info = fs::metadata(path).map_err(|_| unavailable(None))?;
    if !info.is_dir() {
        return Err(Error::new(
            ErrorKind::UnsafeProjectOwnerLock,
            "the project owner lock path is not a directory",
        ));
    }
    require_owner(path, &info)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| unavailable(None))?;
    }
    Ok(())
}
/// `_open_owner_lock`. A probe (Windows only) opens read-only with full
/// sharing and, as Python's `OPEN_ALWAYS` does, creates a missing file.
fn open_lock(path: &Path, project_id: &str, shared: bool, probe: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = (shared, probe);
        options
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // std requires write access to create; access_mode then narrows a
        // probe to GENERIC_READ (0x80000000) as Python requests.
        options.read(true).write(true).create(true).truncate(false);
        if probe {
            options.access_mode(0x8000_0000);
        }
        options.share_mode(if shared || probe { 3 } else { 1 });
    }
    let file = options.open(path).map_err(|e| {
        if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) {
            Error::new(
                ErrorKind::ProjectInUse,
                "another long-lived process owns this project",
            )
            .details(json!({"project": project_id, "owner": "unknown"}))
            .retryable(true)
        } else {
            unavailable(Some(project_id))
        }
    })?;
    let info = file.metadata().map_err(|_| unavailable(Some(project_id)))?;
    if !info.is_file() {
        return Err(Error::new(
            ErrorKind::UnsafeProjectOwnerLock,
            "the project ownership lock is unsafe",
        )
        .details(json!({ "project": project_id })));
    }
    require_owner(path, &info)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| unavailable(Some(project_id)))?;
    }
    Ok(file)
}
fn try_lock(file: &File, shared: bool) -> Result<bool> {
    #[cfg(unix)]
    {
        let result = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match result {
            Ok(()) => Ok(true),
            Err(std::fs::TryLockError::WouldBlock) => Ok(false),
            Err(std::fs::TryLockError::Error(_)) => Err(unavailable(None)),
        }
    }
    #[cfg(windows)]
    {
        // The requested share mode was acquired atomically on open.
        let _ = (file, shared);
        Ok(true)
    }
}

const PRUNE_STAMP_FILENAME: &str = "prune-stamp";
const PRUNE_INTERVAL_SECONDS: f64 = 24.0 * 60.0 * 60.0;
/// Records younger than this are never swept (`_OWNER_PRUNE_MIN_AGE_SECONDS`).
pub const PRUNE_MIN_AGE_SECONDS: f64 = 7.0 * 24.0 * 60.0 * 60.0;

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

fn mtime(path: &Path) -> Option<f64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(match modified.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs_f64(),
        Err(before) => -before.duration().as_secs_f64(),
    })
}

/// `_prune_owner_locks`: at most once a day, unlink the records no live
/// process holds. Keys are per project, so throwaway data roots would
/// otherwise accumulate forever.
pub fn prune_owner_locks(lock_root: &Path, now: f64) -> Result<()> {
    let stamp = lock_root.join(PRUNE_STAMP_FILENAME);
    if mtime(&stamp).is_some_and(|modified| now - modified < PRUNE_INTERVAL_SECONDS) {
        return Ok(());
    }
    // Claim the sweep before running it so concurrent opens do not all scan.
    let claimed = open_lock(&stamp, "prune", false, false)?;
    let when = UNIX_EPOCH + Duration::from_secs_f64(now.max(0.0));
    claimed
        .set_times(fs::FileTimes::new().set_accessed(when).set_modified(when))
        .map_err(Error::io)?;
    drop(claimed);
    let mut keys = std::collections::BTreeSet::new();
    for entry in fs::read_dir(lock_root).map_err(Error::io)? {
        let name = entry.map_err(Error::io)?.file_name();
        let name = name.to_string_lossy();
        if let Some((key, _)) = name.split_once(".lock") {
            keys.insert(format!("{key}.lock"));
        }
    }
    for key in keys {
        prune_owner_lock_key(&lock_root.join(key), now);
    }
    Ok(())
}

fn prunable(path: &Path, now: f64) -> bool {
    mtime(path).is_some_and(|modified| now - modified >= PRUNE_MIN_AGE_SECONDS)
}

/// `_prune_owner_lock_key`: drop one project's dead records under its
/// admission lock, which every acquisition holds between open and lock, so
/// no acquirer can be moved onto an unlinked inode. The admission file
/// itself is never removed; it is what makes the sweep safe.
fn prune_owner_lock_key(lock_path: &Path, now: f64) {
    let records = [
        lock_path.to_path_buf(),
        suffixed(lock_path, "desktop"),
        suffixed(lock_path, "broker"),
    ];
    if !records.iter().any(|path| prunable(path, now)) {
        return;
    }
    let Ok(admission) = open_lock(&suffixed(lock_path, "admission"), "prune", false, false) else {
        return;
    };
    if !try_lock(&admission, false).unwrap_or(false) {
        return;
    }
    for path in &records {
        if !prunable(path, now) {
            continue;
        }
        // A Windows share-mode holder is refused at open time.
        let Ok(handle) = open_lock(path, "prune", false, false) else {
            continue;
        };
        let dead = try_lock(&handle, false).unwrap_or(false);
        drop(handle);
        if dead {
            let _ = fs::remove_file(path);
        }
    }
    drop(admission);
}
pub fn validate_database_file(path: &Path) -> Result<()> {
    validate_database_metadata(path, &fs::metadata(path).map_err(Error::io)?)
}
/// `validate_project_database_file`.
fn validate_database_metadata(path: &Path, info: &fs::Metadata) -> Result<()> {
    if !info.is_file() {
        return Err(Error::new(
            ErrorKind::UnsafeProjectDatabase,
            "the project database is not a regular file",
        ));
    }
    #[cfg(unix)]
    let links = {
        use std::os::unix::fs::MetadataExt;
        info.nlink()
    };
    #[cfg(windows)]
    let links = u64::from(windows::link_count(path)?);
    if links != 1 {
        return Err(Error::new(
            ErrorKind::UnsafeProjectDatabase,
            "the project database has multiple filesystem links",
        )
        .hint(
            "Remove every hard-link alias except the intended project database \
path before unlocking or changing its unlock policy.",
        )
        .details(json!({ "link_count": links })));
    }
    require_owner(path, info)
}
fn require_owner(path: &Path, info: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        if info.uid() == nix::unistd::getuid().as_raw() {
            return Ok(());
        }
    }
    #[cfg(windows)]
    {
        let _ = info;
        if windows::owned_by_current_user(path)? {
            return Ok(());
        }
    }
    Err(Error::new(
        ErrorKind::UnsafeProjectOwner,
        "the project path is owned by another OS user",
    ))
}
fn principal() -> Result<String> {
    #[cfg(unix)]
    {
        Ok(nix::unistd::getuid().to_string())
    }
    #[cfg(windows)]
    {
        windows::current_sid()
    }
}
fn file_identity(path: &Path, info: &fs::Metadata) -> Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Ok(format!("file:{}:{}", info.dev(), info.ino()))
    }
    #[cfg(windows)]
    {
        let _ = info;
        windows::file_identity(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn isolated_namespace_preserves_caller_parent_and_requires_it_to_exist() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let data = root.path().join("data");
        fs::create_dir(&data).unwrap();
        fs::write(data.join("kassiber.sqlite3"), "synthetic").unwrap();
        let namespace = OwnerNamespace::isolated(root.path().join("owners"));
        let project = namespace.canonical_project(&data).unwrap();
        let lease = namespace
            .acquire(project.clone(), OwnerKind::Desktop, "synthetic")
            .unwrap();
        assert_eq!(
            fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
            0o755
        );
        drop(lease);
        let namespace = OwnerNamespace::isolated(root.path().join("absent/owners"));
        assert!(namespace
            .acquire(project, OwnerKind::Desktop, "synthetic")
            .is_err());
        assert!(!root.path().join("absent").exists());
    }
    #[test]
    fn roles_aliases_duplicates_and_replacement() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        fs::create_dir(&data).unwrap();
        fs::write(data.join("kassiber.sqlite3"), "synthetic").unwrap();
        fs::create_dir(t.path().join("run")).unwrap();
        let ns = OwnerNamespace::isolated(t.path().join("run/owners"));
        let p = ns.canonical_project(&data).unwrap();
        let first = ns
            .acquire(p.clone(), OwnerKind::Desktop, "desktop-test")
            .unwrap();
        assert_eq!(
            ns.acquire(p.clone(), OwnerKind::Desktop, "duplicate")
                .err()
                .unwrap()
                .kind,
            ErrorKind::ProjectInUse
        );
        let broker = ns
            .acquire(p.clone(), OwnerKind::Broker, "broker-test")
            .unwrap();
        #[cfg(unix)]
        {
            let alias = t.path().join("alias");
            std::os::unix::fs::symlink(&data, &alias).unwrap();
            assert_eq!(ns.canonical_project(&alias).unwrap().identity, p.identity);
        }
        fs::rename(&p.database, data.join("old")).unwrap();
        fs::write(&p.database, "replacement").unwrap();
        assert_eq!(
            first.verify_current(&ns).unwrap_err().kind,
            ErrorKind::OperatorProjectReplaced
        );
        drop(broker);
        assert_eq!(
            ns.acquire(
                ns.canonical_project(&data).unwrap(),
                OwnerKind::Broker,
                "replacement"
            )
            .err()
            .unwrap()
            .kind,
            ErrorKind::ProjectInUse
        );
        let copies = first.duplicate_for_child().unwrap();
        drop(first);
        assert!(ns
            .acquire(p.clone(), OwnerKind::Desktop, "inherited")
            .is_err());
        drop(copies);
        assert!(ns.acquire(p, OwnerKind::Desktop, "released").is_ok());
    }
    #[test]
    fn rejected_alias_rolls_back_only_its_new_locks() {
        let t = tempfile::tempdir().unwrap();
        let first = t.path().join("first");
        let alias_root = t.path().join("alias");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&alias_root).unwrap();
        fs::write(first.join("kassiber.sqlite3"), "database").unwrap();
        // A different book whose desktop owns the alias directory's local lock.
        fs::write(alias_root.join("kassiber.sqlite3"), "other").unwrap();
        fs::create_dir(t.path().join("run")).unwrap();
        let ns = OwnerNamespace::isolated(t.path().join("run/owners"));
        let project = ns.canonical_project(&first).unwrap();
        let other = ns.canonical_project(&alias_root).unwrap();
        let _desktop = ns.acquire(other, OwnerKind::Desktop, "other").unwrap();
        let mut owner = ns
            .acquire(project.clone(), OwnerKind::Broker, "broker")
            .unwrap();
        let handles = owner.handles.len();
        let paths = owner.paths.clone();
        let alias = CanonicalProject {
            database: alias_root.join("kassiber.sqlite3"),
            alias_lock_path: ns.root().join("path-alias.lock"),
            local_lock_path: alias_root.join(".operator-owner.lock"),
            ..project
        };
        let error = owner.add_alias(&alias).unwrap_err();
        assert_eq!(error.code, "project_in_use");
        assert_eq!(
            error.message,
            "another long-lived process owns a different project at this path"
        );
        assert_eq!(
            error.hint.as_deref(),
            Some("Close the process using the replaced project path, then retry.")
        );
        assert_eq!(error.details["owner"], "desktop");
        assert_eq!(owner.handles.len(), handles);
        assert_eq!(owner.paths, paths);
    }
    #[test]
    fn owner_records_are_python_json_dumps_bytes() {
        let bytes = record_bytes(OwnerKind::Broker, "g\u{e4}", "abc");
        let expected = format!(
            "{{\"generation\": \"g\\u00e4\", \"identity\": \"abc\", \"owner\": \"broker\", \"pid\": {}, \"schema_version\": 2}}\n",
            std::process::id()
        );
        assert_eq!(String::from_utf8(bytes).unwrap(), expected);
    }
    #[test]
    fn hardlinks_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("kassiber.sqlite3");
        fs::write(&path, "x").unwrap();
        fs::hard_link(&path, t.path().join("alias")).unwrap();
        assert_eq!(
            validate_database_file(&path).unwrap_err().kind,
            ErrorKind::UnsafeProjectDatabase
        );
    }
}
