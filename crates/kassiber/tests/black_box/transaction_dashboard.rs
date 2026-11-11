//! `ui.transactions.dashboard`. Replaces the kill-holding tests of
//! `tests/test_rust_transaction_dashboard.py`. Every request names its
//! `until`, so no bucket depends on the day the test runs.

use serde_json::{json, Value};

use crate::desktop::{self, compare, some, ACTIVITY, DECADE};
use crate::support::Transcript;

const KIND: &str = "ui.transactions.dashboard";

fn window(period: &str) -> Value {
    json!({"period": period, "since": "2025-01-01T00:00:00Z", "until": "2025-06-30T23:59:59Z"})
}

#[test]
fn test_period_aliases_bucket_like_their_canonical_names() {
    let sandbox = desktop::book();
    sandbox.sql(ACTIVITY);
    let mut t = Transcript::new("transaction_dashboard/period_aliases");
    let mut args = Vec::new();
    let periods = [
        "30d", "30day", "3m", "3month", "6m", "1y", "1years", "5y", "5year", "10y", "15year",
    ];
    for period in periods {
        args.push(window(period));
        args.push(json!({"period": period, "since": "2024-10-20T00:00:00Z",
            "until": "2025-04-10T12:00:00Z", "timezone": "Europe/Vienna"}));
    }
    let responses = compare(&mut t, &sandbox, KIND, &some(args));
    let keys = |index: usize| -> Vec<String> {
        responses[index * 2]["data"]["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["bucketKey"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(keys(0)[..2], ["2025-01-01", "2025-01-02"]);
    assert_eq!(keys(2)[..2], ["2024-12-30", "2025-01-06"]);
    assert_eq!(keys(7), ["2025-Q1", "2025-Q2"]);
    t.finish();
}

#[test]
fn test_auto_period_selection_matches_wire_bytes() {
    let sandbox = desktop::book();
    sandbox.sql(ACTIVITY);
    let mut t = Transcript::new("transaction_dashboard/auto_period_selection");
    let mut args = Vec::new();
    for until in [
        "2025-04-30T00:00:00Z",
        "2024-02-29T12:00:00Z",
        "2028-02-29T12:00:00Z",
        "2045-01-01T00:00:00Z",
        "2023-12-31T23:30:00Z",
        "2020-01-01T00:00:00Z",
    ] {
        for timezone in ["UTC", "Europe/Vienna", "Pacific/Kiritimati"] {
            args.push(
                json!({"period": "all", "until": until, "timezone": timezone,
                "resolveAuto": true}),
            );
            args.push(
                json!({"period": "all", "until": until, "timezone": timezone,
                "resolveAuto": true, "wallet": "exchange"}),
            );
        }
    }
    compare(&mut t, &sandbox, KIND, &some(args));
    sandbox.sql("UPDATE transactions SET amount=1 WHERE profile_id='p';");
    compare(
        &mut t,
        &sandbox,
        KIND,
        &some(vec![
            json!({"period": "all", "until": "2025-04-30T00:00:00Z", "resolveAuto": true}),
        ]),
    );
    sandbox.sql("DELETE FROM transactions WHERE profile_id='p';");
    compare(
        &mut t,
        &sandbox,
        KIND,
        &some(vec![
            json!({"period": "all", "until": "2025-04-30T00:00:00Z", "resolveAuto": true}),
            json!({"period": "all", "until": "2025-04-30T00:00:00Z"}),
            window("30days"),
        ]),
    );
    t.finish();
}

#[test]
fn test_years_index_and_bucket_sizes_match_wire_bytes() {
    let sandbox = desktop::book();
    sandbox.sql(&format!("{ACTIVITY}{DECADE}"));
    let mut t = Transcript::new("transaction_dashboard/years_index_and_bucket_sizes");
    let until = "2025-06-30T23:59:59Z";
    let mut args = Vec::new();
    for timezone in [
        "UTC",
        "Europe/Vienna",
        "America/New_York",
        "Pacific/Kiritimati",
    ] {
        args.push(json!({"period": "all", "timezone": timezone, "until": until}));
        args.push(
            json!({"period": "all", "timezone": timezone, "wallet": "exchange",
            "until": until}),
        );
        args.push(
            json!({"period": "all", "timezone": timezone, "resolveAuto": true,
            "until": until}),
        );
    }
    for period in [
        "4years", "4y", "4 Year", "5years", "10y", "15years", "30days", "3months", "ytd", "1year",
    ] {
        args.push(json!({"period": period, "since": "2016-01-01T00:00:00Z", "until": until}));
    }
    for until in [
        "2014-01-01T00:00:00Z",
        "2017-06-01T00:00:00Z",
        "2025-04-30T00:00:00Z",
    ] {
        args.push(json!({"period": "all", "until": until, "resolveAuto": true}));
    }
    let responses = compare(&mut t, &sandbox, KIND, &some(args));
    let first = &responses[0]["data"];
    assert_eq!(first["bucketSize"], "year");
    let years: Vec<i64> = first["history"]["years"]
        .as_array()
        .unwrap()
        .iter()
        .map(|y| y["year"].as_i64().unwrap())
        .collect();
    assert_eq!(years.last(), Some(&2012));
    assert!(!years.contains(&1970));
    t.finish();
}
