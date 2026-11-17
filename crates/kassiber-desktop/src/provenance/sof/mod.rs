//! Source of funds (`/source-of-funds`, dev-gated; `/proof-funds` redirects here):
//! routes/source-funds/** — the four-step case journey (choose funds, trace & resolve, review
//! the report, export).
//!
//! - `model`: the pure helpers (model.ts, caseScope.ts, journey.ts, findingCopy.ts,
//!   provenanceGraph.ts and the evidence graph's layout), unit-tested;
//! - `kit`: the shadcn pieces and Tailwind tones the stages use;
//! - `graph`: the interactive `EvidenceGraph` canvas over the report's `simplified_flow`;
//! - `target` / `trace` / `disclose`: the stages and their panels;
//! - `page`: the journey frame (steps, case strip, alerts, footer).
//!
//! Every read is local and scoped with the book's `expected_scope` from `ui.workspace.health`,
//! as the web's `DaemonScopeContext` does. Writes run only from a press, except the web's
//! auto-assembly: opening a target with no reviewed history runs `ui.source_funds.assemble`
//! once per (profile, target, input version), which derives structural chain edges from local
//! observations (no network). Explorer links on the disclosure list open the user's configured
//! explorer only from a press (egress the user asks for). Exports write into the book's
//! `exports/reports` folder, then ask where to keep a copy with the native Save dialog.

pub mod disclose;
pub mod graph;
pub mod kit;
pub mod model;
pub mod page;
pub mod scenes;
pub mod target;
pub mod trace;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rune::{FileDialogOptions, KeyEvent, TextEdit, Tx};
use serde_json::{Map, Value, json};

use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use model::{Filters, LinkForm, NO_ATTACHMENT};

pub const ROUTE: &str = "/source-of-funds";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Target,
    Trace,
    Disclose,
    Export,
}

impl Stage {
    pub const ALL: [Stage; 4] = [Stage::Target, Stage::Trace, Stage::Disclose, Stage::Export];
    pub fn id(self) -> &'static str {
        match self {
            Stage::Target => "target",
            Stage::Trace => "trace",
            Stage::Disclose => "disclose",
            Stage::Export => "export",
        }
    }
    /// `migrateStage`: older drafts carry the retired wizard step names.
    pub fn parse(v: &str) -> Stage {
        match v {
            "trace" | "review" => Stage::Trace,
            "disclose" => Stage::Disclose,
            "export" => Stage::Export,
            _ => Stage::Target,
        }
    }
    pub fn index(self) -> usize {
        Stage::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    Select(&'static str),
}

/// One local read the case keeps (a `useDaemon` query): replies cached by their argument key,
/// at most one request in flight; a key whose cache is stale refetches while still shown.
#[derive(Default)]
pub struct Query {
    pub cache: HashMap<String, Value>,
    pub errors: HashMap<String, String>,
    pub stale: HashSet<String>,
    pub inflight: Option<String>,
    pub current: String,
}

impl Query {
    pub fn data(&self) -> Option<&Value> {
        self.cache.get(&self.current)
    }
    pub fn fetching(&self) -> bool {
        self.inflight.as_deref() == Some(self.current.as_str())
    }
    /// No data yet for the current key and a request on its way (`isLoading`).
    pub fn loading(&self) -> bool {
        self.data().is_none()
            && !self.errors.contains_key(&self.current)
            && (self.fetching() || self.inflight.is_some())
    }
    pub fn error(&self) -> bool {
        self.errors.contains_key(&self.current)
    }
    pub fn success(&self) -> bool {
        self.data().is_some() && !self.error()
    }
    /// Marks everything stale (a mutation invalidated it); the current key refetches on the next
    /// poll while its data stays on screen.
    pub fn invalidate(&mut self) {
        let keep = self.current.clone();
        self.cache.retain(|k, _| *k == keep);
        self.errors.clear();
        self.stale = self.cache.keys().cloned().collect();
    }
}

/// The source / gap form (`sourceForm`).
pub struct SourceForm {
    pub source_type: String,
    pub link_type: String,
    pub label: TextEdit,
    pub asset: TextEdit,
    pub amount: TextEdit,
    pub description: TextEdit,
    pub attachment_id: String,
    pub to_transaction: String,
}

impl Default for SourceForm {
    fn default() -> Self {
        SourceForm {
            source_type: "fiat_purchase".into(),
            link_type: "manual_source".into(),
            label: TextEdit::new(""),
            asset: TextEdit::new("BTC"),
            amount: TextEdit::new(""),
            description: TextEdit::new(""),
            attachment_id: NO_ATTACHMENT.into(),
            to_transaction: String::new(),
        }
    }
}

/// The manual link form (`manualLinkForm`).
pub struct ManualForm {
    pub from_transaction: String,
    pub to_transaction: String,
    pub link_type: String,
    pub confidence: String,
    pub allocation: TextEdit,
    pub from_allocation: TextEdit,
    pub explanation: TextEdit,
    pub attachment_id: String,
}

impl Default for ManualForm {
    fn default() -> Self {
        ManualForm {
            from_transaction: String::new(),
            to_transaction: String::new(),
            link_type: "self_transfer".into(),
            confidence: "strong".into(),
            allocation: TextEdit::new(""),
            from_allocation: TextEdit::new(""),
            explanation: TextEdit::new(""),
            attachment_id: NO_ATTACHMENT.into(),
        }
    }
}

/// The link editor's text fields over `LinkForm`'s selects.
pub struct LinkEdit {
    pub link_type: String,
    pub confidence: String,
    pub allocation: TextEdit,
    pub from_allocation: TextEdit,
    pub explanation: TextEdit,
    pub attachment_id: String,
}

impl Default for LinkEdit {
    fn default() -> Self {
        LinkEdit::from(&LinkForm::default())
    }
}

impl LinkEdit {
    pub fn from(f: &LinkForm) -> LinkEdit {
        LinkEdit {
            link_type: f.link_type.clone(),
            confidence: f.confidence.clone(),
            allocation: TextEdit::new(&f.allocation_amount),
            from_allocation: TextEdit::new(&f.from_allocation_amount),
            explanation: TextEdit::new(&f.explanation),
            attachment_id: f.attachment_id.clone(),
        }
    }
    pub fn form(&self) -> LinkForm {
        LinkForm {
            link_type: self.link_type.clone(),
            confidence: self.confidence.clone(),
            allocation_amount: self.allocation.text.to_owned(),
            from_allocation_amount: self.from_allocation.text.to_owned(),
            explanation: self.explanation.text.to_owned(),
            attachment_id: self.attachment_id.clone(),
        }
    }
}

/// The fields the web persists per book (`sourceFundsDrafts[draftKey]`).
#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub target: String,
    pub target_amount: String,
    pub report_purpose: String,
    pub planned_destination: String,
    pub planned_note: String,
    pub reveal_mode: String,
    pub diagram_detail: String,
    pub selected_recipient: String,
    pub current_step: String,
}

/// A finished export: the case it saved and the file it wrote, for the recipe it was made from.
#[derive(Clone, Debug)]
pub struct Completed {
    pub recipe: String,
    pub case: Value,
    pub pdf: Option<String>,
    pub bundle: Option<String>,
}

/// A written export waiting for its Save As answer.
#[derive(Clone, Debug)]
pub struct Written {
    pub bundle: bool,
    pub path: String,
    pub filename: String,
}

/// The graph's camera and selection (`EvidenceGraph`).
#[derive(Clone, Debug, Default)]
pub struct GraphView {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub touched: bool,
    pub drag: Option<(f32, f32)>,
    pub selected: Option<(bool, String)>,
}

