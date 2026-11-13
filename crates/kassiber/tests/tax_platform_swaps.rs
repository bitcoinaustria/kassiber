//! A CoinTracking export with a BTC -> ETH trade, end to end through the
//! binary on a disposable data root (docs/plan/23-crypto-assets.md). In an
//! Austrian book the import pairs the swap; once its legs are priced the swap
//! books no gain and carries the BTC basis into ETH.

use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn kassiber(root: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_kassiber"))
        .arg("--data-root")
        .arg(root)
        .arg("--machine")
        .args(args)
        .env("KASSIBER_NO_EGRESS", "1")
        .env_remove("KASSIBER_TEST_NO_EGRESS")
        .env_remove("KASSIBER_RUST_TEST_FAKE_STORE")
        .env("HOME", root)
        .env("XDG_DATA_HOME", root.join("data-home"))
        .env("XDG_CONFIG_HOME", root.join("config-home"))
        .output()
        .expect("the binary runs");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str::<Value>(&stdout).unwrap()["data"].clone()
}

#[test]
fn an_austrian_cointracking_swap_carries_btc_basis_into_eth() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    let book = ["--workspace", "Main", "--profile", "Book"];
    let run = |args: &[&str]| kassiber(root, args);
    let in_book = |args: &[&str]| {
        let mut all = args.to_vec();
        all.extend(book);
        kassiber(root, &all)
    };
    run(&["init"]);
    run(&["workspaces", "create", "Main"]);
    run(&[
        "profiles",
        "create",
        "Book",
        "--workspace",
        "Main",
        "--fiat-currency",
        "EUR",
        "--tax-country",
        "at",
    ]);
    in_book(&[
        "wallets",
        "create",
        "--label",
        "CT",
        "--kind",
        "cointracking",
        "--config",
        r#"{"asset_scope": "crypto"}"#,
    ]);
    let file = root.join("cointracking.csv");
    std::fs::write(
        &file,
        "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Trade-Group,Comment,Date,Tx-ID\n\
         Trade,0.1,BTC,3000,EUR,,,Kraken,,,2024-01-10 10:00:00,buy-1\n\
         Trade,1.5,ETH,0.05,BTC,,,Kraken,,,2025-03-01 10:00:00,swap-1\n",
    )
    .unwrap();
    let imported = in_book(&[
        "wallets",
        "import-cointracking",
        "--wallet",
        "CT",
        "--file",
        file.to_str().unwrap(),
    ]);
    assert_eq!(imported["imported"], 3);
    assert_eq!(imported["swap_pairs"], 1);

    // The export names no value currency, so the legs are unpriced and the
    // Austrian carry waits for their prices instead of guessing.
    let processed = in_book(&["journals", "process"]);
    assert_eq!(processed["quarantined"], 2);
    let held = in_book(&["journals", "quarantined"]);
    let held = held.as_array().unwrap();
    assert_eq!(held.len(), 2);
    for row in held {
        assert_eq!(row["reason"], "at_swap_basis_carry_unresolved");
        assert_eq!(row["detail"]["reason_code"], "missing_spot_price");
        in_book(&[
            "journals",
            "quarantine",
            "resolve",
            "price-override",
            "--transaction",
            row["transaction_id"].as_str().unwrap(),
            "--fiat-value",
            "4000",
        ]);
    }

    let processed = in_book(&["journals", "process"]);
    assert_eq!(processed["quarantined"], 0);
    let entries = in_book(&["journals", "list"]);
    let entry = |kind: &str, asset: &str| {
        entries
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entry_type"] == kind && e["asset"] == asset)
            .unwrap_or_else(|| panic!("{kind} {asset}: {entries}"))
            .clone()
    };
    // The swap is tax-neutral: the BTC leaves at its basis, no gain.
    let out = entry("disposal", "BTC");
    assert_eq!(out["at_category"], "neu_swap");
    assert_eq!(out["gain_loss"], 0.0);
    assert_eq!(out["cost_basis"], out["proceeds"]);
    // ETH arrives with half the 3000 EUR basis of the 0.1 BTC bought.
    let into = entry("acquisition", "ETH");
    assert_eq!(into["quantity"], 1.5);
    assert_eq!(into["fiat_value"], 1500.0);
}
