//! Native workbook projections. Reuse the admitted transaction-list projection;
//! workbook bytes have no authority to establish accounting or saved markers.
use super::*;
use artifact_csv::{list, File, Prepared, Section};
use kassiber_xlsx::{Cell, Document, Operation, Sheet, Style};

pub(super) fn styles() -> Vec<Style> {
    let text = Style {
        font_size: Some(11.0),
        valign: Some("top".into()),
        text_wrap: true,
        ..Style::default()
    };
    vec![
        Style {
            bold: true,
            font_size: Some(14.0),
            valign: Some("vcenter".into()),
            ..Style::default()
        },
        Style {
            bold: true,
            bg_color: Some("#EFEFEF".into()),
            bottom: 1,
            ..text.clone()
        },
        text.clone(),
        Style {
            num_format: Some("0".into()),
            text_wrap: false,
            ..text.clone()
        },
        Style {
            num_format: Some("0.00000000".into()),
            text_wrap: false,
            ..text.clone()
        },
        Style {
            num_format: Some("#,##0.00".into()),
            text_wrap: false,
            ..text.clone()
        },
        Style {
            font_color: Some("#185FA5".into()),
            underline: 1,
            ..text
        },
    ]
}

fn format_name(header: &str, value: &Value) -> usize {
    if matches!(value, Value::Null) || value == &s("") {
        return 2;
    }
    if [
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
        "rows",
        "tx_count",
    ]
    .contains(&header)
    {
        3
    } else if [
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
    ]
    .contains(&header)
    {
        5
    } else if header.ends_with("_sat")
        || [
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
        ]
        .contains(&header)
    {
        4
    } else {
        2
    }
}

pub(super) fn cell(value: &Value) -> Result<Cell> {
    Ok(match value {
        Value::Null => Cell::Blank,
        Value::Bool(v) => Cell::Boolean(*v),
        Value::Int(_) | Value::Float(_) => Cell::Number(
            py_str(value)
                .parse()
                .map_err(|_| AppError::new("internal_error", "invalid workbook number"))?,
        ),
        _ if value == &s("") => Cell::Blank,
        _ => Cell::String(py_str(value)),
    })
}
fn length(value: &Value) -> usize {
    py_str(value).chars().count()
}
pub(super) fn lines(value: &Value, width: f64) -> usize {
    if matches!(value, Value::Null) || value == &s("") {
        return 1;
    }
    let width = (width as usize).saturating_sub(1).max(1);
    py_str(value)
        .split('\n')
        .map(|s| {
            s.trim_end_matches(|c: char| kassiber_pytext::is_space(u32::from(c)))
                .chars()
                .count()
                .div_ceil(width)
                .max(1)
        })
        .sum::<usize>()
        .max(1)
}

