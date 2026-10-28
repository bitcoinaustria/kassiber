use super::*;
use base64::Engine;
use kassiber_net::{Gate, HttpRequest};
use serde_json::json;
use std::io::Read;

pub struct CoreClient {
    http: HttpClient,
}
impl CoreClient {
    pub fn new(backend: Backend, gate: Gate, version: &str) -> Self {
        Self {
            http: HttpClient::new(backend, gate, version),
        }
    }
    pub fn backend(&self) -> &Backend {
        &self.http.backend
    }
    /// Observation-only RPC. Spending, signing, broadcast and unrestricted node
    /// administration are deliberately absent from this port's public API.
    pub fn call(
        &self,
        method: &str,
        params: Option<&Value>,
        wallet: Option<&str>,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<Value> {
        if ![
            "getblockchaininfo",
            "getnetworkinfo",
            "listwallets",
            "getbestblockhash",
            "getblockfilter",
            "getblockhash",
            "getblockheader",
            "getblock",
            "getrawtransaction",
            "decoderawtransaction",
            "gettransaction",
            "getaddressinfo",
            "getdescriptorinfo",
            "listtransactions",
            "listsinceblock",
            "listunspent",
            "getwalletinfo",
            "gettxout",
            "scantxoutset",
        ]
        .contains(&method)
        {
            return Err(AppError::new(
                "validation",
                "Unsupported watch-only Bitcoin Core RPC method",
            ));
        }
        let backend = &self.http.backend;
        // Gate first: missing/offline consent cannot even cause credential reads.
        self.http
            .gate
            .authorize(backend.request.clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        let url = wallet
            .filter(|w| !w.is_empty())
            .map(|w| config::append_path(&backend.url, &format!("wallet/{}", segment(w))))
            .transpose()?
            .unwrap_or_else(|| backend.url.clone());
        let id = format!("kassiber-{method}");
        let payload = json!({"jsonrpc":"1.0", "id":id, "method":method, "params":params.unwrap_or(&json!([]))});
        let auth = auth(backend)?;
        let response = self.http.retry(
            &url,
            consent,
            deadline,
            http::StatusPolicy::Core,
            || {
                HttpRequest::new("POST", &url)
                    .and_then(|r| r.header("Accept", "application/json"))
                    .and_then(|r| r.header("Content-Type", "application/json"))
                    .and_then(|r| {
                        r.header("User-Agent", &format!("kassiber/{}", self.http.version))
                    })
                    .and_then(|r| r.header("Authorization", &auth))
                    .map(|r| r.body(payload.to_string().into_bytes()))
                    .map_err(|e| e.app_error())
            },
            if method == "listsinceblock" {
                CORE_HISTORY_LIMIT
            } else {
                RESPONSE_LIMIT
            },
        )?;
        let envelope = if matches!(response.status, 400 | 404 | 500) {
            let value = serde_json::from_slice::<Value>(&response.body).ok();
            let valid = response.body.len() <= 65_536
                && value.as_ref().is_some_and(|v| {
                    v.is_object()
                        && v.get("id") == Some(&json!(id))
                        && v.get("result").is_none_or(Value::is_null)
                        && v.get("error").is_some_and(|e| {
                            e.is_object() && e.get("code").is_some_and(|c| c.as_i64().is_some())
                        })
                });
            if !valid {
                return Err(app("Bitcoin Core returned an invalid RPC error response")
                    .with_details(details(json!({"http_status":response.status}))));
            }
            value.unwrap()
        } else {
            http::decode_json(&response.body)?
        };
        if !envelope.is_object() {
            return Err(app(format!(
                "Bitcoin Core RPC {method} returned an unexpected payload"
            )));
        }
        if let Some(error) = envelope.get("error").filter(|e| truthy(e)) {
            let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
                app(format!(
                    "Bitcoin Core RPC {method} returned an invalid error"
                ))
            })?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            let rescan = (!matches!(response.status, 400 | 404 | 500)
                && error.get("category").and_then(Value::as_str) == Some("rescan failed"))
                || (message.contains("rescan")
                    && (message.contains("fail") || message.contains("prun")));
            return Err(app(format!(
                "Bitcoin Core RPC {method} failed ({code}){}",
                if rescan { ": rescan failed" } else { "" }
            ))
            .with_details(details(json!({"rpc_method":method,"rpc_error_code":code}))));
        }
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    }
}
fn auth(backend: &Backend) -> Result<String> {
    let token = if let Some(path) = config::value(&backend.config, &["cookiefile", "cookie_file"]) {
        let path = if let Some(tail) = path.strip_prefix("~/") {
            std::path::PathBuf::from(
                std::env::var_os("HOME").ok_or_else(|| app("Home directory unavailable"))?,
            )
            .join(tail)
        } else {
            path.into()
        };
        let mut token = String::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(65537).read_to_string(&mut token))
            .map_err(|_| app("Could not read Bitcoin Core cookie file"))?;
        if token.len() > 65536 {
            return Err(app("Bitcoin Core cookie file is too large"));
        }
        token.trim().to_owned()
    } else {
        let user = config::value(&backend.config, &["username", "rpcuser", "rpc_user"])
            .filter(|s| !s.is_empty());
        let password = config::value(
            &backend.config,
            &["password", "rpcpassword", "rpc_password"],
        );
        let (Some(user), Some(password)) = (user, password) else {
            return Err(app(format!(
                "Bitcoin Core backend '{}' requires USERNAME/PASSWORD or COOKIEFILE configuration",
                backend.name
            )));
        };
        format!("{user}:{password}")
    };
    Ok(format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(token)
    ))
}
fn segment(raw: &str) -> String {
    let mut result = String::new();
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}
