//! Per-turn tool exposure (`kassiber/ai/tools.py:3475`–`3667` and
//! `kassiber/ai/prompt.py`'s `build_responses_tools`): capability packs, the
//! keyword rules and the review-checkpoint continuation.

use std::collections::BTreeSet;

use kassiber_envelope::{get, py_str, AppError};
use kassiber_pyjson::{Key, PyStr, Value};

use crate::catalog::{tools, KindClass, ToolEntry};
use crate::pyre::{self, lossy};

/// `TOOL_PROFILE_NAMES`.
pub const TOOL_PROFILE_NAMES: [&str; 3] = ["core", "scoped", "full"];

/// `TOOL_CAPABILITY_NAMES`.
pub const TOOL_CAPABILITY_NAMES: [&str; 12] = [
    "core",
    "review",
    "workspace",
    "transactions",
    "reports",
    "wallets",
    "loans",
    "privacy",
    "source_funds",
    "merchant",
    "transfers",
    "operations",
];

/// `CORE_TOOL_NAMES` (after its union with `REVIEW_TOOL_NAMES`): the default
/// `kassiber chat` profile.
pub const CORE_TOOL_NAMES: [&str; 60] = [
    "read_skill_reference",
    "status",
    "ui.attachments.list",
    "ui.audit.changes_since_last_answer",
    "ui.audit.evidence.summary",
    "ui.backends.list",
    "ui.custody.coverage.snapshot",
    "ui.custody.gaps.history",
    "ui.custody.gaps.list",
    "ui.custody.gaps.review_context",
    "ui.custody.lineage.snapshot",
    "ui.custody.review.apply",
    "ui.custody.review.plan",
    "ui.documents.list",
    "ui.journals.process",
    "ui.journals.quarantine",
    "ui.journals.quarantine.resolve",
    "ui.journals.snapshot",
    "ui.journals.transfers.list",
    "ui.maintenance.run",
    "ui.maintenance.settings",
    "ui.next_actions",
    "ui.overview.snapshot",
    "ui.profiles.snapshot",
    "ui.rates.coverage",
    "ui.rates.rebuild",
    "ui.rates.summary",
    "ui.report.blockers",
    "ui.reports.balance_history",
    "ui.reports.balance_sheet",
    "ui.reports.capital_gains",
    "ui.reports.portfolio_summary",
    "ui.reports.summary",
    "ui.reports.tax_summary",
    "ui.review.apply",
    "ui.review.cases",
    "ui.review.plan",
    "ui.review.receipt",
    "ui.review.request_input",
    "ui.review.worklist",
    "ui.transactions.extremes",
    "ui.transactions.graph",
    "ui.transactions.history",
    "ui.transactions.list",
    "ui.transactions.resolve",
    "ui.transactions.review_context",
    "ui.transactions.search",
    "ui.transfers.components.apply",
    "ui.transfers.components.list",
    "ui.transfers.components.plan",
    "ui.transfers.list",
    "ui.transfers.pair",
    "ui.transfers.review_context",
    "ui.transfers.rules.list",
    "ui.transfers.suggest",
    "ui.wallets.analyze_file",
    "ui.wallets.identify",
    "ui.wallets.list",
    "ui.wallets.sync",
    "ui.workspace.health",
];

/// `REVIEW_TOOL_NAMES`: a quarantine investigation's bounded pack.
pub const REVIEW_TOOL_NAMES: [&str; 35] = [
    "ui.attachments.list",
    "ui.audit.evidence.summary",
    "ui.custody.coverage.snapshot",
    "ui.custody.gaps.history",
    "ui.custody.gaps.list",
    "ui.custody.gaps.review_context",
    "ui.custody.lineage.snapshot",
    "ui.custody.review.apply",
    "ui.custody.review.plan",
    "ui.documents.list",
    "ui.journals.process",
    "ui.journals.quarantine",
    "ui.journals.quarantine.resolve",
    "ui.journals.snapshot",
    "ui.rates.coverage",
    "ui.report.blockers",
    "ui.review.apply",
    "ui.review.cases",
    "ui.review.plan",
    "ui.review.receipt",
    "ui.review.request_input",
    "ui.review.worklist",
    "ui.transactions.graph",
    "ui.transactions.history",
    "ui.transactions.list",
    "ui.transactions.resolve",
    "ui.transactions.review_context",
    "ui.transfers.components.apply",
    "ui.transfers.components.list",
    "ui.transfers.components.plan",
    "ui.transfers.list",
    "ui.transfers.pair",
    "ui.transfers.review_context",
    "ui.wallets.identify",
    "ui.wallets.list",
];

