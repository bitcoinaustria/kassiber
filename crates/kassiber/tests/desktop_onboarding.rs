//! Desktop wallet onboarding kinds served by the daemon, against frozen
//! goldens (see `support`). These replaced the Python-compared cases in
//! `tests/test_rust_wallet_{onboarding,identify_onchain,import_desktop,
//! document_import,reveal}.py`.

#![cfg(feature = "test-harness")]

mod support;

use std::sync::{Arc, Condvar, Mutex};

use kassiber_net::test_support::{FakeJsonRpc, FakeService, Reply};
use kassiber_pyjson::Value;
use support::{kind_of, list, path, text, Daemon, Desk, DESKTOP_SQL};

const XPUB: &str = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";
const MASTER: &str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
const TPUB: &str = "tpubD6NzVbkrYhZ4XgiXtGrdW5XDAPFCL9h7we1vwNCpn8tGbBcgfVYjXyhWo4E1xkh56hjod1RhGjxbaTLV3X4FyWuejifB9jusQ46QzG87VKp";
const XPRV: &str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
const BOLT11: &str = "lnbc1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdpl2pkx2ctnv5sxxmmwwd5kgetjypeh2ursdae8g6twvus8g6rfwvs8qun0dfjkxaq9qrsgq357wnc5r2ueh7ck6q93dj32dlqnls087fxdwk8qakdyafkq3yap9us6v52vjjsrvywa6rt52cm9r9zqt8r2t7mlcwspyetp5h2tztugp9lfyql";
/// `tests/test_silent_payments.py::_BASE`.
const SP_BASE: &str = "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv";
const LISTED: &str = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

