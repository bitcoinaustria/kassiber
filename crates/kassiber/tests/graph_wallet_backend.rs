//! The desktop's automatic transaction-graph lookup from the wallet's own
//! sync backend (`ui.transactions.graph` with `lookup: "wallet_backend"`),
//! end to end through the daemon against owned loopback Esplora fakes
//! (docs/reference/privacy-and-security.md#external-requests-complete-list).
//! Offline mode, `KASSIBER_NO_EGRESS`, the preference turned off, a wallet
//! without a synced backend and plain reads contact nothing; an allowed
//! lookup contacts only the wallet's backend, never the book's default or
//! another configured backend, and a cached result is reused.

#![cfg(feature = "test-harness")]

mod support;

use std::fs;

use kassiber_net::test_support::{FakeService, Reply};
use kassiber_pyjson::Value;
use support::{list, path, text, Desk};

const TX: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const UNSYNCED: &str = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";
const PARENT: &str = "d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4";
const OWN: &str = "0014c0cebcd6c3d3ca8c75dc5ec62ebe55330ef910e2";
const ADDRESS: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";

/// An Esplora `tx/<txid>` answer with every spent output's amount.
fn esplora_tx(txid: &str) -> String {
    format!(
        r#"{{"txid": "{txid}", "version": 2, "locktime": 0, "size": 222, "weight": 561, "fee": 1000,
            "status": {{"confirmed": true, "block_height": 800000}},
            "vin": [{{"txid": "{PARENT}", "vout": 0, "sequence": 4294967295,
                      "prevout": {{"scriptpubkey": "{OWN}", "scriptpubkey_address": "{ADDRESS}", "value": 60000}}}}],
            "vout": [{{"scriptpubkey": "00143e34985dca6fddc9fb369940e4c7d8e2873f529c", "value": 40000}},
                     {{"scriptpubkey": "{OWN}", "value": 19000}}]}}"#
    )
}

fn esplora() -> FakeService {
    FakeService::start(|request| match request.target.rsplit_once("/tx/") {
        Some((_, txid)) if txid.len() == 64 => Reply::json(200, &esplora_tx(txid)),
        _ => Reply::json(404, "{}"),
    })
    .unwrap()
}

fn failing() -> FakeService {
    FakeService::start(|_| Reply::json(503, "{}")).unwrap()
}

/// A stored transaction whose spent output has no amount: the graph needs a
/// lookup to show input amounts and the fee.
fn row(id: &str, wallet: &str, txid: &str) -> String {
    format!(
        r#"('{id}','w','p','{wallet}','{id}','{txid}','BTC','outbound',41000000,1000000,'2025-05-01T00:00:00Z','2025','2025','withdrawal','{{"txid": "{txid}", "vin": [{{"txid": "{PARENT}", "vout": 0}}], "vout": [{{"n": 0, "scriptpubkey": "00143e34985dca6fddc9fb369940e4c7d8e2873f529c", "value_sats": 40000}}, {{"n": 1, "scriptpubkey": "{OWN}", "value_sats": 19000}}]}}')"#
    )
}

/// A book whose `home` wallet syncs from the `home` backend, while the book's
/// default is `other`; `fresh` names a backend but never synced.
fn desk(home: &FakeService, other: &FakeService) -> Desk {
    let mut desk = Desk::new("graph_wallet_backend");
    desk.env.insert(
        "KASSIBER_WALLET_BACKEND_LOOKUP_FILE".into(),
        desk.base_path()
            .join("preferences/wallet-backend-lookup.json")
            .display()
            .to_string(),
    );
    for (name, service) in [("home", home), ("other", other)] {
        desk.setup_cli(&[
            "backends",
            "create",
            name,
            "--kind",
            "esplora",
            "--url",
            &format!("{}/api", service.origin()),
        ]);
    }
    desk.setup_cli(&["backends", "set-default", "other"]);
    desk.change(&format!(
        r#"INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('home','w','p','Home','address','{{"addresses":["{ADDRESS}"],"chain":"bitcoin","network":"mainnet","backend":"home","last_synced_at":"2025-06-01T00:00:00Z"}}','2025'),
 ('fresh','w','p','Fresh','address','{{"addresses":["{ADDRESS}"],"chain":"bitcoin","network":"mainnet","backend":"home"}}','2025');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,kind,raw_json) VALUES
 {},
 {};"#,
        row("t1", "home", TX),
        row("t2", "fresh", UNSYNCED),
    ));
    desk
}

fn graph(desk: &Desk, args: &str) -> Value {
    let response = desk.ask("ui.transactions.graph", args);
    assert_eq!(
        support::kind_of(&response),
        "ui.transactions.graph",
        "{response:?}"
    );
    path(&response, &["data"]).unwrap().clone()
}

fn codes(graph: &Value) -> Vec<String> {
    list(graph, &["warnings"])
        .iter()
        .filter_map(|w| path(w, &["code"]).and_then(text).map(str::to_owned))
        .collect()
}

