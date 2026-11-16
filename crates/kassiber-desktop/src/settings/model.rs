//! Pure rules behind the panels: backend rows (SettingsModel.ts), the privacy posture of an
//! endpoint (lib/backendTrust.ts), explorer links, the market-rate rows, the backend payload
//! the daemon takes, and small formatters. No UI, no I/O; unit-tested below.

use serde_json::{Map, Value, json};

pub const PLAINTEXT_DELETE_ACK: &str = "DELETE LOCAL DATA";
pub const WIPE_EVERYTHING_ACK: &str = "WIPE EVERYTHING";
pub const KRAKEN_OHLCVT_SUPPORT_URL: &str = "https://support.kraken.com/hc/articles/360047124832-Downloadable-historical-OHLCVT-Open-High-Low-Close-Volume-Trades-data";
pub const KRAKEN_MARKET_DATA_BLOG_URL: &str =
    "https://blog.kraken.com/product/api/unlocked-3-the-market-data-feeds-systematic-traders-use";
pub const CLN_SENTINEL_COMMANDO_PEER: &str = "Configured peer";
pub const CLN_SENTINEL_LIGHTNING_DIR: &str = "Configured directory";
pub const CLN_SENTINEL_RPC_FILE: &str = "Configured RPC file";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Net {
    #[default]
    Btc,
    Liquid,
    Ln,
    Fx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Mine,
    ThirdParty,
}

