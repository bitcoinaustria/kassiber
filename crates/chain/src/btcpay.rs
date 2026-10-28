use super::*;
use kassiber_net::{Gate, HttpRequest, HttpUrl};
use probes::optional;
use serde_json::json;

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
            .unwrap_or("btcpay")
            .to_lowercase();
        let mut name = String::new();
        for c in label.chars() {
            if c.is_ascii_alphanumeric() || "_.-".contains(c) {
                name.push(c);
            } else if !name.ends_with('-') {
                name.push('-');
            }
        }
        let name = name.trim_matches('-');
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
    let base = normalize_url(raw)?;
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
        return Err(status_error(response.status, &response.body));
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
fn normalize_url(raw: &str) -> Result<String> {
    if !raw.contains("://") {
        return Err(AppError::new(
            "validation",
            "BTCPay server URL must start with https:// or http://",
        )
        .with_hint("Use the full address, for example https://btcpay.example.com.")
        .with_details(details(json!({"reason":"missing_scheme"}))));
    }
    let mut url = HttpUrl::parse(raw)
        .map_err(|_| AppError::new("validation", "BTCPay server URL is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::new(
            "validation",
            "BTCPay server URL must use https:// or http://",
        )
        .with_details(details(json!({"reason":"unsupported_scheme"}))));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::new(
            "validation",
            "BTCPay server URL must not contain a user name or password",
        )
        .with_hint("Remove the credentials from the URL and use a Greenfield API key instead."));
    }
    let mut path = url.path().to_owned();
    for marker in [
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
    ] {
        if let Some(index) = path.to_lowercase().find(marker) {
            path.truncate(index);
        }
    }
    url.set_path(path.trim_end_matches('/'));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}
fn status_error(status: u16, body: &[u8]) -> AppError {
    let decoded =
        serde_json::from_slice::<Value>(&body[..body.len().min(4096)]).unwrap_or(Value::Null);
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
                let base=missing.split(':').next().unwrap_or(missing);
                let label=match base {"btcpay.store.canviewstoresettings"=>"View your stores","btcpay.store.canmodifystoresettings"=>"Modify your stores",_=>base};
                AppError::new("auth_error",format!("BTCPay API key is missing the '{label}' permission for the store list"))
            }else{AppError::new("auth_error","BTCPay API key is missing a required permission for the store list (HTTP 403)")}
        }.with_hint("Grant the API key access to view stores."),
        404=>AppError::new("not_found",if code.is_some_and(|c|c.contains("store")){"BTCPay store not found (HTTP 404)"}else if code.is_some_and(|c|c.contains("payment-method") || c.contains("paymentmethod")){"BTCPay payment method is not configured for this store (HTTP 404)"}else{"BTCPay resource not found for the store list (HTTP 404)"}).with_hint("Verify the store ID and payment method (default BTC-CHAIN)."),
        _=>AppError::new("protocol_error",format!("BTCPay returned HTTP {status} for the store list{}",decoded["message"].as_str().map(|s|format!(": {}",redacted(&s.chars().take(200).collect::<String>()))).unwrap_or_default())),
    };
    error.details = Some(details(fields));
    error
}
