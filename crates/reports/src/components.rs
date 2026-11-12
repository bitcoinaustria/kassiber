//! Load only the raw facts the shared R2 component validator consumes.
use super::*;
use kassiber_custody::snapshot::{Component, Record, Snapshot};
use serde_json::{json, Map, Value as Json};
use std::collections::BTreeSet;

fn json_value(v: &Value) -> Result<Json> {
    let encoded = kassiber_pyjson::dumps(v, &Default::default())
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    serde_json::from_str(&encoded).map_err(|e| AppError::new("internal_error", e.to_string()))
}
fn records(conn: &Connection, query: &str, id: &str) -> Result<Vec<Record>> {
    rows(conn, query, [id])?
        .iter()
        .map(|row| {
            let mut row = json_value(row)?.as_object().cloned().expect("SQL record");
            for key in ["config_json", "wallet_config_json"] {
                if let Some(raw) = row.get(key) {
                    let parsed: Json = raw
                        .as_str()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(Json::Null);
                    if key == "config_json" {
                        row.insert(
                            "config_text_mentions_samourai".into(),
                            json!(raw.to_string().to_lowercase().contains("samourai")),
                        );
                    }
                    let mut facts = Map::new();
                    for key in [
                        "chain",
                        "network",
                        "bitcoin_network",
                        "chain_network",
                        "chain_instance_id",
                        "samourai",
                    ] {
                        if let Some(value) = parsed.get(key) {
                            let value = if key == "samourai" {
                                value
                                    .as_object()
                                    .map(|v| {
                                        Json::Object(
                                            v.iter()
                                                .filter(|(k, _)| {
                                                    ["role", "group_id", "section"]
                                                        .contains(&k.as_str())
                                                })
                                                .map(|(k, v)| (k.clone(), v.clone()))
                                                .collect(),
                                        )
                                    })
                                    .unwrap_or(Json::Null)
                            } else {
                                value.clone()
                            };
                            facts.insert(key.into(), value);
                        }
                    }
                    row.insert(key.into(), Json::String(Json::Object(facts).to_string()));
                }
            }
            Ok(row.into())
        })
        .collect()
}
pub fn blockers(conn: &Connection, profile: &Value) -> Result<Vec<Value>> {
    let id = text(profile, "id");
    let headers = records(conn,"SELECT * FROM custody_components WHERE profile_id=? AND state='active' ORDER BY created_at,revision,id",&id)?;
    if headers.is_empty() {
        return Ok(Vec::new());
    }
    let mut components = Vec::new();
    let mut anchor_ids = BTreeSet::new();
    let mut wallet_ids = BTreeSet::new();
    let mut revision_ids = BTreeSet::new();
    for header in headers {
        let cid = header["id"].as_str().expect("component id");
        let legs = records(
            conn,
            "SELECT * FROM custody_component_legs WHERE component_id=? ORDER BY ordinal,id",
            cid,
        )?;
        let allocations = records(
            conn,
            "SELECT * FROM custody_component_allocations WHERE component_id=? ORDER BY ordinal,id",
            cid,
        )?;
        let economic_terms = records(conn,"SELECT * FROM custody_component_economic_terms WHERE component_id=? ORDER BY ordinal,id",cid)?;
        let commitments = records(conn,"SELECT * FROM custody_component_evidence_commitments WHERE component_id=? ORDER BY ordinal,id",cid)?.into_iter().filter(|r|r.get("profile_id").and_then(Json::as_str)==Some(&id)).collect();
        for leg in &legs {
            for key in ["transaction_id", "anchor_transaction_id"] {
                if let Some(id) = leg.get(key).and_then(Json::as_str) {
                    anchor_ids.insert(id.to_owned());
                }
            }
            if let Some(id) = leg.get("wallet_id").and_then(Json::as_str) {
                wallet_ids.insert(id.to_owned());
            }
        }
        for key in ["supersedes_component_id", "superseded_by_component_id"] {
            if let Some(id) = header.get(key).and_then(Json::as_str) {
                revision_ids.insert(id.to_owned());
            }
        }
        components.push(Component {
            header,
            legs,
            allocations,
            economic_terms,
            commitments,
        });
    }
    let mut anchors = Map::new();
    for id in anchor_ids {
        let row = records(conn,"SELECT t.*,w.kind AS wallet_kind,w.config_json,observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash,observation.fee_attribution AS observation_fee_attribution,observation.application_revision AS observation_application_revision FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.id=?",&id)?.into_iter().next();
        anchors.insert(
            id,
            row.map(|row| Json::Object(row.into_map()))
                .unwrap_or(Json::Null),
        );
    }
    let mut anchor_wallets = Vec::new();
    for id in wallet_ids {
        anchor_wallets.extend(records(
            conn,
            "SELECT id,kind,config_json FROM wallets WHERE id=?",
            &id,
        )?);
    }
    let mut revision_headers = Vec::new();
    for id in revision_ids {
        revision_headers.extend(records(
            conn,
            "SELECT * FROM custody_components WHERE id=?",
            &id,
        )?);
    }
    // Start with an empty cache; transaction views are initialized by custody.
    let mut snapshot = Snapshot::default();
    snapshot.schema_version = 1;
    snapshot.operation = "custody".into();
    snapshot.as_of = "report-read".into();
    snapshot.profile = json_value(profile)?.as_object().cloned().unwrap().into();
    snapshot.components = components;
    snapshot.anchors = anchors;
    snapshot.anchor_wallets = anchor_wallets;
    snapshot.revision_headers = revision_headers;
    snapshot.transactions = records(conn,"SELECT t.*,w.kind AS wallet_kind,w.config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at,t.created_at,t.id",&id)?;
    // Python admission uses include_local_evidence=False.
    let computed = kassiber_custody::components::compute(&snapshot)
        .map_err(|e| AppError::new("internal_error", e.message))?;
    computed["blockers"]
        .as_array()
        .expect("component blockers")
        .iter()
        .map(|b| {
            let mut fields = Vec::new();
            for key in ["component_id", "lineage_id", "revision", "issue_codes"] {
                let v = kassiber_pytext::loads(&b[key].to_string())
                    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
                fields.push((key, v));
            }
            Ok(dict(fields))
        })
        .collect()
}
