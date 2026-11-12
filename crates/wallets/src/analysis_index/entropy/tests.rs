use super::*;
use kassiber_pyjson::{dumps, Options};

#[test]
fn entropy_matches_python_captured_facts_byte_exact() {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_entropy.json")).unwrap();
    let cases = list(field(&vectors, "cases"));
    assert!(cases.len() > 100);
    let snapshots = field(&vectors, "snapshots");
    let before = snapshots.clone();
    for case in cases {
        let actual = fixture::evaluate(case, snapshots);
        let canonical = |v: &Value| dumps(v, &Options::new().sort_keys(true).compact()).unwrap();
        assert_eq!(
            canonical(&actual),
            canonical(field(case, "expected")),
            "{}",
            field(case, "name").as_str().unwrap()
        );
    }
    assert_eq!(*snapshots, before);
}

#[test]
fn entropy_logarithms_match_python_large_integers_byte_exact() {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_entropy.json")).unwrap();
    for row in list(field(&vectors, "logarithms")) {
        let n = integer(field(row, "count")).unwrap();
        let actual = dumps(&Value::Float(log2(&n)), &Options::new()).unwrap();
        assert_eq!(
            actual,
            dumps(field(row, "expected"), &Options::new()).unwrap()
        );
    }
}
