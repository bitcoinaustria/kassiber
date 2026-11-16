//! Source of funds: the pure pieces of routes/source-funds (model.ts, caseScope.ts, journey.ts,
//! findingCopy.ts, provenanceGraph.ts and the evidence graph's layered layout). Payloads stay
//! `serde_json::Value`s as the daemon sends them; every helper here is presentation or request
//! shaping only, never a provenance decision.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use serde_json::{Map, Value, json};

pub const SOURCE_TYPES: &[&str] = &[
    "fiat_purchase",
    "exchange_withdrawal",
    "mining",
    "income",
    "gift",
    "opening_balance_attestation",
    "missing_history",
    "unknown",
];
pub const LINK_TYPES: &[&str] = &[
    "self_transfer",
    "exchange_transfer",
    "trade",
    "swap",
    "peg_in",
    "peg_out",
    "lightning_funding",
    "lightning_close",
    "lightning_routed",
    "lightning_swap",
    "coinjoin",
    "payjoin",
    "manual_source",
    "missing_history",
];
pub const CONFIDENCE_LEVELS: &[&str] = &["exact", "strong", "weak", "unknown"];
pub const REVEAL_MODES: &[&str] = &["labels_only", "minimal", "standard", "full"];
pub const NO_ATTACHMENT: &str = "__none__";
pub const COVERAGE_BUCKETS: &[&str] = &[
    "fully_traced",
    "attested",
    "unattributed",
    "in_review",
    "untraced",
    "not_classified",
];
/// The gap actions a finding card can dispatch (`GateRow`).
pub const GAP_ACTIONS: &[&str] = &[
    "open_source_creator",
    "open_link_review",
    "open_review_queue",
    "open_source",
    "open_transaction",
];

// ---- Small readers ---------------------------------------------------------------------------

pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

pub fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

pub fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// First non-empty string among `keys`.
pub fn first<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .map(|k| s(v, k))
        .find(|x| !x.is_empty())
        .unwrap_or("")
}

/// `value.replaceAll("_", " ")`.
pub fn pretty(v: &str) -> String {
    v.replace('_', " ")
}

/// `shortId`: `first10...last6` above 18 characters, `-` when empty.
pub fn short_id(v: &str) -> String {
    if v.is_empty() {
        return "-".into();
    }
    let n = v.chars().count();
    if n > 18 {
        let head: String = v.chars().take(10).collect();
        let tail: String = v.chars().skip(n - 6).collect();
        format!("{head}...{tail}")
    } else {
        v.to_owned()
    }
}

/// `formatBtc`: `toFixed(8) ASSET`, `-` without a number.
pub fn format_btc(v: Option<f64>, asset: &str) -> String {
    match v {
        Some(x) => format!("{x:.8} {}", if asset.is_empty() { "BTC" } else { asset }),
        None => "-".into(),
    }
}

/// `formatDateTime`: `YYYY-MM-DD HH:MM`, `-` when empty.
pub fn format_datetime(v: &str) -> String {
    if v.is_empty() {
        return "-".into();
    }
    v.replacen('T', " ", 1)
        .replacen('Z', "", 1)
        .chars()
        .take(16)
        .collect()
}

/// An exact millisatoshi integer from a JSON number or string.
pub fn msat(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| n.as_f64().filter(|x| x.fract() == 0.0).map(|x| x as i128)),
        Value::String(t) if !t.trim().is_empty() => t.trim().parse().ok(),
        _ => None,
    }
}

/// `amountInput` / `msatToBtcInput`: exact BTC text for an msat integer (trailing zeros cut),
/// `""` when there is no amount.
pub fn amount_input(v: &Value) -> String {
    let Some(m) = msat(v) else {
        return String::new();
    };
    let neg = m < 0;
    let a = m.unsigned_abs();
    let whole = a / 100_000_000_000;
    let frac = format!("{:011}", a % 100_000_000_000);
    let frac = frac.trim_end_matches('0');
    format!(
        "{}{whole}{}",
        if neg { "-" } else { "" },
        if frac.is_empty() {
            String::new()
        } else {
            format!(".{frac}")
        }
    )
}

// ---- Transaction rows (TransactionRow) -------------------------------------------------------

pub fn tx_ref(r: &Value) -> String {
    first(r, &["id", "transaction_id", "external_id"]).to_owned()
}

pub fn tx_wallet(r: &Value) -> String {
    let w = first(r, &["wallet", "account"]);
    if w.is_empty() {
        "Wallet".into()
    } else {
        w.to_owned()
    }
}

