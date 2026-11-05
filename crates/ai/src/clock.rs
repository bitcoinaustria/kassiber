//! UTC timestamps the provider store and the discovery cache write, in
//! Python's shapes: `now_iso()` (seconds, `Z`) and `datetime.isoformat()`
//! with `Z`, whose fraction is six digits or absent when it is zero.
//!
//! Test-harness builds take fixed values from `KASSIBER_RUST_TEST_AI_TIME`
//! and `KASSIBER_RUST_TEST_AI_CHECKED_AT`, so differential drivers can
//! compare envelopes byte for byte.

use std::time::{SystemTime, UNIX_EPOCH};

/// `now_iso()`: `2026-11-05T12:00:00Z`.
pub fn now_iso() -> String {
    #[cfg(feature = "test-harness")]
    if let Ok(fixed) = std::env::var("KASSIBER_RUST_TEST_AI_TIME") {
        return fixed;
    }
    let (seconds, _) = now();
    format_utc(seconds, None)
}

/// `datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")`.
pub fn checked_at() -> String {
    #[cfg(feature = "test-harness")]
    if let Ok(fixed) = std::env::var("KASSIBER_RUST_TEST_AI_CHECKED_AT") {
        return fixed;
    }
    let (seconds, micros) = now();
    format_utc(seconds, Some(micros))
}

fn now() -> (i64, u32) {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (
        i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        elapsed.subsec_micros(),
    )
}

/// Seconds since the epoch as an ISO 8601 UTC time; a fraction only when
/// one is given and nonzero, as `isoformat()` writes it.
pub(crate) fn format_utc(seconds: i64, micros: Option<u32>) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let fraction = match micros {
        Some(micros) if micros != 0 => format!(".{micros:06}"),
        _ => String::new(),
    };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{fraction}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::format_utc;

    #[test]
    fn timestamps_match_python_isoformat() {
        assert_eq!(format_utc(0, None), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_793_448_000, None), "2026-10-31T12:00:00Z");
        assert_eq!(
            format_utc(951_782_400, Some(5)),
            "2000-02-29T00:00:00.000005Z"
        );
        // isoformat() drops a zero fraction.
        assert_eq!(format_utc(951_782_400, Some(0)), "2000-02-29T00:00:00Z");
        assert_eq!(
            format_utc(4_102_444_799, Some(999_999)),
            "2099-12-31T23:59:59.999999Z"
        );
    }
}
