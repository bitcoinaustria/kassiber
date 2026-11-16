//! `fetch_kraken_records`: signed `TradesHistory` then `Ledgers` pages, and
//! `normalize_kraken_records` over the merged payloads.
use super::*;
use assets::{Scope, Tally};
use kassiber_pyjson::BigInt;

pub(super) fn fetch(client: &Client<'_>) -> Result<Fetched, AppError> {
    let mut pages = 0;
    let trades = paginated(client, "/0/private/TradesHistory", "trades", &mut pages)?;
    let ledgers = paginated(client, "/0/private/Ledgers", "ledger", &mut pages)?;
    // The reference's hint for a crypto-quoted trade sends the user to the
    // generic ledger, which refuses the other asset's leg; a sync knows the
    // wallet, so it names the scope command that imports both legs.
    normalize(&ledgers, &trades, client.scope).map_err(|error| {
        if error.hint.as_deref() != Some(CROSS_ASSET) {
            return error;
        }
        error.with_hint(match client.scope {
            Scope::Bitcoin => format!(
                "Kassiber does not book one leg of a swap as a sale or purchase. To import both legs, run {} and sync again.",
                assets::keep_command(client.wallet)
            ),
            Scope::Crypto => "Kassiber did not find this trade's other leg; record the trade yourself with the generic CSV/JSON import, a swap as two rows sharing a swap_ref.".into(),
        })
    })
}
const CROSS_ASSET: &str =
    "Cross-asset Kraken trades need explicit user review through the generic ledger.";

/// The desktop key check: the first page of both histories the sync reads,
/// so a key without either query permission fails here. Kraken does not say
/// what else a key may do.
pub(super) fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    for path in ["/0/private/TradesHistory", "/0/private/Ledgers"] {
        if !matches!(at(&post(client, path, "0")?, "result"), Py::Dict(_)) {
            return Err(AppError::new(
                "protocol_error",
                "Kraken response was not a JSON result object",
            ));
        }
    }
    Ok(None)
}

/// `_kraken_private_paginated`: rows keep their first position and take their
/// latest value, and the reply is `{"result": {field: rows, "count": n}}`.
fn paginated(
    client: &Client<'_>,
    path: &str,
    field: &str,
    pages: &mut usize,
) -> Result<Py, AppError> {
    let mut rows: Vec<(Key, Py)> = vec![];
    let mut offset = BigInt::from(0);
    let mut total: Option<BigInt> = None;
    while total.as_ref().is_none_or(|total| offset < *total) {
        *pages += 1;
        if *pages > MAX_PAGES {
            return Err(limit());
        }
        let payload = post(client, path, &offset.to_string())?;
        let result = at(&payload, "result");
        if !matches!(result, Py::Dict(_)) {
            return Err(AppError::new(
                "protocol_error",
                "Kraken response was not a JSON result object",
            ));
        }
        let Py::Dict(page) = at(result, field) else {
            return Err(AppError::new(
                "protocol_error",
                format!("Kraken response result.{field} was not a JSON object"),
            ));
        };
        for (id, row) in page {
            if !matches!(row, Py::Dict(_)) {
                continue;
            }
            let id = Key::Str(key_text(id));
            match rows.iter_mut().find(|(k, _)| *k == id) {
                Some((_, slot)) => *slot = row.clone(),
                None => rows.push((id, row.clone())),
            }
        }
        if rows.len() > MAX_ROWS {
            return Err(limit());
        }
        let count = BigInt::from(rows.len());
        let parsed = match get(result, "count") {
            None => count.clone(),
            Some(value) => integer(value)?.unwrap_or_else(|| count.clone()),
        };
        if page.is_empty() || count >= parsed {
            break;
        }
        total = Some(parsed);
        offset += BigInt::from(page.len());
    }
    let count = Py::from(rows.len() as i64);
    Ok(dict([(
        "result",
        dict([(field, Py::Dict(rows)), ("count", count)]),
    )]))
}

/// `int(value)`; `TypeError` and `ValueError` fall back to the row count.
fn integer(value: &Py) -> Result<Option<BigInt>, AppError> {
    Ok(match value {
        Py::Int(n) => Some(n.clone()),
        Py::Bool(b) => Some(BigInt::from(i32::from(*b))),
        Py::Float(f) if f.is_nan() => None,
        Py::Float(f) if f.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Py::Float(f) => f.trunc().to_string().parse().ok(),
        Py::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok()),
        _ => None,
    })
}

