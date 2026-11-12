//! The year handoff for EA / Jahresabschluss work in the shared design, from
//! the year handoff report (`year_handoff::build`). The roll-forward and
//! reconciliation checks stay live spreadsheet formulas.
use super::*;
use crate::report_design::{
    fact, intern as leak, list, Block, Cell, Column, Document, Fact, Kind, Lang, Section, Table,
};

/// The sheet specs and their report keys, in the workbook's order.
const SHEETS: [(&str, &str, &str); 7] = [
    ("Wallets", "wallets", "yh.wallets"),
    ("Transactions", "transactions", "yh.transactions"),
    ("Journal", "journal", "yh.journal"),
    ("Balances", "balances", "yh.balances"),
    ("Roll-forward", "roll_forward", "yh.roll_forward"),
    (
        "Reconciliation",
        "reconciliation",
        "yh.reconciliation_sheet",
    ),
    ("Excluded", "excluded", "yh.excluded"),
];

/// A report value as the column's kind: exact decimals stay exact.
fn typed(v: &Value, kind: Kind) -> Cell {
    let decimal = || match v {
        Value::Decimal(d) => Some(d.clone()),
        Value::Float(_) | Value::Int(_) => Some(crate::report_design::from_float(v)),
        Value::Str(raw) => raw.as_str().and_then(|t| t.trim().parse().ok()),
        _ => None,
    };
    match v {
        Value::Null => return Cell::Empty,
        Value::Str(raw) if raw.as_str() == Some("") => return Cell::Empty,
        _ => (),
    }
    match kind {
        Kind::Fiat => decimal()
            .map(Cell::Fiat)
            .unwrap_or_else(|| Cell::Text(py_str(v))),
        Kind::Btc => decimal()
            .map(Cell::Btc)
            .unwrap_or_else(|| Cell::Text(py_str(v))),
        Kind::Count => match v {
            Value::Int(n) => i64::try_from(n.clone())
                .map(Cell::Count)
                .unwrap_or_else(|_| Cell::Text(py_str(v))),
            Value::Bool(b) => Cell::Count(i64::from(*b)),
            _ => Cell::Text(py_str(v)),
        },
        _ => Cell::Text(match v {
            Value::Decimal(d) => d.to_string(),
            other => py_str(other),
        }),
    }
}

fn kind_of(format: &str) -> Kind {
    match format {
        "int" | "sats" => Kind::Count,
        "quantity" => Kind::Btc,
        "money" => Kind::Fiat,
        _ => Kind::Text,
    }
}

