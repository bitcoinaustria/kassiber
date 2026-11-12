use super::*;
pub mod bark;
pub mod client;
pub mod discovery;
pub mod fetch;
pub mod normalize;
pub mod payment_methods;
pub mod permissions;
pub mod url;
use kassiber_net::{Gate, HttpRequest};
use probes::optional;
use serde_json::json;

/// `re.sub(r"[^a-z0-9_.-]+", "-", label.strip().lower()).strip("-")`.
pub fn slug_label(label: &str) -> String {
    let lowered = kassiber_pytext::lower(kassiber_pytext::strip(label));
    let mut name = String::new();
    let mut in_run = false;
    for c in lowered.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c) {
            name.push(c);
            in_run = false;
        } else if !in_run {
            name.push('-');
            in_run = true;
        }
    }
    name.trim_matches('-').to_owned()
}

pub(crate) fn probe(
    gate: &Gate,
    version: &str,
    args: &Value,
    saved: Option<Value>,
) -> Result<Value> {
    let timeout = args.get("timeout").cloned().unwrap_or(json!(10));
    let Some(timeout) = timeout.as_u64().filter(|n| (1..=10).contains(n)) else {
        return Err(AppError::new(
            "validation",
            "ui.backends.btcpay.test timeout must be an integer between 1 and 10 seconds",
        ));
    };
    let mut config = if let Some(config) = saved {
        if ["server_url", "url", "api_key", "token"]
            .iter()
            .any(|key| optional(args, key).is_some())
        {
            return Err(AppError::new(
                "validation",
                "BTCPay setup received both a saved instance and inline credentials",
            )
            .with_hint("Choose a saved BTCPay instance or enter a new instance, not both."));
        }
        if config["kind"] != "btcpay" {
            return Err(AppError::new(
                "validation",
                "Choose a backend whose kind is btcpay.",
            ));
        }
        config
    } else {
        let label = optional(args, "backend_label")
            .or_else(|| optional(args, "instance_label"))
            .unwrap_or("btcpay");
        let name = slug_label(label);
        if name.is_empty() {
            return Err(
                AppError::new("validation", "BTCPay instance label is required.")
                    .with_hint("Enter a short local name for this BTCPay instance."),
            );
        }
        let url = optional(args, "server_url")
            .or_else(|| optional(args, "url"))
            .ok_or_else(|| AppError::new("validation", "BTCPay server URL is required"))?;
        let token = optional(args, "api_key")
            .or_else(|| optional(args, "token"))
            .ok_or_else(|| AppError::new("validation", "BTCPay API key is required"))?;
        json!({"name":name,"kind":"btcpay","url":url,"token":token})
    };
    let raw = config::value(&config, &["url"])
        .ok_or_else(|| AppError::new("config_error", "BTCPay instance is missing 'url'"))?;
    let base = url::normalize_server_url(raw)?;
    let token = config::value(&config, &["token"])
        .ok_or_else(|| {
            AppError::new(
                "config_error",
                "BTCPay instance is missing 'token' (api key)",
            )
            .with_hint("Enter a Greenfield API key for this BTCPay instance.")
        })?
        .to_owned();
    config["url"] = json!(base);
    config["timeout"] = json!(timeout);
    let backend = Backend::new(&config, false)?;
    let consent = Consent::explicit_trigger(backend.request());
    let client = HttpClient::new(backend, gate.clone(), version);
    let url = format!("{base}/api/v1/stores");
    let response = client.retry(
        &url,
        Some(&consent),
        Instant::now() + Duration::from_secs(timeout),
        http::StatusPolicy::Return,
        || {
            HttpRequest::new("GET", &url)
                .and_then(|r| r.header("Accept", "application/json"))
                .and_then(|r| r.header("Authorization", &format!("token {token}")))
                .map_err(|e| e.app_error())
        },
        RESPONSE_LIMIT,
    )?;
    if !(200..300).contains(&response.status) {
        let withheld = config::credential_forms_of(&config, &[&token]);
        return Err(status_error(response.status, &response.body, &withheld));
    }
    let stores = serde_json::from_slice::<Value>(&response.body).map_err(|_| {
        AppError::new(
            "protocol_error",
            "BTCPay returned a response that is not JSON",
        )
        .with_hint("Check that the URL points at BTCPay Server and not at a login page or proxy.")
    })?;
    let stores = stores.as_array().ok_or_else(|| {
        AppError::new(
            "protocol_error",
            "BTCPay response for stores was not a JSON array",
        )
    })?;
    Ok(json!({"backend":config["name"],"ok":true,"stores_seen":stores.len()}))
}
fn status_error(status: u16, body: &[u8], withheld: &kassiber_credentials::Forms) -> AppError {
    // BTCPay's error body is the server's text: a message, code or missing
    // permission repeating the API key must not carry it into the error.
    let decoded = withhold_json(
        serde_json::from_slice::<Value>(&body[..body.len().min(4096)]).unwrap_or(Value::Null),
        withheld,
    );
    let code = decoded["code"].as_str();
    let mut fields = json!({"http_status":status});
    if let Some(code) = code {
        fields["btcpay_code"] = json!(code);
    }
    let mut error=match status {
        401=>AppError::new("auth_error","BTCPay rejected the API key (HTTP 401)").with_hint("The key is unknown, revoked, or belongs to another server. Create a new key in BTCPay (Account → API Keys) and update this connection."),
        403=>{
            if let Some(missing)=decoded["missingPermission"].as_str(){
                fields["missing_permission"]=json!(missing);
                let label=permissions::permission_label(missing);
                AppError::new("auth_error",format!("BTCPay API key is missing the '{label}' permission for the store list"))
            }else{AppError::new("auth_error","BTCPay API key is missing a required permission for the store list (HTTP 403)")}
        }.with_hint("Grant the API key access to view stores."),
        404=>AppError::new("not_found",if code.is_some_and(|c|c.contains("store")){"BTCPay store not found (HTTP 404)"}else if code.is_some_and(|c|c.contains("payment-method") || c.contains("paymentmethod")){"BTCPay payment method is not configured for this store (HTTP 404)"}else{"BTCPay resource not found for the store list (HTTP 404)"}).with_hint("Verify the store ID and payment method (default BTC-CHAIN)."),
        _=>AppError::new("protocol_error",format!("BTCPay returned HTTP {status} for the store list{}",decoded["message"].as_str().map(|s|format!(": {}",redacted(&s.chars().take(200).collect::<String>()))).unwrap_or_default())),
    };
    error.details = Some(details(fields));
    error
}

/// `withhold_credentials_value` over a decoded body's keys and strings.
fn withhold_json(value: Value, withheld: &kassiber_credentials::Forms) -> Value {
    match value {
        Value::String(text) => Value::String(kassiber_credentials::withhold(&text, withheld)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| withhold_json(item, withheld))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, item)| {
                    (
                        kassiber_credentials::withhold(&key, withheld),
                        withhold_json(item, withheld),
                    )
                })
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests;
