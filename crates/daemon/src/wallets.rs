//! Local descriptor preview; network-backed detection remains disabled.
use crate::{
    output::{with_request_id, RequestId},
    Daemon, Handled,
};
use kassiber_envelope::{dict, get, s, set, success, type_name, AppError};
use kassiber_pyjson::{dumps, Value};
use kassiber_wallets::{config, descriptors, material, Operation};
use serde_json::{json, Value as Json};
pub const KINDS: &[&str] = &[
    "ui.wallets.list",
    "ui.wallets.utxos",
    "ui.wallets.create",
    "ui.wallets.preview_descriptor",
    "ui.wallets.update",
    "ui.wallets.delete",
];

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    if kind == "ui.wallets.list" {
        if let Some(locked) = daemon.open_lazily(request_id)? {
            return Ok((Some(locked), false));
        }
        let payload = daemon
            .book
            .as_ref()
            .expect("opened")
            .wallets_snapshot(daemon.runtime_config.default_backend.as_deref())?;
        return Ok((
            Some(with_request_id(success(kind, payload), request_id)),
            false,
        ));
    }
    if kind == "ui.wallets.utxos" {
        if let Some(locked) = daemon.open_lazily(request_id)? {
            return Ok((Some(locked), false));
        }
        // Python passes the raw request args; the snapshot validates them.
        let payload = daemon
            .book
            .as_ref()
            .expect("opened")
            .wallet_utxos(args, daemon.runtime_config.default_backend.as_deref())?;
        return Ok((
            Some(with_request_id(success(kind, payload), request_id)),
            false,
        ));
    }
    if kind != "ui.wallets.preview_descriptor" {
        return lifecycle(daemon, kind, args, request_id);
    }
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    let mut args = match args {
        None | Some(Value::Null) => dict::<&str>([]),
        Some(v @ Value::Dict(_)) => v.clone(),
        Some(v) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(v)))]))
                .into())
        }
    };
    for key in ["descriptor", "change_descriptor", "wallet_material"] {
        let value = crate::imports::optional_string(&args, key)?;
        set(&mut args, key, value.map_or(Value::Null, s));
    }
    if get(&args, "wallet_material").is_some_and(|v| !matches!(v, Value::Null)) {
        let value = crate::imports::optional_string(&args, "script_type")?;
        set(&mut args, "script_type", value.map_or(Value::Null, s));
        if let Some(value) = get(&args, "script_types") {
            if !matches!(value, Value::Null | Value::Str(_) | Value::List(_)) {
                return Err(AppError::new(
                    "validation",
                    "script_types must be an array of script type names",
                )
                .with_details(dict([("type", s(type_name(value)))]))
                .into());
            }
        }
    }
    for key in ["chain", "network"] {
        let value = crate::imports::optional_string(&args, key)?;
        set(&mut args, key, value.map_or(Value::Null, s));
    }
    let args =
        serde_json::from_str(&dumps(&args, &Default::default()).map_err(|_| {
            AppError::new("internal_error", "Invalid descriptor preview arguments")
        })?)
        .map_err(|_| AppError::new("internal_error", "Invalid descriptor preview arguments"))?;
    let payload = kassiber_wallets::preview(&args)?;
    Ok((
        Some(with_request_id(success(kind, payload), request_id)),
        false,
    ))
}

