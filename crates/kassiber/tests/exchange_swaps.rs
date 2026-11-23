//! The ETH canary (docs/plan/24-crypto-assets.md): a Kraken API sync against
//! a loopback fake into an Austrian book, through the binary on a disposable
//! data root. ETH bought for EUR, swapped into BTC, the BTC sold for EUR.
//! The `crypto` scope stores every row, pairs the swap, carries the ETH
//! basis into BTC and realizes the gain on the BTC sale; the `bitcoin` scope
//! refuses the swap's BTC leg instead of booking it as a sale or a buy.

use kassiber_net::test_support::HandledProvider;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const KEY: &str = "synthetic-exchange-key";
// Kraken secrets are base64.
const SECRET: &str = "c3ludGhldGljLXNlY3JldA==";

fn command(root: &Path, args: &[&str]) -> Output {
    binary(root, args).output().expect("the binary runs")
}

fn binary(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
    command
        .arg("--data-root")
        .arg(root)
        .arg("--machine")
        .args(args)
        // The test observer lets loopback through and stops anything else.
        .env("KASSIBER_TEST_NO_EGRESS", "1")
        .env_remove("KASSIBER_NO_EGRESS")
        .env_remove("KASSIBER_RUST_TEST_FAKE_STORE")
        .env("HOME", root)
        .env("XDG_DATA_HOME", root.join("data-home"))
        .env("XDG_CONFIG_HOME", root.join("config-home"))
        .env(
            "KASSIBER_OFFLINE_PREFERENCE_FILE",
            root.join("offline.json"),
        );
    command
}