/// The spent output's amount, which only a lookup can supply here.
fn input_sats(graph: &Value) -> Option<&Value> {
    list(graph, &["inputs"])
        .first()
        .and_then(|input| path(input, &["valueSats"]))
        .filter(|v| **v != Value::Null)
}

fn preference(desk: &Desk) -> std::path::PathBuf {
    desk.base_path()
        .join("preferences/wallet-backend-lookup.json")
}

const AUTO: &str = r#"{"transaction": "t1", "lookup": "wallet_backend"}"#;

#[test]
fn refusals_contact_nothing() {
    let (home, other) = (esplora(), esplora());
    let mut desk = desk(&home, &other);

    // Plain reads stay local.
    let local = graph(&desk, r#"{"transaction": "t1"}"#);
    assert!(input_sats(&local).is_none());

    // KASSIBER_NO_EGRESS (the desk's default environment) refuses first.
    let refused = graph(&desk, AUTO);
    assert!(
        codes(&refused).contains(&"wallet_backend_lookup_offline".into()),
        "{:?}",
        codes(&refused)
    );

    desk.env.remove("KASSIBER_NO_EGRESS");

    // Offline mode refuses before any request.
    let offline = desk.env["KASSIBER_OFFLINE_PREFERENCE_FILE"].clone();
    fs::write(&offline, "{\"enabled\": true, \"schema_version\": 1}\n").unwrap();
    let refused = graph(&desk, AUTO);
    assert!(codes(&refused).contains(&"wallet_backend_lookup_offline".into()));
    fs::remove_file(&offline).unwrap();

    // So does a damaged offline preference, failing closed.
    fs::write(&offline, "not json").unwrap();
    let refused = graph(&desk, AUTO);
    assert!(codes(&refused).contains(&"wallet_backend_lookup_offline".into()));
    fs::remove_file(&offline).unwrap();

    // The preference turned off, and a damaged preference file, failing closed.
    for content in ["{\"enabled\": false, \"schema_version\": 1}\n", "{"] {
        fs::write(preference(&desk), content).unwrap();
        let refused = graph(&desk, AUTO);
        assert!(codes(&refused).contains(&"wallet_backend_lookup_disabled".into()));
    }
    fs::remove_file(preference(&desk)).unwrap();

    // A wallet that never synced from a backend gets no automatic lookup.
    let unsynced = graph(
        &desk,
        r#"{"transaction": "t2", "lookup": "wallet_backend"}"#,
    );
    assert!(codes(&unsynced).contains(&"wallet_backend_lookup_unavailable".into()));

    // Conflicting flags are refused before anything else.
    let conflict = desk.ask(
        "ui.transactions.graph",
        r#"{"transaction": "t1", "lookup": "wallet_backend", "allowPublicLookup": true}"#,
    );
    assert_eq!(support::kind_of(&conflict), "error");

    assert_eq!(
        home.paths(),
        Vec::<String>::new(),
        "no traffic to the wallet's backend"
    );
    assert_eq!(
        other.paths(),
        Vec::<String>::new(),
        "no traffic to the default backend"
    );
}

#[test]
fn only_the_wallets_backend_is_asked_and_the_result_is_cached() {
    let (home, other) = (esplora(), esplora());
    let mut desk = desk(&home, &other);
    desk.env.remove("KASSIBER_NO_EGRESS");

    // On by default: no preference file has been written.
    assert!(!preference(&desk).exists());
    let filled = graph(&desk, AUTO);
    assert_eq!(
        input_sats(&filled),
        Some(&Value::Int(60000.into())),
        "{filled:?}"
    );
    assert_eq!(
        path(&filled, &["fee", "valueSats"]),
        Some(&Value::Int(1000.into()))
    );
    assert_eq!(home.paths(), vec![format!("GET /api/tx/{TX}")]);
    assert!(other.paths().is_empty(), "never the book's default backend");

    // The cached references answer the next opening without a request.
    let again = graph(&desk, AUTO);
    assert_eq!(input_sats(&again), Some(&Value::Int(60000.into())));
    // So does a plain local read now.
    let local = graph(&desk, r#"{"transaction": "t1"}"#);
    assert_eq!(input_sats(&local), Some(&Value::Int(60000.into())));
    assert_eq!(home.paths().len(), 1);
    assert!(other.paths().is_empty());
}

#[test]
fn a_failing_wallet_backend_does_not_fall_back() {
    let (home, other) = (failing(), esplora());
    let mut desk = desk(&home, &other);
    desk.env.remove("KASSIBER_NO_EGRESS");

    let failed = graph(&desk, AUTO);
    assert!(input_sats(&failed).is_none());
    assert!(
        codes(&failed)
            .iter()
            .any(|code| code.starts_with("bitcoin_reference_lookup_failed")),
        "{:?}",
        codes(&failed)
    );
    assert!(!home.paths().is_empty());
    assert!(other.paths().is_empty(), "no fallback to another backend");
}
