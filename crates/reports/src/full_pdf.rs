//! The full text PDF report (`build_pdf_report_lines`) over the same stored
//! inputs as the full CSV and XLSX. Only presentation happens here; the
//! fixed-pitch document itself comes from `kassiber_pdf::text`.
use super::*;
use artifact_csv::{File, Prepared};
use kassiber_pdf::text::format_table;

/// Python's `,` thousands grouping of an already formatted number.
pub(super) fn grouped(raw: &str) -> String {
    let (whole, fraction) = raw
        .split_once('.')
        .map_or((raw, None), |(a, b)| (a, Some(b)));
    let (sign, digits) = whole.strip_prefix('-').map_or(("", whole), |s| ("-", s));
    let mut out = sign.to_owned();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

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

/// `f"{float(value):,.{places}f}"`.
pub(super) fn fixed(n: f64, places: usize) -> String {
    if !n.is_finite() {
        return format!("{n}").to_lowercase();
    }
    grouped(&format!("{n:.places$}"))
}
fn btc_text(v: &Value) -> Result<String> {
    Ok(fixed(py_float(v)?, 8))
}
fn fiat_text(v: &Value) -> Result<String> {
    Ok(fixed(py_float(v)?, 2))
}
/// `_report_count`: `f"{int(value or 0):,}"`.
fn count(v: &Value) -> String {
    match v {
        Value::Int(n) => grouped(&n.to_string()),
        Value::Bool(b) => u8::from(*b).to_string(),
        _ => "0".into(),
    }
}
fn msat_btc(msat: i64) -> Result<String> {
    Ok(fixed(py_float(&float(&btc(msat)?)?)?, 8))
}
/// `str(value)[:10]`.
fn day(v: &Value) -> String {
    py_str(v).chars().take(10).collect()
}
fn kv(lines: &mut Vec<String>, pairs: Vec<(&str, String)>) {
    for (label, value) in pairs {
        let label = format!("{label}:");
        let padding = " ".repeat(28usize.saturating_sub(label.chars().count()));
        lines.push(format!("{label}{padding} {value}"));
    }
}
fn heading(lines: &mut Vec<String>, title: &str) {
    lines.extend([
        String::new(),
        title.to_owned(),
        "-".repeat(title.chars().count()),
    ]);
}
fn table(
    lines: &mut Vec<String>,
    headers: &[&str],
    rows: Vec<Vec<String>>,
    widths: &[usize],
    right: &[usize],
    empty: &str,
) {
    if rows.is_empty() {
        lines.push(empty.to_owned());
    } else {
        lines.extend(format_table(headers, &rows, widths, right));
    }
}

/// The report title and lines Python hands to `write_text_pdf`.
pub fn model(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(String, Vec<String>)> {
    let context = full_csv::context(conn, workspace, profile, args, sources)?;
    let summary = &context.summary;
    let metrics = value(summary, "metrics");
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
    let title = format!("Kassiber PDF Report - {scope}");
    let mut lines = vec![
        title.clone(),
        "=".repeat(title.chars().count()),
        String::new(),
    ];
    let or_empty = |v: &Value| if truthy(v) { py_str(v) } else { String::new() };
    kv(
        &mut lines,
        vec![
            ("Generated at", context.generated_at.clone()),
            ("Workspace", text(workspace, "label")),
            ("Profile", text(profile, "label")),
            (
                "Wallet scope",
                context
                    .wallet
                    .as_ref()
                    .map(|w| w.label.clone())
                    .unwrap_or_else(|| "All wallets".into()),
            ),
            ("Fiat currency", text(profile, "fiat_currency")),
            ("Tax country", text(profile, "tax_country")),
            ("Tax long-term days", text(profile, "tax_long_term_days")),
            ("Gains algorithm", text(profile, "gains_algorithm")),
            (
                "Last processed at",
                or_empty(value(profile, "last_processed_at")),
            ),
            (
                "Processed tx count",
                count(value(profile, "last_processed_tx_count")),
            ),
        ],
    );
    heading(&mut lines, "Executive Summary");
    let holdings = value(summary, "holdings");
    let realized = value(summary, "realized");
    kv(
        &mut lines,
        vec![
            (
                "Wallets in scope",
                grouped(&context.inventory.len().to_string()),
            ),
            ("Assets in scope", count(value(metrics, "assets_in_scope"))),
            (
                "Transactions (active)",
                count(value(metrics, "active_transactions")),
            ),
            (
                "Transactions (excluded)",
                count(value(metrics, "excluded_transactions")),
            ),
            (
                "Inbound transactions",
                count(value(metrics, "inbound_transactions")),
            ),
            (
                "Outbound transactions",
                count(value(metrics, "outbound_transactions")),
            ),
            ("Journal entries", count(value(metrics, "journal_entries"))),
            ("Quarantines", count(value(metrics, "quarantines"))),
            (
                "Priced transactions",
                count(value(metrics, "priced_transactions")),
            ),
            (
                "Transactions with notes",
                count(value(metrics, "transactions_with_notes")),
            ),
            (
                "Transactions with tags",
                count(value(metrics, "transactions_with_tags")),
            ),
            (
                "First transaction",
                or_empty(value(metrics, "first_transaction_at")),
            ),
            (
                "Last transaction",
                or_empty(value(metrics, "last_transaction_at")),
            ),
            (
                "Holdings cost basis",
                fiat_text(value(holdings, "cost_basis"))?,
            ),
            (
                "Holdings market value",
                fiat_text(value(holdings, "market_value"))?,
            ),
            (
                "Unrealized PnL",
                fiat_text(value(holdings, "unrealized_pnl"))?,
            ),
            ("Realized proceeds", fiat_text(value(realized, "proceeds"))?),
            (
                "Realized cost basis",
                fiat_text(value(realized, "cost_basis"))?,
            ),
            (
                "Realized gain/loss",
                fiat_text(value(realized, "gain_loss"))?,
            ),
        ],
    );

    heading(&mut lines, "Wallet Inventory");
    let wallets = context
        .inventory
        .iter()
        .map(|row| {
            ["wallet", "kind", "chain", "network", "backend", "gap_limit"]
                .iter()
                .map(|key| text(row, key))
                .collect()
        })
        .collect();
    table(
        &mut lines,
        &["Wallet", "Kind", "Chain", "Network", "Backend", "Gap"],
        wallets,
        &[18, 12, 8, 10, 12, 5],
        &[],
        "No wallets in scope.",
    );

    let flow_row = |row: &Value, prefix: &[&str]| -> Result<Vec<String>> {
        let mut cells: Vec<String> = prefix.iter().map(|key| text(row, key)).collect();
        for key in ["tx_count", "inbound_count", "outbound_count"] {
            cells.push(count(value(row, key)));
        }
        for key in [
            "inbound_amount_msat",
            "outbound_amount_msat",
            "fee_amount_msat",
        ] {
            cells.push(msat_btc(int(row, key))?);
        }
        Ok(cells)
    };
    heading(&mut lines, "Asset Flow Summary");
    let asset_flow = artifact_csv::list(value(summary, "asset_flow"))
        .iter()
        .map(|row| flow_row(row, &["asset"]))
        .collect::<Result<_>>()?;
    table(
        &mut lines,
        &["Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees"],
        asset_flow,
        &[6, 6, 6, 6, 14, 14, 14],
        &[1, 2, 3, 4, 5, 6],
        "No active transactions in scope.",
    );

    heading(&mut lines, "Wallet Transaction Metrics");
    let wallet_flow = artifact_csv::list(value(summary, "wallet_flow"))
        .iter()
        .map(|row| flow_row(row, &["wallet", "asset"]))
        .collect::<Result<_>>()?;
    table(
        &mut lines,
        &[
            "Wallet", "Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees",
        ],
        wallet_flow,
        &[18, 6, 6, 6, 6, 14, 14, 14],
        &[2, 3, 4, 5, 6, 7],
        "No wallet transaction metrics available.",
    );

    heading(&mut lines, "Balance Sheet");
    let balances = context
        .balances
        .iter()
        .map(|row| {
            Ok(vec![
                text(row, "account"),
                text(row, "asset"),
                btc_text(value(row, "quantity"))?,
                fiat_text(value(row, "cost_basis"))?,
                fiat_text(value(row, "market_value"))?,
                fiat_text(value(row, "unrealized_pnl"))?,
            ])
        })
        .collect::<Result<_>>()?;
    table(
        &mut lines,
        &[
            "Bucket",
            "Asset",
            "Quantity",
            "Cost Basis",
            "Market Value",
            "Unrealized",
        ],
        balances,
        &[16, 6, 14, 14, 14, 14],
        &[2, 3, 4, 5],
        "No current holdings in scope.",
    );

    heading(&mut lines, "Portfolio Summary");
    let portfolio = context
        .portfolio
        .iter()
        .map(|row| {
            Ok(vec![
                text(row, "wallet"),
                text(row, "account"),
                text(row, "asset"),
                btc_text(value(row, "quantity"))?,
                fiat_text(value(row, "avg_cost"))?,
                fiat_text(value(row, "cost_basis"))?,
                fiat_text(value(row, "market_value"))?,
                fiat_text(value(row, "unrealized_pnl"))?,
            ])
        })
        .collect::<Result<_>>()?;
    table(
        &mut lines,
        &[
            "Wallet",
            "Bucket",
            "Asset",
            "Quantity",
            "Avg Cost",
            "Cost Basis",
            "Market",
            "Unreal.",
        ],
        portfolio,
        &[16, 12, 6, 12, 12, 12, 12, 12],
        &[3, 4, 5, 6, 7],
        "No portfolio rows available.",
    );

    heading(&mut lines, "Capital Gains Summary");
    if context.capital.is_empty() {
        lines.push("No realized disposals in scope.".into());
    } else {
        // Python groups into an insertion-ordered dict, summing floats in row order.
        let mut buckets: Vec<((Value, Value), usize, [f64; 3])> = Vec::new();
        for row in &context.capital {
            let key = (value(row, "wallet").clone(), value(row, "asset").clone());
            let index = match buckets.iter().position(|(k, _, _)| *k == key) {
                Some(index) => index,
                None => {
                    buckets.push((key, 0, [0.0; 3]));
                    buckets.len() - 1
                }
            };
            buckets[index].1 += 1;
            for (i, field) in ["proceeds", "cost_basis", "gain_loss"].iter().enumerate() {
                buckets[index].2[i] += py_float(value(row, field))?;
            }
        }
        let summary_rows = buckets
            .iter()
            .map(|((wallet, asset), rows, sums)| {
                vec![
                    py_str(wallet),
                    py_str(asset),
                    grouped(&rows.to_string()),
                    fixed(sums[0], 2),
                    fixed(sums[1], 2),
                    fixed(sums[2], 2),
                ]
            })
            .collect::<Vec<_>>();
        lines.extend(format_table(
            &[
                "Wallet",
                "Asset",
                "Rows",
                "Proceeds",
                "Cost Basis",
                "Gain/Loss",
            ],
            &summary_rows,
            &[16, 6, 6, 14, 14, 14],
            &[2, 3, 4, 5],
        ));
        heading(&mut lines, "Capital Gains Detail");
        let detail = context
            .capital
            .iter()
            .map(|row| {
                Ok(vec![
                    day(value(row, "occurred_at")),
                    text(row, "wallet"),
                    text(row, "asset"),
                    btc_text(value(row, "quantity"))?,
                    fiat_text(value(row, "proceeds"))?,
                    fiat_text(value(row, "cost_basis"))?,
                    fiat_text(value(row, "gain_loss"))?,
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        lines.extend(format_table(
            &[
                "Date",
                "Wallet",
                "Asset",
                "Qty",
                "Proceeds",
                "Basis",
                "Gain/Loss",
            ],
            &detail,
            &[10, 16, 6, 12, 12, 12, 12],
            &[3, 4, 5, 6],
        ));
    }

    heading(&mut lines, "Balance History");
    let history = context
        .history
        .iter()
        .map(|row| {
            Ok(vec![
                day(value(row, "period_start")),
                text(row, "asset"),
                btc_text(value(row, "quantity"))?,
                fiat_text(value(row, "cumulative_cost_basis"))?,
                fiat_text(value(row, "market_value"))?,
            ])
        })
        .collect::<Result<_>>()?;
    table(
        &mut lines,
        &[
            "Period Start",
            "Asset",
            "Quantity",
            "Cost Basis",
            "Market Value",
        ],
        history,
        &[12, 6, 14, 14, 14],
        &[2, 3, 4],
        "No balance history rows available.",
    );

    heading(&mut lines, "Data Quality");
    let quality = quarantines
        .iter()
        .map(|row| vec![text(row, "reason"), count(value(row, "count"))])
        .collect();
    table(
        &mut lines,
        &["Reason", "Count"],
        quality,
        &[28, 10],
        &[1],
        "No quarantined transactions.",
    );

    heading(&mut lines, "Transactions");
    let transactions = transactions
        .iter()
        .map(|row| {
            Ok(vec![
                day(value(row, "occurred_at")),
                text(row, "wallet"),
                text(row, "direction").chars().take(3).collect(),
                text(row, "asset"),
                msat_btc(int(row, "amount_msat"))?,
                msat_btc(int(row, "fee_msat"))?,
                text(row, "description"),
            ])
        })
        .collect::<Result<_>>()?;
    table(
        &mut lines,
        &[
            "Date",
            "Wallet",
            "Dir",
            "Asset",
            "Amount",
            "Fee",
            "Description",
        ],
        transactions,
        &[10, 14, 3, 6, 12, 12, 28],
        &[4, 5],
        "No transactions in scope.",
    );
    Ok((title, lines))
}

pub(super) fn prepare(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (title, lines) = model(conn, workspace, profile, args, sources)?;
    let rendered = kassiber_pdf::text::render(&title, &lines);
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
            bytes: rendered.bytes,
            rows: 0,
        }],
        metadata: dict([
            ("pages", Value::Int(rendered.pages.into())),
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
