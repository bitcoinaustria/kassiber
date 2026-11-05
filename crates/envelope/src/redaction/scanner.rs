//! The forward-only credential scanners of `kassiber/redaction_scanner.py`.
//!
//! Every function mirrors the Python function of the same name (without its
//! leading underscore), over code points as Python indexes a `str`. Python's
//! regular expressions are written out as matchers that accept exactly what
//! `re` accepts, including its backtracking choices and `$` matching before a
//! final newline. Scans stay linear: line facts come from one table per
//! record, and the floor's run lookups only move forward.
//!
//! This file uses only the standard library: the desktop supervisor
//! (`ui-tauri/src-tauri/src/supervisor.rs`) compiles the same source.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// Longer records are masked whole: no scanner runs on an unbounded input.
pub const REDACTION_LIMIT: usize = 1_000_000;

type Span = (usize, usize, String);
/// A start and end.
type Range = (usize, usize);
type Scan = fn(&[char], &Offsets) -> Vec<Span>;

// --- character classes -----------------------------------------------------

/// `_SPACE`.
fn is_space(c: char) -> bool {
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

/// `_LINES`.
fn is_line(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `_XML_SPACE`.
fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0b' | '\x0c')
}

/// `[ \t\v\f]` plus `_LINES`: a gap that ends a URL token.
fn is_gap(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\x0b' | '\x0c') || is_line(c)
}

/// `_IDENTIFIER`'s class `[A-Za-z0-9_.-]`.
fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// `[A-Za-z0-9_]`.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

fn in_set(c: char, set: &str) -> bool {
    set.contains(c)
}

// --- slices and strings ----------------------------------------------------

/// `text[a:b]` with Python's clamping: an empty slice when `a >= b`.
fn sl(text: &[char], a: usize, b: usize) -> &[char] {
    let b = b.min(text.len());
    if a >= b {
        &[]
    } else {
        &text[a..b]
    }
}

fn starts(text: &[char], pos: usize, needle: &str) -> bool {
    (pos..)
        .zip(needle.chars())
        .all(|(index, c)| text.get(index) == Some(&c))
}

fn starts_chars(text: &[char], pos: usize, needle: &[char]) -> bool {
    pos <= text.len()
        && text.len() - pos >= needle.len()
        && &text[pos..pos + needle.len()] == needle
}

fn find_char(text: &[char], c: char, from: usize, to: usize) -> Option<usize> {
    (from..to.min(text.len())).find(|&i| text[i] == c)
}

fn rfind_char(text: &[char], c: char, from: usize, to: usize) -> Option<usize> {
    (from..to.min(text.len())).rev().find(|&i| text[i] == c)
}

/// `text.find(needle, from)`.
fn find_chars(text: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return (from <= text.len()).then_some(from);
    }
    let last = text.len().checked_sub(needle.len())?;
    (from..=last).find(|&i| text[i] == needle[0] && &text[i..i + needle.len()] == needle)
}

fn string(text: &[char]) -> String {
    text.iter().collect()
}

fn chars(text: &str) -> Vec<char> {
    text.chars().collect()
}

fn strip(text: &[char], set: impl Fn(char) -> bool) -> &[char] {
    let mut a = 0;
    let mut b = text.len();
    while a < b && set(text[a]) {
        a += 1;
    }
    while b > a && set(text[b - 1]) {
        b -= 1;
    }
    &text[a..b]
}

fn rstrip<'a>(text: &'a [char], set: &str) -> &'a [char] {
    let mut b = text.len();
    while b > 0 && in_set(text[b - 1], set) {
        b -= 1;
    }
    &text[..b]
}

fn lstrip(text: &[char], set: impl Fn(char) -> bool) -> &[char] {
    let mut a = 0;
    while a < text.len() && set(text[a]) {
        a += 1;
    }
    &text[a..]
}

/// `text.lower() == target` for an ASCII target. No non-ASCII code point
/// lowercases to the ASCII letters these targets need (only U+212A
/// lowercases to `k`, which none holds).
fn lower_is(text: &[char], target: &str) -> bool {
    text.len() == target.len()
        && text
            .iter()
            .zip(target.chars())
            .all(|(c, t)| c.is_ascii() && c.to_ascii_lowercase() == t)
}

/// `re.fullmatch(r"[0-9]+", text)`.
fn all_digits(text: &[char]) -> bool {
    !text.is_empty() && text.iter().all(char::is_ascii_digit)
}

// --- offsets ---------------------------------------------------------------

/// Original-coordinate offsets of a decoded view: identity or a table.
struct Offsets(Option<Vec<usize>>);

impl Offsets {
    fn identity() -> Self {
        Self(None)
    }

    fn at(&self, index: usize) -> usize {
        match &self.0 {
            None => index,
            Some(table) => table[index],
        }
    }
}

/// `original[offsets[pos]:offsets[pos + 1]] == view[pos]`: the view's
/// character is literally in the original.
fn literal(view: &[char], original: &[char], offsets: &Offsets, pos: usize) -> bool {
    let (a, b) = (offsets.at(pos), offsets.at(pos + 1));
    b == a + 1 && original.get(a) == view.get(pos)
}

// --- vocabulary ------------------------------------------------------------

const SECRET_WORDS: &str = "authorization bearer blinding cookie cookies credential credentials descriptor
macaroon mfa mnemonic otp pass passcode passphrase passwd password pin pw pwd rpcauth rune secret seed sessid
token totp twofa wif xprv";
const PART_NAMES: &str = "auth jwt session sid sig sign signature user";
const LAST_NAMES: &str = "mfa otp pin pw sign totp twofa";
const MODIFIERS: &str = "backup confirm copy enc encoded hex new old plain raw repeat value";
const WEAK_NAMES: &str = "pass user auth mfa twofa";
const COMPOUND_PAIRS: &[(&str, &str)] = &[
    ("access", "key"),
    ("api", "key"),
    ("api", "secret"),
    ("api", "token"),
    ("auth", "code"),
    ("auth", "header"),
    ("auth", "key"),
    ("backup", "code"),
    ("blinding", "key"),
    ("encryption", "key"),
    ("hmac", "key"),
    ("master", "key"),
    ("mfa", "code"),
    ("otp", "code"),
    ("pin", "code"),
    ("priv", "key"),
    ("private", "key"),
    ("recovery", "code"),
    ("recovery", "phrase"),
    ("scan", "key"),
    ("scan", "private"),
    ("secret", "key"),
    ("seed", "phrase"),
    ("seed", "words"),
    ("session", "id"),
    ("session", "key"),
    ("signing", "key"),
    ("silent", "payment"),
    ("silent", "payments"),
    ("sp", "descriptor"),
    ("sp", "scan"),
    ("sp", "spend"),
    ("spend", "key"),
    ("subscription", "key"),
    ("totp", "code"),
    ("twofa", "code"),
    ("verification", "code"),
];
const METADATA: &str = "disabled enabled fd file hint path statistics stdin";
const COUNT_WORDS: &str = "count max min limit length len size ttl";
const QUALIFIERS: &str = "access account admin api app aws b base bitcoin client code data db default dev
device electrum enc encoded encryption env github hash header hex id key keys lnd local login main master mfa
my mysql new old payment payments pem pg phrase plain priv private prod raw recovery refresh remote root rpc
scan server service signing silent sp spend str test text total twofa v value verification wallet web words
x azure binance bitfinex bitpanda bitstamp blockstream btcpay cln coinbase core discord electrs fulcrum gcp
gitlab google kassiber kraken lightning mempool mongo nostr postgres redis slack smtp strike stripe telegram
umbrel";
const ORDINARY: &str = "bypass compass dnsseed encompass overpass passable passage passed passenger passerby
passing passive passover passport passthrough passwordless prune secretar seeded seeder seednode spin surpass
swift tokeniz trespass underpass wifi";
const HINT_EXTRA: &str = "2fa code key phrase recovery scan silent spend";

