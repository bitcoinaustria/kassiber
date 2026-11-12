//! Read-only custody facts inside the caller's journal transaction.
//!
//! This private pipeline seam is deliberately separate from product routing.
//! SQL rows populate today's custody Snapshot directly. Shared column layouts
//! retain Python's insertion order exclusively for the test-only v1 byte gate.
mod ownership;
mod receipts;
mod revisions;
#[cfg(test)]
mod tests;
mod worklist;

use kassiber_pyjson::{Key, Options, Value};
use kassiber_store::rusqlite::{self, types::ValueRef, Connection, Params};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug)]
pub enum Error {
    /// Never start, commit, or repair a transaction on the caller's behalf.
    TransactionRequired,
    Database(rusqlite::Error),
    Unsupported(&'static str),
    Invalid(String),
    Json(kassiber_pyjson::Error),
    Reference(crate::Refusal),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // SQL, bound values and wallet material are private, even on failure.
        f.write_str(match self {
            Self::TransactionRequired => "custody loading requires the caller's write transaction",
            Self::Database(_) => "custody snapshot database read failed",
            Self::Unsupported(message) => message,
            Self::Invalid(_) => "custody snapshot contains unsupported facts",
            Self::Json(_) => "custody snapshot v1 serialization failed",
            Self::Reference(_) => "custody snapshot reference refused",
        })
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}
type Result<T> = std::result::Result<T, Error>;

fn app_refusal(error: kassiber_envelope::AppError) -> Error {
    let mut refusal = crate::Refusal::new("AppError", error.message);
    refusal.code = Some(error.code);
    refusal.hint = error.hint.map(String::from);
    refusal.details = error
        .details
        .as_ref()
        .map(kassiber_custody::codec::to_json)
        .unwrap_or(serde_json::Value::Null);
    refusal.retryable = Some(error.retryable);
    Error::Reference(refusal)
}

mod direct;
pub use direct::{load, load_read_snapshot, LoadedSnapshot};
pub use ownership::{owned_index, owned_index_and_wallets};

pub(super) fn convert(value: Value) -> Result<serde_json::Value> {
    use serde_json::Value as J;
    Ok(match value {
        Value::Null => J::Null,
        Value::Bool(value) => J::Bool(value),
        Value::Int(value) => J::Number(
            value
                .to_string()
                .parse()
                .map_err(|_| Error::Invalid("integer".into()))?,
        ),
        Value::Float(value) => {
            // The existing adapter keeps arbitrary-precision number
            // spellings, including Python's exponent padding. Overflowing
            // exponents retain SQLite infinities until a consumer validates
            // them; codec::from_json restores the original Python float.
            J::Number(
                float_storage(value)?
                    .parse()
                    .map_err(|_| Error::Invalid("float".into()))?,
            )
        }
        Value::Str(value) => J::String(
            value
                .as_str()
                .ok_or(Error::Unsupported(
                    "custody adapter cannot represent surrogate strings",
                ))?
                .into(),
        ),
        Value::Bytes(value) => kassiber_custody::codec::to_json(&Value::Bytes(value)),
        Value::List(values) => J::Array(values.into_iter().map(convert).collect::<Result<_>>()?),
        Value::Dict(values) => kassiber_custody::codec::json_dict(
            values
                .into_iter()
                .map(|(key, value)| {
                    let Key::Str(key) = key else {
                        return Err(Error::Invalid("key".into()));
                    };
                    Ok((
                        key.as_str()
                            .ok_or(Error::Unsupported(
                                "custody adapter cannot represent surrogate keys",
                            ))?
                            .into(),
                        convert(value)?,
                    ))
                })
                .collect::<Result<_>>()?,
        ),
        Value::Decimal(_) => return Err(Error::Invalid("unexpected Decimal".into())),
    })
}

fn float_storage(value: f64) -> Result<String> {
    if value.is_nan() {
        // SQLite stores a bound NaN as NULL. No SQL REAL can take this path.
        return Err(Error::Invalid("SQLite REAL is NaN".into()));
    }
    Ok(if value == f64::INFINITY {
        "1e999".into()
    } else if value == f64::NEG_INFINITY {
        "-1e999".into()
    } else {
        kassiber_pyjson::float_repr(value)
    })
}

fn dict<const N: usize>(values: [(&str, Value); N]) -> Value {
    Value::Dict(
        values
            .into_iter()
            .map(|(key, value)| (Key::from(key), value))
            .collect(),
    )
}
fn list(values: Vec<Value>) -> Value {
    Value::List(values)
}
fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    if let Value::Dict(values) = value {
        if let Some((_, value)) = values
            .iter()
            .find(|(k, _)| matches!(k,Key::Str(k) if k.is(key)))
        {
            return value;
        }
    }
    &Value::Null
}
fn set(value: &mut Value, key: &str, item: Value) {
    let Value::Dict(values) = value else {
        unreachable!("loader record");
    };
    if let Some((_, value)) = values
        .iter_mut()
        .find(|(k, _)| matches!(k,Key::Str(k) if k.is(key)))
    {
        *value = item;
    } else {
        values.push((Key::from(key), item));
    }
}
fn text(value: &Value) -> String {
    kassiber_envelope::py_str(value)
}
fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Int(value) => *value != 0.into(),
        Value::Float(value) => *value != 0.0,
        Value::Str(value) => !value.is(""),
        Value::Bytes(value) => !value.is_empty(),
        Value::List(value) => !value.is_empty(),
        Value::Dict(value) => !value.is_empty(),
        Value::Decimal(value) => !value.is_zero(),
    }
}
fn parsed(value: &Value) -> Value {
    match value {
        Value::Str(value) => value
            .as_str()
            .and_then(|s| kassiber_pytext::loads(s).ok())
            .unwrap_or(Value::Null),
        Value::Bytes(bytes) => kassiber_pyjson::json_bytes_string(bytes)
            .ok()
            .and_then(|text| kassiber_pytext::loads_py(&text).ok())
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}
fn sql_value(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::from(value),
        ValueRef::Real(value) => Value::Float(value),
        ValueRef::Text(value) => Value::from(
            std::str::from_utf8(value)
                .map_err(|_| Error::Unsupported("SQLite TEXT is not UTF-8"))?,
        ),
        ValueRef::Blob(value) => Value::Bytes(value.to_vec()),
    })
}

