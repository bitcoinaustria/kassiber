//! `mcp serve` pinned to the desktop book under every grant and access
//! mode: the head (listing, instructions) and every covered tool, writes
//! included. The message lists and agent-access files the Python parity
//! test built from the live registries are frozen in
//! `tests/black_box/ai/mcp_every_grant.json`. Replaces the kill-holding test
//! of `tests/test_rust_mcp.py`.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;

use serde_json::Value;

use crate::support::{calendar_relative, number_hex, Sandbox, Transcript};

const DESKTOP: &str = include_str!("../../../transactions/tests/data/desktop.sql");

/// One session; input ends only once every request was answered. Requests
/// go in windows of 16 answered before the next is sent (the server
/// refuses calls beyond 32 in flight).
fn serve(sandbox: &Sandbox, messages: &[Value], extra: &[&str]) -> Vec<String> {
    let root = sandbox.root.to_string_lossy().into_owned();
    let mut command = sandbox.command();
    command
        .args(["--data-root", &root, "mcp", "serve"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let stderr = child.stderr.take().unwrap();
    let drain = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut text);
        text
    });
    let mut lines = Vec::new();
    let mut expected = 0;
    for window in messages.chunks(16) {
        expected += window
            .iter()
            .filter(|m| m.get("method").is_some() && m.get("id").is_some())
            .count();
        for message in window {
            stdin.write_all(message.to_string().as_bytes()).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
        stdin.flush().unwrap();
        while lines.len() < expected {
            let mut line = String::new();
            if stdout.read_line(&mut line).unwrap() == 0 {
                break;
            }
            lines.push(line.trim_end_matches('\n').to_owned());
        }
    }
    drop(stdin);
    for line in stdout.lines() {
        lines.push(line.unwrap());
    }
    let status = child.wait().unwrap();
    let stderr = drain.join().unwrap();
    assert_eq!((status.code(), stderr.as_str()), (Some(0), ""));
    lines
}

#[test]
fn test_every_grant_lists_and_runs_what_it_covers() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("ai/mcp_every_grant.json")).unwrap();
    let mut t = Transcript::new("mcp/every_grant");
    t.compact();
    t.mask(calendar_relative);
    t.mask(|text| {
        let text = number_hex(text, "ca-ref:", 64);
        let text = number_hex(&text, "snapshot_id\":\"", 64);
        let text = number_hex(&text, "snapshot_id\\\":\\\"", 64);
        // Each server names its calls' audit rows with a random id.
        number_hex(&text, "call_id=t:\"mcp-", 32)
    });
    // Each grant runs on its own copy at once; the transcript keeps their order.
    let runs: Vec<(Sandbox, String, Vec<String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = cases
            .iter()
            .map(|case| scope.spawn(move || run_case(case)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (case, (sandbox, before, lines)) in cases.iter().zip(&runs) {
        let grant = case["grant"].as_str().unwrap();
        let mode = case["access"].as_str().unwrap();
        t.note(&format!("grant {grant}, access {mode}"));
        let _ = std::fmt::Write::write_str(t.text_mut(), "$ mcp serve --workspace w --profile p\n");
        for line in lines {
            let _ = writeln_line(&mut t, sandbox, line);
            assert!(!line.contains("book_pin_required"), "{line}");
        }
        t.changes(sandbox, before, "after the session");
        let answered = case["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m.get("method").is_some() && m.get("id").is_some())
            .count();
        assert!(lines.len() >= answered, "{} < {answered}", lines.len());
    }
    t.finish();
}

/// One grant's session on a fresh copy of the desktop book: the sandbox,
/// its dump before, and the reply lines.
fn run_case(case: &Value) -> (Sandbox, String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;
    let mut sandbox = Sandbox::book(DESKTOP);
    let runtime = sandbox.path("rt");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let access = sandbox.path("agents.json");
    std::fs::write(&access, case["agent_access"].as_str().unwrap()).unwrap();
    std::fs::set_permissions(&access, std::fs::Permissions::from_mode(0o600)).unwrap();
    sandbox
        .set(
            "KASSIBER_OPERATOR_RUNTIME_DIR",
            runtime.to_string_lossy().into_owned(),
        )
        .set("KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR", "1")
        .set("KASSIBER_TEST_NO_EGRESS", "strict")
        // The analysis index draws its instance id (and so every
        // snapshot_id) from one counter.
        .set("KASSIBER_RUST_TEST_ANALYSIS_IDS", "sequential")
        .set("KASSIBER_RUST_TEST_ANALYSIS_TIME", "2026-03-04T05:06:07Z");
    let before = sandbox.dump();
    let lines = serve(
        &sandbox,
        case["messages"].as_array().unwrap(),
        &["--workspace", "w", "--profile", "p"],
    );
    (sandbox, before, lines)
}

fn writeln_line(t: &mut Transcript, sandbox: &Sandbox, line: &str) -> std::fmt::Result {
    std::fmt::Write::write_fmt(t.text_mut(), format_args!("< {}\n", sandbox.scrub(line)))
}
