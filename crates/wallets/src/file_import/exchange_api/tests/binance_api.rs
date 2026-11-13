//! Binance in each asset scope (docs/plan/23-crypto-assets.md), over the
//! signed loopback fake. The fake answers each history within the query's
//! time window, as Binance does, and records every read.
use super::*;
use serde_json::Value as J;

const DAY: u64 = 86_400_000;
/// Mid-2023, inside one window of every history.
const T: u64 = 1_700_000_000_000;

/// The histories one fake Binance holds, by endpoint.
#[derive(Clone, Default)]
struct Histories {
    fiat: Vec<J>,
    deposits: Vec<J>,
    withdrawals: Vec<J>,
    dividends: Vec<J>,
    converts: Vec<J>,
    /// Convert pages report `moreData` whenever a window holds more rows.
    convert_page: usize,
}

fn query(target: &str) -> Vec<(String, String)> {
    target
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}
fn param(target: &str, key: &str) -> Option<u64> {
    query(target)
        .into_iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.parse().ok())
}

fn binance(histories: Histories) -> loopback::Fake {
    serve("binance", move |_, _, target| {
        let path = target.split('?').next().unwrap();
        let (start, end) = (
            param(target, "startTime").unwrap_or(0),
            param(target, "endTime").unwrap_or(u64::MAX),
        );
        let within = |rows: &[J], key: &str| -> Vec<J> {
            rows.iter()
                .filter(|row| (start..=end).contains(&row[key].as_u64().unwrap()))
                .cloned()
                .collect()
        };
        let paged = |rows: Vec<J>| -> Vec<J> {
            let offset = param(target, "offset").unwrap_or(0) as usize;
            let limit = param(target, "limit").unwrap_or(1000) as usize;
            rows.into_iter().skip(offset).take(limit).collect()
        };
        let body = match path {
            "/sapi/v1/fiat/payments" => json!({"data": histories.fiat}),
            "/sapi/v1/capital/deposit/hisrec" => {
                J::Array(paged(within(&histories.deposits, "insertTime")))
            }
            "/sapi/v1/capital/withdraw/history" => {
                J::Array(paged(within(&histories.withdrawals, "applyTime")))
            }
            "/sapi/v1/asset/assetDividend" => {
                let rows = within(&histories.dividends, "divTime");
                json!({"rows": rows, "total": rows.len()})
            }
            "/sapi/v1/convert/tradeFlow" => {
                let rows = within(&histories.converts, "createTime");
                let page = histories.convert_page.max(1);
                let more = rows.len() > page;
                json!({"list": rows.into_iter().take(page).collect::<Vec<_>>(), "moreData": more})
            }
            _ => return (404, "{}".into()),
        };
        (200, body.to_string())
    })
}

/// The Binance wallet a sync finds by label, in `scope`.
fn wallet(scope: &'static str) -> impl Fn(&Connection) {
    move |conn: &Connection| {
        conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('bw','ws','p','Binance','binance',?,'2025-01-01')",
            [json!({"backend": "fixture", "asset_scope": scope}).to_string()],
        )
        .unwrap();
        conn.execute_batch("UPDATE profiles SET tax_country='at' WHERE id='p'")
            .unwrap();
    }
}

fn sync(fake: &loopback::Fake, scope: &'static str, consent: bool) -> loopback::Run {
    loopback::run_prepared(
        &backend("binance", &fake.url),
        "binance",
        "",
        consent,
        online(),
        &wallet(scope),
    )
}

fn dumped(value: &Py) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