impl Owner {
    pub fn wire(self) -> &'static str {
        match self {
            Owner::Mine => "self",
            Owner::ThirdParty => "third_party",
        }
    }
    pub fn parse(v: &str) -> Option<Owner> {
        match v {
            "self" => Some(Owner::Mine),
            "third_party" => Some(Owner::ThirdParty),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub host: String,
    pub port: String,
    pub redacted: bool,
}

/// A backend as the panels show it (`Backend` in SettingsModel.ts). `proxy`: None = not set by
/// the form (leave it), Some(None) = clear it, Some(Some(p)) = this proxy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Backend {
    pub id: String,
    pub name: String,
    pub url: String,
    pub net_kind: Net,
    pub kind: Option<String>,
    pub chain: Option<String>,
    pub network: Option<String>,
    pub health: String,
    pub on: bool,
    pub is_default: bool,
    pub auth: String,
    pub auth_header: Option<String>,
    pub token: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub commando_peer_id: Option<String>,
    pub lightning_cli: Option<String>,
    pub lightning_dir: Option<String>,
    pub rpc_file: Option<String>,
    pub trust_ssl: Option<bool>,
    pub infrastructure_owner: Option<Owner>,
    pub certificate: Option<String>,
    pub silent_payments: Option<bool>,
    pub sp_scan_file: Option<String>,
    pub sp_scan_path: Option<String>,
    pub proxy: Option<Option<Proxy>>,
    pub wallet_refs: Vec<String>,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn opt(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

/// `backendNetFromRow`.
pub fn net_from_row(row: &Value) -> Net {
    let chain = s(row, "chain").to_lowercase();
    let kind = s(row, "kind").to_lowercase();
    if kind == "lnd" || kind == "coreln" {
        return Net::Ln;
    }
    if chain == "liquid" || kind == "liquid-esplora" {
        return Net::Liquid;
    }
    if chain == "fx"
        || chain == "market"
        || ["coinbase-exchange", "coingecko", "kraken-csv"].contains(&kind.as_str())
    {
        return Net::Fx;
    }
    Net::Btc
}

/// `backendAuthLabel`.
pub fn auth_label(row: &Value) -> &'static str {
    if b(row, "has_auth_header") {
        "bearer"
    } else if b(row, "has_token") {
        "apikey"
    } else if b(row, "has_username") || b(row, "has_password") {
        "basic"
    } else {
        "none"
    }
}

/// `parseProxyEndpoint`.
pub fn parse_proxy(value: &str) -> Option<Proxy> {
    let t = value.trim();
    if t.is_empty() {
        return None;
    }
    if t.contains("://") {
        let u = Url::parse(t)?;
        if u.host.is_empty() || u.port.is_empty() {
            return None;
        }
        return Some(Proxy {
            host: u.host,
            port: u.port,
            redacted: u.user == "redacted",
        });
    }
    let (host, port) = t.rsplit_once(':')?;
    if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(Proxy {
        host: host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned(),
        port: port.to_owned(),
        redacted: false,
    })
}

/// `backendRowToSettingsBackend`.
pub fn backend_from_row(row: &Value) -> Backend {
    let net_kind = net_from_row(row);
    let id = if s(row, "name").is_empty() {
        "backend".to_owned()
    } else {
        s(row, "name").to_owned()
    };
    let display = s(row, "display_name").trim();
    let name = if display.is_empty() {
        id.clone()
    } else {
        display.to_owned()
    };
    let is_default = b(row, "is_default") || s(row, "default") == "yes";
    let url = match s(row, "url") {
        "" if b(row, "has_url") => "Configured endpoint".to_owned(),
        "" => "Missing endpoint".to_owned(),
        u => u.to_owned(),
    };
    let health = if is_default {
        "default".to_owned()
    } else {
        [s(row, "source"), s(row, "kind")]
            .into_iter()
            .find(|v| !v.is_empty())
            .unwrap_or("configured")
            .to_owned()
    };
    Backend {
        id,
        name,
        url,
        net_kind,
        kind: opt(row, "kind"),
        chain: opt(row, "chain"),
        network: opt(row, "network"),
        health,
        on: row.get("has_url").and_then(Value::as_bool) != Some(false),
        is_default,
        auth: auth_label(row).to_owned(),
        commando_peer_id: b(row, "has_commando_peer_id")
            .then(|| CLN_SENTINEL_COMMANDO_PEER.to_owned()),
        lightning_dir: b(row, "has_lightning_dir").then(|| CLN_SENTINEL_LIGHTNING_DIR.to_owned()),
        rpc_file: b(row, "has_rpc_file").then(|| CLN_SENTINEL_RPC_FILE.to_owned()),
        certificate: opt(row, "certificate"),
        trust_ssl: row.get("insecure").and_then(Value::as_bool),
        infrastructure_owner: Owner::parse(s(row, "infrastructure_owner")),
        silent_payments: Some(b(row, "silent_payments")),
        proxy: Some(parse_proxy(s(row, "tor_proxy"))),
        wallet_refs: row
            .get("wallet_refs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        ..Backend::default()
    }
}

/// The book's backends (`ui.backends.settings.list`), FX rows from maintenance settings added
/// when the list has none (`SettingsScreen` `backends`).
pub fn backends(list: Option<&Value>, maintenance: Option<&Value>) -> Vec<Backend> {
    let rows: Vec<Backend> = list
        .and_then(|v| v["backends"].as_array())
        .map(|a| a.iter().map(backend_from_row).collect())
        .unwrap_or_default();
    if rows.iter().any(|b| b.net_kind == Net::Fx) {
        return rows;
    }
    let mut out = rows.clone();
    out.extend(market_rate_backends(
        maintenance.map(|m| &m["settings"]),
        &rows,
    ));
    out
}

pub fn bootstrap_mode(list: Option<&Value>) -> &str {
    list.and_then(|v| v["summary"]["bootstrap_mode"].as_str())
        .unwrap_or("public")
}

pub const MARKET_PROVIDERS: [&str; 3] = ["coinbase-exchange", "coingecko", "mempool"];

/// `marketRateProviderLabel` (hard-coded English in the web).
pub fn provider_label(p: &str) -> String {
    match p {
        "coinbase-exchange" => "Coinbase Exchange".into(),
        "coingecko" => "CoinGecko".into(),
        "mempool" => "Configured mempool backend".into(),
        other => other.into(),
    }
}

fn is_http(url: &str) -> bool {
    Url::parse(url).is_some_and(|u| u.scheme == "http" || u.scheme == "https")
}

/// `configuredBitcoinPriceBackend`.
fn price_backend(backends: &[Backend]) -> Option<&Backend> {
    let mut c: Vec<&Backend> = backends
        .iter()
        .filter(|b| {
            let kind = b.kind.clone().unwrap_or_default().to_lowercase();
            let kind = if kind.is_empty() {
                "esplora".to_owned()
            } else {
                kind
            };
            b.net_kind == Net::Btc && (kind == "esplora" || kind == "mempool") && is_http(&b.url)
        })
        .collect();
    c.sort_by(|a, b2| {
        let mine = |x: &Backend| {
            if x.infrastructure_owner == Some(Owner::Mine) || inferred_owner(&x.url) == Owner::Mine
            {
                0
            } else {
                1
            }
        };
        mine(a)
            .cmp(&mine(b2))
            .then((!a.is_default).cmp(&!b2.is_default))
            .then(a.id.cmp(&b2.id))
    });
    c.into_iter().next()
}

/// `marketRateBackends`.
pub fn market_rate_backends(settings: Option<&Value>, configured: &[Backend]) -> Vec<Backend> {
    let selected = settings
        .and_then(|s| s["market_rate_provider"].as_str())
        .unwrap_or("coinbase-exchange");
    let auto = settings
        .and_then(|s| s["source_classes"]["market_rates"].as_bool())
        .unwrap_or(false);
    let providers: Vec<String> = settings
        .and_then(|s| s["market_rate_providers"].as_array())
        .filter(|a| !a.is_empty())
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| MARKET_PROVIDERS.iter().map(|s| s.to_string()).collect());
    let mempool = price_backend(configured);
    providers
        .iter()
        .map(|p| {
            let on = auto && selected == p;
            if p == "mempool" {
                return Backend {
                    id: "market-mempool".into(),
                    name: provider_label("mempool"),
                    url: mempool
                        .map(|m| m.url.clone())
                        .unwrap_or_else(|| "No HTTP mempool backend configured".into()),
                    net_kind: Net::Fx,
                    kind: Some("mempool".into()),
                    chain: Some("market".into()),
                    network: Some("main".into()),
                    health: mempool
                        .map(|m| format!("via {}", m.name))
                        .unwrap_or_else(|| "configure backend".into()),
                    on: on && mempool.is_some(),
                    auth: "none".into(),
                    proxy: Some(mempool.and_then(|m| m.proxy.clone().flatten())),
                    infrastructure_owner: mempool.map(|m| {
                        m.infrastructure_owner
                            .unwrap_or_else(|| inferred_owner(&m.url))
                    }),
                    ..Backend::default()
                };
            }
            Backend {
                id: format!("market-{p}"),
                name: provider_label(p),
                url: if p == "coinbase-exchange" {
                    "https://api.exchange.coinbase.com".into()
                } else {
                    "https://api.coingecko.com/api/v3".into()
                },
                net_kind: Net::Fx,
                kind: Some(p.clone()),
                chain: Some("market".into()),
                network: Some("main".into()),
                health: if on {
                    "selected".into()
                } else {
                    "available".into()
                },
                on,
                auth: "none".into(),
                ..Backend::default()
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Bitcoin,
    Lightning,
    Liquid,
}

impl Layer {
    pub fn id(self) -> &'static str {
        match self {
            Layer::Bitcoin => "bitcoin",
            Layer::Lightning => "lightning",
            Layer::Liquid => "liquid",
        }
    }
    pub fn net_kind(self) -> Net {
        match self {
            Layer::Bitcoin => Net::Btc,
            Layer::Lightning => Net::Ln,
            Layer::Liquid => Net::Liquid,
        }
    }
}

pub fn for_layer(backends: &[Backend], layer: Layer) -> Vec<&Backend> {
    backends
        .iter()
        .filter(|b| b.net_kind == layer.net_kind())
        .collect()
}

// ---- URLs ----

/// The parts of a URL the panels look at (WHATWG `URL` for the shapes Kassiber stores).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub user: String,
    pub host: String,
    pub port: String,
    pub rest: String,
}

impl Url {
    pub fn parse(raw: &str) -> Option<Url> {
        let raw = raw.trim();
        let (scheme, after) = raw.split_once("://")?;
        if scheme.is_empty()
            || !scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        {
            return None;
        }
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let (authority, rest) = after.split_at(end);
        let (user, hostport) = match authority.rfind('@') {
            Some(i) => (&authority[..i], &authority[i + 1..]),
            None => ("", authority),
        };
        let (host, port) = if let Some(stripped) = hostport.strip_prefix('[') {
            let close = stripped.find(']')?;
            let h = &stripped[..close];
            let p = stripped[close + 1..].strip_prefix(':').unwrap_or("");
            (h.to_owned(), p.to_owned())
        } else {
            match hostport.rsplit_once(':') {
                Some((h, p)) => (h.to_owned(), p.to_owned()),
                None => (hostport.to_owned(), String::new()),
            }
        };
        if !port.chars().all(|c| c.is_ascii_digit()) || host.chars().any(|c| c.is_whitespace()) {
            return None;
        }
        let scheme = scheme.to_lowercase();
        let http_like = scheme == "http" || scheme == "https";
        if http_like && host.is_empty() {
            return None;
        }
        // Default ports are dropped, as `URL` does.
        let port = if (scheme == "http" && port == "80") || (scheme == "https" && port == "443") {
            String::new()
        } else {
            port
        };
        Some(Url {
            scheme,
            user: user.split(':').next().unwrap_or("").to_owned(),
            host: host.to_lowercase(),
            port,
            rest: rest.to_owned(),
        })
    }

    /// `URL.host`: hostname plus a non-default port (IPv6 bracketed).
    pub fn host_port(&self) -> String {
        let h = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port.is_empty() {
            h
        } else {
            format!("{h}:{}", self.port)
        }
    }

    /// `URL.toString()` without credentials.
    pub fn display(&self) -> String {
        let rest = if self.rest.is_empty() && (self.scheme == "http" || self.scheme == "https") {
            "/".to_owned()
        } else {
            self.rest.clone()
        };
        format!("{}://{}{}", self.scheme, self.host_port(), rest)
    }
}

/// `endpointHost`: a host even for schemes like `ssl://` or a bare `host:port`.
pub fn endpoint_host(url: &str) -> String {
    let raw = url.trim();
    if raw.is_empty() {
        return String::new();
    }
    let candidate = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    Url::parse(&candidate)
        .map(|u| u.host)
        .unwrap_or_else(|| raw.to_lowercase())
}

pub fn is_onion(url: &str) -> bool {
    endpoint_host(url).ends_with(".onion")
}

/// `isLocalOrPrivateHost`.
pub fn is_local_host(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    if host == "localhost"
        || host == "0.0.0.0"
        || host == "::1"
        || host.ends_with(".local")
        || host.ends_with(".localhost")
    {
        return true;
    }
    let oct: Vec<&str> = host.split('.').collect();
    let n = |i: usize| oct.get(i).and_then(|o| o.parse::<u32>().ok());
    if oct.len() >= 2 && oct[0].chars().all(|c| c.is_ascii_digit()) {
        match (n(0), n(1)) {
            (Some(127), _) | (Some(10), _) => return true,
            (Some(192), Some(168)) | (Some(169), Some(254)) => return true,
            (Some(172), Some(x)) if (16..=31).contains(&x) => return true,
            _ => {}
        }
    }
    let lower = host.to_lowercase();
    if lower.contains(':') {
        let first = lower.split(':').next().unwrap_or("");
        if (first.starts_with("fc") || first.starts_with("fd")) && !first.is_empty() {
            return true;
        }
        if first.len() >= 3
            && first.starts_with("fe")
            && matches!(first.as_bytes()[2], b'8' | b'9' | b'a' | b'b')
        {
            return true;
        }
    }
    false
}

pub fn inferred_owner(url: &str) -> Owner {
    if is_local_host(&endpoint_host(url)) {
        Owner::Mine
    } else {
        Owner::ThirdParty
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Posture {
    OnDevice,
    SelfHosted,
    Shielded,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trust {
    pub posture: Posture,
    pub label: String,
    pub note: String,
}

const PROXY_AWARE: [&str; 6] = [
    "bitcoinrpc",
    "btcpay",
    "electrum",
    "esplora",
    "liquid-esplora",
    "mempool",
];

/// `backendTrustFromEndpoint`. The web keeps these in English; here they come from the
/// bundles (`settings:trust.*`, English values unchanged) so German reads German.
pub fn trust_for(url: &str, has_proxy: bool, kind: Option<&str>, owner: Option<Owner>) -> Trust {
    use crate::i18n::t;
    let host = endpoint_host(url);
    let onion = host.ends_with(".onion");
    let proxied =
        has_proxy && kind.is_none_or(|k| PROXY_AWARE.contains(&k.to_lowercase().as_str()));
    if is_local_host(&host) {
        return Trust {
            posture: Posture::OnDevice,
            label: t("settings:trust.onDevice"),
            note: t("settings:trust.onDeviceNote"),
        };
    }
    if owner == Some(Owner::Mine) {
        return Trust {
            posture: Posture::SelfHosted,
            label: t("settings:trust.self"),
            note: t("settings:trust.selfNote"),
        };
    }
    if onion || proxied {
        let label = match (owner == Some(Owner::ThirdParty), onion) {
            (true, true) => "settings:trust.thirdPartyTor",
            (true, false) => "settings:trust.thirdPartyProxy",
            (false, true) => "settings:trust.tor",
            (false, false) => "settings:trust.proxy",
        };
        let note = if onion {
            "settings:trust.torNote"
        } else {
            "settings:trust.proxyNote"
        };
        return Trust {
            posture: Posture::Shielded,
            label: t(label),
            note: t(note),
        };
    }
    Trust {
        posture: Posture::Remote,
        label: t("settings:trust.remote"),
        note: t("settings:trust.remoteNote"),
    }
}

pub fn backend_trust(b: &Backend) -> Trust {
    trust_for(
        &b.url,
        b.proxy.as_ref().is_some_and(Option::is_some),
        b.kind.as_deref(),
        b.infrastructure_owner,
    )
}

/// `backendProtocolLabel`.
pub fn protocol_label(b: &Backend) -> &'static str {
    match b.kind.as_deref().unwrap_or("").to_lowercase().as_str() {
        "esplora" | "liquid-esplora" => "Explorer API",
        "electrum" => "Electrum / Fulcrum",
        "bitcoinrpc" => "Bitcoin Core RPC",
        "btcpay" => "BTCPay",
        "lnd" => "LND REST",
        "coreln" => "Core Lightning",
        _ if b.net_kind == Net::Fx => "Rate provider",
        _ => "Endpoint",
    }
}

/// `backendExplorerBaseUrl`: Explorer-API backends' base, `/api` and trailing slashes dropped.
pub fn explorer_base(b: &Backend) -> Option<String> {
    let kind = b.kind.as_deref().unwrap_or("").to_lowercase();
    if kind != "esplora" && kind != "liquid-esplora" {
        return None;
    }
    let u = Url::parse(&b.url)?;
    if u.scheme != "http" && u.scheme != "https" {
        return None;
    }
    let mut s = u.display().trim().trim_end_matches('/').to_owned();
    if s.to_lowercase().ends_with("/api") {
        s.truncate(s.len() - 4);
    }
    Some(s)
}

/// `explorerHostLabel` / `endpointHostLabel`.
pub fn host_label(url: &str) -> String {
    if url.is_empty() {
        return "No endpoint".into();
    }
    Url::parse(url)
        .map(|u| u.host_port())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| url.to_owned())
}

/// `deriveExplorerSettings().publicFallbacks`.
pub fn public_fallbacks(backends: &[Backend], bootstrap: &str) -> bool {
    let regtest_default = backends.iter().any(|b| {
        b.is_default
            && matches!(
                b.network.as_deref().unwrap_or("").to_lowercase().as_str(),
                "regtest" | "elementsregtest"
            )
    });
    bootstrap == "public" && !regtest_default
}

pub fn is_coreln_commando(b: &Backend) -> bool {
    b.kind
        .as_deref()
        .unwrap_or("")
        .eq_ignore_ascii_case("coreln")
        && (b.url.to_lowercase().starts_with("cln://commando")
            || b.auth == "apikey"
            || b.commando_peer_id
                .as_deref()
                .is_some_and(|p| !p.trim().is_empty()))
}

/// `backendPayload`: what `ui.backends.create` / `update` take.
pub fn backend_payload(b: &Backend) -> Value {
    let chain = b.chain.clone().unwrap_or_else(|| {
        if b.net_kind == Net::Liquid {
            "liquid"
        } else {
            "bitcoin"
        }
        .into()
    });
    let network = b.network.clone().unwrap_or_else(|| {
        if b.net_kind == Net::Liquid {
            "liquidv1"
        } else {
            "main"
        }
        .into()
    });
    let kind = b.kind.clone().unwrap_or_else(|| {
        if b.net_kind == Net::Liquid {
            "electrum"
        } else {
            "esplora"
        }
        .into()
    });
    let mut payload = Map::new();
    payload.insert(
        "name".into(),
        json!(if b.id.is_empty() { &b.name } else { &b.id }),
    );
    payload.insert("kind".into(), json!(kind));
    payload.insert("url".into(), json!(b.url));
    payload.insert("chain".into(), json!(chain));
    payload.insert("network".into(), json!(network));
    let mut config = Map::new();
    if !b.name.trim().is_empty() {
        config.insert("display_name".into(), json!(b.name.trim()));
    }
    if let Some(t) = b.trust_ssl {
        config.insert("insecure".into(), json!(t));
    }
    if let Some(o) = b.infrastructure_owner {
        config.insert("infrastructure_owner".into(), json!(o.wire()));
    }
    if let Some(c) = b.certificate.as_deref().filter(|c| !c.is_empty()) {
        config.insert("certificate".into(), json!(c));
    }
    if let Some(sp) = b.silent_payments {
        config.insert("silent_payments".into(), json!(sp));
    }
    let sp_on = b.silent_payments != Some(false);
    if sp_on {
        if let Some(f) = b
            .sp_scan_file
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
        {
            config.insert("silent_payment_scan_file".into(), json!(f));
        }
        if let Some(p) = b
            .sp_scan_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            config.insert("silent_payment_scan_path".into(), json!(p));
        }
    }
    for (k, v) in [
        ("lightning_cli", &b.lightning_cli),
        ("lightning_dir", &b.lightning_dir),
        ("rpc_file", &b.rpc_file),
    ] {
        if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
            config.insert(k.into(), json!(v));
        }
    }
    let mut clear: Vec<&str> = Vec::new();
    let add = |c: &mut Vec<&str>, k: &'static str| {
        if !c.contains(&k) {
            c.push(k);
        }
    };
    let kind_l = kind.to_lowercase();
    if kind_l == "coreln" {
        add(&mut clear, "commando_peer_id");
    }
    let http_tls = ["esplora", "liquid-esplora", "bitcoinrpc"].contains(&kind_l.as_str());
    if http_tls && b.trust_ssl.is_none() {
        config.insert("insecure".into(), json!(false));
        add(&mut clear, "certificate");
    } else if http_tls
        && (b.trust_ssl == Some(true)
            || b.certificate.as_deref().is_none_or(|c| c.trim().is_empty()))
    {
        add(&mut clear, "certificate");
    }
    if clear.contains(&"certificate") {
        config.remove("certificate");
    }
    match b.auth.as_str() {
        "none" => ["auth_header", "token", "username", "password"]
            .into_iter()
            .for_each(|k| add(&mut clear, k)),
        "bearer" => ["token", "username", "password"]
            .into_iter()
            .for_each(|k| add(&mut clear, k)),
        "apikey" => ["auth_header", "username", "password"]
            .into_iter()
            .for_each(|k| add(&mut clear, k)),
        "basic" => ["auth_header", "token"]
            .into_iter()
            .for_each(|k| add(&mut clear, k)),
        _ => {}
    }
    if b.auth == "bearer"
        && let Some(h) = b.auth_header.as_deref().filter(|h| !h.is_empty())
    {
        payload.insert("auth_header".into(), json!(h));
    }
    if b.auth == "apikey"
        && let Some(t) = b.token.as_deref().filter(|t| !t.is_empty())
    {
        payload.insert("token".into(), json!(t));
    }
    if b.auth == "basic" {
        if let Some(u) = b.username.as_deref().filter(|u| !u.is_empty()) {
            config.insert("username".into(), json!(u));
        }
        if let Some(p) = b.password.as_deref().filter(|p| !p.is_empty()) {
            config.insert("password".into(), json!(p));
        }
    }
    match &b.proxy {
        Some(Some(p)) if p.redacted => {}
        Some(Some(p)) if !p.host.is_empty() && !p.port.is_empty() => {
            payload.insert("tor_proxy".into(), json!(format!("{}:{}", p.host, p.port)));
        }
        Some(None) => add(&mut clear, "tor_proxy"),
        _ => {}
    }
    if !config.is_empty() {
        payload.insert("config".into(), Value::Object(config));
    }
    if !clear.is_empty() {
        payload.insert("clear".into(), json!(clear));
    }
    Value::Object(payload)
}

