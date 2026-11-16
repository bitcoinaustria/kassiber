//! Bitcoin.de's Trading API v4 over a loopback fake built from the
//! documented response shapes (docs/reference/bitcoinde.md; the archived v4
//! documentation: showPermissions, showAccountLedger, showMyTrades and the
//! global `errors`, `credits` and `page` values). One test per rule.
use super::*;
use serde_json::Value as J;
use std::sync::Mutex;

const TX_IN: &str = "a1545e842dd311d998cc4797be7502f573e3d1a6cb5c6cf0e8713e64e28be63a";
const TX_OUT: &str = "5D2F0C8B1E4A3F6E7D9C0B1A2F3E4D5C6B7A8F9E0D1C2B3A4F5E6D7C8B9A0F1E";
const AT: &str = "2024-02-01T10:00:00+01:00";

/// One statement booking as `showAccountLedger` documents it.
fn booking(date: &str, kind: &str, reference: &str, cashflow: &str, balance: &str) -> J {
    json!({"date": date, "type": kind, "reference": reference, "cashflow": cashflow, "balance": balance})
}
/// A buy or sell booking with the statement's own trade details.
fn traded(date: &str, kind: &str, id: &str, pair: &str, cashflow: &str, balance: &str) -> J {
    let mut row = booking(date, kind, id, cashflow, balance);
    row["trade"] = json!({"trade_id": id, "trading_pair": pair, "price": "40000"});
    row
}
/// One trade as `showMyTrades` documents it, with partner details that must
/// stay out of the book.
fn trade(id: &str, kind: &str, before: &str, after: &str, volume: &str, paid: &str) -> J {
    json!({
        "trade_id": id, "is_external_wallet_trade": false, "trading_pair": "btceur", "type": kind,
        "amount_currency_to_trade": before, "price": "40000", "volume_currency_to_pay": volume,
        "amount_currency_to_trade_after_fee": after, "volume_currency_to_pay_after_fee": paid,
        "fee_currency_to_pay": "2.00", "fee_currency_to_trade": "0.0001", "state": 1,
        "my_rating_for_trading_partner": "positive",
        "trading_partner_information": {"username": "synthetic-partner", "bank_name": "Synthetic Bank", "bic": "SYNTDEFFXXX"},
        "payment_method": 1, "created_at": "2024-01-02T09:58:00+01:00", "successfully_finished_at": "2024-01-02T10:00:00+01:00"
    })
}

/// Every booking kind Kassiber maps, newest first as the API lists them.
fn statement() -> Vec<J> {
    vec![
        booking("2024-01-07T00:00:00+01:00", "affiliate", "AFF-1", "0.00002000", "0.00888000"),
        traded("2024-01-06T00:00:00+01:00", "sell", "TRADE2", "btceur", "-0.00200000", "0.00886000"),
        booking("2024-01-05T12:30:45+01:00", "outgoing_fee_voluntary", TX_OUT, "-0.00010000", "0.01086000"),
        booking("2024-01-05T12:30:45+01:00", "payout", TX_OUT, "-0.00400000", "0.01096000"),
        booking("2024-01-04T00:00:00+01:00", "kickback", "KB-1", "0.00001000", "0.01496000"),
        booking("2024-01-03T00:00:00+01:00", "inpayment", TX_IN, "0.00500000", "0.01495000"),
        traded("2024-01-02T10:00:00+01:00", "buy", "TRADE1", "btceur", "0.00990000", "0.00995000"),
        booking("2023-12-01T09:00:00+01:00", "welcome_btc", "", "0.00005000", "0.00005000"),
    ]
}
fn trades() -> Vec<J> {
    vec![
        // Booked today: the statement lists it tomorrow.
        trade("TRADE3", "buy", "0.001", "0.00099", "40.00", "39.80"),
        trade("TRADE2", "sell", "0.002", "0.00198", "84.00", "83.58"),
        trade("TRADE1", "buy", "0.01", "0.0099", "400.00", "398.00"),
    ]
}

