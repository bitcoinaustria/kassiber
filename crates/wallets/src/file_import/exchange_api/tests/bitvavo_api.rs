//! Bitvavo's REST API over a loopback fake built from the documented
//! response shapes: `GET /v2/account/history`
//! <https://docs.bitvavo.com/docs/rest-api/get-transaction-history/>,
//! `GET /v2/balance` <https://docs.bitvavo.com/docs/rest-api/get-account-balance/>
//! and the errors <https://docs.bitvavo.com/docs/errors/> lists. Kept out of
//! rustfmt (tests.rs) so its fixture tables stay one row per line.
use super::*;
use serde_json::Value as J;
use std::sync::Mutex;

/// One `/account/history` item; Bitvavo omits the legs a transaction lacks.
fn item(id: &str, kind: &str, at: &str, legs: &[(&str, &str, &str)]) -> J {
    let mut row = json!({"transactionId": id, "executedAt": at, "type": kind});
    for (prefix, currency, amount) in legs {
        row[format!("{prefix}Currency")] = json!(currency);
        row[format!("{prefix}Amount")] = json!(amount);
    }
    row
}

/// Every event kind Kassiber maps, one of each, plus what it skips. The buys
/// state their cash amount once before the fee and once after it.
fn history() -> Vec<J> {
    vec![
        item("dep-eur", "deposit", "2024-01-01T08:00:00.000Z", &[("received", "EUR", "1000"), ("fees", "EUR", "0")]),
        item("buy-gross", "buy", "2024-01-02T09:00:00.250Z", &[("sent", "EUR", "400.00"), ("received", "BTC", "0.01"), ("fees", "EUR", "1.00"), ("price", "EUR", "40000")]),
        item("buy-net", "buy", "2024-01-02T10:00:00.000Z", &[("sent", "EUR", "25.00"), ("received", "BTC", "0.0009999"), ("fees", "EUR", "0.062494"), ("price", "EUR", "24940")]),
        item("dep-btc", "deposit", "2024-01-03T00:00:00.000Z", &[("received", "BTC", "0.005"), ("fees", "BTC", "0")]),
        item("wd-btc", "withdrawal", "2024-07-05T12:30:45.999Z", &[("sent", "BTC", "0.004"), ("fees", "BTC", "0.0001")]),
        item("sell", "sell", "2024-07-06T00:00:00.000Z", &[("sent", "BTC", "0.002"), ("received", "EUR", "84.00"), ("fees", "EUR", "0.21"), ("price", "EUR", "42000")]),
        item("stake", "staking", "2024-07-07T00:00:00.000Z", &[("received", "BTC", "0.00001")]),
        item("aff-eur", "affiliate", "2024-07-08T00:00:00.000Z", &[("received", "EUR", "1.5")]),
        item("buy-eth", "buy", "2024-07-09T00:00:00.000Z", &[("sent", "EUR", "100"), ("received", "ETH", "0.04"), ("fees", "EUR", "0.25"), ("price", "EUR", "2500")]),
    ]
}

