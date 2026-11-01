use kassiber_envelope::get;
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
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
    let Value::List(cases) = kassiber_pytext::loads(&raw).unwrap() else {
        panic!("fixture list required")
    };
    let values = cases
        .iter()
        .map(|case| {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(include_str!(
                "../../reports/tests/data/privacy_hygiene_seed.sql"
            ))
            .unwrap();
            conn.execute_batch(include_str!("../../reports/tests/data/book.sql"))
                .unwrap();
            let sql = match get(case, "sql") {
                Some(Value::Str(v)) => v.as_str().unwrap(),
                _ => "",
            };
            conn.execute_batch(sql).unwrap();
            kassiber_wallets::analysis_index::reads::fixture::evaluate(case, &conn).unwrap()
        })
        .collect();
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    println!(
        "{}",
        dumps(
            &Value::List(values),
            &Options::new().sort_keys(true).compact()
        )
        .unwrap()
    );
}
