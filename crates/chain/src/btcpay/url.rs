//! `normalize_server_url` and `server_url_identity` (`kassiber/btcpay/client.py`)
//! over CPython 3.11's `urlsplit`, frozen by `python_btcpay_urls.json`.
//! Lone surrogates (possible in JSON input) ride through as private code points.
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::PyStr;

const SURROGATE_BASE: u32 = 0x10_FC00;

/// A Python `str` as Rust text; lone surrogates map into plane 16's tail.
pub fn from_py(text: &PyStr) -> String {
    text.code_points()
        .map(|p| {
            char::from_u32(p)
                .or_else(|| char::from_u32(SURROGATE_BASE + p - 0xD800))
                .expect("code point")
        })
        .collect()
}

pub fn to_py(text: &str) -> PyStr {
    PyStr::from_code_points(text.chars().map(|c| {
        let p = u32::from(c);
        if (SURROGATE_BASE..SURROGATE_BASE + 0x800).contains(&p) {
            p - SURROGATE_BASE + 0xD800
        } else {
            p
        }
    }))
    .expect("valid code points")
}

/// The parts of `urlsplit` the normalizer reads.
pub(crate) struct Split {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Invalid;

/// Characters whose NFKC form contains a netloc delimiter (`_checknetloc`).
const NFKC_DELIMITERS: [char; 19] = [
    '\u{2047}', '\u{2048}', '\u{2049}', '\u{2100}', '\u{2101}', '\u{2105}', '\u{2106}', '\u{2a74}',
    '\u{fe13}', '\u{fe16}', '\u{fe55}', '\u{fe56}', '\u{fe5f}', '\u{fe6b}', '\u{ff03}', '\u{ff0f}',
    '\u{ff1a}', '\u{ff1f}', '\u{ff20}',
];

fn bracketed_host_valid(host: &str) -> bool {
    if let Some(rest) = host.strip_prefix('v') {
        // `\Av[a-fA-F0-9]+\..+\Z`
        let digits = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
        return digits > 0
            && rest[digits..]
                .strip_prefix('.')
                .is_some_and(|tail| !tail.is_empty());
    }
    // `ipaddress.ip_address`, refusing IPv4 inside brackets.
    let (address, scope) = match host.split_once('%') {
        Some((address, scope)) => (address, Some(scope)),
        None => (host, None),
    };
    if scope.is_some_and(|scope| scope.is_empty() || scope.contains('%')) {
        return false;
    }
    address.parse::<std::net::Ipv6Addr>().is_ok()
}

/// `urllib.parse.urlsplit(url)` for the scheme, netloc and path.
pub(crate) fn urlsplit(url: &str) -> Result<Split, Invalid> {
    let url = url.trim_start_matches(|c: char| c <= ' ');
    let mut url: String = url
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut scheme = String::new();
    if let Some(i) = url.find(':') {
        let first = url.chars().next().expect("non-empty");
        if i > 0
            && first.is_ascii_alphabetic()
            && url[..i]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        {
            scheme = url[..i].to_ascii_lowercase();
            url = url[i + 1..].to_owned();
        }
    }
    let mut netloc = String::new();
    if url.starts_with("//") {
        let end = url[2..].find(['/', '?', '#']).map_or(url.len(), |i| i + 2);
        netloc = url[2..end].to_owned();
        url = url[end..].to_owned();
        if netloc.contains('[') != netloc.contains(']') {
            return Err(Invalid);
        }
        if netloc.contains('[') {
            // `_check_bracketed_netloc`: nothing before the bracket, and only
            // a port delimiter after it; then `_check_bracketed_host`.
            let info = netloc.rsplit_once('@').map_or(netloc.as_str(), |(_, h)| h);
            if let Some((before, bracketed)) = info.split_once('[') {
                let (host, port) = bracketed.split_once(']').unwrap_or((bracketed, ""));
                if !before.is_empty()
                    || (!port.is_empty() && !port.starts_with(':'))
                    || !bracketed_host_valid(host)
                {
                    return Err(Invalid);
                }
            }
        }
    }
    if let Some((before, _)) = url.split_once('#') {
        url = before.to_owned();
    }
    if let Some((before, _)) = url.split_once('?') {
        url = before.to_owned();
    }
    if !netloc.is_ascii() && netloc.contains(NFKC_DELIMITERS) {
        return Err(Invalid);
    }
    Ok(Split {
        scheme,
        netloc,
        path: url,
    })
}

impl Split {
    fn hostinfo(&self) -> &str {
        self.netloc
            .rsplit_once('@')
            .map_or(self.netloc.as_str(), |(_, host)| host)
    }