/// One mounted case (`SourceFundsCase`): reset whenever the route's target or the book changes.
pub struct Case {
    pub stage: Stage,
    pub purpose: String,
    pub target: TextEdit,
    pub amount: TextEdit,
    pub planned_destination: TextEdit,
    pub planned_note: TextEdit,
    pub reveal_mode: String,
    pub diagram_detail: String,
    pub amount_precision: String,
    pub mask_recipient: bool,
    pub omit_sections: Vec<String>,
    pub reveal_overrides: BTreeMap<String, String>,
    pub recipient: String,
    pub search: TextEdit,
    pub filters: Filters,
    pub show_filters: bool,
    pub show_advanced: bool,
    pub show_coverage: bool,
    pub open: HashSet<String>,
    pub selected_link: String,
    pub inspected: Option<Value>,
    pub link: LinkEdit,
    pub source: SourceForm,
    pub manual: ManualForm,
    pub assembled: Option<Value>,
    pub reviewed_fingerprint: Option<String>,
    pub completed: Option<Completed>,
    pub export_failure: Option<String>,
    /// The export running: (bundle?, recipe key it started from, saved case once saved).
    pub exporting: Option<(bool, String, Option<Value>)>,
    pub graph: HashMap<&'static str, GraphView>,
    /// Transaction pages of the current filter key: rows and the next cursor.
    pub pages: Vec<Value>,
    pub next_cursor: Option<String>,
    pub pages_key: String,
    pub pages_error: bool,
    pub pages_loading: bool,
    pub last_target: String,
}

impl Case {
    fn new(d: &Draft, initial: &str) -> Case {
        // A route opened for another transaction must not inherit the previous case amount/options.
        let fresh = Draft::default();
        let d = if !initial.is_empty() && initial != d.target {
            &fresh
        } else {
            d
        };
        let or = |v: &str, def: &str| {
            if v.is_empty() {
                def.to_owned()
            } else {
                v.to_owned()
            }
        };
        Case {
            stage: Stage::parse(&d.current_step),
            purpose: or(&d.report_purpose, "existing_transaction"),
            target: TextEdit::new(if initial.is_empty() {
                &d.target
            } else {
                initial
            }),
            amount: TextEdit::new(&d.target_amount),
            planned_destination: TextEdit::new(&d.planned_destination),
            planned_note: TextEdit::new(&d.planned_note),
            reveal_mode: or(&d.reveal_mode, "standard"),
            diagram_detail: or(&d.diagram_detail, "summary"),
            amount_precision: "btc".into(),
            mask_recipient: false,
            omit_sections: vec![],
            reveal_overrides: BTreeMap::new(),
            recipient: d.selected_recipient.clone(),
            search: TextEdit::new(""),
            filters: Filters::default(),
            show_filters: false,
            show_advanced: false,
            show_coverage: false,
            open: HashSet::new(),
            selected_link: String::new(),
            inspected: None,
            link: LinkEdit::default(),
            source: SourceForm::default(),
            manual: ManualForm::default(),
            assembled: None,
            reviewed_fingerprint: None,
            completed: None,
            export_failure: None,
            exporting: None,
            graph: HashMap::new(),
            pages: vec![],
            next_cursor: None,
            pages_key: String::new(),
            pages_error: false,
            pages_loading: false,
            last_target: String::new(),
        }
    }

    pub fn draft(&self) -> Draft {
        Draft {
            target: self.target.text.to_owned(),
            target_amount: self.amount.text.to_owned(),
            report_purpose: self.purpose.clone(),
            planned_destination: self.planned_destination.text.to_owned(),
            planned_note: self.planned_note.text.to_owned(),
            reveal_mode: self.reveal_mode.clone(),
            diagram_detail: self.diagram_detail.clone(),
            selected_recipient: self.recipient.clone(),
            current_step: self.stage.id().to_owned(),
        }
    }

    pub fn selected_target(&self) -> String {
        self.target.text.trim().to_owned()
    }

    pub fn is_open(&self, k: &str) -> bool {
        self.open.contains(k)
    }
}

#[derive(Default)]
pub struct Sof {
    /// `ui.workspace.health`: the canonical scope, None until it answers; Err when it failed.
    pub scope: Option<Result<(String, String), String>>,
    pub health_sent: bool,
    pub mounted: Option<String>,
    pub case: Option<Case>,
    pub drafts: HashMap<String, Draft>,
    pub auto_assembled: HashSet<String>,
    pub popup: Option<Popup>,
    pub trigger_w: HashMap<&'static str, f32>,
    pub preview: Query,
    pub resolved: Query,
    pub diagram: Query,
    pub coverage: Query,
    pub sources: Query,
    pub evidence: Query,
    pub recipients: Query,
    pub wallets: Query,
    /// Mutations in flight, by tag.
    pub busy: HashSet<&'static str>,
    pub pending_save: Option<Written>,
    /// For each source being created, in send order, the link it gets once created: the form
    /// as it was submitted, not as it reads when the source comes back.
    pub source_links: std::collections::VecDeque<Value>,
    /// For each link review in flight, in send order: the link, the evidence to attach once
    /// accepted, and its type (for the toast), as submitted. The page may be gone or showing
    /// another link when the answer comes.
    pub reviews: std::collections::VecDeque<(String, String, String)>,
    /// For each evidence attach in flight: the reviewed link's type (for the toast).
    pub attaching: std::collections::VecDeque<String>,
    /// For each case save of an export in flight: (bundle, recipe) as asked. The mounted case
    /// may be another one when the answer comes.
    pub saves: std::collections::VecDeque<(bool, String)>,
    /// For each export render in flight: (bundle, recipe, the saved case) as asked.
    pub exports: std::collections::VecDeque<(bool, String, Value)>,
}

// ---- Scope and requests -----------------------------------------------------------------------

fn scope_value(app: &App) -> Option<Value> {
    match &app.provenance.sof.scope {
        Some(Ok((w, p))) => Some(json!({"workspace_id": w, "profile_id": p})),
        _ => None,
    }
}

/// Every request inside the case carries the book's scope (`scopedDaemonArgs`).
fn scoped(app: &App, args: Value) -> Value {
    let mut a = match args {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    if let Some(s) = scope_value(app) {
        a.insert("expected_scope".into(), s);
    }
    Value::Object(a)
}

fn send(app: &mut App, kind: &str, args: Value, tag: &'static str) -> bool {
    let a = scoped(app, args);
    app.send(kind, a, tag).is_some()
}

fn database_identity(app: &App) -> String {
    app.launch
        .data_root
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "local".into())
}

fn draft_key(app: &App) -> Option<String> {
    match &app.provenance.sof.scope {
        Some(Ok((w, p))) => Some(model::draft_key(&database_identity(app), w, p)),
        _ => None,
    }
}

/// `?tx=` / `?transaction=` on the route.
pub fn initial_target(app: &App) -> String {
    let q = app.route.0.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut tx = None;
    let mut transaction = None;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(v);
        match k {
            "tx" => tx = Some(v),
            "transaction" => transaction = Some(v),
            _ => {}
        }
    }
    tx.or(transaction).unwrap_or_default()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 3 <= b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `previewArgs`.
pub fn preview_args(c: &Case) -> Value {
    let mut a = Map::new();
    a.insert("target_transaction".into(), json!(c.selected_target()));
    if !c.amount.text.is_empty() {
        a.insert("target_amount".into(), json!(c.amount.text));
    }
    a.insert("report_purpose".into(), json!(c.purpose));
    if c.purpose == "planned_exchange_sale" {
        if !c.planned_destination.text.is_empty() {
            a.insert(
                "planned_destination".into(),
                json!(c.planned_destination.text),
            );
        }
        if !c.planned_note.text.is_empty() {
            a.insert("planned_note".into(), json!(c.planned_note.text));
        }
    }
    a.insert("reveal_mode".into(), json!(c.reveal_mode));
    if !c.recipient.is_empty() {
        a.insert("recipient".into(), json!(c.recipient));
    }
    a.insert(
        "report_options".into(),
        json!({
            "diagram_detail": c.diagram_detail,
            "amount_precision": c.amount_precision,
            "mask_recipient": c.mask_recipient,
            "omit_sections": c.omit_sections,
            "reveal_overrides": c.reveal_overrides,
        }),
    );
    Value::Object(a)
}

