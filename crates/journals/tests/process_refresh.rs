#![cfg(feature = "test-harness")]
#[allow(dead_code)]
mod support;
use kassiber_envelope::AppError;
use kassiber_journals::process::{self, RefreshOptions};
use kassiber_store::rusqlite::{Connection, TransactionState};
use kassiber_transfers::{Scope, Sources};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};

fn vectors() -> Value {
    serde_json::from_str(include_str!("data/process-refresh.json")).unwrap()
}
fn expand(vectors: &Value, case: &Value, side: &str) -> Value {
    let mut dump = vectors["base"].clone();
    for (table, rows) in case[side].as_object().unwrap() {
        dump[table]["rows"] = rows.clone();
    }
    dump
}
fn load(vectors: &Value, case: &Value) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    support::load(&c, &expand(vectors, case, "before"));
    for sql in case["schema_sql"].as_array().unwrap() {
        c.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    c
}
struct Tapes<'a> {
    case: &'a Value,
    ids: RefCell<std::slice::Iter<'a, Value>>,
    ticks: RefCell<std::slice::Iter<'a, Value>>,
    precise: Cell<usize>,
    panic_precise: Option<usize>,
}
impl<'a> Tapes<'a> {
    fn new(case: &'a Value) -> Self {
        Self {
            case,
            ids: RefCell::new(case["ids"].as_array().unwrap().iter()),
            ticks: RefCell::new(case["ticks"].as_array().unwrap().iter()),
            precise: Cell::new(0),
            panic_precise: None,
        }
    }
}
impl Sources for Tapes<'_> {
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        self.ids
            .borrow_mut()
            .next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::new("internal_error", "UUID tape exhausted"))
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        self.ticks
            .borrow_mut()
            .next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::new("internal_error", "clock tape exhausted"))
    }
    fn now_precise(&self, _: &Connection) -> Result<String, AppError> {
        self.precise.set(self.precise.get() + 1);
        assert_ne!(
            self.panic_precise,
            Some(self.precise.get()),
            "pairing cancelled"
        );
        Ok(self.case["clock"].as_str().unwrap().to_owned())
    }
}
fn scope(case: &Value) -> Scope<'_> {
    Scope {
        workspace_id: case["profile"]["workspace_id"].as_str().unwrap(),
        profile_id: case["profile"]["id"].as_str().unwrap(),
    }
}

fn settings(c: &Connection) -> (i64, i64) {
    (
        c.pragma_query_value(None, "cache_size", |r| r.get(0))
            .unwrap(),
        c.pragma_query_value(None, "wal_autocheckpoint", |r| r.get(0))
            .unwrap(),
    )
}

