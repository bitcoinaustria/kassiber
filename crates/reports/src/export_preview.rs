//! `preview_export`: what an export would write, without writing it. Each
//! preview runs its export's own builder on a read snapshot, so it fails
//! closed exactly as the export does and never publishes, fetches or syncs.
use super::*;
use kassiber_pydecimal::Rounding;
use kassiber_setup::Sources;

const KIND: &str = "ui.exports.preview";
const ROWS: usize = 8;
const EXPORTS: &[&str] = &["transactions", "year_handoff", "labels"];
const FORMATS: &[&str] = &["xlsx", "csv", "cointracking", "blockpit"];
const INT_FIELDS: &[&str] = &[
    "amount_msat",
    "count",
    "fee_msat",
    "fee_amount_msat",
    "gap_limit",
    "in_amount_msat",
    "in_fee_msat",
    "inbound_amount_msat",
    "inbound_count",
    "out_amount_msat",
    "out_fee_msat",
    "outbound_amount_msat",
    "outbound_count",
    "quantity_msat",
];
const MONEY_FIELDS: &[&str] = &[
    "avg_cost",
    "cost_basis",
    "cumulative_cost_basis",
    "fiat_fee",
    "fiat_price",
    "fiat_value",
    "fmv",
    "gain_loss",
    "market_value",
    "proceeds",
    "unrealized_pnl",
];
const QUANTITY_FIELDS: &[&str] = &[
    "amount",
    "fee",
    "fee_amount",
    "in_amount",
    "in_fee",
    "inbound_amount",
    "out_amount",
    "out_fee",
    "outbound_amount",
    "quantity",
];

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
fn formatting(_: kassiber_pydecimal::DecimalError) -> AppError {
    AppError::new("internal_error", "number formatting failed")
}
/// Insert `,` thousands separators into a fixed-point number.
fn grouped(fixed: String) -> String {
    let (sign, body) = match fixed.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", fixed.as_str()),
    };
    let (int, frac) = match body.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (body, None),
    };
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    match frac {
        Some(frac) => format!("{sign}{out}.{frac}"),
        None => format!("{sign}{out}"),
    }
}
/// `_text(value, number_format)`.
fn display(v: &Value, format: &str) -> Result<String> {
    match v {
        Value::Null => return Ok(String::new()),
        Value::Str(t) if t.as_str() == Some("") => return Ok(String::new()),
        Value::Bool(b) => return Ok(if *b { "TRUE" } else { "FALSE" }.into()),
        Value::Int(_) | Value::Float(_) | Value::Decimal(_) if format != "text" => {}
        Value::Decimal(d) => {
            return d
                .normalize(&kassiber_pydecimal::Context::new(28, Rounding::HalfEven))
                .and_then(|n| n.to_fixed_string())
                .map_err(formatting)
        }
        _ => return Ok(py_str(v)),
    }
    let number: Decimal = match v {
        Value::Decimal(d) => d.clone(),
        _ => py_str(v)
            .parse()
            .map_err(|_| AppError::new("internal_error", "number formatting failed"))?,
    };
    Ok(match format {
        "money" => grouped(
            number
                .to_fixed_places(2, Rounding::HalfEven)
                .map_err(formatting)?,
        ),
        "int" => grouped(
            number
                .to_fixed_places(0, Rounding::HalfEven)
                .map_err(formatting)?,
        ),
        "sats" => grouped(
            number
                .normalize(&kassiber_pydecimal::Context::new(28, Rounding::HalfEven))
                .and_then(|n| n.to_fixed_string())
                .map_err(formatting)?,
        ),
        _ => number
            .to_fixed_places(8, Rounding::HalfEven)
            .map_err(formatting)?,
    })
}
fn transaction_format(header: &str) -> &'static str {
    if INT_FIELDS.contains(&header) {
        "int"
    } else if MONEY_FIELDS.contains(&header) {
        "money"
    } else if QUANTITY_FIELDS.contains(&header) {
        "quantity"
    } else {
        "text"
    }
}
fn sheet(name: &str, columns: Vec<String>, rows: Vec<Vec<Value>>, total: usize) -> Value {
    dict([
        ("name", s(name)),
        ("columns", Value::List(columns.into_iter().map(s).collect())),
        (
            "rows",
            Value::List(rows.into_iter().take(ROWS).map(Value::List).collect()),
        ),
        ("total", Value::Int(total.into())),
    ])
}
fn optional_string(args: &Value, name: &str) -> Result<()> {
    if !matches!(value(args, name), Value::Null | Value::Str(_)) {
        return Err(validation(format!("{KIND} {name} must be a string")));
    }
    Ok(())
}

/// The BIP329 records a labels export would write for these wallet refs.
pub type Labels<'a> = dyn Fn(&Value, &Option<Vec<String>>) -> Result<Vec<Value>> + 'a;

