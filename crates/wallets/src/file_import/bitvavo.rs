//! Bitvavo's transaction-history CSV export (Transaction history → Export).
//! An active custodial ledger: each row is read into the transaction shape
//! the API sync reads and mapped by the same rules, so a transaction in both
//! gets one record (`bitvavo:<Transaction ID>`). The layout is pinned by
//! public sample exports, so the parser is strict: exact headers, known
//! statuses, Europe/Amsterdam or UTC wall times, no guessed amounts.
use super::exchange_api::bitvavo::{self as shared, Event, Leg};
use super::exchange_api::{asset_code, malformed, strict_decimal};
use super::*;
use kassiber_pytext::zone::Zone;

const PROVIDER: &str = "Bitvavo";
pub(super) const HEADERS: [&str; 14] = [
    "Timezone",
    "Date",
    "Time",
    "Type",
    "Currency",
    "Amount",
    "Quote Currency",
    "Quote Price",
    "Received / Paid Currency",
    "Received / Paid Amount",
    "Fee currency",
    "Fee amount",
    "Status",
    "Transaction ID",
];

/// The export's rows under the wallet's asset scope (`None`: the `bitcoin`
/// scope, naming no wallet), and the rows the scope left out.
pub(super) fn parse(
    rows: Vec<csv::Row>,
    target: Option<tax_platform::Target<'_>>,
) -> Result<(Vec<csv::Row>, assets::Tally), AppError> {
    let Some(first) = rows.first() else {
        return Ok((vec![], assets::Tally::default()));
    };
    let present: Vec<&str> = first
        .headers
        .iter()
        .map(|h| kassiber_pytext::strip(h))
        .collect();
    let missing: Vec<&str> = HEADERS
        .into_iter()
        .filter(|h| !present.contains(h))
        .collect();
    if !missing.is_empty() {
        return Err(AppError::new(
            "validation",
            format!("Bitvavo CSV is missing required columns: {}", missing.join(", ")),
        )
        .with_hint("Export the transaction history from Bitvavo (Transaction history → Export) and import that file unchanged."));
    }
    let mut events = vec![];
    for row in &rows {
        let columns = exchange::Columns::new(row);
        let blank = |v: &serde_json::Value| {
            v.as_str()
                .is_none_or(|t| kassiber_pytext::strip(t).is_empty())
        };
        if columns
            .json()
            .as_object()
            .is_some_and(|o| o.values().all(blank))
        {
            continue;
        }
        if row.rest.is_some() {
            return Err(AppError::new(
                "validation",
                "Bitvavo CSV has a row with more fields than headers",
            ));
        }
        events.extend(event(&columns)?);
    }
    let mapped = shared::map(events, "bitvavo_csv", shared::FeeReading::AfterFee, target)?;
    let rows = mapped
        .records
        .into_iter()
        .map(|record| csv::Row {
            headers: vec![],
            data: generic_json::projection(&record),
            rest: None,
            original: Some(record),
        })
        .collect();
    Ok((rows, mapped.tally))
}

