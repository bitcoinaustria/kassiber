//! Book settings the engine computes: a generic book's own tax year and
//! zone, the per-wallet moving average, and the United States' change from
//! the global to the wallet pool on 2025-01-01
//! (`docs/reference/tax-engine.md`, `docs/reference/tax.md#book-settings`).

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

/// A UTC `ts` at `hour` of `date` (`YYYY-MM-DD`).
fn ts(date: &str, hour: i64) -> Value {
    let parts: Vec<i64> = date.split('-').map(|p| p.parse().unwrap()).collect();
    let us = (days(parts[0], parts[1], parts[2]) * 86_400 + hour * 3600) * 1_000_000;
    json!({
        "raw": format!("{date}T{hour:02}:00:00+00:00"),
        "us": us,
        "offset_s": 0,
        "date": date,
        "year": parts[0],
        "vienna_date": null,
        "display": format!("{date} {hour:02}:00:00+00:00"),
        "iso": format!("{date}T{hour:02}:00:00+00:00"),
    })
}

fn buy(row: i64, date: &str, wallet: &str, amount: &str, fiat: &str) -> Value {
    json!({"kind": "in", "row": row, "unique_id": format!("b{row}"), "asset": "BTC", "transaction_type": "BUY",
           "notes": null, "exchange": wallet, "holder": "H", "spot_price": "1E0", "crypto_in": amount,
           "fiat_in_no_fee": fiat, "fiat_in_with_fee": fiat, "fiat_fee": "0E0", "ts": ts(date, 12)})
}

fn sell_at(row: i64, ts: Value, wallet: &str, amount: &str, fiat: &str) -> Value {
    json!({"kind": "out", "row": row, "unique_id": format!("s{row}"), "asset": "BTC", "transaction_type": "SELL",
           "notes": null, "exchange": wallet, "holder": "H", "spot_price": "1E0", "crypto_out_no_fee": amount,
           "crypto_fee": "0E0", "fiat_out_no_fee": fiat, "fiat_fee": "0E0", "ts": ts})
}

fn sell(row: i64, date: &str, wallet: &str, amount: &str, fiat: &str) -> Value {
    sell_at(row, ts(date, 12), wallet, amount, fiat)
}

fn mv(row: i64, date: &str, from: &str, to: &str, sent: &str, received: &str, spot: &str) -> Value {
    json!({"kind": "intra", "row": row, "unique_id": format!("m{row}"), "asset": "BTC", "notes": null,
           "from_exchange": from, "from_holder": "H", "to_exchange": to, "to_holder": "H",
           "spot_price": spot, "crypto_sent": sent, "crypto_received": received, "ts": ts(date, 12)})
}

fn run(country: Value, method: &str, scope: &str, entries: Vec<Value>) -> Value {
    let request = json!({"schema_version": 1, "operation": "compute", "country": country,
                         "method": method, "pool_scope": scope,
                         "assets": [{"asset": "BTC", "entries": entries}]});
    serde_json::from_str(&compute(&request.to_string())).unwrap()
}

fn generic() -> Value {
    json!({"kind": "generic", "long_term_days": 365})
}

/// `(event row, lot row, amount, cost basis)` of each gain/loss.
fn fragments(response: &Value) -> Vec<(i64, Option<i64>, f64, f64)> {
    assert_eq!(response["ok"], true, "{response}");
    response["assets"][0]["gain_losses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gl| {
            (
                gl["event"]["row"].as_i64().unwrap(),
                gl["lot"].as_i64(),
                gl["crypto_amount"].as_str().unwrap().parse().unwrap(),
                gl["fiat_cost_basis"].as_str().unwrap().parse().unwrap(),
            )
        })
        .collect()
}

/// `(wallet, quantity, cost basis)` of each remaining holding.
fn holdings(response: &Value) -> Vec<(String, f64, f64)> {
    response["assets"][0]["wallet_holdings"]
        .as_array()
        .expect("wallet holdings")
        .iter()
        .map(|h| {
            (
                h["exchange"].as_str().unwrap().to_owned(),
                h["quantity"].as_str().unwrap().parse().unwrap(),
                h["cost_basis"].as_str().unwrap().parse().unwrap(),
            )
        })
        .collect()
}

fn years(response: &Value) -> Vec<i64> {
    response["assets"][0]["yearly"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["transaction_type"] == "SELL" || row["transaction_type"] == "sell")
        .map(|row| row["year"].as_i64().unwrap())
        .collect()
}