/// `formatBytes`.
pub fn format_bytes(bytes: f64) -> String {
    if bytes < 1024.0 {
        format!("{} B", bytes as u64)
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KiB", bytes / 1024.0)
    } else {
        format!("{:.1} MiB", bytes / (1024.0 * 1024.0))
    }
}

/// `aiSecretStoreLabel`.
pub fn secret_store_label(id: &str) -> &'static str {
    match id {
        "macos_keychain" => "macOS Keychain",
        "windows_dpapi" => "Windows Credential Manager",
        "linux_secret_service" => "Secret Service",
        _ => "SQLCipher inline",
    }
}

/// `aiSecretStateLabel`.
pub fn secret_state_label(state: &str) -> &'static str {
    match state {
        "ok" => "ok",
        "needs_reauth" => "needs re-entry",
        "unavailable" => "unavailable",
        _ => "missing",
    }
}

/// CLI agents behind the broker (`NATIVE_AI_PROVIDER_BY_LOCATOR`).
pub fn native_runtime(url: &str) -> Option<&'static str> {
    match url.trim().to_lowercase().as_str() {
        "claude-cli://default" => Some("claude"),
        "codex-cli://default" => Some("codex"),
        "opencode-cli://default" => Some("opencode"),
        "copilot-cli://default" => Some("copilot"),
        _ => None,
    }
}