/// Starts the read `q` for `key` when it has none (or a stale one) and nothing is in flight.
fn want(
    app: &mut App,
    pick: fn(&mut Sof) -> &mut Query,
    kind: &str,
    args: Value,
    tag: &'static str,
    enabled: bool,
) {
    let key = if enabled {
        args.to_string()
    } else {
        String::new()
    };
    let q = pick(&mut app.provenance.sof);
    q.current = key.clone();
    if !enabled || q.inflight.is_some() {
        return;
    }
    let need =
        (!q.cache.contains_key(&key) && !q.errors.contains_key(&key)) || q.stale.contains(&key);
    if !need {
        return;
    }
    q.inflight = Some(key.clone());
    q.stale.remove(&key);
    if !send(app, kind, args, tag) {
        pick(&mut app.provenance.sof).inflight = None;
    }
}

fn landed(q: &mut Query, r: &Reply) {
    let Some(key) = q.inflight.take() else { return };
    match &r.error {
        Some(e) => {
            q.errors.insert(key.clone(), e.clone());
            q.cache.remove(&key);
        }
        None => {
            q.errors.remove(&key);
            q.cache.insert(key, r.data.clone());
        }
    }
}

/// The case's review packet (`preview.data.data`).
pub fn context(app: &App) -> Option<&Value> {
    app.provenance.sof.preview.data()
}

/// `report`: the canonical report, with the printable diagrams while disclosure is shown.
pub fn report(app: &App) -> Option<&Value> {
    context(app).map(|c| &c["report"]).filter(|r| r.is_object())
}

pub fn diagrams(app: &App) -> Option<&Value> {
    let sof = &app.provenance.sof;
    if sof.diagram.fetching() {
        return None;
    }
    sof.diagram
        .data()
        .map(|d| &d["diagrams"])
        .filter(|d| d.is_object())
}

pub fn selected_tx_id(app: &App) -> String {
    context(app)
        .map(|c| model::s(&c["target"], "transaction_id").to_owned())
        .unwrap_or_default()
}

pub fn selected_tx(app: &App) -> Option<&Value> {
    app.provenance
        .sof
        .resolved
        .data()
        .map(|d| &d["transaction"])
        .filter(|t| t.is_object())
}

pub fn exportable(app: &App) -> bool {
    report(app).is_some_and(|r| r["explain_gates"]["exportable"].as_bool() == Some(true))
}

pub fn fingerprint(app: &App) -> Option<String> {
    context(app)
        .map(|c| model::s(c, "review_fingerprint").to_owned())
        .filter(|s| !s.is_empty())
}

fn scope_truncated(app: &App) -> bool {
    context(app).is_some_and(|c| c["scope_truncated"].as_bool() == Some(true))
}

/// `current`: a report for the current recipe that is neither refreshing, failed nor truncated.
pub fn current(app: &App) -> bool {
    let sof = &app.provenance.sof;
    report(app).is_some()
        && !sof.preview.fetching()
        && !sof.preview.error()
        && !sof.resolved.error()
        && !scope_truncated(app)
}

pub fn preview_ready(app: &App) -> bool {
    let d = &app.provenance.sof.diagram;
    model::can_approve(
        current(app),
        exportable(app),
        d.fetching() || d.data().is_none(),
        d.error(),
        fingerprint(app).as_deref(),
    )
}

pub fn can_export(app: &App) -> bool {
    let rf = app
        .provenance
        .sof
        .case
        .as_ref()
        .and_then(|c| c.reviewed_fingerprint.clone());
    model::is_reviewed(
        current(app),
        exportable(app),
        fingerprint(app).as_deref(),
        rf.as_deref(),
    )
}

/// The stage on screen: no target → target; export without a matching review → disclose.
pub fn stage(app: &App) -> Stage {
    let sof = &app.provenance.sof;
    let Some(c) = &sof.case else {
        return Stage::Target;
    };
    if c.selected_target().is_empty() {
        return Stage::Target;
    }
    if c.stage == Stage::Export {
        let still = model::is_reviewed(
            !sof.preview.error() && !sof.resolved.error() && !scope_truncated(app),
            exportable(app),
            fingerprint(app).as_deref(),
            c.reviewed_fingerprint.as_deref(),
        );
        if !still {
            return Stage::Disclose;
        }
    }
    c.stage
}

/// The recipe a completed export or a failure is bound to.
pub fn recipe_key(app: &App) -> String {
    let c = app.provenance.sof.case.as_ref();
    json!([c.map(preview_args).unwrap_or(Value::Null), fingerprint(app)]).to_string()
}

pub fn links(app: &App) -> Vec<Value> {
    context(app)
        .map(|c| model::arr(c, "links").to_vec())
        .unwrap_or_default()
}

pub fn sources(app: &App) -> Vec<Value> {
    let sof = &app.provenance.sof;
    let adv = sof.case.as_ref().is_some_and(|c| c.show_advanced);
    if adv && let Some(d) = sof.sources.data() {
        return model::arr(d, "sources").to_vec();
    }
    context(app)
        .map(|c| model::arr(c, "sources").to_vec())
        .unwrap_or_default()
}

pub fn evidence(app: &App) -> Vec<Value> {
    let sof = &app.provenance.sof;
    let adv = sof.case.as_ref().is_some_and(|c| c.show_advanced);
    if adv && let Some(d) = sof.evidence.data() {
        return model::arr(d, "attachments").to_vec();
    }
    context(app)
        .map(|c| model::arr(c, "evidence").to_vec())
        .unwrap_or_default()
}

