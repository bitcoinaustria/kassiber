use super::exchange::Columns;
use super::river::{abs, number, positive, repr, truth, zero};
use super::*;

const REQUIRED: &[&str] = &[
    "Operation Date",
    "Currency Ticker",
    "Operation Type",
    "Operation Amount",
    "Operation Hash",
];

pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Ledger Live")?;
    rows.into_iter()
        .enumerate()
        .filter_map(|(index, row)| record(row, index + 1).transpose())
        .collect::<Result<_, _>>()
        .map(synthetic::number_occurrences)
}

pub(super) fn record(mut row: csv::Row, index: usize) -> Result<Option<csv::Row>, AppError> {
    for (key, value) in row.data.as_object_mut().unwrap() {
        if ["account xpub", "account xpub fingerprint", "xpub"]
            .contains(&river::column(key).as_str())
        {
            *value = json!("[redacted]");
        }
    }
    let c = Columns::new(&row);
    let Some(asset) = exchange::bitcoin(
        c.opt(&["Currency Ticker"])
            .map(|v| super::asset(&v))
            .as_deref(),
    ) else {
        return Ok(None);
    };
    let operation = c.get(&["Operation Type"]);
    let tag = kassiber_pytext::casefold(kassiber_pytext::strip(
        operation.as_deref().unwrap_or_default(),
    ));
    let (direction, kind) = match tag.as_str() {
        "in" | "receive" | "received" => ("inbound", "deposit"),
        "out" | "send" | "sent" => ("outbound", "withdrawal"),
        _ => {
            return Err(AppError::new(
                "validation",
                format!(
                    "Ledger Live CSV has unsupported operation type '{}'",
                    operation.as_deref().unwrap_or("None")
                ),
            )
            .with_hint(
                "Ledger Live imports support IN/OUT wallet movement rows for BTC/LBTC only.",
            ))
        }
    };
    let amount = number(c.get(&["Operation Amount"]))?;
    if amount
        .as_ref()
        .is_some_and(|n| matches!(n, PriceNumber::Special(v) if v.contains("sNaN")))
    {
        return Err(decimal_error(DecimalError::InvalidOperation));
    }
    if amount.as_ref().is_none_or(|n| !truth(n)) {
        let fee = abs(number(c.get(&["Operation Fees"]))?
            .filter(truth)
            .unwrap_or_else(zero))?;
        if positive(&fee)? {
            return Ok(None);
        }
        return Err(AppError::new(
            "validation",
            "Ledger Live CSV has a BTC row with an empty amount",
        ));
    }
    let fee = abs(number(c.get(&["Operation Fees"]))?
        .filter(truth)
        .unwrap_or_else(zero))?;
    let occurred = c.get(&["Operation Date"]);
    let amount = abs(amount.unwrap())?;
    let _ = index;
    let hash = c.opt(&["Operation Hash"]);
    let synthetic_id = hash.is_none();
    let txid = match hash {
        Some(hash) => hash,
        None => {
            use synthetic::Part::{Number, Text};
            synthetic::economic_id(
                "ledgerlive",
                &[
                    Text(occurred.as_deref()),
                    Text(Some(direction)),
                    Text(Some(asset)),
                    Number(&amount),
                    Number(&fee),
                ],
            )?
        }
    };
    let account = c.opt(&["Account Name"]);
    let description = account.map_or_else(
        || format!("Ledger Live {kind}"),
        |name| format!("Ledger Live {kind} - {name}"),
    );
    Ok(Some(csv::Row {
        data: json!({
            "txid": txid,
            "occurred_at": occurred,
            "direction": direction,
            "asset": asset,
            "amount": repr(&amount),
            "fee": repr(&if direction == "outbound" { fee } else { zero() }),
            "kind": kind,
            "description": description,
            "counterparty": "Ledger Live",
            "raw_json": config::stored_json(&c.json()),
            synthetic::MARKER: synthetic_id,
        }),
        headers: vec![],
        rest: None,
        original: None,
    }))
}
