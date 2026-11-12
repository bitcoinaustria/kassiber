use super::*;

struct Transaction {
    id: String,
    occurred_at: String,
    confirmed_at: Option<String>,
    asset: String,
    amount: Sql,
    currency: Option<String>,
    rate: Sql,
    value: Sql,
    rate_exact: Sql,
    value_exact: Sql,
}

struct CachedRate {
    pair: String,
    timestamp: String,
    rate: Sql,
    rate_exact: Sql,
    source: String,
    fetched_at: String,
    granularity: Option<String>,
    method: Option<String>,
}

enum Price {
    Finite(Decimal),
    Special(String),
}
impl Price {
    fn parse(raw: &str) -> Result<Self, Refusal> {
        let stripped = kassiber_pytext::strip(raw);
        match stripped.parse() {
            Ok(decimal) => Ok(Self::Finite(decimal)),
            Err(kassiber_pydecimal::DecimalError::NonFinite) => {
                let normalized = stripped.chars().filter(|&c| c != '_').collect::<String>();
                let sign = if normalized.starts_with('-') { "-" } else { "" };
                let unsigned = normalized.trim_start_matches(['+', '-']).to_lowercase();
                let text = if let Some(payload) = unsigned.strip_prefix("snan") {
                    format!("{sign}sNaN{}", payload.trim_start_matches('0'))
                } else if let Some(payload) = unsigned.strip_prefix("nan") {
                    format!("{sign}NaN{}", payload.trim_start_matches('0'))
                } else {
                    format!("{sign}Infinity")
                };
                Ok(Self::Special(text))
            }
            Err(error) => {
                let mut refusal = Refusal::new("AppError", format!("Invalid decimal value: {raw}"));
                refusal.code = Some("app_error".into());
                let mut cause = Refusal::new(
                    "InvalidOperation",
                    format!("[<class 'decimal.{}'>]", error.name()),
                );
                cause.retryable = None;
                refusal.cause = Some(Box::new(cause));
                Err(refusal)
            }
        }
    }
    fn multiply(&self, quantity: &Decimal, context: Context) -> Result<Self, Refusal> {
        match self {
            Self::Finite(d) => Ok(Self::Finite(d.mul(quantity, &context).map_err(arithmetic)?)),
            Self::Special(text) if text.contains("sNaN") => Err(Refusal::new(
                "InvalidOperation",
                "[<class 'decimal.InvalidOperation'>]",
            )),
            Self::Special(text) => Ok(Self::Special(text.clone())),
        }
    }
    fn divide(&self, quantity: &Decimal, context: Context) -> Result<Self, Refusal> {
        match self {
            Self::Finite(d) => Ok(Self::Finite(d.div(quantity, &context).map_err(arithmetic)?)),
            Self::Special(text) if text.contains("sNaN") => Err(Refusal::new(
                "InvalidOperation",
                "[<class 'decimal.InvalidOperation'>]",
            )),
            Self::Special(text) => Ok(Self::Special(text.clone())),
        }
    }
    fn exact(&self) -> Result<String, Refusal> {
        match self {
            Self::Finite(d) => Ok(d.to_fixed_string()?),
            Self::Special(text) => Ok(text.clone()),
        }
    }
    fn real(&self) -> Result<Sql, Refusal> {
        Ok(Sql::Real(match self {
            Self::Finite(d) => d
                .to_string()
                .parse()
                .map_err(|_| Refusal::new("OverflowError", "decimal float conversion failed"))?,
            Self::Special(text) if text.contains("sNaN") => {
                return Err(Refusal::new(
                    "ValueError",
                    "cannot convert signaling NaN to float",
                ))
            }
            Self::Special(text) if text.contains("NaN") => f64::NAN,
            Self::Special(text) if text.starts_with('-') => f64::NEG_INFINITY,
            Self::Special(_) => f64::INFINITY,
        }))
    }
}

