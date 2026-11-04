//! Local bootstrap/backend-default projection for `init` and `status`. Merely
//! reading or seeding a backend never grants consent to contact it.
use super::{setting, storage_error as error};
fn timestamp() -> Result<String, AppError> {
    super::Sources::now(
        &super::SystemSources,
        &Connection::open_in_memory().map_err(super::sql_error)?,
    )
}
use kassiber_envelope::AppError;
use kassiber_store::map_sqlite_error;
use kassiber_store::rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(super) type Backend = BTreeMap<String, String>;
pub(super) const BUILTINS: [&str; 4] = ["fulcrum", "liquid", "liquid-blockstream", "mempool"];
pub(super) const RESERVED: [&str; 17] = [
    "name",
    "kind",
    "chain",
    "network",
    "url",
    "auth_header",
    "token",
    "batch_size",
    "timeout",
    "tor_proxy",
    "config_json",
    "notes",
    "created_at",
    "updated_at",
    "config",
    "is_default",
    "source",
];
pub(super) const KINDS: [&str; 12] = [
    "binance",
    "bitcoinrpc",
    "btcpay",
    "coinbase",
    "coreln",
    "custom",
    "electrum",
    "esplora",
    "kraken",
    "liquid-esplora",
    "lnd",
    "mempool",
];
#[derive(Clone)]
pub struct Config {
    pub(super) seed: BTreeMap<String, Backend>,
    pub(super) seed_order: Vec<String>,
    pub(super) runtime: BTreeMap<String, Backend>,
    pub(super) dotenv: BTreeSet<String>,
    pub(super) dotenv_fields: BTreeMap<String, BTreeSet<String>>,
    pub(super) env_names: BTreeSet<String>,
    pub(super) env_default: bool,
    bootstrap_default: String,
    pub(super) env_fields: BTreeMap<String, BTreeSet<String>>,
    pub(super) view: BTreeMap<String, kassiber_pyjson::Value>,
    pub(super) env_file: String,
    pub default_backend: String,
}
pub(super) fn canonical(field: &str) -> String {
    let field = field.to_lowercase().replace('-', "_");
    match field.as_str() {
        "bip352" => "silent_payments",
        "commando" | "commando_id" => "commando_peer_id",
        "cookie_file" => "cookiefile",
        "display_label" => "display_name",
        "lightningdir" | "lightning_directory" => "lightning_dir",
        "rpcfile" => "rpc_file",
        "rpcpassword" | "rpc_password" => "password",
        "rpcuser" | "rpc_user" => "username",
        "sp_capable" => "silent_payments",
        "sp_scan_file" => "silent_payment_scan_file",
        "sp_scan_path" => "silent_payment_scan_path",
        "wallet_prefix" => "walletprefix",
        _ => &field,
    }
    .to_owned()
}
fn apply(
    backends: &mut BTreeMap<String, Backend>,
    env: &BTreeMap<String, String>,
    source: &str,
    override_source: bool,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for prefix in ["SATBOOKS_BACKEND_", "KASSIBER_BACKEND_"] {
        for (key, value) in env {
            if let Some((name, field)) = key.strip_prefix(prefix).and_then(|k| k.split_once('_')) {
                if name.is_empty() || field.is_empty() {
                    continue;
                }
                let name = name.to_lowercase();
                let backend = backends.entry(name.clone()).or_default();
                backend.entry("name".into()).or_insert_with(|| name.clone());
                if override_source || !backend.contains_key("source") {
                    backend.insert("source".into(), source.into());
                }
                backend.insert(canonical(field), kassiber_pytext::strip(value).to_owned());
                names.insert(name);
            }
        }
    }
    names
}
fn default(env: &BTreeMap<String, String>) -> Option<&str> {
    env.get("KASSIBER_DEFAULT_BACKEND")
        .filter(|s| !s.is_empty())
        .or_else(|| {
            env.get("SATBOOKS_DEFAULT_BACKEND")
                .filter(|s| !s.is_empty())
        })
        .map(String::as_str)
}
fn fields(env: &BTreeMap<String, String>) -> BTreeMap<String, BTreeSet<String>> {
    let mut fields: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for key in env.keys() {
        for prefix in ["SATBOOKS_BACKEND_", "KASSIBER_BACKEND_"] {
            if let Some((name, field)) = key.strip_prefix(prefix).and_then(|s| s.split_once('_')) {
                if !name.is_empty() && !field.is_empty() {
                    fields
                        .entry(name.to_lowercase())
                        .or_default()
                        .insert(canonical(field));
                }
            }
        }
    }
    fields
}
fn set(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[key,value]).map(|_|()).map_err(map_sqlite_error).map_err(error)
}
pub(super) fn tombstones(conn: &Connection) -> Result<BTreeSet<String>, AppError> {
    let raw = setting(conn, "bootstrap_backend_tombstones")?.unwrap_or_default();
    Ok(serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect())
}
impl Config {
    /// Public diagnostics only: the private merged configuration never leaves this service.
    pub fn diagnostics_summary(&self) -> kassiber_pyjson::Value {
        let config = kassiber_envelope::dict([
            (
                "backends",
                kassiber_envelope::dict(self.view.iter().map(|(k, v)| (k.as_str(), v.clone()))),
            ),
            (
                "default_backend",
                kassiber_envelope::s(self.default_backend.as_str()),
            ),
        ]);
        kassiber_diagnostics::state::backends(&config)
    }

