//! Watch-only BIP392 configuration. Private scan material cannot spend.
//! Retain Python's lexical validation and compact stored descriptor verbatim.
use crate::{config, descriptors, time};
use kassiber_envelope::AppError;
use regex::Regex;
use serde_json::{json, Value};

pub(crate) mod scan;

/// `has_silent_payment_sync_material(config)`.
pub(crate) fn has_material(config: &Value) -> bool {
    config::optional_text(config.get("sp_descriptor")).is_some()
}

fn error(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
fn matches(pattern: &str, value: &str) -> bool {
    Regex::new(pattern)
        .expect("constant key grammar")
        .is_match(value)
}
fn origin(value: &str) -> &str {
    if value.starts_with('[') {
        if let Some(end) = value.find(']').filter(|end| *end > 0) {
            return &value[end + 1..];
        }
    }
    value
}
fn private(value: &str) -> bool {
    let value = origin(value);
    matches(
        r"^t?spspend1q[023456789acdefghjklmnpqrstuvwxyz]{8,}$",
        &value.to_lowercase(),
    ) || matches(
        r"^(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}$",
        value,
    ) || matches(r"^[KLc5][1-9A-HJ-NP-Za-km-z]{20,}$", value)
}
fn public(value: &str) -> bool {
    let value = origin(value);
    matches(r"^(?:02|03)[0-9a-fA-F]{64}$", value)
        || matches(
            r"^(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}$",
            value,
        )
}
fn network(value: &str, expected: &str, private: bool, label: &str) -> Result<(), AppError> {
    let value = origin(value);
    let (main, test) = if private {
        (r"^(?:xprv|yprv|zprv|K|L)", r"^(?:tprv|uprv|vprv|c)")
    } else {
        (r"^(?:xpub|ypub|zpub)", r"^(?:tpub|upub|vpub)")
    };
    let actual = if matches(main, value) {
        "main"
    } else if matches(test, value) {
        "test"
    } else {
        return Ok(());
    };
    if actual != if expected == "main" { "main" } else { "test" } {
        return Err(error(format!(
            "Silent Payments {label} material is for {actual}, but wallet network is {expected}"
        ))
        .with_hint(format!(
            "Use {} {label} material for this wallet.",
            if expected == "main" {
                "mainnet"
            } else {
                "testnet"
            }
        )));
    }
    Ok(())
}
fn descriptor(
    material: &str,
    chain: &str,
    network_name: &str,
) -> Result<(String, &'static str), AppError> {
    if chain != "bitcoin" {
        return Err(
            error("Silent Payments are only supported for Bitcoin networks")
                .with_hint("Do not use BIP352/BIP392 material for Liquid wallets."),
        );
    }
    let compact: String = material
        .chars()
        .filter(|c| !kassiber_pytext::is_space(u32::from(*c)))
        .collect();
    let compact = compact.split('#').next().unwrap_or_default().to_owned();
    if !compact.to_lowercase().starts_with("sp(") || !compact.ends_with(')') {
        return Err(error(
            "Silent Payments material must be a top-level BIP392 sp(...) descriptor",
        )
        .with_hint(
            "Use watch-only material such as sp(spscan1q...) or sp(<scan-private>,<spend-public>).",
        ));
    }
    let inner = &compact[3..compact.len() - 1];
    let mut args = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (at, c) in inner.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth < 0 {
                    return Err(error("Malformed sp() descriptor"));
                }
            }
            ',' if depth == 0 => {
                args.push(&inner[start..at]);
                start = at + 1;
            }
            _ => (),
        }
    }
    if depth != 0 {
        return Err(error("Malformed sp() descriptor"));
    }
    args.push(&inner[start..]);
    args.retain(|s| !s.is_empty());
    if args.is_empty() {
        return Err(error("sp() requires watch-only key material"));
    }
    if args.iter().any(|s| s.to_lowercase().contains("spspend")) {
        return Err(descriptors::spending_error());
    }
    let format = match args.as_slice() {
        [scan] => {
            let token = origin(scan).to_lowercase();
            if !matches(
                r"^t?spscan1q[023456789acdefghjklmnpqrstuvwxyz]{8,}$",
                &token,
            ) {
                return Err(
                    error("sp(KEY) must use watch-only spscan material").with_hint(
                        "Use sp(spscan1q...) for mainnet or sp(tspscan1q...) for test networks.",
                    ),
                );
            }
            let actual = if token.starts_with('t') {
                "tspscan"
            } else {
                "spscan"
            };
            let expected = if network_name == "main" {
                "spscan"
            } else {
                "tspscan"
            };
            if actual != expected {
                return Err(error(format!("Silent Payments material is for {actual}, but wallet network is {network_name}"))
                    .with_hint(format!("Use {expected} material for this network.")));
            }
            "bip392-spscan"
        }
        [scan, spend] => {
            if !private(scan) {
                return Err(error("sp(scan, spend) requires private scan material")
                .with_hint("The scan key may be private because it cannot spend, but the spend key must be public."));
            }
            if matches(r"^5[1-9A-HJ-NP-Za-km-z]{20,}$", origin(scan)) {
                return Err(error(
                    "Uncompressed scan private keys are not accepted for BIP352",
                ));
            }
            network(scan, network_name, true, "scan key")?;
            if spend
                .split(['(', ')', ','])
                .filter(|s| !s.is_empty())
                .any(private)
            {
                return Err(descriptors::spending_error());
            }
            if !public(spend) {
                return Err(error(
                    "sp(scan, spend) requires a public compressed spend key or public extended key",
                ));
            }
            network(spend, network_name, false, "spend key")?;
            "bip392-two-key-watch-only"
        }
        _ => {
            return Err(error(
                "sp() supports one spscan key or two scan/spend key expressions",
            ))
        }
    };
    Ok((compact, format))
}
pub(crate) fn validate(input: &Value) -> Result<Value, AppError> {
    let mut out = input.clone();
    let chain = descriptors::normalize_chain(&descriptors::field(&out, "chain"))
        .map_err(descriptors::Error::into_app)?;
    let network = descriptors::normalize_network(&chain, &descriptors::field(&out, "network"))
        .map_err(descriptors::Error::into_app)?;
    let material = config::optional_text(out.get("sp_descriptor")).ok_or_else(|| {
        error("Silent Payments wallets require --sp-descriptor watch-only material")
    })?;
    let (material, format) = descriptor(&material, &chain, &network)?;
    out["sp_descriptor"] = json!(material);
    out["sp_material_format"] = json!(format);
    out["chain"] = json!(chain);
    out["network"] = json!(network);
    let raw_mode = out
        .get("sp_scan_mode")
        .filter(|v| descriptors::truthy(v))
        .cloned()
        .unwrap_or(json!("local_index"));
    let normalized = kassiber_pytext::strip(&descriptors::text(&raw_mode))
        .to_lowercase()
        .replace('-', "_");
    let mode = match normalized.as_str() {
        "local" | "local_scanner" | "local_index" => "local_index",
        "server" | "server_assisted" | "server_assisted_scan" => "server_assisted",
        _ => {
            return Err(error(format!(
                "Unsupported Silent Payments scan mode '{}'",
                descriptors::text(out.get("sp_scan_mode").unwrap_or(&Value::Null))
            ))
            .with_hint("Use local-index or server-assisted."))
        }
    };
    out["sp_scan_mode"] = json!(mode);
    let height = match out
        .get("sp_scan_start_height")
        .filter(|v| !v.is_null() && **v != json!(""))
    {
        None => None,
        Some(value) => {
            let n = kassiber_pytext::parse_int(kassiber_pytext::strip(&descriptors::text(value)))
                .map_err(|_| {
                AppError::new(
                    "app_error",
                    format!("Invalid integer value: {}", descriptors::text(value)),
                )
            })?;
            if n < 0.into() {
                return Err(error(
                    "Silent Payments scan start height must be non-negative",
                ));
            }
            Some(serde_json::from_str::<Value>(&n.to_string()).expect("integer"))
        }
    };
    if let Some(height) = &height {
        out["sp_scan_start_height"] = height.clone();
    } else {
        out.as_object_mut().unwrap().remove("sp_scan_start_height");
    }
    let date = config::optional_text(out.get("sp_scan_start_date"));
    if let Some(date) = &date {
        if time::instant(date)?.is_none() {
            return Err(error(
                "Silent Payments scan start date must be an RFC3339 timestamp",
            ));
        }
        out["sp_scan_start_date"] = json!(date);
    } else {
        out.as_object_mut().unwrap().remove("sp_scan_start_date");
    }
    let full = config::boolean(out.get("sp_full_history"))?;
    out["sp_full_history"] = json!(full);
    if full {
        out.as_object_mut().unwrap().remove("sp_scan_start_height");
        out.as_object_mut().unwrap().remove("sp_scan_start_date");
    }
    if height.is_none() && date.is_none() && !full {
        return Err(AppError::new("silent_payment_scan_start_required", "Silent Payments wallets require a scan start height/date or explicit full-history mode")
            .with_hint("Pass --sp-scan-start-height, --sp-scan-start-date, or --sp-full-history with --sp-acknowledge-full-history-warning."));
    }
    if full && !config::boolean(out.get("sp_acknowledge_full_history_warning"))? {
        return Err(AppError::new(
            "silent_payment_full_history_warning_required",
            "Full-history Silent Payments scans require an explicit warning acknowledgement",
        )
        .with_hint(
            "Pass --sp-acknowledge-full-history-warning after reviewing runtime and privacy costs.",
        ));
    }
    if mode == "server_assisted" && !config::boolean(out.get("sp_acknowledge_server_warning"))? {
        return Err(AppError::new("silent_payment_server_warning_required", "Server-assisted Silent Payments scans require an explicit privacy and completeness warning acknowledgement")
            .with_hint("Pass --sp-acknowledge-server-warning after selecting a backend deliberately and accepting that omitted scan candidates can make reports incomplete."));
    }
    if config::optional_text(out.get("backend")).is_none() {
        return Err(
            error("Silent Payments wallets require an explicit --backend").with_hint(
                "Select an SP-capable local scanner or server-assisted backend explicitly.",
            ),
        );
    }
    Ok(out)
}
