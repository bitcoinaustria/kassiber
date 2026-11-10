//! Spain's layer of the shared design: the casilla 1804 annex of every
//! transmission and its totals per coin and type of consideration, the
//! income casillas, the indicative crypto-only integration of the savings
//! base, and the review flags (`docs/reference/tax-country-es.md`). The data
//! comes from `crate::es_tax`; nothing here decides a figure.

use super::*;
use kassiber_store::rusqlite::Connection;

pub(crate) fn collect(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
) -> crate::Result<Value> {
    crate::es_tax::model(conn, profile, wallet)
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

fn note(text: impl Into<String>, accent: bool) -> Block {
    Block::Note {
        text: text.into(),
        accent,
    }
}

/// The label of a stored code, `es.<group>.<code>`, or the code itself.
fn label(lang: Lang, group: &str, code: &str) -> String {
    lang.get(&format!("es.{group}.{code}"))
        .map_or_else(|| code.to_owned(), str::to_owned)
}

pub(crate) fn sections(data: &Value, lang: Lang) -> Vec<Section> {
    let t = |key: &str| lang.t(key);
    let mut sections = Vec::new();
    let wallet_scoped = truthy(value(data, "wallet_scoped"));

    // Casilla 1804: every transmission, then the totals per year, coin and
    // type of consideration the form asks for.
    let rows: Vec<Vec<Cell>> = list(data, "disposals")
        .iter()
        .map(|row| {
            vec![
                Cell::Text(int(row, "year").to_string()),
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row),
                Cell::Text(label(lang, "kind", &text(row, "kind"))),
                Cell::Text(label(lang, "consideration", &text(row, "contraprestacion"))),
                exact(row, "valor_transmision"),
                exact(row, "valor_adquisicion"),
                exact(row, "ganancia"),
                Cell::Text(
                    t(if truthy(value(row, "computable")) {
                        "es.yes"
                    } else {
                        "es.no"
                    })
                    .into(),
                ),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("es.1804.lead"), false)];
    if wallet_scoped {
        blocks.push(note(t("es.wallet_scoped"), true));
    }
    if rows.is_empty() {
        blocks.push(note(t("es.1804.none"), false));
    } else {
        blocks.push(table(
            "es_transmisiones",
            t("es.1804.rows").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("date", t("col.date"), Kind::Date, 0.8),
                col("wallet", t("col.wallet"), Kind::Text, 0.9),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("kind", t("col.kind"), Kind::Text, 0.8),
                col(
                    "contraprestacion",
                    t("es.col.consideration"),
                    Kind::Text,
                    1.1,
                ),
                col(
                    "valor_transmision",
                    t("es.col.transmission"),
                    Kind::Fiat,
                    1.0,
                ),
                col(
                    "valor_adquisicion",
                    t("es.col.acquisition"),
                    Kind::Fiat,
                    1.0,
                ),
                col("ganancia", t("es.col.gain"), Kind::Fiat, 1.0),
                col("computable", t("es.col.computable"), Kind::Text, 0.6),
                col("transaction_id", t("es.col.transaction"), Kind::Code, 1.2),
            ],
            rows,
        ));
    }
    let mut coins = Vec::new();
    let mut totals = Vec::new();
    for year in list(data, "years") {
        let y = int(year, "year").to_string();
        for line in list(year, "by_coin") {
            coins.push(vec![
                Cell::Text(y.clone()),
                Cell::Text(text(line, "asset")),
                Cell::Text(label(
                    lang,
                    "consideration",
                    &text(line, "contraprestacion"),
                )),
                exact(line, "valor_transmision"),
                exact(line, "valor_adquisicion"),
                exact(line, "saldo"),
            ]);
        }
        totals.push(vec![
            Cell::Text(y),
            exact(year, "ganancias"),
            exact(year, "perdidas"),
            exact(year, "saldo"),
            exact(year, "non_computable_losses"),
        ]);
    }
    if !coins.is_empty() {
        blocks.push(table(
            "es_casilla_1804",
            t("es.1804.by_coin").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col(
                    "contraprestacion",
                    t("es.col.consideration"),
                    Kind::Text,
                    1.2,
                ),
                col(
                    "valor_transmision",
                    t("es.col.transmission"),
                    Kind::Fiat,
                    1.0,
                ),
                col(
                    "valor_adquisicion",
                    t("es.col.acquisition"),
                    Kind::Fiat,
                    1.0,
                ),
                col("saldo", t("es.col.balance"), Kind::Fiat, 1.0),
            ],
            coins,
        ));
    }
    if !totals.is_empty() {
        blocks.push(table(
            "es_casilla_1804_totals",
            t("es.1804.totals").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("ganancias", t("es.col.gains"), Kind::Fiat, 1.0),
                col("perdidas", t("es.col.losses"), Kind::Fiat, 1.0),
                col("saldo", t("es.col.balance"), Kind::Fiat, 1.0),
                col(
                    "non_computable_losses",
                    t("es.col.non_computable"),
                    Kind::Fiat,
                    1.0,
                )
                .sparse(),
            ],
            totals,
        ));
    }
    sections.push(Section {
        id: "es_casilla_1804",
        title: t("es.1804.title").into(),
        lead: String::new(),
        page_break: true,
        landscape: true,
        blocks,
    });

    // Income: each receipt with its casilla and base, and the casilla
    // totals per year.
    let receipts: Vec<Vec<Cell>> = list(data, "income")
        .iter()
        .map(|row| {
            let casilla = text(row, "casilla");
            vec![
                Cell::Date(text(row, "date")),
                Cell::Text(text(row, "kind")),
                Cell::Text(text(row, "wallet")),
                Cell::Text(text(row, "asset")),
                quantity(row),
                exact(row, "value"),
                Cell::Text(label(lang, "casilla", &casilla)),
                Cell::Text(label(lang, "base", &text(row, "base"))),
                Cell::Text(text(row, "transaction_id")),
            ]
        })
        .collect();
    let income_totals: Vec<Vec<Cell>> = list(data, "years")
        .iter()
        .map(|year| {
            vec![
                Cell::Text(int(year, "year").to_string()),
                exact(year, "casilla_0027"),
                exact(year, "rcm_art25_2"),
                exact(year, "casilla_0304"),
            ]
        })
        .collect();
    let mut blocks = vec![note(t("es.income.lead"), false)];
    if !receipts.is_empty() {
        blocks.push(table(
            "es_rendimientos",
            t("es.income.rows").into(),
            vec![
                col("date", t("col.date"), Kind::Date, 0.8),
                col("kind", t("col.kind"), Kind::Text, 0.8),
                col("wallet", t("col.wallet"), Kind::Text, 0.9),
                col("asset", t("col.asset"), Kind::Text, 0.5),
                col("quantity", t("col.quantity"), Kind::Btc, 1.0),
                col("value", t("es.col.income_value"), Kind::Fiat, 1.0),
                col("casilla", t("es.col.casilla"), Kind::Text, 1.0),
                col("base", t("es.col.base"), Kind::Text, 0.7),
                col("transaction_id", t("es.col.transaction"), Kind::Code, 1.2),
            ],
            receipts,
        ));
        blocks.push(table(
            "es_casillas_rendimientos",
            t("es.income.totals").into(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("casilla_0027", t("es.col.c0027"), Kind::Fiat, 1.0),
                col("rcm_art25_2", t("es.col.rcm"), Kind::Fiat, 1.0),
                col("casilla_0304", t("es.col.c0304"), Kind::Fiat, 1.0),
            ],
            income_totals,
        ));
    } else {
        blocks.push(note(t("es.income.none"), false));
    }
    sections.push(Section {
        id: "es_rendimientos",
        title: t("es.income.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: true,
        blocks,
    });

    // The savings base, crypto only: indicative, since the taxpayer's other
    // gains and yields change it.
    let mut integration = Vec::new();
    let mut carry = Vec::new();
    for year in list(data, "years") {
        let y = int(year, "year");
        integration.push(vec![
            Cell::Text(y.to_string()),
            exact(year, "saldo"),
            exact(year, "rcm"),
            exact(year, "rcm_offset_25pct"),
            exact(year, "prior_losses_applied"),
            exact(year, "base"),
        ]);
        for balance in list(year, "carry_forward") {
            carry.push(vec![
                Cell::Text(y.to_string()),
                Cell::Text(int(balance, "origin").to_string()),
                exact(balance, "amount"),
                Cell::Text(int(balance, "last_year").to_string()),
            ]);
        }
    }
    let mut blocks = vec![note(t("es.ahorro.lead"), true)];
    if !integration.is_empty() {
        blocks.push(table(
            "es_integracion_ahorro",
            String::new(),
            vec![
                col("tax_year", t("col.tax_year"), Kind::Text, 0.5),
                col("saldo", t("es.col.balance"), Kind::Fiat, 1.0),
                col("rcm", t("es.col.yields"), Kind::Fiat, 1.0),
                col("rcm_offset_25pct", t("es.col.offset"), Kind::Fiat, 1.0),
                col("prior_losses_applied", t("es.col.prior"), Kind::Fiat, 1.0),
                col("base", t("es.col.base_ahorro"), Kind::Fiat, 1.0),
            ],
            integration,
        ));
    }
    if !carry.is_empty() {
        blocks.push(table(
            "es_perdidas_pendientes",
            t("es.ahorro.carry").into(),
            vec![
                col("tax_year", t("es.col.after_year"), Kind::Text, 0.6),
                col("origin", t("es.col.origin"), Kind::Text, 0.6),
                col("amount", t("es.col.pending"), Kind::Fiat, 1.0),
                col("last_year", t("es.col.last_year"), Kind::Text, 0.7),
            ],
            carry,
        ));
    }
    blocks.push(note(t("es.ahorro.scale"), false));
    sections.push(Section {
        id: "es_integracion_ahorro",
        title: t("es.ahorro.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // Review flags: what a Spanish adviser should look at.
    let flags: Vec<Vec<Cell>> = list(data, "flags")
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
    let mut blocks = vec![note(t("es.flags.lead"), false)];
    if flags.is_empty() {
        blocks.push(note(t("es.flags.none"), false));
    } else {
        blocks.push(table(
            "es_revision",
            String::new(),
            vec![
                col("transaction_id", t("es.col.transaction"), Kind::Code, 1.2),
                col("code", t("es.col.code"), Kind::Code, 1.3),
                col("explanation", t("es.col.explanation"), Kind::Text, 3.0),
            ],
            flags,
        ));
    }
    sections.push(Section {
        id: "es_revision",
        title: t("es.flags.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });
    sections
}

pub(crate) fn method(_data: &Value, lang: Lang) -> Vec<Fact> {
    let t = |key: &str| lang.t(key);
    vec![
        fact(
            "long_term_rule",
            t("method.long_term_rule"),
            Cell::Text(t("es.long_term_rule").into()),
        ),
        fact(
            "pool_scope",
            t("method.pool_scope"),
            Cell::Text(t("es.pool").into()),
        ),
        fact(
            "calendar",
            t("es.method.calendar"),
            Cell::Text(t("es.method.calendar_value").into()),
        ),
        fact(
            "forms",
            t("es.method.forms"),
            Cell::Text(t("es.method.forms_value").into()),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        let disposal = |kind: &str, consideration: &str, gain: &str, computable: bool| {
            dict([
                ("transaction_id", s("t")),
                ("wallet", s("Exchange")),
                ("asset", s("BTC")),
                ("year", Value::Int(2025.into())),
                ("date", s("2025-11-12")),
                ("kind", s(kind)),
                ("contraprestacion", s(consideration)),
                ("quantity", s("0.22")),
                ("valor_transmision", s("13766.32")),
                ("valor_adquisicion", s("6676.36")),
                ("ganancia", s(gain)),
                ("computable", Value::Bool(computable)),
            ])
        };
        dict([
            ("country", s("es")),
            ("wallet_scoped", Value::Bool(false)),
            (
                "disposals",
                Value::List(vec![
                    disposal("sale", "moneda_curso_legal", "7089.96", true),
                    disposal("gift", "sin_contraprestacion", "-1000", false),
                ]),
            ),
            (
                "income",
                Value::List(vec![dict([
                    ("transaction_id", s("i")),
                    ("wallet", s("Lender")),
                    ("asset", s("BTC")),
                    ("date", s("2025-03-01")),
                    ("kind", s("lending_interest")),
                    ("quantity", s("0.002")),
                    ("value", s("120")),
                    ("casilla", s("rcm_art25_2")),
                    ("base", s("ahorro")),
                ])]),
            ),
            (
                "years",
                Value::List(vec![dict([
                    ("year", Value::Int(2025.into())),
                    ("ganancias", s("7089.96")),
                    ("perdidas", s("0")),
                    ("saldo", s("7089.96")),
                    ("non_computable_losses", s("1000")),
                    ("casilla_0027", s("0")),
                    ("rcm_art25_2", s("120")),
                    ("rcm", s("120")),
                    ("casilla_0304", s("0")),
                    ("rcm_offset_25pct", s("0")),
                    ("prior_losses_applied", s("0")),
                    ("base", s("7209.96")),
                    (
                        "carry_forward",
                        Value::List(vec![dict([
                            ("origin", Value::Int(2024.into())),
                            ("amount", s("10")),
                            ("last_year", Value::Int(2028.into())),
                        ])]),
                    ),
                    (
                        "by_coin",
                        Value::List(vec![dict([
                            ("asset", s("BTC")),
                            ("contraprestacion", s("moneda_curso_legal")),
                            ("valor_transmision", s("13766.32")),
                            ("valor_adquisicion", s("6676.36")),
                            ("saldo", s("7089.96")),
                        ])]),
                    ),
                ])]),
            ),
            (
                "flags",
                Value::List(vec![dict([
                    ("event", s("t")),
                    ("code", s("es_gift_loss_not_computable")),
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
    fn the_annex_rows_carry_their_consideration_in_spanish() {
        let sections = sections(&data(), Lang::Es);
        let rows = &table(&sections, "es_transmisiones").rows;
        let shown: Vec<String> = rows[1].iter().map(|cell| cell.display(Lang::Es)).collect();
        assert_eq!(shown[1], "12/11/2025");
        assert_eq!(shown[5], "Donación");
        assert_eq!(shown[6], "Sin contraprestación");
        assert_eq!(shown[9], "\u{2212}1.000,00");
        assert_eq!(shown[10], "No");
        let income = &table(&sections, "es_rendimientos").rows[0];
        assert_eq!(income[6].display(Lang::Es), "Art. 25.2 (por confirmar)");
        let carry = &table(&sections, "es_perdidas_pendientes").rows[0];
        assert_eq!(carry[3].csv(), "2028");
        let flags = &table(&sections, "es_revision").rows[0];
        assert!(flags[2].csv().starts_with("Pérdida de una donación"));
        let ids: Vec<&str> = sections.iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                "es_casilla_1804",
                "es_rendimientos",
                "es_integracion_ahorro",
                "es_revision"
            ]
        );
    }

    #[test]
    fn the_full_report_is_spanish_and_carries_the_notice() {
        let model = dict([
            (
                "profile",
                dict([("tax_country", s("es")), ("fiat_currency", s("EUR"))]),
            ),
            ("country_layer", data()),
        ]);
        let mut document = crate::report_design::full(&model);
        assert_eq!(document.lang, Lang::Es);
        assert_eq!(document.kicker, "Kassiber · España");
        let notice = kassiber_tax_countries::PREVIEW_NOTICE_ES;
        assert_eq!(document.preview_notice(), Some(notice));
        let ids: Vec<&str> = document.sections.iter().map(|s| s.id).collect();
        let income = ids.iter().position(|id| *id == "income").unwrap();
        assert_eq!(ids[income + 1], "es_casilla_1804");
        let facts = method(&data(), Lang::Es);
        assert!(facts.iter().any(|f| f.key == "forms"));
        document.generated_at = "2026-01-02T03:04:05Z".into();
        let stamp = kassiber_pdf::engine::Stamp::parse("2026-01-02T03:04:05Z").unwrap();
        let pdf = crate::report_design::pdf_data(&document, stamp);
        assert_eq!(value(value(&pdf, "meta"), "notice"), &s(notice));
        let files = crate::report_design::csv(&document).unwrap();
        let summary = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(summary.contains(notice));
        assert!(files
            .iter()
            .any(|(name, _)| name.contains("es_transmisiones")));
    }
}
