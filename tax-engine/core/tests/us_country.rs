//! The United States country and the wallet pool scope, case by case.
//! `us_fixtures.rs` compares them with RP2 over generated histories; these
//! pin what RP2 cannot show (docs/reference/tax-country-us.md).

use kassiber_tax_core::compute;
use serde_json::{json, Value};

/// Days from 1970-01-01 to `y-m-d`.
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// A UTC `ts` at noon of `date` (`YYYY-MM-DD`), or `hour` given.
fn ts(date: &str, hour: i64, offset_h: i64) -> Value {
    let parts: Vec<i64> = date.split('-').map(|p| p.parse().unwrap()).collect();
    let local = days(parts[0], parts[1], parts[2]) * 86_400 + hour * 3600;
    let us = (local - offset_h * 3600) * 1_000_000;
    let offset = format!(
        "{}{:02}:00",
        if offset_h < 0 { '-' } else { '+' },
        offset_h.abs()
    );
    json!({
        "raw": format!("{date}T{hour:02}:00:00{offset}"),
        "us": us,
        "offset_s": offset_h * 3600,
        "date": date,
        "year": parts[0],
        "vienna_date": null,
        "display": format!("{date} {hour:02}:00:00{offset}"),
        "iso": format!("{date}T{hour:02}:00:00{offset}"),
    })
}

fn buy(row: i64, date: &str, wallet: &str, amount: &str, fiat: &str) -> Value {
    json!({"kind": "in", "row": row, "unique_id": format!("b{row}"), "asset": "BTC", "transaction_type": "BUY",
           "notes": null, "exchange": wallet, "holder": "H", "spot_price": "1E0", "crypto_in": amount,
           "fiat_in_no_fee": fiat, "fiat_in_with_fee": fiat, "fiat_fee": "0E0", "ts": ts(date, 12, 0)})
}

fn sell(row: i64, date: &str, wallet: &str, amount: &str, fiat: &str) -> Value {
    json!({"kind": "out", "row": row, "unique_id": format!("s{row}"), "asset": "BTC", "transaction_type": "SELL",
           "notes": null, "exchange": wallet, "holder": "H", "spot_price": "1E0", "crypto_out_no_fee": amount,
           "crypto_fee": "0E0", "fiat_out_no_fee": fiat, "fiat_fee": "0E0", "ts": ts(date, 12, 0)})
}

fn mv(row: i64, date: &str, from: &str, to: &str, sent: &str, received: &str, spot: &str) -> Value {
    json!({"kind": "intra", "row": row, "unique_id": format!("m{row}"), "asset": "BTC", "notes": null,
           "from_exchange": from, "from_holder": "H", "to_exchange": to, "to_holder": "H",
           "spot_price": spot, "crypto_sent": sent, "crypto_received": received, "ts": ts(date, 12, 0)})
}

fn run(method: &str, scope: &str, entries: Vec<Value>) -> Value {
    let request = json!({"schema_version": 1, "operation": "compute", "country": {"kind": "us"},
                         "method": method, "pool_scope": scope,
                         "assets": [{"asset": "BTC", "entries": entries}]});
    serde_json::from_str(&compute(&request.to_string())).unwrap()
}

/// `(event row, lot row, amount, long_term)` of each gain/loss.
fn lots(response: &Value) -> Vec<(i64, Option<i64>, String, bool)> {
    assert_eq!(response["ok"], true, "{response}");
    response["assets"][0]["gain_losses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gl| {
            (
                gl["event"]["row"].as_i64().unwrap(),
                gl["lot"].as_i64(),
                gl["crypto_amount"].as_str().unwrap().to_owned(),
                gl["long_term"].as_bool().unwrap(),
            )
        })
        .collect()
}

/// Two lots in A, one moved to B, a sale in each: the universal pass and
/// RP2's per-wallet engine sell lot 1 twice in spirit; the wallet scope
/// sells lot 2 in A and lot 1 in B.
fn moved_lot_book() -> Vec<Value> {
    vec![
        buy(1, "2025-01-02", "A", "1E0", "100000E0"),
        buy(2, "2025-02-01", "A", "1E0", "200000E0"),
        mv(3, "2025-03-01", "A", "B", "1E0", "1E0", "0E0"),
        sell(4, "2025-04-01", "A", "1E0", "400000E0"),
        sell(5, "2025-05-01", "B", "1E0", "400000E0"),
    ]
}

