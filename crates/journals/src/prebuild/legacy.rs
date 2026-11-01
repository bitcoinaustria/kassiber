use super::*;
use kassiber_envelope::AppError;
use kassiber_transfers::{legacy_refresh, Scope, Sources};
use serde_json::Value;
use std::cell::RefCell;

pub use legacy_refresh::ConsolidationResult;

#[derive(Debug)]
pub struct LegacyRefresh {
    pub consolidation: ConsolidationResult,
    pub profile: Value,
}

struct Clock<'a> {
    callback: RefCell<&'a mut dyn FnMut() -> Result<String, Refusal>>,
    refusal: RefCell<Option<Refusal>>,
}

impl Sources for Clock<'_> {
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        match (self.callback.borrow_mut())() {
            Ok(now) => Ok(now),
            Err(error) => {
                let message = error.message.clone();
                *self.refusal.borrow_mut() = Some(error);
                Err(AppError::new("internal_error", message))
            }
        }
    }
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        Err(AppError::new(
            "internal_error",
            "legacy refresh requested entropy",
        ))
    }
}

pub fn refresh_drifted_legacy_pair_components(
    conn: &Connection,
    profile: &Value,
    clock: &mut dyn FnMut() -> Result<String, Refusal>,
) -> Result<LegacyRefresh, Refusal> {
    require_writer(conn)?;
    atomic(conn, "r4_journal_legacy", || {
        let sources = Clock {
            callback: RefCell::new(clock),
            refusal: RefCell::new(None),
        };
        let scope = Scope {
            workspace_id: profile["workspace_id"]
                .as_str()
                .ok_or_else(|| Refusal::new("KeyError", "workspace_id"))?,
            profile_id: profile["id"]
                .as_str()
                .ok_or_else(|| Refusal::new("KeyError", "id"))?,
        };
        let consolidation =
            legacy_refresh::refresh_drifted_legacy_pair_components(conn, &scope, &sources)
                .map_err(|e| {
                    sources
                        .refusal
                        .borrow_mut()
                        .take()
                        .unwrap_or_else(|| match e {
                            legacy_refresh::Error::App(e) => app(e),
                            legacy_refresh::Error::Python { family, message } => {
                                Refusal::new(family, message)
                            }
                            legacy_refresh::Error::Sqlite(e) => e.into(),
                        })
                })?;
        let profile = if consolidation.changed() {
            sources::rows(
                conn,
                "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
                [scope.profile_id, scope.workspace_id],
            )?
            .into_iter()
            .next()
            .ok_or_else(|| {
                Refusal::new(
                    "RuntimeError",
                    "journal profile disappeared during legacy refresh",
                )
            })?
        } else {
            profile.clone()
        };
        Ok(LegacyRefresh {
            consolidation,
            profile,
        })
    })
}
