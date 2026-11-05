use kassiber_envelope::{dict, get, py_str, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_reports::source_funds_cases::{save_snapshot, FrozenCase};
use kassiber_store::rusqlite::{types::ValueRef, Connection};
use std::cell::RefCell;

pub fn value<'a>(row: &'a Value, key: &str) -> &'a Value {
    get(row, key).unwrap_or(&Value::Null)
}

pub fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    match value(row, key) {
        Value::Str(raw) => raw.as_str().unwrap(),
        _ => panic!("missing text {key}"),
    }
}

pub fn encode(value: &Value) -> String {
    dumps(value, &Options::new().sort_keys(true).compact()).unwrap()
}

fn snapshot(value: &Value) -> Value {
    match value {
        Value::Dict(items) if items.len() == 1 && get(value, "$decimal").is_some() => {
            Value::Decimal(text(value, "$decimal").parse().unwrap())
        }
        Value::Dict(items) => Value::Dict(
            items
                .iter()
                .map(|(key, item)| (key.clone(), snapshot(item)))
                .collect(),
        ),
        Value::List(items) => Value::List(items.iter().map(snapshot).collect()),
        other => other.clone(),
    }
}

pub struct Fixed {
    ids: Vec<String>,
    pub calls: RefCell<Vec<Value>>,
}

impl Fixed {
    pub fn new(case: &Value) -> Self {
        let ids = match value(case, "ids") {
            Value::List(items) => items.iter().map(py_str).collect(),
            _ => vec!["new-case".into(), "new-snapshot".into()],
        };
        Self {
            ids,
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl kassiber_setup::Sources for Fixed {
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        let mut calls = self.calls.borrow_mut();
        let id = self.ids[calls.len()].clone();
        calls.push(s("uuid"));
        Ok(id)
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        self.calls.borrow_mut().push(s("now"));
        Ok("2026-01-02T03:04:05Z".into())
    }
}

pub fn save(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value, AppError> {
    let optional = |key| match value(args, key) {
        Value::Null => None,
        Value::Str(raw) => Some(raw.as_str().unwrap()),
        _ => panic!("invalid optional text"),
    };
    let Value::Int(amount) = value(args, "target_amount_msat") else {
        panic!()
    };
    save_snapshot(
        conn,
        &FrozenCase {
            workspace_id: text(args, "workspace_id"),
            profile_id: text(args, "profile_id"),
            target_transaction_id: text(args, "target_transaction_id"),
            target_amount_msat: i64::try_from(amount.clone()).unwrap(),
            asset: text(args, "asset"),
            reveal_mode: text(args, "reveal_mode"),
            status: text(args, "status"),
            snapshot: &snapshot(value(args, "snapshot")),
            label: optional("label"),
            target_external_id: optional("target_external_id"),
            recipient_id: optional("recipient_id"),
            recipient_label_snapshot: optional("recipient_label_snapshot"),
            recipient_kind_snapshot: optional("recipient_kind_snapshot"),
            recipient_reveal_mode_snapshot: optional("recipient_reveal_mode_snapshot"),
        },
        sources,
    )
}

pub fn outcome(conn: &Connection, case: &Value) -> Value {
    let fixed = Fixed::new(case);
    let result = save(conn, value(case, "args"), &fixed);
    let calls = Value::List(fixed.calls.into_inner());
    match result {
        Ok(result) => {
            let envelope = kassiber_envelope::success("source-funds.case-snapshot", result.clone());
            dict([
                ("ok", result),
                ("calls", calls),
                ("cli", s(kassiber_envelope::cli_text(&envelope).unwrap())),
                (
                    "daemon",
                    s(kassiber_envelope::daemon_line(&envelope).unwrap()),
                ),
            ])
        }
        Err(error) => dict([
            (
                "error",
                dict([("code", s(error.code)), ("message", s(error.message))]),
            ),
            ("calls", calls),
        ]),
    }
}

#[allow(dead_code)]
pub fn dump(conn: &Connection) -> Value {
    let tables: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(tables.len(), 113);
    dict(tables.into_iter().map(|table| {
        let mut statement = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let names: Vec<_> = statement
            .column_names()
            .iter()
            .map(|name| name.to_string())
            .collect();
        let mut rows: Vec<Value> = statement
            .query_map([], |row| {
                Ok(dict(names.iter().enumerate().map(|(i, name)| {
                    (
                        name.clone(),
                        match row.get_ref(i).unwrap() {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => Value::Int(n.into()),
                            ValueRef::Real(n) => Value::Float(n),
                            ValueRef::Text(raw) => s(std::str::from_utf8(raw).unwrap()),
                            ValueRef::Blob(_) => panic!("unexpected blob"),
                        },
                    )
                })))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.sort_by_key(encode);
        (table, Value::List(rows))
    }))
}