/// `_kraken_private_post`: one nonce-signed form POST and its error check.
fn post(client: &Client<'_>, path: &str, offset: &str) -> Result<Py, AppError> {
    let nonce = now()?.as_nanos().to_string();
    let body = format!("nonce={nonce}&ofs={offset}");
    let mut secret = base64_decode(client.secret)
        .ok_or_else(|| AppError::new("config_error", "Kraken API secret must be base64 encoded"))?;
    let digest = sha256::Hash::hash(format!("{nonce}{body}").as_bytes());
    let signature = kassiber_chain::encode_base64(&hmac_sha512(
        &secret,
        &[path.as_bytes(), digest.as_byte_array()],
    ));
    secret.fill(0);
    let url = format!("{}{path}", client.base);
    let payload = client.read(
        "POST",
        &url,
        &[
            ("API-Key", client.key),
            ("API-Sign", &signature),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        Some(body.as_bytes()),
    )?;
    let errors = at(&payload, "error");
    if !truthy(errors) {
        return Ok(payload);
    }
    // One error string is one item, so an echoed credential stays whole.
    let items: Vec<PyStr> = match errors {
        Py::List(items) => items.iter().map(text).collect(),
        Py::Str(text) => vec![text.clone()],
        Py::Dict(items) => items.iter().map(|(k, _)| key_text(k)).collect(),
        other => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "'{}' object is not iterable",
                    kassiber_envelope::type_name(other)
                ),
            ))
        }
    };
    let permission = items
        .iter()
        .any(|item| lossy(&lower(item)).contains("permission"));
    let items = items.iter().map(|item| Py::Str(item.clone())).collect();
    Err(AppError::new(
        if permission {
            "auth_error"
        } else {
            "protocol_error"
        },
        "Kraken API returned an error",
    )
    .with_details(dict([("kraken_errors", Py::List(items))])))
}

/// `base64.b64decode(text)`: non-validating `binascii.a2b_base64`, which skips
/// characters outside the alphabet and stops at a complete pad sequence.
pub(super) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() {
        return None;
    }
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut result, mut quad, mut left, mut pads) = (vec![], 0, 0u8, 0);
    for c in text.bytes() {
        if c == b'=' {
            pads += 1;
            if quad >= 2 && quad + pads >= 4 {
                return Some(result);
            }
            continue;
        }
        let Some(v) = value(c) else { continue };
        pads = 0;
        match quad {
            0 => {
                quad = 1;
                left = v;
            }
            1 => {
                quad = 2;
                result.push((left << 2) | (v >> 4));
                left = v & 0x0f;
            }
            2 => {
                quad = 3;
                result.push((left << 4) | (v >> 2));
                left = v & 0x03;
            }
            _ => {
                quad = 0;
                result.push((left << 6) | v);
            }
        }
    }
    (quad == 0).then_some(result)
}

/// `_mapping_rows(payload, *keys)`. Paging always assembles mappings, so the
/// reference's list form is unreachable here.
fn mapping_rows(payload: &Py, keys: &[&str]) -> Vec<(PyStr, Py)> {
    let mut value = payload;
    for key in keys {
        if let Some(found) = get(value, key) {
            value = found;
        }
    }
    match value {
        Py::Dict(items) => items
            .iter()
            .filter(|(_, item)| matches!(item, Py::Dict(_)))
            .map(|(key, item)| (key_text(key), item.clone()))
            .collect(),
        _ => vec![],
    }
}

