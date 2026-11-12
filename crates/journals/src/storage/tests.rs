use super::*;
use crate::{array, model::JournalMsat};
use std::sync::Arc;

fn msat(value: Value) -> JournalMsat {
    JournalMsat::Wide(Box::new(value))
}

fn same<const N: usize>(old: &[Sql; N], new: &[Sql; N], reals: &[usize]) -> bool {
    let c = Connection::open_in_memory().unwrap();
    let mut statement = c
        .prepare(&format!("SELECT {}", vec!["?"; N].join(",")))
        .unwrap();
    statement
        .query_row(params_from_iter(old), |row| same_row(row, new, reals))
        .unwrap()
}
fn table(conn: &Connection, name: &str, columns: &[&str]) {
    let columns = columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let kind = if name == "journal_entries" && (10..=14).contains(&i) {
                "REAL"
            } else {
                "TEXT"
            };
            format!("{c} {kind}{}", if i == 0 { " PRIMARY KEY" } else { "" })
        })
        .collect::<Vec<_>>()
        .join(",");
    conn.execute_batch(&format!("CREATE TABLE {name}({columns})"))
        .unwrap();
}
fn entry(id: &str, at: &str, created: &str) -> [Sql; 29] {
    let mut row = std::array::from_fn(|_| Sql::Null);
    for (i, s) in [
        (0, id),
        (1, "w"),
        (2, "p"),
        (6, at),
        (24, "description"),
        (28, created),
    ] {
        row[i] = Sql::Text(s.into());
    }
    row[10] = Sql::Real(0.0);
    row
}
fn snapshot<const N: usize>(c: &Connection, table: &str) -> Vec<(i64, [Sql; N])> {
    c.prepare(&format!("SELECT rowid,* FROM {table} ORDER BY 1"))
        .unwrap()
        .query_map([], |r| {
            let mut values = std::array::from_fn(|_| Sql::Null);
            for (i, v) in values.iter_mut().enumerate() {
                *v = r.get(i + 1)?;
            }
            Ok((r.get(0)?, values))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
#[test]
fn reused_insert_bindings_match_full_binds_for_every_scalar_transition() {
    let c = Connection::open_in_memory().unwrap();
    for table in ["journal_entries", "reference"] {
        c.execute_batch(&format!(
            "CREATE TABLE {table}({})",
            ENTRY_COLUMNS.join(",")
        ))
        .unwrap();
    }
    let values = [
        Sql::Null,
        Sql::Null,
        Sql::Integer(1),
        Sql::Integer(1),
        Sql::Real(1.0),
        Sql::Real(1.0),
        Sql::Real(0.0),
        Sql::Real(-0.0),
        Sql::Real(0.0),
        Sql::Blob(Vec::new()),
        Sql::Text(String::new()),
        Sql::Text("é🔥\0".into()),
        Sql::Text("é🔥\0".into()),
        Sql::Blob(b"bytes\0".to_vec()),
        Sql::Null,
    ];
    let rows: Vec<_> = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let mut row = entry(&format!("id-{index}"), "time", "created");
            row[9] = value;
            row
        })
        .collect();
    let mut reference = c
        .prepare(&format!(
            "INSERT INTO reference VALUES({})",
            vec!["?"; ENTRY_COLUMNS.len()].join(",")
        ))
        .unwrap();
    for row in &rows {
        reference.execute(params_from_iter(row)).unwrap();
    }
    replace_journal_entries(&c, "p", &rows, &mut ()).unwrap();
    let actual = snapshot::<29>(&c, "journal_entries");
    let expected = snapshot::<29>(&c, "reference");
    assert_eq!(actual, expected);
    for ((_, actual), (_, expected)) in actual.iter().zip(&expected) {
        if let (Sql::Real(actual), Sql::Real(expected)) = (&actual[9], &expected[9]) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }
}
#[test]
fn entry_deltas_rewrite_timestamp_groups_and_retain_untouched_rowids() {
    let c = Connection::open_in_memory().unwrap();
    table(&c, "journal_entries", &ENTRY_COLUMNS);
    let a = entry("a", "2024-01-01", "first");
    let b = entry("b", "2024-01-01", "first");
    let other = entry("other", "2024-02-01", "first");
    replace_journal_entries(&c, "p", &[a.clone(), b.clone(), other.clone()], &mut ()).unwrap();
    let before = snapshot::<29>(&c, "journal_entries");
    let mut same = vec![a.clone(), b.clone(), other.clone()];
    for r in &mut same {
        r[28] = Sql::Text("later".into());
    }
    replace_journal_entries(&c, "p", &same, &mut ()).unwrap();
    assert_eq!(snapshot::<29>(&c, "journal_entries"), before);
    same[0][24] = Sql::Text("changed".into());
    replace_journal_entries(&c, "p", &same, &mut ()).unwrap();
    let after = snapshot::<29>(&c, "journal_entries");
    assert_eq!(
        after.iter().find(|(_, r)| r[0] == other[0]).unwrap(),
        &before[2]
    );
    for (_, r) in &after {
        if r[6] == a[6] {
            assert_eq!(r[28], Sql::Text("later".into()));
        }
    }
    // Appended equal-time rows rewrite all siblings, in the new engine order.
    let new = entry("new", "2024-01-01", "third");
    for r in &mut same {
        r[28] = Sql::Text("third".into());
    }
    same.insert(0, new);
    replace_journal_entries(&c, "p", &same, &mut ()).unwrap();
    let ids: Vec<_> = snapshot::<29>(&c, "journal_entries")
        .into_iter()
        .filter(|(_, r)| r[6] == a[6])
        .map(|(_, r)| r[0].clone())
        .collect();
    assert_eq!(
        ids,
        vec![Sql::Text("new".into()), a[0].clone(), b[0].clone()]
    );
    // Removed entries are deleted; removal alone does not dirty the survivors.
    let before = snapshot::<29>(&c, "journal_entries");
    same.remove(1);
    replace_journal_entries(&c, "p", &same, &mut ()).unwrap();
    assert_eq!(
        snapshot::<29>(&c, "journal_entries"),
        before
            .into_iter()
            .filter(|(_, r)| r[0] != a[0])
            .collect::<Vec<_>>()
    );
}
#[test]
fn removing_a_mixed_clock_sibling_keeps_the_survivors_rowid_and_creation_time() {
    for (survivor, removed) in [("a", "z"), ("z", "a")] {
        let c = Connection::open_in_memory().unwrap();
        table(&c, "journal_entries", &ENTRY_COLUMNS);
        let a = entry(survivor, "same", "later-old-clock");
        let b = entry(removed, "same", "earlier-old-clock");
        replace_journal_entries(&c, "p", &[a.clone(), b], &mut ()).unwrap();
        let before = snapshot::<29>(&c, "journal_entries");
        let mut rebuilt = a.clone();
        rebuilt[28] = Sql::Text("new-build-clock".into());
        replace_journal_entries(&c, "p", &[rebuilt], &mut ()).unwrap();
        assert_eq!(
            snapshot::<29>(&c, "journal_entries"),
            before
                .into_iter()
                .filter(|(_, row)| row[0] == a[0])
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn mixed_timestamps_and_signed_zero_force_entry_rewrites() {
    let c = Connection::open_in_memory().unwrap();
    table(&c, "journal_entries", &ENTRY_COLUMNS);
    let a = entry("a", "same", "old-a");
    let b = entry("b", "same", "old-b");
    replace_journal_entries(&c, "p", &[a.clone(), b.clone()], &mut ()).unwrap();
    let mut new = [a, b];
    for r in &mut new {
        r[28] = Sql::Text("new".into());
    }
    replace_journal_entries(&c, "p", &new, &mut ()).unwrap();
    assert!(snapshot::<29>(&c, "journal_entries")
        .iter()
        .all(|(_, r)| r[28] == Sql::Text("new".into())));
    let mut negative = new[0].clone();
    negative[10] = Sql::Real(-0.0);
    assert!(!same(&new[0], &negative, &[10]));
    assert!(same(&new[0], &negative, &[]));
    let mut equal = new[0].clone();
    equal[28] = Sql::Text("ignored".into());
    assert!(same(&new[0], &equal, &[10]));
    for index in 0..28 {
        let mut different = new[0].clone();
        different[index] = Sql::Text("different".into());
        assert!(!same(&new[0], &different, &[10]), "{index}");
    }
}
#[test]
fn posting_deltas_reject_duplicates_and_keep_unchanged_rows() {
    let c = Connection::open_in_memory().unwrap();
    table(&c, "journal_quantity_postings", &POSTING_COLUMNS);
    let mut a = std::array::from_fn(|_| Sql::Null);
    a[0] = Sql::Text("a".into());
    a[2] = Sql::Text("p".into());
    a[11] = Sql::Text("first".into());
    let mut b = a.clone();
    b[0] = Sql::Text("b".into());
    replace_quantity_postings(&c, "p", &[a.clone(), b.clone()], &mut ()).unwrap();
    let before = snapshot::<12>(&c, "journal_quantity_postings");
    let mut next = a.clone();
    next[11] = Sql::Text("next".into());
    replace_quantity_postings(&c, "p", &[next.clone(), b.clone()], &mut ()).unwrap();
    assert_eq!(snapshot::<12>(&c, "journal_quantity_postings"), before);
    assert!(replace_quantity_postings(&c, "p", &[a.clone(), a], &mut ()).is_err());
    assert_eq!(snapshot::<12>(&c, "journal_quantity_postings"), before);
    next[9] = Sql::Text("changed".into());
    replace_quantity_postings(&c, "p", &[next.clone()], &mut ()).unwrap();
    assert_eq!(
        snapshot::<12>(&c, "journal_quantity_postings")
            .into_iter()
            .map(|(_, r)| r)
            .collect::<Vec<_>>(),
        vec![next]
    );
    assert!(replace_journal_entries(&c, "p", &[], &mut ()).is_err()); // Missing table is a storage error.
}

#[test]
fn posting_time_order_keeps_identity_when_times_move_or_are_null() {
    for nullable in [false, true] {
        let c = Connection::open_in_memory().unwrap();
        table(&c, "journal_quantity_postings", &POSTING_COLUMNS);
        let rows: Vec<[Sql; 12]> = [("z", "first"), ("m", "middle"), ("a", "last")]
            .into_iter()
            .map(|(id, time)| {
                let mut row = std::array::from_fn(|_| Sql::Null);
                row[0] = Sql::Text(id.into());
                row[2] = Sql::Text("p".into());
                row[5] = if nullable && id == "z" {
                    Sql::Null
                } else {
                    Sql::Text(time.into())
                };
                row[11] = Sql::Text("first-build".into());
                row
            })
            .collect();
        replace_quantity_postings(&c, "p", &rows, &mut ()).unwrap();
        let before = snapshot::<12>(&c, "journal_quantity_postings");
        let mut next = rows.clone();
        next.reverse();
        for row in &mut next {
            row[11] = Sql::Text("next-build".into());
        }
        replace_quantity_postings(&c, "p", &next, &mut ()).unwrap();
        assert_eq!(snapshot::<12>(&c, "journal_quantity_postings"), before);
        next[1][5] = Sql::Text("earliest".into());
        replace_quantity_postings(&c, "p", &next, &mut ()).unwrap();
        let after = snapshot::<12>(&c, "journal_quantity_postings");
        for retained in before.iter().filter(|(_, row)| row[0] != rows[1][0]) {
            assert!(after.contains(retained));
        }
        let changed = after.iter().find(|(_, row)| row[0] == rows[1][0]).unwrap();
        assert_eq!(changed.1, next[1]);
        next.retain(|row| row[0] != rows[0][0]);
        replace_quantity_postings(&c, "p", &next, &mut ()).unwrap();
        assert_eq!(
            snapshot::<12>(&c, "journal_quantity_postings"),
            after
                .into_iter()
                .filter(|(_, row)| row[0] != rows[0][0])
                .collect::<Vec<_>>()
        );
    }
}
#[test]
fn storage_encodings_and_markers_match_python() {
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/data/storage.json")).unwrap();
    for v in array(&vectors["ids"]) {
        assert_eq!(
            journal_entry_id(
                v[0].as_str().unwrap(),
                v[1].as_str().unwrap(),
                v[2].as_str().unwrap(),
                v[3].as_str().unwrap(),
                v[4].as_str().unwrap(),
                v[5].as_u64().unwrap() as usize
            )
            .unwrap(),
            v[6]
        );
    }
    for v in array(&vectors["calculations"]) {
        let (legacy, compact) = encode_calculation(&v[0]).unwrap();
        assert_eq!(json!([legacy, compact]), v[1], "{}", v[0]);
    }
    for v in array(&vectors["markers"]) {
        assert_eq!(
            journal_processed_at(v[0].as_str().unwrap(), v[1].as_str()).unwrap(),
            v[2]
        );
    }
}
#[test]
fn scoped_cache_and_checkpoint_restore_exact_settings() {
    let c = Connection::open_in_memory().unwrap();
    c.pragma_update(None, "cache_size", 73).unwrap();
    let get = || {
        c.pragma_query_value(None, "cache_size", |r| r.get::<_, i64>(0))
            .unwrap()
    };
    {
        let scope = RebuildCache::enter(&c).unwrap();
        assert_eq!(get(), -131072);
        scope.restore().unwrap();
    }
    assert_eq!(get(), 73);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = RebuildCache::enter(&c).unwrap();
        panic!("injected");
    }));
    assert!(caught.is_err());
    assert_eq!(get(), 73);
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(DeferredCheckpoint::enter(&c).is_err());
    c.execute_batch("ROLLBACK").unwrap();
    DeferredCheckpoint::enter(&c)
        .unwrap()
        .after_reply()
        .unwrap();
    let checkpoint = DeferredCheckpoint::enter(&c).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(checkpoint.after_reply().is_err());
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn quarantine_deduplication_and_enrichment_match_lossless_python_json() {
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/data/storage.json")).unwrap();
    for v in array(&vectors["quarantines"]) {
        let rows: Vec<crate::model::QuarantineRow> = serde_json::from_value(v[0].clone()).unwrap();
        let actual = crate::persistence::dedupe_quarantines(&rows).unwrap();
        assert_eq!(serde_json::to_value(actual).unwrap(), v[1]);
    }
    for v in array(&vectors["enrichment"]) {
        assert_eq!(
            crate::persistence::enrich_quarantine_detail(v[0].as_str().unwrap(), array(&v[1]))
                .unwrap(),
            v[2]
        );
    }
}

#[test]
fn malformed_storage_id_and_tie_shapes_refuse_without_writes() {
    for retained in [false, true] {
        let c = Connection::open_in_memory().unwrap();
        table(&c, "journal_entries", &ENTRY_COLUMNS);
        let a = entry("a", "same", "old");
        if retained {
            replace_journal_entries(&c, "p", std::slice::from_ref(&a), &mut ()).unwrap();
        }
        let before = snapshot::<29>(&c, "journal_entries");
        for value in [
            Sql::Null,
            Sql::Integer(1),
            Sql::Real(1.0),
            Sql::Blob(vec![0]),
        ] {
            for index in [0, 6] {
                let mut invalid = a.clone();
                invalid[index] = value.clone();
                assert!(replace_journal_entries(&c, "p", &[invalid], &mut ()).is_err());
                assert_eq!(snapshot::<29>(&c, "journal_entries"), before);
            }
        }
        assert!(replace_journal_entries(&c, "p", &[a.clone(), a], &mut ()).is_err());
        assert_eq!(snapshot::<29>(&c, "journal_entries"), before);
    }
}

#[test]
fn processing_marker_refuses_datetime_overflow() {
    let result = journal_processed_at("2020-01-01", Some("9999-12-31T23:59:59.999999Z"));
    assert_eq!(result.unwrap_err().family, "OverflowError");
}

#[test]
fn typed_calculations_preserve_compact_bytes_and_all_dynamic_fallbacks() {
    fn check(c: &crate::Calculation) {
        let encoded = encode_typed_calculation(c).unwrap();
        assert_eq!(
            encoded,
            encode_calculation(&serde_json::to_value(c).unwrap()).unwrap()
        );
        let mut sources = compare::Sources::default();
        let prepared = sources
            .prepare(Some(c), &mut CalculationEncoder::default())
            .unwrap();
        fn cell(s: Option<&str>) -> ValueRef<'_> {
            s.map_or(ValueRef::Null, |s| ValueRef::Text(s.as_bytes()))
        }
        assert!(prepared.matches(
            &sources,
            cell(encoded.0.as_deref()),
            cell(encoded.1.as_deref())
        ));
        for compact in [false, true] {
            let mut changed = encoded.clone();
            let text = if compact {
                &mut changed.1
            } else {
                &mut changed.0
            };
            *text = Some(format!(" {}", text.as_deref().unwrap_or("")));
            assert!(!prepared.matches(
                &sources,
                cell(changed.0.as_deref()),
                cell(changed.1.as_deref())
            ));
        }
    }
    let cases: Value = serde_json::from_str(include_str!("../../tests/data/render.json")).unwrap();
    let mut compared = 0;
    for case in array(&cases) {
        let Ok(response) = crate::boundary::ledger_response(&case["response"]) else {
            continue;
        };
        let Ok(state) = crate::boundary::build(
            &case["profile"],
            &case["custody"],
            array(&case["wallets"]),
            &response,
            String::new,
        ) else {
            continue;
        };
        for entry in &state.entries {
            let Some(c) = &entry.calculation else {
                continue;
            };
            check(c);
            compared += 1;
            for mode in 0..19 {
                let mut c = c.clone();
                match mode {
                    0 => c.schema_version = 2,
                    1 => c.method = "é🔥\0\n\"\\".into(),
                    2 => c.fragments.clear(),
                    3 => c.fragments.extend(c.fragments.clone()),
                    _ if c.fragments.is_empty() => continue,
                    4 => c.fragments[0].quantity_msat = json!(true).into(),
                    5 => c.fragments[0].quantity_msat = json!(1.0).into(),
                    6 => c.fragments[0].quantity_msat_exact = "wrong".into(),
                    7 => {
                        c.fragments[0].acquisition_quantity_msat = None;
                        c.fragments[0].acquisition_quantity_msat_exact = None;
                    }
                    8 => {
                        c.fragments[0].acquisition_quantity_msat = None;
                        c.fragments[0].acquisition_quantity_msat_exact = Some("0".into());
                    }
                    9 => c.fragments[0].acquisition_quantity_msat = Some(json!(true).into()),
                    10 => {
                        Arc::make_mut(&mut c.fragments[0].event).crypto_fee_msat_exact =
                            "wrong".into()
                    }
                    11 => {
                        Arc::make_mut(&mut c.fragments[0].event).crypto_fee_msat =
                            json!(false).into()
                    }
                    12 => {
                        Arc::make_mut(&mut Arc::make_mut(&mut c.fragments[0].event).provenance)
                            .transaction_id = json!(1)
                    }
                    13 => {
                        Arc::make_mut(&mut Arc::make_mut(&mut c.fragments[0].event).provenance)
                            .pricing[0] = json!(true)
                    }
                    14 => c.fragments[0].lot = None,
                    17 => {
                        Arc::make_mut(&mut Arc::make_mut(&mut c.fragments[0].event).provenance)
                            .pricing[0] = json!(1)
                    }
                    18 => {
                        let mut positive = c.fragments[0].clone();
                        positive.lot = None;
                        Arc::make_mut(&mut positive.event).crypto_fee_msat = json!(0).into();
                        Arc::make_mut(&mut positive.event).crypto_fee_msat_exact = "0".into();
                        let mut negative = positive.clone();
                        Arc::make_mut(&mut negative.event).crypto_fee_msat =
                            serde_json::from_str::<Value>("-0").unwrap().into();
                        Arc::make_mut(&mut negative.event).crypto_fee_msat_exact = "-0".into();
                        c.fragments = vec![negative, positive];
                    }
                    16 => {
                        c.fragments[0].acquisition_quantity_msat = Some(Value::Null.into());
                        c.fragments[0].acquisition_quantity_msat_exact = None;
                    }
                    15 => {
                        let f = &mut c.fragments[0];
                        f.quantity_msat = serde_json::from_str::<Value>(
                            "999999999999999999999999999999999999999",
                        )
                        .unwrap()
                        .into();
                        f.quantity_msat_exact = f.quantity_msat.to_string();
                    }
                    _ => unreachable!(),
                }
                check(&c);
            }
        }
    }
    assert!(compared > 10);
}

#[test]
fn compact_tuple_refuses_nonintegral_or_structured_scalars() {
    for v in [json!(true), json!(1.0), json!({"bad":1})] {
        assert!(compact_tuple(&v).is_none());
    }
    assert_eq!(
        compact_tuple(&json!([null, "é", 1, []])).unwrap(),
        "[null,\"\\u00e9\",1,[]]"
    );
}

#[test]
fn typed_calculation_source_dictionary_preserves_order_and_deduplicates_large_inputs() {
    use crate::model::{Calculation, Fragment, PricingFacts, Source, SourceProvenance};
    let mut calculation = Calculation {
        schema_version: 1,
        engine: "kassiber-tax",
        method: "fifo".into(),
        fragments: Vec::new(),
        loss_deductible: None,
        valuation: None,
    };
    for index in 0..4096 {
        let source = Source {
            provenance: Arc::new(SourceProvenance {
                transaction_id: json!(format!("transaction-{index}")),
                engine_event_id: format!("event-{index}").into(),
                occurred_at: "2026-01-01T00:00:00Z".into(),
                pricing: PricingFacts::Owned(Box::new(std::array::from_fn(|_| Value::Null))),
            }),
            asset: "BTC".into(),
            spot_price_exact: "30000".into(),
            fiat_fee_exact: "0".into(),
            crypto_fee_msat: json!(0).into(),
            crypto_fee_msat_exact: "0".into(),
        };
        calculation.fragments.push(Fragment {
            quantity_msat: json!(1).into(),
            quantity_msat_exact: "1".into(),
            cost_basis_exact: "1".into(),
            proceeds_exact: "2".into(),
            gain_loss_exact: "1".into(),
            expenses_exact: None,
            unit_basis_override_exact: None,
            basis_authority: "engine",
            acquisition_basis_exact: None,
            acquisition_quantity_msat: None,
            acquisition_quantity_msat_exact: None,
            event: source.clone().into(),
            lot: Some(source.into()),
        });
    }
    calculation.fragments.extend(calculation.fragments.clone());
    let encoded = encode_typed_calculation(&calculation).unwrap();
    assert_eq!(
        encoded,
        encode_calculation(&serde_json::to_value(&calculation).unwrap()).unwrap()
    );
    let mut sources = compare::Sources::default();
    let prepared = sources
        .prepare(Some(&calculation), &mut CalculationEncoder::default())
        .unwrap();
    assert!(prepared.matches(
        &sources,
        ValueRef::Null,
        ValueRef::Text(encoded.1.as_ref().unwrap().as_bytes())
    ));
    let compact: Value = serde_json::from_str(encoded.1.as_ref().unwrap()).unwrap();
    assert_eq!(array(&compact[3]).len(), 4096);
    for (index, fragment) in array(&compact[4]).iter().enumerate() {
        assert_eq!(fragment[8], index % 4096);
        assert_eq!(fragment[9], index % 4096);
    }
}

#[test]
fn a_fragment_with_expenses_keeps_them_in_the_legacy_shape() {
    use crate::model::{Calculation, Fragment, PricingFacts, Source, SourceProvenance};
    let source = Source {
        provenance: Arc::new(SourceProvenance {
            transaction_id: json!("sale"),
            engine_event_id: "event".into(),
            occurred_at: "2024-06-01T10:00:00Z".into(),
            pricing: PricingFacts::Owned(Box::new(std::array::from_fn(|_| json!("x")))),
        }),
        asset: "BTC".into(),
        spot_price_exact: "60000".into(),
        fiat_fee_exact: "0".into(),
        crypto_fee_msat: json!(0).into(),
        crypto_fee_msat_exact: "0".into(),
    };
    // de-21: the 2023 lot's share of a sale whose fee units went first.
    let fragment = |expenses: Option<&'static str>| Fragment {
        quantity_msat: json!(99_980_000_000_i64).into(),
        quantity_msat_exact: "99980000000".into(),
        cost_basis_exact: "14997".into(),
        proceeds_exact: "59988".into(),
        gain_loss_exact: "44976.0024".into(),
        expenses_exact: expenses.map(Into::into),
        unit_basis_override_exact: None,
        basis_authority: "acquired_lot",
        acquisition_basis_exact: Some("15000".into()),
        acquisition_quantity_msat: Some(json!(100_000_000_000_i64).into()),
        acquisition_quantity_msat_exact: Some("100000000000".into()),
        event: source.clone().into(),
        lot: Some(source.clone().into()),
    };
    let calculation = |expenses| Calculation {
        schema_version: 1,
        engine: "kassiber-tax",
        method: "fifo".into(),
        fragments: vec![fragment(expenses)],
        loss_deductible: None,
        valuation: None,
    };
    // Without expenses the fragment stays a compact tuple.
    let plain = calculation(None);
    let (legacy, compact) = encode_typed_calculation(&plain).unwrap();
    assert!(legacy.is_none() && compact.is_some());
    // With them, the sorted legacy JSON keeps the field, and the no-op
    // comparison reads that shape.
    let german = calculation(Some("11.9976"));
    let (legacy, compact) = encode_typed_calculation(&german).unwrap();
    assert!(compact.is_none());
    let legacy = legacy.unwrap();
    let stored: Value = serde_json::from_str(&legacy).unwrap();
    assert_eq!(stored["fragments"][0]["expenses_exact"], "11.9976");
    assert_eq!(
        (Some(legacy.clone()), None),
        encode_calculation(&serde_json::to_value(&german).unwrap()).unwrap()
    );
    let mut sources = compare::Sources::default();
    let prepared = sources
        .prepare(Some(&german), &mut CalculationEncoder::default())
        .unwrap();
    assert!(prepared.matches(&sources, ValueRef::Text(legacy.as_bytes()), ValueRef::Null));
    let encoded_plain = encode_typed_calculation(&plain).unwrap().1.unwrap();
    assert!(!prepared.matches(
        &sources,
        ValueRef::Null,
        ValueRef::Text(encoded_plain.as_bytes())
    ));
}