fn sqlite_text<'a>(bytes: &'a [u8], column: &str) -> Result<&'a str> {
    std::str::from_utf8(bytes).map_err(|_| {
        let c_string = |s: &[u8]| {
            let s = &s[..s.len().min(198)];
            s.split(|b| *b == 0).next().unwrap_or_default().to_vec()
        };
        let mut message = b"Could not decode to UTF-8 column '".to_vec();
        message.extend(c_string(column.as_bytes()));
        message.extend(b"' with text '");
        message.extend(c_string(bytes));
        message.push(b'\'');
        let message: String = message
            .into_iter()
            .take(198)
            .map(|b| {
                if b.is_ascii() {
                    char::from(b)
                } else {
                    '\u{fffd}'
                }
            })
            .collect();
        Error::Reference(crate::Refusal::new("OperationalError", message))
    })
}

fn row_error(row: &rusqlite::Row<'_>, error: Error) -> Error {
    for index in 0..row.as_ref().column_count() {
        if let Ok(ValueRef::Text(bytes)) = row.get_ref(index) {
            if let Err(error) = sqlite_text(bytes, row.as_ref().column_name(index).unwrap_or("")) {
                return error;
            }
        }
    }
    error
}

fn query(conn: &Connection, sql: &str, parameters: impl Params) -> Result<Vec<Value>> {
    let mut statement = conn.prepare_cached(sql)?;
    let mut seen = BTreeSet::new();
    let columns: Vec<_> = statement
        .column_names()
        .into_iter()
        .enumerate()
        .filter(|(_, name)| seen.insert(*name))
        .map(|(index, name)| (index, Key::from(name)))
        .collect();
    let discarded: Vec<_> = (0..statement.column_count())
        .filter(|index| !columns.iter().any(|(selected, _)| selected == index))
        .collect();
    let mut cursor = statement.query(parameters)?;
    let mut values = vec![];
    while let Some(row) = cursor.next()? {
        let record = (|| -> Result<Value> {
            for index in &discarded {
                if let ValueRef::Text(bytes) = row.get_ref(*index)? {
                    sql_value(ValueRef::Text(bytes))?;
                }
            }
            // sqlite3.Row's name lookup uses the first duplicate column. Keep its
            // insertion position, just as dict(row) does.
            let mut record = Vec::with_capacity(columns.len());
            for (index, key) in &columns {
                record.push((key.clone(), sql_value(row.get_ref(*index)?)?));
            }
            Ok(Value::Dict(record))
        })()
        .map_err(|error| row_error(row, error))?;
        values.push(record);
    }
    Ok(values)
}
fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(!query(
        conn,
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?",
        [name],
    )?
    .is_empty())
}
fn operational(error: &Error) -> bool {
    matches!(error, Error::Database(rusqlite::Error::SqliteFailure(code,_) | rusqlite::Error::SqlInputError{error:code,..}) if code.code == rusqlite::ErrorCode::Unknown)
        || matches!(error, Error::Reference(refusal) if refusal.family == "OperationalError")
}

