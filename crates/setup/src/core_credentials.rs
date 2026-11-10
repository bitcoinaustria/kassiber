//! Re-resolve opaque Core references on an explicit backend-create action.
//! Local configuration reads only; detection/probes are separate refused kinds.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
type Settings = BTreeMap<String, BTreeMap<String, String>>;
fn parse(text: &str) -> Settings {
    let mut settings = Settings::from([("global".into(), BTreeMap::new())]);
    let mut section = "global".to_owned();
    for line in text
        .split([
            '\n', '\r', '\u{000b}', '\u{000c}', '\u{001c}', '\u{001d}', '\u{001e}', '\u{0085}',
            '\u{2028}', '\u{2029}',
        ])
        .map(kassiber_pytext::strip)
    {
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = kassiber_pytext::strip(name).to_lowercase();
            if section.is_empty() {
                section = "global".into();
            }
            settings.entry(section.clone()).or_default();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = kassiber_pytext::strip(key).to_lowercase();
        let value = kassiber_pytext::strip(value).to_owned();
        if key.is_empty() {
            continue;
        }
        if let Some((network, key)) = key
            .split_once('.')
            .filter(|(n, k)| !n.is_empty() && !k.is_empty())
        {
            settings
                .entry(kassiber_pytext::strip(network).to_lowercase())
                .or_default()
                .insert(kassiber_pytext::strip(key).to_lowercase(), value);
        } else {
            settings
                .entry(section.clone())
                .or_default()
                .insert(key, value);
        }
    }
    settings
}
pub fn reference(candidate: &Value) -> String {
    let parts = [
        "network",
        "url",
        "auth_source",
        "credential_source",
        "cookiefile",
        "username",
    ]
    .map(|k| {
        optional(candidate, k)
            .filter(|v| truthy(v))
            .map(py_str)
            .unwrap_or_default()
    });
    let digest = Sha256::digest(parts.join("|").as_bytes());
    let hex = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("local-core:{}", &hex[..24])
}
pub(crate) fn loopback(url: &str) -> bool {
    let clean = url
        .trim_start_matches(|c: char| c.is_ascii_control() || c == ' ')
        .replace(['\n', '\r', '\t'], "");
    let url = clean.as_str();
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !["http", "https"].contains(&scheme.to_lowercase().as_str()) {
        return false;
    }
    let hostport = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    let host = if hostport.starts_with('[') {
        hostport
            .strip_prefix('[')
            .and_then(|s| s.split_once(']'))
            .map(|(h, _)| h)
            .unwrap_or_default()
    } else {
        hostport.split(':').next().unwrap_or_default()
    };
    if hostport.starts_with('[')
        && host
            .split('%')
            .next()
            .unwrap_or_default()
            .parse::<std::net::Ipv6Addr>()
            .is_err()
    {
        return false;
    }
    let host = kassiber_pytext::strip(host).to_lowercase();
    let ip = if host.contains(':') {
        host.split('%').next().unwrap_or_default()
    } else {
        &host
    };
    host == "localhost"
        || ip
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}
fn rpc_url(network: &str, settings: &BTreeMap<String, String>) -> String {
    let get = |k| {
        settings
            .get(k)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    };
    if let Some(url) =
        get("rpcurl").filter(|u| u.starts_with("http://") || u.starts_with("https://"))
    {
        return url.into();
    }
    let mut host = kassiber_pytext::strip(
        get("rpcconnect")
            .or_else(|| get("rpcbind"))
            .unwrap_or("127.0.0.1"),
    )
    .split(',')
    .next()
    .map(kassiber_pytext::strip)
    .unwrap_or_default();
    if ["", "0.0.0.0", "::", "[::]"].contains(&host) {
        host = "127.0.0.1";
    }
    let mut host_port = None;
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            if let Some(port) = host[end + 1..]
                .strip_prefix(':')
                .filter(|s| !s.is_empty() && s.chars().all(python_is_digit))
            {
                host_port = Some(port);
                host = &host[..=end];
            }
        }
    } else if host.matches(':').count() == 1 {
        if let Some((h, p)) = host
            .rsplit_once(':')
            .filter(|(_, p)| !p.is_empty() && p.chars().all(python_is_digit))
        {
            host = h;
            host_port = Some(p);
        }
    }
    let display = if host.starts_with('[') && host.contains(']') || !host.contains(':') {
        host.to_owned()
    } else {
        format!("[{host}]")
    };
    let default = match network {
        "test" => "18332",
        "regtest" => "18443",
        "signet" => "38332",
        _ => "8332",
    };
    let port = get("rpcport")
        .or(host_port)
        .and_then(|p| kassiber_pytext::parse_int(p).ok())
        .map(|n| n.to_string())
        .unwrap_or_else(|| default.into());
    format!("http://{display}:{port}")
}
pub fn candidates(home: &Path) -> Result<Vec<Value>> {
    let dir = home.join(".bitcoin");
    let conf = dir.join("bitcoin.conf");
    let settings = if conf.exists() {
        match std::fs::read_to_string(&conf) {
            Ok(text) => parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                return Err(AppError::new(
                    "internal_error",
                    "Bitcoin Core configuration is not UTF-8",
                ))
            }
            Err(_) => Settings::new(),
        }
    } else {
        Settings::new()
    };
    let mut seen = BTreeSet::new();
    let mut result = vec![];
    for (network, aliases, parts) in [
        ("main", &["main", "mainnet"][..], &[".cookie"][..]),
        (
            "test",
            &["test", "testnet", "testnet3", "testnet4"][..],
            &["testnet3", ".cookie"][..],
        ),
        (
            "test",
            &["test", "testnet", "testnet3", "testnet4"][..],
            &["testnet4", ".cookie"][..],
        ),
        ("regtest", &["regtest"][..], &["regtest", ".cookie"][..]),
        ("signet", &["signet"][..], &["signet", ".cookie"][..]),
    ] {
        let mut merged = settings.get("global").cloned().unwrap_or_default();
        for alias in aliases {
            if let Some(s) = settings.get(*alias) {
                merged.extend(s.clone());
            }
        }
        let url = rpc_url(network, &merged);
        if !loopback(&url) {
            continue;
        }
        let get = |k| merged.get(k).map(String::as_str).filter(|v| !v.is_empty());
        let cookie = if let Some(raw) = get("rpccookiefile") {
            let mut context = kassiber_paths::Context::from_env().map_err(storage_error)?;
            context.home = home.to_owned();
            let path = context.expand_user(Path::new(raw)).map_err(storage_error)?;
            let path = if path.is_absolute() {
                path
            } else {
                dir.join(path)
            };
            context.expand_user(&path).map_err(storage_error)?
        } else {
            parts.iter().fold(dir.clone(), |p, part| p.join(part))
        };
        let mut add = |auth: &str, credentials: Value, source: &str, identity: &str| {
            if !seen.insert((
                network.to_owned(),
                url.clone(),
                auth.to_owned(),
                identity.to_owned(),
            )) {
                return;
            }
            let mut c = dict([
                ("name", s(format!("local-core-{network}"))),
                ("kind", s("bitcoinrpc")),
                ("chain", s("bitcoin")),
                ("network", s(network)),
                ("url", s(url.clone())),
                ("auth_source", s(auth)),
                ("credential_source", s(source)),
                ("timeout", Value::Int(2.into())),
            ]);
            if let Value::Dict(fields) = credentials {
                for (k, v) in fields {
                    if let kassiber_pyjson::Key::Str(k) = k {
                        replace(&mut c, k.as_str().unwrap(), v);
                    }
                }
            }
            result.push(c);
        };
        if let (Some(user), Some(password)) = (get("rpcuser"), get("rpcpassword")) {
            add(
                "basic",
                dict([("username", s(user)), ("password", s(password))]),
                "bitcoin.conf",
                user,
            );
        }
        if cookie.exists() {
            let cookie = cookie.to_string_lossy();
            add(
                "cookiefile",
                dict([("cookiefile", s(cookie.as_ref()))]),
                if get("rpccookiefile").is_some() {
                    "bitcoin.conf"
                } else {
                    "default"
                },
                &cookie,
            );
        }
    }
    Ok(result)
}
pub fn resolve(home: &Path, requested: &str) -> Result<Value> {
    candidates(home)?
        .into_iter()
        .find(|c| reference(c) == requested)
        .ok_or_else(unavailable)
}
pub fn unavailable() -> AppError {
    AppError::new(
        "validation",
        "Detected Bitcoin Core credentials are no longer available",
    )
    .with_hint("Run Detect my node again or enter RPC credentials manually.")
}

// CPython Unicode 14 isdigit additions beyond decimal characters.
fn python_is_digit(c: char) -> bool {
    kassiber_pytext::decimal_value(u32::from(c)).is_some()
        || matches!(u32::from(c),
            0xb2..=0xb3 | 0xb9 | 0x1369..=0x1371 | 0x19da | 0x2070 | 0x2074..=0x2079 | 0x2080..=0x2089 | 0x2460..=0x2468 | 0x2474..=0x247c | 0x2488..=0x2490 | 0x24ea | 0x24f5..=0x24fd | 0x24ff | 0x2776..=0x277e | 0x2780..=0x2788 | 0x278a..=0x2792 | 0x10a40..=0x10a43 | 0x10e60..=0x10e68 | 0x11052..=0x1105a | 0x1f100..=0x1f10a
        )
}
