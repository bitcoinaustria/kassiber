use super::*;
use kassiber_envelope::type_name;
fn optional_string(args: &Value, key: &str) -> Result<Option<String>> {
    match optional(args, key) {
        None => Ok(None),
        Some(v @ Value::Str(_)) => Ok(string(v)),
        Some(v) => Err(
            AppError::new("validation", format!("{key} must be a string"))
                .with_details(dict([("type", s(type_name(v)))])),
        ),
    }
}
fn required(args: &Value, key: &str, label: &str) -> Result<String> {
    optional_string(args, key)?.ok_or_else(|| {
        AppError::new("validation", format!("{label} is required."))
            .with_hint(format!("Enter {}.", label.to_lowercase()))
    })
}
fn common(args: &Value) -> Result<Value> {
    let mut result = dict::<&str>([]);
    let clear = match value(args, "clear") {
        Value::Null => Value::List(vec![]),
        Value::List(v) if v.iter().all(|v| matches!(v, Value::Str(_))) => {
            Value::List(v.iter().filter_map(string).map(s).collect())
        }
        v => {
            return Err(
                AppError::new("validation", "clear must be a list of backend field names")
                    .with_details(dict([("type", s(type_name(v)))])),
            )
        }
    };
    for k in [
        "kind",
        "url",
        "chain",
        "network",
        "auth_header",
        "token",
        "tor_proxy",
        "notes",
    ] {
        replace(
            &mut result,
            k,
            optional_string(args, k)?.map_or(Value::Null, s),
        );
    }
    let mut config = match value(args, "config") {
        Value::Null => dict::<&str>([]),
        v @ Value::Dict(_) => v.clone(),
        v => {
            return Err(AppError::new("validation", "config must be an object")
                .with_details(dict([("type", s(type_name(v)))])))
        }
    };
    if let Some(v) = optional(args, "silent_payments") {
        if !matches!(v, Value::Bool(_)) {
            return Err(
                AppError::new("validation", "silent_payments must be a boolean")
                    .with_details(dict([("type", s(type_name(v)))])),
            );
        }
        replace(&mut config, "silent_payments", v.clone());
    }
    for k in ["silent_payment_scan_file", "silent_payment_scan_path"] {
        if optional(args, k).is_some() {
            replace(
                &mut config,
                k,
                optional_string(args, k)?.map_or(Value::Null, s),
            );
        }
    }
    replace(
        &mut result,
        "config",
        if truthy(&config) { config } else { Value::Null },
    );
    replace(&mut result, "clear", clear);
    for k in ["batch_size", "timeout"] {
        let v = value(args, k);
        if !matches!(v, Value::Null | Value::Int(_) | Value::Bool(_)) {
            return Err(
                AppError::new("validation", format!("{k} must be an integer"))
                    .with_details(dict([("type", s(type_name(v)))])),
            );
        }
        replace(&mut result, k, v.clone());
    }
    Ok(result)
}
fn resolve(config: &Config, name: &str) -> Result<Value> {
    let n = kassiber_pytext::strip(name).to_lowercase();
    config.view.get(&n).cloned().ok_or_else(|| {
        AppError::new(
            "app_error",
            format!("Backend '{n}' is not configured in {}", config.env_file),
        )
    })
}
fn promote(conn: &Connection, config: &Config, name: &str, sources: &dyn Sources) -> Result<()> {
    let name = kassiber_pytext::strip(name).to_lowercase();
    if names(conn)?.contains(&name) {
        return Ok(());
    }
    if let Some(b) = config.seed.get(&name) {
        let mut args = raw_backend(b);
        let (extra, _) = normalize_config(&args, false)?;
        replace(&mut args, "config", extra);
        for k in ["batch_size", "timeout"] {
            if let Some(v) = string(value(&args, k)) {
                replace(
                    &mut args,
                    k,
                    Value::Int(kassiber_pytext::parse_int(&v).map_err(|_| {
                        AppError::new("app_error", format!("Invalid integer value: {v}"))
                    })?),
                );
            }
        }
        create(conn, &args, sources)?;
    }
    Ok(())
}
fn cookie_policy(kind: &str, url: &str, config: &Value) -> Result<()> {
    if kind.trim().to_lowercase() != "bitcoinrpc" {
        return Ok(());
    }
    let Some(cookie) =
        string(value(config, "cookiefile")).or_else(|| string(value(config, "cookie_file")))
    else {
        return Ok(());
    };
    if !super::core_credentials::loopback(url) {
        return Err(AppError::new("validation","Bitcoin Core cookie-file credentials can only be used with a loopback RPC URL").with_hint("Use username/password auth for remote Core RPC, or point the cookie-file backend at localhost."));
    }
    let context = kassiber_paths::Context::from_env().map_err(storage_error)?;
    let path = context
        .expand_user(std::path::Path::new(&cookie))
        .map_err(storage_error)?;
    let home = context
        .expand_user(std::path::Path::new("~/.bitcoin"))
        .map_err(storage_error)?;
    let path = kassiber_paths::canonicalize_allow_missing(&path).map_err(storage_error)?;
    let home = kassiber_paths::canonicalize_allow_missing(&home).map_err(storage_error)?;
    if path.file_name().is_none_or(|n| n != ".cookie") || !path.starts_with(home) {
        return Err(AppError::new("validation","Desktop Bitcoin Core cookie-file credentials must point at a default .bitcoin cookie file").with_hint("Use a cookie path under ~/.bitcoin ending in .cookie, or use username/password auth for non-standard Core data directories."));
    }
    Ok(())
}
fn capabilities(kind: &str) -> Value {
    dict(
        [
            "nodeSnapshot",
            "routingProfitability",
            "channelBalances",
            "channelLifecycle",
            "forwardEvents",
            "invoiceActivity",
            "paymentActivity",
            "onchainBalance",
        ]
        .map(|k| (k, Value::Bool(["coreln", "lnd"].contains(&kind)))),
    )
}
fn summary(conn: &Connection, config: &Config, count: usize, mode: bool) -> Result<Value> {
    let mut result = dict([
        ("count", Value::Int(count.into())),
        (
            "default_backend",
            if config.default_backend.is_empty() {
                Value::Null
            } else {
                s(&config.default_backend)
            },
        ),
    ]);
    if mode {
        replace(
            &mut result,
            "bootstrap_mode",
            s(if manual_mode(conn)? {
                "manual"
            } else {
                "public"
            }),
        );
    }
    Ok(result)
}
/// `_string_or_empty(value)`: only strings count, stripped.
fn string_or_empty(v: &Value) -> String {
    match v {
        Value::Str(_) => string(v).unwrap_or_default(),
        _ => String::new(),
    }
}
/// The backend name `_wallet_backend_summary` reports for one wallet: its
/// explicit backend, or the default for descriptor and address syncs.
fn wallet_backend(kind: &str, config: &Value, default_backend: &str) -> String {
    let explicit = string_or_empty(value(config, "backend"));
    if !explicit.is_empty() {
        return explicit;
    }
    let descriptor = string(value(config, "descriptor")).is_some()
        || (string(value(config, "xpub")).is_some() && truthy(value(config, "script_types")));
    let uses_default = if string_or_empty(value(config, "sync_source")) == "btcpay"
        || !string_or_empty(value(config, "source_file")).is_empty()
            && !string_or_empty(value(config, "source_format")).is_empty()
        || string(value(config, "sp_descriptor")).is_some() && kind == "silent-payment"
    {
        false
    } else {
        descriptor && ["descriptor", "xpub", "address"].contains(&kind)
            || truthy(value(config, "addresses")) && kind == "address"
    };
    if uses_default {
        string_or_empty(&s(default_backend))
    } else {
        String::new()
    }
}
/// `build_backends_list_snapshot`: only backends the active profile's wallets
/// use, reduced to an allowlist. It reads configuration and never probes.
fn list(conn: &Connection, config: &Config) -> Result<Value> {
    const ALLOWED: &[&str] = &[
        "name",
        "kind",
        "chain",
        "network",
        "batch_size",
        "timeout",
        "insecure",
        "has_auth_header",
        "has_token",
        "has_cookiefile",
        "has_username",
        "has_password",
    ];
    let context = crate::context::snapshot(conn)?;
    let label = |key: &str| match value(&context, key) {
        Value::Str(v) if v.code_points().next().is_some() => Value::Str(v.clone()),
        _ => Value::Null,
    };
    let default_backend = config.default_backend.clone();
    let mut referenced = BTreeSet::new();
    if !text(&context, "workspace_id").is_empty() && !text(&context, "profile_id").is_empty() {
        let profile = rows(
            conn,
            "SELECT id FROM profiles WHERE id=?",
            [text(&context, "profile_id")],
        )?;
        if let Some(profile) = profile.first() {
            for wallet in rows(
                conn,
                "SELECT kind,config_json FROM wallets WHERE profile_id=? ORDER BY label ASC",
                [text(profile, "id")],
            )? {
                let raw = match value(&wallet, "config_json") {
                    Value::Str(v) if v.code_points().next().is_some() => {
                        v.as_str().map(str::to_owned)
                    }
                    Value::Null | Value::Str(_) => Some("{}".to_owned()),
                    _ => None,
                };
                let config = raw
                    .and_then(|raw| kassiber_pytext::loads(&raw).ok())
                    .filter(|v| matches!(v, Value::Dict(_)))
                    .unwrap_or_else(|| dict::<&str>([]));
                let name = wallet_backend(&text(&wallet, "kind"), &config, &default_backend);
                if !name.is_empty() {
                    referenced.insert(name);
                }
            }
        }
    }
    let mut backends = vec![];
    for (name, b) in &config.view {
        if !referenced.contains(name) || !matches!(b, Value::Dict(_)) {
            continue;
        }
        let mut row = dict([("name", s(name))]);
        for k in [
            "kind",
            "chain",
            "network",
            "url",
            "batch_size",
            "source",
            "auth_header",
            "token",
            "cookiefile",
            "username",
            "password",
            "config_json",
        ] {
            replace(&mut row, k, get(b, k).cloned().unwrap_or_else(|| s("")));
        }
        let mut safe = output::redact(&row)?;
        if let Value::Dict(fields) = &mut safe {
            fields.retain(|(k, _)| k.as_str().is_some_and(|k| ALLOWED.contains(&k)));
        }
        replace(
            &mut safe,
            "has_url",
            Value::Bool(!string_or_empty(value(b, "url")).is_empty()),
        );
        replace(
            &mut safe,
            "is_default",
            Value::Bool(*name == default_backend),
        );
        let kind = string(value(b, "kind")).map_or_else(String::new, |_| py_str(value(b, "kind")));
        if ["coreln", "lnd", "nwc"].contains(&kind.as_str()) {
            replace(&mut safe, "lightningCapabilities", capabilities(&kind));
        }
        backends.push(safe);
    }
    let count = backends.len();
    Ok(dict([
        ("backends", Value::List(backends)),
        (
            "summary",
            dict([
                ("workspace", label("workspace_label")),
                ("profile", label("profile_label")),
                ("count", Value::Int(count.into())),
                (
                    "default_backend",
                    if referenced.contains(&default_backend) {
                        s(&default_backend)
                    } else {
                        Value::Null
                    },
                ),
                ("scope", s("active_profile")),
            ]),
        ),
    ]))
}
pub(super) fn execute(
    conn: &Connection,
    op: &str,
    args: &Value,
    config: &mut Config,
    sources: &dyn Sources,
) -> Result<Value> {
    match op {
        "ui.backends.options" | "ui.backends.settings.list" => {
            let Value::List(mut backends) = output::list(config)? else {
                unreachable!()
            };
            for b in &mut backends {
                let name = text(b, "name");
                let kind = text(b, "kind");
                if op == "ui.backends.options" {
                    let allowed = [
                        "name",
                        "display_name",
                        "kind",
                        "chain",
                        "network",
                        "batch_size",
                        "timeout",
                        "insecure",
                        "has_auth_header",
                        "has_token",
                        "has_certificate",
                        "has_cookiefile",
                        "has_username",
                        "has_password",
                        "silent_payments",
                    ];
                    let has_url = truthy(value(b, "url"));
                    let default = text(b, "default") == "yes";
                    if let Value::Dict(fields) = b {
                        fields.retain(|(k, _)| k.as_str().is_some_and(|k| allowed.contains(&k)));
                    }
                    replace(b, "has_url", Value::Bool(has_url));
                    replace(b, "is_default", Value::Bool(default));
                } else {
                    replace(b, "is_default", Value::Bool(name == config.default_backend));
                    replace(b, "wallet_refs", wallet_refs(conn, &name)?);
                    if ["electrum", "esplora", "liquid-esplora", "bitcoinrpc"]
                        .contains(&kind.as_str())
                    {
                        if let Some(certificate) = config
                            .view
                            .get(&name)
                            .and_then(|b| string(value(b, "certificate")))
                        {
                            replace(b, "certificate", s(certificate));
                        }
                    }
                }
                if ["coreln", "lnd", "nwc"].contains(&kind.as_str()) {
                    replace(b, "lightningCapabilities", capabilities(&kind));
                }
            }
            let mut result = dict([
                ("backends", Value::List(backends)),
                ("summary", summary(conn, config, config.view.len(), true)?),
            ]);
            if op == "ui.backends.options" {
                replace(
                    &mut result,
                    "suggestions",
                    Value::List(
                        [
                            (
                                "esplora",
                                "mempool",
                                "Built-in mempool.space Bitcoin backend",
                                "bitcoin",
                                "mainnet",
                            ),
                            (
                                "electrum",
                                "liquid",
                                "Built-in BullBitcoin Liquid Electrum backend",
                                "liquid",
                                "liquidv1",
                            ),
                            (
                                "electrum",
                                "liquid-blockstream",
                                "Built-in Blockstream Liquid Electrum backend",
                                "liquid",
                                "liquidv1",
                            ),
                        ]
                        .map(|(kind, name, label, chain, network)| {
                            dict([
                                ("kind", s(kind)),
                                ("name", s(name)),
                                ("label", s(label)),
                                ("chain", s(chain)),
                                ("network", s(network)),
                            ])
                        })
                        .to_vec(),
                    ),
                );
            }
            Ok(result)
        }
        "ui.backends.list" => list(conn, config),
        "ui.backends.public_defaults" => {
            let mut backends = vec![];
            let names = config.seed_order.clone();
            for name in names {
                let b = config
                    .view
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| raw_backend(&config.seed[&name]));
                if !["electrum", "esplora", "liquid-esplora"].contains(&text(&b, "kind").as_str())
                    || !truthy(value(&b, "url"))
                {
                    continue;
                }
                let mut row = dict(
                    ["name", "kind", "chain", "network"]
                        .map(|k| (k, get(&b, k).cloned().unwrap_or_else(|| s("")))),
                );
                replace(&mut row, "url", s(output::url(value(&b, "url"), false)?));
                replace(&mut row, "source", value(&b, "source").clone());
                replace(
                    &mut row,
                    "is_default",
                    Value::Bool(name == config.default_backend),
                );
                replace(
                    &mut row,
                    "liquid_opt_in",
                    Value::Bool(
                        ["liquid", "liquid-blockstream"].contains(&name.as_str())
                            && !config.dotenv.contains(&name)
                            && !config.env_names.contains(&name),
                    ),
                );
                backends.push(row);
            }
            let n = backends.len();
            Ok(dict([
                ("backends", Value::List(backends)),
                ("summary", summary(conn, config, n, false)?),
            ]))
        }
        "ui.backends.create" => {
            let mut common = common(args)?;
            replace(
                &mut common,
                "name",
                s(required(args, "name", "Backend name")?),
            );
            for k in ["kind", "url"] {
                if matches!(value(&common, k), Value::Null) {
                    replace(&mut common, k, s(""));
                }
            }
            if let Some(reference) = optional_string(args, "credential_ref")? {
                if text(&common, "kind").trim().to_lowercase() != "bitcoinrpc" {
                    return Err(AppError::new(
                        "validation",
                        "credential_ref is only supported for Bitcoin Core RPC backends",
                    ));
                }
                let home = kassiber_paths::Context::from_env()
                    .map_err(storage_error)?
                    .home;
                let detected = crate::core_credentials::resolve(&home, &reference)?;
                let url = text(&detected, "url");
                cookie_policy("bitcoinrpc", &url, &detected)?;
                merge_detected(&mut common, &detected)?;
            }
            cookie_policy(
                &text(&common, "kind"),
                &text(&common, "url"),
                value(&common, "config"),
            )?;
            output::redact(&create(conn, &common, sources)?)
        }
        "ui.backends.update" => {
            let name = required(args, "name", "Backend name")?;
            let mut common = common(args)?;
            replace(&mut common, "name", s(&name));
            let current = resolve(config, &name)?;
            let kind = string(value(&common, "kind")).unwrap_or_else(|| text(&current, "kind"));
            let url = string(value(&common, "url")).unwrap_or_else(|| text(&current, "url"));
            let mut effective = current.clone();
            if let Value::List(clear) = value(&common, "clear") {
                for k in clear {
                    remove(&mut effective, &py_str(k));
                }
            }
            if let Value::Dict(fields) = value(&common, "config") {
                for (k, v) in fields {
                    replace(&mut effective, k.as_str().unwrap(), v.clone());
                }
            }
            cookie_policy(&kind, &url, &effective)?;
            promote(conn, config, &name, sources)?;
            output::redact(&update(conn, &common, sources)?)
        }
        "ui.backends.delete" | "ui.backends.set_default" => {
            let name = required(args, "name", "Backend name")?;
            if op == "ui.backends.set_default" {
                resolve(config, &name)?;
            }
            promote(conn, config, &name, sources)?;
            let args = dict([("name", s(name))]);
            if op == "ui.backends.delete" {
                delete(conn, &args, config)
            } else {
                set_default(conn, &args, config)
            }
        }
        _ => Err(AppError::new(
            "not_implemented",
            "backend desktop operation is not implemented",
        )),
    }
}

