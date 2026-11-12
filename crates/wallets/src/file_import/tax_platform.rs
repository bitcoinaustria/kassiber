use super::exchange::Columns;
use super::river::{abs, arithmetic, nonnegative, positive, repr, truth, zero};
use super::*;

const COINTRACKING_REQUIRED: &[&str] = &[
    "Type",
    "Buy Amount",
    "Buy Currency",
    "Sell Amount",
    "Sell Currency",
    "Date",
];
const BLOCKPIT_REQUIRED: &[&str] = &[
    "Date (UTC)",
    "Integration Name",
    "Label",
    "Outgoing Asset",
    "Outgoing Amount",
    "Incoming Asset",
    "Incoming Amount",
];
struct Platform {
    name: &'static str,
    required: &'static [&'static str],
    incoming_asset: &'static [&'static str],
    incoming_amount: &'static [&'static str],
    outgoing_asset: &'static [&'static str],
    outgoing_amount: &'static [&'static str],
    fee_asset: &'static [&'static str],
    fee_amount: &'static [&'static str],
    date: &'static [&'static str],
    kind: &'static [&'static str],
    counterparty: &'static [&'static str],
    txid: &'static [&'static str],
    note: [&'static [&'static str]; 2],
}
const COINTRACKING: Platform = Platform {
    name: "CoinTracking",
    required: COINTRACKING_REQUIRED,
    incoming_asset: &["Buy Currency"],
    incoming_amount: &["Buy Amount"],
    outgoing_asset: &["Sell Currency"],
    outgoing_amount: &["Sell Amount"],
    fee_asset: &["Fee Currency", "Fee Cur."],
    fee_amount: &["Fee", "Fee Amount"],
    date: &["Date"],
    kind: &["Type"],
    counterparty: &["Exchange"],
    txid: &["Tx-ID (optional)", "Tx-ID", "Trade ID"],
    note: [&["Trade-Group"], &["Comment"]],
};
const BLOCKPIT: Platform = Platform {
    name: "Blockpit",
    required: BLOCKPIT_REQUIRED,
    incoming_asset: &["Incoming Asset"],
    incoming_amount: &["Incoming Amount"],
    outgoing_asset: &["Outgoing Asset"],
    outgoing_amount: &["Outgoing Amount"],
    fee_asset: &["Fee Asset", "Fee Asset (optional)"],
    fee_amount: &["Fee Amount", "Fee Amount (optional)"],
    date: &["Date (UTC)", "Date"],
    kind: &["Label"],
    counterparty: &["Integration Name"],
    txid: &["Trx. ID", "Trx. ID (optional)", "Transaction ID"],
    note: [
        &["Comments", "Comment", "Comment (optional)"],
        &["Source Name"],
    ],
};
fn platform(format: Format) -> &'static Platform {
    match format {
        Format::Blockpit => &BLOCKPIT,
        _ => &COINTRACKING,
    }
}
pub(super) const FIAT: &[&str] = &[
    "AED", "ARS", "AUD", "BRL", "CAD", "CHF", "CLP", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD",
    "HUF", "ILS", "INR", "JPY", "KRW", "MXN", "NOK", "NZD", "PLN", "RON", "SEK", "SGD", "THB",
    "TRY", "USD", "ZAR",
];
fn fail(message: impl Into<String>, hint: &str) -> AppError {
    AppError::new("validation", message).with_hint(hint)
}
pub(super) fn number(
    value: Option<String>,
    field: &str,
    label: &str,
) -> Result<Option<PriceNumber>, AppError> {
    number_in(value, field, label, None)
}

/// Python's `_GENERIC_LEDGER_CURRENCY_SYMBOLS`, in alternation order.
const UNIT_SYMBOLS: [&str; 13] = [
    "$", "€", "£", "CHF", "USD", "EUR", "GBP", "L-BTC", "LBTC", "BTC", "XBT", "SATS", "SAT",
];

