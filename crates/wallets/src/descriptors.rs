//! Python-compatible descriptor plans over mature Bitcoin/Elements parsers.
//! Private spending keys are rejected with constant, secret-free errors.
use bitcoin::{bip32, Network, PrivateKey};
use elements_miniscript::{self as em, elements};
use kassiber_envelope::{dict, s, AppError};
use miniscript::descriptor::DescriptorSecretKey;
use miniscript::{Descriptor, DescriptorPublicKey, ForEachKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::str::FromStr;

mod view;
use view::ViewKey;

pub const SCRIPT_TYPES: [&str; 4] = ["p2pkh", "p2sh-p2wpkh", "p2wpkh", "p2tr"];

/// Retains the distinction between Python parser ValueErrors and typed refusals.
/// Deliberately has no Debug implementation: parser inputs can include view keys.
pub enum Error {
    Value(String),
    App(AppError),
}
impl Error {
    pub fn into_app(self) -> AppError {
        match self {
            Self::Value(message) => AppError::new("app_error", message),
            Self::App(error) => error,
        }
    }
    pub fn projection(&self) -> Value {
        match self {
            Self::Value(message) => json!({"exception":{"type":"ValueError","message":message}}),
            Self::App(e) => json!({"error":{"code":e.code,"message":e.message,"hint":e.hint,
                "details": e.details.as_ref().map(|d|serde_json::from_str::<Value>(&kassiber_pyjson::dumps(d, &Default::default()).expect("error JSON")).expect("error JSON")), "retryable":e.retryable}}),
        }
    }
}
impl From<AppError> for Error {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

pub fn spending_error() -> AppError {
    AppError::new("wallet_spending_private_material", "Spending-private wallet material is not accepted")
        .with_hint("Export a watch-only descriptor or extended public key. Liquid blinding/view keys may remain private because they cannot spend.")
}

pub fn text(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(t) => t.clone(),
        _ => value.to_string(),
    }
}
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::Number(v) => v.as_f64() != Some(0.0),
    }
}
pub fn field(config: &Value, key: &str) -> String {
    config
        .get(key)
        .filter(|v| truthy(v))
        .map(text)
        .unwrap_or_default()
}

pub fn normalize_chain(value: &str) -> Result<String> {
    Ok(
        match kassiber_pytext::strip(value).to_lowercase().as_str() {
            "" | "bitcoin" | "btc" | "onchain" => "bitcoin",
            "liquid" | "lbtc" | "elements" => "liquid",
            _ => return Err(Error::Value(format!("Unsupported wallet chain '{value}'"))),
        }
        .into(),
    )
}
pub fn normalize_network(chain: &str, value: &str) -> Result<String> {
    let chain = normalize_chain(chain)?;
    let normalized = kassiber_pytext::strip(value).to_lowercase();
    let network = if chain == "bitcoin" {
        match normalized.as_str() {
            "" | "main" | "mainnet" | "bitcoin" => "main",
            "test" | "testnet" => "test",
            "signet" => "signet",
            "regtest" => "regtest",
            _ => {
                return Err(Error::Value(format!(
                    "Unsupported bitcoin network '{value}'"
                )))
            }
        }
    } else {
        match normalized.as_str() {
            "" | "liquid" | "liquidv1" | "main" | "mainnet" => "liquidv1",
            "test" | "testnet" | "liquidtestnet" => "liquidtestnet",
            "elements" | "elementsregtest" | "regtest" => "elementsregtest",
            _ => {
                return Err(Error::Value(format!(
                    "Unsupported liquid network '{value}'"
                )))
            }
        }
    };
    Ok(network.into())
}

pub fn normalize_descriptor_text(chain: &str, input: &str) -> Result<String> {
    let compact: String = input
        .chars()
        .filter(|c| !kassiber_pytext::is_space(*c as u32))
        .collect();
    let compact = compact.split('#').next().unwrap_or_default();
    if compact.is_empty() {
        return Ok(String::new());
    }
    if normalize_chain(chain)? != "liquid" {
        return Ok(compact.into());
    }
    let mut result = compact.to_owned();
    if let Some(rest) = result.strip_prefix("ct(") {
        result = format!("blinded({rest}");
    }
    for (a, b) in [
        ("elwpkh(", "wpkh("),
        ("elwsh(", "wsh("),
        ("elsh(", "sh("),
        ("eltr(", "tr("),
    ] {
        result = result.replace(a, b);
    }
    let pattern = regex::Regex::new(r"slip77\(([0-9a-fA-F]{64})\)").expect("constant regex");
    let mut failure = false;
    let normalized = pattern.replace_all(&result, |m: &regex::Captures<'_>| {
        match bitcoin::hex::FromHex::from_hex(&m[1])
            .ok()
            .and_then(|b: Vec<u8>| bitcoin::secp256k1::SecretKey::from_slice(&b).ok())
        {
            Some(key) => format!(
                "slip77({})",
                PrivateKey::new(key, Network::Bitcoin).to_wif()
            ),
            None => {
                failure = true;
                String::new()
            }
        }
    });
    if failure {
        return Err(Error::Value("Secret is not valid (larger then N?)".into()));
    }
    Ok(normalized.into_owned())
}

