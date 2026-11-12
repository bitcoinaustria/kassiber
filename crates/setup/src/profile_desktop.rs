//! Desktop policy mutations delegate to the same profile services as the CLI.
use super::*;
pub(super) fn required(args: &Value, key: &str, message: &str, hint: &str) -> Result<String> {
    match get(args, key) {
        Some(Value::Str(s))
            if s.as_str()
                .is_some_and(|s| !kassiber_pytext::strip(s).is_empty()) =>
        {
            Ok(kassiber_pytext::strip(&py_str(&Value::Str(s.clone()))).to_owned())
        }
        _ => Err(AppError::new("validation", message).with_hint(hint)),
    }
}
fn optional_string(args: &Value, key: &str) -> Result<Option<String>> {
    match optional(args, key) {
        None => Ok(None),
        Some(Value::Str(t)) => Ok(t
            .as_str()
            .map(kassiber_pytext::strip)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)),
        _ => Err(AppError::new(
            "validation",
            format!("{key} must be a string"),
        )),
    }
}
fn selected(conn: &Connection, id: &str, hint: &str) -> Result<Value> {
    rows(conn, "SELECT * FROM profiles WHERE id=?", [id])?
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError::new("validation", "Book not found.")
                .with_hint(hint)
                .with_details(dict([("profile_id", s(id))]))
        })
}
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    if command == "ui.profiles.create" {
        return create(conn, args, sources);
    }
    let action = command.rsplit('.').next().unwrap_or_default();
    let hint = match action {
        "rename" => "Choose the book to rename.",
        "update" => "Choose the book to update.",
        _ => "Select a book from the current books snapshot.",
    };
    let id = required(args, "profile_id", "Book selection is missing.", hint)?;
    let name = if action == "rename" {
        Some(required(
            args,
            "label",
            "Book name is required.",
            "Enter a book name.",
        )?)
    } else {
        None
    };
    let mut updates = dict::<&str>([]);
    if action == "update" {
        for (key, message, hint) in [
            (
                "gains_algorithm",
                "Accounting method is required.",
                "Choose an accounting method for the book.",
            ),
            (
                "cost_basis_pool_scope",
                "Cost-basis pool scope is required.",
                "Choose a supported cost-basis pool scope for the book.",
            ),
            (
                "tax_country",
                "Region is required.",
                "Choose a supported region for the book.",
            ),
        ] {
            if optional(args, key).is_some() {
                replace(&mut updates, key, s(required(args, key, message, hint)?));
            }
        }
        if !truthy(&updates) {
            return Err(AppError::new(
                "validation",
                "No book policy change was provided.",
            ));
        }
    }
    let p = selected(
        conn,
        &id,
        if action == "switch" {
            "Refresh books and choose an existing book."
        } else {
            "Choose an existing book."
        },
    )?;
    let wid = text(&p, "workspace_id");
    match action {
        "switch" => {
            let w = one(conn, "SELECT * FROM workspaces WHERE id=?", [&wid])?;
            set_setting(conn, "context_workspace", &wid)?;
            set_setting(conn, "context_profile", &id)?;
            Ok(dict([
                ("activeProfileId", s(id.as_str())),
                ("activeWorkspaceId", s(wid.as_str())),
                (
                    "profile",
                    dict([("id", s(id)), ("name", get(&p, "label").unwrap().clone())]),
                ),
                (
                    "workspace",
                    dict([("id", s(wid)), ("name", get(&w, "label").unwrap().clone())]),
                ),
            ]))
        }
        "rename" => {
            let name = name.unwrap();
            conn.execute("UPDATE profiles SET label=? WHERE id=?", [&name, &id])
                .map_err(|e| {
                    if integrity(&e) {
                        AppError::new("conflict", "Book name already exists in this books set.")
                            .with_hint("Choose a different book name.")
                            .with_details(dict([
                                ("workspace_id", s(wid.as_str())),
                                ("label", s(name.as_str())),
                            ]))
                    } else {
                        sql_error(e)
                    }
                })?;
            Ok(dict([
                ("profile", dict([("id", s(id)), ("name", s(name))])),
                ("workspace", dict([("id", s(wid))])),
            ]))
        }
        "update" => {
            replace(&mut updates, "workspace", s(wid));
            replace(&mut updates, "profile", s(id));
            profiles::execute(conn, "profiles.set", &updates, sources)
        }
        _ => Err(AppError::new(
            "not_implemented",
            "profile desktop operation is not implemented",
        )),
    }
}
fn create(conn: &Connection, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let wid = required(
        args,
        "workspace_id",
        "Books set selection is missing.",
        "Choose the books set that should own the new book.",
    )?;
    let label = required(
        args,
        "label",
        "Book name is required.",
        "Enter a book name.",
    )?;
    let source = if optional(args, "source_profile_id").is_some() {
        Some(required(
            args,
            "source_profile_id",
            "Book settings source is invalid.",
            "Choose an existing book to copy settings from.",
        )?)
    } else {
        None
    };
    let current = setting(conn, "context_profile")?.unwrap_or_default();
    let mut defaults = if let Some(id) = &source {
        rows(
            conn,
            "SELECT * FROM profiles WHERE workspace_id=? AND id=?",
            [&wid, id],
        )?
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError::new("validation", "source book not found in books set")
                .with_hint("Choose a book from the same books set as the new book.")
                .with_details(dict([("source_profile_id", s(id.as_str()))]))
        })?
    } else {
        let candidates = rows(
            conn,
            "SELECT * FROM profiles WHERE workspace_id=? ORDER BY created_at ASC,label ASC",
            [&wid],
        )?;
        candidates
            .iter()
            .find(|p| text(p, "id") == current)
            .or(candidates.first())
            .cloned()
            .unwrap_or_else(|| {
                dict([
                    ("fiat_currency", s("EUR")),
                    ("tax_country", s(kassiber_tax_countries::DEFAULT_CODE)),
                    ("tax_long_term_days", Value::Int(365.into())),
                    ("gains_algorithm", s("FIFO")),
                    ("bitcoin_rail_carrying_value", Value::Bool(true)),
                    (
                        "cost_basis_pool_scope",
                        s(
                            kassiber_tax_countries::lookup(kassiber_tax_countries::DEFAULT_CODE)
                                .unwrap()
                                .default_pool_scope,
                        ),
                    ),
                ])
            })
    };
    if source.is_none() {
        let country = optional_string(args, "tax_country")?;
        let algorithm = optional_string(args, "gains_algorithm")?;
        let pool = optional_string(args, "cost_basis_pool_scope")?;
        if let Some(country) = country {
            if country != text(&defaults, "tax_country") {
                replace(&mut defaults, "tax_country", s(country.as_str()));
                replace(&mut defaults, "tax_long_term_days", Value::Int(365.into()));
                // An unknown country keeps `global`; profiles.create refuses it.
                let scope = kassiber_tax_countries::lookup(
                    &kassiber_pytext::strip(&country).to_lowercase(),
                )
                .map_or("global", |entry| entry.default_pool_scope);
                replace(&mut defaults, "cost_basis_pool_scope", s(scope));
            }
        }
        if let Some(algorithm) = algorithm {
            replace(&mut defaults, "gains_algorithm", s(algorithm));
        }
        if let Some(pool) = pool {
            replace(&mut defaults, "cost_basis_pool_scope", s(pool));
        }
    }
    replace(&mut defaults, "workspace", s(wid.as_str()));
    replace(&mut defaults, "label", s(label));
    let p = profiles::execute(conn, "profiles.create", &defaults, sources)?;
    let w = one(conn, "SELECT * FROM workspaces WHERE id=?", [&wid])?;
    Ok(dict([
        ("activeProfileId", get(&p, "id").unwrap().clone()),
        ("activeWorkspaceId", s(wid.as_str())),
        (
            "profile",
            dict([
                ("id", get(&p, "id").unwrap().clone()),
                ("name", get(&p, "label").unwrap().clone()),
            ]),
        ),
        (
            "workspace",
            dict([("id", s(wid)), ("name", get(&w, "label").unwrap().clone())]),
        ),
        (
            "defaults",
            dict(
                [
                    "fiat_currency",
                    "tax_country",
                    "tax_long_term_days",
                    "gains_algorithm",
                    "cost_basis_pool_scope",
                ]
                .map(|k| (k, get(&p, k).unwrap().clone())),
            ),
        ),
    ]))
}
