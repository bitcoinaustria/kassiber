//! A recorded session's starting point, driven by hand: the same book,
//! environment and tape, with the test's own requests and assertions.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::golden::{calls, string, Golden, Scratch};
use crate::session::{field, prepare, Daemon, Prepared, ELECTRUM_FETCH, ESPLORA_FETCH};
use kassiber_envelope::get;
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

pub const ARGS: &str = r#"{"rates": false, "journals": false}"#;

pub struct Live {
    pub scratch: Scratch,
    pub case: Value,
    pub prepared: Prepared,
}

/// The fetch request a loopback chain held, for the case's protocol.
pub fn fetch(param: &str) -> &'static str {
    if param.contains("electrum") {
        ELECTRUM_FETCH
    } else {
        ESPLORA_FETCH
    }
}

impl Live {
    /// Session `index` of the recorded case `test[param]`, prepared.
    pub fn new(module: &str, test: &str, param: &str, index: usize) -> Self {
        let golden = Golden::load(module);
        let case = golden
            .cases(test)
            .into_iter()
            .find(|case| string(case, "param") == param)
            .unwrap_or_else(|| panic!("no recording of {test}[{param}]"))
            .clone();
        let scratch = Scratch::new(&format!("{test}-{param}"));
        let session = calls(&case)[index].clone();
        let prepared = prepare(&golden, &scratch, index, &session);
        Self {
            scratch,
            case,
            prepared,
        }
    }

    /// Start the daemon; with `hold`, the first request containing it is
    /// held until `release`.
    pub fn start(&self, hold: Option<&str>) -> Daemon {
        if let Some(pattern) = hold {
            std::fs::write(self.prepared.hold.join("arm"), pattern).unwrap();
        }
        Daemon::start(
            &self.prepared.root,
            &self.prepared.env,
            &self.prepared.tape,
            Some(&self.prepared.hold),
        )
    }

    /// Arm a hold on a daemon already running (the next matching request).
    pub fn arm(&self, pattern: &str) {
        std::fs::write(self.prepared.hold.join("arm"), pattern).unwrap();
    }

    pub fn wait_held(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !self.held() {
            assert!(Instant::now() < deadline, "the held request never came");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn held(&self) -> bool {
        self.prepared.hold.join("held").exists()
    }

    pub fn release(&self) {
        std::fs::write(self.prepared.hold.join("release"), b"").unwrap();
    }

    pub fn book(&self) -> Connection {
        open(&self.prepared.book, None)
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.book().query_row(sql, [], |row| row.get(0)).unwrap()
    }

    pub fn execute(&self, sql: &str) {
        self.book().execute_batch(sql).unwrap();
    }
}

pub fn open(book: &Path, key: Option<&str>) -> Connection {
    let conn = Connection::open(book).unwrap();
    if let Some(key) = key {
        conn.pragma_update(None, "key", key).unwrap();
    }
    conn
}

/// The one completed job of a `test.freshness.execute` answer.
pub fn completed(response: &Value) -> Value {
    assert_eq!(
        field(response, "kind"),
        "test.freshness.execute",
        "{response:?}"
    );
    let Some(Value::List(jobs)) = get(response, "data").and_then(|d| get(d, "completed")) else {
        panic!("no completed jobs: {response:?}")
    };
    assert_eq!(jobs.len(), 1, "{response:?}");
    jobs[0].clone()
}

/// `path` in `value`, by keys.
pub fn at<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
    path.iter()
        .fold(value, |v, key| get(v, key).unwrap_or(&Value::Null))
}

pub fn str_at(value: &Value, path: &[&str]) -> String {
    kassiber_envelope::py_str(at(value, path))
}

/// Nothing a sync publishes reached the book.
pub fn unpublished(live: &Live) {
    for table in [
        "transactions",
        "chain_observer_instances",
        "wallet_utxo_refreshes",
    ] {
        assert_eq!(
            live.count(&format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
}

pub fn graph_pending(daemon: &mut Daemon, request_id: &str) -> i64 {
    daemon.send("test.freshness.graph.pending", request_id, None);
    let response = daemon.until(request_id, None);
    assert_eq!(
        field(&response, "kind"),
        "test.freshness.graph.pending",
        "{response:?}"
    );
    str_at(&response, &["data", "grants"]).parse().unwrap()
}

/// Read rows while more arrive within 100 ms; none may answer `request_id`.
pub fn drain_without(daemon: &mut Daemon, request_id: &str) {
    while let Some(row) = daemon.pending(Duration::from_millis(100)) {
        assert_ne!(field(&row, "request_id"), request_id, "{row:?}");
    }
}

pub fn same(value: &Value, json: &str) -> bool {
    crate::session::text(value) == crate::session::text(&kassiber_pytext::loads(json).unwrap())
}

/// The canonical snapshot of an encrypted book.
pub fn snapshot_keyed(book: &Path, key: &str) -> Value {
    crate::golden::snapshot_of(&open(book, Some(key)))
}
