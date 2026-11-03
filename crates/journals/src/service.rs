//! Production book binding for journal processing. No connection discovery,
//! provider calls, or transport output belongs to this service.
use crate::{loader, prebuild, process, storage, Refusal};
use kassiber_envelope::{get, py_str, AppError};
use kassiber_pyjson::Value;
use kassiber_setup::references::{resolve, Entity};
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::{Sources, SystemSources};

type DurableFile = (std::path::PathBuf, Option<bitcoin_hashes::sha256::Hash>);
/// A refusal may be replayed only after rollback and an exact comparison of
/// the durable database files. Lock-index SHM is not persisted book state.
pub struct ReplayGuard(Option<Vec<DurableFile>>);
impl ReplayGuard {
    pub fn capture(conn: &Connection) -> Self {
        let Some(path) = conn.path().filter(|p| !p.is_empty()) else {
            return Self(None);
        };
        Self::capture_path(std::path::Path::new(path))
    }
    pub fn capture_path(path: &std::path::Path) -> Self {
        Self(
            ["", "-wal", "-journal"]
                .into_iter()
                .map(|suffix| {
                    let mut file = path.as_os_str().to_owned();
                    file.push(suffix);
                    let file = std::path::PathBuf::from(file);
                    digest(&file).map(|hash| (file, hash))
                })
                .collect::<std::io::Result<_>>()
                .ok(),
        )
    }
    pub fn unchanged(&self, conn: &Connection) -> bool {
        conn.is_autocommit() && self.files_unchanged()
    }
    pub fn files_unchanged(&self) -> bool {
        self.0.as_ref().is_some_and(|files| {
            files
                .iter()
                .all(|(file, before)| digest(file).is_ok_and(|after| &after == before))
        })
    }
}

fn digest(path: &std::path::Path) -> std::io::Result<Option<bitcoin_hashes::sha256::Hash>> {
    use bitcoin_hashes::{sha256, Hash, HashEngine};
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut hash = sha256::Hash::engine();
    let mut buffer = [0; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.input(&buffer[..read]);
    }
    Ok(Some(sha256::Hash::from_engine(hash)))
}

/// Resolve selectors using the same ID/label/context rules as other services,
/// retaining every stored profile value through the loader's checked adapter.
fn profile(
    conn: &Connection,
    workspace: Option<&Value>,
    profile: Option<&Value>,
) -> Result<serde_json::Value, Refusal> {
    let workspace = resolve(conn, Entity::Workspace, workspace, None).map_err(prebuild::app)?;
    let workspace_id = py_str(get(&workspace, "id").expect("resolved workspace id"));
    let profile =
        resolve(conn, Entity::Profile, profile, Some(&workspace_id)).map_err(prebuild::app)?;
    loader::convert(profile).map_err(|error| Refusal::new("NotImplementedError", error.to_string()))
}

/// The production entry supplies real entropy and the clock at each lifecycle
/// call point. Tests may use `process_with_sources` without changing routing.
pub fn process<'a>(
    conn: &'a Connection,
    workspace: Option<&Value>,
    selected_profile: Option<&Value>,
    progress: &mut impl FnMut(&'static str),
) -> Result<process::Committed<'a>, Refusal> {
    process_with_sources(
        conn,
        workspace,
        selected_profile,
        &SystemSources,
        progress,
        &mut (),
    )
}

pub fn process_with_sources<'a>(
    conn: &'a Connection,
    workspace: Option<&Value>,
    selected_profile: Option<&Value>,
    sources: &impl Sources,
    progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
) -> Result<process::Committed<'a>, Refusal> {
    process::process_native(
        conn,
        |conn| profile(conn, workspace, selected_profile),
        kassiber_pydecimal::Context::RP2,
        sources,
        progress,
        writes,
    )
}

pub fn refresh<'a>(
    conn: &'a Connection,
    workspace: Option<&Value>,
    selected_profile: Option<&Value>,
    options: process::RefreshOptions,
    progress: &mut impl FnMut(&'static str),
) -> Result<process::Committed<'a>, Refusal> {
    refresh_with_sources(
        conn,
        workspace,
        selected_profile,
        &SystemSources,
        options,
        progress,
        &mut (),
    )
}

pub fn refresh_with_sources<'a>(
    conn: &'a Connection,
    workspace: Option<&Value>,
    selected_profile: Option<&Value>,
    sources: &impl Sources,
    options: process::RefreshOptions,
    progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
) -> Result<process::Committed<'a>, Refusal> {
    let initial = profile(conn, workspace, selected_profile)?;
    let workspace_id = initial["workspace_id"]
        .as_str()
        .ok_or_else(|| Refusal::new("KeyError", "workspace_id"))?;
    let profile_id = initial["id"]
        .as_str()
        .ok_or_else(|| Refusal::new("KeyError", "id"))?;
    let workspace = kassiber_envelope::s(workspace_id);
    let selected_profile = kassiber_envelope::s(profile_id);
    let progress = std::cell::RefCell::new(progress);
    process::refresh_native(
        conn,
        &kassiber_transfers::Scope {
            workspace_id,
            profile_id,
        },
        |conn| profile(conn, Some(&workspace), Some(&selected_profile)),
        sources,
        options,
        &mut |phase| (progress.borrow_mut())(phase),
        &mut |phase| (progress.borrow_mut())(phase),
        writes,
    )
}

