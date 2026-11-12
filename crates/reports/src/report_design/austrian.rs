//! The Austrian E 1kv report in the shared design, from the model Python
//! hands to its writer (`austrian_pdf::model`): the E 1kv report, the
//! current portfolio, the tax year's transactions and the report clock.
//! Today's numbered Austrian sections (1.1 … 4.5) become a `section` column
//! on the shared Disposals and Income tables, indexed on the Kennzahl page.
use super::*;
use crate::report_design::{
    btc, col, count, fact, fiat, from_float, list, method_name, msat, txt, Block, Cell, Column,
    Document, Fact, Kind, Kpi, Lang, Section, Table,
};

/// The FinanzOnline fields in the order and wording of Kassiber's Austrian
/// PDF; domestic-provider fields (`None`) are not modelled.
const FORM_FIELDS: [(&str, &str, bool); 12] = [
    ("801", "Einkünfte aus Spekulationsgeschäften (§ 31)", true),
    (
        "803",
        "Einkünfte aus Leistungen im Sinne des § 29 Z. 3 EStG",
        true,
    ),
    (
        "857",
        "Sonstige tarifsteuerpflichtige Einkünfte aus Kapitalvermögen",
        true,
    ),
    ("862", "Einkünfte aus der Überlassung von Kapital", true),
    ("995", "Einkünfte aus verbrieften Derivaten", true),
    ("896", "Verluste aus verbrieften Derivaten", true),
    ("172", "Ausländische laufende Einkünfte", true),
    (
        "174",
        "Ausländische Überschüsse aus realisierten Wertsteigerungen",
        true,
    ),
    ("176", "Ausländische Verluste", true),
    ("171", "Inländische laufende Einkünfte", false),
    (
        "173",
        "Inländische Überschüsse aus realisierten Wertsteigerungen",
        false,
    ),
    ("175", "Inländische Verluste", false),
];

/// The Austrian sections' titles, as Kassiber's Austrian workbook and PDF
/// print them.
const SECTION_TITLES: [(&str, &str); 13] = [
    ("1.1", "Steuerpflichtige Einkünfte aus dem An- und Verkauf von Kryptowährungen gem. § 27b Abs 3 EStG"),
    ("1.2", "Steuerpflichtige Einkünfte aus Margin, Derivaten und Futures"),
    ("1.3", "Steuerpflichtige Einkünfte aus Spekulationsgeschäften mit Kryptowährungen (§ 31 EStG)"),
    ("2.1", "Steuerpflichtige laufende Einkünfte aus der Überlassung von Kryptowährungen"),
    ("2.2", "Steuerpflichtige laufende Einkünfte aus Leistungen zur Transaktionsverarbeitung"),
    ("3.1", "Nicht steuerbare Einkünfte aus Altvermögen außerhalb der Spekulationsfrist"),
    ("3.2", "Nicht steuerbare Einkünfte gem. § 27b Abs 2 Z 2 Satz 2 EStG"),
    ("3.3", "Nicht steuerbare Steuergebühren und Rückerstattungen"),
    ("4.1", "Eingegangene Spenden/Trinkgeld"),
    ("4.2", "Ausgegangene Spenden/Schenkungen"),
    ("4.3", "Gestohlene, gehackte und verlorene Coins"),
    ("4.4", "Mining (kommerziell)"),
    ("4.5", "Minting"),
];

/// A wallet and asset's movements: transactions, inbound and outbound
/// counts; inbound, outbound and fee amounts.
type Flow = ((String, String), [i64; 3], [Decimal; 3]);

/// Integer euro cents as an exact amount.
fn cents(v: &Value) -> Cell {
    match v {
        Value::Int(n) => n
            .to_string()
            .parse::<Decimal>()
            .ok()
            .and_then(|d| d.div(&Decimal::from(100), &DECIMAL).ok())
            .map(Cell::Fiat)
            .unwrap_or(Cell::Empty),
        Value::Null => Cell::Fiat(Decimal::zero()),
        _ => Cell::Empty,
    }
}
fn cents_of(row: &Value, key: &str) -> Cell {
    cents(value(row, key))
}
fn sum_cents<'a>(rows: impl Iterator<Item = &'a Value>, key: &str) -> Cell {
    let mut total = kassiber_pyjson::BigInt::from(0);
    for row in rows {
        if let Value::Int(n) = value(row, key) {
            total += n.clone();
        }
    }
    cents(&Value::Int(total))
}
fn table(id: &'static str, title: String, columns: Vec<Column>, rows: Vec<Vec<Cell>>) -> Table {
    Table {
        id,
        title,
        columns,
        rows,
        total: None,
        note: String::new(),
    }
}
fn note(text: impl Into<String>) -> Block {
    Block::Note {
        text: text.into(),
        accent: false,
    }
}