pub fn bulk_eligible(app: &App) -> Vec<String> {
    context(app)
        .map(|c| {
            model::arr(&c["bulk_review"], "eligible_link_ids")
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The link the editor shows: the chosen one, else the first suggested, else the first.
pub fn selected_link(app: &App) -> Option<Value> {
    let ls = links(app);
    let id = app
        .provenance
        .sof
        .case
        .as_ref()
        .map(|c| c.selected_link.clone())
        .unwrap_or_default();
    ls.iter()
        .find(|l| model::s(l, "id") == id)
        .or_else(|| ls.iter().find(|l| model::s(l, "state") == "suggested"))
        .or(ls.first())
        .cloned()
}

/// Rows loaded so far (all pages) plus the resolved target (`txById`).
pub fn tx_by_id(app: &App) -> HashMap<String, Value> {
    let mut m = HashMap::new();
    let rows = app
        .provenance
        .sof
        .case
        .as_ref()
        .map(|c| c.pages.clone())
        .unwrap_or_default();
    for r in rows.iter().chain(selected_tx(app)) {
        for k in ["id", "transaction_id"] {
            let v = model::s(r, k);
            if !v.is_empty() {
                m.insert(v.to_owned(), r.clone());
            }
        }
    }
    m
}

pub fn tx_name(app: &App, id: &str) -> String {
    tx_by_id(app)
        .get(id)
        .map(model::tx_label)
        .unwrap_or_else(|| model::short_id(id))
}

pub fn source_name(app: &App, id: &str) -> String {
    sources(app)
        .iter()
        .find(|s| model::s(s, "id") == id)
        .map(|s| model::s(s, "label").to_owned())
        .unwrap_or_else(|| model::short_id(id))
}

// ---- Mount, poll and replies ------------------------------------------------------------------

fn mount(app: &mut App) {
    let key = draft_key(app).unwrap_or_default();
    let initial = initial_target(app);
    let d = app
        .provenance
        .sof
        .drafts
        .get(&key)
        .cloned()
        .unwrap_or_default();
    app.provenance.sof.case = Some(Case::new(&d, &initial));
    app.provenance.sof.popup = None;
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    if let Some(w) = app.provenance.sof.pending_save.take() {
        ask_save(w, tx);
        changed = true;
    }
    changed | sync(app)
}

/// Mounts the case for the route and starts its reads (no `Tx` needed: headless primes too).
fn sync(app: &mut App) -> bool {
    let mut changed = false;
    if app.route.path() != ROUTE {
        if app.provenance.sof.mounted.take().is_some() {
            app.provenance.sof.case = None;
            app.provenance.sof.popup = None;
            changed = true;
        }
        return changed;
    }
    // The scope wrapper: health first (unscoped), then a case per (book, route target).
    if !app.provenance.sof.health_sent {
        app.provenance.sof.health_sent = true;
        app.send("ui.workspace.health", Value::Null, "provenance.sof.health");
        return true;
    }
    let Some(Ok(_)) = &app.provenance.sof.scope else {
        return changed;
    };
    let mount_key = format!(
        "{}|{}",
        draft_key(app).unwrap_or_default(),
        initial_target(app)
    );
    if app.provenance.sof.mounted.as_deref() != Some(mount_key.as_str()) {
        app.provenance.sof.mounted = Some(mount_key);
        mount(app);
        changed = true;
    }
    changed | step(app)
}

/// Every frame on the route: persist the draft, start the reads the case needs, run the
/// auto-assembly effect.
fn step(app: &mut App) -> bool {
    let Some(c) = app.provenance.sof.case.as_ref() else {
        return false;
    };
    let before = app.pending.len();
    // Persist under the canonical key only.
    if let Some(k) = draft_key(app) {
        let d = c.draft();
        app.provenance.sof.drafts.insert(k, d);
    }
    let c = app.provenance.sof.case.as_ref().expect("case");
    let selected = c.selected_target();
    let args = preview_args(c);
    let show_adv = c.show_advanced;
    let show_cov = c.show_coverage;
    let stage_now = stage(app);
    let disclosure = matches!(stage_now, Stage::Disclose | Stage::Export);
    // Target switch: forms follow it; the assembled summary belongs to one target.
    if app
        .provenance
        .sof
        .case
        .as_ref()
        .is_some_and(|c| c.last_target != selected)
        && !selected.is_empty()
    {
        let c = app.provenance.sof.case.as_mut().expect("case");
        c.last_target = selected.clone();
        c.assembled = None;
        c.source.to_transaction = selected.clone();
        c.manual.to_transaction = selected.clone();
    }
    // Transaction pages: a new filter key restarts from the first page.
    let c = app.provenance.sof.case.as_ref().expect("case");
    let mut fl = c.filters.clone();
    fl.query = c.search.text.to_owned();
    let now_ms = (crate::format::now_secs() * 1000.0) as i64;
    let targs = model::target_query_args(&fl, now_ms, local_offset(now_ms / 1000));
    let tkey = targs.to_string();
    if c.pages_key != tkey && !c.pages_loading {
        let c = app.provenance.sof.case.as_mut().expect("case");
        c.pages_key = tkey;
        c.pages.clear();
        c.next_cursor = None;
        c.pages_error = false;
        c.pages_loading = true;
        if !send(app, "ui.transactions.list", targs, "provenance.sof.txs") {
            app.provenance
                .sof
                .case
                .as_mut()
                .expect("case")
                .pages_loading = false;
        }
    }
    want(
        app,
        |s| &mut s.wallets,
        "ui.wallets.list",
        json!({}),
        "provenance.sof.wallets",
        true,
    );
    want(
        app,
        |s| &mut s.recipients,
        "ui.source_funds.recipients.list",
        json!({"include_inactive": true}),
        "provenance.sof.recipients",
        true,
    );
    want(
        app,
        |s| &mut s.preview,
        "ui.source_funds.review_context",
        args.clone(),
        "provenance.sof.preview",
        !selected.is_empty(),
    );
    let target_id = selected_tx_id(app);
    want(
        app,
        |s| &mut s.resolved,
        "ui.transactions.resolve",
        json!({"query": target_id}),
        "provenance.sof.resolve",
        !target_id.is_empty(),
    );
    want(
        app,
        |s| &mut s.sources,
        "ui.source_funds.sources.list",
        json!({}),
        "provenance.sof.sources",
        show_adv,
    );
    want(
        app,
        |s| &mut s.evidence,
        "ui.source_funds.evidence.list",
        json!({}),
        "provenance.sof.evidence",
        show_adv,
    );
    want(
        app,
        |s| &mut s.coverage,
        "ui.source_funds.coverage",
        json!({}),
        "provenance.sof.coverage",
        show_cov,
    );
    want(
        app,
        |s| &mut s.diagram,
        "ui.source_funds.preview",
        args,
        "provenance.sof.diagram",
        disclosure && !target_id.is_empty(),
    );
    reconcile_link(app);
    auto_assemble(app);
    app.pending.len() != before
}

/// Keeps the link editor on the selected link (`reconcileLinkForm` on every change).
fn reconcile_link(app: &mut App) {
    let latest = selected_link(app);
    let c = app.provenance.sof.case.as_mut().expect("case");
    let Some(latest) = latest else {
        c.inspected = None;
        return;
    };
    if let Some(i) = &c.inspected {
        let same = |k: &str| i[k] == latest[k];
        if same("id")
            && same("updated_at")
            && same("allocation_amount_msat")
            && same("from_allocation_amount_msat")
        {
            return;
        }
    }
    c.selected_link = model::s(&latest, "id").to_owned();
    let (form, inspected) =
        model::reconcile_link_form(&c.link.form(), c.inspected.as_ref(), &latest);
    if c.inspected.as_ref() != Some(&inspected) || c.link.form() != form {
        let attach = c.link.attachment_id.clone();
        c.link = LinkEdit::from(&form);
        if c.inspected
            .as_ref()
            .is_some_and(|i| i["id"] == inspected["id"])
        {
            c.link.attachment_id = attach;
        }
    }
    c.inspected = Some(inspected);
}

fn mutating(app: &App) -> bool {
    let b = &app.provenance.sof.busy;
    [
        "assemble",
        "suggest",
        "bulk",
        "review",
        "attach",
        "create_link",
        "create_source",
    ]
    .iter()
    .any(|k| b.contains(k))
}

fn has_drafts(c: &Case) -> bool {
    let d = model::Drafts {
        source_label: &c.source.label.text,
        source_amount: &c.source.amount.text,
        source_description: &c.source.description.text,
        source_attachment: &c.source.attachment_id,
        manual_from: &c.manual.from_transaction,
        manual_allocation: &c.manual.allocation.text,
        manual_from_allocation: &c.manual.from_allocation.text,
        manual_explanation: &c.manual.explanation.text,
        manual_attachment: &c.manual.attachment_id,
    };
    model::has_unsaved_drafts(&c.link.form(), c.inspected.as_ref(), &d)
}

/// Chain evidence assembles itself: once per (profile, target, input version), only while the
/// case has no reviewed history, never while a read/write is in flight or a draft is open.
fn auto_assemble(app: &mut App) {
    let Some(ctx) = context(app) else { return };
    let target = model::s(&ctx["target"], "transaction_id").to_owned();
    let key = model::auto_assemble_key(
        model::s(ctx, "profile_id"),
        &target,
        ctx["input_version"].as_i64(),
    );
    let edges = model::arr(&ctx["report"]["graph"], "edges").len();
    let c = app.provenance.sof.case.as_ref().expect("case");
    let busy = app.provenance.sof.preview.fetching() || mutating(app) || has_drafts(c);
    if !model::should_auto_assemble(
        &target,
        edges,
        busy,
        &key,
        &app.provenance.sof.auto_assembled,
    ) {
        return;
    }
    app.provenance.sof.auto_assembled.insert(key);
    run_assembly(app, false);
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let Some(sub) = tag.strip_prefix("provenance.sof.") else {
        return;
    };
    let sof = &mut app.provenance.sof;
    match sub {
        "health" => {
            sof.scope = Some(match &r.error {
                Some(e) => Err(e.clone()),
                None => {
                    let w = r.data["workspace"]["id"].as_str().unwrap_or("").to_owned();
                    let p = r.data["profile"]["id"].as_str().unwrap_or("").to_owned();
                    if w.is_empty() || p.is_empty() {
                        Err(String::new())
                    } else {
                        Ok((w, p))
                    }
                }
            });
        }
        "txs" => {
            if let Some(c) = sof.case.as_mut() {
                c.pages_loading = false;
                match &r.error {
                    Some(_) => c.pages_error = true,
                    None => {
                        c.pages.extend(model::transaction_rows(&r.data));
                        c.next_cursor = r.data["nextCursor"]
                            .as_str()
                            .map(str::to_owned)
                            .filter(|s| !s.is_empty());
                    }
                }
            }
        }
        "wallets" => landed(&mut sof.wallets, r),
        "recipients" => landed(&mut sof.recipients, r),
        "preview" => landed(&mut sof.preview, r),
        "resolve" => landed(&mut sof.resolved, r),
        "sources" => landed(&mut sof.sources, r),
        "evidence" => landed(&mut sof.evidence, r),
        "coverage" => landed(&mut sof.coverage, r),
        "diagram" => landed(&mut sof.diagram, r),
        "assemble" | "assemble.quiet" => {
            sof.busy.remove("assemble");
            on_assembled(app, r, sub == "assemble");
        }
        "review.reviewed" | "review.rejected" => {
            sof.busy.remove("review");
            on_reviewed(app, r, sub == "review.reviewed");
        }
        "attach" => {
            sof.busy.remove("attach");
            let ty = sof.attaching.pop_front().unwrap_or_default();
            invalidate(app);
            reviewed_toast(app, true, &ty);
        }
        "create_link" => {
            sof.busy.remove("create_link");
            invalidate(app);
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Error,
                    &t("sourceFunds:header.title"),
                    Some(e.clone()),
                    Some("sof"),
                ),
                None => {
                    if let Some(c) = app.provenance.sof.case.as_mut() {
                        c.manual.allocation = TextEdit::new("");
                        c.manual.from_allocation = TextEdit::new("");
                        c.manual.explanation = TextEdit::new("");
                        c.manual.attachment_id = NO_ATTACHMENT.into();
                    }
                    app.toast(
                        ToastTone::Success,
                        &t("sourceFunds:toast.manualLinkAdded"),
                        Some(t("sourceFunds:toast.manualLinkBody")),
                        Some("sof"),
                    );
                }
            }
        }
        "create_source" => on_source_created(app, r),
        "source_link" => {
            sof.busy.remove("create_link");
            invalidate(app);
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Error,
                    &t("sourceFunds:header.title"),
                    Some(e.clone()),
                    Some("sof"),
                ),
                None => {
                    let gap = app
                        .provenance
                        .sof
                        .case
                        .as_ref()
                        .is_some_and(|c| c.source.source_type == "missing_history");
                    if let Some(c) = app.provenance.sof.case.as_mut() {
                        c.source.label = TextEdit::new("");
                        c.source.amount = TextEdit::new("");
                        c.source.description = TextEdit::new("");
                        c.source.attachment_id = NO_ATTACHMENT.into();
                    }
                    app.toast(
                        ToastTone::Success,
                        &t(if gap {
                            "sourceFunds:toast.gapMarked"
                        } else {
                            "sourceFunds:toast.sourceLinked"
                        }),
                        Some(t("sourceFunds:toast.sourcePathBody")),
                        Some("sof"),
                    );
                }
            }
        }
        "save" => on_saved(app, r),
        "export" => on_exported(app, r),
        _ => {}
    }
}

