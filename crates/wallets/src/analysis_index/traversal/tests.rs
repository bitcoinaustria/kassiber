use super::*;
use kassiber_pyjson::{dumps, Options};

#[test]
fn traversal_matches_python_captured_snapshots_byte_exact() {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_traversal.json"))
            .unwrap();
    let cases = list(field(&vectors, "cases"));
    assert!(cases.len() >= 200);
    let snapshots = field(&vectors, "snapshots");
    let before = snapshots.clone();
    for case in cases {
        let actual = evaluate_fixture(case, snapshots);
        let canonical = |v: &Value| dumps(v, &Options::new().sort_keys(true).compact()).unwrap();
        assert_eq!(
            canonical(&actual),
            canonical(field(case, "expected")),
            "{}",
            text(case, "name")
        );
    }
    assert_eq!(*snapshots, before);
}
