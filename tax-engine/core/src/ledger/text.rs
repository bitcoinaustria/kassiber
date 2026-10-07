//! RP2's `str(transaction)` for an engine entry.
//!
//! RP2 embeds the text in some messages (a negative balance, a duplicate
//! entry, swap-marker errors), which the lot engine builds from
//! [`Entry::text`]. A v1 request carries the text, rendered in Python by
//! `native_request.entry_text`; the ledger layer renders it here instead,
//! after the constructor has derived the values, as `native.py` does.
//!
//! The format is `native_request.py:321-382`: the class name, then one
//! `name=value` line per field, joined with a newline and two spaces.
//! Decimals are `format(x, '.4f')` (fiat) or `'.8f'` (crypto) under RP2's
//! context, and the timestamp is `strftime("%Y-%m-%d %H:%M:%S.%f %z")` in the
//! entry's own offset.

use crate::decimal::Decimal;
use crate::entry::{Detail, Entry};
use crate::error::EngineResult;
use crate::num::CTX;
use crate::time::Timestamp;

const US_PER_SECOND: i128 = 1_000_000;
const US_PER_DAY: i128 = 86_400 * US_PER_SECOND;

/// Places of a fiat value in the text.
const FIAT_PLACES: u32 = 4;

/// Places of a crypto quantity in the text.
const CRYPTO_PLACES: u32 = 8;

/// `str(transaction)` for a constructed entry.
pub fn entry_text(entry: &Entry) -> EngineResult<String> {
    let stamp = timestamp_text(&entry.ts);
    let mut lines = vec![
        format!("{}:", entry.kind.class_name()),
        format!("id={}", entry.row),
        format!("timestamp={stamp}"),
        format!("asset={}", entry.asset),
    ];
    let transaction_type = format!("transaction_type={}", entry.transaction_type);
    let spot_price = format!("spot_price={}", fiat(&entry.spot_price)?);
    let unique_id = format!("unique_id={}", entry.unique_id);
    // `native_request.entry_text` reads `is_taxable` from the derived values,
    // where a comparison that raises is `None`.
    let is_taxable = format!(
        "is_taxable={}",
        match entry.is_taxable() {
            Ok(true) => "True",
            Ok(false) => "False",
            Err(_) => "None",
        }
    );
    let fiat_taxable_amount = format!(
        "fiat_taxable_amount={}",
        fiat(&entry.fiat_taxable_amount())?
    );
    match &entry.detail {
        Detail::In {
            exchange,
            holder,
            crypto_in,
            fiat_fee,
            fiat_in_no_fee,
            fiat_in_with_fee,
            ..
        } => lines.extend([
            format!("exchange={exchange}"),
            format!("holder={holder}"),
            transaction_type,
            spot_price,
            format!("crypto_in={}", crypto(crypto_in)?),
            format!("fiat_fee={}", fiat(fiat_fee)?),
            format!("fiat_in_no_fee={}", fiat(fiat_in_no_fee)?),
            format!("fiat_in_with_fee={}", fiat(fiat_in_with_fee)?),
            unique_id,
            is_taxable,
            fiat_taxable_amount,
            "from_lot=".to_owned(),
            "to_lots=".to_owned(),
            format!("cost_basis_timestamp={stamp}"),
        ]),
        Detail::Out {
            exchange,
            holder,
            crypto_out_no_fee,
            crypto_fee,
            ..
        } => lines.extend([
            format!("exchange={exchange}"),
            format!("holder={holder}"),
            transaction_type,
            spot_price,
            format!("crypto_out_no_fee={}", crypto(crypto_out_no_fee)?),
            format!("crypto_fee={}", crypto(crypto_fee)?),
            unique_id,
            is_taxable,
            fiat_taxable_amount,
        ]),
        Detail::Intra {
            from_exchange,
            from_holder,
            to_exchange,
            to_holder,
            crypto_sent,
            crypto_received,
            crypto_fee,
            fiat_fee,
        } => lines.extend([
            format!("from_exchange={from_exchange}"),
            format!("from_holder={from_holder}"),
            format!("to_exchange={to_exchange}"),
            format!("to_holder={to_holder}"),
            transaction_type,
            spot_price,
            format!("crypto_sent={}", crypto(crypto_sent)?),
            format!("crypto_received={}", crypto(crypto_received)?),
            format!("crypto_fee={}", crypto(crypto_fee)?),
            format!("fiat_fee={}", fiat(fiat_fee)?),
            unique_id,
            is_taxable,
            fiat_taxable_amount,
        ]),
    }
    Ok(lines.join("\n  "))
}