/// A source-funds write refreshes every case read (`refreshDaemonQueries`).
fn invalidate(app: &mut App) {
    let s = &mut app.provenance.sof;
    for q in [
        &mut s.preview,
        &mut s.resolved,
        &mut s.diagram,
        &mut s.coverage,
        &mut s.sources,
        &mut s.evidence,
        &mut s.recipients,
        &mut s.wallets,
    ] {
        q.invalidate();
    }
}

// ---- Actions ----------------------------------------------------------------------------------

/// `runAssembly`: local assembly of the reviewed flow graph behind the target.
pub fn run_assembly(app: &mut App, notify: bool) {
    let Some(c) = app.provenance.sof.case.as_ref() else {
        return;
    };
    let target = c.selected_target();
    if target.is_empty() || app.provenance.sof.busy.contains("assemble") {
        return;
    }
    if let Some(c) = app.provenance.sof.case.as_mut() {
        c.assembled = None;
    }
    if send(
        app,
        "ui.source_funds.assemble",
        json!({"target_transaction": target}),
        if notify {
            "provenance.sof.assemble"
        } else {
            "provenance.sof.assemble.quiet"
        },
    ) {
        app.provenance.sof.busy.insert("assemble");
    }
}

fn on_assembled(app: &mut App, r: &Reply, notify: bool) {
    if r.error.is_some() {
        app.toast(
            ToastTone::Warning,
            &t("sourceFunds:toast.assemblyFailed"),
            Some(t("sourceFunds:toast.assemblyFailedBody")),
            Some("sof"),
        );
        return;
    }
    invalidate(app);
    let d = r.data.clone();
    let reviewed = d["auto_reviewed"].as_u64().unwrap_or(0) as usize;
    let manual = d["awaiting_manual_review"].as_u64().unwrap_or(0) as usize;
    let missing = model::arr(&d, "chain_attestation_missing").len();
    if let Some(c) = app.provenance.sof.case.as_mut() {
        c.assembled = Some(d.clone());
    }
    if missing > 0 && reviewed == 0 {
        // Not a gap in history: the observer never recorded which inputs were its own. The
        // notification center keeps it until an assembly connects them.
        let at = app.clock;
        app.shell.note(
            &t("sourceFunds:toast.rescanNeeded"),
            Some(tc("sourceFunds:toast.rescanNeededBody", missing, &[])),
            ToastTone::Warning,
            at,
        );
        if notify {
            app.toast(
                ToastTone::Warning,
                &t("sourceFunds:toast.rescanNeeded"),
                Some(tc("sourceFunds:toast.rescanNeededBody", missing, &[])),
                Some("sof"),
            );
        }
        return;
    }
    if notify || reviewed > 0 {
        let title = t(if reviewed > 0 {
            "sourceFunds:case.historyAssembled"
        } else {
            "sourceFunds:actionsBar.assembleResultEmpty"
        });
        let body = if reviewed > 0 {
            tc("sourceFunds:toast.assembledBody", reviewed, &[])
        } else {
            tr(
                "sourceFunds:toast.deterministicBody",
                &[
                    ("count", &reviewed.to_string()),
                    ("reviewed", &reviewed.to_string()),
                    ("skipped", &manual.to_string()),
                ],
            )
        };
        app.toast(
            if reviewed > 0 {
                ToastTone::Success
            } else {
                ToastTone::Info
            },
            &title,
            Some(body),
            Some("sof"),
        );
    }
}

/// `reviewSelectedLink`: accept or reject the inspected link with only the deliberate edits.
pub fn review_link(app: &mut App, accept: bool) {
    let Some(sel) = selected_link(app) else {
        return;
    };
    if app.provenance.sof.busy.contains("review") {
        return;
    }
    let Some(c) = app.provenance.sof.case.as_ref() else {
        return;
    };
    let base = c.inspected.clone().unwrap_or(sel);
    let payload = model::link_review_payload(
        &base,
        &c.link.form(),
        if accept { "reviewed" } else { "rejected" },
    );
    let asked = (
        c.selected_link.clone(),
        c.link.attachment_id.clone(),
        c.link.link_type.clone(),
    );
    if send(
        app,
        "ui.source_funds.links.review",
        payload,
        if accept {
            "provenance.sof.review.reviewed"
        } else {
            "provenance.sof.review.rejected"
        },
    ) {
        app.provenance.sof.busy.insert("review");
        app.provenance.sof.reviews.push_back(asked);
    }
}

fn on_reviewed(app: &mut App, r: &Reply, accepted: bool) {
    // What was submitted, whatever the page shows now (or if it was left).
    let (link_id, attach, link_type) = app.provenance.sof.reviews.pop_front().unwrap_or_default();
    if r.error.is_some() {
        // A stale inspection is not retryable as-is: reload it.
        let stale = r.error_detail["code"].as_str() == Some("source_funds_link_stale");
        if stale {
            invalidate(app);
        }
        app.toast(
            ToastTone::Warning,
            &t("sourceFunds:toast.linkReviewFailed"),
            Some(t(if stale {
                "sourceFunds:toast.linkReviewStaleBody"
            } else {
                "sourceFunds:toast.linkReviewFailedBody"
            })),
            Some("sof"),
        );
        return;
    }
    invalidate(app);
    if accepted && !attach.is_empty() && attach != NO_ATTACHMENT && !link_id.is_empty() {
        if send(
            app,
            "ui.source_funds.links.attach",
            json!({"link": link_id, "attachment_id": attach}),
            "provenance.sof.attach",
        ) {
            app.provenance.sof.busy.insert("attach");
            app.provenance.sof.attaching.push_back(link_type);
        }
        return;
    }
    reviewed_toast(app, accepted, &link_type);
}

