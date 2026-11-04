//! The narrow store interface the CLI and daemon shell need.
//!
//! Data roots, projects, SQLCipher, unlock, and ownership locks belong to
//! the storage crates (plan 21, R1/R3). The shell only needs the operations
//! below, in the shapes Python's `core/runtime.py`, `db.py`, `projects.py`,
//! and `operator/project.py` expose them to `cli/main.py` and `daemon.py`.
//! [`StubStore`] fails every operation closed for negative seam tests;
//! [`crate::fake::FakeStore`] (feature `fake`) drives the shell's
//! tests.

use std::fmt;

use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;

/// A passphrase or other secret, zeroed when dropped and never printed.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(text: String) -> Self {
        Secret(text.into_bytes())
    }

    /// The secret text. Callers must not copy it into logs or envelopes.
    pub fn expose(&self) -> &str {
        // Built from a String, so always UTF-8.
        std::str::from_utf8(&self.0).unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for byte in self.0.iter_mut() {
            *byte = 0;
        }
        std::hint::black_box(&self.0);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// The project the command line selected: `--project`, `--data-root`, and
/// `--env-file`, unresolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub project: Option<String>,
    pub data_root: Option<String>,
    pub env_file: Option<String>,
}

/// `effective_unlock_mode(data_root)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlockMode {
    Manual,
    Brokered,
    Unattended,
}

/// Python's `RuntimePaths`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimePaths {
    pub data_root: String,
    pub env_file: String,
    pub state_root: String,
    pub config_root: String,
    pub settings_file: String,
    pub exports_root: String,
    pub attachments_root: String,
    pub database: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub project_root: Option<String>,
}

/// The parts of `load_runtime_config` (merged with the book's backends)
/// that status reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub default_backend: Option<String>,
    pub env_file: String,
}

/// `bootstrap_runtime(args, needs_db=False)`: resolved paths, the project
/// layout ensured, and the runtime configuration loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bootstrap {
    pub paths: RuntimePaths,
    pub runtime_config: RuntimeConfig,
    /// Whether `--project` or `--data-root` named the project, so opening
    /// it must not change the selected project.
    pub selection_explicit: bool,
}

/// How a caller unlocks a book.
#[derive(Debug)]
pub enum Unlock<'a> {
    /// The CLI's chain (`bootstrap_runtime` with `needs_db=True`): the
    /// `--db-passphrase-fd` pipe, read once and closed; then remembered
    /// unlock; then a TTY prompt when `allow_prompt`; otherwise
    /// `interaction_required` with `details.reason = database_passphrase`.
    Cli {
        passphrase_fd: Option<i64>,
        allow_prompt: bool,
        expected_database_identity: Option<&'a str>,
        /// Explicit `init` persists bootstrap backends and the app version.
        initialize_version: Option<&'a str>,
    },
    /// The daemon's `open_db`: this passphrase or none. An encrypted book
    /// without one fails `passphrase_required`; a wrong one `unlock_failed`.
    Daemon {
        passphrase: Option<&'a Secret>,
        require_existing_schema: bool,
        /// File identity reserved by the daemon before opening or migrating.
        expected_project_identity: &'a str,
    },
}

/// Desktop project catalog operations (`kassiber.projects`). None opens a
/// book; each answers the affected entry's `_project_payload`, or a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectCatalog<'a> {
    /// `list_projects()`, refreshing and persisting encryption metadata.
    List,
    /// `get_project(id)`.
    Get(&'a str),
    /// `create_project(name, project_id=..., select=...,
    /// replace_existing=False, allow_existing_database=False)`.
    Create {
        name: &'a str,
        project_id: Option<&'a str>,
        select: bool,
    },
    /// `mark_project_opened(id, data_root, select)` when `stamp`, otherwise
    /// `refresh_project_metadata(id, data_root)`.
    Opened {
        project_id: &'a str,
        data_root: &'a str,
        stamp: bool,
        select: bool,
    },
    /// `set_selected_project(id, last_opened_at=now_iso())`.
    Select(&'a str),
}

/// `current_context_snapshot(conn)`: the active workspace and profile,
/// each an empty string when none is selected (as Python returns them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BookContext {
    pub workspace_id: String,
    pub workspace_label: String,
    pub profile_id: String,
    pub profile_label: String,
}