/// The daemon's answer to one request for the current book, as the
/// desktop sends it.
fn daemon(root: &Path, kind: &str, args: &Value) -> Value {
    let mut child = binary(root, &["daemon"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the daemon starts");
    let request = json!({"request_id": "1", "kind": kind, "args": args});
    writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
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

fn envelope(output: &Output, args: &[&str]) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<Value>(&stdout).unwrap_or_else(|_| {
        panic!(
            "{args:?}: {stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// A Kraken that answers `TradesHistory` with `trades` and `Ledgers` with
/// `ledger`, each in one page.
fn kraken(ledger: Value, trades: Value) -> HandledProvider {
    HandledProvider::start(move |request| {
        let (field, rows) = if request.target.starts_with("/0/private/TradesHistory") {
            ("trades", &trades)
        } else {
            ("ledger", &ledger)
        };
        let count = rows.as_object().map_or(0, |rows| rows.len());
        (
            200,
            json!({"error": [], "result": {field: rows, "count": count}}).to_string(),
        )
    })
    .unwrap()
}

fn row(asset: &str, amount: &str, refid: &str, time: i64) -> Value {
    json!({"type": "trade", "asset": asset, "amount": amount, "fee": "0", "refid": refid, "time": time, "aclass": "currency"})
}

/// 1 ETH bought for 2,000 EUR (2024-01-10), swapped for 0.05 BTC
/// (2025-03-01), the BTC sold for 4,000 EUR (2025-06-01).
fn eth_round_trip() -> HandledProvider {
    let (buy, swap, sell) = (1_704_880_800, 1_740_823_200, 1_748_772_000);
    kraken(
        json!({
            "L1": row("XETH", "1.0", "T1", buy),
            "L1E": row("ZEUR", "-2000", "T1", buy),
            "L2": row("XETH", "-1.0", "T2", swap),
            "L2B": row("XXBT", "0.05", "T2", swap),
            "L3": row("XXBT", "-0.05", "T3", sell),
            "L3E": row("ZEUR", "4000", "T3", sell),
        }),
        json!({
            "T1": {"pair": "XETHZEUR", "cost": "2000", "fee": "0", "price": "2000"},
            "T2": {"pair": "XETHXXBT", "cost": "0.05", "fee": "0", "price": "0.05"},
            "T3": {"pair": "XXBTZEUR", "cost": "4000", "fee": "0", "price": "80000"},
        }),
    )
}

const SYNC: [&str; 6] = [
    "wallets",
    "sync-kraken",
    "--backend",
    "kraken",
    "--wallet",
    "Kraken",
];

struct Book<'a> {
    root: &'a Path,
}

impl Book<'_> {
    const IN_BOOK: [&'static str; 4] = ["--workspace", "Main", "--profile", "Book"];

    fn new<'a>(root: &'a Path, scope: &str, url: &str) -> Book<'a> {
        let book = Book { root };
        book.run(&["init"]);
        book.run(&["workspaces", "create", "Main"]);
        book.run(&[
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
        book.run(&[
            "backends",
            "create",
            "kraken",
            "--kind",
            "kraken",
            "--url",
            url,
            "--token",
            KEY,
            "--auth-header",
            SECRET,
        ]);
        let config = json!({"backend": "kraken", "asset_scope": scope}).to_string();
        book.ok(&[
            "wallets", "create", "--label", "Kraken", "--kind", "kraken", "--config", &config,
        ]);
        book
    }

    fn run(&self, args: &[&str]) -> Value {
        let output = command(self.root, args);
        let value = envelope(&output, args);
        assert!(output.status.success(), "{args:?}: {value}");
        value["data"].clone()
    }

    fn args<'b>(&self, args: &[&'b str]) -> Vec<&'b str> {
        let mut all = args.to_vec();
        all.extend(Self::IN_BOOK);
        all
    }

    fn ok(&self, args: &[&str]) -> Value {
        self.run(&self.args(args))
    }

    /// The envelope of a command that fails.
    fn refused(&self, args: &[&str]) -> Value {
        let args = self.args(args);
        let output = command(self.root, &args);
        let value = envelope(&output, &args);
        assert!(!output.status.success(), "{args:?}: {value}");
        value
    }

    /// How many transactions the book holds.
    fn transactions(&self) -> usize {
        let listed = self.ok(&["transactions", "list", "--limit", "100"]);
        listed.as_array().unwrap().len()
    }
}

#[test]
fn an_austrian_kraken_eth_round_trip_carries_basis_into_btc() {
    let fake = eth_round_trip();
    let home = tempfile::tempdir().unwrap();
    let book = Book::new(home.path(), "crypto", fake.base());

    let synced = book.ok(&SYNC);
    assert_eq!(synced["imported"], 4, "{synced}");
    assert_eq!(synced["swap_pairs"], 1, "{synced}");
    assert_eq!(
        synced["assets"],
        json!({"scope": "crypto", "skipped": {}, "excluded": {}})
    );

    // Stored rows: the ETH buy priced from its trade, two unpriced swap
    // legs, the BTC sell priced from its trade.
    let listed = book.ok(&["transactions", "list", "--limit", "100"]);
    let stored = |external: &str| {
        listed
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["external_id"] == format!("kraken:{external}"))
            .unwrap_or_else(|| panic!("{external}: {listed}"))
            .clone()
    };
    // `(asset, direction, kind, amount in whole units, exact fiat value)`.
    let shape = |external: &str| {
        let row = stored(external);
        json!([
            row["asset"],
            row["direction"],
            row["kind"],
            row["amount"],
            row["fiat_value_exact"]
        ])
    };
    assert_eq!(shape("L1"), json!(["ETH", "inbound", "buy", 1.0, "2000"]));
    assert_eq!(shape("L2"), json!(["ETH", "outbound", "swap", 1.0, null]));
    assert_eq!(shape("L2B"), json!(["BTC", "inbound", "swap", 0.05, null]));
    assert_eq!(
        shape("L3"),
        json!(["BTC", "outbound", "sell", 0.05, "4000"])
    );
    assert_eq!(book.transactions(), 4);

    // The swap pair: ETH out, BTC in, carrying value as Austria recommends.
    let pairs = book.ok(&["transfers", "list"]);
    let pairs = pairs.as_array().unwrap();
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    let pair = &pairs[0];
    assert_eq!(pair["out"]["external_id"], "kraken:L2", "{pair}");
    assert_eq!(pair["in"]["external_id"], "kraken:L2B", "{pair}");
    assert_eq!(pair["policy"], "carrying-value", "{pair}");
    assert_eq!(pair["pair_source"], "provider", "{pair}");
    // Only an import authors a provider pair; no caller may claim the source.
    let claimed = daemon(
        home.path(),
        "ui.transfers.pair",
        &json!({"tx_out": stored("L2")["id"], "tx_in": stored("L2B")["id"], "pair_source": "provider"}),
    );
    assert!(
        claimed.to_string().contains("pair_source") && claimed["ok"] != json!(true),
        "{claimed}"
    );

    // Kraken states no fiat value for the swap and the book has no BTC rate,
    // so journals hold both legs until they are priced, and the BTC sale
    // finds no lots while the swap's BTC is held.
    let processed = book.ok(&["journals", "process"]);
    assert_eq!(processed["quarantined"], 3, "{processed}");
    let held = book.ok(&["journals", "quarantined"]);
    let reasons: Vec<(&str, &str)> = held
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["external_id"].as_str().unwrap(),
                row["reason"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        reasons,
        [
            ("kraken:L3", "insufficient_lots"),
            ("kraken:L2", "at_swap_basis_carry_unresolved"),
            ("kraken:L2B", "at_swap_basis_carry_unresolved"),
        ],
        "{held}"
    );
    for row in held.as_array().unwrap().iter().skip(1) {
        assert_eq!(row["detail"]["reason_code"], "missing_spot_price", "{row}");
        book.ok(&[
            "journals",
            "quarantine",
            "resolve",
            "price-override",
            "--transaction",
            row["transaction_id"].as_str().unwrap(),
            "--fiat-value",
            "3000",
        ]);
    }
    let processed = book.ok(&["journals", "process"]);
    assert_eq!(processed["quarantined"], 0, "{processed}");

    // The swap is tax-neutral: it realizes nothing on the ETH, whatever it
    // was worth (3,000), and the BTC carries the ETH's 2,000 basis.
    let entries = book.ok(&["journals", "list"]);
    let entry = |kind: &str, asset: &str| {
        entries
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entry_type"] == kind && e["asset"] == asset)
            .unwrap_or_else(|| panic!("{kind} {asset}: {entries}"))
            .clone()
    };
    let out = entry("disposal", "ETH");
    assert_eq!(out["at_category"], "neu_swap", "{out}");
    assert_eq!(out["gain_loss"], 0.0, "{out}");
    assert_eq!(out["cost_basis"], out["proceeds"], "{out}");
    let into = entry("acquisition", "BTC");
    assert_eq!(into["quantity"], 0.05, "{into}");
    assert_eq!(into["fiat_value"], 2000.0, "{into}");

    // Selling the BTC for 4,000 realizes 2,000, the only realized result.
    let gains = book.ok(&["reports", "capital-gains"]);
    let realized: Vec<Value> = gains
        .as_array()
        .unwrap_or_else(|| panic!("{gains}"))
        .iter()
        .map(|row| {
            json!([
                row["asset"],
                row["at_category"],
                row["quantity"],
                row["proceeds"],
                row["cost_basis"],
                row["gain_loss"]
            ])
        })
        .collect();
    assert_eq!(
        realized,
        [json!(["BTC", "neu_gain", 0.05, 4000.0, 2000.0, 2000.0])],
        "{gains}"
    );
}

#[test]
fn a_bitcoin_scope_kraken_sync_refuses_the_btc_leg_of_an_eth_swap() {
    let fake = eth_round_trip();
    let home = tempfile::tempdir().unwrap();
    let book = Book::new(home.path(), "bitcoin", fake.base());

    let refused = book.refused(&SYNC);
    let error = &refused["error"];
    assert_eq!(error["code"], "validation", "{refused}");
    assert_eq!(
        error["message"], "Kraken BTC trade 'L2B' is not fiat-quoted",
        "{refused}"
    );
    // Nothing is booked: no sale, no buy, no pair.
    assert_eq!(book.transactions(), 0);
    assert_eq!(book.ok(&["transfers", "list"]), json!([]));
}

/// 1 ETH bought for 2,000 EUR, then half swapped for 0.025 BTC and, an
/// hour later, half for 10 SOL.
fn eth_into_btc_and_sol() -> HandledProvider {
    let (buy, swap, later) = (1_704_880_800, 1_740_823_200, 1_740_826_800);
    kraken(
        json!({
            "L1": row("XETH", "1.0", "T1", buy),
            "L1E": row("ZEUR", "-2000", "T1", buy),
            "L2": row("XETH", "-0.5", "T2", swap),
            "L2B": row("XXBT", "0.025", "T2", swap),
            "L4": row("XETH", "-0.5", "T4", later),
            "L4S": row("SOL", "10", "T4", later),
        }),
        json!({
            "T1": {"pair": "XETHZEUR", "cost": "2000", "fee": "0", "price": "2000"},
            "T2": {"pair": "XETHXXBT", "cost": "0.025", "fee": "0", "price": "0.05"},
            "T4": {"pair": "SOLXETH", "cost": "0.5", "fee": "0", "price": "0.05"},
        }),
    )
}

#[test]
fn a_cached_btc_rate_prices_both_legs_of_a_btc_swap_and_no_other_swap() {
    let fake = eth_into_btc_and_sol();
    let home = tempfile::tempdir().unwrap();
    let book = Book::new(home.path(), "crypto", fake.base());
    let synced = book.ok(&SYNC);
    assert_eq!(synced["swap_pairs"], 2, "{synced}");
    book.run(&["rates", "set", "BTC-EUR", "2025-03-01T00:00:00Z", "60000"]);

    let processed = book.ok(&["journals", "process"]);
    // Both BTC swap legs, priced from the one cached BTC rate.
    assert_eq!(processed["auto_priced"], 2, "{processed}");
    let listed = book.ok(&["transactions", "list", "--limit", "100"]);
    let pricing = |external: &str| {
        let row = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["external_id"] == format!("kraken:{external}"))
            .unwrap_or_else(|| panic!("{external}: {listed}"));
        json!([
            row["fiat_value_exact"],
            row["pricing_source_kind"],
            row["pricing_pair"],
            row["pricing_method"]
        ])
    };
    // The BTC leg is worth 0.025 BTC at 60,000; the ETH leg takes that value.
    assert_eq!(
        pricing("L2B"),
        json!(["1500.000", "manual_rate_cache", "BTC-EUR", "manual"])
    );
    assert_eq!(
        pricing("L2"),
        json!([
            "1500.000",
            "manual_rate_cache",
            "BTC-EUR",
            "swap_counter_leg"
        ])
    );
    // A swap without a Bitcoin leg stays unpriced and in review.
    for leg in ["L4", "L4S"] {
        assert_eq!(pricing(leg), json!([null, null, null, null]), "{leg}");
    }
    let held = book.ok(&["journals", "quarantined"]);
    let held: Vec<Value> = held
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            json!([
                row["external_id"],
                row["reason"],
                row["detail"]["reason_code"]
            ])
        })
        .collect();
    assert_eq!(
        held,
        [
            json!([
                "kraken:L4",
                "at_swap_basis_carry_unresolved",
                "missing_spot_price"
            ]),
            json!([
                "kraken:L4S",
                "at_swap_basis_carry_unresolved",
                "missing_spot_price"
            ]),
        ]
    );

    // The priced swap carries half the ETH basis into the BTC.
    let entries = book.ok(&["journals", "list"]);
    let into = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["entry_type"] == "acquisition" && e["asset"] == "BTC")
        .unwrap_or_else(|| panic!("{entries}"));
    assert_eq!(into["quantity"], 0.025, "{into}");
    assert_eq!(into["fiat_value"], 1000.0, "{into}");
}

