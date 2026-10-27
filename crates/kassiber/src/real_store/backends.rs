//! Local bootstrap/backend-default projection for `init` and `status`. Merely
//! reading or seeding a backend never grants consent to contact it.
use super::{error, setting, timestamp};
use kassiber_envelope::AppError;
use kassiber_store::map_sqlite_error;
use kassiber_store::rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

type Backend = BTreeMap<String, String>;
const BUILTINS: [&str; 4] = ["fulcrum", "liquid", "liquid-blockstream", "mempool"];
const RESERVED: [&str; 17] = [
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
const KINDS: [&str; 12] = [
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
pub struct Config {
    seed: BTreeMap<String, Backend>,
    runtime: BTreeMap<String, Backend>,
    dotenv: BTreeSet<String>,
    env_names: BTreeSet<String>,
    env_default: bool,
    bootstrap_default: String,
    pub default_backend: String,
}
fn canonical(field: &str) -> String {
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
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for prefix in ["SATBOOKS_BACKEND_", "KASSIBER_BACKEND_"] {
        for (key, value) in env {
            if let Some((name, field)) = key.strip_prefix(prefix).and_then(|k| k.split_once('_')) {
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
fn default(env: &BTreeMap<String, String>) -> Option<&str> {
    env.get("KASSIBER_DEFAULT_BACKEND")
        .filter(|s| !s.is_empty())
        .or_else(|| {
            env.get("SATBOOKS_DEFAULT_BACKEND")
                .filter(|s| !s.is_empty())
        })
        .map(String::as_str)
}
fn set(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[key,value]).map(|_|()).map_err(map_sqlite_error).map_err(error)
}
fn tombstones(conn: &Connection) -> Result<BTreeSet<String>, AppError> {
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
    pub fn load(file: &Path) -> Result<Self, AppError> {
        let mut seed = BTreeMap::new();
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
        let dotenv = apply(&mut seed, &file_env);
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
        let process: BTreeMap<String, String> = std::env::vars().collect();
        let mut runtime = seed.clone();
        let env_names = apply(&mut runtime, &process);
        let env_default = process.contains_key("KASSIBER_DEFAULT_BACKEND")
            || process.contains_key("SATBOOKS_DEFAULT_BACKEND");
        let current =
            kassiber_pytext::strip(default(&process).unwrap_or(&bootstrap_default)).to_lowercase();
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
            seed,
            runtime,
            dotenv,
            env_names,
            env_default,
            bootstrap_default,
            default_backend,
        })
    }
    pub fn merge(&mut self, conn: &Connection) -> Result<(), AppError> {
        let tombstones = tombstones(conn)?
            .difference(&self.dotenv)
            .cloned()
            .collect::<BTreeSet<_>>();
        for name in &tombstones {
            if !self.env_names.contains(name) {
                self.runtime.remove(name);
            }
        }
        let mut stmt = conn
            .prepare("SELECT name FROM backends")
            .map_err(map_sqlite_error)
            .map_err(error)?;
        for name in stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(map_sqlite_error)
            .map_err(error)?
        {
            let name = name
                .map_err(map_sqlite_error)
                .map_err(error)?
                .to_lowercase();
            if !tombstones.contains(&name) || self.env_names.contains(&name) {
                self.runtime.entry(name).or_default();
            }
        }
        let manual = manual_mode(conn)?;
        if !self.env_default {
            if let Some(stored) = setting(conn, "default_backend")?.filter(|s| !s.is_empty()) {
                if !self.runtime.contains_key(&stored) {
                    return Err(AppError::new("config_error",format!("Stored default backend '{stored}' is not configured")).with_hint("Run `kassiber backends set-default <name>` with a valid backend, or `backends clear-default` to fall back to the env default."));
                }
                self.default_backend = stored;
            } else if manual && !self.runtime.contains_key(&self.default_backend) {
                self.default_backend.clear();
            }
        } else if !self.runtime.contains_key(&self.default_backend) {
            if manual && tombstones.contains(&self.default_backend) {
                self.default_backend.clear();
            } else {
                return Err(AppError::new(
                    "config_error",
                    format!(
                        "Environment default backend '{}' is not configured",
                        self.default_backend
                    ),
                )
                .with_hint(
                    "Define that backend in the current process or remove the env override.",
                ));
            }
        }
        Ok(())
    }
    pub fn seed(&mut self, conn: &Connection, version: &str) -> Result<(), AppError> {
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
            set(conn, "app_version", version)?;
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

fn manual_mode(conn: &Connection) -> Result<bool, AppError> {
    match setting(conn, "backend_bootstrap_mode")?.as_deref() {
        None | Some("public") => Ok(false),
        Some("manual") => Ok(true),
        _ => Err(
            AppError::new("config_error", "Stored backend setup mode is invalid")
                .with_hint("Choose built-in, custom, or offline backend setup again."),
        ),
    }
}
fn normalize_chain(value: &str) -> Result<&'static str, AppError> {
    match kassiber_pytext::strip(value).to_lowercase().as_str() {
        "" | "bitcoin" | "btc" | "onchain" => Ok("bitcoin"),
        "liquid" | "lbtc" | "elements" => Ok("liquid"),
        _ => Err(AppError::new(
            "app_error",
            format!("Unsupported wallet chain '{value}'"),
        )),
    }
}
fn normalize_network(chain: Option<&str>, value: &str) -> Result<&'static str, AppError> {
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