#[test]
fn a_generic_books_own_tax_year_cuts_in_its_zone() {
    // 2024-06-30T15:00Z is 1 July 01:00 in Sydney: the 2024-25 year that
    // starts on 1 July; 13:00Z is still 30 June 23:00, the 2023-24 year.
    let book = |hour| {
        vec![
            buy(1, "2024-01-02", "A", "2E0", "100E0"),
            sell_at(2, ts("2024-06-30", hour), "A", "1E0", "80E0"),
        ]
    };
    let australian = json!({"kind": "generic", "long_term_days": 365, "tax_year_start": "07-01", "timezone": "Australia/Sydney"});
    assert_eq!(
        years(&run(australian.clone(), "fifo", "global", book(15))),
        [2024]
    );
    assert_eq!(years(&run(australian, "fifo", "global", book(13))), [2023]);
    // Without either field the calendar year in the timestamp's offset.
    assert_eq!(years(&run(generic(), "fifo", "global", book(15))), [2024]);
    let april = json!({"kind": "generic", "long_term_days": 365, "tax_year_start": "04-06"});
    let response = run(
        april,
        "fifo",
        "global",
        vec![
            buy(1, "2024-01-02", "A", "2E0", "100E0"),
            sell(2, "2025-04-05", "A", "1E0", "80E0"),
        ],
    );
    assert_eq!(years(&response), [2024]);
    // The holding period stays whole days: neither field changes it.
    let held = run(
        json!({"kind": "generic", "long_term_days": 365, "timezone": "Pacific/Auckland"}),
        "fifo",
        "global",
        vec![
            buy(1, "2024-01-02", "A", "1E0", "100E0"),
            sell(2, "2025-01-01", "A", "1E0", "80E0"),
        ],
    );
    assert_eq!(
        held["assets"][0]["gain_losses"][0]["long_term"], true,
        "{held}"
    );
    // A day that is not one every year, and a zone without a calendar, are
    // request errors.
    for country in [
        json!({"kind": "generic", "long_term_days": 365, "tax_year_start": "02-29"}),
        json!({"kind": "generic", "long_term_days": 365, "timezone": "Mars/Olympus"}),
    ] {
        let refused = run(country, "fifo", "global", book(15));
        assert_eq!(refused["ok"], false, "{refused}");
    }
}

#[test]
fn the_wallet_moving_average_keeps_each_wallets_own_average() {
    // A buys 1 at 100 and 1 at 300 (average 200); B buys 1 at 1000; A moves
    // one unit to B, carrying A's average: B holds 2 at 1200 (average 600).
    let response = run(
        generic(),
        "moving_average",
        "wallet",
        vec![
            buy(1, "2025-01-02", "A", "1E0", "100E0"),
            buy(2, "2025-01-03", "A", "1E0", "300E0"),
            buy(3, "2025-01-04", "B", "1E0", "1000E0"),
            mv(4, "2025-02-01", "A", "B", "1E0", "1E0", "0E0"),
            sell(5, "2025-03-01", "B", "1E0", "700E0"),
        ],
    );
    // The sale takes B's FIFO portion (lot 1, moved from A) at B's average.
    assert_eq!(fragments(&response), [(5, Some(1), 1.0, 600.0)]);
    assert_eq!(
        holdings(&response),
        [("A".into(), 1.0, 200.0), ("B".into(), 1.0, 600.0)]
    );
    // The global pool's average is the book's: (100 + 300 + 1000) / 3.
    let global = run(
        generic(),
        "moving_average",
        "global",
        vec![
            buy(1, "2025-01-02", "A", "1E0", "100E0"),
            buy(2, "2025-01-03", "A", "1E0", "300E0"),
            buy(3, "2025-01-04", "B", "1E0", "1000E0"),
            sell(5, "2025-03-01", "B", "1E0", "700E0"),
        ],
    );
    let (_, _, _, basis) = fragments(&global)[0];
    assert!((basis - 1400.0 / 3.0).abs() < 1e-6, "{basis}");
}

#[test]
fn a_wallet_moving_average_moves_fee_is_a_disposal_at_the_sources_average() {
    let response = run(
        generic(),
        "moving_average",
        "wallet",
        vec![
            buy(1, "2025-01-02", "A", "1E0", "100E0"),
            buy(2, "2025-01-03", "A", "1E0", "300E0"),
            mv(3, "2025-02-01", "A", "B", "1E0", "9E-1", "500E0"),
        ],
    );
    // The fee (0.1) leaves A at A's average of 200 per unit; the 0.9 that
    // arrive carry 180 to B.
    assert_eq!(fragments(&response), [(3, Some(1), 0.1, 20.0)]);
    assert_eq!(
        holdings(&response),
        [("A".into(), 1.0, 200.0), ("B".into(), 0.9, 180.0)]
    );
    // Other pool methods still have no per-wallet form.
    let refused = run(
        json!({"kind": "at"}),
        "moving_average_at",
        "wallet",
        vec![buy(1, "2025-01-02", "A", "1E0", "1E0")],
    );
    assert_eq!(refused["ok"], false);
}