/// The row counts `build_status_payload` reports, in its order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub workspaces: i64,
    pub profiles: i64,
    pub accounts: i64,
    pub wallets: i64,
    pub transactions: i64,
    pub journal_entries: i64,
    pub quarantines: i64,
}

/// What `build_status_payload(conn, data_root)` reads, before the payload
/// is assembled in Python's key order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusFacts {
    /// `resolve_runtime_paths(data_root=...)` for the open book.
    pub paths: RuntimePaths,
    /// The live file is SQLCipher, not plaintext SQLite.
    pub database_encrypted: bool,
    /// Project metadata of the data root, falling back to the paths'.
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub project_root: Option<String>,
    pub context: BookContext,
    pub counts: StatusCounts,
}

/// A committed journal response retains its deferred checkpoint until the
/// transport has published and flushed the terminal envelope.
pub trait JournalReply {
    fn payload(&self) -> &Value;
    fn after_reply(self: Box<Self>) -> Result<(), AppError>;
}

pub struct JournalFailure {
    pub error: Box<AppError>,
    pub replay_shape: Option<String>,
}
impl From<AppError> for JournalFailure {
    fn from(error: AppError) -> Self {
        Self {
            error: Box::new(error),
            replay_shape: None,
        }
    }
}

pub struct DocumentTarget {
    pub fiat_currency: String,
    pub wallet_id: Option<String>,
}

pub struct ReviewedDocument<'a> {
    pub source_file: &'a str,
    pub profile_id: &'a str,
    pub wallet: &'a str,
    pub rows: &'a [Value],
    pub selected_row_ids: &'a [String],
    pub source_sha256: &'a str,
    pub include_quarantined: bool,
    pub confidence_threshold: &'a Value,
}

/// An open, unlocked book.
/// The daemon's background local-evidence pass
/// (`daemon_chain_analysis_watches`): authorized source runs, then due watch
/// evaluation, on the worker's own connection to the same book.
pub trait WatchWorker: Send {
    /// One `worker_tick`, then the undelivered inbox ids for a wake-up event.
    fn pass(&mut self, cancelled: &dyn Fn() -> bool) -> Result<Vec<String>, AppError>;
    /// The receipt after the event was written.
    fn delivered(&mut self, ids: &[String]) -> Result<(), AppError>;
}

