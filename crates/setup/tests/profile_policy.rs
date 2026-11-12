//! Ports of policy/method/preservation/region and daemon payload cases in
//! tests/test_profile_policy_label.py; display projection tests remain deferred.
mod support;
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;
use support::*;
fn policy(f: &Fixture, country: &str, algo: &str) {
    f.run(
        "profiles.set",
        dict([("tax_country", s(country)), ("gains_algorithm", s(algo))]),
    );
}
#[test]
fn austrian_explicit_method_and_default() {
    for algo in ["FIFO", "HIFO", "MOVING_AVERAGE_AT"] {
        let f = Fixture::seed();
        policy(&f, "at", algo);
        assert_eq!(text(&f.profile(), "gains_algorithm"), algo);
        assert_eq!(text(&f.profile(), "fiat_currency"), "EUR");
        assert_eq!(
            get(&f.profile(), "tax_long_term_days"),
            Some(&Value::Int(i64::MAX.into()))
        );
    }
    let f = Fixture::new();
    f.run("workspaces.create", dict([("label", s("Main"))]));
    let p = f.run(
        "profiles.create",
        dict([("label", s("AT")), ("tax_country", s("at"))]),
    );
    assert_eq!(text(&p, "gains_algorithm"), "MOVING_AVERAGE_AT");
}
#[test]
fn generic_method_preserved() {
    let f = Fixture::seed();
    policy(&f, "generic", "HIFO");
    assert_eq!(text(&f.profile(), "gains_algorithm"), "HIFO");
}
#[test]
fn coarse_review_preserves_fifo() {
    let f = Fixture::seed();
    policy(&f, "at", "FIFO");
    f.run(
        "profiles.set",
        dict([("require_coarse_review", Value::Bool(true))]),
    );
    assert_eq!(text(&f.profile(), "gains_algorithm"), "FIFO");
}
#[test]
fn label_change_preserves_method_and_version() {
    let f = Fixture::seed();
    policy(&f, "at", "FIFO");
    let v = f.version();
    f.run("profiles.set", dict([("label", s("Renamed"))]));
    assert_eq!(f.version(), v);
    assert_eq!(text(&f.profile(), "gains_algorithm"), "FIFO");
    // Persisted pool spelling is normalized before comparing policy changes.
    f.book
        .connection()
        .execute("UPDATE profiles SET cost_basis_pool_scope=' GLOBAL '", [])
        .unwrap();
    f.run("profiles.set", dict([("label", s("Renamed again"))]));
    assert_eq!(f.version(), v);
    assert_eq!(text(&f.profile(), "cost_basis_pool_scope"), "global");
}
#[test]
fn method_change_invalidates_journals() {
    let f = Fixture::seed();
    let v = f.version();
    f.book.connection().execute("UPDATE profiles SET last_processed_at='2026-01-01',last_processed_tx_count=42,ownership_review_counts_json='{}'",[]).unwrap();
    f.run("profiles.set", dict([("gains_algorithm", s("LIFO"))]));
    assert_eq!(f.version(), v + 1);
    assert_eq!(get(&f.profile(), "last_processed_at"), Some(&Value::Null));
    assert_eq!(
        get(&f.profile(), "last_processed_tx_count"),
        Some(&Value::Int(0.into()))
    );
    let saved: Option<String> = f
        .book
        .connection()
        .query_row(
            "SELECT ownership_review_counts_json FROM profiles",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, None);
}
#[test]
fn switch_to_austria_and_back() {
    let f = Fixture::seed();
    policy(&f, "at", "MOVING_AVERAGE_AT");
    assert_eq!(text(&f.profile(), "tax_country"), "at");
    policy(&f, "generic", "FIFO");
    assert_eq!(text(&f.profile(), "tax_country"), "generic");
}
#[test]
fn switch_to_austria_preserves_explicit_fifo() {
    let f = Fixture::seed();
    policy(&f, "at", "FIFO");
    assert_eq!(text(&f.profile(), "gains_algorithm"), "FIFO");
}
#[test]
fn generic_refuses_austrian_method_atomically() {
    let f = Fixture::seed();
    policy(&f, "at", "MOVING_AVERAGE_AT");
    let before = f.profile();
    assert_eq!(
        f.fail(
            "profiles.set",
            dict([
                ("tax_country", s("generic")),
                ("gains_algorithm", s("MOVING_AVERAGE_AT"))
            ])
        )
        .code,
        "validation"
    );
    assert_eq!(f.profile(), before);
}
#[test]
fn daemon_region_and_method_payload() {
    let f = Fixture::seed();
    let id = text(&f.profile(), "id");
    f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_country", s("at")),
            ("gains_algorithm", s("MOVING_AVERAGE_AT")),
        ]),
    );
    assert_eq!(text(&f.profile(), "tax_country"), "at");
    f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("gains_algorithm", s("HIFO")),
        ]),
    );
    assert_eq!(text(&f.profile(), "gains_algorithm"), "HIFO");
    assert_eq!(
        f.fail(
            "ui.profiles.update",
            dict([("profile_id", s(id)), ("tax_country", s("  "))])
        )
        .code,
        "validation"
    );
}
#[test]
fn daemon_create_region_method_and_reset_holding_period() {
    let f = Fixture::seed();
    let wid = text(&f.profile(), "workspace_id");
    for (label, country, algo) in [
        ("AT", "at", "MOVING_AVERAGE_AT"),
        ("generic", "generic", "HIFO"),
        ("AT FIFO", "at", "FIFO"),
        ("Generic again", "generic", "FIFO"),
    ] {
        let p = f.run(
            "ui.profiles.create",
            dict([
                ("workspace_id", s(wid.as_str())),
                ("label", s(label)),
                ("tax_country", s(country)),
                ("gains_algorithm", s(algo)),
            ]),
        );
        let d = get(&p, "defaults").unwrap();
        assert_eq!(text(d, "tax_country"), country);
        assert_eq!(text(d, "gains_algorithm"), algo);
        assert_eq!(
            get(d, "tax_long_term_days"),
            Some(&Value::Int(
                (if country == "at" { i64::MAX } else { 365 }).into()
            ))
        );
    }
}
#[test]
fn daemon_copy_inherits_ignoring_region_override() {
    let f = Fixture::seed();
    policy(&f, "at", "MOVING_AVERAGE_AT");
    let source = f.profile();
    let p = f.run(
        "ui.profiles.create",
        dict([
            (
                "workspace_id",
                get(&source, "workspace_id").unwrap().clone(),
            ),
            ("source_profile_id", get(&source, "id").unwrap().clone()),
            ("label", s("Copy")),
            ("tax_country", s("generic")),
            ("gains_algorithm", s("FIFO")),
        ]),
    );
    let d = get(&p, "defaults").unwrap();
    assert_eq!(text(d, "tax_country"), "at");
    assert_eq!(text(d, "gains_algorithm"), "MOVING_AVERAGE_AT");
}
#[test]
fn united_states_profiles_follow_the_registry() {
    let f = Fixture::new();
    f.run("workspaces.create", dict([("label", s("Main"))]));
    let p = f.run(
        "profiles.create",
        dict([
            ("label", s("US")),
            ("tax_country", s("us")),
            ("fiat_currency", s("EUR")),
            ("tax_long_term_days", Value::Int(10.into())),
        ]),
    );
    // USD is required, the holding period is the calendar rule (stored 365),
    // FIFO and per-wallet identification are the defaults.
    assert_eq!(text(&p, "fiat_currency"), "USD");
    assert_eq!(get(&p, "tax_long_term_days"), Some(&Value::Int(365.into())));
    assert_eq!(text(&p, "gains_algorithm"), "FIFO");
    assert_eq!(text(&p, "cost_basis_pool_scope"), "wallet");
    let details = f.run("profiles.get", dict([("profile", s("US"))]));
    assert_eq!(
        get(&details, "allowed_cost_basis_pool_scopes"),
        Some(&Value::List(vec![s("global"), s("wallet")]))
    );
    // A US book may go back to the global scope (before 2025) but not to a
    // moving average.
    f.run(
        "profiles.set",
        dict([("profile", s("US")), ("cost_basis_pool_scope", s("global"))]),
    );
    let error = f.fail(
        "profiles.set",
        dict([
            ("profile", s("US")),
            ("gains_algorithm", s("MOVING_AVERAGE")),
        ]),
    );
    assert_eq!(error.code, "validation");
    assert_eq!(
        error.message,
        "Unsupported gains algorithm 'MOVING_AVERAGE' for tax_country='us'"
    );
    // Generic and Austrian books still refuse the wallet scope.
    let error = f.fail(
        "profiles.create",
        dict([
            ("label", s("G")),
            ("tax_country", s("generic")),
            ("cost_basis_pool_scope", s("wallet")),
        ]),
    );
    assert_eq!(error.hint.as_deref(), Some("Choose one of: global"));
}
#[test]
fn spanish_and_japanese_profiles_follow_the_registry() {
    let f = Fixture::new();
    f.run("workspaces.create", dict([("label", s("Main"))]));
    let es = f.run(
        "profiles.create",
        dict([
            ("label", s("ES")),
            ("tax_country", s("es")),
            ("fiat_currency", s("USD")),
        ]),
    );
    assert_eq!(text(&es, "fiat_currency"), "EUR");
    assert_eq!(text(&es, "gains_algorithm"), "FIFO");
    assert_eq!(text(&es, "cost_basis_pool_scope"), "global");
    // Japan's averages are methods Python does not list: accepted for a
    // Japanese book, refused for another with that country's choices.
    let jp = f.run(
        "profiles.create",
        dict([
            ("label", s("JP")),
            ("tax_country", s("jp")),
            ("gains_algorithm", s("jp_moving_average")),
        ]),
    );
    assert_eq!(text(&jp, "fiat_currency"), "JPY");
    assert_eq!(text(&jp, "gains_algorithm"), "JP_MOVING_AVERAGE");
    let error = f.fail(
        "profiles.create",
        dict([
            ("label", s("ES2")),
            ("tax_country", s("es")),
            ("gains_algorithm", s("JP_TOTAL_AVERAGE")),
        ]),
    );
    assert_eq!(error.hint.as_deref(), Some("Choose one of: FIFO"));
    let error = f.fail(
        "profiles.create",
        dict([
            ("label", s("JP2")),
            ("tax_country", s("jp")),
            ("gains_algorithm", s("FIFO")),
        ]),
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("Choose one of: JP_MOVING_AVERAGE, JP_TOTAL_AVERAGE")
    );
    // An unknown method keeps Python's message and list.
    let error = f.fail(
        "profiles.create",
        dict([
            ("label", s("JP3")),
            ("tax_country", s("jp")),
            ("gains_algorithm", s("BOGUS")),
        ]),
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("Choose one of: FIFO, LIFO, HIFO, LOFO, MOVING_AVERAGE, MOVING_AVERAGE_AT")
    );
}
#[test]
fn preview_books_carry_the_snapshot_flag_and_label() {
    let f = Fixture::new();
    f.run("workspaces.create", dict([("label", s("Main"))]));
    f.run(
        "profiles.create",
        dict([("label", s("G")), ("fiat_currency", s("EUR"))]),
    );
    f.run(
        "profiles.create",
        dict([("label", s("US")), ("tax_country", s("us"))]),
    );
    let snapshot = f.run("ui.profiles.snapshot", dict::<&str>([]));
    let Some(Value::List(workspaces)) = get(&snapshot, "workspaces") else {
        panic!("{snapshot:?}");
    };
    let Some(Value::List(profiles)) = get(&workspaces[0], "profiles") else {
        panic!("no profiles");
    };
    let by_name = |name: &str| {
        profiles
            .iter()
            .find(|p| get(p, "name") == Some(&s(name)))
            .unwrap()
            .clone()
    };
    // Reviewed books keep the reference's projection: no flag at all.
    assert_eq!(get(&by_name("G"), "taxCountryPreview"), None);
    let us = by_name("US");
    assert_eq!(get(&us, "taxCountryPreview"), Some(&Value::Bool(true)));
    assert_eq!(
        text(&us, "taxPolicy"),
        "United States (preview) - FIFO - USD - long-term after more than 1 year"
    );
    assert_eq!(text(&workspaces[0], "jurisdiction"), "Mixed");
}
#[test]
fn a_currency_change_reports_the_rows_it_holds() {
    let f = Fixture::seed();
    let profile = text(&f.profile(), "id");
    let workspace = text(&f.profile(), "workspace_id");
    let conn = f.book.connection();
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('w',?1,?2,'W','custom','fixed')",
        [&workspace, &profile],
    )
    .unwrap();
    for (id, currency, rate, value, excluded) in [
        ("eur", Some("EUR"), Some(40000.0), None, 0),
        ("eur-value", Some("eur "), None, Some("20000"), 0),
        ("gbp", Some("GBP"), Some(30000.0), None, 0),
        ("usd", Some("USD"), Some(45000.0), None, 0),
        ("unpriced", Some("EUR"), None, Some(""), 0),
        ("excluded", Some("EUR"), Some(40000.0), None, 1),
        ("blank", Some(""), Some(40000.0), None, 0),
        ("unrecorded", None, Some(40000.0), None, 0),
    ] {
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fiat_currency,fiat_rate,fiat_value_exact,excluded,created_at) VALUES(?1,?2,?3,'w',?1,'2024-01-01T00:00:00Z','inbound','BTC',1,?4,?5,?6,?7,'fixed')",
            kassiber_store::rusqlite::params![id, workspace, profile, currency, rate, value, excluded],
        )
        .unwrap();
    }
    let holds = |update: Value| get(&update, "price_currency_holds").cloned();
    let expected = |book: &str, total: i64, by: Vec<(&str, i64)>| {
        Some(dict([
            ("reason", s("price_currency_mismatch")),
            ("book_currency", s(book)),
            ("transactions", Value::Int(total.into())),
            (
                "by_currency",
                dict(by.into_iter().map(|(c, n)| (c, Value::Int(n.into())))),
            ),
        ]))
    };
    let update = f.run("profiles.set", dict([("fiat_currency", s("eur"))]));
    assert_eq!(
        holds(update),
        expected("EUR", 2, vec![("GBP", 1), ("USD", 1)])
    );
    // An update that keeps the currency reports nothing.
    let update = f.run("profiles.set", dict([("label", s("Renamed"))]));
    assert_eq!(holds(update), None);
    // A country's own currency counts as a change of the book's.
    let update = f.run("profiles.set", dict([("tax_country", s("us"))]));
    assert_eq!(
        holds(update),
        expected("USD", 3, vec![("EUR", 2), ("GBP", 1)])
    );
}
