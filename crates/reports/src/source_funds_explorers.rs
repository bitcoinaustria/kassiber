use super::*;
use kassiber_pytext::{lower, strip};
use kassiber_wallets::descriptors::{normalize_chain, normalize_network};

fn optional(row: &Value, key: &str) -> String {
    get(row, key)
        .filter(|v| !matches!(v, Value::Null))
        .map(|v| strip(&py_str(v)).to_owned())
        .unwrap_or_default()
}

fn base_url(raw: &str) -> Result<Option<String>> {
    let raw = raw
        .trim_start_matches(|c| c <= ' ')
        .replace(['\t', '\r', '\n'], "");
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Ok(None);
    };
    let scheme = lower(scheme);
    if !["http", "https"].contains(&scheme.as_str()) {
        return Ok(None);
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() {
        return Ok(None);
    }
    if authority.contains('[') != authority.contains(']') {
        return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
    }
    if authority.contains(|c: char| "⁇⁈⁉℀℁℅℆⩴︓︖﹕﹖﹟﹫＃／：？＠".contains(c))
    {
        return Err(AppError::new(
            "internal_error",
            format!("netloc '{authority}' contains invalid characters under NFKC normalization"),
        ));
    }
    let host = authority.rsplit('@').next().unwrap_or_default();
    let (host, port) = if let Some(bracketed) = host.strip_prefix('[') {
        let Some((host, suffix)) = bracketed.split_once(']') else {
            return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
        };
        let address = host.split('%').next().unwrap_or_default();
        if let Some(future) = address.strip_prefix('v') {
            if !future.split_once('.').is_some_and(|(v, a)| {
                !v.is_empty() && v.bytes().all(|c| c.is_ascii_hexdigit()) && !a.is_empty()
            }) {
                return Err(AppError::new(
                    "internal_error",
                    "IPvFuture address is invalid",
                ));
            }
        } else if address
            .parse::<std::net::IpAddr>()
            .ok()
            .is_none_or(|v| v.is_ipv4())
        {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "{} does not appear to be an IPv4 or IPv6 address",
                    kassiber_pytext::repr_str(host)
                ),
            ));
        }
        if !suffix.is_empty() && !suffix.starts_with(':') {
            return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
        }
        let host = host.split_once('%').map_or_else(
            || lower(host),
            |(address, zone)| format!("{}%{zone}", lower(address)),
        );
        (format!("[{host}]"), suffix.strip_prefix(':'))
    } else {
        let (host, port) = host
            .split_once(':')
            .map_or((host, None), |(h, p)| (h, Some(p)));
        (lower(host), port)
    };
    if host.is_empty() {
        return Ok(None);
    }
    let port = match port {
        Some("") | None => String::new(),
        Some(port) if port.bytes().all(|c| c.is_ascii_digit()) => match port.parse::<u16>() {
            Ok(n) => format!(":{n}"),
            Err(_) => return Ok(None),
        },
        Some(_) => return Ok(None),
    };
    let path = rest[end..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('/');
    let path = if path.to_ascii_lowercase().ends_with("/api") {
        &path[..path.len() - 4]
    } else {
        path
    };
    Ok(Some(format!("{scheme}://{host}{port}{path}")))
}

pub fn link(conn: &Connection, txid: &str, tx: &Value) -> Result<Option<Value>> {
    let checked = strip(txid);
    if checked.len() != 64 || !checked.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let raw = value(tx, "wallet_config_json");
    let mut valid_config = true;
    let config = if truthy(raw) {
        match kassiber_pytext::loads(&py_str(raw)) {
            Ok(Value::Dict(v)) => Value::Dict(v),
            Ok(_) => dict::<&str>([]),
            Err(_) => {
                valid_config = false;
                dict::<&str>([])
            }
        }
    } else {
        dict::<&str>([])
    };
    let chain = optional(&config, "chain");
    let chain = if chain.is_empty() {
        if kassiber_setup::accounts::normalize_asset(&text(tx, "asset")) == "LBTC" {
            "liquid"
        } else {
            "bitcoin"
        }
    } else {
        &chain
    };
    let (chain, network) = if valid_config {
        normalize_chain(chain)
            .and_then(|chain| {
                normalize_network(&chain, &optional(&config, "network"))
                    .map(|network| (chain, network))
            })
            .unwrap_or_default()
    } else {
        (String::new(), String::new())
    };
    let default = rows(
        conn,
        "SELECT value FROM settings WHERE key='default_backend'",
        [],
    )?
    .first()
    .map(|r| text(r, "value"))
    .unwrap_or_default();
    let mut candidates = Vec::new();
    for mut row in rows(conn, "SELECT * FROM backends ORDER BY name ASC", [])? {
        if let Ok(Value::Dict(fields)) = kassiber_pytext::loads(&optional(&row, "config_json")) {
            for (k, v) in fields {
                if let kassiber_pyjson::Key::Str(k) = k {
                    replace(&mut row, k.as_str().unwrap_or_default(), v);
                }
            }
        }
        if !["esplora", "mempool", "liquid-esplora"]
            .contains(&lower(&optional(&row, "kind")).as_str())
        {
            continue;
        }
        let bc = optional(&row, "chain");
        let Ok(bc) = normalize_chain(if bc.is_empty() { "bitcoin" } else { &bc }) else {
            continue;
        };
        let bn = optional(&row, "network");
        let Ok(bn) = normalize_network(&bc, &bn) else {
            continue;
        };
        if !chain.is_empty() && chain != bc || !network.is_empty() && network != bn {
            continue;
        }
        let name = optional(&row, "name");
        let own = optional(&row, "infrastructure_owner") == "self";
        if ["fulcrum", "liquid", "liquid-blockstream", "mempool"].contains(&name.as_str()) && !own {
            continue;
        }
        if let Some(base) = base_url(&optional(&row, "url"))? {
            let label = optional(&row, "display_name");
            candidates.push((
                name != default,
                !own,
                name.clone(),
                if label.is_empty() { name } else { label },
                base,
            ));
        }
    }
    candidates.sort();
    let selected = if let Some((_, _, _, label, base)) = candidates.into_iter().next() {
        Some((label, base))
    } else {
        match (chain.as_str(), network.as_str()) {
            ("bitcoin", "main") => Some(("mempool.space".into(), "https://mempool.space".into())),
            ("bitcoin", "test") => Some((
                "mempool.space testnet".into(),
                "https://mempool.space/testnet".into(),
            )),
            ("bitcoin", "signet") => Some((
                "mempool.space signet".into(),
                "https://mempool.space/signet".into(),
            )),
            ("liquid", "liquidv1") => {
                Some(("Liquid Network".into(), "https://liquid.network".into()))
            }
            ("liquid", "liquidtestnet") => Some((
                "Liquid Network Testnet".into(),
                "https://liquid.network/testnet".into(),
            )),
            _ => None,
        }
    };
    Ok(selected.map(|(label, base)| {
        dict([
            ("txid", s(txid)),
            (
                "asset",
                s(kassiber_setup::accounts::normalize_asset(&text(
                    tx, "asset",
                ))),
            ),
            ("chain", s(chain)),
            ("network", s(network)),
            ("label", s(label)),
            ("url", s(format!("{base}/tx/{txid}"))),
        ])
    }))
}
