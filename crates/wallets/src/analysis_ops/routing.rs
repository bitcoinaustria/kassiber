//! Saved backend rows and their routing identity. Reading configuration does no I/O.
use super::*;

/// `backends.get_db_backend`: the stored row only, never runtime overlays.
pub(super) fn get_db_backend(conn: &Connection, name: &str) -> Result<Value> {
    let row = first(
        conn,
        "SELECT * FROM backends WHERE name = ?",
        [lower(name)],
    )?
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Backend '{name}' not found in the database"),
        )
        .with_hint("Use `kassiber backends list` to see configured backends, or `kassiber backends create` to add one.")
    })?;
    let or_empty = |key: &str| {
        if truthy(field(&row, key)) {
            field(&row, key).clone()
        } else {
            s("")
        }
    };
    let mut backend = dict([
        ("name", field(&row, "name").clone()),
        ("kind", field(&row, "kind").clone()),
        ("chain", or_empty("chain")),
        ("network", or_empty("network")),
        ("url", field(&row, "url").clone()),
        ("batch_size", field(&row, "batch_size").clone()),
        ("auth_header", or_empty("auth_header")),
        ("token", or_empty("token")),
        ("timeout", field(&row, "timeout").clone()),
        ("tor_proxy", or_empty("tor_proxy")),
        ("notes", or_empty("notes")),
        ("source", s("database")),
        ("created_at", field(&row, "created_at").clone()),
        ("updated_at", field(&row, "updated_at").clone()),
    ]);
    // `_load_backend_config`: only a JSON object overlays the row.
    if let Value::Str(raw) = field(&row, "config_json") {
        if let Ok(Value::Dict(pairs)) = kassiber_pytext::loads(&py_text(&Value::Str(raw.clone()))) {
            for (key, value) in pairs {
                if let kassiber_pyjson::Key::Str(key) = key {
                    set(&mut backend, &py_text(&Value::Str(key)), value);
                }
            }
        }
    }
    Ok(backend)
}

pub(super) fn py_text(value: &Value) -> String {
    match value {
        Value::Str(text) => text
            .code_points()
            .map(|c| char::from_u32(c).unwrap_or('\u{fffd}'))
            .collect(),
        other => kassiber_envelope::py_str(other),
    }
}

/// `backend_value`: the first non-blank `str()` across keys.
pub(super) fn backend_value(backend: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match get(backend, key) {
        None | Some(Value::Null) => None,
        Some(value) => {
            let text = kassiber_pytext::strip(&py_text(value)).to_owned();
            (!text.is_empty()).then_some(text)
        }
    })
}

/// Python `int(text)` over a stripped decimal literal.
fn python_int(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    text.replace('_', "").parse().ok()
}

/// `backend_timeout(backend)`, defaulting to 30 seconds.
pub(super) fn backend_timeout(backend: &Value) -> Result<i64> {
    match backend_value(backend, &["timeout"]) {
        None => Ok(30),
        Some(text) => python_int(&text)
            .ok_or_else(|| AppError::new("app_error", format!("Invalid integer value: {text}"))),
    }
}

struct Split {
    scheme: String,
    netloc: String,
    path: String,
}

/// `urllib.parse.urlsplit` (Python 3.11) for scheme, netloc and path.
fn urlsplit(raw: &str) -> std::result::Result<Split, ()> {
    let mut url: String = raw
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut scheme = String::new();
    if let Some(i) = url.find(':') {
        let candidate = &url[..i];
        if i > 0
            && candidate.starts_with(|c: char| c.is_ascii_alphabetic())
            && candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            scheme = candidate.to_ascii_lowercase();
            url = url[i + 1..].to_owned();
        }
    }
    let mut netloc = String::new();
    if url.starts_with("//") {
        let end = url[2..].find(['/', '?', '#']).map_or(url.len(), |n| n + 2);
        netloc = url[2..end].to_owned();
        url = url[end..].to_owned();
        if netloc.contains('[') != netloc.contains(']') {
            return Err(());
        }
    }
    let path = url
        .split('#')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_owned();
    Ok(Split {
        scheme,
        netloc,
        path,
    })
}

/// `[scheme, hostname, port, path?]` for one configured endpoint.
fn endpoint(value: Option<&str>, path: bool) -> std::result::Result<Value, ()> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Ok(Value::Null);
    };
    let split = urlsplit(value)?;
    let hostinfo = split
        .netloc
        .rsplit_once('@')
        .map_or(split.netloc.as_str(), |(_, h)| h);
    let (host, port) = match hostinfo.split_once('[') {
        Some((_, bracketed)) => {
            let (host, rest) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            if !host.starts_with('v')
                && host
                    .split('%')
                    .next()
                    .unwrap_or("")
                    .parse::<std::net::Ipv6Addr>()
                    .is_err()
            {
                return Err(());
            }
            (host, rest.split_once(':').map_or("", |(_, p)| p))
        }
        None => hostinfo.split_once(':').unwrap_or((hostinfo, "")),
    };
    let hostname = if host.is_empty() {
        Value::Null
    } else {
        let (name, zone) = host
            .split_once('%')
            .map_or((host, None), |(n, z)| (n, Some(z)));
        s(match zone {
            Some(zone) => format!("{}%{zone}", lower(name)),
            None => lower(name),
        }
        .as_str())
    };
    let port = if port.is_empty() {
        Value::Null
    } else {
        if !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(());
        }
        match port.trim_start_matches('0').parse::<u32>() {
            Ok(n) if n <= 65535 => Value::Int(n.into()),
            _ if port.bytes().all(|b| b == b'0') => Value::Int(0.into()),
            _ => return Err(()),
        }
    };
    Ok(Value::List(vec![
        s(split.scheme.as_str()),
        hostname,
        port,
        if path {
            s(split.path.as_str())
        } else {
            Value::Null
        },
    ]))
}

/// `chain_analysis_acquisition._routing_identity`: destinations and TLS
/// choices, never credentials or their fingerprints.
pub(super) fn routing_identity(backend: &Value) -> Result<Value> {
    let failure = || invalid("Invalid backend routing configuration");
    let url = match field(backend, "url") {
        value if !truthy(value) => None,
        Value::Str(text) => Some(py_text(&Value::Str(text.clone()))),
        _ => return Err(failure()),
    };
    Ok(Value::List(vec![
        endpoint(url.as_deref(), true).map_err(|_| failure())?,
        endpoint(
            backend_value(backend, &["tor_proxy", "proxy"]).as_deref(),
            false,
        )
        .map_err(|_| failure())?,
        backend_value(backend, &["certificate"]).map_or(Value::Null, |v| s(v.as_str())),
        backend_value(backend, &["insecure"]).map_or(Value::Null, |v| s(v.as_str())),
        Value::Int(backend_timeout(backend)?.into()),
    ]))
}

pub(super) fn lower(text: &str) -> String {
    kassiber_pytext::lower(text)
}
