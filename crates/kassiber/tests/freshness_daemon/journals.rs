//! tests/test_rust_freshness_journals.py: cancelling the development
//! journal replay of an encrypted book.

use std::time::{Duration, Instant};

use crate::live::{open, str_at, Live};
use crate::session::field;

pub fn test_encrypted_journal_job_and_hybrid_key_handoff() {
    // The native refresh fails after its write (an armed test fault), so
    // the development replay starts. It ran the Python reference; a stand-in
    // that never answers keeps it running until the cancel, which must stop
    // the stand-in (kill it) and leave the book's domain tables as they were.
    let key = "synthetic-registration-only";
    let live = Live::new(
        "journals",
        "test_encrypted_journal_job_and_hybrid_key_handoff",
        "cancel",
        0,
    );
    let stand_in = live.scratch.0.join("journal-reference.sh");
    std::fs::write(&stand_in, "exec sleep 600\n").unwrap();
    let mut env = live.prepared.env.clone();
    env.retain(|(k, _)| {
        !k.starts_with("KASSIBER_RUST_TEST_JOURNAL_PYTHON")
            && k != "KASSIBER_RUST_TEST_JOURNAL_REFERENCE"
    });
    env.push(("KASSIBER_RUST_TEST_JOURNAL_PYTHON".into(), "/bin/sh".into()));
    env.push((
        "KASSIBER_RUST_TEST_JOURNAL_REFERENCE".into(),
        stand_in.display().to_string(),
    ));
    let log = live.scratch.0.join("journal-census.jsonl");
    env.retain(|(k, _)| k != "KASSIBER_HYBRID_LOG");
    env.push(("KASSIBER_HYBRID_LOG".into(), log.display().to_string()));
    let domain = |snapshot: kassiber_pyjson::Value| {
        let kassiber_pyjson::Value::Dict(tables) = snapshot else {
            unreachable!()
        };
        kassiber_pyjson::Value::Dict(
            tables
                .into_iter()
                .filter(|(name, _)| {
                    let name = format!("{name:?}");
                    !name.contains("freshness_jobs") && !name.contains("freshness_source_states")
                })
                .collect(),
        )
    };
    let before = domain(crate::live::snapshot_keyed(&live.prepared.book, key));
    let mut daemon =
        crate::session::Daemon::start(&live.prepared.root, &env, &live.prepared.tape, None);
    daemon.send(
        "daemon.unlock",
        "unlock",
        Some(&format!(
            r#"{{"auth_response": {{"passphrase_secret": "{key}"}}}}"#
        )),
    );
    assert_eq!(
        field(&daemon.until("unlock", None), "kind"),
        "daemon.unlock"
    );
    daemon.send("ui.freshness.run", "refresh", Some(r#"{"rates": false}"#));
    let deadline = Instant::now() + Duration::from_secs(120);
    while !std::fs::read_to_string(&log)
        .unwrap_or_default()
        .contains("\"served_by\":\"python\"")
    {
        assert!(Instant::now() < deadline, "the replay never started");
        std::thread::sleep(Duration::from_millis(10));
    }
    daemon.send(
        "test.freshness.request.cancel",
        "cancel",
        Some(r#"{"target_request_id": "refresh"}"#),
    );
    assert_eq!(
        str_at(&daemon.until("cancel", None), &["data", "cancelled"]),
        "True"
    );
    let result = daemon.until("refresh", Some("ui.freshness.run"));
    assert_eq!(
        str_at(&result, &["data", "journals", "status"]),
        "cancelled",
        "{result:?}"
    );
    daemon.send("daemon.lock", "lock", None);
    assert_eq!(field(&daemon.until("lock", None), "kind"), "daemon.lock");
    let rows = daemon.finish();
    assert!(!rows
        .iter()
        .map(crate::session::text)
        .any(|t| t.contains(key)));
    let conn = open(&live.prepared.book, Some(key));
    let processed: Option<String> = conn
        .query_row("SELECT last_processed_at FROM profiles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(processed.as_deref(), Some("2020-01-01T00:00:00Z"));
    let jobs: Vec<(String, String)> = conn
        .prepare("SELECT job_type,status FROM freshness_jobs")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        jobs,
        vec![("journal_refresh".to_owned(), "cancelled".to_owned())]
    );
    drop(conn);
    let after = domain(crate::live::snapshot_keyed(&live.prepared.book, key));
    crate::golden::assert_same_book("cancel", &after, &before);
    let records: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(records.len(), 2, "{records:?}");
    assert!(
        records[0].contains("\"refusal_shape\":\"storing:DatabaseError\"")
            && records[0].contains("\"replay_safe\":true")
    );
    assert!(records[1].contains("\"served_by\":\"python\""));
}
