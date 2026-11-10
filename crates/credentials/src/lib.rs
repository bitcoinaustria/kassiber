//! Withholds the credentials a request carried from text a server or tool
//! sends back, as `kassiber/redaction.py` does, byte for byte.
//!
//! Pattern redaction cannot recognize a bare API key, macaroon, rune or
//! password. A caller that still holds the credentials it sent lists their
//! spellings with [`credential_forms`] and replaces each one in the
//! untrusted text with `<redacted>` ([`withhold`]) before the text is cut
//! short or reaches an error, a log, an AI projection or the book.
//!
//! The helpers that list what a request carries ([`authorization_credentials`],
//! [`url_credentials`], [`header_credentials`], [`backend_credentials`])
//! mirror CPython 3.11's `urllib.parse.urlsplit`, `unquote`, `parse_qsl`,
//! `quote`/`quote_plus`, `base64.b64decode(validate=True)`, `str.strip`,
//! `str.lower` and `json.dumps` string escaping, so both runtimes withhold
//! the same spellings. `tests/data/vectors.json` holds the Python outputs.

use kassiber_pyjson::{dumps_str_into, Options, PyStr};
use kassiber_pytext::{lower, strip};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// What a withheld credential reads as.
pub const WITHHELD: &str = "<redacted>";

/// `SENSITIVE_KEY_PARTS` in `kassiber/redaction.py`.
const SENSITIVE_KEY_PARTS: [&str; 26] = [
    "api_key",
    "api-key",
    "auth_header",
    "auth-header",
    "auth_response",
    "blinding",
    "cookie",
    "descriptor",
    "mnemonic",
    "password",
    "passphrase",
    "private",
    "recovery",
    "secret",
    "seed",
    "scan_key",
    "scankey",
    "scan_private",
    "scanprivate",
    "silent_payment",
    "silentpayment",
    "sp_descriptor",
    "spscan",
    "spspend",
    "token",
    "xprv",
];

/// Request headers whose whole value authenticates the request.
const CREDENTIAL_HEADERS: [&str; 9] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "grpc-metadata-macaroon",
    "api-key",
    "x-api-key",
    "x-mbx-apikey",
    "cb-access-key",
    "rune",
];
const QUERY_CREDENTIAL_PARTS: [&str; 4] = ["key", "auth", "sig", "access"];
const USERNAME_FIELDS: [&str; 3] = ["username", "rpcuser", "rpc_user"];
const PASSWORD_FIELDS: [&str; 3] = ["password", "rpcpassword", "rpc_password"];

/// `is_sensitive_key`: a key name that holds a secret.
pub fn is_sensitive_key(key: &str) -> bool {
    let lowered = lower(key).replace('-', "_");
    SENSITIVE_KEY_PARTS
        .iter()
        .any(|part| lowered.contains(&part.replace('-', "_")))
}

/// Every spelling of a set of credentials the scrub withholds, longest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Forms(Vec<String>);

impl Forms {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

/// `credential_forms`: each non-empty credential as sent, percent-encoded
/// (`quote` and `quote_plus` with `safe=""`) and JSON-escaped (with and
/// without `\/`), sorted by length in code points (longest first), then text.
pub fn credential_forms<I, S>(credentials: I) -> Forms
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut forms = BTreeSet::new();
    for credential in credentials {
        let credential = credential.as_ref();
        if credential.is_empty() {
            continue;
        }
        let escaped = json_escaped(credential);
        forms.insert(credential.to_owned());
        forms.insert(quote(credential, false));
        forms.insert(quote(credential, true));
        forms.insert(escaped.replace('/', "\\/"));
        forms.insert(escaped);
    }
    let mut forms: Vec<String> = forms.into_iter().collect();
    // Python compares str by code point, which is UTF-8 byte order.
    forms.sort_by(|a, b| {
        b.chars()
            .count()
            .cmp(&a.chars().count())
            .then_with(|| a.cmp(b))
    });
    Forms(forms)
}

