//! Generic JSON is pass-through input, using CPython's decoder and coercions.
//! Keep the original Python-shaped row until normalization: JSON numbers are
//! Python floats/ints, and raw evidence must retain their Python spelling.
use super::*;
use crate::unicode_binding_error as unicode_error;
use kassiber_envelope::{get, truthy as py_truthy, type_name};
use kassiber_pyjson::{Key, Options};

// Interpretation must never erase a surrogate and accidentally turn an
// invalid identifier/quantity into a valid one. Original evidence keeps the
// code points; text that SQLite would bind is checked separately below.
fn py_str(value: &Py) -> String {
    match value {
        Py::Bytes(bytes) => kassiber_pyjson::bytes_repr(bytes),
        Py::Str(s) => s
            .code_points()
            .map(|p| char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
        other => kassiber_envelope::py_str(other),
    }
}
pub(super) fn load(file: &str) -> Result<Vec<csv::Row>, AppError> {
    let mut payload = csv::read_json(file, "JSON import", true)?;
    if matches!(payload, Py::Dict(_)) {
        payload = get(&payload, "transactions")
            .cloned()
            .unwrap_or(Py::List(vec![]));
    }
    let Py::List(records) = payload else {
        return Err(AppError::new(
            "app_error",
            "JSON import must be a list of transaction objects",
        ));
    };
    records
        .into_iter()
        .map(|record| {
            Ok(csv::Row {
                headers: vec![],
                data: projection(&record),
                rest: None,
                original: Some(record),
            })
        })
        .collect()
}

/// A projection for evidence interpretation only. The serialized original is
/// retained independently; non-finite floats never change the evidence bytes.
pub(super) fn projection(value: &Py) -> Json {
    match value {
        Py::Bytes(bytes) => json!(kassiber_pyjson::bytes_repr(bytes)),
        Py::Null => Json::Null,
        Py::Bool(b) => json!(b),
        Py::Str(_) => json!(py_str(value)),
        Py::Int(n) => serde_json::from_str(&n.to_string()).unwrap(),
        Py::Float(n) => {
            serde_json::from_str(&kassiber_pyjson::float_repr(*n)).unwrap_or(Json::Null)
        }
        Py::Decimal(d) => json!(d.to_string()),
        Py::List(items) => Json::Array(items.iter().map(projection).collect()),
        Py::Dict(items) => Json::Object(
            items
                .iter()
                .filter_map(|(k, v)| match k {
                    Key::Str(k) => Some((py_str(&Py::Str(k.clone())), projection(v))),
                    _ => None,
                })
                .collect(),
        ),
    }
}
fn dump(value: &Py) -> Result<String, AppError> {
    kassiber_pyjson::dumps(
        value,
        &Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))
}
pub(super) fn raw(value: &Py) -> Result<(String, Json), AppError> {
    let raw = match value {
        Py::Str(s) => s
            .code_points()
            .map(|p| char::from_u32(p).map_or_else(|| format!("\\u{p:04x}"), |c| c.to_string()))
            .collect::<String>(),
        other => dump(other)?,
    };
    let parsed = kassiber_pytext::loads(&raw);
    if !matches!(parsed, Ok(Py::Dict(_))) {
        if let Py::Str(s) = value {
            if s.has_surrogates() {
                return Err(unicode_error(s));
            }
        }
    }
    let Ok(mut parsed) = parsed else {
        return Ok((raw, Json::Null));
    };
    let serialized = if let Py::Dict(items) = &mut parsed {
        items.retain(
            |(key, _)| !matches!(key, Key::Str(s) if s.as_str() == Some("_kassiber_provenance")),
        );
        dump(&parsed)?
    } else {
        raw
    };
    Ok((serialized, projection(&parsed)))
}