struct Vocabulary {
    secret: HashSet<&'static str>,
    part: HashSet<&'static str>,
    last: HashSet<&'static str>,
    modifiers: HashSet<&'static str>,
    weak: HashSet<&'static str>,
    compounds: HashMap<(&'static str, &'static str), String>,
    compound_words: HashSet<String>,
    metadata: HashSet<&'static str>,
    count: HashSet<&'static str>,
    plurals: HashSet<String>,
    vocab: HashSet<String>,
    sizes: Vec<usize>,
    /// `_EMBEDDED` with each word's `_CONTAINERS`.
    embedded: Vec<(String, Vec<(&'static str, usize)>)>,
    named: HashSet<String>,
    hint: Vec<String>,
}

fn words(list: &'static str) -> HashSet<&'static str> {
    list.split_whitespace().collect()
}

fn vocabulary() -> &'static Vocabulary {
    static VOCABULARY: OnceLock<Vocabulary> = OnceLock::new();
    VOCABULARY.get_or_init(|| {
        let secret = words(SECRET_WORDS);
        let part = words(PART_NAMES);
        let compounds: HashMap<_, _> = COMPOUND_PAIRS
            .iter()
            .map(|&(first, second)| ((first, second), format!("{first}{second}")))
            .collect();
        let compound_words: HashSet<String> = compounds.values().cloned().collect();
        let metadata = words(METADATA);
        let count = words(COUNT_WORDS);
        let mut qualifiers: HashSet<&str> = words(QUALIFIERS);
        qualifiers.extend(&count);
        qualifiers.extend(&metadata);
        let plurals: HashSet<String> = secret
            .iter()
            .chain(&part)
            .map(|word| (*word).to_owned())
            .chain(compound_words.iter().cloned())
            .filter(|word| !word.ends_with('s'))
            .map(|word| word + "s")
            .collect();
        let mut vocab: HashSet<String> = secret.iter().map(|w| (*w).to_owned()).collect();
        vocab.extend(part.iter().map(|w| (*w).to_owned()));
        vocab.extend(qualifiers.iter().map(|w| (*w).to_owned()));
        vocab.extend(compound_words.iter().cloned());
        vocab.extend(plurals.iter().cloned());
        let mut sizes: Vec<usize> = vocab
            .iter()
            .map(String::len)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        let mut ordinary: Vec<&'static str> = ORDINARY.split_whitespace().collect();
        ordinary.sort_unstable();
        let mut embedded_words: Vec<String> = secret
            .iter()
            .map(|w| (*w).to_owned())
            .chain(compound_words.iter().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        embedded_words.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        let embedded = embedded_words
            .into_iter()
            .map(|word| {
                let mut containers = Vec::new();
                for &ordinary in &ordinary {
                    if ordinary.len() >= word.len() {
                        for offset in 0..=ordinary.len() - word.len() {
                            if ordinary[offset..].starts_with(word.as_str()) {
                                containers.push((ordinary, offset));
                            }
                        }
                    }
                }
                (word, containers)
            })
            .collect();
        let mut named: HashSet<String> = secret.iter().map(|w| (*w).to_owned()).collect();
        named.extend(compound_words.iter().cloned());
        named.extend(plurals.iter().cloned());
        let mut hint: Vec<String> = secret
            .iter()
            .chain(&part)
            .chain(HINT_EXTRA.split_whitespace().collect::<Vec<_>>().iter())
            .map(|w| (*w).to_owned())
            .collect();
        hint.sort();
        hint.dedup();
        Vocabulary {
            secret,
            part,
            last: words(LAST_NAMES),
            modifiers: words(MODIFIERS),
            weak: words(WEAK_NAMES),
            compounds,
            compound_words,
            metadata,
            count,
            plurals,
            vocab,
            sizes,
            embedded,
            named,
            hint,
        }
    })
}

// --- names -----------------------------------------------------------------

/// `_segment(part)`: `part` holds only lowercase ASCII letters.
fn segment(part: &str) -> Vec<String> {
    let v = vocabulary();
    if v.vocab.contains(part) {
        return vec![part.to_owned()];
    }
    let n = part.len();
    if n > 40 {
        return embedded_segment(part);
    }
    // Fewest words; ties keep the longest leading word.
    let mut best: Vec<Option<(usize, usize)>> = vec![None; n + 1];
    best[n] = Some((0, n));
    for start in (0..n).rev() {
        for &size in &v.sizes {
            let end = start + size;
            let following = if end <= n { best[end] } else { None };
            if let Some((count, _)) = following {
                if v.vocab.contains(&part[start..end])
                    && best[start].is_none_or(|(current, _)| count + 1 < current)
                {
                    best[start] = Some((count + 1, end));
                }
            }
        }
    }
    if best[0].is_none() {
        return embedded_segment(part);
    }
    let mut words = Vec::new();
    let mut start = 0;
    while start < n {
        let end = best[start].map_or(n, |(_, end)| end);
        words.push(part[start..end].to_owned());
        start = end;
    }
    words
}

fn known_prefix(part: &str, end: usize) -> bool {
    // An ordinary word counts only at the start or after known words:
    // `pruneheight` and `mytokenizer`, not `ftprune` (`ftp` + `rune`).
    let v = vocabulary();
    end == 0
        || (end <= 40
            && segment(&part[..end])
                .iter()
                .all(|piece| v.vocab.contains(piece)))
}

/// Split an unknown part around its last secret word outside ordinary words.
fn embedded_segment(part: &str) -> Vec<String> {
    let v = vocabulary();
    'words: for (word, containers) in &v.embedded {
        let mut start = part.rfind(word.as_str());
        while let Some(s) = start {
            let contained = containers.iter().any(|&(ordinary, offset)| {
                s >= offset
                    && part[s - offset..].starts_with(ordinary)
                    && known_prefix(part, s - offset)
            });
            if !contained {
                break;
            }
            let limit = s + word.len() - 1;
            start = part[..limit].rfind(word.as_str());
        }
        let Some(s) = start else {
            continue;
        };
        let rest = &part[s + word.len()..];
        let mut pieces: Vec<String> = if s > 0 {
            vec![part[..s].to_owned()]
        } else {
            Vec::new()
        };
        if rest == "s" {
            pieces.push(format!("{word}s"));
            return pieces;
        }
        let tail: Vec<String> = if rest.is_empty() {
            Vec::new()
        } else if rest.len() <= 40 {
            segment(rest)
        } else {
            vec![rest.to_owned()]
        };
        if v.last.contains(word.as_str())
            && !tail
                .iter()
                .all(|piece| v.modifiers.contains(piece.as_str()))
        {
            // `rootpwdconfirm` names `pwd`, not a non-final `otp`.
            continue 'words;
        }
        pieces.push(word.clone());
        pieces.extend(tail);
        return pieces;
    }
    vec![part.to_owned()]
}

type Reading = Vec<(String, bool)>;

/// `_VERSION.sub("", token)`: `[_-]v[0-9]+_*$` with IGNORECASE.
fn strip_version(token: &[char]) -> Vec<char> {
    let n = token.len();
    for i in 0..n {
        if !matches!(token[i], '_' | '-') || !matches!(token.get(i + 1), Some('v' | 'V')) {
            continue;
        }
        let mut end = i + 2;
        while end < n && token[end].is_ascii_digit() {
            end += 1;
        }
        if end == i + 2 {
            continue;
        }
        while end < n && token[end] == '_' {
            end += 1;
        }
        if end == n || (end + 1 == n && token[end] == '\n') {
            let mut out = token[..i].to_vec();
            out.extend_from_slice(&token[end..]);
            return out;
        }
    }
    token.to_vec()
}

/// `_TWOFA.sub("twofa", token)`.
fn replace_twofa(token: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(token.len());
    let mut i = 0;
    while i < token.len() {
        if token[i] == '2'
            && token
                .get(i + 1)
                .is_some_and(|c| c.eq_ignore_ascii_case(&'f'))
            && token
                .get(i + 2)
                .is_some_and(|c| c.eq_ignore_ascii_case(&'a'))
        {
            out.extend("twofa".chars());
            i += 3;
        } else {
            out.push(token[i]);
            i += 1;
        }
    }
    out
}

/// `_KEY_PARTS.split(chunk)` for an ASCII alphanumeric chunk.
fn key_parts(chunk: &[char]) -> Vec<&[char]> {
    let mut parts = Vec::new();
    let mut from = 0;
    for p in 1..chunk.len() {
        let (before, here) = (chunk[p - 1], chunk[p]);
        let next = chunk.get(p + 1).copied();
        let split = ((before.is_ascii_lowercase() || before.is_ascii_digit())
            && here.is_ascii_uppercase())
            || (before.is_ascii_uppercase()
                && here.is_ascii_uppercase()
                && next.is_some_and(|c| c.is_ascii_lowercase()))
            || (before.is_ascii_digit() && here.is_ascii_alphabetic());
        if split {
            parts.push(&chunk[from..p]);
            from = p;
        }
    }
    parts.push(&chunk[from..]);
    parts
}

/// Lowercase words of an identifier, each marked when it is a whole part.
///
/// Case runs are hints, not boundaries: `rpcSECRETkeyOLD` and `newPWDb64`
/// split inside a word, so a chunk is also read concatenated when that
/// reading segments into known words. Either reading can name a secret.
fn key_readings(token: &[char]) -> Vec<Reading> {
    let v = vocabulary();
    let token = strip_version(token);
    let token = replace_twofa(strip(&token, |c| matches!(c, '_' | '.' | '-')));
    let mut split: Reading = Vec::new();
    let mut joined: Reading = Vec::new();
    let mut i = 0;
    while i < token.len() {
        if !token[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < token.len() && token[i].is_ascii_alphanumeric() {
            i += 1;
        }
        let parts: Vec<String> = key_parts(&token[start..i])
            .into_iter()
            .map(|raw| {
                let lower: String = raw.iter().map(char::to_ascii_lowercase).collect();
                lower.trim_matches(|c: char| c.is_ascii_digit()).to_owned()
            })
            .filter(|part| !part.is_empty())
            .collect();
        let mut pieces: Reading = Vec::new();
        for part in &parts {
            let segmented = segment(part);
            let whole = segmented.len() == 1;
            pieces.extend(segmented.into_iter().map(|piece| (piece, whole)));
        }
        split.extend(pieces.iter().cloned());
        let segmented = if parts.len() > 1 {
            segment(&parts.concat())
        } else {
            Vec::new()
        };
        if !segmented.is_empty()
            && (segmented.iter().all(|piece| v.vocab.contains(piece))
                || segmented.iter().any(|piece| v.named.contains(piece)))
        {
            let whole = segmented.len() == 1;
            joined.extend(segmented.into_iter().map(|piece| (piece, whole)));
        } else {
            joined.extend(pieces);
        }
    }
    if joined == split {
        vec![split]
    } else {
        vec![split, joined]
    }
}

fn reading_names(words: &Reading) -> Vec<String> {
    let v = vocabulary();
    let Some((last, _)) = words.last() else {
        return Vec::new();
    };
    if v.metadata.contains(last.as_str()) {
        return Vec::new();
    }
    let mut names = Vec::new();
    // Whether only modifiers follow each word, so a final name may carry one.
    let mut last_only = vec![true; words.len()];
    for index in (0..words.len().saturating_sub(1)).rev() {
        last_only[index] =
            last_only[index + 1] && v.modifiers.contains(words[index + 1].0.as_str());
    }
    for (index, (word, whole)) in words.iter().enumerate() {
        if let Some((second, _)) = words.get(index + 1) {
            let pair = v
                .compounds
                .get(&(word.as_str(), second.as_str()))
                .or_else(|| {
                    second
                        .strip_suffix('s')
                        .and_then(|single| v.compounds.get(&(word.as_str(), single)))
                });
            if let Some(pair) = pair {
                names.push(pair.clone());
            }
        }
        let base = if v.plurals.contains(word) && !v.secret.contains(word.as_str()) {
            &word[..word.len() - 1]
        } else {
            word.as_str()
        };
        if v.compound_words.contains(base) {
            names.push(base.to_owned());
            continue;
        }
        if (base == "user" || base == "auth") && base != word {
            continue;
        }
        if v.part.contains(base) && !whole {
            continue;
        }
        if v.last.contains(base) && !last_only[index] {
            continue;
        }
        if v.secret.contains(base) || v.part.contains(base) {
            names.push(base.to_owned());
        }
    }
    names
}

/// `_HINT.search(token)`.
fn has_hint(token: &[char]) -> bool {
    let lower: String = token
        .iter()
        .map(|c| {
            if c.is_ascii() {
                c.to_ascii_lowercase()
            } else {
                '\0'
            }
        })
        .collect();
    vocabulary()
        .hint
        .iter()
        .any(|word| lower.contains(word.as_str()))
}

/// `_secret_name(token)`: the longest (then smallest) secret name it holds.
pub fn secret_name(token: &[char]) -> String {
    if !has_hint(token) {
        return String::new();
    }
    key_readings(token)
        .iter()
        .flat_map(reading_names)
        .min_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)))
        .unwrap_or_default()
}

/// `_COUNT_VALUE.fullmatch(value)`.
fn count_value(value: &[char]) -> bool {
    let mut i = 0;
    while i < value.len() && value[i].is_ascii_digit() {
        i += 1;
    }
    if !(1..=9).contains(&i) {
        return false;
    }
    if i == value.len() {
        return true;
    }
    if value[i] != '.' {
        return false;
    }
    let fraction = value.len() - i - 1;
    (1..=9).contains(&fraction) && value[i + 1..].iter().all(char::is_ascii_digit)
}