/// 1 ETH bought for 2,000 EUR, then 0.5 ETH withdrawn with a 0.001 ETH fee
/// to the user's own wallet, which Kassiber does not observe.
fn eth_withdrawn() -> HandledProvider {
    let (buy, withdrawal) = (1_704_880_800, 1_740_823_200);
    kraken(
        json!({
            "L1": row("XETH", "1.0", "T1", buy),
            "L1E": row("ZEUR", "-2000", "T1", buy),
            "W1": {"type": "withdrawal", "asset": "XETH", "amount": "-0.5", "fee": "0.001", "time": withdrawal, "aclass": "currency"},
        }),
        json!({
            "T1": {"pair": "XETHZEUR", "cost": "2000", "fee": "0", "price": "2000"},
        }),
    )
}

#[test]
fn an_eth_withdrawal_to_an_untracked_own_wallet_is_no_disposal() {
    let fake = eth_withdrawn();
    let home = tempfile::tempdir().unwrap();
    let book = Book::new(home.path(), "crypto", fake.base());
    book.ok(&SYNC);
    let held_reasons = || {
        let held = book.ok(&["journals", "quarantined"]);
        held.as_array()
            .unwrap()
            .iter()
            .map(|row| {
                json!([
                    row["external_id"],
                    row["reason"],
                    row["detail"]["required_for"]
                ])
            })
            .collect::<Vec<_>>()
    };

    // Unreviewed, the withdrawal leaves the book: a disposal waiting for a
    // price.
    book.ok(&["journals", "process"]);
    assert_eq!(
        held_reasons(),
        [json!(["kraken:W1", "missing_spot_price", "disposal"])]
    );

    // The owner reviews it as moved to an own wallet Kassiber does not
    // track: the complete debit, the fee, and the rest retained in custody.
    let components = json!([{
        "legs": [
            {"role": "source", "transaction": "kraken:W1", "amount_msat": 50_100_000_000_i64},
            {"role": "fee", "asset": "ETH", "occurred_at": "2025-03-01T10:00:00Z",
             "amount_msat": 100_000_000},
            {"role": "retained", "untracked_wallet": "My ETH wallet", "asset": "ETH",
             "notes": "reviewed_residual:retained_custody",
             "occurred_at": "2025-03-01T10:00:00Z", "amount_msat": 50_000_000_000_i64},
        ]
    }])
    .to_string();
    let plan = book.ok(&[
        "transfers",
        "components",
        "plan",
        "--action",
        "create",
        "--json",
        &components,
    ]);
    assert_eq!(
        plan["components"][0]["validation"]["activatable"], true,
        "{plan}"
    );
    let version = plan["input_version"].to_string();
    book.ok(&[
        "transfers",
        "components",
        "apply",
        "--action",
        "create",
        "--json",
        &components,
        "--expected-input-version",
        &version,
    ]);

    // Custody now resolves; the withdrawal still needs a price for its fee.
    let processed = book.ok(&["journals", "process"]);
    assert_eq!(processed["custody_quantity"]["issues"], 0, "{processed}");
    let held = book.ok(&["journals", "quarantined"]);
    book.ok(&[
        "journals",
        "quarantine",
        "resolve",
        "price-override",
        "--transaction",
        held[0]["transaction_id"].as_str().unwrap(),
        "--fiat-rate",
        "2000",
    ]);
    let processed = book.ok(&["journals", "process"]);
    assert_eq!(processed["quarantined"], 0, "{processed}");

    // The fee is spent; the 0.5 ETH is not disposed of and realizes nothing.
    let entries = book.ok(&["journals", "list"]);
    let booked: Vec<Value> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| json!([e["entry_type"], e["asset"], e["quantity"], e["gain_loss"]]))
        .collect();
    assert_eq!(
        booked,
        [
            json!(["fee", "ETH", -0.001, 0.0]),
            json!(["acquisition", "ETH", 1.0, 0]),
        ],
        "{entries}"
    );
    assert_eq!(book.ok(&["reports", "capital-gains"]), json!([]));
}