fn reviewed_toast(app: &mut App, accepted: bool, ty: &str) {
    let label = link_type_label(ty);
    app.toast(
        if accepted {
            ToastTone::Success
        } else {
            ToastTone::Info
        },
        &t(if accepted {
            "sourceFunds:toast.linkAccepted"
        } else {
            "sourceFunds:toast.linkRejected"
        }),
        Some(tr(
            if accepted {
                "sourceFunds:toast.linkReviewedBody"
            } else {
                "sourceFunds:toast.linkRejectedBody"
            },
            &[("type", &label)],
        )),
        Some("sof"),
    );
}

pub fn link_type_label(ty: &str) -> String {
    let k = format!("sourceFunds:linkType.{ty}");
    if crate::i18n::has(&k) {
        t(&k)
    } else {
        model::pretty(ty)
    }
}

/// `createManualLink`: a reviewed, explicit link between two transactions.
pub fn create_manual_link(app: &mut App) {
    let Some(c) = app.provenance.sof.case.as_ref() else {
        return;
    };
    let m = &c.manual;
    let mut a = json!({
        "from_transaction": m.from_transaction,
        "to_transaction": if m.to_transaction.is_empty() { c.selected_target() } else { m.to_transaction.clone() },
        "link_type": m.link_type,
        "state": "reviewed",
        "confidence": m.confidence,
        "method": "manual",
        "allocation_amount": m.allocation.text,
        "allocation_policy": "explicit",
        "explanation": m.explanation.text,
    });
    if !m.from_allocation.text.is_empty() {
        a["from_allocation_amount"] = json!(m.from_allocation.text);
    }
    if m.attachment_id != NO_ATTACHMENT {
        a["attachment_id"] = json!(m.attachment_id);
    }
    if send(
        app,
        "ui.source_funds.links.create",
        a,
        "provenance.sof.create_link",
    ) {
        app.provenance.sof.busy.insert("create_link");
    }
}

/// `createSourceLink`, step 1: the reviewed root source (or attested gap).
pub fn create_source_link(app: &mut App) {
    let Some(c) = app.provenance.sof.case.as_ref() else {
        return;
    };
    let s = &c.source;
    let mut a = json!({
        "source_type": s.source_type,
        "label": s.label.text,
        "asset": s.asset.text,
        "amount": s.amount.text,
        "description": s.description.text,
    });
    let mut link = json!({
        "to_transaction": if s.to_transaction.is_empty() { c.selected_target() } else { s.to_transaction.clone() },
        "link_type": s.link_type,
        "state": "reviewed",
        "confidence": if s.source_type == "missing_history" { "unknown" } else { "strong" },
        "method": "manual",
        "allocation_amount": s.amount.text,
        "allocation_policy": "explicit",
        "explanation": s.description.text,
    });
    if s.attachment_id != NO_ATTACHMENT {
        a["attachment_id"] = json!(s.attachment_id);
        link["attachment_id"] = json!(s.attachment_id);
    }
    if send(
        app,
        "ui.source_funds.sources.create",
        a,
        "provenance.sof.create_source",
    ) {
        app.provenance.sof.source_links.push_back(link);
        app.provenance.sof.busy.insert("create_source");
    }
}

/// Step 2: link the new source to the transaction it funds.
fn on_source_created(app: &mut App, r: &Reply) {
    app.provenance.sof.busy.remove("create_source");
    let link = app.provenance.sof.source_links.pop_front();
    if let Some(e) = &r.error {
        app.toast(
            ToastTone::Error,
            &t("sourceFunds:header.title"),
            Some(e.clone()),
            Some("sof"),
        );
        return;
    }
    let (Some(id), Some(mut a)) = (r.data["id"].as_str().map(str::to_owned), link) else {
        return;
    };
    a["from_source"] = json!(id);
    if send(
        app,
        "ui.source_funds.links.create",
        a,
        "provenance.sof.source_link",
    ) {
        app.provenance.sof.busy.insert("create_link");
    }
}

/// `prefillOriginForm`: document where funds came from (never defaults to attesting a gap).
pub fn prefill_origin(app: &mut App, gap: Option<&Value>) {
    let ids = tx_by_id(app);
    let Some(c) = app.provenance.sof.case.as_mut() else {
        return;
    };
    let amount_default = c.amount.text.to_owned();
    let s = &mut c.source;
    if s.source_type == "missing_history" {
        s.source_type = "fiat_purchase".into();
    }
    s.link_type = "manual_source".into();
    if let Some(g) = gap {
        let asset = model::s(g, "asset");
        if !asset.is_empty() {
            s.asset = TextEdit::new(asset);
        }
        let reff = model::s(g, "ref");
        if !reff.is_empty() && ids.contains_key(reff) {
            s.to_transaction = reff.to_owned();
        }
    }
    let amt = gap
        .map(|g| model::amount_input(&g["amount_msat"]))
        .unwrap_or_default();
    let next = if !amt.is_empty() {
        amt
    } else if !s.amount.text.is_empty() {
        s.amount.text.to_owned()
    } else {
        amount_default
    };
    s.amount = TextEdit::new(&next);
}

/// `prefillGapForm`: attest that the history is genuinely missing.
pub fn prefill_gap(app: &mut App, gap: Option<&Value>) {
    let ids = tx_by_id(app);
    let Some(c) = app.provenance.sof.case.as_mut() else {
        return;
    };
    let amount_default = c.amount.text.to_owned();
    let s = &mut c.source;
    s.source_type = "missing_history".into();
    s.link_type = "missing_history".into();
    if s.label.text.is_empty() {
        s.label = TextEdit::new(t("sourceFunds:gapDefaults.label"));
    }
    if let Some(g) = gap {
        let asset = model::s(g, "asset");
        if !asset.is_empty() {
            s.asset = TextEdit::new(asset);
        }
        let reff = model::s(g, "ref");
        if !reff.is_empty() && ids.contains_key(reff) {
            s.to_transaction = reff.to_owned();
        }
    }
    let amt = gap
        .map(|g| model::amount_input(&g["amount_msat"]))
        .unwrap_or_default();
    let next = if !amt.is_empty() {
        amt
    } else if !s.amount.text.is_empty() {
        s.amount.text.to_owned()
    } else {
        amount_default
    };
    s.amount = TextEdit::new(&next);
    if s.description.text.is_empty() {
        s.description = TextEdit::new(t("sourceFunds:gapDefaults.description"));
    }
}

/// `GateRow`'s action dispatch.
pub fn gap_action(app: &mut App, action: &str, finding: &Value) {
    let reff = model::s(finding, "ref").to_owned();
    let set_adv = |a: &mut App| {
        if let Some(c) = a.provenance.sof.case.as_mut() {
            c.show_advanced = true;
        }
    };
    match action {
        "open_source_creator" => {
            prefill_origin(app, Some(finding));
            set_adv(app);
        }
        "attest_missing_history" => {
            prefill_gap(app, Some(finding));
            set_adv(app);
        }
        "open_link_review" => {
            if !reff.is_empty() && links(app).iter().any(|l| model::s(l, "id") == reff) {
                if let Some(c) = app.provenance.sof.case.as_mut() {
                    c.selected_link = reff.clone();
                }
            } else if !reff.is_empty() && tx_by_id(app).contains_key(&reff) {
                open_tx(app, &reff);
            }
            set_adv(app);
        }
        "open_review_queue" | "open_source" => set_adv(app),
        "open_transaction" if !reff.is_empty() && tx_by_id(app).contains_key(&reff) => {
            open_tx(app, &reff)
        }
        _ => {}
    }
}

