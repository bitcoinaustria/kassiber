use super::super::value::{json, Value};
use kassiber_envelope::{py_str, AppError};
use kassiber_pyjson::{BigInt, Value as Py};
use std::collections::BTreeSet;

pub(super) fn py(v: &Value) -> Py {
    v.py()
}
pub(super) fn json(v: &Py) -> Result<Value, AppError> {
    Ok(Value::from_py(v))
}
pub(super) fn text(v: &Value) -> String {
    match v {
        Value::String(v) => v
            .code_points()
            .map(|c| char::from_u32(c).unwrap_or('\u{fffd}'))
            .collect(),
        _ => py_str(&py(v)),
    }
}
pub(super) fn truth(v: &Value) -> bool {
    kassiber_envelope::truthy(&py(v))
}
pub(super) fn or_text(v: &Value) -> String {
    if truth(v) {
        text(v)
    } else {
        String::new()
    }
}
pub(super) fn integer(v: &Value) -> Option<BigInt> {
    if v.is_boolean() {
        return None;
    }
    let n = crate::scan::integer(&v.serde())?;
    (n >= BigInt::from(0) && kassiber_pytext::strip(&text(v)) == n.to_string()).then_some(n)
}
pub(super) fn nonnegative(v: &Value) -> bool {
    matches!(v, Value::Int(n) if n >= &BigInt::from(0))
}
pub(super) fn canonical(v: &Value) -> Option<String> {
    let text = text(v);
    let text = kassiber_pytext::strip(&text);
    (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}
pub(super) fn mapping(v: &Value, nested: bool) -> Value {
    parsed_mapping(v, nested).unwrap_or_else(|| json!({}))
}
pub(super) fn parsed_mapping(v: &Value, nested: bool) -> Option<Value> {
    crate::publication::physical::stored_tx_mapping(&py(v), nested).and_then(|v| json(&v).ok())
}
pub(super) fn validate_mappings(row: &Value, names: &[&str]) -> Result<(), AppError> {
    for name in names {
        if let Some(value) =
            crate::publication::physical::stored_tx_mapping(&py(&row[*name]), false)
        {
            json(&value)?;
        }
    }
    Ok(())
}
pub(super) fn digest(v: &Value) -> Result<String, AppError> {
    super::super::digest(&py(v))
}
pub(super) fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub(super) fn script(entry: &Value) -> Option<String> {
    let nested = &entry["scriptPubKey"];
    let value = [
        &entry["scriptpubkey"],
        &entry["script_hex"],
        if nested.is_object() {
            if truth(&nested["hex"]) {
                &nested["hex"]
            } else {
                &nested["scriptpubkey"]
            }
        } else if nested.is_string() {
            nested
        } else {
            &Value::Null
        },
    ]
    .into_iter()
    .find(|v| !v.is_null())?;
    if value == "" {
        return None;
    }
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&text(value)));
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut high = true;
    for c in text.bytes() {
        if c.is_ascii_hexdigit() {
            high = !high;
        } else if !(high && matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 11 | 12)) {
            return None;
        }
    }
    high.then_some(text)
}
pub(super) fn address(entry: &Value) -> Value {
    let mut value = if truth(&entry["scriptpubkey_address"]) {
        &entry["scriptpubkey_address"]
    } else {
        &entry["address"]
    };
    if value.is_null() || value == "" {
        let nested = &entry["scriptPubKey"];
        if nested.is_object() {
            value = &nested["address"];
            if value.is_null() || value == "" {
                if let Some(first) = array(&nested["addresses"]).first() {
                    value = first;
                }
            }
        }
    }
    if value.is_null() || value == "" {
        return Value::Null;
    }
    let text = string(value);
    let text = kassiber_pytext::strip_py(&text);
    if text.code_points().next().is_none() {
        Value::Null
    } else {
        json!(text)
    }
}
pub(super) fn branch(label: &Value, index: &Value) -> Value {
    static ALNUM: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let alnum = ALNUM.get_or_init(|| {
        regex::Regex::new(&format!("^{}$", kassiber_pytext::REGEX_WORD))
            .expect("Unicode 14 word table")
    });
    let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&or_text(label)))
        .chars()
        .map(|c| {
            if c != '_' && alnum.is_match(c.encode_utf8(&mut [0; 4])) {
                c
            } else {
                ' '
            }
        })
        .collect::<String>();
    let mut tokens = normalized.split_whitespace().collect::<BTreeSet<_>>();
    let compact = normalized.replace(' ', "");
    tokens.insert(&compact);
    let role = if tokens.contains("change") || tokens.contains("internal") {
        Some("change")
    } else if ["receive", "external", "deposit"]
        .iter()
        .any(|v| tokens.contains(v))
    {
        Some("receive")
    } else {
        None
    };
    if let Some(role) = role {
        let imported = ["p2pkh", "p2shp2wpkh", "p2sh", "p2wpkh", "p2tr"]
            .iter()
            .any(|v| tokens.contains(v));
        return json!({"branch_role":role,"branch_evidence_level":"exact","change_evidence":if imported {"imported"} else {"ground_truth"},"branch_source":if imported {"imported_branch_role"} else {"wallet_branch_role"}});
    }
    let number = if index.is_boolean() {
        None
    } else {
        crate::scan::integer(&index.serde())
            .filter(|n| kassiber_pytext::strip(&text(index)) == n.to_string())
    };
    let role = number
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n < 8)
        .map(|n| if n % 2 == 0 { "receive" } else { "change" });
    json!({"branch_role":role.unwrap_or("unknown"),"branch_evidence_level":if role.is_some() {"derived"} else {"unknown"},"change_evidence":if role.is_some() {"heuristic"} else {"unavailable"},"branch_source":if role.is_some() {"numeric_branch_convention"} else {"branch_metadata_unavailable"}})
}