/// One character of `re.IGNORECASE` matching against an ASCII symbol: a
/// letter matches its case variants, and `s` also matches the long s.
fn unit_char(actual: char, symbol: char) -> bool {
    if !symbol.is_ascii_alphabetic() {
        return actual == symbol;
    }
    let lower = symbol.to_ascii_lowercase();
    let mut folded = actual.to_lowercase();
    (folded.next() == Some(lower) && folded.next().is_none()) || (lower == 's' && actual == 'ſ')
}

/// `canonical_unit` in `_generic_ledger_decimal_cell`.
fn canonical_unit(code: &str) -> String {
    if code == "SATS" || code == "SAT" {
        return "SATS".into();
    }
    match kassiber_pytext::upper(kassiber_pytext::strip(code)).as_str() {
        "BTC" | "XBT" | "XXBT" => "BTC".into(),
        "LBTC" | "L-BTC" => "LBTC".into(),
        _ => code.into(),
    }
}

/// `re.sub` over the unit symbols, case-insensitively, refusing a unit
/// that names a different asset than the cell's asset column.
fn strip_units(
    text: &str,
    field: &str,
    label: &str,
    asset: Option<&str>,
) -> Result<String, AppError> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        let found = UNIT_SYMBOLS.iter().find(|symbol| {
            let symbol: Vec<char> = symbol.chars().collect();
            at + symbol.len() <= chars.len()
                && symbol
                    .iter()
                    .zip(&chars[at..])
                    .all(|(s, a)| unit_char(*a, *s))
        });
        let Some(symbol) = found else {
            out.push(chars[at]);
            at += 1;
            continue;
        };
        let width = symbol.chars().count();
        let matched: String = chars[at..at + width].iter().collect();
        let upper = kassiber_pytext::upper(&matched);
        let unit = match upper.as_str() {
            "$" => "USD".to_owned(),
            "€" => "EUR".to_owned(),
            "£" => "GBP".to_owned(),
            _ => upper,
        };
        if let Some(asset) = asset.filter(|a| !a.is_empty()) {
            if canonical_unit(&unit) != canonical_unit(asset) {
                return Err(fail(
                    format!("{label}: {field} unit {unit} does not match asset {asset}"),
                    "Use the amount unit named by the asset column; SATS and BTC have different units.",
                ));
            }
        }
        at += width;
    }
    Ok(out)
}

