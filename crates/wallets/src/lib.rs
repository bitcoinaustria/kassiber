//! Watch-only wallet services. Local operations are inert; compatibility
//! discovery requires its own explicit action grant and the network gate.
pub mod config;
pub mod file_import;
mod silent_payments;
mod snapshot;
pub mod time;
pub use snapshot::snapshot;

pub mod acquisition;
pub mod analysis_features;
pub mod analysis_index;
pub mod analysis_ops;
pub mod bdk;
mod bdk_state;
pub mod bitcoin_records;
pub mod compatibility_cache;
pub mod composition;
mod derive;
mod lifecycle;
mod network;
pub mod psbt;
pub mod psbt_privacy;
pub use lifecycle::{execute, execute_with_creation_identity, CreationIdentity, Operation};
pub mod descriptors;
pub mod fetch;
pub mod freshness;
pub mod material;
pub mod observer_apply;
pub mod observer_state;
pub mod publication;
pub mod scan;
pub mod sync;
pub use derive::{derive_wallet, Derive};
mod preview;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{self, Connection, OptionalExtension, Row};
pub use preview::preview;

/// Map database failures without disclosing SQL, bound values or wallet material.
pub fn database_error(error: rusqlite::Error) -> AppError {
    let error = kassiber_store::map_sqlite_error(error);
    AppError {
        code: error.code.to_owned(),
        message: error.message.to_owned(),
        hint: error.hint.map(Box::from),
        details: kassiber_pytext::loads(&error.details.to_string()).ok(),
        retryable: error.retryable,
        message_codepoints: None,
        sqlite_origin: true,
    }
}

pub(crate) fn unicode_binding_error(value: &kassiber_pyjson::PyStr) -> AppError {
    let points = value.code_points().collect::<Vec<_>>();
    let start = points
        .iter()
        .position(|p| (0xd800..=0xdfff).contains(p))
        .unwrap();
    let count = points[start..]
        .iter()
        .take_while(|p| (0xd800..=0xdfff).contains(*p))
        .count();
    let position = if count == 1 {
        format!("character '\\u{:04x}' in position {start}", points[start])
    } else {
        format!("characters in position {start}-{}", start + count - 1)
    };
    AppError::new(
        "internal_error",
        format!("'utf-8' codec can't encode {position}: surrogates not allowed"),
    )
}

/// The deliberately small projection needed by import filters and deletion.
#[derive(Clone, Debug, PartialEq)]
pub struct Wallet {
    pub id: String,
    pub label: String,
    pub account_code: Option<String>,
}

fn wallet(row: &Row<'_>) -> rusqlite::Result<Wallet> {
    Ok(Wallet {
        id: row.get(0)?,
        label: row.get(1)?,
        account_code: row.get(2)?,
    })
}

/// Python's `repo.wallets.resolve_wallet`: exact id before case-insensitive label,
/// always within the selected profile. Ambiguous labels never select a keeper.
pub fn resolve(conn: &Connection, profile_id: &str, reference: &str) -> Result<Wallet, AppError> {
    let normalized = kassiber_pytext::strip(reference);
    let select =
        "SELECT w.id,w.label,a.code FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id";
    if let Some(found) = conn
        .query_row(
            &format!("{select} WHERE w.profile_id=? AND w.id=? LIMIT 1"),
            [profile_id, normalized],
            wallet,
        )
        .optional()
        .map_err(database_error)?
    {
        return Ok(found);
    }
    let mut stmt = conn
        .prepare(&format!(
        "{select} WHERE w.profile_id=? AND lower(w.label)=lower(?) ORDER BY w.label ASC,w.id ASC"
    ))
        .map_err(database_error)?;
    let matches = stmt
        .query_map([profile_id, normalized], wallet)
        .map_err(database_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?;
    match matches.as_slice() {
        [found] => Ok(found.clone()),
        [] => Err(AppError::new(
            "not_found",
            format!("Wallet '{reference}' not found"),
        )),
        _ => Err(AppError::new(
            "validation",
            format!("Wallet label '{reference}' is ambiguous"),
        )
        .with_hint("Use the wallet id instead of the non-unique label.")
        .with_details(dict([(
            "matches",
            Value::List(
                matches
                    .into_iter()
                    .map(|w| {
                        dict([
                            ("id", s(w.id)),
                            ("label", s(w.label)),
                            ("account_code", w.account_code.map_or(Value::Null, s)),
                        ])
                    })
                    .collect(),
            ),
        )]))),
    }
}

/// Shared journal readiness invalidation. Never permit cached report delivery
/// after removing observations or their source.
pub fn invalidate_journals(conn: &Connection, profile_id: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?",
        [profile_id],
    ).map_err(database_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_prefers_id_refuses_ambiguity_and_scopes_profiles() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE accounts(id TEXT,code TEXT);CREATE TABLE wallets(id TEXT,profile_id TEXT,account_id TEXT,label TEXT);
            INSERT INTO wallets VALUES('a','p',NULL,'Cold'),('b','p',NULL,'COLD'),('Cold','p',NULL,'Exact'),('other','q',NULL,'Foreign');").unwrap();
        assert_eq!(resolve(&conn, "p", "\u{1c}Cold ").unwrap().id, "Cold");
        let ambiguous = resolve(&conn, "p", "cold").unwrap_err();
        assert_eq!(ambiguous.code, "validation");
        assert_eq!(
            ambiguous.details,
            Some(dict([(
                "matches",
                Value::List(vec![
                    dict([
                        ("id", s("b")),
                        ("label", s("COLD")),
                        ("account_code", Value::Null)
                    ]),
                    dict([
                        ("id", s("a")),
                        ("label", s("Cold")),
                        ("account_code", Value::Null)
                    ]),
                ])
            )]))
        );
        assert_eq!(resolve(&conn, "p", "other").unwrap_err().code, "not_found");
        assert_eq!(
            resolve(&conn, "p", "Foreign").unwrap_err().code,
            "not_found"
        );
    }
}
