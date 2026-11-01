use crate::{legacy_privacy, Result};
use kassiber_envelope::{dict, get};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

pub fn snapshot(conn: &Connection, args: &Value) -> Result<Value> {
    legacy_privacy::snapshot(conn, args)
        .map(ordered)
        .map_err(|mut error| {
            if get(error.details.as_ref().unwrap_or(&Value::Null), "limit").is_some() {
                if let Some(limit) = get(args, "limit") {
                    error.details = Some(dict([("limit", limit.clone())]));
                }
            }
            error
        })
}

fn ordered(value: Value) -> Value {
    match value {
        Value::List(items) => Value::List(items.into_iter().map(ordered).collect()),
        Value::Dict(fields) => {
            let value = Value::Dict(fields);
            let has = |key| get(&value, key).is_some();
            let keys: &[&str] = if has("summary") && has("coverage") {
                &[
                    "profile",
                    "scope",
                    "summary",
                    "coverage",
                    "wallets",
                    "transactions",
                    "findings",
                    "meta",
                ]
            } else if has("engine") {
                &[
                    "engine",
                    "local_only",
                    "egress",
                    "scope",
                    "phase2_deferred",
                    "context",
                ]
            } else if has("wallet_count") && has("state") {
                &[
                    "state",
                    "wallet_count",
                    "transaction_count",
                    "risk_weight",
                    "risk_count",
                    "unknown_count",
                    "risk_level",
                    "finding_counts",
                    "top_findings",
                ]
            } else if has("wallets_with_inventory") {
                &[
                    "wallet_count",
                    "wallets_with_inventory",
                    "inventory_outputs",
                    "active_utxos",
                    "transaction_total",
                    "transaction_full",
                    "transaction_partial",
                    "transaction_not_analysable",
                    "transaction_scored",
                ]
            } else if has("known_address_count") {
                &[
                    "known_address_count",
                    "reused_address_count",
                    "active_utxo_count",
                    "dust_utxo_count",
                    "script_type_counts",
                ]
            } else if has("known_input_values") {
                &[
                    "level",
                    "reason",
                    "input_count",
                    "output_count",
                    "known_input_values",
                    "known_output_values",
                ]
            } else if has("occurrences") {
                &[
                    "code",
                    "severity",
                    "scope",
                    "count",
                    "impact",
                    "occurrences",
                    "evidence_level",
                    "remediation",
                    "attribution",
                    "details",
                ]
            } else if has("risk_weight") && has("wallet_id") {
                &[
                    "id",
                    "wallet_id",
                    "wallet_label",
                    "occurred_at",
                    "direction",
                    "asset",
                    "state",
                    "support",
                    "risk_weight",
                    "risk_count",
                    "unknown_count",
                    "risk_level",
                    "finding_counts",
                    "top_findings",
                ]
            } else if has("risk_weight") && has("id") {
                &[
                    "id",
                    "label",
                    "kind",
                    "state",
                    "transaction_count",
                    "scored_transaction_count",
                    "inventory_output_count",
                    "active_utxo_count",
                    "address",
                    "risk_weight",
                    "risk_count",
                    "unknown_count",
                    "risk_level",
                    "finding_counts",
                    "top_findings",
                ]
            } else if has("positive") && has("critical") {
                &["positive", "info", "low", "medium", "high", "critical"]
            } else if has("wallet") && has("transaction") {
                &["wallet", "transaction"]
            } else if has("workspace") && has("profile") {
                &["workspace", "profile"]
            } else if has("id") && has("label") {
                &["id", "label"]
            } else if has("rule_version") && has("feature_codes") {
                &[
                    "rule_version",
                    "feature_codes",
                    "assumptions",
                    "contradictions",
                ]
            } else {
                &[]
            };
            let Value::Dict(fields) = value else {
                unreachable!()
            };
            let mut result: Vec<_> = keys
                .iter()
                .filter_map(|key| {
                    fields
                        .iter()
                        .find(|(k, _)| *k == (*key).into())
                        .map(|(key, value)| (key.clone(), ordered(value.clone())))
                })
                .collect();
            result.extend(
                fields
                    .into_iter()
                    .filter(|(key, _)| !keys.iter().any(|k| *key == (*k).into()))
                    .map(|(key, value)| (key, ordered(value))),
            );
            Value::Dict(result)
        }
        other => other,
    }
}
