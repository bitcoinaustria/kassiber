//! Chain analysis (`/chain-analysis`, `routes/ChainAnalysis.tsx` and `routes/chain-analysis/**`):
//! the local investigation workbench over `ui.chain_analysis.*`.
//!
//! Every read on mount is local (the bounded, cache-only query; watches; the book's network
//! binding; datasets). Acquisition, recurring sources and anything else that reaches a backend
//! run only from their own explicit button, after a local preview, exactly as the web gates them.

pub mod datasets;
pub mod graph;
pub mod inspector;
pub mod kit;
pub mod model;
pub mod page;
pub mod panels;
pub mod psbt;
pub mod results;
pub mod scenes;

use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use rune::{Key, KeyEvent, TextEdit, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::t;

use model::{AResult, Query, Sel, Workspace};

/// The workbench's state (`app.provenance.chain`). Re-created whenever the route (its search)
/// or the open book changes, as the web keys `ChainAnalysisWorkbench` by both.
#[derive(Default)]
pub struct State {
    /// The route + book scope this state was mounted for.
    pub mounted_for: Option<String>,
    pub query: Query,
    pub result: Option<Rc<AResult>>,
    pub historical: bool,
    pub sel: Option<Sel>,
    pub highlighted: Vec<String>,
    pub table: bool,
    pub tab: &'static str,
    pub show_acquire: bool,
    pub show_saved: bool,
    pub workspace: Workspace,
    pub initial_network: Option<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// `toastopen`: Open / Show in Finder for the last export, shown while its "saved" notice is.
    pub export_actions: Vec<crate::toastopen::ToastAction>,
    /// Versions of the queries in flight (oldest first) and the current one: a late answer to
    /// an older query is dropped.
    pub inflight: VecDeque<u64>,
    pub version: u64,
    pub refresh_failed: bool,
    /// The open `<select>` listbox or menu.
    pub popup: Option<String>,
    /// Open `<details>` disclosures.
    pub open: HashSet<String>,
    pub edits: HashMap<String, TextEdit>,
    pub graph: graph::GraphState,
    pub copied: Option<f64>,
    pub asking: bool,
    pub export: Option<&'static str>,
    pub binding: Bind,
    pub inventory: Option<Value>,
    pub backends: Bind,
    pub inbox: panels::Inbox,
    pub watch: HashMap<String, panels::WatchUi>,
    /// The watch control whose request is in flight.
    pub watch_busy: Option<String>,
    pub saved: panels::Saved,
    pub acquire: panels::Acquire,
    pub sources: panels::Sources,
    pub entropy: results::Entropy,
    pub labels: results::Labels,
    pub psbt: psbt::Psbt,
    pub datasets: datasets::Datasets,
    /// A key to scroll into view on the next poll (scenes).
    pub reveal: Option<String>,
}

/// A shared local read.
#[derive(Default, Clone, Debug)]
pub enum Bind {
    #[default]
    Idle,
    Loading,
    Ready(Value),
    Failed(String),
}

impl Bind {
    pub fn ready(&self) -> Option<&Value> {
        match self {
            Bind::Ready(v) => Some(v),
            _ => None,
        }
    }
}

pub fn st(app: &mut App) -> &mut State {
    &mut app.provenance.chain
}

fn mount_key(app: &App) -> String {
    format!("{}|{}", app.route.0, app.expected_scope())
}

/// Mounts the workbench for the current route: the link's query, workspace and tab, the
/// mount reads, and the bounded local graph read when the link asks for one.
fn mount(app: &mut App) {
    let search = model::parse_search(&model::raw_search(&app.route.0));
    let query = model::search_query(&search);
    // Queries still out stay listed and the version keeps counting, so an earlier mount's
    // reply is dropped instead of taking this mount's place.
    let old = &mut app.provenance.chain;
    let mut s = State {
        inflight: std::mem::take(&mut old.inflight),
        version: old.version,
        mounted_for: Some(mount_key(app)),
        tab: search.tab.unwrap_or("findings"),
        workspace: search.workspace.unwrap_or_default(),
        initial_network: search
            .fields
            .get("network")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..State::default()
    };
    s.query = query.clone();
    // A deep link in path mode without a target opens Advanced so the field shows.
    if query.mode == "path"
        && query
            .target
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        s.open.insert("filters".into());
    }
    app.provenance.chain = s;
    sync_edits(app);
    app.send(
        "ui.chain_analysis.watches.list",
        Value::Null,
        "provenance.chain.watches.list",
    );
    app.send(
        "ui.chain_analysis.watches.inbox",
        json!({"limit": 20}),
        "provenance.chain.watches.inbox",
    );
    load_binding(app);
    datasets::load_list(app);
    let runnable = query.mode == "overview"
        || (query
            .subject
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            && (query.mode != "path"
                || query
                    .target
                    .as_deref()
                    .is_some_and(|t| !t.trim().is_empty())));
    if app.provenance.chain.workspace == Workspace::Graph && runnable {
        execute(app, query);
    }
}