fn arithmetic(error: kassiber_pydecimal::DecimalError) -> Refusal {
    let family = error.name();
    Refusal::new(family, format!("[<class 'decimal.{family}'>]"))
}

fn timestamp(raw: &str) -> Result<String, Refusal> {
    kassiber_rates::timestamp(raw, "occurred_at").map_err(|error| {
        let stripped = kassiber_pytext::strip(raw);
        if kassiber_pytext::iso_datetime(stripped).is_some() {
            return Refusal::new("OverflowError", "date value out of range");
        }
        let mut refusal = app(error);
        let iso = stripped
            .strip_suffix('Z')
            .map(|s| format!("{s}+00:00"))
            .unwrap_or_else(|| stripped.into());
        let message = date_error(&iso).unwrap_or_else(|| {
            format!(
                "Invalid isoformat string: {}",
                kassiber_pytext::repr_str(&iso)
            )
        });
        refusal.cause = Some(Box::new(Refusal::new("ValueError", message)));
        refusal
    })
}

fn date_error(iso: &str) -> Option<String> {
    let bytes = iso.as_bytes();
    let extended = bytes.get(4) == Some(&b'-');
    let positions = if extended {
        [(0, 4), (5, 7), (8, 10)]
    } else {
        [(0, 4), (4, 6), (6, 8)]
    };
    if extended && bytes.get(7) != Some(&b'-') {
        return None;
    }
    let numbers = positions
        .iter()
        .map(|&(a, b)| iso.get(a..b)?.parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    let mut corrected = bytes.to_vec();
    for (&(a, b), replacement) in positions.iter().zip(["2000", "01", "01"]) {
        corrected[a..b].copy_from_slice(replacement.as_bytes());
    }
    let mut reason = None;
    let year = numbers[0];
    let month = numbers[1];
    let day = numbers[2];
    if year == 0 {
        reason = Some("year 0 is out of range".to_owned());
    }
    if reason.is_none() && !(1..=12).contains(&month) {
        reason = Some("month must be in 1..12".to_owned());
    }
    if reason.is_none() {
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        if day == 0 || day > days[(month - 1) as usize] {
            reason = Some("day is out of range for month".to_owned());
        }
    }
    let end = positions[2].1;
    if iso.len() > end {
        let separator = iso.get(end..)?.chars().next()?;
        let start = end + separator.len_utf8();
        let rest = iso.get(start..)?;
        let extended_time = rest.as_bytes().get(2) == Some(&b':');
        for (field, a, b, max) in if extended_time {
            [
                ("hour", 0, 2, 23),
                ("minute", 3, 5, 59),
                ("second", 6, 8, 59),
            ]
        } else {
            [
                ("hour", 0, 2, 23),
                ("minute", 2, 4, 59),
                ("second", 4, 6, 59),
            ]
        } {
            let Some(part) = rest.get(a..b) else {
                break;
            };
            let Ok(value) = part.parse::<u32>() else {
                break;
            };
            if value > max && reason.is_none() {
                reason = Some(format!("{field} must be in 0..{max}"));
            }
            corrected[start + a..start + b].copy_from_slice(b"00");
        }
    }
    kassiber_pytext::iso_datetime(std::str::from_utf8(&corrected).ok()?)?;
    reason
}

fn bytes_repr(bytes: &[u8]) -> String {
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') {
        b'"'
    } else {
        b'\''
    };
    let mut text = format!("b{}", char::from(quote));
    for &byte in bytes {
        match byte {
            b'\\' => text.push_str("\\\\"),
            b'\t' => text.push_str("\\t"),
            b'\n' => text.push_str("\\n"),
            b'\r' => text.push_str("\\r"),
            _ if byte == quote => {
                text.push('\\');
                text.push(char::from(byte));
            }
            32..=126 => text.push(char::from(byte)),
            _ => text.push_str(&format!("\\x{byte:02x}")),
        }
    }
    text.push(char::from(quote));
    text
}