    /// Resolve raw adapter configuration inside the backend, never project it
    /// to AI or the renderer. DB rows override seeded/dotenv configuration;
    /// explicit process fields override the DB one field at a time.
    pub fn resolve(&self, conn: &Connection, name: &str) -> Result<Value, AppError> {
        use kassiber_store::rusqlite::{types::ValueRef, OptionalExtension};
        let name = name.trim().to_lowercase();
        let stored = conn
            .query_row(
                "SELECT * FROM backends WHERE lower(name)=?",
                [&name],
                |row| {
                    let mut result = serde_json::Map::new();
                    for (index, key) in row.as_ref().column_names().iter().enumerate() {
                        let value = match row.get_ref(index)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => json!(n),
                            ValueRef::Real(n) => json!(n),
                            ValueRef::Text(s) => json!(String::from_utf8_lossy(s)),
                            ValueRef::Blob(_) => continue,
                        };
                        result.insert((*key).into(), value);
                    }
                    Ok(Value::Object(result))
                },
            )
            .optional()
            .map_err(map_sqlite_error)
            .map_err(error)?;
        if stored.is_none() && !self.runtime.contains_key(&name) {
            return Err(AppError::new(
                "app_error",
                format!("Unknown backend '{name}'"),
            ));
        }
        let mut result = stored.unwrap_or_else(|| json!(self.runtime.get(&name).unwrap()));
        if let Some(raw) = result.get("config_json").and_then(Value::as_str) {
            let extra: Value = serde_json::from_str(raw)
                .map_err(|_| AppError::new("config_error", "Invalid backend configuration JSON"))?;
            if let Some(fields) = extra.as_object() {
                for (k, v) in fields {
                    result[k] = v.clone();
                }
            }
            result.as_object_mut().unwrap().remove("config_json");
        }
        let mut process = BTreeMap::new();
        fn apply_probe_fields(
            backends: &mut BTreeMap<String, Backend>,
            env: &BTreeMap<String, String>,
        ) -> BTreeSet<String> {
            let mut names = BTreeSet::new();
            for prefix in ["SATBOOKS_BACKEND_", "KASSIBER_BACKEND_"] {
                for (key, value) in env {
                    if let Some((name, field)) =
                        key.strip_prefix(prefix).and_then(|k| k.split_once('_'))
                    {
                        if name.is_empty() || field.is_empty() {
                            continue;
                        }
                        let name = name.to_lowercase();
                        backends
                            .entry(name.clone())
                            .or_default()
                            .insert(canonical(field), kassiber_pytext::strip(value).to_owned());
                        names.insert(name);
                    }
                }
            }
            names
        }
        apply_probe_fields(&mut process, &std::env::vars().collect());
        if let Some(fields) = process.get(&name) {
            for (k, v) in fields {
                result[k] = json!(v);
            }
        }
        result["name"] = json!(name);
        Ok(result)
    }

    /// One merged backend as Python's `runtime_config["backends"][name]`,
    /// for local classification only; callers must not project it.
    pub fn configured(&self, name: &str) -> Option<&kassiber_pyjson::Value> {
        self.view.get(name)
    }

    pub fn kinds(&self) -> BTreeMap<String, String> {
        self.runtime
            .iter()
            .map(|(name, backend)| {
                (
                    name.clone(),
                    backend.get("kind").cloned().unwrap_or_default(),
                )
            })
            .collect()
    }

    pub fn load(file: &Path) -> Result<Self, AppError> {
        Self::load_with_env(file, &std::env::vars().collect())
    }
    /// Explicit environment injection keeps tests off process-global state.
    pub fn load_with_env(
        file: &Path,
        process: &BTreeMap<String, String>,
    ) -> Result<Self, AppError> {
        let mut seed: BTreeMap<String, Backend> = BTreeMap::new();
        for (name, kind, chain, network, url, batch) in [
            (
                "mempool",
                "esplora",
                "bitcoin",
                "main",
                "https://mempool.bitcoin-austria.at/api",
                "",
            ),
            (
                "fulcrum",
                "electrum",
                "bitcoin",
                "main",
                "ssl://index.bitcoin-austria.at:50002",
                "100",
            ),
            (
                "liquid",
                "electrum",
                "liquid",
                "liquidv1",
                "ssl://les.bullbitcoin.com:995",
                "100",
            ),
            (
                "liquid-blockstream",
                "electrum",
                "liquid",
                "liquidv1",
                "ssl://blockstream.info:995",
                "100",
            ),
        ] {
            seed.insert(
                name.to_owned(),
                [
                    ("name", name),
                    ("source", "built-in default"),
                    ("kind", kind),
                    ("chain", chain),
                    ("network", network),
                    ("url", url),
                    ("batch_size", batch),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
            );
        }
        let mut file_env = BTreeMap::new();
        let raw = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(error(kassiber_paths::Error::io(e))),
        };
        for line in raw.lines().map(kassiber_pytext::strip) {
            if line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let mut value = kassiber_pytext::strip(value);
                if value.len() >= 2
                    && value.starts_with(['\'', '"'])
                    && value.ends_with(['\'', '"'])
                {
                    value = &value[1..value.len() - 1];
                }
                file_env.insert(kassiber_pytext::strip(key).to_owned(), value.to_owned());
            }
        }
        seed.get_mut("mempool").unwrap().remove("batch_size");
        let mut seed_order: Vec<String> = ["mempool", "fulcrum", "liquid", "liquid-blockstream"]
            .map(str::to_owned)
            .to_vec();
        for prefix in ["SATBOOKS_BACKEND_", "KASSIBER_BACKEND_"] {
            for line in raw.lines().map(kassiber_pytext::strip) {
                if line.starts_with('#') {
                    continue;
                }
                if let Some((key, _)) = line.split_once('=') {
                    if let Some((name, field)) = kassiber_pytext::strip(key)
                        .strip_prefix(prefix)
                        .and_then(|s| s.split_once('_'))
                    {
                        let name = name.to_lowercase();
                        if !name.is_empty() && !field.is_empty() && !seed_order.contains(&name) {
                            seed_order.push(name);
                        }
                    }
                }
            }
        }
        let dotenv = apply(&mut seed, &file_env, &file.display().to_string(), true);
        let bootstrap_default =
            kassiber_pytext::strip(default(&file_env).unwrap_or("fulcrum")).to_lowercase();
        let bootstrap_default = if bootstrap_default.is_empty() {
            "fulcrum".into()
        } else {
            bootstrap_default
        };
        if !seed.contains_key(&bootstrap_default) {
            return Err(AppError::new("app_error",format!("Default backend '{bootstrap_default}' is not defined. Add KASSIBER_BACKEND_{}_KIND and _URL to {}.",bootstrap_default.to_uppercase(),file.display())));
        }

        let mut runtime = seed.clone();
        let env_names = apply(&mut runtime, process, "environment", false);
        let env_fields = fields(process);
        let dotenv_fields = fields(&file_env);
        let env_default = process.contains_key("KASSIBER_DEFAULT_BACKEND")
            || process.contains_key("SATBOOKS_DEFAULT_BACKEND");
        let current =
            kassiber_pytext::strip(default(process).unwrap_or(&bootstrap_default)).to_lowercase();
        let default_backend = if current.is_empty() {
            bootstrap_default.clone()
        } else {
            current
        };
        for (name, backend) in &runtime {
            if ["kind", "url"]
                .iter()
                .any(|key| backend.get(*key).is_none_or(String::is_empty))
            {
                return Err(AppError::new(
                    "app_error",
                    format!("Backend '{name}' is missing kind or url configuration"),
                ));
            }
        }
        Ok(Self {
            env_file: file.display().to_string(),
            env_fields,
            view: BTreeMap::new(),
            seed,
            seed_order,
            runtime,
            dotenv,
            dotenv_fields,
            env_names,
            env_default,
            bootstrap_default,
            default_backend,
        })
    }
    pub fn merge(&mut self, conn: &Connection) -> Result<(), AppError> {
        super::backends::merge(conn, self)
    }
    pub fn seed(&mut self, conn: &Connection, version: &str) -> Result<(), AppError> {
        self.seed_inner(conn, Some(version))
    }
    pub fn seed_configuration(&mut self, conn: &Connection) -> Result<(), AppError> {
        self.seed_inner(conn, None)
    }
    fn seed_inner(&mut self, conn: &Connection, version: Option<&str>) -> Result<(), AppError> {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(map_sqlite_error)
            .map_err(error)?;
        let result = (|| {
            let mut removed = tombstones(conn)?;
            let before = removed.clone();
            removed.retain(|n| !self.dotenv.contains(n));
            if before != removed {
                set(
                    conn,
                    "bootstrap_backend_tombstones",
                    &json!(removed).to_string(),
                )?;
            }
            let manual = manual_mode(conn)?;
            for (name, backend) in &self.seed {
                if removed.contains(name)
                    || (manual && BUILTINS.contains(&name.as_str()) && !self.dotenv.contains(name))
                {
                    continue;
                }
                let get = |key: &str| {
                    backend
                        .get(key)
                        .filter(|v| !v.is_empty())
                        .map(String::as_str)
                };
                let number = |key: &str| -> Result<Option<i64>, AppError> {
                    let n = get(key)
                        .map(|value| {
                            let parsed = kassiber_pytext::parse_int(value).map_err(|_| {
                                AppError::new(
                                    "app_error",
                                    format!("Invalid integer value: {value}"),
                                )
                            })?;
                            i64::try_from(parsed).map_err(|_| {
                                AppError::new(
                                    "unsupported_configuration",
                                    "backend integer exceeds SQLite storage range",
                                )
                            })
                        })
                        .transpose()?;
                    if n.is_some_and(|n| n <= 0) {
                        return Err(AppError::new(
                            "validation",
                            if key == "timeout" {
                                "Backend timeout must be positive"
                            } else {
                                "Backend batch size must be positive"
                            },
                        ));
                    }
                    Ok(n)
                };
                let kind = get("kind").unwrap_or_default().to_lowercase();
                if !KINDS.contains(&kind.as_str()) {
                    return Err(AppError::new(
                        "validation",
                        format!("Unsupported backend kind '{kind}'"),
                    )
                    .with_hint(format!("Choose one of: {}", KINDS.join(", "))));
                }
                let chain = get("chain").map(normalize_chain).transpose()?;
                let network = get("network")
                    .map(|n| normalize_network(chain, n))
                    .transpose()?;
                let mut config = serde_json::Map::new();
                for (key, value) in backend {
                    if RESERVED.contains(&key.as_str()) {
                        continue;
                    }
                    if ["insecure", "silent_payments"].contains(&key.as_str()) {
                        let boolean = match value.to_lowercase().as_str() {
                            "1" | "true" | "yes" | "on" => true,
                            "0" | "false" | "no" | "off" => false,
                            _ => {
                                return Err(AppError::new(
                                    "app_error",
                                    format!("Invalid boolean value: {value}"),
                                ))
                            }
                        };
                        config.insert(key.clone(), json!(boolean));
                    } else if !value.is_empty() {
                        config.insert(key.clone(), json!(value));
                    }
                }
                let config = kassiber_pytext::loads(&Value::Object(config).to_string())
                    .map_err(|_| AppError::new("config_error", "invalid backend configuration"))?;
                let config = kassiber_pyjson::dumps(
                    &config,
                    &kassiber_pyjson::Options {
                        sort_keys: true,
                        ..Default::default()
                    },
                )
                .map_err(|_| AppError::new("config_error", "invalid backend configuration"))?;
                let ts = timestamp()?;
                conn.execute("INSERT INTO backends(name,kind,chain,network,url,auth_header,token,batch_size,timeout,tor_proxy,config_json,notes,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,NULL,?,?) ON CONFLICT(name) DO NOTHING", params![name,kind,chain,network,get("url"),get("auth_header"),get("token"),number("batch_size")?,number("timeout")?,get("tor_proxy"),config,ts,ts]).map_err(map_sqlite_error).map_err(error)?;
            }
            if setting(conn, "bootstrap_default_backend")?.is_none_or(|s| s.is_empty()) {
                let available: Vec<(String, String, String)> = conn
                    .prepare(
                        "SELECT name,coalesce(chain,'bitcoin'),kind FROM backends ORDER BY name",
                    )
                    .map_err(map_sqlite_error)
                    .map_err(error)?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(map_sqlite_error)
                    .map_err(error)?
                    .collect::<Result<_, _>>()
                    .map_err(map_sqlite_error)
                    .map_err(error)?;
                let default = if available
                    .iter()
                    .any(|(n, _, _)| n == &self.bootstrap_default)
                {
                    Some(self.bootstrap_default.clone())
                } else {
                    let candidates: Vec<_> = available
                        .iter()
                        .filter(|(_, chain, kind)| {
                            (chain.is_empty() || chain == "bitcoin")
                                && [
                                    "bitcoinrpc",
                                    "custom",
                                    "electrum",
                                    "esplora",
                                    "liquid-esplora",
                                    "mempool",
                                ]
                                .contains(&kind.as_str())
                        })
                        .map(|(n, _, _)| n)
                        .collect();
                    candidates
                        .iter()
                        .find(|n| !BUILTINS.contains(&n.as_str()))
                        .or_else(|| candidates.iter().find(|n| n.as_str() == "mempool"))
                        .or(candidates.first())
                        .map(|n| (*n).clone())
                };
                if let Some(default) = default {
                    set(conn, "bootstrap_default_backend", &default)?;
                }
            }
            if setting(conn, "default_backend")?.is_none_or(|s| s.is_empty()) {
                if let Some(default) = setting(conn, "bootstrap_default_backend")? {
                    set(conn, "default_backend", &default)?;
                }
            }
            if let Some(version) = version {
                set(conn, "app_version", version)?;
            }
            Ok(())
        })();
        if result.is_ok() {
            conn.execute_batch("COMMIT")
                .map_err(map_sqlite_error)
                .map_err(error)?;
        } else {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    }
}

