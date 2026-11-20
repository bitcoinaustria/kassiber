//! A Bisq trade's escrow through the real binary: the watch-only Bisq wallet
//! observes the deposit and payout transactions, `wallets import-bisq` adds
//! the escrow ledger and pairs its legs by the txids Bisq names, and
//! `journals process` books one buy and one sell with no disposal of a
//! security deposit and no `insufficient_lots`.

use serde_json::Value;
use std::process::{Command, Stdio};

const SAMPLES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/exchanges/samples/bisq"
);

/// Run one space-separated command against `root`, returning its envelope;
/// `TRADES`, `TRANSACTIONS` and `ONCHAIN` stand for the export files.
fn cli(root: &std::path::Path, line: &str) -> Value {
    let arg = |word| match word {
        "TRADES" => format!("{SAMPLES}/tradeHistory.csv").into(),
        "TRANSACTIONS" => format!("{SAMPLES}/transactions.csv").into(),
        "ONCHAIN" => root.join("onchain.csv"),
        word => std::path::PathBuf::from(word),
    };
    let output = Command::new(env!("CARGO_BIN_EXE_kassiber"))
        .arg("--data-root")
        .arg(root.join("data"))
        .arg("--machine")
        .args(line.split(' ').map(arg))
        .env_remove("KASSIBER_TEST_NO_EGRESS")
        .env_remove("KASSIBER_NO_EGRESS")
        .env_remove("KASSIBER_RUST_TEST_FAKE_STORE")
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env(
            "KASSIBER_OFFLINE_PREFERENCE_FILE",
            root.join("offline.json"),
        )
        .stdin(Stdio::null())
        .output()
        .expect("the binary runs");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{line}: {stdout}{stderr}");
    serde_json::from_str(&stdout).unwrap()
}

#[test]
fn a_bisq_buy_and_sell_book_through_their_escrow_and_a_held_trade_stays_quarantined() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let run = |line: &str| cli(&root, line);
    run("init");
    run("workspaces create main");
    run("profiles create --workspace main --fiat-currency EUR --tax-country at book");
    // The watch-only Bisq wallet as Kassiber's native observation records
    // it: an earlier purchase, the taker fee, each trade's deposit as the
    // wallet's whole debit (share plus miner fees) with no separate fee, its
    // payout, and the mediated trade's deposit. Kassiber saw the deposits a
    // few minutes after Bisq's dates.
    let mut onchain = "txid,occurred_at,direction,asset,amount,fee,fiat_rate,kind\n".to_owned();
    for row in [
        "9f,2024-03-01T12:00:00Z,inbound,BTC,0.2,0,50000,buy",
        "e3,2024-03-15T14:31:02Z,outbound,BTC,0.000175,0.000045,60000,",
        "a1,2024-03-15T14:45:00Z,outbound,BTC,0.00605,0,60000,",
        "b1,2024-03-16T10:00:00Z,inbound,BTC,0.031,0,60500,",
        "c2,2024-04-02T09:20:00Z,outbound,BTC,0.05754,0,62000,",
        "d2,2024-04-03T08:00:00Z,inbound,BTC,0.0075,0,62500,",
        "f4,2024-05-01T11:25:00Z,outbound,BTC,0.00605,0,58000,",
    ] {
        onchain += &format!("{}{}\n", row[..2].repeat(31), row);
    }
    std::fs::write(root.join("onchain.csv"), onchain).unwrap();
    run("wallets create --label Watch --kind custom");
    run("wallets import-csv --wallet Watch --file ONCHAIN");
    let imported =
        run("wallets import-bisq --file TRADES --transactions TRANSACTIONS --timezone UTC");
    let data = &imported["data"];
    assert_eq!(data["imported"], 6);
    assert_eq!(data["bisq_pairs_created"].as_array().unwrap().len(), 4);
    assert_eq!(data["bisq_review_required"][0]["trade_id"], "d15e0001");

    // Only the mediated trade's deposit is held, by its persisted review,
    // instead of booking as a disposal.
    let processed = run("journals process");
    assert_eq!(processed["data"]["quarantined"], 1, "{processed}");
    let quarantined = run("journals quarantined");
    let held = &quarantined["data"][0];
    assert_eq!(held["external_id"], "f4".repeat(32), "{quarantined}");
    assert_eq!(held["reason"], "bisq_trade_review_required");

    let listed = run("journals list");
    let entries = listed["data"]["entries"]
        .as_array()
        .or_else(|| listed["data"].as_array());
    // Entries of `kind` as `wallet sats value basis gain`, in cents so float
    // noise cannot hide a wrong basis.
    let of = |kind: &str| {
        let cents = |e: &Value, key: &str| (e[key].as_f64().unwrap_or(0.0) * 100.0).round() as i64;
        let mut found = entries
            .expect("journal entries")
            .iter()
            .filter(|e| e["entry_type"] == kind)
            .map(|e| {
                let sats = (e["quantity"].as_f64().unwrap() * 1e8).round() as i64;
                let wallet = e["wallet"].as_str().unwrap();
                let money = [
                    cents(e, "fiat_value"),
                    cents(e, "cost_basis"),
                    cents(e, "gain_loss"),
                ];
                format!("{wallet} {sats} {} {} {}", money[0], money[1], money[2])
            })
            .collect::<Vec<_>>();
        found.sort();
        found
    };
    // One sale at the contract's volume against the Austrian moving
    // average (0.19973 BTC at 50,000 plus 0.025 BTC at 60,000 after the
    // fees), and the on-chain taker fee. No security deposit is disposed of.
    assert_eq!(
        of("disposal"),
        [
            "Bisq -5000000 310000 255562 54438",
            "Watch -22000 1050 1100 -50"
        ]
    );
    assert_eq!(
        of("acquisition"),
        ["Bisq 2500000 150000 0 0", "Watch 20000000 1000000 0 0"]
    );
    // Each native gross deposit's miner fees, as Bisq states them, book as
    // transfer fees at basis: nothing is left in suspense.
    assert_eq!(
        of("transfer_fee"),
        ["Watch -4000 204 204 0", "Watch -5000 250 250 0"]
    );
    // Deposits move into the escrow and payouts back, as transfers.
    let moves = of("transfer_in")
        .iter()
        .map(|e| e.split(' ').take(2).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>();
    assert_eq!(
        moves,
        [
            "Bisq 5750000",
            "Bisq 600000",
            "Watch 3100000",
            "Watch 750000"
        ]
    );
}
