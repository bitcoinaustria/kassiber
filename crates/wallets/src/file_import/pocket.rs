//! Pocket exchange evidence joins each gross purchase to one exported withdrawal.
use super::exchange::Columns;
use super::river::{abs, arithmetic, nonnegative, number, repr, truth, zero};
use super::*;
const REQUIRED: &[&str] = &[
    "type",
    "date",
    "reference",
    "price.currency",
    "price.amount",
    "cost.currency",
    "cost.amount",
    "fee.currency",
    "fee.amount",
    "value.currency",
    "value.amount",
];
fn curr(c: &Columns, key: &str) -> Option<String> {
    c.opt(&[key]).map(|s| asset(&s))
}
fn bitcoin(c: &Columns, key: &str) -> Option<&'static str> {
    let v = curr(c, key);
    exchange::bitcoin(v.as_deref())
}
fn row_type(c: &Columns) -> String {
    kassiber_pytext::casefold(kassiber_pytext::strip(
        &c.get(&["type"]).unwrap_or_default(),
    ))
}
fn equal(a: &PriceNumber, b: &PriceNumber) -> bool {
    match (a, b) {
        (PriceNumber::Finite(a), PriceNumber::Finite(b)) => a == b,
        (PriceNumber::Special(a), PriceNumber::Special(b)) => {
            !a.contains("NaN") && !b.contains("NaN") && a == b
        }
        _ => false,
    }
}
struct Withdrawal {
    asset: &'static str,
    gross: PriceNumber,
    date: String,
    index: usize,
}
pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Pocket Bitcoin")?;
    let mut withdrawals = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let c = Columns::unstripped(row);
        if row_type(&c) != "withdrawal" {
            continue;
        }
        let currency = curr(&c, "value.currency");
        let asset = exchange::bitcoin(currency.as_deref());
        let amount = number(c.get(&["value.amount"]))?;
        let fee = number(c.get(&["fee.amount"]))?;
        let fee_currency = curr(&c, "fee.currency");
        if let (Some(asset), Some(amount), Some(fee)) = (asset, amount, fee) {
            if fee_currency == currency {
                withdrawals.push(Withdrawal {
                    asset,
                    gross: arithmetic(&abs(amount)?, &abs(fee)?, '+')?,
                    date: c.get(&["date"]).unwrap_or_default(),
                    index,
                });
            }
        }
    }
    withdrawals.sort_by(|a, b| a.date.cmp(&b.date));
    let mut pairs = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let c = Columns::unstripped(row);
        if row_type(&c) != "exchange" {
            continue;
        }
        let asset = bitcoin(&c, "value.currency");
        let gross = number(c.get(&["value.amount"]))?;
        let (Some(asset), Some(gross)) = (asset, gross) else {
            continue;
        };
        let gross = abs(gross)?;
        let candidates = withdrawals
            .iter()
            .enumerate()
            .filter(|(_, w)| w.asset == asset && equal(&w.gross, &gross))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            continue;
        }
        let date = c.get(&["date"]).unwrap_or_default();
        let selected = candidates
            .iter()
            .find(|(_, w)| date.is_empty() || w.date.is_empty() || w.date >= date)
            .unwrap_or(&candidates[0])
            .0;
        let withdrawal = withdrawals.remove(selected);
        pairs.insert(index, withdrawal.index);
    }
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            record(row, pairs.get(&index).map(|i| &rows[*i]), index + 1).transpose()
        })
        .collect::<Result<_, _>>()
        .map(synthetic::number_occurrences)
}
pub(super) fn record(
    row: &csv::Row,
    withdrawal: Option<&csv::Row>,
    index: usize,
) -> Result<Option<csv::Row>, AppError> {
    let c = Columns::new(row);
    if row_type(&c) != "exchange" {
        return Ok(None);
    }
    let cost_currency = curr(&c, "cost.currency");
    let value_currency = curr(&c, "value.currency");
    let fee_currency = curr(&c, "fee.currency");
    let cost = number(c.get(&["cost.amount"]))?;
    let value = number(c.get(&["value.amount"]))?;
    let fee = number(c.get(&["fee.amount"]))?;
    let (Some(cost), Some(value)) = (cost, value) else {
        return Err(AppError::new(
            "app_error",
            "Pocket Bitcoin CSV has an exchange row with an empty amount",
        ));
    };
    let (direction, asset, mut amount, mut fiat_value, fiat, kind) = match (
        exchange::bitcoin(cost_currency.as_deref()),
        exchange::bitcoin(value_currency.as_deref()),
    ) {
        (None, Some(asset)) => (
            "inbound",
            asset,
            abs(value)?,
            abs(cost)?,
            cost_currency,
            "buy",
        ),
        (Some(asset), None) => (
            "outbound",
            asset,
            abs(cost)?,
            abs(value)?,
            value_currency,
            "sell",
        ),
        _ => return Ok(None),
    };
    if fee_currency == fiat {
        if let Some(fee) = fee {
            fiat_value = if direction == "inbound" {
                arithmetic(&fiat_value, &abs(fee)?, '+')?
            } else {
                nonnegative(arithmetic(&fiat_value, &abs(fee)?, '-')?)?
            };
        }
    }
    let mut btc_fee = zero();
    let mut occurred = c.get(&["date"]);
    let mut raw = c.json();
    if let Some(withdrawal) = withdrawal {
        let w = Columns::unstripped(withdrawal);
        let currency = curr(&w, "value.currency");
        let withdrawal_asset = exchange::bitcoin(currency.as_deref());
        let withdrawal_amount = number(w.get(&["value.amount"]))?;
        let withdrawal_fee = number(w.get(&["fee.amount"]))?;
        let fee_currency = curr(&w, "fee.currency");
        if direction == "inbound" && withdrawal_asset == Some(asset) {
            if let Some(value) = withdrawal_amount {
                amount = abs(value)?;
                occurred = w.get(&["date"]).or(occurred);
                if fee_currency == currency {
                    if let Some(fee) = withdrawal_fee {
                        btc_fee = abs(fee)?
                    }
                }
            }
        }
        raw["_pocketbitcoin_withdrawal"] = Columns::new(withdrawal).json();
    }
    let rate = match number(c.get(&["price.amount"]))? {
        Some(v) => Some(v),
        None if truth(&amount) => Some(arithmetic(&fiat_value, &amount, '/')?),
        None => None,
    };
    let _ = index;
    let provided = c.opt(&["reference"]);
    let synthetic_id = provided.is_none();
    let reference = match provided {
        Some(reference) => reference,
        None => {
            use synthetic::Part::{Number, Text};
            synthetic::economic_id(
                "pocketbitcoin",
                &[
                    Text(occurred.as_deref()),
                    Text(Some(direction)),
                    Text(Some(asset)),
                    Number(&amount),
                    Number(&btc_fee),
                    Text(fiat.as_deref()),
                    Number(&fiat_value),
                ],
            )?
        }
    };
    Ok(Some(csv::Row {
        headers: vec![],
        rest: None,
        original: None,
        data: json!({"txid":format!("pocketbitcoin:{reference}"),"occurred_at":occurred,"direction":direction,"asset":asset,"amount":repr(&amount),"fee":repr(&btc_fee),"fiat_rate":rate.as_ref().map(repr),"fiat_value":repr(&fiat_value),"fiat_currency":fiat,"pricing_source_kind":"exchange_execution","pricing_provider":"Pocket Bitcoin","pricing_pair":fiat.as_ref().map(|f|format!("{asset}-{f}")),"pricing_timestamp":c.get(&["date"]),"pricing_method":"pocketbitcoin_csv","pricing_external_ref":reference,"pricing_quality":"exact","kind":kind,"description":format!("Pocket Bitcoin {kind}"),"counterparty":"Pocket Bitcoin","_exchange_evidence_match_by_economics":true,"_synthetic_csv_id":synthetic_id,"_exchange_evidence_match_time_tolerance_seconds":172800,"raw_json":config::stored_json(&raw)}),
    }))
}
