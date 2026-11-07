//! The redaction rules of `kassiber/redaction.py`.
//!
//! The daemon passes error messages and hints through
//! [`redact_secret_text`] and error details through
//! [`redact_operational_value`] of [`redact_secret_value`] before they leave
//! the process. The secret floor ([`floor`], running the credential
//! [`scanner`]) mirrors Python with hand-written matchers and uses only the
//! standard library, so the desktop supervisor compiles the same source. The
//! operational patterns are Python's, with three translations so the
//! backtracking engine matches what Python's `re` matches on `str`:
//!
//! - `\b` is a boundary between a word character (a letter, a number, or
//!   `_`, as `str.isalnum` decides) and anything else;
//! - `\s` also matches `\x1c` to `\x1f`, which `str.isspace` accepts;
//! - `$` (no `MULTILINE`) also matches before a final newline.
//!
//! Character classes and IGNORECASE equivalences come from generated
//! CPython 3.11 / Unicode 14.0.0 tables. Boundary assertions never fold
//! case. Amount pseudonyms hash with a
//! salt drawn per process, as Python's do, so they agree only within one
//! process. A string holding surrogates is redacted with each surrogate
//! standing in as a private-use character, which every rule treats as
//! Python treats a surrogate (not a word, space, or digit character). If an
//! operational pattern exceeds the engine's backtracking limit, the whole
//! text becomes `[redacted]` rather than passing through unredacted.

pub mod floor;
pub mod scanner;

pub use floor::{is_sensitive_key, redact_secret_text, SENSITIVE_KEY_PARTS};

use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;

use fancy_regex::{Captures, Regex};
use kassiber_pyjson::{float_repr, Key, PyStr, Value};
use kassiber_pytext::{is_space, REGEX_ASCII_CASES, REGEX_DIGIT, REGEX_SPACE, REGEX_WORD};

/// The depth past which `redact_secret_value` stops descending.
const MAX_DEPTH: usize = 8;

/// Python's `\b` for `str` patterns.
const B: &str = r"\b";
/// Python's `\s` for `str` patterns.
const S: &str = r"\s";
/// Python's `$` without `MULTILINE`.
const END: &str = r"$";
const AMOUNT_UNITS: &str =
    "BTC|XBT|LBTC|sats?|msats?|EUR|USD|CHF|GBP|JPY|CAD|AUD|NZD|SEK|NOK|DKK|PLN|CZK|HUF";
const AMOUNT_NUMBER: &str = r"[+-]?(?:(?:\d{1,3}(?:[,_ .]\d{3})+)|\d+)(?:[.,]\d+)?";
const CURRENCY_SYMBOLS: &str = "€$£¥₿";

struct Patterns {
    txid: Regex,
    amount_id: Regex,
    keyed_amount: Regex,
    amount: Regex,
    amount_num: Regex,
    market_rate_pair_prefix: Regex,
    unit_spaces: Regex,
}

fn compile(pattern: &str) -> Regex {
    let pattern = python_pattern(pattern);
    match Regex::new(&pattern) {
        Ok(regex) => regex,
        Err(error) => panic!("redaction pattern {pattern:?} does not compile: {error}"),
    }
}