pub fn tx_direction(r: &Value) -> String {
    let d = first(r, &["direction", "type"]);
    if d.is_empty() {
        "transaction".into()
    } else {
        d.to_lowercase()
    }
}

pub fn tx_label(r: &Value) -> String {
    let date: String = first(r, &["occurred_at", "date"])
        .chars()
        .take(10)
        .collect();
    let kind = first(r, &["direction", "type"]);
    let kind = if kind.is_empty() { "transaction" } else { kind };
    let id = first(r, &["external_id", "externalId", "id"]);
    [date, tx_wallet(r), kind.to_owned(), id.to_owned()]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `txFlow`: incoming / outgoing / transfer / swap.
pub fn tx_flow(r: &Value) -> &'static str {
    match s(r, "flow") {
        "incoming" => return "incoming",
        "outgoing" => return "outgoing",
        "transfer" => return "transfer",
        "swap" => return "swap",
        _ => {}
    }
    let ty = s(r, "type").to_lowercase();
    let tag = s(r, "tag").to_lowercase();
    if tag.contains("swap") || ty == "swap" || ty == "mint" || ty == "melt" {
        return "swap";
    }
    if r.get("internal").and_then(Value::as_bool) == Some(true)
        || ty == "transfer"
        || ty == "rebalance"
    {
        return "transfer";
    }
    if let Some(sat) = f(r, "amountSat") {
        return if sat >= 0.0 { "incoming" } else { "outgoing" };
    }
    let d = tx_direction(r);
    if d.contains("inbound") || d.contains("receive") {
        return "incoming";
    }
    if d.contains("outbound") || d.contains("send") {
        return "outgoing";
    }
    match f(r, "amount") {
        Some(a) if a < 0.0 => "outgoing",
        Some(a) if a > 0.0 => "incoming",
        _ => "transfer",
    }
}

/// `txSignedAmount`: `+0.01000000 BTC` for incoming, `-…` outgoing, unsigned otherwise.
pub fn tx_signed_amount(r: &Value) -> String {
    let asset = {
        let a = s(r, "asset");
        if a.is_empty() { "BTC" } else { a }
    };
    let sign = match tx_flow(r) {
        "incoming" => "+",
        "outgoing" => "-",
        _ => "",
    };
    let v = f(r, "amount")
        .or_else(|| f(r, "amountSat").map(|x| x / 1e8))
        .or_else(|| f(r, "amount_msat").map(|x| x / 1e11));
    match v {
        Some(x) => format!("{sign}{:.8} {asset}", x.abs()),
        None => asset.to_owned(),
    }
}

/// `txDate`: the day, or "Unknown date".
pub fn tx_date(r: &Value) -> String {
    let d: String = first(r, &["occurred_at", "date"])
        .chars()
        .take(10)
        .collect();
    if d.is_empty() {
        "Unknown date".into()
    } else {
        d
    }
}

/// `transactionRows`: `{txs}` / `{transactions}` / a bare array.
pub fn transaction_rows(data: &Value) -> Vec<Value> {
    if let Some(a) = data.get("txs").and_then(Value::as_array) {
        return a.clone();
    }
    if let Some(a) = data.get("transactions").and_then(Value::as_array) {
        return a.clone();
    }
    data.as_array().cloned().unwrap_or_default()
}

pub fn unique_sorted(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = values
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    v.sort_by_key(|a| a.to_lowercase());
    v
}

// ---- Target picker (caseScope.ts) -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filters {
    pub query: String,
    pub flow: String,
    pub date: String,
    pub status: String,
    pub network: String,
    pub asset: String,
    pub wallet: String,
}

impl Default for Filters {
    fn default() -> Self {
        let all = || "all".to_owned();
        Filters {
            query: String::new(),
            flow: all(),
            date: all(),
            status: all(),
            network: all(),
            asset: all(),
            wallet: all(),
        }
    }
}

impl Filters {
    pub fn active(&self) -> bool {
        !self.query.is_empty()
            || [
                &self.flow,
                &self.date,
                &self.status,
                &self.network,
                &self.asset,
                &self.wallet,
            ]
            .iter()
            .any(|v| v.as_str() != "all")
    }
}

/// An ISO timestamp (`toISOString`, millisecond precision, `Z`) for epoch milliseconds.
pub fn iso_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let day = crate::format::Day::from_epoch_days(days).iso();
    let (h, m, sec, milli) = (
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000,
    );
    format!("{day}T{h:02}:{m:02}:{sec:02}.{milli:03}Z")
}

