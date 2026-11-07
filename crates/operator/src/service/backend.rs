//! What the service needs from the rest of Kassiber, as one seam.
//!
//! Python's service imports `canonical_project`, `acquire_project_ownership`,
//! `open_db`, the mode functions, argparse and the path resolver directly,
//! and its tests patch them. Here they come through [`Backend`], which the
//! binary implements over the storage crates and the tests implement with
//! fakes. The runner that launches children is a separate seam
//! ([`Runner`]).

use crate::error::Result;
use kassiber_ownership::CanonicalProject;
use std::path::PathBuf;

/// `Capability` (`command_capabilities.py`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    Read,
    Operator,
    AccountingDecisions,
    Admin,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Read => "read",
            Capability::Operator => "operator",
            Capability::AccountingDecisions => "accounting_decisions",
            Capability::Admin => "admin",
        }
    }

    /// `Capability(value)`.
    pub fn parse(value: &str) -> Option<Capability> {
        Some(match value {
            "read" => Capability::Read,
            "operator" => Capability::Operator,
            "accounting_decisions" => Capability::AccountingDecisions,
            "admin" => Capability::Admin,
            _ => return None,
        })
    }

    fn order(self) -> u8 {
        match self {
            Capability::Read => 1,
            Capability::Operator => 2,
            Capability::AccountingDecisions => 3,
            Capability::Admin => 4,
        }
    }

    /// `capability_allows(granted, required)`: cumulative, and never admin.
    pub fn allows(granted: Capability, required: Capability) -> bool {
        if required == Capability::Admin || granted == Capability::Admin {
            return false;
        }
        granted.order() >= required.order()
    }
}

/// `_parse_argv(argv)`: the command path, its capability, and the parsed
/// destinations admission reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classified {
    pub command_path: String,
    pub capability: Capability,
    /// `hasattr(parsed, "workspace")` and its value when truthy.
    pub workspace: Option<Option<String>>,
    pub profile: Option<Option<String>>,
    /// `bool(getattr(parsed, "install", False))`.
    pub install: bool,
    /// `getattr(parsed, name, None)`, empty strings included.
    pub data_root: Option<String>,
    pub project: Option<String>,
    pub env_file: Option<String>,
}

/// A held broker ownership lease (`ProjectOwnerLease`).
pub trait Owner: Send {
    /// `add_alias(project)`: also hold this path's locks.
    fn add_alias(&mut self, project: &CanonicalProject) -> Result<()>;
    /// `duplicate_for_child()`: handles one child inherits.
    fn duplicate_for_child(&self) -> Result<Box<dyn ChildOwner>>;
    /// `release()`; a failure leaves the owner held for a retry.
    fn release(&mut self) -> Result<()>;
}

/// `ProjectOwnerChildHandles`.
pub trait ChildOwner: Send {
    /// The descriptor or handle numbers the child inherits.
    fn tokens(&self) -> Vec<kassiber_ipc::process::Inheritable>;
    fn close(self: Box<Self>) -> Result<()>;
}

/// A connection `open_db` returned.
pub trait Database: Send {
    /// `database_instance_id(connection)`; `None` for a connection without
    /// a database (a test double), which Python reads as the first 32
    /// characters of the project identity.
    fn instance_id(&self) -> Result<Option<String>>;
    /// `current_context_snapshot(connection)`: the workspace and profile
    /// ids, each `None` when empty.
    fn context(&self) -> Result<(Option<String>, Option<String>)>;
    /// `close()`; a failure keeps the connection for a retry.
    fn close(&mut self) -> Result<()>;
}

/// Kassiber's storage, policy and argv grammar, as the broker uses them.
pub trait Backend: Send + Sync {
    fn canonical_project(&self, data_root: &str) -> Result<CanonicalProject>;
    /// `acquire_project_ownership(project, owner_kind="broker", generation)`.
    fn acquire_owner(&self, project: &CanonicalProject, generation: &str)
        -> Result<Box<dyn Owner>>;
    /// `open_db(data_root, passphrase=..., require_existing_schema=True,
    /// expected_database_identity=...)`.
    fn open_database(
        &self,
        data_root: &str,
        passphrase: &str,
        expected_database_identity: Option<&str>,
    ) -> Result<Box<dyn Database>>;
    /// `resolve_config_root(data_root) / "auth_backoff.json"`.
    fn auth_backoff_path(&self, data_root: &str) -> Result<Option<PathBuf>>;
    /// `effective_unlock_mode(data_root)`.
    fn effective_unlock_mode(&self, data_root: &str) -> Result<String>;
    /// `set_unlock_mode(data_root, mode, database_identity=...,
    /// expected_project_identity=...)`.
    fn set_unlock_mode(
        &self,
        data_root: &str,
        mode: &str,
        database_identity: Option<&str>,
        expected_project_identity: Option<&str>,
    ) -> Result<String>;
    /// `bind_project_policy(data_root, database_identity,
    /// expected_project_identity=...)`.
    fn bind_project_policy(
        &self,
        data_root: &str,
        database_identity: &str,
        expected_project_identity: Option<&str>,
    ) -> Result<()>;
    /// `_parse_argv(argv)` with the broker's secret labels already
    /// normalized away by the caller.
    fn classify(&self, argv: &[String]) -> Result<Classified>;
    /// `resolve_runtime_paths(data_root, env_file, project).data_root`.
    fn resolve_data_root(
        &self,
        data_root: Option<&str>,
        env_file: Option<&str>,
        project: Option<&str>,
    ) -> Result<String>;
}

/// A dispatched operation as the runner sees it.
pub struct Dispatch {
    pub id: String,
    pub project_identity: String,
    pub database_identity: String,
    pub data_root: String,
    pub argv: Vec<String>,
    pub command_path: String,
    pub capability: Capability,
    /// The command-specific secrets in admission order.
    pub secret_arguments: Vec<(String, zeroize::Zeroizing<Vec<u8>>)>,
    pub working_directory: Option<String>,
    pub no_egress: bool,
    pub owner_tokens: Vec<kassiber_ipc::process::Inheritable>,
}

/// `OperationResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationResult {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
}

impl OperationResult {
    pub fn new(exit_code: i64, stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        OperationResult {
            exit_code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }
}

/// Runs one dispatched operation with the lease passphrase.
pub trait Runner: Send + Sync {
    fn run(&self, dispatch: &Dispatch, passphrase: &[u8]) -> Result<OperationResult>;
}

impl<F> Runner for F
where
    F: Fn(&Dispatch, &[u8]) -> Result<OperationResult> + Send + Sync,
{
    fn run(&self, dispatch: &Dispatch, passphrase: &[u8]) -> Result<OperationResult> {
        self(dispatch, passphrase)
    }
}
