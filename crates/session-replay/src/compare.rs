//! Canonical text for comparing a replay with its recording.
//!
//! A run's generated values differ between runs: uuid4s, `uuid4().hex`
//! values, and wall-clock readings inside the run's own window (or a whole
//! number of hours, up to three days, or a policy period of 7, 14, 30, 60,
//! 90, 180 or 365 days, away from it, as for expiries). Each side's
//! transcript is rewritten so every such value becomes a placeholder numbered
//! by first appearance; equal transcripts then mean the two runs generated
//! values in the same pattern, the same bijection everywhere. Values that came
//! in through the inputs (argv, stdin, the starting state) are pinned and
//! never replaced. A uuid5 derived from a generated value differs between
//! runs too, so a uuid5 is numbered like a uuid4 unless an input names it or
//! both runs printed it (`pin_shared_uuid5`). Random request ids
//! (`chat-<32 hex>`), url-safe tokens and upload ids, and the random part of
//! temporary names inside the session's roots are generated as well. Everything else (hashes, fixture
//! dates) compares exactly.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;

pub const SLACK_US: i64 = 5_000_000;
const HOUR_US: i64 = 3_600_000_000;
const MAX_OFFSET_HOURS: i64 = 72;
/// Expiry periods the product uses (a dismissal lasts 90 days).
const OFFSET_DAYS: [i64; 7] = [7, 14, 30, 60, 90, 180, 365];

fn uuid4() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
            .unwrap()
    })
}

fn uuid4_hex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[0-9a-f]{12}4[0-9a-f]{3}[89ab][0-9a-f]{15}").unwrap())
}

fn uuid5() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
            .unwrap()
    })
}

/// The CLI chat's and the MCP server's random request ids: `chat-`, `chat-consent-`, `mcp-`,
/// `chat-provider-list-` and the like, then 32 random hex digits.
fn chat_request() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(?:chat|mcp)-(?:[a-z]+-)*([0-9a-f]{32})\b").unwrap())
}

/// Random url-safe tokens: 256-bit ones under a `token` or `*_token` key (a
/// document import's `document_token`), 128-bit ones under `upload_id`, and
/// analysis `snapshot_id`s, which hash the index's random instance id.
fn url_token() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?:\b|_)token\\?"\s*:\s*\\?"([A-Za-z0-9_-]{43})\\?"|\bupload_id\\?"\s*:\s*\\?"([A-Za-z0-9_-]{22})\\?"|\bsnapshot_id\\?"\s*:\s*\\?"([0-9a-f]{64}|[0-9a-f]{40})\\?"|\bsnapshot_id=text "([0-9a-f]{64}|[0-9a-f]{40})""#,
        )
        .unwrap()
    })
}

/// A path inside the session's state roots (`/mnt/...` in a namespaced
/// recording) and, in it, a temporary directory or file name: a prefix and
/// six to eight random characters (`kassiber-chat-uploads-XjMTS7Hk`,
/// `tmpzxLZfBnQ`, `.tmpA1b2C3`).
fn root_path() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"/mnt/(?:tmp|bt)/[^"'\s\\]+"#).unwrap())
}

fn temporary_name() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?:[A-Za-z][A-Za-z0-9.]*(?:-[A-Za-z0-9.]+)*[-_]|\.?tmp)([A-Za-z0-9_-]{6,8})$")
            .unwrap()
    })
}

fn datetime() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?(Z|[+-]\d{2}:?\d{2})?",
        )
        .unwrap()
    })
}

fn compact() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d{8})([T_-]?)(\d{6})(Z?)").unwrap())
}

/// Relative ages a display computes from a stored time and now.
fn relative_age() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\bjust now\b|\b\d+ ?(?:s|sec|secs|m|min|mins|h|hr|hrs|d|day|days) ago\b")
            .unwrap()
    })
}

