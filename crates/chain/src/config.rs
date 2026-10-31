use super::*;
use kassiber_net::{Endpoint, HttpUrl, Request, Subsystem, TlsTrust};
use std::path::Path;

/// Immutable backend snapshot; credentials deliberately have no Debug output.
/// Keep the exact Request when minting consent. Reconfiguration creates a new
/// snapshot and cannot inherit a grant for the old route/trust/proxy.
#[derive(Clone)]
pub struct Backend {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) timeout: Duration,
    pub(crate) batch_size: usize,
    pub(crate) request: Request,
    pub(crate) config: Value,
}

impl Backend {
    /// Python backend mapping, including nested `config` / serialized config_json.
    /// This performs only local configuration/certificate reads, never a probe.
    pub fn new(config: &Value, electrum: bool) -> Result<Self> {
        Self::with_trust(config, electrum, None)
    }
    pub(crate) fn with_trust(
        config: &Value,
        electrum: bool,
        trust: Option<TlsTrust>,
    ) -> Result<Self> {
        let mut config = config.clone();
        if !config.is_object() {
            return Err(app("Backend configuration must be an object"));
        }
        if let Some(nested) = config.get("config_json").and_then(Value::as_str) {
            let nested = serde_json::from_str::<Value>(nested)
                .map_err(|_| app("Invalid backend configuration JSON"))?;
            config["config"] = nested;
        }
        let name = value(&config, &["name"]).unwrap_or("candidate").to_owned();
        let url = value(&config, &["url"]).unwrap_or("").to_owned();
        let (host, port, secure) = if electrum {
            socket_endpoint(&url)?
        } else {
            let parsed = HttpUrl::parse(&url).map_err(|_| app("Invalid backend URL"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(app("HTTP backend URL must start with http:// or https://"));
            }
            (
                parsed
                    .host_str()
                    .unwrap_or("")
                    .trim_matches(['[', ']'])
                    .to_owned(),
                parsed.port_or_known_default().unwrap_or(80),
                parsed.scheme() == "https",
            )
        };
        let insecure = field(&config, &["insecure"])
            .map(boolean)
            .transpose()?
            .unwrap_or(false);
        let trust = if let Some(trust) = trust {
            trust
        } else if electrum && !secure {
            TlsTrust::Plaintext
        } else if insecure {
            TlsTrust::ExplicitInsecure
        } else if let Some(cert) = value(&config, &["certificate"]).filter(|s| !s.is_empty()) {
            TlsTrust::backend_certificate(Path::new(cert)).map_err(|e| e.app_error())?
        } else if secure {
            TlsTrust::python_default().map_err(|e| e.app_error())?
        } else {
            TlsTrust::Plaintext
        };
        let trust = if secure { trust } else { TlsTrust::Plaintext };
        let mut request = Request::new(
            Subsystem::Sync,
            Endpoint::new(&host, port).map_err(|e| e.app_error())?,
            trust,
        );
        if let Some(raw) = field(&config, &["tor_proxy", "proxy"]) {
            let proxy = raw.as_str().ok_or_else(|| app("Invalid proxy URL"))?;
            request = request.with_proxy(parse_proxy(proxy)?);
        }
        let timeout = positive(&config, "timeout", 30)?;
        let batch_size = positive(&config, "batch_size", 100)? as usize;
        Ok(Self {
            name,
            url,
            timeout: Duration::from_secs(timeout),
            batch_size,
            request,
            config,
        })
    }
    pub fn request(&self) -> &Request {
        &self.request
    }
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }
}
fn positive(config: &Value, key: &str, default: u64) -> Result<u64> {
    let number = match field(config, &[key]) {
        None | Some(Value::Null) => default,
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    };
    if number == 0 {
        return Err(AppError::new(
            "validation",
            if key == "timeout" {
                "Backend timeout must be positive"
            } else {
                "Backend batch size must be positive"
            },
        ));
    }
    Ok(number)
}
pub(crate) fn field<'a>(config: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for key in keys {
        if let Some(v) = config
            .get(key)
            .filter(|v| !v.is_null() && v.as_str() != Some(""))
        {
            return Some(v);
        }
        if let Some(v) = config
            .get("config")
            .and_then(|v| v.get(key))
            .filter(|v| !v.is_null() && v.as_str() != Some(""))
        {
            return Some(v);
        }
    }
    None
}
pub(crate) fn value<'a>(config: &'a Value, keys: &[&str]) -> Option<&'a str> {
    field(config, keys).and_then(Value::as_str)
}
pub(crate) fn boolean(value: &Value) -> Result<bool> {
    if value.is_null() {
        return Ok(false);
    }
    let text = kassiber_envelope::py_str(&details(value.clone()));
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(app(format!("Invalid boolean value: {text}"))),
    }
}
pub fn socket_endpoint(raw: &str) -> Result<(String, u16, bool)> {
    let normalized = if raw.contains("://") {
        raw.into()
    } else {
        format!("ssl://{raw}")
    };
    let parsed = HttpUrl::parse(&normalized).map_err(|_| app("Invalid backend socket URL"))?;
    let port = parsed
        .port()
        .or(match parsed.scheme() {
            "ssl" => Some(50002),
            "tcp" => Some(50001),
            _ => None,
        })
        .ok_or_else(|| app("Invalid backend socket URL"))?;
    let secure = match parsed.scheme() {
        "ssl" | "tls" => true,
        "tcp" => false,
        scheme => return Err(app(format!("Unsupported Electrum transport '{scheme}'"))),
    };
    let host = parsed
        .host_str()
        .ok_or_else(|| app("Invalid backend socket URL"))?;
    Ok((host.trim_matches(['[', ']']).into(), port, secure))
}
fn unquote(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (
                (bytes[index + 1] as char).to_digit(16),
                (bytes[index + 2] as char).to_digit(16),
            ) {
                output.push((a * 16 + b) as u8);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}
fn parse_proxy(raw: &str) -> Result<kassiber_net::Proxy> {
    let normalized = if raw.contains("://") {
        raw.into()
    } else {
        format!("socks5h://{raw}")
    };
    let url = HttpUrl::parse(&normalized).map_err(|_| app("Invalid proxy URL"))?;
    let credentials = if url.username().is_empty() && url.password().is_none() {
        None
    } else {
        Some((
            unquote(url.username()),
            unquote(url.password().unwrap_or("")),
        ))
    };
    kassiber_net::Proxy::new(
        url.scheme(),
        Endpoint::new(
            url.host_str().unwrap_or("").trim_matches(['[', ']']),
            url.port()
                .unwrap_or(if url.scheme() == "http" { 8080 } else { 9050 }),
        )
        .map_err(|e| e.app_error())?,
        credentials,
    )
    .map_err(|e| e.app_error())
}

/// Append without discarding a reverse-proxy prefix or query, as Python does.
pub(crate) fn append_path(base: &str, extra: &str) -> Result<String> {
    let mut url = HttpUrl::parse(base).map_err(|_| app("Invalid backend URL"))?;
    let path = format!(
        "{}/{}",
        url.path().trim_end_matches('/'),
        extra.trim_start_matches('/')
    );
    url.set_path(&path);
    Ok(url.to_string())
}