/// `openTxDetailById`: the shared transaction detail panel.
pub fn open_tx(app: &mut App, id: &str) {
    if !id.is_empty() {
        crate::txdetail::open(app, id, "details");
    }
}

/// `goToStage`.
pub fn go_stage(app: &mut App, s: Stage) {
    if let Some(c) = app.provenance.sof.case.as_mut() {
        c.stage = s;
    }
    app.provenance.sof.popup = None;
}

/// The footer's primary action (`forward`).
pub fn forward(app: &mut App) {
    match stage(app) {
        Stage::Target if !selected_tx_id(app).is_empty() => go_stage(app, Stage::Trace),
        Stage::Trace if current(app) => go_stage(app, Stage::Disclose),
        Stage::Disclose if preview_ready(app) => {
            // The explicit confirmation: bound to the fingerprint of the preview on screen.
            let fp = fingerprint(app);
            if let Some(c) = app.provenance.sof.case.as_mut() {
                c.reviewed_fingerprint = fp;
            }
            go_stage(app, Stage::Export);
        }
        _ => {}
    }
}

pub fn back(app: &mut App) {
    let i = stage(app).index();
    if i > 0 {
        go_stage(app, Stage::ALL[i - 1]);
    }
}

/// `investigate`: hands the exact recipe to the assistant (parked in its composer).
pub fn investigate(app: &mut App) {
    if !assistant_available(app) {
        return;
    }
    let Some(ctx) = context(app) else { return };
    let prompt = tr(
        "sourceFunds:case.assistantPrompt",
        &[
            ("target", model::s(&ctx["target"], "transaction_id")),
            ("recipe", &ctx["recipe"].to_string()),
        ],
    );
    crate::assistant::hand_off(app, &prompt);
}

pub fn assistant_available(app: &App) -> bool {
    app.prefs.ai_features
        && context(app).is_some()
        && !app.provenance.sof.preview.fetching()
        && !app.assistant.streaming
}

pub fn retry_preview(app: &mut App) {
    let s = &mut app.provenance.sof;
    let k = s.preview.current.clone();
    s.preview.errors.remove(&k);
    s.preview.stale.insert(k);
}

pub fn retry_diagram(app: &mut App) {
    let s = &mut app.provenance.sof;
    let k = s.diagram.current.clone();
    s.diagram.errors.remove(&k);
    s.diagram.stale.insert(k);
}

pub fn load_more(app: &mut App) {
    let Some(c) = app.provenance.sof.case.as_ref() else {
        return;
    };
    let Some(cursor) = c.next_cursor.clone() else {
        return;
    };
    if c.pages_loading {
        return;
    }
    let mut fl = c.filters.clone();
    fl.query = c.search.text.to_owned();
    let now_ms = (crate::format::now_secs() * 1000.0) as i64;
    let mut a = model::target_query_args(&fl, now_ms, local_offset(now_ms / 1000));
    a["cursor"] = json!(cursor);
    app.provenance
        .sof
        .case
        .as_mut()
        .expect("case")
        .pages_loading = true;
    if !send(app, "ui.transactions.list", a, "provenance.sof.txs") {
        app.provenance
            .sof
            .case
            .as_mut()
            .expect("case")
            .pages_loading = false;
    }
}

// ---- Export -----------------------------------------------------------------------------------

/// `handleExport`: save the reviewed case (bound to the inspected fingerprint), then render
/// the PDF or bundle from that frozen snapshot.
pub fn export(app: &mut App, bundle: bool) {
    if !exportable(app) || app.provenance.sof.preview.fetching() {
        return;
    }
    if app
        .provenance
        .sof
        .case
        .as_ref()
        .is_some_and(|c| c.exporting.is_some())
    {
        return;
    }
    let recipe = recipe_key(app);
    let Some(c) = app.provenance.sof.case.as_ref() else {
        return;
    };
    let mut args = preview_args(c);
    args["expected_review_fingerprint"] = json!(fingerprint(app));
    if let Some(c) = app.provenance.sof.case.as_mut() {
        c.export_failure = None;
        c.exporting = Some((bundle, recipe.clone(), None));
    }
    if send(
        app,
        "ui.source_funds.cases.save",
        args,
        "provenance.sof.save",
    ) {
        app.provenance.sof.saves.push_back((bundle, recipe));
    } else if let Some(c) = app.provenance.sof.case.as_mut() {
        c.exporting = None;
    }
}

/// Ends the mounted case's running export if it is the one for `recipe`.
fn end_export(app: &mut App, recipe: &str) -> bool {
    match app.provenance.sof.case.as_mut() {
        Some(c) if c.exporting.as_ref().is_some_and(|e| e.1 == recipe) => {
            c.exporting = None;
            true
        }
        _ => false,
    }
}

/// Whether the export started for `recipe` still belongs to the case on screen.
fn still_current(app: &App, recipe: &str) -> bool {
    app.route.path() == ROUTE && recipe_key(app) == recipe
}

fn on_saved(app: &mut App, r: &Reply) {
    // The export this save was for, whatever case is mounted now.
    let Some((bundle, recipe)) = app.provenance.sof.saves.pop_front() else {
        return;
    };
    if r.error.is_some() {
        if end_export(app, &recipe)
            && let Some(c) = app.provenance.sof.case.as_mut()
        {
            c.export_failure = Some(recipe);
        }
        invalidate(app);
        return;
    }
    let case = r.data["case"].clone();
    let id = case["id"].as_str().map(str::to_owned);
    if !still_current(app, &recipe) || id.is_none() {
        end_export(app, &recipe);
        return;
    }
    let kind = if bundle {
        "ui.source_funds.export_bundle"
    } else {
        "ui.source_funds.export_pdf"
    };
    if let Some(c) = app.provenance.sof.case.as_mut() {
        c.exporting = Some((bundle, recipe.clone(), Some(case.clone())));
    }
    if send(app, kind, json!({"case": id}), "provenance.sof.export") {
        app.provenance.sof.exports.push_back((bundle, recipe, case));
    } else {
        end_export(app, &recipe);
    }
}

fn on_exported(app: &mut App, r: &Reply) {
    // The case this render was for, as saved; not whichever case is mounted now.
    let Some((bundle, recipe, case)) = app.provenance.sof.exports.pop_front() else {
        return;
    };
    let mounted = end_export(app, &recipe);
    if let Some(e) = &r.error {
        if mounted && let Some(c) = app.provenance.sof.case.as_mut() {
            c.export_failure = Some(recipe);
        }
        app.toast(
            ToastTone::Error,
            &t("sourceFunds:case.exportError"),
            Some(e.clone()),
            Some("sof-export"),
        );
        return;
    }
    if !still_current(app, &recipe) {
        return;
    }
    let filename = r.data["filename"].as_str().unwrap_or("").to_owned();
    let path = r.data["file"].as_str().unwrap_or("").to_owned();
    if let Some(c) = app.provenance.sof.case.as_mut() {
        let mut done = Completed {
            recipe: recipe.clone(),
            case,
            pdf: None,
            bundle: None,
        };
        if bundle {
            done.bundle = Some(filename.clone());
        } else {
            done.pdf = Some(filename.clone());
        }
        c.completed = Some(done);
    }
    if !path.is_empty() {
        app.provenance.sof.pending_save = Some(Written {
            bundle,
            path,
            filename,
        });
    }
}

/// The completed export of the recipe on screen.
pub fn completed(app: &App) -> Option<&Completed> {
    let rk = recipe_key(app);
    app.provenance
        .sof
        .case
        .as_ref()
        .and_then(|c| c.completed.as_ref())
        .filter(|c| c.recipe == rk)
}

pub fn export_error(app: &App) -> bool {
    let rk = recipe_key(app);
    app.provenance
        .sof
        .case
        .as_ref()
        .and_then(|c| c.export_failure.as_ref())
        .is_some_and(|f| *f == rk)
}