/// `_LITERAL.match(text, pos)`: the literal's end.
fn literal_match(text: &[char], pos: usize) -> Option<usize> {
    let n = text.len();
    let boundary = |end: usize| end == n || !(is_word(text[end]) || text[end] == '.');
    let mut i = pos;
    if text.get(i) == Some(&'-') {
        i += 1;
    }
    let digits = i;
    while i < n && text[i].is_ascii_digit() {
        i += 1;
    }
    if i > digits {
        if text.get(i) == Some(&'.') && text.get(i + 1).is_some_and(char::is_ascii_digit) {
            i += 1;
            while i < n && text[i].is_ascii_digit() {
                i += 1;
            }
        }
        if matches!(text.get(i), Some('e' | 'E')) {
            let mut j = i + 1;
            if matches!(text.get(j), Some('+' | '-')) {
                j += 1;
            }
            if text.get(j).is_some_and(char::is_ascii_digit) {
                while j < n && text[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        return boundary(i).then_some(i);
    }
    for keyword in ["null", "true", "false", "None", "True", "False"] {
        if starts(text, pos, keyword) && boundary(pos + keyword.len()) {
            return Some(pos + keyword.len());
        }
    }
    None
}

fn count_name(key: &[char], name: &str) -> bool {
    // A count word or the name's plural, whole or compound (`descriptors`,
    // `disable_private_keys`).
    let v = vocabulary();
    let plural = format!("{name}s");
    for words in key_readings(key) {
        for (index, (word, _)) in words.iter().enumerate() {
            if v.count.contains(word.as_str())
                || (*word == plural && !v.secret.contains(word.as_str()))
                || (index > 0 && format!("{}{word}", words[index - 1].0) == plural)
            {
                return true;
            }
        }
    }
    false
}

// --- URL context -----------------------------------------------------------

fn is_scheme_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')
}

/// `_SCHEME` at `p`: the end of the match.
fn scheme_at(text: &[char], p: usize) -> Option<usize> {
    let n = text.len();
    if p < n && is_scheme_char(text[p]) && (p == 0 || !is_scheme_char(text[p - 1])) {
        let mut end = p;
        while end < n && is_scheme_char(text[end]) {
            end += 1;
        }
        if starts(text, end, "://") {
            return Some(end + 3);
        }
    }
    if starts(text, p, "://") {
        return Some(p + 3);
    }
    if starts(text, p, "//") {
        return Some(p + 2);
    }
    None
}

/// `_SCHEME.finditer(text)`.
fn scheme_matches(text: &[char]) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let mut p = 0;
    while p < text.len() {
        if let Some(end) = scheme_at(text, p) {
            matches.push((p, end));
            p = end;
        } else {
            p += 1;
        }
    }
    matches
}

/// `_quote_at(text, pos)`: the end of a run of backslashes and a quote, or
/// `pos` when none starts there.
fn quote_at(text: &[char], pos: usize) -> usize {
    let mut end = pos;
    while end < text.len() && text[end] == '\\' {
        end += 1;
    }
    if end < text.len() && matches!(text[end], '"' | '\'') {
        end + 1
    } else {
        pos
    }
}

fn quoted_end(
    text: &[char],
    mut pos: usize,
    delimiter: &[char],
    limit: Option<usize>,
) -> (usize, bool) {
    let mut slashes = 0usize;
    let slash_count = delimiter.len() - 1;
    let period = 2
        * (slash_count
            + if slash_count != 0 && slash_count.is_multiple_of(2) {
                2
            } else {
                1
            });
    let mut first_newline = None;
    let limit = limit.unwrap_or(text.len());
    let last = delimiter[delimiter.len() - 1];
    while pos < limit {
        let ch = text[pos];
        if is_line(ch) && first_newline.is_none() {
            first_newline = Some(pos);
        }
        if ch == last && slashes % period == delimiter.len() - 1 {
            return (pos + 1, true);
        }
        slashes = if ch == '\\' { slashes + 1 } else { 0 };
        pos += 1;
    }
    (yaml_end(text, first_newline.unwrap_or(pos)), false)
}

/// `_IDENTIFIER.match(text, pos)`: the identifier's end.
fn ident_match(text: &[char], pos: usize) -> Option<usize> {
    let mut end = pos;
    while end < text.len() && is_ident(text[end]) {
        end += 1;
    }
    (end > pos).then_some(end)
}

fn value_boundary(text: &[char], mut pos: usize) -> bool {
    let n = text.len();
    while pos < n {
        if in_set(text[pos], ")]}") {
            pos += 1;
        } else {
            let body = quote_at(text, pos);
            if body == pos {
                break;
            }
            pos = body;
        }
    }
    if pos == n || is_space(text[pos]) {
        return true;
    }
    if in_set(text[pos], ",;") {
        pos += 1;
        let gap = pos;
        while pos < n && is_space(text[pos]) && !is_line(text[pos]) {
            pos += 1;
        }
        if pos == n || is_line(text[pos]) {
            return true;
        }
        let body = quote_at(text, pos);
        let delimiter = &text[pos..body];
        let key = if delimiter.is_empty() {
            None
        } else {
            ident_match(text, body)
        };
        if let Some(key_end) = key {
            if starts_chars(text, key_end, delimiter) {
                let mut tail = key_end + delimiter.len();
                while tail < n && is_space(text[tail]) {
                    tail += 1;
                }
                return tail < n && in_set(text[tail], ":=");
            }
        }
        let prose = if pos > gap && delimiter.is_empty() {
            ident_match(text, pos)
        } else {
            None
        };
        if let Some(end) = prose {
            if text[pos..end].iter().all(char::is_ascii_lowercase) {
                return end == n || is_space(text[end]) || in_set(text[end], ".!?");
            }
        }
    }
    false
}

fn record_opener(
    view: &[char],
    original: &[char],
    offsets: &Offsets,
    quote: Option<usize>,
) -> bool {
    let Some(quote) = quote else {
        return false;
    };
    if !literal(view, original, offsets, quote) {
        return false;
    }
    let mut pos = quote as isize - 1;
    let at = |p: isize| view[p as usize];
    while pos >= 0 && at(pos) == '\\' {
        pos -= 1;
    }
    while pos >= 0 && is_space(at(pos)) {
        pos -= 1;
    }
    if pos < 0 || !in_set(at(pos), ":=") {
        return false;
    }
    pos -= 1;
    while pos >= 0 && is_space(at(pos)) {
        pos -= 1;
    }
    if pos < 0 || !in_set(at(pos), "\"'") || !literal(view, original, offsets, pos as usize) {
        return false;
    }
    let delimiter = at(pos);
    pos -= 1;
    while pos >= 0 && at(pos) == '\\' {
        pos -= 1;
    }
    let end = pos;
    while pos >= 0 && (in_set(at(pos), "_.-") || at(pos).is_ascii_alphanumeric()) {
        pos -= 1;
    }
    pos < end && pos >= 0 && at(pos) == delimiter && literal(view, original, offsets, pos as usize)
}

fn url_record_end(
    view: &[char],
    original: &[char],
    offsets: &Offsets,
    start: usize,
    end: usize,
    record: bool,
) -> usize {
    if !record {
        return end;
    }
    let Some(pos) = (start..end).find(|&i| in_set(view[i], "\\\"'")) else {
        return end;
    };
    let body = quote_at(view, pos);
    if body == pos
        || sl(original, offsets.at(pos), offsets.at(body)) != &view[pos..body]
        || body >= end
        || !in_set(view[body], ",;")
        || !value_boundary(view, body)
    {
        return end;
    }
    let prefix = &view[start..pos];
    for candidate in [prefix, rstrip(prefix, ").>]")] {
        if clear_host(candidate) || public_url_prefix(candidate, original, offsets, start) {
            return pos;
        }
    }
    end
}

/// `_url_heads(text)`: each URL's credential context, in `text` coordinates.
fn url_heads(text: &[char]) -> Vec<(usize, usize, bool)> {
    // Preserve context even when a keyed pass masks a URL's original @.
    let (view, offsets) = url_view(text);
    let matches = scheme_matches(&view);
    let final_at = rfind_char(&view, '@', 0, view.len());
    let quotes: Vec<usize> = (0..view.len())
        .filter(|&i| in_set(view[i], "\"'"))
        .collect();
    let mut next_quote = 0;
    let mut previous_quote: Option<usize> = None;
    let mut checked_quote: Option<usize> = None;
    let mut record = false;
    let mut heads = Vec::with_capacity(matches.len());
    for (index, &(match_start, match_end)) in matches.iter().enumerate() {
        while next_quote < quotes.len() && quotes[next_quote] < match_start {
            previous_quote = Some(quotes[next_quote]);
            next_quote += 1;
        }
        if previous_quote != checked_quote {
            record = record_opener(&view, text, &offsets, previous_quote);
            checked_quote = previous_quote;
        }
        let start = match_end;
        let segment_end = matches.get(index + 1).map_or(view.len(), |m| m.0);
        let limit = url_record_end(&view, text, &offsets, start, segment_end, record);
        let limit = url_candidate_end(
            &view,
            text,
            &offsets,
            start,
            url_line_end(&view, text, &offsets, start, limit),
        );
        let at = rfind_char(&view, '@', start, limit);
        let credentials = at.is_some()
            || (limit == segment_end
                && index + 1 < matches.len()
                && final_at.is_some_and(|a| a >= limit));
        let end = if credentials {
            at.map_or(limit, |a| a + 1)
        } else {
            let mut end = start;
            while end < limit
                && !in_set(view[end], "\"'<>[]/{}()?#&|")
                && !(is_space(view[end]) && literal(&view, text, &offsets, end))
            {
                end += 1;
            }
            end
        };
        heads.push((offsets.at(start), offsets.at(end), credentials));
    }
    heads
}

fn line_end(text: &[char], mut pos: usize) -> usize {
    while pos < text.len() && !is_line(text[pos]) {
        pos += 1;
    }
    pos
}

// --- keyed values ----------------------------------------------------------

const COMMANDS: &[&str] = &[
    "cmd",
    "kassiber",
    "bitcoind",
    "bitcoin-cli",
    "electrs",
    "lncli",
    "lightning-cli",
    "curl",
    "python",
    "node",
    "env",
    "sudo",
    "docker",
];

/// `(?=[ \t]|$)`.
fn command_end(text: &[char], pos: usize) -> bool {
    pos == text.len()
        || matches!(text[pos], ' ' | '\t')
        || (pos + 1 == text.len() && text[pos] == '\n')
}

/// `_COMMAND_LINE.finditer(text)`.
fn command_matches(text: &[char]) -> Vec<(usize, usize)> {
    let n = text.len();
    let mut matches = Vec::new();
    let mut p = 0;
    while p <= n {
        if p == 0 || is_line(text[p - 1]) {
            let mut i = p;
            while i < n && matches!(text[i], ' ' | '\t') {
                i += 1;
            }
            if i < n && matches!(text[i], '$' | '>') && matches!(text.get(i + 1), Some(' ' | '\t'))
            {
                i += 1;
                while i < n && matches!(text[i], ' ' | '\t') {
                    i += 1;
                }
            }
            let word = COMMANDS
                .iter()
                .find(|word| starts(text, i, word) && command_end(text, i + word.len()));
            if let Some(word) = word {
                let end = i + word.len();
                matches.push((p, end));
                p = end.max(p + 1);
                continue;
            }
        }
        p += 1;
    }
    matches
}

/// `_Lines`: one record's line facts, built once and on demand.
///
/// A value's indented continuation and a YAML block end are lookups instead
/// of scans, so an indentation staircase stays linear however many keys and
/// lines share one continuation.
struct Lines<'a> {
    text: &'a [char],
    starts: Vec<usize>,
    ends: Vec<usize>,
    depth: Vec<isize>,
    below: Vec<usize>,
    yaml: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a [char]) -> Self {
        let mut starts = vec![0];
        let mut ends = Vec::new();
        let mut i = 0;
        while i < text.len() {
            if text[i] == '\r' && text.get(i + 1) == Some(&'\n') {
                ends.push(i);
                starts.push(i + 2);
                i += 2;
            } else if is_line(text[i]) {
                ends.push(i);
                starts.push(i + 1);
                i += 1;
            } else {
                i += 1;
            }
        }
        ends.push(text.len());
        Self {
            text,
            starts,
            ends,
            depth: Vec::new(),
            below: Vec::new(),
            yaml: Vec::new(),
        }
    }

    fn line(&self, pos: usize) -> Option<usize> {
        let line = self.starts.partition_point(|&start| start <= pos) - 1;
        // The LF of a CRLF pair is its own empty line to the scanners.
        (pos <= self.ends[line]).then_some(line)
    }

    fn continued_end(&mut self, pos: usize, column: usize) -> usize {
        let Some(line) = self.line(pos) else {
            return continued_end(self.text, pos, column);
        };
        if self.depth.is_empty() {
            // Plain indents; a blank line (-1) ends every continuation. `below`
            // is the next shallower line, so each walk descends strictly.
            for (&start, &end) in self.starts.iter().zip(&self.ends) {
                let mut indent = start;
                while indent < self.text.len() && matches!(self.text[indent], ' ' | '\t') {
                    indent += 1;
                }
                self.depth.push(if indent < end {
                    (indent - start) as isize
                } else {
                    -1
                });
            }
            let count = self.depth.len();
            self.below = vec![count; count];
            let mut stack: Vec<usize> = Vec::new();
            for index in (0..count).rev() {
                while stack
                    .last()
                    .is_some_and(|&top| self.depth[top] >= self.depth[index])
                {
                    stack.pop();
                }
                if let Some(&top) = stack.last() {
                    self.below[index] = top;
                }
                stack.push(index);
            }
        }
        let mut following = line + 1;
        while following < self.depth.len() && self.depth[following] > column as isize {
            following = self.below[following];
        }
        self.ends[following - 1]
    }

    fn continues(&mut self, pos: usize, column: usize) -> bool {
        self.continued_end(pos, column) != line_end(self.text, pos)
    }

    fn yaml_end(&mut self, pos: usize) -> usize {
        let Some(line) = self.line(pos) else {
            return yaml_end(self.text, pos);
        };
        if self.yaml.is_empty() {
            self.yaml = self.ends.clone();
            for index in (0..self.ends.len().saturating_sub(1)).rev() {
                let start = self.starts[index + 1];
                if !(start < self.ends[index + 1] && !matches!(self.text[start], ' ' | '\t')) {
                    self.yaml[index] = self.yaml[index + 1];
                }
            }
        }
        self.yaml[line]
    }
}

fn continued_end(text: &[char], pos: usize, column: usize) -> usize {
    let mut end = line_end(text, pos);
    while end < text.len() {
        let line = end + if starts(text, end, "\r\n") { 2 } else { 1 };
        let mut indent = line;
        while indent < text.len() && matches!(text[indent], ' ' | '\t') {
            indent += 1;
        }
        if indent - line <= column || indent == text.len() || is_line(text[indent]) {
            break;
        }
        end = line_end(text, indent);
    }
    end
}

