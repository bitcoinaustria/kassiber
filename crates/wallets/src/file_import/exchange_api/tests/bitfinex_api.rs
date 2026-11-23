//! Bitfinex's REST API v2 over a loopback fake built from the documented
//! shapes (docs/reference/bitfinex.md lists the pages), and the Reports CSV
//! exports of the same events (docs/exchanges/samples/bitfinex). Kept out of
//! rustfmt (tests.rs) so its fixture tables stay one row per line.
use super::*;
use serde_json::Value as J;
use std::sync::Mutex;

const DAY: i64 = 86_400_000;
/// 2024-01-01T00:00:00Z in ms.
const JAN1: i64 = 1_704_067_200_000;
const DEPOSIT_TX: &str = "AA00000000000000000000000000000000000000000000000000000000000001";
const WITHDRAWAL_TX: &str = "bb00000000000000000000000000000000000000000000000000000000000002";

/// A JSON number exactly as written (serde_json keeps arbitrary precision).
fn n(text: &str) -> J {
    serde_json::from_str(text).unwrap()
}
/// `[ID, CURRENCY, WALLET, MTS, _, AMOUNT, BALANCE, _, DESCRIPTION]`.
fn ledger(id: i64, wallet: &str, mts: i64, amount: &str, description: &str) -> J {
    json!([id, "BTC", wallet, mts, null, n(amount), 1.0, null, description])
}
/// `[ID, SYMBOL, MTS, ORDER_ID, EXEC_AMOUNT, EXEC_PRICE, ORDER_TYPE, ORDER_PRICE, MAKER, FEE, FEE_CURRENCY, CID]`.
#[allow(clippy::too_many_arguments)]
fn trade(id: i64, symbol: &str, mts: i64, amount: &str, price: &str, order: &str, fee: &str, fee_currency: &str) -> J {
    json!([id, symbol, mts, id + 1000, n(amount), n(price), order, n(price), 1, n(fee), fee_currency, 7])
}
/// The 22 documented movement fields; unnamed ones are placeholders.
fn movement(id: i64, mts: i64, status: &str, amount: &str, fees: &str, tx: Option<&str>) -> J {
    json!([id.to_string(), "BTC", "BITCOIN", null, null, mts - 600_000, mts, null, null, status, null, null,
        amount, fees, null, null, "bc1qsyntheticdestination", null, null, null, tx, null])
}
fn permissions(write: &[&str]) -> J {
    let scopes = ["account", "history", "orders", "positions", "funding", "settings", "wallets", "withdraw", "ui_withdraw", "bfxpay"];
    J::Array(scopes.iter().map(|scope| json!([scope, 1, i64::from(write.contains(scope))])).collect())
}

