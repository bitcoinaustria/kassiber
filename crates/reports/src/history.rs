//! Historical holdings reconstructed from the stored journal, never recomputed.
use super::*;
use kassiber_rates::time;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql};
use std::collections::{BTreeMap, BTreeSet};

pub const INTERVALS: &[&str] = &["hour", "day", "week", "month"];
const DAY: i64 = 86_400_000_000;

// Python's msat_to_btc first applies int(), including to legacy numeric rows.
// Invalid stored quantities must raise instead of becoming a zero balance.
pub(super) fn msat(raw: &Value) -> Result<Decimal> {
    if let Value::Int(n) = raw {
        if let Ok(n) = i64::try_from(n.clone()) {
            return btc(n);
        }
    }
    let integer = match raw {
        Value::Int(n) => n.to_string(),
        Value::Float(n) if n.is_finite() => format!("{:.0}", n.trunc()),
        Value::Float(n) => {
            return Err(AppError::new(
                "internal_error",
                if n.is_nan() {
                    "cannot convert float NaN to integer"
                } else {
                    "cannot convert float infinity to integer"
                },
            ))
        }
        Value::Str(t) => {
            let text = t.as_str().unwrap_or_default();
            kassiber_pytext::parse_int(text).map_err(|error| {
                let message = match error {
                    kassiber_pytext::NumberError::TooManyDigits(n) => format!("Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
                    _ => format!("invalid literal for int() with base 10: {}", kassiber_pytext::repr_str(text)),
                };
                AppError::new("internal_error",message)
            })?.to_string()
        }
        _ => {
            return Err(AppError::new(
                "internal_error",
                "invalid stored msat quantity",
            ))
        }
    };
    decimal(&s(integer))?
        .div(&Decimal::from(100_000_000_000_i64), &DECIMAL)
        .map_err(arithmetic)
}

pub fn interval(args: &Value) -> String {
    get(args, "interval")
        .map(py_str)
        .unwrap_or_else(|| "month".into())
}
pub fn validate_interval(args: &Value) -> Result<()> {
    let interval = interval(args);
    if !INTERVALS.contains(&interval.as_str()) {
        return Err(
            AppError::new("validation", format!("Unsupported interval '{interval}'"))
                .with_hint(format!("Choose one of: {}", INTERVALS.join(", "))),
        );
    }
    Ok(())
}
pub(super) fn parse(raw: &Value, field: &str) -> Result<Option<i64>> {
    if matches!(raw, Value::Null) || raw == &s("") {
        return Ok(None);
    }
    if !matches!(raw, Value::Str(_)) {
        return Err(AppError::new(
            "internal_error",
            format!(
                "'{}' object has no attribute 'strip'",
                kassiber_envelope::type_name(raw)
            ),
        ));
    }
    time::parse(&py_str(raw)).map(Some).ok_or_else(|| {
        AppError::new(
            "validation",
            format!("Invalid {field} timestamp '{}'", py_str(raw)),
        )
        .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
    })
}
pub(super) fn stamp(us: i64) -> Result<String> {
    time::format(us).ok_or_else(|| AppError::new("internal_error", "date value out of range"))
}
pub(super) fn floor(us: i64, interval: &str) -> Result<i64> {
    let day = us / DAY * DAY;
    Ok(match interval {
        "hour" => us / 3_600_000_000 * 3_600_000_000,
        "day" => day,
        "week" => day - (us / DAY % 7) * DAY,
        "month" => time::parse(&format!("{}-01", &stamp(us)?[..7])).unwrap(),
        _ => unreachable!(),
    })
}
pub(super) fn next(us: i64, interval: &str) -> Result<i64> {
    match interval {
        "hour" => Ok(us + 3_600_000_000),
        "day" => Ok(us + DAY),
        "week" => Ok(us + 7 * DAY),
        "month" => {
            let raw = stamp(us)?;
            let year: i64 = raw[..4].parse().unwrap();
            let month: i64 = raw[5..7].parse().unwrap();
            time::parse(&format!(
                "{:04}-{:02}-01",
                year + i64::from(month == 12),
                month % 12 + 1
            ))
            .ok_or_else(|| AppError::new("internal_error", "year 10000 is out of range"))
        }
        _ => unreachable!(),
    }
}

// Account resolution follows repo.accounts: exact id, code, then unique label.
fn account(conn: &Connection, profile: &str, raw: &Value) -> Result<String> {
    let reference = py_str(raw);
    let normalized = kassiber_pytext::strip(&reference);
    for predicate in ["id=?", "lower(code)=lower(?)"] {
        if let Some(row) = rows(
            conn,
            &format!("SELECT id FROM accounts WHERE profile_id=? AND {predicate} LIMIT 1"),
            [profile, normalized],
        )?
        .first()
        {
            return Ok(text(row, "id"));
        }
    }
    let matches = rows(conn, "SELECT id,code,label FROM accounts WHERE profile_id=? AND lower(label)=lower(?) ORDER BY code ASC", [profile, normalized])?;
    match matches.as_slice() {
        [row] => Ok(text(row, "id")),
        [] => Err(AppError::new(
            "not_found",
            format!("Account '{reference}' not found"),
        )),
        _ => Err(AppError::new(
            "validation",
            format!("Account label '{reference}' is ambiguous"),
        )
        .with_hint("Use the account code or id instead of the non-unique label.")
        .with_details(dict([("matches", Value::List(matches))]))),
    }
}

struct Event {
    at: i64,
    asset: String,
    kind: String,
    quantity: Decimal,
    fiat: Decimal,
    basis: Decimal,
}
fn events(conn: &Connection, profile: &str, filters: &str, params: Vec<Sql>) -> Result<Vec<Event>> {
    let mut bound = vec![Sql::Text(profile.into())];
    bound.extend(params);
    let mut events = Vec::new();
    sql::visit_rows(conn, &format!("SELECT occurred_at,asset,entry_type,quantity,fiat_value,COALESCE(cost_basis,0) AS cost_basis FROM journal_entries WHERE profile_id=?{filters} ORDER BY occurred_at ASC,created_at ASC,id ASC"), params_from_iter(bound), |row| {
        events.push(Event {
            at: parse(value(row,"occurred_at"),"occurred_at")?.ok_or_else(|| AppError::new("internal_error", "missing journal timestamp"))?,
            asset: text(row,"asset"), kind: text(row,"entry_type"), quantity: msat(value(row,"quantity"))?,
            fiat: decimal(value(row,"fiat_value"))?, basis: decimal(value(row,"cost_basis"))?,
        });
        Ok(())
    })?;
    Ok(events)
}
struct Holdings {
    quantity: Decimal,
    basis: Decimal,
}
impl Default for Holdings {
    fn default() -> Self {
        Self {
            quantity: Decimal::zero(),
            basis: Decimal::zero(),
        }
    }
}
fn accumulate(holdings: &mut BTreeMap<String, Holdings>, event: &Event) -> Result<()> {
    let held = holdings.entry(event.asset.clone()).or_default();
    // Income is already in acquisition; transfer_out already contains its fee.
    if !["income", "transfer_fee"].contains(&event.kind.as_str()) {
        held.quantity = held
            .quantity
            .add(&event.quantity, &DECIMAL)
            .map_err(arithmetic)?;
    }
    if event.kind != "income" {
        held.basis = if event.quantity >= Decimal::zero() {
            held.basis.add(&event.fiat, &DECIMAL)
        } else {
            held.basis.sub(&event.basis, &DECIMAL)
        }
        .map_err(arithmetic)?;
    }
    Ok(())
}
struct Quote {
    at: i64,
    asset: String,
    rate: Decimal,
}
fn transaction_quotes(conn: &Connection, profile: &str, needed: bool) -> Result<Vec<Quote>> {
    let mut quotes = Vec::new();
    sql::visit_rows(conn,&format!("SELECT occurred_at,asset,amount,fiat_rate,fiat_value FROM transactions WHERE profile_id=? AND excluded=0 AND (fiat_rate IS NOT NULL OR fiat_value IS NOT NULL) AND {} ORDER BY occurred_at ASC,created_at ASC,id ASC", kassiber_store::price_currency::price_in_book_currency_sql("transactions")),[profile], |row| {
        if !needed { return Ok(()); }
        // Historical Python reads these stored numeric columns, not *_exact.
        let rate = if !matches!(value(row, "fiat_rate"), Value::Null) {
            decimal(value(row, "fiat_rate"))?
        } else if truthy(value(row, "amount")) {
            decimal(value(row, "fiat_value"))?
                .div(&msat(value(row, "amount"))?, &DECIMAL)
                .map_err(arithmetic)?
        } else {
            return Ok(());
        };
        quotes.push(Quote {
            at: parse(value(row, "occurred_at"), "occurred_at")?.unwrap(),
            asset: text(row, "asset"),
            rate,
        });
        Ok(())
    })?;
    Ok(quotes)
}
fn cached_quotes(
    conn: &Connection,
    profile: &Value,
    assets: BTreeSet<String>,
    end: i64,
) -> Result<Vec<Quote>> {
    let mut quotes = Vec::new();
    for asset in assets {
        let Some(pair) =
            kassiber_rates::transaction_rate_pair(&asset, &text(profile, "fiat_currency"))
        else {
            continue;
        };
        for row in sql::optional_rows(conn,"SELECT timestamp,rate,rate_exact FROM rates_cache WHERE pair=? AND timestamp<=? ORDER BY timestamp ASC,CASE WHEN source='manual' THEN 1 ELSE 0 END ASC,fetched_at ASC,source DESC",[pair,stamp(end)?])? {
            let raw = if matches!(value(&row,"rate_exact"), Value::Null) { value(&row,"rate") } else { value(&row,"rate_exact") };
            if matches!(raw, Value::Null) { continue; }
            quotes.push(Quote { at: parse(value(&row,"timestamp"),"rate_timestamp")?.unwrap(), asset: asset.clone(), rate: decimal(raw)? });
        }
    }
    quotes.sort_by_key(|quote| quote.at);
    Ok(quotes)
}

pub fn report(conn: &Connection, profile: &Value, args: &Value) -> Result<Vec<Value>> {
    let interval = interval(args);
    let start = parse(value(args, "start"), "start")?;
    let end = parse(value(args, "end"), "end")?;
    if matches!((start,end), (Some(a),Some(b)) if a > b) {
        return Err(AppError::new(
            "validation",
            "--start must not be after --end",
        ));
    }
    let profile_id = text(profile, "id");
    let mut filters = String::new();
    let mut params = Vec::new();
    let scoped = truthy(value(args, "wallet")) || truthy(value(args, "account"));
    if truthy(value(args, "wallet")) {
        let wallet = kassiber_wallets::resolve(conn, &profile_id, &text(args, "wallet"))?;
        filters.push_str(" AND wallet_id=?");
        params.push(Sql::Text(wallet.id));
    }
    if truthy(value(args, "account")) {
        let id = account(conn, &profile_id, value(args, "account"))?;
        filters.push_str(" AND account_id=?");
        params.push(Sql::Text(id));
    }
    let asset = value(args, "asset");
    let mut pool_filters = String::new();
    let mut pool_params = Vec::new();
    if truthy(asset) {
        if matches!(asset, Value::List(_) | Value::Dict(_)) {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "Error binding parameter {}: type '{}' is not supported",
                    params.len() + 2,
                    kassiber_envelope::type_name(asset)
                ),
            ));
        }
        pool_filters.push_str(" AND asset=?");
        pool_params.push(Sql::Text(py_str(asset)));
        filters.push_str(&pool_filters);
        params.extend(pool_params.clone());
    }
    let events = events(conn, &profile_id, &filters, params)?;
    let pool_events = if scoped {
        self::events(conn, &profile_id, &pool_filters, pool_params)?
    } else {
        Vec::new()
    };
    let rates = transaction_quotes(
        conn,
        &profile_id,
        !events.is_empty() || (start.is_some() && end.is_some()),
    )?;
    if events.is_empty() && !(start.is_some() && end.is_some()) {
        return Ok(Vec::new());
    }
    let range_start = start
        .or_else(|| events.first().map(|e| e.at))
        .unwrap_or_else(time::now);
    let range_end = end.unwrap_or_else(time::now);
    if range_start > range_end {
        return Ok(Vec::new());
    }
    let mut assets: BTreeSet<String> = events.iter().map(|e| e.asset.clone()).collect();
    if truthy(asset) {
        assets.insert(py_str(asset));
    }
    let cached = cached_quotes(conn, profile, assets, range_end)?;
    let mut cumulative = BTreeMap::new();
    let mut pool = BTreeMap::new();
    let mut current_rates = BTreeMap::new();
    let mut cached_rates = BTreeMap::new();
    let (mut event_idx, mut pool_idx, mut rate_idx, mut cached_idx) = (0, 0, 0, 0);
    let mut bucket = floor(range_start, &interval)?;
    let cap = floor(range_end, &interval)?;
    let mut results = Vec::new();
    while bucket <= cap {
        let bucket_end = next(bucket, &interval)?;
        while event_idx < events.len() && events[event_idx].at < bucket_end {
            accumulate(&mut cumulative, &events[event_idx])?;
            event_idx += 1;
        }
        while pool_idx < pool_events.len() && pool_events[pool_idx].at < bucket_end {
            accumulate(&mut pool, &pool_events[pool_idx])?;
            pool_idx += 1;
        }
        while rate_idx < rates.len() && rates[rate_idx].at < bucket_end {
            current_rates.insert(rates[rate_idx].asset.clone(), rates[rate_idx].rate.clone());
            rate_idx += 1;
        }
        while cached_idx < cached.len() && cached[cached_idx].at < bucket_end {
            cached_rates.insert(
                cached[cached_idx].asset.clone(),
                cached[cached_idx].rate.clone(),
            );
            cached_idx += 1;
        }
        let emitted = if matches!(asset, Value::Null) {
            cumulative.keys().map(|s_| s(s_.as_str())).collect()
        } else {
            if matches!(asset, Value::List(_) | Value::Dict(_)) {
                return Err(AppError::new(
                    "internal_error",
                    format!("unhashable type: '{}'", kassiber_envelope::type_name(asset)),
                ));
            }
            vec![asset.clone()]
        };
        for asset in emitted {
            let zero = Holdings::default();
            let key = if matches!(asset, Value::Str(_)) {
                Some(py_str(&asset))
            } else {
                None
            };
            let held = key
                .as_ref()
                .and_then(|k| cumulative.get(k))
                .unwrap_or(&zero);
            if held.quantity == Decimal::zero() && matches!(value(args, "asset"), Value::Null) {
                continue;
            }
            let basis = if scoped {
                let pool = pool.get(&py_str(&asset)).unwrap_or(&zero);
                if pool.quantity > Decimal::zero() {
                    held.quantity
                        .mul(
                            &pool
                                .basis
                                .div(&pool.quantity, &DECIMAL)
                                .map_err(arithmetic)?,
                            &DECIMAL,
                        )
                        .map_err(arithmetic)?
                } else {
                    Decimal::zero()
                }
            } else {
                held.basis.clone()
            };
            let zero_rate = Decimal::zero();
            let rate = cached_rates
                .get(&py_str(&asset))
                .or_else(|| current_rates.get(&py_str(&asset)))
                .unwrap_or(&zero_rate);
            results.push(dict([
                ("period_start", s(stamp(bucket)?)),
                ("period_end", s(stamp(bucket_end - 1_000_000)?)),
                ("asset", asset),
                ("quantity", float(&held.quantity)?),
                ("cumulative_cost_basis", float(&basis)?),
                (
                    "market_value",
                    float(&held.quantity.mul(rate, &DECIMAL).map_err(arithmetic)?)?,
                ),
            ]));
        }
        bucket = bucket_end;
    }
    Ok(results)
}
