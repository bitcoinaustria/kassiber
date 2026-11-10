//! Python's Austrian terminal report, rendered from the shared annual payload.
use super::*;
fn list(v: &Value) -> &[Value] {
    if let Value::List(v) = v {
        v
    } else {
        unreachable!()
    }
}
fn number(v: &Value, places: usize) -> String {
    super::summary_text::number(v, places)
}
fn cents(v: &Value) -> Result<Value> {
    float(
        &decimal(v)?
            .div(&Decimal::from(100), &DECIMAL)
            .map_err(arithmetic)?,
    )
}
fn money(v: &Value) -> Result<String> {
    Ok(number(&cents(v)?, 2))
}
fn kv(lines: &mut Vec<String>, label: &str, text: String, width: usize) {
    let label = format!("{label}:");
    lines.push(format!(
        "{label}{} {text}",
        " ".repeat(width.saturating_sub(label.chars().count()))
    ));
}
fn heading(lines: &mut Vec<String>, title: &str) {
    lines.extend([
        String::new(),
        title.into(),
        "-".repeat(if title == "E 1kv Kennzahlen" {
            title.chars().count() + 1
        } else {
            title.chars().count()
        }),
    ]);
}
fn table(lines: &mut Vec<String>, headers: Vec<String>, rows: Vec<Vec<String>>) {
    let line = |cells: Vec<String>| {
        format!(
            "| {} |",
            cells
                .into_iter()
                .map(|s| s.replace('|', "/").replace('\n', " "))
                .collect::<Vec<_>>()
                .join(" | ")
        )
    };
    lines.push(line(headers.clone()));
    lines.push(line(vec!["---".into(); headers.len()]));
    lines.extend(rows.into_iter().map(line));
}
fn split(report: &Value, code: i64, key: &str) -> Result<Value> {
    let mut sum = Value::Int(0.into());
    for row in list(value(report, "rows")) {
        if value(row, "kennzahl") == &Value::Int(code.into()) {
            if let (Value::Int(sum), Value::Int(n)) = (&mut sum, value(row, key)) {
                *sum += n;
            }
        }
    }
    Ok(sum)
}
pub fn render(report: &Value) -> Result<String> {
    let title = format!(
        "Kassiber Austrian E 1kv / Steuerbericht - {} ({})",
        text(report, "profile"),
        text(report, "tax_year")
    );
    let mut lines = vec![
        title.clone(),
        "=".repeat(title.chars().count()),
        String::new(),
    ];
    for (label, key) in [
        ("Workspace", "workspace"),
        ("Profile", "profile"),
        ("Tax year", "tax_year"),
        ("Fiat currency", "fiat_currency"),
        ("Tax country", "tax_country"),
        ("Form", "form"),
        ("Section", "form_section"),
    ] {
        kv(&mut lines, label, text(report, key), 28);
    }
    heading(&mut lines, "Hinweise zum Berichtsumfang");
    for row in list(value(report, "assumptions")) {
        lines.push(format!("{}: {}", text(row, "code"), text(row, "message")));
    }
    let summary = list(value(report, "summary_rows"));
    let summary_cells = |row: &Value| -> Result<Vec<String>> {
        Ok(vec![
            text(row, "kennzahl"),
            text(row, "label"),
            number(value(row, "row_count"), 0),
            money(value(row, "amount_eur_cents"))?,
        ])
    };
    heading(&mut lines, "E 1kv Kennzahlen");
    table(
        &mut lines,
        vec![
            "KZ".into(),
            "Description".into(),
            "Rows".into(),
            "Amount EUR".into(),
        ],
        summary
            .iter()
            .filter(|r| text(r, "form") == "E 1kv")
            .map(summary_cells)
            .collect::<Result<Vec<_>>>()?,
    );
    let outside: Vec<_> = summary
        .iter()
        .filter(|r| text(r, "form") != "E 1kv")
        .collect();
    if !outside.is_empty() {
        heading(&mut lines, "Other Austrian Kennzahlen");
        table(
            &mut lines,
            vec![
                "Form".into(),
                "KZ".into(),
                "Description".into(),
                "Rows".into(),
                "Amount EUR".into(),
            ],
            outside
                .into_iter()
                .map(|r| {
                    let mut cells = vec![text(r, "form")];
                    cells.extend(summary_cells(r)?);
                    Ok(cells)
                })
                .collect::<Result<Vec<_>>>()?,
        );
    }
    heading(&mut lines, "I. Übersicht");
    let sections = value(report, "sections");
    let mut current_section = String::new();
    let mut amount_index = 0;
    for entry in list(value(austrian::metadata(), "overview")) {
        let entry = list(entry);
        let kind = py_str(&entry[0]);
        let label = py_str(&entry[1]);
        match kind.as_str() {
            "heading" => heading(&mut lines, &label),
            "section" => {
                current_section = label[..3].into();
                amount_index = 0;
                lines.extend([String::new(), label]);
            }
            _ => {
                let amount = if current_section == "1.1" {
                    let code = if amount_index < 3 { 174 } else { 176 };
                    split(
                        report,
                        code,
                        [
                            "proceeds_eur_cents",
                            "cost_basis_eur_cents",
                            "form_amount_eur_cents",
                        ][amount_index % 3],
                    )?
                } else if ["1.3", "2.1", "2.2", "3.1"].contains(&current_section.as_str()) {
                    value(
                        value(value(sections, &current_section), "totals"),
                        "amount_eur_cents",
                    )
                    .clone()
                } else {
                    Value::Int(0.into())
                };
                amount_index += 1;
                kv(&mut lines, &label, format!("{} EUR", money(&amount)?), 54);
            }
        }
    }
    heading(&mut lines, "II. Detail Sections");
    for (i, spec) in list(value(austrian::metadata(), "section_tables"))
        .iter()
        .enumerate()
    {
        heading(&mut lines, &text(spec, "title"));
        let id = py_str(&list(value(report, "section_order"))[i]);
        let section = value(sections, &id);
        let mut table_rows = Vec::new();
        for row in list(value(section, "detail_rows")) {
            let income = ["2.1", "2.2"].contains(&id.as_str());
            let mut cells = if income {
                vec![
                    text(row, "wallet"),
                    text(row, "kind"),
                    text(row, "date"),
                    decimal(value(row, "quantity"))?
                        .to_fixed_places(8, Rounding::HalfEven)
                        .map_err(arithmetic)?,
                    text(row, "asset"),
                    if truthy(value(row, "tx_id")) {
                        text(row, "tx_id")
                    } else {
                        text(row, "note")
                    },
                    money(value(row, "form_amount_eur_cents"))?,
                ]
            } else {
                let mut cells = vec![
                    text(row, "wallet"),
                    text(row, "asset"),
                    decimal(value(row, "quantity"))?
                        .to_fixed_places(8, Rounding::HalfEven)
                        .map_err(arithmetic)?,
                    String::new(),
                    text(row, "date"),
                    String::new(),
                ];
                if ["1.3", "3.1"].contains(&id.as_str()) {
                    cells.push(String::new());
                }
                cells.extend([
                    text(row, "at_category_label"),
                    money(value(row, "cost_basis_eur_cents"))?,
                    money(value(row, "proceeds_eur_cents"))?,
                    money(value(row, "gain_loss_eur_cents"))?,
                ]);
                cells
            };
            table_rows.push(std::mem::take(&mut cells));
        }
        if table_rows.is_empty() {
            lines.push("No rows in scope.".into());
        } else {
            table(
                &mut lines,
                list(value(spec, "headers")).iter().map(py_str).collect(),
                table_rows,
            );
        }
        lines.push(String::new());
        for (index, total) in list(value(spec, "total_rows")).iter().enumerate() {
            let label = py_str(&list(total)[0]);
            let amount = if id == "1.1" {
                split(
                    report,
                    if index == 0 { 174 } else { 176 },
                    "form_amount_eur_cents",
                )?
            } else {
                value(value(section, "totals"), "amount_eur_cents").clone()
            };
            kv(&mut lines, &label, format!("{} EUR", money(&amount)?), 54);
        }
    }
    heading(&mut lines, "Data Quality");
    let quality = value(report, "data_quality");
    let quarantines = list(value(quality, "quarantines"));
    if quarantines.is_empty() {
        lines.push("No quarantined transactions in scope.".into());
    } else {
        lines.push("Quarantined transactions remain outside this export:".into());
        let row = |reason: &str, count: &str| {
            format!(
                "{}  {}",
                summary_text::cell(reason, 32, false),
                summary_text::cell(count, 8, true)
            )
            .trim_end()
            .to_owned()
        };
        lines.push(row("Reason", "Count"));
        lines.push(format!("{}  {}", "-".repeat(32), "-".repeat(8)));
        for q in quarantines {
            lines.push(row(&text(q, "reason"), &number(value(q, "count"), 0)));
        }
    }
    let mismatches = list(value(quality, "kennzahl_mismatches"));
    if mismatches.is_empty() {
        lines.push("Stored Kennzahlen match the current export mapping.".into());
    } else {
        lines.push(
            "Some rows had stale stored Kennzahlen; the export used the current category mapping."
                .into(),
        );
        table(
            &mut lines,
            ["Tx ID", "Category", "Stored KZ", "Export KZ"]
                .into_iter()
                .map(String::from)
                .collect(),
            mismatches
                .iter()
                .map(|row| {
                    vec![
                        text(row, "tx_id"),
                        text(
                            value(austrian::metadata(), "AUSTRIAN_E1KV_CATEGORY_LABELS"),
                            &text(row, "at_category"),
                        ),
                        text(row, "stored_kennzahl"),
                        text(row, "export_kennzahl"),
                    ]
                })
                .collect(),
        );
    }
    heading(&mut lines, "Review Footer");
    lines.push(text(report, "review_gate"));
    Ok(lines.join("\n"))
}
