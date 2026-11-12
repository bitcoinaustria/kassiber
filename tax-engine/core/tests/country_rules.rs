//! The shared country rules (`{"kind": "rules"}`), one feature at a time,
//! through the schema-1 lot engine. `docs/reference/tax-countries.md` lists
//! which country uses which; the worked cases are the countries' vectors.

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

/// The `ts` of a UTC time `YYYY-MM-DDTHH:MM`.
fn ts(utc: &str) -> Value {
    let field = |range: std::ops::Range<usize>| utc[range].parse::<i64>().unwrap();
    let date = &utc[..10];
    let seconds = days(field(0..4), field(5..7), field(8..10)) * 86_400
        + field(11..13) * 3600
        + field(14..16) * 60;
    json!({
        "raw": format!("{utc}:00+00:00"),
        "us": seconds * 1_000_000,
        "offset_s": 0,
        "date": date,
        "year": field(0..4),
        "vienna_date": null,
        "display": format!("{date} {}:00+00:00", &utc[11..16]),
        "iso": format!("{utc}:00+00:00"),
    })
}

fn buy(row: i64, at: &str, wallet: &str, amount: &str, fiat: &str) -> Value {
    json!({"kind": "in", "row": row, "unique_id": format!("b{row}"), "asset": "BTC", "transaction_type": "BUY",
           "notes": null, "exchange": wallet, "holder": "H", "spot_price": "1E0", "crypto_in": amount,
           "fiat_in_no_fee": fiat, "fiat_in_with_fee": fiat, "fiat_fee": "0E0", "ts": ts(at)})
}

fn sell(row: i64, at: &str, wallet: &str, amount: &str, fiat: &str) -> Value {
    json!({"kind": "out", "row": row, "unique_id": format!("s{row}"), "asset": "BTC", "transaction_type": "SELL",
           "notes": null, "exchange": wallet, "holder": "H", "spot_price": "1E0", "crypto_out_no_fee": amount,
           "crypto_fee": "0E0", "fiat_out_no_fee": fiat, "fiat_fee": "0E0", "ts": ts(at)})
}

/// Germany's calendar rules, as its registry entry will state them.
fn berlin() -> Value {
    json!({"kind": "rules", "timezone": "Europe/Berlin", "holding": "more_than_one_year"})
}

fn run(country: Value, method: &str, scope: &str, entries: Vec<Value>) -> Value {
    let request = json!({"schema_version": 1, "operation": "compute", "country": country,
                         "method": method, "pool_scope": scope,
                         "assets": [{"asset": "BTC", "entries": entries}]});
    serde_json::from_str(&compute(&request.to_string())).unwrap()
}

/// The `long_term` flag of each gain/loss.
fn terms(response: &Value) -> Vec<bool> {
    assert_eq!(response["ok"], true, "{response}");
    response["assets"][0]["gain_losses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gl| gl["long_term"].as_bool().unwrap())
        .collect()
}

#[test]
fn holding_periods_count_local_dates_in_the_rules_zone() {
    // de-06: bought 2023-06-10T22:30Z (11 June in Berlin), sold
    // 2024-06-11T21:30Z (11 June, 23:30 in Berlin): the anniversary, so
    // still within the year. By UTC dates the lot was bought on 10 June and
    // the sale is a day after its anniversary.
    let book = || {
        vec![
            buy(1, "2023-06-10T22:30", "A", "1E0", "5000E0"),
            sell(2, "2024-06-11T21:30", "A", "1E0", "13000E0"),
        ]
    };
    assert_eq!(terms(&run(berlin(), "fifo", "global", book())), [false]);
    let utc = json!({"kind": "rules", "timezone": "UTC", "holding": "more_than_one_year"});
    assert_eq!(terms(&run(utc, "fifo", "global", book())), [true]);
    // The day after the Berlin anniversary is past the year.
    let later = vec![
        buy(1, "2023-06-10T22:30", "A", "1E0", "5000E0"),
        sell(2, "2024-06-11T22:30", "A", "1E0", "13000E0"),
    ];
    assert_eq!(terms(&run(berlin(), "fifo", "wallet", later)), [true]);
}

#[test]
fn a_year_is_counted_in_calendar_dates_not_days() {
    // de-05: 365 × 24 hours across 29 February are still within the year;
    // RP2's day count (the generic rule) calls them long-term.
    let book = || {
        vec![
            buy(1, "2023-03-01T09:00", "A", "1E0", "21000E0"),
            sell(2, "2024-02-29T09:00", "A", "1E0", "55000E0"),
        ]
    };
    assert_eq!(terms(&run(berlin(), "fifo", "global", book())), [false]);
    let generic = json!({"kind": "generic", "long_term_days": 365});
    assert_eq!(terms(&run(generic, "fifo", "global", book())), [true]);
    // de-04: 29 February's anniversary is 28 February.
    let leap = vec![
        buy(1, "2024-02-29T09:00", "A", "1E-1", "5000E0"),
        sell(2, "2025-02-28T09:00", "A", "5E-2", "4500E0"),
        sell(3, "2025-03-01T09:00", "A", "5E-2", "4600E0"),
    ];
    assert_eq!(terms(&run(berlin(), "fifo", "global", leap)), [false, true]);
}

#[test]
fn without_a_holding_period_every_gain_is_short_term() {
    let never = json!({"kind": "rules", "timezone": "Europe/Madrid", "holding": "never"});
    let book = vec![
        buy(1, "2020-01-01T09:00", "A", "1E0", "7000E0"),
        sell(2, "2025-01-01T09:00", "A", "1E0", "90000E0"),
    ];
    assert_eq!(terms(&run(never, "fifo", "global", book)), [false]);
}

#[test]
fn rules_fail_closed_on_an_unknown_zone_and_before_its_calendar() {
    let book = || {
        vec![
            buy(1, "2023-01-01T09:00", "A", "1E0", "1E0"),
            sell(2, "2024-01-01T09:00", "A", "1E0", "2E0"),
        ]
    };
    let paris =
        json!({"kind": "rules", "timezone": "Europe/Paris", "holding": "more_than_one_year"});
    let response = run(paris, "fifo", "global", book());
    assert_eq!(response["error"]["class"], "RequestError", "{response}");
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("\"Europe/Paris\", which has no calendar"));
    let extra = json!({"kind": "rules", "timezone": "UTC", "holding": "never", "pool": "x"});
    let response = run(extra, "fifo", "global", book());
    assert_eq!(response["error"]["class"], "RequestError", "{response}");
    // 1995 has no Berlin calendar: the gain is not given a guessed term.
    let old = vec![
        buy(1, "1995-12-31T09:00", "A", "1E0", "1E0"),
        sell(2, "1997-01-01T09:00", "A", "1E0", "2E0"),
    ];
    let response = run(berlin(), "fifo", "global", old);
    assert_eq!(response["error"]["class"], "ValueError", "{response}");
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("Kassiber has no Europe/Berlin calendar"));
}

// ------------------------------------------------------- the ledger operation

/// `[-]digitsEexp` as plain decimal text without trailing zeros.
fn plain(value: &Value) -> String {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("a decimal: {value}"));
    let (mantissa, exponent) = text.split_once('E').unwrap();
    let (sign, digits) = match mantissa.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("", mantissa),
    };
    let exponent: i64 = exponent.parse().unwrap();
    let mut out = if exponent >= 0 {
        format!("{digits}{}", "0".repeat(exponent as usize))
    } else {
        let places = (-exponent) as usize;
        let padded = format!("{digits:0>width$}", width = places + 1);
        let (whole, fraction) = padded.split_at(padded.len() - places);
        format!("{whole}.{fraction}")
    };
    if out.contains('.') {
        out = out.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    let out = out.trim_start_matches('0');
    let out = if out.is_empty() || out.starts_with('.') {
        format!("0{out}")
    } else {
        out.to_owned()
    };
    if out == "0" {
        out
    } else {
        format!("{sign}{out}")
    }
}

