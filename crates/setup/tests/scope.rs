mod support;
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;
use support::*;
#[test]
fn id_lookup_precedes_labels_and_folded_labels_are_ambiguous() {
    let f = Fixture::seed();
    let wid = text(&f.profile(), "workspace_id");
    f.run("workspaces.create", dict([("label", s(wid.as_str()))]));
    let a = f.run("profiles.list", dict([("workspace", s(wid))]));
    assert!(matches!(a,Value::List(v) if v.len()==1));
    f.run("workspaces.create", dict([("label", s("MAIN"))]));
    let error = f.fail("profiles.list", dict([("workspace", s("main"))]));
    assert_eq!(error.code, "validation");
    let Value::List(v) = get(error.details.as_ref().unwrap(), "matches").unwrap() else {
        panic!()
    };
    assert_eq!(v.len(), 2);
}
#[test]
fn profile_labels_are_scoped_and_ids_win() {
    let f = Fixture::seed();
    let pid = text(&f.profile(), "id");
    f.run(
        "profiles.create",
        dict([("label", s(pid.as_str())), ("fiat_currency", s("USD"))]),
    );
    assert_eq!(
        text(
            &f.run("profiles.get", dict([("profile", s(pid.as_str()))])),
            "id"
        ),
        pid
    );
    f.run(
        "profiles.create",
        dict([("label", s("BOOK")), ("fiat_currency", s("USD"))]),
    );
    assert_eq!(
        f.fail("profiles.get", dict([("profile", s("book"))])).code,
        "validation"
    );
    f.run("workspaces.create", dict([("label", s("Other"))]));
    f.run(
        "profiles.create",
        dict([("label", s("Book")), ("fiat_currency", s("USD"))]),
    );
    assert_eq!(
        text(
            &f.run("profiles.get", dict([("profile", s("book"))])),
            "label"
        ),
        "Book"
    );
}
#[test]
fn failed_context_switch_rolls_back_workspace() {
    let f = Fixture::seed();
    let before = f.run("context.show", dict::<&str>([]));
    f.run("workspaces.create", dict([("label", s("Other"))]));
    f.run(
        "context.set",
        dict([("workspace", s("Main")), ("profile", s("Book"))]),
    );
    assert_eq!(
        f.fail(
            "context.set",
            dict([("workspace", s("Other")), ("profile", s("absent"))])
        )
        .code,
        "not_found"
    );
    assert_eq!(f.run("context.current", dict::<&str>([])), before);
    f.run("context.set", dict([("workspace", s("Other"))]));
    assert_eq!(
        text(&f.run("context.show", dict::<&str>([])), "profile_id"),
        ""
    );
}
#[test]
fn accounts_seed_only_treasury_and_normalize_asset_hints() {
    let f = Fixture::seed();
    let Value::List(a) = f.run("accounts.list", dict::<&str>([])) else {
        panic!()
    };
    assert_eq!(a.len(), 1);
    assert_eq!(text(&a[0], "code"), "treasury");
    let a = f.run(
        "accounts.create",
        dict([
            ("code", s("  Saving Bucket  ")),
            ("label", s("Save")),
            ("type", s("ASSET")),
            ("asset", s("l-btc")),
        ]),
    );
    assert_eq!(text(&a, "code"), "saving-bucket");
    assert_eq!(text(&a, "asset"), "LBTC");
    assert_eq!(
        f.fail(
            "accounts.create",
            dict([
                ("code", s("saving-bucket")),
                ("label", s("Save")),
                ("type", s("asset"))
            ])
        )
        .code,
        "conflict"
    );
    assert_eq!(
        f.fail(
            "accounts.create",
            dict([
                ("code", s("bad")),
                ("label", s("Save")),
                ("type", s(" asset "))
            ])
        )
        .code,
        "app_error"
    );
    // Malformed persisted text must fail closed, never become a replacement label.
    f.book
        .connection()
        .execute("UPDATE workspaces SET label=CAST(x'80' AS TEXT)", [])
        .unwrap();
    assert_eq!(
        f.fail("workspaces.list", dict::<&str>([])).code,
        "internal_error"
    );
}