fn epoch() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d{16}|\d{13}|\d{10}(?:\.\d+)?").unwrap())
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The run's wall-clock span in epoch microseconds, and the local offset
/// naive readings may carry.
#[derive(Clone, Debug)]
pub struct Window {
    pub start: i64,
    pub end: i64,
    pub local_offset_us: i64,
    /// Other spans whose readings also count as generated: on the replay
    /// side, the recording's window, because the restored state carries
    /// values the recorded run (or an earlier one) read from its clock; and
    /// on both sides the window of a run that re-recorded the session.
    pub also: Vec<(i64, i64)>,
}

impl Window {
    fn generated(&self, instant: i64) -> bool {
        let spans = std::iter::once((self.start, self.end)).chain(self.also.iter().copied());
        spans.into_iter().any(|(start, end)| {
            let inside = |t: i64| start - SLACK_US <= t && t <= end + SLACK_US;
            if inside(instant) {
                return true;
            }
            let hours = ((instant - start) as f64 / HOUR_US as f64).round() as i64;
            let days = ((instant - start) as f64 / (24 * HOUR_US) as f64).round() as i64;
            (hours != 0 && hours.abs() <= MAX_OFFSET_HOURS && inside(instant - hours * HOUR_US))
                || (OFFSET_DAYS.contains(&days.abs()) && inside(instant - days * 24 * HOUR_US))
        })
    }

    fn generated_naive(&self, instant: i64) -> bool {
        self.generated(instant) || self.generated(instant - self.local_offset_us)
    }
}

fn boundary_ok(text: &str, start: usize, end: usize, digits_only: bool) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    let bad = |c: Option<char>| match c {
        Some(c) if digits_only => c.is_ascii_digit(),
        Some(c) => c.is_ascii_hexdigit(),
        None => false,
    };
    !bad(before) && !bad(after)
}

fn iso_instant(caps: &regex::Captures<'_>) -> Option<(i64, bool)> {
    let number = |i: usize| {
        caps.get(i)
            .map(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(Some(0))
    };
    let (year, month, day) = (number(1)?, number(2)?, number(3)?);
    let (hour, minute, second) = (number(4)?, number(5)?, number(6)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let fraction = caps.get(7).map(|m| m.as_str()).unwrap_or("");
    let micros: i64 = format!("{:0<6}", &fraction[..fraction.len().min(6)])
        .parse()
        .unwrap_or(0);
    let mut instant = ((days_from_civil(year, month, day) * 24 + hour) * 60 + minute) * 60 + second;
    instant = instant * 1_000_000 + micros;
    match caps.get(8).map(|m| m.as_str()) {
        Some("Z") => Some((instant, true)),
        Some(zone) => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let digits: String = zone[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            let hours: i64 = digits.get(..2)?.parse().ok()?;
            let minutes: i64 = digits.get(2..4).unwrap_or("00").parse().ok()?;
            Some((instant - sign * (hours * 60 + minutes) * 60_000_000, true))
        }
        None => Some((instant, false)),
    }
}

/// Names of fields holding amounts, prices and tax figures: a number under
/// one is never a clock reading, however close to the run's time it falls
/// (an msat amount can look like a millisecond epoch).
fn figure_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:amount|msat|sat|sats|price|fee|fees|gain|gains|loss|losses|basis|proceeds|tax|cost|value|quantity|qty|balance|total|fiat|eur|usd|income|rate|fx)",
        )
        .unwrap()
    })
}

