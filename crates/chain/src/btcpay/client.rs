//! `kassiber/btcpay/client.py`: a read-only Greenfield client. Every request
//! is a `GET` through the `kassiber-net` gate under the caller's exact action
//! grant; construction is inert and the API key never leaves this type.
use super::normalize::{field, text};
use super::permissions::permission_label;
use super::url::normalize_server_url;
use crate::{
    config, details, http, AppError, Backend, Consent, HttpClient, Result, RESPONSE_LIMIT,
};
use kassiber_envelope::{dict, get, py_str, s};
use kassiber_net::{Gate, HttpRequest, Request};
use kassiber_pyjson::Value;
use std::time::Instant;

const ERROR_BODY_LIMIT: usize = 4096;

/// `GreenfieldHttpError`: a non-retryable HTTP status with BTCPay's decoded body.
#[derive(Debug)]
pub struct HttpFailure {
    pub status: u16,
    pub body: Value,
}

impl HttpFailure {
    fn body_text(&self, key: &str) -> Option<String> {
        if !matches!(self.body, Value::Dict(_)) {
            return None;
        }
        let value = field(&self.body, key);
        kassiber_envelope::truthy(value).then(|| py_str(&Value::Str(text(value))))
    }
    pub fn error_code(&self) -> Option<String> {
        self.body_text("code")
    }
    pub fn missing_permission(&self) -> Option<String> {
        self.body_text("missingPermission")
    }
}

pub enum Failure {
    Http(HttpFailure),
    App(AppError),
}

impl From<AppError> for Failure {
    fn from(error: AppError) -> Self {
        Failure::App(error)
    }
}

#[derive(Default, Clone, Copy)]
pub struct Context<'a> {
    pub context: Option<&'a str>,
    pub permission_hint: Option<&'a str>,
    pub not_found_hint: Option<&'a str>,
}