/// The book's network binding (`ui.networks.binding`), read once per mount.
pub fn load_binding(app: &mut App) {
    if matches!(app.provenance.chain.binding, Bind::Idle)
        && app
            .send(
                "ui.networks.binding",
                Value::Null,
                "provenance.chain.binding",
            )
            .is_some()
    {
        app.provenance.chain.binding = Bind::Loading;
    }
}

/// `ui.backends.options`, once per mount (the acquisition panels).
pub fn load_backends(app: &mut App) {
    if matches!(app.provenance.chain.backends, Bind::Idle)
        && app
            .send(
                "ui.backends.options",
                Value::Null,
                "provenance.chain.backends",
            )
            .is_some()
    {
        app.provenance.chain.backends = Bind::Loading;
    }
}

/// The query fields' text from `query` (after a programmatic change).
pub fn sync_edits(app: &mut App) {
    let q = app.provenance.chain.query.clone();
    let start = q
        .start
        .as_deref()
        .map(|s| s.trim_end_matches('Z').to_owned())
        .unwrap_or_default();
    let end = q
        .end
        .as_deref()
        .map(|s| s.trim_end_matches('Z').to_owned())
        .unwrap_or_default();
    for (k, v) in [
        ("q.subject", q.subject.clone().unwrap_or_default()),
        ("q.target", q.target.clone().unwrap_or_default()),
        ("q.depth", q.depth.to_string()),
        ("q.node_limit", q.node_limit.to_string()),
        ("q.edge_limit", q.edge_limit.to_string()),
        ("q.min", q.min_amount_msat.clone().unwrap_or_default()),
        ("q.start", start),
        ("q.end", end),
    ] {
        kit::set_val(app, k, &v);
    }
}

/// The subject field changed: `trace` once a subject is typed from the overview, back to
/// `overview` when a trace's subject is cleared.
pub fn on_subject(app: &mut App) {
    let subject = kit::val(app, "q.subject");
    let q = &mut app.provenance.chain.query;
    q.mode = if q.mode == "overview" && !subject.trim().is_empty() {
        "trace".into()
    } else if q.mode == "trace" && subject.trim().is_empty() {
        "overview".into()
    } else {
        q.mode.clone()
    };
    q.subject = Some(subject);
}

/// The Advanced fields changed (`update`): numbers as `Number(value)`, empty text as unset.
pub fn on_filters(app: &mut App) {
    let v = |a: &App, k: &str| kit::val(a, k);
    let num = |s: String| {
        s.trim()
            .parse::<f64>()
            .map(|f| f as i64)
            .unwrap_or(i64::MIN)
    };
    let (target, depth, nodes, edges, min, start, end) = (
        v(app, "q.target"),
        num(v(app, "q.depth")),
        num(v(app, "q.node_limit")),
        num(v(app, "q.edge_limit")),
        v(app, "q.min"),
        v(app, "q.start"),
        v(app, "q.end"),
    );
    let q = &mut app.provenance.chain.query;
    q.target = Some(target);
    q.depth = depth;
    q.node_limit = nodes;
    q.edge_limit = edges;
    q.min_amount_msat = (!min.is_empty()).then_some(min);
    q.start = model::utc_input(&start);
    q.end = model::utc_input(&end);
}