pub fn is_native_locator(url: &str) -> bool {
    native_runtime(url).is_some()
}

/// `isLoopbackAiHttpUrl`: localhost, 127/8 or an IPv6 loopback.
pub fn is_loopback_http(raw: &str) -> bool {
    let raw = raw.trim();
    let Some(rest) = raw
        .strip_prefix("http://")
        .or_else(|| raw.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let hostinfo = &authority[authority.rfind('@').map(|i| i + 1).unwrap_or(0)..];
    let host = if let Some(h) = hostinfo.strip_prefix('[') {
        match h.find(']') {
            Some(e) => h[..e].to_lowercase(),
            None => return false,
        }
    } else {
        hostinfo.split(':').next().unwrap_or("").to_lowercase()
    };
    if host == "localhost" {
        return true;
    }
    let oct: Vec<&str> = host.split('.').collect();
    if oct.len() == 4
        && oct
            .iter()
            .all(|o| !o.is_empty() && o.len() <= 3 && o.chars().all(|c| c.is_ascii_digit()))
    {
        if oct
            .iter()
            .any(|o| (o.len() > 1 && o.starts_with('0')) || o.parse::<u32>().unwrap_or(999) > 255)
        {
            return false;
        }
        return oct[0] == "127";
    }
    if !host.contains(':') {
        return false;
    }
    host.parse::<std::net::Ipv6Addr>().is_ok_and(|a| {
        a.is_loopback() || a.to_ipv4_mapped().is_some_and(|v4| v4.octets()[0] == 127)
    })
}

/// `httpOrigin`.
pub fn http_origin(raw: &str) -> Option<String> {
    let u = Url::parse(raw)?;
    if (u.scheme != "http" && u.scheme != "https") || u.host.is_empty() {
        return None;
    }
    Some(format!("{}://{}", u.scheme, u.host_port()).to_lowercase())
}

/// `autoRemoteReason`.
pub fn auto_remote_reason(kind: &str, url: &str) -> Option<&'static str> {
    if kind != "local" {
        return None;
    }
    let url = url.trim();
    if is_native_locator(url) {
        return Some("cli");
    }
    let host_start = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    match host_start {
        Some(r) if r.chars().next().is_some_and(|c| !"/?#".contains(c)) => {}
        _ => return None,
    }
    if is_loopback_http(url) {
        None
    } else {
        Some("url")
    }
}

