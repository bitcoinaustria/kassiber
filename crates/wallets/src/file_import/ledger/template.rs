//! `write_generic_ledger_template` (`kassiber/importers.py`): the blank
//! fill-in ledger, as `.xlsx` (a Transactions sheet with a Type dropdown and
//! a Legend sheet) or `.csv` (the header and the example rows). The example
//! rows are Bitcoin only and change nothing until the user imports the file.
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value as Py;
use kassiber_xlsx::{Cell, Document, Operation, Sheet, Style};

/// `GENERIC_LEDGER_COLUMNS`.
pub const COLUMNS: [&str; 16] = [
    "Type",
    "Date",
    "Received Amount",
    "Received Asset",
    "Sent Amount",
    "Sent Asset",
    "Fee Amount",
    "Fee Asset",
    "Fiat Value",
    "Counterparty",
    "Note",
    "Tx-ID",
    "Payment Hash",
    "Payment Hash Source",
    "Swap Refund Funding Tx-ID",
    "Swap Refund Funding Vout",
];

/// `GENERIC_LEDGER_TYPE_GROUPS`.
const TYPE_GROUPS: [(&str, &[&str]); 4] = [
    ("Acquire", &["Buy", "Deposit", "Gift received"]),
    ("Dispose", &["Sell", "Withdrawal", "Spend"]),
    (
        "Earn",
        &["Income", "Mining", "Staking", "Interest", "Airdrop", "Fork"],
    ),
    (
        "Outflow (review)",
        &["Gift sent", "Donation", "Lost", "Stolen"],
    ),
];

/// `_GENERIC_LEDGER_EXAMPLE_ROWS`: twelve cells each, fewer than the header.
#[rustfmt::skip]
const EXAMPLE_ROWS: [[&str; 12]; 7] = [
    ["Buy", "2026-01-15", "0.05000000", "BTC", "3000.00", "EUR", "3.50", "EUR", "", "Coinfinity", "First stack", ""],
    ["Sell", "2026-02-10", "2200.00", "EUR", "0.03000000", "BTC", "1.00", "EUR", "", "Kraken", "Took some profit", ""],
    ["Mining", "2026-03-10", "0.00050000", "BTC", "", "", "", "", "32.50", "Solo pool", "Block reward", ""],
    ["Income", "2026-03-20", "250000", "SATS", "", "", "", "", "160.00", "Freelance client", "Invoice paid in sats", ""],
    ["Withdrawal", "2026-04-01", "", "", "0.02000000", "BTC", "0.00002000", "BTC", "", "", "Moved to cold storage", ""],
    ["Spend", "2026-04-15", "", "", "0.00100000", "BTC", "", "", "65.00", "Local merchant", "Coffee and groceries", ""],
    ["Gift sent", "2026-05-01", "", "", "0.00100000", "BTC", "", "", "", "", "Birthday gift to a friend", ""],
];

/// `_GENERIC_LEDGER_LEGEND`.
const LEGEND: [(&str, &str); 24] = [
    ("How to fill in this ledger", ""),
    ("", ""),
    (
        "One row per transaction. Each row has exactly one Bitcoin leg.",
        "",
    ),
    (
        "Type",
        "What happened. Pick from the dropdown / list below.",
    ),
    (
        "Date",
        "Required. 2026-01-15, a full timestamp, or 15.01.2026.",
    ),
    (
        "Received Amount / Asset",
        "What came in. For a Buy: the Bitcoin you bought.",
    ),
    (
        "Sent Amount / Asset",
        "What went out. For a Buy: the fiat you paid.",
    ),
    (
        "Fee Amount / Asset",
        "Optional. Blank Fee Asset means the fee is in Bitcoin.",
    ),
    ("", "A fiat fee must match the row's fiat currency."),
    (
        "Fiat Value",
        "Fair-market value in your book currency. Use it for",
    ),
    ("", "Income/Mining/Spend/Gift rows that have no cash leg."),
    ("Counterparty", "Optional. Exchange, merchant, or person."),
    ("Note", "Optional free text. Stored on the transaction."),
    (
        "Tx-ID",
        "Optional but recommended — lets you re-import safely.",
    ),
    ("", ""),
    (
        "Amounts are in BTC (e.g. 0.05000000) unless the Asset is SATS.",
        "",
    ),
    ("A comma or dot decimal both work (0,05 = 0.05).", ""),
    (
        "Fiat columns must be in your book's currency (a EUR book takes",
        "",
    ),
    (
        "EUR; a JPY book takes JPY). Mixed currencies are rejected.",
        "",
    ),
    (
        "Gift sent / Donation / Lost / Stolen are flagged for review,",
        "",
    ),
    ("not auto-sold at market value.", ""),
    (
        "To move Bitcoin between your own wallets, import a Withdrawal",
        "",
    ),
    (
        "into the source wallet and a Deposit into the destination",
        "",
    ),
    ("wallet with the same Tx-ID.", ""),
];

