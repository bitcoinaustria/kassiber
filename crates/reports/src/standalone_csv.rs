//! Observed transaction lists can be drafts; they never establish tax facts.
use super::*;
use artifact_csv::{File, Prepared, Section};

const STALE: &str = "Reports require fresh journals. Run `kassiber journals process` first.";

/// The transaction CSV's columns in the shared design's machine format:
/// amounts carry their unit and are exact decimal text; `status` says on
/// every row whether the file is a draft list (`draft`) or not (`final`).
const COLUMNS: [(&str, &str); 24] = [
    ("occurred_at", "occurred_at"),
    ("wallet", "wallet"),
    ("transaction_id", "transaction_id"),
    ("direction", "direction"),
    ("asset", "asset"),
    ("amount_btc", "amount_exact"),
    ("amount_msat", "amount_msat"),
    ("fee_btc", "fee_exact"),
    ("fee_msat", "fee_msat"),
    ("fiat_currency", "fiat_currency"),
    ("fiat_price", "fiat_price_exact"),
    ("fiat_value", "fiat_value_exact"),
    ("fmv", "fmv_exact"),
    ("fiat_fee", "fiat_fee_exact"),
    ("cost_basis", "cost_basis_exact"),
    ("gain_loss", "gain_loss_exact"),
    ("description", "description"),
    ("note", "note"),
    ("counterparty", "counterparty"),
    ("transfer", "transfer"),
    ("tags", "tags"),
    ("references", "references"),
    ("attachments", "attachments"),
    ("status", "status"),
];

pub(super) fn prepare(conn: &Connection, profile: &Value, args: &Value) -> Result<Prepared> {
    let (section, metadata) = project(conn, profile, args)?;
    let count = section.rows.len();
    let header: Vec<&str> = COLUMNS.iter().map(|(name, _)| *name).collect();
    let lines = section.rows.iter().map(|row| {
        COLUMNS
            .iter()
            .map(|(name, key)| {
                // A draft list withholds basis and gain: its float is null.
                let withheld = matches!(*name, "cost_basis" | "gain_loss")
                    && matches!(value(row, name), Value::Null);
                if withheld {
                    String::new()
                } else {
                    artifact_csv::cell(value(row, key))
                }
            })
            .collect()
    });
    Ok(Prepared {
        files: vec![File {
            name: "report.csv".into(),
            sheet: "".into(),
            bytes: report_design::csv_table(&header, lines),
            rows: count,
        }],
        metadata,
        completion: None,
    })
}