/// The browser's form validation before Run: a required path target, numbers in range and a
/// digits-only minimum amount. An invalid field opens Advanced (`reveal`).
pub fn valid(q: &Query) -> bool {
    let target_ok = q.mode != "path" || q.target.as_deref().is_some_and(|t| !t.trim().is_empty());
    let subject_ok = q.mode == "overview" || q.subject.as_deref().is_some_and(|s| !s.is_empty());
    let nums = (1..=50).contains(&q.depth)
        && (25..=2000).contains(&q.node_limit)
        && (50..=6000).contains(&q.edge_limit);
    let min_ok = q
        .min_amount_msat
        .as_deref()
        .is_none_or(|m| m.bytes().all(|b| b.is_ascii_digit()));
    let date_ok = |d: &Option<String>| {
        d.as_deref()
            .is_none_or(|d| d.len() >= 17 && d.as_bytes()[4] == b'-' && d.as_bytes()[10] == b'T')
    };
    target_ok && subject_ok && nums && min_ok && date_ok(&q.start) && date_ok(&q.end)
}

/// The Run button (the form's submit).
pub fn run(app: &mut App) {
    let q = app.provenance.chain.query.clone();
    if running(app) {
        return;
    }
    if !valid(&q) {
        if q.mode != "overview" && q.subject.as_deref().is_none_or(str::is_empty) {
            return;
        }
        app.provenance.chain.open.insert("filters".into());
        return;
    }
    execute(app, q);
}

/// `execute`: the bounded local query for `next` (it becomes the draft too).
pub fn execute(app: &mut App, next: Query) {
    let args = next.run_args();
    let s = st(app);
    s.version += 1;
    let version = s.version;
    s.error = None;
    s.notice = None;
    s.query = next;
    if app
        .send("ui.chain_analysis.query", args, "provenance.chain.query")
        .is_some()
    {
        st(app).inflight.push_back(version);
    }
    sync_edits(app);
}

/// Whether a query is in flight (`run.isPending`).
pub fn running(app: &App) -> bool {
    let s = &app.provenance.chain;
    s.inflight.contains(&s.version)
}

/// The error banner (`reportError`).
pub fn report(app: &mut App, e: impl Into<String>) {
    st(app).error = Some(e.into());
}

/// Selects a node or an edge (graph, table, inspector, findings).
pub fn select(app: &mut App, sel: Sel) {
    graph::on_select(app, &sel);
    let s = st(app);
    s.sel = Some(sel);
    s.copied = None;
}

/// Sources / Destinations in the inspector: a trace from the selected node.
pub fn trace(app: &mut App, direction: &str) {
    let s = &app.provenance.chain;
    let Some(node) = s
        .sel
        .as_ref()
        .filter(|x| !x.edge)
        .and_then(|x| s.result.as_ref()?.node(&x.id).cloned())
    else {
        return;
    };
    let next = Query {
        mode: "trace".into(),
        subject: Some(node.id),
        target: None,
        direction: direction.into(),
        ..s.query.clone()
    };
    execute(app, next);
}

/// Loads a saved investigation (`load`): its stored result, marked historical.
pub fn load_case(app: &mut App, saved: &Value) {
    let Some(result) = saved.get("result").filter(|r| r.is_object()) else {
        return;
    };
    let s = st(app);
    s.version += 1;
    s.query = Query::from_json(saved.get("query").unwrap_or(&Value::Null));
    s.result = Some(Rc::new(AResult::parse(result.clone())));
    s.historical = true;
    s.refresh_failed = false;
    s.sel = None;
    s.highlighted.clear();
    s.error = None;
    s.graph = Default::default();
    sync_edits(app);
}