fn fiat(value: &Decimal) -> EngineResult<String> {
    Ok(value.to_fixed_places(FIAT_PLACES, CTX.rounding)?)
}

fn crypto(value: &Decimal) -> EngineResult<String> {
    Ok(value.to_fixed_places(CRYPTO_PLACES, CTX.rounding)?)
}

/// `strftime("%Y-%m-%d %H:%M:%S.%f %z")` of the timestamp in its own offset.
/// `%Y` is glibc's, which CPython uses on Linux: years before 1000 are not
/// padded (`999-06-01`).
fn timestamp_text(ts: &Timestamp) -> String {
    let local = i128::from(ts.us) + i128::from(ts.offset_s) * US_PER_SECOND;
    let time = local.rem_euclid(US_PER_DAY);
    let seconds = time / US_PER_SECOND;
    let date = ts.date;
    format!(
        "{}-{:02}-{:02} {:02}:{:02}:{:02}.{:06} {}",
        date.year,
        date.month,
        date.day,
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60,
        time % US_PER_SECOND,
        utc_offset_text(ts.offset_s)
    )
}

/// `strftime("%z")`: `+HHMM`, or `+HHMMSS` when the offset has seconds.
fn utc_offset_text(offset_s: i32) -> String {
    let sign = if offset_s < 0 { '-' } else { '+' };
    let total = offset_s.unsigned_abs();
    let (hours, minutes, seconds) = (total / 3_600, total / 60 % 60, total % 60);
    if seconds == 0 {
        format!("{sign}{hours:02}{minutes:02}")
    } else {
        format!("{sign}{hours:02}{minutes:02}{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::CivilDate;

    fn ts(us: i64, offset_s: i32, year: i32, month: u32, day: u32) -> Timestamp {
        Timestamp {
            raw: String::new(),
            us,
            offset_s,
            date: CivilDate { year, month, day },
            vienna_date: None,
            display: String::new(),
            iso: String::new(),
        }
    }

    #[test]
    fn timestamps_render_as_strftime_does() {
        // Expected values are CPython 3.11's `datetime.strftime` on Linux.
        let cases = [
            (ts(0, 0, 1970, 1, 1), "1970-01-01 00:00:00.000000 +0000"),
            (
                ts(-500_000, 0, 1969, 12, 31),
                "1969-12-31 23:59:59.500000 +0000",
            ),
            (
                ts(1_709_200_799_999_999, 50_400, 2024, 2, 29),
                "2024-02-29 23:59:59.999999 +1400",
            ),
            (
                ts(-30_628_648_503_210_988, -19_800, 999, 6, 1),
                "999-06-01 12:34:56.789012 -0530",
            ),
            (
                ts(1_614_556_860_000_001, -60, 2021, 3, 1),
                "2021-03-01 00:00:00.000001 -0001",
            ),
            // `datetime(2024, 1, 2, tzinfo=timezone(timedelta(seconds=19807)))`.
            (
                ts(1_704_133_793_000_000, 19_807, 2024, 1, 2),
                "2024-01-02 00:00:00.000000 +053007",
            ),
        ];
        for (stamp, expected) in cases {
            assert_eq!(timestamp_text(&stamp), expected, "{}", stamp.us);
        }
        assert_eq!(utc_offset_text(-19_807), "-053007");
        assert_eq!(utc_offset_text(-1), "-000001");
    }
}
