//! Ireland's layer of the shared design: the Form CG1 lines per year, the
//! tax by payment period and rate, the disposals with their rate and
//! period, Case IV income, and the rows a reviewer must look at
//! (`docs/reference/tax-country-ie.md`). The data comes from
//! `crate::ie_tax`; nothing here decides a figure.

use super::*;
use kassiber_store::rusqlite::Connection;

pub(crate) fn collect(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
) -> crate::Result<Value> {
    crate::ie_tax::model(conn, profile, wallet)
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

/// A form line: whole euros, the cents dropped (IE-R15, IE-Q8).
fn line(row: &Value, key: &str) -> Cell {
    match exact(row, key) {
        Cell::Fiat(d) => d
            .to_fixed_places(0, Rounding::Down)
            .ok()
            .and_then(|whole| whole.parse().ok())
            .map(Cell::Fiat)
            .unwrap_or(Cell::Empty),
        other => other,
    }
}

fn quantity(row: &Value) -> Cell {
    match exact(row, "quantity") {
        Cell::Fiat(d) => Cell::Btc(d),
        other => other,
    }
}

fn percent(row: &Value, key: &str) -> Cell {
    match exact(row, key) {
        Cell::Fiat(rate) => rate
            .mul(&Decimal::from(100u8), &DECIMAL)
            .map(Cell::Percent)
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

/// A code's explanation, or the code itself.
fn code_text(lang: Lang, code: &str) -> String {
    lang.get(&format!("ie.code.{code}"))
        .map_or_else(|| code.to_owned(), str::to_owned)
}

pub(crate) fn sections(data: &Value, lang: Lang) -> Vec<Section> {
    let t = |key: &str| lang.t(key);
    let mut sections = Vec::new();
    let years = list(data, "years");

    // Form CG1: one row per year, then the tax by period and rate.
    let mut blocks = vec![note(t("ie.cg1.lead"), false)];
    if truthy(value(data, "wallet_scoped")) {
        blocks.push(note(t("ie.wallet_scoped"), true));
    }
    let unsupported: Vec<String> = years
        .iter()
        .filter(|y| text(y, "status") != "supported")
        .map(|y| {
            format!(
                "{} ({})",
                int(y, "year"),
                code_text(lang, &text(y, "status"))
            )
        })
        .collect();
    if !unsupported.is_empty() {
        blocks.push(note(
            lang.tf(
                "ie.unsupported_years",
                &[("years", &unsupported.join(", "))],
            ),
            true,
        ));
    }
    if years.is_empty() {
        blocks.push(note(t("ie.none"), false));
    } else {
        let rows = years
            .iter()
            .map(|y| {
                vec![
                    Cell::Text(int(y, "year").to_string()),
                    Cell::Count(int(y, "disposals")),
                    line(y, "consideration"),
                    line(y, "line_10"),
                    line(y, "line_11"),
                    line(y, "line_14"),
                    line(y, "line_17"),
                    line(y, "line_18"),
                    line(y, "line_19"),
                    line(y, "line_21"),
                    line(y, "line_22"),
                    line(y, "line_23"),
                ]
            })
            .collect();
        blocks.push(table(
            "ie_cg1",
            t("ie.cg1.table").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                col(
                    "line_1k_disposals",
                    t("ie.col.line_1k_count"),
                    Kind::Count,
                    0.6,
                ),
                col(
                    "line_1k_consideration",
                    t("ie.col.line_1k"),
                    Kind::Fiat,
                    1.0,
                ),
                col("line_10", t("ie.col.line_10"), Kind::Fiat, 0.9),
                col("line_11", t("ie.col.line_11"), Kind::Fiat, 0.9),
                col("line_14", t("ie.col.line_14"), Kind::Fiat, 0.9),
                col("line_17", t("ie.col.line_17"), Kind::Fiat, 0.9),
                col("line_18", t("ie.col.line_18"), Kind::Fiat, 0.9),
                col("line_19", t("ie.col.line_19"), Kind::Fiat, 0.9),
                col("line_21", t("ie.col.line_21"), Kind::Fiat, 0.9),
                col("line_22", t("ie.col.line_22"), Kind::Fiat, 0.9),
                col("line_23", t("ie.col.line_23"), Kind::Fiat, 0.9),
            ],
            rows,
        ));
        let mut tax_rows = Vec::new();
        for y in years.iter().filter(|y| text(y, "status") == "supported") {
            for rate_line in list(y, "rate_lines") {
                tax_rows.push(vec![
                    Cell::Text(int(y, "year").to_string()),
                    Cell::Text(text(rate_line, "line")),
                    percent(rate_line, "rate"),
                    exact(rate_line, "amount"),
                ]);
            }
        }
        if !tax_rows.is_empty() {
            blocks.push(table(
                "ie_rate_lines",
                t("ie.rates.table").into(),
                vec![
                    col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                    col("line", t("ie.col.line"), Kind::Code, 0.4),
                    col("rate", t("ie.col.rate"), Kind::Percent, 0.5),
                    col("net_gain", t("ie.col.net_gain"), Kind::Fiat, 1.0),
                ],
                tax_rows,
            ));
        }
        let payments = years
            .iter()
            .filter(|y| text(y, "status") == "supported")
            .map(|y| {
                vec![
                    Cell::Text(int(y, "year").to_string()),
                    exact(y, "exemption"),
                    exact(y, "tax_total"),
                    exact(y, "payment_initial"),
                    exact(y, "payment_later"),
                    exact(y, "overpaid_initial"),
                    exact(y, "case_iv_income"),
                ]
            })
            .collect::<Vec<_>>();
        if !payments.is_empty() {
            blocks.push(table(
                "ie_payments",
                t("ie.payments.table").into(),
                vec![
                    col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                    col("exemption", t("ie.col.exemption"), Kind::Fiat, 0.9),
                    col("tax_total", t("ie.col.tax_total"), Kind::Fiat, 0.9),
                    col(
                        "payment_initial",
                        t("ie.col.payment_initial"),
                        Kind::Fiat,
                        1.0,
                    ),
                    col("payment_later", t("ie.col.payment_later"), Kind::Fiat, 1.0),
                    col("overpaid_initial", t("ie.col.overpaid"), Kind::Fiat, 1.0),
                    col("case_iv_income", t("ie.col.case_iv"), Kind::Fiat, 1.0),
                ],
                payments,
            ));
        }
        blocks.push(note(t("ie.not_modelled"), false));
    }
    sections.push(Section {
        id: "ie_cgt",
        title: t("ie.cg1.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // The disposals with their rate and payment period.
    let rows: Vec<Vec<Cell>> = list(data, "rows")
        .iter()
        .map(|row| {
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "asset")),
                Cell::Text(
                    lang.get(&format!("ie.kind.{}", text(row, "kind")))
                        .unwrap_or("")
                        .into(),
                ),
                quantity(row),
                exact(row, "consideration"),
                exact(row, "allowable_cost"),
                exact(row, "gain"),
                Cell::Text(
                    t(if text(row, "period") == "later" {
                        "ie.period.later"
                    } else {
                        "ie.period.initial"
                    })
                    .into(),
                ),
                percent(row, "rate"),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("ie.disposals.lead"), false)];
    blocks.push(if rows.is_empty() {
        note(t("ie.disposals.none"), false)
    } else {
        table(
            "ie_disposals",
            t("ie.disposals.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("kind", t("col.kind"), Kind::Text, 0.7),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("consideration", t("ie.col.consideration"), Kind::Fiat, 1.0),
                col(
                    "allowable_cost",
                    t("ie.col.allowable_cost"),
                    Kind::Fiat,
                    1.0,
                ),
                col("gain_loss", t("ie.col.gain"), Kind::Fiat, 1.0),
                col("period", t("ie.col.period"), Kind::Text, 0.7),
                col("rate", t("ie.col.rate"), Kind::Percent, 0.5),
                col("wallet", t("col.wallet"), Kind::Text, 0.8),
                col("transaction_id", t("ie.col.transaction"), Kind::Code, 1.2),
            ],
            rows,
        )
    });
    sections.push(Section {
        id: "ie_disposals",
        title: t("ie.disposals.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: true,
        blocks,
    });

    // Case IV income and the review list.
    let income: Vec<Vec<Cell>> = list(data, "income")
        .iter()
        .map(|row| {
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "kind")),
                Cell::Text(text(row, "asset")),
                quantity(row),
                exact(row, "value"),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("ie.income.lead"), false)];
    if !income.is_empty() {
        blocks.push(table(
            "ie_income",
            t("ie.income.table").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("kind", t("col.kind"), Kind::Text, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("value", t("ie.col.income_value"), Kind::Fiat, 1.0),
                col("wallet", t("col.wallet"), Kind::Text, 0.8),
                col("transaction_id", t("ie.col.transaction"), Kind::Code, 1.2),
            ],
            income,
        ));
    }
    let review: Vec<Vec<Cell>> = list(data, "flags")
        .iter()
        .map(|f| (f, "ie.review.flag"))
        .chain(
            list(data, "quarantines")
                .iter()
                .map(|q| (q, "ie.review.held")),
        )
        .map(|(row, what)| {
            let code = text(row, "code");
            vec![
                Cell::Text(t(what).into()),
                Cell::Text(code.clone()),
                Cell::Text(code_text(lang, &code)),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    blocks.push(note(t("ie.review.lead"), false));
    if !review.is_empty() {
        blocks.push(table(
            "ie_review",
            t("ie.review.table").into(),
            vec![
                col("status", t("ie.col.review"), Kind::Text, 0.6),
                col("code", t("ie.col.code"), Kind::Code, 1.2),
                col("meaning", t("col.description"), Kind::Text, 2.0),
                col("transaction_id", t("ie.col.transaction"), Kind::Code, 1.2),
            ],
            review,
        ));
    }
    sections.push(Section {
        id: "ie_income",
        title: t("ie.income.title").into(),
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
        ("pool_scope", "method.pool_scope", "ie.pool"),
        ("calendar", "ie.method.calendar", "ie.method.calendar_value"),
        ("rates", "ie.method.rates", "ie.method.rates_value"),
        (
            "exemption",
            "ie.method.exemption",
            "ie.method.exemption_value",
        ),
        ("losses", "ie.method.losses", "ie.method.losses_value"),
        (
            "residence",
            "ie.method.residence",
            "ie.method.residence_value",
        ),
        ("forms", "ie.method.forms", "ie.method.forms_value"),
    ]
    .into_iter()
    .map(|(key, label, value)| fact(key, t(label), Cell::Text(t(value).into())))
    .collect()
}
