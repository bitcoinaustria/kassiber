use kassiber_journals::{boundary, enrich_quarantine_detail};
use serde_json::{json, Value};
fn corpus(group: &str) {
    let cases: Vec<Value> = serde_json::from_str(include_str!("data/render.json")).unwrap();
    let mut count = 0;
    for c in cases.iter().filter(|c| c["group"] == group) {
        count += 1;
        let response = if c["response"].is_null() {
            Ok(boundary::empty_response())
        } else {
            boundary::ledger_response(&c["response"])
        };
        let mut ids = c["entry_ids"].as_array().unwrap().iter();
        let state = response.and_then(|r| {
            boundary::build(
                &c["profile"],
                &c["custody"],
                c["wallets"].as_array().unwrap(),
                &r,
                || ids.next().unwrap().as_str().unwrap().to_owned(),
            )
        });
        match state {
            Ok(state) => {
                assert_eq!(
                    boundary::canonical(&state).unwrap(),
                    c["expected"]["state"].as_str().unwrap(),
                    "{}",
                    c["name"]
                );
                assert!(
                    ids.next().is_none(),
                    "UUID tape not consumed exactly: {}",
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
    }
    assert!(count > 0);
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
}