/// 1 ETH bought for 2,000 EUR, then half swapped for 0.025 BTC and, an
/// hour later, a quarter for 0.01 BTC.
fn eth_into_btc_twice() -> HandledProvider {
    let (buy, swap, later) = (1_704_880_800, 1_740_823_200, 1_740_826_800);
    kraken(
        json!({
            "L1": row("XETH", "1.0", "T1", buy),
            "L1E": row("ZEUR", "-2000", "T1", buy),
            "L2": row("XETH", "-0.5", "T2", swap),
            "L2B": row("XXBT", "0.025", "T2", swap),
            "L5": row("XETH", "-0.25", "T5", later),
            "L5B": row("XXBT", "0.01", "T5", later),
        }),
        json!({
            "T1": {"pair": "XETHZEUR", "cost": "2000", "fee": "0", "price": "2000"},
            "T2": {"pair": "XETHXXBT", "cost": "0.025", "fee": "0", "price": "0.05"},
            "T5": {"pair": "XETHXXBT", "cost": "0.01", "fee": "0", "price": "0.04"},
        }),
    )
}

/// A book of [`eth_into_btc_twice`] whose BTC legs the rates cache priced
/// at 60,000 EUR, so the ETH legs took 1,500 and 600 from them.
fn priced_swaps<'a>(root: &'a Path, url: &str) -> Book<'a> {
    let book = Book::new(root, "crypto", url);
    let synced = book.ok(&SYNC);
    assert_eq!(synced["swap_pairs"], 2, "{synced}");
    book.run(&["rates", "set", "BTC-EUR", "2025-03-01T00:00:00Z", "60000"]);
    book.ok(&["journals", "process"]);
    assert_eq!(
        counter_leg(&book, "L2"),
        json!(["1500.000", "swap_counter_leg"])
    );
    assert_eq!(
        counter_leg(&book, "L5"),
        json!(["600.00", "swap_counter_leg"])
    );
    book
}

