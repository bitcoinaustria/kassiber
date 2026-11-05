//! The full report in the shared design, from the full report's model
//! (`full_pdf::model`, Python's `_build_full_report_context`): scope,
//! headline figures, holdings, disposals, flows, history, wallets, open
//! issues, method and every transaction.
use super::*;
use crate::report_design::{
    balance_chart, btc, col, count, day, fact, fiat, float, from_float, list, method_name, msat,
    txt, Block, Cell, Column, Document, Fact, Kind, Kpi, Lang, Section, Table,
};

fn table(id: &'static str, title: &str, columns: Vec<Column>, rows: Vec<Vec<Cell>>) -> Table {
    Table {
        id,
        title: title.into(),
        columns,
        rows,
        total: None,
        note: String::new(),
    }
}
/// A table, or the report's sentence for an empty one.
fn table_or(table: Table, empty: &str) -> Block {
    if table.rows.is_empty() {
        Block::Note {
            text: empty.into(),
            accent: false,
        }
    } else {
        Block::Table(table)
    }
}
/// An integer msat amount and its exact BTC value.
fn msat_pair(row: &Value, key: &str) -> [Cell; 2] {
    match value(row, key) {
        Value::Int(n) => {
            let btc = n
                .to_string()
                .parse::<Decimal>()
                .ok()
                .and_then(|d| d.div(&Decimal::from(100_000_000_000_i64), &DECIMAL).ok());
            [
                btc.map(Cell::Btc).unwrap_or(Cell::Empty),
                Cell::Msat(n.clone()),
            ]
        }
        _ => [Cell::Empty, Cell::Empty],
    }
}
/// `sum(float(...))` in row order, as Python totals the report's rows.
fn float_sum(rows: &[&Value], key: &str) -> Cell {
    Cell::Fiat(from_float(&Value::Float(
        rows.iter().map(|r| float(r, key)).sum::<f64>(),
    )))
}

