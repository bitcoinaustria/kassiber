#![cfg(feature = "test-harness")]
mod support;
use kassiber_journals::{
    process::{self, Prebuild, Repairs},
    Refusal,
};
use kassiber_store::rusqlite::{Connection, TransactionState};
use serde_json::Value;

fn cases() -> Vec<Value> {
    serde_json::from_str(include_str!("data/persistence.json")).unwrap()
}
struct Prepared {
    profile: Value,
    fail: Option<&'static str>,
    mutate: bool,
}
impl Prebuild for Prepared {
    fn resolve(&mut self, c: &Connection) -> Result<Value, Refusal> {
        assert_eq!(
            c.transaction_state(Some("main")).unwrap(),
            TransactionState::Write
        );
        if self.fail == Some("resolve") {
            return Err("resolve refused".to_owned().into());
        }
        Ok(self.profile.clone())
    }
    fn repair(&mut self, c: &Connection, p: &mut Value) -> Result<Repairs, Refusal> {
        if self.mutate {
            c.execute("UPDATE profiles SET label='repaired' WHERE id='p'", [])
                .unwrap();
            p["label"] = Value::from("repaired");
        }
        if self.fail == Some("repair") {
            return Err("repair refused".to_owned().into());
        }
        Ok(Repairs::default())
    }
    fn price(&mut self, c: &Connection, _: &Value) -> Result<usize, Refusal> {
        if self.mutate {
            c.execute("UPDATE transactions SET note='priced'", [])
                .unwrap();
        }
        if self.fail == Some("price") {
            return Err("pricing refused".to_owned().into());
        }
        Ok(0)
    }
}
fn prepared(v: &Value) -> Prepared {
    Prepared {
        profile: v["record"]["profile"].clone(),
        fail: None,
        mutate: false,
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
fn process_complete_dumps_match_python_from_raw_books() {
    for v in cases() {
        let c = Connection::open_in_memory().unwrap();
        support::load(&c, &v["before"]);
        let old = settings(&c);
        let mut ids = v["ids"].as_array().unwrap().iter();
        let mut phases = Vec::new();
        let result = process::process(
            &c,
            &mut prepared(&v),
            v["created_at"].as_str().unwrap(),
            &mut || Ok(ids.next().expect("UUID tape").as_str().unwrap().to_owned()),
            &mut |phase| phases.push(phase),
            &mut (),
        )
        .unwrap();
        assert!(ids.next().is_none(), "unused UUID for {}", v["name"]);
        assert!(c.is_autocommit());
        assert_eq!(
            phases,
            [
                "writer_wait",
                "preparing",
                "repairing",
                "pricing",
                "building",
                "storing",
                "complete"
            ]
        );
        let actual = support::snapshot(&c);
        for (table, expected) in v["expected"].as_object().unwrap() {
            assert!(
                support::logical_equal(&actual[table], expected),
                "{} table {table}:\nactual {}\nexpected {expected}",
                v["name"],
                actual[table]
            );
        }
        assert_eq!(
            actual.as_object().unwrap().len(),
            v["expected"].as_object().unwrap().len()
        );
        let columns = v["expected"]["profiles"]["columns"].as_array().unwrap();
        let at = columns
            .iter()
            .position(|c| c == "last_processed_at")
            .unwrap();
        assert_eq!(
            result.payload["processed_at"],
            v["expected"]["profiles"]["rows"][0][at]
        );
        assert_eq!(result.payload["profile"], v["record"]["profile"]["label"]);
        assert!(result.payload["custody_quantity"]["economic_relations"].is_number());
        result.after_reply().unwrap();
        assert_eq!(settings(&c), old);
    }
}

#[test]
fn process_prebuild_refusals_and_phase_panics_restore_the_entire_book() {
    let v = &cases()[0];
    for fail in [
        "resolve",
        "repair",
        "price",
        "preparing",
        "repairing",
        "pricing",
        "building",
        "storing",
    ] {
        let c = Connection::open_in_memory().unwrap();
        support::load(&c, &v["before"]);
        let before = support::snapshot(&c);
        let old = settings(&c);
        let mut hooks = prepared(v);
        hooks.fail = Some(fail);
        hooks.mutate = true;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process::process(
                &c,
                &mut hooks,
                v["created_at"].as_str().unwrap(),
                &mut || Ok("unused".into()),
                &mut |phase| assert_ne!(phase, fail, "injected phase panic"),
                &mut (),
            )
        }));
        assert!(result.is_err() || result.unwrap().is_err(), "{fail}");
        assert!(
            support::logical_equal(&support::snapshot(&c), &before),
            "{fail}"
        );
        assert!(c.is_autocommit());
        assert_eq!(settings(&c), old);
    }
}