/// msat of a BTC amount written as decimal text.
fn msat(btc: &str) -> u64 {
    let (whole, fraction) = btc.split_once('.').unwrap_or((btc, ""));
    let fraction = format!("{fraction:0<11}");
    whole.parse::<u64>().unwrap() * 100_000_000_000 + fraction.parse::<u64>().unwrap()
}

/// A finalized projection row of `asset` at `at` (UTC `YYYY-MM-DDTHH:MM`).
fn ledger_row(
    id: &str,
    asset: &str,
    wallet: &str,
    direction: &str,
    at: &str,
    amount: &str,
) -> Value {
    json!({
        "id": format!("custody-tax:{id}"),
        "anchor": id,
        "asset": asset,
        "wallet_id": wallet,
        "direction": direction,
        "ts": ts(at),
        "cutoff_instant": {"us": ts(at)["us"]},
        "amount_msat": msat(amount),
        "fee_msat": 0,
        "rate": null,
        "value": null,
        "coarse": false,
        "kind": if direction == "inbound" { "buy" } else { "sell" },
        "text": id,
        "regime_override": null,
        "category_override": null,
        "taxability_off": false,
        "privacy_hop": false,
        "hold": null,
        "chain_order": null,
    })
}

/// `row` with its fiat value, as canonical decimal text.
fn valued(mut row: Value, value: &str) -> Value {
    row["value"] = json!(value);
    row
}

/// The legs of a move of `amount` from `from` to `to`, paying `fee` priced
/// at `rate` per unit.
fn move_legs(
    id: &str,
    from: &str,
    to: &str,
    at: &str,
    amount: &str,
    fee: &str,
    rate: &str,
) -> [Value; 2] {
    let mut out = ledger_row(&format!("{id}-out"), "BTC", from, "outbound", at, amount);
    let mut inbound = ledger_row(&format!("{id}-in"), "BTC", to, "inbound", at, amount);
    out["fee_msat"] = json!(msat(fee));
    for leg in [&mut out, &mut inbound] {
        leg["anchor"] = json!(id);
        leg["kind"] = json!("transfer");
        leg["rate"] = json!(rate);
    }
    [out, inbound]
}

fn move_pair(id: &str) -> Value {
    json!({"out": format!("custody-tax:{id}-out"), "in": format!("custody-tax:{id}-in"),
           "source": "reviewed_custody_component", "group_id": null})
}

/// Runs a ledger request over `rows` in wallets A and B.
fn ledger_run(
    country: Value,
    method: &str,
    scope: &str,
    rows: Vec<Value>,
    moves: Vec<Value>,
    cross: Vec<Value>,
) -> Value {
    let mut assets: Vec<&str> = rows
        .iter()
        .map(|row| row["asset"].as_str().unwrap())
        .collect();
    assets.sort_unstable();
    assets.dedup();
    let mut policy = json!({"country": country, "method": method, "method_label": method.to_uppercase(),
                            "require_coarse_review": false});
    if scope == "wallet" {
        policy["pool_scope"] = json!("wallet");
    }
    let request = json!({
        "schema_version": 2, "operation": "ledger", "policy": policy,
        "config": {"assets": assets, "exchanges": ["A", "B"], "holders": ["H"]},
        "holder": "H",
        "wallets": [{"id": "A", "label": "A"}, {"id": "B", "label": "B"}],
        "rows": rows, "intra_pairs": moves, "cross_asset_pairs": cross,
    });
    serde_json::from_str(&kassiber_tax_core::ledger(&request.to_string())).unwrap()
}

