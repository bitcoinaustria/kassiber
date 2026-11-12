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
fn malformed(raw: &str) -> String {
    // Exact ASCII fullmatch of Python's _PLAIN_BACKEND_LOCATION_RE. This is
    // a disclosure rule, not DNS/IP validation (even [.] and port 99999 match).
    let (host, suffix) = if raw.starts_with('[') {
        raw.find(']')
            .map_or(("", raw), |end| (&raw[..=end], &raw[end + 1..]))
    } else {
        raw.split_once(':')
            .map_or((raw, ""), |(host, _)| (host, &raw[host.len()..]))
    };
    let host_ok = if let Some(address) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        !address.is_empty()
            && address
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
    } else {
        host.as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && host
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    let port_ok = suffix.is_empty()
        || suffix.strip_prefix(':').is_some_and(|port| {
            (1..=5).contains(&port.len()) && port.bytes().all(|b| b.is_ascii_digit())
        });
    if host_ok && port_ok {
        raw.to_owned()
    } else {
        "<redacted>".to_owned()
    }
}
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
        return Ok(malformed(&raw));
    };
    if scheme.is_empty()
        || !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return Ok(malformed(&raw));
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let (userinfo, hostport) = authority
        .rsplit_once('@')
        .map_or(("", authority), |(u, h)| (u, h));
    let (host, port) = if hostport.starts_with('[') {
        let Some(i) = hostport.find(']') else {
            return Ok(malformed(&raw));
        };
        let address = &hostport[1..i];
        let ipvfuture = address
            .strip_prefix('v')
            .and_then(|s| s.split_once('.'))
            .is_some_and(|(version, host)| {
                !version.is_empty()
                    && version.bytes().all(|b| b.is_ascii_hexdigit())
                    && !host.is_empty()
            });
        let ipv6 = address
            .split('%')
            .next()
            .unwrap_or_default()
            .parse::<std::net::Ipv6Addr>()
            .is_ok();
        if !ipvfuture && !ipv6 {
            return Ok(malformed(&raw));
        }
        (&hostport[..=i], hostport[i + 1..].strip_prefix(':'))
    } else {
        if hostport.contains(['[', ']']) {
            return Ok(malformed(&raw));
        }
        hostport
            .split_once(':')
            .map_or((hostport, None), |(h, p)| (h, Some(p)))
    };
    if host.is_empty() {
        return Ok(malformed(&raw));
    }
    // CPython lowercases the address but preserves an IPv6 scope identifier.
    let mut host = host.split_once('%').map_or_else(
        || host.to_lowercase(),
        |(address, scope)| format!("{}%{scope}", address.to_lowercase()),
    );
    if let Some(port) = port.filter(|p| !p.is_empty()) {
        let Ok(p) = port.parse::<u16>() else {
            return Ok(malformed(&raw));
        };
        if !port.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(malformed(&raw));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_backend_location_fullmatch_preserves_only_the_python_allowlist() {
        for raw in [
            "a",
            "electrum.example:50002",
            "127.0.0.1:8332",
            "[::1]:50001",
            "node.local",
            "A_b-c.d9",
            "[.]",
            "[bad]:99999",
            "node:00000",
        ] {
            assert_eq!(malformed(raw), raw, "{raw}");
            assert_eq!(url(&s(raw), false).unwrap(), raw, "{raw}");
        }
        for raw in [
            "",
            "host:",
            "host:123456",
            "host:１２",
            "host:synthetic-token",
            "host:50002?token=synthetic-token",
            "alice@host:50002",
            "host:50002#synthetic-token",
            "host/path",
            "host\n",
            "-host",
            "host.",
            "éxample",
            "[]",
            "[::1]suffix",
            "[::1]:",
            "[::1]:123456",
            "[::1%secret]",
            "[invalid]",
        ] {
            assert_eq!(malformed(raw), "<redacted>", "{raw}");
        }
        for (raw, expected) in [
            ("electrum.example:50002", "electrum.example:50002"),
            ("[::1]:50001", "[::1]:50001"),
            ("node:99999", "node:99999"),
            ("[.]", "[.]"),
            ("node:123456", "<redacted>"),
            ("node:synthetic-token", "<redacted>"),
            ("[.]secret", "<redacted>"),
        ] {
            assert_eq!(url(&s(raw), true).unwrap(), expected, "proxy {raw}");
        }
    }
}