/// The full report document; the book's tax country picks the language.
pub fn full(model: &Value) -> Document {
    let profile = value(model, "profile");
    let currency = text(profile, "fiat_currency");
    let country = text(profile, "tax_country");
    let lang = Lang::for_country(&country);
    let austria = country == "at";
    let t = |key: &str| lang.t(key);
    let currency_arg = [("fiat", currency.as_str())];
    let tf = |key: &str| lang.tf(key, &currency_arg);
    let metrics = value(model, "metrics");
    let rollups = value(model, "rollups");
    let holdings = value(rollups, "holdings");
    let realized = value(rollups, "realized");
    let input_version = int(profile, "journal_input_version");
    let book = format!("{} / {}", text(model, "workspace"), text(profile, "label"));
    let scope_label = match value(model, "wallet") {
        Value::Null => t("scope.all_wallets").to_owned(),
        wallet => py_str(wallet),
    };
    let mut sections = Vec::new();

    // 1. Scope
    let mut scope = vec![
        fact("book", t("scope.book"), Cell::Text(book.clone())),
        fact(
            "wallet_scope",
            t("scope.wallet_scope"),
            Cell::Text(scope_label.clone()),
        ),
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
    ];
    if !austria {
        scope.push(fact(
            "long_term_days",
            t("scope.long_term_days"),
            count(profile, "tax_long_term_days"),
        ));
    }
    let processed = value(profile, "last_processed_at");
    scope.extend([
        Fact {
            display: truthy(processed).then(|| lang.date_time(&py_str(processed))),
            ..fact(
                "last_processed_at",
                t("scope.last_processed"),
                txt(profile, "last_processed_at"),
            )
        },
        fact(
            "processed_tx_count",
            t("scope.processed_count"),
            count(profile, "last_processed_tx_count"),
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
    ]);
    sections.push(Section {
        id: "scope",
        title: t("scope.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![Block::Facts(scope)],
    });

    // 2. Summary: headline figures and the book's counts
    let kpi = |key: &'static str, label: String, value: Cell| Kpi {
        key,
        label,
        value,
        detail: String::new(),
        accent: false,
    };
    let kpis = vec![
        kpi(
            "holdings_cost_basis",
            tf("kpi.holdings_cost_basis"),
            fiat(holdings, "cost_basis"),
        ),
        kpi(
            "holdings_market_value",
            tf("kpi.holdings_market_value"),
            fiat(holdings, "market_value"),
        ),
        kpi(
            "holdings_unrealized",
            tf("kpi.holdings_unrealized"),
            fiat(holdings, "unrealized_pnl"),
        ),
        // Python's realized totals add income to the disposals.
        kpi(
            "realized_proceeds",
            tf("kpi.realized_proceeds_income"),
            fiat(realized, "proceeds"),
        ),
        kpi(
            "realized_cost_basis",
            tf("kpi.realized_cost_basis"),
            fiat(realized, "cost_basis"),
        ),
        kpi(
            "realized_gain",
            tf("kpi.realized_gain_income"),
            fiat(realized, "gain_loss"),
        ),
    ];
    let when = |key: &str| -> Cell {
        let raw = text(metrics, key);
        if raw.is_empty() {
            Cell::Empty
        } else {
            Cell::Text(raw)
        }
    };
    let moment = |key: &'static str, label: &str| {
        let cell = when(key);
        Fact {
            display: match &cell {
                Cell::Text(raw) => Some(lang.date_time(raw)),
                _ => None,
            },
            ..fact(key, label, cell)
        }
    };
    let counts = vec![
        fact(
            "wallets_in_scope",
            t("counts.wallets"),
            count(metrics, "wallets_in_scope"),
        ),
        fact(
            "assets_in_scope",
            t("counts.assets"),
            count(metrics, "assets_in_scope"),
        ),
        fact(
            "active_transactions",
            t("counts.active"),
            count(metrics, "active_transactions"),
        ),
        fact(
            "excluded_transactions",
            t("counts.excluded"),
            count(metrics, "excluded_transactions"),
        ),
        fact(
            "inbound_transactions",
            t("counts.inbound"),
            count(metrics, "inbound_transactions"),
        ),
        fact(
            "outbound_transactions",
            t("counts.outbound"),
            count(metrics, "outbound_transactions"),
        ),
        fact(
            "journal_entries",
            t("counts.journal_entries"),
            count(metrics, "journal_entries"),
        ),
        Fact {
            accent: int(metrics, "quarantines") > 0,
            ..fact(
                "quarantines",
                t("issues.quarantined"),
                count(metrics, "quarantines"),
            )
        },
        fact(
            "priced_transactions",
            t("issues.priced"),
            count(metrics, "priced_transactions"),
        ),
        fact(
            "transactions_with_notes",
            t("counts.notes"),
            count(metrics, "transactions_with_notes"),
        ),
        fact(
            "transactions_with_tags",
            t("counts.tags"),
            count(metrics, "transactions_with_tags"),
        ),
        moment("first_transaction_at", t("counts.first")),
        moment("last_transaction_at", t("counts.last")),
    ];
    sections.push(Section {
        id: "summary",
        title: t("summary.title.figures").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![
            Block::Kpis(kpis),
            Block::Note {
                text: t("counts.title").into(),
                accent: false,
            },
            Block::Facts(counts),
        ],
    });

    // 3. Holdings: by account, then by wallet, at the latest rate
    let balances = list(model, "balances");
    let mut by_bucket = table(
        "holdings_by_account",
        t("holdings.by_bucket"),
        vec![
            col("account", t("col.bucket"), Kind::Text, 1.2),
            col("asset", t("col.asset"), Kind::Text, 0.6),
            col("quantity", t("col.quantity"), Kind::Btc, 1.3),
            col(
                "quantity_msat",
                t("col.quantity_msat_plain"),
                Kind::Msat,
                1.0,
            ),
            col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.2),
            col("market_value", tf("col.market_value"), Kind::Fiat, 1.2),
            col("unrealized", tf("col.unrealized"), Kind::Fiat, 1.2),
        ],
        balances
            .iter()
            .map(|row| {
                let quantity = btc(row, "quantity");
                vec![
                    txt(row, "account"),
                    txt(row, "asset"),
                    quantity.clone(),
                    msat(&quantity),
                    fiat(row, "cost_basis"),
                    fiat(row, "market_value"),
                    fiat(row, "unrealized_pnl"),
                ]
            })
            .collect(),
    );
    if !balances.is_empty() {
        let blank = || Cell::Text(String::new());
        by_bucket.total = Some(vec![
            Cell::Text(t("doc.total").into()),
            blank(),
            blank(),
            blank(),
            fiat(holdings, "cost_basis"),
            fiat(holdings, "market_value"),
            fiat(holdings, "unrealized_pnl"),
        ]);
    }
    let by_wallet = table(
        "holdings_by_wallet",
        t("holdings.by_wallet"),
        vec![
            col("wallet", t("col.wallet"), Kind::Text, 1.2),
            col("account", t("col.bucket"), Kind::Text, 0.9),
            col("asset", t("col.asset"), Kind::Text, 0.6),
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
                let quantity = btc(row, "quantity");
                vec![
                    txt(row, "wallet"),
                    txt(row, "account"),
                    txt(row, "asset"),
                    quantity.clone(),
                    msat(&quantity),
                    fiat(row, "avg_cost"),
                    fiat(row, "cost_basis"),
                    fiat(row, "market_value"),
                    fiat(row, "unrealized_pnl"),
                ]
            })
            .collect(),
    );
    let holdings_blocks = if balances.is_empty() && by_wallet.rows.is_empty() {
        vec![table_or(by_bucket, t("holdings.none"))]
    } else {
        vec![Block::Table(by_bucket), Block::Table(by_wallet)]
    };
    sections.push(Section {
        id: "holdings",
        title: t("holdings.title").into(),
        lead: t("holdings.current_lead").into(),
        page_break: false,
        landscape: false,
        blocks: holdings_blocks,
    });

    // 4. Disposals, then income: Python's capital rows hold both, and the
    // journal's entry type says which each row is. Income is no disposal and
    // carries no cost basis, so it gets its own table and total.
    let capital = list(model, "capital");
    let disposals: Vec<&Value> = capital
        .iter()
        .filter(|r| text(r, "entry_type") != "income")
        .collect();
    let income: Vec<&Value> = capital
        .iter()
        .filter(|r| text(r, "entry_type") == "income")
        .collect();
    let blank = || Cell::Text(String::new());
    let mut blocks = Vec::new();
    if disposals.is_empty() {
        blocks.push(Block::Note {
            text: t("disposals.none").into(),
            accent: false,
        });
    } else {
        let mut groups: Vec<((String, String), Vec<&Value>)> = Vec::new();
        for &row in &disposals {
            let key = (text(row, "wallet"), text(row, "asset"));
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, rows)) => rows.push(row),
                None => groups.push((key, vec![row])),
            }
        }
        let mut summary = table(
            "disposals_by_wallet",
            t("disposals.by_wallet"),
            vec![
                col("wallet", t("col.wallet"), Kind::Text, 1.3),
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col("disposals", t("col.disposals"), Kind::Count, 0.8),
                col("proceeds", tf("col.proceeds"), Kind::Fiat, 1.2),
                col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.2),
                col("gain", tf("col.gain"), Kind::Fiat, 1.2),
            ],
            groups
                .iter()
                .map(|((wallet, asset), rows)| {
                    vec![
                        Cell::Text(wallet.clone()),
                        Cell::Text(asset.clone()),
                        Cell::Count(rows.len() as i64),
                        float_sum(rows, "proceeds"),
                        float_sum(rows, "cost_basis"),
                        float_sum(rows, "gain_loss"),
                    ]
                })
                .collect(),
        );
        summary.total = Some(vec![
            Cell::Text(t("doc.total").into()),
            blank(),
            Cell::Count(disposals.len() as i64),
            float_sum(&disposals, "proceeds"),
            float_sum(&disposals, "cost_basis"),
            float_sum(&disposals, "gain_loss"),
        ]);
        blocks.push(Block::Table(summary));
        blocks.push(Block::Table(table(
            "all_disposals",
            t("disposals.all"),
            vec![
                col("date", t("col.date"), Kind::Date, 0.9),
                col("wallet", t("col.wallet"), Kind::Text, 1.2),
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col("quantity", t("col.quantity"), Kind::Btc, 1.2),
                col(
                    "quantity_msat",
                    t("col.quantity_msat_plain"),
                    Kind::Msat,
                    1.0,
                ),
                col("proceeds", tf("col.proceeds"), Kind::Fiat, 1.2),
                col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.2),
                col("gain", tf("col.gain"), Kind::Fiat, 1.1),
            ],
            disposals
                .iter()
                .map(|row| {
                    let quantity = btc(row, "quantity");
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
        )));
    }
    sections.push(Section {
        id: "disposals",
        title: t("disposals.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });
    let income_block = if income.is_empty() {
        Block::Note {
            text: t("income.none").into(),
            accent: false,
        }
    } else {
        let mut receipts = table(
            "income",
            t("income.all"),
            vec![
                col("date", t("col.date"), Kind::Date, 0.9),
                col("wallet", t("col.wallet"), Kind::Text, 1.2),
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col("quantity", t("col.quantity"), Kind::Btc, 1.2),
                col(
                    "quantity_msat",
                    t("col.quantity_msat_plain"),
                    Kind::Msat,
                    1.0,
                ),
                // The journal books income at its value when received.
                col("value", tf("col.income_value"), Kind::Fiat, 1.3),
            ],
            income
                .iter()
                .map(|row| {
                    let quantity = btc(row, "quantity");
                    vec![
                        day(row, "occurred_at"),
                        txt(row, "wallet"),
                        txt(row, "asset"),
                        quantity.clone(),
                        msat(&quantity),
                        fiat(row, "proceeds"),
                    ]
                })
                .collect(),
        );
        receipts.total = Some(vec![
            Cell::Text(t("doc.total").into()),
            blank(),
            blank(),
            blank(),
            blank(),
            float_sum(&income, "proceeds"),
        ]);
        Block::Table(receipts)
    };
    sections.push(Section {
        id: "income",
        title: t("income.title").into(),
        lead: t("income.lead").into(),
        page_break: false,
        landscape: false,
        blocks: vec![income_block],
    });

    // 5. Flows and fees per asset and per wallet
    let flow_table = |id: &'static str, title: &str, key: &str, wallet: bool| {
        let mut columns = Vec::new();
        if wallet {
            columns.push(col("wallet", t("col.wallet"), Kind::Text, 1.1));
        }
        columns.extend([
            col("asset", t("col.asset"), Kind::Text, 0.6),
            col("transactions", t("col.transactions"), Kind::Count, 0.9),
            col("inbound_count", t("col.inbound_count"), Kind::Count, 0.7),
            col("outbound_count", t("col.outbound_count"), Kind::Count, 0.7),
            col("inbound_amount", t("col.inbound_amount"), Kind::Btc, 1.2),
            col(
                "inbound_amount_msat",
                t("col.inbound_amount_msat"),
                Kind::Msat,
                1.0,
            ),
            col("outbound_amount", t("col.outbound_amount"), Kind::Btc, 1.2),
            col(
                "outbound_amount_msat",
                t("col.outbound_amount_msat"),
                Kind::Msat,
                1.0,
            ),
            col("fee_amount", t("col.fee_amount"), Kind::Btc, 1.1).sparse(),
            col("fee_amount_msat", t("col.fee_amount_msat"), Kind::Msat, 1.0),
        ]);
        let rows = list(model, key)
            .iter()
            .map(|row| {
                let mut cells = Vec::new();
                if wallet {
                    cells.push(txt(row, "wallet"));
                }
                cells.extend([
                    txt(row, "asset"),
                    count(row, "tx_count"),
                    count(row, "inbound_count"),
                    count(row, "outbound_count"),
                ]);
                for key in ["inbound_msat", "outbound_msat", "fee_msat"] {
                    cells.extend(msat_pair(row, key));
                }
                cells
            })
            .collect();
        table(id, title, columns, rows)
    };
    let by_asset = flow_table("flows_by_asset", t("flows.by_asset"), "asset_flow", false);
    let blocks = if by_asset.rows.is_empty() {
        vec![table_or(by_asset, t("flows.none"))]
    } else {
        vec![
            Block::Table(by_asset),
            Block::Table(flow_table(
                "flows_by_wallet",
                t("flows.by_wallet"),
                "wallet_flow",
                true,
            )),
        ]
    };
    sections.push(Section {
        id: "flows",
        title: t("flows.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    });

    // 6. Balance history: the fiat totals per month as a chart, the
    // quantity line only when the history is one asset's
    let history = list(model, "history");
    let mut blocks = Vec::new();
    if !history.is_empty() {
        let mut months: Vec<String> = Vec::new();
        let mut market: Vec<f64> = Vec::new();
        let mut basis: Vec<f64> = Vec::new();
        let mut quantity: Vec<f64> = Vec::new();
        for row in history {
            let month: String = text(row, "period_start").chars().take(7).collect();
            if months.last() != Some(&month) {
                months.push(month);
                market.push(0.0);
                basis.push(0.0);
                quantity.push(0.0);
            }
            *market.last_mut().unwrap() += float(row, "market_value");
            *basis.last_mut().unwrap() += float(row, "cumulative_cost_basis");
            *quantity.last_mut().unwrap() += float(row, "quantity");
        }
        let assets: std::collections::BTreeSet<String> =
            history.iter().map(|r| text(r, "asset")).collect();
        blocks.push(Block::Chart(balance_chart(
            lang,
            t("history.chart"),
            &months,
            &market,
            &basis,
            (assets.len() == 1).then_some(quantity.as_slice()),
            &currency,
        )));
    }
    blocks.push(table_or(
        table(
            "balance_history",
            "",
            vec![
                col("month", t("col.month"), Kind::Date, 1.2),
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col("quantity", t("col.quantity"), Kind::Btc, 1.3),
                col(
                    "quantity_msat",
                    t("col.quantity_msat_plain"),
                    Kind::Msat,
                    1.0,
                ),
                col("cost_basis", tf("col.cost_basis"), Kind::Fiat, 1.3),
                col("market_value", tf("col.market_value"), Kind::Fiat, 1.3),
            ],
            history
                .iter()
                .map(|row| {
                    let quantity = btc(row, "quantity");
                    vec![
                        Cell::Date(text(row, "period_start").chars().take(7).collect()),
                        txt(row, "asset"),
                        quantity.clone(),
                        msat(&quantity),
                        fiat(row, "cumulative_cost_basis"),
                        fiat(row, "market_value"),
                    ]
                })
                .collect(),
        ),
        t("history.none"),
    ));
    sections.push(Section {
        id: "history",
        title: t("history.title").into(),
        lead: if history.is_empty() {
            String::new()
        } else {
            t("history.lead").into()
        },
        page_break: false,
        landscape: false,
        blocks,
    });

    // 7. Wallets in scope
    sections.push(Section {
        id: "wallets",
        title: t("wallets.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![table_or(
            table(
                "wallets",
                "",
                vec![
                    col("wallet", t("col.wallet"), Kind::Text, 1.4),
                    col("kind", t("col.kind"), Kind::Text, 1.0),
                    col("chain", t("col.chain"), Kind::Text, 0.8),
                    col("network", t("col.network"), Kind::Text, 0.9),
                    col("backend", t("col.backend"), Kind::Text, 1.0),
                    col("gap_limit", t("col.gap_limit"), Kind::Text, 0.7),
                ],
                list(model, "wallets")
                    .iter()
                    .map(|row| {
                        ["label", "kind", "chain", "network", "backend", "gap_limit"]
                            .iter()
                            .map(|key| Cell::Text(text(row, key)))
                            .collect()
                    })
                    .collect(),
            ),
            t("wallets.none"),
        )],
    });

    // 8. Open issues: quarantine reasons
    let quarantines = list(model, "quarantines");
    let issues = if quarantines.is_empty() {
        Block::Note {
            text: t("issues.no_quarantine_scope").into(),
            accent: false,
        }
    } else {
        Block::Table(Table {
            note: t("issues.reasons_note").into(),
            ..table(
                "quarantines",
                t("issues.reasons"),
                vec![
                    col("reason", t("col.reason"), Kind::Code, 3.0),
                    col("transactions", t("col.transactions"), Kind::Count, 1.0),
                ],
                quarantines
                    .iter()
                    .map(|r| vec![txt(r, "reason"), count(r, "count")])
                    .collect(),
            )
        })
    };
    sections.push(Section {
        id: "issues",
        title: t("issues.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![issues],
    });

    // 9. Method and provenance
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
    ];
    if austria {
        method.push(fact(
            "regime",
            t("method.regimes"),
            Cell::Text(t("method.regimes_value").into()),
        ));
    }
    sections.push(Section {
        id: "method",
        title: t("method.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![
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
        ],
    });

    // 10. Every transaction in scope
    let transactions = table(
        "transactions",
        "",
        vec![
            col("date", t("col.date"), Kind::Date, 0.9),
            col("wallet", t("col.wallet"), Kind::Text, 1.0),
            col("direction", t("col.direction"), Kind::Text, 0.7),
            col("asset", t("col.asset"), Kind::Text, 0.5),
            col("amount", t("col.amount"), Kind::Btc, 1.1),
            col("amount_msat", t("col.amount_msat"), Kind::Msat, 1.0),
            col("fee", t("col.fee"), Kind::Btc, 1.0).sparse(),
            col("fee_msat", t("col.fee_msat"), Kind::Msat, 1.0),
            col("description", t("col.description"), Kind::Text, 1.8),
        ],
        list(model, "transactions")
            .iter()
            .map(|row| {
                let direction = text(row, "direction");
                let [amount, amount_msat] = msat_pair(row, "amount_msat");
                let [fee, fee_msat] = msat_pair(row, "fee_msat");
                vec![
                    day(row, "occurred_at"),
                    txt(row, "wallet"),
                    match lang.get(&format!("direction.{direction}")) {
                        Some(word) => Cell::Text(word.into()),
                        None => Cell::Text(direction),
                    },
                    txt(row, "asset"),
                    amount,
                    amount_msat,
                    fee,
                    fee_msat,
                    txt(row, "description"),
                ]
            })
            .collect(),
    );
    sections.push(Section {
        id: "transactions",
        title: t("transactions.title").into(),
        lead: String::new(),
        page_break: !transactions.rows.is_empty(),
        landscape: false,
        blocks: vec![table_or(transactions, t("transactions.none"))],
    });

    Document {
        kind: "full",
        title: t("full.title").into(),
        kicker: t(if austria {
            "doc.kicker.at"
        } else {
            "doc.kicker"
        })
        .into(),
        book,
        period: scope_label,
        period_start: String::new(),
        period_end: String::new(),
        generated_at: text(model, "generated_at"),
        input_version: Some(input_version),
        fiat: currency,
        country,
        lang,
        sections,
    }
}
