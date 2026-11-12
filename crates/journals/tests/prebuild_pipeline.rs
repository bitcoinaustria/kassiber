mod support;
use kassiber_journals::{
    boundary, persistence,
    prebuild::{
        auto_price_transactions_from_rates_cache, prepare_journals, repair_journal_inputs,
        with_prebuild, Prepared, PricingScope,
    },
    Refusal,
};
use kassiber_pydecimal::{Context, Rounding};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!("data/prebuild-pipeline.json")).unwrap()
}
fn expand(v: &Value, case: &Value, side: &str) -> Value {
    let mut dump = v["base"].clone();
    for (table, rows) in case[side].as_object().unwrap() {
        dump[table]["rows"] = rows.clone();
    }
    dump
}
fn context() -> Context {
    Context::new(32, Rounding::HalfEven)
}
fn projection(p: &Prepared) -> Value {
    let r = &p.repaired;
    json!({"repair":r.source_overlap_repair,"warning":r.source_overlap_warning,
        "consolidation":{"activated":r.consolidation.activated,"unchanged":r.consolidation.unchanged,"skipped":r.consolidation.skipped},
        "profile":r.profile,"auto_priced":p.auto_priced})
}

#[test]
fn prebuild_pipeline_matches_complete_python_dumps_and_ordered_fields() {
    check_pipeline(vectors());
}

#[test]
fn prebuild_campaign_matches_stored_generator_scenarios() {
    check_pipeline(serde_json::from_str(include_str!("data/prebuild-campaign.json")).unwrap());
}