fn json_args(args: &Value) -> Result<Json, AppError> {
    serde_json::from_str(
        &dumps(args, &Default::default())
            .map_err(|_| AppError::new("validation", "Invalid wallet arguments"))?,
    )
    .map_err(|_| AppError::new("validation", "Invalid wallet arguments"))
}
fn required(args: &Value, key: &str, label: &str) -> Result<String, AppError> {
    crate::imports::optional_string(args, key)?.ok_or_else(|| {
        AppError::new("validation", format!("{label} is required."))
            .with_hint(format!("Enter {}.", label.to_lowercase()))
    })
}
fn wallet_ref(args: &Value, kind: &str) -> Result<String, AppError> {
    match get(args, "wallet") {
        Some(Value::Str(v)) if kassiber_pytext::strip_py(v).code_points().next().is_some() => {
            crate::imports::optional_string(args, "wallet")?
                .ok_or_else(|| AppError::new("validation", format!("{kind} requires wallet")))
        }
        _ => Err(
            AppError::new("validation", format!("{kind} requires wallet"))
                .with_hint("Pass the wallet id or label for the active book."),
        ),
    }
}
fn auth(
    daemon: &mut Daemon,
    args: &Value,
    request_id: &RequestId,
    scope: &str,
    label: &str,
) -> Result<Option<Value>, AppError> {
    let response = get(args, "auth_response");
    if daemon.store.database_is_encrypted(&daemon.data_root)? {
        let supplied = response.and_then(|a| get(a, "passphrase_secret"));
        let Some(Value::Str(value)) = supplied.filter(|v| kassiber_envelope::truthy(v)) else {
            return Ok(Some(with_request_id(
                success(
                    "auth_required",
                    dict([("scope", s(scope)), ("label", s(label))]),
                ),
                request_id,
            )));
        };
        let verified = if let Some(value) = value.as_str() {
            daemon.store.verify_passphrase(
                &daemon.data_root,
                scope,
                &kassiber_runtime::store::Secret::new(value.into()),
            )?
        } else {
            false
        };
        if !verified {
            return Ok(Some(crate::error_envelope(
                "local_auth_denied",
                "passphrase verification failed",
                request_id,
                None,
                None,
                true,
            )));
        }
    } else {
        let (key, value) = if scope == "delete_wallet" {
            ("plaintext_delete_ack", "DELETE LOCAL DATA")
        } else {
            ("plaintext_change_ack", "CHANGE LOCAL DATA")
        };
        if response.and_then(|a| get(a, key)) != Some(&s(value)) {
            return Err(AppError::new(
                "validation",
                format!("{scope} requires plaintext acknowledgement"),
            )
            .with_hint(format!(
                "Ask the user to type {} before changing plaintext local data.",
                kassiber_pytext::repr_str(value)
            )));
        }
    }
    Ok(None)
}
pub(crate) fn source_file(args: &Value) -> Result<Option<String>, AppError> {
    let Some(file) = crate::imports::optional_string(args, "source_file")? else {
        return Ok(None);
    };
    let expanded = if let Some(tail) = file.strip_prefix("~/") {
        std::env::var("HOME")
            .map(|h| format!("{h}/{tail}"))
            .unwrap_or_else(|_| file.clone())
    } else {
        file.clone()
    };
    let path = std::path::Path::new(&expanded);
    if !path.exists() {
        return Err(
            AppError::new("not_found", format!("Source file not found: {file}"))
                .with_hint("Choose an existing local export file."),
        );
    }
    path.canonicalize()
        .map(|p| Some(p.to_string_lossy().into_owned()))
        .map_err(|_| AppError::new("internal_error", "Could not resolve source file"))
}
pub(crate) const SOURCE_FORMATS: &[&str] = kassiber_wallets::connections::SOURCE_FORMATS;
fn build_config(args: &Value, update: bool) -> Result<Json, AppError> {
    let raw = json_args(args)?;
    let mut c = json!({});
    for key in [
        "backend",
        "chain",
        "network",
        "policy_asset",
        "descriptor",
        "change_descriptor",
        "birthday",
        "store_id",
        "payment_method_id",
        "source_format",
    ] {
        if let Some(value) = crate::imports::optional_string(args, key)? {
            c[key] = json!(value);
        }
    }
    if !update {
        for key in ["sp_descriptor", "sp_scan_mode", "sp_scan_start_date"] {
            if let Some(value) = crate::imports::optional_string(args, key)? {
                c[key] = json!(value);
            }
        }
        if let Some(value) =
            get(args, "sp_scan_start_height").filter(|v| **v != Value::Null && **v != s(""))
        {
            if !matches!(value, Value::Bool(_) | Value::Int(_)) {
                return Err(AppError::new(
                    "validation",
                    "sp_scan_start_height must be an integer",
                ));
            }
            c["sp_scan_start_height"] = raw["sp_scan_start_height"].clone();
        }
        for key in [
            "sp_full_history",
            "sp_acknowledge_full_history_warning",
            "sp_acknowledge_server_warning",
        ] {
            if let Some(value) = get(args, key) {
                c[key] = json!(kassiber_envelope::truthy(value));
            }
        }
    } else if let Some(value) = get(args, "deprecated").filter(|v| **v != Value::Null) {
        if !matches!(value, Value::Bool(_)) {
            return Err(AppError::new("validation", "deprecated must be a boolean")
                .with_details(dict([("type", s(type_name(value)))])));
        }
        c["deprecated"] = raw["deprecated"].clone();
    }
    if let Some(format) = c.get("source_format").and_then(Json::as_str) {
        if !SOURCE_FORMATS.contains(&format) {
            return Err(AppError::new(
                "validation",
                format!("Unsupported source format '{format}'"),
            )
            .with_hint("Choose a supported file format."));
        }
    }
    if let Some(file) = source_file(args)? {
        c["source_file"] = json!(file);
    }
    if let Some(gap) = get(args, "gap_limit").filter(|v| **v != Value::Null && **v != s("")) {
        if !matches!(gap, Value::Bool(_) | Value::Int(_)) {
            return Err(AppError::new("validation", "gap_limit must be an integer")
                .with_details(dict([("type", s(type_name(gap)))])));
        }
        let n = if let Value::Bool(b) = gap {
            i64::from(*b)
        } else {
            kassiber_envelope::py_str(gap)
                .parse::<i64>()
                .unwrap_or(i64::MAX)
        };
        if n <= 0 {
            return Err(AppError::new("validation", "gap_limit must be positive"));
        }
        if n > 5000 {
            return Err(
                AppError::new("validation", "gap_limit must be 5000 or lower")
                    .with_hint("Use a smaller unused-address scan window."),
            );
        }
        c["gap_limit"] = raw["gap_limit"].clone();
    }
    if let Some(addresses) = raw
        .get("addresses")
        .filter(|v| !v.is_null() && **v != json!(""))
    {
        c["addresses"] = json!(config::addresses(Some(addresses))?);
    }
    let material = crate::imports::optional_string(args, "wallet_material")?;
    let types = raw.get("script_types").filter(|v| !v.is_null());
    if let Some(types) = types {
        if !types.is_array() && !types.is_string() {
            return Err(AppError::new(
                "validation",
                "script_types must be an array of script type names",
            )
            .with_details(dict([(
                "type",
                s(type_name(get(args, "script_types").unwrap())),
            )])));
        }
    }
    if let Some(material) = material {
        let single = crate::imports::optional_string(args, "script_type")?;
        let mut options = json!({"script_type":single});
        if let Some(types) = types {
            options["script_types"] = types.clone();
        }
        let m = material::normalize(&material, &options).map_err(descriptors::Error::into_app)?;
        if !update {
            if let Some(network) = m.get("network") {
                if let Some(explicit) = config::optional_text(c.get("network")) {
                    let explicit = descriptors::normalize_network("bitcoin", &explicit)
                        .map_err(descriptors::Error::into_app)?;
                    let family = if explicit == "main" { "main" } else { "test" };
                    if network != family {
                        return Err(AppError::new(
                            "validation",
                            "Wallet material does not match the configured Bitcoin network",
                        )
                        .with_details(dict([
                            ("configured_network", s(explicit)),
                            ("key_network", s(network.as_str().unwrap_or_default())),
                        ])));
                    }
                } else {
                    c["network"] = network.clone();
                }
            }
        }
        if m.get("xpub").is_some() {
            c["xpub"] = m["xpub"].clone();
            c["script_types"] = m["script_types"].clone();
            for key in [
                "descriptor",
                "change_descriptor",
                "descriptor_source",
                "synthesize_change",
            ] {
                if update {
                    c[key] = Json::Null;
                } else {
                    c.as_object_mut().unwrap().remove(key);
                }
            }
        } else {
            if update || c.get("descriptor").is_none() {
                c["descriptor"] = m["descriptor"].clone();
            }
            if let Some(change) = m.get("change_descriptor") {
                if update || c.get("change_descriptor").is_none() {
                    c["change_descriptor"] = change.clone();
                }
            } else if update && c.get("change_descriptor").is_none() {
                c["change_descriptor"] = Json::Null;
            }
            for key in ["descriptor_source", "synthesize_change"] {
                if update || m.get(key).is_some() {
                    c[key] = m.get(key).cloned().unwrap_or(Json::Null);
                }
            }
            if update {
                c["xpub"] = Json::Null;
                c["script_types"] = Json::Null;
            }
        }
    } else if update {
        if let Some(types) = types {
            let types = material::script_types(types).map_err(descriptors::Error::into_app)?;
            if types.is_empty() {
                return Err(AppError::new(
                    "validation",
                    "Select at least one script type",
                ));
            }
            c["script_types"] = json!(types);
        }
    }
    if update {
        if let Some(routes) = raw.get("btcpay_provenance").filter(|v| !v.is_null()) {
            c["btcpay_provenance"] = config::provenance(routes)?;
        }
    }
    Ok(c)
}
fn lifecycle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    let empty = dict::<&str>([]);
    let args = match args {
        None | Some(Value::Null) => &empty,
        Some(v @ Value::Dict(_)) => v,
        Some(v) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(v)))]))
                .into())
        }
    };
    let (workspace_id, profile_id) = daemon
        .book
        .as_ref()
        .expect("opened")
        .resolve_scope(None, None)?;
    let operation = if kind == "ui.wallets.create" {
        let label = required(args, "label", "Connection label")?;
        let kind = required(args, "kind", "Connection type")?;
        let config = build_config(args, false)?;
        let account = crate::imports::optional_string(args, "account")?;
        Operation::Create {
            workspace_id,
            profile_id,
            label,
            kind,
            account,
            config,
        }
    } else {
        if kind == "ui.wallets.delete" && get(args, "confirm") != Some(&s("DELETE")) {
            return Err(
                AppError::new("validation", "ui.wallets.delete requires confirm='DELETE'")
                    .with_hint("Ask the user to confirm the destructive wallet-source deletion.")
                    .into(),
            );
        }
        let cascade = match get(args, "cascade") {
            None => false,
            Some(Value::Bool(b)) => *b,
            _ if kind == "ui.wallets.delete" => {
                return Err(AppError::new(
                    "validation",
                    "ui.wallets.delete cascade must be a boolean",
                )
                .with_hint("Pass cascade=true only when the user confirmed local row deletion.")
                .into())
            }
            _ => false,
        };
        let wallet = wallet_ref(args, kind)?;
        let config = if kind == "ui.wallets.update" {
            build_config(args, true)?
        } else {
            json!({})
        };
        let mut clear = Vec::new();
        let mut label = None;
        if kind == "ui.wallets.update" {
            if let Some(value) = get(args, "clear").filter(|v| **v != Value::Null) {
                let Value::List(values) = value else {
                    return Err(AppError::new(
                        "validation",
                        "clear must be a list of config field names",
                    )
                    .into());
                };
                for value in values {
                    let Value::Str(v) = value else {
                        return Err(AppError::new(
                            "validation",
                            "clear must be a list of config field names",
                        )
                        .into());
                    };
                    if let Some(v) = v
                        .as_str()
                        .map(kassiber_pytext::strip)
                        .filter(|v| !v.is_empty())
                    {
                        clear.push(v.to_owned());
                    }
                }
            }
            if let Some(value) = get(args, "label").filter(|v| **v != Value::Null) {
                let text = match value {
                    Value::Str(v) => v
                        .as_str()
                        .map(kassiber_pytext::strip)
                        .filter(|v| !v.is_empty()),
                    _ => None,
                }
                .ok_or_else(|| AppError::new("validation", "label must be a non-empty string"))?;
                label = Some(text.into());
            }
            if label.is_none() && config.as_object().unwrap().is_empty() && clear.is_empty() {
                return Err(AppError::new(
                    "validation",
                    "ui.wallets.update requires label, config, or clear",
                )
                .with_hint("Pass at least one field to change.")
                .into());
            }
        }
        let current = daemon
            .book
            .as_mut()
            .expect("opened")
            .wallets(Operation::Get {
                profile_id: profile_id.clone(),
                wallet: wallet.clone(),
            })?;
        let current_label = get(&current, "label")
            .and_then(|v| match v {
                Value::Str(v) => v.as_str(),
                _ => None,
            })
            .unwrap_or_default();
        if kind == "ui.wallets.delete" && get(args, "confirm_wallet") != Some(&s(current_label)) {
            return Err(AppError::new(
                "validation",
                "ui.wallets.delete requires the current wallet label",
            )
            .with_hint("Ask the user to type the exact wallet label before deleting it.")
            .with_details(dict([("expected_wallet", s(current_label))]))
            .into());
        }
        let (scope, verb) = if kind == "ui.wallets.delete" {
            ("delete_wallet", "delete")
        } else {
            ("update_wallet", "change")
        };
        let prompt = format!(
            "Re-enter database passphrase to {verb} wallet source {}",
            kassiber_pytext::repr_str(current_label)
        );
        if let Some(required) = auth(daemon, args, request_id, scope, &prompt)? {
            return Ok((Some(required), false));
        }
        if kind == "ui.wallets.delete" {
            Operation::Delete {
                profile_id,
                wallet,
                cascade,
            }
        } else {
            Operation::Update {
                profile_id,
                wallet,
                label,
                account: None,
                config,
                clear,
            }
        }
    };
    let payload = daemon.book.as_mut().expect("opened").wallets(operation)?;
    Ok((
        Some(with_request_id(
            success(kind, dict([("wallet", payload)])),
            request_id,
        )),
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_runtime::{
        fake::{FakeConfig, FakeStore},
        store::{BookStore, Selection},
        BuildInfo,
    };
    use std::sync::Arc;
    fn daemon(encrypted: bool) -> Daemon {
        let store = FakeStore::new(FakeConfig {
            encrypted,
            passphrase: Some("synthetic local passphrase".into()),
            ..Default::default()
        });
        let bootstrap = store.bootstrap(&Selection::default()).unwrap();
        Daemon::new(
            Arc::new(store),
            crate::DaemonConfig {
                build: BuildInfo {
                    version: "test",
                    channel: None,
                    commit: None,
                },
                bootstrap,
            },
            crate::output::Output::new(Box::new(std::io::sink())),
        )
    }
    #[test]
    fn wallet_mutations_require_fresh_encrypted_auth_and_exact_plaintext_ack() {
        let request_id = RequestId::Value(s("auth"));
        for (scope, key, ack) in [
            ("update_wallet", "plaintext_change_ack", "CHANGE LOCAL DATA"),
            ("delete_wallet", "plaintext_delete_ack", "DELETE LOCAL DATA"),
        ] {
            let mut encrypted = daemon(true);
            let missing = auth(
                &mut encrypted,
                &dict::<&str>([]),
                &request_id,
                scope,
                "prompt",
            )
            .unwrap()
            .unwrap();
            assert_eq!(get(&missing, "kind"), Some(&s("auth_required")));
            let wrong = dict([("auth_response", dict([("passphrase_secret", s("wrong"))]))]);
            let denied = auth(&mut encrypted, &wrong, &request_id, scope, "prompt")
                .unwrap()
                .unwrap();
            assert_eq!(
                get(get(&denied, "error").unwrap(), "code"),
                Some(&s("local_auth_denied"))
            );
            let correct = dict([(
                "auth_response",
                dict([("passphrase_secret", s("synthetic local passphrase"))]),
            )]);
            assert!(auth(&mut encrypted, &correct, &request_id, scope, "prompt")
                .unwrap()
                .is_none());
            // A previous successful verification never authorizes the next request.
            assert!(auth(
                &mut encrypted,
                &dict::<&str>([]),
                &request_id,
                scope,
                "prompt"
            )
            .unwrap()
            .is_some());
            let mut plain = daemon(false);
            assert!(auth(&mut plain, &dict::<&str>([]), &request_id, scope, "prompt").is_err());
            let response = dict([("auth_response", dict([(key, s(ack.to_lowercase()))]))]);
            assert!(auth(&mut plain, &response, &request_id, scope, "prompt").is_err());
            let response = dict([("auth_response", dict([(key, s(ack))]))]);
            assert!(auth(&mut plain, &response, &request_id, scope, "prompt")
                .unwrap()
                .is_none());
        }
    }
}
