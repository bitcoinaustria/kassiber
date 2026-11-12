//! The R6-5 privacy fences and the executor's gates, over a fake book that
//! counts every database visit.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use kassiber_ai::consent::{once_only, ConsentState};
use kassiber_ai::executor::{
    execute_read_only, is_advertised, model_output, Book, Maintenance, ParsedCall, ReadHost,
    Runtime,
};
use kassiber_ai::mutate::{execute_mutating, record_nonexecuted_custody_call, CustodyAudit};
use kassiber_ai::previews::consent_request;
use kassiber_ai::select::{build_responses_tools, on_device_only, strip_on_device_only};
use kassiber_ai::{get_tool, tools, KindClass, ToolError};
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;

/// A book on a fake request loop: records each visit and answers reads.
struct FakeBook {
    visits: usize,
    reads: Vec<String>,
    writes: Vec<String>,
    written: Vec<Value>,
    audits: Vec<Value>,
    auto_sync: Option<Value>,
    previewed: Option<Result<Value, String>>,
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
    fn maintain_for_read(&mut self, _sync_attempted: bool) -> Result<Maintenance, ToolError> {
        Ok(Maintenance {
            auto_sync: self.auto_sync.clone(),
            journals: self.maintained.clone(),
        })
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
    fn mutate(
        &mut self,
        kind: &str,
        arguments: &Value,
        source: &'static str,
    ) -> Result<Value, ToolError> {
        self.writes.push(format!("{kind}:{source}"));
        self.written.push(arguments.clone());
        Ok(self
            .payload
            .clone()
            .unwrap_or_else(|| dict([("value", s("ok"))])))
    }
    fn custody_assisted_write(
        &mut self,
        kind: &str,
        arguments: &Value,
        audit: &Value,
    ) -> Result<Value, ToolError> {
        self.audits.push(audit.clone());
        self.mutate(kind, arguments, "ai_tool")
    }
    fn append_custody_audit(&mut self, audit: &Value) -> Result<Value, ToolError> {
        self.audits.push(audit.clone());
        Ok(Value::Null)
    }
    fn decode_analysis_references(
        &mut self,
        _profile: &str,
        arguments: &Value,
    ) -> Result<Value, ToolError> {
        Ok(arguments.clone())
    }
    fn report_blockers(&mut self, auto_sync: Option<&Value>) -> Result<Value, ToolError> {
        self.reads.push("ui.report.blockers".into());
        Ok(dict([("ready", Value::Bool(auto_sync.is_none()))]))
    }
    fn dataset_preview(&mut self, _arguments: &Value) -> Result<Value, ToolError> {
        self.reads.push("dataset.preview".into());
        match self.previewed.clone() {
            Some(Ok(preview)) => Ok(preview),
            Some(Err(code)) => Err(kassiber_envelope::AppError::new(code, "preview").into()),
            None => Err(kassiber_envelope::AppError::new(
                "chain_analysis_preview_required",
                "preview",
            )
            .into()),
        }
    }
    fn review(
        &mut self,
        kind: &str,
        arguments: &Value,
        _full_access: bool,
    ) -> Result<Value, ToolError> {
        // Review kinds are `ai_tool`-authored reads and writes.
        if kind == "ui.review.apply" {
            return self.mutate(kind, arguments, "ai_tool");
        }
        self.read(kind, arguments, "ai_tool")
    }
    fn review_consent_preview(
        &mut self,
        _arguments: &Value,
        _full_access: bool,
    ) -> Result<Value, ToolError> {
        self.reads.push("review.preview".into());
        Ok(dict([("status", s("ready"))]))
    }
    fn profile_fiat_currency(&mut self) -> Result<String, ToolError> {
        Ok("EUR".into())
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
        writes: Vec::new(),
        written: Vec::new(),
        audits: Vec::new(),
        auto_sync: None,
        previewed: None,
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

fn names(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Str(text) => text.as_str().unwrap().to_owned(),
                other => panic!("{other:?}"),
            })
            .collect(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn load_tools_adds_requested_areas_only_when_offered() {
    let mut fake = book();
    let areas = |areas: &[&str]| {
        dict([(
            "areas",
            Value::List(areas.iter().map(|area| s(*area)).collect()),
        )])
    };
    // A broker turn (or `full`) never enables loading.
    let mut state = dict([("advertised_tools", Value::List(vec![s("load_tools")]))]);
    let denied = run(
        &mut fake,
        &mut state,
        "load_tools",
        areas(&["loans"]),
        false,
    );
    assert_eq!(reason(&denied).as_deref(), Some("tool_not_advertised"));
    let enabled = |profile: &str, on_device: bool| {
        dict([
            ("advertised_tools", Value::List(vec![s("load_tools")])),
            ("load_tools_enabled", Value::Bool(true)),
            ("tool_profile", s(profile)),
            ("provider_on_device", Value::Bool(on_device)),
        ])
    };
    let mut state = enabled("scoped", false);
    let loaded = run(
        &mut fake,
        &mut state,
        "load_tools",
        areas(&["loans", "loans"]),
        false,
    );
    let data = get(get(&loaded, "envelope").unwrap(), "data").unwrap();
    assert_eq!(names(get(data, "areas")), ["loans"]);
    let added = names(get(data, "added_tools"));
    let loans: Vec<&str> = tools()
        .iter()
        .filter(|tool| tool.name.starts_with("ui.loans."))
        .map(|tool| tool.provider_name())
        .collect();
    assert_eq!(added, loans);
    assert_eq!(get(data, "already_offered"), Some(&Value::from(0)));
    let mut advertised = vec!["load_tools".to_owned()];
    advertised.extend(added.iter().cloned());
    assert_eq!(names(get(&state, "advertised_tools")), advertised);
    assert!(
        matches!(get(&state, "pending_tool_definitions"), Some(Value::List(pending)) if pending.len() == loans.len())
    );
    // Loading again adds nothing new.
    let again = run(
        &mut fake,
        &mut state,
        "load_tools",
        areas(&["loans"]),
        false,
    );
    let data = get(get(&again, "envelope").unwrap(), "data").unwrap();
    assert!(names(get(data, "added_tools")).is_empty());
    assert_eq!(
        get(data, "already_offered"),
        Some(&Value::from(loans.len()))
    );
    // A restricted provider never gets the on-device-only tools.
    for on_device in [false, true] {
        let mut state = enabled("scoped", on_device);
        let loaded = run(
            &mut fake,
            &mut state,
            "load_tools",
            areas(&["privacy"]),
            false,
        );
        let data = get(get(&loaded, "envelope").unwrap(), "data").unwrap();
        let added = names(get(data, "added_tools"));
        assert_eq!(added.iter().any(|name| on_device_only(name)), on_device);
    }
    // The `core` profile keeps its own limit.
    let mut state = enabled("core", true);
    let loaded = run(
        &mut fake,
        &mut state,
        "load_tools",
        areas(&["loans"]),
        false,
    );
    let data = get(get(&loaded, "envelope").unwrap(), "data").unwrap();
    assert!(names(get(data, "added_tools")).is_empty());
    assert_eq!(fake.visits, 0);
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
        assert_eq!(fake.visits, 0, "{name}");
        // Bound consent reaches the tool over its native route.
        let mut bound = dict([("consented_call_ids", Value::List(vec![s("call_1")]))]);
        let peer_row = || {
            dict([
                ("id", s("chan-id-secret")),
                ("shortChannelId", s("800000x1x0")),
                ("fundingOutpoint", s("outpoint-secret")),
                ("peerAlias", s("peer")),
                ("peerPubkey", s("03peerkey")),
                ("capacitySat", Value::from(5)),
            ])
        };
        fake.payload = Some(dict([
            ("alias", s("my-node")),
            ("pubkey", s("02operatorkey")),
            (
                "connection",
                dict([
                    ("label", s("Node")),
                    ("kind", s("lnd")),
                    ("id", s("conn-id-secret")),
                ]),
            ),
            ("windowLabel", s("30d")),
            ("summary", dict([("netProfitSat", Value::from(1))])),
            ("channels", Value::List(vec![peer_row()])),
            ("closedChannels", Value::List(vec![peer_row()])),
            (
                "forwards",
                Value::List(vec![dict([
                    ("id", s("fwd-id-secret")),
                    ("inPeerAlias", s("peer")),
                    ("inShortChannelId", s("800000x1x0")),
                    ("outPeerAlias", s("peer")),
                    ("outShortChannelId", s("800000x1x0")),
                    ("feeMsat", Value::from(7)),
                ])]),
            ),
        ]));
        let result = run(&mut fake, &mut bound, name, arguments(), false);
        assert_eq!(fake.visits, 1, "{name}");
        let output = model_output(&result);
        if name == "ui.connections.node.snapshot" {
            // `snapshot_to_dict_for_ai`: operational figures and the
            // operator's own label, never a pubkey, peer or channel id.
            for kept in [
                "my-node",
                "\"capacitySat\":5",
                "\"feeMsat\":7",
                "\"label\":\"Node\"",
                "\"pubkey\":null",
            ] {
                assert!(output.contains(kept), "{kept}: {output}");
            }
            for dropped in [
                "02operatorkey",
                "03peerkey",
                "peer\"",
                "800000x1x0",
                "secret",
            ] {
                assert!(!output.contains(dropped), "{dropped}: {output}");
            }
        } else {
            // The AI projection keeps the aggregate only.
            assert!(
                output.contains("30d")
                    && !output.contains("peer")
                    && !output.contains("\"connection\""),
                "{output}"
            );
        }
    }
}

#[test]
fn custody_reads_need_full_agent_access() {
    let mut fake = book();
    // A restricted off-device provider, then full access, then on device.
    for (field, allowed) in [
        ("provider_full_access_absent", false),
        ("provider_full_access", true),
        ("provider_on_device", true),
    ] {
        let mut state = dict([("provider_kind", s("remote")), (field, Value::Bool(true))]);
        let result = run(
            &mut fake,
            &mut state,
            "ui.custody.gaps.list",
            empty(),
            false,
        );
        assert_eq!(
            reason(&result).as_deref() == Some("agent_access_restricted"),
            !allowed,
            "{field}"
        );
        if !allowed {
            let remediation = get(&result, "remediation").unwrap();
            assert_eq!(
                get(remediation, "action"),
                Some(&s("enable_full_agent_access"))
            );
            assert_eq!(get(remediation, "desktop_only"), Some(&Value::Bool(true)));
        }
    }
    assert_eq!(
        fake.reads,
        [
            "ui.custody.gaps.list:ai_tool",
            "ui.custody.gaps.list:ai_tool"
        ]
    );
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
            Some("agent_access_restricted"),
            "{name}: {result:?}"
        );
        assert_eq!(
            get(&result, "hint"),
            Some(&s("Set the provider's agent access to full in Settings → Assistant, or use the desktop workflow.")),
        );
        assert_eq!(
            get(get(&result, "remediation").unwrap(), "action"),
            Some(&s("enable_full_agent_access"))
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
                        "ui.wallets.sync",
                        // Rows from an attached file or document into a file
                        // wallet (kind custom): no descriptor, backend or key.
                        "ui.wallets.import_attachment.plan",
                        "ui.wallets.import_attachment.apply",
                        "ui.wallets.import_document.plan",
                        "ui.wallets.import_document.apply",
                        "ui.wallets.read_document",
                        // A desktop handoff card naming a source kind; the
                        // user enters the source in the wizard.
                        "ui.wallets.request_input"
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
    // Since the 5h/5i floor the URL's span runs to the end of the line and
    // takes the path with it, as redact_ai_tool_result does ("see <redacted-url>").
    assert!(output.contains("<redacted-url>"));
}

#[test]
fn visuals_refresh_journals_only_for_report_sources() {
    for (source, refreshed) in [
        ("overview", true),
        ("tax_summary", true),
        ("transactions", false),
    ] {
        let mut fake = book();
        fake.maintained = Some(dict([("processed_transactions", Value::from(1))]));
        let mut state = empty();
        let result = run(
            &mut fake,
            &mut state,
            "ui_visuals_render",
            dict([("source", s(source))]),
            false,
        );
        assert_eq!(get(&result, "ok"), Some(&Value::Bool(true)), "{result:?}");
        assert_eq!(
            get(&result, "auto_journal_process").is_some(),
            refreshed,
            "{source}"
        );
    }
}

#[test]
fn stale_journal_figures_never_reach_the_model() {
    // #681: an AI read of unprocessed journals gets an explicit state and
    // `null` figures, never zeros that read as "nothing booked".
    let mut fake = book();
    fake.payload = Some(dict([
        (
            "status",
            dict([
                ("needsJournals", Value::Bool(true)),
                ("lastProcessedAt", Value::Null),
            ]),
        ),
        ("recent", Value::List(vec![s("entry")])),
    ]));
    let mut state = empty();
    let result = run(
        &mut fake,
        &mut state,
        "ui.journals.snapshot",
        empty(),
        false,
    );
    let data = get(get(&result, "envelope").unwrap(), "data").unwrap();
    assert_eq!(get(data, "status"), Some(&s("unavailable")));
    assert_eq!(get(data, "reason"), Some(&s("journals_never_processed")));
    assert_eq!(get(data, "recent"), Some(&Value::Null));
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
    // A model calls by wire name and refreshes the same (09912e4c0).
    let result = run(
        &mut fake,
        &mut empty(),
        "ui_reports_summary",
        empty(),
        false,
    );
    assert!(get(&result, "auto_journal_process").is_some());
}

/// tests/rust_port/repro_ai_refresh_wire_name.py: models call tools by wire
/// name, and the refresh decides on the catalog entry.
#[test]
fn wire_name_reads_refresh_like_catalog_names() {
    let mut fake = book();
    fake.maintained = Some(dict([("processed_transactions", Value::from(2))]));
    for name in [
        "ui_reports_summary",
        "ui.reports.summary",
        "ui_review_worklist",
    ] {
        let mut state = empty();
        let result = run(&mut fake, &mut state, name, empty(), false);
        assert!(get(&result, "auto_journal_process").is_some(), "{name}");
    }
    let result = run(&mut fake, &mut empty(), "ui_rates_summary", empty(), false);
    assert!(get(&result, "auto_journal_process").is_none());
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
                state.wait(
                    "c1",
                    &tool,
                    &AtomicBool::new(false),
                    Duration::from_secs(5),
                    false,
                    false,
                )
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
        let again = state.wait(
            "c2",
            tool,
            &cancelled,
            Duration::from_millis(30),
            false,
            false,
        );
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
fn permission_modes_approve_without_asking_or_release_waiting_calls() {
    use kassiber_ai::consent::SessionAllows;
    let allows = SessionAllows::default();
    let state = Arc::new(ConsentState::new(Arc::clone(&allows)));
    assert_eq!(state.permission_mode(), "approval");
    // `approval` asks for everything.
    assert_eq!(state.preapproval("ui.transfers.pair", true, false), None);
    // `auto` approves a reversible tool that stays on this machine only.
    state.set_permission_mode("auto");
    assert_eq!(
        state.preapproval("ui.transfers.pair", true, false),
        Some("mode_auto")
    );
    assert_eq!(state.preapproval("ui.wallets.sync", true, true), None);
    assert_eq!(
        state.preapproval("ui.transfers.rules.apply", false, false),
        None
    );
    // `full_access` approves everything, once-only and network tools too.
    state.set_permission_mode("full_access");
    assert_eq!(
        state.preapproval("ui.review.apply", false, true),
        Some("mode_full_access")
    );
    // A mode switch releases the waiting calls it approves, in order.
    state.set_permission_mode("approval");
    let spawn = |call: &'static str, reversible: bool, egresses: bool| {
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            state.wait(
                call,
                "ui.transfers.pair",
                &AtomicBool::new(false),
                Duration::from_secs(5),
                reversible,
                egresses,
            )
        })
    };
    let reversible = spawn("c-rev", true, false);
    let network = spawn("c-net", true, true);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while state.waiting() < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "waiters never registered"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let released = state.set_permission_mode("auto");
    assert_eq!(released, ["c-rev"]);
    assert_eq!(reversible.join().unwrap(), "mode_auto");
    let released = state.set_permission_mode("full_access");
    assert_eq!(released, ["c-net"]);
    assert_eq!(network.join().unwrap(), "mode_full_access");
    // "Allow for this chat" lives in the shared session set.
    state.set_permission_mode("approval");
    allows.lock().unwrap().insert("ui.transfers.pair".into());
    assert_eq!(
        state.preapproval("ui.transfers.pair", false, false),
        Some("allow_session")
    );
    allows.lock().unwrap().insert("ui.review.apply".into());
    assert_eq!(state.preapproval("ui.review.apply", false, false), None);
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
            Duration::from_secs(1),
            false,
            false
        ),
        "deny"
    );
    assert_eq!(
        state.wait(
            "x",
            "ui.wallets.sync",
            &AtomicBool::new(true),
            Duration::from_secs(1),
            false,
            false
        ),
        "cancelled"
    );
    let started = std::time::Instant::now();
    assert_eq!(
        state.wait(
            "y",
            "ui.wallets.sync",
            &AtomicBool::new(false),
            Duration::from_millis(40),
            false,
            false
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

fn write(
    book: &mut FakeBook,
    state: &mut Value,
    name: &str,
    arguments: Value,
    audit: Option<&CustodyAudit>,
) -> Value {
    let call = ParsedCall::new("call_1", name, arguments);
    let mut runtime = Runtime {
        data_root: "/data",
        state,
    };
    execute_mutating(&call, &mut runtime, book, audit)
}

fn consented() -> Value {
    dict([("consented_call_ids", Value::List(vec![s("call_1")]))])
}

fn audit() -> CustodyAudit {
    CustodyAudit {
        provider_kind: "local".into(),
        model: "m".into(),
        consent_decision: "allow_once".into(),
        consent_requested_at: "2026-01-01T00:00:00Z".into(),
        consent_decided_at: "2026-01-01T00:00:01Z".into(),
    }
}

#[test]
fn mutating_tools_run_only_with_a_consent_bound_to_the_call() {
    for tool in tools()
        .iter()
        .filter(|tool| tool.kind_class == KindClass::Mutating)
    {
        let mut fake = book();
        let name = tool.name.as_str();
        // No consent, or a consent for another call: nothing runs.
        for mut state in [
            empty(),
            dict([("consented_call_ids", Value::List(vec![s("call_2")]))]),
            dict([("consented_call_ids", s("call_1"))]),
        ] {
            if name == "ui.custody.review.apply" {
                kassiber_envelope::set(&mut state, "provider_full_access", Value::Bool(true));
            }
            let result = write(&mut fake, &mut state, name, empty(), Some(&audit()));
            assert_eq!(
                reason(&result).as_deref(),
                Some("consent_required"),
                "{name}"
            );
        }
        assert_eq!(
            (fake.visits, fake.writes.len(), fake.audits.len()),
            (0, 0, 0),
            "{name}"
        );
    }
}

#[test]
fn a_chat_file_kept_as_evidence_never_names_the_file_to_a_restricted_provider() {
    let stored = || {
        dict([
            ("id", s("att-1")),
            ("original_filename", s("statement_max.mustermann.pdf")),
            ("filename", s("statement_max.mustermann.pdf")),
            ("label", s("statement_max.mustermann.pdf")),
            ("display_label", s("statement_max.mustermann.pdf")),
        ])
    };
    let chat_files = Value::List(vec![dict([
        ("id", s("file-1")),
        ("source_file", s("/chat/statement_max.mustermann.pdf")),
        ("filename", s("statement_max.mustermann.pdf")),
        ("kind", s("pdf")),
    ])]);
    for full_access in [false, true] {
        let mut fake = book();
        fake.payload = Some(stored());
        let mut state = consented();
        kassiber_envelope::set(&mut state, "chat_files", chat_files.clone());
        kassiber_envelope::set(&mut state, "provider_full_access", Value::Bool(full_access));
        let result = write(
            &mut fake,
            &mut state,
            "ui.attachments.add_from_chat",
            dict([("transaction", s("tx-1")), ("file", s("file-1"))]),
            None,
        );
        assert_eq!(fake.writes, ["ui.attachments.add:ai_tool"]);
        // The managed copy is made from the grant's file, named by it.
        assert_eq!(
            get(&fake.written[0], "file"),
            Some(&s("/chat/statement_max.mustermann.pdf"))
        );
        let text = format!("{result:?}");
        assert_eq!(text.contains("mustermann"), full_access, "{text}");
    }
}

#[test]
fn mutating_tools_run_only_when_advertised_this_turn() {
    let mut fake = book();
    let mut state = consented();
    kassiber_envelope::set(
        &mut state,
        "advertised_tools",
        Value::List(vec![s("ui_transactions_metadata_update")]),
    );
    let arguments = || dict([("transaction", s("a")), ("note", s("n"))]);
    let denied = write(
        &mut fake,
        &mut state,
        "ui_transactions_history_revert",
        dict([("transaction", s("a")), ("event", s("e"))]),
        None,
    );
    assert_eq!(reason(&denied).as_deref(), Some("tool_not_advertised"));
    assert_eq!(fake.visits, 0);
    let allowed = write(
        &mut fake,
        &mut state,
        "ui_transactions_metadata_update",
        arguments(),
        None,
    );
    assert_eq!(get(&allowed, "ok"), Some(&Value::Bool(true)), "{allowed:?}");
    // Edits are authored by the assistant, whatever the model asked for.
    assert_eq!(fake.writes, vec!["ui.transactions.metadata.update:ai_tool"]);
    assert_eq!(get(&fake.written[0], "source"), Some(&s("ai_tool")));
}

#[test]
fn mutating_tools_check_the_frozen_project_and_book() {
    let mut fake = book();
    fake.in_scope = false;
    let mut state = consented();
    let result = write(
        &mut fake,
        &mut state,
        "ui.loans.unmark",
        dict([("txid", s("a"))]),
        None,
    );
    assert_eq!(reason(&result).as_deref(), Some("stale_context"));
    let mut fake = book();
    let mut state = consented();
    kassiber_envelope::set(&mut state, "scope_workspace_id", s("w1"));
    kassiber_envelope::set(&mut state, "scope_profile_id", s("other"));
    let result = write(
        &mut fake,
        &mut state,
        "ui.loans.unmark",
        dict([("txid", s("a"))]),
        None,
    );
    assert_eq!(reason(&result).as_deref(), Some("stale_context"));
    assert!(fake.writes.is_empty());
}

#[test]
fn pdf_and_audit_package_exports_run_under_their_own_kinds() {
    let mut fake = book();
    let mut state = consented();
    // The commercial evidence review reaches its desktop route.
    let result = write(
        &mut fake,
        &mut state,
        "ui.btcpay.provenance.review",
        dict([("link", s("l")), ("state", s("suggested"))]),
        None,
    );
    assert_eq!(get(&result, "ok"), Some(&Value::Bool(true)), "{result:?}");
    assert_eq!(fake.writes, ["ui.btcpay.provenance.review:gui"]);
    fake.writes.clear();
    fake.written.clear();
    // Every export target runs, saved locally, under its own kind.
    for (arguments, kind) in [
        (
            dict([("report", s("full")), ("format", s("csv"))]),
            "ui.reports.export_csv",
        ),
        (
            dict([("report", s("full")), ("format", s("pdf"))]),
            "ui.reports.export_pdf",
        ),
        (
            dict([
                ("report", s("audit_package")),
                ("format", s("package")),
                ("transaction", s("a")),
            ]),
            "ui.reports.export_audit_package",
        ),
        (
            dict([
                ("report", s("exit_tax")),
                ("format", s("pdf")),
                ("departure_date", s("2025-06-30")),
                ("destination", s("eu_eea")),
            ]),
            "ui.reports.export_exit_tax_pdf",
        ),
    ] {
        let result = write(&mut fake, &mut state, "ui.reports.export", arguments, None);
        let envelope = get(&result, "envelope").unwrap_or_else(|| panic!("{result:?}"));
        assert_eq!(get(envelope, "kind"), Some(&s(kind)));
        let data = get(envelope, "data").unwrap();
        assert_eq!(get(data, "artifact_kind"), Some(&s(kind)));
        assert_eq!(get(data, "saved_locally"), Some(&Value::Bool(true)));
    }
    for (format, kind) in [
        ("pdf", "ui.source_funds.export_pdf"),
        ("bundle", "ui.source_funds.export_bundle"),
    ] {
        let result = write(
            &mut fake,
            &mut state,
            "ui.source_funds.export",
            dict([("case", s(" c ")), ("format", s(format))]),
            None,
        );
        let envelope = get(&result, "envelope").unwrap_or_else(|| panic!("{result:?}"));
        assert_eq!(get(envelope, "kind"), Some(&s(kind)));
    }
    assert_eq!(
        fake.writes,
        vec![
            "ui.reports.export_csv:gui",
            "ui.reports.export_pdf:gui",
            "ui.reports.export_audit_package:gui",
            "ui.reports.export_exit_tax_pdf:gui",
            "ui.source_funds.export_pdf:gui",
            "ui.source_funds.export_bundle:gui",
        ]
    );
    // The package receives its scope field; a case reference is stripped.
    assert_eq!(fake.written[2], dict([("transaction", s("a"))]));
    assert_eq!(fake.written[4], dict([("case", s("c"))]));
}

#[test]
fn dataset_imports_are_bound_to_the_completed_preview() {
    let mut fake = book();
    let mut state = consented();
    kassiber_envelope::set(&mut state, "provider_on_device", Value::Bool(true));
    let manifest = || {
        dict([
            ("dataset_key", s("k")),
            ("name", s("n")),
            ("version", s("1")),
            ("chain", s("bitcoin")),
            ("network", s("main")),
            ("source", s("s")),
            ("license", s("l")),
            ("attribution_method", s("a")),
            ("visibility", s("public")),
        ])
    };
    let arguments = || {
        dict([
            ("source_token", s("t")),
            ("manifest", manifest()),
            ("expected_sha256", s("a".repeat(64))),
        ])
    };
    let refused = write(
        &mut fake,
        &mut state,
        "ui.chain_analysis.datasets.import",
        arguments(),
        None,
    );
    assert_eq!(
        reason(&refused).as_deref(),
        Some("chain_analysis_preview_required")
    );
    assert!(fake
        .reads
        .iter()
        .all(|read| !read.starts_with("ui.chain_analysis.datasets.import")));
    fake.previewed = Some(Ok(dict([("sha256", s("a".repeat(64)))])));
    write(
        &mut fake,
        &mut state,
        "ui.chain_analysis.datasets.import",
        arguments(),
        None,
    );
    assert!(
        fake.reads
            .contains(&"ui.chain_analysis.datasets.import:gui".to_string()),
        "{:?}",
        fake.reads
    );
}

#[test]
fn composite_reads_report_the_refresh_they_ran() {
    let failed = dict([("ok", Value::Bool(false)), ("reason", s("sync_failed"))]);
    for (auto_sync, contacted) in [(None, false), (Some(failed.clone()), true)] {
        let mut fake = book();
        fake.auto_sync = auto_sync.clone();
        fake.payload = Some(dict([(
            "safety",
            dict([
                ("local_only_reads", Value::Bool(true)),
                ("network_contacted", Value::Bool(false)),
            ]),
        )]));
        let mut state = empty();
        let result = run(
            &mut fake,
            &mut state,
            "ui.transactions.review_context",
            dict([("transaction", s("a"))]),
            true,
        );
        let safety = get(
            get(get(&result, "envelope").unwrap(), "data").unwrap(),
            "safety",
        )
        .unwrap();
        assert_eq!(
            get(safety, "network_contacted"),
            Some(&Value::Bool(contacted))
        );
        assert_eq!(
            get(safety, "network_contact_reason"),
            Some(&if contacted {
                s("opt_in_freshness_sync")
            } else {
                Value::Null
            })
        );
        // A failed refresh also answers the readiness it blocks.
        assert_eq!(
            get(&result, "auto_report_blockers").is_some(),
            contacted,
            "{result:?}"
        );
        assert_eq!(
            get(&state, "auto_sync_attempted"),
            auto_sync.as_ref().map(|_| &Value::Bool(true))
        );
    }
}

#[test]
fn the_executors_never_cross_kind_classes() {
    let mut fake = book();
    let mut state = consented();
    for name in ["ui.rates.summary", "status", "nonexistent", ""] {
        let result = write(&mut fake, &mut state, name, empty(), None);
        assert_eq!(
            reason(&result).as_deref(),
            Some("tool_not_allowed"),
            "{name}"
        );
    }
    assert_eq!(fake.visits, 0);
}

#[test]
fn custody_writes_need_full_agent_access_and_are_audited() {
    let arguments = || {
        dict([
            ("action", s("reopen")),
            ("gap_id", s("gap-1")),
            ("expected_input_version", Value::from(0)),
        ])
    };
    let mut fake = book();
    let mut remote = consented();
    kassiber_envelope::set(&mut remote, "provider_kind", s("remote"));
    let denied = write(
        &mut fake,
        &mut remote,
        "ui.custody.review.apply",
        arguments(),
        Some(&audit()),
    );
    assert_eq!(reason(&denied).as_deref(), Some("agent_access_restricted"));
    assert_eq!(fake.visits, 0);
    // A provider with full access, without the consent audit context or
    // the frozen book scope, is refused before any write.
    let mut local = consented();
    kassiber_envelope::set(&mut local, "provider_kind", s("remote"));
    kassiber_envelope::set(&mut local, "provider_full_access", Value::Bool(true));
    let missing = write(
        &mut fake,
        &mut local,
        "ui.custody.review.apply",
        arguments(),
        None,
    );
    assert_eq!(
        reason(&missing).as_deref(),
        Some("custody_ai_audit_required")
    );
    let unscoped = write(
        &mut fake,
        &mut local,
        "ui.custody.review.apply",
        arguments(),
        Some(&audit()),
    );
    assert_eq!(reason(&unscoped).as_deref(), Some("stale_context"));
    assert!(fake.writes.is_empty() && fake.audits.is_empty());
    kassiber_envelope::set(&mut local, "scope_workspace_id", s("w1"));
    kassiber_envelope::set(&mut local, "scope_profile_id", s("p1"));
    let applied = write(
        &mut fake,
        &mut local,
        "ui.custody.review.apply",
        arguments(),
        Some(&audit()),
    );
    assert_eq!(get(&applied, "ok"), Some(&Value::Bool(true)), "{applied:?}");
    assert_eq!(fake.writes, vec!["ui.custody.review.apply:ai_tool"]);
    let expected = dict([
        ("workspace_id", s("w1")),
        ("profile_id", s("p1")),
        ("tool_name", s("ui.custody.review.apply")),
        ("daemon_kind", s("ui.custody.review.apply")),
        ("call_id", s("call_1")),
        ("provider_kind", s("local")),
        ("model", s("m")),
        ("model_proposal", arguments()),
        ("final_proposal", arguments()),
        ("consent_decision", s("allow_once")),
        ("consent_requested_at", s("2026-01-01T00:00:00Z")),
        ("consent_decided_at", s("2026-01-01T00:00:01Z")),
        ("execution_status", s("executed")),
        ("execution_code", Value::Null),
    ]);
    assert_eq!(fake.audits, vec![expected]);
}

#[test]
fn denied_custody_proposals_are_audited_without_running() {
    let mut fake = book();
    let mut state = dict([
        ("scope_workspace_id", s("w1")),
        ("scope_profile_id", s("p1")),
    ]);
    let call = ParsedCall::new(
        "call_9",
        "ui_custody_review_apply",
        dict([("gap_id", s("g"))]),
    );
    let mut runtime = Runtime {
        data_root: "/data",
        state: &mut state,
    };
    record_nonexecuted_custody_call(
        &call,
        &mut runtime,
        &mut fake,
        &audit(),
        "denied",
        "user_denied",
    )
    .unwrap();
    assert!(fake.writes.is_empty());
    assert_eq!(fake.audits.len(), 1);
    assert_eq!(get(&fake.audits[0], "execution_status"), Some(&s("denied")));
    assert_eq!(
        get(&fake.audits[0], "execution_code"),
        Some(&s("user_denied"))
    );
    // Other tools are never audited as custody decisions.
    let other = ParsedCall::new("call_9", "ui_transfers_pair", empty());
    let mut runtime = Runtime {
        data_root: "/data",
        state: &mut state,
    };
    record_nonexecuted_custody_call(
        &other,
        &mut runtime,
        &mut fake,
        &audit(),
        "denied",
        "user_denied",
    )
    .unwrap();
    assert_eq!(fake.audits.len(), 1);
}

#[test]
fn mutating_results_are_redacted_before_they_leave_the_executor() {
    let mut fake = book();
    fake.payload = Some(dict([
        (
            "note",
            s("see https://private.example/x and /Users/alice/file.pdf"),
        ),
        ("api_token", s("sk-abcdef123456")),
    ]));
    let mut state = consented();
    let result = write(
        &mut fake,
        &mut state,
        "ui.transactions.metadata.update",
        dict([("transaction", s("a"))]),
        None,
    );
    // The value itself, not the model bytes (which redact once more).
    let output = kassiber_ai::planner::model_json(&result);
    for private in ["private.example", "/Users/alice", "sk-abcdef"] {
        assert!(!output.contains(private), "{private} in {output}");
    }
}

#[test]
fn reversible_writes_name_the_call_that_takes_them_back() {
    let mut fake = book();
    fake.payload = Some(dict([("id", s(" pair-9 "))]));
    let mut state = consented();
    let pair = dict([("tx_out", s("a")), ("tx_in", s("b"))]);
    let result = write(&mut fake, &mut state, "ui.transfers.pair", pair, None);
    let data = get(get(&result, "envelope").unwrap(), "data").unwrap();
    assert_eq!(
        get(data, "undo"),
        Some(&dict([
            ("tool", s("ui_transfers_unpair")),
            ("arguments", dict([("pair_id", s("pair-9"))])),
        ]))
    );
    // A tool that is not reversible gets no hint, whatever its result.
    let mut state = consented();
    let saved = write(
        &mut fake,
        &mut state,
        "ui.saved_views.delete",
        dict([("view_id", s("v"))]),
        None,
    );
    let data = get(get(&saved, "envelope").unwrap(), "data").unwrap();
    assert_eq!(get(data, "undo"), None);
}

#[test]
fn consent_previews_never_write() {
    let mut fake = book();
    let mut state = dict([("provider_on_device", Value::Bool(false))]);
    for (name, arguments) in [
        (
            "ui.review.apply",
            dict([("artifact", empty()), ("idempotency_key", s("k"))]),
        ),
        ("ui.chain_analysis.acquire.apply", dict([("plan", empty())])),
        (
            "ui.chain_analysis.datasets.import",
            dict([("manifest", empty())]),
        ),
        (
            "ui.transfers.pair",
            dict([("tx_out", s("a")), ("tx_in", s("b"))]),
        ),
    ] {
        let entry = get_tool(name).unwrap();
        let call = ParsedCall::new("call_1", name, arguments);
        let mut runtime = Runtime {
            data_root: "/data",
            state: &mut state,
        };
        let card = consent_request(entry, &call, &mut runtime, &mut fake).unwrap();
        assert_eq!(get(&card, "call_id"), Some(&s("call_1")), "{name}");
        if name.starts_with("ui.chain_analysis.") {
            // Other providers never see a recomputed acquisition or dataset.
            assert_eq!(
                get(&card, "arguments_preview"),
                Some(&dict([
                    ("status", s("blocked")),
                    ("code", s("agent_access_restricted"))
                ])),
                "{name}"
            );
        }
    }
    assert!(fake.writes.is_empty() && fake.audits.is_empty());
    assert_eq!(fake.reads, vec!["review.preview"]);
}
