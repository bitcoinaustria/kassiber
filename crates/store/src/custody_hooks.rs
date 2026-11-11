//! Custody steps of the ordered compatibility pass that the store cannot
//! evaluate itself.
//!
//! Python's `ensure_schema_compat` calls into the custody core for three
//! book migrations: componentless dismissal recovery, legacy reviewed
//! pair/payout consolidation and the legacy pair fee review. They need the
//! component validator and the gap matcher, which live in crates that depend
//! on this one. The product binary installs their implementation once at
//! start-up, so every migrating open in the process (CLI, daemon, backup
//! export, secrets init/rekey, restore, broker) runs them at Python's
//! positions. Without an installation the pass still refuses these books
//! before writing anything the step would have written.
use crate::{Error, ErrorKind, Result};
use rusqlite::Connection;
use serde_json::json;
use std::sync::OnceLock;

/// The clock a custody migration step reads. Product opens read the system
/// clock; deterministic test opens pin one instant (the same one the store's
/// `strftime` override returns).
#[derive(Clone, Copy, Debug, Default)]
pub struct MigrationClock<'a> {
    pinned: Option<&'a str>,
}

impl<'a> MigrationClock<'a> {
    pub fn system() -> Self {
        Self { pinned: None }
    }
    /// A clock fixed at `timestamp` (`YYYY-MM-DDTHH:MM:SS[.fff]Z`).
    pub fn pinned(timestamp: &'a str) -> Self {
        Self {
            pinned: Some(timestamp),
        }
    }
    pub fn pinned_timestamp(&self) -> Option<&'a str> {
        self.pinned
    }
}

/// The custody-layer migrations. Each runs inside the caller's write
/// transaction and leaves committing (or rolling back) to the store.
pub trait CustodyMigrations: Sync {
    /// `refresh_legacy_authored_components`: consolidate pending legacy
    /// reviewed pairs and direct payouts into authored components.
    fn refresh_legacy_authored_components(
        &self,
        conn: &Connection,
        clock: MigrationClock<'_>,
    ) -> Result<()>;
    /// Whether the component's validation reports
    /// `legacy_pair_fee_allocation_review_required`.
    fn requires_legacy_pair_fee_review(
        &self,
        conn: &Connection,
        component_id: &str,
    ) -> Result<bool>;
}

static INSTALLED: OnceLock<&'static dyn CustodyMigrations> = OnceLock::new();

/// Install the custody migrations for this process. The first installation
/// wins; later calls are no-ops.
pub fn install(migrations: &'static dyn CustodyMigrations) {
    let _ = INSTALLED.set(migrations);
}

pub fn installed() -> Option<&'static dyn CustodyMigrations> {
    INSTALLED.get().copied()
}

/// The refusal when a step is pending and no custody layer is installed.
pub(crate) fn unavailable(step: &str, reason: &str) -> Error {
    Error::new(
        ErrorKind::UnsupportedSchemaLayout,
        "this book needs a custody migration that this build does not include",
    )
    .hint("Open this book with the Kassiber application or command-line binary, which migrates it.")
    .details(json!({"migration_step":step,"reason":reason}))
}
