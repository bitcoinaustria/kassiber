//! The shell adapter. Paths, authentication, migrations and exclusion live in
//! the storage crates; this module projects their results onto R1's transport
//! contract without opening a network client.
mod backends;

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

#[derive(Default)]
pub struct RealStore {
    auth: Mutex<()>,
}

fn error(error: kassiber_paths::Error) -> AppError {
    let details = kassiber_pytext::loads(&error.details.to_string()).ok();
    AppError {
        code: error.code.to_owned(),
        message: error.message.to_owned(),
        hint: error.hint.map(str::to_owned),
        details,
        retryable: error.retryable,
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
fn timestamp() -> Result<String, AppError> {
    Connection::open_in_memory()
        .map_err(map_sqlite_error)
        .map_err(error)?
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%f','now') || '000+00:00'",
            [],
            |r| r.get(0),
        )
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
    fn local_service(&self, operation: &str, args: &Value) -> Result<Value, AppError> {
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
                &kassiber_setup::Sources::now(
                    &kassiber_setup::SystemSources,
                    &Connection::open_in_memory()
                        .map_err(map_sqlite_error)
                        .map_err(error)?,
                )?,
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
        let resolved = context
            .resolve_for_bootstrap(&options(selection))
            .map_err(error)?;
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
        let resolved = match context.resolve_for_bootstrap(&options(selection)) {
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
                context
                    .resolve_for_bootstrap(&options(selection))
                    .map_err(error)?
            }
            Err(e) => return Err(error(e)),
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
        config.default_backend = Some(backends.default_backend.clone());
        Ok(Box::new(RealBook {
            book: Some(book),
            data_root: root.to_owned(),
            rates: kassiber_rates::Providers::system(env!("CARGO_PKG_VERSION")),
            db_passphrase: retained_key.into_inner(),
            backends,
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
        let context = context()?;
        let file = context
            .default_state_root()
            .map_err(error)?
            .join("config/projects.json");
        let mut catalog = kassiber_paths::load_catalog(&file, &context).map_err(error)?;
        let entry = catalog
            .projects
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| AppError::new("project_not_found", "project is not registered"))?;
        entry.last_opened_at = Some(timestamp()?);
        entry.encrypted = self.database_is_encrypted(root)?;
        if select {
            catalog.selected_project_id = Some(id.to_owned());
        }
        kassiber_store::layout::write_catalog(&file, &catalog).map_err(error)
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
    rates: kassiber_rates::Providers,
    book: Option<StoredBook>,
    data_root: String,
    db_passphrase: Option<Secret>,
    backends: backends::Config,
}
impl RealBook {
    fn conn(&self) -> Result<&Connection, AppError> {
        self.book
            .as_ref()
            .map(StoredBook::connection)
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))
    }
}

impl Book for RealBook {
    fn rates(&mut self, command: &str, args: &Value) -> Result<Value, AppError> {
        use kassiber_envelope::{py_str, truthy};
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
        if operation == "ui.profiles.reset_data" {
            let attachments = context()?
                .state_root_for_data(Path::new(&self.data_root))
                .map_err(error)?
                .join("attachments");
            return kassiber_setup::profile_reset::execute(self.conn()?, args, &attachments);
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
    fn derive_wallet(&mut self, request: kassiber_wallets::Derive) -> Result<Value, AppError> {
        kassiber_wallets::derive_wallet(self.conn()?, request)
    }
    fn wallets(&mut self, operation: kassiber_wallets::Operation) -> Result<Value, AppError> {
        kassiber_wallets::execute(self.conn()?, operation)
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
        let p = context()?
            .resolve(&ResolveOptions {
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