/// Each realized entry as `(type, row anchor, quantity, proceeds, cost,
/// gain, expenses, term)`, plain decimals.
fn realized(response: &Value) -> Vec<[String; 8]> {
    assert_eq!(response["ok"], true, "{response}");
    response["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry.get("gain_loss").is_some())
        .map(|entry| {
            [
                entry["entry_type"].as_str().unwrap().to_owned(),
                entry["row"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("custody-tax:")
                    .to_owned(),
                plain(&entry["quantity"]),
                plain(&entry["proceeds"]),
                plain(&entry["cost_basis"]),
                plain(&entry["gain_loss"]),
                entry.get("expenses").map_or("-".to_owned(), plain),
                entry["capital_gains_type"].as_str().unwrap().to_owned(),
            ]
        })
        .collect()
}

fn rules(fee_units: &str) -> Value {
    json!({"kind": "rules", "timezone": "Europe/Berlin", "holding": "more_than_one_year",
           "fee_units": fee_units})
}

fn strings<const N: usize>(items: [&str; N]) -> [String; N] {
    items.map(str::to_owned)
}

/// de-21: two lots, a sale of 1 BTC for 60,000 paying 0.0002 BTC (12.00).
fn sale_with_fee_book() -> Vec<Value> {
    let mut sale = valued(
        ledger_row("s1", "BTC", "A", "outbound", "2024-06-01T10:00", "1"),
        "60000E0",
    );
    sale["fee_msat"] = json!(msat("0.0002"));
    vec![
        valued(
            ledger_row("b1", "BTC", "A", "inbound", "2023-01-10T10:00", "1"),
            "15000E0",
        ),
        valued(
            ledger_row("b2", "BTC", "A", "inbound", "2024-02-01T10:00", "0.5"),
            "20000E0",
        ),
        sale,
    ]
}

#[test]
fn fee_units_of_a_sale_are_a_disposal_before_the_sale() {
    for scope in ["global", "wallet"] {
        let response = ledger_run(
            rules("disposal_before_sale"),
            "fifo",
            scope,
            sale_with_fee_book(),
            vec![],
            vec![],
        );
        assert_eq!(
            realized(&response),
            [
                strings(["fee", "s1", "-0.0002", "12", "3", "9", "-", "long"]),
                strings([
                    "disposal",
                    "s1",
                    "-0.9998",
                    "59988",
                    "14997",
                    "44979.0024",
                    "11.9976",
                    "long"
                ]),
                strings(["disposal", "s1", "-0.0002", "12", "8", "3.9976", "0.0024", "short"]),
            ],
            "{scope}"
        );
        // The fee rows count in the year's summary; the sale's carries its
        // expenses.
        let summary = &response["tax_summary"];
        let types: Vec<&str> = summary
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["transaction_type"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["fee", "sell", "sell"], "{scope}");
        assert_eq!(plain(&summary[1]["expenses"]), "0.0024");
    }
}

#[test]
fn without_the_split_a_sales_fee_units_stay_part_of_the_sale() {
    // One disposal of 1.0002 BTC at the sale's price: FIFO takes all of
    // b1 and 0.0002 of b2, the price split by quantity.
    for fee_units in ["basis", "disposal"] {
        let entries = realized(&ledger_run(
            rules(fee_units),
            "fifo",
            "global",
            sale_with_fee_book(),
            vec![],
            vec![],
        ));
        let lots: Vec<[&str; 4]> = entries
            .iter()
            .map(|e| [e[0].as_str(), e[2].as_str(), e[4].as_str(), e[6].as_str()])
            .collect();
        assert_eq!(
            lots,
            [
                ["disposal", "-1", "15000", "-"],
                ["disposal", "-0.0002", "8", "-"]
            ],
            "{fee_units}"
        );
        let proceeds: f64 = entries.iter().map(|e| e[3].parse::<f64>().unwrap()).sum();
        assert!((proceeds - 60000.0).abs() < 1e-9, "{fee_units}: {proceeds}");
    }
}

/// de-18: a move of 0.5 BTC paying 0.0001 BTC worth 5.80 from a 40,000 lot.
fn move_book() -> (Vec<Value>, Vec<Value>) {
    let mut rows = vec![valued(
        ledger_row("b1", "BTC", "A", "inbound", "2024-01-10T10:00", "1"),
        "40000E0",
    )];
    rows.extend(move_legs(
        "m1",
        "A",
        "B",
        "2024-03-01T10:00",
        "0.5",
        "0.0001",
        "58000E0",
    ));
    rows.push(valued(
        ledger_row("s1", "BTC", "B", "outbound", "2024-05-01T10:00", "0.5"),
        "31000E0",
    ));
    (rows, vec![move_pair("m1")])
}

#[test]
fn a_moves_fee_units_are_a_disposal_at_their_value() {
    for fee_units in ["disposal", "disposal_before_sale"] {
        let (rows, moves) = move_book();
        let response = ledger_run(rules(fee_units), "fifo", "wallet", rows, moves, vec![]);
        assert_eq!(
            realized(&response),
            [
                strings([
                    "transfer_fee",
                    "m1-out",
                    "-0.0001",
                    "5.8",
                    "4",
                    "1.8",
                    "-",
                    "short"
                ]),
                strings(["disposal", "s1", "-0.5", "31000", "20000", "11000", "-", "short"]),
            ],
            "{fee_units}"
        );
        let types: Vec<&str> = response["tax_summary"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["transaction_type"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["move", "sell"], "{fee_units}");
    }
    // The generic rule: the fee's basis leaves without a gain, and the
    // summary has no move row.
    let (rows, moves) = move_book();
    let response = ledger_run(rules("basis"), "fifo", "wallet", rows, moves, vec![]);
    assert_eq!(
        realized(&response)[0],
        strings([
            "transfer_fee",
            "m1-out",
            "-0.0001",
            "4",
            "4",
            "0",
            "-",
            "short"
        ])
    );
    assert_eq!(response["tax_summary"].as_array().unwrap().len(), 1);
}

// ------------------------------------------------------------------ swaps

/// Each acquisition as `(row anchor, asset, quantity, value)`.
fn acquisitions(response: &Value) -> Vec<[String; 4]> {
    assert_eq!(response["ok"], true, "{response}");
    response["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["entry_type"] == "acquisition")
        .map(|entry| {
            [
                entry["row"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("custody-tax:")
                    .to_owned(),
                entry["asset"].as_str().unwrap().to_owned(),
                plain(&entry["quantity"]),
                plain(&entry["fiat_value"]),
            ]
        })
        .collect()
}

fn swap_rules(swap_values: &str) -> Value {
    json!({"kind": "rules", "timezone": "Europe/Berlin", "holding": "more_than_one_year",
           "swap_values": swap_values})
}

/// de-13: 0.25 BTC (worth 14,250, from a 20,000 lot of 0.5) for 15,500
/// USDT (worth 14,300, or unpriced), which sell for 14,400.
fn swap_book(usdt_value: Option<&str>) -> (Vec<Value>, Vec<Value>) {
    let mut usdt = ledger_row("x1-in", "USDT", "A", "inbound", "2024-04-10T10:00", "15500");
    if let Some(value) = usdt_value {
        usdt["value"] = json!(value);
    }
    usdt["anchor"] = json!("x1");
    let mut btc = valued(
        ledger_row("x1-out", "BTC", "A", "outbound", "2024-04-10T10:00", "0.25"),
        "14250E0",
    );
    btc["anchor"] = json!("x1");
    let rows = vec![
        valued(
            ledger_row("b1", "BTC", "A", "inbound", "2024-01-10T10:00", "0.5"),
            "20000E0",
        ),
        btc,
        usdt,
        valued(
            ledger_row("s1", "USDT", "A", "outbound", "2024-05-10T10:00", "15500"),
            "14400E0",
        ),
    ];
    let pair = json!({"pair_id": "x1", "out_id": "custody-tax:x1-out", "in_id": "custody-tax:x1-in",
                      "out_transaction_id": "x1", "in_transaction_id": "x1", "out_asset": "BTC",
                      "in_asset": "USDT", "policy": "taxable", "bitcoin_rail": false});
    (rows, vec![pair])
}

/// `(BTC disposal proceeds, USDT cost, USDT sale gain)`.
fn swapped(swap_values: &str, usdt_value: Option<&str>) -> [String; 3] {
    let (rows, cross) = swap_book(usdt_value);
    let response = ledger_run(
        swap_rules(swap_values),
        "fifo",
        "wallet",
        rows,
        vec![],
        cross,
    );
    let realized = realized(&response);
    let usdt = acquisitions(&response)
        .into_iter()
        .find(|a| a[1] == "USDT")
        .unwrap();
    [
        realized[0][3].clone(),
        usdt[3].clone(),
        realized[1][5].clone(),
    ]
}

#[test]
fn swap_values_choose_the_received_or_given_or_higher_value() {
    let usdt = Some("14300E0");
    // Each leg at its own value.
    assert_eq!(swapped("own", usdt), strings(["14250", "14300", "100"]));
    // de-13: proceeds are the USDT's value, its cost the BTC's.
    assert_eq!(
        swapped("received_given", usdt),
        strings(["14300", "14250", "150"])
    );
    // ie-06, jp-03: both are the received value.
    assert_eq!(
        swapped("received", usdt),
        strings(["14300", "14300", "100"])
    );
    // es-03: both are the higher value.
    assert_eq!(swapped("higher", usdt), strings(["14300", "14300", "100"]));
    let lower = Some("14000E0");
    assert_eq!(swapped("higher", lower), strings(["14250", "14250", "150"]));
    assert_eq!(
        swapped("received", lower),
        strings(["14000", "14000", "400"])
    );
}

#[test]
fn a_swap_leg_without_a_value_takes_the_other_legs() {
    // de-14: the USDT has no market value, so the BTC's is both.
    for mode in ["received_given", "received", "higher"] {
        assert_eq!(
            swapped(mode, None),
            strings(["14250", "14250", "150"]),
            "{mode}"
        );
    }
    // Under each leg's own value the unpriced USDT is quarantined.
    let (rows, cross) = swap_book(None);
    let response = ledger_run(swap_rules("own"), "fifo", "wallet", rows, vec![], cross);
    assert_eq!(
        response["quarantines"][0]["reason"], "missing_spot_price",
        "{response}"
    );
    // A carrying-value pair is not an exchange: its legs keep their values.
    let (rows, mut cross) = swap_book(Some("14300E0"));
    cross[0]["policy"] = json!("carrying-value");
    let response = ledger_run(
        swap_rules("received_given"),
        "fifo",
        "wallet",
        rows,
        vec![],
        cross,
    );
    assert_eq!(realized(&response)[0][3], "14250");
}

// ------------------------------------------------------------------ gifts

fn gift_rules(fields: Value) -> Value {
    let mut country = json!({"kind": "rules", "timezone": "Europe/Madrid", "holding": "never"});
    for (key, value) in fields.as_object().unwrap() {
        country[key] = value.clone();
    }
    country
}

fn kind(mut row: Value, kind: &str) -> Value {
    row["kind"] = json!(kind);
    row
}

/// es-06: a BTC lot of 30,000 and an ETH lot of 3,000; 0.5 BTC given away
/// worth 40,000, 1 ETH worth 2,000.
fn gift_book() -> Vec<Value> {
    vec![
        valued(
            ledger_row("b1", "BTC", "A", "inbound", "2025-01-10T10:00", "1"),
            "30000E0",
        ),
        valued(
            ledger_row("e1", "ETH", "A", "inbound", "2025-01-10T10:00", "1"),
            "3000E0",
        ),
        kind(
            valued(
                ledger_row("g1", "BTC", "A", "outbound", "2025-06-01T10:00", "0.5"),
                "40000E0",
            ),
            "gift",
        ),
        kind(
            valued(
                ledger_row("g2", "ETH", "A", "outbound", "2025-06-01T10:00", "1"),
                "2000E0",
            ),
            "gift",
        ),
        valued(
            ledger_row("s1", "BTC", "A", "outbound", "2025-07-01T10:00", "0.1"),
            "7000E0",
        ),
    ]
}

fn reasons(response: &Value) -> Vec<(String, String)> {
    response["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            (
                q["row"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("custody-tax:")
                    .to_owned(),
                q["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_country_may_book_a_gift_as_a_disposal_at_its_value() {
    let country =
        gift_rules(json!({"gifts_given": "market_value", "gift_losses": "not_deductible"}));
    let response = ledger_run(country, "fifo", "global", gift_book(), vec![], vec![]);
    assert_eq!(
        realized(&response),
        [
            strings(["disposal", "g1", "-0.5", "40000", "15000", "25000", "-", "short"]),
            strings(["disposal", "s1", "-0.1", "7000", "3000", "4000", "-", "short"]),
            strings(["disposal", "g2", "-1", "2000", "3000", "-1000", "-", "short"]),
        ]
    );
    // es-06: the gift's loss is reported but does not count; the gain has
    // no mark.
    let marks: Vec<Value> = response["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["entry_type"] == "disposal")
        .map(|entry| entry.get("loss_deductible").cloned().unwrap_or(Value::Null))
        .collect();
    assert_eq!(marks, [Value::Null, Value::Null, json!(false)]);
    let types: Vec<&str> = response["tax_summary"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["transaction_type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["gift", "sell", "gift"]);
    // Losses that count carry no mark.
    let country = gift_rules(json!({"gifts_given": "market_value"}));
    let response = ledger_run(country, "fifo", "global", gift_book(), vec![], vec![]);
    assert!(response["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry.get("loss_deductible").is_none()));
}

#[test]
fn gifts_donations_and_losses_stay_quarantined_unless_the_country_books_them() {
    // The generic rule quarantines every gift.
    let response = ledger_run(
        gift_rules(json!({})),
        "fifo",
        "global",
        gift_book(),
        vec![],
        vec![],
    );
    assert_eq!(
        reasons(&response),
        [
            ("g1".to_owned(), "non_sale_disposal_kind".to_owned()),
            // The quarantined disposition leaves the lots uncertain.
            ("s1".to_owned(), "basis_provenance_incomplete".to_owned()),
            ("g2".to_owned(), "non_sale_disposal_kind".to_owned())
        ]
    );
    // Gifts booked, donations and lost coins not.
    let mut rows = gift_book();
    rows[2]["kind"] = json!("donation");
    rows[3]["kind"] = json!("lost");
    let country = gift_rules(json!({"gifts_given": "market_value"}));
    let response = ledger_run(country, "fifo", "global", rows.clone(), vec![], vec![]);
    assert_eq!(
        reasons(&response),
        [
            ("g1".to_owned(), "non_sale_disposal_kind".to_owned()),
            // The quarantined disposition leaves the lots uncertain.
            ("s1".to_owned(), "basis_provenance_incomplete".to_owned()),
            ("g2".to_owned(), "non_sale_disposal_kind".to_owned())
        ]
    );
    // A country booking donations books them as donations.
    let country = gift_rules(json!({"donations_given": "market_value"}));
    let response = ledger_run(country, "fifo", "global", rows, vec![], vec![]);
    assert_eq!(
        reasons(&response),
        [("g2".to_owned(), "non_sale_disposal_kind".to_owned())]
    );
    assert_eq!(response["tax_summary"][0]["transaction_type"], "donate");
}

#[test]
fn a_received_gift_without_the_donors_data_is_refused_and_taints_later_disposals() {
    // de-26: 0.1 BTC received as a gift, then sold.
    let rows = || {
        vec![
            valued(
                ledger_row("b1", "BTC", "A", "inbound", "2024-01-10T10:00", "1"),
                "40000E0",
            ),
            kind(
                valued(
                    ledger_row("g1", "BTC", "A", "inbound", "2024-03-01T10:00", "0.1"),
                    "6000E0",
                ),
                "gift",
            ),
            valued(
                ledger_row("s1", "BTC", "A", "outbound", "2024-05-01T10:00", "0.1"),
                "6500E0",
            ),
        ]
    };
    let country = gift_rules(json!({"gifts_received": "quarantine"}));
    let response = ledger_run(country, "fifo", "wallet", rows(), vec![], vec![]);
    assert_eq!(
        reasons(&response),
        [
            (
                "g1".to_owned(),
                "acquisition_valuation_unsupported".to_owned()
            ),
            ("s1".to_owned(), "basis_provenance_incomplete".to_owned()),
        ]
    );
    assert_eq!(
        response["quarantines"][0]["detail"]["valuation_mode"],
        "donor_basis"
    );
    // The generic rule books it as an acquisition at its value.
    let response = ledger_run(
        gift_rules(json!({})),
        "fifo",
        "wallet",
        rows(),
        vec![],
        vec![],
    );
    assert!(reasons(&response).is_empty(), "{response}");
    assert_eq!(acquisitions(&response).len(), 2);
}

#[test]
fn a_refused_receipt_is_held_with_the_countrys_code_before_its_price() {
    // de-23, de-25: an unclassified airdrop and an unpriced hard fork, then
    // a sale of the forked coins.
    let rows = || {
        let mut fork = kind(
            ledger_row("h1", "BCH", "A", "inbound", "2024-03-01T10:00", "1"),
            "hardfork",
        );
        fork["value"] = Value::Null;
        vec![
            kind(
                valued(
                    ledger_row("a1", "XYZ", "A", "inbound", "2024-02-10T10:00", "100"),
                    "50E0",
                ),
                "airdrop",
            ),
            fork,
            valued(
                ledger_row("s1", "BCH", "A", "outbound", "2024-05-01T10:00", "1"),
                "300E0",
            ),
        ]
    };
    let refused = json!({"refused_receipts": [
        {"kinds": ["airdrop"], "code": "airdrop_classification_required", "detail": "classify"},
        {"kinds": ["hard_fork", "hardfork"], "code": "hardfork_cost_split_unsupported", "detail": "split"},
    ]});
    let response = ledger_run(
        gift_rules(refused),
        "fifo",
        "wallet",
        rows(),
        vec![],
        vec![],
    );
    assert_eq!(
        reasons(&response),
        [
            (
                "a1".to_owned(),
                "acquisition_valuation_unsupported".to_owned()
            ),
            (
                "h1".to_owned(),
                "acquisition_valuation_unsupported".to_owned()
            ),
            // The held fork never reaches the wallet.
            ("s1".to_owned(), "insufficient_lots".to_owned()),
        ]
    );
    let codes: Vec<&Value> = response["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| &q["detail"]["code"])
        .collect();
    assert_eq!(
        codes,
        [
            &json!("airdrop_classification_required"),
            &json!("hardfork_cost_split_unsupported"),
            &Value::Null
        ]
    );
    // Without the refusal the airdrop books as income and the unpriced fork
    // waits for its price.
    let response = ledger_run(
        gift_rules(json!({})),
        "fifo",
        "wallet",
        rows(),
        vec![],
        vec![],
    );
    let reasons = reasons(&response);
    assert!(!reasons.iter().any(|(row, _)| row == "a1"), "{response}");
    assert!(reasons.contains(&("h1".to_owned(), "missing_spot_price".to_owned())));
}

// -------------------------------------------------------------- tax years

/// The summary year of a sale at `at` (UTC) under `country`.
fn summary_year(country: Value, at: &str) -> i64 {
    let rows = vec![
        valued(
            ledger_row("b1", "BTC", "A", "inbound", "2024-01-10T10:00", "1"),
            "10000E0",
        ),
        valued(
            ledger_row("s1", "BTC", "A", "outbound", at, "0.2"),
            "18000E0",
        ),
    ];
    let response = ledger_run(country, "fifo", "global", rows, vec![], vec![]);
    assert_eq!(response["ok"], true, "{response}");
    response["tax_summary"][0]["year"].as_i64().unwrap()
}

#[test]
fn the_summary_counts_the_countrys_tax_year_in_its_zone() {
    // es-14: 2025-12-31T23:30Z is 1 January 2026 in Madrid.
    let madrid = json!({"kind": "rules", "timezone": "Europe/Madrid", "holding": "never"});
    assert_eq!(summary_year(madrid.clone(), "2025-12-31T23:30"), 2026);
    assert_eq!(summary_year(madrid, "2025-12-31T22:30"), 2025);
    // gb-09: 6 April 00:30 BST starts 2025-26.
    let london = json!({"kind": "rules", "timezone": "Europe/London", "holding": "never",
                        "tax_year_start": "04-06"});
    assert_eq!(summary_year(london.clone(), "2025-04-05T23:30"), 2025);
    assert_eq!(summary_year(london, "2025-04-05T22:30"), 2024);
    // The generic year is the timestamp's own.
    let generic = json!({"kind": "generic", "long_term_days": 365});
    assert_eq!(summary_year(generic, "2025-12-31T23:30"), 2025);
    for start in ["4-6", "13-01", "02-30", "04-06x"] {
        let bad = json!({"kind": "rules", "timezone": "UTC", "holding": "never", "tax_year_start": start});
        let rows = vec![valued(
            ledger_row("b1", "BTC", "A", "inbound", "2024-01-10T10:00", "1"),
            "1E0",
        )];
        let response = ledger_run(bad, "fifo", "global", rows, vec![], vec![]);
        assert_eq!(response["ok"], false, "{start}");
        assert!(
            response["error"]["inner"]["message"]
                .as_str()
                .unwrap()
                .contains("not a MM-DD day"),
            "{response}"
        );
    }
}

// ------------------------------------------------------------ pool scopes

/// de-17: a 2023 lot in A, a 2024 lot in B; 0.5 sold from B.
fn two_wallet_book() -> Vec<Value> {
    vec![
        valued(
            ledger_row("a1", "BTC", "A", "inbound", "2023-01-10T10:00", "1"),
            "15000E0",
        ),
        valued(
            ledger_row("b1", "BTC", "B", "inbound", "2024-02-01T10:00", "1"),
            "38000E0",
        ),
        valued(
            ledger_row("s1", "BTC", "B", "outbound", "2024-07-01T10:00", "0.5"),
            "28000E0",
        ),
    ]
}

#[test]
fn a_country_that_identifies_units_per_wallet_refuses_the_global_pool() {
    let wallet = json!({"kind": "rules", "timezone": "Europe/Berlin",
                        "holding": "more_than_one_year", "pool_scope": "wallet"});
    // B's own lot: within the year, gain 9,000.
    let response = ledger_run(
        wallet.clone(),
        "fifo",
        "wallet",
        two_wallet_book(),
        vec![],
        vec![],
    );
    assert_eq!(
        realized(&response),
        [strings([
            "disposal", "s1", "-0.5", "28000", "19000", "9000", "-", "short"
        ])]
    );
    // The global pool would take A's 2023 lot; the rules refuse it.
    let response = ledger_run(wallet, "fifo", "global", two_wallet_book(), vec![], vec![]);
    assert_eq!(response["error"]["stage"], "compute", "{response}");
    assert!(response["error"]["inner"]["message"]
        .as_str()
        .unwrap()
        .starts_with("This country's rules identify units per wallet"));
    // A single-pool country refuses the wallet scope; one without a
    // required scope takes either.
    let global = json!({"kind": "rules", "timezone": "Europe/London", "holding": "never",
                        "pool_scope": "global"});
    let response = ledger_run(global, "fifo", "wallet", two_wallet_book(), vec![], vec![]);
    assert!(
        response["error"]["inner"]["message"]
            .as_str()
            .unwrap()
            .contains("across all wallets"),
        "{response}"
    );
    let either = json!({"kind": "rules", "timezone": "Europe/London", "holding": "never"});
    let response = ledger_run(either, "fifo", "global", two_wallet_book(), vec![], vec![]);
    assert_eq!(realized(&response)[0][4], "7500");
}

// ------------------------------------------------------- matching methods

/// Each wallet holding as `(wallet, quantity, cost)`.
fn holdings(response: &Value) -> Vec<[String; 3]> {
    response["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            [
                h["wallet"].as_str().unwrap().to_owned(),
                plain(&h["quantity"]),
                plain(&h["cost_basis"]),
            ]
        })
        .collect()
}

fn average_rules() -> Value {
    json!({"kind": "rules", "timezone": "Europe/Berlin", "holding": "more_than_one_year",
           "pool_scope": "wallet"})
}

#[test]
fn the_fifo_period_at_the_wallets_moving_average() {
    // de-19: 0.5 (2023, 8,000) and 0.5 (2024, 20,000) in A, average
    // 28,000/BTC; 0.6 sold for 36,000. FIFO dates: 0.5 outside the year,
    // 0.1 within; both cost the average.
    let rows = vec![
        valued(
            ledger_row("b1", "BTC", "A", "inbound", "2023-01-10T10:00", "0.5"),
            "8000E0",
        ),
        valued(
            ledger_row("b2", "BTC", "A", "inbound", "2024-02-01T10:00", "0.5"),
            "20000E0",
        ),
        valued(
            ledger_row("s1", "BTC", "A", "outbound", "2024-06-01T10:00", "0.6"),
            "36000E0",
        ),
    ];
    let response = ledger_run(
        average_rules(),
        "moving_average_fifo_period",
        "wallet",
        rows,
        vec![],
        vec![],
    );
    assert_eq!(
        realized(&response),
        [
            strings(["disposal", "s1", "-0.5", "30000", "14000", "16000", "-", "long"]),
            strings(["disposal", "s1", "-0.1", "6000", "2800", "3200", "-", "short"]),
        ]
    );
    assert_eq!(holdings(&response), [strings(["A", "0.4", "11200"])]);
}

#[test]
fn a_move_carries_its_sources_average_to_its_destination() {
    // A holds 1 at 10,000, B 1 at 30,000; half of A moves to B (B: 1.5 at
    // 35,000); 0.75 sold from B costs 0.75 × 35,000 / 1.5.
    let mut rows = vec![
        valued(
            ledger_row("a1", "BTC", "A", "inbound", "2024-01-10T10:00", "1"),
            "10000E0",
        ),
        valued(
            ledger_row("b1", "BTC", "B", "inbound", "2024-01-11T10:00", "1"),
            "30000E0",
        ),
    ];
    rows.extend(move_legs(
        "m1",
        "A",
        "B",
        "2024-02-01T10:00",
        "0.5",
        "0",
        "20000E0",
    ));
    rows.push(valued(
        ledger_row("s1", "BTC", "B", "outbound", "2024-03-01T10:00", "0.75"),
        "30000E0",
    ));
    let response = ledger_run(
        average_rules(),
        "moving_average_fifo_period",
        "wallet",
        rows,
        vec![move_pair("m1")],
        vec![],
    );
    // FIFO in B: the moved half of a1 first (acquired 10 January), then b1.
    let entries = realized(&response);
    let cost: f64 = entries.iter().map(|e| e[4].parse::<f64>().unwrap()).sum();
    assert!((cost - 17500.0).abs() < 1e-9, "{entries:?}");
    assert_eq!((entries.len(), entries[0][2].as_str()), (1, "-0.75"));
    let disposal = response["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["entry_type"] == "disposal")
        .unwrap();
    let lots: Vec<&str> = disposal["fragments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["lot"]["row"].as_str().unwrap())
        .collect();
    assert_eq!(lots, ["custody-tax:a1", "custody-tax:b1"]);
    // The 32-digit average leaves the remaining cost a unit in the last digit
    // away from 17,500.
    let held = holdings(&response);
    assert_eq!(held[0], strings(["A", "0.5", "5000"]));
    assert_eq!(held[1][..2], strings(["B", "0.75"]));
    assert!(
        (held[1][2].parse::<f64>().unwrap() - 17500.0).abs() < 1e-9,
        "{held:?}"
    );
}

#[test]
fn the_fifo_period_average_needs_the_wallet_scope() {
    let rows = vec![
        valued(
            ledger_row("b1", "BTC", "A", "inbound", "2023-01-10T10:00", "1"),
            "8000E0",
        ),
        valued(
            ledger_row("s1", "BTC", "A", "outbound", "2024-06-01T10:00", "0.5"),
            "36000E0",
        ),
    ];
    let rules =
        json!({"kind": "rules", "timezone": "Europe/Berlin", "holding": "more_than_one_year"});
    let response = ledger_run(
        rules,
        "moving_average_fifo_period",
        "global",
        rows,
        vec![],
        vec![],
    );
    assert!(
        response["error"]["inner"]["message"]
            .as_str()
            .unwrap()
            .contains("averages per wallet"),
        "{response}"
    );
    // Schema 1 reads the method too.
    let entries = vec![
        buy(1, "2023-01-10T10:00", "A", "1E0", "8000E0"),
        sell(2, "2024-06-01T10:00", "A", "5E-1", "36000E0"),
    ];
    let response = run(
        average_rules(),
        "moving_average_fifo_period",
        "wallet",
        entries,
    );
    assert_eq!(terms(&response), [true]);
    assert_eq!(
        response["assets"][0]["gain_losses"][0]["unit_cost_basis_override"],
        "8000E0"
    );
}

#[test]
fn the_austrian_swap_runner_refuses_a_matching_method() {
    let with = |mut entry: Value, asset: &str, notes: &str| {
        entry["asset"] = json!(asset);
        entry["notes"] = json!(notes);
        entry["ts"]["vienna_date"] = entry["ts"]["date"].clone();
        entry
    };
    let request = json!({
        "schema_version": 1, "operation": "compute_multi", "country": {"kind": "at"},
        "method": "moving_average_fifo_period",
        "assets": [
            {"asset": "BTC", "entries": [
                with(buy(1, "2023-01-10T10:00", "A", "1E0", "100E0"), "BTC", "at_regime=neu"),
                with(sell(2, "2023-02-10T10:00", "A", "1E0", "200E0"), "BTC", "at_regime=neu at_swap_link=x1"),
            ]},
            {"asset": "ETH", "entries": [
                with(buy(3, "2023-02-10T10:00", "A", "1E0", "200E0"), "ETH", "at_regime=neu at_swap_link=x1"),
            ]},
        ],
    });
    let response: Value = serde_json::from_str(&compute(&request.to_string())).unwrap();
    assert_eq!(response["error"]["class"], "ValueError", "{response}");
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("has no Austrian swap-runner form"));
}

// --------------------------------------------------------- share matching

/// A failed response's message, at the top or inside its stage.
fn error_text(response: &Value) -> String {
    assert_eq!(response["ok"], false, "{response}");
    let error = &response["error"];
    error["message"]
        .as_str()
        .or(error["inner"]["message"].as_str())
        .unwrap_or_else(|| panic!("no message: {response}"))
        .to_owned()
}

fn london() -> Value {
    json!({"kind": "rules", "timezone": "Europe/London", "holding": "never",
           "tax_year_start": "04-06", "pool_scope": "global"})
}

fn token(id: &str, direction: &str, at: &str, amount: &str, value: &str) -> Value {
    valued(ledger_row(id, "TOK", "A", direction, at, amount), value)
}

/// Each realized entry as `(row, quantity, cost, gain)`.
fn gains(response: &Value) -> Vec<[String; 4]> {
    realized(response)
        .into_iter()
        .map(|[_, row, quantity, _, cost, gain, _, _]| [row, quantity, cost, gain])
        .collect()
}

#[test]
fn share_matching_takes_the_same_day_before_the_pool() {
    // gb-02 (CRYPTO22252): a day's two sales are one disposal of 1,500,
    // matched with 1,500 of the 1,600 bought that day (937.50); the other
    // 100 join the pool. The 09:00 sale takes the 14:00 purchase.
    let rows = vec![
        token("b0", "inbound", "2024-01-15T10:00", "5000", "500E0"),
        token("s1", "outbound", "2024-06-23T08:00", "1000", "800E0"),
        token("b1", "inbound", "2024-06-23T13:00", "1600", "1000E0"),
        token("s2", "outbound", "2024-06-23T19:00", "500", "600E0"),
    ];
    let response = ledger_run(
        london(),
        "uk_share_matching",
        "global",
        rows,
        vec![],
        vec![],
    );
    assert_eq!(
        gains(&response),
        [
            strings(["s1", "-1000", "625", "175"]),
            strings(["s2", "-500", "312.5", "287.5"]),
        ]
    );
    // The pool: 5,000 at 500 and 100 at 62.50, held at its average (to the
    // engine's 32 digits).
    let holdings = holdings(&response);
    assert_eq!(holdings[0][..2], strings(["A", "5100"]));
    assert!(
        holdings[0][2].starts_with("562.49999999999999999999"),
        "{holdings:?}"
    );
}

#[test]
fn share_matching_looks_30_days_ahead_then_takes_the_pool_average() {
    // gb-06 (CRYPTO22256): same day, 30 days and the pool interacting.
    let rows = vec![
        token("b0", "inbound", "2024-01-10T10:00", "100000", "300000E0"),
        token("b1", "inbound", "2024-07-31T08:00", "10000", "45000E0"),
        token("s1", "outbound", "2024-07-31T09:00", "30000", "150000E0"),
        token("s2", "outbound", "2024-08-05T09:00", "20000", "100000E0"),
        token("b2", "inbound", "2024-08-06T09:00", "50000", "225000E0"),
        token("s3", "outbound", "2024-08-07T09:00", "100000", "150000E0"),
    ];
    let response = ledger_run(
        london(),
        "uk_share_matching",
        "global",
        rows,
        vec![],
        vec![],
    );
    let gains = gains(&response);
    // 31 July: 10,000 same day (45,000) and 20,000 of 6 August (90,000).
    assert_eq!(gains[0], strings(["s1", "-30000", "135000", "15000"]));
    // 5 August: the other 20,000 of 6 August's 50,000 (90,000).
    assert_eq!(gains[1], strings(["s2", "-20000", "90000", "10000"]));
    // 7 August: 100,000 of the pool of 110,000 at 345,000.
    assert_eq!(gains[2][..2], strings(["s3", "-100000"]));
    assert!(gains[2][2].starts_with("313636.36363636"), "{gains:?}");
    let holdings = holdings(&response);
    assert_eq!(holdings[0][..2], strings(["A", "10000"]));
    assert!(holdings[0][2].starts_with("31363.63636363"), "{holdings:?}");
}

#[test]
fn share_matching_windows_30_days_and_refuses_what_the_pool_lacks() {
    // gb-10: day 30 is matched, day 31 joins the pool.
    let rows = vec![
        token("b1", "inbound", "2024-01-10T10:00", "1", "30000E0"),
        token("s1", "outbound", "2024-06-01T09:00", "0.5", "35000E0"),
        token("b2", "inbound", "2024-07-01T09:00", "0.2", "12000E0"),
        token("b3", "inbound", "2024-07-02T09:00", "0.2", "12500E0"),
    ];
    let response = ledger_run(
        london(),
        "uk_share_matching",
        "global",
        rows,
        vec![],
        vec![],
    );
    assert_eq!(
        gains(&response),
        [strings(["s1", "-0.5", "21000", "14000"])]
    );
    // London days: 23:30 UTC on 30 June is 1 July in BST, day 30 and inside
    // the window; 23:30 UTC on 1 July is 2 July, outside it.
    let window = |at: &str| {
        let rows = vec![
            token("b1", "inbound", "2024-01-10T10:00", "1", "30000E0"),
            token("s1", "outbound", "2024-06-01T09:00", "0.5", "35000E0"),
            token("b2", "inbound", at, "0.2", "12000E0"),
        ];
        gains(&ledger_run(
            london(),
            "uk_share_matching",
            "global",
            rows,
            vec![],
            vec![],
        ))
    };
    assert_eq!(
        window("2024-06-30T23:30"),
        [strings(["s1", "-0.5", "21000", "14000"])]
    );
    assert_eq!(
        window("2024-07-01T23:30"),
        [strings(["s1", "-0.5", "15000", "20000"])]
    );
    // The global scope only, and only with a country calendar.
    let rows = vec![
        token("b1", "inbound", "2024-01-10T10:00", "1", "30000E0"),
        token("s1", "outbound", "2024-06-01T09:00", "0.5", "35000E0"),
    ];
    let mut wallet = london();
    wallet["pool_scope"] = json!("wallet");
    let response = ledger_run(
        wallet,
        "uk_share_matching",
        "wallet",
        rows.clone(),
        vec![],
        vec![],
    );
    assert!(
        error_text(&response).contains("global pool scope"),
        "{response}"
    );
    let generic = json!({"kind": "generic", "long_term_days": 365});
    let response = ledger_run(generic, "uk_share_matching", "global", rows, vec![], vec![]);
    assert!(
        error_text(&response).contains("needs a country calendar"),
        "{response}"
    );
}

#[test]
fn share_matching_refuses_a_disposal_the_pool_cannot_cover() {
    // Through schema 1, which has no wallet balance gate: the day's sale
    // needs more than the pool holds.
    let entries = vec![
        buy(1, "2024-01-10T10:00", "A", "1E-1", "5000E0"),
        sell(2, "2024-06-01T09:00", "A", "2E-1", "11000E0"),
    ];
    let response = run(london(), "uk_share_matching", "global", entries);
    assert_eq!(response["ok"], false, "{response}");
    assert!(error_text(&response)
        .starts_with("Total in-transaction crypto value < total taxable crypto value"));
}

// ------------------------------------------------- kinds and valuations

/// Spain's refusals, as its registry entry states them: an inbound hard
/// fork, and outbound kinds containing `lost` or `donat`.
fn spanish_refusals() -> Value {
    gift_rules(json!({"refused_receipts": [
        {"kinds": ["hardfork"], "code": "es_hardfork_requires_review", "detail": "fork"},
        {"direction": "outbound", "kinds": ["lost"], "code": "es_lost_requires_review", "detail": "lost"},
        {"direction": "outbound", "kinds": ["donat"], "code": "es_donation_requires_review", "detail": "donation"},
    ]}))
}

/// Each quarantine's row, reason and country code (`-` without one).
fn coded_reasons(response: &Value) -> Vec<[String; 3]> {
    response["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            [
                q["row"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("custody-tax:")
                    .to_owned(),
                q["reason"].as_str().unwrap().to_owned(),
                q["detail"]["code"].as_str().unwrap_or("-").to_owned(),
            ]
        })
        .collect()
}

#[test]
fn an_outbound_refusal_is_held_with_the_countrys_code_before_its_price() {
    // es-16 and es-17: lost coins (unpriced) and a hard fork are refused
    // with Spain's codes; the generic rules call them otherwise.
    let rows = || {
        vec![
            valued(
                ledger_row("b1", "BTC", "A", "inbound", "2025-01-10T10:00", "1"),
                "30000E0",
            ),
            kind(
                ledger_row("l1", "BTC", "A", "outbound", "2025-06-01T10:00", "0.1"),
                "lost_keys",
            ),
            kind(
                valued(
                    ledger_row("f1", "BCH", "A", "inbound", "2025-06-01T10:00", "1"),
                    "300E0",
                ),
                "hardfork",
            ),
            kind(
                valued(
                    ledger_row("d1", "BTC", "A", "outbound", "2025-07-01T10:00", "0.1"),
                    "6000E0",
                ),
                "donation",
            ),
            valued(
                ledger_row("s1", "BTC", "A", "outbound", "2025-08-01T10:00", "0.1"),
                "7000E0",
            ),
        ]
    };
    let response = ledger_run(spanish_refusals(), "fifo", "global", rows(), vec![], vec![]);
    assert_eq!(response["ok"], true, "{response}");
    let held = |row: &str, reason: &str, code: &str| [row, reason, code].map(str::to_owned);
    assert_eq!(
        coded_reasons(&response),
        [
            held("l1", "non_sale_disposal_kind", "es_lost_requires_review"),
            held(
                "d1",
                "non_sale_disposal_kind",
                "es_donation_requires_review"
            ),
            // The refusals leave the lots uncertain, as every quarantine.
            held("s1", "basis_provenance_incomplete", "-"),
            held(
                "f1",
                "acquisition_valuation_unsupported",
                "es_hardfork_requires_review"
            ),
        ]
    );
    let lost = &response["quarantines"][0];
    assert_eq!(lost["detail"]["kind"], "lost_keys");
    assert_eq!(lost["detail"]["detail"], "lost");
    assert_eq!(lost["detail_kind"], "plain");
    // The generic rules: an unpriced loss misses its price, a hard fork is
    // income, a donation a non-sale disposition without a code.
    let response = ledger_run(
        gift_rules(json!({})),
        "fifo",
        "global",
        rows(),
        vec![],
        vec![],
    );
    assert_eq!(
        coded_reasons(&response),
        [
            held("l1", "missing_spot_price", "-"),
            held("d1", "non_sale_disposal_kind", "-"),
            held("s1", "basis_provenance_incomplete", "-"),
        ]
    );
    // An inbound refusal does not read outbound rows, nor an outbound one
    // inbound rows: `lost` as an inbound kind is not refused.
    let mut country = spanish_refusals();
    country["refused_receipts"][1]["direction"] = json!("inbound");
    let response = ledger_run(country, "fifo", "global", rows(), vec![], vec![]);
    assert_eq!(
        coded_reasons(&response)[0],
        held("l1", "missing_spot_price", "-")
    );
    // An outbound refusal may name only a non-sale disposal kind.
    let mut country = spanish_refusals();
    country["refused_receipts"][2]["kinds"] = json!(["sell"]);
    let response = ledger_run(country, "fifo", "global", rows(), vec![], vec![]);
    assert_eq!(
        response["error"]["inner"]["class"], "RequestError",
        "{response}"
    );
}

#[test]
fn a_zero_cost_kind_is_an_acquisition_at_zero_without_income() {
    // jp-10: BCH from a fork, unpriced, sold for 50,000 yen: all gain.
    let rows = vec![
        kind(
            ledger_row("f1", "BCH", "A", "inbound", "2025-02-01T01:00", "1"),
            "hardfork",
        ),
        valued(
            ledger_row("s1", "BCH", "A", "outbound", "2025-03-01T01:00", "1"),
            "50000E0",
        ),
    ];
    let country = json!({"kind": "rules", "timezone": "Asia/Tokyo", "holding": "never",
                         "zero_cost_kinds": ["hardfork", "hard_fork"]});
    let response = ledger_run(country, "fifo", "global", rows.clone(), vec![], vec![]);
    assert_eq!(
        realized(&response),
        [strings([
            "disposal", "s1", "-1", "50000", "0", "50000", "-", "short"
        ])]
    );
    assert_eq!(acquisitions(&response), [strings(["f1", "BCH", "1", "0"])]);
    // Without the rule an unpriced fork misses its price.
    let generic = json!({"kind": "rules", "timezone": "Asia/Tokyo", "holding": "never"});
    let response = ledger_run(generic, "fifo", "global", rows, vec![], vec![]);
    assert_eq!(
        reasons(&response),
        [
            ("f1".to_owned(), "missing_spot_price".to_owned()),
            ("s1".to_owned(), "insufficient_lots".to_owned())
        ]
    );
}

/// A sale of 1 BTC for `price` when the market valued it at `market`.
fn below_market_book(price: &str, market: &str) -> Vec<Value> {
    let mut sale = valued(
        ledger_row("s1", "BTC", "A", "outbound", "2025-05-20T01:00", "1"),
        price,
    );
    sale["market_value"] = json!(market);
    vec![
        valued(
            ledger_row("b1", "BTC", "A", "inbound", "2025-04-09T01:00", "1"),
            "450000E0",
        ),
        sale,
    ]
}

fn valuation(response: &Value) -> Value {
    response["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["entry_type"] == "disposal")
        .and_then(|entry| entry.get("valuation").cloned())
        .unwrap_or(Value::Null)
}

#[test]
fn a_sale_below_market_value_books_at_the_countrys_floor() {
    let rules = |below: &str| {
        json!({"kind": "rules", "timezone": "Asia/Tokyo", "holding": "never",
               "below_market": below})
    };
    // jp-12: sold for 450,000 when worth 1,000,000: 70 % is 700,000.
    let response = ledger_run(
        rules("seventy_percent"),
        "fifo",
        "global",
        below_market_book("450000E0", "1000000E0"),
        vec![],
        vec![],
    );
    assert_eq!(
        realized(&response),
        [strings([
            "disposal", "s1", "-1", "700000", "450000", "250000", "-", "short"
        ])]
    );
    assert_eq!(valuation(&response), "seventy_percent");
    // jp-13: exactly 70 % is not below it.
    let response = ledger_run(
        rules("seventy_percent"),
        "fifo",
        "global",
        below_market_book("700000E0", "1000000E0"),
        vec![],
        vec![],
    );
    assert_eq!(realized(&response)[0][3], "700000");
    assert_eq!(valuation(&response), Value::Null);
    // es-12: Spain takes the market value whenever it is higher.
    let response = ledger_run(
        rules("market_value"),
        "fifo",
        "global",
        below_market_book("700000E0", "1000000E0"),
        vec![],
        vec![],
    );
    assert_eq!(realized(&response)[0][3], "1000000");
    assert_eq!(valuation(&response), "market_value");
    // The generic rule keeps the price; a market value above it is ignored.
    let generic = json!({"kind": "rules", "timezone": "Asia/Tokyo", "holding": "never"});
    let response = ledger_run(
        generic,
        "fifo",
        "global",
        below_market_book("450000E0", "1000000E0"),
        vec![],
        vec![],
    );
    assert_eq!(realized(&response)[0][3], "450000");
    assert_eq!(valuation(&response), Value::Null);
}

// ------------------------------------------------------- Japan's averages

fn tokyo() -> Value {
    json!({"kind": "rules", "timezone": "Asia/Tokyo", "holding": "never", "pool_scope": "global"})
}

/// jp-04 and jp-05 (NTA FAQ 2-4): three purchases over two exchanges and
/// two sales in one Tokyo year.
fn faq_2_4_book() -> Vec<Value> {
    vec![
        valued(
            ledger_row("t1", "BTC", "A", "inbound", "2025-03-01T01:00", "4"),
            "1845000E0",
        ),
        valued(
            ledger_row("t2", "BTC", "B", "inbound", "2025-06-20T01:00", "2"),
            "1650000E0",
        ),
        valued(
            ledger_row("t3", "BTC", "B", "outbound", "2025-07-10T01:00", "2"),
            "2400000E0",
        ),
        valued(
            ledger_row("t4", "BTC", "A", "inbound", "2025-09-15T01:00", "0.5"),
            "542800E0",
        ),
        valued(
            ledger_row("t5", "BTC", "A", "outbound", "2025-11-30T01:00", "3"),
            "2895000E0",
        ),
    ]
}

/// Each disposal's cost, plain.
fn costs(response: &Value) -> Vec<String> {
    realized(response)
        .into_iter()
        .map(|row| row[4].clone())
        .collect()
}

#[test]
fn the_total_average_prices_every_disposal_of_a_year_alike() {
    // 4,037,800 ÷ 6.5 = 621,200 a unit, whichever wallet sells: 2 and 3 BTC.
    let response = ledger_run(
        tokyo(),
        "jp_total_average",
        "global",
        faq_2_4_book(),
        vec![],
        vec![],
    );
    assert_eq!(costs(&response), ["1242400", "1863600"]);
    // The year's later purchase counts for the earlier sale too; the 1.5
    // BTC left are worth 931,800 (each open lot at the closing unit cost).
    let holdings: Vec<[String; 3]> = holdings(&response);
    let total: f64 = holdings.iter().map(|h| h[2].parse::<f64>().unwrap()).sum();
    assert!((total - 931_800.0).abs() < 1e-6, "{holdings:?}");
}

#[test]
fn the_moving_average_rounds_the_book_left_up_to_the_yen() {
    // jp-05: 3,080,200 in all (1,165,000 then 1,915,200).
    let response = ledger_run(
        tokyo(),
        "jp_moving_average",
        "global",
        faq_2_4_book(),
        vec![],
        vec![],
    );
    assert_eq!(costs(&response), ["1165000", "1915200"]);
    // jp-08: 1,000,000 for 3 BTC, 1 sold: the 2 left are ROUNDUP(666,666.67)
    // = 666,667, so the sale costs 333,333.
    let rows = vec![
        valued(
            ledger_row("t1", "BTC", "A", "inbound", "2025-01-10T01:00", "3"),
            "1000000E0",
        ),
        valued(
            ledger_row("t2", "BTC", "A", "outbound", "2025-02-10T01:00", "1"),
            "50000070E-2",
        ),
    ];
    let moving = ledger_run(
        tokyo(),
        "jp_moving_average",
        "global",
        rows.clone(),
        vec![],
        vec![],
    );
    assert_eq!(costs(&moving), ["333333"]);
    // jp-07: the total average keeps the exact third.
    let total = ledger_run(tokyo(), "jp_total_average", "global", rows, vec![], vec![]);
    assert_eq!(costs(&total), ["333333.33333333333333333333333333"]);
}

#[test]
fn the_total_average_carries_the_year_end_value_unrounded() {
    // jp-06: 2024 ends with 1.5 BTC at 931,800; 2025 adds 0.5 BTC for
    // 5,000,000, so 1 BTC sold costs (931,800 + 5,000,000) ÷ 2.
    let mut rows = faq_2_4_book();
    for row in &mut rows {
        let raw = row["ts"]["raw"].as_str().unwrap().replace("2025", "2024");
        let at = format!("{}T{}", &raw[..10], &raw[11..16]);
        let id = row["anchor"].as_str().unwrap().to_owned();
        let direction = row["direction"].as_str().unwrap().to_owned();
        let wallet = row["wallet_id"].as_str().unwrap().to_owned();
        let amount = plain(&json!(format!("{}E-11", row["amount_msat"])));
        let value = row["value"].clone();
        *row = ledger_row(&id, "BTC", &wallet, &direction, &at, &amount);
        row["value"] = value;
    }
    rows.push(valued(
        ledger_row("t6", "BTC", "A", "inbound", "2025-02-01T01:00", "0.5"),
        "5000000E0",
    ));
    rows.push(valued(
        ledger_row("t7", "BTC", "A", "outbound", "2025-08-01T01:00", "1"),
        "15000000E0",
    ));
    let response = ledger_run(tokyo(), "jp_total_average", "global", rows, vec![], vec![]);
    assert_eq!(costs(&response), ["1242400", "1863600", "2965900"]);
}

#[test]
fn japans_averages_pool_every_wallet() {
    let response = ledger_run(
        tokyo(),
        "jp_total_average",
        "wallet",
        faq_2_4_book(),
        vec![],
        vec![],
    );
    assert_eq!(response["ok"], false, "{response}");
    let generic_scope = json!({"kind": "rules", "timezone": "Asia/Tokyo", "holding": "never"});
    let response = ledger_run(
        generic_scope,
        "jp_moving_average",
        "wallet",
        faq_2_4_book(),
        vec![],
        vec![],
    );
    assert!(
        response["error"]["inner"]["message"]
            .as_str()
            .unwrap()
            .contains("averages over every wallet"),
        "{response}"
    );
}
