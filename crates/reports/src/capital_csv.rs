//! The desktop capital-gains CSV is a record projection, without a saved marker.
use super::*;
use artifact_csv::{File, Prepared};
const HEADERS: &[&str] = &[
    "occurred_at",
    "wallet",
    "transaction_id",
    "entry_type",
    "asset",
    "quantity",
    "quantity_msat",
    "proceeds",
    "cost_basis",
    "gain_loss",
    "description",
    "at_category",
    "at_kennzahl",
];

pub(super) fn prepare(conn: &Connection, profile: &Value, args: &Value) -> Result<Prepared> {
    let raw = value(args, "year");
    let year = if matches!(raw, Value::Null) {
        None
    } else {
        Some(austrian::normalize_tax_year(raw)?)
    };
    let mut records = artifact_csv::list(&capital_gains(conn, profile)?).to_vec();
    if let Some(year) = year {
        if kassiber_pytext::lower(&text(profile, "tax_country")) == "at" {
            let (start, end) = calendar::window(year);
            records.retain(|r| {
                let at = text(r, "occurred_at");
                at >= start && at < end
            });
            let mut selected = Vec::new();
            for row in records {
                if calendar::vienna(&text(&row, "occurred_at"))?.0 == year {
                    selected.push(row);
                }
            }
            records = selected;
        } else {
            records.retain(|r| {
                text(r, "occurred_at").chars().take(4).collect::<String>() == year.to_string()
            });
        }
    }
    let mut rows: Vec<Vec<Value>> = vec![HEADERS.iter().map(|h| s(*h)).collect()];
    rows.extend(
        records
            .iter()
            .map(|r| HEADERS.iter().map(|h| value(r, h).clone()).collect()),
    );
    Ok(Prepared {
        files: vec![File {
            name: "capital-gains.csv".into(),
            sheet: "Capital gains".into(),
            bytes: artifact_csv::encode(&rows),
            rows: records.len(),
        }],
        metadata: dict([("rows", Value::Int(records.len().into()))]),
        completion: None,
    })
}
