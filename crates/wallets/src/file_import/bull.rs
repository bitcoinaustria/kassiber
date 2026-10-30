//! Bull Bitcoin order exports are commercial evidence, never observer authority.
use super::exchange::Columns;
use super::river::{abs, arithmetic, number, repr};
use super::*;

const REQUIRED: &[&str] = &[
    "ORDER_STATUS",
    "PAYIN_AMOUNT",
    "PAYIN_CURRENCY",
    "PAYOUT_AMOUNT",
    "PAYOUT_CURRENCY",
    "COMPLETED_AT (UTC)",
    "TRANSACTION_ID",
];
pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Bull Bitcoin")?;
    rows.into_iter()
        .filter_map(|row| record(row).transpose())
        .collect()
}
fn record(row: csv::Row) -> Result<Option<csv::Row>, AppError> {
    let columns = Columns::new(&row);
    for key in ["ORDER_STATUS", "PAYIN_STATUS", "PAYOUT_STATUS"] {
        if columns
            .opt(&[key])
            .is_some_and(|v| river::column(&v) != "completed")
        {
            return Ok(None);
        }
    }
    let payin_currency = columns
        .opt(&["PAYIN_CURRENCY"])
        .map(|s| kassiber_pytext::upper(&s));
    let payout_currency = columns
        .opt(&["PAYOUT_CURRENCY"])
        .map(|s| kassiber_pytext::upper(&s));
    let payin = number(columns.get(&["PAYIN_AMOUNT"]))?;
    let payout = number(columns.get(&["PAYOUT_AMOUNT"]))?;
    let (Some(payin), Some(payout)) = (payin, payout) else {
        return Err(AppError::new(
            "app_error",
            "Bull Bitcoin CSV has a completed row with an empty amount",
        ));
    };
    let (direction, asset, amount, value, fiat, kind) = match (
        exchange::bitcoin(payin_currency.as_deref()),
        exchange::bitcoin(payout_currency.as_deref()),
    ) {
        (Some(asset), None) => (
            "outbound",
            asset,
            abs(payin)?,
            abs(payout)?,
            payout_currency,
            "sell",
        ),
        (None, Some(asset)) => (
            "inbound",
            asset,
            abs(payout)?,
            abs(payin)?,
            payin_currency,
            "buy",
        ),
        _ => return Ok(None),
    };
    let Some(txid) = columns.opt(&["TRANSACTION_ID"]) else {
        return Ok(None);
    };
    let rate = match number(columns.get(&["EXCHANGE_RATE_AMOUNT"]))? {
        Some(rate) => rate,
        None => arithmetic(&value, &amount, '/')?,
    };
    let reference = columns
        .opt(&["ORDER_ID", "ORDER_NUMBER"])
        .unwrap_or_else(|| txid.clone());
    let occurred = columns.get(&["COMPLETED_AT (UTC)", "SENT_AT (UTC)", "CREATED_AT (UTC)"]);
    let methods = [
        columns.opt(&["PAYIN_METHOD"]),
        columns.opt(&["PAYOUT_METHOD"]),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" to ");
    let description = if methods.is_empty() {
        format!("Bull Bitcoin {kind}")
    } else {
        format!("Bull Bitcoin {kind} - {methods}")
    };
    Ok(Some(csv::Row {
        headers: vec![],
        rest: None,
        original: None,
        data: json!({"txid":txid,"occurred_at":occurred,"direction":direction,"asset":asset,"amount":repr(&amount),"fee":"0","fiat_rate":repr(&rate),"fiat_value":repr(&value),"fiat_currency":fiat,"pricing_source_kind":"exchange_execution","pricing_provider":"Bull Bitcoin","pricing_pair":fiat.as_ref().map(|f|format!("{asset}-{f}")),"pricing_method":"bullbitcoin_csv","pricing_external_ref":reference,"pricing_quality":"exact","kind":kind,"description":description,"counterparty":"Bull Bitcoin","raw_json":config::stored_json(&columns.json())}),
    }))
}