/// Whether the token at `start` is the value of a figure field: JSON
/// (`"amount_msat": `, also escaped) or a dumped row (`amount_msat=int `).
pub fn figure_key_before(text: &str, start: usize) -> bool {
    static KEY: OnceLock<Regex> = OnceLock::new();
    let key = KEY.get_or_init(|| {
        Regex::new(r#"([A-Za-z_][A-Za-z0-9_]*)(?:\\?"\s*:\s*\\?"?|=(?:int|real|text) "?)$"#)
            .unwrap()
    });
    let from = text[..start]
        .char_indices()
        .rev()
        .nth(79)
        .map_or(0, |(i, _)| i);
    key.captures(&text[from..start])
        .is_some_and(|c| figure_key().is_match(&c[1]))
}

/// The generated tokens in `text`, by byte span, in order.
pub fn generated_spans(
    text: &str,
    window: &Window,
    pinned: &HashSet<String>,
) -> Vec<(usize, usize, bool)> {
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    // Pinning keeps input identities literal. Clock readings are never
    // pinned: a value an earlier process wrote a moment ago cannot be told
    // from one this run read in the same second.
    let take = |start: usize, end: usize, spans: &mut Vec<(usize, usize, bool)>| {
        if spans.iter().any(|(s, e, _)| start < *e && *s < end) {
            return;
        }
        spans.push((start, end, false));
    };
    let take_identity = |start: usize, end: usize, spans: &mut Vec<(usize, usize, bool)>| {
        if pinned.contains(&text[start..end]) {
            return;
        }
        if spans.iter().any(|(s, e, _)| start < *e && *s < end) {
            return;
        }
        spans.push((start, end, true));
    };
    for m in uuid4().find_iter(text) {
        if boundary_ok(text, m.start(), m.end(), false) {
            take_identity(m.start(), m.end(), &mut spans);
        }
    }
    for m in uuid5().find_iter(text) {
        if boundary_ok(text, m.start(), m.end(), false) {
            take_identity(m.start(), m.end(), &mut spans);
        }
    }
    for caps in chat_request()
        .captures_iter(text)
        .chain(url_token().captures_iter(text))
    {
        let token = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .or_else(|| caps.get(4))
            .unwrap();
        take_identity(token.start(), token.end(), &mut spans);
    }
    for path in root_path().find_iter(text) {
        let mut at = path.start();
        for component in path.as_str().split('/') {
            if let Some(caps) = temporary_name().captures(component) {
                let suffix = caps.get(1).unwrap();
                // Random: it mixes letters and digits or cases.
                let tail = suffix.as_str();
                let kinds = [
                    tail.bytes().any(|b| b.is_ascii_digit()),
                    tail.bytes().any(|b| b.is_ascii_uppercase()),
                    tail.bytes().any(|b| b.is_ascii_lowercase()),
                ];
                if kinds.iter().filter(|k| **k).count() >= 2 {
                    take_identity(at + suffix.start(), at + suffix.end(), &mut spans);
                }
            }
            at += component.len() + 1;
        }
    }
    for m in uuid4_hex().find_iter(text) {
        // Only a whole value: quoted, or delimited by non-word characters.
        let before = text[..m.start()].chars().next_back();
        let after = text[m.end()..].chars().next();
        let delimited =
            |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
        if delimited(before) && delimited(after) {
            take_identity(m.start(), m.end(), &mut spans);
        }
    }
    for caps in datetime().captures_iter(text) {
        let whole = caps.get(0).unwrap();
        if !boundary_ok(text, whole.start(), whole.end(), true) {
            continue;
        }
        if let Some((instant, aware)) = iso_instant(&caps) {
            let generated = if aware {
                window.generated(instant)
            } else {
                window.generated_naive(instant)
            };
            if generated {
                take(whole.start(), whole.end(), &mut spans);
            }
        }
    }
    for caps in compact().captures_iter(text) {
        let whole = caps.get(0).unwrap();
        if !boundary_ok(text, whole.start(), whole.end(), true) {
            continue;
        }
        let day = &caps[1];
        let clock = &caps[3];
        let parse = |s: &str| s.parse::<i64>().ok();
        let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s)) = (
            parse(&day[..4]),
            parse(&day[4..6]),
            parse(&day[6..8]),
            parse(&clock[..2]),
            parse(&clock[2..4]),
            parse(&clock[4..6]),
        ) else {
            continue;
        };
        if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
            continue;
        }
        let instant = (((days_from_civil(y, mo, d) * 24 + h) * 60 + mi) * 60 + s) * 1_000_000;
        if window.generated_naive(instant) {
            take(whole.start(), whole.end(), &mut spans);
        }
    }
    for m in epoch().find_iter(text) {
        if !boundary_ok(text, m.start(), m.end(), true)
            || text[m.end()..].starts_with('.')
            || figure_key_before(text, m.start())
        {
            continue;
        }
        let token = m.as_str();
        let instant = if let Some((secs, fraction)) = token.split_once('.') {
            let micros: i64 = format!("{:0<6}", &fraction[..fraction.len().min(6)])
                .parse()
                .unwrap_or(0);
            secs.parse::<i64>().unwrap_or(0) * 1_000_000 + micros
        } else {
            let value: i64 = token.parse().unwrap_or(0);
            match token.len() {
                10 => value * 1_000_000,
                13 => value * 1_000,
                _ => value,
            }
        };
        if window.generated(instant) {
            take(m.start(), m.end(), &mut spans);
        }
    }
    for m in relative_age().find_iter(text) {
        take(m.start(), m.end(), &mut spans);
    }
    spans.sort();
    spans
}

