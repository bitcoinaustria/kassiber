//! `core/wallet_completeness.py`: which kind of source a wallet's history came
//! from (`completeness_class`) and the observable edges of that history
//! (`scan_bounds`). Both read only the kind, the presence of source config,
//! the sync time and transaction rows, never a descriptor, address or path.
//! `wallets list` and `ui.wallets.list` share [`describe`].
use crate::publication::{balance, row};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::Connection;
use std::collections::BTreeMap;

const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";
const DESCRIPTOR_KINDS: [&str; 4] = ["descriptor", "xpub", "silent-payment", "samourai"];
const LIGHTNING_NODE_KINDS: [&str; 2] = ["coreln", "lnd"];
const EXCHANGE_API_KINDS: [&str; 3] = ["kraken", "coinbase", "binance"];
const CUSTODIAL_EXPORT_KINDS: [&str; 9] = [
    "river",
    "bullbitcoin",
    "coinfinity",
    "21bitcoin",
    "pocketbitcoin",
    "strike",
    "kraken",
    "coinbase",
    "binance",
];
/// Classes Kassiber scans itself, so a missing sync time means nothing was
/// observed yet.
const LIVE_CLASSES: [&str; 4] = [
    "descriptor-derived",
    "address-list",
    "lightning-node",
    "provider-api",
];

/// `str_or_none(value)`: `str(value).strip()` or `None` when blank.
fn str_or_none(value: &Value) -> Option<String> {
    if *value == Value::Null {
        return None;
    }
    let text = kassiber_pytext::strip(&py_str(value)).to_owned();
    (!text.is_empty()).then_some(text)
}

fn field<'a>(config: &'a Value, key: &str) -> &'a Value {
    get(config, key).unwrap_or(&Value::Null)
}

/// `_config(config_json)`: `json.loads(value or "{}")`, `{}` unless an object.
fn config(raw: &Value) -> Result<Value, AppError> {
    let text = if truthy(raw) {
        py_str(raw)
    } else {
        "{}".to_owned()
    };
    match kassiber_pytext::loads(&text) {
        Ok(parsed @ Value::Dict(_)) => Ok(parsed),
        Ok(_) => Ok(dict::<&str>([])),
        Err(error) if error.is_decode_error() => Ok(dict::<&str>([])),
        Err(error) => match error.kind {
            kassiber_pytext::JsonErrorKind::IntTooLarge { .. } => Ok(dict::<&str>([])),
            _ => Err(AppError::new("internal_error", error.to_string())),
        },
    }
}

/// `parse_bool(value, default=False)`.
fn parse_bool(value: &Value) -> Result<bool, AppError> {
    if *value == Value::Null {
        return Ok(false);
    }
    match kassiber_pytext::lower(kassiber_pytext::strip(&py_str(value))).as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::new(
            "app_error",
            format!("Invalid boolean value: {}", py_str(value)),
        )),
    }
}

/// Whether `normalize_addresses(value)` keeps any address.
fn has_addresses(value: &Value) -> Result<bool, AppError> {
    let kept = |text: String| !kassiber_pytext::strip(&text).is_empty();
    Ok(match value {
        Value::Null => false,
        Value::Str(_) => py_str(value).split(',').any(|part| kept(part.to_owned())),
        Value::List(items) => items.iter().any(|item| kept(py_str(item))),
        Value::Dict(pairs) => pairs.iter().any(|(key, _)| match key {
            Key::Str(text) => kept(py_str(&Value::Str(text.clone()))),
            _ => true,
        }),
        other => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "'{}' object is not iterable",
                    kassiber_envelope::type_name(other)
                ),
            ))
        }
    })
}

fn kind_text(kind: &Value) -> String {
    let kind = if truthy(kind) {
        py_str(kind)
    } else {
        String::new()
    };
    kassiber_pytext::lower(kassiber_pytext::strip(&kind))
}

/// `completeness_class(kind, config)`.
fn class(kind: &str, config: &Value) -> &'static str {
    let has_file = str_or_none(field(config, "source_file")).is_some();
    let source_format = str_or_none(field(config, "source_format"));
    if kind == "untracked" {
        return "untracked";
    }
    if str_or_none(field(config, "sync_source"))
        .is_some_and(|source| kassiber_pytext::lower(kassiber_pytext::strip(&source)) == "btcpay")
    {
        return "provider-api";
    }
    if DESCRIPTOR_KINDS.contains(&kind) {
        let descriptor = str_or_none(field(config, "descriptor")).is_some()
            || (str_or_none(field(config, "xpub")).is_some()
                && truthy(field(config, "script_types")));
        let silent = str_or_none(field(config, "sp_descriptor")).is_some();
        if kind == "samourai" || descriptor || silent || !has_file {
            return "descriptor-derived";
        }
        return "file-import";
    }
    if kind == "address" {
        return "address-list";
    }
    let backend = str_or_none(field(config, "backend")).is_some();
    if LIGHTNING_NODE_KINDS.contains(&kind) && backend {
        return "lightning-node";
    }
    if EXCHANGE_API_KINDS.contains(&kind) && backend && !has_file {
        return "provider-api";
    }
    if CUSTODIAL_EXPORT_KINDS.contains(&kind) {
        return "exchange-export";
    }
    if kind == "custom"
        && source_format
            .as_deref()
            .is_none_or(|f| f == "generic_ledger")
    {
        return "manual";
    }
    "file-import"
}

