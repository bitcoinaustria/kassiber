//! Raw v1 request. Open record fields preserve historical SQLite shapes.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Exact signed-64 database quantity. Never a binary float or decimal BTC.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Msat(pub i64);

pub type Record = Map<String, Value>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Component {
    pub header: Record,
    pub legs: Vec<Record>,
    pub allocations: Vec<Record>,
    pub economic_terms: Vec<Record>,
    pub commitments: Vec<Record>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GapWorklist {
    pub large_book: bool,
    pub rows: Vec<Record>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub operation: String,
    pub as_of: String,
    pub profile: Record,
    pub book_network: Option<Value>,
    pub wallets: Vec<Record>,
    pub transactions: Vec<Record>,
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
    pub dismissed_gap_fingerprints: Record,
    pub pair_dismissals: Vec<Record>,
    pub gap_worklist: GapWorklist,
}

impl Snapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.operation != "custody" {
            return Err("unsupported custody snapshot schema or operation".into());
        }
        if self.as_of.is_empty() {
            return Err("custody snapshot requires as_of".into());
        }
        for row in &self.transactions {
            for name in ["amount", "fee"] {
                if let Some(value) = row.get(name).filter(|v| !v.is_null()) {
                    serde_json::from_value::<Msat>(value.clone())
                        .map_err(|_| format!("{name} must be an exact signed-64 msat integer"))?;
                }
            }
        }
        Ok(())
    }
}
