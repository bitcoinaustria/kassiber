//! The Swiss layer of the shared design: the securities list
//! (Wertschriftenverzeichnis) with each holding's tax value at 31 December
//! and its income, the income by category, the KS 36 indicators, and the
//! tax-free realized gains for information (`docs/reference/tax-country-ch.md`).
//! The data comes from `crate::ch_tax`; nothing here decides a figure.

use super::*;
use kassiber_store::rusqlite::Connection;

pub(crate) fn collect(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
) -> crate::Result<Value> {
    crate::ch_tax::model(conn, profile, wallet)
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

/// A coded value's label, the code itself when the table has none.
fn label(lang: Lang, prefix: &str, code: &str) -> String {
    lang.get(&format!("{prefix}.{code}"))
        .map_or_else(|| code.to_owned(), str::to_owned)
}

pub(crate) fn sections(data: &Value, lang: Lang) -> Vec<Section> {
    let t = |key: &str| lang.t(key);
    let years = list(data, "years");
    let mut sections = Vec::new();

    // Wertschriftenverzeichnis: each holding at 31 December with its tax
    // value and the year's income from it (column B, no withholding tax).
    let income = list(data, "income");
    let mut holdings = Vec::new();
    let mut totals = Vec::new();
    for year in years {
        let y = int(year, "year");
        for row in list(year, "wealth") {
            let mut income_b = Decimal::zero();
            for item in income.iter().filter(|i| {
                text(i, "date").starts_with(&y.to_string())
                    && text(i, "category") == crate::ch_tax::MOVABLE
                    && text(i, "wallet") == text(row, "wallet")
                    && text(i, "asset") == text(row, "asset")
            }) {
                if let Cell::Fiat(v) = exact(item, "value") {
                    income_b = income_b.add(&v, &DECIMAL).unwrap_or(income_b);
                }
            }
            let flags: Vec<String> = match value(row, "flags") {
                Value::List(flags) => flags
                    .iter()
                    .map(|f| label(lang, "ch.flag", &py_str(f)))
                    .collect(),
                _ => Vec::new(),
            };
            let source = match value(row, "source") {
                Value::Null => String::new(),
                source => py_str(source),
            };
            holdings.push(vec![
                Cell::Date(format!("{y}-12-31")),
                Cell::Text(text(row, "asset")),
                Cell::Text(text(row, "wallet")),
                quantity(row),
                exact(row, "unit_value"),
                exact(row, "value"),
                Cell::Text(if source.is_empty() {
                    "\u{2014}".into()
                } else {
                    label(lang, "ch.source", &source)
                }),
                Cell::Fiat(income_b),
                Cell::Text(flags.join("; ")),
            ]);
        }
        totals.push(vec![
            Cell::Text(y.to_string()),
            if truthy(value(year, "wealth_complete")) {
                exact(year, "wealth_total")
            } else {
                Cell::Text(t("ch.wealth.incomplete").into())
            },
            // Column B is income from movable assets; mining and other
            // income go in the main form.
            exact(value(year, "income_by_category"), crate::ch_tax::MOVABLE),
        ]);
    }
    let mut blocks = vec![note(t("ch.wealth.lead").into(), false)];
    if truthy(value(data, "wallet_scoped")) {
        blocks.push(note(t("ch.wallet_scoped").into(), true));
    }
    if holdings.is_empty() {
        blocks.push(note(t("ch.wealth.none").into(), false));
    } else {
        blocks.push(table(
            "wertschriften",
            t("ch.wealth.table").into(),
            vec![
                col("date", t("ch.col.date"), Kind::Date, 0.7),
                col("asset", t("ch.col.description"), Kind::Text, 0.6),
                col("wallet", t("col.wallet"), Kind::Text, 0.8),
                col("quantity", t("ch.col.quantity"), Kind::Btc, 1.0),
                col("unit_value", t("ch.col.unit_value"), Kind::Fiat, 0.9),
                col("tax_value", t("ch.col.tax_value"), Kind::Fiat, 0.9),
                col("source", t("ch.col.source"), Kind::Text, 1.0),
                col("income_b", t("ch.col.income_b"), Kind::Fiat, 0.9),
                col("flags", t("ch.col.flags"), Kind::Text, 1.2),
            ],
            holdings,
        ));
        blocks.push(table(
            "wertschriften_total",
            t("ch.wealth.totals").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col("tax_value", t("ch.col.tax_value_total"), Kind::Fiat, 1.0),
                col("income_b", t("ch.col.income_b_total"), Kind::Fiat, 1.0),
            ],
            totals,
        ));
    }
    sections.push(Section {
        id: "ch_wertschriften",
        title: t("ch.wealth.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // Income by receipt and category, and the receipts kept out of it.
    let rows: Vec<Vec<Cell>> = income
        .iter()
        .map(|row| {
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "kind")),
                Cell::Text(label(lang, "ch.category", &text(row, "category"))),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row),
                exact(row, "value"),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("ch.income.lead").into(), false)];
    if rows.is_empty() {
        blocks.push(note(t("ch.income.none").into(), false));
    } else {
        blocks.push(table(
            "ch_income",
            t("ch.income.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("kind", t("col.kind"), Kind::Text, 0.8),
                col("category", t("ch.col.category"), Kind::Text, 1.4),
                col("wallet", t("col.wallet"), Kind::Text, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("value", t("ch.col.income_value"), Kind::Fiat, 0.9),
                col("transaction_id", t("ch.col.transaction"), Kind::Code, 1.1),
            ],
            rows,
        ));
    }
    let outside: Vec<Vec<Cell>> = list(data, "outside")
        .iter()
        .map(|row| {
            let kind = text(row, "kind");
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(label(lang, "ch.outside", &kind)),
                exact(row, "value"),
                Cell::Text(t(&format!("ch.treatment.{kind}")).into()),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    if !outside.is_empty() {
        blocks.push(table(
            "ch_outside_income",
            t("ch.outside.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("kind", t("col.kind"), Kind::Text, 0.8),
                col("value", t("ch.col.income_value"), Kind::Fiat, 0.9),
                col("treatment", t("ch.col.treatment"), Kind::Text, 2.0),
                col("transaction_id", t("ch.col.transaction"), Kind::Code, 1.1),
            ],
            outside,
        ));
    }
    sections.push(Section {
        id: "ch_income",
        title: t("ch.income.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // KS 36: the five criteria per year.
    let criterion = |code: String| Cell::Text(label(lang, "ch.criterion", &code));
    let rows: Vec<Vec<Cell>> = years
        .iter()
        .map(|year| {
            let ks = value(year, "ks36");
            vec![
                Cell::Text(int(year, "year").to_string()),
                criterion(text(ks, "c1")),
                criterion(text(ks, "c2")),
                exact(ks, "volume"),
                exact(ks, "limit"),
                criterion(text(ks, "c3")),
                exact(ks, "gains"),
                exact(ks, "half_income"),
                criterion(text(ks, "c4")),
                criterion(text(ks, "c5")),
                Cell::Text(label(lang, "ch.overall", &text(ks, "overall"))),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("ch.ks36.lead").into(), false)];
    if !truthy(value(data, "inputs_given")) {
        blocks.push(note(t("ch.ks36.no_inputs").into(), true));
    }
    if !rows.is_empty() {
        blocks.push(table(
            "ks36",
            t("ch.ks36.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("c1_holding", t("ch.col.c1"), Kind::Text, 0.9),
                col("c2_volume", t("ch.col.c2"), Kind::Text, 0.9),
                col("volume", t("ch.col.volume"), Kind::Fiat, 0.9),
                col("volume_limit", t("ch.col.limit"), Kind::Fiat, 0.9),
                col("c3_income", t("ch.col.c3"), Kind::Text, 0.9),
                col("realized_gains", t("ch.col.gains"), Kind::Fiat, 0.9),
                col("half_net_income", t("ch.col.half_income"), Kind::Fiat, 0.9),
                col("c4_financing", t("ch.col.c4"), Kind::Text, 0.9),
                col("c5_derivatives", t("ch.col.c5"), Kind::Text, 0.9),
                col("overall", t("ch.col.overall"), Kind::Text, 1.1),
            ],
            rows,
        ));
    }
    blocks.push(note(t("ch.ks36.caveat").into(), false));
    sections.push(Section {
        id: "ch_ks36",
        title: t("ch.ks36.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // Realized gains, tax-free for a private investor: information only.
    let mut rows = Vec::new();
    for disposal in list(data, "disposals") {
        let date = text(disposal, "date");
        for fragment in list(disposal, "fragments") {
            let acquired = text(fragment, "acquired");
            rows.push(vec![
                Cell::Date(date.clone()),
                Cell::Text(text(disposal, "wallet")),
                Cell::Text(text(disposal, "asset")),
                quantity(fragment),
                if acquired.is_empty() {
                    Cell::Empty
                } else {
                    Cell::Date(acquired)
                },
                Cell::Text(
                    t(if truthy(value(fragment, "held_6_months")) {
                        "ch.yes"
                    } else {
                        "ch.no"
                    })
                    .into(),
                ),
                exact(fragment, "proceeds"),
                exact(fragment, "cost"),
                exact(fragment, "gain"),
                Cell::Text(text(disposal, "transaction_id")),
            ]);
        }
    }
    let mut blocks = vec![note(t("ch.disposals.lead").into(), false)];
    if rows.is_empty() {
        blocks.push(note(t("ch.disposals.none").into(), false));
    } else {
        blocks.push(table(
            "ch_disposals",
            t("ch.disposals.table").into(),
            vec![
                col("disposed_at", t("ch.col.disposed"), Kind::Date, 0.8),
                col("wallet", t("col.wallet"), Kind::Text, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("acquired_at", t("ch.col.acquired"), Kind::Date, 0.8),
                col("held_6_months", t("ch.col.held_6_months"), Kind::Text, 0.7),
                col("proceeds", t("ch.col.proceeds"), Kind::Fiat, 0.9),
                col("cost", t("ch.col.cost"), Kind::Fiat, 0.9),
                col("gain", t("ch.col.gain"), Kind::Fiat, 0.9),
                col("transaction_id", t("ch.col.transaction"), Kind::Code, 1.1),
            ],
            rows,
        ));
    }
    sections.push(Section {
        id: "ch_disposals",
        title: t("ch.disposals.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });
    sections
}

pub(crate) fn method(data: &Value, lang: Lang) -> Vec<Fact> {
    let t = |key: &str| lang.t(key);
    let mut facts = vec![
        fact(
            "capital_gains",
            t("ch.method.gains"),
            Cell::Text(t("ch.method.gains_value").into()),
        ),
        fact(
            "valuation",
            t("ch.method.wealth"),
            Cell::Text(t("ch.method.wealth_value").into()),
        ),
        fact(
            "ks36_inputs",
            t("ch.method.inputs"),
            Cell::Text(t("ch.method.inputs_value").into()),
        ),
    ];
    if let Value::Str(canton) = value(data, "canton") {
        facts.push(fact(
            "canton",
            t("ch.method.canton"),
            Cell::Text(py_str(&Value::Str(canton.clone()))),
        ));
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        let year = |y: i64, complete: bool| {
            dict([
                ("year", Value::Int(y.into())),
                ("income_total", s("348.25")),
                (
                    "income_by_category",
                    dict([("movable_assets_income", s("348.25"))]),
                ),
                (
                    "wealth_total",
                    if complete {
                        s("42605.165")
                    } else {
                        Value::Null
                    },
                ),
                ("wealth_complete", Value::Bool(complete)),
                (
                    "wealth",
                    Value::List(vec![dict([
                        ("wallet", s("A")),
                        ("asset", s("BTC")),
                        ("quantity", s("0.5")),
                        ("unit_value", s("85210.33")),
                        ("value", s("42605.165")),
                        ("source", s("market_close")),
                        ("flags", Value::List(vec![s("not_estv_tax_value")])),
                    ])]),
                ),
                (
                    "ks36",
                    dict([
                        ("c1", s("not_met")),
                        ("c2", s("met")),
                        ("volume", s("180000")),
                        ("limit", s("180000")),
                        ("c3", s("not_assessable")),
                        ("gains", s("1234567.891")),
                        ("half_income", Value::Null),
                        ("c4", s("not_assessable")),
                        ("c5", s("not_assessable")),
                        ("overall", s("not_met")),
                    ]),
                ),
            ])
        };
        dict([
            ("wallet_scoped", Value::Bool(false)),
            ("inputs_given", Value::Bool(false)),
            ("canton", s("Zürich")),
            (
                "years",
                Value::List(vec![year(2024, true), year(2025, false)]),
            ),
            (
                "income",
                Value::List(vec![dict([
                    ("transaction_id", s("i1")),
                    ("wallet", s("A")),
                    ("asset", s("BTC")),
                    ("date", s("2024-03-31")),
                    ("kind", s("lending_interest")),
                    ("quantity", s("0.002")),
                    ("value", s("120.40")),
                    ("category", s("movable_assets_income")),
                ])]),
            ),
            (
                "outside",
                Value::List(vec![dict([
                    ("transaction_id", s("w1")),
                    ("date", s("2024-01-25")),
                    ("kind", s("wages")),
                    ("value", s("2000")),
                ])]),
            ),
            ("disposals", Value::List(vec![])),
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
    fn the_securities_list_states_values_sources_and_income_in_swiss_german() {
        let sections = sections(&data(), Lang::DeCh);
        let list = table(&sections, "wertschriften");
        let first: Vec<String> = list.rows[0].iter().map(|c| c.display(Lang::DeCh)).collect();
        assert_eq!(
            first,
            [
                "31.12.2024",
                "BTC",
                "A",
                "0.50000000",
                "85'210.33",
                "42'605.16",
                "Marktwert (nicht ESTV-Steuerwert)",
                "120.40",
                "kein ESTV-Steuerwert"
            ]
        );
        let totals = table(&sections, "wertschriften_total");
        assert_eq!(totals.rows[1][1].display(Lang::DeCh), "unvollständig");
        let ks = table(&sections, "ks36");
        assert_eq!(ks.rows[0][1].display(Lang::DeCh), "nicht erfüllt");
        assert_eq!(ks.rows[0][6].display(Lang::DeCh), "1'234'567.89");
        assert_eq!(ks.rows[0][7].display(Lang::DeCh), "\u{2014}");
        let outside = table(&sections, "ch_outside_income");
        assert_eq!(outside.rows[0][1].display(Lang::DeCh), "Lohn (Lohnausweis)");
        // Without inputs, the section says so.
        assert!(sections[2]
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Note { accent: true, .. })));
        for section in &sections {
            for block in &section.blocks {
                if let Block::Note { text, .. } = block {
                    assert!(!text.contains('ß'), "{text}");
                }
            }
        }
    }

    #[test]
    fn the_full_report_places_the_swiss_layer_and_notice() {
        let model = dict([
            (
                "profile",
                dict([("tax_country", s("ch")), ("fiat_currency", s("CHF"))]),
            ),
            ("country_layer", data()),
        ]);
        let document = crate::report_design::full(&model);
        assert_eq!(document.lang, Lang::DeCh);
        let ids: Vec<&str> = document.sections.iter().map(|s| s.id).collect();
        let income = ids.iter().position(|id| *id == "income").unwrap();
        assert_eq!(
            ids[income + 1..income + 5],
            ["ch_wertschriften", "ch_income", "ch_ks36", "ch_disposals"]
        );
        assert_eq!(
            document.preview_notice(),
            Some(kassiber_tax_countries::PREVIEW_NOTICE_DE)
        );
        assert!(document.kicker.ends_with("Schweiz"), "{}", document.kicker);
        let facts = method(&data(), Lang::DeCh);
        assert_eq!(facts.last().unwrap().key, "canton");
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
