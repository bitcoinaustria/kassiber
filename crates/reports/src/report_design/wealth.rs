//! The Stichtag wealth statement (`wealth::model`) in the shared report design:
//! PDF (uniform Portrait layout, diagonal ENTWURF/DRAFT watermark when draft),
//! XLSX (with live `=MIN(...)` lower-of-cost-or-market formulas), and CSV ZIP
//! bundle.
use super::*;
use crate::report_design::{
    col, fact, list, method_name, Block, Cell, Column, Document, Fact, Kind, Kpi, Lang, Section,
    Table,
};

fn dec_cell(v: &Value, key: &str, as_btc: bool) -> Cell {
    let raw = value(v, key);
    if matches!(raw, Value::Null) || raw == &s("") {
        return Cell::Empty;
    }
    let parsed = py_str(raw).parse::<Decimal>().ok();
    match parsed {
        Some(d) if as_btc => Cell::Btc(d),
        Some(d) => Cell::Fiat(d),
        None => Cell::Empty,
    }
}

fn pct_cell(v: &Value, key: &str) -> Cell {
    let raw = value(v, key);
    if matches!(raw, Value::Null) || raw == &s("") {
        return Cell::Empty;
    }
    py_str(raw)
        .parse::<Decimal>()
        .map(Cell::Percent)
        .unwrap_or(Cell::Empty)
}

fn msat_cell(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Int(n) => Cell::Msat(n.clone()),
        _ => Cell::Empty,
    }
}

fn table(
    id: &'static str,
    title: String,
    columns: Vec<Column>,
    rows: Vec<Vec<Cell>>,
    total: Option<Vec<Cell>>,
    note: String,
) -> Table {
    Table {
        id,
        title,
        columns,
        rows,
        total,
        note,
    }
}

fn format_rate_source(lang: Lang, source: &str, provider: &str) -> String {
    let p = provider.trim().to_ascii_lowercase();
    if p == "manual" {
        return lang.t("wealth.rate_source.manual").to_owned();
    }
    if p == "estv_kursliste" {
        return "ESTV-Kursliste".to_owned();
    }
    if p == "transaction" || source == "transaction" {
        return lang.t("exit.source.transaction").to_owned();
    }
    let known = match p.as_str() {
        "kraken" | "kraken_csv" => Some("Kraken"),
        "coinbase" | "coinbase_csv" => Some("Coinbase"),
        "bitstamp" | "bitstamp_csv" => Some("Bitstamp"),
        "coingecko" => Some("CoinGecko"),
        "bitpanda" | "bitpanda_csv" => Some("Bitpanda"),
        "binance" | "binance_csv" => Some("Binance"),
        "cryptocompare" => Some("CryptoCompare"),
        "mempool" => Some("Mempool"),
        "ecb" => Some("ECB"),
        _ => None,
    };
    if let Some(name) = known {
        return lang.tf("wealth.rate_source.market", &[("provider", name)]);
    }
    if !p.is_empty() {
        let base = p.split('_').next().unwrap_or(&p);
        let mut chars = base.chars();
        let pretty = match chars.next() {
            Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
            None => String::new(),
        };
        if !pretty.is_empty() {
            return lang.tf("wealth.rate_source.market", &[("provider", &pretty)]);
        }
    }
    lang.get(&format!("exit.source.{source}"))
        .unwrap_or(source)
        .to_owned()
}

fn localize_draft_reasons(lang: Lang, report: &Value) -> Vec<String> {
    let codes = list(report, "draftReasonCodes");
    if codes.is_empty() {
        return list(report, "draftReasons").iter().map(py_str).collect();
    }
    codes
        .iter()
        .map(|entry| {
            let code = text(entry, "code");
            let count = int(entry, "count").to_string();
            let day_raw = text(entry, "as_of");
            let day_fmt = if day_raw.is_empty() {
                String::new()
            } else {
                lang.date(&day_raw)
            };
            match code.as_str() {
                "freshness" => lang.tf("wealth.reason.freshness", &[("count", &count)]),
                "silent_payments" => lang.tf("wealth.reason.silent_payments", &[("count", &count)]),
                "custody_components" => {
                    lang.tf("wealth.reason.custody_components", &[("count", &count)])
                }
                "quantity_issues" => lang.tf(
                    "wealth.reason.quantity_issues",
                    &[("count", &count), ("date", &day_fmt)],
                ),
                "stale_journals" => lang.t("wealth.reason.stale_journals").to_owned(),
                "quarantines" => lang.tf(
                    "wealth.reason.quarantines",
                    &[("count", &count), ("date", &day_fmt)],
                ),
                _ => code,
            }
        })
        .collect()
}

