use super::*;

fn compare(prefix: &str) {
    let cases = kassiber_pytext::loads(include_str!(
        "../../../tests/data/chain_analysis_reads.json"
    ))
    .unwrap();
    let mut checked = 0;
    for case in list(&cases)
        .iter()
        .filter(|c| text(c, "name").starts_with(prefix))
    {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(text(case, "sql")).unwrap();
        let result = fixture::evaluate(case, &conn).unwrap();
        let expected = list(field(case, "expected"));
        assert_eq!(list(&result).len(), expected.len());
        for (step, (actual, expected)) in list(&result).iter().zip(expected).enumerate() {
            let a = text(actual, "wire");
            let b = text(expected, "wire");
            if a != b {
                let at = a
                    .bytes()
                    .zip(b.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(a.len().min(b.len()));
                panic!(
                    "{} step {step} envelope byte {at}: native {:?}, Python {:?}",
                    text(case, "name"),
                    a.get(at.saturating_sub(50)..(at + 160).min(a.len())),
                    b.get(at.saturating_sub(50)..(at + 160).min(b.len()))
                );
            }
            assert_eq!(
                json(field(actual, "state")).unwrap(),
                json(field(expected, "state")).unwrap(),
                "{} step {step} all tables",
                text(case, "name")
            );
            assert_eq!(
                field(actual, "in_transaction"),
                field(expected, "in_transaction"),
                "{} step {step} transaction",
                text(case, "name")
            );
        }
        assert!(conn.is_autocommit());
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn chain_analysis_query_retains_generation_observer_scope_and_rollback() {
    compare("query");
}
#[test]
fn chain_analysis_entropy_preserves_validation_and_physical_subjects() {
    compare("entropy");
}
#[test]
fn chain_analysis_cases_verify_integrity_and_compare_all_surfaces() {
    compare("cases");
}
#[test]
fn chain_analysis_datasets_bind_cursors_state_and_visibility() {
    compare("datasets");
}
#[test]
fn chain_analysis_labels_remain_ordered_and_book_scoped() {
    compare("labels");
}