/// `_CORE_TOOL_NAMES`: the tools of the `core` capability pack.
pub const CORE_CAPABILITY_TOOL_NAMES: [&str; 6] = [
    "read_skill_reference",
    "status",
    "ui.next_actions",
    "ui.overview.snapshot",
    "ui.report.blockers",
    "ui.workspace.health",
];

/// Phrases that ask for the full catalog.
const DISCOVERY_PHRASES: [&str; 4] = [
    "what can you do",
    "all tools",
    "capabilities",
    "tool catalog",
];

/// Phrases that make a turn a review request.
const REVIEW_PHRASES: [&str; 6] = [
    "quarantine",
    "quarantäne",
    "quarantaene",
    "review cases",
    "review receipt",
    "accounting review",
];

/// Terse continuations that keep the last review checkpoint's pack.
const CONTINUATIONS: [&str; 6] = [
    "continue",
    "please continue",
    "weiter",
    "bitte weiter",
    "fortsetzen",
    "bitte fortsetzen",
];

/// The keyword groups of `select_tool_capabilities`, in Python's order.
const KEYWORD_GROUPS: [(&str, &[&str]); 10] = [
    (
        "workspace",
        &[
            "all books",
            "book set",
            "books set",
            "treasury",
            "across books",
            "workspace overview",
            "alle bücher",
            "alle buecher",
            "buchset",
            "buch-set",
            "über alle bücher",
            "ueber alle buecher",
            "gesamtvermögen",
            "gesamtvermoegen",
        ],
    ),
    (
        "transactions",
        &[
            "transaction",
            "txid",
            "note",
            "tag",
            "metadata",
            "evidence",
            "attachment",
            "quarantine",
            "edit",
        ],
    ),
    (
        "reports",
        &[
            "report",
            "summary",
            "total",
            "journal",
            "tax",
            "gain",
            "balance",
            "portfolio",
            "price",
            "rate",
            "export",
            "steuer",
            "e1kv",
            "exit tax",
            "lightning",
            "routing",
            "profitable",
            "profitability",
        ],
    ),
    (
        "wallets",
        &[
            "wallet",
            "backend",
            "sync",
            "source",
            "utxo",
            "connection",
            "csv",
            "xlsx",
            "export",
            "import",
            "spreadsheet",
            "exchange",
            "broker",
            "datei",
            "tabelle",
            "börse",
            "boerse",
            "node",
            "channel",
        ],
    ),
    (
        "loans",
        &[
            "loan",
            "collateral",
            "borrowed",
            "principal",
            "liquidation",
            "darlehen",
            "kredit",
        ],
    ),
    (
        "privacy",
        &[
            "privacy",
            "linkable",
            "egress",
            "outbound",
            "psbt",
            "chain-analysis",
            "chain analysis",
            "chainanalysis",
            "chainanalyse",
            "trace",
            "tracing",
            "entropy",
            "clustering",
            "payjoin",
        ],
    ),
    (
        "source_funds",
        &[
            "source of funds",
            "source-of-funds",
            "provenance",
            "audit package",
            "proof of funds",
            "mittelherkunft",
            "herkunftsnachweis",
            "herkunft der mittel",
        ],
    ),
    (
        "merchant",
        &[
            "btcpay",
            "invoice",
            "receipt",
            "merchant",
            "commercial",
            "document",
        ],
    ),
    (
        "transfers",
        &[
            "quarantine",
            "quarantäne",
            "quarantaene",
            "transfer",
            "swap",
            "payout",
            "boltz",
            "peg",
            "coinjoin",
            "whirlpool",
            "pair",
            "custody",
            "missing wallet",
            "wallet roll",
            "samourai",
            "basis continuity",
            "saved view",
            "saved views",
            "saved filter",
            "gespeicherte ansicht",
        ],
    ),
    (
        "operations",
        &[
            "health",
            "pending",
            "next",
            "ready",
            "stale",
            "maintenance",
            "diagnose",
            "broken",
            "failed",
            "review",
            "worklist",
            "unresolved",
            "to do",
            "todo",
        ],
    ),
];

/// A capability selection: `None` advertises the whole catalog.
pub type Capabilities = Option<BTreeSet<&'static str>>;

