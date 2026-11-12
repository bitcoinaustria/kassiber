use super::*;

#[test]
fn datasets_match_python_captured_rows_byte_exact() {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_datasets.json")).unwrap();
    let cases = list(field(&vectors, "cases"));
    assert!(cases.len() > 100);
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
fn datasets_reject_nonmapping_captured_source_records() {
    let mut vectors =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_datasets.json")).unwrap();
    let Value::Dict(top) = &mut vectors else {
        unreachable!()
    };
    let snapshots = top
        .iter_mut()
        .find(|(k, _)| k.as_str() == Some("snapshots"))
        .unwrap();
    let Value::Dict(snapshots) = &mut snapshots.1 else {
        unreachable!()
    };
    let base = snapshots
        .iter_mut()
        .find(|(k, _)| k.as_str() == Some("base"))
        .unwrap();
    let mut claims = field(&base.1, "claims").clone();
    let Value::List(rows) = &mut claims else {
        unreachable!()
    };
    for row in rows {
        kassiber_envelope::set(row, "source_record_json", s("\"not a source mapping\""));
    }
    let err = match_captured(
        field(&base.1, "packs"),
        &claims,
        &Value::List(vec![Value::List(vec![
            s("bitcoin"),
            s("main"),
            s("a".repeat(64)),
        ])]),
        "public",
        &number(5),
        &s("2026-02-01T00:00:00Z"),
    )
    .unwrap_err();
    assert_eq!(err.code, "validation");
    assert_eq!(err.message, "Invalid captured attribution source record");
}