pub fn wealth(model: &Value) -> Document {
    let report = value(model, "report");
    let country = text(report, "taxCountry").to_lowercase();
    let lang = Lang::for_country(&country);
    let t = |key: &str| lang.t(key);
    let currency = text(report, "fiatCurrency");
    let ccy = [("fiat", currency.as_str())];
    let tf = |key: &str| lang.tf(key, &ccy);

    let as_of = text(report, "asOfDate");
    let tz = text(report, "asOfTimezone");
    let is_draft = truthy(value(report, "isDraft"));
    let privacy = truthy(value(report, "privacyMode"));
    let include_regime = truthy(value(report, "includeRegime"));
    let regime_resolved = truthy(value(report, "regimeResolved"));
    let include_valuation = truthy(value(report, "includeValuation"));
    let include_wallets = get(report, "includeWallets").is_none_or(truthy);
    let per_wallet_basis = truthy(value(report, "perWalletBasis"));
    let pool_scope = text(report, "costBasisPoolScope");
    let data_source = text(report, "dataSource");
    let totals = value(report, "totals");
    let is_austrian = super::austrian_rules(&country);
    let book = format!(
        "{} / {}",
        text(report, "workspace"),
        text(report, "profile")
    );
    let wallet_scope_items: Vec<String> = list(report, "walletScope").iter().map(py_str).collect();
    let wallet_scope_text = if privacy {
        let classes: Vec<String> = list(report, "scopeCustodyClasses")
            .iter()
            .map(py_str)
            .collect();
        let has_self = classes.iter().any(|class| class != "platform");
        let has_platform = classes.iter().any(|class| class == "platform");
        match (has_self, has_platform) {
            (false, true) => t("privacy.platform").to_owned(),
            (true, true) => format!("{}, {}", t("privacy.self_custody"), t("privacy.platform")),
            _ => t("privacy.self_custody").to_owned(),
        }
    } else if wallet_scope_items.is_empty() {
        t("scope.all_wallets").to_owned()
    } else {
        wallet_scope_items.join(", ")
    };

    let pool_key = match pool_scope.as_str() {
        "wallet" => "wealth.pool_scope.wallet",
        "global_then_wallet" => "wealth.pool_scope.global_then_wallet",
        _ => "wealth.pool_scope.global",
    };
    let source_key = if data_source == "journals" {
        "wealth.data_source.journals"
    } else {
        "wealth.data_source.observed"
    };
    let status_key = if is_draft {
        "wealth.status.draft"
    } else {
        "wealth.status.final"
    };

    let input_version = match value(report, "journalInputVersion") {
        Value::Int(n) => i64::try_from(n.clone()).ok(),
        _ => None,
    };

    let as_of_display = lang.tf(
        "wealth.as_of_cutoff",
        &[("date", &lang.date(&as_of)), ("timezone", &tz)],
    );
    let missing_fmv: Vec<String> = list(report, "missingFmvAssets")
        .iter()
        .map(py_str)
        .collect();
    let quarantines = int(report, "quarantinesBeforeAsOf");
    let draft_reasons = localize_draft_reasons(lang, report);
    let clean_final = !is_draft && missing_fmv.is_empty() && quarantines == 0;

    let mut sections = Vec::new();

    // 1. Scope
    let mut scope_facts = vec![
        fact("book", t("scope.book"), Cell::Text(book.clone())),
        Fact {
            display: Some(as_of_display),
            ..fact("as_of_date", t("wealth.as_of"), Cell::Date(as_of.clone()))
        },
        Fact {
            display: Some(t(status_key).into()),
            accent: is_draft,
            ..fact(
                "status",
                t("wealth.status"),
                Cell::Text(text(report, "status")),
            )
        },
        Fact {
            display: Some(t(source_key).into()),
            accent: data_source != "journals",
            ..fact(
                "data_source",
                t("wealth.data_source"),
                Cell::Text(data_source.clone()),
            )
        },
        fact("wallets", t("scope.wallets"), Cell::Text(wallet_scope_text)),
        fact(
            "fiat_currency",
            t("scope.fiat_currency"),
            Cell::Text(currency.clone()),
        ),
        fact(
            "tax_country",
            t("scope.tax_country"),
            Cell::Text(lang.country_name(&country).into()),
        ),
        fact(
            "method",
            t("scope.method"),
            Cell::Text(method_name(lang, &text(report, "method").to_uppercase())),
        ),
        Fact {
            display: Some(t(pool_key).into()),
            ..fact("pool_scope", t("wealth.pool_scope"), Cell::Text(pool_scope))
        },
    ];
    if clean_final {
        scope_facts.push(Fact {
            display: Some(t("wealth.audit_status.verified").into()),
            ..fact(
                "verification",
                t("wealth.audit_status"),
                Cell::Text("verified".into()),
            )
        });
    }
    if let Some(v) = input_version {
        scope_facts.push(fact(
            "journal_input_version",
            t("wealth.input_version"),
            Cell::Count(v),
        ));
    }
    scope_facts.push(Fact {
        display: Some(lang.date_time(&text(model, "generated_at"))),
        ..fact(
            "generated_at",
            t("scope.generated_at"),
            Cell::Text(text(model, "generated_at")),
        )
    });

    let mut scope_blocks = Vec::new();
    if is_draft {
        let notice_text = if draft_reasons.is_empty() {
            t("draft.notice").to_owned()
        } else {
            format!("{} {}", t("draft.notice"), draft_reasons.join(" "))
        };
        scope_blocks.push(Block::Note {
            text: notice_text,
            accent: true,
        });
    }
    scope_blocks.push(Block::Facts(scope_facts));

    sections.push(Section {
        id: "scope",
        title: t("scope.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: scope_blocks,
    });

    // 2. Summary KPIs
    let total_qty_detail = if regime_resolved {
        let alt_cell = dec_cell(totals, "altQuantityBtc", true);
        let neu_cell = dec_cell(totals, "neuQuantityBtc", true);
        lang.tf(
            "wealth.kpi.alt_neu_split",
            &[
                ("alt", &alt_cell.display(lang)),
                ("neu", &neu_cell.display(lang)),
            ],
        )
    } else {
        String::new()
    };
    let lower_detail = match dec_cell(totals, "impairmentIndicator", false) {
        Cell::Fiat(ref d) if *d > Decimal::zero() => lang.tf(
            "wealth.kpi.impairment_detail",
            &[("amount", &lang.number(d, 2)), ("fiat", &currency)],
        ),
        Cell::Fiat(_) => t("wealth.kpi.no_impairment").to_owned(),
        _ => String::new(),
    };
    let mut kpis = vec![Kpi {
        key: "total_quantity",
        label: t("wealth.kpi.total_qty").into(),
        value: dec_cell(totals, "quantityBtc", true),
        detail: total_qty_detail,
        accent: false,
    }];
    if include_valuation {
        kpis.push(Kpi {
            key: "market_value",
            label: tf("wealth.kpi.market_value"),
            value: dec_cell(totals, "marketValue", false),
            detail: String::new(),
            accent: false,
        });
        kpis.push(Kpi {
            key: "cost_basis",
            label: tf("wealth.kpi.cost_basis"),
            value: dec_cell(totals, "costBasis", false),
            detail: String::new(),
            accent: false,
        });
        kpis.push(Kpi {
            key: "lower_of_cost_or_market",
            label: tf("wealth.kpi.lower_of_cost_or_market"),
            value: dec_cell(totals, "lowerOfCostOrMarket", false),
            detail: lower_detail,
            accent: false,
        });
    } else if regime_resolved {
        kpis.push(Kpi {
            key: "alt_quantity",
            label: t("wealth.kpi.alt_qty").into(),
            value: dec_cell(totals, "altQuantityBtc", true),
            detail: String::new(),
            accent: false,
        });
        kpis.push(Kpi {
            key: "neu_quantity",
            label: t("wealth.kpi.neu_qty").into(),
            value: dec_cell(totals, "neuQuantityBtc", true),
            detail: String::new(),
            accent: false,
        });
    }
    let mut summary_blocks = vec![Block::Kpis(kpis)];
    if !missing_fmv.is_empty() {
        summary_blocks.push(Block::Note {
            text: lang.tf(
                "wealth.valuation.missing",
                &[("assets", &missing_fmv.join(", "))],
            ),
            accent: true,
        });
    }
    sections.push(Section {
        id: "summary",
        title: t("wealth.summary.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: summary_blocks,
    });

    // 3. Holdings by asset (uniform Portrait layout with compact column headers)
    let regime_label = |code: &str| -> String {
        let key = format!("wealth.regime.{code}");
        lang.get(&key).unwrap_or(code).to_owned()
    };
    let asset_rows_raw = list(report, "assets");
    let assets_lead = if !include_valuation {
        t("wealth.assets.lead_quantity_only")
    } else if per_wallet_basis {
        t("wealth.assets.lead_wallet")
    } else {
        t("wealth.assets.lead_global")
    };
    let mut asset_cols = vec![col("asset", t("col.asset"), Kind::Text, 0.55)];
    if include_regime {
        asset_cols.push(col("regime", t("wealth.col.regime"), Kind::Text, 0.92));
    }
    // Rows hold each asset's quantity in its own unit; the header says BTC
    // only when every row is Bitcoin, and only Bitcoin rows carry msat.
    let all_bitcoin = list(totals, "excludedQuantityAssets").is_empty();
    let quantity_header = if all_bitcoin {
        t("col.quantity_btc")
    } else {
        t("col.quantity")
    };
    asset_cols.push(col("quantity_btc", quantity_header, Kind::Btc, 1.28));
    asset_cols.push(col(
        "quantity_msat",
        t("col.quantity_msat"),
        Kind::Msat,
        1.0,
    ));
    if include_valuation {
        asset_cols.push(col("avg_cost", tf("wealth.col.avg_cost"), Kind::Fiat, 0.98));
        asset_cols.push(col(
            "cost_basis",
            tf("wealth.col.cost_basis"),
            Kind::Fiat,
            1.02,
        ));
        asset_cols.push(col(
            "unit_rate",
            tf("wealth.col.unit_rate"),
            Kind::Fiat,
            1.02,
        ));
        asset_cols.push(col(
            "market_value",
            tf("col.market_value"),
            Kind::Fiat,
            1.05,
        ));
        asset_cols.push(col(
            "lower_of_cost_or_market",
            tf("wealth.col.lower_value"),
            Kind::Fiat,
            1.05,
        ));
        asset_cols.push(col(
            "unrealized_pnl",
            tf("wealth.col.unrealized"),
            Kind::Fiat,
            1.0,
        ));
    }
    let asset_table_rows: Vec<Vec<Cell>> = asset_rows_raw
        .iter()
        .map(|row| {
            let mut cells = vec![Cell::Text(text(row, "asset"))];
            if include_regime {
                cells.push(Cell::Text(regime_label(&text(row, "regime"))));
            }
            cells.push(dec_cell(row, "quantity", true));
            cells.push(msat_cell(row, "quantityMsat"));
            if include_valuation {
                let basis_cell = dec_cell(row, "costBasis", false);
                let market_cell = dec_cell(row, "marketValue", false);
                let lower_cached = dec_cell(row, "lowerOfCostOrMarket", false);
                let lower_cell =
                    if !matches!(basis_cell, Cell::Empty) && !matches!(market_cell, Cell::Empty) {
                        Cell::Formula {
                            expression: "MIN({cost_basis},{market_value})".into(),
                            cached: Box::new(lower_cached),
                        }
                    } else {
                        lower_cached
                    };
                cells.push(dec_cell(row, "avgCost", false));
                cells.push(basis_cell);
                cells.push(dec_cell(row, "unitRate", false));
                cells.push(market_cell);
                cells.push(lower_cell);
                cells.push(dec_cell(row, "unrealizedPnl", false));
            }
            cells
        })
        .collect();

    let asset_total = if !asset_table_rows.is_empty() {
        let mut tot = vec![Cell::Text(t("doc.total").into())];
        if include_regime {
            tot.push(Cell::Empty);
        }
        if all_bitcoin {
            tot.push(dec_cell(totals, "quantityBtc", true));
            tot.push(msat_cell(totals, "quantityMsat"));
        } else {
            // Quantities in different units have no sum.
            tot.push(Cell::Empty);
            tot.push(Cell::Empty);
        }
        if include_valuation {
            tot.push(Cell::Empty);
            tot.push(dec_cell(totals, "costBasis", false));
            tot.push(Cell::Empty);
            tot.push(dec_cell(totals, "marketValue", false));
            tot.push(dec_cell(totals, "lowerOfCostOrMarket", false));
            tot.push(dec_cell(totals, "unrealizedPnl", false));
        }
        Some(tot)
    } else {
        None
    };

    sections.push(Section {
        id: "holdings_by_asset",
        title: t("wealth.assets.title").into(),
        lead: assets_lead.into(),
        page_break: false,
        landscape: false,
        blocks: vec![if asset_table_rows.is_empty() {
            Block::Note {
                text: t("wealth.assets.none").into(),
                accent: false,
            }
        } else {
            Block::Table(table(
                "holdings_by_asset",
                String::new(),
                asset_cols,
                asset_table_rows,
                asset_total,
                String::new(),
            ))
        }],
    });

    // 4. Holdings by wallet / custody (optional)
    if include_wallets {
        let wallet_rows_raw = list(report, "walletHoldings");
        let wallets_title = if privacy {
            t("wealth.custody.title")
        } else {
            t("wealth.wallets.title")
        };
        let wallets_lead = if privacy {
            t("wealth.custody.lead")
        } else if !include_valuation {
            t("wealth.wallets.lead_quantity_only")
        } else if per_wallet_basis {
            t("wealth.wallets.lead_wallet")
        } else if is_austrian {
            t("wealth.wallets.lead_global_at")
        } else {
            t("wealth.wallets.lead_global")
        };
        let custody_label = |row: &Value| -> String {
            let class = text(row, "custodyClass");
            match class.as_str() {
                "platform" => t("wealth.custody.platform").to_owned(),
                "self_custody_lightning" => t("wealth.custody.self_custody_lightning").to_owned(),
                "self_custody_liquid" => t("wealth.custody.self_custody_liquid").to_owned(),
                _ => t("wealth.custody.self_custody").to_owned(),
            }
        };
        let mut wallet_cols = vec![
            col("wallet", t("col.wallet"), Kind::Text, 1.4),
            col("wallet_kind", t("wealth.col.wallet_kind"), Kind::Text, 1.1),
            col("asset", t("col.asset"), Kind::Text, 0.55),
            col("quantity_btc", quantity_header, Kind::Btc, 1.2),
            col("quantity_msat", t("col.quantity_msat"), Kind::Msat, 1.0),
            col("share_percent", t("wealth.col.share"), Kind::Percent, 0.65),
        ];
        if include_valuation {
            let basis_label = if per_wallet_basis {
                tf("col.cost_basis")
            } else {
                tf("wealth.col.cost_basis_allocated")
            };
            wallet_cols.push(col("cost_basis", basis_label, Kind::Fiat, 1.25));
            wallet_cols.push(col("market_value", tf("col.market_value"), Kind::Fiat, 1.2));
        }
        let wallet_table_rows: Vec<Vec<Cell>> = wallet_rows_raw
            .iter()
            .map(|row| {
                let wallet_cell = if privacy {
                    let class = text(row, "custodyClass");
                    if class == "platform" {
                        t("privacy.platform").to_owned()
                    } else {
                        t("privacy.self_custody").to_owned()
                    }
                } else {
                    text(row, "wallet")
                };
                let mut cells = vec![
                    Cell::Text(wallet_cell),
                    Cell::Text(custody_label(row)),
                    Cell::Text(text(row, "asset")),
                    dec_cell(row, "quantity", true),
                    msat_cell(row, "quantityMsat"),
                    pct_cell(row, "sharePercent"),
                ];
                if include_valuation {
                    cells.push(dec_cell(row, "costBasis", false));
                    cells.push(dec_cell(row, "marketValue", false));
                }
                cells
            })
            .collect();
        sections.push(Section {
            id: "holdings_by_wallet",
            title: wallets_title.into(),
            lead: wallets_lead.into(),
            page_break: false,
            landscape: false,
            blocks: vec![if wallet_table_rows.is_empty() {
                Block::Note {
                    text: t("wealth.wallets.none").into(),
                    accent: false,
                }
            } else {
                Block::Table(table(
                    "holdings_by_wallet",
                    String::new(),
                    wallet_cols,
                    wallet_table_rows,
                    None,
                    String::new(),
                ))
            }],
        });
    }

    // 5. Valuation sources
    if include_valuation {
        let sources_raw = list(report, "valuationSources");
        if !sources_raw.is_empty() {
            let val_cols = vec![
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col("pair", t("exit.col.pair"), Kind::Code, 0.9),
                col("rate", tf("wealth.col.unit_rate"), Kind::Fiat, 1.1),
                col("source", t("exit.col.source"), Kind::Text, 1.5),
                col("as_of", t("exit.col.as_of"), Kind::Code, 1.4),
            ];
            let val_rows: Vec<Vec<Cell>> = sources_raw
                .iter()
                .map(|row| {
                    let rate_cell = if truthy(value(row, "rateExact")) {
                        decimal(value(row, "rateExact"))
                            .map(Cell::Fiat)
                            .unwrap_or(Cell::Empty)
                    } else {
                        match value(row, "rate") {
                            Value::Null => Cell::Empty,
                            raw => Cell::Fiat(super::from_float(raw)),
                        }
                    };
                    let src = text(row, "source");
                    let provider = text(row, "provider");
                    let src_label = format_rate_source(lang, &src, &provider);
                    let as_of_cell = if truthy(value(row, "asOf")) {
                        Cell::Text(lang.date_time(&text(row, "asOf")))
                    } else {
                        Cell::Empty
                    };
                    vec![
                        Cell::Text(text(row, "asset")),
                        Cell::Text(text(row, "pair")),
                        rate_cell,
                        Cell::Text(src_label),
                        as_of_cell,
                    ]
                })
                .collect();
            sections.push(Section {
                id: "valuation_sources",
                title: t("wealth.valuation.title").into(),
                lead: t("wealth.valuation.lead").into(),
                page_break: false,
                landscape: false,
                blocks: vec![Block::Table(table(
                    "valuation_sources",
                    String::new(),
                    val_cols,
                    val_rows,
                    None,
                    String::new(),
                ))],
            });
        }
    }

    // 6. Draft notes (only shown when the report is a draft or has open items)
    if !clean_final {
        let mut issue_blocks = Vec::new();
        for reason in &draft_reasons {
            issue_blocks.push(Block::Note {
                text: reason.clone(),
                accent: true,
            });
        }
        if !missing_fmv.is_empty() {
            issue_blocks.push(Block::Note {
                text: lang.tf(
                    "wealth.issues.missing_fmv",
                    &[("assets", &missing_fmv.join(", "))],
                ),
                accent: true,
            });
        }
        if issue_blocks.is_empty() {
            issue_blocks.push(Block::Note {
                text: t("draft.notice").into(),
                accent: true,
            });
        }
        sections.push(Section {
            id: "issues",
            title: t("wealth.issues.title").into(),
            lead: String::new(),
            page_break: false,
            landscape: false,
            blocks: issue_blocks,
        });
    }

    // 7. Method and accounting notes
    let mut method_facts = vec![fact(
        "cutoff",
        t("wealth.method.cutoff"),
        Cell::Text(lang.tf("wealth.method.cutoff_value", &[("timezone", &tz)])),
    )];
    if include_valuation {
        method_facts.push(fact(
            "valuation",
            t("wealth.method.valuation"),
            Cell::Text(t("wealth.method.valuation_value").into()),
        ));
        method_facts.push(fact(
            "accounting",
            t("wealth.method.accounting"),
            Cell::Text(t("wealth.method.accounting_value").into()),
        ));
    }
    if is_austrian && include_regime {
        method_facts.push(fact(
            "regime_cutoff",
            t("wealth.method.regime"),
            Cell::Text(t("wealth.method.regime_value").into()),
        ));
    }
    sections.push(Section {
        id: "method",
        title: t("wealth.method.title").into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks: vec![
            Block::Facts(method_facts),
            Block::Note {
                text: t(if is_draft {
                    "wealth.method.disclaimer_draft"
                } else {
                    "wealth.method.disclaimer"
                })
                .into(),
                accent: false,
            },
        ],
    });

    Document {
        kind: "wealth",
        title: t("wealth.title").into(),
        kicker: lang.kicker(&country),
        book,
        period: lang.tf(
            "wealth.period",
            &[("date", &lang.date(&as_of)), ("timezone", &tz)],
        ),
        period_start: as_of.clone(),
        period_end: as_of,
        generated_at: text(model, "generated_at"),
        input_version,
        fiat: currency,
        country,
        lang,
        watermark: is_draft.then(|| t("doc.watermark.draft").into()),
        privacy_mode: privacy,
        sections,
    }
}