#[derive(Clone)]
struct Script {
    permissions: J,
    ledger: Vec<J>,
    trades: Vec<J>,
    ledger_page: usize,
    credits: i64,
    /// Replies served instead of the scripted one, by request ordinal.
    faults: Vec<(usize, u16, String)>,
}
impl Script {
    fn new() -> Self {
        Self {
            permissions: json!(["showPermissions", "showAccountLedger", "showMyTrades", "showAccountInfo"]),
            ledger: statement(), trades: trades(), ledger_page: 3, credits: 20, faults: vec![],
        }
    }
    fn reply(&self, ordinal: usize, target: &str) -> (u16, String) {
        if let Some((_, status, body)) = self.faults.iter().find(|(at, _, _)| *at == ordinal) {
            return (*status, body.clone());
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let page: usize = query.split('&').find_map(|p| p.strip_prefix("page=")).map_or(1, |p| p.parse().unwrap());
        let paged = |rows: &[J], size: usize, field: &str| {
            let (last, start) = (rows.len().div_ceil(size).max(1), (page - 1) * size);
            let end = (start + size).min(rows.len());
            json!({field: rows[start.min(end)..end], "page": {"current": page, "last": last}, "errors": [], "credits": self.credits}).to_string()
        };
        match path {
            "/v4/permissions" => (200, json!({"permissions": self.permissions, "errors": [], "credits": self.credits}).to_string()),
            "/v4/btc/account/ledger" => (200, paged(&self.ledger, self.ledger_page, "account_ledger")),
            "/v4/btceur/trades" => (200, paged(&self.trades, 2, "trades")),
            _ => (404, r#"{"errors":[{"message":"Invalid route","code":7}],"credits":5}"#.into()),
        }
    }
}
fn statement_of(ledger: Vec<J>) -> Script {
    Script { ledger, trades: vec![], ledger_page: 50, ..Script::new() }
}

fn serve_script(script: Script) -> loopback::Fake {
    let script = Mutex::new(script);
    serve("bitcoinde", move |ordinal, _, target| script.lock().unwrap().reply(ordinal, target))
}
fn targets(fake: &loopback::Fake) -> Vec<String> {
    fake.seen.lock().unwrap().iter().map(|r| r.target.clone()).collect()
}
fn sync(script: Script) -> (loopback::Fake, loopback::Run) {
    let fake = serve_script(script);
    let run = run(&backend("bitcoinde", &fake.url), "bitcoinde", "", true, online());
    (fake, run)
}
fn refusal(script: Script) -> AppError {
    sync(script).1.result.unwrap_err()
}
fn eur_book() -> Connection {
    let conn = crate::file_import::tests::fixture();
    conn.execute_batch("UPDATE profiles SET fiat_currency='EUR';
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','BTC','fixed');").unwrap();
    conn
}
/// One sync of `script` into `conn`'s provider wallet.
fn sync_into(conn: &Connection, script: Script) -> Result<Py, AppError> {
    let fake = serve_script(script);
    let gate = Gate::new(|| kassiber_net::Offline::Online, kassiber_net::Ledger::default());
    let mut consent = |route: &Route| Some(Consent::explicit_trigger(route.request()));
    execute(conn, "p", "", &backend("bitcoinde", &fake.url), "bitcoinde", &gate, "test", &mut consent, None)
}
fn json_text(value: &Py) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

type Row = (String, String, i64, i64, Option<String>, String);
fn rows(conn: &Connection) -> Vec<Row> {
    let mut statement = conn.prepare("SELECT external_id, kind, amount, fee, fiat_value_exact, occurred_at FROM transactions ORDER BY occurred_at").unwrap();
    statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).unwrap().collect::<Result<_, _>>().unwrap()
}
/// The rows the fixture statement stores, oldest first.
fn expected_rows() -> Vec<Row> {
    let row = |id: &str, kind: &str, amount, fee, value: Option<&str>, at: &str| (id.into(), kind.into(), amount, fee, value.map(str::to_owned), at.into());
    vec![
        row("bitcoinde:20231201T080000Z:0:5000000", "income", 5_000_000, 0, None, "2023-12-01T08:00:00Z"),
        // A buyer pays 400 EUR minus half the 1% fee and receives the BTC minus the whole fee.
        row("bitcoinde:TRADE1", "buy", 990_000_000, 0, Some("398.00"), "2024-01-02T09:00:00Z"),
        row("bitcoinde:20240102T230000Z:995000000:1495000000", "deposit", 500_000_000, 0, None, "2024-01-02T23:00:00Z"),
        row("bitcoinde:20240103T230000Z:1495000000:1496000000", "income", 1_000_000, 0, None, "2024-01-03T23:00:00Z"),
        // The payout carries its network fee; its txid travels in raw_json.
        row("bitcoinde:20240105T113045Z:1496000000:1096000000", "withdrawal", 400_000_000, 10_000_000, None, "2024-01-05T11:30:45Z"),
        // A seller gives the agreed BTC and receives 84 EUR minus half the fee.
        row("bitcoinde:TRADE2", "sell", 200_000_000, 0, Some("83.58"), "2024-01-05T23:00:00Z"),
        row("bitcoinde:20240106T230000Z:886000000:888000000", "income", 2_000_000, 0, None, "2024-01-06T23:00:00Z"),
    ]
}

#[test]
fn bitcoinde_reads_only_and_maps_every_btc_booking() {
    let (fake, run) = sync(Script::new());
    let result = run.result.unwrap();
    assert_eq!((at(&result, "imported"), at(&result, "fetched")), (&Py::from(7), &Py::from(7)), "{result:?}");
    assert_eq!(at(&result, "input_format"), &s("bitcoinde_api"));
    assert_eq!(json_text(at(&result, "skipped")), r#"{"registration_rows": 0, "trades_not_yet_in_statement": 1}"#);
    assert_eq!(json_text(at(&result, "reported_balance")), r#"{"asset": "BTC", "amount_msat": 888000000, "as_of": "2024-01-06T23:00:00Z", "scope": "account_statement_balance_after_last_booking"}"#);
    // Only signed reads with rising nonces, in this order.
    assert!(fake.seen.lock().unwrap().iter().all(|r| r.signed));
    let since = "datetime_start=2011-01-01T00%3A00%3A00%2B01%3A00";
    assert_eq!(targets(&fake), [
        "/v4/permissions".to_owned(),
        format!("/v4/btc/account/ledger?{since}&page=1"),
        format!("/v4/btc/account/ledger?{since}&page=2"),
        format!("/v4/btc/account/ledger?{since}&page=3"),
        "/v4/btceur/trades?state=1&page=1".to_owned(),
        "/v4/btceur/trades?state=1&page=2".to_owned(),
    ]);
    assert_eq!(rows(&run.conn), expected_rows());
    let wallet: (String, String) = run.conn.query_row("SELECT label, config_json FROM wallets WHERE kind='bitcoinde'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(wallet, ("Bitcoin.de".to_owned(), r#"{"backend": "fixture"}"#.to_owned()));
    // The evidence keeps the booking and the trade, never the partner.
    let raw: String = run.conn.query_row("SELECT raw_json FROM transactions WHERE external_id='bitcoinde:TRADE1'", [], |r| r.get(0)).unwrap();
    assert!(raw.contains(r#""ledger": {"#) && raw.contains(r#""volume_currency_to_pay_after_fee": "398.00""#), "{raw}");
    for private in ["synthetic-partner", "Synthetic Bank", "SYNTDEFFXXX", "trading_partner", "my_rating"] {
        assert!(!raw.contains(private), "{raw}");
    }
    // The run is an import batch whose positions follow the running balance.
    let positions = "SELECT group_concat(t.external_id, ' ') FROM (SELECT t.external_id FROM import_batch_transactions x JOIN transactions t ON t.id=x.transaction_id JOIN import_batches b ON b.id=x.batch_id WHERE b.source_format='bitcoinde_api' ORDER BY x.source_position) t";
    let order: String = run.conn.query_row(positions, [], |r| r.get(0)).unwrap();
    assert_eq!(order, expected_rows().into_iter().map(|r| r.0).collect::<Vec<_>>().join(" "));
}

#[test]
fn bitcoinde_refuses_keys_that_can_write() {
    for write in ["createOrder", "executeTrade", "createWithdrawal", "requestDepositAddress", "someNewMethod"] {
        let script = Script { permissions: json!(["showPermissions", "showAccountLedger", "showMyTrades", write]), ..Script::new() };
        let (fake, run) = sync(script);
        let error = run.result.unwrap_err();
        assert_eq!(error.code, "auth_error", "{write}");
        assert!(error.message.contains("can do more than read"), "{}", error.message);
        assert_eq!(json_text(error.details.as_ref().unwrap()), format!(r#"{{"permissions": ["{write}"]}}"#));
        // Nothing but the permission check was read, and nothing was imported.
        assert_eq!(targets(&fake), ["/v4/permissions"]);
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
    let error = refusal(Script { permissions: json!(["showPermissions", "showAccountInfo"]), ..Script::new() });
    assert_eq!(json_text(error.details.as_ref().unwrap()), r#"{"missing_permissions": ["showAccountLedger", "showMyTrades"]}"#);
    // Every documented read may be granted.
    let reads = "listAddressPool showAccountInfo showAccountLedger showDeposit showDeposits showMyOrderDetails showMyOrders showMyTradeDetails showMyTrades showOrderDetails showOrderbook showOrderbookCompact showPermissions showPublicTradeHistory showRates showWithdrawal showWithdrawalMinNetworkFee showWithdrawals";
    assert!(sync(Script { permissions: json!(reads.split(' ').collect::<Vec<_>>()), ..Script::new() }).1.result.is_ok());
    for (permissions, message) in [(json!("all"), "Bitcoin.de permissions were not a JSON array"), (json!([1]), "Bitcoin.de listed a permission that is not text")] {
        assert_eq!(refusal(Script { permissions, ..Script::new() }).message, message);
    }
}

/// Errors keep the status and a documented code (else "unknown"), never
/// Bitcoin.de's text; nothing is imported.
#[test]
fn bitcoinde_errors_keep_the_status_and_a_documented_code() {
    let cases = [
        (403, "3", "auth_error", "Bitcoin.de refused the API key (HTTP 403)", "3"),
        (403, "14", "auth_error", "Bitcoin.de refused a method the sync needs (HTTP 403)", "14"),
        (403, "4", "auth_error", "Bitcoin.de refused the request nonce (HTTP 403)", "4"),
        (422, "13", "upstream_error", "Bitcoin.de Trading API is unavailable (HTTP 422)", "13"),
        (422, "31", "protocol_error", "Bitcoin.de refused the request (HTTP 422)", "31"),
        (401, "999", "auth_error", "Bitcoin.de refused the API key (HTTP 401)", "\"unknown\""),
        (401, "\"3\"", "auth_error", "Bitcoin.de refused the API key (HTTP 401)", "\"unknown\""),
        (200, "26", "protocol_error", "Bitcoin.de refused the request (HTTP 200)", "\"unknown\""),
    ];
    for (status, code, kind, message, shown) in cases {
        let body = format!(r#"{{"errors":[{{"message":"bad {}","code":{code},"field":"X-API-KEY"}}],"credits":3}}"#, loopback::KEY);
        let (_, run) = sync(Script { faults: vec![(1, status, body)], ..Script::new() });
        let error = run.result.unwrap_err();
        assert_eq!((error.code.as_str(), error.message.as_str(), error.retryable), (kind, message, kind == "upstream_error"), "{code}");
        assert_eq!(json_text(error.details.as_ref().unwrap()), format!(r#"{{"http_status": {status}, "bitcoinde_error_code": {shown}}}"#));
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
    // Another status keeps the shared transport's withheld body prefix.
    let body = format!("{}{}", "x".repeat(195), loopback::SECRET);
    let error = refusal(Script { faults: vec![(1, 500, body)], ..Script::new() });
    assert!(error.message.ends_with("x<reda") && !error.message.contains("synth"), "{}", error.message);
}

/// A throttled read is retried by the transport with a fresh nonce (the fake
/// refuses one that does not rise); exhausted retries import nothing.
#[test]
fn bitcoinde_throttling_is_retried_with_a_fresh_nonce() {
    let (fake, run) = sync(Script { faults: vec![(1, 429, "{}".into())], ..Script::new() });
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(7));
    let seen = fake.seen.lock().unwrap().clone();
    let targets = targets(&fake);
    assert!(seen.iter().all(|r| r.signed) && targets[1] == targets[2]);
    let (_, run) = sync(Script { faults: (1..4).map(|n| (n, 429, "{}".to_owned())).collect(), ..Script::new() });
    let error = run.result.unwrap_err();
    assert_eq!((error.code.as_str(), error.retryable), ("rate_limited", true));
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

#[test]
fn bitcoinde_waits_for_credits_and_refuses_a_long_deficit() {
    // Two credits before each three-credit read: one idle second refills.
    let started = std::time::Instant::now();
    let (fake, run) = sync(Script { credits: 2, ledger_page: 8, trades: trades()[1..].to_vec(), ..Script::new() });
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(7));
    assert_eq!(targets(&fake).len(), 3);
    assert!(started.elapsed() >= Duration::from_secs(2), "{:?}", started.elapsed());
    let (fake, run) = sync(Script { credits: -200, ..Script::new() });
    let error = run.result.unwrap_err();
    assert_eq!((error.code.as_str(), error.retryable), ("rate_limited", true));
    assert_eq!(targets(&fake), ["/v4/permissions"]);
}

#[test]
fn bitcoinde_malformed_pages_fail_closed() {
    for (body, message) in [
        ("not json", "Bitcoin.de response was not JSON"),
        ("[]", "Bitcoin.de response was not a JSON object"),
        (r#"{"account_ledger":{},"errors":[]}"#, "Bitcoin.de response has no account_ledger list"),
        (r#"{"account_ledger":[],"errors":[]}"#, "Bitcoin.de page did not say which page it is"),
        (r#"{"account_ledger":[],"page":{"current":"1","last":1}}"#, "Bitcoin.de page did not say which page it is"),
        (r#"{"account_ledger":[{"type":"buy"}],"page":{"current":2,"last":3}}"#, "Bitcoin.de returned a different page than requested"),
    ] {
        let error = refusal(Script { faults: vec![(1, 200, body.to_owned())], ..Script::new() });
        assert_eq!((error.code.as_str(), error.message.as_str()), ("protocol_error", message), "{body}");
    }
    // An empty statement may say it has no pages; it reports no balance.
    let empty = r#"{"account_ledger":[],"page":{"current":0,"last":0},"errors":[],"credits":9}"#;
    let result = sync(Script { faults: vec![(1, 200, empty.into())], trades: vec![], ..Script::new() }).1.result.unwrap();
    assert_eq!((at(&result, "imported"), get(&result, "reported_balance")), (&Py::from(0), None));
    // A trade repeated with other contents is refused.
    let mut script = Script::new();
    let mut changed = script.trades[1].clone();
    changed["volume_currency_to_pay_after_fee"] = json!("1.00");
    script.trades.push(changed);
    assert!(refusal(script).message.contains("trade 'TRADE2' twice with different contents"));
}

/// Each booking Kassiber cannot map exactly refuses the whole sync, naming it.
#[test]
fn bitcoinde_refuses_unmappable_bookings() {
    type Change = fn(&mut Script);
    let cases: Vec<(&str, Change, &str)> = vec![
        // Rounding to the 32-digit context would make this 0.1.
        ("finer than msat", |s| s.ledger[4]["cashflow"] = json!("0.100000000000000000000000000000001"), "has a BTC amount finer than a millisatoshi"),
        ("missing booking", |s| drop(s.ledger.remove(4)), "has a balance that does not follow the booking before it"),
        ("repeated booking", |s| { let row = s.ledger[0].clone(); s.ledger.insert(3, row) }, "does not decide the order"),
        ("unknown type", |s| s.ledger[4]["type"] = json!("staking"), "(type 'staking', reference 'KB-1') has a type Kassiber does not map"),
        ("moving registration", |s| s.ledger[4]["type"] = json!("registration"), "has a type Kassiber does not map"),
        ("shop", |s| { s.ledger[4]["type"] = json!("buy_goldshop"); s.ledger[4]["cashflow"] = json!("-0.00001000") }, "spends BTC in a Bitcoin.de shop"),
        ("altcoin trade", |s| s.ledger[1] = traded("2024-01-06T00:00:00+01:00", "sell", "DASH1", "dashbtc", "-0.00200000", "0.00886000"), "is a trade on dashbtc that is not in the BTC/EUR trade list"),
        ("amount differs", |s| s.ledger[6]["cashflow"] = json!("0.01000000"), "moves a different BTC amount than its trade details"),
        ("type differs", |s| s.trades[2]["type"] = json!("sell"), "is a buy but its trade is a sell"),
        ("fee without payout", |s| s.ledger[2]["reference"] = json!("other"), "is a network fee without a payout in the same transaction"),
        ("wrong way", |s| s.ledger[5]["cashflow"] = json!("-0.00500000"), "(type 'inpayment', reference 'a1545e842dd311d998cc4797be7502f573e3d1a6cb5c6cf0e8713e64e28be63a') moves BTC the other way"),
        ("zero", |s| s.ledger[4]["cashflow"] = json!("0"), "moves no BTC"),
        ("no reference", |s| s.ledger[5]["reference"] = json!(""), "has no reference"),
        ("no offset", |s| s.ledger[4]["date"] = json!("2024-01-04T00:00:00"), "has a date without a time zone"),
        ("exponent", |s| s.ledger[4]["cashflow"] = json!("1E-5"), "has a cashflow that is not a plain decimal"),
        ("other pair", |s| s.trades[0]["trading_pair"] = json!("etheur"), "has trading pair 'etheur' in the BTC/EUR trade list"),
        ("external wallet", |s| s.trades[0]["is_external_wallet_trade"] = json!(true), "has an external-wallet settlement"),
        ("repeated trade", |s| s.ledger[0] = traded("2024-01-07T00:00:00+01:00", "buy", "TRADE1", "btceur", "0.00990000", "0.01878000"), "repeats a trade id"),
    ];
    for (name, change, message) in cases {
        let mut script = Script::new();
        change(&mut script);
        let (_, run) = sync(script);
        let error = run.result.unwrap_err();
        assert_eq!(error.code, "validation", "{name}: {}", error.message);
        assert!(error.message.contains(message), "{name}: {}", error.message);
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0, "{name}");
    }
}

/// Bookings without an id are named by their second and balance step; one
/// second's bookings must chain from the balance before it in one order.
#[test]
fn bitcoinde_the_running_balance_orders_and_names_bookings() {
    let ids = |conn: &Connection| -> Vec<String> {
        let order = format!("SELECT t.external_id FROM transactions t ORDER BY {}", kassiber_store::transaction_order::transaction_order_sql("t", false));
        conn.prepare(&order).unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    };
    // Two payouts in one transaction with one fee: each keeps its own row,
    // the fee goes to the payout next to it, the txid is evidence.
    let mut script = statement_of(statement()[2..].to_vec());
    script.ledger[1]["balance"] = json!("0.00996000");
    script.ledger.insert(2, booking("2024-01-05T12:30:45+01:00", "payout", TX_OUT, "-0.00100000", "0.01396000"));
    script.ledger[0]["balance"] = json!("0.00986000");
    script.trades = trades();
    let (_, run) = sync(script);
    assert_eq!(at(at(&run.result.unwrap(), "reported_balance"), "amount_msat"), &Py::from(986_000_000));
    let payouts: Vec<(String, i64, String)> = run.conn.prepare("SELECT external_id, fee, json_extract(raw_json, '$.txid') FROM transactions WHERE kind='withdrawal' ORDER BY amount").unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<Result<_, _>>().unwrap();
    let txid = TX_OUT.to_ascii_lowercase();
    assert_eq!(payouts, [
        ("bitcoinde:20240105T113045Z:1496000000:1396000000".to_owned(), 0, txid.clone()),
        ("bitcoinde:20240105T113045Z:1396000000:996000000".to_owned(), 10_000_000, txid),
    ]);
    // Same-second buys keep the order their balances prove (newest listed first).
    let mut script = statement_of(vec![
        traded(AT, "buy", "T-2", "btceur", "0.20000000", "0.30000000"),
        traded(AT, "buy", "T-1", "btceur", "0.10000000", "0.10000000"),
    ]);
    script.trades = vec![trade("T-2", "buy", "0.2", "0.2", "10000.00", "10000.00"), trade("T-1", "buy", "0.1", "0.1", "4000.00", "4000.00")];
    let (_, run) = sync(script);
    run.result.unwrap();
    assert_eq!(ids(&run.conn), ["bitcoinde:T-1", "bitcoinde:T-2"]);
    // Two orders fit the balances (A,S,B or B,S,A give different lots): refused.
    let mut script = statement_of(vec![
        traded(AT, "buy", "T-B", "btceur", "0.10000000", "0.10000000"),
        traded(AT, "sell", "T-S", "btceur", "-0.10000000", "0.00000000"),
        booking(AT, "inpayment", TX_IN, "0.10000000", "0.10000000"),
    ]);
    script.trades = vec![trade("T-B", "buy", "0.1", "0.1", "6000.00", "6000.00"), trade("T-S", "sell", "0.1", "0.1", "5000.00", "5000.00")];
    let error = refusal(script);
    assert_eq!(error.message, "Bitcoin.de's statement does not decide the order of the bookings at 2024-02-01T09:00:00Z: their balances allow more than one order");
    // A statement that does not start from zero lacks its history.
    let error = refusal(statement_of(vec![booking(AT, "inpayment", TX_IN, "0.10000000", "0.30000000")]));
    assert!(error.message.ends_with("starts from a balance of 20000000000 msat, not zero: the history before it is missing"), "{}", error.message);
}

/// JSON numbers are read from their written digits, never through a float.
#[test]
fn bitcoinde_numeric_amounts_keep_every_digit() {
    let numeric = |value: &str| {
        let mut row = booking(AT, "inpayment", TX_IN, "0", "0");
        row["cashflow"] = serde_json::from_str(value).unwrap();
        row["balance"] = serde_json::from_str(value).unwrap();
        statement_of(vec![row])
    };
    let (_, run) = sync(numeric("9000000.00000000001"));
    let result = run.result.unwrap();
    assert_eq!(count(&run.conn, "SELECT amount FROM transactions"), 900_000_000_000_000_001);
    assert_eq!(at(at(&result, "reported_balance"), "amount_msat"), &Py::from(900_000_000_000_000_001i64));
    assert!(refusal(numeric("0.100000000000000000000000000000001")).message.contains("finer than a millisatoshi"));
}

/// Imports only append: stored Bitcoin.de rows must come back unchanged and
/// new ones must follow the newest stored second.
#[test]
fn bitcoinde_imports_only_append() {
    let deposit = |at: &str, reference: &str, cashflow: &str, balance: &str| booking(at, "inpayment", reference, cashflow, balance);
    let first = deposit(AT, TX_IN, "0.10000000", "0.10000000");
    let differs = |at: &str| format!("Bitcoin.de's statement differs from what Kassiber imported earlier (first difference at {at})");
    let cases: Vec<(&str, Vec<J>, String)> = vec![
        ("corrected amount", vec![deposit("2024-02-02T10:00:00+01:00", TX_OUT, "0.20000000", "0.35000000"), deposit(AT, TX_IN, "0.15000000", "0.15000000")], differs("2024-02-01T09:00:00Z")),
        ("corrected time", vec![deposit("2024-02-01T10:05:00+01:00", TX_IN, "0.10000000", "0.10000000")], differs("2024-02-01T09:00:00Z")),
        ("changed kind", vec![booking(AT, "affiliate", TX_IN, "0.10000000", "0.10000000")], differs("2024-02-01T09:00:00Z")),
        ("stored second gains a booking", vec![deposit(AT, TX_OUT, "0.20000000", "0.30000000"), first.clone()], differs("2024-02-01T09:00:00Z")),
    ];
    for (name, second, message) in cases {
        let conn = eur_book();
        sync_into(&conn, statement_of(vec![first.clone()])).unwrap();
        let error = sync_into(&conn, statement_of(second)).unwrap_err();
        assert_eq!((error.code.as_str(), error.message.as_str()), ("validation", message.as_str()), "{name}");
        assert!(error.hint.as_deref().unwrap().contains("imports rollback"));
        assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 1, "{name}");
    }
    let conn = eur_book();
    let payout = booking("2024-02-02T10:00:00+01:00", "payout", TX_OUT, "-0.02000000", "0.08000000");
    sync_into(&conn, statement_of(vec![payout.clone(), first.clone()])).unwrap();
    let fee = booking("2024-02-02T10:00:00+01:00", "outgoing_fee_voluntary", TX_OUT, "-0.00100000", "0.07900000");
    assert!(sync_into(&conn, statement_of(vec![fee, payout.clone(), first.clone()])).unwrap_err().message.starts_with("Bitcoin.de's statement differs"));
    // A later second is appended; manual rows naming Bitcoin.de do not bind.
    conn.execute_batch("CREATE TEMP TABLE manual AS SELECT * FROM transactions WHERE kind='deposit';
        UPDATE manual SET id='manual-1', external_id='bitcoinde:manual', fingerprint='manual-fp', amount=7, occurred_at='2030-01-01T00:00:00Z';
        INSERT INTO transactions SELECT * FROM manual;").unwrap();
    let later = deposit("2024-02-03T10:00:00+01:00", TX_IN, "0.50000000", "0.58000000");
    let result = sync_into(&conn, statement_of(vec![later, payout, first])).unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(1));
}

/// A successful page that echoes the request signature (16+ characters)
/// refuses the sync before anything is stored, naming no value.
#[test]
fn bitcoinde_an_echoed_signature_refuses_the_sync() {
    let fake = loopback::serve_seeing("bitcoinde", |ordinal, _, target, headers| {
        let signature = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("X-API-SIGNATURE")).map(|(_, v)| v.clone()).unwrap_or_default();
        let (status, body) = statement_of(vec![booking(AT, "inpayment", TX_IN, "0.10000000", "0.10000000")]).reply(ordinal, target);
        (status, body.replace(TX_IN, &format!("{TX_IN} {signature}")))
    });
    let run = run(&backend("bitcoinde", &fake.url), "bitcoinde", "", true, online());
    let error = run.result.unwrap_err();
    assert_eq!(error.code, "credential_echo", "{}", error.message);
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

/// The sync reads only BTC endpoints and cannot see another coin's
/// bookings, so a wallet whose asset scope is `crypto` is refused before
/// any request.
#[test]
fn bitcoinde_sync_refuses_a_crypto_scope_before_any_request() {
    let fake = serve_script(Script::new());
    let prepare = |conn: &Connection| {
        conn.execute_batch("UPDATE profiles SET fiat_currency='EUR'").unwrap();
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('bd','ws','p','Bitcoin.de','bitcoinde','{\"backend\": \"fixture\", \"asset_scope\": \"crypto\"}','2025-01-01')", []).unwrap();
    };
    let run = loopback::run_prepared(&backend("bitcoinde", &fake.url), "bitcoinde", "", true, online(), &prepare);
    let error = run.result.unwrap_err();
    assert_eq!(error.message, "Bitcoin.de sync reads only BTC, and this wallet's asset_scope is crypto");
    assert!(targets(&fake).is_empty());
}

/// Waiting for credits never holds the process-wide nonce lock, so another
/// Bitcoin.de sync can sign while this one waits.
#[test]
fn bitcoinde_waits_for_credits_without_holding_the_nonce_lock() {
    let watcher: Arc<Mutex<Option<std::thread::JoinHandle<bool>>>> = Arc::default();
    let started = watcher.clone();
    // One credit left before each three-credit read: a two-second wait.
    let script = Mutex::new(Script { credits: 1, ..statement_of(vec![booking(AT, "inpayment", TX_IN, "0.10000000", "0.10000000")]) });
    let fake = serve("bitcoinde", move |ordinal, _, target| {
        if ordinal == 0 {
            // From just after this answer, into the wait before the next read.
            *started.lock().unwrap() = Some(std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(300));
                let until = Instant::now() + Duration::from_millis(1_400);
                while Instant::now() < until {
                    if !matches!(bitcoinde::NONCE.try_lock(), Err(std::sync::TryLockError::WouldBlock)) {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                false
            }));
        }
        script.lock().unwrap().reply(ordinal, target)
    });
    let run = run(&backend("bitcoinde", &fake.url), "bitcoinde", "", true, online());
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(1));
    let free = watcher.lock().unwrap().take().unwrap().join().unwrap();
    assert!(free, "the nonce lock was held through the credit wait");
}

