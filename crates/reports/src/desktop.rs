//! Desktop report transports over the shared stored-journal service.
use super::{int, value};
use kassiber_envelope::{dict, get};
use kassiber_pyjson::Value;

use std::collections::BTreeMap;
fn number(v: &Value, key: &str) -> f64 {
    match get(v, key) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => v.to_string().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}
pub fn holdings(rows: Vec<Value>) -> Value {
    let mut totals: BTreeMap<String, Value> = BTreeMap::new();
    for row in &rows {
        let asset = get(row, "asset").cloned().unwrap_or(Value::Null);
        let name = kassiber_envelope::py_str(&asset);
        let bucket = totals.entry(name).or_insert_with(|| {
            dict([
                ("asset", asset),
                ("quantity", Value::Float(0.0)),
                ("quantity_msat", Value::Int(0.into())),
                ("cost_basis", Value::Float(0.0)),
                ("market_value", Value::Float(0.0)),
                ("unrealized_pnl", Value::Float(0.0)),
            ])
        });
        if let Value::Dict(fields) = bucket {
            for (k, v) in fields {
                let kassiber_pyjson::Key::Str(k) = k else {
                    continue;
                };
                let key = k.as_str().unwrap_or_default();
                if key == "quantity_msat" {
                    if let (Value::Int(n), Some(Value::Int(add))) = (v, get(row, key)) {
                        *n += add;
                    }
                } else if ["quantity", "cost_basis", "market_value", "unrealized_pnl"]
                    .contains(&key)
                {
                    *v = Value::Float(
                        match v {
                            Value::Float(n) => *n,
                            _ => 0.0,
                        } + number(row, key),
                    );
                }
            }
        }
    }
    for bucket in totals.values_mut() {
        let sat = number(bucket, "quantity_msat") / 1000.0;
        if let Value::Dict(fields) = bucket {
            fields.push(("quantity_sat".into(), Value::Float(sat)));
        }
    }
    let summary = dict([
        ("row_count", Value::Int(rows.len().into())),
        ("asset_count", Value::Int(totals.len().into())),
    ]);
    dict([
        ("rows", Value::List(rows)),
        (
            "totals_by_asset",
            Value::List(totals.into_values().collect()),
        ),
        ("summary", summary),
    ])
}

pub fn tax_summary(mut rows: Vec<Value>, year: &Value) -> Value {
    let mut years: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            get(row, "year")
                .filter(|v| matches!(v, Value::Int(n) if n >= &0.into()))
                .cloned()
        })
        .collect();
    years.sort_by(|a, b| match (a, b) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        _ => unreachable!(),
    });
    years.dedup();
    if !matches!(year, Value::Null) {
        rows.retain(|row| get(row, "year") == Some(year));
    }
    let summary = dict([
        ("row_count", Value::Int(rows.len().into())),
        ("available_year_count", Value::Int(years.len().into())),
    ]);
    dict([
        ("rows", Value::List(rows)),
        ("available_years", Value::List(years)),
        ("filters", dict([("year", year.clone())])),
        ("summary", summary),
    ])
}
pub fn history(mut rows: Vec<Value>, args: &Value) -> Value {
    let total = rows.len();
    let limit = int(args, "limit") as usize;
    if rows.len() > limit {
        rows.drain(..rows.len() - limit);
    }
    dict([
        ("rows", Value::List(rows.clone())),
        (
            "filters",
            dict(
                [
                    "interval", "start", "end", "wallet", "account", "asset", "limit",
                ]
                .map(|k| (k, value(args, k).clone())),
            ),
        ),
        (
            "summary",
            dict([
                ("row_count", Value::Int(rows.len().into())),
                ("total_row_count", Value::Int(total.into())),
                ("truncated", Value::Bool(total > rows.len())),
            ]),
        ),
    ])
}
