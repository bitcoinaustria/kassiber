//! Full-report CSV uses the same stored holdings, gains, history and summary.
use super::*;
use artifact_csv::{list, File, Prepared, Section};

fn number(row: &Value, key: &str) -> f64 {
    py_str(value(row, key)).parse().unwrap_or(0.0)
}
pub(super) fn aggregate(portfolio: &[Value]) -> Result<Vec<Value>> {
    let mut grouped: Vec<Value> = Vec::new();
    for row in portfolio {
        let index = grouped.iter().position(|r| {
            value(r, "account") == value(row, "account") && value(r, "asset") == value(row, "asset")
        });
        let index = index.unwrap_or_else(|| {
            grouped.push(dict([
                ("account", value(row, "account").clone()),
                ("asset", value(row, "asset").clone()),
            ]));
            grouped.len() - 1
        });
        for key in ["quantity", "cost_basis", "market_value", "unrealized_pnl"] {
            let sum = number(&grouped[index], key) + number(row, key);
            replace(&mut grouped[index], key, Value::Float(sum));
        }
    }
    grouped.sort_by_key(|row| {
        (
            artifact_csv::cell(value(row, "account")),
            artifact_csv::cell(value(row, "asset")),
        )
    });
    for row in &mut grouped {
        // Python round-trips the summed quantity float through Decimal(str()).
        let msat = decimal(value(row, "quantity"))?
            .mul(&Decimal::from(100_000_000_000_i64), &DECIMAL)
            .map_err(arithmetic)?;
        let msat = kassiber_pytext::parse_int(
            &msat
                .to_fixed_places(0, Rounding::HalfUp)
                .map_err(arithmetic)?,
        )
        .map_err(|_| AppError::new("internal_error", "invalid report quantity"))?;
        let n: f64 = msat
            .to_string()
            .parse()
            .map_err(|_| AppError::new("internal_error", "invalid report quantity"))?;
        replace(row, "quantity_sat", Value::Float(n / 1000.0));
        replace(row, "quantity_msat", Value::Int(msat));
    }
    Ok(grouped)
}

pub(super) fn inventory(
    conn: &Connection,
    profile: &str,
    wallet: Option<&str>,
) -> Result<Vec<Value>> {
    Ok(listed(conn, profile, wallet)?
        .into_iter()
        .map(|(_, row)| row)
        .collect())
}

/// `list_wallets` with each row's id, validating every wallet's configuration
/// before a scope applies, as Python does.
pub(super) fn listed(
    conn: &Connection,
    profile: &str,
    wallet: Option<&str>,
) -> Result<Vec<(String, Value)>> {
    let mut result = Vec::new();
    for row in rows(
        conn,
        "SELECT id,label,kind,config_json FROM wallets WHERE profile_id=? ORDER BY label",
        [profile],
    )? {
        let raw = value(&row, "config_json");
        let raw = if truthy(raw) {
            py_str(raw)
        } else {
            "{}".into()
        };
        let config = kassiber_pytext::loads(&raw)
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        if !matches!(config, Value::Dict(_)) {
            let message = if matches!(config, Value::List(_)) {
                "pop expected at most 1 argument, got 2".into()
            } else {
                format!(
                    "'{}' object has no attribute 'pop'",
                    kassiber_envelope::type_name(&config)
                )
            };
            return Err(AppError::new("internal_error", message));
        }
        let mut chain = String::new();
        let mut network = String::new();
        let mut descriptor = false;
        if [
            "descriptor",
            "change_descriptor",
            "xpub",
            "addresses",
            "sp_descriptor",
            "chain",
            "network",
        ]
        .iter()
        .any(|key| truthy(value(&config, key)))
        {
            chain = kassiber_wallets::descriptors::normalize_chain(&artifact_csv::cell(value(
                &config, "chain",
            )))
            .map_err(descriptor_error)?;
            network = kassiber_wallets::descriptors::normalize_network(
                &chain,
                &artifact_csv::cell(value(&config, "network")),
            )
            .map_err(descriptor_error)?;
        }
        if truthy(value(&config, "descriptor")) || truthy(value(&config, "xpub")) {
            let selected = dict(
                [
                    "descriptor",
                    "change_descriptor",
                    "xpub",
                    "script_types",
                    "synthesize_change",
                    "descriptor_source",
                    "chain",
                    "network",
                    "gap_limit",
                ]
                .into_iter()
                .filter_map(|k| get(&config, k).map(|v| (k, v.clone()))),
            );
            let encoded = kassiber_pyjson::dumps(&selected, &Default::default())
                .map_err(|e| AppError::new("internal_error", e.to_string()))?;
            let loaded = serde_json::from_str(&encoded)
                .ok()
                .map(|config| kassiber_wallets::descriptors::load(&config));
            match loaded {
                Some(Ok(Some(plan))) => {
                    chain = plan.chain;
                    network = plan.network;
                    descriptor = true;
                }
                Some(Ok(None)) => (),
                Some(Err(kassiber_wallets::descriptors::Error::App(error)))
                    if error.code != "dependency_missing" =>
                {
                    return Err(error)
                }
                _ => descriptor = true,
            }
        }
        // Python lists/validates every wallet before applying report scope.
        if wallet.is_some_and(|id| text(&row, "id") != id) {
            continue;
        }
        result.push((
            text(&row, "id"),
            dict([
                ("wallet", value(&row, "label").clone()),
                ("kind", value(&row, "kind").clone()),
                ("chain", s(chain)),
                ("network", s(network)),
                (
                    "backend",
                    get(&config, "backend").cloned().unwrap_or_else(|| s("")),
                ),
                (
                    "gap_limit",
                    get(&config, "gap_limit").cloned().unwrap_or_else(|| {
                        if descriptor {
                            Value::Int(100.into())
                        } else {
                            s("")
                        }
                    }),
                ),
            ]),
        ));
    }
    Ok(result)
}