/// The year handoff document; the book's tax country picks the language.
pub fn year_handoff(report: &Value) -> Document {
    let profile = value(report, "profile");
    let country = text(profile, "tax_country");
    let lang = Lang::for_country(&country);
    let t = |key: &str| lang.t(key);
    let fiat = if truthy(value(profile, "fiat_currency")) {
        text(profile, "fiat_currency")
    } else {
        String::new()
    };
    let year = py_str(value(report, "year"));
    let book = format!(
        "{} / {}",
        text(value(report, "workspace"), "label"),
        text(profile, "label")
    );
    let metadata = crate::year_handoff_xlsx::metadata();
    let mut sections = Vec::new();

    // 1. Scope and run
    let mut assets = std::collections::BTreeMap::new();
    for key in ["transactions", "balances", "journal"] {
        for row in list(report, key) {
            assets
                .entry(text(row, "asset_id"))
                .or_insert_with(|| text(row, "asset"));
        }
    }
    let processed = value(profile, "last_processed_at");
    let scope = vec![
        fact("book", t("scope.book"), Cell::Text(book.clone())),
        fact("year", t("col.tax_year"), Cell::Text(year.clone())),
        fact(
            "calendar",
            t("yh.calendar"),
            Cell::Text(text(report, "calendar")),
        ),
        fact(
            "period_start",
            t("yh.period_start"),
            Cell::Text(text(report, "period_start")),
        ),
        fact(
            "period_end",
            t("yh.period_end"),
            Cell::Text(text(report, "period_end")),
        ),
        fact(
            "fiat_currency",
            t("scope.fiat_currency"),
            Cell::Text(fiat.clone()),
        ),
        fact(
            "tax_country",
            t("scope.tax_country"),
            Cell::Text(country.clone()),
        ),
        fact(
            "lot_method",
            t("scope.method"),
            Cell::Text(crate::report_design::method_name(
                lang,
                &text(profile, "gains_algorithm"),
            )),
        ),
        fact(
            "wallets",
            t("counts.wallets"),
            Cell::Count(list(report, "wallets").len() as i64),
        ),
        Fact {
            display: Some(
                assets
                    .iter()
                    .map(|(id, label)| format!("{label} ({id})"))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            ..fact("assets", t("col.assets"), Cell::Count(assets.len() as i64))
        },
        Fact {
            display: Some(lang.date_time(&text(report, "generated_at"))),
            ..fact(
                "generated_at",
                t("scope.generated_at"),
                Cell::Text(text(report, "generated_at")),
            )
        },
        Fact {
            display: truthy(processed).then(|| lang.date_time(&py_str(processed))),
            ..fact(
                "journals_processed_at",
                t("scope.last_processed"),
                if truthy(processed) {
                    Cell::Text(py_str(processed))
                } else {
                    Cell::Empty
                },
            )
        },
        fact(
            "kassiber_version",
            t("yh.version"),
            Cell::Text(text(report, "kassiber_version")),
        ),
    ];
    sections.push(Section {
        id: "scope",
        title: t("scope.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![Block::Facts(scope)],
    });

    // 2. Status: reconciliation is a blocker until explained
    let reconciled = truthy(value(report, "reconciled"));
    let mut status = vec![
        Fact {
            display: Some(
                t(if reconciled {
                    "yh.ok"
                } else {
                    "yh.differences"
                })
                .into(),
            ),
            accent: !reconciled,
            ..fact(
                "reconciliation",
                t("yh.reconciliation"),
                Cell::Text(if reconciled { "OK" } else { "DIFFERENCES" }.into()),
            )
        },
        fact("quarantined", t("yh.quarantined"), Cell::Count(0)),
    ];
    for (key, label, list_key) in [
        (
            "transactions_in_year",
            "yh.count.transactions",
            "transactions",
        ),
        ("excluded_in_year", "yh.count.excluded", "excluded"),
        ("journal_entries_in_year", "yh.count.journal", "journal"),
    ] {
        status.push(fact(
            key,
            t(label),
            Cell::Count(list(report, list_key).len() as i64),
        ));
    }
    let mut blocks = vec![Block::Facts(status)];
    for warning in list(report, "warnings") {
        blocks.push(Block::Note {
            text: py_str(warning),
            accent: true,
        });
    }
    let mut classes = std::collections::BTreeMap::<String, i64>::new();
    for row in list(report, "transactions") {
        *classes.entry(text(row, "classification")).or_default() += 1;
    }
    blocks.push(Block::Table(Table {
        id: "classification",
        title: t("yh.classification").into(),
        columns: vec![
            Column::new(
                "classification",
                t("yh.col.classification"),
                Kind::Text,
                2.0,
            ),
            Column::new("transactions", t("col.transactions"), Kind::Count, 1.0),
        ],
        rows: classes
            .into_iter()
            .map(|(k, n)| vec![Cell::Text(k), Cell::Count(n)])
            .collect(),
        total: None,
        note: String::new(),
    }));
    for (index, _) in list(metadata, "notes").iter().enumerate() {
        blocks.push(Block::Note {
            text: t(&format!("yh.note.{}", index + 1)).into(),
            accent: false,
        });
    }
    sections.push(Section {
        id: "status",
        title: t("yh.status").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 3. The ledgers, one landscape section each, checks as live formulas
    let specs = list(metadata, "sheets");
    for (name, key, title) in SHEETS {
        let Some(spec) = specs.iter().find(|s| text(s, "name") == name) else {
            continue;
        };
        let columns_spec = list(spec, "columns");
        let columns: Vec<Column> = columns_spec
            .iter()
            .map(|c| {
                let column_key = text(c, "key");
                let label = lang
                    .get(&format!("yh.col.{key}.{column_key}"))
                    .or_else(|| lang.get(&format!("yh.col.{column_key}")))
                    .map(str::to_owned)
                    .unwrap_or_else(|| text(c, "label"))
                    .replace("{fiat}", &fiat);
                let kind = kind_of(&text(c, "format"));
                Column::new(
                    leak(column_key),
                    label,
                    kind,
                    if kind == Kind::Text { 1.2 } else { 1.0 },
                )
            })
            .collect();
        let rows = list(report, key)
            .iter()
            .map(|row| {
                columns_spec
                    .iter()
                    .zip(&columns)
                    .map(|(c, column)| {
                        let raw = value(row, &text(c, "key"));
                        let cell = if text(c, "key") == "attachments" {
                            let entries = list(row, "attachments_list");
                            if get(row, "attachments_list").is_some() {
                                Cell::Text(
                                    entries
                                        .iter()
                                        .map(|e| text(e, "display_name"))
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                )
                            } else {
                                typed(raw, column.kind)
                            }
                        } else {
                            typed(raw, column.kind)
                        };
                        match value(c, "formula") {
                            Value::Str(formula) if truthy(value(c, "formula")) => Cell::Formula {
                                expression: formula
                                    .as_str()
                                    .unwrap_or_default()
                                    .replace("{decimals}", &py_str(value(row, "_decimals"))),
                                cached: Box::new(cell),
                            },
                            _ => cell,
                        }
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Cell>>>();
        let table = Table {
            id: leak(key.to_owned()),
            title: String::new(),
            columns,
            rows,
            total: None,
            note: String::new(),
        };
        sections.push(Section {
            id: leak(key.to_owned()),
            title: lang.tf(title, &[("year", &year)]),
            lead: String::new(),
            page_break: false,
            landscape: true,
            blocks: vec![if table.rows.is_empty() {
                Block::Note {
                    text: t("yh.no_rows").into(),
                    accent: false,
                }
            } else {
                Block::Table(table)
            }],
        });
    }

    Document {
        kind: "year_handoff",
        title: lang.tf("yh.title", &[("year", &year)]),
        kicker: lang.kicker(&country),
        book,
        period: lang.tf(
            "doc.period",
            &[
                ("start", &lang.date(&text(report, "period_start"))),
                ("end", &lang.date(&text(report, "period_end"))),
            ],
        ),
        period_start: text(report, "period_start"),
        period_end: text(report, "period_end"),
        generated_at: text(report, "generated_at"),
        input_version: match value(profile, "journal_input_version") {
            Value::Int(n) => i64::try_from(n.clone()).ok(),
            _ => None,
        },
        fiat,
        country,
        lang,
        sections,
    }
}
