//! The AI tool executor through the daemon's test-harness kinds
//! (`test.ai_tool`, `test.ai_consent_preview`, `test.ai_custody_record`) on
//! the empty, desktop and reports fixture books. The request lists the
//! Python parity tests built from the live registries are frozen under
//! `tests/black_box/ai/`; each session's records and the rows it changed
//! are pinned. Replaces the kill-holding tests of `tests/test_rust_ai_tools.py`.

use serde_json::{json, Value};

use crate::support::{number_hex, Live, Sandbox, Transcript};

pub const CLOCK: &str = "2026-03-04T05:06:07Z";
const DESKTOP: &str = include_str!("../../../transactions/tests/data/desktop.sql");
const REPORTS: &str = include_str!("../../../reports/tests/data/book.sql");

pub const ATTACHMENT_CSV: &str =
    "Date,Type,Amount,Fee\n2025-01-02,Buy,0.01,0\n2025-01-03,Sell,0.005,0\n";
pub const LEDGER_CSV: &str = "Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fee Amount,Fee Asset,Fiat Value,Note,Tx-ID\n\
Deposit,2025-01-02,0.01,BTC,,,,,,From the exchange,\n\
Withdrawal,2025-01-05,,,0.004,BTC,0.00001,BTC,,To cold storage,\n\
Deposit,2025-02-01,0.002,BTC,,,,,,Later,\n";

/// The parity harness's environment: fixed journal and analysis clocks and
/// ids, and the strict egress guard (any gated connection trips it).
pub fn sandbox(fixture: &str) -> Sandbox {
    let seed = match fixture {
        "empty" => "",
        "desktop" => DESKTOP,
        "reports" => REPORTS,
        other => panic!("no fixture {other}"),
    };
    let mut sandbox = Sandbox::book(seed);
    sandbox
        .set("KASSIBER_RUST_TEST_JOURNAL_TIME", CLOCK)
        .set("KASSIBER_RUST_TEST_JOURNAL_IDS", "sequential")
        .set("KASSIBER_RUST_TEST_ANALYSIS_IDS", "sequential")
        .set("KASSIBER_RUST_TEST_ANALYSIS_TIME", CLOCK)
        .set("KASSIBER_TEST_NO_EGRESS", "strict");
    sandbox
}

/// One frozen session list: `{fixtures, method, unchanged, requests}`.
pub fn sessions(text: &str) -> Vec<Value> {
    serde_json::from_str(text).unwrap()
}

/// The requests with `<BASE>` naming this sandbox's directory,
/// `<AI_FIXTURES>` the files frozen beside them, and
/// `request_id`s numbered as the harness numbered them.
pub fn requests(sandbox: &Sandbox, session: &Value) -> Vec<Value> {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/black_box/ai");
    let text = session["requests"]
        .to_string()
        .replace("<BASE>", &sandbox.base.to_string_lossy())
        .replace("<AI_FIXTURES>", &fixtures.to_string_lossy());
    let list: Vec<Value> = serde_json::from_str(&text).unwrap();
    list.into_iter()
        .enumerate()
        .map(|(index, mut request)| {
            request["request_id"] = json!(index.to_string());
            request
        })
        .collect()
}

pub fn transcript(name: &str) -> Transcript {
    let mut t = Transcript::new(name);
    // Handles are keyed by a per-process secret and the book's path.
    t.mask(|text| number_hex(text, "ca-ref:", 64));
    t.compact();
    // Dismissal expiries count from the wall clock.
    t.future_is_generated();
    t
}

/// The model-visible result of a `test.ai_tool` record.
pub fn tool_result(record: &Value) -> Value {
    if record["kind"] == "ai.chat.tool_result" {
        return record["data"].clone();
    }
    record["data"]["tool_result"]["data"].clone()
}