fn marker(v: &Value) -> Option<String> {
    if v.is_null() {
        return None;
    }
    let text = text(v);
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&text)).replace('-', "_");
    (!text.is_empty()).then(|| {
        if [
            "coinjoin",
            "collaborative",
            "payjoin",
            "payment_in_coinjoin",
            "sweep",
        ]
        .contains(&text.as_str())
        {
            text
        } else {
            "unknown".into()
        }
    })
}
fn imported(raw: &Value) -> Option<String> {
    for payload in [raw.clone(), mapping(&raw["raw_json"], false)] {
        for key in [
            "privacy_boundary",
            "privacy_boundary_kind",
            "privacyBoundary",
            "privacyBoundaryKind",
            "privacy_hop",
            "privacy_hop_kind",
            "privacyHop",
            "privacyHopKind",
        ] {
            if let Some(value) = marker(&payload[key]) {
                return Some(value);
            }
        }
        if [
            "islikelycoinjoin",
            "is_likely_coinjoin",
            "isLikelyCoinJoin",
            "likely_coinjoin",
            "likelyCoinJoin",
        ]
        .iter()
        .any(|key| {
            ["1", "true", "yes", "on"].contains(
                &kassiber_pytext::lower(kassiber_pytext::strip(&text(&payload[key]))).as_str(),
            )
        }) {
            return Some("coinjoin".into());
        }
    }
    None
}
pub(super) fn collaboration(row: &Value, raw: &Value) -> Value {
    let boundary = if !row["privacy_boundary"].is_null() && row["privacy_boundary"] != "" {
        marker(&row["privacy_boundary"]).or_else(|| Some("unknown".into()))
    } else {
        imported(raw)
    };
    if let Some(kind) = boundary {
        if kind == "unknown" {
            return json!({"kind":"unknown","source":"invalid_privacy_metadata","evidence_level":"unavailable"});
        }
        if kind != "sweep" {
            return json!({"kind":kind,"source":"reviewed_or_imported_privacy_boundary","evidence_level":if ["accepted","completed","reviewed"].contains(&row["review_status"].as_str().unwrap_or("")) {"reviewed"} else {"imported"}});
        }
    }
    let tx = mapping(raw, true);
    if !tx["vin"].is_array() || !tx["vout"].is_array() {
        return Value::Null;
    }
    let leg = |value, entry: &Value| crate::coinjoin_shape::Leg {
        value,
        script: script(entry),
    };
    let inputs: Vec<_> = array(&tx["vin"])
        .iter()
        .filter(|v| v.is_object())
        .map(|v| {
            let prevout = if v["prevout"].is_object() {
                &v["prevout"]
            } else {
                &Value::Null
            };
            let value = crate::analysis_features::value_sats(&prevout.serde())
                .or_else(|| crate::analysis_features::value_sats(&v.serde()));
            leg(value, prevout)
        })
        .collect();
    let outputs: Vec<_> = array(&tx["vout"])
        .iter()
        .filter(|v| v.is_object())
        .filter(|v| {
            let kind = if truth(&v["scriptpubkey_type"]) {
                &v["scriptpubkey_type"]
            } else {
                &v["type"]
            };
            kassiber_pytext::lower(&or_text(kind)) != "op_return"
                && !script(v).is_some_and(|s| s.starts_with("6a"))
        })
        .map(|v| leg(crate::analysis_features::value_sats(&v.serde()), v))
        .collect();
    let Some(source) = crate::coinjoin_shape::shape(array(&tx["vin"]).len(), &inputs, &outputs)
    else {
        return Value::Null;
    };
    json!({"kind":"coinjoin","source":source,"evidence_level":"heuristic"})
}
pub(super) fn stronger(a: Value, b: Value) -> Value {
    if !truth(&a) {
        return b;
    }
    if !truth(&b) {
        return a;
    }
    fn rank(v: &Value) -> (u8, u8, String) {
        let kind = match v["kind"].as_str() {
            Some("coinjoin") => 1,
            Some("collaborative") => 2,
            Some("payjoin" | "payment_in_coinjoin") => 3,
            _ => 4,
        };
        let level = match v["evidence_level"].as_str() {
            Some("heuristic") => 1,
            Some("imported") => 2,
            Some("reviewed") => 3,
            _ => 0,
        };
        (
            kind,
            level,
            kassiber_pyjson::dumps(&py(v), &kassiber_pyjson::Options::new().sort_keys(true))
                .unwrap(),
        )
    }
    if rank(&b) > rank(&a) {
        b
    } else {
        a
    }
}