/// `_generic_ledger_decimal_cell(value, field, row_label, asset=asset)`.
pub(super) fn number_in(
    value: Option<String>,
    field: &str,
    label: &str,
    asset: Option<&str>,
) -> Result<Option<PriceNumber>, AppError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let mut text = kassiber_pytext::strip(&raw).to_owned();
    let negative = text.starts_with('(') && text.ends_with(')');
    if negative {
        text = kassiber_pytext::strip(&text[1..text.len() - 1]).into();
    }
    text = strip_units(&text, field, label, asset)?;
    text = kassiber_pytext::strip(&text.replace(['\u{a0}', ' '], "")).into();
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
    // A non-finite amount is "not a number" too, as in Python.
    match PriceNumber::parse(&text) {
        Ok(PriceNumber::Special(_)) => Err(()),
        parsed => parsed.map_err(|_| ()),
    }
    .map(Some)
    .map_err(|_| {
        fail(
            format!(
                "{label}: {field} is not a number ({})",
                kassiber_pytext::repr_str(&raw)
            ),
            "Use digits like 0.05000000 or 3000,00 — amounts are in BTC unless the asset is SATS.",
        )
    })
}
pub(super) fn date(value: Option<String>, index: usize) -> Result<String, AppError> {
    let text = value
        .as_deref()
        .map(kassiber_pytext::strip)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            fail(
                format!("Ledger row {index}: missing Date"),
                "Use an ISO date such as 2026-01-15, a full timestamp, or DD.MM.YYYY.",
            )
        })?;
    let mut candidate = text.to_owned();
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(&format!(
            r"^({d}{{1,2}})\.({d}{{1,2}})\.({d}{{4}})(?:[ T](.+))?$",
            d = kassiber_pytext::REGEX_DIGIT
        ))
        .unwrap()
    });
    if let Some(c) = re.captures(text) {
        let digits = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    char::from(b'0' + kassiber_pytext::decimal_value(u32::from(c)).unwrap() as u8)
                })
                .collect()
        };
        candidate = format!(
            "{}-{:02}-{:02}",
            &c[3],
            digits(&c[2]).parse::<u32>().unwrap(),
            digits(&c[1]).parse::<u32>().unwrap()
        );
        if let Some(rest) = c.get(4) {
            candidate.push('T');
            candidate.push_str(kassiber_pytext::strip(rest.as_str()));
        }
    }
    crate::time::timestamp(&candidate).map_err(|_| {
        fail(
            format!("Ledger row {index}: Date '{text}' is not a recognized date"),
            "Use ISO 2026-01-15, a full RFC3339 timestamp, or DD.MM.YYYY.",
        )
    })?;
    Ok(candidate)
}
fn mapped(
    raw: Option<String>,
    inbound: bool,
    provider: &Platform,
) -> Result<(&'static str, &'static str, Option<&'static str>), AppError> {
    let text = raw
        .as_deref()
        .map(kassiber_pytext::strip)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "validation",
                format!("{} row is missing its transaction type", provider.name),
            )
        })?;
    let key = river::column(text);
    let key = if provider.name == "Blockpit" {
        match key.as_str() {
            "trade" | "transfer" => key.as_str(),
            "unlabeled deposit"
            | "unlabeled incoming"
            | "unlabeled (in)"
            | "unlabeled incoming (deposit)"
            | "fehlendes label (ein)"
            | "deposit" => "deposit",
            "non-taxable (in)" | "steuerfrei (ein)" | "gift (incoming)" | "gift (in)"
            | "geschenk (ein)" => "gift / tip",
            "airdrop" => "airdrop",
            "bounty"
            | "cashback"
            | "income"
            | "other income"
            | "sonstiger ertrag"
            | "masternode"
            | "security token income"
            | "security-token-ertrag" => "income",
            "staking" => "staking",
            "interest" | "zinsen" => "interest income",
            "hard fork" | "hardfork" => "fork",
            "mining" => "mining",
            "unlabeled withdrawal"
            | "unlabeled outgoing"
            | "unlabeled (out)"
            | "unlabeled outgoing (withdrawal)"
            | "fehlendes label (aus)"
            | "withdrawal" => "withdrawal",
            "non-taxable (out)" | "steuerfrei (aus)" | "gift (outgoing)" | "gift (out)"
            | "geschenk (aus)" => "gift",
            "payment" | "zahlung" | "fee" | "gebühr" => "spend",
            "lost" | "verloren" => "lost",
            _ => "unsupported",
        }
    } else {
        key.as_str()
    };
    let (label,direction,kind) = match key {
        "trade" if inbound => ("Buy","inbound",Some("buy")),
        "trade" => ("Sell","outbound",Some("sell")),
        "transfer" if inbound => ("Deposit","inbound",Some("deposit")),
        "transfer" => ("Withdrawal","outbound",Some("withdrawal")),
        "deposit" => ("Deposit","inbound",Some("deposit")),
        "withdrawal" => ("Withdrawal","outbound",Some("withdrawal")),
        "gift / tip" | "gift/tip(in)" | "income (non taxable)" | "other income (non taxable)" | "airdrop (non taxable)" => ("Gift received","inbound",None),
        "mining" | "mining (commercial)" => ("Mining","inbound",Some("mining")),
        "fork" if provider.name == "Blockpit" => ("Fork","inbound",Some("hardfork")),
        "staking" => ("Staking","inbound",Some("staking")),
        "airdrop" => ("Airdrop","inbound",Some("airdrop")),
        "lending income" => ("Lending Interest","inbound",Some("lending_interest")),
        "interest income" => ("Interest","inbound",Some("interest")),
        "income" | "reward / bonus" | "masternode" | "minting" | "dividends income" | "lp rewards" | "other income" => ("Income","inbound",Some("income")),
        "spend" | "other fee" | "other expense" => ("Spend","outbound",Some("spend")),
        "donation" => ("Donation","outbound",Some("donation")),
        "gift" | "gift(out)" | "expense (non taxable)" => ("Gift sent","outbound",Some("gift")),
        "stolen" | "stolen / hacked / fraud" => ("Stolen","outbound",Some("stolen")),
        "lost" => ("Lost","outbound",Some("lost")),
        _ => return Err(fail(format!("{} transaction type '{text}' needs review before import", provider.name),"This type has tax semantics Kassiber cannot safely infer. Filter it into a separate export or convert the reviewed row with the Generic ledger template.")),
    };
    Ok((label, direction, kind))
}
pub(super) fn load(file: &str, format: Format) -> Result<Vec<csv::Row>, AppError> {
    let provider = platform(format);
    let (header, rows) = platform_csv::load(file)?;
    let missing: Vec<_> = provider
        .required
        .iter()
        .filter(|k| !header.iter().any(|h| river::column(h) == river::column(k)))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(fail(
            format!(
                "{} CSV is missing required columns: {}",
                provider.name,
                missing.join(", ")
            ),
            &format!(
                "Export the full transaction history from {}, not a summary or tax-form CSV.",
                provider.name
            ),
        ));
    }
    let mut occurrences = BTreeMap::<String, usize>::new();
    let mut result = vec![];
    for (index, row) in rows.into_iter().enumerate() {
        if let Some(mut row) = record(row, index + 1, format)? {
            let payload = dict([
                ("amount", number_float(&row.data["amount"])),
                ("asset", s(text(&row.data, "asset"))),
                ("direction", s(text(&row.data, "direction"))),
                ("fee", number_float(&row.data["fee"])),
                ("occurred_at", s(text(&row.data, "occurred_at"))),
            ]);
            let encoded = kassiber_pyjson::dumps(&payload, &Default::default()).unwrap();
            let digest = Sha256::digest(encoded.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let occurrence = occurrences.entry(digest.clone()).or_default();
            *occurrence += 1;
            row.data["_tax_platform_row_identity"] =
                json!(format!("{}:{digest}:{occurrence}", format.migration_kind()));
            result.push(row);
        }
    }
    Ok(result)
}
fn number_float(value: &Json) -> Py {
    let n: f64 = value.as_str().unwrap().parse().unwrap_or(f64::NAN);
    Py::Float(n)
}
fn record(row: csv::Row, index: usize, format: Format) -> Result<Option<csv::Row>, AppError> {
    let provider = platform(format);
    let name = provider.name;
    let c = Columns::new(&row);
    let code = |names: &[&str]| c.opt(names).map(|v| asset(&v));
    let incoming = code(provider.incoming_asset);
    let mut outgoing = code(provider.outgoing_asset);
    let inbound = exchange::bitcoin(incoming.as_deref());
    let mut outbound = exchange::bitcoin(outgoing.as_deref());
    if inbound.is_some() && outbound.is_some() {
        return Err(fail(
            format!("{name} row {index} has Bitcoin on both sides"),
            "Review BTC/LBTC swaps explicitly instead of importing them as a fiat trade.",
        ));
    }
    let mut fee_code = code(provider.fee_asset);
    let mut fee_raw = c.get(provider.fee_amount);
    let mut fee = number(
        fee_raw.clone(),
        "Fee Amount",
        &format!("{name} row {index}"),
    )?;
    let standalone =
        inbound.is_none() && outbound.is_none() && exchange::bitcoin(fee_code.as_deref()).is_some();
    let mut sent_raw = c.get(provider.outgoing_amount);
    if standalone {
        outgoing = fee_code.clone();
        outbound = exchange::bitcoin(outgoing.as_deref());
        sent_raw = fee_raw.take();
        fee_code = None;
        fee = None;
    }
    if inbound.is_none() && outbound.is_none() {
        return Ok(None);
    }
    let opposing = if inbound.is_some() {
        &outgoing
    } else {
        &incoming
    };
    if let Some(code) = opposing.as_deref().filter(|v| !FIAT.contains(v)) {
        return Err(fail(format!("{name} row {index} trades Bitcoin against unsupported asset {code}"),"The provider CSV does not identify the currency behind its account-currency value columns. Review this cross-asset row with the Generic ledger and enter an explicit fiat currency and value."));
    }
    let (label, type_direction, mut kind) = if standalone {
        ("Spend", "outbound", Some("spend"))
    } else {
        mapped(c.get(provider.kind), inbound.is_some(), provider)?
    };
    let fiat = opposing.as_deref().filter(|v| FIAT.contains(v));
    let accepted_fee = exchange::bitcoin(fee_code.as_deref()).is_some()
        || fee_code
            .as_deref()
            .is_some_and(|v| FIAT.contains(&v) && Some(v) == fiat);
    if !accepted_fee {
        if fee
            .as_ref()
            .map(|n| abs(n.clone()).and_then(|n| positive(&n)))
            .transpose()?
            .unwrap_or(false)
        {
            return Err(fail(format!("{name} row {index} has a fee in unsupported asset {}",fee_code.as_deref().or(c.get(provider.fee_asset).as_deref()).unwrap_or("None")),"Kassiber cannot safely convert a third-asset fee into the Bitcoin or fiat leg. Review this row with the Generic ledger template."));
        }
        fee_raw = None;
    }
    let ledger = format!("Ledger row {index}");
    // The generic ledger row this maps to names each leg's asset, and the
    // ledger checks every amount unit against it.
    let received_asset = (inbound.is_some() || fiat.is_some())
        .then(|| incoming.clone())
        .flatten();
    let sent_asset = (outbound.is_some() || fiat.is_some())
        .then(|| outgoing.clone())
        .flatten();
    let received = if inbound.is_some() || fiat.is_some() {
        number_in(
            c.get(provider.incoming_amount),
            "Received Amount",
            &ledger,
            received_asset.as_deref(),
        )?
    } else {
        None
    };
    let sent = if outbound.is_some() || fiat.is_some() {
        number_in(sent_raw, "Sent Amount", &ledger, sent_asset.as_deref())?
    } else {
        None
    };
    let crypto = |asset: &str| {
        ["BTC", "XBT", "XXBT", "LBTC", "L-BTC", "SATS", "SAT"]
            .contains(&kassiber_pytext::upper(asset).as_str())
    };
    let fee_unit = fee_code
        .as_deref()
        .filter(|code| accepted_fee && !code.is_empty())
        .or_else(|| match received_asset.as_deref() {
            Some(asset) if crypto(asset) => Some(asset),
            _ => sent_asset.as_deref(),
        });
    let fee = number_in(fee_raw, "Fee Amount", &ledger, fee_unit)?;
    let amount = if inbound.is_some() { received.clone() } else { sent.clone() }.ok_or_else(|| fail(format!("Ledger row {index}: no Bitcoin leg found (expected BTC, LBTC, or SATS on the Received or Sent side)"),"Put the Bitcoin amount + asset on the Received side (inbound) or the Sent side (outbound)."))?;
    let amount = abs(amount)?;
    let direction = if inbound.is_some() {
        "inbound"
    } else {
        "outbound"
    };
    if direction != type_direction {
        return Err(fail(format!("Ledger row {index}: Type '{label}' is {type_direction} but the Bitcoin leg is {direction}"),"Inbound Types (Buy/Deposit/Income/…) need the Bitcoin amount on the Received side; outbound Types (Sell/Withdrawal/Spend/Gift sent/…) on the Sent side."));
    }
    let (mut btc_fee, mut fiat_fee) = (zero(), zero());
    if let Some(fee) = fee {
        let fee = abs(fee)?;
        if positive(&fee)? {
            if exchange::bitcoin(fee_code.as_deref()).is_some() {
                btc_fee = fee;
            } else {
                let cash = if inbound.is_some() {
                    sent.as_ref()
                } else {
                    received.as_ref()
                };
                if cash.is_none() {
                    return Err(fail(format!("{ledger}: a fiat fee ({}) needs a matching cash leg",fee_code.as_deref().unwrap()),"Put network fees in BTC (Fee Asset BTC or blank), or add the Sent/Received fiat amount this fee applies to."));
                }
                fiat_fee = fee;
            }
        }
    }
    let cash = if inbound.is_some() { sent } else { received };
    let priced = cash.is_some() && fiat.is_some();
    let value = if let Some(cash) = cash.filter(|_| priced) {
        let base = abs(cash)?;
        Some(if direction == "inbound" {
            arithmetic(&base, &fiat_fee, '+')?
        } else {
            nonnegative(arithmetic(&base, &fiat_fee, '-')?)?
        })
    } else {
        None
    };
    let rate = if let Some(value) = &value {
        if truth(&amount) && positive(&amount)? {
            Some(arithmetic(value, &amount, '/')?)
        } else {
            None
        }
    } else {
        None
    };
    let occurred = date(c.get(provider.date), index)?;
    let source_type = river::column(&c.get(provider.kind).unwrap_or_default());
    if format == Format::CoinTracking && source_type == "expense (non taxable)"
        || format == Format::Blockpit
            && ["non-taxable (out)", "steuerfrei (aus)"].contains(&source_type.as_str())
    {
        kind = Some("expense_non_taxable");
    }
    let counterparty = c.opt(provider.counterparty).unwrap_or_else(|| name.into());
    let note = [c.opt(provider.note[0]), c.opt(provider.note[1])]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let txid = c.opt(provider.txid);
    Ok(Some(csv::Row {
        data: json!({"txid":txid.clone().unwrap_or_default(),"occurred_at":occurred,"direction":direction,"asset":inbound.or(outbound).unwrap(),"amount":repr(&amount),"fee":repr(&btc_fee),"fiat_value":value.as_ref().map(repr),"fiat_rate":rate.as_ref().map(repr),"fiat_currency":if priced { fiat } else { None },"pricing_source_kind":if priced {Some("exchange_execution")} else {None},"pricing_provider":if priced {Some(&counterparty)} else {None},"pricing_pair":if priced {Some(format!("{}-{}",inbound.or(outbound).unwrap(),fiat.unwrap()))} else {None},"pricing_method":format.name(),"pricing_external_ref":txid,"pricing_quality":if priced {Some("exact")} else {None},"kind":kind,"description":if note.is_empty() {None} else {Some(note)},"counterparty":counterparty,"raw_json":config::stored_json(&c.json())}),
        headers: vec![],
        rest: None,
        original: None,
    }))
}

