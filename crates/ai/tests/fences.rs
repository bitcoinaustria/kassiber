//! The R6-5 privacy fences and the executor's gates, over a fake book that
//! counts every database visit.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use kassiber_ai::consent::{once_only, ConsentState};
use kassiber_ai::executor::{
    execute_read_only, is_advertised, model_output, Book, ParsedCall, ReadHost, Runtime,
};
use kassiber_ai::select::{build_responses_tools, on_device_only, strip_on_device_only};
use kassiber_ai::{get_tool, tools, KindClass, ToolError};
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;

/// A book on a fake request loop: records each visit and answers reads.
struct FakeBook {
    visits: usize,
    reads: Vec<String>,
    in_scope: bool,
    context: (Value, Value),
    payload: Option<Value>,
    maintained: Option<Value>,
}

impl Book for FakeBook {
    fn database_in_scope(&mut self, _data_root: &str) -> Result<bool, ToolError> {
        Ok(self.in_scope)
    }
    fn context(&mut self) -> Result<(Value, Value), ToolError> {
        Ok(self.context.clone())
    }
    fn maintain_for_read(&mut self, _sync_attempted: bool) -> Result<Option<Value>, ToolError> {
        Ok(self.maintained.clone())
    }
    fn active_profile(&mut self) -> Result<String, ToolError> {
        Ok(self.profile())
    }
    fn read(
        &mut self,
        kind: &str,
        _arguments: &Value,
        source: &'static str,
    ) -> Result<Value, ToolError> {
        self.reads.push(format!("{kind}:{source}"));
        Ok(self
            .payload
            .clone()
            .unwrap_or_else(|| dict([("value", s("ok"))])))
    }
}

impl FakeBook {
    fn profile(&self) -> String {
        "p1".to_owned()
    }
}

impl ReadHost for FakeBook {
    fn on_request_loop(
        &mut self,
        work: &mut dyn FnMut(&mut dyn Book) -> Result<Value, ToolError>,
    ) -> Result<Value, ToolError> {
        self.visits += 1;
        work(self)
    }
}

fn book() -> FakeBook {
    FakeBook {
        visits: 0,
        reads: Vec::new(),
        in_scope: true,
        context: (s("w1"), s("p1")),
        payload: None,
        maintained: None,
    }
}

fn run(
    book: &mut FakeBook,
    state: &mut Value,
    name: &str,
    arguments: Value,
    planned: bool,
) -> Value {
    let call = ParsedCall::new("call_1", name, arguments);
    let mut runtime = Runtime {
        data_root: "/data",
        state,
    };
    execute_read_only(&call, &mut runtime, book, planned)
}

fn reason(result: &Value) -> Option<String> {
    match get(result, "reason") {
        Some(Value::Str(reason)) => Some(reason.as_str().unwrap().to_owned()),
        _ => None,
    }
}

fn empty() -> Value {
    Value::Dict(Vec::new())
}

#[test]
fn only_tools_advertised_this_turn_run() {
    let mut fake = book();
    let mut state = dict([("advertised_tools", Value::List(vec![s("status")]))]);
    let denied = run(&mut fake, &mut state, "ui_rates_summary", empty(), false);
    assert_eq!(reason(&denied).as_deref(), Some("tool_not_advertised"));
    assert_eq!(fake.visits, 0);
    // The planner's own reads are checked against the advertised list by
    // the auto-read runner, not here.
    let planned = run(&mut fake, &mut state, "ui.rates.summary", empty(), true);
    assert_eq!(get(&planned, "ok"), Some(&Value::Bool(true)));
    let advertised = run(&mut fake, &mut state, "status", empty(), false);
    assert_eq!(get(&advertised, "ok"), Some(&Value::Bool(true)));
    assert_eq!(fake.visits, 2);
    // Without a list, direct callers keep the catalog allowlist.
    assert!(is_advertised(get_tool("status").unwrap(), &empty()));
}

