//! The reviewed Rust constants against the Python registries recorded in
//! `src/data/catalog.json` (`scripts/gen_catalog.py`), and the embedded
//! catalog itself.

use kassiber_ai::catalog::catalog_document;
use kassiber_ai::consent::{CONSENT_TIMEOUT, ONCE_ONLY_CONSENT};
use kassiber_ai::executor::{AUTO_JOURNAL_REFRESH_TOOLS, LOCAL_CUSTODY_READ_KINDS};
use kassiber_ai::planner::{
    AUTO_CONTEXT_ENTRY_MAX_CHARS, AUTO_CONTEXT_LIST_LIMIT, AUTO_CONTEXT_MAX_CHARS,
    AUTO_CONTEXT_STRING_LIMIT, SWAP_REVIEW_DEFAULT_LIMIT,
};
use kassiber_ai::redact::SENSITIVE_ARGUMENT_KEY_PARTS;
use kassiber_ai::select::{
    CHAT_DOCUMENT_CAPABILITY, CHAT_DOCUMENT_TOOL_NAMES, CORE_CAPABILITY_TOOL_NAMES,
    CORE_TOOL_NAMES, REVIEW_TOOL_NAMES, TOOL_CAPABILITY_NAMES, TOOL_PROFILE_NAMES,
    WALLET_IMPORT_TOOL_NAMES,
};
use kassiber_ai::skills::{read_skill_reference, SKILL_REFERENCE_NAMES};
use kassiber_ai::{get_tool, tools, KindClass};
use kassiber_envelope::get;
use kassiber_pyjson::{PyStr, Value};

fn constant(name: &str) -> Value {
    get(&catalog_document(), "constants")
        .and_then(|constants| get(constants, name))
        .cloned()
        .unwrap_or_else(|| panic!("missing constant {name}"))
}

fn names(name: &str) -> Vec<String> {
    match constant(name) {
        Value::List(items) => items
            .iter()
            .map(|item| match item {
                Value::Str(text) => text.as_str().unwrap().to_owned(),
                other => panic!("{other:?}"),
            })
            .collect(),
        other => panic!("{name}: {other:?}"),
    }
}

fn int(name: &str) -> i64 {
    match constant(name) {
        Value::Int(n) => i64::try_from(n).unwrap(),
        other => panic!("{name}: {other:?}"),
    }
}

fn sorted(items: &[&str]) -> Vec<String> {
    let mut items: Vec<String> = items.iter().map(|item| (*item).to_owned()).collect();
    items.sort();
    items
}

#[test]
fn constants_match_python() {
    assert_eq!(
        names("sensitive_argument_key_parts"),
        SENSITIVE_ARGUMENT_KEY_PARTS
    );
    assert_eq!(names("skill_reference_names"), SKILL_REFERENCE_NAMES);
    assert_eq!(names("tool_profile_names"), TOOL_PROFILE_NAMES);
    assert_eq!(names("tool_capability_names"), TOOL_CAPABILITY_NAMES);
    assert_eq!(names("core_tool_names"), sorted(&CORE_TOOL_NAMES));
    assert_eq!(
        names("wallet_import_tool_names"),
        sorted(&WALLET_IMPORT_TOOL_NAMES)
    );
    assert_eq!(
        names("chat_document_tool_names"),
        sorted(&CHAT_DOCUMENT_TOOL_NAMES)
    );
    assert_eq!(
        constant("chat_document_capability"),
        kassiber_envelope::s(CHAT_DOCUMENT_CAPABILITY)
    );
    let Value::Dict(areas) = constant("tool_area_summaries") else {
        panic!("tool_area_summaries")
    };
    let areas: Vec<&str> = areas
        .iter()
        .map(|(key, _)| match key {
            kassiber_pyjson::Key::Str(key) => key.as_str().unwrap(),
            other => panic!("{other:?}"),
        })
        .collect();
    // Every loadable area is a capability pack.
    assert!(areas
        .iter()
        .all(|area| TOOL_CAPABILITY_NAMES.contains(area)));
    assert_eq!(areas.len(), 11);
    assert_eq!(names("review_tool_names"), sorted(&REVIEW_TOOL_NAMES));
    assert_eq!(
        names("core_capability_tool_names"),
        sorted(&CORE_CAPABILITY_TOOL_NAMES)
    );
    assert_eq!(names("once_only_consent"), sorted(&ONCE_ONLY_CONSENT));
    assert_eq!(
        names("auto_journal_refresh_tools"),
        sorted(&AUTO_JOURNAL_REFRESH_TOOLS)
    );
    assert_eq!(
        names("local_custody_read_kinds"),
        sorted(&LOCAL_CUSTODY_READ_KINDS)
    );
    assert_eq!(
        constant("consent_timeout_seconds"),
        Value::Float(CONSENT_TIMEOUT.as_secs_f64())
    );
    assert_eq!(int("auto_context_max_chars"), AUTO_CONTEXT_MAX_CHARS as i64);
    assert_eq!(
        int("auto_context_entry_max_chars"),
        AUTO_CONTEXT_ENTRY_MAX_CHARS as i64
    );
    assert_eq!(
        int("auto_context_list_limit"),
        AUTO_CONTEXT_LIST_LIMIT as i64
    );
    assert_eq!(
        int("auto_context_string_limit"),
        AUTO_CONTEXT_STRING_LIMIT as i64
    );
    assert_eq!(int("swap_review_default_limit"), SWAP_REVIEW_DEFAULT_LIMIT);
}

