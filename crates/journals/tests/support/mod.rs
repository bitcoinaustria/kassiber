//! Test-only full logical dumps, including rowids. Never a product projection.
use kassiber_journals::{boundary, persistence, storage, Refusal};
use kassiber_store::rusqlite::{
    self, params_from_iter,
    types::{Value as Sql, ValueRef},
    Connection,
};
use serde_json::{json, Value};

/// A complete SQLite scalar comparison: distinguish INTEGER from REAL, and
/// compare every REAL bit. JSON exponent spellings are transport syntax.
pub fn logical_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) if !a.is_i64() && !b.is_i64() => {
            a.as_f64().map(f64::to_bits) == b.as_f64().map(f64::to_bits)
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| logical_equal(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, a)| b.get(k).is_some_and(|b| logical_equal(a, b)))
        }
        _ => a == b,
    }
}

fn scalar(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::String(s) => Sql::Text(s.clone()),
        Value::Number(n) if n.is_i64() => Sql::Integer(n.as_i64().unwrap()),
        Value::Number(n) => Sql::Real(n.as_f64().unwrap()),
        Value::Object(o) if o.len() == 1 && o.contains_key("$real_bits") => Sql::Real(
            f64::from_bits(u64::from_str_radix(o["$real_bits"].as_str().unwrap(), 16).unwrap()),
        ),
        Value::Object(o) if o.len() == 1 && o.contains_key("$bytes") => {
            let hex = o["$bytes"].as_str().unwrap();
            assert_eq!(hex.len() % 2, 0);
            Sql::Blob(
                hex.as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                    .collect(),
            )
        }
        _ => panic!("dump scalar {v}"),
    }
}
#[allow(dead_code)] // The capture probe supplies its observed schema instead.
pub fn load(c: &Connection, dump: &Value) {
    load_schema(c, dump, &Value::Null);
}
pub fn load_schema(c: &Connection, dump: &Value, schema: &Value) {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../store/fixtures/current-schema.json")).unwrap();
    let objects = schema
        .as_array()
        .unwrap_or_else(|| manifest["objects"].as_array().unwrap());
    c.execute_batch("PRAGMA foreign_keys=OFF;PRAGMA ignore_check_constraints=ON;BEGIN")
        .unwrap();
    for o in objects.iter().filter(|o| o[0] == "table") {
        c.execute_batch(o[3].as_str().unwrap()).unwrap();
    }
    insert_dump(c, dump);
    for kind in ["index", "view", "trigger"] {
        for o in objects.iter().filter(|o| o[0] == kind) {
            if let Some(sql) = o[3].as_str() {
                c.execute_batch(sql).unwrap();
            }
        }
    }
    c.execute_batch("COMMIT;PRAGMA foreign_keys=ON;PRAGMA ignore_check_constraints=OFF")
        .unwrap();
}

