//! No product transaction opener begins deferred unless it is a named
//! read-only snapshot or a one-time open migration.
//!
//! A deferred transaction that reads and then writes is refused SQLITE_BUSY
//! at once when another connection holds the writer (SQLite skips the busy
//! handler on that upgrade), so write transactions open with
//! `kassiber_store::begin_write_savepoint`, `write_savepoint`,
//! `write_transaction`, `BEGIN IMMEDIATE` or `TransactionBehavior::Immediate`.
//! The Python counterpart is tests/test_write_transaction_locks.py; the
//! inventory is docs/reference/rust-write-transactions.md.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Openers that may begin deferred, by (file, enclosing function). Read-only
/// snapshots never upgrade, so they cannot hit the refused upgrade. Open
/// migrations run while `Book::open_migrating`/`create` opens the file,
/// before the daemon starts its workers, and are read-only once applied.
const DEFERRED_ALLOWED: &[(&str, &str, &str)] = &[
    (
        "crates/journals/src/reproduce.rs",
        "bundle_source",
        "read-only: loads the replay bundle's snapshot in one consistent read and rolls back",
    ),
    (
        "crates/replay/src/lib.rs",
        "export",
        "read-only: one consistent snapshot for the bundle's source, stored figures and local check, rolled back",
    ),
    (
        "crates/store/src/write_lock.rs",
        "begin_write_savepoint",
        "the helper itself",
    ),
    (
        "crates/store/src/write_lock.rs",
        "write_savepoint",
        "the helper itself",
    ),
    (
        "crates/store/src/write_lock.rs",
        "write_transaction",
        "the helper itself; deferred only when the connection cannot write",
    ),
    (
        "crates/store/src/lib.rs",
        "open_with_policy",
        "read-only snapshot: the schema gate, rolled back",
    ),
    (
        "crates/store/src/lib.rs",
        "with_savepoint",
        "nested in Book::transaction's BEGIN IMMEDIATE",
    ),
    (
        "crates/store/src/observer.rs",
        "snapshot_with",
        "read-only snapshot",
    ),
    (
        "crates/store/src/compat.rs",
        "install_graph",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_ordered.rs",
        "transaction_page_index",
        "open migration, read-only once applied",
    ),
    (
        "crates/store/src/compat_ordered.rs",
        "import_batch_sequence",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_ownership.rs",
        "instance_id",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_ownership.rs",
        "inline_history",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_liquid.rs",
        "liquid",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_backfill.rs",
        "payment_hash",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_backfill.rs",
        "swap_fees",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_backfill.rs",
        "bip329",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_backfill.rs",
        "inbox",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_backfill.rs",
        "gap_inputs",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_backfill.rs",
        "swap_schema",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_custody.rs",
        "relation_schema",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_custody.rs",
        "review_relations",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_custody.rs",
        "evidence",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_custody.rs",
        "durable_evidence",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_schema.rs",
        "migrate_recipient_unique",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_schema.rs",
        "projection_shapes",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_schema.rs",
        "economic_notes",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_schema.rs",
        "immutability",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_schema.rs",
        "commercial",
        "one-time open migration",
    ),
    (
        "crates/store/src/compat_schema.rs",
        "step",
        "one-time open migration",
    ),
    (
        "crates/store/src/retired.rs",
        "retire_ledger_with_observer",
        "one-time open migration",
    ),
    (
        "crates/setup/src/lib.rs",
        "execute",
        "deferred only for reads_only commands; debug builds assert they wrote nothing",
    ),
    (
        "crates/setup/src/backends.rs",
        "execute",
        "deferred only for reads_only operations; debug builds assert they wrote nothing",
    ),
    (
        "crates/setup/src/networks.rs",
        "read_snapshot",
        "read-only snapshot, rolled back",
    ),
    (
        "crates/journals/src/review_workflow.rs",
        "run",
        "private in-memory clones of the book (before and after)",
    ),
    (
        "crates/journals/src/review_workflow.rs",
        "inspect_effects",
        "read-only snapshot",
    ),
    (
        "crates/journals/src/review_workflow.rs",
        "request_input",
        "read-only snapshot",
    ),
    (
        "crates/transfers/src/review_cases.rs",
        "inspect_with_error",
        "read-only snapshot",
    ),
    (
        "crates/reports/src/lib.rs",
        "execute_with_sources",
        "read-only snapshot, rolled back",
    ),
    (
        "crates/reports/src/artifact_xlsx.rs",
        "prepare",
        "read-only snapshot, rolled back",
    ),
    (
        "crates/reports/src/artifact_csv.rs",
        "prepare",
        "read-only snapshot, rolled back",
    ),
    (
        "crates/reports/src/export_preview.rs",
        "preview",
        "read-only snapshot, rolled back",
    ),
    (
        "crates/reports/src/psbt_privacy.rs",
        "report_with_baseline",
        "read-only snapshot",
    ),
    (
        "crates/reports/src/source_funds_review.rs",
        "review_context",
        "read-only snapshot, rolled back",
    ),
    (
        "crates/wallets/src/analysis_index/views.rs",
        "with_read",
        "read-only snapshot",
    ),
    (
        "crates/wallets/src/analysis_index/sources.rs",
        "build_index_at",
        "read-only snapshot",
    ),
    (
        "crates/custody/src/confirmation/liquid.rs",
        "decodable",
        "not a database transaction: the Liquid decoder",
    ),
];

