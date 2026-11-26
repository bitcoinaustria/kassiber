//! Balance checkpoints through the CLI (`wallets checkpoints
//! list|add|remove|import|export`) and the daemon (`ui.wallets.checkpoints.*`)
//! on a processed synthetic book (docs/reference/balance-checkpoints.md).
//! Every command is local: the sandbox forbids egress.

use crate::support::Sandbox;
use serde_json::{json, Value};

const CLOCK: &str = "2026-10-31T12:00:00Z";

/// Five priced rows in one manual wallet: the wallet holds 0.30999 BTC after
/// 2024-04-01 (1 + 1 - 1.5 - 0.00001 fee + 0.01 - 0.2).
const BOOK: &str = "
INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020');
INSERT INTO profiles(id,workspace_id,label,gains_algorithm,tax_country,fiat_currency,created_at)
 VALUES('p','w','P','fifo','generic','EUR','2020');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at)
 VALUES('wallet','w','p','Wallet','manual','2020');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,
 amount,fee,kind,fiat_currency,fiat_rate,fiat_rate_exact,pricing_source_kind,pricing_quality,
 pricing_provider,raw_json,created_at) VALUES
 ('a','w','p','wallet','a','2024-01-01T00:00:00Z','inbound','BTC',100000000000,0,'buy','EUR',10000.0,'10000','manual','exact','reviewed-receipt','{}','2020'),
 ('b','w','p','wallet','b','2024-02-01T00:00:00Z','inbound','BTC',100000000000,0,'buy','EUR',20000.0,'20000','manual','exact','reviewed-receipt','{}','2020'),
 ('s','w','p','wallet','s','2024-03-01T00:00:00Z','outbound','BTC',150000000000,1000000,'sell','EUR',30000.0,'30000','manual','exact','reviewed-receipt','{}','2020'),
 ('i','w','p','wallet','i','2024-03-02T00:00:00Z','inbound','BTC',1000000000,0,'interest','EUR',30000.0,'30000','manual','exact','reviewed-receipt','{}','2020'),
 ('t','w','p','wallet','t','2024-04-01T00:00:00Z','outbound','BTC',20000000000,0,'sell','EUR',31000.5,'31000.5','manual','exact','reviewed-receipt','{}','2020');
INSERT OR REPLACE INTO settings VALUES('context_workspace','w'),('context_profile','p');
";

fn processed() -> Sandbox {
    let mut sandbox = Sandbox::book(BOOK);
    sandbox.journal_clock(CLOCK);
    let out = sandbox.machine(&["journals", "process"]);
    assert_eq!(out.code, 0, "{}", out.text());
    sandbox
}

fn ok(sandbox: &Sandbox, argv: &[&str]) -> Value {
    let out = sandbox.machine(argv);
    assert_eq!(out.code, 0, "{argv:?}: {}", out.text());
    out.json()["data"].clone()
}

/// The book's journal rows, which no checkpoint command may touch.
fn journals(sandbox: &Sandbox) -> Vec<String> {
    sandbox.query_strings(
        "SELECT posting_id||'|'||amount_msat FROM journal_quantity_postings ORDER BY posting_id",
    )
}