    /// `SplitResult.hostname`: lowercased, zone suffix kept as written.
    pub fn hostname(&self) -> Option<String> {
        let info = self.hostinfo();
        let host = match info.split_once('[') {
            Some((_, bracketed)) => bracketed.split_once(']').map_or(bracketed, |(h, _)| h),
            None => info.split_once(':').map_or(info, |(h, _)| h),
        };
        if host.is_empty() {
            return None;
        }
        Some(match host.split_once('%') {
            Some((name, zone)) => format!("{}%{zone}", kassiber_pytext::lower(name)),
            None => kassiber_pytext::lower(host),
        })
    }

    /// `SplitResult.port`; `Err` is the reference's `ValueError`.
    pub fn port(&self) -> Result<Option<u32>, Invalid> {
        let info = self.hostinfo();
        let port = match info.split_once('[') {
            Some((_, bracketed)) => bracketed
                .split_once(']')
                .map_or("", |(_, rest)| rest.split_once(':').map_or("", |(_, p)| p)),
            None => info.split_once(':').map_or("", |(_, p)| p),
        };
        if port.is_empty() {
            return Ok(None);
        }
        if !port.chars().all(|c| c.is_ascii_digit()) {
            return Err(Invalid);
        }
        let value = port.trim_start_matches('0');
        match value.parse::<u32>() {
            Ok(n) if n <= 65535 => Ok(Some(n)),
            _ if value.is_empty() => Ok(Some(0)),
            _ => Err(Invalid),
        }
    }

    fn credentials(&self) -> bool {
        match self.netloc.rsplit_once('@') {
            Some((userinfo, _)) => match userinfo.split_once(':') {
                Some((user, password)) => !user.is_empty() || !password.is_empty(),
                None => !userinfo.is_empty(),
            },
            None => false,
        }
    }
}

fn validation(message: &str) -> AppError {
    AppError::new("validation", message)
}

const ROUTE_MARKERS: [&str; 13] = [
    "/api/v1",
    "/api-keys",
    "/stores/",
    "/apps/",
    "/invoices",
    "/i/",
    "/account",
    "/login",
    "/docs",
    "/wallets/",
    "/server/",
    "/payment-requests",
    "/pull-payments",
];

/// `normalize_server_url(str(value or ""))`: the canonical BTCPay base URL.
pub fn normalize_server_url(raw: &str) -> Result<String, AppError> {
    let raw = kassiber_pytext::strip(raw);
    if raw.is_empty() {
        return Err(validation("BTCPay server URL is required").with_hint(
            "Enter the address you open BTCPay Server with, for example https://btcpay.example.com.",
        ));
    }
    if !raw.contains("://") {
        return Err(
            validation("BTCPay server URL must start with https:// or http://")
                .with_hint("Use the full address, for example https://btcpay.example.com.")
                .with_details(dict([("reason", s("missing_scheme"))])),
        );
    }
    let parts = urlsplit(raw).map_err(|_| validation("BTCPay server URL is not a valid URL"))?;
    if !matches!(parts.scheme.as_str(), "http" | "https") {
        return Err(validation("BTCPay server URL must use https:// or http://")
            .with_details(dict([("reason", s("unsupported_scheme"))])));
    }
    if parts.hostname().is_none() {
        return Err(validation("BTCPay server URL is missing a host name"));
    }
    if parts.credentials() {
        return Err(
            validation("BTCPay server URL must not contain a user name or password").with_hint(
                "Remove the credentials from the URL and use a Greenfield API key instead.",
            ),
        );
    }
    // Python slices the original path at the index found in its lowercase
    // form, both in code points, even where lowering changes the length.
    let mut path: Vec<char> = parts.path.chars().collect();
    let mut lowered: Vec<char> = kassiber_pytext::lower(&parts.path).chars().collect();
    for marker in ROUTE_MARKERS {
        let marker: Vec<char> = marker.chars().collect();
        if let Some(index) = lowered.windows(marker.len()).position(|w| w == marker) {
            path.truncate(index);
            lowered.truncate(index);
        }
    }
    let path: String = path.into_iter().collect();
    Ok(format!(
        "{}://{}{}",
        parts.scheme,
        parts.netloc,
        path.trim_end_matches('/')
    ))
}

/// `server_url_identity`: `Err` is the reference's uncaught `ValueError` for
/// an unparsable port.
pub fn server_url_identity(raw: &str) -> Result<String, AppError> {
    let Ok(normalized) = normalize_server_url(raw) else {
        return Ok(kassiber_pytext::lower(
            kassiber_pytext::strip(raw).trim_end_matches('/'),
        ));
    };
    let invalid = || AppError::new("internal_error", "Port could not be cast to integer value");
    let parts = urlsplit(&normalized).map_err(|_| invalid())?;
    let host = kassiber_pytext::lower(&parts.hostname().unwrap_or_default());
    let port = parts.port().map_err(|_| invalid())?;
    let default = if parts.scheme == "https" { 443 } else { 80 };
    let netloc = match port {
        Some(port) if port != default => format!("{host}:{port}"),
        _ => host,
    };
    Ok(format!(
        "{}://{netloc}{}",
        parts.scheme,
        kassiber_pytext::lower(parts.path.trim_end_matches('/'))
    ))
}

fn host_of(raw: &str) -> String {
    urlsplit(raw)
        .ok()
        .and_then(|parts| parts.hostname())
        .unwrap_or_default()
}

/// `is_onion_url`.
pub fn is_onion_url(raw: &str) -> bool {
    kassiber_pytext::lower(&host_of(raw)).ends_with(".onion")
}

/// `is_loopback_url`.
pub fn is_loopback_url(raw: &str) -> bool {
    let host = kassiber_pytext::lower(&host_of(raw));
    ["localhost", "127.0.0.1", "::1"].contains(&host.as_str()) || host.starts_with("127.")
}

/// `_unparsed_backend_location`: only a bare `host[:port]` is echoed.
fn unparsed_location(value: &str) -> String {
    let (host, port) = if let Some(rest) = value.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inner, tail))
                if !inner.is_empty()
                    && inner
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() || ":.".contains(c)) =>
            {
                (true, tail)
            }
            _ => (false, ""),
        }
    } else {
        let end = value.find(':').unwrap_or(value.len());
        let name = &value[..end];
        let ok = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            && name.starts_with(|c: char| c.is_ascii_alphanumeric())
            && name.ends_with(|c: char| c.is_ascii_alphanumeric());
        (ok, &value[end..])
    };
    let port_ok = port.is_empty()
        || port
            .strip_prefix(':')
            .is_some_and(|p| (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()));
    if host && port_ok {
        value.to_owned()
    } else {
        "<redacted>".into()
    }
}

