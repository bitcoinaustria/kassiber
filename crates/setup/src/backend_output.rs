use super::*;
const SAFE: &[&str] = &[
    "name",
    "kind",
    "chain",
    "network",
    "batch_size",
    "timeout",
    "notes",
    "source",
    "created_at",
    "updated_at",
    "default",
    "is_default",
];
const PRESENCE: &[(&str, &[&str])] = &[
    ("has_auth_header", &["auth_header"]),
    ("has_token", &["token"]),
    ("has_certificate", &["certificate"]),
    ("has_commando_peer_id", &["commando_peer_id"]),
    ("has_lightning_dir", &["lightning_dir"]),
    ("has_rpc_file", &["rpc_file"]),
    ("has_cookiefile", &["cookiefile", "cookie_file"]),
    ("has_username", &["username", "rpcuser", "rpc_user"]),
    ("has_password", &["password", "rpcpassword", "rpc_password"]),
];
/// Python urlsplit's display projection, without URL resolution or a client.
/// Preserve the endpoint/path, remove userinfo/query/fragment, normalize host.
pub(super) fn url(v: &Value, proxy: bool) -> Result<String> {
    let Some(raw) = string(v) else {
        return Ok(String::new());
    };
    let has_scheme = raw.contains("://");
    let input = if proxy && !has_scheme {
        format!("socks5h://{raw}")
    } else {
        raw.clone()
    };
    let clean = input
        .trim_start_matches(|c: char| c.is_ascii_control() || c == ' ')
        .replace(['\n', '\r', '\t'], "");
    let Some((scheme, rest)) = clean.split_once("://") else {
        return Ok(raw);
    };
    if scheme.is_empty()
        || !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return Ok(raw);
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let (userinfo, hostport) = authority
        .rsplit_once('@')
        .map_or(("", authority), |(u, h)| (u, h));
    let (host, port) = if hostport.starts_with('[') {
        let Some(i) = hostport.find(']') else {
            return Ok(raw);
        };
        (&hostport[..=i], hostport[i + 1..].strip_prefix(':'))
    } else {
        hostport
            .split_once(':')
            .map_or((hostport, None), |(h, p)| (h, Some(p)))
    };
    if host.is_empty() {
        return Ok(raw);
    }
    let mut host = host.to_lowercase();
    if let Some(port) = port.filter(|p| !p.is_empty()) {
        let p = port
            .parse::<u16>()
            .map_err(|_| AppError::new("validation", "Invalid backend URL port"))?;
        host.push_str(&format!(":{p}"));
    }
    if !userinfo.is_empty() {
        host = format!("{}@{host}", if proxy { "redacted" } else { "<redacted>" });
    }
    if proxy {
        return Ok(if has_scheme {
            format!("{}://{host}", scheme.to_lowercase())
        } else {
            host
        });
    }
    let path = rest[end..].split(['?', '#']).next().unwrap_or_default();
    Ok(format!("{}://{host}{path}", scheme.to_lowercase()))
}
pub(super) fn redact(b: &Value) -> Result<Value> {
    let mut out = dict::<&str>([]);
    for k in SAFE {
        if let Some(v) = get(b, k) {
            replace(&mut out, k, v.clone());
        }
    }
    if let Some(v) = get(b, "url") {
        replace(&mut out, "url", s(url(v, false)?));
    }
    if let Some(v) = get(b, "tor_proxy") {
        replace(&mut out, "tor_proxy", s(url(v, true)?));
    }
    for k in [
        "display_name",
        "infrastructure_owner",
        "insecure",
        "silent_payments",
        "walletprefix",
    ] {
        let v = value(b, k);
        if string(v).is_none() {
            continue;
        }
        let v = if ["insecure", "silent_payments"].contains(&k) {
            boolean(v).unwrap_or_else(|_| v.clone())
        } else {
            s(string(v).unwrap())
        };
        replace(&mut out, k, v);
    }
    if get(&out, "silent_payments").is_none()
        && [
            "silent_payment_scan_file",
            "silent_payment_scan_path",
            "sp_scan_file",
            "sp_scan_path",
        ]
        .iter()
        .any(|k| string(value(b, k)).is_some())
    {
        replace(&mut out, "silent_payments", Value::Bool(true));
    }
    for (flag, keys) in PRESENCE {
        replace(
            &mut out,
            flag,
            Value::Bool(keys.iter().any(|k| string(value(b, k)).is_some())),
        );
    }
    Ok(out)
}
pub(super) fn list(config: &Config) -> Result<Value> {
    let mut result = vec![];
    for (name, b) in &config.view {
        let mut row = dict::<&str>([]);
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
            "tor_proxy",
            "insecure",
            "certificate",
            "commando_peer_id",
            "display_name",
            "lightning_dir",
            "rpc_file",
            "infrastructure_owner",
            "silent_payments",
            "silent_payment_scan_file",
            "silent_payment_scan_path",
            "username",
            "password",
        ] {
            replace(&mut row, k, get(b, k).cloned().unwrap_or_else(|| s("")));
        }
        replace(&mut row, "name", s(name));
        replace(
            &mut row,
            "default",
            s(if *name == config.default_backend {
                "yes"
            } else {
                ""
            }),
        );
        let raw_url = value(&row, "url").clone();
        let mut out = redact(&row)?;
        let safe = string(&raw_url).is_some_and(|v| {
            (v.to_lowercase().starts_with("http://") || v.to_lowercase().starts_with("https://"))
                && get(&out, "url") == Some(&raw_url)
        }) && [
            "has_auth_header",
            "has_token",
            "has_cookiefile",
            "has_username",
            "has_password",
        ]
        .iter()
        .all(|k| !flag(&out, k))
            && !truthy(value(b, "tor_proxy"));
        replace(&mut out, "url_safe_for_http_probe", Value::Bool(safe));
        result.push(out);
    }
    Ok(Value::List(result))
}