#[test]
fn the_cli_records_compares_exports_imports_and_removes() {
    let sandbox = processed();
    let before = journals(&sandbox);
    let matches = ok(
        &sandbox,
        &[
            "wallets",
            "checkpoints",
            "add",
            "--wallet",
            "Wallet",
            "--btc",
            "0.30999",
            "--date",
            "2024-04-30",
            "--note",
            "April statement",
        ],
    );
    assert_eq!(matches["status"], "matches", "{matches}");
    assert_eq!(matches["as_of"], "2024-04-30T23:59:59.999999Z");
    assert_eq!(matches["book_msat"], 30_999_000_000i64);
    let differs = ok(
        &sandbox,
        &[
            "wallets",
            "checkpoints",
            "add",
            "--wallet",
            "wallet",
            "--btc",
            "0.4",
            "--date",
            "2024-05-01T12:00:00+02:00",
        ],
    );
    assert_eq!(differs["status"], "differs");
    assert_eq!(differs["difference_msat"], 9_001_000_000i64);
    assert_eq!(differs["difference_btc"], "0.09001000");
    let early = ok(
        &sandbox,
        &[
            "wallets",
            "checkpoints",
            "add",
            "--wallet",
            "Wallet",
            "--btc",
            "1",
            "--date",
            "2023-12-31",
        ],
    );
    assert_eq!(
        (early["status"].clone(), early["reason"].clone()),
        (json!("unknown"), json!("before_coverage"))
    );

    // Fail closed on ambiguous or inexact input; nothing is stored.
    for (argv, code) in [
        (
            vec!["--btc", "0.000000000001", "--date", "2024-04-30"],
            "validation",
        ),
        (vec!["--btc", "-1", "--date", "2024-04-30"], "validation"),
        (
            vec!["--btc", "1", "--date", "2024-04-30T10:00:00"],
            "validation",
        ),
        (
            vec!["--btc", "1", "--date", "2024-04-30", "--asset", "USDT"],
            "validation",
        ),
    ] {
        let mut full = vec!["wallets", "checkpoints", "add", "--wallet", "Wallet"];
        full.extend(argv.iter().copied());
        let out = sandbox.machine(&full);
        assert_eq!(out.code, 1, "{full:?}: {}", out.text());
        assert_eq!(out.json()["error"]["code"], code, "{full:?}");
    }

    // The latest manual checkpoint is the one dated last.
    let list = ok(&sandbox, &["wallets", "checkpoints", "list"]);
    assert_eq!(list["journals"]["status"], "current");
    let latest = list["checkpoints"].as_array().unwrap();
    assert_eq!(latest.len(), 1, "{list}");
    assert_eq!(latest[0]["status"], "differs");
    assert_eq!(list["wallets"][0]["status"], "differs");
    let history = ok(
        &sandbox,
        &[
            "wallets",
            "checkpoints",
            "list",
            "--wallet",
            "Wallet",
            "--history",
        ],
    );
    assert_eq!(history["checkpoints"].as_array().unwrap().len(), 3);

    // Export to a new file, refuse to overwrite it, re-import it unchanged.
    let file = sandbox.path("checkpoints.csv");
    let file = file.to_str().unwrap();
    let exported = ok(
        &sandbox,
        &["wallets", "checkpoints", "export", "--file", file],
    );
    assert_eq!(exported["rows"], 3);
    let text = std::fs::read_to_string(file).unwrap();
    assert!(
        text.starts_with("date,wallet_id,wallet,btc_amount,asset,origin,note,checkpoint_id\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "2024-04-30T23:59:59.999999Z,wallet,Wallet,0.30999000,BTC,manual,April statement,"
        ),
        "{text}"
    );
    let again = sandbox.machine(&["wallets", "checkpoints", "export", "--file", file]);
    assert_eq!(again.json()["error"]["code"], "conflict");
    let imported = ok(
        &sandbox,
        &["wallets", "checkpoints", "import", "--file", file],
    );
    assert_eq!(
        (imported["created"].clone(), imported["unchanged"].clone()),
        (json!(0), json!(3))
    );
    let extra = sandbox.path("more.csv");
    std::fs::write(
        &extra,
        "date,wallet,btc_amount,note\n2024-06-30,Wallet,0.30999,June\n",
    )
    .unwrap();
    let imported = ok(
        &sandbox,
        &[
            "wallets",
            "checkpoints",
            "import",
            "--file",
            extra.to_str().unwrap(),
        ],
    );
    assert_eq!(imported["created"], 1);
    let list = ok(&sandbox, &["wallets", "checkpoints", "list"]);
    assert_eq!(list["checkpoints"][0]["status"], "matches");
    assert_eq!(
        list["checkpoints"][0]["as_of"],
        "2024-06-30T23:59:59.999999Z"
    );

    let id = differs["id"].as_str().unwrap();
    let removed = ok(&sandbox, &["wallets", "checkpoints", "remove", "--id", id]);
    assert_eq!(removed["removed"], true);
    let missing = sandbox.machine(&["wallets", "checkpoints", "remove", "--id", id]);
    assert_eq!(missing.json()["error"]["code"], "not_found");

    assert_eq!(
        journals(&sandbox),
        before,
        "checkpoints never touch journals"
    );
    let catalog = ok(
        &sandbox,
        &["commands", "describe", "wallets", "checkpoints"],
    );
    assert_eq!(catalog["count"], 5);
}

#[test]
fn the_daemon_serves_the_same_model_without_file_paths() {
    let sandbox = processed();
    let lines = sandbox.daemon(&[
        json!({"request_id": "add", "kind": "ui.wallets.checkpoints.add",
               "args": {"wallet": "wallet", "btc": "0.30999", "date": "2024-04-30"}}),
        json!({"request_id": "list", "kind": "ui.wallets.checkpoints.list", "args": {"wallet": "wallet"}}),
        json!({"request_id": "import", "kind": "ui.wallets.checkpoints.import",
               "args": {"csv": "date,wallet,btc_amount\n2024-05-31,Wallet,0.5\n", "source": "may.csv"}}),
        json!({"request_id": "path", "kind": "ui.wallets.checkpoints.import",
               "args": {"file": "/etc/passwd"}}),
        json!({"request_id": "export", "kind": "ui.wallets.checkpoints.export", "args": {}}),
        json!({"request_id": "latest", "kind": "ui.wallets.checkpoints.list", "args": {}}),
    ]);
    let by_id = |id: &str| -> Value {
        lines
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|value| value["request_id"] == id)
            .unwrap_or_else(|| panic!("{id}: {lines:?}"))
    };
    assert_eq!(by_id("add")["data"]["status"], "matches");
    assert_eq!(
        by_id("list")["data"]["checkpoints"][0]["book_msat"],
        30_999_000_000i64
    );
    assert_eq!(by_id("import")["data"]["created"], 1);
    // The renderer never hands the daemon a path.
    assert_eq!(by_id("path")["error"]["code"], "validation");
    let csv = by_id("export")["data"]["csv"].as_str().unwrap().to_owned();
    assert!(
        csv.contains("2024-05-31T23:59:59.999999Z,wallet,Wallet,0.50000000,BTC,manual,"),
        "{csv}"
    );
    let latest = by_id("latest");
    assert_eq!(latest["data"]["checkpoints"][0]["status"], "differs");
    assert_eq!(latest["data"]["counts"]["differs"], 1);
}
