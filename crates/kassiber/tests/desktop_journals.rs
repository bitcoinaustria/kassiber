//! Desktop journal kinds served by the daemon, and the journal CLI leaves
//! they share, against frozen goldens (see `support`). These replaced
//! `tests/test_rust_journal_{desktop,views,commands,ai,quarantine_resolve}.py`
//! and `tests/test_rust_quarantine_groups.py`, which compared the same
//! sessions with the Python reference. The books are the Python-seeded ones
//! those tests started from, frozen in `tests/data/books`.

#![cfg(feature = "test-harness")]

mod support;

use kassiber_pyjson::Value;
use support::{kind_of, list, path, text, Daemon, Desk};

const KIND: &str = "ui.journals.process";
const OPEN: &str = r#"{"kind": "status", "request_id": "open"}"#;

fn journal_desk(golden: &str, book: &str) -> Desk {
    let mut desk = Desk::from_book(golden, book);
    desk.journal_env();
    desk
}

/// The test kind that reports whether the native route serves a request.
fn select_request(args: Option<&str>) -> String {
    format!(
        r#"{{"kind": "test.freshness.select", "request_id": "select", "args": {{"kind": "ui.journals.process", "args": {}, "request_id": "process"}}}}"#,
        args.unwrap_or("null")
    )
}