/// `normalize_kraken_records(ledger_payload, trades_payload)` in `bitcoin`
/// scope; in `crypto` scope every crypto asset, from the ledger grouped by
/// `refid` (docs/reference/imports.md#kraken-api).
pub(super) fn normalize(ledgers: &Py, trades: &Py, scope: Scope) -> Result<Fetched, AppError> {
    let mut trade_rows = mapping_rows(trades, &["result", "trades"]);
    if trade_rows.is_empty() {
        trade_rows = mapping_rows(trades, &["trades"]);
    }
    // dict(rows): a repeated id keeps its first position and its last value.
    let mut by_id: Vec<(PyStr, Py)> = vec![];
    for (id, row) in trade_rows {
        match by_id.iter_mut().find(|(k, _)| *k == id) {
            Some((_, slot)) => *slot = row,
            None => by_id.push((id, row)),
        }
    }
    let mut rows = mapping_rows(ledgers, &["result", "ledger"]);
    if rows.is_empty() {
        rows = mapping_rows(ledgers, &["ledger"]);
    }
    let mut tally = Tally::default();
    let (records, internal) = match scope {
        Scope::Bitcoin => (bitcoin(rows, &by_id, &mut tally)?, 0),
        Scope::Crypto => crypto(&rows, &by_id, &mut tally)?,
    };
    Ok(Fetched {
        records,
        skipped: (internal > 0).then(|| dict([("internal_moves", Py::from(internal))])),
        tally: Some(tally),
        reported: None,
    })
}

/// One ledger row's common fields, read in the reference's order so a bad
/// value reports the same error first.
#[derive(Clone)]
struct Parsed {
    ledger_id: PyStr,
    kind: PyStr,
    amount: PriceNumber,
    fee: PriceNumber,
    occurred_at: Option<PyStr>,
    external_id: PyStr,
    raw: Py,
}
fn parse(ledger_id: &PyStr, row: &Py) -> Result<Parsed, AppError> {
    let kind = at(row, "type");
    let kind = lower(&kassiber_pytext::strip_py(&if truthy(kind) {
        text(kind)
    } else {
        "".into()
    }));
    let amount = decimal(at(row, "amount"), "0")?;
    let fee = decimal(at(row, "fee"), "0")?;
    let occurred_at = epoch(at(row, "time"), false)?;
    Ok(Parsed {
        external_id: concat(&["kraken:".into(), ledger_id.clone()]),
        raw: merged(&[("ledger_id", Py::Str(ledger_id.clone()))], row, &[]),
        ledger_id: ledger_id.clone(),
        kind,
        amount,
        fee,
        occurred_at,
    })
}

/// The reference's Bitcoin-only mapping; every other non-fiat row is
/// counted by ticker.
fn bitcoin(
    rows: Vec<(PyStr, Py)>,
    by_id: &[(PyStr, Py)],
    tally: &mut Tally,
) -> Result<Vec<Py>, AppError> {
    let mut records = vec![];
    for (ledger_id, row) in rows {
        let currency = asset(at(&row, "asset"));
        if !is_bitcoin(&currency) {
            left_out(&row, tally);
            continue;
        }
        let parsed = parse(&ledger_id, &row)?;
        let token = parsed.kind.as_str().unwrap_or("");
        if matches!(token, "deposit" | "withdrawal") {
            records.push(movement(parsed, currency)?);
            continue;
        }
        if token != "trade" {
            return Err(AppError::new("validation", "")
                .with_python_message(concat(&[
                    "Kraken BTC ledger row '".into(),
                    ledger_id,
                    "' has unsupported type '".into(),
                    parsed.kind,
                    "'".into(),
                ]))
                .with_hint(
                    "Only BTC/LBTC deposits, withdrawals, and fiat-quoted trades are imported automatically.",
                ));
        }
        records.push(fiat_trade(parsed, currency, &row, by_id)?);
    }
    Ok(records)
}

/// Counts a row the `bitcoin` scope leaves out. Fiat is not a holding.
/// Bitcoin in a Kraken earn or staking balance (`XBT.M`) is left out here
/// too, as the reference left it out, and counted as BTC: the `crypto` scope
/// reads those balances.
fn left_out(row: &Py, tally: &mut Tally) {
    let raw = at(row, "asset");
    let code = code(raw).unwrap_or_else(|| normalized(raw));
    let class = aclass(row)
        .flatten()
        .unwrap_or_else(|| assets::class(&code));
    match class {
        _ if code.is_empty() => {}
        assets::Class::Bitcoin => {
            tally.admit(Scope::Bitcoin, &code, assets::Class::Crypto);
        }
        class => {
            tally.admit(Scope::Bitcoin, &code, class);
        }
    }
}