/// `(asset, direction, kind, amount, fee, fiat_value)` of one stored row.
fn stored(conn: &Connection, id: &str) -> (String, String, String, i64, i64, Option<String>) {
    conn.query_row(
        "SELECT asset,direction,kind,amount,fee,CAST(fiat_value_exact AS TEXT) FROM transactions WHERE external_id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    )
    .unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn fiat_buy(order: &str, coin: &str, amount: &str, eur: &str) -> J {
    json!({"orderNo": order, "status": "Completed", "cryptoCurrency": coin, "fiatCurrency": "EUR",
        "obtainAmount": amount, "sourceAmount": eur, "createTime": T})
}
fn convert(order: &str, from: &str, from_amount: &str, to: &str, to_amount: &str, at: u64) -> J {
    json!({"quoteId": format!("q{order}"), "orderId": order, "orderStatus": "SUCCESS",
        "fromAsset": from, "fromAmount": from_amount, "toAsset": to, "toAmount": to_amount,
        "ratio": "1", "inverseRatio": "1", "createTime": at})
}

fn every_kind() -> Histories {
    Histories {
        fiat: vec![
            fiat_buy("F1", "BTC", "0.01", "-400"),
            fiat_buy("F2", "ETH", "0.5", "-1000"),
        ],
        deposits: vec![
            json!({"id": "d-btc", "coin": "BTC", "network": "BTC", "status": 1, "amount": "0.005", "insertTime": T, "txId": "btc-tx"}),
            json!({"id": "d-eth", "coin": "ETH", "network": "ETH", "status": 1, "amount": "1.5", "insertTime": T + 1, "txId": "eth-tx"}),
            json!({"id": "d-arb", "coin": "ETH", "network": "ARBITRUM", "status": 1, "amount": "0.25", "insertTime": T + 2, "txId": "eth-tx"}),
            json!({"id": "d-eur", "coin": "EUR", "status": 1, "amount": "100", "insertTime": T + 3, "txId": "sepa"}),
            json!({"id": "d-pending", "coin": "SOL", "network": "SOL", "status": 0, "amount": "1", "insertTime": T + 4, "txId": "sol-tx"}),
        ],
        withdrawals: vec![
            json!({"id": "w-usdt", "coin": "USDT", "network": "TRX", "status": 6, "amount": "50", "transactionFee": "1", "applyTime": T + 10, "txId": "trx-tx"}),
            // Binance's withdrawal status 1 is "cancelled".
            json!({"id": "w-cancelled", "coin": "ETH", "network": "ETH", "status": 1, "amount": "2", "transactionFee": "0.001", "applyTime": T + 11}),
        ],
        dividends: vec![
            json!({"id": 41, "tranId": 9041, "asset": "DOT", "amount": "0.25", "divTime": T + 20, "enInfo": "ETH 2.0 Staking"}),
            json!({"id": 42, "tranId": 9042, "asset": "BTC", "amount": "0.0000123", "divTime": T + 21, "enInfo": "BTC Mining Pool"}),
        ],
        converts: vec![
            convert("C1", "BTC", "0.01", "ETH", "0.2", T + 30),
            convert("C2", "EUR", "100", "SOL", "0.5", T + 31),
            convert("C3", "ETH", "0.1", "EUR", "180", T + 32),
            {
                let mut open = convert("C4", "BTC", "0.01", "ETH", "0.2", T + 33);
                open["orderStatus"] = json!("PROCESS");
                open
            },
        ],
        convert_page: 1000,
    }
}

#[test]
fn binance_bitcoin_scope_reads_btc_as_before_and_counts_other_assets() {
    let fake = binance(every_kind());
    let run = sync(&fake, "bitcoin", true);
    let result = run.result.unwrap();
    let seen = fake.seen.lock().unwrap().clone();
    let paths: Vec<_> = seen
        .iter()
        .map(|r| r.target.split('?').next().unwrap().to_owned())
        .collect();
    assert_eq!(
        paths,
        [
            "/sapi/v1/fiat/payments",
            "/sapi/v1/capital/deposit/hisrec",
            "/sapi/v1/capital/withdraw/history",
            "/sapi/v1/asset/assetDividend"
        ]
    );
    assert!(seen[1]
        .target
        .contains("coin=BTC&startTime=1498867200000&endTime="));
    // The fake ignores `coin`, so every coin's rows come back: BTC is kept,
    // the others are counted, fiat is neither.
    assert_eq!(
        dumped(at(&result, "assets")),
        r#"{"scope": "bitcoin", "skipped": {"DOT": 1, "ETH": 3, "USDT": 1}, "excluded": {}, "next_step": "kassiber wallets update --wallet 'Binance' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep DOT, ETH, USDT"}"#
    );
    assert_eq!(at(&result, "swap_pairs"), &Py::from(0));
    assert_eq!(at(&result, "imported"), &Py::from(3), "{result:?}");
    assert_eq!(
        count(
            &run.conn,
            "SELECT count(*) FROM transactions WHERE asset<>'BTC'"
        ),
        0
    );
    // The wallet holds BTC rows whose rail is "BTC"; turning the scope on
    // must still pass the wallet's network check.
    assets::Scope::Crypto.store(&run.conn, "p", "bw").unwrap();
}

#[test]
fn binance_crypto_scope_reads_every_coin_in_bounded_windows_only_after_consent() {
    let fake = binance(every_kind());
    // No consent, or offline: no request at all.
    let refused = sync(&fake, "crypto", false);
    assert_eq!(refused.result.unwrap_err().code, "interaction_required");
    let offline = loopback::run_prepared(
        &backend("binance", &fake.url),
        "binance",
        "",
        true,
        Arc::new(AtomicBool::new(true)),
        &wallet("crypto"),
    );
    assert!(offline.result.is_err());
    assert!(fake.seen.lock().unwrap().is_empty());

    let run = sync(&fake, "crypto", true);
    let result = run.result.unwrap();
    let seen = fake.seen.lock().unwrap().clone();
    // Every read is a signed GET (the fake refuses anything else).
    assert!(seen.iter().all(|r| r.signed), "{seen:?}");
    let now = now().unwrap().as_millis() as u64;
    let mut paths: Vec<&str> = vec![];
    for (path, span, limit) in [
        ("/sapi/v1/capital/deposit/hisrec", 89 * DAY, "1000"),
        ("/sapi/v1/capital/withdraw/history", 89 * DAY, "1000"),
        ("/sapi/v1/asset/assetDividend", 179 * DAY, "500"),
        ("/sapi/v1/convert/tradeFlow", 29 * DAY, "1000"),
    ] {
        let reads: Vec<_> = seen
            .iter()
            .filter(|r| r.target.starts_with(&format!("{path}?")))
            .collect();
        // Contiguous windows from Binance's launch to now, each within the
        // endpoint's limit, never naming a coin.
        let mut next = 1_498_867_200_000;
        for read in &reads {
            let (start, end) = (
                param(&read.target, "startTime").unwrap(),
                param(&read.target, "endTime").unwrap(),
            );
            assert_eq!(start, next, "{}", read.target);
            assert!(end - start < span, "{}", read.target);
            assert!(
                read.target.contains(&format!("limit={limit}")),
                "{}",
                read.target
            );
            assert!(!read.target.contains("coin="), "{}", read.target);
            next = end + 1;
        }
        assert!(next > now - 60_000 && next <= now + 60_000, "{path}");
        paths.push(path);
    }
    assert_eq!(
        seen.len(),
        1 + seen
            .iter()
            .filter(|r| paths.iter().any(|p| r.target.starts_with(p)))
            .count()
    );
    assert_eq!(at(&result, "imported"), &Py::from(12), "{result:?}");
    assert_eq!(at(&result, "swap_pairs"), &Py::from(1));
    assert_eq!(
        dumped(at(&result, "assets")),
        r#"{"scope": "crypto", "skipped": {}, "excluded": {}}"#
    );
    let conn = &run.conn;
    let unit = 100_000_000_000;
    assert_eq!(
        stored(conn, "binance:F2"),
        (
            "ETH".into(),
            "inbound".into(),
            "buy".into(),
            unit / 2,
            0,
            Some("1000".into())
        )
    );
    // BTC keys on its chain txid, as before; other coins on Binance's id,
    // since one chain transaction can carry several deposits.
    assert_eq!(stored(conn, "btc-tx").0, "BTC");
    assert_eq!(
        stored(conn, "binance:deposit:d-eth"),
        (
            "ETH".into(),
            "inbound".into(),
            "deposit".into(),
            15 * unit / 10,
            0,
            None
        )
    );
    assert_eq!(stored(conn, "binance:deposit:d-arb").3, unit / 4);
    assert_eq!(
        stored(conn, "binance:withdrawal:w-usdt"),
        (
            "USDT".into(),
            "outbound".into(),
            "withdrawal".into(),
            50 * unit,
            unit,
            None
        )
    );
    assert_eq!(
        stored(conn, "binance:41"),
        (
            "DOT".into(),
            "inbound".into(),
            "income".into(),
            unit / 4,
            0,
            None
        )
    );
    assert_eq!(stored(conn, "binance:42").2, "mining");
    // Convert: a swap pair between crypto assets, priced trades against fiat.
    assert_eq!(
        stored(conn, "binance:convert:C1:out"),
        (
            "BTC".into(),
            "outbound".into(),
            "swap".into(),
            unit / 100,
            0,
            None
        )
    );
    assert_eq!(
        stored(conn, "binance:convert:C1:in"),
        (
            "ETH".into(),
            "inbound".into(),
            "swap".into(),
            unit / 5,
            0,
            None
        )
    );
    assert_eq!(
        stored(conn, "binance:convert:C2"),
        (
            "SOL".into(),
            "inbound".into(),
            "buy".into(),
            unit / 2,
            0,
            Some("100".into())
        )
    );
    assert_eq!(
        stored(conn, "binance:convert:C3"),
        (
            "ETH".into(),
            "outbound".into(),
            "sell".into(),
            unit / 10,
            0,
            Some("180".into())
        )
    );
    // Fiat deposits, pending deposits, cancelled withdrawals and open
    // converts are not imported.
    for id in ["d-eur", "d-pending", "w-cancelled"] {
        assert_eq!(
            count(
                conn,
                &format!("SELECT count(*) FROM transactions WHERE raw_json LIKE '%\"{id}\"%'")
            ),
            0,
            "{id}"
        );
    }
    assert_eq!(
        count(
            conn,
            "SELECT count(*) FROM transactions WHERE external_id LIKE '%C4%'"
        ),
        0
    );

    // A re-sync stores nothing new and authors no second pair.
    let again = loopback::sync(
        conn,
        &backend("binance", &fake.url),
        "binance",
        "",
        true,
        online(),
    )
    .unwrap();
    assert_eq!(at(&again, "imported"), &Py::from(0), "{again:?}");
    assert_eq!(at(&again, "swap_pairs"), &Py::from(0));
}

#[test]
fn binance_full_windows_are_paged_or_split_and_fail_closed_when_they_cannot_be() {
    // 1,001 deposits in one window: a second page at offset 1000.
    let deposits: Vec<J> = (0..1001)
        .map(|i| json!({"id": format!("d{i}"), "coin": "ETH", "status": 1, "amount": "0.001", "insertTime": T + i}))
        .collect();
    // Converts a day apart, served one per page: each window is halved
    // until every part holds one.
    let converts: Vec<J> = (0..3)
        .map(|i| convert(&format!("C{i}"), "EUR", "10", "SOL", "0.1", T + i * DAY))
        .collect();
    let fake = binance(Histories {
        deposits,
        converts: converts.clone(),
        convert_page: 1,
        ..Default::default()
    });
    let run = sync(&fake, "crypto", true);
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(1004));
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen
        .iter()
        .any(|r| r.target.starts_with("/sapi/v1/capital/deposit/hisrec?")
            && r.target.contains("&offset=1000&")));
    let windows = seen
        .iter()
        .filter(|r| r.target.starts_with("/sapi/v1/convert/tradeFlow?"))
        .count();
    assert!(windows > 120, "{windows}");

    // Two converts in the same minute that never fit one page: refused.
    let crowded = vec![
        convert("X1", "EUR", "10", "SOL", "0.1", T),
        convert("X2", "EUR", "10", "SOL", "0.1", T + 1),
    ];
    let fake = binance(Histories {
        converts: crowded,
        convert_page: 1,
        ..Default::default()
    });
    let run = sync(&fake, "crypto", true);
    let error = run.result.unwrap_err();
    assert_eq!(error.code, "protocol_error");
    assert!(
        error
            .message
            .contains("more than 1000 rows within one minute"),
        "{}",
        error.message
    );
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

/// One `crypto`-scope sync that must be refused with `message`, storing
/// nothing.
fn refused(histories: Histories, message: &str) {
    let fake = binance(histories);
    let run = sync(&fake, "crypto", true);
    let error = run.result.unwrap_err();
    assert_eq!(error.code, "validation", "{message}: {error:?}");
    assert!(
        error.message.contains(message),
        "{message}: {}",
        error.message
    );
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

#[test]
fn binance_crypto_scope_refuses_rows_it_cannot_map_exactly() {
    let deposit = |coin: &str, amount: &str| Histories {
        deposits: vec![
            json!({"id": "d1", "coin": coin, "status": 1, "amount": amount, "insertTime": T}),
        ],
        ..Default::default()
    };
    refused(
        deposit("ET-H", "1"),
        "Binance deposit 'd1' has coin 'ET-H', which Kassiber cannot map exactly",
    );
    refused(
        Histories {
            deposits: vec![
                json!({"id": "d9", "coin": "ETH", "status": 9, "amount": "1", "insertTime": T}),
            ],
            ..Default::default()
        },
        "Binance deposit 'd9' has status '9', which Kassiber does not know",
    );
    refused(
        deposit("ETH", "0.000000000001"),
        "Binance deposit 'd1': ETH amount 1E-12 has more than 11 decimal places",
    );
    refused(
        Histories {
            dividends: vec![json!({"asset": "DOT", "amount": "1", "divTime": T})],
            ..Default::default()
        },
        "Binance DOT dividend has no id or tranId",
    );
    refused(
        Histories {
            converts: vec![{
                let mut row = convert("", "ETH", "1", "DOT", "50", T);
                row["orderId"] = J::Null;
                row
            }],
            convert_page: 1000,
            ..Default::default()
        },
        "Binance convert order has no orderId",
    );
    refused(
        Histories {
            converts: vec![convert("Z", "ETH", "0", "DOT", "50", T)],
            convert_page: 1000,
            ..Default::default()
        },
        "Binance convert 'Z' has no positive ETH amount",
    );
    refused(
        Histories {
            converts: vec![convert("Q", "ETH", "1", "D/OT", "50", T)],
            convert_page: 1000,
            ..Default::default()
        },
        "Binance convert 'Q' has coin 'D/OT'",
    );
}

#[test]
fn binance_converts_follow_the_scope() {
    let row = |value: J| to_py(&value).unwrap();
    let mut tally = assets::Tally::default();
    let bitcoin = assets::Scope::Bitcoin;
    // Bitcoin scope keeps BTC against fiat, counts other assets, and refuses
    // a swap between BTC and another asset rather than drop one side.
    let sold = binance::convert(
        &row(convert("C2", "BTC", "0.01", "EUR", "400", T)),
        bitcoin,
        &mut tally,
    )
    .unwrap();
    assert_eq!(sold.len(), 1);
    let other = binance::convert(
        &row(convert("C3", "ETH", "1", "DOT", "50", T)),
        bitcoin,
        &mut tally,
    )
    .unwrap();
    assert!(other.is_empty());
    assert_eq!(
        dumped(&tally.envelope(bitcoin, "B")),
        r#"{"scope": "bitcoin", "skipped": {"DOT": 1, "ETH": 1}, "excluded": {}, "next_step": "kassiber wallets update --wallet 'B' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep DOT, ETH"}"#
    );
    let error = binance::convert(
        &row(convert("C1", "BTC", "0.01", "ETH", "0.2", T)),
        bitcoin,
        &mut tally,
    )
    .unwrap_err();
    assert_eq!(error.message, "Binance convert 'C1' swaps BTC for ETH");
}
