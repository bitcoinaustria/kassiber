//! The shell adapter. Paths, authentication, migrations and exclusion live in
//! the storage crates; this module projects their results onto R1's transport
//! contract without opening a network client.
mod backends;
mod journals;
mod lightning;
mod rates_rebuild;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use kassiber_envelope::{dict, s, AppError};
use kassiber_ownership::{OwnerLease, OwnerNamespace};
use kassiber_paths::{Context, ResolveOptions};
use kassiber_pyjson::Value;
use kassiber_runtime::store::*;
use kassiber_secrets::{NativeCredentialReader, UnlockOptions, UnlockSession};
use kassiber_store::rusqlite::{Connection, OptionalExtension};
use kassiber_store::{map_sqlite_error, Book as StoredBook, OpenOptions};

#[cfg(feature = "test-harness")]
struct FixedSyncSources {
    prefix: String,
    ids: u64,
    now: i64,
}

#[cfg(feature = "test-harness")]
impl kassiber_wallets::publication::wallet::Sources for FixedSyncSources {
    fn id(&mut self) -> String {
        self.ids += 1;
        format!("{}-{:04}", self.prefix, self.ids)
    }
    fn now(&mut self) -> String {
        self.now += 1;
        kassiber_chain::format_unix_time(1_893_456_000 + self.now).unwrap()
    }
}
fn sync_sources() -> (Box<dyn kassiber_wallets::publication::wallet::Sources>, u64) {
    #[cfg(feature = "test-harness")]
    if let Ok(prefix) = std::env::var("KASSIBER_RUST_TEST_SYNC_PREFIX") {
        return (
            Box::new(FixedSyncSources {
                prefix,
                ids: 0,
                now: std::env::var("KASSIBER_RUST_TEST_SYNC_NOW")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
            }),
            std::env::var("KASSIBER_RUST_TEST_SYNC_START")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1_700_000_000),
        );
    }
    (
        Box::new(kassiber_wallets::publication::wallet::SystemSources::default()),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
}

#[cfg(feature = "test-harness")]
struct FixedGraphSources {
    prefix: String,
}
#[cfg(feature = "test-harness")]
impl kassiber_wallets::publication::wallet::Sources for FixedGraphSources {
    fn id(&mut self) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static IDS: AtomicU64 = AtomicU64::new(0);
        format!(
            "{}-{:04}",
            self.prefix,
            IDS.fetch_add(1, Ordering::Relaxed) + 1
        )
    }
    fn now(&mut self) -> String {
        "2030-01-01T00:00:01Z".into()
    }
}
fn graph_sources() -> Box<dyn kassiber_wallets::publication::wallet::Sources> {
    #[cfg(feature = "test-harness")]
    if let Ok(prefix) = std::env::var("KASSIBER_RUST_TEST_GRAPH_PREFIX") {
        return Box::new(FixedGraphSources { prefix });
    }
    sync_sources().0
}

#[derive(Default)]
pub struct RealStore {
    auth: Mutex<()>,
}

fn error(error: kassiber_paths::Error) -> AppError {
    let details = kassiber_pytext::loads(&error.details.to_string()).ok();
    AppError {
        code: error.code.to_owned(),
        message: error.message.to_owned(),
        hint: error.hint.map(Box::from),
        details,
        retryable: error.retryable,
        message_codepoints: None,
        sqlite_origin: false,
    }
}

fn namespace() -> Result<OwnerNamespace, AppError> {
    #[cfg(feature = "test-harness")]
    if let Some(root) = std::env::var_os("KASSIBER_RUST_TEST_OWNER_ROOT") {
        return Ok(OwnerNamespace::isolated(root.into()));
    }
    OwnerNamespace::system().map_err(error)
}

fn context() -> Result<Context, AppError> {
    Context::from_env().map_err(error)
}
fn options(selection: &Selection) -> ResolveOptions {
    ResolveOptions {
        data_root: selection.data_root.as_ref().map(PathBuf::from),
        project: selection.project.clone(),
        env_file: selection.env_file.as_ref().map(PathBuf::from),
    }
}
fn path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn paths(p: &kassiber_paths::RuntimePaths) -> RuntimePaths {
    RuntimePaths {
        data_root: path(&p.data_root),
        env_file: path(&p.env_file),
        state_root: path(&p.state_root),
        config_root: path(&p.config_root),
        settings_file: path(&p.settings_file),
        exports_root: path(&p.exports_root),
        attachments_root: path(&p.attachments_root),
        database: path(&p.database),
        project_id: p.project_id.clone(),
        project_name: p.project_name.clone(),
        project_root: p.project_root.as_deref().map(path),
    }
}
fn project(p: &kassiber_ownership::CanonicalProject) -> ProjectRef {
    ProjectRef {
        identity: p.identity.clone(),
        public_id: p.public_id.clone(),
        data_root: path(p.database.parent().expect("database parent")),
    }
}
/// `kassiber.projects.now_iso()` for the catalog and layout markers. Only
/// test builds accept a fixed project clock.
fn timestamp() -> Result<String, AppError> {
    #[cfg(feature = "test-harness")]
    if let Ok(value) = std::env::var("KASSIBER_RUST_TEST_PROJECT_TIMESTAMP") {
        return Ok(value);
    }
    Connection::open_in_memory()
        .map_err(map_sqlite_error)
        .map_err(error)?
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
            r.get(0)
        })
        .map_err(map_sqlite_error)
        .map_err(error)
}
fn setting(conn: &Connection, key: &str) -> Result<Option<String>, AppError> {
    conn.query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(map_sqlite_error)
    .map_err(error)
}

struct RealOwner {
    lease: Option<OwnerLease>,
    project: ProjectRef,
}
impl ProjectOwner for RealOwner {
    fn project(&self) -> &ProjectRef {
        &self.project
    }
    fn release(&mut self) -> Result<(), AppError> {
        self.lease.take();
        Ok(())
    }
    fn refresh_after_open(&mut self) -> Result<(), AppError> {
        if let Some(lease) = &mut self.lease {
            lease.bind_created_database(&namespace()?).map_err(error)?;
            self.project = project(lease.project());
        }
        Ok(())
    }
}