/// Kraken's own asset class (`aclass`): `Some(None)` for a currency (crypto
/// or fiat, told apart by code), `Some(Some(Excluded))` for a tokenized
/// asset such as an xStock, `None` for a class Kassiber does not know.
fn aclass(row: &Py) -> Option<Option<assets::Class>> {
    let class = at(row, "aclass");
    if !truthy(class) {
        return Some(None);
    }
    match lossy(&text(class)).as_str() {
        "currency" => Some(None),
        "tokenized_asset" => Some(Some(assets::Class::Excluded)),
        _ => None,
    }
}

/// A deposit or withdrawal: wallet movement, unpriced.
fn movement(parsed: Parsed, currency: String) -> Result<Py, AppError> {
    let inbound = at_least_zero(&parsed.amount)?;
    emit(Entry {
        provider: "Kraken",
        external_id: parsed.external_id,
        occurred_at: parsed.occurred_at,
        inbound,
        asset: currency,
        amount: parsed.amount,
        fee: parsed.fee,
        kind: if inbound { "deposit" } else { "withdrawal" },
        raw: parsed.raw,
        description: None,
        pricing: None,
    })
}

/// A fiat-quoted trade's crypto leg, priced from its `TradesHistory` row.
fn fiat_trade(
    parsed: Parsed,
    currency: String,
    row: &Py,
    by_id: &[(PyStr, Py)],
) -> Result<Py, AppError> {
    let Parsed {
        ledger_id,
        amount,
        fee,
        occurred_at,
        external_id,
        raw,
        ..
    } = parsed;
    let refid = at(row, "refid");
    let refid = truthy(refid).then(|| text(refid));
    let wanted = refid.clone().unwrap_or_else(|| "".into());
    let trade = by_id
        .iter()
        .find(|(id, _)| *id == wanted)
        .map(|(_, trade)| trade)
        .filter(|trade| truthy(trade));
    let Some(trade) = trade else {
        return Err(AppError::new("validation", "")
            .with_python_message(concat(&[
                "Kraken trade ledger row '".into(),
                ledger_id,
                "' is missing its TradesHistory row".into(),
            ]))
            .with_hint(
                "Provide both Kraken ledger and trade history so execution prices can be verified.",
            ));
    };
    let pair = at(trade, "pair");
    let pair = if truthy(pair) {
        lossy(&text(pair))
    } else {
        String::new()
    };
    let Some(fiat_currency) = quote(&pair, &currency)
        .map(|quote| fiat_code(asset(&s(quote.as_str()))))
        .filter(|code| is_fiat(code))
    else {
        let named = if is_bitcoin(&currency) {
            "BTC"
        } else {
            &currency
        };
        return Err(AppError::new("validation", "")
            .with_python_message(concat(&[
                format!("Kraken {named} trade '").into(),
                ledger_id,
                "' is not fiat-quoted".into(),
            ]))
            .with_hint(CROSS_ASSET));
    };
    let cost = abs(decimal(at(trade, "cost"), "0")?)?;
    let trade_fee = abs(decimal(at(trade, "fee"), "0")?)?;
    let inbound = at_least_zero(&amount)?;
    let fiat_value = if inbound {
        cost.clone()
    } else {
        nonnegative(arithmetic(&cost, &trade_fee, '-')?)?
    };
    let raw = merged(&[], &raw, &[("trade", trade.clone())]);
    // Kraken's price is cost / volume; without it, rebuild that rather
    // than divide a sell's net-of-fee proceeds.
    let price = at(trade, "price");
    let fiat_rate = if *price != Py::Null && *price != s("") {
        Some(decimal(price, "0")?)
    } else if truth(&cost) && truth(&amount) {
        Some(divide(&cost, &abs(amount.clone())?)?)
    } else {
        None
    };
    emit(Entry {
        provider: "Kraken",
        external_id,
        occurred_at,
        inbound,
        asset: currency,
        amount,
        fee,
        kind: if inbound { "buy" } else { "sell" },
        raw,
        description: None,
        pricing: Some(Pricing {
            fiat_currency,
            fiat_value,
            fiat_rate,
            method: "kraken_api",
            external_ref: refid.unwrap_or(ledger_id),
        }),
    })
}

/// `record(entry)` after the exact-unit check names the ledger row.
fn emit(entry: Entry) -> Result<Py, AppError> {
    let id = entry.external_id.clone();
    let id = lossy(&id);
    exact(
        &entry,
        &format!("ledger row '{}'", id.trim_start_matches("kraken:")),
    )?;
    record(entry)
}

