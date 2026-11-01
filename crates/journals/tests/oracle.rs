use kassiber_journals::{boundary, enrich_quarantine_detail};
use serde_json::{json, Value};
fn corpus(group: &str) {
    check_cases(include_str!("data/render.json"), Some(group));
}
fn check_cases(text: &str, group: Option<&str>) {
    let cases: Vec<Value> = serde_json::from_str(text).unwrap();
    let mut count = 0;
    for c in cases
        .iter()
        .filter(|c| group.is_none_or(|g| c["group"] == g))
    {
        count += 1;
        let response = if c["response"].is_null() {
            Ok(boundary::empty_response())
        } else {
            boundary::ledger_response(&c["response"])
        };
        let mut ids = c["entry_ids"].as_array().unwrap().iter();
        let state = match &response {
            Ok(r) if c.get("expected_audit").is_some() => {
                let inputs = kassiber_journals::LedgerInputs {
                    profile: &c["profile"],
                    projection: &c["custody"]["finalized_tax_projection"],
                    wallets: c["wallets"].as_array().unwrap(),
                    direct_payout_records: c["custody"]["direct_payout_records"]
                        .as_array()
                        .unwrap(),
                };
                kassiber_journals::render_ledger(&inputs, r, || {
                    ids.next().unwrap().as_str().unwrap().to_owned()
                })
                .and_then(|ledger| {
                    if let Some(expected) = c.get("expected_audit") {
                        let canonical = |value: &Value| {
                            kassiber_pyjson::dumps(
                                &kassiber_custody::codec::from_json(value).unwrap(),
                                &kassiber_pyjson::Options::new().sort_keys(true).compact(),
                            )
                            .unwrap()
                        };
                        assert_eq!(
                            canonical(&serde_json::to_value(&ledger.intra_audit).unwrap()),
                            canonical(expected),
                            "{}",
                            c["name"]
                        );
                    }
                    boundary::finish(&c["custody"], ledger)
                })
            }
            Ok(r) => boundary::build(
                &c["profile"],
                &c["custody"],
                c["wallets"].as_array().unwrap(),
                r,
                || ids.next().unwrap().as_str().unwrap().to_owned(),
            ),
            Err(error) => Err(error.clone()),
        };
        match state {
            Ok(state) => {
                assert_eq!(
                    boundary::canonical(&state).unwrap(),
                    c["expected"]["state"].as_str().unwrap(),
                    "{}",
                    c["name"]
                );
            }
            Err(error) => assert_eq!(
                serde_json::to_string(&serde_json::to_value(error).unwrap()).unwrap(),
                serde_json::to_string(&c["expected"]["error"]).unwrap(),
                "{}",
                c["name"]
            ),
        }
        assert!(
            ids.next().is_none(),
            "UUID tape not consumed exactly: {}",
            c["name"]
        );
    }
    assert!(count > 0);
}
#[test]
fn legacy_audit_and_carry_floats_match_python() {
    check_cases(include_str!("data/render-legacy-floats.json"), None);
}
#[test]
fn ledger_rendering_matches_python() {
    corpus("ledger");
}
#[test]
fn ownership_reviews_and_reconciliations_match_python() {
    corpus("joins");
}
#[test]
fn refusal_mapping_matches_python() {
    corpus("refusals");
}
#[test]
fn legacy_boundary_values_and_refusal_order_match_python() {
    corpus("boundary");
}
#[test]
fn native_render_and_finish_match_combined_stage() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("data/render.json")).unwrap();
    for case in cases.iter().filter(|c| c["expected"]["state"].is_string()) {
        let response = if case["response"].is_null() {
            boundary::empty_response()
        } else {
            boundary::ledger_response(&case["response"]).unwrap()
        };
        let input = kassiber_journals::LedgerInputs {
            profile: &case["profile"],
            projection: &case["custody"]["finalized_tax_projection"],
            wallets: case["wallets"].as_array().unwrap(),
            direct_payout_records: case["custody"]["direct_payout_records"].as_array().unwrap(),
        };
        let mut ids = case["entry_ids"].as_array().unwrap().iter();
        let rendered = kassiber_journals::render_ledger(&input, &response, || {
            ids.next().unwrap().as_str().unwrap().into()
        })
        .unwrap();
        let state = boundary::finish(&case["custody"], rendered).unwrap();
        assert_eq!(
            boundary::canonical(&state).unwrap(),
            case["expected"]["state"],
            "{}",
            case["name"]
        );
        assert!(ids.next().is_none());
    }
}
#[test]
fn quarantine_candidate_enrichment_is_compact_sorted_and_ascii() {
    let candidates = vec![json!({"out_id":"a","in_id":"é"})];
    assert_eq!(
        enrich_quarantine_detail(&json!(r#"{"z":1,"a":2}"#), &candidates).unwrap(),
        r#"{"a":2,"ownership_review_candidates":[{"in_id":"\u00e9","out_id":"a"}],"z":1}"#
    );
    for invalid in [
        Value::Null,
        json!("broken"),
        json!("[]"),
        json!("null"),
        json!(1),
    ] {
        assert_eq!(enrich_quarantine_detail(&invalid, &[]).unwrap(), "{}");
    }
    assert_eq!(
        enrich_quarantine_detail(&json!("{\"ownership_review_candidates\":[1]}"), &[]).unwrap(),
        "{\"ownership_review_candidates\":[1]}"
    );
    assert_eq!(
        enrich_quarantine_detail(
            &json!(r#"{"z":NaN,"s":"\ud800","i":Infinity}"#),
            &candidates
        )
        .unwrap(),
        r#"{"i":Infinity,"ownership_review_candidates":[{"in_id":"\u00e9","out_id":"a"}],"s":"\ud800","z":NaN}"#
    );
}
