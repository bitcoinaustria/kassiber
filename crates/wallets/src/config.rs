//! Stored Python wallet configuration and its explicit output allowlist.
//! Raw configuration must never be used as an ordinary transport projection.
use crate::descriptors::{self, field, text, truthy};
use elements_miniscript::elements::hex::ToHex;
use kassiber_envelope::AppError;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::str::FromStr;

pub const KINDS: &[&str] = &[
    "descriptor",
    "xpub",
    "address",
    "silent-payment",
    "coreln",
    "lnd",
    "nwc",
    "phoenix",
    "river",
    "bullbitcoin",
    "coinfinity",
    "21bitcoin",
    "pocketbitcoin",
    "strike",
    "ledgerlive",
    "cointracking",
    "blockpit",
    "kraken",
    "coinbase",
    "binance",
    "bitpanda",
    "wasabi",
    "samourai",
    "untracked",
    "custom",
];
const SAFE: &[&str] = &[
    "addresses",
    "backend",
    "chain",
    "network",
    "gap_limit",
    "policy_asset",
    "sync_source",
    "store_id",
    "payment_method_id",
    "source_mode",
    "payment_method_ids",
    "btcpay_wallet_fingerprint",
    "btcpay_provenance",
    "bullbitcoin_wallet_network",
    "bullbitcoin_wallet_exports",
    "source_file",
    "source_format",
    "altbestand",
    "wasabi_metadata",
    "samourai",
    "descriptor_source",
    "ownership_scan_to_index",
    "synthesize_change",
    "script_types",
    "sp_material_format",
    "sp_scan_mode",
    "sp_scan_start_height",
    "sp_scan_start_date",
    "sp_full_history",
    "sp_scan_start_kind",
    "deprecated",
];
const PRIVATE: &[&str] = &[
    "addresses",
    "blinding_key",
    "chain",
    "change_descriptor",
    "descriptor",
    "gap_limit",
    "network",
    "ownership_scan_to_index",
    "samourai",
    "script_types",
    "synthesize_change",
    "xpub",
];
pub(crate) fn safe_fields() -> &'static [&'static str] {
    SAFE
}
pub fn optional_text(value: Option<&Value>) -> Option<String> {
    value
        .filter(|v| !v.is_null())
        .map(text)
        .map(|s| kassiber_pytext::strip(&s).to_owned())
        .filter(|s| !s.is_empty())
}
/// `wallet_in_sync_all`: retired wallets are skipped only for ongoing syncs.
/// One whose history the book has not imported (never synced, or cleared by
/// a book reset) is included, and so is every retired wallet in a forced
/// full rescan.
pub fn in_sync_all(config: &Value, force_full: bool) -> Result<bool, AppError> {
    if force_full || !boolean(config.get("deprecated"))? {
        return Ok(true);
    }
    let synced = config.get("last_synced_at").filter(|v| truthy(v));
    Ok(synced.is_none_or(|v| kassiber_pytext::strip(&text(v)).is_empty()))
}

pub fn boolean(value: Option<&Value>) -> Result<bool, AppError> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(false);
    };
    match kassiber_pytext::strip(&text(value)).to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::new(
            "app_error",
            format!("Invalid boolean value: {}", text(value)),
        )),
    }
}
pub fn kind(input: &str) -> Result<String, AppError> {
    let kind = kassiber_pytext::strip(input).to_lowercase();
    if !KINDS.contains(&kind.as_str()) {
        return Err(AppError::new(
            "app_error",
            format!(
                "Unsupported wallet kind '{input}'. Supported: {}",
                KINDS.join(", ")
            ),
        ));
    }
    Ok(kind)
}
pub fn addresses(value: Option<&Value>) -> Result<Vec<String>, AppError> {
    let values = match value {
        None | Some(Value::Null) => vec![],
        Some(Value::String(t)) => t.split(',').map(|t| json!(t)).collect(),
        Some(Value::Array(v)) => v.clone(),
        Some(Value::Object(v)) => v.keys().map(|t| json!(t)).collect(),
        Some(value) => {
            let kind = match value {
                Value::Bool(_) => "bool",
                Value::Number(n) if n.is_i64() || n.is_u64() => "int",
                _ => "float",
            };
            return Err(AppError::new(
                "internal_error",
                format!("'{kind}' object is not iterable"),
            ));
        }
    };
    let mut seen = BTreeSet::new();
    Ok(values
        .iter()
        .filter_map(|v| {
            let address = kassiber_pytext::strip(&text(v)).to_owned();
            if address.is_empty() {
                return None;
            }
            let script = bitcoin::Address::from_str(&address)
                .ok()
                .map(|a| a.assume_checked().script_pubkey().to_hex_string())
                .or_else(|| {
                    elements_miniscript::elements::Address::from_str(&address)
                        .ok()
                        .map(|a| a.script_pubkey().to_hex())
                });
            let key = script
                .map(|s| format!("script:{s}"))
                .unwrap_or_else(|| format!("text:{address}"));
            seen.insert(key).then_some(address)
        })
        .collect())
}

