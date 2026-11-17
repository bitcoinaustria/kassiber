//! components/kb/backendConnectionRows.ts `regtestBackendConnections`, as AddConnectionDialog
//! feeds it: `ui.backends.settings.list` rows → `backendRowToSettingsBackend`
//! (settings/SettingsModel.ts) → the local (regtest) backends the backend-settings step lists.
//! With `backendProtocolLabel`, `backendNetFromRow` (SettingsModel.ts) and `endpointWithPort` /
//! `settingsHashForConnection` (lib/connectionHealth.ts).

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalBackend {
    /// `backend:<name>`.
    pub id: String,
    /// The backend's name (`backendId`).
    pub backend_id: Option<String>,
    /// Display name, else the name.
    pub label: String,
    /// `endpointWithPort(url)`.
    pub endpoint: String,
    /// `backendProtocolLabel`.
    pub sync_source: String,
    /// Settings section: bitcoin / liquid / lightning / market.
    pub settings_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Net {
    Btc,
    Liquid,
    Ln,
    Fx,
}

fn s<'a>(row: &'a Value, k: &str) -> Option<&'a str> {
    row.get(k).and_then(Value::as_str)
}

/// `backendNetFromRow`.
pub fn net_from_row(row: &Value) -> Net {
    let chain = s(row, "chain").unwrap_or("").to_lowercase();
    let kind = s(row, "kind").unwrap_or("").to_lowercase();
    if kind == "lnd" || kind == "coreln" {
        return Net::Ln;
    }
    if chain == "liquid" || kind == "liquid-esplora" {
        return Net::Liquid;
    }
    if chain == "fx"
        || chain == "market"
        || ["coinbase-exchange", "coingecko", "kraken-csv"].contains(&kind.as_str())
    {
        return Net::Fx;
    }
    Net::Btc
}

/// `backendProtocolLabel`.
pub fn protocol_label(kind: &str, net_kind: Net) -> &'static str {
    match kind.to_lowercase().as_str() {
        "esplora" | "liquid-esplora" => "Explorer API",
        "electrum" => "Electrum / Fulcrum",
        "bitcoinrpc" => "Bitcoin Core RPC",
        "btcpay" => "BTCPay",
        "lnd" => "LND REST",
        "coreln" => "Core Lightning",
        _ if net_kind == Net::Fx => "Rate provider",
        _ => "Endpoint",
    }
}

/// `settingsHashForConnection`.
pub fn settings_hash(net_kind: Net) -> &'static str {
    match net_kind {
        Net::Btc => "bitcoin",
        Net::Liquid => "liquid",
        Net::Ln => "lightning",
        Net::Fx => "market",
    }
}

fn special_default_port(scheme: &str) -> Option<&'static str> {
    match scheme {
        "https" | "wss" => Some("443"),
        "http" | "ws" => Some("80"),
        "ftp" => Some("21"),
        _ => None,
    }
}

/// `endpointWithPort`: `scheme://host:port[/path][?query]`, with the scheme's default port
/// filled in (443/80, Electrum's 50002 for ssl and 50001 for tcp). Unparseable input is
/// returned trimmed, as `new URL` failing does on the web.
pub fn endpoint_with_port(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "Missing endpoint".into();
    }
    let full = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let Some((scheme, rest)) = full.split_once("://") else {
        return trimmed.to_owned();
    };
    let scheme_ok = scheme
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    if !scheme_ok || rest.chars().any(char::is_whitespace) {
        return trimmed.to_owned();
    }
    let scheme = scheme.to_lowercase();
    let special = matches!(
        scheme.as_str(),
        "http" | "https" | "ws" | "wss" | "ftp" | "file"
    );
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(auth_end);
    // userinfo@ is dropped from the result (only host and port are printed).
    let hostport = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let (host, port) = if let Some(stripped) = hostport.strip_prefix('[') {
        let Some(close) = stripped.find(']') else {
            return trimmed.to_owned();
        };
        let host = format!("[{}]", &stripped[..close]);
        let after = &stripped[close + 1..];
        match after.strip_prefix(':') {
            Some(p) => (host, p.to_owned()),
            None if after.is_empty() => (host, String::new()),
            None => return trimmed.to_owned(),
        }
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_owned(), p.to_owned()),
            None => (hostport.to_owned(), String::new()),
        }
    };
    if special && host.is_empty() {
        return trimmed.to_owned();
    }
    if !port.chars().all(|c| c.is_ascii_digit()) || port.parse::<u32>().is_ok_and(|p| p > 65535) {
        return trimmed.to_owned();
    }
    let host = if special { host.to_lowercase() } else { host };
    // `new URL` drops a port equal to the scheme's default, then the fallback adds it back.
    let port =
        if port.is_empty() || special_default_port(&scheme) == Some(port.trim_start_matches('0')) {
            String::new()
        } else {
            port.trim_start_matches('0').to_owned()
        };
    let port = if port.is_empty() {
        match scheme.as_str() {
            "https" | "wss" => "443".to_owned(),
            "http" | "ws" => "80".to_owned(),
            "ssl" => "50002".to_owned(),
            "tcp" => "50001".to_owned(),
            _ => String::new(),
        }
    } else {
        port
    };
    // `parsed.hostname` keeps IPv6 brackets, so the web wraps them once more.
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    let tail = tail.split('#').next().unwrap_or("");
    let (path, search) = match tail.find('?') {
        Some(i) => (&tail[..i], &tail[i..]),
        None => (tail, ""),
    };
    let search = if search == "?" { "" } else { search };
    let path = if path == "/" { "" } else { path };
    format!(
        "{scheme}://{host}{}{path}{search}",
        if port.is_empty() {
            String::new()
        } else {
            format!(":{port}")
        }
    )
}

