//! The summary PDF's report model (`build_summary_pdf_report_data`): stored
//! journal and transaction reads only, in Python's order, with Python's
//! Decimal and float conversions. The renderer only formats this model.
use super::*;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql};
use std::collections::{BTreeMap, BTreeSet};

const MONTH: &str = "month";

fn add(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.add(b, &DECIMAL).map_err(arithmetic)
}
fn mul(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.mul(b, &DECIMAL).map_err(arithmetic)
}
fn div(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.div(b, &DECIMAL).map_err(arithmetic)
}
/// `msat_to_btc(value or 0)`.
fn msat_btc(row: &Value, key: &str) -> Result<Decimal> {
    let raw = value(row, key);
    if !truthy(raw) {
        return Ok(Decimal::zero());
    }
    history::msat(raw)
}
/// `dec(value)` of a column Python reads as a number.
fn dec(row: &Value, key: &str) -> Result<Decimal> {
    decimal(value(row, key))
}
fn stamp(us: i64) -> Result<String> {
    history::stamp(us)
}
/// A stored timestamp parsed like `hooks.parse_iso_datetime`.
fn instant(row: &Value, key: &str, field: &str) -> Result<i64> {
    history::parse(value(row, key), field)?.ok_or_else(|| {
        AppError::new(
            "internal_error",
            "'NoneType' object has no attribute 'astimezone'",
        )
    })
}
fn month_key(us: i64) -> Result<String> {
    Ok(stamp(us)?[..7].to_owned())
}
fn int_or_zero(v: &Value) -> i64 {
    match v {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or(0),
        Value::Float(f) if f.is_finite() => f.trunc() as i64,
        _ => 0,
    }
}

/// The admitted wallets: id, label and the scope text inputs.
#[derive(Clone)]
pub(super) struct Wallet {
    pub id: String,
    pub label: Value,
    pub kind: Value,
    pub chain: String,
}

/// `_summary_pdf_period`.
fn period(args: &Value, now: &str) -> Result<(i64, i64)> {
    let end = value(args, "end");
    let end = if truthy(end) {
        history::parse(end, "end")?
    } else {
        history::parse(&s(now), "now")?
    }
    .ok_or_else(|| AppError::new("internal_error", "missing report clock"))?;
    let end = end - end.rem_euclid(1_000_000);
    let start = value(args, "start");
    let start = if truthy(start) {
        let start = history::parse(start, "start")?
            .ok_or_else(|| AppError::new("internal_error", "missing start"))?;
        start - start.rem_euclid(1_000_000)
    } else {
        kassiber_rates::time::parse(&format!("{}-01-01", &stamp(end)?[..4]))
            .ok_or_else(|| AppError::new("internal_error", "year is out of range"))?
    };
    if start > end {
        return Err(AppError::new(
            "validation",
            "--start must not be after --end",
        ));
    }
    Ok((start, end))
}

fn all_wallets(conn: &Connection, profile: &Value) -> Result<Vec<Wallet>> {
    Ok(full_csv::listed(conn, &text(profile, "id"), None)?
        .into_iter()
        .map(|(id, row)| Wallet {
            id,
            label: value(&row, "wallet").clone(),
            kind: value(&row, "kind").clone(),
            chain: text(&row, "chain"),
        })
        .collect())
}

/// `_resolve_wallet_scope_refs`.
fn scope(conn: &Connection, profile: &Value, refs: &Value) -> Result<Vec<Wallet>> {
    let all = all_wallets(conn, profile)?;
    if matches!(refs, Value::Null) {
        return Ok(all);
    }
    let refs: Vec<String> = artifact_csv::list(refs)
        .iter()
        .map(|r| kassiber_pytext::strip(&py_str(r)).to_owned())
        .filter(|r| !r.is_empty())
        .collect();
    if refs.is_empty() {
        return Err(AppError::new(
            "validation",
            "Summary PDF requires at least one wallet in scope",
        ));
    }
    let mut resolved: Vec<Wallet> = Vec::new();
    for reference in refs {
        let found = all
            .iter()
            .find(|w| w.id == reference)
            // A later wallet with the same label wins, as in Python's dict.
            .or_else(|| all.iter().rev().find(|w| py_str(&w.label) == reference));
        let wallet = match found {
            Some(w) => w.clone(),
            None => {
                let w = kassiber_wallets::resolve(conn, &text(profile, "id"), &reference)?;
                all.iter()
                    .find(|row| row.id == w.id)
                    .cloned()
                    .unwrap_or(Wallet {
                        id: w.id,
                        label: s(w.label),
                        kind: Value::Null,
                        chain: String::new(),
                    })
            }
        };
        if !resolved.iter().any(|w| w.id == wallet.id) {
            resolved.push(wallet);
        }
    }
    Ok(resolved)
}

fn scope_is_all(conn: &Connection, profile: &Value, wallets: &[Wallet]) -> Result<bool> {
    let all: BTreeSet<String> = all_wallets(conn, profile)?
        .into_iter()
        .map(|w| w.id)
        .collect();
    let selected: BTreeSet<String> = wallets.iter().map(|w| w.id.clone()).collect();
    Ok(all == selected)
}

/// `_wallet_scope_sql`.
fn wallet_sql(column: &str, wallets: &[Wallet]) -> Result<(String, Vec<Sql>)> {
    if wallets.is_empty() {
        return Err(AppError::new(
            "validation",
            "Summary PDF requires at least one wallet in scope",
        ));
    }
    Ok((
        format!("{column} IN ({})", vec!["?"; wallets.len()].join(",")),
        wallets.iter().map(|w| Sql::Text(w.id.clone())).collect(),
    ))
}

