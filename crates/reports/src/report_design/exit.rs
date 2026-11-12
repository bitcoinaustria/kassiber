//! The exit-tax estimate (Wegzugsbesteuerung) in the shared design, from
//! the exit-tax report (`report_exit_tax`) and the export clock. Austrian
//! books read German, others English.
use super::*;
use crate::report_design::{
    col, fact, from_float, list, method_name, Block, Cell, Column, Document, Fact, Kind, Kpi, Lang,
    Section, Table,
};

/// Keys of the exit-tax notes Python writes in English; the English table
/// holds Python's text verbatim, so a note is found by its text.
const NOTES: [&str; 11] = [
    "exit.note.altbestand",
    "exit.note.rate",
    "exit.note.eu_eea",
    "exit.note.third_country",
    "exit.note.fmv_window",
    "exit.note.fmv_missing",
    "exit.note.tightening",
    "exit.note.income",
    "exit.note.non_reportable",
    "exit.note.non_austrian",
    "exit.note.review_gate",
];

fn translated(lang: Lang, note: &str) -> String {
    NOTES
        .iter()
        .find(|key| Lang::En.get(key) == Some(note))
        .and_then(|key| lang.get(key))
        .unwrap_or(note)
        .to_owned()
}
/// A float amount, `None` when Python left it unknown.
fn amount(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Fiat(from_float(raw)),
    }
}
/// Integer satoshis as BTC, with its msat companion.
fn sats(v: &Value, key: &str) -> [Cell; 2] {
    match value(v, key) {
        Value::Int(n) => {
            let btc = n
                .to_string()
                .parse::<Decimal>()
                .ok()
                .and_then(|d| d.div(&Decimal::from(100_000_000), &DECIMAL).ok());
            [
                btc.map(Cell::Btc).unwrap_or(Cell::Empty),
                Cell::Msat(n.clone() * 1000),
            ]
        }
        _ => [Cell::Empty, Cell::Empty],
    }
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

/// The exit-tax document. `model` holds `report` and `generated_at`.
pub fn exit_tax(model: &Value) -> Document {
    let report = value(model, "report");
    let country = text(report, "jurisdictionCode").to_lowercase();
    let lang = Lang::for_country(&country);
    let t = |key: &str| lang.t(key);
    let currency = text(report, "fiatCurrency");
    let currency_arg = [("fiat", currency.as_str())];
    let tf = |key: &str| lang.tf(key, &currency_arg);
    let totals = value(report, "totals");
    let status = value(report, "status");
    let departure = text(report, "departureDate");
    let input_version = value(report, "journalInputVersion");
    let book = format!(
        "{} / {}",
        text(report, "workspace"),
        text(report, "profile")
    );
    let word = |prefix: &str, raw: &str| {
        lang.get(&format!("{prefix}.{raw}"))
            .map(str::to_owned)
            .unwrap_or_else(|| raw.to_owned())
    };
    let mut sections = Vec::new();

    // 1. Scope
    sections.push(Section {
        id: "scope",
        title: t("scope.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![Block::Facts(vec![
            fact("book", t("scope.book"), Cell::Text(book.clone())),
            fact(
                "departure_date",
                t("exit.departure"),
                Cell::Date(departure.clone()),
            ),
            fact(
                "departure_timezone",
                t("exit.timezone"),
                Cell::Text(text(report, "departureTimezone")),
            ),
            Fact {
                display: Some(word("exit.destination", &text(report, "destination"))),
                ..fact(
                    "destination",
                    t("exit.destination"),
                    Cell::Text(text(report, "destination")),
                )
            },
            fact(
                "method",
                t("scope.method"),
                Cell::Text(method_name(lang, &text(report, "method").to_uppercase())),
            ),
            fact(
                "fiat_currency",
                t("scope.fiat_currency"),
                Cell::Text(currency.clone()),
            ),
            match input_version {
                Value::Int(n) => fact(
                    "journal_input_version",
                    t("scope.input_version"),
                    Cell::Count(i64::try_from(n.clone()).unwrap_or(0)),
                ),
                _ => Fact {
                    display: Some(t("exit.unavailable").into()),
                    ..fact(
                        "journal_input_version",
                        t("scope.input_version"),
                        Cell::Empty,
                    )
                },
            },
            Fact {
                display: Some(lang.date_time(&text(model, "generated_at"))),
                ..fact(
                    "generated_at",
                    t("scope.generated_at"),
                    Cell::Text(text(model, "generated_at")),
                )
            },
        ])],
    });

    // 2. The estimate; an incomplete estimate is a blocker
    let rate = match value(totals, "estimatedTaxRate") {
        Value::Null => Cell::Empty,
        raw => Cell::Percent(
            from_float(raw)
                .mul(&Decimal::from(100), &DECIMAL)
                .unwrap_or_else(|_| Decimal::zero()),
        ),
    };
    let kpi = |key: &'static str, label: String, value: Cell, detail: String| Kpi {
        key,
        label,
        value,
        detail,
        accent: false,
    };
    let timing = text(totals, "collectionTiming");
    let mut blocks = vec![Block::Kpis(vec![
        kpi(
            "taxable_gain",
            tf("exit.taxable_gain"),
            amount(totals, "taxableGain"),
            String::new(),
        ),
        kpi(
            "estimated_tax_rate",
            t("exit.rate").into(),
            rate,
            String::new(),
        ),
        kpi(
            "estimated_tax",
            tf("exit.estimated_tax"),
            amount(totals, "estimatedTax"),
            word("exit.collection", &timing),
        ),
    ])];
    let missing: Vec<String> = list(status, "missingFmvAssets")
        .iter()
        .map(py_str)
        .collect();
    if !missing.is_empty() {
        blocks.push(Block::Note {
            text: lang.tf("exit.incomplete", &[("assets", &missing.join(", "))]),
            accent: true,
        });
    }
    let quarantines = int(status, "quarantines");
    if quarantines > 0 {
        blocks.push(Block::Note {
            text: lang.tf(
                "exit.quarantines",
                &[("count", &lang.integer(quarantines.into()))],
            ),
            accent: true,
        });
    }
    sections.push(Section {
        id: "summary",
        title: t("exit.estimate").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 3. The deemed disposal at fair market value, per lot and in total
    let regime = |raw: &str| word("at.regime", raw);
    let mut lots = table(
        "deemed_disposal",
        String::new(),
        vec![
            col("asset", t("col.asset"), Kind::Text, 0.6),
            col("regime", t("at.col.regime"), Kind::Text, 0.9),
            col("quantity", t("col.quantity_btc"), Kind::Btc, 1.2),
            col("quantity_msat", t("col.quantity_msat"), Kind::Msat, 1.0),
            col("market_value", tf("col.market_value"), Kind::Fiat, 1.2),
            col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.2),
            col("gain", tf("col.gain"), Kind::Fiat, 1.1),
            col("taxable", t("exit.col.taxable"), Kind::Text, 0.8),
            col("kennzahl", t("col.kennzahl"), Kind::Text, 0.4),
        ],
        list(report, "lots")
            .iter()
            .map(|lot| {
                let [quantity, quantity_msat] = sats(lot, "quantitySats");
                vec![
                    Cell::Text(text(lot, "asset")),
                    Cell::Text(regime(&text(lot, "regime"))),
                    quantity,
                    quantity_msat,
                    amount(lot, "marketValue"),
                    amount(lot, "costBasis"),
                    amount(lot, "gain"),
                    Cell::Text(
                        t(if truthy(value(lot, "taxable")) {
                            "exit.yes"
                        } else {
                            "exit.no"
                        })
                        .into(),
                    ),
                    match value(lot, "kennzahl") {
                        Value::Null => Cell::Empty,
                        v => Cell::Text(py_str(v)),
                    },
                ]
            })
            .collect(),
    );
    lots.note = t("exit.lots_note").into();
    let [neu, neu_msat] = sats(totals, "neuQuantitySats");
    let [alt, alt_msat] = sats(totals, "altQuantitySats");
    let summary = table(
        "deemed_disposal_totals",
        t("exit.totals").into(),
        vec![
            col("regime", t("at.col.regime"), Kind::Text, 1.4),
            col("quantity", t("col.quantity_btc"), Kind::Btc, 1.2),
            col("quantity_msat", t("col.quantity_msat"), Kind::Msat, 1.0),
            col("market_value", tf("col.market_value"), Kind::Fiat, 1.2),
            col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.2),
            col("gain", tf("col.gain"), Kind::Fiat, 1.2),
        ],
        vec![
            vec![
                Cell::Text(t("exit.neu_taxable").into()),
                neu,
                neu_msat,
                amount(totals, "neuMarketValue"),
                amount(totals, "neuCostBasis"),
                amount(totals, "neuGain"),
            ],
            vec![
                Cell::Text(t("exit.alt_excluded").into()),
                alt,
                alt_msat,
                amount(totals, "altMarketValue"),
                Cell::Empty,
                Cell::Empty,
            ],
        ],
    );
    let mut blocks = vec![Block::Table(summary)];
    if !lots.rows.is_empty() {
        blocks.push(Block::Table(lots));
    }
    sections.push(Section {
        id: "deemed_disposal",
        title: t("exit.deemed_disposal").into(),
        lead: t("exit.deemed_disposal_lead").into(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 4. Fair-market-value quotes and holdings per wallet
    let quotes = table(
        "fmv_quotes",
        t("exit.quotes").into(),
        vec![
            col("asset", t("col.asset"), Kind::Text, 0.6),
            col("pair", t("exit.col.pair"), Kind::Text, 0.9),
            col("rate", tf("exit.col.rate"), Kind::Fiat, 1.2),
            col("source", t("exit.col.source"), Kind::Text, 1.0),
            col("as_of", t("exit.col.as_of"), Kind::Text, 1.4),
        ],
        list(report, "fmvSource")
            .iter()
            .map(|quote| {
                let as_of = value(quote, "asOf");
                vec![
                    Cell::Text(text(quote, "asset")),
                    Cell::Text(text(quote, "pair")),
                    amount(quote, "rate"),
                    Cell::Text(word("exit.source", &text(quote, "source"))),
                    if truthy(as_of) {
                        Cell::Text(lang.date_time(&py_str(as_of)))
                    } else {
                        Cell::Empty
                    },
                ]
            })
            .collect(),
    );
    let wallets = table(
        "wallet_holdings",
        t("holdings.by_wallet").into(),
        vec![
            col("wallet", t("col.wallet"), Kind::Text, 1.4),
            col("asset", t("col.asset"), Kind::Text, 0.6),
            col("quantity", t("col.quantity_btc"), Kind::Btc, 1.2),
            col("quantity_msat", t("col.quantity_msat"), Kind::Msat, 1.0),
            col("market_value", tf("col.market_value"), Kind::Fiat, 1.2),
        ],
        list(report, "walletHoldings")
            .iter()
            .map(|row| {
                let [quantity, quantity_msat] = sats(row, "quantitySats");
                vec![
                    Cell::Text(text(row, "wallet")),
                    Cell::Text(text(row, "asset")),
                    quantity,
                    quantity_msat,
                    amount(row, "marketValue"),
                ]
            })
            .collect(),
    );
    let mut blocks = vec![Block::Table(quotes)];
    if !wallets.rows.is_empty() {
        blocks.push(Block::Table(wallets));
    }
    sections.push(Section {
        id: "holdings",
        title: t("exit.valuation").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 5. Method: assumptions and the review gate
    let assumptions = table(
        "assumptions",
        String::new(),
        vec![col("message", t("at.col.message"), Kind::Text, 1.0)],
        list(report, "assumptions")
            .iter()
            .map(|note| vec![Cell::Text(translated(lang, &py_str(note)))])
            .collect(),
    );
    sections.push(Section {
        id: "method",
        title: t("method.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![
            Block::Table(assumptions),
            Block::Note {
                text: translated(lang, &text(report, "reviewGate")),
                accent: false,
            },
        ],
    });

    Document {
        kind: "exit_tax",
        title: t("exit.title").into(),
        kicker: lang.kicker(&country),
        book,
        period: lang.tf("exit.period", &[("date", &lang.date(&departure))]),
        period_start: departure.clone(),
        period_end: departure,
        generated_at: text(model, "generated_at"),
        input_version: match input_version {
            Value::Int(n) => i64::try_from(n.clone()).ok(),
            _ => None,
        },
        fiat: currency,
        country,
        lang,
        sections,
    }
}