pub fn assert_standalone_watch_only(input: &str) -> Result<()> {
    let input = kassiber_pytext::strip(input);
    let key_start = input.find(']').map_or(0, |i| i + 1);
    let end = input[key_start..]
        .find('/')
        .map_or(input.len(), |i| i + key_start);
    let encoded = &input[key_start..end];
    let mut normalized = None;
    if let Ok(mut raw) = bitcoin::base58::decode_check(encoded) {
        if raw.len() == 78
            && raw[45] == 0
            && [
                [0x04, 0x9d, 0x78, 0x78],
                [0x04, 0xb2, 0x43, 0x0c],
                [0x04, 0x4a, 0x4e, 0x28],
                [0x04, 0x5f, 0x18, 0xbc],
            ]
            .contains(&raw[..4].try_into().expect("four bytes"))
        {
            raw[..4].copy_from_slice(&[0x04, 0x88, 0xad, 0xe4]);
            normalized = Some(format!(
                "{}{}{}",
                &input[..key_start],
                bitcoin::base58::encode_check(&raw),
                &input[end..]
            ));
        }
    }
    if DescriptorSecretKey::from_str(normalized.as_deref().unwrap_or(input)).is_ok() {
        return Err(spending_error().into());
    }
    Ok(())
}

/// Check key expressions, never the Liquid blinding slot. No error from a
/// dependency is allowed to echo a parsed private key or the complete input.
pub fn assert_watch_only(input: &str) -> Result<()> {
    let normalized = normalize_descriptor_text("liquid", input)?;
    let spend = spending_text(&normalized).0;
    for token in spend.split(['(', ')', ',', '{', '}']) {
        assert_standalone_watch_only(token)?;
    }
    Ok(())
}

fn spending_text(input: &str) -> (&str, Option<&str>) {
    if let Some(rest) = input.strip_prefix("blinded(") {
        let mut depth = 0;
        for (i, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    return (
                        rest[i + 1..].strip_suffix(')').unwrap_or(&rest[i + 1..]),
                        Some(&rest[..i]),
                    )
                }
                _ => {}
            }
        }
    }
    (input, None)
}

fn invalid_descriptor() -> Error {
    AppError::new("validation", "Invalid wallet descriptor").into()
}
fn public_path_preflight(input: &str) -> Result<()> {
    for token in input.split(['(', ')', ',', '{', '}']) {
        let mut token = token;
        if let Some(origin) = token.strip_prefix('[') {
            if let Some((origin, rest)) = origin.split_once(']') {
                let fingerprint = origin.split('/').next().unwrap_or_default();
                if fingerprint.len() % 2 != 0 {
                    return Err(Error::Value("Odd-length string".into()));
                }
                token = rest;
            }
        }
        if token.starts_with("xpub") || token.starts_with("tpub") {
            for step in token.split('/').skip(1) {
                if step == "*" || step.starts_with('<') || step.ends_with('>') {
                    continue;
                }
                if step.ends_with(['h', 'H', '\'']) {
                    return Err(invalid_descriptor());
                }
                let n = kassiber_pytext::parse_int(step).map_err(|_| {
                    Error::Value(format!(
                        "invalid literal for int() with base 10: {}",
                        kassiber_pytext::repr_str(step)
                    ))
                })?;
                if n < 0.into() || n >= 2147483648_u64.into() {
                    return Err(invalid_descriptor());
                }
            }
        }
    }
    Ok(())
}

