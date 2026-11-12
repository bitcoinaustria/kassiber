//! ID-less CSV rows: economic identities numbered per identical occurrence,
//! from `importers.py`'s `_csv_economic_id`, `_number_synthetic_csv_occurrences`
//! and `synthetic_csv_identity`.
use super::*;

/// The marker a parser sets on a row whose ID it had to synthesize. Numbering
/// removes it from every row, as Python pops `_synthetic_csv_id`.
pub(super) const MARKER: &str = "_synthetic_csv_id";

/// One value of an economic tuple, as `canonical()` reads it.
pub(super) enum Part<'a> {
    /// `str(value or "")`.
    Text(Option<&'a str>),
    /// A `Decimal`: `format(value, "f")` without trailing fractional zeros.
    Number(&'a PriceNumber),
}

/// `_csv_economic_id(provider, *values)`: never depends on the file position.
pub(super) fn economic_id(provider: &str, parts: &[Part]) -> Result<String, AppError> {
    let mut canonical = Vec::with_capacity(parts.len());
    for part in parts {
        canonical.push(Py::Str(
            match part {
                Part::Text(text) => text.unwrap_or_default().to_owned(),
                Part::Number(PriceNumber::Finite(value)) => {
                    let text = value.to_fixed_string().map_err(decimal_error)?;
                    if text.contains('.') {
                        text.trim_end_matches('0').trim_end_matches('.').to_owned()
                    } else {
                        text
                    }
                }
                Part::Number(PriceNumber::Special(text)) => text.clone(),
            }
            .into(),
        ));
    }
    let options = kassiber_pyjson::Options {
        ensure_ascii: false,
        ..Default::default()
    };
    let payload = kassiber_pyjson::dumps(&Py::List(canonical), &options)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let digest: String = Sha256::digest(payload.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("{provider}:economic:{digest}"))
}

/// `_number_synthetic_csv_occurrences`: identical ID-less rows in one export
/// become `<economic id>:1`, `:2`, ... and stay stable across overlapping
/// exports. An economic pricing reference gets the same suffix.
pub(super) fn number_occurrences(mut rows: Vec<csv::Row>) -> Vec<csv::Row> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in &mut rows {
        let Some(data) = row.data.as_object_mut() else {
            continue;
        };
        if data.remove(MARKER) != Some(Json::Bool(true)) {
            continue;
        }
        let identity = data
            .get("txid")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned();
        let count = counts.entry(identity.clone()).or_default();
        *count += 1;
        let occurrence = *count;
        data.insert("txid".into(), json!(format!("{identity}:{occurrence}")));
        if let Some(reference) = data
            .get("pricing_external_ref")
            .and_then(Json::as_str)
            .filter(|r| r.contains(":economic:"))
            .map(str::to_owned)
        {
            data.insert(
                "pricing_external_ref".into(),
                json!(format!("{reference}:{occurrence}")),
            );
        }
    }
    rows
}

/// A stored raw row as the parser saw it: its keys in stored order.
fn stored_row(raw: &serde_json::Map<String, Json>) -> csv::Row {
    csv::Row {
        headers: raw.keys().cloned().collect(),
        data: Json::Object(raw.clone()),
        rest: None,
        original: None,
    }
}

/// `synthetic_csv_identity(external_id, raw_json)`: (economic key, ordinal,
/// legacy) only for recognized ID-less rows. Explicit provider IDs and
/// malformed or unrelated raw payloads never qualify. Legacy ordinals are
/// file positions; current ordinals count identical rows.
pub fn synthetic_csv_identity(external_id: &str, raw_json: &str) -> Option<(String, i64, bool)> {
    const PREFIXES: [&str; 5] = [
        "ledgerlive:",
        "coinfinity:coinfinity:",
        "pocketbitcoin:pocketbitcoin:",
        "binance-supplemental:",
        "binance:",
    ];
    let old_prefix = *PREFIXES.iter().find(|p| external_id.starts_with(**p))?;
    let Ok(Json::Object(raw)) = serde_json::from_str::<Json>(raw_json) else {
        return None;
    };
    let row = stored_row(&raw);
    let record = match old_prefix {
        "ledgerlive:" => ledgerlive::record(row, 0),
        "coinfinity:coinfinity:" => coinfinity::record(row, 0),
        "pocketbitcoin:pocketbitcoin:" => {
            let withdrawal = match raw.get("_pocketbitcoin_withdrawal") {
                Some(Json::Object(w)) => Some(stored_row(w)),
                _ => None,
            };
            pocket::record(&row, withdrawal.as_ref(), 0)
        }
        "binance-supplemental:" => binance::investment(&exchange::Columns::new(&row), 0),
        _ => binance::dividend(&exchange::Columns::new(&row), 0),
    }
    .ok()??;
    if record.data.get(MARKER) != Some(&Json::Bool(true)) {
        return None;
    }
    let economic = record.data.get("txid")?.as_str()?.to_owned();
    let current = external_id.starts_with(&format!("{economic}:"));
    let ordinal = if current {
        &external_id[economic.len() + 1..]
    } else if old_prefix == "binance:" {
        // `binance:<n>` is also the API importer's ID for a dividend keyed by
        // its epoch time. Only CSV cells, which are always text, qualify.
        if !raw.values().all(|v| v.is_null() || v.is_string()) {
            return None;
        }
        &external_id[old_prefix.len()..]
    } else {
        // Old IDs are `<prefix><raw date>:...:<row index>`; earlier parsers
        // spelled the amount differently, so only the date is matched.
        let occurred = match record.data.get("occurred_at") {
            Some(Json::String(text)) => text.clone(),
            _ => "None".to_owned(),
        };
        if !external_id.starts_with(&format!("{old_prefix}{occurred}:")) {
            return None;
        }
        external_id.rsplit(':').next()?
    };
    if ordinal.is_empty() || !ordinal.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ordinal: i64 = ordinal.parse().ok()?;
    (ordinal >= 1).then_some((economic, ordinal, !current))
}