#[test]
fn refresh_matches_complete_python_books_ordered_results_phases_and_tapes() {
    let vectors = vectors();
    for case in vectors["cases"].as_array().unwrap() {
        let c = load(&vectors, case);
        let scope = scope(case);
        let sources = Tapes::new(case);
        let phases = RefCell::new(Vec::new());
        let result = process::refresh_native(
            &c,
            &scope,
            |c| {
                assert_eq!(
                    c.transaction_state(Some("main")).unwrap(),
                    TransactionState::Write
                );
                support::profile(c, scope.profile_id)
            },
            &sources,
            RefreshOptions {
                auto_pair: case["auto_pair"] == true,
                skip_rebuild_when_current: case["skip_rebuild_when_current"] == true,
                ..Default::default()
            },
            &mut |phase| {
                assert!(
                    c.is_autocommit(),
                    "freshness callback could commit pending pairs"
                );
                phases.borrow_mut().push(json!({"phase":phase}));
            },
            &mut |phase| phases.borrow_mut().push(json!({"phase":phase})),
            &mut (),
        );
        match result {
            Ok(result) => {
                assert!(
                    case["error"].is_null(),
                    "{}: expected refusal",
                    case["name"]
                );
                assert_eq!(result.payload, case["result"], "{}", case["name"]);
                assert_eq!(
                    kassiber_pyjson::dumps(&result.ordered_payload, &Default::default()).unwrap(),
                    case["payload_json"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
                result.after_reply().unwrap();
            }
            Err(error) => assert_eq!(
                serde_json::to_value(error).unwrap(),
                case["error"],
                "{}",
                case["name"]
            ),
        }
        let actual = support::snapshot(&c);
        let expected = expand(&vectors, case, "expected");
        for (table, expected) in expected.as_object().unwrap() {
            assert!(
                support::logical_equal(&actual[table], expected),
                "{} {table}: {} != {expected}",
                case["name"],
                actual[table]
            );
        }
        assert_eq!(
            *phases.borrow(),
            *case["phases"].as_array().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            sources.precise.get(),
            case["precise_reads"].as_u64().unwrap() as usize,
            "{}",
            case["name"]
        );
        assert!(
            sources.ids.borrow_mut().next().is_none(),
            "unused UUID: {}",
            case["name"]
        );
        assert!(
            sources.ticks.borrow_mut().next().is_none(),
            "unused clock: {}",
            case["name"]
        );
        assert!(c.is_autocommit());
    }
}

#[test]
fn refresh_service_resolves_explicit_scope_and_matches_pairing_books() {
    let vectors = vectors();
    for case in vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["auto_pair"] == true)
    {
        let conn = load(&vectors, case);
        let scope = scope(case);
        let sources = Tapes::new(case);
        let mut phases = Vec::new();
        let result = kassiber_journals::service::refresh_with_sources(
            &conn,
            Some(&kassiber_envelope::s(scope.workspace_id)),
            Some(&kassiber_envelope::s(scope.profile_id)),
            &sources,
            RefreshOptions {
                auto_pair: true,
                skip_rebuild_when_current: case["skip_rebuild_when_current"] == true,
                ..Default::default()
            },
            &mut |phase| phases.push(json!({"phase":phase})),
            &mut (),
        );
        match result {
            Ok(result) => {
                assert!(case["error"].is_null(), "{}", case["name"]);
                assert_eq!(
                    kassiber_pyjson::dumps(&result.ordered_payload, &Default::default()).unwrap(),
                    case["payload_json"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
                result.after_reply().unwrap();
            }
            Err(error) => assert_eq!(
                serde_json::to_value(error).unwrap(),
                case["error"],
                "{}",
                case["name"]
            ),
        }
        assert!(
            support::logical_equal(
                &support::snapshot(&conn),
                &expand(&vectors, case, "expected")
            ),
            "{}",
            case["name"]
        );
        assert_eq!(
            phases,
            *case["phases"].as_array().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            sources.precise.get(),
            case["precise_reads"].as_u64().unwrap() as usize
        );
        assert!(sources.ids.borrow_mut().next().is_none());
        assert!(sources.ticks.borrow_mut().next().is_none());
        assert!(conn.is_autocommit());
    }
}

#[test]
fn refresh_cancellation_rolls_back_pairs_and_keeps_only_the_first_commit() {
    let vectors = vectors();
    for current in [false, true] {
        let name = format!("exact-current{}-skip0", usize::from(current));
        let case = vectors["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        for cancel in ["preparing", "repairing", "pricing", "building", "storing"] {
            let c = load(&vectors, case);
            let sources = Tapes::new(case);
            let previous_settings = settings(&c);
            let scope = scope(case);
            let retained = RefCell::new(support::snapshot(&c));
            let first_complete = Cell::new(current);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                process::refresh_native(
                    &c,
                    &scope,
                    |c| support::profile(c, scope.profile_id),
                    &sources,
                    RefreshOptions {
                        auto_pair: true,
                        ..Default::default()
                    },
                    &mut |_| assert!(c.is_autocommit()),
                    &mut |phase| {
                        if phase == "complete" {
                            *retained.borrow_mut() = support::snapshot(&c);
                            first_complete.set(true);
                        } else if first_complete.get() {
                            assert_ne!(phase, cancel, "rebuild cancelled");
                        }
                    },
                    &mut (),
                )
                .map(|result| result.after_reply())
            }));
            assert!(result.is_err(), "{name}: {cancel}");
            assert!(c.is_autocommit());
            assert_eq!(settings(&c), previous_settings);
            assert!(
                support::logical_equal(&support::snapshot(&c), &retained.borrow()),
                "{name}: {cancel}"
            );
        }
    }
}

