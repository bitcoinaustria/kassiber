#[allow(dead_code)]
mod support;
use kassiber_journals::prebuild::refresh_drifted_legacy_pair_components;
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!("data/prebuild-legacy.json")).unwrap()
}

fn expand(v: &Value, case: &Value, side: &str) -> Value {
    let mut dump = v["base"].clone();
    for (table, rows) in case[side].as_object().unwrap() {
        dump[table]["rows"] = rows.clone();
    }
    dump
}

fn profile(c: &Connection) -> Value {
    c.query_row(
        "SELECT json_object('id',id,'workspace_id',workspace_id) FROM profiles WHERE id='p'",
        [],
        |r| r.get::<_, String>(0),
    )
    .map(|s| serde_json::from_str(&s).unwrap())
    .unwrap()
}

fn check(names: &[&str]) {
    let v = vectors();
    let mut count = 0;
    for case in v["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if !names.is_empty() && !names.contains(&name) {
            continue;
        }
        count += 1;
        let c = Connection::open_in_memory().unwrap();
        let before = expand(&v, case, "before");
        support::load(&c, &before);
        c.execute_batch(
            "BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller-write','retained')",
        )
        .unwrap();
        let mut ticks = case["ticks"].as_array().unwrap().iter();
        let mut clock = || {
            Ok(ticks
                .next()
                .expect("legacy clock tape")
                .as_str()
                .unwrap()
                .to_owned())
        };
        let result = refresh_drifted_legacy_pair_components(&c, &case["profile"], &mut clock);
        if case["error"].is_null() {
            let result = result.unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(
                json!({"consolidation": {"activated":result.consolidation.activated,"unchanged":result.consolidation.unchanged,"skipped":result.consolidation.skipped},"profile":result.profile}),
                case["result"],
                "{name}"
            );
        } else {
            assert_eq!(
                serde_json::to_value(result.unwrap_err()).unwrap(),
                case["error"],
                "{name}"
            );
        }
        assert!(ticks.next().is_none(), "{name}: unused clock tick");
        assert_eq!(
            c.query_row(
                "SELECT value FROM settings WHERE key='caller-write'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "retained"
        );
        c.execute("DELETE FROM settings WHERE key='caller-write'", [])
            .unwrap();
        let actual = support::snapshot(&c);
        for (table, expected) in expand(&v, case, "expected").as_object().unwrap() {
            assert!(
                support::logical_equal(&actual[table], expected),
                "{name}: {table}: {} != {expected}",
                actual[table]
            );
        }
        c.execute_batch("ROLLBACK").unwrap();
        assert!(
            support::logical_equal(&support::snapshot(&c), &before),
            "{name}: caller rollback"
        );
    }
    assert!(count > 0);
    if !names.is_empty() {
        assert_eq!(count, names.len());
    }
}

#[test]
fn prebuild_legacy_full_python_dumps_and_refusals() {
    check(&[]);
}

#[test]
fn prebuild_legacy_repeated_drift_retargets_each_immutable_revision() {
    check(&["repeated-drift", "repeated-drift-refusal"]);
}

#[test]
fn prebuild_legacy_qualification() {
    check(&[
        "no-pairs",
        "unlinked",
        "unchanged",
        "authored-user",
        "reviewed-evidence",
        "superseded",
        "draft",
        "deleted",
        "fee-invalid",
        "amount-invalid",
        "excluded",
        "asset-drift",
    ]);
}

#[test]
fn prebuild_legacy_connected_groups_and_issues() {
    check(&[
        "connected-split",
        "connected-conversion",
        "mixed-linked-group",
        "unlinked-neighbor",
        "two-groups",
        "resolved-issue",
        "repeated-issue",
        "changing-clock",
    ]);
}

#[test]
fn prebuild_legacy_domain_refusal_restores_partial_activation() {
    check(&[
        "membership-refusal",
        "draft-exists",
        "invalid-created-at",
        "empty-created-at",
        "offset-created-at",
    ]);
}

fn fixture(name: &str) -> Connection {
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap();
    let c = Connection::open_in_memory().unwrap();
    support::load(&c, &expand(&v, case, "before"));
    c
}

#[test]
fn prebuild_legacy_reloads_profile_on_activated_and_skipped() {
    for name in ["kind-drift", "invalid-reviewed-amount"] {
        let c = fixture(name);
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let result = refresh_drifted_legacy_pair_components(&c, &profile(&c), &mut || {
            Ok("2026-10-31T12:00:00Z".into())
        })
        .unwrap();
        assert!(result.consolidation.changed());
        assert!(
            result.profile.get("journal_input_version").is_some(),
            "{name}: profile was not reloaded"
        );
        assert_eq!(
            result.profile["journal_input_version"].as_i64().unwrap(),
            c.query_row(
                "SELECT journal_input_version FROM profiles WHERE id='p'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
        );
    }
}

#[test]
fn prebuild_legacy_refusal_and_unwind_restore_every_write() {
    for panic in [false, true] {
        let c = fixture("two-groups");
        assert!(
            refresh_drifted_legacy_pair_components(&c, &profile(&c), &mut || unreachable!())
                .is_err()
        );
        c.execute_batch(
            "BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller','retained')",
        )
        .unwrap();
        let before = support::snapshot(&c);
        let mut calls = 0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            refresh_drifted_legacy_pair_components(&c, &profile(&c), &mut || {
                calls += 1;
                if calls == 2 {
                    assert!(!panic, "injected legacy clock panic");
                    return Err("injected legacy clock refusal".to_owned().into());
                }
                Ok("2026-10-31T12:00:00Z".into())
            })
        }));
        if panic {
            assert!(result.is_err());
        } else {
            assert_eq!(
                result.unwrap().unwrap_err().message,
                "injected legacy clock refusal"
            );
        }
        assert!(support::logical_equal(&support::snapshot(&c), &before));
        assert!(!c.is_autocommit());
    }
}

