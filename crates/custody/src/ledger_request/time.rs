//! Two deliberately independent timestamp interpretations in a request row.
use super::*;
const DAY: i64 = 86_400_000_000;
const EPOCH: i64 = 719_162 * DAY;
const END: i64 = 3_652_059 * DAY;
include!("vienna.rs");
fn digit(s: &str) -> Option<i64> {
    s.bytes().try_fold(0, |n, c| {
        c.is_ascii_digit().then(|| n * 10 + i64::from(c - b'0'))
    })
}
fn take<'a>(s: &mut &'a str, n: usize) -> Option<&'a str> {
    let result = s.get(..n)?;
    *s = &s[n..];
    Some(result)
}
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = y - 1;
    let leap = (y + 1) % 4 == 0 && ((y + 1) % 100 != 0 || (y + 1) % 400 == 0);
    let months = [
        31,
        if leap { 29 } else { 28 },
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
    y * 365 + y / 4 - y / 100 + y / 400 + months[..(m - 1) as usize].iter().sum::<i64>() + d - 1
}
fn date_error(y: i64, m: i64, d: i64) -> Option<String> {
    if !(1..=9999).contains(&y) {
        Some(format!("year {y} is out of range"))
    } else if !(1..=12).contains(&m) {
        Some("month must be in 1..12".into())
    } else {
        let length = if m == 12 {
            days(y + 1, 1, 1) - days(y, m, 1)
        } else {
            days(y, m + 1, 1) - days(y, m, 1)
        };
        (!(1..=length).contains(&d)).then(|| "day is out of range for month".into())
    }
}
fn civil(when: i64) -> Option<String> {
    if !(0..END).contains(&when) {
        return None;
    }
    Some(crate::gaps::iso_datetime(when)[..10].into())
}
fn year(when: i64) -> i64 {
    civil(when)
        .and_then(|s| s[..4].parse().ok())
        .unwrap_or(9999)
}
fn vienna(when: i64) -> Option<String> {
    if !(0..END).contains(&when) {
        return None;
    }
    let unix = (when - EPOCH).div_euclid(1_000_000);
    let offset = if unix <= VIENNA.last().unwrap().0 {
        VIENNA
            .iter()
            .rev()
            .find(|(at, _)| *at <= unix)
            .map(|(_, offset)| *offset)
            .unwrap_or(3921)
    } else {
        let y = year(when);
        let sunday = |month| {
            let last = days(y, month, 31);
            last - (last + 1) % 7
        };
        let start = sunday(3) * DAY + 3_600_000_000;
        let end = sunday(10) * DAY + 3_600_000_000;
        if when >= start && when < end {
            7200
        } else {
            3600
        }
    };
    civil(when + offset * 1_000_000)
}
struct Parsed {
    y: i64,
    m: i64,
    d: i64,
    h: i64,
    min: i64,
    sec: i64,
    micro: i64,
    offset: Option<i64>,
}
fn parse(raw: &str) -> Result<Parsed, String> {
    let unknown = || format!("Unknown string format: {raw}");
    // Python's regex uses Unicode decimal digits; separators stay literal.
    let normalized: String = raw
        .chars()
        .map(|c| {
            crate::python_text::decimal_digit(c)
                .map(|d| char::from(b'0' + d))
                .unwrap_or(c)
        })
        .collect();
    let mut s = normalized.as_str();
    let y = digit(take(&mut s, 4).ok_or_else(unknown)?).ok_or_else(unknown)?;
    if take(&mut s, 1) != Some("-") {
        return Err(unknown());
    }
    let m = digit(take(&mut s, 2).ok_or_else(unknown)?).ok_or_else(unknown)?;
    if take(&mut s, 1) != Some("-") {
        return Err(unknown());
    }
    let d = digit(take(&mut s, 2).ok_or_else(unknown)?).ok_or_else(unknown)?;
    let (mut h, mut min, mut sec, mut micro) = (0, 0, 0, 0);
    let mut offset = None;
    if !s.is_empty() {
        if !matches!(take(&mut s, 1), Some("T" | " ")) {
            return Err(unknown());
        }
        h = digit(take(&mut s, 2).ok_or_else(unknown)?).ok_or_else(unknown)?;
        if take(&mut s, 1) != Some(":") {
            return Err(unknown());
        }
        min = digit(take(&mut s, 2).ok_or_else(unknown)?).ok_or_else(unknown)?;
        if let Some(tail) = s.strip_prefix(':') {
            s = tail;
            sec = digit(take(&mut s, 2).ok_or_else(unknown)?).ok_or_else(unknown)?;
            if s.starts_with(['.', ',']) {
                s = &s[1..];
                let count = s.bytes().take_while(u8::is_ascii_digit).count();
                if count == 0 {
                    return Err(unknown());
                }
                let n = count.min(6);
                micro = digit(&s[..n]).unwrap() * 10_i64.pow((6 - n) as u32);
                s = &s[count..];
            }
        }
        if s == "Z" || s == "z" {
            offset = Some(0);
            s = "";
        } else if s.starts_with(['+', '-']) {
            let negative = s.starts_with('-');
            s = &s[1..];
            let oh = digit(take(&mut s, 2).ok_or_else(unknown)?).ok_or_else(unknown)?;
            if s.starts_with(':') {
                s = &s[1..];
                if s.len() < 2 {
                    return Err(unknown());
                }
            }
            let om = if s.is_empty() {
                0
            } else {
                digit(take(&mut s, 2).ok_or_else(unknown)?).ok_or_else(unknown)?
            };
            let off = (oh * 60 + om) * 60 * if negative { -1 } else { 1 };
            if off.abs() >= 86400 {
                // timedelta repr normalizes a negative offset into days.
                let total = off;
                let day = total.div_euclid(86400);
                let seconds = total.rem_euclid(86400);
                let mut fields = Vec::new();
                if day != 0 {
                    fields.push(format!("days={day}"));
                }
                if seconds != 0 {
                    fields.push(format!("seconds={seconds}"));
                }
                return Err(format!("offset must be a timedelta strictly between -timedelta(hours=24) and timedelta(hours=24), not datetime.timedelta({}).",fields.join(", ")));
            }
            offset = Some(off);
        }
    }
    if !s.is_empty() {
        return Err(unknown());
    }
    let error = date_error(y, m, d).or_else(|| {
        if h > 23 {
            Some("hour must be in 0..23".into())
        } else if min > 59 {
            Some("minute must be in 0..59".into())
        } else if sec > 59 {
            Some("second must be in 0..59".into())
        } else {
            None
        }
    });
    if let Some(error) = error {
        return Err(format!("{error}: {raw}"));
    }
    Ok(Parsed {
        y,
        m,
        d,
        h,
        min,
        sec,
        micro,
        offset,
    })
}
pub(super) fn timestamp(value: &Value) -> Value {
    let Some(raw) = value.as_str() else {
        return json!({"raw":string(value),"error":format!("Parameter 'timestamp' has non-string value {}",repr(value)),"error_class":"TypeError"});
    };
    let p = match parse(raw) {
        Ok(p) => p,
        Err(e) => {
            return json!({"raw":raw,"error":format!("Error parsing parameter 'timestamp': {e}")})
        }
    };
    let Some(offset) = p.offset else {
        return json!({"raw":raw,"error":format!("Parameter 'timestamp' value has no timezone info: {raw}")});
    };
    let local = days(p.y, p.m, p.d) * DAY + (p.h * 3600 + p.min * 60 + p.sec) * 1_000_000 + p.micro;
    let us = local - offset * 1_000_000;
    let date = format!("{:04}-{:02}-{:02}", p.y, p.m, p.d);
    let fraction = if p.micro == 0 {
        String::new()
    } else {
        format!(".{:06}", p.micro)
    };
    let zone = format!(
        "{}{:02}:{:02}",
        if offset < 0 { "-" } else { "+" },
        offset.abs() / 3600,
        offset.abs() % 3600 / 60
    );
    let clock = format!("{:02}:{:02}:{:02}{fraction}{zone}", p.h, p.min, p.sec);
    json!({"raw":raw,"us":us-EPOCH,"offset_s":offset,"date":date,"year":p.y,"vienna_date":vienna(us),"display":format!("{date} {clock}"),"iso":format!("{date}T{clock}")})
}
// Normalize the calendar/basic spelling only to recover CPython constructor
// diagnostics after the independent fromisoformat parser rejects an instant.
// Never normalize the operand used for a successful cutoff conversion.
fn diagnostic_spelling(raw: &str) -> Option<String> {
    let extended = raw.as_bytes().get(4) == Some(&b'-');
    let end = if extended { 10 } else { 8 };
    let date = raw.get(..end)?;
    let mut result = if extended {
        if date.as_bytes().get(7) != Some(&b'-') {
            return None;
        }
        for range in [0..4, 5..7, 8..10] {
            digit(date.get(range)?)?;
        }
        date.to_owned()
    } else {
        digit(date)?;
        format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..])
    };
    let rest = &raw[end..];
    if rest.is_empty() {
        return Some(result);
    }
    let separator = rest.chars().next()?;
    let rest = &rest[separator.len_utf8()..];
    let zone = rest.find(['+', '-', 'Z', 'z']).unwrap_or(rest.len());
    let (clock, zone) = rest.split_at(zone);
    let fraction = clock.find(['.', ',']).unwrap_or(clock.len());
    let (base, fraction) = clock.split_at(fraction);
    let parts: Vec<_> = if base.contains(':') {
        base.split(':').collect()
    } else {
        if !base.bytes().all(|byte| byte.is_ascii_digit()) || !matches!(base.len(), 2 | 4 | 6) {
            return None;
        }
        (0..base.len())
            .step_by(2)
            .map(|i| &base[i..i + 2])
            .collect()
    };
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    for part in &parts {
        if part.len() != 2 {
            return None;
        }
        digit(part)?;
    }
    result.push('T');
    result.push_str(parts[0]);
    result.push(':');
    result.push_str(parts.get(1).unwrap_or(&"00"));
    result.push(':');
    result.push_str(parts.get(2).unwrap_or(&"00"));
    result.push_str(fraction);
    result.push_str(zone);
    Some(result)
}

pub(super) fn cutoff(value: &Value) -> Result<Value, Refusal> {
    let raw = string(value);
    let rewritten = raw.replace('Z', "+00:00");
    match crate::components::time::strict_parsed(&json!(rewritten)) {
        Some(us) => Ok(json!({"us":us-EPOCH})),
        None => {
            let message = diagnostic_spelling(&rewritten)
                .and_then(|candidate| match parse(&candidate) {
                    Err(error) if !error.starts_with("Unknown string format:") => Some(
                        error
                            .strip_suffix(&format!(": {candidate}"))
                            .unwrap_or(&error)
                            .to_owned(),
                    ),
                    _ => None,
                })
                .unwrap_or_else(|| {
                    format!("Invalid isoformat string: {}", repr(&json!(rewritten)))
                });
            Ok(json!({"error":message}))
        }
    }
}
