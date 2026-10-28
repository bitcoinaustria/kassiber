//! Single resolver for the base Python `core.repo.context` contract. ID first,
//! then SQLite's case-insensitive label lookup; labels differing only by case
//! remain ambiguous. This deliberately follows the pre-phase-5 reference.
use super::*;

#[derive(Clone, Copy)]
pub enum Entity {
    Workspace,
    Profile,
}

pub fn resolve(
    conn: &Connection,
    entity: Entity,
    selector: Option<&Value>,
    workspace: Option<&str>,
) -> Result<Value> {
    let profile = matches!(entity, Entity::Profile);
    let (table, title, setting_key, noun) = if profile {
        ("profiles", "Profile", "context_profile", "profile")
    } else {
        ("workspaces", "Workspace", "context_workspace", "workspace")
    };
    let selected = selector
        .filter(|v| truthy(v))
        .map(py_str)
        .or(setting(conn, setting_key)?)
        .unwrap_or_default();
    let selected = kassiber_pytext::strip(&selected);
    if selected.is_empty() {
        return Err(AppError::new(
            "app_error",
            format!("No {noun} selected. Create one or run `kassiber context set --{noun} ...`."),
        ));
    }
    let scope = if profile { " AND workspace_id=?2" } else { "" };
    let query = |predicate: &str| -> Result<Vec<Value>> {
        let sql =
            format!("SELECT * FROM {table} WHERE {predicate}{scope} ORDER BY label ASC,id ASC");
        if profile {
            rows(conn, &sql, [selected, workspace.unwrap_or_default()])
        } else {
            rows(conn, &sql, [selected])
        }
    };
    if let Some(row) = query("id=?1")?.into_iter().next() {
        return Ok(row);
    }
    let matches = query("lower(label)=lower(?1)")?;
    if matches.len() == 1 {
        return Ok(matches.into_iter().next().expect("one match"));
    }
    let suffix = if profile {
        " in the selected workspace"
    } else {
        ""
    };
    if matches.is_empty() {
        return Err(AppError::new(
            "not_found",
            format!("{title} '{selected}' not found{suffix}"),
        ));
    }
    Err(AppError::new(
        "validation",
        format!("{title} label '{selected}' is ambiguous{suffix}"),
    )
    .with_hint(format!(
        "Use the {noun} id instead of the non-unique label."
    ))
    .with_details(dict([(
        "matches",
        Value::List(
            matches
                .iter()
                .map(|r| {
                    dict([
                        ("id", get(r, "id").unwrap().clone()),
                        ("label", get(r, "label").unwrap().clone()),
                    ])
                })
                .collect(),
        ),
    )])))
}

pub fn workspace(conn: &Connection, args: &Value) -> Result<Value> {
    resolve(conn, Entity::Workspace, get(args, "workspace"), None)
}
pub fn scope(conn: &Connection, args: &Value) -> Result<(Value, Value)> {
    let w = workspace(conn, args)?;
    let p = resolve(
        conn,
        Entity::Profile,
        get(args, "profile"),
        Some(&text(&w, "id")),
    )?;
    Ok((w, p))
}
