//! Real-writer Strike and River books (tests/test_exchange_txid_transfer.py):
//! an exact exchange withdrawal books an exchange_txid_exact move, and every
//! less exact variant keeps its hold. Whole finalized stage, compared with
//! Python (crates/custody/scripts/gen_exchange_txid_vectors.py).
use kassiber_custody::codec;
use kassiber_pyjson::{Key, Value as Py};

fn get<'a>(v: &'a Py, name: &str) -> &'a Py {
    match v {
        Py::Dict(entries) => {
            &entries
                .iter()
                .find(|(k, _)| *k == Key::Str(name.into()))
                .expect(name)
                .1
        }
        _ => panic!("expected record"),
    }
}

fn check(name: &str) {
    let Py::List(cases) = codec::loads(include_str!(
        "../../../tests/fixtures/custody_exchange_txid_stage.json"
    ))
    .unwrap() else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| get(case, "case") == &Py::Str(name.into()))
        .expect(name);
    let raw = codec::dumps(get(case, "input"), true).unwrap();
    let snapshot = kassiber_custody::parse(&raw).unwrap();
    let (rows, stages) = kassiber_custody::run_with_rows(&snapshot, true).unwrap();
    let projection = stages
        .finalized_projection
        .unwrap()
        .py(&rows.transactions)
        .unwrap();
    let projection = codec::dumps(&projection, true).unwrap();
    assert_eq!(
        projection,
        codec::dumps(get(case, "expected"), true).unwrap(),
        "{name}"
    );
    // The frozen Python result itself books the exact match only.
    assert_eq!(
        projection.contains("exchange_txid_exact"),
        name.ends_with("_exact"),
        "{name}"
    );
}

macro_rules! cases {
    ($($test:ident => $case:literal),* $(,)?) => { $(#[test] fn $test() { check($case); })* };
}
cases! {
    exchange_txid_strike_exact => "strike_exact",
    exchange_txid_strike_gross => "strike_gross",
    exchange_txid_strike_missing_txid => "strike_missing_txid",
    exchange_txid_strike_not_owned => "strike_not_owned",
    exchange_txid_strike_owned_input => "strike_owned_input",
    exchange_txid_strike_two_outputs => "strike_two_outputs",
    exchange_txid_strike_two_wallets => "strike_two_wallets",
    exchange_txid_strike_two_platforms => "strike_two_platforms",
    exchange_txid_strike_dismissed => "strike_dismissed",
    exchange_txid_strike_other_kind => "strike_other_kind",
    exchange_txid_strike_foreign_owner => "strike_foreign_owner",
    exchange_txid_strike_short_receipt => "strike_short_receipt",
    exchange_txid_strike_reviewed_component => "strike_reviewed_component",
    exchange_txid_river_exact => "river_exact",
    exchange_txid_river_gross => "river_gross",
    exchange_txid_river_missing_txid => "river_missing_txid",
    exchange_txid_river_not_owned => "river_not_owned",
    exchange_txid_river_owned_input => "river_owned_input",
    exchange_txid_river_two_outputs => "river_two_outputs",
    exchange_txid_river_two_wallets => "river_two_wallets",
    exchange_txid_river_two_platforms => "river_two_platforms",
    exchange_txid_river_dismissed => "river_dismissed",
    exchange_txid_river_other_kind => "river_other_kind",
    exchange_txid_river_foreign_owner => "river_foreign_owner",
    exchange_txid_river_short_receipt => "river_short_receipt",
    exchange_txid_river_reviewed_component => "river_reviewed_component",
}
