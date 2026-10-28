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

/// An open, unlocked book.
pub trait Book: Send {
    /// The rates service family. Unported/test stores fail closed.
    fn rates(&mut self, command: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("rates", command))
    }
    /// Domain service dispatch, shared by CLI and daemon family adapters.
    fn service(&mut self, operation: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("service", operation))
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
    fn derive_wallet(&mut self, _request: kassiber_wallets::Derive) -> Result<Value, AppError> {
        Err(not_implemented("wallets", "descriptor derivation"))
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
    /// Local configuration services that do not open a database.
    fn local_service(&self, operation: &str, _args: &Value) -> Result<Value, AppError> {
        Err(not_implemented("service", operation))
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
