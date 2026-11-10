//! CPython 3.11 datetime.fromisoformat instants used by component validation.
//! Naive values mean UTC. Keep microseconds; chronology is not a text sort.
use crate::{codec::pystr, python_text::PythonText};
use serde_json::Value;

pub const SKEW: i64 = 48 * 3600 * 1_000_000;

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
// Custody follows time_utils.py's trailing-Z rewrite and CPython offset parsing.
// Keep this vector-pinned path separate from setup's iso_datetime helper.
fn parsed(v: &Value) -> Option<i64> {
    parsed_impl(v, true)
}
// The ledger cutoff calls fromisoformat directly, without str.strip or a
// bounded astimezone conversion. Datetime subtraction may cross year bounds.
pub(crate) fn strict_parsed(v: &Value) -> Option<i64> {
    parsed_impl(v, false)
}
fn parsed_impl(v: &Value, strip: bool) -> Option<i64> {
    if v.is_null() || v == "" {
        return None;
    }
    let raw = match v {
        Value::String(text) => std::borrow::Cow::Borrowed(text.as_str()),
        v => std::borrow::Cow::Owned(pystr(v)),
    };
    let raw = if strip { raw.py_strip() } else { &raw };
    // The rewritten text of an ordinary timestamp fits on the stack.
    let mut buffer = [0u8; 64];
    let rewritten;
    let s = if let Some(prefix) = raw.strip_suffix('Z') {
        let len = prefix.len() + 6;
        if len <= buffer.len() {
            buffer[..prefix.len()].copy_from_slice(prefix.as_bytes());
            buffer[prefix.len()..len].copy_from_slice(b"+00:00");
            std::str::from_utf8(&buffer[..len]).expect("text and an ASCII zone")
        } else {
            rewritten = format!("{prefix}+00:00");
            rewritten.as_str()
        }
    } else {
        raw
    };
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
fn range(value: i64) -> Result<i64, super::Error> {
    if !(0..315_537_897_600_000_000).contains(&value) {
        return Err(super::Error {
            code: "OverflowError",
            message: "date value out of range".into(),
        });
    }
    Ok(value)
}
pub fn instant(v: &Value) -> Result<Option<i64>, super::Error> {
    parsed(v).map(range).transpose()
}
pub fn skewed(when: i64) -> Result<i64, super::Error> {
    range(when + SKEW)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpython_component_times() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_component_semantics.json"
        ))
        .unwrap();
        let mismatches: Vec<_> = vectors["times"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| match instant(&v["input"]) {
                Ok(us) => !v["error_code"].is_null() || us != v["us"].as_i64(),
                Err(e) => v["error_code"] != e.code,
            })
            .map(|v| {
                format!(
                    "{}: {:?} != {:?}",
                    v["input"],
                    instant(&v["input"]),
                    v["us"].as_i64()
                )
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "{} mismatches: {:?}",
            mismatches.len(),
            &mismatches[..mismatches.len().min(100)]
        );
    }
    #[test]
    fn cpython_component_integers() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_component_semantics.json"
        ))
        .unwrap();
        for v in vectors["integers"].as_array().unwrap() {
            assert_eq!(
                crate::codec::integer(&v["input"]).ok(),
                (!v["integer"].is_null()).then(|| v["integer"].clone()),
                "{}",
                v["input"]
            );
        }
    }
    #[test]
    fn cpython_component_uppercase() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_component_semantics.json"
        ))
        .unwrap();
        for v in vectors["uppercase"].as_array().unwrap() {
            assert_eq!(
                v["input"].as_str().unwrap().py_upper(),
                v["upper"],
                "{}",
                v["input"]
            );
        }
    }
}
