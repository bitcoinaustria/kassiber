use super::*;
use kassiber_pyjson::{dumps, Options};

fn project(mut row: Value) -> Value {
    let filter = optional(&row, "filter_json")
        .filter(|v| truthy(v))
        .map(py_str)
        .and_then(|s| kassiber_pytext::loads(&s).ok())
        .unwrap_or_else(|| dict::<&str>([]));
    if let Value::Dict(fields) = &mut row {
        fields.retain(|(k, _)| *k != "filter_json".into());
    }
    // The wire order is fixed independently of SELECT *'s schema order.
    dict(
        [
            "id",
            "workspace_id",
            "profile_id",
            "surface",
            "name",
            "filter",
            "created_at",
            "updated_at",
        ]
        .map(|k| {
            (
                k,
                if k == "filter" {
                    filter.clone()
                } else {
                    get(&row, k).unwrap().clone()
                },
            )
        }),
    )
}
fn require(field: &str, value: &str) -> Result<()> {
    if kassiber_pytext::strip(value).is_empty() {
        Err(AppError::new(
            "validation",
            format!("Missing required field '{field}'"),
        ))
    } else {
        Ok(())
    }
}
// Python dict keys identify equal bool/int/float values with one retained key.
fn same_key(a: &kassiber_pyjson::Key, b: &kassiber_pyjson::Key) -> bool {
    use kassiber_pyjson::{BigInt, Key};
    let integer = |key: &Key| -> Option<BigInt> {
        match key {
            Key::Int(n) => Some(n.clone()),
            Key::Bool(v) => Some(u8::from(*v).into()),
            Key::Float(v) if v.is_finite() && v.fract() == 0.0 => {
                if *v == 0.0 {
                    return Some(0.into());
                }
                let bits = v.to_bits();
                let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
                let significand = BigInt::from((bits & ((1_u64 << 52) - 1)) | (1_u64 << 52));
                let n = if exponent >= 0 {
                    significand << exponent as usize
                } else {
                    significand >> (-exponent) as usize
                };
                Some(if *v < 0.0 { -n } else { n })
            }
            _ => None,
        }
    };
    match (integer(a), integer(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b || matches!((a,b),(Key::Float(x),Key::Float(y)) if x.is_nan() && y.is_nan()),
    }
}
fn filter_payload(value: &Value) -> Result<Value> {
    // dict(filter_payload or {}) also accepts an array of key/value pairs.
    let payload = if !truthy(value) {
        dict::<&str>([])
    } else if matches!(value, Value::Dict(_)) {
        value.clone()
    } else {
        let Value::List(items) = value else {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "'{}' object is not iterable",
                    kassiber_envelope::type_name(value)
                ),
            ));
        };
        let mut pairs = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let elements = match item {
                Value::List(v) => v.clone(),
                Value::Str(t) => t
                    .code_points()
                    .map(|c| {
                        Value::Str(
                            kassiber_pyjson::PyStr::from_code_points([c]).expect("valid point"),
                        )
                    })
                    .collect(),
                _ => {
                    return Err(AppError::new(
                        "internal_error",
                        format!(
                            "cannot convert dictionary update sequence element #{i} to a sequence"
                        ),
                    ))
                }
            };
            if elements.len() != 2 {
                return Err(AppError::new(
                    "internal_error",
                    format!(
                        "dictionary update sequence element #{i} has length {}; 2 is required",
                        elements.len()
                    ),
                ));
            }
            use kassiber_pyjson::Key;
            let key = match &elements[0] {
                Value::Str(t) => Key::Str(t.clone()),
                Value::Int(n) => Key::Int(n.clone()),
                Value::Bool(b) => Key::Bool(*b),
                Value::Null => Key::Null,
                Value::Float(n) => Key::Float(*n),
                value => {
                    return Err(AppError::new(
                        "internal_error",
                        format!("unhashable type: '{}'", kassiber_envelope::type_name(value)),
                    ))
                }
            };
            if let Some((_, slot)) = pairs.iter_mut().find(|(k, _)| same_key(k, &key)) {
                *slot = elements[1].clone();
            } else {
                pairs.push((key, elements[1].clone()));
            }
        }
        Value::Dict(pairs)
    };
    Ok(payload)
}
fn filter_json(payload: &Value) -> Result<String> {
    dumps(
        payload,
        &Options {
            sort_keys: true,
            ..Options::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))
}
fn find(conn: &Connection, pid: &str, id: &str) -> Result<Value> {
    rows(
        conn,
        "SELECT * FROM saved_views WHERE id=? AND profile_id=?",
        [id, pid],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Saved view '{id}' not found for this profile"),
        )
        .with_details(dict([("id", s(id))]))
    })
}
/// Library update operation retained for the `test_saved_views` Rust ports.
pub fn update(
    conn: &Connection,
    pid: &str,
    id: &str,
    name: Option<&str>,
    filter: Option<&Value>,
    sources: &dyn Sources,
) -> Result<Value> {
    let old = find(conn, pid, id)?;
    let name = name
        .map(str::to_owned)
        .unwrap_or_else(|| text(&old, "name"));
    require("name", &name)?;
    let filter = match filter {
        Some(v) => filter_json(&filter_payload(v)?)?,
        None => text(&old, "filter_json"),
    };
    let surface = text(&old, "surface");
    conn.execute(
        "UPDATE saved_views SET name=?,filter_json=?,updated_at=? WHERE id=?",
        rusqlite::params![name, filter, sources.now(conn)?, id],
    )
    .map_err(|e| {
        if integrity(&e) {
            AppError::new(
                "conflict",
                format!("Saved view name '{name}' already exists on surface '{surface}'"),
            )
            .with_details(dict([("surface", s(surface)), ("name", s(name))]))
        } else {
            sql_error(e)
        }
    })?;
    Ok(project(find(conn, pid, id)?))
}
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let daemon = command.starts_with("ui.");
    let action = command.rsplit('.').next().unwrap_or_default();
    let filter = if action == "create" {
        if daemon {
            let f = optional(args, "filter")
                .filter(|v| truthy(v))
                .cloned()
                .unwrap_or_else(|| dict::<&str>([]));
            if !matches!(f, Value::Dict(_)) {
                return Err(AppError::new(
                    "validation",
                    "ui.saved_views.create filter must be an object",
                ));
            }
            f
        } else {
            let raw = optional(args, "filter_json")
                .filter(|v| truthy(v))
                .map(py_str)
                .unwrap_or_else(|| "{}".into());
            kassiber_pytext::loads(&raw)
                .map_err(|e| AppError::new("validation", format!("Invalid --filter JSON: {e}")))?
        }
    } else {
        Value::Null
    };
    if daemon && action == "delete" && !flag(args, "view_id") {
        return Err(AppError::new(
            "validation",
            "ui.saved_views.delete requires view_id",
        ));
    }
    let (w, p) = references::scope(conn, args)?;
    let pid = text(&p, "id");
    match action {
        "list" => {
            let result = if flag(args, "surface") {
                rows(conn,"SELECT * FROM saved_views WHERE profile_id=? AND surface=? ORDER BY created_at DESC,id ASC",[pid,text(args,"surface")])?
            } else {
                rows(conn,"SELECT * FROM saved_views WHERE profile_id=? ORDER BY surface ASC,created_at DESC,id ASC",[pid])?
            };
            let views = Value::List(result.into_iter().map(project).collect());
            Ok(if daemon {
                dict([("views", views)])
            } else {
                views
            })
        }
        "create" => {
            let name = optional(args, "name")
                .filter(|v| !daemon || truthy(v))
                .map(py_str)
                .unwrap_or_default();
            let surface = optional(args, "surface")
                .filter(|v| !daemon || truthy(v))
                .map(py_str)
                .unwrap_or_default();
            require("surface", &surface)?;
            require("name", &name)?;
            let payload = filter_payload(&filter)?;
            let id = sources.uuid(conn)?;
            let time = sources.now(conn)?;
            let filter = filter_json(&payload)?;
            conn.execute("INSERT INTO saved_views(id,workspace_id,profile_id,surface,name,filter_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)",rusqlite::params![id,text(&w,"id"),pid,surface,name,filter,time,time]).map_err(|e| {
                if integrity(&e) {AppError::new("conflict",format!("Saved view '{name}' already exists on surface '{surface}' for this profile")).with_details(dict([("surface",s(surface)),("name",s(name))]))} else {sql_error(e)}
            })?;
            Ok(project(find(conn, &pid, &id)?))
        }
        "delete" => {
            let id = text(args, "view_id");
            find(conn, &pid, &id)?;
            conn.execute("DELETE FROM saved_views WHERE id=?", [&id])
                .map_err(sql_error)?;
            Ok(dict([("deleted", s(id))]))
        }
        _ => Err(AppError::new(
            "not_implemented",
            "saved-view operation is not implemented",
        )),
    }
}