/// One CSV row as a shared transaction, `None` for a row that has not (or
/// will never have) happened.
fn event(columns: &exchange::Columns) -> Result<Option<Event>, AppError> {
    let text = |name: &str| columns.opt(&[name]).unwrap_or_default();
    let id = text("Transaction ID");
    if id.is_empty() {
        return Err(AppError::new(
            "validation",
            "Bitvavo CSV has a row without a Transaction ID",
        ));
    }
    let bad = |what: &str| malformed(PROVIDER, &id, what);
    let status = text("Status");
    match kassiber_pytext::lower(&status).as_str() {
        "completed" | "distributed" => {}
        "pending" | "canceled" | "cancelled" => return Ok(None),
        _ => {
            let status: String = status.chars().take(40).collect();
            return Err(bad(&format!(
                "status '{status}', which Kassiber does not know"
            )));
        }
    }
    let kind = kassiber_pytext::lower(&text("Type"));
    let at = time(&text("Timezone"), &text("Date"), &text("Time")).map_err(|what| bad(&what))?;
    // A currency and a signed amount, both present or both blank.
    let leg = |currency: &str, amount: &str| -> Result<Option<(String, Decimal)>, AppError> {
        match (text(currency), text(amount)) {
            (code, value) if code.is_empty() && value.is_empty() => Ok(None),
            (code, value) if !code.is_empty() && !value.is_empty() => {
                let number = strict_decimal(&value, true)
                    .ok_or_else(|| bad(&format!("a {amount} that is not a plain decimal")))?;
                Ok(Some((asset_code(&code), number)))
            }
            _ => Err(bad(&format!(
                "{currency} and {amount} that do not come together"
            ))),
        }
    };
    let main = leg("Currency", "Amount")?.ok_or_else(|| bad("no Currency or Amount"))?;
    let counter = leg("Received / Paid Currency", "Received / Paid Amount")?;
    let fee = leg("Fee currency", "Fee amount")?;
    let price = leg("Quote Currency", "Quote Price")?;
    if [&fee, &price]
        .into_iter()
        .flatten()
        .any(|(_, v)| v.is_negative())
    {
        return Err(bad("a negative fee or price"));
    }
    let as_leg = |(asset, amount): (String, Decimal)| Leg {
        asset,
        amount: amount.copy_abs(),
    };
    // The signed Amount says which way the row's Currency moved; the counter
    // amount, when there is one, moves the other way.
    let outgoing = main.1.is_negative();
    let (sent, received) = match counter {
        Some(counter) if counter.1.is_negative() == outgoing && !counter.1.is_zero() => {
            return Err(bad(
                "an Amount and a Received / Paid Amount with the same sign",
            ));
        }
        counter if outgoing => (Some(as_leg(main)), counter.map(as_leg)),
        counter => (counter.map(as_leg), Some(as_leg(main))),
    };
    Ok(Some(Event {
        id: id.as_str().into(),
        kind: s(kind.as_str()),
        at,
        sent,
        received,
        fee: fee.map(as_leg),
        price: price.map(as_leg),
        // The export's withdrawal Amount is everything that left, fee included.
        fee_included: kind == "withdrawal",
        raw: kassiber_pytext::loads(&columns.json().to_string())
            .map_err(|e| AppError::new("internal_error", e.to_string()))?,
    }))
}

/// `Date` and `Time` (`YYYY-MM-DD`, `HH:MM:SS[.fff]`), wall time in `zone`,
/// as Unix milliseconds. A wall time the clock change skips or repeats has
/// no single instant and is refused.
fn time(zone: &str, date: &str, clock: &str) -> Result<i64, String> {
    use kassiber_pytext::zone::{civil_from_days, days_from_civil};
    let zone = match zone {
        "Europe/Amsterdam" => Zone::eu("Europe/Amsterdam", 3600),
        "UTC" | "Etc/UTC" => Zone::utc("UTC"),
        _ => {
            let zone: String = zone.chars().take(40).collect();
            return Err(format!(
                "time zone '{zone}'; Kassiber reads Bitvavo exports in Europe/Amsterdam or UTC"
            ));
        }
    };
    let bad = || "a Date or Time Kassiber cannot read".to_owned();
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let fields = |text: &str, separator: char, widths: [usize; 3]| -> Option<[i64; 3]> {
        let parts: Vec<&str> = text.split(separator).collect();
        let ok = parts.len() == 3
            && parts
                .iter()
                .zip(widths)
                .all(|(p, w)| p.len() == w && digits(p));
        ok.then(|| [0, 1, 2].map(|i| parts[i].parse().unwrap_or(-1)))
    };
    let (whole, fraction) = clock.split_once('.').unwrap_or((clock, "0"));
    let ([year, month, day], [hour, minute, second]) = (
        fields(date, '-', [4, 2, 2]).ok_or_else(bad)?,
        fields(whole, ':', [2, 2, 2]).ok_or_else(bad)?,
    );
    let days = days_from_civil(year, month, day);
    // Bitvavo launched in 2018; the EU summer-time rule read here holds since 1996.
    if !digits(fraction)
        || year < 1996
        || civil_from_days(days) != (year, month, day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(bad());
    }
    // The fraction to whole milliseconds (`.5` is 500, `.473` 473).
    let millis: i64 = format!("{:0<3}", &fraction[..fraction.len().min(3)])
        .parse()
        .map_err(|_| bad())?;
    let local = days * 86_400 + hour * 3600 + minute * 60 + second;
    match (
        zone.local_offset_fold(local, false),
        zone.local_offset_fold(local, true),
    ) {
        (Some(a), Some(b)) if a == b => Ok((local - a) * 1_000 + millis),
        _ => Err("a wall time the summer-time change skips or repeats".into()),
    }
}
