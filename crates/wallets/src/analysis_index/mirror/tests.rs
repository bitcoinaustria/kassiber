use super::*;
use crate::analysis_index::fixture::tables;
use kassiber_pyjson::{dumps, Options};
use kassiber_store::rusqlite::Connection;
fn canonical(v: &Value) -> String {
    dumps(v, &Options::new().sort_keys(true).compact()).unwrap()
}
#[test]
fn mirror_matches_python_report_bytes_and_all_112_tables() {
    let corpus =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_mirror.json")).unwrap();
    let cases = list(field(&corpus, "cases"));
    assert!(cases.len() >= 140);
    for case in cases {
        compare(&corpus, case);
    }
}
fn database(corpus: &Value, case: &Value) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
        .unwrap();
    conn.execute_batch(
        field(field(corpus, "books"), text(case, "book"))
            .as_str()
            .unwrap(),
    )
    .unwrap();
    conn
}
fn compare(corpus: &Value, case: &Value) {
    let conn = database(corpus, case);
    let before = tables(&conn);
    conn.execute_batch("SAVEPOINT caller").unwrap();
    let actual = evaluate_fixture(case, &conn).unwrap();
    assert_eq!(
        canonical(&actual),
        canonical(field(case, "expected")),
        "{}",
        text(case, "name")
    );
    assert_eq!(tables(&conn), before);
    conn.execute_batch("ROLLBACK TO caller; RELEASE caller")
        .unwrap();
    assert!(conn.is_autocommit());
}

#[test]
fn mirror_relevance_attribution_budgets_and_redaction_match_python() {
    let corpus =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_mirror.json")).unwrap();
    for case in list(field(&corpus, "cases")) {
        if [
            "diamond-False",
            "diamond-owned-False",
            "large-False",
            "dense-False",
            "entropy-time-budget-False",
            "public-claims-False",
            "stored-True",
        ]
        .contains(&text(case, "name"))
        {
            compare(&corpus, case);
        }
    }
}
#[test]
fn mirror_unsafe_findings_remain_coverage_only() {
    let corpus =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_mirror.json")).unwrap();
    let case = list(field(&corpus, "cases"))
        .iter()
        .find(|c| text(c, "name") == "stale-False")
        .unwrap();
    let conn = database(&corpus, case);
    let before = tables(&conn);
    super::super::views::with_read(&conn, "p", "owner", |owner| {
        let public = owner.for_observer("public")?;
        let analysis = analyze_snapshot(owner, &query())?;
        let stale = list(field(&analysis, "nodes"))
            .iter()
            .find(|n| text(n, "status") == "stale")
            .unwrap();
        let row = dict([
            ("id", s("stale")),
            ("code", s("script_reuse")),
            ("node_ids", Value::List(vec![field(stale, "id").clone()])),
        ]);
        assert!(findings(
            owner,
            &public,
            &BTreeSet::new(),
            &dict([("findings", Value::List(vec![row]))])
        )?
        .is_empty());
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&conn), before);
}
