use super::fixture::tables;
use super::*;
use kassiber_store::rusqlite::Connection;

fn canonical(v: &Value) -> String {
    dumps(v, &Options::new().sort_keys(true).compact()).unwrap()
}
fn run(group: &str) {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_index.json")).unwrap();
    let mut selected = 0;
    for case in list(&cases)
        .iter()
        .filter(|c| field(c, "group") == &s(group))
    {
        selected += 1;
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(field(case, "sql").as_str().unwrap_or(""))
            .unwrap();
        let before = tables(&conn);
        conn.execute_batch("SAVEPOINT captured_read").unwrap();
        let actual = fixture::evaluate(case, &conn).unwrap();
        assert_eq!(
            canonical(&actual),
            canonical(field(case, "expected")),
            "{}",
            field(case, "name").as_str().unwrap()
        );
        assert_eq!(tables(&conn), before);
        conn.execute_batch("ROLLBACK TO captured_read; RELEASE captured_read")
            .unwrap();
    }
    assert!(selected > 20, "empty selection: {group}");
}
#[test]
fn physical_occurrences_match_python_and_all_112_tables() {
    run("occurrences");
}
#[test]
fn physical_subjects_match_python_and_preserve_qualifiers() {
    run("parse");
}
#[test]
fn public_observer_matches_python_and_all_112_tables() {
    run("observer");
}

#[test]
fn immutable_snapshot_rejects_malformed_indexes_and_retains_lookup() {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_index.json")).unwrap();
    let case = list(&cases)
        .iter()
        .find(|v| field(v, "group") == &s("observer"))
        .unwrap();
    let snapshot = field(case, "snapshot").clone();
    let index = AnalysisIndex::from_snapshot(snapshot.clone()).unwrap();
    for (key, value) in fields(field(&snapshot, "nodes")) {
        assert_eq!(index.lookup("nodes", key), Some(value));
    }
    assert!(index.lookup("nodes", "missing").is_none());
    for key in [
        "nodes",
        "edges",
        "incoming",
        "outgoing",
        "subjects",
        "transaction_facts",
        "output_facts",
        "coverage",
        "snapshot_id",
        "findings",
        "labels",
        "profile_seeds",
    ] {
        let mut bad = snapshot.clone();
        kassiber_envelope::set(&mut bad, key, Value::Null);
        assert!(AnalysisIndex::from_snapshot(bad).is_err(), "{key}");
    }
    for (map, value) in [
        ("nodes", dict([("bad", Value::List(vec![]))])),
        ("outgoing", dict([("bad", dict::<&str>([]))])),
        (
            "nodes",
            dict([
                ("duplicate", dict::<&str>([])),
                ("duplicate", dict::<&str>([])),
            ]),
        ),
    ] {
        let mut bad = snapshot.clone();
        kassiber_envelope::set(&mut bad, map, value);
        assert!(AnalysisIndex::from_snapshot(bad).is_err());
    }
    for map in [
        "nodes",
        "edges",
        "outgoing",
        "incoming",
        "transaction_facts",
        "output_facts",
    ] {
        let mut bad = snapshot.clone();
        let value = if ["outgoing", "incoming"].contains(&map) {
            Value::List(vec![])
        } else {
            dict::<&str>([])
        };
        let key =
            kassiber_pyjson::Key::Str(kassiber_pyjson::PyStr::from_code_points([0xd800]).unwrap());
        kassiber_envelope::set(&mut bad, map, Value::Dict(vec![(key, value)]));
        assert!(AnalysisIndex::from_snapshot(bad).is_err(), "{map}");
    }
}

#[test]
fn confirmation_overlay_matches_python_and_all_112_tables() {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_ingestion.json")).unwrap();
    let mut selected = 0;
    for case in list(&cases)
        .iter()
        .filter(|c| field(c, "group") == &s("confirmations"))
    {
        selected += 1;
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(field(case, "sql").as_str().unwrap())
            .unwrap();
        let before = tables(&conn);
        conn.execute_batch("SAVEPOINT confirmation_read").unwrap();
        let actual = fixture::evaluate(case, &conn).unwrap();
        assert_eq!(
            canonical(&actual),
            canonical(field(case, "expected")),
            "{}",
            field(case, "name").as_str().unwrap()
        );
        assert_eq!(tables(&conn), before);
        conn.execute_batch("ROLLBACK TO confirmation_read; RELEASE confirmation_read")
            .unwrap();
    }
    assert!(selected > 70);
}