fn query(conn: &Connection, sql: &str, args: &[Sql]) -> Result<Vec<Fields>, AppError> {
    let mut stmt = conn.prepare(sql).map_err(database_error)?;
    let keys = stmt
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let rows = stmt
        .query_map(params_from_iter(args), |row| {
            keys.iter()
                .enumerate()
                .map(|(i, key)| Ok((key.clone(), row.get(i)?)))
                .collect()
        })
        .map_err(database_error)?;
    rows.collect::<Result<_, _>>().map_err(database_error)
}
fn chain_id(value: &str) -> Option<String> {
    let value = kassiber_pytext::strip(value);
    (value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}
fn economic(row: &Fields) -> (String, String, String, i64) {
    let amount = if let Sql::Integer(amount) = row["amount"] {
        amount
    } else {
        0
    };
    (
        string(row, "occurred_at"),
        string(row, "direction"),
        string(row, "asset"),
        amount,
    )
}
pub(super) fn reconcile(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    ids: &mut Ids<'_>,
    result: &mut Vec<(&str, Py)>,
    invalidated: bool,
) -> Result<(), AppError> {
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [&request.profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let authoritative = query(conn,"SELECT t.external_id,t.occurred_at,t.direction,t.asset,t.amount FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND w.kind IN ('descriptor','silent-payment','xpub')",&[Sql::Text(request.profile_id.clone())])?;
    let rows = query(conn,"SELECT t.id,t.external_id,t.occurred_at,t.direction,t.asset,t.amount,t.fee,t.excluded FROM transactions t WHERE t.wallet_id=? ORDER BY t.occurred_at,t.created_at,t.id",&[Sql::Text(wallet.id.clone())])?;
    let edits = query(conn,"SELECT e.transaction_id,f.after_value FROM transaction_edit_events e JOIN transaction_edit_fields f ON f.event_id=e.id JOIN transactions t ON t.id=e.transaction_id WHERE t.wallet_id=? AND f.field='excluded' ORDER BY e.transaction_id,e.journal_input_version_after DESC,e.changed_at DESC,e.id DESC,f.id DESC",&[Sql::Text(wallet.id.clone())])?;
    let mut authored = BTreeMap::new();
    for edit in edits {
        authored
            .entry(string(&edit, "transaction_id"))
            .or_insert_with(|| {
                serde_json::from_str::<Json>(&string(&edit, "after_value"))
                    .ok()
                    .and_then(|v| v.as_bool())
            });
    }
    let prefix = request.format.migration_kind();
    let name = platform(request.format).name;
    let matched_code = format!("{prefix}-matched");
    let ambiguous_code = format!("{prefix}-ambiguous");
    let tag_args = [
        Sql::Text(request.profile_id.clone()),
        Sql::Text(matched_code),
        Sql::Text(ambiguous_code),
    ];
    let tags = query(
        conn,
        "SELECT id,code FROM tags WHERE profile_id=? AND code IN (?,?)",
        &tag_args,
    )?;
    let mut tag_ids: BTreeMap<String, String> = tags
        .iter()
        .map(|r| (string(r, "code"), string(r, "id")))
        .collect();
    let affected: std::collections::BTreeSet<String> = result
        .iter()
        .filter(|(key, _)| ["inserted_records", "updated_records"].contains(key))
        .flat_map(|(_, value)| {
            if let Py::List(rows) = value {
                rows.iter().collect()
            } else {
                vec![]
            }
        })
        .filter_map(|row| {
            kassiber_envelope::get(row, "transaction_id")
                .and_then(|v| if let Py::Str(s) = v { s.as_str() } else { None })
                .map(str::to_owned)
        })
        .collect();
    let mut by_txid = BTreeMap::new();
    let mut by_economics = std::collections::BTreeSet::new();
    for row in &authoritative {
        if let Some(id) = chain_id(&string(row, "external_id")) {
            *by_txid
                .entry((id, string(row, "direction"), string(row, "asset")))
                .or_insert(0) += 1;
        }
        by_economics.insert(economic(row));
    }
    let mut source_counts = BTreeMap::new();
    let string_fee = |row: &Fields| {
        if let Sql::Integer(fee) = row["fee"] {
            fee
        } else {
            0
        }
    };
    for row in &rows {
        if string(row, "external_id").is_empty() {
            *source_counts
                .entry((economic(row), string_fee(row)))
                .or_insert(0) += 1;
        }
    }
    let (
        mut matched,
        mut ambiguous,
        mut unmatched,
        mut excluded,
        mut changed,
        mut cleared,
        mut reactivated,
        mut total,
    ) = (0, 0, 0, 0, 0, 0, 0, 0);
    for row in &rows {
        let id = string(row, "id");
        if !affected.contains(&id) {
            continue;
        }
        total += 1;
        let txid = chain_id(&string(row, "external_id"));
        let matches = txid
            .and_then(|id| {
                by_txid
                    .get(&(id, string(row, "direction"), string(row, "asset")))
                    .copied()
            })
            .unwrap_or(0);
        let collision = string(row, "external_id").is_empty()
            && source_counts
                .get(&(economic(row), string_fee(row)))
                .copied()
                .unwrap_or(0)
                > 1;
        let status = if matches > 0 {
            if matches == 1 {
                "matched"
            } else {
                "ambiguous"
            }
        } else if collision || by_economics.contains(&economic(row)) {
            "ambiguous"
        } else {
            "unmatched"
        };
        match status {
            "matched" => matched += 1,
            "ambiguous" => ambiguous += 1,
            _ => unmatched += 1,
        };
        let current = query(conn,"SELECT tags.code FROM transaction_tags tt JOIN tags ON tags.id=tt.tag_id WHERE tt.transaction_id=? AND tags.profile_id=? AND tags.code IN (?,?)",&[Sql::Text(id.clone()),tag_args[0].clone(),tag_args[1].clone(),tag_args[2].clone()])?;
        let is_excluded = row.get("excluded") != Some(&Sql::Integer(0));
        let choice = authored.get(&id).copied().flatten();
        if status == "unmatched" {
            if !current.is_empty() {
                for tag in tag_ids.values() {
                    conn.execute(
                        "DELETE FROM transaction_tags WHERE transaction_id=? AND tag_id=?",
                        [&id, tag],
                    )
                    .map_err(database_error)?;
                }
                changed += 1;
                cleared += 1;
                if choice != Some(true) && is_excluded {
                    conn.execute("UPDATE transactions SET excluded=0 WHERE id=?", [&id])
                        .map_err(database_error)?;
                    reactivated += 1;
                }
            }
            continue;
        }
        let desired = choice.unwrap_or(true);
        excluded += i64::from(desired);
        let code = format!("{prefix}-{status}");
        if current.len() == 1 && string(&current[0], "code") == code && is_excluded == desired {
            continue;
        }
        for (suffix, label) in [
            ("matched", format!("{name} matched")),
            ("ambiguous", format!("{name} ambiguous")),
        ] {
            let code = format!("{prefix}-{suffix}");
            if let std::collections::btree_map::Entry::Vacant(entry) = tag_ids.entry(code) {
                let tag = ids.next(conn)?;
                conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)",params![tag,workspace,request.profile_id,entry.key(),label,ids.time(conn)?]).map_err(database_error)?;
                entry.insert(tag);
            }
        }
        for tag in tag_ids.values() {
            conn.execute(
                "DELETE FROM transaction_tags WHERE transaction_id=? AND tag_id=?",
                [&id, tag],
            )
            .map_err(database_error)?;
        }
        conn.execute(
            "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",
            [&id, &tag_ids[&code]],
        )
        .map_err(database_error)?;
        conn.execute(
            "UPDATE transactions SET excluded=? WHERE id=?",
            params![i64::from(desired), id],
        )
        .map_err(database_error)?;
        changed += 1;
    }
    if changed > 0 && !invalidated {
        invalidate_journals(conn, &request.profile_id)?;
    }
    result
        .iter_mut()
        .find(|(k, _)| *k == "journal_invalidated")
        .unwrap()
        .1 = Py::Bool(invalidated || changed > 0);
    result.extend([
        ("matched", Py::from(matched)),
        ("ambiguous", Py::from(ambiguous)),
        ("unmatched", Py::from(unmatched)),
        ("excluded", Py::from(excluded)),
        ("reconciliation_rows", Py::from(total)),
        ("reconciliation_changed", Py::from(changed)),
    ]);
    if cleared > 0 {
        result.push(("reconciliation_flags_cleared", Py::from(cleared)));
    }
    if reactivated > 0 {
        result.push(("reactivated", Py::from(reactivated)));
    }
    Ok(())
}