/// Kraken's own names for an asset (`/0/public/Assets`): the legacy X/Z
/// four-letter codes and the short forms its pairs use.
const ALIASES: [(&str, &str); 24] = [
    ("XXBT", "BTC"),
    ("XBT", "BTC"),
    ("XXDG", "DOGE"),
    ("XDG", "DOGE"),
    ("XETH", "ETH"),
    ("XETC", "ETC"),
    ("XLTC", "LTC"),
    ("XXLM", "XLM"),
    ("XXMR", "XMR"),
    ("XXRP", "XRP"),
    ("XZEC", "ZEC"),
    ("XREP", "REP"),
    ("XMLN", "MLN"),
    ("XICN", "ICN"),
    ("XNMC", "NMC"),
    ("XXVN", "XVN"),
    ("XDAO", "DAO"),
    ("ZEUR", "EUR"),
    ("ZUSD", "USD"),
    ("ZGBP", "GBP"),
    ("ZCAD", "CAD"),
    ("ZJPY", "JPY"),
    ("ZAUD", "AUD"),
    ("ZCHF", "CHF"),
];
/// Kraken's balance suffixes: staked (`.S`), opt-in rewards (`.M`),
/// flexible (`.F`), bonded (`.B`), parachain (`.P`) and held (`.HOLD`). The
/// balance is still the base asset.
const SUFFIXES: [&str; 6] = ["S", "M", "F", "B", "P", "HOLD"];
/// Bonded staking balances that carry their bonding days in the code.
const BONDED: [(&str, &str); 7] = [
    ("ATOM21", "ATOM"),
    ("DOT28", "DOT"),
    ("FLOW14", "FLOW"),
    ("KAVA21", "KAVA"),
    ("KSM07", "KSM"),
    ("SCRT21", "SCRT"),
    ("SOL03", "SOL"),
];

/// The canonical code of a Kraken asset, or `None` when it cannot be mapped
/// exactly: an unknown suffix, a staked code whose trailing digits might be
/// bonding days or part of the ticker, or text that is no ticker.
pub(super) fn code(value: &Py) -> Option<String> {
    let code = normalized(value);
    let (base, suffixed) = match code.split_once('.') {
        Some((base, suffix)) if SUFFIXES.contains(&suffix) => (base, true),
        Some(_) => return None,
        None => (code.as_str(), false),
    };
    let base = match BONDED.iter().find(|(bonded, _)| *bonded == base) {
        Some((_, unbonded)) if suffixed => *unbonded,
        _ if suffixed
            && base.len() > 2
            && base.bytes().rev().take(2).all(|b| b.is_ascii_digit()) =>
        {
            return None
        }
        _ => base,
    };
    let base = ALIASES
        .iter()
        .find(|(alias, _)| *alias == base)
        .map_or(base, |(_, canonical)| *canonical);
    (base == "LBTC" || ticker(base)).then(|| base.to_owned())
}

/// One ledger row in `crypto` scope.
struct Leg<'a> {
    parsed: Parsed,
    row: &'a Py,
    code: String,
    class: assets::Class,
    subtype: String,
}
impl Leg<'_> {
    fn token(&self) -> &str {
        self.parsed.kind.as_str().unwrap_or("")
    }
    fn id(&self) -> String {
        lossy(&self.parsed.ledger_id)
    }
}

fn refused(message: String, hint: &str) -> AppError {
    AppError::new("validation", message).with_hint(hint.to_owned())
}
const MAPPED: &str = "Kassiber imports Kraken deposits, withdrawals, trades, Buy Crypto (spend/receive), staking and earn rewards, and spot/staking moves; anything else needs review: record it yourself with the generic CSV/JSON import (a swap as two rows sharing a swap_ref).";

