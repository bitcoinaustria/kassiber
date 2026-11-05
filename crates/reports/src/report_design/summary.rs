//! The summary report in the shared design, from the summary model
//! (`build_summary_pdf_report_data`), the book's tax settings and, for
//! Austria, the E 1kv Kennzahl totals of each tax year in the period.
use super::*;
use crate::report_design::{
    balance_chart, btc, col, count, day, fact, fiat, float, from_float, list, method_name,
    month_labels, msat, status_name, txt, Axis, Block, Cell, Chart, Document, Fact, Kind, Kpi,
    Lang, Legend, Section, Series, Table,
};

/// The E 1kv form wording of each Kennzahl the report fills, as Kassiber's
/// Austrian PDF prints it for FinanzOnline.
const KENNZAHLEN: [(&str, &str); 4] = [
    ("172", "Ausländische laufende Einkünfte"),
    (
        "174",
        "Ausländische Überschüsse aus realisierten Wertsteigerungen",
    ),
    ("176", "Ausländische Verluste"),
    ("801", "Einkünfte aus Spekulationsgeschäften (§ 31)"),
];

fn flow_chart(lang: Lang, flows: &[Value], currency: &str) -> Option<Chart> {
    if flows.is_empty() {
        return None;
    }
    let inflow: Vec<f64> = flows.iter().map(|r| float(r, "inflow_volume")).collect();
    let outflow: Vec<f64> = flows.iter().map(|r| -float(r, "outflow_volume")).collect();
    let axis = Axis::new(&[inflow.clone(), outflow.clone()].concat(), 4, lang, true);
    let total = flows.len() as f64;
    let zero = axis.at(0.0);
    let bars = |values: &[f64]| {
        values
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0.0)
            .map(|(i, v)| {
                (
                    i as f64 / total + 0.15 / total,
                    (i as f64 + 0.85) / total,
                    zero,
                    axis.at(*v),
                )
            })
            .collect()
    };
    let months: Vec<String> = flows.iter().map(|r| text(r, "period")).collect();
    let fiat = [("fiat", currency)];
    Some(Chart {
        title: lang.t("activity.flow_chart").into(),
        height: 90.0,
        y2_labels: vec![],
        x_labels: month_labels(lang, &months),
        series: vec![
            Series::Bars {
                bars: bars(&inflow),
                hollow: false,
                tone: "ink",
            },
            Series::Bars {
                bars: bars(&outflow),
                hollow: true,
                tone: "ink",
            },
        ],
        legend: vec![
            Legend {
                label: lang.tf("legend.inflow", &fiat),
                mark: "filled",
                tone: "ink",
            },
            Legend {
                label: lang.tf("legend.outflow", &fiat),
                mark: "hollow",
                tone: "ink",
            },
        ],
        y_labels: axis.ticks,
    })
}

