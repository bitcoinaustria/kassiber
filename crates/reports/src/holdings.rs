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
    sql::visit_rows(conn,&format!("SELECT asset,fiat_rate,fiat_value,fiat_rate_exact,fiat_value_exact,amount FROM transactions WHERE profile_id=? AND excluded=0 AND {} ORDER BY {}", kassiber_store::price_currency::price_in_book_currency_sql("transactions"), kassiber_store::transaction_order::transaction_order_sql("transactions", true)),[text(profile,"id")], |row| {
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

/// The book's own value of one whole unit of each asset, in its currency:
/// the newest transaction priced in the book currency (its rate, or value
/// over amount), else a cached market rate for a pair the rates crate knows.
/// The same lookup `reports portfolio-summary` values its holdings with; an
/// asset absent from the map has no value in the book (unknown, not zero).
pub fn holding_rates(
    conn: &Connection,
    profile: &Value,
    assets: &[&str],
) -> Result<BTreeMap<String, f64>> {
    let assets: Vec<Value> = assets
        .iter()
        .map(|asset| dict([("asset", Value::from(*asset))]))
        .collect();
    let mut output = BTreeMap::new();
    for (asset, rate) in rates(conn, profile, &assets)? {
        if let Value::Float(rate) = float(&rate)? {
            output.insert(asset, rate);
        }
    }
    Ok(output)
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
        // Bitcoin without a price keeps its zero valuation; another asset
        // without one has no value in the book, so its market value and
        // unrealized result stay unknown (`null`), never zero (plan 24).
        let bitcoin = kassiber_store::assets::is_bitcoin(&text(&row, "asset"));
        let market = match rates.get(&text(&row, "asset")) {
            Some(rate) => Some(quantity.mul(rate, &DECIMAL).map_err(arithmetic)?),
            None if bitcoin => Some(Decimal::zero()),
            None => None,
        };
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
        ]);
        // Sats and msat name Bitcoin units: another asset (ETH) keeps its
        // exact `quantity` and leaves both empty (plan 24).
        if bitcoin {
            fields.extend([
                ("quantity_sat", Value::Float(quantity_msat as f64 / 1000.0)),
                ("quantity_msat", Value::Int(quantity_msat.into())),
            ]);
        } else {
            fields.extend([
                ("quantity_sat", Value::Null),
                ("quantity_msat", Value::Null),
            ]);
        }
        if wallet {
            fields.push((
                "avg_cost",
                float(&basis.div(&quantity, &DECIMAL).map_err(arithmetic)?)?,
            ));
        }
        fields.extend([
            ("cost_basis", float(&basis)?),
            (
                "market_value",
                market.as_ref().map_or(Ok(Value::Null), float)?,
            ),
            (
                "unrealized_pnl",
                match &market {
                    Some(market) => float(&market.sub(&basis, &DECIMAL).map_err(arithmetic)?)?,
                    None => Value::Null,
                },
            ),
        ]);
        if wallet && include_wallet_id {
            fields.push(("wallet_id", value(&row, "wallet_id").clone()));
        }
        result.push(dict(fields));
    }
    Ok(Value::List(result))
}

#[cfg(test)]
mod same_time_tests {
    use super::*;

    #[test]
    fn holdings_rate_is_the_last_same_time_row_in_source_order() {
        let conn = crate::same_time_rate_book();
        let profile = dict([("id", Value::from("p"))]);
        let rates = rates(&conn, &profile, &[Value::from("BTC")]).unwrap();
        assert_eq!(rates["BTC"].to_string(), "10000");
    }

    #[test]
    fn holdings_never_take_a_rate_priced_in_another_currency() {
        let conn = crate::same_time_rate_book();
        // The book now reads USD; only the one row priced in USD is a rate.
        conn.execute_batch("UPDATE profiles SET fiat_currency=' usd '; UPDATE transactions SET fiat_currency='USD ' WHERE id='y'")
            .unwrap();
        let profile = dict([("id", Value::from("p"))]);
        let found = rates(&conn, &profile, &[Value::from("BTC")]).unwrap();
        assert_eq!(found["BTC"].to_string(), "20000");
        conn.execute("UPDATE transactions SET fiat_currency='EUR'", [])
            .unwrap();
        assert!(!rates(&conn, &profile, &[]).unwrap().contains_key("BTC"));
    }
}

#[cfg(test)]
mod unpriced_asset_tests {
    use super::*;

    /// Priced BTC beside ETH the book has no price for: the ETH value stays
    /// unknown in the portfolio, and the summary totals it would add to are
    /// unknown too instead of reading as complete. BTC keeps its valuation.
    #[test]
    fn an_unpriced_asset_has_no_value_and_leaves_totals_unknown() {
        let conn = crate::same_time_rate_book();
        conn.execute_batch(
            "INSERT INTO journal_wallet_holdings(id,workspace_id,profile_id,wallet_id,wallet_label,account_code,asset,quantity,cost_basis,created_at) VALUES
             ('h1','ws','p','w','Cold','treasury','BTC',100000000000,5000,'t'),
             ('h2','ws','p','w','Cold','treasury','ETH',200000000000,300,'t');",
        )
        .unwrap();
        let profile = crate::sql::rows(&conn, "SELECT * FROM profiles WHERE id='p'", [])
            .unwrap()
            .remove(0);
        let Value::List(rows) = report(&conn, &profile, true).unwrap() else {
            panic!("a list")
        };
        let row = |asset: &str| {
            rows.iter()
                .find(|row| text(row, "asset") == asset)
                .unwrap_or_else(|| panic!("{asset}: {rows:?}"))
                .clone()
        };
        let btc = row("BTC");
        assert_eq!(value(&btc, "market_value"), &Value::Float(10_000.0));
        assert_eq!(value(&btc, "unrealized_pnl"), &Value::Float(5_000.0));
        let eth = row("ETH");
        assert_eq!(value(&eth, "cost_basis"), &Value::Float(300.0));
        assert_eq!(value(&eth, "market_value"), &Value::Null);
        assert_eq!(value(&eth, "unrealized_pnl"), &Value::Null);

        let workspace = crate::sql::rows(&conn, "SELECT * FROM workspaces WHERE id='ws'", [])
            .unwrap()
            .remove(0);
        let summary =
            crate::summary::report(&conn, &workspace, &profile, &dict::<&str>([])).unwrap();
        let totals = value(&summary, "holdings");
        assert_eq!(value(totals, "cost_basis"), &Value::Float(5_300.0));
        assert_eq!(value(totals, "market_value"), &Value::Null);
        assert_eq!(value(totals, "unrealized_pnl"), &Value::Null);
        // The balances bucket of the full report keeps it unknown as well.
        let balances = crate::full_csv::aggregate(&rows).unwrap();
        let eth = balances
            .iter()
            .find(|row| text(row, "asset") == "ETH")
            .unwrap();
        assert_eq!(value(eth, "market_value"), &Value::Null);
    }
}
