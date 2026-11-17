//! Setup's data and rules, without a screen: the form (`types.ts` / `constants.ts`
//! `DEFAULT_FORM`), the step gates (`gates.ts`), the field hints, the network reach of a choice
//! (`networkReach.ts`), the identity the app keeps (`store/ui.ts` `Identity`), and the step
//! summaries (`index.tsx` `stepSummary`).

use rune::TextEdit;
use serde_json::{Map, Value, json};

use crate::i18n::{t, tr};
use crate::reports::books::model as reg;

// ---- constants.ts -------------------------------------------------------------------------------

pub const DEFAULT_BACKEND_NAME: &str = "fulcrum";
pub const DEFAULT_BACKEND_URL: &str = "ssl://index.bitcoin-austria.at:50002";
pub const DEFAULT_ELECTRUM_HOST: &str = "index.bitcoin-austria.at";
pub const DEFAULT_ELECTRUM_SSL_PORT: &str = "50002";
pub const DEFAULT_ELECTRUM_TCP_PORT: &str = "50001";
pub const DEFAULT_AI_PROVIDER_NAME: &str = "ollama";
pub const DEFAULT_AI_BASE_URL: &str = "http://localhost:11434/v1";
pub const MIN_DATABASE_PASSPHRASE_CHARS: usize = 12;
pub const DEFAULT_TOR_PROXY_HOST: &str = "127.0.0.1";
pub const DEFAULT_TOR_PROXY_PORT: &str = "9050";
pub const FIAT_CURRENCIES: [&str; 4] = ["EUR", "USD", "CHF", "GBP"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbMode {
    Sqlcipher,
    Plaintext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendMode {
    Default,
    Custom,
    Skip,
}

impl BackendMode {
    pub fn id(self) -> &'static str {
        match self {
            BackendMode::Default => "default",
            BackendMode::Custom => "custom",
            BackendMode::Skip => "skip",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    Esplora,
    Electrum,
    BitcoinRpc,
    LiquidEsplora,
}

impl BackendKind {
    pub const ALL: [BackendKind; 4] = [
        BackendKind::Esplora,
        BackendKind::Electrum,
        BackendKind::BitcoinRpc,
        BackendKind::LiquidEsplora,
    ];
    pub fn id(self) -> &'static str {
        match self {
            BackendKind::Esplora => "esplora",
            BackendKind::Electrum => "electrum",
            BackendKind::BitcoinRpc => "bitcoinrpc",
            BackendKind::LiquidEsplora => "liquid-esplora",
        }
    }
    pub fn from_id(id: &str) -> Option<BackendKind> {
        BackendKind::ALL.into_iter().find(|k| k.id() == id)
    }
    /// `BACKEND_KIND_LABELS` (English in the web, too).
    pub fn label(self) -> &'static str {
        kind_label(self.id())
    }
}

/// `BACKEND_KIND_LABELS[kind] ?? kind`.
pub fn kind_label(kind: &str) -> &str {
    match kind {
        "esplora" => "Explorer API",
        "electrum" => "Electrum / Fulcrum",
        "bitcoinrpc" => "Bitcoin Core RPC",
        "liquid-esplora" => "Liquid Explorer API",
        other => other,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiMode {
    Local,
    Remote,
    Disabled,
}

impl AiMode {
    pub fn id(self) -> &'static str {
        match self {
            AiMode::Local => "local",
            AiMode::Remote => "remote",
            AiMode::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiKind {
    Local,
    Remote,
    Tee,
}

impl AiKind {
    pub fn id(self) -> &'static str {
        match self {
            AiKind::Local => "local",
            AiKind::Remote => "remote",
            AiKind::Tee => "tee",
        }
    }
    pub fn from_id(id: &str) -> Option<AiKind> {
        [AiKind::Local, AiKind::Remote, AiKind::Tee]
            .into_iter()
            .find(|k| k.id() == id)
    }
    /// `AI_PROVIDER_KIND_LABELS`.
    pub fn label(self) -> &'static str {
        match self {
            AiKind::Local => "Local",
            AiKind::Remote => "Remote",
            AiKind::Tee => "TEE",
        }
    }
}

/// `OnboardingForm`; text fields keep their editing state.
#[derive(Clone)]
pub struct Form {
    pub workspace: TextEdit,
    /// The hidden default book label (no field in the web either).
    pub profile: String,
    pub tax_country: String,
    pub fiat: String,
    pub long_term_days: TextEdit,
    pub gains: String,
    pub db_mode: DbMode,
    pub passphrase: TextEdit,
    pub confirm: TextEdit,
    pub enable_touch_id: bool,
    pub recovery_ack: bool,
    pub plaintext_ack: bool,
    pub migrate_credentials: bool,
    pub update_checks: bool,
    /// Unset until the user picks (privacy by default); the express path sets it.
    pub backend_mode: Option<BackendMode>,
    pub include_liquid: bool,
    pub backend_kind: BackendKind,
    pub backend_name: TextEdit,
    pub backend_url: TextEdit,
    pub backend_host: TextEdit,
    pub backend_port: TextEdit,
    pub use_ssl: bool,
    pub trust_ssl: bool,
    pub certificate: TextEdit,
    pub use_proxy: bool,
    pub proxy_host: TextEdit,
    pub proxy_port: TextEdit,
    pub ai_mode: AiMode,
    pub ai_kind: AiKind,
    pub ai_name: TextEdit,
    pub ai_url: TextEdit,
    pub ai_remote_ack: bool,
}

impl Default for Form {
    /// `DEFAULT_FORM`.
    fn default() -> Form {
        Form {
            workspace: TextEdit::new("My Books"),
            profile: "Private".into(),
            tax_country: "at".into(),
            fiat: "EUR".into(),
            long_term_days: TextEdit::new("365"),
            gains: "MOVING_AVERAGE_AT".into(),
            db_mode: DbMode::Sqlcipher,
            passphrase: TextEdit::new(""),
            confirm: TextEdit::new(""),
            enable_touch_id: false,
            recovery_ack: false,
            plaintext_ack: false,
            migrate_credentials: true,
            update_checks: false,
            backend_mode: None,
            include_liquid: false,
            backend_kind: BackendKind::Electrum,
            backend_name: TextEdit::new(DEFAULT_BACKEND_NAME),
            backend_url: TextEdit::new(DEFAULT_BACKEND_URL),
            backend_host: TextEdit::new(DEFAULT_ELECTRUM_HOST),
            backend_port: TextEdit::new(DEFAULT_ELECTRUM_SSL_PORT),
            use_ssl: true,
            trust_ssl: false,
            certificate: TextEdit::new(""),
            use_proxy: false,
            proxy_host: TextEdit::new(""),
            proxy_port: TextEdit::new(""),
            ai_mode: AiMode::Local,
            ai_kind: AiKind::Local,
            ai_name: TextEdit::new(DEFAULT_AI_PROVIDER_NAME),
            ai_url: TextEdit::new(DEFAULT_AI_BASE_URL),
            ai_remote_ack: false,
        }
    }
}

/// Replaces a field's text, keeping the field's focus.
pub fn set_text(e: &mut TextEdit, s: &str) {
    let focused = e.focused;
    *e = TextEdit::new(s);
    e.focused = focused;
}

// ---- Tax country registry bits the web reads in setup ------------------------------------------

/// `requiredFiatCurrency`: the currency a country fixes (generic chooses its own).
pub fn required_fiat(country: &str) -> Option<&'static str> {
    match country {
        "at" | "de" | "ie" | "es" => Some("EUR"),
        "us" => Some("USD"),
        "ch" => Some("CHF"),
        "gb" => Some("GBP"),
        "jp" => Some("JPY"),
        _ => None,
    }
}

/// `usesLongTermDays`: only the generic country takes the book's day count.
pub fn uses_long_term_days(country: &str) -> bool {
    country == "generic"
}

/// The registry's ISO code for the legacy `Identity.country` field.
pub fn iso(country: &str) -> Option<&'static str> {
    match country {
        "at" => Some("AT"),
        "us" => Some("US"),
        "de" => Some("DE"),
        "ch" => Some("CH"),
        "ie" => Some("IE"),
        "gb" => Some("GB"),
        "es" => Some("ES"),
        "jp" => Some("JP"),
        _ => None,
    }
}

/// `onboardingRegionLabel`.
pub fn region_label(country: &str) -> String {
    match country {
        "at" => t("onboarding:essentials.jurisdictionAustria"),
        "generic" => t("onboarding:essentials.jurisdictionGeneric"),
        _ => reg::region_label(country),
    }
}

/// `t("books.method.<token>")` (setup keys every token, UK share matching too).
pub fn method_label(method: &str) -> String {
    t(&format!("onboarding:books.method.{method}"))
}

// ---- URL shapes (the WHATWG parser's answers the checks rely on) --------------------------------

#[derive(Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub credentials: bool,
    pub host: String,
    pub port: String,
}

