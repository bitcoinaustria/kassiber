use kassiber_journals::boundary;
use serde_json::Value;

fn check(group: &str) {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("data/render-branches.json")).unwrap();
    let mut count = 0;
    let mut failures = vec![];
    for case in cases.iter().filter(|case| case["group"] == group) {
        count += 1;
        let response = boundary::ledger_response(&case["response"]).unwrap();
        let ids = case["entry_ids"].as_array().unwrap();
        let mut consumed = 0;
        let actual = boundary::build(
            &case["profile"],
            &case["custody"],
            case["wallets"].as_array().unwrap(),
            &response,
            || {
                let id = ids.get(consumed).and_then(Value::as_str).unwrap_or("");
                consumed += 1;
                id.into()
            },
        );
        let matches = match actual {
            Ok(state) => {
                let actual = boundary::canonical(&state).unwrap();
                let expected = case["expected"]["state"].as_str().unwrap_or("");
                if actual != expected {
                    let actual: Value = serde_json::from_str(&actual).unwrap();
                    let expected: Value = serde_json::from_str(expected).unwrap_or(Value::Null);
                    failures.push(format!(
                        "{}: actual review {}, expected {}",
                        case["name"],
                        actual["ownership_review_candidates"],
                        expected["ownership_review_candidates"]
                    ));
                }
                actual == expected
            }
            Err(error) => {
                let actual = serde_json::to_value(error).unwrap();
                if actual != case["expected"]["error"] {
                    failures.push(format!(
                        "{}: actual {actual}, expected {}",
                        case["name"], case["expected"]
                    ));
                }
                actual == case["expected"]["error"]
            }
        };
        if consumed != ids.len() {
            failures.push(format!("{}: UUIDs {consumed}/{}", case["name"], ids.len()));
        }
        assert!(matches || !failures.is_empty());
    }
    assert!(count > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn component_exclusions_and_reason_precedence_match_python() {
    check("exclusions");
}