#[test]
fn the_wallet_scope_disposes_only_of_the_disposing_wallets_lots() {
    assert_eq!(
        lots(&run("fifo", "wallet", moved_lot_book())),
        vec![
            (4, Some(2), "1E0".into(), false),
            (5, Some(1), "1E0".into(), false)
        ]
    );
    // The global scope sells lot 1 first, wherever it sits.
    let universal: Vec<Value> = moved_lot_book()
        .into_iter()
        .map(|mut entry| {
            // Before 2025, when the global scope may still dispose.
            entry["ts"] = ts("2024-06-01", 12, 0);
            entry
        })
        .collect();
    let response = run("fifo", "global", universal);
    assert_eq!(lots(&response)[0].1, Some(1));
}

#[test]
fn lifo_hifo_and_lofo_order_a_wallets_portions_by_their_lots() {
    let book = |method| {
        run(
            method,
            "wallet",
            vec![
                buy(1, "2025-01-02", "A", "1E0", "300E0"),
                buy(2, "2025-01-03", "A", "1E0", "100E0"),
                buy(3, "2025-01-04", "B", "1E0", "200E0"),
                mv(4, "2025-01-05", "B", "A", "1E0", "1E0", "0E0"),
                sell(5, "2025-02-01", "A", "1E0", "500E0"),
            ],
        )
    };
    // The moved lot (3) keeps its acquisition: latest for LIFO although it
    // arrived last too, middle cost for HIFO and LOFO.
    assert_eq!(lots(&book("lifo"))[0].1, Some(3));
    assert_eq!(lots(&book("hifo"))[0].1, Some(1));
    assert_eq!(lots(&book("lofo"))[0].1, Some(2));
    let fifo = run(
        "fifo",
        "wallet",
        vec![
            buy(1, "2025-01-02", "B", "1E0", "300E0"),
            buy(2, "2025-01-03", "A", "1E0", "100E0"),
            mv(3, "2025-01-05", "B", "A", "1E0", "1E0", "0E0"),
            sell(4, "2025-02-01", "A", "1E0", "500E0"),
        ],
    );
    // FIFO by acquisition, not by arrival (RP2's per-wallet lots arrive
    // dated at the move).
    assert_eq!(lots(&fifo)[0].1, Some(1));
}

#[test]
fn a_moves_fee_leaves_first_then_its_principal() {
    let response = run(
        "fifo",
        "wallet",
        vec![
            buy(1, "2024-01-02", "A", "1E0", "100E0"),
            buy(2, "2024-01-03", "A", "1E0", "300E0"),
            mv(3, "2024-02-01", "A", "B", "15E-1", "14E-1", "1000E0"),
            sell(4, "2024-03-01", "B", "14E-1", "1400E0"),
        ],
    );
    // The 0.1 fee comes from lot 1 first; the 1.4 principal is lot 1's
    // remaining 0.9 and 0.5 of lot 2.
    assert_eq!(
        lots(&response),
        vec![
            (3, Some(1), "1E-1".into(), false),
            (4, Some(1), "9E-1".into(), false),
            (4, Some(2), "5E-1".into(), false),
        ]
    );
    let holdings = &response["assets"][0]["wallet_holdings"];
    assert_eq!(
        holdings,
        &json!([{"exchange": "A", "holder": "H", "quantity": "5E-1", "cost_basis": "1500E-1"}])
    );
}

#[test]
fn a_same_instant_move_funds_a_sale_at_its_destination() {
    let response = run(
        "fifo",
        "wallet",
        vec![
            buy(1, "2025-01-02", "A", "1E0", "100E0"),
            sell(2, "2025-02-01", "B", "5E-1", "80E0"),
            mv(3, "2025-02-01", "A", "B", "5E-1", "5E-1", "0E0"),
        ],
    );
    assert_eq!(lots(&response), vec![(2, Some(1), "5E-1".into(), false)]);
}

