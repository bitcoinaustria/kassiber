//! Explicit form probes. This service never runs on construction, display,
//! saved configuration, launch or a timer. The daemon boundary supplies a
//! non-persistable grant bound to the exact kind, arguments and open book.
use super::*;
use kassiber_net::Gate;
use serde_json::json;

pub const KINDS: &[&str] = &[
    "ui.backends.bitcoinrpc.test",
    "ui.backends.btcpay.test",
    "ui.backends.detect_core",
    "ui.backends.electrum.test",
    "ui.backends.http.test",
];
/// Preserve Python's observable JSON key order without enabling a workspace-wide
/// serde_json feature (which would change unrelated service serialization).
pub fn python_payload(kind: &str, value: &Value) -> kassiber_pyjson::Value {
    fn convert(value: &Value, keys: &[&str]) -> kassiber_pyjson::Value {
        use kassiber_pyjson::Value as Py;
        if let Some(object) = value.as_object() {
            let mut result = Vec::new();
            for key in keys
                .iter()
                .copied()
                .chain(object.keys().map(String::as_str))
            {
                if result
                    .iter()
                    .any(|(k, _)| *k == kassiber_pyjson::Key::from(key))
                {
                    continue;
                }
                if let Some(value) = object.get(key) {
                    let nested = match key {
                        "error" => &["code", "message", "hint", "retryable"][..],
                        "wallet_rpc" => &["available", "loaded_wallet_count", "error"][..],
                        "block_filters" => &["available", "error"][..],
                        _ => &[],
                    };
                    result.push((key.into(), convert(value, nested)));
                }
            }
            Py::Dict(result)
        } else if let Some(array) = value.as_array() {
            Py::List(
                array
                    .iter()
                    .map(|v| {
                        convert(
                            v,
                            &[
                                "url",
                                "chain",
                                "network",
                                "auth_source",
                                "credential_source",
                                "blocks",
                                "headers",
                                "peers",
                                "status",
                                "pruned",
                                "pruneheight",
                                "earliest_retained_at",
                                "ibd",
                                "wallet_rpc",
                                "block_filters",
                                "warnings",
                                "credential_ref",
                                "cookiefile",
                            ],
                        )
                    })
                    .collect(),
            )
        } else {
            details(value.clone())
        }
    }
    let keys = match kind {
        "ui.backends.http.test" => &["ok", "url", "status", "logs"][..],
        "ui.backends.electrum.test" => &["ok", "url", "trust_self_signed", "logs"][..],
        "ui.backends.btcpay.test" => &["backend", "ok", "stores_seen"][..],
        "ui.backends.bitcoinrpc.test" => &[
            "reachable",
            "status",
            "chain",
            "network",
            "blocks",
            "headers",
            "peers",
            "pruned",
            "pruneheight",
            "earliest_retained_at",
            "rescan_start_at",
            "birthday_covered",
            "rescan_possible",
            "version",
            "ibd",
            "wallet_rpc",
            "block_filters",
            "warnings",
            "error",
        ][..],
        _ => &[],
    };
    convert(value, keys)
}
pub struct ProbeConsent {
    kind: String,
    args: Value,
    book: String,
}
impl ProbeConsent {
    /// Trusted explicit daemon-action boundary only. Never call from a read,
    /// source listing or a generic refresh/background dispatcher.
    pub fn explicit(kind: &str, args: &Value, book: &str) -> Self {
        Self {
            kind: kind.into(),
            args: args.clone(),
            book: book.into(),
        }
    }
}
pub struct Probes {
    gate: Gate,
    version: String,
}
impl Probes {
    pub fn new(gate: Gate, version: &str) -> Self {
        Self {
            gate,
            version: version.into(),
        }
    }
    pub fn run(
        &self,
        kind: &str,
        args: &Value,
        book: &str,
        saved: Option<Value>,
        consent: Option<&ProbeConsent>,
    ) -> Result<Value> {
        if !consent.is_some_and(|c| c.kind == kind && c.args == *args && c.book == book) {
            return Err(AppError::new(
                "interaction_required",
                "Network action requires current consent",
            ));
        }
        validate_args(kind, args)?;
        match kind {
            "ui.backends.btcpay.test" => {
                super::btcpay::probe(&self.gate, &self.version, args, saved)
            }
            "ui.backends.http.test" => self.http(args),
            "ui.backends.electrum.test" => self.electrum(args),
            "ui.backends.bitcoinrpc.test" => self.core(args, saved),
            "ui.backends.detect_core" => self.detect(),
            _ => Err(AppError::new(
                "not_implemented",
                "Backend probe is not implemented in the Rust backend yet",
            )),
        }
    }
    fn detect(&self) -> Result<Value> {
        let mut found = Vec::new();
        for config in detection::candidates(&detection::bitcoin_dir()?)? {
            let result = (|| {
                let backend = Backend::new(&config, false)?;
                let consent = Consent::explicit_trigger(backend.request());
                let deadline = Instant::now() + Duration::from_secs(10);
                let client = CoreClient::new(backend, self.gate.clone(), &self.version);
                core_probe(&client, &consent, deadline, 0)
            })();
            let Ok(mut probe) = result else {
                continue;
            };
            for key in [
                "reachable",
                "rescan_start_at",
                "birthday_covered",
                "rescan_possible",
                "version",
            ] {
                probe.as_object_mut().unwrap().remove(key);
            }
            for key in ["url", "auth_source", "credential_source", "cookiefile"] {
                if let Some(v) = config.get(key) {
                    probe[key] = v.clone();
                }
            }
            probe["credential_ref"] = json!(detection::credential_ref(&config));
            found.push(probe);
        }
        Ok(json!({"candidates":found}))
    }
    fn http(&self, args: &Value) -> Result<Value> {
        let url = required(args, "url", "HTTP backend URL")?;
        if !url.to_ascii_lowercase().starts_with("http://")
            && !url.to_ascii_lowercase().starts_with("https://")
        {
            return Err(AppError::new(
                "validation",
                "HTTP backend URL must start with http:// or https://",
            ));
        }
        let timeout = timeout(args);
        let proxy = optional(args, "proxy");
        let certificate = optional(args, "certificate");
        let insecure = args
            .get("insecure")
            .map(config::boolean)
            .transpose()?
            .unwrap_or(false);
        let mut logs = vec![
            format!("$ curl -fsS -L --max-time {timeout} -H 'Accept: application/json' {url}"),
            format!("> GET {url}"),
            proxy
                .map(|p| format!("Proxy: {p}."))
                .unwrap_or_else(|| "Proxy: disabled.".into()),
        ];
        logs.push(
            if insecure {
                "TLS certificate verification: disabled for this backend test."
            } else if certificate.is_some() {
                "TLS certificate verification: custom CA bundle."
            } else {
                "TLS certificate verification: system trust store."
            }
            .into(),
        );
        let config = json!({"name":"candidate","url":url,"timeout":timeout,"tor_proxy":proxy,"certificate":certificate,"insecure":insecure});
        let backend = match Backend::new(&config, false) {
            Ok(v) => v,
            Err(e) if e.code == "backend_tls_config_invalid" => {
                logs.push(format!("TLS configuration failed: {e}"));
                return Ok(json!({"ok":false,"url":url,"logs":logs}));
            }
            Err(e) => return Err(e),
        };
        let consent = Consent::explicit_trigger(backend.request());
        let client = HttpClient::new(backend, self.gate.clone(), &self.version);
        match probe_deadline(timeout).and_then(|deadline| client.probe(Some(&consent), deadline)) {
            Ok(response) => {
                logs.push(
                    format!("< HTTP {} {}", response.status, response.reason)
                        .trim_end()
                        .into(),
                );
                logs.push(format!(
                    "< content-type: {}",
                    response.header("content-type").unwrap_or("unknown")
                ));
                logs.push(format!(
                    "< body: {} bytes sampled",
                    response.body.len().min(4096)
                ));
                let mut result =
                    json!({"ok":(200..400).contains(&response.status),"url":url,"logs":logs});
                if response.status < 400 {
                    result["status"] = json!(response.status);
                }
                Ok(result)
            }
            Err(error) => {
                logs.push(format!("< connection failed: {error}"));
                Ok(json!({"ok":false,"url":url,"logs":logs}))
            }
        }
    }
    fn electrum(&self, args: &Value) -> Result<Value> {
        let url = required(args, "url", "Electrum endpoint URL")?;
        let proxy = optional(args, "proxy");
        let certificate = optional(args, "certificate");
        let insecure = args.get("trust_self_signed") == Some(&json!(true));
        let timeout = timeout(args);
        let mut logs = vec![format!("Opening Electrum connection to {url}")];
        logs.push(if insecure {
            "Certificate verification: self-signed certificate trusted for this test.".into()
        } else if let Some(cert) = certificate {
            format!("Certificate verification: pinned certificate {cert}.")
        } else {
            "Certificate verification: system trust store.".into()
        });
        logs.push(
            proxy
                .map(|p| format!("Proxy: {p}."))
                .unwrap_or_else(|| "Proxy: disabled.".into()),
        );
        let result = (|| {
            let backend = Backend::new(
                &json!({"name":"candidate","url":url,"timeout":timeout,"tor_proxy":proxy,"certificate":certificate,"insecure":insecure}),
                true,
            )?;
            let consent = Consent::explicit_trigger(backend.request());
            let deadline = probe_deadline(timeout)?;
            ElectrumClient::connect(backend, &self.gate, Some(consent), deadline)
                .map(|c| (c, deadline))
        })();
        match result {
            Err(error) => {
                logs.push(format!("Connection failed: {error}"));
                Ok(json!({"ok":false,"url":url,"trust_self_signed":insecure,"logs":logs}))
            }
            Ok((mut client, deadline)) => {
                logs.push("Connected.".into());
                if !client.server_version.is_null() {
                    logs.push(format!("Server version: {}", py(&client.server_version)));
                }
                match client.call("server.banner", None, deadline) {
                    Ok(banner) if !banner.is_null() && banner != json!("") => {
                        logs.push(format!("Server banner: {}", py(&banner)))
                    }
                    Err(error) => logs.push(format!("Server banner unavailable: {error}")),
                    _ => {}
                }
                Ok(json!({"ok":true,"url":url,"trust_self_signed":insecure,"logs":logs}))
            }
        }
    }
    fn core(&self, args: &Value, saved: Option<Value>) -> Result<Value> {
        let detected = optional(args, "credential_ref")
            .map(|reference| {
                detection::candidates(&detection::bitcoin_dir()?)?
                    .into_iter()
                    .find(|v| detection::credential_ref(v) == reference)
                    .ok_or_else(|| {
                        AppError::new(
                            "validation",
                            "Detected Bitcoin Core credentials are no longer available",
                        )
                        .with_hint("Run Detect my node again or enter RPC credentials manually.")
                    })
            })
            .transpose()?;
        let mut config = if let Some(mut detected) = detected {
            if let Some(timeout) = args
                .get("timeout")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
            {
                detected["timeout"] = json!(timeout);
            }
            detected
        } else if let Some(mut saved) = saved {
            if config::value(&saved, &["kind"]) != Some("bitcoinrpc") {
                return Err(AppError::new(
                    "validation",
                    format!(
                        "Backend '{}' is not a Bitcoin Core RPC backend",
                        optional(args, "backend").unwrap_or("")
                    ),
                )
                .with_hint("Choose a backend whose kind is bitcoinrpc."));
            }
            let pending = args.get("config").and_then(Value::as_object);
            let effective_url = pending
                .and_then(|c| c.get("url"))
                .and_then(Value::as_str)
                .or_else(|| optional(args, "url"));
            if let Some(url) = effective_url {
                if !same_origin(config::value(&saved, &["url"]).unwrap_or(""), url) {
                    for key in CREDENTIALS {
                        saved.as_object_mut().unwrap().remove(*key);
                        if let Some(nested) = saved.get_mut("config").and_then(Value::as_object_mut)
                        {
                            nested.remove(*key);
                        }
                    }
                }
            }
            if let Some(url) = optional(args, "url") {
                saved["url"] = json!(url);
            }
            if let Some(extra) = args.get("config").and_then(Value::as_object) {
                for (k, v) in extra {
                    saved[k] = v.clone();
                }
            }
            saved
        } else {
            let url = required(args, "url", "Bitcoin Core RPC URL")?;
            let mut config = json!({"name":optional(args,"name").unwrap_or("candidate"),"kind":"bitcoinrpc","url":url,"timeout":timeout(args)});
            if let Some(extra) = args.get("config").and_then(Value::as_object) {
                for (k, v) in extra {
                    config[k] = v.clone();
                }
            }
            for key in CREDENTIALS {
                if let Some(v) = optional(args, key) {
                    config[*key] = json!(v);
                }
            }
            config
        };
        if let Some(proxy) = optional(args, "proxy").or_else(|| optional(args, "tor_proxy")) {
            config["tor_proxy"] = json!(proxy);
        }
        detection::validate_cookie(&config)?;
        let birthday = optional(args, "birthday")
            .map(|raw| {
                time::parse(raw)
                    .map(|t| (t - time::UNIX) / 1_000_000)
                    .ok_or_else(|| {
                        AppError::new(
                            "validation",
                            format!("Invalid occurred_at/date value '{raw}'"),
                        )
                        .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
                    })
            })
            .transpose()?
            .unwrap_or(0);
        let backend = Backend::new(&config, false)?;
        let proof = Consent::explicit_trigger(backend.request());
        let deadline = Instant::now()
            .checked_add(
                backend
                    .timeout()
                    .checked_mul(7)
                    .ok_or_else(|| kassiber_net::Error::InvalidTimeout.app_error())?,
            )
            .ok_or_else(|| kassiber_net::Error::InvalidTimeout.app_error())?;
        let client = CoreClient::new(backend, self.gate.clone(), &self.version);
        match core_probe(&client, &proof, deadline, birthday) {
            Ok(payload) => Ok(payload),
            Err(error) => Ok(
                json!({"reachable":false,"status":"unresponsive","chain":null,"network":null,"blocks":null,"headers":null,"peers":null,"pruned":null,"pruneheight":null,"version":null,"ibd":null,"wallet_rpc":null,"block_filters":null,"warnings":["unresponsive"],"error":{"code":error.code,"message":error.message,"hint":error.hint,"retryable":error.retryable}}),
            ),
        }
    }
}
const CREDENTIALS: &[&str] = &[
    "username",
    "rpcuser",
    "rpc_user",
    "password",
    "rpcpassword",
    "rpc_password",
    "cookiefile",
    "cookie_file",
    "auth_header",
    "token",
];
fn same_origin(a: &str, b: &str) -> bool {
    use kassiber_net::HttpUrl;
    match (HttpUrl::parse(a), HttpUrl::parse(b)) {
        (Ok(a), Ok(b)) => {
            a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port() == b.port()
        }
        _ => false,
    }
}
fn py(value: &Value) -> String {
    kassiber_envelope::py_str(&details(value.clone()))
}
pub(crate) fn optional<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
fn required<'a>(args: &'a Value, key: &str, label: &str) -> Result<&'a str> {
    optional(args, key).ok_or_else(|| {
        AppError::new("validation", format!("{label} is required."))
            .with_hint(format!("Enter {}.", label.to_lowercase()))
    })
}
fn validate_args(kind: &str, args: &Value) -> Result<()> {
    let fields = match kind {
        "ui.backends.http.test" | "ui.backends.electrum.test" => {
            &["url", "proxy", "certificate"][..]
        }
        "ui.backends.bitcoinrpc.test" => &[
            "backend",
            "url",
            "credential_ref",
            "birthday",
            "name",
            "network",
            "proxy",
            "tor_proxy",
            "cookiefile",
            "cookie_file",
            "username",
            "password",
            "rpcuser",
            "rpc_user",
            "rpcpassword",
            "rpc_password",
        ][..],
        "ui.backends.btcpay.test" => &[
            "backend",
            "server_url",
            "url",
            "api_key",
            "token",
            "backend_label",
            "instance_label",
        ][..],
        _ => &[],
    };
    for key in fields {
        if let Some(value) = args.get(key).filter(|v| !v.is_null() && !v.is_string()) {
            return Err(
                AppError::new("validation", format!("{key} must be a string")).with_details(
                    details(json!({"type":kassiber_envelope::type_name(&details(value.clone()))})),
                ),
            );
        }
    }
    if kind == "ui.backends.bitcoinrpc.test" {
        if let Some(value) = args
            .get("config")
            .filter(|v| !v.is_null() && !v.is_object())
        {
            return Err(
                AppError::new("validation", "config must be an object").with_details(details(
                    json!({"type":kassiber_envelope::type_name(&details(value.clone()))}),
                )),
            );
        }
    }
    Ok(())
}
fn timeout(args: &Value) -> u64 {
    args.get("timeout")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .unwrap_or(10)
}
fn probe_deadline(timeout: u64) -> Result<Instant> {
    Instant::now()
        .checked_add(Duration::from_secs(timeout))
        .ok_or_else(|| kassiber_net::Error::InvalidTimeout.app_error())
}
pub(crate) fn core_probe(
    client: &CoreClient,
    consent: &Consent,
    deadline: Instant,
    birthday: i64,
) -> Result<Value> {
    let call = |method: &str, params: Option<&Value>| {
        let deadline = if ["listwallets", "getbestblockhash", "getblockfilter"].contains(&method) {
            deadline.min(Instant::now() + Duration::from_secs(5))
        } else {
            deadline
        };
        client.call(method, params, None, Some(consent), deadline)
    };
    let chain = call("getblockchaininfo", None)?;
    if !chain.is_object() {
        return Err(AppError::new(
            "bitcoinrpc_unexpected_response",
            "Bitcoin Core getblockchaininfo returned an unexpected payload",
        )
        .retryable(true));
    }
    let network = call("getnetworkinfo", None)?;
    if !network.is_object() {
        return Err(AppError::new(
            "bitcoinrpc_unexpected_response",
            "Bitcoin Core getnetworkinfo returned an unexpected payload",
        )
        .retryable(true));
    }
    let pruned = chain["pruned"] == true;
    let mut earliest = None;
    if pruned {
        if let Some(height) = integer(&chain["pruneheight"]) {
            let header = call("getblockhash", Some(&json!([height])))
                .and_then(|hash| call("getblockheader", Some(&json!([hash]))));
            if let Ok(header) = header {
                earliest = integer(&header["time"])
                    .filter(|n| *n > 0)
                    .and_then(time::epoch);
            }
        }
    }
    let rescan_start = if pruned && birthday > 0 {
        time::epoch((birthday - 7200).max(0))
    } else {
        None
    };
    let wallet_rpc = match call("listwallets", None) {
        Ok(wallets) => {
            json!({"available":true,"loaded_wallet_count":wallets.as_array().map(Vec::len)})
        }
        Err(e) => {
            json!({"available":false,"error":{"code":e.code,"message":e.message,"hint":"Kassiber needs Bitcoin Core wallet RPC support to create a watch-only descriptor wallet."}})
        }
    };
    let filter = call("getbestblockhash", None)
        .and_then(|hash| call("getblockfilter", Some(&json!([hash]))));
    let block_filters = match filter {
        Ok(_) => json!({"available":true}),
        Err(e) => {
            json!({"available":false,"error":{"code":e.code,"message":e.message,"hint":null}})
        }
    };
    let mut warnings = Vec::new();
    if chain["initialblockdownload"] == true {
        warnings.push("initial_block_download");
    }
    if pruned {
        warnings.push("pruned");
    }
    if wallet_rpc["available"] != true {
        warnings.push("wallet_rpc_unavailable");
    }
    if block_filters["available"] != true {
        warnings.push("blockfilterindex_disabled");
    }
    let status = if network["connections"].as_i64().unwrap_or(0) == 0 {
        "connecting"
    } else if chain["initialblockdownload"] == true
        || chain["headers"].as_i64().unwrap_or(0) > chain["blocks"].as_i64().unwrap_or(0)
    {
        "synchronizing"
    } else {
        "synchronized"
    };
    Ok(
        json!({"reachable":true,"status":status,"chain":chain["chain"],"network":chain["chain"],"blocks":chain["blocks"],"headers":chain["headers"],"peers":network["connections"],"pruned":pruned,"pruneheight":chain["pruneheight"],"earliest_retained_at":earliest,"rescan_start_at":rescan_start,"birthday_covered":if pruned {Value::Null} else {json!(true)},"rescan_possible":if pruned {Value::Null} else {json!(true)},"version":network["version"],"ibd":chain["initialblockdownload"]==true,"wallet_rpc":wallet_rpc,"block_filters":block_filters,"warnings":warnings}),
    )
}

fn integer(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}
