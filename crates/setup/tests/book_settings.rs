//! Book settings: one read and one change for every setting of any book,
//! with a dry run (docs/reference/tax.md#book-settings).
mod support;
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;
use support::*;

fn stored(f: &Fixture, id: &str) -> Option<String> {
    f.book
        .connection()
        .query_row(
            "SELECT value FROM settings WHERE key=?",
            [format!("tax.calendar.profile.{id}")],
            |r| r.get(0),
        )
        .ok()
}

fn fields(preview: &Value) -> Vec<String> {
    let Some(Value::List(changes)) = get(preview, "changes") else {
        panic!("{preview:?}")
    };
    changes.iter().map(|c| text(c, "field")).collect()
}

#[test]
fn settings_read_any_book_with_its_choices() {
    let f = Fixture::seed();
    let id = text(&f.profile(), "id");
    // A second book becomes the open one; the first is still readable.
    f.run(
        "profiles.create",
        dict([("label", s("Other")), ("tax_country", s("gb"))]),
    );
    let settings = f.run(
        "ui.profiles.settings",
        dict([("profile_id", s(id.as_str()))]),
    );
    assert_eq!(text(&settings, "profile_id"), id);
    assert_eq!(get(&settings, "is_current"), Some(&Value::Bool(false)));
    assert_eq!(text(&settings, "tax_year_start"), "01-01");
    assert_eq!(text(&settings, "timezone"), "UTC");
    assert_eq!(
        get(&settings, "tax_year_editable"),
        Some(&Value::Bool(true))
    );
    let Some(Value::List(presets)) = get(&settings, "tax_year_presets") else {
        panic!()
    };
    let ids: Vec<String> = presets.iter().map(|p| text(p, "id")).collect();
    assert_eq!(ids, ["calendar", "uk", "april", "july", "march"]);
    let gb = f.run("profiles.settings", dict([("profile", s("Other"))]));
    assert_eq!(text(&gb, "tax_year_start"), "04-06");
    assert_eq!(text(&gb, "timezone"), "Europe/London");
    assert_eq!(get(&gb, "tax_year_editable"), Some(&Value::Bool(false)));
    assert_eq!(get(&gb, "fiat_currency_locked"), Some(&Value::Bool(true)));
    assert_eq!(get(&gb, "tax_year_presets"), Some(&Value::List(vec![])));
    // An unknown book.
    assert_eq!(
        f.fail("ui.profiles.settings", dict([("profile_id", s("nope"))]))
            .code,
        "validation"
    );
}

#[test]
fn a_dry_run_reports_the_change_and_writes_nothing() {
    let f = Fixture::seed();
    let id = text(&f.profile(), "id");
    let before = f.profile();
    let version = f.version();
    let preview = f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_year_start", s("july")),
            ("timezone", s("Australia/Sydney")),
            ("fiat_currency", s("AUD")),
            ("dry_run", Value::Bool(true)),
        ]),
    );
    assert_eq!(get(&preview, "dry_run"), Some(&Value::Bool(true)));
    assert_eq!(
        fields(&preview),
        ["fiat_currency", "tax_year_start", "timezone"]
    );
    let journals = get(&preview, "journals").unwrap();
    assert_eq!(get(journals, "reprocess"), Some(&Value::Bool(true)));
    let Some(Value::List(readiness)) = get(&preview, "readiness") else {
        panic!()
    };
    assert!(readiness.contains(&s("tax_years_regrouped")));
    assert!(get(&preview, "price_currency_holds").is_some());
    assert_eq!(
        text(get(&preview, "settings").unwrap(), "tax_year_start"),
        "07-01"
    );
    // Nothing was written.
    assert_eq!(f.profile(), before);
    assert_eq!(f.version(), version);
    assert_eq!(stored(&f, &id), None);
    // A refused change is refused in the dry run too.
    let error = f.fail(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_year_start", s("02-29")),
            ("dry_run", Value::Bool(true)),
        ]),
    );
    assert_eq!(error.code, "tax_year_start_invalid");
    assert_eq!(
        f.fail(
            "ui.profiles.update",
            dict([
                ("profile_id", s(id.as_str())),
                ("timezone", s("UTC")),
                ("dry_run", s("yes")),
            ]),
        )
        .code,
        "validation"
    );
}