fn leg<'a>(ledger_id: &PyStr, row: &'a Py) -> Result<Leg<'a>, AppError> {
    let parsed = parse(ledger_id, row)?;
    let id = lossy(ledger_id);
    let raw = at(row, "asset");
    let Some(code) = code(raw) else {
        return Err(refused(
            format!(
                "Kraken ledger row '{id}' has asset '{}', which Kassiber cannot map exactly",
                lossy(&text(raw))
            ),
            "Kassiber maps Kraken's asset codes, including its X/Z codes and staking suffixes, without guessing; record this row yourself with the generic CSV/JSON import.",
        ));
    };
    let class = match aclass(row) {
        Some(class) => class,
        None => {
            return Err(refused(
                format!(
                    "Kraken ledger row '{id}' has asset class '{}'",
                    lossy(&text(at(row, "aclass")))
                ),
                "Kassiber imports Kraken currency balances (crypto and fiat) and leaves out tokenized assets; any other asset class is not mapped.",
            ))
        }
    }
    .unwrap_or_else(|| assets::class(&code));
    let subtype = at(row, "subtype");
    let subtype = if truthy(subtype) {
        kassiber_pytext::lower(kassiber_pytext::strip(&lossy(&text(subtype))))
    } else {
        String::new()
    };
    Ok(Leg {
        parsed,
        row,
        class,
        code,
        subtype,
    })
}

/// Every crypto asset, from the ledger: deposits and withdrawals, trades and
/// Buy Crypto grouped by `refid`, rewards as income, and moves between
/// Kraken's own spot, staking and earn balances skipped. Returns the records
/// and the internal moves skipped.
fn crypto(
    rows: &[(PyStr, Py)],
    by_id: &[(PyStr, Py)],
    tally: &mut Tally,
) -> Result<(Vec<Py>, i64), AppError> {
    let legs = rows
        .iter()
        .map(|(id, row)| leg(id, row))
        .collect::<Result<Vec<_>, _>>()?;
    let mut members: BTreeMap<PyStr, Vec<usize>> = BTreeMap::new();
    for (index, leg) in legs.iter().enumerate() {
        if matches!(leg.token(), "trade" | "spend" | "receive") {
            let refid = at(leg.row, "refid");
            if !truthy(refid) {
                return Err(refused(
                    format!("Kraken {} row '{}' has no refid", leg.token(), leg.id()),
                    "Kraken groups the legs of a trade by refid; without it the legs cannot be matched.",
                ));
            }
            members.entry(text(refid)).or_default().push(index);
        }
    }
    let (mut records, mut internal) = (vec![], 0);
    for (index, leg) in legs.iter().enumerate() {
        if leg.class == assets::Class::Excluded {
            // Counted; a trade that pays for it in crypto is refused below.
            tally.admit(Scope::Crypto, &leg.code, leg.class);
        }
        match leg.token() {
            "trade" | "spend" | "receive" => {
                let refid = text(at(leg.row, "refid"));
                let group = &members[&refid];
                if group[0] == index {
                    let group: Vec<&Leg> = group.iter().map(|&i| &legs[i]).collect();
                    records.extend(trade(&refid, &group, by_id)?);
                }
            }
            // A fiat cash movement is not a holding; a tokenized asset is
            // never imported.
            _ if matches!(leg.class, assets::Class::Fiat | assets::Class::Excluded) => {}
            "deposit" | "withdrawal" => {
                if tally.admit(Scope::Crypto, &leg.code, leg.class) {
                    records.push(movement(leg.parsed.clone(), leg.code.clone())?);
                }
            }
            "staking" => records.push(reward(leg)?),
            "earn" if leg.subtype == "reward" => records.push(reward(leg)?),
            "earn"
                if matches!(
                    leg.subtype.as_str(),
                    "allocation" | "deallocation" | "autoallocation" | "migration"
                ) =>
            {
                internal += moved(leg)?;
            }
            "transfer"
                if matches!(
                    leg.subtype.as_str(),
                    "spottostaking" | "stakingfromspot" | "stakingtospot" | "spotfromstaking"
                ) =>
            {
                internal += moved(leg)?;
            }
            other => {
                let subtype = if leg.subtype.is_empty() {
                    String::new()
                } else {
                    format!(" (subtype '{}')", leg.subtype)
                };
                return Err(refused(
                    format!(
                        "Kraken ledger row '{}' has unsupported type '{other}'{subtype}",
                        leg.id()
                    ),
                    MAPPED,
                ));
            }
        }
    }
    Ok((records, internal))
}

