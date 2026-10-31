use super::*;
use crate::array;

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
