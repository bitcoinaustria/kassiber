//! Unregistered native review composition. Loader acceptance still gates routes.
use crate::review_effects;
use crate::{pipeline::Prepared, Refusal};
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
