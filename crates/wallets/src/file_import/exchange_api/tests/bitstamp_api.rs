//! Bitstamp's API v2 over the signed loopback fake, in the shapes of
//! Bitstamp's published OpenAPI document (<https://www.bitstamp.net/api/>):
//! `POST /api/v2/user_transactions/` (`id`, `datetime`, numeric `type` as a
//! string, `fee`, one signed amount per currency key, the pair rate under
//! `<base>_<quote>`, `order_id`), `POST /api/v2/account_balances/btc/`, and
//! errors `{"status": "error", "code", "reason"}`. Every value is synthetic.
//! Kept out of rustfmt (tests.rs) so its fixture tables stay one row per line.
use super::*;
use crate::file_import::tests::{import_into, BITSTAMP_CSV as CSV};
use serde_json::Value as J;
use std::sync::Mutex;

/// A user transaction as Bitstamp lists it: every currency key the account
/// knows, zero unless it moved (`usd` as a JSON number, as Bitstamp sends some).
fn tx(id: i64, kind: &str, at: &str, fields: &[(&str, J)]) -> J {
    let mut row = json!({"id": id, "datetime": at, "type": kind, "fee": "0.00", "usd": 0.0,
        "eur": "0.00", "btc": "0.00000000", "eth": "0.00000000", "order_id": null});
    for (key, value) in fields {
        row[*key] = value.clone();
    }
    row
}

/// Every type Kassiber maps, plus what it skips, then 1,000 EUR deposits so
/// the sync pages.
fn history() -> Vec<J> {
    let mut rows = vec![
        tx(1, "0", "2024-01-01 09:00:00.000000", &[("eur", json!("1000.00"))]),
        tx(2, "2", "2024-01-02 10:00:00.123000", &[("btc", json!("0.01000000")), ("eur", json!("-400.00")), ("btc_eur", json!(40000.0)), ("fee", json!("1.20")), ("order_id", json!(9001))]),
        tx(3, "0", "2024-01-03 00:00:00", &[("btc", json!("0.00500000"))]),
        tx(4, "1", "2024-01-04 12:30:45.999999", &[("btc", json!("-0.00400000")), ("fee", json!("0.00010000"))]),
        tx(5, "2", "2024-01-05 00:00:00", &[("btc", json!("-0.00200000")), ("eur", json!("84.00")), ("btc_eur", json!("42000.00")), ("fee", json!("0.34")), ("order_id", json!(9002)), ("self_trade", json!(false))]),
        tx(6, "14", "2024-01-06 00:00:00", &[("btc", json!("-0.00100000"))]),
        tx(7, "35", "2024-01-07 00:00:00", &[("btc", json!("0.00050000"))]),
        tx(8, "32", "2024-01-08 00:00:00", &[("btc", json!("0.00001000"))]),
        tx(9, "0", "2024-01-09 00:00:00", &[("eth", json!("1.00000000"))]),
        tx(10, "2", "2024-01-10 00:00:00", &[("eth", json!("0.50000000")), ("eur", json!("-1000.00")), ("eth_eur", json!(2000))]),
        tx(11, "36", "2024-01-11 00:00:00", &[("btc", json!("0.00100000")), ("eur", json!("-50.00")), ("btc_eur", json!("50000")), ("fee", json!("0.50"))]),
        tx(12, "1", "2024-01-12 00:00:00", &[("eur", json!("-100.00"))]),
    ];
    rows.extend((0..1000).map(|n| tx(100 + n, "0", "2024-02-01 00:00:00", &[("eur", json!("1.00"))])));
    rows
}

const BALANCE: &str = r#"{"currency": "btc", "total": "0.00851000", "available": "0.00800000", "reserved": "0.00051000"}"#;

