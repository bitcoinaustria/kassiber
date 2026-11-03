//! CPython 3.11 `zoneinfo.ZoneInfo`: key validation, TZPATH lookup, TZif
//! loading (`zoneinfo._common.load_data`) and the module's offset selection
//! for local wall times (fold 0) and UTC instants. Everything is in whole
//! seconds since 1970-01-01, as zoneinfo computes it.
use std::path::{Path, PathBuf};

const DAY: i64 = 86_400;

/// Days since 1970-01-01 of a proleptic Gregorian date.
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
/// The proleptic Gregorian date of days since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}
fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
fn month_days(year: i64, month: i64) -> i64 {
    match month {
        2 if leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
fn year_of(seconds: i64) -> i64 {
    civil_from_days(seconds.div_euclid(DAY)).0
}

#[derive(Clone, Debug, PartialEq)]
enum Offset {
    /// `Jn` (julian) or `n`.
    Day { d: i64, julian: bool, seconds: i64 },
    /// `Mm.w.d`.
    Calendar {
        m: i64,
        w: i64,
        d: i64,
        seconds: i64,
    },
}
impl Offset {
    fn year_to_epoch(&self, year: i64) -> i64 {
        // `_post_epoch_days_before_year`: the day before January 1st.
        let before_year = days_from_civil(year, 1, 1) - 1;
        match *self {
            Offset::Day { d, julian, seconds } => {
                let d = if julian && d >= 59 && leap(year) {
                    d + 1
                } else {
                    d
                };
                (before_year + d) * DAY + seconds
            }
            Offset::Calendar { m, w, d, seconds } => {
                // calendar.monthrange: Monday is 0.
                let first = (days_from_civil(year, m, 1) + 3).rem_euclid(7);
                let mut month_day = (d - (first + 1)).rem_euclid(7) + 1 + (w - 1) * 7;
                if month_day > month_days(year, m) {
                    month_day -= 7;
                }
                days_from_civil(year, m, month_day) * DAY + seconds
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    std: i64,
    dst: i64,
    start: Offset,
    end: Offset,
}
impl Rule {
    fn diff(&self) -> i64 {
        self.dst - self.std
    }
    /// `_TZStr._get_trans_info(ts, year, fold=0)`.
    fn local(&self, ts: i64, year: i64) -> i64 {
        self.local_fold(ts, year, false)
    }
    /// `_TZStr._get_trans_info(ts, year, fold)`.
    fn local_fold(&self, ts: i64, year: i64, fold: bool) -> i64 {
        let (mut start, mut end) = (self.start.year_to_epoch(year), self.end.year_to_epoch(year));
        if fold == (self.diff() >= 0) {
            end -= self.diff();
        } else {
            start += self.diff();
        }
        let dst = if start < end {
            start <= ts && ts < end
        } else {
            !(end <= ts && ts < start)
        };
        if dst {
            self.dst
        } else {
            self.std
        }
    }
    /// The fold `_TZStr._get_trans_info_fromutc(ts, year)` returns.
    fn utc_fold(&self, ts: i64, year: i64) -> bool {
        let start = self.start.year_to_epoch(year) - self.std;
        let end = self.end.year_to_epoch(year) - self.dst;
        let (from, to) = if self.diff() > 0 {
            (end, end + self.diff())
        } else {
            (start, start - self.diff())
        };
        from <= ts && ts < to
    }
    /// `_TZStr._get_trans_info_fromutc(ts, year)`.
    fn utc(&self, ts: i64, year: i64) -> i64 {
        let start = self.start.year_to_epoch(year) - self.std;
        let end = self.end.year_to_epoch(year) - self.dst;
        let dst = if start < end {
            start <= ts && ts < end
        } else {
            !(end <= ts && ts < start)
        };
        if dst {
            self.dst
        } else {
            self.std
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum After {
    Fixed(i64),
    Rule(Rule),
}

/// A loaded zone, keeping only the UTC offsets zoneinfo selects with.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub key: String,
    trans_utc: Vec<i64>,
    trans_local: Vec<i64>,
    /// Wall-clock transitions for fold 1, at the smaller adjacent offset.
    trans_local_fold: Vec<i64>,
    offsets: Vec<i64>,
    before: Option<i64>,
    after: After,
}

impl Zone {
    /// `utcoffset(datetime(..., tzinfo=zone))` for a fold-0 wall time.
    pub fn local_offset(&self, local: i64) -> Option<i64> {
        let lt = &self.trans_local;
        if !lt.is_empty() && local < lt[0] {
            return self.before;
        }
        if lt.is_empty() || local > *lt.last().unwrap() {
            return Some(match &self.after {
                After::Fixed(offset) => *offset,
                After::Rule(rule) => rule.local(local, year_of(local)),
            });
        }
        let idx = lt.partition_point(|t| *t <= local) - 1;
        Some(self.offsets[idx])
    }
    /// `utcoffset()` of a wall time with an explicit `fold`.
    pub fn local_offset_fold(&self, local: i64, fold: bool) -> Option<i64> {
        let lt = if fold {
            &self.trans_local_fold
        } else {
            &self.trans_local
        };
        if !lt.is_empty() && local < lt[0] {
            return self.before;
        }
        if lt.is_empty() || local > *lt.last().unwrap() {
            return Some(match &self.after {
                After::Fixed(offset) => *offset,
                After::Rule(rule) => rule.local_fold(local, year_of(local), fold),
            });
        }
        let idx = lt.partition_point(|t| *t <= local) - 1;
        Some(self.offsets[idx])
    }
    /// Whether `fromutc` marks a UTC instant's wall time as its second
    /// occurrence (`fold=1`), following zoneinfo's detection.
    pub fn utc_fold(&self, utc: i64) -> bool {
        let tu = &self.trans_utc;
        let n = tu.len();
        if n > 0 && utc < tu[0] {
            return false;
        }
        if n == 0 || utc > tu[n - 1] {
            match &self.after {
                After::Rule(rule) => return rule.utc_fold(utc, year_of(utc)),
                After::Fixed(_) if n == 0 => return false,
                After::Fixed(_) => {}
            }
        }
        let idx = tu.partition_point(|t| *t <= utc);
        let (previous, current) = if n > 1 && utc >= tu[1] {
            (Some(self.offsets[idx - 2]), self.offsets[idx - 1])
        } else if utc > tu[n - 1] {
            let After::Fixed(after) = self.after else {
                unreachable!()
            };
            (Some(self.offsets[n - 1]), after)
        } else {
            (self.before, self.offsets[0])
        };
        previous.is_some_and(|previous| previous - current > utc - tu[idx - 1])
    }
    /// The offset `fromutc` applies to a UTC instant.
    pub fn utc_offset(&self, utc: i64) -> Option<i64> {
        let tu = &self.trans_utc;
        if !tu.is_empty() && utc < tu[0] {
            return self.before;
        }
        if tu.is_empty() || utc > *tu.last().unwrap() {
            return Some(match &self.after {
                After::Fixed(offset) => *offset,
                After::Rule(rule) => rule.utc(utc, year_of(utc)),
            });
        }
        let idx = tu.partition_point(|t| *t <= utc);
        Some(self.offsets[idx - 1])
    }

    /// UTC, offset 0, with no transitions.
    pub fn utc(key: &str) -> Zone {
        Zone {
            key: key.into(),
            trans_utc: Vec::new(),
            trans_local: Vec::new(),
            trans_local_fold: Vec::new(),
            offsets: Vec::new(),
            before: None,
            after: After::Fixed(0),
        }
    }
    /// A zone from UTC transitions `(at, offset)` after an initial offset,
    /// with an EU-style rule afterwards (the pinned Europe/Vienna table).
    pub fn pinned(
        key: &str,
        initial: i64,
        transitions: &[(i64, i64)],
        rule: Option<(i64, i64)>,
    ) -> Zone {
        let mut utcoff = vec![initial];
        let mut trans_idx = Vec::new();
        let mut trans_utc = Vec::new();
        for (at, offset) in transitions {
            let index = utcoff.iter().position(|o| o == offset).unwrap_or_else(|| {
                utcoff.push(*offset);
                utcoff.len() - 1
            });
            trans_utc.push(*at);
            trans_idx.push(index);
        }
        let isdst: Vec<bool> = utcoff.iter().map(|o| *o > initial.max(3600)).collect();
        let after = rule.map(|(std, dst)| {
            After::Rule(Rule {
                std,
                dst,
                start: Offset::Calendar {
                    m: 3,
                    w: 5,
                    d: 0,
                    seconds: 2 * 3600,
                },
                end: Offset::Calendar {
                    m: 10,
                    w: 5,
                    d: 0,
                    seconds: 3 * 3600,
                },
            })
        });
        build(key, trans_utc, trans_idx, utcoff, isdst, after).expect("pinned zone")
    }
}

fn build(
    key: &str,
    trans_utc: Vec<i64>,
    trans_idx: Vec<usize>,
    utcoff: Vec<i64>,
    isdst: Vec<bool>,
    after: Option<After>,
) -> Option<Zone> {
    if trans_idx.iter().any(|i| *i >= utcoff.len()) {
        return None;
    }
    // `_ts_to_local`, fold 0: each transition at the larger adjacent offset.
    let mut trans_local = trans_utc.clone();
    let mut trans_local_fold = trans_utc.clone();
    if !trans_utc.is_empty() {
        let (a, b) = if utcoff.len() > 1 {
            (utcoff[0], utcoff[trans_idx[0]])
        } else {
            (utcoff[0], utcoff[0])
        };
        trans_local[0] += a.max(b);
        trans_local_fold[0] += a.min(b);
        for i in 1..trans_idx.len() {
            let (a, b) = (utcoff[trans_idx[i - 1]], utcoff[trans_idx[i]]);
            trans_local[i] += a.max(b);
            trans_local_fold[i] += a.min(b);
        }
    }
    let offsets: Vec<i64> = trans_idx.iter().map(|i| utcoff[*i]).collect();
    let before = isdst
        .iter()
        .position(|dst| !dst)
        .map(|i| utcoff[i])
        .or_else(|| offsets.first().copied());
    let after = match after {
        Some(after) => after,
        None => After::Fixed(*offsets.last().or(utcoff.last())?),
    };
    Some(Zone {
        key: key.into(),
        trans_utc,
        trans_local,
        trans_local_fold,
        offsets,
        before,
        after,
    })
}

/// Why a key resolves to no zone (`ZoneInfoNotFoundError`/`ValueError`).
#[derive(Debug, PartialEq, Eq)]
pub struct Unknown;

/// `zoneinfo.TZPATH`: absolute `PYTHONTZPATH` entries, else the POSIX default.
pub fn tzpath(environment: Option<&str>) -> Vec<PathBuf> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    match environment {
        Some(value) if !value.is_empty() => value
            .split(separator)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect(),
        Some(_) => Vec::new(),
        None if cfg!(windows) => Vec::new(),
        None => [
            "/usr/share/zoneinfo",
            "/usr/lib/zoneinfo",
            "/usr/share/lib/zoneinfo",
            "/etc/zoneinfo",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect(),
    }
}

/// `posixpath.normpath`.
fn normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let initial = if path.starts_with("//") && !path.starts_with("///") {
        2
    } else if path.starts_with('/') {
        1
    } else {
        0
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part != ".." || (initial == 0 && parts.is_empty()) || parts.last() == Some(&"..") {
            parts.push(part);
        } else if !parts.is_empty() {
            parts.pop();
        }
    }
    let joined = format!("{}{}", "/".repeat(initial), parts.join("/"));
    if joined.is_empty() {
        ".".into()
    } else {
        joined
    }
}

/// `_tzpath._validate_tzfile_path`.
fn valid_key(key: &str) -> bool {
    if key.starts_with('/') || key.contains('\0') {
        return false;
    }
    let normalized = normpath(key);
    if normalized.chars().count() != key.chars().count() {
        return false;
    }
    normpath(&format!("_/{normalized}")).starts_with("_/")
}

/// Why `ZoneInfo(key)` raises: `ZoneInfoNotFoundError`, or another
/// exception (an invalid key, directory or TZif file) with its message.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    NotFound,
    Invalid(String),
}

/// [`load`], keeping Python's split between unknown keys and other failures.
pub fn lookup(key: &str, tzpath: &[PathBuf]) -> Result<Zone, Failure> {
    if !valid_key(key) {
        let reason = if key.contains('\0') {
            "embedded null byte".to_owned()
        } else if key.starts_with('/') {
            format!("ZoneInfo keys may not be absolute paths, got: {key}")
        } else if normpath(key).chars().count() != key.chars().count() {
            format!("ZoneInfo keys must be normalized relative paths, got: {key}")
        } else {
            format!("ZoneInfo keys must refer to subdirectories of TZPATH, got: {key}")
        };
        return Err(Failure::Invalid(reason));
    }
    for root in tzpath {
        let path = root.join(key);
        if path.is_file() {
            return parse(key, &path)
                .ok_or_else(|| Failure::Invalid("Invalid TZif file: magic not found".into()));
        }
    }
    if tzpath.iter().any(|root| root.join(key).is_dir()) {
        return Err(Failure::Invalid("[Errno 21] Is a directory".into()));
    }
    Err(Failure::NotFound)
}

/// `ZoneInfo(key)` from `tzpath`. Unknown keys, invalid TZif data and keys
/// Python could only answer from its bundled tzdata package are refused.
pub fn load(key: &str, tzpath: &[PathBuf]) -> Result<Zone, Unknown> {
    if !valid_key(key) {
        return Err(Unknown);
    }
    for root in tzpath {
        let path = root.join(key);
        if path.is_file() {
            return parse(key, &path).ok_or(Unknown);
        }
    }
    Err(Unknown)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(slice)
    }
    fn int(&mut self, size: usize) -> Option<i64> {
        let raw = self.take(size)?;
        Some(match size {
            1 => i64::from(raw[0] as i8),
            4 => i64::from(i32::from_be_bytes(raw.try_into().ok()?)),
            _ => i64::from_be_bytes(raw.try_into().ok()?),
        })
    }
    fn header(&mut self) -> Option<(u32, [usize; 6])> {
        if self.take(4)? != b"TZif" {
            return None;
        }
        let version = match self.take(1)?[0] {
            0 => 1,
            b @ b'0'..=b'9' => u32::from(b - b'0'),
            _ => return None,
        };
        self.take(15)?;
        let mut counts = [0; 6];
        for count in &mut counts {
            *count = usize::try_from(self.int(4)?).ok()?;
        }
        Some((version, counts))
    }
}

fn parse(key: &str, path: &Path) -> Option<Zone> {
    let bytes = std::fs::read(path).ok()?;
    let mut reader = Reader {
        bytes: &bytes,
        at: 0,
    };
    let (version, mut counts) = reader.header()?;
    let size = if version == 1 { 4 } else { 8 };
    if version != 1 {
        let [isut, isstd, leap, time, kind, chars] = counts;
        reader.take(time * 5 + kind * 6 + chars + leap * 8 + isstd + isut)?;
        counts = reader.header()?.1;
    }
    let [isut, isstd, leap, time, kind, chars] = counts;
    let trans_utc = (0..time)
        .map(|_| reader.int(size))
        .collect::<Option<Vec<_>>>()?;
    let trans_idx = (0..time)
        .map(|_| reader.take(1).map(|b| usize::from(b[0])))
        .collect::<Option<Vec<_>>>()?;
    let mut utcoff = Vec::new();
    let mut isdst = Vec::new();
    for _ in 0..kind {
        utcoff.push(reader.int(4)?);
        isdst.push(reader.int(1)? != 0);
        reader.take(1)?;
    }
    reader.take(chars)?;
    let tz = if version >= 2 {
        reader.take(isut + isstd + leap * 12)?;
        if reader.take(1)? != b"\n" {
            return None;
        }
        let rest = &reader.bytes[reader.at..];
        let end = rest.iter().position(|b| *b == b'\n')?;
        Some(std::str::from_utf8(&rest[..end]).ok()?.to_owned())
    } else {
        None
    };
    let after = match tz.filter(|t| !t.is_empty()) {
        Some(tz) => Some(rule(&tz)?),
        None if trans_idx.is_empty() && utcoff.is_empty() => return None,
        None => None,
    };
    build(key, trans_utc, trans_idx, utcoff, isdst, after)
}

fn ascii_digits(text: &str, min: usize, max: usize) -> Option<(i64, &str)> {
    let count = text
        .bytes()
        .take_while(u8::is_ascii_digit)
        .take(max)
        .count();
    if count < min {
        return None;
    }
    Some((text[..count].parse().ok()?, &text[count..]))
}
/// `[+-]?hhh(:mm(:ss)?)?` as `(sign, h, m, s, rest)`; `whole` requires the end.
fn clock(text: &str) -> Option<(bool, i64, i64, i64, &str)> {
    let (negative, text) = match text.as_bytes().first() {
        Some(b'+') => (false, &text[1..]),
        Some(b'-') => (true, &text[1..]),
        _ => (false, text),
    };
    let (h, mut rest) = ascii_digits(text, 1, 3)?;
    let (mut m, mut s) = (0, 0);
    if let Some((minutes, after)) = rest.strip_prefix(':').and_then(|t| ascii_digits(t, 2, 2)) {
        m = minutes;
        rest = after;
        if let Some((seconds, after)) = rest.strip_prefix(':').and_then(|t| ascii_digits(t, 2, 2)) {
            s = seconds;
            rest = after;
        }
    }
    Some((negative, h, m, s, rest))
}
fn abbreviation(text: &str) -> Option<&str> {
    if let Some(quoted) = text.strip_prefix('<') {
        let end = quoted.find('>')?;
        let inner = &quoted[..end];
        if inner.is_empty()
            || !inner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-')
        {
            return None;
        }
        return Some(&quoted[end + 1..]);
    }
    let count = text
        .chars()
        .take_while(|c| !matches!(c, '<' | '0'..='9' | ':' | '.' | '+' | '-'))
        .map(char::len_utf8)
        .sum::<usize>();
    (count > 0).then(|| &text[count..])
}
/// `_parse_tz_delta`: POSIX offsets are west-positive.
fn delta(negative: bool, h: i64, m: i64, s: i64) -> Option<i64> {
    if h > 24 {
        return None;
    }
    let total = h * 3600 + m * 60 + s;
    Some(if negative { total } else { -total })
}
fn transition(text: &str) -> Option<Offset> {
    let (date, time) = match text.split_once('/') {
        Some((date, time)) => (date, Some(time)),
        None => (text, None),
    };
    let seconds = match time {
        None => 2 * 3600,
        Some(time) => {
            let (negative, h, m, s, rest) = clock(time)?;
            if !rest.is_empty() || h > 167 {
                return None;
            }
            let total = h * 3600 + m * 60 + s;
            if negative {
                -total
            } else {
                total
            }
        }
    };
    if let Some(spec) = date.strip_prefix('M') {
        let (m, rest) = ascii_digits(spec, 1, 2)?;
        let rest = rest.strip_prefix('.')?;
        let (w, rest) = ascii_digits(rest, 1, 1)?;
        // CPython's pattern leaves the second separator unescaped.
        let mut chars = rest.chars();
        chars.next()?;
        let (d, rest) = ascii_digits(chars.as_str(), 1, 1)?;
        if !rest.is_empty()
            || !(1..=12).contains(&m)
            || !(1..=5).contains(&w)
            || !(0..=6).contains(&d)
        {
            return None;
        }
        return Some(Offset::Calendar { m, w, d, seconds });
    }
    let (julian, number) = match date.strip_prefix('J') {
        Some(number) => (true, number),
        None => (false, date),
    };
    let d = i64::try_from(crate::parse_int(number).ok()?).ok()?;
    if !(i64::from(julian)..=365).contains(&d) {
        return None;
    }
    Some(Offset::Day { d, julian, seconds })
}
/// `_parse_tz_str`.
fn rule(tz: &str) -> Option<After> {
    let (offset, rules) = match tz.split_once(',') {
        Some((offset, rules)) => (offset, Some(rules)),
        None => (tz, None),
    };
    let rest = abbreviation(offset)?;
    if rest.is_empty() {
        return rules.is_none().then_some(After::Fixed(0));
    }
    let (negative, h, m, s, rest) = clock(rest)?;
    let std = delta(negative, h, m, s)?;
    if rest.is_empty() {
        return rules.is_none().then_some(After::Fixed(std));
    }
    let rest = abbreviation(rest)?;
    let dst = if rest.is_empty() {
        std + 3600
    } else {
        let (negative, h, m, s, rest) = clock(rest)?;
        if !rest.is_empty() {
            return None;
        }
        delta(negative, h, m, s)?
    };
    let (start, end) = rules?.split_once(',')?;
    Some(After::Rule(Rule {
        std,
        dst,
        start: transition(start)?,
        end: transition(end)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn civil_round_trip_and_keys() {
        for days in [-719_162, -1, 0, 59, 10_957, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-719_162), (1, 1, 1));
        for (key, ok) in [
            ("Europe/Vienna", true),
            ("...", true),
            ("", false),
            ("..", false),
            ("./UTC", false),
            ("Europe//Vienna", false),
            ("Europe/Vienna/", false),
            ("a/../b", false),
            ("/etc/passwd", false),
            ("a\0b", false),
        ] {
            assert_eq!(valid_key(key), ok, "{key:?}");
        }
        assert_eq!(normpath("//a//./b/../c/"), "//a/c");
        assert_eq!(normpath("../../a"), "../../a");
        assert_eq!(normpath("/../a"), "/a");
    }
    #[test]
    fn tzpath_keeps_only_absolute_entries() {
        let separator = if cfg!(windows) { ";" } else { ":" };
        let root = if cfg!(windows) { "C:\\zones" } else { "/zones" };
        assert_eq!(
            tzpath(Some(&format!("relative{separator}{root}"))),
            vec![PathBuf::from(root)]
        );
        assert!(tzpath(Some("")).is_empty());
        assert_eq!(tzpath(None).is_empty(), cfg!(windows));
    }
    #[test]
    fn posix_rules_parse_like_cpython() {
        assert_eq!(rule("UTC0"), Some(After::Fixed(0)));
        assert_eq!(rule("<+0545>-5:45"), Some(After::Fixed(20_700)));
        assert_eq!(rule("EST5"), Some(After::Fixed(-18_000)));
        assert_eq!(rule("EST5,M3.2.0,M11.1.0"), None);
        assert_eq!(rule("EST5EDT"), None);
        assert_eq!(rule("A25"), None);
        let Some(After::Rule(rule)) = rule("<-02>2<-01>,M3.5.0/-1,M10.5x0/0") else {
            panic!()
        };
        assert_eq!((rule.std, rule.dst), (-7200, -3600));
        assert_eq!(
            rule.start,
            Offset::Calendar {
                m: 3,
                w: 5,
                d: 0,
                seconds: -3600
            }
        );
        assert_eq!(
            rule.end,
            Offset::Calendar {
                m: 10,
                w: 5,
                d: 0,
                seconds: 0
            }
        );
        assert!(transition("J0").is_none());
        assert!(transition("0").is_some());
        assert!(transition("M3.5.0/168").is_none());
    }
}
