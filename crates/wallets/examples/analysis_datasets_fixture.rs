use kassiber_envelope::get;
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_wallets::analysis_index::datasets::evaluate_fixture;
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
    let vectors = kassiber_pytext::loads(&raw).unwrap();
    let snapshots = get(&vectors, "snapshots").unwrap();
    let Value::List(cases) = get(&vectors, "cases").unwrap() else {
        panic!("cases required")
    };
    let before = snapshots.clone();
    let result = Value::List(
        cases
            .iter()
            .map(|case| evaluate_fixture(case, snapshots))
            .collect(),
    );
    assert_eq!(snapshots, &before);
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    println!(
        "{}",
        dumps(&result, &Options::new().sort_keys(true).compact()).unwrap()
    );
}