fn query(conn: &Connection, sql: &str, params: Vec<Sql>) -> Result<Vec<Value>> {
    rows(conn, sql, params_from_iter(params.iter()))
}

/// A historical holding: (wallet id, wallet label, account, asset), then
/// its quantity and cost basis.
type Holding = ((Value, Value, Value, Value), Decimal, Decimal);

/// `_historical_portfolio_summary(..., include_wallet_id=True)`.
fn historical_portfolio(conn: &Connection, profile: &Value, as_of: i64) -> Result<Vec<Value>> {
    let id = text(profile, "id");
    let at = stamp(as_of)?;
    let entries = query(conn, "SELECT je.occurred_at,je.wallet_id,w.label AS wallet,COALESCE(a.code,a.label,'') AS account,je.asset,je.entry_type,je.quantity,je.fiat_value,COALESCE(je.cost_basis,0) AS cost_basis FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id LEFT JOIN accounts a ON a.id=je.account_id WHERE je.profile_id=? AND je.occurred_at<=? ORDER BY je.occurred_at ASC,je.created_at ASC,je.id ASC", vec![Sql::Text(id.clone()), Sql::Text(at.clone())])?;
    let rate_rows = query(conn, "SELECT occurred_at,asset,amount,fiat_rate,fiat_value FROM transactions WHERE profile_id=? AND excluded=0 AND occurred_at<=? AND (fiat_rate IS NOT NULL OR fiat_value IS NOT NULL) ORDER BY occurred_at ASC,created_at ASC,id ASC", vec![Sql::Text(id.clone()), Sql::Text(at.clone())])?;
    // Keys (wallet id, wallet label, account, asset) in first-seen order.
    let mut holdings: Vec<Holding> = Vec::new();
    let mut fallback: Vec<(String, Decimal)> = Vec::new();
    for row in &rate_rows {
        let rate = if !matches!(value(row, "fiat_rate"), Value::Null) {
            Some(dec(row, "fiat_rate")?)
        } else if !matches!(value(row, "fiat_value"), Value::Null) && truthy(value(row, "amount")) {
            Some(div(
                &dec(row, "fiat_value")?,
                &history::msat(value(row, "amount"))?,
            )?)
        } else {
            None
        };
        if let Some(rate) = rate {
            let asset = text(row, "asset");
            match fallback.iter_mut().find(|(a, _)| *a == asset) {
                Some(slot) => slot.1 = rate,
                None => fallback.push((asset, rate)),
            }
        }
    }
    for row in &entries {
        let quantity = history::msat(value(row, "quantity"))?;
        let key = (
            value(row, "wallet_id").clone(),
            value(row, "wallet").clone(),
            value(row, "account").clone(),
            value(row, "asset").clone(),
        );
        let index = match holdings.iter().position(|(k, _, _)| *k == key) {
            Some(index) => index,
            None => {
                holdings.push((key, Decimal::zero(), Decimal::zero()));
                holdings.len() - 1
            }
        };
        let kind = text(row, "entry_type");
        if !["income", "transfer_fee"].contains(&kind.as_str()) {
            holdings[index].1 = add(&holdings[index].1, &quantity)?;
        }
        if kind != "income" {
            let delta = if quantity >= Decimal::zero() {
                dec(row, "fiat_value")?
            } else {
                Decimal::zero()
                    .sub(&dec(row, "cost_basis")?, &DECIMAL)
                    .map_err(arithmetic)?
            };
            holdings[index].2 = add(&holdings[index].2, &delta)?;
        }
    }
    let mut pool: Vec<(Value, Decimal, Decimal)> = Vec::new();
    for ((_, _, _, asset), quantity, basis) in &holdings {
        match pool.iter_mut().find(|(a, _, _)| a == asset) {
            Some(slot) => {
                slot.1 = add(&slot.1, quantity)?;
                slot.2 = add(&slot.2, basis)?;
            }
            None => pool.push((asset.clone(), quantity.clone(), basis.clone())),
        }
    }
    let mut average = Vec::new();
    for (asset, quantity, basis) in &pool {
        average.push((
            asset.clone(),
            if *quantity > Decimal::zero() {
                div(basis, quantity)?
            } else {
                Decimal::zero()
            },
        ));
    }
    // market_rates_for_profile_at_or_before(assets=pool, fallback_rates=...)
    let fallback = if fallback.is_empty() {
        latest_transaction_rates(conn, &id)?
    } else {
        fallback
    };
    let mut assets: Vec<String> = pool
        .iter()
        .map(|(asset, _, _)| {
            kassiber_pytext::upper(kassiber_pytext::strip(&py_str_or_empty(asset)))
        })
        .filter(|a| !a.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assets.sort();
    let mut market: Vec<(String, Decimal)> = Vec::new();
    for asset in assets {
        let mut rate = None;
        if let Some(pair) =
            kassiber_rates::transaction_rate_pair(&asset, &text(profile, "fiat_currency"))
        {
            if let Some(cached) = kassiber_rates::cached_at_or_before(conn, &pair, &at)? {
                for raw in [value(&cached, "rate_exact"), value(&cached, "rate")] {
                    if !matches!(raw, Value::Null) && raw != &s("") {
                        rate = Some(decimal(raw)?);
                        break;
                    }
                }
            }
        }
        if rate.is_none() {
            rate = fallback
                .iter()
                .find(|(a, _)| *a == asset)
                .map(|(_, r)| r.clone());
        }
        if let Some(rate) = rate {
            market.push((asset, rate));
        }
    }
    for (asset, rate) in &fallback {
        if !market.iter().any(|(a, _)| a == asset) {
            market.push((asset.clone(), rate.clone()));
        }
    }
    // sorted(holdings.items(), key=(wallet label, asset)), stable.
    let mut ordered: Vec<&Holding> = holdings.iter().collect();
    ordered.sort_by(|a, b| {
        let key = |h: &&Holding| (py_str(&h.0 .1), py_str(&h.0 .3));
        key(a).cmp(&key(b))
    });
    let mut results = Vec::new();
    for ((wallet_id, wallet, account, asset), quantity, _) in ordered {
        if *quantity <= Decimal::zero() {
            continue;
        }
        let avg = average
            .iter()
            .find(|(a, _)| a == asset)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(Decimal::zero);
        let basis = mul(quantity, &avg)?;
        let rate = market
            .iter()
            .find(|(a, _)| s(a.as_str()) == *asset)
            .map(|(_, r)| r.clone())
            .unwrap_or_else(Decimal::zero);
        let value_ = mul(quantity, &rate)?;
        let msat = quantity
            .mul(&Decimal::from(100_000_000_000_i64), &DECIMAL)
            .map_err(arithmetic)?
            .to_fixed_places(0, Rounding::HalfUp)
            .map_err(arithmetic)?;
        let msat = kassiber_pytext::parse_int(&msat)
            .map_err(|_| AppError::new("internal_error", "invalid portfolio quantity"))?;
        let sat = msat
            .to_string()
            .parse::<f64>()
            .map_err(|_| AppError::new("internal_error", "invalid portfolio quantity"))?
            / 1000.0;
        results.push(dict([
            ("wallet", wallet.clone()),
            ("account", account.clone()),
            ("asset", asset.clone()),
            ("quantity", float(quantity)?),
            ("quantity_sat", Value::Float(sat)),
            ("quantity_msat", Value::Int(msat)),
            ("avg_cost", float(&avg)?),
            ("cost_basis", float(&basis)?),
            ("market_value", float(&value_)?),
            (
                "unrealized_pnl",
                float(&value_.sub(&basis, &DECIMAL).map_err(arithmetic)?)?,
            ),
            ("wallet_id", wallet_id.clone()),
        ]));
    }
    Ok(results)
}

fn py_str_or_empty(v: &Value) -> String {
    if truthy(v) {
        py_str(v)
    } else {
        String::new()
    }
}

/// `latest_transaction_rates_for_profile`: the newest usable rate per asset.
fn latest_transaction_rates(conn: &Connection, profile: &str) -> Result<Vec<(String, Decimal)>> {
    let mut rates: Vec<(String, Decimal)> = Vec::new();
    for row in rows(conn, "SELECT asset,fiat_rate,fiat_value,fiat_rate_exact,fiat_value_exact,amount FROM transactions WHERE profile_id=? AND excluded=0 ORDER BY occurred_at DESC,created_at DESC,id DESC", [profile])? {
        let asset = text(&row, "asset");
        if rates.iter().any(|(a, _)| *a == asset) {
            continue;
        }
        let pick = |exact: &str, plain: &str| -> Result<Option<Decimal>> {
            for key in [exact, plain] {
                if !matches!(value(&row, key), Value::Null) {
                    return dec(&row, key).map(Some);
                }
            }
            Ok(None)
        };
        if let Some(rate) = pick("fiat_rate_exact", "fiat_rate")? {
            rates.push((asset, rate));
        } else if let Some(total) = pick("fiat_value_exact", "fiat_value")? {
            if truthy(value(&row, "amount")) {
                rates.push((asset, div(&total, &history::msat(value(&row, "amount"))?)?));
            }
        }
    }
    Ok(rates)
}

/// `_summary_pdf_portfolio_rows`.
fn portfolio(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    as_of: Option<i64>,
) -> Result<Vec<Value>> {
    let rows = match as_of {
        Some(at) => historical_portfolio(conn, profile, at)?,
        None => artifact_csv::list(&holdings::report_with(conn, profile, true, true)?).to_vec(),
    };
    if scope_is_all(conn, profile, wallets)? {
        return Ok(rows);
    }
    Ok(rows
        .into_iter()
        .filter(|row| wallets.iter().any(|w| w.id == text(row, "wallet_id")))
        .collect())
}

/// `_summary_pdf_wallet_holdings_from_portfolio`.
fn wallet_holdings(
    wallets: &[Wallet],
    portfolio: &[Value],
    tx_counts: &BTreeMap<String, i64>,
) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    for wallet in wallets {
        let mut assets: BTreeMap<String, Decimal> = BTreeMap::new();
        let mut asset_set = BTreeSet::new();
        let (mut quantity, mut basis, mut market) =
            (Decimal::zero(), Decimal::zero(), Decimal::zero());
        for row in portfolio
            .iter()
            .filter(|r| text(r, "wallet_id") == wallet.id)
        {
            let asset = text(row, "asset");
            asset_set.insert(asset.clone());
            let q = dec(row, "quantity")?;
            let slot = assets.entry(asset).or_insert_with(Decimal::zero);
            *slot = add(slot, &q)?;
            quantity = add(&quantity, &q)?;
            basis = add(&basis, &dec(row, "cost_basis")?)?;
            market = add(&market, &dec(row, "market_value")?)?;
        }
        let chain = if wallet.chain.is_empty() {
            "chain".to_owned()
        } else {
            wallet.chain.clone()
        };
        let mut asset_quantities = Vec::new();
        for (asset, q) in &assets {
            if *q != Decimal::zero() {
                asset_quantities.push(dict([
                    ("asset", s(asset.as_str())),
                    ("quantity", float(q)?),
                ]));
            }
        }
        results.push(dict([
            ("wallet_id", s(wallet.id.as_str())),
            ("wallet", wallet.label.clone()),
            ("scope", s(format!("{} / {chain}", py_str(&wallet.kind)))),
            (
                "assets",
                Value::List(asset_set.into_iter().map(s).collect()),
            ),
            ("asset_quantities", Value::List(asset_quantities)),
            ("quantity", float(&quantity)?),
            ("cost_basis", float(&basis)?),
            ("market_value", float(&market)?),
            (
                "tx_count",
                Value::Int(tx_counts.get(&wallet.id).copied().unwrap_or(0).into()),
            ),
        ]));
    }
    Ok(results)
}