/// `new URL(raw)` reduced to what setup asks of it: the scheme, whether a user name or password
/// is present, the host and the port; None where the parser throws.
pub fn parse_url(raw: &str) -> Option<Url> {
    let raw = raw.trim();
    let colon = raw.find(':')?;
    let scheme = raw[..colon].to_ascii_lowercase();
    let mut chars = scheme.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    let special = matches!(
        scheme.as_str(),
        "http" | "https" | "ws" | "wss" | "ftp" | "file"
    );
    let rest = &raw[colon + 1..];
    let after = if let Some(r) = rest.strip_prefix("//") {
        r
    } else if special {
        rest.trim_start_matches(['/', '\\'])
    } else {
        // An opaque path: no host.
        return Some(Url {
            scheme,
            credentials: false,
            host: String::new(),
            port: String::new(),
        });
    };
    let end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let authority = &after[..end];
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let credentials = userinfo.is_some_and(|u| !u.is_empty() && u != ":");
    let (host, port) = if let Some(r) = hostport.strip_prefix('[') {
        let close = r.find(']')?;
        let host = format!("[{}]", &r[..close]);
        let tail = &r[close + 1..];
        let port = match tail.strip_prefix(':') {
            Some(p) => p,
            None if tail.is_empty() => "",
            None => return None,
        };
        (host, port)
    } else {
        match hostport.rfind(':') {
            Some(i) => (hostport[..i].to_owned(), &hostport[i + 1..]),
            None => (hostport.to_owned(), ""),
        }
    };
    if !port.chars().all(|c| c.is_ascii_digit())
        || (!port.is_empty() && port.parse::<u32>().ok()? > 65535)
    {
        return None;
    }
    if host
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '^' | '|' | '%' | '\\'))
    {
        return None;
    }
    if special && host.is_empty() {
        return None;
    }
    let default = matches!(
        (scheme.as_str(), port),
        ("http" | "ws", "80") | ("https" | "wss", "443")
    );
    let port = if default || port.is_empty() {
        String::new()
    } else {
        port.parse::<u32>().ok()?.to_string()
    };
    Some(Url {
        scheme,
        credentials,
        host: host.to_ascii_lowercase(),
        port,
    })
}

