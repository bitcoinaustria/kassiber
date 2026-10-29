//! Public-safe diagnostics: Python reference redaction and report summaries.
use fancy_regex::Regex;
use kassiber_envelope::{dict, py_str, s};
use kassiber_pyjson::{Key, Value};
use kassiber_pytext::{REGEX_ASCII_CASES, REGEX_DIGIT, REGEX_SPACE, REGEX_WORD};
use std::sync::OnceLock;
const _SENSITIVE_KEY_TOKENS: &[&str] = &[
    r###"address"###,
    r###"amount"###,
    r###"auth"###,
    r###"balance"###,
    r###"basis"###,
    r###"blinding"###,
    r###"code"###,
    r###"config"###,
    r###"cookie"###,
    r###"counterparty"###,
    r###"credential"###,
    r###"cursor"###,
    r###"date"###,
    r###"description"###,
    r###"descriptor"###,
    r###"end"###,
    r###"env"###,
    r###"external"###,
    r###"fee"###,
    r###"file"###,
    r###"fingerprint"###,
    r###"gain"###,
    r###"id"###,
    r###"key"###,
    r###"label"###,
    r###"loss"###,
    r###"mnemonic"###,
    r###"name"###,
    r###"note"###,
    r###"output"###,
    r###"passphrase"###,
    r###"password"###,
    r###"path"###,
    r###"price"###,
    r###"proceeds"###,
    r###"profile"###,
    r###"rate"###,
    r###"raw"###,
    r###"recovery"###,
    r###"ref"###,
    r###"secret"###,
    r###"seed"###,
    r###"source"###,
    r###"start"###,
    r###"store"###,
    r###"time"###,
    r###"timestamp"###,
    r###"token"###,
    r###"tx"###,
    r###"url"###,
    r###"user"###,
    r###"value"###,
    r###"wallet"###,
    r###"workspace"###,
];
const _SENSITIVE_EXACT_KEYS: &[&str] = &[
    r###"account"###,
    r###"asset"###,
    r###"backend"###,
    r###"pair_id"###,
    r###"payload"###,
    r###"tag"###,
    r###"transaction"###,
    r###"type"###,
];
const _SAFE_ENUM_KEYS: &[&str] = &[
    r###"all"###,
    r###"chain"###,
    r###"command"###,
    r###"debug"###,
    r###"dry_run"###,
    r###"format"###,
    r###"insecure"###,
    r###"kind"###,
    r###"machine"###,
    r###"network"###,
    r###"policy"###,
    r###"save"###,
    r###"sync_all"###,
];
const _SKIP_ARG_KEYS: &[&str] = &[
    r###"command"###,
    r###"data_root"###,
    r###"diagnostics_out"###,
    r###"env_file"###,
    r###"output"###,
    r###"runtime_config"###,
];
fn python_pattern(pattern: &str) -> String {
    let (ignore_case, pattern) = pattern
        .strip_prefix("(?i)")
        .map_or((false, pattern), |rest| (true, rest));
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    let mut in_class = false;
    while index < chars.len() {
        let c = chars[index];
        if c == '\\' {
            index += 1;
            match chars[index] {
                'b' => out.push_str(&format!(
                    "(?:(?<={REGEX_WORD})(?!{REGEX_WORD})|(?<!{REGEX_WORD})(?={REGEX_WORD}))"
                )),
                's' => out.push_str(REGEX_SPACE),
                'd' => out.push_str(REGEX_DIGIT),
                escaped => {
                    out.push('\\');
                    out.push(escaped);
                    if escaped == 'x' {
                        out.extend(&chars[index + 1..index + 3]);
                        index += 2;
                    }
                }
            }
        } else if c == '(' && chars[index..].starts_with(&['(', '?', 'P', '<']) {
            // Group names are syntax, not case-insensitive literals.
            while chars[index] != '>' {
                out.push(chars[index]);
                index += 1;
            }
            out.push('>');
        } else if c == '[' {
            in_class = true;
            out.push(c);
        } else if c == ']' {
            in_class = false;
            out.push(c);
        } else if c == '$' && !in_class {
            out.push_str(r"(?=\n?\z)");
        } else if ignore_case && c.is_ascii_alphabetic() {
            if !in_class {
                out.push('[');
            }
            let mut end = c;
            if in_class && chars.get(index + 1) == Some(&'-') {
                if let Some(&last) = chars
                    .get(index + 2)
                    .filter(|last| last.is_ascii_alphabetic())
                {
                    end = last;
                    index += 2;
                }
            }
            for letter in c..=end {
                out.push_str(
                    REGEX_ASCII_CASES[letter.to_ascii_lowercase() as usize - 'a' as usize],
                );
            }
            if !in_class {
                out.push(']');
            }
        } else {
            out.push(c);
        }
        index += 1;
    }
    out
}

