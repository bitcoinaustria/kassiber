use super::*;

fn run(group: &str) {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_analytics.json"))
            .unwrap();
    let cases: Vec<_> = list(field(&vectors, "cases"))
        .iter()
        .filter(|case| text(case, "group") == group)
        .collect();
    assert!(cases.len() >= 100);
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

#[test]
fn clustering_matches_python_captured_snapshots_byte_exact() {
    run("clustering");
}

#[test]
fn patterns_match_python_captured_snapshots_byte_exact() {
    run("patterns");
}

#[test]
fn attribution_matches_python_captured_snapshots_byte_exact() {
    run("attribution");
}

#[test]
fn analytics_composition_matches_python_captured_snapshots_byte_exact() {
    run("full");
}