/// Bitcoin address scripts for local ownership/receipt readers. This has the
/// same Bitcoin-only scope as core.address_scripts; Liquid display addresses
/// must not be treated as ownership scripts by those callers.
pub fn bitcoin_address_script(address: &str) -> Option<String> {
    bitcoin::Address::from_str(address)
        .ok()
        .map(|address| address.assume_checked().script_pubkey().to_hex_string())
}
pub fn safe(config: &Value) -> Result<Value, AppError> {
    let mut safe = json!({});
    for key in SAFE {
        if let Some(value) = config.get(key) {
            safe[key] = if *key == "addresses" {
                json!(addresses(Some(value))?)
            } else {
                value.clone()
            };
        }
    }
    for key in ["descriptor", "change_descriptor", "xpub", "sp_descriptor"] {
        if config.get(key).is_some() {
            safe[key] = json!("[redacted]");
        }
    }
    Ok(safe)
}
pub fn private_material(config: &Value) -> Value {
    let mut out = json!({});
    for key in PRIVATE {
        if let Some(value) = config
            .get(key)
            .filter(|v| !v.is_null() && **v != json!("") && **v != json!([]))
        {
            out[key] = value.clone();
        }
    }
    out
}
pub fn canonical(config: &Value) -> Value {
    let mut out = config.clone();
    if out.as_object().is_some_and(|v| !v.is_empty()) {
        let chain = descriptors::normalize_chain(&field(&out, "chain"))
            .unwrap_or_else(|_| field(&out, "chain").trim().to_lowercase());
        let network = descriptors::normalize_network(&chain, &field(&out, "network"))
            .unwrap_or_else(|_| field(&out, "network").trim().to_lowercase());
        out["chain"] = json!(chain);
        out["network"] = json!(network);
    }
    if let Some(v) = out.get("script_types").cloned() {
        let values = match v {
            Value::String(_) => vec![v],
            Value::Array(v) => v,
            _ => return out,
        };
        let types: BTreeSet<_> = values
            .iter()
            .filter(|v| truthy(v))
            .filter_map(|v| optional_text(Some(v)).map(|s| s.to_lowercase()))
            .collect();
        out["script_types"] = json!(types);
    }
    out
}
pub fn policy_identity(config: &Value) -> Value {
    let mut material = private_material(config);
    material.as_object_mut().unwrap().remove("gap_limit");
    material
        .as_object_mut()
        .unwrap()
        .remove("ownership_scan_to_index");
    canonical(&material)
}
pub fn sync_identity(config: &Value) -> Value {
    let mut config = config.clone();
    config.as_object_mut().unwrap().remove("deprecated");
    config
        .as_object_mut()
        .unwrap()
        .remove("ownership_scan_to_index");
    canonical(&config)
}
pub fn live_chain(config: &Value) -> Result<Option<(String, String)>, AppError> {
    if ![
        "descriptor",
        "change_descriptor",
        "xpub",
        "addresses",
        "sp_descriptor",
        "chain",
        "network",
    ]
    .iter()
    .any(|k| config.get(k).is_some_and(truthy))
    {
        return Ok(None);
    }
    let chain = descriptors::normalize_chain(&field(config, "chain"))
        .map_err(descriptors::Error::into_app)?;
    let network = descriptors::normalize_network(&chain, &field(config, "network"))
        .map_err(descriptors::Error::into_app)?;
    Ok(Some((chain, network)))
}
pub fn asset_code(input: &str) -> String {
    let text = kassiber_pytext::strip(input);
    if text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        text.to_lowercase()
    } else if text.eq_ignore_ascii_case("l-btc") || text.eq_ignore_ascii_case("lbtc") {
        "LBTC".into()
    } else {
        text.to_uppercase()
    }
}
pub fn payment_method(input: &str) -> Result<String, AppError> {
    let raw = kassiber_pytext::strip(input).to_uppercase();
    if raw.is_empty() {
        return Err(AppError::new(
            "validation",
            "BTCPay payment method id cannot be empty",
        ));
    }
    let parts = raw.split_once('-').or_else(|| raw.split_once('_'));
    let Some((currency, handler)) = parts else {
        return Ok(
            if raw.chars().count() <= 5 && raw.chars().all(char::is_alphabetic) {
                format!("{raw}-CHAIN")
            } else {
                raw
            },
        );
    };
    let handler = handler.trim().replace(['-', '_'], "");
    let handler = match handler.as_str() {
        "CHAIN" | "ONCHAIN" | "BTCLIKE" => "CHAIN",
        "LN" | "LIGHTNINGNETWORK" | "LIGHTNINGLIKE" => "LN",
        "LNURL" | "LNURLPAY" => "LNURL",
        _ => return Ok(raw),
    };
    Ok(format!("{}-{handler}", currency.trim()))
}
pub(crate) fn normalize_btcpay(config: &mut Value) -> Result<(), AppError> {
    let Some(source) = optional_text(config.get("sync_source")) else {
        return Ok(());
    };
    let source = source.to_lowercase();
    if source != "btcpay" {
        return Err(AppError::new(
            "validation",
            format!("Unsupported source refresh type '{source}'"),
        )
        .with_hint("Supported refresh sources: btcpay"));
    }
    let backend = optional_text(config.get("backend")).ok_or_else(|| {
        AppError::new(
            "validation",
            "BTCPay-backed wallets require a named --backend",
        )
        .with_hint("Set --backend to a btcpay backend before refreshing this source.")
    })?;
    let store = optional_text(config.get("store_id"))
        .ok_or_else(|| AppError::new("validation", "BTCPay-backed wallets require --store-id"))?;
    config["sync_source"] = json!("btcpay");
    config["backend"] = json!(backend.to_lowercase());
    config["store_id"] = json!(store);
    config["payment_method_id"] =
        json!(optional_text(config.get("payment_method_id")).unwrap_or_else(|| "BTC-CHAIN".into()));
    Ok(())
}
pub fn provenance(raw: &Value) -> Result<Value, AppError> {
    if raw.is_null() {
        return Ok(json!([]));
    }
    let routes = raw
        .as_array()
        .ok_or_else(|| AppError::new("validation", "BTCPay provenance config must be a list"))?;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for route in routes {
        if !route.is_object() {
            return Err(AppError::new(
                "validation",
                "BTCPay provenance routes must be objects",
            ));
        }
        let backend = optional_text(route.get("backend"));
        let store = optional_text(route.get("store_id"));
        let (Some(backend), Some(store)) = (backend, store) else {
            return Err(AppError::new(
                "validation",
                "BTCPay provenance routes require backend and store_id",
            ));
        };
        let method = payment_method(
            &optional_text(route.get("payment_method_id")).unwrap_or_else(|| "BTC-CHAIN".into()),
        )?;
        let route =
            json!({"backend":backend.to_lowercase(),"store_id":store,"payment_method_id":method});
        if seen.insert(route.to_string()) {
            out.push(route);
        }
    }
    Ok(json!(out))
}
fn bull_network(value: &Value) -> Result<String, AppError> {
    let raw = optional_text(Some(value)).ok_or_else(|| {
        AppError::new("validation", "Bull Bitcoin wallet network cannot be empty")
    })?;
    let network = match raw.to_lowercase().as_str() {
        "btc" | "onchain" | "chain" | "bitcoin" => "bitcoin",
        "lbtc" | "liquidv1" | "liquid" => "liquid",
        "ln" | "lightning" => "lightning",
        _ => {
            return Err(AppError::new(
                "validation",
                format!("Unsupported Bull Bitcoin wallet network '{}'", text(value)),
            )
            .with_hint("Supported networks: bitcoin, liquid, lightning."))
        }
    };
    Ok(network.into())
}
fn bull_routes(config: &mut Value) -> Result<(), AppError> {
    if let Some(value) = config
        .get("bullbitcoin_wallet_network")
        .filter(|v| !v.is_null() && **v != json!(""))
    {
        config["bullbitcoin_wallet_network"] = json!(bull_network(value)?);
    }
    if let Some(value) = config.get("bullbitcoin_wallet_exports") {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        if !value.is_null() {
            let routes = value.as_array().ok_or_else(|| {
                AppError::new(
                    "validation",
                    "Bull Bitcoin wallet export config must be a list",
                )
            })?;
            for route in routes {
                if !route.is_object() {
                    return Err(AppError::new(
                        "validation",
                        "Bull Bitcoin wallet export routes must be objects",
                    ));
                }
                let file = optional_text(route.get("source_file")).ok_or_else(|| {
                    AppError::new(
                        "validation",
                        "Bull Bitcoin wallet export routes require source_file",
                    )
                })?;
                let file = if let Some(tail) = file.strip_prefix("~/") {
                    std::env::var("HOME")
                        .map(|h| format!("{h}/{tail}"))
                        .unwrap_or(file)
                } else {
                    file
                };
                let file = std::path::absolute(file).map_err(|_| {
                    AppError::new("internal_error", "Could not resolve source file")
                })?;
                let route = json!({"source_file":file.to_string_lossy(),"network":bull_network(route.get("network").unwrap_or(&Value::Null))?});
                if seen.insert(route.to_string()) {
                    out.push(route);
                }
            }
        }
        config["bullbitcoin_wallet_exports"] = json!(out);
    }
    Ok(())
}
/// Validate before any wallet row, epoch, or observer state can be written.
pub fn validate(kind: &str, input: &Value) -> Result<Value, AppError> {
    let mut config = input.clone();
    if !config.is_object() {
        return Err(AppError::new(
            "validation",
            "Wallet config must be an object",
        ));
    }
    if let Some(value) = config.get("ownership_scan_to_index") {
        let n = match value {
            Value::Bool(b) => Some(i64::from(*b)),
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|n| n as i64)),
            Value::String(s) => kassiber_pytext::parse_int(s)
                .ok()
                .and_then(|n| i64::try_from(n).ok()),
            _ => None,
        }
        .ok_or_else(|| AppError::new("validation", "ownership_scan_to_index must be an integer"))?;
        if !(0..=20_000).contains(&n) {
            return Err(AppError::new(
                "validation",
                "ownership_scan_to_index must be between 0 and 20000",
            ));
        }
        config["ownership_scan_to_index"] = json!(n);
    }
    if config.get("addresses").is_some() {
        config["addresses"] = json!(addresses(config.get("addresses"))?);
    }
    if config.get("birthday").is_some() {
        if let Some(birthday) = optional_text(config.get("birthday")) {
            config["birthday"] = json!(crate::time::timestamp(&birthday)?);
        } else {
            config.as_object_mut().unwrap().remove("birthday");
        }
    }
    if config.get("deprecated").is_some() {
        config["deprecated"] = json!(boolean(config.get("deprecated"))?);
    }
    if kind == "silent-payment" {
        for key in ["descriptor", "change_descriptor"] {
            if let Some(material) = optional_text(config.get(key)) {
                descriptors::assert_watch_only(&material).map_err(descriptors::Error::into_app)?;
            }
        }
        if let Some(material) = optional_text(config.get("xpub")) {
            descriptors::assert_standalone_watch_only(&material)
                .map_err(descriptors::Error::into_app)?;
        }
        return crate::silent_payments::validate(&config);
    }
    let plan = if config.get("descriptor").is_some_and(truthy)
        || config.get("xpub").is_some_and(truthy)
        || config.get("change_descriptor").is_some_and(truthy)
    {
        descriptors::load(&config).map_err(descriptors::Error::into_app)?
    } else {
        None
    };
    let scope = match &plan {
        Some(p) => Some((p.chain.clone(), p.network.clone())),
        None => live_chain(&config)?,
    };
    if kind == "address"
        && !config.get("addresses").is_some_and(truthy)
        && !config.get("source_file").is_some_and(truthy)
    {
        return Err(AppError::new(
            "validation",
            "Address wallets require at least one --address or a file-based source",
        ));
    }
    if ["descriptor", "xpub"].contains(&kind)
        && plan.is_none()
        && !config.get("source_file").is_some_and(truthy)
    {
        return Err(AppError::new("validation", "Descriptor and xpub wallets require a descriptor, an xpub with script types, or a file-based source").with_hint("Pass --script-type with a bare xpub, or set config script_types."));
    }
    if kind == "coreln" && !config.get("backend").is_some_and(truthy) {
        return Err(AppError::new(
            "validation",
            "Core Lightning wallets require a --backend for live read-only sync",
        ));
    }
    if let Some((chain, network)) = scope {
        if chain == "liquid" {
            let file = config.get("source_file").is_some_and(truthy);
            if plan.is_none() && !file {
                return Err(AppError::new("validation", "Liquid live refresh currently requires a descriptor with private blinding keys"));
            }
            if let Some(p) = &plan {
                if !p.can_unblind() {
                    return Err(AppError::new("validation", "Liquid descriptor wallets require private blinding keys for full sync and fee accounting"));
                }
                if !config.get("backend").is_some_and(truthy) && !file {
                    return Err(AppError::new("validation", "Liquid descriptor wallets require an explicit --backend; no public Liquid default is built in"));
                }
                let asset = asset_code(&field(&config, "policy_asset"));
                let asset = if asset.is_empty() || asset.eq_ignore_ascii_case("lbtc") {
                    match network.as_str() {
                        "liquidv1" => {
                            "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d"
                        }
                        "liquidtestnet" => {
                            "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49"
                        }
                        _ => "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
                    }
                    .to_owned()
                } else {
                    asset
                };
                config["policy_asset"] = json!(asset);
            }
        }
        config["chain"] = json!(chain);
        config["network"] = json!(network);
    }
    // Route normalizers are kept local and never contact configured providers.
    bull_routes(&mut config)?;
    normalize_btcpay(&mut config)?;
    Ok(config)
}

