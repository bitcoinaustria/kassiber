//! One writer scope owns review operations, canonical journals and the receipt.
use crate::{persistence, storage::RebuildCache};
use crate::{
    review_effects, review_simulation,
    review_workflow::{derived_ids, facts, prepare, profile, Error},
};
use kassiber_custody::codec;
use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_store::rusqlite::{params, Connection};
use kassiber_transfers::review_support::sql;
use kassiber_transfers::review_support::{text, value};
use kassiber_transfers::{review_artifact, review_operations, review_support, Scope, Sources};
use std::collections::BTreeSet;

pub struct Apply<'a> {
    pub artifact: &'a Value,
    pub idempotency_key: &'a Value,
    pub authored_source: &'a str,
    /// False leaves every successful write pending in the caller's transaction.
    pub commit: bool,
}

struct Transaction<'a> {
    conn: &'a Connection,
    owns: bool,
    released: bool,
    commit: bool,
    complete: bool,
}
impl<'a> Transaction<'a> {
    fn begin(conn: &'a Connection, commit: bool) -> Result<Self, Error> {
        let owns = conn.is_autocommit();
        if owns {
            conn.execute_batch("BEGIN IMMEDIATE").map_err(sql::error)?;
        }
        let transaction = Self {
            conn,
            owns,
            released: false,
            commit,
            complete: false,
        };
        conn.execute_batch("SAVEPOINT review_workflow_apply")
            .map_err(sql::error)?;
        Ok(transaction)
    }

    fn finish(mut self) -> Result<(), Error> {
        self.conn
            .execute_batch("RELEASE SAVEPOINT review_workflow_apply")
            .map_err(sql::error)?;
        self.released = true;
        if self.commit {
            self.conn.execute_batch("COMMIT").map_err(sql::error)?;
        }
        self.complete = true;
        Ok(())
    }
}
impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        if !self.released {
            let _ = self.conn.execute_batch(
                "ROLLBACK TO SAVEPOINT review_workflow_apply; RELEASE SAVEPOINT review_workflow_apply",
            );
        }
        if self.owns || (self.released && self.commit) {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(rows) => rows,
        _ => &[],
    }
}

fn summary(operation: &Value) -> Value {
    if value(operation, "type") != s("custody_component") {
        return operation.clone();
    }
    let request = value(operation, "request");
    let mut result = dict([
        ("type", s("custody_component")),
        ("action", value(&request, "action")),
    ]);
    for key in ["component_id", "reason", "activate"] {
        if let Some(value) = get(&request, key) {
            set(&mut result, key, value.clone());
        }
    }
    if let Some(components) = get(&request, "components") {
        set(
            &mut result,
            "component_count",
            list(components).len().into(),
        );
    }
    result
}

fn transaction_ids(results: &Value) -> Value {
    let mut ids = BTreeSet::<PyStr>::new();
    let mut include = |value: Value| {
        if let Value::Str(id) = value {
            if id.code_points().next().is_some() {
                ids.insert(id);
            }
        }
    };
    for item in list(results) {
        let result = value(item, "result");
        include(value(&result, "transaction_id"));
        for id in list(&value(&result, "transaction_ids")) {
            include(id.clone());
        }
        let components = value(&result, "components");
        let component = value(&result, "component");
        for component in list(&components).iter().chain(std::iter::once(&component)) {
            for leg in list(&value(component, "legs")) {
                let anchor = value(leg, "anchor_transaction_id");
                include(if kassiber_envelope::truthy(&anchor) {
                    anchor
                } else {
                    value(leg, "transaction_id")
                });
            }
        }
    }
    Value::List(ids.into_iter().map(Value::Str).collect())
}

fn stale(message: &str) -> Error {
    AppError::new("review_plan_stale", message).into()
}