/// Rewrites a run's artifacts with numbered placeholders.
pub struct Canon<'a> {
    window: Window,
    pinned: &'a HashSet<String>,
    seen: HashMap<String, usize>,
}

impl<'a> Canon<'a> {
    pub fn new(window: Window, pinned: &'a HashSet<String>) -> Self {
        Self {
            window,
            pinned,
            seen: HashMap::new(),
        }
    }

    /// `text` with every generated value replaced; `numbered` gives each
    /// distinct value its first-appearance index, otherwise all collapse.
    /// Durations compare within tolerance (any value), and JSON lists whose
    /// elements a generated id orders are compared in a shape order.
    pub fn rewrite(&mut self, text: &str, numbered: bool) -> String {
        let text = mask_durations(text);
        let text = if numbered {
            self.sort_generated_id_lists(&text)
        } else {
            text
        };
        let text = text.as_str();
        let spans = generated_spans(text, &self.window, self.pinned);
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for (start, end, identity) in spans {
            out.push_str(&text[at..start]);
            if !identity {
                // Clock readings: which ones fall in the same second differs
                // between runs, so they collapse instead of mapping.
                out.push_str("<time>");
            } else if numbered {
                let next = self.seen.len();
                let index = *self.seen.entry(text[start..end].to_owned()).or_insert(next);
                out.push_str(&format!("<gen:{index}>"));
            } else {
                out.push_str("<gen>");
            }
            at = end;
        }
        out.push_str(&text[at..]);
        out
    }

    /// Table rows in an order both runs share: by their shape with generated
    /// values collapsed, original order breaking ties.
    pub fn rows(&mut self, rows: &[String]) -> Vec<String> {
        let mut keyed: Vec<(String, usize)> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| (self.rewrite(row, false), i))
            .collect();
        keyed.sort();
        keyed
            .into_iter()
            .map(|(_, i)| self.rewrite(&rows[i], true))
            .collect()
    }
}

impl Canon<'_> {
    /// JSON arrays of objects whose `id` is a generated identity: their
    /// order follows random ids, so they compare sorted by shape.
    fn sort_generated_id_lists(&mut self, text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        let mut i = 0;
        let mut in_string = false;
        while i < bytes.len() {
            let c = bytes[i];
            if in_string {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                in_string = c != b'"';
            } else if c == b'"' {
                in_string = true;
            } else if c == b'[' {
                if let Some(end) = matching(bytes, i) {
                    out.push_str(&text[at..i + 1]);
                    let inner = self.sort_generated_id_lists(&text[i + 1..end]);
                    out.push_str(&self.sorted_elements(&inner));
                    out.push(']');
                    at = end + 1;
                    i = end + 1;
                    continue;
                }
            }
            i += 1;
        }
        out.push_str(&text[at..]);
        out
    }

    fn sorted_elements(&mut self, inner: &str) -> String {
        let pieces = split_top_level(inner);
        let generated_id = |element: &str| {
            let element = element.trim();
            element.starts_with('{')
                && json_id(element).is_some_and(|id| {
                    !self.pinned.contains(id) && (uuid4().is_match(id) || uuid4_hex().is_match(id))
                })
        };
        if pieces.len() < 2 || !pieces.iter().all(|p| generated_id(p)) {
            return inner.to_owned();
        }
        let mut elements: Vec<(String, String)> = pieces
            .iter()
            .map(|p| {
                let trimmed = p.trim().to_owned();
                let shape = collapse(&trimmed, &self.window, self.pinned);
                (shape, trimmed)
            })
            .collect();
        elements.sort();
        let mut out = String::new();
        for (index, piece) in pieces.iter().enumerate() {
            let lead = &piece[..piece.len() - piece.trim_start().len()];
            let trail = &piece[piece.trim_end().len()..];
            if index > 0 {
                out.push(',');
            }
            out.push_str(lead);
            out.push_str(&elements[index].1);
            out.push_str(trail);
        }
        out
    }
}