/// A row's `(fiat_value_exact, pricing_method)` and its transaction id.
fn priced(book: &Book<'_>, external: &str) -> (Value, String) {
    let listed = book.ok(&["transactions", "list", "--limit", "100"]);
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["external_id"] == format!("kraken:{external}"))
        .unwrap_or_else(|| panic!("{external}: {listed}"))
        .clone();
    (
        json!([row["fiat_value_exact"], row["pricing_method"]]),
        row["id"].as_str().unwrap().to_owned(),
    )
}

fn counter_leg(book: &Book<'_>, external: &str) -> Value {
    priced(book, external).0
}

/// The id of the pair whose outbound leg is `external`.
fn pair_of(book: &Book<'_>, external: &str) -> String {
    let pairs = book.ok(&["transfers", "list"]);
    let pair = pairs
        .as_array()
        .unwrap()
        .iter()
        .find(|pair| pair["out"]["external_id"] == format!("kraken:{external}"))
        .unwrap_or_else(|| panic!("{external}: {pairs}"))
        .clone();
    pair["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{pair}"))
        .to_owned()
}

/// Whether journals hold `external` for a missing price.
fn held_for_price(book: &Book<'_>, external: &str) -> bool {
    let held = book.ok(&["journals", "quarantined"]);
    held.as_array().unwrap().iter().any(|row| {
        row["external_id"] == format!("kraken:{external}")
            && row["detail"]["reason_code"] == "missing_spot_price"
    })
}