#[test]
fn observation_ingestion_matches_python_variants_and_all_112_tables() {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_ingestion.json")).unwrap();
    let mut selected = 0;
    for case in list(&cases)
        .iter()
        .filter(|c| field(c, "group") == &s("ingestion"))
    {
        selected += 1;
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(field(case, "sql").as_str().unwrap())
            .unwrap();
        let before = tables(&conn);
        conn.execute_batch("SAVEPOINT ingestion_read").unwrap();
        let actual = fixture::evaluate(case, &conn).unwrap();
        let expected = if list(field(case, "variants")).is_empty() {
            vec![field(case, "expected")]
        } else {
            list(field(case, "variants")).iter().collect()
        };
        assert!(
            expected.iter().any(|v| canonical(v) == canonical(&actual)),
            "{}\nactual={}\nexpected={}",
            field(case, "name").as_str().unwrap(),
            canonical(&actual),
            canonical(expected[0])
        );
        assert_eq!(tables(&conn), before);
        conn.execute_batch("ROLLBACK TO ingestion_read; RELEASE ingestion_read")
            .unwrap();
    }
    assert!(selected > 200);
}

#[test]
fn confirmation_cache_stays_with_its_caller_snapshot_and_rolls_back() {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_ingestion.json")).unwrap();
    let case = list(&cases)
        .iter()
        .find(|c| field(c, "name") == &s("anchored"))
        .unwrap();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
        .unwrap();
    conn.execute_batch(field(case, "sql").as_str().unwrap())
        .unwrap();
    let before = tables(&conn);
    conn.execute_batch("SAVEPOINT owned_transaction").unwrap();
    conn.execute("UPDATE profiles SET label='caller-write' WHERE id='p'", [])
        .unwrap();
    conn.execute_batch("SAVEPOINT confirmation_read").unwrap();
    let mut overlay = confirmations::ConfirmationOverlay::read(&conn, "p").unwrap();
    let node = &list(field(case, "nodes"))[0];
    let old = overlay.apply(node).unwrap();
    conn.execute(
        "UPDATE chain_analysis_acquisition_grants SET verified_tip_height=120 WHERE id='g'",
        [],
    )
    .unwrap();
    assert_eq!(overlay.apply(node).unwrap(), old);
    let fresh = confirmations::ConfirmationOverlay::read(&conn, "p")
        .unwrap()
        .apply(node)
        .unwrap();
    assert_eq!(field(&fresh, "confirmations"), &Value::Int(21.into()));
    conn.execute_batch("ROLLBACK TO confirmation_read; RELEASE confirmation_read")
        .unwrap();
    assert_eq!(
        confirmations::ConfirmationOverlay::read(&conn, "p")
            .unwrap()
            .apply(node)
            .unwrap(),
        old
    );
    assert_eq!(
        conn.query_row("SELECT label FROM profiles WHERE id='p'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "caller-write"
    );
    assert!(!conn.is_autocommit());
    conn.execute_batch("ROLLBACK TO owned_transaction; RELEASE owned_transaction")
        .unwrap();
    assert_eq!(tables(&conn), before);
}

#[test]
fn observation_ingestion_accepts_ignored_legacy_values_without_writes() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
        .unwrap();
    let before = tables(&conn);
    for raw in [
        r#"{"unexpected":NaN}"#,
        r#"{"unexpected":Infinity}"#,
        r#"{"unexpected":"\ud800"}"#,
    ] {
        let mut builder = ingestion::ObservationBuilder::read(&conn, "p").unwrap();
        let row = super::value::json!({"id":"legacy","raw_json":raw});
        builder.transaction(&row, false).unwrap();
        let row = super::value::json!({"payload_json":raw});
        builder.reference(&row, true).unwrap();
        assert_eq!(tables(&conn), before);
    }
}

