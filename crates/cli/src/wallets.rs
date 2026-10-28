//! Wallet family registration. Unsupported wallet operations stay disabled.
use crate::argparse::Namespace;
use kassiber_envelope::AppError;
use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_wallets::{config, descriptors, material, Operation};
use serde_json::{json, Value as Json};
use std::io::{Read, Write};

pub fn execute(
    namespace: &Namespace,
    book: &mut dyn Book,
    stdin: &mut dyn Read,
    stderr: &mut dyn Write,
) -> Result<Value, AppError> {
    let (workspace_id, profile_id) =
        book.resolve_scope(namespace.get("workspace"), namespace.get("profile"))?;
    let wallet = namespace.text("wallet").unwrap_or_default().to_owned();
    let operation = match namespace.text("wallets_command").unwrap_or_default() {
        "import-csv" | "import-json" | "import-phoenix" => {
            return book.import_wallet_file(kassiber_wallets::file_import::Request {
                profile_id,
                wallet,
                file: namespace.text("file").unwrap_or_default().into(),
                format: match namespace.text("wallets_command") {
                    Some("import-json") => kassiber_wallets::file_import::Format::Json,
                    Some("import-phoenix") => kassiber_wallets::file_import::Format::Phoenix,
                    _ => kassiber_wallets::file_import::Format::Csv,
                },
            })
        }
        "list" => Operation::List { profile_id },
        "get" => Operation::Get { profile_id, wallet },
        "delete" => Operation::Delete {
            profile_id,
            wallet,
            cascade: namespace.get("cascade") == Some(&Value::Bool(true)),
        },
        "create" => Operation::Create {
            workspace_id,
            profile_id,
            label: namespace.text("label").unwrap_or_default().into(),
            kind: namespace.text("kind").unwrap_or_default().into(),
            account: namespace.text("account").map(str::to_owned),
            config: cli_config(namespace, false, stdin, stderr)?,
        },
        "update" => Operation::Update {
            profile_id,
            wallet,
            label: namespace.text("label").map(str::to_owned),
            account: namespace.text("account").map(str::to_owned),
            config: cli_config(namespace, true, stdin, stderr)?,
            clear: match namespace.get("clear") {
                Some(Value::List(v)) => v.iter().map(kassiber_envelope::py_str).collect(),
                _ => vec![],
            },
        },
        _ => {
            return book.derive_wallet(kassiber_wallets::Derive {
                profile_id,
                wallet,
                branch: namespace.text("branch").map(str::to_owned),
                start: namespace.get("start").cloned().unwrap_or(Value::from(0)),
                count: namespace.get("count").cloned(),
            })
        }
    };
    book.wallets(operation)
}