pub(super) fn manual_mode(conn: &Connection) -> Result<bool, AppError> {
    match setting(conn, "backend_bootstrap_mode")?.as_deref() {
        None | Some("public") => Ok(false),
        Some("manual") => Ok(true),
        _ => Err(
            AppError::new("config_error", "Stored backend setup mode is invalid")
                .with_hint("Choose built-in, custom, or offline backend setup again."),
        ),
    }
}
pub(super) fn normalize_chain(value: &str) -> Result<&'static str, AppError> {
    match kassiber_pytext::strip(value).to_lowercase().as_str() {
        "" | "bitcoin" | "btc" | "onchain" => Ok("bitcoin"),
        "liquid" | "lbtc" | "elements" => Ok("liquid"),
        _ => Err(AppError::new(
            "app_error",
            format!("Unsupported wallet chain '{value}'"),
        )),
    }
}
pub(super) fn normalize_network(
    chain: Option<&str>,
    value: &str,
) -> Result<&'static str, AppError> {
    let chain = chain.unwrap_or("bitcoin");
    match (chain, kassiber_pytext::strip(value).to_lowercase().as_str()) {
        ("bitcoin", "" | "main" | "mainnet" | "bitcoin") => Ok("main"),
        ("bitcoin", "test" | "testnet") => Ok("test"),
        ("bitcoin", "regtest") => Ok("regtest"),
        ("bitcoin", "signet") => Ok("signet"),
        ("liquid", "" | "main" | "mainnet" | "liquid" | "liquidv1") => Ok("liquidv1"),
        ("liquid", "test" | "testnet" | "liquidtestnet") => Ok("liquidtestnet"),
        ("liquid", "regtest" | "elements" | "elementsregtest") => Ok("elementsregtest"),
        _ => Err(AppError::new(
            "app_error",
            format!("Unsupported {chain} network '{value}'"),
        )),
    }
}