fn process_request(args: Option<&str>) -> String {
    match args {
        Some(args) => {
            format!(r#"{{"kind": "ui.journals.process", "request_id": "process", "args": {args}}}"#)
        }
        None => r#"{"kind": "ui.journals.process", "request_id": "process"}"#.to_owned(),
    }
}

/// `run()` of the Python test: open the book, check the hybrid selection,
/// then process; returns the process terminal.
fn process(desk: &mut Desk, args: Option<&str>, mutates: bool) -> Value {
    let requests = [OPEN.to_owned(), select_request(args), process_request(args)];
    let responses = desk.raw_session(KIND, &requests, mutates);
    assert_eq!(
        path(&responses[1], &["data", "supported"]),
        Some(&Value::Bool(true))
    );
    responses[2].clone()
}

#[test]
fn journals_desktop_match_ordered_envelopes_events_and_all_tables() {
    let mut desk = journal_desk(
        "journals_desktop_match_ordered_envelopes_events_and_all_tables",
        "auto-exact",
    );
    let terminal = process(&mut desk, None, true);
    assert_eq!(kind_of(&terminal), KIND);
    let applied = path(&terminal, &["data", "auto_pair", "applied"]);
    assert!(
        matches!(applied, Some(Value::Int(n)) if *n > 0.into()),
        "{applied:?}"
    );
    desk.check();
}

/// Review recalculates after the owner's own decisions (#707): nothing is
/// paired, and the rebuild recounts the cached transfer candidates.
#[test]
fn journals_desktop_without_auto_pair_recount_like_python() {
    let mut desk = journal_desk(
        "journals_desktop_without_auto_pair_recount_like_python",
        "auto-exact",
    );
    let terminal = process(&mut desk, Some(r#"{"auto_pair": false}"#), true);
    assert_eq!(kind_of(&terminal), KIND);
    assert!(path(&terminal, &["data", "auto_pair"]).is_none());
    desk.check();
}

#[test]
fn journals_desktop_arguments_cannot_change_scope_or_pairing_authority() {
    let mut desk = journal_desk(
        "journals_desktop_arguments_cannot_change_scope_or_pairing_authority",
        "auto-exact",
    );
    for args in [
        "[]",
        "false",
        "7",
        "\"text\"",
        r#"{"auto_pair": "no"}"#,
        r#"{"auto_pair": null}"#,
        r#"{"auto_pair": false, "profile": "foreign"}"#,
        r#"{"profile": "foreign"}"#,
        r#"{"wallet": "foreign", "all": true}"#,
        r#"{"consent": true}"#,
    ] {
        process(&mut desk, Some(args), false);
    }
    desk.check();
}

#[test]
fn journals_desktop_unresolved_active_profile_matches_python_plain_resolution() {
    let mut desk = journal_desk(
        "journals_desktop_unresolved_active_profile_matches_python_plain_resolution",
        "auto-exact",
    );
    desk.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile')");
    process(&mut desk, None, true);
    desk.check();
}

/// The native worker pauses at `phase` until a release file appears; none
/// does here, so only a cancel or a retirement ends it.
fn paused(desk: &Desk, phase: &str) -> [(String, String); 2] {
    [
        (
            "KASSIBER_RUST_TEST_JOURNAL_WAIT_PHASE".to_owned(),
            phase.to_owned(),
        ),
        (
            "KASSIBER_RUST_TEST_JOURNAL_WAIT_RELEASE".to_owned(),
            desk.base_path()
                .join(format!("release-{phase}"))
                .to_string_lossy()
                .into_owned(),
        ),
    ]
}

fn read_until_phase(daemon: &mut Daemon, phase: &str) {
    loop {
        let row = kassiber_pytext::loads(&daemon.read()).unwrap();
        if path(&row, &["data", "phase"]).and_then(text) == Some(phase) {
            return;
        }
    }
}

#[test]
fn journals_desktop_cancellation_and_retirement_join_the_connection_owner() {
    let desk = journal_desk(
        "journals_desktop_cancellation_and_retirement_join_the_connection_owner",
        "auto-exact",
    );
    let before = desk.tables();
    let env = paused(&desk, "storing");
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut daemon = Daemon::spawn(&desk, &env);
    daemon.send(OPEN);
    daemon.until("open", "status");
    daemon.request(KIND, "process", None);
    read_until_phase(&mut daemon, "storing");
    daemon.request(
        "ui.freshness.run",
        "compete",
        Some(r#"{"rates": false, "journals": true}"#),
    );
    let busy = daemon.until("compete", "ui.freshness.run");
    assert_eq!(
        path(&busy, &["error", "code"]).and_then(text),
        Some("project_operation_in_progress")
    );
    daemon.request(
        "test.freshness.request.cancel",
        "control",
        Some(r#"{"target_request_id": "process"}"#),
    );
    let control = daemon.until("control", "test.freshness.request.cancel");
    assert_eq!(
        path(&control, &["data", "cancelled"]),
        Some(&Value::Bool(true))
    );
    let terminal = daemon.until("process", KIND);
    assert_eq!(
        path(&terminal, &["error", "code"]).and_then(text),
        Some("cancelled"),
        "{terminal:?}"
    );
    daemon.request("daemon.shutdown", "stop", None);
    daemon.until("stop", "daemon.shutdown");
    daemon.exit_cleanly();
    assert_eq!(desk.tables(), before);
}

#[test]
fn journals_desktop_cancellation_before_commit_rolls_back_pairs_and_every_journal_write() {
    let mut desk = journal_desk(
        "journals_desktop_cancellation_before_commit_rolls_back_pairs_and_every_journal_write",
        "auto-exact-current",
    );
    desk.env.insert(
        "KASSIBER_RUST_TEST_JOURNAL_CANCEL_BEFORE_COMMIT".to_owned(),
        "1".to_owned(),
    );
    let terminal = process(&mut desk, None, false);
    assert_eq!(
        path(&terminal, &["error", "code"]).and_then(text),
        Some("cancelled")
    );
    desk.check();
}

#[test]
fn journals_desktop_closed_terminal_output_preserves_committed_book_and_stops() {
    // The book a complete run leaves.
    let mut reference = journal_desk(
        "journals_desktop_closed_terminal_output_preserves_committed_book_and_stops",
        "auto-exact-current",
    );
    let before = reference.masked_tables();
    reference.raw_session(KIND, &[OPEN.to_owned(), process_request(None)], true);
    let expected = support::table_changes(&before, &reference.tables());
    assert!(!expected.is_empty());
    reference.check();

    let desk = journal_desk("unused", "auto-exact-current");
    let before = desk.tables();
    let env = paused(&desk, "complete");
    let release = std::path::PathBuf::from(&env[1].1);
    let mut command = desk.command(&["daemon"]);
    command.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    use std::io::{BufRead, Write};
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    writeln!(stdin, "{OPEN}").unwrap();
    writeln!(stdin, "{}", process_request(None)).unwrap();
    stdin.flush().unwrap();
    loop {
        line.clear();
        assert!(
            stdout.read_line(&mut line).unwrap() > 0,
            "no complete phase"
        );
        let row = kassiber_pytext::loads(line.trim_end()).unwrap();
        if path(&row, &["data", "phase"]).and_then(text) == Some("complete") {
            break;
        }
    }
    drop(stdout);
    std::fs::write(&release, "").unwrap();
    // The daemon must stop by itself; its stdin stays open.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the daemon kept running after its output closed");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    drop(stdin);
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert_eq!(status.code(), Some(1));
    assert_eq!(stderr, "");
    assert_eq!(support::table_changes(&before, &desk.tables()), expected);
}

#[test]
fn journals_ai_journal_step_never_pairs_and_matches_python() {
    let mut desk = journal_desk(
        "journals_ai_journal_step_never_pairs_and_matches_python",
        "auto-exact",
    );
    let requests = [
        OPEN.to_owned(),
        r#"{"kind": "test.journals.refresh", "request_id": "tool", "args": {"auto_pair": false}}"#
            .to_owned(),
    ];
    let responses = desk.raw_session("test.journals.refresh", &requests, true);
    assert_eq!(kind_of(&responses[1]), KIND);
    assert!(path(&responses[1], &["data", "auto_pair"]).is_none());
    desk.check();
}

/// `journals process` with the development replay armed: typed refusals
/// answer natively, with Python's envelope.
fn hybrid_refusal(golden: &str, book: &str) {
    let mut desk = journal_desk(golden, book);
    desk.env.insert(
        "KASSIBER_RUST_TEST_JOURNAL_FALLBACK".to_owned(),
        "1".to_owned(),
    );
    let output = desk.run(&["journals", "process"], b"");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    desk.golden
        .line(&format!("exit {:?}", output.status.code()));
    for line in stdout.lines() {
        let line = desk.normalize(line);
        desk.golden.line(&format!("stdout {line}"));
    }
    for line in stderr.lines() {
        let line = desk.normalize(line);
        desk.golden.line(&format!("stderr {line}"));
    }
    assert_ne!(
        output.status.code(),
        Some(3),
        "requested a replay: {stdout}"
    );
    desk.check();
}

#[test]
fn journals_hybrid_plain_counts_native_build_refusal() {
    hybrid_refusal(
        "journals_hybrid_plain_counts_native_build_refusal",
        "plain-build-refusal",
    );
}

#[test]
fn journals_hybrid_plain_counts_native_label_refusal() {
    hybrid_refusal(
        "journals_hybrid_plain_counts_native_label_refusal",
        "plain-label-refusal",
    );
}

// --- the registered reads (tests/test_rust_journal_views.py) ---

const VIEW_KINDS: &[&str] = &[
    "ui.journals.snapshot",
    "ui.journals.events.list",
    "ui.journals.transfers.list",
    "ui.journals.quarantine",
];

const VIEW_ARGUMENTS: &[&str] = &[
    "null",
    "{}",
    "[]",
    "7",
    "\"text\"",
    r#"{"limit": 1}"#,
    r#"{"limit": true}"#,
    r#"{"limit": "2"}"#,
    r#"{"limit": 2.9}"#,
    r#"{"limit": 0}"#,
    r#"{"limit": -3}"#,
    r#"{"limit": "x"}"#,
    r#"{"limit": null}"#,
    r#"{"limit": [1]}"#,
    r#"{"limit": 1000000000000000000000000000000}"#,
    r#"{"limit": 9999}"#,
    r#"{"unexpected": 1, "another": 2}"#,
];

const EVENT_ARGUMENTS: &[&str] = &[
    r#"{"transaction": "a"}"#,
    r#"{"transaction": "  a  "}"#,
    r#"{"transaction": "   "}"#,
    r#"{"transaction": "missing"}"#,
    r#"{"transaction": 7}"#,
    r#"{"transaction": null}"#,
    r#"{"transaction": "a", "limit": 1}"#,
];

const QUARANTINE_ARGUMENTS: &[&str] = &[
    r#"{"scope": "attention"}"#,
    r#"{"scope": "waiting"}"#,
    r#"{"scope": "all", "offset": 1}"#,
    r#"{"scope": "x"}"#,
    r#"{"scope": 7}"#,
    r#"{"offset": -1}"#,
    r#"{"offset": true}"#,
    r#"{"offset": "1"}"#,
    r#"{"offset": 1000000000000000000000000000000}"#,
    r#"{"limit": 1, "offset": 2}"#,
    r#"{"limit": 200, "scope": "waiting"}"#,
];

/// Every registered read once per argument, after opening the book; reads
/// never write. Returns the terminal of every read.
fn views(desk: &mut Desk) -> Vec<Value> {
    let mut requests = vec![OPEN.to_owned()];
    let mut add = |kind: &str, args: &str| {
        let number = requests.len() - 1;
        requests.push(format!(
            r#"{{"kind": "{kind}", "request_id": "r{number}", "args": {args}}}"#
        ));
    };
    for kind in VIEW_KINDS {
        for args in VIEW_ARGUMENTS {
            add(kind, args);
        }
    }
    for args in EVENT_ARGUMENTS {
        add("ui.journals.events.list", args);
    }
    for args in QUARANTINE_ARGUMENTS {
        add("ui.journals.quarantine", args);
    }
    let mut rows = desk.raw_session("ui.journals.snapshot", &requests, false);
    rows.remove(0);
    rows
}

#[test]
fn journals_views_plain_books_match_python() {
    let mut desk = journal_desk("journals_views_plain_books_match_python", "plain-unchanged");
    views(&mut desk);
    desk.check();
}

#[test]
fn journals_views_plain_blocked_processed_books_match_python() {
    let mut desk = journal_desk(
        "journals_views_plain_blocked_processed_books_match_python",
        "plain-blocked-processed",
    );
    let rows = views(&mut desk);
    assert!(rows.iter().any(|row| kind_of(row) == "ui.journals.snapshot"
        && path(row, &["data", "status", "quarantines"]).is_some_and(kassiber_envelope::truthy)));
    desk.check();
}

#[test]
fn journals_views_rows_carry_pair_metadata_like_python() {
    let mut desk = journal_desk(
        "journals_views_rows_carry_pair_metadata_like_python",
        "paired-fee-processed",
    );
    views(&mut desk);
    assert!(desk.golden.text().contains("\"pairId\""));
    desk.check();
}

#[test]
fn journals_views_malformed_freshness_markers_match_python() {
    let mut desk = journal_desk(
        "journals_views_malformed_freshness_markers_match_python",
        "plain-changed-processed",
    );
    desk.change("UPDATE profiles SET journal_input_version=journal_input_version+1");
    views(&mut desk);
    desk.check();
}

#[test]
fn journals_views_generated_quarantines_match_python() {
    let mut desk = journal_desk(
        "journals_views_generated_quarantines_match_python",
        "gaps-10-processed",
    );
    views(&mut desk);
    desk.check();
}

#[test]
fn journals_quarantine_pair_evidence_and_refresh_error_match_python() {
    let mut desk = journal_desk(
        "journals_quarantine_pair_evidence_and_refresh_error_match_python",
        "paired-short-processed",
    );
    desk.change(
        "INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,last_error_at,\
         last_error_code,last_error_message,last_success_at,updated_at) VALUES('p','journals:p','journals','Journals','error',\
         '2099-01-01T00:00:00Z','journal_refresh_failed','failed for cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd','2020-01-01T00:00:00Z','2020')",
    );
    let rows = views(&mut desk);
    let pages: Vec<&Value> = rows
        .iter()
        .filter(|row| kind_of(row) == "ui.journals.quarantine")
        .filter_map(|row| path(row, &["data"]))
        .collect();
    assert!(pages
        .iter()
        .any(|page| path(page, &["summary", "freshness", "last_error"])
            .is_some_and(kassiber_envelope::truthy)));
    assert!(pages
        .iter()
        .any(|page| list(page, &["items"]).iter().any(|item| path(
            item,
            &["evidence", "pair_fingerprint"]
        )
        .is_some())));
    desk.check();
}

// --- ui.journals.quarantine.resolve (tests/test_rust_journal_quarantine_resolve.py) ---

const RESOLVE: &str = "ui.journals.quarantine.resolve";

/// The metadata service draws its audit ids and edit times from the system
/// clock, so only those are masked, as the Python test did.
fn resolve_table_mask(table: &str, columns: &[String], values: &mut [String]) {
    let edit = matches!(table, "transaction_edit_events" | "transaction_edit_fields");
    if !(edit || table.starts_with("journal_") || table == "transactions") {
        return;
    }
    for value in values.iter_mut() {
        *value = support::mask_uuid_and_now(value);
    }
    if table == "transactions" {
        // The override stamps its capture time from the system clock.
        let index = columns
            .iter()
            .position(|c| c == "pricing_fetched_at")
            .unwrap();
        if values[index] != "NULL" && values[index] != "\"\"" {
            values[index] = "<now>".to_owned();
        }
    }
}

fn resolve_desk(golden: &str, book: &str, sql: &str) -> Desk {
    let mut desk = journal_desk(golden, book);
    desk.change(sql);
    desk.line_mask = Some(support::mask_uuid_and_now);
    desk.table_mask = Some(resolve_table_mask);
    desk
}

fn quarantined(golden: &str) -> Desk {
    let desk = resolve_desk(
        golden,
        "plain-blocked-processed",
        "UPDATE transactions SET external_id='ext-a' WHERE id='a'",
    );
    let count: i64 = desk
        .connection()
        .query_row("SELECT COUNT(*) FROM journal_quarantines", [], |r| r.get(0))
        .unwrap();
    assert!(count > 0);
    desk
}

/// Open the book, then one `kind` request per argument (`None` omits it).
fn reads(kind: &str, args: &[Option<&str>]) -> Vec<String> {
    let mut requests = vec![OPEN.to_owned()];
    for (number, args) in args.iter().enumerate() {
        requests.push(match args {
            Some(args) => {
                format!(r#"{{"kind": "{kind}", "request_id": "r{number}", "args": {args}}}"#)
            }
            None => format!(r#"{{"kind": "{kind}", "request_id": "r{number}"}}"#),
        });
    }
    requests
}

const RESOLVE_REFUSALS: &[Option<&str>] = &[
    None,
    Some("[]"),
    Some("{}"),
    Some(r#"{"bogus": 1, "transaction": "a"}"#),
    Some(r#"{"transaction": "  ", "action": "exclude", "reason": "r"}"#),
    Some(r#"{"transaction": 7, "action": "exclude", "reason": "r"}"#),
    Some(r#"{"transaction": "a", "action": "delete", "reason": "r"}"#),
    Some(r#"{"transaction": "a", "action": ["exclude"], "reason": "r"}"#),
    Some(r#"{"transaction": "a", "action": "exclude"}"#),
    Some(r#"{"transaction": "a", "action": "exclude", "reason": "   "}"#),
    Some(r#"{"transaction": "a", "action": "exclude", "reason": "r", "reprocess": 1}"#),
    Some(r#"{"transaction": "a", "action": "price_override", "reason": "r"}"#),
    Some(
        r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": 1, "fiat_value": 2}"#,
    ),
    Some(r#"{"transaction": "a", "action": "exclude", "reason": "r", "fiat_value": 2}"#),
    Some(r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": "abc"}"#),
    Some(r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": "-1"}"#),
    Some(r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": "0"}"#),
    Some(r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": ""}"#),
    Some(r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": "NaN"}"#),
    Some(
        r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_value": "Infinity"}"#,
    ),
    Some(r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_value": "-5"}"#),
    Some(r#"{"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": true}"#),
    Some(r#"{"transaction": "b", "action": "exclude", "reason": "r"}"#),
    Some(r#"{"transaction": "missing", "action": "exclude", "reason": "r"}"#),
];

#[test]
fn journals_quarantine_resolve_refusals_match_python_without_writes() {
    let mut desk = quarantined("journals_quarantine_resolve_refusals_match_python_without_writes");
    let rows = desk.raw_session(RESOLVE, &reads(RESOLVE, RESOLVE_REFUSALS), false);
    assert!(rows[1..].iter().all(|row| kind_of(row) == "error"));
    desk.check();
}

#[test]
fn journals_quarantine_resolve_matches_python_envelope_and_book() {
    for (name, request) in [
        (
            "rate",
            r#""action": "price_override", "fiat_rate": "41234.5""#,
        ),
        (
            "float_rate",
            r#""action": "price_override", "fiat_rate": 41234.5"#,
        ),
        (
            "value",
            r#""action": "price_override", "fiat_value": "4123.45678901234567890123456789""#,
        ),
        ("exclude", r#""action": "exclude""#),
    ] {
        let mut desk = quarantined(&format!(
            "journals_quarantine_resolve_matches_python_envelope_and_book_{name}"
        ));
        let args = format!(
            r#"{{"transaction": " ext-a ", "reason": " reviewed receipt ", "reprocess": false, {request}}}"#
        );
        let rows = desk.raw_session(RESOLVE, &reads(RESOLVE, &[Some(&args)]), true);
        assert_eq!(kind_of(&rows[1]), RESOLVE, "{name}");
        assert_eq!(
            path(&rows[1], &["data", "reprocessed"]),
            Some(&Value::Bool(false))
        );
        desk.check();
    }
}

// --- quarantine root causes (tests/test_rust_quarantine_groups.py) ---

/// `a` is a root waiting rows name; `b` and `s` wait on it; `i` is
/// downstream with no nameable root; `t` is a root of its own.
const HELD: &str = r#"
DELETE FROM journal_quarantines;
INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at) VALUES
 ('a','w','p','missing_spot_price','{}','2024-05-01T00:00:00Z'),
 ('b','w','p','transfer_pair_dependency_blocked','{"blocked_by_transaction_ids": ["a"]}','2024-05-01T00:00:00Z'),
 ('s','w','p','custody_basis_barrier','{"root_transaction_ids": ["a", "missing"]}','2024-05-01T00:00:00Z'),
 ('i','w','p','basis_provenance_incomplete','{}','2024-05-01T00:00:00Z'),
 ('t','w','p','insufficient_lots','{"asset": "BTC"}','2024-05-01T00:00:00Z');
UPDATE profiles SET fiat_currency='eur';
"#;

fn held(golden: &str) -> Desk {
    resolve_desk(golden, "plain-unchanged", HELD)
}

/// `journals quarantine groups`: exit status, stdout and stderr in the golden.
fn groups(desk: &mut Desk, extra: &[&str]) -> Option<Value> {
    let mut args = vec!["journals", "quarantine", "groups"];
    args.extend_from_slice(extra);
    let before = desk.tables();
    let output = desk.run(&args, b"");
    assert_eq!(desk.tables(), before);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    desk.golden.line(&format!("$ {}", extra.join(" ")));
    desk.golden
        .line(&format!("exit {:?}", output.status.code()));
    for line in stdout.lines() {
        let line = desk.normalize(line);
        desk.golden.line(&format!("stdout {line}"));
    }
    for line in stderr.lines() {
        let line = desk.normalize(line);
        desk.golden.line(&format!("stderr {line}"));
    }
    (!stdout.is_empty()).then(|| kassiber_pytext::loads(&stdout).unwrap())
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// `base64.urlsafe_b64encode`.
fn b64encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `base64.urlsafe_b64decode` of a well-formed cursor.
fn b64decode(text: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut count = 0;
    let mut out = Vec::new();
    for c in text.bytes().filter(|c| *c != b'=') {
        let value = B64.iter().position(|b| *b == c).unwrap() as u32;
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

fn tamper(cursor: &str, change: impl FnOnce(&mut Value)) -> String {
    let text = String::from_utf8(b64decode(cursor)).unwrap();
    let mut payload = kassiber_pytext::loads(&text).unwrap();
    change(&mut payload);
    let text = kassiber_pyjson::dumps(&payload, &kassiber_pyjson::Options::new()).unwrap();
    b64encode(text.as_bytes())
}

fn after(values: Vec<Value>) -> impl FnOnce(&mut Value) {
    move |payload| kassiber_envelope::set(payload, "after", Value::List(values))
}

fn scope_item(index: usize, change: fn(&Value) -> Value) -> impl FnOnce(&mut Value) {
    move |payload| {
        let Some(Value::List(scope)) = path(payload, &["scope"]).cloned() else {
            panic!("scope")
        };
        let mut scope = scope;
        scope[index] = change(&scope[index]);
        kassiber_envelope::set(payload, "scope", Value::List(scope));
    }
}

#[test]
fn journals_groups_leaf_matches_python_byte_for_byte() {
    let mut desk = held("journals_groups_leaf_matches_python_byte_for_byte");
    let overview = groups(&mut desk, &[]).unwrap();
    let data = path(&overview, &["data"]).unwrap();
    assert_eq!(path(data, &["count"]), Some(&Value::from(5)));
    assert_eq!(path(data, &["waiting_count"]), Some(&Value::from(2)));
    let ids = |key: &str| -> Vec<String> {
        list(data, &[key])
            .iter()
            .map(|g| path(g, &["group_id"]).and_then(text).unwrap().to_owned())
            .collect()
    };
    assert_eq!(ids("by_root"), ["root:a", "root:t"]);
    assert_eq!(ids("unrooted"), ["unrooted:basis_provenance_incomplete"]);
    for extra in [
        &["--limit", "1"][..],
        &["--limit", "100"],
        &["--limit", "0"],
        &["--limit", "101"],
        &["--group-id", "root:a"],
        &["--group-id", "root:t"],
        &["--group-id", "root:b"],
        &["--group-id", "reason:missing_spot_price"],
        &["--group-id", "unrooted:basis_provenance_incomplete"],
        &["--group-id", "reason:none"],
        &["--group-id", "bogus"],
        &["--group-id", ""],
        &["--group-id", "root:a", "--limit", "200"],
        &["--group-id", "root:a", "--limit", "201"],
        &["--cursor", "abc"],
        &["--workspace", "missing"],
    ] {
        groups(&mut desk, extra);
    }
    // Page through root:a one member at a time, with every broken cursor.
    let mut cursor: Option<String> = None;
    let mut members = Vec::new();
    loop {
        let mut extra = vec!["--group-id", "root:a", "--limit", "1"];
        if let Some(cursor) = &cursor {
            extra.extend(["--cursor", cursor.as_str()]);
        }
        let page = groups(&mut desk, &extra).unwrap();
        for member in list(&page, &["data", "members"]) {
            members.push(
                path(member, &["transaction_id"])
                    .and_then(text)
                    .unwrap()
                    .to_owned(),
            );
        }
        cursor = path(&page, &["data", "next_cursor"])
            .and_then(text)
            .map(str::to_owned);
        let Some(next) = &cursor else { break };
        let empty = Value::Str(Default::default());
        let broken = [
            next[..next.len() - 2].to_owned(),
            format!("{next}!!"),
            "e30=".to_owned(),
            b64encode(b"[]"),
            tamper(next, after(vec![Value::from(0), Value::Str("x".into())])),
            tamper(
                next,
                after(vec![Value::Bool(true), empty.clone(), empty.clone()]),
            ),
            tamper(
                next,
                scope_item(1, |v| match v {
                    Value::Int(n) => Value::Float(n.to_string().parse().unwrap()),
                    other => panic!("scope[1] is {other:?}"),
                }),
            ),
            tamper(next, scope_item(4, |_| Value::Str("0".repeat(16).into()))),
            tamper(next, after(vec![Value::from(-1), empty.clone(), empty])),
        ];
        for broken in &broken {
            groups(&mut desk, &["--group-id", "root:a", "--cursor", broken]);
        }
        groups(&mut desk, &["--group-id", "root:t", "--cursor", next]);
    }
    assert_eq!(members, ["a", "b", "s"]);
    desk.check();
}

#[test]
fn journals_quarantine_summary_carries_currency_and_definitions() {
    let mut desk = held("journals_quarantine_summary_carries_currency_and_definitions");
    let rows = desk.raw_session(
        "ui.journals.quarantine",
        &reads(
            "ui.journals.quarantine",
            &[
                Some("{}"),
                Some(r#"{"scope": "attention"}"#),
                Some(r#"{"scope": "waiting", "limit": 1}"#),
                Some(r#"{"offset": 3}"#),
            ],
        ),
        false,
    );
    let summary = path(&rows[1], &["data", "summary"]).unwrap();
    assert_eq!(
        path(summary, &["fiat_currency"]).and_then(text),
        Some("EUR")
    );
    let Some(Value::Dict(definitions)) = path(summary, &["definitions"]) else {
        panic!("definitions")
    };
    let keys: Vec<String> = definitions
        .iter()
        .map(|(k, _)| match k {
            kassiber_pyjson::Key::Str(k) => k.as_str().unwrap().to_owned(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        keys,
        [
            "count",
            "by_reason",
            "attention_count",
            "waiting_count",
            "blocking_count",
            "scope_count",
            "group_count"
        ]
    );
    desk.check();
}

#[test]
fn journals_resolve_by_group_reports_the_same_effect_and_book() {
    for (name, args) in [
        (
            "root_reprocess",
            r#"{"group_id": " root:a ", "action": "price_override", "fiat_rate": "10000", "reason": "Statement price"}"#,
        ),
        (
            "root_unmeasured",
            r#"{"group_id": "root:a", "action": "price_override", "fiat_value": "100", "reason": "r", "reprocess": false}"#,
        ),
        (
            "own_root_unmeasured",
            r#"{"group_id": "root:t", "action": "exclude", "reason": "Not ours", "reprocess": false}"#,
        ),
        (
            "own_root_reprocess",
            r#"{"group_id": "root:t", "action": "exclude", "reason": "Not ours"}"#,
        ),
        (
            "transaction_reprocess",
            r#"{"transaction": "a", "action": "price_override", "fiat_rate": "10000", "reason": "r"}"#,
        ),
        (
            "downstream_unmeasured",
            r#"{"transaction": "i", "action": "exclude", "reason": "r", "reprocess": false}"#,
        ),
    ] {
        let mut desk = held(&format!(
            "journals_resolve_by_group_reports_the_same_effect_and_book_{name}"
        ));
        let rows = desk.raw_session(RESOLVE, &reads(RESOLVE, &[Some(args)]), true);
        let data = path(&rows[1], &["data"]).unwrap_or_else(|| panic!("{name}: {:?}", rows[1]));
        let status = path(data, &["quarantine_delta", "status"]).and_then(text);
        if args.contains("\"reprocess\": false") {
            assert_eq!(path(data, &["dependents_cleared"]), Some(&Value::Null));
            assert_eq!(status, Some("not_measured"), "{name}");
        } else {
            assert_eq!(status, Some("measured"), "{name}");
        }
        desk.check();
    }
}