#[test]
fn applying_a_calendar_stores_only_the_choice_and_invalidates_journals() {
    let f = Fixture::seed();
    let id = text(&f.profile(), "id");
    let version = f.version();
    let applied = f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_year_start", s("04-06")),
            ("timezone", s("Europe/Vienna")),
        ]),
    );
    assert_eq!(text(&applied, "tax_year_start"), "04-06");
    assert_eq!(text(&applied, "timezone"), "Europe/Vienna");
    assert_eq!(f.version(), version + 1);
    assert_eq!(
        stored(&f, &id).as_deref(),
        Some(r#"{"tax_year_start":"04-06","timezone":"Europe/Vienna"}"#)
    );
    // Applying the same again changes nothing.
    f.run(
        "ui.profiles.update",
        dict([("profile_id", s(id.as_str())), ("tax_year_start", s("uk"))]),
    );
    assert_eq!(f.version(), version + 1);
    // Back to the country's: the key goes, the journals are stale again.
    f.run(
        "profiles.configure",
        dict([
            ("profile", s(id.as_str())),
            ("tax_year_start", s("default")),
            ("timezone", s("default")),
        ]),
    );
    assert_eq!(stored(&f, &id), None);
    assert_eq!(f.version(), version + 2);
    // The other settings, for a book that is not the open one.
    f.run(
        "profiles.create",
        dict([("label", s("Other")), ("fiat_currency", s("EUR"))]),
    );
    f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_long_term_days", Value::Int(400.into())),
            ("require_coarse_review", Value::Bool(true)),
            ("bitcoin_rail_carrying_value", Value::Bool(false)),
            ("fiat_currency", s("chf")),
        ]),
    );
    let settings = f.run(
        "ui.profiles.settings",
        dict([("profile_id", s(id.as_str()))]),
    );
    assert_eq!(
        get(&settings, "tax_long_term_days"),
        Some(&Value::Int(400.into()))
    );
    assert_eq!(
        get(&settings, "require_coarse_review"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        get(&settings, "bitcoin_rail_carrying_value"),
        Some(&Value::Bool(false))
    );
    assert_eq!(text(&settings, "fiat_currency"), "CHF");
    assert_eq!(
        f.fail(
            "ui.profiles.update",
            dict([
                ("profile_id", s(id.as_str())),
                ("require_coarse_review", s("true"))
            ]),
        )
        .code,
        "validation"
    );
}

#[test]
fn a_legal_tax_year_is_locked_and_a_region_switch_resets_a_choice() {
    let f = Fixture::seed();
    let id = text(&f.profile(), "id");
    f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_year_start", s("07-01")),
        ]),
    );
    // Switching to Austria resets the generic book's own year to 1 January.
    let switched = f.run(
        "ui.profiles.update",
        dict([("profile_id", s(id.as_str())), ("tax_country", s("at"))]),
    );
    assert_eq!(
        get(&switched, "tax_calendar_reset"),
        Some(&Value::Bool(true))
    );
    assert_eq!(stored(&f, &id), None);
    // Austria's year and zone cannot be changed; asking for its own is fine.
    let error = f.fail(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_year_start", s("07-01")),
        ]),
    );
    assert_eq!(error.code, "tax_year_start_locked");
    assert_eq!(
        f.fail(
            "ui.profiles.update",
            dict([("profile_id", s(id.as_str())), ("timezone", s("UTC"))]),
        )
        .code,
        "timezone_locked"
    );
    // Asking for a new region's locked year together with it fails whole.
    let error = f.fail(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_country", s("generic")),
            ("gains_algorithm", s("FIFO")),
            ("tax_year_start", s("02-29")),
        ]),
    );
    assert_eq!(error.code, "tax_year_start_invalid");
    assert_eq!(text(&f.profile(), "tax_country"), "at");
    // Australia keeps 1 July and lets the book choose its state's zone.
    f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_country", s("au")),
            ("gains_algorithm", s("FIFO")),
            ("timezone", s("Australia/Perth")),
        ]),
    );
    let settings = f.run(
        "ui.profiles.settings",
        dict([("profile_id", s(id.as_str()))]),
    );
    assert_eq!(text(&settings, "tax_year_start"), "07-01");
    assert_eq!(text(&settings, "timezone"), "Australia/Perth");
    assert_eq!(text(&settings, "fiat_currency"), "AUD");
    assert_eq!(
        get(&settings, "tax_country_preview"),
        Some(&Value::Bool(true))
    );
}