/// Translate this module's ASCII regex syntax, rather than enabling the
/// engine's IGNORECASE (which also folds boundary character classes).
/// SRE's ASCII literal/range equivalences include İ, ı, ſ, and K.
///
/// `kassiber-ai` compiles its Python patterns through this too, so `\w`
/// is Python's word class and `\Z` the end of the text.
pub fn python_pattern(pattern: &str) -> String {
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
                'w' => out.push_str(REGEX_WORD),
                'Z' => out.push_str(r"\z"),
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

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let n = AMOUNT_NUMBER;
        let u = AMOUNT_UNITS;
        let c = CURRENCY_SYMBOLS;
        Patterns {
            txid: compile(&format!(r"(?i){B}[0-9a-f]{{64,}}{B}")),
            amount_id: compile(r"(?i)(?<![A-Za-z0-9_-])(?:[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}|[0-9a-f]{32})(?![A-Za-z0-9_-])"),
            keyed_amount: compile(&format!(
                r#"(?i){B}([A-Za-z][A-Za-z0-9_]*(?:msats?|sats?)){B}(['"]?{S}*[:=]{S}*['"]?)([+-]?\d[\d.,_]*\d|[+-]?\d)"#
            )),
            amount: compile(&format!(
                r"(?i){B}(?:{u}){S}*{n}{B}|(?<![A-Za-z0-9]){n}{S}*(?:{u}){B}|[{c}]{S}*{n}{B}|(?<![A-Za-z0-9]){n}{S}*[{c}]"
            )),
            amount_num: compile(r"[+-]?\d[\d.,_ ]*\d|[+-]?\d"),
            market_rate_pair_prefix: compile(&format!(r"(?i)(?:^|{B})(?:{u}){END}")),
            unit_spaces: compile(&format!(r"{S}+")),
        }
    })
}

/// A rule failed (the backtracking limit); the caller withholds the text.
struct Withheld;

fn substitute(
    regex: &Regex,
    text: &str,
    mut replacement: impl FnMut(&Captures<'_, str>) -> String,
) -> Result<String, Withheld> {
    regex
        .try_replacen(text, 0, |caps: &Captures<'_, str>| replacement(caps))
        .map(Cow::into_owned)
        .map_err(|_| Withheld)
}

fn nth<'a>(caps: &Captures<'a, str>, index: usize) -> &'a str {
    caps.get(index).map_or("", |m| m.as_str())
}