#[test]
fn refresh_replay_requires_rollback_of_the_whole_invocation() {
    let vectors = vectors();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/r4-13/tests");
    std::fs::create_dir_all(&root).unwrap();
    for cipher in [false, true] {
        for current in [false, true] {
            let name = format!("second-rebuild-failure-current{}", usize::from(current));
            let case = vectors["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|case| case["name"] == name)
                .unwrap();
            let path = root.join(format!(
                "refresh-replay-{}-{cipher}-{current}.sqlite3",
                std::process::id()
            ));
            let conn = Connection::open(&path).unwrap();
            if cipher {
                conn.pragma_update(None, "key", "synthetic-refresh-replay-only")
                    .unwrap();
            }
            conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
            support::load(&conn, &expand(&vectors, case, "before"));
            for sql in case["schema_sql"].as_array().unwrap() {
                conn.execute_batch(sql.as_str().unwrap()).unwrap();
            }
            let guard = kassiber_journals::service::ReplayGuard::capture(&conn);
            let scope = scope(case);
            let sources = Tapes::new(case);
            let result = process::refresh_native(
                &conn,
                &scope,
                |conn| support::profile(conn, scope.profile_id),
                &sources,
                RefreshOptions {
                    auto_pair: true,
                    ..Default::default()
                },
                &mut |_| {},
                &mut |_| {},
                &mut (),
            );
            assert!(result.is_err(), "{name}");
            drop(result);
            assert!(conn.is_autocommit());
            assert_eq!(guard.unchanged(&conn), current, "{name}, cipher={cipher}");
            assert!(support::logical_equal(
                &support::snapshot(&conn),
                &expand(&vectors, case, "expected"),
            ));
            conn.close().unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }
}

#[test]
fn refresh_pairing_panics_restore_the_entire_current_book() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "exact-current1-skip0")
        .unwrap();
    for precise in 1..=4 {
        let c = load(&vectors, case);
        let before = support::snapshot(&c);
        let previous_settings = settings(&c);
        let scope = scope(case);
        let mut sources = Tapes::new(case);
        sources.panic_precise = Some(precise);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process::refresh_native(
                &c,
                &scope,
                |c| support::profile(c, scope.profile_id),
                &sources,
                RefreshOptions {
                    auto_pair: true,
                    ..Default::default()
                },
                &mut |_| {},
                &mut |_| {},
                &mut (),
            )
            .map(|result| result.after_reply())
        }));
        assert!(result.is_err());
        assert!(c.is_autocommit());
        assert_eq!(settings(&c), previous_settings);
        assert!(support::logical_equal(&support::snapshot(&c), &before));
    }
}

#[test]
fn refresh_defers_wal_checkpoint_across_both_commits_and_restores_settings() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "exact-current0-skip0")
        .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/r4-13/tests");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("refresh-wal-{}.sqlite3", std::process::id()));
    let c = Connection::open(&path).unwrap();
    c.pragma_update(None, "journal_mode", "WAL").unwrap();
    support::load(&c, &expand(&vectors, case, "before"));
    c.pragma_update(None, "wal_autocheckpoint", 37).unwrap();
    c.pragma_update(None, "cache_size", 123).unwrap();
    let sources = Tapes::new(case);
    let scope = scope(case);
    let commits = Cell::new(0);
    let result = process::refresh_native(
        &c,
        &scope,
        |c| support::profile(c, scope.profile_id),
        &sources,
        RefreshOptions {
            auto_pair: true,
            ..Default::default()
        },
        &mut |_| assert_eq!(settings(&c), (123, 0)),
        &mut |phase| {
            assert_eq!(settings(&c), (-131072, 0));
            if phase == "complete" {
                assert!(c.is_autocommit());
                commits.set(commits.get() + 1);
            }
        },
        &mut (),
    )
    .unwrap();
    assert_eq!(commits.get(), 2);
    assert_eq!(settings(&c), (123, 0));
    assert!(support::logical_equal(
        &support::snapshot(&c),
        &expand(&vectors, case, "expected")
    ));
    result.after_reply().unwrap();
    assert_eq!(settings(&c), (123, 37));
    c.close().unwrap();
    std::fs::remove_file(path).unwrap();
}