#[test]
fn shared_calculation_encoder_keeps_every_source_operand_and_refusal() {
    let cases: Value = serde_json::from_str(include_str!("../../tests/data/render.json")).unwrap();
    let responses: Vec<_> = array(&cases)
        .iter()
        .map(|case| crate::boundary::ledger_response(&case["response"]).ok())
        .collect();
    let calculation = array(&cases)
        .iter()
        .zip(&responses)
        .find_map(|(case, response)| {
            let state = crate::boundary::build(
                &case["profile"],
                &case["custody"],
                array(&case["wallets"]),
                response.as_ref()?,
                String::new,
            )
            .ok()?;
            state
                .entries
                .into_iter()
                .filter_map(|entry| entry.calculation)
                .find(|calculation| !calculation.fragments.is_empty())
        })
        .unwrap();
    let mut calculations = vec![calculation.clone()];
    for mode in 0..9 {
        let mut next = calculation.clone();
        let source = Arc::make_mut(&mut next.fragments[0].event);
        match mode {
            0 => source.asset = "LBTC".into(),
            1 => source.spot_price_exact = "123.4500".into(),
            2 => source.fiat_fee_exact = "12".into(),
            3 => {
                source.crypto_fee_msat = json!(7).into();
                source.crypto_fee_msat_exact = "7".into();
            }
            4 => source.crypto_fee_msat_exact = "mismatch".into(),
            5 => source.crypto_fee_msat = msat(json!(true)),
            6 => Arc::make_mut(&mut source.provenance).pricing[0] = json!(true),
            7 => Arc::make_mut(&mut source.provenance).transaction_id = json!(42),
            8 => Arc::make_mut(&mut source.provenance).occurred_at = "other".into(),
            _ => unreachable!(),
        }
        calculations.push(next);
    }
    let mut encoder = CalculationEncoder::default();
    for _ in 0..2 {
        for calculation in &calculations {
            assert_eq!(
                encoder.encode(calculation).unwrap(),
                encode_calculation(&serde_json::to_value(calculation).unwrap()).unwrap()
            );
        }
    }
}