#[test]
fn a_counter_leg_price_follows_its_pair_on_every_pass() {
    let unpriced = json!([null, null]);
    let fake = eth_into_btc_twice();

    // Unpaired: the ETH leg has no pair to take a value from.
    let home = tempfile::tempdir().unwrap();
    let book = priced_swaps(home.path(), fake.base());
    let pair = pair_of(&book, "L2");
    book.ok(&["transfers", "unpair", "--pair-id", &pair]);
    book.ok(&["journals", "process"]);
    assert_eq!(counter_leg(&book, "L2"), unpriced);
    assert_eq!(
        counter_leg(&book, "L5"),
        json!(["600.00", "swap_counter_leg"])
    );

    // The BTC leg priced by hand: the ETH leg is unpriced and back in review.
    let home = tempfile::tempdir().unwrap();
    let book = priced_swaps(home.path(), fake.base());
    let (_, btc) = priced(&book, "L2B");
    book.run(&["context", "set", "--workspace", "Main", "--profile", "Book"]);
    let updated = daemon(
        home.path(),
        "ui.transactions.metadata.update",
        &json!({"transaction": btc, "fiat_value": "1700", "pricing_source_kind": "manual_override"}),
    );
    assert_eq!(
        updated["kind"], "ui.transactions.metadata.update",
        "{updated}"
    );
    assert_eq!(
        counter_leg(&book, "L2B"),
        json!(["1700", "desktop_transaction_detail"])
    );
    book.ok(&["journals", "process"]);
    assert_eq!(counter_leg(&book, "L2"), unpriced);
    assert!(held_for_price(&book, "L2"));

    // A paired BTC leg cannot be excluded; unpaired first, then excluded,
    // it leaves the ETH leg unpriced.
    let home = tempfile::tempdir().unwrap();
    let book = priced_swaps(home.path(), fake.base());
    let (_, btc) = priced(&book, "L2B");
    let refused = book.refused(&["metadata", "exclude", "--transaction", &btc]);
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    let pair = pair_of(&book, "L2");
    book.ok(&["transfers", "unpair", "--pair-id", &pair]);
    book.ok(&["metadata", "exclude", "--transaction", &btc]);
    book.ok(&["journals", "process"]);
    assert_eq!(counter_leg(&book, "L2"), unpriced);

    // Re-paired to the other BTC leg: the ETH leg takes that leg's value.
    let home = tempfile::tempdir().unwrap();
    let book = priced_swaps(home.path(), fake.base());
    for leg in ["L2", "L5"] {
        let pair = pair_of(&book, leg);
        book.ok(&["transfers", "unpair", "--pair-id", &pair]);
    }
    book.ok(&[
        "transfers",
        "pair",
        "--tx-out",
        "kraken:L2",
        "--tx-in",
        "kraken:L5B",
        "--policy",
        "carrying-value",
    ]);
    book.ok(&["journals", "process"]);
    assert_eq!(
        counter_leg(&book, "L2"),
        json!(["600.00", "swap_counter_leg"])
    );
    assert_eq!(counter_leg(&book, "L5"), unpriced);
}

