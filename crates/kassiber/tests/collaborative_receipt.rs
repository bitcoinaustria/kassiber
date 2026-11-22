//! A Payjoin the owner received, through the real binary: the wallet adds
//! one input, an outside party adds another and pays the fee, and the wallet
//! ends up 0.1 BTC richer. The chain proves the wallet's net delta but not
//! what the outside party's contribution was, so the receipt is held as
//! `privacy_hop_unresolved` and a later spend waits behind the basis barrier.
//! Classifying the receipt (`kind_override` through `review plan/apply`)
//! books the net 0.1 BTC as that acquisition at market value; the wallet's
//! own input never leaves it. A generic `deposit`, or another row of the
//! same transaction (also an excluded one), keeps the hold. See docs/reference/custody-resolution.md.

use kassiber_store::rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};

const PAYJOIN: &str = "4f6201d6b8eb75beb1f0a8cffbc8a2d98d477b6df8333f5b7f55f915cd87799a";
const FUNDING: &str = "c58e13619c31dfcc8050b9cc363a34c76d8aab0c8e619cd86e4275c45415c9ae";
const FOREIGN_PARENT: &str = "ef168ee8e4a5d6124edfb429d878c67a9bb3fb127a75567971354797cf37bcfa";
const SPEND: &str = "7777777777777777777777777777777777777777777777777777777777777777";
/// The wallet's input and its receiving output (regtest P2WPKH).
const INPUT_ADDRESS: &str = "bcrt1qef30zg2r2pfarjegm9rz86m0dhk78cndpshkmh";
const INPUT_SCRIPT: &str = "0014ca62f121435053d1cb28d94623eb6f6dede3e26d";
const OUTPUT_ADDRESS: &str = "bcrt1q88u0c3l8rtsw2d3zxau4tg8lu0vsq2myr646ec";
const OUTPUT_SCRIPT: &str = "001439f8fc47e71ae0e53622377955a0ffe3d9002b64";
const MSAT_PER_SAT: i64 = 1000;

fn cli(root: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_kassiber"))
        .arg("--data-root")
        .arg(root.join("data"))
        .arg("--machine")
        .args(args)
        .env("KASSIBER_NO_EGRESS", "1")
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
    assert!(output.status.success(), "{args:?}: {stdout}{stderr}");
    if stdout.trim().is_empty() {
        // `--output` wrote the envelope to its file.
        return Value::Null;
    }
    serde_json::from_str(&stdout).unwrap()
}

/// The Payjoin as native sync stores it for the receiving wallet: the
/// wallet's 0.50568205 BTC input and 0.60568205 BTC output, the outside
/// party's input and output, and the row's amount as the wallet's net delta.
fn payjoin_graph() -> String {
    json!({
        "_kassiber_electrum_graph": {"kind": "bitcoin_electrum", "version": 2},
        "component": {"fee_attribution": "implicit_wallet_delta"},
        "fee": 2000,
        "locktime": 0,
        "observer_owned_scripts": [OUTPUT_SCRIPT, INPUT_SCRIPT],
        "privacy_boundary": "collaborative",
        "status": {"confirmed": true},
        "txid": PAYJOIN,
        "version": 2,
        "vin": [
            {"prevout": {"scriptpubkey": INPUT_SCRIPT, "value": 50_568_205},
             "sequence": 4_294_967_293_u32, "txid": FUNDING, "vout": 1},
            {"prevout": {"scriptpubkey": "00149077229335a41ac184e757a0b7374d2548dcf5fe", "value": 312_500_444},
             "sequence": 4_294_967_293_u32, "txid": FOREIGN_PARENT, "vout": 0}
        ],
        "vout": [
            {"n": 0, "scriptpubkey": OUTPUT_SCRIPT, "value": 60_568_205},
            {"n": 1, "scriptpubkey": "0014a179da5c3fac44f162860834124b03a0bd92a3e9", "value": 302_498_444}
        ]
    })
    .to_string()
}

struct Book {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    workspace: String,
    profile: String,
    wallet: String,
}

