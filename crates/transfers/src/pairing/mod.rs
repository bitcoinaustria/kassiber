//! Automatic pairing for the journal pipeline. No command is registered here.
//! Evidence is country-neutral; policies and durable reviews are applied later.
mod htlc;
mod matcher;
mod provider;
mod service;

pub use matcher::{
    dismissal_is_active, exclusion_counts, finalize, suggest, Candidate, MatchOptions, Population,
};
pub use service::{
    apply_rules, auto_pair_before_journals, auto_pair_before_journals_captured, bulk_pair,
    recount_candidates, suggestions, AutomaticPairingError, PairingOptions, ProjectionGuards,
};

use super::*;
use kassiber_envelope::{set, truthy};
use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Dict(pairs) => pairs
            .iter()
            .rev()
            .find_map(|(key_value, value)| match key_value {
                kassiber_pyjson::Key::Str(name) if name.is(key) => Some(value),
                _ => None,
            }),
        _ => None,
    }
}
fn value(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}

// Most observations contain ordinary UTF-8. Preserve the envelope's surrogate
// fallback while copying ordinary strings without decoding every code point.
fn py_str(v: &Value) -> String {
    match v {
        Value::Str(v) if v.as_str().is_some() => v.as_str().unwrap().to_owned(),
        v => kassiber_envelope::py_str(v),
    }
}
fn text(v: &Value, k: &str) -> String {
    get(v, k).map(py_str).unwrap_or_default()
}
fn s(v: impl AsRef<str>) -> Value {
    kassiber_envelope::s(v.as_ref())
}
fn num(n: i128) -> Value {
    Value::Int(n.into())
}
fn n(v: &Value) -> Result<i128> {
    integer(v)?.to_string().parse().map_err(|_| {
        AppError::new(
            "validation",
            "Pairing amount exceeds the exact signed-128 range",
        )
    })
}
fn count(v: usize) -> Value {
    Value::Int(v.into())
}
fn list(v: &Value) -> &[Value] {
    if let Value::List(v) = v {
        v
    } else {
        &[]
    }
}
fn st(v: &Value, k: &str) -> String {
    get(v, k)
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default()
}
fn lower(v: &str) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(v))
}
fn upper(v: &str) -> String {
    kassiber_pytext::upper(kassiber_pytext::strip(v))
}
fn hex_id(v: &str) -> Option<String> {
    let v = kassiber_pytext::strip(v);
    (v.len() == 64 && v.bytes().all(|c| c.is_ascii_hexdigit())).then(|| v.to_ascii_lowercase())
}
fn instant(v: &Value) -> Option<f64> {
    kassiber_custody::components::review::matching_instant(&kassiber_custody::codec::to_json(v))
}
fn kind_alias(v: &str) -> String {
    match lower(v).replace('_', "-").as_str() {
        "core-ln" | "core-lightning" => "coreln".into(),
        v => v.into(),
    }
}
fn lightning(v: &str) -> bool {
    matches!(
        v,
        "cln" | "coreln" | "lightning" | "lnd" | "nwc" | "phoenix"
    )
}
fn chain(v: &str) -> bool {
    matches!(
        v,
        "address" | "descriptor" | "samourai" | "silent-payment" | "wasabi" | "xpub"
    )
}
fn asset(v: &str) -> Option<&'static str> {
    match upper(v).as_str() {
        "BTC" | "XBT" | "XXBT" => Some("BTC"),
        "LBTC" | "L-BTC" => Some("LBTC"),
        _ => None,
    }
}
fn default_kind(o: &str, i: &str, ow: &str, iw: &str) -> &'static str {
    let (Some(o), Some(i)) = (asset(o), asset(i)) else {
        return "manual";
    };
    let ow = kind_alias(ow);
    let iw = kind_alias(iw);
    if lightning(&ow) && !lightning(&iw) {
        "reverse-submarine-swap"
    } else if lightning(&iw) && !lightning(&ow) {
        "submarine-swap"
    } else if chain(&ow) && chain(&iw) && o == "BTC" && i == "LBTC" {
        "peg-in"
    } else if chain(&ow) && chain(&iw) && o == "LBTC" && i == "BTC" {
        "peg-out"
    } else {
        "manual"
    }
}
fn compatible(o: &Option<String>, i: &Option<String>) -> bool {
    o.is_none() || i.is_none() || o == i
}
// Scalar text and graph identity are independent of object iteration order.
// Python's str(container) in a provider text field is the rare exception.
fn ordered_provider_text(raw: &serde_json::Value) -> bool {
    raw.as_object().is_some_and(|fields| {
        fields.iter().any(|(key, value)| {
            (value.is_object() || value.is_array())
                && matches!(
                    key.as_str(),
                    "provider"
                        | "source"
                        | "source_format"
                        | "swap_id"
                        | "swapId"
                        | "swapID"
                        | "provider_swap_id"
                        | "boltz_id"
                        | "boltzId"
                        | "id"
                        | "flow"
                        | "type"
                        | "swap_type"
                        | "swapType"
                        | "kind"
                        | "status"
                        | "state"
                        | "finality"
                        | "send_txid"
                        | "sendTxid"
                        | "lockup_txid"
                        | "lockupTxid"
                        | "receive_txid"
                        | "receiveTxid"
                        | "claim_txid"
                        | "claimTxid"
                        | "refund_txid"
                        | "refundTxid"
                        | "version"
                        | "swap_version"
                        | "swapVersion"
                        | "taproot"
                        | "is_taproot"
                        | "isTaproot"
                        | "cooperative"
                        | "cooperative_claim"
                        | "cooperativeClaim"
                        | "spend_path"
                        | "spendPath"
                        | "claim_path"
                        | "claimPath"
                )
        })
    })
}
fn payload(row: &Value) -> Value {
    let raw = value(row, "raw_json");
    if matches!(raw, Value::Dict(_)) {
        raw
    } else if truthy(&raw) {
        let text = py_str(&raw);
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .filter(|raw| !ordered_provider_text(raw))
            .and_then(|v| kassiber_custody::codec::from_json(&v).ok())
            .or_else(|| parse_json(&text).ok())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

// The shared chain interpreter accepts parsed payloads. Preserve its treatment
// of Python's nonfinite JSON scalars while avoiding three reparses per row.
fn identity_payload(v: &Value) -> serde_json::Value {
    match v {
        Value::Float(n) if !n.is_finite() => {
            serde_json::Value::String(kassiber_pyjson::float_repr(*n))
        }
        Value::List(v) => serde_json::Value::Array(v.iter().map(identity_payload).collect()),
        Value::Dict(v) => serde_json::Value::Object(
            v.iter()
                .filter_map(|(k, v)| {
                    if let kassiber_pyjson::Key::Str(k) = k {
                        Some((k.as_str()?.to_owned(), identity_payload(v)))
                    } else {
                        None
                    }
                })
                .collect(),
        ),
        v => kassiber_custody::codec::to_json(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_field_lookup_preserves_exact_keys_and_last_duplicate() {
        use kassiber_pyjson::{Key, PyStr};
        let row = Value::Dict(vec![
            (Key::Str("amount".into()), 1.into()),
            (Key::Str("é 钱包".into()), 2.into()),
            (Key::Int(1.into()), 3.into()),
            (Key::Str("amount".into()), 4.into()),
            (Key::Str("amount_extra".into()), 5.into()),
            (
                Key::Str(PyStr::from_code_points([0xd800]).unwrap()),
                6.into(),
            ),
        ]);
        for key in ["amount", "amount_extra", "é 钱包", "1", "missing", "", "�"] {
            assert_eq!(get(&row, key), kassiber_envelope::get(&row, key));
            assert_eq!(value(&row, key), super::super::value(&row, key));
        }
        assert_eq!(get(&row, "amount"), Some(&4.into()));
        assert_eq!(get(&Value::Null, "amount"), None);
    }

    #[test]
    fn pairing_encoded_provider_metadata_preserves_python_mapping_order() {
        // Python accepts non-scalar provider text fields through str(). Their
        // insertion order survives json.loads and is observable in suggestions.
        let raw =
            r#"{"provider":"boltz","swap_id":"ordered","flow":"chain","version":{"z":1,"a":2}}"#;
        let row = dict([("raw_json", s(raw))]);
        let evidence = provider::extract(&payload(&row)).unwrap().unwrap();
        assert_eq!(evidence.version, "{'z': 1, 'a': 2}");
    }
}
