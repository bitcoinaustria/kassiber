//! Coinfinity's broker-side order labels describe the opposite user movement.
use super::exchange::Columns;
use super::river::{abs, arithmetic, nonnegative, number, repr, truth, zero};
use super::*;

const REQUIRED: &[&str] = &[
    "Order ID",
    "Type",
    "Date",
    "Amount EUR",
    "Amount Crypto",
    "Crypto",
    "Rate EUR",
    "Mining Fee Crypto",
    "Total Fee EUR",
    "Transaction",
];
pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Coinfinity")?;
    rows.into_iter()
        .enumerate()
        .filter_map(|(index, row)| record(row, index + 1).transpose())
        .collect::<Result<_, _>>()
        .map(synthetic::number_occurrences)
}
pub(super) fn record(row: csv::Row, index: usize) -> Result<Option<csv::Row>, AppError> {
    let columns = Columns::new(&row);
    let (direction, kind) =
        match river::column(&columns.opt(&["Type"]).unwrap_or_default()).as_str() {
            "sell" => ("inbound", "buy"),
            "buy" => ("outbound", "sell"),
            _ => return Ok(None),
        };
    let currency = columns.opt(&["Crypto"]).map(|s| asset(&s));
    let Some(asset) = currency
        .as_deref()
        .filter(|v| ["BTC", "XBT", "XXBT"].contains(v))
        .map(|_| "BTC")
    else {
        return Ok(None);
    };
    let amount = number(columns.get(&["Amount Crypto"]))?;
    let fiat = number(columns.get(&["Amount EUR"]))?;
    let fee_fiat = number(columns.get(&["Total Fee EUR"]))?
        .filter(truth)
        .unwrap_or_else(zero);
    let (Some(amount), Some(fiat)) = (amount, fiat) else {
        return Err(AppError::new(
            "app_error",
            "Coinfinity CSV has a BTC/EUR row with an empty amount",
        ));
    };
    let (value, fee) = if direction == "inbound" {
        (
            arithmetic(&abs(fiat.clone())?, &abs(fee_fiat.clone())?, '+')?,
            zero(),
        )
    } else {
        (
            nonnegative(arithmetic(
                &abs(fiat.clone())?,
                &abs(fee_fiat.clone())?,
                '-',
            )?)?,
            number(columns.get(&["Mining Fee Crypto"]))?
                .filter(truth)
                .unwrap_or_else(zero),
        )
    };
    let occurred = columns.get(&["Date"]);
    let _ = index;
    let order = columns.opt(&["Order ID"]);
    let transaction = columns.opt(&["Transaction"]);
    let invoice = columns.opt(&["LN Invoice"]);
    let synthetic_id = transaction.is_none() && invoice.is_none() && order.is_none();
    let reference = match order {
        Some(order) => order,
        None => {
            use synthetic::Part::{Number, Text};
            synthetic::economic_id(
                "coinfinity",
                &[
                    Text(occurred.as_deref()),
                    Text(Some(direction)),
                    Text(Some(asset)),
                    Number(&abs(amount.clone())?),
                    Number(&fee),
                    Number(&fiat),
                    Number(&fee_fiat),
                ],
            )?
        }
    };
    let economics = transaction.is_none() && invoice.is_none();
    let external = transaction
        .or(invoice)
        .unwrap_or_else(|| format!("coinfinity:{reference}"));
    let description = columns.opt(&["Transaction type"]).map_or_else(
        || format!("Coinfinity {kind}"),
        |t| format!("Coinfinity {kind} - {t}"),
    );
    let rate = match number(columns.get(&["Rate EUR"]))? {
        Some(rate) => Some(rate),
        None if truth(&amount) => Some(arithmetic(&value, &abs(amount.clone())?, '/')?),
        None => None,
    };
    let mut data = json!({"txid":external,"occurred_at":occurred,"direction":direction,"asset":asset,"amount":repr(&abs(amount)?),"fee":repr(&abs(fee)?),"fiat_rate":rate.as_ref().map(repr),"fiat_value":repr(&value),"fiat_currency":"EUR","pricing_source_kind":"exchange_execution","pricing_provider":"Coinfinity","pricing_pair":format!("{asset}-EUR"),"pricing_timestamp":occurred,"pricing_method":"coinfinity_csv","pricing_external_ref":reference,"pricing_quality":"exact","kind":kind,"description":description,"counterparty":"Coinfinity","raw_json":config::stored_json(&columns.json())});
    data[synthetic::MARKER] = json!(synthetic_id);
    if economics {
        data["_exchange_evidence_match_by_economics"] = json!(true);
        data["_exchange_evidence_match_time_tolerance_seconds"] = json!(172800);
    }
    Ok(Some(csv::Row {
        data,
        headers: vec![],
        rest: None,
        original: None,
    }))
}