fn json_value(value: &Value) -> Result<Json, AppError> {
    serde_json::from_str(
        &kassiber_pyjson::dumps(value, &Default::default())
            .map_err(|_| AppError::new("validation", "Invalid wallet configuration"))?,
    )
    .map_err(|_| AppError::new("validation", "Invalid wallet configuration"))
}
fn read_file(path: &str, expand: bool) -> Result<String, AppError> {
    let expanded = if expand {
        if let Some(tail) = path.strip_prefix("~/") {
            std::env::var("HOME")
                .map(|h| format!("{h}/{tail}"))
                .unwrap_or_else(|_| path.into())
        } else {
            path.into()
        }
    } else {
        path.into()
    };
    std::fs::read_to_string(&expanded).map_err(|e| {
        let reason = match e.kind() {
            std::io::ErrorKind::NotFound => "No such file or directory",
            std::io::ErrorKind::PermissionDenied => "Permission denied",
            _ => "Could not read file",
        };
        AppError::new(
            "internal_error",
            format!(
                "[Errno {}] {reason}: {}",
                e.raw_os_error().unwrap_or(5),
                kassiber_pytext::repr_str(&expanded)
            ),
        )
    })
}
fn parse_json(raw: &str) -> Result<Json, AppError> {
    let value =
        kassiber_pytext::loads(raw).map_err(|e| AppError::new("internal_error", e.to_string()))?;
    json_value(&value)
}
fn secret(
    n: &Namespace,
    name: &str,
    label: &str,
    stdin: &mut dyn Read,
    stderr: &mut dyn Write,
) -> Result<Option<String>, AppError> {
    if let Some(value) = n.get(&format!("{name}_fd")).filter(|v| **v != Value::Null) {
        let fd = kassiber_envelope::py_str(value)
            .parse::<i32>()
            .map_err(|_| {
                AppError::new(
                    "invalid_passphrase",
                    "passphrase descriptor must be non-negative",
                )
            })?;
        let passphrase = kassiber_secrets::read_passphrase_from_fd(fd)
            .map_err(|e| AppError::new("invalid_passphrase", e.message))?;
        return Ok(Some(passphrase.expose().into()));
    }
    if n.get(&format!("{name}_stdin")) == Some(&Value::Bool(true)) {
        let mut text = String::new();
        stdin
            .read_to_string(&mut text)
            .map_err(|_| AppError::new("invalid_secret_input", "Could not read secret input"))?;
        if text.ends_with('\n') {
            text.pop();
        }
        if text.ends_with('\r') {
            text.pop();
        }
        if text.is_empty() {
            return Err(AppError::new(
                "invalid_secret_input",
                format!("{label} value read from stdin is empty"),
            ));
        }
        if text.contains('\0') {
            return Err(AppError::new(
                "invalid_secret_input",
                format!("{label} value contains NUL bytes"),
            ));
        }
        return Ok(Some(text));
    }
    if let Some(value) = n.text(name).filter(|s| !s.is_empty()) {
        let dashed = name.replace('_', "-");
        let _=writeln!(stderr,"warning: --{dashed} <value> exposes secrets in shell history; prefer --{dashed}-stdin or --{dashed}-fd FD.");
        return Ok(Some(value.into()));
    }
    Ok(None)
}
fn cli_config(
    n: &Namespace,
    update: bool,
    stdin: &mut dyn Read,
    stderr: &mut dyn Write,
) -> Result<Json, AppError> {
    let mut c = json!({});
    if !update {
        let active = ["descriptor", "change_descriptor", "sp_descriptor"]
            .into_iter()
            .filter(|name| n.get(&format!("{name}_stdin")) == Some(&Value::Bool(true)))
            .collect::<Vec<_>>();
        if active.len() > 1 {
            return Err(AppError::new(
                "invalid_secret_input",
                "only one --*-stdin option may be used per invocation",
            )
            .with_hint("Use --<name>-fd FD for additional secret inputs.")
            .with_details(dict([(
                "conflicting",
                Value::List(active.into_iter().map(s).collect()),
            )])));
        }
        for (name, label) in [
            ("descriptor", "Descriptor"),
            ("change_descriptor", "Change descriptor"),
            ("sp_descriptor", "Silent Payments descriptor"),
        ] {
            let text = secret(n, name, label, stdin, stderr)?;
            let text = match text {
                Some(s) if !s.is_empty() => Some(s),
                _ => n
                    .text(&format!("{name}_file"))
                    .map(|p| read_file(p, true))
                    .transpose()?,
            };
            if let Some(text) = text {
                let text = kassiber_pytext::strip(&text);
                if text.is_empty() && n.text(&format!("{name}_file")).is_some() {
                    return Err(AppError::new(
                        "app_error",
                        format!(
                            "{label} file '{}' is empty",
                            n.text(&format!("{name}_file")).unwrap_or_default()
                        ),
                    ));
                }
                if !text.is_empty() {
                    c[name] = json!(text);
                }
            }
        }
    }
    let secret_fields = c.clone();
    c = json!({});
    for raw in [
        n.text("config").map(str::to_owned),
        n.text("config_file")
            .map(|p| read_file(p, false))
            .transpose()?,
    ]
    .into_iter()
    .flatten()
    {
        let value = parse_json(&raw)?;
        let obj = value
            .as_object()
            .ok_or_else(|| AppError::new("validation", "Wallet config must be an object"))?;
        c.as_object_mut().unwrap().extend(obj.clone());
    }
    c.as_object_mut()
        .unwrap()
        .extend(secret_fields.as_object().unwrap().clone());
    if let Some(backend) = n.text("backend").filter(|s| !s.is_empty()) {
        c["backend"] = json!(kassiber_pytext::strip(backend).to_lowercase());
    }
    if !update {
        if let Some(text) = secret_fields.get("descriptor").and_then(Json::as_str) {
            if let Some(v) = material::bsms(text).map_err(descriptors::Error::into_app)? {
                c["descriptor"] = v["descriptor"].clone();
                if c.get("change_descriptor").is_none() {
                    if let Some(change) = v.get("change_descriptor") {
                        c["change_descriptor"] = change.clone();
                    }
                }
                c["descriptor_source"] = json!("bsms");
                c["synthesize_change"] = json!(false);
            }
        }
        if let Some(types) = n.get("script_type").filter(|v| **v != Value::Null) {
            let types = json_value(types)?;
            if types.as_array().is_some_and(|v| !v.is_empty()) {
                let descriptor=secret_fields.get("descriptor").and_then(Json::as_str).ok_or_else(||AppError::new("validation","--script-type requires a bare xpub via --descriptor/--descriptor-file/--descriptor-stdin"))?;
                let m = material::normalize(descriptor, &json!({"script_types":types}))
                    .map_err(descriptors::Error::into_app)?;
                if m.get("xpub").is_some() {
                    c.as_object_mut().unwrap().remove("descriptor");
                    c.as_object_mut().unwrap().remove("change_descriptor");
                    c["xpub"] = m["xpub"].clone();
                    c["script_types"] = m["script_types"].clone();
                }
            }
        }
        let mut addresses = config::addresses(c.get("addresses"))?;
        if let Some(value) = n.get("address").filter(|v| **v != Value::Null) {
            addresses.extend(config::addresses(Some(&json_value(value)?))?);
        }
        if !addresses.is_empty() {
            c["addresses"] = json!(config::addresses(Some(&json!(addresses)))?);
        }
    }
    for key in [
        "chain",
        "network",
        "birthday",
        "policy_asset",
        "source_file",
        "source_format",
        "sp_scan_mode",
        "sp_scan_start_height",
        "sp_scan_start_date",
    ] {
        if let Some(value) = n.get(key).filter(|v| **v != Value::Null && **v != s("")) {
            c[key] = json_value(value)?;
        }
    }
    if let Some(gap) = n.get("gap_limit").filter(|v| **v != Value::Null) {
        let gap = json_value(gap)?;
        let value = gap.as_i64().unwrap_or(i64::MAX);
        if !(1..=5000).contains(&value) {
            return Err(AppError::new(
                if update { "validation" } else { "app_error" },
                if value <= 0 {
                    "Descriptor gap limit must be positive"
                } else {
                    "Descriptor gap limit must be 5000 or lower"
                },
            ));
        }
        c["gap_limit"] = gap;
    }
    for key in [
        "sp_full_history",
        "sp_acknowledge_full_history_warning",
        "sp_acknowledge_server_warning",
    ] {
        if n.get(key) == Some(&Value::Bool(true)) {
            c[key] = json!(true);
        }
    }
    if let Some(asset) = n.text("policy_asset").filter(|s| !s.is_empty()) {
        c["policy_asset"] = json!(config::asset_code(asset));
    }
    if let Some(file) = n.text("source_file") {
        c["source_file"] = json!(std::path::absolute(file)
            .map_err(|_| AppError::new("internal_error", "Could not resolve source file"))?
            .to_string_lossy());
    }
    if let Some(store) = n.text("store_id") {
        let store = kassiber_pytext::strip(store);
        if store.is_empty() {
            return Err(AppError::new(
                "validation",
                "BTCPay store id cannot be empty",
            ));
        }
        c["store_id"] = json!(store);
        c["sync_source"] = json!("btcpay");
    }
    if let Some(method) = n.text("payment_method_id") {
        c["payment_method_id"] = json!(config::payment_method(method)?);
        c["sync_source"] = json!("btcpay");
    }
    if !update {
        if n.text("store_id").is_some() || n.text("payment_method_id").is_some() {
            let method = c
                .get("payment_method_id")
                .and_then(Json::as_str)
                .unwrap_or("BTC-CHAIN");
            if !["BTC-CHAIN", "LBTC-CHAIN"].contains(&method) {
                let hint = if method.ends_with("-LN") || method.ends_with("-LNURL") {
                    "Lightning payments cannot be imported as wallet history. Keep this method as invoice provenance and add the Lightning node or wallet that receives the payments as its own connection."
                } else {
                    "Use an on-chain method such as BTC-CHAIN or LBTC-CHAIN, or keep this method as invoice provenance only."
                };
                return Err(AppError::new("validation", format!("BTCPay payment method '{method}' is not available through wallet-history sync")).with_hint(hint));
            }
            c["payment_method_id"] = json!(method);
        }
        if let Some(routes) = c.get("btcpay_provenance") {
            c["btcpay_provenance"] = config::provenance(routes)?;
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs(fields: &[(&str, Value)]) -> Namespace {
        let mut n = Namespace::default();
        for (key, value) in fields {
            n.set(key, value.clone());
        }
        n
    }
    fn descriptor() -> String {
        let vectors: Json = serde_json::from_str(include_str!(
            "../../wallets/tests/vectors/python_descriptors.json"
        ))
        .unwrap();
        vectors["configs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "kind-descriptor")
            .unwrap()["config"]["descriptor"]
            .as_str()
            .unwrap()
            .into()
    }
    #[test]
    fn cli_config_ports_public_xpub_sets_bsms_and_safe_stdin() {
        let descriptor = descriptor();
        let key = descriptor.strip_prefix("wpkh(").unwrap();
        let xpub = key.rsplit(']').next().unwrap().split('/').next().unwrap();
        let n = inputs(&[
            ("descriptor", s(xpub)),
            ("script_type", Value::List(vec![s("p2wpkh"), s("p2tr")])),
        ]);
        let c = cli_config(&n, false, &mut &b""[..], &mut Vec::new()).unwrap();
        assert_eq!(c["xpub"], xpub);
        assert_eq!(c["script_types"], json!(["p2tr", "p2wpkh"]));
        assert!(c.get("descriptor").is_none());
        let bsms = format!(
            "BSMS 1.0\n{}\n/0/*,/1/*\nfirst-address",
            descriptor.replace("/0/*", "/**")
        );
        let n = inputs(&[("descriptor_stdin", Value::Bool(true))]);
        let c = cli_config(&n, false, &mut bsms.as_bytes(), &mut Vec::new()).unwrap();
        assert_eq!(c["descriptor"], descriptor);
        assert_eq!(c["change_descriptor"], descriptor.replace("/0/*", "/1/*"));
        assert_eq!(c["descriptor_source"], "bsms");
        assert_eq!(c["synthesize_change"], false);
        let n = inputs(&[("descriptor", s(descriptor.as_str()))]);
        assert_eq!(
            cli_config(&n, false, &mut &b""[..], &mut Vec::new()).unwrap()["descriptor"],
            descriptor
        );
        let n = inputs(&[("script_type", Value::List(vec![s("p2wpkh")]))]);
        assert_eq!(
            cli_config(&n, false, &mut &b""[..], &mut Vec::new())
                .unwrap_err()
                .code,
            "validation"
        );
    }
    #[test]
    fn cli_config_requires_exclusive_stdin_and_onchain_btcpay_creation() {
        let n = inputs(&[
            ("descriptor_stdin", Value::Bool(true)),
            ("change_descriptor_stdin", Value::Bool(true)),
        ]);
        assert_eq!(
            cli_config(&n, false, &mut &b""[..], &mut Vec::new())
                .unwrap_err()
                .code,
            "invalid_secret_input"
        );
        let n = inputs(&[("descriptor_stdin", Value::Bool(true))]);
        assert_eq!(
            cli_config(&n, false, &mut &b"\n"[..], &mut Vec::new())
                .unwrap_err()
                .code,
            "invalid_secret_input"
        );
        let n = inputs(&[("payment_method_id", s("btc-ln"))]);
        assert_eq!(
            cli_config(&n, false, &mut &b""[..], &mut Vec::new())
                .unwrap_err()
                .code,
            "validation"
        );
        let n = inputs(&[(
            "config",
            s(r#"{"descriptor":"raw-authored","policy_asset":"authored-asset"}"#),
        )]);
        let c = cli_config(&n, false, &mut &b""[..], &mut Vec::new()).unwrap();
        assert_eq!(c["descriptor"], "raw-authored");
        assert_eq!(c["policy_asset"], "authored-asset");
    }
}
