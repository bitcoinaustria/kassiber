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

/// The wording for a placeholder time: "unconfirmed" unless the row says it
/// was confirmed (a real `confirmed_at`), then "unknown date".
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
/// their wording, using the sibling `confirmed_at` to choose it. Only for
/// human formats: JSON and CSV keep the stored value.
pub fn humanize(value: Value) -> Value {
    match value {
        Value::List(items) => Value::List(items.into_iter().map(humanize).collect()),
        Value::Dict(pairs) => {
            let confirmed = pairs.iter().find_map(|(key, item)| match (key, item) {
                (Key::Str(name), Value::Str(text)) if name.as_str() == Some("confirmed_at") => {
                    text.as_str().map(str::to_owned)
                }
                _ => None,
            });
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
                                Value::from(if is_confirmed_key {
                                    UNKNOWN_DATE_LABEL
                                } else {
                                    label(confirmed.as_deref())
                                })
                            }
                            other => humanize(other),
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
                {"occurred_at":"2026-11-21T20:11:40Z","confirmed_at":null}]"#,
        )
        .unwrap();
        let expected = loads(
            r#"[{"occurred_at":"unconfirmed","confirmed_at":null,"amount":0.3,
                 "root":{"occurred_at":"unconfirmed","wallet":"w"},
                 "detail":{"barrier_event":["1970-01-01T00:00:00Z","bitcoin"]},
                 "note":"1970-01-01T00:00:00Z"},
                {"occurred_at":"unknown date","confirmed_at":"unknown date"},
                {"payout_occurred_at":"unknown date","confirmed_at":"2026-01-01T00:00:00Z"},
                {"occurred_at":"2026-11-21T20:11:40Z","confirmed_at":null}]"#,
        )
        .unwrap();
        assert_eq!(humanize(payload), expected);
    }
}
