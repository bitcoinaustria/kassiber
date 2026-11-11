//! Retired ownership policies are private history, never current authority.
use super::{backfills, data, python_json as py, repairs::statement};
use crate::{map_sqlite_error, Error, ErrorKind, Result};
use kassiber_pyjson::{Key, Value};
use rusqlite::{params, Connection};
use std::collections::BTreeSet;

pub(super) fn unsupported() -> Error {
    Error::new(
        ErrorKind::UnsupportedSchemaLayout,
        "stored migration input holds a value this build cannot convert exactly",
    )
}
pub(super) fn json(raw: Option<&str>) -> Result<Value> {
    match py::loads(raw.filter(|v| !v.is_empty()).unwrap_or("{}")) {
        Ok(value) => Ok(value),
        Err(py::ParseError::Invalid) => Ok(Value::Null),
        Err(_) => Err(unsupported()),
    }
}
// These fields are Python str(value or ""). Collections need Python repr,
// which the migration must refuse rather than replacing with JSON text.
pub(super) fn text(value: Option<&Value>) -> Result<String> {
    Ok(match value.filter(|v| backfills::truthy(v)) {
        None => String::new(),
        Some(Value::Str(v)) => v.as_str().ok_or_else(unsupported)?.into(),
        Some(Value::Bool(true)) => "True".into(),
        Some(Value::Int(v)) => v.to_string(),
        Some(Value::Float(v)) => kassiber_pyjson::float_repr(*v),
        _ => return Err(unsupported()),
    })
}
pub(super) fn normalize(
    kind: &str,
    value: Option<&Value>,
    default: &str,
) -> Result<Option<String>> {
    let value = text(value)?;
    let value = value.trim_matches(backfills::whitespace).to_lowercase();
    let value = if value.is_empty() { default } else { &value };
    Ok(data()["normalization"][kind][value]
        .as_str()
        .map(str::to_owned))
}
#[cfg(any(test, feature = "test-hooks"))]
pub(super) fn uuid_hex(conn: &Connection) -> Result<String> {
    let mut bytes: Vec<u8> = conn
        .query_row("SELECT randomblob(16)", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn instance_id(
    conn: &mut Connection,
    uuid: &mut impl FnMut(&Connection) -> Result<String>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    use rusqlite::OptionalExtension;
    let existing = conn
        .query_row(
            "SELECT value FROM settings WHERE key='database_instance_id'",
            [],
            |r| r.get::<_, rusqlite::types::Value>(0),
        )
        .optional()
        .map_err(map_sqlite_error)?;
    if matches!(existing,Some(rusqlite::types::Value::Text(ref value)) if crate::valid_hex(value,32))
    {
        return Ok(());
    }
    let identity = uuid(conn)?;
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    tx.execute("INSERT INTO settings(key,value) VALUES('database_instance_id',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[identity]).map_err(map_sqlite_error)?;
    observer("database-identity.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("database-identity.after_commit")
}
pub(crate) fn inline_history(
    conn: &mut Connection,
    uuid: &mut impl FnMut(&Connection) -> Result<String>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let mut stmt = tx.prepare("SELECT id, workspace_id, profile_id, created_at, config_json FROM wallets WHERE config_json LIKE '%\"ownership_history\"%'").map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(stmt);
    for (id, workspace, profile, created, raw) in rows {
        let Value::Dict(mut config) = json(raw.as_deref())? else {
            continue;
        };
        let Some(index) = config
            .iter()
            .position(|(k, _)| k == &Key::from("ownership_history"))
        else {
            continue;
        };
        let Value::List(history) = config.remove(index).1 else {
            continue;
        };
        let mut stmt = tx.prepare("SELECT private_material_json FROM wallet_policy_epochs WHERE wallet_id=? AND status='retired'").map_err(map_sqlite_error)?;
        let mut existing = stmt
            .query_map([&id], |r| r.get::<_, String>(0))
            .map_err(map_sqlite_error)?
            .collect::<rusqlite::Result<BTreeSet<_>>>()
            .map_err(map_sqlite_error)?;
        drop(stmt);
        let config_value = Value::Dict(config.clone());
        for item in history {
            if !matches!(item, Value::Dict(_)) {
                continue;
            }
            let mut material = Vec::new();
            for field in data()["ownership_private_fields"]
                .as_array()
                .expect("private policy fields")
            {
                let field = field.as_str().expect("private policy field");
                if let Some(value) = py::get(&item, field) {
                    if !matches!(value, Value::Null)
                        && value != &Value::from("")
                        && value != &Value::List(Vec::new())
                    {
                        material.push((Key::from(field), value.clone()));
                    }
                }
            }
            if let Some(scan) = py::get(&item, "scan_to_index")
                .filter(|v| !matches!(v, Value::Null) && *v != &Value::from(""))
            {
                let current = material
                    .iter()
                    .find(|(k, _)| k == &Key::from("ownership_scan_to_index"))
                    .map(|(_, v)| v)
                    .filter(|v| backfills::truthy(v))
                    .cloned()
                    .unwrap_or_else(|| Value::from(0));
                // int(float('nan')) raises ValueError, caught by the reference.
                let integer = |v: &Value| {
                    if matches!(v,Value::Float(x) if x.is_nan()) {
                        Ok(None)
                    } else {
                        backfills::integer(v)
                    }
                };
                if let (Some(current), Some(scan)) = (integer(&current)?, integer(scan)?) {
                    material.retain(|(k, _)| k != &Key::from("ownership_scan_to_index"));
                    material.push((
                        Key::from("ownership_scan_to_index"),
                        Value::Int(current.max(scan)),
                    ));
                }
            }
            if material.is_empty() {
                continue;
            }
            let material = Value::Dict(material);
            let encoded = py::dumps(&material, true, false)?;
            if existing.contains(&encoded) {
                continue;
            }
            let choose = |field| {
                py::get(&material, field)
                    .filter(|v| backfills::truthy(v))
                    .or_else(|| py::get(&config_value, field))
            };
            let scope = match normalize("chain", choose("chain"), "bitcoin")? {
                Some(chain) => normalize(
                    if chain == "bitcoin" {
                        "bitcoin_network"
                    } else {
                        "liquid_network"
                    },
                    choose("network"),
                    if chain == "bitcoin" {
                        "main"
                    } else {
                        "liquidv1"
                    },
                )?
                .map(|network| (chain, network)),
                None => None,
            };
            let (chain, network) = scope.unwrap_or_else(|| ("bitcoin".into(), "main".into()));
            let hex = uuid(&tx)?;
            let epoch = format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            );
            tx.execute(
                statement(
                    "_migrate_inline_ownership_history",
                    "INSERT INTO wallet_policy_epochs",
                ),
                params![epoch, workspace, profile, id, chain, network, encoded, created, created],
            )
            .map_err(map_sqlite_error)?;
            observer("ownership-history.after_epoch")?;
            existing.insert(encoded);
        }
        tx.execute(
            "UPDATE wallets SET config_json=? WHERE id=?",
            params![py::dumps(&config_value, true, false)?, id],
        )
        .map_err(map_sqlite_error)?;
        observer("ownership-history.after_config")?;
    }
    observer("ownership-history.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("ownership-history.after_commit")
}