/// A staking or earn reward: income, unpriced until reviewed.
fn reward(leg: &Leg) -> Result<Py, AppError> {
    if !positive(&leg.parsed.amount)? {
        return Err(refused(
            format!(
                "Kraken reward row '{}' does not credit a positive amount",
                leg.id()
            ),
            MAPPED,
        ));
    }
    let parsed = leg.parsed.clone();
    emit(Entry {
        provider: "Kraken",
        external_id: parsed.external_id,
        occurred_at: parsed.occurred_at,
        inbound: true,
        asset: leg.code.clone(),
        amount: parsed.amount,
        fee: parsed.fee,
        kind: "income",
        raw: parsed.raw,
        description: Some("Kraken reward".into()),
        pricing: None,
    })
}

/// A move between the user's own Kraken balances (spot, staking, earn):
/// both rows name the same asset, so the book skips them. A move that costs
/// a fee is not balance-neutral and is refused.
fn moved(leg: &Leg) -> Result<i64, AppError> {
    if !is_zero(&leg.parsed.fee)? {
        return Err(refused(
            format!(
                "Kraken {} row '{}' charges a fee on a move between Kraken balances",
                leg.token(),
                leg.id()
            ),
            "Moves between Kraken spot, staking and earn balances are skipped only when they cost nothing.",
        ));
    }
    Ok(1)
}

/// The legs of one `refid`: one crypto leg against fiat (a buy or a sell),
/// two crypto legs (a swap), and fee-only rows in a third asset (`KFEE`
/// credits, for instance) as their own `fee` rows.
fn trade(refid: &PyStr, group: &[&Leg], by_id: &[(PyStr, Py)]) -> Result<Vec<Py>, AppError> {
    let name = lossy(refid);
    let buy_crypto = |leg: &Leg| leg.token() != "trade";
    if group
        .iter()
        .any(|leg| buy_crypto(leg) != buy_crypto(group[0]))
    {
        return Err(refused(
            format!("Kraken trade '{name}' mixes trade and spend/receive rows"),
            MAPPED,
        ));
    }
    let (mut holdings, mut fees, mut cash, mut excluded) = (vec![], vec![], vec![], 0);
    for leg in group {
        if leg.class == assets::Class::Excluded {
            excluded += 1;
        } else if leg.class == assets::Class::Fiat {
            cash.push(*leg);
        } else if !is_zero(&leg.parsed.amount)? {
            holdings.push(*leg);
        } else if !is_zero(&leg.parsed.fee)? {
            fees.push(*leg);
        }
    }
    if excluded > 0 {
        // Bought or sold for fiat, it is left out (and counted); paid for
        // with crypto, the crypto side is a disposal that cannot be dropped.
        if !holdings.is_empty() {
            return Err(refused(
                format!("Kraken trade '{name}' exchanges crypto for a tokenized asset"),
                "Kassiber does not import tokenized assets, so a crypto leg against one cannot be booked; record it yourself with the generic CSV/JSON import.",
            ));
        }
        return Ok(vec![]);
    }
    if let Some(fee) = fees
        .iter()
        .find(|fee| holdings.iter().any(|leg| leg.code == fee.code))
    {
        return Err(refused(
            format!(
                "Kraken trade '{name}' has a separate fee row '{}' in a traded asset",
                fee.id()
            ),
            MAPPED,
        ));
    }
    let mut records = match holdings.as_slice() {
        [] => vec![],
        [leg] if !buy_crypto(leg) => vec![fiat_trade(
            leg.parsed.clone(),
            leg.code.clone(),
            leg.row,
            by_id,
        )?],
        [leg] => vec![cash_trade(&name, leg, &cash)?],
        [first, second] => swap(refid, first, second, &cash)?,
        legs => {
            return Err(refused(
                format!(
                    "Kraken trade '{name}' has {} crypto legs; a trade has one, a swap two",
                    legs.len()
                ),
                MAPPED,
            ))
        }
    };
    for leg in fees {
        let parsed = leg.parsed.clone();
        records.push(emit(Entry::third_asset_fee(
            "Kraken",
            parsed.external_id,
            parsed.occurred_at,
            leg.code.clone(),
            parsed.fee,
            parsed.raw,
            format!("Kraken fee for trade {name}").into(),
        ))?);
    }
    Ok(records)
}

