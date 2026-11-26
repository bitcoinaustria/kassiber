//! Log redaction, ported from lib/appLogs.ts: the secret floor applied at insert, and the
//! `high_signal` / `public_safe` render tiers (txids and amounts always pseudonymized).
//!
//! The floor runs the backend's own rules first (`crate::secret_floor`, compiled from
//! `crates/envelope`: the full credential scanner plus the key, descriptor and token patterns),
//! then appLogs.ts's extra patterns through `super::regex`. The daemon's own ring is already
//! redacted daemon-side.

use std::sync::OnceLock;

use serde_json::Value;

use super::regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    HighSignal,
    PublicSafe,
}

impl Mode {
    pub fn id(self) -> &'static str {
        match self {
            Mode::HighSignal => "high_signal",
            Mode::PublicSafe => "public_safe",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Opts {
    pub redacted: bool,
    pub mask_amounts: bool,
    pub mode: Mode,
}

impl Opts {
    pub fn screen(redacted: bool, mask_amounts: bool) -> Opts {
        Opts {
            redacted,
            mask_amounts,
            mode: Mode::PublicSafe,
        }
    }
}

pub const SECRET_FLOOR_TYPES: [&str; 4] = ["api_key", "descriptor", "xpriv", "xpub"];
pub const OPERATIONAL_TYPES: [&str; 8] = [
    "address", "email", "ip", "label", "onion", "path", "txid", "url",
];

fn re(p: &str, icase: bool) -> Regex {
    Regex::new(p, icase).unwrap_or_else(|e| panic!("redaction pattern {p}: {e}"))
}

struct Patterns {
    seed_runs: Vec<Regex>,
    nsec: Regex,
    wif: Regex,
    xprv: Regex,
    xpub: Regex,
    sp_desc: Regex,
    sp_keys: Regex,
    sp_addr: Regex,
    descriptor: Regex,
    bearer: Regex,
    sk: Regex,
    url: Regex,
    email: Regex,
    rate: Regex,
    bech32: Regex,
    base58: Regex,
    onion: Regex,
    path: Regex,
    txid: Regex,
    amount: Regex,
    keyed_amount: Regex,
    amount_id: Regex,
    rate_prefix: Regex,
    script_type: Regex,
    origin: Regex,
    num: Regex,
}

const NUM: &str = r"[+-]?(?:(?:\d{1,3}(?:[,_ .]\d{3})+)|\d+)(?:[.,]\d+)?";
const UNITS: &str =
    "BTC|XBT|LBTC|sats?|msats?|EUR|USD|CHF|GBP|JPY|CAD|AUD|NZD|SEK|NOK|DKK|PLN|CZK|HUF";
const SYMS: &str = "\\u20ac$\\u00a3\\u00a5\\u20bf";

fn pats() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        seed_runs: [24, 21, 18, 15, 12].iter().map(|n| re(&format!(r"\b(?:[a-z]{{3,8}}\s+){{{}}}[a-z]{{3,8}}\b", n - 1), false)).collect(),
        nsec: re(r"(?<![A-Za-z0-9_])nsec1[023456789acdefghjklmnpqrstuvwxyz]{20,}(?![A-Za-z0-9_])", true),
        wif: re(r"(?<![A-Za-z0-9_])(?:5[1-9A-HJ-NP-Za-km-z]{49,50}|[KL][1-9A-HJ-NP-Za-km-z]{50,51})(?![A-Za-z0-9_])", false),
        xprv: re(r"(?<![A-Za-z0-9_])(?:xprv|tprv|yprv|zprv|uprv|vprv)[A-Za-z0-9]{16,}(?![A-Za-z0-9_])", true),
        xpub: re(r"(?<![A-Za-z0-9_])(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}(?![A-Za-z0-9_])", true),
        sp_desc: re(r"(?<![A-Za-z0-9_])sp\([^\n]{8,800}\)(?:#[a-z0-9]{8})?", true),
        sp_keys: re(r"(?<![A-Za-z0-9_])(?:t?spscan|t?spspend)1[023456789acdefghjklmnpqrstuvwxyz]{20,1020}(?![A-Za-z0-9_])", true),
        sp_addr: re(r"(?<![A-Za-z0-9_])(?:sp|tsp)1[023456789acdefghjklmnpqrstuvwxyz]{100,1020}(?![A-Za-z0-9_])", true),
        descriptor: re(r"(?<![A-Za-z0-9_])(wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,400}\)", true),
        bearer: re(r"(?<![A-Za-z0-9_])[Bb]earer\s+[A-Za-z0-9._~+/-]+=*", false),
        sk: re(r"(?<![A-Za-z0-9_])sk-[A-Za-z0-9._~+/-]{6,}(?![A-Za-z0-9_])", false),
        url: re(r#"\b(?:https?|tcp|ssl)://[^\s,;"')\]}]+"#, true),
        email: re(r"\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b", true),
        rate: re(&format!(r"\b(?:{UNITS})[/-](?:{UNITS})\s*(?::|=|at|rate)?\s*{NUM}\b"), true),
        bech32: re(r"\b(?:bc1|tb1|bcrt1|lq1|ex1)[023456789acdefghjklmnpqrstuvwxyz]{20,90}\b", true),
        base58: re(r"\b[13][1-9A-HJ-NP-Za-km-z]{25,34}\b", false),
        onion: re(r"\b[A-Za-z0-9.-]{16,}\.onion\b", true),
        path: re(r#"(?:^|[\s"'(])(?:/Users|/home|/var|/private|/tmp)/[^\s,;"')\]}]+"#, false),
        txid: re(r"\b[0-9a-f]{64,}\b", true),
        amount: re(
            &[
                format!(r"\b(?:{UNITS})\s*{NUM}\b"),
                format!(r"(?<![A-Za-z0-9]){NUM}\s*(?:{UNITS})\b"),
                format!(r"[{SYMS}]\s*{NUM}\b"),
                format!(r"(?<![A-Za-z0-9]){NUM}\s*[{SYMS}]"),
            ]
            .join("|"),
            true,
        ),
        keyed_amount: re(r#"\b([A-Za-z][A-Za-z0-9_]*(?:msats?|sats?))\b(['"]?\s*[:=]\s*['"]?)([+-]?\d[\d.,_]*\d|[+-]?\d)"#, true),
        amount_id: re(r"(?<![A-Za-z0-9_-])(?:[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}|[0-9a-f]{32})(?![A-Za-z0-9_-])", true),
        rate_prefix: re(&format!(r"(?:^|\b)(?:{UNITS})$"), true),
        script_type: re(r"\b(wpkh|sh|wsh|tr|pkh|combo|sp)\(", true),
        origin: re(r"\[[^\]\n]{1,120}\]", false),
        num: re(r"[+-]?\d[\d.,_ ]*\d|[+-]?\d", false),
    })
}

/// FNV-1a (32-bit) over UTF-16 code units, as `stableHash` (and the daemon's `_stable_hash`).
pub fn stable_hash(value: &str) -> String {
    let mut h: u32 = 0x811c9dc5;
    for u in value.encode_utf16() {
        h ^= u as u32;
        h = h.wrapping_mul(0x01000193);
    }
    format!("{h:08x}")
}

/// Unit tests salt amount pseudonyms with the corpus salt of
/// `tests/fixtures/operational_redaction_cases.json`, so expected pseudonyms are fixed.
#[cfg(test)]
fn salt() -> &'static str {
    "0000000100000002"
}

#[cfg(not(test))]
fn salt() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| {
        // A runtime nonce, never exported (it only prevents dictionary reversal of amounts).
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let addr = &t as *const u128 as usize;
        format!(
            "{:016x}",
            (t as u64) ^ (addr as u64).rotate_left(17) ^ std::process::id() as u64
        )
    })
}

