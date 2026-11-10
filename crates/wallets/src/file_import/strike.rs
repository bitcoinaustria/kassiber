//! Strike active custodial history retains Lightning identity separately from txids.
use super::exchange::Columns;
use super::river::{abs, arithmetic, nonnegative, number, positive, repr, truth, zero};
use super::*;
const REQUIRED: &[&str] = &[
    "Reference",
    "Date & Time (UTC)",
    "Transaction Type",
    "Amount BTC",
];
fn decimal(value: Option<String>) -> Result<Option<PriceNumber>, AppError> {
    if value
        .as_ref()
        .is_some_and(|s| ["-", "–", "—"].contains(&kassiber_pytext::strip(s)))
    {
        return Ok(None);
    }
    number(value)
}
fn date(value: Option<String>) -> Option<String> {
    let value = value
        .map(|s| kassiber_pytext::strip(&s).to_owned())
        .filter(|s| !s.is_empty())?;
    let bits = value
        .split(|c| kassiber_pytext::is_space(u32::from(c)))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if bits.len() != 4 {
        return Some(value);
    }
    // strptime's month lookup rejects the non-ASCII lookalikes its regex may match.
    let month = bits[0]
        .chars()
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();
    let month = match month.as_str() {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return Some(value),
    };
    let clock = bits[3].split(':').collect::<Vec<_>>();
    if bits[2].chars().count() != 4
        || !bits[2]
            .chars()
            .all(|c| kassiber_pytext::decimal_value(u32::from(c)).is_some())
        || clock.len() != 3
    {
        return Some(value);
    }
    // Python's day regex restricts ASCII digits more tightly than its clock regexes.
    let valid = |text: &str, which: usize| {
        let v = text.chars().collect::<Vec<_>>();
        if !(1..=2).contains(&v.len())
            || !v
                .iter()
                .all(|c| kassiber_pytext::decimal_value(u32::from(*c)).is_some())
        {
            return false;
        }
        if v.len() == 1 {
            return which != 0 || ('1'..='9').contains(&v[0]);
        }
        match which {
            0 => {
                ['1', '2'].contains(&v[0])
                    || v[0] == '0' && ('1'..='9').contains(&v[1])
                    || v[0] == '3' && ['0', '1'].contains(&v[1])
            }
            1 => ['0', '1'].contains(&v[0]) || v[0] == '2' && ['0', '1', '2', '3'].contains(&v[1]),
            _ => {
                ['0', '1', '2', '3', '4', '5'].contains(&v[0])
                    || which == 3 && v[0] == '6' && ['0', '1'].contains(&v[1])
            }
        }
    };
    if !valid(bits[1], 0) || !valid(clock[0], 1) || !valid(clock[1], 2) || !valid(clock[2], 3) {
        return Some(value);
    }
    let fields = [bits[2], bits[1], clock[0], clock[1], clock[2]];
    let Some(n) = fields
        .into_iter()
        .map(|s| {
            kassiber_pytext::parse_int(s)
                .ok()
                .and_then(|n| n.to_string().parse::<i64>().ok())
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Some(value);
    };
    let iso = format!(
        "{:04}-{month:02}-{:02}T{:02}:{:02}:{:02}Z",
        n[0], n[1], n[2], n[3], n[4]
    );
    Some(crate::time::timestamp(&iso).unwrap_or(value))
}
fn currency(c: &Columns) -> Option<String> {
    for key in c.keys() {
        let label = kassiber_pytext::strip(key);
        let lower = kassiber_pytext::casefold(label);
        if ["btc price", "amount btc", "fee btc"].contains(&lower.as_str()) {
            continue;
        }
        let crypto = |s: &str| exchange::bitcoin(Some(s)).is_some();
        if label.contains('(')
            && label.contains(')')
            && ["amount", "fee", "cost basis"]
                .iter()
                .any(|p| lower.starts_with(p))
        {
            let candidate = label.rsplit_once('(').unwrap().1.split(')').next().unwrap();
            let value = asset(kassiber_pytext::strip(candidate));
            if !value.is_empty() && !crypto(&value) {
                return Some(value);
            }
        }
        let parts = label
            .split(|ch| kassiber_pytext::is_space(u32::from(ch)))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if parts.len() >= 2 && ["amount", "fee"].contains(&river::column(parts[0]).as_str()) {
            let value = asset(parts.last().unwrap());
            if !value.is_empty() && !crypto(&value) {
                return Some(value);
            }
        }
    }
    None
}
fn kind(tag: &str, direction: &str) -> String {
    let tag = kassiber_pytext::casefold(kassiber_pytext::strip(tag));
    match tag.as_str() {
        "buy" | "purchase" => "buy".into(),
        "sell" => "sell".into(),
        "receive" | "received" => "receive".into(),
        "send" | "sent" => "send".into(),
        "withdraw" | "withdrawal" => "withdrawal".into(),
        "" => direction.into(),
        _ => tag.replace(' ', "_"),
    }
}
pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Strike")?;
    rows.into_iter()
        .filter_map(|r| record(r).transpose())
        .collect()
}
fn record(row: csv::Row) -> Result<Option<csv::Row>, AppError> {
    let c = Columns::new(&row);
    let reference = c.opt(&["Reference"]).ok_or_else(|| {
        AppError::new(
            "app_error",
            "Strike CSV has a BTC-side row without a Reference",
        )
    })?;
    let Some(amount_signed) = decimal(c.get(&["Amount BTC"]))? else {
        return Ok(None);
    };
    if !truth(&amount_signed) {
        return Ok(None);
    }
    let direction = if positive(&amount_signed)? {
        "inbound"
    } else {
        "outbound"
    };
    let amount = abs(amount_signed)?;
    let fee = abs(decimal(c.get(&["Fee BTC"]))?
        .filter(truth)
        .unwrap_or_else(zero))?;
    let fiat = currency(&c);
    let family = |key: &str, parentheses_first: bool| -> Result<Option<PriceNumber>, AppError> {
        let Some(fiat) = &fiat else { return Ok(None) };
        let plain = format!("{key} {fiat}");
        let parentheses = format!("{key} ({fiat})");
        let names = if parentheses_first {
            [parentheses.as_str(), plain.as_str()]
        } else {
            [plain.as_str(), parentheses.as_str()]
        };
        decimal(c.get(&names))
    };
    let amount_fiat = family("Amount", false)?;
    let fee_fiat = abs(family("Fee", false)?.filter(truth).unwrap_or_else(zero))?;
    let cost_basis = family("Cost Basis", true)?;
    let mut rate = Some(abs(decimal(c.get(&["BTC Price"]))?
        .filter(truth)
        .unwrap_or_else(zero))?);
    if rate.as_ref().is_some_and(|n| !truth(n)) {
        rate = None
    }
    let tag = c
        .opt(&["Transaction Type"])
        .unwrap_or_else(|| "transaction".into());
    let destination = c.opt(&["Destination"]);
    let description = c.opt(&["Description"]);
    let note = c.opt(&["Note"]);
    let transaction = c.opt(&["Transaction Hash"]);
    let lightning = river::column(&format!(
        "{tag} {}",
        destination.as_deref().unwrap_or_default()
    ))
    .contains("lightning")
        || destination.as_ref().is_some_and(|s| {
            ["lnbc", "lntb", "lnbcrt", "lightning:"]
                .iter()
                .any(|p| river::column(s).starts_with(p))
        });
    let payment_hash = if lightning {
        transaction.as_deref().and_then(hash)
    } else {
        None
    };
    let external = match (lightning, transaction) {
        (false, Some(hash)) => hash,
        _ => format!("strike:{reference}"),
    };
    let kind = kind(&tag, direction);
    let value = if let Some(value) = amount_fiat {
        Some(if direction == "inbound" && kind == "buy" {
            arithmetic(&abs(value)?, &fee_fiat, '+')?
        } else if direction == "outbound" && kind == "sell" {
            nonnegative(arithmetic(&abs(value)?, &fee_fiat, '-')?)?
        } else {
            abs(value)?
        })
    } else if let Some(rate) = &rate {
        Some(arithmetic(&amount, rate, '*')?)
    } else if kind == "buy" {
        cost_basis.map(abs).transpose()?
    } else {
        None
    };
    if rate.is_none() {
        if let Some(value) = value.as_ref() {
            if positive(&amount)? {
                rate = Some(arithmetic(value, &amount, '/')?)
            }
        }
    }
    let priced = rate.is_some() || value.is_some();
    let summary = [Some(tag), description, note]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" - ");
    let mut raw = c.json();
    raw["_strike_lightning"] = json!(lightning);
    let occurred = date(c.get(&["Date & Time (UTC)"]));
    Ok(Some(csv::Row {
        headers: vec![],
        rest: None,
        original: None,
        data: json!({"txid":external,"occurred_at":occurred,"direction":direction,"asset":"BTC","amount":repr(&amount),"fee":repr(&fee),"fiat_rate":rate.as_ref().map(repr),"fiat_value":value.as_ref().map(repr),"fiat_currency":fiat,"pricing_source_kind":priced.then_some("exchange_execution"),"pricing_provider":priced.then_some("Strike"),"pricing_pair":fiat.as_ref().filter(|_|priced).map(|f|format!("BTC-{f}")),"pricing_timestamp":if priced{occurred}else{None},"pricing_method":priced.then_some("strike_csv"),"pricing_external_ref":priced.then_some(reference),"pricing_quality":priced.then_some("exact"),"kind":kind,"description":summary,"counterparty":"Strike","payment_hash":payment_hash,"payment_hash_source":payment_hash.as_ref().map(|_|"importer"),"raw_json":config::stored_json(&raw)}),
    }))
}
