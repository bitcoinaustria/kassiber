//! The schema-2 request and response against the projection-level goldens.
//!
//! `data/ledger_fixtures.json` (`tax-engine/scripts/gen_ledger_fixtures.py`)
//! stores, per case, the request production's Python builder makes of the
//! case's finalized projection, and the engine's response, since production
//! routes every book to the engine. Every stored request must deserialize
//! into the typed request, which rejects unknown fields, and serialize back
//! to the same document, so the builder and the engine read one schema. The
//! ledger operation reproduces every stored response (the generator
//! certified that production's reader renders each into the oracle's
//! outcome); the `prepare` operation answers for every book (its final
//! pass's normalized and prepared stages, the generic rail carry, and the
//! swap-link selection, which `tests/test_tax_ledger_prepare.py` compares
//! with the oracle's), agrees with the ledger operation on the carry, and
//! lists the stages it reached when it fails; the `manifest` operation
//! answers, and other schema versions, unknown fields, and missing fields
//! (nullable ones included) are refused.

use std::collections::{HashMap, HashSet};

use kassiber_tax_core::decimal::Decimal;
use kassiber_tax_core::ledger;
use kassiber_tax_core::ledger::request::LedgerRequest;
use kassiber_tax_core::manifest::manifest;
use kassiber_tax_core::ENGINE_VERSION;
use serde_json::{json, Value};

const FIXTURES: &str = include_str!("data/ledger_fixtures.json");

/// Fewest stored requests and responses, so a truncated or emptied file
/// cannot pass.
const MIN_REQUESTS: usize = 100;