impl BookStore for RealStore {
    #[cfg(feature = "test-harness")]
    fn journal_replay_guard(&self, selection: &Selection) -> Option<Box<dyn Fn() -> bool + Send>> {
        // Resolve without bootstrap: layout creation, restore recovery and
        // compatibility migrations must all remain inside this comparison.
        let database = context()
            .ok()?
            .resolve_for_bootstrap(&options(selection))
            .ok()?
            .database;
        if !database.is_file() {
            return None;
        }
        let guard = kassiber_journals::service::ReplayGuard::capture_path(&database);
        let selection = selection.clone();
        Some(Box::new(move || {
            context()
                .ok()
                .and_then(|context| context.resolve_for_bootstrap(&options(&selection)).ok())
                .is_some_and(|paths| paths.database == database)
                && guard.files_unchanged()
        }))
    }
    fn restore_maintenance(&self, data_root: &str) -> Result<Box<dyn Send>, AppError> {
        Ok(Box::new(
            kassiber_store::restore::maintenance(
                &namespace()?,
                Path::new(data_root),
                Some(kassiber_ownership::OwnerKind::Desktop),
            )
            .map_err(kassiber_backup::service::restore_error)?,
        ))
    }
    fn restore_invalidate(
        &self,
        data_root: &str,
        project_id: Option<&str>,
    ) -> Result<RuntimeConfig, AppError> {
        let namespace = namespace()?;
        let root = Path::new(data_root);
        kassiber_backup::desktop::invalidate_credentials(&namespace, root)?;
        let resolved = context()?
            .resolve_for_bootstrap(&ResolveOptions {
                data_root: Some(root.to_owned()),
                ..Default::default()
            })
            .map_err(error)?;
        let config = backends::Config::load(&resolved.env_file)?;
        if let Some(id) = project_id {
            kassiber_setup::projects::get_project(&context()?, id)?;
        }
        Ok(RuntimeConfig {
            default_backend: Some(config.default_backend),
            env_file: path(&resolved.env_file),
        })
    }
    fn rotate_passphrase(
        &self,
        data_root: &str,
        env_file: &str,
        current: &Secret,
        new: &Secret,
    ) -> Result<Value, AppError> {
        let resolved = context()?
            .resolve_for_bootstrap(&options(&Selection {
                data_root: Some(data_root.into()),
                env_file: Some(env_file.into()),
                project: None,
            }))
            .map_err(error)?;
        let current =
            kassiber_secrets::Passphrase::new(zeroize::Zeroizing::new(current.expose().into()))
                .map_err(error)?;
        let new = kassiber_secrets::Passphrase::new(zeroize::Zeroizing::new(new.expose().into()))
            .map_err(error)?;
        kassiber_secrets::service::rotate(
            &namespace()?,
            &resolved,
            &current,
            &new,
            &NativeCredentialReader,
            Some(kassiber_ownership::OwnerKind::Desktop),
        )
    }
    fn init_encryption(&self, data_root: &str, passphrase: &Secret) -> Result<Value, AppError> {
        let namespace = namespace()?;
        let root = Path::new(data_root);
        let database = kassiber_paths::resolve_database_path(root);
        let present = std::fs::metadata(&database).is_ok_and(|m| m.len() > 0);
        if present {
            let result = kassiber_store::encryption::encrypt_plaintext(
                &namespace,
                root,
                passphrase.expose(),
                Some(kassiber_ownership::OwnerKind::Desktop),
            )
            .map_err(error)?;
            return Ok(dict([
                ("encrypted", Value::Bool(true)),
                ("already_encrypted", Value::Bool(false)),
                ("database", s(path(&result.encrypted_path))),
                ("backup_path", s(path(&result.backup_path))),
                ("integrity_check", s("ok")),
                ("cipher_integrity_check", s("ok")),
                ("credential_marker_clean", Value::Bool(true)),
            ]));
        }
        let _maintenance = kassiber_store::restore::maintenance(
            &namespace,
            root,
            Some(kassiber_ownership::OwnerKind::Desktop),
        )
        .map_err(kassiber_backup::service::restore_error)?;
        kassiber_store::encryption::create_empty_database(&database, passphrase.expose())
            .map_err(error)?;
        Ok(dict([
            ("encrypted", Value::Bool(true)),
            ("already_encrypted", Value::Bool(false)),
            ("database", s(path(&database))),
            ("backup_path", Value::Null),
            ("integrity_check", s("ok")),
            ("cipher_integrity_check", Value::Null),
            ("credential_marker_clean", Value::Bool(true)),
        ]))
    }
    fn forget_cli_unlock(&self, data_root: &str) -> Result<Value, AppError> {
        kassiber_secrets::policy::forget(
            &namespace()?,
            Path::new(data_root),
            &NativeCredentialReader,
            true,
        )
    }
    fn local_service(&self, operation: &str, args: &Value) -> Result<Value, AppError> {
        if operation == "verify-download" {
            let field = |key| {
                kassiber_envelope::py_str(kassiber_envelope::get(args, key).unwrap_or(&Value::Null))
            };
            return kassiber_release::verify::verify_download(
                &field("artifact"),
                &field("manifest"),
                &field("signature"),
                &field("public_key"),
                &field("fingerprint"),
            );
        }
        if let Some(action) = operation.strip_prefix("update.") {
            use kassiber_release::update;
            let context = context()?;
            let version = kassiber_envelope::py_str(
                kassiber_envelope::get(args, "version").unwrap_or(&Value::Null),
            );
            let channel = match kassiber_envelope::get(args, "channel") {
                Some(Value::Str(text)) => text.as_str().map(str::to_owned),
                _ => None,
            };
            let build = update::Build {
                version: &version,
                channel: channel.as_deref(),
            };
            return match action {
                "disable" | "enable" => {
                    update::set_enabled(&context, action == "enable").map(|_| Value::Null)
                }
                "status" => update::preference(&context),
                "check" => update::check(&context, &build),
                _ => Err(not_implemented("update", operation)),
            };
        }
        if operation == "ui.egress.allowlist" {
            // The bootstrap runtime view a locked desktop daemon holds.
            let root = kassiber_envelope::py_str(
                kassiber_envelope::get(args, "data_root").unwrap_or(&Value::Null),
            );
            let config =
                backends::Config::load(Path::new(&self.runtime_config_for(&root)?.env_file))?;
            return egress_inputs(&config, None);
        }
        if operation == "ui.egress.database" {
            let root = kassiber_envelope::py_str(
                kassiber_envelope::get(args, "data_root").unwrap_or(&Value::Null),
            );
            let effective = context()?
                .effective_data_root(Path::new(&root))
                .map_err(error)?;
            return Ok(s(path(&kassiber_paths::resolve_database_path(&effective))));
        }
        if operation.starts_with("secrets.") || operation.starts_with("backup.") {
            let text = |field| {
                kassiber_envelope::get(args, field).and_then(|v| match v {
                    Value::Str(text) => text.as_str().map(str::to_owned),
                    _ => None,
                })
            };
            let resolved = kassiber_store::restore::resolve_for_bootstrap(
                &context()?,
                &namespace()?,
                &options(&Selection {
                    data_root: text("data_root"),
                    project: text("project"),
                    env_file: text("env_file"),
                }),
            )
            .map_err(kassiber_backup::service::restore_error)?;
            return if operation.starts_with("backup.") {
                kassiber_backup::service::execute(
                    &namespace()?,
                    &resolved,
                    operation,
                    args,
                    env!("CARGO_PKG_VERSION"),
                )
            } else {
                kassiber_secrets::service::execute(&namespace()?, &resolved, operation, args)
            };
        }
        if operation.starts_with("offline.") || operation.starts_with("ui.network.offline") {
            return kassiber_setup::offline::execute(&context()?, operation, args);
        }
        let passphrase = if operation == "projects.create" {
            match kassiber_envelope::get(args, "new_passphrase_fd") {
                Some(Value::Int(fd)) => {
                    let fd = i32::try_from(fd.clone()).map_err(|_| {
                        AppError::new(
                            "invalid_passphrase",
                            "passphrase descriptor must be non-negative",
                        )
                    })?;
                    let secret = kassiber_secrets::read_passphrase_from_fd(fd).map_err(error)?;
                    if secret.expose().chars().count() < 12 {
                        return Err(AppError::new(
                            "invalid_passphrase",
                            "passphrase must be at least 12 characters long",
                        )
                        .with_hint("Pick a long passphrase from a password manager."));
                    }
                    Some(secret)
                }
                _ => None,
            }
        } else {
            None
        };
        let context = context()?;
        let result = kassiber_setup::projects::execute(&context, operation, args)?;
        if let Some(secret) = passphrase {
            let field = |key| {
                kassiber_envelope::get(&result, key)
                    .map(kassiber_envelope::py_str)
                    .expect("project field")
            };
            kassiber_store::encryption::create_empty_database(
                Path::new(&field("database")),
                secret.expose(),
            )
            .map_err(error)?;
            return kassiber_setup::projects::created_encrypted(
                &context,
                &field("id"),
                !kassiber_envelope::get(args, "no_select").is_some_and(kassiber_envelope::truthy),
                &timestamp()?,
            );
        }
        Ok(result)
    }
    fn migrate_default_state_root(&self) -> Result<(), AppError> {
        kassiber_store::state_root::migrate_hidden_home(&context()?)
            .map(|_| ())
            .map_err(error)
    }
    fn bootstrap(&self, selection: &Selection) -> Result<Bootstrap, AppError> {
        let context = context()?;
        if selection.data_root.is_none() && selection.project.is_none() {
            kassiber_store::projects::ensure_default(
                &context,
                None,
                &timestamp()?,
                &mut |_| Ok(()),
            )
            .map_err(error)?;
        }
        let resolved = kassiber_store::restore::resolve_for_bootstrap(
            &context,
            &namespace()?,
            &options(selection),
        )
        .map_err(kassiber_backup::service::restore_error)?;
        kassiber_store::layout::initialize_layout(&resolved).map_err(error)?;
        let config = backends::Config::load(&resolved.env_file)?;
        Ok(Bootstrap {
            paths: paths(&resolved),
            runtime_config: RuntimeConfig {
                default_backend: Some(config.default_backend),
                env_file: path(&resolved.env_file),
            },
            selection_explicit: selection.data_root.is_some() || selection.project.is_some(),
        })
    }
    fn effective_unlock_mode(&self, selection: &Selection) -> Result<UnlockMode, AppError> {
        let context = context()?;
        let resolved = match kassiber_store::restore::resolve_for_bootstrap(
            &context,
            &namespace()?,
            &options(selection),
        ) {
            Ok(p) => p,
            Err(e)
                if selection.data_root.is_none()
                    && selection.project.is_none()
                    && matches!(
                        e.kind,
                        kassiber_paths::ErrorKind::NotInitialized
                            | kassiber_paths::ErrorKind::RequiresPythonMigration
                    ) =>
            {
                kassiber_store::projects::ensure_default(
                    &context,
                    None,
                    &timestamp()?,
                    &mut |_| Ok(()),
                )
                .map_err(error)?;
                kassiber_store::restore::resolve_for_bootstrap(
                    &context,
                    &namespace()?,
                    &options(selection),
                )
                .map_err(kassiber_backup::service::restore_error)?
            }
            Err(e) => return Err(kassiber_backup::service::restore_error(e)),
        };
        let settings = kassiber_paths::load_settings(&resolved.settings_file).map_err(error)?;
        let configured = match settings
            .get("operator_unlock_mode")
            .filter(|v| !v.is_null())
        {
            None => UnlockMode::Manual,
            Some(v) if v == "manual" => UnlockMode::Manual,
            Some(v) if v == "brokered" => UnlockMode::Brokered,
            Some(v) if v == "unattended" => UnlockMode::Unattended,
            _ => {
                return Err(AppError::new(
                    "invalid_operator_unlock_mode",
                    "the configured operator unlock mode is invalid",
                )
                .with_hint(
                    "Choose manual, brokered, or unattended with `kassiber operator mode`.",
                ))
            }
        };
        if configured != UnlockMode::Manual {
            let current = namespace()?
                .canonical_project(&resolved.data_root)
                .map_err(error)?;
            let bound = settings
                .get("operator_policy_project_identity")
                .and_then(|v| v.as_str())
                == Some(&current.identity)
                && settings
                    .get("operator_policy_database_identity")
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| kassiber_store::valid_hex(v, 32));
            if !bound {
                return Ok(UnlockMode::Manual);
            }
        }
        Ok(configured)
    }
    fn database_is_encrypted(&self, root: &str) -> Result<bool, AppError> {
        let db = kassiber_paths::resolve_database_path(Path::new(root));
        Ok(db.is_file()
            && std::fs::metadata(&db)
                .map_err(kassiber_paths::Error::io)
                .map_err(error)?
                .len()
                > 0
            && !kassiber_paths::looks_like_plaintext(&db).map_err(error)?)
    }
    fn canonical_project(&self, root: &str) -> Result<ProjectRef, AppError> {
        namespace()?
            .canonical_project(Path::new(root))
            .map(|p| project(&p))
            .map_err(error)
    }
    fn acquire_ownership(
        &self,
        p: &ProjectRef,
        _: OwnerKind,
        generation: &str,
    ) -> Result<Box<dyn ProjectOwner>, AppError> {
        let ns = namespace()?;
        let canonical = ns
            .canonical_project(Path::new(&p.data_root))
            .map_err(error)?;
        if canonical.identity != p.identity {
            return Err(AppError::new(
                "operator_project_replaced",
                "the project database was replaced",
            ));
        }
        let lease = ns
            .acquire(
                canonical,
                kassiber_ownership::OwnerKind::Desktop,
                generation,
            )
            .map_err(error)?;
        Ok(Box::new(RealOwner {
            lease: Some(lease),
            project: p.clone(),
        }))
    }
    fn open_book(
        &self,
        root: &str,
        unlock: Unlock<'_>,
        config: &mut RuntimeConfig,
    ) -> Result<Box<dyn Book>, AppError> {
        let ns = namespace()?;
        let canonical = ns.canonical_project(Path::new(root)).map_err(error)?;
        if let Unlock::Daemon {
            expected_project_identity,
            ..
        } = &unlock
        {
            if *expected_project_identity != canonical.identity {
                return Err(AppError::new(
                    "operator_project_replaced",
                    "the project database was replaced before opening",
                ));
            }
        }
        let retained_key = std::cell::RefCell::new(None);
        let open = |passphrase: Option<&str>,
                    expected_database_identity: Option<&str>,
                    require_existing_schema| {
            *retained_key.borrow_mut() = passphrase.map(|v| Secret::new(v.to_owned()));
            #[cfg(feature = "test-harness")]
            if let (Ok(identity), Ok(timestamp)) = (
                std::env::var("KASSIBER_RUST_TEST_MIGRATION_IDENTITY"),
                std::env::var("KASSIBER_RUST_TEST_MIGRATION_TIMESTAMP"),
            ) {
                return StoredBook::open_migrating_with_sources(
                    Path::new(root),
                    OpenOptions {
                        passphrase,
                        expected_database_identity,
                        expected_project: Some(&canonical),
                    },
                    require_existing_schema,
                    &identity,
                    &timestamp,
                );
            }
            StoredBook::open_migrating(
                Path::new(root),
                OpenOptions {
                    passphrase,
                    expected_database_identity,
                    expected_project: Some(&canonical),
                },
                require_existing_schema,
            )
        };
        let initialize_version = match &unlock {
            Unlock::Cli {
                initialize_version, ..
            } => *initialize_version,
            _ => None,
        };
        let book = match unlock {
            Unlock::Cli {
                passphrase_fd,
                allow_prompt,
                expected_database_identity,
                initialize_version: _,
            } => {
                let mut session = UnlockSession::default();
                if let Some(fd) = passphrase_fd {
                    session
                        .prime_fd(i32::try_from(fd).map_err(|_| {
                            AppError::new("invalid_passphrase", "invalid passphrase descriptor")
                        })?)
                        .map_err(error)?;
                }
                let result = session.open_in_namespace(
                    Path::new(root),
                    UnlockOptions {
                        non_interactive: !allow_prompt,
                        expected_database_identity,
                    },
                    &NativeCredentialReader,
                    &ns,
                    |pass, id| open(pass, id, false),
                );
                for _ in session.take_notices() {
                    eprintln!("remembered_unlock_stale: stored passphrase did not unlock this database; run `kassiber secrets remember-unlock` to re-enroll.");
                }
                result.map_err(error)?
            }
            Unlock::Daemon {
                passphrase,
                require_existing_schema,
                expected_project_identity: _,
            } => open(
                passphrase.map(Secret::expose),
                None,
                require_existing_schema,
            )
            .map_err(error)?,
        };
        kassiber_store::projects::validate_after_unlock(
            &context()?,
            Path::new(root),
            book.connection(),
            &timestamp()?,
            &mut |_| Ok(()),
        )
        .map_err(error)?;
        let mut backends = backends::Config::load(Path::new(&config.env_file))?;
        if let Some(version) = initialize_version {
            backends.seed(book.connection(), version)?;
        }
        backends.merge(book.connection())?;
        let backend_kinds = backends.kinds();
        config.default_backend = Some(backends.default_backend.clone());
        Ok(Box::new(RealBook {
            lightning_gate: lightning::network_gate(),
            book: Some(book),
            data_root: root.to_owned(),
            rates: kassiber_rates::Providers::system(env!("CARGO_PKG_VERSION")),
            db_passphrase: retained_key.into_inner(),
            backends,
            backend_kinds,
            report_read_attempts: Default::default(),
        }))
    }
    fn verify_passphrase(
        &self,
        root: &str,
        scope: &str,
        passphrase: &Secret,
    ) -> Result<bool, AppError> {
        let _guard = self
            .auth
            .lock()
            .map_err(|_| AppError::new("internal_error", "authentication state unavailable"))?;
        let state = context()?
            .state_root_for_data(Path::new(root))
            .map_err(error)?
            .join("config/auth_backoff.json");
        let mut saved = kassiber_paths::read_private_json(&state)
            .map_err(error)?
            .unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let until = saved
            .get("locked_until")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let wait = (until - now).clamp(0.0, 30.0);
        if wait > 0.0 {
            if until > now + 30.0 {
                saved.insert("locked_until".into(), serde_json::json!(now + wait));
                write_auth(&state, &saved)?;
            }
            return Err(AppError::new(
                "local_auth_rate_limited",
                "too many failed passphrase attempts",
            )
            .retryable(true)
            .with_hint("Wait before trying the passphrase again.")
            .with_details(dict([
                ("scope", s(scope)),
                ("throttle", s("database")),
                (
                    "retry_after_seconds",
                    Value::Int(((wait + 0.999) as i64).max(1).into()),
                ),
            ])));
        }
        let verified = kassiber_store::verify_passphrase(Path::new(root), passphrase.expose())
            .map_err(error)?;
        if verified {
            match std::fs::remove_file(&state) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(error(kassiber_paths::Error::io(e))),
            }
        } else {
            let failures = saved
                .get("failures")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .saturating_add(1);
            let delay = if failures < 3 {
                0.0
            } else {
                (5.0 * 2.0_f64.powi((failures - 3).min(3) as i32)).min(30.0)
            };
            saved.insert("failures".into(), serde_json::json!(failures));
            saved.insert(
                "locked_until".into(),
                serde_json::json!(if delay > 0.0 { now + delay } else { 0.0 }),
            );
            write_auth(&state, &saved)?;
        }
        Ok(verified)
    }
    fn mark_project_opened(&self, id: &str, root: &str, select: bool) -> Result<(), AppError> {
        kassiber_setup::projects::record_opened(
            &context()?,
            id,
            Path::new(root),
            Some(&timestamp()?),
            select,
        )
        .map(|_| ())
    }
    fn project_catalog(&self, operation: ProjectCatalog<'_>) -> Result<Value, AppError> {
        use kassiber_setup::projects as catalog;
        let context = context()?;
        let entry = match operation {
            ProjectCatalog::List => {
                return Ok(Value::List(
                    catalog::list(&context)?
                        .iter()
                        .map(catalog::payload)
                        .collect(),
                ))
            }
            ProjectCatalog::Get(id) => catalog::get_project(&context, id)?,
            ProjectCatalog::Create {
                name,
                project_id,
                select,
            } => catalog::create_for_desktop(&context, name, project_id, select)?,
            ProjectCatalog::Opened {
                project_id,
                data_root,
                stamp,
                select,
            } => catalog::record_opened(
                &context,
                project_id,
                Path::new(data_root),
                stamp.then(timestamp).transpose()?.as_deref(),
                select,
            )?,
            ProjectCatalog::Select(id) => catalog::set_selected(&context, id, Some(&timestamp()?))?,
        };
        Ok(catalog::payload(&entry))
    }
    fn create_encrypted_database(
        &self,
        database: &str,
        passphrase: &Secret,
    ) -> Result<(), AppError> {
        kassiber_store::encryption::create_empty_database(Path::new(database), passphrase.expose())
            .map_err(error)
    }
    fn runtime_config_for(&self, data_root: &str) -> Result<RuntimeConfig, AppError> {
        let resolved = context()?
            .resolve_for_bootstrap(&ResolveOptions {
                data_root: Some(PathBuf::from(data_root)),
                ..Default::default()
            })
            .map_err(error)?;
        let config = backends::Config::load(&resolved.env_file)?;
        Ok(RuntimeConfig {
            default_backend: Some(config.default_backend),
            env_file: path(&resolved.env_file),
        })
    }
}
fn write_auth(
    path: &Path,
    saved: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), AppError> {
    kassiber_store::layout::write_private_json(path, &serde_json::Value::Object(saved.clone()))
        .map_err(error)
}

