//! Fiat exchange rates. Python's core/rates.py remains the behavior reference.
//!
//! Cache reads never construct a client or grant network authority. Rates retain
//! their exact Decimal spelling alongside the legacy REAL projection. Pricing
//! callers use `cached_at_or_before`; there is no interpolation or TTL in this
//! Python base. Provider fetching requires a separate, exact-action grant.
mod csv;
mod desktop;
pub use desktop::run as desktop_command;
mod providers;
pub mod time;
pub use providers::{require_live_enabled, FetchAction, FetchConsent, Providers};

use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pydecimal::Decimal;
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{self, params, types::ValueRef, Connection, OptionalExtension};

pub type Result<T> = std::result::Result<T, AppError>;
pub const PAIRS: &[&str] = &["BTC-USD", "BTC-EUR"];
pub const SOURCES: &[&str] = &["coinbase-exchange", "mempool", "kraken-csv", "coingecko"];
pub const LIVE_SOURCES: &[&str] = &["coinbase-exchange", "coingecko", "mempool"];
const COLUMNS: &str = "pair, timestamp, rate, rate_exact, source, fetched_at, granularity, method";
const LATEST_ORDER: &str = "timestamp DESC, CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC, fetched_at DESC, source ASC";
// The pricing lookup in this base intentionally has no source/id tie-break.
const LOOKUP_ORDER: &str =
    "timestamp DESC, CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC, fetched_at DESC";