/// The scripted server: Bitstamp's `since_id` cursor over `rows`.
#[derive(Clone)]
struct Script {
    rows: Vec<J>,
    /// Replies served instead of the scripted one, by request ordinal.
    faults: Vec<(usize, u16, String)>,
}
impl Script {
    fn new() -> Self {
        Self { rows: history(), faults: vec![] }
    }
    fn with(rows: Vec<J>) -> Self {
        Self { rows, faults: vec![] }
    }
    fn reply(&self, ordinal: usize, target: &str) -> (u16, String) {
        if let Some((_, status, body)) = self.faults.iter().find(|(at, _, _)| *at == ordinal) {
            return (*status, body.clone());
        }
        let (path, body) = target.split_once('?').unwrap_or((target, ""));
        let param = |name: &str| body.split('&').find_map(|p| p.strip_prefix(&format!("{name}="))).map(|v| v.parse::<u64>().unwrap());
        match path {
            "/api/v2/user_transactions/" => {
                let since = param("since_id").unwrap_or(0);
                let mut page: Vec<J> = self.rows.iter().filter(|r| r["id"].as_u64().unwrap() >= since).cloned().collect();
                page.sort_by_key(|r| r["id"].as_u64().unwrap());
                page.truncate(param("limit").unwrap() as usize);
                (200, J::Array(page).to_string())
            }
            "/api/v2/account_balances/btc/" => (200, BALANCE.into()),
            _ => (404, r#"{"status": "error", "reason": "Not found"}"#.into()),
        }
    }
}

/// The fake, and every request's `X-Auth-Nonce`.
fn serve_script(script: Script) -> (loopback::Fake, Arc<Mutex<Vec<String>>>) {
    let (script, nonces) = (Mutex::new(script), Arc::new(Mutex::new(vec![])));
    let seen = nonces.clone();
    let fake = loopback::serve_seeing("bitstamp", move |ordinal, _, target, headers| {
        let nonce = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("X-Auth-Nonce")).unwrap();
        seen.lock().unwrap().push(nonce.1.clone());
        script.lock().unwrap().reply(ordinal, target)
    });
    (fake, nonces)
}
fn targets(fake: &loopback::Fake) -> Vec<String> {
    fake.seen.lock().unwrap().iter().map(|r| r.target.clone()).collect()
}
fn sync(script: Script) -> (loopback::Fake, loopback::Run) {
    let fake = serve_script(script).0;
    let run = run(&backend("bitstamp", &fake.url), "bitstamp", "", true, online());
    (fake, run)
}
fn refusal(script: Script) -> AppError {
    let (_, run) = sync(script);
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    run.result.unwrap_err()
}
/// A EUR book with a BTC account, for syncs and imports into one wallet.
fn book() -> Connection {
    let conn = crate::file_import::tests::fixture();
    conn.execute_batch("UPDATE profiles SET fiat_currency='EUR';
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','BTC','fixed');").unwrap();
    conn
}
fn sync_into(conn: &Connection, fake: &loopback::Fake) -> Py {
    let gate = Gate::new(|| kassiber_net::Offline::Online, kassiber_net::Ledger::default());
    let consent = &mut |route: &Route| Some(Consent::explicit_trigger(route.request()));
    execute(conn, "p", "", &backend("bitstamp", &fake.url), "bitstamp", &gate, "test", consent, None).unwrap()
}

type Row = (String, String, i64, i64, Option<String>, Option<String>, String);
fn rows(conn: &Connection) -> Vec<Row> {
    let sql = "SELECT external_id, kind, amount, fee, fiat_value_exact, pricing_method, occurred_at
        FROM transactions ORDER BY occurred_at, external_id";
    let mut statement = conn.prepare(sql).unwrap();
    let read = |r: &kassiber_store::rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?));
    statement.query_map([], read).unwrap().collect::<Result<_, _>>().unwrap()
}
/// What the API imports; the CSV export imports the same rows but 6, 7 and
/// 11 (its transfers are unsigned, its sample has no simple buy).
fn expected() -> Vec<Row> {
    let row = |id: &str, kind: &str, amount, fee, value: Option<&str>, at: &str| -> Row {
        (format!("bitstamp:{id:0>20}"), kind.into(), amount, fee, value.map(Into::into), value.map(|_| "bitstamp".into()), at.into())
    };
    vec![
        // A buy costs the EUR amount plus the EUR fee.
        row("2", "buy", 1_000_000_000, 0, Some("401.20"), "2024-01-02T10:00:00Z"),
        row("3", "deposit", 500_000_000, 0, None, "2024-01-03T00:00:00Z"),
        // The BTC fee is on top of the amount withdrawn.
        row("4", "withdrawal", 400_000_000, 10_000_000, None, "2024-01-04T12:30:45Z"),
        // A sell yields the EUR amount less the EUR fee.
        row("5", "sell", 200_000_000, 0, Some("83.66"), "2024-01-05T00:00:00Z"),
        row("6", "withdrawal", 100_000_000, 0, None, "2024-01-06T00:00:00Z"),
        row("7", "deposit", 50_000_000, 0, None, "2024-01-07T00:00:00Z"),
        row("8", "income", 1_000_000, 0, None, "2024-01-08T00:00:00Z"),
        row("11", "buy", 100_000_000, 0, Some("50.50"), "2024-01-11T00:00:00Z"),
    ]
}