#[test]
fn mutating_and_unknown_tools_never_reach_the_read_executor() {
    let mut fake = book();
    let mut state = empty();
    for name in [
        "ui.wallets.sync",
        "ui.review.apply",
        "nonexistent",
        "",
        "ui_transfers_pair",
    ] {
        let result = run(&mut fake, &mut state, name, empty(), false);
        assert_eq!(
            reason(&result).as_deref(),
            Some("tool_not_allowed"),
            "{name}"
        );
    }
    assert_eq!(fake.visits, 0);
}

#[test]
fn egressing_reads_need_a_consent_bound_to_the_call() {
    let arguments = || dict([("connection", s("node"))]);
    for name in [
        "ui.reports.lightning_profitability",
        "ui.connections.node.snapshot",
    ] {
        let mut fake = book();
        let mut state = empty();
        let result = run(&mut fake, &mut state, name, arguments(), false);
        assert_eq!(
            reason(&result).as_deref(),
            Some("consent_required"),
            "{name}"
        );
        // A consent for another call does not authorize this one.
        let mut other = dict([("consented_call_ids", Value::List(vec![s("call_2")]))]);
        let result = run(&mut fake, &mut other, name, arguments(), false);
        assert_eq!(
            reason(&result).as_deref(),
            Some("consent_required"),
            "{name}"
        );
        // Bound consent reaches the tool, which has no native route yet.
        let mut bound = dict([("consented_call_ids", Value::List(vec![s("call_1")]))]);
        let result = run(&mut fake, &mut bound, name, arguments(), false);
        assert_eq!(
            reason(&result).as_deref(),
            Some("not_implemented"),
            "{name}"
        );
        assert_eq!(fake.visits, 0, "{name}");
    }
}

#[test]
fn custody_reads_stay_with_local_providers() {
    let mut fake = book();
    for (kind, allowed) in [("remote", false), ("local", true)] {
        let mut state = dict([("provider_kind", s(kind))]);
        let result = run(
            &mut fake,
            &mut state,
            "ui.custody.gaps.list",
            empty(),
            false,
        );
        assert_eq!(
            reason(&result).as_deref() == Some("local_provider_required"),
            !allowed,
            "{kind}"
        );
    }
    assert_eq!(fake.reads, ["ui.custody.gaps.list:ai_tool"]);
}

#[test]
fn on_device_only_tools_are_never_offered_to_other_providers() {
    let all = build_responses_tools(None, None, Some("full")).unwrap();
    let remote = strip_on_device_only(all.clone(), false);
    assert!(remote
        .iter()
        .all(|tool| !on_device_only(tool.provider_name())));
    let stripped: Vec<&str> = all
        .iter()
        .filter(|tool| on_device_only(tool.provider_name()))
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(stripped.len(), 11);
    assert_eq!(strip_on_device_only(all.clone(), true).len(), all.len());
    // The executor refuses them too: after the scope check, as Python does,
    // and before any analysis read or reference.
    let mut fake = book();
    let mut state = dict([("provider_on_device", Value::Bool(false))]);
    let acquire = dict([
        ("backend", s("core")),
        ("subject", s("x")),
        ("chain", s("bitcoin")),
        ("network", s("main")),
        ("direction", s("backward")),
    ]);
    for (name, arguments) in [
        (
            "ui.chain_analysis.sources.list",
            dict([("limit", Value::from(1))]),
        ),
        ("ui.chain_analysis.acquire.plan", acquire),
    ] {
        let result = run(&mut fake, &mut state, name, arguments, true);
        assert_eq!(
            reason(&result).as_deref(),
            Some("local_provider_required"),
            "{name}: {result:?}"
        );
    }
    assert!(fake.reads.is_empty());
}