fn line_indent(text: &[char], start: usize) -> usize {
    // The line's own indent, past YAML sequence markers (`- password: x`).
    let mut pos = start;
    loop {
        while pos < text.len() && matches!(text[pos], ' ' | '\t') {
            pos += 1;
        }
        if !(starts(text, pos, "- ") || starts(text, pos, "-\t")) {
            return pos - start;
        }
        pos += 1;
    }
}

fn literal_end(
    text: &[char],
    mut pos: usize,
    column: usize,
    lines: &mut Lines<'_>,
    argv: bool,
) -> bool {
    // The literal is the whole value: closers then a record boundary, nothing
    // but whitespace through its line and any continuation, or in argv a
    // following flag or `name=` argument.
    let n = text.len();
    let closers = pos;
    while pos < n && in_set(text[pos], ")]}") {
        pos += 1;
    }
    if pos < n && in_set(text[pos], ",;") {
        return value_boundary(text, pos);
    }
    let closed = pos > closers;
    let gap = pos;
    while pos < n && matches!(text[pos], ' ' | '\t') {
        pos += 1;
    }
    if pos == n {
        return true;
    }
    if is_line(text[pos]) {
        // A closed structure ends its literal; a bare one owns its continuation.
        return closed || !lines.continues(pos, column);
    }
    pos > gap && (closed || (argv && argv_boundary(text, pos)))
}

/// A flat object whose members are all numbers, null or booleans.
fn count_object(text: &[char], mut pos: usize) -> bool {
    let n = text.len();
    pos += 1;
    loop {
        while pos < n && is_space(text[pos]) {
            pos += 1;
        }
        if starts(text, pos, "}") {
            return true;
        }
        let body = quote_at(text, pos);
        if body == pos {
            return false;
        }
        let delimiter = &text[pos..body];
        let Some(key_end) = ident_match(text, body) else {
            return false;
        };
        if !starts_chars(text, key_end, delimiter) {
            return false;
        }
        pos = key_end + delimiter.len();
        while pos < n && matches!(text[pos], ' ' | '\t') {
            pos += 1;
        }
        if !starts(text, pos, ":") {
            return false;
        }
        pos += 1;
        while pos < n && matches!(text[pos], ' ' | '\t') {
            pos += 1;
        }
        let Some(end) = literal_match(text, pos) else {
            return false;
        };
        pos = end;
        while pos < n && is_space(text[pos]) {
            pos += 1;
        }
        if starts(text, pos, ",") {
            pos += 1;
        } else if !starts(text, pos, "}") {
            return false;
        }
    }
}

// Bitcoin Core's argument-free options. A bare single-dash word can be part of
// a passphrase (`--passphrase correct -horse battery`), so only these, `-h`,
// long `--flags`, `-name=` assignments and a letter flag before a `NAME=`
// argument (`docker run -e NETWORK=mainnet`) end an argv value.
const CORE_FLAGS: &str = "addrinfo blocksonly coinstatsindex daemon daemonwait debug disablewallet discover dns dnsseed
fixedseeds forcednsseed generate getinfo h help help-debug listen listenonion logips logsourcelocations
logthreadnames logtimestamps named natpmp netinfo nodebuglogfile peerblockfilters peerbloomfilters
persistmempool printtoconsole regtest reindex reindex-chainstate rest rpcwait server shrinkdebugfile signet
stdin stdinrpcpass stdinwalletpassphrase testnet testnet4 txindex upnp version walletbroadcast";

