//! The pipeline's in-process ledger call (`engine::ledger`) returns what
//! the text path returns: the request written as text, the engine's
//! response text, and that text decoded. Every ledger golden is compared,
//! successes and refusals, plus documents the typed request cannot read.
use kassiber_custody::ledger_request::request_json;
use kassiber_journals::{boundary, engine, Refusal};
use kassiber_tax_core::ledger::request::LedgerRequest;
use kassiber_tax_core::ledger::response::LedgerResponse;
use serde::Deserialize;
use serde_json::{json, Value};

fn outcome(result: Result<LedgerResponse, Refusal>) -> Value {
    match result {
        Ok(response) => json!({"ok": response.to_json().expect("response text")}),
        Err(refusal) => json!({"refusal": serde_json::to_value(refusal).expect("refusal")}),
    }
}

fn compare(name: &str, request: &Value) -> Value {
    let typed = outcome(engine::ledger(request));
    let text = outcome(boundary::ledger_text(&request_json(request)));
    assert_eq!(typed, text, "{name}");
    typed
}

#[test]
fn typed_ledger_matches_the_text_path_on_every_golden() {
    let document: Value = serde_json::from_str(include_str!(
        "../../../tax-engine/core/tests/data/ledger_fixtures.json"
    ))
    .unwrap();
    let (mut ok, mut refused) = (0, 0);
    for case in document["cases"].as_array().unwrap() {
        let request = &case["request"];
        if request.is_null() {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        assert!(
            LedgerRequest::deserialize(request).is_ok(),
            "{name} reads typed"
        );
        if compare(name, request).get("ok").is_some() {
            ok += 1;
        } else {
            refused += 1;
        }
    }
    assert!(
        ok >= 50 && refused >= 5,
        "{ok} successes, {refused} refusals"
    );
}

#[test]
fn documents_the_typed_request_cannot_read_take_the_text_path() {
    let document: Value = serde_json::from_str(include_str!(
        "../../../tax-engine/core/tests/data/ledger_fixtures.json"
    ))
    .unwrap();
    let case = document["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["response"]["ok"] == true && !case["request"]["rows"][0].is_null())
        .unwrap();
    type Edit = (&'static str, fn(&mut Value));
    let edits: [Edit; 4] = [
        ("amount beyond u64", |r| {
            r["rows"][0]["amount_msat"] = serde_json::from_str("18446744073709551616").unwrap()
        }),
        ("negative fee", |r| r["rows"][0]["fee_msat"] = json!(-1)),
        ("non-canonical rate", |r| {
            r["rows"][0]["rate"] = json!("1.0")
        }),
        ("prepare operation", |r| r["operation"] = json!("prepare")),
    ];
    for (name, edit) in edits {
        let mut request = case["request"].clone();
        edit(&mut request);
        let typed = LedgerRequest::deserialize(&request).ok();
        assert!(
            typed.is_none_or(|t| kassiber_tax_core::ledger_typed(&t).is_err()),
            "{name} must not run typed"
        );
        if name == "prepare operation" {
            // The pipeline only sends `ledger`. The in-process call refuses
            // another operation up front; the text path refuses its response.
            let typed = outcome(engine::ledger(&request));
            let text = outcome(boundary::ledger_text(&request_json(&request)));
            assert_eq!(
                typed["refusal"]["family"], text["refusal"]["family"],
                "{name}: {typed} / {text}"
            );
            assert!(typed.get("refusal").is_some(), "{name}: {typed}");
            continue;
        }
        let result = compare(name, &request);
        assert!(result.get("refusal").is_some(), "{name}: {result}");
    }
}

/// A row of a rules-country book at UTC midnight of `date`.
fn rules_row(
    id: &str,
    wallet: &str,
    direction: &str,
    date: &str,
    msat: u64,
    fields: Value,
) -> Value {
    let days = kassiber_tax_countries::calendar::Date::parse(date)
        .unwrap()
        .to_days();
    let us = days * 86_400_000_000;
    let mut row = json!({
        "id": format!("custody-tax:{id}"), "anchor": id, "asset": "BTC", "wallet_id": wallet,
        "direction": direction,
        "ts": {"raw": format!("{date}T00:00:00Z"), "us": us, "offset_s": 0, "date": date,
               "year": date[..4].parse::<i32>().unwrap(), "vienna_date": null,
               "display": format!("{date} 00:00:00+00:00"), "iso": format!("{date}T00:00:00+00:00")},
        "cutoff_instant": {"us": us}, "amount_msat": msat, "fee_msat": 0,
        "rate": "40000E0", "value": null, "coarse": false,
        "kind": if direction == "inbound" { "buy" } else { "sell" }, "text": id,
        "regime_override": null, "category_override": null, "taxability_off": false,
        "privacy_hop": false, "hold": null, "chain_order": null,
    });
    for (key, value) in fields.as_object().unwrap() {
        row[key] = value.clone();
    }
    row
}

#[test]
fn a_rules_countrys_expenses_and_move_rows_decode() {
    let btc = 100_000_000_000u64;
    let request = json!({
        "schema_version": 2, "operation": "ledger",
        "policy": {"country": {"kind": "rules", "timezone": "Europe/Berlin",
                               "holding": "more_than_one_year", "tax_year_start": "04-06",
                               "pool_scope": "wallet",
                               "fee_units": "disposal_before_sale",
                               "swap_values": "received_given", "gifts_given": "market_value",
                               "donations_given": "market_value", "gifts_received": "quarantine",
                               "gift_losses": "not_deductible"},
                   "method": "fifo", "method_label": "FIFO", "require_coarse_review": false,
                   "pool_scope": "wallet"},
        "config": {"assets": ["BTC"], "exchanges": ["A", "B"], "holders": ["H"]},
        "holder": "H",
        "wallets": [{"id": "A", "label": "A"}, {"id": "B", "label": "B"}],
        "rows": [
            rules_row("b1", "A", "inbound", "2024-01-10", btc, json!({})),
            rules_row("m1-out", "A", "outbound", "2024-02-01", btc / 2,
                      json!({"anchor": "m1", "kind": "transfer", "fee_msat": btc / 10_000})),
            rules_row("m1-in", "B", "inbound", "2024-02-01", btc / 2,
                      json!({"anchor": "m1", "kind": "transfer"})),
            rules_row("s1", "B", "outbound", "2024-03-01", btc / 4,
                      json!({"fee_msat": btc / 10_000, "rate": "60000E0"})),
        ],
        "intra_pairs": [{"out": "custody-tax:m1-out", "in": "custody-tax:m1-in",
                         "source": "reviewed_custody_component", "group_id": null}],
        "cross_asset_pairs": [],
    });
    let response = compare("rules", &request)["ok"].clone();
    let response: Value = serde_json::from_str(response.as_str().unwrap()).unwrap();
    let summary: Vec<&str> = response["tax_summary"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["transaction_type"].as_str().unwrap())
        .collect();
    assert_eq!(summary, ["fee", "move", "sell"]);
    let sale = response["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["entry_type"] == "disposal")
        .unwrap();
    assert_eq!(sale["expenses"], "60000E-4");
    assert_eq!(sale["fragments"][0]["expenses"], "60000E-4");
}