/// Deterministic clock injection exists only in test harness builds.
struct SetupSources;
impl kassiber_setup::Sources for SetupSources {
    fn uuid(&self, conn: &Connection) -> Result<String, AppError> {
        #[cfg(feature = "test-harness")]
        if std::env::var("KASSIBER_RUST_TEST_SETUP_IDS").as_deref() == Ok("sequential") {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(1);
            return Ok(format!(
                "00000000-0000-4000-8000-{:012x}",
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        }
        kassiber_setup::Sources::uuid(&kassiber_setup::SystemSources, conn)
    }
    fn now(&self, conn: &Connection) -> Result<String, AppError> {
        #[cfg(feature = "test-harness")]
        if let Ok(value) = std::env::var("KASSIBER_RUST_TEST_SETUP_TIMESTAMP") {
            return Ok(value);
        }
        kassiber_setup::Sources::now(&kassiber_setup::SystemSources, conn)
    }
}

struct RealBook {
    lightning_gate: kassiber_net::Gate,
    rates: kassiber_rates::Providers,
    backend_kinds: std::collections::BTreeMap<String, String>,
    book: Option<StoredBook>,
    data_root: String,
    db_passphrase: Option<Secret>,
    backends: backends::Config,
    /// Report-read refresh attempts of this open book, per profile: when the
    /// last one started and its payload. A reopened book starts empty.
    report_read_attempts: std::collections::HashMap<String, (std::time::Instant, Option<Value>)>,
}

struct NativeJournalReply<'a> {
    result: kassiber_journals::process::Committed<'a>,
    _slot: Option<kassiber_wallets::freshness::planning::Execution>,
}
impl JournalReply for NativeJournalReply<'_> {
    fn payload(&self) -> &Value {
        &self.result.ordered_payload
    }
    fn after_reply(self: Box<Self>) -> Result<(), AppError> {
        self.result
            .after_reply()
            .map_err(kassiber_journals::service::application_error)
    }
}

/// Only test builds accept an injected report clock. UUIDs retain their native
/// source and process oracles declare a one-to-one mapping for generated ids.
struct ReportSources;
#[derive(Default)]
struct AnalysisSources(kassiber_wallets::publication::wallet::SystemSources);
impl kassiber_wallets::publication::wallet::Sources for AnalysisSources {
    fn id(&mut self) -> String {
        #[cfg(feature = "test-harness")]
        if std::env::var("KASSIBER_RUST_TEST_ANALYSIS_IDS").as_deref() == Ok("sequential") {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let offset = std::env::var("KASSIBER_RUST_TEST_ANALYSIS_ID_OFFSET")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or_default();
            return format!(
                "00000000-0000-4000-8000-{:012x}",
                NEXT.fetch_add(1, Ordering::Relaxed).saturating_add(offset)
            );
        }
        self.0.id()
    }
    fn now(&mut self) -> String {
        #[cfg(feature = "test-harness")]
        if let Ok(stamp) = std::env::var("KASSIBER_RUST_TEST_ANALYSIS_TIME") {
            return stamp;
        }
        self.0.now()
    }
}
/// The background local-evidence pass on its own fenced connection.
struct WatchWorker(kassiber_wallets::analysis_ops::LocalWorker);
impl kassiber_runtime::store::WatchWorker for WatchWorker {
    fn pass(&mut self, cancelled: &dyn Fn() -> bool) -> Result<Vec<String>, AppError> {
        self.0.pass(cancelled)
    }
    fn delivered(&mut self, ids: &[String]) -> Result<(), AppError> {
        self.0.delivered(ids)
    }
}