/// A stored calendar its country no longer allows (a registry change, a
/// hand-edited row) fails closed where the calendar is computed with, but
/// the book's settings still open, report it, and accept the `default`
/// reset that repairs it.
#[test]
fn a_calendar_the_country_no_longer_allows_is_reported_and_can_be_reset() {
    let f = Fixture::seed();
    let id = text(&f.profile(), "id");
    f.run(
        "ui.profiles.update",
        dict([("profile_id", s(id.as_str())), ("tax_country", s("at"))]),
    );
    let key = format!("tax.calendar.profile.{id}");
    let corrupt = |value: &str| {
        f.book
            .connection()
            .execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)",
                [key.as_str(), value],
            )
            .unwrap();
    };
    corrupt(r#"{"tax_year_start":"07-01"}"#);
    let version = f.version();
    let settings = f.run(
        "ui.profiles.settings",
        dict([("profile_id", s(id.as_str()))]),
    );
    let invalid = get(&settings, "calendar_invalid").expect("reported");
    assert_eq!(text(invalid, "code"), "tax_year_start_locked");
    assert_eq!(text(invalid, "tax_year_start"), "07-01");
    assert_eq!(get(invalid, "timezone"), Some(&Value::Null));
    assert_eq!(text(&settings, "tax_year_start"), "07-01");
    assert_eq!(text(&settings, "timezone"), "Europe/Vienna");
    assert_eq!(
        get(&settings, "calendar_is_default"),
        Some(&Value::Bool(false))
    );
    let details = f.run("profiles.get", dict([("profile", s(id.as_str()))]));
    assert_eq!(
        text(get(&details, "calendar_invalid").unwrap(), "code"),
        "tax_year_start_locked"
    );
    // A change that leaves it in place is still refused.
    assert_eq!(
        f.fail(
            "ui.profiles.update",
            dict([("profile_id", s(id.as_str())), ("timezone", s("default"))]),
        )
        .code,
        "tax_year_start_locked"
    );
    // The reset previews, then applies and invalidates the journals.
    let reset = dict([
        ("profile_id", s(id.as_str())),
        ("tax_year_start", s("default")),
        ("timezone", s("default")),
    ]);
    let dry = dict([
        ("profile_id", s(id.as_str())),
        ("tax_year_start", s("default")),
        ("timezone", s("default")),
        ("dry_run", Value::Bool(true)),
    ]);
    let preview = f.run("ui.profiles.update", dry);
    assert_eq!(fields(&preview), ["tax_year_start"]);
    assert_eq!(
        get(get(&preview, "journals").unwrap(), "reprocess"),
        Some(&Value::Bool(true))
    );
    assert!(get(get(&preview, "settings").unwrap(), "calendar_invalid").is_none());
    assert!(stored(&f, &id).is_some());
    assert_eq!(f.version(), version);
    f.run("ui.profiles.update", reset);
    assert_eq!(stored(&f, &id), None);
    assert_eq!(f.version(), version + 1);
    let settings = f.run(
        "ui.profiles.settings",
        dict([("profile_id", s(id.as_str()))]),
    );
    assert!(get(&settings, "calendar_invalid").is_none());
    assert_eq!(text(&settings, "tax_year_start"), "01-01");
    // A value that cannot be read is reported too, and any change drops it.
    corrupt("not json");
    let settings = f.run(
        "ui.profiles.settings",
        dict([("profile_id", s(id.as_str()))]),
    );
    assert_eq!(
        text(get(&settings, "calendar_invalid").unwrap(), "code"),
        "internal_error"
    );
    let applied = f.run(
        "profiles.configure",
        dict([("profile", s(id.as_str())), ("label", s("Renamed"))]),
    );
    assert_eq!(
        get(&applied, "tax_calendar_reset"),
        Some(&Value::Bool(true))
    );
    assert_eq!(stored(&f, &id), None);
}

#[test]
fn methods_without_a_form_in_the_scope_are_refused_up_front() {
    let f = Fixture::new();
    f.run("workspaces.create", dict([("label", s("Main"))]));
    f.run(
        "profiles.create",
        dict([("label", s("DE")), ("tax_country", s("de"))]),
    );
    let us = f.run(
        "profiles.create",
        dict([
            ("label", s("US")),
            ("tax_country", s("us")),
            ("cost_basis_pool_scope", s("global_then_wallet")),
        ]),
    );
    assert_eq!(text(&us, "cost_basis_pool_scope"), "global_then_wallet");
    // Germany's wallet-only order of use with Japan's global average.
    let error = f.fail(
        "profiles.set",
        dict([
            ("profile", s("DE")),
            ("tax_country", s("jp")),
            ("gains_algorithm", s("JP_TOTAL_AVERAGE")),
        ]),
    );
    // Japan allows the global pool only: the scope check comes first.
    assert_eq!(error.code, "validation");
    let settings = f.run("profiles.settings", dict([("profile", s("US"))]));
    let scopes = get(get(&settings, "method_pool_scopes").unwrap(), "FIFO").unwrap();
    assert_eq!(
        scopes,
        &Value::List(vec![s("global"), s("global_then_wallet"), s("wallet")])
    );
    // A generic book has the global pool only until its switch is signed
    // off (docs/reference/tax.md#book-settings).
    let error = f.fail(
        "profiles.create",
        dict([
            ("label", s("G")),
            ("gains_algorithm", s("MOVING_AVERAGE")),
            ("cost_basis_pool_scope", s("wallet")),
        ]),
    );
    assert_eq!(error.code, "validation");
}