/// The summary report document. `model` holds `summary` (Python's summary
/// model), `profile` (tax settings) and `kennzahlen` (Austria only). The
/// book's tax country picks the language.
pub fn summary(model: &Value) -> Document {
    let report = value(model, "summary");
    let profile = value(model, "profile");
    let currency = text(report, "fiat_currency");
    let country = text(profile, "tax_country");
    let lang = Lang::for_country(&country);
    let t = |key: &str| lang.t(key);
    let currency_arg = [("fiat", currency.as_str())];
    let tf = |key: &str| lang.tf(key, &currency_arg);
    let austria = country == "at";
    let timeframe = value(report, "timeframe");
    let integrity = value(report, "data_integrity");
    let journals = value(integrity, "journals");
    let metrics = value(report, "metrics");
    let input_version = int(journals, "journal_input_version");
    let period_end = lang.date(&text(timeframe, "end"));
    let mut sections = Vec::new();

    // 1. Scope
    let wallets: Vec<String> = list(report, "wallets")
        .iter()
        .map(|w| text(w, "label"))
        .collect();
    let status = text(journals, "status");
    let scope = vec![
        fact(
            "book",
            t("scope.book"),
            Cell::Text(format!(
                "{} / {}",
                text(report, "workspace"),
                text(report, "profile")
            )),
        ),
        fact(
            "period_start",
            t("scope.period_start"),
            day(timeframe, "start"),
        ),
        fact("period_end", t("scope.period_end"), day(timeframe, "end")),
        Fact {
            display: Some(wallets.join(", ")),
            ..fact(
                "wallets",
                t("scope.wallets"),
                Cell::Count(wallets.len() as i64),
            )
        },
        fact(
            "fiat_currency",
            t("scope.fiat_currency"),
            Cell::Text(currency.clone()),
        ),
        Fact {
            display: Some(
                t(if austria {
                    "country.at"
                } else {
                    "country.generic"
                })
                .into(),
            ),
            ..fact(
                "tax_country",
                t("scope.tax_country"),
                Cell::Text(country.clone()),
            )
        },
        fact(
            "method",
            t("scope.method"),
            Cell::Text(method_name(lang, &text(profile, "gains_algorithm"))),
        ),
        Fact {
            display: Some(lang.date_time(&text(report, "generated_at"))),
            ..fact(
                "generated_at",
                t("scope.generated_at"),
                Cell::Text(text(report, "generated_at")),
            )
        },
        fact(
            "journal_input_version",
            t("scope.input_version"),
            Cell::Count(input_version),
        ),
        Fact {
            display: Some(status_name(lang, &status)),
            accent: status != "current",
            ..fact("journals", t("scope.journals"), Cell::Text(status.clone()))
        },
    ];
    sections.push(Section {
        id: "scope",
        title: t("scope.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![Block::Facts(scope)],
    });

    // 2. Summary
    let start = from_float(value(metrics, "period_start_value"));
    let end = from_float(value(metrics, "period_end_value"));
    let change = if start > Decimal::zero() {
        let pct = end
            .sub(&start, &DECIMAL)
            .and_then(|d| d.div(&start, &DECIMAL))
            .and_then(|d| d.mul(&Decimal::from(100), &DECIMAL))
            .ok();
        pct.map(|p| lang.tf("kpi.vs_start", &[("change", &lang.percent(&p))]))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let benchmark = value(report, "benchmark");
    let spot = match value(benchmark, "change_pct") {
        Value::Null => String::new(),
        raw => lang.tf(
            "kpi.btc_spot",
            &[("change", &lang.percent(&from_float(raw)))],
        ),
    };
    let age = value(report, "holding_age");
    let age_detail = match value(age, "weighted_days") {
        Value::Null => String::new(),
        raw => lang.tf(
            "kpi.weighted_age",
            &[("days", &lang.number(&from_float(raw), 0))],
        ),
    };
    let kpi = |key: &'static str, label: String, value: Cell, detail: String| Kpi {
        key,
        label,
        value,
        detail,
        accent: false,
    };
    let join = |parts: Vec<String>| {
        parts
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let kpis = vec![
        kpi(
            "start_value",
            tf("kpi.start_value"),
            fiat(metrics, "period_start_value"),
            String::new(),
        ),
        kpi(
            "end_value",
            tf("kpi.end_value"),
            fiat(metrics, "period_end_value"),
            join(vec![change, spot]),
        ),
        kpi(
            "net_flow",
            tf("kpi.net_flow"),
            fiat(metrics, "net_flow"),
            String::new(),
        ),
        kpi(
            "realized_gain",
            tf("kpi.realized_gain"),
            fiat(metrics, "realized_pnl"),
            String::new(),
        ),
        kpi(
            "unrealized_gain",
            tf("kpi.unrealized_gain"),
            fiat(metrics, "unrealized_pnl"),
            String::new(),
        ),
        kpi(
            "cost_basis_end",
            tf("kpi.cost_basis_end"),
            fiat(metrics, "end_cost_basis"),
            String::new(),
        ),
        kpi(
            "quantity_end",
            t("kpi.quantity_end").into(),
            btc(metrics, "btc_stack_end"),
            join(vec![
                lang.tf(
                    "kpi.quantity_start",
                    &[("quantity", &btc(metrics, "btc_stack_start").display(lang))],
                ),
                age_detail,
            ]),
        ),
        kpi(
            "fees",
            tf("kpi.fees"),
            fiat(metrics, "fees_fiat"),
            lang.tf(
                "kpi.fees_btc",
                &[("quantity", &btc(metrics, "fees_btc").display(lang))],
            ),
        ),
    ];
    sections.push(Section {
        id: "summary",
        title: t("summary.title.figures").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![Block::Kpis(kpis)],
    });

    // 3. Holdings at period end (and now, when the snapshot was asked for)
    let holdings_table =
        |id: &'static str, title: String, rows: &[Value], totals: &Value| -> Table {
            let columns = vec![
                col("wallet", t("col.wallet"), Kind::Text, 1.4),
                col("scope", t("col.scope"), Kind::Text, 1.2),
                col("assets", t("col.assets"), Kind::Text, 0.8),
                col("quantity_btc", t("col.quantity_btc"), Kind::Btc, 1.3),
                col("quantity_msat", t("col.quantity_msat"), Kind::Msat, 1.0),
                col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.2),
                col("market_value", tf("col.market_value"), Kind::Fiat, 1.2),
                col("transactions", t("col.transactions"), Kind::Count, 0.9),
            ];
            let rows = rows
                .iter()
                .map(|row| {
                    let quantity = btc(row, "quantity");
                    vec![
                        txt(row, "wallet"),
                        txt(row, "scope"),
                        Cell::Text(
                            list(row, "assets")
                                .iter()
                                .map(py_str)
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                        quantity.clone(),
                        msat(&quantity),
                        fiat(row, "cost_basis"),
                        fiat(row, "market_value"),
                        count(row, "tx_count"),
                    ]
                })
                .collect::<Vec<_>>();
            let total_quantity = btc(totals, "total_quantity");
            let blank = || Cell::Text(String::new());
            let total = vec![
                Cell::Text(t("doc.total").into()),
                blank(),
                blank(),
                total_quantity.clone(),
                msat(&total_quantity),
                blank(),
                fiat(totals, "total_market_value"),
                blank(),
            ];
            Table {
                id,
                title,
                columns,
                rows,
                total: Some(total),
                note: String::new(),
            }
        };
    let mut holdings = vec![Block::Table(holdings_table(
        "holdings_period_end",
        lang.tf("holdings.period_end", &[("date", &period_end)]),
        list(report, "wallet_holdings"),
        value(report, "holdings_totals"),
    ))];
    let snapshot = value(report, "snapshot");
    if truthy(snapshot) {
        let mut table = holdings_table(
            "holdings_now",
            lang.tf(
                "holdings.now",
                &[("at", &lang.date_time(&text(snapshot, "as_of")))],
            ),
            list(snapshot, "wallets"),
            snapshot,
        );
        table.note = t("holdings.now_note").into();
        holdings.push(Block::Table(table));
    }
    let holdings = Section {
        id: "holdings",
        title: t("holdings.title").into(),
        lead: t("holdings.lead").into(),
        page_break: false,
        landscape: false,
        blocks: holdings,
    };

    // 4. Activity: charts and the month table. It comes before the holdings
    // so it starts on page 1 (charts never split across pages and would
    // otherwise leave the rest of the page empty).
    let history = list(report, "balance_history");
    let flows = list(report, "flow_periods");
    let realized = list(report, "realized_pnl_periods");
    let mut blocks = Vec::new();
    if !history.is_empty() {
        let months: Vec<String> = history.iter().map(|r| text(r, "period")).collect();
        let series = |key: &str| history.iter().map(|r| float(r, key)).collect::<Vec<_>>();
        blocks.push(Block::Chart(balance_chart(
            lang,
            t("activity.balance_chart"),
            &months,
            &series("market_value"),
            &series("cumulative_cost_basis"),
            Some(&series("quantity")),
            &currency,
        )));
    }
    if let Some(chart) = flow_chart(lang, flows, &currency) {
        blocks.push(Block::Chart(chart));
    }
    let by_month =
        |rows: &[Value], period: &str| rows.iter().find(|r| text(r, "period") == period).cloned();
    let months = vec![
        col("month", t("col.month"), Kind::Date, 1.45),
        col("inflow", tf("col.inflow"), Kind::Fiat, 1.1).sparse(),
        col("outflow", tf("col.outflow"), Kind::Fiat, 1.1).sparse(),
        col("realized_gain", tf("col.realized_gain"), Kind::Fiat, 1.1).sparse(),
        col("disposals", t("col.disposals"), Kind::Count, 0.9).sparse(),
        col("fees", tf("col.fees"), Kind::Fiat, 0.85).sparse(),
        col("fees_btc", t("col.fees_btc"), Kind::Btc, 1.0).sparse(),
        col("fees_msat", t("col.fees_msat"), Kind::Msat, 1.0),
        col("quantity_btc", t("col.quantity_end_btc"), Kind::Btc, 1.2),
        col("quantity_msat", t("col.quantity_end_msat"), Kind::Msat, 1.0),
        col("market_value", tf("col.value_end"), Kind::Fiat, 1.1),
    ];
    let mut rows = Vec::new();
    for flow in flows {
        let period = text(flow, "period");
        let gain = by_month(realized, &period).unwrap_or(Value::Null);
        let balance = by_month(history, &period).unwrap_or(Value::Null);
        let fees = btc(flow, "fees_btc");
        let quantity = btc(&balance, "quantity");
        rows.push(vec![
            Cell::Date(period),
            fiat(flow, "inflow_volume"),
            fiat(flow, "outflow_volume"),
            fiat(&gain, "realized_pnl"),
            count(&gain, "count"),
            fiat(flow, "fees_fiat"),
            fees.clone(),
            msat(&fees),
            quantity.clone(),
            msat(&quantity),
            fiat(&balance, "market_value"),
        ]);
    }
    let partial = history
        .last()
        .is_some_and(|r| truthy(value(r, "period_partial")));
    blocks.push(Block::Table(Table {
        id: "monthly_activity",
        title: t("activity.by_month").into(),
        columns: months,
        rows,
        total: None,
        note: if partial {
            lang.tf("activity.partial", &[("date", &period_end)])
        } else {
            String::new()
        },
    }));
    sections.push(Section {
        id: "activity",
        title: t("activity.title").into(),
        lead: t("activity.lead").into(),
        page_break: false,
        landscape: false,
        blocks,
    });
    sections.push(holdings);

    // 5. Largest disposals and movements
    let disposals = list(report, "top_disposals");
    let movements = list(report, "top_movements");
    let largest = vec![
        Block::Table(Table {
            id: "largest_disposals",
            title: t("largest.disposals").into(),
            columns: vec![
                col("date", t("col.date"), Kind::Date, 0.9),
                col("wallet", t("col.wallet"), Kind::Text, 1.1),
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col("quantity_btc", t("col.quantity_btc"), Kind::Btc, 1.2),
                col("quantity_msat", t("col.quantity_msat"), Kind::Msat, 1.0),
                col("proceeds", tf("col.proceeds"), Kind::Fiat, 1.2),
                col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.2),
                col("gain", tf("col.gain"), Kind::Fiat, 1.1),
            ],
            rows: disposals
                .iter()
                .map(|row| {
                    // The model keeps the journal's signed disposal quantity.
                    let quantity = match btc(row, "quantity") {
                        Cell::Btc(d) => Cell::Btc(d.neg(&DECIMAL).unwrap_or(d)),
                        other => other,
                    };
                    vec![
                        day(row, "occurred_at"),
                        txt(row, "wallet"),
                        txt(row, "asset"),
                        quantity.clone(),
                        msat(&quantity),
                        fiat(row, "proceeds"),
                        fiat(row, "cost_basis"),
                        fiat(row, "gain_loss"),
                    ]
                })
                .collect(),
            total: None,
            note: t(if disposals.is_empty() {
                "largest.no_disposals"
            } else {
                "largest.disposals_note"
            })
            .into(),
        }),
        Block::Table(Table {
            id: "largest_movements",
            title: t("largest.movements").into(),
            columns: vec![
                col("date", t("col.date"), Kind::Date, 0.9),
                col("wallet", t("col.wallet"), Kind::Text, 1.0),
                col("direction", t("col.direction"), Kind::Text, 0.8),
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col("quantity_btc", t("col.quantity_btc"), Kind::Btc, 1.2),
                col("quantity_msat", t("col.quantity_msat"), Kind::Msat, 1.0),
                col("value", tf("col.value"), Kind::Fiat, 1.1),
                col("counterparty", t("col.counterparty"), Kind::Text, 1.6),
            ],
            rows: movements
                .iter()
                .map(|row| {
                    let quantity = btc(row, "quantity");
                    let direction = text(row, "direction");
                    vec![
                        day(row, "occurred_at"),
                        txt(row, "wallet"),
                        match lang.get(&format!("direction.{direction}")) {
                            Some(word) => Cell::Text(word.into()),
                            None => txt(row, "direction"),
                        },
                        txt(row, "asset"),
                        quantity.clone(),
                        msat(&quantity),
                        fiat(row, "fiat_value"),
                        txt(row, "counterparty"),
                    ]
                })
                .collect(),
            total: None,
            note: t(if movements.is_empty() {
                "largest.no_movements"
            } else {
                "largest.movements_note"
            })
            .into(),
        }),
    ];
    sections.push(Section {
        id: "largest",
        title: t("largest.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: largest,
    });

    // 6. Country layer: Austria's E 1kv Kennzahlen per tax year.
    if austria {
        let mut rows = Vec::new();
        if let Value::Dict(years) = value(model, "kennzahlen") {
            for (year, totals) in years {
                let year = match year {
                    kassiber_pyjson::Key::Str(y) => y.as_str().unwrap_or_default().to_owned(),
                    other => format!("{other:?}"),
                };
                for (code, wording) in KENNZAHLEN {
                    let row = value(totals, code);
                    let cents = match value(row, "amount_eur_cents") {
                        Value::Int(n) => n.clone(),
                        _ => 0.into(),
                    };
                    let amount = cents
                        .to_string()
                        .parse::<Decimal>()
                        .ok()
                        .and_then(|d| d.div(&Decimal::from(100), &DECIMAL).ok())
                        .unwrap_or_else(Decimal::zero);
                    rows.push(vec![
                        Cell::Text(year.clone()),
                        Cell::Text(code.into()),
                        Cell::Text(wording.into()),
                        Cell::Text(if code == "801" {
                            "E 1".into()
                        } else {
                            "E 1kv".into()
                        }),
                        Cell::Fiat(amount),
                        count(row, "row_count"),
                    ]);
                }
            }
        }
        sections.push(Section {
            id: "country_at",
            title: t("at.kennzahlen.title").into(),
            lead: t("at.kennzahlen.lead").into(),
            page_break: false,
            landscape: false,
            blocks: vec![
                Block::Table(Table {
                    id: "e1kv_kennzahlen",
                    title: String::new(),
                    columns: vec![
                        col("tax_year", t("col.tax_year"), Kind::Text, 0.6),
                        col("kennzahl", t("col.kennzahl"), Kind::Text, 0.4),
                        col("field", t("col.field"), Kind::Text, 3.2),
                        col("form", t("col.form"), Kind::Text, 0.6),
                        col("amount_eur", t("col.amount_eur"), Kind::Fiat, 1.0),
                        col("rows", t("col.rows"), Kind::Count, 0.5),
                    ],
                    rows,
                    total: None,
                    note: String::new(),
                }),
                Block::Note {
                    text: t("at.domestic_note").into(),
                    accent: false,
                },
            ],
        });
    }

    // 7. Open issues. Red marks blockers only: stale journals and
    // quarantined transactions.
    let total_tx = int(integrity, "total_transactions");
    let priced = int(integrity, "priced_transactions");
    let internal = value(integrity, "internal_transfers");
    let issues = vec![
        Fact {
            display: Some(lang.tf(
                "issues.priced_value",
                &[
                    ("priced", &lang.integer(priced.into())),
                    ("total", &lang.integer(total_tx.into())),
                    (
                        "percent",
                        &lang.percent(&from_float(value(integrity, "priced_percentage"))),
                    ),
                ],
            )),
            ..fact(
                "priced_transactions",
                t("issues.priced"),
                Cell::Count(priced),
            )
        },
        Fact {
            accent: int(integrity, "quarantine_count") > 0,
            ..fact(
                "quarantines",
                t("issues.quarantined"),
                count(integrity, "quarantine_count"),
            )
        },
        Fact {
            display: Some(lang.tf(
                "issues.internal_value",
                &[
                    ("count", &lang.integer(int(internal, "count").into())),
                    ("value", &fiat(internal, "fiat_volume").display(lang)),
                    ("fiat", &currency),
                ],
            )),
            ..fact(
                "internal_transfers",
                t("issues.internal"),
                count(internal, "count"),
            )
        },
        Fact {
            display: Some(status_name(lang, &status)),
            accent: status != "current",
            ..fact(
                "journal_status",
                t("issues.journal_status"),
                Cell::Text(status.clone()),
            )
        },
    ];
    let mut blocks = vec![Block::Facts(issues)];
    let reasons = list(integrity, "quarantine_reasons");
    if reasons.is_empty() {
        blocks.push(Block::Note {
            text: t("issues.no_quarantine").into(),
            accent: false,
        });
    } else {
        blocks.push(Block::Table(Table {
            id: "quarantines",
            title: t("issues.reasons").into(),
            columns: vec![
                col("reason", t("col.reason"), Kind::Code, 3.0),
                col("transactions", t("col.transactions"), Kind::Count, 1.0),
            ],
            rows: reasons
                .iter()
                .map(|r| vec![txt(r, "reason"), count(r, "count")])
                .collect(),
            total: None,
            note: t("issues.reasons_note").into(),
        }));
    }
    sections.push(Section {
        id: "issues",
        title: t("issues.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 8. Method and provenance
    let mut method = vec![
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
            "valuation",
            t("method.valuation"),
            Cell::Text(tf("method.valuation_value")),
        ),
    ];
    if austria {
        method.push(fact(
            "regime",
            t("method.regimes"),
            Cell::Text(t("method.regimes_value").into()),
        ));
    } else {
        method.push(fact(
            "long_term_days",
            t("method.long_term_days"),
            Cell::Count(int(profile, "tax_long_term_days")),
        ));
    }
    let blocks = vec![
        Block::Facts(method),
        Block::Note {
            text: t(if austria {
                "method.review_gate"
            } else {
                "method.disclaimer"
            })
            .into(),
            accent: false,
        },
    ];
    sections.push(Section {
        id: "method",
        title: t("method.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    Document {
        kind: "summary",
        title: t("summary.title").into(),
        kicker: t(if austria {
            "doc.kicker.at"
        } else {
            "doc.kicker"
        })
        .into(),
        book: format!(
            "{} / {}",
            text(report, "workspace"),
            text(report, "profile")
        ),
        period: lang.tf(
            "doc.period",
            &[
                ("start", &lang.date(&text(timeframe, "start"))),
                ("end", &period_end),
            ],
        ),
        period_start: text(timeframe, "start"),
        period_end: text(timeframe, "end"),
        generated_at: text(report, "generated_at"),
        input_version: Some(input_version),
        fiat: currency,
        country,
        lang,
        sections,
    }
}

/// The data the summary document is built from, read like Python's
/// reports: the summary model, the profile's tax settings and, for
/// Austria, the E 1kv Kennzahl totals of each tax year the period covers.
pub fn summary_model(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    require_context(conn, &profile, false)?;
    let report = summary_pdf::model(conn, &workspace, &profile, args, sources)?;
    let settings = dict(
        [
            "fiat_currency",
            "gains_algorithm",
            "journal_input_version",
            "tax_country",
            "tax_long_term_days",
        ]
        .map(|key| (key, value(&profile, key).clone())),
    );
    let mut kennzahlen = Vec::new();
    if text(&profile, "tax_country") == "at" {
        let timeframe = value(&report, "timeframe");
        let first: i64 = text(timeframe, "start")[..4].parse().unwrap_or(0);
        let last: i64 = text(timeframe, "end")[..4].parse().unwrap_or(0);
        for year in first..=last {
            let annual = crate::austrian::report(
                conn,
                &workspace,
                &profile,
                &dict([("year", Value::Int(year.into()))]),
            )?;
            kennzahlen.push((year.to_string(), value(&annual, "kennzahl_totals").clone()));
        }
    }
    Ok(dict([
        ("summary", report),
        ("profile", settings),
        ("kennzahlen", dict(kennzahlen)),
    ]))
}

/// `reports.export-summary-pdf`: the summary report in the shared design.
/// The envelope keeps Python's keys and title; pages and bytes are the new
/// PDF's.
pub(crate) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<crate::artifact_csv::Prepared> {
    let model = summary_model(conn, args, sources)?;
    let (bytes, pages) = super::pdf(&summary(&model))?;
    let report = value(&model, "summary");
    let timeframe = value(report, "timeframe");
    let snapshot = value(report, "snapshot");
    let snapshot_on = truthy(snapshot);
    let title = text(report, "title");
    let year = |key: &str| -> Result<Value> {
        let head: String = text(timeframe, key).chars().take(4).collect();
        kassiber_pytext::parse_int(&head)
            .map(Value::Int)
            .map_err(|_| AppError::new("internal_error", "invalid summary period"))
    };
    let or_list = |key: &str| match value(report, key) {
        v if truthy(v) => v.clone(),
        _ => Value::List(vec![]),
    };
    let metadata = dict([
        ("pages", Value::Int(pages.into())),
        (
            "title",
            s(if title.is_empty() {
                "Kassiber Summary Report".into()
            } else {
                title
            }),
        ),
        ("timeframe", timeframe.clone()),
        ("wallets", value(report, "wallets").clone()),
        ("snapshot", Value::Bool(snapshot_on)),
        ("data_integrity", value(report, "data_integrity").clone()),
        ("wallet_holdings", value(report, "wallet_holdings").clone()),
        ("holdings_totals", value(report, "holdings_totals").clone()),
        ("metrics", value(report, "metrics").clone()),
        ("benchmark", value(report, "benchmark").clone()),
        ("top_movements", or_list("top_movements")),
        ("top_disposals", or_list("top_disposals")),
        ("holding_age", value(report, "holding_age").clone()),
        (
            "snapshot_totals",
            if snapshot_on {
                dict(
                    ["total_quantity", "asset_quantities", "total_market_value"]
                        .map(|key| (key, value(snapshot, key).clone())),
                )
            } else {
                Value::Null
            },
        ),
        (
            "snapshot_wallets",
            if snapshot_on {
                value(snapshot, "wallets").clone()
            } else {
                Value::Null
            },
        ),
        ("balance_history", value(report, "balance_history").clone()),
    ]);
    let wallet_ids = list(report, "wallets")
        .iter()
        .map(|w| s(py_str(value(w, "id"))))
        .collect();
    Ok(crate::artifact_csv::Prepared {
        files: vec![crate::artifact_csv::File {
            name: "summary.pdf".into(),
            sheet: String::new(),
            bytes,
            rows: 0,
        }],
        metadata,
        completion: Some(crate::export_completion::Completion {
            kind: "summary.pdf".into(),
            year: Some(year("start")?),
            end_year: Some(year("end")?),
            scope: dict([
                ("wallet_ids", Value::List(wallet_ids)),
                ("occurred_at_start", timeframe_value(timeframe, "start")),
                ("occurred_at_end", timeframe_value(timeframe, "end")),
            ]),
        }),
    })
}

fn timeframe_value(timeframe: &Value, key: &str) -> Value {
    value(timeframe, key).clone()
}