fn first(values: &[&Sql]) -> Result<Option<Price>, Refusal> {
    for value in values {
        let raw = match value {
            Sql::Null => continue,
            Sql::Text(text) if text.is_empty() => continue,
            Sql::Text(text) => text.clone(),
            Sql::Integer(n) => n.to_string(),
            Sql::Real(n) => kassiber_pyjson::float_repr(*n),
            Sql::Blob(bytes) => bytes_repr(bytes),
        };
        return Price::parse(&raw).map(Some);
    }
    Ok(None)
}
fn exact(value: &Option<Price>) -> Result<Option<String>, Refusal> {
    value.as_ref().map(Price::exact).transpose()
}
fn real(value: &Option<Price>) -> Result<Sql, Refusal> {
    value.as_ref().map_or(Ok(Sql::Null), Price::real)
}

fn quantity(amount: &Sql, context: Context) -> Result<Option<Decimal>, Refusal> {
    let integer = match amount {
        Sql::Integer(n) if *n <= 0 => return Ok(None),
        Sql::Integer(n) => Decimal::from(*n),
        Sql::Real(n) if *n <= 0.0 => return Ok(None),
        Sql::Real(n) if n.is_finite() => format!("{:.0}", n.trunc()).parse::<Decimal>()?,
        Sql::Real(_) => {
            return Err(Refusal::new(
                "OverflowError",
                "cannot convert float infinity to integer",
            ))
        }
        Sql::Text(_) | Sql::Blob(_) | Sql::Null => {
            let kind = match amount {
                Sql::Text(_) => "str",
                Sql::Blob(_) => "bytes",
                _ => "NoneType",
            };
            return Err(Refusal::new(
                "TypeError",
                format!("'>' not supported between instances of '{kind}' and 'int'"),
            ));
        }
    };
    Ok(Some(
        integer
            .div(&Decimal::from(100_000_000_000i64), &context)
            .map_err(arithmetic)?,
    ))
}

/// A stored rate and value, either of which may be absent.
type Valuation = (Option<Price>, Option<Price>);

/// The rate and value `auto_price_transactions_from_rates_cache` stores, or
/// `None` where `finite_decimal` or `validate_transaction_price` refuses them
/// with `validation`. Other failures still abort the whole pass.
fn valuation(
    row: &Transaction,
    cached: &CachedRate,
    context: Context,
) -> Result<Option<Valuation>, Refusal> {
    let mut rate = first(&[&row.rate_exact, &cached.rate_exact, &row.rate, &cached.rate])?;
    if let Some(found) = rate {
        let Some(bounded) = bounded(found) else {
            return Ok(None);
        };
        rate = Some(bounded);
    }
    let mut value = first(&[&row.value_exact])?;
    if value.is_none() {
        if let Some(rate) = &rate {
            if let Some(quantity) = quantity(&row.amount, context)? {
                value = Some(rate.multiply(&quantity, context)?);
            }
        }
    }
    if value.is_none() {
        value = first(&[&row.value])?;
    }
    if rate.is_none() {
        if let Some(value) = &value {
            if let Some(quantity) = quantity(&row.amount, context)? {
                rate = Some(value.divide(&quantity, context)?);
            }
        }
    }
    // `pricing_payload(amount_msat=...)`: both prices, then both valuations.
    let rate = match rate {
        Some(rate) => match bounded(rate) {
            Some(rate) => Some(rate),
            None => return Ok(None),
        },
        None => None,
    };
    let value = match value {
        Some(value) => match bounded(value) {
            Some(value) => Some(value),
            None => return Ok(None),
        },
        None => None,
    };
    let rate_exact = exact(&rate)?;
    let value_exact = exact(&value)?;
    match kassiber_pydecimal::validate_transaction_price(
        "pricing input",
        &msat(&row.amount)?,
        rate_exact.as_deref(),
        value_exact.as_deref(),
        &context,
    ) {
        Ok(()) => Ok(Some((rate, value))),
        Err(kassiber_pydecimal::TransactionPriceError::Validation(_)) => Ok(None),
        Err(kassiber_pydecimal::TransactionPriceError::Arithmetic(error)) => Err(arithmetic(error)),
    }
}

