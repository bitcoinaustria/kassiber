//! Desktop report transports over the shared stored-journal service.
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