fn safe_row(mut row: Value, configs: &mut HashMap<String, (String, bool)>) -> Result<Value> {
    for key in ["config_json", "wallet_config_json"] {
        if matches!(&row,Value::Dict(values) if values.iter().any(|(k,_)| matches!(k,Key::Str(k) if k.is(key))))
        {
            let raw = get(&row, key);
            let cache_key = if matches!(raw, Value::Bytes(_)) {
                // config_facts treats every bytes value as a non-mapping.
                // This key cannot collide with a serialized JSON value.
                "BLOB".into()
            } else {
                kassiber_pyjson::dumps(raw, &Options::new()).map_err(Error::Json)?
            };
            let (normalized, mentions) = if let Some(value) = configs.get(&cache_key) {
                value.clone()
            } else {
                let value = match raw {
                    Value::Str(_) => parsed(raw),
                    _ => raw.clone(),
                };
                let mapping = matches!(&value, Value::Dict(_));
                let mut facts = if let Value::Dict(values) = value {
                    Value::Dict(values.into_iter().filter(|(key,_)| matches!(key,Key::Str(key) if ["chain","network","samourai","chain_instance_id","bitcoin_network","chain_network"].iter().any(|name|key.is(name)))).collect())
                } else {
                    dict([])
                };
                let block = get(&facts, "samourai");
                if !matches!(block, Value::Null)
                    || matches!(&facts,Value::Dict(v) if v.iter().any(|(k,_)|matches!(k,Key::Str(k) if k.is("samourai"))))
                {
                    let sanitized = if let Value::Dict(_) = block {
                        Value::Dict(
                            ["role", "group_id", "section"]
                                .into_iter()
                                .filter_map(|key| {
                                    let Value::Dict(values) = block else {
                                        unreachable!()
                                    };
                                    values
                                        .iter()
                                        .find(|(k, _)| matches!(k,Key::Str(k) if k.is(key)))
                                        .map(|(_, value)| (Key::from(key), value.clone()))
                                })
                                .collect(),
                        )
                    } else {
                        Value::Null
                    };
                    set(&mut facts, "samourai", sanitized);
                }
                let normalized =
                    kassiber_pyjson::dumps(&facts, &Options::new().ensure_ascii(false))
                        .map_err(Error::Json)?;
                let result = (
                    normalized,
                    mapping && text(raw).to_lowercase().contains("samourai"),
                );
                configs.insert(cache_key, result.clone());
                result
            };
            set(&mut row, key, Value::from(normalized));
            if key == "config_json" {
                set(
                    &mut row,
                    "config_text_mentions_samourai",
                    Value::Bool(mentions),
                );
            }
        }
    }
    if let Value::Dict(values) = &mut row {
        values.retain(
            |(key, _)| !matches!(key, Key::Str(key) if key.is("_closed_confirmation_observation")),
        );
    }
    Ok(row)
}
fn safe_rows(
    rows: Vec<Value>,
    configs: &mut HashMap<String, (String, bool)>,
) -> Result<Vec<Value>> {
    rows.into_iter().map(|row| safe_row(row, configs)).collect()
}