fn has_http_url(raw: &str) -> bool {
    parse_url(raw).is_some_and(|u| u.scheme == "http" || u.scheme == "https")
}

fn has_inline_credential(raw: &str) -> bool {
    let candidate = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("ssl://{raw}")
    };
    parse_url(&candidate).is_some_and(|u| u.credentials)
}

fn has_socket_endpoint(raw: &str) -> bool {
    let candidate = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("ssl://{raw}")
    };
    parse_url(&candidate).is_some_and(|u| {
        (u.scheme == "ssl" || u.scheme == "tcp") && !u.host.is_empty() && !u.port.is_empty()
    })
}

/// `electrumEndpointUrl`.
pub fn electrum_url(host: &str, port: &str, ssl: bool) -> String {
    let (host, port) = (host.trim(), port.trim());
    if host.is_empty() || port.is_empty() {
        return String::new();
    }
    format!("{}://{host}:{port}", if ssl { "ssl" } else { "tcp" })
}

/// `backendEndpointDescription` (English in the web).
pub fn endpoint_description(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Electrum => "Use ssl://host:50002, tcp://host:50001, or host:port.",
        BackendKind::BitcoinRpc => {
            "Use the RPC URL only; add cookies or passwords after onboarding."
        }
        _ => "Use an Esplora-compatible http:// or https:// endpoint.",
    }
}

/// `backendEndpointHint` (English in the web).
pub fn endpoint_hint(kind: BackendKind, raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some("Endpoint is required.");
    }
    if has_inline_credential(trimmed) {
        return Some("Do not include usernames or passwords in the endpoint.");
    }
    if kind == BackendKind::Electrum {
        return (!has_socket_endpoint(trimmed))
            .then_some("Use ssl://host:50002, tcp://host:50001, or host:port.");
    }
    (!has_http_url(trimmed)).then_some("Use an http:// or https:// URL.")
}

/// `isNativeAiProviderLocator`.
pub fn is_native_locator(raw: &str) -> bool {
    matches!(
        raw.trim().to_lowercase().as_str(),
        "claude-cli://default"
            | "codex-cli://default"
            | "opencode-cli://default"
            | "copilot-cli://default"
    )
}

/// `aiBaseUrlHint`.
pub fn ai_url_hint(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some("Base URL is required.");
    }
    if has_inline_credential(trimmed) {
        return Some("Do not include usernames or passwords in the endpoint.");
    }
    (!(has_http_url(trimmed) || is_native_locator(trimmed)))
        .then_some("Use an http:// or https:// URL, or a supported native CLI locator.")
}

/// `localAiBaseUrlHint`.
pub fn local_ai_url_hint(raw: &str) -> Option<&'static str> {
    ai_url_hint(raw).or_else(|| {
        (!is_loopback_http(raw)).then_some("Local AI providers must use http://localhost, http://127.0.0.1, or http://[::1]. Use remote mode for off-device endpoints.")
    })
}

/// `isLoopbackAiHttpUrl`: the host as written (no shorthand IPv4), localhost, 127/8, ::1 or
/// IPv4-mapped 127/8.
pub fn is_loopback_http(raw: &str) -> bool {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    let rest = if lower.starts_with("http://") {
        &s[7..]
    } else if lower.starts_with("https://") {
        &s[8..]
    } else {
        return false;
    };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let hostinfo = &authority[authority.rfind('@').map_or(0, |i| i + 1)..];
    let host = if let Some(r) = hostinfo.strip_prefix('[') {
        match r.find(']') {
            Some(end) => &r[..end],
            None => return false,
        }
    } else {
        hostinfo.split(':').next().unwrap_or("")
    }
    .to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
    {
        if parts
            .iter()
            .any(|p| (p.len() > 1 && p.starts_with('0')) || p.parse::<u32>().unwrap_or(256) > 255)
        {
            return false;
        }
        return parts[0] == "127";
    }
    if !host.contains(':') {
        return false;
    }
    match host.parse::<std::net::Ipv6Addr>() {
        Ok(ip) => ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.octets()[0] == 127),
        Err(_) => false,
    }
}

