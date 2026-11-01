//! Production book binding for journal processing. No connection discovery,
//! provider calls, or transport output belongs to this service.
use crate::{loader, prebuild, process, storage, Refusal};
use kassiber_envelope::{get, py_str, AppError};
use kassiber_pyjson::Value;
use kassiber_setup::references::{resolve, Entity};
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::{Sources, SystemSources};

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

/// Match the reference's outer exception projection. Refusal details remain
/// local until the transport applies its ordinary redaction boundary.
pub fn application_error(refusal: Refusal) -> AppError {
    if refusal.family == "AppError" {
        AppError {
            code: refusal.code.clone().unwrap_or_else(|| "app_error".into()),
            message: refusal.message.clone(),
            hint: refusal.hint.clone(),
            details: (!refusal.details.is_null())
                .then(|| kassiber_custody::codec::from_json(&refusal.details).ok())
                .flatten(),
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
}