/// Custody composition reads transactions in the source order
/// (`kassiber_store::transaction_order`), as Python's `_transactions` does.
static TRANSACTIONS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    transactions_sql(&kassiber_store::transaction_order::transaction_order_sql(
        "t", false,
    ))
});
/// A narrow legacy table without the source order's columns keeps the
/// legacy order, as Python's `_transactions` falls back.
static LEGACY_TRANSACTIONS: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| transactions_sql("t.occurred_at ASC, t.created_at ASC, t.id ASC"));
fn transactions_sql(order: &str) -> String {
    format!("SELECT t.*,w.label AS wallet_label,w.kind AS wallet_kind,w.account_id AS wallet_account_id,w.config_json AS config_json,observation.authority_version AS observation_authority_version,observation.observer_kinds_json AS observation_observer_kinds_json,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash,observation.fee_attribution AS observation_fee_attribution,observation.application_revision AS observation_application_revision,observation.observed_at AS observation_observed_at,COALESCE(a.code,'treasury') AS account_code,COALESCE(a.label,'Treasury') AS account_label FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN accounts a ON a.id=w.account_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.profile_id=? AND t.excluded=0 ORDER BY {order}")
}
/// Python's `"no such column"`/`"no such table"` check on the fallback.
fn missing_column(error: &Error) -> bool {
    let message = match error {
        Error::Database(rusqlite::Error::SqlInputError { msg, .. })
        | Error::Database(rusqlite::Error::SqliteFailure(_, Some(msg))) => msg.as_str(),
        Error::Reference(refusal) => refusal.message.as_str(),
        _ => "",
    };
    operational(error) && (message.contains("no such column") || message.contains("no such table"))
}
/// `transaction_rate_rows`: latest first, a same-time group in reverse source
/// order, without prices stored in another currency than the book's.
static RATE_ROWS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| rate_rows_sql(0, true));

/// The `_latest_first_rows` attempts in Python's order: the source order,
/// the legacy order, then without the exact prices (`shape` 0 to 2), each
/// with (`book_currency`) and then without the book-currency filter.
pub(crate) fn rate_rows_sql(shape: usize, book_currency: bool) -> String {
    let columns = if shape == 2 {
        "asset,fiat_rate,fiat_value,amount"
    } else {
        "asset,fiat_rate,fiat_value,fiat_rate_exact,fiat_value_exact,amount"
    };
    let filter = if book_currency {
        format!(
            " AND {}",
            kassiber_store::price_currency::price_in_book_currency_sql("transactions")
        )
    } else {
        String::new()
    };
    let order = if shape == 0 {
        format!(
            "occurred_at DESC, {}",
            kassiber_store::transaction_order::same_time_order_sql("transactions", true)
        )
    } else {
        "occurred_at DESC, created_at DESC, id DESC".into()
    };
    format!("SELECT {columns} FROM transactions WHERE profile_id=? AND excluded=0{filter} ORDER BY {order}")
}

/// Dominant snapshot read floor: step the same SQLite VM without decoding rows.
/// Ordered as transactions, rates, network observations, gap worklist.
pub fn floor_queries() -> [&'static str; 4] {
    [TRANSACTIONS.as_str(),
     RATE_ROWS.as_str(),
     "SELECT t.id,t.wallet_id,t.asset,t.raw_json,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? ORDER BY t.id",
     "SELECT t.id,t.profile_id,t.wallet_id,w.label,w.kind,t.occurred_at,t.direction,t.asset,t.amount,t.fee,t.amount_includes_fee,t.excluded,t.kind,t.kind_override,t.privacy_boundary,t.external_id,t.raw_json,w.config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at,t.created_at,t.id"]
}