/// `_ARG_END`.
fn arg_end(text: &[char], pos: usize) -> bool {
    pos == text.len()
        || matches!(
            text[pos],
            ' ' | '\t' | '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
}

/// `_NEXT_FLAG.match(text, pos)`.
fn next_flag(text: &[char], pos: usize) -> bool {
    let n = text.len();
    let run_end = |mut end: usize| {
        while end < n && is_ident(text[end]) {
            end += 1;
        }
        end
    };
    if starts(text, pos, "--") && text.get(pos + 2).is_some_and(char::is_ascii_alphabetic) {
        let end = run_end(pos + 3);
        if end == n || in_set(text[end], "= \t\r\n\u{85}\u{2028}\u{2029}") {
            return true;
        }
    }
    if text.get(pos) != Some(&'-') || !text.get(pos + 1).is_some_and(char::is_ascii_alphabetic) {
        return false;
    }
    if text.get(run_end(pos + 2)) == Some(&'=') {
        return true;
    }
    if matches!(text.get(pos + 2), Some(' ' | '\t')) {
        let mut name = pos + 2;
        while name < n && matches!(text[name], ' ' | '\t') {
            name += 1;
        }
        if text
            .get(name)
            .is_some_and(|&c| c.is_ascii_alphabetic() || c == '_')
        {
            let mut end = name + 1;
            while end < n && is_word(text[end]) {
                end += 1;
            }
            if text.get(end) == Some(&'=') {
                return true;
            }
        }
    }
    let mut heads = vec![pos + 1];
    if starts(text, pos + 1, "no") {
        heads.insert(0, pos + 3);
    }
    heads.into_iter().any(|head| {
        CORE_FLAGS
            .split_whitespace()
            .any(|flag| starts(text, head, flag) && arg_end(text, head + flag.len()))
    })
}

/// `_ARGV_ASSIGNMENT.match(text, pos)`.
fn argv_assignment(text: &[char], pos: usize) -> bool {
    if !text
        .get(pos)
        .is_some_and(|&c| c.is_ascii_alphabetic() || c == '_')
    {
        return false;
    }
    let mut end = pos + 1;
    while end < text.len() && is_ident(text[end]) {
        end += 1;
    }
    text.get(end) == Some(&'=')
}

fn flag_value_end(text: &[char], mut end: usize) -> bool {
    if end >= text.len() || !matches!(text[end], ' ' | '\t') || text[end - 1] == '\\' {
        return false;
    }
    while end < text.len() && matches!(text[end], ' ' | '\t') {
        end += 1;
    }
    next_flag(text, end)
}

fn argv_boundary(text: &[char], pos: usize) -> bool {
    next_flag(text, pos) || argv_assignment(text, pos)
}

fn flag_value_gap(text: &[char], key_start: usize, after_key: usize, pos: usize) -> bool {
    if !sl(text, after_key, pos).iter().any(|&c| is_line(c)) {
        return true;
    }
    let mut line = key_start;
    while line > 0 && !is_line(text[line - 1]) {
        line -= 1;
    }
    strip(&text[line..key_start], |c| in_set(c, " \t#>$.-")).is_empty()
}

fn ambiguous_tail(text: &[char], end: usize) -> bool {
    let n = text.len();
    let mut pos = end;
    while pos < n && matches!(text[pos], ' ' | '\t') {
        pos += 1;
    }
    if pos == end || pos == n || is_line(text[pos]) {
        return false;
    }
    let mut tail = pos;
    while tail < n && !is_space(text[tail]) {
        tail += 1;
    }
    let token = &text[pos..tail];
    !readable_tail(token)
}

/// `[a-z]+[.,!?]?|-{1,2}[a-z][a-z-]*|[a-z0-9_.-]+[:=].*` as a full match.
fn readable_tail(token: &[char]) -> bool {
    let letters = token.iter().take_while(|c| c.is_ascii_lowercase()).count();
    if letters > 0
        && (letters == token.len()
            || (letters + 1 == token.len() && in_set(token[letters], ".,!?")))
    {
        return true;
    }
    for dashes in [2, 1] {
        if token.len() > dashes
            && token[..dashes].iter().all(|&c| c == '-')
            && token[dashes].is_ascii_lowercase()
            && token[dashes + 1..]
                .iter()
                .all(|&c| c.is_ascii_lowercase() || c == '-')
        {
            return true;
        }
    }
    let run = token
        .iter()
        .take_while(|&&c| c.is_ascii_lowercase() || c.is_ascii_digit() || in_set(c, "_.-"))
        .count();
    run > 0 && token.get(run).is_some_and(|&c| in_set(c, ":=")) && !token[run..].contains(&'\n')
}

const HELP_VALUES: &[&str] = &[
    "to set a value.",
    "instead.",
    "flag is deprecated.",
    "flag is available.",
    "option is deprecated.",
    "option is available.",
    "to",
    "instead",
    "flag",
    "option",
    "none",
    "missing",
    "the",
];

fn help_value(value: &[char]) -> bool {
    HELP_VALUES.iter().any(|help| {
        value.len() == help.len()
            && value
                .iter()
                .zip(help.chars())
                .all(|(c, h)| c.eq_ignore_ascii_case(&h))
    })
}

fn alias_prose(name: &str, value: &[char]) -> bool {
    // A single lowercase password is still a password. Only known status/user
    // prose can exempt a weak alias, and only after a spaced colon / XML tag.
    let value = string(value);
    let allowed: &[&str] = if name == "user" {
        &["alice", "bob", "root", "anonymous", "alice logged in"]
    } else if matches!(name, "auth" | "mfa" | "twofa") {
        &["ok", "required", "enabled", "disabled", "none", "expired"]
    } else {
        &[]
    };
    allowed.contains(&value.as_str())
}

fn yaml_marker(text: &[char], mut pos: usize) -> bool {
    let n = text.len();
    pos += 1;
    if pos < n && in_set(text[pos], "-+") {
        pos += 1;
    }
    while pos < n && is_space(text[pos]) && !is_line(text[pos]) {
        pos += 1;
    }
    pos == n || is_line(text[pos])
}

fn balanced_end(text: &[char], start: usize, lines: &mut Lines<'_>) -> usize {
    let mut stack: Vec<char> = Vec::new();
    // An incomplete opener owns its indented continuation, not later records.
    let mut limit = lines.yaml_end(start);
    let following = limit + if starts(text, limit, "\r\n") { 2 } else { 1 };
    if following < text.len() && in_set(text[following], "}])") {
        limit = line_end(text, following);
    }
    let mut pos = start + usize::from(starts(text, start, "$("));
    while pos < limit {
        let ch = text[pos];
        match ch {
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '(' => stack.push(')'),
            '}' | ']' | ')' => {
                if stack.last() != Some(&ch) {
                    return line_end(text, pos);
                }
                stack.pop();
                if stack.is_empty() {
                    return pos + 1;
                }
            }
            '"' | '\'' => {
                let (end, closed) = quoted_end(text, pos + 1, &[ch], Some(limit));
                if !closed {
                    return limit;
                }
                pos = end;
                continue;
            }
            _ => {}
        }
        pos += 1;
    }
    limit
}

fn yaml_end(text: &[char], start: usize) -> usize {
    let mut end = line_end(text, start);
    while end < text.len() {
        let pos = end + if starts(text, end, "\r\n") { 2 } else { 1 };
        if pos < text.len() && !matches!(text[pos], ' ' | '\t') && !is_line(text[pos]) {
            break;
        }
        end = line_end(text, pos);
    }
    end
}

/// `re.search(r"(?:^|[^A-Za-z])tests[ \t]+$", window)`.
fn after_tests(window: &[char]) -> bool {
    let mut ends = vec![window.len()];
    if window.last() == Some(&'\n') {
        ends.push(window.len() - 1);
    }
    ends.into_iter().any(|end| {
        let mut word_end = end;
        while word_end > 0 && matches!(window[word_end - 1], ' ' | '\t') {
            word_end -= 1;
        }
        word_end < end
            && word_end >= 5
            && starts(window, word_end - 5, "tests")
            && (word_end == 5 || !window[word_end - 6].is_ascii_alphabetic())
    })
}

fn keyed_spans(text: &[char], offsets: &Offsets) -> Vec<Span> {
    let v = vocabulary();
    let n = text.len();
    let mut spans: Vec<Span> = Vec::new();
    let mut cursor = 0;
    let commands = command_matches(text);
    let mut command_index = 0;
    let mut command_end = commands.first().map(|c| line_end(text, c.1));
    let heads = url_heads(text);
    let mut head_index = 0;
    let mut head: Option<(usize, usize, bool)> = heads.first().copied();
    let mut previous: Option<(usize, usize)> = None;
    let (mut line_start, mut scanned, mut indent) = (0, 0, 0);
    let mut line_seen: Option<usize> = None;
    let mut lines = Lines::new(text);

    let mut p = 0;
    while p < n {
        if !is_ident(text[p]) {
            p += 1;
            continue;
        }
        let (ms, me) = (p, ident_match(text, p).unwrap_or(p + 1));
        p = me;
        if ms < cursor {
            continue;
        }
        let key = &text[ms..me];
        let before = previous;
        previous = Some((ms, me));
        while let Some(end) = command_end.filter(|&end| end < ms) {
            let _ = end;
            command_index += 1;
            command_end = commands.get(command_index).map(|c| line_end(text, c.1));
        }
        let in_command = commands
            .get(command_index)
            .is_some_and(|c| c.0 <= ms && command_end.is_some_and(|end| ms <= end));
        let mut name = secret_name(key);
        let key_text = string(key);
        if matches!(key_text.as_str(), "-u" | "-b")
            || (in_command && matches!(key_text.as_str(), "--user" | "--cookie"))
        {
            name = if matches!(key_text.as_str(), "-b" | "--cookie") {
                "cookie"
            } else {
                "credentials"
            }
            .to_owned();
        }
        if name.is_empty() && lower_is(key, "key") && ms > 0 && in_set(text[ms - 1], "?&") {
            name = "apikey".to_owned();
        }
        if name.is_empty() {
            if let Some((bs, be)) = before {
                if matches!(sl(text, be, ms), [' '] | ['\t']) {
                    // Spaced names (`api key:`, `"Secret Key":`) still name a secret
                    // when the second word changes what the first one means.
                    let mut joined = text[bs..be].to_vec();
                    joined.push('_');
                    joined.extend_from_slice(key);
                    name = secret_name(&joined);
                    if name == secret_name(&text[bs..be]) {
                        name.clear();
                    }
                }
            }
        }
        if name.is_empty() {
            continue;
        }
        while scanned < ms {
            if is_line(text[scanned]) {
                line_start = scanned + 1;
            }
            scanned += 1;
        }
        if line_seen != Some(line_start) {
            line_seen = Some(line_start);
            indent = line_indent(text, line_start);
        }
        while let Some((_, head_end, _)) = head.filter(|h| h.1 < ms) {
            let _ = head_end;
            head_index += 1;
            head = heads.get(head_index).copied();
        }
        let (head_start, head_end, credential_context) = head.unwrap_or((usize::MAX, 0, false));
        let in_url = head.is_some() && head_start <= ms && ms < head_end;
        let mut pos = quote_at(text, me);
        let key_quote = pos > me;
        let after_key = pos;
        while pos < n && is_space(text[pos]) {
            pos += 1;
        }
        let mut flag = false;
        let mut separator = '\0';
        if pos < n && in_set(text[pos], ":=") {
            separator = text[pos];
            if in_url
                && (ms == head_start || v.weak.contains(name.as_str()))
                && separator == ':'
                && pos + 1 < n
                && !is_space(text[pos + 1])
                && (credential_context || all_digits(sl(text, pos + 1, head_end)))
            {
                continue;
            }
            pos += 1;
            if pos < n && matches!((separator, text[pos]), ('=', '>') | (':', '=')) {
                separator = '=';
                pos += 1;
            }
            while pos < n && is_space(text[pos]) {
                pos += 1;
            }
        } else if starts(text, pos, "->") {
            separator = '=';
            pos += 2;
            while pos < n && matches!(text[pos], ' ' | '\t') {
                pos += 1;
            }
        } else if (key[0] == '-' || in_command)
            && pos > after_key
            && flag_value_gap(text, ms, after_key, pos)
        {
            flag = true;
        } else {
            continue;
        }
        let start = pos;
        while pos < n && in_set(text[pos], ":=>") && !(text[pos] == '>' && yaml_marker(text, pos)) {
            pos += 1;
            while pos < n && matches!(text[pos], ' ' | '\t') {
                pos += 1;
            }
        }
        if pos == n {
            continue;
        }
        let body = quote_at(text, pos);
        let end;
        let resume;
        let replacement;
        if body > pos {
            let mut delimiter = text[pos..body].to_vec();
            let closed;
            let tripled = delimiter.repeat(3);
            if starts_chars(text, pos, &tripled) {
                delimiter = tripled;
                let body = pos + delimiter.len();
                match find_chars(text, &delimiter, body) {
                    Some(close) => {
                        end = close + delimiter.len();
                        closed = true;
                    }
                    None => {
                        end = lines.yaml_end(body);
                        closed = false;
                    }
                }
            } else {
                (end, closed) = quoted_end(text, body, &delimiter, None);
            }
            let end =
                if closed && end < n && (!value_boundary(text, end) || ambiguous_tail(text, end)) {
                    lines.yaml_end(end)
                } else {
                    end
                };
            replacement = if start == pos {
                let mut masked = string(&delimiter);
                masked.push_str("[redacted]");
                if closed {
                    masked.push_str(&string(&delimiter));
                }
                masked
            } else {
                "[redacted]".to_owned()
            };
            resume = end;
            if end <= start {
                continue;
            }
            spans.push((offsets.at(start), offsets.at(end), replacement));
            cursor = resume;
            continue;
        }
        let argv = key[0] == '-' || in_command;
        if let Some(literal_end_at) = literal_match(text, pos) {
            if literal_end(text, literal_end_at, indent, &mut lines, argv) {
                // A bare count or boolean after a count-shaped name
                // (`max_tokens: 16`, `descriptors=true`) and JSON null,
                // booleans, sampling seeds and counts are metadata. A string
                // value never is.
                let value = &text[pos..literal_end_at];
                let value_text = string(value);
                let is_bool = matches!(value_text.as_str(), "true" | "false" | "True" | "False");
                if ((count_value(value) || is_bool) && count_name(key, &name))
                    || (key_quote
                        && (matches!(value_text.as_str(), "null" | "None")
                            || is_bool
                            || name == "seed"
                            || count_name(key, &name)))
                {
                    cursor = literal_end_at;
                    continue;
                }
                if key_quote {
                    // A JSON literal ends unambiguously; later fields stay readable.
                    spans.push((
                        offsets.at(start),
                        offsets.at(literal_end_at),
                        "[redacted]".to_owned(),
                    ));
                    cursor = literal_end_at;
                    continue;
                }
            }
        }
        if key_quote && text[pos] == '{' && count_name(key, &name) && count_object(text, pos) {
            // `"prompt_tokens_details": {"cached_tokens": 0}`: each member
            // is decided on its own key.
            continue;
        }
        let mut token_end = pos;
        while token_end < n
            && !matches!(text[token_end], ' ' | '\t' | '\x0b' | '\x0c' | '"' | '\'')
            && !is_line(text[token_end])
        {
            token_end += 1;
        }
        let token = &text[pos..token_end];
        // A plain value owns lines indented deeper than its line (a YAML or
        // shell continuation), so neither exceptions nor masks stop at the first.
        let value_line_end = lines.continued_end(pos, indent);
        // Exceptions describe the entire value. A familiar first word never
        // licenses the rest of an otherwise unquoted credential.
        let whole = (value_line_end - pos <= 256)
            .then(|| strip(&text[pos..value_line_end], |c| matches!(c, ' ' | '\t')));
        if let Some(whole) = whole {
            if separator == ':'
                && !key_quote
                && (lower_is(whole, "none")
                    || lower_is(whole, "expired")
                    || lower_is(whole, "(not set)")
                    || (v.weak.contains(name.as_str()) && alias_prose(&name, whole))
                    || (name == "pass"
                        && all_digits(whole)
                        && after_tests(sl(text, ms.saturating_sub(64), ms))))
            {
                continue;
            }
            if flag
                && ((!in_command && help_value(whole))
                    || (name == "seedwords"
                        && matches!(string(whole).as_str(), "12" | "15" | "18" | "21" | "24")))
            {
                continue;
            }
        }
        let mut end;
        let mut resume = token_end;
        let token_text = string(token);
        if matches!(text[pos], '{' | '[' | '(')
            || starts(text, pos, "$(")
            || starts(text, pos, "${")
        {
            end = balanced_end(text, pos, &mut lines);
            if !strip(sl(text, end, line_end(text, end)), |c| {
                matches!(c, ' ' | '\t')
            })
            .is_empty()
            {
                end = line_end(text, end);
            }
            resume = end;
        } else if matches!(token_text.as_str(), "|" | "|-" | "|+" | ">" | ">-" | ">+")
            || matches!(text[pos], '`' | '<')
        {
            end = lines.yaml_end(pos);
            resume = end;
        } else if argv && flag_value_end(text, token_end) {
            // Whitespace before a known flag ends an argv value.
            end = token_end;
        } else {
            // Without a value opener, quotes and words are credential data,
            // including a stray quote followed by a multi-word passphrase.
            end = value_line_end;
        }
        let header = name == "authorization"
            || (matches!(name.as_str(), "cookie" | "cookies") && !key_quote && key[0] != '-');
        if header {
            end = lines.yaml_end(pos);
        }
        while end > pos && matches!(text[end - 1], ' ' | '\t') {
            end -= 1;
        }
        if end <= start {
            continue;
        }
        spans.push((offsets.at(start), offsets.at(end), "[redacted]".to_owned()));
        // Later keys on a plain value's line still establish their own blocks.
        cursor = resume;
    }
    spans
}

// --- named pairs, XML and CLI ----------------------------------------------

/// `_PAIR.finditer(text)`: each match's span and its name group.
fn pair_matches(text: &[char]) -> Vec<(usize, usize, usize, usize)> {
    let n = text.len();
    let quote = |i: usize| text.get(i).is_some_and(|&c| c == '"' || c == '\'');
    let lit = |i: usize, word: &str| {
        word.chars()
            .enumerate()
            .all(|(k, w)| text.get(i + k).is_some_and(|c| c.eq_ignore_ascii_case(&w)))
    };
    let spaces = |mut i: usize| {
        while i < n && matches!(text[i], ' ' | '\t') {
            i += 1;
        }
        i
    };
    let run = |mut i: usize| {
        while i < n && is_ident(text[i]) {
            i += 1;
        }
        i
    };
    // The tail after the name group: `["']?[,ws]*["']?value["']?[ \t]*[:=][ \t]*`.
    let tail = |mut i: usize| -> Option<usize> {
        if quote(i) {
            i += 1;
        }
        while i < n
            && matches!(
                text[i],
                ',' | ' '
                    | '\t'
                    | '\r'
                    | '\n'
                    | '\x0b'
                    | '\x0c'
                    | '\u{85}'
                    | '\u{2028}'
                    | '\u{2029}'
            )
        {
            i += 1;
        }
        if quote(i) {
            i += 1;
        }
        if !lit(i, "value") {
            return None;
        }
        i += 5;
        if quote(i) {
            i += 1;
        }
        i = spaces(i);
        if !text.get(i).is_some_and(|&c| c == ':' || c == '=') {
            return None;
        }
        Some(spaces(i + 1))
    };
    let attempt = |p: usize| -> Option<(usize, usize, usize, usize)> {
        if p > 0 && is_ident(text[p - 1]) {
            return None;
        }
        let mut i = p;
        if quote(i) {
            i += 1;
        }
        if lit(i, "name") {
            i += 4;
        } else if lit(i, "key") {
            i += 3;
        } else {
            return None;
        }
        if quote(i) {
            i += 1;
        }
        i = spaces(i);
        if !text.get(i).is_some_and(|&c| c == ':' || c == '=') {
            return None;
        }
        i = spaces(i + 1);
        if quote(i) {
            i += 1;
        }
        let group = i;
        let first = run(group);
        if first == group {
            return None;
        }
        if text.get(first) == Some(&':') {
            let second = run(first + 1);
            for end in (first + 2..=second).rev() {
                if let Some(match_end) = tail(end) {
                    return Some((p, match_end, group, end));
                }
            }
        }
        for end in (group + 1..=first).rev() {
            if let Some(match_end) = tail(end) {
                return Some((p, match_end, group, end));
            }
        }
        None
    };
    let mut matches = Vec::new();
    let mut p = 0;
    while p < n {
        if let Some(found) = attempt(p) {
            p = found.1;
            matches.push(found);
        } else {
            p += 1;
        }
    }
    matches
}

fn named_pair_spans(text: &[char], offsets: &Offsets) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    for (pair_start, start, group_start, group_end) in pair_matches(text) {
        if pair_start < cursor || secret_name(&text[group_start..group_end]).is_empty() {
            continue;
        }
        let body = quote_at(text, start);
        let (end, replacement) = if body > start {
            let delimiter = &text[start..body];
            let (end, closed) = quoted_end(text, body, delimiter, None);
            let mut masked = string(delimiter);
            masked.push_str("[redacted]");
            if closed {
                masked.push_str(&string(delimiter));
            }
            (end, masked)
        } else {
            (yaml_end(text, start), "[redacted]".to_owned())
        };
        spans.push((offsets.at(start), offsets.at(end), replacement));
        cursor = end;
    }
    spans
}