/// `backends.redact_backend_url`: scheme, host, port and path only.
pub fn redact_backend_url(raw: &str) -> String {
    let value = kassiber_pytext::strip(raw);
    if value.is_empty() {
        return String::new();
    }
    let Ok(parts) = urlsplit(value) else {
        return unparsed_location(value);
    };
    let Ok(port) = parts.port() else {
        return unparsed_location(value);
    };
    let hostname = parts.hostname().unwrap_or_default();
    if parts.scheme.is_empty() || parts.netloc.is_empty() || hostname.is_empty() {
        return unparsed_location(value);
    }
    let mut host = if hostname.contains(':') && !hostname.starts_with('[') {
        format!("[{hostname}]")
    } else {
        hostname
    };
    if let Some(port) = port {
        host = format!("{host}:{port}");
    }
    if parts.credentials() {
        host = format!("<redacted>@{host}");
    }
    let path = if !parts.path.is_empty() && !parts.path.starts_with('/') {
        format!("/{}", parts.path)
    } else {
        parts.path.clone()
    };
    format!("{}://{host}{path}", parts.scheme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::{get, py_str};
    use kassiber_pyjson::Value;

    /// `str(value or "")` for the frozen inputs.
    fn text(value: &Value) -> String {
        match value {
            Value::Str(text) => from_py(text),
            v if !kassiber_envelope::truthy(v) => String::new(),
            Value::Bool(true) => "True".into(),
            other => py_str(other),
        }
    }

    fn outcome(result: Result<String, AppError>) -> Value {
        match result {
            Ok(text) => dict([("ok", Value::Str(to_py(&text)))]),
            Err(e) if e.code == "internal_error" => dict([("exception", s("ValueError"))]),
            Err(e) => dict([(
                "error",
                dict([
                    ("code", s(e.code.as_str())),
                    ("message", s(e.message.to_string())),
                    ("hint", e.hint.as_deref().map_or(Value::Null, s)),
                    ("details", e.details.clone().unwrap_or(Value::Null)),
                ]),
            )]),
        }
    }

    #[test]
    fn btcpay_urls_match_python_vectors() {
        let Value::List(cases) =
            kassiber_pytext::loads(include_str!("../../fixtures/python_btcpay_urls.json")).unwrap()
        else {
            panic!("cases");
        };
        assert_eq!(cases.len(), 93);
        for case in cases {
            let input = text(get(&case, "input").unwrap());
            assert_eq!(
                outcome(normalize_server_url(&input)),
                get(&case, "normalized").unwrap().clone(),
                "{input:?}"
            );
            assert_eq!(
                outcome(server_url_identity(&input)),
                get(&case, "identity").unwrap().clone(),
                "{input:?}"
            );
            assert_eq!(
                Value::Bool(is_onion_url(&input)),
                get(&case, "onion").unwrap().clone(),
                "{input:?}"
            );
            assert_eq!(
                Value::Bool(is_loopback_url(&input)),
                get(&case, "loopback").unwrap().clone(),
                "{input:?}"
            );
            assert_eq!(
                outcome(Ok(redact_backend_url(&input))),
                get(&case, "redacted").unwrap().clone(),
                "{input:?}"
            );
        }
    }
}