/// `tool_capabilities(tool)`: the packs that advertise a tool.
pub fn tool_capabilities(tool: &ToolEntry) -> BTreeSet<&'static str> {
    let name = tool.name.as_str();
    let mut capabilities = BTreeSet::new();
    if REVIEW_TOOL_NAMES.contains(&name) {
        capabilities.insert("review");
    }
    if CORE_CAPABILITY_TOOL_NAMES.contains(&name) {
        capabilities.insert("core");
    }
    if ["ui.profiles.snapshot", "ui.workspace.overview.snapshot"].contains(&name) {
        capabilities.insert("workspace");
    }
    if ["ui.transactions.", "ui.activity.", "ui.attachments."]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        capabilities.insert("transactions");
    }
    if [
        "ui.audit.evidence.summary",
        "ui.review.badges",
        "ui.review.worklist",
    ]
    .contains(&name)
    {
        capabilities.extend(["transactions", "operations"]);
    }
    if name.starts_with("ui.loans.") {
        capabilities.insert("loans");
    }
    if name.starts_with("ui.reports.")
        || name.starts_with("ui.rates.")
        || ["ui.report.blockers", "ui.audit.changes_since_last_answer"].contains(&name)
    {
        capabilities.insert("reports");
    }
    if name.starts_with("ui.wallets.")
        || name.starts_with("ui.backends.")
        || [
            "ui.maintenance.settings",
            "ui.maintenance.configure",
            "ui.maintenance.run",
            "ui.connections.node.snapshot",
        ]
        .contains(&name)
    {
        capabilities.extend(["wallets", "operations"]);
    }
    if name.contains("privacy")
        || name == "ui.egress.snapshot"
        || name.starts_with("ui.chain_analysis.")
    {
        capabilities.insert("privacy");
    }
    if name.starts_with("ui.source_funds.") {
        capabilities.insert("source_funds");
    }
    if name.starts_with("ui.btcpay.")
        || name.starts_with("ui.documents.")
        || name == "ui.transactions.commercial_context"
    {
        capabilities.insert("merchant");
    }
    if ["ui.transfers.", "ui.saved_views.", "ui.custody."]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || ["ui.journals.transfers.list", "ui.wallets.identify"].contains(&name)
    {
        capabilities.insert("transfers");
    }
    if name.starts_with("ui.journals.") {
        capabilities.extend(["transactions", "reports"]);
    }
    if capabilities.is_empty() {
        capabilities.insert("core");
    }
    capabilities
}

fn dict_keys(value: &Value) -> BTreeSet<&PyStr> {
    match value {
        Value::Dict(pairs) => pairs
            .iter()
            .filter_map(|(key, _)| match key {
                Key::Str(key) => Some(key),
                _ => None,
            })
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn is_nonempty_dict(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Dict(pairs)) if !pairs.is_empty())
}

/// `json.loads` on a fenced block; `None` where Python raises
/// `ValueError` or `RecursionError`.
fn loads_block(text: &str) -> Option<Value> {
    kassiber_pytext::loads(text).ok()
}

/// `_continues_review_checkpoint(messages, latest)`.
fn continues_review_checkpoint(messages: &[Value], latest: &str) -> bool {
    let trimmed = kassiber_pytext::strip(latest).trim_end_matches(['.', '!', '?']);
    if !CONTINUATIONS.contains(&trimmed) {
        return false;
    }
    let content = messages
        .iter()
        .rev()
        .find(|message| matches!(get(message, "role"), Some(Value::Str(role)) if role.is("assistant")))
        .map(|message| get(message, "content"));
    let Some(Some(Value::Str(content))) = content else {
        return false;
    };
    let content = lossy(content);
    let tail = pyre::tail_chars(&content, 16_384);
    for block in pyre::review_checkpoint_blocks(tail) {
        let Some(payload) = loads_block(&block) else {
            continue;
        };
        let keys = dict_keys(&payload);
        if matches!(payload, Value::Dict(_))
            && keys.len() == 1
            && keys
                .iter()
                .next()
                .is_some_and(|key| key.is("review_checkpoint"))
            && is_nonempty_dict(get(&payload, "review_checkpoint"))
        {
            return true;
        }
    }
    false
}

/// The lower-cased text of the last three user messages with string content.
fn recent_user_messages(messages: &[Value]) -> Vec<String> {
    let all: Vec<String> = messages
        .iter()
        .filter(|message| matches!(get(message, "role"), Some(Value::Str(role)) if role.is("user")))
        .filter_map(|message| match get(message, "content") {
            Some(Value::Str(content)) => Some(kassiber_pytext::lower(&lossy(content))),
            _ => None,
        })
        .collect();
    let start = all.len().saturating_sub(3);
    all[start..].to_vec()
}