/// `targetQueryArgs`: the paginated `ui.transactions.list` contract. `now_ms` is the current
/// time, `offset_s` the local UTC offset (local midnight bounds the date ranges, as `Date` does).
pub fn target_query_args(fl: &Filters, now_ms: i64, offset_s: i64) -> Value {
    let mut a = json!({"limit": 100, "sort": "occurred-at", "order": "desc"});
    let q = fl.query.trim();
    if !q.is_empty() {
        a["query"] = json!(q);
    }
    for (k, v) in [
        ("flow", &fl.flow),
        ("status", &fl.status),
        ("network", &fl.network),
        ("asset", &fl.asset),
        ("wallet", &fl.wallet),
    ] {
        if v != "all" {
            a[k] = json!(v);
        }
    }
    // `start.setHours(0,0,0,0)` in local time, then back `days`.
    let local = now_ms + offset_s * 1000;
    let midnight = local - local.rem_euclid(86_400_000) - offset_s * 1000;
    let days = match fl.date.as_str() {
        "yesterday" => 1,
        "7days" => 7,
        "30days" | "older" => 30,
        _ => 0,
    };
    let start = midnight - days * 86_400_000;
    if fl.date == "older" {
        a["until"] = json!(iso_ms(start - 1));
    } else if fl.date != "all" {
        a["since"] = json!(iso_ms(start));
        if fl.date == "yesterday" {
            a["until"] = json!(iso_ms(start + 86_400_000 - 1));
        }
    }
    a
}

/// `sourceFundsDraftKey`: a database path alone does not distinguish profiles in one book.
pub fn draft_key(database: &str, workspace: &str, profile: &str) -> String {
    json!(["source-funds-v2", database, workspace, profile]).to_string()
}

// ---- Journey (journey.ts) ---------------------------------------------------------------------

/// A review click applies to a current, fully rendered disclosure preview only.
pub fn can_approve(
    current: bool,
    exportable: bool,
    diagram_loading: bool,
    diagram_error: bool,
    fingerprint: Option<&str>,
) -> bool {
    current
        && exportable
        && !diagram_loading
        && !diagram_error
        && fingerprint.is_some_and(|f| !f.is_empty())
}

/// Navigation history and saved exports never approve a changed current case.
pub fn is_reviewed(
    current: bool,
    exportable: bool,
    fingerprint: Option<&str>,
    reviewed: Option<&str>,
) -> bool {
    current && exportable && fingerprint.is_some_and(|f| !f.is_empty()) && fingerprint == reviewed
}

// ---- Link review (model.ts) -------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkForm {
    pub link_type: String,
    pub confidence: String,
    pub allocation_amount: String,
    pub from_allocation_amount: String,
    pub explanation: String,
    pub attachment_id: String,
}

impl Default for LinkForm {
    fn default() -> Self {
        LinkForm {
            link_type: "self_transfer".into(),
            confidence: "strong".into(),
            allocation_amount: String::new(),
            from_allocation_amount: String::new(),
            explanation: String::new(),
            attachment_id: NO_ATTACHMENT.into(),
        }
    }
}

/// `linkReviewFormFromLink`: built from exact stored msat, never the 8-decimal projection.
pub fn link_form_from(link: &Value) -> LinkForm {
    LinkForm {
        link_type: s(link, "link_type").to_owned(),
        confidence: s(link, "confidence").to_owned(),
        allocation_amount: amount_input(&link["allocation_amount_msat"]),
        from_allocation_amount: amount_input(&link["from_allocation_amount_msat"]),
        explanation: s(link, "explanation").to_owned(),
        attachment_id: NO_ATTACHMENT.into(),
    }
}

/// `linkReviewPayload`: only what the reviewer deliberately changed, bound to the inspected
/// amounts; a rejection never carries an allocation.
pub fn link_review_payload(link: &Value, form: &LinkForm, state: &str) -> Value {
    let base = link_form_from(link);
    let mut p = Map::new();
    p.insert("link".into(), json!(s(link, "id")));
    p.insert("state".into(), json!(state));
    if !link["allocation_amount_msat"].is_null() && link.get("allocation_amount_msat").is_some() {
        p.insert(
            "expected_allocation_amount_msat".into(),
            link["allocation_amount_msat"].clone(),
        );
    }
    if !link["from_allocation_amount_msat"].is_null()
        && link.get("from_allocation_amount_msat").is_some()
    {
        p.insert(
            "expected_from_allocation_amount_msat".into(),
            link["from_allocation_amount_msat"].clone(),
        );
    }
    if form.link_type != base.link_type {
        p.insert("link_type".into(), json!(form.link_type));
    }
    if form.confidence != base.confidence {
        p.insert("confidence".into(), json!(form.confidence));
    }
    if form.explanation.trim() != base.explanation.trim() {
        p.insert("explanation".into(), json!(form.explanation));
    }
    if state == "reviewed" {
        if form.allocation_amount.trim() != base.allocation_amount.trim() {
            p.insert("allocation_amount".into(), json!(form.allocation_amount));
        }
        if form.from_allocation_amount.trim() != base.from_allocation_amount.trim() {
            p.insert(
                "from_allocation_amount".into(),
                json!(form.from_allocation_amount),
            );
        }
        p.insert("allocation_policy".into(), json!("explicit"));
    }
    Value::Object(p)
}

