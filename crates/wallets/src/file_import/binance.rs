use super::exchange::Columns;
use super::river::{abs, arithmetic, number, repr, truth, zero};
use super::*;

const AUTOINVEST: &[&str] = &[
    "timestamp utc",
    "base asset symbol",
    "quote asset amount + symbol",
    "trading fee (in quote asset)",
    "base asset amount + symbol",
];
const DIVIDENDS: &[&str] = &["id", "amount", "asset", "divtime"];
const FIAT: &[&str] = &[
    "AED", "ARS", "AUD", "BRL", "CAD", "CHF", "CLP", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD",
    "HUF", "ILS", "INR", "JPY", "KRW", "MXN", "NOK", "NZD", "PLN", "RON", "SEK", "SGD", "THB",
    "TRY", "USD", "ZAR",
];

pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    let Some(first) = rows.first() else {
        return Ok(rows);
    };
    let has = |required: &[&str]| {
        required
            .iter()
            .all(|key| first.headers.iter().any(|h| river::column(h) == *key))
    };
    let autoinvest = has(AUTOINVEST);
    if !autoinvest && !has(DIVIDENDS) {
        return Err(AppError::new(
            "validation",
            "Binance supplemental CSV is not a supported BTC supplemental export",
        )
        .with_hint(
            "Use Binance autoinvest CSV or asset-dividend rows with id, amount, asset, divTime.",
        ));
    }
    rows.into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let columns = Columns::new(&row);
            if autoinvest {
                investment(&columns, index + 1)
            } else {
                dividend(&columns, index + 1)
            }
            .transpose()
        })
        .collect::<Result<_, _>>()
        .map(synthetic::number_occurrences)
}
fn currency(value: Option<String>) -> Option<String> {
    value.map(|v| asset(&v))
}
fn split(
    value: Option<String>,
    field: &str,
) -> Result<(Option<PriceNumber>, Option<String>), AppError> {
    let Some(value) = value.filter(|s| !kassiber_pytext::strip(s).is_empty()) else {
        return Ok((None, None));
    };
    let parts = value
        .split(|c: char| kassiber_pytext::is_space(u32::from(c)))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return Err(AppError::new(
            "validation",
            format!("Binance supplemental CSV has invalid {field}"),
        )
        .with_hint(format!("Expected '<amount> <asset>' in {field}.")));
    }
    Ok((
        number(Some(parts[0].into()))?,
        currency(Some(parts[1].into())),
    ))
}
pub(super) fn investment(c: &Columns, index: usize) -> Result<Option<csv::Row>, AppError> {
    let base = currency(c.opt(&["base asset symbol", "Base Asset Symbol"]));
    let (quote, fiat) = split(
        c.get(&["quote asset amount + symbol", "Quote Asset Amount + Symbol"]),
        "quote asset amount",
    )?;
    let (amount, parsed) = split(
        c.get(&["base asset amount + symbol", "Base Asset Amount + Symbol"]),
        "base asset amount",
    )?;
    // A declared Bitcoin base and a different amount suffix cannot both set
    // the quantity unit.
    if let (Some(declared), Some(suffix)) = (
        base.as_deref().filter(|s| !s.is_empty()),
        parsed.as_deref().filter(|s| !s.is_empty()),
    ) {
        let declared_bitcoin = exchange::bitcoin(Some(declared));
        let suffix_bitcoin = exchange::bitcoin(Some(suffix));
        if (declared_bitcoin.is_some() || suffix_bitcoin.is_some())
            && declared_bitcoin.unwrap_or(declared) != suffix_bitcoin.unwrap_or(suffix)
        {
            return Err(AppError::new(
                "validation",
                "Binance supplemental base asset symbols conflict",
            ));
        }
    }
    let fee_text = c.opt(&["trading fee (in quote asset)", "Trading Fee"]);
    let (fee, fee_asset) = match fee_text.as_deref() {
        None | Some("--" | "-") => (Some(zero()), None),
        _ => split(fee_text, "trading fee")?,
    };
    let Some(base) =
        exchange::bitcoin(base.as_deref()).or_else(|| exchange::bitcoin(parsed.as_deref()))
    else {
        return Ok(None);
    };
    let (Some(amount), Some(quote)) = (amount, quote) else {
        return Err(AppError::new(
            "app_error",
            "Binance supplemental CSV has a BTC autoinvest row with an empty amount",
        ));
    };
    if !fiat.as_deref().is_some_and(|f| FIAT.contains(&f)) {
        return Err(AppError::new("validation", "Binance supplemental BTC autoinvest rows must be funded by fiat for exact import")
            .with_hint("Crypto-funded Binance autoinvest rows are cross-asset trades; use the generic ledger with explicit review instead."));
    }
    if fee_asset.is_some() && fee_asset != fiat {
        return Err(AppError::new(
            "validation",
            "Binance supplemental BTC autoinvest fee asset does not match the quote asset",
        ));
    }
    let occurred = c.get(&["timestamp utc", "Timestamp UTC"]);
    let source = c.opt(&["source of funds", "Source of Funds"]);
    let _ = index;
    let fee = abs(fee.filter(truth).unwrap_or_else(zero))?;
    let quote = abs(quote)?;
    let value = arithmetic(&quote, &fee, '+')?;
    let reference = {
        use synthetic::Part::{Number, Text};
        synthetic::economic_id(
            "binance-supplemental:autoinvest",
            &[
                Text(occurred.as_deref()),
                Text(Some(base)),
                Number(&abs(amount.clone())?),
                Text(fiat.as_deref()),
                Number(&quote),
                Text(fee_asset.as_deref()),
                Number(&fee),
            ],
        )?
    };
    let amount = abs(amount)?;
    let rate = if truth(&amount) {
        Some(arithmetic(&value, &amount, '/')?)
    } else {
        None
    };
    Ok(Some(csv::Row {
        headers: vec![],
        rest: None,
        original: None,
        data: json!({"txid":reference,"occurred_at":occurred,"direction":"inbound","asset":base,"amount":repr(&amount),"fee":"0","fiat_rate":rate.as_ref().map(repr),"fiat_value":repr(&value),"fiat_currency":fiat,"pricing_source_kind":"exchange_execution","pricing_provider":"Binance","pricing_pair":format!("{base}-{}",fiat.as_deref().unwrap()),"pricing_timestamp":occurred,"pricing_method":"binance_supplemental_csv","pricing_external_ref":reference,"pricing_quality":"exact","kind":"buy","description":source.map_or_else(||"Binance autoinvest buy".into(), |s|format!("Binance autoinvest buy - {s}")),"counterparty":"Binance","raw_json":config::stored_json(&c.json()),"_synthetic_csv_id":true}),
    }))
}
fn epoch(value: Option<String>) -> Result<Option<String>, AppError> {
    let Some(value) = value.filter(|v| !kassiber_pytext::strip(v).is_empty()) else {
        return Ok(None);
    };
    let parsed = PriceNumber::parse(&value).map_err(|_| {
        AppError::new(
            "app_error",
            "Binance supplemental CSV has invalid millisecond timestamp",
        )
    })?;
    let seconds = arithmetic(&parsed, &PriceNumber::Finite(dec("1000")?), '/')?;
    let value = match &seconds {
        PriceNumber::Special(v) if v.contains("NaN") => f64::NAN,
        _ => kassiber_pytext::parse_float(&repr(&seconds)).unwrap_or(f64::INFINITY),
    };
    if value.is_nan() {
        return Err(AppError::new(
            "internal_error",
            "Invalid value NaN (not a number)",
        ));
    }
    if !value.is_finite() {
        return Err(AppError::new(
            "internal_error",
            "timestamp out of range for platform time_t",
        ));
    }
    let integral = value.trunc();
    let micros = (value.fract() * 1_000_000.).round_ties_even();
    let seconds = integral + (micros / 1_000_000.).floor();
    if seconds.abs() >= i64::MAX as f64 {
        return Err(AppError::new(
            "internal_error",
            "timestamp out of range for platform time_t",
        ));
    }
    let seconds = seconds as i64;
    if !(-62_135_596_800..253_402_300_800).contains(&seconds) {
        let day = seconds.div_euclid(86400) + 719468;
        let era = day.div_euclid(146097);
        let doe = day - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let month = (5 * doy + 2) / 153;
        let year = yoe + era * 400 + i64::from(month >= 10);
        if !(i64::from(i32::MIN) + 1900..=i64::from(i32::MAX) + 1900).contains(&year) {
            return Err(AppError::new(
                "internal_error",
                "[Errno 75] Value too large for defined data type",
            ));
        }
        return Err(AppError::new(
            "internal_error",
            format!("year {year} is out of range"),
        ));
    }
    crate::time::shifted("1970-01-01T00:00:00Z", seconds).map(Some)
}
pub(super) fn dividend(c: &Columns, index: usize) -> Result<Option<csv::Row>, AppError> {
    let currency = currency(c.opt(&["asset", "Asset"]));
    let Some(asset) = exchange::bitcoin(currency.as_deref()) else {
        return Ok(None);
    };
    let Some(amount) = number(c.get(&["amount", "Amount"]))? else {
        return Ok(None);
    };
    if repr(&amount).contains("sNaN") {
        return Err(decimal_error(DecimalError::InvalidOperation));
    }
    if !truth(&amount) {
        return Ok(None);
    }
    let _ = index;
    let provided = c.opt(&["id", "tranId", "tran id"]);
    let synthetic_id = provided.is_none();
    let occurred = epoch(c.get(&["divTime", "divtime", "time"]))?
        .or_else(|| c.get(&["timestamp", "Timestamp"]));
    let info = c.opt(&["enInfo", "info", "description"]);
    let kind = if info
        .as_ref()
        .is_some_and(|s| river::column(s).contains("mining"))
    {
        "mining"
    } else {
        "income"
    };
    let reference = match provided {
        Some(reference) => reference,
        None => {
            use synthetic::Part::{Number, Text};
            synthetic::economic_id(
                "binance:dividend",
                &[
                    Text(occurred.as_deref()),
                    Text(Some(asset)),
                    Number(&abs(amount.clone())?),
                    Text(Some(kind)),
                ],
            )?
        }
    };
    Ok(Some(csv::Row {
        headers: vec![],
        rest: None,
        original: None,
        data: json!({"txid":format!("binance:{reference}"),"occurred_at":occurred,"direction":"inbound","asset":asset,"amount":repr(&abs(amount)?),"fee":"0","kind":kind,"description":info.map_or_else(||"Binance income".into(),|s|format!("Binance income - {s}")),"counterparty":"Binance","raw_json":config::stored_json(&c.json()),"_synthetic_csv_id":synthetic_id}),
    }))
}
