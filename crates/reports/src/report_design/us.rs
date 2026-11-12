//! The United States layer of the shared design: Form 8949 rows by tax
//! year and part, the Schedule D lines they feed, ordinary income by kind
//! and network fees (`docs/reference/tax-country-us.md`). The data comes
//! from `crate::us_tax`; nothing here decides a figure.

use super::*;
use kassiber_store::rusqlite::Connection;

pub(crate) fn collect(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
) -> crate::Result<Value> {
    crate::us_tax::model(conn, profile, wallet)
}

fn exact(row: &Value, key: &str) -> Cell {
    match value(row, key) {
        Value::Str(raw) => raw
            .as_str()
            .and_then(|raw| raw.parse::<Decimal>().ok())
            .map(Cell::Fiat)
            .unwrap_or(Cell::Empty),
        _ => Cell::Empty,
    }
}

fn quantity(row: &Value) -> Cell {
    match exact(row, "quantity") {
        Cell::Fiat(d) => Cell::Btc(d),
        other => other,
    }
}

fn table(
    id: &'static str,
    title: String,
    columns: Vec<Column>,
    rows: Vec<Vec<Cell>>,
    note: String,
) -> Block {
    Block::Table(Table {
        id,
        title,
        columns,
        rows,
        total: None,
        note,
    })
}