fn check_pipeline(v: Value) {
    for case in v["cases"].as_array().unwrap() {
        let c = Connection::open_in_memory().unwrap();
        let before = expand(&v, case, "before");
        support::load(&c, &before);
        c.execute_batch(
            "BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller','retained')",
        )
        .unwrap();
        let mut ticks = case["ticks"].as_array().unwrap().iter();
        let mut ids = case["ids"].as_array().unwrap().iter();
        let result = prepare_journals(
            &c,
            &case["profile"],
            context(),
            &mut || {
                Ok(ticks
                    .next()
                    .expect("pipeline clock tape")
                    .as_str()
                    .unwrap()
                    .to_owned())
            },
            &mut || {
                Ok(ids
                    .next()
                    .expect("pipeline identity tape")
                    .as_str()
                    .unwrap()
                    .to_owned())
            },
        );
        if case["error"].is_null() {
            let p = result.unwrap_or_else(|e| panic!("{}: {e:?}", case["name"]));
            assert_eq!(projection(&p), case["result"], "{}", case["name"]);
            assert_eq!(
                kassiber_pyjson::dumps(
                    &p.repaired.result_fields().unwrap(),
                    &kassiber_pyjson::Options::new()
                )
                .unwrap(),
                case["wire_fields"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        } else {
            assert_eq!(
                serde_json::to_value(result.unwrap_err()).unwrap(),
                case["error"],
                "{}",
                case["name"]
            );
        }
        assert!(ticks.next().is_none(), "{}: unused clock", case["name"]);
        assert!(ids.next().is_none(), "{}: unused ID", case["name"]);
        assert_eq!(
            c.query_row("SELECT value FROM settings WHERE key='caller'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "retained"
        );
        c.execute("DELETE FROM settings WHERE key='caller'", [])
            .unwrap();
        let actual = support::snapshot(&c);
        let expected = expand(&v, case, "expected");
        for (table, data) in expected.as_object().unwrap() {
            assert!(
                support::logical_equal(&actual[table], data),
                "{}: {table}: {} != {data}",
                case["name"],
                actual[table]
            );
        }
        c.execute_batch("ROLLBACK").unwrap();
        assert!(support::logical_equal(&support::snapshot(&c), &before));
    }
}

fn fixture() -> (Connection, Value) {
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "all-stages")
        .unwrap()
        .clone();
    let c = Connection::open_in_memory().unwrap();
    support::load(&c, &expand(&v, &case, "before"));
    (c, case)
}

fn run<T>(
    c: &Connection,
    case: &Value,
    continuation: impl FnOnce(&Prepared) -> Result<T, Refusal>,
) -> Result<(Prepared, T), Refusal> {
    let mut ticks = case["ticks"].as_array().unwrap().iter();
    let mut ids = case["ids"].as_array().unwrap().iter();
    with_prebuild(
        c,
        &case["profile"],
        context(),
        &mut || Ok(ticks.next().unwrap().as_str().unwrap().to_owned()),
        &mut || Ok(ids.next().unwrap().as_str().unwrap().to_owned()),
        continuation,
    )
}

// A real rendered acquisition on the repaired book exercises entry persistence
// as well as profile/checkpoint writes. No parallel accounting implementation.
fn custody() -> Value {
    json!({"finalized_tax_projection":{"rows":[{"id":"legacy-in","occurred_at":"2026-10-31T12:00:00Z"}],"quarantines":[],"cross_asset_pairs":[]},
        "component_blockers":[],"quantity_state":null,"gap_ignored_transaction_ids":[],"gap_accounting_ignored_transaction_ids":[]})
}
fn response() -> kassiber_tax_core::ledger::response::LedgerResponse {
    boundary::ledger_response(&json!({"ok":true,"carried_rows":[],"intra_audit":[],"holdings":[],"quarantines":[],"tax_summary":[],"entries":[{"entry_type":"acquisition","row":"legacy-in","wallet":"Legacy","asset":"BTC","quantity":"9E-9","fiat_value":"36E-5","unit_cost":"40000E0","description":"pipeline rollback witness"}]})).unwrap()
}
fn store(
    c: &Connection,
    p: &Prepared,
    writes: &mut support::Writes,
) -> Result<persistence::Stored, Refusal> {
    let custody = custody();
    let wallets = [json!({"id":"legacy-wallet","label":"Legacy","wallet_account_id":null})];
    let response = response();
    let state = boundary::build(
        &p.repaired.profile,
        &custody,
        &wallets,
        &response,
        String::new,
    )?;
    persistence::store_ledger_state(
        c,
        &p.repaired.profile,
        &state,
        "2026-10-31T13:00:00Z",
        &mut || unreachable!("no random IDs for this acquisition"),
        writes,
    )
}

#[test]
fn prebuild_pipeline_rolls_back_after_every_later_storage_failure_and_panic() {
    let (c, case) = fixture();
    c.execute_batch("BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller','retained')")
        .unwrap();
    let before = support::snapshot(&c);
    let mut success = support::Writes::default();
    let result = run(&c, &case, |p| store(&c, p, &mut success)).unwrap();
    assert!(result.0.auto_priced > 0);
    assert!(result.0.repaired.source_overlap_repair.is_some());
    assert_eq!(result.0.repaired.consolidation.activated, 1);
    assert!(success.count > 7);
    assert_eq!(
        c.query_row("SELECT count(*) FROM journal_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    c.execute_batch(
        "ROLLBACK;BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller','retained')",
    )
    .unwrap();
    for fail in 1..=success.count {
        for panic in [false, true] {
            let mut writes = support::Writes {
                fail_at: (!panic).then_some(fail),
                panic_at: panic.then_some(fail),
                ..Default::default()
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(&c, &case, |p| store(&c, p, &mut writes))
            }));
            assert!(
                matches!(result, Ok(Err(_)) | Err(_)),
                "write {fail}, panic {panic}"
            );
            assert!(
                support::logical_equal(&support::snapshot(&c), &before),
                "write {fail}, panic {panic}"
            );
            assert!(!c.is_autocommit());
        }
    }
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn prebuild_pipeline_rolls_back_after_actual_build_refusal() {
    let (c, case) = fixture();
    c.execute_batch("BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller','retained')")
        .unwrap();
    let before = support::snapshot(&c);
    let error = run(&c, &case, |p| {
        let mut custody = custody();
        custody["finalized_tax_projection"]["rows"] = json!([]);
        let wallets = [json!({"id":"legacy-wallet","label":"Legacy","wallet_account_id":null})];
        boundary::build(
            &p.repaired.profile,
            &custody,
            &wallets,
            &response(),
            String::new,
        )
        .map(|_| ())
    })
    .unwrap_err();
    assert_eq!(error.family, "TypeError", "{error:?}");
    assert_eq!(error.message, "'NoneType' object is not subscriptable");
    assert!(support::logical_equal(&support::snapshot(&c), &before));
    assert!(!c.is_autocommit());
}

#[test]
fn prebuild_pipeline_split_repairs_restore_sources_after_legacy_refusal() {
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "legacy-refusal")
        .unwrap();
    let c = Connection::open_in_memory().unwrap();
    let before = expand(&v, case, "before");
    support::load(&c, &before);
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut ids = case["ids"].as_array().unwrap().iter();
    let mut ticks = case["ticks"].as_array().unwrap().iter();
    assert!(repair_journal_inputs(
        &c,
        &case["profile"],
        &mut || Ok(ticks.next().unwrap().as_str().unwrap().to_owned()),
        &mut || Ok(ids.next().unwrap().as_str().unwrap().to_owned())
    )
    .is_err());
    assert!(support::logical_equal(&support::snapshot(&c), &before));
}

#[test]
fn prebuild_pipeline_split_phases_match_combined_order_and_reloaded_profile() {
    let (c, case) = fixture();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut ticks = case["ticks"].as_array().unwrap().iter();
    let mut ids = case["ids"].as_array().unwrap().iter();
    let repaired = repair_journal_inputs(
        &c,
        &case["profile"],
        &mut || Ok(ticks.next().unwrap().as_str().unwrap().to_owned()),
        &mut || Ok(ids.next().unwrap().as_str().unwrap().to_owned()),
    )
    .unwrap();
    let auto_priced = auto_price_transactions_from_rates_cache(
        &c,
        &PricingScope {
            profile_id: repaired.profile["id"].as_str().unwrap(),
            fiat_currency: repaired.profile["fiat_currency"].as_str().unwrap(),
            decimal_context: context(),
        },
    )
    .unwrap();
    let prepared = Prepared {
        repaired,
        auto_priced,
    };
    assert_eq!(projection(&prepared), case["result"]);
    assert!(ticks.next().is_none());
    assert!(ids.next().is_none());
    let v = vectors();
    assert!(support::logical_equal(
        &support::snapshot(&c),
        &expand(&v, &case, "expected")
    ));
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn prebuild_pipeline_requires_a_reserved_writer_before_callbacks_or_continuation() {
    let (c, case) = fixture();
    for deferred in [false, true] {
        if deferred {
            c.execute_batch("BEGIN;SELECT * FROM profiles").unwrap();
        }
        let results = [
            repair_journal_inputs(
                &c,
                &case["profile"],
                &mut || unreachable!(),
                &mut || unreachable!(),
            )
            .map(|_| ()),
            prepare_journals(
                &c,
                &case["profile"],
                context(),
                &mut || unreachable!(),
                &mut || unreachable!(),
            )
            .map(|_| ()),
            with_prebuild::<()>(
                &c,
                &case["profile"],
                context(),
                &mut || unreachable!(),
                &mut || unreachable!(),
                |_| unreachable!("no continuation without writer"),
            )
            .map(|_| ()),
        ];
        for result in results {
            assert_eq!(result.unwrap_err().family, "RuntimeError");
        }
        if deferred {
            c.execute_batch("ROLLBACK").unwrap();
        }
    }
}

#[test]
fn prebuild_pipeline_missing_profile_fields_refuse_atomically() {
    let v = vectors();
    let empty = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "empty")
        .unwrap();
    for field in ["id", "workspace_id", "fiat_currency"] {
        let c = Connection::open_in_memory().unwrap();
        support::load(&c, &expand(&v, empty, "before"));
        let mut profile = empty["profile"].clone();
        profile.as_object_mut().unwrap().remove(field);
        c.execute_batch(
            "BEGIN IMMEDIATE;INSERT INTO settings(key,value) VALUES('caller','retained')",
        )
        .unwrap();
        let before = support::snapshot(&c);
        let error = prepare_journals(
            &c,
            &profile,
            context(),
            &mut || panic!("missing profile field must not consume a tick"),
            &mut || panic!("missing profile field must not consume an identity"),
        )
        .unwrap_err();
        assert_eq!(error.family, "KeyError");
        assert_eq!(error.message, field);
        assert!(support::logical_equal(&support::snapshot(&c), &before));
        assert!(!c.is_autocommit());
    }
}