/// `isOnionEndpoint` (backendTrust's `endpointHost` defaults to https for a bare host).
pub fn is_onion(url: &str) -> bool {
    let raw = url.trim();
    if raw.is_empty() {
        return false;
    }
    let candidate = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let host = match parse_url(&candidate) {
        Some(u) => u
            .host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned(),
        None => raw.to_lowercase(),
    };
    host.ends_with(".onion")
}

/// `normalizeOnionHttpEndpoint`.
pub fn normalize_onion_http(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains("://") || !is_onion(trimmed) {
        return value.to_owned();
    }
    format!("http://{trimmed}")
}

// ---- networkReach.ts ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    Internet,
    Device,
    None,
}

impl Reach {
    pub fn id(self) -> &'static str {
        match self {
            Reach::Internet => "internet",
            Reach::Device => "device",
            Reach::None => "none",
        }
    }
}

/// `endpointHost`: the host and a non-default port of any scheme, or the input.
pub fn endpoint_host(url: &str) -> String {
    let trimmed = url.trim();
    match parse_url(trimmed) {
        Some(u) if !u.host.is_empty() => {
            if u.port.is_empty() {
                u.host
            } else {
                format!("{}:{}", u.host, u.port)
            }
        }
        _ => trimmed.to_owned(),
    }
}

/// `endpointReach`.
pub fn endpoint_reach(url: &str) -> Reach {
    let trimmed = url.trim();
    let as_http = match trimmed.find("://") {
        Some(i)
            if trimmed[..i]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && trimmed[..i]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
        {
            format!("http://{}", &trimmed[i + 3..])
        }
        _ => format!("http://{trimmed}"),
    };
    if is_loopback_http(&as_http) {
        Reach::Device
    } else {
        Reach::Internet
    }
}

/// `NetworkSummary`.
#[derive(Clone, Debug)]
pub struct Network {
    pub reach: Reach,
    pub label: String,
    /// (host, label)
    pub hosts: Vec<(String, Option<String>)>,
    pub pending: Option<String>,
}

pub fn no_connections() -> Network {
    Network {
        reach: Reach::None,
        label: t("onboarding:network.none"),
        hosts: vec![],
        pending: None,
    }
}

// ---- constants.ts validation ------------------------------------------------------------------

/// `parseTaxLongTermDays`.
pub fn parse_long_term_days(raw: &str) -> Option<u32> {
    let s = raw.trim();
    if s.is_empty() || s.starts_with('0') || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse::<u64>()
        .ok()
        .filter(|d| *d <= 9_007_199_254_740_991)
        .map(|d| d.min(u32::MAX as u64) as u32)
}

/// `taxLongTermDaysHint` (English in the web).
pub fn long_term_days_hint(raw: &str) -> Option<&'static str> {
    let s = raw.trim();
    if s.is_empty() {
        return Some("Required.");
    }
    if parse_long_term_days(s).is_some() {
        return None;
    }
    let int = s.strip_prefix('-').unwrap_or(s);
    if !int.is_empty()
        && int.chars().all(|c| c.is_ascii_digit())
        && s.parse::<i64>().is_ok_and(|n| n < 1)
    {
        return Some("Must be at least 1 day.");
    }
    Some("Use a whole number of days.")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassIssue {
    Required,
    TooShort,
    ConfirmRequired,
    Mismatch,
}

impl PassIssue {
    pub fn hint(self) -> String {
        let key = match self {
            PassIssue::Required => "required",
            PassIssue::TooShort => "tooShort",
            PassIssue::ConfirmRequired => "confirmRequired",
            PassIssue::Mismatch => "mismatch",
        };
        tr(
            &format!("onboarding:security.hint.{key}"),
            &[("min", &MIN_DATABASE_PASSPHRASE_CHARS.to_string())],
        )
    }
}

/// `databasePassphraseIssue` (UTF-16 length, as `.length`).
pub fn passphrase_issue(passphrase: &str, confirmation: &str) -> Option<PassIssue> {
    if passphrase.is_empty() {
        return Some(PassIssue::Required);
    }
    if passphrase.encode_utf16().count() < MIN_DATABASE_PASSPHRASE_CHARS {
        return Some(PassIssue::TooShort);
    }
    if confirmation.is_empty() {
        return Some(PassIssue::ConfirmRequired);
    }
    (passphrase != confirmation).then_some(PassIssue::Mismatch)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Weak,
    Fair,
    Strong,
}

