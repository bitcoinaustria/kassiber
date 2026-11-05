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
