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

/// Acquisition kinds the owner can give a collaborative receipt's net amount
/// (`kind_override`, the acquisition review). `deposit` says nothing about
/// where the value came from, so it does not count.
const RECEIPT_KINDS: [&str; 9] = [
    "airdrop",
    "buy",
    "hardfork",
    "income",
    "interest",
    "lending_interest",
    "mining",
    "staking",
    "wages",
];

/// A collaborative receipt (Payjoin and the like) whose net amount the owner
/// classified: an inbound row whose amount is the wallet's net delta
/// (`implicit_wallet_delta`) with an authored acquisition kind. Row-local
/// only; the interpreter also requires a current authoritative observation
/// and no other row of the same transaction.
pub(crate) fn classified_receipt<R: Fields + ?Sized>(row: &R) -> bool {
    let lower = |key: &str| codec::pystr(row.at(key)).py_strip().py_lower();
    if row.at("privacy_boundary").is_null() || row.at("kind_override").is_null() {
        return false;
    }
    lower("direction") == "inbound"
        && lower("observation_fee_attribution") == "implicit_wallet_delta"
        && RECEIPT_KINDS.contains(&lower("kind_override").replace(['-', ' '], "_").as_str())
}

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

/// The classified collaborative receipts whose net amount the chain proves:
/// a current authoritative observation states the row's amount as the
/// wallet's net delta, and no other row in the book (another wallet, an
/// import) shares the transaction, so none of the owner's value in it is
/// unaccounted for. The owner's acquisition kind supplies only the commercial
/// meaning the graph cannot.
fn classified_receipts<R: Row>(rows: &[R], context: &Context) -> Ids {
    let txid = |row: &R| codec::pystr(row.at("external_id")).py_strip().py_lower();
    let mut rows_per_txid: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows {
        let key = txid(row);
        if !key.is_empty() && !context.excluded.contains(&codec::text(row.at("id"))) {
            *rows_per_txid.entry(key).or_default() += 1;
        }
    }
    let mut ids = Ids::new();
    for row in rows {
        if !classified_receipt(row) {
            continue;
        }
        let key = txid(row);
        if key.is_empty() || rows_per_txid.get(&key) != Some(&1) {
            continue;
        }
        let Some(observation) = context.observations.get(&anchor(row)) else {
            continue;
        };
        if codec::truth(&observation["authoritative_chain_observation"])
            && observation["fee_attribution"] == "implicit_wallet_delta"
            && observation["direction"] == "inbound"
            && super::claims::principal(observation) > 0
        {
            ids.insert(codec::text(row.at("id")));
        }
    }
    ids
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
    resolved.extend(classified_receipts(rows, context));
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
    fn only_an_inbound_net_delta_with_an_acquisition_kind_is_a_classified_receipt() {
        let receipt = json!({"direction":"inbound","privacy_boundary":"collaborative",
            "observation_fee_attribution":"implicit_wallet_delta","kind_override":"income"});
        assert!(classified_receipt(&receipt));
        for (key, value) in [
            ("direction", json!("outbound")),
            ("privacy_boundary", Value::Null),
            ("observation_fee_attribution", json!("exact")),
            ("kind_override", Value::Null),
            ("kind_override", json!("deposit")),
            ("kind_override", json!("withdrawal")),
        ] {
            let mut row = receipt.clone();
            row[key] = value.clone();
            assert!(!classified_receipt(&row), "{key}={value}");
        }
        let mut spelled = receipt.clone();
        spelled["kind_override"] = json!(" Lending-Interest ");
        assert!(classified_receipt(&spelled));
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
