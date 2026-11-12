//! The German layer of the shared design: the Anlage SO figures per tax
//! year with their Freigrenze verdicts, the § 23 disposals per consumed lot,
//! income by section and the holdings at each year's end
//! (`docs/reference/tax-country-de.md`). The data comes from
//! `crate::de_tax`; nothing here decides a figure.

use super::*;
use kassiber_store::rusqlite::Connection;

pub(crate) fn collect(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
) -> crate::Result<Value> {
    crate::de_tax::model(conn, profile, wallet)
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

fn note(text: String, accent: bool) -> Block {
    Block::Note { text, accent }
}

/// A verdict's text, `—` for an unknown year.
fn verdict(lang: Lang, verdict: &Value) -> Cell {
    Cell::Text(
        lang.get(&format!("de.result.{}", text(verdict, "result")))
            .unwrap_or("\u{2014}")
            .into(),
    )
}

pub(crate) fn sections(data: &Value, lang: Lang) -> Vec<Section> {
    let t = |key: &str| lang.t(key);
    let years = list(data, "years");
    let mut sections = Vec::new();

    // Anlage SO: one row per tax year.
    let mut blocks = vec![note(t("de.anlage_so.lead").into(), false)];
    if truthy(value(data, "wallet_scoped")) {
        blocks.push(note(t("de.wallet_scoped").into(), true));
    }
    let unknown: Vec<String> = years
        .iter()
        .filter(|y| matches!(value(y, "lines"), Value::Null))
        .map(|y| int(y, "year").to_string())
        .collect();
    if !unknown.is_empty() {
        blocks.push(note(
            lang.tf("de.lines_unknown", &[("years", &unknown.join(", "))]),
            true,
        ));
    }
    let lines = |year: &Value, key: &str| -> Cell {
        match value(year, "lines") {
            Value::Null => Cell::Text("\u{2014}".into()),
            lines => Cell::Text(text(lines, key)),
        }
    };
    let p23_rows: Vec<Vec<Cell>> = years
        .iter()
        .map(|year| {
            let p23 = value(year, "p23");
            vec![
                Cell::Text(int(year, "year").to_string()),
                lines(year, "kryptowerte"),
                exact(year, "taxable_proceeds"),
                exact(year, "taxable_cost"),
                exact(year, "taxable_expenses"),
                exact(year, "net_gain"),
                exact(p23, "limit"),
                verdict(lang, p23),
                exact(p23, "amount"),
                exact(year, "outside"),
                lines(year, "loss"),
            ]
        })
        .collect();
    let p22_rows: Vec<Vec<Cell>> = years
        .iter()
        .map(|year| {
            let p22 = value(year, "p22_3_verdict");
            vec![
                Cell::Text(int(year, "year").to_string()),
                lines(year, "leistungen"),
                exact(year, "p22_3"),
                exact(p22, "limit"),
                verdict(lang, p22),
                exact(p22, "amount"),
                exact(year, "mining"),
            ]
        })
        .collect();
    if p23_rows.is_empty() {
        blocks.push(note(t("de.anlage_so.none").into(), false));
    } else {
        blocks.push(table(
            "anlage_so",
            t("de.anlage_so.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("p23_lines", t("de.col.p23_lines"), Kind::Text, 0.9),
                col("p23_proceeds", t("de.col.proceeds"), Kind::Fiat, 1.0),
                col("p23_cost", t("de.col.cost"), Kind::Fiat, 1.0),
                col("p23_expenses", t("de.col.expenses"), Kind::Fiat, 0.9),
                col("p23_net_gain", t("de.col.net_gain"), Kind::Fiat, 1.0),
                col("p23_freigrenze", t("de.col.freigrenze"), Kind::Fiat, 0.8),
                col("p23_result", t("de.col.result"), Kind::Text, 1.3),
                col("p23_amount", t("de.col.amount"), Kind::Fiat, 1.0),
                col("p23_outside", t("de.col.outside"), Kind::Fiat, 1.0),
                col("loss_line", t("de.col.loss_line"), Kind::Text, 0.6),
            ],
            p23_rows,
        ));
        blocks.push(table(
            "anlage_so_leistungen",
            t("de.anlage_so.table_p22_3").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("p22_3_lines", t("de.col.p22_3_lines"), Kind::Text, 0.8),
                col("p22_3_total", t("de.col.p22_3_total"), Kind::Fiat, 1.0),
                col("p22_3_freigrenze", t("de.col.freigrenze"), Kind::Fiat, 0.8),
                col("p22_3_result", t("de.col.result"), Kind::Text, 1.3),
                col("p22_3_amount", t("de.col.amount"), Kind::Fiat, 1.0),
                col("mining_total", t("de.col.mining"), Kind::Fiat, 1.0),
            ],
            p22_rows,
        ));
    }
    blocks.push(note(t("de.freigrenze.caveat").into(), false));
    sections.push(Section {
        id: "de_anlage_so",
        title: t("de.anlage_so.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // § 23 disposals, one row per consumed lot (Rn. 102: the list proves the
    // holding periods, so rows outside the period stay).
    let mut rows = Vec::new();
    for disposal in list(data, "disposals") {
        let date = text(disposal, "date");
        let kind = text(disposal, "kind");
        for fragment in list(disposal, "fragments") {
            let taxable = truthy(value(fragment, "taxable"));
            let acquired = text(fragment, "acquired");
            rows.push(vec![
                Cell::Text(date.get(..4).unwrap_or("").into()),
                Cell::Date(date.clone()),
                Cell::Text(
                    lang.get(&format!("de.kind.{kind}"))
                        .map_or_else(|| kind.clone(), str::to_owned),
                ),
                Cell::Text(text(disposal, "wallet")),
                Cell::Text(text(disposal, "asset")),
                quantity(fragment),
                if acquired.is_empty() {
                    Cell::Empty
                } else {
                    Cell::Date(acquired)
                },
                Cell::Text(
                    t(if taxable {
                        "de.holding.within"
                    } else {
                        "de.holding.beyond"
                    })
                    .into(),
                ),
                Cell::Text(t(if taxable { "de.yes" } else { "de.no" }).into()),
                exact(fragment, "proceeds"),
                exact(fragment, "cost"),
                exact(fragment, "expenses"),
                exact(fragment, "gain"),
                Cell::Text(text(disposal, "transaction_id")),
            ]);
        }
    }
    let mut blocks = vec![note(t("de.disposals.lead").into(), false)];
    if rows.is_empty() {
        blocks.push(note(t("de.disposals.none").into(), false));
    } else {
        blocks.push(table(
            "p23_disposals",
            t("de.disposals.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("disposed_at", t("de.col.disposed"), Kind::Date, 0.8),
                col("kind", t("col.kind"), Kind::Text, 0.7),
                col("wallet", t("col.wallet"), Kind::Text, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("acquired_at", t("de.col.acquired"), Kind::Date, 0.8),
                col("holding", t("de.col.holding"), Kind::Text, 0.6),
                col("taxable", t("de.col.taxable"), Kind::Text, 0.6),
                col("proceeds", t("de.col.proceeds"), Kind::Fiat, 0.9),
                col("cost", t("de.col.cost"), Kind::Fiat, 0.9),
                col("expenses", t("de.col.expenses"), Kind::Fiat, 0.8),
                col("gain", t("de.col.gain"), Kind::Fiat, 0.9),
                col("transaction_id", t("de.col.transaction"), Kind::Code, 1.1),
            ],
            rows,
        ));
    }
    sections.push(Section {
        id: "de_disposals",
        title: t("de.disposals.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // Income by section, and the holdings at each year's end.
    let income: Vec<Vec<Cell>> = list(data, "income")
        .iter()
        .map(|row| {
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "kind")),
                Cell::Text(t(&format!("de.section.{}", text(row, "section"))).into()),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row),
                exact(row, "value"),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("de.income.lead").into(), false)];
    if income.is_empty() {
        blocks.push(note(t("de.income.none").into(), false));
    } else {
        blocks.push(table(
            "income_sections",
            t("de.income.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("kind", t("col.kind"), Kind::Text, 0.8),
                col("section", t("de.col.section"), Kind::Text, 1.2),
                col("wallet", t("col.wallet"), Kind::Text, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("value", t("de.col.income_value"), Kind::Fiat, 0.9),
                col("transaction_id", t("de.col.transaction"), Kind::Code, 1.1),
            ],
            income,
        ));
    }
    let holdings: Vec<Vec<Cell>> = list(data, "holdings")
        .iter()
        .map(|row| {
            vec![
                Cell::Date(format!("{}-12-31", int(row, "year"))),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row),
            ]
        })
        .collect();
    blocks.push(note(t("de.holdings.lead").into(), false));
    if !holdings.is_empty() {
        blocks.push(table(
            "year_end_holdings",
            t("de.holdings.table").into(),
            vec![
                col("date", t("de.col.year_end"), Kind::Date, 0.8),
                col("wallet", t("col.wallet"), Kind::Text, 1.0),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
            ],
            holdings,
        ));
    }
    sections.push(Section {
        id: "de_income",
        title: t("de.income.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });
    sections
}

pub(crate) fn method(data: &Value, lang: Lang) -> Vec<Fact> {
    let t = |key: &str| lang.t(key);
    let average = text(data, "method") == "MOVING_AVERAGE_FIFO_PERIOD";
    let prices = list(data, "price_granularity")
        .iter()
        .map(|row| {
            let name = text(row, "granularity");
            format!(
                "{} {}",
                lang.get(&format!("de.granularity.{name}")).unwrap_or(&name),
                lang.integer(int(row, "rows").into())
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        fact(
            "long_term_rule",
            t("method.long_term_rule"),
            Cell::Text(t("de.holding_rule").into()),
        ),
        fact(
            "pool_scope",
            t("method.pool_scope"),
            Cell::Text(
                t(if average {
                    "de.pool.average"
                } else {
                    "de.pool.fifo"
                })
                .into(),
            ),
        ),
        fact(
            "fees",
            t("de.method.fees"),
            Cell::Text(t("de.method.fees_value").into()),
        ),
        fact(
            "price_granularity",
            t("de.method.prices"),
            Cell::Text(if prices.is_empty() {
                t("doc.none").into()
            } else {
                prices
            }),
        ),
        fact(
            "forms",
            t("de.method.forms"),
            Cell::Text(t("de.method.forms_value").into()),
        ),
        fact(
            "open_classifications",
            t("de.method.open"),
            Cell::Text(t("de.method.open_value").into()),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        let fragment = |acquired: &str, taxable: bool| {
            dict([
                ("lot", s("buy")),
                ("acquired", s(acquired)),
                ("quantity", s("0.3")),
                ("cost", s("9000")),
                ("proceeds", s("18000")),
                ("expenses", s("30")),
                ("gain", s("8970")),
                ("taxable", Value::Bool(taxable)),
            ])
        };
        let verdict = |result: &str| {
            dict([
                ("limit", s("1000.00")),
                ("result", s(result)),
                ("amount", s("1990")),
            ])
        };
        dict([
            ("wallet_scoped", Value::Bool(false)),
            ("method", s("FIFO")),
            (
                "disposals",
                Value::List(vec![dict([
                    ("transaction_id", s("s1")),
                    ("wallet", s("A")),
                    ("asset", s("BTC")),
                    ("kind", s("sell")),
                    ("date", s("2024-09-01")),
                    (
                        "fragments",
                        Value::List(vec![
                            fragment("2022-05-01", false),
                            fragment("2024-01-20", true),
                        ]),
                    ),
                ])]),
            ),
            (
                "years",
                Value::List(vec![
                    dict([
                        ("year", Value::Int(2024.into())),
                        ("net_gain", s("1990")),
                        ("p23", verdict("taxable")),
                        ("p22_3_verdict", verdict("none")),
                        (
                            "lines",
                            dict([
                                ("leistungen", s("10–16")),
                                ("kryptowerte", s("41–47, 54–55")),
                                ("loss", s("62")),
                            ]),
                        ),
                    ]),
                    dict([
                        ("year", Value::Int(2027.into())),
                        ("p23", dict([("result", s("unknown"))])),
                        ("p22_3_verdict", dict([("result", s("unknown"))])),
                        ("lines", Value::Null),
                    ]),
                ]),
            ),
            (
                "income",
                Value::List(vec![dict([
                    ("transaction_id", s("i1")),
                    ("date", s("2024-05-01")),
                    ("kind", s("mining")),
                    ("section", s("mining_classification_required")),
                    ("quantity", s("0.01")),
                    ("value", s("600")),
                ])]),
            ),
            (
                "price_granularity",
                Value::List(vec![dict([
                    ("granularity", s("daily")),
                    ("rows", Value::Int(2.into())),
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
    fn anlage_so_rows_carry_the_lines_and_verdicts_in_german() {
        let sections = sections(&data(), Lang::DeDe);
        let anlage = table(&sections, "anlage_so");
        let cells: Vec<String> = anlage.rows[0]
            .iter()
            .map(|c| c.display(Lang::DeDe))
            .collect();
        assert_eq!(cells[0], "2024");
        assert_eq!(cells[1], "41–47, 54–55");
        assert_eq!(cells[5], "1.990,00");
        assert_eq!(cells[7], "steuerpflichtig");
        // A year without lines on file prints none, and the note names it.
        assert_eq!(anlage.rows[1][1].display(Lang::DeDe), "\u{2014}");
        assert_eq!(anlage.rows[1][7].display(Lang::DeDe), "\u{2014}");
        assert!(sections[0]
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Note { text, .. } if text.contains("2027"))));
        let disposals = table(&sections, "p23_disposals");
        let first: Vec<String> = disposals.rows[0]
            .iter()
            .map(|c| c.display(Lang::DeDe))
            .collect();
        assert_eq!(
            first[..9],
            [
                "2024",
                "01.09.2024",
                "Verkauf",
                "A",
                "BTC",
                "0,30000000",
                "01.05.2022",
                "> 1 Jahr",
                "nein"
            ]
        );
        assert_eq!(disposals.rows[1][7].display(Lang::DeDe), "≤ 1 Jahr");
        assert_eq!(disposals.rows[1][8].display(Lang::DeDe), "ja");
        let income = table(&sections, "income_sections");
        assert_eq!(
            income.rows[0][2].display(Lang::DeDe),
            "Blockerstellung – Einordnung erforderlich"
        );
    }

    #[test]
    fn method_states_the_rule_the_pool_and_the_prices() {
        let facts = method(&data(), Lang::DeDe);
        let keys: Vec<&str> = facts.iter().map(|f| f.key).collect();
        assert_eq!(
            keys,
            [
                "long_term_rule",
                "pool_scope",
                "fees",
                "price_granularity",
                "forms",
                "open_classifications"
            ]
        );
        assert!(matches!(&facts[1].value, Cell::Text(t) if t.contains("FIFO je Wallet")));
        assert!(matches!(&facts[3].value, Cell::Text(t) if t == "Tageskurs 2"));
    }

    #[test]
    fn the_full_report_places_the_layer_and_the_german_notice() {
        let model = dict([
            (
                "profile",
                dict([("tax_country", s("de")), ("fiat_currency", s("EUR"))]),
            ),
            ("country_layer", data()),
        ]);
        let document = crate::report_design::full(&model);
        assert_eq!(document.lang, Lang::DeDe);
        let ids: Vec<&str> = document.sections.iter().map(|s| s.id).collect();
        let income = ids.iter().position(|id| *id == "income").unwrap();
        assert_eq!(
            ids[income + 1..income + 4],
            ["de_anlage_so", "de_disposals", "de_income"]
        );
        assert_eq!(
            document.preview_notice(),
            Some(kassiber_tax_countries::PREVIEW_NOTICE_DE)
        );
        assert!(
            document.kicker.ends_with("Deutschland"),
            "{}",
            document.kicker
        );
        // The layer's tables become sheets and CSV files next to the shared
        // ones and the full workbook's sheets: every name once.
        let mut ids: Vec<&str> = document
            .sections
            .iter()
            .flat_map(|s| &s.blocks)
            .filter_map(|b| match b {
                Block::Table(t) => Some(t.id),
                _ => None,
            })
            .chain(["summary", "disposals", "holdings", "income", "acquisitions"])
            .collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "{ids:?}");
    }
}