#[test]
fn bitstamp_reads_only_and_maps_every_btc_event() {
    let (fake, nonces) = serve_script(Script::new());
    let conn = book();
    let result = sync_into(&conn, &fake);
    assert_eq!(at(&result, "imported"), &Py::from(8), "{result:?}");
    assert_eq!(at(&result, "source"), &s("bitstamp:api:fixture"));
    let dump = |value: &Py| kassiber_pyjson::dumps(value, &Default::default()).unwrap();
    assert_eq!(dump(at(&result, "skipped")), r#"{"fiat_only_transactions": 1002, "non_btc_transactions": 2}"#);
    let balance = dump(at(&result, "reported_balance"));
    assert!(balance.starts_with(r#"{"asset": "BTC", "amount_msat": 851000000, "as_of": ""#), "{balance}");
    assert!(balance.ends_with(r#"Z", "scope": "account_available_plus_reserved"}"#), "{balance}");
    // Signed POSTs only: full pages by the id cursor, then the balance, each
    // with its own nonce.
    assert!(fake.seen.lock().unwrap().iter().all(|r| r.signed));
    assert_eq!(targets(&fake), [
        "/api/v2/user_transactions/?limit=1000&since_id=1&sort=asc",
        "/api/v2/user_transactions/?limit=1000&since_id=1088&sort=asc",
        "/api/v2/account_balances/btc/?",
    ]);
    let nonces = nonces.lock().unwrap().clone();
    assert_eq!(nonces.iter().collect::<std::collections::BTreeSet<_>>().len(), 3);
    assert_eq!(rows(&conn), expected());
    let sql = "SELECT raw_json, pricing_external_ref, pricing_pair, description FROM transactions WHERE external_id='bitstamp:00000000000000000005'";
    let stored: (String, String, String, String) = conn.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
    assert!(stored.0.contains(r#""order_id": 9002"#), "{}", stored.0);
    assert_eq!((stored.1.as_str(), stored.2.as_str(), stored.3.as_str()), ("5", "BTC-EUR", "Bitstamp market trade"));
    // A second sync adds nothing.
    assert_eq!(at(&sync_into(&conn, &fake), "imported"), &Py::from(0));
    assert_eq!(count(&conn, "SELECT count(*) FROM wallets WHERE kind='bitstamp'"), 1);
}

/// The export and the API name one transaction alike, so importing both into
/// one wallet, in either order, keeps one record per transaction.
#[test]
fn bitstamp_csv_and_api_dedupe_in_either_order() {
    for csv_first in [true, false] {
        let (conn, fake) = (book(), serve_script(Script::new()).0);
        let file = || import_into(&conn, CSV, crate::file_import::Format::Bitstamp, "").unwrap();
        let (first, second) = match csv_first {
            true => (file(), sync_into(&conn, &fake)),
            false => (sync_into(&conn, &fake), file()),
        };
        let added = if csv_first { (5, 3) } else { (8, 0) };
        assert_eq!((at(&first, "imported"), at(&second, "imported")), (&Py::from(added.0), &Py::from(added.1)), "{csv_first}");
        assert_eq!(rows(&conn), expected(), "csv first: {csv_first}");
    }
}

/// One BTC row Kassiber cannot book exactly refuses the whole sync by name,
/// after the history and before the balance.
#[test]
fn bitstamp_refuses_btc_rows_it_cannot_book_by_name() {
    let at = "2024-08-01 00:00:00";
    let trade = |btc: &str, eur: &str, rate: &str, fee: &str| tx(50, "2", at, &[("btc", json!(btc)), ("eur", json!(eur)), ("btc_eur", json!(rate)), ("fee", json!(fee))]);
    for (row, message) in [
        (tx(50, "2", at, &[("btc", json!("-0.10000000")), ("eth", json!("2.00000000")), ("eth_btc", json!("0.05"))]),
            "(type 'market trade') trades BTC against ETH, which is not a fiat currency"),
        (tx(50, "2", at, &[("btc", json!("0.10000000")), ("usdc", json!("-6000.00")), ("btc_usdc", json!("60000"))]),
            "(type 'market trade') trades BTC against USDC, which is not a fiat currency"),
        (tx(50, "99", at, &[("btc", json!("0.00010000"))]), "(type 'an undocumented type') has a type Kassiber does not map"),
        (tx(50, "27", at, &[("btc", json!("0.00010000"))]), "(type 'staking reward') is type 27, which Kassiber does not import"),
        (tx(50, "0", at, &[("btc", json!("0.00010000")), ("fee", J::Null)]), "(type 'deposit') does not state its fee"),
        (tx(50, "0", at, &[("btc", json!("0.00010000")), ("eth", json!("1.0"))]), "(type 'deposit') moves BTC together with ETH"),
        (tx(50, "0", at, &[("btc", json!("-0.00010000"))]), "(type 'deposit') moves BTC the other way"),
        (tx(50, "0", at, &[("btc", json!("0.00010000")), ("fee", json!("0.00001"))]), "(type 'deposit') charges a BTC fee Kassiber cannot place"),
        (tx(50, "0", at, &[("btc", json!("0.000000000001"))]), "(type 'deposit') has a BTC amount that is not a whole number of millisatoshi"),
        (tx(50, "32", at, &[("btc", json!("-0.00010000"))]), "(type 'referral reward') is not a plain BTC credit"),
        (tx(50, "0", at, &[("fee", json!("0.10"))]), "(type 'deposit') charges a fee without moving any amount"),
        (tx(50, "2", at, &[("btc", json!("0.001")), ("eur", json!("100.00")), ("btc_eur", json!("100000"))]), "(type 'market trade') moves BTC and EUR the same way"),
        (tx(50, "2", at, &[("btc", json!("0.001")), ("eur", json!("-100.00"))]), "(type 'market trade') has no BTC exchange rate"),
        (trade("0.001", "-102.00", "100000", "0.25"), "(type 'market trade') has a EUR amount that is not its BTC amount times its rate"),
        (trade("0.001", "-100.00", "100000", "0.01"), "(type 'market trade') has a fee too small to tell whether its EUR amount includes it"),
        (trade("-0.001", "0.10", "100", "0.50"), "(type 'market trade') has a fee that exceeds its proceeds"),
        (trade("0.001", "-100.00", "100000.00000000000000000000001", "0.25"),
            "(type 'market trade') has a EUR amount, rate or fee with more digits than Kassiber holds exactly"),
    ] {
        let mut rows = history();
        rows.push(row);
        let error = refusal(Script::with(rows));
        assert_eq!(error.code, "validation");
        assert_eq!(error.message, format!("Bitstamp transaction '50' {message}"));
    }
}

/// A refused key reports only Kassiber's values: the status and a
/// documented code with its hint, never Bitstamp's text (which may echo the key).
#[test]
fn bitstamp_errors_keep_the_documented_code_and_none_of_the_text() {
    let reason = format!("Missing permission for {}", loopback::KEY);
    for (status, code, kind, message, details) in [
        (403, "API0003", "auth_error", "Bitstamp refused the API key (HTTP 403, API0003)", r#"{"http_status": 403, "bitstamp_code": "API0003"}"#),
        (403, "x-unknown", "auth_error", "Bitstamp refused the API key (HTTP 403)", r#"{"http_status": 403, "bitstamp_code": "unknown"}"#),
        (200, "API0005", "auth_error", "Bitstamp refused the API key (API0005)", r#"{"http_status": null, "bitstamp_code": "API0005"}"#),
        (400, "400.002", "rate_limited", "Bitstamp rate limited the request", r#"{"http_status": 400, "bitstamp_code": "400.002"}"#),
        (200, "x-unknown", "protocol_error", "Bitstamp refused the request", r#"{"http_status": null, "bitstamp_code": "unknown"}"#),
    ] {
        let mut script = Script::new();
        script.faults = vec![(0, status, json!({"status": "error", "code": code, "reason": reason}).to_string())];
        let error = refusal(script);
        assert_eq!((error.code.as_str(), error.message.as_str()), (kind, message), "{code}");
        assert_eq!(kassiber_pyjson::dumps(error.details.as_ref().unwrap(), &Default::default()).unwrap(), details);
        assert!(!format!("{error:?}").contains(loopback::KEY));
    }
}

#[test]
fn bitstamp_throttling_is_retried_with_a_fresh_signature_then_imports() {
    let mut script = Script::new();
    script.faults = vec![(1, 429, "{}".into())];
    let (fake, nonces) = serve_script(script);
    let run = run(&backend("bitstamp", &fake.url), "bitstamp", "", true, online());
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(8));
    let targets = targets(&fake);
    assert_eq!(targets[1], targets[2]);
    // The fake checks every signature; the retry's nonce is new.
    assert!(fake.seen.lock().unwrap().iter().all(|r| r.signed));
    let nonces = nonces.lock().unwrap();
    assert_ne!(nonces[1], nonces[2]);
}

#[test]
fn bitstamp_malformed_pages_fail_closed() {
    let page = |row: J| J::Array(vec![row]).to_string();
    let deposit = |key: &str, value: J| {
        let mut row = tx(1, "0", "2024-01-01 00:00:00", &[]);
        row[key] = value;
        page(row)
    };
    for (ordinal, body, message) in [
        (0, "not json".to_owned(), "Bitstamp response was not JSON"),
        (0, "{}".to_owned(), "Bitstamp user transactions were not a JSON array"),
        (0, "[1]".to_owned(), "Bitstamp user transaction without an id"),
        (0, r#"[{"id": "x"}]"#.to_owned(), "Bitstamp user transaction without an id"),
        (0, page(json!([2])), "Bitstamp user transaction without an id"),
        (0, deposit("type", json!("deposit")), "Bitstamp transaction '1' has no numeric type"),
        (0, deposit("datetime", json!("2024-01-01T00:00:00")), "Bitstamp transaction '1' has a date and time Kassiber cannot read"),
        (0, deposit("datetime", json!("2024-01-01 00:00:00Z")), "Bitstamp transaction '1' has a date and time Kassiber cannot read"),
        (0, deposit("fee", json!("-0.1")), "Bitstamp transaction '1' has a negative fee"),
        (0, deposit("fee", json!("1e-3")), "Bitstamp transaction '1' has a fee that is not an exact decimal"),
        (0, deposit("btc", json!(true)), "Bitstamp transaction '1' has a BTC amount that is not an exact decimal"),
        (0, deposit("eth", J::Null), "Bitstamp transaction '1' has a ETH amount that is null"),
        (0, deposit("eth_eur", json!("x")), "Bitstamp transaction '1' has a ETH/EUR rate that is not an exact decimal"),
        (0, J::Array(vec![tx(5, "0", "2024-01-01 00:00:00", &[]), tx(4, "0", "2024-01-01 00:00:00", &[])]).to_string(),
            "Bitstamp returned transaction 4 out of id order; nothing was imported"),
        (2, r#"{"currency": "eur", "total": "1", "available": "1", "reserved": "0"}"#.to_owned(), "Bitstamp balance is not the BTC balance"),
        (2, r#"{"currency": "btc", "total": "1e-3", "available": "1e-3", "reserved": "0"}"#.to_owned(), "Bitstamp BTC balance was not an exact BTC amount"),
        (2, r#"{"currency": "btc", "total": "1.1", "available": "1", "reserved": "0"}"#.to_owned(),
            "Bitstamp BTC balance total is not its available plus reserved amount"),
    ] {
        let mut script = Script::new();
        script.faults = vec![(ordinal, 200, body)];
        assert_eq!(refusal(script).message, message, "{ordinal}");
    }
}

/// No request before consent, with the egress switch closed, or without both
/// credentials. The wallet itself is created first, as for every exchange.
#[test]
fn bitstamp_needs_consent_an_open_gate_and_both_credentials_before_any_request() {
    let fake = serve("bitstamp", |_, _, _| (200, "[]".into()));
    let refused = run(&backend("bitstamp", &fake.url), "bitstamp", "", false, online());
    assert_eq!(refused.result.unwrap_err().code, "interaction_required");
    assert_eq!(count(&refused.conn, "SELECT count(*) FROM wallets WHERE label='Bitstamp'"), 1);
    let offline = Arc::new(AtomicBool::new(true));
    assert!(run(&backend("bitstamp", &fake.url), "bitstamp", "", true, offline).result.is_err());
    for field in ["token", "auth_header"] {
        let mut missing = backend("bitstamp", &fake.url);
        missing[field] = json!(" ");
        let error = run(&missing, "bitstamp", "", true, online()).result.unwrap_err();
        assert_eq!((error.code.as_str(), error.message.as_str()), ("config_error", "Bitstamp backend is missing API key/secret"));
    }
    assert!(fake.seen.lock().unwrap().is_empty());
}

#[test]
fn bitstamp_signs_the_documented_text_over_the_base_urls_host_and_path() {
    // `printf %s 'BITSTAMP synthetic-keyPOSTwww.bitstamp.net/api/v2/user_transactions/application/x-www-form-urlencoded<nonce>1700000000000v2limit=1000&offset=0&sort=asc' | openssl dgst -sha256 -hmac synthetic-secret`
    let nonce = "f93c979d-b00d-43a9-9b9c-fd4cd9547fa6";
    let sign = |path, content_type, body| bitstamp::signature("synthetic-secret", ["synthetic-key", "POST", "www.bitstamp.net", path, content_type, nonce, "1700000000000", body]);
    assert_eq!(sign("/api/v2/user_transactions/", "application/x-www-form-urlencoded", "limit=1000&offset=0&sort=asc"), "23a19daf35f71d85260c2caf57244a87ae5fcac26b84383e571d28918df65cae");
    // Without a body the signed text has no content type either.
    assert_eq!(sign("/api/v2/account_balances/btc/", "", ""), "9972b858457a176dae6cb4d7c792baf5dd038dc526a4151d4a4e982176eb3b65");
    for (base, url, host, path) in [
        ("https://www.bitstamp.net", "https://www.bitstamp.net/api/v2/x/", "www.bitstamp.net", "/api/v2/x/"),
        ("https://www.bitstamp.net:443", "https://www.bitstamp.net:443/api/v2/x/", "www.bitstamp.net", "/api/v2/x/"),
        ("http://127.0.0.1:8080/proxy", "http://127.0.0.1:8080/proxy/api/v2/x/", "127.0.0.1:8080", "/proxy/api/v2/x/"),
    ] {
        assert_eq!(bitstamp::target(base, "/api/v2/x/").unwrap(), (url.into(), host.into(), path.into()));
    }
    for hostile in ["ftp://www.bitstamp.net", "https://user:pw@www.bitstamp.net", "https://www.bitstamp.net/?x=1", "www.bitstamp.net"] {
        assert_eq!(bitstamp::target(hostile, "/api/v2/x/").unwrap_err().code, "config_error", "{hostile}");
    }
}

/// The key check reads the balance and one transaction and cannot tell
/// whether the key may trade.
#[test]
fn bitstamp_check_reads_the_balance_and_one_row() {
    let (fake, _) = serve_script(Script::new());
    let gate = Gate::new(|| kassiber_net::Offline::Online, kassiber_net::Ledger::default());
    let consent = &mut |route: &Route| Some(Consent::explicit_trigger(route.request()));
    let checked = desktop::probe(&crate::file_import::tests::fixture(), &backend("bitstamp", &fake.url), &gate, "test", consent).unwrap();
    assert_eq!(at(&checked, "read_only"), &Py::Null);
    assert_eq!(targets(&fake), ["/api/v2/account_balances/btc/?", "/api/v2/user_transactions/?limit=1&offset=0&sort=asc"]);
}

/// Rows are stored in time order to the microsecond (Bitstamp pages by id,
/// ties keep the id order), and a later sync's row in a second an earlier
/// one filled with the opposite direction is tagged for review with it.
#[test]
fn bitstamp_orders_same_second_rows_by_time_and_tags_them_across_syncs() {
    let buy = tx(61, "2", "2024-08-01 10:00:00.100000", &[("btc", json!("0.001")), ("eur", json!("-60.00")), ("btc_eur", json!("60000")), ("fee", json!("0.15"))]);
    let sell = tx(60, "2", "2024-08-01 10:00:00.900000", &[("btc", json!("-0.001")), ("eur", json!("60.00")), ("btc_eur", json!("60000")), ("fee", json!("0.15"))]);
    let result = sync(Script::with(vec![sell.clone(), buy.clone()])).1.result.unwrap();
    let Py::List(inserted) = at(&result, "inserted_records") else { panic!("inserted records") };
    let order: Vec<&Py> = inserted.iter().map(|r| at(r, "external_id")).collect();
    assert_eq!(order, [&s("bitstamp:00000000000000000061"), &s("bitstamp:00000000000000000060")]);
    let conn = book();
    for rows in [vec![buy.clone()], vec![buy, sell]] {
        sync_into(&conn, &serve_script(Script::with(rows)).0);
    }
    let tagged = "SELECT group_concat(id) FROM (SELECT t.external_id AS id FROM transactions t JOIN transaction_tags x
        ON x.transaction_id=t.id JOIN tags g ON g.id=x.tag_id WHERE g.code='same-second-order-uncertain' ORDER BY 1)";
    let tagged: String = conn.query_row(tagged, [], |r| r.get(0)).unwrap();
    assert_eq!(tagged, "bitstamp:00000000000000000060,bitstamp:00000000000000000061");
}

/// Bitstamp's `X-Auth` headers are request credentials: a page that echoes
/// the request's signature refuses the sync before anything is stored.
#[test]
fn bitstamp_refuses_a_page_echoing_its_signature() {
    let fake = loopback::serve_seeing("bitstamp", |_, _, target, headers| {
        let signature = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("X-Auth-Signature")).unwrap().1.clone();
        match target.starts_with("/api/v2/user_transactions/") {
            true => (200, J::Array(vec![tx(1, "0", "2024-01-01 00:00:00", &[("btc", json!("0.001")), ("order_id", json!(signature))])]).to_string()),
            false => (200, BALANCE.into()),
        }
    });
    let run = run(&backend("bitstamp", &fake.url), "bitstamp", "", true, online());
    assert_eq!(run.result.unwrap_err().code, "credential_echo");
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

/// Each signed request gets its own random version-4 UUID nonce.
#[test]
fn bitstamp_request_nonces_differ_between_builds() {
    let (first, second) = (bitstamp::nonce().unwrap(), bitstamp::nonce().unwrap());
    assert_ne!(first, second);
    for nonce in [first, second] {
        let groups: Vec<usize> = nonce.split('-').map(str::len).collect();
        assert_eq!((groups, &nonce[14..15]), (vec![8, 4, 4, 4, 12], "4"), "{nonce}");
    }
}

/// The Bitstamp wallet a sync or an import finds by label, in `scope`.
fn scoped(conn: &Connection, scope: &str) {
    let config = json!({"backend": "fixture", "asset_scope": scope}).to_string();
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('bs','ws','p','Bitstamp','bitstamp',?,'2025-01-01')", [config]).unwrap();
}
fn sync_scoped(script: Script, scope: &'static str) -> loopback::Run {
    let fake = serve_script(script).0;
    loopback::run_prepared(&backend("bitstamp", &fake.url), "bitstamp", "", true, online(), &|conn| scoped(conn, scope))
}
fn dumped(value: &Py) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}
/// `(external_id, asset, direction, kind, amount, fee, fiat_value)`, in store order.
type Stored = (String, String, String, String, i64, i64, Option<String>);
fn stored(conn: &Connection) -> Vec<Stored> {
    let sql = "SELECT external_id, asset, direction, kind, amount, fee, CAST(fiat_value_exact AS TEXT) FROM transactions ORDER BY occurred_at, external_id";
    let mut statement = conn.prepare(sql).unwrap();
    let read = |r: &kassiber_store::rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?));
    statement.query_map([], read).unwrap().collect::<Result<_, _>>().unwrap()
}
fn row(id: &str, asset: &str, direction: &str, kind: &str, amount: i64, fee: i64, value: Option<&str>) -> Stored {
    let (id, leg) = id.split_once(':').map_or((id, String::new()), |(id, leg)| (id, format!(":{leg}")));
    (format!("bitstamp:{id:0>20}{leg}"), asset.into(), direction.into(), kind.into(), amount, fee, value.map(Into::into))
}
const KEEP: &str = r#"kassiber wallets update --wallet 'Bitstamp' --config '{"asset_scope": "crypto"}'"#;

/// `bitcoin` scope keeps the BTC rows, counts each other asset a skipped
/// transaction moves (an ETH row of a type Kassiber does not know too), in
/// the sync and the export alike, and refuses a BTC trade against another
/// crypto asset naming the command that keeps both.
#[test]
fn bitstamp_bitcoin_scope_counts_other_assets_and_names_the_scope_command() {
    let run = sync_scoped(Script::new(), "bitcoin");
    let result = run.result.unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(8));
    let skipped = |rows: i64| format!(r#"{{"scope": "bitcoin", "skipped": {{"ETH": {rows}}}, "excluded": {{}}, "next_step": "{}, then sync or import again to keep ETH"}}"#, KEEP.replace('"', "\\\""));
    assert_eq!(dumped(at(&result, "assets")), skipped(2));
    let conn = book();
    scoped(&conn, "bitcoin");
    let unknown = format!("{CSV}60,Main Account,Deposit reverted,,2024-03-01T00:00:00Z,-1.00000000,ETH,,,,,,,\n");
    let file = import_into(&conn, &unknown, crate::file_import::Format::Bitstamp, "").unwrap();
    assert_eq!(at(&file, "imported"), &Py::from(5));
    assert_eq!(dumped(at(&file, "assets")), skipped(3));
    let at = "2024-08-01 00:00:00";
    let mut rows = history();
    rows.push(tx(50, "2", at, &[("btc", json!("-0.10000000")), ("eth", json!("2.00000000")), ("eth_btc", json!("0.05"))]));
    let error = sync_scoped(Script::with(rows), "bitcoin").result.unwrap_err();
    assert!(error.hint.as_deref().unwrap().contains(KEEP), "{error:?}");
}

/// `crypto` scope keeps every crypto asset exactly: fiat trades priced,
/// trades between two crypto assets as paired swap legs whose quote leg
/// carries the fee as a fiat trade's cash does, movements and rewards in
/// their own asset.
#[test]
fn bitstamp_crypto_scope_maps_trades_swaps_and_movements_of_every_asset() {
    let at = |day: u32| format!("2024-09-{day:02} 00:00:00");
    let rows = vec![
        tx(9, "0", &at(1), &[("eth", json!("1.00000000"))]),
        tx(10, "2", &at(2), &[("eth", json!("0.50000000")), ("eur", json!("-1000.00")), ("eth_eur", json!("2000.00")), ("fee", json!("2.00"))]),
        tx(20, "2", &at(3), &[("eth", json!("1.00000000")), ("btc", json!("-0.05000000")), ("eth_btc", json!("0.05000000")), ("fee", json!("0.00010000"))]),
        tx(21, "2", &at(4), &[("eth", json!("-1.00000000")), ("btc", json!("0.05000000")), ("eth_btc", json!("0.05000000")), ("fee", json!("0.00010000"))]),
        tx(22, "1", &at(5), &[("eth", json!("-0.20000000")), ("fee", json!("0.00100000"))]),
        tx(23, "32", &at(6), &[("eth", json!("0.01000000"))]),
        tx(24, "0", &at(7), &[("eur", json!("10.00"))]),
    ];
    let run = sync_scoped(Script::with(rows), "crypto");
    let result = run.result.unwrap();
    assert_eq!(dumped(at_key(&result, "assets")), r#"{"scope": "crypto", "skipped": {}, "excluded": {}}"#);
    assert_eq!(at_key(&result, "swap_pairs"), &Py::from(2));
    assert_eq!(dumped(at_key(&result, "skipped")), r#"{"fiat_only_transactions": 1, "non_btc_transactions": 0}"#);
    assert_eq!(stored(&run.conn), [
        row("9", "ETH", "inbound", "deposit", 100_000_000_000, 0, None),
        row("10", "ETH", "inbound", "buy", 50_000_000_000, 0, Some("1002.00")),
        // A buy pays the BTC fee on top of the BTC amount.
        row("20:in", "ETH", "inbound", "swap", 100_000_000_000, 0, None),
        row("20:out", "BTC", "outbound", "swap", 5_000_000_000, 10_000_000, None),
        // A sale receives the BTC amount less the fee.
        row("21:in", "BTC", "inbound", "swap", 4_990_000_000, 0, None),
        row("21:out", "ETH", "outbound", "swap", 100_000_000_000, 0, None),
        row("22", "ETH", "outbound", "withdrawal", 20_000_000_000, 100_000_000, None),
        row("23", "ETH", "inbound", "income", 1_000_000_000, 0, None),
    ]);
}
fn at_key<'a>(value: &'a Py, key: &str) -> &'a Py {
    at(value, key)
}
fn header() -> &'static str {
    CSV.lines().next().unwrap()
}

/// The export states each fee's currency: a fee in a third crypto asset is a
/// fee-only row, a swap's fee in its quote asset is placed as the API's is,
/// and an unsigned transfer of any kept asset is refused.
#[test]
fn bitstamp_csv_crypto_scope_places_third_asset_and_swap_fees() {
    let conn = book();
    scoped(&conn, "crypto");
    let csv = format!("{}\n70,Main Account,Market,Buy,2024-09-01T00:00:00Z,1.00000000,ETH,0.05000000,BTC,0.05000000,BTC,0.00010000,BTC,1
71,Main Account,Market,Buy,2024-09-02T00:00:00Z,0.01000000,BTC,400.00,EUR,40000.00,EUR,0.00100000,ETH,2
", header());
    let result = import_into(&conn, &csv, crate::file_import::Format::Bitstamp, "").unwrap();
    assert_eq!(at(&result, "swap_pairs"), &Py::from(1));
    assert_eq!(stored(&conn), [
        row("70:in", "ETH", "inbound", "swap", 100_000_000_000, 0, None),
        row("70:out", "BTC", "outbound", "swap", 5_000_000_000, 10_000_000, None),
        row("71", "BTC", "inbound", "buy", 1_000_000_000, 0, Some("400.00")),
        row("71:fee", "ETH", "outbound", "fee", 0, 100_000_000, None),
    ]);
    let transfer = format!("{}\n72,Main Account,Inter Account Transfer,,2024-09-03T00:00:00Z,1.00000000,ETH,,,,,,,\n", header());
    let error = import_into(&conn, &transfer, crate::file_import::Format::Bitstamp, "").unwrap_err();
    assert_eq!(error.message, "Bitstamp CSV line 2 (Inter Account Transfer) does not say which way its ETH moved");
}

/// A row cut short after its amount currency is refused: its missing fee is
/// no confirmed absence of a fee.
#[test]
fn bitstamp_csv_refuses_a_truncated_row() {
    let conn = book();
    let csv = format!("{}
73,Main Account,Withdrawal,,2024-09-04T00:00:00Z,0.01000000,BTC
", header());
    let error = import_into(&conn, &csv, crate::file_import::Format::Bitstamp, "").unwrap_err();
    assert_eq!(error.message, "Bitstamp CSV line 2 has fewer cells than the header");
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 0);
}

/// What `crypto` scope cannot book exactly still refuses the sync by name.
#[test]
fn bitstamp_crypto_scope_refuses_what_it_cannot_place() {
    let at = "2024-08-01 00:00:00";
    for (row, message) in [
        (tx(50, "2", at, &[("eth", json!("1.00000000")), ("btc", json!("-0.05000000"))]), "(type 'market trade') has no rate between its two assets"),
        (tx(50, "27", at, &[("eth", json!("0.01000000"))]), "(type 'staking reward') is type 27, which Kassiber does not import"),
        (tx(50, "0", at, &[("btc", json!("0.00010000")), ("eth", json!("1.00000000"))]), "(type 'deposit') moves BTC and ETH together"),
        (tx(50, "2", at, &[("eth", json!("1.00000000")), ("btc", json!("-0.05000000")), ("eth_btc", json!("0.05000000")), ("fee", json!("0.000000000001"))]),
            "(type 'market trade') has a BTC fee that is not a whole number of millisatoshi"),
        (tx(50, "2", at, &[("eth", json!("1.00000000")), ("btc", json!("-0.05000000")), ("eur", json!("-10.00")), ("eth_btc", json!("0.05000000"))]),
            "(type 'market trade') swaps BTC and ETH but also moves EUR"),
    ] {
        let run = sync_scoped(Script::with(vec![row]), "crypto");
        let error = run.result.unwrap_err();
        assert_eq!(error.message, format!("Bitstamp transaction '50' {message}"));
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
}

type Checkpoint = (String, String, String, i64, String, String, Option<String>);
fn checkpoints(conn: &Connection) -> Vec<Checkpoint> {
    let sql = "SELECT id, asset, as_of, amount_msat, origin, scope, origin_ref FROM balance_checkpoints ORDER BY created_at, id";
    let mut statement = conn.prepare(sql).unwrap();
    let read = |r: &kassiber_store::rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?));
    statement.query_map([], read).unwrap().collect::<Result<_, _>>().unwrap()
}

/// The balance the sync already read is stored as an `api` checkpoint of the
/// synced wallet, with no request of its own; the card output is unchanged.
#[test]
fn bitstamp_sync_stores_its_reported_balance_as_a_checkpoint() {
    let (fake, _) = serve_script(Script::new());
    let conn = book();
    let result = sync_into(&conn, &fake);
    assert_eq!(targets(&fake).len(), 3);
    let stored = checkpoints(&conn);
    assert_eq!(stored.len(), 1, "{stored:?}");
    let (id, asset, as_of, msat, origin, scope, source) = &stored[0];
    assert_eq!(at(&result, "balance_checkpoint_id"), &s(id.as_str()));
    assert_eq!((asset.as_str(), *msat, origin.as_str(), scope.as_str()), ("BTC", 851_000_000, "api", "account_available_plus_reserved"));
    assert_eq!(source.as_deref(), Some("bitstamp:api:fixture"));
    assert_eq!(at(at(&result, "reported_balance"), "as_of"), &s(as_of.as_str()));
    let wallet: String = conn.query_row("SELECT wallet_id FROM balance_checkpoints", [], |r| r.get(0)).unwrap();
    assert_eq!(count(&conn, &format!("SELECT count(*) FROM wallets WHERE id='{wallet}' AND kind='bitstamp'")), 1);
    let card = kassiber_pyjson::dumps(at(&result, "provider_balance"), &Default::default()).unwrap();
    assert_eq!(card, r#"{"status": "mismatch", "asset": "BTC", "provider_msat": 851000000, "book_msat": 941000000, "difference_msat": 90000000}"#);
}

/// A checkpoint that cannot be stored is rolled back alone: the history is
/// imported and the card still compares the balance.
#[test]
fn a_balance_checkpoint_that_cannot_be_stored_never_fails_the_sync() {
    let (fake, _) = serve_script(Script::new());
    let conn = book();
    conn.execute_batch("CREATE TEMP TRIGGER refuse_checkpoint BEFORE INSERT ON balance_checkpoints BEGIN SELECT RAISE(ABORT, 'refused'); END;").unwrap();
    let result = sync_into(&conn, &fake);
    assert_eq!(at(&result, "imported"), &Py::from(8));
    assert_eq!(at(&result, "balance_checkpoint_id"), &Py::Null);
    assert_eq!(at(at(&result, "provider_balance"), "status"), &s("mismatch"));
    assert_eq!(rows(&conn), expected());
    assert!(checkpoints(&conn).is_empty());
    assert!(conn.is_autocommit(), "the sync's write is closed");
}

/// A sync that fails, or is refused before publication, stores no checkpoint.
#[test]
fn a_failed_or_unpublished_sync_stores_no_checkpoint() {
    let mut script = Script::new();
    script.faults = vec![(2, 200, r#"{"currency": "btc", "total": "1", "available": "0", "reserved": "0"}"#.into())];
    let (_, run) = sync(script);
    assert_eq!(run.result.unwrap_err().code, "protocol_error");
    assert!(checkpoints(&run.conn).is_empty());

    use kassiber_store::rusqlite::functions::FunctionFlags;
    let fake = serve_script(Script::new()).0;
    let offline = online();
    let flag = offline.clone();
    let run = loopback::run_prepared(&backend("bitstamp", &fake.url), "bitstamp", "", true, offline, &move |conn| {
        let flag = flag.clone();
        conn.create_scalar_function("go_offline", 0, FunctionFlags::SQLITE_UTF8, move |_| {
            flag.store(true, Ordering::SeqCst);
            Ok(0)
        })
        .unwrap();
        conn.execute_batch("CREATE TEMP TRIGGER offline_on_checkpoint AFTER INSERT ON balance_checkpoints BEGIN SELECT go_offline(); END;").unwrap();
    });
    assert_eq!(run.result.unwrap_err().code, "network_egress_disabled");
    assert!(checkpoints(&run.conn).is_empty());
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}