pub fn stored_json(config: &Value) -> String {
    let value = kassiber_pytext::loads(&config.to_string()).expect("JSON configuration");
    kassiber_pyjson::dumps(
        &value,
        &kassiber_pyjson::Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .expect("stored configuration JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_address_containers_fail_closed() {
        for value in [json!(true), json!(false), json!(0), json!(1), json!(1.5)] {
            let error = validate("custom", &json!({"addresses":value})).unwrap_err();
            assert_eq!(error.code, "internal_error");
            assert!(error.message.ends_with("object is not iterable"));
        }
        assert_eq!(
            addresses(Some(&json!({"one":0,"two":1}))).unwrap(),
            vec!["one", "two"]
        );
        assert_eq!(addresses(Some(&Value::Null)).unwrap(), Vec::<String>::new());
        assert_eq!(
            canonical(&json!({"script_types":[false,0,null,"P2TR","p2tr"]}))["script_types"],
            json!(["p2tr"])
        );
    }
    #[test]
    fn payment_method_aliases_match_reference() {
        for (input, expected) in [
            ("btc", "BTC-CHAIN"),
            ("lbtc_onchain", "LBTC-CHAIN"),
            ("btc-LightningNetwork", "BTC-LN"),
            ("btc-lnurlpay", "BTC-LNURL"),
            ("btc-unknown", "BTC-UNKNOWN"),
        ] {
            assert_eq!(payment_method(input).unwrap(), expected);
        }
        assert_eq!(payment_method(" ").unwrap_err().code, "validation");
    }
    #[test]
    fn liquid_policy_assets_resolve_regtest_and_preserve_hex_case() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        let mut config = vectors["configs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "liquid-config")
            .unwrap()["config"]
            .clone();
        config["network"] = json!("elementsregtest");
        assert_eq!(
            validate("descriptor", &config).unwrap()["policy_asset"],
            "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225"
        );
        config["policy_asset"] = json!(" L-BTC ");
        assert_eq!(
            validate("descriptor", &config).unwrap()["policy_asset"],
            "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225"
        );
        config["policy_asset"] = json!("AB".repeat(32));
        assert_eq!(
            validate("descriptor", &config).unwrap()["policy_asset"],
            "ab".repeat(32)
        );
    }
    #[test]
    fn python_wallet_config_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        let mut mismatches = Vec::new();
        for case in vectors["configs"].as_array().unwrap() {
            let actual = match validate(case["kind"].as_str().unwrap(), &case["config"]) {
                Ok(config) => {
                    json!({"ok":{"config_json":stored_json(&config),"safe_config":safe(&config).unwrap(),"private_policy_material":private_material(&config),"policy_identity_material":policy_identity(&config),"config":config}})
                }
                Err(error) => descriptors::Error::App(error).projection(),
            };
            if actual != case["expected"] {
                mismatches.push(format!(
                    "{}: {actual} != {}",
                    case["name"], case["expected"]
                ));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }
}
