use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::*;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Request wrappers whose kind is the first string literal argument.
const SENDERS: &[&str] = &[
    "send",
    "send_job",
    "mutate",
    "send_t",
    "send_tok",
    "run",
    "request",
    "send_read",
];

/// Kinds the app's code sends that ALLOWED_DAEMON_KINDS lacks, with the reason.
const KNOWN_UNLISTED: &[(&str, &str)] = &[(
    "ui.connections.bullbitcoin_wallet.create",
    "the Bull Bitcoin wallet dialog sends it but it is not allowlisted, so the app refuses it with kind_not_allowed",
)];

fn looks_like_kind(s: &str) -> bool {
    (s == "status" || ["ui.", "ai.", "daemon."].iter().any(|p| s.starts_with(p)))
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
        && !s.ends_with('.')
}

/// The identifier of the innermost call `pos` is an argument of, on this line.
fn caller(line: &str, pos: usize) -> Option<&str> {
    let mut depth = 0i32;
    for (i, c) in line[..pos].char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => {
                let before = &line[..i];
                let start = before
                    .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .map(|j| j + 1)
                    .unwrap_or(0);
                return Some(&before[start..]);
            }
            '(' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn sent_kinds(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sent_kinds(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs")
            || path.ends_with("tests.rs")
            || path.ends_with("kinds.rs")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let quotes: Vec<usize> = line.match_indices('"').map(|(i, _)| i).collect();
            for pair in quotes.chunks(2) {
                let [a, b] = pair else { continue };
                let lit = &line[a + 1..*b];
                if !looks_like_kind(lit) {
                    continue;
                }
                // Only the first string argument of a request wrapper is a kind.
                let Some(f) = caller(line, *a) else { continue };
                let open = line[..*a].rfind('(').unwrap();
                if SENDERS.contains(&f) && !line[open..*a].contains('"') {
                    out.push((format!("{}:{}", path.display(), n + 1), lit.to_owned()));
                }
            }
        }
    }
}

#[test]
fn every_kind_the_app_sends_is_allowed() {
    let mut found = Vec::new();
    sent_kinds(&manifest().join("src"), &mut found);
    assert!(
        found.len() > 150,
        "the scanner found only {} send sites",
        found.len()
    );
    let unknown: BTreeSet<String> = found
        .iter()
        .filter(|(_, k)| {
            !ALLOWED_DAEMON_KINDS.contains(&k.as_str())
                && !KNOWN_UNLISTED.iter().any(|(u, _)| u == k)
        })
        .map(|(at, k)| format!("{k} ({at})"))
        .collect();
    assert!(
        unknown.is_empty(),
        "kinds sent but not allowed: {unknown:#?}"
    );
}

#[test]
fn refusals_follow_daemon_invoke() {
    assert_eq!(
        check("ui.shell.exec", true).unwrap_err().code,
        "kind_not_allowed"
    );
    assert_eq!(check("ui.overview.snapshot", false), Ok(()));
    assert_eq!(
        check("ai.chat", false).unwrap_err().code,
        "ai_features_disabled"
    );
    assert_eq!(check("ai.chat", true), Ok(()));
    // Providers stay configurable while AI is off.
    assert_eq!(check("ai.providers.list", false), Ok(()));
    // Nothing on the locked list is outside the allowlist.
    for k in ALLOWED_WHILE_LOCKED {
        assert!(ALLOWED_DAEMON_KINDS.contains(k), "{k}");
    }
    assert!(is_streaming("ui.freshness.run") && !is_streaming("ui.overview.snapshot"));
    assert!(
        is_read("ui.transactions.list")
            && is_read("status")
            && !is_read("ui.freshness.run")
            && !is_read("ui.transactions.metadata.update")
    );
}

#[test]
fn book_bound_writes_are_told_apart_from_reads_and_machine_kinds() {
    // The writes the review named (loan, attachment, unpair) and the detail's other actions.
    for k in [
        "ui.loans.mark",
        "ui.loans.unmark",
        "ui.loans.link",
        "ui.attachments.add",
        "ui.attachments.remove",
        "ui.attachments.rename",
        "ui.transfers.unpair",
        "ui.transactions.metadata.update",
        "ui.review.apply",
        "ui.wallets.delete",
        "ui.profiles.rename",
    ] {
        assert!(is_book_mutation(k), "{k}");
    }
    // Reads, the lock's own kinds, the book switch, machine-wide settings and job cancels don't.
    for k in [
        "ui.transactions.list",
        "status",
        "daemon.unlock",
        "ui.projects.select",
        "ui.profiles.switch",
        "ui.network.offline.set",
        "ai.providers.update",
        "ui.backends.electrum.test",
        "ui.chain_analysis.jobs.cancel",
        "ui.onboarding.complete",
        "ui.secrets.init",
    ] {
        assert!(!is_book_mutation(k), "{k}");
    }
}
