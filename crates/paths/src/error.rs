use serde::Serialize;
use serde_json::{json, Value};
use std::fmt;

/// Stable boundary codes. Details never contain raw SQL, credentials or driver text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    RequiresPythonMigration,
    UnsupportedSchemaLayout,
    NotInitialized,
    InvalidFlagCombination,
    Validation,
    ProjectNotFound,
    UnsafeConfiguration,
    UnsupportedConfiguration,
    UnsafeProjectDatabase,
    UnsafeProjectOwner,
    UnsafeProjectOwnerLock,
    ProjectOwnerLockUnavailable,
    ProjectInUse,
    ProjectOwnerMismatch,
    OperatorProjectReplaced,
    OperatorProjectBindingInvalid,
    OperatorPolicyBindingRequired,
    OperatorPolicyBindingMismatch,
    InvalidOperatorUnlockMode,
    InvalidProjectDatabase,
    InvalidPassphrase,
    PassphraseRequired,
    PlaintextDatabase,
    UnlockFailed,
    InteractionRequired,
    DatabaseBusy,
    DatabaseJournalModeUnavailable,
    DatabaseError,
    DatabaseExists,
    MigrationVerificationFailed,
    ResumeRequired,
    BackupExists,
    NotPlaintextDatabase,
    MissingDatabase,
    RekeyVerificationFailed,
    MigrationLeaksPlaintext,
    LegacyLedgerPresent,
    CustodyReviewRelationMigrationConflict,
    CustodySchemaMigrationFailed,
    StateRootMigrationFailed,
    StateRootConflict,
    StateRootMigrationInUse,
    SecretStoreUnavailable,
    UnsafeRestoreTarget,
    RestoreInstallFailed,
    RestoreRollbackFailed,
    RestoreRecoveryRequired,
    Io,
}
impl ErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::RequiresPythonMigration => "requires_python_migration",
            Self::UnsupportedSchemaLayout => "unsupported_schema_layout",
            Self::NotInitialized => "not_initialized",
            Self::InvalidFlagCombination => "invalid_flag_combination",
            Self::Validation => "validation",
            Self::ProjectNotFound => "project_not_found",
            Self::UnsafeConfiguration => "unsafe_configuration",
            Self::UnsupportedConfiguration => "unsupported_configuration",
            Self::UnsafeProjectDatabase => "unsafe_project_database",
            Self::UnsafeProjectOwner => "unsafe_project_owner",
            Self::UnsafeProjectOwnerLock => "unsafe_project_owner_lock",
            Self::ProjectOwnerLockUnavailable => "project_owner_lock_unavailable",
            Self::ProjectInUse => "project_in_use",
            Self::ProjectOwnerMismatch => "project_owner_mismatch",
            Self::OperatorProjectReplaced => "operator_project_replaced",
            Self::OperatorProjectBindingInvalid => "operator_project_binding_invalid",
            Self::OperatorPolicyBindingRequired => "operator_policy_binding_required",
            Self::OperatorPolicyBindingMismatch => "operator_policy_binding_mismatch",
            Self::InvalidOperatorUnlockMode => "invalid_operator_unlock_mode",
            Self::InvalidProjectDatabase => "invalid_project_database",
            Self::InvalidPassphrase => "invalid_passphrase",
            Self::PassphraseRequired => "passphrase_required",
            Self::PlaintextDatabase => "plaintext_database",
            Self::UnlockFailed => "unlock_failed",
            Self::InteractionRequired => "interaction_required",
            Self::DatabaseBusy => "database_busy",
            Self::DatabaseJournalModeUnavailable => "database_journal_mode_unavailable",
            Self::DatabaseError => "internal_error",
            Self::DatabaseExists => "database_exists",
            Self::MigrationVerificationFailed => "migration_verification_failed",
            Self::ResumeRequired => "resume_required",
            Self::BackupExists => "backup_exists",
            Self::NotPlaintextDatabase => "not_plaintext_database",
            Self::MissingDatabase => "missing_database",
            Self::RekeyVerificationFailed => "rekey_verification_failed",
            Self::MigrationLeaksPlaintext => "migration_leaks_plaintext",
            Self::LegacyLedgerPresent => "legacy_ledger_present",
            Self::CustodyReviewRelationMigrationConflict => {
                "custody_review_relation_migration_conflict"
            }
            Self::CustodySchemaMigrationFailed => "custody_schema_migration_failed",
            Self::StateRootMigrationFailed => "state_root_migration_failed",
            Self::StateRootConflict => "state_root_conflict",
            Self::StateRootMigrationInUse => "state_root_migration_in_use",
            Self::SecretStoreUnavailable => "secret_store_unavailable",
            Self::UnsafeRestoreTarget => "unsafe_restore_target",
            Self::RestoreInstallFailed => "restore_install_failed",
            Self::RestoreRollbackFailed => "restore_rollback_failed",
            Self::RestoreRecoveryRequired => "restore_recovery_required",
            Self::Io => "storage_io_error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Error {
    #[serde(skip)]
    pub kind: ErrorKind,
    pub code: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<std::borrow::Cow<'static, str>>,
    pub retryable: bool,
    pub details: Value,
}
impl Error {
    pub fn new(kind: ErrorKind, message: &'static str) -> Self {
        Self {
            kind,
            code: kind.code(),
            message,
            hint: None,
            retryable: false,
            details: json!({}),
        }
    }
    pub fn retryable(mut self, value: bool) -> Self {
        self.retryable = value;
        self
    }
    pub fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint.into());
        self
    }
    /// A hint built at run time, such as one naming the holder's pid.
    pub fn hint_text(mut self, hint: String) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn details(mut self, value: Value) -> Self {
        self.details = value;
        self
    }
    pub fn migration(reason: &'static str) -> Self {
        Self::new(
            ErrorKind::RequiresPythonMigration,
            "this book's storage layout is not supported by this Kassiber build",
        )
        .details(json!({"reason": reason}))
    }
    pub fn io(_: std::io::Error) -> Self {
        Self::new(ErrorKind::Io, "local storage operation failed")
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