fn total(rows: &[Value], key: &str) -> Result<Value> {
    let mut sum = Decimal::zero();
    for row in rows {
        sum = add(&sum, &dec(row, key)?)?;
    }
    float(&sum)
}

fn total_assets(rows: &[Value]) -> Result<Value> {
    let mut totals: BTreeMap<String, Decimal> = BTreeMap::new();
    for row in rows {
        for item in artifact_csv::list(value(row, "asset_quantities")) {
            let asset = kassiber_pytext::upper(kassiber_pytext::strip(&py_str_or_empty(value(
                item, "asset",
            ))));
            if !asset.is_empty() {
                let slot = totals.entry(asset).or_insert_with(Decimal::zero);
                *slot = add(slot, &dec(item, "quantity")?)?;
            }
        }
    }
    let mut out = Vec::new();
    for (asset, q) in totals {
        if q != Decimal::zero() {
            out.push(dict([("asset", s(asset)), ("quantity", float(&q)?)]));
        }
    }
    Ok(Value::List(out))
}

fn holdings_totals(rows: &[Value]) -> Result<Value> {
    Ok(dict([
        ("total_quantity", total(rows, "quantity")?),
        ("asset_quantities", total_assets(rows)?),
        ("total_market_value", total(rows, "market_value")?),
    ]))
}