fn json_escaped(text: &str) -> String {
    let mut out = String::new();
    dumps_str_into(text, &Options::new(), &mut out);
    out[1..out.len() - 1].to_owned()
}

/// `urllib.parse.quote(text, safe="")`, or `quote_plus` when `plus`.
fn quote(text: &str, plus: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(char::from(byte))
            }
            b' ' if plus => out.push('+'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

fn is_hex(form: &[u32]) -> bool {
    !form.is_empty()
        && form
            .iter()
            .all(|&p| char::from_u32(p).is_some_and(|c| c.is_ascii_hexdigit()))
}

fn points(text: &str) -> Vec<u32> {
    text.chars().map(u32::from).collect()
}

fn same_point(text: u32, form: u32, fold: bool) -> bool {
    text == form
        || (fold && form < 0x80 && text < 0x80 && (text as u8).eq_ignore_ascii_case(&(form as u8)))
}

/// One form replaced leftmost-first without overlaps, as `str.replace`
/// (or, for a hex form, `re.sub(..., flags=re.IGNORECASE | re.ASCII)`).
fn replace_form(text: &[u32], form: &[u32], replacement: &[u32]) -> Vec<u32> {
    let fold = is_hex(form);
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if text.len() - at >= form.len()
            && text[at..at + form.len()]
                .iter()
                .zip(form)
                .all(|(&t, &f)| same_point(t, f, fold))
        {
            out.extend_from_slice(replacement);
            at += form.len();
        } else {
            out.push(text[at]);
            at += 1;
        }
    }
    out
}

/// `_withhold_forms` over code points, which may include surrogates.
pub fn withhold_points(text: &[u32], forms: &Forms) -> Vec<u32> {
    let replacement = points(WITHHELD);
    let mut text = text.to_vec();
    for form in &forms.0 {
        text = replace_form(&text, &points(form), &replacement);
    }
    text
}

/// `_withhold_forms`: replace every form in `text` with `<redacted>`.
pub fn withhold(text: &str, forms: &Forms) -> String {
    if forms.is_empty() {
        return text.to_owned();
    }
    withhold_points(&points(text), forms)
        .into_iter()
        .filter_map(char::from_u32)
        .collect()
}

/// [`withhold`] for a Python string that may hold surrogates.
pub fn withhold_py(text: &PyStr, forms: &Forms) -> PyStr {
    if forms.is_empty() {
        return text.clone();
    }
    let scrubbed = withhold_points(&text.code_points().collect::<Vec<_>>(), forms);
    // Forms and the replacement are valid code points, as were the input's.
    PyStr::from_code_points(scrubbed).unwrap_or_else(|_| text.clone())
}

/// `withhold_credentials(text, credentials)`.
pub fn withhold_credentials<I, S>(text: &str, credentials: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    withhold(text, &credential_forms(credentials))
}

fn partition(text: &str, separator: char) -> (&str, bool, &str) {
    match text.split_once(separator) {
        Some((head, tail)) => (head, true, tail),
        None => (text, false, ""),
    }
}

fn rpartition(text: &str, separator: char) -> (&str, bool, &str) {
    match text.rsplit_once(separator) {
        Some((head, tail)) => (head, true, tail),
        None => ("", false, text),
    }
}

/// `authorization_credentials`: the whole header value, the part after its
/// scheme and, for `Basic`, the decoded `user:password` and the password.
pub fn authorization_credentials(value: &str) -> Vec<String> {
    let text = strip(value);
    if text.is_empty() {
        return Vec::new();
    }
    let mut found = vec![text.to_owned()];
    let (scheme, _, rest) = partition(text, ' ');
    let rest = strip(rest);
    if !rest.is_empty() {
        found.push(rest.to_owned());
        if lower(scheme) == "basic" {
            if let Some(decoded) = b64decode_validate(rest).and_then(|b| String::from_utf8(b).ok())
            {
                if !decoded.is_empty() {
                    let (_, separator, password) = partition(&decoded, ':');
                    let password = password.to_owned();
                    found.push(decoded.clone());
                    if separator && !password.is_empty() {
                        found.push(password);
                    }
                }
            }
        }
    }
    found
}

