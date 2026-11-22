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

/// The shape of a collaborative receipt (Payjoin and the like), apart from
/// its classification: an inbound row with a privacy boundary whose amount is
/// the wallet's net delta (`implicit_wallet_delta`), over a stored graph whose
/// every input carries its outpoint and complete previous output. Bitcoin
/// Core's fallback states a net delta without every previous output; such a
/// row stays held. Row-local only.
fn receipt_shape<R: Fields + ?Sized>(row: &R) -> bool {
    let lower = |key: &str| codec::pystr(row.at(key)).py_strip().py_lower();
    !row.at("privacy_boundary").is_null()
        && lower("direction") == "inbound"
        && lower("observation_fee_attribution") == "implicit_wallet_delta"
        && complete_inputs(row.at("raw_json"))
}

/// Every input of the stored graph has its outpoint, previous output value
/// and script, and the graph's sources do not conflict.
fn complete_inputs(raw: &Value) -> bool {
    let Ok(Some(graph)) = crate::graph::parse(raw) else {
        return false;
    };
    let Some(inputs) = graph["inputs"]
        .as_array()
        .filter(|inputs| !inputs.is_empty())
    else {
        return false;
    };
    !codec::truth(&graph["evidence_conflicts"])
        && inputs.iter().all(|input| {
            codec::truth(&input["outpoint"])
                && codec::truth(&input["script"])
                && !input["value_sats"].is_null()
        })
}

/// The owner's acquisition kind on the row (`kind_override`).
fn acquisition_kind<R: Fields + ?Sized>(row: &R) -> bool {
    !row.at("kind_override").is_null()
        && RECEIPT_KINDS.contains(
            &codec::pystr(row.at("kind_override"))
                .py_strip()
                .py_lower()
                .replace(['-', ' '], "_")
                .as_str(),
        )
}

/// A collaborative receipt whose net amount the owner classified with an
/// acquisition kind. Row-local only; the interpreter also requires a current
/// authoritative observation and no other row of the same transaction.
pub(crate) fn classified_receipt<R: Fields + ?Sized>(row: &R) -> bool {
    acquisition_kind(row) && receipt_shape(row)
}

/// A row's transaction id as the receipt rule compares it.
pub(crate) fn receipt_txid(value: &Value) -> String {
    codec::pystr(value).py_strip().py_lower()
}

/// Whether the loader must report excluded rows of this row's transaction:
/// it could be a collaborative receipt.
pub(crate) fn may_be_receipt<R: Fields + ?Sized>(row: &R) -> bool {
    !row.at("privacy_boundary").is_null()
        && codec::pystr(row.at("direction")).py_strip().py_lower() == "inbound"
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

/// The collaborative receipts whose net amount the chain proves, split into
/// (unclassified, classified): a current authoritative observation states
/// the row's amount as the wallet's net delta, and no other row of the book
/// shares the transaction (another wallet, an import, a component anchor, or
/// an excluded row: `excluded_txids`), so none of the owner's value in it is
/// unaccounted for. The owner's acquisition kind supplies only the commercial
/// meaning the graph cannot; until then the hold says it can be classified.
fn receipts<R: Row>(rows: &[R], context: &Context, excluded_txids: &[String]) -> (Ids, Ids) {
    let mut rows_per_txid: BTreeMap<String, usize> = BTreeMap::new();
    let known = rows
        .iter()
        .map(|row| receipt_txid(row.at("external_id")))
        .chain(excluded_txids.iter().map(|txid| receipt_txid(&json!(txid))));
    for key in known {
        if !key.is_empty() {
            *rows_per_txid.entry(key).or_default() += 1;
        }
    }
    let (mut unclassified, mut classified) = (Ids::new(), Ids::new());
    for row in rows {
        if !receipt_shape(row) {
            continue;
        }
        let key = receipt_txid(row.at("external_id"));
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
            let id = codec::text(row.at("id"));
            if acquisition_kind(row) {
                classified.insert(id);
            } else {
                unclassified.insert(id);
            }
        }
    }
    (unclassified, classified)
}

