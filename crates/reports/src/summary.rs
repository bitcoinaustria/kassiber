use super::*;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql};
#[path = "summary_queries.rs"]
mod queries;

fn number(row: &Value, key: &str) -> f64 {
    match value(row, key) {
        Value::Float(n) => *n,
        Value::Int(n) => n.to_string().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}
fn project(row: &Value, keys: &[&str]) -> Value {
    dict(keys.iter().map(|k| (*k, value(row, k).clone())))
}
fn pairs(conn: &Connection, profile: &str, wallet: Option<&Value>) -> Result<Value> {
    let mut args = vec![Sql::Text(profile.into())];
    let mut where_sql = "p.profile_id=?".to_owned();
    if let Some(wallet) = wallet {
        where_sql += " AND (tout.wallet_id=? OR tin.wallet_id=?)";
        args.extend([Sql::Text(text(wallet, "id")), Sql::Text(text(wallet, "id"))]);
    }
    let paired = rows(
        conn,
        &queries::PAIRS.replace("{pair_where}", &where_sql),
        params_from_iter(args),
    )?;
    let mut args = vec![Sql::Text(profile.into())];
    let mut where_sql = "p.profile_id=?".to_owned();
    if let Some(wallet) = wallet {
        where_sql += " AND tout.wallet_id=?";
        args.push(Sql::Text(text(wallet, "id")));
    }
    let direct = rows(
        conn,
        &queries::DIRECT.replace("{direct_payout_where}", &where_sql),
        params_from_iter(args),
    )?;
    let mut results = Vec::new();
    for (row, direct) in paired
        .into_iter()
        .map(|r| (r, false))
        .chain(direct.into_iter().map(|r| (r, true)))
    {
        let swap = int(&row, "swap_fee_msat");
        let out = int(&row, "out_amount");
        let out_fee = int(&row, "out_fee");
        let amount = int(&row, if direct { "payout_amount" } else { "in_amount" });
        let in_fee = if direct { 0 } else { int(&row, "in_fee") };
        let fallback = |key, fallback: Value| {
            if truthy(value(&row, key)) {
                value(&row, key).clone()
            } else {
                fallback
            }
        };
        results.push(dict([
            ("pair_id", value(&row, "id").clone()),
            (
                "pair_type",
                s(if direct {
                    "direct_swap_payout"
                } else if value(&row, "out_asset") == value(&row, "in_asset") {
                    "transfer"
                } else {
                    "swap"
                }),
            ),
            ("kind", value(&row, "kind").clone()),
            ("policy", value(&row, "policy").clone()),
            ("swap_fee", float(&btc(swap)?)?),
            ("swap_fee_msat", Value::Int(swap.into())),
            ("swap_fee_kind", value(&row, "swap_fee_kind").clone()),
            ("out_occurred_at", value(&row, "out_occurred_at").clone()),
            ("out_wallet", value(&row, "out_wallet").clone()),
            (
                "out_transaction_id",
                value(&row, "out_transaction_id").clone(),
            ),
            ("out_asset", value(&row, "out_asset").clone()),
            ("out_amount", float(&btc(out)?)?),
            ("out_amount_msat", Value::Int(out.into())),
            ("out_fee", float(&btc(out_fee)?)?),
            ("out_fee_msat", Value::Int(out_fee.into())),
            (
                "in_occurred_at",
                if direct {
                    fallback("payout_occurred_at", value(&row, "out_occurred_at").clone())
                } else {
                    value(&row, "in_occurred_at").clone()
                },
            ),
            (
                "in_wallet",
                if direct {
                    fallback("counterparty", s("external"))
                } else {
                    value(&row, "in_wallet").clone()
                },
            ),
            (
                "in_transaction_id",
                if direct {
                    fallback("payout_external_id", s(""))
                } else {
                    value(&row, "in_transaction_id").clone()
                },
            ),
            (
                "in_asset",
                value(&row, if direct { "payout_asset" } else { "in_asset" }).clone(),
            ),
            ("in_amount", float(&btc(amount)?)?),
            ("in_amount_msat", Value::Int(amount.into())),
            ("in_fee", float(&btc(in_fee)?)?),
            ("in_fee_msat", Value::Int(in_fee.into())),
            ("notes", value(&row, "notes").clone()),
            ("created_at", value(&row, "created_at").clone()),
        ]));
    }
    Ok(Value::List(results))
}

pub fn report(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
) -> Result<Value> {
    let id = text(profile, "id");
    let wallet = if truthy(value(args, "wallet")) {
        let raw = text(args, "wallet");
        let selector = kassiber_pytext::strip(&raw);
        let query = "SELECT w.*,a.code AS account_code,a.label AS account_label FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id WHERE w.profile_id=? AND ";
        let exact = rows(
            conn,
            &format!("{query}w.id=? LIMIT 1"),
            [id.as_str(), selector],
        )?;
        if let Some(row) = exact.into_iter().next() {
            Some(row)
        } else {
            let mut found = rows(
                conn,
                &format!("{query}lower(w.label)=lower(?) ORDER BY w.label ASC,w.id ASC"),
                [id.as_str(), selector],
            )?;
            if found.len() > 1 {
                return Err(AppError::new(
                    "validation",
                    format!("Wallet label '{raw}' is ambiguous"),
                )
                .with_hint("Use the wallet id instead of the non-unique label.")
                .with_details(dict([(
                    "matches",
                    Value::List(
                        found
                            .iter()
                            .map(|r| project(r, &["id", "label", "account_code"]))
                            .collect(),
                    ),
                )])));
            }
            Some(
                found.pop().ok_or_else(|| {
                    AppError::new("not_found", format!("Wallet '{raw}' not found"))
                })?,
            )
        }
    } else {
        None
    };
    let mut tx_where = "t.profile_id=?".to_owned();
    let mut tx_args = vec![Sql::Text(id.clone())];
    if let Some(w) = &wallet {
        tx_where += " AND t.wallet_id=?";
        tx_args.push(Sql::Text(text(w, "id")));
    }
    let summary = rows(
        conn,
        &queries::SUMMARY.replace("{tx_where}", &tx_where),
        params_from_iter(tx_args.iter()),
    )?
    .remove(0);
    let count = |sql: String| -> Result<Value> {
        Ok(value(
            &rows(conn, &sql, params_from_iter(tx_args.iter()))?.remove(0),
            "n",
        )
        .clone())
    };
    let tagged=count(format!("SELECT COUNT(DISTINCT tt.transaction_id) AS n FROM transaction_tags tt JOIN transactions t ON t.id=tt.transaction_id WHERE {tx_where} AND t.excluded=0"))?;
    let quarantines=count(format!("SELECT COUNT(*) AS n FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id WHERE {tx_where}"))?;
    let mut journal_where = "je.profile_id=?".to_owned();
    if wallet.is_some() {
        journal_where += " AND je.wallet_id=?";
    }
    let journal_entries = count(format!(
        "SELECT COUNT(*) AS n FROM journal_entries je WHERE {journal_where}"
    ))?;
    let wallet_count = if wallet.is_some() {
        1
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM wallets WHERE workspace_id=? AND profile_id=?",
            [text(workspace, "id"), id.clone()],
            |r| r.get::<_, i64>(0),
        )
        .map_err(sql_error)?
    };
    let Value::List(mut holdings) = holdings::report(conn, profile, true)? else {
        unreachable!()
    };
    // Keep the complete SQL conversion and identity checks, and the original
    // row order for float sums. Summary does not emit per-entry references or
    // quantities, so it need not construct those presentation fields.
    let database: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key='database_instance_id'",
            [],
            |r| r.get(0),
        )
        .ok();
    let mut nonempty = false;
    let mut realized_totals = [0.0; 3];
    sql::visit_rows(conn, CAPITAL_GAINS_SQL, [&id], |row| {
        nonempty = true;
        if wallet
            .as_ref()
            .is_none_or(|w| value(row, "wallet") == value(w, "label"))
        {
            for (total, key) in
                realized_totals
                    .iter_mut()
                    .zip(["proceeds", "cost_basis", "gain_loss"])
            {
                *total += number(row, key);
            }
        }
        Ok(())
    })?;
    validate_capital_identity(database.as_deref(), nonempty)?;
    if let Some(w) = &wallet {
        holdings.retain(|r| value(r, "wallet") == value(w, "label"));
    }
    // Python first groups wallet floats into account buckets, then sums sorted
    // buckets. Preserve both rounding stages and the iteration order.
    let mut buckets: Vec<((Value, Value), [f64; 3])> = Vec::new();
    for row in holdings {
        let key = (value(&row, "account").clone(), value(&row, "asset").clone());
        let index = match buckets.iter().position(|(k, _)| *k == key) {
            Some(index) => index,
            None => {
                buckets.push((key, [0.0; 3]));
                buckets.len() - 1
            }
        };
        let b = &mut buckets[index].1;
        for (i, k) in ["cost_basis", "market_value", "unrealized_pnl"]
            .iter()
            .enumerate()
        {
            b[i] += number(&row, k);
        }
    }
    // Null and empty account codes are distinct buckets with equal sort keys;
    // Python's stable sort preserves their first encounter order.
    buckets.sort_by_key(|((account, asset), _)| {
        let key = |v: &Value| if truthy(v) { py_str(v) } else { String::new() };
        (key(account), key(asset))
    });
    let mut totals = [0.0; 3];
    for (_, b) in &buckets {
        for i in 0..3 {
            totals[i] += b[i];
        }
    }
    let holdings = dict([
        ("cost_basis", Value::Float(totals[0])),
        ("market_value", Value::Float(totals[1])),
        ("unrealized_pnl", Value::Float(totals[2])),
    ]);
    let realized = dict(
        ["proceeds", "cost_basis", "gain_loss"]
            .into_iter()
            .zip(realized_totals)
            .map(|(k, total)| (k, Value::Float(total))),
    );
    let flow = |wallet_flow: bool| -> Result<Value> {
        let select = if wallet_flow {
            "w.label AS wallet,t.asset,"
        } else {
            "t.asset,"
        };
        let group = if wallet_flow {
            "w.label,t.asset"
        } else {
            "t.asset"
        };
        let query=format!("SELECT {select}COUNT(*) AS tx_count,SUM(CASE WHEN t.direction='inbound' THEN 1 ELSE 0 END) AS inbound_count,SUM(CASE WHEN t.direction='outbound' THEN 1 ELSE 0 END) AS outbound_count,SUM(CASE WHEN t.direction='inbound' THEN t.amount ELSE 0 END) AS inbound_amount,SUM(CASE WHEN t.direction='outbound' THEN t.amount ELSE 0 END) AS outbound_amount,SUM(t.fee) AS fee_amount,MIN(t.occurred_at) AS first_at,MAX(t.occurred_at) AS last_at FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE {tx_where} AND t.excluded=0 GROUP BY {group} ORDER BY {group}");
        let mut result = Vec::new();
        for row in rows(conn, &query, params_from_iter(tx_args.iter()))? {
            let mut r = if wallet_flow {
                project(
                    &row,
                    &[
                        "wallet",
                        "asset",
                        "tx_count",
                        "inbound_count",
                        "outbound_count",
                    ],
                )
            } else {
                project(
                    &row,
                    &["asset", "tx_count", "inbound_count", "outbound_count"],
                )
            };
            for key in ["inbound_amount", "outbound_amount", "fee_amount"] {
                let msat = int(&row, key);
                replace(&mut r, key, float(&btc(msat)?)?);
                replace(
                    &mut r,
                    &format!("{key}_sat"),
                    Value::Float(msat as f64 / 1000.0),
                );
                replace(&mut r, &format!("{key}_msat"), Value::Int(msat.into()));
            }
            if wallet_flow {
                replace(
                    &mut r,
                    "first_transaction_at",
                    value(&row, "first_at").clone(),
                );
                replace(
                    &mut r,
                    "last_transaction_at",
                    value(&row, "last_at").clone(),
                );
            }
            result.push(r);
        }
        Ok(Value::List(result))
    };
    let mut metrics = dict([
        ("wallets_in_scope", Value::Int(wallet_count.into())),
        (
            "assets_in_scope",
            Value::Int(int(&summary, "asset_count").into()),
        ),
    ]);
    for key in [
        "active_transactions",
        "excluded_transactions",
        "inbound_transactions",
        "outbound_transactions",
    ] {
        replace(&mut metrics, key, Value::Int(int(&summary, key).into()));
    }
    for (key, v) in [
        ("journal_entries", journal_entries),
        ("quarantines", quarantines),
        (
            "priced_transactions",
            Value::Int(int(&summary, "priced_transactions").into()),
        ),
        (
            "transactions_with_notes",
            Value::Int(int(&summary, "noted_transactions").into()),
        ),
        ("transactions_with_tags", tagged),
        (
            "first_transaction_at",
            value(&summary, "first_transaction_at").clone(),
        ),
        (
            "last_transaction_at",
            value(&summary, "last_transaction_at").clone(),
        ),
    ] {
        replace(&mut metrics, key, v);
    }
    Ok(dict([
        ("workspace", value(workspace, "label").clone()),
        ("profile", value(profile, "label").clone()),
        (
            "wallet",
            wallet
                .as_ref()
                .map(|w| value(w, "label").clone())
                .unwrap_or(Value::Null),
        ),
        ("fiat_currency", value(profile, "fiat_currency").clone()),
        ("tax_country", value(profile, "tax_country").clone()),
        (
            "tax_long_term_days",
            Value::Int(int(profile, "tax_long_term_days").into()),
        ),
        ("gains_algorithm", value(profile, "gains_algorithm").clone()),
        (
            "last_processed_at",
            value(profile, "last_processed_at").clone(),
        ),
        (
            "processed_tx_count",
            Value::Int(int(profile, "last_processed_tx_count").into()),
        ),
        ("metrics", metrics),
        ("holdings", holdings),
        ("realized", realized),
        ("asset_flow", flow(false)?),
        ("wallet_flow", flow(true)?),
        ("transfer_pairs", pairs(conn, &id, wallet.as_ref())?),
    ]))
}