/// FNV-1a over code points, as `_stable_hash` (and the webview's
/// `stableHash`) computes it.
fn stable_hash(text: &str) -> String {
    let mut hash: u32 = 0x811C_9DC5;
    for c in text.chars() {
        hash ^= u32::from(c);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// A per-process salt, as `secrets.token_hex(16)` is per Python process.
/// The standard library seeds `RandomState` from the operating system; two
/// keyed SipHash outputs give 128 unpredictable bits without another
/// dependency.
fn amount_salt() -> &'static str {
    static SALT: OnceLock<String> = OnceLock::new();
    SALT.get_or_init(|| {
        let mut salt = String::new();
        for round in 0u64..2 {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u64(round);
            salt.push_str(&format!("{:016x}", hasher.finish()));
        }
        salt
    })
}

fn amount_pseudonym(num: &str, unit: &str) -> String {
    let digits: String = num
        .chars()
        .filter(|c| !matches!(c, ',' | '_' | ' '))
        .collect();
    let key = format!("{}|{digits}|{}", amount_salt(), unit.to_lowercase());
    format!("amount#{}", stable_hash(&key))
}

fn rstrip(text: &str) -> &str {
    text.trim_end_matches(|c: char| is_space(u32::from(c)))
}

fn is_market_rate_tail(text: &str, start: usize) -> Result<bool, Withheld> {
    let before = &text[..start];
    let Some(previous) = before.chars().next_back() else {
        return Ok(false);
    };
    if !matches!(previous, '/' | '-') {
        return Ok(false);
    }
    let prefix = rstrip(&before[..before.len() - previous.len_utf8()]);
    patterns()
        .market_rate_pair_prefix
        .is_match(prefix)
        .map_err(|_| Withheld)
}

fn operational_text(value: &str) -> Result<String, Withheld> {
    let p = patterns();
    let text = substitute(&p.txid, value, |caps| {
        format!("txid#{}", stable_hash(&nth(caps, 0).to_lowercase()))
    })?;
    // Shield only the ID itself. An amount regex can start in an all-digit ID
    // tail and swallow a neighboring amount; exempting that whole match leaks
    // the amount. Scan the surrounding segments independently instead.
    let mut out = String::new();
    let mut cursor = 0;
    for identity in p.amount_id.find_iter(&text) {
        let identity = identity.map_err(|_| Withheld)?;
        out.push_str(&redact_segment(&text[cursor..identity.start()])?);
        out.push_str(identity.as_str());
        cursor = identity.end();
    }
    out.push_str(&redact_segment(&text[cursor..])?);
    Ok(out)
}

/// `_redact_segment`: standalone amounts, then keyed (`amount_sat=`) ones.
fn redact_segment(segment: &str) -> Result<String, Withheld> {
    let p = patterns();
    let mut failed = false;
    let text = substitute(&p.amount, segment, |caps| {
        let Some(whole) = caps.get(0) else {
            return String::new();
        };
        let token = whole.as_str();
        match is_market_rate_tail(segment, whole.start()) {
            Ok(true) => return token.to_owned(),
            Ok(false) => {}
            Err(Withheld) => {
                failed = true;
                return token.to_owned();
            }
        }
        let num = match p.amount_num.find(token) {
            Ok(Some(found)) => found.as_str(),
            Ok(None) => token,
            Err(_) => {
                failed = true;
                token
            }
        };
        let rest = token.replacen(num, "", 1);
        let unit = match p.unit_spaces.try_replacen(&rest, 0, "") {
            Ok(unit) => unit.into_owned(),
            Err(_) => {
                failed = true;
                rest
            }
        };
        amount_pseudonym(num, &unit)
    })?;
    if failed {
        return Err(Withheld);
    }
    substitute(&p.keyed_amount, &text, |caps| {
        let (key, sep, num) = (nth(caps, 1), nth(caps, 2), nth(caps, 3));
        let unit = if key.to_lowercase().contains("msat") {
            "msat"
        } else {
            "sat"
        };
        format!("{key}{sep}{}", amount_pseudonym(num, unit))
    })
}

fn secret_text(value: &str) -> Result<String, Withheld> {
    Ok(redact_secret_text(value))
}

/// Runs `rule` over a string that may hold surrogates. Each surrogate
/// stands in as a distinct private-use character the text does not hold,
/// then comes back wherever the rule kept it.
fn with_surrogates(text: &PyStr, rule: impl Fn(&str) -> Result<String, Withheld>) -> PyStr {
    let withheld = || PyStr::from("[redacted]");
    if let Some(plain) = text.as_str() {
        return rule(plain).map_or_else(|_| withheld(), PyStr::from);
    }
    let present: std::collections::HashSet<u32> = text.code_points().collect();
    let mut stand_ins = (0xF_0000u32..=0x10_FFFD).filter(|p| !present.contains(p));
    let mut originals = std::collections::HashMap::new();
    let mut masked = String::new();
    for point in text.code_points() {
        match char::from_u32(point) {
            Some(c) => masked.push(c),
            None => {
                let Some(stand_in) = stand_ins.next().and_then(char::from_u32) else {
                    return withheld();
                };
                originals.insert(stand_in, point);
                masked.push(stand_in);
            }
        }
    }
    let Ok(redacted) = rule(&masked) else {
        return withheld();
    };
    let points = redacted
        .chars()
        .map(|c| originals.get(&c).copied().unwrap_or(u32::from(c)));
    PyStr::from_code_points(points).unwrap_or_else(|_| withheld())
}

/// `redact_secret_text(value)` for a string that may hold surrogates.
pub fn redact_secret_py(value: &PyStr) -> PyStr {
    with_surrogates(value, secret_text)
}

/// `redact_operational_text(value)`: txids and unit-tagged amounts become
/// stable pseudonyms.
pub fn redact_operational_text(value: &str) -> String {
    operational_text(value).unwrap_or_else(|_| "[redacted]".to_owned())
}

/// `str(key)` for a dict key.
fn key_text(key: &Key) -> String {
    match key {
        Key::Null => "None".to_owned(),
        Key::Bool(true) => "True".to_owned(),
        Key::Bool(false) => "False".to_owned(),
        Key::Int(n) => n.to_string(),
        Key::Float(f) => float_repr(*f),
        Key::Str(text) => text.code_points().filter_map(char::from_u32).collect(),
    }
}

/// `redact_secret_value(value)`: sensitive keys become `[redacted]`, strings
/// pass through [`redact_secret_text`], and nesting past eight levels
/// becomes `[truncated]`.
pub fn redact_secret_value(value: &Value) -> Value {
    secret_value(value, 0)
}

fn secret_value(value: &Value, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return Value::Str(PyStr::from("[truncated]"));
    }
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .map(|(key, item)| {
                    let redacted = if is_sensitive_key(&key_text(key)) {
                        Value::Str(PyStr::from("[redacted]"))
                    } else {
                        secret_value(item, depth + 1)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::List(items) => Value::List(
            items
                .iter()
                .map(|item| secret_value(item, depth + 1))
                .collect(),
        ),
        Value::Str(text) => Value::Str(redact_secret_py(text)),
        other => other.clone(),
    }
}

/// `redact_operational_value(value)`: string leaves pass through
/// [`redact_operational_text`]; past eight levels values pass unchanged.
pub fn redact_operational_value(value: &Value) -> Value {
    operational_value(value, 0)
}

fn operational_value(value: &Value, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return value.clone();
    }
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .map(|(key, item)| (key.clone(), operational_value(item, depth + 1)))
                .collect(),
        ),
        Value::List(items) => Value::List(
            items
                .iter()
                .map(|item| operational_value(item, depth + 1))
                .collect(),
        ),
        Value::Str(text) => Value::Str(with_surrogates(text, operational_text)),
        other => other.clone(),
    }
}