/// `_summary_pdf_balance_history_from_report`.
fn balance_history(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    start: i64,
    end: i64,
) -> Result<Vec<Value>> {
    let args = |wallet: Option<&str>| -> Result<Value> {
        Ok(dict([
            ("interval", s(MONTH)),
            ("start", s(stamp(start)?)),
            ("end", s(stamp(end)?)),
            ("wallet", wallet.map(s).unwrap_or(Value::Null)),
        ]))
    };
    let rows = if scope_is_all(conn, profile, wallets)? {
        history::report(conn, profile, &args(None)?)?
    } else {
        let mut rows = Vec::new();
        for wallet in wallets {
            rows.extend(history::report(conn, profile, &args(Some(&wallet.id))?)?);
        }
        rows
    };
    let mut buckets: BTreeMap<String, [Decimal; 3]> = BTreeMap::new();
    let mut ends: BTreeMap<String, Value> = BTreeMap::new();
    for row in &rows {
        let key = text(row, "period_start");
        let bucket = buckets
            .entry(key.clone())
            .or_insert_with(|| [Decimal::zero(), Decimal::zero(), Decimal::zero()]);
        for (i, field) in ["quantity", "cumulative_cost_basis", "market_value"]
            .iter()
            .enumerate()
        {
            bucket[i] = add(&bucket[i], &dec(row, field)?)?;
        }
        ends.insert(key, value(row, "period_end").clone());
    }
    let mut results = Vec::new();
    for (start_key, bucket) in buckets {
        let period_end = ends[&start_key].clone();
        let period_end_at = history::parse(&period_end, "period_end")?
            .ok_or_else(|| AppError::new("internal_error", "missing period end"))?;
        results.push(dict([
            ("period_start", s(start_key.as_str())),
            ("period_end", period_end),
            ("period", s(start_key.chars().take(7).collect::<String>())),
            ("period_partial", Value::Bool(end < period_end_at)),
            ("quantity", float(&bucket[0])?),
            ("cumulative_cost_basis", float(&bucket[1])?),
            ("market_value", float(&bucket[2])?),
        ]));
    }
    Ok(results)
}

/// Monthly keys from `start`'s month through `end`'s month.
fn months(start: i64, end: i64) -> Result<Vec<String>> {
    let mut cursor = history::floor(start, MONTH)?;
    let cap = history::floor(end, MONTH)?;
    let mut keys = Vec::new();
    while cursor <= cap {
        keys.push(month_key(cursor)?);
        cursor = history::next(cursor, MONTH)?;
    }
    Ok(keys)
}

struct FlowTotals {
    inflow: Decimal,
    outflow: Decimal,
    fees_btc: Decimal,
    fees_fiat: Decimal,
}

fn bounds(
    profile: &Value,
    wallets: &[Wallet],
    column: &str,
    start: i64,
    end: i64,
) -> Result<(String, Vec<Sql>)> {
    let (filter, ids) = wallet_sql(column, wallets)?;
    let mut params = vec![Sql::Text(text(profile, "id"))];
    params.extend(ids);
    params.push(Sql::Text(stamp(start)?));
    params.push(Sql::Text(stamp(end)?));
    Ok((filter, params))
}

