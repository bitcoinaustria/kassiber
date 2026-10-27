//! Python-compatible owner exclusion. A broker and a desktop may coexist for
//! one file identity; duplicate roles and different files at a held path cannot.
#[cfg(windows)]
use kassiber_paths::windows;
use kassiber_paths::{canonicalize_allow_missing, resolve_database_path, Error, ErrorKind, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

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
            .ok_or_else(unavailable)?
            .dir
            .join(".kassiber/run/operator-owners");
        #[cfg(windows)]
        let root = windows::local_appdata()?.join("Kassiber/run/owners");
        Ok(Self {
            root,
            harden_parent: true,
        })
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
        let parent = database.parent().ok_or_else(unavailable)?;
        let material = match fs::metadata(&database) {
            Ok(info) => {
                validate_database_metadata(&database, &info)?;
                file_identity(&database, &info)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if parent.exists() {
                    let info = fs::metadata(parent).map_err(Error::io)?;
                    require_owner(parent, &info)?;
                    if !info.is_dir() {
                        return Err(Error::new(
                            ErrorKind::UnsafeProjectDatabase,
                            "project data path is not a directory",
                        ));
                    }
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
    /// Acquire before opening a long-lived book. No stale-record unlinking is
    /// needed for exclusion; Python's optional age-based janitor may prune it.
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
            return Err(unavailable());
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
impl OwnerLease {
    /// Bind a newly created file while retaining the path/local reservations.
    /// An existing file may never use this operation to accept replacement.
    pub fn bind_created_database(&mut self, namespace: &OwnerNamespace) -> Result<()> {
        let current =
            namespace.canonical_project(self.project.database.parent().ok_or_else(unavailable)?)?;
        if current.identity == self.project.identity {
            return Ok(());
        }
        if !self.pending_creation {
            return self.verify_current(namespace);
        }
        self.add_path(&current.lock_path, &current)?;
        let record = json!({"schema_version":2,"owner":self.role.as_str(),"generation":self.generation,"identity":current.identity,"pid":std::process::id()});
        let bytes = serde_json::to_vec(&record).expect("owner record");
        for file in &mut self.handles {
            let old = read_record(file);
            if old.get("generation").and_then(Value::as_str) == Some(&self.generation)
                && old.get("owner").and_then(Value::as_str) == Some(self.role.as_str())
            {
                file.set_len(0).map_err(Error::io)?;
                file.seek(SeekFrom::Start(0)).map_err(Error::io)?;
                file.write_all(&bytes).map_err(Error::io)?;
                file.write_all(b"\n").map_err(Error::io)?;
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
    /// Copies are CLOEXEC/non-inheritable by default. A broker explicitly marks
    /// only these child copies inheritable as part of its launch protocol.
    pub fn duplicate_for_child(&self) -> Result<Vec<File>> {
        self.handles
            .iter()
            .map(|f| f.try_clone().map_err(Error::io))
            .collect()
    }
    pub fn verify_current(&self, namespace: &OwnerNamespace) -> Result<()> {
        if namespace
            .canonical_project(self.project.database.parent().ok_or_else(unavailable)?)?
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
                "project alias resolves to a different database",
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
        let _admission = admission(path)?;
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
            if let Some(record) = active_record(&suffixed(path, self.role.other().as_str()))? {
                if record.get("identity").and_then(Value::as_str) != Some(&project.identity) {
                    return Err(in_use(project, record));
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
        let mut file = match open_lock(path, shared, false) {
            Ok(f) => f,
            Err(e) if e.kind == ErrorKind::ProjectInUse => {
                let record = open_lock(path, false, true)
                    .map(|mut f| read_record(&mut f))
                    .unwrap_or(json!({}));
                return Err(in_use(project, record));
            }
            Err(e) => return Err(e),
        };
        if !try_lock(&file, shared)? {
            return Err(in_use(project, read_record(&mut file)));
        }
        if !shared {
            let record = json!({"schema_version":2,"owner":self.role.as_str(),"generation":self.generation,"identity":project.identity,"pid":std::process::id()});
            file.set_len(0).map_err(Error::io)?;
            file.seek(SeekFrom::Start(0)).map_err(Error::io)?;
            file.write_all(
                serde_json::to_string(&record)
                    .expect("JSON record")
                    .as_bytes(),
            )
            .map_err(Error::io)?;
            file.write_all(b"\n").map_err(Error::io)?;
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
fn admission(path: &Path) -> Result<File> {
    let path = suffixed(path, "admission");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match open_lock(&path, false, false) {
            Ok(file) => {
                #[cfg(unix)]
                {
                    while !try_lock(&file, false)? {
                        if Instant::now() >= deadline {
                            return Err(admission_busy());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                return Ok(file);
            }
            Err(e) if e.kind == ErrorKind::ProjectInUse && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(e),
        }
    }
}
#[cfg(unix)]
fn admission_busy() -> Error {
    Error::new(ErrorKind::ProjectInUse, "project owner admission is busy").retryable(true)
}
fn active_record(path: &Path) -> Result<Option<Value>> {
    let mut file = match open_lock(path, false, false) {
        Ok(file) => file,
        Err(e) if cfg!(windows) && e.kind == ErrorKind::ProjectInUse => {
            return open_lock(path, false, true).map(|mut f| Some(read_record(&mut f)))
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
    Ok(active_record(path)?.is_some())
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
fn in_use(project: &CanonicalProject, record: Value) -> Error {
    let owner = record
        .get("owner")
        .and_then(Value::as_str)
        .filter(|s| ["desktop", "broker"].contains(s))
        .unwrap_or("unknown");
    let generation = record.get("generation").and_then(Value::as_str);
    let pid = record.get("pid").filter(|v| v.is_i64() || v.is_u64());
    Error::new(
        ErrorKind::ProjectInUse,
        match owner {
            "desktop" => "another desktop app or preview owns this project",
            "broker" => "another CLI broker owns this project",
            _ => "another long-lived process owns this project path",
        },
    )
    .retryable(true)
    .details(json!({"project":project.public_id,"owner":owner,"generation":generation,"pid":pid}))
}
fn unavailable() -> Error {
    Error::new(
        ErrorKind::ProjectOwnerLockUnavailable,
        "project ownership lock is unavailable",
    )
    .retryable(true)
}
fn ensure_directory(path: &Path) -> Result<()> {
    if path.is_symlink() {
        return Err(Error::new(
            ErrorKind::UnsafeProjectOwnerLock,
            "project owner directory may not be a symlink",
        ));
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| unavailable())?;
    let info = fs::metadata(path).map_err(|_| unavailable())?;
    if !info.is_dir() {
        return Err(Error::new(
            ErrorKind::UnsafeProjectOwnerLock,
            "project owner path is not a directory",
        ));
    }
    require_owner(path, &info)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| unavailable())?;
    }
    Ok(())
}
fn open_lock(path: &Path, shared: bool, probe: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(!probe).create(!probe);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
        let _ = shared;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(if shared || probe { 3 } else { 1 });
    }
    let file = options.open(path).map_err(|e| {
        if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) {
            Error::new(ErrorKind::ProjectInUse, "project ownership lock is held").retryable(true)
        } else {
            unavailable()
        }
    })?;
    let info = file.metadata().map_err(|_| unavailable())?;
    if !info.is_file() {
        return Err(Error::new(
            ErrorKind::UnsafeProjectOwnerLock,
            "project ownership lock is unsafe",
        ));
    }
    require_owner(path, &info)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| unavailable())?;
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
            Err(std::fs::TryLockError::Error(_)) => Err(unavailable()),
        }
    }
    #[cfg(windows)]
    {
        let _ = (file, shared);
        Ok(true)
    }
}
pub fn validate_database_file(path: &Path) -> Result<()> {
    validate_database_metadata(path, &fs::metadata(path).map_err(Error::io)?)
}
fn validate_database_metadata(path: &Path, info: &fs::Metadata) -> Result<()> {
    if !info.is_file() {
        return Err(Error::new(
            ErrorKind::UnsafeProjectDatabase,
            "project database is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if info.nlink() != 1 {
            return Err(Error::new(
                ErrorKind::UnsafeProjectDatabase,
                "project database has multiple filesystem links",
            ));
        }
    }
    #[cfg(windows)]
    {
        windows::validate_links(path)?;
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
        "project path is owned by another OS user",
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