/// One opener found in product code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Opener {
    file: String,
    function: String,
    opener: String,
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn product_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(name.as_str(), "tests" | "examples" | "benches" | "target") {
                product_sources(&path, out);
            }
        } else if name.ends_with(".rs")
            && name != "tests.rs"
            && !name.ends_with("_tests.rs")
            && name != "fixture.rs"
        {
            out.push(path);
        }
    }
}

/// The SQL statements of a string literal that open a transaction (product
/// SQL spells keywords in capitals; a lower-case "begin" is a stage name).
fn literal_openers(literal: &str) -> Vec<String> {
    literal
        .split(';')
        .filter_map(|statement| {
            let words: Vec<_> = statement.split_whitespace().collect();
            match words.first().copied() {
                Some("SAVEPOINT") => Some("SAVEPOINT".to_owned()),
                Some("BEGIN") => Some(match words.get(1).copied() {
                    Some(kind @ ("IMMEDIATE" | "EXCLUSIVE" | "DEFERRED")) => {
                        format!("BEGIN {kind}")
                    }
                    _ => "BEGIN".to_owned(),
                }),
                _ => None,
            }
        })
        .collect()
}

const CALLS: &[(&str, &str)] = &[
    (".transaction()", "transaction()"),
    (".unchecked_transaction()", "unchecked_transaction()"),
    (".savepoint()", "savepoint()"),
    ("savepoint_with_name(", "savepoint_with_name()"),
    (
        "TransactionBehavior::Deferred",
        "TransactionBehavior::Deferred",
    ),
    (
        "TransactionBehavior::Immediate",
        "TransactionBehavior::Immediate",
    ),
    (
        "TransactionBehavior::Exclusive",
        "TransactionBehavior::Exclusive",
    ),
];