fn patterns() -> &'static Vec<(Regex, &'static str)> {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| vec![
(Regex::new(&python_pattern(r###"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{6,}"###)).expect("reference regex"), r###"Bearer <redacted>"###),
(Regex::new(&python_pattern(r###"\bsk-[A-Za-z0-9._-]{6,}\b"###)).expect("reference regex"), r###"<redacted-secret>"###),
(Regex::new(&python_pattern(r###"(?i)\b(?P<key>[A-Za-z0-9_.-]*(?:mnemonic|recovery[_-]?phrase|seed(?:[_-]?(?:phrase|words))?)[A-Za-z0-9_.-]*)(?P<sep>\s*[:=]\s*)(?P<value>[^,;}{]*?)(?=(?:\s+[A-Za-z0-9_.-]+\s*[:=])|[,;}'\"]|$)"###)).expect("reference regex"), r###"$key${sep}<redacted>"###),
(Regex::new(&python_pattern(r###"(?i)\b(?P<key>[A-Za-z0-9_.-]*(?:api[_-]?key|auth[_-]?header|cookie|mnemonic|passphrase|password|recovery[_-]?phrase|secret|seed(?:[_-]?(?:phrase|words))?|token)[A-Za-z0-9_.-]*)(?P<sep>\s*[:=]\s*)(?P<value>[^\s,;]+)"###)).expect("reference regex"), r###"$key${sep}<redacted>"###),
(Regex::new(&python_pattern(r###"(?i)\b[a-z][a-z0-9+.-]*://[^\s'\"<>]+"###)).expect("reference regex"), r###"<url>"###),
(Regex::new(&python_pattern(r###"(?i)\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:Z|[+-]\d{2}:\d{2})?\b"###)).expect("reference regex"), r###"<timestamp>"###),
(Regex::new(&python_pattern(r###"\b[0-9a-fA-F]{64}\b"###)).expect("reference regex"), r###"<hex64>"###),
(Regex::new(&python_pattern(r###"(?i)\b(?:xpub|ypub|zpub|tpub|upub|vpub|xprv|yprv|zprv|tprv|uprv|vprv)[A-Za-z0-9]{20,}\b"###)).expect("reference regex"), r###"<xkey>"###),
(Regex::new(&python_pattern(r###"(?i)\bsp\([^)\n]{8,}\)"###)).expect("reference regex"), r###"<silent-payment>"###),
(Regex::new(&python_pattern(r###"(?i)\b(?:t?spscan|t?spspend|t?sp)1q[023456789acdefghjklmnpqrstuvwxyz]{8,}\b"###)).expect("reference regex"), r###"<silent-payment>"###),
(Regex::new(&python_pattern(r###"(?i)\b(?:bc1|tb1|bcrt1|lq1|ex1)[0-9a-z]{8,90}\b"###)).expect("reference regex"), r###"<address>"###),
(Regex::new(&python_pattern(r###"(?P<path>(?:/[A-Za-z0-9_.@+-][^\s'\":)]*)|(?:[A-Za-z]:\\[^\s'\":)]+))"###)).expect("reference regex"), r###"<path>"###),
(Regex::new(&python_pattern(r###"(?i)(?<![A-Za-z0-9_])[-+]?\d+(?:\.\d+)?\s*(?:msat|sats?|btc|lbtc|usd|eur)\b"###)).expect("reference regex"), r###"<amount>"###),
(Regex::new(&python_pattern(r###"(?<![A-Za-z_])[-+]?\d+(?:\.\d+)?(?![A-Za-z_])"###)).expect("reference regex"), r###"<number>"###),
 ])
}
/// Apply all fourteen redaction rules in Python's order.
pub fn sanitize_text(text: &str) -> String {
    let mut text = text.to_owned();
    for (regex, replacement) in patterns() {
        let mut failed = false;
        let mut out = String::new();
        let mut end = 0;
        for capture in regex.captures_iter(&text) {
            match capture {
                Ok(caps) => {
                    let m = caps.get(0).expect("whole match");
                    out.push_str(&text[end..m.start()]);
                    if replacement.starts_with("$key") {
                        out.push_str(caps.name("key").expect("key").as_str());
                        out.push_str(caps.name("sep").expect("sep").as_str());
                        out.push_str("<redacted>");
                    } else {
                        out.push_str(replacement);
                    }
                    end = m.end();
                }
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            return "<redacted>".into();
        }
        out.push_str(&text[end..]);
        text = out;
    }
    text
}
pub fn is_sensitive_key(key: &str) -> bool {
    let lowered = kassiber_pytext::lower(key).replace('-', "_");
    _SENSITIVE_EXACT_KEYS.contains(&lowered.as_str())
        || lowered
            .split('_')
            .any(|p| _SENSITIVE_KEY_TOKENS.contains(&p))
}
/// Preserve lone surrogates with non-word stand-ins while applying CPython regexes.
pub fn sanitize_py(text: &kassiber_pyjson::PyStr) -> kassiber_pyjson::PyStr {
    if let Some(text) = text.as_str() {
        return sanitize_text(text).into();
    }
    let present: std::collections::HashSet<_> = text.code_points().collect();
    let mut stand_ins = (0xF_0000u32..=0x10_FFFD).filter(|p| !present.contains(p));
    let mut originals = std::collections::HashMap::new();
    let mut masked = String::new();
    for point in text.code_points() {
        if let Some(c) = char::from_u32(point) {
            masked.push(c);
        } else {
            let Some(c) = stand_ins.next().and_then(char::from_u32) else {
                return "<redacted>".into();
            };
            originals.insert(c, point);
            masked.push(c);
        }
    }
    let redacted = sanitize_text(&masked);
    kassiber_pyjson::PyStr::from_code_points(
        redacted
            .chars()
            .map(|c| originals.get(&c).copied().unwrap_or(u32::from(c))),
    )
    .expect("valid original code points")
}
fn key_text(key: &Key) -> kassiber_pyjson::PyStr {
    match key {
        Key::Str(v) => v.clone(),
        Key::Null => "None".into(),
        Key::Bool(v) => py_str(&Value::Bool(*v)).into(),
        Key::Int(v) => v.to_string().into(),
        Key::Float(v) => py_str(&Value::Float(*v)).into(),
    }
}
fn redacted_shape(value: &Value) -> Value {
    match value {
        Value::Dict(pairs) => {
            let mut keys = pairs
                .iter()
                .map(|(k, _)| sanitize_py(&key_text(k)))
                .collect::<Vec<_>>();
            keys.sort();
            dict([(
                "keys",
                Value::List(keys.into_iter().map(Value::Str).collect()),
            )])
        }
        Value::List(items) => dict([("items", Value::Int(items.len().into()))]),
        Value::Null => Value::Null,
        _ => s("<redacted>"),
    }
}
pub fn sanitize_details(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) => value.clone(),
        Value::Dict(pairs) => {
            let mut sanitized: Vec<(Key, Value)> = Vec::new();
            for (key, value) in pairs {
                let key = sanitize_py(&key_text(key));
                let scalar_key = key
                    .code_points()
                    .map(|c| char::from_u32(c).unwrap_or(''))
                    .collect::<String>();
                let value = if is_sensitive_key(&scalar_key) {
                    redacted_shape(value)
                } else {
                    sanitize_details(value)
                };
                let key = Key::Str(key);
                if let Some((_, old)) = sanitized.iter_mut().find(|(k, _)| k == &key) {
                    *old = value;
                } else {
                    sanitized.push((key, value));
                }
            }
            Value::Dict(sanitized)
        }
        Value::List(items) => dict([("items", Value::Int(items.len().into()))]),
        Value::Int(_) | Value::Float(_) => s("<number>"),
        Value::Str(text) => Value::Str(sanitize_py(text)),
        _ => s(sanitize_text(&py_str(value))),
    }
}
pub mod report;
pub mod state;
