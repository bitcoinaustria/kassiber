//! Replays Python's frozen workspace shell reads (scripts/export_vectors.py)
//! on synthetic books and requires byte-equal JSON and unchanged storage.
use kassiber_envelope::{get, s};
use kassiber_pyjson::Value;
use kassiber_store::Book;
use kassiber_workspace::{overview, readiness};

fn encode(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

fn replay(group: &str, projection: &str) {
    let Value::List(cases) = kassiber_pytext::loads(include_str!("data/vectors.json")).unwrap()
    else {
        panic!("cases")
    };
    let mut selected = 0;
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    let conn = book.connection();
    for case in cases {
        if get(&case, "group") != Some(&s(group)) {
            continue;
        }
        selected += 1;
        conn.execute_batch("SAVEPOINT workspace_case").unwrap();
        let Value::Str(sql) = get(&case, "sql").unwrap() else {
            panic!("sql")
        };
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
        let before = conn.total_changes();
        let (context, profile) = readiness::scope(conn, None).unwrap();
        let sources = kassiber_setup::SystemSources;
        let clock = overview::Clock::default();
        let actual = match projection {
            "health" => readiness::health(conn, &context, profile.as_ref()),
            "next_actions" => readiness::next_actions(
                readiness::health(conn, &context, profile.as_ref()).unwrap(),
                || Ok(get(&case, "coverage").unwrap().clone()),
            ),
            "badges" => readiness::badges(conn, profile.as_ref()),
            "overview" => overview::overview(conn, &clock, &sources),
            "workspace_overview" => {
                let args = kassiber_envelope::dict([("workspace_id", s("w"))]);
                overview::workspace_overview(conn, Some(&args), &clock, &sources)
            }
            _ => panic!("projection"),
        };
        let expected = get(&case, projection).unwrap();
        match (actual, get(expected, "error")) {
            (Err(error), Some(_)) => {
                assert_eq!(
                    s(error.message.as_str()),
                    *get(expected, "message").unwrap(),
                    "{:?}: {projection}",
                    get(&case, "name")
                );
            }
            (Ok(actual), _) => assert_eq!(
                encode(&actual),
                encode(expected),
                "{:?}: {projection}",
                get(&case, "name")
            ),
            (Err(error), None) => panic!("{:?}: {projection}: {error:?}", get(&case, "name")),
        }
        assert_eq!(conn.total_changes(), before, "read changed stored state");
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tables, kassiber_store::table_names());
        conn.execute_batch("ROLLBACK TO workspace_case; RELEASE workspace_case")
            .unwrap();
    }
    assert!(selected > 0);
}

#[test]
fn workspace_health_matches_python_scopes_freshness_semantics_and_queue() {
    for group in ["no_scope", "health", "overview"] {
        replay(group, "health");
    }
}
#[test]
fn workspace_next_actions_matches_python_priority_prices_and_reports() {
    for group in ["no_scope", "health", "overview"] {
        replay(group, "next_actions");
    }
}
#[test]
fn workspace_badges_match_python_roots_chains_cycles_and_cached_swaps() {
    for group in ["no_scope", "health", "badges", "overview"] {
        replay(group, "badges");
    }
}
#[test]
fn book_overview_matches_python_balances_series_valuation_and_rows() {
    for group in ["no_scope", "health", "badges", "overview"] {
        replay(group, "overview");
    }
}
#[test]
fn book_set_overview_matches_python_rollups_and_book_boundaries() {
    for group in ["no_scope", "health", "overview"] {
        replay(group, "workspace_overview");
    }
}
