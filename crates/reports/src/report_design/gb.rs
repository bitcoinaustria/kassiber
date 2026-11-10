//! The United Kingdom's layer of the shared design: the SA108 boxes per tax
//! year with the annual exempt amount and the reporting tests, 2024-25's
//! split at 30 October, one computation per token and UK day with its
//! matches (the computation HMRC asks for), the section 104 pools and
//! miscellaneous income (`docs/reference/tax-country-gb.md`). The data
//! comes from `crate::gb_tax`; nothing here decides a figure.

use super::*;
use kassiber_store::rusqlite::Connection;
use kassiber_tax_countries::calendar::Date;

pub(crate) fn collect(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
) -> crate::Result<Value> {
    crate::gb_tax::model(conn, profile, wallet)
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

fn btc(row: &Value, key: &str) -> Cell {
    match exact(row, key) {
        Cell::Fiat(d) => Cell::Btc(d),
        other => other,
    }
}

/// A box: whole pounds, the pence dropped (Q1).
fn pounds(row: &Value, key: &str) -> Cell {
    match value(row, key) {
        Value::Str(raw) => raw
            .as_str()
            .and_then(|raw| raw.parse::<Decimal>().ok())
            .and_then(|d| crate::gb_tax::pounds(&d).ok())
            .and_then(|whole| whole.parse().ok())
            .map(Cell::Fiat)
            .unwrap_or(Cell::Empty),
        _ => Cell::Empty,
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

fn answer(lang: Lang, v: &Value) -> Cell {
    Cell::Text(
        lang.t(match v {
            Value::Bool(true) => "gb.yes",
            Value::Bool(false) => "gb.no",
            _ => "gb.unknown",
        })
        .into(),
    )
}

/// The years whose last disposal is less than 31 days before the report
/// was generated: a purchase in the 30 days after it would change them.
fn provisional(data: &Value) -> Vec<String> {
    let generated = text(data, "generated_at");
    let Some(today) = generated.get(..10).and_then(Date::parse) else {
        return Vec::new();
    };
    list(data, "years")
        .iter()
        .filter(|y| {
            text(y, "last_disposal")
                .get(..10)
                .and_then(Date::parse)
                .is_some_and(|last| today.to_days() - last.to_days() <= 30)
        })
        .map(|y| text(y, "label"))
        .collect()
}

pub(crate) fn sections(data: &Value, lang: Lang) -> Vec<Section> {
    let t = |key: &str| lang.t(key);
    let mut sections = Vec::new();
    let refusals = list(data, "refusals");
    let mut blocks = vec![note(t("gb.sa108.lead"), false)];
    if truthy(value(data, "wallet_scoped")) {
        blocks.push(note(t("gb.wallet_scoped"), true));
    }
    if !refusals.is_empty() {
        // GB-FAILCLOSED: the figures are withheld until each is resolved.
        blocks.push(note(t("gb.refused"), true));
        let rows = refusals
            .iter()
            .map(|r| {
                let code = text(r, "code");
                vec![
                    Cell::Text(code.clone()),
                    Cell::Text(
                        lang.get(&format!("gb.code.{code}"))
                            .map_or_else(|| code.clone(), str::to_owned),
                    ),
                    Cell::Text(text(r, "transaction_id")),
                ]
            })
            .collect();
        blocks.push(table(
            "gb_refusals",
            t("gb.refusals.table").into(),
            vec![
                col("code", t("gb.col.code"), Kind::Code, 1.2),
                col("meaning", t("col.description"), Kind::Text, 2.0),
                col("transaction_id", t("gb.col.transaction"), Kind::Code, 1.2),
            ],
            rows,
        ));
        sections.push(Section {
            id: "gb_sa108",
            title: t("gb.sa108.title").into(),
            lead: String::new(),
            page_break: true,
            landscape: false,
            blocks,
        });
        return sections;
    }
    let years = list(data, "years");
    let pending = provisional(data);
    if !pending.is_empty() {
        blocks.push(note(
            lang.tf("gb.provisional", &[("years", &pending.join(", "))]),
            true,
        ));
    }
    let unknown: Vec<String> = years
        .iter()
        .filter(|y| matches!(value(y, "annual_exempt_amount"), Value::Null))
        .map(|y| text(y, "label"))
        .collect();
    if !unknown.is_empty() {
        blocks.push(note(
            lang.tf("gb.aea_unknown", &[("years", &unknown.join(", "))]),
            true,
        ));
    }
    if years.is_empty() {
        blocks.push(note(t("gb.none"), false));
    } else {
        let rows = years
            .iter()
            .map(|y| {
                vec![
                    Cell::Text(text(y, "label")),
                    Cell::Text(
                        t(if truthy(value(y, "crypto_boxes")) {
                            "gb.section.crypto"
                        } else {
                            "gb.section.other"
                        })
                        .into(),
                    ),
                    Cell::Count(int(y, "disposals")),
                    pounds(y, "proceeds"),
                    pounds(y, "allowable_costs"),
                    pounds(y, "gains"),
                    pounds(y, "losses"),
                ]
            })
            .collect();
        blocks.push(table(
            "gb_sa108",
            t("gb.sa108.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col("section", t("gb.col.section"), Kind::Text, 1.4),
                col("box_disposals", t("gb.col.box_count"), Kind::Count, 0.7),
                col("box_proceeds", t("gb.col.box_proceeds"), Kind::Fiat, 0.9),
                col("box_costs", t("gb.col.box_costs"), Kind::Fiat, 0.9),
                col("box_gains", t("gb.col.box_gains"), Kind::Fiat, 0.9),
                col("box_losses", t("gb.col.box_losses"), Kind::Fiat, 0.9),
            ],
            rows,
        ));
        let rows = years
            .iter()
            .map(|y| {
                vec![
                    Cell::Text(text(y, "label")),
                    exact(y, "proceeds"),
                    exact(y, "allowable_costs"),
                    exact(y, "gains"),
                    exact(y, "losses"),
                    exact(y, "net_gains"),
                    exact(y, "annual_exempt_amount"),
                    exact(y, "gains_above_aea"),
                    exact(y, "loss_available"),
                    answer(lang, value(y, "proceeds_over_threshold")),
                    answer(lang, value(y, "gains_over_aea")),
                    answer(lang, value(y, "loss_to_claim")),
                    exact(y, "misc_income"),
                ]
            })
            .collect();
        blocks.push(table(
            "gb_years",
            t("gb.years.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col("proceeds", t("gb.col.proceeds"), Kind::Fiat, 1.0),
                col(
                    "allowable_costs",
                    t("gb.col.allowable_costs"),
                    Kind::Fiat,
                    1.0,
                ),
                col("gains", t("gb.col.gains"), Kind::Fiat, 0.9),
                col("losses", t("gb.col.losses"), Kind::Fiat, 0.9),
                col("net_gains", t("gb.col.net_gains"), Kind::Fiat, 0.9),
                col("annual_exempt_amount", t("gb.col.aea"), Kind::Fiat, 0.8),
                col("gains_above_aea", t("gb.col.above_aea"), Kind::Fiat, 0.9),
                col(
                    "loss_available",
                    t("gb.col.loss_available"),
                    Kind::Fiat,
                    0.9,
                ),
                col(
                    "proceeds_over_threshold",
                    t("gb.col.test_proceeds"),
                    Kind::Text,
                    0.7,
                ),
                col("gains_over_aea", t("gb.col.test_gains"), Kind::Text, 0.7),
                col("loss_to_claim", t("gb.col.test_loss"), Kind::Text, 0.7),
                col("misc_income_box17", t("gb.col.box17"), Kind::Fiat, 0.9),
            ],
            rows,
        ));
        blocks.push(note(t("gb.tests.note"), false));
        let split: Vec<Vec<Cell>> = years
            .iter()
            .filter(|y| !matches!(value(y, "split"), Value::Null))
            .map(|y| {
                let figures = list(y, "split");
                let cell = |i: usize| match figures.get(i) {
                    Some(Value::Str(raw)) => raw
                        .as_str()
                        .and_then(|raw| raw.parse::<Decimal>().ok())
                        .map_or(Cell::Empty, Cell::Fiat),
                    _ => Cell::Empty,
                };
                vec![
                    Cell::Text(text(y, "label")),
                    Cell::Date(text(y, "split_date")),
                    cell(0),
                    cell(1),
                    cell(2),
                    cell(3),
                ]
            })
            .collect();
        if !split.is_empty() {
            blocks.push(note(t("gb.split.lead"), false));
            blocks.push(table(
                "gb_rate_split",
                t("gb.split.table").into(),
                vec![
                    col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                    col("split_date", t("gb.col.split_date"), Kind::Date, 0.8),
                    col("gains_before", t("gb.col.gains_before"), Kind::Fiat, 1.0),
                    col("losses_before", t("gb.col.losses_before"), Kind::Fiat, 1.0),
                    col("gains_from", t("gb.col.gains_from"), Kind::Fiat, 1.0),
                    col("losses_from", t("gb.col.losses_from"), Kind::Fiat, 1.0),
                ],
                split,
            ));
        }
    }
    sections.push(Section {
        id: "gb_sa108",
        title: t("gb.sa108.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // The computations and their matches: the attached computation.
    let computations = list(data, "computations");
    let matched = |c: &Value, rule: &str, key: &str| -> Cell {
        let mut total = Decimal::zero();
        let mut any = false;
        for m in list(c, "matches")
            .iter()
            .filter(|m| text(m, "rule") == rule)
        {
            if let Cell::Fiat(d) = exact(m, key) {
                total = total.add(&d, &DECIMAL).unwrap_or(total);
                any = true;
            }
        }
        match (any, key) {
            (false, _) => Cell::Empty,
            (true, "quantity") => Cell::Btc(total),
            (true, _) => Cell::Fiat(total),
        }
    };
    let rows: Vec<Vec<Cell>> = computations
        .iter()
        .map(|c| {
            vec![
                Cell::Text(text(c, "tax_year")),
                Cell::Date(text(c, "date")),
                Cell::Text(text(c, "asset")),
                btc(c, "quantity"),
                exact(c, "proceeds"),
                exact(c, "allowable_costs"),
                exact(c, "incidental"),
                exact(c, "gain"),
                matched(c, "same_day", "quantity"),
                matched(c, "same_day", "cost"),
                matched(c, "thirty_day", "quantity"),
                matched(c, "thirty_day", "cost"),
                matched(c, "s104", "quantity"),
                matched(c, "s104", "cost"),
                Cell::Text(
                    list(c, "events")
                        .iter()
                        .map(py_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("gb.computations.lead"), false)];
    blocks.push(if rows.is_empty() {
        note(t("gb.computations.none"), false)
    } else {
        table(
            "gb_computations",
            t("gb.computations.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col("date", t("col.date"), Kind::Date, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("proceeds", t("gb.col.proceeds"), Kind::Fiat, 1.0),
                col(
                    "allowable_costs",
                    t("gb.col.allowable_costs"),
                    Kind::Fiat,
                    1.0,
                ),
                col("incidental_costs", t("gb.col.incidental"), Kind::Fiat, 0.8),
                col("gain_loss", t("gb.col.gain"), Kind::Fiat, 1.0),
                col(
                    "same_day_quantity",
                    t("gb.col.same_day_quantity"),
                    Kind::Btc,
                    0.9,
                ),
                col("same_day_cost", t("gb.col.same_day_cost"), Kind::Fiat, 0.9),
                col(
                    "thirty_day_quantity",
                    t("gb.col.thirty_day_quantity"),
                    Kind::Btc,
                    0.9,
                ),
                col(
                    "thirty_day_cost",
                    t("gb.col.thirty_day_cost"),
                    Kind::Fiat,
                    0.9,
                ),
                col("s104_quantity", t("gb.col.s104_quantity"), Kind::Btc, 0.9),
                col("s104_cost", t("gb.col.s104_cost"), Kind::Fiat, 0.9),
                col("transactions", t("gb.col.transactions"), Kind::Code, 1.2),
            ],
            rows,
        )
    });
    let detail: Vec<Vec<Cell>> = computations
        .iter()
        .flat_map(|c| {
            list(c, "matches").iter().map(move |m| {
                vec![
                    Cell::Date(text(c, "date")),
                    Cell::Text(text(c, "asset")),
                    Cell::Text(lang.t(&format!("gb.rule.{}", text(m, "rule"))).into()),
                    match value(m, "acquired") {
                        Value::Null => Cell::Empty,
                        _ => Cell::Date(text(m, "acquired")),
                    },
                    btc(m, "quantity"),
                    exact(m, "cost"),
                    btc(m, "pool_quantity_before"),
                    exact(m, "pool_cost_before"),
                ]
            })
        })
        .collect();
    if !detail.is_empty() {
        blocks.push(table(
            "gb_matches",
            t("gb.matches.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("rule", t("gb.col.rule"), Kind::Text, 1.0),
                col("acquired", t("gb.col.acquired"), Kind::Date, 0.8),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("cost", t("gb.col.match_cost"), Kind::Fiat, 1.0),
                col(
                    "pool_quantity_before",
                    t("gb.col.pool_quantity_before"),
                    Kind::Btc,
                    1.0,
                ),
                col(
                    "pool_cost_before",
                    t("gb.col.pool_cost_before"),
                    Kind::Fiat,
                    1.0,
                ),
            ],
            detail,
        ));
    }
    sections.push(Section {
        id: "gb_computations",
        title: t("gb.computations.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: true,
        blocks,
    });

    // The pools at each tax year's end, and miscellaneous income.
    let pools: Vec<Vec<Cell>> = years
        .iter()
        .flat_map(|y| {
            list(y, "pools").iter().map(move |p| {
                vec![
                    Cell::Text(text(y, "label")),
                    Cell::Text(text(p, "asset")),
                    btc(p, "quantity"),
                    exact(p, "cost"),
                ]
            })
        })
        .collect();
    let mut blocks = vec![note(t("gb.pools.lead"), false)];
    if !pools.is_empty() {
        blocks.push(table(
            "gb_pools",
            t("gb.pools.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("pooled_cost", t("gb.col.pooled_cost"), Kind::Fiat, 1.0),
            ],
            pools,
        ));
    }
    let income: Vec<Vec<Cell>> = list(data, "income")
        .iter()
        .map(|row| {
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "tax_year")),
                Cell::Text(text(row, "kind")),
                Cell::Text(text(row, "asset")),
                btc(row, "quantity"),
                exact(row, "value"),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    blocks.push(note(t("gb.income.lead"), false));
    if !income.is_empty() {
        blocks.push(table(
            "gb_income",
            t("gb.income.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col("kind", t("col.kind"), Kind::Text, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("value", t("gb.col.income_value"), Kind::Fiat, 1.0),
                col("wallet", t("col.wallet"), Kind::Text, 0.8),
                col("transaction_id", t("gb.col.transaction"), Kind::Code, 1.2),
            ],
            income,
        ));
    }
    sections.push(Section {
        id: "gb_pools",
        title: t("gb.pools.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });
    sections
}

pub(crate) fn method(_data: &Value, lang: Lang) -> Vec<Fact> {
    let t = |key: &str| lang.t(key);
    [
        (
            "long_term_rule",
            "method.long_term_rule",
            "method.no_holding_period",
        ),
        ("pool_scope", "method.pool_scope", "gb.pool"),
        ("matching", "gb.method.matching", "gb.method.matching_value"),
        ("calendar", "gb.method.calendar", "gb.method.calendar_value"),
        ("fees", "gb.method.fees", "gb.method.fees_value"),
        (
            "residence",
            "gb.method.residence",
            "gb.method.residence_value",
        ),
        ("forms", "gb.method.forms", "gb.method.forms_value"),
    ]
    .into_iter()
    .map(|(key, label, value)| fact(key, t(label), Cell::Text(t(value).into())))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(generated_at: &str, refusals: Vec<Value>) -> Value {
        let year = dict([
            ("year", Value::Int(2024.into())),
            ("label", s("2024-25")),
            ("crypto_boxes", Value::Bool(true)),
            ("disposals", Value::Int(1.into())),
            ("proceeds", s("35006.999")),
            ("annual_exempt_amount", s("3000.00")),
            ("gains_over_aea", Value::Bool(true)),
            ("last_disposal", s("2025-03-06")),
        ]);
        dict([
            ("generated_at", s(generated_at)),
            ("refusals", Value::List(refusals)),
            ("years", Value::List(vec![year])),
        ])
    }

    fn notes(sections: &[Section]) -> Vec<String> {
        sections
            .iter()
            .flat_map(|section| &section.blocks)
            .filter_map(|block| match block {
                Block::Note { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_year_is_provisional_until_30_days_after_its_last_disposal() {
        // Day 30 after 6 March is 5 April: a purchase then still matches.
        let pending = |at: &str| {
            notes(&sections(&data(at, vec![]), Lang::En))
                .iter()
                .any(|note| note.contains("2024-25 are provisional"))
        };
        assert!(pending("2025-04-05T23:00:00Z"));
        assert!(!pending("2025-04-06T00:00:00Z"));
        assert!(!pending(""));
    }

    #[test]
    fn boxes_drop_the_pence_and_refusals_withhold_every_figure() {
        let sections = sections(&data("2026-01-01T00:00:00Z", vec![]), Lang::En);
        let boxes = sections
            .iter()
            .flat_map(|section| &section.blocks)
            .find_map(|block| match block {
                Block::Table(table) if table.id == "gb_sa108" => Some(table),
                _ => None,
            })
            .unwrap();
        assert_eq!(boxes.rows[0][3].csv(), "35006");
        let refused = data(
            "2026-01-01T00:00:00Z",
            vec![dict([
                ("code", s("gift_recipient_classification_required")),
                ("transaction_id", s("t1")),
            ])],
        );
        let sections = super::sections(&refused, Lang::En);
        let tables: Vec<&str> = sections
            .iter()
            .flat_map(|section| &section.blocks)
            .filter_map(|block| match block {
                Block::Table(table) => Some(table.id),
                _ => None,
            })
            .collect();
        assert_eq!(tables, ["gb_refusals"]);
    }

    #[test]
    fn the_full_report_places_the_uk_and_irish_layers_after_income() {
        for (country, first) in [("gb", "gb_sa108"), ("ie", "ie_cgt")] {
            let model = dict([
                (
                    "profile",
                    dict([("tax_country", s(country)), ("fiat_currency", s("GBP"))]),
                ),
                ("country_layer", data("2026-01-01T00:00:00Z", vec![])),
            ]);
            let document = crate::report_design::full(&model);
            let ids: Vec<&str> = document.sections.iter().map(|s| s.id).collect();
            let income = ids.iter().position(|id| *id == "income").unwrap();
            assert_eq!(ids[income + 1], first, "{country}");
            assert_eq!(
                document.preview_notice(),
                Some(kassiber_tax_countries::PREVIEW_NOTICE_EN)
            );
            let method = document.sections.iter().find(|s| s.id == "method").unwrap();
            let Block::Facts(facts) = &method.blocks[0] else {
                panic!("method facts")
            };
            assert!(facts.iter().any(|f| f.key == "long_term_rule"), "{country}");
            assert!(
                !facts.iter().any(|f| f.key == "long_term_days"),
                "{country}"
            );
        }
    }
}