/// `_summary_pdf_flow_periods`.
fn flow_periods(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    start: i64,
    end: i64,
) -> Result<(Vec<Value>, FlowTotals)> {
    let (filter, params) = bounds(profile, wallets, "t.wallet_id", start, end)?;
    let rows = query(conn, &format!("SELECT t.occurred_at,t.direction,t.amount,t.fee,t.fiat_rate,t.fiat_value FROM transactions t WHERE t.profile_id=? AND {filter} AND t.excluded=0 AND t.occurred_at>=? AND t.occurred_at<=? ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC"), params)?;
    let zero = || {
        [
            Decimal::zero(),
            Decimal::zero(),
            Decimal::zero(),
            Decimal::zero(),
        ]
    };
    let mut buckets: BTreeMap<String, [Decimal; 4]> = BTreeMap::new();
    let mut totals = zero();
    for row in &rows {
        let key = month_key(instant(row, "occurred_at", "occurred_at")?)?;
        let amount = msat_btc(row, "amount")?;
        let fee = msat_btc(row, "fee")?;
        let rate = value(row, "fiat_rate");
        let volume = if !matches!(value(row, "fiat_value"), Value::Null) {
            dec(row, "fiat_value")?
        } else if !matches!(rate, Value::Null) {
            mul(&amount, &decimal(rate)?)?
        } else {
            Decimal::zero()
        };
        let fee_fiat = if !matches!(rate, Value::Null) {
            mul(&fee, &decimal(rate)?)?
        } else {
            Decimal::zero()
        };
        let bucket = buckets.entry(key).or_insert_with(zero);
        let side = if text(row, "direction") == "inbound" {
            0
        } else {
            1
        };
        bucket[side] = add(&bucket[side], &volume)?;
        totals[side] = add(&totals[side], &volume)?;
        bucket[2] = add(&bucket[2], &fee)?;
        bucket[3] = add(&bucket[3], &fee_fiat)?;
        totals[2] = add(&totals[2], &fee)?;
        totals[3] = add(&totals[3], &fee_fiat)?;
    }
    let mut periods = Vec::new();
    for key in months(start, end)? {
        let bucket = buckets.get(&key).cloned().unwrap_or_else(zero);
        periods.push(dict([
            ("period", s(key)),
            ("inflow_volume", float(&bucket[0])?),
            ("outflow_volume", float(&bucket[1])?),
            ("fees_btc", float(&bucket[2])?),
            ("fees_fiat", float(&bucket[3])?),
        ]));
    }
    let [inflow, outflow, fees_btc, fees_fiat] = totals;
    Ok((
        periods,
        FlowTotals {
            inflow,
            outflow,
            fees_btc,
            fees_fiat,
        },
    ))
}

const DISPOSALS: &str = "je.entry_type='disposal' AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap' AND je.occurred_at>=? AND je.occurred_at<=?";

/// `_summary_pdf_realized_periods`: the periods and the realized total.
fn realized_periods(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    start: i64,
    end: i64,
) -> Result<(Vec<Value>, Decimal)> {
    let (filter, params) = bounds(profile, wallets, "je.wallet_id", start, end)?;
    let rows = query(conn, &format!("SELECT je.occurred_at,COALESCE(je.proceeds,0) AS proceeds,COALESCE(je.cost_basis,0) AS cost_basis,COALESCE(je.gain_loss,0) AS gain_loss FROM journal_entries je LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND {filter} AND {DISPOSALS} ORDER BY je.occurred_at ASC,je.created_at ASC,je.id ASC"), params)?;
    let mut buckets: BTreeMap<String, ([Decimal; 3], i64)> = BTreeMap::new();
    let mut realized = Decimal::zero();
    for row in &rows {
        let key = month_key(instant(row, "occurred_at", "occurred_at")?)?;
        let bucket = buckets
            .entry(key)
            .or_insert_with(|| ([Decimal::zero(), Decimal::zero(), Decimal::zero()], 0));
        for (i, field) in ["proceeds", "cost_basis", "gain_loss"].iter().enumerate() {
            bucket.0[i] = add(&bucket.0[i], &dec(row, field)?)?;
        }
        bucket.1 += 1;
        realized = add(&realized, &dec(row, "gain_loss")?)?;
    }
    let mut periods = Vec::new();
    for key in months(start, end)? {
        let (sums, count) = buckets
            .get(&key)
            .cloned()
            .unwrap_or_else(|| ([Decimal::zero(), Decimal::zero(), Decimal::zero()], 0));
        periods.push(dict([
            ("period", s(key)),
            ("proceeds", float(&sums[0])?),
            ("cost_basis", float(&sums[1])?),
            ("realized_pnl", float(&sums[2])?),
            ("count", Value::Int(count.into())),
        ]));
    }
    Ok((periods, realized))
}

fn rate_at(conn: &Connection, pair: &str, at: &str) -> Result<Option<f64>> {
    let found = rows(conn, "SELECT rate FROM rates_cache WHERE pair=? AND timestamp<=? ORDER BY timestamp DESC,fetched_at DESC LIMIT 1", [pair, at])?;
    found
        .first()
        .map(|row| full_pdf::py_float(value(row, "rate")))
        .transpose()
}

