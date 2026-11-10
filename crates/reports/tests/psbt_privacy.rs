#![cfg(feature = "test-harness")]

use kassiber_envelope::{dict, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_reports::psbt_privacy::{report_inventory, report_with_baseline, table_rows};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::psbt_privacy::{fixture_baseline, fixture_no_egress};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn canonical(value: &Value) -> String {
    dumps(
        value,
        &Options {
            sort_keys: true,
            separators: Some((",".into(), ":".into())),
            ..Default::default()
        },
    )
    .unwrap()
}

fn tables(conn: &Connection) -> BTreeMap<String, String> {
    let names = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r| r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    // Books built from the frozen seed keep the layout it was cut from.
    assert_eq!(
        names.len(),
        include_str!("data/privacy_hygiene_seed.sql")
            .lines()
            .filter(|line| line.starts_with("CREATE TABLE "))
            .count()
    );
    names
        .into_iter()
        .map(|name| {
            let mut statement = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let columns: Vec<_> = statement
                .column_names()
                .iter()
                .map(|n| n.to_string())
                .collect();
            let mut rows = statement
                .query_map([], |row| {
                    Ok(dict(columns.iter().enumerate().map(|(i, name)| {
                        use kassiber_store::rusqlite::types::ValueRef;
                        let value = match row.get_ref(i).unwrap() {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => Value::Int(n.into()),
                            ValueRef::Real(n) => Value::Float(n),
                            ValueRef::Text(v) => s(std::str::from_utf8(v).unwrap()),
                            ValueRef::Blob(_) => panic!("fixture blob needs an explicit encoding"),
                        };
                        (name.as_str(), value)
                    })))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort_by_cached_key(canonical);
            (
                name,
                format!(
                    "{:x}",
                    Sha256::digest(canonical(&Value::List(rows)).as_bytes())
                ),
            )
        })
        .collect()
}

fn run(group: &str) {
    run_cases(group, include_str!("data/psbt_privacy.json"));
}

fn run_cases(group: &str, corpus: &str) {
    fixture_no_egress(|| {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
            .unwrap();
        let cases: Vec<serde_json::Value> = serde_json::from_str(corpus).unwrap();
        let mut count = 0;
        for case in cases.iter().filter(|v| v["group"] == group) {
            count += 1;
            conn.execute_batch("SAVEPOINT fixture").unwrap();
            conn.execute_batch(include_str!("data/book.sql")).unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = tables(&conn);
            assert_eq!(
                before,
                serde_json::from_value(case["table_sha256"].clone()).unwrap(),
                "{}: reference table bytes",
                case["name"]
            );
            let changes = conn.total_changes();
            let args = kassiber_pytext::loads(&case["args"].to_string()).unwrap();
            let actual = if case["native_history"] == true {
                kassiber_reports::psbt_privacy::report(&conn, &args, case["psbt"].as_str().unwrap())
            } else if case["native_inventory"] == true {
                report_inventory(&conn, &args, case["psbt"].as_str().unwrap())
            } else {
                report_with_baseline(
                    &conn,
                    &args,
                    case["psbt"].as_str().unwrap(),
                    |_, profile| {
                        assert_eq!(profile, case["baseline_profile"].as_str().unwrap());
                        Ok(fixture_baseline(&case["baseline"]))
                    },
                )
            };
            let actual = match actual {
                Ok(payload) => dict([("ok", payload.clone()), ("rows", table_rows(&payload))]),
                Err(e) => dict([(
                    "error",
                    dict([
                        ("code", s(e.code)),
                        ("message", s(e.message)),
                        ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                        ("details", e.details.unwrap_or(Value::Null)),
                        ("retryable", Value::Bool(e.retryable)),
                    ]),
                )]),
            };
            let actual = canonical(&actual);
            let expected =
                canonical(&kassiber_pytext::loads(&case["expected"].to_string()).unwrap());
            assert_eq!(actual, expected, "{}", case["name"]);
            assert!(
                !actual.contains("sentinel-private-"),
                "{}: private material escaped",
                case["name"]
            );
            for node in case["baseline"]["nodes"]
                .as_object()
                .into_iter()
                .flat_map(|n| n.values())
            {
                if let Some(script) = node["script_key"].as_str().filter(|s| !s.is_empty()) {
                    assert!(!actual.contains(script), "{}: script escaped", case["name"]);
                }
            }
            if case["psbt"].as_str().unwrap().len() > 30 {
                assert!(!actual.contains(case["psbt"].as_str().unwrap()));
            }
            assert_eq!(
                tables(&conn),
                before,
                "{}: every seed table after report",
                case["name"]
            );
            assert_eq!(conn.total_changes(), changes);
            assert!(!conn.is_autocommit());
            conn.execute_batch("ROLLBACK TO fixture; RELEASE fixture")
                .unwrap();
        }
        assert!(count > 0);
    });
}

#[test]
fn psbt_native_inventory_capture_matches_python_all_112_tables() {
    run_cases("native_inventory", include_str!("data/psbt_inventory.json"));
}

#[test]
fn psbt_native_inventory_refuses_unported_sources_and_bounds() {
    run_cases(
        "admission_refusal",
        include_str!("data/psbt_inventory.json"),
    );
}

#[test]
fn psbt_native_inventory_missing_sources_fail_closed_and_release_snapshot() {
    for table in [
        "transaction_graph_cache",
        "chain_analysis_observations",
        "chain_analysis_reference_assertions",
        "chain_analysis_datasets",
        "book_network_bindings",
        "transactions",
        "wallet_utxos",
    ] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
            .unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("data/psbt_inventory.json")).unwrap();
        let case = &cases[0];
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
        let before = conn.total_changes();
        let error =
            report_inventory(&conn, &dict::<&str>([]), case["psbt"].as_str().unwrap()).unwrap_err();
        assert_eq!(error.code, "internal_error", "{table}");
        assert_eq!(conn.total_changes(), before);
        assert!(conn.is_autocommit());
    }
}

#[test]
fn psbt_native_inventory_source_admission_uses_the_scope_snapshot() {
    fixture_no_egress(|| {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("book.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
            .unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("data/psbt_privacy.json")).unwrap();
        let case = &cases[0];
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        report_with_baseline(
            &conn,
            &dict::<&str>([]),
            case["psbt"].as_str().unwrap(),
            |snapshot, _| {
                let writer = Connection::open(&path).unwrap();
                writer
                    .execute(
                        "UPDATE transactions SET external_id=? WHERE id='in'",
                        ["07".repeat(32)],
                    )
                    .unwrap();
                writer
                    .execute("UPDATE wallet_utxos SET amount=1 WHERE id='coin-0'", [])
                    .unwrap();
                let payload =
                    report_inventory(snapshot, &dict::<&str>([]), case["psbt"].as_str().unwrap())?;
                assert_eq!(
                    canonical(&payload),
                    canonical(
                        &kassiber_pytext::loads(&case["expected"]["ok"].to_string()).unwrap()
                    )
                );
                Ok(fixture_baseline(&case["baseline"]))
            },
        )
        .unwrap();
        assert!(conn.is_autocommit());
        let error =
            report_inventory(&conn, &dict::<&str>([]), case["psbt"].as_str().unwrap()).unwrap_err();
        assert_eq!(error.code, "not_implemented");
        assert!(conn.is_autocommit());
    });
}

#[test]
fn psbt_inventory_matching_and_redaction_match_python_all_112_tables() {
    run("inventory");
}

#[test]
fn psbt_observer_components_and_conditional_deltas_match_python() {
    run("linkage");
}

#[test]
fn psbt_transaction_tells_and_exact_fees_match_python() {
    run("features");
}

#[test]
fn psbt_scope_and_table_projection_match_python() {
    run("scope");
}

#[test]
fn psbt_report_refusals_preserve_reference_bug_and_all_tables() {
    run("errors");
}

#[test]
fn psbt_scope_and_baseline_share_one_database_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("book.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
        .unwrap();
    conn.execute_batch(include_str!("data/book.sql")).unwrap();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/psbt_privacy.json")).unwrap();
    let case = &cases[0];
    conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
    let payload = report_with_baseline(
        &conn,
        &dict::<&str>([]),
        case["psbt"].as_str().unwrap(),
        |snapshot, profile| {
            assert_eq!(profile, "p");
            let writer = Connection::open(&path).unwrap();
            writer
                .execute(
                    "UPDATE settings SET value='other' WHERE key='context_profile'",
                    [],
                )
                .unwrap();
            assert_eq!(
                snapshot
                    .query_row(
                        "SELECT value FROM settings WHERE key='context_profile'",
                        [],
                        |row| row.get::<_, String>(0)
                    )
                    .unwrap(),
                "p"
            );
            Ok(fixture_baseline(&case["baseline"]))
        },
    )
    .unwrap();
    assert_eq!(
        canonical(&payload),
        canonical(&kassiber_pytext::loads(&case["expected"]["ok"].to_string()).unwrap())
    );
    assert!(conn.is_autocommit());
    assert_eq!(
        conn.query_row(
            "SELECT value FROM settings WHERE key='context_profile'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "other"
    );
}

#[test]
fn psbt_native_history_matches_python_all_112_tables() {
    run_cases("history", include_str!("data/psbt_history.json"));
}

#[test]
fn psbt_historical_sources_and_adversary_anchors_share_the_scope_snapshot() {
    fixture_no_egress(|| {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("book.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
            .unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("data/psbt_history.json")).unwrap();
        let case = cases
            .iter()
            .find(|c| c["name"] == "spend/2/None/confirmed/both")
            .unwrap();
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        let args = dict::<&str>([]);
        let proposal = case["psbt"].as_str().unwrap();
        let expected = kassiber_reports::psbt_privacy::report(&conn, &args, proposal).unwrap();
        let payload = report_with_baseline(&conn, &args, proposal, |snapshot, profile| {
            let writer = Connection::open(&path).unwrap();
            writer.execute_batch("BEGIN; UPDATE settings SET value='other' WHERE key='context_profile'; UPDATE transactions SET counterparty=NULL,raw_json='{}' WHERE id='in'; UPDATE source_funds_links SET state='suggested'; COMMIT;").unwrap();
            kassiber_wallets::psbt_privacy::history::capture(snapshot, profile)
        }).unwrap();
        assert_eq!(canonical(&payload), canonical(&expected));
        assert!(conn.is_autocommit());
        assert_ne!(
            canonical(&kassiber_reports::psbt_privacy::report(&conn, &args, proposal).unwrap()),
            canonical(&expected)
        );
    });
}
