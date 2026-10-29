//! Desktop-only restore capabilities. A preview retains a validated snapshot,
//! never either passphrase. Transport/lifecycle adapters implement `Host`.
use crate::*;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::{Duration, Instant};

pub const KINDS: &[&str] = &[
    "ui.backup.export",
    "ui.backup.preview",
    "ui.backup.apply",
    "ui.backup.cancel",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    files: Vec<(PathBuf, Vec<u128>)>,
    root: PathBuf,
    project: Option<String>,
    connection: usize,
    changes: u64,
    version: i64,
}

fn stamp(path: &Path, follow: bool) -> Result<Vec<u128>> {
    let meta = if follow {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
    .map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(vec![
            meta.dev().into(),
            meta.ino().into(),
            meta.len().into(),
            meta.mtime() as u128,
            meta.mtime_nsec() as u128,
        ])
    }
    #[cfg(not(unix))]
    {
        Ok(vec![
            meta.len().into(),
            meta.modified()
                .map_err(io_error)?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(io_error)?
                .as_nanos(),
        ])
    }
}

// pathlib.rglob does not descend through directory symlinks. Sort complete
// relative paths, as Python does, rather than relying on filesystem order.
fn descendants(root: &Path) -> Result<Vec<PathBuf>> {
    fn walk(root: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
        for item in fs::read_dir(root).map_err(io_error)? {
            let item = item.map_err(io_error)?;
            let path = item.path();
            let recurse = item.file_type().map_err(io_error)?.is_dir();
            paths.push(path.clone());
            if recurse {
                walk(&path, paths)?;
            }
        }
        Ok(())
    }
    let mut paths = vec![];
    walk(root, &mut paths)?;
    paths.sort_by(|a, b| a.components().cmp(b.components()));
    Ok(paths)
}

pub fn fingerprint(conn: &Connection, data: &Path, project: Option<&str>) -> Result<Fingerprint> {
    let targets = kassiber_store::restore::targets(data).map_err(service::restore_error)?;
    let mut files = vec![];
    for (_, target) in targets.iter().skip(1) {
        let mut paths = vec![target.clone()];
        if target.is_dir() {
            paths.extend(descendants(target)?);
        }
        for path in paths {
            if path.exists() {
                let info = stamp(&path, false)?;
                files.push((path, info));
            }
        }
    }
    files.push((targets[0].1.clone(), stamp(&targets[0].1, true)?));
    Ok(Fingerprint {
        files,
        root: kassiber_paths::canonicalize_allow_missing(data).map_err(service::restore_error)?,
        project: project.map(str::to_owned),
        connection: conn as *const Connection as usize,
        changes: conn.total_changes(),
        version: conn
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .map_err(sql_error)?,
    })
}

pub fn digest_tree(staging: &Path) -> Result<Vec<u8>> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    for path in descendants(staging)? {
        if path.is_symlink() {
            return Err(AppError::new(
                "stale_backup_preview",
                "Restore staging changed",
            ));
        }
        if path.is_file() {
            digest.update(
                path.strip_prefix(staging)
                    .expect("descendant")
                    .to_str()
                    .ok_or_else(|| AppError::new("internal_error", "Restore path is not UTF-8"))?
                    .as_bytes(),
            );
            let mut file = File::open(&path).map_err(io_error)?;
            loop {
                let size = file.read(&mut buffer).map_err(io_error)?;
                if size == 0 {
                    break;
                }
                digest.update(&buffer[..size]);
            }
        }
    }
    Ok(digest.finalize().to_vec())
}

pub fn summary(conn: &Connection) -> Result<Value> {
    let labels = |sql| -> Result<Value> {
        let mut query = conn.prepare(sql).map_err(sql_error)?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(sql_error)?;
        Ok(Value::List(
            rows.map(|r| r.map(s).map_err(sql_error))
                .collect::<Result<_>>()?,
        ))
    };
    Ok(dict([
        (
            "workspaces",
            labels("SELECT label FROM workspaces ORDER BY label LIMIT 100")?,
        ),
        (
            "books",
            labels("SELECT label FROM profiles ORDER BY label LIMIT 100")?,
        ),
        (
            "book_count",
            Value::from(
                conn.query_row("SELECT count(*) FROM profiles", [], |r| r.get::<_, i64>(0))
                    .map_err(sql_error)?,
            ),
        ),
    ]))
}

