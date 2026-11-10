//! Bitpanda's public API over a loopback fake built from the documented
//! response shapes (docs/reference/bitpanda.md). Bitpanda is Rust only, so
//! these tests pin the mapping itself rather than a Python reference.
use super::*;
use serde_json::Value as J;
use std::sync::Mutex;

const BTC: &str = "b86c034b-efe3-11eb-b56f-0691764446a7";
const ETH: &str = "b86c0a4c-efe3-11eb-b56f-0691764446a7";
const GOLD: &str = "b86c0e8a-efe3-11eb-b56f-0691764446a7";
const BEST: &str = "b86c12c8-efe3-11eb-b56f-0691764446a7";
const EUR: &str = "b88b8466-efe3-11eb-b56f-0691764446a7";
const USD: &str = "b88b8aa4-efe3-11eb-b56f-0691764446a7";

fn btc(value: &str) -> J {
    json!({"value": value, "asset_id": BTC, "currency_id": null})
}
fn eur(value: &str) -> J {
    json!({"value": value, "asset_id": null, "currency_id": EUR})
}
fn asset(id: &str, value: &str) -> J {
    json!({"value": value, "asset_id": id, "currency_id": null})
}

/// One transaction as `/v1/operations` documents it.
fn tx(id: &str, amount: J, flow: &str, at: &str, wallet: &str) -> J {
    json!({
        "transaction_id": id,
        "asset_id": amount["asset_id"],
        "currency_id": amount["currency_id"],
        "wallet_id": wallet,
        "wallet_owner": "user",
        "asset_amount": amount,
        "fee_amount": null,
        "transaction_type": "",
        "flow": flow,
        "order_id": null,
        "credited_at": at,
        "asset_balance_after": null,
        "compensates": null,
        "compensates_info": null,
        "index_asset_id": null,
        "trade": null,
    })
}
fn with(mut value: J, key: &str, item: J) -> J {
    value[key] = item;
    value
}
fn after(value: J, balance: &str) -> J {
    with(value, "asset_balance_after", btc(balance))
}
fn traded(value: J, id: &str) -> J {
    with(
        value,
        "trade",
        json!({"trade_id": id, "fee": eur("1.49"), "fee_percentage": "0.0149", "rate": "40000", "rate_with_fee": "40149", "to_eur_rate": "1"}),
    )
}
fn op(id: &str, kind: &str, transactions: Vec<J>) -> J {
    json!({"operation_id": id, "operation_type": kind, "transactions": transactions})
}

