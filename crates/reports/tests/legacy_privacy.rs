#![cfg(feature = "test-harness")]
use kassiber_envelope::{dict, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_reports::legacy_privacy::{fixture_probes, snapshot};
use kassiber_store::rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn canonical(v: &Value) -> String {
    dumps(
        v,
        &Options {
            sort_keys: true,
            separators: Some((",".into(), ":".into())),
            ..Default::default()
        },
    )
    .unwrap()
}
fn tables(conn: &Connection) -> BTreeMap<String, String> {
    let names = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
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
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let columns: Vec<_> = stmt.column_names().iter().map(|s| s.to_string()).collect();
            let mut rows = stmt
                .query_map([], |r| {
                    Ok(dict(columns.iter().enumerate().map(|(i, k)| {
                        use kassiber_store::rusqlite::types::ValueRef;
                        let v = match r.get_ref(i).unwrap() {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => Value::Int(n.into()),
                            ValueRef::Real(n) => Value::Float(n),
                            ValueRef::Text(s) => {
                                kassiber_envelope::s(std::str::from_utf8(s).unwrap())
                            }
                            ValueRef::Blob(_) => panic!("unexpected fixture blob"),
                        };
                        (k.as_str(), v)
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
    kassiber_wallets::psbt_privacy::fixture_no_egress(|| {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
            .unwrap();
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("data/legacy_privacy.json")).unwrap();
        let mut count = 0;
        for case in cases.iter().filter(|c| c["group"] == group) {
            count += 1;
            conn.execute_batch("SAVEPOINT fixture").unwrap();
            conn.execute_batch(include_str!("data/book.sql")).unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = tables(&conn);
            assert_eq!(
                before,
                serde_json::from_value(case["table_sha256"].clone()).unwrap(),
                "{} tables before",
                case["name"]
            );
            let changes = conn.total_changes();
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let actual = match snapshot(&conn, &args) {
                Ok(v) => dict([("ok", v)]),
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
            assert_eq!(
                canonical(&actual),
                canonical(&kassiber_pytext::loads(&case["expected"].to_string()).unwrap()),
                "{} snapshot",
                case["name"]
            );
            let probes = fixture_probes(&conn).unwrap();
            assert_eq!(
                canonical(&kassiber_pytext::loads(&probes.to_string()).unwrap()),
                canonical(&kassiber_pytext::loads(&case["probes"].to_string()).unwrap()),
                "{} scores",
                case["name"]
            );
            assert_eq!(tables(&conn), before, "{} tables after", case["name"]);
            assert_eq!(changes, conn.total_changes());
            conn.execute_batch("ROLLBACK TO fixture; RELEASE fixture")
                .unwrap();
        }
        assert!(count > 0);
    });
}
#[test]
fn legacy_transaction_rules_match_python_bytes_and_all_112_tables() {
    run("transaction");
}
#[test]
fn legacy_inventory_rules_match_python_bytes_and_all_112_tables() {
    run("inventory");
}
#[test]
fn legacy_collaboration_and_inbound_attribution_match_python() {
    run("collaboration");
}
#[test]
fn legacy_scope_and_refusals_match_python() {
    run("scope");
}
#[test]
fn legacy_structural_features_match_python_without_raw_payloads() {
    run("structure");
}
#[test]
fn legacy_ranking_and_limits_match_python_deterministically() {
    run("ordering");
}
#[test]
fn legacy_numeric_coercion_and_exact_msat_match_python() {
    run("coercion");
}