/// The wall clock and a fresh, fail-closed read of the offline preference.
struct AnalysisRuntime;
impl kassiber_wallets::analysis_ops::Runtime for AnalysisRuntime {
    fn epoch(&self) -> i64 {
        #[cfg(feature = "test-harness")]
        if let Some(epoch) = std::env::var("KASSIBER_RUST_TEST_ANALYSIS_EPOCH")
            .ok()
            .and_then(|value| value.parse().ok())
        {
            return epoch;
        }
        kassiber_wallets::analysis_ops::SystemRuntime { offline: true }.epoch()
    }
    fn offline(&self) -> bool {
        Context::from_env()
            .ok()
            .and_then(|ctx| kassiber_setup::offline::preference_path(&ctx).ok())
            .is_none_or(|path| kassiber_setup::offline::enabled(&path))
    }
    /// Re-reads the preference on every route check, failing closed.
    fn gate(&self) -> kassiber_net::Gate {
        use kassiber_net::{Gate, Ledger, Offline};
        Gate::new(
            || match Context::from_env()
                .ok()
                .and_then(|ctx| kassiber_setup::offline::preference_path(&ctx).ok())
            {
                Some(path) if kassiber_setup::offline::enabled(&path) => Offline::Offline,
                Some(_) => Offline::Online,
                None => Offline::Unavailable,
            },
            Ledger::default(),
        )
    }
}
impl kassiber_setup::Sources for ReportSources {
    fn uuid(&self, conn: &Connection) -> kassiber_setup::Result<String> {
        kassiber_setup::SystemSources.uuid(conn)
    }
    fn now(&self, conn: &Connection) -> kassiber_setup::Result<String> {
        #[cfg(feature = "test-harness")]
        if let Ok(stamp) = std::env::var("KASSIBER_RUST_TEST_REPORT_TIME") {
            return Ok(stamp);
        }
        kassiber_setup::SystemSources.now(conn)
    }
}
impl RealBook {
    /// Workspace shell reads (`kassiber-workspace`); `None` for other operations.
    fn workspace_read(&self, operation: &str, args: &Value) -> Option<Result<Value, AppError>> {
        use kassiber_envelope::{get, truthy};
        use kassiber_workspace::{overview, readiness};
        if !matches!(
            operation,
            "health"
                | "next-actions"
                | "ui.workspace.health"
                | "ui.next_actions"
                | "ui.review.badges"
                | "ui.overview.snapshot"
                | "ui.workspace.overview.snapshot"
        ) {
            return None;
        }
        Some((|| {
            let conn = self.conn()?;
            match operation {
                "ui.overview.snapshot" => {
                    return overview::overview(conn, &Default::default(), &SetupSources)
                }
                "ui.workspace.overview.snapshot" => {
                    let args = (*args != Value::Null).then_some(args);
                    return overview::workspace_overview(
                        conn,
                        args,
                        &Default::default(),
                        &SetupSources,
                    );
                }
                _ => {}
            }
            let cli = !operation.starts_with("ui.");
            let flag = |key| get(args, key).is_some_and(truthy);
            if cli && flag("workspace") != flag("profile") {
                return Err(AppError::new(
                    "validation",
                    format!("{operation} requires --workspace and --profile together"),
                ));
            }
            let (context, profile) = if cli && flag("workspace") {
                let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
                let id = |row: &Value| kassiber_envelope::py_str(get(row, "id").unwrap());
                readiness::scope(conn, Some((&id(&workspace), &id(&profile))))?
            } else {
                readiness::scope(conn, None)?
            };
            if operation == "ui.review.badges" {
                return readiness::badges(conn, profile.as_ref());
            }
            let health = readiness::health(conn, &context, profile.as_ref())?;
            if matches!(operation, "health" | "ui.workspace.health") {
                return Ok(health);
            }
            readiness::next_actions(health, || {
                kassiber_rates::desktop_command(
                    conn,
                    "ui.rates.coverage",
                    &dict([("limit", Value::Int(1.into()))]),
                    &self.rates,
                )
            })
        })())
    }
    fn conn(&self) -> Result<&Connection, AppError> {
        self.book
            .as_ref()
            .map(StoredBook::connection)
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))
    }
}