/// The Austrian report document (German: the book's tax country is `at`).
pub fn austrian(model: &Value) -> Document {
    let report = value(model, "report");
    let profile = value(model, "profile");
    let country = text(profile, "tax_country");
    let lang = Lang::for_country(&country);
    let t = |key: &str| lang.t(key);
    let eur = [("fiat", "EUR")];
    let tf = |key: &str| lang.tf(key, &eur);
    let year = py_str(value(report, "tax_year"));
    let transactions = list(model, "transactions");
    let sections_data = value(report, "sections");
    let input_version = int(profile, "journal_input_version");
    let book = format!("{} / {}", text(model, "workspace"), text(profile, "label"));
    let section_title = |id: &str| {
        SECTION_TITLES
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, title)| title.to_string())
            .unwrap_or_else(|| text(value(sections_data, id), "label"))
    };
    let mut sections = Vec::new();

    // 1. Scope
    let dates: Vec<String> = {
        let mut dates: Vec<String> = transactions
            .iter()
            .filter(|r| truthy(value(r, "occurred_at")))
            .map(|r| text(r, "occurred_at"))
            .collect();
        dates.sort();
        dates
    };
    let wallets: std::collections::BTreeSet<String> = transactions
        .iter()
        .filter(|r| truthy(value(r, "wallet")))
        .map(|r| text(r, "wallet"))
        .collect();
    let first_last = |date: Option<&String>| match date {
        Some(d) => Cell::Date(d.chars().take(10).collect()),
        None => Cell::Empty,
    };
    let period = lang.tf(
        "doc.period",
        &[
            ("start", &lang.date(&format!("{year}-01-01"))),
            ("end", &lang.date(&format!("{year}-12-31"))),
        ],
    );
    let scope = vec![
        fact("book", t("scope.book"), Cell::Text(book.clone())),
        fact("tax_year", t("col.tax_year"), Cell::Text(year.clone())),
        fact("period", t("at.period"), Cell::Text(period.clone())),
        fact(
            "fiat_currency",
            t("scope.fiat_currency"),
            txt(profile, "fiat_currency"),
        ),
        fact(
            "method",
            t("scope.method"),
            Cell::Text(method_name(lang, &text(profile, "gains_algorithm"))),
        ),
        fact(
            "transactions",
            t("col.transactions"),
            Cell::Count(transactions.len() as i64),
        ),
        fact(
            "wallets",
            t("at.wallets_used"),
            Cell::Count(wallets.len() as i64),
        ),
        fact(
            "first_transaction",
            t("counts.first"),
            first_last(dates.first()),
        ),
        fact(
            "last_transaction",
            t("counts.last"),
            first_last(dates.last()),
        ),
        Fact {
            display: Some(lang.date_time(&text(model, "generated_at"))),
            ..fact(
                "generated_at",
                t("scope.generated_at"),
                txt(model, "generated_at"),
            )
        },
        fact(
            "journal_input_version",
            t("scope.input_version"),
            Cell::Count(input_version),
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

    // 2. Headline Kennzahlen
    let totals = value(report, "kennzahl_totals");
    let kz = |code: &str| cents_of(value(totals, code), "amount_eur_cents");
    let kpi = |key: &'static str, code: &str, label: &str| Kpi {
        key,
        label: label.into(),
        value: kz(code),
        detail: String::new(),
        accent: false,
    };
    sections.push(Section {
        id: "summary",
        title: t("summary.title.figures").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![Block::Kpis(vec![
            kpi("kz_172", "172", t("at.kpi.172")),
            kpi("kz_174", "174", t("at.kpi.174")),
            kpi("kz_176", "176", t("at.kpi.176")),
            kpi("kz_801", "801", t("at.kpi.801")),
        ])],
    });

    // 3. The Kennzahl page: FinanzOnline fields and the section index
    let form = table(
        "e1kv_kennzahlen",
        String::new(),
        vec![
            col("kennzahl", t("col.kennzahl"), Kind::Text, 0.5),
            col("field", t("col.field"), Kind::Text, 3.4),
            col("form", t("col.form"), Kind::Text, 0.6),
            col("amount_eur", t("col.amount_eur"), Kind::Fiat, 1.0),
        ],
        FORM_FIELDS
            .iter()
            .map(|(code, wording, modelled)| {
                vec![
                    Cell::Text(code.to_string()),
                    Cell::Text(wording.to_string()),
                    Cell::Text(
                        if matches!(*code, "801" | "803") {
                            "E 1"
                        } else {
                            "E 1kv"
                        }
                        .into(),
                    ),
                    if *modelled { kz(code) } else { Cell::Empty },
                ]
            })
            .collect(),
    );
    let order: Vec<String> = match value(report, "section_order") {
        Value::List(ids) => ids.iter().map(py_str).collect(),
        _ => SECTION_TITLES
            .iter()
            .map(|(id, _)| id.to_string())
            .collect(),
    };
    let index = table(
        "e1kv_sections",
        t("at.sections.title").into(),
        vec![
            col("section", t("col.section"), Kind::Text, 0.6),
            col("title", t("at.col.title"), Kind::Text, 3.2),
            col("kennzahlen", t("col.kennzahl"), Kind::Text, 0.7),
            col("status", t("at.col.status"), Kind::Text, 1.1),
            col("rows", t("col.rows"), Kind::Count, 0.5),
            col("amount_eur", t("col.amount_eur"), Kind::Fiat, 1.0),
        ],
        order
            .iter()
            .map(|id| {
                let section = value(sections_data, id);
                let codes = list(section, "kennzahlen")
                    .iter()
                    .map(py_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                let status = text(section, "status");
                vec![
                    Cell::Text(id.clone()),
                    Cell::Text(section_title(id)),
                    if codes.is_empty() {
                        Cell::Empty
                    } else {
                        Cell::Text(codes)
                    },
                    Cell::Text(
                        lang.get(&format!("at.status.{status}"))
                            .map(str::to_owned)
                            .unwrap_or(status),
                    ),
                    count(value(section, "totals"), "row_count"),
                    cents_of(value(section, "totals"), "amount_eur_cents"),
                ]
            })
            .collect(),
    );
    sections.push(Section {
        id: "country_at",
        title: t("at.kennzahlen.title").into(),
        lead: t("at.kennzahlen.year_lead").into(),
        page_break: false,
        landscape: false,
        blocks: vec![
            Block::Table(form),
            note(t("at.domestic_note")),
            Block::Table(index),
            note(t("at.sections.note")),
        ],
    });

    // 4./5. Disposals and income: every section's detail rows, in section
    // order, with the section they belong to
    let mut disposal_rows = Vec::new();
    let mut income_rows = Vec::new();
    for id in &order {
        for row in list(value(sections_data, id), "detail_rows") {
            if text(row, "entry_type") == "income" {
                income_rows.push((id.clone(), row));
            } else {
                disposal_rows.push((id.clone(), row));
            }
        }
    }
    let regime = |row: &Value| match text(row, "at_regime").as_str() {
        "" | "None" => Cell::Empty,
        other => Cell::Text(
            lang.get(&format!("at.regime.{other}"))
                .map(str::to_owned)
                .unwrap_or_else(|| other.to_owned()),
        ),
    };
    let code = |row: &Value| match value(row, "kennzahl") {
        Value::Null => Cell::Empty,
        v => Cell::Text(py_str(v)),
    };
    let quantity = |row: &Value| btc(row, "quantity");
    let mut disposals = table(
        "disposals",
        String::new(),
        vec![
            col("section", t("col.section"), Kind::Text, 0.55),
            col("date", t("col.date"), Kind::Date, 0.85),
            col("wallet", t("col.wallet"), Kind::Text, 0.9),
            col("asset", t("col.asset"), Kind::Text, 0.5),
            col("quantity", t("col.quantity"), Kind::Btc, 1.1),
            col(
                "quantity_msat",
                t("col.quantity_msat_plain"),
                Kind::Msat,
                1.0,
            ),
            col("regime", t("at.col.regime"), Kind::Text, 0.65),
            col("holding_days", t("at.col.holding_days"), Kind::Count, 0.7),
            col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.0),
            col("proceeds", tf("col.proceeds"), Kind::Fiat, 1.0),
            col("gain", tf("col.gain"), Kind::Fiat, 1.0),
            col("kennzahl", t("col.kennzahl"), Kind::Text, 0.4),
        ],
        disposal_rows
            .iter()
            .map(|(id, row)| {
                let q = quantity(row);
                vec![
                    Cell::Text(id.clone()),
                    Cell::Date(text(row, "date")),
                    txt(row, "wallet"),
                    txt(row, "asset"),
                    q.clone(),
                    msat(&q),
                    regime(row),
                    count(row, "holding_period_days"),
                    cents_of(row, "cost_basis_eur_cents"),
                    cents_of(row, "proceeds_eur_cents"),
                    cents_of(row, "gain_loss_eur_cents"),
                    code(row),
                ]
            })
            .collect(),
    );
    if !disposals.rows.is_empty() {
        let rows = || disposal_rows.iter().map(|(_, r)| *r);
        let blank = || Cell::Text(String::new());
        disposals.total = Some(vec![
            Cell::Text(t("doc.total").into()),
            blank(),
            blank(),
            blank(),
            blank(),
            blank(),
            blank(),
            blank(),
            sum_cents(rows(), "cost_basis_eur_cents"),
            sum_cents(rows(), "proceeds_eur_cents"),
            sum_cents(rows(), "gain_loss_eur_cents"),
            blank(),
        ]);
    }
    sections.push(Section {
        id: "disposals",
        title: t("disposals.title").into(),
        lead: t("at.disposals.lead").into(),
        page_break: false,
        landscape: false,
        blocks: vec![if disposals.rows.is_empty() {
            note(t("at.disposals.none"))
        } else {
            Block::Table(disposals)
        }],
    });
    let mut income = table(
        "income",
        String::new(),
        vec![
            col("section", t("col.section"), Kind::Text, 0.55),
            col("date", t("col.date"), Kind::Date, 0.85),
            col("wallet", t("col.wallet"), Kind::Text, 1.0),
            col("asset", t("col.asset"), Kind::Text, 0.5),
            col("quantity", t("col.quantity"), Kind::Btc, 1.1),
            col(
                "quantity_msat",
                t("col.quantity_msat_plain"),
                Kind::Msat,
                1.0,
            ),
            col("kind", t("at.col.kind"), Kind::Text, 1.0),
            col("income", t("at.col.income"), Kind::Fiat, 1.1),
            col("kennzahl", t("col.kennzahl"), Kind::Text, 0.4),
        ],
        income_rows
            .iter()
            .map(|(id, row)| {
                let q = quantity(row);
                vec![
                    Cell::Text(id.clone()),
                    Cell::Date(text(row, "date")),
                    txt(row, "wallet"),
                    txt(row, "asset"),
                    q.clone(),
                    msat(&q),
                    txt(row, "kind"),
                    cents_of(row, "income_eur_cents"),
                    code(row),
                ]
            })
            .collect(),
    );
    if !income.rows.is_empty() {
        let blank = || Cell::Text(String::new());
        income.total = Some(vec![
            Cell::Text(t("doc.total").into()),
            blank(),
            blank(),
            blank(),
            blank(),
            blank(),
            blank(),
            sum_cents(income_rows.iter().map(|(_, r)| *r), "income_eur_cents"),
            blank(),
        ]);
    }
    sections.push(Section {
        id: "income",
        title: t("at.income.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![if income.rows.is_empty() {
            note(t("at.income.none"))
        } else {
            Block::Table(income)
        }],
    });

    // 6. Holdings now and the tax year's movements per wallet and asset
    let portfolio = table(
        "holdings_by_wallet",
        t("holdings.by_wallet").into(),
        vec![
            col("wallet", t("col.wallet"), Kind::Text, 1.1),
            col("account", t("col.bucket"), Kind::Text, 0.8),
            col("asset", t("col.asset"), Kind::Text, 0.5),
            col("quantity", t("col.quantity"), Kind::Btc, 1.2),
            col(
                "quantity_msat",
                t("col.quantity_msat_plain"),
                Kind::Msat,
                1.0,
            ),
            col("avg_cost", tf("col.avg_cost"), Kind::Fiat, 1.1),
            col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.1),
            col("market_value", tf("col.market_value"), Kind::Fiat, 1.1),
            col("unrealized", tf("col.unrealized"), Kind::Fiat, 1.1),
        ],
        list(model, "portfolio")
            .iter()
            .map(|row| {
                let q = btc(row, "quantity");
                vec![
                    txt(row, "wallet"),
                    txt(row, "account"),
                    txt(row, "asset"),
                    q.clone(),
                    msat(&q),
                    fiat(row, "avg_cost"),
                    fiat(row, "cost_basis"),
                    fiat(row, "market_value"),
                    fiat(row, "unrealized_pnl"),
                ]
            })
            .collect(),
    );
    let mut flows: Vec<Flow> = Vec::new();
    for row in transactions {
        let key = (text(row, "wallet"), text(row, "asset"));
        let index = match flows.iter().position(|(k, _, _)| *k == key) {
            Some(i) => i,
            None => {
                flows.push((
                    key,
                    [0; 3],
                    [Decimal::zero(), Decimal::zero(), Decimal::zero()],
                ));
                flows.len() - 1
            }
        };
        let (_, counts, amounts) = &mut flows[index];
        counts[0] += 1;
        let amount = from_float(value(row, "amount"));
        match text(row, "direction").as_str() {
            "inbound" => {
                counts[1] += 1;
                amounts[0] = amounts[0]
                    .add(&amount, &DECIMAL)
                    .unwrap_or_else(|_| amounts[0].clone());
            }
            "outbound" => {
                counts[2] += 1;
                amounts[1] = amounts[1]
                    .add(&amount, &DECIMAL)
                    .unwrap_or_else(|_| amounts[1].clone());
            }
            _ => (),
        }
        let fee = from_float(value(row, "fee"));
        amounts[2] = amounts[2]
            .add(&fee, &DECIMAL)
            .unwrap_or_else(|_| amounts[2].clone());
    }
    flows.sort_by(|a, b| a.0.cmp(&b.0));
    let movements = table(
        "movements",
        t("at.movements.title").into(),
        vec![
            col("wallet", t("col.wallet"), Kind::Text, 1.1),
            col("asset", t("col.asset"), Kind::Text, 0.5),
            col("transactions", t("col.transactions"), Kind::Count, 0.9),
            col("inbound_count", t("col.inbound_count"), Kind::Count, 0.7),
            col("outbound_count", t("col.outbound_count"), Kind::Count, 0.7),
            col("inbound_amount", t("col.inbound_amount"), Kind::Btc, 1.2),
            col("outbound_amount", t("col.outbound_amount"), Kind::Btc, 1.2),
            col("fee_amount", t("col.fee_amount"), Kind::Btc, 1.1).sparse(),
        ],
        flows
            .into_iter()
            .map(|((wallet, asset), counts, amounts)| {
                let [inbound, outbound, fees] = amounts;
                vec![
                    Cell::Text(wallet),
                    Cell::Text(asset),
                    Cell::Count(counts[0]),
                    Cell::Count(counts[1]),
                    Cell::Count(counts[2]),
                    Cell::Btc(inbound),
                    Cell::Btc(outbound),
                    Cell::Btc(fees),
                ]
            })
            .collect(),
    );
    let mut blocks = vec![if portfolio.rows.is_empty() {
        note(t("holdings.none"))
    } else {
        Block::Table(portfolio)
    }];
    blocks.push(if movements.rows.is_empty() {
        note(t("at.movements.none"))
    } else {
        Block::Table(movements)
    });
    sections.push(Section {
        id: "holdings",
        title: t("holdings.title").into(),
        lead: t("at.holdings.lead").into(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 7. Open issues: quarantines, Kennzahl mismatches, unmodelled sections
    let quality = value(report, "data_quality");
    let quarantines = list(quality, "quarantines");
    let mismatches = list(quality, "kennzahl_mismatches");
    let quarantined: i64 = quarantines.iter().map(|r| int(r, "count")).sum();
    let mut blocks = vec![Block::Facts(vec![
        Fact {
            accent: quarantined > 0,
            ..fact(
                "quarantines",
                t("issues.quarantined"),
                Cell::Count(quarantined),
            )
        },
        Fact {
            accent: !mismatches.is_empty(),
            ..fact(
                "kennzahl_mismatches",
                t("at.issues.mismatches"),
                Cell::Count(mismatches.len() as i64),
            )
        },
    ])];
    if !quarantines.is_empty() {
        blocks.push(Block::Table(Table {
            note: t("at.issues.quarantine_note").into(),
            ..table(
                "quarantines",
                t("issues.reasons").into(),
                vec![
                    col("reason", t("col.reason"), Kind::Code, 3.0),
                    col("transactions", t("col.transactions"), Kind::Count, 1.0),
                ],
                quarantines
                    .iter()
                    .map(|r| vec![txt(r, "reason"), count(r, "count")])
                    .collect(),
            )
        }));
    }
    if !mismatches.is_empty() {
        blocks.push(Block::Table(Table {
            note: t("at.issues.mismatch_note").into(),
            ..table(
                "kennzahl_mismatches",
                t("at.issues.mismatches").into(),
                vec![
                    col("transaction", t("at.col.reference"), Kind::Code, 2.4),
                    col(
                        "stored_kennzahl",
                        t("at.col.stored_kennzahl"),
                        Kind::Text,
                        1.0,
                    ),
                    col(
                        "export_kennzahl",
                        t("at.col.export_kennzahl"),
                        Kind::Text,
                        1.0,
                    ),
                ],
                mismatches
                    .iter()
                    .map(|r| {
                        vec![
                            txt(r, "tx_id"),
                            txt(r, "stored_kennzahl"),
                            txt(r, "export_kennzahl"),
                        ]
                    })
                    .collect(),
            )
        }));
    }
    blocks.push(note(t("at.issues.not_modelled")));
    sections.push(Section {
        id: "issues",
        title: t("issues.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 8. Method and provenance: the report's assumptions and review gate
    let assumptions = table(
        "assumptions",
        t("at.assumptions.title").into(),
        vec![
            col("code", t("at.col.code"), Kind::Code, 1.3),
            col("message", t("at.col.message"), Kind::Text, 4.0),
        ],
        list(report, "assumptions")
            .iter()
            .map(|row| {
                let code = text(row, "code");
                let message = lang
                    .get(&format!("at.assumption.{code}"))
                    .map(|template| {
                        template.replace("{scope}", &text(report, "cost_basis_pool_scope"))
                    })
                    .unwrap_or_else(|| text(row, "message"));
                vec![Cell::Text(code), Cell::Text(message)]
            })
            .collect(),
    );
    sections.push(Section {
        id: "method",
        title: t("method.title").into(),
        lead: t("at.method.lead").into(),
        page_break: false,
        landscape: false,
        blocks: vec![
            Block::Facts(vec![
                fact(
                    "method",
                    t("method.method"),
                    Cell::Text(method_name(lang, &text(profile, "gains_algorithm"))),
                ),
                fact(
                    "engine",
                    t("method.engine"),
                    Cell::Text(t("method.engine_value").into()),
                ),
                fact(
                    "regime",
                    t("method.regimes"),
                    Cell::Text(t("method.regimes_value").into()),
                ),
                fact("form", t("col.form"), txt(report, "form")),
            ]),
            Block::Table(assumptions),
            note(t("method.review_gate")),
        ],
    });

    // 9. Questions an adviser asks first
    sections.push(Section {
        id: "faq",
        title: t("at.faq.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![Block::Facts(
            ["data", "zero", "file", "domestic"]
                .iter()
                .map(|key| {
                    let answer = t(&format!("at.faq.{key}.a"));
                    Fact {
                        display: Some(answer.into()),
                        ..fact(
                            "faq",
                            t(&format!("at.faq.{key}.q")),
                            Cell::Text(answer.into()),
                        )
                    }
                })
                .collect(),
        )],
    });

    // 10. Every transaction of the tax year with its journal amounts
    let mut by_tx: Vec<(
        String,
        [kassiber_pyjson::BigInt; 3],
        std::collections::BTreeSet<String>,
    )> = Vec::new();
    for row in list(report, "rows") {
        let id = text(row, "transaction_id");
        let index = match by_tx.iter().position(|(k, _, _)| *k == id) {
            Some(i) => i,
            None => {
                by_tx.push((id, [0.into(), 0.into(), 0.into()], Default::default()));
                by_tx.len() - 1
            }
        };
        let (_, sums, codes) = &mut by_tx[index];
        for (i, key) in [
            "cost_basis_eur_cents",
            "proceeds_eur_cents",
            "gain_loss_eur_cents",
        ]
        .iter()
        .enumerate()
        {
            if let Value::Int(n) = value(row, key) {
                sums[i] += n.clone();
            }
        }
        if !matches!(value(row, "kennzahl"), Value::Null) {
            codes.insert(text(row, "kennzahl"));
        }
    }
    let ledger = table(
        "transactions",
        String::new(),
        vec![
            col("date", t("col.date"), Kind::Date, 0.85),
            col("wallet", t("col.wallet"), Kind::Text, 0.8),
            col("reference", t("at.col.reference"), Kind::Code, 1.9),
            col("label", t("at.col.label"), Kind::Text, 1.4),
            col("asset", t("col.asset"), Kind::Text, 0.45),
            col("inbound", t("direction.inbound"), Kind::Btc, 1.0),
            col(
                "inbound_msat",
                t("col.inbound_amount_msat"),
                Kind::Msat,
                1.0,
            ),
            col("outbound", t("direction.outbound"), Kind::Btc, 1.0),
            col(
                "outbound_msat",
                t("col.outbound_amount_msat"),
                Kind::Msat,
                1.0,
            ),
            col("fee", t("col.fee"), Kind::Btc, 0.9).sparse(),
            col("fee_msat", t("col.fee_msat"), Kind::Msat, 1.0),
            col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 0.9).sparse(),
            col("proceeds", tf("col.proceeds"), Kind::Fiat, 0.9).sparse(),
            col("gain", tf("col.gain"), Kind::Fiat, 0.9).sparse(),
            col("kennzahl", t("col.kennzahl"), Kind::Text, 0.4),
        ],
        transactions
            .iter()
            .map(|row| {
                let id = text(row, "transaction_id");
                let tax = by_tx.iter().find(|(k, _, _)| *k == id);
                let sum = |i: usize| match tax {
                    Some((_, sums, _)) => cents(&Value::Int(sums[i].clone())),
                    None => Cell::Fiat(Decimal::zero()),
                };
                let direction = text(row, "direction");
                let amount = || {
                    let q = btc(row, "amount");
                    [q.clone(), msat(&q)]
                };
                let none = || [Cell::Empty, Cell::Empty];
                let [inbound, inbound_msat] = if direction == "inbound" {
                    amount()
                } else {
                    none()
                };
                let [outbound, outbound_msat] = if direction == "outbound" {
                    amount()
                } else {
                    none()
                };
                let fee = btc(row, "fee");
                let reference = match text(row, "external_id") {
                    e if e.is_empty() => format!("#{id}"),
                    e => e,
                };
                let first = match text(row, "kind") {
                    k if k.is_empty() => text(row, "description"),
                    k => k,
                };
                let tags = text(row, "tags");
                let label = [
                    first,
                    text(row, "note"),
                    if tags.is_empty() {
                        String::new()
                    } else {
                        lang.tf("at.tags", &[("tags", &tags)])
                    },
                ]
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" | ");
                vec![
                    Cell::Date(text(row, "occurred_at").chars().take(10).collect()),
                    txt(row, "wallet"),
                    Cell::Text(reference),
                    Cell::Text(label),
                    txt(row, "asset"),
                    inbound,
                    inbound_msat,
                    outbound,
                    outbound_msat,
                    fee.clone(),
                    msat(&fee),
                    sum(0),
                    sum(1),
                    sum(2),
                    match tax {
                        Some((_, _, codes)) if !codes.is_empty() => {
                            Cell::Text(codes.iter().cloned().collect::<Vec<_>>().join(", "))
                        }
                        _ => Cell::Empty,
                    },
                ]
            })
            .collect(),
    );
    sections.push(Section {
        id: "transactions",
        title: t("transactions.title").into(),
        lead: t("at.transactions.lead").into(),
        page_break: false,
        landscape: !ledger.rows.is_empty(),
        blocks: vec![if ledger.rows.is_empty() {
            note(t("at.transactions.none"))
        } else {
            Block::Table(ledger)
        }],
    });

    Document {
        kind: "austrian_e1kv",
        title: lang.tf("at.title", &[("year", &year)]),
        kicker: t("doc.kicker.at").into(),
        book,
        period,
        period_start: format!("{year}-01-01"),
        period_end: format!("{year}-12-31"),
        generated_at: text(model, "generated_at"),
        input_version: Some(input_version),
        fiat: "EUR".into(),
        country,
        lang,
        sections,
    }
}