/// `_declared_scan_start(kind, config)`.
fn declared_start(kind: &str, config: &Value) -> Result<(Value, Value), AppError> {
    if kind == "silent-payment" && !parse_bool(field(config, "sp_full_history"))? {
        let date = str_or_none(field(config, "sp_scan_start_date"));
        let height = match field(config, "sp_scan_start_height") {
            height @ Value::Int(_) => height.clone(),
            _ => Value::Null,
        };
        if date.is_some() || height != Value::Null {
            return Ok((date.map_or(Value::Null, s), height));
        }
    }
    Ok((
        str_or_none(field(config, "birthday")).map_or(Value::Null, s),
        Value::Null,
    ))
}

/// `scan_bounds(kind, config, rows, klass=...)`.
fn scan_bounds(kind: &str, config: &Value, rows: &[Value], klass: &str) -> Result<Value, AppError> {
    let mut dated: Vec<String> = rows
        .iter()
        .map(|row| row::field(row, "occurred_at"))
        .filter(|at| !row::blank(at) && **at != Value::from(UNKNOWN_OCCURRED_AT))
        .map(py_str)
        .collect();
    dated.sort();
    let undated = rows.len() - dated.len();
    let last_synced_at = str_or_none(field(config, "last_synced_at"));
    let (declared_at, declared_height) = declared_start(kind, config)?;
    let mut warnings = Vec::new();
    if ["address-list", "untracked"].contains(&klass) {
        warnings.push("incomplete_by_construction");
    }
    let live = LIVE_CLASSES.contains(&klass)
        && !(klass == "address-list" && !has_addresses(field(config, "addresses"))?);
    if live && last_synced_at.is_none() {
        warnings.push("never_synced");
    }
    if rows.is_empty() {
        warnings.push("no_transactions");
    }
    if undated > 0 {
        warnings.push("undated_transactions");
    }
    let deficits = if rows.is_empty() {
        vec![]
    } else {
        balance::events(rows)?
    };
    if !deficits.is_empty() {
        warnings.push("history_starts_after_first_funding");
    }
    let history = if !live || last_synced_at.is_none() {
        "unknown"
    } else if declared_at != Value::Null || declared_height != Value::Null {
        "declared_start"
    } else {
        "scanned"
    };
    let assets: std::collections::BTreeSet<String> = deficits
        .iter()
        .map(|item| py_str(row::field(item, "asset")))
        .collect();
    let text = |value: Option<&String>| value.map_or(Value::Null, |v| s(v.as_str()));
    Ok(dict([
        ("first_transaction_at", text(dated.first())),
        ("last_transaction_at", text(dated.last())),
        ("undated_transactions", Value::Int(undated.into())),
        ("last_synced_at", text(last_synced_at.as_ref())),
        ("declared_scan_start_at", declared_at),
        ("declared_scan_start_height", declared_height),
        ("history_before_first_transaction", s(history)),
        (
            "deficit_assets",
            Value::List(assets.into_iter().map(s).collect()),
        ),
        (
            "warnings",
            Value::List(warnings.into_iter().map(s).collect()),
        ),
    ]))
}

/// One wallet to describe: its id, `kind` and stored `config_json`.
pub struct Wallet {
    pub id: String,
    pub kind: Value,
    pub config_json: Value,
}

