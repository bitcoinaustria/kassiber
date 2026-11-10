//! `release-notes` (scripts/release_notes.py) and `build-info`
//! (scripts/write_build_info.py).
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use kassiber_envelope::{dict, s};
use kassiber_pyjson::{Options, Value};
use kassiber_release::publish::py_path;

use crate::args::{parse, required, value, Spec, Usage};
use crate::usage;

/// `[0-9]+\.[0-9]+\.[0-9]+(?:[.-][A-Za-z0-9.-]+)?` over the whole text:
/// some split leaves three digit groups and an optional suffix.
fn notes_version(text: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    (0..=text.len())
        .filter(|i| text.is_char_boundary(*i))
        .any(|end| {
            let (core, suffix) = text.split_at(end);
            let parts: Vec<&str> = core.split('.').collect();
            parts.len() == 3
                && parts.iter().all(|p| digits(p))
                && (suffix.is_empty()
                    || (suffix.len() > 1
                        && matches!(suffix.as_bytes()[0], b'.' | b'-')
                        && suffix[1..]
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')))
        })
}

/// `extract_notes(changelog, version)`.
pub fn extract_notes(changelog: &str, version: &str) -> Result<String, String> {
    let version = version.strip_prefix('v').unwrap_or(version);
    if !notes_version(version) {
        return Err("Invalid release version".into());
    }
    // `^## ([^\n]+)\n` with re.MULTILINE: (heading start, body start, title).
    let mut headings = Vec::new();
    let mut offset = 0;
    for line in changelog.split_inclusive('\n') {
        if let Some(title) = line.strip_prefix("## ").and_then(|l| l.strip_suffix('\n')) {
            if !title.is_empty() {
                headings.push((offset, offset + line.len(), title));
            }
        }
        offset += line.len();
    }
    let matches: Vec<usize> = headings
        .iter()
        .enumerate()
        .filter(|(_, (_, _, title))| *title == version)
        .map(|(i, _)| i)
        .collect();
    if matches.len() != 1 {
        return Err(format!(
            "Require exactly one changelog section for {version}"
        ));
    }
    let index = matches[0];
    let end = headings
        .get(index + 1)
        .map_or(changelog.len(), |(start, _, _)| *start);
    let notes = changelog[headings[index].1..end].trim_matches('\n');
    if kassiber_pytext::strip(notes).is_empty() {
        return Err(format!("Empty changelog section for {version}"));
    }
    Ok(format!("{notes}\n"))
}

/// Python's `str(OSError)`: `[Errno N] message: 'path'`.
fn os_error(error: &std::io::Error, path: &Path) -> String {
    let text = error.to_string();
    let message = text.split(" (os error").next().unwrap_or(&text);
    match error.raw_os_error() {
        Some(code) => format!("[Errno {code}] {message}: '{}'", path.display()),
        None => message.to_owned(),
    }
}

pub fn release_notes(argv: &[String]) -> Result<ExitCode, Usage> {
    let specs: [Spec; 3] = [required("version"), value("changelog"), required("output")];
    let args = parse(argv, &specs)?;
    let changelog = py_path(Path::new(args.get("changelog").unwrap_or("CHANGELOG.md")));
    let notes = std::fs::read(&changelog)
        .map_err(|error| os_error(&error, &changelog))
        .and_then(|bytes| {
            String::from_utf8(bytes).map_err(|_| "'utf-8' codec can't decode the changelog".into())
        })
        // `read_text()` reads with universal newlines.
        .map(|text| text.replace("\r\n", "\n").replace('\r', "\n"))
        .and_then(|text| extract_notes(&text, args.req("version")));
    match notes {
        Ok(notes) => {
            let output = py_path(Path::new(args.req("output")));
            if let Err(error) = std::fs::write(&output, notes) {
                eprintln!("Release notes: {}", os_error(&error, &output));
                return Ok(ExitCode::from(1));
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(message) => {
            eprintln!("Release notes: {message}");
            Ok(ExitCode::from(1))
        }
    }
}

/// `int(text)` for a Unix timestamp: surrounding whitespace, a sign, and
/// single underscores between digits.
fn python_int(text: &str) -> Option<i64> {
    let text = kassiber_pytext::strip(text);
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    let value: i64 = digits.replace('_', "").parse().ok()?;
    Some(if negative { -value } else { value })
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `datetime.fromtimestamp(seconds, utc).isoformat().replace("+00:00", "Z")`.
pub fn iso(seconds: i64, micros: u32) -> String {
    let (year, month, day) = civil(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    let mut text = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    );
    if micros != 0 {
        text.push_str(&format!(".{micros:06}"));
    }
    text.push('Z');
    text
}

/// `resolve_built_at(explicit)`.
pub fn built_at(explicit: Option<&str>, epoch: Option<&str>) -> Result<String, String> {
    if let Some(explicit) = explicit.filter(|e| !e.is_empty()) {
        return Ok(explicit.to_owned());
    }
    if let Some(epoch) = epoch {
        let seconds =
            python_int(epoch).ok_or("SOURCE_DATE_EPOCH must be an integer Unix timestamp")?;
        if seconds < 0 {
            return Err("SOURCE_DATE_EPOCH must not be negative".into());
        }
        return Ok(iso(seconds, 0));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(iso(now.as_secs() as i64, now.subsec_micros()))
}

pub fn build_info(argv: &[String]) -> Result<ExitCode, Usage> {
    let mut channel = required("channel");
    channel.choices = &["dev", "prerelease", "release"];
    let specs = [
        required("output"),
        required("version"),
        required("commit"),
        required("ref"),
        value("run-id"),
        channel,
        value("built-at"),
    ];
    let args = parse(argv, &specs)?;
    let epoch = std::env::var("SOURCE_DATE_EPOCH").ok();
    let built_at = built_at(args.get("built-at"), epoch.as_deref()).map_err(usage)?;
    let payload = dict([
        ("schema_version", Value::Int(1.into())),
        ("version", s(args.req("version"))),
        ("channel", s(args.req("channel"))),
        ("commit", s(args.req("commit"))),
        ("ref", s(args.req("ref"))),
        ("run_id", s(args.get("run-id").unwrap_or("local"))),
        ("built_at", s(built_at)),
    ]);
    let text = kassiber_pyjson::dumps(&payload, &Options::new().indent(2).sort_keys(true)).unwrap();
    let output = py_path(Path::new(args.req("output")));
    let written = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&output, format!("{text}\n")));
    if let Err(error) = written {
        eprintln!("kassiber-release: error: {}", os_error(&error, &output));
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_and_versions() {
        assert_eq!(built_at(None, Some("0")).unwrap(), "1970-01-01T00:00:00Z");
        assert_eq!(
            built_at(Some("2026-01-02T03:04:05Z"), Some("0")).unwrap(),
            "2026-01-02T03:04:05Z"
        );
        assert!(built_at(None, Some("not-a-timestamp"))
            .unwrap_err()
            .contains("integer Unix timestamp"));
        assert!(built_at(None, Some("-1")).unwrap_err().contains("negative"));
        assert_eq!(
            built_at(None, Some(" 1_700_000_000 ")).unwrap(),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(iso(951_782_400, 5), "2000-02-29T00:00:00.000005Z");
        for good in ["1.2.3", "1.2.3-rc.1", "10.20.30.post1"] {
            assert!(notes_version(good), "{good}");
        }
        for bad in ["1.2", "1.2.3-", "a.b.c", "1.2.3_x"] {
            assert!(!notes_version(bad), "{bad}");
        }
    }

    #[test]
    fn selects_exact_version_preserving_body() {
        let text = "# Changelog\n\n## 1.2.30\n\nOther\n\n## 1.2.3\n\n### Fixed\n\n- A & B.\n\n## 1.2.2\n\nOlder\n";
        assert_eq!(
            extract_notes(text, "v1.2.3").unwrap(),
            "### Fixed\n\n- A & B.\n"
        );
        for text in [
            "# Changelog\n",
            "## 1.2.3\n\n",
            "## 1.2.3\nA\n## 1.2.3\nB\n",
        ] {
            assert!(extract_notes(text, "1.2.3").is_err(), "{text:?}");
        }
    }
}
