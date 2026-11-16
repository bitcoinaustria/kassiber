//! Local-time formatting as the web's `Intl` does it in en-US and de-AT: `formatHistoryDate`
//! (`dateStyle: medium, timeStyle: short`), `formatHistoryRelative` (`RelativeTimeFormat`,
//! `numeric: auto`) and Egress's `toLocaleTimeString` with two-digit fields.
//!
//! std has no time zone database, so the local offset is the system's current one (`date +%z`,
//! read once); a timestamp on the other side of a DST change is off by an hour.

use std::sync::OnceLock;

use crate::format::{Day, parse_ts};
use crate::i18n::{Lang, lang};

fn local_offset() -> f64 {
    static OFF: OnceLock<f64> = OnceLock::new();
    *OFF.get_or_init(|| {
        let out = std::process::Command::new("date").arg("+%z").output().ok();
        let s = out
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        if s.len() == 5 {
            let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
            let h: f64 = s[1..3].parse().unwrap_or(0.0);
            let m: f64 = s[3..5].parse().unwrap_or(0.0);
            sign * (h * 3600.0 + m * 60.0)
        } else {
            0.0
        }
    })
}

struct Local {
    day: Day,
    h: i64,
    m: i64,
    s: i64,
}

fn local(secs: f64) -> Local {
    let t = (secs + local_offset()).floor() as i64;
    let day = Day::from_epoch_days(t.div_euclid(86_400));
    let r = t.rem_euclid(86_400);
    Local {
        day,
        h: r / 3600,
        m: r / 60 % 60,
        s: r % 60,
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn h12(h: i64) -> (i64, &'static str) {
    let ap = if h < 12 { "AM" } else { "PM" };
    let h = h % 12;
    (if h == 0 { 12 } else { h }, ap)
}

/// `formatHistoryDate`: `Nov 15, 2026, 12:32 PM` / `15.11.2026, 12:32`.
pub fn history_date(ts: Option<&str>) -> String {
    let Some(ts) = ts.filter(|s| !s.is_empty()) else {
        return "Unknown time".into();
    };
    let Some(secs) = parse_ts(ts) else {
        return ts.to_owned();
    };
    let l = local(secs);
    match lang() {
        Lang::En => {
            let (h, ap) = h12(l.h);
            format!(
                "{} {}, {}, {}:{:02}\u{202f}{ap}",
                MONTHS[(l.day.m - 1) as usize],
                l.day.d,
                l.day.y,
                h,
                l.m
            )
        }
        Lang::De => format!(
            "{:02}.{:02}.{}, {:02}:{:02}",
            l.day.d, l.day.m, l.day.y, l.h, l.m
        ),
    }
}

/// `formatHistoryRelative`.
pub fn history_relative(ts: Option<&str>, now: f64) -> String {
    let Some(ts) = ts.filter(|s| !s.is_empty()) else {
        return "Unknown".into();
    };
    let Some(secs) = parse_ts(ts) else {
        return ts.to_owned();
    };
    let delta = (secs - now) * 1000.0;
    let abs = delta.abs();
    for (unit, size) in [
        ("day", 86_400_000.0),
        ("hour", 3_600_000.0),
        ("minute", 60_000.0),
    ] {
        if abs >= size {
            return relative(unit, (delta / size).round() as i64);
        }
    }
    "just now".into()
}

fn relative(unit: &str, n: i64) -> String {
    let a = n.abs();
    match lang() {
        Lang::En => {
            match (unit, n) {
                ("day", -1) => return "yesterday".into(),
                ("day", 1) => return "tomorrow".into(),
                ("day", 0) => return "today".into(),
                ("hour", 0) => return "this hour".into(),
                ("minute", 0) => return "this minute".into(),
                _ => {}
            }
            let u = if a == 1 {
                unit.to_owned()
            } else {
                format!("{unit}s")
            };
            if n < 0 {
                format!("{a} {u} ago")
            } else {
                format!("in {a} {u}")
            }
        }
        Lang::De => {
            match (unit, n) {
                ("day", -2) => return "vorgestern".into(),
                ("day", -1) => return "gestern".into(),
                ("day", 0) => return "heute".into(),
                ("day", 1) => return "morgen".into(),
                ("day", 2) => return "übermorgen".into(),
                ("hour", 0) => return "in dieser Stunde".into(),
                ("minute", 0) => return "in dieser Minute".into(),
                _ => {}
            }
            let u = match (unit, a == 1, n < 0) {
                ("day", true, _) => "Tag",
                ("day", false, true) => "Tagen",
                ("day", false, false) => "Tagen",
                ("hour", true, _) => "Stunde",
                ("hour", false, _) => "Stunden",
                ("minute", true, _) => "Minute",
                _ => "Minuten",
            };
            if n < 0 {
                format!("vor {a} {u}")
            } else {
                format!("in {a} {u}")
            }
        }
    }
}

/// The local calendar day of a timestamp, as days since the epoch (the Activity page groups
/// its edits by it).
pub fn local_day(ts: Option<&str>) -> Option<i64> {
    let secs = parse_ts(ts.filter(|s| !s.is_empty())?)?;
    Some(((secs + local_offset()) / 86_400.0).floor() as i64)
}

/// A day group's heading: "Today", "Yesterday", else the medium date (`Nov 15, 2026` /
/// `15.11.2026`).
pub fn day_heading(day: i64, now: f64) -> String {
    let today = ((now + local_offset()) / 86_400.0).floor() as i64;
    if day == today {
        return crate::i18n::t("sourceFunds:filters.date.today");
    }
    if day == today - 1 {
        return crate::i18n::t("sourceFunds:filters.date.yesterday");
    }
    let d = Day::from_epoch_days(day);
    match lang() {
        Lang::En => format!("{} {}, {}", MONTHS[(d.m - 1) as usize], d.d, d.y),
        Lang::De => format!("{:02}.{:02}.{}", d.d, d.m, d.y),
    }
}

/// The time of day (`12:32 PM` / `12:32`).
pub fn short_time(ts: Option<&str>) -> String {
    let Some(secs) = ts.filter(|s| !s.is_empty()).and_then(parse_ts) else {
        return "—".into();
    };
    let l = local(secs);
    match lang() {
        Lang::En => {
            let (h, ap) = h12(l.h);
            format!("{h}:{:02}\u{202f}{ap}", l.m)
        }
        Lang::De => format!("{:02}:{:02}", l.h, l.m),
    }
}

/// Egress's `toLocaleTimeString({hour, minute, second: "2-digit"})`.
pub fn clock_time(ts: &str) -> String {
    let Some(secs) = parse_ts(ts) else {
        return ts.to_owned();
    };
    let l = local(secs);
    match lang() {
        Lang::En => {
            let (h, ap) = h12(l.h);
            format!("{h:02}:{:02}:{:02}\u{202f}{ap}", l.m, l.s)
        }
        Lang::De => format!("{:02}:{:02}:{:02}", l.h, l.m, l.s),
    }
}

/// `new Date(ms).toLocaleTimeString(locale)` (Logs' raw-view deadline).
pub fn plain_time(secs: f64) -> String {
    let l = local(secs);
    match lang() {
        Lang::En => {
            let (h, ap) = h12(l.h);
            format!("{h}:{:02}:{:02}\u{202f}{ap}", l.m, l.s)
        }
        Lang::De => format!("{:02}:{:02}:{:02}", l.h, l.m, l.s),
    }
}

/// The Logs surface's `lineTime`: `toTimeString().slice(0, 8)`, always 24 h `HH:MM:SS`.
pub fn line_time(ts: &str) -> String {
    let Some(secs) = parse_ts(ts) else {
        return ts.to_owned();
    };
    let l = local(secs);
    format!("{:02}:{:02}:{:02}", l.h, l.m, l.s)
}
