use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::analysis_index::fixture::{evaluate, tables};
use std::io::Read;

fn main() {
    let gate = kassiber_net::Gate::new(
        || kassiber_net::Offline::Unavailable,
        kassiber_net::Ledger::default(),
    );
    let mut raw = String::new();
    std::io::stdin()
        .take(32 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let Value::List(cases) = kassiber_pytext::loads(&raw).unwrap() else {
        panic!("fixture list required")
    };
    let results: Vec<_> = cases
        .iter()
        .map(|case| {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(include_str!(
                "../../reports/tests/data/privacy_hygiene_seed.sql"
            ))
            .unwrap();
            conn.execute_batch(include_str!("../../reports/tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(
                kassiber_envelope::get(case, "sql")
                    .and_then(|v| match v {
                        Value::Str(v) => v.as_str(),
                        _ => None,
                    })
                    .unwrap_or(""),
            )
            .unwrap();
            let before = tables(&conn);
            let result = evaluate(case, &conn).unwrap();
            assert_eq!(tables(&conn), before);
            result
        })
        .collect();
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    println!(
        "{}",
        dumps(
            &Value::List(results),
            &Options::new().sort_keys(true).compact()
        )
        .unwrap()
    );
}
