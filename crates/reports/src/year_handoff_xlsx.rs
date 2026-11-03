use super::*;
use artifact_csv::{list, File, Prepared};
use kassiber_xlsx::{Cell, Document, Operation, Sheet};

pub(super) fn metadata() -> &'static Value {
    static META: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    META.get_or_init(|| kassiber_pytext::loads(include_str!("year_handoff_metadata.json")).unwrap())
}
fn scalar(value: &Value) -> String {
    match value {
        Value::Decimal(d) => d.to_string(),
        _ => py_str(value),
    }
}
fn cell(value: &Value) -> Result<Cell> {
    match value {
        Value::Decimal(d) => artifact_xlsx::cell(&float(d)?),
        _ => artifact_xlsx::cell(value),
    }
}
fn address(row: usize, col: usize) -> String {
    let mut col = col + 1;
    let mut name = String::new();
    while col > 0 {
        name.insert(0, (b'A' + ((col - 1) % 26) as u8) as char);
        col = (col - 1) / 26;
    }
    format!("{name}{}", row + 1)
}
fn sheet(spec: &Value, title: String, rows: &[Value], fiat: &str) -> Result<Sheet> {
    let columns = list(value(spec, "columns"));
    let labels: Vec<_> = columns
        .iter()
        .map(|c| text(c, "label").replace("{fiat}", fiat))
        .collect();
    let widths: Vec<_> = columns
        .iter()
        .zip(&labels)
        .map(|(c, label)| {
            let sample = rows
                .iter()
                .take(50)
                .map(|r| {
                    let v = value(r, &text(c, "key"));
                    if truthy(v) {
                        scalar(v).chars().count().min(36)
                    } else {
                        0
                    }
                })
                .max()
                .unwrap_or(0);
            (label.chars().count().max(sample) + 2).clamp(10, 38) as f64
        })
        .collect();
    let last = (columns.len() - 1) as u16;
    let mut operations = vec![
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
        Operation::Row {
            row: 0,
            height: 28.0,
        },
        Operation::Merge {
            first_row: 0,
            first_col: 0,
            last_row: 0,
            last_col: last,
            text: title,
            style: 0,
        },
    ];
    let header_lines = labels
        .iter()
        .enumerate()
        .map(|(i, l)| artifact_xlsx::lines(&s(l.as_str()), widths[i]))
        .max()
        .unwrap_or(1);
    operations.push(Operation::Row {
        row: 1,
        height: (header_lines * 15 + 4).max(24) as f64,
    });
    for (i, label) in labels.iter().enumerate() {
        operations.push(Operation::Column {
            first: i as u16,
            last: i as u16,
            width: widths[i],
        });
        operations.push(Operation::Cell {
            row: 1,
            col: i as u16,
            value: Cell::String(label.clone()),
            style: 1,
        });
    }
    operations.push(Operation::Freeze { row: 2, col: 0 });
    for (i, row) in rows.iter().enumerate() {
        let mut lines = 1;
        for (col, column) in columns.iter().enumerate() {
            let key = text(column, "key");
            let raw = value(row, &key);
            let format = text(column, "format");
            let numeric_style = match format.as_str() {
                "int" => 3,
                "quantity" => 7,
                "sats" => 8,
                "money" => 5,
                _ => 2,
            };
            let mut style = if matches!(raw, Value::Null) || raw == &s("") {
                2
            } else {
                numeric_style
            };
            let cell = if key == "attachments" && get(row, "attachments_list").is_some() {
                let entries = list(value(row, "attachments_list"));
                lines = lines.max(entries.len().max(1));
                if entries.len() == 1 && truthy(value(&entries[0], "url")) {
                    style = 6;
                    Cell::Link {
                        url: text(&entries[0], "url"),
                        text: text(&entries[0], "display_name"),
                    }
                } else {
                    style = 2;
                    if entries.is_empty() {
                        Cell::Blank
                    } else {
                        Cell::String(
                            entries
                                .iter()
                                .map(|e| text(e, "display_name"))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    }
                }
            } else if truthy(value(column, "formula")) {
                let mut expression =
                    text(column, "formula").replace("{decimals}", &py_str(value(row, "_decimals")));
                for (j, other) in columns.iter().enumerate() {
                    expression = expression
                        .replace(&format!("{{{}}}", text(other, "key")), &address(i + 2, j));
                }
                style = numeric_style;
                let result = if let Value::Decimal(d) = raw {
                    py_str(&float(d)?)
                } else {
                    scalar(raw)
                };
                Cell::Formula { expression, result }
            } else {
                if format == "text" {
                    lines = lines.max(artifact_xlsx::lines(raw, widths[col]));
                }
                cell(raw)?
            };
            operations.push(Operation::Cell {
                row: (i + 2) as u32,
                col: col as u16,
                value: cell,
                style,
            });
        }
        operations.push(Operation::Row {
            row: (i + 2) as u32,
            height: (lines * 15 + 4).max(16) as f64,
        });
    }
    if rows.is_empty() {
        operations.push(Operation::Cell {
            row: 2,
            col: 0,
            value: Cell::String("No rows in scope.".into()),
            style: 2,
        });
    }
    operations.push(Operation::Autofilter {
        first_row: 1,
        first_col: 0,
        last_row: (rows.len() + 1).max(2) as u32,
        last_col: last,
    });
    Ok(Sheet {
        name: text(spec, "name"),
        operations,
    })
}
pub(super) fn overview(report: &Value) -> Vec<Value> {
    let profile = value(report, "profile");
    let mut rows = Vec::new();
    let mut add = |group: &str, field: &str, v: Value| {
        rows.push(dict([
            ("group", s(group)),
            ("field", s(field)),
            ("value", v),
        ]))
    };
    add(
        "Scope",
        "Workspace",
        value(value(report, "workspace"), "label").clone(),
    );
    add("Scope", "Book", value(profile, "label").clone());
    add("Scope", "Year", value(report, "year").clone());
    add("Scope", "Calendar", value(report, "calendar").clone());
    add(
        "Scope",
        "Period start (UTC)",
        value(report, "period_start").clone(),
    );
    add(
        "Scope",
        "Period end (UTC, exclusive)",
        value(report, "period_end").clone(),
    );
    for (field, key) in [
        ("Book currency", "fiat_currency"),
        ("Tax country", "tax_country"),
        ("Lot method", "gains_algorithm"),
    ] {
        add("Scope", field, value(profile, key).clone());
    }
    add(
        "Scope",
        "Wallets in book",
        Value::Int(list(value(report, "wallets")).len().into()),
    );
    let mut assets = std::collections::BTreeMap::new();
    for sheet in ["transactions", "balances", "journal"] {
        for row in list(value(report, sheet)) {
            assets
                .entry(text(row, "asset_id"))
                .or_insert_with(|| text(row, "asset"));
        }
    }
    for (id, label) in assets {
        add("Scope", "Asset", s(format!("{label} ({id})")));
    }
    add("Run", "Generated at", value(report, "generated_at").clone());
    add(
        "Run",
        "Journals processed at",
        value(profile, "last_processed_at").clone(),
    );
    add(
        "Run",
        "Kassiber version",
        value(report, "kassiber_version").clone(),
    );
    add(
        "Status",
        "Reconciliation",
        s(if truthy(value(report, "reconciled")) {
            "OK"
        } else {
            "DIFFERENCES"
        }),
    );
    add(
        "Status",
        "Quarantined transactions in or before the year",
        Value::Int(0.into()),
    );
    for warning in list(value(report, "warnings")) {
        add("Status", "Warning", warning.clone());
    }
    for (field, key) in [
        ("Transactions in the year", "transactions"),
        ("Excluded transactions in the year", "excluded"),
        ("Journal entries in the year", "journal"),
    ] {
        add(
            "Counts",
            field,
            Value::Int(list(value(report, key)).len().into()),
        );
    }
    let mut counts = std::collections::BTreeMap::<_, usize>::new();
    for row in list(value(report, "transactions")) {
        *counts.entry(text(row, "classification")).or_default() += 1;
    }
    for (classification, count) in counts {
        add("Classification", &classification, Value::Int(count.into()));
    }
    for note in list(value(metadata(), "notes")) {
        add("Notes", "", note.clone());
    }
    rows
}
pub(super) fn project(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(Prepared, Document)> {
    let report = year_handoff::build(conn, args, sources)?;
    let year = int(&report, "year");
    let profile = value(&report, "profile");
    let fiat = if truthy(value(profile, "fiat_currency")) {
        text(profile, "fiat_currency")
    } else {
        String::new()
    };
    let titles = [
        "Kassiber year handoff",
        "Wallets in this book",
        "Transactions",
        "Journal entries",
        "Opening and closing balances",
        "Per-asset roll-forward",
        "Journal vs observed wallet movements",
        "Excluded transactions",
    ];
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
    let overview = overview(&report);
    let sheets = list(value(metadata(), "sheets"))
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            sheet(
                spec,
                format!("{} {year}", titles[i]),
                if i == 0 {
                    &overview
                } else {
                    list(value(&report, keys[i]))
                },
                &fiat,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut formats = artifact_xlsx::styles();
    for number in ["#,##0.00000000###", "#,##0.###"] {
        formats.push(kassiber_xlsx::Style {
            font_size: Some(11.0),
            valign: Some("top".into()),
            num_format: Some(number.into()),
            ..Default::default()
        });
    }
    let document = Document {
        title: format!("Kassiber year handoff {year} - {}", text(profile, "label")),
        subject: "Kassiber year handoff for EA / Jahresabschluss".into(),
        author: "Kassiber".into(),
        comments: "Generated from the processed journal; checks are live formulas.".into(),
        created: sources.now(conn)?,
        formats,
        sheets,
        names: vec![],
    };
    let metadata = dict([
        ("year", Value::Int(year.into())),
        ("calendar", value(&report, "calendar").clone()),
        ("period_start", value(&report, "period_start").clone()),
        ("period_end", value(&report, "period_end").clone()),
        (
            "sheets",
            Value::List(
                document
                    .sheets
                    .iter()
                    .map(|sheet| s(sheet.name.as_str()))
                    .collect(),
            ),
        ),
        (
            "rows",
            dict(
                [
                    "wallets",
                    "transactions",
                    "journal",
                    "balances",
                    "reconciliation",
                    "excluded",
                ]
                .map(|k| (k, Value::Int(list(value(&report, k)).len().into()))),
            ),
        ),
        ("reconciled", value(&report, "reconciled").clone()),
        ("warnings", value(&report, "warnings").clone()),
    ]);
    Ok((
        Prepared {
            files: vec![],
            metadata,
            completion: Some(export_completion::Completion {
                kind: "year-handoff.xlsx".into(),
                year: Some(Value::Int(year.into())),
                scope: Value::Null,
            }),
        },
        document,
    ))
}
pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (mut prepared, document) = project(conn, args, sources)?;
    let bytes = kassiber_xlsx::render(&document)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    prepared.files = vec![File {
        name: "report.xlsx".into(),
        sheet: String::new(),
        bytes,
        rows: 0,
    }];
    Ok(prepared)
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
            Ok("00000000-0000-4000-8000-000000000001".into())
        }
    }
    fn operations(doc: &Document) -> Vec<(String, Vec<String>)> {
        doc.sheets
            .iter()
            .map(|sheet| {
                let mut operations: Vec<String> = sheet
                    .operations
                    .iter()
                    .map(|op| {
                        let mut json = serde_json::to_value(op).unwrap();
                        if let Some(index) = json.get("style").and_then(|n| n.as_u64()) {
                            json["style"] =
                                serde_json::to_value(&doc.formats[index as usize]).unwrap();
                        }
                        serde_json::to_string(&json).unwrap()
                    })
                    .collect();
                operations.sort();
                (sheet.name.clone(), operations)
            })
            .collect()
    }
    fn normalize_numbers(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Number(number)
                if number.as_u64().is_none() && number.as_i64().is_none() =>
            {
                *number = serde_json::Number::from_f64(number.as_f64().unwrap()).unwrap();
            }
            serde_json::Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
            serde_json::Value::Object(fields) => fields.values_mut().for_each(normalize_numbers),
            _ => (),
        }
    }
    #[test]
    fn year_handoff_workbook_pins_observations_journals_rates_and_barriers() {
        let mut cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/year_handoff.json")).unwrap();
        normalize_numbers(&mut cases);
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        for case in cases.as_array().unwrap() {
            conn.execute_batch("SAVEPOINT xlsx_vector").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let result = project(conn, &args, &Fixed);
            if case["expected"].get("error").is_some() {
                let error = result.unwrap_err();
                let actual = dict([
                    ("code", s(error.code)),
                    ("message", s(error.message)),
                    ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                    ("details", error.details.unwrap_or(Value::Null)),
                    ("retryable", Value::Bool(error.retryable)),
                ]);
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(
                        &kassiber_pyjson::dumps(&actual, &Default::default()).unwrap()
                    )
                    .unwrap(),
                    case["expected"]["error"],
                    "{} refusal",
                    case["name"]
                );
            } else {
                let (prepared, actual) =
                    result.unwrap_or_else(|e| panic!("{}: {} {}", case["name"], e.code, e.message));
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(
                        &kassiber_pyjson::dumps(&prepared.metadata, &Default::default()).unwrap()
                    )
                    .unwrap(),
                    case["expected"]["metadata"],
                    "{} metadata",
                    case["name"]
                );
                let expected: Document =
                    serde_json::from_value(case["expected"]["document"].clone()).unwrap();
                assert_eq!(actual.title, expected.title, "{} title", case["name"]);
                assert_eq!(actual.subject, expected.subject, "{} subject", case["name"]);
                assert_eq!(actual.author, expected.author, "{} author", case["name"]);
                assert_eq!(
                    actual.comments, expected.comments,
                    "{} comments",
                    case["name"]
                );
                assert_eq!(actual.created, expected.created, "{} created", case["name"]);
                assert_eq!(
                    serde_json::to_string(&actual.names).unwrap(),
                    serde_json::to_string(&expected.names).unwrap()
                );
                let actual = operations(&actual);
                let expected = operations(&expected);
                assert_eq!(actual.len(), expected.len(), "{}", case["name"]);
                for ((name, actual), (wanted, expected)) in actual.iter().zip(&expected) {
                    assert_eq!(name, wanted, "{}", case["name"]);
                    assert_eq!(actual.len(), expected.len(), "{} {name}", case["name"]);
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert_eq!(actual, expected, "{} {name}", case["name"]);
                    }
                }
            }
            conn.execute_batch("ROLLBACK TO xlsx_vector; RELEASE xlsx_vector")
                .unwrap();
        }
    }
    #[test]
    fn year_handoff_managed_publication_saves_actual_hash_and_retains_freshness() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        let args = dict([("year", Value::Int(2024.into()))]);
        let reports = root.path().join("reports");
        std::fs::create_dir(&reports).unwrap();
        let reserved = reports.join("kassiber-year-handoff-2024-20260102-030405.xlsx");
        std::fs::write(&reserved, b"previous artifact").unwrap();
        let result = export_request::execute(
            conn,
            "ui.reports.export_year_handoff",
            &args,
            root.path(),
            &Fixed,
        )
        .unwrap();
        assert_eq!(text(&result, "format"), "xlsx");
        assert_eq!(text(&result, "scope"), "year_handoff");
        assert_eq!(
            text(&result, "filename"),
            "kassiber-year-handoff-2024-20260102-030405-2.xlsx"
        );
        assert_eq!(std::fs::read(&reserved).unwrap(), b"previous artifact");
        let target = std::path::PathBuf::from(text(&result, "file"));
        assert_eq!(
            int(&result, "bytes"),
            std::fs::metadata(&target).unwrap().len() as i64
        );
        let snapshot = value(&result, "report_snapshot");
        assert_eq!(text(snapshot, "report_state"), "saved");
        assert_eq!(text(snapshot, "report_kind"), "year-handoff.xlsx");
        assert_eq!(int(snapshot, "period_start_year"), 2024);
        assert_eq!(int(snapshot, "period_end_year"), 2024);
        assert_eq!(
            text(snapshot, "content_sha256"),
            export_completion::content_sha256(&[target]).unwrap()
        );
        assert!(rows(
            conn,
            "SELECT id FROM filed_report_snapshots WHERE report_state='filed'",
            []
        )
        .unwrap()
        .is_empty());
        for args in [
            dict::<&str>([]),
            dict([("year", Value::Bool(true))]),
            dict([("year", s("2024"))]),
            dict([("year", Value::Float(2024.0))]),
        ] {
            let error = export_request::execute(
                conn,
                "ui.reports.export_year_handoff",
                &args,
                root.path(),
                &Fixed,
            )
            .unwrap_err();
            assert_eq!(
                error.message,
                "ui.reports.export_year_handoff requires an integer year"
            );
        }
        let unknown = dict([("year", Value::Int(2024.into())), ("profile", s("p"))]);
        assert_eq!(
            export_request::execute(
                conn,
                "ui.reports.export_year_handoff",
                &unknown,
                root.path(),
                &Fixed
            )
            .unwrap_err()
            .message,
            "ui.reports.export_year_handoff received unsupported arguments"
        );
        conn.execute_batch("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','a','wallet','Cold','failed',1,'fixed'); UPDATE profiles SET last_processed_input_version=-1 WHERE id='p';").unwrap();
        for command in [
            "ui.reports.export_year_handoff",
            "reports.export-year-handoff",
        ] {
            assert_eq!(
                export_request::execute(conn, command, &args, root.path(), &Fixed)
                    .unwrap_err()
                    .code,
                "report_freshness_blocked"
            );
        }
    }
}
