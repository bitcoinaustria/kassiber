//! Wallet completeness classes and scan bounds (`ui.wallets.list`,
//! `wallets list`) and `transactions verify` against stored chain graphs.
//! Replaces the kill-holding tests of `tests/test_rust_wallet_completeness.py`.

use serde_json::{json, Value};

use crate::desktop::{self, compare, graph, some, verify, NO_CONTEXT, WALLETS};
use crate::support::{Sandbox, Transcript};

const ADDR: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
const SCRIPT: &str = "0014c0cebcd6c3d3ca8c75dc5ec62ebe55330ef910e2";
const OTHER: &str = "00143e34985dca6fddc9fb369940e4c7d8e2873f529c";
const TX1: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const TX2: &str = "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";

/// One CLI read: `--machine`, or `--format fmt`; no table may change.
fn cli(t: &mut Transcript, sandbox: &Sandbox, fmt: Option<&str>, argv: &[&str]) -> Option<Value> {
    let before = sandbox.dump();
    let mut args = match fmt {
        Some(fmt) => vec!["--format", fmt],
        None => vec!["--machine"],
    };
    args.extend_from_slice(argv);
    let out = t.run(sandbox, &args);
    assert_eq!(sandbox.dump(), before, "{argv:?} changed the book");
    fmt.is_none().then(|| out.json())
}

#[test]
fn test_classes_and_scan_bounds_match_wire_bytes() {
    let sandbox = desktop::book();
    sandbox.sql(WALLETS);
    let mut t = Transcript::new("wallet_completeness/classes_and_scan_bounds");
    let responses = compare(
        &mut t,
        &sandbox,
        "ui.wallets.list",
        &[None, Some(json!(null)), Some(json!({}))],
    );
    let wallets = responses[0]["data"]["wallets"].as_array().unwrap();
    let mut classes: Vec<&str> = wallets
        .iter()
        .map(|w| w["completeness_class"].as_str().unwrap())
        .collect();
    classes.sort_unstable();
    classes.dedup();
    assert_eq!(
        classes,
        [
            "address-list",
            "descriptor-derived",
            "exchange-export",
            "file-import",
            "lightning-node",
            "manual",
            "provider-api",
            "untracked"
        ]
    );
    let bounds = |id: &str| wallets.iter().find(|w| w["id"] == id).unwrap()["scan_bounds"].clone();
    assert_eq!(bounds("desc")["deficit_assets"], json!(["BTC"]));
    assert!(bounds("addr")["warnings"]
        .as_array()
        .unwrap()
        .contains(&json!("history_starts_after_first_funding")));
    assert_eq!(
        bounds("sp")["history_before_first_transaction"],
        "declared_start"
    );
    assert_eq!(
        bounds("desc-new")["warnings"],
        json!(["never_synced", "no_transactions"])
    );
    sandbox.sql("UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p';");
    compare(&mut t, &sandbox, "ui.wallets.list", &some(vec![json!({})]));
    sandbox.sql("DELETE FROM settings WHERE key='context_profile';");
    compare(&mut t, &sandbox, "ui.wallets.list", &some(vec![json!({})]));
    t.finish();
}

#[test]
fn test_cli_list_matches_every_format() {
    let sandbox = desktop::book();
    sandbox.sql(WALLETS);
    let mut t = Transcript::new("wallet_completeness/cli_list_every_format");
    // The seed's undecodable configuration fails the CLI.
    let out = cli(&mut t, &sandbox, None, &["wallets", "list"]).unwrap();
    assert_eq!(out["error"]["code"], "internal_error");
    sandbox.sql("UPDATE wallets SET config_json='{}' WHERE id='custom';");
    let out = cli(&mut t, &sandbox, None, &["wallets", "list"]).unwrap();
    assert_eq!(out["error"]["code"], "internal_error");
    for raw in ["null", "3", "\"x\""] {
        sandbox.sql(&format!(
            "UPDATE wallets SET config_json='{raw}' WHERE id='phoenix';"
        ));
        cli(&mut t, &sandbox, None, &["wallets", "list"]);
    }
    sandbox.sql("UPDATE wallets SET config_json='{}' WHERE id='phoenix';");
    let rows = cli(&mut t, &sandbox, None, &["wallets", "list"]).unwrap();
    assert!(rows["data"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row.get("scan_bounds").is_some()));
    for fmt in ["json", "table", "plain", "csv"] {
        cli(&mut t, &sandbox, Some(fmt), &["wallets", "list"]);
    }
    cli(
        &mut t,
        &sandbox,
        None,
        &["wallets", "list", "--profile", "Other"],
    );
    cli(
        &mut t,
        &sandbox,
        None,
        &["wallets", "list", "--profile", "absent"],
    );
    t.finish();
}

fn verify_all(t: &mut Transcript, sandbox: &Sandbox, refs: &[&str]) -> Vec<Value> {
    refs.iter()
        .map(|r| {
            cli(
                t,
                sandbox,
                None,
                &["transactions", "verify", "--transaction", r],
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn test_refusals_and_graphless_rows_match_bytes() {
    let sandbox = desktop::book();
    sandbox.sql(&graph());
    let mut t = Transcript::new("wallet_completeness/verify_refusals_and_graphless_rows");
    verify_all(
        &mut t,
        &sandbox,
        &["", " ", "absent", "g4", "g5", "h", "k", "a"],
    );
    cli(
        &mut t,
        &sandbox,
        None,
        &[
            "transactions",
            "verify",
            "--transaction",
            "g1",
            "--profile",
            "absent",
        ],
    );
    cli(&mut t, &sandbox, None, &["transactions", "verify"]);
    sandbox.sql(NO_CONTEXT);
    verify_all(&mut t, &sandbox, &["g1"]);
    t.finish();
}

#[test]
fn test_stored_graphs_match_bytes() {
    let sandbox = desktop::book();
    sandbox.sql(&verify());
    let mut t = Transcript::new("wallet_completeness/verify_stored_graphs");
    let ok_txid = "1".repeat(64);
    let mut refs = vec!["g1", TX1, TX2, "g3", "g6", " g2 "];
    let numbered: Vec<String> = (1..=8).map(|n| format!("v{n}")).collect();
    refs.extend(numbered.iter().map(String::as_str));
    refs.push(&ok_txid);
    let results = verify_all(&mut t, &sandbox, &refs);
    let mut statuses: Vec<&str> = results
        .iter()
        .filter(|r| r["kind"] != "error")
        .map(|r| r["data"]["status"].as_str().unwrap())
        .collect();
    statuses.sort_unstable();
    statuses.dedup();
    assert_eq!(statuses, ["mismatch", "ok", "partial"]);
    let ok = &results[6]["data"];
    assert_eq!(ok["conservation"]["status"], "ok");
    assert_eq!(ok["network_contacted"], false);
    let rendered = serde_json::to_string(&results).unwrap();
    for secret in [ADDR, SCRIPT, OTHER] {
        assert!(!rendered.contains(secret));
    }
    for fmt in ["table", "plain"] {
        cli(
            &mut t,
            &sandbox,
            Some(fmt),
            &["transactions", "verify", "--transaction", "v1"],
        );
    }
    t.finish();
}