impl Book {
    /// A generic EUR book whose wallet bought the Payjoin's input, received
    /// the Payjoin, then spent 0.55 BTC.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        cli(&root, &["init"]);
        cli(&root, &["workspaces", "create", "main"]);
        let profile = cli(
            &root,
            &[
                "profiles",
                "create",
                "--workspace",
                "main",
                "--fiat-currency",
                "EUR",
                "--tax-country",
                "generic",
                "--gains-algorithm",
                "FIFO",
                "book",
            ],
        );
        let wallet = cli(
            &root,
            &[
                "wallets",
                "create",
                "--label",
                "Hot",
                "--kind",
                "address",
                "--chain",
                "bitcoin",
                "--network",
                "regtest",
                "--address",
                INPUT_ADDRESS,
                "--address",
                OUTPUT_ADDRESS,
            ],
        );
        let book = Self {
            _dir: dir,
            root,
            workspace: profile["data"]["workspace_id"].as_str().unwrap().to_owned(),
            profile: profile["data"]["id"].as_str().unwrap().to_owned(),
            wallet: wallet["data"]["id"].as_str().unwrap().to_owned(),
        };
        let funding = 50_568_205 * MSAT_PER_SAT;
        book.insert(
            "funding",
            &book.wallet,
            FUNDING,
            "2025-01-01T00:00:00Z",
            "inbound",
            funding,
            "buy",
            40_000,
            "{}",
        );
        book.insert(
            "spend",
            &book.wallet,
            SPEND,
            "2025-03-01T00:00:00Z",
            "outbound",
            55_000_000 * MSAT_PER_SAT,
            "withdrawal",
            60_000,
            "{}",
        );
        book.insert_native("payjoin", &book.wallet, 10_000_000 * MSAT_PER_SAT);
        book
    }

    fn db(&self) -> Connection {
        Connection::open(self.root.join("data/kassiber.sqlite3")).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        &self,
        id: &str,
        wallet: &str,
        txid: &str,
        at: &str,
        direction: &str,
        amount: i64,
        kind: &str,
        rate: i64,
        raw: &str,
    ) {
        self.db()
            .execute(
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,confirmed_at,direction,asset,amount,fee,fiat_currency,fiat_rate,fiat_rate_exact,kind,raw_json,created_at)
                 VALUES(?1,?2,?3,?4,?5,?1,?6,?6,?7,'BTC',?8,0,'EUR',?9,?10,?11,?12,'2025-01-01T00:00:00Z')",
                params![id, self.workspace, self.profile, wallet, txid, at, direction, amount, rate as f64, rate.to_string(), kind, raw],
            )
            .unwrap();
    }

    /// The Payjoin's row in `wallet` with the closed observer provenance
    /// native sync records for it.
    fn insert_native(&self, id: &str, wallet: &str, amount: i64) {
        let raw = payjoin_graph();
        self.insert(
            id,
            wallet,
            PAYJOIN,
            "2025-02-01T00:00:00Z",
            "inbound",
            amount,
            "deposit",
            50_000,
            &raw,
        );
        let db = self.db();
        db.execute(
            "UPDATE transactions SET external_id_kind='txid', privacy_boundary='collaborative' WHERE id=?",
            [id],
        )
        .unwrap();
        let row = json!({"wallet_id": wallet, "external_id": PAYJOIN, "direction": "inbound",
            "asset": "BTC", "amount": amount, "fee": 0, "amount_includes_fee": 0});
        let graph = kassiber_custody::evidence::graph_hash(&json!(raw))
            .unwrap()
            .0;
        let quantity = kassiber_custody::evidence::observed_hash(&row).unwrap().0;
        db.execute(
            "INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,authority_version,observer_ids_json,observer_kinds_json,chain,network,application_revision,graph_hash,quantity_hash,fee_attribution,observed_at,updated_at)
             VALUES(?1,?2,?3,?4,1,'[\"electrum\"]','[\"electrum\"]','bitcoin','regtest','r',?5,?6,'implicit_wallet_delta','2025-02-01T00:00:00Z','2025-02-01T00:00:00Z')",
            params![id, self.workspace, self.profile, wallet, graph, quantity],
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> Value {
        cli(&self.root, args)
    }

    /// `(transaction_id, reason)` of every quarantined row, sorted.
    fn held(&self) -> Vec<(String, String)> {
        let listed = self.run(&["journals", "quarantined"]);
        let mut held: Vec<_> = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["transaction_id"].as_str().unwrap().to_owned(),
                    row["reason"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        held.sort();
        held
    }

    /// A review plan of one `kind_override` on the Payjoin.
    fn plan(&self, kind: &str) -> (std::path::PathBuf, Value) {
        let cases = self.run(&["review", "cases", "--limit", "100"]);
        let version = match &cases["data"]["input_version"] {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let operations = self.root.join(format!("{kind}.json"));
        std::fs::write(
            &operations,
            json!([{"type": "kind_override", "transaction_id": "payjoin", "kind": kind,
                    "reason": "Payjoin receive: the counterparty paid the invoice"}])
            .to_string(),
        )
        .unwrap();
        let artifact = self.root.join(format!("{kind}-plan.json"));
        let out = artifact.to_str().unwrap();
        self.run(&[
            "--output",
            out,
            "review",
            "plan",
            "--operations-file",
            operations.to_str().unwrap(),
            "--expected-input-version",
            &version,
        ]);
        let plan: Value = serde_json::from_slice(&std::fs::read(&artifact).unwrap()).unwrap();
        (artifact, plan)
    }
}

fn held_after(plan: &Value) -> Vec<(String, String)> {
    let mut held: Vec<_> = plan["data"]["after"]["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["transaction_id"].as_str().unwrap().to_owned(),
                row["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    held.sort();
    held
}

fn pair(id: &str, reason: &str) -> (String, String) {
    (id.to_owned(), reason.to_owned())
}

#[test]
fn a_classified_payjoin_receipt_books_its_net_amount_and_releases_the_barrier() {
    let book = Book::new();
    book.run(&["journals", "process"]);
    assert_eq!(
        book.held(),
        [
            pair("payjoin", "privacy_hop_unresolved"),
            pair("spend", "custody_basis_barrier")
        ]
    );
    // The held receipt leads with classifying it, on the quarantine page and
    // in the review case.
    let page = book.run(&["journals", "quarantined"]);
    let receipt = page["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["transaction_id"] == "payjoin")
        .unwrap();
    let actions: Vec<_> = receipt["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        ["classify", "connect_wallet", "attach_evidence"],
        "{receipt}"
    );
    let cases = book.run(&["review", "cases", "--limit", "100"]);
    let case = cases["data"]["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["transaction_id"] == "payjoin")
        .unwrap();
    assert_eq!(
        case["supported_operations"],
        json!(["kind_override", "custody_component"]),
        "{case}"
    );

    // A generic deposit says nothing about the outside party's part.
    let (_, deposit) = book.plan("deposit");
    assert_eq!(
        held_after(&deposit),
        [
            pair("payjoin", "privacy_hop_unresolved"),
            pair("spend", "custody_basis_barrier")
        ]
    );

    let (artifact, income) = book.plan("income");
    assert_eq!(held_after(&income), Vec::<(String, String)>::new());
    let receipt = book.run(&[
        "review",
        "apply",
        "--artifact-file",
        artifact.to_str().unwrap(),
        "--idempotency-key",
        "payjoin-income",
    ]);
    assert_eq!(receipt["data"]["status"], "verified", "{receipt}");
    assert_eq!(book.held(), Vec::<(String, String)>::new());

    // The net 0.1 BTC is income at 50,000 EUR; the spend disposes of the
    // bought 0.50568205 BTC first (the input stayed in the wallet), then
    // 0.04431795 BTC of the Payjoin's lot.
    let db = book.db();
    let entries = |id: &str| -> Vec<(String, i64, String, String)> {
        let mut statement = db
            .prepare("SELECT entry_type,quantity,COALESCE(cost_basis_exact,''),COALESCE(gain_loss_exact,'') FROM journal_entries WHERE transaction_id=? ORDER BY entry_type")
            .unwrap();
        statement
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let payjoin = entries("payjoin");
    assert!(
        payjoin
            .iter()
            .any(|(kind, quantity, _, _)| kind == "income" && *quantity == 10_000_000_000),
        "{payjoin:?}"
    );
    let spend = entries("spend");
    let basis = 0.505_682_05 * 40_000.0 + 0.044_317_95 * 50_000.0;
    let disposal = spend
        .iter()
        .find(|(kind, ..)| kind == "disposal")
        .unwrap_or_else(|| panic!("{spend:?}"));
    assert_eq!(disposal.1.abs(), 55_000_000_000, "{spend:?}");
    let booked = disposal.2.parse::<f64>().unwrap().abs();
    assert!((booked - basis).abs() < 0.01, "{booked} != {basis}");
}

#[test]
fn another_row_of_the_same_payjoin_keeps_the_receipt_held() {
    let book = Book::new();
    // A second wallet of the book also has a row in the Payjoin, so the
    // receiving wallet's net delta is not all of the owner's part in it.
    let other = book.run(&[
        "wallets",
        "create",
        "--label",
        "Cold",
        "--kind",
        "address",
        "--chain",
        "bitcoin",
        "--network",
        "regtest",
        "--address",
        "bcrt1q6jdxw2rxx26m8xfqzhqvdhex4578vv7kc4x0m0",
    ]);
    let other = other["data"]["id"].as_str().unwrap().to_owned();
    book.insert(
        "sibling",
        &other,
        PAYJOIN,
        "2025-02-01T00:00:00Z",
        "outbound",
        1_000_000,
        "withdrawal",
        50_000,
        "{}",
    );
    // Still held when the sibling is classified, and when the user excluded
    // it: the loader drops excluded rows but reports their transaction.
    for excluded in [false, true] {
        book.db()
            .execute(
                "UPDATE transactions SET excluded=? WHERE id='sibling'",
                [excluded],
            )
            .unwrap();
        book.run(&["journals", "process"]);
        let (_, income) = book.plan("income");
        assert!(
            held_after(&income).contains(&pair("payjoin", "privacy_hop_unresolved")),
            "excluded={excluded}: {:?}",
            held_after(&income)
        );
        // Not provable apart from its classification, so neither the
        // quarantine page nor the review case offers classifying it.
        let page = book.run(&["journals", "quarantined"]);
        let receipt = page["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["transaction_id"] == "payjoin")
            .unwrap();
        assert!(
            !receipt["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action["kind"] == "classify"),
            "excluded={excluded}: {receipt}"
        );
        let cases = book.run(&["review", "cases", "--limit", "100"]);
        let case = cases["data"]["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["transaction_id"] == "payjoin")
            .unwrap();
        assert_eq!(
            case["supported_operations"],
            json!(["custody_component"]),
            "excluded={excluded}: {case}"
        );
    }
}
