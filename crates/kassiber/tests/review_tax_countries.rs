//! The review workflow on a processed book of every registered tax country.
//! Review used to build its ledger request through the legacy text
//! serializer, whose field allowlist lacked the fields registry countries
//! emit (`refused_receipts`, `zero_cost_kinds`, `below_market`), so review
//! panicked for them while `journals process` succeeded. Review now builds
//! through the same typed path as processing.

#![cfg(feature = "test-harness")]

mod support;

use kassiber_envelope::get;
use kassiber_pyjson::Value;
use support::{kind_of, path, text, Desk};

const NOW: &str = "2026-01-01T00:00:00Z";

/// Every country of the registry, with its fixed currency (the profile's
/// own for `generic`).
const COUNTRIES: &[(&str, &str)] = &[
    ("generic", "USD"),
    ("at", "EUR"),
    ("us", "USD"),
    ("de", "EUR"),
    ("ch", "CHF"),
    ("ie", "EUR"),
    ("gb", "GBP"),
    ("es", "EUR"),
    ("jp", "JPY"),
];

fn ids(desk: &Desk) -> (String, String) {
    desk.connection()
        .query_row("SELECT workspace_id, id FROM profiles", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
}

/// Two wallets, a priced purchase and sale, and an unpaired transfer the
/// review plan pairs.
fn seed(workspace: &str, profile: &str, fiat: &str) -> String {
    let mut sql = String::new();
    for (wallet, label) in [("source-wallet", "Source"), ("sink-wallet", "Sink")] {
        sql += &format!(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
             VALUES('{wallet}','{workspace}','{profile}','{label}','custom','{{}}','{NOW}');"
        );
    }
    for (tx, wallet, direction, amount, rate, occurred) in [
        (
            "buy",
            "source-wallet",
            "inbound",
            5_000_000_000_i64,
            "30000",
            "2024-02-01T00:00:00Z",
        ),
        (
            "sell",
            "source-wallet",
            "outbound",
            1_000_000_000,
            "50000",
            "2024-09-01T00:00:00Z",
        ),
        (
            "out",
            "source-wallet",
            "outbound",
            100_000_000,
            "60000",
            "2025-03-01T00:00:00Z",
        ),
        (
            "in",
            "sink-wallet",
            "inbound",
            99_000_000,
            "60000",
            "2025-03-01T00:10:00Z",
        ),
    ] {
        sql += &format!(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,\
             occurred_at,direction,asset,amount,fee,fiat_rate,fiat_currency,created_at) \
             VALUES('{tx}','{workspace}','{profile}','{wallet}','fingerprint-{tx}','external-{tx}',\
             '{occurred}','{direction}','BTC',{amount},0,{rate},'{fiat}','{occurred}');"
        );
    }
    sql
}

fn input_version(desk: &Desk) -> i64 {
    desk.connection()
        .query_row(
            "SELECT COALESCE(journal_input_version, 0) FROM profiles",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn require(response: &Value, kind: &str, country: &str) {
    assert_eq!(
        kind_of(response),
        kind,
        "{country}: {}",
        kassiber_pyjson::dumps(response, &kassiber_pyjson::Options::new()).unwrap()
    );
}

fn review_country(country: &str, fiat: &str) {
    {
        let desk = Desk::with_seed("review_tax_countries", None);
        desk.setup_cli(&["workspaces", "create", "Main"]);
        desk.setup_cli(&[
            "profiles",
            "create",
            "Book",
            "--tax-country",
            country,
            "--fiat-currency",
            fiat,
        ]);
        let (workspace, profile) = ids(&desk);
        desk.change(&seed(&workspace, &profile, fiat));
        let processed = desk.setup_cli(&["journals", "process"]);
        assert!(
            path(&processed, &["entries_created"]).is_some(),
            "{country}: {processed:?}"
        );

        let cases = desk.ask("ui.review.cases", "{}");
        require(&cases, "ui.review.cases", country);

        let operations = r#"{"type": "pair", "out_transaction_id": "out", "in_transaction_id": "in", "reason": "Same owner"}"#;
        let plan = desk.ask(
            "ui.review.plan",
            &format!(
                r#"{{"operations": [{operations}], "expected_input_version": {}}}"#,
                input_version(&desk)
            ),
        );
        require(&plan, "ui.review.plan", country);
        let artifact = get(&plan, "data").unwrap();
        let apply = desk.ask(
            "ui.review.apply",
            &format!(
                r#"{{"artifact": {}, "idempotency_key": "pair-{country}"}}"#,
                kassiber_pyjson::dumps(artifact, &kassiber_pyjson::Options::new()).unwrap()
            ),
        );
        require(&apply, "ui.review.apply", country);
        assert_eq!(
            path(&apply, &["data", "status"]).and_then(text),
            Some("verified"),
            "{country}: {apply:?}"
        );
    }
}

#[test]
fn review_cases_plan_and_apply_work_for_every_registered_tax_country() {
    let failed: Vec<&str> = COUNTRIES
        .iter()
        .filter(|(country, fiat)| {
            std::panic::catch_unwind(|| review_country(country, fiat)).is_err()
        })
        .map(|(country, _)| *country)
        .collect();
    assert!(failed.is_empty(), "review failed for {failed:?}");
}
