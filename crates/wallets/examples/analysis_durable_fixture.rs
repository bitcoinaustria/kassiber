#[path = "analysis_durable/crash.rs"]
mod crash;
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::analysis_index::durable::fixture::evaluate;
use std::io::Read;

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
    let results = if let Value::List(cases) = &parsed {
        Value::List(
            cases
                .iter()
                .map(|case| {
                    let conn = Connection::open_in_memory().unwrap();
                    conn.execute_batch(include_str!(
                        "../../reports/tests/data/privacy_hygiene_seed.sql"
                    ))
                    .unwrap();
                    conn.execute_batch(include_str!("../../reports/tests/data/book.sql"))
                        .unwrap();
                    let sql = kassiber_envelope::get(case, "sql")
                        .and_then(|v| match v {
                            Value::Str(v) => v.as_str(),
                            _ => None,
                        })
                        .unwrap_or("");
                    conn.execute_batch(sql).unwrap();
                    evaluate(case, &conn).unwrap()
                })
                .collect(),
        )
    } else {
        crash::run(&parsed)
    };
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    println!(
        "{}",
        dumps(&results, &Options::new().sort_keys(true).compact()).unwrap()
    );
}