pub fn pseudo_txid(v: &str) -> String {
    format!("txid#{}", stable_hash(&v.to_lowercase()))
}

fn strip_grouping(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(c, ',' | '_' | ' '))
        .collect()
}

/// `~0.01`, `~100`, `~1e7`: the coarse magnitude a pseudonymized amount keeps.
pub fn amount_magnitude(raw: &str) -> String {
    let n: f64 = js_parse_float(&strip_grouping(raw));
    if !n.is_finite() || n == 0.0 {
        return "~0".into();
    }
    let exp = n.abs().log10().floor() as i32;
    let sign = if n < 0.0 { "-" } else { "" };
    let body = if exp < 0 {
        format!("{:.*}", (-exp) as usize, 10f64.powi(exp))
    } else if exp <= 4 {
        format!("{}", 10i64.pow(exp as u32))
    } else {
        format!("1e{exp}")
    };
    format!("~{sign}{body}")
}

/// `Number.parseFloat`: the longest numeric prefix.
fn js_parse_float(s: &str) -> f64 {
    let s = s.trim_start();
    let mut end = 0;
    let mut seen_dot = false;
    for (i, c) in s.char_indices() {
        let ok =
            c.is_ascii_digit() || (i == 0 && (c == '+' || c == '-')) || (c == '.' && !seen_dot);
        if c == '.' {
            seen_dot = true;
        }
        if !ok {
            break;
        }
        end = i + c.len_utf8();
    }
    s[..end].parse().unwrap_or(f64::NAN)
}