/// Lex one file: string literals and code outside comments, the enclosing
/// `fn` of each position, and `#[cfg(test)]` items skipped.
fn scan(file: &str, text: &str) -> Vec<Opener> {
    let bytes = text.as_bytes();
    let mut found: Vec<(String, String)> = Vec::new();
    let mut functions: Vec<(String, usize)> = Vec::new();
    let mut pending_fn: Option<String> = None;
    let mut depth = 0usize;
    let mut skip_pending = false;
    let mut skip_until: Option<usize> = None;
    let mut i = 0;
    let current = |functions: &Vec<(String, usize)>| {
        functions
            .last()
            .map(|f| f.0.clone())
            .unwrap_or_else(|| "<module>".to_owned())
    };
    while i < bytes.len() {
        let rest = &text[i..];
        let skipping = skip_until.is_some();
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            i += rest.find("*/").map_or(rest.len(), |end| end + 2);
            continue;
        }
        // Raw strings r"..", r#".."#, br#".."#.
        let raw = rest.strip_prefix("br").or_else(|| rest.strip_prefix('r'));
        if let Some(after) = raw.filter(|after| {
            let hashes = after.len() - after.trim_start_matches('#').len();
            after[hashes..].starts_with('"')
                && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        }) {
            let hashes = after.len() - after.trim_start_matches('#').len();
            let start = rest.len() - after.len() + hashes + 1;
            let close = format!("\"{}", "#".repeat(hashes));
            let end = rest[start..].find(&close).unwrap() + start;
            if !skipping {
                for opener in literal_openers(&rest[start..end]) {
                    found.push((current(&functions), opener));
                }
            }
            i += end + close.len();
            continue;
        }
        let c = bytes[i];
        if c == b'"' {
            let mut j = i + 1;
            let mut literal = String::new();
            while bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    match bytes[j + 1] {
                        b'n' => literal.push('\n'),
                        b'\n' => {
                            j += 2;
                            while bytes[j].is_ascii_whitespace() {
                                j += 1;
                            }
                            continue;
                        }
                        other => literal.push(other as char),
                    }
                    j += 2;
                } else {
                    let ch = text[j..].chars().next().unwrap();
                    literal.push(ch);
                    j += ch.len_utf8();
                }
            }
            if !skipping {
                for opener in literal_openers(&literal) {
                    found.push((current(&functions), opener));
                }
            }
            i = j + 1;
            continue;
        }
        if c == b'\'' {
            // A char literal ('x', '\n', '\''), else a lifetime.
            if bytes.get(i + 1) == Some(&b'\\') {
                i += text[i + 2..].find('\'').unwrap() + 3;
                continue;
            }
            let ch = text[i + 1..].chars().next().unwrap();
            if text[i + 1 + ch.len_utf8()..].starts_with('\'') {
                i += 2 + ch.len_utf8();
                continue;
            }
            i += 1;
            continue;
        }
        if rest.starts_with("#[cfg(test)]") {
            skip_pending = skip_until.is_none();
            i += "#[cfg(test)]".len();
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let end = rest
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(rest.len());
            let word = &rest[..end];
            if word == "fn" {
                let after = rest[end..].trim_start();
                let name_end = after
                    .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                    .unwrap_or(after.len());
                if name_end > 0 {
                    pending_fn = Some(after[..name_end].to_owned());
                }
            }
            if !skipping {
                for (needle, opener) in CALLS {
                    if needle.starts_with(|ch: char| ch.is_ascii_alphabetic())
                        && rest.starts_with(needle)
                    {
                        found.push((current(&functions), (*opener).to_owned()));
                    }
                }
            }
            i += end;
            continue;
        }
        if c == b'.' && !skipping {
            for (needle, opener) in CALLS {
                if needle.starts_with('.') && rest.starts_with(needle) {
                    found.push((current(&functions), (*opener).to_owned()));
                }
            }
        }
        match c {
            b'{' => {
                if skip_pending {
                    skip_pending = false;
                    skip_until = Some(depth);
                }
                if let Some(name) = pending_fn.take() {
                    functions.push((name, depth));
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if functions.last().is_some_and(|f| f.1 == depth) {
                    functions.pop();
                }
                if skip_until == Some(depth) {
                    skip_until = None;
                }
            }
            b';' => {
                pending_fn = None;
                if skip_pending {
                    skip_pending = false;
                }
            }
            _ => {}
        }
        i += 1;
    }
    // A savepoint behind this function's own BEGIN IMMEDIATE (when no caller
    // transaction is open) already holds the writer.
    let immediate: BTreeSet<String> = found
        .iter()
        .filter(|(_, opener)| {
            matches!(
                opener.as_str(),
                "BEGIN IMMEDIATE"
                    | "BEGIN EXCLUSIVE"
                    | "TransactionBehavior::Immediate"
                    | "TransactionBehavior::Exclusive"
            )
        })
        .map(|(function, _)| function.clone())
        .collect();
    found
        .into_iter()
        .filter(|(function, opener)| {
            !opener.starts_with("TransactionBehavior::Immediate")
                && !opener.starts_with("TransactionBehavior::Exclusive")
                && opener != "BEGIN IMMEDIATE"
                && opener != "BEGIN EXCLUSIVE"
                && !(opener == "SAVEPOINT" && immediate.contains(function))
        })
        .map(|(function, opener)| Opener {
            file: file.to_owned(),
            function,
            opener,
        })
        .collect()
}

