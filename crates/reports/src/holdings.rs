use super::*;
use std::collections::BTreeMap;

fn exact(row: &Value, field: &str) -> Result<Option<Decimal>> {
    let exact = value(row, &format!("{field}_exact"));
    let raw = if matches!(exact, Value::Null) {
        value(row, field)
    } else {
        exact
    };
    if matches!(raw, Value::Null) {
        Ok(None)
    } else {
        decimal(raw).map(Some)
    }
}
fn rates(
    conn: &Connection,
    profile: &Value,
    assets: &[Value],
) -> Result<BTreeMap<String, Decimal>> {
    let mut rates = BTreeMap::new();
    sql::visit_rows(conn,"SELECT asset,fiat_rate,fiat_value,fiat_rate_exact,fiat_value_exact,amount FROM transactions WHERE profile_id=? AND excluded=0 ORDER BY occurred_at DESC,created_at DESC,id DESC",[text(profile,"id")], |row| {
        let asset = text(row,"asset");
        if rates.contains_key(&asset) { return Ok(()); }
        if let Some(rate) = exact(row,"fiat_rate")? { rates.insert(asset,rate); }
        else if let Some(total) = exact(row,"fiat_value")? {
            let amount = int(row,"amount");
            if amount != 0 { rates.insert(asset,total.div(&btc(amount)?,&DECIMAL).map_err(arithmetic)?); }
        }
        Ok(())
    })?;
    for asset in assets {
        let asset = kassiber_pytext::strip(&text(asset, "asset")).to_uppercase();
        if let Some(pair) =
            kassiber_rates::transaction_rate_pair(&asset, &text(profile, "fiat_currency"))
        {
            let rows = sql::optional_rows(conn,"SELECT rate,rate_exact FROM rates_cache WHERE pair=? ORDER BY timestamp DESC, CASE WHEN source='manual' THEN 0 ELSE 1 END ASC, fetched_at DESC, source ASC LIMIT 1",[&pair])?;
            if let Some(row) = rows.first() {
                if let Some(rate) = exact(row, "rate")? {
                    rates.insert(asset, rate);
                }
            }
        }
    }
    Ok(rates)
}

pub fn report(conn: &Connection, profile: &Value, wallet: bool) -> Result<Value> {
    report_with(conn, profile, wallet, false)
}

/// `report_portfolio_summary(..., include_wallet_id=...)`: wallet rows end
/// with their wallet id when asked.
pub fn report_with(
    conn: &Connection,
    profile: &Value,
    wallet: bool,
    include_wallet_id: bool,
) -> Result<Value> {
    let query = if wallet {
        "SELECT wallet_id,wallet_label,account_code,asset,quantity,cost_basis FROM journal_wallet_holdings WHERE profile_id=? ORDER BY wallet_label ASC,asset ASC,id ASC"
    } else {
        "SELECT account_code,account_label,asset,quantity,cost_basis FROM journal_account_holdings WHERE profile_id=? ORDER BY account_code ASC,asset ASC,id ASC"
    };
    let holding_rows = rows(conn, query, [text(profile, "id")])?;
    let rates = rates(conn, profile, &holding_rows)?;
    let mut result = Vec::new();
    for row in holding_rows {
        let quantity_msat = int(&row, "quantity");
        if quantity_msat <= 0 {
            continue;
        }
        let quantity = btc(quantity_msat)?;
        let basis = decimal(value(&row, "cost_basis"))?;
        let rate = rates
            .get(&text(&row, "asset"))
            .cloned()
            .unwrap_or_else(Decimal::zero);
        let market = quantity.mul(&rate, &DECIMAL).map_err(arithmetic)?;
        let mut fields = Vec::new();
        if wallet {
            fields.push(("wallet", value(&row, "wallet_label").clone()));
        }
        fields.extend([
            (
                "account",
                if wallet || truthy(value(&row, "account_code")) {
                    value(&row, "account_code").clone()
                } else {
                    value(&row, "account_label").clone()
                },
            ),
            ("asset", value(&row, "asset").clone()),
            ("quantity", float(&quantity)?),
            ("quantity_sat", Value::Float(quantity_msat as f64 / 1000.0)),
            ("quantity_msat", Value::Int(quantity_msat.into())),
        ]);
        if wallet {
            fields.push((
                "avg_cost",
                float(&basis.div(&quantity, &DECIMAL).map_err(arithmetic)?)?,
            ));
        }
        fields.extend([
            ("cost_basis", float(&basis)?),
            ("market_value", float(&market)?),
            (
                "unrealized_pnl",
                float(&market.sub(&basis, &DECIMAL).map_err(arithmetic)?)?,
            ),
        ]);
        if wallet && include_wallet_id {
            fields.push(("wallet_id", value(&row, "wallet_id").clone()));
        }
        result.push(dict(fields));
    }
    Ok(Value::List(result))
}