/// `text` with every generated value collapsed, for ordering.
fn collapse(text: &str, window: &Window, pinned: &HashSet<String>) -> String {
    let mut out = String::new();
    let mut at = 0;
    for (start, end, _) in generated_spans(text, window, pinned) {
        out.push_str(&text[at..start]);
        out.push_str("<gen>");
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// The index of the `]` or `}` closing the bracket at `open`, string aware.
fn matching(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut i = open;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                i += 2;
                continue;
            }
            in_string = c != b'"';
        } else {
            match c {
                b'"' => in_string = true,
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Split an array's inside at its top-level commas, string aware.
fn split_top_level(inner: &str) -> Vec<&str> {
    let bytes = inner.as_bytes();
    let (mut depth, mut in_string, mut start, mut i) = (0i32, false, 0, 0);
    let mut pieces = Vec::new();
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                i += 2;
                continue;
            }
            in_string = c != b'"';
        } else {
            match c {
                b'"' => in_string = true,
                b'[' | b'{' => depth += 1,
                b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    pieces.push(&inner[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    if !inner[start..].trim().is_empty() {
        pieces.push(&inner[start..]);
    }
    pieces
}

/// The object's own `"id"` string value (the first `"id"` key at depth one).
fn json_id(object: &str) -> Option<&str> {
    let bytes = object.as_bytes();
    let (mut depth, mut in_string, mut i) = (0i32, false, 0);
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                i += 2;
                continue;
            }
            in_string = c != b'"';
        } else {
            match c {
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth -= 1,
                b'"' if depth == 1 && object[i..].starts_with("\"id\"") => {
                    let rest = object[i + 4..].trim_start().strip_prefix(':')?.trim_start();
                    let value = rest.strip_prefix('"')?;
                    return value.split('"').next();
                }
                b'"' => in_string = true,
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Durations (`*elapsed*` / `*duration*` keys or columns, and measured times:
/// fractional `seconds`, `user`, `sys`, `cpu`, `wall`, also with a `*_` prefix)
/// vary run to run; they compare as present, never by value.
pub fn mask_durations(text: &str) -> String {
    static JSON: OnceLock<Regex> = OnceLock::new();
    static ROW: OnceLock<Regex> = OnceLock::new();
    let json = JSON.get_or_init(|| {
        Regex::new(r#"(\\?"[A-Za-z_]*(?:elapsed|duration)[A-Za-z_]*\\?"\s*:\s*)-?\d+(?:\.\d+)?"#)
            .unwrap()
    });
    let row = ROW.get_or_init(|| {
        Regex::new(r"(\b[A-Za-z_]*(?:elapsed|duration)[A-Za-z_]*=(?:int|real) )-?[\d.]+").unwrap()
    });
    static TYPED: OnceLock<Regex> = OnceLock::new();
    // Log records carry a duration as `{"type":"duration_ms","value":39}`.
    let typed = TYPED.get_or_init(|| {
        Regex::new(
            r#"(\\?"type\\?"\s*:\s*\\?"duration_ms\\?"\s*,\s*\\?"value\\?"\s*:\s*)-?\d+(?:\.\d+)?"#,
        )
        .unwrap()
    });
    static MEASURED: OnceLock<Regex> = OnceLock::new();
    // Measured times are fractional; whole numbers under these keys stay.
    let measured = MEASURED.get_or_init(|| {
        Regex::new(
            r#"(\\?"(?:[A-Za-z_]*_)?(?:seconds|user|sys|cpu|wall)\\?"\s*:\s*)-?\d+\.\d+(?:[eE]-?\d+)?"#,
        )
        .unwrap()
    });
    let text = typed.replace_all(text, "${1}<duration>");
    let text = measured.replace_all(&text, "${1}<duration>");
    let text = json.replace_all(&text, "${1}<duration>");
    row.replace_all(&text, "${1}<duration>").into_owned()
}

/// Daemon output in an order concurrency cannot change: each request's
/// records keep their order, requests sort by id, unsolicited records first.
pub fn group_by_request(text: &str) -> String {
    static ID: OnceLock<Regex> = OnceLock::new();
    let id = ID.get_or_init(|| Regex::new(r#""request_id"\s*:\s*"([^"]*)"\s*\}\s*$"#).unwrap());
    let mut groups: std::collections::BTreeMap<String, Vec<&str>> =
        std::collections::BTreeMap::new();
    for line in text.lines() {
        let key = id
            .captures(line)
            .map(|c| format!("1{}", &c[1]))
            .unwrap_or_else(|| "0".to_owned());
        groups.entry(key).or_default().push(line);
    }
    let mut out = String::new();
    for lines in groups.values() {
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Every token in `text` that could be mistaken for a generated value.
/// The identity tokens (uuid4, `uuid4().hex`) in `text`, in order.
pub fn identities(text: &str) -> Vec<String> {
    let wide = Window {
        start: 0,
        end: 0,
        local_offset_us: 0,
        also: Vec::new(),
    };
    let none = HashSet::new();
    generated_spans(text, &wide, &none)
        .into_iter()
        .filter(|(_, _, identity)| *identity)
        .map(|(start, end, _)| text[start..end].to_owned())
        .collect()
}

/// Pin the uuid5s both runs printed: those did not derive from a generated
/// value, so they compare literally.
pub fn pin_shared_uuid5(recorded: &str, replayed: &str, pinned: &mut HashSet<String>) {
    let found = |text: &str| -> HashSet<String> {
        uuid5()
            .find_iter(text)
            .map(|m| m.as_str().to_owned())
            .collect()
    };
    let replayed = found(replayed);
    pinned.extend(found(recorded).into_iter().filter(|u| replayed.contains(u)));
}

pub fn pin_inputs(text: &str, pinned: &mut HashSet<String>) {
    let wide = Window {
        start: i64::MIN / 4,
        end: i64::MAX / 4,
        local_offset_us: 0,
        also: Vec::new(),
    };
    let none = HashSet::new();
    for (start, end, _) in generated_spans(text, &wide, &none) {
        pinned.insert(text[start..end].to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Window {
        // 2026-11-10T12:00:00Z for ten seconds.
        let start = days_from_civil(2026, 11, 10) * 86_400_000_000 + 12 * HOUR_US;
        Window {
            start,
            end: start + 10_000_000,
            local_offset_us: HOUR_US,
            also: Vec::new(),
        }
    }

    #[test]
    fn generated_values_number_by_first_appearance() {
        let pinned = HashSet::new();
        let mut a = Canon::new(window(), &pinned);
        let mut b = Canon::new(window(), &pinned);
        let left = a.rewrite(
            "id=0b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11 at 2026-11-10T12:00:03Z again 0b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11",
            true,
        );
        let right = b.rewrite(
            "id=11111111-2222-4333-8444-555555555555 at 2026-11-10T12:00:07.5Z again 11111111-2222-4333-8444-555555555555",
            true,
        );
        assert_eq!(left, right);
        assert_eq!(left, "id=<gen:0> at <time> again <gen:0>");
    }

    #[test]
    fn a_broken_bijection_differs() {
        let pinned = HashSet::new();
        let mut a = Canon::new(window(), &pinned);
        let mut b = Canon::new(window(), &pinned);
        let left = a.rewrite(
            "0b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11 0b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11",
            true,
        );
        let right = b.rewrite(
            "11111111-2222-4333-8444-555555555555 22222222-2222-4333-8444-555555555555",
            true,
        );
        assert_ne!(left, right);
    }

    #[test]
    fn fixture_dates_hashes_and_pinned_values_stay() {
        let mut pinned = HashSet::new();
        pin_inputs("given 0b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11", &mut pinned);
        let mut canon = Canon::new(window(), &pinned);
        let text = "0b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11 2024-03-01T10:00:00Z 5a8f0d0b5e1c4d2aa3b4c5d6e7f8091a2b3c4d5e";
        assert_eq!(canon.rewrite(text, true), text);
    }

    #[test]
    fn expiries_and_epochs_in_the_window_are_generated() {
        let pinned = HashSet::new();
        let mut canon = Canon::new(window(), &pinned);
        let w = window();
        let secs = w.start / 1_000_000 + 2;
        let text = format!("expires 2026-11-11T12:00:01Z at {secs} or {}", secs * 1000);
        assert_eq!(
            canon.rewrite(&text, true),
            "expires <time> at <time> or <time>"
        );
        assert_eq!(
            canon.rewrite("2026-11-20T12:00:01Z", true),
            "2026-11-20T12:00:01Z"
        );
        // A 90-day dismissal expiry.
        assert_eq!(canon.rewrite("2027-02-08T12:00:03Z", true), "<time>");
    }

    #[test]
    fn durations_and_random_id_lists_normalise() {
        let pinned = HashSet::new();
        let mut a = Canon::new(window(), &pinned);
        let mut b = Canon::new(window(), &pinned);
        let left = a.rewrite(
            r#"{"elapsed_ms":1931,"items":[{"id":"0b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11","name":"b"},{"id":"1b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11","name":"a"}]}"#,
            true,
        );
        let right = b.rewrite(
            r#"{"elapsed_ms":7,"items":[{"id":"21111111-2222-4333-8444-555555555555","name":"a"},{"id":"31111111-2222-4333-8444-555555555555","name":"b"}]}"#,
            true,
        );
        assert_eq!(left, right);
        // A list ordered by meaningful values keeps its order.
        let mut c = Canon::new(window(), &pinned);
        let kept = c.rewrite(r#"[{"id":"fixed-1","n":2},{"id":"fixed-0","n":1}]"#, true);
        assert_eq!(kept, r#"[{"id":"fixed-1","n":2},{"id":"fixed-0","n":1}]"#);
        assert_eq!(
            mask_durations("duration_ms=int 99 | x=int 3"),
            "duration_ms=int <duration> | x=int 3"
        );
        assert_eq!(
            mask_durations(r#"{"duration_ms":{"type":"duration_ms","value":39}}"#),
            r#"{"duration_ms":{"type":"duration_ms","value":<duration>}}"#
        );
        assert_eq!(
            mask_durations(
                r#"{"seconds":0.87,"adapter_sys":4.9e-5,"user":0.01,"transactions":3,"sys":2}"#
            ),
            r#"{"seconds":<duration>,"adapter_sys":<duration>,"user":<duration>,"transactions":3,"sys":2}"#
        );
        // Inside JSON stored as text, with escaped quotes.
        assert_eq!(
            mask_durations(r#"{\"elapsed_ms\":1257,\"n\":1}"#),
            r#"{\"elapsed_ms\":<duration>,\"n\":1}"#
        );
    }

    #[test]
    fn derived_uuid5s_and_chat_requests_number_unless_shared() {
        let mut pinned = HashSet::new();
        let shared = "64f35537-8fa9-551c-87da-04a22bef228c";
        pin_shared_uuid5(
            &format!("{shared} 11111111-2222-5333-8444-555555555555"),
            &format!("{shared} 21111111-2222-5333-8444-555555555555"),
            &mut pinned,
        );
        let mut a = Canon::new(window(), &pinned);
        let mut b = Canon::new(window(), &pinned);
        let left = a.rewrite(
            &format!("{shared} 11111111-2222-5333-8444-555555555555 chat-ff226452ce6acd3a9a0c4192351dabb5"),
            true,
        );
        let right = b.rewrite(
            &format!("{shared} 21111111-2222-5333-8444-555555555555 chat-258731caf09b53238d7e7a13292c9c0f"),
            true,
        );
        assert_eq!(left, right);
        assert!(left.starts_with(shared), "{left}");
        let left = a.rewrite(
            r#"{"document_token":"x1J2WRUXPP0soDDwndV6djrhK_OrFAnCFU8uNiuNixo"} chat-provider-list-4d741916abc1550a2e3ff9fa483dbab3"#,
            true,
        );
        let right = b.rewrite(
            r#"{"document_token":"zIJsCmBiD4H0mSFKOwqB0mKzzmOxs76DYdSeSlVhSPk"} chat-provider-list-d949957e66972ad045439423d93fb062"#,
            true,
        );
        assert_eq!(left, right);
        let left = a.rewrite(
            r#"{"token": "zQ3xoXZBaXYJqgEdfF73rzQXEUXr5PcYRTYUpSRQA5g", "upload_id": "db1PhNKH3B_1jozyVtn14A"}"#,
            true,
        );
        let right = b.rewrite(
            r#"{"token": "x1J2WRUXPP0soDDwndV6djrhK_OrFAnCFU8uNiuNixo", "upload_id": "RO0M4G8Il68isMLStnipVA"}"#,
            true,
        );
        assert_eq!(left, right);
    }

    #[test]
    fn temporary_names_in_root_paths_number() {
        let mut pinned = HashSet::new();
        pin_inputs("/mnt/tmp/kassiber-r5-desktop-od6dr16w/data", &mut pinned);
        let mut a = Canon::new(window(), &pinned);
        let mut b = Canon::new(window(), &pinned);
        let left = a.rewrite(
            r#"{"path":"/mnt/tmp/kassiber-chat-uploads-XjMTS7Hk/tmpzxLZfBnQ/paste.csv","root":"/mnt/tmp/kassiber-r5-desktop-od6dr16w/data"}"#,
            true,
        );
        let right = b.rewrite(
            r#"{"path":"/mnt/tmp/kassiber-chat-uploads-ucqErM-f/tmp0HCb0AO9/paste.csv","root":"/mnt/tmp/kassiber-r5-desktop-od6dr16w/data"}"#,
            true,
        );
        assert_eq!(left, right);
        assert!(left.contains("kassiber-r5-desktop-od6dr16w"), "{left}");
        // Outside the roots, and names that are words, stay.
        let kept = a.rewrite("/home/x/tmpzxLZfBnQ /mnt/tmp/kassiber-data/statement", true);
        assert_eq!(kept, "/home/x/tmpzxLZfBnQ /mnt/tmp/kassiber-data/statement");
    }

    #[test]
    fn daemon_output_groups_by_request() {
        let left = "{\"kind\":\"a\",\"request_id\":\"r2\"}\n{\"kind\":\"b\",\"request_id\":\"r1\"}\n{\"kind\":\"c\",\"request_id\":\"r2\"}\n";
        let right = "{\"kind\":\"b\",\"request_id\":\"r1\"}\n{\"kind\":\"a\",\"request_id\":\"r2\"}\n{\"kind\":\"c\",\"request_id\":\"r2\"}\n";
        assert_eq!(group_by_request(left), group_by_request(right));
        let swapped =
            "{\"kind\":\"c\",\"request_id\":\"r2\"}\n{\"kind\":\"a\",\"request_id\":\"r2\"}\n";
        assert_ne!(group_by_request(swapped), group_by_request(left));
    }

    #[test]
    fn rows_reorder_by_shape() {
        let pinned = HashSet::new();
        let mut a = Canon::new(window(), &pinned);
        let mut b = Canon::new(window(), &pinned);
        let left = a.rows(&[
            "id=0b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11 | n=2".into(),
            "id=1b9e4c58-7f43-4d0e-9a3a-5c2d1e8f0a11 | n=1".into(),
        ]);
        let right = b.rows(&[
            "id=21111111-2222-4333-8444-555555555555 | n=1".into(),
            "id=31111111-2222-4333-8444-555555555555 | n=2".into(),
        ]);
        assert_eq!(left, right);
    }
}
