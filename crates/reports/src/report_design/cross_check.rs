//! The cross-check against another tool's export, in the shared design. It
//! stays a diagnostic workbook: a summary, then one table per outcome.
use super::*;
use crate::report_design::{
    fact, intern, list, Block, Cell, Column, Document, Kind, Lang, Section, Table,
};

/// A number from the report (a tool's figure or Kassiber's): exact.
fn exact(v: &Value) -> Option<Decimal> {
    match v {
        Value::Null => None,
        Value::Str(raw) if raw.as_str() == Some("") => None,
        Value::Float(_) | Value::Int(_) => Some(crate::report_design::from_float(v)),
        Value::Decimal(d) => Some(d.clone()),
        other => py_str(other).trim().parse().ok(),
    }
}

fn items(v: &Value) -> &[Value] {
    crate::artifact_csv::list(v)
}

/// The table id of an outcome sheet (`Only in tool` is `only_in_tool`).
fn id(name: &str) -> &'static str {
    intern(name.to_lowercase().replace(' ', "_"))
}

/// The cross-check document in the book's language. `report` is the
/// cross-check report, `created` the export clock.
pub fn cross_check(report: &Value, created: &str, lang: Lang) -> Document {
    let t = |key: &str| lang.t(key);
    let tool = text(report, "tool_label");
    let fiat = text(report, "book_currency");
    let fill = |label: &str| label.replace("{tool}", &tool).replace("{fiat}", &fiat);
    let mut sections = Vec::new();

    // 1. What was compared, and the outcome counts
    let mut facts = vec![
        fact("file", t("cc.file"), Cell::Text(text(report, "file_name"))),
        fact(
            "year",
            t("col.tax_year"),
            Cell::Text(if truthy(value(report, "year")) {
                text(report, "year")
            } else {
                t("cc.all_years").into()
            }),
        ),
        fact(
            "calendar",
            t("yh.calendar"),
            Cell::Text(if truthy(value(report, "calendar")) {
                text(report, "calendar")
            } else {
                "UTC".into()
            }),
        ),
        fact(
            "wallets",
            t("scope.wallets"),
            Cell::Text(if truthy(value(report, "wallet")) {
                text(report, "wallet")
            } else {
                t("scope.all_wallets").into()
            }),
        ),
        fact(
            "tax_differences",
            t("cc.tax_differences"),
            match value(report, "tax_differences") {
                Value::Int(n) => Cell::Count(i64::try_from(n.clone()).unwrap_or(0)),
                other => Cell::Text(py_str(other)),
            },
        ),
    ];
    for status in crate::cross_check::STATUSES {
        facts.push(fact(
            intern(status.to_owned()),
            t(&format!("cc.status.{status}")),
            match value(value(report, "counts"), status) {
                Value::Int(n) => Cell::Count(i64::try_from(n.clone()).unwrap_or(0)),
                _ => Cell::Empty,
            },
        ));
    }
    let mut blocks = vec![Block::Facts(facts)];
    let totals: Vec<Vec<Cell>> = list(report, "totals")
        .iter()
        .map(|total| {
            let income = text(total, "metric") == "income_value";
            let figure = |key: &str| match exact(value(total, key)) {
                None => Cell::Empty,
                Some(d) if income => Cell::Fiat(d),
                Some(d) => Cell::Btc(d),
            };
            vec![
                Cell::Text(text(total, "asset")),
                Cell::Text(
                    lang.get(&format!("cc.metric.{}", text(total, "metric")))
                        .map(fill)
                        .unwrap_or_else(|| text(total, "metric")),
                ),
                figure("tool"),
                figure("kassiber"),
                figure("delta"),
            ]
        })
        .collect();
    if !totals.is_empty() {
        blocks.push(Block::Table(Table {
            id: "totals",
            title: t("cc.totals").into(),
            columns: vec![
                Column::new("asset", t("col.asset"), Kind::Text, 0.6),
                Column::new("metric", t("cc.metric"), Kind::Text, 1.3),
                Column::new("tool", fill(t("cc.tool")), Kind::Text, 1.1),
                Column::new("kassiber", "Kassiber", Kind::Text, 1.1),
                Column::new("difference", t("cc.difference"), Kind::Text, 1.1),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                if i >= 2 {
                    Column {
                        kind: Kind::Btc,
                        ..c
                    }
                } else {
                    c
                }
            })
            .collect(),
            rows: totals,
            total: None,
            note: t("cc.totals_note").into(),
        }));
    }
    let pairs = list(report, "pairs");
    if !pairs.is_empty() {
        blocks.push(Block::Table(Table {
            id: "wallet_pairs",
            title: t("cc.pairs").into(),
            columns: vec![
                Column::new("wallet", t("cc.col.kassiber_wallet"), Kind::Text, 1.2),
                Column::new("tool_names", fill(t("cc.col.tool_name")), Kind::Text, 1.4),
                Column::new("state", t("cc.col.state"), Kind::Text, 1.0),
                Column::new("rows", t("col.rows"), Kind::Count, 0.6),
                Column::new("to_review", t("cc.col.to_review"), Kind::Count, 0.7),
            ],
            rows: pairs
                .iter()
                .map(|pair| {
                    vec![
                        Cell::Text(if truthy(value(pair, "wallet")) {
                            text(pair, "wallet")
                        } else {
                            String::new()
                        }),
                        Cell::Text(
                            list(pair, "tool_names")
                                .iter()
                                .map(py_str)
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                        Cell::Text(text(pair, "state").replace('_', " ")),
                        match value(pair, "rows") {
                            Value::Int(n) => Cell::Count(i64::try_from(n.clone()).unwrap_or(0)),
                            _ => Cell::Empty,
                        },
                        match value(pair, "issues") {
                            Value::Int(n) => Cell::Count(i64::try_from(n.clone()).unwrap_or(0)),
                            _ => Cell::Empty,
                        },
                    ]
                })
                .collect(),
            total: None,
            note: String::new(),
        }));
    }
    for insight in list(report, "insights") {
        blocks.push(Block::Note {
            text: text(insight, "message"),
            accent: false,
        });
    }
    sections.push(Section {
        id: "summary",
        title: lang.tf("cc.title", &[("tool", &tool)]),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 2. One table per outcome, with both sides of every row
    let metadata = crate::cross_check_csv::metadata();
    let columns_spec = items(value(metadata, "columns"));
    for spec in items(value(metadata, "sheets")) {
        let spec = items(spec);
        let name = py_str(&spec[0]);
        let statuses = items(&spec[1]);
        let columns: Vec<Column> = columns_spec
            .iter()
            .map(|c| {
                let c = items(c);
                let key = py_str(&c[0]);
                let kind = match key.as_str() {
                    "tool_amount" | "tool_fee" | "book_amount" | "book_fee" => Kind::Btc,
                    "tool_value" | "book_value" => Kind::Fiat,
                    "txid" | "kassiber_ref" => Kind::Code,
                    _ => Kind::Text,
                };
                let label = lang
                    .get(&format!("cc.col.{key}"))
                    .map(fill)
                    .unwrap_or_else(|| fill(&py_str(&c[1])));
                Column::new(intern(key), label, kind, 1.0)
            })
            .collect();
        let rows: Vec<Vec<Cell>> = list(report, "rows")
            .iter()
            .filter(|r| statuses.contains(value(r, "status")))
            .map(|r| {
                let tool_side = value(r, "tool");
                let book = value(r, "kassiber");
                let mut differences = list(r, "differences")
                    .iter()
                    .map(|d| text(d, "message"))
                    .collect::<Vec<_>>()
                    .join("; ");
                if truthy(value(r, "reason")) {
                    differences = text(r, "reason");
                }
                columns
                    .iter()
                    .map(|column| {
                        let pick = |primary: &str, secondary: &str| {
                            if truthy(value(book, primary)) {
                                value(book, primary).clone()
                            } else {
                                value(tool_side, secondary).clone()
                            }
                        };
                        let raw = match column.key {
                            "status" => s(t(&format!("cc.status.{}", text(r, "status")))),
                            "match" => value(r, "match").clone(),
                            "differences" => s(differences.as_str()),
                            "tool_row" => value(tool_side, "row").clone(),
                            "tool_source" => value(tool_side, "source").clone(),
                            "tool_type" => value(tool_side, "type").clone(),
                            "tool_date" => value(tool_side, "occurred_at").clone(),
                            "direction" => pick("direction", "direction"),
                            "asset" => pick("asset", "asset"),
                            "wallet" => value(book, "wallet").clone(),
                            "classification" => value(book, "classification").clone(),
                            "book_date" => value(book, "occurred_at").clone(),
                            "txid" => pick("txid", "txid"),
                            "kassiber_ref" => s(if matches!(book, Value::Null) {
                                String::new()
                            } else {
                                text(book, "ref").chars().take(12).collect()
                            }),
                            k if k.starts_with("tool_") => value(tool_side, &k[5..]).clone(),
                            k => value(book, &k[5..]).clone(),
                        };
                        match column.kind {
                            Kind::Btc => exact(&raw).map(Cell::Btc).unwrap_or(Cell::Empty),
                            Kind::Fiat => exact(&raw).map(Cell::Fiat).unwrap_or(Cell::Empty),
                            _ if !truthy(&raw) => Cell::Text(String::new()),
                            _ => Cell::Text(py_str(&raw)),
                        }
                    })
                    .collect()
            })
            .collect();
        let table_id = id(&name);
        sections.push(Section {
            id: table_id,
            title: lang
                .get(&format!("cc.sheet.{table_id}"))
                .map(fill)
                .unwrap_or_else(|| fill(&name)),
            lead: String::new(),
            page_break: false,
            landscape: true,
            blocks: vec![if rows.is_empty() {
                Block::Note {
                    text: t("cc.none").into(),
                    accent: false,
                }
            } else {
                Block::Table(Table {
                    id: table_id,
                    title: String::new(),
                    columns,
                    rows,
                    total: None,
                    note: String::new(),
                })
            }],
        });
    }

    Document {
        kind: "cross_check",
        title: lang.tf("cc.title", &[("tool", &tool)]),
        kicker: t("doc.kicker").into(),
        book: text(report, "file_name"),
        period: if truthy(value(report, "year")) {
            text(report, "year")
        } else {
            t("cc.all_years").into()
        },
        period_start: String::new(),
        period_end: String::new(),
        generated_at: created.to_owned(),
        input_version: None,
        fiat,
        country: String::new(),
        lang,
        sections,
    }
}
