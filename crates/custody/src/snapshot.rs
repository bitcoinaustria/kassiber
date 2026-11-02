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
/// Typed admission inputs of a loaded snapshot.
pub(crate) struct AdmissionFacts {
    pub book_network: Option<NetworkObject<NetworkFacts>>,
    pub wallets: Vec<WalletRef>,
    pub rate_rows: Vec<RateRow>,
}

/// Compatibility model for stages and native callers awaiting their typed slice.
/// Text parsing and admission use `BookSnapshot` directly.
pub type Snapshot = StageSnapshot;
mod record;
pub use record::{Columns, Record};

/// Exact signed-64 database quantity. Never a binary float or decimal BTC.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Msat(pub i64);

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
    pub gap_reviews: Vec<Record>,
    pub pair_dismissals: Vec<Record>,
    pub gap_worklist: GapWorklist,
    #[serde(skip)]
    pub(crate) rows: std::sync::OnceLock<Vec<Value>>,
}

impl StageSnapshot {
    pub fn escape_wire_tags(&mut self) {
        self.rows.take();
        self.profile.escape_wire_tags();
        for rows in [
            &mut self.wallets,
            &mut self.transactions,
            &mut self.rate_rows,
            &mut self.anchor_wallets,
            &mut self.revision_headers,
            &mut self.receipt_sources,
            &mut self.channel_records,
            &mut self.loan_legs,
            &mut self.evidence_snapshots,
            &mut self.migration_issues,
            &mut self.gap_reviews,
            &mut self.pair_dismissals,
            &mut self.gap_worklist.rows,
        ] {
            rows.iter_mut().for_each(Record::escape_wire_tags);
        }
        for component in &mut self.components {
            component.header.escape_wire_tags();
            for rows in [
                &mut component.legs,
                &mut component.allocations,
                &mut component.economic_terms,
                &mut component.commitments,
            ] {
                rows.iter_mut().for_each(Record::escape_wire_tags);
            }
        }
        self.book_network
            .iter_mut()
            .chain(self.owned_index.iter_mut())
            .chain(self.anchors.values_mut())
            .for_each(crate::codec::escape_wire_tags);
    }
    /// Each transaction row as a JSON object, in order; shared by every reader.
    pub fn rows(&self) -> &[Value] {
        self.rows.get_or_init(|| {
            let _span = crate::timing::span("snapshot.rows");
            let rows: Vec<Value> = crate::par::map(&self.transactions, |row| {
                Value::Object(row.clone().into_map())
            })
            .into_iter()
            .map(crate::par::unwrap)
            .collect();
            // Moving the vector keeps its elements where they are.
            crate::graph::note_rows(&self.transactions, &rows);
            rows
        })
    }

    /// An owned copy of the JSON rows, built from the records when the
    /// shared rows are not.
    pub fn row_values(&self) -> Vec<Value> {
        match self.rows.get() {
            Some(rows) => rows.clone(),
            None => self
                .transactions
                .iter()
                .map(|row| Value::Object(row.clone().into_map()))
                .collect(),
        }
    }

