//! Raw v1 request. Open record fields preserve historical SQLite shapes.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GapWorklist {
    pub large_book: bool,
    pub rows: Vec<Record>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
        let mut records: Vec<&Record> = self
            .wallets
            .iter()
            .chain(&self.transactions)
            .chain(&self.anchor_wallets)
            .chain(&self.channel_records)
            .chain(&self.gap_worklist.rows)
            .collect();
        records.extend(self.anchors.values().filter_map(Value::as_object));
        for row in records {
            for name in ["config_json", "wallet_config_json"] {
                if let Some(value) = row.get(name).filter(|v| !v.is_null()) {
                    let config: Value = if let Some(text) = value.as_str() {
                        serde_json::from_str(text)
                            .map_err(|_| "custody configuration must be normalized facts")?
                    } else {
                        value.clone()
                    };
                    let object = config
                        .as_object()
                        .ok_or("custody configuration must be normalized facts")?;
                    if object.keys().any(|key| {
                        ![
                            "chain",
                            "network",
                            "bitcoin_network",
                            "chain_network",
                            "chain_instance_id",
                            "samourai",
                        ]
                        .contains(&key.as_str())
                    }) {
                        return Err(
                            "custody configuration contains fields outside normalized facts".into(),
                        );
                    }
                    if let Some(block) = config.get("samourai").and_then(Value::as_object) {
                        if block
                            .keys()
                            .any(|key| !["role", "group_id", "section"].contains(&key.as_str()))
                        {
                            return Err("custody Samourai metadata contains fields outside normalized facts".into());
                        }
                    }
                }
            }
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
