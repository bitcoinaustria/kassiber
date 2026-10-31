//! Unregistered native review composition. Loader acceptance still gates routes.
use crate::{review_cases, Scope};
use kassiber_envelope::{dict, s, AppError};
use kassiber_journals::{pipeline::Prepared, Refusal};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

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