/// "Investigate with assistant": the daemon projects the result's query and subject
/// (`ui.chain_analysis.ai_context`); only its projected identities enter the prompt.
pub fn ask(app: &mut App) {
    if app.provenance.chain.asking || app.assistant.streaming || !app.prefs.ai_features {
        return;
    }
    let s = &app.provenance.chain;
    let Some(r) = s.result.clone() else { return };
    let picked = s
        .sel
        .as_ref()
        .filter(|x| !x.edge)
        .and_then(|x| r.node(&x.id))
        .map(|n| n.id.clone())
        .or_else(|| r.query.subject.clone().filter(|v| !v.is_empty()))
        .or_else(|| s.query.subject.clone().filter(|v| !v.is_empty()));
    let mut args = json!({"query": r.raw.get("query").cloned().unwrap_or(Value::Null), "expected_snapshot_id": r.snapshot_id});
    if let Some(p) = picked {
        args["subject"] = json!(p);
    }
    if app
        .send(
            "ui.chain_analysis.ai_context",
            args,
            "provenance.chain.ai_context",
        )
        .is_some()
    {
        st(app).asking = true;
    }
}

/// The projected handoff (shared with the Privacy mirror): `assistantPrompt` with the daemon's
/// projected subject, query and snapshot.
pub fn handoff_prompt(data: &Value) -> String {
    let subject = data
        .get("subject")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t("chainAnalysis:overview"));
    let query = data.get("query").map(Value::to_string).unwrap_or_default();
    let snapshot = data
        .get("snapshot_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    crate::i18n::tr(
        "chainAnalysis:assistantPrompt",
        &[
            ("subject", &subject),
            ("query", &query),
            ("snapshot", &snapshot),
        ],
    )
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let Some(sub) = tag.strip_prefix("provenance.chain.") else {
        return;
    };
    match sub {
        "query" => {
            let version = st(app).inflight.pop_front();
            if version != Some(app.provenance.chain.version) {
                return;
            }
            match &r.error {
                Some(e) => {
                    let s = st(app);
                    s.refresh_failed = true;
                    s.error = Some(e.clone());
                }
                None if r.data.is_object() => {
                    let s = st(app);
                    s.result = Some(Rc::new(AResult::parse(r.data.clone())));
                    s.historical = false;
                    s.refresh_failed = false;
                    s.sel = None;
                    s.highlighted.clear();
                    s.graph = Default::default();
                }
                None => {}
            }
        }
        "ai_context" => {
            st(app).asking = false;
            match &r.error {
                Some(e) => report(app, e.clone()),
                None if r.data.is_object() => {
                    let prompt = handoff_prompt(&r.data);
                    crate::assistant::hand_off(app, &prompt);
                }
                None => {}
            }
        }
        "binding" => {
            let s = st(app);
            s.binding = match &r.error {
                Some(e) => Bind::Failed(e.clone()),
                None => Bind::Ready(r.data.clone()),
            };
            // An unbound book shows the network its wallets suggest (a local read).
            let unbound = app
                .provenance
                .chain
                .binding
                .ready()
                .is_some_and(|b| psbt::book_network(b).is_none());
            if unbound && app.provenance.chain.inventory.is_none() {
                app.send(
                    "ui.networks.inventory",
                    json!({}),
                    "provenance.chain.inventory",
                );
            }
        }
        "inventory" => {
            st(app).inventory = Some(if r.error.is_none() {
                r.data.clone()
            } else {
                json!({})
            })
        }
        "backends" => {
            st(app).backends = match &r.error {
                Some(e) => Bind::Failed(e.clone()),
                None => Bind::Ready(r.data.clone()),
            }
        }
        s if s.starts_with("watch")
            || s.starts_with("cases")
            || s.starts_with("acquire")
            || s.starts_with("sources") =>
        {
            panels::on_reply(app, s, r)
        }
        s if s.starts_with("entropy") || s.starts_with("labels") => results::on_reply(app, s, r),
        s if s.starts_with("psbt") || s.starts_with("stage") => psbt::on_reply(app, s, r),
        s if s.starts_with("datasets") => datasets::on_reply(app, s, r),
        _ => {}
    }
}

