//! Frozen CPython outputs, selectable by scenario for mutation witnesses.
use kassiber_custody::gaps::{self, Limits};
use serde_json::Value;
use std::collections::BTreeSet;

fn vectors() -> &'static Value {
    static V: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_gap_cases.json"
        ))
        .unwrap()
    })
}
fn check(case: &Value) {
    let limits: Limits = serde_json::from_value(case["limits"].clone()).unwrap();
    let ignored: BTreeSet<String> = serde_json::from_value(case["ignored"].clone()).unwrap();
    let result = gaps::search(case["rows"].as_array().unwrap(), &ignored, &limits);
    if let Some(error) = case["error"].as_str() {
        assert_eq!(result.unwrap_err(), error, "{}", case["case"]);
    } else {
        assert_eq!(
            result.unwrap().value(),
            case["expected"],
            "{}",
            case["case"]
        );
    }
}
#[test]
fn gaps_search_vectors() {
    for case in vectors()["search"].as_array().unwrap() {
        check(case);
    }
}
#[test]
fn gaps_compute_vectors() {
    for case in vectors()["compute"].as_array().unwrap() {
        let ignored = serde_json::from_value(case["ignored_transaction_ids"].clone()).unwrap();
        let accounting =
            serde_json::from_value(case["accounting_ignored_transaction_ids"].clone()).unwrap();
        let result = gaps::compute(
            case["rows"].as_array().unwrap(),
            case["large_book"].as_bool().unwrap(),
            &ignored,
            &accounting,
            250,
        );
        assert_eq!(result.unwrap(), case["expected"]);
    }
}
#[test]
fn gaps_fingerprint_vectors() {
    for case in vectors()["fingerprints"].as_array().unwrap() {
        assert_eq!(
            gaps::candidate_fingerprint(&case["candidate"]).unwrap(),
            case["candidate_fingerprint"]
        );
        assert_eq!(
            gaps::authored_claim_fingerprint(&case["candidate"]).unwrap(),
            case["authored_claim_fingerprint"]
        );
    }
}
#[test]
fn gaps_review_revision_vectors() {
    for case in vectors()["reviews"].as_array().unwrap() {
        assert_eq!(
            gaps::dismissed_fingerprints(case["rows"].as_array().unwrap()),
            case["expected"]
        );
    }
}
#[test]
fn gaps_snapshot_vectors() {
    for case in vectors()["display"].as_array().unwrap() {
        assert_eq!(
            gaps::snapshot_gap(&case["candidate"], case["rows"].as_array().unwrap()).unwrap(),
            case["expected"]
        );
    }
}
#[test]
fn gaps_cursor_vectors() {
    for case in vectors()["cursors"].as_array().unwrap() {
        let actual = gaps::pagination::decode_cursor(case["cursor"].as_str().unwrap());
        if case.get("error").is_some() {
            assert!(actual.is_err(), "{}", case["case"]);
        } else {
            // Preserve lone-surrogate cursor ids through the CPython codec.
            let mut expected =
                kassiber_custody::codec::loads(case["expected_json"].as_str().unwrap()).unwrap();
            // ASCII JSON joins adjacent escaped surrogates. Keep the oracle's
            // actual code points for raw surrogatepass cursor byte inputs.
            if let kassiber_pyjson::Value::List(items) = &mut expected {
                let points: Vec<u32> =
                    serde_json::from_value(case["expected_id_codepoints"].clone()).unwrap();
                items[2] = kassiber_pyjson::Value::Str(
                    kassiber_pyjson::PyStr::from_code_points(points).unwrap(),
                );
            }
            assert_eq!(actual.unwrap(), expected, "{}", case["case"]);
        }
    }
}
#[test]
fn gaps_seeded_pagination_vectors() {
    for case in vectors()["pages"].as_array().unwrap() {
        let actual = gaps::pagination::page_candidates(
            &case["version"],
            case["population"].as_array().unwrap(),
            case["limit"].as_u64().unwrap() as usize,
            case["cursor"].as_str(),
            case["gap_id"].as_str(),
        );
        if case.get("error").is_some() {
            assert!(actual.is_err());
        } else {
            assert_eq!(actual.unwrap(), case["expected"]);
        }
    }
}

#[test]
fn gaps_adversarial_timing_smoke() {
    use serde_json::json;
    let mut rows:Vec<_>=(0..60_000).map(|i|json!({"id":format!("past-{i}"),"profile_id":"p","wallet_id":"new","direction":"inbound","asset":"BTC","amount":1_000_000,"fee":0,"occurred_at":"2021-01-01T00:00:00Z"})).collect();
    rows.extend((0..300).map(|i|json!({"id":format!("source-{i}"),"profile_id":"p","wallet_id":"old","direction":"outbound","asset":"BTC","amount":1_000_000,"fee":0,"occurred_at":"2022-01-01T00:00:00Z","privacy_boundary":"coinjoin"})));
    rows.push(json!({"id":"future","profile_id":"p","wallet_id":"new","direction":"inbound","asset":"BTC","amount":990_000,"fee":0,"occurred_at":"2023-01-01T00:00:00Z"}));
    let started = std::time::Instant::now();
    let actual = gaps::search(&rows, &BTreeSet::new(), &Limits::default()).unwrap();
    assert!(started.elapsed().as_secs_f64() < 20.0);
    assert!(!actual.search_complete);
    assert_eq!(actual.blocking_source_ids.len(), 300);
    assert_eq!(actual.candidates.len(), 250);
    assert_eq!(actual.candidate_count, 251);
}

#[test]
fn gaps_worklist_ranges_vectors() {
    for case in vectors()["worklist_ranges"].as_array().unwrap() {
        let actual = gaps::worklist_amount_ranges(case["rows"].as_array().unwrap());
        if let Some(error) = case["error"].as_str() {
            assert_eq!(actual.unwrap_err(), error);
        } else {
            assert_eq!(
                serde_json::to_value(
                    actual
                        .unwrap()
                        .into_iter()
                        .map(|(asset, low, high)| serde_json::json!([
                            asset,
                            low.to_string().parse::<serde_json::Value>().unwrap(),
                            high.to_string().parse::<serde_json::Value>().unwrap()
                        ]))
                        .collect::<Vec<_>>()
                )
                .unwrap(),
                case["expected"]
            );
        }
    }
}

#[test]
fn gaps_compute_admission_precedes_ignored_ids() {
    use serde_json::json;
    for (amount, when, error) in [
        (json!("²"), "2020-01-01T00:00:00Z", "ValueError"),
        (json!(1000), "0001-01-01T00:00:00+01:00", "OverflowError"),
    ] {
        let rows = vec![
            json!({"id":"out","profile_id":"p","wallet_id":"old","direction":"outbound","asset":"BTC","amount":amount,"fee":0,"occurred_at":when}),
        ];
        let ignored = BTreeSet::from(["out".to_owned()]);
        assert_eq!(
            gaps::compute(&rows, false, &ignored, &ignored, 250).unwrap_err(),
            error
        );
    }
}
