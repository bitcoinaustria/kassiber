use super::*;
use artifact_csv::list;
use kassiber_xlsx::{Cell, Document, Operation, Sheet, Style};
fn s(raw: impl AsRef<str>) -> Value {
    super::s(raw.as_ref())
}

fn cell(ops: &mut Vec<Operation>, row: usize, col: usize, value: Cell, style: usize) {
    ops.push(Operation::Cell {
        row: row as u32,
        col: col as u16,
        value,
        style,
    });
}
fn string(ops: &mut Vec<Operation>, row: usize, col: usize, text: impl Into<String>, style: usize) {
    cell(ops, row, col, Cell::String(text.into()), style);
}
fn numeric_cell(value: &Value) -> Result<Cell> {
    let raw = py_str(value);
    let non_finite = raw.trim_start_matches(['+', '-']).starts_with("NaN")
        || raw.parse::<f64>().is_ok_and(|v| !v.is_finite());
    if non_finite {
        return Err(AppError::new(
            "internal_error",
            "NAN/INF not supported in write_number() without 'nan_inf_to_errors' Workbook() option",
        ));
    }
    artifact_xlsx::cell(&float(&decimal(value)?)?)
}
pub(super) fn document(report: &Value, created: String) -> Result<Document> {
    let label = text(report, "tool_label");
    let fiat = text(report, "book_currency");
    let mut styles = artifact_xlsx::styles();
    let numeric = Style {
        font_size: Some(11.0),
        valign: Some("top".into()),
        num_format: Some("#,##0.00000000###".into()),
        ..Style::default()
    };
    styles.push(numeric.clone());
    styles.push(Style {
        num_format: Some("#,##0.00".into()),
        ..numeric
    });
    let mut ops = vec![
        Operation::Column {
            first: 0,
            last: 0,
            width: 28.0,
        },
        Operation::Column {
            first: 1,
            last: 4,
            width: 20.0,
        },
    ];
    string(&mut ops, 0, 0, format!("Kassiber vs {label}"), 0);
    let mut facts = vec![
        ("File".into(), value(report, "file_name").clone()),
        (
            "Year".into(),
            s(if truthy(value(report, "year")) {
                text(report, "year")
            } else {
                "All years".into()
            }),
        ),
        (
            "Calendar".into(),
            s(if truthy(value(report, "calendar")) {
                text(report, "calendar")
            } else {
                "UTC".into()
            }),
        ),
        (
            "Wallets".into(),
            s(if truthy(value(report, "wallet")) {
                text(report, "wallet")
            } else {
                "All wallets".into()
            }),
        ),
        (
            "Tax-relevant differences".into(),
            value(report, "tax_differences").clone(),
        ),
    ];
    facts.extend(cross_check::STATUSES.map(|status| {
        let label = status.replace('_', " ");
        let mut chars = label.chars();
        let label = chars.next().unwrap().to_uppercase().collect::<String>() + chars.as_str();
        (label, value(value(report, "counts"), status).clone())
    }));
    let mut line = 2;
    for (label, v) in facts {
        string(&mut ops, line, 0, label, 1);
        cell(&mut ops, line, 1, artifact_xlsx::cell(&v)?, 2);
        line += 1;
    }
    line += 1;
    for (i, label) in ["Asset", "Total", &label, "Kassiber", "Difference"]
        .into_iter()
        .enumerate()
    {
        string(&mut ops, line, i, label, 1);
    }
    for total in list(value(report, "totals")) {
        line += 1;
        let metric = text(total, "metric");
        let style = if metric == "income_value" { 8 } else { 7 };
        let metric = if metric == "income_value" {
            format!("Income value ({fiat})")
        } else {
            let mut c = metric.chars();
            c.next().unwrap().to_uppercase().collect::<String>() + c.as_str()
        };
        string(&mut ops, line, 0, text(total, "asset"), 2);
        string(&mut ops, line, 1, metric, 2);
        for (col, key) in [(2, "tool"), (3, "kassiber"), (4, "delta")] {
            if value(total, key) == &s("") {
                string(&mut ops, line, col, "not in file", 2);
            } else {
                cell(&mut ops, line, col, numeric_cell(value(total, key))?, style);
            }
        }
    }
    let pairs = list(value(report, "pairs"));
    if !pairs.is_empty() {
        line += 2;
        for (i, label) in [
            "Kassiber wallet",
            &format!("{label} name"),
            "State",
            "Rows",
            "To review",
        ]
        .into_iter()
        .enumerate()
        {
            string(&mut ops, line, i, label, 1);
        }
        for pair in pairs {
            line += 1;
            string(
                &mut ops,
                line,
                0,
                if truthy(value(pair, "wallet")) {
                    text(pair, "wallet")
                } else {
                    String::new()
                },
                2,
            );
            string(
                &mut ops,
                line,
                1,
                list(value(pair, "tool_names"))
                    .iter()
                    .map(py_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                2,
            );
            string(&mut ops, line, 2, text(pair, "state").replace('_', " "), 2);
            cell(
                &mut ops,
                line,
                3,
                artifact_xlsx::cell(value(pair, "rows"))?,
                2,
            );
            cell(
                &mut ops,
                line,
                4,
                artifact_xlsx::cell(value(pair, "issues"))?,
                2,
            );
        }
    }
    let insights = list(value(report, "insights"));
    if !insights.is_empty() {
        line += 2;
        string(&mut ops, line, 0, "Notes", 1);
        for i in insights {
            line += 1;
            string(&mut ops, line, 0, text(i, "message"), 2);
        }
    }
    let mut sheets = vec![Sheet {
        name: "Summary".into(),
        operations: ops,
    }];
    let columns = list(value(cross_check_csv::metadata(), "columns"));
    for spec in list(value(cross_check_csv::metadata(), "sheets")) {
        let spec = list(spec);
        let statuses = list(&spec[1]);
        let mut ops = vec![Operation::Freeze { row: 1, col: 0 }];
        for (col, spec) in columns.iter().enumerate() {
            let spec = list(spec);
            let label = py_str(&spec[1])
                .replace("{tool}", &label)
                .replace("{fiat}", &fiat);
            let width = py_str(&spec[2]).parse().unwrap();
            ops.push(Operation::Column {
                first: col as u16,
                last: col as u16,
                width,
            });
            string(&mut ops, 0, col, label, 1);
        }
        let rows = list(value(report, "rows"))
            .iter()
            .filter(|r| statuses.contains(value(r, "status")))
            .collect::<Vec<_>>();
        for (i, r) in rows.iter().enumerate() {
            let t = value(r, "tool");
            let b = value(r, "kassiber");
            let mut differences = list(value(r, "differences"))
                .iter()
                .map(|d| text(d, "message"))
                .collect::<Vec<_>>()
                .join("; ");
            if truthy(value(r, "reason")) {
                differences = text(r, "reason");
            }
            for (col, spec) in columns.iter().enumerate() {
                let key = py_str(&list(spec)[0]);
                let fallback = |primary: &str, secondary: &str| {
                    if truthy(value(b, primary)) {
                        value(b, primary).clone()
                    } else {
                        value(t, secondary).clone()
                    }
                };
                let v = match key.as_str() {
                    "status" => s(text(r, "status").replace('_', " ")),
                    "match" => {
                        if truthy(value(r, "match")) {
                            value(r, "match").clone()
                        } else {
                            s("")
                        }
                    }
                    "differences" => s(&differences),
                    "tool_row" => value(t, "row").clone(),
                    "tool_source" => value(t, "source").clone(),
                    "tool_type" => value(t, "type").clone(),
                    "tool_date" => value(t, "occurred_at").clone(),
                    "direction" => fallback("direction", "direction"),
                    "asset" => fallback("asset", "asset"),
                    "wallet" => value(b, "wallet").clone(),
                    "classification" => value(b, "classification").clone(),
                    "book_date" => value(b, "occurred_at").clone(),
                    "txid" => fallback("txid", "txid"),
                    "kassiber_ref" => s(if matches!(b, Value::Null) {
                        String::new()
                    } else {
                        text(b, "ref").chars().take(12).collect()
                    }),
                    k if k.starts_with("tool_") => value(t, &k[5..]).clone(),
                    k => value(b, &k[5..]).clone(),
                };
                let numeric =
                    ["tool_amount", "tool_fee", "book_amount", "book_fee"].contains(&key.as_str());
                let money = ["tool_value", "book_value"].contains(&key.as_str());
                let style = if !matches!(v, Value::Null) && v != s("") && (numeric || money) {
                    if money {
                        8
                    } else {
                        7
                    }
                } else {
                    2
                };
                let cell_value = if style != 2 {
                    numeric_cell(&v)?
                } else {
                    artifact_xlsx::cell(&v)?
                };
                cell(&mut ops, i + 1, col, cell_value, style);
            }
        }
        ops.push(Operation::Autofilter {
            first_row: 0,
            first_col: 0,
            last_row: rows.len().max(1) as u32,
            last_col: columns.len() as u16 - 1,
        });
        sheets.push(Sheet {
            name: py_str(&spec[0]),
            operations: ops,
        });
    }
    Ok(Document {
        title: format!("Kassiber vs {label}"),
        subject: "Cross-check of a tool export against the Kassiber book".into(),
        author: "Kassiber".into(),
        comments: String::new(),
        created,
        formats: styles,
        sheets,
        names: vec![],
    })
}
pub(super) fn publish(report: &Value, path: &std::path::Path, created: String) -> Result<Value> {
    export::create_directory(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new(".")),
    )?;
    let bytes = kassiber_xlsx::render(&document(report, created)?)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    std::fs::write(path, &bytes).map_err(|e| export::io_error(e, Some(path)))?;
    let absolute = path
        .canonicalize()
        .map_err(|e| export::io_error(e, Some(path)))?;
    Ok(dict([
        ("file", s(absolute.to_string_lossy())),
        ("bytes", Value::Int(bytes.len().into())),
        ("tool", value(report, "tool").clone()),
        ("year", value(report, "year").clone()),
        ("counts", value(report, "counts").clone()),
        ("tax_differences", value(report, "tax_differences").clone()),
        ("agrees", value(report, "agrees").clone()),
    ]))
}