enum Parsed {
    Bitcoin(Box<Descriptor<DescriptorPublicKey>>, Vec<(String, String)>),
    Liquid(Box<em::Descriptor<em::DescriptorPublicKey>>, Option<Blind>),
}
/// The blinding slot of a Liquid descriptor.
#[derive(Clone)]
enum Blind {
    Slip77(em::confidential::slip77::MasterBlindingKey),
    /// Any other key, as embit reads and derives it.
    View(Box<ViewKey>),
}
/// embit's non-SLIP-77 key must mirror the spending descriptor's wildcard
/// and branch count.
fn mirror(view: &ViewKey, descriptor: &em::Descriptor<em::DescriptorPublicKey>) -> Result<()> {
    let branches = descriptor
        .clone()
        .into_single_descriptors()
        .map_err(|_| invalid_descriptor())?
        .len();
    if view.is_wildcard() != descriptor.has_wildcard() || view.branches() != branches {
        return Err(invalid_descriptor());
    }
    Ok(())
}
impl Parsed {
    fn parse(chain: &str, input: &str) -> Result<Self> {
        let input = &legacy_sets(input);
        assert_watch_only(input)?;
        let (spend, view) = spending_text(input);
        // embit reads the blinding key before the spending descriptor.
        let blind = match view {
            Some(view) if chain != "bitcoin" => Some(
                match view
                    .strip_prefix("slip77(")
                    .and_then(|s| s.strip_suffix(')'))
                {
                    Some(key) => {
                        let hex = PrivateKey::from_wif(key)
                            .map(|k| k.inner.display_secret().to_string())
                            .unwrap_or_else(|_| key.into());
                        Blind::Slip77(
                            hex.parse()
                                .map_err(|_| Error::Value("Invalid blinding key".into()))?,
                        )
                    }
                    None => Blind::View(Box::new(ViewKey::parse(view)?)),
                },
            ),
            _ => None,
        };
        public_path_preflight(spend)?;
        // embit accepts these script wrappers; other top-level shapes are
        // rejected even when the Rust dependency understands them.
        if !["pkh(", "wpkh(", "sh(", "wsh(", "tr("]
            .iter()
            .any(|p| spend.starts_with(p))
        {
            let prefix: String = spend.chars().take(7).collect();
            return Err(Error::Value(format!(
                "Invalid descriptor (starts with '{}')",
                prefix
            )));
        }
        if chain == "bitcoin" {
            let keys = regex::Regex::new(
                r"\b(?:xpub|tpub|ypub|zpub|Ypub|Zpub|upub|vpub|Upub|Vpub)[1-9A-HJ-NP-Za-km-z]+",
            )
            .expect("constant regex");
            let mut versions = vec![];
            let normalized = keys.replace_all(input, |m: &regex::Captures<'_>| {
                let Ok(mut bytes) = bitcoin::base58::decode_check(&m[0]) else {
                    return m[0].to_owned();
                };
                if bytes.len() != 78 {
                    return m[0].to_owned();
                }
                let main = matches!(&m[0][..4], "xpub" | "ypub" | "zpub" | "Ypub" | "Zpub");
                bytes[..4].copy_from_slice(if main {
                    &[0x04, 0x88, 0xb2, 0x1e]
                } else {
                    &[0x04, 0x35, 0x87, 0xcf]
                });
                let normalized = bitcoin::base58::encode_check(&bytes);
                versions.push((normalized.clone(), m[0].to_owned()));
                normalized
            });
            let parsed = Descriptor::from_str(&normalized).map_err(|_| parse_error(spend))?;
            Ok(Self::Bitcoin(Box::new(parsed), versions))
        } else {
            let el = elements_text(spend);
            let descriptor = em::Descriptor::from_str(&el).map_err(|_| parse_error(spend))?;
            if let Some(Blind::View(view)) = &blind {
                mirror(view, &descriptor)?;
            }
            Ok(Self::Liquid(Box::new(descriptor), blind))
        }
    }
    fn canonical(&self) -> String {
        match self {
            Self::Bitcoin(d, versions) => {
                let canonical = format!("{d:#}").replace('\'', "h");
                let keys = regex::Regex::new(r"\b(?:xpub|tpub)[1-9A-HJ-NP-Za-km-z]+")
                    .expect("constant regex");
                let mut used = std::collections::BTreeMap::<String, usize>::new();
                keys.replace_all(&canonical, |m: &regex::Captures<'_>| {
                    let count = used.entry(m[0].to_owned()).or_default();
                    let original = versions
                        .iter()
                        .filter(|(key, _)| key == &m[0])
                        .nth(*count)
                        .map(|(_, original)| original.clone())
                        .unwrap_or_else(|| m[0].to_owned());
                    *count += 1;
                    original
                })
                .into_owned()
            }
            Self::Liquid(d, view) => {
                let mut t = format!("{d:#}").replace('\'', "h");
                for (a, b) in [
                    ("elwpkh(", "wpkh("),
                    ("elwsh(", "wsh("),
                    ("elsh(", "sh("),
                    ("eltr(", "tr("),
                    ("elpkh(", "pkh("),
                ] {
                    t = t.replace(a, b);
                }
                if let Some(key) = view {
                    let key = match key {
                        Blind::Slip77(k) => {
                            let sk = bitcoin::secp256k1::SecretKey::from_slice(k.as_bytes())
                                .expect("valid view key");
                            format!("slip77({})", PrivateKey::new(sk, Network::Bitcoin).to_wif())
                        }
                        Blind::View(view) => view.to_string(),
                    };
                    t = format!("blinded({key},{t})");
                }
                t
            }
        }
    }
    fn singles(&self) -> Result<Vec<Self>> {
        match self {
            Self::Bitcoin(d, versions) => Ok(d
                .as_ref()
                .clone()
                .into_single_descriptors()
                .map_err(|_| Error::Value("Invalid multipath descriptor".into()))?
                .into_iter()
                .map(|d| Self::Bitcoin(Box::new(d), versions.clone()))
                .collect()),
            Self::Liquid(d, k) => {
                let singles = d
                    .as_ref()
                    .clone()
                    .into_single_descriptors()
                    .map_err(|_| Error::Value("Invalid multipath descriptor".into()))?;
                let multi = singles.len() > 1;
                // `LDescriptor.branch` branches the view key alongside.
                Ok(singles
                    .into_iter()
                    .enumerate()
                    .map(|(i, d)| {
                        let key = match k {
                            Some(Blind::View(view)) if multi => {
                                Some(Blind::View(Box::new(view.branch(i))))
                            }
                            other => other.clone(),
                        };
                        Self::Liquid(Box::new(d), key)
                    })
                    .collect())
            }
        }
    }
    fn ranged(&self) -> bool {
        match self {
            Self::Bitcoin(d, _) => d.has_wildcard(),
            Self::Liquid(d, _) => d.has_wildcard(),
        }
    }
    fn can_unblind(&self) -> bool {
        match self {
            Self::Bitcoin(_, _) => true,
            Self::Liquid(_, Some(Blind::Slip77(_))) => true,
            Self::Liquid(_, Some(Blind::View(view))) => view.is_private(),
            Self::Liquid(_, None) => false,
        }
    }
}
/// embit reads a legacy `{a,b}` branch set wherever a key's derivation has
/// one and spells it `<a;b>`; only a `/` can precede it, never a taproot
/// tree's brace.
fn legacy_sets(input: &str) -> String {
    let pattern = regex::Regex::new(r"/\{([^{}]*)\}").expect("constant regex");
    pattern
        .replace_all(input, |m: &regex::Captures<'_>| {
            format!("/<{}>", m[1].replace(',', ";"))
        })
        .into_owned()
}

