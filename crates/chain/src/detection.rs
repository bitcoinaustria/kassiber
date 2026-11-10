//! Local Core discovery runs only under the explicit detect action. Reading a
//! bitcoin.conf is configuration, never authority for a non-loopback probe.
use super::*;
use kassiber_net::HttpUrl;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) fn bitcoin_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(|p| PathBuf::from(p).join(".bitcoin"))
        .ok_or_else(|| AppError::new("config_error", "Home directory unavailable"))
}
fn expand(path: &str) -> Result<PathBuf> {
    Ok(if let Some(tail) = path.strip_prefix("~/") {
        bitcoin_dir()?.parent().unwrap().join(tail)
    } else {
        PathBuf::from(path)
    })
}
pub(crate) fn loopback(raw: &str) -> bool {
    let Ok(url) = HttpUrl::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
    // HttpUrl already parses and canonicalizes IPv4/IPv6 literals. No resolver.
    host == "localhost" || host.starts_with("127.") || host == "::1"
}
pub(crate) fn validate_cookie(config: &Value) -> Result<()> {
    let Some(path) = config::value(config, &["cookiefile", "cookie_file"]) else {
        return Ok(());
    };
    if !loopback(config::value(config, &["url"]).unwrap_or("")) {
        return Err(AppError::new("validation","Bitcoin Core cookie-file credentials can only be used with a loopback RPC URL")
            .with_hint("Use username/password auth for remote Core RPC, or point the cookie-file backend at localhost."));
    }
    let path = resolve(&expand(path)?)?;
    let root = resolve(&bitcoin_dir()?)?;
    if path.file_name().is_none_or(|n| n != ".cookie") || !path.starts_with(root) {
        return Err(AppError::new("validation","Desktop Bitcoin Core cookie-file credentials must point at a default .bitcoin cookie file")
            .with_hint("Use a cookie path under ~/.bitcoin ending in .cookie, or use username/password auth for non-standard Core data directories."));
    }
    Ok(())
}
fn resolve(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| app("Working directory unavailable"))?
            .join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => {
                result.push(other.as_os_str());
            }
        }
        // Resolve existing prefixes so symlinks cannot escape ~/.bitcoin.
        if let Ok(canonical) = result.canonicalize() {
            result = canonical;
        }
    }
    Ok(result)
}
type Settings = BTreeMap<String, BTreeMap<String, String>>;
fn parse_conf(text: &str) -> Settings {
    let mut result = Settings::new();
    let mut section = "global".to_owned();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(next) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = next.trim().to_lowercase();
            if section.is_empty() {
                section = "global".into();
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        let (scope, key) = key
            .split_once('.')
            .filter(|(a, b)| !a.is_empty() && !b.is_empty())
            .unwrap_or((&section, &key));
        result
            .entry(scope.trim().into())
            .or_default()
            .insert(key.trim().into(), value.trim().into());
    }
    result
}
pub(crate) fn candidates(root: &Path) -> Result<Vec<Value>> {
    let settings = std::fs::read_to_string(root.join("bitcoin.conf"))
        .map(|s| parse_conf(&s))
        .unwrap_or_default();
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    for (network, port, cookie, aliases) in [
        ("main", 8332, ".cookie", &["main", "mainnet"][..]),
        (
            "test",
            18332,
            "testnet3/.cookie",
            &["test", "testnet", "testnet3", "testnet4"][..],
        ),
        (
            "test",
            18332,
            "testnet4/.cookie",
            &["test", "testnet", "testnet3", "testnet4"][..],
        ),
        ("regtest", 18443, "regtest/.cookie", &["regtest"][..]),
        ("signet", 38332, "signet/.cookie", &["signet"][..]),
    ] {
        let mut merged = settings.get("global").cloned().unwrap_or_default();
        for alias in aliases {
            if let Some(extra) = settings.get(*alias) {
                merged.extend(extra.clone());
            }
        }
        let url = rpc_url(port, &merged);
        if !loopback(&url) {
            continue;
        }
        let configured = merged.get("rpccookiefile").filter(|s| !s.is_empty());
        let path = if let Some(path) = configured {
            let p = expand(path)?;
            if p.is_absolute() {
                p
            } else {
                root.join(p)
            }
        } else {
            root.join(cookie)
        };
        let mut common = json!({"name":format!("local-core-{network}"),"kind":"bitcoinrpc","chain":"bitcoin","network":network,"url":url,"timeout":2});
        if let (Some(user), Some(password)) = (
            merged.get("rpcuser").filter(|s| !s.is_empty()),
            merged.get("rpcpassword").filter(|s| !s.is_empty()),
        ) {
            let mut basic = common.clone();
            basic["auth_source"] = json!("basic");
            basic["credential_source"] = json!("bitcoin.conf");
            basic["username"] = json!(user);
            basic["password"] = json!(password);
            if seen.insert((network.to_owned(), url.clone(), "basic", user.clone())) {
                found.push(basic);
            }
        }
        if path.exists() {
            common["auth_source"] = json!("cookiefile");
            common["credential_source"] = json!(if configured.is_some() {
                "bitcoin.conf"
            } else {
                "default"
            });
            common["cookiefile"] = json!(path.to_string_lossy());
            if seen.insert((
                network.to_owned(),
                url,
                "cookiefile",
                path.to_string_lossy().into_owned(),
            )) {
                found.push(common);
            }
        }
    }
    Ok(found)
}
fn rpc_url(default_port: u16, settings: &BTreeMap<String, String>) -> String {
    if let Some(url) = settings
        .get("rpcurl")
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
    {
        return url.clone();
    }
    let mut host = settings
        .get("rpcconnect")
        .or_else(|| settings.get("rpcbind"))
        .map(String::as_str)
        .unwrap_or("127.0.0.1")
        .split(',')
        .next()
        .unwrap_or("")
        .trim();
    if matches!(host, "" | "0.0.0.0" | "::" | "[::]") {
        host = "127.0.0.1";
    }
    let mut port = None;
    if let Some((address, suffix)) = host.rsplit_once(':') {
        if (host.matches(':').count() == 1 || address.ends_with(']'))
            && suffix.parse::<u16>().is_ok()
        {
            host = address;
            port = suffix.parse().ok();
        }
    }
    let port = settings
        .get("rpcport")
        .map(|s| s.parse::<u16>().unwrap_or(default_port))
        .or(port)
        .unwrap_or(default_port);
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.into()
    };
    format!("http://{host}:{port}")
}
pub(crate) fn credential_ref(config: &Value) -> String {
    let source = [
        "network",
        "url",
        "auth_source",
        "credential_source",
        "cookiefile",
        "username",
    ]
    .map(|key| config[key].as_str().unwrap_or(""))
    .join("|");
    let digest: String = Sha256::digest(source.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("local-core:{}", &digest[..24])
}