/// Buy Crypto: one crypto leg against exactly one fiat leg moving the other
/// way. A buy is valued at the fiat spent; a sell at the fiat received net
/// of its fee, as trades are.
fn cash_trade(name: &str, leg: &Leg, cash: &[&Leg]) -> Result<Py, AppError> {
    let inbound = at_least_zero(&leg.parsed.amount)?;
    let moving: Vec<&&Leg> = cash
        .iter()
        .filter(|cash| !is_zero(&cash.parsed.amount).unwrap_or(true))
        .collect();
    let [counter] = moving.as_slice() else {
        return Err(refused(
            format!(
                "Kraken trade '{name}' has {} fiat legs; Buy Crypto has one",
                moving.len()
            ),
            MAPPED,
        ));
    };
    if at_least_zero(&counter.parsed.amount)? == inbound {
        return Err(refused(
            format!("Kraken trade '{name}' moves its crypto and fiat legs the same way"),
            MAPPED,
        ));
    }
    let gross = abs(counter.parsed.amount.clone())?;
    let fiat_value = if inbound {
        gross
    } else {
        nonnegative(arithmetic(&gross, &abs(counter.parsed.fee.clone())?, '-')?)?
    };
    let parsed = leg.parsed.clone();
    emit(Entry {
        provider: "Kraken",
        external_id: parsed.external_id,
        occurred_at: parsed.occurred_at,
        inbound,
        asset: leg.code.clone(),
        amount: parsed.amount,
        fee: parsed.fee,
        kind: if inbound { "buy" } else { "sell" },
        raw: merged(
            &[],
            &parsed.raw,
            &[("cash_leg", counter.parsed.raw.clone())],
        ),
        description: None,
        pricing: Some(Pricing {
            fiat_currency: counter.code.clone(),
            fiat_value,
            fiat_rate: None,
            method: "kraken_api",
            external_ref: name.into(),
        }),
    })
}

/// A crypto-to-crypto trade: two `swap` legs sharing the trade's `refid`,
/// unpriced (Kraken states no fiat value for them). Each leg keeps its own
/// fee in its own asset.
fn swap(refid: &PyStr, first: &Leg, second: &Leg, cash: &[&Leg]) -> Result<Vec<Py>, AppError> {
    let name = lossy(refid);
    for leg in cash {
        if !is_zero(&leg.parsed.amount)? || !is_zero(&leg.parsed.fee)? {
            return Err(refused(
                format!("Kraken swap '{name}' also moves fiat in row '{}'", leg.id()),
                MAPPED,
            ));
        }
    }
    let first_in = at_least_zero(&first.parsed.amount)?;
    if first_in == at_least_zero(&second.parsed.amount)? || first.code == second.code {
        return Err(refused(
            format!("Kraken swap '{name}' needs one asset out and another in"),
            MAPPED,
        ));
    }
    let reference = concat(&["kraken:".into(), refid.clone()]);
    [first, second]
        .into_iter()
        .map(|leg| {
            let parsed = leg.parsed.clone();
            let entry = Entry {
                provider: "Kraken",
                external_id: parsed.external_id,
                occurred_at: parsed.occurred_at,
                inbound: at_least_zero(&parsed.amount)?,
                asset: leg.code.clone(),
                amount: parsed.amount,
                fee: parsed.fee,
                kind: "swap",
                raw: parsed.raw,
                description: Some(format!("Kraken swap {name}").into()),
                pricing: None,
            };
            exact(&entry, &format!("ledger row '{}'", leg.id()))?;
            swap_leg(entry, &reference)
        })
        .collect()
}

/// `_kraken_quote_from_pair(pair, base_asset)`. Of the fiat suffixes, the
/// four-character `Z` forms sort first; at most one suffix per length matches.
pub(super) fn quote(pair: &str, base: &str) -> Option<String> {
    let normalized = kassiber_pytext::upper(pair).replace('/', "");
    let alias = if base == "BTC" { "XBT" } else { base };
    for candidate in [base, alias] {
        for prefix in [candidate.to_owned(), format!("X{candidate}")] {
            if let Some(rest) = normalized.strip_prefix(&prefix) {
                return (!rest.is_empty()).then(|| rest.to_owned());
            }
        }
    }
    FIAT.iter()
        .map(|code| format!("Z{code}"))
        .chain(FIAT.iter().map(|code| (*code).to_owned()))
        .find(|code| normalized.ends_with(code.as_str()))
}
