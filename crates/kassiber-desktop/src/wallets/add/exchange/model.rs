//! exchangeApiModel.ts: exchanges read through a read-only API key, their key guides, the
//! credentials a test or save sends, and what each daemon error code means.

use serde_json::{Map, Value, json};

/// `EXCHANGE_KINDS`.
pub const KINDS: [&str; 4] = ["kraken", "coinbase", "binance", "bitpanda"];

pub fn is_kind(kind: &str) -> bool {
    KINDS.contains(&kind.trim().to_lowercase().as_str())
}

/// Bitpanda authenticates with the key alone; the others sign with a secret.
pub fn needs_secret(kind: &str) -> bool {
    kind != "bitpanda"
}

pub fn provider_name(kind: &str) -> &'static str {
    match kind {
        "kraken" => "Kraken",
        "coinbase" => "Coinbase",
        "binance" => "Binance",
        "bitpanda" => "Bitpanda",
        _ => "Exchange",
    }
}

/// `ExchangeKeyGuide`.
pub struct KeyGuide {
    pub key_page_url: &'static str,
    pub grant: &'static [&'static str],
    pub deny: &'static [&'static str],
}

/// `EXCHANGE_KEY_GUIDES`.
pub fn key_guide(kind: &str) -> KeyGuide {
    match kind {
        "coinbase" => KeyGuide {
            key_page_url: "https://www.coinbase.com/settings/api",
            grant: &["wallet:accounts:read", "wallet:transactions:read"],
            deny: &[
                "wallet:transactions:send",
                "wallet:buys:create",
                "wallet:sells:create",
                "wallet:withdrawals:create",
            ],
        },
        "binance" => KeyGuide {
            key_page_url: "https://www.binance.com/en/my/settings/api-management",
            grant: &["Enable Reading"],
            deny: &[
                "Enable Spot & Margin Trading",
                "Enable Withdrawals",
                "Enable Futures",
                "Permits Universal Transfer",
            ],
        },
        "bitpanda" => KeyGuide {
            key_page_url: "https://app.bitpanda.com/my-account/apikey",
            grant: &["Transaction", "Trade (Read)"],
            deny: &["Trade (Write)", "Earn (Write)", "Balances"],
        },
        _ => KeyGuide {
            key_page_url: "https://pro.kraken.com/app/settings/api",
            grant: &["Query closed orders & trades", "Query ledger entries"],
            deny: &[
                "Deposit funds",
                "Withdraw funds",
                "Create & modify orders",
                "Cancel & close orders",
            ],
        },
    }
}

/// `SECRET_LABEL_KEYS` (Bitpanda reuses Coinbase's label; it never shows a secret field).
pub fn secret_label_key(kind: &str) -> &'static str {
    match kind {
        "kraken" => "connections:exchange.field.secret.kraken",
        "binance" => "connections:exchange.field.secret.binance",
        _ => "connections:exchange.field.secret.coinbase",
    }
}

/// The pasted credentials, exactly as a check or a save sends them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Credentials {
    pub exchange: String,
    pub api_key: String,
    pub api_secret: String,
}

pub fn credential_args(c: &Credentials) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("exchange".into(), json!(c.exchange));
    m.insert("api_key".into(), json!(c.api_key.trim()));
    if needs_secret(&c.exchange) {
        m.insert("api_secret".into(), json!(c.api_secret.trim()));
    }
    m
}

/// True when two drafts would send the same credentials.
pub fn same_credentials(a: &Credentials, b: Option<&Credentials>) -> bool {
    b.is_some_and(|b| credential_args(a) == credential_args(b))
}

/// "api_key" | "api_secret".
pub fn credential_issue(c: &Credentials) -> Option<&'static str> {
    if c.api_key.trim().is_empty() {
        return Some("api_key");
    }
    if needs_secret(&c.exchange) && c.api_secret.trim().is_empty() {
        return Some("api_secret");
    }
    None
}

/// `ExchangeErrorInfo`: the error object a failed daemon request carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ErrorInfo {
    pub code: Option<String>,
    pub message: Option<String>,
    pub hint: Option<String>,
    pub details: Option<Value>,
}

impl ErrorInfo {
    /// `exchangeErrorInfo`: from a reply's `error_detail` (and its message).
    pub fn from_reply(detail: &Value, message: Option<&str>) -> ErrorInfo {
        let s = |k: &str| detail.get(k).and_then(Value::as_str).map(str::to_owned);
        ErrorInfo {
            code: s("code"),
            message: s("message").or_else(|| message.map(str::to_owned)),
            hint: s("hint").filter(|h| !h.is_empty()),
            details: detail.get("details").filter(|d| d.is_object()).cloned(),
        }
    }
}