pub fn preview(
    conn: &Connection,
    raw: &Value,
    sources: &dyn Sources,
    labels: &Labels<'_>,
) -> Result<Value> {
    let Value::Dict(fields) = raw else {
        return Err(validation("daemon args must be an object")
            .with_details(dict([("type", s(kassiber_envelope::type_name(raw)))])));
    };
    let allowed = [
        "export", "format", "wallet", "wallets", "year", "range", "from", "to", "timezone",
    ];
    let mut unknown: Vec<_> = fields
        .iter()
        .filter_map(|(k, _)| match k {
            kassiber_pyjson::Key::Str(k) if !allowed.iter().any(|a| k.as_str() == Some(a)) => {
                Some(k.clone())
            }
            _ => None,
        })
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(
            validation(format!("{KIND} received unsupported arguments")).with_details(dict([(
                "unsupported",
                Value::List(unknown.into_iter().map(Value::Str).collect()),
            )])),
        );
    }
    for name in ["export", "format"] {
        optional_string(raw, name)?;
    }
    let export = if truthy(value(raw, "export")) {
        text(raw, "export")
    } else {
        String::new()
    };
    let year = value(raw, "year").clone();
    if export == "year_handoff" && !matches!(year, Value::Int(_)) {
        return Err(validation(format!("{KIND} requires an integer year")));
    }
    let wallets = export_request::wallet_args(KIND, raw)?;
    let refs: Option<Vec<String>> = match &wallets {
        Value::List(items) => Some(items.iter().map(py_str).collect()),
        _ => None,
    };
    let range_args = ["range", "from", "to", "timezone"];
    for name in range_args {
        let v = value(raw, name);
        if !matches!(v, Value::Null | Value::Str(_)) {
            return Err(validation(format!("{KIND} {name} must be a string")));
        }
        if matches!(v, Value::Str(t) if t.as_str().is_some_and(|t| kassiber_pytext::strip(t).is_empty()))
        {
            return Err(validation(format!("{KIND} {name} must not be empty")));
        }
    }
    let mut scope_args = dict([("workspace", Value::Null), ("profile", Value::Null)]);
    let range = if range_args
        .iter()
        .all(|k| matches!(value(raw, k), Value::Null))
    {
        None
    } else {
        let (_, profile) = kassiber_setup::references::scope(conn, &scope_args)?;
        export_request::range(conn, &profile, raw, sources)?
    };
    if !EXPORTS.contains(&export.as_str()) {
        return Err(validation(format!("Unknown export '{export}'"))
            .with_hint(format!("Use one of: {}.", EXPORTS.join(", "))));
    }
    let mut format = value(raw, "format").clone();
    conn.execute_batch("SAVEPOINT export_preview_read")
        .map_err(sql_error)?;
    let result = (|| -> Result<Vec<(&'static str, Value)>> {
        match export.as_str() {
            "transactions" => {
                let chosen = if truthy(&format) {
                    py_str(&format)
                } else {
                    "xlsx".into()
                };
                format = s(chosen.as_str());
                if !FORMATS.contains(&chosen.as_str()) {
                    return Err(validation(format!(
                        "Unknown transaction export format '{chosen}'"
                    ))
                    .with_hint(format!("Use one of: {}.", FORMATS.join(", "))));
                }
                replace(&mut scope_args, "wallets", wallets.clone());
                if let Some(range) = &range {
                    replace(&mut scope_args, "export_range", range.payload()?);
                }
                let suffix = match &range {
                    Some(range) => format!(
                        "-{}",
                        range
                            .label()
                            .map_err(|e| AppError::new("internal_error", e))?
                    ),
                    None => String::new(),
                };
                if chosen == "cointracking" || chosen == "blockpit" {
                    replace(&mut scope_args, "platform", s(chosen.as_str()));
                    let (_, profile) = kassiber_setup::references::scope(conn, &scope_args)?;
                    let (table, metadata) = platform_csv::table(conn, &profile, &scope_args)?;
                    let headers: Vec<String> = table[0].iter().map(py_str).collect();
                    let rows: Vec<Vec<Value>> = table[1..].to_vec();
                    let count = rows.len();
                    let label = if chosen == "cointracking" {
                        "CoinTracking CSV"
                    } else {
                        "Blockpit CSV"
                    };
                    return Ok(vec![
                        ("file_name", s(format!("kassiber-{chosen}{suffix}.csv"))),
                        ("row_count", Value::Int(count.into())),
                        (
                            "sheets",
                            Value::List(vec![sheet(label, headers, rows, count)]),
                        ),
                        ("warnings", value(&metadata, "warnings").clone()),
                    ]);
                }
                let (_, profile) = kassiber_setup::references::scope(conn, &scope_args)?;
                let (section, metadata) = standalone_csv::project(conn, &profile, &scope_args)?;
                let count = section.rows.len();
                let rows = section
                    .rows
                    .iter()
                    .take(ROWS)
                    .map(|row| {
                        section
                            .headers
                            .iter()
                            .map(|h| display(value(row, h), transaction_format(h)).map(s))
                            .collect::<Result<Vec<_>>>()
                    })
                    .collect::<Result<Vec<_>>>()?;
                let columns = section
                    .headers
                    .iter()
                    .map(|h| artifact_csv::label(h))
                    .collect();
                Ok(vec![
                    (
                        "file_name",
                        s(format!("kassiber-transactions{suffix}.{chosen}")),
                    ),
                    ("row_count", Value::Int(count.into())),
                    (
                        "sheets",
                        Value::List(vec![sheet("Transactions", columns, rows, count)]),
                    ),
                    ("warnings", value(&metadata, "warnings").clone()),
                ])
            }
            "year_handoff" => {
                replace(&mut scope_args, "year", year.clone());
                let report = year_handoff::build(conn, &scope_args, sources)?;
                let profile = value(&report, "profile");
                let fiat = if truthy(value(profile, "fiat_currency")) {
                    text(profile, "fiat_currency")
                } else {
                    String::new()
                };
                let keys = [
                    "overview",
                    "wallets",
                    "transactions",
                    "journal",
                    "balances",
                    "roll_forward",
                    "reconciliation",
                    "excluded",
                ];
                let overview = year_handoff_xlsx::overview(&report);
                let mut sheets = Vec::new();
                for (i, spec) in artifact_csv::list(value(year_handoff_xlsx::metadata(), "sheets"))
                    .iter()
                    .enumerate()
                {
                    let rows = if i == 0 {
                        overview.as_slice()
                    } else {
                        artifact_csv::list(value(&report, keys[i]))
                    };
                    let columns = artifact_csv::list(value(spec, "columns"));
                    let labels = columns
                        .iter()
                        .map(|c| text(c, "label").replace("{fiat}", &fiat))
                        .collect();
                    let cells = rows
                        .iter()
                        .take(ROWS)
                        .map(|row| {
                            columns
                                .iter()
                                .map(|c| {
                                    display(value(row, &text(c, "key")), &text(c, "format")).map(s)
                                })
                                .collect::<Result<Vec<_>>>()
                        })
                        .collect::<Result<Vec<_>>>()?;
                    sheets.push(sheet(&text(spec, "name"), labels, cells, rows.len()));
                }
                let year = value(&report, "year").clone();
                Ok(vec![
                    (
                        "file_name",
                        s(format!("kassiber-year-handoff-{}.xlsx", py_str(&year))),
                    ),
                    (
                        "row_count",
                        Value::Int(
                            artifact_csv::list(value(&report, "transactions"))
                                .len()
                                .into(),
                        ),
                    ),
                    ("sheets", Value::List(sheets)),
                    ("warnings", value(&report, "warnings").clone()),
                    ("reconciled", value(&report, "reconciled").clone()),
                    ("year", year),
                ])
            }
            _ => {
                let (_, profile) = kassiber_setup::references::scope(conn, &scope_args)?;
                let records = labels(&profile, &refs)?;
                let rows = records
                    .iter()
                    .take(ROWS)
                    .map(|record| {
                        let kind = if truthy(value(record, "type")) {
                            text(record, "type")
                        } else {
                            String::new()
                        };
                        let reference = if truthy(value(record, "ref")) {
                            text(record, "ref")
                        } else {
                            String::new()
                        };
                        let shown = if ["xpub", "pubkey"].contains(&kind.as_str()) {
                            let chars: Vec<char> = reference.chars().collect();
                            if chars.len() > 14 {
                                format!(
                                    "{}…{}",
                                    chars[..6].iter().collect::<String>(),
                                    chars[chars.len() - 4..].iter().collect::<String>()
                                )
                            } else {
                                "…".into()
                            }
                        } else {
                            reference
                        };
                        let label = if truthy(value(record, "label")) {
                            text(record, "label")
                        } else {
                            String::new()
                        };
                        vec![s(kind), s(shown), s(label)]
                    })
                    .collect();
                Ok(vec![
                    ("file_name", s("kassiber-labels.jsonl")),
                    ("row_count", Value::Int(records.len().into())),
                    (
                        "sheets",
                        Value::List(vec![sheet(
                            "BIP 329",
                            vec!["type".into(), "ref".into(), "label".into()],
                            rows,
                            records.len(),
                        )]),
                    ),
                    ("warnings", Value::List(vec![])),
                ])
            }
        }
    })();
    conn.execute_batch("ROLLBACK TO export_preview_read; RELEASE export_preview_read")
        .map_err(sql_error)?;
    let mut preview = result?;
    if !preview.iter().any(|(k, _)| *k == "reconciled") {
        preview.push(("reconciled", Value::Null));
    }
    let range = match (&range, export.as_str()) {
        (Some(range), "transactions") => range.payload()?,
        _ => Value::Null,
    };
    let mut payload = vec![
        ("export", s(export.as_str())),
        ("format", format),
        ("wallets", wallets),
        ("range", range),
    ];
    payload.extend(preview);
    Ok(dict(payload))
}