pub(crate) fn sql_error(e: rusqlite::Error) -> AppError {
    let e = kassiber_store::map_sqlite_error(e);
    AppError::new(e.code, e.message).retryable(e.retryable)
}
pub(crate) fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
pub(crate) fn text<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    match get(args, name) {
        Some(Value::Str(v)) => v.as_str(),
        _ => None,
    }
}
pub(crate) fn optional(value: Option<&str>) -> Value {
    value.map_or(Value::Null, s)
}
pub(crate) fn integer(args: &Value, name: &str, default: i64) -> Result<i64> {
    let Some(v) = get(args, name).filter(|v| !matches!(v, Value::Null)) else {
        return Ok(default);
    };
    match v {
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Float(f) if f.is_finite() && *f >= i64::MIN as f64 && *f < i64::MAX as f64 => {
            Ok(*f as i64)
        }
        _ => kassiber_pytext::parse_int(&py_str(v))
            .ok()
            .and_then(|n| i64::try_from(n).ok())
            .ok_or_else(|| validation(format!("{name} must be an integer"))),
    }
}
pub fn normalize_pair(pair: &str) -> Result<String> {
    if pair.is_empty() {
        return Err(validation("Pair is required"));
    }
    let raw = kassiber_pytext::strip(pair)
        .to_uppercase()
        .replace('/', "-");
    let Some((asset, fiat)) = raw.split_once('-') else {
        return Err(validation(format!("Invalid pair '{pair}'"))
            .with_hint("Use <ASSET>-<FIAT>, e.g. BTC-USD"));
    };
    if asset.is_empty() || fiat.is_empty() {
        return Err(validation(format!("Invalid pair '{pair}'")));
    }
    Ok(raw)
}
pub fn require_supported_pair(pair: &str) -> Result<String> {
    let pair = normalize_pair(pair)?;
    if !PAIRS.contains(&pair.as_str()) {
        return Err(validation(format!("Pair '{pair}' is not supported"))
            .with_hint("Supported pairs: BTC-USD, BTC-EUR"));
    }
    Ok(pair)
}
pub fn transaction_rate_pair(asset: &str, fiat: &str) -> Option<String> {
    let asset = kassiber_pytext::strip(asset).to_uppercase();
    let asset = if asset == "LBTC" { "BTC" } else { &asset };
    let pair = format!("{asset}-{}", kassiber_pytext::strip(fiat).to_uppercase());
    PAIRS.contains(&pair.as_str()).then_some(pair)
}
pub fn normalize_market_provider(source: &str) -> Result<String> {
    let normalized = kassiber_pytext::strip(if source.is_empty() {
        "coinbase-exchange"
    } else {
        source
    })
    .to_lowercase();
    if !LIVE_SOURCES.contains(&normalized.as_str()) {
        return Err(validation(format!(
            "Market-rate provider '{source}' is not supported for automatic refresh"
        ))
        .with_hint(
            "Supported automatic market-rate providers: coinbase-exchange, coingecko, mempool",
        ));
    }
    Ok(normalized)
}
pub fn market_provider(conn: &Connection) -> Result<String> {
    Ok(normalize_market_provider(
        setting(conn, "market_rate_provider")?
            .as_deref()
            .unwrap_or(""),
    )
    .unwrap_or_else(|_| "coinbase-exchange".into()))
}
pub fn set_market_provider(conn: &Connection, source: &str) -> Result<String> {
    let source = normalize_market_provider(source)?;
    conn.execute("INSERT INTO settings(key,value) VALUES('market_rate_provider',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&source]).map_err(sql_error)?;
    Ok(source)
}
pub(crate) fn setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(sql_error)
}
pub fn timestamp(value: &str, field: &str) -> Result<String> {
    time::parse(value).and_then(time::format).ok_or_else(|| {
        validation(format!("Invalid {field} timestamp '{value}'"))
            .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
    })
}
pub fn decimal(value: &str) -> Result<Decimal> {
    kassiber_pytext::strip(value)
        .parse()
        .map_err(|_| validation(format!("Invalid rate '{value}'")))
}
fn exact(value: Option<&str>) -> Result<(Option<f64>, Option<String>)> {
    let Some(raw) = value.filter(|s| !s.is_empty()) else {
        return Ok((None, None));
    };
    let d = kassiber_pydecimal::finite_price(raw)
        .map_err(|e| validation(e.message("rate cache value")))?;
    let fixed = d
        .to_fixed_string()
        .map_err(|_| validation(format!("Invalid rate '{raw}'")))?;
    let f = d
        .to_string()
        .parse::<f64>()
        .map_err(|_| validation(format!("Invalid rate '{raw}'")))?;
    Ok((Some(f), Some(fixed)))
}
#[derive(Default, Clone)]
pub struct Sample<'a> {
    pub pair: &'a str,
    pub timestamp: &'a str,
    pub rate: &'a str,
    pub source: &'a str,
    pub fetched_at: &'a str,
    pub granularity: Option<&'a str>,
    pub method: Option<&'a str>,
    pub open: Option<&'a str>,
    pub high: Option<&'a str>,
    pub low: Option<&'a str>,
    pub close: Option<&'a str>,
    pub volume: Option<&'a str>,
    pub trades: Option<i64>,
}
pub fn upsert(conn: &Connection, sample: &Sample<'_>) -> Result<Value> {
    let pair = normalize_pair(sample.pair)?;
    let ts = timestamp(sample.timestamp, "rate_timestamp")?;
    let rate = exact(Some(sample.rate))?;
    if rate.1.is_none() {
        return Err(validation(format!("Invalid rate '{}'", sample.rate)));
    }
    let open = exact(sample.open)?;
    let high = exact(sample.high)?;
    let low = exact(sample.low)?;
    let close = exact(sample.close.or(Some(sample.rate)))?;
    let volume = exact(sample.volume)?;
    begin_write(conn)?;
    conn.prepare_cached("INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at,granularity,method,open_rate,open_rate_exact,high_rate,high_rate_exact,low_rate,low_rate_exact,close_rate,close_rate_exact,volume,volume_exact,trades) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19) ON CONFLICT(pair,timestamp,source) DO UPDATE SET rate=excluded.rate,rate_exact=excluded.rate_exact,fetched_at=excluded.fetched_at,granularity=excluded.granularity,method=excluded.method,open_rate=excluded.open_rate,open_rate_exact=excluded.open_rate_exact,high_rate=excluded.high_rate,high_rate_exact=excluded.high_rate_exact,low_rate=excluded.low_rate,low_rate_exact=excluded.low_rate_exact,close_rate=excluded.close_rate,close_rate_exact=excluded.close_rate_exact,volume=excluded.volume,volume_exact=excluded.volume_exact,trades=excluded.trades").map_err(sql_error)?.execute(params![pair,ts,rate.0,rate.1,sample.source,sample.fetched_at,sample.granularity,sample.method,open.0,open.1,high.0,high.1,low.0,low.1,close.0,close.1,volume.0,volume.1,sample.trades]).map_err(sql_error)?;
    Ok(dict([
        ("pair", s(pair)),
        ("timestamp", s(ts)),
        ("rate", rate.0.map_or(Value::Null, Value::Float)),
        ("rate_exact", optional(rate.1.as_deref())),
        ("source", s(sample.source)),
        ("fetched_at", s(sample.fetched_at)),
        ("granularity", optional(sample.granularity)),
        ("method", optional(sample.method)),
    ]))
}
pub(crate) fn invalidate(conn: &Connection, pair: &str) -> Result<()> {
    if PAIRS.contains(&pair) {
        begin_write(conn)?;
        conn.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE upper(fiat_currency)=?",[pair.split_once('-').unwrap().1]).map_err(sql_error)?;
    }
    Ok(())
}
pub fn set_manual(conn: &Connection, args: &Value) -> Result<Value> {
    let pair = normalize_pair(text(args, "pair").unwrap_or(""))?;
    let raw = text(args, "rate").unwrap_or("");
    let d = kassiber_pydecimal::finite_price(raw).map_err(|e| validation(e.message("rate")))?;
    if d <= Decimal::zero() {
        return Err(validation("Rate must be positive"));
    }
    let source = text(args, "source").unwrap_or("manual");
    let granularity = text(args, "granularity")
        .filter(|s| !s.is_empty())
        .unwrap_or(if source == "manual" {
            "exact"
        } else {
            "unknown"
        });
    let method = text(args, "method")
        .filter(|s| !s.is_empty())
        .unwrap_or(if source == "manual" {
            "manual"
        } else {
            "operator_supplied"
        });
    let now = time::iso_now();
    let row = upsert(
        conn,
        &Sample {
            pair: &pair,
            timestamp: text(args, "timestamp").unwrap_or(""),
            rate: &d.to_string(),
            source,
            fetched_at: &now,
            granularity: Some(granularity),
            method: Some(method),
            ..Default::default()
        },
    )?;
    invalidate(conn, &pair)?;
    Ok(row)
}
fn row_value(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let mut entries = Vec::new();
    for (i, name) in row.as_ref().column_names().iter().enumerate() {
        let value = match row.get_ref(i)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => Value::from(n),
            ValueRef::Real(n) => Value::Float(n),
            ValueRef::Text(v) => s(String::from_utf8_lossy(v).into_owned()),
            ValueRef::Blob(_) => {
                return Err(rusqlite::Error::InvalidColumnType(
                    i,
                    name.to_string(),
                    rusqlite::types::Type::Blob,
                ))
            }
        };
        entries.push((kassiber_pyjson::Key::from(*name), value));
    }
    Ok(Value::Dict(entries))
}
pub(crate) fn query(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Value>> {
    conn.prepare(sql)
        .map_err(sql_error)?
        .query_map(params, row_value)
        .map_err(sql_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_error)
}
pub fn latest(conn: &Connection, pair: &str) -> Result<Value> {
    let pair = normalize_pair(pair)?;
    query(
        conn,
        &format!("SELECT {COLUMNS} FROM rates_cache WHERE pair=? ORDER BY {LATEST_ORDER} LIMIT 1"),
        [&pair],
    )?
    .pop()
    .ok_or_else(|| {
        AppError::new("not_found", format!("No cached rate for pair '{pair}'"))
            .with_hint("Run `kassiber rates sync` first")
    })
}
pub fn cached_at_or_before(
    conn: &Connection,
    pair: &str,
    occurred_at: &str,
) -> Result<Option<Value>> {
    let pair = require_supported_pair(pair)?;
    let ts = timestamp(occurred_at, "occurred_at")?;
    Ok(query(conn,&format!("SELECT {COLUMNS} FROM rates_cache WHERE pair=? AND timestamp <= ? ORDER BY {LOOKUP_ORDER} LIMIT 1"),[pair,ts])?.pop())
}
pub fn range(conn: &Connection, args: &Value) -> Result<Value> {
    let pair = normalize_pair(text(args, "pair").unwrap_or(""))?;
    let limit = get(args, "limit")
        .filter(|v| !matches!(v, Value::Null))
        .map(|_| integer(args, "limit", 0))
        .transpose()?;
    if limit.is_some_and(|v| v <= 0) {
        return Err(validation("--limit must be positive"));
    }
    let order = text(args, "order").unwrap_or("asc");
    if !["asc", "desc"].contains(&order) {
        return Err(validation("--order must be asc or desc"));
    }
    let mut sql = format!("SELECT {COLUMNS} FROM rates_cache WHERE pair=?");
    let mut params = vec![rusqlite::types::Value::Text(pair)];
    for (field, op) in [("start", ">="), ("end", "<=")] {
        if let Some(v) = text(args, field).filter(|v| !v.is_empty()) {
            sql += &format!(" AND timestamp {op} ?");
            params.push(timestamp(v, field)?.into());
        }
    }
    sql+=&format!(" ORDER BY timestamp {order}, CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC, fetched_at DESC, source ASC");
    if let Some(n) = limit {
        sql += " LIMIT ?";
        params.push(n.into())
    }
    Ok(Value::List(query(
        conn,
        &sql,
        rusqlite::params_from_iter(params),
    )?))
}
pub fn pairs(conn: &Connection) -> Result<Value> {
    let mut rows=query(conn,"SELECT pair, COUNT(*) AS sample_count, MIN(timestamp) AS first_timestamp, MAX(timestamp) AS last_timestamp FROM rates_cache GROUP BY pair ORDER BY pair ASC",[])?;
    let mut names: Vec<String> = PAIRS.iter().map(|s| s.to_string()).collect();
    names.extend(
        rows.iter()
            .filter_map(|r| text(r, "pair"))
            .filter(|p| !PAIRS.contains(p))
            .map(str::to_owned),
    );
    Ok(Value::List(
        names
            .into_iter()
            .map(|pair| {
                let row = rows
                    .iter()
                    .position(|r| text(r, "pair") == Some(&pair))
                    .map(|i| rows.remove(i));
                let field = |k| {
                    row.as_ref()
                        .and_then(|r| get(r, k))
                        .cloned()
                        .unwrap_or(Value::Null)
                };
                dict([
                    ("pair", s(pair.as_str())),
                    ("supported", Value::Bool(PAIRS.contains(&pair.as_str()))),
                    ("cached", Value::Bool(row.is_some())),
                    (
                        "sample_count",
                        if row.is_some() {
                            field("sample_count")
                        } else {
                            Value::from(0)
                        },
                    ),
                    ("first_timestamp", field("first_timestamp")),
                    ("last_timestamp", field("last_timestamp")),
                ])
            })
            .collect(),
    ))
}
/// Local cache operations. Low-level writes leave commit/rollback to the caller;
/// `command` supplies the CLI boundary and sync owns its provider batch commits.
pub fn local_command(conn: &Connection, command: &str, args: &Value) -> Result<Value> {
    match command {
        "pairs" => pairs(conn),
        "latest" => latest(conn, text(args, "pair").unwrap_or("")),
        "range" => range(conn, args),
        "set" => set_manual(conn, args),
        _ => Err(AppError::new(
            "not_implemented",
            "Rates operation is not implemented",
        )),
    }
}

#[cfg(test)]
mod tests;

pub fn command(
    conn: &Connection,
    command: &str,
    args: &Value,
    providers: &Providers,
    consent: Option<&FetchConsent>,
) -> Result<Value> {
    if ["pairs", "latest", "range"].contains(&command) {
        return local_command(conn, command, args);
    }
    if command == "sync" {
        return providers.sync(conn, args, consent, FetchAction::Sync);
    }
    conn.execute_batch("SAVEPOINT rates_command")
        .map_err(sql_error)?;
    let result = match command {
        "set" => set_manual(conn, args),
        "rebuild" => rebuild(conn, args, providers, consent),
        _ => Err(AppError::new(
            "not_implemented",
            "Rates operation is not implemented",
        )),
    };
    match result {
        Ok(v) => {
            conn.execute_batch("RELEASE SAVEPOINT rates_command")
                .map_err(sql_error)?;
            Ok(v)
        }
        Err(e) => {
            conn.execute_batch(
                "ROLLBACK TO SAVEPOINT rates_command; RELEASE SAVEPOINT rates_command",
            )
            .map_err(sql_error)?;
            Err(e)
        }
    }
}
fn rebuild(
    conn: &Connection,
    args: &Value,
    providers: &Providers,
    consent: Option<&FetchConsent>,
) -> Result<Value> {
    let raw = text(args, "source").unwrap_or("coinbase-exchange");
    let source = raw.trim().to_lowercase();
    if !SOURCES.contains(&source.as_str()) {
        return Err(validation(format!("Unknown rate source '{raw}'"))
            .with_hint("Supported sources: coinbase-exchange, mempool, kraken-csv, coingecko"));
    }
    let pair = text(args, "pair")
        .filter(|v| !v.is_empty())
        .map(require_supported_pair)
        .transpose()?;
    let path = text(args, "path").filter(|v| !v.is_empty());
    if source == "kraken-csv" && path.is_none() {
        return Err(validation("--path is required for --source kraken-csv"));
    }
    if source != "kraken-csv" && path.is_some() {
        return Err(validation(
            "--path is only supported for --source kraken-csv",
        ));
    }
    let days = integer(args, "days", 30)?;
    if days <= 0 {
        return Err(validation("--days must be positive"));
    }
    let reprice = get(args, "reprice_transactions").is_some_and(truthy);
    let mut transactions = 0;
    let mut profiles = std::collections::BTreeSet::new();
    if reprice {
        let profile = text(args, "profile_id");
        let rows=query(conn,"SELECT t.id,t.profile_id,t.asset,t.fiat_currency,p.fiat_currency AS profile_fiat_currency,t.pricing_source_kind,t.pricing_provider,t.pricing_pair FROM transactions t JOIN profiles p ON p.id=t.profile_id WHERE (t.pricing_source_kind='fmv_provider' OR (t.fiat_price_source='rates_cache' AND t.pricing_source_kind IS NULL AND t.pricing_quality IS NULL))",[])?;
        for row in rows {
            if profile.is_some_and(|p| text(&row, "profile_id") != Some(p)) {
                continue;
            }
            if text(&row, "pricing_source_kind") == Some("fmv_provider")
                && text(&row, "pricing_provider") != Some(&source)
            {
                continue;
            }
            let candidate = text(&row, "pricing_pair")
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    transaction_rate_pair(
                        text(&row, "asset").unwrap_or(""),
                        text(&row, "fiat_currency")
                            .filter(|v| !v.is_empty())
                            .or_else(|| text(&row, "profile_fiat_currency"))
                            .unwrap_or(""),
                    )
                });
            let selected = if let Some(pair) = &pair {
                candidate.as_ref() == Some(pair)
            } else {
                candidate.as_deref().is_some_and(|p| PAIRS.contains(&p))
            };
            if !selected {
                continue;
            }
            conn.execute("UPDATE transactions SET fiat_rate=NULL,fiat_value=NULL,fiat_price_source=NULL,fiat_rate_exact=NULL,fiat_value_exact=NULL,pricing_source_kind=NULL,pricing_provider=NULL,pricing_pair=NULL,pricing_timestamp=NULL,pricing_fetched_at=NULL,pricing_granularity=NULL,pricing_method=NULL,pricing_external_ref=NULL,pricing_quality=NULL WHERE id=?",[text(&row,"id").unwrap_or("")]).map_err(sql_error)?;
            profiles.insert(text(&row, "profile_id").unwrap_or("").to_owned());
            transactions += 1;
        }
        for profile in &profiles {
            conn.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?",[profile]).map_err(sql_error)?;
        }
    }
    let mut deleted = Vec::new();
    for table in ["rates_cache", "rates_checked_minutes"] {
        let sql = format!(
            "DELETE FROM {table} WHERE source=?{}",
            if pair.is_some() { " AND pair=?" } else { "" }
        );
        let n = if let Some(pair) = &pair {
            conn.execute(&sql, params![source, pair])
        } else {
            conn.execute(&sql, [&source])
        }
        .map_err(sql_error)?;
        deleted.push(n);
    }
    let summary = providers.sync(conn, args, consent, FetchAction::Rebuild)?;
    Ok(dict([
        ("source", s(source)),
        ("pair", optional(pair.as_deref())),
        ("days", Value::from(days)),
        ("reprice_transactions", Value::Bool(reprice)),
        (
            "deleted",
            dict([
                ("rates", Value::from(deleted[0])),
                ("checked_minutes", Value::from(deleted[1])),
                ("transaction_prices", Value::from(transactions)),
                ("profiles_invalidated", Value::from(profiles.len())),
            ]),
        ),
        ("sync", summary),
    ]))
}