/// `GENERIC_LEDGER_DISPLAY_TYPES`: the dropdown, in group order.
pub fn display_types() -> Vec<&'static str> {
    TYPE_GROUPS
        .iter()
        .flat_map(|(_, labels)| labels.iter().copied())
        .collect()
}

/// `_generic_ledger_template_format(output_path)`.
pub fn format_for(path: &str) -> &'static str {
    if path.to_lowercase().ends_with(".csv") {
        "csv"
    } else {
        "xlsx"
    }
}

/// The CSV template: Python's `csv.writer` (excel dialect, CRLF).
pub fn csv_bytes() -> Vec<u8> {
    let mut out = String::new();
    let mut row = |cells: &[&str]| {
        for (index, cell) in cells.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            if cell.contains([',', '"', '\r', '\n']) || (cells.len() == 1 && cell.is_empty()) {
                out.push('"');
                out.push_str(&cell.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(cell);
            }
        }
        out.push_str("\r\n");
    };
    row(&COLUMNS);
    for example in &EXAMPLE_ROWS {
        row(example);
    }
    out.into_bytes()
}

/// The workbook as `_write_generic_ledger_xlsx_template` lays it out.
/// `created` is the export clock (XlsxWriter stamps the write time).
pub fn workbook(created: &str) -> Document {
    const PLAIN: usize = 0;
    const HEADER: usize = 1;
    const EXAMPLE: usize = 2;
    const TITLE: usize = 3;
    const TERM: usize = 4;
    let formats = vec![
        Style::default(),
        Style {
            bold: true,
            bg_color: Some("#F2A900".into()),
            font_color: Some("#1A1A1A".into()),
            border: 1,
            ..Style::default()
        },
        Style {
            italic: true,
            font_color: Some("#666666".into()),
            ..Style::default()
        },
        Style {
            bold: true,
            font_size: Some(13.0),
            ..Style::default()
        },
        Style {
            bold: true,
            ..Style::default()
        },
    ];
    // XlsxWriter's `write`: an empty string is a blank cell, kept only when
    // it carries a format.
    let cell = |operations: &mut Vec<Operation>, row: u32, col: u16, text: &str, style| {
        if !text.is_empty() {
            operations.push(Operation::Cell {
                row,
                col,
                value: Cell::String(text.into()),
                style,
            });
        } else if style != PLAIN {
            operations.push(Operation::Cell {
                row,
                col,
                value: Cell::Blank,
                style,
            });
        }
    };
    let column = |first, last, width| Operation::Column { first, last, width };

    let mut transactions = vec![Operation::Freeze { row: 1, col: 0 }];
    for (col, name) in COLUMNS.iter().enumerate() {
        cell(&mut transactions, 0, col as u16, name, HEADER);
    }
    for (index, example) in EXAMPLE_ROWS.iter().enumerate() {
        for (col, value) in example.iter().enumerate() {
            cell(
                &mut transactions,
                index as u32 + 1,
                col as u16,
                value,
                EXAMPLE,
            );
        }
    }
    transactions.extend([
        column(0, 0, 14.0),
        column(1, 1, 12.0),
        column(2, 7, 14.0),
        column(8, 8, 12.0),
        column(9, 11, 22.0),
    ]);
    let last_row = 200.max(EXAMPLE_ROWS.len() as u32 + 50);
    transactions.push(Operation::ListValidation {
        first_row: 1,
        first_col: 0,
        last_row,
        last_col: 0,
        values: display_types().into_iter().map(str::to_owned).collect(),
        error_message: "Pick a Type from the list (see the Legend sheet).".into(),
    });

    let mut legend = vec![column(0, 0, 32.0), column(1, 1, 60.0)];
    cell(&mut legend, 0, 0, "Generic ledger import", TITLE);
    for (index, (term, definition)) in LEGEND.iter().enumerate() {
        let row = index as u32 + 2;
        let style = if definition.is_empty() { PLAIN } else { TERM };
        cell(&mut legend, row, 0, term, style);
        cell(&mut legend, row, 1, definition, PLAIN);
    }
    let type_start = LEGEND.len() as u32 + 4;
    cell(&mut legend, type_start, 0, "Transaction Types", TITLE);
    for (offset, (group, labels)) in TYPE_GROUPS.iter().enumerate() {
        let row = type_start + 1 + offset as u32;
        cell(&mut legend, row, 0, group, TERM);
        cell(&mut legend, row, 1, &labels.join(", "), PLAIN);
    }
    Document {
        created: created.into(),
        formats,
        sheets: vec![
            Sheet {
                name: "Transactions".into(),
                operations: transactions,
            },
            Sheet {
                name: "Legend".into(),
                operations: legend,
            },
        ],
        ..Document::default()
    }
}

