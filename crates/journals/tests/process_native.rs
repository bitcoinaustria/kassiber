#![cfg(feature = "test-harness")]
mod support;
use kassiber_envelope::AppError;
use kassiber_journals::process;
use kassiber_store::rusqlite::{Connection, TransactionState};
use kassiber_transfers::Sources;
use serde_json::Value;
use std::cell::{Cell, RefCell};

fn vectors() -> Value {
    serde_json::from_str(include_str!("data/process-native.json")).unwrap()
}

fn expand(vectors: &Value, case: &Value, side: &str) -> Value {
    let mut result = vectors["base"].clone();
    for (table, rows) in case[side].as_object().unwrap() {
        result[table]["rows"] = rows.clone();
    }
    result
}

struct Tapes<'a> {
    case: &'a Value,
    ticks: RefCell<std::slice::Iter<'a, Value>>,
    ids: RefCell<std::slice::Iter<'a, Value>>,
    build_reads: Cell<usize>,
}
impl<'a> Tapes<'a> {
    fn new(case: &'a Value) -> Self {
        Self {
            case,
            ticks: RefCell::new(case["ticks"].as_array().unwrap().iter()),
            ids: RefCell::new(case["ids"].as_array().unwrap().iter()),
            build_reads: Cell::new(0),
        }
    }
}
impl Sources for Tapes<'_> {
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        self.ticks
            .borrow_mut()
            .next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::new("internal_error", "clock tape exhausted"))
    }
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        self.ids
            .borrow_mut()
            .next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::new("internal_error", "UUID tape exhausted"))
    }
    fn now_precise(&self, _: &Connection) -> Result<String, AppError> {
        self.build_reads.set(self.build_reads.get() + 1);
        Ok(self.case["clock"].as_str().unwrap().to_owned())
    }
}

#[test]
fn native_process_real_prebuild_matches_complete_python_books_and_lifecycle_tapes() {
    let vectors = vectors();
    for case in vectors["cases"].as_array().unwrap() {
        for scoped in [false, true] {
            let c = Connection::open_in_memory().unwrap();
            support::load(&c, &expand(&vectors, case, "before"));
            let sources = Tapes::new(case);
            let mut phases = Vec::new();
            let result = if scoped {
                kassiber_journals::service::process_with_sources(
                    &c,
                    Some(&kassiber_envelope::s(
                        case["profile"]["workspace_id"].as_str().unwrap(),
                    )),
                    Some(&kassiber_envelope::s(
                        case["profile"]["id"].as_str().unwrap(),
                    )),
                    &sources,
                    &mut |phase| phases.push(serde_json::json!({"phase":phase})),
                    &mut (),
                )
            } else {
                process::process_native(
                    &c,
                    |conn| {
                        assert_eq!(
                            conn.transaction_state(Some("main")).unwrap(),
                            TransactionState::Write
                        );
                        Ok(case["profile"].clone())
                    },
                    kassiber_pydecimal::Context::RP2,
                    &sources,
                    &mut |phase| phases.push(serde_json::json!({"phase":phase})),
                    &mut (),
                )
            };
            match result {
                Ok(result) => {
                    assert!(case["error"].is_null(), "{}", case["name"]);
                    assert_eq!(result.payload, case["result"], "{}", case["name"]);
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
            assert!(
                sources.ids.borrow_mut().next().is_none(),
                "unused identity: {}",
                case["name"]
            );
            assert!(
                sources.ticks.borrow_mut().next().is_none(),
                "unused clock: {}",
                case["name"]
            );
            assert_eq!(
                phases,
                *case["phases"].as_array().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                sources.build_reads.get(),
                usize::from(phases.iter().any(|p| p["phase"] == "building")),
                "{}",
                case["name"]
            );
            assert!(c.is_autocommit());
        }
    }
}

#[test]
fn native_process_storage_failure_reverts_real_source_legacy_and_price_writes() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "all-stages")
        .unwrap();
    for fail in [1, 8] {
        let c = Connection::open_in_memory().unwrap();
        let before = expand(&vectors, case, "before");
        support::load(&c, &before);
        let sources = Tapes::new(case);
        let mut reached_storage = false;
        let mut writes = support::Writes {
            fail_at: Some(fail),
            ..Default::default()
        };
        let result = process::process_native(
            &c,
            |_| Ok(case["profile"].clone()),
            kassiber_pydecimal::Context::RP2,
            &sources,
            &mut |phase| {
                if phase == "storing" {
                    reached_storage = true;
                    assert!(!support::logical_equal(&support::snapshot(&c), &before));
                }
            },
            &mut writes,
        );
        assert!(result.is_err());
        assert!(reached_storage);
        assert_eq!(writes.count, fail);
        assert!(support::logical_equal(&support::snapshot(&c), &before));
        assert!(c.is_autocommit());
    }
}

#[test]
fn native_process_uses_the_profile_returned_by_legacy_refresh() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "all-stages")
        .unwrap();
    let c = Connection::open_in_memory().unwrap();
    support::load(&c, &expand(&vectors, case, "before"));
    let sources = Tapes::new(case);
    let mut profile = case["profile"].clone();
    profile["label"] = Value::from("stale caller profile");
    let result = process::process_native(
        &c,
        |_| Ok(profile.clone()),
        kassiber_pydecimal::Context::RP2,
        &sources,
        &mut |_| {},
        &mut (),
    )
    .unwrap();
    assert_eq!(result.payload, case["result"]);
    result.after_reply().unwrap();
}

#[test]
fn native_process_late_clock_refusal_restores_real_prebuild_writes() {
    let vectors = vectors();
    let mut case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "all-stages")
        .unwrap()
        .clone();
    case["ticks"].as_array_mut().unwrap().pop();
    let c = Connection::open_in_memory().unwrap();
    let before = expand(&vectors, &case, "before");
    support::load(&c, &before);
    let sources = Tapes::new(&case);
    let mut phases = Vec::new();
    let result = process::process_native(
        &c,
        |_| Ok(case["profile"].clone()),
        kassiber_pydecimal::Context::RP2,
        &sources,
        &mut |phase| phases.push(phase),
        &mut (),
    );
    let Err(error) = result else {
        panic!("late clock refusal must abort");
    };
    assert_eq!(error.message, "clock tape exhausted");
    assert_eq!(phases.last(), Some(&"storing"));
    assert!(support::logical_equal(&support::snapshot(&c), &before));
    assert!(c.is_autocommit());
}