/// Every stored case with a request: its name, request, and the response
/// stored for it.
fn stored_cases() -> Vec<(String, Value, Value)> {
    let document: Value = serde_json::from_str(FIXTURES).expect("ledger fixtures are JSON");
    document["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .filter(|case| !case["request"].is_null())
        .map(|case| {
            let name = case["name"].as_str().expect("case name").to_string();
            (name, case["request"].clone(), case["response"].clone())
        })
        .collect()
}

/// Every stored request, labelled by its case.
fn stored_requests() -> Vec<(String, Value)> {
    stored_cases()
        .into_iter()
        .map(|(name, request, _)| (name, request))
        .collect()
}

/// The stage of a failed response, `None` for a success.
fn failed_stage(response: &str) -> Option<String> {
    let value: Value = serde_json::from_str(response).expect("response is JSON");
    assert_eq!(value["schema_version"], 2, "{response}");
    if value["ok"] == true {
        return None;
    }
    Some(value["error"]["stage"].as_str().expect("stage").to_string())
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
fn the_ledger_operation_reproduces_every_stored_response() {
    let mut replayed = 0;
    let mut carried = 0;
    for (name, request, stored) in stored_cases() {
        let text = ledger(&request.to_string());
        assert!(!stored.is_null(), "{name}: no stored response");
        let response: Value = serde_json::from_str(&text).expect("response is JSON");
        assert_eq!(response, stored, "{name}");
        // Pure: the same request gives the same document.
        assert_eq!(ledger(&request.to_string()), text, "{name}");
        replayed += 1;
        carried += usize::from(
            request["policy"]["country"]["kind"] == "generic"
                && has_carrying_pair(&request, true)
                && response["ok"] == true,
        );
    }
    assert!(replayed >= MIN_REQUESTS, "only {replayed} stored responses");
    // The generic rail carry's goldens (D-carry-1..3 among them).
    assert!(carried >= 15, "only {carried} generic carry books");
}

/// Whether a book has a carrying-value pair whose legs are both rows: the
/// pairs the Austrian swap-link selection considers, and on a generic book,
/// when they are Bitcoin rail pairs, the rail carry's.
fn has_carrying_pair(request: &Value, rail: bool) -> bool {
    let rows: HashSet<&str> = request["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["id"].as_str().expect("row id"))
        .collect();
    request["cross_asset_pairs"]
        .as_array()
        .expect("cross-asset pairs")
        .iter()
        .any(|pair| {
            pair["policy"] == "carrying-value"
                && (!rail || pair["bitcoin_rail"] == true)
                && rows.contains(pair["out_id"].as_str().expect("out id"))
                && rows.contains(pair["in_id"].as_str().expect("in id"))
        })
}

#[test]
fn prepare_normalizes_and_walks_every_book() {
    let mut prepared = 0;
    let mut austrian = 0;
    let mut swaps = 0;
    let mut second_passes = 0;
    let mut carries = 0;
    let mut carry_blocks = 0;
    for (name, stored) in stored_requests() {
        let mut request = stored.clone();
        request["operation"] = json!("prepare");
        let text = ledger(&request.to_string());
        prepared += 1;
        austrian += usize::from(stored["policy"]["country"]["kind"] == "at");
        swaps += usize::from(has_carrying_pair(&stored, false));
        // Pure: the same request gives the same document.
        assert_eq!(ledger(&request.to_string()), text, "{name}");
        let response: Value = serde_json::from_str(&text).expect("response is JSON");
        assert_eq!(response["schema_version"], 2, "{name}");
        if response["ok"] != true {
            // Only a book whose user text marks swaps or repeats a marker
            // fails before composition, in the walk's constructor (a
            // duplicate `at_regime=` the constructor reads), and a rail
            // carry book whose method the policy refuses, in the carry's
            // first pass (the method gate, before any later pass). Its
            // stages reached are listed.
            let (stage, ..) = failure(&text);
            if name == "carry-method-bogus" {
                assert_eq!(stage, "compute_multi", "{name}: {text}");
                assert!(response["rail_carry"].is_null(), "{name}");
                assert!(!response["assets"].as_array().unwrap().is_empty(), "{name}");
                continue;
            }
            assert_eq!(stage, "prepare", "{name}: {text}");
            assert!(name.starts_with("at-usertag-"), "{name}: {text}");
            continue;
        }
        assert_eq!(response["engine_version"], ENGINE_VERSION, "{name}");
        let rows = request["rows"].as_array().expect("rows");
        let anchor: HashMap<&str, &str> = rows
            .iter()
            .map(|row| (row["id"].as_str().unwrap(), row["anchor"].as_str().unwrap()))
            .collect();
        let carry_excluded = check_rail_carry(&name, &stored, &response);
        carries += usize::from(response["rail_carry"]["passes"] != 0);
        carry_blocks += usize::from(!carry_excluded.is_empty());
        let mut excluded = check_swap_links(&name, &request, &response);
        second_passes += usize::from(!excluded.is_empty());
        excluded.extend(carry_excluded);
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
                // A row a blocked swap or rail pair leaves out of the final
                // pass is named by the pair's quarantine (a direct payout's
                // only by its source's), and appears nowhere in the pass.
                if excluded.contains(id) {
                    assert!(!covered.contains(id), "{name}: excluded row {id} booked");
                    continue;
                }
                assert!(
                    covered.contains(id) || named.contains(anchor[id]),
                    "{name}: row {id} vanished"
                );
            }
            check_prepared_stage(&name, &request, &anchor, &events, &outs, &asset["prepared"]);
        }
    }
    assert!(prepared >= 100, "only {prepared} books");
    assert!(austrian >= 50, "only {austrian} Austrian books");
    assert!(swaps >= 15, "only {swaps} Austrian books with swap pairs");
    assert!(
        second_passes >= 6,
        "only {second_passes} second passes leave rows out"
    );
    assert!(carries >= 15, "only {carries} books run the rail carry");
    assert!(
        carry_blocks >= 4,
        "only {carry_blocks} rail carries block a pair"
    );
}

/// The generic rail carry of a successful `prepare`: none on an Austrian
/// book or a generic one without a carrying-value Bitcoin rail pair; on a
/// carry book, at most one pass more than its eligible pairs, quarantines
/// on blocked rows only, both legs of a pair withheld or neither, carried
/// rows that are legs of eligible pairs, an event the final pass makes of
/// a carried row valued at its carried value, and the same carried rows
/// and leading quarantines as the `ledger` operation gives. Returns the
/// rows the final pass leaves out.
fn check_rail_carry<'r>(name: &str, request: &'r Value, response: &Value) -> HashSet<&'r str> {
    let carry = &response["rail_carry"];
    if request["policy"]["country"]["kind"] == "at" || !has_carrying_pair(request, true) {
        assert_eq!(
            carry,
            &json!({"carried": [], "blocked": [], "quarantines": [], "passes": 0}),
            "{name}"
        );
        return HashSet::new();
    }
    let rows: HashSet<&'r str> = request["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["id"].as_str().expect("row id"))
        .collect();
    let eligible: Vec<(&str, &str)> = request["cross_asset_pairs"]
        .as_array()
        .expect("cross-asset pairs")
        .iter()
        .filter(|pair| pair["policy"] == "carrying-value" && pair["bitcoin_rail"] == true)
        .map(|pair| {
            (
                pair["out_id"].as_str().unwrap(),
                pair["in_id"].as_str().unwrap(),
            )
        })
        .filter(|(out, inbound)| rows.contains(out) && rows.contains(inbound))
        .collect();
    let passes = carry["passes"].as_u64().expect("passes") as usize;
    assert!(
        (1..=eligible.len() + 1).contains(&passes),
        "{name}: {passes} passes for {} pairs",
        eligible.len()
    );
    let blocked: HashSet<&str> = carry["blocked"]
        .as_array()
        .expect("blocked")
        .iter()
        .map(|id| id.as_str().expect("blocked id"))
        .collect();
    for quarantine in carry["quarantines"].as_array().expect("quarantines") {
        assert_eq!(
            quarantine["reason"], "bitcoin_rail_carry_basis_unresolved",
            "{name}"
        );
        assert_eq!(quarantine["detail_kind"], "plain", "{name}");
        let row = quarantine["row"].as_str().expect("row");
        assert!(
            blocked.contains(row),
            "{name}: {row} quarantined, not blocked"
        );
    }
    for (out, inbound) in &eligible {
        assert_eq!(
            blocked.contains(out),
            blocked.contains(inbound),
            "{name}: one leg of {out} -> {inbound} withheld"
        );
    }
    let legs: HashSet<&str> = eligible
        .iter()
        .flat_map(|(out, inbound)| [*out, *inbound])
        .collect();
    let events: HashMap<&str, &Value> = response["assets"]
        .as_array()
        .expect("assets")
        .iter()
        .flat_map(|asset| asset["normalized"]["events"].as_array().expect("events"))
        .map(|event| (event["row"].as_str().expect("event row"), event))
        .collect();
    for item in carry["carried"].as_array().expect("carried rows") {
        let row = item["row"].as_str().expect("row");
        assert!(legs.contains(row), "{name}: {row} carried, not a rail leg");
        let value: Decimal = item["value"].as_str().unwrap().parse().expect("a value");
        assert!(value.is_positive(), "{name}: {row} carried at {value}");
        // The final pass reads the carried value; a pair priced, then
        // blocked, keeps its pricing but leaves the pass.
        if let Some(event) = events.get(row) {
            let booked = Decimal::from_canonical(event["fiat_value"].as_str().unwrap()).unwrap();
            assert!(
                booked.same_representation(&value),
                "{name}: {row} booked at {booked:?}, carried {value:?}"
            );
        }
    }
    // The ledger operation carries the same rows and lists the carry's
    // quarantines first.
    let mut ledger_request = request.clone();
    ledger_request["operation"] = json!("ledger");
    let ledger_response: Value =
        serde_json::from_str(&ledger(&ledger_request.to_string())).unwrap();
    if ledger_response["ok"] == true {
        assert_eq!(ledger_response["carried_rows"], carry["carried"], "{name}");
        let quarantines = carry["quarantines"].as_array().unwrap();
        assert_eq!(
            &ledger_response["quarantines"].as_array().unwrap()[..quarantines.len()],
            &quarantines[..],
            "{name}"
        );
    }
    rows.into_iter().filter(|id| blocked.contains(id)).collect()
}