/// The scripted server: documented shapes, `page` items per page.
#[derive(Clone)]
struct Script {
    history: Vec<J>,
    page: usize,
    balance: String,
    /// Replies served instead of the scripted one, by request ordinal.
    faults: Vec<(usize, u16, String)>,
}
impl Script {
    fn new() -> Self {
        Self {
            history: history(),
            page: 4,
            balance: r#"[{"symbol":"BTC","available":"0.0038999","inOrder":"0.0001"}]"#.into(),
            faults: vec![],
        }
    }
    fn reply(&self, ordinal: usize, target: &str) -> (u16, String) {
        if let Some((_, status, body)) = self.faults.iter().find(|(at, _, _)| *at == ordinal) {
            return (*status, body.clone());
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let param = |name: &str| -> usize {
            let value = query.split('&').find_map(|p| p.strip_prefix(&format!("{name}=")));
            value.unwrap().parse().unwrap()
        };
        match path {
            "/v2/account/history" => {
                let (page, size) = (param("page"), self.page.min(param("maxItems")));
                let end = (page * size).min(self.history.len());
                let rows = &self.history[((page - 1) * size).min(end)..end];
                let total = self.history.len().div_ceil(size);
                let body = json!({"items": rows, "currentPage": page, "totalPages": total, "maxItems": size});
                (200, body.to_string())
            }
            "/v2/balance" => (200, self.balance.clone()),
            _ => (404, r#"{"errorCode":110,"error":"Invalid endpoint."}"#.into()),
        }
    }
}

fn serve_script(script: Script) -> loopback::Fake {
    let script = Mutex::new(script);
    serve("bitvavo", move |ordinal, _, target| script.lock().unwrap().reply(ordinal, target))
}
fn targets(fake: &loopback::Fake) -> Vec<String> {
    fake.seen.lock().unwrap().iter().map(|r| r.target.clone()).collect()
}
fn sync(script: Script) -> (loopback::Fake, loopback::Run) {
    let fake = serve_script(script);
    let run = run(&backend("bitvavo", &fake.url), "bitvavo", "", true, online());
    (fake, run)
}
fn refusal(script: Script) -> AppError {
    sync(script).1.result.unwrap_err()
}
fn with_item(row: J) -> Script {
    let mut script = Script::new();
    script.history.push(row);
    script
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
    execute(conn, "p", "", &backend("bitvavo", &fake.url), "bitvavo", &gate, "test", consent, None).unwrap()
}

type Row = (String, String, i64, i64, Option<String>, Option<String>, String);
fn rows(conn: &Connection) -> Vec<Row> {
    let sql = "SELECT external_id, kind, amount, fee, fiat_value_exact, pricing_method, occurred_at
        FROM transactions ORDER BY occurred_at, external_id";
    let mut statement = conn.prepare(sql).unwrap();
    let read = |r: &kassiber_store::rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?));
    statement.query_map([], read).unwrap().collect::<Result<_, _>>().unwrap()
}
/// What both sources import, with `method` on the priced rows.
fn expected(method: &str) -> Vec<Row> {
    let row = |id: &str, kind: &str, amount, fee, value: Option<&str>, at: &str| -> Row {
        let priced = value.map(|_| method.to_owned());
        (format!("bitvavo:{id}"), kind.into(), amount, fee, value.map(Into::into), priced, at.into())
    };
    vec![
        // The cash amount before the fee: the fee is added to the cost.
        row("buy-gross", "buy", 1_000_000_000, 0, Some("401"), "2024-01-02T09:00:00Z"),
        // The cash amount after the fee is the cost as it stands.
        row("buy-net", "buy", 99_990_000, 0, Some("25"), "2024-01-02T10:00:00Z"),
        row("dep-btc", "deposit", 500_000_000, 0, None, "2024-01-03T00:00:00Z"),
        // 0.004 left on chain, the 0.0001 fee on top.
        row("wd-btc", "withdrawal", 400_000_000, 10_000_000, None, "2024-07-05T12:30:45Z"),
        // Proceeds net of the fee.
        row("sell", "sell", 200_000_000, 0, Some("83.79"), "2024-07-06T00:00:00Z"),
        row("stake", "staking", 1_000_000, 0, None, "2024-07-07T00:00:00Z"),
    ]
}

