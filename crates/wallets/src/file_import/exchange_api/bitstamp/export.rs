//! Bitstamp's transaction history CSV export in its "RFC 4180" format, an
//! active custodial ledger. Bitstamp does not publish the format; its
//! columns come from third-party parsers, so it is read strictly
//! (docs/reference/bitstamp.md). Each row becomes the API sync's event and
//! maps through the same rules, so an exported transaction and its API row
//! share their id (`bitstamp:<ID>`) and record.
use super::*;
use super::{self as api, Event, Leg};
use exchange::Columns;

const COLUMNS: [&str; 14] = [
    "ID",
    "Account",
    "Type",
    "Subtype",
    "Datetime",
    "Amount",
    "Amount currency",
    "Value",
    "Value currency",
    "Rate",
    "Rate currency",
    "Fee",
    "Fee currency",
    "Order ID",
];
/// The legacy export: no transaction id and minute-precision times.
const LEGACY: [&str; 8] = [
    "Type", "Datetime", "Account", "Amount", "Value", "Rate", "Fee", "Sub Type",
];
const EXPORT: &str =
    "Export the transaction history from Bitstamp in the RFC 4180 format and import it unchanged.";

/// The export's transactions as events, mapped by the API sync's rules.
pub(super) fn load(file: &str, mapper: &mut Mapper) -> Result<(), AppError> {
    let rows = csv::rows(&csv::read_sig(file)?)?;
    let Some(first) = rows.first() else {
        return Ok(());
    };
    let names: Vec<&str> = first
        .headers
        .iter()
        .map(|h| kassiber_pytext::strip(h))
        .collect();
    if !COLUMNS.iter().all(|c| names.contains(c)) && LEGACY.iter().all(|c| names.contains(c)) {
        return Err(AppError::new(
            "validation",
            "This is Bitstamp's legacy transaction export, which has no transaction IDs",
        )
        .with_hint("In Bitstamp, open Transaction history, export all transactions in the RFC 4180 format, and import that file."));
    }
    let missing: Vec<&str> = COLUMNS.into_iter().filter(|c| !names.contains(c)).collect();
    if !missing.is_empty() {
        let message = format!(
            "Bitstamp CSV is missing required columns: {}",
            missing.join(", ")
        );
        return Err(AppError::new("app_error", message).with_hint(EXPORT));
    }
    let scope = mapper.scope();
    let mut accounts = std::collections::BTreeSet::new();
    let mut seen: BTreeMap<String, Json> = BTreeMap::new();
    let mut events = vec![];
    // Line 1 is the header.
    for (line, row) in (2..).zip(&rows) {
        // A row cut short leaves its last cells missing, not empty: an absent
        // fee is no confirmed zero fee.
        if row.headers.iter().any(|header| row.data[header].is_null()) {
            return Err(AppError::new(
                "validation",
                format!("Bitstamp CSV line {line} has fewer cells than the header"),
            )
            .with_hint(
                "Export the file from Bitstamp again; Kassiber does not guess a missing cell.",
            ));
        }
        let columns = Columns::new(row);
        accounts.extend(columns.opt(&["Account"]));
        let event = event(line, &columns, scope)?;
        let raw = columns.json();
        match seen.get(&event.id) {
            Some(previous) if *previous == raw => continue,
            Some(_) => {
                return Err(AppError::new(
                    "validation",
                    format!(
                        "Bitstamp CSV line {line} repeats transaction ID {} with different values",
                        event.id
                    ),
                ))
            }
            None => seen.insert(event.id.clone(), raw),
        };
        events.push(event);
    }
    if accounts.len() > 1 {
        return Err(AppError::new("validation", "Bitstamp CSV holds several accounts")
            .with_hint("Export each Bitstamp account (main or sub account) separately and import it into its own wallet."));
    }
    // File order may be newest first; each record's source position decides
    // rows that share a second.
    api::chronological(&mut events);
    api::map(&events, mapper)
}

/// The export's type names, as the API's numeric types.
fn code(kind: &str) -> Option<u32> {
    Some(match kind.to_ascii_lowercase().as_str() {
        "deposit" => 0,
        "withdrawal" => 1,
        "market" => 2,
        "sub account transfer" => 14,
        "staking reward" => 27,
        "referral reward" => 32,
        "inter account transfer" => 35,
        _ => return None,
    })
}