pub(crate) fn begin_write(conn: &Connection) -> Result<()> {
    if conn.is_autocommit() {
        conn.execute_batch("BEGIN DEFERRED").map_err(sql_error)?;
    }
    Ok(())
}
pub(crate) fn commit(conn: &Connection) -> Result<()> {
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(sql_error)?;
    }
    Ok(())
}

/// Seed packaged Kraken history only for pairs absent at the requested
/// granularity. Existing hourly history never counts as a daily seed.
pub fn ensure_bundled_seed(conn: &Connection, pair: Option<&str>, hourly: bool) -> Result<Value> {
    let pairs = pair
        .map(require_supported_pair)
        .transpose()?
        .map(|p| vec![p])
        .unwrap_or_else(|| PAIRS.iter().map(|p| p.to_string()).collect());
    let granularity = if hourly { "hourly" } else { "daily" };
    let mut missing = Vec::new();
    for pair in &pairs {
        let present:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM rates_cache WHERE pair=? AND source='kraken-csv' AND granularity=? AND method='ohlcvt_csv')",params![pair,granularity],|r|r.get(0)).map_err(sql_error)?;
        if !present {
            missing.push(pair.clone());
        }
    }
    if missing.is_empty() {
        return Ok(Value::List(
            pairs
                .iter()
                .map(|pair| {
                    dict([
                        ("pair", s(pair.as_str())),
                        ("source", s("kraken-csv")),
                        ("samples", Value::from(0)),
                        ("rows", Value::from(0)),
                        ("files", Value::from(0)),
                        ("skipped_rows", Value::from(0)),
                        ("skipped_files", Value::from(0)),
                        ("granularity", s(granularity)),
                        ("method", s("ohlcvt_csv")),
                        ("already_seeded", Value::Bool(true)),
                        ("fetched_at", Value::Null),
                    ])
                })
                .collect(),
        ));
    }
    let mut rows = csv::sync_bundled(
        conn,
        if missing.len() == 1 {
            Some(missing[0].as_str())
        } else {
            None
        },
        hourly,
    )?;
    if let Value::List(rows) = &mut rows {
        for row in rows {
            if let Value::Dict(fields) = row {
                fields.push(("already_seeded".into(), Value::Bool(false)));
            }
        }
    }
    commit(conn)?;
    Ok(rows)
}
