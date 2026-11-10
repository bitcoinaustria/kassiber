//! Pure Bitcoin observation normalization. No transport, storage or tax facts.
//! Quantities stay integral; decimal BTC text exists only at Python's record
//! boundary. The graph and witness evidence remain observations of this source.
use bitcoin::hashes::{hash160, sha256, Hash};
use kassiber_envelope::AppError;
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{dumps, Options};
use serde_json::{json, Value};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, AppError>;
fn invalid() -> AppError {
    AppError::new("protocol_error", "Invalid Bitcoin record projection")
}
pub(crate) fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn normalized(v: &str) -> String {
    kassiber_pytext::strip(v).to_lowercase()
}
pub(crate) fn entries<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map_or(&[], Vec::as_slice)
}
pub(crate) fn script(v: &Value) -> &str {
    for key in ["scriptpubkey", "script_hex"] {
        if !v[key].is_null() {
            return text(&v[key]);
        }
    }
    let nested = &v["scriptPubKey"];
    if nested.is_string() {
        text(nested)
    } else if crate::descriptors::truthy(&nested["hex"]) {
        text(&nested["hex"])
    } else {
        text(&nested["scriptpubkey"])
    }
}
pub(crate) fn value_sats(v: &Value) -> Result<Option<i128>> {
    let explicit = !v["value_sats"].is_null();
    let v = if explicit {
        &v["value_sats"]
    } else {
        &v["value"]
    };
    if v.is_null() || v.is_boolean() || v.as_str() == Some("") {
        return Ok(None);
    }
    let integer = explicit
        || v.as_number()
            .is_some_and(|n| !n.to_string().contains(['.', 'e', 'E']))
        || v.as_str().is_some_and(|s| !s.contains('.'));
    if integer {
        return crate::scan::integer(v)
            .map(|n| i128::try_from(n).map_err(|_| invalid()))
            .transpose();
    }
    let raw = v
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string());
    let Ok(decimal) = kassiber_pytext::strip(&raw).parse::<Decimal>() else {
        return Ok(None);
    };
    // Shared finite-decimal arithmetic reproduces Python's BTC-valued legacy
    // graph fields, including half-even rounding to whole sats.
    let sats = decimal
        .mul(&Decimal::from(100_000_000_u64), &Context::RP2)
        .map_err(|_| invalid())?;
    let integer = sats
        .to_fixed_places(0, Rounding::HalfEven)
        .map_err(|_| invalid())?;
    integer.parse().map(Some).map_err(|_| invalid())
}
fn add(a: i128, b: i128) -> Result<i128> {
    a.checked_add(b).ok_or_else(invalid)
}
fn sub(a: i128, b: i128) -> Result<i128> {
    a.checked_sub(b).ok_or_else(invalid)
}
/// CPython Decimal(integer_sats) / 100_000_000, with no float round trip.
pub(crate) fn btc(sats: i128) -> String {
    if sats == 0 {
        return "0".into();
    }
    let mut digits = sats.to_string();
    let mut exponent = -8_i32;
    while digits.ends_with('0') {
        digits.pop();
        exponent += 1;
    }
    if digits.len() as i32 + exponent - 1 < -6 {
        let tail = if digits.len() == 1 {
            String::new()
        } else {
            format!(".{}", &digits[1..])
        };
        return format!(
            "{}{tail}E{}",
            &digits[..1],
            digits.len() as i32 + exponent - 1
        );
    }
    if exponent >= 0 {
        return format!("{digits}{}", "0".repeat(exponent as usize));
    }
    let position = digits.len() as i32 + exponent;
    if position <= 0 {
        format!("0.{}{digits}", "0".repeat((-position) as usize))
    } else {
        format!(
            "{}.{}",
            &digits[..position as usize],
            &digits[position as usize..]
        )
    }
}
pub(crate) fn sorted_json(v: &Value) -> Result<String> {
    let py = kassiber_pytext::loads(&v.to_string()).map_err(|_| invalid())?;
    dumps(&py, &Options::new().sort_keys(true)).map_err(|_| invalid())
}
/// Scripts used by this graph; unresolved inputs preserve all given scripts.
pub fn graph_scoped_scripts(tx: &Value, scripts: &BTreeSet<String>) -> Vec<String> {
    let given: BTreeSet<_> = scripts
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| normalized(s))
        .collect();
    let mut found = BTreeSet::new();
    for input in entries(tx, "vin") {
        let value = script(&input["prevout"]);
        if value.is_empty() {
            return given.into_iter().collect();
        }
        found.insert(normalized(value));
    }
    for output in entries(tx, "vout") {
        let value = script(output);
        if !value.is_empty() {
            found.insert(normalized(value));
        }
    }
    given.intersection(&found).cloned().collect()
}
pub(crate) fn timestamp(value: &Value) -> Result<Option<String>> {
    if value.is_null() || value.as_str() == Some("") || value == 0 || value == "0" {
        return Ok(None);
    }
    let n = crate::scan::integer(value)
        .and_then(|n| i64::try_from(n).ok())
        .ok_or_else(invalid)?;
    // Gregorian civil date, from Unix days. Bounds match datetime's years.
    if !(-62_135_596_800..=253_402_300_799).contains(&n) {
        return Err(invalid());
    }
    let z = n.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let seconds = n.rem_euclid(86400);
    Ok(Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )))
}
pub(crate) const UNKNOWN: &str = "1970-01-01T00:00:00Z";

