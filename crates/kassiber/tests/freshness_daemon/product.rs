//! tests/test_rust_freshness_product.py: a product sync held at its first
//! history fetch while its source is paused.

use crate::live::{at, fetch, same, str_at, unpublished, Live};
use crate::session::{assert_same_rows, field};

pub fn test_product_source_controls_remain_available_during_fetch() {
    let param = "pause-ui.wallets.sync-esplora";
    let kind = "ui.wallets.sync";
    let args = r#"{"process_journals": false}"#;
    let live = Live::new(
        "product",
        "test_product_source_controls_remain_available_during_fetch",
        param,
        0,
    );
    let mut daemon = live.start(Some(fetch(param)));
    daemon.send(kind, "sync", Some(args));
    live.wait_held();
    let before = crate::golden::snapshot(&live.prepared.book);
    daemon.send(
        "test.freshness.select",
        "busy",
        Some(&format!(r#"{{"kind": "{kind}", "args": {args}}}"#)),
    );
    assert_eq!(
        str_at(&daemon.until("busy", None), &["error", "code"]),
        "project_operation_in_progress"
    );
    daemon.send(kind, "competing", Some(args));
    assert_eq!(
        str_at(&daemon.until("competing", None), &["error", "code"]),
        "project_operation_in_progress"
    );
    crate::golden::assert_same_book(
        param,
        &crate::golden::snapshot(&live.prepared.book),
        &before,
    );
    let source_key: String = live
        .book()
        .query_row("SELECT source_key FROM freshness_jobs", [], |r| r.get(0))
        .unwrap();
    daemon.send(
        "ui.freshness.pause",
        "control",
        Some(&format!(r#"{{"source_key": "{source_key}"}}"#)),
    );
    assert_eq!(
        field(&daemon.until("control", None), "kind"),
        "ui.freshness.pause"
    );
    live.release();
    daemon.until("sync", Some(kind));
    // The sync's rows and events are the reference's (recorded where they
    // agreed).
    let session = &crate::golden::calls(&live.case)[0];
    let recorded: Vec<_> = crate::golden::list(crate::golden::value(session, "rows"))
        .iter()
        .filter(|row| field(row, "request_id") == "sync" || same(at(row, &["event"]), "true"))
        .cloned()
        .collect();
    let actual: Vec<_> = daemon
        .rows
        .iter()
        .filter(|row| field(row, "request_id") == "sync" || same(at(row, &["event"]), "true"))
        .cloned()
        .collect();
    assert_same_rows(param, &actual, &recorded);
    daemon.send(
        "ui.freshness.resume",
        "resume",
        Some(&format!(r#"{{"source_key": "{source_key}"}}"#)),
    );
    assert!(same(
        at(&daemon.until("resume", None), &["data", "result", "paused"]),
        "false"
    ));
    unpublished(&live);
    daemon.finish();
    crate::golden::assert_same_book(
        param,
        &crate::golden::snapshot(&live.prepared.book),
        crate::golden::value(session, "after"),
    );
}