#[test]
fn process_persistence_failure_rolls_back_prebuild_and_freshness() {
    let v = &cases()[0];
    let c = Connection::open_in_memory().unwrap();
    support::load(&c, &v["before"]);
    let before = support::snapshot(&c);
    let old = settings(&c);
    let mut hooks = prepared(v);
    hooks.mutate = true;
    let mut phases = Vec::new();
    let mut writes = support::Writes {
        fail_at: Some(3),
        ..Default::default()
    };
    let result = process::process(
        &c,
        &mut hooks,
        v["created_at"].as_str().unwrap(),
        &mut || Ok("unused".into()),
        &mut |phase| phases.push(phase),
        &mut writes,
    );
    assert!(result.is_err());
    assert_eq!(writes.count, 3);
    assert!(!phases.contains(&"complete"));
    assert!(support::logical_equal(&support::snapshot(&c), &before));
    assert!(c.is_autocommit());
    assert_eq!(settings(&c), old);
}

#[test]
fn process_commits_repairs_and_pricing_with_the_projection() {
    let v = &cases()[0];
    let c = Connection::open_in_memory().unwrap();
    support::load(&c, &v["before"]);
    let mut hooks = prepared(v);
    hooks.mutate = true;
    let mut ids = v["ids"].as_array().unwrap().iter();
    let result = process::process(
        &c,
        &mut hooks,
        v["created_at"].as_str().unwrap(),
        &mut || Ok(ids.next().expect("UUID tape").as_str().unwrap().to_owned()),
        &mut |_| {},
        &mut (),
    )
    .unwrap();
    assert_eq!(result.payload["profile"], "repaired");
    let label: String = c
        .query_row("SELECT label FROM profiles WHERE id='p'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(label, "repaired");
    let unpriced: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE note IS NOT 'priced'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unpriced, 0);
    assert!(c.is_autocommit());
    result.after_reply().unwrap();
}

#[test]
fn process_refuses_tax_policy_before_prebuild_and_rolls_back_fusion_refusals() {
    let v = &cases()[0];
    for tax in [true, false] {
        let c = Connection::open_in_memory().unwrap();
        support::load(&c, &v["before"]);
        let before = support::snapshot(&c);
        let mut hooks = prepared(v);
        hooks.mutate = true;
        if tax {
            hooks.profile["tax_country"] = Value::from("unsupported");
        } else {
            hooks.profile["cost_basis_pool_scope"] = Value::from("wallet");
        }
        let mut phases = Vec::new();
        let result = process::process(
            &c,
            &mut hooks,
            v["created_at"].as_str().unwrap(),
            &mut || Ok("unused".into()),
            &mut |p| phases.push(p),
            &mut (),
        );
        let error = result.err().expect("policy refused");
        if tax {
            assert_eq!(error.family, "ValueError");
            assert_eq!(phases, ["writer_wait", "preparing"]);
        } else {
            assert_eq!(error.family, "AppError");
            assert_eq!(
                phases,
                [
                    "writer_wait",
                    "preparing",
                    "repairing",
                    "pricing",
                    "building"
                ]
            );
        }
        assert!(c.is_autocommit());
        assert!(support::logical_equal(&support::snapshot(&c), &before));
    }
}

#[test]
fn process_missing_prebuild_and_nested_transactions_fail_closed() {
    let c = Connection::open_in_memory().unwrap();
    let mut phases = Vec::new();
    let result = process::process(
        &c,
        &mut process::MissingPrebuild,
        "clock",
        &mut || Ok("unused".into()),
        &mut |p| phases.push(p),
        &mut (),
    );
    assert_eq!(result.err().unwrap().family, "NotImplementedError");
    assert!(c.is_autocommit());
    assert_eq!(phases, ["writer_wait", "preparing"]);
    c.execute_batch("BEGIN").unwrap();
    phases.clear();
    let result = process::process(
        &c,
        &mut process::MissingPrebuild,
        "clock",
        &mut || Ok("unused".into()),
        &mut |p| phases.push(p),
        &mut (),
    );
    assert_eq!(result.err().unwrap().family, "RuntimeError");
    assert!(phases.is_empty());
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
}