pub fn insert_dump(c: &Connection, dump: &Value) {
    for (table, data) in dump.as_object().unwrap() {
        let columns = data["columns"].as_array().unwrap();
        let query = format!(
            "INSERT INTO \"{table}\"({}) VALUES({})",
            columns
                .iter()
                .map(|s| format!("\"{}\"", s.as_str().unwrap()))
                .collect::<Vec<_>>()
                .join(","),
            vec!["?"; columns.len()].join(",")
        );
        let mut insert = c.prepare(&query).unwrap();
        for row in data["rows"].as_array().unwrap() {
            insert
                .execute(params_from_iter(row.as_array().unwrap().iter().map(scalar)))
                .unwrap();
        }
    }
}
pub fn snapshot(c: &Connection) -> Value {
    let tables: Vec<(String, String)> = c
        .prepare("SELECT name,sql FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let mut out = serde_json::Map::new();
    for (table, sql) in tables {
        let mut columns: Vec<String> = c
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        if !sql.to_uppercase().contains("WITHOUT ROWID") {
            columns.insert(0, "_rowid_".into());
        }
        let query = format!(
            "SELECT {} FROM \"{table}\"",
            columns
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut select = c.prepare(&query).unwrap();
        let mut rows:Vec<Value>=select.query_map([],|r|{
            let mut values=Vec::new();
            for i in 0..columns.len(){values.push(match r.get_ref(i)?{
                ValueRef::Null=>Value::Null,ValueRef::Integer(n)=>json!(n),ValueRef::Real(n) if !n.is_finite()=>json!({"$real_bits":format!("{:016x}", n.to_bits())}),ValueRef::Real(n)=>json!(n),ValueRef::Text(s)=>json!(std::str::from_utf8(s).unwrap()),ValueRef::Blob(s)=>json!({"$bytes":s.iter().map(|b|format!("{b:02x}")).collect::<String>()}),
            });}Ok(Value::Array(values))
        }).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        rows.sort_by_cached_key(|v| {
            kassiber_custody::codec::dumps(&kassiber_custody::codec::from_json(v).unwrap(), true)
                .unwrap()
        });
        out.insert(table, json!({"columns":columns,"rows":rows}));
    }
    Value::Object(out)
}

/// A live profile for the private process probe. Production uses setup's scope
/// resolver, never the injected profile stored in an oracle request.
#[allow(dead_code)]
pub fn profile(c: &Connection, id: &str) -> Result<Value, Refusal> {
    let mut statement = c.prepare("SELECT * FROM profiles WHERE id=?")?;
    let names = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut rows = statement.query([id])?;
    let row = rows
        .next()?
        .ok_or_else(|| Refusal::from("oracle profile missing".to_owned()))?;
    let mut values = serde_json::Map::new();
    for (index, name) in names.into_iter().enumerate() {
        let value = match row.get_ref(index)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => json!(n),
            ValueRef::Real(n) => json!(n),
            ValueRef::Text(text) => Value::String(std::str::from_utf8(text).unwrap().into()),
            ValueRef::Blob(bytes) => {
                kassiber_custody::codec::to_json(&kassiber_pyjson::Value::Bytes(bytes.to_vec()))
            }
        };
        values.insert(name, value);
    }
    Ok(Value::Object(values))
}

#[derive(Default)]
pub struct Writes {
    pub count: usize,
    pub fail_at: Option<usize>,
    pub panic_at: Option<usize>,
    pub tables: Vec<&'static str>,
}
impl storage::WriteObserver for Writes {
    fn wrote(&mut self, table: &'static str) -> rusqlite::Result<()> {
        self.count += 1;
        self.tables.push(table);
        assert_ne!(self.panic_at, Some(self.count), "injected write panic");
        if self.fail_at == Some(self.count) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(())
    }
}
#[allow(dead_code)] // The shared probe renders outside its persistence timer.
pub fn store(
    c: &Connection,
    v: &Value,
    writes: &mut impl storage::WriteObserver,
) -> Result<persistence::Stored, Refusal> {
    store_with(c, v, writes, |_| {})
}
#[allow(dead_code)]
pub fn store_with(
    c: &Connection,
    v: &Value,
    writes: &mut impl storage::WriteObserver,
    edit: impl FnOnce(&mut kassiber_journals::JournalState<'_>),
) -> Result<persistence::Stored, Refusal> {
    let r = &v["record"];
    let response = if r["response"].is_null() {
        boundary::empty_response()
    } else {
        boundary::ledger_response(&r["response"])?
    };
    let mut state = boundary::build(
        &r["profile"],
        &r["custody"],
        r["wallets"].as_array().unwrap(),
        &response,
        String::new,
    )?;
    edit(&mut state);
    let mut ids = v["ids"].as_array().unwrap().iter();
    let mut identity = || {
        ids.next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Refusal::from("persistence UUID tape exhausted".to_owned()))
    };
    let result = persistence::store_ledger_state(
        c,
        &r["profile"],
        &state,
        v["created_at"].as_str().unwrap(),
        &mut identity,
        writes,
    )?;
    assert!(ids.next().is_none(), "unused persistence UUIDs");
    Ok(result)
}