#[test]
fn bitvavo_reads_only_and_maps_every_btc_event() {
    let (fake, run) = sync(Script::new());
    let result = run.result.unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(6), "{result:?}");
    assert_eq!(at(&result, "source"), &s("bitvavo:api:fixture"));
    assert_eq!(
        kassiber_pyjson::dumps(at(&result, "skipped"), &Default::default()).unwrap(),
        r#"{"fiat_only_transactions": 2, "non_btc_transactions": 1}"#
    );
    // Only signed GETs: three history pages with one fixed end, then the balance.
    assert!(fake.seen.lock().unwrap().iter().all(|r| r.signed));
    let targets = targets(&fake);
    let until = targets[0].split("toDate=").nth(1).unwrap().split('&').next().unwrap();
    assert_eq!(until.len(), 13);
    for (page, target) in targets[..3].iter().enumerate() {
        let page = page + 1;
        assert_eq!(target, &format!("/v2/account/history?fromDate=0&toDate={until}&page={page}&maxItems=100"));
    }
    assert_eq!(targets[3..], ["/v2/balance?symbol=BTC"]);
    assert_eq!(rows(&run.conn), expected("bitvavo_api"));
    // 0.0038999 available plus 0.0001 in orders.
    let balance = kassiber_pyjson::dumps(at(&result, "reported_balance"), &Default::default()).unwrap();
    assert!(balance.starts_with(r#"{"asset": "BTC", "amount_msat": 399990000, "as_of": ""#), "{balance}");
    assert!(balance.ends_with(r#"Z", "scope": "spot_available_plus_in_order"}"#), "{balance}");
    let sql = "SELECT raw_json, pricing_pair FROM transactions WHERE external_id='bitvavo:sell'";
    let (raw, pair): (String, String) = run.conn.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(raw.contains(r#""transactionId": "sell""#), "{raw}");
    assert_eq!(pair, "BTC-EUR");
    // A second sync adds nothing.
    assert_eq!(at(&sync_into(&run.conn, &fake), "imported"), &Py::from(0));
    assert_eq!(count(&run.conn, "SELECT count(*) FROM wallets WHERE kind='bitvavo'"), 1);
}

/// One BTC row Kassiber cannot book exactly refuses the whole sync by name,
/// after the history and before the balance.
#[test]
fn bitvavo_refuses_btc_rows_it_cannot_book_by_name() {
    let at = "2024-08-01T00:00:00Z";
    for (row, message) in [
        (item("usdc", "buy", at, &[("sent", "USDC", "60000"), ("received", "BTC", "1"), ("fees", "USDC", "15"), ("price", "USDC", "60000")]),
            "(type 'buy') trades BTC against USDC, which is not a fiat currency"),
        (item("eth", "sell", at, &[("sent", "BTC", "0.1"), ("received", "ETH", "2"), ("price", "ETH", "20")]),
            "(type 'sell') trades BTC against ETH, which is not a fiat currency"),
        (item("altfee", "buy", at, &[("sent", "EUR", "100"), ("received", "ETH", "0.04"), ("fees", "BTC", "0.0001"), ("price", "EUR", "2500")]),
            "(type 'buy') pays a BTC fee on a trade between EUR and ETH"),
        (item("convert", "convert_buy", at, &[("sent", "ETH", "2"), ("received", "BTC", "0.1")]),
            "(type 'convert_buy') is a Convert swap between BTC and another asset"),
        (item("margin", "margin_loan_borrow", at, &[("received", "BTC", "0.1")]),
            "(type 'margin_loan_borrow') is a margin entry; margin trading is out of scope"),
        (item("loan", "loan", at, &[("received", "BTC", "0.1")]), "(type 'loan') is a loan entry; lending is out of scope"),
        (item("rebate", "rebate", at, &[("received", "BTC", "0.0001")]), "(type 'rebate') has a type Kassiber does not map"),
        (item("btcfee", "buy", at, &[("sent", "EUR", "100"), ("received", "BTC", "0.001"), ("fees", "BTC", "0.00001"), ("price", "EUR", "100000")]),
            "(type 'buy') charges its fee in BTC instead of EUR; Kassiber cannot place it exactly"),
        (item("off", "buy", at, &[("sent", "EUR", "102"), ("received", "BTC", "0.001"), ("fees", "EUR", "1"), ("price", "EUR", "100000")]),
            "(type 'buy') has a cash amount that does not match its amount, price and fee"),
        (item("noprice", "sell", at, &[("sent", "BTC", "0.001"), ("received", "EUR", "100")]),
            "(type 'sell') has no EUR price to check its amounts against"),
        (item("fine", "deposit", at, &[("received", "BTC", "0.000000000001")]),
            "(type 'deposit') has a BTC amount that is not a whole number of millisatoshi"),
    ] {
        let id = row["transactionId"].as_str().unwrap().to_owned();
        let (fake, run) = sync(with_item(row));
        let error = run.result.unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(error.message, format!("Bitvavo transaction '{id}' {message}"));
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
        assert!(targets(&fake).iter().all(|t| t.starts_with("/v2/account/history")));
    }
}

/// A refused key reports only Kassiber's values: the status, a documented
/// `errorCode` and its hint, never Bitvavo's text (which echoes the key).
#[test]
fn bitvavo_auth_errors_keep_the_documented_code_and_none_of_the_text() {
    let mut script = Script::new();
    let body = format!(r#"{{"errorCode":311,"error":"No View access for {}"}}"#, loopback::KEY);
    script.faults = vec![(0, 403, body)];
    let error = refusal(script);
    assert_eq!((error.code.as_str(), error.message.as_str()), ("auth_error", "Bitvavo refused the API key (HTTP 403)"));
    assert!(error.hint.unwrap().contains("lacks View access"));
    let details = kassiber_pyjson::dumps(error.details.as_ref().unwrap(), &Default::default()).unwrap();
    assert_eq!(details, r#"{"http_status": 403, "bitvavo_error_code": 311}"#);
    // Another status keeps the shared transport's withheld body prefix.
    let mut script = Script::new();
    script.faults = vec![(1, 400, format!("{}{}", "x".repeat(195), loopback::SECRET))];
    let error = refusal(script);
    assert!(error.message.ends_with("x<reda"), "{}", error.message);
}

#[test]
fn bitvavo_throttling_is_retried_with_a_fresh_signature_then_imports() {
    let mut script = Script::new();
    script.faults = vec![(1, 429, r#"{"errorCode":105,"error":"Rate limit"}"#.into())];
    let (fake, run) = sync(script);
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(6));
    let targets = targets(&fake);
    assert_eq!(targets[1], targets[2]);
    // The fake checks every signature, the retry's too.
    assert!(fake.seen.lock().unwrap().iter().all(|r| r.signed));
    let mut script = Script::new();
    script.faults = (1..4).map(|n| (n, 429, "{}".to_owned())).collect();
    let (_, run) = sync(script);
    let error = run.result.unwrap_err();
    assert_eq!((error.code.as_str(), error.retryable), ("rate_limited", true));
    assert!(error.message.starts_with("Bitvavo rate limited"), "{}", error.message);
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

#[test]
fn bitvavo_malformed_pages_fail_closed() {
    let page = |rows: &str| format!(r#"{{"items":[{rows}],"currentPage":1,"totalPages":1}}"#);
    let deposit = |fields: &str| page(&format!(r#"{{"transactionId":"x","type":"deposit",{fields}}}"#));
    let full = vec!["{}"; 101].join(",");
    for (ordinal, body, message) in [
        (0, "not json".to_owned(), "Bitvavo response was not JSON"),
        (0, "[]".to_owned(), "Bitvavo history page was not a JSON object"),
        (0, r#"{"items":{},"currentPage":1,"totalPages":1}"#.to_owned(), "Bitvavo history items were not a JSON array"),
        (0, r#"{"items":[],"currentPage":2,"totalPages":3}"#.to_owned(), "Bitvavo returned a different history page"),
        (0, r#"{"items":[],"currentPage":1}"#.to_owned(), "Bitvavo history page has no page count"),
        (1, r#"{"items":[],"currentPage":2,"totalPages":4}"#.to_owned(), "Bitvavo history changed while it was read; sync again"),
        (0, page(&full), "Bitvavo history page holds more rows than requested"),
        (0, page(r#"{"type":"buy"}"#), "Bitvavo history row without a transactionId"),
        (0, deposit(r#""executedAt":"2024-01-01T00:00:00","receivedCurrency":"BTC","receivedAmount":"1""#),
            "Bitvavo transaction 'x' has an execution time without a time zone"),
        (0, deposit(r#""executedAt":"2024-01-01T00:00:00Z","receivedCurrency":"BTC","receivedAmount":"-1""#),
            "Bitvavo transaction 'x' has a receivedAmount that is not a plain decimal"),
        (0, deposit(r#""executedAt":"2024-01-01T00:00:00Z","receivedCurrency":"BTC","receivedAmount":0.001"#),
            "Bitvavo transaction 'x' has a receivedAmount that is not a plain decimal"),
        (0, deposit(r#""executedAt":"2024-01-01T00:00:00Z","receivedCurrency":"BTC""#),
            "Bitvavo transaction 'x' has receivedCurrency and receivedAmount that do not come together"),
        (3, r#"{"symbol":"BTC"}"#.to_owned(), "Bitvavo balance was not a JSON array"),
        (3, r#"[{"symbol":"BTC","available":"1e-3","inOrder":"0"}]"#.to_owned(), "Bitvavo BTC balance is not a plain decimal"),
        (3, r#"[{"symbol":"BTC","available":"0.000000000001","inOrder":"0"}]"#.to_owned(), "Bitvavo BTC balance is finer than a millisatoshi"),
        (3, r#"[{"symbol":"BTC","available":"1","inOrder":"0"},{"symbol":"BTC","available":"1","inOrder":"0"}]"#.to_owned(),
            "Bitvavo listed its BTC balance twice"),
    ] {
        let mut script = Script::new();
        script.faults = vec![(ordinal, 200, body)];
        assert_eq!(refusal(script).message, message, "{ordinal}");
    }
    // A repeated transaction must be identical; an identical one is read once.
    let mut script = Script::new();
    let mut twin = script.history[1].clone();
    script.history.push(twin.clone());
    assert_eq!(at(&sync(script.clone()).1.result.unwrap(), "imported"), &Py::from(6));
    twin["receivedAmount"] = json!("0.02");
    *script.history.last_mut().unwrap() = twin;
    assert_eq!(refusal(script).message, "Bitvavo returned transaction 'buy-gross' twice with different contents");
}

/// No request before consent, with the egress switch closed, or without both
/// credentials. The wallet itself is created first, as for every exchange.
#[test]
fn bitvavo_needs_consent_an_open_gate_and_both_credentials_before_any_request() {
    let fake = serve("bitvavo", |_, _, _| (200, "[]".into()));
    let refused = run(&backend("bitvavo", &fake.url), "bitvavo", "", false, online());
    assert_eq!(refused.result.unwrap_err().code, "interaction_required");
    assert_eq!(count(&refused.conn, "SELECT count(*) FROM wallets WHERE label='Bitvavo'"), 1);
    let offline = Arc::new(AtomicBool::new(true));
    assert!(run(&backend("bitvavo", &fake.url), "bitvavo", "", true, offline).result.is_err());
    for field in ["token", "auth_header"] {
        let mut missing = backend("bitvavo", &fake.url);
        missing[field] = json!(" ");
        let error = run(&missing, "bitvavo", "", true, online()).result.unwrap_err();
        assert_eq!((error.code.as_str(), error.message.as_str()), ("config_error", "Bitvavo backend is missing API key/secret"));
    }
    assert!(fake.seen.lock().unwrap().is_empty());
}

#[test]
fn bitvavo_signs_the_path_the_base_url_carries() {
    let path = "/v2/account/history?fromDate=0&toDate=1700000000000&page=1&maxItems=100";
    // `printf %s '1700000000000GET/v2/...' | openssl dgst -sha256 -hmac secret`
    assert_eq!(
        bitvavo::signature("secret", "1700000000000", path),
        "493ae6ba224a2d72d129a26a62f346fd6b2d50770fb6b72cccbe38e8c8066a15"
    );
}

/// The export's quote amount is after the fee; the API's is whichever single
/// reading fits within a cent, and both fitting imports unpriced.
#[test]
fn bitvavo_trade_values_follow_the_source_reading() {
    use bitvavo::{FeeReading::*, TradeValue::*};
    let d = |v: &str| v.parse::<Decimal>().unwrap();
    for (btc, price, cash, fee, buy, reading, expected) in [
        ("0.01", "40000", "400.00", "1.00", true, Unknown, Exact(d("401.00"))),
        ("0.002", "42000", "84.00", "0.21", false, Unknown, Exact(d("83.79"))),
        ("0.0009999", "24940", "25.00", "0.062494", true, Unknown, Exact(d("25.00"))),
        ("0.0002", "50000", "10.00", "0.004", true, Unknown, Ambiguous),
        ("0.001", "50000", "50.02", "0", true, Unknown, Mismatch),
        ("0.00054923", "34580", "19.03", "0.0376266", true, AfterFee, Exact(d("19.03"))),
        ("0.0002", "50000", "10.00", "0.004", false, AfterFee, Exact(d("10.00"))),
        ("0.01", "40000", "400.00", "1.00", true, AfterFee, Mismatch),
        ("0.0000001", "50000", "0.00", "0.005", false, AfterFee, Mismatch),
    ] {
        let value = bitvavo::trade_value(&d(btc), &d(price), &d(cash), &d(fee), buy, reading).unwrap();
        assert_eq!(value, expected, "{btc} {price} {cash} {fee} {reading:?}");
    }
}

/// Within one second the sync stores rows in the order they happened, and a
/// later sync's row in a second an earlier one filled with the opposite
/// direction is tagged for review with it.
#[test]
fn bitvavo_api_orders_same_second_rows_and_tags_them_across_syncs() {
    let buy = [("sent", "EUR", "60.00"), ("received", "BTC", "0.001"), ("fees", "EUR", "0.15"), ("price", "EUR", "60000")];
    let sell = [("sent", "BTC", "0.001"), ("received", "EUR", "60.00"), ("fees", "EUR", "0.15"), ("price", "EUR", "60000")];
    let earlier = item("zzz-earlier", "buy", "2024-08-01T10:00:00.100Z", &buy);
    let later = item("aaa-later", "sell", "2024-08-01T10:00:00.900Z", &sell);
    let mut script = Script::new();
    script.history = vec![later.clone(), earlier.clone()];
    let result = sync(script).1.result.unwrap();
    let Py::List(inserted) = at(&result, "inserted_records") else {
        panic!("inserted records")
    };
    let order: Vec<&Py> = inserted.iter().map(|r| at(r, "external_id")).collect();
    assert_eq!(order, [&s("bitvavo:zzz-earlier"), &s("bitvavo:aaa-later")]);
    let conn = book();
    for history in [vec![later.clone()], vec![later, earlier]] {
        let mut script = Script::new();
        script.history = history;
        sync_into(&conn, &serve_script(script));
    }
    let tagged = "SELECT group_concat(id) FROM (SELECT t.external_id AS id FROM transactions t JOIN transaction_tags x
        ON x.transaction_id=t.id JOIN tags g ON g.id=x.tag_id WHERE g.code='same-second-order-uncertain' ORDER BY 1)";
    let tagged: String = conn.query_row(tagged, [], |r| r.get(0)).unwrap();
    assert_eq!(tagged, "bitvavo:aaa-later,bitvavo:zzz-earlier");
}

/// The Bitvavo wallet a sync or an import finds by label, in `scope`.
fn scoped(conn: &Connection, scope: &str) {
    let config = json!({"backend": "fixture", "asset_scope": scope}).to_string();
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('bv','ws','p','Bitvavo','bitvavo',?,'2025-01-01')", [config]).unwrap();
}
fn sync_scoped(script: Script, scope: &'static str) -> loopback::Run {
    let fake = serve_script(script);
    loopback::run_prepared(&backend("bitvavo", &fake.url), "bitvavo", "", true, online(), &|conn| scoped(conn, scope))
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
    (format!("bitvavo:{id}"), asset.into(), direction.into(), kind.into(), amount, fee, value.map(Into::into))
}
const KEEP: &str = r#"kassiber wallets update --wallet 'Bitvavo' --config '{"asset_scope": "crypto"}'"#;

/// `bitcoin` scope keeps the BTC rows, counts each other asset a skipped
/// transaction moves, in the sync and the export alike, and refuses a BTC
/// trade against another crypto asset naming the command that keeps both.
#[test]
fn bitvavo_bitcoin_scope_counts_other_assets_and_names_the_scope_command() {
    let run = sync_scoped(Script::new(), "bitcoin");
    let result = run.result.unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(6));
    let skipped = format!(r#"{{"scope": "bitcoin", "skipped": {{"ETH": 1}}, "excluded": {{}}, "next_step": "{}, then sync or import again to keep ETH"}}"#, KEEP.replace('"', "\\\""));
    assert_eq!(dumped(at(&result, "assets")), skipped);
    assert_eq!(at(&result, "swap_pairs"), &Py::from(0));
    let at = "2024-08-01T00:00:00Z";
    for row in [
        item("eth", "sell", at, &[("sent", "BTC", "0.1"), ("received", "ETH", "2"), ("price", "ETH", "20")]),
        item("convert", "convert_buy", at, &[("sent", "ETH", "2"), ("received", "BTC", "0.1")]),
        item("altfee", "buy", at, &[("sent", "EUR", "100"), ("received", "BTC", "0.001"), ("fees", "ETH", "0.01"), ("price", "EUR", "100000")]),
    ] {
        let error = sync_scoped(with_item(row), "bitcoin").result.unwrap_err();
        assert!(error.hint.as_deref().unwrap().contains(KEEP), "{error:?}");
    }
}

/// `crypto` scope keeps every crypto asset exactly: fiat trades priced,
/// crypto-quoted trades and Converts as paired swap legs, a third-asset fee
/// as a fee-only row, movements and rewards in their own asset.
#[test]
fn bitvavo_crypto_scope_maps_trades_swaps_fees_and_movements_of_every_asset() {
    let at = |day: u32| format!("2024-09-{day:02}T00:00:00Z");
    let mut script = Script::new();
    script.history = vec![
        item("dep-eur", "deposit", &at(1), &[("received", "EUR", "1000")]),
        item("buy-eth", "buy", &at(2), &[("sent", "EUR", "100"), ("received", "ETH", "0.04"), ("fees", "EUR", "0.25"), ("price", "EUR", "2500")]),
        item("btc-eth", "buy", &at(3), &[("sent", "BTC", "0.05"), ("received", "ETH", "1"), ("price", "BTC", "0.05")]),
        item("conv", "convert_sell", &at(4), &[("sent", "ETH", "0.5"), ("received", "SOL", "10"), ("fees", "BTC", "0.0001")]),
        item("wd-eth", "withdrawal", &at(5), &[("sent", "ETH", "0.2"), ("fees", "ETH", "0.001")]),
        item("stake-sol", "staking", &at(6), &[("received", "SOL", "0.5")]),
    ];
    let script_again = script.clone();
    let run = sync_scoped(script, "crypto");
    let result = run.result.unwrap();
    assert_eq!(dumped(at_key(&result, "assets")), r#"{"scope": "crypto", "skipped": {}, "excluded": {}}"#);
    assert_eq!(at_key(&result, "swap_pairs"), &Py::from(2));
    assert_eq!(dumped(at_key(&result, "skipped")), r#"{"fiat_only_transactions": 1, "non_btc_transactions": 0}"#);
    assert_eq!(stored(&run.conn), [
        // The API's EUR amount is before the fee: the cost adds it.
        row("buy-eth", "ETH", "inbound", "buy", 4_000_000_000, 0, Some("100.25")),
        row("btc-eth:in", "ETH", "inbound", "swap", 100_000_000_000, 0, None),
        row("btc-eth:out", "BTC", "outbound", "swap", 5_000_000_000, 0, None),
        // A BTC fee on an ETH-SOL Convert is a fee-only BTC spend.
        row("conv:fee", "BTC", "outbound", "fee", 0, 10_000_000, None),
        row("conv:in", "SOL", "inbound", "swap", 1_000_000_000_000, 0, None),
        row("conv:out", "ETH", "outbound", "swap", 50_000_000_000, 0, None),
        row("wd-eth", "ETH", "outbound", "withdrawal", 20_000_000_000, 100_000_000, None),
        row("stake-sol", "SOL", "inbound", "staking", 50_000_000_000, 0, None),
    ]);
    // A re-sync adds nothing and pairs nothing again.
    let again = sync_into(&run.conn, &serve_script(script_again));
    assert_eq!(at_key(&again, "imported"), &Py::from(0));
    assert_eq!(at_key(&again, "swap_pairs"), &Py::from(0));
}
fn at_key<'a>(value: &'a Py, key: &str) -> &'a Py {
    at(value, key)
}

/// What `crypto` scope cannot book exactly still refuses the sync by name.
#[test]
fn bitvavo_crypto_scope_refuses_what_it_cannot_place() {
    let at = "2024-08-01T00:00:00Z";
    for (row, message, hint) in [
        (item("feeswap", "buy", at, &[("sent", "BTC", "0.05"), ("received", "ETH", "1"), ("fees", "BTC", "0.0001"), ("price", "BTC", "0.05")]),
            "(type 'buy') charges a BTC fee Kassiber cannot place on a swap", "this wallet cannot be synced while it holds this trade. Import Bitvavo's transaction-history CSV export"),
        (item("cash", "convert_buy", at, &[("sent", "EUR", "100"), ("received", "ETH", "0.04")]),
            "(type 'convert_buy') is a Convert without two crypto assets", "generic ledger import"),
        (item("odd", "deposit", at, &[("received", "ETH.S", "1")]), "(type 'deposit') names an asset 'ETH.S' that is not a ticker", "generic ledger import"),
        (item("basefee", "buy", at, &[("sent", "EUR", "100"), ("received", "ETH", "0.04"), ("fees", "ETH", "0.0001"), ("price", "EUR", "2500")]),
            "(type 'buy') charges its fee in ETH instead of EUR; Kassiber cannot place it exactly", "generic ledger import"),
    ] {
        let id = row["transactionId"].as_str().unwrap().to_owned();
        let run = sync_scoped(with_item(row), "crypto");
        let error = run.result.unwrap_err();
        assert_eq!(error.message, format!("Bitvavo transaction '{id}' {message}"));
        assert!(error.hint.as_deref().unwrap().contains(hint), "{error:?}");
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
    let fine = item("fine", "deposit", at, &[("received", "ETH", "0.000000000001")]);
    let error = sync_scoped(with_item(fine), "crypto").result.unwrap_err();
    assert!(error.message.contains("has more than 11 decimal places"), "{}", error.message);
}