#[test]
fn no_shell_filesystem_secret_or_configuration_tool_exists() {
    const FORBIDDEN: [&str; 14] = [
        "shell",
        "exec",
        "dispatch",
        "filesystem",
        "cli",
        "descriptor",
        "xpub",
        "secret",
        "passphrase",
        "reveal",
        "keychain",
        "providers",
        "unlock",
        "settings.",
    ];
    for tool in tools() {
        let kind = tool.daemon_kind.as_deref().unwrap_or("");
        for word in FORBIDDEN {
            assert!(
                !tool.name.contains(word) && !kind.contains(word),
                "{} names {word}",
                tool.name
            );
        }
        assert!(
            kind.is_empty() || kind == "status" || kind.starts_with("ui."),
            "{kind}"
        );
        for prefix in ["ui.backends.", "ui.wallets."] {
            if kind.starts_with(prefix) {
                assert!(
                    [
                        "ui.backends.list",
                        "ui.wallets.list",
                        "ui.wallets.utxos",
                        "ui.wallets.identify",
                        "ui.wallets.analyze_file",
                        "ui.wallets.sync"
                    ]
                    .contains(&kind),
                    "{kind} configures sources"
                );
            }
        }
    }
}

#[test]
fn results_are_redacted_before_they_leave_the_executor() {
    let mut fake = book();
    fake.payload = Some(dict([
        (
            "note",
            s("see https://private.example/x?token=a and /Users/alice/tax/file.pdf"),
        ),
        ("api_token", s("sk-abcdef123456")),
        ("debug", s("Traceback")),
        ("descriptor", s("wpkh(xpub...)")),
    ]));
    let mut state = empty();
    let result = run(&mut fake, &mut state, "ui.rates.summary", empty(), false);
    let output = model_output(&result);
    for private in [
        "private.example",
        "/Users/alice",
        "sk-abcdef",
        "Traceback",
        "xpub",
    ] {
        assert!(!output.contains(private), "{private} in {output}");
    }
    assert!(output.contains("<redacted-url>") && output.contains("<redacted-path>"));
}

#[test]
fn scoped_reads_check_the_frozen_project_and_book() {
    let mut fake = book();
    fake.in_scope = false;
    let mut state = empty();
    let result = run(&mut fake, &mut state, "ui.rates.summary", empty(), false);
    assert_eq!(reason(&result).as_deref(), Some("stale_context"));
    let mut fake = book();
    let mut state = dict([
        ("scope_workspace_id", s("w1")),
        ("scope_profile_id", s("p2")),
    ]);
    let result = run(&mut fake, &mut state, "ui.rates.summary", empty(), false);
    assert_eq!(reason(&result).as_deref(), Some("stale_context"));
    assert!(fake.reads.is_empty());
    let mut state = dict([
        ("scope_workspace_id", s("w1")),
        ("scope_profile_id", s("p1")),
    ]);
    let result = run(&mut fake, &mut state, "ui.rates.summary", empty(), false);
    assert_eq!(get(&result, "ok"), Some(&Value::Bool(true)));
}

#[test]
fn refresh_reads_report_journal_processing_unless_disabled() {
    let mut fake = book();
    fake.maintained = Some(dict([("processed_transactions", Value::from(2))]));
    let mut state = empty();
    let result = run(&mut fake, &mut state, "ui.reports.summary", empty(), false);
    assert!(get(&result, "auto_journal_process").is_some());
    assert!(get(&state, "auto_journal_process").is_some());
    let mut state = dict([("read_maintenance", s("disabled"))]);
    let result = run(&mut fake, &mut state, "ui.reports.summary", empty(), false);
    assert!(get(&result, "auto_journal_process").is_none());
    let result = run(&mut fake, &mut empty(), "ui.rates.summary", empty(), false);
    assert!(get(&result, "auto_journal_process").is_none());
}

#[test]
fn refused_reads_never_touch_the_book() {
    let mut fake = book();
    let mut state = empty();
    for tool in tools()
        .iter()
        .filter(|tool| tool.kind_class == KindClass::ReadOnly)
    {
        if kassiber_ai::executor::refused_read(&tool.name).is_some() && !tool.egresses {
            let result = run(&mut fake, &mut state, &tool.name, empty(), true);
            let reason = reason(&result);
            assert!(
                matches!(
                    reason.as_deref(),
                    Some("not_implemented") | Some("validation")
                ),
                "{}: {result:?}",
                tool.name
            );
        }
    }
    assert_eq!(fake.visits, 0);
}