fn is_regtest(row: &Value) -> bool {
    matches!(
        s(row, "network").unwrap_or("").to_lowercase().as_str(),
        "regtest" | "elementsregtest"
    )
}

/// `regtestBackendConnections(rows.map(backendRowToSettingsBackend))`.
pub fn regtest_backend_connections(data: &Value) -> Vec<LocalBackend> {
    let rows: &[Value] = data
        .get("backends")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    // `visibleConnectionBackends` keeps every regtest row when any exists; the final filter
    // keeps only regtest rows either way.
    rows.iter()
        .filter(|r| is_regtest(r))
        .map(|row| {
            let id = s(row, "name")
                .filter(|n| !n.is_empty())
                .unwrap_or("backend")
                .to_owned();
            let label = s(row, "display_name")
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| id.clone());
            let has_url = row.get("has_url").and_then(Value::as_bool);
            let url = s(row, "url")
                .filter(|u| !u.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    if has_url == Some(true) {
                        "Configured endpoint".into()
                    } else {
                        "Missing endpoint".into()
                    }
                });
            let net_kind = net_from_row(row);
            LocalBackend {
                id: format!("backend:{id}"),
                backend_id: Some(id),
                label,
                endpoint: endpoint_with_port(&url),
                sync_source: protocol_label(s(row, "kind").unwrap_or(""), net_kind).to_owned(),
                settings_hash: Some(settings_hash(net_kind).to_owned()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn endpoints_get_default_ports() {
        assert_eq!(
            endpoint_with_port("tcp://127.0.0.1:18543"),
            "tcp://127.0.0.1:18543"
        );
        assert_eq!(
            endpoint_with_port("http://127.0.0.1:18544/api"),
            "http://127.0.0.1:18544/api"
        );
        assert_eq!(
            endpoint_with_port("ssl://electrum.example"),
            "ssl://electrum.example:50002"
        );
        assert_eq!(
            endpoint_with_port("tcp://Host.example"),
            "tcp://Host.example:50001"
        );
        assert_eq!(
            endpoint_with_port("mempool.example/api/"),
            "https://mempool.example:443/api/"
        );
        assert_eq!(
            endpoint_with_port("https://Example.com:443/"),
            "https://example.com:443"
        );
        assert_eq!(
            endpoint_with_port("http://example.com/?q=1"),
            "http://example.com:80?q=1"
        );
        assert_eq!(endpoint_with_port(""), "Missing endpoint");
        assert_eq!(
            endpoint_with_port("Configured endpoint"),
            "Configured endpoint"
        );
    }

    #[test]
    fn lists_only_regtest_rows() {
        let data = json!({"backends": [
            {"name": "bitcoin-electrum-regtest", "kind": "electrum", "chain": "bitcoin", "network": "regtest", "url": "tcp://127.0.0.1:18543", "display_name": "Bitcoin Electrum Regtest"},
            {"name": "liquid-mempool-regtest", "kind": "liquid-esplora", "chain": "liquid", "network": "elementsregtest", "url": "http://127.0.0.1:18546/api"},
            {"name": "mempool", "kind": "esplora", "chain": "bitcoin", "network": "main", "url": "https://mempool.space/api", "is_default": true},
            {"name": "core-regtest", "kind": "bitcoinrpc", "chain": "bitcoin", "network": "regtest", "has_url": true},
        ]});
        let rows = regtest_backend_connections(&data);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            LocalBackend {
                id: "backend:bitcoin-electrum-regtest".into(),
                backend_id: Some("bitcoin-electrum-regtest".into()),
                label: "Bitcoin Electrum Regtest".into(),
                endpoint: "tcp://127.0.0.1:18543".into(),
                sync_source: "Electrum / Fulcrum".into(),
                settings_hash: Some("bitcoin".into()),
            }
        );
        assert_eq!(rows[1].label, "liquid-mempool-regtest");
        assert_eq!(rows[1].sync_source, "Explorer API");
        assert_eq!(rows[1].settings_hash.as_deref(), Some("liquid"));
        assert_eq!(rows[2].endpoint, "Configured endpoint");
        assert_eq!(rows[2].sync_source, "Bitcoin Core RPC");
    }

    #[test]
    fn protocol_labels() {
        assert_eq!(protocol_label("lnd", Net::Ln), "LND REST");
        assert_eq!(protocol_label("coingecko", Net::Fx), "Rate provider");
        assert_eq!(protocol_label("mempool", Net::Btc), "Endpoint");
    }
}
