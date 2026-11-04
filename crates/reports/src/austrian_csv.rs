//! Austrian CSV bundle presentation over the existing annual report payload.
use super::*;
use artifact_csv::{list, File, Prepared};

fn money(cents: &Value) -> Result<Value> {
    if matches!(cents, Value::Null) {
        return Ok(s(""));
    }
    let eur = float(
        &decimal(cents)?
            .div(&Decimal::from(100), &DECIMAL)
            .map_err(arithmetic)?,
    )?;
    Ok(s(summary_text::number(&eur, 2)))
}
fn split(report: &Value, code: i64, key: &str) -> Value {
    let mut sum = Value::Int(0.into());
    for row in list(value(report, "rows")) {
        if int(row, "kennzahl") == code {
            if let (Value::Int(sum), Value::Int(n)) = (&mut sum, value(row, key)) {
                *sum += n;
            }
        }
    }
    sum
}
fn amount(report: &Value, id: &str) -> Value {
    value(value(value(report, "sections"), id), "totals").clone()
}
fn overview(report: &Value) -> Result<Vec<Vec<Value>>> {
    let mut rows = vec![vec![s("I. Übersicht")]];
    let mut section = String::new();
    let mut index = 0;
    for entry in list(value(austrian::metadata(), "overview")) {
        let entry = list(entry);
        let label = entry[1].clone();
        match py_str(&entry[0]).as_str() {
            "heading" => {
                rows.push(vec![]);
                rows.push(vec![label]);
            }
            "section" => {
                section = py_str(&label)[..3].into();
                index = 0;
                rows.push(vec![label]);
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
                    value(&amount(report, &section), "amount_eur_cents").clone()
                } else {
                    Value::Int(0.into())
                };
                rows.push(vec![label, money(&cents)?, s("EUR")]);
                index += 1;
            }
        }
    }
    Ok(rows)
}
fn detail(report: &Value, id: &str, spec: &Value) -> Result<Vec<Vec<Value>>> {
    let mut rows = vec![
        vec![value(spec, "title").clone()],
        list(value(spec, "headers")).to_vec(),
    ];
    let details = list(value(value(value(report, "sections"), id), "detail_rows"));
    if details.is_empty() {
        rows.push(vec![s("No rows in scope.")]);
    }
    for row in details {
        let quantity = if matches!(value(row, "quantity"), Value::Null) {
            s("")
        } else {
            s(decimal(value(row, "quantity"))?
                .to_fixed_places(8, Rounding::HalfEven)
                .map_err(arithmetic)?)
        };
        let cells = if ["2.1", "2.2"].contains(&id) {
            vec![
                value(row, "wallet").clone(),
                value(row, "kind").clone(),
                value(row, "date").clone(),
                quantity,
                value(row, "asset").clone(),
                if truthy(value(row, "tx_id")) {
                    value(row, "tx_id").clone()
                } else {
                    value(row, "note").clone()
                },
                money(value(row, "form_amount_eur_cents"))?,
            ]
        } else {
            let mut cells = vec![
                value(row, "wallet").clone(),
                value(row, "asset").clone(),
                quantity,
                s(""),
                value(row, "date").clone(),
                s(""),
            ];
            if ["1.3", "3.1"].contains(&id) {
                cells.push(value(row, "holding_period_days").clone());
            }
            let category = ["at_category_label", "at_category", "kind"]
                .iter()
                .map(|key| value(row, key))
                .find(|v| truthy(v))
                .cloned()
                .unwrap_or_else(|| s(""));
            cells.extend([
                category,
                money(value(row, "cost_basis_eur_cents"))?,
                money(value(row, "proceeds_eur_cents"))?,
                money(value(row, "gain_loss_eur_cents"))?,
            ]);
            cells
        };
        rows.push(cells);
    }
    rows.push(vec![]);
    let value_column = int(spec, "value_column") as usize;
    let width = list(value(spec, "headers")).len().max(value_column + 1);
    for (index, total) in list(value(spec, "total_rows")).iter().enumerate() {
        let mut row = vec![s(""); width];
        row[0] = list(total)[0].clone();
        let cents = if id == "1.1" {
            split(
                report,
                if index == 0 { 174 } else { 176 },
                "form_amount_eur_cents",
            )
        } else {
            value(&amount(report, id), "amount_eur_cents").clone()
        };
        row[value_column] = money(&cents)?;
        rows.push(row);
    }
    Ok(rows)
}
fn explanations(report: &Value) -> Vec<Vec<Value>> {
    let mut rows=vec![
        vec![s("Erläuterungen zum Steuerreport")],vec![],vec![s("Berichtsumfang")],
        vec![s("Dieser CSV-Bundle-Export ist als jährliche Arbeitsunterlage für die österreichische E 1kv-Prüfung aufgebaut. Die Übersicht fasst die Beträge zusammen; die nummerierten Dateien enthalten Detailzeilen oder explizite Null-Platzhalter für noch nicht modellierte Bereiche.")],
        vec![s("Prüfung")],vec![value(report,"review_gate").clone()],vec![s("Aktuelle Kassiber-Annahmen")],
    ];
    for row in list(value(report, "assumptions")) {
        rows.push(vec![s(format!(
            "{}: {}",
            text(row, "code"),
            text(row, "message")
        ))]);
    }
    let quality = value(report, "data_quality");
    let mismatches = list(value(quality, "kennzahl_mismatches"));
    rows.extend([vec![s("Datenqualität")],vec![s(format!("Quarantäne-Gründe im Jahr: {}; abweichende gespeicherte Kennzahlen: {}. Quarantinierte Transaktionen bleiben außerhalb dieses Bundles, bis sie aufgelöst sind.",list(value(quality,"quarantines")).len(),mismatches.len()))]]);
    if !mismatches.is_empty() {
        rows.extend([
            vec![s("Kennzahl-Abweichungen")],
            vec![
                s("Transaktion"),
                s("Kategorie"),
                s("Gespeicherte KZ"),
                s("Export KZ"),
            ],
        ]);
        for row in mismatches {
            let category = value(row, "at_category");
            rows.push(vec![
                value(row, "tx_id").clone(),
                get(
                    value(austrian::metadata(), "AUSTRIAN_E1KV_CATEGORY_LABELS"),
                    &py_str(category),
                )
                .unwrap_or(category)
                .clone(),
                value(row, "stored_kennzahl").clone(),
                value(row, "export_kennzahl").clone(),
            ]);
        }
    }
    rows.extend([vec![s("Nicht modellierte Dateien")],vec![s("Margin/Derivate/Futures, Steuergebühren, Spenden/Schenkungen, verlorene Coins, kommerzielles Mining und Minting werden heute als leere Nullabschnitte dargestellt, weil Kassiber dafür noch keine strukturierten Steuerereignisse speichert.")]]);
    rows
}
pub(super) fn prepare(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
) -> Result<Prepared> {
    austrian::validate(profile)?;
    let report = austrian::report(conn, workspace, profile, args)?;
    let make = |name: String, sheet: String, rows: Vec<Vec<Value>>| File {
        name,
        sheet,
        bytes: artifact_csv::encode(&rows),
        rows: rows.len().saturating_sub(1),
    };
    let mut files = vec![make(
        "00_uebersicht.csv".into(),
        "Übersicht".into(),
        overview(&report)?,
    )];
    for (index, spec) in list(value(austrian::metadata(), "section_tables"))
        .iter()
        .enumerate()
    {
        let id = py_str(&list(value(&report, "section_order"))[index]);
        files.push(make(
            text(spec, "filename"),
            text(spec, "sheet_name"),
            detail(&report, &id, spec)?,
        ));
    }
    files.push(make(
        "99_erlaeuterungen_zum_steuerreport.csv".into(),
        "Erläuterungen zum Steuerreport".into(),
        explanations(&report),
    ));
    Ok(Prepared {
        files,
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
        completion: Some(crate::export_completion::Completion {
            kind: "austrian-e1kv.csv-bundle".into(),
            year: Some(value(&report, "tax_year").clone()),
            end_year: None,
            scope: Value::Null,
        }),
    })
}
