//! Read-only path resolution. No mkdir, legacy migration, catalog refresh, or
//! settings rewrite occurs here. Explicit bootstrap belongs to kassiber-store.
pub mod durable;
mod error;
#[cfg(windows)]
pub mod windows;
pub use error::{Error, ErrorKind, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const DB_FILENAME: &str = "kassiber.sqlite3";
pub const LEGACY_DB_FILENAME: &str = "satbooks.sqlite3";

/// Python's `os.path.abspath`: normalize dots lexically without following
/// symlinks. This is for explicit user file paths, not project identity.
pub fn abspath(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    #[cfg(unix)]
    if absolute.as_os_str().as_encoded_bytes().starts_with(b"//")
        && !absolute.as_os_str().as_encoded_bytes().starts_with(b"///")
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let mut bytes = vec![b'/'];
        bytes.extend_from_slice(result.as_os_str().as_bytes());
        result = PathBuf::from(OsString::from_vec(bytes));
    }
    Ok(result)
}
const APP_ID: &str = "at.bitcoinaustria.kassiber";
const MIGRATION_MARKER: &str = ".state-root-migration.json";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Linux,
    Macos,
    Windows,
}
impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
    pub fn python_name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "darwin",
            Self::Windows => "win32",
        }
    }
}

/// Inject environments in tests instead of mutating process-global variables.
#[derive(Clone, Debug)]
pub struct Context {
    pub platform: Platform,
    pub home: PathBuf,
    pub environ: BTreeMap<OsString, OsString>,
}
impl Context {
    pub fn from_env() -> Result<Self> {
        let environ: BTreeMap<_, _> = std::env::vars_os().collect();
        #[cfg(unix)]
        let home = environ
            .get(&OsString::from("HOME"))
            .map(|p| {
                if p.is_empty() {
                    PathBuf::from("/")
                } else {
                    normalize_python_path(Path::new(p))
                }
            })
            .or_else(|| {
                nix::unistd::User::from_uid(nix::unistd::getuid())
                    .ok()
                    .flatten()
                    .map(|u| u.dir)
            });
        #[cfg(windows)]
        let home = environ
            .get(&OsString::from("USERPROFILE"))
            .map(PathBuf::from)
            .or_else(|| {
                Some(PathBuf::from(format!(
                    "{}{}",
                    environ.get(&OsString::from("HOMEDRIVE"))?.to_str()?,
                    environ.get(&OsString::from("HOMEPATH"))?.to_str()?
                )))
            });
        Ok(Self {
            platform: Platform::current(),
            home: home.ok_or_else(|| {
                Error::new(
                    ErrorKind::UnsafeConfiguration,
                    "home directory is unavailable",
                )
            })?,
            environ,
        })
    }
    fn env_path(&self, name: &str) -> Option<PathBuf> {
        self.environ
            .get(&OsString::from(name))
            .filter(|x| !x.is_empty())
            .map(|p| normalize_python_path(Path::new(p)))
    }
    pub fn expand_user(&self, path: &Path) -> Result<PathBuf> {
        let text = path.to_str().ok_or_else(|| {
            Error::new(ErrorKind::UnsafeConfiguration, "path is not valid Unicode")
        })?;
        if text == "~" {
            return Ok(normalize_python_path(&self.home));
        }
        if let Some(rest) = text.strip_prefix("~/").or_else(|| {
            text.strip_prefix("~\\")
                .filter(|_| self.platform == Platform::Windows)
        }) {
            return Ok(normalize_python_path(&self.home.join(rest)));
        }
        if let Some(stripped) = text.strip_prefix('~') {
            #[cfg(windows)]
            let _ = stripped;
            #[cfg(unix)]
            {
                let (name, rest) = stripped.split_once('/').unwrap_or((stripped, ""));
                if let Some(user) = nix::unistd::User::from_name(name).ok().flatten() {
                    return Ok(normalize_python_path(&user.dir.join(rest)));
                }
            }
            return Err(Error::new(
                ErrorKind::UnsafeConfiguration,
                "cannot expand home directory",
            ));
        }
        Ok(normalize_python_path(path))
    }
    pub fn native_state_root(&self) -> PathBuf {
        let home = normalize_python_path(&self.home);
        let root = match self.platform {
            Platform::Macos => home
                .join("Library")
                .join("Application Support")
                .join(APP_ID),
            Platform::Windows => self
                .env_path("LOCALAPPDATA")
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join("AppData").join("Local"))
                .join(APP_ID),
            Platform::Linux => self
                .env_path("XDG_DATA_HOME")
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(".local").join("share"))
                .join("kassiber"),
        };
        normalize_python_path(&root)
    }
    /// Matches Python's *read-only* default_state_root (before migration).
    pub fn default_state_root(&self) -> Result<PathBuf> {
        let legacy = normalize_python_path(&self.home).join(".kassiber");
        Ok(if movable_state(&legacy)? {
            legacy
        } else {
            self.native_state_root()
        })
    }
    pub fn effective_data_root(&self, requested: &Path) -> Result<PathBuf> {
        let requested = self.expand_user(requested)?;
        if requested == self.default_state_root()?.join("data") && !requested.exists() {
            for legacy in self.legacy_flat_roots() {
                if [DB_FILENAME, LEGACY_DB_FILENAME]
                    .iter()
                    .any(|n| legacy.join(n).is_file())
                {
                    return Ok(legacy);
                }
            }
        }
        Ok(requested)
    }
    fn legacy_flat_roots(&self) -> [PathBuf; 2] {
        let home = normalize_python_path(&self.home);
        [
            home.join(".local").join("share").join("kassiber"),
            home.join(".local").join("share").join("satbooks"),
        ]
    }
    pub fn state_root_for_data(&self, data: &Path) -> Result<PathBuf> {
        let data = self.effective_data_root(data)?;
        Ok(
            if !self.legacy_flat_roots().contains(&data)
                && data.file_name().is_some_and(|x| x == "data")
            {
                normalize_python_path(data.parent().unwrap_or(Path::new(".")))
            } else {
                data
            },
        )
    }
    /// Resolve a catalog or explicit target without creating or refreshing it.
    pub fn resolve(&self, options: &ResolveOptions) -> Result<RuntimePaths> {
        self.resolve_layout(options, true, true)
    }
    /// Resolve for an explicitly requested CLI/daemon bootstrap. The caller
    /// publishes the missing layout with the store's durable layout writer.
    pub fn resolve_for_bootstrap(&self, options: &ResolveOptions) -> Result<RuntimePaths> {
        self.resolve_layout(options, false, true)
    }
    /// Locate the selected container before admitting its settings. Only the
    /// restore admission path uses this: pending replacement may have moved
    /// settings away or installed an uncommitted, malformed sidecar. Resolves
    /// paths without creating anything or trusting saved path manifests.
    pub fn resolve_for_recovery(&self, options: &ResolveOptions) -> Result<RuntimePaths> {
        self.resolve_layout(options, false, false)
    }
    fn resolve_layout(
        &self,
        options: &ResolveOptions,
        require_layout: bool,
        validate_settings: bool,
    ) -> Result<RuntimePaths> {
        if options.data_root.is_some() && options.project.is_some() {
            return Err(Error::new(
                ErrorKind::InvalidFlagCombination,
                "--project and --data-root cannot be used together",
            ).hint("Use --project for a catalog project or --data-root for an explicit project data directory, not both."));
        }
        // A hidden-home root (`~/.kassiber`) the cutover left in place (it is
        // a symlink, holds one, lives on another file system or its proof
        // is invalid) stays the state root, as in Python.
        let global = self.default_state_root()?;
        // Explicit data roots bypass default-state-root migration entirely.
        // Without one, a hidden-home root the cutover left in place (or that
        // a caller such as `--env-file` never offered to it) is used where it
        // is, as Python's default_state_root does; resolution never moves it.
        if options.data_root.is_none() {
            let native = self.native_state_root();
            let legacy = self.home.join(".kassiber");
            if native.join(MIGRATION_MARKER).exists() && movable_state(&legacy)? {
                return Err(Error::migration("state_root_conflict"));
            }
        }
        let catalog_path = global.join("config/projects.json");
        let project = if let Some(data) = &options.data_root {
            // The explicit root is the selection; the default root's catalog
            // only names it (Python's project_metadata_for_data_root). A
            // catalog Python would read as empty, or one Rust refuses to
            // trust, names nothing: it neither blocks nor redirects the
            // explicit root, and nothing is written back to it.
            let catalog = load_catalog(&catalog_path, self).unwrap_or_default();
            let target = canonicalize_allow_missing(&self.effective_data_root(data)?)?;
            catalog
                .projects
                .iter()
                .find(|entry| {
                    canonicalize_allow_missing(&entry.path.join("data")).is_ok_and(|p| p == target)
                })
                .cloned()
        } else {
            let catalog = load_catalog(&catalog_path, self)?;
            if catalog.projects.is_empty() {
                if options.project.is_some() {
                    return Err(Error::new(
                        ErrorKind::ProjectNotFound,
                        "project is not registered",
                    ));
                }
                if std::iter::once(global.join("data"))
                    .chain(self.legacy_flat_roots())
                    .any(|p| resolve_database_path(&p).exists())
                {
                    return Err(Error::migration("legacy_project_layout"));
                }
                return Err(Error::new(
                    ErrorKind::NotInitialized,
                    "no Kassiber project is initialized",
                ));
            }
            let selected = options
                .project
                .as_ref()
                .map(|s| sanitize_project_id(s))
                .transpose()?
                .or(catalog.selected_project_id.clone());
            Some(
                catalog
                    .projects
                    .iter()
                    .find(|p| Some(&p.id) == selected.as_ref())
                    .cloned()
                    .ok_or_else(|| {
                        Error::new(ErrorKind::ProjectNotFound, "project is not registered")
                    })?,
            )
        };
        let requested = options
            .data_root
            .clone()
            .or_else(|| project.as_ref().map(|p| p.path.join("data")))
            .expect("target selected");
        let data_root = self.effective_data_root(&requested)?;
        let state_root = self.state_root_for_data(&data_root)?;
        let config_root = normalize_python_path(&state_root.join("config"));
        let settings_file = config_root.join("settings.json");
        // Validate metadata now, without using saved path manifests for routing.
        if validate_settings {
            load_settings(&settings_file)?;
        }
        let env_file = if let Some(path) = options
            .env_file
            .as_ref()
            .filter(|p| !p.as_os_str().is_empty())
        {
            self.expand_user(path)?
        } else {
            [
                config_root.join("backends.env"),
                state_root.join("config.env"),
                state_root.join(".env"),
            ]
            .into_iter()
            .find(|p| p.exists())
            .unwrap_or_else(|| config_root.join("backends.env"))
        };
        if require_layout
            && ([
                &data_root,
                &config_root,
                &state_root.join("exports"),
                &state_root.join("attachments"),
            ]
            .into_iter()
            .any(|p| !p.is_dir())
                || !settings_file.is_file()
                || !env_file.parent().unwrap_or(Path::new(".")).is_dir())
        {
            return Err(Error::new(
                ErrorKind::NotInitialized,
                "runtime layout is not initialized",
            ));
        }
        Ok(RuntimePaths {
            database: resolve_database_path(&data_root),
            data_root,
            config_root,
            settings_file,
            exports_root: normalize_python_path(&state_root.join("exports")),
            attachments_root: normalize_python_path(&state_root.join("attachments")),
            state_root,
            env_file: normalize_python_path(&env_file),
            project_id: project.as_ref().map(|p| p.id.clone()),
            project_name: project.as_ref().map(|p| p.name.clone()),
            project_root: project.map(|p| p.path),
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResolveOptions {
    pub data_root: Option<PathBuf>,
    pub project: Option<String>,
    pub env_file: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePaths {
    pub data_root: PathBuf,
    pub env_file: PathBuf,
    pub state_root: PathBuf,
    pub config_root: PathBuf,
    pub settings_file: PathBuf,
    pub exports_root: PathBuf,
    pub attachments_root: PathBuf,
    pub database: PathBuf,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub project_root: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub encrypted: bool,
    pub last_opened_at: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub selected_project_id: Option<String>,
    pub projects: Vec<ProjectEntry>,
}

pub fn sanitize_project_id(value: &str) -> Result<String> {
    let mut result = String::new();
    let mut invalid = false;
    for c in value.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || "_.-".contains(c) {
            result.push(c);
            invalid = false;
        } else if !invalid {
            result.push('-');
            invalid = true;
        }
    }
    let result: String = result.trim_matches(['.', '-']).chars().take(80).collect();
    if result.is_empty() {
        return Err(Error::new(
            ErrorKind::Validation,
            "project id must contain at least one letter or number",
        ));
    }
    Ok(result)
}
pub fn load_catalog(path: &Path, context: &Context) -> Result<Catalog> {
    let Some(object) = read_private_json(path)? else {
        return Ok(Catalog::default());
    };
    check_schema(&object)?;
    let entries = object
        .get("projects")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::new(ErrorKind::UnsafeConfiguration, "project catalog is invalid"))?;
    let mut projects = Vec::new();
    for entry in entries {
        let get = |key| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::UnsafeConfiguration,
                        "project catalog entry is invalid",
                    )
                })
        };
        projects.push(ProjectEntry {
            id: sanitize_project_id(get("id")?)?,
            name: get("name")?.trim().into(),
            path: context.expand_user(Path::new(get("path")?))?,
            encrypted: entry.get("encrypted").is_some_and(python_truthy),
            last_opened_at: entry
                .get("last_opened_at")
                .and_then(Value::as_str)
                .map(String::from),
        });
    }
    let selected_project_id = object
        .get("selected_project_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(sanitize_project_id)
        .transpose()?
        .or_else(|| projects.first().map(|p| p.id.clone()));
    Ok(Catalog {
        selected_project_id,
        projects,
    })
}
fn python_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.),
    }
}
fn check_schema(object: &Map<String, Value>) -> Result<()> {
    if object.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(Error::new(
            ErrorKind::UnsupportedConfiguration,
            "unsupported configuration schema",
        ));
    }
    Ok(())
}
pub fn load_settings(path: &Path) -> Result<Map<String, Value>> {
    match read_private_json(path)? {
        Some(object) => {
            check_schema(&object)?;
            Ok(object)
        }
        None => Ok(Map::new()),
    }
}
/// Pathlib's missing-path rules; access errors stay present so reads fail closed.
/// A final dangling symlink is present, while missing/looping parents are absent.
pub fn preference_path_is_absent(path: &Path) -> bool {
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return true;
    }
    let Err(error) = fs::symlink_metadata(path) else {
        return false;
    };
    if matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    ) {
        return true;
    }
    #[cfg(unix)]
    if error
        .raw_os_error()
        .is_some_and(|number| [nix::libc::ELOOP, nix::libc::EBADF].contains(&number))
    {
        return true;
    }
    false
}

