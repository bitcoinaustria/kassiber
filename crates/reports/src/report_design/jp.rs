//! Japan's layer of the shared design: the NTA 暗号資産の計算書 per coin and
//! Tokyo year, the disposals with their average cost, the income receipts,
//! the return figures (確定申告書 第一表・第二表), and the review flags and
//! refused years (`docs/reference/tax-country-jp.md`). The data comes from
//! `crate::jp_tax`; nothing here decides a figure.

use super::*;
use kassiber_store::rusqlite::Connection;

pub(crate) fn collect(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
) -> crate::Result<Value> {
    crate::jp_tax::model(conn, profile, wallet)
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

/// A whole-yen figure of the return, as an integer.
fn yen(row: &Value, key: &str) -> Cell {
    match exact(row, key) {
        Cell::Fiat(d) => d
            .to_fixed_places(0, Rounding::HalfEven)
            .ok()
            .and_then(|text| text.parse::<i64>().ok())
            .map(Cell::Count)
            .unwrap_or(Cell::Fiat(d)),
        other => other,
    }
}

fn quantity(row: &Value, key: &str) -> Cell {
    match exact(row, key) {
        Cell::Fiat(d) => Cell::Btc(d),
        other => other,
    }
}

fn table(id: &'static str, title: String, columns: Vec<Column>, rows: Vec<Vec<Cell>>) -> Block {
    Block::Table(Table {
        id,
        title,
        columns,
        rows,
        total: None,
        note: String::new(),
    })
}

fn note(text: impl Into<String>, accent: bool) -> Block {
    Block::Note {
        text: text.into(),
        accent,
    }
}

fn label(lang: Lang, group: &str, code: &str) -> String {
    lang.get(&format!("jp.{group}.{code}"))
        .map_or_else(|| code.to_owned(), str::to_owned)
}

pub(crate) fn sections(data: &Value, lang: Lang) -> Vec<Section> {
    let t = |key: &str| lang.t(key);
    let mut sections = Vec::new();
    let method = text(data, "method");

    // The calculation sheet: per year and coin, the NTA sheet's columns.
    let mut sheets = Vec::new();
    for year in list(data, "years") {
        let y = int(year, "year").to_string();
        for sheet in list(year, "sheets") {
            sheets.push(vec![
                Cell::Text(y.clone()),
                Cell::Text(text(sheet, "asset")),
                quantity(sheet, "opening_qty"),
                exact(sheet, "opening_value"),
                quantity(sheet, "acquired_qty"),
                exact(sheet, "acquired_value"),
                exact(sheet, "unit_cost"),
                quantity(sheet, "disposed_qty"),
                exact(sheet, "cost_of_disposals"),
                quantity(sheet, "closing_qty"),
                exact(sheet, "closing_value"),
                exact(sheet, "revenue"),
                exact(sheet, "other_expenses"),
                yen(sheet, "revenue_rounded"),
                yen(sheet, "expenses_rounded"),
                yen(sheet, "income"),
            ]);
        }
    }
    let mut blocks = vec![note(
        t(if method == "moving_average" {
            "jp.sheet.lead_moving"
        } else {
            "jp.sheet.lead_total"
        }),
        false,
    )];
    if truthy(value(data, "wallet_scoped")) {
        blocks.push(note(t("jp.wallet_scoped"), true));
    }
    if sheets.is_empty() {
        blocks.push(note(t("jp.sheet.none"), false));
    } else {
        blocks.push(table(
            "jp_keisansho",
            String::new(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("opening_qty", t("jp.col.opening_qty"), Kind::Btc, 0.9),
                col("opening_value", t("jp.col.opening_value"), Kind::Fiat, 1.0),
                col("acquired_qty", t("jp.col.acquired_qty"), Kind::Btc, 0.9),
                col(
                    "acquired_value",
                    t("jp.col.acquired_value"),
                    Kind::Fiat,
                    1.0,
                ),
                col("unit_cost", t("jp.col.unit_cost"), Kind::Fiat, 1.0),
                col("disposed_qty", t("jp.col.disposed_qty"), Kind::Btc, 0.9),
                col("cost_of_disposals", t("jp.col.cost"), Kind::Fiat, 1.0),
                col("closing_qty", t("jp.col.closing_qty"), Kind::Btc, 0.9),
                col("closing_value", t("jp.col.closing_value"), Kind::Fiat, 1.0),
                col("revenue", t("jp.col.revenue"), Kind::Fiat, 1.0),
                col(
                    "other_expenses",
                    t("jp.col.other_expenses"),
                    Kind::Fiat,
                    0.9,
                )
                .sparse(),
                col(
                    "revenue_rounded",
                    t("jp.col.revenue_rounded"),
                    Kind::Count,
                    1.0,
                ),
                col(
                    "expenses_rounded",
                    t("jp.col.expenses_rounded"),
                    Kind::Count,
                    1.0,
                ),
                col("income", t("jp.col.income"), Kind::Count, 1.0),
            ],
            sheets,
        ));
    }
    sections.push(Section {
        id: "jp_keisansho",
        title: t("jp.sheet.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // Disposals with their average cost, and the income receipts.
    let disposals: Vec<Vec<Cell>> = list(data, "disposals")
        .iter()
        .map(|row| {
            vec![
                Cell::Text(int(row, "year").to_string()),
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row, "quantity"),
                exact(row, "revenue"),
                exact(row, "cost"),
                exact(row, "expenses"),
                Cell::Text(label(lang, "valuation", &text(row, "valuation"))),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let receipts: Vec<Vec<Cell>> = list(data, "income")
        .iter()
        .map(|row| {
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(label(lang, "kind", &text(row, "kind"))),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row, "quantity"),
                exact(row, "value"),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("jp.disposals.lead"), false)];
    if !disposals.is_empty() {
        blocks.push(table(
            "jp_disposals",
            t("jp.disposals.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("date", t("col.date"), Kind::Date, 0.8),
                col("wallet", t("col.wallet"), Kind::Text, 0.9),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("revenue", t("jp.col.revenue"), Kind::Fiat, 1.0),
                col("cost", t("jp.col.cost"), Kind::Fiat, 1.0),
                col("expenses", t("jp.col.other_expenses"), Kind::Fiat, 0.9).sparse(),
                col("valuation", t("jp.col.valuation"), Kind::Text, 1.0),
                col("transaction_id", t("jp.col.transaction"), Kind::Code, 1.2),
            ],
            disposals,
        ));
    }
    blocks.push(note(t("jp.income.lead"), false));
    if !receipts.is_empty() {
        blocks.push(table(
            "jp_income_receipts",
            t("jp.income.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("kind", t("col.kind"), Kind::Text, 0.9),
                col("wallet", t("col.wallet"), Kind::Text, 0.9),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("value", t("jp.col.receipt_value"), Kind::Fiat, 1.0),
                col("transaction_id", t("jp.col.transaction"), Kind::Code, 1.2),
            ],
            receipts,
        ));
    }
    sections.push(Section {
        id: "jp_disposals",
        title: t("jp.disposals.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: true,
        blocks,
    });

    // The return figures: 第一表 雑・その他 (区分 2) and 第二表.
    let figures: Vec<Vec<Cell>> = list(data, "years")
        .iter()
        .map(|year| {
            vec![
                Cell::Text(int(year, "year").to_string()),
                yen(year, "return_revenue"),
                yen(year, "income_receipts_rounded"),
                match value(year, "total") {
                    Value::Null => Cell::Empty,
                    _ => yen(year, "total"),
                },
                match value(year, "return_income") {
                    Value::Null => Cell::Empty,
                    _ => yen(year, "return_income"),
                },
                Cell::Text(t("jp.return.kubun").into()),
                Cell::Text(text(year, "venues")),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("jp.return.lead"), false)];
    if !figures.is_empty() {
        blocks.push(table(
            "jp_return",
            String::new(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col(
                    "revenue_zatsu_sonota",
                    t("jp.col.return_revenue"),
                    Kind::Count,
                    1.1,
                ),
                col("income_receipts", t("jp.col.receipts"), Kind::Count, 1.0),
                col("zatsu_crypto_total", t("jp.col.total"), Kind::Count, 1.0),
                col(
                    "income_zatsu_sonota",
                    t("jp.col.return_income"),
                    Kind::Count,
                    1.1,
                ),
                col("kubun", t("jp.col.kubun"), Kind::Text, 0.5),
                col("venues", t("jp.col.venues"), Kind::Text, 1.4),
            ],
            figures,
        ));
    }
    blocks.push(note(t("jp.return.threshold"), false));
    blocks.push(note(t("jp.return.no_tax"), false));
    sections.push(Section {
        id: "jp_return",
        title: t("jp.return.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // Review: flags and refused years.
    let mut rows: Vec<Vec<Cell>> = list(data, "flags")
        .iter()
        .map(|flag| {
            let code = text(flag, "code");
            vec![
                Cell::Text(text(flag, "event")),
                Cell::Text(code.clone()),
                Cell::Text(label(lang, "flag", &code)),
            ]
        })
        .collect();
    for refusal in list(data, "refusals") {
        let code = text(refusal, "reason");
        rows.push(vec![
            Cell::Text(int(refusal, "year").to_string()),
            Cell::Text(code.clone()),
            Cell::Text(label(lang, "flag", &code)),
        ]);
    }
    let mut blocks = vec![note(t("jp.flags.lead"), false)];
    if rows.is_empty() {
        blocks.push(note(t("jp.flags.none"), false));
    } else {
        blocks.push(table(
            "jp_review",
            String::new(),
            vec![
                col("event", t("jp.col.event"), Kind::Code, 1.0),
                col("code", t("jp.col.code"), Kind::Code, 1.4),
                col("explanation", t("jp.col.explanation"), Kind::Text, 3.0),
            ],
            rows,
        ));
    }
    sections.push(Section {
        id: "jp_review",
        title: t("jp.flags.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });
    sections
}

pub(crate) fn method(data: &Value, lang: Lang) -> Vec<Fact> {
    let t = |key: &str| lang.t(key);
    vec![
        fact(
            "long_term_rule",
            t("method.long_term_rule"),
            Cell::Text(t("jp.long_term_rule").into()),
        ),
        fact(
            "pool_scope",
            t("method.pool_scope"),
            Cell::Text(t("jp.pool").into()),
        ),
        fact(
            "cost_method",
            t("jp.method.cost"),
            Cell::Text(
                t(if text(data, "method") == "moving_average" {
                    "jp.method.moving"
                } else {
                    "jp.method.total"
                })
                .into(),
            ),
        ),
        fact(
            "calendar",
            t("jp.method.calendar"),
            Cell::Text(t("jp.method.calendar_value").into()),
        ),
        fact(
            "forms",
            t("jp.method.forms"),
            Cell::Text(t("jp.method.forms_value").into()),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        let sheet = dict([
            ("asset", s("BTC")),
            ("opening_qty", s("0")),
            ("opening_value", s("0")),
            ("acquired_qty", s("3")),
            ("acquired_value", s("1000000")),
            ("unit_cost", s("333333.33333333333333333333333333")),
            ("disposed_qty", s("1")),
            ("cost_of_disposals", s("333333.33333333333333333333333333")),
            ("closing_qty", s("2")),
            ("closing_value", s("666666.66666666666666666666666667")),
            ("revenue", s("500000.7")),
            ("other_expenses", s("0")),
            ("revenue_rounded", s("500000")),
            ("expenses_rounded", s("333334")),
            ("income", s("166666")),
        ]);
        dict([
            ("country", s("jp")),
            ("method", s("total_average")),
            ("wallet_scoped", Value::Bool(false)),
            (
                "years",
                Value::List(vec![dict([
                    ("year", Value::Int(2025.into())),
                    ("sheets", Value::List(vec![sheet])),
                    ("return_revenue", s("500000")),
                    ("income_receipts_rounded", s("0")),
                    ("total", s("166666")),
                    ("return_income", s("166666")),
                    ("venues", s("Exchange")),
                ])]),
            ),
            (
                "disposals",
                Value::List(vec![dict([
                    ("transaction_id", s("t2")),
                    ("wallet", s("Exchange")),
                    ("asset", s("BTC")),
                    ("year", Value::Int(2025.into())),
                    ("date", s("2025-02-10")),
                    ("quantity", s("1")),
                    ("revenue", s("500000.7")),
                    ("cost", s("333333.33333333333333333333333333")),
                    ("expenses", s("0")),
                    ("valuation", s("seventy_percent")),
                ])]),
            ),
            ("income", Value::List(vec![])),
            (
                "flags",
                Value::List(vec![dict([
                    ("event", s("2025")),
                    ("code", s("jp_zatsu_loss_no_offset")),
                ])]),
            ),
            (
                "refusals",
                Value::List(vec![dict([
                    ("year", Value::Int(2027.into())),
                    ("reason", s("jp_separate_taxation_regime_unconfirmed")),
                ])]),
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
    fn the_sheet_rounds_the_return_figures_to_whole_yen() {
        let sections = sections(&data(), Lang::Ja);
        let sheet = &table(&sections, "jp_keisansho").rows[0];
        let shown: Vec<String> = sheet.iter().map(|cell| cell.display(Lang::Ja)).collect();
        assert_eq!(shown[6], "333,333.33");
        assert_eq!(shown[13..], ["500,000", "333,334", "166,666"]);
        // XLSX and CSV keep the exact value.
        assert_eq!(sheet[10].csv(), "666666.66666666666666666666666667");
        let disposal = &table(&sections, "jp_disposals").rows[0];
        assert_eq!(disposal[1].display(Lang::Ja), "2025/02/10");
        assert!(disposal[8].csv().starts_with("時価の70%"));
        let review = &table(&sections, "jp_review").rows;
        assert_eq!(review[1][0].csv(), "2027");
        assert!(review[1][2].csv().contains("申告分離課税"));
        let ids: Vec<&str> = sections.iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            ["jp_keisansho", "jp_disposals", "jp_return", "jp_review"]
        );
    }

    #[test]
    fn the_full_report_is_japanese_and_carries_the_notice() {
        let model = dict([
            (
                "profile",
                dict([("tax_country", s("jp")), ("fiat_currency", s("JPY"))]),
            ),
            ("country_layer", data()),
        ]);
        let mut document = crate::report_design::full(&model);
        assert_eq!(document.lang, Lang::Ja);
        assert_eq!(document.kicker, "Kassiber · 日本");
        let notice = kassiber_tax_countries::PREVIEW_NOTICE_JA;
        assert_eq!(document.preview_notice(), Some(notice));
        let facts = method(&data(), Lang::Ja);
        assert!(matches!(&facts[2].value, Cell::Text(t) if t.starts_with("総平均法")));
        document.generated_at = "2026-01-02T03:04:05Z".into();
        let (bytes, _) = crate::report_design::pdf(&document).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        // Japanese text is drawn with the bundled face, not replaced.
        assert!(bytes.windows(10).any(|w| w == b"NotoSansJP"));
        let (again, _) = crate::report_design::pdf(&document).unwrap();
        assert_eq!(bytes, again);
    }
}
