//! Bull Bitcoin order exports are commercial evidence, never observer authority.
//!
//! Kassiber reads only BTC from Bull Bitcoin (docs/plan/24-crypto-assets.md).
//! A wallet import counts an order in another crypto asset as `unread` when
//! the wallet's asset scope is `bitcoin` and refuses it by the coin's name
//! when it is `crypto`, and refuses BTC traded against another crypto asset
//! by name in either, instead of booking it as a fiat-priced trade.
use super::assets::{Scope, Tally};
use super::exchange::Columns;
use super::river::{abs, arithmetic, number, repr};
use super::tax_platform::{ticker, Ticker};
use super::*;

const PROVIDER: &str = "Bull Bitcoin";
const REQUIRED: &[&str] = &[
    "ORDER_STATUS",
    "PAYIN_AMOUNT",
    "PAYIN_CURRENCY",
    "PAYOUT_AMOUNT",
    "PAYOUT_CURRENCY",
    "COMPLETED_AT (UTC)",
    "TRANSACTION_ID",
];
/// The orders to import. With a `scope` (a target wallet), other crypto
/// assets' orders are counted in `tally` (`bitcoin`) or refused (`crypto`);
/// without one (the reference's reading, which relevant mode without a
/// wallet uses) the import never refuses them here: such an order matches no
/// book row, and BTC priced in another coin fails the profile currency check.
pub(super) fn parse(
    rows: Vec<csv::Row>,
    scope: Option<Scope>,
    tally: &mut Tally,
) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Bull Bitcoin")?;
    let mut unread = BTreeMap::new();
    let mut out = vec![];
    for (line, row) in (2..).zip(rows) {
        if let Some(record) = record(row, line, scope, &mut unread)? {
            out.push(record);
        }
    }
    tally.unread(PROVIDER, &unread);
    Ok(out)
}
fn record(
    row: csv::Row,
    line: usize,
    scope: Option<Scope>,
    unread: &mut BTreeMap<String, i64>,
) -> Result<Option<csv::Row>, AppError> {
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
    // A wallet import: another crypto asset on either side (paid in, else
    // paid out). Without a target wallet, the reference's reading.
    let other = |code: &Option<String>| {
        code.clone()
            .filter(|code| matches!(ticker(code), Ticker::Crypto))
    };
    let coin = other(&payin_currency).or_else(|| other(&payout_currency));
    if let (Some(scope), Some(coin)) = (scope, coin) {
        let name = format!("Bull Bitcoin CSV line {line}");
        let bitcoin = exchange::bitcoin(payin_currency.as_deref())
            .or_else(|| exchange::bitcoin(payout_currency.as_deref()));
        if bitcoin.is_some() {
            return Err(AppError::new(
                "validation",
                format!("{name} trades BTC against {coin}, which is not a fiat currency"),
            )
            .with_hint("Kassiber reads only BTC bought or sold for fiat from Bull Bitcoin. Record this order yourself (generic CSV/JSON import) after checking it in Bull Bitcoin, and import the export without it."));
        }
        if scope == Scope::Crypto {
            return Err(AppError::new(
                "validation",
                format!("{name} books {coin}; Kassiber reads only BTC from Bull Bitcoin"),
            )
            .with_hint(assets::only_btc(PROVIDER)));
        }
        *unread.entry(coin).or_default() += 1;
        return Ok(None);
    }
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
