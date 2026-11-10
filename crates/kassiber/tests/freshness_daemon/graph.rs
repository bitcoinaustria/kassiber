//! tests/test_rust_freshness_graph.py, daemon half: the graph reference
//! worker after a sync, held at its first lookup while the test acts.

use std::time::Duration;

use crate::live::{at, completed, drain_without, open, same, str_at, Live, ARGS};
use crate::session::{field, Daemon};

const MODULE: &str = "graph";
const RAW_TXID: &str = "UPDATE transactions SET raw_json=json_object('txid',external_id)";

/// A sync, then the graph worker started and held at its first lookup
/// (`pattern`, or any request).
fn synced(live: &Live, before_graph: &str) -> Daemon {
    let mut daemon = live.start(None);
    daemon.send("test.freshness.execute", "sync", Some(ARGS));
    let response = daemon.until("sync", Some("test.freshness.execute"));
    assert_eq!(str_at(&completed(&response), &["status"]), "done");
    if !before_graph.is_empty() {
        live.execute(before_graph);
    }
    daemon
}

fn background(daemon: &mut Daemon) -> kassiber_pyjson::Value {
    while !daemon
        .rows
        .iter()
        .any(|r| field(r, "kind") == "ui.freshness.background")
    {
        daemon.read();
    }
    daemon
        .rows
        .iter()
        .find(|r| field(r, "kind") == "ui.freshness.background")
        .unwrap()
        .clone()
}

fn start_graph(live: &Live, daemon: &mut Daemon, hold: Option<&str>) {
    if let Some(pattern) = hold {
        live.arm(pattern);
    }
    daemon.send("test.freshness.graph.start", "start", None);
    assert_eq!(
        field(&daemon.until("start", None), "kind"),
        "test.freshness.graph.start"
    );
    if hold.is_some() {
        live.wait_held();
    }
}

