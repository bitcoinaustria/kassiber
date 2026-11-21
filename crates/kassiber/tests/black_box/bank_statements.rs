//! Bank statements end to end through the CLI and MCP: every assistant and
//! external-agent tool that can carry bank evidence shows it only projected
//! (docs/reference/bank-statements.md#privacy). Synthetic sentinels mark
//! what must never reach a model: a private counterparty, the remittance
//! text, the bank's references and full IBANs.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;

use serde_json::{json, Value};

use crate::support::Sandbox;

const BOOK: &str = "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z');
INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Book','EUR','2025-01-01T00:00:00Z');
INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('kraken','w','p','Kraken','kraken','2025-01-01T00:00:00Z');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,fiat_currency,fiat_value_exact,created_at) VALUES
 ('buy1','w','p','kraken','f1','BTC','inbound',2000000000,0,'2025-03-05T10:00:00Z','buy','EUR','1000.00','2025-01-01T00:00:00Z'),
 ('buy2','w','p','kraken','f2','BTC','inbound',10000000000,0,'2025-03-12T10:00:00Z','buy','EUR','500.00','2025-01-01T00:00:00Z');";

const STATEMENT: &str = "Buchungsdatum;Empfänger;IBAN;Verwendungszweck;Bankreferenz;Betrag;Währung
03.03.2025;Sentinel Holder GmbH;AT61 1904 3002 3457 3201;Kraken deposit SENTINEL-REMIT-1;SENTINEL-REF-1;-1.000,00;EUR
10.03.2025;Sentinel Second Person;DE89 3704 0044 0532 0130 00;Kraken top-up SENTINEL-REMIT-2;SENTINEL-REF-2;-500,00;EUR
";

const SENTINELS: &[&str] = &[
    "Sentinel Holder",
    "Sentinel Second",
    "SENTINEL-REMIT",
    "SENTINEL-REF",
    "AT611904300234573201",
    "1904 3002",
    "DE89370400440532013000",
];

fn serve(sandbox: &Sandbox, messages: &[Value]) -> Vec<String> {
    let root = sandbox.root.to_string_lossy().into_owned();
    let mut command = sandbox.command();
    command
        .args([
            "--data-root",
            &root,
            "mcp",
            "serve",
            "--workspace",
            "w",
            "--profile",
            "p",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    for message in messages {
        stdin.write_all(message.to_string().as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
    }
    drop(stdin);
    let lines: Vec<String> = stdout.lines().map(Result::unwrap).collect();
    assert!(child.wait().unwrap().success());
    lines
}

fn mcp_ready(sandbox: &mut Sandbox) {
    use std::os::unix::fs::PermissionsExt;
    let runtime = sandbox.path("rt");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let access = sandbox.path("agents.json");
    std::fs::write(
        &access,
        r#"{"agent_access": "full", "ai_features_enabled": null, "grant": "accounting_decisions", "mcp_enabled": true, "schema_version": 1}"#,
    )
    .unwrap();
    std::fs::set_permissions(&access, std::fs::Permissions::from_mode(0o600)).unwrap();
    sandbox
        .set(
            "KASSIBER_OPERATOR_RUNTIME_DIR",
            runtime.to_string_lossy().into_owned(),
        )
        .set("KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR", "1")
        .set("KASSIBER_TEST_NO_EGRESS", "strict")
        .set("KASSIBER_RUST_TEST_REPORT_TIME", "2026-03-04T05:06:07Z");
}

#[test]
fn every_tool_that_can_carry_bank_evidence_projects_it() {
    let mut sandbox = Sandbox::book(BOOK);
    mcp_ready(&mut sandbox);
    // The file is named after the account's IBAN: the name is never kept.
    let file = sandbox.path("statement_AT611904300234573201.csv");
    std::fs::write(&file, STATEMENT).unwrap();
    let import = [
        "documents",
        "import-statement",
        "--file",
        file.to_str().unwrap(),
        "--account",
        "AT611904300234573201",
    ];
    let imported = sandbox.machine(&import).json();
    assert_eq!(imported["data"]["lines_new"], 2, "{imported}");
    let names =
        sandbox.query_strings("SELECT coalesce(original_filename,'')||label FROM attachments");
    assert!(!names.concat().contains("1904300234"), "{names:?}");
    let listed = sandbox
        .machine(&["source-funds", "bank-lines", "list", "--direction", "out"])
        .json();
    // Each leaf answers under its own kind, as `commands describe` advertises.
    assert_eq!(listed["kind"], "source-funds.bank-lines.list", "{listed}");
    let first = listed["data"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["amount"] == "-1000.00")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let accepted = sandbox
        .machine(&[
            "source-funds",
            "bank-lines",
            "accept",
            "--line",
            &first,
            "--transaction",
            "buy1",
        ])
        .json();
    assert_eq!(accepted["data"]["state"], "accepted", "{accepted}");
    assert_eq!(accepted["kind"], "source-funds.bank-lines.accept");
    // The CLI is the user's own terminal: it shows the counterparty and the
    // bank reference, with IBANs masked.
    let report = sandbox
        .machine(&["reports", "source-funds", "--target-transaction", "buy1"])
        .text();
    assert!(
        report.contains("Sentinel Holder GmbH") && report.contains("SENTINEL-REF-1"),
        "{report}"
    );
    assert!(
        report.contains("AT** **** **** **** 3201") && !report.contains("AT611904300234573201"),
        "{report}"
    );

    let meta = json!({"io.modelcontextprotocol/clientCapabilities": {}, "io.modelcontextprotocol/protocolVersion": "2026-07-28"});
    let call = |id: &str, name: &str, arguments: Value| json!({"id": id, "jsonrpc": "2.0", "method": "tools/call", "params": {"_meta": meta, "name": name, "arguments": arguments}});
    let tools = [
        (
            "preview",
            "source_funds_preview",
            json!({"target_transaction": "buy1"}),
        ),
        (
            "context",
            "source_funds_review_context",
            json!({"target_transaction": "buy1"}),
        ),
        ("suggest", "source_funds_bank_lines_suggest", json!({})),
        ("sources", "source_funds_sources_list", json!({})),
        ("links", "source_funds_links_list", json!({})),
        ("attachments", "attachments_list", json!({})),
        ("evidence", "source_funds_evidence_list", json!({})),
        (
            "audit",
            "audit_evidence_summary",
            json!({"transaction": "buy1"}),
        ),
        ("documents", "documents_list", json!({})),
    ];
    let mut messages = vec![
        json!({"id": "init", "jsonrpc": "2.0", "method": "initialize", "params": {"capabilities": {}, "protocolVersion": "2025-06-18"}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    ];
    messages.extend(
        tools
            .iter()
            .map(|(id, name, args)| call(id, name, args.clone())),
    );
    let lines = serve(&sandbox, &messages);
    for (id, name, _) in &tools {
        let answer = lines
            .iter()
            .find(|l| l.contains(&format!("\"id\":\"{id}\"")))
            .unwrap_or_else(|| panic!("{name}: no answer in {lines:?}"));
        assert!(!answer.contains("\"isError\":true"), "{name}: {answer}");
        for sentinel in SENTINELS {
            assert!(
                !answer.contains(sentinel),
                "{name} leaked {sentinel}: {answer}"
            );
        }
        if *id == "preview" || *id == "context" {
            assert!(
                answer.contains("bank_evidence") && answer.contains("AT** **** **** **** 3201"),
                "{name}: {answer}"
            );
        }
        if *id == "suggest" {
            assert!(
                answer.contains("-500.00") && answer.contains("Kraken"),
                "{name}: {answer}"
            );
        }
    }
}