/// A pair converting only part of its source (0.25 of the 0.5 ETH leg) does
/// not copy the BTC leg's whole value onto the ETH leg: it stays unpriced.
#[test]
fn a_partial_swap_takes_no_counter_leg_price() {
    let fake = eth_into_btc_twice();
    let home = tempfile::tempdir().unwrap();
    let book = priced_swaps(home.path(), fake.base());
    let pair = pair_of(&book, "L2");
    book.ok(&["transfers", "unpair", "--pair-id", &pair]);
    book.ok(&[
        "transfers",
        "pair",
        "--tx-out",
        "kraken:L2",
        "--tx-in",
        "kraken:L2B",
        "--policy",
        "carrying-value",
        "--out-amount",
        "0.25",
    ]);
    book.ok(&["journals", "process"]);
    assert_eq!(counter_leg(&book, "L2"), json!([null, null]));
}

/// A non-Bitcoin leg given a market-price source by hand prices nothing:
/// the SOL leg of the ETH -> SOL swap stays unpriced.
#[test]
fn only_a_bitcoin_leg_prices_its_counter_leg() {
    let fake = eth_into_btc_and_sol();
    let home = tempfile::tempdir().unwrap();
    let book = Book::new(home.path(), "crypto", fake.base());
    book.ok(&SYNC);
    let (_, eth) = priced(&book, "L4");
    book.run(&["context", "set", "--workspace", "Main", "--profile", "Book"]);
    let updated = daemon(
        home.path(),
        "ui.transactions.metadata.update",
        &json!({"transaction": eth, "fiat_value": "1000", "pricing_source_kind": "fmv_provider"}),
    );
    assert_eq!(
        updated["kind"], "ui.transactions.metadata.update",
        "{updated}"
    );
    book.ok(&["journals", "process"]);
    assert_eq!(counter_leg(&book, "L4S"), json!([null, null]));
}