fn merge_detected(common: &mut Value, detected: &Value) -> Result<()> {
    let mut config = match value(common, "config") {
        Value::Dict(_) => value(common, "config").clone(),
        _ => dict::<&str>([]),
    };
    let cookie = string(value(detected, "cookiefile"));
    let remove: &[&str] = if let Some(cookie) = cookie {
        replace(&mut config, "cookiefile", s(cookie));
        &["username", "password"]
    } else {
        let username =
            string(value(detected, "username")).ok_or_else(crate::core_credentials::unavailable)?;
        let password =
            string(value(detected, "password")).ok_or_else(crate::core_credentials::unavailable)?;
        replace(&mut config, "username", s(username));
        replace(&mut config, "password", s(password));
        &["cookiefile"]
    };
    if let Value::Dict(fields) = &mut config {
        fields.retain(|(k, _)| !remove.iter().any(|r| *k == (*r).into()));
    }
    replace(common, "config", config);
    replace(common, "chain", s("bitcoin"));
    replace(common, "network", get(detected, "network").unwrap().clone());
    replace(common, "kind", s("bitcoinrpc"));
    replace(common, "url", get(detected, "url").unwrap().clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn core_reference_merge_replaces_endpoint_and_conflicting_auth() {
        let mut common = dict([
            ("kind", s("bitcoinrpc")),
            ("url", s("https://example.test")),
            (
                "config",
                dict([
                    ("username", s("wrong")),
                    ("password", s("wrong")),
                    ("certificate", s("/synthetic/ca")),
                ]),
            ),
        ]);
        let cookie = dict([
            ("cookiefile", s("/synthetic/.cookie")),
            ("url", s("http://127.0.0.1:8332")),
            ("network", s("main")),
        ]);
        merge_detected(&mut common, &cookie).unwrap();
        let config = value(&common, "config");
        assert!(get(config, "username").is_none());
        assert!(get(config, "password").is_none());
        assert_eq!(get(config, "certificate"), Some(&s("/synthetic/ca")));
        assert_eq!(get(&common, "url"), get(&cookie, "url"));
        let basic = dict([
            ("username", s("alice")),
            ("password", s("synthetic-password")),
            ("url", s("http://127.0.0.1:18443")),
            ("network", s("regtest")),
        ]);
        merge_detected(&mut common, &basic).unwrap();
        assert!(get(value(&common, "config"), "cookiefile").is_none());
        assert_eq!(
            get(value(&common, "config"), "password"),
            get(&basic, "password")
        );
        assert_eq!(get(&common, "network"), Some(&s("regtest")));
        assert_eq!(get(&common, "url"), get(&basic, "url"));
    }
}