fn json_str(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

// --- identification (tests/test_rust_wallet_onboarding.py) ---

#[test]
fn onboarding_identify_no_active_profile_and_sqlcipher_match_wire_bytes() {
    let golden = "onboarding_identify_no_active_profile_and_sqlcipher_match_wire_bytes";
    let mut desk = Desk::from_book(golden, "identify-onboarding");
    desk.change("DELETE FROM settings WHERE key='context_profile';");
    let listed = format!(r#"{{"candidates": [{}]}}"#, json_str(LISTED));
    desk.session(
        "ui.wallets.identify",
        &[Some(&listed), Some("\"bad\"")],
        false,
    );
    desk.change("INSERT INTO settings(key,value) VALUES('context_profile','p');");
    desk.encrypt("synthetic identify key");
    let candidates = format!(
        r#"{{"candidates": [{}, {}, {}]}}"#,
        json_str(LISTED),
        json_str(BOLT11),
        json_str(SP_BASE)
    );
    desk.session("ui.wallets.identify", &[Some(&candidates)], false);
    desk.check();
}

#[test]
fn onboarding_ledger_preview_sqlcipher_matches_wire_bytes() {
    let mut desk = Desk::new("onboarding_ledger_preview_sqlcipher_matches_wire_bytes");
    desk.encrypt("synthetic preview key");
    // base64 of "Date,Amount\n2025-01-01,1\n"
    desk.session(
        "ui.wallets.ledger_preview",
        &[Some(
            r#"{"source_bytes_base64": "RGF0ZSxBbW91bnQKMjAyNS0wMS0wMSwxCg=="}"#,
        )],
        false,
    );
    desk.check();
}

#[test]
fn onboarding_detection_fallbacks_and_refusals_match_wire_bytes() {
    let mut desk = Desk::new("onboarding_detection_fallbacks_and_refusals_match_wire_bytes");
    let material =
        |extra: &str, value: &str| format!(r#"{{"wallet_material": {}{extra}}}"#, json_str(value));
    let args: Vec<String> = vec![
        material("", " "),
        r#"{"wallet_material": 3}"#.to_owned(),
        material("", XPRV),
        material("", &format!("wpkh({XPUB}/0/*)")),
        material("", "xpubbogus"),
        material("", XPUB),
        material(r#", "chain": 3"#, XPUB),
        material(r#", "network": []"#, XPUB),
        material(r#", "backend": "nope""#, XPUB),
        material(r#", "backend": " FULCRUM ""#, XPUB),
        material(r#", "backend": "liquid""#, XPUB),
        material(r#", "backend": "mempool""#, XPUB),
        material("", TPUB),
        material(r#", "network": "signet", "backend": "nope""#, TPUB),
        material(r#", "chain": "doge""#, XPUB),
        material(r#", "chain": "liquid""#, XPUB),
        material(r#", "network": "bogus""#, XPUB),
        material(r#", "network": "testnet""#, XPUB),
    ];
    let mut all: Vec<Option<&str>> = vec![None, Some("null"), Some("[]"), Some("{}")];
    all.extend(args.iter().map(|a| Some(a.as_str())));
    desk.session("ui.wallets.detect_script_types", &all, false);
    desk.check();
}

// --- on-chain identification (tests/test_rust_wallet_identify_onchain.py) ---

const LISTED_SCRIPT: &str = "0014e8df018c7e326cc253faac7e46cdc51e68542c42";
const RAW: &str = "020000000199999999999999999999999999999999999999999999999999999999999999990100000000ffffffff020500000000000000160014e8df018c7e326cc253faac7e46cdc51e68542c420000000000000000036a01ff00000000";
const RAW_TXID: &str = "b7af5e7b39ca9b6b84254de6028812f1ee5c3316f531ed448f74fc573c2f87f1";

fn known_tx(txid: &str) -> Option<String> {
    let external = format!("0014{}", "ee".repeat(20));
    let (a, b, d) = ("a".repeat(64), "b".repeat(64), "d".repeat(64));
    Some(if txid == a {
        format!(
            r#"{{"vin": [{{"txid": "{}", "vout": 1, "prevout": {{"scriptpubkey": "{external}"}}}}], "vout": [{{"scriptpubkey": "{LISTED_SCRIPT}", "value": 5}}, {{"scriptpubkey": "6a01ff", "value": 0}}]}}"#,
            "9".repeat(64)
        )
    } else if txid == b {
        format!(
            r#"{{"vin": [{{"txid": "{}", "vout": 0, "prevout": {{"scriptpubkey": "{LISTED_SCRIPT}"}}}}], "vout": [{{"scriptpubkey": "{external}", "value": 4}}]}}"#,
            "c".repeat(64)
        )
    } else if txid == d {
        format!(
            r#"{{"vin": [{{"txid": "{}", "vout": 0}}], "vout": [{{"scriptpubkey": "{LISTED_SCRIPT}"}}]}}"#,
            "e".repeat(64)
        )
    } else {
        return None;
    })
}

/// A loopback HTTP server answering `respond(method, path, body)`, on
/// kassiber-net's service fake (one thread per connection, so a held
/// answer does not block the others). Each request is recorded as
/// `METHOD target body` when it arrives, before it is answered.
struct Http {
    url: String,
    hits: Arc<Mutex<Vec<String>>>,
    _service: FakeService,
}

type Responder = dyn Fn(&str, &str, &str) -> (u16, String) + Send + Sync;

impl Http {
    fn serve(respond: Arc<Responder>) -> Self {
        let hits = Arc::new(Mutex::new(Vec::new()));
        let recorded = hits.clone();
        let service = FakeService::start(move |request| {
            let body = String::from_utf8_lossy(&request.body);
            recorded.lock().unwrap().push(
                format!("{} {} {body}", request.method, request.target)
                    .trim_end()
                    .to_owned(),
            );
            let (status, reply) = respond(&request.method, &request.target, &body);
            Reply::json(status, &reply)
        })
        .unwrap();
        Self {
            url: service.origin(),
            hits,
            _service: service,
        }
    }

    fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}

/// Esplora `/tx/<txid>`; the held txid blocks until released.
struct Esplora {
    http: Http,
    gate: Arc<(Mutex<(bool, bool)>, Condvar)>,
}

impl Esplora {
    fn new(hold: Option<String>) -> Self {
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let waiting = gate.clone();
        let http = Http::serve(Arc::new(move |_, target, _| {
            let txid = target.rsplit('/').next().unwrap_or_default();
            if hold.as_deref() == Some(txid) {
                let (lock, signal) = &*waiting;
                let mut state = lock.lock().unwrap();
                state.0 = true;
                signal.notify_all();
                let _ = signal
                    .wait_timeout_while(state, std::time::Duration::from_secs(30), |s| !s.1)
                    .unwrap();
            }
            match known_tx(txid) {
                Some(body) => (200, body),
                None => (404, "Transaction not found".to_owned()),
            }
        }));
        Self { http, gate }
    }

    fn wait_held(&self) {
        let (lock, signal) = &*self.gate;
        let state = lock.lock().unwrap();
        let (state, timeout) = signal
            .wait_timeout_while(state, std::time::Duration::from_secs(30), |s| !s.0)
            .unwrap();
        assert!(
            state.0 && !timeout.timed_out(),
            "the held lookup never arrived"
        );
    }

    fn release(&self) {
        let (lock, signal) = &*self.gate;
        lock.lock().unwrap().1 = true;
        signal.notify_all();
    }
}

/// Newline JSON-RPC Electrum: version negotiation and one raw transaction,
/// on kassiber-net's JSON-RPC fake.
struct Electrum {
    url: String,
    calls: Arc<Mutex<Vec<String>>>,
    _fake: FakeJsonRpc,
}

impl Electrum {
    fn new() -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let fake = FakeJsonRpc::start(0, move |line| {
            let request: serde_json::Value = serde_json::from_str(line).unwrap();
            let method = request["method"].as_str().unwrap_or_default().to_owned();
            recorded.lock().unwrap().push(method.clone());
            let mut reply = serde_json::json!({"jsonrpc": "2.0", "id": request["id"]});
            if method == "server.version" {
                reply["result"] = serde_json::json!(["Fixture 1.0", "1.4"]);
            } else if request["params"] == serde_json::json!([RAW_TXID]) {
                reply["result"] = serde_json::json!(RAW);
            } else {
                reply["error"] = serde_json::json!({"code": 2, "message": "missing transaction"});
            }
            reply.to_string()
        })
        .unwrap();
        Self {
            url: fake.url().to_owned(),
            calls,
            _fake: fake,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn onchain_desk(golden: &str, esplora: &Esplora, electrum: &Electrum) -> Desk {
    let mut desk = Desk::with_seed(golden, None);
    // Loopback fixtures only: the product switch would refuse them.
    desk.env.remove("KASSIBER_NO_EGRESS");
    desk.replacements
        .push((esplora.http.url.clone(), "$ESPLORA".to_owned()));
    desk.replacements
        .push((electrum.url.clone(), "$ELECTRUM".to_owned()));
    desk.set_seed(&onchain_seed(esplora, electrum));
    desk
}

fn onchain_seed(esplora: &Esplora, electrum: &Electrum) -> String {
    let f = "f".repeat(64);
    format!(
        "{DESKTOP_SQL}\
         INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
         VALUES('list','w','p','Watch list','address','{{\"addresses\": [\"{LISTED}\"]}}','2025');\
         INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,\
         occurred_at,direction,asset,amount,fee,amount_includes_fee,excluded,raw_json,created_at) \
         VALUES('local','w','p','list','{f}','local-fp','2025','inbound','BTC',1,0,0,0,\
         '{{\"txid\": \"{f}\", \"vin\": [], \"vout\": [{{\"scriptpubkey\": \"{LISTED_SCRIPT}\"}}]}}','2025');\
         INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at) VALUES\
         ('esp','esplora','bitcoin','main','{esp}','2025','2025'),\
         ('espnochain','mempool',NULL,NULL,'{esp}','2025','2025'),\
         ('elx','electrum','bitcoin','main','{elx}','2025','2025'),\
         ('elxnochain','electrum',NULL,NULL,'{elx}','2025','2025'),\
         ('core','bitcoinrpc','bitcoin','main','http://127.0.0.1:1','2025','2025');",
        esp = esplora.http.url,
        elx = electrum.url,
    )
}

const ONCHAIN: &str = "ui.wallets.identify_onchain";

fn record_traffic(desk: &mut Desk, esplora: &Esplora, electrum: &Electrum) {
    for hit in esplora.http.hits() {
        desk.golden.line(&format!("esplora {hit}"));
    }
    for call in electrum.calls() {
        desk.golden.line(&format!("electrum {call}"));
    }
}

fn terminals(records: &[Value], kind: &str) -> Vec<Value> {
    records
        .iter()
        .filter(|r| kind_of(r) == kind || kind_of(r) == "error")
        .cloned()
        .collect()
}

#[test]
fn onboarding_onchain_local_answers_refusals_and_lookups_match_python() {
    let (esplora, electrum) = (Esplora::new(None), Electrum::new());
    let mut desk = onchain_desk(
        "onboarding_onchain_local_answers_refusals_and_lookups_match_python",
        &esplora,
        &electrum,
    );
    let txids = format!(
        "[\"{}\", \"{}\", \"{}\", \"{}\", \"{}\"]",
        "a".repeat(64),
        "b".repeat(64),
        "d".repeat(64),
        "0".repeat(64),
        "f".repeat(64)
    );
    let f = "f".repeat(64);
    let args = [
        "null".to_owned(),
        "[]".to_owned(),
        r#"{"txids": 3}"#.to_owned(),
        format!(r#"{{"backend": "nope", "txids": {txids}}}"#),
        format!(r#"{{"backend": "core", "txids": {txids}}}"#),
        r#"{"backend": 7}"#.to_owned(),
        format!(r#"{{"txids": ["{f}"], "candidates": ["{LISTED}", "bc1qnope"]}}"#),
        format!(r#"{{"txids": ["{f}"], "backend": "esp", "scan_to_index": 0}}"#),
        format!(r#"{{"txids": {txids}, "candidates": ["{LISTED}"], "backend": "esp"}}"#),
        format!(r#"{{"txids": {txids}, "backend": " ESPNOCHAIN "}}"#),
        format!(
            r#"{{"txids": ["{RAW_TXID}", "{}"], "backend": "elx"}}"#,
            "0".repeat(64)
        ),
        format!(
            r#"{{"txids": ["{}"], "backend": "elxnochain"}}"#,
            "a".repeat(64)
        ),
    ];
    let records = desk.scripted(false, &|line| line.to_owned(), |daemon| {
        for (index, args) in args.iter().enumerate() {
            daemon.step(&index.to_string(), ONCHAIN, Some(args));
        }
        false
    });
    record_traffic(&mut desk, &esplora, &electrum);
    let terminal = terminals(&records, ONCHAIN);
    assert_eq!(
        path(&terminal[8], &["data", "summary", "owned"]),
        Some(&Value::from(5))
    );
    assert!(path(&terminal[8], &["data", "cancelled"]).is_some());
    assert!(path(&terminal[7], &["data", "cancelled"]).is_none());
    assert_eq!(
        path(
            &list(&terminal[10], &["data", "results"])[0],
            &["classification"]
        )
        .and_then(text),
        Some("undetermined")
    );
    assert_eq!(kind_of(&terminal[11]), "error");
    desk.check();
}

#[test]
fn onboarding_onchain_cancel_mid_request_and_before_registration_match_python() {
    let held = format!(
        "[\"{}\", \"{}\", \"{}\"]",
        "a".repeat(64),
        "b".repeat(64),
        "d".repeat(64)
    );
    let (esplora, electrum) = (Esplora::new(Some("a".repeat(64))), Electrum::new());
    let mut desk = onchain_desk(
        "onboarding_onchain_cancel_mid_request_and_before_registration_match_python",
        &esplora,
        &electrum,
    );
    let lookup = format!(r#"{{"txids": {held}, "backend": "esp"}}"#);
    let records = desk.scripted(false, &|line| line.to_owned(), |daemon: &mut Daemon| {
        daemon.request(ONCHAIN, "v", Some(&lookup));
        // Progress 0 of 3.
        loop {
            let row = kassiber_pytext::loads(&daemon.read()).unwrap();
            if path(&row, &["request_id"]).and_then(text) == Some("v") {
                break;
            }
        }
        esplora.wait_held();
        daemon.step(
            "c",
            "ui.wallets.identify_onchain.cancel",
            Some(r#"{"target_request_id": "v"}"#),
        );
        esplora.release();
        daemon.terminal("v");
        for (index, args) in [
            None,
            Some("{}"),
            Some(r#"{"target_request_id": ""}"#),
            Some(r#"{"target_request_id": 7}"#),
            Some(r#"{"target_request_id": "v"}"#),
            Some(r#"{"target_request_id": "later"}"#),
        ]
        .into_iter()
        .enumerate()
        {
            daemon.step(&format!("x{index}"), "ui.wallets.identify_onchain.cancel", args);
        }
        daemon.step("later", ONCHAIN, Some(&lookup));
        daemon.send(&format!(
            r#"{{"request_id": 7, "kind": "{ONCHAIN}", "args": {{"txids": ["{}"], "backend": "esp"}}}}"#,
            "b".repeat(64)
        ));
        loop {
            let row = kassiber_pytext::loads(&daemon.read()).unwrap();
            if path(&row, &["request_id"]) == Some(&Value::from(7))
                && !kind_of(&row).ends_with(".progress")
            {
                break;
            }
        }
        false
    });
    record_traffic(&mut desk, &esplora, &electrum);
    let reports: Vec<&Value> = records.iter().filter(|r| kind_of(r) == ONCHAIN).collect();
    assert_eq!(
        path(reports[0], &["data", "cancelled"]),
        Some(&Value::Bool(true))
    );
    let last_warning = |report: &Value| {
        list(report, &["data", "warnings"])
            .last()
            .and_then(text)
            .map(str::to_owned)
    };
    assert_eq!(
        last_warning(reports[0]).as_deref(),
        Some("Stopped after 1 of 3 on-chain lookups; the rest stay unknown.")
    );
    assert_eq!(
        last_warning(reports[1]).as_deref(),
        Some("Stopped after 0 of 3 on-chain lookups; the rest stay unknown.")
    );
    desk.check();
}

#[test]
fn onboarding_onchain_sqlcipher_lookup_matches_python() {
    let (esplora, electrum) = (Esplora::new(None), Electrum::new());
    let mut desk = onchain_desk(
        "onboarding_onchain_sqlcipher_lookup_matches_python",
        &esplora,
        &electrum,
    );
    desk.encrypt("synthetic onchain key");
    let args = format!(r#"{{"txids": ["{}"], "backend": "esp"}}"#, "a".repeat(64));
    let records = desk.scripted(false, &|line| line.to_owned(), |daemon| {
        daemon.step("v", ONCHAIN, Some(&args));
        false
    });
    record_traffic(&mut desk, &esplora, &electrum);
    assert_eq!(kind_of(&records[0]), "daemon.unlock");
    assert_eq!(
        path(&records[records.len() - 2], &["data", "summary", "owned"]),
        Some(&Value::from(1))
    );
    desk.check();
}

// --- file imports and analysis (tests/test_rust_wallet_import_desktop.py) ---

const JSON_ROWS: &str = r#"[{"txid": "funding-001", "occurred_at": "2025-01-03T10:00:00Z", "direction": "inbound", "asset": "BTC", "amount": 0.5, "fee": 0, "fiat_rate": 44000, "description": "Initial funding", "kind": "deposit"}, {"txid": "payment-001", "occurred_at": "2025-02-10T14:30:00Z", "direction": "outbound", "asset": "BTC", "amount": 0.1, "fee": 0.0002, "fiat_rate": 51000, "description": "Vendor payment", "kind": "withdrawal"}]"#;
const CSV: &str = "txid,occurred_at,direction,asset,amount,fee,fiat_rate,description\n\
                   csv-1,2025-03-01T00:00:00Z,inbound,BTC,0.25,0,40000,First\n\
                   csv-2,2025-03-02T00:00:00Z,outbound,BTC,0.05,0.0001,41000,Second\n";
const LEDGER: &str = "Wann,Menge,Typ\n2025-04-01,0.5,Kauf\n2025-04-02,0.1,Verkauf\n";
const WASABI: &str = r#"{"gethistory": [{"datetime": "2025-05-01T00:00:00+00:00", "height": 800000, "amount": 150000, "label": "Salary", "tx": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee", "islikelycoinjoin": false}], "listkeys": [{"keyState": 2}, {"keyState": 0}]}"#;

/// `RustDesktopImportTest.seed`: the import files and an `imports` wallet.
fn import_desk(golden: &str) -> (Desk, std::path::PathBuf) {
    let mut desk = Desk::with_seed(golden, None);
    let files = desk.base_path().join("files");
    std::fs::create_dir_all(&files).unwrap();
    for (name, text) in [
        ("rows.json", JSON_ROWS),
        ("rows.csv", CSV),
        ("ledger.csv", LEDGER),
        ("notes.pdf", "%PDF-1.4\n"),
    ] {
        std::fs::write(files.join(name), text).unwrap();
    }
    desk.set_seed(&format!(
        "{DESKTOP_SQL}INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
         VALUES('imports','w','p','Imports','custom','{{}}','2025');"
    ));
    (desk, files)
}

fn imported(desk: &Desk) -> i64 {
    desk.connection()
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE wallet_id='imports'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// One import session, masked as the Python test masked it: the import
/// ends the daemon session.
fn import(desk: &mut Desk, args: &str) {
    let before = imported(desk);
    desk.line_mask = Some(support::mask_import_line);
    desk.table_mask = Some(support::mask_import_row);
    desk.ends_session = true;
    desk.session("ui.wallets.import_file", &[Some(args)], true);
    assert!(imported(desk) > before, "the import added rows");
}

fn file_arg(files: &std::path::Path, name: &str) -> String {
    json_str(&files.join(name).to_string_lossy())
}

#[test]
fn onboarding_import_refusals_keep_the_session_and_match_python() {
    let (mut desk, files) =
        import_desk("onboarding_import_refusals_keep_the_session_and_match_python");
    let csv = file_arg(&files, "rows.csv");
    let ledger = file_arg(&files, "ledger.csv");
    let args = [
        r#"{"source_format": 3}"#.to_owned(),
        r#"{"source_format": " "}"#.to_owned(),
        r#"{"source_format": "csv"}"#.to_owned(),
        r#"{"source_format": "csv", "source_file": 4}"#.to_owned(),
        r#"{"source_format": "csv", "source_file": "/no/such/export.csv"}"#.to_owned(),
        format!(r#"{{"source_format": "pdf", "source_file": {csv}, "wallet": "imports"}}"#),
        r#"{"source_format": "csv", "source_bundle": {}}"#.to_owned(),
        r#"{"source_format": "wasabi_bundle", "source_bundle": {}}"#.to_owned(),
        r#"{"source_format": "wasabi_bundle", "source_bundle": {}, "wallet": "absent"}"#.to_owned(),
        format!(r#"{{"source_format": "csv", "source_file": {csv}}}"#),
        format!(r#"{{"source_format": "csv", "source_file": {csv}, "wallet": "absent"}}"#),
        format!(
            r#"{{"source_format": "generic_ledger", "source_file": {ledger}, "wallet": "imports", "column_map": []}}"#
        ),
        format!(
            r#"{{"source_format": "generic_ledger", "source_file": {ledger}, "wallet": "imports", "column_map": {{"date": "Nope", "amount": "Menge"}}}}"#
        ),
        format!(r#"{{"source_format": "bullbitcoin_csv", "source_file": {csv}, "mode": "bogus"}}"#),
        format!(r#"{{"source_format": "strike_csv", "source_file": {csv}, "wallet": "imports"}}"#),
    ];
    let mut all: Vec<Option<&str>> = vec![None, Some("null"), Some("[]"), Some("{}")];
    all.extend(args.iter().map(|a| Some(a.as_str())));
    desk.session("ui.wallets.import_file", &all, false);
    desk.check();
}

#[test]
fn onboarding_import_json_runs_the_journal_step() {
    let (mut desk, files) = import_desk("onboarding_import_json_runs_the_journal_step");
    import(
        &mut desk,
        &format!(
            r#"{{"source_format": "json", "source_file": {}, "wallet": " Imports "}}"#,
            file_arg(&files, "rows.json")
        ),
    );
    desk.check();
}

#[test]
fn onboarding_import_csv_with_journals_disabled() {
    let (mut desk, files) = import_desk("onboarding_import_csv_with_journals_disabled");
    desk.change(
        "INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p',\
         '{\"source_classes\": {\"journals\": false}}');",
    );
    import(
        &mut desk,
        &format!(
            r#"{{"source_format": "csv", "source_file": {}, "wallet": "imports"}}"#,
            file_arg(&files, "rows.csv")
        ),
    );
    desk.check();
}

#[test]
fn onboarding_import_inline_wasabi_bundle() {
    let (mut desk, _) = import_desk("onboarding_import_inline_wasabi_bundle");
    import(
        &mut desk,
        &format!(
            r#"{{"source_format": "wasabi_bundle", "source_bundle": {WASABI}, "wallet": "imports"}}"#
        ),
    );
    desk.check();
}

#[test]
fn onboarding_samourai_refusals_match_python() {
    let mut desk = Desk::with_seed("onboarding_samourai_refusals_match_python", None);
    desk.set_seed(&format!(
        "{DESKTOP_SQL}INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) \
         VALUES('treasury','w','p','treasury','Treasury','asset','2025');"
    ));
    let args = [
        r#"{"label": 3}"#.to_owned(),
        r#"{"label": "S", "mnemonic": "abandon"}"#.to_owned(),
        r#"{"label": "S", "backup_file": "", "source_file": "/x"}"#.to_owned(),
        r#"{"label": "S", "source_set": []}"#.to_owned(),
        r#"{"label": "S", "gap_limit": "20"}"#.to_owned(),
        r#"{"label": "S", "gap_limit": 2.5}"#.to_owned(),
        r#"{"label": "S", "account": 3}"#.to_owned(),
        r#"{"label": "S"}"#.to_owned(),
        format!(r#"{{"label": "S", "source_set": {{"deposit": {{"bip84": "{XPRV}"}}}}}}"#),
        r#"{"label": "S", "source_set_file": "/no/such/set.json"}"#.to_owned(),
        format!(r#"{{"label": "S", "source_set": {{"deposit": {{"bip84": "{XPUB}"}}}}}}"#),
        format!(r#"{{"label": "S", "source_set": {{"xpubs": [{{"xpub": "{XPUB}"}}]}}}}"#),
    ];
    let mut all: Vec<Option<&str>> = vec![None, Some("null"), Some("{}")];
    all.extend(args.iter().map(|a| Some(a.as_str())));
    desk.session("ui.wallets.import_samourai", &all, false);
    desk.check();
}

/// `base64.b64encode`.
fn b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn upload(text: &str, filename: &str, extra: &str) -> String {
    format!(
        r#"{{"source_bytes_base64": "{}", "filename": {}{extra}}}"#,
        b64(text.as_bytes()),
        json_str(filename)
    )
}

#[test]
fn onboarding_analyze_uploads_tokens_and_refusals_match_python() {
    let mut desk = Desk::new("onboarding_analyze_uploads_tokens_and_refusals_match_python");
    let args = [
        r#"{"token": 3}"#.to_owned(),
        r#"{"token": "missing-grant"}"#.to_owned(),
        r#"{"sample_rows": "x"}"#.to_owned(),
        r#"{"sample_rows": true}"#.to_owned(),
        r#"{"column_map": "date=Wann"}"#.to_owned(),
        r#"{"source_bytes_base64": ""}"#.to_owned(),
        upload("x", "export.doc", ""),
        upload("x", "README", ""),
        r#"{"source_bytes_base64": "AA=A", "filename": "a.csv"}"#.to_owned(),
        upload("Timestamp,Side,BTC Amount,Fee,Currency,Total\n2024-03-01T10:00:00Z,Buy,0.5,0.0001,EUR,21000\n2024-04-02T10:00:00Z,Sell,0.25,0.0001,EUR,12000\n", "export.csv", ""),
        upload("Timestamp;Side;BTC Amount\n2024-03-01T10:00:00Z;Buy;0,5\n", "semi.TSV", ""),
        upload("Ausführung,Richtung,Stück\n2024-03-01T10:00:00Z,Buy,0.5\n", "export.csv", ""),
        upload(
            "Ausführung,Richtung,Stück\n2024-03-01T10:00:00Z,Buy,0.5\n",
            "export.csv",
            r#", "column_map": {"date": "Ausführung", "type": "Richtung", "amount": "Stück"}"#,
        ),
        upload(
            "Trade Date,Action,Qty,Currency,Total\n2024-05-01T09:00:00Z,ACQ-MKT,1.5,EUR,60000\n",
            "export.csv",
            r#", "column_map": {"date": "Trade Date", "type": "Action", "amount": "Qty", "fiat_currency": "Currency", "fiat_value": "Total", "type_map": {"ACQ-MKT": "Buy", "acq mkt": "Sell"}}"#,
        ),
        upload("Date,Amount\n2024-01-01,250000\n2024-01-02,-1000\n", "export.csv", r#", "sample_rows": 0"#),
        upload("Date,Amount\n2024-01-01,0.5\n", "export.csv", r#", "sample_rows": 99"#),
        upload("Account holder,Max Mustermann\nDate,Amount,Fee\n2024-01-01,1,0\n2024-01-02,2,0\n", "export.csv", ""),
        upload("just one column\nno delimiter\n", "notes.txt", ""),
        upload("%PDF-1.4\n", "statement.pdf", ""),
        upload("x", "photo.JPG", ""),
        upload("", "empty.csv", ""),
    ];
    let mut all: Vec<Option<&str>> = vec![None, Some("null"), Some("[]"), Some("{}")];
    all.extend(args.iter().map(|a| Some(a.as_str())));
    desk.session("ui.wallets.analyze_file", &all, false);
    desk.check();
}

#[test]
fn onboarding_analyze_sqlcipher_analysis_matches_python() {
    let mut desk = Desk::new("onboarding_analyze_sqlcipher_analysis_matches_python");
    desk.encrypt("synthetic analysis key");
    desk.session(
        "ui.wallets.analyze_file",
        &[Some(&upload(
            "Date,Amount\n2024-01-01,0.5\n",
            "export.csv",
            "",
        ))],
        false,
    );
    desk.check();
}

// --- document imports (tests/test_rust_wallet_document_import.py) ---

const STAGE: &str = "internal.document_import.stage";
const PREVIEW: &str = "ui.wallets.document_import.preview";
const IMPORT: &str = "ui.wallets.document_import.import";
/// `crates/wallets/scripts/generate_document_vectors.py::SOURCE`.
const SOURCE: &[u8] = b"Kassiber synthetic local OCR evidence\n";

/// The local OCR provider: two rows, the second unreadable
/// (`generate_document_vectors.base()`; the confidence keys sorted).
fn ocr() -> Http {
    let row = |amount: &str, confidence: &str| {
        format!(
            r#"{{"occurred_at": "2026-01-02", "direction": "inbound", "asset": "BTC", "amount_btc": "{amount}", "fee_btc": "0", "fiat_currency": "EUR", "fiat_value": "500.00", "counterparty": "OTC Desk", "description": "Receipt row", "confidence": {confidence}, "cell_confidences": {{"amount_btc": 0.99, "asset": 0.99, "counterparty": 0.99, "description": 0.99, "direction": 0.99, "fee_btc": 0.99, "fiat_currency": 0.99, "fiat_rate": 0.99, "fiat_value": 0.99, "occurred_at": 0.99}}, "source_region": {{"page": 1, "x": 0.1, "y": 0.2, "width": 0.7, "height": 0.1, "unit": "relative"}}, "evidence_text": "02.01.2026 BTC 0.01000000 EUR 500 OTC Desk"}}"#
        )
    };
    let rows = format!(
        r#"{{"rows": [{}, {}]}}"#,
        row("0.01000000", "0.94"),
        row("x", "0.2")
    );
    let reply = serde_json::json!({"output": [{"type": "message", "content": [{"type": "output_text", "text": rows}]}]})
        .to_string();
    Http::serve(Arc::new(move |method, _, _| {
        if method == "GET" {
            (200, r#"{"data": [{"id": "glm-ocr"}]}"#.to_owned())
        } else {
            (200, reply.clone())
        }
    }))
}

fn document_desk(golden: &str, ocr: &Http) -> (Desk, String) {
    let mut desk = Desk::with_seed(golden, None);
    let source = desk.base_path().join("receipt.png");
    std::fs::write(&source, SOURCE).unwrap();
    desk.replacements.push((ocr.url.clone(), "$OCR".to_owned()));
    desk.set_seed(&format!(
        "{DESKTOP_SQL}INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
         VALUES('imports','w','p','Imports','custom','{{}}','2025');\
         INSERT INTO settings(key,value) VALUES('ai_providers_seeded','1') \
         ON CONFLICT(key) DO UPDATE SET value='1';\
         INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) \
         VALUES('local','{}/v1','local','2026-01-01','2026-01-01');",
        ocr.url
    ));
    desk.line_mask = Some(support::mask_import_line);
    desk.table_mask = Some(support::mask_import_row);
    (desk, json_str(&source.to_string_lossy()))
}

/// The opaque tokens each run issues, read from the replies.
#[derive(Default, Clone)]
struct Tokens(Arc<Mutex<Vec<String>>>);

impl Tokens {
    fn learn(&self, reply: &Value) -> Value {
        for key in ["document_token", "upload_id"] {
            if let Some(token) = path(reply, &["data", key]).and_then(text) {
                self.0.lock().unwrap().push(token.to_owned());
            }
        }
        reply.clone()
    }

    fn mask(&self, line: &str) -> String {
        let mut tokens = self.0.lock().unwrap().clone();
        tokens.sort_by_key(|t| std::cmp::Reverse(t.len()));
        let mut line = line.to_owned();
        for token in tokens {
            line = line.replace(&token, "<token>");
        }
        line
    }
}

fn record_ocr(desk: &mut Desk, ocr: &Http) {
    for hit in ocr.hits() {
        let hit = desk.normalize(&hit);
        desk.golden.line(&format!("ocr {hit}"));
    }
}

#[test]
fn onboarding_document_grants_previews_and_refusals_match_python() {
    let server = ocr();
    let (mut desk, source) = document_desk(
        "onboarding_document_grants_previews_and_refusals_match_python",
        &server,
    );
    let tokens = Tokens::default();
    let learned = tokens.clone();
    let records = desk.scripted(true, &move |line| learned.mask(line), |daemon| {
        let mut step = |id: &str, kind: &str, args: Option<&str>| tokens.learn(&daemon.step(id, kind, args));
        for (index, args) in [
            None,
            Some("[]".to_owned()),
            Some("{}".to_owned()),
            Some(r#"{"source_file": 3}"#.to_owned()),
            Some(r#"{"source_file": "/no/such/receipt.png"}"#.to_owned()),
            Some(format!(r#"{{"source_file": {source}, "review_recipe": {{}}}}"#)),
            Some(format!(
                r#"{{"source_file": {source}, "review_case_id": "x", "review_recipe": {{}}}}"#
            )),
            Some(format!(
                r#"{{"source_file": {source}, "review_case_id": "source_funds:x"}}"#
            )),
        ]
        .iter()
        .enumerate()
        {
            step(&format!("s{index}"), STAGE, args.as_deref());
        }
        let stage = step("stage", STAGE, Some(&format!(r#"{{"source_file": {source}}}"#)));
        let token = path(&stage, &["data", "document_token"]).and_then(text).unwrap().to_owned();
        step("analyze", "ui.wallets.analyze_file", Some(&format!(r#"{{"token": "{token}"}}"#)));
        assert!(server.hits().is_empty(), "staging and analysis reach no OCR provider");
        for (index, args) in [
            "{}".to_owned(),
            r#"{"document_token": " "}"#.to_owned(),
            r#"{"document_token": "unknown"}"#.to_owned(),
            format!(r#"{{"document_token": "{token}", "rows": []}}"#),
            format!(r#"{{"document_token": "{token}", "source_file": {source}}}"#),
            format!(r#"{{"document_token": "{token}", "confidence_threshold": "x"}}"#),
            format!(r#"{{"document_token": "{token}", "max_pages": 0}}"#),
        ]
        .iter()
        .enumerate()
        {
            step(&format!("p{index}"), PREVIEW, Some(args));
        }
        let preview = step(
            "preview",
            PREVIEW,
            Some(&format!(r#"{{"document_token": "{token}", "provider": "local"}}"#)),
        );
        let data = path(&preview, &["data"]).unwrap_or_else(|| panic!("{preview:?}"));
        let previewed = path(data, &["document_token"]).and_then(text).unwrap().to_owned();
        let rows = list(data, &["rows"]);
        let ids = |ready_only: bool| -> String {
            let ids: Vec<String> = rows
                .iter()
                .filter(|row| !ready_only || path(row, &["status"]).and_then(text) == Some("ready"))
                .map(|row| format!("\"{}\"", path(row, &["id"]).and_then(text).unwrap()))
                .collect();
            format!("[{}]", ids.join(", "))
        };
        let (ready, all) = (ids(true), ids(false));
        for (index, args) in [
            format!(r#"{{"document_token": "{previewed}"}}"#),
            format!(r#"{{"document_token": "{token}", "wallet": "imports", "selected_row_ids": {ready}}}"#),
            format!(r#"{{"document_token": "{previewed}", "wallet": "imports"}}"#),
            format!(r#"{{"document_token": "{previewed}", "wallet": "imports", "selected_row_ids": "x"}}"#),
            format!(r#"{{"document_token": "{previewed}", "wallet": "imports", "selected_row_ids": [""]}}"#),
            format!(r#"{{"document_token": "{previewed}", "wallet": "imports", "selected_row_ids": ["nope"]}}"#),
            format!(r#"{{"document_token": "{previewed}", "wallet": "imports", "selected_row_ids": {all}}}"#),
            format!(r#"{{"document_token": "{previewed}", "wallet": "absent", "selected_row_ids": {ready}}}"#),
            format!(
                r#"{{"document_token": "{previewed}", "wallet": "imports", "selected_row_ids": {ready}, "include_quarantined": true}}"#
            ),
        ]
        .iter()
        .enumerate()
        {
            step(&format!("i{index}"), IMPORT, Some(args));
        }
        step("lock", "daemon.lock", None);
        step(
            "after-lock",
            PREVIEW,
            Some(&format!(r#"{{"document_token": "{token}", "provider": "local"}}"#)),
        );
        false
    });
    record_ocr(&mut desk, &server);
    assert!(records.iter().any(|r| kind_of(r) == PREVIEW));
    assert!(records.iter().any(|r| kind_of(r) == STAGE));
    desk.check();
}

#[test]
fn onboarding_document_reviewed_rows_import_and_end_the_session() {
    document_import_reviewed_rows(
        "onboarding_document_reviewed_rows_import_and_end_the_session",
        None,
    );
}

#[test]
fn onboarding_document_sqlcipher_import_matches_python() {
    document_import_reviewed_rows(
        "onboarding_document_sqlcipher_import_matches_python",
        Some("synthetic document key"),
    );
}

fn document_import_reviewed_rows(golden: &str, passphrase: Option<&str>) {
    let server = ocr();
    let (mut desk, source) = document_desk(golden, &server);
    if let Some(passphrase) = passphrase {
        desk.encrypt(passphrase);
    }
    let tokens = Tokens::default();
    let learned = tokens.clone();
    desk.scripted(true, &move |line| learned.mask(line), |daemon| {
        let stage = tokens.learn(&daemon.step("stage", STAGE, Some(&format!(r#"{{"source_file": {source}}}"#))));
        let token = path(&stage, &["data", "document_token"]).and_then(text).unwrap().to_owned();
        let preview = tokens.learn(&daemon.step(
            "preview",
            PREVIEW,
            Some(&format!(r#"{{"document_token": "{token}", "provider": "local"}}"#)),
        ));
        let data = path(&preview, &["data"]).unwrap_or_else(|| panic!("{preview:?}"));
        let previewed = path(data, &["document_token"]).and_then(text).unwrap().to_owned();
        let ready: Vec<String> = list(data, &["rows"])
            .iter()
            .filter(|row| path(row, &["status"]).and_then(text) == Some("ready"))
            .map(|row| format!("\"{}\"", path(row, &["id"]).and_then(text).unwrap()))
            .collect();
        let reply = daemon.step(
            "import",
            IMPORT,
            Some(&format!(
                r#"{{"document_token": "{previewed}", "wallet": "imports", "selected_row_ids": [{}]}}"#,
                ready.join(", ")
            )),
        );
        assert_eq!(kind_of(&reply), IMPORT, "{reply:?}");
        true
    });
    record_ocr(&mut desk, &server);
    desk.check();
}

#[test]
fn onboarding_document_profile_switch_withdraws_grants() {
    let server = ocr();
    let (mut desk, source) = document_desk(
        "onboarding_document_profile_switch_withdraws_grants",
        &server,
    );
    let tokens = Tokens::default();
    let learned = tokens.clone();
    let records = desk.scripted(true, &move |line| learned.mask(line), |daemon| {
        let stage = tokens.learn(&daemon.step(
            "stage",
            STAGE,
            Some(&format!(r#"{{"source_file": {source}}}"#)),
        ));
        let token = path(&stage, &["data", "document_token"])
            .and_then(text)
            .unwrap()
            .to_owned();
        daemon.step(
            "switch",
            "ui.profiles.switch",
            Some(r#"{"workspace_id": "w", "profile_id": "p"}"#),
        );
        daemon.step(
            "after-switch",
            PREVIEW,
            Some(&format!(
                r#"{{"document_token": "{token}", "provider": "local"}}"#
            )),
        );
        false
    });
    record_ocr(&mut desk, &server);
    assert_eq!(
        path(&records[records.len() - 2], &["error", "code"]).and_then(text),
        Some("document_import_session_expired")
    );
    desk.check();
}

// --- descriptor reveal (tests/test_rust_wallet_reveal.py) ---

const ACK: &str = r#"{"plaintext_reveal_ack": "COPY LOCAL SECRET"}"#;

fn reveal_desk(golden: &str) -> Desk {
    let mut desk = Desk::with_seed(golden, None);
    desk.set_seed(&format!(
        "{DESKTOP_SQL}INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES\
         ('hot','w','p','Hot','descriptor','{{\"descriptor\": \" wpkh({XPUB}/0/*) \", \"change_descriptor\": \"wpkh({XPUB}/1/*)\", \"blinding_key\": \"synthetic-not-revealed\", \"backend\": \"fulcrum\"}}','2025'),\
         ('xp','w','p','Xpub only','xpub','{{\"xpub\": \"{MASTER}\", \"script_types\": [\"p2wpkh\"]}}','2025'),\
         ('sp','w','p','Silent','silent-payment','{{\"sp_descriptor\": \"sp(spscan1qsynthetic)\", \"descriptor\": 0}}','2025');"
    ));
    desk
}

#[test]
fn onboarding_reveal_plaintext_acknowledgement_and_material_match_python() {
    let mut desk =
        reveal_desk("onboarding_reveal_plaintext_acknowledgement_and_material_match_python");
    let args = [
        r#"{"name": 3}"#.to_owned(),
        r#"{"name": ""}"#.to_owned(),
        r#"{"wallet": " ", "backend": "hot"}"#.to_owned(),
        r#"{"name": "hot"}"#.to_owned(),
        r#"{"name": "hot", "auth_response": []}"#.to_owned(),
        r#"{"name": "hot", "auth_response": {}}"#.to_owned(),
        r#"{"name": "hot", "auth_response": {"plaintext_reveal_ack": "copy local secret"}}"#
            .to_owned(),
        format!(r#"{{"name": "hot", "auth_response": {ACK}}}"#),
        format!(r#"{{"wallet": "HOT", "auth_response": {ACK}}}"#),
        format!(r#"{{"backend": "xp", "auth_response": {ACK}}}"#),
        format!(r#"{{"name": "sp", "auth_response": {ACK}}}"#),
        format!(r#"{{"name": "absent", "auth_response": {ACK}}}"#),
        format!(r#"{{"name": "cold", "auth_response": {ACK}}}"#),
        format!(r#"{{"name": "hot", "workspace": "w", "profile": "p", "auth_response": {ACK}}}"#),
        format!(
            r#"{{"name": "hot", "workspace": "w", "profile": "foreign", "auth_response": {ACK}}}"#
        ),
        format!(r#"{{"name": "hot", "profile": "nope", "auth_response": {ACK}}}"#),
    ];
    let mut all: Vec<Option<&str>> = vec![None, Some("null"), Some("[]"), Some("{}")];
    all.extend(args.iter().map(|a| Some(a.as_str())));
    desk.session("wallets.reveal_descriptor", &all, false);
    desk.check();
}

#[test]
fn onboarding_reveal_encrypted_books_require_a_fresh_passphrase() {
    let mut desk = reveal_desk("onboarding_reveal_encrypted_books_require_a_fresh_passphrase");
    desk.encrypt("synthetic reveal key");
    let responses = desk.session(
        "wallets.reveal_descriptor",
        &[
            Some(r#"{"name": "hot"}"#),
            Some(&format!(r#"{{"name": "hot", "auth_response": {ACK}}}"#)),
            Some(r#"{"name": "hot", "auth_response": {"passphrase_secret": ""}}"#),
            Some(r#"{"name": "hot", "auth_response": {"passphrase_secret": "wrong"}}"#),
            Some(r#"{"name": "hot", "auth_response": {"passphrase_secret": "synthetic reveal key"}}"#),
        ],
        false,
    );
    let kinds: Vec<&str> = responses.iter().map(kind_of).collect();
    assert_eq!(
        kinds,
        [
            "auth_required",
            "auth_required",
            "auth_required",
            "error",
            "wallets.reveal_descriptor"
        ]
    );
    assert_eq!(
        path(&responses[0], &["data", "scope"]).and_then(text),
        Some("reveal_descriptor")
    );
    assert!(!desk.golden.text().contains("synthetic-not-revealed"));
    desk.check();
}
