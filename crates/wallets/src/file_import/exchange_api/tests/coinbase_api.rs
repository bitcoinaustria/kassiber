//! Coinbase's v2 accounts and transactions over a loopback fake, for the
//! asset scopes of docs/plan/24-crypto-assets.md. Shapes follow Coinbase's
//! documented transaction resource; the Bitcoin-scope rows stay pinned by the
//! Python reference vectors.
use super::*;
use serde_json::Value as J;

fn money(amount: &str, currency: &str) -> J {
    json!({"amount": amount, "currency": currency})
}
fn account(id: &str, code: &str, kind: &str) -> J {
    json!({"id": id, "type": "wallet", "currency": {"code": code, "type": kind}})
}
/// One transaction: `amount` in `currency`, valued at `native` EUR.
fn tx(id: &str, kind: &str, at: &str, amount: &str, currency: &str, native: Option<&str>) -> J {
    let mut row = json!({"id": id, "type": kind, "status": "completed", "created_at": at,
        "amount": money(amount, currency)});
    if let Some(native) = native {
        row["native_amount"] = money(native, "EUR");
    }
    row
}
fn with(mut value: J, key: &str, item: J) -> J {
    value[key] = item;
    value
}
fn fill(id: &str, at: &str, amount: &str, currency: &str, order: &str, price: &str) -> J {
    with(
        tx(
            id,
            "advanced_trade_fill",
            at,
            amount,
            currency,
            Some("-100"),
        ),
        "advanced_trade_fill",
        json!({"fill_price": price, "product_id": "ETH-BTC", "order_id": order,
            "commission": "0.00001", "order_side": "BUY"}),
    )
}
fn page(rows: Vec<J>) -> String {
    json!({"data": rows, "pagination": {"next_uri": null}}).to_string()
}

/// The accounts list and each account's transactions, by path.
fn serve_accounts(accounts: Vec<J>, history: Vec<(&'static str, Vec<J>)>) -> loopback::Fake {
    let accounts = page(accounts);
    let history: Vec<(String, String)> = history
        .into_iter()
        .map(|(id, rows)| (format!("/v2/accounts/{id}/transactions"), page(rows)))
        .collect();
    serve("coinbase", move |_, _, target| {
        if target == "/v2/accounts" {
            return (200, accounts.clone());
        }
        match history.iter().find(|(path, _)| path == target) {
            Some((_, body)) => (200, body.clone()),
            None => (404, "{\"errors\":[{\"id\":\"not_found\"}]}".into()),
        }
    })
}

/// A Coinbase wallet in `scope` on an Austrian EUR book.
fn book(scope: &str) -> Connection {
    let conn = crate::file_import::tests::fixture();
    conn.execute_batch(&format!(
        "UPDATE profiles SET fiat_currency='EUR', tax_country='at';
         INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
         VALUES('cb','ws','p','Coinbase','coinbase','{{\"backend\":\"fixture\",\"asset_scope\":\"{scope}\"}}','2025-01-01');"
    ))
    .unwrap();
    conn
}

fn sync(conn: &Connection, url: &str) -> Result<Py, AppError> {
    let gate = Gate::new(
        || kassiber_net::Offline::Online,
        kassiber_net::Ledger::default(),
    );
    execute(
        conn,
        "p",
        "",
        &backend("coinbase", url),
        "coinbase",
        &gate,
        "test",
        &mut |route| Some(Consent::explicit_trigger(route.request())),
        None,
    )
}

fn dump(value: &Py) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

fn requested(fake: &loopback::Fake) -> Vec<String> {
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|r| r.signed), "{seen:?}");
    seen.into_iter().map(|r| r.target).collect()
}

