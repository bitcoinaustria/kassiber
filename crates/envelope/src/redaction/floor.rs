//! The secret floor of `kassiber/redaction.py`: `redact_secret_text` and
//! `is_sensitive_key`.
//!
//! The credential scanner runs first; the key, descriptor and token patterns
//! follow in Python's order. Each pattern is written out as a matcher that
//! accepts what Python's `re` accepts (all of them are `re.ASCII`), scanning
//! left to right as `re.sub` does. Run ends only move forward, so a record of
//! repeated openers stays linear.
//!
//! This file uses only the standard library: the desktop supervisor
//! (`ui-tauri/src-tauri/src/supervisor.rs`) compiles the same source.

use super::scanner::redact_credential_chars;

/// `SENSITIVE_KEY_PARTS`.
pub const SENSITIVE_KEY_PARTS: &[&str] = &[
    "api_key",
    "api-key",
    "authorization",
    "auth_header",
    "auth-header",
    "auth_response",
    "blinding",
    "cookie",
    "descriptor",
    "mnemonic",
    "password",
    "rpcauth",
    "rpc_auth",
    "rune",
    "macaroon",
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

/// `is_sensitive_key(key)`.
pub fn is_sensitive_key(key: &str) -> bool {
    let lowered = key.to_lowercase().replace('-', "_");
    SENSITIVE_KEY_PARTS
        .iter()
        .any(|part| lowered.contains(&part.replace('-', "_")))
}

/// `redact_secret_text(value)`.
pub fn redact_secret_text(value: &str) -> String {
    let text: Vec<char> = value.chars().collect();
    redact_secret_chars(&text).into_iter().collect()
}

/// [`redact_secret_text`] over code points.
pub fn redact_secret_chars(value: &[char]) -> Vec<char> {
    let mut text = redact_credential_chars(value);
    let rules: [(Rule, &str); 10] = [
        (private_key, "[redacted-private-key]"),
        (nostr_secret, "[redacted-private-key]"),
        (wif, "[redacted-private-key]"),
        (extended_key, "[redacted-extended-key]"),
        (sp_descriptor, "[redacted-silent-payment-descriptor]"),
        (sp_key, "[redacted-silent-payment-key]"),
        (sp_address, "[redacted-silent-payment-address]"),
        (descriptor, "[redacted-descriptor]"),
        (bearer, "Bearer [redacted]"),
        (sk_secret, "[redacted-secret]"),
    ];
    for (rule, replacement) in rules {
        text = substitute(&text, rule, replacement);
    }
    text
}

/// A pattern at a position: the match end. `Runs` carries forward-only scan
/// state between positions of one pass.
type Rule = fn(&[char], usize, &mut Runs) -> Option<usize>;

/// `re.sub(pattern, replacement, text)` for a pattern that never matches
/// empty.
fn substitute(text: &[char], rule: Rule, replacement: &str) -> Vec<char> {
    let mut runs = Runs::default();
    let mut out = Vec::with_capacity(text.len());
    let mut pos = 0;
    while pos < text.len() {
        match rule(text, pos, &mut runs) {
            Some(end) => {
                out.extend(replacement.chars());
                pos = end;
            }
            None => {
                out.push(text[pos]);
                pos += 1;
            }
        }
    }
    out
}

/// Forward-only lookups for one pass: the next `)` or newline, and the end
/// of a run of `sk-` token characters with its last non-word character.
#[derive(Default)]
struct Runs {
    stop: Option<(usize, usize)>,
    token_end: usize,
    token_run: Option<(usize, Option<usize>)>,
}

fn is_stop(c: char) -> bool {
    matches!(c, ')' | '\n')
}

impl Runs {
    /// The first `)` or `\n` at or after `from`. Openers differ in length, so
    /// a query may start a few characters before the previous one.
    fn stop(&mut self, text: &[char], from: usize) -> usize {
        if let Some((searched, stop)) = self.stop {
            if from < searched {
                return (from..searched).find(|&i| is_stop(text[i])).unwrap_or(stop);
            }
            if from <= stop {
                return stop;
            }
        }
        let stop = (from..text.len())
            .find(|&i| is_stop(text[i]))
            .unwrap_or(text.len());
        self.stop = Some((from, stop));
        stop
    }

    /// The end of the `[A-Za-z0-9._~+/-]` run holding `from`, and the last
    /// non-word character in it.
    fn token(&mut self, text: &[char], from: usize) -> (usize, Option<usize>) {
        if let Some((end, last)) = self.token_run {
            if from <= end && from >= self.token_end {
                return (end, last);
            }
        }
        let mut end = from;
        let mut last = None;
        while end < text.len() && is_token(text[end]) {
            if !is_word(text[end]) {
                last = Some(end);
            }
            end += 1;
        }
        self.token_end = from;
        self.token_run = Some((end, last));
        (end, last)
    }
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_token(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '+' | '/' | '-')
}

/// `(?<![A-Za-z0-9_])` at `pos`.
fn word_start(text: &[char], pos: usize) -> bool {
    pos == 0 || !is_word(text[pos - 1])
}

/// `(?![A-Za-z0-9_])` at `pos`.
fn word_end(text: &[char], pos: usize) -> bool {
    pos == text.len() || !is_word(text[pos])
}

/// An ASCII case-insensitive literal at `pos`.
fn literal_ci(text: &[char], pos: usize, word: &str) -> bool {
    word.chars().enumerate().all(|(k, w)| {
        text.get(pos + k)
            .is_some_and(|c| c.eq_ignore_ascii_case(&w))
    })
}

fn run(text: &[char], from: usize, class: impl Fn(char) -> bool) -> usize {
    let mut end = from;
    while end < text.len() && class(text[end]) {
        end += 1;
    }
    end
}

/// The first of `prefixes` at `pos`, case-insensitively: its end.
fn prefix_ci(text: &[char], pos: usize, prefixes: &[&str]) -> Option<usize> {
    prefixes
        .iter()
        .find(|prefix| literal_ci(text, pos, prefix))
        .map(|prefix| pos + prefix.len())
}

fn is_base58(c: char) -> bool {
    matches!(c, '1'..='9' | 'A'..='H' | 'J'..='N' | 'P'..='Z' | 'a'..='k' | 'm'..='z')
}

/// `[023456789acdefghjklmnpqrstuvwxyz]` under IGNORECASE.
fn is_bech32_ci(c: char) -> bool {
    let c = c.to_ascii_lowercase();
    c.is_ascii() && "023456789acdefghjklmnpqrstuvwxyz".contains(c)
}

/// A maximal run of `class` from `from`, at least `min` long, ending on a
/// word boundary.
fn bounded_run(
    text: &[char],
    from: usize,
    min: usize,
    class: impl Fn(char) -> bool,
) -> Option<usize> {
    let end = run(text, from, class);
    (end - from >= min && word_end(text, end)).then_some(end)
}

/// `(?:xprv|tprv|yprv|zprv|uprv|vprv)[A-Za-z0-9]{16,}`, IGNORECASE.
fn private_key(text: &[char], pos: usize, _: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) {
        return None;
    }
    let body = prefix_ci(text, pos, &["xprv", "tprv", "yprv", "zprv", "uprv", "vprv"])?;
    bounded_run(text, body, 16, |c| c.is_ascii_alphanumeric())
}

