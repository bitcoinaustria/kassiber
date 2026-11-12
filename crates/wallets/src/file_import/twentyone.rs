//! 21bitcoin is an active custodial ledger. Provider row ids are not chain ids.
use super::exchange::Columns;
use super::river::{abs, arithmetic, number, positive, repr, truth, zero};
use super::*;
const REQUIRED: &[&str] = &[
    "id",
    "transaction_date",
    "buy_asset",
    "buy_amount",
    "sell_asset",
    "sell_amount",
    "fee_asset",
    "fee_amount",
    "transaction_type",
];

pub(super) fn parse(mut rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    rows.retain(|r| {
        r.rest.is_some()
            || r.data.as_object().unwrap().values().any(|v| {
                v.as_str()
                    .is_some_and(|s| !kassiber_pytext::strip(s).is_empty())
            })
    });
    exchange::headers(&rows, REQUIRED, "21bitcoin")?;
    rows.into_iter()
        .filter_map(|r| record(r).transpose())
        .collect()
}
fn date(value: Option<String>) -> Result<Option<String>, AppError> {
    let Some(value) = value
        .map(|v| kassiber_pytext::strip(&v).to_owned())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    let (day, clock) = value.split_once(' ').unwrap_or((&value, ""));
    let bits = day.split('.').collect::<Vec<_>>();
    let clock = kassiber_pytext::strip(clock);
    let clock = if clock.is_empty() { "00:00:00" } else { clock };
    let time = clock.split(':').collect::<Vec<_>>();
    if bits.len() != 3 || time.len() != 3 {
        return Ok(Some(value));
    }
    let year = kassiber_pytext::strip(bits[2]);
    let short = year.chars().count() == 2 || year.chars().count() == 3 && year.starts_with('2');
    let text = if year.chars().count() == 3 && year.starts_with('2') {
        year.chars().skip(1).collect::<String>()
    } else {
        year.into()
    };
    let Ok(mut year_number) = kassiber_pytext::parse_int(&text) else {
        return Ok(Some(value));
    };
    if short {
        year_number += 2000;
    }
    let year = year_number.to_string();
    let values = [
        year.as_str(),
        bits[1],
        bits[0],
        time[0],
        time[1],
        time[2].split('.').next().unwrap(),
    ];
    let Ok(numbers) = values
        .into_iter()
        .map(kassiber_pytext::parse_int)
        .collect::<Result<Vec<_>, _>>()
    else {
        return Ok(Some(value));
    };
    let mut values = Vec::new();
    for number in numbers {
        if number > i64::MAX.into() || number < i64::MIN.into() {
            return Err(AppError::new(
                "internal_error",
                "Python int too large to convert to C long",
            ));
        }
        if number > i32::MAX.into() {
            return Err(AppError::new(
                "internal_error",
                "signed integer is greater than maximum",
            ));
        }
        if number < i32::MIN.into() {
            return Err(AppError::new(
                "internal_error",
                "signed integer is less than minimum",
            ));
        }
        values.push(number.to_string().parse::<i32>().expect("bounded integer"));
    }
    let iso = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        values[0], values[1], values[2], values[3], values[4], values[5]
    );
    Ok(Some(crate::time::timestamp(&iso).unwrap_or(value)))
}
fn kind(tag: &str, direction: &str) -> String {
    let tag = kassiber_pytext::lower(kassiber_pytext::strip(tag));
    match tag.as_str() {
        "deposit" => "deposit".into(),
        "withdrawal" => "withdrawal".into(),
        "trade" => if direction == "inbound" {
            "buy"
        } else {
            "sell"
        }
        .into(),
        "" => direction.into(),
        _ => tag.replace(' ', "_"),
    }
}
fn record(row: csv::Row) -> Result<Option<csv::Row>, AppError> {
    let c = Columns::new(&row);
    let id = c.opt(&["id"]).ok_or_else(|| {
        AppError::new(
            "app_error",
            "21bitcoin CSV has a BTC-side row without an id",
        )
    })?;
    let currency = |key| {
        c.opt(&[key]).map(|s| {
            let value = asset(&s);
            exchange::bitcoin(Some(&value)).map_or(value.clone(), str::to_owned)
        })
    };
    let buy = currency("buy_asset");
    let sell = currency("sell_asset");
    let fee_currency = currency("fee_asset");
    let buy_amount = number(c.get(&["buy_amount"]))?;
    let sell_amount = number(c.get(&["sell_amount"]))?;
    let fee = abs(number(c.get(&["fee_amount"]))?
        .filter(truth)
        .unwrap_or_else(zero))?;
    let crypto = |v: &Option<String>| v.as_deref().is_some_and(|s| ["BTC", "LBTC"].contains(&s));
    let fiat_fee = fee_currency.is_some() && !crypto(&fee_currency);
    let tag = c
        .opt(&["transaction_type"])
        .unwrap_or_else(|| "transaction".into());
    let (direction, asset, amount, fiat, value, kind) = match (crypto(&buy), crypto(&sell)) {
        (true, false) => {
            let amount = abs(buy_amount.ok_or_else(|| {
                AppError::new(
                    "app_error",
                    "21bitcoin CSV has a BTC buy row with an empty buy_amount",
                )
            })?)?;
            let value = if sell.is_some() {
                sell_amount
                    .map(|n| {
                        arithmetic(&abs(n)?, &if fiat_fee { fee.clone() } else { zero() }, '+')
                    })
                    .transpose()?
            } else {
                None
            };
            let kind = if sell.is_some() {
                "buy".into()
            } else {
                kind(&tag, "inbound")
            };
            (
                "inbound",
                buy.clone().unwrap(),
                amount,
                if value.is_some() { sell.clone() } else { None },
                value,
                kind,
            )
        }
        (false, true) => {
            let amount = abs(sell_amount.ok_or_else(|| {
                AppError::new(
                    "app_error",
                    "21bitcoin CSV has a BTC sell/withdrawal row with an empty sell_amount",
                )
            })?)?;
            let value = if buy.is_some() {
                buy_amount.map(abs).transpose()?
            } else {
                None
            };
            let kind = if value.is_some() {
                "sell".into()
            } else {
                kind(&tag, "outbound")
            };
            (
                "outbound",
                sell.clone().unwrap(),
                amount,
                if value.is_some() { buy.clone() } else { None },
                value,
                kind,
            )
        }
        _ => return Ok(None),
    };
    let btc_fee = if fee_currency.as_deref() == Some(&asset) {
        fee
    } else {
        zero()
    };
    let rate = match value.as_ref() {
        Some(value) if positive(&amount)? => Some(arithmetic(value, &amount, '/')?),
        _ => None,
    };
    let source = rate.as_ref().map(|_| "exchange_execution");
    let description = [Some(tag), c.opt(&["note"]), c.opt(&["depot_name"])]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" - ");
    Ok(Some(csv::Row {
        headers: vec![],
        rest: None,
        original: None,
        data: json!({"txid":format!("21bitcoin:{id}"),"occurred_at":date(c.get(&["transaction_date"]))?,"direction":direction,"asset":asset,"amount":repr(&amount),"fee":repr(&btc_fee),"fiat_rate":rate.as_ref().map(repr),"fiat_value":value.as_ref().map(repr),"fiat_currency":fiat,"pricing_source_kind":source,"pricing_provider":"21bitcoin","pricing_pair":fiat.as_ref().map(|f|format!("{asset}-{f}")),"pricing_method":source.map(|_|"21bitcoin_csv"),"pricing_external_ref":id,"pricing_quality":source.map(|_|"exact"),"kind":kind,"description":description,"counterparty":c.opt(&["exchange_name"]).unwrap_or_else(||"21bitcoin".into()),"raw_json":config::stored_json(&c.json())}),
    }))
}