#[test]
fn once_only_grants_are_never_remembered() {
    let state = Arc::new(ConsentState::default());
    let cancelled = AtomicBool::new(false);
    for (tool, remembered) in [
        ("ui.review.apply", false),
        ("ui.maintenance.configure", false),
        ("ui.wallets.sync", true),
    ] {
        assert_eq!(once_only(tool), !remembered);
        let waiter = {
            let state = Arc::clone(&state);
            let tool = tool.to_owned();
            std::thread::spawn(move || {
                state.wait("c1", &tool, &AtomicBool::new(false), Duration::from_secs(5))
            })
        };
        while !state.record("c1", "allow_session") {
            std::thread::sleep(Duration::from_millis(1));
        }
        let decision = waiter.join().unwrap();
        assert_eq!(
            decision,
            if remembered {
                "allow_session"
            } else {
                "allow_once"
            }
        );
        assert_eq!(state.has_session_allow(tool), remembered, "{tool}");
        let again = state.wait("c2", tool, &cancelled, Duration::from_millis(30));
        assert_eq!(
            again,
            if remembered {
                "allow_session"
            } else {
                "consent_timeout"
            },
            "{tool}"
        );
    }
}

#[test]
fn consent_waits_end_on_decision_cancel_or_timeout() {
    let state = ConsentState::default();
    assert!(!state.record("unknown", "allow_once"), "nobody waits on it");
    state.expect("early");
    assert!(
        state.record("early", "deny"),
        "an expected call takes a decision before the wait"
    );
    assert_eq!(
        state.wait(
            "early",
            "ui.wallets.sync",
            &AtomicBool::new(false),
            Duration::from_secs(1)
        ),
        "deny"
    );
    assert_eq!(
        state.wait(
            "x",
            "ui.wallets.sync",
            &AtomicBool::new(true),
            Duration::from_secs(1)
        ),
        "cancelled"
    );
    let started = std::time::Instant::now();
    assert_eq!(
        state.wait(
            "y",
            "ui.wallets.sync",
            &AtomicBool::new(false),
            Duration::from_millis(40)
        ),
        "consent_timeout"
    );
    assert!(started.elapsed() >= Duration::from_millis(40));
    // A finished wait forgets its call: a late decision is refused.
    assert!(!state.record("y", "allow_once"));
    assert_eq!(
        kassiber_ai::consent::CONSENT_TIMEOUT,
        Duration::from_secs(300)
    );
}

#[test]
fn auto_reads_never_run_egressing_tools() {
    use kassiber_ai::executor::run_auto_reads;
    use kassiber_ai::planner::AutoRead;
    let mut fake = book();
    let mut state = dict([(
        "advertised_tools",
        Value::List(vec![
            s("ui_reports_lightning_profitability"),
            s("ui_connections_node_snapshot"),
            s("status"),
        ]),
    )]);
    let planned = [
        AutoRead {
            name: "ui.reports.lightning_profitability",
            arguments: dict([("connection", s("node"))]),
        },
        AutoRead {
            name: "ui.connections.node.snapshot",
            arguments: dict([("connection", s("node"))]),
        },
        AutoRead {
            name: "status",
            arguments: empty(),
        },
    ];
    let mut records = Vec::new();
    let mut runtime = Runtime {
        data_root: "/data",
        state: &mut state,
    };
    let context = run_auto_reads(
        &planned,
        &mut runtime,
        &mut fake,
        &mut |record| records.push(record),
        &|| false,
    );
    // The status record, then one call and one result: only `status` ran.
    assert_eq!(records.len(), 3);
    assert_eq!(fake.visits, 1);
    assert!(context.is_some());
    for record in &records {
        let text = format!("{record:?}");
        assert!(
            !text.contains("lightning") && !text.contains("node.snapshot"),
            "{text}"
        );
    }
}