/// `describe_wallets`: `{wallet_id: (completeness_class, scan_bounds)}` with
/// one transaction query for the whole list, in the caller's wallet order.
pub fn describe(
    conn: &Connection,
    profile_id: &str,
    wallets: &[Wallet],
) -> Result<BTreeMap<String, (Value, Value)>, AppError> {
    let mut rows_by_wallet: BTreeMap<String, Vec<Value>> = wallets
        .iter()
        .map(|wallet| (wallet.id.clone(), Vec::new()))
        .collect();
    if !wallets.is_empty() {
        // raw_json only orders unconfirmed chain rows; skip it elsewhere.
        for found in row::rows(
            conn,
            "SELECT wallet_id, id, external_id, occurred_at, confirmed_at,
                    CASE WHEN confirmed_at IS NULL THEN raw_json END AS raw_json,
                    direction, asset, amount, fee, created_at
             FROM transactions
             WHERE profile_id = ? AND excluded = 0",
            [profile_id],
        )? {
            let wallet = py_str(row::field(&found, "wallet_id"));
            if let Some(bucket) = rows_by_wallet.get_mut(&wallet) {
                bucket.push(found);
            }
        }
    }
    let mut described = BTreeMap::new();
    for wallet in wallets {
        let config = config(&wallet.config_json)?;
        let kind = kind_text(&wallet.kind);
        let klass = class(&kind, &config);
        let bounds = scan_bounds(&kind, &config, &rows_by_wallet[&wallet.id], klass)?;
        described.insert(wallet.id.clone(), (s(klass), bounds));
    }
    Ok(described)
}

/// `annotate_wallet_rows`: `completeness_class` and `scan_bounds` appended
/// to each listed wallet row (a dict with an `id`).
pub fn annotate(
    conn: &Connection,
    profile_id: &str,
    rows: Vec<Value>,
) -> Result<Vec<Value>, AppError> {
    let stored = row::rows(
        conn,
        "SELECT id, kind, config_json FROM wallets WHERE profile_id = ?",
        [profile_id],
    )?;
    let wallets: Vec<Wallet> = stored
        .iter()
        .map(|wallet| Wallet {
            id: py_str(row::field(wallet, "id")),
            kind: row::field(wallet, "kind").clone(),
            config_json: row::field(wallet, "config_json").clone(),
        })
        .collect();
    let described = describe(conn, profile_id, &wallets)?;
    Ok(rows
        .into_iter()
        .map(|mut wallet| {
            let id = py_str(row::field(&wallet, "id"));
            if let Some((klass, bounds)) = described.get(&id) {
                kassiber_envelope::set(&mut wallet, "completeness_class", klass.clone());
                kassiber_envelope::set(&mut wallet, "scan_bounds", bounds.clone());
            }
            wallet
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(text: &str) -> Value {
        config(&Value::from(text)).unwrap()
    }

    #[test]
    fn classes_follow_the_source_shape() {
        let cases = [
            ("untracked", r#"{"sync_source":"btcpay"}"#, "untracked"),
            ("custom", r#"{"sync_source":" BTCPay "}"#, "provider-api"),
            (
                "xpub",
                r#"{"xpub":"x","script_types":[]}"#,
                "descriptor-derived",
            ),
            (
                "xpub",
                r#"{"xpub":"x","script_types":[],"source_file":"f"}"#,
                "file-import",
            ),
            (
                "descriptor",
                r#"{"descriptor":"d","source_file":"f"}"#,
                "descriptor-derived",
            ),
            (
                "silent-payment",
                r#"{"sp_descriptor":"d","source_file":"f"}"#,
                "descriptor-derived",
            ),
            ("samourai", r#"{"source_file":"f"}"#, "descriptor-derived"),
            ("address", "{}", "address-list"),
            ("lnd", r#"{"backend":"n"}"#, "lightning-node"),
            ("lnd", "{}", "file-import"),
            ("kraken", r#"{"backend":"k"}"#, "provider-api"),
            (
                "kraken",
                r#"{"backend":"k","source_file":"f"}"#,
                "exchange-export",
            ),
            ("river", "{}", "exchange-export"),
            ("custom", r#"{"source_format":"generic_ledger"}"#, "manual"),
            ("custom", r#"{"source_format":" "}"#, "manual"),
            ("custom", r#"{"source_format":"bisq"}"#, "file-import"),
            ("phoenix", "{}", "file-import"),
        ];
        for (kind, raw, expected) in cases {
            assert_eq!(class(kind, &object(raw)), expected, "{kind} {raw}");
        }
    }

    #[test]
    fn python_str_and_bool_spellings() {
        assert_eq!(str_or_none(&Value::Bool(false)).as_deref(), Some("False"));
        assert_eq!(str_or_none(&Value::from(" ")), None);
        assert!(parse_bool(&Value::Int(1.into())).unwrap());
        assert!(!parse_bool(&Value::from(" OFF ")).unwrap());
        let error = parse_bool(&Value::Float(1.0)).unwrap_err();
        assert_eq!(error.message, "Invalid boolean value: 1.0");
        assert!(!has_addresses(&Value::from(" , ")).unwrap());
        assert!(has_addresses(&Value::List(vec![Value::Null])).unwrap());
        assert_eq!(
            has_addresses(&Value::Int(3.into())).unwrap_err().message,
            "'int' object is not iterable"
        );
        assert_eq!(object("[1]"), dict::<&str>([]));
        assert_eq!(object("{bad"), dict::<&str>([]));
    }
}