pub fn apply(
    conn: &Connection,
    scope: &Scope<'_>,
    request: Apply<'_>,
    as_of: &str,
    sources: &dyn Sources,
) -> Result<Value, Error> {
    let _cache = RebuildCache::enter(conn).map_err(sql::error)?;
    let artifact = request.artifact;
    let normalized = review_artifact::admit(artifact, scope, request.idempotency_key)?;
    let transaction = Transaction::begin(conn, request.commit)?;
    // Reserve the writer even when a caller supplied a deferred transaction.
    conn.execute(
        "UPDATE profiles SET journal_input_version=journal_input_version WHERE id=?",
        [scope.profile_id],
    )
    .map_err(sql::error)?;
    let existing = sql::rows(
        conn,
        "SELECT artifact_digest,receipt_json FROM review_workflow_receipts WHERE profile_id=? AND idempotency_key=?",
        params![scope.profile_id, sql::bind(request.idempotency_key, 2)?],
    )?;
    let receipt = if let Some(existing) = existing.first() {
        if value(existing, "artifact_digest") != value(artifact, "digest") {
            return Err(AppError::new(
                "review_idempotency_conflict",
                "Idempotency key already belongs to another proposal",
            )
            .into());
        }
        review_support::parse_json(&text(existing, "receipt_json"))?
    } else {
        review_support::require_input_version(conn, scope, &value(artifact, "base_input_version"))?;
        let before = prepare(conn, scope, as_of)?;
        let state = before.review_state(conn, derived_ids())?;
        if !review_artifact::equal(
            &review_effects::effects(&state, conn, scope, &normalized)?,
            &value(artifact, "before"),
        ) {
            return Err(stale("Review evidence changed; create a fresh preview"));
        }
        let results = review_simulation::apply(
            conn,
            scope,
            list(&normalized),
            sources,
            request.authored_source,
            review_simulation::Cases::from_state(&state),
            |conn| facts(conn, scope, as_of),
        )?;
        let after = prepare(conn, scope, as_of)?;
        // Persistence derives canonical entry identities from their economic
        // keys; temporary render identities never enter the durable journal.
        let state = after.review_state(conn, derived_ids())?;
        let mut effects = review_effects::effects(&state, conn, scope, &normalized)?;
        if !review_artifact::equal(&effects, &value(artifact, "after")) {
            return Err(stale("Review effects changed; create a fresh preview"));
        }
        let current = profile(conn, scope)?;
        let stored = persistence::store_ledger_state(
            conn,
            &codec::to_json(&current),
            &state,
            &sources.now(conn)?,
            &mut || sources.uuid(conn).map_err(service_refusal),
            &mut (),
        )?;
        set(&mut effects, "processed_at", s(stored.processed_at));
        let receipt = dict([
            ("id", s(sources.uuid(conn)?)),
            ("schema_version", 1.into()),
            ("workspace_id", value(&current, "workspace_id")),
            ("profile_id", value(&current, "id")),
            ("idempotency_key", request.idempotency_key.clone()),
            ("artifact_digest", value(artifact, "digest")),
            ("base_input_version", value(artifact, "base_input_version")),
            (
                "result_input_version",
                review_support::input_version(&current).into(),
            ),
            ("status", s("verified")),
            ("transaction_ids", transaction_ids(&results)),
            ("operations", results),
            ("before", value(artifact, "before")),
            (
                "proposed_operations",
                Value::List(list(&normalized).iter().map(summary).collect()),
            ),
            ("verification", effects),
            ("authored_source", s(request.authored_source)),
            ("created_at", s(sources.now(conn)?)),
        ]);
        conn.execute(
            "INSERT INTO review_workflow_receipts(id,workspace_id,profile_id,idempotency_key,artifact_digest,receipt_json,created_at) VALUES(?,?,?,?,?,?,?)",
            params![text(&receipt, "id"),scope.workspace_id,scope.profile_id,sql::bind(request.idempotency_key,4)?,text(artifact,"digest"),review_operations::canonical(&receipt)?,text(&receipt,"created_at")],
        ).map_err(sql::error)?;
        receipt
    };
    transaction.finish()?;
    Ok(receipt)
}

fn service_refusal(error: AppError) -> crate::Refusal {
    error.into()
}