fn worker_controls(control: &str) {
    let param = format!("{control}-esplora");
    let live = Live::new(
        MODULE,
        "test_native_graph_worker_controls_and_owned_connection",
        &param,
        0,
    );
    let mut before = RAW_TXID.to_owned();
    if control == "switch" {
        before.push_str(";INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','Other','EUR','fixed')");
    }
    let mut daemon = synced(&live, &before);
    start_graph(&live, &mut daemon, Some(""));
    daemon.send("status", "status", None);
    assert_eq!(field(&daemon.until("status", None), "kind"), "status");
    let kind = if control.starts_with("graph-") {
        "wallet_graph_references"
    } else {
        "onchain_wallet_history"
    };
    let (job, source): (String, String) = live
        .book()
        .query_row(
            "SELECT id,source_key FROM freshness_jobs WHERE job_type=?",
            [kind],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let acknowledged = matches!(
        control,
        "cancel"
            | "revoke"
            | "offline"
            | "source-cancel"
            | "source-pause"
            | "graph-cancel"
            | "graph-pause"
    );
    match control {
        "cancel" | "revoke" => daemon.send(
            &format!("test.freshness.request.{control}"),
            "control",
            Some(r#"{"target_request_id": "sync"}"#),
        ),
        "source-cancel" | "graph-cancel" => daemon.send(
            "test.freshness.cancel",
            "control",
            Some(&format!(r#"{{"job_id": "{job}"}}"#)),
        ),
        "source-pause" | "graph-pause" => daemon.send(
            "test.freshness.pause",
            "control",
            Some(&format!(r#"{{"source_key": "{source}"}}"#)),
        ),
        "restore" => daemon.send(
            "ui.backup.apply",
            "control",
            Some(r#"{"token": "missing", "confirm": "RESTORE"}"#),
        ),
        other => daemon.send(&format!("daemon.{other}"), "control", None),
    }
    if acknowledged {
        assert_ne!(field(&daemon.until("control", None), "kind"), "error");
    } else {
        drain_without(&mut daemon, "control");
    }
    live.release();
    if matches!(control, "lock" | "switch" | "shutdown" | "restore") {
        assert_eq!(
            field(&daemon.until("control", None), "kind") == "error",
            control == "restore"
        );
    }
    let event = background(&mut daemon);
    assert!(
        same(at(&event, &["event"]), "true")
            && kassiber_envelope::get(&event, "request_id").is_none()
    );
    let kassiber_pyjson::Value::List(jobs) = at(&event, &["data", "completed"]) else {
        panic!("{event:?}")
    };
    let status = str_at(&jobs[0], &["status"]);
    if control == "source-cancel" {
        assert_eq!(status, "done");
    } else {
        assert!(
            matches!(status.as_str(), "cancelled" | "error"),
            "{control}: {status}"
        );
    }
    if control.ends_with("pause") {
        daemon.send(
            "test.freshness.resume",
            "resume",
            Some(&format!(r#"{{"source_key": "{source}"}}"#)),
        );
        assert!(same(
            at(&daemon.until("resume", None), &["data", "result", "paused"]),
            "false"
        ));
    }
    if control != "lock" {
        let cached = live.count("SELECT count(*) FROM transaction_graph_cache") > 0;
        assert_eq!(cached, control == "source-cancel", "{control}");
        assert_eq!(
            live.count("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references' AND status='running'"),
            0
        );
    }
    daemon.finish();
    crate::live::no_extra_requests(&live);
}

pub fn test_native_graph_worker_controls_and_owned_connection() {
    for control in [
        "cancel",
        "graph-cancel",
        "graph-pause",
        "lock",
        "restore",
        "source-pause",
    ] {
        worker_controls(control);
    }
}

pub fn test_native_graph_request_identity_and_replay_are_fenced() {
    let live = Live::new(
        MODULE,
        "test_native_graph_request_identity_and_replay_are_fenced",
        "",
        0,
    );
    let mut daemon = synced(&live, RAW_TXID);
    start_graph(&live, &mut daemon, Some(""));
    daemon.send(
        "test.freshness.request.cancel",
        "foreign",
        Some(r#"{"target_request_id": "another-request"}"#),
    );
    assert!(same(
        at(&daemon.until("foreign", None), &["data"]),
        r#"{"cancelled": true, "queued": true}"#
    ));
    live.release();
    let event = background(&mut daemon);
    let kassiber_pyjson::Value::List(jobs) = at(&event, &["data", "completed"]) else {
        panic!("{event:?}")
    };
    assert_eq!(str_at(&jobs[0], &["status"]), "done");
    daemon.send("test.freshness.execute", "sync", Some(ARGS));
    assert_eq!(
        str_at(&daemon.until("sync", None), &["error", "code"]),
        "interaction_required"
    );
    daemon.finish();
    crate::live::no_extra_requests(&live);
    assert_eq!(
        live.count("SELECT count(*) FROM freshness_jobs WHERE job_type='onchain_wallet_history'"),
        1
    );
    assert_eq!(
        live.count("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'"),
        1
    );
}

pub fn test_native_graph_closed_output_joins_worker_before_exit() {
    let live = Live::new(
        MODULE,
        "test_native_graph_closed_output_joins_worker_before_exit",
        "esplora",
        0,
    );
    let mut daemon = synced(&live, RAW_TXID);
    start_graph(&live, &mut daemon, Some(""));
    daemon.close_output();
    live.release();
    let Daemon { mut child, .. } = daemon;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon did not exit"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success());
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert_eq!(stderr, "");
    assert_eq!(
        live.count("SELECT count(*) FROM freshness_jobs WHERE status='running'"),
        0
    );
    crate::live::no_extra_requests(&live);
}

pub fn test_native_graph_enabled_worker_waits_for_terminal_handoff() {
    // Automatic: the product run hands its grant to the worker only once
    // its terminal row is out.
    let live = Live::new(
        MODULE,
        "test_native_graph_enabled_worker_waits_for_terminal_handoff",
        "True-esplora",
        0,
    );
    let mut daemon = live.start(None);
    daemon.send("test.freshness.product.execute", "sync", Some(ARGS));
    let terminal = daemon.until("sync", Some("ui.freshness.run"));
    let kassiber_pyjson::Value::List(jobs) = at(&terminal, &["data", "completed"]) else {
        panic!("{terminal:?}")
    };
    assert_eq!(str_at(&jobs[0], &["status"]), "done");
    assert!(!daemon
        .rows
        .iter()
        .any(|r| field(r, "kind") == "ui.freshness.background"));
    let event = background(&mut daemon);
    assert!(
        same(at(&event, &["event"]), "true")
            && kassiber_envelope::get(&event, "request_id").is_none()
    );
    let kassiber_pyjson::Value::List(jobs) = at(&event, &["data", "completed"]) else {
        panic!("{event:?}")
    };
    assert_eq!(str_at(&jobs[0], &["status"]), "done");
    assert!(same(at(&jobs[0], &["result", "attempted"]), "0"));
    daemon.send("test.freshness.graph.start", "again", None);
    assert_eq!(
        field(&daemon.until("again", None), "kind"),
        "test.freshness.graph.start"
    );
    daemon.finish();
    crate::live::no_extra_requests(&live);
    assert_eq!(
        live.count("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'"),
        1
    );
}

/// Every 64-digit hex run in `text`, in order.
fn hashes(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let run = bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_hexdigit())
            .count();
        if run == 64 {
            found.push(&text[at..at + 64]);
        }
        at += run.max(1);
    }
    found
}

/// The first prevout lookup the recorded graph worker made: after its
/// first `blockchain.transaction.get` of `txid`, the first one of another
/// transaction.
fn first_prevout_lookup(tape: &str, txid: &str) -> String {
    let mut worker = false;
    for line in tape.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.first() != Some(&"w") || parts.len() < 4 {
            continue;
        }
        let bytes = crate::session::base64(parts[3]);
        let text = String::from_utf8_lossy(&bytes);
        if !text.contains("blockchain.transaction.get\"") {
            continue;
        }
        for hash in hashes(&text) {
            if hash == txid {
                worker = true;
            } else if worker {
                return hash.to_owned();
            }
        }
    }
    panic!("no prevout lookup on the tape")
}

pub fn test_native_graph_cancels_between_electrum_prevout_batches() {
    let live = Live::new(
        MODULE,
        "test_native_graph_cancels_between_electrum_prevout_batches",
        "",
        0,
    );
    let tape = std::fs::read_to_string(&live.prepared.tape).unwrap();
    let session = &crate::golden::calls(&live.case)[0];
    // The row the test added: the recorded book's transaction.
    let txid = match crate::live::at(crate::golden::value(session, "after"), &["transactions"]) {
        kassiber_pyjson::Value::List(rows) => match &rows[0] {
            kassiber_pyjson::Value::List(row) => kassiber_envelope::py_str(&row[4]),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    let parent = first_prevout_lookup(&tape, &txid);
    let mut daemon = live.start(None);
    daemon.send("test.freshness.execute", "sync", Some(ARGS));
    let response = daemon.until("sync", Some("test.freshness.execute"));
    assert_eq!(str_at(&completed(&response), &["status"]), "done");
    live.execute(&format!(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,amount,asset,raw_json,created_at) \
         VALUES('row-0','w','p','a','{txid}','fp-0','fixed','inbound',99000000,'BTC','{{\"txid\": \"{txid}\"}}','fixed')"
    ));
    start_graph(&live, &mut daemon, Some(&parent));
    daemon.send(
        "test.freshness.request.cancel",
        "cancel",
        Some(r#"{"target_request_id": "sync"}"#),
    );
    assert!(same(
        at(&daemon.until("cancel", None), &["data", "cancelled"]),
        "true"
    ));
    live.release();
    background(&mut daemon);
    assert_eq!(
        live.count("SELECT count(*) FROM transaction_graph_cache"),
        0
    );
    daemon.finish();
    crate::live::no_extra_requests(&live);
    // No prevout batch after the cancel: the same lookups as recorded.
    let needle = "blockchain.transaction.get\"";
    assert_eq!(
        crate::session::replayed_requests(&live.prepared.tape, needle),
        crate::session::recorded_requests(&live.prepared.tape, needle)
    );
}

fn encrypted(param: &str) {
    let key = "Synthetic graph worker 🔑 passphrase";
    let replacement = "Synthetic replacement 🔑 passphrase";
    let stop = !param.starts_with("complete");
    let rotate = param.starts_with("rotate");
    let live = Live::new(
        MODULE,
        "test_native_graph_encrypted_worker_handoff_and_cleanup",
        param,
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
    open(&live.prepared.book, Some(key))
        .execute_batch(RAW_TXID)
        .unwrap();
    start_graph(&live, &mut daemon, Some(""));
    if stop {
        if rotate {
            daemon.send(
                "ui.secrets.change_passphrase",
                "lock",
                Some(&format!(
                    r#"{{"auth_response": {{"passphrase_secret": "{key}"}}, "new_passphrase_secret": "{replacement}"}}"#
                )),
            );
            while !daemon
                .rows
                .iter()
                .any(|r| field(r, "kind") == "test.freshness.graph.stopping")
            {
                assert_ne!(field(&daemon.read(), "request_id"), "lock");
            }
        } else {
            daemon.send("daemon.lock", "lock", None);
        }
        drain_without(&mut daemon, "lock");
    }
    live.release();
    if stop {
        assert_eq!(
            field(&daemon.until("lock", None), "kind"),
            if rotate {
                "ui.secrets.change_passphrase"
            } else {
                "daemon.lock"
            }
        );
    }
    let event = background(&mut daemon);
    let kassiber_pyjson::Value::List(jobs) = at(&event, &["data", "completed"]) else {
        panic!("{event:?}")
    };
    assert_eq!(
        str_at(&jobs[0], &["status"]),
        if stop { "cancelled" } else { "done" }
    );
    if !stop {
        daemon.send("daemon.lock", "lock", None);
        assert_eq!(field(&daemon.until("lock", None), "kind"), "daemon.lock");
    }
    let cached: i64 = open(
        &live.prepared.book,
        Some(if rotate { replacement } else { key }),
    )
    .query_row("SELECT count(*) FROM transaction_graph_cache", [], |r| {
        r.get(0)
    })
    .unwrap();
    assert!(
        if stop { cached == 0 } else { cached > 0 },
        "{param}: {cached}"
    );
    daemon.send("test.freshness.graph.start", "locked-start", None);
    assert_eq!(
        field(&daemon.until("locked-start", None), "kind"),
        "test.freshness.graph.start"
    );
    let rows = daemon.finish();
    crate::live::no_extra_requests(&live);
    let text: String = rows.iter().map(crate::session::text).collect();
    for secret in [key, replacement] {
        assert!(!text.contains(secret));
        assert!(!text.contains(&crate::session::text(&kassiber_pyjson::Value::from(secret))));
    }
}

pub fn test_native_graph_encrypted_worker_handoff_and_cleanup() {
    for param in ["complete-electrum", "rotate-esplora"] {
        encrypted(param);
    }
}
