use super::*;

pub fn create(conn: &Connection, label: &Value, sources: &dyn Sources) -> Result<Value> {
    let label = py_str(label);
    let id = sources.uuid(conn)?;
    conn.execute(
        "INSERT INTO workspaces(id,label,created_at) VALUES(?,?,?)",
        [&id, &label, &sources.now(conn)?],
    )
    .map_err(|e| {
        if integrity(&e) {
            AppError::new("conflict", format!("Workspace '{label}' already exists"))
                .with_hint("Choose a different workspace label or use the existing workspace.")
        } else {
            sql_error(e)
        }
    })?;
    set_setting(conn, "context_workspace", &id)?;
    set_setting(conn, "context_profile", "")?;
    one(conn, "SELECT * FROM workspaces WHERE id=?", [id])
}
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    match command {
        "workspaces.create" => create(conn, get(args, "label").unwrap_or(&Value::Null), sources),
        "workspaces.list" => {
            let current = setting(conn, "context_workspace")?.unwrap_or_default();
            let values = rows(
                conn,
                "SELECT id,label,created_at FROM workspaces ORDER BY created_at ASC",
                [],
            )?;
            Ok(Value::List(
                values
                    .into_iter()
                    .map(|r| {
                        dict([
                            ("id", get(&r, "id").unwrap().clone()),
                            ("label", get(&r, "label").unwrap().clone()),
                            (
                                "current",
                                s(if text(&r, "id") == current { "yes" } else { "" }),
                            ),
                            ("created_at", get(&r, "created_at").unwrap().clone()),
                        ])
                    })
                    .collect(),
            ))
        }
        _ => Err(AppError::new(
            "not_implemented",
            "workspace operation is not implemented",
        )),
    }
}