/// `databasePassphraseStrength` (code points, as `[...s].length`).
pub fn passphrase_strength(p: &str) -> Option<Strength> {
    let len = p.chars().count();
    if len < MIN_DATABASE_PASSPHRASE_CHARS {
        return None;
    }
    let classes = [
        p.chars().any(|c| c.is_ascii_lowercase()),
        p.chars().any(|c| c.is_ascii_uppercase()),
        p.chars().any(|c| c.is_ascii_digit()),
        p.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    Some(if len >= 20 || (len >= 16 && classes >= 3) {
        Strength::Strong
    } else if len >= 14 || classes >= 3 {
        Strength::Fair
    } else {
        Strength::Weak
    })
}

// ---- gates.ts -----------------------------------------------------------------------------------

impl Form {
    /// The custom endpoint as the backend will store it.
    pub fn custom_endpoint(&self) -> String {
        if self.backend_kind == BackendKind::Electrum {
            electrum_url(
                &self.backend_host.text,
                &self.backend_port.text,
                self.use_ssl,
            )
        } else {
            self.backend_url.text.trim().to_owned()
        }
    }

    /// `effectiveBackendUrl` (the URL the hint checks, untrimmed for non-Electrum).
    pub fn effective_backend_url(&self) -> String {
        if self.backend_kind == BackendKind::Electrum {
            electrum_url(
                &self.backend_host.text,
                &self.backend_port.text,
                self.use_ssl,
            )
        } else {
            self.backend_url.text.clone()
        }
    }

    pub fn identity_complete(&self) -> bool {
        !self.workspace.text.trim().is_empty() && !self.profile.trim().is_empty()
    }

    pub fn accounting_complete(&self) -> bool {
        !uses_long_term_days(&self.tax_country)
            || parse_long_term_days(&self.long_term_days.text).is_some()
    }

    pub fn connections_complete(&self) -> bool {
        match self.backend_mode {
            None => false,
            Some(BackendMode::Skip | BackendMode::Default) => true,
            Some(BackendMode::Custom) => {
                !self.backend_name.text.trim().is_empty()
                    && endpoint_hint(self.backend_kind, &self.effective_backend_url()).is_none()
            }
        }
    }

    pub fn ai_complete(&self) -> bool {
        if self.backend_mode == Some(BackendMode::Skip) {
            return true;
        }
        match self.ai_mode {
            AiMode::Local => {
                if local_ai_url_hint(&self.ai_url.text).is_some() {
                    return false;
                }
            }
            AiMode::Remote => {
                if ai_url_hint(&self.ai_url.text).is_some() {
                    return false;
                }
            }
            AiMode::Disabled => {}
        }
        if self.ai_mode == AiMode::Remote {
            return !self.ai_name.text.trim().is_empty() && self.ai_remote_ack;
        }
        true
    }

    pub fn essentials_complete(&self) -> bool {
        self.identity_complete() && self.accounting_complete()
    }

    pub fn security_complete(&self) -> bool {
        match self.db_mode {
            DbMode::Plaintext => self.plaintext_ack,
            DbMode::Sqlcipher => {
                self.recovery_ack
                    && passphrase_issue(&self.passphrase.text, &self.confirm.text).is_none()
            }
        }
    }

    pub fn review_complete(&self) -> bool {
        self.essentials_complete()
            && self.connections_complete()
            && self.ai_complete()
            && self.security_complete()
    }

    /// The step at `index`'s gate (`DEFAULT_STEPS[i].isComplete`).
    pub fn step_complete(&self, index: usize) -> bool {
        match index {
            0 => self.essentials_complete(),
            1 => self.connections_complete(),
            2 => self.ai_complete(),
            3 => self.security_complete(),
            _ => self.review_complete(),
        }
    }

    /// The proxy `host:port` a custom backend routes through, when both are set.
    pub fn backend_proxy(&self) -> Option<String> {
        let (h, p) = (self.proxy_host.text.trim(), self.proxy_port.text.trim());
        (self.backend_mode == Some(BackendMode::Custom)
            && self.use_proxy
            && !h.is_empty()
            && !p.is_empty())
        .then(|| format!("{h}:{p}"))
    }

    /// `ui.onboarding.complete`'s args (`finish`).
    pub fn complete_args(&self, mode: BackendMode, long_term_days: u32, gains: &str) -> Value {
        let mut args = json!({
            "workspace_label": or(self.workspace.text.trim(), "My Books"),
            "profile_label": or(self.profile.trim(), "Private"),
            "tax_country": self.tax_country,
            "fiat_currency": self.fiat,
            "tax_long_term_days": long_term_days,
            "gains_algorithm": gains,
            "backend_setup_mode": mode.id(),
        });
        if mode == BackendMode::Default {
            args["include_liquid_backends"] = json!(self.include_liquid);
        }
        if mode == BackendMode::Custom {
            let liquid = self.backend_kind == BackendKind::LiquidEsplora;
            let mut b = json!({
                "name": or(self.backend_name.text.trim(), "custom"),
                "kind": self.backend_kind.id(),
                "url": self.custom_endpoint(),
                "chain": if liquid { "liquid" } else { "bitcoin" },
                "network": if liquid { "liquidv1" } else { "main" },
            });
            if self.backend_kind == BackendKind::Electrum
                && self.use_ssl
                && !self.trust_ssl
                && !self.certificate.text.trim().is_empty()
            {
                b["certificate"] = json!(self.certificate.text.trim());
            }
            if let Some(p) = self.backend_proxy() {
                b["tor_proxy"] = json!(p);
            }
            args["backend"] = b;
        }
        args
    }
}

pub fn or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    if s.is_empty() { fallback } else { s }
}