const VOID_TAGS: &str = "area base br col embed hr img input link meta param source track wbr";

struct Tag {
    start: usize,
    end: usize,
    closing: bool,
    name: (usize, usize),
    attrs: (usize, usize),
}

fn is_tag_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')
}

/// `_TAG.finditer(text)`.
fn tag_matches(text: &[char]) -> Vec<Tag> {
    let n = text.len();
    let mut tags = Vec::new();
    let mut p = 0;
    while p < n {
        if text[p] != '<' {
            p += 1;
            continue;
        }
        let mut i = p + 1;
        let closing =
            text.get(i) == Some(&'/') && text.get(i + 1).is_some_and(char::is_ascii_alphabetic);
        if closing {
            i += 1;
        }
        if !text.get(i).is_some_and(char::is_ascii_alphabetic) {
            p += 1;
            continue;
        }
        let name_start = i;
        while i < n && is_tag_char(text[i]) {
            i += 1;
        }
        let name_end = i;
        while i < n && !matches!(text[i], '<' | '>') {
            i += 1;
        }
        if i < n && text[i] == '>' {
            tags.push(Tag {
                start: p,
                end: i + 1,
                closing,
                name: (name_start, name_end),
                attrs: (name_end, i),
            });
            p = i + 1;
        } else {
            p += 1;
        }
    }
    tags
}

/// `_ATTR.finditer(attrs)`: name and value spans within `attrs`.
fn attr_matches(attrs: &[char]) -> Vec<(Range, Range)> {
    let n = attrs.len();
    let ws = |mut i: usize| {
        while i < n && is_xml_space(attrs[i]) {
            i += 1;
        }
        i
    };
    let attempt = |q: usize| -> Option<(Range, Range, usize)> {
        if q > 0 && is_tag_char(attrs[q - 1]) {
            return None;
        }
        if !(attrs[q].is_ascii_alphabetic() || matches!(attrs[q], '_' | ':')) {
            return None;
        }
        let mut i = q + 1;
        while i < n && is_tag_char(attrs[i]) {
            i += 1;
        }
        let name = (q, i);
        i = ws(i);
        if attrs.get(i) != Some(&'=') {
            return None;
        }
        i = ws(i + 1);
        if let Some(&quote) = attrs.get(i).filter(|&&c| c == '"' || c == '\'') {
            if let Some(close) = find_char(attrs, quote, i + 1, n) {
                return Some((name, (i + 1, close), close + 1));
            }
        }
        let mut end = i;
        while end < n && !is_xml_space(attrs[end]) && attrs[end] != '>' {
            end += 1;
        }
        (end > i).then_some((name, (i, end), end))
    };
    let mut found = Vec::new();
    let mut q = 0;
    while q < n {
        if let Some((name, value, end)) = attempt(q) {
            found.push((name, value));
            q = end;
        } else {
            q += 1;
        }
    }
    found
}

/// `text.find(needle, from)`, remembering the last answer per needle: tags
/// are visited in order, so an earlier answer at or after `from` still holds.
#[derive(Default)]
struct Finder {
    found: HashMap<Vec<char>, (usize, Option<usize>)>,
}

impl Finder {
    fn find(&mut self, text: &[char], needle: &[char], from: usize) -> Option<usize> {
        if let Some(&(searched, result)) = self.found.get(needle) {
            if searched <= from && result.is_none_or(|at| at >= from) {
                return result;
            }
        }
        let result = find_chars(text, needle, from);
        self.found.insert(needle.to_vec(), (from, result));
        result
    }
}

fn xml_spans(text: &[char], offsets: &Offsets) -> Vec<Span> {
    let v = vocabulary();
    let mut spans = Vec::new();
    let mut cursor = 0;
    let mut plist_secret = false;
    let mut finder = Finder::default();
    for tag in tag_matches(text) {
        if tag.start < cursor || tag.closing {
            continue;
        }
        let key = &text[tag.name.0..tag.name.1];
        let attrs_text = &text[tag.attrs.0..tag.attrs.1];
        if matches!(lstrip(attrs_text, is_xml_space).first(), Some(':' | '-')) {
            continue;
        }
        let local = key.rsplit(|&c| c == ':').next().unwrap_or(key);
        let mut name = secret_name(local);
        let attrs: Vec<(String, &[char], usize, usize)> = attr_matches(attrs_text)
            .into_iter()
            .map(|((ns, ne), (vs, ve))| {
                let attr_name = &attrs_text[ns..ne];
                let attr_local = attr_name.rsplit(|&c| c == ':').next().unwrap_or(attr_name);
                (
                    string(attr_local).to_ascii_lowercase(),
                    &attrs_text[vs..ve],
                    vs,
                    ve,
                )
            })
            .collect();
        let mut keyed = attrs.iter().any(|(k, value, _, _)| {
            matches!(k.as_str(), "key" | "name" | "type") && !secret_name(value).is_empty()
        });
        // Even malformed XML like <entrykey="password"value="..."> declares
        // a secret context; spacing mistakes cannot exempt the value.
        if attrs_text.first() == Some(&'=') {
            let quoted = attrs_text.get(1).is_some_and(|&c| c == '"' || c == '\'');
            if quoted {
                let run = attrs_text[2..]
                    .iter()
                    .take_while(|&&c| c != '"' && c != '\'')
                    .count();
                let selector =
                    (run > 0 && 2 + run < attrs_text.len()).then(|| &attrs_text[2..2 + run]);
                keyed = keyed || selector.is_some_and(|s| !secret_name(s).is_empty());
            }
        }
        if keyed || !name.is_empty() {
            for (attr_key, value, value_start, value_end) in &attrs {
                if !matches!(attr_key.as_str(), "value" | "content")
                    || (!keyed && v.weak.contains(name.as_str()) && alias_prose(&name, value))
                {
                    continue;
                }
                let start = tag.attrs.0 + value_start;
                let end = tag.attrs.0 + value_end;
                spans.push((offsets.at(start), offsets.at(end), "[redacted]".to_owned()));
                cursor = end;
            }
        }
        let mut closing: Vec<char> = vec!['<', '/'];
        closing.extend_from_slice(key);
        if key == ['k', 'e', 'y'] {
            let end = finder.find(text, &closing, tag.end);
            plist_secret = end.is_some_and(|end| {
                !secret_name(strip(&text[tag.end..end], is_xml_space)).is_empty()
            });
            continue;
        }
        if name.is_empty() && plist_secret {
            name = "secret".to_owned();
        }
        plist_secret = false;
        if name.is_empty() && !keyed {
            continue;
        }
        if rstrip_by(attrs_text, is_xml_space).last() == Some(&'/') {
            continue;
        }
        let mut end = finder.find(text, &closing, tag.end);
        while let Some(at) = end {
            let mut tail = at + closing.len();
            while tail < text.len() && is_xml_space(text[tail]) {
                tail += 1;
            }
            if tail < text.len() && text[tail] == '>' {
                break;
            }
            end = finder.find(text, &closing, tail);
        }
        let end = match end {
            Some(end) => end,
            None => {
                // HTML void elements such as <input> and <meta> have no body.
                let lower = string(key).to_ascii_lowercase();
                if VOID_TAGS.split_whitespace().any(|tag| tag == lower) {
                    continue;
                }
                yaml_end(text, tag.end)
            }
        };
        let value = sl(text, tag.end, end);
        if !keyed
            && v.weak.contains(name.as_str())
            && alias_prose(&name, strip(value, is_xml_space))
        {
            continue;
        }
        spans.push((
            offsets.at(tag.end),
            offsets.at(end),
            "[redacted]".to_owned(),
        ));
        cursor = end;
    }
    spans
}

fn rstrip_by(text: &[char], set: impl Fn(char) -> bool) -> &[char] {
    let mut b = text.len();
    while b > 0 && set(text[b - 1]) {
        b -= 1;
    }
    &text[..b]
}

/// The CLI scanner's regular expression at `p`: the match end.
fn cli_at(text: &[char], p: usize) -> Option<usize> {
    let n = text.len();
    if p > 0 && is_word(text[p - 1]) {
        return None;
    }
    let blanks = |mut i: usize| {
        while i < n && matches!(text[i], ' ' | '\t') {
            i += 1;
        }
        i
    };
    for (command, flag) in [("curl", "-u"), ("sshpass", "-p")] {
        if starts(text, p, command) {
            let i = p + command.len();
            let j = blanks(i);
            if j > i && starts(text, j, flag) {
                return Some(blanks(j + 2));
            }
        }
    }
    if starts(text, p, "mysql") {
        let i = p + 5;
        let j = blanks(i);
        if j > i && starts(text, j, "-u") {
            let k = blanks(j + 2);
            if k > j + 2 {
                let mut user = k;
                while user < n && !matches!(text[user], ' ' | '\t' | '\r' | '\n') {
                    user += 1;
                }
                let gap = blanks(user);
                if user > k && gap > user && starts(text, gap, "-p") {
                    return Some(gap + 2);
                }
            }
        }
    }
    None
}

fn cli_spans(text: &[char], offsets: &Offsets) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    let mut p = 0;
    while p < text.len() {
        let Some(start) = cli_at(text, p) else {
            p += 1;
            continue;
        };
        let match_start = p;
        p = start.max(p + 1);
        if match_start < cursor {
            continue;
        }
        let body = quote_at(text, start);
        let (end, replacement) = if body > start {
            let delimiter = &text[start..body];
            let (end, closed) = quoted_end(text, body, delimiter, None);
            let mut masked = string(delimiter);
            masked.push_str("[redacted]");
            if closed {
                masked.push_str(&string(delimiter));
            }
            (end, masked)
        } else {
            (line_end(text, start), "[redacted]".to_owned())
        };
        spans.push((offsets.at(start), offsets.at(end), replacement));
        cursor = end;
    }
    spans
}

// --- the decoded view ------------------------------------------------------