/// The daemon's treatment of error details:
/// `redact_operational_value(redact_secret_value(details))`.
pub fn redact_error_details(details: &Value) -> Value {
    redact_operational_value(&redact_secret_value(details))
}

const TRACEBACK_CAP: usize = 8000;
const TRACEBACK_HEAD: usize = 1000;
const TRACEBACK_TAIL: usize = 7000;

/// `log_ring.relativize_path(path)`: keep what follows `site-packages/`, a
/// `kassiber/` or `tests/` tail, or `~` for the home directory; otherwise the
/// basename. Pure string logic.
pub fn relativize_path(path: &str) -> String {
    // os.path.expanduser("~"): HOME on POSIX, USERPROFILE on Windows.
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()
    } else {
        std::env::var("HOME").ok()
    };
    relativize_path_for(path, home.as_deref().unwrap_or("~"))
}

fn relativize_path_for(path: &str, home: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let norm = path.replace('\\', "/");
    if let Some(index) = norm.rfind("site-packages/") {
        return norm[index + "site-packages/".len()..].to_owned();
    }
    if norm.starts_with("kassiber/") || norm.starts_with("tests/") {
        return norm;
    }
    let best = ["/kassiber/", "/tests/"]
        .iter()
        .filter_map(|marker| norm.rfind(marker))
        .max();
    if let Some(index) = best {
        return norm[index + 1..].to_owned();
    }
    let home = home.replace('\\', "/");
    let home = home.trim_end_matches('/');
    if !home.is_empty() && home != "~" && norm.starts_with(&format!("{home}/")) {
        return format!("~{}", &norm[home.len()..]);
    }
    match norm.rsplit_once('/') {
        Some((_, tail)) => tail.to_owned(),
        None => norm,
    }
}