fn validate_graph(tx: &Value) -> Result<()> {
    if !tx.is_object()
        || ["vin", "vout"]
            .iter()
            .any(|key| tx.get(*key).is_some_and(|v| !v.is_array()))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Normalize an Esplora-shaped graph, also used by the native BDK observer.
pub fn from_esplora(
    tx: &Value,
    scripts: &BTreeSet<String>,
    backend: &str,
) -> Result<Option<Value>> {
    validate_graph(tx)?;
    if crate::descriptors::truthy(&tx["status"]) && !tx["status"].is_object() {
        return Err(invalid());
    }
    let block_time = &tx["status"]["block_time"];
    let occurred = if crate::descriptors::truthy(block_time) {
        block_time
    } else {
        &tx["observed_at"]
    };
    let confirmed = if tx["status"]["confirmed"] == true || !block_time.is_null() {
        timestamp(block_time)?
    } else {
        None
    };
    from_graph(
        tx,
        scripts,
        backend,
        &tx["txid"],
        timestamp(occurred)?.as_deref().unwrap_or(UNKNOWN),
        confirmed.as_deref(),
        tx.get("fee"),
    )
}
/// `tx` is the decoded, inline-prevout graph returned by acquisition. Its
/// existing Electrum graph marker and raw wire fields are preserved verbatim.
pub fn from_electrum(
    txid: &str,
    tx: &Value,
    occurred_at: &str,
    scripts: &BTreeSet<String>,
    backend: &str,
) -> Result<Option<Value>> {
    validate_graph(tx)?;
    let time = (occurred_at != UNKNOWN).then_some(occurred_at);
    let mut graph = tx.clone();
    // `_normalize_electrum_bitcoin_graph_for_storage` stamps the current graph
    // version whatever the input carried (1ec7e7da0).
    graph["_kassiber_electrum_graph"] = json!({"kind":"bitcoin_electrum","version":2});
    // Python json_ready turns the decoder's Decimal fields into floats before
    // storing the graph. Keep its raw JSON bytes (including .0) while the
    // normalized `value`/prevout sat fields and accounting math stay integral.
    if let Some(value) = graph["total_output_sats"].as_f64() {
        graph["total_output_sats"] = json!(value);
    }
    if let Some(outputs) = graph["vout"].as_array_mut() {
        for output in outputs {
            if let Some(value) = output["value_sats"].as_f64() {
                output["value_sats"] = json!(value);
            }
        }
    }
    graph["status"] = json!({"confirmed":time.is_some()});
    from_graph(
        &graph,
        scripts,
        backend,
        &json!(txid),
        occurred_at,
        time,
        None,
    )
}

pub(crate) fn from_graph(
    tx: &Value,
    scripts: &BTreeSet<String>,
    backend: &str,
    txid: &Value,
    occurred: &str,
    confirmed: Option<&str>,
    explicit_fee: Option<&Value>,
) -> Result<Option<Value>> {
    let tracked: BTreeSet<_> = scripts
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| normalized(s))
        .collect();
    let mut received = 0;
    let mut total_output = 0;
    for output in entries(tx, "vout").iter().filter(|v| v.is_object()) {
        let value = value_sats(output)?.unwrap_or(0);
        total_output = add(total_output, value)?;
        if tracked.contains(&normalized(script(output))) {
            received = add(received, value)?;
        }
    }
    let mut sent = 0;
    let mut total_input = 0;
    let mut inputs_owned = !entries(tx, "vin").is_empty();
    for input in entries(tx, "vin") {
        if !input.is_object() {
            inputs_owned = false;
            continue;
        }
        let value = match value_sats(&input["prevout"])? {
            Some(v) => Some(v),
            None => value_sats(input)?,
        };
        let owned = tracked.contains(&normalized(script(&input["prevout"])));
        inputs_owned &= owned && value.is_some();
        if let Some(value) = value {
            total_input = add(total_input, value)?;
            if owned {
                sent = add(sent, value)?;
            }
        }
    }
    if received == 0 && sent == 0 {
        return Ok(None);
    }
    let fee = match explicit_fee.filter(|v| !v.is_null()) {
        Some(v) => i128::try_from(crate::scan::integer(v).ok_or_else(invalid)?)
            .map_err(|_| invalid())?
            .max(0),
        None => sub(total_input, total_output)?.max(0),
    };
    let collaborative = sent > 0 && !inputs_owned;
    let (direction, amount, row_fee, kind) = if received > sent {
        ("inbound", sub(received, sent)?, 0, "deposit")
    } else {
        let gross = sub(sent, received)?;
        let amount = if collaborative {
            gross
        } else {
            sub(gross, fee)?.max(0)
        };
        (
            "outbound",
            amount,
            if collaborative { 0 } else { fee },
            if amount > 0 { "withdrawal" } else { "fee" },
        )
    };
    let mut raw = tx.clone();
    raw["fee"] = serde_json::from_str(&fee.to_string()).map_err(|_| invalid())?;
    raw["observer_owned_scripts"] = json!(graph_scoped_scripts(tx, &tracked));
    let mut boundary = privacy_boundary(tx)?;
    if collaborative {
        raw["component"] = json!({"fee_attribution":"implicit_wallet_delta"});
        if received != sent && boundary.is_none() {
            boundary = Some("collaborative".into());
        }
    }
    if let Some(boundary) = &boundary {
        raw["privacy_boundary"] = json!(boundary);
    }
    let mut record = json!({"txid":txid,"occurred_at":occurred,"confirmed_at":confirmed,"direction":direction,"asset":"BTC","amount":btc(amount),"fee":btc(row_fee),"fiat_rate":null,"fiat_value":null,"kind":kind,"description":format!("Synced from {backend}"),"counterparty":null,"raw_json":sorted_json(&raw)?});
    if let Some(boundary) = boundary {
        record["privacy_boundary"] = json!(boundary);
    }
    witness_evidence(tx, &mut record);
    Ok(Some(record))
}