/// A read session: every record, and no authored table changed.
fn reads(
    t: &mut Transcript,
    fixture: &str,
    session: &Value,
    setup: impl Fn(&Sandbox),
) -> Vec<Value> {
    let sandbox = sandbox(fixture);
    setup(&sandbox);
    t.note(&format!("fixture {fixture}"));
    let before = sandbox.dump();
    let records = t.daemon(&sandbox, &requests(&sandbox, session));
    t.changes(&sandbox, &before, "after the reads");
    records
}

/// A write session on a fresh copy of the fixture, with the report clock
/// pinned: every record, then every changed row.
pub fn writes(
    t: &mut Transcript,
    fixture: &str,
    session: &Value,
    setup: &dyn Fn(&Sandbox),
) -> Vec<Value> {
    let mut sandbox = sandbox(fixture);
    sandbox.set("KASSIBER_RUST_TEST_REPORT_TIME", CLOCK);
    setup(&sandbox);
    t.note(&format!("fixture {fixture}"));
    let before = sandbox.dump();
    let records = t.daemon(&sandbox, &requests(&sandbox, session));
    if session["unchanged"] == true {
        // The local analysis index is a derived projection a preview may
        // refresh; nothing else may change.
        let authored = |dump: &str| -> Vec<String> {
            dump.lines()
                .filter(|l| !l.starts_with("chain_index_") && !l.starts_with('('))
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(authored(&sandbox.dump()), authored(&before));
    }
    t.changes(&sandbox, &before, "after the writes");
    records
}

fn fixtures(session: &Value) -> Vec<String> {
    session["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_owned())
        .collect()
}

fn place_attachment(sandbox: &Sandbox) {
    std::fs::write(sandbox.base.join("export.csv"), ATTACHMENT_CSV).unwrap();
}

/// Run every session of a frozen file; `compare` sessions as reads,
/// `compare_writes` sessions as writes. Sessions run on their own books,
/// a few at a time; the transcript keeps their order.
fn run_all(
    t: &mut Transcript,
    text: &str,
    setup: impl Fn(&Sandbox) + Sync,
) -> Vec<(String, Vec<Value>)> {
    let jobs: Vec<(String, Value)> = sessions(text)
        .into_iter()
        .flat_map(|session| {
            fixtures(&session)
                .into_iter()
                .map(move |fixture| (fixture, session.clone()))
        })
        .collect();
    let mut out = Vec::new();
    for batch in jobs.chunks(6) {
        let parts: Vec<(Transcript, Vec<Value>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|(fixture, session)| {
                    let mut part = t.part();
                    let setup = &setup;
                    scope.spawn(move || {
                        let records = if session["method"] == "compare" {
                            reads(&mut part, fixture, session, setup)
                        } else {
                            writes(&mut part, fixture, session, setup)
                        };
                        (part, records)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for ((fixture, _), (part, records)) in batch.iter().zip(parts) {
            t.append(part);
            out.push((fixture.clone(), records));
        }
    }
    out
}

#[test]
fn test_read_tools_match_python_on_every_fixture() {
    let mut t = transcript("ai_tools/read_tools");
    let results = run_all(
        &mut t,
        include_str!("ai/read_tools_match_python_on_every_fixture.json"),
        place_attachment,
    );
    assert_eq!(results.len(), 3);
    for (fixture, records) in &results {
        let compared = records
            .iter()
            .filter(|r| {
                let result = tool_result(r);
                result["reason"] != "consent_required" && result["reason"] != "not_implemented"
            })
            .count();
        assert!(compared > 200, "{fixture}: {compared}");
    }
    t.finish();
}

#[test]
fn test_formerly_refused_tools_match_python() {
    let mut t = transcript("ai_tools/formerly_refused_tools");
    let results = run_all(
        &mut t,
        include_str!("ai/formerly_refused_tools_match_python.json"),
        |_| {},
    );
    for (_, records) in &results {
        for record in records {
            assert_ne!(tool_result(record)["reason"], "not_implemented", "{record}");
        }
    }
    t.finish();
}

#[test]
fn test_mutating_tools_match_python_on_every_fixture() {
    let mut t = transcript("ai_tools/mutating_tools");
    let results = run_all(
        &mut t,
        include_str!("ai/mutating_tools_match_python_on_every_fixture.json"),
        |_| {},
    );
    assert!(results.iter().all(|(_, records)| !records.is_empty()));
    t.finish();
}

#[test]
fn test_custody_writes_are_audited_or_rolled_back() {
    let mut t = transcript("ai_tools/custody_writes_audited_or_rolled_back");
    run_all(
        &mut t,
        include_str!("ai/custody_writes_are_audited_or_rolled_back.json"),
        |_| {},
    );
    t.finish();
}

#[test]
fn test_ai_authored_writes_match_python() {
    let mut t = transcript("ai_tools/ai_authored_writes");
    run_all(
        &mut t,
        include_str!("ai/ai_authored_writes_match_python.json"),
        |_| {},
    );
    t.finish();
}

#[test]
fn test_custody_audit_records_match_python() {
    let mut t = transcript("ai_tools/custody_audit_records");
    run_all(
        &mut t,
        include_str!("ai/custody_audit_records_match_python.json"),
        |_| {},
    );
    t.finish();
}

#[test]
fn test_attachment_import_plans_match_python() {
    let mut t = transcript("ai_tools/attachment_import_plans");
    let results = run_all(
        &mut t,
        include_str!("ai/attachment_import_plans_match_python.json"),
        place_attachment,
    );
    let mut outputs = String::new();
    for (_, records) in &results {
        assert!(records.len() > 40);
        for record in records {
            assert_ne!(tool_result(record)["reason"], "not_implemented");
            outputs.push_str(record["data"]["output"].as_str().unwrap_or_default());
        }
    }
    for code in [
        "target_not_file_wallet",
        "row_kinds_from_amount_sign",
        "wallet_label_taken",
    ] {
        assert!(outputs.contains(&format!("\"code\":\"{code}\"")), "{code}");
    }
    t.finish();
}

#[test]
fn test_unreadable_wallet_config_refuses_wallet_sync_typed() {
    let sessions = sessions(include_str!(
        "ai/unreadable_wallet_config_refuses_wallet_sync_typed.json"
    ));
    assert_eq!(sessions.len(), 1);
    let mut t = transcript("ai_tools/unreadable_wallet_config");
    for stored in ["bad", "[]", "null", "\"text\""] {
        t.note(&format!("config_json={stored}"));
        let records = writes(&mut t, "desktop", &sessions[0], &|sandbox| {
            sandbox.sql_raw(&format!(
                "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
                 VALUES('bad','w','p','Bad','custom','{}','2025-01-01T00:00:00Z')",
                stored.replace('\'', "''")
            ));
        });
        assert_eq!(records.len(), 3);
        for record in &records {
            assert!(
                record.to_string().contains("invalid_wallet_config"),
                "{record}"
            );
        }
    }
    t.finish();
}

const REMOTE: (&str, bool) = ("remote", false);
const LOCAL: (&str, bool) = ("local", true);

/// The ids of the fixture's frozen scope and rows.
fn ids(fixture: &str) -> (&'static str, &'static str, &'static str) {
    match fixture {
        "desktop" => ("w", "p", "c"),
        "reports" => ("ws", "p", "no-tax"),
        other => panic!("no ids for {other}"),
    }
}

fn state(provider: (&str, bool), fixture: &str, extra: Value) -> Value {
    let (ws, profile, _) = ids(fixture);
    let mut state = json!({"provider_kind": provider.0, "provider_on_device": provider.1,
        "scope_workspace_id": ws, "scope_profile_id": profile});
    for (key, value) in extra.as_object().unwrap() {
        state[key] = value.clone();
    }
    state
}

fn tool(name: &str, arguments: Value, call_id: &str, state: Value) -> Value {
    let mut state = state;
    state["consented_call_ids"] = json!([call_id]);
    json!({"kind": "test.ai_tool", "args": {"name": name, "arguments": arguments,
        "call_id": call_id, "state": state}})
}

/// The model-visible output of a record, parsed.
fn output(record: &Value) -> Value {
    serde_json::from_str(record["data"]["output"].as_str().unwrap()).unwrap()
}

#[test]
fn test_review_apply_flow_matches_python() {
    // The desktop book holds incompatible network histories, which block
    // review: it pins the refusals, the reports book the applied flow.
    let mut t = transcript("ai_tools/review_apply_flow");
    for fixture in ["desktop", "reports"] {
        t.note(&format!("fixture {fixture}"));
        let sandbox = sandbox(fixture);
        let before = sandbox.dump();
        let (_, _, quarantined) = ids(fixture);
        let remote = state(REMOTE, fixture, json!({}));
        let mut live = Live::start(&sandbox);
        let cases = output(&live.send(
            &mut t,
            &sandbox,
            tool("ui_review_cases", json!({}), "call_cases", remote.clone()),
        ));
        let ok = cases["ok"] == true;
        let version = if ok {
            cases["envelope"]["data"]["input_version"].clone()
        } else {
            json!(0)
        };
        let quarantine_ids: Vec<Value> = if ok {
            cases["envelope"]["data"]["cases"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|c| {
                    c["case_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("quarantine:"))
                })
                .map(|c| c["case_id"].clone())
                .take(1)
                .collect()
        } else {
            Vec::new()
        };
        let case_ids = if quarantine_ids.is_empty() {
            json!(["quarantine:none"])
        } else {
            json!(quarantine_ids)
        };
        // Before the plan changes the input version: the request's digest
        // covers the stored, redacted explanation.
        let requested = live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_review_request_input",
                json!({"action": "attach_evidence", "case_ids": case_ids,
                    "expected_input_version": version,
                    "explanation": "Attach the bank statement from /home/user/statements/2025.pdf"}),
                "call_input",
                remote.clone(),
            ),
        );
        let planned = output(&live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_review_plan",
                json!({"operations": [{"type": "exclude", "transaction_id": quarantined,
                    "reason": "Spam dust"}], "expected_input_version": version}),
                "call_plan",
                remote.clone(),
            ),
        ));
        let artifact = if planned["ok"] == true {
            planned["envelope"]["data"].clone()
        } else {
            json!({"operations": []})
        };
        let apply = json!({"artifact": artifact, "idempotency_key": "ai-review-1"});
        let mut preview_state = remote.clone();
        preview_state
            .as_object_mut()
            .unwrap()
            .remove("consented_call_ids");
        let preview = json!({"kind": "test.ai_consent_preview", "args": {"name": "ui_review_apply",
            "arguments": apply, "call_id": "call_apply", "state": preview_state}});
        live.send(&mut t, &sandbox, preview.clone());
        let applied = live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_review_apply",
                apply.clone(),
                "call_apply",
                remote.clone(),
            ),
        );
        live.send(&mut t, &sandbox, preview);
        live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_review_receipt",
                json!({"idempotency_key": "ai-review-1"}),
                "call_receipt",
                remote,
            ),
        );
        live.finish();
        assert_eq!(
            tool_result(&applied)["ok"],
            json!(fixture == "reports"),
            "{applied}"
        );
        assert_eq!(
            tool_result(&requested)["ok"],
            json!(fixture == "reports"),
            "{requested}"
        );
        t.changes(&sandbox, &before, "after the flow");
    }
    t.finish();
}