#[test]
fn prebuild_legacy_sql_failure_propagates_and_rolls_back() {
    let c = fixture("two-groups");
    c.execute_batch("CREATE TRIGGER late_legacy_failure BEFORE UPDATE OF component_id ON transaction_pairs WHEN NEW.id='pair-two' BEGIN SELECT RAISE(ABORT,'late legacy SQL failure');END;BEGIN IMMEDIATE").unwrap();
    let before = support::snapshot(&c);
    let error = refresh_drifted_legacy_pair_components(&c, &profile(&c), &mut || {
        Ok("2026-10-31T12:00:00Z".into())
    })
    .unwrap_err();
    assert_eq!(error.message, "late legacy SQL failure");
    assert_eq!(error.family, "IntegrityError");
    assert_eq!(error.code, None);
    assert!(support::logical_equal(&support::snapshot(&c), &before));
    assert!(!c.is_autocommit());
}

#[test]
fn prebuild_legacy_wide_and_malformed_reviewed_amounts() {
    check(&[
        "reviewed-null",
        "reviewed-empty",
        "reviewed-invalid-text",
        "reviewed-fraction-text",
        "reviewed-exponent-text",
        "reviewed-unicode-text",
        "reviewed-i128-text",
        "reviewed-negative-i128-text",
        "reviewed-max-digits",
        "reviewed-too-many-digits",
        "reviewed-leading-zero-limit",
        "reviewed-real-negative",
        "reviewed-below-i128-text",
        "two-groups-invalid-text",
        "two-groups-oversized",
        "missing-outbound",
        "missing-inbound",
        "missing-commitments",
        "resolved-issue-recurs",
    ]);
}

#[test]
fn prebuild_legacy_missing_scope_refuses_before_callbacks_or_writes() {
    for field in ["id", "workspace_id"] {
        let c = fixture("two-groups");
        let mut malformed = profile(&c);
        malformed.as_object_mut().unwrap().remove(field);
        c.execute_batch(
            "BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller','retained')",
        )
        .unwrap();
        let before = support::snapshot(&c);
        let error = refresh_drifted_legacy_pair_components(&c, &malformed, &mut || {
            panic!("invalid scope must not consume a clock tick")
        })
        .unwrap_err();
        assert_eq!(error.family, "KeyError");
        assert_eq!(error.message, field);
        assert!(support::logical_equal(&support::snapshot(&c), &before));
        assert!(!c.is_autocommit());
    }
}

#[test]
fn prebuild_legacy_profile_reload_refusal_restores_callback_writes() {
    let c = fixture("kind-drift");
    c.execute(
        "INSERT INTO workspaces(id,label,created_at) SELECT 'replacement','Replacement workspace',created_at FROM workspaces WHERE id='w'",
        [],
    )
    .unwrap();
    let scope = profile(&c);
    c.execute_batch("BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller','retained')")
        .unwrap();
    let before = support::snapshot(&c);
    let mut ticks = 0;
    let error = refresh_drifted_legacy_pair_components(&c, &scope, &mut || {
        ticks += 1;
        if ticks == 1 {
            c.execute(
                "UPDATE profiles SET workspace_id='replacement' WHERE id='p'",
                [],
            )
            .unwrap();
        }
        Ok("2026-10-31T12:00:00Z".into())
    })
    .unwrap_err();
    assert_eq!(ticks, 1);
    assert_eq!(error.family, "RuntimeError");
    assert_eq!(
        error.message,
        "journal profile disappeared during legacy refresh"
    );
    assert!(support::logical_equal(&support::snapshot(&c), &before));
    assert!(!c.is_autocommit());
}

#[test]
fn prebuild_legacy_missing_table_is_an_operational_refusal() {
    let c = fixture("kind-drift");
    let scope = profile(&c);
    c.execute_batch("PRAGMA foreign_keys=OFF;DROP TABLE transaction_pairs;BEGIN IMMEDIATE")
        .unwrap();
    let before = support::snapshot(&c);
    let error = refresh_drifted_legacy_pair_components(&c, &scope, &mut || {
        panic!("missing table must not consume a clock tick")
    })
    .unwrap_err();
    assert_eq!(error.family, "OperationalError");
    assert!(error.message.contains("no such table"));
    assert!(support::logical_equal(&support::snapshot(&c), &before));
    assert!(!c.is_autocommit());
}