fn deferred_openers() -> Vec<Opener> {
    let root = workspace();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            product_sources(&src, &mut files);
        }
    }
    files.sort();
    let mut found = Vec::new();
    for path in files {
        let file = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        found.extend(scan(&file, &std::fs::read_to_string(&path).unwrap()));
    }
    found
}

fn allowed() -> BTreeMap<(&'static str, &'static str), &'static str> {
    DEFERRED_ALLOWED
        .iter()
        .map(|(file, function, reason)| ((*file, *function), *reason))
        .collect()
}

/// Hand-written `ROLLBACK TO` statements per product file, as they stood when
/// `kassiber_store::WriteLock` arrived. A write held by a `WriteLock` rolls
/// back on every early return and commits once; a hand-rolled savepoint has
/// to repeat that on each path, and a missed path commits a partial write.
/// The counts may only fall: lower an entry when its file moves to WriteLock.
const HAND_ROLLED_ROLLBACKS: &[(&str, usize)] = &[
    ("crates/imports/src/lib.rs", 1),
    ("crates/journals/src/persistence.rs", 1),
    ("crates/journals/src/prebuild.rs", 1),
    ("crates/journals/src/review_apply.rs", 1),
    ("crates/journals/src/review_tax.rs", 1),
    ("crates/kassiber/src/real_store.rs", 1),
    ("crates/rates/src/lib.rs", 1),
    ("crates/reports/src/artifact_csv.rs", 1),
    ("crates/reports/src/artifact_xlsx.rs", 1),
    ("crates/reports/src/austrian_xlsx.rs", 1),
    ("crates/reports/src/cross_check.rs", 1),
    ("crates/reports/src/exit_tax.rs", 1),
    ("crates/reports/src/export.rs", 2),
    ("crates/reports/src/export_preview.rs", 1),
    ("crates/reports/src/full_xlsx.rs", 1),
    ("crates/reports/src/lib.rs", 1),
    ("crates/reports/src/platform_csv.rs", 1),
    ("crates/reports/src/privacy_mirror.rs", 1),
    ("crates/reports/src/snapshot_write.rs", 3),
    ("crates/reports/src/source_funds_batch.rs", 1),
    ("crates/reports/src/source_funds_cases.rs", 1),
    ("crates/reports/src/source_funds_suggest.rs", 1),
    ("crates/reports/src/year_handoff_xlsx.rs", 3),
    ("crates/setup/src/book_reset.rs", 2),
    ("crates/setup/src/book_settings.rs", 1),
    ("crates/setup/src/networks.rs", 1),
    ("crates/transactions/src/sql.rs", 2),
    ("crates/transfers/src/legacy_refresh.rs", 1),
    ("crates/transfers/src/pairing_commands.rs", 1),
    ("crates/transfers/src/sql.rs", 2),
    ("crates/wallets/src/analysis_index/reads.rs", 1),
    ("crates/wallets/src/analysis_ops.rs", 2),
    ("crates/wallets/src/btcpay/provenance.rs", 1),
    ("crates/wallets/src/btcpay/setup.rs", 1),
    ("crates/wallets/src/file_import/attachment.rs", 1),
    ("crates/wallets/src/file_import/document/coordinator.rs", 1),
    ("crates/wallets/src/file_import/document_chat.rs", 2),
    ("crates/wallets/src/file_import/exchange_api.rs", 1),
    ("crates/wallets/src/file_import/samourai.rs", 1),
    ("crates/wallets/src/lifecycle.rs", 1),
    ("crates/wallets/src/sync/btcpay.rs", 1),
];