/// One export row as the API's event. Amounts are unsigned; the type (and a
/// market row's Buy or Sell subtype) gives the direction.
fn event(line: usize, columns: &Columns, scope: assets::Scope) -> Result<Event, AppError> {
    let bad = |what: String| {
        AppError::new("validation", format!("Bitstamp CSV line {line} {what}")).with_hint(EXPORT)
    };
    let text = |name: &str| columns.opt(&[name]);
    let id = text("ID")
        .and_then(|id| api::row_id(&s(id)))
        .ok_or_else(|| bad("has no transaction ID".into()))?;
    let label = text("Type").ok_or_else(|| bad("has no type".into()))?;
    let occurred_us = text("Datetime")
        .and_then(|t| api::datetime(&t, 'T'))
        .ok_or_else(|| bad("has a date and time Kassiber cannot read".into()))?;
    let unsigned = |name: &str| -> Result<Option<Decimal>, AppError> {
        match text(name).map(|value| api::amount(&value).filter(|a| !a.is_negative())) {
            None => Ok(None),
            Some(Some(amount)) => Ok(Some(amount)),
            Some(None) => Err(bad(format!(
                "has a {name} that is not a plain unsigned decimal"
            ))),
        }
    };
    let currency = |name: &str| text(name).map(|c| asset_code(&c));
    let code = code(&label);
    // A type Kassiber does not know may carry a sign (a reversal); the
    // mapping refuses it when it touches BTC.
    let amount = match code {
        Some(_) => unsigned("Amount")?,
        None => text("Amount")
            .map(|v| {
                api::amount(&v)
                    .ok_or_else(|| bad("has an Amount that is not a plain decimal".into()))
            })
            .transpose()?,
    }
    .ok_or_else(|| bad("has no amount".into()))?;
    let amount_currency =
        currency("Amount currency").ok_or_else(|| bad("has no amount currency".into()))?;
    let (value, value_currency) = (unsigned("Value")?, currency("Value currency"));
    let leg = |currency: &str, amount: Decimal| Leg {
        currency: currency.to_owned(),
        amount,
    };
    // Whether the row moves an asset the scope keeps.
    let kept = |c: &str| {
        c == "BTC" || (scope == assets::Scope::Crypto && assets::class(c) != assets::Class::Fiat)
    };
    let touches = kept(&amount_currency) || value_currency.as_deref().is_some_and(kept);
    let mut legs = match code {
        Some(1) => vec![leg(&amount_currency, amount.copy_negate())],
        Some(2) => {
            let (Some(value), Some(value_currency)) = (value.clone(), value_currency.as_deref()) else {
                return Err(bad("is a market trade without a value".into()));
            };
            match text("Subtype").map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("buy") => vec![leg(&amount_currency, amount), leg(value_currency, value.copy_negate())],
                Some("sell") => vec![leg(&amount_currency, amount.copy_negate()), leg(value_currency, value)],
                _ if touches => return Err(bad("is a market trade that is neither Buy nor Sell".into())),
                _ => vec![leg(&amount_currency, amount), leg(value_currency, value)],
            }
        }
        // The export does not sign a transfer between Bitstamp accounts.
        Some(14 | 35) if touches => {
            return Err(bad(format!("({label}) does not say which way its {amount_currency} moved")).with_hint(
                "Sync this Bitstamp account through its read-only API instead; the API states each transfer's direction.",
            ))
        }
        // Credits, and types Kassiber does not know (refused by the mapping
        // when they touch BTC).
        _ => vec![leg(&amount_currency, amount)],
    };
    // Outside a market trade a value is a second leg, so BTC in any column
    // reaches the mapping, which refuses a movement of two currencies.
    if let Some(value) = value.filter(|v| code != Some(2) && !v.is_zero()) {
        let value_currency =
            value_currency.ok_or_else(|| bad("has a value without a currency".into()))?;
        legs.push(leg(&value_currency, value));
    }
    legs.retain(|l| !l.amount.is_zero());
    let rates = match (unsigned("Rate")?, currency("Rate currency")) {
        (Some(rate), Some(quote)) => vec![(amount_currency.clone(), quote, rate)],
        _ => vec![],
    };
    let fee = unsigned("Fee")?;
    let fee_currency = currency("Fee currency");
    if fee.as_ref().is_some_and(|f| !f.is_zero()) && fee_currency.is_none() {
        return Err(bad("has a fee without a currency".into()));
    }
    let raw = kassiber_pytext::loads(&columns.json().to_string())
        .map_err(|_| AppError::new("internal_error", "Bitstamp CSV row"))?;
    Ok(Event {
        id,
        code,
        label,
        occurred_us,
        legs,
        rates,
        // The export writes no fee as an empty column.
        fee: Some(fee.unwrap_or_else(Decimal::zero)),
        fee_currency,
        raw,
    })
}