/// The Python generic report worksheet layout, shared by transaction lists and
/// later native report projections. Values remain typed, even in text columns.
pub(super) fn sheet(section: &Section) -> Result<Sheet> {
    let mut ops = vec![
        Operation::Landscape,
        Operation::Fit {
            width: 1,
            height: 0,
        },
        Operation::Margins {
            left: 0.35,
            right: 0.35,
            top: 0.5,
            bottom: 0.5,
        },
    ];
    let headers = section.headers;
    let widths: Vec<f64> = headers
        .iter()
        .map(|header| {
            let label = artifact_csv::label(header);
            let sample = section
                .rows
                .iter()
                .take(50)
                .map(|r| length(value(r, header)).min(36))
                .max()
                .unwrap_or(0)
                .max(label.chars().count() + 2);
            (sample + 2).clamp(10, 38) as f64
        })
        .collect();
    ops.push(Operation::Row {
        row: 0,
        height: 28.0,
    });
    if headers.len() == 1 {
        ops.push(Operation::Cell {
            row: 0,
            col: 0,
            value: Cell::String(section.title.clone()),
            style: 0,
        });
    } else {
        ops.push(Operation::Merge {
            first_row: 0,
            first_col: 0,
            last_row: 0,
            last_col: headers.len().saturating_sub(1) as u16,
            text: section.title.clone(),
            style: 0,
        });
    }
    let height = headers
        .iter()
        .enumerate()
        .map(|(i, h)| lines(&s(artifact_csv::label(h)), widths[i]))
        .max()
        .unwrap_or(1);
    ops.push(Operation::Row {
        row: 1,
        height: (height * 15 + 4).max(24) as f64,
    });
    for (col, header) in headers.iter().enumerate() {
        ops.push(Operation::Column {
            first: col as u16,
            last: col as u16,
            width: widths[col],
        });
        ops.push(Operation::Cell {
            row: 1,
            col: col as u16,
            value: Cell::String(artifact_csv::label(header)),
            style: 1,
        });
    }
    ops.push(Operation::Freeze { row: 2, col: 0 });
    for (i, row) in section.rows.iter().enumerate() {
        let row_index = u32::try_from(i + 2)
            .map_err(|_| AppError::new("internal_error", "workbook row limit exceeded"))?;
        let mut height = 1;
        for (col, header) in headers.iter().enumerate() {
            let raw = value(row, header);
            let mut style = format_name(header, raw);
            let cell = if *header == "attachments" && get(row, "attachments_list").is_some() {
                let entries = list(value(row, "attachments_list"));
                height = height.max(entries.len().max(1));
                if entries.len() == 1 && truthy(value(&entries[0], "url")) {
                    style = 6;
                    Cell::Link {
                        url: text(&entries[0], "url"),
                        text: text(&entries[0], "display_name"),
                    }
                } else if entries.is_empty() {
                    style = 2;
                    Cell::Blank
                } else {
                    style = 2;
                    Cell::String(
                        entries
                            .iter()
                            .map(|e| text(e, "display_name"))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                }
            } else {
                if style == 2 {
                    height = height.max(lines(raw, widths[col]));
                }
                cell(raw)?
            };
            ops.push(Operation::Cell {
                row: row_index,
                col: col as u16,
                value: cell,
                style,
            });
        }
        ops.push(Operation::Row {
            row: row_index,
            height: (height * 15 + 4).max(16) as f64,
        });
    }
    if section.rows.is_empty() {
        ops.push(Operation::Cell {
            row: 2,
            col: 0,
            value: Cell::String("No rows in scope.".into()),
            style: 2,
        });
    }
    ops.push(Operation::Autofilter {
        first_row: 1,
        first_col: 0,
        last_row: (section.rows.len() + 1).max(2) as u32,
        last_col: headers.len().saturating_sub(1) as u16,
    });
    Ok(Sheet {
        name: section.name.into(),
        operations: ops,
    })
}
pub fn prepare(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    if !matches!(
        command,
        "reports.export-transactions-xlsx"
            | "reports.export-austrian-e1kv-xlsx"
            | "reports.export-xlsx"
            | "reports.export-exit-tax-xlsx"
            | "reports.export-year-handoff"
    ) {
        return Err(AppError::new(
            "not_implemented",
            format!("{command} is not implemented in the Rust backend yet"),
        ));
    }
    let outermost = conn.is_autocommit();
    conn.execute_batch("SAVEPOINT report_xlsx_read")
        .map_err(sql_error)?;
    let result = (|| {
        if command == "reports.export-austrian-e1kv-xlsx" {
            return austrian_xlsx::prepare(conn, args, sources);
        }
        if command == "reports.export-exit-tax-xlsx" {
            return exit_xlsx::prepare(conn, args, sources);
        }
        if command == "reports.export-year-handoff" {
            return year_handoff_xlsx::prepare(conn, args, sources);
        }
        if command == "reports.export-xlsx" {
            return full_xlsx::prepare(conn, args, sources);
        }
        let (_, profile) = kassiber_setup::references::scope(conn, args)?;
        let (section, mut metadata) = standalone_csv::project(conn, &profile, args)?;
        let rows = section.rows.len();
        let document = Document {
            title: text(&metadata, "title"),
            subject: "Kassiber transactions export".into(),
            author: "Kassiber".into(),
            comments: "Transaction ledger with notes, tags, and linked evidence.".into(),
            created: sources.now(conn)?,
            formats: styles(),
            sheets: vec![sheet(&section)?],
            names: vec![],
        };
        let bytes = kassiber_xlsx::render(&document)
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        // Match Python's metadata insertion order, not merely dictionary equality.
        let range = value(&metadata, "range").clone();
        let warnings = value(&metadata, "warnings").clone();
        if let Value::Dict(fields) = &mut metadata {
            fields.retain(|(k, _)| {
                *k != "rows".into() && *k != "warnings".into() && *k != "range".into()
            });
        }
        replace(
            &mut metadata,
            "sheets",
            Value::List(vec![s("Transactions")]),
        );
        replace(&mut metadata, "rows", Value::Int(rows.into()));
        replace(&mut metadata, "warnings", warnings);
        replace(&mut metadata, "range", range);
        Ok(Prepared {
            files: vec![File {
                name: "report.xlsx".into(),
                sheet: String::new(),
                bytes,
                rows,
            }],
            metadata,
            completion: None,
        })
    })();
    kassiber_store::debug_assert_no_write(conn, outermost, "report_xlsx_read");
    let release = conn
        .execute_batch("ROLLBACK TO report_xlsx_read; RELEASE report_xlsx_read")
        .map_err(sql_error);
    match result {
        Ok(prepared) => {
            release?;
            Ok(prepared)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixed;
    impl kassiber_setup::Sources for Fixed {
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
        fn uuid(&self, _: &Connection) -> Result<String> {
            panic!("observed list requested an identity")
        }
    }
    #[test]
    fn transaction_xlsx_cells_preserve_types_formats_and_links() {
        let spec = Section {
            name: "Transactions",
            title: "Transactions".into(),
            headers: &["cost_basis", "amount_msat", "attachments", "note"],
            rows: vec![dict([
                ("cost_basis", Value::Float(-12345.6789)),
                ("amount_msat", Value::Int(1001.into())),
                ("note", Value::Bool(true)),
                (
                    "attachments_list",
                    Value::List(vec![dict([
                        ("display_name", s("Receipt €")),
                        ("url", s("https://example.invalid/receipt")),
                    ])]),
                ),
            ])],
        };
        let sheet = sheet(&spec).unwrap();
        let at = |col| {
            sheet
                .operations
                .iter()
                .find(|op| matches!(op, Operation::Cell { row: 2, col: c, .. } if *c == col))
                .unwrap()
        };
        assert!(
            matches!(at(0), Operation::Cell { value: Cell::Number(n), style: 5, .. } if *n == -12345.6789)
        );
        assert!(
            matches!(at(1), Operation::Cell { value: Cell::Number(n), style: 3, .. } if *n == 1001.0)
        );
        assert!(
            matches!(at(2), Operation::Cell { value: Cell::Link { url, text }, style: 6, .. } if url == "https://example.invalid/receipt" && text == "Receipt €")
        );
        assert!(matches!(
            at(3),
            Operation::Cell {
                value: Cell::Boolean(true),
                style: 2,
                ..
            }
        ));
    }
    #[test]
    fn transaction_xlsx_export_publishes_native_bytes_and_registers_nothing() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch("UPDATE profiles SET last_processed_input_version=-1 WHERE id='p'")
            .unwrap();
        let target = root.path().join("transactions.xlsx");
        let result = export::export(
            conn,
            "reports.export-transactions-xlsx",
            &dict::<&str>([]),
            &target,
            &Fixed,
        )
        .unwrap();
        let bytes = std::fs::read(target).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert_eq!(int(&result, "bytes"), bytes.len() as i64);
        assert_eq!(
            value(&result, "sheets"),
            &Value::List(vec![s("Transactions")])
        );
        assert!(list(value(&result, "warnings"))
            .iter()
            .any(|w| py_str(w).contains("Draft list:")));
        assert!(get(&result, "report_snapshot").is_none());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}