pub fn pseudo_amount(raw: &str, unit: &str, show_scale: bool) -> String {
    let key = format!("{}|{}|{}", salt(), strip_grouping(raw), unit.to_lowercase());
    let token = format!("amount#{}", stable_hash(&key));
    if !show_scale {
        return token;
    }
    let mag = amount_magnitude(raw);
    if unit.is_empty() {
        format!("{token} ({mag})")
    } else {
        format!("{token} ({mag} {unit})")
    }
}

fn parse_typed_amount(v: &Value) -> (String, String) {
    let text = match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().to_owned(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let chars: Vec<char> = text.chars().collect();
    if let Some(&c) = chars.first()
        && "€$£¥₿".contains(c)
    {
        let rest: String = chars[1..]
            .iter()
            .collect::<String>()
            .trim_start()
            .to_owned();
        let num: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || ".,_ +-".contains(*c))
            .collect();
        if num.chars().any(|c| c.is_ascii_digit()) {
            return (num.trim().to_owned(), c.to_string());
        }
    }
    let num: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || ".,_ +-".contains(*c))
        .collect();
    let num = num
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .to_owned();
    if num.is_empty() || !num.chars().any(|c| c.is_ascii_digit()) {
        return (text, String::new());
    }
    let rest = text[num.len()..].trim_start();
    let unit: String = if rest.starts_with(|c: char| "€$£¥₿".contains(c)) {
        rest.chars().take(1).collect()
    } else {
        rest.chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .take(6)
            .collect()
    };
    (num.trim().to_owned(), unit)
}

fn keep_short(v: &str, head: usize, tail: usize) -> String {
    let c: Vec<char> = v.chars().collect();
    if c.len() <= head + tail + 1 {
        return v.to_owned();
    }
    format!(
        "{}...{}",
        c[..head].iter().collect::<String>(),
        c[c.len() - tail..].iter().collect::<String>()
    )
}

fn mask_path(v: &str) -> String {
    let n = v.replace('\\', "/");
    let name = n.split('/').rfind(|p| !p.is_empty()).unwrap_or("path");
    format!("~/.../{name}")
}

pub fn mask_descriptor(v: &str) -> String {
    let p = pats();
    let s: Vec<char> = v.chars().collect();
    let Some((a, b)) = p.script_type.find_at(&s, 0) else {
        return "[redacted-descriptor]".into();
    };
    let script: String = s[a..b - 1].iter().collect();
    let origin = p.origin.find_all(v).into_iter().next().unwrap_or_default();
    format!("{script}({origin}[redacted-key])")
}

pub fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

pub fn stable_masked_value(ty: &str, value: &Value) -> String {
    let v = match value {
        Value::Null => String::new(),
        other => stringify(other),
    };
    if v.is_empty() {
        return String::new();
    }
    match ty {
        "address" => keep_short(&v, 5, 4),
        "url" => format!("url#{}", stable_hash(&v)),
        "xpub" => format!("xpub#{}", stable_hash(&v)),
        "xpriv" => "[redacted-private-key]".into(),
        "descriptor" => mask_descriptor(&v),
        "txid" => pseudo_txid(&v),
        "path" => mask_path(&v),
        "label" => format!("wallet#{}", stable_hash(&v)),
        "onion" => format!("onion#{}", stable_hash(&v)),
        "api_key" => "[redacted-api-key]".into(),
        "email" => format!("email#{}", stable_hash(&v)),
        "ip" => format!("ip#{}", stable_hash(&v)),
        "amount" => {
            let (n, u) = parse_typed_amount(value);
            pseudo_amount(&n, &u, false)
        }
        _ => v,
    }
}

