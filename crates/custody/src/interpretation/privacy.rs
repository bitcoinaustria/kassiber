//! Privacy annotations are uncertainty, never ownership or transfer authority.
use super::{anchor, holds::Ids, recorded, Context};
use crate::fields::{Fields, Row};
use crate::{codec, graph::stored, python_text::PythonText};
use kassiber_pyjson::{Options, Value as Py};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const LIKELY: [&str; 5] = [
    "islikelycoinjoin",
    "is_likely_coinjoin",
    "isLikelyCoinJoin",
    "likely_coinjoin",
    "likelyCoinJoin",
];

fn boolish(value: &Py) -> bool {
    if matches!(value, Py::Bool(true)) {
        return true;
    }
    if matches!(value, Py::Bool(false) | Py::Null) {
        return false;
    }
    matches!(
        super::python_value_string(value)
            .py_strip()
            .py_lower()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn evidence<R: Fields + ?Sized>(row: &R) -> Option<Py> {
    if row.at("privacy_boundary").is_null() {
        return None;
    }
    let boundary = codec::pystr(row.at("privacy_boundary"))
        .py_strip()
        .py_lower()
        .replace('-', "_");
    if !matches!(
        boundary.as_str(),
        "coinjoin" | "collaborative" | "payjoin" | "payment_in_coinjoin" | "sweep"
    ) {
        return None;
    }
    let raw = stored::object(row.at("raw_json"));
    let source = stored::get(&raw, "source");
    let mut detail = codec::from_json(&json!({"privacy_boundary":boundary,"privacy_hop":boundary,"islikelycoinjoin":LIKELY.iter().any(|key| boolish(stored::get(&raw,key))),"required_for":"explicit_user_owned_provenance"})).expect("detail object");
    stored::put(
        &mut detail,
        "source",
        if stored::truth(source) {
            source.clone()
        } else {
            Py::Str("".into())
        },
    );
    Some(detail)
}

pub(super) fn compute<R: Row>(
    rows: &[R],
    auto: &[Value],
    derived: &[Value],
    context: &Context,
    wallets: &BTreeMap<String, Value>,
    samourai_candidates: &Ids,
    native_ids: &Ids,
) -> Result<Value, String> {
    let mut resolved = samourai_candidates.clone();
    resolved.extend(recorded::resolved_ids(auto, context, false));
    resolved.extend(recorded::resolved_ids(derived, context, false));
    for row in rows {
        if row.at("observation_fee_attribution") != "implicit_wallet_delta" {
            continue;
        }
        let id = anchor(row);
        if let Some(observation) = context.observations.get(&id) {
            if codec::truth(&observation["authoritative_chain_observation"])
                && observation["fee_attribution"] == "exact"
                && super::claims::principal(observation) == 0
                && super::integer_or_zero(&observation["fee_msat"])?
                    .as_i64()
                    .is_some_and(|n| n > 0)
            {
                resolved.insert(id);
            }
        }
    }
    resolved.extend(context.excluded.iter().cloned());
    resolved.extend(native_ids.iter().cloned());
    let has_boundaries = rows
        .iter()
        .any(|row| !row.at("privacy_boundary").is_null() && row.at("privacy_boundary") != "");
    let mut quarantines = Vec::new();
    if has_boundaries {
        for row in rows {
            let id = codec::text(row.at("id"));
            if resolved.contains(&id) {
                continue;
            }
            let Some(extra) = evidence(row) else {
                continue;
            };
            let wallet = wallets
                .get(&codec::text(row.at("wallet_id")))
                .unwrap_or(&Value::Null);
            let label = wallet.get("label").unwrap_or(row.at("wallet_id"));
            let mut detail = codec::from_json(
                &json!({"wallet":label,"asset":codec::text(row.at("asset")).py_upper(),"direction":row.at("direction")}),
            )?;
            if let Py::Dict(items) = extra {
                for (key, value) in items {
                    let kassiber_pyjson::Key::Str(key) = key else {
                        unreachable!("privacy detail key");
                    };
                    stored::put(&mut detail, key.as_str().expect("ASCII key"), value);
                }
            }
            let detail = kassiber_pyjson::dumps(&detail, &Options::new().sort_keys(true))
                .map_err(|e| e.to_string())?;
            quarantines.push(json!({"transaction_id":id,"workspace_id":row.at("workspace_id"),"profile_id":row.at("profile_id"),"reason":"privacy_hop_unresolved","detail_json":detail}));
        }
    }
    let blocked: Ids = quarantines
        .iter()
        .map(|item| codec::text(&item["transaction_id"]))
        .collect();
    Ok(json!({"resolved_ids":resolved,"blocked_transaction_ids":blocked,"quarantines":quarantines}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn privacy_evidence_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_safety_helpers.json"
        ))
        .unwrap();
        for vector in vectors.iter().filter(|v| v["op"] == "privacy_evidence") {
            let actual = evidence(&vector["row"])
                .map(|v| kassiber_pyjson::dumps(&v, &Options::new().sort_keys(true)).unwrap());
            assert_eq!(json!(actual), vector["expected"], "{}", vector["case"]);
        }
    }
    #[test]
    fn privacy_resolution_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_safety_helpers.json"
        ))
        .unwrap();
        for vector in vectors.iter().filter(|v| v["op"] == "privacy_resolution") {
            let ids = |key: &str| {
                super::super::array(&vector[key])
                    .iter()
                    .map(codec::text)
                    .collect()
            };
            let context = Context {
                observations: super::super::owned_observations(&vector["observations"]),
                canonical: super::super::Events::json(&Value::Null),
                excluded: ids("excluded"),
            };
            let result = compute(
                super::super::array(&vector["rows"]),
                &[],
                &[],
                &context,
                &serde_json::from_value(vector["wallets"].clone()).unwrap(),
                &ids("candidates"),
                &ids("native_ids"),
            )
            .unwrap();
            assert_eq!(result, vector["expected"], "{}", vector["case"]);
        }
    }
}
