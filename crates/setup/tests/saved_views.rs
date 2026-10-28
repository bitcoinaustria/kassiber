//! Ports of all nine SavedViewsTests in tests/test_saved_views.py.
mod support;
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;
use kassiber_setup::views;
use support::*;
#[test]
fn create_and_list() {
    let f = Fixture::seed();
    let v = f.view("swap", "A");
    let list = f.run("views.list", dict::<&str>([]));
    assert_eq!(list, Value::List(vec![v]));
}
#[test]
fn list_filters_by_surface() {
    let f = Fixture::seed();
    let a = f.view("swap", "A");
    f.view("other", "B");
    assert_eq!(
        f.run("views.list", dict([("surface", s("swap"))])),
        Value::List(vec![a])
    );
}
#[test]
fn duplicate_name_on_surface_rejected() {
    let f = Fixture::seed();
    f.view("swap", "A");
    assert_eq!(
        f.fail(
            "views.create",
            dict([("surface", s("swap")), ("name", s("A"))])
        )
        .code,
        "conflict"
    );
}
#[test]
fn same_name_on_different_surface_allowed() {
    let f = Fixture::seed();
    f.view("swap", "A");
    f.view("other", "A");
    let Value::List(v) = f.run("views.list", dict::<&str>([])) else {
        panic!()
    };
    assert_eq!(v.len(), 2);
}
#[test]
fn delete_view() {
    let f = Fixture::seed();
    let v = f.view("swap", "A");
    let id = text(&v, "id");
    assert_eq!(
        f.run("views.delete", dict([("view_id", s(id.as_str()))])),
        dict([("deleted", s(id))])
    );
    assert_eq!(f.run("views.list", dict::<&str>([])), Value::List(vec![]));
}
#[test]
fn delete_other_profile_view_blocked() {
    let f = Fixture::seed();
    let v = f.view("swap", "A");
    f.run(
        "profiles.create",
        dict([("label", s("Other")), ("fiat_currency", s("USD"))]),
    );
    assert_eq!(
        f.fail(
            "views.delete",
            dict([("view_id", get(&v, "id").unwrap().clone())])
        )
        .code,
        "not_found"
    );
}
#[test]
fn update_view_name_and_filter() {
    let f = Fixture::seed();
    let v = f.view("swap", "A");
    let filter = dict([("new", Value::Bool(true))]);
    let updated = views::update(
        f.book.connection(),
        &text(&v, "profile_id"),
        &text(&v, "id"),
        Some("New"),
        Some(&filter),
        &f.sources,
    )
    .unwrap();
    assert_eq!(text(&updated, "name"), "New");
    assert_eq!(get(&updated, "filter"), Some(&filter));
    assert_eq!(get(&updated, "created_at"), get(&v, "created_at"));
}
#[test]
fn update_to_duplicate_name_rejected() {
    let f = Fixture::seed();
    f.view("swap", "Taken");
    let v = f.view("swap", "Free");
    assert_eq!(
        views::update(
            f.book.connection(),
            &text(&v, "profile_id"),
            &text(&v, "id"),
            Some("Taken"),
            None,
            &f.sources
        )
        .unwrap_err()
        .code,
        "conflict"
    );
}
#[test]
fn empty_name_rejected() {
    let f = Fixture::seed();
    assert_eq!(
        f.fail(
            "views.create",
            dict([("surface", s("swap")), ("name", s(" \u{1c}"))])
        )
        .code,
        "validation"
    );
}
