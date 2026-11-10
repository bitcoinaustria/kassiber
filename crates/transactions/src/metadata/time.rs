//! CPython ISO filter timestamps, using the component parser's calendar rules.
use crate::sql::Result;
use kassiber_envelope::AppError;
use kassiber_store::rusqlite::{functions::FunctionFlags, Connection};

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
        let (day, used) = if extended
            && s.get(end) == Some(&b'-')
            && !s.get(end + 2).is_some_and(u8::is_ascii_digit)
        {
            (digits(s.get(end + 1..end + 2)?)?, end + 2)
        } else if !extended && s.get(end).is_some_and(u8::is_ascii_digit) && {
            let n = s[end..].iter().take_while(|b| b.is_ascii_digit()).count();
            n == 1 || n % 2 == 0
        } {
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
fn parsed(raw: &str) -> Option<i64> {
    let s = kassiber_pytext::strip(raw);
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
        // CPython's time parser consumes one trailing separator before a
        // zone, or ignores the remainder after six fractional digits.
        let raw_local = &rest[..at];
        let local = if let Some(dot) = raw_local.find(['.', ',']) {
            let digits = raw_local[dot + 1..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            if digits >= 6 {
                &raw_local[..dot + 1 + digits]
            } else if dot + 1 == raw_local.len() {
                &raw_local[..dot]
            } else {
                raw_local
            }
        } else if raw_local
            .chars()
            .last()
            .is_some_and(|c| !c.is_ascii_digit())
            || (!raw_local.contains(':') && matches!(raw_local.len(), 3 | 5))
        {
            &raw_local[..raw_local.char_indices().last()?.0]
        } else {
            raw_local
        };
        (clock(local.as_bytes(), false)?, offset)
    } else {
        (clock(rest.as_bytes(), false)?, 0)
    };
    Some(day * 86_400_000_000 + local - offset)
}

pub(crate) fn normalized(raw: &str) -> Option<i64> {
    let stripped = kassiber_pytext::strip(raw);
    let with_offset = stripped
        .strip_suffix('Z')
        .map(|prefix| format!("{prefix}+00:00"));
    parsed(with_offset.as_deref().unwrap_or(stripped))
}

/// Exact persisted instants, matching Python's best-effort timestamp parser.
pub(crate) fn instant(raw: &str) -> Option<i64> {
    normalized(raw).filter(|us| (0..315_537_897_600_000_000).contains(us))
}

/// Bound comparisons use exact microseconds, never lexical suffixes or SQLite
/// date functions (which round sub-millisecond instants). SQL names/operators
/// are internal literals. Unknown persisted values cannot satisfy a bound.
pub fn predicate(conn: &Connection, column: &str, op: &str) -> Result<String> {
    let installed: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_function_list WHERE name='kassiber_timestamp_us')",
            [],
            |row| row.get(0),
        )
        .map_err(crate::sql::error)?;
    if !installed {
        conn.create_scalar_function(
            "kassiber_timestamp_us",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                Ok(context
                    .get::<Option<String>>(0)?
                    .as_deref()
                    .and_then(instant))
            },
        )
        .map_err(crate::sql::error)?;
    }
    Ok(format!(
        "kassiber_timestamp_us({column}) {op} kassiber_timestamp_us(?)"
    ))
}

pub fn timestamp(raw: Option<&str>, field: &str) -> Result<String> {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Ok(String::new());
    };
    let us = normalized(raw).ok_or_else(|| {
        AppError::new("validation", format!("Invalid {field} timestamp '{raw}'"))
            .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
    })?;
    format_timestamp(us)
}

/// Microseconds since 0001-01-01, the same epoch as the ISO parser.
pub(crate) fn format_timestamp(us: i64) -> Result<String> {
    if !(0..315_537_897_600_000_000).contains(&us) {
        return Err(AppError::new("internal_error", "date value out of range"));
    }
    let day = us / 86_400_000_000;
    let mut low = 1;
    let mut high = 10000;
    while low + 1 < high {
        let mid = (low + high) / 2;
        if days(mid, 1, 1).unwrap() <= day {
            low = mid;
        } else {
            high = mid;
        }
    }
    let year = low;
    let month = (1..=12)
        .rev()
        .find(|m| days(year, *m, 1).unwrap() <= day)
        .unwrap();
    let date = day - days(year, month, 1).unwrap() + 1;
    let secs = (us % 86_400_000_000) / 1_000_000;
    let fraction = us % 1_000_000;
    let fraction = if fraction == 0 {
        String::new()
    } else {
        format!(".{fraction:06}")
    };
    Ok(format!(
        "{year:04}-{month:02}-{date:02}T{:02}:{:02}:{:02}{fraction}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::get;
    use kassiber_pyjson::Value;

    #[test]
    fn sql_instants_match_the_cpython_timestamp_vectors() {
        let conn = Connection::open_in_memory().unwrap();
        let _ = predicate(&conn, "stamp", ">=").unwrap();
        let data =
            kassiber_pytext::loads(include_str!("../../tests/data/metadata-vectors.json")).unwrap();
        let Some(Value::List(times)) = get(&data, "timestamps") else {
            panic!("missing timestamp vectors")
        };
        for row in times {
            let raw = get(row, "input").and_then(crate::sql::string).unwrap();
            let expected = if get(row, "code") == Some(&Value::Null) {
                get(row, "value")
                    .and_then(crate::sql::string)
                    .filter(|s| !s.is_empty())
            } else {
                None
            };
            let actual: Option<i64> = conn
                .query_row("SELECT kassiber_timestamp_us(?)", [raw], |r| r.get(0))
                .unwrap();
            assert_eq!(
                actual.map(|us| format_timestamp(us).unwrap()).as_deref(),
                expected,
                "{row:?}"
            );
        }
    }
}