/// `nsec1[bech32]{20,}`, IGNORECASE.
fn nostr_secret(text: &[char], pos: usize, _: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) || !literal_ci(text, pos, "nsec1") {
        return None;
    }
    bounded_run(text, pos + 5, 20, is_bech32_ci)
}

/// `5[base58]{49,50}|[KL][base58]{50,51}`.
fn wif(text: &[char], pos: usize, _: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) {
        return None;
    }
    let (low, high) = match text.get(pos) {
        Some('5') => (49, 50),
        Some('K' | 'L') => (50, 51),
        _ => return None,
    };
    let end = run(text, pos + 1, is_base58);
    let length = end - pos - 1;
    ((low..=high).contains(&length) && word_end(text, end)).then_some(end)
}

/// `(?:xpub|tpub|ypub|zpub|upub|vpub)[base58]{20,}`, IGNORECASE: the folded
/// class holds every ASCII letter and the digits 1 to 9.
fn extended_key(text: &[char], pos: usize, _: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) {
        return None;
    }
    let body = prefix_ci(text, pos, &["xpub", "tpub", "ypub", "zpub", "upub", "vpub"])?;
    bounded_run(text, body, 20, |c| {
        c.is_ascii_alphabetic() || matches!(c, '1'..='9')
    })
}

/// `[^)\n]{min,}\)` from `body`.
fn parenthesized(text: &[char], body: usize, min: usize, runs: &mut Runs) -> Option<usize> {
    let stop = runs.stop(text, body);
    (stop < text.len() && text[stop] == ')' && stop - body >= min).then_some(stop + 1)
}