/// The events of the sample exports, as the API serves them.
#[derive(Clone)]
struct Script {
    permissions: J,
    ledger: Vec<J>,
    trades: Vec<J>,
    movements: Vec<J>,
    wallets: J,
    /// Replies served instead of the scripted one, by request ordinal.
    faults: Vec<(usize, u16, String)>,
}
impl Script {
    fn new() -> Self {
        Self {
            permissions: permissions(&[]),
            ledger: vec![
                ledger(1, "exchange", JAN1, "0.05", "Deposit (BITCOIN) #301 on wallet exchange"),
                ledger(2, "exchange", JAN1 + DAY, "0.01", "Exchange 0.01 BTC for USD @ 42000.0 on wallet exchange"),
                ledger(3, "exchange", JAN1 + DAY, "-0.00002", "Trading fees for 0.01 BTC (BTCUSD) @ 42000.0 on BFX (0.2%) on wallet exchange"),
                ledger(4, "exchange", JAN1 + 2 * DAY, "-0.005", "Exchange -0.005 BTC for USD @ 40000.0 on wallet exchange"),
                ledger(5, "exchange", JAN1 + 3 * DAY, "-0.02", "Transfer of 0.02 BTC from wallet Exchange to Funding on wallet exchange"),
                ledger(6, "funding", JAN1 + 3 * DAY, "0.02", "Transfer of 0.02 BTC from wallet Exchange to Funding on wallet funding"),
                ledger(7, "funding", JAN1 + 3 * DAY + 3_600_000, "0.0000123", "Margin Funding Payment on wallet funding"),
                ledger(8, "exchange", JAN1 + 4 * DAY, "-0.01", "Bitcoin Withdrawal #302 on wallet exchange"),
                ledger(9, "exchange", JAN1 + 4 * DAY, "-0.0004", "Crypto Withdrawal fee on wallet exchange"),
                ledger(10, "exchange", JAN1 + 5 * DAY, "-0.001", "Bitcoin Withdrawal #303 on wallet exchange"),
            ],
            trades: vec![
                trade(101, "tBTCUSD", JAN1 + DAY, "0.01", "42000.0", "EXCHANGE LIMIT", "-0.00002", "BTC"),
                trade(102, "tBTCUSD", JAN1 + 2 * DAY, "-0.005", "40000.0", "EXCHANGE MARKET", "-0.4", "USD"),
                trade(103, "tETHUSD", JAN1 + 2 * DAY, "1.0", "2300.0", "EXCHANGE LIMIT", "-0.002", "ETH"),
            ],
            movements: vec![
                movement(301, JAN1, "COMPLETED", "0.05", "0", Some(DEPOSIT_TX)),
                movement(302, JAN1 + 4 * DAY, "COMPLETED", "-0.01", "-0.0004", Some(WITHDRAWAL_TX)),
                movement(303, JAN1 + 5 * DAY, "COMPLETED", "-0.001", "0", None),
                movement(304, JAN1 + 6 * DAY, "CANCELED", "-0.5", "-0.0004", None),
                movement(305, JAN1 + 7 * DAY, "PROCESSING", "-0.002", "-0.0004", None),
            ],
            wallets: json!([["exchange", "BTC", n("0.02358"), 0, 0.02, null, null], ["funding", "BTC", n("0.0200123"), 0, 0.01, null, null], ["margin", "BTC", 1.5, 0, 1.5, null, null], ["exchange", "USD", 1234.5, 0, 1234.5, null, null]]),
            faults: vec![],
        }
    }
    /// Entries at or before `end`, newest first, `limit` of them.
    fn page(rows: &[J], time: usize, body: &J) -> (u16, String) {
        let end = body["end"].as_i64().unwrap_or(i64::MAX);
        let mut page: Vec<&J> = rows.iter().filter(|row| row[time].as_i64().unwrap_or(0) <= end).collect();
        page.sort_by_key(|row| std::cmp::Reverse(row[time].as_i64().unwrap_or(0)));
        page.truncate(body["limit"].as_u64().unwrap() as usize);
        (200, json!(page).to_string())
    }
    fn reply(&self, ordinal: usize, target: &str) -> (u16, String) {
        if let Some((_, status, body)) = self.faults.iter().find(|(at, _, _)| *at == ordinal) {
            return (*status, body.clone());
        }
        let (path, body) = target.split_once('?').unwrap();
        let body: J = serde_json::from_str(body).unwrap();
        match path {
            "/v2/auth/r/permissions" => (200, self.permissions.to_string()),
            "/v2/auth/r/ledgers/BTC/hist" => Self::page(&self.ledger, 3, &body),
            "/v2/auth/r/trades/hist" => Self::page(&self.trades, 2, &body),
            "/v2/auth/r/movements/BTC/hist" => Self::page(&self.movements, 6, &body),
            "/v2/auth/r/wallets" => (200, self.wallets.to_string()),
            _ => (500, r#"["error",10020,"unknown endpoint"]"#.into()),
        }
    }
}

/// The fake and the `bfx-nonce` of every request it saw.
struct Served {
    fake: loopback::Fake,
    nonces: Arc<Mutex<Vec<u64>>>,
}
impl Served {
    fn targets(&self) -> Vec<String> {
        self.fake.seen.lock().unwrap().iter().map(|r| r.target.clone()).collect()
    }
}
fn serve_script(script: Script) -> Served {
    let (script, nonces) = (Mutex::new(script), Arc::new(Mutex::new(vec![])));
    let seen = nonces.clone();
    let fake = loopback::serve_seeing("bitfinex", move |ordinal, _, target, headers| {
        let nonce = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("bfx-nonce")).unwrap();
        seen.lock().unwrap().push(nonce.1.parse().unwrap());
        script.lock().unwrap().reply(ordinal, target)
    });
    Served { fake, nonces }
}
fn usd(conn: &Connection) {
    conn.execute_batch("UPDATE profiles SET fiat_currency='USD'").unwrap();
}
fn sync(script: Script) -> (Served, loopback::Run) {
    let served = serve_script(script);
    let run = loopback::run_prepared(&backend("bitfinex", &served.fake.url), "bitfinex", "", true, online(), &usd);
    (served, run)
}
fn refusal(script: Script) -> AppError {
    let (_, run) = sync(script);
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    run.result.unwrap_err()
}
fn sync_into(conn: &Connection, url: &str) -> Py {
    let gate = Gate::new(|| kassiber_net::Offline::Online, kassiber_net::Ledger::default());
    let consent = &mut |route: &Route| Some(Consent::explicit_trigger(route.request()));
    execute(conn, "p", "", &backend("bitfinex", url), "bitfinex", &gate, "test", consent, None).unwrap()
}