/// True when the form still shows exactly what the link says.
pub fn form_matches_link(form: &LinkForm, link: &Value) -> bool {
    let b = link_form_from(link);
    form.link_type == b.link_type
        && form.confidence == b.confidence
        && form.allocation_amount.trim() == b.allocation_amount.trim()
        && form.from_allocation_amount.trim() == b.from_allocation_amount.trim()
        && form.explanation.trim() == b.explanation.trim()
}

/// `reconcileLinkForm`: a pristine form refreshes; a dirty one keeps its edits AND the record it
/// was built from, so a stale write is refused instead of silently winning.
pub fn reconcile_link_form(
    form: &LinkForm,
    inspected: Option<&Value>,
    latest: &Value,
) -> (LinkForm, Value) {
    match inspected {
        Some(i) if s(i, "id") == s(latest, "id") && !form_matches_link(form, i) => {
            (form.clone(), i.clone())
        }
        _ => (link_form_from(latest), latest.clone()),
    }
}

/// The source / manual-link editors' text, for the draft check.
pub struct Drafts<'a> {
    pub source_label: &'a str,
    pub source_amount: &'a str,
    pub source_description: &'a str,
    pub source_attachment: &'a str,
    pub manual_from: &'a str,
    pub manual_allocation: &'a str,
    pub manual_from_allocation: &'a str,
    pub manual_explanation: &'a str,
    pub manual_attachment: &'a str,
}

/// `hasUnsavedDrafts`: auto-assembly waits while any editor holds work in progress.
pub fn has_unsaved_drafts(link_form: &LinkForm, inspected: Option<&Value>, d: &Drafts) -> bool {
    if inspected.is_some_and(|i| !form_matches_link(link_form, i)) {
        return true;
    }
    if !d.source_label.trim().is_empty()
        || !d.source_amount.trim().is_empty()
        || !d.source_description.trim().is_empty()
        || d.source_attachment != NO_ATTACHMENT
    {
        return true;
    }
    !d.manual_from.trim().is_empty()
        || !d.manual_allocation.trim().is_empty()
        || !d.manual_from_allocation.trim().is_empty()
        || !d.manual_explanation.trim().is_empty()
        || d.manual_attachment != NO_ATTACHMENT
}

/// `autoAssembleKey`: per book, target and journal input version.
pub fn auto_assemble_key(profile: &str, target: &str, input_version: Option<i64>) -> String {
    format!(
        "{profile}:{target}:{}",
        input_version.map(|v| v.to_string()).unwrap_or_default()
    )
}

/// `shouldAutoAssemble`: once per attempt, only with no reviewed history, never while busy.
pub fn should_auto_assemble(
    target: &str,
    reviewed_edges: usize,
    busy: bool,
    key: &str,
    done: &HashSet<String>,
) -> bool {
    !target.is_empty() && !busy && !done.contains(key) && reviewed_edges == 0
}

// ---- Findings (findingCopy.ts) ----------------------------------------------------------------

/// A finding's English (title, next step, messages by key).
type FindingCopy = (String, String, BTreeMap<String, String>);

fn english_findings() -> &'static HashMap<String, FindingCopy> {
    static T: OnceLock<HashMap<String, FindingCopy>> = OnceLock::new();
    T.get_or_init(|| {
        let v: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/i18n/locales/en/sourceFunds.json"
        )))
        .unwrap_or(Value::Null);
        let mut out = HashMap::new();
        if let Some(m) = v["findings"].as_object() {
            for (code, x) in m {
                let msgs = x["messages"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                            .collect()
                    })
                    .unwrap_or_default();
                out.insert(
                    code.clone(),
                    (s(x, "title").to_owned(), s(x, "nextStep").to_owned(), msgs),
                );
            }
        }
        out
    })
}