#[derive(Default)]
struct DeltaWrites(usize);
impl WriteObserver for DeltaWrites {
    fn wrote(&mut self, _: &'static str) -> rusqlite::Result<()> {
        self.0 += 1;
        Ok(())
    }
}

#[test]
fn every_retained_content_column_forces_writes_but_the_build_clock_does_not() {
    fn check<const N: usize>(
        name: &str,
        columns: &[&str; N],
        original: [Sql; N],
        reals: &[usize],
        replace: impl Fn(&Connection, &str, &[[Sql; N]], &mut DeltaWrites) -> rusqlite::Result<()>,
    ) {
        for column in 0..N {
            let c = Connection::open_in_memory().unwrap();
            table(&c, name, columns);
            replace(
                &c,
                "p",
                std::slice::from_ref(&original),
                &mut DeltaWrites::default(),
            )
            .unwrap();
            let before = snapshot::<N>(&c, name);
            let mut next = original.clone();
            next[N - 1] = Sql::Text("later".into());
            let mut writes = DeltaWrites::default();
            replace(&c, "p", std::slice::from_ref(&next), &mut writes).unwrap();
            assert_eq!(writes.0, 0, "{name} clock");
            assert_eq!(snapshot::<N>(&c, name), before);
            if column != N - 1 {
                next[column] = if reals.contains(&column) {
                    Sql::Real(2.5)
                } else {
                    Sql::Text("changed".into())
                };
            }
            replace(&c, "p", std::slice::from_ref(&next), &mut writes).unwrap();
            assert_eq!(
                writes.0,
                if column == N - 1 { 0 } else { 2 },
                "{name} {column}"
            );
            if column != N - 1 {
                let after = snapshot::<N>(&c, name);
                assert_eq!(after.len(), 1);
                assert_eq!(after[0].1, next, "{name} {column}");
            }
        }
    }
    check(
        "journal_entries",
        &ENTRY_COLUMNS,
        entry("a", "time", "old"),
        &[10, 11, 12, 13, 14],
        replace_journal_entries,
    );
    let mut posting = std::array::from_fn(|_| Sql::Null);
    posting[0] = Sql::Text("posting".into());
    posting[2] = Sql::Text("p".into());
    posting[11] = Sql::Text("old".into());
    check(
        "journal_quantity_postings",
        &POSTING_COLUMNS,
        posting,
        &[],
        replace_quantity_postings,
    );
}