type Row = (String, String, i64, i64, Option<String>, Option<String>, String, String);
fn txid(raw: &str) -> String {
    serde_json::from_str::<J>(raw).unwrap()["txid"].as_str().unwrap_or("").to_owned()
}
fn rows(conn: &Connection) -> Vec<Row> {
    let sql = "SELECT external_id, kind, amount, fee, fiat_value_exact, pricing_method, occurred_at, raw_json
        FROM transactions ORDER BY occurred_at, external_id";
    let mut statement = conn.prepare(sql).unwrap();
    let read = |r: &kassiber_store::rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, txid(&r.get::<_, String>(7)?)));
    statement.query_map([], read).unwrap().collect::<Result<_, _>>().unwrap()
}
/// What both sources import (the last field: `raw_json.txid`, lowercased).
fn expected() -> Vec<Row> {
    let row = |id: &str, kind: &str, amount, fee, value: Option<&str>, at: &str, txid: &str| -> Row {
        let method = value.map(|_| "bitfinex_trade".to_owned());
        (format!("bitfinex:{id}"), kind.into(), amount, fee, value.map(Into::into), method, at.into(), txid.into())
    };
    vec![
        row("movement:301", "deposit", 5_000_000_000, 0, None, "2024-01-01T00:00:00Z", &DEPOSIT_TX.to_lowercase()),
        // A BTC fee leaves fewer coins for the same cost.
        row("trade:101", "buy", 998_000_000, 0, Some("420"), "2024-01-02T00:00:00Z", ""),
        // A fiat fee comes off the proceeds.
        row("trade:102", "sell", 500_000_000, 0, Some("199.6"), "2024-01-03T00:00:00Z", ""),
        // Interest credited to the funding wallet.
        row("ledger:7", "lending_interest", 1_230_000, 0, None, "2024-01-04T01:00:00Z", ""),
        // The fee entry proves AMOUNT excludes FEES.
        row("movement:302", "withdrawal", 1_000_000_000, 40_000_000, None, "2024-01-05T00:00:00Z", WITHDRAWAL_TX),
        row("movement:303", "withdrawal", 100_000_000, 0, None, "2024-01-06T00:00:00Z", ""),
    ]
}

const HISTORY: [&str; 5] = [
    r#"/v2/auth/r/permissions?{}"#,
    r#"/v2/auth/r/ledgers/BTC/hist?{"limit":2500}"#,
    r#"/v2/auth/r/trades/hist?{"limit":2500,"sort":-1}"#,
    r#"/v2/auth/r/movements/BTC/hist?{"limit":1000}"#,
    r#"/v2/auth/r/wallets?{}"#,
];

#[test]
fn bitfinex_signature_matches_an_independent_hmac_sha384() {
    // `openssl dgst -sha384 -hmac` over `/api/v2/auth/r/wallets` + nonce + `{}`.
    let expected = "f19c80e7ced913aead3c4a3c7ec643c2bbe6a267f2e46e636b7b6992f75764f929fc014423302b43924e719939a506d8";
    assert_eq!(super::bitfinex::signature(loopback::SECRET, "v2/auth/r/wallets", "1700000000000000", "{}"), expected);
}

