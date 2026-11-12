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

/// A keyed in-memory book with one authorized Bitcoin Core subject grant.
fn granted(url: &str, max_bytes: i64) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(super::super::fixture::WATCH_KEY)
        .unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    conn.execute_batch(include_str!(
        "../../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
        .unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO book_network_bindings(profile_id,environment_id,environment,chain_instance_id,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','env','main',NULL,'[{{\"chain\":\"bitcoin\",\"network\":\"main\",\"domain_id\":\"bound-main\",\"chain_instance_id\":null}}]','[]','d','fixed');
         INSERT INTO backends(name,kind,chain,network,url,config_json,created_at,updated_at) VALUES('core','bitcoinrpc','bitcoin','main','{url}','{{\"username\":\"u\",\"password\":\"p\"}}','fixed','fixed');"
    ))
    .unwrap();
    let spec = dict([
        ("backend", s("core")),
        ("network", s("main")),
        ("mode", s("subject")),
        ("subject", s(T1)),
        ("max_requests", Value::Int(10.into())),
        ("max_bytes", Value::Int(max_bytes.into())),
    ]);
    let plan = call(&conn, "sources.plan", spec, false).unwrap();
    call(&conn, "sources.authorize", dict([("plan", plan)]), false).unwrap();
    conn
}

fn rpc(id: &str, result: &str) -> (u16, String) {
    (
        200,
        format!(r#"{{"result":{result},"error":null,"id":"chain-analysis-{id}"}}"#),
    )
}

fn decoded() -> String {
    format!(
        r#"{{"txid":"{T1}","version":2,"locktime":0,"size":100,"vsize":80,"weight":320,"confirmations":2,"blockhash":"{}",
        "vin":[{{"txid":"{T0}","vout":0,"sequence":4294967293,"scriptSig":{{"asm":"","hex":""}},"txinwitness":["30440220"],
        "prevout":{{"value":0.0005,"scriptPubKey":{{"hex":"0014{}","type":"witness_v0_keyhash"}}}}}}],
        "vout":[{{"value":0.0004,"n":0,"scriptPubKey":{{"hex":"0014{}","type":"witness_v0_keyhash"}}}}]}}"#,
        "dd".repeat(32),
        "11".repeat(20),
        "22".repeat(20)
    )
}

fn grant_row(conn: &Connection) -> (String, String, i64, i64, Option<String>) {
    conn.query_row(
        "SELECT status,COALESCE(last_code,''),requests_used,bytes_used,lease_token FROM chain_analysis_acquisition_grants",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .unwrap()
}

#[test]
fn source_runner_reserves_quota_settles_and_releases_its_lease() {
    let fake = FakeProvider::replies(vec![
        rpc("getblockhash", &format!("\"{GENESIS}\"")),
        rpc("getrawtransaction", &decoded()),
    ])
    .unwrap();
    let conn = granted(fake.base(), 1_000_000_000);
    let mut sources = FixedSources::default();
    worker_tick(&conn, &mut sources, &TestRuntime { offline: false }).unwrap();
    assert_eq!(paths(&fake).len(), 2);
    let (status, code, requests, bytes, lease) = grant_row(&conn);
    assert_eq!(
        (status.as_str(), code.as_str(), requests, lease),
        ("active", "observed", 2, None)
    );
    // Each successful response refunds its unused reservation.
    assert!(bytes > 0 && bytes < 1024 * 1024, "{bytes}");
    let payload: String = conn
        .query_row(
            "SELECT payload_json FROM chain_analysis_reference_assertions WHERE active=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!payload.contains("30440220") && payload.contains("\"sequence\":4294967293"));
}

#[test]
fn source_runner_pauses_at_its_lifetime_quota() {
    let fake = FakeProvider::replies(vec![
        rpc("getblockhash", &format!("\"{GENESIS}\"")),
        rpc("getrawtransaction", &decoded()),
    ])
    .unwrap();
    let conn = granted(fake.base(), 8 * 1024 * 1024);
    worker_tick(
        &conn,
        &mut FixedSources::default(),
        &TestRuntime { offline: false },
    )
    .unwrap();
    assert_eq!(paths(&fake).len(), 1);
    let (status, code, requests, _, lease) = grant_row(&conn);
    assert_eq!(
        (status.as_str(), code.as_str(), requests, lease),
        ("paused", "acquisition_quota", 1, None)
    );
}

#[test]
fn source_runner_skips_a_leased_grant_and_pauses_offline() {
    let fake = FakeProvider::new(GENESIS).unwrap();
    let conn = granted(fake.base(), 1_000_000_000);
    conn.execute_batch(&format!(
        "UPDATE chain_analysis_acquisition_grants SET lease_token='other',lease_until={}",
        super::super::fixture::EPOCH + 60
    ))
    .unwrap();
    worker_tick(
        &conn,
        &mut FixedSources::default(),
        &TestRuntime { offline: false },
    )
    .unwrap();
    assert_eq!(grant_row(&conn).4.as_deref(), Some("other"));
    conn.execute_batch(
        "UPDATE chain_analysis_acquisition_grants SET lease_token=NULL,lease_until=NULL",
    )
    .unwrap();
    worker_tick(
        &conn,
        &mut FixedSources::default(),
        &TestRuntime { offline: true },
    )
    .unwrap();
    assert!(fake.requests().is_empty());
    // Offline pauses the schedule itself: no pass, no outcome, no lease.
    let (status, code, requests, _, lease) = grant_row(&conn);
    assert_eq!(
        (status.as_str(), code.as_str(), requests, lease),
        ("active", "", 0, None)
    );
}