pub trait Book: Send {
    /// Python's `_journals_refresh_payload`: the desktop action pairs
    /// automatically, the assistant's scoped tool does not.
    fn refresh_journals(
        &mut self,
        _context: &BookContext,
        _auto_pair: bool,
        _cancel: &kassiber_wallets::scan::Cancellation,
        _progress: &mut dyn FnMut(&Value) -> Result<(), AppError>,
    ) -> Result<Box<dyn JournalReply + '_>, AppError> {
        Err(not_implemented("journals", "desktop processing"))
    }

    fn process_journals(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        _auto_pair: bool,
        _progress: &mut dyn FnMut(&'static str),
    ) -> Result<Box<dyn JournalReply + '_>, JournalFailure> {
        Err(not_implemented("journals", "process").into())
    }

    fn chain_analysis_reads(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        operation: &str,
        _args: &Value,
        _local: &mut kassiber_wallets::analysis_index::reads::Local,
    ) -> Result<Value, AppError> {
        Err(not_implemented("chain_analysis", operation))
    }

    /// A background worker for an encrypted book; plaintext books get none.
    fn chain_analysis_watch_worker(&mut self) -> Result<Option<Box<dyn WatchWorker>>, AppError> {
        Ok(None)
    }

    /// Daemon dataset import over a staged token, and the `.start` job forms
    /// on the shared analysis runtime (`daemon_chain_analysis.job_starter`).
    fn chain_analysis_datasets(
        &mut self,
        _operation: &str,
        _args: &Value,
        _runtime: &std::sync::Arc<kassiber_wallets::analysis_index::runtime::Runtime>,
    ) -> Result<Value, AppError> {
        Err(not_implemented("chain-analysis", "datasets"))
    }

    /// `chain-analysis datasets import` over a caller-opened local file.
    fn chain_analysis_import(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        _args: &Value,
        _file: Box<dyn std::io::BufRead + Send>,
    ) -> Result<Value, AppError> {
        Err(not_implemented("chain-analysis", "datasets.import"))
    }

    fn chain_analysis_ops(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        operation: &str,
        _args: &Value,
    ) -> Result<Value, AppError> {
        Err(not_implemented("chain-analysis", operation))
    }

    /// Redacted report over this admitted connection; never exposes raw rows/configuration.
    fn diagnostics(
        &self,
        _args: &Value,
        _version: &str,
        _error: Option<&AppError>,
    ) -> Result<Value, AppError> {
        Err(not_implemented("diagnostics", "public report"))
    }

    fn backup_fingerprint(
        &self,
        _project: Option<&str>,
    ) -> Result<kassiber_backup::desktop::Fingerprint, AppError> {
        Err(not_implemented("backup", "restore fingerprint"))
    }
    fn backup_summary(&self) -> Result<Value, AppError> {
        Err(not_implemented("backup", "book summary"))
    }
    fn backup_export(
        &self,
        _output: &std::path::Path,
        _passphrase: &str,
        _version: &str,
    ) -> Result<Value, AppError> {
        Err(not_implemented("backup", "desktop export"))
    }
    fn close_for_restore(
        &mut self,
        _expected: &kassiber_backup::desktop::Fingerprint,
        _project: Option<&str>,
    ) -> Result<(), AppError> {
        Err(not_implemented("backup", "restore close"))
    }
    /// The rates service family. Unported/test stores fail closed.
    fn rates(&mut self, command: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("rates", command))
    }
    /// Updated local default after configuration mutations.
    fn current_default_backend(&self) -> Option<String> {
        None
    }
    /// Domain service dispatch, shared by CLI and daemon family adapters.
    fn service(&mut self, operation: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("service", operation))
    }
    fn documents(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        _command: &kassiber_transactions::documents::Command,
    ) -> Result<Value, AppError> {
        Err(not_implemented("documents", "document evidence services"))
    }
    fn commercial(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        _command: &kassiber_transactions::commercial::Command,
    ) -> Result<Value, AppError> {
        Err(not_implemented("commercial", "stored commercial services"))
    }
    /// Desktop transaction projections over the current book context.
    fn transaction_desktop(
        &mut self,
        _projection: kassiber_transactions::desktop::Projection,
        _args: Option<&Value>,
    ) -> Result<Value, AppError> {
        Err(not_implemented(
            "transactions",
            "desktop transaction projections",
        ))
    }

    /// `_auto_maintain_for_read`: rebuild stale journals of the active book
    /// before a report-like read. Stores without journals have nothing to do.
    fn maintain_for_read(&mut self) -> Result<(), AppError> {
        Ok(())
    }
    /// `ui.transactions.dashboard` over the caller's suggestion candidates.
    fn transaction_dashboard(
        &mut self,
        _args: &Value,
        _candidates: &[Value],
    ) -> Result<Value, AppError> {
        Err(not_implemented(
            "transactions",
            "desktop transaction dashboard",
        ))
    }

    /// Observed transaction pages, without journal processing or report authority.
    fn transactions(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        _options: &kassiber_transactions::listing::Options,
    ) -> Result<Value, AppError> {
        Err(not_implemented("transactions", "transaction listing"))
    }

    /// Desktop journal reads over the active book (`ui.journals.*` views).
    fn journal_view(&mut self, kind: &str, _args: Option<&Value>) -> Result<Value, AppError> {
        Err(not_implemented("journals", kind))
    }
    /// Local stored transfer/review services. No journal or network fallback.
    fn transfers(&mut self, operation: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("transfers", operation))
    }
    /// Local metadata edits and append-only history state.
    fn metadata(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        _command: &kassiber_transactions::metadata::Command,
    ) -> Result<Value, AppError> {
        Err(not_implemented("metadata", "metadata services"))
    }
    /// Managed attachment evidence. No URL is fetched by this API.
    fn attachments(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        _command: &kassiber_transactions::attachments::Command,
    ) -> Result<Value, AppError> {
        Err(not_implemented("attachments", "attachment services"))
    }
    /// Local loan marks, shared by CLI and daemon transports.
    fn loans(
        &mut self,
        _workspace: Option<&Value>,
        _profile: Option<&Value>,
        _command: &kassiber_transactions::loans::Command,
    ) -> Result<Value, AppError> {
        Err(not_implemented("loans", "loan services"))
    }
    fn wallets_snapshot(&self, _default_backend: Option<&str>) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "wallet snapshot"))
    }
    /// `ui.wallets.utxos`: the stored inventory of one wallet; never refreshes.
    fn wallet_utxos(
        &self,
        _args: Option<&Value>,
        _default_backend: Option<&str>,
    ) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "wallet UTXO inventory"))
    }
    fn wallets(&mut self, _operation: kassiber_wallets::Operation) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "wallet lifecycle"))
    }
    fn wallet_sync_supported(
        &self,
        _profile_id: &str,
        _wallet: Option<&str>,
        _all: bool,
        _force_full: bool,
    ) -> Result<bool, AppError> {
        Err(not_implemented("wallets", "wallet sync capability"))
    }
    fn freshness_plan(
        &self,
        _request: &kassiber_wallets::freshness::planning::Request,
    ) -> Result<kassiber_wallets::freshness::planning::Plan, AppError> {
        Err(not_implemented("wallets", "freshness admission"))
    }
    fn freshness_execute(
        &mut self,
        _plan: kassiber_wallets::freshness::planning::Plan,
        _options: &kassiber_wallets::freshness::execution::Options,
        _consent: Option<&kassiber_wallets::freshness::native::Consent>,
        _cancel: &kassiber_wallets::scan::Cancellation,
        _progress: &mut dyn FnMut(&Value) -> Result<(), AppError>,
    ) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "freshness execution"))
    }
    fn freshness_control(&mut self, _action: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "freshness control"))
    }
    fn graph_followup(
        &mut self,
        _grant: kassiber_wallets::freshness::graph::Grant,
        _stop: &kassiber_wallets::scan::Cancellation,
    ) -> Result<Option<Value>, AppError> {
        Err(not_implemented("wallets", "graph reference followup"))
    }
    fn wallet_sync(
        &mut self,
        _profile_id: &str,
        _wallet: Option<&str>,
        _all: bool,
        _force_full: bool,
    ) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "wallet sync"))
    }
    /// Stored reports, local exports and append-only saved/filed markers;
    /// no Python or network fallback.
    fn reports(&mut self, command: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("reports", command))
    }
    fn derive_wallet(&mut self, _request: kassiber_wallets::Derive) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "descriptor derivation"))
    }
    fn import_wallet_file(
        &mut self,
        _request: kassiber_wallets::file_import::Request,
    ) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "file import"))
    }
    fn import_ledger_file(
        &mut self,
        _request: kassiber_wallets::file_import::Request,
        _column_map: Option<kassiber_wallets::file_import::ledger::ColumnMap>,
    ) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "generic ledger import"))
    }
    fn document_target(
        &self,
        _profile_id: &str,
        _wallet: Option<&str>,
    ) -> Result<DocumentTarget, AppError> {
        Err(not_implemented("wallets", "document target"))
    }
    fn preview_document(
        &mut self,
        _request: kassiber_wallets::file_import::document::DocumentPreview,
    ) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "document preview"))
    }
    fn import_document(&mut self, _request: ReviewedDocument<'_>) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "reviewed document import"))
    }
    fn import_samourai(
        &mut self,
        _request: kassiber_wallets::file_import::samourai::Request,
    ) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "Samourai import"))
    }
    /// Local file-import run services. Unsupported seam stores fail closed.
    fn import_runs(&mut self, _operation: kassiber_imports::Operation) -> Result<Value, AppError> {
        Err(not_implemented("imports", "file import runs"))
    }
    /// `cmd_status`'s check: an `app_version` setting, or a legacy
    /// workspace row.
    fn is_initialized(&self) -> Result<bool, AppError>;
    fn context(&self) -> Result<BookContext, AppError>;
    fn status_facts(&self) -> Result<StatusFacts, AppError>;
    /// `resolve_scope(conn, workspace, profile)`: the ids a request's
    /// explicit `workspace`/`profile` selectors name.
    fn resolve_scope(
        &self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
    ) -> Result<(String, String), AppError>;
    /// Undo the request's uncommitted writes (the daemon does this after a
    /// failed request).
    fn rollback(&mut self) -> Result<(), AppError>;
    /// Closes the connection. A failed close leaves the book open, and the
    /// caller keeps it to retry before releasing ownership.
    fn close(&mut self) -> Result<(), AppError>;
}

