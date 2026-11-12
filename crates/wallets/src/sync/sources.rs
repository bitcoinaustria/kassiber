//! Wallet sync routes beyond chain observation: file sources, and the Bull
//! Bitcoin export enrichment Python merges into every synced wallet. Both are
//! local reads; neither opens the network.
use crate::bull_connect;
use crate::file_import::{self, Format, RecordImport, Request};
use crate::publication::row::{field, or_text};
use kassiber_envelope::{dict, get, py_str, s, set, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::Connection;

type Result<T> = std::result::Result<T, AppError>;

const ACTIVE_FORMATS: &[&str] = &["21bitcoin_csv", "strike_csv"];
const EVIDENCE_FORMATS: &[&str] = &[
    "bullbitcoin_csv",
    "coinfinity_csv",
    "pocketbitcoin_csv",
    "binance_supplemental_csv",
];

fn validation(message: &str) -> AppError {
    AppError::new("validation", message)
}

/// The wallet's `config_json` as Python decodes it; not a dict reads as empty.
pub(crate) fn config(row: &Value) -> Value {
    config_from(&or_text(field(row, "config_json")))
}

pub(crate) fn config_from(raw: &str) -> Value {
    match kassiber_pytext::loads(if raw.is_empty() { "{}" } else { raw }) {
        Ok(value @ Value::Dict(_)) => value,
        _ => dict::<&str>([]),
    }
}

/// Python's `bool(config.get(key))` for the file-source keys.
fn present(config: &Value, key: &str) -> Option<Value> {
    get(config, key)
        .filter(|v| kassiber_envelope::truthy(v))
        .cloned()
}

/// Whether BTCPay provenance routes are configured (`btcpay_provenance`).
pub(crate) fn has_btcpay_provenance(config: &Value) -> bool {
    match get(config, "btcpay_provenance") {
        Some(Value::List(items)) => !items.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

/// `(source_file, source_format)` when both are set: Python's file route.
pub(crate) fn file_source(config: &Value) -> Option<(Value, Value)> {
    Some((
        present(config, "source_file")?,
        present(config, "source_format")?,
    ))
}

/// `wallet_bullbitcoin_wallet_export_config`: deduplicated
/// `(absolute source file, network)` routes.
pub(crate) fn bull_routes(config: &Value) -> Result<Vec<(String, String)>> {
    let raw = match get(config, "bullbitcoin_wallet_exports") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(raw) => raw,
    };
    let Value::List(items) = raw else {
        return Err(validation(
            "Bull Bitcoin wallet export config must be a list",
        ));
    };
    let mut routes = Vec::new();
    for item in items {
        if !matches!(item, Value::Dict(_)) {
            return Err(validation(
                "Bull Bitcoin wallet export routes must be objects",
            ));
        }
        let file = get(item, "source_file")
            .filter(|v| !matches!(v, Value::Null))
            .map(|v| kassiber_pytext::strip(&py_str(v)).to_owned())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| validation("Bull Bitcoin wallet export routes require source_file"))?;
        let route = (
            bull_connect::abspath(&file)?,
            bull_connect::network(get(item, "network").unwrap_or(&Value::Null))?,
        );
        if !routes.contains(&route) {
            routes.push(route);
        }
    }
    Ok(routes)
}

fn count(outcome: &Value, key: &str) -> i64 {
    match get(outcome, key) {
        Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or_default(),
        Some(Value::Bool(b)) => i64::from(*b),
        _ => 0,
    }
}

/// `enrich_wallet_from_bullbitcoin_wallet_exports`: match each route's rows
/// against the wallet's transactions, never inserting. `None` without routes.
pub(crate) fn bull_enrichment(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    config: &Value,
) -> Result<Option<Value>> {
    let routes = bull_routes(config)?;
    if routes.is_empty() {
        return Ok(None);
    }
    let mut totals = [0i64; 6];
    let mut results = Vec::new();
    for (file, network) in routes {
        let all = file_import::bull_wallet_rows(&file)?;
        let total = all.len() as i64;
        let rows = file_import::bull_wallet_filter(all, &network);
        let outcome = file_import::import_records(
            conn,
            &RecordImport {
                profile_id,
                wallet_id,
                label: &format!("bullbitcoin-wallet:{network}"),
                match_existing_only: true,
                report_updates: true,
                btcpay: false,
                settlement_ids: &[],
            },
            &rows,
        )?;
        let updated = match get(&outcome, "updated_records") {
            Some(Value::List(items)) => items.len() as i64,
            _ => 0,
        };
        let result = [
            rows.len() as i64,
            total,
            updated,
            count(&outcome, "unchanged"),
            count(&outcome, "skipped"),
        ];
        totals[0] += 1;
        for (slot, value) in totals[1..].iter_mut().zip(result) {
            *slot += value;
        }
        results.push(dict([
            ("source_file", s(file)),
            ("network", s(network)),
            ("rows", Value::Int(result[0].into())),
            ("rows_total", Value::Int(result[1].into())),
            ("updated", Value::Int(result[2].into())),
            ("unchanged", Value::Int(result[3].into())),
            ("skipped", Value::Int(result[4].into())),
            (
                "journal_invalidated",
                Value::Bool(
                    get(&outcome, "journal_invalidated").is_some_and(kassiber_envelope::truthy),
                ),
            ),
        ]));
    }
    let mut pairs: Vec<(&str, Value)> = [
        "routes",
        "rows",
        "rows_total",
        "updated",
        "unchanged",
        "skipped",
    ]
    .into_iter()
    .zip(totals)
    .map(|(key, value)| (key, Value::Int(value.into())))
    .collect();
    pairs.push(("route_results", Value::List(results)));
    Ok(Some(dict(pairs)))
}

/// `_import_file_for_sync`: the wallet's own file source, as the matching
/// import command would read it.
pub(crate) fn import_file(
    conn: &Connection,
    profile_id: &str,
    wallet: &Value,
    file: &Value,
    format: &Value,
) -> Result<Value> {
    let config = config(wallet);
    let file = py_str(file);
    let format_name = py_str(format);
    let wallet_id = or_text(field(wallet, "id"));
    let Some(parsed) = Format::from_source_format(&format_name) else {
        // `load_import_records` checks the file before the format.
        if !std::path::Path::new(&file).exists() {
            return Err(
                AppError::new("not_found", format!("Import file not found: {file}"))
                    .with_hint("Check the wallet source_file or pass an existing --file path."),
            );
        }
        return Err(AppError::new(
            "app_error",
            format!("Unsupported input format '{format_name}'"),
        ));
    };
    let (wallet_ref, mode) = if EVIDENCE_FORMATS.contains(&format_name.as_str()) {
        let mode = get(&config, "import_mode")
            .filter(|v| kassiber_envelope::truthy(v))
            .map(py_str)
            .unwrap_or_else(|| "relevant".into());
        let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&mode));
        let wallet_ref = if normalized == "full" {
            wallet_id.clone()
        } else {
            String::new()
        };
        (wallet_ref, Some(mode))
    } else {
        (wallet_id.clone(), None)
    };
    let mut outcome = file_import::execute(
        conn,
        Request {
            profile_id: profile_id.to_owned(),
            wallet: wallet_ref,
            file,
            format: parsed,
            mode,
        },
        None,
    )?;
    // The import command appends the active ledger's mode; the sync does not.
    if ACTIVE_FORMATS.contains(&format_name.as_str()) {
        if let Value::Dict(pairs) = &mut outcome {
            if pairs
                .last()
                .is_some_and(|(key, _)| *key == Key::from("mode"))
            {
                pairs.pop();
            }
        }
    }
    Ok(outcome)
}