pub fn secret_floor_text(v: &str) -> String {
    let p = pats();
    let mut out = crate::secret_floor::floor::redact_secret_text(v);
    for r in &p.seed_runs {
        out = r.replace_with(&out, "[redacted-seed-phrase]");
    }
    out = p.nsec.replace_with(&out, "[redacted-private-key]");
    out = p.wif.replace_with(&out, "[redacted-private-key]");
    out = p.xprv.replace_with(&out, "[redacted-private-key]");
    out = p.xpub.replace_with(&out, "[redacted-extended-key]");
    out = p.sp_desc.replace_all(&out, |m, _, _| mask_descriptor(m));
    out = p
        .sp_keys
        .replace_with(&out, "[redacted-silent-payment-key]");
    out = p
        .sp_addr
        .replace_with(&out, "[redacted-silent-payment-address]");
    out = p.descriptor.replace_with(&out, "[redacted-descriptor]");
    out = p.bearer.replace_with(&out, "Bearer [redacted]");
    p.sk.replace_with(&out, "[redacted-secret]")
}

/// Runs `f` on the text between complete UUID / 32-hex ids (which never count as amounts).
fn outside_ids(text: &str, f: impl Fn(&str) -> String) -> String {
    let p = pats();
    let s: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let (mut cursor, mut from) = (0, 0);
    while let Some((a, b)) = p.amount_id.find_at(&s, from) {
        out.push_str(&f(&s[cursor..a].iter().collect::<String>()));
        out.extend(&s[a..b]);
        cursor = b;
        from = b.max(a + 1);
    }
    out.push_str(&f(&s[cursor..].iter().collect::<String>()));
    out
}

fn replace_amounts(text: &str, show_scale: bool) -> String {
    let p = pats();
    outside_ids(text, |seg| {
        let full: Vec<char> = seg.chars().collect();
        p.amount.replace_all(seg, |m, off, _| {
            if off > 0 && matches!(full[off - 1], '/' | '-') {
                let before: String = full[..off - 1].iter().collect();
                if p.rate_prefix.is_match(before.trim_end()) {
                    return m.to_owned();
                }
            }
            let num = p
                .num
                .find_all(m)
                .into_iter()
                .next()
                .unwrap_or_else(|| m.to_owned());
            let unit: String = m
                .replacen(&num, "", 1)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            pseudo_amount(&num, &unit, show_scale)
        })
    })
}

fn replace_keyed_amounts(text: &str, show_scale: bool) -> String {
    let p = pats();
    outside_ids(text, |seg| {
        p.keyed_amount.replace_all(seg, |m, _, _| {
            // key, separator and number, re-split without capture groups.
            let key: String = m
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let rest = &m[key.len()..];
            let num_start = rest
                .find(|c: char| c.is_ascii_digit() || c == '+' || c == '-')
                .unwrap_or(rest.len());
            let (sep, num) = rest.split_at(num_start);
            let unit = if key.to_lowercase().contains("msat") {
                "msat"
            } else {
                "sat"
            };
            format!("{key}{sep}{}", pseudo_amount(num, unit, show_scale))
        })
    })
}

fn high_signal_text(v: &str, show_scale: bool) -> String {
    let p = pats();
    let out = p.txid.replace_all(v, |m, _, _| pseudo_txid(m));
    let out = replace_amounts(&out, show_scale);
    replace_keyed_amounts(&out, show_scale)
}

fn public_safe_text(v: &str, show_scale: bool) -> String {
    let p = pats();
    let mut out = secret_floor_text(v);
    out = p.url.replace_with(&out, "[redacted-url]");
    out = p.email.replace_with(&out, "[redacted-email]");
    out = p.rate.replace_with(&out, "[redacted-rate]");
    out = p.txid.replace_all(&out, |m, _, _| pseudo_txid(m));
    out = replace_amounts(&out, show_scale);
    out = replace_keyed_amounts(&out, show_scale);
    out = p.bech32.replace_with(&out, "[redacted-address]");
    out = p.base58.replace_with(&out, "[redacted-address]");
    out = p.onion.replace_with(&out, "[redacted-onion]");
    p.path.replace_all(&out, |m, _, _| {
        let first = m.chars().next().unwrap_or('/');
        if first == '/' {
            "[redacted-path]".into()
        } else {
            format!("{first}[redacted-path]")
        }
    })
}

pub fn text_for_mode(v: &str, o: &Opts) -> String {
    let show = !o.mask_amounts;
    match o.mode {
        Mode::PublicSafe => public_safe_text(v, show),
        Mode::HighSignal => high_signal_text(&secret_floor_text(v), show),
    }
}