pub(super) fn project(
    conn: &Connection,
    profile: &Value,
    args: &Value,
) -> Result<(Section, Value)> {
    let id = text(profile, "id");
    let mut reasons = Vec::new();
    if let Err(error) = require_context(conn, profile, false) {
        match error.code.as_str() {
            "custody_component_incomplete" => {
                reasons.push(error.message);
                let issues = quantity_issues(conn, &id)?;
                if !issues.is_empty() {
                    reasons.push("Reports are blocked by unresolved custody quantity.".into());
                }
            }
            "custody_quantity_unresolved" => reasons.push(error.message),
            "app_error" if error.message == STALE => (),
            _ => return Err(error),
        }
    }
    let selected = match value(args, "wallets") {
        Value::Null => {
            if truthy(value(args, "wallet")) {
                Some(vec![value(args, "wallet").clone()])
            } else {
                None
            }
        }
        Value::List(refs) => Some(refs.clone()),
        _ => return Err(AppError::new("validation", "wallets must be a list")),
    };
    let mut wallets = Vec::new();
    if let Some(refs) = selected {
        if refs.is_empty() {
            return Err(AppError::new(
                "validation",
                "Select at least one wallet to export",
            ));
        }
        for reference in refs {
            let wallet = kassiber_wallets::resolve(conn, &id, &py_str(&reference))?;
            if !wallets
                .iter()
                .any(|w: &kassiber_wallets::Wallet| w.id == wallet.id)
            {
                wallets.push(wallet);
            }
        }
    }
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [&id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let current = semantics_current(profile)
        && truthy(value(profile, "last_processed_at"))
        && int(profile, "last_processed_tx_count") == active
        && int(profile, "journal_input_version") == int(profile, "last_processed_input_version");
    let wallet = if wallets.len() == 1 {
        Some(wallets[0].id.as_str())
    } else {
        None
    };
    let mut transactions = transaction_csv::report_rows_current(conn, profile, wallet, current)?;
    if wallets.len() > 1 {
        transactions.retain(|r| wallets.iter().any(|w| text(r, "wallet") == w.label));
    }
    // This service accepts an already resolved ExportRange payload. Calendar
    // selection belongs to the shared transport admission, before preparation.
    let range = value(args, "export_range").clone();
    let mut undated = 0;
    if !matches!(range, Value::Null) {
        let parse = |raw: &str| {
            kassiber_rates::time::parse(raw).ok_or_else(|| {
                AppError::new(
                    "internal_error",
                    format!(
                        "Invalid isoformat string: {}",
                        kassiber_pytext::repr_str(raw)
                    ),
                )
            })
        };
        let start = parse(&text(&range, "start"))?;
        let end = parse(&text(&range, "end"))?;
        let mut included = Vec::new();
        for row in transactions {
            let raw = text(&row, "occurred_at");
            let contains = if raw.is_empty() {
                false
            } else {
                let at = parse(&raw)?;
                start <= at && at < end
            };
            if contains {
                included.push(row);
            } else if raw.is_empty() || raw == "1970-01-01T00:00:00Z" {
                undated += 1;
            }
        }
        transactions = included;
    }
    if !current {
        reasons.push(STALE.into());
    }
    let withheld = if current {
        "cost basis and gain/loss"
    } else {
        "cost basis, gain/loss and transfer links"
    };
    let status = if reasons.is_empty() { "final" } else { "draft" };
    for row in &mut transactions {
        replace(row, "status", s(status));
    }
    let mut warnings = Vec::new();
    if !reasons.is_empty() {
        for row in &mut transactions {
            replace(row, "cost_basis", Value::Null);
            replace(row, "gain_loss", Value::Null);
            if !current {
                replace(row, "transfer", s(""));
            }
        }
        warnings.push(s(format!("Draft list: {} Every observed transaction is listed; {withheld} stay empty until that is resolved.", reasons.join(" "))));
    }
    if undated > 0 {
        let (rows, verb) = if undated == 1 {
            ("1 transaction".into(), "is")
        } else {
            (format!("{undated} transactions"), "are")
        };
        warnings.push(s(format!("{rows} without a known date could belong to this period but {verb} not in this file. Export All time to include undated transactions.")));
    }
    let scope = if wallets.len() > 3 {
        format!("{} wallets", wallets.len())
    } else {
        wallets
            .iter()
            .map(|w| w.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let title = format!(
        "Kassiber Transactions - {}",
        if scope.is_empty() {
            text(profile, "label")
        } else {
            scope.clone()
        }
    );
    let count = transactions.len();
    let section = Section {
        name: "Transactions",
        title: if reasons.is_empty() {
            "Transactions".into()
        } else {
            format!("Transactions (draft: {withheld} withheld)")
        },
        headers: transaction_csv::HEADERS,
        rows: transactions,
    };
    Ok((
        section,
        dict([
            ("title", s(title)),
            ("wallet", s(scope)),
            ("rows", Value::Int(count.into())),
            ("warnings", Value::List(warnings)),
            ("range", range),
        ]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture book with whole-satoshi, sub-satoshi and zero amounts.
    fn export(stale: bool) -> (Vec<String>, Vec<Vec<String>>, Value) {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(
            "UPDATE transactions SET amount=5000000000, fee=0 WHERE id='in';
             UPDATE transactions SET amount=123456789, fee=1000 WHERE id='out';
             UPDATE transactions SET amount=0, fee=1 WHERE id='earn';",
        )
        .unwrap();
        if stale {
            conn.execute_batch("UPDATE profiles SET journal_input_version=8 WHERE id='p'")
                .unwrap();
        }
        let (_, profile) = kassiber_setup::references::scope(conn, &dict::<&str>([])).unwrap();
        let prepared = prepare(conn, &profile, &dict::<&str>([])).unwrap();
        let text = String::from_utf8(prepared.files[0].bytes.clone()).unwrap();
        let mut lines = records(&text).into_iter();
        let header = lines.next().unwrap();
        (header, lines.collect(), prepared.metadata)
    }

    /// RFC 4180 records; quoted cells may hold commas and line breaks.
    fn records(text: &str) -> Vec<Vec<String>> {
        let (mut records, mut record, mut cell) = (Vec::new(), Vec::new(), String::new());
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('"', true) if chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                ('"', _) => quoted = !quoted,
                (',', false) => record.push(std::mem::take(&mut cell)),
                ('\n', false) => {
                    record.push(std::mem::take(&mut cell));
                    records.push(std::mem::take(&mut record));
                }
                _ => cell.push(c),
            }
        }
        records
    }

    fn column<'a>(header: &[String], row: &'a [String], name: &str) -> &'a str {
        &row[header.iter().position(|h| h == name).unwrap()]
    }

    /// A fixture transaction's row, found by its date: the CSV carries the
    /// external transaction id, which the fixture leaves empty.
    fn row<'a>(header: &[String], rows: &'a [Vec<String>], id: &str) -> &'a [String] {
        let at = match id {
            "in" => "2024-01-01T00:00:00Z",
            "out" => "2024-12-31T23:30:00Z",
            "earn" => "2025-01-02T00:00:00Z",
            _ => unreachable!("{id}"),
        };
        rows.iter()
            .find(|r| column(header, r, "occurred_at") == at)
            .unwrap_or_else(|| panic!("no {id} in {rows:?}"))
    }

    #[test]
    fn btc_columns_have_eight_places_and_sub_satoshi_digits_only_when_present() {
        let (header, rows, _) = export(false);
        for (id, amount, fee) in [
            ("in", "0.05000000", "0.00000000"),
            ("out", "0.00123456789", "0.00000001"),
            ("earn", "0.00000000", "0.00000000001"),
        ] {
            let row = row(&header, &rows, id);
            assert_eq!(column(&header, row, "amount_btc"), amount, "{id}");
            assert_eq!(column(&header, row, "fee_btc"), fee, "{id}");
        }
        let row = row(&header, &rows, "out");
        assert_eq!(column(&header, row, "amount_msat"), "123456789");
        assert_eq!(column(&header, row, "fee_msat"), "1000");
    }

    #[test]
    fn the_status_column_says_whether_the_file_is_a_draft() {
        for (stale, status) in [(false, "final"), (true, "draft")] {
            let (header, rows, metadata) = export(stale);
            assert_eq!(header.last().map(String::as_str), Some("status"));
            assert_eq!(header.len(), COLUMNS.len());
            assert!(!rows.is_empty());
            for row in &rows {
                assert_eq!(row.last().map(String::as_str), Some(status), "{row:?}");
            }
            let warned = artifact_csv::list(value(&metadata, "warnings"))
                .iter()
                .any(|w| py_str(w).starts_with("Draft list:"));
            assert_eq!(warned, stale);
            // A draft withholds basis and gain; a final list carries them.
            let out = row(&header, &rows, "out");
            assert_eq!(column(&header, out, "cost_basis").is_empty(), stale);
        }
    }
}