/// `_summary_pdf_benchmark`.
fn benchmark(conn: &Connection, profile: &Value, start: i64, end: i64) -> Result<Value> {
    let fiat = value(profile, "fiat_currency");
    if !truthy(fiat) {
        return Ok(Value::Null);
    }
    let pair = format!("BTC-{}", kassiber_pytext::upper(&py_str(fiat)));
    let start_rate = rate_at(conn, &pair, &stamp(start)?)?;
    let end_rate = rate_at(conn, &pair, &stamp(end)?)?;
    let optional = |v: Option<f64>| v.map(Value::Float).unwrap_or(Value::Null);
    let change = match (start_rate, end_rate) {
        (Some(a), Some(b)) if a > 0.0 => Value::Float((b - a) / a * 100.0),
        _ => Value::Null,
    };
    Ok(dict([
        ("pair", s(pair)),
        ("start_rate", optional(start_rate)),
        ("end_rate", optional(end_rate)),
        ("change_pct", change),
    ]))
}

/// `_summary_pdf_top_movements`.
fn top_movements(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    start: i64,
    end: i64,
) -> Result<Vec<Value>> {
    let (filter, params) = bounds(profile, wallets, "t.wallet_id", start, end)?;
    let rows = query(conn, &format!("SELECT t.occurred_at,t.direction,t.asset,t.amount,t.fiat_value,t.fiat_rate,t.counterparty,t.description,w.label AS wallet FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND {filter} AND t.excluded=0 AND t.occurred_at>=? AND t.occurred_at<=?"), params)?;
    let mut ranked = Vec::new();
    for row in &rows {
        let amount = msat_btc(row, "amount")?;
        let fiat = if !matches!(value(row, "fiat_value"), Value::Null) {
            dec(row, "fiat_value")?
        } else if !matches!(value(row, "fiat_rate"), Value::Null) {
            mul(&amount, &dec(row, "fiat_rate")?)?
        } else {
            Decimal::zero()
        };
        let fiat = float(&fiat)?;
        let counterparty = [value(row, "counterparty"), value(row, "description")]
            .into_iter()
            .find(|v| truthy(v))
            .cloned()
            .unwrap_or_else(|| s(""));
        let Value::Float(magnitude) = fiat else {
            unreachable!("float() is a float")
        };
        ranked.push((
            magnitude.abs(),
            dict([
                ("occurred_at", value(row, "occurred_at").clone()),
                ("wallet", value(row, "wallet").clone()),
                ("direction", value(row, "direction").clone()),
                ("asset", value(row, "asset").clone()),
                ("quantity", float(&amount)?),
                ("fiat_value", fiat),
                ("counterparty", counterparty),
            ]),
        ));
    }
    // list.sort(reverse=True) is stable: equal magnitudes keep their order.
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(ranked.into_iter().take(5).map(|(_, row)| row).collect())
}

/// `_summary_pdf_top_disposals`.
fn top_disposals(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    start: i64,
    end: i64,
) -> Result<Vec<Value>> {
    let (filter, mut params) = bounds(profile, wallets, "je.wallet_id", start, end)?;
    params.push(Sql::Integer(5));
    let rows = query(conn, &format!("SELECT je.occurred_at,je.asset,je.quantity,COALESCE(je.proceeds,0) AS proceeds,COALESCE(je.cost_basis,0) AS cost_basis,COALESCE(je.gain_loss,0) AS gain_loss,w.label AS wallet FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND {filter} AND {DISPOSALS} ORDER BY ABS(je.gain_loss) DESC,je.occurred_at DESC LIMIT ?"), params)?;
    rows.iter()
        .map(|row| {
            Ok(dict([
                ("occurred_at", value(row, "occurred_at").clone()),
                ("wallet", value(row, "wallet").clone()),
                ("asset", value(row, "asset").clone()),
                ("quantity", float(&msat_btc(row, "quantity")?)?),
                ("proceeds", float(&dec(row, "proceeds")?)?),
                ("cost_basis", float(&dec(row, "cost_basis")?)?),
                ("gain_loss", float(&dec(row, "gain_loss")?)?),
            ]))
        })
        .collect()
}

/// `_summary_pdf_internal_transfers`.
fn internal_transfers(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    start: i64,
    end: i64,
) -> Result<Value> {
    let (filter, params) = bounds(profile, wallets, "tout.wallet_id", start, end)?;
    let found = query(conn, &format!("SELECT COUNT(*) AS count,COALESCE(SUM(COALESCE(fiat_value,0)),0) AS fiat_volume,COALESCE(SUM(amount),0) AS amount_msat FROM (SELECT DISTINCT tout.id,tout.fiat_value,tout.amount FROM journal_custody_decisions decision JOIN transactions tout ON tout.id=decision.source_transaction_id WHERE decision.profile_id=? AND decision.source_asset=decision.target_asset AND {filter} AND tout.occurred_at>=? AND tout.occurred_at<=?)"), params)?;
    let row = &found[0];
    Ok(dict([
        ("count", Value::Int(int_or_zero(value(row, "count")).into())),
        ("fiat_volume", float(&dec(row, "fiat_volume")?)?),
        ("btc_volume", float(&msat_btc(row, "amount_msat")?)?),
    ]))
}

