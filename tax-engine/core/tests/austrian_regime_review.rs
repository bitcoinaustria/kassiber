//! An Austrian sale's reviewed regime through the production schema-2 ledger
//! entry point: the regime a sale is reviewed as is the one it depletes.

use kassiber_tax_core::ledger;
use kassiber_tax_countries::calendar::{zone, Date};
use serde_json::{json, Value};

const BTC: u64 = 100_000_000_000;

fn timestamp(day: &str, hour: u32, offset_hours: i32) -> Value {
    let date = Date::parse(day).unwrap();
    let seconds =
        date.to_days() * 86_400 + i64::from(hour) * 3_600 - i64::from(offset_hours) * 3_600;
    let us = seconds * 1_000_000;
    let offset = format!("{offset_hours:+03}:00");
    let iso = format!("{day}T{hour:02}:00:00{offset}");
    let raw = if offset_hours == 0 {
        format!("{day}T{hour:02}:00:00Z")
    } else {
        iso.clone()
    };
    json!({
        "raw": raw, "us": us, "offset_s": offset_hours * 3_600, "date": day,
        "year": date.year, "vienna_date": zone("Europe/Vienna").unwrap().local_date(us).unwrap().iso(),
        "display": iso.replace('T', " "), "iso": iso,
    })
}

fn row(id: &str, wallet: &str, direction: &str, ts: Value, btc: u64, unit_price: u64) -> Value {
    json!({
        "id": format!("custody-tax:{id}"), "anchor": id, "asset": "BTC", "wallet_id": wallet,
        "direction": direction, "ts": ts, "cutoff_instant": {"us": ts["us"]},
        "amount_msat": btc * BTC, "fee_msat": 0, "rate": format!("{unit_price}E0"),
        "value": null, "coarse": false, "kind": if direction == "inbound" { "buy" } else { "sell" },
        "text": id, "regime_override": null, "category_override": null,
        "taxability_off": false, "privacy_hop": false, "hold": null, "chain_order": null,
    })
}

fn request(austrian: bool, wallet_b: &str, rows: Vec<Value>) -> Value {
    let policy = if austrian {
        json!({"country":{"kind":"at"},"method":"moving_average_at","method_label":"MOVING_AVERAGE_AT","require_coarse_review":false})
    } else {
        json!({"country":{"kind":"us"},"method":"fifo","method_label":"FIFO","require_coarse_review":false,"pool_scope":"wallet"})
    };
    let exchanges: std::collections::BTreeSet<_> = ["Cold", wallet_b].into_iter().collect();
    json!({
        "schema_version":2, "operation":"ledger", "policy":policy,
        "config":{"assets":["BTC"],"exchanges":exchanges,"holders":["Default"]},
        "holder":"Default", "wallets":[{"id":"A","label":"Cold"},{"id":"B","label":wallet_b}],
        "rows":rows, "intra_pairs":[], "cross_asset_pairs":[],
    })
}

fn run(req: &Value) -> Value {
    serde_json::from_str(&ledger(&req.to_string())).unwrap()
}

fn disposals(response: &Value) -> Vec<Value> {
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["quarantines"], json!([]), "{response}");
    response["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["entry_type"] == "disposal")
        .map(|entry| {
            json!([
                entry["at_category"],
                entry["cost_basis"],
                entry["gain_loss"]
            ])
        })
        .collect()
}

#[test]
fn a_reviewed_alt_sale_leaves_neu_inventory_for_the_next_sale() {
    let mut first = row(
        "first",
        "A",
        "outbound",
        timestamp("2025-03-09", 0, 0),
        1,
        600,
    );
    first["regime_override"] = json!("alt");
    let mut req = request(
        true,
        "Hot",
        vec![
            row("alt", "A", "inbound", timestamp("2020-01-01", 0, 0), 2, 100),
            row("neu", "A", "inbound", timestamp("2024-09-09", 0, 0), 1, 500),
            first,
            row(
                "second",
                "A",
                "outbound",
                timestamp("2025-04-09", 0, 0),
                1,
                600,
            ),
        ],
    );
    // One Alt and one Neu unit remain after selling one of the two Alt
    // units. Kassiber's documented default in this mixed wallet is Neu.
    let actual = run(&req);
    req["rows"][3]["regime_override"] = json!("neu");
    let expected = vec![
        json!(["alt_taxfree", "100E0", "500E0"]),
        json!(["neu_gain", "500E0", "100E0"]),
    ];
    assert_eq!(disposals(&run(&req)), expected);
    assert_eq!(disposals(&actual), expected);
}
