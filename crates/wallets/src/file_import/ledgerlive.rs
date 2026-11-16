use super::assets::{Scope, Tally};
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

/// The rows a wallet import keeps. Without a `scope` (no target wallet),
/// the reference's reading: BTC/LBTC rows only, others left out uncounted.
/// With one, another crypto asset's row is kept in the `crypto` scope as a
/// deposit or withdrawal of that asset, or counted (`assets::file_row`).
pub(super) fn parse(
    rows: Vec<csv::Row>,
    scope: Option<Scope>,
    tally: &mut Tally,
) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Ledger Live")?;
    rows.into_iter()
        .enumerate()
        .filter_map(|(index, row)| record(row, index + 1, scope, tally).transpose())
        .collect::<Result<_, _>>()
        .map(synthetic::number_occurrences)
}

pub(super) fn record(
    mut row: csv::Row,
    index: usize,
    scope: Option<Scope>,
    tally: &mut Tally,
) -> Result<Option<csv::Row>, AppError> {
    for (key, value) in row.data.as_object_mut().unwrap() {
        if ["account xpub", "account xpub fingerprint", "xpub"]
            .contains(&river::column(key).as_str())
        {
            *value = json!("[redacted]");
        }
    }
    let c = Columns::new(&row);
    let ticker = c.opt(&["Currency Ticker"]).map(|v| super::asset(&v));
    let name = format!("Ledger Live row {index}");
    let asset = match scope {
        None => exchange::bitcoin(ticker.as_deref()).map(str::to_owned),
        Some(scope) => assets::file_row(ticker.as_deref(), scope, tally, &name)?,
    };
    let Some(asset) = asset else {
        return Ok(None);
    };
    let bitcoin = exchange::bitcoin(Some(&asset)).is_some();
    let operation = c.get(&["Operation Type"]);
    let tag = kassiber_pytext::casefold(kassiber_pytext::strip(
        operation.as_deref().unwrap_or_default(),
    ));
    let (direction, kind) = match tag.as_str() {
        "in" | "receive" | "received" => ("inbound", "deposit"),
        "out" | "send" | "sent" => ("outbound", "withdrawal"),
        _ => {
            let operation = operation.as_deref().unwrap_or("None");
            return Err(AppError::new(
                "validation",
                match scope {
                    Some(_) => format!("{name} has unsupported operation type '{operation}'"),
                    None => format!("Ledger Live CSV has unsupported operation type '{operation}'"),
                },
            )
            .with_hint(
                "Ledger Live imports support IN/OUT wallet movement rows for BTC/LBTC only.",
            ));
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
            // A Bitcoin fee-only row is the reference's skip. Another
            // asset's (a token send's gas, a failed transaction, a contract
            // call) moves value the export does not attribute: refused.
            if bitcoin {
                return Ok(None);
            }
            return Err(AppError::new(
                "validation",
                format!("{name} has no {asset} amount but a fee of {}", repr(&fee)),
            )
            .with_hint("Ledger Live's export does not say which asset pays this fee or what the operation moved, so Kassiber does not guess it. Record it yourself with the generic CSV/JSON import."));
        }
        return Err(AppError::new(
            "validation",
            match scope {
                Some(_) => format!("{name} has no {asset} amount"),
                None => "Ledger Live CSV has a BTC row with an empty amount".into(),
            },
        ));
    }
    let fee = abs(number(c.get(&["Operation Fees"]))?
        .filter(truth)
        .unwrap_or_else(zero))?;
    let occurred = c.get(&["Operation Date"]);
    let amount = abs(amount.unwrap())?;
    // A token transfer's fee may be paid in the chain's coin, and the export
    // does not say which: an outgoing row of another asset with a fee is
    // refused rather than charge that fee in the wrong asset.
    if !bitcoin && direction == "outbound" && positive(&fee)? {
        return Err(AppError::new(
            "validation",
            format!("{name} sends {asset} with a fee of {}", repr(&fee)),
        )
        .with_hint("Ledger Live's export does not say which asset pays a send's fee (a token's fee is paid in its chain's coin), so Kassiber does not guess it. Record this send yourself with the generic CSV/JSON import."));
    }
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
                    Text(Some(&asset)),
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
