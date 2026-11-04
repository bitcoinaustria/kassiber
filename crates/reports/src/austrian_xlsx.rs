//! Austrian workbook presentation over the admitted annual report.
use super::*;
use artifact_csv::{list, File, Prepared};
use kassiber_xlsx::{Document, Operation, Sheet, Style};

const TEXT: usize = 10;
const MONEY: usize = 13;

fn styles() -> Vec<Style> {
    let style = |size, bold, align: &str, wrap, number: Option<&str>| Style {
        font_size: Some(size),
        bold,
        valign: Some(align.into()),
        text_wrap: wrap,
        num_format: number.map(String::from),
        ..Style::default()
    };
    vec![
        style(16.0, true, "vcenter", false, None),
        style(12.0, true, "vcenter", false, None),
        style(11.0, true, "vcenter", false, None),
        style(11.0, false, "vcenter", false, None),
        style(11.0, false, "vcenter", false, Some("#,##0.00")),
        style(11.0, false, "vcenter", false, None),
        style(11.0, true, "vcenter", false, None),
        style(11.0, true, "vcenter", false, Some("#,##0.00")),
        Style {
            align: Some("center".into()),
            ..style(13.0, true, "vcenter", true, None)
        },
        style(11.0, true, "top", true, None),
        style(11.0, false, "top", true, None),
        style(11.0, false, "top", false, Some("0")),
        style(11.0, false, "top", false, Some("0.00000000")),
        style(11.0, false, "top", false, Some("#,##0.00")),
        style(11.0, true, "vcenter", true, None),
        style(11.0, true, "vcenter", false, Some("#,##0.00")),
        style(16.0, true, "vcenter", false, None),
        style(12.0, true, "top", false, None),
        style(11.0, false, "top", true, None),
    ]
}
fn margins() -> Operation {
    Operation::Margins {
        left: 0.35,
        right: 0.35,
        top: 0.5,
        bottom: 0.5,
    }
}
fn height(ops: &mut Vec<Operation>, row: u32, height: f64) {
    ops.push(Operation::Row { row, height });
}
fn write(ops: &mut Vec<Operation>, row: u32, col: u16, v: &Value, style: usize) -> Result<()> {
    ops.push(Operation::Cell {
        row,
        col,
        value: artifact_xlsx::cell(v)?,
        style,
    });
    Ok(())
}
fn merge(ops: &mut Vec<Operation>, row: u32, last: u16, text: String, style: usize) {
    ops.push(Operation::Merge {
        first_row: row,
        first_col: 0,
        last_row: row,
        last_col: last,
        text,
        style,
    });
}
fn euro(cents: &Value) -> Result<Value> {
    if matches!(cents, Value::Null) {
        return Ok(Value::Null);
    }
    float(
        &decimal(cents)?
            .div(&Decimal::from(100), &DECIMAL)
            .map_err(arithmetic)?,
    )
}
fn split(report: &Value, code: i64, key: &str) -> Value {
    let mut sum = kassiber_pyjson::BigInt::from(0);
    for row in list(value(report, "rows")) {
        if int(row, "kennzahl") == code {
            if let Value::Int(n) = value(row, key) {
                sum += n;
            }
        }
    }
    Value::Int(sum)
}
fn total(report: &Value, id: &str) -> Value {
    value(
        value(value(value(report, "sections"), id), "totals"),
        "amount_eur_cents",
    )
    .clone()
}
fn overview(report: &Value) -> Result<Sheet> {
    let mut ops = vec![margins()];
    for (col, width) in [72.0, 20.0, 8.0].into_iter().enumerate() {
        ops.push(Operation::Column {
            first: col as u16,
            last: col as u16,
            width,
        });
    }
    height(&mut ops, 0, 28.0);
    merge(&mut ops, 0, 2, "I. Übersicht".into(), 0);
    let mut row = 2;
    let mut section = String::new();
    let mut index = 0;
    for entry in list(value(austrian::metadata(), "overview")) {
        let entry = list(entry);
        let label = py_str(&entry[1]);
        match py_str(&entry[0]).as_str() {
            "heading" | "section" => {
                let heading = entry[0] == s("heading");
                if !heading {
                    section = label[..3].into();
                    index = 0;
                }
                height(
                    &mut ops,
                    row,
                    25.0_f64.max((artifact_xlsx::lines(&entry[1], 100.0) * 15 + 6) as f64),
                );
                merge(&mut ops, row, 2, label, if heading { 1 } else { 2 });
                row += if heading { 2 } else { 1 };
            }
            _ => {
                let cents = if section == "1.1" {
                    split(
                        report,
                        if index < 3 { 174 } else { 176 },
                        [
                            "proceeds_eur_cents",
                            "cost_basis_eur_cents",
                            "form_amount_eur_cents",
                        ][index % 3],
                    )
                } else if ["1.3", "2.1", "2.2", "3.1"].contains(&section.as_str()) {
                    total(report, &section)
                } else {
                    Value::Int(0.into())
                };
                let is_total = truthy(&entry[3]);
                height(
                    &mut ops,
                    row,
                    22.0_f64.max((artifact_xlsx::lines(&entry[1], 72.0) * 15 + 4) as f64),
                );
                write(&mut ops, row, 0, &entry[1], if is_total { 6 } else { 3 })?;
                write(
                    &mut ops,
                    row,
                    1,
                    &euro(&cents)?,
                    if is_total { 7 } else { 4 },
                )?;
                write(&mut ops, row, 2, &s("EUR"), 5)?;
                index += 1;
                row += 1;
            }
        }
    }
    Ok(Sheet {
        name: "Übersicht".into(),
        operations: ops,
    })
}
fn detail_values(row: &Value, id: &str) -> Result<Vec<Value>> {
    let or_empty = |key| {
        let v = value(row, key);
        if truthy(v) {
            v.clone()
        } else {
            s("")
        }
    };
    let mut cells = if ["2.1", "2.2"].contains(&id) {
        let kind = if truthy(value(row, "kind")) {
            or_empty("kind")
        } else {
            or_empty("entry_type")
        };
        vec![
            or_empty("wallet"),
            kind,
            or_empty("date"),
            value(row, "quantity").clone(),
            or_empty("asset"),
            if truthy(value(row, "tx_id")) {
                or_empty("tx_id")
            } else {
                or_empty("note")
            },
            euro(value(row, "form_amount_eur_cents"))?,
        ]
    } else {
        vec![
            or_empty("wallet"),
            or_empty("asset"),
            value(row, "quantity").clone(),
            s(""),
            or_empty("date"),
            s(""),
        ]
    };
    if !["2.1", "2.2"].contains(&id) {
        if ["1.3", "3.1"].contains(&id) {
            cells.push(value(row, "holding_period_days").clone());
        }
        cells.push(
            ["at_category_label", "at_category", "kind"]
                .iter()
                .map(|k| value(row, k))
                .find(|v| truthy(v))
                .cloned()
                .unwrap_or_else(|| s("")),
        );
        for k in [
            "cost_basis_eur_cents",
            "proceeds_eur_cents",
            "gain_loss_eur_cents",
        ] {
            cells.push(euro(value(row, k))?);
        }
    }
    Ok(cells)
}
fn detail(report: &Value, id: &str, spec: &Value) -> Result<Sheet> {
    let headers = list(value(spec, "headers"));
    let widths: Vec<f64> = list(value(spec, "column_widths"))
        .iter()
        .map(|v| py_str(v).parse().unwrap())
        .collect();
    let mut ops = vec![
        Operation::Landscape,
        Operation::Fit {
            width: 1,
            height: 0,
        },
        margins(),
    ];
    height(&mut ops, 0, 31.0);
    merge(
        &mut ops,
        0,
        (headers.len() - 1) as u16,
        text(spec, "title"),
        8,
    );
    let lines = headers
        .iter()
        .enumerate()
        .map(|(i, h)| artifact_xlsx::lines(h, widths[i]))
        .max()
        .unwrap_or(1);
    height(&mut ops, 1, 34.0_f64.max((lines * 15 + 6) as f64));
    for (i, h) in headers.iter().enumerate() {
        ops.push(Operation::Column {
            first: i as u16,
            last: i as u16,
            width: widths[i],
        });
        write(&mut ops, 1, i as u16, h, 9)?;
    }
    let mut row_index = 2;
    let details = list(value(value(value(report, "sections"), id), "detail_rows"));
    if details.is_empty() {
        height(&mut ops, row_index, 20.0);
        for i in 0..headers.len() {
            write(&mut ops, row_index, i as u16, &Value::Null, TEXT)?;
        }
        row_index += 1;
    } else {
        for row in details {
            let cells = detail_values(row, id)?;
            let lines = cells
                .iter()
                .zip(&widths)
                .map(|(v, w)| artifact_xlsx::lines(v, *w))
                .max()
                .unwrap_or(1);
            height(&mut ops, row_index, 22.0_f64.max((lines * 15 + 4) as f64));
            for (i, v) in cells.iter().enumerate() {
                let style = match list(value(spec, "row_format_names"))
                    .get(i)
                    .map(py_str)
                    .as_deref()
                {
                    Some("int") => 11,
                    Some("quantity") => 12,
                    Some("money") => MONEY,
                    _ => TEXT,
                };
                write(&mut ops, row_index, i as u16, v, style)?;
            }
            row_index += 1;
        }
    }
    row_index += 1;
    let col = int(spec, "value_column") as u16;
    let width = if col > 0 {
        widths[..col as usize].iter().sum()
    } else {
        *widths.first().unwrap_or(&16.0)
    };
    for (index, t) in list(value(spec, "total_rows")).iter().enumerate() {
        let label = &list(t)[0];
        let cents = if id == "1.1" {
            split(
                report,
                if index == 0 { 174 } else { 176 },
                "form_amount_eur_cents",
            )
        } else {
            total(report, id)
        };
        height(
            &mut ops,
            row_index,
            22.0_f64.max((artifact_xlsx::lines(label, width) * 15 + 4) as f64),
        );
        if col > 0 {
            merge(&mut ops, row_index, col - 1, py_str(label), 14);
        } else {
            write(&mut ops, row_index, 0, label, 14)?;
        }
        write(&mut ops, row_index, col, &euro(&cents)?, 15)?;
        row_index += 1;
    }
    Ok(Sheet {
        name: text(spec, "sheet_name"),
        operations: ops,
    })
}
fn explanations(report: &Value) -> Result<Sheet> {
    let mut rows=vec![
        ("Erläuterungen zum Steuerreport".into(),16),("".into(),18),("Berichtsumfang".into(),17),
        ("Dieser XLSX-Export ist als jährliche Arbeitsunterlage für die österreichische E 1kv-Prüfung aufgebaut. Die Übersicht fasst die Beträge zusammen; die nummerierten Blätter enthalten die dazugehörigen Detailzeilen oder explizite Null-Platzhalter für noch nicht modellierte Bereiche.".into(),18),
        ("Prüfung".into(),17),(text(report,"review_gate"),18),("Aktuelle Kassiber-Annahmen".into(),17),
    ];
    for r in list(value(report, "assumptions")) {
        rows.push((format!("{}: {}", text(r, "code"), text(r, "message")), 18));
    }
    let quality = value(report, "data_quality");
    let mismatches = list(value(quality, "kennzahl_mismatches"));
    rows.extend([("Datenqualität".into(),17),(format!("Quarantäne-Gründe im Jahr: {}; abweichende gespeicherte Kennzahlen: {}. Quarantinierte Transaktionen bleiben außerhalb dieser Arbeitsmappe, bis sie aufgelöst sind.",list(value(quality,"quarantines")).len(),mismatches.len()),18)]);
    if !mismatches.is_empty() {
        rows.extend([
            ("Kennzahl-Abweichungen".into(), 17),
            ("Transaktion | Kategorie | gespeichert | Export".into(), 18),
        ]);
        for r in mismatches {
            let category = value(r, "at_category");
            let label = get(
                value(austrian::metadata(), "AUSTRIAN_E1KV_CATEGORY_LABELS"),
                &py_str(category),
            )
            .unwrap_or(category);
            rows.push((
                format!(
                    "{} | {} | {} | {}",
                    text(r, "tx_id"),
                    py_str(label),
                    py_str(value(r, "stored_kennzahl")),
                    py_str(value(r, "export_kennzahl"))
                ),
                18,
            ));
        }
    }
    rows.extend([("Nicht modellierte Blätter".into(),17),("Margin/Derivate/Futures, Steuergebühren, Spenden/Schenkungen, verlorene Coins, kommerzielles Mining und Minting werden heute als leere Nullabschnitte dargestellt, weil Kassiber dafür noch keine strukturierten Steuerereignisse speichert.".into(),18)]);
    let mut ops = vec![
        Operation::Column {
            first: 0,
            last: 0,
            width: 105.0,
        },
        margins(),
    ];
    for (i, (text, style)) in rows.into_iter().enumerate() {
        let h = if [16, 17].contains(&style) {
            26.0
        } else if text.is_empty() {
            12.0
        } else {
            20.0_f64.max((artifact_xlsx::lines(&s(text.clone()), 105.0) * 15 + 6) as f64)
        };
        height(&mut ops, i as u32, h);
        write(&mut ops, i as u32, 0, &s(text), style)?;
    }
    Ok(Sheet {
        name: "Erläuterungen zum Steuerreport".into(),
        operations: ops,
    })
}
fn project(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(Value, Document)> {
    // The export transport holds admission through publication and completion.
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    require_context(conn, &profile, false)?;
    austrian::validate(&profile)?;
    let report = austrian::report(conn, &workspace, &profile, args)?;
    let mut sheets = vec![overview(&report)?];
    for (i, spec) in list(value(austrian::metadata(), "section_tables"))
        .iter()
        .enumerate()
    {
        sheets.push(detail(
            &report,
            &py_str(&list(value(&report, "section_order"))[i]),
            spec,
        )?);
    }
    sheets.push(explanations(&report)?);
    let document = Document {
        title: format!(
            "Kassiber Austrian E 1kv Report - {} ({})",
            text(&report, "profile"),
            py_str(value(&report, "tax_year"))
        ),
        subject: "Austrian E 1kv cryptocurrency tax handoff".into(),
        author: "Kassiber".into(),
        comments: text(&report, "review_gate"),
        created: sources.now(conn)?,
        formats: styles(),
        sheets,
        names: vec![],
    };
    Ok((report, document))
}

pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (report, document) = project(conn, args, sources)?;
    let bytes = kassiber_xlsx::render(&document)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    Ok(Prepared {
        files: vec![File {
            name: "report.xlsx".into(),
            sheet: String::new(),
            bytes,
            rows: list(value(&report, "rows")).len(),
        }],
        metadata: dict([
            ("form", value(&report, "form").clone()),
            ("tax_year", value(&report, "tax_year").clone()),
            (
                "sheets",
                value(austrian::metadata(), "AUSTRIAN_E1KV_XLSX_SHEETS").clone(),
            ),
            (
                "rows",
                Value::Int(list(value(&report, "rows")).len().into()),
            ),
            (
                "summary_rows",
                Value::Int(list(value(&report, "summary_rows")).len().into()),
            ),
        ]),
        completion: Some(export_completion::Completion {
            kind: "austrian-e1kv.xlsx".into(),
            year: Some(value(&report, "tax_year").clone()),
            end_year: None,
            scope: Value::Null,
        }),
    })
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
    // Resolve styles and compare operations independently of write order and
    // format indices. Openpyxl process tests additionally compare rendered cells.
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
    fn austrian_xlsx_projection_matches_python_workbook_vectors() {
        let mut cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/austrian_xlsx.json")).unwrap();
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
                assert_eq!(
                    error.code,
                    case["expected"]["error"]["code"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
                assert_eq!(
                    error.message,
                    case["expected"]["error"]["message"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
            } else {
                let (_, actual) = result.unwrap();
                let expected: Document =
                    serde_json::from_value(case["expected"]["document"].clone()).unwrap();
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
    fn austrian_xlsx_export_hashes_actual_bytes_and_saves_without_filing() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch("UPDATE profiles SET tax_country='at' WHERE id='p'")
            .unwrap();
        let target = root.path().join("annual.xlsx");
        let result = export::export(
            conn,
            "reports.export-austrian-e1kv-xlsx",
            &dict([("year", Value::Int(2025.into()))]),
            &target,
            &Fixed,
        )
        .unwrap();
        assert!(std::fs::read(&target).unwrap().starts_with(b"PK"));
        let snapshot = value(&result, "report_snapshot");
        assert_eq!(text(snapshot, "report_state"), "saved");
        assert_eq!(text(snapshot, "report_kind"), "austrian-e1kv.xlsx");
        assert_eq!(int(snapshot, "period_start_year"), 2025);
        assert_eq!(int(snapshot, "period_end_year"), 2025);
        assert_eq!(
            text(snapshot, "content_sha256"),
            export_completion::content_sha256(&[target]).unwrap()
        );
        assert_eq!(
            rows(
                conn,
                "SELECT id FROM filed_report_snapshots WHERE report_state='filed'",
                []
            )
            .unwrap()
            .len(),
            0
        );
    }
    #[test]
    fn austrian_xlsx_managed_request_preserves_format_and_freshness() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch("UPDATE profiles SET tax_country='at' WHERE id='p'")
            .unwrap();
        let args = dict([("year", Value::Int(2025.into()))]);
        let result = export_request::execute(
            conn,
            "ui.reports.export_austrian_e1kv_xlsx",
            &args,
            root.path(),
            &Fixed,
        )
        .unwrap();
        assert_eq!(
            text(&result, "filename"),
            "kassiber-austrian-e1kv-2025-20260102-030405.xlsx"
        );
        assert_eq!(text(&result, "format"), "xlsx");
        assert_eq!(text(&result, "scope"), "austrian_e1kv");
        assert!(std::fs::read(text(&result, "file"))
            .unwrap()
            .starts_with(b"PK"));
        conn.execute_batch("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','source','wallet','Cold','error',1,'fixed')").unwrap();
        assert_eq!(
            export_request::execute(
                conn,
                "ui.reports.export_austrian_e1kv_xlsx",
                &args,
                root.path(),
                &Fixed
            )
            .unwrap_err()
            .code,
            "report_freshness_blocked"
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("reports"))
                .unwrap()
                .count(),
            1
        );
    }
}