    /// The JSON rows, if built.
    pub(crate) fn built_rows(&self) -> Option<&[Value]> {
        self.rows.get().map(Vec::as_slice)
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

    /// The typed facts loaded admission reads, exactly as `typed` would read
    /// them, without serializing the transactions.
    pub(crate) fn admission_facts(&self) -> Result<AdmissionFacts, String> {
        use serde::de::IntoDeserializer;
        let book_network = Option::<NetworkObject<NetworkFacts>>::deserialize(
            self.book_network
                .as_ref()
                .unwrap_or(&Value::Null)
                .into_deserializer(),
        )
        .map_err(|e| e.to_string())?;
        let wallets = self
            .wallets
            .iter()
            .map(Record::typed)
            .collect::<Result<_, _>>()?;
        let rate_rows = self
            .rate_rows
            .iter()
            .map(Record::typed)
            .collect::<Result<_, _>>()?;
        Ok(AdmissionFacts {
            book_network,
            wallets,
            rate_rows,
        })
    }

    /// Database loading retains legacy SQLite scalars until their consuming
    /// stage applies Python's conversion/refusal rules. The JSON adapter keeps
    /// its stricter quantity contract in `validate`; configuration admission
    /// applies to both paths and never admits wallet configuration secrets.
    pub fn validate_loaded(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.operation != "custody" {
            return Err("unsupported custody snapshot schema or operation".into());
        }
        if self.as_of.is_empty() {
            return Err("custody snapshot requires as_of".into());
        }
        let names = ["config_json", "wallet_config_json"];
        let values = self
            .wallets
            .iter()
            .chain(&self.transactions)
            .chain(&self.anchor_wallets)
            .chain(&self.channel_records)
            .chain(&self.gap_worklist.rows)
            .flat_map(|row| names.map(|name| row.get(name)))
            .chain(
                self.anchors
                    .values()
                    .filter_map(Value::as_object)
                    .flat_map(|row| names.map(|name| row.get(name))),
            )
            .flatten()
            .filter(|value| !value.is_null());
        let mut checked = std::collections::HashSet::new();
        for value in values {
            if value.as_str().is_some_and(|text| checked.contains(text)) {
                continue;
            }
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
                    return Err(
                        "custody Samourai metadata contains fields outside normalized facts".into(),
                    );
                }
            }
            if let Some(text) = value.as_str() {
                checked.insert(text);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn admitted() -> Snapshot {
        Snapshot {
            schema_version: 1,
            operation: "custody".into(),
            as_of: "2026-10-31T12:00:00Z".into(),
            ..Default::default()
        }
    }

    #[test]
    fn shared_snapshot_config_admission_checks_every_population() {
        let valid = json!({"config_json": "{\"chain\": \"bitcoin\"}"});
        for group in 0..6 {
            for key in ["config_json", "wallet_config_json"] {
                for invalid in [
                    json!("{broken"),
                    json!("null"),
                    json!("[]"),
                    json!(42),
                    json!({"secret": "private"}),
                    json!({"samourai": {"secret": "private"}}),
                    json!("{\"samourai\": {\"secret\": \"private\"}}"),
                ] {
                    let mut snapshot = admitted();
                    let record: Record = serde_json::from_value(valid.clone()).unwrap();
                    snapshot.wallets = vec![record.clone(), record];
                    let invalid = json!({key: invalid});
                    let row = serde_json::from_value(invalid.clone()).unwrap();
                    match group {
                        0 => snapshot.wallets.push(row),
                        1 => snapshot.transactions.push(row),
                        2 => snapshot.anchor_wallets.push(row),
                        3 => snapshot.channel_records.push(row),
                        4 => snapshot.gap_worklist.rows.push(row),
                        5 => {
                            snapshot.anchors.insert("anchor".into(), invalid);
                        }
                        _ => unreachable!(),
                    }
                    assert!(snapshot.validate().is_err(), "{group} {key}");
                }
            }
        }
        let mut snapshot = admitted();
        snapshot.wallets = [valid.clone(), valid, json!({"config_json": null}), json!({"wallet_config_json": {"samourai": {"role": "postmix", "group_id": "g", "section": "s"}}})]
            .into_iter().map(|v| serde_json::from_value(v).unwrap()).collect();
        assert!(snapshot.validate().is_ok());
    }

    #[test]
    fn shared_snapshot_wire_tags_reset_cached_rows() {
        let mut snapshot = admitted();
        snapshot.transactions = vec![serde_json::from_value(json!({
            "note": {"$kassiber_bytes": "ff00"}
        }))
        .unwrap()];
        let cached = snapshot.rows()[0].clone();
        snapshot.escape_wire_tags();
        assert_ne!(snapshot.rows()[0], cached);
        assert_eq!(
            crate::codec::from_json(&snapshot.rows()[0]["note"]).unwrap(),
            kassiber_pyjson::Value::Dict(vec![(
                kassiber_pyjson::Key::from("$kassiber_bytes"),
                kassiber_pyjson::Value::from("ff00")
            )])
        );
    }

    #[test]
    fn loaded_admission_facts_match_the_text_round_trip() {
        let mut snapshot = admitted();
        let values = [
            json!(null),
            json!(1),
            json!(-0),
            json!(1.5),
            serde_json::from_str("1e999").unwrap(),
            serde_json::from_str("1E5").unwrap(),
            serde_json::from_str("18446744073709551616").unwrap(),
            serde_json::from_str("-9223372036854775809").unwrap(),
            json!("text"),
            json!(true),
            json!([1, {"a": 2.0}]),
            json!({"z": 1, "a": [null]}),
        ];
        for value in &values {
            let row = json!({"id": "w", "asset": value, "fiat_rate": value, "config_json": value, "extra": value});
            snapshot
                .wallets
                .push(serde_json::from_value(row.clone()).unwrap());
            snapshot
                .rate_rows
                .push(serde_json::from_value(row.clone()).unwrap());
            snapshot
                .transactions
                .push(serde_json::from_value(row).unwrap());
        }
        let typed = snapshot.typed().unwrap();
        for (record, row) in snapshot.transactions.iter().zip(&typed.transactions) {
            assert_eq!(record.typed::<TxRow>().unwrap().json(), row.json());
        }
        for book_network in [
            None,
            Some(json!(null)),
            Some(json!(3)),
            Some(
                json!({"observations": [{"wallet_id": "w", "asset": 1e5}, 4], "references": [], "binding": null}),
            ),
            Some(json!({"observations": 5})),
        ] {
            snapshot.book_network = book_network;
            let typed = snapshot.typed().unwrap();
            let facts = snapshot.admission_facts().unwrap();
            fn text<T: serde::Serialize + ?Sized>(value: &T) -> String {
                serde_json::to_string(value).unwrap()
            }
            assert_eq!(text(&facts.book_network), text(&*typed.book_network));
            assert_eq!(text(&facts.wallets), text(&typed.wallets));
            assert_eq!(text(&facts.rate_rows), text(&typed.rate_rows));
        }
    }

    #[test]
    fn shared_snapshot_keeps_exact_msat_admission() {
        for field in ["amount", "fee"] {
            for (text, valid) in [
                ("null", true),
                ("0", true),
                ("-9223372036854775808", true),
                ("9223372036854775807", true),
                ("1.0", false),
                ("\"1\"", false),
                ("true", false),
                ("9223372036854775808", false),
            ] {
                let mut snapshot = admitted();
                snapshot.transactions =
                    vec![serde_json::from_str(&format!("{{\"{field}\":{text}}}")).unwrap()];
                assert_eq!(snapshot.validate().is_ok(), valid, "{field} {text}");
                assert!(snapshot.validate_loaded().is_ok(), "{field} {text}");
            }
        }
    }
}