/// `sp\([^)\n]{8,}\)`, IGNORECASE.
fn sp_descriptor(text: &[char], pos: usize, runs: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) || !literal_ci(text, pos, "sp(") {
        return None;
    }
    parenthesized(text, pos + 3, 8, runs)
}

/// `(?:t?spscan|t?spspend)1q[bech32]{8,}`, IGNORECASE.
fn sp_key(text: &[char], pos: usize, _: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) {
        return None;
    }
    let head = pos + usize::from(text.get(pos).is_some_and(|c| c.eq_ignore_ascii_case(&'t')));
    let body = [head, pos]
        .into_iter()
        .find_map(|at| prefix_ci(text, at, &["spscan1q", "spspend1q"]))?;
    bounded_run(text, body, 8, is_bech32_ci)
}

/// `(?:t?sp)1q[bech32]{20,}`, IGNORECASE.
fn sp_address(text: &[char], pos: usize, _: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) {
        return None;
    }
    let head = pos + usize::from(text.get(pos).is_some_and(|c| c.eq_ignore_ascii_case(&'t')));
    let body = [head, pos]
        .into_iter()
        .find_map(|at| prefix_ci(text, at, &["sp1q"]))?;
    bounded_run(text, body, 20, is_bech32_ci)
}

/// `(?:wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,}\)`, IGNORECASE.
fn descriptor(text: &[char], pos: usize, runs: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) {
        return None;
    }
    let body = prefix_ci(
        text,
        pos,
        &["wpkh(", "sh(", "wsh(", "tr(", "pkh(", "combo("],
    )?;
    parenthesized(text, body, 16, runs)
}

fn is_bearer_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\r' | '\n' | '\x0b' | '\x0c' | '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `[Bb]earer[spaces]+[A-Za-z0-9._~+/-]+=*`.
fn bearer(text: &[char], pos: usize, _: &mut Runs) -> Option<usize> {
    if !word_start(text, pos) || !matches!(text.get(pos), Some('B' | 'b')) {
        return None;
    }
    if !"earer"
        .chars()
        .enumerate()
        .all(|(k, c)| text.get(pos + 1 + k) == Some(&c))
    {
        return None;
    }
    let gap = pos + 6;
    let token = run(text, gap, is_bearer_space);
    if token == gap {
        return None;
    }
    let end = run(text, token, is_token);
    if end == token {
        return None;
    }
    Some(run(text, end, |c| c == '='))
}

/// `sk-[A-Za-z0-9._~+/-]{6,}(?![A-Za-z0-9_])`: the greedy run backs off to
/// its last position before a non-word character.
fn sk_secret(text: &[char], pos: usize, runs: &mut Runs) -> Option<usize> {
    if !word_start(text, pos)
        || !"sk-"
            .chars()
            .enumerate()
            .all(|(k, c)| text.get(pos + k) == Some(&c))
    {
        return None;
    }
    let body = pos + 3;
    let (end, last_nonword) = runs.token(text, body);
    let shortest = body + 6;
    if end < shortest {
        return None;
    }
    if word_end(text, end) {
        return Some(end);
    }
    last_nonword.filter(|&at| at >= shortest)
}