/// `redactField`.
pub fn field(ty: &str, value: &Value, o: &Opts) -> (String, Value) {
    let show = !o.mask_amounts;
    if SECRET_FLOOR_TYPES.contains(&ty) {
        return ("text".into(), Value::String(stable_masked_value(ty, value)));
    }
    if ty == "txid" {
        return (
            "text".into(),
            Value::String(pseudo_txid(&match value {
                Value::Null => String::new(),
                v => stringify(v),
            })),
        );
    }
    if ty == "amount" {
        let (n, u) = parse_typed_amount(value);
        return ("text".into(), Value::String(pseudo_amount(&n, &u, show)));
    }
    if ty == "text" {
        return (
            ty.into(),
            Value::String(text_for_mode(&stringify(value), o)),
        );
    }
    if OPERATIONAL_TYPES.contains(&ty) {
        if o.mode == Mode::PublicSafe {
            return ("text".into(), Value::String(stable_masked_value(ty, value)));
        }
        return (
            ty.into(),
            Value::String(text_for_mode(&stringify(value), o)),
        );
    }
    if o.mode == Mode::HighSignal {
        if let Value::String(s) = value {
            return (ty.into(), Value::String(text_for_mode(s, o)));
        }
        return (ty.into(), value.clone());
    }
    (
        ty.into(),
        Value::String(text_for_mode(&stringify(value), o)),
    )
}

/// `redactedFieldName`.
pub fn field_name(name: &str, ty: &str) -> String {
    if ty == "amount" || (!SECRET_FLOOR_TYPES.contains(&ty) && !OPERATIONAL_TYPES.contains(&ty)) {
        return name.to_owned();
    }
    if matches!(ty, "xpub" | "xpriv" | "descriptor") {
        return "wallet_material".into();
    }
    ty.to_owned()
}

#[cfg(test)]
mod corpus_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hashes_and_amounts() {
        // FNV-1a test vector.
        assert_eq!(stable_hash("a"), "e40c292c");
        assert_eq!(amount_magnitude("0.0123"), "~0.01");
        assert_eq!(amount_magnitude("12345678"), "~1e7");
        assert_eq!(amount_magnitude("250"), "~100");
        let o = Opts::screen(true, false);
        let tx = "ab".repeat(32);
        let out = text_for_mode(&format!("sent 0.5 BTC in {tx}"), &o);
        assert!(
            out.contains("txid#") && out.contains("amount#") && out.contains("(~0.1 BTC)"),
            "{out}"
        );
        assert!(!out.contains(&tx));
        let out = text_for_mode("fee_msat=1000 at https://example.com/x", &o);
        assert!(
            out.contains("fee_msat=amount#") && out.contains("[redacted-url]"),
            "{out}"
        );
        let out = text_for_mode("kind=ui.activity.history request_id=7", &o);
        assert_eq!(out, "kind=ui.activity.history request_id=7");
        // The backend floor redacts a keyed secret to the end of its value run.
        assert_eq!(
            secret_floor_text("password=hunter2 ok"),
            crate::secret_floor::floor::redact_secret_text("password=hunter2 ok")
        );
        assert!(!secret_floor_text("password=hunter2 ok").contains("hunter2"));
        assert_eq!(
            field("address", &json!("bcrt1qxyzxyzxyzxyzxyzxyzxyz"), &o).1,
            json!("bcrt1...zxyz")
        );
        assert_eq!(field_name("w", "xpub"), "wallet_material");
    }

    /// The floor never redacts less than the backend: every case in the envelope crate's scanner
    /// corpus keeps at least as many redaction markers as the backend's own output.
    #[test]
    fn floor_covers_the_backend_scanner_corpus() {
        let families: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../envelope/tests/data/scanner_families.json"
        ))
        .unwrap();
        let mut cases = 0;
        for (family, list) in families.as_object().unwrap() {
            for case in list.as_array().unwrap() {
                let (input, backend) = (case[0].as_str().unwrap(), case[1].as_str().unwrap());
                let ours = secret_floor_text(input);
                let markers = |s: &str| s.matches("[redacted").count();
                assert!(
                    markers(&ours) >= markers(backend),
                    "{family}: {input:?}\n  backend {backend:?}\n  desktop {ours:?}"
                );
                cases += 1;
            }
        }
        assert!(cases > 50, "corpus too small: {cases}");
        // Missed by the earlier approximation, whose secret words did not include `pwd`.
        assert_eq!(
            secret_floor_text("newPWDb64=Pw9Leak"),
            "newPWDb64=[redacted]"
        );
    }
}