/// The macOS Touch ID helper reports failures as `kassiber native auth:
/// <text>`, which the credential scanner reads as an `auth: <secret>` pair.
/// Exactly these fixed texts, as the whole message, pass unchanged; anything
/// else after `auth:` is still redacted. `log_ring.NATIVE_AUTH_ERROR_*` in
/// Python keeps the same lists.
pub const NATIVE_AUTH_ERROR_PREFIX: &str = "kassiber native auth: ";
pub const NATIVE_AUTH_ERROR_MESSAGES: &[&str] = &[
    "missing native auth action",
    "missing native auth account",
    "unsupported native auth action",
    "native auth output must be an inherited pipe",
    "native auth secret exceeds 16 KiB",
    "native auth launch gate requires inherited pipes",
    "native auth launch gate received an invalid release",
    "missing --ready-fd",
    "missing --go-fd",
    "missing --output-fd",
    "missing --secret-fd",
    "invalid --ready-fd",
    "invalid --go-fd",
    "invalid --output-fd",
    "invalid --secret-fd",
    "Touch ID is unavailable on this platform",
    "native-auth helper is not inside a signed app bundle",
    "native-auth helper does not support this macOS architecture",
    "native-auth helper parent is not the bundled Kassiber sidecar",
    "native-auth helper and broker signatures do not match",
    "native-auth helper parent has an unexpected signing identifier",
    "native-auth sidecar signing identifier is invalid",
    "native-auth helper has an invalid TeamIdentifier",
    "could not inspect native-auth parent process",
    "native-auth code signature verification failed",
    "native-auth code signature inspection failed",
    "native-auth code signature has no Identifier",
    "native-auth helper requires a production TeamIdentifier",
    "database passphrase must not be empty",
    "Touch ID operator account is invalid.",
    "Touch ID operator unlock is only available in the macOS desktop app.",
    "This build cannot access the protected Keychain item; reopen a production-entitled build to manage it.",
    "No saved desktop biometric passphrase was found.",
    "LocalAuthentication is unavailable on this Mac",
    "Touch ID is not available or not enrolled for this Mac user.",
    "The credential remained in the native store after deletion.",
    "secret_ref_missing",
    "secret_store_locked_or_denied",
];
/// Texts that end in an OSStatus: `<head><signed 32-bit integer><tail>`.
pub const NATIVE_AUTH_ERROR_STATUS_TEMPLATES: &[(&str, &str)] = &[
    ("macOS protected Keychain rejected the operation (", ")."),
    ("could not resolve the running native-auth parent code (", ")"),
    ("could not compile the native-auth sidecar requirement (", ")"),
    ("running native-auth parent did not match the bundled sidecar requirement (", ")"),
];

fn is_status_code(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    (1..=10).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
}

/// `log_ring.is_known_native_auth_error`: whether `text` is, in full, one of
/// the Touch ID helper's fixed errors.
pub fn is_known_native_auth_error(text: &str) -> bool {
    let Some(message) = text.strip_prefix(NATIVE_AUTH_ERROR_PREFIX) else {
        return false;
    };
    NATIVE_AUTH_ERROR_MESSAGES.contains(&message)
        || NATIVE_AUTH_ERROR_STATUS_TEMPLATES.iter().any(|(head, tail)| {
            message
                .strip_prefix(head)
                .and_then(|rest| rest.strip_suffix(tail))
                .is_some_and(is_status_code)
        })
}