pub(super) fn string(value: &Value) -> kassiber_pyjson::PyStr {
    match value {
        Value::String(v) => v.clone(),
        _ => py_str(&py(value)).into(),
    }
}

pub(in crate::analysis_index) fn scope(
    row: &Value,
) -> Result<(String, String), kassiber_custody::components::Error> {
    let raw = mapping(&row["raw_json"], false);
    let config = mapping(
        if truth(&row["config_json"]) {
            &row["config_json"]
        } else {
            &row["wallet_config_json"]
        },
        false,
    );
    kassiber_custody::evidence::scope(&serde_json::json!({
        "chain": row["chain"].serde(), "network": row["network"].serde(),
        "wallet_kind": row["wallet_kind"].serde(), "asset": row["asset"].serde(),
        "raw_json": {"chain": raw["chain"].serde(), "network": raw["network"].serde()},
        "config_json": {"chain": config["chain"].serde(), "network": config["network"].serde()}
    }))
}

#[cfg(test)]
mod coinjoin_shape_tests {
    use super::{collaboration, py, text, Value};
    use kassiber_pyjson::{dumps, Options};

    #[test]
    fn collaboration_shapes_match_python_for_every_vector() {
        let load = |source: &str| Value::from_py(&kassiber_pytext::loads(source).unwrap());
        let cases = load(include_str!("../../../tests/data/coinjoin_shapes.json"));
        let cases = cases.as_array().unwrap();
        assert!(cases.len() > 60);
        let canonical =
            |v: &Value| dumps(&py(v), &Options::new().sort_keys(true).compact()).unwrap();
        let row = load("{}");
        for case in cases {
            assert_eq!(
                canonical(&collaboration(&row, &case["raw"])),
                canonical(&case["collaboration"]),
                "{}",
                text(&case["name"])
            );
        }
    }
}