/// `findingTranslationKeys`: (title, message, next step) keys; unknown variants stay verbatim.
pub fn finding_keys(finding: &Value) -> (Option<String>, Option<String>, Option<String>) {
    let code = s(finding, "code");
    let Some((_, next, msgs)) = english_findings().get(code) else {
        return (None, None, None);
    };
    let message = msgs
        .iter()
        .find(|(_, m)| m.as_str() == s(finding, "message"))
        .map(|(k, _)| format!("sourceFunds:findings.{code}.messages.{k}"));
    let headline = finding["next_step"]["headline"]
        .as_str()
        .map(str::trim)
        .unwrap_or("");
    let next_key =
        (next.as_str() == headline).then(|| format!("sourceFunds:findings.{code}.nextStep"));
    (
        Some(format!("sourceFunds:findings.{code}.title")),
        message,
        next_key,
    )
}

// ---- Provenance graph (provenanceGraph.ts) ----------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct GNode {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub title: String,
    /// `positive` (an origin) / `muted` (a deferred privacy hop) / default.
    pub tone: &'static str,
    pub role: String,
    pub node_type: String,
    pub transaction_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub label: String,
    pub title: String,
    /// A deferred privacy hop: warning colour, dashed.
    pub warning: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<GNode>,
    pub edges: Vec<GEdge>,
    pub subject: Option<String>,
}

/// Presentation adapter only: reachability, allocations and privacy stops come from the report;
/// level adjacency never implies an edge.
pub fn provenance_graph(
    flow: &Value,
    role_label: &dyn Fn(&str) -> String,
    direction_label: &dyn Fn(&str) -> String,
    link_label: &dyn Fn(&str) -> String,
) -> Graph {
    let mut g = Graph::default();
    let mut ids: HashSet<String> = HashSet::new();
    for level in arr(flow, "levels") {
        let role = {
            let r = s(level, "role");
            if r.is_empty() { "flow" } else { r }
        };
        for n in arr(level, "nodes") {
            let id = s(n, "id").to_owned();
            if !ids.insert(id.clone()) {
                // `new Map(...)`: a later level's entry for the same id wins its place's data.
                g.nodes.retain(|x| x.id != id);
            }
            let kind = s(n, "kind");
            let label = if !s(n, "wallet").is_empty() {
                let dir = if kind == "inbound" || kind == "outbound" {
                    direction_label(kind)
                } else {
                    String::new()
                };
                [s(n, "wallet").to_owned(), dir]
                    .into_iter()
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ")
            } else if !s(n, "label").is_empty() {
                s(n, "label").to_owned()
            } else {
                id.clone()
            };
            let asset = {
                let a = s(n, "asset");
                if a.is_empty() { "BTC" } else { a }
            };
            let amount = format_btc(f(n, "amount"), asset);
            let mut parts: Vec<String> = Vec::new();
            for p in [
                role_label(role),
                label.clone(),
                s(n, "label").to_owned(),
                amount.clone(),
            ] {
                if !p.is_empty() && !parts.contains(&p) {
                    parts.push(p);
                }
            }
            let tone = if n["deferred_privacy_hop"].as_bool() == Some(true) {
                "muted"
            } else if s(n, "node_type") == "source" {
                "positive"
            } else {
                "default"
            };
            g.nodes.push(GNode {
                id,
                label,
                detail: amount,
                title: parts.join(" · "),
                tone,
                role: role.to_owned(),
                node_type: s(n, "node_type").to_owned(),
                transaction_id: s(n, "transaction_id").to_owned(),
            });
        }
    }
    for e in arr(flow, "edges") {
        let (id, from, to) = (s(e, "id"), s(e, "from"), s(e, "to"));
        if id.is_empty()
            || from.is_empty()
            || to.is_empty()
            || !ids.contains(from)
            || !ids.contains(to)
        {
            continue;
        }
        let ll = link_label(s(e, "link_type"));
        let label = if ll.is_empty() {
            role_label("flow")
        } else {
            ll
        };
        let warning = e["deferred_privacy_hop"].as_bool() == Some(true);
        let asset = {
            let a = s(e, "asset");
            if a.is_empty() { "BTC" } else { a }
        };
        let title = [
            label.clone(),
            format_btc(f(e, "amount"), asset),
            if warning {
                role_label("privacy_boundary")
            } else {
                String::new()
            },
        ]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        g.edges.push(GEdge {
            id: id.to_owned(),
            source: from.to_owned(),
            target: to.to_owned(),
            label,
            title,
            warning,
        });
    }
    g.subject = g
        .nodes
        .iter()
        .find(|n| n.role == "target")
        .map(|n| n.id.clone());
    g
}