#[test]
fn a_lot_moved_away_and_back_is_one_portion() {
    let response = run(
        "fifo",
        "wallet",
        vec![
            buy(1, "2025-01-02", "A", "1E0", "100E0"),
            buy(2, "2025-01-03", "A", "1E0", "200E0"),
            mv(3, "2025-02-01", "A", "B", "5E-1", "5E-1", "0E0"),
            mv(4, "2025-03-01", "B", "A", "5E-1", "5E-1", "0E0"),
            sell(5, "2025-04-01", "A", "12E-1", "300E0"),
        ],
    );
    assert_eq!(
        lots(&response),
        vec![
            (5, Some(1), "10E-1".into(), false),
            (5, Some(2), "2E-1".into(), false)
        ]
    );
}

#[test]
fn the_wallet_scope_fails_closed() {
    // A wallet without the units, although another wallet has them.
    let response = run(
        "fifo",
        "wallet",
        vec![
            buy(1, "2025-01-02", "A", "1E0", "100E0"),
            buy(2, "2025-01-02", "B", "1E0", "100E0"),
            sell(3, "2025-02-01", "A", "15E-1", "300E0"),
        ],
    );
    assert_eq!(response["ok"], false);
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("in wallet A"));
    // A moving average has no per-wallet form.
    let response = run(
        "moving_average",
        "wallet",
        vec![buy(1, "2025-01-02", "A", "1E0", "1E0")],
    );
    assert_eq!(response["ok"], false);
    // A move fee without a price would lose its basis.
    let response = run(
        "fifo",
        "wallet",
        vec![
            buy(1, "2025-01-02", "A", "1E0", "100E0"),
            // A price so small the fee's value rounds to nothing at 13 places.
            mv(2, "2025-02-01", "A", "B", "1E0", "999E-3", "1E-12"),
        ],
    );
    assert_eq!(response["ok"], false);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("needs a price"),
        "{response}"
    );
}

#[test]
fn the_global_scope_ends_with_2024() {
    let book = |date: &str| {
        run(
            "fifo",
            "global",
            vec![
                buy(1, "2024-01-02", "A", "1E0", "100E0"),
                sell(2, date, "A", "5E-1", "80E0"),
            ],
        )
    };
    assert_eq!(book("2024-12-31")["ok"], true);
    let refused = book("2025-01-01");
    assert_eq!(refused["ok"], false);
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("per wallet from 2025"));
    // An earn receipt is no disposal.
    let income = run(
        "fifo",
        "global",
        vec![json!({
        "kind": "in", "row": 1, "unique_id": "i", "asset": "BTC", "transaction_type": "STAKING", "notes": null,
        "exchange": "A", "holder": "H", "spot_price": "1E0", "crypto_in": "1E0", "fiat_in_no_fee": "10E0",
        "fiat_in_with_fee": "10E0", "fiat_fee": "0E0", "ts": ts("2025-06-01", 12, 0)})],
    );
    assert_eq!(income["ok"], true, "{income}");
}

#[test]
fn the_holding_period_counts_utc_calendar_dates() {
    let term = |acquired: Value, sold: Value| {
        let mut b = buy(1, "2024-06-17", "A", "1E0", "100E0");
        b["ts"] = acquired;
        let mut s = sell(2, "2025-06-17", "A", "1E0", "200E0");
        s["ts"] = sold;
        lots(&run("fifo", "wallet", vec![b, s]))[0].3
    };
    // Publication 544: the anniversary is not more than a year.
    assert!(!term(ts("2024-06-17", 12, 0), ts("2025-06-17", 12, 0)));
    assert!(term(ts("2024-06-17", 12, 0), ts("2025-06-18", 12, 0)));
    // UTC dates: an evening purchase in New York is the next UTC day.
    assert!(!term(ts("2024-06-17", 22, -5), ts("2025-06-18", 10, -5)));
    // 29 February: 28 February is the anniversary.
    assert!(!term(ts("2024-02-29", 12, 0), ts("2025-02-28", 12, 0)));
    assert!(term(ts("2024-02-29", 12, 0), ts("2025-03-01", 12, 0)));
}