/// Every event kind Kassiber maps, one of each, plus what it skips.
fn history() -> Vec<J> {
    vec![
        op(
            "op-buy",
            "BUY",
            vec![
                after(
                    traded(
                        tx(
                            "t-buy",
                            btc("0.01"),
                            "INCOMING",
                            "2024-01-02T10:00:00+01:00",
                            "w-btc",
                        ),
                        "trade-1",
                    ),
                    "0.01",
                ),
                tx(
                    "t-buy-eur",
                    eur("401.49"),
                    "OUTGOING",
                    "2024-01-02T10:00:00+01:00",
                    "w-eur",
                ),
            ],
        ),
        op(
            "op-deposit",
            "DEPOSIT",
            vec![after(
                tx(
                    "t-deposit",
                    btc("0.005"),
                    "INCOMING",
                    "2024-01-03T00:00:00Z",
                    "w-btc",
                ),
                "0.015",
            )],
        ),
        op(
            "op-internal",
            "TRANSFER",
            vec![
                after(
                    tx(
                        "t-out",
                        btc("0.005"),
                        "OUTGOING",
                        "2024-01-04T00:00:00Z",
                        "w-btc",
                    ),
                    "0.010",
                ),
                after(
                    tx(
                        "t-in",
                        btc("0.005"),
                        "INCOMING",
                        "2024-01-04T00:00:00Z",
                        "w-earn",
                    ),
                    "0.005",
                ),
            ],
        ),
        op(
            "op-withdrawal",
            "WITHDRAWAL",
            vec![with(
                after(
                    tx(
                        "t-withdrawal",
                        btc("0.004"),
                        "OUTGOING",
                        "2024-01-05T12:30:45.999Z",
                        "w-btc",
                    ),
                    "0.0059",
                ),
                "fee_amount",
                btc("0.0001"),
            )],
        ),
        op(
            "op-sell",
            "SELL",
            vec![
                after(
                    traded(
                        tx(
                            "t-sell",
                            btc("0.002"),
                            "OUTGOING",
                            "2024-01-06T00:00:00Z",
                            "w-btc",
                        ),
                        "trade-2",
                    ),
                    "0.0039",
                ),
                tx(
                    "t-sell-eur",
                    eur("84.00"),
                    "INCOMING",
                    "2024-01-06T00:00:00Z",
                    "w-eur",
                ),
            ],
        ),
        op(
            "op-reward",
            "REWARD",
            vec![after(
                tx(
                    "t-reward",
                    btc("0.00001"),
                    "INCOMING",
                    "2024-01-07T00:00:00Z",
                    "w-btc",
                ),
                "0.00391",
            )],
        ),
        op(
            "op-fiat",
            "DEPOSIT",
            vec![tx(
                "t-fiat",
                eur("500"),
                "INCOMING",
                "2024-01-01T00:00:00Z",
                "w-eur",
            )],
        ),
        op(
            "op-eth",
            "BUY",
            vec![
                traded(
                    tx(
                        "t-eth",
                        asset(ETH, "1"),
                        "INCOMING",
                        "2024-01-02T00:00:00Z",
                        "w-eth",
                    ),
                    "trade-3",
                ),
                tx(
                    "t-eth-eur",
                    eur("2000"),
                    "OUTGOING",
                    "2024-01-02T00:00:00Z",
                    "w-eur",
                ),
            ],
        ),
        op(
            "op-savings",
            "SAVINGS_PLAN",
            vec![
                after(
                    traded(
                        tx(
                            "t-savings",
                            btc("0.001"),
                            "INCOMING",
                            "2024-01-08T00:00:00Z",
                            "w-btc",
                        ),
                        "trade-4",
                    ),
                    "0.00491",
                ),
                tx(
                    "t-savings-eur",
                    eur("25"),
                    "OUTGOING",
                    "2024-01-08T00:00:00Z",
                    "w-eur",
                ),
            ],
        ),
        op(
            "op-gold",
            "BUY",
            vec![
                traded(
                    tx(
                        "t-gold",
                        asset(GOLD, "0.5"),
                        "INCOMING",
                        "2024-01-09T00:00:00Z",
                        "w-gold",
                    ),
                    "trade-5",
                ),
                tx(
                    "t-gold-eur",
                    eur("30"),
                    "OUTGOING",
                    "2024-01-09T00:00:00Z",
                    "w-eur",
                ),
            ],
        ),
    ]
}

