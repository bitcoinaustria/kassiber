//! Native review composition behind `review` and `ui.review.*`: current cases,
//! input requests, RAM-only plans, atomic application and stored receipts.
pub use crate::review_apply::{apply, Apply};
use crate::{pipeline, review_effects, review_simulation, JournalState, Refusal};
use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::review_support::sql;
use kassiber_transfers::review_support::value;
use kassiber_transfers::{review_artifact, review_operations, review_support, Sources};
use kassiber_transfers::{review_cases, Scope};
use std::collections::{BTreeMap, BTreeSet};

/// Keep reference exception families and attributes intact through composition.
#[derive(Clone, Debug)]
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

pub(crate) fn profile(conn: &Connection, scope: &Scope<'_>) -> Result<Value, Error> {
    sql::rows(
        conn,
        "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
        [scope.profile_id, scope.workspace_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review profile was not found").into())
}

/// Review's journal state of the scope's profile on `conn`, built by the
/// same path `journals process` stores from (`pipeline::load_review_state`).
pub(crate) fn with_state<R>(
    conn: &Connection,
    scope: &Scope<'_>,
    as_of: &str,
    then: impl FnOnce(&JournalState<'_>) -> Result<R, Error>,
) -> Result<R, Error> {
    with_state_until(conn, scope, as_of, || Ok(()), then)
}

/// [`with_state`], asking `proceed` between the build's steps.
fn with_state_until<R>(
    conn: &Connection,
    scope: &Scope<'_>,
    as_of: &str,
    proceed: impl Fn() -> Result<(), Error>,
    then: impl FnOnce(&JournalState<'_>) -> Result<R, Error>,
) -> Result<R, Error> {
    pipeline::load_review_state_until(conn, profile(conn, scope)?, as_of, proceed, |state| {
        then(&state)
    })?
}

pub(crate) fn facts(
    conn: &Connection,
    scope: &Scope<'_>,
    as_of: &str,
) -> Result<review_simulation::Cases, Error> {
    with_state(conn, scope, as_of, |state| {
        Ok(review_simulation::Cases::from_state(state))
    })
}

pub fn plan(
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &Value,
    expected_input_version: &Value,
    as_of: &str,
    sources: &(dyn Sources + Sync),
) -> Result<Value, Error> {
    prepare_plan(conn, scope, operations, expected_input_version, as_of)?.run(sources, &|| false)
}

/// A plan taken off the book: its normalized operations and two private
/// in-memory copies of the book at the bound input version. One stays as the
/// book is and yields the effects before; the operations apply to the other.
/// Taking it is quick and needs the book's connection; running it needs
/// neither, so the daemon can run it on a worker and let a newer preview
/// cancel it.
pub struct PlanJob {
    before: Connection,
    after: Connection,
    workspace_id: String,
    profile_id: String,
    operations: Value,
    expected_input_version: Value,
    as_of: String,
}

/// Normalize the operations, copy the book and check its input version.
pub fn prepare_plan(
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &Value,
    expected_input_version: &Value,
    as_of: &str,
) -> Result<PlanJob, Error> {
    let normalized = review_operations::normalize(operations)?;
    let ephemeral_key = |conn: &Connection| -> Result<String, Refusal> {
        conn.query_row("SELECT lower(hex(randomblob(32)))", [], |row| row.get(0))
            .map_err(Refusal::from)
    };
    let before = pipeline::memory_snapshot(conn, || ephemeral_key(conn))?;
    profile(&before, scope)?;
    review_support::require_input_version(&before, scope, expected_input_version)?;
    // The second copy is taken from the first, so both hold the same book
    // even when another process writes the live one in between.
    let after = pipeline::memory_snapshot(&before, || ephemeral_key(&before))?;
    Ok(PlanJob {
        before,
        after,
        workspace_id: scope.workspace_id.to_owned(),
        profile_id: scope.profile_id.to_owned(),
        operations: normalized,
        expected_input_version: expected_input_version.clone(),
        as_of: as_of.to_owned(),
    })
}

/// The error a plan answers once `cancelled` holds at one of its steps.
pub fn plan_cancelled() -> AppError {
    AppError::new(
        "review_plan_cancelled",
        "The review plan was cancelled before it finished",
    )
}

impl PlanJob {
    /// Rebuild the book before and after the operations, on two threads: the
    /// copies are independent, so the effects are the ones one thread would
    /// compute in turn. The operations wait for the cases before only when
    /// they read them. A failure before the operations wins over theirs, as
    /// when the steps run in order. `cancelled` is checked between steps.
    pub fn run(
        self,
        sources: &(dyn Sources + Sync),
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Value, Error> {
        let PlanJob {
            before: unchanged,
            after: changed,
            workspace_id,
            profile_id,
            mut operations,
            expected_input_version,
            as_of,
        } = self;
        let scope = Scope {
            workspace_id: &workspace_id,
            profile_id: &profile_id,
        };
        let check = || {
            if cancelled() {
                Err(Error::Service(plan_cancelled()))
            } else {
                Ok(())
            }
        };
        let as_of = as_of.as_str();
        let scope = &scope;
        // The effects before see the operations as they were given; an
        // acceptance is filled in with its cases only on the changed copy.
        let given = operations.clone();
        let (before, after) = std::thread::scope(|threads| {
            let started = threads.spawn(
                move || -> Result<(review_simulation::Cases, Value), Error> {
                    check()?;
                    let _simulation = unchanged.unchecked_transaction().map_err(sql::error)?;
                    with_state_until(&unchanged, scope, as_of, check, |state| {
                        let before =
                            review_effects::effects(state, &unchanged, scope, &given, sources)?;
                        Ok((review_simulation::Cases::from_state(state), before))
                    })
                },
            );
            let started = std::cell::RefCell::new(Some(started));
            let joined = std::cell::RefCell::new(None);
            let join = || {
                if let Some(thread) = started.borrow_mut().take() {
                    let result = thread
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                    *joined.borrow_mut() = Some(result);
                }
            };
            let after = (|| -> Result<Value, Error> {
                check()?;
                let _simulation = changed.unchecked_transaction().map_err(sql::error)?;
                let Value::List(batch) = &mut operations else {
                    unreachable!("normalized operations")
                };
                // An acceptance that names no transaction is filled in with the
                // cases it accepts, so the artifact names each one.
                review_simulation::apply(
                    &changed,
                    scope,
                    batch,
                    sources,
                    "user",
                    true,
                    review_simulation::Before::later(|| {
                        join();
                        match joined.borrow().as_ref() {
                            Some(Ok((cases, _))) => Ok(cases.clone()),
                            Some(Err(error)) => Err(error.clone()),
                            None => unreachable!("the cases before were joined"),
                        }
                    }),
                    |conn| facts(conn, scope, as_of),
                )?;
                check()?;
                with_state_until(&changed, scope, as_of, check, |state| {
                    review_effects::effects(state, &changed, scope, &operations, sources)
                })
            })();
            join();
            let before = joined
                .into_inner()
                .expect("the cases before were joined")
                .map(|(_, before)| before);
            (before, after)
        });
        let (before, after) = (before?, after?);
        Ok(review_artifact::assemble(
            scope,
            expected_input_version,
            operations,
            before,
            after,
        )?)
    }
}

pub fn validate(
    conn: &Connection,
    scope: &Scope<'_>,
    artifact: &Value,
    as_of: &str,
    sources: &(dyn Sources + Sync),
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

/// Build economic and policy effects on one live read snapshot, without the
/// per-year tax figures (those store the state). This is not a plan: ordered
/// operations and their simulated effects are still required.
pub fn inspect_effects(
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &Value,
    as_of: &str,
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
    pipeline::load_review_state(conn, profile, as_of, |state| {
        Ok(review_effects::effects_without_tax(
            &state, conn, scope, operations,
        )?)
    })?
}

/// Resolve cursor, profile, canonical facts and freshness on one live snapshot.
/// Clock and derived identities remain caller supplied; neither grants authority.
pub fn cases(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    as_of: &str,
) -> Result<Value, Error> {
    review_cases::inspect_with_error(conn, scope, args, |live, profile| {
        let quarantines =
            pipeline::load_review_state(live, profile.clone(), as_of, |state| state.quarantines)?;
        quarantines
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

fn invalid_input(message: &str) -> Error {
    AppError::new("validation", message).into()
}

/// Describe missing user input for current canonical cases. The packet is a
/// stable correlation ID for the interface; it grants no authority and writes
/// nothing, so a stale case refuses rather than becoming a resolution.
pub fn request_input(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    as_of: &str,
) -> Result<Value, Error> {
    let action = value(args, "action");
    if !matches!(&action, Value::Str(action) if ["connect_wallet", "import_history", "attach_evidence"].iter().any(|known| action.is(known)))
    {
        return Err(invalid_input("Unsupported review input action"));
    }
    let case_ids = match get(args, "case_ids") {
        Some(Value::List(ids)) => ids.as_slice(),
        _ => &[],
    };
    let prefix: Vec<u32> = "quarantine:".chars().map(u32::from).collect();
    let ids: Vec<PyStr> = case_ids
        .iter()
        .filter_map(|id| match id {
            Value::Str(id) => Some(id.clone()),
            _ => None,
        })
        .filter(|id| {
            let points: Vec<u32> = id.code_points().collect();
            points.starts_with(&prefix) && points.len() <= 300
        })
        .collect();
    let distinct: BTreeSet<&PyStr> = ids.iter().collect();
    if !(1..=20).contains(&case_ids.len())
        || ids.len() != case_ids.len()
        || distinct.len() != ids.len()
    {
        return Err(invalid_input(
            "Review input requires 1 to 20 distinct canonical case IDs",
        ));
    }
    if action == s("attach_evidence") && ids.len() != 1 {
        return Err(invalid_input(
            "Attach evidence requires exactly one review case",
        ));
    }
    let explanation = match get(args, "explanation") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Str(text))
            if (1..=1000).contains(&kassiber_pytext::strip_py(text).code_points().count())
                && !text
                    .code_points()
                    .any(|point| point < 32 && point != 10 && point != 9) =>
        {
            Value::Str(kassiber_pytext::strip_py(text))
        }
        Some(_) => {
            return Err(invalid_input(
                "Review input explanation must be bounded plain text",
            ))
        }
    };
    let expected = value(args, "expected_input_version");
    let _read = if conn.is_autocommit() {
        Some(conn.unchecked_transaction().map_err(sql::error)?)
    } else {
        None
    };
    let current = profile(conn, scope)?;
    review_support::require_input_version(conn, scope, &expected)?;
    let reasons: BTreeMap<PyStr, String> = with_state(conn, scope, as_of, |state| {
        Ok(state
            .quarantines
            .iter()
            .map(|row| {
                (
                    PyStr::from(kassiber_custody::codec::pystr(&row.transaction_id).as_str()),
                    row.reason.clone(),
                )
            })
            .collect())
    })?;
    let mut sorted = ids;
    sorted.sort();
    let mut cases = Vec::new();
    for case_id in sorted {
        let transaction_id = PyStr::from_code_points(case_id.code_points().skip(prefix.len()))
            .expect("code points of a valid string");
        let row = sql::rows(
            conn,
            "SELECT id AS transaction_id, wallet_id, direction, asset, occurred_at FROM transactions WHERE id = ? AND profile_id = ?",
            kassiber_store::rusqlite::params![
                review_support::reference_bind(&Value::Str(transaction_id.clone()), 1)?,
                scope.profile_id
            ],
        )?
        .into_iter()
        .next();
        let (Some(mut row), Some(reason)) = (row, reasons.get(&transaction_id)) else {
            return Err(AppError::new(
                "review_case_stale",
                "Requested review case is no longer current",
            )
            .into());
        };
        set(&mut row, "case_id", Value::Str(case_id));
        set(&mut row, "reason", s(reason.as_str()));
        cases.push(row);
    }
    let mut packet = dict([
        ("schema_version", 1.into()),
        ("workspace_id", value(&current, "workspace_id")),
        ("profile_id", value(&current, "id")),
        ("input_version", expected),
        ("action", action),
        ("cases", Value::List(cases)),
        ("explanation", explanation),
    ]);
    let digest = review_operations::digest(&packet)?;
    set(&mut packet, "request_id", s(digest));
    Ok(packet)
}

/// The production composition root shared by the CLI and the daemon. Callers
/// admit their own argument shapes; `authored_source` labels applied history.
/// Every build samples the clock once per request; nothing here reaches the
/// network, and only `review.apply` writes.
pub fn execute(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    authored_source: &str,
    sources: &(dyn Sources + Sync),
) -> Result<Value, Error> {
    if operation == "review.receipt" {
        return Ok(kassiber_transfers::execute(conn, scope, operation, args)?);
    }
    let as_of = sources.now_precise(conn)?;
    match operation {
        "review.cases" => cases(conn, scope, args, &as_of),
        "review.request_input" => request_input(conn, scope, args, &as_of),
        "review.plan" => plan(
            conn,
            scope,
            &value(args, "operations"),
            &value(args, "expected_input_version"),
            &as_of,
            sources,
        ),
        // `validate_review`: the consent preview's recomputation.
        "review.validate" => validate(conn, scope, &value(args, "artifact"), &as_of, sources),
        "review.apply" => {
            let artifact = value(args, "artifact");
            let receipt = apply(
                conn,
                scope,
                Apply {
                    artifact: &artifact,
                    idempotency_key: &value(args, "idempotency_key"),
                    authored_source,
                    commit: true,
                },
                &as_of,
                sources,
            )?;
            if review_operations::changes_transfer_candidates(&artifact) {
                // The shared review count caches open moves; keep it with the book.
                kassiber_transfers::pairing::recount_candidates(conn, scope, sources);
            }
            Ok(receipt)
        }
        _ => Err(AppError::new("validation", "Unsupported review command").into()),
    }
}
