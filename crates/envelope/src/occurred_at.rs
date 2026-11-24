//! The stored placeholder for a transaction time nobody reported, and its
//! human wording.
//!
//! Sync stores [`UNKNOWN`] as `occurred_at` when a source reports no time:
//! an unconfirmed (mempool) transaction, an RBF-replaced one, or a Lightning
//! or import row without a timestamp. The stored value and machine JSON keep
//! it, since books, goldens and consumers depend on it (`confirmed_at: null`
//! already says a transaction is unconfirmed). Human output must not present
//! it as a date: [`label`] gives the wording, and [`humanize`] rewrites a
//! payload for the CLI's table and plain formats.

use kassiber_pyjson::{Key, Value};

/// The stored `occurred_at` (and Lightning `confirmed_at`) of a transaction
/// whose source reported no time.
pub const UNKNOWN: &str = "1970-01-01T00:00:00Z";

/// Shown instead of the placeholder when the transaction is not confirmed.
pub const UNCONFIRMED_LABEL: &str = "unconfirmed";
/// Shown instead of the placeholder when the source reported the
/// transaction as settled but gave no time.
pub const UNKNOWN_DATE_LABEL: &str = "unknown date";

/// Whether a stored timestamp is the placeholder rather than a real time.
pub fn is_unknown(value: &str) -> bool {
    value.trim() == UNKNOWN
}

/// The wording for a placeholder time, given the row's `confirmed_at`:
/// "unconfirmed" when it is null or empty, otherwise "unknown date". A set
/// `confirmed_at` may itself be the placeholder: LND stores it in both fields
/// for a settled payment without a timestamp.
pub fn label(confirmed_at: Option<&str>) -> &'static str {
    match confirmed_at.map(str::trim) {
        Some(at) if !at.is_empty() => UNKNOWN_DATE_LABEL,
        _ => UNCONFIRMED_LABEL,
    }
}

/// `value` when it is a real time, otherwise the placeholder's wording.
pub fn display<'a>(value: &'a str, confirmed_at: Option<&str>) -> &'a str {
    if is_unknown(value) {
        label(confirmed_at)
    } else {
        value
    }
}

/// The keys whose placeholder value [`humanize`] replaces.
fn is_time_key(key: &str) -> bool {
    key == "occurred_at" || key.ends_with("_occurred_at") || key == "confirmed_at"
}

/// Replace placeholder times in every dict of `value` (recursively) with
/// their wording. The wording follows the nearest `confirmed_at`: the dict's
/// own, else the closest enclosing dict's. A row with no `confirmed_at`
/// anywhere above it carries no confirmation signal, so its placeholder reads
/// "unknown date" rather than guessing "unconfirmed". Only for human formats:
/// JSON and CSV keep the stored value.
pub fn humanize(value: Value) -> Value {
    humanize_in(value, None)
}

/// `inherited` is `None` when no enclosing dict has a `confirmed_at` key,
/// and `Some(text)` (`Some(None)` for null or non-text) when one does.
fn humanize_in(value: Value, inherited: Option<Option<String>>) -> Value {
    match value {
        Value::List(items) => Value::List(
            items
                .into_iter()
                .map(|item| humanize_in(item, inherited.clone()))
                .collect(),
        ),
        Value::Dict(pairs) => {
            let own = pairs.iter().find_map(|(key, item)| match key {
                Key::Str(name) if name.as_str() == Some("confirmed_at") => Some(match item {
                    Value::Str(text) => text.as_str().map(str::to_owned),
                    _ => None,
                }),
                _ => None,
            });
            let confirmed = own.or(inherited);
            Value::Dict(
                pairs
                    .into_iter()
                    .map(|(key, item)| {
                        let time_key = matches!(&key, Key::Str(name)
                            if name.as_str().is_some_and(is_time_key));
                        let item = match item {
                            Value::Str(text)
                                if time_key && text.as_str().is_some_and(is_unknown) =>
                            {
                                let is_confirmed_key = matches!(&key, Key::Str(name)
                                    if name.as_str() == Some("confirmed_at"));
                                Value::from(match &confirmed {
                                    _ if is_confirmed_key => UNKNOWN_DATE_LABEL,
                                    Some(at) => label(at.as_deref()),
                                    None => UNKNOWN_DATE_LABEL,
                                })
                            }
                            other => humanize_in(other, confirmed.clone()),
                        };
                        (key, item)
                    })
                    .collect(),
            )
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_pytext::loads;

    #[test]
    fn placeholder_gets_wording_and_real_times_pass_through() {
        assert!(is_unknown(UNKNOWN));
        assert!(is_unknown(" 1970-01-01T00:00:00Z "));
        assert!(!is_unknown("1970-01-01T00:00:01Z"));
        assert!(!is_unknown(""));
        assert_eq!(label(None), UNCONFIRMED_LABEL);
        assert_eq!(label(Some("")), UNCONFIRMED_LABEL);
        assert_eq!(label(Some("2026-11-21T20:11:40Z")), UNKNOWN_DATE_LABEL);
        // LND stores the placeholder in both fields for an untimed payment.
        assert_eq!(label(Some(UNKNOWN)), UNKNOWN_DATE_LABEL);
        assert_eq!(display(UNKNOWN, None), "unconfirmed");
        assert_eq!(
            display("2026-11-21T20:11:40Z", None),
            "2026-11-21T20:11:40Z"
        );
    }

    #[test]
    fn humanize_rewrites_nested_rows_and_keeps_everything_else() {
        let payload = loads(
            r#"[{"occurred_at":"1970-01-01T00:00:00Z","confirmed_at":null,"amount":0.3,
                 "root":{"occurred_at":"1970-01-01T00:00:00Z","wallet":"w"},
                 "detail":{"barrier_event":["1970-01-01T00:00:00Z","bitcoin"]},
                 "note":"1970-01-01T00:00:00Z"},
                {"occurred_at":"1970-01-01T00:00:00Z","confirmed_at":"1970-01-01T00:00:00Z"},
                {"payout_occurred_at":"1970-01-01T00:00:00Z","confirmed_at":"2026-01-01T00:00:00Z"},
                {"occurred_at":"2026-11-21T20:11:40Z","confirmed_at":null},
                {"transaction_id":"t","occurred_at":"1970-01-01T00:00:00Z"}]"#,
        )
        .unwrap();
        let expected = loads(
            r#"[{"occurred_at":"unconfirmed","confirmed_at":null,"amount":0.3,
                 "root":{"occurred_at":"unconfirmed","wallet":"w"},
                 "detail":{"barrier_event":["1970-01-01T00:00:00Z","bitcoin"]},
                 "note":"1970-01-01T00:00:00Z"},
                {"occurred_at":"unknown date","confirmed_at":"unknown date"},
                {"payout_occurred_at":"unknown date","confirmed_at":"2026-01-01T00:00:00Z"},
                {"occurred_at":"2026-11-21T20:11:40Z","confirmed_at":null},
                {"transaction_id":"t","occurred_at":"unknown date"}]"#,
        )
        .unwrap();
        assert_eq!(humanize(payload), expected);
    }
}