#[test]
fn test_attachment_import_flow_matches_python() {
    let mut t = transcript("ai_tools/attachment_import_flow");
    // Plans over what the apply created (the wallet, the run) hash those
    // generated ids.
    t.mask(|text| {
        let text = number_hex(text, "plan_digest\":\"", 64);
        let text = number_hex(&text, "plan_digest\\\":\\\"", 64);
        // An imported row's fingerprint hashes its new wallet's id.
        number_hex(&text, "fingerprint=t:\"", 64)
    });
    for fixture in ["desktop", "reports"] {
        t.note(&format!("fixture {fixture}"));
        let sandbox = sandbox(fixture);
        let export = sandbox.base.join("Kraken ledger.csv");
        std::fs::write(&export, LEDGER_CSV).unwrap();
        let files = json!({"chat_files": [{"id": "file-1",
            "source_file": export.to_string_lossy(), "filename": "Kraken ledger.csv",
            "label": "Exchange ledger", "kind": "delimited", "new": true, "token": "t"}]});
        let local = state(LOCAL, fixture, files.clone());
        let remote = state(REMOTE, fixture, files);
        let target = json!({"new_wallet": {"label": "Exchange import"}});
        let before = sandbox.dump();
        let mut live = Live::start(&sandbox);
        let restricted = live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_wallets_import_attachment_plan",
                target.clone(),
                "call_restricted",
                remote.clone(),
            ),
        );
        let planned = output(&live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_wallets_import_attachment_plan",
                target.clone(),
                "call_plan",
                local.clone(),
            ),
        ));
        let mut approved = target.clone();
        if planned["ok"] == true {
            for (key, value) in planned["envelope"]["data"]["apply_with"]
                .as_object()
                .unwrap()
            {
                approved[key] = value.clone();
            }
        }
        live.send(
            &mut t,
            &sandbox,
            json!({"kind": "test.ai_consent_preview", "args": {
                "name": "ui_wallets_import_attachment_apply", "arguments": approved,
                "call_id": "call_apply", "state": local}}),
        );
        let mut tampered = approved.clone();
        tampered["plan_digest"] = json!("0".repeat(64));
        let stale = live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_wallets_import_attachment_apply",
                tampered,
                "call_tampered",
                local.clone(),
            ),
        );
        // The file changes on disk after the user approved its plan.
        std::fs::write(&export, LEDGER_CSV.replace("0.01,BTC", "0.02,BTC")).unwrap();
        let changed = live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_wallets_import_attachment_apply",
                approved.clone(),
                "call_changed",
                local.clone(),
            ),
        );
        std::fs::write(&export, LEDGER_CSV).unwrap();
        let applied = output(&live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_wallets_import_attachment_apply",
                approved.clone(),
                "call_apply",
                local.clone(),
            ),
        ));
        let again = live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_wallets_import_attachment_apply",
                approved,
                "call_again",
                local.clone(),
            ),
        );
        live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_wallets_import_attachment_plan",
                json!({"wallet": "Exchange import"}),
                "call_present",
                local.clone(),
            ),
        );
        live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_wallets_import_attachment_plan",
                json!({"wallet": "Exchange import"}),
                "call_present_r",
                remote,
            ),
        );
        live.send(
            &mut t,
            &sandbox,
            tool("ui_imports_list", json!({}), "call_runs", local.clone()),
        );
        let batch = if applied["ok"] == true {
            applied["envelope"]["data"]["import_batch_id"].clone()
        } else {
            json!("missing")
        };
        live.send(
            &mut t,
            &sandbox,
            tool(
                "ui_imports_rollback_plan",
                json!({"batch": batch}),
                "call_rollback",
                local,
            ),
        );
        live.finish();
        t.changes_blanked(&sandbox, &before, "after the flow");
        if fixture == "desktop" {
            // The desktop book has no default account to hold a new wallet.
            assert!(planned.to_string().contains("Account 'treasury' not found"));
            continue;
        }
        assert_eq!(planned["ok"], true, "{planned}");
        assert_eq!(planned["envelope"]["data"]["blockers"], json!([]));
        assert_eq!(output(&stale)["reason"], "import_plan_stale");
        assert_eq!(output(&changed)["reason"], "attachment_changed");
        assert_eq!(applied["ok"], true, "{applied}");
        assert_eq!(applied["envelope"]["data"]["rows"]["to_import"], 3);
        assert_eq!(output(&again)["reason"], "import_plan_stale");
        // A restricted provider never sees the file's values.
        let shown = restricted["data"]["output"].as_str().unwrap();
        for value in ["Kraken ledger", "2025-01-02", "1000000000"] {
            assert!(!shown.contains(value), "{value}");
        }
    }
    t.finish();
}
