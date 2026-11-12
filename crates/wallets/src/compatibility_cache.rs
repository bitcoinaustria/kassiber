//! Pure compatibility cache policy. Cached histories remain observations and
//! never grant ownership. Complete source-bound membership rounds may prepare
//! absence candidates for atomic application. No I/O or persistence here.
use kassiber_envelope::AppError;
use kassiber_pyjson::{dumps, Options};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Force a fetch without destroying the prior positive absence baseline.
/// Dependency observers keep their persisted tip and canonical txids, so a
/// forced rebuild refuses a backend behind the book and retracts only
/// against what it published before (`esplora_history.full_scan_checkpoint`).
pub(crate) fn full_scan_checkpoint(checkpoint: &Value) -> Value {
    let Some(checkpoint) = checkpoint.as_object() else {
        return json!({});
    };
    let mut retained = serde_json::Map::new();
    if let Some(saved) = checkpoint.get("esplora_history_memberships") {
        if saved.is_object() && saved["version"].as_u64() == Some(1) {
            retained.insert("esplora_history_memberships".into(), saved.clone());
        }
    }
    if let Some(instances) = checkpoint
        .get("observer_instances")
        .and_then(Value::as_object)
    {
        let baselines: BTreeMap<&String, serde_json::Map<String, Value>> = instances
            .iter()
            .filter_map(|(id, instance)| {
                let instance = instance.as_object()?;
                Some((
                    id,
                    ["tip_height", "canonical_txids"]
                        .into_iter()
                        .filter_map(|key| {
                            instance
                                .get(key)
                                .filter(|v| !v.is_null())
                                .map(|v| (key.to_owned(), v.clone()))
                        })
                        .collect(),
                ))
            })
            .collect();
        if !baselines.is_empty() {
            retained.insert(
                "observer_instances".into(),
                Value::Object(
                    baselines
                        .into_iter()
                        .map(|(id, baseline)| (id.clone(), Value::Object(baseline)))
                        .collect(),
                ),
            );
        }
    }
    Value::Object(retained)
}