impl Book for RealBook {
    fn refresh_journals(
        &mut self,
        context: &BookContext,
        auto_pair: bool,
        cancel: &kassiber_wallets::scan::Cancellation,
        progress: &mut dyn FnMut(&Value) -> Result<(), AppError>,
    ) -> Result<Box<dyn JournalReply + '_>, AppError> {
        let slot = kassiber_wallets::freshness::planning::Execution::acquire()?;
        let result = journals::explicit(self.conn()?, context, auto_pair, cancel, progress)?;
        Ok(Box::new(NativeJournalReply {
            result,
            _slot: Some(slot),
        }))
    }
    fn journal_view(&mut self, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
        if kind == "ui.journals.quarantine.resolve" {
            let empty = kassiber_envelope::dict::<&str>([]);
            let args = match args {
                None | Some(Value::Null) => &empty,
                Some(value @ Value::Dict(_)) => value,
                Some(other) => {
                    return Err(AppError::new("validation", "daemon args must be an object")
                        .with_details(kassiber_envelope::dict([(
                            "type",
                            kassiber_envelope::s(kassiber_envelope::type_name(other)),
                        )])))
                }
            };
            return journals::quarantine_resolve(
                self.conn()?,
                &|| self.resolve_scope(None, None),
                args,
            );
        }
        kassiber_journals::views::execute(self.conn()?, kind, args)
    }
    fn process_journals(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        auto_pair: bool,
        progress: &mut dyn FnMut(&'static str),
    ) -> Result<Box<dyn JournalReply + '_>, JournalFailure> {
        let conn = self.conn()?;
        let guard = kassiber_journals::service::ReplayGuard::capture(conn);
        let mut last_phase = "admission";
        let result = journals::process(conn, workspace, profile, auto_pair, &mut |phase| {
            last_phase = phase;
            progress(phase);
        })
        .map_err(|refusal| JournalFailure {
            // Typed AppError refusals are exact natively (registered-route
            // census); only exception families keep the development replay.
            replay_shape: (last_phase != "complete"
                && refusal.family != "AppError"
                && guard.unchanged(conn))
            .then(|| format!("{last_phase}:{}", refusal.family)),
            error: Box::new(kassiber_journals::service::application_error(refusal)),
        })?;
        Ok(Box::new(NativeJournalReply {
            result,
            _slot: None,
        }))
    }

    fn chain_analysis_reads(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        operation: &str,
        args: &Value,
        local: &mut kassiber_wallets::analysis_index::reads::Local,
    ) -> Result<Value, AppError> {
        let (workspace, profile) = self.resolve_scope(workspace, profile)?;
        kassiber_wallets::analysis_index::reads::execute_local(
            self.conn()?,
            &workspace,
            &profile,
            operation,
            args,
            &mut kassiber_wallets::publication::wallet::SystemSources::default(),
            local,
        )
    }

    fn diagnostics(
        &self,
        args: &Value,
        version: &str,
        error: Option<&AppError>,
    ) -> Result<Value, AppError> {
        let mut args = args.clone();
        if self.db_passphrase.as_ref().is_some_and(|p| !p.is_empty()) {
            kassiber_envelope::set(&mut args, "db_passphrase_fd", Value::Null);
            kassiber_envelope::set(
                &mut args,
                "_db_passphrase_cached",
                kassiber_envelope::s("<present>"),
            );
        }
        let conn = self.conn()?;
        let state = kassiber_diagnostics::state::state(conn, self.backends.diagnostics_summary())?;
        Ok(kassiber_diagnostics::report::collect(
            &args,
            version,
            &kassiber_diagnostics::report::now()?,
            Some(state),
            kassiber_diagnostics::state::checks(Some(conn)),
            error,
        ))
    }

    fn backup_fingerprint(
        &self,
        project: Option<&str>,
    ) -> Result<kassiber_backup::desktop::Fingerprint, AppError> {
        kassiber_backup::desktop::fingerprint(self.conn()?, Path::new(&self.data_root), project)
    }
    fn backup_summary(&self) -> Result<Value, AppError> {
        kassiber_backup::desktop::summary(self.conn()?)
    }
    fn backup_export(
        &self,
        output: &Path,
        passphrase: &str,
        version: &str,
    ) -> Result<Value, AppError> {
        let resolved = context()?
            .resolve_for_bootstrap(&ResolveOptions {
                data_root: Some(PathBuf::from(&self.data_root)),
                ..Default::default()
            })
            .map_err(error)?;
        kassiber_backup::desktop::export(
            &resolved,
            output,
            self.db_passphrase.as_ref().map(Secret::expose),
            passphrase,
            version,
        )
    }
    fn close_for_restore(
        &mut self,
        expected: &kassiber_backup::desktop::Fingerprint,
        project: Option<&str>,
    ) -> Result<(), AppError> {
        kassiber_backup::desktop::check_close(
            self.conn()?,
            Path::new(&self.data_root),
            project,
            expected,
        )?;
        self.close()
    }
    fn rates(&mut self, command: &str, args: &Value) -> Result<Value, AppError> {
        use kassiber_envelope::{py_str, truthy};
        if command == "ui.rates.rebuild" {
            return self.rates_rebuild(args);
        }
        if command.starts_with("ui.") {
            return kassiber_rates::desktop_command(self.conn()?, command, args, &self.rates);
        }
        let consent = if ["sync", "rebuild"].contains(&command) {
            let source = kassiber_envelope::get(args, "source")
                .map(py_str)
                .unwrap_or_else(|| "coinbase-exchange".into())
                .trim()
                .to_lowercase();
            let pair = kassiber_envelope::get(args, "pair")
                .filter(|v| truthy(v))
                .map(py_str);
            Some(kassiber_rates::FetchConsent::explicit(
                self.conn()?,
                if command == "sync" {
                    kassiber_rates::FetchAction::Sync
                } else {
                    kassiber_rates::FetchAction::Rebuild
                },
                &source,
                pair.as_deref(),
            )?)
        } else {
            None
        };
        kassiber_rates::command(self.conn()?, command, args, &self.rates, consent.as_ref())
    }

    fn current_default_backend(&self) -> Option<String> {
        Some(self.backends.default_backend.clone())
    }
    fn service(&mut self, operation: &str, args: &Value) -> Result<Value, AppError> {
        if let Some(result) = self.workspace_read(operation, args) {
            return result;
        }
        if operation == "ui.secrets.migrate_credentials" {
            // `migrate_dotenv_credentials(conn, env_file)`, then reload and
            // merge the rewritten runtime configuration.
            let env_file = PathBuf::from(kassiber_envelope::py_str(
                kassiber_envelope::get(args, "env_file").unwrap_or(&Value::Null),
            ));
            let conn = self
                .book
                .as_ref()
                .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
                .connection();
            let result = kassiber_secrets::credentials::migrate(conn, &env_file)?;
            let mut config = backends::Config::load(&env_file)?;
            config.merge(conn)?;
            self.backends = config;
            return Ok(result);
        }
        if operation == "ui.egress.allowlist" {
            let conn = self
                .book
                .as_ref()
                .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
                .connection();
            return egress_inputs(&self.backends, Some(conn));
        }
        if operation == "ui.onboarding.complete" {
            let conn = self
                .book
                .as_ref()
                .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
                .connection();
            return self.backends.onboard(conn, args, &SetupSources);
        }
        if kassiber_chain::probes::KINDS.contains(&operation) {
            use kassiber_net::{Gate, Ledger, Offline};
            let args = serde_json::from_str::<serde_json::Value>(
                &kassiber_pyjson::dumps(args, &Default::default())
                    .map_err(|_| AppError::new("validation", "Invalid probe arguments"))?,
            )
            .map_err(|_| AppError::new("validation", "Invalid probe arguments"))?;
            let book_id = setting(self.conn()?, "database_instance_id")?
                .ok_or_else(|| AppError::new("validation", "Book identity is unavailable"))?;
            let saved = args
                .get("backend")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(|name| self.backends.resolve(self.conn()?, name))
                .transpose()?;
            let gate = Gate::new(
                || match Context::from_env()
                    .ok()
                    .and_then(|ctx| kassiber_setup::offline::preference_path(&ctx).ok())
                {
                    Some(path) if kassiber_setup::offline::enabled(&path) => Offline::Offline,
                    Some(_) => Offline::Online,
                    None => Offline::Unavailable,
                },
                Ledger::process(),
            );
            // This branch is reached only for one explicit registered daemon
            // request, after expected_scope and database unlock checks.
            let consent =
                kassiber_chain::probes::ProbeConsent::explicit(operation, &args, &book_id);
            let value = kassiber_chain::probes::Probes::new(gate, env!("CARGO_PKG_VERSION")).run(
                operation,
                &args,
                &book_id,
                saved,
                Some(&consent),
            )?;
            return Ok(kassiber_chain::probes::python_payload(operation, &value));
        }
        if operation == "ui.profiles.reset_data" {
            let attachments = context()?
                .state_root_for_data(Path::new(&self.data_root))
                .map_err(error)?
                .join("attachments");
            return kassiber_setup::profile_reset::execute(self.conn()?, args, &attachments);
        }
        if operation == "ui.backends.lightning.test" {
            // One explicit desktop request for one configured backend.
            return lightning::backend_test(
                self.conn()?,
                &self.backends,
                args,
                self.lightning_gate.clone(),
                env!("CARGO_PKG_VERSION"),
            );
        }
        if kassiber_wallets::freshness::settings::KINDS.contains(&operation) {
            return kassiber_wallets::freshness::settings::execute(
                self.conn()?,
                operation,
                args,
                &SetupSources,
            );
        }
        if operation.starts_with("backends.") || operation.starts_with("ui.backends.") {
            let conn = self
                .book
                .as_ref()
                .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
                .connection();
            if matches!(
                operation,
                "backends.update"
                    | "backends.delete"
                    | "backends.set-default"
                    | "backends.clear-default"
            ) {
                self.backends.seed_configuration(conn)?;
                self.backends.merge(conn)?;
            }
            return self.backends.execute(conn, operation, args, &SetupSources);
        }
        if matches!(operation, "networks.split" | "ui.networks.partition_export") {
            use kassiber_envelope::{dict, get, py_str};
            let desktop = operation.starts_with("ui.");
            let scope = if desktop {
                dict::<&str>([])
            } else {
                args.clone()
            };
            let (_, profile) = kassiber_setup::references::scope(self.conn()?, &scope)?;
            let mut payload = args.clone();
            if let Value::Dict(fields) = &mut payload {
                for (from, to) in [
                    ("declare_wallet", "declared_wallet_ids"),
                    ("wallet", "wallet_ids"),
                ] {
                    if let Some(v) = get(args, from) {
                        fields.push((to.into(), v.clone()));
                    }
                }
            }
            let password = if desktop {
                None
            } else {
                match get(args, "backup_passphrase_fd") {
                    Some(Value::Int(fd)) => Some(
                        kassiber_secrets::read_passphrase_from_fd(
                            i32::try_from(fd.clone()).map_err(|_| {
                                AppError::new(
                                    "invalid_passphrase",
                                    "passphrase descriptor must be non-negative",
                                )
                            })?,
                        )
                        .map_err(error)?,
                    ),
                    _ => None,
                }
            };
            let path = get(args, if desktop { "file_path" } else { "file" })
                .filter(|v| !matches!(v, Value::Null))
                .map(py_str)
                .unwrap_or_default();
            let destination = if path.trim().is_empty() {
                std::path::PathBuf::new()
            } else {
                std::path::absolute(context()?.expand_user(Path::new(&path)).map_err(error)?)
                    .map_err(|e| AppError::new("internal_error", e.to_string()))?
            };
            let recipient = get(args, "recipient")
                .filter(|v| !matches!(v, Value::Null))
                .map(py_str);
            let desktop_password = if desktop {
                get(args, "backup_passphrase")
                    .filter(|v| !matches!(v, Value::Null))
                    .map(|v| Secret::new(py_str(v)))
            } else {
                None
            };
            let attachments = context()?
                .state_root_for_data(Path::new(&self.data_root))
                .map_err(error)?
                .join("attachments");
            return kassiber_setup::networks::partition::export(
                self.conn()?,
                &get(&profile, "id").map(py_str).expect("profile id"),
                &payload,
                kassiber_setup::networks::partition::Export {
                    data_root: Path::new(&self.data_root),
                    attachments_root: &attachments,
                    destination: &destination,
                    recipient: recipient.as_deref(),
                    backup_passphrase: password
                        .as_ref()
                        .map(|p| p.expose())
                        .or_else(|| desktop_password.as_ref().map(Secret::expose)),
                    db_passphrase: self.db_passphrase.as_ref().map(Secret::expose),
                },
                &kassiber_setup::SystemSources,
            );
        }
        kassiber_setup::execute(self.conn()?, operation, args, &SetupSources)
    }
    fn documents(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        command: &kassiber_transactions::documents::Command,
    ) -> Result<Value, AppError> {
        kassiber_transactions::documents::validate_before_scope(command)?;
        let (workspace_id, profile_id) = self.resolve_scope(workspace, profile)?;
        let paths = context()?
            .resolve(&ResolveOptions {
                data_root: Some(PathBuf::from(&self.data_root)),
                ..Default::default()
            })
            .map_err(error)?;
        kassiber_transactions::documents::execute(
            self.conn()?,
            &kassiber_transactions::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            &paths.attachments_root,
            command,
        )
    }
    fn commercial(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        command: &kassiber_transactions::commercial::Command,
    ) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.resolve_scope(workspace, profile)?;
        kassiber_transactions::commercial::execute(
            self.conn()?,
            &kassiber_transactions::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            command,
        )
    }
    fn transaction_desktop(
        &mut self,
        projection: kassiber_transactions::desktop::Projection,
        args: Option<&Value>,
    ) -> Result<Value, AppError> {
        let context = self.context()?;
        kassiber_transactions::desktop::execute(
            self.conn()?,
            &kassiber_transactions::Scope {
                workspace_id: &context.workspace_id,
                profile_id: &context.profile_id,
            },
            projection,
            args,
            &|out_id, in_id| {
                kassiber_transfers::review_reading::reading(
                    self.conn()?,
                    &kassiber_transfers::Scope {
                        workspace_id: &context.workspace_id,
                        profile_id: &context.profile_id,
                    },
                    out_id,
                    in_id,
                )
            },
        )
    }

    fn report_read_sync(&mut self, force: bool) -> Result<Option<Value>, AppError> {
        use kassiber_envelope::{get, set, truthy};
        use kassiber_wallets::freshness::planning::{Plan, Request, Scope};
        use std::time::{Duration, Instant};
        const MIN_INTERVAL: Duration = Duration::from_secs(60);
        let context = self.context()?;
        if context.profile_id.is_empty() {
            return Ok(None);
        }
        let profile = context.profile_id.clone();
        let (mut sources, start_time) = sync_sources();
        let policy = kassiber_wallets::freshness::Queue::new(self.conn()?, sources.as_mut())
            .policy(&profile)?;
        if !force && !get(&policy, "report_read_sync").is_some_and(truthy) {
            return Ok(None);
        }
        let failure = |error: &AppError| {
            dict([
                ("ok", Value::Bool(false)),
                (
                    "reason",
                    s(if error.code.is_empty() {
                        "sync_failed"
                    } else {
                        error.code.as_str()
                    }),
                ),
                (
                    "message",
                    s(kassiber_wallets::freshness::ui::text(&error.message)),
                ),
            ])
        };
        let request = Request {
            scope: Scope::ActiveProfile,
            wallet: None,
            rates: None,
            journals: Some(false),
            force_full: false,
            auto_pair: false,
            wallets_sync: false,
        };
        let plan = {
            let backends = &self.backends;
            Plan::new(
                self.conn()?,
                &request,
                env!("CARGO_PKG_VERSION"),
                |conn, name| kassiber_wallets::sync::resolve_backend(conn, backends, name),
            )
        };
        // Another refresh holds the execution slot: the automatic attempt
        // answers busy without counting against the rate limit.
        if let Err(error) = &plan {
            if error.code == "project_operation_in_progress" {
                return Ok(Some(failure(error)));
            }
        }
        if !force {
            let now = Instant::now();
            if let Some((last, cached)) = self.report_read_attempts.get(&profile) {
                let elapsed = now.duration_since(*last);
                if elapsed < MIN_INTERVAL {
                    let mut payload = match cached {
                        None => dict([
                            ("ok", Value::Bool(true)),
                            ("status", s("skipped")),
                            ("reason", s("auto_sync_rate_limited")),
                        ]),
                        Some(cached) => {
                            let mut payload = cached.clone();
                            set(&mut payload, "status", s("cached"));
                            set(&mut payload, "reason", s("auto_sync_rate_limited"));
                            payload
                        }
                    };
                    let retry = (MIN_INTERVAL - elapsed).as_secs_f64().trunc() as i64;
                    set(
                        &mut payload,
                        "retry_after_seconds",
                        Value::Int(retry.into()),
                    );
                    return Ok(Some(payload));
                }
            }
            self.report_read_attempts
                .insert(profile.clone(), (now, None));
        }
        let outcome = (|| {
            if self.backends.default_backend.is_empty() && self.backends.kinds().is_empty() {
                // Python falls back to the explicit wallets-sync path here.
                return Err(AppError::new(
                    "not_implemented",
                    "Report-read refresh without configured backends is not implemented in the Rust daemon yet",
                ));
            }
            if let Err(error) = &plan {
                if error.code == "native_fallback_required" {
                    let conn = self.conn()?;
                    if let Some(idle) = kassiber_wallets::freshness::planning::report_read_idle(
                        conn,
                        &profile,
                        &policy,
                        force,
                        sources.as_mut(),
                    )? {
                        return Ok(idle);
                    }
                }
            }
            let plan = plan.map_err(|error| {
                if error.code == "native_fallback_required" {
                    AppError::new(
                        "not_implemented",
                        "Report-read refresh of these sources is not implemented in the Rust daemon yet",
                    )
                    .with_details(error.details.clone().unwrap_or(Value::Null))
                } else {
                    error
                }
            })?;
            let consent = if force {
                kassiber_wallets::freshness::native::Consent::explicit_trigger(&plan)
            } else {
                kassiber_wallets::freshness::native::Consent::report_read(&plan)
            };
            let gate = kassiber_net::Gate::new(
                || match Context::from_env()
                    .ok()
                    .and_then(|ctx| kassiber_setup::offline::preference_path(&ctx).ok())
                {
                    Some(path) if kassiber_setup::offline::enabled(&path) => {
                        kassiber_net::Offline::Offline
                    }
                    Some(_) => kassiber_net::Offline::Online,
                    None => kassiber_net::Offline::Unavailable,
                },
                kassiber_net::Ledger::default(),
            );
            let backends = &self.backends;
            let conn = self
                .book
                .as_mut()
                .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
                .connection_mut();
            plan.execute_report_read(
                conn,
                force,
                sources.as_mut(),
                &gate,
                &consent,
                &kassiber_wallets::scan::Cancellation::default(),
                Instant::now() + Duration::from_secs(900),
                start_time,
                env!("CARGO_PKG_VERSION"),
                |conn, name| kassiber_wallets::sync::resolve_backend(conn, backends, name),
            )
        })();
        let payload = match outcome {
            Ok(payload) => payload,
            Err(error) if error.code == "not_implemented" || error.code == "internal_error" => {
                return Err(error)
            }
            Err(error) => failure(&error),
        };
        if !force {
            if let Some(entry) = self.report_read_attempts.get_mut(&profile) {
                entry.1 = Some(payload.clone());
            }
        }
        Ok(Some(payload))
    }

    fn auto_process_journals(&mut self) -> Result<Option<Value>, AppError> {
        let context = self.context()?;
        if context.profile_id.is_empty() {
            return Ok(None);
        }
        let conn = self.conn()?;
        let stale: bool = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM transactions WHERE profile_id=p.id AND excluded=0) AS active, \
                 COALESCE(p.last_processed_at,'')<>'' AND COALESCE(p.last_processed_tx_count,0)=(SELECT COUNT(*) FROM transactions WHERE profile_id=p.id AND excluded=0) \
                 AND COALESCE(p.last_processed_input_version,0)=COALESCE(p.journal_input_version,0) AS current FROM profiles p WHERE p.id=?",
                [&context.profile_id],
                |row| Ok(row.get::<_, i64>(0)? > 0 && !row.get::<_, bool>(1)?),
            )
            .optional()
            .map_err(map_sqlite_error)
            .map_err(error)?
            .unwrap_or(false);
        if !stale {
            return Ok(None);
        }
        let reply = self
            .process_journals(None, None, false, &mut |_| {})
            .map_err(|failure| *failure.error)?;
        let payload = reply.payload().clone();
        reply.after_reply()?;
        Ok(Some(payload))
    }

    fn journal_events(&mut self, transaction: &str, limit: i64) -> Result<Value, AppError> {
        kassiber_journals::journal_events::for_transaction(self.conn()?, transaction, limit)
    }

    fn transaction_direction(&mut self, transaction: &str) -> Result<String, AppError> {
        let direction: Option<Option<String>> = self
            .conn()?
            .query_row(
                "SELECT direction FROM transactions WHERE id = ?",
                [transaction],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_sqlite_error)
            .map_err(error)?;
        Ok(direction.flatten().unwrap_or_default())
    }

    fn transaction_graph(&mut self, args: Option<&Value>) -> Result<Value, AppError> {
        kassiber_journals::transaction_graph::snapshot(self.conn()?, args)
    }

    fn evidence_summary(&mut self, args: &Value) -> Result<Value, AppError> {
        use kassiber_journals::evidence_summary;
        let options = evidence_summary::options(args)?;
        let paths = context()?
            .resolve(&ResolveOptions {
                data_root: Some(PathBuf::from(&self.data_root)),
                ..Default::default()
            })
            .map_err(error)?;
        std::fs::create_dir_all(&paths.attachments_root)
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        evidence_summary::summary(self.conn()?, &paths.attachments_root, &options)
    }

    fn report_blockers(&mut self, auto_sync: Option<&Value>) -> Result<Value, AppError> {
        use kassiber_workspace::readiness;
        let conn = self.conn()?;
        let (context, profile) = readiness::scope(conn, None)?;
        let health = readiness::health(conn, &context, profile.as_ref())?;
        let coverage = kassiber_rates::desktop_command(
            conn,
            "ui.rates.coverage",
            &dict([("limit", Value::Int(10.into()))]),
            &self.rates,
        )?;
        let snapshot = kassiber_journals::report_blockers::snapshot(conn, health, coverage)?;
        Ok(kassiber_journals::report_blockers::apply_sync_failure(
            snapshot, auto_sync,
        ))
    }

    fn transaction_dashboard(
        &mut self,
        args: &Value,
        candidates: &[Value],
    ) -> Result<Value, AppError> {
        let context = self.context()?;
        let tzpath = kassiber_reports::zone::tzpath(std::env::var("PYTHONTZPATH").ok().as_deref());
        kassiber_transactions::desktop::dashboard(
            self.conn()?,
            &kassiber_transactions::Scope {
                workspace_id: &context.workspace_id,
                profile_id: &context.profile_id,
            },
            args,
            candidates,
            &kassiber_transactions::desktop::Zones {
                display: &|key| kassiber_reports::zone::lookup(key, &tzpath),
                book: &|austrian| kassiber_reports::export_range::book_zone(austrian, &tzpath),
            },
        )
    }

    fn transactions(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        options: &kassiber_transactions::listing::Options,
    ) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.resolve_scope(workspace, profile)?;
        kassiber_transactions::listing::list(
            self.conn()?,
            &kassiber_transactions::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            options,
        )
    }

    fn transfers(&mut self, operation: &str, args: &Value) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.resolve_scope(
            kassiber_envelope::get(args, "workspace"),
            kassiber_envelope::get(args, "profile"),
        )?;
        kassiber_transfers::execute(
            self.conn()?,
            &kassiber_transfers::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            operation,
            args,
        )
    }
    fn review(
        &mut self,
        operation: &str,
        args: &Value,
        authored_source: &str,
    ) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.resolve_scope(
            kassiber_envelope::get(args, "workspace"),
            kassiber_envelope::get(args, "profile"),
        )?;
        #[cfg(feature = "test-harness")]
        let sources = journals::Sources;
        #[cfg(not(feature = "test-harness"))]
        let sources = kassiber_transfers::SystemSources;
        kassiber_journals::review_workflow::execute(
            self.conn()?,
            &kassiber_transfers::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            operation,
            args,
            authored_source,
            &sources,
        )
        .map_err(|error| match error {
            kassiber_journals::review_workflow::Error::Service(error) => error,
            kassiber_journals::review_workflow::Error::Build(refusal) => {
                kassiber_journals::service::application_error(refusal)
            }
        })
    }
    fn metadata(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        command: &kassiber_transactions::metadata::Command,
    ) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.resolve_scope(workspace, profile)?;
        if let kassiber_transactions::metadata::Command::Bip329(
            kassiber_transactions::metadata::Bip329::DesktopExport { wallets, mode },
        ) = command
        {
            let paths = context()?
                .resolve(&ResolveOptions {
                    data_root: Some(PathBuf::from(&self.data_root)),
                    ..Default::default()
                })
                .map_err(error)?;
            return kassiber_transactions::metadata::managed_bip329_export(
                self.conn()?,
                &kassiber_transactions::Scope {
                    workspace_id: &workspace_id,
                    profile_id: &profile_id,
                },
                &paths.exports_root,
                wallets,
                mode,
            );
        }
        kassiber_transactions::metadata::execute(
            self.conn()?,
            &kassiber_transactions::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            command,
        )
    }
    fn chain_analysis_watch_worker(
        &mut self,
    ) -> Result<Option<Box<dyn kassiber_runtime::store::WatchWorker>>, AppError> {
        let conn = self.conn()?;
        if !kassiber_wallets::analysis_ops::watch_worker_ready(conn) {
            return Ok(None);
        }
        let identity = setting(conn, "database_instance_id")?.unwrap_or_default();
        let root = self.data_root.clone();
        let passphrase = self.db_passphrase.as_ref().map(|p| p.expose().to_owned());
        let open: kassiber_wallets::analysis_ops::jobs::OpenWorker = Box::new(move || {
            StoredBook::open(
                Path::new(&root),
                OpenOptions {
                    passphrase: passphrase.as_deref(),
                    expected_database_identity: Some(&identity),
                    expected_project: None,
                },
            )
            .map(|book| Box::new(book) as Box<dyn kassiber_wallets::analysis_ops::jobs::WorkerBook>)
            .map_err(error)
        });
        Ok(Some(Box::new(WatchWorker(
            kassiber_wallets::analysis_ops::LocalWorker::new(
                open,
                Box::new(AnalysisSources::default()),
                Box::new(AnalysisRuntime),
            ),
        ))))
    }
    fn chain_analysis_datasets(
        &mut self,
        operation: &str,
        args: &Value,
        runtime: &std::sync::Arc<kassiber_wallets::analysis_index::runtime::Runtime>,
    ) -> Result<Value, AppError> {
        use kassiber_wallets::analysis_ops::{jobs, Scope};
        let (workspace_id, profile_id) = self.resolve_scope(None, None)?;
        let scope = Scope {
            workspace_id: &workspace_id,
            profile_id: &profile_id,
        };
        let conn = self.conn()?;
        if operation == "datasets.import" {
            return jobs::import_token(
                conn,
                &scope,
                runtime,
                args,
                &mut AnalysisSources::default(),
            );
        }
        // The worker opens its own connection, fenced to this book's identity.
        let identity = setting(conn, "database_instance_id")?.unwrap_or_default();
        let root = self.data_root.clone();
        let passphrase = self.db_passphrase.as_ref().map(|p| p.expose().to_owned());
        let open: jobs::OpenWorker = Box::new(move || {
            StoredBook::open(
                Path::new(&root),
                OpenOptions {
                    passphrase: passphrase.as_deref(),
                    expected_database_identity: Some(&identity),
                    expected_project: None,
                },
            )
            .map(|book| Box::new(book) as Box<dyn jobs::WorkerBook>)
            .map_err(error)
        });
        jobs::start(
            conn,
            &scope,
            runtime,
            operation,
            args,
            open,
            Box::new(AnalysisSources::default()),
        )
    }
    fn chain_analysis_import(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        args: &Value,
        file: Box<dyn std::io::BufRead + Send>,
    ) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.resolve_scope(workspace, profile)?;
        kassiber_wallets::analysis_ops::import_dataset(
            self.conn()?,
            &kassiber_wallets::analysis_ops::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            args,
            file,
            &mut AnalysisSources::default(),
        )
    }
    fn chain_analysis_ops(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        operation: &str,
        args: &Value,
    ) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.resolve_scope(workspace, profile)?;
        let conn = self.conn()?;
        let result = kassiber_wallets::analysis_ops::execute_with_sources(
            conn,
            &kassiber_wallets::analysis_ops::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            operation,
            args,
            &mut AnalysisSources::default(),
            &mut AnalysisRuntime,
        )?;
        if kassiber_wallets::analysis_ops::is_write(operation) && !conn.is_autocommit() {
            conn.execute_batch("COMMIT")
                .map_err(kassiber_wallets::database_error)?;
        }
        Ok(result)
    }
    fn attachments(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        command: &kassiber_transactions::attachments::Command,
    ) -> Result<Value, AppError> {
        let paths = context()?
            .resolve(&ResolveOptions {
                data_root: Some(PathBuf::from(&self.data_root)),
                ..Default::default()
            })
            .map_err(error)?;
        let scope = if matches!(
            command,
            kassiber_transactions::attachments::Command::Gc { .. }
        ) {
            None
        } else {
            Some(self.resolve_scope(workspace, profile)?)
        };
        let scope = scope
            .as_ref()
            .map(|(workspace_id, profile_id)| kassiber_transactions::Scope {
                workspace_id,
                profile_id,
            });
        kassiber_transactions::attachments::execute(
            self.conn()?,
            scope.as_ref(),
            &paths.attachments_root,
            command,
        )
    }
    fn loans(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        command: &kassiber_transactions::loans::Command,
    ) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.resolve_scope(workspace, profile)?;
        kassiber_transactions::loans::execute(
            self.conn()?,
            &kassiber_transactions::Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
            },
            command,
        )
    }
    fn wallets_snapshot(&self, default_backend: Option<&str>) -> Result<Value, AppError> {
        kassiber_wallets::snapshot(self.conn()?, default_backend, &self.backend_kinds)
    }
    fn wallet_utxos(
        &self,
        args: Option<&Value>,
        default_backend: Option<&str>,
    ) -> Result<Value, AppError> {
        kassiber_wallets::utxos::snapshot(self.conn()?, args, default_backend, &|name| {
            self.backends.configured(name).cloned()
        })
    }
    fn reports(&mut self, command: &str, args: &Value) -> Result<Value, AppError> {
        if lightning::COMMANDS.contains(&command) {
            return lightning::execute(
                self.conn()?,
                &self.backends,
                command,
                args,
                self.lightning_gate.clone(),
                env!("CARGO_PKG_VERSION"),
            );
        }
        if kassiber_reports::export_request::handles(command) {
            let paths = context()?
                .resolve(&ResolveOptions {
                    data_root: Some(PathBuf::from(&self.data_root)),
                    ..Default::default()
                })
                .map_err(error)?;
            let mut args = args.clone();
            if command == "reports.cross-check" {
                if let Some(raw) = kassiber_envelope::get(&args, "xlsx")
                    .filter(|v| kassiber_envelope::truthy(v))
                    .map(kassiber_envelope::py_str)
                {
                    let path = context()?.expand_user(Path::new(&raw)).map_err(error)?;
                    if let Value::Dict(fields) = &mut args {
                        for (k, v) in fields {
                            if *k == "xlsx".into() {
                                *v = kassiber_envelope::s(path.to_string_lossy().into_owned());
                            }
                        }
                    }
                }
            }
            if !command.starts_with("ui.") {
                let key = if command == "reports.export-austrian-e1kv-csv" {
                    "dir"
                } else {
                    "file"
                };
                let raw = kassiber_envelope::get(&args, key)
                    .map(kassiber_envelope::py_str)
                    .unwrap_or_default();
                let path = context()?.expand_user(Path::new(&raw)).map_err(error)?;
                if let Value::Dict(fields) = &mut args {
                    for (k, v) in fields {
                        if *k == key.into() {
                            *v = kassiber_envelope::s(path.to_string_lossy().into_owned());
                        }
                    }
                }
            }
            return kassiber_reports::export_request::execute(
                self.conn()?,
                command,
                &args,
                &paths.exports_root,
                &ReportSources,
            );
        }
        if command == "ui.exports.preview" {
            let conn = self.conn()?;
            return kassiber_reports::export_preview::preview(
                conn,
                args,
                &ReportSources,
                &|profile, refs| {
                    let text = |key| {
                        kassiber_envelope::get(profile, key)
                            .map(kassiber_envelope::py_str)
                            .unwrap_or_default()
                    };
                    let (workspace_id, profile_id) = (text("workspace_id"), text("id"));
                    kassiber_transactions::metadata::bip329_export_records(
                        conn,
                        &kassiber_transactions::Scope {
                            workspace_id: &workspace_id,
                            profile_id: &profile_id,
                        },
                        refs,
                        "all",
                    )
                },
            );
        }
        kassiber_reports::execute_with_sources(self.conn()?, command, args, &ReportSources)
    }
    fn derive_wallet(&mut self, request: kassiber_wallets::Derive) -> Result<Value, AppError> {
        kassiber_wallets::derive_wallet(self.conn()?, request)
    }
    fn document_target(
        &self,
        profile_id: &str,
        wallet: Option<&str>,
    ) -> Result<DocumentTarget, AppError> {
        #[cfg(feature = "test-harness")]
        if let Ok(time) = std::env::var("KASSIBER_RUST_TEST_DOCUMENT_TIME") {
            self.conn()?
                .create_scalar_function(
                    "strftime",
                    -1,
                    kassiber_store::rusqlite::functions::FunctionFlags::SQLITE_UTF8,
                    move |_| Ok(time.clone()),
                )
                .map_err(map_sqlite_error)
                .map_err(error)?;
        }
        #[cfg(feature = "test-harness")]
        if let Ok(ids) = std::env::var("KASSIBER_RUST_TEST_DOCUMENT_ATTACHMENT_IDS") {
            let ids = Mutex::new(
                ids.split(',')
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            self.conn()?
                .create_scalar_function(
                    "randomblob",
                    1,
                    kassiber_store::rusqlite::functions::FunctionFlags::SQLITE_UTF8,
                    move |_| {
                        let id = ids
                            .lock()
                            .unwrap()
                            .next()
                            .expect("document attachment entropy")
                            .replace('-', "");
                        Ok((0..32)
                            .step_by(2)
                            .map(|i| u8::from_str_radix(&id[i..i + 2], 16).unwrap())
                            .collect::<Vec<_>>())
                    },
                )
                .map_err(map_sqlite_error)
                .map_err(error)?;
        }
        let fiat_currency = self
            .conn()?
            .query_row(
                "SELECT fiat_currency FROM profiles WHERE id=?",
                [profile_id],
                |r| r.get(0),
            )
            .map_err(map_sqlite_error)
            .map_err(error)?;
        let wallet_id = wallet
            .map(|wallet| kassiber_wallets::resolve(self.conn()?, profile_id, wallet).map(|w| w.id))
            .transpose()?;
        Ok(DocumentTarget {
            fiat_currency,
            wallet_id,
        })
    }
    fn preview_document(
        &mut self,
        request: kassiber_wallets::file_import::document::DocumentPreview,
    ) -> Result<Value, AppError> {
        use kassiber_wallets::file_import::document::{DocumentConsent, Providers};
        let conn = self.conn()?;
        let consent = DocumentConsent::explicit(conn, &request)?;
        Providers::system().preview(conn, &request, Some(consent))
    }
    fn import_document(&mut self, request: ReviewedDocument<'_>) -> Result<Value, AppError> {
        use kassiber_wallets::file_import::document::{import_draft, ImportDraft};
        #[cfg(feature = "test-harness")]
        let identity = std::env::var("KASSIBER_RUST_TEST_IMPORT_IDS")
            .ok()
            .and_then(|ids| {
                Some(kassiber_wallets::file_import::Identity {
                    ids: ids.split(',').map(str::to_owned).collect(),
                    timestamp: std::env::var("KASSIBER_RUST_TEST_IMPORT_TIME").ok()?,
                })
            });
        #[cfg(not(feature = "test-harness"))]
        let identity = None;
        import_draft(
            self.conn()?,
            ImportDraft {
                source_file: request.source_file,
                data_root: Path::new(&self.data_root),
                profile_id: request.profile_id,
                wallet: request.wallet,
                rows: request.rows,
                include_quarantined: request.include_quarantined,
                selected_row_ids: Some(request.selected_row_ids),
                expected_source_sha256: Some(request.source_sha256),
                confidence_threshold: request.confidence_threshold,
                attach_evidence: true,
            },
            identity.as_ref(),
        )
    }
    fn import_wallet_file(
        &mut self,
        request: kassiber_wallets::file_import::Request,
    ) -> Result<Value, AppError> {
        #[cfg(feature = "test-harness")]
        let identity = std::env::var("KASSIBER_RUST_TEST_IMPORT_IDS")
            .ok()
            .and_then(|ids| {
                Some(kassiber_wallets::file_import::Identity {
                    ids: ids.split(',').map(str::to_owned).collect(),
                    timestamp: std::env::var("KASSIBER_RUST_TEST_IMPORT_TIME").ok()?,
                })
            });
        #[cfg(not(feature = "test-harness"))]
        let identity = None;
        kassiber_wallets::file_import::execute(self.conn()?, request, identity.as_ref())
    }
    fn import_ledger_file(
        &mut self,
        request: kassiber_wallets::file_import::Request,
        column_map: Option<kassiber_wallets::file_import::ledger::ColumnMap>,
    ) -> Result<Value, AppError> {
        #[cfg(feature = "test-harness")]
        let identity = std::env::var("KASSIBER_RUST_TEST_IMPORT_IDS")
            .ok()
            .and_then(|ids| {
                Some(kassiber_wallets::file_import::Identity {
                    ids: ids.split(',').map(str::to_owned).collect(),
                    timestamp: std::env::var("KASSIBER_RUST_TEST_IMPORT_TIME").ok()?,
                })
            });
        #[cfg(not(feature = "test-harness"))]
        let identity = None;
        kassiber_wallets::file_import::execute_with_ledger_map(
            self.conn()?,
            request,
            column_map.as_ref(),
            identity.as_ref(),
        )
    }
    fn import_samourai(
        &mut self,
        request: kassiber_wallets::file_import::samourai::Request,
    ) -> Result<Value, AppError> {
        #[cfg(feature = "test-harness")]
        let identity = std::env::var("KASSIBER_RUST_TEST_IMPORT_IDS")
            .ok()
            .and_then(|ids| {
                Some(kassiber_wallets::file_import::Identity {
                    ids: ids.split(',').map(str::to_owned).collect(),
                    timestamp: std::env::var("KASSIBER_RUST_TEST_IMPORT_TIME").ok()?,
                })
            });
        #[cfg(not(feature = "test-harness"))]
        let identity = None;
        kassiber_wallets::file_import::samourai::execute(self.conn()?, request, identity.as_ref())
    }
    fn wallets(&mut self, operation: kassiber_wallets::Operation) -> Result<Value, AppError> {
        #[cfg(feature = "test-harness")]
        if let (Ok(id), Ok(created_at)) = (
            std::env::var("KASSIBER_RUST_TEST_WALLET_ID"),
            std::env::var("KASSIBER_RUST_TEST_WALLET_TIME"),
        ) {
            return kassiber_wallets::execute_with_creation_identity(
                self.conn()?,
                operation,
                Some(&kassiber_wallets::CreationIdentity { id, created_at }),
            );
        }
        kassiber_wallets::execute(self.conn()?, operation)
    }
    fn freshness_plan(
        &self,
        request: &kassiber_wallets::freshness::planning::Request,
    ) -> Result<kassiber_wallets::freshness::planning::Plan, AppError> {
        kassiber_wallets::freshness::planning::Plan::new(
            self.conn()?,
            request,
            env!("CARGO_PKG_VERSION"),
            |conn, name| kassiber_wallets::sync::resolve_backend(conn, &self.backends, name),
        )
    }
    fn wallet_sync_supported(
        &self,
        profile_id: &str,
        wallet: Option<&str>,
        all: bool,
        force_full: bool,
    ) -> Result<bool, AppError> {
        use kassiber_wallets::sync::{resolve_backend, Request, Sync};
        match Sync::new(
            self.conn()?,
            &Request {
                profile_id,
                wallet,
                all,
                force_full,
                version: env!("CARGO_PKG_VERSION"),
            },
            |conn, name| resolve_backend(conn, &self.backends, name),
        ) {
            Ok(_) => Ok(true),
            Err(error) if error.code == "native_fallback_required" => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn freshness_execute(
        &mut self,
        plan: kassiber_wallets::freshness::planning::Plan,
        options: &kassiber_wallets::freshness::execution::Options,
        consent: Option<&kassiber_wallets::freshness::native::Consent>,
        cancel: &kassiber_wallets::scan::Cancellation,
        progress: &mut dyn FnMut(&Value) -> Result<(), AppError>,
    ) -> Result<Value, AppError> {
        use kassiber_net::{Gate, Ledger, Offline};
        use std::time::{Duration, Instant};
        let gate = Gate::new(
            || match Context::from_env()
                .ok()
                .and_then(|ctx| kassiber_setup::offline::preference_path(&ctx).ok())
            {
                Some(path) if kassiber_setup::offline::enabled(&path) => Offline::Offline,
                Some(_) => Offline::Online,
                None => Offline::Unavailable,
            },
            Ledger::process(),
        );
        let backends = &self.backends;
        let conn = self
            .book
            .as_mut()
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
            .connection_mut();
        let (mut sources, start_time) = sync_sources();
        let mut journal_handler = journals::Refresh {
            cancel,
            consent,
            context: kassiber_setup::context::snapshot(conn)?,
            deadline: Instant::now() + Duration::from_secs(900),
            #[cfg(feature = "test-harness")]
            test_passphrase: self.db_passphrase.as_ref().map(Secret::expose),
        };
        plan.execute_native(
            conn,
            options,
            sources.as_mut(),
            progress,
            &gate,
            consent,
            cancel,
            Instant::now() + Duration::from_secs(900),
            start_time,
            env!("CARGO_PKG_VERSION"),
            |conn, name| kassiber_wallets::sync::resolve_backend(conn, backends, name),
            Some(&mut journal_handler),
        )
    }
    fn freshness_control(&mut self, action: &str, args: &Value) -> Result<Value, AppError> {
        let (mut sources, _) = sync_sources();
        kassiber_wallets::freshness::control(self.conn()?, sources.as_mut(), action, args)
    }
    fn graph_followup(
        &mut self,
        grant: kassiber_wallets::freshness::graph::Grant,
        stop: &kassiber_wallets::scan::Cancellation,
    ) -> Result<Option<Value>, AppError> {
        use kassiber_net::{Gate, Ledger, Offline};
        let gate = Gate::new(
            || match Context::from_env()
                .ok()
                .and_then(|ctx| kassiber_setup::offline::preference_path(&ctx).ok())
            {
                Some(path) if kassiber_setup::offline::enabled(&path) => Offline::Offline,
                Some(_) => Offline::Online,
                None => Offline::Unavailable,
            },
            Ledger::process(),
        );
        let backends = &self.backends;
        let conn = self
            .book
            .as_mut()
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
            .connection_mut();
        let mut sources = graph_sources();
        kassiber_wallets::freshness::graph::followup(
            conn,
            grant,
            &gate,
            stop,
            sources.as_mut(),
            env!("CARGO_PKG_VERSION"),
            |conn, name| kassiber_wallets::sync::resolve_backend(conn, backends, name),
        )
    }
    fn wallet_sync(
        &mut self,
        profile_id: &str,
        wallet: Option<&str>,
        all: bool,
        force_full: bool,
    ) -> Result<Value, AppError> {
        use kassiber_net::{Gate, Ledger, Offline};
        use kassiber_wallets::{
            scan::Cancellation,
            sync::{resolve_backend, Request, Sync, SyncConsent},
        };
        use std::time::{Duration, Instant};
        let gate = Gate::new(
            || match Context::from_env()
                .ok()
                .and_then(|ctx| kassiber_setup::offline::preference_path(&ctx).ok())
            {
                Some(path) if kassiber_setup::offline::enabled(&path) => Offline::Offline,
                Some(_) => Offline::Online,
                None => Offline::Unavailable,
            },
            Ledger::process(),
        );
        let backends = &self.backends;
        let conn = self
            .book
            .as_mut()
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
            .connection_mut();
        let action = Sync::new(
            conn,
            &Request {
                profile_id,
                wallet,
                all,
                force_full,
                version: env!("CARGO_PKG_VERSION"),
            },
            |conn, name| resolve_backend(conn, backends, name),
        )?;
        let consent = SyncConsent::explicit_trigger(&action);
        let (mut sources, start_time) = sync_sources();
        let result = action.run(
            conn,
            &gate,
            Some(&consent),
            &Cancellation::default(),
            Instant::now() + Duration::from_secs(900),
            start_time,
            sources.as_mut(),
            |conn, name| resolve_backend(conn, backends, name),
        )?;
        Ok(Value::List(result))
    }
    fn import_runs(&mut self, operation: kassiber_imports::Operation) -> Result<Value, AppError> {
        kassiber_imports::execute(self.conn()?, operation)
    }
    fn is_initialized(&self) -> Result<bool, AppError> {
        if setting(self.conn()?, "app_version")?.is_some_and(|v| !v.is_empty()) {
            return Ok(true);
        }
        self.conn()?
            .query_row("SELECT EXISTS(SELECT 1 FROM workspaces)", [], |r| r.get(0))
            .map_err(map_sqlite_error)
            .map_err(error)
    }
    fn context(&self) -> Result<BookContext, AppError> {
        let conn = self.conn()?;
        let workspace = conn.query_row("SELECT id,label FROM workspaces WHERE id=(SELECT value FROM settings WHERE key='context_workspace')", [], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?))).optional().map_err(map_sqlite_error).map_err(error)?;
        let profile = conn.query_row("SELECT id,label,workspace_id FROM profiles WHERE id=(SELECT value FROM settings WHERE key='context_profile')", [], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(map_sqlite_error).map_err(error)?;
        let mut result = BookContext::default();
        if let Some((id, label)) = workspace {
            result.workspace_id = id;
            result.workspace_label = label;
        }
        if let Some((id, label, wid)) = profile {
            if result.workspace_id.is_empty() || result.workspace_id == wid {
                result.profile_id = id;
                result.profile_label = label;
            }
        }
        Ok(result)
    }
    fn status_facts(&self) -> Result<StatusFacts, AppError> {
        // `resolve_runtime_paths` never requires the layout: a project the
        // desktop just created has no settings file yet.
        let p = context()?
            .resolve_for_bootstrap(&ResolveOptions {
                data_root: Some(PathBuf::from(&self.data_root)),
                ..Default::default()
            })
            .map_err(error)?;
        let counts: Vec<i64> = [
            "workspaces",
            "profiles",
            "accounts",
            "wallets",
            "transactions",
            "journal_entries",
            "journal_quarantines",
        ]
        .iter()
        .map(|table| {
            self.conn()?
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .map_err(map_sqlite_error)
                .map_err(error)
        })
        .collect::<Result<_, _>>()?;
        Ok(StatusFacts {
            paths: paths(&p),
            database_encrypted: self.book.as_ref().is_some_and(StoredBook::encrypted),
            project_id: p.project_id.clone(),
            project_name: p.project_name.clone(),
            project_root: p.project_root.as_deref().map(path),
            context: self.context()?,
            counts: StatusCounts {
                workspaces: counts[0],
                profiles: counts[1],
                accounts: counts[2],
                wallets: counts[3],
                transactions: counts[4],
                journal_entries: counts[5],
                quarantines: counts[6],
            },
        })
    }
    fn resolve_scope(
        &self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
    ) -> Result<(String, String), AppError> {
        use kassiber_setup::references::{resolve, Entity};
        let w = resolve(self.conn()?, Entity::Workspace, workspace, None)?;
        let wid =
            kassiber_envelope::py_str(kassiber_envelope::get(&w, "id").expect("workspace id"));
        let p = resolve(self.conn()?, Entity::Profile, profile, Some(&wid))?;
        Ok((
            wid,
            kassiber_envelope::py_str(kassiber_envelope::get(&p, "id").expect("profile id")),
        ))
    }
    fn rollback(&mut self) -> Result<(), AppError> {
        if !self.conn()?.is_autocommit() {
            self.conn()?
                .execute_batch("ROLLBACK")
                .map_err(map_sqlite_error)
                .map_err(error)?;
        }
        Ok(())
    }
    fn close(&mut self) -> Result<(), AppError> {
        if let Some(book) = self.book.take() {
            if let Err(failure) = book.close() {
                self.book = Some(*failure.resource);
                return Err(error(failure.error));
            }
        }
        self.db_passphrase = None;
        Ok(())
    }
}

/// `_egress_allowlist(ctx)` inputs: runtime backends as `[name, url,
/// source]` in Python's dict order, and, for an open book, AI providers as
/// `[name, base_url, kind]` (an unreadable table reads as none).
fn egress_inputs(config: &backends::Config, conn: Option<&Connection>) -> Result<Value, AppError> {
    let triple = |(a, b, c): (String, Value, Value)| Value::List(vec![s(a), b, c]);
    let backends = config
        .egress_backends(conn)?
        .into_iter()
        .map(triple)
        .collect();
    let mut result = vec![("backends", Value::List(backends))];
    if let Some(conn) = conn {
        let providers = conn
            .prepare("SELECT name, base_url, kind FROM ai_providers ORDER BY name")
            .and_then(|mut statement| {
                let rows = statement.query_map([], |r| {
                    let text = |i| -> kassiber_store::rusqlite::Result<Value> {
                        Ok(r.get::<_, Option<String>>(i)?.map_or(Value::Null, s))
                    };
                    Ok(Value::List(vec![text(0)?, text(1)?, text(2)?]))
                })?;
                rows.collect::<Result<Vec<_>, _>>()
            })
            .unwrap_or_default();
        result.push(("ai_providers", Value::List(providers)));
    }
    Ok(dict(result))
}

#[cfg(test)]
mod project_clock_tests {
    /// `now_iso()` writes whole seconds with a `Z` suffix into the catalog and
    /// migration markers, never SQLite's fractional `+00:00` spelling.
    #[test]
    fn project_clock_matches_python_now_iso() {
        if std::env::var_os("KASSIBER_RUST_TEST_PROJECT_TIMESTAMP").is_some() {
            return;
        }
        let stamp = super::timestamp().unwrap();
        let bytes = stamp.as_bytes();
        assert_eq!(bytes.len(), 20, "{stamp}");
        for (index, byte) in bytes.iter().enumerate() {
            let expected = match index {
                4 | 7 => Some(b'-'),
                10 => Some(b'T'),
                13 | 16 => Some(b':'),
                19 => Some(b'Z'),
                _ => None,
            };
            match expected {
                Some(expected) => assert_eq!(*byte, expected, "{stamp}"),
                None => assert!(byte.is_ascii_digit(), "{stamp}"),
            }
        }
    }
}
