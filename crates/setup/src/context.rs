use super::*;
use references::{resolve, Entity};

pub fn snapshot(conn: &Connection) -> Result<Value> {
    let w = rows(
        conn,
        "SELECT id,label FROM workspaces WHERE id=?",
        [setting(conn, "context_workspace")?.unwrap_or_default()],
    )?
    .into_iter()
    .next();
    let p = rows(
        conn,
        "SELECT id,workspace_id,label FROM profiles WHERE id=?",
        [setting(conn, "context_profile")?.unwrap_or_default()],
    )?
    .into_iter()
    .next()
    .filter(|p| {
        w.as_ref()
            .is_none_or(|w| text(p, "workspace_id") == text(w, "id"))
    });
    let field = |row: &Option<Value>, key: &str| {
        row.as_ref()
            .and_then(|v| get(v, key))
            .cloned()
            .unwrap_or_else(|| s(""))
    };
    Ok(dict([
        ("workspace_id", field(&w, "id")),
        ("workspace_label", field(&w, "label")),
        ("profile_id", field(&p, "id")),
        ("profile_label", field(&p, "label")),
    ]))
}
pub fn execute(conn: &Connection, command: &str, args: &Value) -> Result<Value> {
    if !["context.show", "context.current", "context.set"].contains(&command) {
        return Err(AppError::new(
            "not_implemented",
            "context operation is not implemented",
        ));
    }
    if command == "context.set" {
        if flag(args, "workspace") {
            let w = references::workspace(conn, args)?;
            let wid = text(&w, "id");
            set_setting(conn, "context_workspace", &wid)?;
            if flag(args, "profile") {
                let p = resolve(conn, Entity::Profile, get(args, "profile"), Some(&wid))?;
                set_setting(conn, "context_profile", &text(&p, "id"))?;
            } else if let Some(pid) = setting(conn, "context_profile")?.filter(|v| !v.is_empty()) {
                if rows(
                    conn,
                    "SELECT 1 FROM profiles WHERE id=? AND workspace_id=?",
                    [&pid, &wid],
                )?
                .is_empty()
                {
                    set_setting(conn, "context_profile", "")?;
                }
            }
        } else if flag(args, "profile") {
            let (_, p) = references::scope(conn, args)?;
            set_setting(conn, "context_profile", &text(&p, "id"))?;
        } else {
            return Err(AppError::new(
                "app_error",
                "Provide --workspace and/or --profile",
            ));
        }
    }
    snapshot(conn)
}