#[test]
fn sql_sources_match_python_variants_and_all_112_tables() {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_sources.json")).unwrap();
    assert!(list(&cases).len() > 100);
    for case in list(&cases) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(field(case, "sql").as_str().unwrap())
            .unwrap();
        let before = tables(&conn);
        conn.execute_batch("SAVEPOINT source_read").unwrap();
        let actual = fixture::evaluate(case, &conn).unwrap();
        let variants = list(field(case, "variants"));
        let expected = if variants.is_empty() {
            vec![field(case, "expected")]
        } else {
            variants.iter().collect()
        };
        assert!(
            expected.iter().any(|v| canonical(v) == canonical(&actual)),
            "{}\nactual={}\nexpected={}",
            field(case, "name").as_str().unwrap(),
            canonical(&actual),
            canonical(expected[0])
        );
        assert_eq!(tables(&conn), before);
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK TO source_read; RELEASE source_read")
            .unwrap();
    }
}

#[test]
fn legacy_sql_sources_match_python_bytes_and_all_112_tables() {
    assert!(run_legacy(&[]) > 400);
}

fn run_legacy(prefixes: &[&str]) -> usize {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_legacy.json")).unwrap();
    let mut selected = 0;
    for case in list(&cases).iter().filter(|case| {
        prefixes.is_empty()
            || prefixes
                .iter()
                .any(|prefix| field(case, "name").as_str().unwrap().starts_with(prefix))
    }) {
        selected += 1;
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(field(case, "sql").as_str().unwrap())
            .unwrap();
        let before = tables(&conn);
        let actual = fixture::evaluate(case, &conn).unwrap();
        assert_eq!(
            canonical(&actual),
            canonical(field(case, "expected")),
            "{}",
            field(case, "name").as_str().unwrap()
        );
        assert!(conn.is_autocommit());
        assert_eq!(tables(&conn), before);
    }
    assert!(selected > 0);
    selected
}

#[test]
fn legacy_json_preserves_opaque_values_and_commitments() {
    run_legacy(&["ignored-", "source-record-", "status-", "surrogate-key-"]);
}

#[test]
fn legacy_json_preserves_observer_aliases() {
    run_legacy(&[
        "surrogate-address-",
        "output-scriptpubkey_address-",
        "surrogate-coinbase-",
    ]);
}

#[test]
fn legacy_json_preserves_invalid_identifiers_and_scope() {
    run_legacy(&[
        "surrogate-txid-fallback-",
        "config-ignored-",
        "raw-network-",
        "surrogate-boundary-",
    ]);
}

#[test]
fn sql_sources_preserve_caller_writes_and_release_failed_reads() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
        .unwrap();
    let before = tables(&conn);
    conn.execute_batch("SAVEPOINT caller_source_work").unwrap();
    conn.execute("UPDATE profiles SET label='caller-write' WHERE id='p'", [])
        .unwrap();
    let index = sources::build_index(&conn, "p", "owner").unwrap();
    assert!(index.lookup("nodes", "bitcoin:main:record:in").is_some());
    assert!(!conn.is_autocommit());
    assert!(sources::build_index(&conn, "p", "invalid").is_err());
    assert!(!conn.is_autocommit());
    let label: String = conn
        .query_row("SELECT label FROM profiles WHERE id='p'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(label, "caller-write");
    conn.execute_batch("ROLLBACK TO caller_source_work; RELEASE caller_source_work")
        .unwrap();
    assert_eq!(tables(&conn), before);
    assert!(conn.is_autocommit());
    assert!(sources::build_index(&conn, "p", "invalid").is_err());
    assert!(conn.is_autocommit());
    assert_eq!(tables(&conn), before);
}

#[test]
fn sql_sources_refuse_malformed_dataset_mappings_without_writes() {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_sources.json")).unwrap();
    let case = list(&cases)
        .iter()
        .find(|case| field(case, "name") == &s("dataset-matches"))
        .unwrap();
    for (column, table, values) in [
        (
            "manifest_json",
            "chain_analysis_datasets",
            &[
                "broken",
                "[]",
                "null",
                "1",
                "{}",
                r#"{"valid_from":1,"valid_until":null,"source":"s","attribution_method":"a","license":"l"}"#,
                r#"{"valid_from":null,"valid_until":null,"source":"s","attribution_method":"a","license":"l","unused":NaN}"#,
                r#"{"valid_from":null,"valid_until":null,"source":"s","attribution_method":"a","license":"l","unused":"\ud800"}"#,
            ][..],
        ),
        (
            "source_record_json",
            "chain_analysis_dataset_claims",
            &["broken", "[]", "null", "1"][..],
        ),
    ] {
        for raw in values {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(include_str!(
                "../../../reports/tests/data/privacy_hygiene_seed.sql"
            ))
            .unwrap();
            conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(field(case, "sql").as_str().unwrap())
                .unwrap();
            conn.execute(&format!("UPDATE {table} SET {column}=?"), [raw])
                .unwrap();
            let before = tables(&conn);
            let error = sources::build_index(&conn, "p", "owner").unwrap_err();
            assert_eq!(error.code, "validation");
            assert!(conn.is_autocommit());
            assert_eq!(tables(&conn), before);
        }
    }
}