pub(super) fn prepare(record: &Py) -> Result<csv::Row, AppError> {
    let mut data = projection(record);
    let Py::Dict(fields) = record else {
        return Err(AppError::new(
            "internal_error",
            format!("'{}' object has no attribute 'get'", type_name(record)),
        ));
    };
    // Python's dec(), str_or_none(), and timestamps use str(value), never
    // JSON serialization. Keep native types for metadata binding below.
    for (key, value) in fields {
        let Key::Str(key) = key else { continue };
        let Some(key) = key.as_str() else { continue };
        if !matches!(key, "raw_json" | "kind" | "description" | "counterparty") {
            data[key] = if *value == Py::Null {
                Json::Null
            } else {
                json!(py_str(value))
            };
        }
    }
    for key in [
        "direction",
        "txid",
        "id",
        "asset",
        "fiat_currency",
        "external_id_kind",
        "pricing_source_kind",
        "fiat_price_source_kind",
        "pricing_quality",
        "_match_existing_ignore_fee",
    ] {
        if get(record, key).is_some_and(|v| !py_truthy(v)) {
            data[key] = json!("");
        }
    }
    for (key, alias) in [
        ("pricing_provider", "provider"),
        ("pricing_pair", "pair"),
        ("pricing_granularity", "granularity"),
        ("pricing_method", "method"),
        ("pricing_external_ref", "external_ref"),
    ] {
        let value = get(record, key)
            .filter(|v| py_truthy(v))
            .or_else(|| get(record, alias));
        data[key] = value
            .filter(|v| **v != Py::Null)
            .map_or(Json::Null, |v| json!(py_str(v)));
        data.as_object_mut().unwrap().remove(alias);
    }
    data["amount_includes_fee"] = json!(
        if get(record, "amount_includes_fee").is_some_and(py_truthy) {
            "1"
        } else {
            ""
        }
    );
    let occurred = ["occurred_at", "timestamp", "date"]
        .into_iter()
        .find_map(|k| get(record, k).filter(|v| py_truthy(v)));
    data["occurred_at"] = occurred.map_or(Json::Null, |v| json!(py_str(v)));
    data.as_object_mut().unwrap().remove("timestamp");
    data.as_object_mut().unwrap().remove("date");
    for key in ["confirmed_at", "pricing_timestamp", "pricing_fetched_at"] {
        if get(record, key).is_some_and(|v| {
            *v != Py::Null && !matches!(v, Py::Str(s) if s.as_str()==Some("")) && !py_truthy(v)
        }) {
            // A present zero/bool timestamp reaches parse_timestamp, which
            // refuses it as missing. Preserve that distinction from absence.
            data[key] = Json::Bool(false);
        }
    }
    let value = get(record, "raw_json")
        .filter(|v| **v != Py::Null)
        .unwrap_or(record);
    // Invalid opaque text is a binding refusal, not a parsing refusal: an
    // unchanged repeat may never write it. The placeholder is never persisted.
    data["raw_json"] = json!(raw(value).map(|v| v.0).unwrap_or_else(|_| py_str(value)));
    if let Some(v) = get(record, "swap_refund_funding_vout") {
        data["swap_refund_funding_vout"] = match v {
            Py::Bool(b) => json!(i32::from(*b).to_string()),
            Py::Float(f) if f.is_finite() => json!(format!("{:.0}", f.trunc())),
            _ => data["swap_refund_funding_vout"].clone(),
        };
    }
    Ok(csv::Row {
        headers: vec![],
        data,
        rest: None,
        original: None,
    })
}

pub(super) fn metadata(record: &Py, key: &str) -> Result<Sql, AppError> {
    match get(record, key) {
        None | Some(Py::Null) => Ok(Sql::Null),
        Some(Py::Bool(b)) => Ok(Sql::Integer(i64::from(*b))),
        Some(Py::Int(n)) => n.to_string().parse().map(Sql::Integer).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        }),
        Some(Py::Float(f)) if f.is_nan() => Ok(Sql::Null),
        Some(Py::Float(f)) => Ok(Sql::Real(*f)),
        Some(Py::Str(s)) if s.has_surrogates() => Err(unicode_error(s)),
        Some(Py::Str(s)) => Ok(Sql::Text(s.as_str().unwrap().into())),
        Some(v) => Err(AppError::new(
            "internal_error",
            format!(
                "Error binding parameter {}: type '{}' is not supported",
                match key {
                    "kind" => 30,
                    "description" => 32,
                    _ => 33,
                },
                type_name(v)
            ),
        )),
    }
}

pub(super) fn text_bindings(record: &Py, fields: &Fields) -> BTreeMap<String, AppError> {
    let mut errors = BTreeMap::new();
    let primary = |key: &str, alias: &str| {
        get(record, key)
            .filter(|v| py_truthy(v))
            .or_else(|| get(record, alias))
    };
    for (key, value) in [
        ("external_id", primary("txid", "id")),
        ("asset", get(record, "asset")),
        ("fiat_currency", get(record, "fiat_currency")),
        ("pricing_provider", primary("pricing_provider", "provider")),
        ("pricing_pair", primary("pricing_pair", "pair")),
        (
            "pricing_granularity",
            primary("pricing_granularity", "granularity"),
        ),
        ("pricing_method", primary("pricing_method", "method")),
        (
            "pricing_external_ref",
            primary("pricing_external_ref", "external_ref"),
        ),
        (
            "pricing_source_kind",
            primary("pricing_source_kind", "fiat_price_source_kind"),
        ),
        ("pricing_quality", get(record, "pricing_quality")),
        ("payment_hash_source", get(record, "payment_hash_source")),
    ] {
        if fields.get(key).is_some_and(|v| *v != Sql::Null) {
            if let Some(Py::Str(s)) = value {
                if s.has_surrogates() {
                    errors.insert(key.into(), unicode_error(s));
                }
            }
        }
    }
    let value = get(record, "raw_json")
        .filter(|v| **v != Py::Null)
        .unwrap_or(record);
    if let Err(error) = raw(value) {
        errors.insert("raw_json".into(), error);
    }
    errors
}
