use crate::{config, details, AppError, Backend, Consent, HttpClient, Result, RESPONSE_LIMIT};
use kassiber_envelope::{dict, get, py_str, truthy};
use kassiber_net::{Gate, HttpRequest, Request, TlsTrust};
use kassiber_pyjson::{BigInt, PyStr, Value};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone)]
pub struct ReportAction {
    pub book_id: String,
    pub profile_id: String,
    pub connection_id: String,
    pub window_days: u16,
    pub export_file: Option<String>,
}

impl ReportAction {
    fn binding(&self, backend: &Json) -> Result<[u8; 32]> {
        if self.book_id.is_empty()
            || self.profile_id.is_empty()
            || self.connection_id.is_empty()
            || !(1..=365).contains(&self.window_days)
        {
            return Err(AppError::new(
                "validation",
                "Invalid Lightning report action scope",
            ));
        }
        let input = serde_json::json!([
            "lightning-profitability-v1",
            self.book_id,
            self.profile_id,
            self.connection_id,
            self.window_days,
            self.export_file,
            backend
        ]);
        Ok(Sha256::digest(input.to_string().as_bytes()).into())
    }
}

#[derive(Debug, PartialEq)]
pub struct Channel {
    pub id: PyStr,
    pub peer_alias: PyStr,
    pub capacity_sat: BigInt,
    pub earned_routing_sat: Option<BigInt>,
}

#[derive(Debug, PartialEq)]
pub struct Routing {
    pub window_label: String,
    pub routing_revenue_sat: BigInt,
    pub payment_cost_sat: BigInt,
    pub rebalance_cost_sat: BigInt,
    pub onchain_cost_sat: BigInt,
    pub net_profit_sat: BigInt,
    pub forward_count: usize,
    pub payment_count: usize,
    pub rebalance_count: usize,
}

#[derive(Debug, PartialEq)]
pub struct ProfitabilitySnapshot {
    pub channels: Vec<Channel>,
    pub routing: Option<Routing>,
}

pub struct LndReportClient {
    http: HttpClient,
    macaroon: String,
    window_days: u16,
}

