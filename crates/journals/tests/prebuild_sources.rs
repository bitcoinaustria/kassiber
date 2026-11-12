#[allow(dead_code)]
mod support;
use kassiber_journals::prebuild::{
    repair_journal_source_overlaps, source_overlap_result_fields, source_overlap_warning,
};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!("data/prebuild-sources.json")).unwrap()
}
fn expand(vectors: &Value, case: &Value, side: &str) -> Value {
    let mut dump = vectors["base"].clone();
    for (table, rows) in case[side].as_object().unwrap() {
        dump[table]["rows"] = rows.clone();
    }
    dump
}
fn check(names: &[&str]) {
    let vectors = vectors();
    let mut count = 0;
    for case in vectors["cases"].as_array().unwrap() {
        if !names.is_empty() && !names.contains(&case["name"].as_str().unwrap()) {
            continue;
        }
        count += 1;
        let before = expand(&vectors, case, "before");
        let expected = expand(&vectors, case, "expected");
        let conn = Connection::open_in_memory().unwrap();
        support::load(&conn, &before);
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        conn.execute(
            "INSERT INTO settings(key,value) VALUES('caller-write','retained')",
            [],
        )
        .unwrap();
        let mut ids = case["ids"].as_array().unwrap().iter();
        let mut ticks = case["ticks"].as_array().unwrap().iter();
        let mut clock = || {
            Ok(ticks
                .next()
                .expect("source audit clock tape")
                .as_str()
                .unwrap()
                .to_string())
        };
        // The oracle records persisted IDs only; a refused repair rolls its
        // audit rows back, so their IDs are never observable.
        let refused = !case["error"].is_null();
        let mut rolled_back = 0;
        let mut identity = || {
            Ok(match ids.next() {
                Some(id) => id.as_str().unwrap().to_string(),
                None if refused => {
                    rolled_back += 1;
                    format!("00000000-0000-4000-8000-{rolled_back:012}")
                }
                None => panic!("source audit ID tape"),
            })
        };
        let result = repair_journal_source_overlaps(&conn, "pf", &mut clock, &mut identity)
            .and_then(|repair| {
                let warning = source_overlap_warning(&conn, "pf", repair.as_ref())?;
                let fields = source_overlap_result_fields(repair.as_ref(), warning.as_ref())?;
                assert_eq!(
                    kassiber_pyjson::dumps(&fields, &kassiber_pyjson::Options::new()).unwrap(),
                    case["wire_fields"].as_str().unwrap(),
                    "{}: serialized result fields",
                    case["name"]
                );
                Ok(json!({"repair":repair,"warning":warning}))
            });
        if case["error"].is_null() {
            assert_eq!(result.unwrap(), case["result"], "{}", case["name"]);
        } else {
            assert_eq!(
                serde_json::to_value(result.unwrap_err()).unwrap(),
                case["error"],
                "{}",
                case["name"]
            );
        }
        assert!(ids.next().is_none(), "unused source audit ID");
        assert!(ticks.next().is_none(), "unused source audit clock tick");
        assert_eq!(
            conn.query_row(
                "SELECT value FROM settings WHERE key='caller-write'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "retained"
        );
        conn.execute("DELETE FROM settings WHERE key='caller-write'", [])
            .unwrap();
        let actual = support::snapshot(&conn);
        for (table, data) in expected.as_object().unwrap() {
            assert!(
                support::logical_equal(&actual[table], data),
                "{}: {table}: {} != {data}",
                case["name"],
                actual[table]
            );
        }
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
        assert!(
            support::logical_equal(&support::snapshot(&conn), &before),
            "{}",
            case["name"]
        );
    }
    assert!(count > 0);
    if !names.is_empty() {
        assert_eq!(count, names.len());
    }
}

#[test]
fn prebuild_sources_full_python_dumps_and_refusals() {
    check(&[]);
}
#[test]
fn prebuild_sources_audited_duplicates_and_trimming() {
    check(&[
        "duplicate",
        "multiple-duplicates",
        "trim-empty",
        "trim-partial",
        "outbound",
        "nested-raw",
        "excluded-duplicate",
        "trim-inventory-checkpoint",
        "address-map-order",
        "address-case",
        "script-hex",
        "script-pubkey-object",
        "script-pubkey-string",
        "changing-clock",
    ]);
}
#[test]
fn prebuild_sources_unresolved_and_review_conflicts() {
    check(&[
        "unresolved-amount",
        "unresolved-fee",
        "unresolved-direction",
        "unresolved-asset",
        "unresolved-raw",
        "unresolved-no-external-id",
        "review-conflict",
        "mixed-conflict",
        "address-only",
        "legacy-real-amount",
        "legacy-real-fee",
    ]);
}
#[test]
fn prebuild_sources_finite_evidence_and_scope() {
    check(&[
        "no-overlap",
        "inventory-overlap",
        "raw-inventory-script",
        "invalid-inventory-script",
        "different-network",
        "deprecated-inactive",
        "deprecated-active",
        "descriptor-horizon",
        "checkpoint-horizon",
        "inventory-horizon",
        "invalid-descriptor",
        "invalid-config",
        "invalid-address",
        "other-profile",
    ]);
}

#[test]
fn prebuild_sources_legacy_economic_inputs_and_refusals() {
    check(&[
        "legacy-real-amount",
        "legacy-real-fee",
        "invalid-economic-amount",
        "invalid-economic-fee",
        "economic-text-int",
        "invalid-deprecated",
        "invalid-deprecated-none",
    ]);
}

#[test]
fn prebuild_sources_requires_writer_and_rolls_back_refusal_or_unwind() {
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "multiple-duplicates")
        .unwrap();
    let before = expand(&v, case, "before");
    let conn = Connection::open_in_memory().unwrap();
    support::load(&conn, &before);
    assert!(repair_journal_source_overlaps(
        &conn,
        "pf",
        &mut || Ok("clock".to_string()),
        &mut || panic!("unexpected ID")
    )
    .is_err());
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    for (panic, fail_at) in [(false, 2), (true, 2), (false, 3), (true, 3)] {
        let mut n = 0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            repair_journal_source_overlaps(
                &conn,
                "pf",
                &mut || Ok("clock".to_string()),
                &mut || {
                    n += 1;
                    if n == fail_at {
                        assert!(!panic, "injected identity failure");
                        return Err("injected identity refusal".to_string().into());
                    }
                    Ok(format!("id-{n}"))
                },
            )
        }));
        assert_eq!(n, fail_at);
        if panic {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }
        assert!(support::logical_equal(&support::snapshot(&conn), &before));
    }
    conn.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn prebuild_sources_late_sql_refusal_restores_all_tables() {
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "trim-inventory-checkpoint")
        .unwrap();
    let before = expand(&v, case, "before");
    let conn = Connection::open_in_memory().unwrap();
    support::load(&conn, &before);
    conn.execute_batch("BEGIN IMMEDIATE; CREATE TEMP TRIGGER refuse_checkpoint BEFORE DELETE ON freshness_source_states BEGIN SELECT RAISE(ABORT,'late repair failure'); END").unwrap();
    let error =
        repair_journal_source_overlaps(&conn, "pf", &mut || Ok("clock".to_string()), &mut || {
            panic!("unexpected audit ID")
        })
        .unwrap_err();
    assert_eq!(error.message, "late repair failure");
    assert_eq!(error.family, "IntegrityError");
    assert!(support::logical_equal(&support::snapshot(&conn), &before));
    conn.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn prebuild_sources_clock_refusal_restores_earlier_audits() {
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "multiple-duplicates")
        .unwrap();
    let before = expand(&v, case, "before");
    let conn = Connection::open_in_memory().unwrap();
    support::load(&conn, &before);
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut ticks = 0;
    let mut ids = 0;
    let error = repair_journal_source_overlaps(
        &conn,
        "pf",
        &mut || {
            ticks += 1;
            if ticks == 2 {
                return Err("injected clock refusal".to_string().into());
            }
            Ok("clock".to_string())
        },
        &mut || {
            ids += 1;
            Ok(format!("id-{ids}"))
        },
    )
    .unwrap_err();
    assert_eq!(error.message, "injected clock refusal");
    assert_eq!(ticks, 2);
    assert_eq!(
        ids, 2,
        "sample each clock before allocating its audit identities"
    );
    assert!(support::logical_equal(&support::snapshot(&conn), &before));
    conn.execute_batch("ROLLBACK").unwrap();
}

