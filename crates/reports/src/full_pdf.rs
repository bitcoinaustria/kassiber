//! The full report PDF in the shared report design, over the same stored
//! inputs as the full CSV and XLSX (`_build_full_report_context`). The model
//! keeps every value exact and unformatted; the design renders it.
use super::*;
use artifact_csv::{File, Prepared};

/// `float(value)`, failing like Python for values it cannot convert.
pub(super) fn py_float(v: &Value) -> Result<f64> {
    match v {
        Value::Float(n) => Ok(*n),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        Value::Int(n) => n
            .to_string()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| AppError::new("internal_error", "int too large to convert to float")),
        Value::Decimal(d) => Ok(d.to_canonical().parse().unwrap_or(f64::NAN)),
        Value::Str(raw) => kassiber_pytext::parse_float(&py_str(v)).map_err(|_| {
            AppError::new(
                "internal_error",
                format!(
                    "could not convert string to float: {}",
                    kassiber_pytext::repr_py(raw)
                ),
            )
        }),
        other => Err(AppError::new(
            "internal_error",
            format!(
                "float() argument must be a string or a real number, not '{}'",
                kassiber_envelope::type_name(other)
            ),
        )),
    }
}

/// `int(value or 0)` for a stored count or msat sum.
fn counted(v: &Value) -> Value {
    match v {
        Value::Int(_) => v.clone(),
        Value::Bool(b) => Value::Int(i64::from(*b).into()),
        _ => Value::Int(0.into()),
    }
}
/// `str(value)`: the text the report prints for a stored value.
fn texts(row: &Value, keys: &[(&str, &str)]) -> Vec<(String, Value)> {
    keys.iter()
        .map(|(name, key)| (name.to_string(), s(text(row, key))))
        .collect()
}
fn raw(row: &Value, keys: &[&str]) -> Vec<(String, Value)> {
    keys.iter()
        .map(|key| (key.to_string(), value(row, key).clone()))
        .collect()
}
fn record(parts: Vec<Vec<(String, Value)>>) -> Value {
    dict(parts.into_iter().flatten())
}