/// `finite_decimal(price)`, canonical zero included; `None` is its
/// `validation` refusal.
fn bounded(price: Price) -> Option<Price> {
    let text = match price {
        Price::Finite(d) => d.to_string(),
        Price::Special(text) => text,
    };
    kassiber_pydecimal::finite_price(&text)
        .ok()
        .map(Price::Finite)
}

/// `int(row["amount"] or 0)`, as `msat_to_btc` reads a stored quantity.
fn msat(amount: &Sql) -> Result<Decimal, Refusal> {
    let invalid = |shown: String| {
        Refusal::new(
            "ValueError",
            format!("invalid literal for int() with base 10: {shown}"),
        )
    };
    Ok(match amount {
        Sql::Null => Decimal::zero(),
        Sql::Integer(n) => Decimal::from(*n),
        Sql::Real(n) if n.is_nan() => {
            return Err(Refusal::new(
                "ValueError",
                "cannot convert float NaN to integer",
            ))
        }
        Sql::Real(n) if n.is_infinite() => {
            return Err(Refusal::new(
                "OverflowError",
                "cannot convert float infinity to integer",
            ))
        }
        Sql::Real(n) => format!("{:.0}", n.trunc()).parse::<Decimal>()?,
        Sql::Text(text) if text.is_empty() => Decimal::zero(),
        Sql::Text(text) => kassiber_pytext::parse_int(text)
            .map_err(|_| invalid(kassiber_pytext::repr_str(text)))?
            .to_string()
            .parse::<Decimal>()?,
        Sql::Blob(bytes) if bytes.is_empty() => Decimal::zero(),
        Sql::Blob(bytes) => std::str::from_utf8(bytes)
            .ok()
            .filter(|text| text.is_ascii())
            .and_then(|text| kassiber_pytext::parse_int(text).ok())
            .ok_or_else(|| invalid(bytes_repr(bytes)))?
            .to_string()
            .parse::<Decimal>()?,
    })
}