/// The duplicate key keeps stored values (41978c2df): an infinite amount or fee
/// on one row keeps it distinct, and on both rows groups them, so the audited
/// exclusion is written before reading the row back refuses `int(inf)`.
#[test]
fn prebuild_sources_nonfinite_economics_stay_distinct_or_refuse_after_the_audit() {
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "duplicate")
        .unwrap();
    for field in ["amount", "fee"] {
        for value in [f64::INFINITY, f64::NEG_INFINITY] {
            let conn = Connection::open_in_memory().unwrap();
            support::load(&conn, &expand(&v, case, "before"));
            conn.execute(
                &format!("UPDATE transactions SET {field}=? WHERE id='exclude'"),
                [value],
            )
            .unwrap();
            let before = support::snapshot(&conn);
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            let repair = repair_journal_source_overlaps(
                &conn,
                "pf",
                &mut || panic!("a distinct row must not allocate a clock tick"),
                &mut || panic!("a distinct row must not allocate audit IDs"),
            )
            .unwrap()
            .unwrap();
            assert_eq!(repair["duplicates_excluded"], 0);
            assert!(support::logical_equal(&support::snapshot(&conn), &before));
            conn.execute_batch("ROLLBACK").unwrap();

            conn.execute(
                &format!("UPDATE transactions SET {field}=? WHERE id='keep'"),
                [value],
            )
            .unwrap();
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            let (mut ticks, mut ids) = (0, 0);
            let error = repair_journal_source_overlaps(
                &conn,
                "pf",
                &mut || {
                    ticks += 1;
                    Ok("2026-10-31T12:00:00Z".into())
                },
                &mut || {
                    ids += 1;
                    Ok(format!("00000000-0000-0000-0000-{ids:012}"))
                },
            )
            .unwrap_err();
            assert_eq!(error.family, "OverflowError");
            assert_eq!(error.message, "cannot convert float infinity to integer");
            assert_eq!((ticks, ids), (1, 2), "Python audits the exclusion first");
            let actual: f64 = conn
                .query_row(
                    &format!("SELECT {field} FROM transactions WHERE id='exclude'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(actual.to_bits(), value.to_bits());
            assert!(!conn.is_autocommit());
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }
}

#[test]
fn prebuild_sources_wide_economic_fingerprints() {
    check(&[
        "wide-economic-amount",
        "wide-economic-fee",
        "wide-economic-max-digits",
        "wide-economic-i128",
    ]);
}

#[test]
fn prebuild_sources_branch_input_witnesses() {
    check(&[
        "nested-scriptpubkey-fallback",
        "integer-outputs",
        "float-inputs",
        "canonical-without-observed-row",
        "all-canonical-observed-rows",
        "comma-separated-addresses",
        "boolean-addresses",
        "float-addresses",
        "blank-address-entries",
        "deprecated-false-text",
        "empty-wallet-label",
        "empty-wallet-kind",
        "unknown-chain-scope",
        "invalid-even-script",
        "inventory-without-scope",
        "checkpoint-boolean-integer",
        "checkpoint-real-integer",
        "checkpoint-negative-integer",
        "checkpoint-invalid-integer",
        "checkpoint-merged-inventory",
        "xpub-script-types",
        "xpub-without-script-types",
        "review-legacy-identity",
        "unknown-network-scope",
        "json-null-inventory-raw",
    ]);
}

#[test]
fn prebuild_sources_unknown_projection_refuses() {
    for invalid in [
        json!({"unexpected":true}),
        json!({"duplicates_excluded":0,"wallets_updated":[{"unexpected":true}]}),
    ] {
        for (repair, warning) in [(Some(&invalid), None), (None, Some(&invalid))] {
            let error = source_overlap_result_fields(repair, warning).unwrap_err();
            assert_eq!(error.family, "RuntimeError");
            assert_eq!(error.message, "unknown source-overlap result projection");
        }
    }
}
