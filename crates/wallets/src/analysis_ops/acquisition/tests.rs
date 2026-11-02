use super::super::fixture::FixedSources;
use super::super::*;
use kassiber_net::{test_support::FakeProvider, Gate, Ledger, Offline};

const GENESIS: &str = "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f";
const T0: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const T1: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const T2: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

struct TestRuntime {
    offline: bool,
}
impl Runtime for TestRuntime {
    fn epoch(&self) -> i64 {
        super::super::fixture::EPOCH
    }
    fn offline(&self) -> bool {
        self.offline
    }
    fn no_egress(&self) -> bool {
        false
    }
    fn gate(&self) -> Gate {
        Gate::new(|| Offline::Online, Ledger::default())
    }
}

fn tx(txid: &str, inputs: &[(&str, u32)], outputs: usize) -> String {
    let vin: Vec<String> = inputs
        .iter()
        .map(|(prev, vout)| {
            if prev.is_empty() {
                r#"{"is_coinbase":true,"txid":"0000000000000000000000000000000000000000000000000000000000000000","vout":4294967295,"sequence":4294967295,"scriptsig":"03aabbcc","witness":["aa"]}"#.to_owned()
            } else {
                format!(r#"{{"txid":"{prev}","vout":{vout},"sequence":4294967293,"scriptsig":"","witness":["30440220","02aa"],"prevout":{{"scriptpubkey":"0014{}","value":5000}}}}"#, "11".repeat(20))
            }
        })
        .collect();
    let vout: Vec<String> = (0..outputs)
        .map(|n| {
            format!(
                r#"{{"scriptpubkey":"0014{}","value":{}}}"#,
                "22".repeat(20),
                4000 - n
            )
        })
        .collect();
    format!(
        r#"{{"txid":"{txid}","version":2,"locktime":0,"vin":[{}],"vout":[{}],"status":{{"confirmed":true,"block_height":5,"block_hash":"{}"}}}}"#,
        vin.join(","),
        vout.join(","),
        "dd".repeat(32)
    )
}

fn book(url: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    conn.execute_batch(include_str!(
        "../../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
        .unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO book_network_bindings(profile_id,environment_id,environment,chain_instance_id,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','env','main',NULL,'[{{\"chain\":\"bitcoin\",\"network\":\"main\",\"domain_id\":\"bound-main\",\"chain_instance_id\":null}}]','[]','d','fixed');
         INSERT INTO backends(name,kind,chain,network,url,config_json,created_at,updated_at) VALUES('esplora','esplora','bitcoin','main','{url}','{{}}','fixed','fixed');"
    ))
    .unwrap();
    conn
}

fn call(conn: &Connection, operation: &str, args: Value, offline: bool) -> Result<Value> {
    let result = execute_with_sources(
        conn,
        &Scope {
            workspace_id: "ws",
            profile_id: "p",
        },
        operation,
        &args,
        &mut FixedSources::default(),
        &mut TestRuntime { offline },
    );
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").unwrap();
    }
    result
}

fn plan(conn: &Connection, direction: &str, depth: i64, maximum: i64) -> Value {
    call(
        conn,
        "acquire.plan",
        dict([
            ("backend", s("esplora")),
            ("subject", s(T1)),
            ("chain", s("bitcoin")),
            ("network", s("main")),
            ("direction", s(direction)),
            ("depth", Value::Int(depth.into())),
            ("max_transactions", Value::Int(maximum.into())),
        ]),
        false,
    )
    .unwrap()
}

fn paths(fake: &FakeProvider) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|r| r.lines().next().unwrap_or("").to_owned())
        .collect()
}

fn stored(conn: &Connection, txid: &str) -> Value {
    let raw: String = conn
        .query_row(
            "SELECT payload_json FROM chain_analysis_observations WHERE txid=?",
            [txid],
            |row| row.get(0),
        )
        .unwrap();
    kassiber_pytext::loads(&raw).unwrap()
}

fn frontier(result: &Value) -> Vec<(String, String)> {
    match field(result, "frontier") {
        Value::List(items) => items
            .iter()
            .map(|item| {
                (
                    field(item, "txid").as_str().unwrap().to_owned(),
                    field(item, "reason").as_str().unwrap().to_owned(),
                )
            })
            .collect(),
        _ => panic!("frontier"),
    }
}

#[test]
fn acquisition_apply_sends_nothing_for_a_changed_plan_or_offline() {
    let fake = FakeProvider::new(GENESIS).unwrap();
    let conn = book(fake.base());
    let reviewed = plan(&conn, "backward", 1, 5);
    let mut changed = reviewed.clone();
    set(&mut changed, "snapshot_id", s("stale"));
    let error = call(&conn, "acquire.apply", dict([("plan", changed)]), false).unwrap_err();
    assert_eq!(error.code, "chain_analysis_stale");
    let error = call(&conn, "acquire.apply", dict([("plan", reviewed)]), true).unwrap_err();
    assert_eq!(error.code, "network_egress_disabled");
    assert!(fake.requests().is_empty());
}

#[test]
fn acquisition_apply_backward_depth_discards_witnesses() {
    let fake = FakeProvider::replies(vec![
        (200, GENESIS.to_owned()),
        (200, tx(T1, &[(T0, 0)], 2)),
        (200, tx(T0, &[("", 0)], 1)),
    ])
    .unwrap();
    let conn = book(fake.base());
    let reviewed = plan(&conn, "backward", 1, 5);
    let result = call(&conn, "acquire.apply", dict([("plan", reviewed)]), false).unwrap();
    assert_eq!(
        paths(&fake),
        [
            "GET /block-height/0 HTTP/1.1".to_owned(),
            format!("GET /tx/{T1} HTTP/1.1"),
            format!("GET /tx/{T0} HTTP/1.1"),
        ]
    );
    assert_eq!(field(&result, "acquired_count"), &Value::Int(2.into()));
    assert_eq!(field(&result, "complete"), &Value::Bool(true));
    let payload = stored(&conn, T1);
    let input = &match field(&payload, "vin") {
        Value::List(items) => items.clone(),
        _ => panic!("inputs"),
    }[0];
    assert_eq!(field(input, "sequence"), &Value::Int(4294967293u32.into()));
    assert!(get(input, "witness").is_none() && get(input, "scriptsig").is_none());
    assert!(get(&payload, crate::analysis_features::PERSISTED_FEATURE_KEY).is_some());
    let coinbase = stored(&conn, T0);
    assert!(kassiber_pyjson::dumps(&coinbase, &Default::default())
        .unwrap()
        .contains("\"is_coinbase\": true"));
}

#[test]
fn acquisition_apply_stops_at_depth_and_transaction_limits() {
    let fake = FakeProvider::replies(vec![
        (200, GENESIS.to_owned()),
        (200, tx(T1, &[(T0, 0)], 2)),
    ])
    .unwrap();
    let conn = book(fake.base());
    let reviewed = plan(&conn, "backward", 1, 1);
    let result = call(&conn, "acquire.apply", dict([("plan", reviewed)]), false).unwrap();
    assert_eq!(
        frontier(&result),
        [(T0.to_owned(), "transaction_limit".to_owned())]
    );
    assert_eq!(paths(&fake).len(), 2);
    let fake = FakeProvider::replies(vec![
        (200, GENESIS.to_owned()),
        (200, tx(T1, &[(T0, 0)], 2)),
        (200, tx(T0, &[(T2, 0)], 1)),
    ])
    .unwrap();
    let conn = book(fake.base());
    let reviewed = plan(&conn, "backward", 1, 5);
    let result = call(&conn, "acquire.apply", dict([("plan", reviewed)]), false).unwrap();
    assert_eq!(
        frontier(&result),
        [(T2.to_owned(), "depth_limit".to_owned())]
    );
    assert_eq!(field(&result, "complete"), &Value::Bool(false));
}

#[test]
fn acquisition_apply_flags_uncorroborated_spender_claims() {
    let unspent = r#"[{"spent":false}]"#.to_owned();
    let fake = FakeProvider::replies(vec![
        (200, GENESIS.to_owned()),
        (200, tx(T1, &[(T0, 0)], 2)),
        (
            200,
            format!(r#"[{{"spent":true,"txid":"{T2}"}},{{"spent":false}}]"#),
        ),
        (200, tx(T0, &[("", 0)], 1)),
        (200, format!(r#"[{{"spent":true,"txid":"{T1}"}}]"#)),
        (200, tx(T2, &[(T1, 1)], 1)),
        (200, unspent),
    ])
    .unwrap();
    let conn = book(fake.base());
    let reviewed = plan(&conn, "both", 1, 5);
    let result = call(&conn, "acquire.apply", dict([("plan", reviewed)]), false).unwrap();
    assert_eq!(paths(&fake).len(), 7);
    assert_eq!(
        frontier(&result),
        [(T1.to_owned(), "spender_claim_not_corroborated".to_owned())]
    );
    let status: String = conn
        .query_row(
            "SELECT status_json FROM chain_analysis_observations WHERE txid=?",
            [T1],
            |row| row.get(0),
        )
        .unwrap();
    assert!(status.contains("\"spenders_checked\":false"));
    let status: String = conn
        .query_row(
            "SELECT status_json FROM chain_analysis_observations WHERE txid=?",
            [T0],
            |row| row.get(0),
        )
        .unwrap();
    assert!(status.contains("\"spenders_checked\":true"));
}