fn descriptor_error(error: kassiber_wallets::descriptors::Error) -> AppError {
    match error {
        kassiber_wallets::descriptors::Error::Value(message) => {
            AppError::new("internal_error", message)
        }
        kassiber_wallets::descriptors::Error::App(error) => error,
    }
}

pub(super) fn prepare(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    project(conn, workspace, profile, args, sources).map(|(_, prepared)| prepared)
}

/// The full report's stored inputs, read in Python's order so the first
/// failing read is the one Python reports. Shared by the CSV, XLSX and PDF.
pub(super) struct Context {
    pub wallet: Option<kassiber_wallets::Wallet>,
    pub inventory: Vec<Value>,
    pub summary: Value,
    pub portfolio: Vec<Value>,
    pub capital: Vec<Value>,
    pub balances: Vec<Value>,
    pub generated_at: String,
    pub history: Vec<Value>,
}

pub(super) fn context(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Context> {
    let id = text(profile, "id");
    let wallet = if truthy(value(args, "wallet")) {
        Some(kassiber_wallets::resolve(conn, &id, &text(args, "wallet"))?)
    } else {
        None
    };
    let wallet_id = wallet.as_ref().map(|w| w.id.as_str());
    let inventory = inventory(conn, &id, wallet_id)?;
    let summary = summary::report(conn, workspace, profile, args)?;
    let mut portfolio = list(&holdings::report(conn, profile, true)?).to_vec();
    let mut capital = list(&capital_gains(conn, profile)?).to_vec();
    if let Some(wallet) = &wallet {
        portfolio.retain(|r| text(r, "wallet") == wallet.label);
        capital.retain(|r| text(r, "wallet") == wallet.label);
    }
    let balances = aggregate(&portfolio)?;
    let generated_at = sources.now(conn)?;
    let mut history = history::report(
        conn,
        profile,
        &dict([
            ("wallet", wallet_id.map(s).unwrap_or(Value::Null)),
            ("end", s(generated_at.as_str())),
        ]),
    )?;
    if let Value::Int(limit) = value(args, "history_limit") {
        if limit > &0.into() {
            history.drain(
                ..history
                    .len()
                    .saturating_sub(usize::try_from(limit.clone()).unwrap_or(usize::MAX)),
            );
        }
    }
    Ok(Context {
        wallet,
        inventory,
        summary,
        portfolio,
        capital,
        balances,
        generated_at,
        history,
    })
}

pub(super) fn project(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(Vec<Section>, Prepared)> {
    let id = text(profile, "id");
    let Context {
        wallet,
        inventory,
        summary,
        portfolio,
        capital,
        balances,
        generated_at,
        history,
    } = context(conn, workspace, profile, args, sources)?;
    let wallet_id = wallet.as_ref().map(|w| w.id.as_str());
    let mut overview = Vec::new();
    let mut add = |group: &str, field: &str, v: Value| {
        overview.push(dict([
            ("group", s(group)),
            ("field", s(field)),
            ("value", v),
        ]))
    };
    add("Report metadata", "Generated at", s(generated_at));
    for (field, key) in [("Workspace", "workspace"), ("Profile", "profile")] {
        add("Report metadata", field, value(&summary, key).clone());
    }
    add(
        "Report metadata",
        "Wallet scope",
        wallet
            .as_ref()
            .map(|w| s(w.label.as_str()))
            .unwrap_or_else(|| s("All wallets")),
    );
    for (field, key) in [
        ("Fiat currency", "fiat_currency"),
        ("Tax country", "tax_country"),
        ("Tax long-term days", "tax_long_term_days"),
        ("Gains algorithm", "gains_algorithm"),
        ("Last processed at", "last_processed_at"),
        ("Processed tx count", "processed_tx_count"),
    ] {
        add("Report metadata", field, value(&summary, key).clone());
    }
    for (field, key) in [
        ("Wallets in scope", "wallets_in_scope"),
        ("Assets in scope", "assets_in_scope"),
        ("Transactions active", "active_transactions"),
        ("Transactions excluded", "excluded_transactions"),
        ("Inbound transactions", "inbound_transactions"),
        ("Outbound transactions", "outbound_transactions"),
        ("Journal entries", "journal_entries"),
        ("Quarantines", "quarantines"),
        ("Priced transactions", "priced_transactions"),
        ("Transactions with notes", "transactions_with_notes"),
        ("Transactions with tags", "transactions_with_tags"),
        ("First transaction", "first_transaction_at"),
        ("Last transaction", "last_transaction_at"),
    ] {
        add(
            "Executive summary",
            field,
            value(value(&summary, "metrics"), key).clone(),
        );
    }
    for (field, key) in [
        ("Holdings cost basis", "cost_basis"),
        ("Holdings market value", "market_value"),
        ("Unrealized PnL", "unrealized_pnl"),
    ] {
        add(
            "Financial summary",
            field,
            value(value(&summary, "holdings"), key).clone(),
        );
    }
    for (field, key) in [
        ("Realized proceeds", "proceeds"),
        ("Realized cost basis", "cost_basis"),
        ("Realized gain/loss", "gain_loss"),
    ] {
        add(
            "Financial summary",
            field,
            value(value(&summary, "realized"), key).clone(),
        );
    }
    let mut capital_summary: Vec<Value> = Vec::new();
    for row in &capital {
        let index = capital_summary.iter().position(|r| {
            value(r, "wallet") == value(row, "wallet") && value(r, "asset") == value(row, "asset")
        });
        let index = index.unwrap_or_else(|| {
            capital_summary.push(dict([
                ("wallet", value(row, "wallet").clone()),
                ("asset", value(row, "asset").clone()),
            ]));
            capital_summary.len() - 1
        });
        let bucket = &mut capital_summary[index];
        let n = int(bucket, "rows") + 1;
        replace(bucket, "rows", Value::Int(n.into()));
        for key in ["proceeds", "cost_basis", "gain_loss"] {
            let sum = number(bucket, key) + number(row, key);
            replace(bucket, key, Value::Float(sum));
        }
    }
    capital_summary.sort_by_key(|r| (text(r, "wallet"), text(r, "asset")));
    let mut quality=rows(conn,"SELECT jq.reason,COUNT(*) AS count,t.wallet_id FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id WHERE t.profile_id=? AND (?2 IS NULL OR t.wallet_id=?2) GROUP BY jq.reason ORDER BY count DESC,jq.reason",kassiber_store::rusqlite::params![id,wallet_id])?;
    for row in &mut quality {
        if let Value::Dict(fields) = row {
            fields.retain(|(k, _)| *k != "wallet_id".into());
        }
    }
    let sections = vec![
        Section {
            name: "Overview",
            title: "Overview".into(),
            headers: &["group", "field", "value"],
            rows: overview,
        },
        Section {
            name: "Wallets",
            title: "Wallet Inventory".into(),
            headers: &["wallet", "kind", "chain", "network", "backend", "gap_limit"],
            rows: inventory,
        },
        Section {
            name: "Asset Flow",
            title: "Asset Flow Summary".into(),
            headers: &[
                "asset",
                "tx_count",
                "inbound_count",
                "outbound_count",
                "inbound_amount",
                "inbound_amount_sat",
                "inbound_amount_msat",
                "outbound_amount",
                "outbound_amount_sat",
                "outbound_amount_msat",
                "fee_amount",
                "fee_amount_sat",
                "fee_amount_msat",
            ],
            rows: list(value(&summary, "asset_flow")).to_vec(),
        },
        Section {
            name: "Wallet Metrics",
            title: "Wallet Transaction Metrics".into(),
            headers: &[
                "wallet",
                "asset",
                "tx_count",
                "inbound_count",
                "outbound_count",
                "inbound_amount",
                "inbound_amount_sat",
                "inbound_amount_msat",
                "outbound_amount",
                "outbound_amount_sat",
                "outbound_amount_msat",
                "fee_amount",
                "fee_amount_sat",
                "fee_amount_msat",
                "first_transaction_at",
                "last_transaction_at",
            ],
            rows: list(value(&summary, "wallet_flow")).to_vec(),
        },
        Section {
            name: "Transfers & Swaps",
            title: "Reviewed Transfers and Swaps".into(),
            headers: &[
                "pair_id",
                "pair_type",
                "kind",
                "policy",
                "swap_fee",
                "swap_fee_msat",
                "swap_fee_kind",
                "out_occurred_at",
                "out_wallet",
                "out_transaction_id",
                "out_asset",
                "out_amount",
                "out_amount_msat",
                "out_fee",
                "out_fee_msat",
                "in_occurred_at",
                "in_wallet",
                "in_transaction_id",
                "in_asset",
                "in_amount",
                "in_amount_msat",
                "in_fee",
                "in_fee_msat",
                "notes",
                "created_at",
            ],
            rows: list(value(&summary, "transfer_pairs")).to_vec(),
        },
        Section {
            name: "Balance Sheet",
            title: "Balance Sheet".into(),
            headers: &[
                "account",
                "asset",
                "quantity",
                "quantity_sat",
                "quantity_msat",
                "cost_basis",
                "market_value",
                "unrealized_pnl",
            ],
            rows: balances,
        },
        Section {
            name: "Portfolio",
            title: "Portfolio Summary".into(),
            headers: &[
                "wallet",
                "account",
                "asset",
                "quantity",
                "quantity_sat",
                "quantity_msat",
                "avg_cost",
                "cost_basis",
                "market_value",
                "unrealized_pnl",
            ],
            rows: portfolio,
        },
        Section {
            name: "Capital Summary",
            title: "Capital Gains Summary".into(),
            headers: &[
                "wallet",
                "asset",
                "rows",
                "proceeds",
                "cost_basis",
                "gain_loss",
            ],
            rows: capital_summary,
        },
        Section {
            name: "Capital Detail",
            title: "Capital Gains Detail".into(),
            headers: &[
                "occurred_at",
                "wallet",
                "transaction_id",
                "entry_type",
                "asset",
                "quantity",
                "quantity_msat",
                "proceeds",
                "cost_basis",
                "gain_loss",
                "description",
                "at_category",
                "at_kennzahl",
            ],
            rows: capital,
        },
        Section {
            name: "Balance History",
            title: "Balance History".into(),
            headers: &[
                "period_start",
                "period_end",
                "asset",
                "quantity",
                "cumulative_cost_basis",
                "market_value",
            ],
            rows: history,
        },
        Section {
            name: "Data Quality",
            title: "Data Quality".into(),
            headers: &["reason", "count"],
            rows: quality,
        },
        Section {
            name: "Transactions",
            title: "Transactions".into(),
            headers: transaction_csv::HEADERS,
            rows: transaction_csv::report_rows(conn, profile, wallet_id)?,
        },
    ];
    let title = format!(
        "Kassiber Report - {}",
        wallet
            .as_ref()
            .map(|w| w.label.as_str())
            .unwrap_or(&text(profile, "label"))
    );
    let count: usize = sections.iter().map(|s| s.rows.len()).sum();
    let prepared = Prepared {
        files: vec![File {
            name: "report.csv".into(),
            sheet: String::new(),
            bytes: artifact_csv::document(&title, &sections),
            rows: count,
        }],
        metadata: dict([
            ("title", s(title)),
            (
                "wallet",
                if truthy(value(args, "wallet")) {
                    value(args, "wallet").clone()
                } else {
                    s("")
                },
            ),
            (
                "sections",
                Value::List(sections.iter().map(|section| s(section.name)).collect()),
            ),
            ("rows", Value::Int(count.into())),
        ]),
        completion: Some(crate::export_completion::Completion {
            kind: "full-report.csv".into(),
            year: None,
            scope: match &wallet {
                Some(wallet) => dict([("wallet_ids", Value::List(vec![s(wallet.id.as_str())]))]),
                None => Value::Null,
            },
        }),
    };
    Ok((sections, prepared))
}