pub(super) fn run(conn: &Connection, scope: &PricingScope<'_>) -> Result<usize, Refusal> {
    let mut statement = conn.prepare_cached(
        "SELECT id,occurred_at,confirmed_at,asset,amount,fiat_currency,fiat_rate,fiat_value,fiat_rate_exact,fiat_value_exact
         FROM transactions WHERE profile_id=? AND excluded=0 AND (
            ((fiat_rate IS NULL OR fiat_rate<=0)
             AND (fiat_rate_exact IS NULL OR CAST(fiat_rate_exact AS REAL)<=0)
             AND (fiat_value IS NULL OR fiat_value<=0)
             AND (fiat_value_exact IS NULL OR CAST(fiat_value_exact AS REAL)<=0))
            OR (fiat_price_source='rates_cache' AND pricing_source_kind IS NULL AND pricing_quality IS NULL))
         ORDER BY occurred_at ASC,created_at ASC,id ASC"
    )?;
    let transactions = statement
        .query_map([scope.profile_id], |r| {
            Ok(Transaction {
                id: r.get(0)?,
                occurred_at: r.get(1)?,
                confirmed_at: r.get(2)?,
                asset: r.get(3)?,
                amount: r.get(4)?,
                currency: r.get(5)?,
                rate: r.get(6)?,
                value: r.get(7)?,
                rate_exact: r.get(8)?,
                value_exact: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if transactions.is_empty() {
        return Ok(0);
    }
    let registered: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_function_list WHERE name='kassiber_timestamp_us')",
        [],
        |r| r.get(0),
    )?;
    if !registered {
        conn.create_scalar_function(
            "kassiber_timestamp_us",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| {
                Ok(match ctx.get_raw(0) {
                    rusqlite::types::ValueRef::Text(raw) => std::str::from_utf8(raw)
                        .ok()
                        .and_then(kassiber_rates::time::parse),
                    _ => None,
                })
            },
        )?;
    }
    let mut lookup = conn.prepare_cached("SELECT pair,timestamp,rate,rate_exact,source,fetched_at,granularity,method
        FROM rates_cache WHERE pair=? AND kassiber_timestamp_us(timestamp)<=kassiber_timestamp_us(?)
        ORDER BY timestamp DESC,CASE WHEN source='manual' THEN 0 ELSE 1 END ASC,fetched_at DESC LIMIT 1")?;
    let mut update = conn.prepare_cached("UPDATE transactions SET fiat_rate=?,fiat_value=?,fiat_price_source=?,fiat_rate_exact=?,fiat_value_exact=?,
        pricing_source_kind=?,pricing_provider=?,pricing_pair=?,pricing_timestamp=?,pricing_fetched_at=?,
        pricing_granularity=?,pricing_method=?,pricing_external_ref=?,pricing_quality=? WHERE id=?")?;
    let mut unsupported = conn.prepare_cached(
        "UPDATE transactions SET pricing_method='unsupported_price_range', pricing_quality='missing' WHERE id=?",
    )?;
    let mut count = 0;
    for row in transactions {
        let missing = [&row.rate, &row.value, &row.rate_exact, &row.value_exact]
            .iter()
            .all(|v| matches!(v, Sql::Null));
        let currency = row
            .currency
            .as_deref()
            .filter(|c| !c.is_empty())
            .unwrap_or(scope.fiat_currency);
        let Some(pair) = kassiber_rates::transaction_rate_pair(&row.asset, currency) else {
            continue;
        };
        let pricing_at = row
            .confirmed_at
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(&row.occurred_at);
        let pricing_at = timestamp(pricing_at)?;
        let mut cached_rows = lookup.query(params![pair, pricing_at])?;
        let Some(cached) = cached_rows.next()? else {
            continue;
        };
        let cached = CachedRate {
            pair: cached.get(0)?,
            timestamp: cached.get(1)?,
            rate: cached.get(2)?,
            rate_exact: cached.get(3)?,
            source: cached.get(4)?,
            fetched_at: cached.get(5)?,
            granularity: cached.get(6)?,
            method: cached.get(7)?,
        };
        drop(cached_rows);
        // Round 5 (f823b4729): a cached valuation outside the shared price
        // bounds stores no price; the row is marked for a later valid cache.
        let Some((rate, value)) = valuation(&row, &cached, scope.decimal_context)? else {
            unsupported.execute([&row.id])?;
            continue;
        };
        let manual = kassiber_pytext::lower(kassiber_pytext::strip(&cached.source)) == "manual";
        let source_kind = if manual {
            "manual_rate_cache"
        } else {
            "fmv_provider"
        };
        let granularity = kassiber_pytext::lower(kassiber_pytext::strip(
            cached.granularity.as_deref().unwrap_or(""),
        ));
        let quality = if manual {
            "exact"
        } else if ["daily", "coarse", "monthly", "yearly"].contains(&granularity.as_str()) {
            "coarse_fallback"
        } else {
            "provider_sample"
        };
        let rate_exact = exact(&rate)?;
        let value_exact = exact(&value)?;
        update.execute(params![
            real(&rate)?,
            real(&value)?,
            "rates_cache",
            rate_exact,
            value_exact,
            source_kind,
            cached.source,
            cached.pair,
            cached.timestamp,
            cached.fetched_at,
            cached.granularity,
            cached.method,
            Option::<String>::None,
            quality,
            row.id
        ])?;
        if missing {
            count += 1;
        }
    }
    Ok(count)
}