/// `exchangeErrorKind`: the `exchange.error.<kind>` explanation for a daemon error code.
pub fn error_kind(code: Option<&str>) -> &'static str {
    match code.unwrap_or("") {
        "auth_error" => "auth",
        "rate_limited" => "rateLimited",
        "network_egress_disabled" => "offline",
        "interaction_required" => "consent",
        "protocol_error" => "protocol",
        "validation" => "refused",
        "config_error" => "config",
        "backend_timeout" => "timeout",
        "network_error" | "network_proxy_required" | "network_proxy_unsupported" => "network",
        "app_error" => "http",
        "conflict" => "conflict",
        // A daemon without these kinds: an older build.
        "daemon_unavailable" | "unsupported_kind" | "kind_not_allowed" | "not_implemented" => {
            "needsRust"
        }
        _ => "unknown",
    }
}

/// Permissions a refused Binance key still holds, when the daemon named them.
pub fn refused_permissions(info: &ErrorInfo) -> Vec<String> {
    info.details
        .as_ref()
        .and_then(|d| d.get("permissions"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(ex: &str, k: &str, s: &str) -> Credentials {
        Credentials {
            exchange: ex.into(),
            api_key: k.into(),
            api_secret: s.into(),
        }
    }

    #[test]
    fn kinds_and_secrets() {
        assert!(
            is_kind("kraken") && is_kind(" Coinbase ") && is_kind("binance") && is_kind("bitpanda")
        );
        assert!(!is_kind("bitstamp") && !is_kind(""));
        assert!(needs_secret("kraken") && !needs_secret("bitpanda"));
        assert_eq!(provider_name("binance"), "Binance");
    }

    #[test]
    fn guides_name_exact_scopes() {
        let k = key_guide("kraken");
        assert_eq!(
            k.grant,
            &["Query closed orders & trades", "Query ledger entries"]
        );
        assert!(k.deny.contains(&"Withdraw funds"));
        assert_eq!(
            key_guide("bitpanda").key_page_url,
            "https://app.bitpanda.com/my-account/apikey"
        );
    }

    #[test]
    fn sends_exactly_the_trimmed_credentials() {
        let args = credential_args(&creds("kraken", " key ", " secret "));
        assert_eq!(
            Value::Object(args),
            json!({"exchange": "kraken", "api_key": "key", "api_secret": "secret"})
        );
        let bp = credential_args(&creds("bitpanda", "key", "ignored"));
        assert_eq!(
            Value::Object(bp),
            json!({"exchange": "bitpanda", "api_key": "key"})
        );
    }

    #[test]
    fn issues_and_sameness() {
        assert_eq!(credential_issue(&creds("kraken", "", "")), Some("api_key"));
        assert_eq!(
            credential_issue(&creds("kraken", "k", " ")),
            Some("api_secret")
        );
        assert_eq!(credential_issue(&creds("bitpanda", "k", "")), None);
        assert!(same_credentials(
            &creds("kraken", "k ", "s"),
            Some(&creds("kraken", "k", " s"))
        ));
        assert!(!same_credentials(
            &creds("kraken", "k", "s"),
            Some(&creds("kraken", "k", "t"))
        ));
        assert!(same_credentials(
            &creds("bitpanda", "k", "a"),
            Some(&creds("bitpanda", "k", "b"))
        ));
        assert!(!same_credentials(&creds("kraken", "k", "s"), None));
    }

    #[test]
    fn error_codes_and_permissions() {
        assert_eq!(error_kind(Some("auth_error")), "auth");
        assert_eq!(error_kind(Some("network_egress_disabled")), "offline");
        assert_eq!(error_kind(Some("unsupported_kind")), "needsRust");
        assert_eq!(error_kind(Some("network_proxy_required")), "network");
        assert_eq!(error_kind(None), "unknown");
        let info = ErrorInfo::from_reply(
            &json!({"code": "validation", "message": "refused", "hint": "", "details": {"permissions": ["enableWithdrawals", 3]}}),
            None,
        );
        assert_eq!(info.hint, None);
        assert_eq!(
            refused_permissions(&info),
            vec!["enableWithdrawals".to_owned()]
        );
        assert!(refused_permissions(&ErrorInfo::default()).is_empty());
    }
}