/// `providerVersionLabel`.
pub fn version_label(v: Option<&str>) -> Option<String> {
    let t = v?.trim();
    if t.is_empty() {
        return None;
    }
    // \d+(\.\d+)+(-[0-9A-Za-z.]+)?
    let bytes: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let mut dots = 0;
            while j + 1 < bytes.len() && bytes[j] == '.' && bytes[j + 1].is_ascii_digit() {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                dots += 1;
            }
            if dots > 0 {
                if j + 1 < bytes.len()
                    && bytes[j] == '-'
                    && (bytes[j + 1].is_ascii_alphanumeric() || bytes[j + 1] == '.')
                {
                    j += 1;
                    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == '.') {
                        j += 1;
                    }
                }
                let m: String = bytes[start..j].iter().collect();
                return Some(format!("v{}", m.trim_end_matches('.')));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    Some(if t.chars().count() > 24 {
        format!("{}…", t.chars().take(23).collect::<String>())
    } else {
        t.to_owned()
    })
}

/// `providerEffortOptions` without a check's model list.
pub fn effort_options(provider: &Value, models: &[Value]) -> Vec<&'static str> {
    const ALL: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];
    let url = provider["base_url"].as_str().unwrap_or("");
    let runtime = native_runtime(url);
    let fixed: Option<&[&str]> = match runtime {
        Some("claude") | Some("copilot") => Some(&["low", "medium", "high", "xhigh", "max"]),
        _ => None,
    };
    let advertised: Vec<String> = match fixed {
        Some(f) => f.iter().map(|s| s.to_string()).collect(),
        None => models
            .iter()
            .flat_map(|m| {
                m["reasoning_efforts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|e| e.as_str().map(str::to_lowercase))
            .collect(),
    };
    let stored = provider["default_reasoning_effort"].as_str();
    let ceiling: Option<&[&str]> = if runtime.is_some() {
        None
    } else if provider["kind"].as_str() == Some("cli_proxy") {
        Some(&["low", "medium", "high", "xhigh", "max"])
    } else {
        Some(&["low", "medium", "high"])
    };
    let out: Vec<&'static str> = ALL
        .into_iter()
        .filter(|e| {
            Some(*e) == stored
                || ((advertised.is_empty() || advertised.iter().any(|a| a == e))
                    && ceiling.is_none_or(|c| c.contains(e)))
        })
        .collect();
    if out.is_empty() { ALL.to_vec() } else { out }
}

/// `checkedAgo` (English/German relative time, `numeric: "auto"`).
pub fn checked_ago(checked_at: Option<&str>, now: f64) -> Option<String> {
    let at = crate::format::parse_ts(checked_at?)?;
    let secs = (at - now).round();
    let de = crate::i18n::lang() == crate::i18n::Lang::De;
    let abs = secs.abs();
    let (n, unit) = if abs < 45.0 {
        return Some(if de { "jetzt".into() } else { "now".into() });
    } else if abs < 45.0 * 60.0 {
        (
            (secs / 60.0).round(),
            if de {
                ("Minute", "Minuten")
            } else {
                ("minute", "minutes")
            },
        )
    } else if abs < 22.0 * 3600.0 {
        (
            (secs / 3600.0).round(),
            if de {
                ("Stunde", "Stunden")
            } else {
                ("hour", "hours")
            },
        )
    } else {
        (
            (secs / 86400.0).round(),
            if de {
                ("Tag", "Tagen")
            } else {
                ("day", "days")
            },
        )
    };
    let k = n.abs() as i64;
    if de {
        let word = if k == 1 { unit.0 } else { unit.1 };
        return Some(if n < 0.0 {
            format!("vor {k} {word}")
        } else {
            format!("in {k} {word}")
        });
    }
    if k == 1 && unit.0 == "day" {
        return Some(if n < 0.0 {
            "yesterday".into()
        } else {
            "tomorrow".into()
        });
    }
    let word = if k == 1 { unit.0 } else { unit.1 };
    Some(if n < 0.0 {
        format!("{k} {word} ago")
    } else {
        format!("in {k} {word}")
    })
}

/// Shell-quotes like `agentAccess.ts`.
pub fn shell_quote(v: &str) -> String {
    if !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:=@%+-".contains(c))
    {
        v.to_owned()
    } else {
        format!("'{}'", v.replace('\'', "'\\''"))
    }
}

pub fn claude_code_command(launcher: &[String], args: &[String]) -> String {
    ["claude", "mcp", "add", "kassiber", "--"]
        .iter()
        .map(|s| s.to_string())
        .chain(launcher.iter().cloned())
        .chain(args.iter().cloned())
        .map(|s| shell_quote(&s))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn mcp_json_config(launcher: &[String], args: &[String]) -> String {
    let (cmd, rest) = launcher
        .split_first()
        .map(|(c, r)| (c.clone(), r.to_vec()))
        .unwrap_or(("kassiber".into(), vec![]));
    let all: Vec<String> = rest.into_iter().chain(args.iter().cloned()).collect();
    serde_json::to_string_pretty(
        &json!({"mcpServers": {"kassiber": {"command": cmd, "args": all}}}),
    )
    .unwrap_or_default()
}

/// `databasePassphraseIssue`.
pub fn passphrase_issue(p: &str, confirm: &str) -> Option<&'static str> {
    if p.is_empty() {
        Some("required")
    } else if p.chars().count() < 12 {
        Some("tooShort")
    } else if confirm.is_empty() {
        Some("confirmRequired")
    } else if p != confirm {
        Some("mismatch")
    } else {
        None
    }
}

/// A random v4 UUID for the regtest instance id (`crypto.randomUUID()`).
pub fn uuid_v4() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_become_backends() {
        let row = json!({"name": "core-regtest", "kind": "bitcoinrpc", "chain": "bitcoin", "network": "regtest", "url": "http://127.0.0.1:18443", "has_username": true, "has_password": true, "default": "", "source": "database", "tor_proxy": ""});
        let b = backend_from_row(&row);
        assert_eq!(b.name, "core-regtest");
        assert_eq!(b.auth, "basic");
        assert_eq!(b.net_kind, Net::Btc);
        assert_eq!(b.health, "database");
        assert!(b.on);
        assert_eq!(backend_trust(&b).posture, Posture::OnDevice);
        let liquid = backend_from_row(
            &json!({"name": "l", "kind": "electrum", "chain": "liquid", "display_name": "Liquid E", "is_default": true}),
        );
        assert_eq!(liquid.net_kind, Net::Liquid);
        assert_eq!(liquid.name, "Liquid E");
        assert_eq!(liquid.health, "default");
        assert_eq!(liquid.url, "Missing endpoint");
    }

    #[test]
    fn trust_postures() {
        assert_eq!(
            trust_for(
                "ssl://index.bitcoin-austria.at:50002",
                false,
                Some("electrum"),
                None
            )
            .posture,
            Posture::Remote
        );
        assert_eq!(
            trust_for("tcp://abc.onion:50001", false, Some("electrum"), None).label,
            "Tor"
        );
        assert_eq!(
            trust_for("https://x.example", true, Some("lnd"), None).posture,
            Posture::Remote
        );
        assert_eq!(
            trust_for(
                "https://x.example",
                true,
                Some("esplora"),
                Some(Owner::ThirdParty)
            )
            .label,
            "Third-party via proxy"
        );
        assert_eq!(
            trust_for("https://x.example", false, None, Some(Owner::Mine)).posture,
            Posture::SelfHosted
        );
        assert!(
            is_local_host("192.168.1.4")
                && is_local_host("172.20.0.1")
                && !is_local_host("172.40.0.1")
                && is_local_host("fd00::1")
        );
    }

    #[test]
    fn explorer_and_hosts() {
        let b = Backend {
            kind: Some("esplora".into()),
            url: "https://mempool.space/api/".into(),
            ..Default::default()
        };
        assert_eq!(explorer_base(&b).as_deref(), Some("https://mempool.space"));
        assert_eq!(host_label("http://127.0.0.1:18544/api"), "127.0.0.1:18544");
        assert_eq!(endpoint_host("ssl://Index.Example:50002"), "index.example");
        assert_eq!(
            parse_proxy("127.0.0.1:9050"),
            Some(Proxy {
                host: "127.0.0.1".into(),
                port: "9050".into(),
                redacted: false
            })
        );
        assert_eq!(
            parse_proxy("socks5h://redacted:x@10.0.0.1:9050").map(|p| p.redacted),
            Some(true)
        );
    }

    #[test]
    fn payload_clears_what_the_auth_mode_drops() {
        let b = Backend {
            id: "core".into(),
            name: "Core".into(),
            url: "http://127.0.0.1:8332".into(),
            kind: Some("bitcoinrpc".into()),
            auth: "basic".into(),
            username: Some("u".into()),
            password: Some("p".into()),
            proxy: Some(None),
            ..Default::default()
        };
        let p = backend_payload(&b);
        assert_eq!(p["config"]["username"], "u");
        assert_eq!(p["config"]["insecure"], false);
        let clear: Vec<&str> = p["clear"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            clear.contains(&"auth_header")
                && clear.contains(&"token")
                && clear.contains(&"certificate")
                && clear.contains(&"tor_proxy")
        );
        assert!(p["config"].get("certificate").is_none());
    }

    #[test]
    fn market_rows_follow_the_settings() {
        let settings =
            json!({"market_rate_provider": "coingecko", "source_classes": {"market_rates": true}});
        let rows = market_rate_backends(Some(&settings), &[]);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].on && !rows[0].on);
        assert_eq!(rows[2].url, "No HTTP mempool backend configured");
        assert!(!rows[2].on);
    }

    #[test]
    fn ai_url_rules() {
        assert!(is_loopback_http("http://127.0.0.1:8000/v1"));
        assert!(is_loopback_http("http://[::1]:8000"));
        assert!(!is_loopback_http("http://10.0.0.2:8000"));
        assert_eq!(
            auto_remote_reason("local", "https://api.openai.com/v1"),
            Some("url")
        );
        assert_eq!(
            auto_remote_reason("local", "claude-cli://default"),
            Some("cli")
        );
        assert_eq!(
            auto_remote_reason("remote", "https://api.openai.com/v1"),
            None
        );
        assert_eq!(
            http_origin("HTTPS://Api.OpenAI.com/v1").as_deref(),
            Some("https://api.openai.com")
        );
        assert_eq!(
            version_label(Some("2.1.287 (Claude Code)")).as_deref(),
            Some("v2.1.287")
        );
        assert_eq!(
            version_label(Some("GitHub Copilot CLI 1.0.93.")).as_deref(),
            Some("v1.0.93")
        );
    }

    #[test]
    fn formatters() {
        assert_eq!(format_bytes(512.0), "512 B");
        assert_eq!(format_bytes(2048.0), "2.0 KiB");
        assert_eq!(format_bytes(5.5 * 1024.0 * 1024.0), "5.5 MiB");
        assert_eq!(passphrase_issue("short", ""), Some("tooShort"));
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(uuid_v4().len(), 36);
    }
}