/// Two lots bought in A in 2024, one unit moved to B and half a unit sold
/// from A in 2024 (the universal pool sells lot 1), then a sale from B in
/// 2025.
fn transition_book() -> Vec<Value> {
    vec![
        buy(1, "2024-01-02", "A", "1E0", "100E0"),
        buy(2, "2024-02-01", "A", "1E0", "200E0"),
        mv(3, "2024-03-01", "A", "B", "1E0", "1E0", "0E0"),
        sell(4, "2024-06-01", "A", "5E-1", "150E0"),
        sell(5, "2025-02-01", "B", "1E0", "400E0"),
    ]
}

#[test]
fn the_us_transition_allocates_the_global_pool_to_wallets_on_2025_01_01() {
    let us = json!({"kind": "us"});
    let response = run(us.clone(), "fifo", "global_then_wallet", transition_book());
    // 2024: the universal pool's FIFO takes half of lot 1. On 2025-01-01 A
    // holds 0.5 and B 1; FIFO allocates the earliest remaining lot first, A
    // before B: A gets lot 1's 0.5, B lot 2's 1, so B's sale takes lot 2.
    assert_eq!(
        fragments(&response),
        [(4, Some(1), 0.5, 50.0), (5, Some(2), 1.0, 200.0)]
    );
    assert_eq!(holdings(&response), [("A".into(), 0.5, 50.0)]);
    // The wallet scope from the start carries lot 1 to B with the move.
    let wallet = run(us.clone(), "fifo", "wallet", transition_book());
    assert_eq!(fragments(&wallet)[1], (5, Some(1), 1.0, 100.0));
    // LIFO orders the allocation by the latest acquisition: A gets lot 2.
    let lifo = run(us.clone(), "lifo", "global_then_wallet", transition_book());
    assert_eq!(
        fragments(&lifo),
        [(4, Some(2), 0.5, 100.0), (5, Some(1), 1.0, 100.0)]
    );
    assert_eq!(holdings(&lifo), [("A".into(), 0.5, 100.0)]);
    // A book that ends in 2024 shows the allocation the rule will make.
    let ended = run(
        us.clone(),
        "fifo",
        "global_then_wallet",
        transition_book()[..4].to_vec(),
    );
    assert_eq!(
        holdings(&ended),
        [("A".into(), 0.5, 50.0), ("B".into(), 1.0, 200.0)]
    );
}

/// The allocation fills wallets in the book's own order (when they were
/// added), which a rename does not change; labels order only wallets the
/// order does not name.
#[test]
fn the_us_transition_fills_wallets_in_the_books_order_not_by_label() {
    let ordered = |order: Value| {
        let request = json!({"schema_version": 1, "operation": "compute", "country": {"kind": "us"},
                             "method": "fifo", "pool_scope": "global_then_wallet", "wallet_order": order,
                             "assets": [{"asset": "BTC", "entries": transition_book()}]});
        serde_json::from_str::<Value>(&compute(&request.to_string())).unwrap()
    };
    // B was added first: it gets lot 1's remaining 0.5 and half of lot 2.
    let response = ordered(json!(["B", "A"]));
    assert_eq!(
        fragments(&response),
        [
            (4, Some(1), 0.5, 50.0),
            (5, Some(1), 0.5, 50.0),
            (5, Some(2), 0.5, 100.0)
        ]
    );
    assert_eq!(holdings(&response), [("A".into(), 0.5, 100.0)]);
    // A wallet the order does not name follows the named ones.
    assert_eq!(fragments(&ordered(json!(["B"]))), fragments(&response));
    // In the order of their labels, as without one.
    assert_eq!(
        fragments(&ordered(json!(["A", "B"]))),
        fragments(&run(
            json!({"kind": "us"}),
            "fifo",
            "global_then_wallet",
            transition_book()
        ))
    );
}

#[test]
fn the_us_transition_fails_closed() {
    // Only the United States has the change, and only for its lot methods.
    let refused = run(generic(), "fifo", "global_then_wallet", transition_book());
    assert_eq!(refused["ok"], false);
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no such change"));
    let refused = run(
        json!({"kind": "us"}),
        "moving_average",
        "global_then_wallet",
        transition_book(),
    );
    assert_eq!(refused["ok"], false);
    // After the change a wallet still cannot sell another wallet's units.
    let refused = run(
        json!({"kind": "us"}),
        "fifo",
        "global_then_wallet",
        vec![
            buy(1, "2024-01-02", "A", "1E0", "100E0"),
            buy(2, "2024-01-02", "B", "1E0", "100E0"),
            sell(3, "2025-02-01", "A", "15E-1", "300E0"),
        ],
    );
    assert_eq!(refused["ok"], false);
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("in wallet A"));
}