/// The Save As for a written export: a native Save dialog named after the managed file.
fn ask_save(w: Written, tx: &mut Tx) {
    let source = PathBuf::from(&w.path);
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned();
    let options = FileDialogOptions {
        title: t("sourceFunds:header.title"),
        file_name: Some(name),
        ..Default::default()
    };
    let options = if w.bundle {
        options.filter("ZIP", &["zip"])
    } else {
        options.filter("PDF", &["pdf"])
    };
    tx.save_file(options, move |a: &mut App, result, _tx| {
        let answer = match result {
            Ok(Some(dest)) => save_copy(a, &source, &dest, w.bundle).map(Some),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        };
        finish(a, &w, answer);
    });
}

/// The export's outcome once the Save As answered (`Ok(None)`: cancelled, the managed file stays).
pub fn finish(app: &mut App, w: &Written, answer: Result<Option<String>, String>) {
    match answer {
        Err(e) => app.toast(
            ToastTone::Error,
            &t("chrome:notifications.feedback.reportSaveFailed"),
            Some(e),
            Some("sof-export"),
        ),
        Ok(saved) => {
            let file = saved
                .as_deref()
                .map(basename)
                .unwrap_or_else(|| w.filename.clone());
            let title = t("chrome:notifications.result.reportExported.title");
            let at = app.clock;
            app.shell
                .note(&title, Some(file.clone()), ToastTone::Info, at);
            // `toastopen`: Open / Show in Finder for the copy (or the managed file).
            let (path, managed) = match &saved {
                Some(dest) => (dest.as_str(), false),
                None => (w.path.as_str(), true),
            };
            crate::toastopen::saved_toast(
                app,
                ToastTone::Success,
                &title,
                Some(file),
                Some("sof-export"),
                path,
                managed,
            );
        }
    }
}

fn basename(p: &str) -> String {
    p.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(p)
        .to_owned()
}

fn state_root(app: &App) -> Option<PathBuf> {
    let data = app.data_root()?;
    Some(
        if data.file_name().and_then(|n| n.to_str()) == Some("data") {
            data.parent().map(Path::to_path_buf).unwrap_or(data)
        } else {
            data
        },
    )
}

/// `validated_report_export_target` + `save_exported_file_as`: the source must sit directly in
/// the book's `exports/reports` folder; the copy lands outside it with a `.pdf` / `.zip` name.
fn save_copy(app: &App, source: &Path, dest: &Path, bundle: bool) -> Result<String, String> {
    let root = state_root(app).ok_or("Managed report export folder could not be found.")?;
    let managed = std::fs::canonicalize(root.join("exports").join("reports"))
        .map_err(|e| format!("Managed report export folder could not be found: {e}"))?;
    let source = std::fs::canonicalize(source)
        .map_err(|e| format!("Report export file could not be found: {e}"))?;
    if source.parent() != Some(managed.as_path()) || !source.is_file() {
        return Err("Only exports from the active Kassiber project can be opened or saved.".into());
    }
    let parent = dest
        .parent()
        .ok_or("Report export destination must include a parent folder.")?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|e| format!("Could not inspect report export destination folder: {e}"))?;
    if parent.starts_with(&managed) {
        return Err("Choose a destination outside Kassiber's managed exports folder.".into());
    }
    let ext = dest
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if ext.as_deref() != Some(if bundle { "zip" } else { "pdf" }) {
        return Err(format!(
            "Report export destination must use .{}.",
            if bundle { "zip" } else { "pdf" }
        ));
    }
    std::fs::copy(&source, dest).map_err(|e| format!("Could not save report export: {e}"))?;
    Ok(dest.to_string_lossy().into_owned())
}

// ---- Keys -------------------------------------------------------------------------------------

pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if app.route.path() != ROUTE {
        return false;
    }
    if matches!(ev.key, rune::Key::Escape) && app.provenance.sof.popup.is_some() {
        app.provenance.sof.popup = None;
        return true;
    }
    false
}

/// The local UTC offset (seconds) at `secs`, as the webview's `Date` applies it.
pub fn local_offset(secs: i64) -> i64 {
    crate::localtime::offset_secs(secs)
}

// ---- Headless ---------------------------------------------------------------------------------

/// Pumps the daemon until the case's reads have landed (up to 20 s).
pub fn prime(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut quiet = 0;
    loop {
        let _ = sync(app);
        app.pump();
        let busy = app
            .pending
            .values()
            .any(|(tag, _, _)| tag.starts_with("provenance.sof."));
        quiet = if busy { 0 } else { quiet + 1 };
        if quiet > 3 || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod source_link_tests {
    use super::*;

    /// "Applies to" changed while the source is being created: its link still goes to the
    /// transaction that was submitted.
    #[test]
    fn a_source_link_uses_the_submitted_form() {
        let mut app = crate::app::tests::quiet_app();
        let mut case = Case::new(&Draft::default(), "tx-a");
        case.source.to_transaction = "tx-a".into();
        case.source.amount = TextEdit::new("0.5");
        app.provenance.sof.case = Some(case);
        create_source_link(&mut app);
        let c = app.provenance.sof.case.as_mut().unwrap();
        c.source.to_transaction = "tx-b".into();
        c.source.amount = TextEdit::new("0.9");
        let link = app
            .provenance
            .sof
            .source_links
            .front()
            .expect("snapshot")
            .clone();
        assert_eq!(link["to_transaction"], "tx-a");
        assert_eq!(link["allocation_amount"], "0.5");
        let reply = Reply {
            kind: "ui.source_funds.sources.create".into(),
            data: json!({ "id": "src-1" }),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_source_created(&mut app, &reply);
        assert!(app.provenance.sof.source_links.is_empty());
        assert!(app.provenance.sof.busy.contains("create_link"));
    }

    /// Case A's export saving, then case B opened and its export started: A's save answer must
    /// not render A's case under B's export (nor end B's).
    #[test]
    fn an_earlier_cases_save_answer_does_not_export_under_another_case() {
        let mut app = crate::app::tests::quiet_app();
        app.provenance
            .sof
            .saves
            .push_back((false, "recipe-a".into()));
        let mut b = Case::new(&Draft::default(), "tx-b");
        b.exporting = Some((false, "recipe-b".into(), None));
        app.provenance.sof.case = Some(b);
        let saved_a = Reply {
            kind: "ui.source_funds.cases.save".into(),
            data: json!({"case": {"id": "case-a"}}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_saved(&mut app, &saved_a);
        assert!(app.provenance.sof.saves.is_empty());
        assert!(
            app.provenance.sof.exports.is_empty(),
            "A's case is not rendered"
        );
        let running = app.provenance.sof.case.as_ref().unwrap().exporting.clone();
        assert_eq!(running.map(|e| e.1), Some("recipe-b".to_owned()));
    }

    /// Accepting a link, then leaving the page (or picking other evidence) before the answer:
    /// the answer attaches the evidence that was submitted, to the link that was reviewed, and
    /// does not need the page.
    #[test]
    fn a_review_answer_after_leaving_uses_what_was_submitted() {
        let mut app = crate::app::tests::quiet_app();
        let mut case = Case::new(&Draft::default(), "tx-a");
        case.selected_link = "link-1".into();
        case.link.attachment_id = "att-1".into();
        case.link.link_type = "purchase".into();
        app.provenance.sof.case = Some(case);
        app.provenance
            .sof
            .reviews
            .push_back(("link-1".into(), "att-1".into(), "purchase".into()));
        // The page is left; then the answer comes.
        app.provenance.sof.case = None;
        let ok = Reply {
            kind: "ui.source_funds.links.review".into(),
            data: json!({}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        let before = app.pending.len();
        on_reviewed(&mut app, &ok, true);
        assert!(app.provenance.sof.reviews.is_empty());
        assert_eq!(app.pending.len(), before + 1, "the attach is sent");
        assert!(
            app.pending
                .values()
                .any(|(tag, _, _)| *tag == "provenance.sof.attach")
        );
        assert_eq!(app.provenance.sof.attaching.front().unwrap(), "purchase");
    }
}