#[test]
fn embedded_catalog_has_every_definition() {
    assert_eq!(tools().len(), 176);
    let reads = tools()
        .iter()
        .filter(|tool| tool.kind_class == KindClass::ReadOnly)
        .count();
    assert_eq!(reads, 110);
    let egressing: Vec<&str> = tools()
        .iter()
        .filter(|tool| tool.egresses)
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(
        egressing,
        [
            "ui.reports.lightning_profitability",
            "ui.connections.node.snapshot",
            "ui.rates.rebuild",
            "ui.wallets.sync",
            "ui.maintenance.run",
            "ui.wallets.read_document",
            "ui.rates.latest",
            "ui.chain_analysis.acquire.apply",
            "ui.chain_analysis.sources.run",
        ]
    );
    // `REVERSIBLE_TOOLS` is the one place reversibility is assigned.
    let Value::Dict(reversible) = constant("reversible_tools") else {
        panic!("reversible_tools")
    };
    let mut marked: Vec<&str> = tools()
        .iter()
        .filter(|tool| tool.reversible)
        .map(|tool| tool.name.as_str())
        .collect();
    marked.sort();
    let listed: Vec<String> = reversible
        .iter()
        .map(|(key, _)| match key {
            kassiber_pyjson::Key::Str(key) => key.as_str().unwrap().to_owned(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(marked, listed);
    assert_eq!(marked.len(), 19);
    for tool in tools().iter().filter(|tool| tool.reversible) {
        assert_eq!(tool.kind_class, KindClass::Mutating, "{}", tool.name);
    }
    // `load_tools` resolves by name but is never part of a pack.
    let load_tools = kassiber_ai::catalog::load_tools_entry();
    assert!(std::ptr::eq(get_tool("load_tools").unwrap(), load_tools));
    assert!(tools().iter().all(|tool| tool.name != "load_tools"));
    assert_eq!(load_tools.kind_class, KindClass::ReadOnly);
    for tool in tools() {
        assert!(
            std::ptr::eq(get_tool(&tool.name).unwrap(), tool),
            "{}",
            tool.name
        );
        assert!(
            std::ptr::eq(get_tool(tool.provider_name()).unwrap(), tool),
            "{}",
            tool.name
        );
        assert!(
            tool.requires_consent() == (tool.kind_class == KindClass::Mutating || tool.egresses)
        );
        let definition = tool.to_responses_tool();
        assert_eq!(get(&definition, "strict"), Some(&Value::Bool(false)));
    }
    assert_eq!(
        get_tool("ui_reports_report_blockers").unwrap().name,
        "ui.report.blockers"
    );
    assert!(get_tool("ui.reports.report_blockers").is_none());
    // Schemas keep their literal key order.
    let Some(Value::Dict(keys)) = Some(&get_tool("read_skill_reference").unwrap().parameters)
    else {
        unreachable!()
    };
    let order: Vec<&str> = keys
        .iter()
        .map(|(key, _)| match key {
            kassiber_pyjson::Key::Str(key) => key.as_str().unwrap(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        order,
        ["type", "additionalProperties", "required", "properties"]
    );
}

#[test]
fn skill_references_come_only_from_the_allowlist() {
    for name in SKILL_REFERENCE_NAMES {
        let reference = read_skill_reference(&PyStr::from(name)).unwrap();
        assert_eq!(get(&reference, "name"), Some(&kassiber_envelope::s(name)));
        assert!(
            matches!(get(&reference, "content"), Some(Value::Str(text)) if !text.as_str().unwrap().is_empty())
        );
    }
    let index = read_skill_reference(&PyStr::from("index")).unwrap();
    let Some(Value::Str(content)) = get(&index, "content") else {
        unreachable!()
    };
    assert!(content
        .as_str()
        .unwrap()
        .contains("Kassiber In-App Skill Index"));
    for name in [
        "../AGENTS",
        "AGENTS",
        "index.md",
        "swap-matching.md",
        "",
        "/etc/passwd",
        "Index",
    ] {
        let error = read_skill_reference(&PyStr::from(name)).unwrap_err();
        assert_eq!(error.code, "tool_not_allowed", "{name}");
    }
}

#[test]
fn chat_constants_match_python() {
    use kassiber_ai::chat::args::SCREEN_ROUTES;
    use kassiber_ai::chat::history::{HISTORY_MODES, SESSION_TITLE_MAX_CHARS};
    use kassiber_ai::chat::registry::{MAX_PENDING_CANCELS, PENDING_CANCEL_TTL};
    use kassiber_ai::chat::turn::CUT_OFF_FINISH_REASONS;
    assert_eq!(names("ai_screen_routes"), sorted(&SCREEN_ROUTES));
    assert_eq!(names("chat_history_modes"), HISTORY_MODES);
    assert_eq!(
        names("cut_off_finish_reasons"),
        sorted(&CUT_OFF_FINISH_REASONS)
    );
    assert_eq!(
        int("session_title_max_chars"),
        SESSION_TITLE_MAX_CHARS as i64
    );
    assert_eq!(int("max_pending_ai_cancels"), MAX_PENDING_CANCELS as i64);
    assert_eq!(
        constant("pending_ai_cancel_ttl_seconds"),
        Value::Float(PENDING_CANCEL_TTL.as_secs_f64())
    );
    let notices = constant("provider_status_notices");
    for (phase, label) in kassiber_ai::transport::PROVIDER_STATUS_NOTICES {
        assert_eq!(get(&notices, phase), Some(&kassiber_envelope::s(*label)));
    }
    assert_eq!(
        get(&catalog_document(), "system_prompt"),
        Some(&kassiber_envelope::s(
            kassiber_ai::prompt::default_system_prompt()
        ))
    );
    assert_eq!(
        get(&catalog_document(), "kassiber_system_prompt"),
        Some(&kassiber_envelope::s(
            kassiber_ai::prompt::kassiber_system_prompt()
        ))
    );
}