/// Every frame on `/chain-analysis`: mounts on a new route or book, polls running jobs, and
/// runs the deferred native dialogs (export, file pick) that need the `Tx`.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    if app.route.path() != "/chain-analysis" {
        if app.provenance.chain.mounted_for.is_some() {
            let old = std::mem::take(&mut app.provenance.chain);
            app.provenance.chain = State {
                inflight: old.inflight,
                version: old.version,
                ..State::default()
            };
        }
        return false;
    }
    let mut changed = false;
    if app.provenance.chain.mounted_for.as_deref() != Some(mount_key(app).as_str())
        && app.read("ui.profiles.snapshot").is_some()
    {
        mount(app);
        changed = true;
    }
    if let Some(format) = st(app).export.take() {
        export(app, tx, format);
        changed = true;
    }
    if let Some(k) = st(app).reveal.take() {
        tx.scroll_into_view(k);
    }
    changed |= results::poll(app, tx);
    changed |= datasets::poll(app, tx);
    changed |= panels::poll(app, tx);
    changed
}

/// Export JSON / CSV: the native save dialog names the destination; nothing else can.
fn export(app: &mut App, tx: &mut Tx, format: &'static str) {
    let Some(r) = app.provenance.chain.result.clone() else {
        return;
    };
    let contents = if format == "json" {
        serde_json::to_string_pretty(&r.raw).unwrap_or_default() + "\n"
    } else {
        model::csv(&r)
    };
    let name = model::export_name(&r.snapshot_id, format);
    let options = rune::FileDialogOptions {
        title: t("chainAnalysis:exportTitle"),
        file_name: Some(name),
        ..Default::default()
    }
    .filter(format.to_uppercase(), &[format]);
    tx.save_file(options, move |a: &mut App, result, _tx| match result {
        Ok(Some(dest)) => match std::fs::write(&dest, contents.as_bytes()) {
            Ok(()) => {
                st(a).export_actions = crate::toastopen::for_path(&dest.to_string_lossy(), false);
                st(a).notice = Some(t("chainAnalysis:exportSaved"));
            }
            Err(e) => report(a, e.to_string()),
        },
        Ok(None) => {}
        Err(e) => report(a, e.0),
    });
}

pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if app.route.path() != "/chain-analysis" {
        return false;
    }
    if matches!(ev.key, Key::Escape) {
        let s = st(app);
        if s.popup.is_some() {
            s.popup = None;
            return true;
        }
    }
    false
}

/// Headless: mount and wait for the mount reads.
pub fn prime(app: &mut App) {
    let mut tx = Tx::default();
    poll(app, &mut tx);
    settle(app);
}

/// Pumps the daemon until no `provenance.chain.` request is in flight (up to 20 s).
pub fn settle(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        app.pump();
        let mut tx = Tx::default();
        poll(app, &mut tx);
        let busy = app
            .pending
            .values()
            .any(|(tag, _, _)| tag.starts_with("provenance.chain."));
        if !busy || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod remount_tests {
    use super::*;

    /// A query from before leaving the workbench answers after it was opened again: it is
    /// dropped, and the new mount's own query still lands.
    #[test]
    fn an_earlier_mounts_query_reply_is_dropped() {
        let mut app = crate::app::tests::quiet_app();
        app.provenance.chain.mounted_for = Some("old".into());
        app.provenance.chain.version = 4;
        app.provenance.chain.inflight.push_back(4);
        app.navigate("/overview");
        let mut tx = Tx::default();
        poll(&mut app, &mut tx);
        let s = &mut app.provenance.chain;
        assert_eq!(s.inflight, [4], "the old query stays listed");
        // The new mount's query.
        s.version += 1;
        s.inflight.push_back(s.version);
        let reply = |n: u64| Reply {
            kind: "ui.chain_analysis.query".into(),
            data: json!({ "n": n }),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "provenance.chain.query", &reply(1));
        assert!(
            app.provenance.chain.result.is_none(),
            "the old answer is dropped"
        );
        assert!(running(&app));
        on_reply(&mut app, "provenance.chain.query", &reply(2));
        assert!(app.provenance.chain.result.is_some());
        assert!(!running(&app));
    }
}
