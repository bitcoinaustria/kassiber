//! The full report's stored inputs (holdings, gains, history and summary)
//! and its CSV bundle.
use super::*;
use artifact_csv::{list, File, Prepared};

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

/// The full report's CSV bundle: the shared design's tables over the full
/// report's model, one CSV per table and `manifest.json`, in one ZIP.
pub(super) fn prepare(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let model = full_pdf::model(conn, workspace, profile, args, sources)?;
    let design = report_design::full(&model);
    let tables: Vec<(&str, usize)> = design
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            report_design::Block::Table(t) => Some((t.id, t.rows.len())),
            _ => None,
        })
        .collect();
    let rows: usize = tables.iter().map(|(_, n)| n).sum();
    let wallet = if truthy(value(args, "wallet")) {
        Some(kassiber_wallets::resolve(
            conn,
            &text(profile, "id"),
            &text(args, "wallet"),
        )?)
    } else {
        None
    };
    let scope = match &wallet {
        Some(wallet) => wallet.label.clone(),
        None => text(profile, "label"),
    };
    Ok(Prepared {
        files: vec![File {
            name: "report.zip".into(),
            sheet: String::new(),
            bytes: report_design::csv_zip(&design)?,
            rows,
        }],
        metadata: dict([
            ("title", s(format!("Kassiber Report - {scope}"))),
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
                Value::List(tables.iter().map(|(id, _)| s(*id)).collect()),
            ),
            ("rows", Value::Int(rows.into())),
        ]),
        completion: Some(crate::export_completion::Completion {
            kind: "full-report.csv".into(),
            year: None,
            end_year: None,
            scope: match &wallet {
                Some(wallet) => dict([("wallet_ids", Value::List(vec![s(wallet.id.as_str())]))]),
                None => Value::Null,
            },
        }),
    })
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
