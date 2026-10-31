//! River's local Bitcoin Activity and Account Activity exports.
use super::*;

const REQUIRED: &[&str] = &[
    "Date",
    "Sent Amount",
    "Sent Currency",
    "Received Amount",
    "Received Currency",
];

// Only folds that can contribute to these ASCII column names are relevant.
// Rust's lowercase covers ordinary case mappings; Python casefold also folds
// long-s and the Latin presentation ligatures into ASCII sequences.
pub(super) fn column(key: &str) -> String {
    kassiber_pytext::lower(key)
        .replace('ſ', "s")
        .replace('ß', "ss")
        .replace('ﬀ', "ff")
        .replace('ﬁ', "fi")
        .replace('ﬂ', "fl")
        .replace('ﬃ', "ffi")
        .replace('ﬄ', "ffl")
        .replace(['ﬅ', 'ﬆ'], "st")
        .split(|c: char| kassiber_pytext::is_space(u32::from(c)))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn clean_decimal(raw: &str) -> Option<String> {
    let mut text = kassiber_pytext::strip(raw).to_owned();
    let negative = text.starts_with('(') && text.ends_with(')');
    if negative {
        text = kassiber_pytext::strip(&text[1..text.len() - 1]).into();
    }
    for symbol in [
        "$", "€", "£", "CHF", "USD", "EUR", "GBP", "LBTC", "BTC", "XBT",
    ] {
        text = text.replace(symbol, "");
    }
    text = text.replace([',', ' '], "");
    text = kassiber_pytext::strip(&text).into();
    if text.is_empty() {
        return None;
    }
    if negative && !text.starts_with('-') {
        text.insert(0, '-');
    }
    Some(text)
}
pub(super) fn number(raw: Option<String>) -> Result<Option<PriceNumber>, AppError> {
    raw.as_deref()
        .and_then(clean_decimal)
        .map(|s| PriceNumber::parse(&s))
        .transpose()
}
pub(super) fn abs(n: PriceNumber) -> Result<PriceNumber, AppError> {
    match n {
        PriceNumber::Finite(v) => v.abs(&CTX).map(PriceNumber::Finite).map_err(decimal_error),
        PriceNumber::Special(v) if v.contains("sNaN") => {
            Err(decimal_error(DecimalError::InvalidOperation))
        }
        PriceNumber::Special(v) => Ok(PriceNumber::Special(v.trim_start_matches('-').into())),
    }
}
pub(super) fn zero() -> PriceNumber {
    PriceNumber::Finite(Decimal::zero())
}
pub(super) fn truth(n: &PriceNumber) -> bool {
    !matches!(n, PriceNumber::Finite(v) if v.is_zero())
}
pub(super) fn repr(n: &PriceNumber) -> String {
    match n {
        PriceNumber::Finite(v) => v.to_string(),
        PriceNumber::Special(v) => v.clone(),
    }
}
pub(super) fn positive(n: &PriceNumber) -> Result<bool, AppError> {
    match n {
        PriceNumber::Finite(v) => Ok(v.is_positive()),
        PriceNumber::Special(v) if v.contains("NaN") => {
            Err(decimal_error(DecimalError::InvalidOperation))
        }
        PriceNumber::Special(v) => Ok(!v.starts_with('-')),
    }
}
pub(super) fn arithmetic(
    a: &PriceNumber,
    b: &PriceNumber,
    op: char,
) -> Result<PriceNumber, AppError> {
    use PriceNumber::{Finite, Special};
    if let (Finite(a), Finite(b)) = (a, b) {
        return match op {
            '+' => a.add(b, &CTX),
            '-' => a.sub(b, &CTX),
            '*' => a.mul(b, &CTX),
            '/' => a.div(b, &CTX),
            _ => unreachable!(),
        }
        .map(Finite)
        .map_err(decimal_error);
    }
    let na = repr(a);
    let nb = repr(b);
    if na.contains("sNaN") || nb.contains("sNaN") {
        return Err(decimal_error(DecimalError::InvalidOperation));
    }
    if na.contains("NaN") {
        return Ok(Special(na));
    }
    if nb.contains("NaN") {
        return Ok(Special(nb));
    }
    let ai = na.contains("Infinity");
    let bi = nb.contains("Infinity");
    let an = na.starts_with('-');
    let bn = nb.starts_with('-');
    if op == '*' && (!truth(a) || !truth(b))
        || op == '/' && ai && bi
        || matches!(op, '+' | '-') && ai && bi && (an != bn) == (op == '+')
    {
        return Err(decimal_error(DecimalError::InvalidOperation));
    }
    if op == '/' && bi {
        return Ok(zero());
    }
    let negative = if matches!(op, '*' | '/') {
        an != bn
    } else if ai {
        an
    } else {
        bn != (op == '-')
    };
    Ok(Special(
        if negative { "-Infinity" } else { "Infinity" }.into(),
    ))
}
pub(super) fn nonnegative(n: PriceNumber) -> Result<PriceNumber, AppError> {
    if repr(&n).contains("NaN") {
        return Err(decimal_error(DecimalError::InvalidOperation));
    }
    // Python max(Decimal("0"), value) retains the first operand on equality.
    Ok(if positive(&n)? { n } else { zero() })
}
fn kind(tag: &str) -> String {
    let tag = kassiber_pytext::lower(kassiber_pytext::strip(tag));
    match tag.as_str() {
        "automatic withdrawal" => "withdrawal".into(),
        "cash deposit" => "cash_deposit".into(),
        "cash withdrawal" => "cash_withdrawal".into(),
        "mining payout" => "mining".into(),
        "referral reward" => "income".into(),
        "bitcoin interest on cash" => "interest".into(),
        "" => "river_activity".into(),
        _ => tag.replace(' ', "_"),
    }
}
pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    let Some(first) = rows.first() else {
        return Ok(rows);
    };
    let missing = REQUIRED
        .iter()
        .filter(|k| {
            !first
                .data
                .as_object()
                .unwrap()
                .keys()
                .any(|h| column(h) == column(k))
        })
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(AppError::new(
            "app_error",
            format!(
                "River CSV is missing required columns: {}",
                missing.join(", ")
            ),
        ));
    }
    rows.into_iter()
        .filter_map(|row| record(row).transpose())
        .collect()
}
fn record(row: csv::Row) -> Result<Option<csv::Row>, AppError> {
    // Python strips keys before casefold lookup. Both operations retain dict
    // insertion order, including colliding aliases and duplicate headings.
    let mut entries: Vec<(String, Json)> = Vec::new();
    let mut original_keys = std::collections::BTreeSet::new();
    for key in &row.headers {
        if !original_keys.insert(key) {
            continue;
        }
        let clean = kassiber_pytext::strip(key).to_owned();
        let value = row.data[key].clone();
        if let Some((_, old)) = entries.iter_mut().find(|(k, _)| *k == clean) {
            *old = value;
        } else {
            entries.push((clean, value));
        }
    }
    let get = |names: &[&str]| -> Option<String> {
        names.iter().find_map(|name| {
            entries
                .iter()
                .rev()
                .find(|(k, _)| column(k) == column(name))
                .and_then(|(_, v)| v.as_str())
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
    };
    let opt = |names: &[&str]| {
        get(names)
            .map(|s| kassiber_pytext::strip(&s).to_owned())
            .filter(|s| !s.is_empty())
    };
    let currency = |name| opt(&[name]).map(|v| asset(&v));
    let sent_currency = currency("Sent Currency");
    let received_currency = currency("Received Currency");
    let fee_currency = currency("Fee Currency");
    let price_currency = currency("Bitcoin Price Currency");
    let btc = |c: &Option<String>| {
        c.as_deref()
            .is_some_and(|v| ["BTC", "XBT", "XXBT"].contains(&v))
    };
    let leg = |name, currency: &Option<String>, is_btc| -> Result<Option<PriceNumber>, AppError> {
        if currency.is_none() || btc(currency) != is_btc {
            return Ok(None);
        }
        number(get(&[name]))?.map(abs).transpose()
    };
    let sent_btc = leg("Sent Amount", &sent_currency, true)?;
    let received_btc = leg("Received Amount", &received_currency, true)?;
    let sent_fiat = leg("Sent Amount", &sent_currency, false)?;
    let received_fiat = leg("Received Amount", &received_currency, false)?;
    let fee_btc = leg("Fee Amount", &fee_currency, true)?
        .filter(truth)
        .unwrap_or_else(zero);
    let fee_fiat = leg("Fee Amount", &fee_currency, false)?
        .filter(truth)
        .unwrap_or_else(zero);
    let price = number(get(&["Bitcoin Price Amount"]))?;
    let reference = opt(&["Transaction ID", "Reference Code"]);
    let tag = opt(&["Tag", "Transaction Type"]);
    let transaction_type = opt(&["Transaction Type"]).or_else(|| tag.clone());
    let (direction, amount, value, cash, kind) = if let Some(amount) = received_btc {
        if let Some(fiat) = &sent_fiat {
            (
                "inbound",
                amount,
                Some(arithmetic(fiat, &fee_fiat, '+')?),
                true,
                "buy".into(),
            )
        } else {
            let value = price
                .as_ref()
                .map(|p| arithmetic(&amount, p, '*'))
                .transpose()?;
            (
                "inbound",
                amount,
                value,
                false,
                kind(transaction_type.as_deref().unwrap_or_default()),
            )
        }
    } else if let Some(amount) = sent_btc {
        if let Some(fiat) = &received_fiat {
            (
                "outbound",
                amount,
                Some(nonnegative(arithmetic(fiat, &fee_fiat, '-')?)?),
                true,
                "sell".into(),
            )
        } else {
            let value = price
                .as_ref()
                .map(|p| arithmetic(&amount, p, '*'))
                .transpose()?;
            (
                "outbound",
                amount,
                value,
                false,
                kind(transaction_type.as_deref().unwrap_or_default()),
            )
        }
    } else {
        return Ok(None);
    };
    let rate = match &value {
        Some(value) if positive(&amount)? => Some(arithmetic(value, &amount, '/')?),
        _ => price,
    };
    let fiat_currency = if sent_fiat.is_some() {
        sent_currency
    } else if received_fiat.is_some() {
        received_currency
    } else {
        price_currency
    };
    let source = opt(&["Source"]);
    let destination = opt(&["Destination"]);
    let method = opt(&["Method"]);
    let description = [
        transaction_type.as_deref(),
        method.as_deref(),
        source.as_deref(),
        destination.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" - ");
    let description = if description.is_empty() {
        "Imported from River".into()
    } else {
        description
    };
    let payment_hash = get(&["Payment Hash", "payment_hash"]).and_then(|v| hash(&v));
    let occurred_at = get(&["Date"]);
    let sanitized = Json::Object(entries.into_iter().collect());
    Ok(Some(csv::Row {
        original: None,
        headers: vec![],
        rest: None,
        data: json!({
            "txid": reference, "occurred_at": occurred_at, "direction": direction, "asset":"BTC",
            "amount":repr(&amount), "fee":repr(&fee_btc), "fiat_rate":rate.as_ref().map(repr), "fiat_value":value.as_ref().map(repr), "fiat_currency":fiat_currency,
            "pricing_source_kind":if cash { Some("exchange_execution") } else if value.is_some() { Some("fmv_provider") } else { None },
            "pricing_provider":"River", "pricing_pair":fiat_currency.as_ref().map(|c| format!("BTC-{c}")), "pricing_method":"river_csv", "pricing_external_ref":reference,
            "pricing_quality":if cash { Some("exact") } else if value.is_some() { Some("provider_sample") } else { None },
            "kind":kind, "description":description, "counterparty":if direction == "outbound" { destination } else { source },
            "payment_hash":payment_hash, "payment_hash_source":payment_hash.as_ref().map(|_| "importer"),
            "_river_tag":tag,"_river_description":description, "raw_json":config::stored_json(&sanitized)
        }),
    }))
}

pub(super) fn metadata(
    conn: &Connection,
    scope: (&str, &str),
    wallet: &str,
    rows: &[csv::Row],
    ids: &mut Ids<'_>,
    result: &mut Vec<(&str, Py)>,
    invalidated: bool,
) -> Result<(), AppError> {
    let (profile, workspace) = scope;
    let mut notes = 0;
    let mut tags = 0;
    let mut created = 0;
    for row in rows {
        let row = &row.data;
        let external = text(row, "txid");
        if external.is_empty() {
            continue;
        }
        let existing:Option<(String,Option<String>)>=conn.query_row("SELECT id,note FROM transactions WHERE profile_id=? AND wallet_id=? AND external_id=? ORDER BY created_at DESC LIMIT 1",[profile,wallet,&external],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error)?;
        let Some((id, note)) = existing else { continue };
        if let Some(description) = optional(row, "_river_description")
            .filter(|_| note.as_ref().is_none_or(String::is_empty))
        {
            conn.execute(
                "UPDATE transactions SET note=? WHERE id=?",
                [description, id.clone()],
            )
            .map_err(database_error)?;
            notes += 1;
        }
        if let Some(kind) = optional(row, "_river_tag") {
            let code = kassiber_pytext::lower(&format!("river:{kind}")).replace(' ', "_");
            let tag: Option<String> = conn
                .query_row(
                    "SELECT id FROM tags WHERE profile_id=? AND code=?",
                    [profile, &code],
                    |r| r.get(0),
                )
                .optional()
                .map_err(database_error)?;
            let tag = match tag {
                Some(id) => id,
                None => {
                    let id = ids.next(conn)?;
                    conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)",params![id,workspace,profile,code,kind,ids.time(conn)?]).map_err(database_error)?;
                    created += 1;
                    id
                }
            };
            tags += conn
                .execute(
                    "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",
                    [id, tag],
                )
                .map_err(database_error)? as i64;
        }
    }
    result.extend([
        ("river_notes_set", Py::from(notes)),
        ("river_tags_added", Py::from(tags)),
        ("river_tags_created", Py::from(created)),
    ]);
    if !invalidated && (notes > 0 || tags > 0) {
        invalidate_journals(conn, profile)?;
        result
            .iter_mut()
            .find(|(k, _)| *k == "journal_invalidated")
            .unwrap()
            .1 = Py::Bool(true);
    }
    Ok(())
}