#[test]
fn sql_views_match_python_bytes_and_all_112_tables() {
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_views.json")).unwrap();
    for case in list(&cases) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(field(case, "sql").as_str().unwrap())
            .unwrap();
        let before = tables(&conn);
        conn.execute_batch("SAVEPOINT caller; UPDATE profiles SET label='Pending' WHERE id='p'")
            .unwrap();
        let pending = tables(&conn);
        let actual = views::evaluate_fixture(case, &conn).unwrap();
        assert_eq!(
            canonical(&actual),
            canonical(field(case, "expected")),
            "{}",
            field(case, "name").as_str().unwrap()
        );
        assert_eq!(tables(&conn), pending);
        conn.execute_batch("ROLLBACK TO caller; RELEASE caller")
            .unwrap();
        assert_eq!(tables(&conn), before);
        assert!(conn.is_autocommit());
    }
}

#[test]
fn sql_views_refuse_missing_state_and_release_failed_reads() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
        .unwrap();
    let before = tables(&conn);
    assert!(views::with_read(&conn, "p", "owner", |_| Ok(())).is_err());
    assert_eq!(tables(&conn), before);
    assert!(conn.is_autocommit());
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_views.json")).unwrap();
    conn.execute_batch(field(&list(&cases)[0], "sql").as_str().unwrap())
        .unwrap();
    conn.execute_batch("SAVEPOINT caller; UPDATE profiles SET label='Pending' WHERE id='p'")
        .unwrap();
    let pending = tables(&conn);
    let result: Result<(), _> = views::with_read(&conn, "p", "public", |_| Err(invalid()));
    assert!(result.is_err());
    assert_eq!(tables(&conn), pending);
    views::with_read(&conn, "p", "owner", |view| {
        assert!(traversal::query_read(view, &dict([("observer", s("public"))])).is_err());
        Ok(())
    })
    .unwrap();
    conn.execute_batch("ROLLBACK TO caller; RELEASE caller")
        .unwrap();
    assert!(conn.is_autocommit());
}

#[test]
fn sql_views_bound_high_degree_lookups_without_writes() {
    use views::GraphRead;
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
        .unwrap();
    let cases =
        kassiber_pytext::loads(include_str!("../../tests/data/analysis_views.json")).unwrap();
    let case = list(&cases)
        .iter()
        .find(|c| field(c, "name") == &s("stored-owner-0"))
        .unwrap();
    conn.execute_batch(field(case, "sql").as_str().unwrap())
        .unwrap();
    conn.execute_batch("WITH RECURSIVE n(x) AS (SELECT 0 UNION ALL SELECT x+1 FROM n WHERE x<7000) INSERT INTO chain_index_edges SELECT 'p',printf('edge%05d',x),'hub','other','{\"kind\":\"custody\"}','{\"kind\":\"custody\"}' FROM n;
    INSERT INTO chain_index_edges VALUES('p','zz-spend','hub','other','{\"kind\":\"spends\"}','{\"kind\":\"spends\"}');
    WITH RECURSIVE n(x) AS (SELECT 0 UNION ALL SELECT x+1 FROM n WHERE x<2100) INSERT INTO chain_index_aliases SELECT 'p','public','wide',printf('node%05d',x) FROM n;").unwrap();
    let before = tables(&conn);
    views::with_read(&conn, "p", "public", |view| {
        assert_eq!(list(&view.get("outgoing", "hub")?).len(), 6002);
        assert!(view.has_spend("hub")?);
        assert_eq!(
            view.get("subjects", "wide").unwrap_err().code,
            "chain_analysis_budget"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&conn), before);
    assert!(conn.is_autocommit());
}
