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
        let inputs = kassiber_journals::LedgerInputs {
            profile: &case["profile"],
            projection: &case["custody"]["finalized_tax_projection"],
            rows: None,
            wallets: case["wallets"].as_array().unwrap(),
            direct_payout_records: case["custody"]["direct_payout_records"].as_array().unwrap(),
        };
        let actual = kassiber_journals::render_ledger(&inputs, &response, || {
            let id = ids.get(consumed).and_then(Value::as_str).unwrap_or("");
            consumed += 1;
            id.into()
        })
        .and_then(|ledger| {
            if let Some(expected) = case.get("expected_audit") {
                let canonical = |value: &Value| {
                    kassiber_pyjson::dumps(
                        &kassiber_custody::codec::from_json(value).unwrap(),
                        &kassiber_pyjson::Options::new().sort_keys(true).compact(),
                    )
                    .unwrap()
                };
                let actual = canonical(&serde_json::to_value(&ledger.intra_audit).unwrap());
                let expected = canonical(expected);
                if actual != expected {
                    failures.push(format!(
                        "{}: audit {actual}, expected {expected}",
                        case["name"]
                    ));
                }
            }
            boundary::finish(&case["custody"], ledger)
        });
        match actual {
            Ok(state) => {
                let actual = boundary::canonical(&state).unwrap();
                let expected = case["expected"]["state"].as_str().unwrap_or("");
                if actual != expected {
                    let actual: Value = serde_json::from_str(&actual).unwrap();
                    let expected: Value = serde_json::from_str(expected).unwrap_or(Value::Null);
                    let changed: Vec<_> = actual
                        .as_object()
                        .unwrap()
                        .iter()
                        .filter(|(key, value)| expected.get(*key) != Some(*value))
                        .map(|(key, value)| {
                            format!("{key}: actual {value}, expected {}", expected[key])
                        })
                        .collect();
                    failures.push(format!("{}: {}", case["name"], changed.join("; ")));
                }
            }
            Err(error) => {
                let actual = serde_json::to_value(error).unwrap();
                if actual != case["expected"]["error"] {
                    failures.push(format!(
                        "{}: actual {actual}, expected {}",
                        case["name"], case["expected"]
                    ));
                }
            }
        }
        if consumed != ids.len() {
            failures.push(format!("{}: UUIDs {consumed}/{}", case["name"], ids.len()));
        }
    }
    assert!(count > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn component_exclusions_and_reason_precedence_match_python() {
    check("exclusions");
}

#[test]
fn graph_scopes_owners_and_tolerances_match_python() {
    check("graph");
}

#[test]
fn balance_explanations_match_python() {
    check("balances");
}

#[test]
fn audit_quarantine_and_entry_branches_match_python() {
    check("other");
}