/// `write_generic_ledger_template(output_path, fmt)`: write the template and
/// describe it. `created` stamps the workbook's document properties.
pub fn write(output: &std::path::Path, format: &str, created: &str) -> Result<Py, AppError> {
    let resolved = format.to_lowercase();
    let (bytes, resolved) = match resolved.as_str() {
        "csv" => (csv_bytes(), "csv"),
        "xlsx" | "xlsm" => (
            kassiber_xlsx::render(&workbook(created))
                .map_err(|error| AppError::new("internal_error", error.to_string()))?,
            "xlsx",
        ),
        _ => {
            return Err(AppError::new(
                "validation",
                format!("Unsupported template format '{resolved}'"),
            )
            .with_hint("Use xlsx or csv."))
        }
    };
    std::fs::write(output, bytes).map_err(|error| {
        AppError::new("internal_error", format!("{error}: '{}'", output.display()))
    })?;
    let file = kassiber_paths::abspath(output)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    Ok(dict([
        ("file", s(file.to_string_lossy().into_owned())),
        ("format", s(resolved)),
        (
            "columns",
            Py::List(COLUMNS.iter().map(|name| s(*name)).collect()),
        ),
        (
            "types",
            Py::List(display_types().into_iter().map(s).collect()),
        ),
    ]))
}

/// `_managed_report_export_path(data_root, "kassiber-ledger-template",
/// suffix)`: `exports/reports/<stem>-<UTC stamp>[-n]<suffix>`, never over an
/// existing file. `now` is an ISO 8601 UTC instant.
pub fn managed_path(
    exports_root: &std::path::Path,
    suffix: &str,
    now: &str,
) -> Result<std::path::PathBuf, AppError> {
    let root = exports_root.join("reports");
    std::fs::create_dir_all(&root).map_err(|error| {
        AppError::new("internal_error", format!("{error}: '{}'", root.display()))
    })?;
    let stamp = now
        .get(..19)
        .unwrap_or(now)
        .replace(['-', ':'], "")
        .replace('T', "-");
    let base = format!("kassiber-ledger-template-{stamp}");
    let mut candidate = root.join(format!("{base}{suffix}"));
    let mut counter = 2;
    while candidate.exists() {
        candidate = root.join(format!("{base}-{counter}{suffix}"));
        counter += 1;
    }
    Ok(candidate)
}

/// `_ledger_template_payload(ctx, args)` after the argument checks: the
/// requested `format` (already stripped; `None` for the default) written
/// under the exports root, with its `filename`.
pub fn managed(
    exports_root: &std::path::Path,
    requested: Option<&str>,
    now: &str,
) -> Result<Py, AppError> {
    let requested = kassiber_pytext::lower(kassiber_pytext::strip(requested.unwrap_or("xlsx")));
    let (suffix, format) = match requested.as_str() {
        "xlsx" | "xlsm" => (".xlsx", "xlsx"),
        "csv" => (".csv", "csv"),
        _ => {
            return Err(AppError::new(
                "validation",
                format!("Unsupported template format '{requested}'"),
            )
            .with_hint("Use xlsx or csv."))
        }
    };
    let path = managed_path(exports_root, suffix, now)?;
    let mut payload = write(&path, format, now)?;
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    kassiber_envelope::set(&mut payload, "filename", s(name));
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dropdown_is_the_importer_display_vocabulary() {
        assert_eq!(display_types(), super::super::strings("display"));
    }

    #[test]
    fn the_csv_template_is_the_header_and_the_examples() {
        let text = String::from_utf8(csv_bytes()).unwrap();
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines.len(), 9);
        assert_eq!(lines[0], COLUMNS.join(","));
        assert_eq!(
            lines[1],
            "Buy,2026-01-15,0.05000000,BTC,3000.00,EUR,3.50,EUR,,Coinfinity,First stack,"
        );
        assert_eq!(lines[8], "");
    }

    #[test]
    fn unknown_formats_are_refused_before_writing() {
        let directory =
            std::env::temp_dir().join(format!("kassiber-ledger-template-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("t.ods");
        let error = write(&path, "ODS", "2026-01-02T03:04:05Z").unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("validation", "Unsupported template format 'ods'")
        );
        assert!(!path.exists());
        assert_eq!(format_for("A.CSV"), "csv");
        assert_eq!(format_for("a.csv.xlsx"), "xlsx");
        let written = write(&path, "xlsm", "2026-01-02T03:04:05Z").unwrap();
        assert_eq!(kassiber_envelope::get(&written, "format"), Some(&s("xlsx")));
        assert!(std::fs::read(&path).unwrap().starts_with(b"PK"));
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