pub(crate) fn sections(data: &Value, lang: Lang) -> Vec<Section> {
    let t = |key: &str| lang.t(key);
    let years = list(data, "years");
    let mut sections = Vec::new();

    // Form 8949: one table with the tax year, part and box of every row,
    // so a spreadsheet can filter what a form page holds.
    let box_of = |year: i64, term: &str| {
        crate::us_tax::boxes(year)
            .map(|(short, long)| if term == "long" { long } else { short })
            .unwrap_or("\u{2014}")
    };
    let mut rows = Vec::new();
    for row in list(data, "rows") {
        let sold = text(row, "sold");
        let year: i64 = sold.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0);
        let term = text(row, "term");
        rows.push(vec![
            Cell::Text(year.to_string()),
            Cell::Text(
                t(if term == "long" {
                    "us.part.long"
                } else {
                    "us.part.short"
                })
                .into(),
            ),
            Cell::Text(box_of(year, &term).into()),
            Cell::Text(format!(
                "{} {}",
                quantity(row).display(Lang::En),
                text(row, "asset")
            )),
            match text(row, "acquired").as_str() {
                "VARIOUS" => Cell::Text("VARIOUS".into()),
                day => Cell::Date(day.into()),
            },
            Cell::Date(sold),
            exact(row, "proceeds"),
            exact(row, "cost"),
            exact(row, "gain"),
            Cell::Text(text(row, "wallet")),
            Cell::Text(text(row, "transaction_id")),
        ]);
    }
    let mut blocks = vec![Block::Note {
        text: t("us.8949.lead").into(),
        accent: false,
    }];
    if truthy(value(data, "wallet_scoped")) {
        blocks.push(Block::Note {
            text: t("us.wallet_scoped").into(),
            accent: true,
        });
    }
    blocks.push(if rows.is_empty() {
        Block::Note {
            text: t("us.8949.none").into(),
            accent: false,
        }
    } else {
        table(
            "form_8949",
            t("us.8949.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col("part", t("us.col.part"), Kind::Text, 0.6),
                col("box", t("us.col.box"), Kind::Code, 0.4),
                col("description", t("us.col.description"), Kind::Text, 1.4),
                col("date_acquired", t("us.col.acquired"), Kind::Date, 0.9),
                col("date_sold", t("us.col.sold"), Kind::Date, 0.9),
                col("proceeds", t("us.col.proceeds"), Kind::Fiat, 1.0),
                col("cost_basis", t("us.col.cost"), Kind::Fiat, 1.0),
                col("gain_loss", t("us.col.gain"), Kind::Fiat, 1.0),
                col("wallet", t("col.wallet"), Kind::Text, 0.9),
                col("transaction_id", t("us.col.transaction"), Kind::Code, 1.2),
            ],
            rows,
            String::new(),
        )
    });
    sections.push(Section {
        id: "us_form_8949",
        title: t("us.8949.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // Schedule D lines and the year's other answers.
    let mut summary = Vec::new();
    for year in years {
        let supported = text(year, "status") == "supported";
        let line = |key: &str| {
            if supported {
                exact(year, key)
            } else {
                Cell::Empty
            }
        };
        summary.push(vec![
            Cell::Text(int(year, "year").to_string()),
            Cell::Text(
                t(if supported {
                    "us.status.supported"
                } else {
                    "us.status.unknown"
                })
                .into(),
            ),
            Cell::Text(if supported {
                text(year, "short_box")
            } else {
                "\u{2014}".into()
            }),
            exact(year, "short_proceeds"),
            exact(year, "short_cost"),
            line("short_gain"),
            Cell::Text(if supported {
                text(year, "long_box")
            } else {
                "\u{2014}".into()
            }),
            exact(year, "long_proceeds"),
            exact(year, "long_cost"),
            line("long_gain"),
            exact(year, "income"),
            exact(year, "fee_cost"),
            Cell::Text(
                t(if truthy(value(year, "digital_asset_question")) {
                    "us.yes"
                } else {
                    "us.no"
                })
                .into(),
            ),
        ]);
    }
    let unknown: Vec<String> = years
        .iter()
        .filter(|y| text(y, "status") != "supported")
        .map(|y| int(y, "year").to_string())
        .collect();
    let mut blocks = vec![Block::Note {
        text: t("us.schedule_d.lead").into(),
        accent: false,
    }];
    if !unknown.is_empty() {
        blocks.push(Block::Note {
            text: lang.tf("us.unknown_years", &[("years", &unknown.join(", "))]),
            accent: true,
        });
    }
    if !summary.is_empty() {
        blocks.push(table(
            "schedule_d",
            String::new(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col("forms", t("us.col.forms"), Kind::Text, 0.8),
                col("short_box", t("us.col.short_box"), Kind::Code, 0.5),
                col(
                    "short_proceeds",
                    t("us.col.short_proceeds"),
                    Kind::Fiat,
                    1.0,
                ),
                col("short_cost_basis", t("us.col.short_cost"), Kind::Fiat, 1.0),
                col("schedule_d_line_3", t("us.col.line3"), Kind::Fiat, 1.0),
                col("long_box", t("us.col.long_box"), Kind::Code, 0.5),
                col("long_proceeds", t("us.col.long_proceeds"), Kind::Fiat, 1.0),
                col("long_cost_basis", t("us.col.long_cost"), Kind::Fiat, 1.0),
                col("schedule_d_line_10", t("us.col.line10"), Kind::Fiat, 1.0),
                col("ordinary_income", t("us.col.income"), Kind::Fiat, 1.0),
                col("network_fee_cost", t("us.col.fee_cost"), Kind::Fiat, 0.9),
                col(
                    "digital_asset_question",
                    t("us.col.da_question"),
                    Kind::Text,
                    0.7,
                ),
            ],
            summary,
            String::new(),
        ));
    }
    sections.push(Section {
        id: "us_schedule_d",
        title: t("us.schedule_d.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: true,
        blocks,
    });

    // Ordinary income by receipt, and network fees.
    let income: Vec<Vec<Cell>> = list(data, "income")
        .iter()
        .map(|row| {
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "kind")),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row),
                exact(row, "value"),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let fees: Vec<Vec<Cell>> = list(data, "fees")
        .iter()
        .map(|row| {
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row),
                exact(row, "cost"),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let mut blocks = vec![Block::Note {
        text: t("us.income.lead").into(),
        accent: false,
    }];
    if !income.is_empty() {
        blocks.push(table(
            "us_income",
            t("us.income.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("kind", t("col.kind"), Kind::Text, 0.8),
                col("wallet", t("col.wallet"), Kind::Text, 0.9),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("value", t("us.col.income_value"), Kind::Fiat, 1.0),
                col("transaction_id", t("us.col.transaction"), Kind::Code, 1.2),
            ],
            income,
            String::new(),
        ));
    }
    blocks.push(Block::Note {
        text: t("us.fees.lead").into(),
        accent: false,
    });
    if !fees.is_empty() {
        blocks.push(table(
            "us_network_fees",
            t("us.fees.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("wallet", t("col.wallet"), Kind::Text, 0.9),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("cost_basis", t("us.col.fee_basis"), Kind::Fiat, 1.0),
                col("transaction_id", t("us.col.transaction"), Kind::Code, 1.2),
            ],
            fees,
            String::new(),
        ));
    }
    sections.push(Section {
        id: "us_income",
        title: t("us.income.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });
    sections
}

pub(crate) fn method(data: &Value, lang: Lang) -> Vec<Fact> {
    let t = |key: &str| lang.t(key);
    let scope = text(data, "pool_scope");
    vec![
        fact(
            "long_term_rule",
            t("method.long_term_rule"),
            Cell::Text(t("us.long_term_rule").into()),
        ),
        fact(
            "pool_scope",
            t("method.pool_scope"),
            Cell::Text(
                t(if scope == "global" {
                    "us.pool.global"
                } else {
                    "us.pool.wallet"
                })
                .into(),
            ),
        ),
        fact(
            "forms",
            t("us.method.forms"),
            Cell::Text(t("us.method.forms_value").into()),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        let row = |sold: &str, term: &str, acquired: &str| {
            dict([
                ("transaction_id", s("t")),
                ("wallet", s("Hot")),
                ("asset", s("BTC")),
                ("quantity", s("0.5")),
                ("acquired", s(acquired)),
                ("sold", s(sold)),
                ("proceeds", s("100")),
                ("cost", s("40")),
                ("gain", s("60")),
                ("term", s(term)),
            ])
        };
        let year = |year: i64, status: &str, short_box: &str, long_box: &str| {
            dict([
                ("year", Value::Int(year.into())),
                ("status", s(status)),
                ("short_box", s(short_box)),
                ("long_box", s(long_box)),
                ("short_gain", s("60")),
                ("long_gain", s("0")),
                ("digital_asset_question", Value::Bool(true)),
            ])
        };
        dict([
            ("pool_scope", s("wallet")),
            ("wallet_scoped", Value::Bool(false)),
            (
                "rows",
                Value::List(vec![
                    row("2024-05-01", "short", "VARIOUS"),
                    row("2025-05-01", "long", "2024-01-02"),
                    row("2027-05-01", "short", "2027-01-02"),
                ]),
            ),
            (
                "years",
                Value::List(vec![
                    year(2024, "supported", "C", "F"),
                    year(2025, "supported", "I", "L"),
                    year(2027, "unknown", "", ""),
                ]),
            ),
        ])
    }

    fn table<'a>(sections: &'a [Section], id: &str) -> &'a Table {
        sections
            .iter()
            .flat_map(|section| &section.blocks)
            .find_map(|block| match block {
                Block::Table(table) if table.id == id => Some(table),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no table {id}"))
    }

    #[test]
    fn form_8949_rows_carry_year_part_box_and_dates() {
        let sections = sections(&data(), Lang::En);
        let rows: Vec<Vec<String>> = table(&sections, "form_8949")
            .rows
            .iter()
            .map(|row| row[..6].iter().map(Cell::csv).collect())
            .collect();
        assert_eq!(
            rows,
            [
                [
                    "2024",
                    "Part I",
                    "C",
                    "0.50000000 BTC",
                    "VARIOUS",
                    "2024-05-01"
                ],
                [
                    "2025",
                    "Part II",
                    "L",
                    "0.50000000 BTC",
                    "2024-01-02",
                    "2025-05-01"
                ],
                [
                    "2027",
                    "Part I",
                    "\u{2014}",
                    "0.50000000 BTC",
                    "2027-01-02",
                    "2027-05-01"
                ],
            ]
        );
    }

    #[test]
    fn schedule_d_states_lines_only_for_years_with_forms() {
        let sections = sections(&data(), Lang::En);
        let schedule = table(&sections, "schedule_d");
        let line3 = schedule
            .columns
            .iter()
            .position(|c| c.key == "schedule_d_line_3")
            .unwrap();
        let cells: Vec<String> = schedule.rows.iter().map(|row| row[line3].csv()).collect();
        assert_eq!(cells, ["60", "60", ""]);
        let note = sections
            .iter()
            .flat_map(|section| &section.blocks)
            .any(|block| matches!(block, Block::Note { text, .. } if text.contains("2027")));
        assert!(note, "the unknown year is named");
    }

    #[test]
    fn method_states_the_rule_and_the_scope() {
        let facts = method(&data(), Lang::En);
        let keys: Vec<&str> = facts.iter().map(|f| f.key).collect();
        assert_eq!(keys, ["long_term_rule", "pool_scope", "forms"]);
        assert!(matches!(&facts[1].value, Cell::Text(t) if t.starts_with("Per wallet")));
    }

    #[test]
    fn the_full_report_places_the_layer_and_the_notice() {
        let model = dict([
            (
                "profile",
                dict([("tax_country", s("us")), ("fiat_currency", s("USD"))]),
            ),
            ("country_layer", data()),
        ]);
        let document = crate::report_design::full(&model);
        let ids: Vec<&str> = document.sections.iter().map(|s| s.id).collect();
        let income = ids.iter().position(|id| *id == "income").unwrap();
        assert_eq!(
            ids[income + 1..income + 4],
            ["us_form_8949", "us_schedule_d", "us_income"]
        );
        assert_eq!(
            document.preview_notice(),
            Some(kassiber_tax_countries::PREVIEW_NOTICE_EN)
        );
        // Every rendering carries the notice: the PDF's meta, the XLSX
        // summary sheet, the CSV summary and manifest.
        let stamp = kassiber_pdf::engine::Stamp::parse("2026-01-02T03:04:05Z").unwrap();
        let mut document = document;
        document.generated_at = "2026-01-02T03:04:05Z".into();
        let notice = kassiber_tax_countries::PREVIEW_NOTICE_EN;
        let pdf = crate::report_design::pdf_data(&document, stamp);
        assert_eq!(value(value(&pdf, "meta"), "notice"), &s(notice));
        let workbook = crate::report_design::xlsx_document(&document);
        assert!(workbook.sheets[0].operations.iter().any(|op| matches!(
            op,
            kassiber_xlsx::Operation::Cell { row: 2, value: kassiber_xlsx::Cell::String(text), .. } if text == notice
        )));
        let files = crate::report_design::csv(&document).unwrap();
        let summary = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(summary
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("preview,notice,Not yet reviewed"));
        let manifest = String::from_utf8(files.last().unwrap().1.clone()).unwrap();
        assert!(manifest.contains("\"preview_notice\": \"Not yet reviewed"));
        // A generic book has neither.
        let generic =
            crate::report_design::full(&dict([("profile", dict([("tax_country", s("generic"))]))]));
        assert!(!generic.sections.iter().any(|s| s.id.starts_with("us_")));
        assert_eq!(generic.preview_notice(), None);
    }
}
