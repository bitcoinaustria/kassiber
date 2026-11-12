#[allow(dead_code)]
mod support;
use kassiber_journals::prebuild::{auto_price_transactions_from_rates_cache, PricingScope};
use kassiber_pydecimal::{Context, Rounding};
use kassiber_store::rusqlite::Connection;
use serde_json::Value;

fn check(names: &[&str]) {
    let vectors: Value = serde_json::from_str(include_str!("data/prebuild-pricing.json")).unwrap();
    let mut selected = 0;
    for case in vectors["cases"].as_array().unwrap() {
        if !names.is_empty() && !names.contains(&case["name"].as_str().unwrap()) {
            continue;
        }
        selected += 1;
        let expand = |side: &str| {
            let mut dump = vectors["base"].clone();
            for (table, rows) in case[side].as_object().unwrap() {
                dump[table]["rows"] = rows.clone();
            }
            dump
        };
        let before = expand("before");
        let expected = expand("expected");
        let conn = Connection::open_in_memory().unwrap();
        support::load(&conn, &before);
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        conn.execute(
            "INSERT INTO settings(key,value) VALUES('caller-write','retained')",
            [],
        )
        .unwrap();
        let result = auto_price_transactions_from_rates_cache(
            &conn,
            &PricingScope {
                profile_id: "p",
                fiat_currency: "EUR",
                decimal_context: Context::new(32, Rounding::HalfEven),
            },
        );
        if case["error"].is_null() {
            assert_eq!(
                result.unwrap(),
                case["count"].as_u64().unwrap() as usize,
                "{}",
                case["name"]
            );
        } else {
            assert_eq!(
                serde_json::to_value(result.unwrap_err()).unwrap(),
                case["error"],
                "{}",
                case["name"]
            );
        }
        assert_eq!(
            conn.query_row(
                "SELECT value FROM settings WHERE key='caller-write'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "retained"
        );
        conn.execute("DELETE FROM settings WHERE key='caller-write'", [])
            .unwrap();
        assert!(
            support::logical_equal(&support::snapshot(&conn), &expected),
            "{}",
            case["name"]
        );
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
        assert!(
            support::logical_equal(&support::snapshot(&conn), &before),
            "{}",
            case["name"]
        );
    }
    assert!(selected > 0);
    if !names.is_empty() {
        assert_eq!(selected, names.len());
    }
}

#[test]
fn prebuild_pricing_full_python_dumps_and_refusals() {
    check(&[]);
}

#[test]
fn prebuild_pricing_scopes_and_skips() {
    check(&[
        "other-profile",
        "excluded",
        "unsupported-asset",
        "unsupported-fiat",
        "cache-miss",
        "live-enabled-cache-miss",
        "lbtc",
        "l-btc-unsupported",
    ]);
}

#[test]
fn prebuild_pricing_timestamp_selection() {
    check(&[
        "confirmed-price",
        "offset-time",
        "fractional-time",
        "rate-offset",
        "invalid-stored-rate-date",
        "invalid-date",
        "date-overflow",
    ]);
}

#[test]
fn prebuild_pricing_precedence_and_precision() {
    check(&[
        "legacy-exact-rate",
        "legacy-exact-value",
        "small-quantity",
        "large-quantity",
        "rate-tiny",
        "unicode-decimal",
        "signed-zero",
        "negative-price",
        "zero-price",
        "empty-exact",
    ]);
}

#[test]
fn prebuild_pricing_provenance_and_count() {
    check(&[
        "legacy-enrichment",
        "legacy-provenance-kept",
        "legacy-quality-kept",
        "manual-cache",
        "coarse-cache",
        "unknown-cache",
        "clear-old-reference",
        "zero-amount",
        "negative-amount",
    ]);
}

#[test]
fn prebuild_pricing_refusal_rolls_back_earlier_writes() {
    check(&[
        "invalid-exact",
        "invalid-cache",
        "signaling-nan",
        "rate-overflow",
    ]);
}

#[test]
fn prebuild_pricing_extreme_decimal_refusals() {
    check(&[
        "rate-context-overflow",
        "rate-extreme-overflow",
        "rate-construction-overflow",
        "rate-subnormal-underflow",
        "rate-large-coefficient",
        "rate-large-nan-payload",
    ]);
}

#[test]
fn prebuild_pricing_legacy_quantity_storage_and_lazy_refusals() {
    check(&[
        "legacy-real-amount",
        "legacy-negative-real-amount",
        "legacy-wide-real-amount",
        "legacy-text-amount",
        "legacy-text-amount-cache-miss",
        "legacy-text-amount-unsupported",
        "legacy-amount-skipped-for-existing-value",
        "value-derived-rate",
        "legacy-text-amount-value-derived-rate",
        "empty-rate-and-value",
    ]);
}

#[test]
fn prebuild_pricing_binary_numeric_refusals_and_special_values() {
    check(&[
        "binary-exact-cache",
        "binary-empty-cache",
        "binary-single-quote-cache",
        "binary-all-bytes-cache",
        "binary-numeric-cache",
        "binary-exact-rate",
        "binary-exact-value",
        "binary-numeric-rate",
        "binary-numeric-value",
        "binary-quantity",
        "binary-cache-ignored",
        "positive-infinity",
        "negative-infinity",
        "signaling-nan-without-quantity",
        "positive-infinite-real-cache",
        "negative-infinite-real-cache",
        "infinite-real-quantity",
    ]);
}

#[test]
fn prebuild_pricing_special_division_and_date_witnesses() {
    check(&[
        "value-derived-quiet-nan",
        "value-derived-signaling-nan",
        "value-derived-positive-infinity",
        "value-derived-negative-infinity",
        "invalid-date-separator",
        "invalid-century-leap-day",
        "invalid-four-century-leap-day",
        "invalid-four-century-leap-hour",
        "invalid-month-short-time",
        "invalid-basic-hour",
        "value-derived-overflow",
    ]);
}

#[test]
fn prebuild_pricing_round_5_price_bounds_mark_unsupported_rows() {
    check(&[
        "rate-tax-limit",
        "rate-mirror-limit",
        "rate-integer-mirror-limit",
        "derived-value-limit",
        "derived-value-tiny",
        "derived-rate-limit",
        "value-limit",
        "stored-rate-limit",
        "unsupported-then-priced",
        "stored-real-rate-yields-to-cache",
        "bounded-rate-before-invalid-value",
        "implied-rate-limit",
        "implied-value-limit",
        "rate-tiny",
        "quiet-nan",
        "positive-infinity",
        "value-derived-quiet-nan",
    ]);
}

/// Group B's edge cases of the same bounds: a limit rate or float mirror on
/// a one-msat amount (the rate bound itself, not its derived value), an
/// out-of-range value on a zero amount, the derived floors, canonical zero
/// and a negative amount's absolute valuation.
#[test]
fn prebuild_pricing_bounds_apart_from_derived_valuations() {
    check(&[
        "rate-limit-small-amount",
        "rate-float-mirror-small-amount",
        "derived-value-floor",
        "value-limit-zero-amount",
        "derived-rate-floor",
        "zero-rate-canonical",
        "negative-derived-amount",
    ]);
}