/// The HTML entity at the start of `window`: its code point and length.
fn entity(window: &[char]) -> Option<(u32, usize)> {
    if window.first() != Some(&'&') {
        return None;
    }
    if window.get(1) == Some(&'#') {
        if matches!(window.get(2), Some('x' | 'X')) {
            let digits = window[3..].iter().take_while(|c| is_hex(**c)).count();
            if (1..=6).contains(&digits) && window.get(3 + digits) == Some(&';') {
                let code = u32::from_str_radix(&string(&window[3..3 + digits]), 16).ok()?;
                return Some((code, 4 + digits));
            }
        }
        let digits = window[2..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if (1..=7).contains(&digits) && window.get(2 + digits) == Some(&';') {
            let code = string(&window[2..2 + digits]).parse().ok()?;
            return Some((code, 3 + digits));
        }
        return None;
    }
    for (name, code) in [
        ("colon", 58),
        ("sol", 47),
        ("commat", 64),
        ("quot", 34),
        ("apos", 39),
        ("lt", 60),
        ("gt", 62),
        ("amp", 38),
        ("equals", 61),
        ("Tab", 9),
        ("NewLine", 10),
    ] {
        if starts(window, 1, name) && window.get(1 + name.len()) == Some(&';') {
            return Some((code, 2 + name.len()));
        }
    }
    None
}

/// `chr(code)`. A surrogate stands in as U+FFFD: no rule classifies either,
/// and a decoded character never compares equal to the original it spans.
fn code_point(code: u32) -> char {
    char::from_u32(code).unwrap_or('\u{fffd}')
}

/// Decode credential syntax, retaining an offset for every original span.
///
/// Three percent layers are a fixed linear envelope. JSON slash escape runs
/// need no depth cap: each run is consumed once, without decoding whole JSON.
fn url_view(text: &[char]) -> (Vec<char>, Offsets) {
    let plain = !text.iter().any(|&c| {
        matches!(c, '%' | '\\' | '&' | 'ſ' | '﹫') || ('\u{ff01}'..='\u{ff5e}').contains(&c)
    });
    if plain {
        return (text.to_vec(), Offsets::identity());
    }
    let mut text = text.to_vec();
    let mut offsets: Vec<usize> = (0..=text.len()).collect();
    for _ in 0..3 {
        let mut decoded: Vec<char> = Vec::with_capacity(text.len());
        let mut positions: Vec<usize> = Vec::with_capacity(text.len() + 1);
        let n = text.len();
        let mut pos = 0;
        while pos < n {
            positions.push(offsets[pos]);
            let found = if text[pos] == '&' {
                entity(&text[pos..(pos + 14).min(n)])
            } else {
                None
            };
            if let Some((code, length)) = found {
                decoded.push(if code <= 0x10ffff {
                    code_point(code)
                } else {
                    '\u{fffd}'
                });
                pos += length;
            } else if text[pos] == '%'
                && pos + 2 < n
                && is_hex(text[pos + 1])
                && is_hex(text[pos + 2])
            {
                let code = u32::from_str_radix(&string(&text[pos + 1..pos + 3]), 16).unwrap_or(0);
                decoded.push(code_point(code));
                pos += 3;
            } else if text[pos] == '\\' {
                let mut end = pos;
                while end < n && text[end] == '\\' {
                    end += 1;
                }
                let width = match text.get(end) {
                    Some('u') => 5,
                    Some('x') => 3,
                    _ => 0,
                };
                if end < n && text[end] == '/' {
                    decoded.push('/');
                    pos = end + 1;
                } else if width > 0
                    && end + width <= n
                    && text[end + 1..end + width].iter().all(|c| is_hex(*c))
                {
                    let code =
                        u32::from_str_radix(&string(&text[end + 1..end + width]), 16).unwrap_or(0);
                    decoded.push(code_point(code));
                    pos = end + width;
                } else {
                    decoded.extend_from_slice(&text[pos..end]);
                    positions.extend_from_slice(&offsets[pos + 1..end]);
                    pos = end;
                }
            } else {
                let ch = text[pos];
                decoded.push(if ('\u{ff01}'..='\u{ff5e}').contains(&ch) {
                    char::from_u32(u32::from(ch) - 0xfee0).unwrap_or(ch)
                } else if ch == 'ſ' {
                    's'
                } else if ch == '﹫' {
                    '@'
                } else {
                    ch
                });
                pos += 1;
            }
        }
        positions.push(offsets[n]);
        if decoded == text {
            break;
        }
        text = decoded;
        offsets = positions;
    }
    (text, Offsets(Some(offsets)))
}

// --- URLs ------------------------------------------------------------------

/// Maximal runs of `[A-Za-z0-9_]` at least `min` long.
fn word_runs(text: &[char], min: usize) -> Vec<&[char]> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if !is_word(text[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < text.len() && is_word(text[i]) {
            i += 1;
        }
        if i - start >= min {
            runs.push(&text[start..i]);
        }
    }
    runs
}

fn has_upper_run(text: &[char], min: usize) -> bool {
    let mut run = 0;
    for &c in text {
        run = if c.is_ascii_uppercase() { run + 1 } else { 0 };
        if run >= min {
            return true;
        }
    }
    false
}

fn mixed_run(text: &[char]) -> bool {
    word_runs(text, 6).iter().any(|part| {
        part.iter().any(char::is_ascii_alphabetic) && part.iter().any(char::is_ascii_digit)
    })
}

fn opaque_token(text: &[char]) -> bool {
    if !word_runs(text, 10).is_empty() || has_upper_run(text, 4) {
        return true;
    }
    mixed_run(text)
}

fn strong_opaque(text: &[char]) -> bool {
    // Long lowercase words are ordinary paths; capitals and mixed runs are not.
    has_upper_run(text, 4) || mixed_run(text)
}

fn first_dotted(text: &[char]) -> &[char] {
    text.split(|&c| c == '.').next().unwrap_or(text)
}

fn clear_host(authority: &[char]) -> bool {
    let n = authority.len();
    if authority.first() == Some(&'[') {
        let mut i = 1;
        while i < n
            && (authority[i].is_ascii_hexdigit()
                || authority[i].is_ascii_lowercase()
                || in_set(authority[i], ":.%"))
        {
            i += 1;
        }
        if i > 1 && authority.get(i) == Some(&']') {
            i += 1;
            if i == n
                || (authority[i] == ':' && authority[i + 1..].iter().all(char::is_ascii_digit))
            {
                return true;
            }
        }
    }
    let host = match authority.iter().rposition(|&c| c == ':') {
        None => authority,
        Some(colon) => {
            if !all_digits(&authority[colon + 1..]) {
                return false;
            }
            &authority[..colon]
        }
    };
    let host_shape = !host.is_empty()
        && host[0].is_ascii_alphanumeric()
        && host[host.len() - 1].is_ascii_alphanumeric()
        && host
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    host_shape && !opaque_token(first_dotted(host))
}

/// `_QUERY_VALUE.sub(r"\1", path)`.
fn drop_query_values(path: &[char]) -> Vec<char> {
    let n = path.len();
    let mut out = Vec::with_capacity(n);
    let mut p = 0;
    while p < n {
        if in_set(path[p], "?&;")
            && path
                .get(p + 1)
                .is_some_and(|&c| c.is_ascii_alphabetic() || c == '_')
        {
            let mut name = p + 2;
            while name < n && is_ident(path[name]) {
                name += 1;
            }
            if path.get(name) == Some(&'=') {
                out.extend_from_slice(&path[p..=name]);
                p = name + 1;
                while p < n && !in_set(path[p], "&;#") {
                    p += 1;
                }
                continue;
            }
        }
        out.push(path[p]);
        p += 1;
    }
    out
}

const STRUCTURE: &str = "\\{}[]\"'<>|";

fn public_url_prefix(prefix: &[char], original: &[char], offsets: &Offsets, start: usize) -> bool {
    // An @ in an ordinary path, handle, package reference or following email
    // cannot create userinfo. Opaque tokens and user:pass shapes stay ambiguous.
    let Some(boundary) = prefix
        .iter()
        .position(|&c| in_set(c, "/?# \t\r\n\x0b\x0c\u{85}\u{2028}\u{2029}"))
    else {
        return false;
    };
    let authority = &prefix[..boundary];
    if !clear_host(authority) {
        return false;
    }
    let tail = &prefix[boundary..];
    // A literal gap following a complete public URL ends that URL, regardless
    // of the length of its path words. Encoded gaps remain credential data.
    // Opaque path words and the words before the later @ stay ambiguous; named
    // query values are left to the keyed scanner.
    let gap = tail.iter().position(|&c| is_gap(c));
    if let Some(gap) = gap {
        let pos = start + boundary + gap;
        if in_set(tail[0], "/?#")
            && sl(original, offsets.at(pos), offsets.at(pos + 1)) == [tail[gap]]
        {
            let (path, after) = (&tail[..gap], &tail[gap..]);
            let words = drop_query_values(path);
            return !(path.contains(&':')
                || path.iter().any(|&c| in_set(c, STRUCTURE))
                || strong_opaque(&words)
                || strong_opaque(after));
        }
    }
    if is_gap(tail[0]) {
        let pos = start + boundary;
        if sl(original, offsets.at(pos), offsets.at(pos + 1)) != [tail[0]] {
            return false;
        }
    }
    let host_name = authority.split(|&c| c == ':').next().unwrap_or(authority);
    if authority.contains(&':')
        && authority.first() != Some(&'[')
        && !(in_set(tail[0], "?#")
            && (authority.contains(&'.') || host_name == chars("localhost").as_slice()))
        && !(is_line(tail[0]) && !opaque_token(tail) && !tail.contains(&':'))
    {
        return false;
    }
    !(tail.contains(&':') || opaque_token(tail) || tail.iter().any(|&c| in_set(c, STRUCTURE)))
}

fn url_line_end(
    view: &[char],
    original: &[char],
    offsets: &Offsets,
    start: usize,
    end: usize,
) -> usize {
    // A literal line break ends a URL after its path starts, unless the next
    // line's first token continues the userinfo with an `@`.
    let mut pos = start;
    let mut authority = true;
    while pos < end {
        let is_literal = literal(view, original, offsets, pos);
        if in_set(view[pos], "/?#") && is_literal {
            authority = false;
        } else if is_line(view[pos]) && is_literal && !authority {
            let mut following = pos;
            while following < end && is_space(view[following]) {
                following += 1;
            }
            let mut token = following;
            while token < end
                && view[token] != '@'
                && !(is_space(view[token]) && literal(view, original, offsets, token))
            {
                token += 1;
            }
            if token == end || view[token] != '@' {
                return pos;
            }
            pos = following;
            continue;
        }
        pos += 1;
    }
    end
}

fn url_candidate_end(
    view: &[char],
    original: &[char],
    offsets: &Offsets,
    start: usize,
    end: usize,
) -> usize {
    let Some(first_at) = find_char(view, '@', start, end) else {
        return end;
    };
    let mut last_at = first_at;
    let mut checked_at = None;
    let mut gap = first_at + 1;
    while gap < end {
        if view[gap] == '@' {
            last_at = gap;
        } else if checked_at != Some(last_at)
            && is_gap(view[gap])
            && literal(view, original, offsets, gap)
        {
            checked_at = Some(last_at);
            let host_part = &view[last_at + 1..gap];
            let tail = &host_part[..host_part
                .iter()
                .position(|&c| in_set(c, "/?#"))
                .unwrap_or(host_part.len())];
            // An encoded @ before a bare word (`%40q (ZK…@h`) may be userinfo
            // data, unless a dotted/port host or ordinary words follow. A line
            // whose first token reaches an @ continues a wrapped password.
            let is_literal = sl(original, offsets.at(last_at), offsets.at(last_at + 1)) == ['@'];
            let following = find_char(view, '@', gap, end);
            if clear_host(tail)
                && (is_literal
                    || tail.first() == Some(&'[')
                    || tail.iter().any(|&c| c == '.' || c == ':')
                    || !(strong_opaque(&view[gap..following.unwrap_or(end)])
                        || wrapped_userinfo(view, original, offsets, gap, end)))
            {
                return gap;
            }
        }
        gap += 1;
    }
    end
}

fn wrapped_userinfo(
    view: &[char],
    original: &[char],
    offsets: &Offsets,
    gap: usize,
    end: usize,
) -> bool {
    if !is_line(view[gap]) {
        return false;
    }
    let mut pos = gap;
    while pos < end && is_space(view[pos]) {
        pos += 1;
    }
    while pos < end
        && view[pos] != '@'
        && !(is_space(view[pos]) && literal(view, original, offsets, pos))
    {
        pos += 1;
    }
    pos < end && view[pos] == '@'
}

fn fragment_userinfo(prefix: &[char]) -> bool {
    // `https://Pw9#Leak@host`: a `#` after a bare token, not a dotted, port or
    // bracketed host, is credential data before the later @.
    match prefix.iter().position(|&c| in_set(c, "/?#")) {
        Some(boundary) => {
            prefix[boundary] == '#' && !prefix[..boundary].iter().any(|&c| in_set(c, ".:["))
        }
        None => false,
    }
}

/// `[^:]*:[0-9]*|sha(?:1|256|384|512):[0-9A-Fa-f]+` as a full match.
fn plain_handle(handle: &[char]) -> bool {
    if let Some(colon) = handle.iter().position(|&c| c == ':') {
        if handle[colon + 1..].iter().all(char::is_ascii_digit) {
            return true;
        }
    }
    ["sha1:", "sha256:", "sha384:", "sha512:"]
        .iter()
        .any(|prefix| {
            starts(handle, 0, prefix)
                && handle.len() > prefix.len()
                && handle[prefix.len()..].iter().all(|c| is_hex(*c))
        })
}

fn url_spans(text: &[char]) -> Vec<Span> {
    let (view, offsets) = url_view(text);
    let matches = scheme_matches(&view);
    let mut spans = Vec::new();
    let mut cursor = 0;
    let mut pending: Option<usize> = None;
    let quotes: Vec<usize> = (0..view.len())
        .filter(|&i| in_set(view[i], "\"'"))
        .collect();
    let mut next_quote = 0;
    let mut previous_quote: Option<usize> = None;
    let mut checked_quote: Option<usize> = None;
    let mut record = false;
    for (index, &(match_start, match_end)) in matches.iter().enumerate() {
        while next_quote < quotes.len() && quotes[next_quote] < match_start {
            previous_quote = Some(quotes[next_quote]);
            next_quote += 1;
        }
        if previous_quote != checked_quote {
            record = record_opener(&view, text, &offsets, previous_quote);
            checked_quote = previous_quote;
        }
        let mut start = match_end;
        if offsets.at(start) < cursor {
            continue;
        }
        let segment_end = matches.get(index + 1).map_or(view.len(), |m| m.0);
        let record_end = url_record_end(&view, text, &offsets, start, segment_end, record);
        let line_end_at = url_line_end(&view, text, &offsets, start, record_end);
        let end = url_candidate_end(&view, text, &offsets, start, line_end_at);
        if find_char(&view, '@', start, end).is_none() {
            // A URL ended by its line cannot merge with a later credential URL.
            if line_end_at == record_end
                && !clear_host(&view[start..end])
                && pending.is_none()
                && !public_url_prefix(&view[start..end], text, &offsets, start)
            {
                pending = Some(start);
            }
            continue;
        }
        let last_at = rfind_char(&view, '@', start, end).unwrap_or(start);
        if public_url_prefix(&view[start..last_at], text, &offsets, start)
            && !fragment_userinfo(&view[start..last_at])
        {
            pending = None;
            // A public @handle path cannot license a `handle:password` pair.
            let handle_len = view[last_at + 1..end]
                .iter()
                .take_while(|&&c| !in_set(c, "/?#") && !is_space(c))
                .count();
            let handle = &view[last_at + 1..last_at + 1 + handle_len];
            if handle.contains(&':') && !plain_handle(handle) {
                spans.push((
                    offsets.at(last_at + 1),
                    offsets.at(last_at + 1 + handle_len),
                    "[redacted-credentials]".to_owned(),
                ));
                cursor = offsets.at(last_at + 1 + handle_len);
            }
            continue;
        }
        if let Some(earlier) = pending {
            start = earlier;
        }
        let host_part = &view[last_at + 1..end];
        let host_tail = &host_part[..host_part
            .iter()
            .position(|&c| in_set(c, "/?#"))
            .unwrap_or(host_part.len())];
        let invalid = sl(&view, start, last_at)
            .iter()
            .any(|&c| is_space(c) || in_set(c, "/[]{}\"<>|"))
            || opaque_token(first_dotted(host_tail));
        let mut finish = last_at + 1;
        if invalid {
            while finish < end && !is_space(view[finish]) && !in_set(view[finish], "/\"'<>") {
                finish += 1;
            }
        }
        let replacement = if invalid {
            "[redacted-credentials]"
        } else {
            "[redacted-credentials]@"
        };
        spans.push((
            offsets.at(start),
            offsets.at(finish),
            replacement.to_owned(),
        ));
        cursor = offsets.at(finish);
        pending = None;
        // A delimiter glued to an unfinished authority (`a@b:#x//@h`) is more
        // userinfo, not a new URL: the next @ extends this span.
        let rest = sl(&view, finish, end);
        if index + 1 < matches.len()
            && end == matches[index + 1].0
            && !clear_host(rstrip(rest, ",;|&"))
            && !(finish..end).any(|gap| is_space(view[gap]) && literal(&view, text, &offsets, gap))
        {
            pending = Some(start);
        }
    }
    spans
}

/// The first `name:` before the last @, found by one forward scan.
fn userinfo_pair(token: &[char], at: usize) -> Option<(usize, usize)> {
    let mut run = 0;
    for (i, &c) in token[..at].iter().enumerate() {
        if !in_set(c, ":=<>\"'({[]@") {
            continue;
        }
        if c == ':' && i > run {
            return Some((run, i + 1));
        }
        run = i + 1;
    }
    None
}

fn bare_userinfo_spans(text: &[char]) -> Vec<Span> {
    let (view, offsets) = url_view(text);
    let mut spans = Vec::new();
    let mut start = 0;
    while start < view.len() {
        let mut end = start;
        while end < view.len() && !(is_gap(view[end]) && literal(&view, text, &offsets, end)) {
            end += 1;
        }
        let mut token = &view[start..end];
        // Userinfo before a later scheme is still `token:token@host`.
        if let Some(&(scheme_start, _)) = scheme_matches(token).first() {
            token = &token[..scheme_start];
        }
        if let Some(at) = rfind_char(token, '@', 0, token.len()).filter(|&at| at > 0) {
            if let Some((pair_start, pair_end)) = userinfo_pair(token, at) {
                let named = !secret_name(&token[pair_start..pair_end - 1]).is_empty();
                let scheme = &token[pair_start..pair_end];
                let mail = ["mailto:", "xmpp:", "sip:", "sips:"]
                    .iter()
                    .any(|prefix| lower_is(scheme, prefix))
                    && !token[pair_end..at].contains(&':');
                if !named && !mail {
                    spans.push((
                        offsets.at(start + pair_start),
                        offsets.at(start + at + 1),
                        "[redacted-credentials]@".to_owned(),
                    ));
                }
            }
        }
        start = end + 1;
    }
    spans
}

// Bot and webhook tokens travel in URL paths. Their extent is ambiguous, so
// the mask runs to the next literal whitespace.

/// `_PATH_TOKEN` at `p`: the match end.
fn path_token_at(text: &[char], p: usize) -> Option<usize> {
    let n = text.len();
    let lit = |i: usize, word: &str| {
        word.chars()
            .enumerate()
            .all(|(k, w)| text.get(i + k).is_some_and(|c| c.eq_ignore_ascii_case(&w)))
    };
    let path_run = |mut i: usize| {
        let from = i;
        while i < n && !matches!(text[i], '/' | ' ' | '\t' | '\r' | '\n' | '\x0b' | '\x0c') {
            i += 1;
        }
        (i > from).then_some(i)
    };
    let digits = |mut i: usize| {
        let from = i;
        while i < n && text[i].is_ascii_digit() {
            i += 1;
        }
        (i - from, i)
    };
    if lit(p, "/bot") {
        let (count, end) = digits(p + 4);
        if count >= 3 && text.get(end) == Some(&':') {
            return Some(p + 4);
        }
    }
    if lit(p, "hooks.slack.com/") {
        let i = p + 16;
        if lit(i, "services/") {
            if let Some(first) = path_run(i + 9).filter(|&e| text.get(e) == Some(&'/')) {
                if let Some(second) = path_run(first + 1).filter(|&e| text.get(e) == Some(&'/')) {
                    return Some(second + 1);
                }
            }
        }
        for word in ["services/", "workflows/", "triggers/"] {
            if lit(i, word) {
                return Some(i + word.len());
            }
        }
    }
    if lit(p, "discord") {
        let mut i = p + 7;
        if lit(i, "app") && lit(i + 3, ".com/api/") {
            i += 3;
        }
        if lit(i, ".com/api/") {
            i += 9;
            if text.get(i).is_some_and(|c| c.eq_ignore_ascii_case(&'v')) {
                let (count, end) = digits(i + 1);
                if count > 0 && text.get(end) == Some(&'/') {
                    i = end + 1;
                }
            }
            if lit(i, "webhooks/") {
                let (count, end) = digits(i + 9);
                if count > 0 && text.get(end) == Some(&'/') {
                    return Some(end + 1);
                }
            }
        }
    }
    if (p == 0 || !is_word(text[p - 1]))
        && ((lit(p, "xox")
            && text.get(p + 3).is_some_and(char::is_ascii_alphabetic)
            && text.get(p + 4) == Some(&'-'))
            || lit(p, "xapp-"))
    {
        return Some(p);
    }
    None
}

fn path_token_spans(text: &[char], offsets: &Offsets) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    let mut p = 0;
    while p < text.len() {
        let Some(match_end) = path_token_at(text, p) else {
            p += 1;
            continue;
        };
        let match_start = p;
        p = match_end.max(p + 1);
        if match_start < cursor {
            continue;
        }
        let mut end = match_end;
        while end < text.len() && !is_space(text[end]) {
            end += 1;
        }
        if end > match_end {
            spans.push((
                offsets.at(match_end),
                offsets.at(end),
                "[redacted]".to_owned(),
            ));
        }
        cursor = end;
    }
    spans
}