/// `canonical_project(data_root)`: the project a data root belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRef {
    /// Opaque identity compared across processes.
    pub identity: String,
    /// The id envelopes may show.
    pub public_id: String,
    /// The canonical data root (`project.database.parent`).
    pub data_root: String,
}

/// A held project ownership lock (`ProjectOwnerLease`).
pub trait ProjectOwner: Send {
    fn project(&self) -> &ProjectRef;
    /// Retain path exclusion when a first open creates the database inode.
    fn refresh_after_open(&mut self) -> Result<(), AppError> {
        Ok(())
    }
    /// Releases the lock; releasing twice is harmless. On failure the
    /// caller keeps the owner as a retired resource and retries.
    fn release(&mut self) -> Result<(), AppError>;
}

/// Who holds ownership (`owner_kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    Desktop,
}

/// The storage operations the shell calls.
pub trait BookStore: Send + Sync {
    /// Test-only hybrid replay must also prove that bootstrap, migrations and
    /// closing the connection preserved the originally selected book files.
    /// A missing guard never permits replay.
    fn journal_replay_guard(&self, _selection: &Selection) -> Option<Box<dyn Fn() -> bool + Send>> {
        None
    }
    fn restore_maintenance(&self, _data_root: &str) -> Result<Box<dyn Send>, AppError> {
        Err(not_implemented("backup", "restore maintenance"))
    }
    fn restore_invalidate(
        &self,
        _data_root: &str,
        _project: Option<&str>,
    ) -> Result<RuntimeConfig, AppError> {
        Err(not_implemented("backup", "restore credentials"))
    }
    /// Narrow desktop maintenance seam. Secrets stay separate from argument
    /// maps and ordinary local_service dispatch. The desktop retains its lease.
    fn rotate_passphrase(
        &self,
        _data_root: &str,
        _env_file: &str,
        _current: &Secret,
        _new: &Secret,
    ) -> Result<Value, AppError> {
        Err(AppError::new(
            "not_implemented",
            "database passphrase rotation is unavailable",
        ))
    }
    /// The desktop `ui.secrets.init` storage step after the daemon closed its
    /// book: encrypt the plaintext database, or create an empty encrypted one,
    /// under exclusive maintenance with the desktop role retained.
    fn init_encryption(&self, _data_root: &str, _passphrase: &Secret) -> Result<Value, AppError> {
        Err(AppError::new(
            "not_implemented",
            "database encryption is unavailable",
        ))
    }
    fn forget_cli_unlock(&self, _data_root: &str) -> Result<Value, AppError> {
        Err(AppError::new(
            "not_implemented",
            "CLI credential revocation is unavailable",
        ))
    }
    /// Local configuration services that do not open a database.
    fn local_service(&self, operation: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("service", operation))
    }
    /// The desktop daemon's project catalog seam.
    fn project_catalog(&self, _operation: ProjectCatalog<'_>) -> Result<Value, AppError> {
        Err(not_implemented("store", "the project catalog"))
    }
    /// `create_empty_encrypted_database(database, passphrase)`.
    fn create_encrypted_database(&self, _database: &str, _: &Secret) -> Result<(), AppError> {
        Err(not_implemented("store", "encrypted database creation"))
    }
    /// `load_runtime_config(resolve_effective_env_file(None, data_root))`,
    /// without creating the layout.
    fn runtime_config_for(&self, _data_root: &str) -> Result<RuntimeConfig, AppError> {
        Err(not_implemented("store", "runtime configuration"))
    }
    /// `migrate_hidden_home_state_root_if_needed()`. The shell calls it
    /// where `_maybe_migrate_default_state_root` does: before broker routing
    /// and bootstrap, and only without `--data-root` or `--env-file`.
    fn migrate_default_state_root(&self) -> Result<(), AppError>;
    /// `bootstrap_runtime(args, needs_db=False)`.
    fn bootstrap(&self, selection: &Selection) -> Result<Bootstrap, AppError>;
    /// `effective_unlock_mode(_selected_data_root(args))`, which decides
    /// whether `route_brokered_command` hands the command to the broker.
    fn effective_unlock_mode(&self, selection: &Selection) -> Result<UnlockMode, AppError>;
    /// `_data_root_database_is_encrypted(data_root)`, without opening it.
    fn database_is_encrypted(&self, data_root: &str) -> Result<bool, AppError>;
    fn canonical_project(&self, data_root: &str) -> Result<ProjectRef, AppError>;
    /// `acquire_project_ownership(project, owner_kind, generation)`,
    /// compatible with the Python broker's and daemon's locks.
    fn acquire_ownership(
        &self,
        project: &ProjectRef,
        owner_kind: OwnerKind,
        generation: &str,
    ) -> Result<Box<dyn ProjectOwner>, AppError>;
    /// Opens and unlocks the book, then runs
    /// `validate_project_migration_after_unlock` and
    /// `merge_db_backends(conn, runtime_config)`, updating the caller's
    /// configuration (loaded by [`BookStore::bootstrap`] from its env file).
    fn open_book(
        &self,
        data_root: &str,
        unlock: Unlock<'_>,
        runtime_config: &mut RuntimeConfig,
    ) -> Result<Box<dyn Book>, AppError>;
    /// `_verify_passphrase_with_backoff`: checks `passphrase` against the
    /// file with a throwaway connection, under the auth backoff for
    /// `scope` (which may refuse with its own error).
    fn verify_passphrase(
        &self,
        data_root: &str,
        scope: &str,
        passphrase: &Secret,
    ) -> Result<bool, AppError>;
    /// `mark_project_opened(project_id, data_root, select)`.
    fn mark_project_opened(
        &self,
        project_id: &str,
        data_root: &str,
        select: bool,
    ) -> Result<(), AppError>;
}

