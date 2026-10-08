//! The schema-2 request against the projection-level goldens.
//!
//! `data/ledger_fixtures.json` (`tax-engine/scripts/gen_ledger_fixtures.py`)
//! stores, per case, the request production's Python builder makes of the
//! case's finalized projection. Every stored request must deserialize into
//! the typed request, which rejects unknown fields, and serialize back to the
//! same document, so the builder and the engine read one schema. The ledger
//! operation answers `Unsupported` until the ledger layer is ported; the
//! `prepare` operation answers for a generic book without the Bitcoin rail
//! carry (its normalized and prepared stages, which
//! `tests/test_tax_ledger_prepare.py` compares with the oracle's), lists the
//! stages it reached when it fails, and is `Unsupported` for the others; the
//! `manifest` operation answers, and other schema versions, unknown fields,
//! and missing fields (nullable ones included) are refused.

use std::collections::{HashMap, HashSet};

use kassiber_tax_core::ledger;
use kassiber_tax_core::ledger::request::LedgerRequest;
use kassiber_tax_core::manifest::manifest;
use kassiber_tax_core::ENGINE_VERSION;
use serde_json::{json, Value};

const FIXTURES: &str = include_str!("data/ledger_fixtures.json");

/// Fewest stored requests, so a truncated or emptied file cannot pass.
const MIN_REQUESTS: usize = 100;