pub fn refresh_with_sources_checked<'a>(
    conn: &'a Connection,
    selected_profile: &str,
    sources: &impl Sources,
    options: process::RefreshOptions,
    phase: &mut impl FnMut(&'static str) -> Result<(), Refusal>,
    journal_progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
) -> Result<process::Committed<'a>, Refusal> {
    let selected = kassiber_envelope::s(selected_profile);
    let workspace_id: String = conn.query_row(
        "SELECT workspace_id FROM profiles WHERE id=?",
        [selected_profile],
        |row| row.get(0),
    )?;
    let workspace = kassiber_envelope::s(workspace_id.as_str());
    process::refresh_native_checked(
        conn,
        &kassiber_transfers::Scope {
            workspace_id: &workspace_id,
            profile_id: selected_profile,
        },
        |conn| profile(conn, Some(&workspace), Some(&selected)),
        sources,
        options,
        phase,
        journal_progress,
        writes,
    )
}

/// Match the reference's outer exception projection. Refusal details remain
/// local until the transport applies its ordinary redaction boundary.
/// `auto_price_transactions_from_rates_cache(conn, profile)` under Python's RP2
/// decimal context, inside the caller's write transaction.
pub fn auto_price(
    conn: &Connection,
    profile_id: &str,
    fiat_currency: &str,
) -> Result<usize, AppError> {
    crate::prebuild::auto_price_transactions_from_rates_cache(
        conn,
        &crate::prebuild::PricingScope {
            profile_id,
            fiat_currency,
            decimal_context: kassiber_pydecimal::Context::RP2,
        },
    )
    .map_err(application_error)
}

pub fn application_error(refusal: Refusal) -> AppError {
    if refusal.family == "AppError" {
        AppError {
            code: refusal.code.clone().unwrap_or_else(|| "app_error".into()),
            message: refusal.message.clone(),
            hint: refusal.hint.clone().map(Into::into),
            message_codepoints: None,
            sqlite_origin: false,
            details: refusal.ordered_details.clone().or_else(|| {
                (!refusal.details.is_null())
                    .then(|| kassiber_custody::codec::from_json(&refusal.details).ok())
                    .flatten()
            }),
            retryable: refusal.retryable.unwrap_or(false),
        }
    } else {
        AppError::new("internal_error", refusal.message.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::s;

    fn book() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings(key TEXT,value TEXT);
            CREATE TABLE workspaces(id TEXT,label TEXT);
            CREATE TABLE profiles(id TEXT,workspace_id TEXT,label TEXT,stored);
            INSERT INTO settings VALUES('context_workspace','w'),('context_profile','p');
            INSERT INTO workspaces VALUES('w','Book'),('other','Other');
            INSERT INTO profiles VALUES('p','w','Personal',7.25),('foreign','other','Foreign',0);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn scope_uses_context_ids_and_labels_and_retains_stored_scalars() {
        let conn = book();
        let context = profile(&conn, None, None).unwrap();
        assert_eq!(context["stored"], serde_json::json!(7.25));
        assert_eq!(
            profile(&conn, Some(&s("Book")), Some(&s("Personal"))).unwrap(),
            context
        );
        assert_eq!(
            profile(&conn, Some(&s("w")), Some(&s("p"))).unwrap(),
            context
        );
        conn.execute("UPDATE profiles SET stored=?", [f64::INFINITY])
            .unwrap();
        assert_eq!(
            kassiber_custody::codec::from_json(&profile(&conn, None, None).unwrap()["stored"])
                .unwrap(),
            Value::Float(f64::INFINITY)
        );
    }

    #[test]
    fn foreign_and_ambiguous_profile_selectors_refuse_without_writes() {
        let conn = book();
        conn.execute(
            "INSERT INTO profiles VALUES('duplicate','w','PERSONAL',1)",
            [],
        )
        .unwrap();
        let before = conn.total_changes();
        for (selector, code) in [("foreign", "not_found"), ("personal", "validation")] {
            let error = profile(&conn, None, Some(&s(selector))).unwrap_err();
            assert_eq!(error.code.as_deref(), Some(code));
            assert_eq!(conn.total_changes(), before);
            assert!(conn.is_autocommit());
        }
    }

    #[test]
    fn replay_guard_requires_complete_rollback_and_identical_durable_files() {
        for cipher in [false, true] {
            let root = std::env::temp_dir()
                .join(format!("journal-replay-{}-{cipher}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("book.sqlite3");
            let conn = Connection::open(&path).unwrap();
            if cipher {
                conn.pragma_update(None, "key", "synthetic-replay-only")
                    .unwrap();
            }
            conn.execute_batch(
                "PRAGMA journal_mode=DELETE;CREATE TABLE state(v);INSERT INTO state VALUES(1)",
            )
            .unwrap();
            let guard = ReplayGuard::capture(&conn);
            assert!(guard.unchanged(&conn));
            conn.execute_batch("BEGIN IMMEDIATE;UPDATE state SET v=2")
                .unwrap();
            assert!(!guard.unchanged(&conn));
            conn.execute_batch("ROLLBACK").unwrap();
            assert!(guard.unchanged(&conn));
            conn.execute_batch("UPDATE state SET v=2").unwrap();
            assert!(!guard.unchanged(&conn));
            drop(conn);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
