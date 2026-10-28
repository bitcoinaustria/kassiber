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
    let host = url
        .split_once("://")
        .filter(|(s, _)| ["http", "https"].contains(s))
        .map(|(_, r)| {
            r.split(['/', '?', '#'])
                .next()
                .unwrap_or_default()
                .rsplit('@')
                .next()
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or_default()
    };
    if host.to_lowercase() != "localhost"
        && !host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    {
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
                let _ = reference;
                if text(&common, "kind").trim().to_lowercase() != "bitcoinrpc" {
                    return Err(AppError::new(
                        "validation",
                        "credential_ref is only supported for Bitcoin Core RPC backends",
                    ));
                }
                return Err(AppError::new(
                    "not_implemented",
                    "Detected Core credentials are not implemented in the Rust backend yet",
                ));
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
