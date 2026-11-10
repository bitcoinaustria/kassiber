//! tests/test_rust_freshness_lifecycle.py: a live freshness request while
//! the chain holds its first history fetch, and the controls around it.

use std::time::Duration;

use crate::live::{
    completed, drain_without, fetch, graph_pending, same, str_at, unpublished, Live, ARGS,
};
use crate::session::{field, Daemon};

const MODULE: &str = "lifecycle";

/// `running()`: a sync started and held at its first history fetch.
fn running(test: &str, param: &str) -> (Live, Daemon) {
    let live = Live::new(MODULE, test, param, 0);
    let mut daemon = live.start(Some(fetch(param)));
    daemon.send("test.freshness.execute", "sync", Some(ARGS));
    live.wait_held();
    (live, daemon)
}

fn controls_during_fetch(param: &str, control: &str) {
    let (live, mut daemon) = running(
        "test_live_freshness_controls_remain_available_during_fetch",
        param,
    );
    daemon.send("status", "read", None);
    assert_eq!(field(&daemon.until("read", None), "kind"), "status");
    daemon.send("test.freshness.execute", "competing", Some(ARGS));
    assert_eq!(
        str_at(&daemon.until("competing", None), &["error", "code"]),
        "project_operation_in_progress"
    );
    let (job_id, source_key): (String, String) = live
        .book()
        .query_row("SELECT id,source_key FROM freshness_jobs", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    match control {
        "cancel" | "revoke" => daemon.send(
            &format!("test.freshness.request.{control}"),
            "control",
            Some(r#"{"target_request_id": "sync"}"#),
        ),
        "offline" => daemon.send(
            "ui.network.offline.set",
            "control",
            Some(r#"{"enabled": true}"#),
        ),
        "source-cancel" => daemon.send(
            "test.freshness.cancel",
            "control",
            Some(&format!(r#"{{"job_id": "{job_id}"}}"#)),
        ),
        _ => daemon.send(
            "test.freshness.pause",
            "control",
            Some(&format!(r#"{{"source_key": "{source_key}"}}"#)),
        ),
    }
    let response = daemon.until("control", None);
    assert_ne!(field(&response, "kind"), "error", "{response:?}");
    if matches!(control, "cancel" | "revoke") {
        assert!(
            same(
                crate::live::at(&response, &["data"]),
                r#"{"cancelled": true, "queued": false}"#
            ),
            "{response:?}"
        );
    }
    live.release();
    let response = daemon.until("sync", Some("test.freshness.execute"));
    if control == "pause" {
        assert!(
            same(crate::live::at(&response, &["data", "completed"]), "[]"),
            "{response:?}"
        );
        daemon.send(
            "test.freshness.resume",
            "resume",
            Some(&format!(r#"{{"source_key": "{source_key}"}}"#)),
        );
        assert!(same(
            crate::live::at(&daemon.until("resume", None), &["data", "result", "paused"]),
            "false"
        ));
    } else {
        let job = completed(&response);
        assert_eq!(
            str_at(&job, &["status"]),
            if matches!(control, "cancel" | "source-cancel") {
                "cancelled"
            } else {
                "error"
            },
            "{job:?}"
        );
        if matches!(control, "revoke" | "offline") {
            assert_eq!(
                str_at(&job, &["error", "code"]),
                if control == "revoke" {
                    "interaction_required"
                } else {
                    "network_egress_disabled"
                }
            );
        }
    }
    unpublished(&live);
    assert_eq!(graph_pending(&mut daemon, "fenced-graph"), 0);
    daemon.finish();
    crate::live::no_extra_requests(&live);
}

pub fn test_live_freshness_controls_remain_available_during_fetch() {
    for control in ["cancel", "revoke", "offline", "source-cancel", "pause"] {
        controls_during_fetch(&format!("{control}-esplora"), control);
    }
}

pub fn test_live_freshness_closed_output_stops_and_joins_the_request() {
    let (live, daemon) = running(
        "test_live_freshness_closed_output_stops_and_joins_the_request",
        "esplora",
    );
    daemon.close_output();
    live.release();
    let Daemon { mut child, .. } = daemon;
    let status = wait_timeout(&mut child);
    assert_eq!(status.code(), Some(1));
    unpublished(&live);
    crate::live::no_extra_requests(&live);
}

fn wait_timeout(child: &mut std::process::Child) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon did not exit"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn test_live_freshness_eof_joins_the_request_before_exit() {
    let (live, mut daemon) = running(
        "test_live_freshness_eof_joins_the_request_before_exit",
        "esplora",
    );
    daemon.close_stdin();
    while let Some(row) = daemon.pending(Duration::from_millis(100)) {
        assert_eq!(
            field(&row, "kind"),
            "test.freshness.execute.progress",
            "{row:?}"
        );
    }
    live.release();
    let response = daemon.until("sync", Some("test.freshness.execute"));
    assert_eq!(str_at(&completed(&response), &["status"]), "cancelled");
    let (status, stderr, _) = daemon.wait();
    assert!(status.success(), "{status}: {stderr}");
    unpublished(&live);
    crate::live::no_extra_requests(&live);
}

pub fn test_live_freshness_private_graph_handoff_and_discard() {
    for control in ["cancel", "revoke"] {
        let param = format!("{control}-esplora");
        let (live, mut daemon) = running(
            "test_live_freshness_private_graph_handoff_and_discard",
            &param,
        );
        assert_eq!(graph_pending(&mut daemon, "before"), 0);
        live.release();
        let result = daemon.until("sync", Some("test.freshness.execute"));
        assert_eq!(str_at(&completed(&result), &["status"]), "done");
        assert_eq!(graph_pending(&mut daemon, "after"), 1);
        let before = crate::golden::snapshot(&live.prepared.book);
        daemon.send("status", "idle", None);
        assert_eq!(field(&daemon.until("idle", None), "kind"), "status");
        assert_eq!(graph_pending(&mut daemon, "idle-graph"), 1);
        crate::golden::assert_same_book(
            &param,
            &crate::golden::snapshot(&live.prepared.book),
            &before,
        );
        let rows: String = daemon.rows.iter().map(crate::session::text).collect();
        assert!(!rows.contains("graph_grants"));
        assert_eq!(
            live.count(
                "SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'"
            ),
            0
        );
        daemon.send(
            &format!("test.freshness.request.{control}"),
            "discard",
            Some(r#"{"target_request_id": "sync"}"#),
        );
        assert!(same(
            crate::live::at(&daemon.until("discard", None), &["data"]),
            r#"{"cancelled": true, "queued": false}"#
        ));
        assert_eq!(graph_pending(&mut daemon, "discarded"), 0);
        daemon.finish();
        crate::live::no_extra_requests(&live);
    }
}

pub fn test_live_freshness_graph_handoff_deduplicates_and_never_recovers_authority() {
    let test = "test_live_freshness_graph_handoff_deduplicates_and_never_recovers_authority";
    let live = Live::new(MODULE, test, "esplora", 0);
    let mut daemon = live.start(None);
    assert_eq!(graph_pending(&mut daemon, "launch"), 0);
    for request_id in ["first", "second"] {
        daemon.send("test.freshness.execute", request_id, Some(ARGS));
        let response = daemon.until(request_id, Some("test.freshness.execute"));
        assert_eq!(str_at(&completed(&response), &["status"]), "done");
        assert_eq!(
            graph_pending(&mut daemon, &format!("{request_id}-graph")),
            1
        );
    }
    daemon.send(
        "test.freshness.request.cancel",
        "old",
        Some(r#"{"target_request_id": "first"}"#),
    );
    assert!(same(
        crate::live::at(&daemon.until("old", None), &["data"]),
        r#"{"cancelled": false, "queued": false}"#
    ));
    assert_eq!(graph_pending(&mut daemon, "current"), 1);
    daemon.finish();
    crate::live::no_extra_requests(&live);
    live.execute("INSERT INTO freshness_jobs(id,profile_id,job_type,source_key,source_type,source_label,status,phase,priority,created_at,updated_at) VALUES('persisted-graph','p','wallet_graph_references','wallet_graph:a','onchain_wallet','Graph','running','backend_fetch',200,'now','now')");
    // The second session reaches no chain: its recorded tape is empty.
    let second = Live::new(MODULE, test, "esplora", 1);
    std::fs::copy(&live.prepared.book, &second.prepared.book).unwrap();
    let mut daemon = second.start(None);
    daemon.send("status", "read", None);
    assert_eq!(field(&daemon.until("read", None), "kind"), "status");
    assert_eq!(graph_pending(&mut daemon, "restart"), 0);
    daemon.send(
        "test.freshness.execute",
        "enqueue",
        Some(r#"{"rates": false, "journals": false, "run": false}"#),
    );
    let response = daemon.until("enqueue", Some("test.freshness.execute"));
    assert!(same(
        crate::live::at(&response, &["data", "completed"]),
        "[]"
    ));
    assert_eq!(graph_pending(&mut daemon, "no-run"), 0);
    assert_eq!(
        second.count(
            "SELECT count(*) FROM freshness_jobs WHERE id='persisted-graph' AND status='running'"
        ),
        1
    );
    daemon.finish();
    crate::live::no_extra_requests(&second);
}

pub fn test_live_freshness_preconsent_and_early_request_fences() {
    let live = Live::new(
        MODULE,
        "test_live_freshness_preconsent_and_early_request_fences",
        "early-cancel",
        0,
    );
    let mut daemon = live.start(None);
    daemon.send("status", "read", None);
    assert_eq!(field(&daemon.until("read", None), "kind"), "status");
    let before = crate::golden::snapshot(&live.prepared.book);
    daemon.send(
        "test.freshness.request.cancel",
        "cancel",
        Some(r#"{"target_request_id": "sync"}"#),
    );
    assert!(same(
        crate::live::at(&daemon.until("cancel", None), &["data"]),
        r#"{"cancelled": true, "queued": true}"#
    ));
    daemon.send("test.freshness.execute", "sync", Some(ARGS));
    let response = daemon.until("sync", Some("test.freshness.execute"));
    assert_eq!(
        str_at(&response, &["error", "code"]),
        "cancelled",
        "{response:?}"
    );
    crate::golden::assert_same_book(
        "early-cancel",
        &crate::golden::snapshot(&live.prepared.book),
        &before,
    );
    assert_eq!(graph_pending(&mut daemon, "fenced-graph"), 0);
    daemon.finish();
    crate::live::no_extra_requests(&live);
}

pub fn test_live_freshness_refuses_replay_and_foreign_request_cancellation() {
    let (live, mut daemon) = running(
        "test_live_freshness_refuses_replay_and_foreign_request_cancellation",
        "",
    );
    daemon.send(
        "test.freshness.request.cancel",
        "stale",
        Some(r#"{"target_request_id": "sync", "expected_scope": {"workspace_id": "w", "profile_id": "other"}}"#),
    );
    assert_eq!(
        str_at(&daemon.until("stale", None), &["error", "code"]),
        "stale_context"
    );
    daemon.send(
        "test.freshness.request.cancel",
        "foreign",
        Some(r#"{"target_request_id": "other"}"#),
    );
    assert!(same(
        crate::live::at(&daemon.until("foreign", None), &["data"]),
        r#"{"cancelled": true, "queued": true}"#
    ));
    daemon.send("test.freshness.execute", "sync", Some(ARGS));
    assert_eq!(
        str_at(&daemon.until("sync", None), &["error", "code"]),
        "interaction_required"
    );
    live.release();
    let response = daemon.until("sync", Some("test.freshness.execute"));
    assert_eq!(str_at(&completed(&response), &["status"]), "done");
    let before = crate::golden::snapshot(&live.prepared.book);
    daemon.send("test.freshness.execute", "sync", Some(ARGS));
    assert_eq!(
        str_at(&daemon.until("sync", None), &["error", "code"]),
        "interaction_required"
    );
    crate::golden::assert_same_book(
        "replay",
        &crate::golden::snapshot(&live.prepared.book),
        &before,
    );
    daemon.finish();
    crate::live::no_extra_requests(&live);
}

pub fn test_live_freshness_quiesces_before_lifecycle_acknowledgement() {
    let param = "ui.profiles.switch-args2-esplora";
    let (live, mut daemon) = running(
        "test_live_freshness_quiesces_before_lifecycle_acknowledgement",
        param,
    );
    live.execute(
        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','Second','EUR','later');
         INSERT INTO workspaces(id,label,created_at) VALUES('v','Other','later');
         INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('r','v','Other','EUR','later');",
    );
    daemon.send(
        "ui.profiles.switch",
        "lifecycle",
        Some(r#"{"profile_id": "r", "workspace_id": "v"}"#),
    );
    daemon.send("status", "barrier", None);
    drain_without(&mut daemon, "lifecycle");
    live.release();
    let response = daemon.until("lifecycle", None);
    assert_eq!(
        field(&response, "kind"),
        "ui.profiles.switch",
        "{response:?}"
    );
    let sync: Vec<_> = daemon
        .rows
        .iter()
        .filter(|row| {
            field(row, "request_id") == "sync" && field(row, "kind") == "test.freshness.execute"
        })
        .cloned()
        .collect();
    assert_eq!(sync.len(), 1);
    assert_eq!(str_at(&completed(&sync[0]), &["status"]), "cancelled");
    unpublished(&live);
    assert_eq!(field(&daemon.until("barrier", None), "kind"), "status");
    daemon.finish();
    crate::live::no_extra_requests(&live);
}

pub fn test_live_freshness_encrypted_graph_handoff_drops_on_lock() {
    let key = "Synthetic graph handoff 🔑 passphrase";
    let live = Live::new(
        MODULE,
        "test_live_freshness_encrypted_graph_handoff_drops_on_lock",
        "esplora",
        0,
    );
    let mut daemon = live.start(None);
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
    daemon.send("test.freshness.execute", "sync", Some(ARGS));
    let response = daemon.until("sync", Some("test.freshness.execute"));
    assert_eq!(str_at(&completed(&response), &["status"]), "done");
    assert_eq!(graph_pending(&mut daemon, "after"), 1);
    daemon.send("daemon.lock", "lock", None);
    assert_eq!(field(&daemon.until("lock", None), "kind"), "daemon.lock");
    assert_eq!(graph_pending(&mut daemon, "locked"), 0);
    daemon.send("test.freshness.execute", "locked-sync", Some(ARGS));
    assert_eq!(
        field(&daemon.until("locked-sync", None), "kind"),
        "auth_required"
    );
    let jobs: i64 = crate::live::open(&live.prepared.book, Some(key))
        .query_row(
            "SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 0);
    let rows: String = daemon.rows.iter().map(crate::session::text).collect();
    assert!(!rows.contains(key) && !rows.contains("\\ud83d"));
    daemon.finish();
    crate::live::no_extra_requests(&live);
}

pub fn test_reference_source_control_checkpoint_repro_and_native_parity() {
    // The pause of a wallet source returns Python's control payload (recorded
    // as the session's `control` row) and leaves Python's book.
    let test = "test_reference_source_control_checkpoint_repro_and_native_parity";
    let live = Live::new(MODULE, test, "wallet-pause", 0);
    let mut daemon = live.start(None);
    daemon.send(
        "test.freshness.execute",
        "enqueue",
        Some(r#"{"rates": false, "journals": false, "run": false}"#),
    );
    daemon.until("enqueue", Some("test.freshness.execute"));
    live.execute(r#"UPDATE freshness_source_states SET checkpoint_json='{"script_membership": {"synthetic": ["private-script-member"]}}'"#);
    let source_key: String = live
        .book()
        .query_row("SELECT source_key FROM freshness_jobs", [], |r| r.get(0))
        .unwrap();
    daemon.send(
        "test.freshness.pause",
        "control",
        Some(&format!(r#"{{"source_key": "{source_key}"}}"#)),
    );
    let actual = daemon.until("control", None);
    assert!(!crate::session::text(&actual).contains("private-script-member"));
    let session = &crate::golden::calls(&live.case)[0];
    let recorded = crate::golden::list(crate::golden::value(session, "rows"))
        .iter()
        .find(|row| field(row, "request_id") == "control")
        .expect("the recorded control row")
        .clone();
    assert_eq!(
        crate::session::text(&actual),
        crate::session::text(&recorded)
    );
    daemon.finish();
    crate::live::no_extra_requests(&live);
    crate::golden::assert_same_book(
        test,
        &crate::golden::snapshot(&live.prepared.book),
        crate::golden::value(session, "after"),
    );
}