/// `GAINS_ALGORITHM_DEFAULTS[country]`.
pub fn default_gains(country: &str) -> String {
    reg::default_method(country)
}

/// `gainsAlgorithmsFor`.
pub fn gains_for(country: &str) -> Vec<String> {
    reg::methods_for(country)
}

// ---- Step summaries (`stepSummary`) -------------------------------------------------------------

pub fn step_summary(step: usize, f: &Form) -> Option<String> {
    let offline = f.backend_mode == Some(BackendMode::Skip);
    match step {
        0 => Some(
            [
                or(f.workspace.text.trim(), "My Books").to_owned(),
                region_label(&f.tax_country),
                f.fiat.clone(),
            ]
            .join(" · "),
        ),
        1 => match f.backend_mode? {
            BackendMode::Skip => Some(t("onboarding:stepper.summary.offline")),
            BackendMode::Default => Some(t("onboarding:stepper.summary.publicBackends")),
            BackendMode::Custom => Some(match f.backend_name.text.trim() {
                "" => t("onboarding:review.sync.customFallbackName"),
                n => n.to_owned(),
            }),
        },
        2 => Some(if offline || f.ai_mode == AiMode::Disabled {
            t("onboarding:stepper.summary.aiOff")
        } else {
            let name = match f.ai_name.text.trim() {
                "" => f.ai_kind.id().to_owned(),
                n => n.to_owned(),
            };
            tr(
                if f.ai_mode == AiMode::Local {
                    "onboarding:stepper.summary.aiLocal"
                } else {
                    "onboarding:stepper.summary.aiRemote"
                },
                &[("name", &name)],
            )
        }),
        3 => Some(t(if f.db_mode == DbMode::Sqlcipher {
            "onboarding:stepper.summary.encrypted"
        } else {
            "onboarding:stepper.summary.plaintext"
        })),
        _ => None,
    }
}

// ---- Identity (`store/ui.ts`), without paths ----------------------------------------------------

/// The persisted `identity`: who the app is set up for. The web's `importedProject` holds the
/// book's paths; here it is the book key of its data root (`persist::model::book_key`), and the
/// path itself lives in the approved-projects file. Fields setup records but nothing in this
/// app reads (backend URLs, certificate paths, AI endpoints) are not kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    pub name: String,
    pub workspace: String,
    pub profile: String,
    pub country: String,
    pub encrypted: bool,
    pub tax_country: Option<String>,
    pub fiat_currency: Option<String>,
    pub tax_long_term_days: Option<u32>,
    pub gains_algorithm: Option<String>,
    pub database_mode: Option<String>,
    pub backend_setup_mode: Option<String>,
    pub ai_setup_mode: Option<String>,
    /// `importedProject`: `db#<hash>` of its data root.
    pub imported: Option<String>,
}

const MAX_LABEL: usize = 200;

impl Identity {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("name".into(), json!(self.name));
        m.insert("workspace".into(), json!(self.workspace));
        m.insert("profile".into(), json!(self.profile));
        m.insert("country".into(), json!(self.country));
        m.insert("encrypted".into(), json!(self.encrypted));
        let mut opt = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                m.insert(k.into(), v);
            }
        };
        opt("taxCountry", self.tax_country.clone().map(Value::from));
        opt("fiatCurrency", self.fiat_currency.clone().map(Value::from));
        opt("taxLongTermDays", self.tax_long_term_days.map(Value::from));
        opt(
            "gainsAlgorithm",
            self.gains_algorithm.clone().map(Value::from),
        );
        opt("databaseMode", self.database_mode.clone().map(Value::from));
        opt(
            "backendSetupMode",
            self.backend_setup_mode.clone().map(Value::from),
        );
        opt("aiSetupMode", self.ai_setup_mode.clone().map(Value::from));
        opt(
            "importedProject",
            self.imported.clone().map(|k| json!({"bookKey": k})),
        );
        Value::Object(m)
    }

    /// Tolerant: an identity needs a name and a workspace; a field that fails its check is left
    /// out, and an `importedProject` without a valid book key (a web file's paths) drops the
    /// whole identity, so setup asks again rather than opening the wrong book.
    pub fn from_json(v: &Value) -> Option<Identity> {
        let o = v.as_object()?;
        let s = |k: &str| {
            o.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= MAX_LABEL)
                .map(str::to_owned)
        };
        let token = |k: &str| {
            s(k).filter(|v| {
                v.len() <= 40
                    && v.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            })
        };
        let name = s("name")?;
        let workspace = s("workspace")?;
        let imported = match o.get("importedProject") {
            None | Some(Value::Null) => None,
            Some(p) => Some(
                p.get("bookKey")
                    .and_then(Value::as_str)
                    .filter(|k| crate::persist::model::is_book_key(k))?
                    .to_owned(),
            ),
        };
        Some(Identity {
            profile: s("profile").unwrap_or_else(|| name.clone()),
            name,
            workspace,
            country: s("country").unwrap_or_else(|| "Generic".into()),
            encrypted: o.get("encrypted").and_then(Value::as_bool).unwrap_or(false),
            tax_country: token("taxCountry"),
            fiat_currency: token("fiatCurrency"),
            tax_long_term_days: o
                .get("taxLongTermDays")
                .and_then(Value::as_u64)
                .map(|d| d.min(u32::MAX as u64) as u32),
            gains_algorithm: token("gainsAlgorithm"),
            database_mode: token("databaseMode"),
            backend_setup_mode: token("backendSetupMode"),
            ai_setup_mode: token("aiSetupMode"),
            imported,
        })
    }

    /// `encryptedWorkspace`.
    pub fn is_encrypted(&self) -> bool {
        self.encrypted || self.database_mode.as_deref() == Some("sqlcipher")
    }
}