fn elements_text(input: &str) -> String {
    // Elements prefixes only the outer script wrapper; nested wsh/wpkh
    // and miniscript expressions retain the Bitcoin spelling.
    format!("el{input}")
}

fn parse_error(input: &str) -> Error {
    if input.starts_with("wsh(") || input.starts_with("sh(") && !input.starts_with("sh(wpkh(") {
        // Miniscript construction errors are typed by the Python wallet
        // wrapper, but key-reader ValueErrors retain their original text.
        let wrappers = ["sh", "wsh", "multi", "sortedmulti", "pk", "pk_k", "pk_h"];
        for token in input
            .split(['(', ')', ',', '{', '}'])
            .filter(|s| !s.is_empty())
        {
            if wrappers.contains(&token) || token.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if DescriptorPublicKey::from_str(token).is_err() {
                return parse_error(&format!("wpkh({token})"));
            }
        }
        return invalid_descriptor();
    }
    // A complete valid key plus malformed wrappers is a descriptor error,
    // rather than an error in base58 decoding. Preserve embit's close count.
    let opens = if input.starts_with("sh(wpkh(") { 2 } else { 1 };
    let key = input
        .split('(')
        .nth(opens)
        .unwrap_or_default()
        .split(')')
        .next()
        .unwrap_or_default();
    let key_without_origin = key
        .strip_prefix('[')
        .and_then(|s| s.split_once(']').map(|(_, s)| s))
        .unwrap_or(key);
    if DescriptorPublicKey::from_str(key).is_ok() {
        let closes = input.chars().rev().take_while(|c| *c == ')').count();
        if closes < opens
            && !input
                .split_once(')')
                .is_some_and(|(_, tail)| tail.chars().any(|c| c != ')'))
        {
            return Error::Value(format!(
                "Invalid descriptor (expected ')' but ends with '{}')",
                ")".repeat(closes)
            ));
        }
        return invalid_descriptor();
    }
    if [66, 130].contains(&key_without_origin.len())
        && ["02", "03", "04"]
            .iter()
            .any(|prefix| key_without_origin.starts_with(prefix))
        && key_without_origin.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return invalid_descriptor();
    }
    // embit's reader reports the first invalid base58 character. Do not
    // include the complete key or descriptor in the error.
    let bad = key_without_origin
        .chars()
        .find(|c| !"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(*c))
        .unwrap_or('(');
    Error::Value(format!("Character '{bad}' is not a valid base58 character"))
}

pub struct Branch {
    pub index: u32,
    pub label: String,
    pub selector: Option<usize>,
    descriptor: String,
    selected: Parsed,
}
/// The blinding material LWK needs, without exposing the key itself.
pub(crate) enum Blinding {
    /// Lowercase hex of the SLIP-77 master blinding key.
    Slip77(String),
    /// A private view key, spelled as in the canonical descriptor text.
    PrivateView,
    /// Public or absent blinding material: LWK cannot unblind with it.
    Unusable,
}
impl Branch {
    pub(crate) fn selected_descriptor(&self) -> String {
        self.selected.canonical()
    }
    /// The canonical (embit-spelled) descriptor text of this plan entry.
    pub(crate) fn descriptor_text(&self) -> &str {
        &self.descriptor
    }
    pub(crate) fn liquid_blinding(&self) -> Blinding {
        match &self.selected {
            Parsed::Liquid(_, Some(Blind::Slip77(key))) => {
                Blinding::Slip77(key.as_bytes().iter().map(|b| format!("{b:02x}")).collect())
            }
            Parsed::Liquid(_, Some(Blind::View(view))) if view.is_private() => {
                Blinding::PrivateView
            }
            _ => Blinding::Unusable,
        }
    }
    pub fn ranged(&self) -> bool {
        self.selected.ranged()
    }
}
pub struct Plan {
    pub chain: String,
    pub network: String,
    pub gap_limit: u32,
    pub fingerprint: String,
    pub branches: Vec<Branch>,
}
pub fn template(kind: &str, key: &str, branch: &str) -> String {
    let key = format!("{key}/{branch}/*");
    match kind {
        "p2pkh" => format!("pkh({key})"),
        "p2sh-p2wpkh" => format!("sh(wpkh({key}))"),
        "p2tr" => format!("tr({key})"),
        _ => format!("wpkh({key})"),
    }
}

