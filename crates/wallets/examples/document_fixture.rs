//! Disposable-book differential driver. No CLI/daemon/AI registration.
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{functions::FunctionFlags, Connection};
use kassiber_wallets::file_import::{document, Identity};
use std::{
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
};
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}
fn text(value: &Value) -> String {
    kassiber_envelope::py_str(value)
}
fn error(e: AppError) -> Value {
    dict([
        ("code", s(e.code)),
        (
            "message",
            e.message_codepoints
                .map_or_else(|| s(e.message), |v| Value::Str(*v)),
        ),
        ("hint", e.hint.map_or(Value::Null, |h| s(h.as_ref()))),
        ("details", e.details.unwrap_or(Value::Null)),
        ("retryable", Value::Bool(e.retryable)),
    ])
}
fn main() {
    let mut input = String::new();
    std::io::stdin()
        .take(32 * 1024 * 1024)
        .read_to_string(&mut input)
        .unwrap();
    let request = kassiber_pytext::loads(&input).unwrap();
    let conn = Connection::open(text(field(&request, "database"))).unwrap();
    if field(&request, "encrypted") == &Value::Bool(true) {
        conn.execute_batch("PRAGMA key='disposable-test-key'")
            .unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let time = text(field(&request, "time"));
    let sql_time = time.clone();
    conn.create_scalar_function("strftime", 2, FunctionFlags::SQLITE_UTF8, move |_| {
        Ok(sql_time.clone())
    })
    .unwrap();
    if field(&request, "action") == &s("preview") {
        let optional = |key| match field(&request, key) {
            Value::Str(v) => v.as_str().map(str::to_owned),
            _ => None,
        };
        let preview = document::DocumentPreview {
            source_file: text(field(&request, "source")),
            provider_name: optional("provider"),
            model: optional("model"),
            confidence_threshold: field(&request, "threshold").clone(),
            max_pages: field(&request, "max_pages").clone(),
            pages: field(&request, "pages").clone(),
            expected_fiat_currency: Some("EUR".into()),
        };
        let result = document::DocumentConsent::explicit(&conn, &preview).and_then(|consent| {
            document::Providers::system().preview(&conn, &preview, Some(consent))
        });
        let output = match result {
            Ok(v) => dict([("result", v)]),
            Err(e) => dict([("error", error(e))]),
        };
        println!(
            "{}",
            kassiber_pyjson::dumps(&output, &Default::default()).unwrap()
        );
        return;
    }
    let Value::List(ids) = field(&request, "attachment_ids") else {
        panic!("attachment ids")
    };
    let ids = Arc::new(Mutex::new(ids.clone().into_iter()));
    conn.create_scalar_function("randomblob", 1, FunctionFlags::SQLITE_UTF8, move |_| {
        let id = text(
            &ids.lock()
                .unwrap()
                .next()
                .expect("attachment entropy exhausted"),
        );
        let hex = id.replace('-', "");
        Ok((0..32)
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>())
    })
    .unwrap();
    let Value::List(ids) = field(&request, "ids") else {
        panic!("import ids")
    };
    let identity = Identity {
        ids: ids.iter().map(text).collect(),
        timestamp: time,
    };
    let selected = match field(&request, "selected") {
        Value::List(v) => Some(v.iter().map(text).collect::<Vec<_>>()),
        _ => None,
    };
    let expected = match field(&request, "expected_hash") {
        Value::Str(v) => v.as_str(),
        _ => None,
    };
    let rows = match field(&request, "rows") {
        Value::List(v) => v.as_slice(),
        _ => panic!("rows"),
    };
    if field(&request, "outer_rollback") == &Value::Bool(true) {
        conn.execute_batch("BEGIN").unwrap();
    }
    let source = text(field(&request, "source"));
    let data_root = text(field(&request, "data_root"));
    let result = document::import_draft(
        &conn,
        document::ImportDraft {
            source_file: &source,
            data_root: Path::new(&data_root),
            profile_id: "p",
            wallet: "Source",
            rows,
            include_quarantined: field(&request, "include_quarantined") == &Value::Bool(true),
            selected_row_ids: selected.as_deref(),
            expected_source_sha256: expected,
            confidence_threshold: field(&request, "threshold"),
            attach_evidence: field(&request, "attach") != &Value::Bool(false),
            check_evidence_budget: false,
        },
        Some(&identity),
    );
    if field(&request, "outer_rollback") == &Value::Bool(true) {
        conn.execute_batch("ROLLBACK").unwrap();
    }
    let output = match result {
        Ok(v) => dict([("result", v)]),
        Err(e) => dict([("error", error(e))]),
    };
    println!(
        "{}",
        kassiber_pyjson::dumps(&output, &Default::default()).unwrap()
    );
}