/// `os.open(path, os.O_RDONLY | os.O_NOFOLLOW)`: a final symlink fails to
/// open. The caller applies its own policy to the open file's metadata.
pub fn open_read_nofollow(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    options.open(path)
}

/// `os.getuid()`.
#[cfg(unix)]
pub fn current_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

/// Read at most `limit` bytes, without metadata's ownership/mode policy.
pub fn read_small_regular_file(path: &Path, limit: usize) -> Option<Vec<u8>> {
    if fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
        return None;
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let mut file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit.checked_add(1)? as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit).then_some(bytes)
}
/// Metadata is local authority: reject final symlinks, non-files, other owners,
/// group/world writes, malformed JSON and unknown schemas. Never rewrite.
pub fn read_private_json(path: &Path) -> Result<Option<Map<String, Value>>> {
    let unsafe_file = || {
        Error::new(
            ErrorKind::UnsafeConfiguration,
            "configuration is unreadable or unsafe",
        )
    };
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    if path.parent().is_some_and(Path::is_symlink)
        || fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(unsafe_file());
    }
    let mut file = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unsafe_file()),
    };
    let info = file.metadata().map_err(|_| unsafe_file())?;
    if !info.is_file() {
        return Err(unsafe_file());
    }
    #[cfg(windows)]
    if !windows::owned_by_current_user(path)? {
        return Err(unsafe_file());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !admit_metadata_permissions(info.uid(), info.mode(), nix::unistd::getuid().as_raw()) {
            return Err(unsafe_file());
        }
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unsafe_file())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(unsafe_file());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| unsafe_file())?;
    value.as_object().cloned().map(Some).ok_or_else(unsafe_file)
}
#[cfg(unix)]
fn admit_metadata_permissions(owner: u32, mode: u32, current_user: u32) -> bool {
    owner == current_user && mode & 0o022 == 0
}
pub fn resolve_database_path(data: &Path) -> PathBuf {
    let current = normalize_python_path(&data.join(DB_FILENAME));
    let legacy = normalize_python_path(&data.join(LEGACY_DB_FILENAME));
    if current.exists() || !legacy.exists() {
        current
    } else {
        legacy
    }
}
pub fn looks_like_plaintext(path: &Path) -> Result<bool> {
    let mut header = [0u8; 16];
    let mut file = File::open(path).map_err(Error::io)?;
    let count = file.read(&mut header).map_err(Error::io)?;
    Ok(count == 16 && &header == b"SQLite format 3\0")
}
fn movable_state(path: &Path) -> Result<bool> {
    match fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries {
                let name = entry.map_err(Error::io)?.file_name();
                if name != "bin" && name != "run" && name != MIGRATION_MARKER {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Ok(path.exists() || path.is_symlink()),
    }
}
/// Path.resolve(strict=False), including symlink parents and missing leaves.
pub fn canonicalize_allow_missing(path: &Path) -> Result<PathBuf> {
    canonicalize_with_depth(path, 0)
}
fn canonicalize_with_depth(path: &Path, depth: usize) -> Result<PathBuf> {
    if depth > 40 {
        return Err(Error::new(
            ErrorKind::UnsafeConfiguration,
            "too many symbolic links in local path",
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(Error::io)?.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => {
                resolved.pop();
            }
            other => {
                resolved.push(other.as_os_str());
                match fs::canonicalize(&resolved) {
                    Ok(p) => {
                        #[cfg(windows)]
                        {
                            resolved = windows::python_resolved_path(&p);
                        }
                        #[cfg(not(windows))]
                        {
                            resolved = p;
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                        ) =>
                    {
                        if resolved.is_symlink() {
                            let target = fs::read_link(&resolved).map_err(Error::io)?;
                            let target = if target.is_absolute() {
                                target
                            } else {
                                resolved.parent().unwrap_or(Path::new(".")).join(target)
                            };
                            resolved = canonicalize_with_depth(&target, depth + 1)?;
                        }
                    }
                    Err(e) => return Err(Error::io(e)),
                }
            }
        }
    }
    Ok(resolved)
}

#[cfg(test)]
#[test]
fn non_strict_resolution_keeps_suffixes_below_regular_files() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("blocked");
    fs::write(&file, b"regular file").unwrap();
    let output = file.join("backup.kassiber");
    assert_eq!(canonicalize_allow_missing(&output).unwrap(), output);
}

/// pathlib.Path collapses separators and current-directory components, but
/// preserves parent components and the special POSIX double-slash root.
fn normalize_python_path(path: &Path) -> PathBuf {
    let result: PathBuf = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    if result.as_os_str().is_empty() {
        return PathBuf::from(".");
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let bytes = path.as_os_str().as_bytes();
        if bytes.starts_with(b"//") && !bytes.starts_with(b"///") {
            let mut prefixed = vec![b'/'];
            prefixed.extend_from_slice(result.as_os_str().as_bytes());
            return PathBuf::from(OsString::from_vec(prefixed));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn bounded_preference_reader_never_follows_links_or_waits_on_fifos() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("preference");
        fs::write(&file, b"1234").unwrap();
        assert_eq!(read_small_regular_file(&file, 4), Some(b"1234".to_vec()));
        assert_eq!(read_small_regular_file(&file, 3), None);
        let link = root.path().join("link");
        symlink(&file, &link).unwrap();
        assert_eq!(read_small_regular_file(&link, 4), None);
        let fifo = root.path().join("fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRUSR).unwrap();
        assert_eq!(read_small_regular_file(&fifo, 4), None);
        assert_eq!(read_small_regular_file(root.path(), 4), None);
    }
    #[cfg(unix)]
    #[test]
    fn metadata_admission_accepts_readable_files_but_refuses_foreign_writers() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let contents = b"{\"schema_version\":1,\"selected_project_id\":null,\"projects\":[]}";
        fs::write(&path, contents).unwrap();
        for mode in [0o600, 0o644, 0o664, 0o666] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            if mode & 0o022 == 0 {
                assert!(load_settings(&path).is_ok(), "mode {mode:o}");
                assert!(
                    load_catalog(&path, &context(root.path())).is_ok(),
                    "mode {mode:o}"
                );
            } else {
                assert_eq!(
                    load_settings(&path).unwrap_err().kind,
                    ErrorKind::UnsafeConfiguration
                );
                assert_eq!(
                    load_catalog(&path, &context(root.path())).unwrap_err().kind,
                    ErrorKind::UnsafeConfiguration
                );
            }
            assert_eq!(fs::read(&path).unwrap(), contents);
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
        // Exercise foreign-owner policy without requiring privileged chown.
        let uid = nix::unistd::getuid().as_raw();
        for mode in [0o600, 0o644, 0o664, 0o666] {
            assert!(!admit_metadata_permissions(uid.wrapping_add(1), mode, uid));
        }
    }
    fn context(home: &Path) -> Context {
        Context {
            platform: Platform::Linux,
            home: home.into(),
            environ: BTreeMap::new(),
        }
    }
    #[test]
    fn roots_and_legacy_filename_are_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let c = context(temp.path());
        assert_eq!(
            c.default_state_root().unwrap(),
            temp.path().join(".local/share/kassiber")
        );
        fs::create_dir_all(temp.path().join(".kassiber/run")).unwrap();
        assert_eq!(c.default_state_root().unwrap(), c.native_state_root());
        fs::write(temp.path().join(".kassiber/unknown"), "").unwrap();
        // A hidden-home root the cutover left in place stays the state root,
        // as in Python: it resolves there (here, to an uninitialized layout).
        assert_eq!(
            c.default_state_root().unwrap(),
            temp.path().join(".kassiber")
        );
        assert_eq!(
            c.resolve(&ResolveOptions::default()).unwrap_err().kind,
            ErrorKind::NotInitialized
        );
        fs::create_dir_all(temp.path().join(".kassiber/data")).unwrap();
        fs::write(temp.path().join(".kassiber/data/kassiber.sqlite3"), "").unwrap();
        assert_eq!(
            c.resolve(&ResolveOptions::default()).unwrap_err().details,
            serde_json::json!({"reason": "legacy_project_layout"})
        );
        assert!(!c.native_state_root().exists());
        fs::write(temp.path().join(LEGACY_DB_FILENAME), "").unwrap();
        assert_eq!(
            resolve_database_path(temp.path()),
            temp.path().join(LEGACY_DB_FILENAME)
        );
    }
    #[test]
    fn xdg_relative_falls_back_and_native_platforms_match() {
        let mut c = context(Path::new("/fake"));
        c.environ.insert("XDG_DATA_HOME".into(), "relative".into());
        assert_eq!(
            c.native_state_root(),
            PathBuf::from("/fake/.local/share/kassiber")
        );
        c.platform = Platform::Macos;
        assert_eq!(
            c.native_state_root(),
            PathBuf::from("/fake/Library/Application Support/at.bitcoinaustria.kassiber")
        );
        c.platform = Platform::Windows;
        assert_eq!(
            c.native_state_root(),
            PathBuf::from("/fake/AppData/Local/at.bitcoinaustria.kassiber")
        );
    }
    #[test]
    fn config_rejects_symlink_and_unknown_schema() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        fs::write(&p, "{\"schema_version\":2}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_eq!(
            load_settings(&p).unwrap_err().kind,
            ErrorKind::UnsupportedConfiguration
        );
        #[cfg(unix)]
        {
            let alias = t.path().join("alias");
            std::os::unix::fs::symlink(&p, &alias).unwrap();
            assert_eq!(
                load_settings(&alias).unwrap_err().kind,
                ErrorKind::UnsafeConfiguration
            );
        }
    }
}
