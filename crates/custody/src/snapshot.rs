//! Raw v1 request. Open record fields preserve historical SQLite shapes.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
mod book;
mod identifiers;
pub use identifiers::InternedId;
mod network;
mod rows;
pub use book::BookSnapshot;
pub use network::{NetworkArray, NetworkFacts, NetworkObject};
pub use rows::{Column, NetworkRow, RateRow, SqlValue, TxRow, WalletRef, WorklistRow};
/// Compatibility model for stages and native callers awaiting their typed slice.
/// Text parsing and admission use `BookSnapshot` directly.
pub type Snapshot = StageSnapshot;

/// Exact signed-64 database quantity. Never a binary float or decimal BTC.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Msat(pub i64);

pub type Record = Map<String, Value>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub header: Record,
    pub legs: Vec<Record>,
    pub allocations: Vec<Record>,
    pub economic_terms: Vec<Record>,
    pub commitments: Vec<Record>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GapWorklist {
    pub large_book: bool,
    pub rows: Vec<Record>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageSnapshot {
    pub schema_version: u32,
    pub operation: String,
    pub as_of: String,
    pub profile: Record,
    pub book_network: Option<Value>,
    pub wallets: Vec<Record>,
    pub transactions: Vec<Value>,
    pub rate_rows: Vec<Record>,
    pub owned_index: Option<Value>,
    pub ownership_warnings: Vec<String>,
    pub components: Vec<Component>,
    pub anchors: Map<String, Value>,
    pub anchor_wallets: Vec<Record>,
    pub revision_headers: Vec<Record>,
    pub receipt_sources: Vec<Record>,
    pub channel_records: Vec<Record>,
    pub loan_legs: Vec<Record>,
    pub evidence_snapshots: Vec<Record>,
    pub migration_issues: Vec<Record>,
    pub gap_reviews: Vec<Record>,
    pub pair_dismissals: Vec<Record>,
    pub gap_worklist: GapWorklist,
}

impl StageSnapshot {
    /// The one explicit JSON boundary for neighbours awaiting their typed slice.
    pub fn rows(&self) -> &[Value] {
        &self.transactions
    }

    /// Test-only legacy constructors receive the same typed schema admission.
    pub fn validate(&self) -> Result<(), String> {
        self.typed()?.validate()
    }

    /// Compatibility records cross the same typed boundary as v1 text.
    pub fn typed(&self) -> Result<BookSnapshot, String> {
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        serde_json::from_str::<BookSnapshot>(&text).map_err(|e| e.to_string())
    }
}
