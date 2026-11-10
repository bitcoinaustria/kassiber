//! CPython 3.11 ISO parsing, following custody's date/clock rules.
//! Python rewrites trailing Z before fromisoformat and permits one ASCII
//! separator between a complete clock unit and its offset, including whitespace.
use kassiber_envelope::AppError;

fn digits(s: &[u8]) -> Option<i64> {
    s.iter().try_fold(0, |n, b| {
        b.is_ascii_digit().then(|| n * 10 + i64::from(b - b'0'))
    })
}
fn leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}
fn days(y: i64, m: i64, d: i64) -> Option<i64> {
    let lengths = [
        31,
        if leap(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || !(1..=lengths[(m - 1) as usize]).contains(&d)
    {
        return None;
    }
    let previous = y - 1;
    Some(
        previous * 365 + previous / 4 - previous / 100
            + previous / 400
            + lengths[..(m - 1) as usize].iter().sum::<i64>()
            + d
            - 1,
    )
}
fn date(s: &[u8]) -> Option<(i64, usize)> {
    let y = digits(s.get(..4)?)?;
    let extended = s.get(4) == Some(&b'-');
    let at = if extended { 5 } else { 4 };
    if s.get(at) == Some(&b'W') {
        let week = digits(s.get(at + 1..at + 3)?)?;
        let end = at + 3;
        let (day, used) = if extended && s.get(end) == Some(&b'-') {
            (digits(s.get(end + 1..end + 2)?)?, end + 2)
        } else if !extended && s.get(end).is_some_and(u8::is_ascii_digit) {
            (digits(s.get(end..end + 1)?)?, end + 1)
        } else {
            (1, end)
        };
        let jan1 = days(y, 1, 1)?;
        let weekday = jan1 % 7; // 0001-01-01 was Monday.
        let weeks = if weekday == 3 || (weekday == 2 && leap(y)) {
            53
        } else {
            52
        };
        if !(1..=weeks).contains(&week) || !(1..=7).contains(&day) {
            return None;
        }
        let jan4 = jan1 + 3;
        let result = jan4 - jan4 % 7 + (week - 1) * 7 + day - 1;
        return (result >= 0 && result <= days(9999, 12, 31)?).then_some((result, used));
    }
    let m = digits(s.get(at..at + 2)?)?;
    let day_at = at + 2 + usize::from(extended);
    if extended && s.get(at + 2) != Some(&b'-') {
        return None;
    }
    let d = digits(s.get(day_at..day_at + 2)?)?;
    Some((days(y, m, d)?, day_at + 2))
}
// ISO's fraction is a fraction of seconds even after hours or minutes in CPython.
fn clock(s: &[u8], offset: bool) -> Option<i64> {
    let hour = digits(s.get(..2)?)?;
    let mut at = 2;
    let mut minute = 0;
    let mut second = 0;
    let extended = s.get(at) == Some(&b':');
    if at < s.len() && !matches!(s[at], b'.' | b',') {
        if extended {
            at += 1;
        }
        minute = digits(s.get(at..at + 2)?)?;
        at += 2;
        if at < s.len() && !matches!(s[at], b'.' | b',') {
            if extended {
                if s[at] != b':' {
                    return None;
                }
                at += 1;
            }
            second = digits(s.get(at..at + 2)?)?;
            at += 2;
        }
    }
    let mut fraction = 0;
    if at < s.len() {
        if !matches!(s[at], b'.' | b',') {
            return None;
        }
        at += 1;
        let part = &s[at..];
        if part.is_empty() || !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let n = part.len().min(6);
        fraction = digits(&part[..n])? * 10_i64.pow((6 - n) as u32);
    }
    if !offset && (hour > 23 || minute > 59 || second > 59) {
        return None;
    }
    let result = (hour * 3600 + minute * 60 + second) * 1_000_000 + fraction;
    // fromisoformat normalizes oversized offset minutes/seconds, then checks
    // the whole offset is strictly smaller than 24 hours.
    (!offset || result < 86_400_000_000).then_some(result)
}
fn parsed(s: &str) -> Option<i64> {
    let (day, used) = date(s.as_bytes())?;
    let rest = s.get(used..)?;
    if rest.is_empty() {
        return Some(day * 86_400_000_000);
    }
    let separator = rest.chars().next()?;
    let rest = &rest[separator.len_utf8()..];
    let zone = rest.bytes().position(|b| matches!(b, b'Z' | b'+' | b'-'));
    let (local, offset) = if let Some(at) = zone {
        let zone = &rest[at..];
        let offset = if zone == "Z" {
            0
        } else {
            let value = clock(&zone.as_bytes()[1..], true)?;
            // CPython treats any all-zero h:m:s offset as UTC (fraction too).
            let value = if value < 1_000_000 { 0 } else { value };
            if zone.starts_with('-') {
                -value
            } else {
                value
            }
        };
        // CPython permits a trailing clock separator immediately before
        // a timezone marker, though the same naive spelling is invalid.
        let raw_local = &rest[..at];
        let local = raw_local
            .strip_suffix(|c: char| c.is_ascii() && !c.is_ascii_digit())
            .filter(|s| !s.contains(['.', ',']))
            .unwrap_or(raw_local);
        (clock(local.as_bytes(), false)?, offset)
    } else {
        (clock(rest.as_bytes(), false)?, 0)
    };
    Some(day * 86_400_000_000 + local - offset)
}

/// Best-effort parser used by Silent Payments. Microseconds since 0001-01-01.
pub(crate) fn instant(input: &str) -> Result<Option<i64>, AppError> {
    let raw = kassiber_pytext::strip(input);
    let raw = raw.strip_suffix('Z').map(|s| format!("{s}+00:00"));
    let value = parsed(
        raw.as_deref()
            .unwrap_or_else(|| kassiber_pytext::strip(input)),
    );
    if value.is_some_and(|v| !(0..315_537_897_600_000_000).contains(&v)) {
        return Err(AppError::new("internal_error", "date value out of range"));
    }
    Ok(value)
}
/// User/wallet import normalization; date-only inputs have a distinct Python rewrite.
pub fn timestamp(input: &str) -> Result<String, AppError> {
    let raw = kassiber_pytext::strip(input);
    let rewritten = (raw.chars().count() == 10).then(|| format!("{raw}T00:00:00+00:00"));
    let us = instant(rewritten.as_deref().unwrap_or(raw))?.ok_or_else(|| {
        AppError::new(
            "validation",
            format!("Invalid occurred_at/date value '{input}'"),
        )
        .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
    })?;
    Ok(render(us))
}
pub(crate) fn shifted(input: &str, seconds: i64) -> Result<String, AppError> {
    let us = instant(input)?.expect("normalized timestamp") + seconds * 1_000_000;
    if !(0..315_537_897_600_000_000).contains(&us) {
        return Err(AppError::new("internal_error", "date value out of range"));
    }
    Ok(render(us))
}
/// An [`instant`] as `datetime.replace(microsecond=0).isoformat()` with `Z`.
pub(crate) fn iso_seconds(us: i64) -> String {
    render(us)
}
/// `datetime.fromtimestamp(seconds, tz=timezone.utc)` rendered with `Z`, or
/// `None` where CPython raises for a year outside 1..=9999.
pub(crate) fn from_unix(seconds: &kassiber_pyjson::BigInt) -> Option<String> {
    let epoch = days(1970, 1, 1).expect("valid epoch") * 86_400;
    let seconds = i64::try_from(seconds.clone()).ok()?.checked_add(epoch)?;
    (0..315_537_897_600)
        .contains(&seconds)
        .then(|| render(seconds * 1_000_000))
}
fn render(us: i64) -> String {
    let day = us / 86_400_000_000;
    let seconds = (us % 86_400_000_000) / 1_000_000;
    let mut low = 1;
    let mut high = 10000;
    while low + 1 < high {
        let mid = (low + high) / 2;
        if days(mid, 1, 1).is_some_and(|d| d <= day) {
            low = mid;
        } else {
            high = mid;
        }
    }
    let year = low;
    let mut month = 1;
    while month < 12 && days(year, month + 1, 1).is_some_and(|d| d <= day) {
        month += 1;
    }
    let date = day - days(year, month, 1).expect("valid month") + 1;
    format!(
        "{year:04}-{month:02}-{date:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