fn hand_rolled_rollbacks() -> BTreeMap<String, usize> {
    let root = workspace();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            product_sources(&src, &mut files);
        }
    }
    files
        .into_iter()
        .filter_map(|path| {
            let file = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let count = std::fs::read_to_string(&path)
                .unwrap()
                .matches("ROLLBACK TO")
                .count();
            (count > 0 && file != "crates/store/src/write_lock.rs").then_some((file, count))
        })
        .collect()
}

#[test]
fn new_write_savepoints_roll_back_through_write_lock() {
    let baseline: BTreeMap<String, usize> = HAND_ROLLED_ROLLBACKS
        .iter()
        .map(|(file, count)| ((*file).to_owned(), *count))
        .collect();
    let live = hand_rolled_rollbacks();
    let grown: Vec<_> = live
        .iter()
        .filter(|(file, count)| baseline.get(*file).is_none_or(|b| *count > b))
        .collect();
    assert!(
        grown.is_empty(),
        "hold the write with kassiber_store::WriteLock::begin (it rolls back when dropped and \
         commits once with commit()) instead of a hand-written ROLLBACK TO: {grown:?}"
    );
    let shrunk: Vec<_> = baseline
        .iter()
        .filter(|(file, count)| live.get(*file).is_none_or(|l| l < *count))
        .collect();
    assert!(
        shrunk.is_empty(),
        "lower these HAND_ROLLED_ROLLBACKS entries to their current counts: {shrunk:?}"
    );
}

#[test]
fn writes_never_begin_deferred() {
    let allowed = allowed();
    let unexpected: BTreeSet<_> = deferred_openers()
        .into_iter()
        .filter(|o| !allowed.contains_key(&(o.file.as_str(), o.function.as_str())))
        .collect();
    assert!(
        unexpected.is_empty(),
        "use kassiber_store::begin_write_savepoint, write_savepoint, write_transaction or BEGIN IMMEDIATE for a write transaction: {unexpected:#?}"
    );
}

#[test]
fn the_allowlist_names_live_openers() {
    let live: BTreeSet<(String, String)> = deferred_openers()
        .into_iter()
        .map(|o| (o.file, o.function))
        .collect();
    let stale: Vec<_> = allowed()
        .into_keys()
        .filter(|(file, function)| !live.contains(&((*file).to_owned(), (*function).to_owned())))
        .collect();
    assert!(
        stale.is_empty(),
        "allowlisted openers no longer exist: {stale:?}"
    );
}

#[test]
fn the_scanner_sees_each_opener_form() {
    let source = r##"
fn deferred(conn: &Connection) {
    conn.execute_batch("SAVEPOINT a").unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN").unwrap();
    conn.execute_batch(&format!("SAVEPOINT {name}")).unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    let sp = conn.savepoint().unwrap();
    let t = conn.transaction().unwrap();
    conn.execute_batch("CREATE TRIGGER t BEFORE INSERT ON x BEGIN SELECT 1; END").unwrap();
    conn.execute_batch("RELEASE a; ROLLBACK TO a").unwrap();
    // conn.execute_batch("SAVEPOINT commented")
    stage("begin");
    let _ = '"';
}
fn immediate(conn: &Connection) {
    if conn.is_autocommit() { conn.execute_batch("BEGIN IMMEDIATE").unwrap(); }
    conn.execute_batch("SAVEPOINT behind_immediate").unwrap();
    conn.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();
    kassiber_store::begin_write_savepoint(conn, "reserved").unwrap();
}
#[cfg(test)]
mod tests {
    fn fixture(conn: &Connection) { conn.execute_batch("BEGIN").unwrap(); }
}
fn after_tests(conn: &Connection) { conn.execute_batch(r#"BEGIN DEFERRED"#).unwrap(); }
"##;
    let found: Vec<_> = scan("x.rs", source)
        .into_iter()
        .map(|o| format!("{} {}", o.function, o.opener))
        .collect();
    assert_eq!(
        found,
        [
            "deferred SAVEPOINT",
            "deferred BEGIN",
            "deferred SAVEPOINT",
            "deferred unchecked_transaction()",
            "deferred savepoint()",
            "deferred transaction()",
            "after_tests BEGIN DEFERRED",
        ]
    );
}