type Row = (String, String, String, String, i64, Option<String>);
fn rows(conn: &Connection) -> Vec<Row> {
    let mut statement = conn
        .prepare(
            "SELECT external_id, asset, direction, kind, amount, CAST(fiat_value_exact AS TEXT)
             FROM transactions ORDER BY external_id",
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
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn coinbase_bitcoin_scope_reads_every_crypto_account_and_counts_other_assets() {
    let fake = serve_accounts(
        vec![
            account("btc", "BTC", "crypto"),
            account("eth", "ETH", "crypto"),
            account("usdc", "USDC", "crypto"),
            account("eur", "EUR", "fiat"),
            account("aapl", "AAPL", "equity"),
        ],
        vec![
            (
                "btc",
                vec![
                    tx(
                        "b1",
                        "buy",
                        "2025-01-02T10:00:00Z",
                        "0.01",
                        "BTC",
                        Some("-400"),
                    ),
                    // A fill against EUR is a sale.
                    with(
                        fill("f1", "2025-01-03T10:00:00Z", "-0.005", "BTC", "O1", "40000"),
                        "advanced_trade_fill",
                        json!({"product_id": "BTC-EUR", "order_id": "O1", "fill_price": "40000"}),
                    ),
                ],
            ),
            (
                "eth",
                vec![
                    tx(
                        "e1",
                        "buy",
                        "2025-01-04T10:00:00Z",
                        "1",
                        "ETH",
                        Some("-2000"),
                    ),
                    tx("e2", "send", "2025-01-05T10:00:00Z", "-0.5", "ETH", None),
                ],
            ),
            (
                "usdc",
                vec![tx(
                    "u1",
                    "buy",
                    "2025-01-06T10:00:00Z",
                    "100",
                    "USDC",
                    Some("-90"),
                )],
            ),
        ],
    );
    let conn = book("bitcoin");
    let result = sync(&conn, &fake.url).unwrap();
    // The same reads as the crypto scope; fiat and equity accounts are never
    // read.
    assert_eq!(
        requested(&fake),
        [
            "/v2/accounts",
            "/v2/accounts/btc/transactions",
            "/v2/accounts/eth/transactions",
            "/v2/accounts/usdc/transactions"
        ]
    );
    assert_eq!(at(&result, "imported"), &Py::from(2), "{result:?}");
    assert_eq!(
        dump(at(&result, "assets")),
        r#"{"scope": "bitcoin", "skipped": {"ETH": 2, "USDC": 1}, "excluded": {"AAPL": 1}, "next_step": "kassiber wallets update --wallet 'Coinbase' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep ETH, USDC"}"#
    );
    assert_eq!(at(&result, "swap_pairs"), &Py::from(0));
    let kinds: Vec<_> = rows(&conn).into_iter().map(|r| (r.0, r.3)).collect();
    assert_eq!(
        kinds,
        [("coinbase:b1", "buy"), ("coinbase:f1", "sell")]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
}

#[test]
fn coinbase_bitcoin_scope_refuses_a_btc_trade_fee_in_another_crypto_asset() {
    let fake = serve_accounts(
        vec![account("btc", "BTC", "crypto")],
        vec![(
            "btc",
            vec![with(
                tx(
                    "b1",
                    "buy",
                    "2025-01-02T10:00:00Z",
                    "0.01",
                    "BTC",
                    Some("-400"),
                ),
                "buy",
                json!({"id": "B1", "fee": money("0.2", "SOL")}),
            )],
        )],
    );
    let conn = book("bitcoin");
    let error = sync(&conn, &fake.url).unwrap_err();
    assert_eq!(error.message, "Coinbase BTC buy 'b1' pays its fee in SOL");
    assert!(error.hint.unwrap().contains("asset_scope"));
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 0);
}

/// Every row kind the crypto scope maps, across BTC, ETH and SOL accounts.
fn crypto_history() -> (Vec<J>, Vec<(&'static str, Vec<J>)>) {
    let convert = |id: &str, amount: &str, currency: &str, native: &str, trade: &str| {
        with(
            tx(
                id,
                "trade",
                "2025-02-01T10:00:00Z",
                amount,
                currency,
                Some(native),
            ),
            "trade",
            json!({"id": trade, "fee": money("0.1", "SOL")}),
        )
    };
    let accounts = vec![
        account("btc", "BTC", "crypto"),
        account("eth", "ETH", "crypto"),
        account("sol", "SOL", "crypto"),
        account("eur", "EUR", "fiat"),
    ];
    let history = vec![
        (
            "btc",
            vec![
                with(
                    tx(
                        "b1",
                        "buy",
                        "2025-01-02T10:00:00Z",
                        "0.01",
                        "BTC",
                        Some("-400"),
                    ),
                    "buy",
                    json!({"id": "B1", "commission": "1.5"}),
                ),
                convert("c-btc", "-0.005", "BTC", "-200", "T1"),
                fill(
                    "f-btc",
                    "2025-03-01T10:00:00Z",
                    "-0.02",
                    "BTC",
                    "O1",
                    "0.04",
                ),
                fill(
                    "g-btc-1",
                    "2025-03-02T10:00:00Z",
                    "-0.01",
                    "BTC",
                    "O2",
                    "0.05",
                ),
                fill(
                    "g-btc-2",
                    "2025-03-02T10:00:01Z",
                    "-0.02",
                    "BTC",
                    "O2",
                    "0.05",
                ),
            ],
        ),
        (
            "eth",
            vec![
                convert("c-eth", "0.1", "ETH", "199", "T1"),
                fill("f-eth", "2025-03-01T10:00:00Z", "0.5", "ETH", "O1", "0.04"),
                fill(
                    "g-eth-1",
                    "2025-03-02T10:00:00Z",
                    "0.2",
                    "ETH",
                    "O2",
                    "0.05",
                ),
                fill(
                    "g-eth-2",
                    "2025-03-02T10:00:01Z",
                    "0.4",
                    "ETH",
                    "O2",
                    "0.05",
                ),
                // A convert to EUR: its other leg is in the fiat account.
                with(
                    tx(
                        "c-fiat",
                        "trade",
                        "2025-04-01T10:00:00Z",
                        "-0.02",
                        "ETH",
                        Some("-40"),
                    ),
                    "trade",
                    json!({"id": "T2"}),
                ),
                tx(
                    "s-eth",
                    "sell",
                    "2025-04-02T10:00:00Z",
                    "-0.05",
                    "ETH",
                    Some("-100"),
                ),
                with(
                    tx(
                        "b-eth",
                        "buy",
                        "2025-04-03T10:00:00Z",
                        "0.3",
                        "ETH",
                        Some("-600"),
                    ),
                    "buy",
                    json!({"id": "B9", "fee": money("0.5", "SOL")}),
                ),
                with(
                    tx(
                        "o-eth",
                        "send",
                        "2025-05-01T10:00:00Z",
                        "-0.01",
                        "ETH",
                        None,
                    ),
                    "network",
                    json!({"status": "confirmed", "hash": "0xabc"}),
                ),
                tx(
                    "i-eth",
                    "receive",
                    "2025-05-02T10:00:00Z",
                    "0.2",
                    "ETH",
                    None,
                ),
                tx(
                    "r-eth",
                    "staking_reward",
                    "2025-05-03T10:00:00Z",
                    "0.001",
                    "ETH",
                    None,
                ),
            ],
        ),
        (
            "sol",
            vec![
                tx(
                    "r-sol",
                    "inflation_reward",
                    "2025-06-01T10:00:00Z",
                    "1.5",
                    "SOL",
                    None,
                ),
                tx(
                    "n-sol",
                    "interest",
                    "2025-06-02T10:00:00Z",
                    "0.25",
                    "SOL",
                    None,
                ),
            ],
        ),
    ];
    (accounts, history)
}

#[test]
fn coinbase_crypto_scope_maps_every_row_kind_and_pairs_swaps_once() {
    let (accounts, history) = crypto_history();
    let fake = serve_accounts(accounts, history);
    let conn = book("crypto");
    let result = sync(&conn, &fake.url).unwrap();
    // The fiat account is never read.
    assert_eq!(
        requested(&fake),
        [
            "/v2/accounts",
            "/v2/accounts/btc/transactions",
            "/v2/accounts/eth/transactions",
            "/v2/accounts/sol/transactions"
        ]
    );
    assert_eq!(
        dump(at(&result, "assets")),
        r#"{"scope": "crypto", "skipped": {}, "excluded": {}}"#
    );
    assert_eq!(at(&result, "swap_pairs"), &Py::from(4), "{result:?}");
    let row =
        |id: &str, asset: &str, direction: &str, kind: &str, amount: i64, value: Option<&str>| {
            (
                id.to_owned(),
                asset.to_owned(),
                direction.to_owned(),
                kind.to_owned(),
                amount,
                value.map(str::to_owned),
            )
        };
    assert_eq!(
        rows(&conn),
        [
            row(
                "0xabc",
                "ETH",
                "outbound",
                "withdrawal",
                1_000_000_000,
                None
            ),
            row(
                "coinbase:b-eth",
                "ETH",
                "inbound",
                "buy",
                30_000_000_000,
                Some("600")
            ),
            row(
                "coinbase:b1",
                "BTC",
                "inbound",
                "buy",
                1_000_000_000,
                Some("401.5")
            ),
            row("coinbase:buy:B9:fee", "SOL", "outbound", "fee", 0, None),
            row(
                "coinbase:c-btc",
                "BTC",
                "outbound",
                "swap",
                500_000_000,
                Some("200")
            ),
            row(
                "coinbase:c-eth",
                "ETH",
                "inbound",
                "swap",
                10_000_000_000,
                Some("199")
            ),
            row(
                "coinbase:c-fiat",
                "ETH",
                "outbound",
                "sell",
                2_000_000_000,
                Some("40")
            ),
            row(
                "coinbase:f-btc",
                "BTC",
                "outbound",
                "swap",
                2_000_000_000,
                Some("100")
            ),
            row(
                "coinbase:f-eth",
                "ETH",
                "inbound",
                "swap",
                50_000_000_000,
                Some("100")
            ),
            row(
                "coinbase:g-btc-1",
                "BTC",
                "outbound",
                "swap",
                1_000_000_000,
                Some("100")
            ),
            row(
                "coinbase:g-btc-2",
                "BTC",
                "outbound",
                "swap",
                2_000_000_000,
                Some("100")
            ),
            row(
                "coinbase:g-eth-1",
                "ETH",
                "inbound",
                "swap",
                20_000_000_000,
                Some("100")
            ),
            row(
                "coinbase:g-eth-2",
                "ETH",
                "inbound",
                "swap",
                40_000_000_000,
                Some("100")
            ),
            row(
                "coinbase:i-eth",
                "ETH",
                "inbound",
                "deposit",
                20_000_000_000,
                None
            ),
            row(
                "coinbase:n-sol",
                "SOL",
                "inbound",
                "interest",
                25_000_000_000,
                None
            ),
            row(
                "coinbase:r-eth",
                "ETH",
                "inbound",
                "staking",
                100_000_000,
                None
            ),
            row(
                "coinbase:r-sol",
                "SOL",
                "inbound",
                "staking",
                150_000_000_000,
                None
            ),
            row(
                "coinbase:s-eth",
                "ETH",
                "outbound",
                "sell",
                5_000_000_000,
                Some("100")
            ),
            row("coinbase:trade:T1:fee", "SOL", "outbound", "fee", 0, None),
        ]
    );
    // Third-asset fees are fee-only spends: the whole SOL quantity is the fee.
    let fees: Vec<(String, i64)> = conn
        .prepare("SELECT external_id, fee FROM transactions WHERE kind='fee' ORDER BY external_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        fees,
        [
            ("coinbase:buy:B9:fee".to_owned(), 50_000_000_000),
            ("coinbase:trade:T1:fee".to_owned(), 10_000_000_000),
        ]
    );
    // Each fill's legs pair by their order, then by their time and price.
    let pairs: Vec<(String, String)> = conn
        .prepare(
            "SELECT min(t.external_id), max(t.external_id)
             FROM custody_component_transaction_memberships m
             JOIN transactions t ON t.id = m.transaction_id
             GROUP BY m.component_id ORDER BY 1",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        pairs,
        [
            ("coinbase:c-btc", "coinbase:c-eth"),
            ("coinbase:f-btc", "coinbase:f-eth"),
            ("coinbase:g-btc-1", "coinbase:g-eth-1"),
            ("coinbase:g-btc-2", "coinbase:g-eth-2"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );

    // A re-sync imports nothing and authors no second pair.
    let again = sync(&conn, &fake.url).unwrap();
    assert_eq!(at(&again, "imported"), &Py::from(0), "{again:?}");
    assert_eq!(at(&again, "swap_pairs"), &Py::from(0));
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 19);
    assert_eq!(
        count(
            &conn,
            "SELECT count(DISTINCT component_id) FROM custody_component_transaction_memberships"
        ),
        4
    );
}

/// The crypto-scope error for one edited ETH history, with nothing stored.
fn crypto_refusal(eth: Vec<J>) -> AppError {
    let fake = serve_accounts(
        vec![
            account("btc", "BTC", "crypto"),
            account("eth", "ETH", "crypto"),
        ],
        vec![
            (
                "btc",
                vec![
                    with(
                        tx(
                            "c-btc",
                            "trade",
                            "2025-02-01T10:00:00Z",
                            "-0.005",
                            "BTC",
                            Some("-200"),
                        ),
                        "trade",
                        json!({"id": "T1"}),
                    ),
                    fill(
                        "f-btc",
                        "2025-03-01T10:00:00Z",
                        "-0.02",
                        "BTC",
                        "O1",
                        "0.04",
                    ),
                ],
            ),
            ("eth", eth),
        ],
    );
    let conn = book("crypto");
    let error = sync(&conn, &fake.url).unwrap_err();
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 0);
    error
}

#[test]
fn coinbase_crypto_scope_refuses_what_it_cannot_map_exactly() {
    let leg = |id: &str, amount: &str, trade: J| {
        with(
            tx(
                id,
                "trade",
                "2025-02-01T10:00:00Z",
                amount,
                "ETH",
                Some("199"),
            ),
            "trade",
            trade,
        )
    };
    let paired = || leg("c-eth", "0.1", json!({"id": "T1"}));
    let matched = || fill("f-eth", "2025-03-01T10:00:00Z", "0.5", "ETH", "O1", "0.04");
    let plain = |id: &str, kind: &str, amount: &str| {
        tx(id, kind, "2025-05-01T10:00:00Z", amount, "ETH", None)
    };
    let product = |product: &str| {
        with(
            matched(),
            "advanced_trade_fill",
            json!({"product_id": product, "order_id": "O1"}),
        )
    };
    let id = |id: &str| json!({ "id": id });
    for (eth, message) in [
        (
            vec![paired(), matched(), plain("e1", "earn_payout", "0.1")],
            "Coinbase ETH transaction 'e1' has unsupported type 'earn_payout'",
        ),
        (
            vec![paired(), leg("c-2", "0.1", id("T1")), matched()],
            "Coinbase trade 'T1' needs one leg in each of two crypto accounts and has 3",
        ),
        (
            vec![leg("c-eth", "-0.1", id("T1")), matched()],
            "Coinbase trade 'T1' does not have one leg out and one leg in",
        ),
        (
            vec![paired(), matched(), leg("c-3", "0.1", json!({}))],
            "Coinbase ETH trade 'c-3' has no trade id to find its other leg",
        ),
        (
            vec![paired()],
            "Coinbase order 'O1' fill at 2025-03-01T10:00:00Z for 0.04 needs one leg in each of two crypto accounts and has 1",
        ),
        (
            vec![paired(), product("SOL-BTC")],
            "Coinbase ETH fill 'f-eth' names product 'SOL-BTC', which is not a pair of its asset",
        ),
        (
            vec![paired(), matched(), leg("x1", "-0.1", id("T5")), leg("x2", "0.1", id("T5"))],
            "Coinbase trade 'T5' has both legs in ETH",
        ),
        (
            vec![paired(), matched(), plain("i1", "receive", "-0.2")],
            "Coinbase ETH receive 'i1' has a negative amount",
        ),
        (
            vec![paired(), matched(), plain("i2", "receive", "0.000000000001")],
            "Coinbase transaction 'coinbase:i2': ETH amount 1E-12 has more than 11 decimal places",
        ),
        // Forty digits would round to exactly 1 ETH at the decimal
        // context's 32 digits; the row is refused before any rounding.
        (
            vec![
                paired(),
                matched(),
                plain("i3", "receive", "1.0000000000000000000000000000000000000001"),
            ],
            "Coinbase transaction 'coinbase:i3': ETH amount 1.0000000000000000000000000000000000000001 has more than 11 decimal places",
        ),
    ] {
        let error = crypto_refusal(eth);
        assert_eq!(error.message, message, "{error:?}");
        assert_eq!(error.code, "validation", "{message}");
    }
}

#[test]
fn coinbase_crypto_scope_refuses_accounts_it_cannot_name() {
    for (listed, message) in [
        (
            json!({"id": "", "currency": {"code": "ETH", "type": "crypto"}}),
            "Coinbase listed an account in ETH without an id",
        ),
        (
            json!({"id": "x", "currency": {"type": "crypto"}}),
            "Coinbase listed an account without a currency",
        ),
    ] {
        let fake = serve_accounts(vec![listed], vec![]);
        let conn = book("crypto");
        let error = sync(&conn, &fake.url).unwrap_err();
        assert_eq!(error.message, message);
        assert_eq!(requested(&fake), ["/v2/accounts"]);
    }
}

/// The bitcoin scope refuses a BTC row that may be one leg of a
/// crypto-to-crypto trade, naming it and the scope command, instead of
/// booking a tax-neutral swap as a fiat-priced sale (an Austrian book would
/// realize a gain). A fill on a fiat-quoted product stays a sale.
#[test]
fn coinbase_bitcoin_scope_refuses_a_btc_leg_of_a_crypto_trade() {
    let convert = with(
        tx(
            "c1",
            "trade",
            "2025-01-03T10:00:00Z",
            "-0.005",
            "BTC",
            Some("-200"),
        ),
        "trade",
        json!({"id": "T1"}),
    );
    let quoted = |product: &str| {
        with(
            fill("f1", "2025-01-03T10:00:00Z", "-0.005", "BTC", "O1", "1"),
            "advanced_trade_fill",
            json!({"product_id": product, "order_id": "O1", "fill_price": "1"}),
        )
    };
    // The other leg, in the other crypto account the sync reads in every
    // scope.
    let other_convert = with(
        tx(
            "c2",
            "trade",
            "2025-01-03T10:00:00Z",
            "0.1",
            "ETH",
            Some("200"),
        ),
        "trade",
        json!({"id": "T1"}),
    );
    let other_fill = |product: &str, amount: &str, currency: &str| {
        with(
            fill("f2", "2025-01-03T10:00:00Z", amount, currency, "O1", "1"),
            "advanced_trade_fill",
            json!({"product_id": product, "order_id": "O1", "fill_price": "1"}),
        )
    };
    for (row, other, message) in [
        (
            convert,
            ("eth", other_convert),
            "Coinbase BTC convert 'c1' trades against ETH, which the bitcoin asset scope does not import",
        ),
        (
            quoted("ETH-BTC"),
            ("eth", other_fill("ETH-BTC", "0.1", "ETH")),
            "Coinbase BTC fill 'f1' trades against ETH, which the bitcoin asset scope does not import",
        ),
        (
            quoted("BTC-USDC"),
            ("usdc", other_fill("BTC-USDC", "200", "USDC")),
            "Coinbase BTC fill 'f1' trades against USDC, which the bitcoin asset scope does not import",
        ),
    ] {
        let fake = serve_accounts(
            vec![
                account("btc", "BTC", "crypto"),
                account(other.0, &other.0.to_uppercase(), "crypto"),
            ],
            vec![("btc", vec![row]), (other.0, vec![other.1])],
        );
        let conn = book("bitcoin");
        let error = sync(&conn, &fake.url).unwrap_err();
        assert_eq!(error.message, message);
        assert_eq!(
            error.hint.as_deref(),
            Some(r#"Kassiber does not book one leg of a swap as a sale or purchase. To import both legs, run kassiber wallets update --wallet 'Coinbase' --config '{"asset_scope": "crypto"}' and sync again."#)
        );
        assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 0);
    }
}

/// An Austrian book's BTC -> ETH convert: the bitcoin scope refuses it and
/// stores nothing; the crypto scope stores it as two paired swap legs (no
/// sale, so no realized gain), once.
#[test]
fn coinbase_austrian_convert_is_a_swap_only_in_the_crypto_scope() {
    let convert = |id: &str, amount: &str, currency: &str, native: &str| {
        with(
            tx(
                id,
                "trade",
                "2025-02-01T10:00:00Z",
                amount,
                currency,
                Some(native),
            ),
            "trade",
            json!({"id": "T1"}),
        )
    };
    let fake = serve_accounts(
        vec![
            account("btc", "BTC", "crypto"),
            account("eth", "ETH", "crypto"),
        ],
        vec![
            ("btc", vec![convert("c-btc", "-0.005", "BTC", "-200")]),
            ("eth", vec![convert("c-eth", "0.1", "ETH", "199")]),
        ],
    );
    let conn = book("bitcoin");
    sync(&conn, &fake.url).unwrap_err();
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 0);
    assets::Scope::Crypto.store(&conn, "p", "cb").unwrap();
    let result = sync(&conn, &fake.url).unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(2), "{result:?}");
    assert_eq!(at(&result, "swap_pairs"), &Py::from(1));
    let kinds: Vec<_> = rows(&conn).into_iter().map(|r| (r.0, r.3)).collect();
    assert_eq!(
        kinds,
        [("coinbase:c-btc", "swap"), ("coinbase:c-eth", "swap")]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    let again = sync(&conn, &fake.url).unwrap();
    assert_eq!(at(&again, "swap_pairs"), &Py::from(0));
}

/// An ETH buy paying its fee in BTC: the bitcoin scope leaves the ETH out
/// but books the BTC fee, as the same row the crypto scope books.
#[test]
fn coinbase_books_a_btc_fee_of_another_assets_trade_in_both_scopes() {
    let history = || {
        vec![(
            "eth",
            vec![with(
                tx(
                    "e1",
                    "buy",
                    "2025-01-02T10:00:00Z",
                    "1",
                    "ETH",
                    Some("-2000"),
                ),
                "buy",
                json!({"id": "E1", "fee": money("0.0001", "BTC")}),
            )],
        )]
    };
    let accounts = || vec![account("eth", "ETH", "crypto")];
    let mut fees = vec![];
    for scope in ["bitcoin", "crypto"] {
        let fake = serve_accounts(accounts(), history());
        let conn = book(scope);
        sync(&conn, &fake.url).unwrap();
        let btc: Vec<Row> = rows(&conn).into_iter().filter(|r| r.1 == "BTC").collect();
        assert_eq!(btc.len(), 1, "{scope}: {btc:?}");
        fees.push((
            btc,
            count(&conn, "SELECT fee FROM transactions WHERE asset='BTC'"),
        ));
    }
    assert_eq!(fees[0], fees[1]);
    assert_eq!(fees[0].1, 10_000_000);
}

/// Two fills of one order at the same time and price pair by size, each
/// outgoing leg with the incoming leg of its own fill.
#[test]
fn coinbase_pairs_fills_sharing_a_time_and_price_by_size() {
    let at = "2025-03-01T10:00:00Z";
    let fake = serve_accounts(
        vec![
            account("btc", "BTC", "crypto"),
            account("eth", "ETH", "crypto"),
        ],
        vec![
            (
                "btc",
                vec![
                    fill("b-1", at, "-0.04", "BTC", "O2", "0.04"),
                    fill("b-2", at, "-0.02", "BTC", "O2", "0.04"),
                ],
            ),
            (
                "eth",
                vec![
                    fill("e-1", at, "0.5", "ETH", "O2", "0.04"),
                    fill("e-2", at, "1", "ETH", "O2", "0.04"),
                ],
            ),
        ],
    );
    let conn = book("crypto");
    let result = sync(&conn, &fake.url).unwrap();
    assert_eq!(at_key(&result, "swap_pairs"), Py::from(2), "{result:?}");
    let scope = kassiber_transfers::Scope {
        workspace_id: "ws",
        profile_id: "p",
    };
    let external = |id: &str| -> String {
        conn.query_row(
            "SELECT external_id FROM transactions WHERE id=?",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    let mut pairs: Vec<(String, String)> = kassiber_transfers::transaction_pairs(&conn, &scope)
        .unwrap()
        .iter()
        .map(|(out, into)| (external(out), external(into)))
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        [
            ("coinbase:b-1", "coinbase:e-2"),
            ("coinbase:b-2", "coinbase:e-1")
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
}

fn at_key(value: &Py, key: &str) -> Py {
    at(value, key).clone()
}
