use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{Connection, OpenFlags};
use kassiber_wallets::analysis_index::durable::{fixture, synchronize_with_sources};
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::rc::Rc;

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    kassiber_envelope::get(v, key).unwrap_or(&Value::Null)
}
fn text(v: &Value) -> Option<&str> {
    if let Value::Str(v) = v {
        v.as_str()
    } else {
        None
    }
}

pub fn run(request: &Value) -> Value {
    let path = text(field(request, "database")).unwrap();
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    if field(request, "encrypted") == &Value::Bool(true) {
        conn.pragma_update(None, "key", "mirror-kill-fixture")
            .unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000")
        .unwrap();
    if let Some(expected) = text(field(request, "journal")) {
        assert_eq!(
            conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    if field(request, "capture") == &Value::Bool(true) {
        return fixture::capture(&conn).unwrap();
    }
    let pause = if let Value::Int(v) = field(request, "pause") {
        v.to_string().parse::<usize>().unwrap()
    } else {
        0
    };
    let count = Rc::new(Cell::new(0));
    let phases = Rc::new(RefCell::new(Vec::<Value>::new()));
    let observed = count.clone();
    let recorded = phases.clone();
    fixture::set_write_observer(Some(Box::new(move |sql| {
        let n = observed.get() + 1;
        observed.set(n);
        let phase = if sql == "before_commit" || sql == "after_commit" {
            sql
        } else if sql.starts_with("CREATE")
            || sql.starts_with("INSERT OR IGNORE INTO chain_index_clock")
        {
            "installation"
        } else if sql.contains("chain_index_state") {
            "generation"
        } else if sql.contains("chain_index_clock") || sql.contains("chain_index_dirty") {
            "source_triggers"
        } else if sql.contains("chain_index_source") {
            "contributions"
        } else if sql.contains("chain_index_label") {
            "labels"
        } else {
            "materialization"
        };
        recorded.borrow_mut().push(phase.into());
        if pause == n {
            println!("{{\"paused\":{n}}}");
            std::io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        }
    })));
    let profile = text(field(request, "profile")).unwrap_or("p");
    let mut sources = fixture::FixedSources(
        text(field(request, "as_of"))
            .unwrap_or("2026-01-01T00:00:00Z")
            .to_owned(),
    );
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let state = synchronize_with_sources(
        &conn,
        profile,
        field(request, "rebuild") == &Value::Bool(true),
        &mut sources,
    )
    .unwrap();
    fixture::checkpoint("before_commit");
    conn.execute_batch("COMMIT").unwrap();
    fixture::checkpoint("after_commit");
    fixture::set_write_observer(None);
    let recorded_phases = phases.borrow().clone();
    kassiber_envelope::dict([
        ("state", state),
        ("capture", fixture::capture(&conn).unwrap()),
        ("writes", Value::Int(count.get().into())),
        ("phases", Value::List(recorded_phases)),
    ])
}
