//! `core.export_range.resolve_export_range`: a preset or a custom day span
//! in the book's calendar (Europe/Vienna for Austrian books, else UTC) or a
//! named IANA zone, as exact UTC bounds. The start is inclusive, the end
//! exclusive; weeks start on Monday. No range means every transaction.
use super::*;
use crate::zone::{civil_from_days, days_from_civil, Zone};
use std::path::PathBuf;

const PRESETS: &[&str] = &[
    "today",
    "yesterday",
    "this_week",
    "last_week",
    "this_month",
    "last_month",
    "this_quarter",
    "last_quarter",
    "this_year",
    "last_year",
];
const DAY: i64 = 86_400;
/// The datetime range CPython represents, in seconds since 1970.
const MIN: i64 = -62_135_596_800;
const MAX: i64 = 253_402_300_799;

type Date = (i64, i64, i64);

/// One resolved range.
#[derive(Clone, Debug)]
pub struct Range {
    pub preset: String,
    pub timezone: String,
    /// UTC seconds since 1970, start inclusive and end exclusive.
    pub start: i64,
    pub end: i64,
    pub first: Date,
    pub last: Date,
}

fn validation(message: String) -> AppError {
    AppError::new("validation", message)
}
fn overflow() -> AppError {
    AppError::new("internal_error", "date value out of range")
}
fn add_days(day: Date, n: i64) -> Result<Date> {
    let days = days_from_civil(day.0, day.1, day.2) + n;
    let result = civil_from_days(days);
    if !(1..=9999).contains(&result.0) {
        return Err(overflow());
    }
    Ok(result)
}
/// `_add_months`: the first day of the month `months` away.
fn add_months(day: Date, months: i64) -> std::result::Result<Date, String> {
    let index = day.0 * 12 + day.1 - 1 + months;
    let year = index.div_euclid(12);
    if !(1..=9999).contains(&year) {
        return Err(format!("year {year} is out of range"));
    }
    Ok((year, index.rem_euclid(12) + 1, 1))
}
fn iso_date(day: Date) -> String {
    format!("{:04}-{:02}-{:02}", day.0, day.1, day.2)
}
/// `strftime("%Y-%m-%dT%H:%M:%SZ")` in UTC; glibc does not pad `%Y`.
fn iso_instant(seconds: i64) -> String {
    let (y, m, d) = civil_from_days(seconds.div_euclid(DAY));
    let s = seconds.rem_euclid(DAY);
    format!(
        "{y}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

impl Range {
    /// `ExportRange.label`; CPython raises for a month past year 9999.
    pub fn label(&self) -> std::result::Result<String, String> {
        let (first, last) = (self.first, self.last);
        if first == (first.0, 1, 1) && last == (first.0, 12, 31) {
            return Ok(first.0.to_string());
        }
        let month_end = |months| -> std::result::Result<Date, String> {
            let next = add_months(first, months)?;
            add_days(next, -1).map_err(|e| e.message)
        };
        if first.2 == 1 && month_end(1)? == last {
            return Ok(format!("{}-{:02}", first.0, first.1));
        }
        if first.2 == 1 && first.1 % 3 == 1 && month_end(3)? == last {
            return Ok(format!("{}-Q{}", first.0, (first.1 - 1) / 3 + 1));
        }
        if first == last {
            return Ok(iso_date(first));
        }
        Ok(format!("{}_{}", iso_date(first), iso_date(last)))
    }
    /// `ExportRange.payload()`.
    pub fn payload(&self) -> Result<Value> {
        let label = self
            .label()
            .map_err(|message| AppError::new("internal_error", message))?;
        Ok(dict([
            ("preset", s(self.preset.as_str())),
            ("timezone", s(self.timezone.as_str())),
            ("start", s(iso_instant(self.start))),
            ("end", s(iso_instant(self.end))),
            ("first_day", s(iso_date(self.first))),
            ("last_day", s(iso_date(self.last))),
            ("label", s(label)),
        ]))
    }
}

/// `str(value)` of an argument the transports already typed.
fn argument(v: &Value) -> String {
    py_str(v)
}

/// `_parse_day`: only the documented `YYYY-MM-DD` calendar form.
fn day(value: &Value, name: &str) -> Result<Date> {
    let text = argument(value);
    let b = text.as_bytes();
    let parsed = (b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()))
    .then(|| {
        let n = |range: std::ops::Range<usize>| text[range].parse::<i64>().unwrap();
        (n(0..4), n(5..7), n(8..10))
    })
    .filter(|&(y, m, d)| y >= 1 && (1..=12).contains(&m) && d >= 1 && d <= month_length(y, m));
    parsed.ok_or_else(|| validation(format!("{name} must be a date like 2025-01-31")))
}
fn month_length(year: i64, month: i64) -> i64 {
    days_from_civil(year + i64::from(month == 12), month % 12 + 1, 1)
        - days_from_civil(year, month, 1)
}

/// The zone a request names, or the book calendar.
fn zone(profile: &Value, timezone: &Value, tzpath: &[PathBuf]) -> Result<Zone> {
    if !truthy(timezone) {
        // `_period_for_profile`: lower-cased, deliberately not stripped.
        let country = value(profile, "tax_country");
        let country = if truthy(country) {
            py_str(country)
        } else {
            String::new()
        };
        return book_zone(
            crate::country::zone(&kassiber_pytext::lower(&country)),
            tzpath,
        );
    }
    let key = argument(timezone);
    crate::zone::load(&key, tzpath).map_err(|_| {
        validation(format!("Unknown timezone '{key}'"))
            .with_hint("Use an IANA timezone name such as Europe/Vienna or UTC.")
    })
}
/// The book calendar (`crate::country::zone`: `None` is UTC). Without
/// tzdata, Vienna reads its pinned table and every other registry zone its
/// rule from `kassiber_tax_countries::calendar` (the EU's summer time, or a
/// fixed offset).
pub fn book_zone(zone: Option<&str>, tzpath: &[PathBuf]) -> Result<Zone> {
    let key = zone.unwrap_or("UTC");
    if let Ok(zone) = crate::zone::load(key, tzpath) {
        return Ok(zone);
    }
    if zone.is_none() {
        return Ok(Zone::utc(key));
    }
    if key != kassiber_tax_countries::VIENNA {
        return match kassiber_tax_countries::calendar::zone(key) {
            // The EU rule changes at 01:00 UTC, which is 02:00 local in
            // Berlin but 01:00 in London and Dublin.
            Some(rule) if rule.eu_summer_time() => {
                Ok(Zone::eu(key, i64::from(rule.standard_offset_s())))
            }
            // A zone without summer time (Tokyo) keeps its standard offset.
            Some(rule) => Ok(Zone::pinned(
                key,
                i64::from(rule.standard_offset_s()),
                &[],
                None,
            )),
            _ => Err(AppError::new(
                "internal_error",
                format!("No calendar for {key} without tzdata"),
            )),
        };
    }
    let Value::List(table) = value(austrian::metadata(), "vienna_transitions") else {
        unreachable!()
    };
    let epoch = kassiber_rates::time::UNIX / 1_000_000;
    let pairs: Vec<(i64, i64)> = table
        .iter()
        .map(|pair| {
            let pair = artifact_csv::list(pair);
            (int_value(&pair[0]) / 1_000_000 - epoch, int_value(&pair[1]))
        })
        .collect();
    Ok(Zone::pinned(key, 3921, &pairs, Some((3600, 7200))))
}
fn int_value(v: &Value) -> i64 {
    match v {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or_default(),
        _ => 0,
    }
}

fn preset_days(preset: &str, today: Date) -> Result<(Date, Date)> {
    Ok(match preset {
        "today" => (today, today),
        "yesterday" => {
            let day = add_days(today, -1)?;
            (day, day)
        }
        "this_week" | "last_week" => {
            let weekday = (days_from_civil(today.0, today.1, today.2) + 3).rem_euclid(7);
            let mut monday = add_days(today, -weekday)?;
            if preset == "last_week" {
                monday = add_days(monday, -7)?;
            }
            (monday, add_days(monday, 6)?)
        }
        "this_month" | "last_month" => {
            let first = (today.0, today.1, 1);
            let first = if preset == "last_month" {
                add_months(first, -1).map_err(|e| AppError::new("internal_error", e))?
            } else {
                first
            };
            let next = add_months(first, 1).map_err(|e| AppError::new("internal_error", e))?;
            (first, add_days(next, -1)?)
        }
        "this_quarter" | "last_quarter" => {
            let first = (today.0, (today.1 - 1) / 3 * 3 + 1, 1);
            let first = if preset == "last_quarter" {
                add_months(first, -3).map_err(|e| AppError::new("internal_error", e))?
            } else {
                first
            };
            let next = add_months(first, 3).map_err(|e| AppError::new("internal_error", e))?;
            (first, add_days(next, -1)?)
        }
        _ => {
            let year = today.0 - i64::from(preset == "last_year");
            ((year, 1, 1), (year, 12, 31))
        }
    })
}

/// The UTC instant of a local midnight (fold 0), within CPython's range.
fn midnight(zone: &Zone, day: Date) -> Result<i64> {
    let local = days_from_civil(day.0, day.1, day.2) * DAY;
    let offset = zone
        .local_offset(local)
        .ok_or_else(|| AppError::new("internal_error", "zone has no offset"))?;
    let utc = local - offset;
    if !(MIN..=MAX).contains(&utc) {
        return Err(overflow());
    }
    Ok(utc)
}

/// `resolve_export_range`; `now` is UTC seconds since 1970.
pub fn resolve(
    profile: &Value,
    preset: &Value,
    first_day: &Value,
    last_day: &Value,
    timezone: &Value,
    now: i64,
    tzpath: &[PathBuf],
) -> Result<Option<Range>> {
    let preset = if truthy(preset) {
        kassiber_pytext::lower(kassiber_pytext::strip(&py_str(preset)))
    } else {
        String::new()
    };
    let days = !matches!(first_day, Value::Null) || !matches!(last_day, Value::Null);
    if (preset.is_empty() || preset == "all") && !days {
        return Ok(None);
    }
    if !preset.is_empty() && preset != "custom" && days {
        return Err(validation(format!(
            "Range '{preset}' does not take a first or last day"
        ))
        .with_hint("Send either a preset or custom from/to days, not both."));
    }
    let zone = zone(profile, timezone, tzpath)?;
    let (preset, first, last) = if preset == "custom" || (preset.is_empty() && days) {
        if matches!(first_day, Value::Null) || matches!(last_day, Value::Null) {
            return Err(validation(
                "A custom range needs both a first and a last day".into(),
            ));
        }
        let (first, last) = (day(first_day, "from")?, day(last_day, "to")?);
        if last < first {
            return Err(validation("The range ends before it starts".into()));
        }
        ("custom".to_owned(), first, last)
    } else if PRESETS.contains(&preset.as_str()) {
        let offset = zone
            .utc_offset(now)
            .ok_or_else(|| AppError::new("internal_error", "zone has no offset"))?;
        let today = civil_from_days((now + offset).div_euclid(DAY));
        let (first, last) = preset_days(&preset, today)?;
        (preset, first, last)
    } else {
        return Err(validation(format!("Unknown range '{preset}'"))
            .with_hint(format!("Use one of: all, custom, {}.", PRESETS.join(", "))));
    };
    let start = midnight(&zone, first)?;
    let end = midnight(&zone, add_days(last, 1)?)?;
    Ok(Some(Range {
        preset,
        timezone: zone.key.clone(),
        start,
        end,
        first,
        last,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixtures() -> Vec<PathBuf> {
        vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/zoneinfo")]
    }
    fn data() -> Value {
        kassiber_pytext::loads(include_str!("../tests/data/ranges.json")).unwrap()
    }
    fn runs(v: &Value) -> Vec<i64> {
        artifact_csv::list(v)
            .iter()
            .flat_map(|run| {
                let run = artifact_csv::list(run);
                std::iter::repeat_n(int_value(&run[0]), int_value(&run[1]) as usize)
            })
            .collect()
    }

    /// CPython's C zoneinfo offsets for wall-clock midnights and UTC instants.
    #[test]
    fn zone_offsets_match_cpython() {
        let data = data();
        let Value::Dict(zones) = value(&data, "zones") else {
            panic!()
        };
        assert_eq!(zones.len(), 25);
        for (key, expected) in zones {
            let kassiber_pyjson::Key::Str(key) = key else {
                panic!()
            };
            let key = key.as_str().unwrap();
            let zone = crate::zone::load(key, &fixtures()).unwrap();
            for year in artifact_csv::list(value(&data, "years")) {
                let year = int_value(year);
                let name = year.to_string();
                let local = runs(value(value(expected, "local"), &name));
                let utc = runs(value(value(expected, "utc"), &name));
                let first = days_from_civil(year, 1, 1);
                for (i, offset) in local.iter().enumerate() {
                    let day = first + i as i64;
                    assert_eq!(
                        zone.local_offset(day * DAY),
                        Some(*offset),
                        "{key} local {day}"
                    );
                }
                for (i, offset) in utc.iter().enumerate() {
                    let day = first + (i / 3) as i64;
                    let hour = [0, 1, 23][i % 3];
                    let at = day * DAY + hour * 3600 + 1800;
                    assert_eq!(zone.utc_offset(at), Some(*offset), "{key} utc {at}");
                }
            }
        }
    }

    /// The pinned Vienna table matches the tzdata file it was frozen from.
    #[test]
    fn pinned_vienna_matches_tzdata() {
        let file = crate::zone::load("Europe/Vienna", &fixtures()).unwrap();
        let pinned = book_zone(Some("Europe/Vienna"), &[]).unwrap();
        for day in (days_from_civil(1890, 1, 1)..days_from_civil(2100, 1, 1)).step_by(3) {
            for second in [0, 3600, 7200, 82_800] {
                let at = day * DAY + second;
                assert_eq!(pinned.local_offset(at), file.local_offset(at), "local {at}");
                assert_eq!(pinned.utc_offset(at), file.utc_offset(at), "utc {at}");
            }
        }
        assert_eq!(book_zone(None, &[]).unwrap(), Zone::utc("UTC"));
    }

    /// Without tzdata, Berlin, Zurich, Dublin, London and Madrid follow the
    /// EU rule and Tokyo its fixed offset, which match tzdata from 1996
    /// (where the registry's calendars start).
    #[test]
    fn rule_zones_match_tzdata_from_1996() {
        for key in [
            "Europe/Berlin",
            "Europe/Zurich",
            "Europe/Dublin",
            "Europe/London",
            "Europe/Madrid",
            "Asia/Tokyo",
        ] {
            let file = crate::zone::load(key, &fixtures()).unwrap();
            let rule = book_zone(Some(key), &[]).unwrap();
            for day in (days_from_civil(1996, 1, 1)..days_from_civil(2100, 1, 1)).step_by(3) {
                for second in [0, 3600, 7200, 82_800] {
                    let at = day * DAY + second;
                    assert_eq!(
                        rule.local_offset(at),
                        file.local_offset(at),
                        "{key} local {at}"
                    );
                    assert_eq!(rule.utc_offset(at), file.utc_offset(at), "{key} utc {at}");
                }
            }
        }
        // A zone without a registry calendar still has none without tzdata.
        assert!(book_zone(Some("Asia/Seoul"), &[]).is_err());
    }

    /// Whether `zone` names a fixture file only because the file system
    /// ignores case (macOS APFS): `europe/vienna` opens `Europe/Vienna` there.
    fn resolves_only_case_insensitively(zone: &Value) -> bool {
        let Value::Str(zone) = zone else {
            return false;
        };
        let Some(zone) = zone.as_str() else {
            return false;
        };
        let mut directory = fixtures().remove(0);
        if !directory.join(zone).is_file() {
            return false;
        }
        for part in zone.split('/') {
            let names: Vec<_> = std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            if !names.iter().any(|name| name == part) {
                // `..`, `.` and empty parts are not listed: not a case question.
                return names.iter().any(|name| {
                    name.to_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(part))
                });
            }
            directory.push(part);
        }
        false
    }

    /// `resolve_export_range` results, payloads and refusals from CPython.
    /// The vectors come from a case-sensitive file system; where the zone
    /// lookup ignores case, CPython accepts those names too, so they are
    /// skipped there.
    #[test]
    fn export_range_vectors() {
        let data = data();
        let cases = artifact_csv::list(value(&data, "cases"));
        assert!(cases.len() > 700);
        let case_insensitive = fixtures()[0].join("EUROPE/VIENNA").is_file();
        let mut skipped = 0;
        for case in cases {
            let name = text(case, "name");
            if resolves_only_case_insensitively(value(case, "timezone")) {
                skipped += 1;
                continue;
            }
            let profile = dict([("tax_country", value(case, "country").clone())]);
            let now = (kassiber_rates::time::parse(&text(case, "now")).unwrap()
                - kassiber_rates::time::UNIX)
                / 1_000_000;
            let result = resolve(
                &profile,
                value(case, "preset"),
                value(case, "first"),
                value(case, "last"),
                value(case, "timezone"),
                now,
                &fixtures(),
            );
            let expected = value(case, "expected");
            match result {
                Err(error) => {
                    let e = value(expected, "error");
                    assert_eq!(
                        (
                            error.code.as_str(),
                            error.message.as_str(),
                            error.hint.as_deref().map(str::to_owned)
                        ),
                        (
                            text(e, "code").as_str(),
                            text(e, "message").as_str(),
                            match value(e, "hint") {
                                Value::Null => None,
                                hint => Some(py_str(hint)),
                            }
                        ),
                        "{name}"
                    );
                }
                Ok(None) => assert!(truthy(value(expected, "none")), "{name}"),
                Ok(Some(range)) => {
                    let ok = value(expected, "ok");
                    assert_eq!(
                        (range.start, range.end),
                        (int_value(value(ok, "start")), int_value(value(ok, "end"))),
                        "{name}"
                    );
                    match range.payload() {
                        Ok(payload) => assert_eq!(&payload, value(ok, "payload"), "{name}"),
                        Err(error) => {
                            assert_eq!(error.message, text(ok, "payload_error"), "{name}")
                        }
                    }
                }
            }
        }
        // Only a case-insensitive file system skips, and only the case variants.
        assert_eq!(skipped > 0, case_insensitive, "{skipped} skipped");
    }

    /// A tzdata directory is not a zone; CPython leaks an internal path there.
    #[test]
    fn zone_directories_are_unknown() {
        let error = resolve(
            &dict([("tax_country", s("generic"))]),
            &Value::Null,
            &s("2024-01-01"),
            &s("2024-01-02"),
            &s("Europe"),
            0,
            &fixtures(),
        )
        .unwrap_err();
        assert_eq!(error.message, "Unknown timezone 'Europe'");
    }
}
