use kassiber_envelope::{get, py_str};
use kassiber_pyjson::Value;
use kassiber_transfers::review_delta::{tax_delta, Fault};
use kassiber_transfers::review_operations::canonical;

#[test]
fn tax_delta_matches_python_on_planned_and_untrusted_effects() {
    let Value::List(cases) = kassiber_pytext::loads(include_str!("review_delta.json")).unwrap()
    else {
        panic!("delta fixtures")
    };
    let (mut deltas, mut faults) = (0, 0);
    let mut mismatches = Vec::new();
    for case in &cases {
        let before = get(case, "before").unwrap();
        let after = get(case, "after").unwrap();
        let actual = tax_delta(before, after);
        match get(case, "delta") {
            Some(expected) => {
                let expected = canonical(expected).unwrap();
                match actual {
                    Ok(actual) if canonical(&actual).unwrap() == expected => (),
                    other => mismatches.push(format!("{other:?} != {expected} for {case:?}")),
                }
                deltas += 1;
            }
            None => {
                let expected = match py_str(get(case, "fault").unwrap()).as_str() {
                    "malformed" => Fault::Malformed,
                    _ => Fault::Internal(py_str(get(case, "message").unwrap())),
                };
                if actual.as_ref().err() != Some(&expected) {
                    mismatches.push(format!("{actual:?} != {expected:?} for {case:?}"));
                }
                faults += 1;
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    assert_eq!((deltas, faults), (91, 100));
}