/// `{"wallet": label, "status": "synced", **outcome}`.
pub(crate) fn synced(label: &Value, outcome: Value) -> Value {
    let mut result = dict([("wallet", label.clone()), ("status", s("synced"))]);
    if let Value::Dict(pairs) = outcome {
        for (key, value) in pairs {
            if let Key::Str(name) = &key {
                set(&mut result, &py_str(&Value::Str(name.clone())), value);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn py(text: &str) -> Value {
        kassiber_pytext::loads(text).unwrap()
    }

    #[test]
    fn bull_routes_normalize_dedupe_and_refuse_malformed_config() {
        let config = py(r#"{"bullbitcoin_wallet_exports": [
                {"source_file": " /x/bull.csv ", "network": "BTC"},
                {"source_file": "/x/y/../bull.csv", "network": "bitcoin"},
                {"source_file": "/x/bull.csv", "network": "lbtc"}]}"#);
        assert_eq!(
            bull_routes(&config).unwrap(),
            [
                ("/x/bull.csv".to_owned(), "bitcoin".to_owned()),
                ("/x/bull.csv".to_owned(), "liquid".to_owned()),
            ]
        );
        assert!(bull_routes(&py("{}")).unwrap().is_empty());
        assert!(bull_routes(&py(r#"{"bullbitcoin_wallet_exports": null}"#))
            .unwrap()
            .is_empty());
        for (raw, message) in [
            (
                r#""nope""#,
                "Bull Bitcoin wallet export config must be a list",
            ),
            (
                r#"[5]"#,
                "Bull Bitcoin wallet export routes must be objects",
            ),
            (
                r#"[{"network": "bitcoin"}]"#,
                "Bull Bitcoin wallet export routes require source_file",
            ),
            (
                r#"[{"source_file": " ", "network": "bitcoin"}]"#,
                "Bull Bitcoin wallet export routes require source_file",
            ),
        ] {
            let config = py(&format!(r#"{{"bullbitcoin_wallet_exports": {raw}}}"#));
            let error = bull_routes(&config).unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                ("validation", message)
            );
        }
        let error = bull_routes(&py(
            r#"{"bullbitcoin_wallet_exports": [{"source_file": "/a", "network": "doge"}]}"#,
        ))
        .unwrap_err();
        assert_eq!(
            error.message,
            "Unsupported Bull Bitcoin wallet network 'doge'"
        );
    }

    #[test]
    fn file_route_needs_both_keys_and_provenance_routes_are_detected() {
        assert!(file_source(&py(r#"{"source_file": "a", "source_format": "csv"}"#)).is_some());
        for raw in [
            r#"{"source_file": "a"}"#,
            r#"{"source_format": "csv"}"#,
            r#"{"source_file": "", "source_format": "csv"}"#,
            r#"{"source_file": "a", "source_format": null}"#,
        ] {
            assert!(file_source(&py(raw)).is_none(), "{raw}");
        }
        assert!(has_btcpay_provenance(&py(r#"{"btcpay_provenance": [{}]}"#)));
        assert!(!has_btcpay_provenance(&py(r#"{"btcpay_provenance": []}"#)));
        assert!(!has_btcpay_provenance(&py(
            r#"{"btcpay_provenance": null}"#
        )));
    }

    #[test]
    fn synced_outcome_keeps_wallet_and_status_first() {
        let outcome =
            py(r#"{"wallet": "Inner", "source": "file:csv", "status": "x", "imported": 2}"#);
        let result = synced(&s("Outer"), outcome);
        assert_eq!(
            kassiber_pyjson::dumps(&result, &Default::default()).unwrap(),
            r#"{"wallet": "Inner", "status": "x", "source": "file:csv", "imported": 2}"#
        );
        let bare = synced(&s("Outer"), py(r#"{"imported": 0}"#));
        assert_eq!(
            kassiber_pyjson::dumps(&bare, &Default::default()).unwrap(),
            r#"{"wallet": "Outer", "status": "synced", "imported": 0}"#
        );
    }

    #[test]
    fn unknown_formats_report_a_missing_file_first() {
        let conn = kassiber_store::rusqlite::Connection::open_in_memory().unwrap();
        let wallet = py(r#"{"id": "a", "label": "A", "config_json": "{}"}"#);
        let dir =
            std::env::temp_dir().join(format!("kassiber-sync-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("gone.csv").to_string_lossy().into_owned();
        let error =
            import_file(&conn, "p", &wallet, &s(missing.as_str()), &s("doge_csv")).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message),
            ("not_found", format!("Import file not found: {missing}"))
        );
        let present = dir.join("here.csv");
        std::fs::write(&present, "x").unwrap();
        let present = present.to_string_lossy().into_owned();
        let error =
            import_file(&conn, "p", &wallet, &s(present.as_str()), &s("doge_csv")).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("app_error", "Unsupported input format 'doge_csv'")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