/// `basic_credentials`: the password, `user:password` and its base64.
pub fn basic_credentials(username: &str, password: &str) -> Vec<String> {
    if password.is_empty() {
        return Vec::new();
    }
    let pair = format!("{username}:{password}");
    let encoded = b64encode(pair.as_bytes());
    vec![password.to_owned(), pair, encoded]
}

/// `url_credentials`: userinfo and credential-like query values of a URL.
pub fn url_credentials(url: &str, default_scheme: Option<&str>) -> Vec<String> {
    let raw = strip(url);
    if raw.is_empty() {
        return Vec::new();
    }
    let raw = match default_scheme {
        Some(scheme) if !scheme.is_empty() && !raw.contains("://") => format!("{scheme}://{raw}"),
        _ => raw.to_owned(),
    };
    let Some((netloc, query)) = urlsplit(&raw) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let (userinfo, at, _) = rpartition(&netloc, '@');
    if at && !userinfo.is_empty() {
        let (user, separator, password) = partition(userinfo, ':');
        if separator && !password.is_empty() {
            found.push(userinfo.to_owned());
            found.extend(basic_credentials(&unquote(user), &unquote(password)));
            found.push(password.to_owned());
        } else if !separator {
            found.push(userinfo.to_owned());
            found.push(unquote(userinfo));
        }
    }
    for (key, item) in parse_qsl(&query) {
        let lowered = lower(&key);
        if is_sensitive_key(&key)
            || QUERY_CREDENTIAL_PARTS
                .iter()
                .any(|part| lowered.contains(part))
        {
            found.push(item);
        }
    }
    found.retain(|item| !item.is_empty());
    found
}

/// `header_credentials`: what the credential headers of a request carry.
pub fn header_credentials<K: AsRef<str>, V: AsRef<str>>(headers: &[(K, V)]) -> Vec<String> {
    let mut found = Vec::new();
    for (name, value) in headers {
        let lowered = lower(name.as_ref());
        if lowered == "authorization" || lowered == "proxy-authorization" {
            found.extend(authorization_credentials(value.as_ref()));
        } else if CREDENTIAL_HEADERS.contains(&lowered.as_str()) || is_sensitive_key(&lowered) {
            let text = strip(value.as_ref());
            if !text.is_empty() {
                found.push(text.to_owned());
            }
        }
    }
    found
}

/// `request_credentials`: every credential one outbound request carries.
pub fn request_credentials<K: AsRef<str>, V: AsRef<str>>(
    url: Option<&str>,
    headers: &[(K, V)],
    proxy_url: Option<&str>,
) -> Vec<String> {
    let mut found = url_credentials(url.unwrap_or(""), None);
    found.extend(header_credentials(headers));
    found.extend(url_credentials(proxy_url.unwrap_or(""), Some("socks5h")));
    found
}

/// `backend_credentials`. `text(key)` is the backend config's value as
/// Python's helper reads it: a string stripped, an int's digits, else "".
pub fn backend_credentials(text: impl Fn(&str) -> String) -> Vec<String> {
    let mut found = Vec::new();
    let token = text("token");
    if !token.is_empty() {
        found.push(token);
    }
    found.extend(authorization_credentials(&text("auth_header")));
    let users: Vec<String> = USERNAME_FIELDS
        .iter()
        .map(|k| text(k))
        .filter(|v| !v.is_empty())
        .collect();
    let passwords: Vec<String> = PASSWORD_FIELDS
        .iter()
        .map(|k| text(k))
        .filter(|v| !v.is_empty())
        .collect();
    let users = if users.is_empty() {
        vec![String::new()]
    } else {
        users
    };
    for user in &users {
        for password in &passwords {
            found.extend(basic_credentials(user, password));
        }
    }
    found.extend(url_credentials(&text("url"), None));
    for key in ["tor_proxy", "proxy"] {
        found.extend(url_credentials(&text(key), Some("socks5h")));
    }
    found.retain(|item| !item.is_empty());
    found
}