#[test]
fn retained_comparisons_distinguish_cell_boundaries_blob_text_and_null_text() {
    for mode in 0..3 {
        let c = Connection::open_in_memory().unwrap();
        table(&c, "journal_entries", &ENTRY_COLUMNS);
        let mut original = entry("a", "time", "old");
        original[24] = Sql::Text("a".into());
        original[25] = Sql::Text("bc".into());
        if mode == 1 {
            original[20] = Sql::Blob(b"{}".to_vec());
        }
        replace_journal_entries(&c, "p", std::slice::from_ref(&original), &mut ()).unwrap();
        let mut next = original;
        next[28] = Sql::Text("later".into());
        match mode {
            0 => {
                next[24] = Sql::Text("ab".into());
                next[25] = Sql::Text("c".into());
            }
            1 => next[20] = Sql::Text("{}".into()),
            2 => next[21] = Sql::Text(String::new()),
            _ => unreachable!(),
        }
        let mut writes = DeltaWrites::default();
        replace_journal_entries(&c, "p", std::slice::from_ref(&next), &mut writes).unwrap();
        assert_eq!(writes.0, 2, "mode {mode}");
        assert_eq!(snapshot::<29>(&c, "journal_entries")[0].1, next);
    }
}

#[test]
fn every_entry_real_column_preserves_the_negative_zero_rewrite_rule() {
    for column in 10..=14 {
        let c = Connection::open_in_memory().unwrap();
        table(&c, "journal_entries", &ENTRY_COLUMNS);
        let mut original = entry("a", "time", "old");
        original[column] = Sql::Real(0.0);
        replace_journal_entries(&c, "p", std::slice::from_ref(&original), &mut ()).unwrap();
        let mut next = original;
        next[column] = Sql::Real(-0.0);
        next[28] = Sql::Text("later".into());
        for _ in 0..2 {
            let mut writes = DeltaWrites::default();
            replace_journal_entries(&c, "p", std::slice::from_ref(&next), &mut writes).unwrap();
            assert_eq!(writes.0, 2, "column {column}");
            assert_eq!(
                snapshot::<29>(&c, "journal_entries")[0].1[28],
                Sql::Text("later".into())
            );
        }
    }
}