/// The full report's model: the values Python's full report prints, in its
/// order, exact and unformatted.
pub fn model(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let context = full_csv::context(conn, workspace, profile, args, sources)?;
    let summary = &context.summary;
    let wallet_id = context.wallet.as_ref().map(|w| w.id.as_str());
    let quarantines = rows(
        conn,
        "SELECT jq.reason,COUNT(*) AS count FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id WHERE t.profile_id=?1 AND (?2 IS NULL OR t.wallet_id=?2) GROUP BY jq.reason ORDER BY count DESC,jq.reason ASC",
        kassiber_store::rusqlite::params![text(profile, "id"), wallet_id],
    )?;
    let transactions = transaction_csv::report_rows(conn, profile, wallet_id)?;
    let scope = match &context.wallet {
        Some(wallet) => wallet.label.clone(),
        None => text(profile, "label"),
    };
    let metrics = value(summary, "metrics");
    let mut counts = vec![(
        "wallets_in_scope".to_string(),
        Value::Int(context.inventory.len().into()),
    )];
    for key in [
        "assets_in_scope",
        "active_transactions",
        "excluded_transactions",
        "inbound_transactions",
        "outbound_transactions",
        "journal_entries",
        "quarantines",
        "priced_transactions",
        "transactions_with_notes",
        "transactions_with_tags",
    ] {
        counts.push((key.into(), counted(value(metrics, key))));
    }
    for key in ["first_transaction_at", "last_transaction_at"] {
        let v = value(metrics, key);
        counts.push((key.into(), if truthy(v) { v.clone() } else { s("") }));
    }
    let pick = |v: &Value, keys: &[&str]| dict(raw(v, keys));
    let flow = |rows: &Value, prefix: &[&str]| -> Value {
        Value::List(
            artifact_csv::list(rows)
                .iter()
                .map(|row| {
                    let mut fields: Vec<(String, Value)> = prefix
                        .iter()
                        .map(|key| (key.to_string(), s(text(row, key))))
                        .collect();
                    for key in ["tx_count", "inbound_count", "outbound_count"] {
                        fields.push((key.into(), counted(value(row, key))));
                    }
                    for (name, key) in [
                        ("inbound_msat", "inbound_amount_msat"),
                        ("outbound_msat", "outbound_amount_msat"),
                        ("fee_msat", "fee_amount_msat"),
                    ] {
                        fields.push((name.into(), counted(value(row, key))));
                    }
                    dict(fields)
                })
                .collect(),
        )
    };
    let table = |rows: &[Value], names: &[(&str, &str)], amounts: &[&str]| -> Value {
        Value::List(
            rows.iter()
                .map(|row| record(vec![texts(row, names), raw(row, amounts)]))
                .collect(),
        )
    };
    let mut profile_fields = raw(
        profile,
        &[
            "label",
            "fiat_currency",
            "tax_country",
            "tax_long_term_days",
            "gains_algorithm",
            "last_processed_at",
        ],
    );
    profile_fields.push((
        "last_processed_tx_count".into(),
        counted(value(profile, "last_processed_tx_count")),
    ));
    profile_fields.push((
        "journal_input_version".into(),
        counted(value(profile, "journal_input_version")),
    ));
    Ok(dict([
        ("title", s(format!("Kassiber PDF Report - {scope}"))),
        ("generated_at", s(context.generated_at.as_str())),
        ("workspace", value(workspace, "label").clone()),
        ("profile", dict(profile_fields)),
        (
            "wallet",
            context
                .wallet
                .as_ref()
                .map(|w| s(w.label.as_str()))
                .unwrap_or(Value::Null),
        ),
        ("metrics", dict(counts)),
        (
            "rollups",
            dict([
                (
                    "holdings",
                    pick(
                        value(summary, "holdings"),
                        &["cost_basis", "market_value", "unrealized_pnl"],
                    ),
                ),
                (
                    "realized",
                    pick(
                        value(summary, "realized"),
                        &["proceeds", "cost_basis", "gain_loss"],
                    ),
                ),
            ]),
        ),
        (
            "wallets",
            table(
                &context.inventory,
                &[
                    ("label", "wallet"),
                    ("kind", "kind"),
                    ("chain", "chain"),
                    ("network", "network"),
                    ("backend", "backend"),
                    ("gap_limit", "gap_limit"),
                ],
                &[],
            ),
        ),
        ("asset_flow", flow(value(summary, "asset_flow"), &["asset"])),
        (
            "wallet_flow",
            flow(value(summary, "wallet_flow"), &["wallet", "asset"]),
        ),
        (
            "balances",
            table(
                &context.balances,
                &[("account", "account"), ("asset", "asset")],
                &["quantity", "cost_basis", "market_value", "unrealized_pnl"],
            ),
        ),
        (
            "portfolio",
            table(
                &context.portfolio,
                &[
                    ("wallet", "wallet"),
                    ("account", "account"),
                    ("asset", "asset"),
                ],
                &[
                    "quantity",
                    "avg_cost",
                    "cost_basis",
                    "market_value",
                    "unrealized_pnl",
                ],
            ),
        ),
        (
            "capital",
            table(
                &context.capital,
                &[
                    ("occurred_at", "occurred_at"),
                    ("wallet", "wallet"),
                    ("asset", "asset"),
                    // Income rows sit beside disposals; the journal says which.
                    ("entry_type", "entry_type"),
                ],
                &["quantity", "proceeds", "cost_basis", "gain_loss"],
            ),
        ),
        (
            "history",
            table(
                &context.history,
                &[("period_start", "period_start"), ("asset", "asset")],
                &["quantity", "cumulative_cost_basis", "market_value"],
            ),
        ),
        (
            "quarantines",
            Value::List(
                quarantines
                    .iter()
                    .map(|row| {
                        dict([
                            ("reason", s(text(row, "reason"))),
                            ("count", counted(value(row, "count"))),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "transactions",
            Value::List(
                transactions
                    .iter()
                    .map(|row| {
                        let mut fields = texts(
                            row,
                            &[
                                ("occurred_at", "occurred_at"),
                                ("wallet", "wallet"),
                                ("direction", "direction"),
                                ("asset", "asset"),
                            ],
                        );
                        fields.push(("amount_msat".into(), counted(value(row, "amount_msat"))));
                        fields.push(("fee_msat".into(), counted(value(row, "fee_msat"))));
                        fields.extend(texts(row, &[("description", "description")]));
                        dict(fields)
                    })
                    .collect(),
            ),
        ),
    ]))
}

pub(super) fn prepare(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let model = model(conn, workspace, profile, args, sources)?;
    let (bytes, pages) = report_design::pdf(&report_design::full(&model))?;
    let title = text(&model, "title");
    let wallet = if truthy(value(args, "wallet")) {
        Some(kassiber_wallets::resolve(
            conn,
            &text(profile, "id"),
            &text(args, "wallet"),
        )?)
    } else {
        None
    };
    Ok(Prepared {
        files: vec![File {
            name: "report.pdf".into(),
            sheet: String::new(),
            bytes,
            rows: 0,
        }],
        metadata: dict([
            ("pages", Value::Int(pages.into())),
            ("title", s(title)),
            (
                "wallet",
                if truthy(value(args, "wallet")) {
                    value(args, "wallet").clone()
                } else {
                    s("")
                },
            ),
        ]),
        completion: Some(crate::export_completion::Completion {
            kind: "full-report.pdf".into(),
            year: None,
            end_year: None,
            scope: match &wallet {
                Some(wallet) => dict([("wallet_ids", Value::List(vec![s(wallet.id.as_str())]))]),
                None => Value::Null,
            },
        }),
    })
}