/// The typed error for work the Rust backend has not ported yet.
pub fn not_implemented(component: &str, what: &str) -> AppError {
    AppError::new(
        "not_implemented",
        format!("{what} is not implemented in the Rust backend yet"),
    )
    .with_details(dict([("component", s(component))]))
}

/// Fails every storage operation closed until the storage crates land.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubStore;

impl BookStore for StubStore {
    fn migrate_default_state_root(&self) -> Result<(), AppError> {
        Err(not_implemented("store", "the default state-root migration"))
    }

    fn bootstrap(&self, _: &Selection) -> Result<Bootstrap, AppError> {
        Err(not_implemented("store", "opening a project"))
    }

    fn effective_unlock_mode(&self, _: &Selection) -> Result<UnlockMode, AppError> {
        Err(not_implemented(
            "store",
            "reading the project's unlock mode",
        ))
    }

    fn database_is_encrypted(&self, _: &str) -> Result<bool, AppError> {
        Err(not_implemented("store", "reading the database file"))
    }

    fn canonical_project(&self, _: &str) -> Result<ProjectRef, AppError> {
        Err(not_implemented("store", "resolving the project"))
    }

    fn acquire_ownership(
        &self,
        _: &ProjectRef,
        _: OwnerKind,
        _: &str,
    ) -> Result<Box<dyn ProjectOwner>, AppError> {
        Err(not_implemented("store", "project ownership"))
    }

    fn open_book(
        &self,
        _: &str,
        _: Unlock<'_>,
        _: &mut RuntimeConfig,
    ) -> Result<Box<dyn Book>, AppError> {
        Err(not_implemented("store", "opening the book"))
    }

    fn verify_passphrase(&self, _: &str, _: &str, _: &Secret) -> Result<bool, AppError> {
        Err(not_implemented("store", "passphrase verification"))
    }

    fn mark_project_opened(&self, _: &str, _: &str, _: bool) -> Result<(), AppError> {
        Err(not_implemented("store", "the project catalog"))
    }
}
