use kassiber_envelope::get;
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use std::io::Read;

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match get(value, key) {
        Some(Value::Str(v)) => v.as_str(),
        _ => None,
    }
}

fn main() {
    let gate = kassiber_net::Gate::new(
        || kassiber_net::Offline::Unavailable,
        kassiber_net::Ledger::default(),
    );
    let mut raw = String::new();
    std::io::stdin()
        .take(64 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let parsed = kassiber_pytext::loads(&raw).unwrap();
    let result = if let Some(spec) = get(&parsed, "jobs") {
        kassiber_wallets::analysis_ops::fixture::evaluate_jobs(spec).unwrap()
    } else if let Value::List(cases) = parsed {
        Value::List(
            cases
                .iter()
                .map(|case| {
                    let conn = Connection::open_in_memory().unwrap();
                    if get(case, "encrypted") == Some(&Value::Bool(true)) {
                        conn.execute_batch(kassiber_wallets::analysis_ops::fixture::WATCH_KEY)
                            .unwrap();
                    }
                    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
                    conn.execute_batch(include_str!(
                        "../../reports/tests/data/privacy_hygiene_seed.sql"
                    ))
                    .unwrap();
                    conn.execute_batch(include_str!("../../reports/tests/data/book.sql"))
                        .unwrap();
                    conn.execute_batch(text(case, "sql").unwrap_or("")).unwrap();
                    kassiber_wallets::analysis_ops::fixture::evaluate(case, &conn).unwrap()
                })
                .collect(),
        )
    } else {
        let conn = Connection::open(text(&parsed, "database").unwrap()).unwrap();
        if get(&parsed, "encrypted") == Some(&Value::Bool(true)) {
            conn.execute_batch("PRAGMA key='analysis-ops-fixture'")
                .unwrap();
        }
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        assert_eq!(
            conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        let result =
            kassiber_wallets::analysis_ops::fixture::evaluate(get(&parsed, "case").unwrap(), &conn)
                .unwrap();
        assert_eq!(
            conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        result
    };
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    println!(
        "{}",
        dumps(&result, &Options::new().sort_keys(true).compact()).unwrap()
    );
}