pub(super) fn compute<R: Row>(
    rows: &[R],
    auto: &[Value],
    derived: &[Value],
    context: &Context,
    wallets: &BTreeMap<String, Value>,
    samourai_candidates: &Ids,
    native_ids: &Ids,
    excluded_txids: &[String],
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
    let (classifiable, classified) = receipts(rows, context, excluded_txids);
    resolved.extend(classified);
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
            // Only the owner's classification is missing; review offers it.
            if classifiable.contains(&id) {
                stored::put(&mut detail, "classifiable_receipt", Py::Bool(true));
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
    const RECEIPT_TXID: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    /// A received Payjoin's graph: the wallet's own input and the outside
    /// party's, each with its complete previous output.
    fn receipt_graph() -> Value {
        let input = |byte: char, value: u64| {
            json!({"txid":byte.to_string().repeat(64),"vout":0,
                "prevout":{"scriptpubkey":format!("0014{}", byte.to_string().repeat(40)),"value":value}})
        };
        json!({"txid":RECEIPT_TXID,"vin":[input('a', 50_000_000), input('b', 20_000_000)],
            "vout":[{"n":0,"scriptpubkey":format!("0014{}", "c".repeat(40)),"value":60_000_000},
                {"n":1,"scriptpubkey":format!("0014{}", "d".repeat(40)),"value":9_990_000}]})
    }

    fn receipt_row(raw: Value) -> Value {
        json!({"id":"receipt","wallet_id":"w1","asset":"BTC","direction":"inbound",
            "privacy_boundary":"collaborative","observation_fee_attribution":"implicit_wallet_delta",
            "kind_override":"income","external_id":RECEIPT_TXID,"raw_json":raw.to_string()})
    }

    fn receipt_context(excluded: &[&str]) -> Context<'static> {
        Context {
            observations: super::super::owned_observations(&json!({"receipt":{
                "authoritative_chain_observation":true,"fee_attribution":"implicit_wallet_delta",
                "direction":"inbound","amount_msat":10_000_000_000_i64}})),
            canonical: super::super::Events::json(&Value::Null),
            excluded: excluded.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    /// The receipt's hold detail, or `None` when the receipt was released.
    fn held(rows: &[Value], context: &Context, excluded_txids: &[String]) -> Option<Value> {
        let result = compute(
            rows,
            &[],
            &[],
            context,
            &BTreeMap::new(),
            &Ids::new(),
            &Ids::new(),
            excluded_txids,
        )
        .unwrap();
        super::super::array(&result["quarantines"])
            .iter()
            .find(|q| q["transaction_id"] == "receipt")
            .map(|q| serde_json::from_str(q["detail_json"].as_str().unwrap()).unwrap())
    }

    #[test]
    fn only_an_inbound_net_delta_with_an_acquisition_kind_is_a_classified_receipt() {
        let receipt = receipt_row(receipt_graph());
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
    fn a_receipt_needs_every_input_with_its_complete_previous_output() {
        let mut incomplete: Vec<(&str, Value)> = Vec::new();
        for (case, pointer) in [
            ("script", "/vin/1/prevout/scriptpubkey"),
            ("value", "/vin/1/prevout/value"),
            ("outpoint", "/vin/1/txid"),
        ] {
            let mut graph = receipt_graph();
            *graph.pointer_mut(pointer).unwrap() = Value::Null;
            incomplete.push((case, graph));
        }
        let mut no_prevout = receipt_graph();
        no_prevout["vin"][1]
            .as_object_mut()
            .unwrap()
            .remove("prevout");
        incomplete.push(("prevout", no_prevout));
        let mut no_inputs = receipt_graph();
        no_inputs["vin"] = json!([]);
        incomplete.push(("inputs", no_inputs));
        // Bitcoin Core's fallback without a graph: wallet details only.
        incomplete.push((
            "graph",
            json!({"source":"bitcoinrpc_wallet_details","details":[],
                "component":{"fee_attribution":"implicit_wallet_delta"}}),
        ));
        let mut conflicting = receipt_graph();
        let mut other = receipt_graph();
        other["vin"][1]["prevout"]["value"] = json!(10_000_000);
        conflicting["ownership_graph"] = other;
        incomplete.push(("conflict", conflicting));
        let context = receipt_context(&[]);
        assert_eq!(held(&[receipt_row(receipt_graph())], &context, &[]), None);
        for (case, graph) in incomplete {
            let row = receipt_row(graph);
            assert!(!classified_receipt(&row), "{case}");
            let detail = held(&[row], &context, &[]).expect(case);
            assert!(detail.get("classifiable_receipt").is_none(), "{case}");
        }
    }

    #[test]
    fn any_other_row_of_the_transaction_keeps_a_receipt_held() {
        let receipt = receipt_row(receipt_graph());
        let sibling = json!({"id":"sibling","wallet_id":"w2","asset":"BTC","direction":"outbound",
            "external_id":RECEIPT_TXID.to_uppercase()});
        // Another loaded row, also when it anchors a custody component.
        for anchors in [&[][..], &["sibling"][..]] {
            let rows = [receipt.clone(), sibling.clone()];
            assert!(held(&rows, &receipt_context(anchors), &[]).is_some());
        }
        // A user-excluded row the loader reports by its txid.
        let excluded = [format!(" {} ", RECEIPT_TXID.to_uppercase())];
        let detail = held(&[receipt.clone()], &receipt_context(&[]), &excluded).unwrap();
        assert!(detail.get("classifiable_receipt").is_none());
        let elsewhere = ["2".repeat(64)];
        assert_eq!(held(&[receipt], &receipt_context(&[]), &elsewhere), None);
    }

    #[test]
    fn a_provable_receipt_without_an_acquisition_kind_says_it_can_be_classified() {
        for kind in [Value::Null, json!("deposit")] {
            let mut row = receipt_row(receipt_graph());
            row["kind_override"] = kind.clone();
            let detail = held(&[row], &receipt_context(&[]), &[]).expect("held");
            assert_eq!(detail["classifiable_receipt"], true, "{kind}");
        }
        let mut context = receipt_context(&[]);
        context.observations = super::super::owned_observations(&json!({"receipt":{
            "authoritative_chain_observation":false,"fee_attribution":"implicit_wallet_delta",
            "direction":"inbound","amount_msat":10_000_000_000_i64}}));
        let mut row = receipt_row(receipt_graph());
        row["kind_override"] = Value::Null;
        let detail = held(&[row.clone()], &context, &[]).expect("held");
        assert!(detail.get("classifiable_receipt").is_none());
        row["kind_override"] = json!("income");
        assert!(held(&[row], &context, &[]).is_some());
    }

    #[test]
    fn excluded_receipt_txids_name_only_a_possible_receipts_transaction() {
        let record =
            |row: Value| -> crate::snapshot::Record { row.as_object().cloned().unwrap().into() };
        let receipt = record(receipt_row(receipt_graph()));
        let mut outbound = receipt_row(receipt_graph());
        outbound["direction"] = json!("outbound");
        outbound["external_id"] = json!("2".repeat(64));
        let excluded = [
            json!(RECEIPT_TXID.to_uppercase()),
            json!(RECEIPT_TXID),
            json!("2".repeat(64)),
            json!("3".repeat(64)),
        ];
        assert_eq!(
            super::super::excluded_receipt_txids(&[receipt, record(outbound)], &excluded),
            [RECEIPT_TXID]
        );
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
                &[],
            )
            .unwrap();
            assert_eq!(result, vector["expected"], "{}", vector["case"]);
        }
    }
}