pub(crate) fn privacy_boundary(tx: &Value) -> Result<Option<String>> {
    if let Some(boundary) = boundary_fields(tx)? {
        return Ok(Some(boundary));
    }
    if let Some(raw) = tx["raw_json"].as_object() {
        return boundary_fields(&Value::Object(raw.clone()));
    }
    if let Some(raw) = tx["raw_json"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_object)
    {
        return boundary_fields(&raw);
    }
    Ok(None)
}
fn boundary_fields(tx: &Value) -> Result<Option<String>> {
    for key in [
        "privacy_boundary",
        "privacy_boundary_kind",
        "privacyBoundary",
        "privacyBoundaryKind",
        "privacy_hop",
        "privacy_hop_kind",
        "privacyHop",
        "privacyHopKind",
    ] {
        if tx[key].is_null() {
            continue;
        }
        let value = crate::descriptors::text(&tx[key]);
        let value = kassiber_pytext::strip(&value);
        if value.is_empty() {
            continue;
        }
        let normalized = value.to_lowercase().replace('-', "_");
        if [
            "coinjoin",
            "collaborative",
            "payjoin",
            "payment_in_coinjoin",
            "sweep",
        ]
        .contains(&normalized.as_str())
        {
            return Ok(Some(normalized));
        }
        return Err(AppError::new("validation", format!("Unsupported privacy boundary '{value}'")).with_hint("Supported privacy boundaries: coinjoin, collaborative, payjoin, payment_in_coinjoin, sweep."));
    }
    for key in [
        "islikelycoinjoin",
        "is_likely_coinjoin",
        "isLikelyCoinJoin",
        "likely_coinjoin",
        "likelyCoinJoin",
    ] {
        if ["1", "true", "yes", "on"]
            .contains(&normalized(&crate::descriptors::text(&tx[key])).as_str())
        {
            return Ok(Some("coinjoin".into()));
        }
    }
    Ok(None)
}

