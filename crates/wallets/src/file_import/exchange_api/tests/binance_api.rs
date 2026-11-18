//! The Binance API sync over the signed loopback fake: Binance reads only
//! BTC through its API, in either asset scope (docs/reference/imports.md#binance-api-and-supplemental-csv).
//! The fake answers each history within the query's time window, as Binance
//! does, and records every read.
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
    /// The spot account's `balances`.
    balances: Vec<J>,
    /// Rows per fiat payment page and per dividend page, when Binance
    /// serves fewer than the query asks for (0: as many as asked).
    fiat_page: usize,
    dividend_page: usize,
    /// Leave `total` out of the fiat payment and dividend answers.
    no_total: bool,
}

fn param(target: &str, key: &str) -> Option<u64> {
    target
        .split(['?', '&'])
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
        .and_then(|v| v.parse().ok())
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
            "/sapi/v1/fiat/payments" => {
                let rows = histories.fiat.len();
                let size = param(target, "rows").unwrap_or(100) as usize;
                let size = match histories.fiat_page {
                    0 => size,
                    page => size.min(page),
                };
                let page = param(target, "page").unwrap_or(1) as usize;
                let data: Vec<J> = histories
                    .fiat
                    .iter()
                    .skip((page - 1) * size)
                    .take(size)
                    .cloned()
                    .collect();
                match histories.no_total {
                    true => json!({"data": data}),
                    false => json!({"data": data, "total": rows}),
                }
            }
            "/sapi/v1/capital/deposit/hisrec" => {
                J::Array(paged(within(&histories.deposits, "insertTime")))
            }
            "/sapi/v1/capital/withdraw/history" => {
                J::Array(paged(within(&histories.withdrawals, "applyTime")))
            }
            "/sapi/v1/asset/assetDividend" => {
                // One page of at most `limit` rows, as Binance answers.
                let mut rows = within(&histories.dividends, "divTime");
                let total = rows.len();
                let size = param(target, "limit").unwrap_or(500) as usize;
                rows.truncate(match histories.dividend_page {
                    0 => size,
                    page => size.min(page),
                });
                match histories.no_total {
                    true => json!({"rows": rows}),
                    false => json!({"rows": rows, "total": total}),
                }
            }
            "/sapi/v1/convert/tradeFlow" => {
                let rows = within(&histories.converts, "createTime");
                let page = histories.convert_page.max(1000);
                let page = if histories.convert_page > 0 {
                    histories.convert_page
                } else {
                    page
                };
                let more = rows.len() > page;
                json!({"list": rows.into_iter().take(page).collect::<Vec<_>>(), "moreData": more})
            }
            "/api/v3/account" => json!({"balances": histories.balances}),
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

fn convert(order: &str, from: &str, from_amount: &str, to: &str, to_amount: &str, at: u64) -> J {
    json!({"quoteId": format!("q{order}"), "orderId": order, "orderStatus": "SUCCESS",
        "fromAsset": from, "fromAmount": from_amount, "toAsset": to, "toAmount": to_amount,
        "ratio": "1", "inverseRatio": "1", "createTime": at})
}

fn every_kind() -> Histories {
    Histories {
        fiat: vec![
            json!({"orderNo": "F1", "status": "Completed", "cryptoCurrency": "BTC", "fiatCurrency": "EUR",
                "obtainAmount": "0.01", "sourceAmount": "-400", "createTime": T}),
            json!({"orderNo": "F2", "status": "Completed", "cryptoCurrency": "ETH", "fiatCurrency": "EUR",
                "obtainAmount": "1", "sourceAmount": "-2000", "createTime": T}),
        ],
        deposits: vec![
            json!({"coin": "BTC", "status": 1, "amount": "0.02", "insertTime": T + DAY, "txId": "d1"}),
        ],
        withdrawals: vec![
            json!({"coin": "BTC", "status": 6, "amount": "0.005", "transactionFee": "0.0001", "applyTime": T + 2 * DAY, "id": "w1"}),
        ],
        dividends: vec![
            json!({"id": 7, "asset": "BTC", "amount": "0.0001", "divTime": T + 3 * DAY, "enInfo": "Simple Earn"}),
        ],
        converts: vec![
            convert("C1", "EUR", "100", "BTC", "0.0025", T + 4 * DAY),
            convert("C2", "ETH", "1", "USDT", "2000", T + 5 * DAY),
            json!({"orderId": "C3", "orderStatus": "PROCESS", "fromAsset": "BTC", "fromAmount": "1",
                "toAsset": "ETH", "toAmount": "20", "createTime": T + 6 * DAY}),
        ],
        convert_page: 0,
        balances: vec![json!({"asset": "BTC", "free": "0.0275", "locked": "0"})],
        ..Histories::default()
    }
}

/// Both scopes read the same BTC histories and Convert orders and book the
/// same rows; other coins are counted as unread, and every sync names the
/// spot trades the API never reads. Without consent nothing is requested.
#[test]
fn binance_reads_the_same_btc_histories_in_either_scope() {
    let fake = binance(every_kind());
    let refused = sync(&fake, "bitcoin", false);
    assert!(refused.result.is_err());
    assert!(fake.seen.lock().unwrap().is_empty());

    let mut stored = vec![];
    for scope in ["bitcoin", "crypto"] {
        let fake = binance(every_kind());
        let run = sync(&fake, scope, true);
        let result = run.result.unwrap();
        assert_eq!(at(&result, "imported"), &Py::from(5), "{result:?}");
        let assets = dumped(at(&result, "assets"));
        assert!(
            assets.starts_with(&format!(
                r#"{{"scope": "{scope}", "skipped": {{}}, "excluded": {{}}, "unread": {{"ETH": 2, "USDT": 1}}"#
            )),
            "{assets}"
        );
        assert!(
            assets.contains("does not read Binance spot trades"),
            "{assets}"
        );
        assert_eq!(
            dumped(at(&result, "provider_balance")),
            r#"{"status": "matches", "asset": "BTC", "provider_msat": 2750000000, "book_msat": 2750000000, "difference_msat": 0}"#
        );
        let mut rows: Vec<(String, String, i64)> = run
            .conn
            .prepare("SELECT external_id, kind, amount FROM transactions ORDER BY external_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.sort();
        stored.push(rows);
    }
    assert_eq!(stored[0], stored[1]);
    let kinds: Vec<&str> = stored[0].iter().map(|r| r.1.as_str()).collect();
    assert_eq!(kinds, ["income", "buy", "buy", "deposit", "withdrawal"]);
}

/// A Convert between BTC and another coin cannot be booked from the API in
/// either scope; the sync is refused and names the CSV export.
#[test]
fn binance_btc_swap_convert_is_refused_in_either_scope() {
    for scope in ["bitcoin", "crypto"] {
        let fake = binance(Histories {
            converts: vec![convert("C9", "BTC", "0.01", "ETH", "0.2", T)],
            ..Default::default()
        });
        let run = sync(&fake, scope, true);
        let error = run.result.unwrap_err();
        assert_eq!(error.message, "Binance convert 'C9' swaps BTC for ETH");
        assert!(
            error
                .hint
                .as_deref()
                .unwrap()
                .contains("Transaction History CSV"),
            "{error:?}"
        );
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
}

#[test]
fn binance_full_windows_are_paged_or_split_and_fail_closed_when_they_cannot_be() {
    // 1,001 deposits in one window: a second page at offset 1000.
    let deposits: Vec<J> = (0..1001)
        .map(|i| json!({"id": format!("d{i}"), "txId": format!("t{i}"), "coin": "BTC", "status": 1, "amount": "0.001", "insertTime": T + i}))
        .collect();
    // Converts a day apart, served one per page: each window is halved
    // until every part holds one.
    let converts: Vec<J> = (0..3)
        .map(|i| convert(&format!("C{i}"), "EUR", "10", "BTC", "0.0002", T + i * DAY))
        .collect();
    let fake = binance(Histories {
        deposits,
        converts,
        convert_page: 1,
        ..Default::default()
    });
    let run = sync(&fake, "bitcoin", true);
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
    let fake = binance(Histories {
        converts: vec![
            convert("X1", "EUR", "10", "BTC", "0.0002", T),
            convert("X2", "EUR", "10", "BTC", "0.0002", T + 1),
        ],
        convert_page: 1,
        ..Default::default()
    });
    let run = sync(&fake, "bitcoin", true);
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

/// The spot balance is whole msat by its own digits or the sync fails closed.
#[test]
fn binance_balance_must_be_exact() {
    for (free, message) in [
        (
            "0.000000000001",
            "Binance BTC balance is finer than a millisatoshi",
        ),
        ("1e-3", "Binance BTC balance is not a plain decimal"),
    ] {
        let fake = binance(Histories {
            balances: vec![json!({"asset": "BTC", "free": free, "locked": "0"})],
            ..Default::default()
        });
        let run = sync(&fake, "bitcoin", true);
        let error = run.result.unwrap_err();
        assert_eq!(error.message, message);
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
}

/// Fiat payments are read page by page until Binance's `total`, and a
/// dividend window that holds more than its answer returned is split: a
/// short page never ends a read early.
#[test]
fn binance_reads_every_fiat_payment_and_dividend_past_a_short_page() {
    let fiat = (0..5_u64)
        .map(|i| json!({"orderNo": format!("F{i}"), "status": "Completed", "cryptoCurrency": "BTC", "fiatCurrency": "EUR",
            "obtainAmount": "0.001", "sourceAmount": "-40", "createTime": T + i}))
        .collect();
    let dividends = (0..30_u64)
        .map(|i| json!({"id": i, "asset": "BTC", "amount": "0.00001", "divTime": T - i * DAY, "enInfo": "Simple Earn"}))
        .collect();
    let fake = binance(Histories {
        fiat,
        dividends,
        fiat_page: 2,
        dividend_page: 10,
        ..Histories::default()
    });
    let run = sync(&fake, "bitcoin", true);
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(35));
}

/// An answer without its paging total refuses: Kassiber cannot tell
/// whether it holds every row.
#[test]
fn binance_refuses_a_page_without_its_total() {
    for histories in [
        Histories {
            no_total: true,
            ..Histories::default()
        },
        Histories {
            no_total: true,
            dividends: vec![
                json!({"id": 1, "asset": "BTC", "amount": "0.00001", "divTime": T, "enInfo": "Simple Earn"}),
            ],
            ..Histories::default()
        },
    ] {
        let run = sync(&binance(histories), "bitcoin", true);
        let error = run.result.unwrap_err();
        assert_eq!(error.code, "protocol_error", "{error:?}");
        assert!(error.message.contains("total"), "{}", error.message);
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
}

/// Deposit and withdrawal statuses are Binance's own per direction: a
/// cancelled withdrawal (1) or a pending deposit (0) books nothing, and a
/// status Binance does not document refuses.
#[test]
fn binance_books_only_settled_deposits_and_withdrawals() {
    let fake = binance(Histories {
        deposits: vec![
            json!({"coin": "BTC", "status": 1, "amount": "0.02", "insertTime": T, "txId": "settled-in"}),
            json!({"coin": "BTC", "status": 0, "amount": "0.03", "insertTime": T + 1, "txId": "pending-in"}),
        ],
        withdrawals: vec![
            json!({"coin": "BTC", "status": 6, "amount": "0.005", "transactionFee": "0.0001", "applyTime": T + 2, "id": "w-done"}),
            json!({"coin": "BTC", "status": 1, "amount": "0.004", "transactionFee": "0.0001", "applyTime": T + 3, "id": "w-cancelled"}),
        ],
        ..Histories::default()
    });
    let run = sync(&fake, "bitcoin", true);
    run.result.unwrap();
    let ids: Vec<String> = run
        .conn
        .prepare("SELECT external_id FROM transactions ORDER BY external_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids, ["settled-in", "w-done"]);
    for (deposits, withdrawals) in [
        (
            vec![
                json!({"coin": "BTC", "status": 9, "amount": "0.02", "insertTime": T, "txId": "odd"}),
            ],
            vec![],
        ),
        (
            vec![],
            vec![
                json!({"coin": "BTC", "status": 7, "amount": "0.02", "applyTime": T, "id": "odd"}),
            ],
        ),
        (
            vec![
                json!({"coin": "BTC", "status": "confirming", "amount": "0.02", "insertTime": T, "txId": "odd"}),
            ],
            vec![],
        ),
    ] {
        let run = sync(
            &binance(Histories {
                deposits,
                withdrawals,
                ..Histories::default()
            }),
            "bitcoin",
            true,
        );
        let error = run.result.unwrap_err();
        assert!(error.message.contains("status"), "{}", error.message);
        assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
    }
}