/// Every stored request, labelled by its case.
fn stored_requests() -> Vec<(String, Value)> {
    let document: Value = serde_json::from_str(FIXTURES).expect("ledger fixtures are JSON");
    document["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .filter(|case| !case["request"].is_null())
        .map(|case| {
            let name = case["name"].as_str().expect("case name").to_string();
            (name, case["request"].clone())
        })
        .collect()
}

/// The stage and inner class of a failed response.
fn failure(response: &str) -> (String, String, String) {
    let value: Value = serde_json::from_str(response).expect("response is JSON");
    assert_eq!(value["schema_version"], 2, "{response}");
    assert_eq!(value["ok"], false, "{response}");
    let error = &value["error"];
    (
        error["stage"].as_str().expect("stage").to_string(),
        error["inner"]["class"].as_str().expect("class").to_string(),
        error["inner"]["message"]
            .as_str()
            .expect("message")
            .to_string(),
    )
}

#[test]
fn every_stored_request_reads_and_writes_back_unchanged() {
    let requests = stored_requests();
    assert!(
        requests.len() >= MIN_REQUESTS,
        "only {} stored requests",
        requests.len()
    );
    let mut countries = std::collections::BTreeSet::new();
    let mut mismatches = Vec::new();
    for (name, stored) in &requests {
        let typed: LedgerRequest = match serde_json::from_value(stored.clone()) {
            Ok(typed) => typed,
            Err(error) => {
                mismatches.push(format!("{name}: does not deserialize: {error}"));
                continue;
            }
        };
        let written = serde_json::to_value(&typed).expect("requests serialize");
        if &written != stored {
            mismatches.push(format!("{name}: serializes to another document"));
        }
        countries.insert(
            stored["policy"]["country"]["kind"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        );
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    assert_eq!(
        countries.into_iter().collect::<Vec<_>>(),
        vec!["at".to_string(), "generic".to_string()]
    );
}

#[test]
fn the_ledger_operation_is_not_implemented_yet() {
    for (name, stored) in stored_requests() {
        let (stage, class, message) = failure(&ledger(&stored.to_string()));
        assert_eq!(
            (stage.as_str(), class.as_str()),
            ("unsupported", "Unsupported"),
            "{name}: {message}"
        );
    }
}

/// Whether the `prepare` operation implements a book: a generic one without
/// a carrying-value Bitcoin rail pair whose legs are both rows (the
/// `generic` capability alone, `kassiber/core/engines/ledger.py`).
fn prepares(request: &Value) -> bool {
    if request["policy"]["country"]["kind"] != "generic" {
        return false;
    }
    let rows: HashSet<&str> = request["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["id"].as_str().expect("row id"))
        .collect();
    !request["cross_asset_pairs"]
        .as_array()
        .expect("cross-asset pairs")
        .iter()
        .any(|pair| {
            pair["policy"] == "carrying-value"
                && pair["bitcoin_rail"] == true
                && rows.contains(pair["out_id"].as_str().expect("out id"))
                && rows.contains(pair["in_id"].as_str().expect("in id"))
        })
}

#[test]
fn prepare_normalizes_and_walks_generic_books_without_the_rail_carry() {
    let mut prepared = 0;
    for (name, stored) in stored_requests() {
        let mut request = stored.clone();
        request["operation"] = json!("prepare");
        let text = ledger(&request.to_string());
        if !prepares(&stored) {
            let (stage, class, message) = failure(&text);
            assert_eq!(
                (stage.as_str(), class.as_str()),
                ("unsupported", "Unsupported"),
                "{name}: {message}"
            );
            continue;
        }
        prepared += 1;
        // Pure: the same request gives the same document.
        assert_eq!(ledger(&request.to_string()), text, "{name}");
        let response: Value = serde_json::from_str(&text).expect("response is JSON");
        assert_eq!(response["ok"], true, "{name}: {text}");
        assert_eq!(response["schema_version"], 2, "{name}");
        assert_eq!(response["engine_version"], ENGINE_VERSION, "{name}");
        let rows = request["rows"].as_array().expect("rows");
        let anchor: HashMap<&str, &str> = rows
            .iter()
            .map(|row| (row["id"].as_str().unwrap(), row["anchor"].as_str().unwrap()))
            .collect();
        let mut assets: Vec<&str> = Vec::new();
        for row in rows {
            let asset = row["asset"].as_str().expect("asset");
            if !assets.contains(&asset) {
                assets.push(asset);
            }
        }
        let prepared_assets = response["assets"].as_array().expect("assets");
        let order: Vec<&str> = prepared_assets
            .iter()
            .map(|asset| asset["asset"].as_str().expect("asset name"))
            .collect();
        assert_eq!(order, assets, "{name}: assets in first-appearance order");
        for asset in prepared_assets {
            let normalized = &asset["normalized"];
            let ids = |list: &str, field: &str| -> Vec<String> {
                normalized[list]
                    .as_array()
                    .expect(list)
                    .iter()
                    .map(|item| item[field].as_str().expect(field).to_string())
                    .collect()
            };
            let (events, outs, ins) = (
                ids("events", "row"),
                ids("transfers", "out"),
                ids("transfers", "in"),
            );
            // The order lists each event at its row and each move at its
            // out leg, once.
            let items = normalized["ordered_items"].as_array().expect("items");
            let listed: HashSet<String> = items
                .iter()
                .map(|item| {
                    format!(
                        "{} {}",
                        item[0].as_str().unwrap(),
                        item[1].as_str().unwrap()
                    )
                })
                .collect();
            let booked: HashSet<String> = events
                .iter()
                .map(|row| format!("event {row}"))
                .chain(outs.iter().map(|row| format!("transfer {row}")))
                .collect();
            assert_eq!(listed.len(), items.len(), "{name}");
            assert_eq!(listed, booked, "{name}");
            // Every row of the asset is booked, moved, or its transaction
            // is named by a quarantine.
            let named: HashSet<&str> = ids("quarantines", "row")
                .iter()
                .map(|row| anchor[row.as_str()])
                .collect();
            let covered: HashSet<&str> = events
                .iter()
                .chain(&outs)
                .chain(&ins)
                .map(String::as_str)
                .collect();
            for row in rows.iter().filter(|row| row["asset"] == asset["asset"]) {
                let id = row["id"].as_str().unwrap();
                assert!(
                    covered.contains(id) || named.contains(anchor[id]),
                    "{name}: row {id} vanished"
                );
            }
            check_prepared_stage(&name, &request, &anchor, &events, &outs, &asset["prepared"]);
        }
    }
    assert!(prepared >= 35, "only {prepared} generic books");
}

/// The prepared stage of one asset: the walk books each normalized event
/// and move or quarantines it, numbers its entries in walk order, lists
/// the inbound entries first, and audits each booked move.
fn check_prepared_stage(
    name: &str,
    request: &Value,
    anchor: &HashMap<&str, &str>,
    events: &[String],
    outs: &[String],
    prepared: &Value,
) {
    let gates = prepared["quarantines"]
        .as_array()
        .expect("gate quarantines");
    for gate in gates {
        assert_eq!(gate["detail_kind"], "plain", "{name}: {gate}");
    }
    let audit = prepared["intra_audit"].as_array().expect("audit");
    let Some(entries) = prepared["entries"].as_array() else {
        // Without an inbound entry the lot engine gets nothing.
        assert!(prepared["entries"].is_null(), "{name}");
        return;
    };
    let kinds: Vec<&str> = entries
        .iter()
        .map(|entry| entry["kind"].as_str().expect("kind"))
        .collect();
    let mut sorted = kinds.clone();
    sorted.sort_by_key(|kind| ["in", "out", "intra"].iter().position(|k| k == kind));
    assert_eq!(kinds, sorted, "{name}: inbound, outbound, then moves");
    assert_eq!(kinds.first(), Some(&"in"), "{name}");
    let mut numbers: Vec<i64> = entries
        .iter()
        .map(|entry| entry["row"].as_i64().expect("row"))
        .collect();
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        (1..=entries.len() as i64).collect::<Vec<_>>(),
        "{name}: rows numbered in walk order"
    );
    let booked: HashSet<&str> = entries
        .iter()
        .map(|entry| entry["unique_id"].as_str().expect("unique id"))
        .collect();
    let gated: HashSet<&str> = gates
        .iter()
        .map(|gate| anchor[gate["row"].as_str().expect("row")])
        .collect();
    for item in events.iter().chain(outs) {
        assert!(
            booked.contains(item.as_str()) || gated.contains(anchor[item.as_str()]),
            "{name}: {item} neither booked nor quarantined by a gate"
        );
    }
    // One audit row per move entry, in order, naming the move's pair.
    let moved: Vec<&str> = entries
        .iter()
        .filter(|entry| entry["kind"] == "intra")
        .map(|entry| entry["unique_id"].as_str().unwrap())
        .collect();
    let audited: Vec<&str> = audit
        .iter()
        .map(|row| {
            let pair = row["pair"].as_u64().expect("pair") as usize;
            request["intra_pairs"][pair]["out"]
                .as_str()
                .expect("out leg")
        })
        .collect();
    assert_eq!(moved, audited, "{name}");
    for entry in entries {
        let text = entry["text"].as_str().expect("text");
        let class = match entry["kind"].as_str() {
            Some("in") => "InTransaction:",
            Some("out") => "OutTransaction:",
            _ => "IntraTransaction:",
        };
        assert!(text.starts_with(class), "{name}: {text}");
    }
}

#[test]
fn a_failed_prepare_lists_the_stages_it_reached() {
    // A generic book's standalone purchase, priced at Infinity: its asset
    // normalizes, and its walk fails where the constructor compares the
    // price.
    let standalone = |request: &Value| -> Option<usize> {
        let legs: HashSet<&str> = request["intra_pairs"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|pair| [pair["out"].as_str().unwrap(), pair["in"].as_str().unwrap()])
            .collect();
        request["rows"].as_array().unwrap().iter().position(|row| {
            row["direction"] == "inbound"
                && row["hold"].is_null()
                && !legs.contains(row["id"].as_str().unwrap())
        })
    };
    let (name, mut request) = stored_requests()
        .into_iter()
        .find(|(_, request)| prepares(request) && standalone(request).is_some())
        .expect("a generic book");
    let index = standalone(&request).unwrap();
    request["operation"] = json!("prepare");
    let row = &mut request["rows"][index];
    row["rate"] = json!("Infinity");
    row["kind"] = json!("buy");
    row["coarse"] = json!(false);
    row["privacy_hop"] = json!(false);
    let asset = row["asset"].clone();
    let text = ledger(&request.to_string());
    let (stage, class, message) = failure(&text);
    assert_eq!(
        (stage.as_str(), class.as_str(), message.as_str()),
        (
            "prepare",
            "InvalidOperation",
            "[<class 'decimal.InvalidOperation'>]"
        ),
        "{name}"
    );
    let response: Value = serde_json::from_str(&text).unwrap();
    let assets = response["assets"].as_array().expect("the stages reached");
    let last = assets.last().expect("the failing asset");
    assert_eq!(last["asset"], asset, "{name}");
    assert!(last["normalized"].is_object(), "{name}");
    assert!(last["prepared"].is_null(), "{name}");
    for earlier in &assets[..assets.len() - 1] {
        assert!(earlier["prepared"].is_object(), "{name}");
    }
    // A request that does not read reached no stage.
    let (stage, ..) = failure(&ledger(r#"{"schema_version": 2, "operation": "prepare"}"#));
    assert_eq!(stage, "request");
    let refused: Value =
        serde_json::from_str(&ledger(r#"{"schema_version": 2, "operation": "prepare"}"#)).unwrap();
    assert_eq!(refused["assets"], json!([]));
}

#[test]
fn other_schema_versions_and_unknown_fields_are_refused() {
    let (_, stored) = stored_requests()
        .into_iter()
        .next()
        .expect("a stored request");
    for version in [0, 1, 3] {
        let mut request = stored.clone();
        request["schema_version"] = json!(version);
        let (stage, class, message) = failure(&ledger(&request.to_string()));
        assert_eq!(
            (stage.as_str(), class.as_str()),
            ("request", "RequestError")
        );
        assert!(
            message.starts_with(&format!("unsupported schema_version {version}")),
            "{message}"
        );
    }
    let mut extra_row_field = stored.clone();
    extra_row_field["rows"][0]["notes"] = json!("at_regime=neu");
    let mut extra_field = stored.clone();
    extra_field["markers"] = json!([]);
    let mut missing_field = stored.clone();
    missing_field
        .as_object_mut()
        .expect("request object")
        .remove("policy");
    let mut bad_decimal = stored.clone();
    bad_decimal["rows"][0]["rate"] = json!("1.5");
    for request in [extra_row_field, extra_field, missing_field, bad_decimal] {
        let (stage, class, message) = failure(&ledger(&request.to_string()));
        assert_eq!(
            (stage.as_str(), class.as_str()),
            ("request", "RequestError"),
            "{message}"
        );
        assert!(message.starts_with("invalid engine request:"), "{message}");
    }
    let (stage, class, _) = failure(&ledger("not json"));
    assert_eq!(
        (stage.as_str(), class.as_str()),
        ("request", "RequestError")
    );
}

/// Every nullable field of the request, as a JSON pointer.
const NULLABLE_FIELDS: [&str; 19] = [
    "/holder",
    "/policy/method",
    "/wallets/0/label",
    "/rows/0/rate",
    "/rows/0/value",
    "/rows/0/text",
    "/rows/0/regime_override",
    "/rows/0/category_override",
    "/rows/0/hold",
    "/rows/0/chain_order",
    "/rows/0/ts/vienna_date",
    "/intra_pairs/0/source",
    "/intra_pairs/0/group_id",
    "/cross_asset_pairs/0/pair_id",
    "/cross_asset_pairs/0/out_transaction_id",
    "/cross_asset_pairs/0/in_transaction_id",
    "/cross_asset_pairs/0/out_asset",
    "/cross_asset_pairs/0/in_asset",
    "/cross_asset_pairs/0/policy",
];

#[test]
fn a_missing_nullable_field_is_refused() {
    // Serde reads a missing `Option` field as `None`; the request must carry
    // each one, as `null` when it has no value, so a builder that stops
    // sending one fails instead of being read as "none".
    let requests = stored_requests();
    for pointer in NULLABLE_FIELDS {
        let (parent, key) = pointer.rsplit_once('/').expect("a field pointer");
        let (name, stored) = requests
            .iter()
            .find(|(_, request)| request.pointer(pointer).is_some())
            .unwrap_or_else(|| panic!("no stored request has {pointer}"));
        let mut missing = stored.clone();
        let removed = missing
            .pointer_mut(parent)
            .and_then(Value::as_object_mut)
            .and_then(|object| object.remove(key));
        assert!(removed.is_some(), "{name}: {pointer}");
        let (stage, class, message) = failure(&ledger(&missing.to_string()));
        assert_eq!(
            (stage.as_str(), class.as_str()),
            ("request", "RequestError"),
            "{name}: {pointer}: {message}"
        );
        // A timestamp is untagged (parsed or failed), so serde names no
        // field when neither shape matches.
        let expected = if key == "vienna_date" {
            "did not match any variant of untagged enum Ts".to_string()
        } else {
            format!("missing field `{key}`")
        };
        assert!(message.contains(&expected), "{name}: {pointer}: {message}");
        // Present as `null`, the same field reads.
        let mut null = stored.clone();
        *null.pointer_mut(pointer).expect("the field") = Value::Null;
        let (stage, _, message) = failure(&ledger(&null.to_string()));
        assert_eq!(stage, "unsupported", "{name}: {pointer}: {message}");
    }
}

#[test]
fn the_manifest_operation_lists_the_engines_tables() {
    let response: Value =
        serde_json::from_str(&ledger(r#"{"schema_version": 2, "operation": "manifest"}"#))
            .expect("manifest response is JSON");
    assert_eq!(response["schema_version"], 2);
    assert_eq!(response["ok"], true);
    let mut expected = serde_json::to_value(manifest()).expect("manifest serializes");
    let object = expected.as_object_mut().expect("manifest object");
    object.insert(String::from("schema_version"), json!(2));
    object.insert(String::from("ok"), json!(true));
    assert_eq!(response, expected);
    let (stage, class, _) = failure(&ledger(
        r#"{"schema_version": 2, "operation": "manifest", "rows": []}"#,
    ));
    assert_eq!(
        (stage.as_str(), class.as_str()),
        ("request", "RequestError")
    );
}