/// The swap-link selection of a successful `prepare`: none on a generic
/// book; on an Austrian one, links name rows the final pass makes events
/// with that link, and blocked pairs' quarantines name their rows' blocked
/// ids. Returns the rows the final pass leaves out.
fn check_swap_links<'r>(name: &str, request: &'r Value, response: &Value) -> HashSet<&'r str> {
    let selection = &response["swap_links"];
    let links = selection["links"].as_array().expect("links");
    let blocked: HashSet<&str> = selection["blocked"]
        .as_array()
        .expect("blocked")
        .iter()
        .map(|id| id.as_str().expect("blocked id"))
        .collect();
    let quarantines = selection["quarantines"].as_array().expect("quarantines");
    if request["policy"]["country"]["kind"] != "at" || !has_carrying_pair(request, false) {
        assert_eq!(
            selection,
            &json!({"links": [], "blocked": [], "quarantines": []}),
            "{name}"
        );
        return HashSet::new();
    }
    let events: HashMap<&str, &Value> = response["assets"]
        .as_array()
        .expect("assets")
        .iter()
        .flat_map(|asset| asset["normalized"]["events"].as_array().expect("events"))
        .map(|event| (event["row"].as_str().expect("event row"), event))
        .collect();
    for link in links {
        let (row, id) = (
            link[0].as_str().expect("row"),
            link[1].as_str().expect("link"),
        );
        // A linked row the pass makes an event carries its link.
        if let Some(event) = events.get(row) {
            assert_eq!(event["at_swap_link"], id, "{name}: {row}");
        }
    }
    for quarantine in quarantines {
        assert_eq!(
            quarantine["reason"], "at_swap_basis_carry_unresolved",
            "{name}"
        );
        assert_eq!(quarantine["detail_kind"], "plain", "{name}");
        let row = quarantine["row"].as_str().expect("row");
        assert!(
            blocked.contains(row),
            "{name}: {row} quarantined, not blocked"
        );
    }
    let rows: HashSet<&str> = request["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["id"].as_str().expect("row id"))
        .collect();
    let excluded: HashSet<&'r str> = rows.into_iter().filter(|id| blocked.contains(id)).collect();
    for id in &excluded {
        assert!(
            !events.contains_key(id),
            "{name}: excluded row {id} is an event"
        );
    }
    excluded
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
        .find(|(_, request)| standalone(request).is_some())
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
    // A row's cutoff instant is required, and is either an instant or
    // Python's error, nothing else.
    let mut no_instant = stored.clone();
    no_instant["rows"][0]
        .as_object_mut()
        .expect("row object")
        .remove("cutoff_instant");
    let mut odd_instant = stored.clone();
    odd_instant["rows"][0]["cutoff_instant"] = json!({"us": 0, "error": "both"});
    let mut text_instant = stored.clone();
    text_instant["rows"][0]["cutoff_instant"] = json!("2024-01-01T00:00:00Z");
    for request in [
        extra_row_field,
        extra_field,
        missing_field,
        bad_decimal,
        no_instant,
        odd_instant,
        text_instant,
    ] {
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
        // Present as `null`, the same field reads: the book succeeds or
        // fails at a stage past reading it.
        let mut null = stored.clone();
        *null.pointer_mut(pointer).expect("the field") = Value::Null;
        let text = ledger(&null.to_string());
        assert_ne!(
            failed_stage(&text).as_deref(),
            Some("request"),
            "{name}: {pointer}: {text}"
        );
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