pub(crate) fn digest(v: &Value) -> String {
    let py = kassiber_pytext::loads(&v.to_string()).expect("JSON value");
    let text = dumps(
        &py,
        &Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .expect("JSON value");
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key)
        .filter(|v| !v.is_null() && v.as_str() != Some(""))
        .or_else(|| v.get("config").and_then(|v| v.get(key)))
}
pub(crate) fn backend_identity(v: &Value, chain: &str, network: &str, batch: usize) -> Value {
    let kind = if v["kind"] == "mempool" {
        "esplora"
    } else {
        v["kind"].as_str().unwrap_or("")
    };
    json!({"name":v["name"],"kind":kind,"chain":chain,"network":network,"batch_size":batch})
}
pub(crate) fn esplora_key(v: &Value, identity: &Value) -> String {
    let auth = field(v, "auth_header")
        .and_then(Value::as_str)
        .map(kassiber_pytext::strip)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            field(v, "token")
                .and_then(Value::as_str)
                .map(kassiber_pytext::strip)
                .filter(|s| !s.is_empty())
                .map(|s| format!("Bearer {s}"))
        });
    let proxy = field(v, "tor_proxy")
        .or_else(|| field(v, "proxy"))
        .cloned()
        .unwrap_or(Value::Null);
    digest(
        &json!({"backend":identity,"url":v["url"],"headers":auth.map(|s|json!({"Authorization":s})),"proxy":proxy,"certificate":field(v,"certificate"),"insecure":field(v,"insecure").or_else(||field(v,"trust_ssl"))}),
    )
}
pub fn stats_fingerprint(stats: &Value) -> String {
    let mut all = serde_json::Map::new();
    for (dest, src) in [("chain", "chain_stats"), ("mempool", "mempool_stats")] {
        let fields = [
            "funded_txo_count",
            "funded_txo_sum",
            "spent_txo_count",
            "spent_txo_sum",
            "tx_count",
        ]
        .into_iter()
        .map(|k| (k.to_owned(), stats[src][k].clone()))
        .collect();
        all.insert(dest.into(), Value::Object(fields));
    }
    digest(&Value::Object(all))
}
fn canonical_id(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
pub(crate) fn memberships(saved: &Value, key: &str) -> BTreeMap<String, Value> {
    if saved["version"].as_u64() != Some(1) || saved["backend_key"] != key {
        return BTreeMap::new();
    }
    let Some(scripts) = saved["scripts"].as_object() else {
        return BTreeMap::new();
    };
    if scripts.iter().any(|(s, e)| {
        !canonical_id(&json!(s))
            || !e.is_object()
            || e["txids"]
                .as_array()
                .is_none_or(|ids| ids.iter().any(|id| !canonical_id(id)))
    }) {
        return BTreeMap::new();
    }
    scripts
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
pub(crate) fn esplora_reusable(
    previous: &Value,
    membership: Option<&Value>,
    fingerprint: &str,
) -> bool {
    previous.is_object()
        && previous["fingerprint"] == fingerprint
        && !crate::descriptors::truthy(&previous["mempool_dirty"])
        && membership.is_some_and(|m| m["complete"] == true && m["fingerprint"] == fingerprint)
}
pub(crate) fn electrum_reusable(
    checkpoint: &Value,
    hash: &str,
    status: &Value,
    same_tip: bool,
) -> bool {
    let previous = &checkpoint["electrum_scripthash_statuses"][hash];
    if status.is_null() && previous.is_null() {
        return true;
    }
    same_tip
        && crate::scan::integer(&checkpoint["electrum_stored_graph_version"])
            .is_some_and(|n| n >= 2.into())
        && status == previous
        && !checkpoint["electrum_dirty_scripthashes"]
            .as_array()
            .is_some_and(|v| v.contains(&json!(hash)))
}
pub(crate) fn stored_transaction(cache: &Value, id: &str) -> Option<Value> {
    let v = cache.get(id)?;
    let v = if let Some(s) = v.as_str() {
        serde_json::from_str(s).ok()?
    } else {
        v.clone()
    };
    let marker = &v["_kassiber_electrum_graph"];
    (v.is_object()
        && marker["kind"] == "bitcoin_electrum"
        && crate::scan::integer(&marker["version"]).is_some_and(|n| n == 2.into())
        && v["txid"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(id))
        && v["vin"].is_array()
        && v["vout"].is_array())
    .then_some(v)
}
pub(crate) fn changed_entries(
    history: &Value,
    prior: &Value,
) -> Result<(Value, Vec<Value>, bool), AppError> {
    let history = history
        .as_array()
        .ok_or_else(|| AppError::new("protocol_error", "Invalid Electrum history"))?;
    let mut current = serde_json::Map::new();
    let mut changed = vec![];
    let mut dirty = false;
    for item in history {
        if !item.is_object() {
            return Err(AppError::new(
                "protocol_error",
                "Invalid Electrum history row",
            ));
        }
        // Explicit historical timestamps also settle non-height histories.
        dirty |= !crate::scan::integer(&item["height"]).is_some_and(|n| n > 0.into())
            && !["confirmed_at", "timestamp", "time", "blocktime", "height"]
                .iter()
                .any(|key| {
                    let value = &item[key];
                    if kassiber_chain::parse_iso_time(&crate::descriptors::text(value)).is_some() {
                        return true;
                    }
                    *key != "height"
                        && !matches!(value, Value::Null)
                        && value.as_str() != Some("")
                        && value.as_str() != Some("0")
                        && value.as_f64() != Some(0.0)
                        && value != &json!(false)
                        && crate::scan::integer(value)
                            .and_then(|n| i64::try_from(n).ok())
                            .and_then(kassiber_chain::format_unix_time)
                            .is_some()
                });
        let Some(id) = item["tx_hash"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        current.insert(id.into(), item.clone());
        if prior[id] != *item {
            changed.push(item.clone());
        }
    }
    Ok((Value::Object(current), changed, dirty))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_cache_fingerprints_cover_every_statistic_and_backend_identity() {
        let stats = json!({"chain_stats":{"funded_txo_count":1,"funded_txo_sum":2,"spent_txo_count":3,"spent_txo_sum":4,"tx_count":5},"mempool_stats":{"funded_txo_count":11,"funded_txo_sum":12,"spent_txo_count":13,"spent_txo_sum":14,"tx_count":15}});
        assert_eq!(
            stats_fingerprint(&stats),
            "6f83e868dc086534b09d67b1332eb04acff48e5ad727c52b994e6d41b6e12986"
        );
        for section in ["chain_stats", "mempool_stats"] {
            for key in [
                "funded_txo_count",
                "funded_txo_sum",
                "spent_txo_count",
                "spent_txo_sum",
                "tx_count",
            ] {
                let mut other = stats.clone();
                other[section][key] = json!(99);
                assert_ne!(stats_fingerprint(&other), stats_fingerprint(&stats));
            }
        }
        let config = json!({"kind":"mempool","name":"one","url":"http://example.invalid"});
        let identity = backend_identity(&config, "bitcoin", "regtest", 7);
        assert_eq!(
            identity,
            json!({"kind":"esplora","name":"one","chain":"bitcoin","network":"regtest","batch_size":7})
        );
        for key in ["name", "kind", "chain", "network", "batch_size"] {
            let mut other = identity.clone();
            other[key] = json!("other");
            assert_ne!(
                esplora_key(&config, &other),
                esplora_key(&config, &identity)
            );
        }
    }
    #[test]
    fn compatibility_cache_explicit_times_match_python_recheck_policy() {
        for value in [
            json!(null),
            json!(""),
            json!(0),
            json!(0.0),
            json!(false),
            json!("0"),
            json!("malformed"),
            json!(1e30),
            json!(1_000_000_000_000i64),
        ] {
            let h = json!([{"tx_hash":"a", "height":0, "timestamp":value}]);
            assert!(changed_entries(&h, &json!({})).unwrap().2, "{value}");
        }
        for value in [
            json!("2024-01-02T03:04:05Z"),
            json!("2024-W01-2"),
            json!(20240102),
            json!(" 1700000000 "),
            json!(1700000000.9),
            json!(true),
            json!("00"),
            json!(0.5),
        ] {
            let h = json!([{"tx_hash":"a", "height":0, "timestamp":value}]);
            assert!(!changed_entries(&h, &json!({})).unwrap().2, "{value}");
        }
        let h = json!([{"tx_hash":"a", "height":"2024-01-02"}]);
        assert!(!changed_entries(&h, &json!({})).unwrap().2);
        let h =
            json!([{"tx_hash":"a", "height":0, "timestamp":"malformed", "blocktime":1700000000}]);
        assert!(!changed_entries(&h, &json!({})).unwrap().2);
    }
    #[test]
    fn compatibility_cache_binds_membership_source_and_dirty_history() {
        let stats = json!({"chain_stats":{"tx_count":1},"mempool_stats":{"tx_count":0}});
        let f = stats_fingerprint(&stats);
        let p = json!({"fingerprint":f,"mempool_dirty":false});
        let m = json!({"fingerprint":f,"complete":true});
        assert!(esplora_reusable(&p, Some(&m), &f));
        assert!(!esplora_reusable(&p, None, &f));
        assert!(!esplora_reusable(
            &p,
            Some(&json!({"fingerprint":f,"complete":false})),
            &f
        ));
        assert!(!esplora_reusable(&p, Some(&m), "changed"));
        assert!(!esplora_reusable(
            &json!({"fingerprint":"other","mempool_dirty":false}),
            Some(&m),
            &f
        ));
        assert!(!esplora_reusable(
            &p,
            Some(&json!({"fingerprint":"other","complete":true})),
            &f
        ));
        assert!(!esplora_reusable(
            &json!({"fingerprint":f,"mempool_dirty":true}),
            Some(&m),
            &f
        ));
        let key = "a".repeat(64);
        let saved = json!({"version":1,"backend_key":"k","scripts":{key.clone():{"txids":[],"complete":true}}});
        assert_eq!(memberships(&saved, "k").len(), 1);
        assert!(memberships(&saved, "other").is_empty());
        let mut bad = saved.clone();
        bad["version"] = json!(true);
        assert!(memberships(&bad, "k").is_empty());
        let mut bad = saved.clone();
        bad["scripts"][&key]["txids"] = json!(["malformed"]);
        assert!(memberships(&bad, "k").is_empty());
        for field in [
            "url",
            "certificate",
            "insecure",
            "proxy",
            "auth_header",
            "token",
        ] {
            let base = json!({"kind":"esplora","name":"one","url":"http://example.invalid"});
            let identity = backend_identity(&base, "bitcoin", "main", 2);
            let key = esplora_key(&base, &identity);
            let mut other = base.clone();
            other[field] = json!("different");
            assert_ne!(esplora_key(&other, &identity), key);
        }
        let cp = json!({"electrum_stored_graph_version":2,"electrum_scripthash_statuses":{"h":"s"},"electrum_history_entries":{"h":{}},"electrum_dirty_scripthashes":[]});
        assert!(electrum_reusable(&cp, "h", &json!("s"), true));
        assert!(!electrum_reusable(&cp, "h", &json!("s"), false));
        assert!(electrum_reusable(&json!({}), "empty", &Value::Null, false));
        assert!(!electrum_reusable(&cp, "h", &json!("changed"), true));
        let mut old = cp.clone();
        old["electrum_stored_graph_version"] = json!(0);
        assert!(!electrum_reusable(&old, "h", &json!("s"), true));
        // Version-1 graphs predate the raw txid check; they rebuild once.
        old["electrum_stored_graph_version"] = json!(1);
        assert!(!electrum_reusable(&old, "h", &json!("s"), true));
        let mut dirty = cp.clone();
        dirty["electrum_dirty_scripthashes"] = json!(["h"]);
        assert!(!electrum_reusable(&dirty, "h", &json!("s"), true));
        assert!(electrum_reusable(&json!({}), "empty", &Value::Null, true));
    }
    #[test]
    fn compatibility_cache_only_reuses_versioned_bound_graphs_and_changed_entries() {
        let id = "a".repeat(64);
        let tx = json!({"txid":id,"vin":[],"vout":[],"_kassiber_electrum_graph":{"kind":"bitcoin_electrum","version":2}});
        let cache = json!({id.clone():tx.clone()});
        assert_eq!(stored_transaction(&cache, &id), Some(tx.clone()));
        assert_eq!(
            stored_transaction(&json!({id.clone():tx.to_string()}), &id),
            Some(tx.clone())
        );
        for (key, value) in [
            ("txid", json!("other")),
            ("vin", Value::Null),
            ("vout", Value::Null),
            (
                "_kassiber_electrum_graph",
                json!({"kind":"bitcoin_electrum","version":1}),
            ),
            (
                "_kassiber_electrum_graph",
                json!({"kind":"bitcoin_electrum","version":3}),
            ),
            (
                "_kassiber_electrum_graph",
                json!({"kind":"foreign","version":2}),
            ),
        ] {
            let mut bad = tx.clone();
            bad[key] = value;
            assert!(stored_transaction(&json!({id.clone():bad}), &id).is_none());
        }
        let prior = json!({id.clone():{"tx_hash":id,"height":5}});
        let history = json!([{"tx_hash":id,"height":5}]);
        let (_, changes, dirty) = changed_entries(&history, &prior).unwrap();
        assert!(changes.is_empty());
        assert!(!dirty);
        let history = json!([{"tx_hash":id,"height":0}]);
        let (_, changes, dirty) = changed_entries(&history, &prior).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(dirty);
        let (_, changes, dirty) = changed_entries(&json!([{"height":0}]), &json!({})).unwrap();
        assert!(changes.is_empty());
        assert!(dirty);
        assert!(changed_entries(&json!([1]), &json!({})).is_err());
    }
}
