use super::*;
use profile_desktop::required;

pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    match command {
        "ui.workspace.create" => {
            let label = required(
                args,
                "label",
                "Books set name is required.",
                "Enter a books set name.",
            )?;
            let w = workspaces::create(conn, &s(label), sources)?;
            Ok(dict([
                (
                    "workspace",
                    dict([
                        ("id", get(&w, "id").unwrap().clone()),
                        ("name", get(&w, "label").unwrap().clone()),
                        (
                            "created",
                            s(text(&w, "created_at").chars().take(10).collect::<String>()),
                        ),
                    ]),
                ),
                ("activeWorkspaceId", get(&w, "id").unwrap().clone()),
                ("activeProfileId", s("")),
            ]))
        }
        "ui.workspace.rename" => {
            let id = required(
                args,
                "workspace_id",
                "Books set selection is missing.",
                "Choose the books set to rename.",
            )?;
            let label = required(
                args,
                "label",
                "Books set name is required.",
                "Enter a books set name.",
            )?;
            if rows(conn, "SELECT id FROM workspaces WHERE id=?", [&id])?.is_empty() {
                return Err(AppError::new("validation", "Books set not found.")
                    .with_hint("Choose an existing books set.")
                    .with_details(dict([("workspace_id", s(id))])));
            }
            conn.execute("UPDATE workspaces SET label=? WHERE id=?", [&label, &id])
                .map_err(|e| {
                    if integrity(&e) {
                        AppError::new("conflict", "Books set name already exists.")
                            .with_hint("Choose a different books set name.")
                            .with_details(dict([
                                ("workspace_id", s(id.as_str())),
                                ("label", s(label.as_str())),
                            ]))
                    } else {
                        sql_error(e)
                    }
                })?;
            Ok(dict([(
                "workspace",
                dict([("id", s(id)), ("name", s(label))]),
            )]))
        }
        "ui.workspace.delete" => {
            let context = context::snapshot(conn)?;
            let id = text(&context, "workspace_id");
            let label = text(&context, "workspace_label");
            if id.is_empty() {
                return Err(AppError::new("state_not_ready","No current books set is selected.").with_hint("Reset the local UI identity if you only need to return to the Welcome flow."));
            }
            let profiles = rows(conn, "SELECT id FROM profiles WHERE workspace_id=?", [&id])?
                .iter()
                .map(|p| text(p, "id"))
                .collect::<Vec<_>>();
            sensitive::require_no_legacy(conn, &profiles, "deleting this books set")?;
            let mut counts = dict::<&str>([]);
            for table in ["profiles", "wallets", "transactions"] {
                let n: i64 = conn
                    .query_row(
                        &format!("SELECT count(*) FROM {table} WHERE workspace_id=?"),
                        [&id],
                        |r| r.get(0),
                    )
                    .map_err(sql_error)?;
                replace(&mut counts, table, Value::Int(n.into()));
            }
            conn.execute("DELETE FROM workspaces WHERE id=?", [&id])
                .map_err(sql_error)?;
            conn.execute(
                "DELETE FROM settings WHERE key IN ('context_workspace','context_profile')",
                [],
            )
            .map_err(sql_error)?;
            Ok(dict([
                ("deleted", Value::Bool(true)),
                ("workspace", dict([("id", s(id)), ("label", s(label))])),
                ("removed", counts),
            ]))
        }
        _ => Err(AppError::new(
            "not_implemented",
            "workspace desktop operation is not implemented",
        )),
    }
}