pub fn export(
    paths: &RuntimePaths,
    output: &Path,
    db_passphrase: Option<&str>,
    passphrase: &str,
    version: &str,
) -> Result<Value> {
    let db_passphrase = db_passphrase.filter(|p| !p.is_empty()).ok_or_else(|| {
        AppError::new(
            "plaintext_database",
            "Encrypt this container before exporting a backup",
        )
    })?;
    if passphrase.is_empty() {
        return Err(AppError::new(
            "backup_passphrase_required",
            "Both backup and database passphrases are required",
        ));
    }
    let result = crate::export(paths, output, Some(db_passphrase), Some(passphrase), None, version)
        .map_err(|error| if error.code == "internal_error" {
            AppError::new("backup_export_failed", "The backup could not be saved")
                .with_hint("Check destination permissions and available space, then choose the destination again.")
        } else { error })?;
    Ok(dict([
        ("path", text(&result.output)),
        (
            "size_bytes",
            Value::from(fs::metadata(&result.output).map_err(io_error)?.len()),
        ),
    ]))
}

pub fn check_close(
    conn: &Connection,
    data: &Path,
    project: Option<&str>,
    expected: &Fingerprint,
) -> Result<()> {
    if &fingerprint(conn, data, project)? != expected {
        return Err(AppError::new(
            "stale_backup_preview",
            "The target changed while preparing restore",
        ));
    }
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .map_err(sql_error)?;
    if busy != 0 {
        return Err(AppError::new(
            "project_in_use",
            "Close other database readers before restoring",
        ));
    }
    Ok(())
}

pub fn invalidate_credentials(
    namespace: &kassiber_ownership::OwnerNamespace,
    root: &Path,
) -> Result<()> {
    kassiber_secrets::policy::invalidate(namespace, root).map_err(service::restore_error)?;
    kassiber_secrets::policy::Policy::load(namespace, root)?.disable(true)
}

pub trait Host {
    fn data_root(&self) -> &Path;
    fn unlocked(&self) -> bool;
    fn fingerprint(&self) -> Result<Fingerprint>;
    fn summary(&self) -> Result<Value>;
    fn export(&mut self, output: &Path, passphrase: &str) -> Result<Value>;
    fn ensure_owner(&mut self) -> Result<()>;
    // The active desktop lease stays held until this guard is dropped.
    fn maintenance(&self) -> Result<Box<dyn Send>>;
    fn close_database(&mut self, expected: &Fingerprint) -> Result<()>;
    fn invalidate_credentials(&mut self) -> Result<()>;
    fn release_owner(&mut self) -> Result<()>;
}

struct Preview {
    token: String,
    imported: Import,
    target: Fingerprint,
    digest: Vec<u8>,
    expires: Instant,
    incoming: Value,
    unavailable: bool,
}

