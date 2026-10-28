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