/// Union secret contexts before masking; one detector cannot erase another.
///
/// Each forward scanner produces ordered original-coordinate spans. Merging a
/// fixed number of streams is linear, including overlaps between decoded and
/// literal syntax. Ambiguous overlaps own their whole union.
pub fn redact_credentials(text: &str) -> String {
    let text = chars(text);
    if text.len() > REDACTION_LIMIT {
        return "[redacted-oversized]".to_owned();
    }
    string(&redact_credential_chars(&text))
}

/// [`redact_credentials`] over code points.
pub fn redact_credential_chars(text: &[char]) -> Vec<char> {
    if text.len() > REDACTION_LIMIT {
        return chars("[redacted-oversized]");
    }
    let (view, offsets) = url_view(text);
    let decoded = view != text;
    let identity = Offsets::identity();
    let mut streams = vec![url_spans(text), bare_userinfo_spans(text)];
    let scans: [Scan; 5] = [
        xml_spans,
        named_pair_spans,
        cli_spans,
        path_token_spans,
        keyed_spans,
    ];
    for scan in scans {
        streams.push(scan(text, &identity));
        if decoded {
            streams.push(scan(&view, &offsets));
        }
    }
    let mut out: Vec<char> = Vec::with_capacity(text.len());
    let mut cursor = 0;
    let mut pending: Option<Span> = None;
    for (start, end, replacement) in merge(streams) {
        if let Some(current) = pending.as_mut() {
            if start < current.1 {
                if end > current.1 {
                    *current = (current.0, end, "[redacted]".to_owned());
                }
                continue;
            }
        }
        if let Some((p_start, p_end, p_replacement)) = pending.take() {
            out.extend_from_slice(sl(text, cursor, p_start));
            out.extend(p_replacement.chars());
            cursor = p_end;
        }
        pending = Some((start, end, replacement));
    }
    if let Some((p_start, p_end, p_replacement)) = pending {
        out.extend_from_slice(sl(text, cursor, p_start));
        out.extend(p_replacement.chars());
        cursor = p_end;
    }
    out.extend_from_slice(sl(text, cursor, text.len()));
    out
}

/// `heapq.merge(*streams, key=lambda span: (span[0], -span[1]))`: the least
/// head by key, ties to the earlier stream.
fn merge(streams: Vec<Vec<Span>>) -> Vec<Span> {
    let total = streams.iter().map(Vec::len).sum();
    let mut heads = vec![0usize; streams.len()];
    let mut merged = Vec::with_capacity(total);
    for _ in 0..total {
        let mut best: Option<usize> = None;
        for (index, stream) in streams.iter().enumerate() {
            let Some(span) = stream.get(heads[index]) else {
                continue;
            };
            let better = best.is_none_or(|b| {
                let other = &streams[b][heads[b]];
                (span.0, std::cmp::Reverse(span.1)) < (other.0, std::cmp::Reverse(other.1))
            });
            if better {
                best = Some(index);
            }
        }
        let Some(index) = best else {
            break;
        };
        merged.push(streams[index][heads[index]].clone());
        heads[index] += 1;
    }
    merged
}
