//! Python `re` semantics for the patterns this crate runs: compiled through
//! `kassiber-envelope`'s translation (Python's `\b`, `\s`, `\d`, `\w`, `$`
//! and ASCII IGNORECASE equivalences), plus the text helpers they need.

use std::collections::HashMap;
use std::sync::OnceLock;

use fancy_regex::Regex;
use kassiber_envelope::redaction::python_pattern;
use kassiber_pyjson::PyStr;

/// Compiles a Python pattern; panics on a pattern this crate wrote wrongly.
pub(crate) fn compile(pattern: &str) -> Regex {
    let translated = python_pattern(pattern);
    Regex::new(&translated)
        .unwrap_or_else(|error| panic!("pattern {pattern:?} does not compile: {error}"))
}

/// A string that may hold surrogates, as UTF-8 with each surrogate replaced
/// by U+FFFD. Used only where no pattern or keyword can tell them apart.
pub(crate) fn lossy(text: &PyStr) -> String {
    match text.as_str() {
        Some(text) => text.to_owned(),
        None => text
            .code_points()
            .map(|point| char::from_u32(point).unwrap_or('\u{fffd}'))
            .collect(),
    }
}

/// Runs `rule` over a string that may hold surrogates: each surrogate stands
/// in as a distinct private-use character the text does not hold (which no
/// Python class treats as word, space or digit, as for a surrogate), then
/// comes back wherever the rule kept it.
pub(crate) fn with_surrogates(text: &PyStr, rule: impl FnOnce(&str) -> String) -> PyStr {
    if let Some(plain) = text.as_str() {
        return PyStr::from(rule(plain));
    }
    let present: std::collections::HashSet<u32> = text.code_points().collect();
    let mut stand_ins = (0xF_0000u32..=0x10_FFFD).filter(|point| !present.contains(point));
    let mut originals = HashMap::new();
    let mut masked = String::new();
    for point in text.code_points() {
        match char::from_u32(point) {
            Some(c) => masked.push(c),
            None => {
                let stand_in = stand_ins
                    .next()
                    .and_then(char::from_u32)
                    .expect("a private-use stand-in");
                originals.insert(stand_in, point);
                masked.push(stand_in);
            }
        }
    }
    let result = rule(&masked);
    let points = result
        .chars()
        .map(|c| originals.get(&c).copied().unwrap_or(u32::from(c)));
    PyStr::from_code_points(points).expect("code points came from a string")
}

/// `text[-count:]` by code point.
pub(crate) fn tail_chars(text: &str, count: usize) -> &str {
    let total = text.chars().count();
    if total <= count {
        return text;
    }
    let start = text
        .char_indices()
        .nth(total - count)
        .map_or(text.len(), |(index, _)| index);
    &text[start..]
}

/// `text[:count]` by code point.
pub(crate) fn head_chars(text: &str, count: usize) -> &str {
    let end = text
        .char_indices()
        .nth(count)
        .map_or(text.len(), |(index, _)| index);
    &text[..end]
}

/// `len(text)` for a Python string.
pub(crate) fn char_len(text: &PyStr) -> usize {
    text.code_points().count()
}

/// The JSON bodies of ```` ```json ```` fences in an assistant message tail
/// (`_continues_review_checkpoint`'s `re.finditer`, `re.DOTALL`).
pub(crate) fn review_checkpoint_blocks(text: &str) -> Vec<String> {
    static FENCE: OnceLock<Regex> = OnceLock::new();
    let fence =
        FENCE.get_or_init(|| compile(r"(?s)(?:^|\n)```json[ \t]*\r?\n(.*?)\r?\n```(?:\s|$)"));
    fence
        .captures_iter(text)
        .filter_map(Result::ok)
        .filter_map(|captures| captures.get(1).map(|group| group.as_str().to_owned()))
        .collect()
}

/// `re.search(pattern, value) is not None`, with a translated pattern; a
/// pattern the engine cannot finish counts as no match.
pub(crate) fn search(regex: &Regex, text: &PyStr) -> bool {
    match text.as_str() {
        Some(text) => regex.is_match(text).unwrap_or(false),
        None => {
            let mut found = false;
            let _ = with_surrogates(text, |plain| {
                found = regex.is_match(plain).unwrap_or(false);
                String::new()
            });
            found
        }
    }
}
