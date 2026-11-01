//! Unregistered native review composition. Loader acceptance still gates routes.
use crate::{review_effects, review_simulation, pipeline::{self, Prepared}, Refusal};
use kassiber_transfers::{review_artifact, review_operations, review_support, Sources};
use kassiber_transfers::review_support::value;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::{review_cases, sql, Scope};

/// Keep reference exception families and attributes intact through composition.
#[derive(Debug)]
pub enum Error {
    Service(AppError),
    Build(Refusal),
}
impl From<AppError> for Error {
    fn from(error: AppError) -> Self {
        Self::Service(error)
    }
}
impl From<Refusal> for Error {
    fn from(error: Refusal) -> Self {
        Self::Build(error)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Service(error) => error.fmt(formatter),
            Self::Build(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for Error {}

fn profile(conn: &Connection, scope: &Scope<'_>) -> Result<Value, Error> {
    sql::rows(
        conn,
        "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
        [scope.profile_id, scope.workspace_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review profile was not found").into())
}

fn prepare(conn: &Connection, scope: &Scope<'_>, as_of: &str) -> Result<Prepared, Error> {
    Ok(Prepared::load(conn, profile(conn, scope)?, as_of)?)
}

fn facts(
    conn: &Connection,
    scope: &Scope<'_>,
    as_of: &str,
) -> Result<review_simulation::Cases, Error> {
    let prepared = prepare(conn, scope, as_of)?;
    let state = prepared.review_state(conn, derived_ids())?;
    Ok(review_simulation::Cases::from_state(&state))
}

fn derived_ids() -> impl FnMut() -> String {
    let mut index = 0u64;
    move || {
        index += 1;
        format!("review-derived-{index}")
    }
}

pub fn plan(
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &Value,
    expected_input_version: &Value,
    as_of: &str,
    sources: &dyn Sources,
) -> Result<Value, Error> {
    let normalized = review_operations::normalize(operations)?;
    let clone = pipeline::memory_snapshot(conn, || {
        conn.query_row("SELECT lower(hex(randomblob(32)))", [], |row| row.get(0))
            .map_err(Refusal::from)
    })?;
    profile(&clone, scope)?;
    review_support::require_input_version(&clone, scope, expected_input_version)?;
    let _simulation = clone.unchecked_transaction().map_err(sql::error)?;
    let prepared = prepare(&clone, scope, as_of)?;
    let state = prepared.review_state(&clone, derived_ids())?;
    let before = review_effects::effects(&state, &clone, scope, &normalized)?;
    let cases = review_simulation::Cases::from_state(&state);
    let Value::List(operations) = &normalized else {
        unreachable!("normalized operations")
    };
    review_simulation::apply(&clone, scope, operations, sources, "user", cases, |conn| {
        facts(conn, scope, as_of)
    })?;
    let prepared = prepare(&clone, scope, as_of)?;
    let state = prepared.review_state(&clone, derived_ids())?;
    let after = review_effects::effects(&state, &clone, scope, &normalized)?;
    Ok(review_artifact::assemble(
        scope,
        expected_input_version.clone(),
        normalized,
        before,
        after,
    )?)
}

pub fn validate(
    conn: &Connection,
    scope: &Scope<'_>,
    artifact: &Value,
    as_of: &str,
    sources: &dyn Sources,
) -> Result<Value, Error> {
    if !matches!(artifact, Value::Dict(_))
        || value(artifact, "workspace_id") != s(scope.workspace_id)
        || value(artifact, "profile_id") != s(scope.profile_id)
    {
        return Err(AppError::new(
            "review_scope_mismatch",
            "Review artifact belongs to another profile",
        )
        .into());
    }
    let fresh = plan(
        conn,
        scope,
        &value(artifact, "operations"),
        &value(artifact, "base_input_version"),
        as_of,
        sources,
    )?;
    if !review_artifact::equal(&fresh, artifact) {
        return Err(AppError::new(
            "review_artifact_invalid",
            "Review artifact changed; create a fresh preview",
        )
        .into());
    }
    Ok(fresh)
}

/// Build economic and policy effects on one live read snapshot. This is not a
/// plan: ordered operations and their simulated effects are still required.
pub fn inspect_effects(
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &Value,
    as_of: &str,
    ids: impl FnMut() -> String,
) -> Result<Value, Error> {
    let _read = if conn.is_autocommit() {
        Some(conn.unchecked_transaction().map_err(sql::error)?)
    } else {
        None
    };
    let profile = sql::rows(
        conn,
        "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
        [scope.profile_id, scope.workspace_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review profile was not found"))?;
    let prepared = Prepared::load(conn, profile, as_of)?;
    let state = prepared.review_state(conn, ids)?;
    Ok(review_effects::effects(&state, conn, scope, operations)?)
}

/// Resolve cursor, profile, canonical facts and freshness on one live snapshot.
/// Clock and derived identities remain caller supplied; neither grants authority.
pub fn cases(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    as_of: &str,
    ids: impl FnMut() -> String,
) -> Result<Value, Error> {
    review_cases::inspect_with_error(conn, scope, args, |live, profile| {
        let prepared = Prepared::load(live, profile.clone(), as_of)?;
        let state = prepared.review_state(live, ids)?;
        state
            .quarantines
            .into_iter()
            .map(|row| {
                let value = |value: &serde_json::Value| {
                    kassiber_custody::codec::from_json(value)
                        .map_err(|message| AppError::new("internal_error", message))
                };
                Ok(dict([
                    ("transaction_id", value(&row.transaction_id)?),
                    ("workspace_id", value(&row.workspace_id)?),
                    ("profile_id", value(&row.profile_id)?),
                    ("reason", s(row.reason)),
                    ("detail_json", s(row.detail_json)),
                ]))
            })
            .collect()
    })
}
