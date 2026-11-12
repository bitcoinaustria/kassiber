//! Journal reads the country layers share: a stored instant, an exact
//! amount, and the book's quarantines with their transaction kinds.

use super::*;

/// µs from 0001-01-01 to 1970-01-01.
const UNIX_EPOCH_ORDINAL_US: i64 = 719_162 * 86_400_000_000;

/// A stored timestamp as µs since the epoch; a value without an offset is
/// read as UTC, as the journal writes them.
pub fn instant(raw: &str) -> Result<i64> {
    kassiber_pytext::iso_datetime(&raw.replace('Z', "+00:00"))
        .map(|dt| dt.utc_us() - UNIX_EPOCH_ORDINAL_US)
        .ok_or_else(|| {
            AppError::new(
                "internal_error",
                format!(
                    "Invalid isoformat string: {}",
                    kassiber_pytext::repr_str(raw)
                ),
            )
        })
}

/// An entry's exact amount (`<key>_exact`), else its float column.
pub fn exact(row: &Value, key: &str) -> Result<Decimal> {
    let exact = value(row, &format!("{key}_exact"));
    if truthy(exact) {
        decimal(exact)
    } else {
        decimal(value(row, key))
    }
}

/// One quarantined transaction.
pub struct Held {
    pub transaction_id: String,
    pub reason: String,
    /// The normalized kind.
    pub kind: String,
    /// The country's code a refused receipt carries in its detail.
    pub code: Option<String>,
}

/// The book's quarantined transactions, in time order. A `wallet` id limits
/// them to that wallet.
pub fn quarantines(conn: &Connection, profile_id: &str, wallet: Option<&str>) -> Result<Vec<Held>> {
    let found = rows(
        conn,
        "SELECT jq.transaction_id,jq.reason,jq.detail_json,COALESCE(NULLIF(t.kind_override,''),t.kind,'') AS kind \
         FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id \
         WHERE jq.profile_id=?1 AND (?2 IS NULL OR t.wallet_id=?2) \
         ORDER BY t.occurred_at,jq.transaction_id",
        kassiber_store::rusqlite::params![profile_id, wallet],
    )?;
    Ok(found
        .iter()
        .map(|row| {
            let kind = kassiber_pytext::lower(kassiber_pytext::strip(&text(row, "kind")))
                .replace(['-', ' '], "_");
            let code = serde_json::from_str::<serde_json::Value>(&text(row, "detail_json"))
                .ok()
                .and_then(|detail| detail["code"].as_str().map(str::to_owned));
            Held {
                transaction_id: text(row, "transaction_id"),
                reason: text(row, "reason"),
                kind,
                code,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_read_offsets_and_utc() {
        assert_eq!(instant("1970-01-01T00:00:01Z").unwrap(), 1_000_000);
        assert_eq!(instant("1970-01-01T01:00:00+01:00").unwrap(), 0);
        assert!(instant("nope").is_err());
    }
}