#[test]
fn a_copied_book_keeps_its_sources_calendar_and_a_deleted_one_forgets_it() {
    let f = Fixture::seed();
    let source = f.profile();
    let id = text(&source, "id");
    f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_year_start", s("march")),
        ]),
    );
    let created = f.run(
        "ui.profiles.create",
        dict([
            (
                "workspace_id",
                get(&source, "workspace_id").unwrap().clone(),
            ),
            ("source_profile_id", s(id.as_str())),
            ("label", s("Copy")),
        ]),
    );
    let copy = text(get(&created, "profile").unwrap(), "id");
    assert_eq!(
        stored(&f, &copy).as_deref(),
        Some(r#"{"tax_year_start":"03-01"}"#)
    );
    let snapshot = f.run("ui.profiles.snapshot", dict::<&str>([]));
    let text_snapshot = format!("{snapshot:?}");
    assert!(text_snapshot.contains("taxYearStart"), "{text_snapshot}");
}

/// A `global_then_wallet` book's engine request names its wallets by when
/// they were added, so the 2025 allocation does not follow a rename; every
/// other book's request has no such key.
#[test]
fn a_transition_books_wallets_reach_the_engine_by_when_they_were_added() {
    let f = Fixture::seed();
    let profile = f.profile();
    let (id, workspace) = (text(&profile, "id"), text(&profile, "workspace_id"));
    let conn = f.book.connection();
    for (wallet, label, created) in [
        ("w-b", "Hot", "2024-01-02T00:00:00Z"),
        ("w-a", "Cold", "2024-01-02T00:00:00Z"),
        ("w-c", "Archive", "2024-01-01T00:00:00Z"),
    ] {
        conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES(?,?,?,?,'custom',?)",
            [wallet, workspace.as_str(), id.as_str(), label, created],
        )
        .unwrap();
    }
    let keys = || kassiber_setup::book_settings::engine_profile_keys(conn, &id).unwrap();
    assert!(keys().is_empty());
    f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_country", s("us")),
            ("gains_algorithm", s("HIFO")),
            ("cost_basis_pool_scope", s("global_then_wallet")),
        ]),
    );
    let order = vec![(
        "wallet_allocation_order",
        r#"["w-c","w-a","w-b"]"#.to_owned(),
    )];
    assert_eq!(keys(), order);
    // A rename keeps the order.
    conn.execute("UPDATE wallets SET label='Vault' WHERE id='w-a'", [])
        .unwrap();
    assert_eq!(keys(), order);
}

/// A generic book is taxed with 365 days when its stored days are zero
/// (the engine reads `int(days or 365)`), so every label and projection
/// says 365, never a zero-day holding period; asking for 0 returns the book
/// to that default.
#[test]
fn zero_long_term_days_read_as_the_default_everywhere() {
    let f = Fixture::seed();
    let id = text(&f.profile(), "id");
    f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_long_term_days", Value::Int(400.into())),
        ]),
    );
    let applied = f.run(
        "ui.profiles.update",
        dict([
            ("profile_id", s(id.as_str())),
            ("tax_long_term_days", Value::Int(0.into())),
        ]),
    );
    assert_eq!(
        get(&applied, "tax_long_term_days"),
        Some(&Value::Int(365.into()))
    );
    // A stored zero (an older book, a hand-edited row).
    f.book
        .connection()
        .execute(
            "UPDATE profiles SET tax_long_term_days=0 WHERE id=?",
            [id.as_str()],
        )
        .unwrap();
    let settings = f.run(
        "ui.profiles.settings",
        dict([("profile_id", s(id.as_str()))]),
    );
    assert_eq!(
        get(&settings, "tax_long_term_days"),
        Some(&Value::Int(365.into()))
    );
    let details = f.run("profiles.get", dict([("profile", s(id.as_str()))]));
    assert_eq!(
        get(&details, "tax_long_term_days"),
        Some(&Value::Int(365.into()))
    );
    let snapshot = kassiber_pyjson::dumps(
        &f.run("ui.profiles.snapshot", dict::<&str>([])),
        &Default::default(),
    )
    .unwrap();
    assert!(snapshot.contains("365 day long-term"), "{snapshot}");
    assert!(snapshot.contains("\"taxLongTermDays\": 365"), "{snapshot}");
    assert!(!snapshot.contains(" 0 day long-term"), "{snapshot}");
}
