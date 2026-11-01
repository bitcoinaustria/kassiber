use kassiber_journals::{native, scalar};
use scalar::Value;

#[path = "support/fidelity.rs"]
mod support;
use support::{cases, list};

#[test]
fn nonfinite_operands_match_python_consumption_and_uuid_tapes() {
    let cases = cases(include_str!("data/render-nonfinite.json"));
    assert_eq!(cases.len(), 1776);
    let field = native::field;
    let mut failures = vec![];
    for case in cases {
        let name = scalar::repr(field(&case, "name"));
        let ids = list(field(&case, "entry_ids"));
        let custody = field(&case, "custody");
        for ledger_only in [true, false] {
            let mut consumed = 0;
            let identity = || {
                let id = ids.get(consumed).map(scalar::text).unwrap_or_default();
                consumed += 1;
                id.as_str().unwrap_or_default().to_owned()
            };
            let projection = field(custody, "finalized_tax_projection");
            let actual = if ledger_only {
                native::read_ledger(
                    field(&case, "profile"),
                    projection,
                    field(custody, "wallet_refs_by_id"),
                    list(field(custody, "direct_payout_records")),
                    field(&case, "response"),
                    identity,
                )
                .map(|ledger| {
                    (
                        "ledger",
                        scalar::encode(&ledger.snapshot(projection)).unwrap(),
                    )
                })
            } else {
                native::build(
                    field(&case, "profile"),
                    custody,
                    field(&case, "response"),
                    identity,
                )
                .map(|state| ("state", scalar::encode(&state.snapshot()).unwrap()))
            };
            let actual = match actual {
                Ok((key, state)) => scalar::dict([(key, Value::from(state))]),
                Err(error) => scalar::dict([("error", error.0)]),
            };
            let expected = if ledger_only {
                field(field(&case, "ledger"), "expected")
            } else {
                field(&case, "expected")
            };
            let expected_consumed = if ledger_only {
                scalar::text(field(field(&case, "ledger"), "consumed"))
                    .as_str()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap()
            } else {
                ids.len()
            };
            if scalar::encode(&actual).unwrap() != scalar::encode(expected).unwrap()
                || consumed != expected_consumed
            {
                failures.push(format!(
                    "{name} ledger={ledger_only}: UUID {consumed}/{expected_consumed}, actual {}, expected {}",
                    scalar::encode(&actual).unwrap(), scalar::encode(expected).unwrap()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