/// Code points whose NFKC form holds `/`, `?`, `#`, `@` or `:` (Unicode
/// 14.0, CPython 3.11's `unicodedata`), which `_checknetloc` refuses.
const NFKC_SEPARATORS: [u32; 19] = [
    0x2047, 0x2048, 0x2049, 0x2100, 0x2101, 0x2105, 0x2106, 0x2a74, 0xfe13, 0xfe16, 0xfe55, 0xfe56,
    0xfe5f, 0xfe6b, 0xff03, 0xff0f, 0xff1a, 0xff1f, 0xff20,
];

/// `urllib.parse.urlsplit(url)`'s netloc and query, or `None` where it
/// raises `ValueError`.
fn urlsplit(url: &str) -> Option<(String, String)> {
    let url: String = url
        .trim_start_matches(|c: char| u32::from(c) <= 0x20)
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut rest = url.as_str();
    if let Some(i) = rest.find(':') {
        let first = rest.chars().next();
        if i > 0
            && first.is_some_and(|c| c.is_ascii_alphabetic())
            && rest[..i]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            rest = &rest[i + 1..];
        }
    }
    let mut netloc = String::new();
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        netloc = after[..end].to_owned();
        rest = &after[end..];
        let open = netloc.contains('[');
        let close = netloc.contains(']');
        if open != close {
            return None;
        }
        if open && close && !bracketed_netloc_ok(&netloc) {
            return None;
        }
    }
    if let Some((head, _)) = rest.split_once('#') {
        rest = head;
    }
    let query = rest
        .split_once('?')
        .map(|(_, q)| q.to_owned())
        .unwrap_or_default();
    if !netloc.is_ascii()
        && netloc
            .chars()
            .any(|c| NFKC_SEPARATORS.contains(&u32::from(c)))
    {
        return None;
    }
    Some((netloc, query))
}

fn bracketed_netloc_ok(netloc: &str) -> bool {
    let (_, _, host_and_port) = rpartition(netloc, '@');
    let (before, open, bracketed) = partition(host_and_port, '[');
    let hostname = if open {
        if !before.is_empty() {
            return false;
        }
        let (hostname, _, port) = partition(bracketed, ']');
        if !port.is_empty() && !port.starts_with(':') {
            return false;
        }
        hostname
    } else {
        partition(host_and_port, ':').0
    };
    if let Some(future) = hostname.strip_prefix('v') {
        // \Av[a-fA-F0-9]+\..+\Z
        let digits = future.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        let after = &future[digits..];
        return digits > 0
            && after.len() > 1
            && after.starts_with('.')
            && !after[1..].contains('\n');
    }
    // ipaddress.ip_address: only an IPv6 address may be bracketed.
    let (address, scoped, scope) = partition(hostname, '%');
    if scoped && (scope.is_empty() || scope.contains('%')) {
        return false;
    }
    address.parse::<std::net::Ipv6Addr>().is_ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|v| v as u8)
}

fn unquote_to_bytes(text: &str) -> Vec<u8> {
    let mut pieces = text.as_bytes().split(|&b| b == b'%');
    let mut out = pieces.next().unwrap_or_default().to_vec();
    for item in pieces {
        match (
            item.first().copied().and_then(hex_value),
            item.get(1).copied().and_then(hex_value),
        ) {
            (Some(high), Some(low)) => {
                out.push(high * 16 + low);
                out.extend_from_slice(&item[2..]);
            }
            _ => {
                out.push(b'%');
                out.extend_from_slice(item);
            }
        }
    }
    out
}