/// `projection_freshness` evaluated for the admitted profile.
fn journals(conn: &Connection, profile: &Value) -> Result<Value> {
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [text(profile, "id")],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let processed_at = value(profile, "last_processed_at");
    let processed = int_or_zero(value(profile, "last_processed_tx_count"));
    let input = int_or_zero(value(profile, "journal_input_version"));
    let processed_version = int_or_zero(value(profile, "last_processed_input_version"));
    let semantics = semantics_current(profile);
    let current =
        truthy(processed_at) && processed == active && input == processed_version && semantics;
    let status = if active == 0 {
        "no_transactions"
    } else if !truthy(processed_at) {
        "not_processed"
    } else if processed != active || input != processed_version || !semantics {
        "stale"
    } else {
        "current"
    };
    Ok(dict([
        ("status", s(status)),
        ("current", Value::Bool(current)),
        ("last_processed_at", processed_at.clone()),
        ("journal_input_version", Value::Int(input.into())),
        (
            "last_processed_input_version",
            Value::Int(processed_version.into()),
        ),
        ("last_processed_tx_count", Value::Int(processed.into())),
        ("current_tx_count", Value::Int(active.into())),
    ]))
}

/// `_summary_pdf_data_integrity`.
fn data_integrity(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    start: i64,
    end: i64,
) -> Result<Value> {
    let internal = internal_transfers(conn, profile, wallets, start, end)?;
    let (filter, params) = bounds(profile, wallets, "t.wallet_id", start, end)?;
    let summary = query(conn, &format!("SELECT COUNT(*) AS total_transactions,SUM(CASE WHEN t.fiat_rate IS NOT NULL OR t.fiat_value IS NOT NULL THEN 1 ELSE 0 END) AS priced_transactions FROM transactions t WHERE t.profile_id=? AND {filter} AND t.excluded=0 AND t.occurred_at>=? AND t.occurred_at<=?"), params.clone())?;
    let quarantines = query(conn, &format!("SELECT jq.reason,COUNT(*) AS count FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id WHERE t.profile_id=? AND {filter} AND t.excluded=0 AND t.occurred_at>=? AND t.occurred_at<=? GROUP BY jq.reason ORDER BY count DESC,jq.reason ASC"), params)?;
    let journals = journals(conn, profile)?;
    let total = int_or_zero(value(&summary[0], "total_transactions"));
    let priced = int_or_zero(value(&summary[0], "priced_transactions"));
    let percentage = if total != 0 {
        priced as f64 / total as f64 * 100.0
    } else {
        100.0
    };
    let reasons: Vec<Value> = quarantines
        .iter()
        .map(|row| {
            dict([
                ("reason", value(row, "reason").clone()),
                ("count", Value::Int(int_or_zero(value(row, "count")).into())),
            ])
        })
        .collect();
    let count: i64 = reasons.iter().map(|r| int(r, "count")).sum();
    Ok(dict([
        ("priced_transactions", Value::Int(priced.into())),
        ("total_transactions", Value::Int(total.into())),
        ("priced_percentage", Value::Float(percentage)),
        ("quarantine_count", Value::Int(count.into())),
        ("quarantine_reasons", Value::List(reasons)),
        ("internal_transfers", internal),
        ("journals", journals),
    ]))
}

/// `_summary_pdf_holding_age`.
fn holding_age(conn: &Connection, profile: &Value, wallets: &[Wallet], end: i64) -> Result<Value> {
    let (filter, ids) = wallet_sql("je.wallet_id", wallets)?;
    let mut params = vec![Sql::Text(text(profile, "id"))];
    params.extend(ids);
    params.push(Sql::Text(stamp(end)?));
    let rows = query(conn, &format!("SELECT je.occurred_at,je.quantity FROM journal_entries je WHERE je.profile_id=? AND {filter} AND je.entry_type='acquisition' AND je.occurred_at<=? ORDER BY je.occurred_at ASC"), params)?;
    if rows.is_empty() {
        return Ok(dict([
            ("weighted_days", Value::Null),
            ("oldest_acquisition", Value::Null),
            ("acquisition_count", Value::Int(0.into())),
        ]));
    }
    let oldest = value(&rows[0], "occurred_at").clone();
    let (mut weighted, mut total) = (Decimal::zero(), Decimal::zero());
    for row in &rows {
        let quantity = msat_btc(row, "quantity")?;
        if quantity <= Decimal::zero() {
            continue;
        }
        let acquired = instant(row, "occurred_at", "occurred_at")?;
        // timedelta.total_seconds(), then Decimal(str(float)).
        let seconds = (end - acquired) as f64 / 1_000_000.0;
        let seconds: Decimal = kassiber_pyjson::float_repr(seconds)
            .parse()
            .map_err(|_| AppError::new("internal_error", "invalid holding age"))?;
        weighted = add(&weighted, &mul(&quantity, &seconds)?)?;
        total = add(&total, &quantity)?;
    }
    let count = Value::Int(rows.len().into());
    if total <= Decimal::zero() {
        return Ok(dict([
            ("weighted_days", Value::Null),
            ("oldest_acquisition", oldest),
            ("acquisition_count", count),
        ]));
    }
    let days = div(&div(&weighted, &total)?, &Decimal::from(86_400_i64))?;
    Ok(dict([
        ("weighted_days", float(&days)?),
        ("oldest_acquisition", oldest),
        ("acquisition_count", count),
    ]))
}

fn tx_counts(
    conn: &Connection,
    profile: &Value,
    wallets: &[Wallet],
    start: i64,
    end: i64,
) -> Result<BTreeMap<String, i64>> {
    let (filter, params) = bounds(profile, wallets, "t.wallet_id", start, end)?;
    Ok(query(conn, &format!("SELECT t.wallet_id,COUNT(*) AS tx_count FROM transactions t WHERE t.profile_id=? AND {filter} AND t.excluded=0 AND t.occurred_at>=? AND t.occurred_at<=? GROUP BY t.wallet_id"), params)?
        .iter()
        .map(|row| (text(row, "wallet_id"), int_or_zero(value(row, "tx_count"))))
        .collect())
}