#[derive(Default)]
pub struct Sessions {
    preview: Option<Preview>,
}
impl Sessions {
    pub fn remaining(&self) -> Option<Duration> {
        self.preview
            .as_ref()
            .map(|p| p.expires.saturating_duration_since(Instant::now()))
    }
    pub fn clear(&mut self) {
        self.preview = None;
    }
    pub fn expire(&mut self) {
        if self
            .preview
            .as_ref()
            .is_some_and(|p| p.expires <= Instant::now())
        {
            self.clear();
        }
    }
    pub fn handle(&mut self, host: &mut dyn Host, kind: &str, args: &Value) -> Result<Value> {
        self.expire();
        if kind == "ui.backup.cancel" {
            if self
                .preview
                .as_ref()
                .is_some_and(|p| get(args, "token") == Some(&s(p.token.as_str())))
            {
                self.clear();
            }
            return Ok(dict([("cancelled", Value::Bool(true))]));
        }
        if !host.unlocked() {
            return Err(AppError::new(
                "backup_target_locked",
                "Unlock the target container before managing backups",
            ));
        }
        match kind {
            "ui.backup.export" => {
                let path = string(args, "path")
                    .map(Path::new)
                    .filter(|p| p.is_absolute() && p.extension().is_some_and(|e| e == "kassiber"))
                    .ok_or_else(|| {
                        AppError::new(
                            "invalid_backup_destination",
                            "Choose a .kassiber backup destination",
                        )
                    })?;
                let target = kassiber_paths::canonicalize_allow_missing(path)
                    .map_err(service::restore_error)?;
                let targets = kassiber_store::restore::targets(host.data_root())
                    .map_err(service::restore_error)?;
                let state = targets[4].1.parent().expect("attachments parent");
                if target.starts_with(state) {
                    return Err(AppError::new(
                        "invalid_backup_destination",
                        "Save the backup outside the active container",
                    ));
                }
                // Host checks plaintext before requesting the outer secret.
                host.export(
                    &target,
                    string(args, "backup_passphrase_secret").unwrap_or(""),
                )
            }
            "ui.backup.preview" => {
                self.clear();
                let archive = string(args, "path")
                    .map(Path::new)
                    .filter(|p| p.is_absolute())
                    .ok_or_else(|| AppError::new("missing_backup", "Choose a backup file"))?;
                let prepare = || -> Result<Preview> {
                    let imported = import(
                        archive,
                        Some(secret(args, "backup_passphrase_secret")?),
                        None,
                    )?;
                    let db = imported.staging().join(DATABASE);
                    if kassiber_paths::looks_like_plaintext(&db).map_err(service::restore_error)? {
                        return Err(AppError::new(
                            "invalid_backup",
                            "Backup database is not encrypted",
                        ));
                    }
                    let conn = kassiber_store::encryption::open_backup_container(
                        &db,
                        secret(args, "database_passphrase_secret")?,
                    )
                    .map_err(service::restore_error)?;
                    let check: String = conn
                        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                        .map_err(sql_error)?;
                    if check != "ok" {
                        return Err(AppError::new(
                            "invalid_backup",
                            "Backup database integrity check failed",
                        ));
                    }
                    // Validate summaries before retaining the capability.
                    let incoming = summary(&conn)?;
                    conn.close().map_err(|(_, e)| sql_error(e))?;
                    let target = host.fingerprint()?;
                    let digest = digest_tree(&imported.staging())?;
                    let unavailable = !imported.unavailable.is_empty();
                    Ok(Preview {
                        token: token()?,
                        imported,
                        target,
                        digest,
                        expires: Instant::now() + Duration::from_secs(600),
                        incoming,
                        unavailable,
                    })
                };
                let preview = prepare().map_err(|e| {
                    if ["backup_passphrase_required", "missing_backup"].contains(&e.code.as_str()) {
                        e
                    } else {
                        invalid_backup()
                    }
                })?;
                let replaces = kassiber_store::restore::targets(host.data_root())
                    .map_err(service::restore_error)?
                    .into_iter()
                    .filter(|(_, p)| p.exists())
                    .map(|(name, _)| s(name))
                    .collect();
                let result = dict([
                    ("token", s(preview.token.as_str())),
                    ("target_data_root", text(&preview.target.root)),
                    ("target", host.summary().map_err(|_| invalid_backup())?),
                    ("incoming", preview.incoming.clone()),
                    (
                        "attachments_files",
                        get(
                            get(&preview.imported.manifest, "entries").unwrap_or(&Value::Null),
                            "attachments_files",
                        )
                        .cloned()
                        .unwrap_or(Value::Null),
                    ),
                    ("replaces", Value::List(replaces)),
                    ("unavailable_secrets", Value::Bool(preview.unavailable)),
                    ("expires_in_seconds", Value::from(600)),
                ]);
                self.preview = Some(preview);
                Ok(result)
            }
            "ui.backup.apply" => {
                if get(args, "confirm") != Some(&s("RESTORE")) {
                    return Err(AppError::new(
                        "backup_confirmation_required",
                        "Confirm replacement before restoring",
                    ));
                }
                if self
                    .preview
                    .as_ref()
                    .is_none_or(|p| get(args, "token") != Some(&s(p.token.as_str())))
                {
                    self.clear();
                    return Err(stale());
                }
                let target = host.fingerprint()?;
                if self.preview.as_ref().is_some_and(|p| p.target != target) {
                    self.clear();
                    return Err(stale());
                }
                let preview = self.preview.as_ref().expect("checked preview");
                if preview.digest != digest_tree(&preview.imported.staging())? {
                    self.clear();
                    return Err(AppError::new(
                        "stale_backup_preview",
                        "The staged backup changed",
                    ));
                }
                host.ensure_owner()?;
                let _maintenance = host.maintenance()?;
                let preview = self.preview.take().expect("checked preview");
                let result = (|| {
                    host.close_database(&preview.target)?;
                    let recovery = kassiber_store::restore::install(
                        &preview.imported.staging(),
                        host.data_root(),
                    )
                    .map_err(service::restore_error)?;
                    let warning = host
                        .invalidate_credentials()
                        .err()
                        .map(|_| s("restore_unlock_settings_failed"))
                        .unwrap_or(Value::Null);
                    Ok(dict([
                        ("warning", warning),
                        ("restored", Value::Bool(true)),
                        ("locked", Value::Bool(true)),
                        (
                            "pre_restore_backup",
                            recovery.as_deref().map_or(Value::Null, text),
                        ),
                        ("unavailable_secrets", Value::Bool(preview.unavailable)),
                    ]))
                })();
                let result = result.map_err(|mut e: AppError| {
                    if !host.unlocked() {
                        let mut fields = match e.details.take() {
                            Some(Value::Dict(p)) => p,
                            _ => vec![],
                        };
                        fields.retain(|(k, _)| *k != "locked".into());
                        fields.push(("locked".into(), Value::Bool(true)));
                        e.details = Some(Value::Dict(fields));
                    }
                    e
                });
                drop(preview);
                if !host.unlocked() {
                    host.release_owner()?;
                }
                result
            }
            _ => Err(AppError::new(
                "unsupported_kind",
                "Unsupported backup action",
            )),
        }
    }
}
fn string<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    match get(args, key) {
        Some(Value::Str(s)) => s.as_str(),
        _ => None,
    }
}
fn secret<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    match get(args, key) {
        Some(Value::Str(value)) if value.code_points().next().is_some() => value
            .as_str()
            .ok_or_else(|| AppError::new("internal_error", "passphrase is not valid UTF-8")),
        _ => Err(AppError::new(
            "backup_passphrase_required",
            "Both backup and database passphrases are required",
        )),
    }
}

fn invalid_backup() -> AppError {
    AppError::new("invalid_backup", "The backup could not be opened or validated")
        .with_hint("Check the backup passphrase and the database passphrase used when this backup was created, then choose a valid backup file.")
}
fn stale() -> AppError {
    AppError::new(
        "stale_backup_preview",
        "The restore preview is no longer current",
    )
    .with_hint("Preview the backup again in the target container.")
}
fn token() -> Result<String> {
    #[cfg(feature = "test-hooks")]
    if let Ok(token) = std::env::var("KASSIBER_RUST_TEST_BACKUP_TOKEN") {
        return Ok(token);
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(io_error)?;
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let (mut bits, mut count) = (0u32, 0);
    let mut out = String::new();
    for byte in bytes {
        bits = (bits << 8) | u32::from(byte);
        count += 8;
        while count >= 6 {
            count -= 6;
            out.push(ALPHABET[((bits >> count) & 63) as usize] as char);
        }
    }
    if count != 0 {
        out.push(ALPHABET[((bits << (6 - count)) & 63) as usize] as char);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "desktop_tests.rs"]
mod tests;
