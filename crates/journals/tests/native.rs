use kassiber_journals::{native, scalar};
use scalar::Value;

fn list(value: &Value) -> &[Value] {
    let Value::List(values) = value else {
        panic!("expected list")
    };
    values
}

#[test]
fn native_ledger_matches_python_consumption_and_scalars() {
    let sources = [
        ("render.json", include_str!("data/render.json")),
        (
            "render-legacy-floats.json",
            include_str!("data/render-legacy-floats.json"),
        ),
        (
            "render-response-refusals.json",
            include_str!("data/render-response-refusals.json"),
        ),
        (
            "render-scalar-coercions.json",
            include_str!("data/render-scalar-coercions.json"),
        ),
    ]
    .map(|(name, raw)| (name, scalar::decode(raw).unwrap()));
    let expected: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/render-native-ledger.json")).unwrap();
    let mut failures = vec![];
    for witness in expected {
        let source = witness["source"].as_str().unwrap();
        let case = &list(&sources.iter().find(|(name, _)| *name == source).unwrap().1)
            [witness["index"].as_u64().unwrap() as usize];
        let field = native::field;
        let projection = field(field(case, "custody"), "finalized_tax_projection");
        let ids = list(field(case, "entry_ids"));
        let mut consumed = 0;
        let result = native::read_ledger(
            field(case, "profile"),
            projection,
            field(field(case, "custody"), "wallet_refs_by_id"),
            list(field(field(case, "custody"), "direct_payout_records")),
            field(case, "response"),
            || {
                let id = ids
                    .get(consumed)
                    .map(scalar::text)
                    .and_then(|s| s.as_str().map(str::to_owned))
                    .unwrap_or_default();
                consumed += 1;
                id
            },
        );
        let expected = &witness["expected"];
        let actual = match result {
            Ok(ledger) => {
                serde_json::json!({"ledger":scalar::encode(&ledger.snapshot(projection)).unwrap()})
            }
            Err(error) => {
                serde_json::json!({"error":serde_json::from_str::<serde_json::Value>(&scalar::encode(&error.0).unwrap()).unwrap()})
            }
        };
        if actual != *expected || consumed != witness["consumed"].as_u64().unwrap() as usize {
            failures.push(format!(
                "{}: consumed {consumed}/{}, actual {}, expected {}",
                witness["name"], witness["consumed"], actual, expected
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