impl LndReportClient {
    pub fn new(config: &Json, gate: Gate, version: &str, action: ReportAction) -> Result<Self> {
        if !config.is_object() || config.as_object().is_some_and(|v| v.is_empty()) {
            return Err(AppError::new(
                "config_error",
                "LND adapter requires a backend row for the connection.",
            )
            .with_hint(
                "Attach a backend (kind=lnd) to the wallet before requesting a node snapshot.",
            ));
        }
        if config
            .get("kind")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_lowercase()
            != "lnd"
        {
            return Err(AppError::new(
                "validation",
                format!(
                    "Backend '{}' has kind '{}', expected 'lnd'",
                    config.get("name").and_then(Json::as_str).unwrap_or("None"),
                    config.get("kind").and_then(Json::as_str).unwrap_or("None")
                ),
            ));
        }
        let mut config = config.clone();
        if let Some(raw) = config.get("config_json").and_then(Json::as_str) {
            config["config"] = serde_json::from_str(raw)
                .map_err(|_| AppError::new("config_error", "Invalid backend configuration JSON"))?;
        }
        let url = config::value(&config, &["url"])
            .unwrap_or("")
            .trim_end_matches('/');
        if url.is_empty() {
            return Err(AppError::new("config_error", "LND backend URL is required"));
        }
        config["url"] = Json::String(url.into());
        let token = config::value(&config, &["token"]).filter(|s| !s.is_empty()).ok_or_else(|| {
            AppError::new("config_error", "LND backend is missing a read-only macaroon")
                .with_hint("Store a scoped read-only macaroon with `backends create --kind lnd --token-stdin` or the desktop backend form.")
        })?;
        let macaroon = kassiber_pytext::strip(token);
        if !macaroon.len().is_multiple_of(2) || !macaroon.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::new("validation", "LND macaroon must be stored as hex")
                .with_hint("Use `xxd -p -c 256 readonly.macaroon | kassiber backends create --kind lnd --token-stdin ...`."));
        }
        let macaroon = macaroon.to_ascii_lowercase();
        let insecure = config::field(&config, &["insecure", "trust_ssl"])
            .is_some_and(|v| lnd_truthy(&details(v.clone())));
        config["insecure"] = Json::Bool(insecure);
        let trust = if insecure {
            Some(TlsTrust::ExplicitInsecure)
        } else if let Some(cert) =
            config::value(&config, &["certificate"]).filter(|s| !s.is_empty())
        {
            let path = expand_path(cert);
            Some(
                if path.exists() {
                    TlsTrust::backend_certificate(&path)
                } else {
                    TlsTrust::backend_pem(cert.as_bytes())
                }
                .map_err(|e| e.app_error())?,
            )
        } else {
            None
        };
        let mut backend = Backend::with_trust(&config, false, trust)?;
        backend.request = backend
            .request
            .clone()
            .with_action_binding(action.binding(&config)?);
        Ok(Self {
            http: HttpClient::new(backend, gate, version),
            macaroon,
            window_days: action.window_days,
        })
    }

    pub fn request(&self) -> &Request {
        self.http.backend().request()
    }

    fn read(
        &self,
        path: &str,
        body: Option<Json>,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<Value> {
        let method = if body.is_some() { "POST" } else { "GET" };
        let url = format!("{}{}", self.http.backend.url, path);
        let mut request = HttpRequest::new(method, &url)
            .and_then(|r| r.header("Accept", "application/json"))
            .and_then(|r| r.header("Content-Type", "application/json"))
            .and_then(|r| r.header("Grpc-Metadata-macaroon", &self.macaroon))
            .and_then(|r| r.header("User-Agent", &format!("kassiber/{}", self.http.version)))
            .map_err(|e| e.app_error())?;
        if let Some(body) = body {
            request = request.body(serde_json::to_vec(&body).expect("JSON request"));
        }
        let response = self
            .http
            .send(request, consent, deadline, RESPONSE_LIMIT)
            .map_err(|error| {
                if [
                    "interaction_required",
                    "network_egress_disabled",
                    "network_proxy_unsupported",
                    "network_proxy_required",
                    "backend_tls_config_invalid",
                ]
                .contains(&error.code.as_str())
                {
                    error
                } else {
                    AppError::new(
                        "backend_error",
                        format!("Failed to reach LND backend: {}", error.message),
                    )
                    .with_hint(
                        "Check the LND REST host, port, TLS certificate, and network reachability.",
                    )
                    .retryable(true)
                }
            })?;
        if !(200..300).contains(&response.status) {
            let mut error = AppError::new(
                "backend_error",
                format!("LND returned HTTP {}", response.status),
            )
            .retryable((500..600).contains(&response.status));
            if [401, 403].contains(&response.status) {
                error = error
                    .with_hint("Check that the macaroon is read-only and valid for this node.");
            }
            return Err(error);
        }
        if response.body.is_empty() {
            return Ok(dict([] as [(&str, Value); 0]));
        }
        let raw = std::str::from_utf8(&response.body).map_err(|_| {
            AppError::new("backend_error", "LND returned invalid UTF-8").retryable(true)
        })?;
        let value = kassiber_pytext::loads(raw).map_err(|_| {
            AppError::new("backend_error", "LND returned non-JSON data").retryable(true)
        })?;
        if !matches!(value, Value::Dict(_)) {
            return Err(AppError::new(
                "backend_error",
                "LND returned an unexpected response shape",
            )
            .retryable(true));
        }
        Ok(value)
    }

    pub fn fetch(
        &self,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<ProfitabilitySnapshot> {
        let info = self.read("/v1/getinfo", None, consent, deadline)?;
        let now = integer(field(&info, "best_header_timestamp"));
        drop(info);
        self.read("/v1/balance/blockchain", None, consent, deadline)?;
        self.read("/v1/balance/channels", None, consent, deadline)?;
        self.read("/v1/fees", None, consent, deadline)?;
        let open = channel_rows(self.read("/v1/channels", None, consent, deadline)?)?;
        let closed = channel_rows(self.read("/v1/channels/closed", None, consent, deadline)?)?;
        let now = now?;
        let start = if now == BigInt::from(0) {
            now.clone()
        } else {
            (&now - BigInt::from(self.window_days) * 86400u32).max(0.into())
        };
        let forwarding = self.read("/v1/switch", Some(serde_json::json!({
            "start_time": start.to_string(), "end_time": if now == BigInt::from(0) { String::new() } else { now.to_string() }, "num_max_events": 1000
        })), consent, deadline)?;
        let forwards = rows(&forwarding, "forwarding_events")
            .into_iter()
            .filter_map(|row| match integer(field(row, "timestamp")) {
                Ok(timestamp)
                    if start != BigInt::from(0)
                        && timestamp != BigInt::from(0)
                        && timestamp < start =>
                {
                    None
                }
                Ok(_) => Some(Ok(row)),
                Err(error) => Some(Err(error)),
            })
            .map(|row| {
                let row = row?;
                Ok(Forward {
                    incoming: optional_text(field(row, "chan_id_in")),
                    outgoing: optional_text(field(row, "chan_id_out")),
                    fee_msat: fee(row, "fee")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        drop(forwarding);
        let payments = self.read(
            "/v1/payments?include_incomplete=true&max_payments=1000",
            None,
            consent,
            deadline,
        )?;
        let mut payment_cost: BigInt = 0.into();
        let mut payment_count = 0;
        for row in rows(&payments, "payments") {
            if start != BigInt::from(0)
                && floor_div(integer(field(row, "creation_time_ns"))?, 1_000_000_000) < start
                && integer(field(row, "creation_date"))? < start
            {
                continue;
            }
            if ["SUCCEEDED", "SUCCESS", "COMPLETE"]
                .contains(&kassiber_pytext::upper(&py_str(field(row, "status"))).as_str())
            {
                payment_count += 1;
                payment_cost += floor_div(fee(row, "fee_sat")?, 1000);
            }
        }
        drop(payments);
        self.read(
            "/v1/invoices?num_max_invoices=1000",
            None,
            consent,
            deadline,
        )?;
        let mut aliases = BTreeMap::new();
        let peers = open
            .iter()
            .chain(&closed)
            .filter(|c| c.private && !truthy(&c.alias))
            .filter_map(|c| c.peer.clone())
            .collect::<BTreeSet<_>>();
        for peer in peers {
            let peer_utf8 = peer.as_str().ok_or_else(|| {
                AppError::new("internal_error", "Invalid LND peer identifier encoding")
            })?;
            let mut path = String::from("/v1/graph/node/");
            for b in peer_utf8.bytes() {
                if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) {
                    path.push(b as char);
                } else {
                    path.push_str(&format!("%{b:02X}"));
                }
            }
            if let Ok(payload) = self.read(&path, None, consent, deadline) {
                let alias = field(field(&payload, "node"), "alias");
                let alias = if truthy(alias) {
                    text(alias)
                } else {
                    PyStr::default()
                };
                let alias = kassiber_pytext::strip_py(&alias);
                if !alias.is("") {
                    aliases.insert(peer, alias);
                }
            }
        }
        for channel in open.iter().chain(&closed) {
            validate_short_id(channel.short_id.as_ref())?;
            if let Some(point) = &channel.point {
                validate_outpoint(point)?;
            }
        }
        let mut earned: BTreeMap<PyStr, BigInt> = BTreeMap::new();
        let mut revenue_msat: BigInt = 0.into();
        for forward in &forwards {
            validate_short_id(forward.incoming.as_ref())?;
            validate_short_id(forward.outgoing.as_ref())?;
            revenue_msat += &forward.fee_msat;
            if let Some(outgoing) = &forward.outgoing {
                *earned.entry(outgoing.clone()).or_insert_with(|| 0.into()) +=
                    floor_div(forward.fee_msat.clone(), 1000);
            }
        }
        let revenue = floor_div(revenue_msat, 1000);
        let channels = open
            .into_iter()
            .map(|c| {
                let alias = if truthy(&c.alias) {
                    text(&c.alias)
                } else if let Some(alias) = c.peer.as_ref().and_then(|p| aliases.get(p)) {
                    alias.clone()
                } else if c.private {
                    "private peer".into()
                } else {
                    c.peer.unwrap_or_else(|| "unknown".into())
                };
                Channel {
                    id: c.id.clone(),
                    peer_alias: alias,
                    capacity_sat: c.capacity,
                    earned_routing_sat: earned.get(&c.id).cloned(),
                }
            })
            .collect();
        Ok(ProfitabilitySnapshot {
            channels,
            routing: Some(Routing {
                window_label: format!("Last {} days", self.window_days),
                net_profit_sat: &revenue - &payment_cost,
                routing_revenue_sat: revenue,
                payment_cost_sat: payment_cost,
                rebalance_cost_sat: 0.into(),
                onchain_cost_sat: 0.into(),
                forward_count: forwards.len(),
                payment_count,
                rebalance_count: 0,
            }),
        })
    }
}

struct Forward {
    incoming: Option<PyStr>,
    outgoing: Option<PyStr>,
    fee_msat: BigInt,
}

struct ChannelRow {
    id: PyStr,
    short_id: Option<PyStr>,
    point: Option<Value>,
    peer: Option<PyStr>,
    alias: Value,
    private: bool,
    capacity: BigInt,
}

fn channel_rows(payload: Value) -> Result<Vec<ChannelRow>> {
    rows(&payload, "channels")
        .into_iter()
        .map(|row| {
            let id = optional_text(field(row, "chan_id"));
            let point = ["channel_point", "funding_txid", "closing_tx_hash"]
                .into_iter()
                .map(|k| field(row, k))
                .find(|v| truthy(v));
            Ok(ChannelRow {
                id: id.clone().or_else(|| point.map(text)).unwrap_or_default(),
                short_id: id,
                point: point.cloned(),
                peer: optional_text(field(row, "remote_pubkey")),
                alias: field(row, "peer_alias").clone(),
                private: lnd_truthy(field(row, "private")),
                capacity: integer(field(row, "capacity"))?,
            })
        })
        .collect()
}

fn field<'a>(row: &'a Value, key: &str) -> &'a Value {
    get(row, key).unwrap_or(&Value::Null)
}
fn rows<'a>(payload: &'a Value, key: &str) -> Vec<&'a Value> {
    match field(payload, key) {
        Value::List(rows) => rows
            .iter()
            .filter(|r| matches!(r, Value::Dict(_)))
            .collect(),
        _ => Vec::new(),
    }
}
fn text(value: &Value) -> PyStr {
    match value {
        Value::Str(s) => s.clone(),
        _ => py_str(value).into(),
    }
}
fn optional_text(value: &Value) -> Option<PyStr> {
    truthy(value).then(|| text(value))
}
fn lnd_truthy(value: &Value) -> bool {
    if let Value::Str(s) = value {
        return s.as_str().is_some_and(|s| {
            ["true", "1", "yes"]
                .contains(&kassiber_pytext::lower(kassiber_pytext::strip(s)).as_str())
        });
    }
    truthy(value)
}
fn integer(value: &Value) -> Result<BigInt> {
    Ok(match value {
        Value::Int(n) => n.clone(),
        Value::Bool(b) => u8::from(*b).into(),
        Value::Str(s) => s
            .as_str()
            .and_then(|s| kassiber_pytext::parse_int(s).ok())
            .unwrap_or_else(|| 0.into()),
        Value::Float(n) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Float(n) if n.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", n.trunc())).unwrap_or_else(|_| 0.into())
        }
        _ => 0.into(),
    })
}
fn fee(row: &Value, sat_field: &str) -> Result<BigInt> {
    let msat = integer(field(row, "fee_msat"))?;
    Ok(if msat == BigInt::from(0) {
        integer(field(row, sat_field))? * 1000
    } else {
        msat
    })
}
fn floor_div(value: BigInt, divisor: u64) -> BigInt {
    let quotient = &value / divisor;
    if value < BigInt::from(0) && &value % divisor != BigInt::from(0) {
        quotient - 1
    } else {
        quotient
    }
}
fn validate_short_id(value: Option<&PyStr>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let valid = value.as_str().is_some_and(|s| {
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        (s.len() <= 20 && digits(s)) || (s.split('x').count() == 3 && s.split('x').all(digits))
    });
    if valid {
        Ok(())
    } else {
        Err(AppError::new(
            "internal_error",
            "Invalid Lightning short channel identifier",
        ))
    }
}
fn validate_outpoint(value: &Value) -> Result<()> {
    let valid = matches!(value, Value::Str(s) if s.as_str().and_then(|s| s.split_once(':')).is_some_and(|(txid, vout)| txid.len() == 64 && txid.bytes().all(|b| b.is_ascii_hexdigit()) && !vout.is_empty() && vout.bytes().all(|b| b.is_ascii_digit())));
    if valid {
        Ok(())
    } else {
        Err(AppError::new(
            "internal_error",
            "Invalid Lightning funding outpoint",
        ))
    }
}
fn expand_path(raw: &str) -> PathBuf {
    if let Some(suffix) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(suffix);
        }
    }
    raw.into()
}

#[cfg(test)]
mod tests;
