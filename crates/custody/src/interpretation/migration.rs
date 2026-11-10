//! Legacy migration issues quarantine rows; they never grant allocation authority.
use crate::{
    codec,
    graph::stored,
    snapshot::{Record, Snapshot},
};
use kassiber_pyjson::{Key, Options, Value as Py};

fn field(row: &Record, key: &str) -> Result<Py, String> {
    row.get(key)
        .ok_or_else(|| "KeyError".to_owned())
        .and_then(codec::from_json)
}
fn decode(row: &Record, key: &str, default: &str) -> Result<Option<Py>, String> {
    let value = field(row, key)?;
    let value = if stored::truth(&value) {
        value
    } else {
        Py::Str(default.into())
    };
    // json.loads catches TypeError/ValueError, including non-string arguments.
    Ok(if let Py::Str(value) = value {
        value.as_str().and_then(|s| codec::loads(s).ok())
    } else {
        None
    })
}
fn items(value: Py) -> Result<Vec<Py>, String> {
    match value {
        Py::List(values) => Ok(values),
        Py::Dict(values) => Ok(values
            .into_iter()
            .map(|(key, _)| {
                let Key::Str(s) = key else {
                    unreachable!("JSON key")
                };
                Py::Str(s)
            })
            .collect()),
        Py::Str(value) => Ok(value
            .code_points()
            .map(|p| {
                Py::Str(kassiber_pyjson::PyStr::from_code_points([p]).expect("decoded code point"))
            })
            .collect()),
        _ => Err("TypeError".into()),
    }
}
fn string(value: Py) -> Py {
    match value {
        Py::Str(_) => value,
        _ => Py::Str(super::text::string(&value).into()),
    }
}
pub fn quarantines(snapshot: &Snapshot) -> Result<Vec<Py>, String> {
    let mut quarantines = Vec::new();
    for row in &snapshot.migration_issues {
        let ids = decode(row, "transaction_ids_json", "[]")?.unwrap_or(Py::List(Vec::new()));
        let mut detail = Py::Dict(vec![]);
        for (key, source) in [
            ("migration_issue_id", "id"),
            ("legacy_table", "legacy_table"),
            ("legacy_source_id", "legacy_source_id"),
            ("issue_code", "issue_code"),
        ] {
            stored::put(&mut detail, key, field(row, source)?);
        }
        if let value @ Py::Dict(_) =
            decode(row, "details_json", "{}")?.unwrap_or(Py::Dict(Vec::new()))
        {
            stored::put(&mut detail, "migration_details", value);
        }
        for id in items(ids)? {
            let id = string(id);
            let workspace = field(row, "workspace_id")?;
            let profile = field(row, "profile_id")?;
            let detail_json = kassiber_pyjson::dumps(&detail, &Options::new().sort_keys(true))
                .map_err(|_| "ValueError".to_owned())?;
            quarantines.push(Py::Dict(vec![
                (Key::Str("transaction_id".into()), id),
                (Key::Str("workspace_id".into()), workspace),
                (Key::Str("profile_id".into()), profile),
                (
                    Key::Str("reason".into()),
                    Py::Str("custody_authored_migration_incomplete".into()),
                ),
                (Key::Str("detail_json".into()), Py::Str(detail_json.into())),
            ]));
        }
    }
    // Held ID-less import duplicates, then kept reset decisions that could
    // not bind, follow, already shaped as quarantines.
    for row in snapshot
        .import_duplicate_quarantines
        .iter()
        .flatten()
        .chain(snapshot.reset_hold_quarantines.iter().flatten())
    {
        let mut quarantine = Vec::new();
        for key in [
            "transaction_id",
            "workspace_id",
            "profile_id",
            "reason",
            "detail_json",
        ] {
            quarantine.push((Key::Str(key.into()), field(row, key)?));
        }
        quarantines.push(Py::Dict(quarantine));
    }
    Ok(quarantines)
}