/// `greenfield_app_error`.
pub fn app_error(failure: &HttpFailure, ctx: Context<'_>) -> AppError {
    let mut fields = vec![("http_status", Value::Int(failure.status.into()))];
    let code = failure.error_code();
    if let Some(code) = &code {
        fields.push(("btcpay_code", s(code.as_str())));
    }
    let place = ctx.context.map(|c| format!(" for {c}")).unwrap_or_default();
    let error = match failure.status {
        401 => AppError::new("auth_error", "BTCPay rejected the API key (HTTP 401)").with_hint(
            "The key is unknown, revoked, or belongs to another server. Create a new key in BTCPay (Account → API Keys) and update this connection.",
        ),
        403 => match failure.missing_permission() {
            Some(missing) => {
                fields.push(("missing_permission", s(missing.as_str())));
                let hint = ctx.permission_hint.map(str::to_owned).unwrap_or_else(|| {
                    format!("Create a key that includes {missing} for this store, or use `kassiber btcpay key-url` to open a pre-filled authorization page.")
                });
                AppError::new(
                    "auth_error",
                    format!(
                        "BTCPay API key is missing the '{}' permission{place}",
                        permission_label(&missing)
                    ),
                )
                .with_hint(hint)
            }
            None => AppError::new(
                "auth_error",
                format!("BTCPay API key is missing a required permission{place} (HTTP 403)"),
            )
            .with_hint(
                ctx.permission_hint
                    .unwrap_or("Check the permissions granted to the Greenfield API key."),
            ),
        },
        404 => {
            let code = code.clone().unwrap_or_default();
            let message = if code.contains("store") {
                "BTCPay store not found (HTTP 404)".to_owned()
            } else if code.contains("payment-method") || code.contains("paymentmethod") {
                "BTCPay payment method is not configured for this store (HTTP 404)".to_owned()
            } else {
                format!("BTCPay resource not found{place} (HTTP 404)")
            };
            AppError::new("not_found", message).with_hint(
                ctx.not_found_hint
                    .unwrap_or("Verify the store ID and payment method (default BTC-CHAIN)."),
            )
        }
        status => {
            let message = match &failure.body {
                Value::Dict(_) => get(&failure.body, "message")
                    .filter(|v| kassiber_envelope::truthy(v))
                    .map(|v| {
                        let text = py_str(&Value::Str(text(v)));
                        format!(": {}", text.chars().take(200).collect::<String>())
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            };
            AppError::new(
                "protocol_error",
                format!("BTCPay returned HTTP {status}{place}{message}"),
            )
        }
    };
    error.with_details(dict(fields))
}

/// `urllib.parse.quote(value, safe="")`.
pub fn quote(raw: &str) -> String {
    let mut out = String::new();
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `urllib.parse.urlencode` (`quote_plus`).
pub fn urlencode(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                quote(k).replace("%20", "+"),
                quote(v).replace("%20", "+")
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn decode_error_body(body: &[u8]) -> Value {
    let raw = String::from_utf8_lossy(&body[..body.len().min(ERROR_BODY_LIMIT)]);
    match kassiber_pytext::loads(&raw) {
        Ok(Value::List(items)) if matches!(items.first(), Some(Value::Dict(_))) => {
            let message = field(&items[0], "message");
            let message = if kassiber_envelope::truthy(message) {
                text(message)
            } else {
                "".into()
            };
            dict([("code", s("validation")), ("message", Value::Str(message))])
        }
        Ok(value @ Value::Dict(_)) => value,
        _ => Value::Null,
    }
}

pub struct Greenfield {
    http: HttpClient,
    base: String,
    token: String,
}

impl Greenfield {
    /// `GreenfieldClient.from_backend`; `binding` names the exact action.
    pub fn from_backend(
        backend: &serde_json::Value,
        gate: Gate,
        version: &str,
        binding: [u8; 32],
        missing_token_hint: Option<&str>,
    ) -> Result<Self> {
        let value = |key: &str| {
            config::value(backend, &[key])
                .map(kassiber_pytext::strip)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let raw = value("url")
            .ok_or_else(|| AppError::new("config_error", "BTCPay instance is missing 'url'"))?;
        let token = value("token").ok_or_else(|| {
            AppError::new("config_error", "BTCPay instance is missing 'token' (api key)")
                .with_hint(missing_token_hint.unwrap_or(
                    "Store the api key with `kassiber backends update --token-stdin` or `--token-fd FD`.",
                ))
        })?;
        let base = normalize_server_url(&raw)?;
        let mut config = backend.clone();
        config["url"] = serde_json::Value::String(base.clone());
        let mut backend = Backend::new(&config, false)?;
        backend.request = backend.request.clone().with_action_binding(binding);
        Ok(Self {
            http: HttpClient::new(backend, gate, version),
            base,
            token,
        })
    }

    /// The request a caller binds its grant to.
    pub fn request(&self) -> &Request {
        self.http.backend().request()
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn url(&self, path: &str, query: &[(&str, String)]) -> String {
        let mut url = format!("{}{path}", self.base);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&urlencode(query));
        }
        url
    }

    /// `get_url`: JSON on success, the decoded body on a non-retryable status.
    pub fn get_url(
        &self,
        url: &str,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> std::result::Result<Value, Failure> {
        let response = self
            .http
            .retry(
                url,
                consent,
                deadline,
                http::StatusPolicy::Return,
                || {
                    HttpRequest::new("GET", url)
                        .and_then(|r| r.header("Accept", "application/json"))
                        .and_then(|r| r.header("Authorization", &format!("token {}", self.token)))
                        .map_err(|e| e.app_error())
                },
                RESPONSE_LIMIT,
            )
            .map_err(|error| {
                if error.code == "network_error" {
                    AppError::new("network_error", "Could not reach the BTCPay server")
                        .retryable(true)
                        .with_hint("Check the server URL, your network or Tor proxy, and that BTCPay is running.")
                        .with_details(details(serde_json::json!({"reason": error.message.to_string()})))
                } else {
                    error
                }
            })?;
        if !(200..300).contains(&response.status) {
            return Err(Failure::Http(HttpFailure {
                status: response.status,
                body: decode_error_body(&response.body),
            }));
        }
        if response.body.is_empty() {
            return Ok(Value::Null);
        }
        let not_json = || {
            AppError::new(
                "protocol_error",
                "BTCPay returned a response that is not JSON",
            )
            .with_hint(
                "Check that the URL points at BTCPay Server and not at a login page or proxy.",
            )
        };
        let raw = std::str::from_utf8(&response.body).map_err(|_| not_json())?;
        Ok(kassiber_pytext::loads(raw).map_err(|_| not_json())?)
    }

    /// `get_json`: HTTP failures become `greenfield_app_error`.
    pub fn get_json(
        &self,
        path: &str,
        query: &[(&str, String)],
        ctx: Context<'_>,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<Value> {
        match self.get_url(&self.url(path, query), consent, deadline) {
            Ok(value) => Ok(value),
            Err(Failure::Http(failure)) => Err(app_error(&failure, ctx)),
            Err(Failure::App(error)) => Err(error),
        }
    }

    /// `get_optional_json`: tolerated statuses are reported, not raised.
    pub fn get_optional(
        &self,
        path: &str,
        query: &[(&str, String)],
        tolerate: &[u16],
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<(Value, Option<HttpFailure>)> {
        match self.get_url(&self.url(path, query), consent, deadline) {
            Ok(value) => Ok((value, None)),
            Err(Failure::Http(failure)) if tolerate.contains(&failure.status) => {
                Ok((Value::Null, Some(failure)))
            }
            Err(Failure::Http(failure)) => Err(app_error(&failure, Context::default())),
            Err(Failure::App(error)) => Err(error),
        }
    }
}

/// `scrub_api_key_payload`: drops the echoed secret from `/api-keys/current`;
/// `None` when the payload is not an object (an unknown grant).
pub fn scrub_api_key(payload: &Value) -> Option<(Value, Vec<String>)> {
    if !matches!(payload, Value::Dict(_)) {
        return None;
    }
    let label = field(payload, "label");
    let label = if kassiber_envelope::truthy(label) {
        let text = text(label);
        if text.is("") {
            Value::Null
        } else {
            Value::Str(text)
        }
    } else {
        Value::Null
    };
    let permissions = match field(payload, "permissions") {
        Value::List(items) => items
            .iter()
            .map(|v| super::url::from_py(&text(v)))
            .collect(),
        _ => Vec::new(),
    };
    Some((label, permissions))
}