fn appendix_row(row: &Value) -> Value {
    dict([
        ("wallet", value(row, "wallet").clone()),
        ("scope", value(row, "scope").clone()),
        ("tx_count", value(row, "tx_count").clone()),
        ("assets", value(row, "assets").clone()),
        ("asset_quantities", value(row, "asset_quantities").clone()),
        ("end_quantity", value(row, "quantity").clone()),
        ("end_market_value", value(row, "market_value").clone()),
    ])
}

/// The report model Python hands to `write_summary_pdf`.
pub fn model(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let (start, end) = if truthy(value(args, "end")) {
        period(args, "")?
    } else {
        period(args, &sources.now(conn)?)?
    };
    let wallets = scope(conn, profile, value(args, "wallets"))?;
    let generated_at = sources.now(conn)?;
    let portfolio_rows = portfolio(conn, profile, &wallets, Some(end))?;
    let counts = tx_counts(conn, profile, &wallets, start, end)?;
    let holdings = wallet_holdings(&wallets, &portfolio_rows, &counts)?;
    let totals = holdings_totals(&holdings)?;
    let history = balance_history(conn, profile, &wallets, start, end)?;
    let (flow, flow_totals) = flow_periods(conn, profile, &wallets, start, end)?;
    let (realized, realized_total) = realized_periods(conn, profile, &wallets, start, end)?;
    let zero = Value::Float(0.0);
    let first = history.first();
    let last = history.last();
    let pick =
        |row: Option<&Value>, key: &str| row.map(|r| value(r, key).clone()).unwrap_or(zero.clone());
    let start_value = pick(first, "market_value");
    let end_value = pick(last, "market_value");
    let end_basis = pick(last, "cumulative_cost_basis");
    let unrealized =
        Value::Float(full_pdf::py_float(&end_value)? - full_pdf::py_float(&end_basis)?);
    let integrity = data_integrity(conn, profile, &wallets, start, end)?;
    let benchmark = benchmark(conn, profile, start, end)?;
    let movements = top_movements(conn, profile, &wallets, start, end)?;
    let disposals = top_disposals(conn, profile, &wallets, start, end)?;
    let age = holding_age(conn, profile, &wallets, end)?;
    let title = format!("Kassiber Summary Report - {}", text(profile, "label"));
    let snapshot = if truthy(value(args, "include_snapshot")) {
        let rows = portfolio(conn, profile, &wallets, None)?;
        let snapshot_holdings = wallet_holdings(&wallets, &rows, &counts)?;
        let snapshot_totals = holdings_totals(&snapshot_holdings)?;
        dict([
            ("as_of", s(generated_at.as_str())),
            ("wallets", Value::List(snapshot_holdings)),
            (
                "total_quantity",
                value(&snapshot_totals, "total_quantity").clone(),
            ),
            (
                "asset_quantities",
                value(&snapshot_totals, "asset_quantities").clone(),
            ),
            (
                "total_market_value",
                value(&snapshot_totals, "total_market_value").clone(),
            ),
        ])
    } else {
        Value::Null
    };
    let label = format!("{} to {}", &stamp(start)?[..10], &stamp(end)?[..10]);
    Ok(dict([
        ("workspace", value(workspace, "label").clone()),
        ("profile", value(profile, "label").clone()),
        ("fiat_currency", value(profile, "fiat_currency").clone()),
        ("generated_at", s(generated_at.as_str())),
        ("title", s(title)),
        (
            "timeframe",
            dict([
                ("start", s(stamp(start)?)),
                ("end", s(stamp(end)?)),
                ("label", s(label)),
            ]),
        ),
        (
            "wallets",
            Value::List(
                wallets
                    .iter()
                    .map(|w| dict([("id", s(w.id.as_str())), ("label", w.label.clone())]))
                    .collect(),
            ),
        ),
        ("data_integrity", integrity),
        (
            "metrics",
            dict([
                ("period_start_value", start_value),
                ("period_end_value", end_value),
                (
                    "net_flow",
                    float(
                        &flow_totals
                            .inflow
                            .sub(&flow_totals.outflow, &DECIMAL)
                            .map_err(arithmetic)?,
                    )?,
                ),
                ("realized_pnl", float(&realized_total)?),
                ("unrealized_pnl", unrealized),
                ("end_cost_basis", end_basis),
                ("btc_stack_start", pick(first, "quantity")),
                ("btc_stack_end", pick(last, "quantity")),
                ("fees_btc", float(&flow_totals.fees_btc)?),
                ("fees_fiat", float(&flow_totals.fees_fiat)?),
            ]),
        ),
        ("benchmark", benchmark),
        ("top_movements", Value::List(movements)),
        ("top_disposals", Value::List(disposals)),
        ("holding_age", age),
        ("balance_history", Value::List(history)),
        ("wallet_holdings", Value::List(holdings.clone())),
        ("holdings_totals", totals),
        ("realized_pnl_periods", Value::List(realized)),
        ("flow_periods", Value::List(flow)),
        (
            "wallet_appendix",
            Value::List(holdings.iter().map(appendix_row).collect()),
        ),
        ("snapshot", snapshot),
    ]))
}
