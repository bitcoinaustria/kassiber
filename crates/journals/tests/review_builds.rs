#![cfg(feature = "test-harness")]
use kassiber_journals::{boundary, pipeline::Prepared};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};

/// A canonical state without its render-time entry identities (persistence
/// derives the stored ones) and the quantity stage's canonical input (which
/// no journal reads, `QuantityState::json_with_views`), with its quantity
/// projection's observations and postings in their own fields.
fn comparable(mut state: Value) -> Value {
    for entry in state["entries"].as_array_mut().unwrap() {
        entry["id"] = json!("");
    }
    state["custody_quantity"]
        .as_object_mut()
        .unwrap()
        .remove("canonical_input");
    let projection = state["custody_quantity"]["projection"]
        .as_object_mut()
        .unwrap();
    let observations = projection.remove("observations").unwrap_or(json!([]));
    let postings = projection.remove("postings").unwrap_or(json!([]));
    json!({"state": state, "observations": observations, "postings": postings})
}

/// The typed review state as [`comparable`] has it, reading each quantity
/// row's fields that `expected` holds through the state's views.
fn typed_projection(state: &kassiber_journals::JournalState<'_>, expected: &Value) -> Value {
    let mut typed = comparable(serde_json::from_str(&boundary::canonical(state).unwrap()).unwrap());
    let (observations, postings) = state.quantity_refs();
    let projection = &expected["custody_quantity"]["projection"];
    let read = |rows: &Value, at: &dyn Fn(usize, &str) -> Value, count: usize| {
        assert_eq!(rows.as_array().map_or(0, Vec::len), count);
        Value::Array(
            rows.as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, row)| {
                    Value::Object(
                        row.as_object()
                            .unwrap()
                            .keys()
                            .map(|key| (key.clone(), at(index, key)))
                            .collect(),
                    )
                })
                .collect(),
        )
    };
    typed["observations"] = read(
        &projection["observations"],
        &|index, key| observations[index].at(key).clone(),
        observations.len(),
    );
    // A posting read in place has no `$type` or `domain`, which no journal
    // reads; take them from the reference.
    typed["postings"] = read(
        &projection["postings"],
        &|index, key| match key {
            "$type" | "domain" => projection["postings"][index][key].clone(),
            _ => postings[index].at(key).clone(),
        },
        postings.len(),
    );
    typed
}

#[test]
fn review_builds_match_complete_python_states() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("data/review-builds.json")).unwrap();
    assert_eq!(cases.len(), 33);
    let mut carrying = 0;
    let mut suspense = 0;
    for case in cases {
        let prepared = Prepared::from_snapshot(
            kassiber_custody::parse(&case["snapshot"].to_string()).unwrap(),
        )
        .unwrap();
        let mut loaded: kassiber_custody::snapshot::Snapshot =
            serde_json::from_value(case["snapshot"].clone()).unwrap();
        loaded.escape_wire_tags();
        let native = kassiber_custody::run_loaded(&loaded, true).unwrap();
        assert_eq!(
            kassiber_custody::response(&loaded, &native, false),
            kassiber_custody::response(
                &prepared.computed.snapshot,
                &prepared.computed.stages,
                false,
            ),
            "typed/native admission and stages {}",
            case["name"]
        );
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE transactions(id TEXT, profile_id TEXT)")
            .unwrap();
        for id in case["live_ids"].as_array().unwrap() {
            conn.execute(
                "INSERT INTO transactions VALUES(?,?)",
                [
                    id.as_str().unwrap(),
                    prepared.profile["id"].as_str().unwrap(),
                ],
            )
            .unwrap();
        }
        conn.execute_batch("BEGIN").unwrap();
        let changes = conn.total_changes();
        let mut ids = case["entry_ids"].as_array().unwrap().iter();
        let result = prepared.review_state(&conn, || {
            ids.next()
                .expect("entry identity tape")
                .as_str()
                .unwrap()
                .into()
        });
        let actual = match result {
            Ok(state) => {
                carrying += state
                    .custody_transfers
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| row["basis_state"] == "eligible")
                    .count();
                for decision in state.custody_quantity["projection"]["decisions"]
                    .as_array()
                    .unwrap()
                {
                    if decision["state"] == "custody_suspense"
                        && decision["reason"] == "reviewed_residual_suspense"
                    {
                        assert!(decision["component_id"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty()));
                        suspense += 1;
                    }
                }
                assert!(ids.next().is_none(), "identity tape {}", case["name"]);
                json!({"state": boundary::canonical(&state).unwrap()})
            }
            Err(error) => json!({"error": error}),
        };
        assert_eq!(
            actual, case["expected"],
            "{}: {}",
            case["name"], case["sources"]
        );
        // Review builds through `journals process`'s own path; it renders
        // the same state but for the identities persistence derives and the
        // quantity projection rows it reads through views.
        let expected: Value =
            serde_json::from_str(case["expected"]["state"].as_str().unwrap()).unwrap();
        let typed = kassiber_journals::pipeline::review_state(&conn, &loaded, |state| {
            typed_projection(&state, &expected)
        })
        .unwrap();
        assert_eq!(
            typed,
            comparable(expected.clone()),
            "typed {}: {}",
            case["name"],
            case["sources"]
        );
        assert_eq!(conn.total_changes(), changes);
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
    }
    assert!(carrying > 0);
    assert!(suspense > 0);
}

#[test]
fn review_builds_keep_primary_reasons_and_only_live_profile_anchors() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("data/review-builds.json")).unwrap();
    let mut prepared = Prepared::from_snapshot(
        kassiber_custody::parse(&cases[0]["snapshot"].to_string()).unwrap(),
    )
    .unwrap();
    prepared.computed.fusion = kassiber_journals::Fusion::Empty;
    let quarantine = |id, reason, detail| {
        json!({
            "transaction_id":id, "workspace_id":prepared.profile["workspace_id"],
            "profile_id":prepared.profile["id"], "reason":reason, "detail_json":detail
        })
    };
    prepared.custody["finalized_tax_projection"]["quarantines"] = json!([
        quarantine("a", "custody_basis_barrier", "{\"upstream\":\"b\"}"),
        quarantine("a", "missing_spot_price", "{\"price\":\"missing\"}"),
        quarantine("synthetic", "missing_spot_price", "{}"),
        quarantine("foreign", "missing_spot_price", "{}")
    ]);
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE transactions(id TEXT,profile_id TEXT)")
        .unwrap();
    conn.execute(
        "INSERT INTO transactions VALUES('a',?)",
        [prepared.profile["id"].as_str().unwrap()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO transactions VALUES('foreign','another-profile')",
        [],
    )
    .unwrap();
    conn.execute_batch("BEGIN").unwrap();
    let changes = conn.total_changes();
    let state = prepared
        .review_state(&conn, || panic!("empty engine generates no ids"))
        .unwrap();
    assert_eq!(state.quarantines.len(), 1);
    let row = &state.quarantines[0];
    assert_eq!(row.transaction_id, "a");
    assert_eq!(row.reason, "missing_spot_price");
    assert_eq!(
        serde_json::from_str::<Value>(&row.detail_json).unwrap(),
        json!({
            "price":"missing", "additional_reasons":[{"reason":"custody_basis_barrier", "detail":{"upstream":"b"}}]
        })
    );
    assert_eq!(conn.total_changes(), changes);
    assert!(!conn.is_autocommit());
}