/// `urllib.parse.unquote(text)`: UTF-8, invalid sequences replaced.
fn unquote(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut run_start = 0;
    let mut in_ascii: Option<bool> = None;
    let flush = |out: &mut String, run: &str, ascii: bool| {
        if ascii {
            out.push_str(&String::from_utf8_lossy(&unquote_to_bytes(run)));
        } else {
            out.push_str(run);
        }
    };
    for (at, c) in text.char_indices() {
        let ascii = c.is_ascii();
        match in_ascii {
            Some(current) if current != ascii => {
                flush(&mut out, &text[run_start..at], current);
                run_start = at;
                in_ascii = Some(ascii);
            }
            None => in_ascii = Some(ascii),
            _ => {}
        }
    }
    if let Some(current) = in_ascii {
        flush(&mut out, &text[run_start..], current);
    }
    out
}

/// `urllib.parse.parse_qsl(query)` with its defaults.
fn parse_qsl(query: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if query.is_empty() {
        return pairs;
    }
    for field in query.split('&') {
        if field.is_empty() {
            continue;
        }
        let (name, _, value) = partition(field, '=');
        if !value.is_empty() {
            pairs.push((
                unquote(&name.replace('+', " ")),
                unquote(&value.replace('+', " ")),
            ));
        }
    }
    pairs
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(BASE64[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `base64.b64decode(text, validate=True)`, which CPython 3.11.17 runs as
/// binascii's strict `a2b_base64`; `None` where it raises.
fn b64decode_validate(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut accumulator = 0u32;
    let mut quad = 0;
    let mut pads = 0;
    let mut padding = false;
    for (at, &byte) in bytes.iter().enumerate() {
        if byte == b'=' {
            if quad == 0 && at == 0 {
                return None; // leading padding
            }
            padding = true;
            if quad >= 2 {
                pads += 1;
                if quad + pads >= 4 {
                    // Excess data after padding is refused.
                    return (at + 1 == bytes.len()).then_some(out);
                }
            } else if quad == 1 {
                pads += 1;
            }
            continue;
        }
        let value = BASE64.iter().position(|&c| c == byte)? as u32;
        if padding {
            return None; // discontinuous padding
        }
        accumulator = (accumulator << 6) | value;
        quad += 1;
        match quad {
            2 => out.push((accumulator >> 4) as u8),
            3 => out.push((accumulator >> 2) as u8),
            4 => {
                out.push(accumulator as u8);
                quad = 0;
                accumulator = 0;
            }
            _ => {}
        }
    }
    (quad == 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_forms_match_either_case_and_sort_deterministically() {
        let forms = credential_forms(["bb", "aa", "abc"]);
        assert_eq!(forms.as_slice(), ["abc", "aa", "bb"]);
        let macaroon = "0201abcdef";
        assert_eq!(
            withhold_credentials("x 0201ABCDEF y 0201abCDef", [macaroon]),
            "x <redacted> y <redacted>"
        );
    }

    #[test]
    fn base64_round_trips_and_refuses_bad_padding() {
        assert_eq!(b64encode(b"u:pw"), "dTpwdw==");
        assert_eq!(b64decode_validate("dTpwdw==").unwrap(), b"u:pw");
        for (text, decoded) in [
            ("YWJj=", &b"abc"[..]),
            ("YWJj===", b"abc"),
            ("YQ==", b"a"),
            ("", b""),
        ] {
            assert_eq!(b64decode_validate(text).unwrap(), decoded, "{text}");
        }
        for text in [
            "YQ=", "YWI", "Y*==", "=", "YQ==x", "YWI==", "YW=I", "YQ=Jj", "Y===", "YWJjZ",
        ] {
            assert!(b64decode_validate(text).is_none(), "{text}");
        }
    }
}