/// `select_tool_capabilities(messages, screen_context)`.
pub fn select_tool_capabilities(
    messages: Option<&[Value]>,
    screen_context: Option<&Value>,
) -> Capabilities {
    let messages = messages?;
    let recent = recent_user_messages(messages);
    let latest = recent.last().cloned().unwrap_or_default();
    if DISCOVERY_PHRASES
        .iter()
        .any(|phrase| latest.contains(phrase))
    {
        return None;
    }
    let mut selected = BTreeSet::from(["core"]);
    let context_field = |key: &str| screen_context.and_then(|context| get(context, key));
    let route = match context_field("route") {
        Some(route) if kassiber_envelope::truthy(route) => kassiber_pytext::lower(&py_str(route)),
        _ => String::new(),
    };
    if let Some(Value::List(requested)) = context_field("capabilities") {
        for item in requested {
            let item = py_str(item);
            if let Some(&name) = TOOL_CAPABILITY_NAMES.iter().find(|name| **name == item) {
                selected.insert(name);
            }
        }
    }
    let haystack = format!("{route} {}", recent.join(" "));
    if continues_review_checkpoint(messages, &latest)
        || REVIEW_PHRASES.iter().any(|word| haystack.contains(word))
    {
        // Evidence, wallet and report words within a repair request should
        // not expand it into every administrative/export capability.
        selected.insert("review");
        return Some(selected);
    }
    for (capability, keywords) in KEYWORD_GROUPS {
        if keywords.iter().any(|keyword| haystack.contains(keyword)) {
            selected.insert(capability);
        }
    }
    if selected.contains("transfers") {
        selected.insert("transactions");
    }
    if route == "/books" {
        selected.insert("wallets");
    }
    Some(selected)
}

/// `responses_tool_definitions(include_mutating=..., capabilities=...,
/// allowed_names=...)`.
pub fn responses_tool_definitions(
    include_mutating: bool,
    capabilities: &Capabilities,
    allowed_names: Option<&[&str]>,
) -> Vec<&'static ToolEntry> {
    tools()
        .iter()
        .filter(|tool| include_mutating || tool.kind_class == KindClass::ReadOnly)
        .filter(|tool| allowed_names.is_none_or(|names| names.contains(&tool.name.as_str())))
        .filter(|tool| {
            capabilities
                .as_ref()
                .is_none_or(|selected| !tool_capabilities(tool).is_disjoint(selected))
        })
        .collect()
}

/// `build_responses_tools(messages, screen_context=..., profile=...)`.
pub fn build_responses_tools(
    messages: Option<&[Value]>,
    screen_context: Option<&Value>,
    profile: Option<&str>,
) -> Result<Vec<&'static ToolEntry>, AppError> {
    if profile.is_some_and(|profile| !TOOL_PROFILE_NAMES.contains(&profile)) {
        return Err(AppError::new("validation", "unknown AI tool profile"));
    }
    let empty: [Value; 0] = [];
    let selected_messages = if profile == Some("core") && messages.is_none() {
        Some(&empty[..])
    } else {
        messages
    };
    let capabilities = if profile == Some("full") {
        None
    } else {
        select_tool_capabilities(selected_messages, screen_context)
    };
    let allowed = (profile == Some("core")).then_some(&CORE_TOOL_NAMES[..]);
    Ok(responses_tool_definitions(true, &capabilities, allowed))
}

/// The tools only an on-device provider is offered: the chat loop strips
/// them from every other provider's list (`daemon.py:9462`).
pub const ON_DEVICE_WIRE_NAMES: [&str; 6] = [
    "ui_chain_analysis_acquire_plan",
    "ui_chain_analysis_acquire_apply",
    "ui_chain_analysis_datasets_preview",
    "ui_chain_analysis_datasets_import",
    "ui_chain_analysis_datasets_preview_start",
    "ui_chain_analysis_datasets_import_start",
];

/// The wire-name prefix of the recurring-source tools, all on-device only.
pub const ON_DEVICE_WIRE_PREFIX: &str = "ui_chain_analysis_sources_";

/// Whether a provider-visible name is offered only to on-device providers.
pub fn on_device_only(wire_name: &str) -> bool {
    wire_name.starts_with(ON_DEVICE_WIRE_PREFIX) || ON_DEVICE_WIRE_NAMES.contains(&wire_name)
}

/// The on-device strip: other providers never see these tools.
pub fn strip_on_device_only(
    tools: Vec<&'static ToolEntry>,
    on_device: bool,
) -> Vec<&'static ToolEntry> {
    if on_device {
        return tools;
    }
    tools
        .into_iter()
        .filter(|tool| !on_device_only(tool.provider_name()))
        .collect()
}
