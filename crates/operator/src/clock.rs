//! The timestamps and random tokens the broker hands out
//! (`time_utils.now_iso`, `secrets.token_urlsafe`).

use crate::error::{Error, Result};
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

/// The last second `datetime` can represent: 9999-12-31T23:59:59.
pub const MAX_UNIX_SECONDS: i64 = 253_402_300_799;

/// Seconds since the epoch, truncated (`.replace(microsecond=0)`).
pub fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// `now_iso()`: the current UTC second as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_iso() -> String {
    format_unix_seconds(unix_seconds())
}

/// `datetime.fromtimestamp(seconds, timezone.utc).isoformat()` with `Z`,
/// for seconds within `datetime`'s range.
pub fn format_unix_seconds(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60
    )
}

/// Howard Hinnant's `civil_from_days`: the proleptic Gregorian date of a day
/// count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `secrets.token_urlsafe(bytes)`: unpadded URL-safe base64 of fresh
/// entropy.
pub fn token_urlsafe(bytes: usize) -> Result<String> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut raw = Zeroizing::new(vec![0u8; bytes]);
    getrandom::fill(&mut raw)
        .map_err(|_| Error::Io(std::io::Error::other("entropy unavailable")))?;
    let mut out = String::with_capacity(bytes.div_ceil(3) * 4);
    for chunk in raw.chunks(3) {
        let block = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..=chunk.len() {
            out.push(ALPHABET[(block >> (18 - 6 * index) & 63) as usize] as char);
        }
    }
    Ok(out)
}

/// The lowercase hex digest of `sha256(text)`.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_match_python_isoformat() {
        assert_eq!(format_unix_seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_seconds(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_unix_seconds(1_767_225_599), "2025-12-31T23:59:59Z");
        assert_eq!(
            format_unix_seconds(MAX_UNIX_SECONDS),
            "9999-12-31T23:59:59Z"
        );
    }

    #[test]
    fn url_safe_tokens_have_python_lengths() {
        // token_urlsafe(n) is ceil(4n/3) characters without padding.
        for (bytes, length) in [(12, 16), (24, 32), (32, 43), (1, 2), (2, 3)] {
            let token = token_urlsafe(bytes).unwrap();
            assert_eq!(token.len(), length);
            assert!(token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        }
    }
}