/// `bookIdentityKey`: the imported book's key, else `id:<workspace>/<profile>`.
pub fn book_identity_key(identity: Option<&Identity>) -> Option<String> {
    let i = identity?;
    if let Some(k) = &i.imported {
        return Some(k.clone());
    }
    let profile = if i.profile.trim().is_empty() {
        i.name.trim()
    } else {
        i.profile.trim()
    };
    let parts: Vec<&str> = [i.workspace.trim(), profile]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    (!parts.is_empty()).then(|| format!("id:{}", parts.join("/")))
}

/// `GAINS_ALGORITHMS` (ImportProjectPanel's accepted tokens).
pub const GAINS_ALGORITHMS: [&str; 10] = [
    "FIFO",
    "LIFO",
    "HIFO",
    "LOFO",
    "MOVING_AVERAGE",
    "MOVING_AVERAGE_AT",
    "MOVING_AVERAGE_FIFO_PERIOD",
    "UK_SHARE_MATCHING",
    "JP_TOTAL_AVERAGE",
    "JP_MOVING_AVERAGE",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form::default()
    }

    #[test]
    fn url_checks_follow_the_web() {
        assert_eq!(
            endpoint_hint(BackendKind::Electrum, ""),
            Some("Endpoint is required.")
        );
        assert_eq!(
            endpoint_hint(BackendKind::Electrum, "ssl://host:50002"),
            None
        );
        assert_eq!(endpoint_hint(BackendKind::Electrum, "host:50001"), None);
        assert!(endpoint_hint(BackendKind::Electrum, "ssl://host").is_some());
        assert_eq!(
            endpoint_hint(BackendKind::Electrum, "ssl://u:p@host:1"),
            Some("Do not include usernames or passwords in the endpoint.")
        );
        assert_eq!(
            endpoint_hint(BackendKind::Esplora, "https://mempool.example/api"),
            None
        );
        assert_eq!(
            endpoint_hint(BackendKind::Esplora, "ssl://x:1"),
            Some("Use an http:// or https:// URL.")
        );
        assert_eq!(
            endpoint_hint(BackendKind::Esplora, "mempool.example"),
            Some("Use an http:// or https:// URL.")
        );
        assert_eq!(electrum_url(" h ", "1", false), "tcp://h:1");
        assert_eq!(electrum_url("", "1", true), "");
    }

    #[test]
    fn loopback_rules_match_the_daemon() {
        assert!(is_loopback_http("http://localhost:11434/v1"));
        assert!(is_loopback_http("http://127.0.0.1:1"));
        assert!(is_loopback_http("http://[::1]:8080"));
        assert!(is_loopback_http("http://[::ffff:127.0.0.1]/"));
        assert!(!is_loopback_http("http://127.1/"));
        assert!(!is_loopback_http("http://0127.0.0.1/"));
        assert!(!is_loopback_http("http://example.com"));
        assert!(!is_loopback_http("ftp://localhost"));
        assert_eq!(local_ai_url_hint(DEFAULT_AI_BASE_URL), None);
        assert!(local_ai_url_hint("https://api.example/v1").is_some());
        assert_eq!(ai_url_hint("claude-cli://default"), None);
        assert_eq!(endpoint_reach("ssl://127.0.0.1:50002"), Reach::Device);
        assert_eq!(
            endpoint_reach("index.bitcoin-austria.at:50002"),
            Reach::Internet
        );
        assert_eq!(
            endpoint_host("ssl://index.bitcoin-austria.at:50002"),
            "index.bitcoin-austria.at:50002"
        );
        assert_eq!(endpoint_host("https://mempool.space/api"), "mempool.space");
        assert!(is_onion("abc.onion:50001"));
        assert_eq!(
            normalize_onion_http("abc.onion/api"),
            "http://abc.onion/api"
        );
    }

    #[test]
    fn gates_follow_gates_ts() {
        let mut f = form();
        assert!(f.essentials_complete());
        assert!(!f.connections_complete(), "no sync choice yet");
        f.backend_mode = Some(BackendMode::Default);
        assert!(f.connections_complete());
        assert!(f.ai_complete());
        assert!(!f.security_complete());
        f.passphrase = TextEdit::new("correct horse battery");
        f.confirm = TextEdit::new("correct horse battery");
        assert!(
            !f.security_complete(),
            "the recovery acknowledgement is required"
        );
        f.recovery_ack = true;
        assert!(f.security_complete() && f.review_complete());
        f.db_mode = DbMode::Plaintext;
        assert!(!f.security_complete());
        f.plaintext_ack = true;
        assert!(f.security_complete());
        f.tax_country = "generic".into();
        f.long_term_days = TextEdit::new("0");
        assert!(!f.essentials_complete());
        assert_eq!(long_term_days_hint("0"), Some("Must be at least 1 day."));
        assert_eq!(
            long_term_days_hint("1.5"),
            Some("Use a whole number of days.")
        );
        assert_eq!(long_term_days_hint(""), Some("Required."));
        f.backend_mode = Some(BackendMode::Custom);
        f.backend_name = TextEdit::new("home");
        f.backend_host = TextEdit::new("");
        assert!(!f.connections_complete());
        f.backend_host = TextEdit::new("node.local");
        assert!(f.connections_complete());
        f.ai_mode = AiMode::Remote;
        f.ai_url = TextEdit::new("https://api.example/v1");
        f.ai_name = TextEdit::new("x");
        assert!(!f.ai_complete());
        f.ai_remote_ack = true;
        assert!(f.ai_complete());
        f.backend_mode = Some(BackendMode::Skip);
        f.ai_url = TextEdit::new("");
        assert!(f.ai_complete(), "offline ignores a half-filled AI answer");
    }

    #[test]
    fn passphrase_rules() {
        assert_eq!(passphrase_issue("", ""), Some(PassIssue::Required));
        assert_eq!(passphrase_issue("short", ""), Some(PassIssue::TooShort));
        assert_eq!(
            passphrase_issue("twelve chars", ""),
            Some(PassIssue::ConfirmRequired)
        );
        assert_eq!(
            passphrase_issue("twelve chars", "twelve charz"),
            Some(PassIssue::Mismatch)
        );
        assert_eq!(passphrase_issue("twelve chars", "twelve chars"), None);
        assert_eq!(passphrase_strength("aaaaaaaaaaa"), None);
        assert_eq!(passphrase_strength("aaaaaaaaaaaa"), Some(Strength::Weak));
        assert_eq!(passphrase_strength("aaaaaaaaaaaaaa"), Some(Strength::Fair));
        assert_eq!(
            passphrase_strength("Aa1aaaaaaaaaaaaa"),
            Some(Strength::Strong)
        );
        assert_eq!(
            passphrase_strength("aaaaaaaaaaaaaaaaaaaa"),
            Some(Strength::Strong)
        );
    }

    #[test]
    fn complete_args_carry_the_choices() {
        let mut f = form();
        f.backend_mode = Some(BackendMode::Custom);
        f.backend_name = TextEdit::new(" home ");
        f.backend_host = TextEdit::new("node.example");
        f.use_proxy = true;
        f.proxy_host = TextEdit::new("127.0.0.1");
        f.proxy_port = TextEdit::new("9050");
        let a = f.complete_args(BackendMode::Custom, 0, "MOVING_AVERAGE_AT");
        assert_eq!(a["backend"]["url"], "ssl://node.example:50002");
        assert_eq!(a["backend"]["name"], "home");
        assert_eq!(a["backend"]["tor_proxy"], "127.0.0.1:9050");
        assert_eq!(a["backend"]["network"], "main");
        assert!(a.get("include_liquid_backends").is_none());
        let d = f.complete_args(BackendMode::Default, 0, "FIFO");
        assert_eq!(d["include_liquid_backends"], false);
        assert!(d.get("backend").is_none());
    }

    #[test]
    fn identity_keeps_no_paths() {
        let id = Identity {
            name: "Private".into(),
            workspace: "My Books".into(),
            profile: "Private".into(),
            country: "AT".into(),
            imported: Some(crate::persist::model::book_key(Some(std::path::Path::new(
                "/tmp/x/data",
            )))),
            ..Default::default()
        };
        let v = id.to_json();
        assert!(!v.to_string().contains("/tmp"));
        assert_eq!(Identity::from_json(&v), Some(id.clone()));
        // A web identity with paths is not taken.
        assert_eq!(
            Identity::from_json(
                &json!({"name": "a", "workspace": "b", "importedProject": {"dataRoot": "/x"}})
            ),
            None
        );
        assert_eq!(
            book_identity_key(Some(&Identity {
                imported: None,
                ..id.clone()
            })),
            Some("id:My Books/Private".into())
        );
        assert!(book_identity_key(Some(&id)).unwrap().starts_with("db#"));
    }
}
