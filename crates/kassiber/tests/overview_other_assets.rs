//! The overview's other crypto assets end to end through the binary on a
//! disposable data root (docs/plan/24-crypto-assets.md): imports into two
//! crypto-scope wallets, journal processing, then `ui.overview.snapshot`
//! from the daemon. Nothing is inserted by hand.

use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
    command
        .arg("--data-root")
        .arg(root)
        .arg("--machine")
        .args(args)
        .env("KASSIBER_NO_EGRESS", "1")
        .env_remove("KASSIBER_TEST_NO_EGRESS")
        .env_remove("KASSIBER_RUST_TEST_FAKE_STORE")
        .env("HOME", root)
        .env("XDG_DATA_HOME", root.join("data-home"))
        .env("XDG_CONFIG_HOME", root.join("config-home"));
    command
}

fn kassiber(root: &Path, args: &[&str]) -> Value {
    let output = command(root, args).output().expect("the binary runs");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str::<Value>(&stdout).unwrap()["data"].clone()
}

/// The daemon's answer to one request for the current book.
fn daemon(root: &Path, kind: &str) -> Value {
    let mut child = command(root, &["daemon"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the daemon starts");
    writeln!(
        child.stdin.take().unwrap(),
        r#"{{"request_id": "1", "kind": "{kind}", "args": {{}}}}"#
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|record| record["request_id"] == "1")
        .unwrap_or_else(|| {
            panic!(
                "{kind}: {stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            )
        })
}

const HEADER: &str = "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Trade-Group,Comment,Date,Tx-ID\n";

/// A priced 1 ETH buy in wallet A, which journals book, and an unpriced
/// 2 ETH deposit in wallet B, which they hold for a price: the overview
/// counts all 3 ETH in both wallets, and the basis covers only the 1 ETH.
#[test]
fn the_overview_counts_unpriced_holdings_beside_booked_ones() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    let book = ["--workspace", "Main", "--profile", "Book"];
    let in_book = |args: &[&str]| {
        let mut all = args.to_vec();
        all.extend(book);
        kassiber(root, &all)
    };
    kassiber(root, &["init"]);
    kassiber(root, &["workspaces", "create", "Main"]);
    kassiber(
        root,
        &[
            "profiles",
            "create",
            "Book",
            "--workspace",
            "Main",
            "--fiat-currency",
            "EUR",
        ],
    );
    for (wallet, rows) in [
        (
            "A",
            "Trade,1,ETH,2000,EUR,,,Kraken,,,2024-01-10 10:00:00,buy-1\n",
        ),
        (
            "B",
            "Deposit,2,ETH,,,,,Kraken,,,2024-02-10 10:00:00,dep-1\n",
        ),
    ] {
        in_book(&[
            "wallets",
            "create",
            "--label",
            wallet,
            "--kind",
            "cointracking",
            "--config",
            r#"{"asset_scope": "crypto"}"#,
        ]);
        let file = root.join(format!("{wallet}.csv"));
        std::fs::write(&file, format!("{HEADER}{rows}")).unwrap();
        let imported = in_book(&[
            "wallets",
            "import-cointracking",
            "--wallet",
            wallet,
            "--file",
            file.to_str().unwrap(),
        ]);
        assert_eq!(imported["imported"], 1, "{wallet}: {imported}");
    }
    let processed = in_book(&["journals", "process"]);
    // The buy is booked; the unpriced deposit waits for its price.
    assert_eq!(processed["entries_created"], 1, "{processed}");
    assert_eq!(processed["quarantined"], 1, "{processed}");

    kassiber(
        root,
        &["context", "set", "--workspace", "Main", "--profile", "Book"],
    );
    let snapshot = daemon(root, "ui.overview.snapshot");
    assert_eq!(snapshot["kind"], "ui.overview.snapshot", "{snapshot}");
    let others = snapshot["data"]["otherAssets"].as_array().unwrap();
    assert_eq!(others.len(), 1, "{others:?}");
    let eth = &others[0];
    assert_eq!(eth["asset"], "ETH");
    assert_eq!(eth["quantity"], "3");
    assert_eq!(eth["quantityUnits"], 300_000_000_000_i64);
    assert_eq!(eth["walletCount"], 2);
    assert_eq!(eth["source"], "books");
    assert_eq!(eth["costBasis"], 2000.0);
    assert_eq!(eth["basisQuantity"], "1");
    assert_eq!(eth["basisComplete"], false);
    // The book's own price values all 3 ETH; with a partial basis there is
    // no unrealized result.
    assert_eq!(eth["marketValue"], 6000.0);
    assert_eq!(eth["unrealized"], Value::Null);
    // Bitcoin totals never include ETH.
    assert_eq!(
        snapshot["data"]["balanceSummary"]["totalBtc"],
        Value::from(0.0),
        "{snapshot}"
    );
}