fn hex(s: &str) -> Option<Vec<u8>> {
    // bytes.fromhex permits ASCII whitespace between whole bytes.
    let mut bytes = vec![];
    let mut high = None;
    for b in s.bytes() {
        if b.is_ascii_whitespace() && high.is_none() {
            continue;
        }
        let digit = (b as char).to_digit(16)? as u8;
        match high.take() {
            Some(h) => bytes.push(h * 16 + digit),
            None => high = Some(digit),
        }
    }
    high.is_none().then_some(bytes)
}
pub(crate) fn hashlock(script: &[u8]) -> Option<&[u8]> {
    let script = script
        .strip_prefix(&[0x82, 0x01, 0x20, 0x88])
        .unwrap_or(script);
    if script.len() < 62
        || script.get(..2)? != [0xa9, 20]
        || script.get(22..25)? != [0x87, 0x63, 33]
        || script[58] != 0x67
    {
        return None;
    }
    let cursor = 59;
    let head = *script.get(cursor)?;
    let push = match head {
        0 | 0x51..=0x60 => 1,
        1..=5 => 1 + head as usize,
        _ => return None,
    };
    let suffix = script.get(cursor + push..)?;
    if suffix.len() != 38 || suffix[..3] != [0xb1, 0x75, 33] || suffix[36..] != [0x68, 0xac] {
        return None;
    }
    Some(&script[2..22])
}
pub(crate) fn witness_evidence(tx: &Value, record: &mut Value) {
    let inputs = entries(tx, "vin");
    let mut refunds = vec![];
    for input in inputs {
        let witness = &input["witness"];
        let Some(witness) = witness.as_array().filter(|v| v.len() >= 2) else {
            continue;
        };
        // Python skips malformed items without promoting them to evidence.
        let items: Vec<_> = witness
            .iter()
            .filter_map(|v| v.as_str().and_then(hex))
            .collect();
        if items.len() < 2 {
            continue;
        }
        let Some(script) = items.last() else {
            continue;
        };
        let Some(hashlock) = hashlock(script) else {
            continue;
        };
        let preimage = items[..items.len() - 1].iter().find(|v| {
            v.len() == 32 && hash160::Hash::hash(v).as_byte_array().as_slice() == hashlock
        });
        let txid = text(&input["txid"]);
        let index = input["vout"].as_u64();
        if let Some(preimage) = preimage {
            let canonical = kassiber_pytext::strip(txid);
            if inputs.len() == 1
                && canonical.len() == 64
                && canonical.bytes().all(|b| b.is_ascii_hexdigit())
                && index.is_some()
            {
                record["payment_hash"] = json!(sha256::Hash::hash(preimage).to_string());
                record["payment_hash_source"] = json!("chain_script_unique_outpoint");
            }
        } else if let Some(index) = index.filter(|_| !txid.is_empty()) {
            let point = (txid.to_owned(), index);
            if !refunds.contains(&point) {
                refunds.push(point);
            }
        }
    }
    if let [(txid, index)] = refunds.as_slice() {
        record["swap_refund_funding_txid"] = json!(txid);
        record["swap_refund_funding_vout"] = json!(index);
    }
}

#[cfg(test)]
mod tests;
