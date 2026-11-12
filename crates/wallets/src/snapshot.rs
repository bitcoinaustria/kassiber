//! Safe desktop wallet-list projection. Reads never probe configured backends.
use crate::{
    config, database_error,
    descriptors::{text, truthy},
    lifecycle::projection,
};
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value as Py;
use kassiber_store::rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn string(c: &Value, key: &str) -> String {
    c.get(key)
        .filter(|v| truthy(v))
        .map(text)
        .unwrap_or_default()
}
fn stripped(c: &Value, key: &str) -> String {
    c.get(key)
        .and_then(Value::as_str)
        .map(kassiber_pytext::strip)
        .unwrap_or_default()
        .to_owned()
}
fn boolean(c: &Value, key: &str) -> Py {
    Py::Bool(c.get(key).is_some_and(truthy))
}
fn value(c: &Value, key: &str) -> Py {
    projection(c.get(key).unwrap_or(&Value::Null))
}
fn txid(v: &Value) -> Option<Value> {
    let raw = text(v);
    let raw = kassiber_pytext::strip(&raw).to_lowercase();
    (raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit())).then(|| json!(raw))
}
fn integer(v: &Value) -> Option<Value> {
    let value = match v {
        Value::Bool(b) => json!(i64::from(*b)),
        Value::Number(n) if n.is_i64() || n.is_u64() => v.clone(),
        Value::Number(n) => serde_json::from_str(&format!("{:.0}", n.as_f64()?.trunc())).ok()?,
        Value::String(s) => {
            serde_json::from_str(&kassiber_pytext::parse_int(s).ok()?.to_string()).ok()?
        }
        _ => return None,
    };
    (!value.to_string().starts_with('-')).then_some(value)
}
fn samourai(c: &Value) -> Value {
    let Some(metadata) = c.get("samourai").and_then(Value::as_object) else {
        return Value::Null;
    };
    let mut out = json!({});
    for (key, v) in metadata {
        let normalized = match key.as_str() {
            "gap_limit"
            | "minimum_mix_count"
            | "mix_count"
            | "target_mix_count"
            | "pool_denomination_sat"
            | "coordinator_fee_sat"
            | "miner_fee_sat"
            | "ricochet_hops" => integer(v),
            "privacy_boundary" | "whirlpool" | "toxic_change" | "watch_only" | "paynym" => {
                Some(json!(truthy(v)))
            }
            "round_txid" => txid(v),
            "round_txids" => v
                .as_array()
                .map(|vs| vs.iter().filter_map(txid).collect::<Vec<_>>())
                .filter(|vs| !vs.is_empty())
                .map(|v| json!(v)),
            "sections" => v
                .as_array()
                .map(|vs| {
                    vs.iter()
                        .map(|v| kassiber_pytext::strip(&text(v)).to_lowercase())
                        .filter(|s| {
                            ["deposit", "badbank", "premix", "postmix", "ricochet"]
                                .contains(&s.as_str())
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|vs| !vs.is_empty())
                .map(|v| json!(v)),
            "mix_count_confidence"
            | "tx0_role"
            | "whirlpool_event"
            | "privacy_event"
            | "exit_kind" => {
                let raw = if truthy(v) { text(v) } else { String::new() };
                let raw = kassiber_pytext::strip(&raw).to_lowercase();
                let allowed: &[&str] = match key.as_str() {
                    "mix_count_confidence" => &["minimum", "exact", "estimated", "unknown"],
                    "tx0_role" => &["deposit", "premix", "badbank", "fee"],
                    "whirlpool_event" => &[
                        "tx0",
                        "premix_pending",
                        "first_mix",
                        "remix",
                        "mix_to_wallet",
                        "external_spend",
                    ],
                    "privacy_event" => &[
                        "coinjoin",
                        "payjoin",
                        "tx0",
                        "first_mix",
                        "remix",
                        "ricochet",
                        "exit",
                    ],
                    _ => &[
                        "cold_storage",
                        "external_spend",
                        "ricochet",
                        "toxic_change_spend",
                    ],
                };
                allowed.contains(&raw.as_str()).then(|| json!(raw))
            }
            "role"
            | "group_id"
            | "group_label"
            | "parent_wallet_id"
            | "source"
            | "section"
            | "script_type"
            | "root_path"
            | "bip47"
            | "scanned_without_explicit_descriptor" => match v {
                Value::String(v) => {
                    let v = kassiber_pytext::strip(v);
                    (!v.is_empty()).then(|| json!(v.chars().take(128).collect::<String>()))
                }
                Value::Bool(_) => Some(v.clone()),
                Value::Number(_)
                    if v.is_i64() || v.is_u64() || !v.to_string().contains(['.', 'e', 'E']) =>
                {
                    Some(v.clone())
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(v) = normalized {
            out[key] = v;
        }
    }
    if out.as_object().unwrap().is_empty() {
        Value::Null
    } else {
        out
    }
}

pub fn snapshot(
    conn: &Connection,
    default_backend: Option<&str>,
    backend_kinds: &BTreeMap<String, String>,
) -> Result<Py, AppError> {
    let workspace = conn.query_row("SELECT id,label FROM workspaces WHERE id=(SELECT value FROM settings WHERE key='context_workspace')", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(database_error)?;
    let profile = conn.query_row("SELECT id,label,workspace_id,last_processed_at,last_processed_tx_count,journal_input_version,last_processed_input_version,required_journal_semantics_version,journal_semantics_version FROM profiles WHERE id=(SELECT value FROM settings WHERE key='context_profile')", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?,r.get::<_,i64>(6)?,r.get::<_,i64>(7)?,r.get::<_,i64>(8)?))).optional().map_err(database_error)?;
    let Some((workspace, profile)) = workspace.zip(profile).filter(|(w, p)| w.0 == p.2) else {
        return Ok(dict([
            ("wallets", Py::List(vec![])),
            (
                "summary",
                dict([
                    ("workspace", Py::Null),
                    ("profile", Py::Null),
                    ("count", 0.into()),
                    ("transaction_count", 0.into()),
                    ("needs_journals", Py::Bool(false)),
                ]),
            ),
        ]));
    };
    let active_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [&profile.0],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let stale = active_count > 0
        && (profile.3.as_deref().is_none_or(str::is_empty)
            || profile.4 != active_count
            || profile.5 != profile.6
            || !kassiber_store::semantics::compatible(profile.7, profile.8));
    let mut stmt = conn.prepare("SELECT w.id,w.label,w.kind,w.config_json,w.created_at,a.code,a.label,COUNT(t.id),MAX(t.occurred_at) FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id LEFT JOIN transactions t ON t.wallet_id=w.id AND t.excluded=0 WHERE w.profile_id=? GROUP BY w.id ORDER BY w.label ASC").map_err(database_error)?;
    let rows = stmt
        .query_map([&profile.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    // How complete each source can be, and the edges of what it has observed.
    let completeness = crate::completeness::describe(
        conn,
        &profile.0,
        &rows
            .iter()
            .map(|row| crate::completeness::Wallet {
                id: row.0.clone(),
                kind: s(row.2.as_str()),
                config_json: row.3.as_deref().map_or(Py::Null, s),
            })
            .collect::<Vec<_>>(),
    )?;
    let mut wallets = Vec::new();
    let mut total = 0;
    for (id, label, kind, raw, created, account_code, account_label, count, last) in rows {
        let (completeness_class, scan_bounds) = completeness[&id].clone();
        let c = raw
            .as_deref()
            .and_then(|v| serde_json::from_str::<Value>(v).ok())
            .filter(Value::is_object)
            .unwrap_or(json!({}));
        let mut backend = stripped(&c, "backend");
        let mut source = if backend.is_empty() {
            "none"
        } else {
            "explicit"
        };
        let has_descriptor = config::optional_text(c.get("descriptor")).is_some()
            || (config::optional_text(c.get("xpub")).is_some()
                && c.get("script_types").is_some_and(truthy));
        let sp = config::optional_text(c.get("sp_descriptor")).is_some();
        let mode = if stripped(&c, "sync_source") == "btcpay" {
            "btcpay"
        } else if !stripped(&c, "source_file").is_empty()
            && !stripped(&c, "source_format").is_empty()
        {
            "file_import"
        } else if sp && kind == "silent-payment" {
            "backend_silent_payment"
        } else if has_descriptor && ["descriptor", "xpub", "address"].contains(&kind.as_str()) {
            "backend_descriptor"
        } else if c.get("addresses").is_some_and(truthy) && kind == "address" {
            "backend_addresses"
        } else {
            "not_configured"
        };
        if backend.is_empty() && ["backend_descriptor", "backend_addresses"].contains(&mode) {
            if let Some(default) = default_backend
                .map(kassiber_pytext::strip)
                .filter(|v| !v.is_empty())
            {
                backend = default.to_owned();
                source = "default";
            }
        }
        let backend_kind = backend_kinds.get(&backend).cloned().unwrap_or_default();
        let sync = stripped(&c, "last_synced_at");
        let types = match c.get("script_types") {
            Some(Value::Array(v)) => v
                .iter()
                .filter(|v| v.is_string())
                .cloned()
                .collect::<Vec<_>>(),
            Some(Value::String(v)) => v.chars().map(|c| json!(c.to_string())).collect(),
            Some(Value::Object(v)) => v.keys().map(|v| json!(v)).collect(),
            Some(v) if truthy(v) => {
                return Err(AppError::new(
                    "internal_error",
                    format!(
                        "'{}' object is not iterable",
                        if v.is_boolean() {
                            "bool"
                        } else if v.is_i64() || v.is_u64() {
                            "int"
                        } else {
                            "float"
                        }
                    ),
                ))
            }
            _ => vec![],
        };
        let routes = config::provenance(c.get("btcpay_provenance").unwrap_or(&Value::Null))?;
        let routes = Py::List(
            routes
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    dict([
                        ("backend", s(string(r, "backend"))),
                        ("store_id", s(string(r, "store_id"))),
                        ("payment_method_id", s(string(r, "payment_method_id"))),
                    ])
                })
                .collect(),
        );
        wallets.push(dict([
            ("id", s(id)),
            ("label", s(label)),
            ("kind", s(kind)),
            (
                "account",
                dict([
                    ("code", s(account_code.unwrap_or_default())),
                    ("label", s(account_label.unwrap_or_default())),
                ]),
            ),
            (
                "backend",
                dict([
                    ("name", s(backend)),
                    ("source", s(source)),
                    ("kind", s(backend_kind)),
                ]),
            ),
            ("chain", s(string(&c, "chain"))),
            ("network", s(string(&c, "network"))),
            ("birthday", s(string(&c, "birthday"))),
            ("descriptor", boolean(&c, "descriptor")),
            ("change_descriptor", boolean(&c, "change_descriptor")),
            (
                "silent_payment",
                dict([
                    ("configured", Py::Bool(sp)),
                    ("material_format", s(string(&c, "sp_material_format"))),
                    ("scan_mode", s(string(&c, "sp_scan_mode"))),
                    ("scan_start_height", value(&c, "sp_scan_start_height")),
                    ("scan_start_date", s(string(&c, "sp_scan_start_date"))),
                    ("full_history", boolean(&c, "sp_full_history")),
                ]),
            ),
            ("sync_mode", s(mode)),
            (
                "sync_source",
                s(if c.get("sync_source").is_some_and(truthy) {
                    string(&c, "sync_source")
                } else {
                    string(&c, "source_format")
                }),
            ),
            ("transaction_count", count.into()),
            ("last_transaction_at", last.map_or(Py::Null, s)),
            (
                "last_synced_at",
                if sync.is_empty() { Py::Null } else { s(sync) },
            ),
            (
                "sync_status",
                s(if count > 0 {
                    "has_transactions"
                } else {
                    "empty"
                }),
            ),
            (
                "deprecated",
                Py::Bool(config::boolean(c.get("deprecated"))?),
            ),
            ("journals_stale", Py::Bool(stale && count > 0)),
            ("btcpay_provenance", routes),
            ("samourai", projection(&samourai(&c))),
            ("script_types", projection(&json!(types))),
            ("created_at", s(created)),
            ("completeness_class", completeness_class),
            ("scan_bounds", scan_bounds),
        ]));
        total += count;
    }
    let count = wallets.len();
    Ok(dict([
        ("wallets", Py::List(wallets)),
        (
            "summary",
            dict([
                ("workspace", s(workspace.1)),
                ("profile", s(profile.1)),
                ("count", count.into()),
                ("transaction_count", total.into()),
                ("needs_journals", Py::Bool(stale)),
            ]),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_wallet_snapshot_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        let schema: Value =
            serde_json::from_str(include_str!("../../store/fixtures/creation.json")).unwrap();
        for case in vectors["snapshots"].as_array().unwrap() {
            let conn = Connection::open_in_memory().unwrap();
            for sql in schema["statements"]
                .as_array()
                .unwrap()
                .iter()
                .chain(case["statements"].as_array().unwrap())
            {
                conn.execute_batch(sql.as_str().unwrap()).unwrap();
            }
            let kinds = case["runtime"]["backends"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v["kind"].as_str().unwrap().to_owned()))
                .collect();
            let result =
                snapshot(&conn, case["runtime"]["default_backend"].as_str(), &kinds).unwrap();
            let result: Value = serde_json::from_str(
                &kassiber_pyjson::dumps(&result, &Default::default()).unwrap(),
            )
            .unwrap();
            assert_eq!(result, case["expected"]["ok"], "{}", case["name"]);
        }
    }
}