/// The scripted server: documented shapes, two operations per page.
#[derive(Clone)]
struct Script {
    limits: (u16, String),
    assets: J,
    operations: Vec<J>,
    page: usize,
    /// Replies served instead of the scripted one, by request ordinal.
    faults: Vec<(usize, u16, String)>,
}
impl Script {
    fn new() -> Self {
        Self {
            limits: (401, r#"{"error":{"code":"insufficient_scope"}}"#.into()),
            assets: json!([
                {"id": "f0f13b6a-0000-4000-8000-000000000001", "name": "Grayscale Bitcoin Mini Trust", "symbol": "BTC", "type": "equity_security", "group": "ETF"},
                {"id": BTC.to_uppercase(), "name": "Bitcoin", "symbol": "BTC", "type": "CRYPTO", "group": "CRYPTOCOIN"},
            ]),
            operations: history(),
            page: 2,
            faults: vec![],
        }
    }
    fn reply(&self, ordinal: usize, target: &str) -> (u16, String) {
        if let Some((_, status, body)) = self.faults.iter().find(|(at, _, _)| *at == ordinal) {
            return (*status, body.clone());
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let param = |name: &str| {
            query
                .split('&')
                .find_map(|p| p.strip_prefix(&format!("{name}=")))
                .map(str::to_owned)
        };
        match path {
            "/v1/trading-limits" => self.limits.clone(),
            "/v1/assets" => (
                200,
                json!({"data": self.assets, "self_cursor": "", "next_cursor": "", "has_next_page": false})
                    .to_string(),
            ),
            "/v1/currencies" => (
                200,
                json!({"data": [{"id": EUR, "symbol": "EUR", "name": "Euro"}, {"id": USD, "symbol": "USD", "name": "US Dollar"}]})
                    .to_string(),
            ),
            "/v1/operations" => {
                // The cursor is the next offset, base64-like so it needs escaping.
                let start: usize = param("cursor")
                    .map(|c| {
                        c.trim_start_matches("c%2B")
                            .trim_end_matches("%3D%3D")
                            .parse()
                            .unwrap()
                    })
                    .unwrap_or(0);
                let end = (start + self.page).min(self.operations.len());
                let more = end < self.operations.len();
                (
                    200,
                    json!({
                        "data": self.operations[start..end],
                        "self_cursor": format!("c+{start}=="),
                        "next_cursor": if more { json!(format!("c+{end}==")) } else { json!(null) },
                        "has_next_page": more,
                    })
                    .to_string(),
                )
            }
            _ => (404, r#"{"error":{"code":"not_found"}}"#.into()),
        }
    }
}

struct Served {
    fake: loopback::Fake,
}
impl Served {
    fn targets(&self) -> Vec<String> {
        self.fake
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.target.clone())
            .collect()
    }
}
fn serve_script(script: Script) -> Served {
    let script = Mutex::new(script);
    Served {
        fake: serve("bitpanda", move |ordinal, _, target| {
            script.lock().unwrap().reply(ordinal, target)
        }),
    }
}
fn sync(script: Script) -> (Served, loopback::Run) {
    let served = serve_script(script);
    let run = run(
        &backend("bitpanda", &served.fake.url),
        "bitpanda",
        "",
        true,
        online(),
    );
    (served, run)
}
fn refusal(script: Script) -> AppError {
    let (_, run) = sync(script);
    run.result.unwrap_err()
}

type Row = (
    String,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);
fn rows(conn: &Connection) -> Vec<Row> {
    let mut statement = conn
        .prepare(
            "SELECT external_id, kind, amount, fee, fiat_value_exact, fiat_currency, pricing_method, occurred_at
             FROM transactions ORDER BY occurred_at",
        )
        .unwrap();
    statement
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn bitpanda_reads_only_and_maps_every_btc_event() {
    let (served, run) = sync(Script::new());
    let result = run.result.unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(6), "{result:?}");
    assert_eq!(at(&result, "fetched"), &Py::from(6));
    assert_eq!(at(&result, "source"), &s("bitpanda:api:fixture"));
    assert_eq!(at(&result, "input_format"), &s("bitpanda_api"));
    assert_eq!(
        kassiber_pyjson::dumps(at(&result, "skipped"), &Default::default()).unwrap(),
        r#"{"fiat_only_operations": 1, "non_btc_operations": 2, "internal_btc_moves": 1}"#
    );
    // Only reads, each carrying the key, in this order; cursors escaped.
    assert!(served.fake.seen.lock().unwrap().iter().all(|r| r.signed));
    assert_eq!(
        served.targets(),
        [
            "/v1/trading-limits",
            "/v1/assets?symbol=BTC&page_size=100",
            "/v1/currencies",
            "/v1/operations?page_size=100",
            "/v1/operations?page_size=100&cursor=c%2B2%3D%3D",
            "/v1/operations?page_size=100&cursor=c%2B4%3D%3D",
            "/v1/operations?page_size=100&cursor=c%2B6%3D%3D",
            "/v1/operations?page_size=100&cursor=c%2B8%3D%3D",
        ]
    );
    let some = |v: &str| Some(v.to_owned());
    let row = |id: &str, kind: &str, amount, fee, value: Option<String>, at: &str| -> Row {
        let priced = value.is_some();
        (
            format!("bitpanda:{id}"),
            kind.into(),
            amount,
            fee,
            value,
            // An unpriced row still carries the book's currency.
            Some("EUR".to_owned()),
            priced.then(|| "bitpanda_api".to_owned()),
            at.into(),
        )
    };
    assert_eq!(
        rows(&run.conn),
        vec![
            // 10:00 at +01:00 is 09:00 UTC; the cash leg is the exact cost.
            row(
                "t-buy",
                "buy",
                1_000_000_000,
                0,
                some("401.49"),
                "2024-01-02T09:00:00Z"
            ),
            row(
                "t-deposit",
                "deposit",
                500_000_000,
                0,
                None,
                "2024-01-03T00:00:00Z"
            ),
            // Balance 0.010 -> 0.0059: the 0.004 left besides the 0.0001 fee.
            row(
                "t-withdrawal",
                "withdrawal",
                400_000_000,
                10_000_000,
                None,
                "2024-01-05T12:30:45Z"
            ),
            row(
                "t-sell",
                "sell",
                200_000_000,
                0,
                some("84.00"),
                "2024-01-06T00:00:00Z"
            ),
            row(
                "t-reward",
                "income",
                1_000_000,
                0,
                None,
                "2024-01-07T00:00:00Z"
            ),
            row(
                "t-savings",
                "buy",
                100_000_000,
                0,
                some("25"),
                "2024-01-08T00:00:00Z"
            ),
        ]
    );
    let (kind, config): (String, String) = run
        .conn
        .query_row(
            "SELECT kind, config_json FROM wallets WHERE label='Bitpanda'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (kind.as_str(), config.as_str()),
        ("bitpanda", r#"{"backend": "fixture"}"#)
    );
    // The stored evidence keeps the operation, the BTC leg and the cash leg.
    let raw: String = run
        .conn
        .query_row(
            "SELECT raw_json FROM transactions WHERE external_id='bitpanda:t-buy'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(raw.contains(r#""cash_transaction": {"#), "{raw}");
    assert!(raw.contains(r#""operation_id": "op-buy""#), "{raw}");
    assert!(!raw.contains(loopback::KEY));
    let reference: String = run
        .conn
        .query_row(
            "SELECT pricing_external_ref FROM transactions WHERE external_id='bitpanda:t-sell'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reference, "trade-2");
}

#[test]
fn bitpanda_resync_is_idempotent() {
    let served = serve_script(Script::new());
    let backend = backend("bitpanda", &served.fake.url);
    let conn = crate::file_import::tests::fixture();
    conn.execute_batch("UPDATE profiles SET fiat_currency='EUR';
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','BTC','fixed');")
        .unwrap();
    let gate = Gate::new(
        || kassiber_net::Offline::Online,
        kassiber_net::Ledger::default(),
    );
    for expected in [6, 0] {
        let result = execute(
            &conn,
            "p",
            "",
            &backend,
            "bitpanda",
            &gate,
            "test",
            &mut |route| Some(Consent::explicit_trigger(route.request())),
            None,
        )
        .unwrap();
        assert_eq!(at(&result, "imported"), &Py::from(expected), "{result:?}");
    }
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 6);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM wallets WHERE kind='bitpanda'"),
        1
    );
}

#[test]
fn bitpanda_withdrawal_amount_may_include_its_fee() {
    let mut script = Script::new();
    // Balance 0.010 -> 0.006: the 0.004 included the 0.0001 fee.
    script.operations[3]["transactions"][0]["asset_balance_after"] = btc("0.006");
    let (_, run) = sync(script);
    run.result.unwrap();
    let (amount, fee): (i64, i64) = run
        .conn
        .query_row(
            "SELECT amount, fee FROM transactions WHERE external_id='bitpanda:t-withdrawal'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((amount, fee), (390_000_000, 10_000_000));
}

#[test]
fn bitpanda_refuses_keys_that_can_trade() {
    for status in [200, 404] {
        let mut script = Script::new();
        script.limits = (status, r#"{"data":{}}"#.into());
        let (served, run) = sync(script);
        let error = run.result.unwrap_err();
        assert_eq!(error.code, "auth_error", "{status}");
        assert!(error.message.contains("can trade"), "{}", error.message);
        // Nothing but the scope check was read, and nothing was imported.
        assert_eq!(served.targets(), ["/v1/trading-limits"]);
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
    // 403 means the scope is missing too, like 401.
    let mut script = Script::new();
    script.limits = (403, "{}".into());
    assert!(sync(script).1.result.is_ok());
    // Any other answer cannot tell, so nothing is read.
    let mut script = Script::new();
    script.limits = (500, "oops".into());
    let (served, run) = sync(script);
    let error = run.result.unwrap_err();
    assert!(
        error.message.starts_with("HTTP 500 from backend"),
        "{}",
        error.message
    );
    assert_eq!(served.targets().len(), 1);
}

#[test]
fn bitpanda_auth_errors_withhold_the_echoed_key() {
    let mut script = Script::new();
    script.faults = vec![(
        3,
        401,
        format!(r#"{{"error":{{"code":"invalid key {}"}}}}"#, loopback::KEY),
    )];
    let error = refusal(script);
    assert_eq!(error.code, "auth_error");
    assert_eq!(error.message, "Bitpanda refused the API key (HTTP 401)");
    let details =
        kassiber_pyjson::dumps(error.details.as_ref().unwrap(), &Default::default()).unwrap();
    assert_eq!(
        details,
        r#"{"http_status": 401, "bitpanda_error": "invalid key <redacted>"}"#
    );
    // Another status keeps the shared transport's withheld body prefix.
    let mut script = Script::new();
    script.faults = vec![(3, 400, format!("{}{}", "x".repeat(195), loopback::KEY))];
    let error = refusal(script);
    assert!(error.message.ends_with("x<reda"), "{}", error.message);
    assert!(!error.message.contains("synth"), "{}", error.message);
}

#[test]
fn bitpanda_throttling_is_retried_then_imports() {
    let mut script = Script::new();
    script.faults = vec![(3, 429, "{}".into())];
    let (served, run) = sync(script);
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(6));
    let targets = served.targets();
    assert_eq!(targets[3], targets[4]);
    // Exhausted retries are a retryable rate limit, with nothing imported.
    let mut script = Script::new();
    script.faults = (3..6).map(|n| (n, 429, "{}".to_owned())).collect();
    let (_, run) = sync(script);
    let error = run.result.unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.retryable),
        ("rate_limited", true)
    );
    assert!(
        error.message.starts_with("Bitpanda rate limited"),
        "{}",
        error.message
    );
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

#[test]
fn bitpanda_malformed_pages_fail_closed() {
    let page = |body: &str| {
        let mut script = Script::new();
        script.faults = vec![(3, 200, body.to_owned())];
        refusal(script)
    };
    for (body, message) in [
        ("not json", "Bitpanda response was not JSON"),
        ("[]", "Bitpanda response was not a JSON object"),
        (
            r#"{"data":{}}"#,
            "Bitpanda response data was not a JSON array",
        ),
        (
            r#"{"data":[]}"#,
            "Bitpanda page did not say whether another follows",
        ),
        (
            r#"{"data":[],"has_next_page":"no"}"#,
            "Bitpanda page did not say whether another follows",
        ),
        (
            r#"{"data":[],"has_next_page":true}"#,
            "Bitpanda page announced another page without a cursor",
        ),
        (
            r#"{"data":[],"has_next_page":true,"next_cursor":""}"#,
            "Bitpanda page announced another page without a cursor",
        ),
        (
            r#"{"data":[{"transactions":[]}],"has_next_page":false}"#,
            "Bitpanda operation without an id",
        ),
    ] {
        let error = page(body);
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("protocol_error", message),
            "{body}"
        );
    }
    // A cursor seen before would loop forever.
    let mut script = Script::new();
    script.faults = vec![(
        4,
        200,
        r#"{"data":[],"has_next_page":true,"next_cursor":"c+2=="}"#.into(),
    )];
    assert_eq!(refusal(script).message, "Bitpanda paging repeated a cursor");
    // An operation repeated across pages must be identical.
    let mut script = Script::new();
    let mut changed = script.operations[0].clone();
    changed["operation_type"] = json!("SELL");
    script.operations.push(changed);
    assert!(refusal(script)
        .message
        .contains("twice with different contents"));
    let mut script = Script::new();
    let same = script.operations[0].clone();
    script.operations.push(same);
    assert!(sync(script).1.result.is_ok());
}

#[test]
fn bitpanda_needs_one_btc_crypto_asset() {
    for assets in [
        json!([]),
        json!([{"id": BTC, "symbol": "BTC", "type": "CRYPTO"}, {"id": ETH, "symbol": "BTC", "group": "cryptocoin"}]),
        json!([{"symbol": "BTC", "type": "CRYPTO"}]),
    ] {
        let mut script = Script::new();
        script.assets = assets.clone();
        let error = refusal(script);
        assert_eq!(
            error.message, "Bitpanda did not list exactly one BTC crypto asset",
            "{assets}"
        );
    }
    // A stock sharing the ticker narrows to the crypto asset.
    let mut script = Script::new();
    script.assets = json!([
        {"id": ETH, "symbol": "BTC", "type": "equity_security", "group": "ETF"},
        {"id": BTC, "symbol": "BTC", "type": "crypto"},
    ]);
    assert!(sync(script).1.result.is_ok());
    // A single match needs no type to decide.
    let mut script = Script::new();
    script.assets = json!([{"id": BTC, "symbol": " btc "}]);
    assert!(sync(script).1.result.is_ok());
}

/// Each ambiguous BTC event refuses the whole sync and imports nothing.
#[test]
fn bitpanda_refuses_ambiguous_btc_events() {
    type Change = fn(&mut Vec<J>);
    let cases: Vec<(&str, Change, &str)> = vec![
        (
            "unknown type",
            |o| o[1]["operation_type"] = json!("STAKE"),
            "has unsupported type 'STAKE'",
        ),
        (
            "missing type",
            |o| o[1]["operation_type"] = json!(null),
            "has unsupported type 'None'",
        ),
        (
            "flow against type",
            |o| o[1]["transactions"][0]["flow"] = json!("OUTGOING"),
            "flows the other way",
        ),
        (
            "unknown flow",
            |o| o[1]["transactions"][0]["flow"] = json!("SIDEWAYS"),
            "flow other than INCOMING or OUTGOING",
        ),
        (
            "contradicting transaction type",
            |o| o[1]["transactions"][0]["transaction_type"] = json!("withdrawal"),
            "but transaction type 'withdrawal'",
        ),
        (
            "reversal",
            |o| o[1]["transactions"][0]["compensates"] = json!("t-deposit-0"),
            "reverses an earlier transaction",
        ),
        (
            "swap",
            |o| {
                o[0]["transactions"][1] = tx(
                    "t-buy-eth",
                    asset(ETH, "0.2"),
                    "OUTGOING",
                    "2024-01-02T09:00:00Z",
                    "w-eth",
                )
            },
            "alongside another asset",
        ),
        (
            "index trade",
            |o| {
                let extra = tx(
                    "t-buy-eth",
                    asset(ETH, "0.2"),
                    "INCOMING",
                    "2024-01-02T09:00:00Z",
                    "w-eth",
                );
                o[0]["transactions"].as_array_mut().unwrap().push(extra);
            },
            "alongside another asset",
        ),
        (
            "BEST fee",
            |o| o[0]["transactions"][0]["fee_amount"] = asset(BEST, "1"),
            "charges a separate fee",
        ),
        (
            "trade fee on the cash leg",
            |o| o[0]["transactions"][1]["fee_amount"] = eur("1.49"),
            "charges a separate fee",
        ),
        (
            "trade fee in BTC",
            |o| o[0]["transactions"][0]["fee_amount"] = btc("0.00001"),
            "charges a separate fee",
        ),
        (
            "trade without cash",
            |o| {
                o[0]["transactions"].as_array_mut().unwrap().pop();
            },
            "is not one BTC leg against one cash leg",
        ),
        (
            "cash both ways",
            |o| o[0]["transactions"][1]["flow"] = json!("INCOMING"),
            "moves BTC and cash the same way",
        ),
        (
            "unknown currency",
            |o| {
                o[0]["transactions"][1]["asset_amount"]["currency_id"] =
                    json!("b88b0000-0000-0000-0000-000000000000")
            },
            "currency Kassiber does not price",
        ),
        (
            "zero cash",
            |o| o[0]["transactions"][1]["asset_amount"]["value"] = json!("0"),
            "has no cash value",
        ),
        (
            "trade typed as deposit",
            |o| o[0]["operation_type"] = json!("DEPOSIT"),
            "carries trade details",
        ),
        (
            "cash without trade",
            |o| o[0]["transactions"][0]["trade"] = json!(null),
            "moves BTC and cash without trade details",
        ),
        (
            "unbalanced internal move",
            |o| o[2]["transactions"][1]["asset_amount"]["value"] = json!("0.004"),
            "do not balance",
        ),
        (
            "deposit fee",
            |o| o[1]["transactions"][0]["fee_amount"] = btc("0.0001"),
            "charges a fee Kassiber cannot place",
        ),
        (
            "withdrawal fee in cash",
            |o| o[3]["transactions"][0]["fee_amount"] = eur("1"),
            "charges a fee Kassiber cannot place",
        ),
        (
            "withdrawal without balances",
            |o| o[3]["transactions"][0]["asset_balance_after"] = json!(null),
            "balances do not show",
        ),
        (
            "withdrawal balance off",
            |o| o[3]["transactions"][0]["asset_balance_after"] = btc("0.005"),
            "balances do not show",
        ),
        (
            "withdrawal at a tie",
            |o| {
                // Without the tie the 01-03 balance would place this fee.
                let leg = &mut o[3]["transactions"][0];
                leg["credited_at"] = json!("2024-01-04T00:00:00Z");
                leg["asset_balance_after"] = btc("0.0109");
            },
            "balances do not show",
        ),
        (
            "first withdrawal",
            |o| {
                o.drain(0..3);
            },
            "balances do not show",
        ),
        (
            "naive time",
            |o| o[1]["transactions"][0]["credited_at"] = json!("2024-01-03T00:00:00"),
            "credit time without a time zone",
        ),
        (
            "garbage time",
            |o| o[1]["transactions"][0]["credited_at"] = json!("yesterday"),
            "credit time without a time zone",
        ),
        (
            "negative amount",
            |o| o[1]["transactions"][0]["asset_amount"]["value"] = json!("-0.005"),
            "plain non-negative decimal",
        ),
        (
            "number amount",
            |o| o[1]["transactions"][0]["asset_amount"]["value"] = json!(0.005),
            "plain non-negative decimal",
        ),
        (
            "exponent amount",
            |o| o[1]["transactions"][0]["asset_amount"]["value"] = json!("5E-3"),
            "plain non-negative decimal",
        ),
        (
            "NaN amount",
            |o| o[1]["transactions"][0]["asset_amount"]["value"] = json!("NaN"),
            "plain non-negative decimal",
        ),
        (
            "sub-msat amount",
            |o| o[1]["transactions"][0]["asset_amount"]["value"] = json!("0.000000000001"),
            "finer than a millisatoshi",
        ),
        (
            "zero BTC",
            |o| o[1]["transactions"][0]["asset_amount"]["value"] = json!("0"),
            "zero or finer than a millisatoshi",
        ),
        (
            "disagreeing asset",
            |o| o[1]["transactions"][0]["asset_id"] = json!(ETH),
            "asset and amount disagree",
        ),
        (
            "BTC only as a fee",
            |o| o[6]["transactions"][0]["fee_amount"] = btc("0.0001"),
            "involves BTC only as a fee",
        ),
        (
            "no transaction id",
            |o| o[1]["transactions"][0]["transaction_id"] = json!(""),
            "a transaction without an id",
        ),
        (
            "no transaction list",
            |o| o[1]["transactions"] = json!({}),
            "has no transaction list",
        ),
    ];
    for (name, change, message) in cases {
        let mut script = Script::new();
        change(&mut script.operations);
        let (_, run) = sync(script);
        let error = run.result.unwrap_err();
        assert_eq!(error.code, "validation", "{name}: {error:?}");
        assert!(error.message.contains(message), "{name}: {}", error.message);
        assert!(error.message.starts_with("Bitpanda operation '"), "{name}");
        assert_eq!(
            count(&run.conn, "SELECT count(*) FROM transactions"),
            0,
            "{name}"
        );
    }
}

#[test]
fn bitpanda_refusals_withhold_echoed_credentials() {
    let mut script = Script::new();
    script.operations[1]["operation_type"] = json!(format!("X {}", loopback::KEY));
    let error = refusal(script);
    assert!(
        error.message.contains("'X <redacted>'"),
        "{}",
        error.message
    );
}

#[test]
fn bitpanda_reads_need_consent_an_open_gate_and_a_key() {
    let served = serve_script(Script::new());
    let url = served.fake.url.clone();
    let refused = run(&backend("bitpanda", &url), "bitpanda", "", false, online());
    assert_eq!(refused.result.unwrap_err().code, "interaction_required");
    let blocked = run(
        &backend("bitpanda", &url),
        "bitpanda",
        "",
        true,
        Arc::new(AtomicBool::new(true)),
    );
    assert!(blocked.result.is_err());
    let mut keyless = backend("bitpanda", &url);
    keyless["token"] = json!(" ");
    let missing = run(&keyless, "bitpanda", "", true, online());
    let error = missing.result.unwrap_err();
    assert_eq!(error.code, "config_error");
    assert_eq!(error.message, "Bitpanda backend is missing its API key");
    assert!(served.targets().is_empty());
    // A stray secret is never needed, but is withheld like the key.
    let mut script = Script::new();
    script.faults = vec![(3, 401, r#"{"error":{"code":"bad stray-secret"}}"#.into())];
    let served = serve_script(script);
    let mut stray = backend("bitpanda", &served.fake.url);
    stray["auth_header"] = json!("stray-secret");
    let error = run(&stray, "bitpanda", "", true, online())
        .result
        .unwrap_err();
    assert!(
        kassiber_pyjson::dumps(error.details.as_ref().unwrap(), &Default::default())
            .unwrap()
            .contains("bad <redacted>")
    );
}

#[test]
fn bitpanda_query_values_are_escaped() {
    assert_eq!(
        bitpanda::escape("eyJsYXN0SWQiOiIxMjM0NTYifQ=="),
        "eyJsYXN0SWQiOiIxMjM0NTYifQ%3D%3D"
    );
    assert_eq!(bitpanda::escape("a+b/c d~-._"), "a%2Bb%2Fc%20d~-._");
}
