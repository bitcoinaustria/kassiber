//! Local project catalog services. These never open or unlock a book.
use super::*;
use kassiber_paths::{Catalog, Context, ProjectEntry, DB_FILENAME};
use std::path::{Path, PathBuf};

fn catalog_path(context: &Context) -> Result<PathBuf> {
    Ok(context
        .default_state_root()
        .map_err(storage_error)?
        .join("config/projects.json"))
}
fn read(context: &Context) -> Result<Catalog> {
    kassiber_paths::load_catalog(&catalog_path(context)?, context).map_err(storage_error)
}
fn write(context: &Context, catalog: &Catalog) -> Result<()> {
    kassiber_store::layout::write_catalog(&catalog_path(context)?, catalog).map_err(storage_error)
}
fn encrypted(entry: &ProjectEntry) -> Result<bool> {
    let database = kassiber_paths::resolve_database_path(&entry.path.join("data"));
    Ok(database.exists()
        && std::fs::metadata(&database)
            .map_err(|e| io_error(e, &database))?
            .len()
            > 0
        && !kassiber_paths::looks_like_plaintext(&database).map_err(storage_error)?)
}
fn refresh(context: &Context) -> Result<Catalog> {
    let mut catalog = read(context)?;
    let mut changed = false;
    for entry in &mut catalog.projects {
        let current = encrypted(entry)?;
        changed |= entry.encrypted != current;
        entry.encrypted = current;
    }
    if changed {
        write(context, &catalog)?;
    }
    Ok(catalog)
}
pub fn get_project(context: &Context, reference: &str) -> Result<ProjectEntry> {
    let wanted = kassiber_paths::sanitize_project_id(reference).map_err(storage_error)?;
    refresh(context)?
        .projects
        .into_iter()
        .find(|p| p.id == wanted)
        .ok_or_else(|| {
            AppError::new(
                "project_not_found",
                format!(
                    "project {} is not registered",
                    kassiber_pytext::repr_str(reference)
                ),
            )
            .with_details(dict([("project_id", s(reference))]))
        })
}
pub fn payload(entry: &ProjectEntry) -> Value {
    let path = |path: &Path| s(path.to_string_lossy().as_ref());
    dict([
        ("id", s(entry.id.as_str())),
        ("name", s(entry.name.as_str())),
        ("path", path(&entry.path)),
        ("data_root", path(&entry.path.join("data"))),
        (
            "database",
            path(&kassiber_paths::resolve_database_path(
                &entry.path.join("data"),
            )),
        ),
        ("encrypted", Value::Bool(entry.encrypted)),
        (
            "last_opened_at",
            entry.last_opened_at.as_deref().map_or(Value::Null, s),
        ),
    ])
}
/// Refresh after explicit encrypted creation, preserving `--no-select` and
/// Python's last-opened behavior. Other catalog entries retain their order.
pub fn created_encrypted(
    context: &Context,
    reference: &str,
    select: bool,
    now: &str,
) -> Result<Value> {
    let mut entry = get_project(context, reference)?;
    let mut catalog = read(context)?;
    if select {
        entry.last_opened_at = Some(now.into());
        catalog.selected_project_id = Some(entry.id.clone());
    }
    for saved in &mut catalog.projects {
        if saved.id == entry.id {
            *saved = entry.clone();
        }
    }
    write(context, &catalog)?;
    Ok(payload(&entry))
}
pub fn execute(context: &Context, command: &str, args: &Value) -> Result<Value> {
    match command {
        "projects.list" => {
            // Read selection before list_projects refreshes encryption metadata.
            let selected = optional(args, "project")
                .filter(|v| truthy(v))
                .map(py_str)
                .or(read(context)?.selected_project_id);
            let catalog = refresh(context)?;
            let projects = catalog
                .projects
                .iter()
                .map(|p| {
                    let mut value = payload(p);
                    replace(
                        &mut value,
                        "selected",
                        Value::Bool(selected.as_ref() == Some(&p.id)),
                    );
                    value
                })
                .collect();
            Ok(dict([
                (
                    "selected_project_id",
                    selected.as_deref().map_or(Value::Null, s),
                ),
                ("projects", Value::List(projects)),
            ]))
        }
        "projects.show" => Ok(payload(&get_project(context, &text(args, "project_ref"))?)),
        "projects.select" => {
            let entry = get_project(context, &text(args, "project_ref"))?;
            let mut catalog = read(context)?;
            catalog.selected_project_id = Some(entry.id.clone());
            write(context, &catalog)?;
            Ok(payload(&entry))
        }
        "projects.create" => create(context, args).map(|entry| payload(&entry)),
        _ => Err(AppError::new(
            "not_implemented",
            "project operation is not implemented",
        )),
    }
}
fn create(context: &Context, args: &Value) -> Result<ProjectEntry> {
    let name = text(args, "name");
    let requested_id = optional(args, "project_id")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| name.clone());
    let id = kassiber_paths::sanitize_project_id(&requested_id).map_err(storage_error)?;
    let root = if let Some(path) = optional(args, "path") {
        context
            .expand_user(Path::new(&py_str(path)))
            .map_err(storage_error)?
    } else {
        context
            .default_state_root()
            .map_err(storage_error)?
            .join("projects")
            .join(&id)
    };
    let mut catalog = read(context)?;
    if let Some(existing) = catalog.projects.iter().find(|p| p.id == id) {
        return Err(AppError::new(
            "project_exists",
            format!("project {} already exists", kassiber_pytext::repr_str(&id)),
        )
        .with_details(dict([
            ("project_id", s(id)),
            ("project_root", s(existing.path.to_string_lossy().as_ref())),
        ])));
    }
    let database = root.join("data").join(DB_FILENAME);
    if database.exists()
        && std::fs::metadata(&database)
            .map_err(|e| io_error(e, &database))?
            .len()
            > 0
    {
        return Err(AppError::new(
            "database_exists",
            format!(
                "refusing to create a project over existing database at {}",
                database.display()
            ),
        )
        .with_details(dict([("database", s(database.to_string_lossy().as_ref()))])));
    }
    directory(&root)?;
    for name in ["data", "config", "exports", "attachments"] {
        directory(&root.join(name))?;
    }
    let stripped = kassiber_pytext::strip(&name);
    let mut entry = ProjectEntry {
        id: id.clone(),
        name: if stripped.is_empty() {
            id
        } else {
            stripped.into()
        },
        path: root,
        encrypted: false,
        last_opened_at: None,
    };
    entry.encrypted = encrypted(&entry)?;
    if !flag(args, "no_select") || catalog.selected_project_id.is_none() {
        catalog.selected_project_id = Some(entry.id.clone());
    }
    catalog.projects.push(entry.clone());
    write(context, &catalog)?;
    Ok(entry)
}
pub(crate) fn io_error(error: std::io::Error, path: &Path) -> AppError {
    let message = if let Some(number) = error.raw_os_error() {
        let text = error.to_string();
        let suffix = format!(" (os error {number})");
        let label = if cfg!(windows) { "WinError" } else { "Errno" };
        format!(
            "[{label} {number}] {}: {}",
            text.strip_suffix(&suffix).unwrap_or(&text),
            kassiber_pytext::repr_str(&path.to_string_lossy())
        )
    } else if path.to_string_lossy().contains('\0') {
        "embedded null byte".into()
    } else {
        error.to_string()
    };
    AppError::new("internal_error", message)
}
/// pathlib.mkdir(parents=True, exist_ok=True), retaining the failing parent.
fn directory(path: &Path) -> Result<()> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(parent) = path.parent().filter(|parent| *parent != path) else {
                return Err(io_error(error, path));
            };
            directory(if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            })?;
            std::fs::create_dir(path)
                .or_else(|error| if path.is_dir() { Ok(()) } else { Err(error) })
                .map_err(|e| io_error(e, path))
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(error) => Err(io_error(error, path)),
    }
}
/// `list_projects()`: every entry, with encryption refreshed and persisted.
pub fn list(context: &Context) -> Result<Vec<ProjectEntry>> {
    Ok(refresh(context)?.projects)
}
/// `_upsert_project(entry, select)`: replace in place or append.
fn upsert(context: &Context, entry: &ProjectEntry, select: bool) -> Result<ProjectEntry> {
    let mut catalog = read(context)?;
    match catalog.projects.iter_mut().find(|p| p.id == entry.id) {
        Some(saved) => *saved = entry.clone(),
        None => catalog.projects.push(entry.clone()),
    }
    if select || catalog.selected_project_id.is_none() {
        catalog.selected_project_id = Some(entry.id.clone());
    }
    write(context, &catalog)?;
    Ok(entry.clone())
}
/// `refresh_project_metadata(id, data_root=...)` and, with a timestamp,
/// `mark_project_opened`: encryption is read at `data_root`, not the entry.
pub fn record_opened(
    context: &Context,
    reference: &str,
    data_root: &Path,
    last_opened_at: Option<&str>,
    select: bool,
) -> Result<ProjectEntry> {
    let mut entry = get_project(context, reference)?;
    let data_root = context.expand_user(data_root).map_err(storage_error)?;
    let database = kassiber_paths::resolve_database_path(&data_root);
    entry.encrypted = database.exists()
        && std::fs::metadata(&database)
            .map_err(|e| io_error(e, &database))?
            .len()
            > 0
        && !kassiber_paths::looks_like_plaintext(&database).map_err(storage_error)?;
    if let Some(now) = last_opened_at {
        entry.last_opened_at = Some(now.into());
    }
    upsert(context, &entry, select)
}
/// `set_selected_project(id, last_opened_at)`.
pub fn set_selected(
    context: &Context,
    reference: &str,
    last_opened_at: Option<&str>,
) -> Result<ProjectEntry> {
    let mut entry = get_project(context, reference)?;
    let mut catalog = read(context)?;
    catalog.selected_project_id = Some(entry.id.clone());
    for saved in &mut catalog.projects {
        if saved.id == entry.id {
            saved.encrypted = encrypted(saved)?;
            if let Some(now) = last_opened_at {
                saved.last_opened_at = Some(now.into());
            }
            entry = saved.clone();
        }
    }
    write(context, &catalog)?;
    Ok(entry)
}
/// The daemon's `create_project(..., replace_existing=False,
/// allow_existing_database=False)` at the default location.
pub fn create_for_desktop(
    context: &Context,
    name: &str,
    project_id: Option<&str>,
    select: bool,
) -> Result<ProjectEntry> {
    let mut args = vec![("name", s(name)), ("no_select", Value::Bool(!select))];
    if let Some(id) = project_id {
        args.push(("project_id", s(id)));
    }
    create(context, &dict(args))
}