pub fn load(config: &Value) -> Result<Option<Plan>> {
    let mut descriptor = kassiber_pytext::strip(&field(config, "descriptor")).to_owned();
    let xpub = kassiber_pytext::strip(&field(config, "xpub")).to_owned();
    let mut change = kassiber_pytext::strip(&field(config, "change_descriptor")).to_owned();
    assert_standalone_watch_only(&xpub)?;
    assert_watch_only(&descriptor)?;
    assert_watch_only(&change)?;
    let types = config
        .get("script_types")
        .and_then(Value::as_array)
        .map(|t| {
            t.iter()
                .map(|v| kassiber_pytext::strip(&text(v)).to_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let types: Vec<_> = SCRIPT_TYPES
        .iter()
        .filter(|s| types.iter().any(|t| t == **s))
        .copied()
        .collect();
    if descriptor.is_empty() && (xpub.is_empty() || types.is_empty()) {
        return Ok(None);
    }
    let mut synthesize = config.get("synthesize_change") != Some(&Value::Bool(false));
    if let Some(record) = crate::material::bsms(&descriptor)? {
        descriptor = field(&record, "descriptor");
        if change.is_empty() {
            change = field(&record, "change_descriptor");
        }
        synthesize = false;
    } else if field(config, "descriptor_source") == "bsms" {
        synthesize = false;
    }
    let chain = normalize_chain(&field(config, "chain"))?;
    let mut network = normalize_network(&chain, &field(config, "network"))?;
    if chain == "bitcoin" {
        let re = regex::Regex::new(r"(?:^|[^A-Za-z0-9])(xpub|tpub)[1-9A-HJ-NP-Za-km-z]")
            .expect("constant regex");
        let mut families = std::collections::BTreeSet::new();
        for source in [&xpub, &descriptor, &change] {
            for cap in re.captures_iter(source) {
                families.insert(if &cap[1] == "xpub" { "main" } else { "test" });
            }
        }
        if families.len() > 1 {
            return Err(AppError::new(
                "validation",
                "Bitcoin descriptors cannot mix mainnet and test-family extended public keys",
            )
            .into());
        }
        if let Some(family) = families.first() {
            if field(config, "network").is_empty() {
                network = (*family).into();
            } else if (network == "main") != (*family == "main") {
                return Err(AppError::new(
                    "validation",
                    "Extended public key network does not match the configured Bitcoin network",
                )
                .with_details(dict([
                    ("configured_network", s(network.as_str())),
                    ("key_network", s(*family)),
                ]))
                .into());
            }
        }
    }
    let gap = config
        .get("gap_limit")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or(json!(100));
    let gap =
        match gap {
            Value::String(v) => v.trim().parse::<i64>().map_err(|_| {
                Error::Value(format!("invalid literal for int() with base 10: '{v}'"))
            })?,
            Value::Number(v) => v
                .as_i64()
                .or_else(|| v.as_f64().map(|n| n as i64))
                .unwrap_or(0),
            Value::Bool(b) => i64::from(b),
            _ => return Err(Error::Value(
                "int() argument must be a string, a bytes-like object or a real number, not 'dict'"
                    .into(),
            )),
        };
    if gap <= 0 {
        return Err(Error::Value("Descriptor gap limit must be positive".into()));
    }
    if gap > 5000 {
        return Err(Error::Value(
            "Descriptor gap limit must be 5000 or lower".into(),
        ));
    }
    let mut branches = vec![];
    if descriptor.is_empty() {
        for kind in types {
            let base = SCRIPT_TYPES
                .iter()
                .position(|t| *t == kind)
                .expect("known type") as u32
                * 2;
            let parsed = Parsed::parse(&chain, &template(kind, &xpub, "<0;1>"))?;
            let canonical = parsed.canonical();
            for (selector, selected) in parsed.singles()?.into_iter().enumerate() {
                branches.push(Branch {
                    index: base + selector as u32,
                    label: format!(
                        "{kind} {}",
                        if selector == 0 { "receive" } else { "change" }
                    ),
                    selector: Some(selector),
                    descriptor: canonical.clone(),
                    selected,
                });
            }
        }
    } else {
        let normalized = normalize_descriptor_text(&chain, &descriptor)?;
        let mut parsed = Parsed::parse(&chain, &normalized)?;
        if !change.is_empty() {
            let selected = Parsed::parse(&chain, &normalize_descriptor_text(&chain, &change)?)?;
            branches.push(Branch {
                index: 0,
                label: "receive".into(),
                selector: None,
                descriptor: parsed.canonical(),
                selected: parsed,
            });
            branches.push(Branch {
                index: 1,
                label: "change".into(),
                selector: None,
                descriptor: selected.canonical(),
                selected,
            });
        } else {
            if synthesize
                && parsed.singles()?.len() < 2
                && parsed.ranged()
                && normalized.contains("/0/*")
            {
                if let Ok(promoted) = Parsed::parse(&chain, &normalized.replace("/0/*", "/<0;1>/*"))
                {
                    if promoted.singles()?.len() == 2 {
                        parsed = promoted;
                    }
                }
            }
            let canonical = parsed.canonical();
            let singles = parsed.singles()?;
            let multi = singles.len() > 1;
            for (i, selected) in singles.into_iter().take(2).enumerate() {
                branches.push(Branch {
                    index: i as u32,
                    label: if i == 0 { "receive" } else { "change" }.into(),
                    selector: multi.then_some(i),
                    descriptor: canonical.clone(),
                    selected,
                });
            }
        }
    }
    let payload = json!({"chain":chain,"network":network,"gap_limit":gap,"branches":branches.iter().map(|b|json!({"branch_index":b.index,"selector":b.selector,"descriptor":b.descriptor})).collect::<Vec<_>>()});
    let fingerprint = Sha256::digest(python_json(&payload).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Some(Plan {
        chain,
        network,
        gap_limit: gap as u32,
        fingerprint,
        branches,
    }))
}

pub fn python_json(value: &Value) -> String {
    let value = kassiber_pytext::loads(&value.to_string()).expect("JSON value");
    kassiber_pyjson::dumps(
        &value,
        &kassiber_pyjson::Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .expect("JSON value")
}

impl Plan {
    pub fn branch_index(&self, branch: Option<&str>) -> std::result::Result<Option<u32>, AppError> {
        let branch = match branch {
            None | Some("" | "all") => return Ok(None),
            Some(branch) => branch,
        };
        let normalized = kassiber_pytext::strip(branch).to_lowercase();
        let valid = self.branches.iter().map(|b| b.index).collect::<Vec<_>>();
        if !normalized.is_empty()
            && normalized
                .chars()
                .all(|c| kassiber_pytext::decimal_value(c as u32).is_some())
        {
            if let Ok(index) = kassiber_pytext::parse_int(&normalized)
                .and_then(|n| u32::try_from(n).map_err(|_| kassiber_pytext::NumberError::Invalid))
            {
                if valid.contains(&index) {
                    return Ok(Some(index));
                }
            }
            return Err(AppError::new("app_error",format!("Descriptor branch '{branch}' is not in this wallet's plan; available branches: {valid:?}")));
        }
        let suffix = match normalized.as_str() {
            "receive" | "external" => Some("receive"),
            "change" | "internal" => Some("change"),
            _ => None,
        };
        if let Some(suffix) = suffix {
            let matching = self
                .branches
                .iter()
                .filter(|b| b.label == suffix || b.label.ends_with(&format!(" {suffix}")))
                .collect::<Vec<_>>();
            if matching.len() == 1 {
                return Ok(Some(matching[0].index));
            }
        }
        let key = |s: &str| {
            kassiber_pytext::strip(s)
                .to_lowercase()
                .replace('-', " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        if let Some(found) = self
            .branches
            .iter()
            .find(|b| key(&b.label) == key(&normalized))
        {
            return Ok(Some(found.index));
        }
        Err(AppError::new("app_error",format!("Descriptor branch must be 'all', a branch index in {valid:?}, or a branch label like 'receive'/'p2tr receive'")))
    }
    pub fn can_unblind(&self) -> bool {
        self.branches.iter().all(|b| b.selected.can_unblind())
    }
    /// `liquid_blinding_secret`: the private blinding key of one derived
    /// target, SLIP-77 or a view key as embit derives it, from the same
    /// script and view key as the target's confidential address (a fixed
    /// branch included, since Python round 14).
    pub(crate) fn liquid_blinding_secret(
        &self,
        branch_index: u32,
        address_index: u32,
    ) -> Result<elements::secp256k1_zkp::SecretKey> {
        let missing = || {
            Error::Value("Liquid descriptor does not include the private blinding material needed for unblinding".into())
        };
        let branch = self
            .branches
            .iter()
            .find(|b| b.index == branch_index)
            .ok_or_else(|| {
                Error::Value(format!(
                    "Descriptor branch '{branch_index}' is not configured"
                ))
            })?;
        let ranged = branch.selected.ranged();
        if !ranged && address_index != 0 {
            return Err(Error::Value(format!(
                "Descriptor branch '{}' is not ranged",
                branch.label
            )));
        }
        let Parsed::Liquid(descriptor, Some(blind)) = &branch.selected else {
            return Err(missing());
        };
        let secp = elements::secp256k1_zkp::Secp256k1::verification_only();
        let script = descriptor
            .derived_descriptor(&secp, address_index)
            .map_err(|_| Error::Value("Can't do hardened derivation from public key".into()))?
            .script_pubkey();
        match blind {
            Blind::Slip77(master) => Ok(master.blinding_private_key(&script)),
            Blind::View(view) => {
                let secret = view
                    .blinding_secret(address_index, script.as_bytes())?
                    .ok_or_else(missing)?;
                elements::secp256k1_zkp::SecretKey::from_slice(&secret.secret_bytes())
                    .map_err(|_| missing())
            }
        }
    }
    pub fn projection(&self, start: u32, end: u32) -> Result<Value> {
        Ok(
            json!({"chain":self.chain,"network":self.network,"gap_limit":self.gap_limit,
            "descriptor_fingerprint":self.fingerprint,"can_unblind":self.can_unblind(),
            "branches":self.branches.iter().map(|b|json!({"branch_index":b.index,"branch_label":b.label,"selector":b.selector,"descriptor":b.descriptor,"selected_descriptor":b.selected.canonical(),"limit":if b.selected.ranged(){self.gap_limit}else{1}})).collect::<Vec<_>>(),
            "targets":self.derive(None,start,end)?}),
        )
    }
    pub fn derive(&self, branch: Option<u32>, start: u32, end: u32) -> Result<Vec<Value>> {
        if end < start {
            return Err(Error::Value(
                "Descriptor range must satisfy 0 <= start <= end".into(),
            ));
        }
        let mut result = vec![];
        for b in &self.branches {
            if branch.is_some_and(|n| n != b.index) {
                continue;
            }
            for index in start..if b.selected.ranged() { end } else { end.min(1) } {
                result.push(self.target(b, index)?);
            }
        }
        Ok(result)
    }
    fn target(&self, branch: &Branch, index: u32) -> Result<Value> {
        let mut paths = vec![];
        let mut origins = vec![];
        let (address, unconfidential, script) = match &branch.selected {
            Parsed::Bitcoin(d, _) => {
                metadata(d, branch.selected.ranged(), index, &mut paths, &mut origins)?;
                let derived = d
                    .derived_descriptor(&bitcoin::secp256k1::Secp256k1::verification_only(), index)
                    .map_err(|_| {
                        Error::Value("Can't do hardened derivation from public key".into())
                    })?;
                let network = match self.network.as_str() {
                    "test" => Network::Testnet,
                    "signet" => Network::Signet,
                    "regtest" => Network::Regtest,
                    _ => Network::Bitcoin,
                };
                (
                    derived
                        .address(network)
                        .map_err(|_| Error::Value("Script has no address".into()))?
                        .to_string(),
                    None,
                    derived.script_pubkey().to_hex_string(),
                )
            }
            Parsed::Liquid(d, view) => {
                // The spending grammar and BIP32 metadata are shared with Bitcoin.
                let canonical = branch.selected.canonical();
                let bitcoin = Descriptor::from_str(spending_text(&canonical).0)
                    .map_err(|_| Error::Value("Unsupported Liquid spending descriptor".into()))?;
                metadata(
                    &bitcoin,
                    branch.selected.ranged(),
                    index,
                    &mut paths,
                    &mut origins,
                )?;
                let secp = elements::secp256k1_zkp::Secp256k1::new();
                let derived = d.derived_descriptor(&secp, index).map_err(|_| {
                    Error::Value("Can't do hardened derivation from public key".into())
                })?;
                let params = match self.network.as_str() {
                    "liquidtestnet" => &elements::AddressParams::LIQUID_TESTNET,
                    "elementsregtest" => &elements::AddressParams::ELEMENTS,
                    _ => &elements::AddressParams::LIQUID,
                };
                let unconfidential = derived
                    .address(params)
                    .map_err(|_| Error::Value("Script has no address".into()))?
                    .to_string();
                let address = match view {
                    Some(Blind::Slip77(key)) => em::confidential::Descriptor {
                        key: em::confidential::Key::Slip77(*key),
                        descriptor: derived.clone(),
                    }
                    .address(&secp, params)
                    .map_err(|_| Error::Value("Invalid blinding key".into()))?
                    .to_string(),
                    Some(Blind::View(view)) => {
                        let script = derived.script_pubkey();
                        let public = view.blinding_public(index, script.as_bytes())?;
                        let public =
                            elements::secp256k1_zkp::PublicKey::from_slice(&public.serialize())
                                .map_err(|_| Error::Value("Invalid blinding key".into()))?;
                        elements::Address::from_script(&script, Some(public), params)
                            .ok_or_else(|| Error::Value("Script has no address".into()))?
                            .to_string()
                    }
                    None => unconfidential.clone(),
                };
                (
                    address,
                    Some(unconfidential),
                    derived
                        .script_pubkey()
                        .as_bytes()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                )
            }
        };
        Ok(
            json!({"chain":self.chain,"network":self.network,"branch_index":branch.index,"branch_label":branch.label,"address_index":index,"address":address,"unconfidential_address":unconfidential,"script_pubkey":script,"derivation_path":if paths.len()==1 {paths.first()}else{None},"derivation_paths":paths,"key_origins":origins}),
        )
    }
}

fn path_string(path: &bip32::DerivationPath) -> String {
    let suffix = path.to_string().replace('h', "'");
    if suffix.is_empty() {
        "m".into()
    } else {
        format!("m/{suffix}")
    }
}
fn metadata(
    d: &Descriptor<DescriptorPublicKey>,
    _ranged: bool,
    index: u32,
    paths: &mut Vec<String>,
    origins: &mut Vec<String>,
) -> Result<()> {
    let mut error = None;
    let mut keys = vec![];
    if let Descriptor::Tr(tr) = d {
        // embit visits the internal key before the script leaves. Miniscript's
        // generic traversal visits it last, which changes public metadata.
        keys.push(tr.internal_key());
        for (_, script) in tr.iter_scripts() {
            script.for_each_key(|key| {
                keys.push(key);
                true
            });
        }
    } else {
        d.for_each_key(|key| {
            keys.push(key);
            true
        });
    }
    for key in keys {
        let has_origin = match key {
            DescriptorPublicKey::Single(k) => k.origin.is_some(),
            DescriptorPublicKey::XPub(k) => {
                k.origin.is_some()
                    || !k.derivation_path.is_empty()
                    || k.wildcard != miniscript::descriptor::Wildcard::None
            }
            DescriptorPublicKey::MultiXPub(_) => true,
        };
        if !has_origin {
            continue;
        }
        let derived = match key.clone().at_derivation_index(index) {
            Ok(d) => d,
            Err(_) => {
                error = Some(Error::Value(
                    "Can't do hardened derivation from public key".into(),
                ));
                break;
            }
        };
        if let Some(path) = derived.full_derivation_path() {
            let path = path_string(&path);
            let fp = key.master_fingerprint();
            let origin = if path == "m" {
                format!("[{fp}]")
            } else {
                format!("[{fp}/{}]", &path[2..])
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
    }
    if let Some(error) = error {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn liquid_blinding_secrets_match_embit_slip77() {
        // `wallet_descriptors.liquid_blinding_secret` on the LWK vector descriptor.
        let config = serde_json::json!({"chain":"liquid","network":"elementsregtest","gap_limit":3,
            "descriptor":"ct(slip77(f46e151e2e0d81601e9e25334f7a9f4ffaf9790192b387af036a70c88304e933),elwpkh(xpub6Bm9M1SxZdzL3TxdNV8897FgtTLBgehR1wVNnMyJ5VLRK5n3tFqXxrCVnVQj4zooN4eFSkf6Sma84reWc5ZCXMxPbLXQs3BcaBdTd4YQa3B/<0;1>/*))"});
        let plan = load(&config).ok().flatten().unwrap();
        for (branch, index, expected) in [
            (
                0,
                0,
                "1b7aca7b05469fdae66679bdef835e835234d24b0eed2c192057ee6d5559b9fc",
            ),
            (
                1,
                2,
                "6efc1c37e8fe3e38757fec0222b45dd882ae7cbd908ef6a2ab938ffcdf66fa6e",
            ),
        ] {
            let secret = plan.liquid_blinding_secret(branch, index).ok().unwrap();
            let hex: String = secret
                .secret_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(hex, expected);
        }
        assert!(plan.liquid_blinding_secret(7, 0).is_err());
    }

    #[test]
    fn liquid_blinding_secrets_match_python_vectors() {
        // `liquid_blinding_secret` per target: SLIP-77 and embit view keys,
        // including a fixed branch, where Python tweaks the base-key script.
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        let mut failures = vec![];
        for case in vectors["blinding_secrets"].as_array().unwrap() {
            let plan = load(&case["config"]).ok().flatten().expect("plan");
            for secret in case["secrets"].as_array().unwrap() {
                let index = |key: &str| secret[key].as_u64().unwrap() as u32;
                let actual = match plan
                    .liquid_blinding_secret(index("branch_index"), index("address_index"))
                {
                    Ok(key) => json!({"ok": key
                        .secret_bytes()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()}),
                    Err(error) => error.projection(),
                };
                if actual != secret["expected"] {
                    failures.push(format!("{} {}", case["name"], secret));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        assert!(matches!(
            load(&vectors["blinding_secrets"][0]["config"]).ok().flatten().unwrap().liquid_blinding_secret(7, 0),
            Err(Error::Value(m)) if m == "Descriptor branch '7' is not configured"
        ));
    }

    #[test]
    fn python_normalization_and_branch_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        for case in vectors["normalizations"].as_array().unwrap() {
            let actual = match normalize_descriptor_text(
                case["chain"].as_str().unwrap(),
                case["text"].as_str().unwrap(),
            ) {
                Ok(value) => json!({"ok":value}),
                Err(error) => error.projection(),
            };
            assert_eq!(actual, case["expected"], "normalization");
        }
        for case in vectors["branch_selections"].as_array().unwrap() {
            let plan = load(&case["config"])
                .ok()
                .flatten()
                .expect("valid public plan");
            let actual = match plan.branch_index(case["branch"].as_str()) {
                Ok(value) => json!({"ok":value}),
                Err(error) => Error::App(error).projection(),
            };
            assert_eq!(actual, case["expected"], "branch {:?}", case["branch"]);
        }
    }
    fn differences(actual: &Value, expected: &Value, path: &str) -> Vec<String> {
        if actual == expected {
            return vec![];
        }
        match (actual, expected) {
            (Value::Object(a), Value::Object(b)) => a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .flat_map(|key| differences(&actual[key], &expected[key], &format!("{path}/{key}")))
                .collect(),
            (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
                .iter()
                .zip(b)
                .enumerate()
                .flat_map(|(i, (a, b))| differences(a, b, &format!("{path}/{i}")))
                .collect(),
            _ => vec![path.into()],
        }
    }
    #[test]
    fn python_descriptor_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        let mut failures = vec![];
        for case in vectors["plans"].as_array().unwrap() {
            let actual = match load(&case["config"])
                .and_then(|p| p.map(|p| p.projection(0, 5)).transpose())
            {
                Ok(v) => json!({"ok":v}),
                Err(e) => e.projection(),
            };
            if actual != case["expected"] {
                failures.push(format!(
                    "{}: {}",
                    case["name"],
                    differences(&actual, &case["expected"], "").join(", ")
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