/// `log_ring.sanitize_traceback_text(text)`: relativize paths, redact
/// secrets and operational ids, and cap at 8,000 code points. A known Touch
/// ID helper error (`is_known_native_auth_error`) passes as is.
pub fn sanitize_traceback_text(text: &str) -> String {
    if is_known_native_auth_error(text) {
        return text.to_owned();
    }
    static PATH: OnceLock<Regex> = OnceLock::new();
    let path = PATH.get_or_init(|| {
        let segment = format!("(?:{REGEX_WORD}|[@.+ -])");
        Regex::new(&format!(
            r"(?:[A-Za-z]:[\\/]|[\\/]{{2}}|~|[\\/]){segment}*(?:[\\/]{segment}+)+"
        ))
        .expect("traceback path pattern")
    });
    let mut relativized = String::with_capacity(text.len());
    let mut end = 0;
    for found in path.find_iter(text) {
        let Ok(found) = found else {
            return "[redacted]".to_owned();
        };
        relativized.push_str(&text[end..found.start()]);
        relativized.push_str(&relativize_path(found.as_str()));
        end = found.end();
    }
    relativized.push_str(&text[end..]);
    let text = redact_operational_text(&redact_secret_text(&relativized));
    let count = text.chars().count();
    if count <= TRACEBACK_CAP {
        return text;
    }
    let head: String = text.chars().take(TRACEBACK_HEAD).collect();
    let tail: String = text.chars().skip(count - TRACEBACK_TAIL).collect();
    format!("{head}...[truncated]...{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_text_matches_python_examples() {
        // A plain value owns the rest of its line.
        assert_eq!(
            redact_secret_text("token=abc123 and password: hunter2"),
            "token=[redacted]"
        );
        assert_eq!(
            redact_secret_text("token=abc123\npassword: hunter2"),
            "token=[redacted]\npassword: [redacted]"
        );
        assert_eq!(
            redact_secret_text("Authorization: Bearer abc.def=="),
            "Authorization: [redacted]"
        );
        assert_eq!(
            redact_secret_text("seed phrase: abandon ability able, next"),
            "seed phrase: [redacted]"
        );
        assert_eq!(
            redact_secret_text("mnemonic=abandon ability able next=1"),
            "mnemonic=[redacted]"
        );
        assert_eq!(redact_secret_text("sk-abcdefgh"), "[redacted-secret]");
        assert_eq!(
            redact_secret_text("{\"api_key\": \"xyz\"}"),
            "{\"api_key\": \"[redacted]\"}"
        );
    }

    #[test]
    fn operational_text_pseudonymizes_txids_and_amounts() {
        let txid = "a".repeat(64);
        assert_eq!(
            redact_operational_text(&format!("tx {txid} done")),
            format!("tx txid#{} done", stable_hash(&txid))
        );
        let first = redact_operational_text("paid 5 BTC");
        assert!(first.starts_with("paid amount#"), "{first}");
        assert_eq!(first, redact_operational_text("paid 5 BTC"));
        assert_eq!(
            redact_operational_text("BTC/EUR 64000.12"),
            "BTC/EUR 64000.12"
        );
        assert!(redact_operational_text("amount_sat=50000").starts_with("amount_sat=amount#"));
    }

    #[test]
    fn surrogates_survive_and_are_not_word_characters() {
        let py = |head: u32, middle: &str, tail: u32| {
            PyStr::from_code_points(
                [head]
                    .into_iter()
                    .chain(middle.chars().map(u32::from))
                    .chain([tail]),
            )
            .unwrap()
        };
        assert_eq!(
            redact_secret_py(&py(0xD800, "token=abc\n", 0xDC00)),
            py(0xD800, "token=[redacted]\n", 0xDC00)
        );
    }

    #[test]
    fn traceback_text_matches_python_vectors() {
        // Generated by kassiber.log_ring with HOME=/home/alice.
        for (path, expected) in [
            ("/home/alice/a/b", "~/a/b"),
            ("/opt/x/tests/y.py", "tests/y.py"),
            ("kassiber/a", "kassiber/a"),
            ("/a/b/c", "c"),
            ("/home/alice", "alice"),
        ] {
            assert_eq!(relativize_path_for(path, "/home/alice"), expected);
        }
        for (text, expected) in [
            (
                r#"File "/usr/lib/python3/site-packages/kassiber/operator/service.py", line 3"#,
                r#"File "kassiber/operator/service.py", line 3"#,
            ),
            (
                r"C:\Users\John Doe\kassiber\x.py failed",
                "kassiber/x.py failed",
            ),
            (
                // Since the 5h/5i secret floor a value runs to the end of
                // the line, as kassiber.log_ring now redacts it.
                "passphrase=hunter2 token=abc /tmp/a b/c",
                "passphrase=[redacted]",
            ),
            ("~/notes/a.txt and //server/share/x", "a.txt and x"),
        ] {
            assert_eq!(sanitize_traceback_text(text), expected);
        }
        let long = format!("{}tail", "x".repeat(9000));
        let capped = sanitize_traceback_text(&long);
        assert_eq!(capped.chars().count(), 1000 + 17 + 7000);
        assert!(capped.starts_with(&"x".repeat(1000)));
        assert!(capped.contains("...[truncated]..."));
        assert!(capped.ends_with("xtail"));
    }
}
