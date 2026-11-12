//! Read-only platform CSV parsing. A row refusal stays a visible diagnostic.
use super::cross_check::*;

pub(super) fn metadata() -> &'static Value {
    static META: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    META.get_or_init(|| kassiber_pytext::loads(include_str!("cross_check_metadata.json")).unwrap())
}
fn folded(raw: &str) -> String {
    kassiber_pytext::casefold(
        &raw.split(|c: char| kassiber_pytext::is_space(c as u32))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}
fn cell(record: &[(String, String)], names: &[&str]) -> String {
    for name in names {
        if let Some((_, v)) = record.iter().rev().find(|(k, _)| folded(k) == folded(name)) {
            if !v.is_empty() {
                return v.clone();
            }
        }
    }
    String::new()
}
fn presentation_cell(record: &[(String, String)], names: &[&str]) -> String {
    record
        .iter()
        .find(|(k, v)| {
            names.iter().any(|n| {
                kassiber_pytext::casefold(kassiber_pytext::strip(k)) == kassiber_pytext::casefold(n)
            }) && !kassiber_pytext::strip(v).is_empty()
        })
        .map(|(_, v)| kassiber_pytext::strip(v).to_owned())
        .unwrap_or_default()
}
fn abs(d: Number) -> Result<Number> {
    d.abs()
}
fn number(raw: &str, field: &str, label: &str) -> Result<Option<Number>> {
    let mut text = kassiber_pytext::strip(raw).to_owned();
    if text.is_empty() {
        return Ok(None);
    }
    let negative = text.starts_with('(') && text.ends_with(')');
    if negative {
        text = kassiber_pytext::strip(&text[1..text.len() - 1]).to_owned();
    }
    for symbol in [
        "$", "€", "£", "CHF", "USD", "EUR", "GBP", "BTC", "XBT", "LBTC", "SATS", "SAT",
    ] {
        text = text.replace(symbol, "");
    }
    text = kassiber_pytext::strip(&text.replace(['\u{a0}', ' '], "")).to_owned();
    if text.is_empty() {
        return Ok(None);
    }
    if text.contains('.') && text.contains(',') {
        text = if text.rfind('.') > text.rfind(',') {
            text.replace(',', "")
        } else {
            text.replace('.', "").replace(',', ".")
        };
    } else if text.contains(',') {
        text = if text.matches(',').count() > 1 {
            text.replace(',', "")
        } else {
            text.replace(',', ".")
        };
    } else if text.matches('.').count() > 1 {
        text = text.replace('.', "");
    }
    if negative && !text.starts_with('-') {
        text.insert(0, '-');
    }
    // A non-finite quantity or cash cell is no number either (f3a0289be).
    match text.parse() {
        Ok(Number::Finite(n)) => Ok(Some(Number::Finite(n))),
        _ => Err(()),
    }
    .map_err(|_| {
        AppError::new(
            "validation",
            format!(
                "{label}: {field} is not a number ({})",
                kassiber_pytext::repr_str(raw)
            ),
        )
    })
}
fn currency(raw: &str) -> String {
    let text = kassiber_pytext::strip(raw);
    if ["l-btc", "lbtc"].contains(&text.to_lowercase().as_str()) {
        "LBTC".into()
    } else if text.len() == 64 && text.bytes().all(|c| c.is_ascii_hexdigit()) {
        text.to_ascii_lowercase()
    } else {
        kassiber_pytext::upper(text)
    }
}
fn bitcoin(raw: &str) -> Option<&'static str> {
    match raw {
        "BTC" | "XBT" => Some("bitcoin:btc"),
        "LBTC" => Some("liquid:lbtc"),
        _ => None,
    }
}
fn fiat(raw: &str) -> bool {
    artifact_csv::list(value(metadata(), "fiat")).contains(&s(raw))
}
fn date(raw: &str, label: &str) -> Result<i64> {
    let raw = kassiber_pytext::strip(raw);
    if raw.is_empty() {
        return Err(AppError::new(
            "validation",
            format!("{label}: missing Date"),
        ));
    }
    let mut candidate = raw.to_owned();
    let mut parts = raw.splitn(3, '.');
    if let (Some(day), Some(month), Some(rest)) = (parts.next(), parts.next(), parts.next()) {
        let (year, tail) = rest.split_once([' ', 'T']).unwrap_or((rest, ""));
        if (1..=2).contains(&day.chars().count())
            && (1..=2).contains(&month.chars().count())
            && year.chars().count() == 4
            && [day, month, year].iter().all(|v| {
                v.chars()
                    .all(|c| kassiber_pytext::decimal_value(c as u32).is_some())
            })
        {
            let integer = |raw: &str| {
                kassiber_pytext::parse_int(raw)
                    .ok()
                    .and_then(|n| u32::try_from(n).ok())
            };
            if let (Some(day), Some(month)) = (integer(day), integer(month)) {
                candidate = format!(
                    "{year}-{month:02}-{day:02}{}",
                    if tail.is_empty() {
                        String::new()
                    } else {
                        format!("T{}", kassiber_pytext::strip(tail))
                    }
                );
            }
        }
    }
    utc(&candidate)?.ok_or_else(|| {
        AppError::new(
            "validation",
            format!("{label}: Date '{raw}' is not a recognized date"),
        )
    })
}
fn normalize(
    record: &[(String, String)],
    index: usize,
    tool: &str,
    label: &str,
    book_currency: &str,
) -> Result<Option<Tool>> {
    let ct = tool == "cointracking";
    let c = |names: &[&str]| cell(record, names);
    let incoming = currency(&c(if ct {
        &["Buy Currency"]
    } else {
        &["Incoming Asset"]
    }));
    let outgoing = currency(&c(if ct {
        &["Sell Currency"]
    } else {
        &["Outgoing Asset"]
    }));
    let mut fee_asset = currency(&c(if ct {
        &["Fee Currency", "Fee Cur."]
    } else {
        &["Fee Asset", "Fee Asset (optional)"]
    }));
    let mut in_asset = bitcoin(&incoming);
    let mut out_asset = bitcoin(&outgoing);
    if in_asset.is_some() && out_asset.is_some() {
        return Err(AppError::new(
            "validation",
            format!("{label} row {index} has Bitcoin on both sides"),
        ));
    }
    let fee_raw = c(if ct {
        &["Fee", "Fee Amount"]
    } else {
        &["Fee Amount", "Fee Amount (optional)"]
    });
    let mut fee = number(&fee_raw, "Fee Amount", &format!("{label} row {index}"))?;
    let standalone = in_asset.is_none() && out_asset.is_none() && bitcoin(&fee_asset).is_some();
    if standalone {
        out_asset = bitcoin(&fee_asset);
        fee_asset.clear();
        fee = None;
    }
    if in_asset.is_none() && out_asset.is_none() {
        return Ok(None);
    }
    let inbound = in_asset.is_some();
    let opposing = if inbound { &outgoing } else { &incoming };
    if !opposing.is_empty() && !fiat(opposing) {
        return Err(AppError::new(
            "validation",
            format!("{label} row {index} trades Bitcoin against unsupported asset {opposing}"),
        ));
    }
    let direction = if inbound { "inbound" } else { "outbound" };
    let raw_type = c(if ct { &["Type"] } else { &["Label"] });
    let key = folded(&raw_type);
    let mapped = if standalone {
        "Spend".into()
    } else if key.is_empty() {
        return Err(AppError::new(
            "validation",
            format!("{label} row is missing its transaction type"),
        ));
    } else if key == "trade" {
        if inbound { "Buy" } else { "Sell" }.into()
    } else if key == "transfer" {
        if inbound { "Deposit" } else { "Withdrawal" }.into()
    } else {
        let v = value(value(metadata(), &format!("{tool}_types")), &key);
        if matches!(v, Value::Null) {
            return Err(AppError::new(
                "validation",
                format!(
                    "{label} transaction type '{}' needs review before import",
                    kassiber_pytext::strip(&raw_type)
                ),
            ));
        }
        py_str(v)
    };
    // Unsupported third-asset fees are refused before generic leg parsing.
    let fee_supported =
        bitcoin(&fee_asset).is_some() || (fiat(&fee_asset) && &fee_asset == opposing);
    if !fee_supported
        && fee
            .as_ref()
            .map(|v| v.abs()?.greater(&Number::zero()))
            .transpose()?
            .unwrap_or(false)
    {
        let display = if fee_asset.is_empty() {
            "None"
        } else {
            &fee_asset
        };
        return Err(AppError::new(
            "validation",
            format!("{label} row {index} has a fee in unsupported asset {display}"),
        ));
    }
    let row_label = format!("Ledger row {index}");
    let received = number(
        &if !standalone && (inbound || fiat(opposing)) {
            c(if ct {
                &["Buy Amount"]
            } else {
                &["Incoming Amount"]
            })
        } else {
            String::new()
        },
        "Received Amount",
        &row_label,
    )?;
    let sent = number(
        &if !standalone && (!inbound || fiat(opposing)) {
            c(if ct {
                &["Sell Amount"]
            } else {
                &["Outgoing Amount"]
            })
        } else {
            String::new()
        },
        "Sent Amount",
        &row_label,
    )?;
    let amount = if standalone {
        number(&fee_raw, "Sent Amount", &row_label)?
    } else if inbound {
        received.clone()
    } else {
        sent.clone()
    };
    let Some(amount) = amount else {
        return Err(AppError::new("validation",format!("{row_label}: no Bitcoin leg found (expected BTC, LBTC, or SATS on the Received or Sent side)")));
    };
    let generic = value(value(metadata(), "generic_types"), &mapped.to_lowercase());
    let generic = artifact_csv::list(generic);
    if generic[0] != s(direction) {
        return Err(AppError::new(
            "validation",
            format!(
                "{row_label}: Type '{mapped}' is {} but the Bitcoin leg is {direction}",
                py_str(&generic[0])
            ),
        ));
    }
    let mut kind = if matches!(generic[1], Value::Null) {
        None
    } else {
        Some(py_str(&generic[1]))
    };
    if (ct && key == "expense (non taxable)")
        || (!ct && ["non-taxable (out)", "steuerfrei (aus)"].contains(&key.as_str()))
    {
        kind = Some("expense_non_taxable".into());
    }
    let cash = if fiat(opposing) {
        if inbound {
            sent
        } else {
            received
        }
    } else {
        None
    };
    let mut fee_btc = Number::zero();
    let mut fee_cash = Number::zero();
    if fee_supported {
        if let Some(fee) = fee {
            if fee.abs()?.greater(&Number::zero())? {
                if bitcoin(&fee_asset).is_some() {
                    fee_btc = abs(fee)?;
                } else if cash.is_none() {
                    return Err(AppError::new(
                        "validation",
                        format!("{row_label}: a fiat fee ({fee_asset}) needs a matching cash leg"),
                    ));
                } else {
                    fee_cash = abs(fee)?;
                }
            }
        }
    }
    let mut cash = cash
        .map(abs)
        .transpose()?
        .map(|n| {
            if inbound {
                n.add(&fee_cash)
            } else {
                let delta = n.sub(&fee_cash)?;
                Ok(if delta.greater(&Number::zero())? {
                    delta
                } else {
                    Number::zero()
                })
            }
        })
        .transpose()?;
    // The importer calculates a rate even though this report consumes only value.
    if cash.is_some() && !amount.is_zero() {
        amount.greater(&Number::zero())?;
    }
    if !["", book_currency].contains(&opposing.as_str()) {
        cash = None;
    }
    let mut value_error = String::new();
    if cash.is_none() {
        let raw = presentation_cell(
            record,
            if inbound {
                &[
                    "Buy Value in Account Currency",
                    "Buy Value in EUR",
                    "Buy Value",
                ]
            } else {
                &[
                    "Sell Value in Account Currency",
                    "Sell Value in EUR",
                    "Sell Value",
                ]
            },
        );
        let mut cleaned = raw.replace(['\u{a0}', ' '], "");
        cleaned = if cleaned.contains(',') && cleaned.rfind(',') > cleaned.rfind('.') {
            cleaned.replace('.', "").replace(',', ".")
        } else {
            cleaned.replace(',', "")
        };
        // `_decimal_cell`: junk is no value; a non-finite value marks only
        // this row reviewable instead of refusing the whole comparison.
        match cleaned.parse::<Number>() {
            Ok(Number::Finite(n)) => cash = Number::Finite(n).abs().ok(),
            Ok(_) => value_error = "Comparison values must be finite".into(),
            Err(_) => {}
        }
    }
    in_asset = in_asset.or(out_asset);
    Ok(Some(Tool {
        index,
        source: String::new(),
        label: String::new(),
        txid: kassiber_pytext::strip(&c(if ct {
            &["Tx-ID (optional)", "Tx-ID", "Trade ID"]
        } else {
            &["Trx. ID", "Trx. ID (optional)", "Transaction ID"]
        }))
        .into(),
        at: Some(date(
            &c(if ct {
                &["Date"]
            } else {
                &["Date (UTC)", "Date"]
            }),
            &row_label,
        )?),
        direction: direction.into(),
        asset: in_asset.unwrap().into(),
        amount: abs(amount)?.msat()?,
        fee: fee_btc.msat()?,
        kind,
        cash,
        error: value_error,
    }))
}
pub(super) fn tool_rows(
    bytes: &[u8],
    tool: &str,
    label: &str,
    book_currency: &str,
) -> Result<(Vec<Tool>, usize)> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|e| {
        let at = e.valid_up_to();
        let len = e.error_len().unwrap_or(bytes.len() - at);
        let location = if len == 1 {
            format!("byte 0x{:02x} in position {at}", bytes[at])
        } else {
            format!("bytes in position {at}-{}", at + len - 1)
        };
        let reason = if e.error_len().is_none() {
            "unexpected end of data"
        } else if !(0xc2..=0xf4).contains(&bytes[at]) {
            "invalid start byte"
        } else {
            "invalid continuation byte"
        };
        AppError::new(
            "internal_error",
            format!("'utf-8' codec can't decode {location}: {reason}"),
        )
    })?;
    let mut records = super::cross_check_dialect::read(text)?
        .into_iter()
        .filter(|r| r.iter().any(|s| !kassiber_pytext::strip(s).is_empty()));
    let header = records
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|s| kassiber_pytext::strip(&s).to_owned())
        .collect::<Vec<_>>();
    let missing = artifact_csv::list(value(metadata(), &format!("{tool}_required")))
        .iter()
        .filter(|v| !header.iter().any(|h| folded(h) == folded(&py_str(v))))
        .map(py_str)
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(AppError::new(
            "validation",
            format!(
                "{label} CSV is missing required columns: {}",
                missing.join(", ")
            ),
        )
        .with_hint(format!(
            "Export the full transaction history from {label}, not a summary or tax-form CSV."
        )));
    }
    let mut result = Vec::new();
    let mut other = 0;
    for (position, raw) in records.enumerate() {
        let index = position + 1;
        let mut record: Vec<(String, String)> = Vec::new();
        for (i, h) in header.iter().enumerate().filter(|(_, h)| !h.is_empty()) {
            let v = raw.get(i).cloned().unwrap_or_default();
            if let Some((_, slot)) = record.iter_mut().find(|(k, _)| k == h) {
                *slot = v;
            } else {
                record.push((h.clone(), v));
            }
        }
        let parsed = normalize(&record, index, tool, label, book_currency);
        let mut row = match parsed {
            Ok(None) => {
                other += 1;
                continue;
            }
            Ok(Some(row)) => row,
            Err(e) if e.code == "internal_error" => return Err(e),
            Err(e) => Tool {
                index,
                source: String::new(),
                label: String::new(),
                txid: presentation_cell(
                    &record,
                    &["Tx-ID", "Tx-ID (optional)", "Trx. ID", "Trx. ID (optional)"],
                ),
                at: utc(&presentation_cell(&record, &["Date", "Date (UTC)"]))?,
                direction: String::new(),
                asset: String::new(),
                amount: 0.into(),
                fee: 0.into(),
                kind: None,
                cash: None,
                error: e.message,
            },
        };
        row.source = presentation_cell(
            &record,
            if tool == "cointracking" {
                &["Exchange"]
            } else {
                &["Integration Name"]
            },
        );
        if row.source.is_empty() {
            row.source = "—".into();
        }
        row.label = presentation_cell(
            &record,
            if tool == "cointracking" {
                &["Type"]
            } else {
                &["Label"]
            },
        );
        result.push(row);
    }
    Ok((result, other))
}