/// `layoutEvidenceGraph`: stable breadth-first levels (graph units), 265 apart, rows 90 apart.
pub struct Layout {
    pub pos: HashMap<String, (f32, f32)>,
    pub width: f32,
    pub height: f32,
}

pub fn layout(g: &Graph) -> Layout {
    let ids: HashSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut indeg: HashMap<&str, usize> = g.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    for e in &g.edges {
        if !ids.contains(e.source.as_str())
            || !ids.contains(e.target.as_str())
            || e.source == e.target
        {
            continue;
        }
        adj.entry(e.source.as_str())
            .or_default()
            .push(e.target.as_str());
        *indeg.entry(e.target.as_str()).or_default() += 1;
    }
    let mut level: HashMap<&str, usize> = HashMap::new();
    let mut queue: Vec<&str> = g
        .nodes
        .iter()
        .map(|n| n.id.as_str())
        .filter(|id| indeg.get(id) == Some(&0))
        .collect();
    for id in &queue {
        level.insert(id, 0);
    }
    let mut i = 0;
    while i < queue.len() {
        let id = queue[i];
        for next in adj.get(id).cloned().unwrap_or_default() {
            if !level.contains_key(next) {
                let l = (level[id] + 1).min(50);
                level.insert(next, l);
                queue.push(next);
            }
        }
        i += 1;
    }
    let mut columns: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for n in &g.nodes {
        columns
            .entry(*level.get(n.id.as_str()).unwrap_or(&0))
            .or_default()
            .push(n.id.as_str());
    }
    let max_rows = columns.values().map(Vec::len).max().unwrap_or(1).max(1);
    let height = (max_rows as f32 * 90.0 + 100.0).max(380.0);
    let mut pos = HashMap::new();
    for (l, entries) in &columns {
        for (i, id) in entries.iter().enumerate() {
            pos.insert(
                (*id).to_owned(),
                (
                    145.0 + *l as f32 * 265.0,
                    (height - entries.len() as f32 * 90.0) / 2.0 + 45.0 + i as f32 * 90.0,
                ),
            );
        }
    }
    let last = columns.keys().max().copied().unwrap_or(0);
    Layout {
        pos,
        width: ((last + 1) as f32 * 265.0 + 40.0).max(700.0),
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(over: Value) -> Value {
        let mut l = json!({"id": "link-1", "to_transaction_id": "in", "link_type": "self_transfer", "state": "suggested", "confidence": "strong", "method": "custody_component", "asset": "BTC", "allocation_amount": 1e-11, "allocation_amount_msat": 1234, "from_allocation_amount": 1e-11, "from_allocation_amount_msat": 1234, "allocation_policy": "heuristic", "explanation": ""});
        for (k, v) in over.as_object().unwrap() {
            l[k] = v.clone();
        }
        l
    }

    #[test]
    fn exact_msat_amounts() {
        assert_eq!(amount_input(&json!(1234)), "0.00000001234");
        assert_eq!(amount_input(&Value::Null), "");
        assert_eq!(amount_input(&json!("")), "");
        assert_eq!(amount_input(&json!(100_000_000_000i64)), "1");
        assert_eq!(
            link_form_from(&link(json!({}))).allocation_amount,
            "0.00000001234"
        );
    }

    #[test]
    fn reviewing_is_not_editing() {
        let l = link(json!({}));
        let p = link_review_payload(&l, &link_form_from(&l), "reviewed");
        assert!(p.get("allocation_amount").is_none() && p.get("from_allocation_amount").is_none());
        assert_eq!(p["allocation_policy"], "explicit");
        assert_eq!(p["expected_allocation_amount_msat"], 1234);
        let mut form = link_form_from(&l);
        form.allocation_amount = "0.00000002".into();
        assert_eq!(
            link_review_payload(&l, &form, "reviewed")["allocation_amount"],
            "0.00000002"
        );
        form.confidence = "weak".into();
        let r = link_review_payload(&l, &form, "rejected");
        assert!(r.get("allocation_amount").is_none() && r.get("allocation_policy").is_none());
        assert_eq!(r["confidence"], "weak");
    }

    #[test]
    fn refreshed_link_cannot_be_approved_against_old_amounts() {
        let inspected = link(
            json!({"allocation_amount_msat": 1000, "from_allocation_amount_msat": 1000, "updated_at": "t1"}),
        );
        let refreshed = link(
            json!({"allocation_amount_msat": 2000, "from_allocation_amount_msat": 2000, "updated_at": "t2"}),
        );
        let (form, ins) =
            reconcile_link_form(&link_form_from(&inspected), Some(&inspected), &refreshed);
        assert_eq!(ins, refreshed);
        let p = link_review_payload(&ins, &form, "reviewed");
        assert_eq!(p["expected_allocation_amount_msat"], 2000);
        assert!(p.get("allocation_amount").is_none());
        let mut dirty = link_form_from(&inspected);
        dirty.allocation_amount = "0.00000003".into();
        let (form, ins) = reconcile_link_form(&dirty, Some(&inspected), &refreshed);
        assert_eq!(form, dirty);
        assert_eq!(
            link_review_payload(&ins, &form, "reviewed")["expected_allocation_amount_msat"],
            1000
        );
        let other = link(json!({"id": "link-2", "allocation_amount_msat": 5000}));
        assert_eq!(
            reconcile_link_form(&dirty, Some(&inspected), &other)
                .0
                .allocation_amount,
            amount_input(&json!(5000))
        );
    }

    #[test]
    fn auto_assembly_rules() {
        let none = HashSet::new();
        assert!(should_auto_assemble("tx-1", 0, false, "k", &none));
        assert!(!should_auto_assemble("tx-1", 3, false, "k", &none));
        assert!(!should_auto_assemble("tx-1", 0, true, "k", &none));
        assert!(!should_auto_assemble("", 0, false, "k", &none));
        let seen: HashSet<String> = [auto_assemble_key("book", "tx-1", Some(7))].into();
        assert!(!should_auto_assemble(
            "tx-1",
            0,
            false,
            &auto_assemble_key("book", "tx-1", Some(7)),
            &seen
        ));
        assert!(should_auto_assemble(
            "tx-1",
            0,
            false,
            &auto_assemble_key("book", "tx-1", Some(8)),
            &seen
        ));
        assert_ne!(
            auto_assemble_key("a", "t", Some(7)),
            auto_assemble_key("b", "t", Some(7))
        );
        let l = link(json!({"allocation_amount_msat": 1000, "from_allocation_amount_msat": 1000}));
        let d = Drafts {
            source_label: "",
            source_amount: "",
            source_description: "",
            source_attachment: NO_ATTACHMENT,
            manual_from: "",
            manual_allocation: "",
            manual_from_allocation: "",
            manual_explanation: "",
            manual_attachment: NO_ATTACHMENT,
        };
        assert!(!has_unsaved_drafts(&link_form_from(&l), Some(&l), &d));
        let mut lf = link_form_from(&l);
        lf.explanation = "half".into();
        assert!(has_unsaved_drafts(&lf, Some(&l), &d));
        assert!(has_unsaved_drafts(
            &link_form_from(&l),
            Some(&l),
            &Drafts {
                source_amount: "0.5",
                ..d
            }
        ));
    }

    #[test]
    fn query_args_and_scope() {
        let mut fl = Filters {
            query: " old-tx ".into(),
            ..Filters::default()
        };
        fl.flow = "transfer".into();
        fl.status = "review".into();
        fl.network = "liquid".into();
        fl.asset = "LBTC".into();
        fl.wallet = "Cold".into();
        assert_eq!(
            target_query_args(&fl, 0, 0),
            json!({"limit": 100, "sort": "occurred-at", "order": "desc", "query": "old-tx", "flow": "transfer", "status": "review", "network": "liquid", "asset": "LBTC", "wallet": "Cold"})
        );
        assert_eq!(
            target_query_args(&Filters::default(), 0, 0),
            json!({"limit": 100, "sort": "occurred-at", "order": "desc"})
        );
        let now = 1_788_609_600_000; // 2026-09-05T12:00:00Z
        let y = target_query_args(
            &Filters {
                date: "yesterday".into(),
                ..Filters::default()
            },
            now,
            0,
        );
        assert_eq!(y["since"], "2026-09-04T00:00:00.000Z");
        assert_eq!(y["until"], "2026-09-04T23:59:59.999Z");
        let o = target_query_args(
            &Filters {
                date: "older".into(),
                ..Filters::default()
            },
            now,
            0,
        );
        assert!(o.get("since").is_none() && o.get("until").is_some());
        assert_ne!(
            draft_key("db:/one", "w", "p"),
            draft_key("db:/one", "w", "q")
        );
        assert_ne!(
            draft_key("db:/one", "w", "p"),
            draft_key("db:/two", "w", "p")
        );
    }

    #[test]
    fn journey_gates() {
        assert!(can_approve(true, true, false, false, Some("fp")));
        assert!(!can_approve(false, true, false, false, Some("fp")));
        assert!(!can_approve(true, true, true, false, Some("fp")));
        assert!(!can_approve(true, true, false, true, Some("fp")));
        assert!(!can_approve(true, true, false, false, None));
        assert!(!is_reviewed(true, true, Some("new"), None));
        assert!(!is_reviewed(true, true, Some("new"), Some("old")));
        assert!(!is_reviewed(false, true, Some("same"), Some("same")));
        assert!(is_reviewed(true, true, Some("same"), Some("same")));
    }

    #[test]
    fn finding_copy() {
        let missing = json!({"code": "missing_history", "severity": "blocker", "message": "The path stops at a transaction without a reviewed root source or missing-history attestation.", "next_step": {"headline": "Attach a root source or attest the gap", "action": "open_source_creator"}});
        let (title, msg, next) = finding_keys(&missing);
        assert_eq!(
            title.as_deref(),
            Some("sourceFunds:findings.missing_history.title")
        );
        assert!(msg.is_some() && next.is_some());
        let unknown = json!({"code": "missing_history", "message": "A new precise diagnostic", "next_step": {"headline": "A newly supplied instruction"}});
        let (_, msg, next) = finding_keys(&unknown);
        assert!(msg.is_none() && next.is_none());
        let amb = json!({"code": "ambiguous_allocation", "message": "The selected amount cannot follow the reviewed allocation ratio in exact millisatoshis."});
        assert_eq!(
            finding_keys(&amb).1.as_deref(),
            Some(
                "sourceFunds:findings.ambiguous_allocation.messages.selectedAmountNotExactlyRepresentable"
            )
        );
        assert_eq!(
            finding_keys(&json!({"code": "future_finding", "message": "x"})),
            (None, None, None)
        );
    }

    #[test]
    fn graph_adapter_never_invents_edges() {
        let flow = json!({"levels": [
            {"role": "source", "nodes": [{"id": "source:a", "label": "Purchase receipt", "node_type": "source", "amount": 0.006, "asset": "BTC"}]},
            {"role": "privacy_boundary", "nodes": [{"id": "tx:b", "label": "collaborative tx", "wallet": "Intermediate wallet", "node_type": "transaction", "transaction_id": "b", "deferred_privacy_hop": true, "amount": 0.006, "asset": "BTC"}]},
            {"role": "target", "nodes": [{"id": "tx:c", "label": "receipt hash", "wallet": "Own wallet", "node_type": "transaction", "transaction_id": "c", "amount": 0.006, "asset": "BTC"}]}],
            "edges": [{"id": "reviewed", "from": "tx:b", "to": "tx:c", "link_type": "payjoin", "deferred_privacy_hop": true, "amount": 0.006, "asset": "BTC"}]});
        let id = |x: &str| x.to_owned();
        let g = provenance_graph(&flow, &id, &id, &id);
        assert_eq!(
            g.edges
                .iter()
                .map(|e| (e.source.as_str(), e.target.as_str()))
                .collect::<Vec<_>>(),
            vec![("tx:b", "tx:c")]
        );
        assert!(g.edges[0].warning && g.edges[0].title.contains("privacy_boundary"));
        assert_eq!(g.subject.as_deref(), Some("tx:c"));
        let c = g.nodes.iter().find(|n| n.id == "tx:c").unwrap();
        assert_eq!(
            (c.label.as_str(), c.detail.as_str()),
            ("Own wallet", "0.00600000 BTC")
        );
        let mut dangling = flow.clone();
        dangling["edges"] = json!([{"id": "outside", "from": "hidden", "to": "tx:c"}]);
        assert!(provenance_graph(&dangling, &id, &id, &id).edges.is_empty());
        let l = layout(&g);
        assert!(l.pos["tx:c"].0 > l.pos["tx:b"].0);
    }

    #[test]
    fn rows() {
        let r = json!({"id": "x", "direction": "outbound", "amount": 0.5, "wallet": "W", "occurred_at": "2025-01-02T03:04:05Z", "external_id": "abc"});
        assert_eq!(tx_flow(&r), "outgoing");
        assert_eq!(tx_signed_amount(&r), "-0.50000000 BTC");
        assert_eq!(tx_label(&r), "2025-01-02 · W · outbound · abc");
        assert_eq!(short_id("0123456789abcdefghij"), "0123456789...efghij");
        assert_eq!(format_datetime("2025-01-02T03:04:05Z"), "2025-01-02 03:04");
    }
}
