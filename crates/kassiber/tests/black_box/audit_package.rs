//! `ui.reports.export_audit_package`: responses, every exported file
//! (bytes, permission bits, and the modification times copied evidence
//! keeps) and an unchanged book. Replaces the kill-holding tests of
//! `tests/test_rust_audit_package.py`.

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::desktop::{self, attachments, compare, NO_CONTEXT};
use crate::support::{Sandbox, Transcript};

const KIND: &str = "ui.reports.export_audit_package";
const CLOCK: &str = "2026-03-04T05:06:07Z";

fn book() -> Sandbox {
    let mut sandbox = desktop::book();
    sandbox.set("KASSIBER_RUST_TEST_REPORT_TIME", CLOCK);
    sandbox.sql(&attachments());
    sandbox
}

fn place_files(sandbox: &Sandbox) {
    let dir = sandbox.path("attachments/p");
    std::fs::create_dir_all(&dir).unwrap();
    let at = |ns: u64| UNIX_EPOCH + Duration::from_nanos(ns);
    // Fixed times, so the evidence copies' carried-over times are pinned.
    let place = |name: &str, bytes: &[u8], modified: u64| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(at(1_600_000_000_123_456_789))
                .set_modified(at(modified)),
        )
        .unwrap();
        path
    };
    place("present.pdf", b"%PDF", 1_500_000_000_000_000_007);
    let good = place("good.pdf", b"%PDF", 1_600_000_001_987_654_321);
    std::fs::set_permissions(&good, std::fs::Permissions::from_mode(0o640)).unwrap();
    place(
        "receipt.txt",
        "Beleg €\n".as_bytes(),
        1_550_000_000_000_000_011,
    );
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let relative = path
            .strip_prefix(base)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            !relative.split('/').any(|part| part.starts_with('.')),
            "staging leaves no hidden path behind: {relative}"
        );
        let meta = std::fs::symlink_metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o7777;
        if meta.is_dir() {
            out.push(format!("{relative}/ mode={mode:o}"));
            walk(&path, base, out);
            continue;
        }
        let mut line = format!("{relative} mode={mode:o}");
        if relative.split('/').any(|part| part == "evidence") {
            let mtime = meta.modified().unwrap().duration_since(UNIX_EPOCH).unwrap();
            let _ = write!(line, " mtime_ns={}", mtime.as_nanos());
        }
        let bytes = std::fs::read(&path).unwrap();
        match String::from_utf8(bytes) {
            Ok(text) => {
                let _ = write!(line, " text:\n{text}");
                if !text.ends_with('\n') {
                    line.push_str("\n<no newline>");
                }
            }
            Err(error) => {
                let _ = write!(line, " bytes={:?}", error.as_bytes());
            }
        }
        out.push(line);
    }
}

/// Every exported path, its mode, bytes, and copied evidence's mtime.
fn tree(sandbox: &Sandbox) -> String {
    let exports = sandbox.path("exports");
    let mut lines = Vec::new();
    if exports.exists() {
        walk(&exports, &exports, &mut lines);
    }
    lines.join("\n") + "\n"
}

fn package(t: &mut Transcript, sandbox: &Sandbox, args: &[Option<Value>]) -> (Vec<Value>, String) {
    place_files(sandbox);
    let exports = sandbox.path("exports");
    if exports.exists() {
        std::fs::remove_dir_all(&exports).unwrap();
    }
    let responses = compare(t, sandbox, KIND, args);
    let tree = tree(sandbox);
    t.section("exports", &sandbox.scrub(&tree));
    (responses, tree)
}

#[test]
fn test_whole_book_options_and_redacted_urls_match_python() {
    let sandbox = book();
    let mut t = Transcript::new("audit_package/whole_book_options_and_urls");
    let (responses, _) = package(
        &mut t,
        &sandbox,
        &[
            None,
            Some(json!({})),
            Some(json!({"include_url_references": false})),
            Some(json!({"include_copied_attachments": false})),
            Some(json!({"include_review_state": false})),
            Some(json!({"include_journal_state": false, "include_edit_history": true})),
            Some(
                json!({"include_copied_attachments": false, "include_url_references": false,
                "include_review_state": false, "include_journal_state": false}),
            ),
        ],
    );
    let first = &responses[0]["data"];
    assert!(first["evidence_file_count"].as_i64().unwrap() > 1);
    assert!(first["url_reference_count"].as_i64().unwrap() > 14);
    let manifest = sandbox
        .path("exports/reports")
        .join(first["filename"].as_str().unwrap())
        .join("manifest.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    let codes: Vec<&str> = manifest["package"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect();
    for code in [
        "attachment_hash_mismatch",
        "attachment_storage_path_invalid",
        "secret_bearing_url_redacted",
        "sensitive_material_excluded",
    ] {
        assert!(codes.contains(&code), "{code} in {codes:?}");
    }
    assert!(manifest["package"]["url_references"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["redacted_url"].as_str().is_some_and(|u| !u.is_empty())));
    t.finish();
}

#[test]
fn test_bounded_scopes_and_refusals_match_python() {
    let sandbox = book();
    let mut t = Transcript::new("audit_package/bounded_scopes_and_refusals");
    package(
        &mut t,
        &sandbox,
        &[
            Some(json!({"transaction": " h "})),
            Some(json!({"transactions": ["i", "h", "h"]})),
            Some(json!({"transactions": ["k"]})),
            Some(json!({"source_funds_case": "case"})),
            Some(json!({"source_funds_case": " case ", "include_edit_history": true})),
            Some(json!("x")),
            Some(json!([])),
            Some(json!({"transaction": 3})),
            Some(json!({"transaction": "h", "transactions": ["i"]})),
            Some(json!({"transactions": []})),
            Some(json!({"include_url_references": "no"})),
            Some(json!({"transaction": "absent"})),
            Some(json!({"source_funds_case": "tampered"})),
            Some(json!({"transaction": "i"})),
        ],
    );
    sandbox.sql(NO_CONTEXT);
    package(
        &mut t,
        &sandbox,
        &[Some(json!({})), Some(json!({"transaction": "h"}))],
    );
    t.finish();
}