#[test]
fn compact_key_sets_and_scalar_guards_preserve_exact_shapes() {
    assert!(keys(&json!({}), &[]));
    assert!(keys(&json!({"required":null}), &["required"]));
    assert!(!keys(&json!({"required":null,"future":1}), &["required"]));
    assert!(!keys(&json!({"future":1}), &["required"]));
    assert!(!keys(&json!({}), &["required"]));
    for value in [Value::Null, json!(false), json!(0), json!(""), json!([])] {
        assert!(!keys(&value, &[]));
    }
    assert!(text(&Value::Null));
    assert!(text(&json!("")));
    for value in [json!(true), json!(0), json!(1.0), json!([]), json!({})] {
        assert!(!text(&value));
    }
    for spelling in [
        "0",
        "-1",
        "18446744073709551616",
        "999999999999999999999999999999999999999999999999",
    ] {
        let value = serde_json::from_str(spelling).unwrap();
        assert!(integer(&value));
        assert!(exact_integer_text(&value, Some(spelling)));
        assert!(!exact_integer_text(&value, Some("different")));
        assert!(!exact_integer_text(&value, None));
    }
    for spelling in ["1.0", "1e0", "1E0", "-0.0"] {
        let value = Value::Number(serde_json::Number::from_string_unchecked(spelling.into()));
        assert!(!integer(&value));
        assert!(!exact_integer_text(&value, Some(spelling)));
    }
    for value in [Value::Null, json!(true), json!("1"), json!([]), json!({})] {
        assert!(!integer(&value));
        assert!(!exact_integer_text(&value, Some("1")));
    }
    let zero = Value::Number(serde_json::Number::from_string_unchecked("-0".into()));
    assert_eq!(compact_tuple(&zero).as_deref(), Some("0"));
}