#[test]
fn bitfinex_reads_only_and_maps_every_btc_event() {
    let (served, run) = sync(Script::new());
    let result = run.result.unwrap();
    assert_eq!(served.targets(), HISTORY);
    assert!(served.fake.seen.lock().unwrap().iter().all(|r| r.signed));
    let nonces = served.nonces.lock().unwrap().clone();
    assert!(nonces.windows(2).all(|w| w[0] < w[1]), "{nonces:?}");
    assert_eq!(rows(&run.conn), expected());
    let skipped = kassiber_pyjson::dumps(at(&result, "skipped"), &Default::default()).unwrap();
    assert_eq!(skipped, r#"{"non_btc_entries": 1, "canceled_movements": 1, "unfinished_movements": 1, "internal_transfers": 2}"#);
    // The exchange and funding wallets' BTC, the two the history covers.
    let balance = at(&result, "reported_balance");
    assert_eq!((at(balance, "amount_msat"), at(balance, "scope")), (&Py::from(4_359_230_000i64), &s("exchange_and_funding_wallets")));
    // A second sync finds nothing new.
    sync_into(&run.conn, &served.fake.url);
    assert_eq!(rows(&run.conn), expected());
    // Without the caller's consent nothing is sent.
    let quiet = serve_script(Script::new());
    let refused = loopback::run(&backend("bitfinex", &quiet.fake.url), "bitfinex", "", false, online());
    assert!(refused.result.is_err());
    assert!(quiet.targets().is_empty());
}

#[test]
fn bitfinex_refuses_keys_with_write_permissions_before_any_history() {
    let mut script = Script::new();
    script.permissions = permissions(&["withdraw", "orders"]);
    let (served, run) = sync(script);
    let error = run.result.unwrap_err();
    assert_eq!(error.code, "auth_error");
    let listed = kassiber_pyjson::dumps(at(error.details.as_ref().unwrap(), "write_permissions"), &Default::default()).unwrap();
    assert_eq!(listed, r#"["orders", "withdraw"]"#);
    assert_eq!(served.targets(), HISTORY[..1]);
}

/// A writable entry that is not shaped like a Bitfinex scope name could be an
/// echoed key or signature: it is counted, never quoted, not even in part.
#[test]
fn bitfinex_write_permissions_never_quote_an_entry_that_is_not_a_scope_name() {
    let echoed = "Kx9fQ2mZ7bW4tR8yL3vN6cH1sD5gJ0pA2eU7iO9kXqWz";
    let mut script = Script::new();
    script.permissions = json!([["orders", 1, 1], [echoed, 1, 1], ["history", 1, 0]]);
    let error = refusal(script);
    assert_eq!(error.code, "auth_error");
    let details = error.details.as_ref().unwrap();
    let listed = kassiber_pyjson::dumps(at(details, "write_permissions"), &Default::default()).unwrap();
    assert_eq!((listed.as_str(), at(details, "other_write_permissions")), (r#"["orders"]"#, &Py::from(1)));
    let shown = kassiber_pyjson::dumps(&error.envelope(None), &Default::default()).unwrap();
    assert!(!shown.contains(&echoed[..8]), "{shown}");
}

#[test]
fn bitfinex_errors_keep_only_documented_codes() {
    for (status, body, code, documented) in [
        (500, r#"["error",10100,"apikey: invalid synthetic-exchange-key"]"#, "auth_error", Py::from(10100)),
        (500, r#"["error",10114,"nonce: small"]"#, "auth_error", Py::from(10114)),
        (500, r#"["error",99999,"synthetic-exchange-key"]"#, "protocol_error", s("unknown")),
    ] {
        let mut script = Script::new();
        script.faults = vec![(0, status, body.into())];
        let error = refusal(script);
        assert_eq!((error.code.as_str(), at(error.details.as_ref().unwrap(), "bitfinex_code")), (code, &documented));
        let shown = kassiber_pyjson::dumps(&error.envelope(None), &Default::default()).unwrap();
        assert!(!shown.contains("synthetic-exchange-key") && !shown.contains("nonce: small"), "{shown}");
    }
    let mut script = Script::new();
    script.faults = vec![(1, 200, r#"{"error":"ERR_RATE_LIMIT"}"#.into())];
    let error = refusal(script);
    assert_eq!((error.code.as_str(), error.retryable), ("rate_limited", true));
}

#[test]
fn bitfinex_refuses_a_page_that_echoes_a_credential() {
    let mut script = Script::new();
    script.ledger[0][7] = json!(loopback::KEY);
    assert_eq!(refusal(script).code, "credential_echo");
}

#[test]
fn bitfinex_pages_by_end_time_and_rereads_the_boundary() {
    // 2501 trades without BTC, two of them in the last page's millisecond.
    let mut script = Script::new();
    script.trades.extend((0..2501).map(|i| trade(1000 + i, "tETHUSD", JAN1 - 1000 * (i / 2), "1", "2300", "EXCHANGE LIMIT", "0", "USD")));
    let (served, run) = sync(script);
    let result = run.result.unwrap();
    let skipped = at(at(&result, "skipped"), "non_btc_entries");
    assert_eq!(skipped, &Py::from(2502));
    let boundary = format!(r#"/v2/auth/r/trades/hist?{{"end":{},"limit":2500,"sort":-1}}"#, JAN1 - 1000 * 1248);
    assert!(served.targets().contains(&boundary), "{:?}", served.targets());
    // A full page within one millisecond cannot be paged past.
    let mut stuck = Script::new();
    stuck.trades = (0..2500).map(|i| trade(1000 + i, "tETHUSD", JAN1, "1", "2300", "EXCHANGE LIMIT", "0", "USD")).collect();
    let error = refusal(stuck);
    assert!(error.message.contains("full page in one millisecond"), "{}", error.message);
}

#[test]
fn bitfinex_malformed_pages_fail_closed() {
    let entry = |body: &str| format!("[{body}]");
    for body in [
        "not json".to_owned(),
        r#"{"rows":[]}"#.to_owned(),
        entry(r#""row""#),
        entry(r#"[1,"BTC","exchange"]"#),
        entry(r#"[1.5,"BTC","exchange",1704067200000,null,0.05,1,null,"Deposit (BITCOIN) #301 on wallet exchange"]"#),
        entry(r#"[1,"BTC","exchange","soon",null,0.05,1,null,"Deposit (BITCOIN) #301 on wallet exchange"]"#),
        entry(r#"[1,"BTC","exchange",1704067200000,null,"abc",1,null,"Deposit (BITCOIN) #301 on wallet exchange"]"#),
        // The same id twice with different contents.
        r#"[[1,"BTC","exchange",1704067200000,null,0.05,1,null,"a"],[1,"BTC","exchange",1704067200000,null,0.04,1,null,"a"]]"#.to_owned(),
    ] {
        let mut script = Script::new();
        script.faults = vec![(1, 200, body.clone())];
        let code = refusal(script).code;
        assert!(matches!(code.as_str(), "protocol_error" | "validation"), "{body}: {code}");
    }
}

/// Every BTC row Kassiber cannot book exactly refuses the import by name.
#[test]
fn bitfinex_refuses_what_it_cannot_book_by_name() {
    type Change = fn(&mut Script);
    let cases: [(Change, &str); 24] = [
        // A BTC-ledger entry naming another base cannot prove a BTC trade.
        (|s| s.ledger[3][8] = json!("Exchange -0.005 ETH for USD @ 40000.0 on wallet exchange"), "books a trade of ETH, not BTC"),
        (|s| s.ledger.push(ledger(20, "margin", JAN1, "0.1", "Exchange 0.1 BTC for USD @ 1 on wallet margin")), "'ledger entry 20' (type 'Exchange 0.1 BTC for USD @ 1 on wallet margin') is in the 'margin' wallet"),
        (|s| s.ledger.push(ledger(20, "funding", JAN1, "-0.0001", "Margin Funding Charge on wallet funding")), "is in the 'funding' wallet"),
        (|s| s.ledger.push(ledger(20, "exchange", JAN1, "0.0001", "Margin Funding Payment on wallet exchange")), "does not map"),
        (|s| s.ledger.push(ledger(20, "exchange", JAN1, "-0.02", "Transfer of 0.02 BTC from wallet Exchange to Margin on wallet exchange")), "does not map"),
        // A transfer whose other leg is missing, or more than 60 s apart.
        (|s| { s.ledger.remove(5); }, "is a transfer without its other leg"),
        (|s| s.ledger[5][3] = json!(JAN1 + 3 * DAY + 61_000), "is a transfer without its other leg"),
        (|s| s.ledger.push(ledger(20, "exchange", JAN1, "-0.02", "Transfer of 0.02 BTC from wallet Exchange to Funding on wallet exchange")), "is a transfer without its other leg"),
        (|s| s.ledger.push(ledger(20, "exchange", JAN1, "0.001", "Adjustment on wallet exchange")), "does not map"),
        // A ledger-only BTC leg of BTC -> ETH is a trade, never a withdrawal.
        (|s| s.ledger.push(ledger(20, "exchange", JAN1, "-0.075", "Exchange 1.5 ETH for BTC @ 0.05 on wallet exchange")), "books a trade of BTC against ETH"),
        (|s| s.trades.push(trade(110, "tBTCF0:USTF0", JAN1, "0.1", "1", "LIMIT", "0", "USTF0")), "derivatives"),
        (|s| s.trades.push(trade(110, "tETHBTC", JAN1, "1", "0.05", "EXCHANGE LIMIT", "0", "BTC")), "(type 'tETHBTC') trades BTC against ETH"),
        (|s| s.trades.push(trade(110, "tBTCUST", JAN1, "0.1", "40000", "EXCHANGE LIMIT", "0", "UST")), "against UST"),
        (|s| s.trades.push(trade(110, "tBTC", JAN1, "0.1", "40000", "EXCHANGE LIMIT", "0", "USD")), "a pair Kassiber cannot read ('tBTC')"),
        (|s| s.trades[0][6] = json!("LIMIT"), "margin trade"),
        (|s| s.trades[1][10] = json!("LEO"), "fee in LEO"),
        (|s| s.trades[1][9] = n("0.4"), "rebate"),
        (|s| s.movements[0][13] = json!("-0.0001"), "deposit fee"),
        (|s| s.movements[0][2] = json!("LNX"), "not the Bitcoin network"),
        (|s| s.trades.push(trade(104, "tBTCUSD", JAN1 + 2 * DAY + 5000, "-0.005", "40000.0", "EXCHANGE MARKET", "-0.4", "USD")), "fits 2 (104, 102)"),
        (|s| s.ledger.push(ledger(20, "exchange", JAN1 + 2 * DAY, "-0.005", "Exchange -0.005 BTC for USD @ 40000.0 on wallet exchange")), "books what ledger entry 4 books"),
        (|s| { s.ledger.remove(8); }, "has a fee the ledger does not show"),
        (|s| { s.ledger.remove(2); }, "no BTC fee entry"),
        (|s| s.ledger[7][5] = n("-0.02"), "do not add up"),
    ];
    for (change, needle) in cases {
        let mut script = Script::new();
        change(&mut script);
        let error = refusal(script);
        assert_eq!(error.code, "validation");
        assert!(error.message.contains(needle), "{needle}: {}", error.message);
    }
}

#[test]
fn bitfinex_api_numbers_are_read_from_their_exact_text() {
    let mut script = Script::new();
    script.trades[0][4] = n(&format!("0.1{}1", "0".repeat(40)));
    let error = refusal(script);
    assert!(error.message.contains("sub-millisatoshi"), "{}", error.message);
}

#[test]
fn bitfinex_ledger_descriptions_follow_their_grammars() {
    use super::bitfinex::mapping::{describe, Described};
    let d = |t: &str| t.parse::<Decimal>().unwrap();
    let trade = |fee, base: &str, quote: &str, amount, price| Described::Trade(fee, base.into(), quote.into(), d(amount), d(price));
    for (text, wallet, expected) in [
        ("Exchange 0.01 BTC for USD @ 42000.0 on wallet exchange", "exchange", trade(false, "BTC", "USD", "0.01", "42000.0")),
        ("Exchange 1.5 ETH for BTC @ 0.05 on wallet exchange", "exchange", trade(false, "ETH", "BTC", "1.5", "0.05")),
        ("Trading fees for 0.01 BTC (BTCUSD) @ 42000.0 on BFX (0.2%) on wallet exchange", "exchange", trade(true, "BTC", "USD", "0.01", "42000.0")),
        ("Trading fees for 2 ETH (ETH:BTC) @ 0.05 on BFX (0.2%) on wallet exchange", "exchange", trade(true, "ETH", "BTC", "2", "0.05")),
        // The wallet named at the end must be the entry's own.
        ("Exchange 0.01 BTC for USD @ 42000.0 on wallet exchange", "funding", Described::Unknown),
        ("Bitcoin Withdrawal #302 on wallet exchange", "exchange", Described::Movement("302".into())),
        ("Deposit (BITCOIN) #301 on wallet exchange", "exchange", Described::Movement("301".into())),
        ("Canceled withdrawal request #305 on wallet exchange", "exchange", Described::Movement("305".into())),
        ("Crypto Withdrawal fee on wallet exchange", "exchange", Described::WithdrawalFee),
        ("Adjustment after withdrawal on wallet exchange", "exchange", Described::Unknown),
        ("Margin Funding Payment on wallet funding", "funding", Described::FundingPayment),
        ("Transfer of 0.02 BTC from wallet Exchange to Funding on wallet funding", "funding", Described::Transfer(d("0.02"), "exchange".into(), "funding".into())),
        ("Settlement @ 42000 on wallet exchange", "exchange", Described::Unknown),
    ] {
        assert_eq!(describe(text, wallet), expected, "{text}");
    }
}

/// The rows tagged "fee from movements, not cross-checked", with their fee.
fn unchecked(conn: &Connection) -> Vec<(String, i64)> {
    let sql = "SELECT t.external_id, t.fee FROM transactions t JOIN transaction_tags tt ON tt.transaction_id=t.id
        JOIN tags g ON g.id=tt.tag_id WHERE g.code='bitfinex-fee-unchecked' ORDER BY 1";
    let mut statement = conn.prepare(sql).unwrap();
    statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap()
}

/// Only a fee-bearing withdrawal without ledger entries, requested before
/// the ledger's first entry, keeps the fee its movement states, tagged. One
/// the ledger shows, or requested from that entry on, must have its fee
/// proven: Bitfinex books a withdrawal when it is requested, so the newest
/// one completes after the last ledger entry.
#[test]
fn bitfinex_withdrawal_fees_come_from_movements_only_before_the_ledger() {
    let with = |movement_row: J, entry: Option<J>| {
        let mut script = Script::new();
        script.movements.push(movement_row);
        script.ledger.extend(entry);
        script
    };
    // `movement()` starts a withdrawal 600 s before it completes.
    let (_, run) = sync(with(movement(300, JAN1 + 599_000, "COMPLETED", "-0.003", "-0.0005", None), None));
    run.result.unwrap();
    assert_eq!(unchecked(&run.conn), [("bitfinex:movement:300".to_owned(), 50_000_000)]);
    let description: String = run.conn.query_row("SELECT description FROM transactions WHERE external_id='bitfinex:movement:300'", [], |r| r.get(0)).unwrap();
    assert!(description.contains("fee from movements, not cross-checked against the ledger"), "{description}");
    let newest = JAN1 + 10 * DAY;
    for script in [
        with(movement(300, JAN1 + 600_000, "COMPLETED", "-0.003", "-0.0005", None), None),
        // The account's newest withdrawal: its debit (fee folded in) is the
        // ledger's last entry, and it completes 30 minutes later.
        with(movement(320, newest + 1_800_000, "COMPLETED", "-1.0004", "-0.0004", None), Some(ledger(30, "exchange", newest, "-1.0004", "Bitcoin Withdrawal #320 on wallet exchange"))),
        with(movement(321, newest, "COMPLETED", "-0.5", "-0.0004", None), None),
        // Its entry is the ledger's first: the ledger shows it, so it is not
        // older than the ledger even though it was requested earlier.
        with(movement(322, JAN1, "COMPLETED", "-0.003", "-0.0005", None), Some(ledger(31, "exchange", JAN1 - 1000, "-0.003", "Bitcoin Withdrawal #322 on wallet exchange"))),
    ] {
        let error = refusal(script);
        assert!(error.message.contains("(type 'withdrawal') has a fee the ledger does not show"), "{}", error.message);
    }
}

/// The Bitfinex wallet a sync or an import finds by label, in `scope`.
fn scoped(conn: &Connection, scope: &str) {
    let config = json!({"backend": "fixture", "asset_scope": scope}).to_string();
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('bf','ws','p','Bitfinex','bitfinex',?,'2025-01-01')", [config]).unwrap();
}

/// Kassiber reads only BTC from Bitfinex. In `bitcoin` scope the sync and
/// the Reports import name each other asset they left out, by ticker,
/// without offering a scope that would not keep it; in `crypto` scope the
/// sync refuses before any request (it reads only BTC history) and the
/// import refuses the first row in another asset by name.
#[test]
fn bitfinex_counts_other_assets_as_unread_in_either_scope() {
    let dump = |value: &Py| kassiber_pyjson::dumps(value, &Default::default()).unwrap();
    let left_out = |scope: &str| format!(r#"{{"scope": "{scope}", "skipped": {{}}, "excluded": {{}}, "unread": {{"ETH": 1}}, "next_step": "Kassiber reads only BTC from Bitfinex, in either asset scope, and left out entries in ETH (1); record them yourself (generic CSV/JSON import)"}}"#);
    let (_, run) = sync(Script::new());
    let bitcoin = run.result.unwrap();
    assert_eq!(dump(at(&bitcoin, "assets")), left_out("bitcoin"));
    // The `crypto` scope reads and books the same BTC rows; the other
    // assets stay unread, since Kassiber reads only BTC from Bitfinex.
    let served = serve_script(Script::new());
    let prepare = |conn: &Connection| {
        usd(conn);
        scoped(conn, "crypto");
    };
    let run = loopback::run_prepared(&backend("bitfinex", &served.fake.url), "bitfinex", "", true, online(), &prepare);
    let crypto = run.result.unwrap();
    assert_eq!(dump(at(&crypto, "assets")), left_out("crypto"));
    assert_eq!(at(&crypto, "imported"), at(&bitcoin, "imported"));
}

/// A buy at .100 and a withdrawal at .900 of one second are stored in that
/// order: the merge of trades and movements keeps the API's milliseconds.
#[test]
fn bitfinex_keeps_the_millisecond_order_of_one_second() {
    let second = JAN1 + 5 * DAY;
    let mut script = Script::new();
    script.ledger.retain(|entry| entry[0] != json!(10));
    script.ledger.push(ledger(10, "exchange", second + 900, "-0.001", "Bitcoin Withdrawal #303 on wallet exchange"));
    script.ledger.push(ledger(11, "exchange", second + 100, "0.003", "Exchange 0.003 BTC for USD @ 40000.0 on wallet exchange"));
    script.trades.push(trade(104, "tBTCUSD", second + 100, "0.003", "40000.0", "EXCHANGE MARKET", "-0.12", "USD"));
    script.movements.retain(|row| row[0] != json!("303"));
    script.movements.push(movement(303, second + 900, "COMPLETED", "-0.001", "0", None));
    let (_, run) = sync(script);
    let result = run.result.unwrap();
    let Py::List(inserted) = at(&result, "inserted_records") else {
        panic!("inserted records")
    };
    let order: Vec<String> = inserted.iter().map(|r| lossy(&text(at(r, "external_id")))).collect();
    let position = |id: &str| order.iter().position(|o| o == id).unwrap_or_else(|| panic!("{id}: {order:?}"));
    assert!(position("bitfinex:trade:104") < position("bitfinex:movement:303"), "{order:?}");
}
