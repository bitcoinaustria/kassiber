//! CPython ISO parsing, adapted from the existing custody parser.
//! Instants are microseconds since 0001-01-01; rendering retains fractions.
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
pub fn parse(raw: &str) -> Option<i64> {
    parse_with_offset(raw).map(|(instant, _)| instant)
}

/// Parse an ISO instant and retain whether its spelling includes a UTC offset.
pub fn parse_with_offset(raw: &str) -> Option<(i64, bool)> {
    let s = kassiber_pytext::strip(raw);
    let (day, used) = date(s.as_bytes())?;
    let rest = s.get(used..)?;
    if rest.is_empty() {
        return Some((day * 86_400_000_000, false));
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
            .strip_suffix([':', '.', ','])
            .filter(|s| !s.contains(['.', ',']))
            .unwrap_or(raw_local);
        (clock(local.as_bytes(), false)?, offset)
    } else {
        (clock(rest.as_bytes(), false)?, 0)
    };
    let us = day * 86_400_000_000 + local - offset;
    (0..315_537_897_600_000_000)
        .contains(&us)
        .then_some((us, zone.is_some()))
}

pub const UNIX: i64 = 62_135_596_800_000_000;
pub fn format(us: i64) -> Option<String> {
    if !(0..315_537_897_600_000_000).contains(&us) {
        return None;
    }
    let day = us / 86_400_000_000;
    let mut lo = 1;
    let mut hi = 10000;
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if days(mid, 1, 1)? <= day {
            lo = mid
        } else {
            hi = mid
        }
    }
    let year = lo;
    let month = (1..=12)
        .rev()
        .find(|m| days(year, *m, 1).is_some_and(|d| d <= day))?;
    let date = day - days(year, month, 1)? + 1;
    let sec = us / 1_000_000 % 86400;
    let fraction = us % 1_000_000;
    let tail = if fraction == 0 {
        String::new()
    } else {
        format!(".{fraction:06}")
    };
    Some(format!(
        "{year:04}-{month:02}-{date:02}T{:02}:{:02}:{:02}{tail}Z",
        sec / 3600,
        sec / 60 % 60,
        sec % 60
    ))
}
pub fn now() -> i64 {
    UNIX + std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_micros() as i64
}
pub fn iso_now() -> String {
    format(now()).expect("current date")
}
pub fn epoch(seconds: i64) -> Option<String> {
    format(UNIX.checked_add(seconds.checked_mul(1_000_000)?)?)
}
