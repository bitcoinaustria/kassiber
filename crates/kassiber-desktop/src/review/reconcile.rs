//! Reconcile (`routes/Reconcile.tsx`): which pasted addresses and txids belong to a wallet in
//! this book (`ui.wallets.identify`, local), with an explicit, owner-pressed on-chain verify
//! (`ui.wallets.identify_onchain`, network egress) and a CSV import; and, in `?mode=tools`,
//! another tax tool's export checked against the book (`tools.rs`).
//!
//! Both modes stay in state while the other shows, so switching keeps a pasted list, a loaded
//! CSV and their results.

use std::collections::HashSet;

use rune::{Color, Cx, El, Key, KeyEvent, TextEdit, Tx, col, el, row, text};
use rune_kit::{Placement, TextArea, TextInput};
use serde_json::{Value, json};

use super::kit::{self, Var};
use super::model::{num, s, strs};
use super::{Popup, toggle};
use crate::app::{App, Reply, Route};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, sensitive, surface};

#[derive(Default)]
pub struct State {
    pub input: TextEdit,
    pub filter: String,
    pub query: TextEdit,
    pub copied_until: f64,
    pub report: Option<Value>,
    /// What the shown report was checked with; Verify re-runs exactly this.
    pub checked_args: Option<Value>,
    pub error: Option<String>,
    pub expanded: HashSet<String>,
    pub csv_text: Option<String>,
    pub csv_name: Option<String>,
    pub checking: bool,
    pub verifying: bool,
    pub progress: Option<(u64, u64)>,
    verify_request: Option<String>,
    pub cancel_pending: bool,
    pub backends: Option<Vec<Value>>,
    backends_requested: bool,
    pub backend: Option<String>,
    /// The failure label of the run in flight, and its args.
    failure: String,
    pending_args: Option<Value>,
}

fn st(app: &mut App) -> &mut State {
    &mut app.review.rc
}

pub fn mount(app: &mut App) {
    // Nothing is read on mount: the backend options load once a result needs Verify.
    let _ = app;
}

fn mode_tools(app: &App) -> bool {
    app.route.0.contains("mode=tools")
}

/// `setMode`: the mode lives in the URL, replacing the entry.
fn set_mode(app: &mut App, tools: bool) {
    app.route = Route(if tools {
        "/reconcile?mode=tools".into()
    } else {
        "/reconcile".into()
    });
}

const STATUSES: [&str; 5] = ["owned", "external", "unknown", "invalid", "unsupported"];

fn status_of(r: &Value) -> &'static str {
    let s = r["status"].as_str().unwrap_or("");
    STATUSES
        .iter()
        .copied()
        .find(|x| *x == s)
        .unwrap_or("unknown")
}

fn status_dot(tk: &Tk, status: &str) -> Color {
    let h = |x: &str| rune::color::hex(x).unwrap();
    match status {
        "owned" => kit::tw("emerald-500"),
        "external" => tk.muted_fg.alpha(0.6),
        "invalid" => tk.destructive,
        "unsupported" => h("#0ea5e9"),
        _ => kit::tw("amber-500"),
    }
}

fn status_label(status: &str) -> String {
    t(&format!("review:reconcile.status.{status}"))
}

fn type_label(ty: &str) -> String {
    let k = match ty {
        "address" => "address",
        "txid" => "txid",
        "lightning_invoice" => "lightningInvoice",
        "lightning_offer" => "lightningOffer",
        "lnurl" => "lnurl",
        "lightning_address" => "lightningAddress",
        "silent_payment_address" => "silentPaymentAddress",
        "outpoint" => "outpoint",
        "extended_public_key" => "extendedPublicKey",
        "private_key" => "privateKey",
        _ => return ty.to_owned(),
    };
    t(&format!("review:reconcile.type.{k}"))
}

fn classification_label(c: &str) -> String {
    let k = match c {
        "owned_address" => "ownedAddress",
        "external_address" => "externalAddress",
        "self_transfer" => "selfTransfer",
        "outbound_payment" => "outboundPayment",
        "inbound_receipt" => "inboundReceipt",
        "touches_wallet" => "touchesWallet",
        "external" => "external",
        "unknown" => "unknown",
        "undetermined" => "undetermined",
        "invalid" => "invalid",
        "unsupported" => "unsupported",
        _ => return c.to_owned(),
    };
    t(&format!("review:reconcile.classification.{k}"))
}

/// `ownerLabel`.
fn owner_label(r: &Value) -> String {
    if let Some(m) = r["matches"].as_array().filter(|m| !m.is_empty()) {
        let mut seen = vec![];
        for x in m {
            let w = s(x, "wallet");
            if !w.is_empty() && !seen.contains(&w) {
                seen.push(w);
            }
        }
        return seen.join(", ");
    }
    strs(&r["wallets"])
        .into_iter()
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `branchLabel`.
fn branch_label(r: &Value) -> String {
    let Some(p) = r["matches"].as_array().and_then(|m| m.first()) else {
        return String::new();
    };
    let where_ = s(p, "branch");
    let where_ = if where_.is_empty() {
        "address".to_owned()
    } else {
        where_
    };
    match p["address_index"].as_i64() {
        Some(i) => format!("{where_} #{i}"),
        None => where_,
    }
}

fn csv_cell(v: &str) -> String {
    if v.contains(['"', ',', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_owned()
    }
}

/// `resultsToCsv`.
fn results_csv(results: &[Value]) -> String {
    let mut lines = vec!["input,type,chain,status,classification,wallet,branch,note".to_owned()];
    for r in results {
        lines.push(
            [
                s(r, "input"),
                s(r, "type"),
                s(r, "chain"),
                s(r, "status"),
                s(r, "classification"),
                owner_label(r),
                branch_label(r),
                s(r, "note"),
            ]
            .iter()
            .map(|v| csv_cell(v))
            .collect::<Vec<_>>()
            .join(","),
        );
    }
    lines.join("\n")
}

/// `countEntries`: distinct entries, split the way the daemon splits pasted text.
fn count_entries(text: &str) -> usize {
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '|'))
        .filter(|x| !x.is_empty())
        .collect::<HashSet<_>>()
        .len()
}

fn results(app: &App) -> Vec<Value> {
    app.review
        .rc
        .report
        .as_ref()
        .and_then(|r| r["results"].as_array().cloned())
        .unwrap_or_default()
}

fn verify_count(app: &App) -> usize {
    let rs = results(app);
    let unknown = app
        .review
        .rc
        .report
        .as_ref()
        .map(|r| num(&r["summary"], "unknown") as usize)
        .unwrap_or(0);
    let missing = rs
        .iter()
        .filter(|r| {
            s(r, "type") == "txid"
                && s(r, "status") != "invalid"
                && r["legs"].as_array().is_none_or(Vec::is_empty)
        })
        .count();
    unknown.max(missing)
}

fn verify_backends(app: &App) -> Vec<Value> {
    app.review
        .rc
        .backends
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|b| {
            s(b, "kind").to_lowercase().contains("esplora")
                || s(b, "kind").to_lowercase().contains("electrum")
        })
        .collect()
}

fn verify_backend(app: &App) -> Option<Value> {
    let bs = verify_backends(app);
    let chosen = app
        .review
        .rc
        .backend
        .as_ref()
        .and_then(|n| bs.iter().find(|b| &s(b, "name") == n).cloned());
    chosen
        .or_else(|| {
            bs.iter()
                .find(|b| b["is_default"].as_bool() == Some(true))
                .cloned()
        })
        .or_else(|| bs.first().cloned())
}

fn current_args(app: &App, csv_override: Option<Option<String>>) -> Option<Value> {
    let rc = &app.review.rc;
    let csv = match csv_override {
        Some(c) => c,
        None => rc.csv_text.clone(),
    };
    let trimmed = rc.input.text.trim();
    if trimmed.is_empty() && csv.is_none() {
        return None;
    }
    let mut m = serde_json::Map::new();
    if !trimmed.is_empty() {
        m.insert("text".into(), json!(rc.input.text));
    }
    if let Some(c) = csv {
        m.insert("csv_text".into(), json!(c));
    }
    Some(Value::Object(m))
}

/// `runMutation`.
fn run(app: &mut App, kind: &str, tag: &'static str, failure: String, args: Option<Value>) {
    let Some(args) = args else { return };
    let rc = st(app);
    rc.error = None;
    rc.filter = "all".into();
    rc.query = TextEdit::new("");
    rc.expanded.clear();
    rc.failure = failure;
    let pending = args.clone();
    if let Some(id) = app.send(kind, args, tag) {
        let rc = st(app);
        if tag == "review.rc.verify" {
            rc.verifying = true;
            rc.verify_request = Some(id);
        } else {
            rc.checking = true;
        }
        rc.pending_args = Some(pending);
    }
}

/// `onCheck`: the local, cache-only ownership check.
fn on_check(app: &mut App) {
    let rc = &app.review.rc;
    let has_input = !rc.input.text.trim().is_empty() || rc.csv_text.is_some();
    if !has_input || rc.checking || rc.verifying {
        return;
    }
    let args = current_args(app, None);
    run(
        app,
        "ui.wallets.identify",
        "review.rc.check",
        t("review:reconcile.checkFailed"),
        args,
    );
}

/// Scenes: run the local check now.
pub fn check_now(app: &mut App) {
    on_check(app);
}

/// `onVerify`: asks the chosen backend about what local data cannot settle. Network egress, only
/// from the owner's press; it re-runs exactly the checked list.
fn on_verify(app: &mut App) {
    let Some(mut args) = app.review.rc.checked_args.clone() else {
        return;
    };
    if let Some(b) = verify_backend(app) {
        args["backend"] = json!(s(&b, "name"));
    }
    st(app).progress = None;
    run(
        app,
        "ui.wallets.identify_onchain",
        "review.rc.verify",
        t("review:reconcile.verifyFailed"),
        Some(args),
    );
}

fn on_stop(app: &mut App) {
    let Some(target) = app.review.rc.verify_request.clone() else {
        return;
    };
    if app
        .send(
            "ui.wallets.identify_onchain.cancel",
            json!({ "target_request_id": target }),
            "review.rc.cancel",
        )
        .is_some()
    {
        st(app).cancel_pending = true;
    }
}

/// The CSV picker: the file's content travels as `csv_text`, harvested daemon-side.
fn import_csv(_app: &mut App, tx: &mut Tx) {
    let options = rune::FileDialogOptions {
        title: t("review:reconcile.importCsv"),
        ..Default::default()
    }
    .filter("CSV", &["csv", "tsv", "txt"]);
    tx.open_file(options, |a: &mut App, result, _tx| {
        let Ok(Some(path)) = result else { return };
        match std::fs::read_to_string(&path) {
            Ok(content) if content.trim().is_empty() => {
                st(a).error = Some(t("review:reconcile.fileEmpty"))
            }
            Ok(content) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                st(a).csv_text = Some(content.clone());
                st(a).csv_name = Some(name);
                let args = current_args(a, Some(Some(content)));
                run(
                    a,
                    "ui.wallets.identify",
                    "review.rc.check",
                    t("review:reconcile.csvImportFailed"),
                    args,
                );
            }
            Err(_) => st(a).error = Some(t("review:reconcile.fileReadFailed")),
        }
    });
}

fn clear_input(app: &mut App) {
    let rc = st(app);
    rc.input = TextEdit::new("");
    rc.csv_text = None;
    rc.csv_name = None;
}

fn copy_csv(app: &mut App, tx: &mut Tx) {
    let rs = results(app);
    if rs.is_empty() {
        return;
    }
    crate::settings::effects::copy(app, tx, results_csv(&rs));
    st(app).copied_until = app.clock + 1.2;
    tx.wake_after(std::time::Duration::from_millis(1210));
}

pub fn on_progress(app: &mut App, env: &Value) {
    let d = &env["data"];
    if let (Some(c), Some(t)) = (d["checked"].as_u64(), d["total"].as_u64()) {
        st(app).progress = Some((c, t));
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "review.rc.check" | "review.rc.verify" => {
            let rc = st(app);
            if tag == "review.rc.verify" {
                rc.verifying = false;
                rc.progress = None;
                rc.verify_request = None;
                rc.cancel_pending = false;
            } else {
                rc.checking = false;
            }
            match &r.error {
                Some(e) => {
                    rc.error = Some(if e.is_empty() {
                        rc.failure.clone()
                    } else {
                        e.clone()
                    });
                }
                None => {
                    rc.report = (!r.data.is_null()).then(|| r.data.clone());
                    rc.checked_args = rc.pending_args.take();
                }
            }
            maybe_backends(app);
        }
        "review.rc.cancel" => st(app).cancel_pending = false,
        "review.rc.backends" if r.error.is_none() => {
            st(app).backends = Some(r.data["backends"].as_array().cloned().unwrap_or_default());
        }
        _ => {}
    }
}

/// The backend options: a config read (no network), once a result needs Verify.
fn maybe_backends(app: &mut App) {
    if verify_count(app) > 0 && !app.review.rc.backends_requested {
        st(app).backends_requested = true;
        app.send("ui.backends.options", Value::Null, "review.rc.backends");
    }
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let _ = (app, ev, tx);
    false
}

// ---- View --------------------------------------------------------------------------------------------

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    use crate::reports::ledger::common as c;
    let tools = mode_tools(app);
    // The page header (as Transfers and Journals): the modes on the left, the checked list's
    // on-chain verify on the right.
    let modes = c::seg_tabs(
        tk,
        "rc-modes",
        t("review:reconcile.modes.label"),
        true,
        vec![
            c::Seg::new(
                "rc-mode-addresses",
                t("review:reconcile.modes.addresses"),
                !tools,
                |a: &mut App| set_mode(a, false),
            )
            .glyph(&I::SEARCH),
            c::Seg::new(
                "rc-mode-tools",
                t("review:reconcile.modes.tools"),
                tools,
                |a: &mut App| set_mode(a, true),
            )
            .glyph(&I::ARROW_LEFT_RIGHT),
        ],
    );
    let actions = if tools {
        None
    } else {
        verify_actions(app, tk, layer)
    };
    let header = c::page_header(tk, modes, actions);
    let lede = |key: &str| {
        text(t(key))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .max_w(tk.r(48.0))
            .wrap()
    };
    let content = if tools {
        col()
            .key("rc-tools-panel")
            .w_full()
            .gap(tk.r(0.75))
            .role("tabpanel")
            .child(lede("review:reconcile.tools.toolsDescription"))
            .child(super::tools::view(app, cx, tk, layer))
    } else {
        col()
            .key("rc-addresses")
            .w_full()
            .gap(tk.r(0.75))
            .role("tabpanel")
            .child(lede("review:reconcile.description"))
            .child(addresses(app, cx, tk))
    };
    c::shell(tk).key("reconcile").child(header).child(content)
}

/// Verify on chain (with its backend menu) and Stop, once a check left something unsettled.
fn verify_actions(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Option<El> {
    let rc = &app.review.rc;
    rc.report
        .as_ref()
        .map(|r| r["summary"].clone())
        .filter(Value::is_object)?;
    let busy = rc.checking || rc.verifying;
    let vcount = verify_count(app);
    let backends = verify_backends(app);
    let backend = verify_backend(app);
    let mut actions = row().gap(tk.r(0.5)).center_y();
    if vcount > 0 {
        let label = if rc.verifying {
            match rc.progress {
                Some((c, total)) if total > 0 => tr(
                    "review:reconcile.verifyingProgress",
                    &[("checked", &c.to_string()), ("total", &total.to_string())],
                ),
                _ => t("review:reconcile.verifying"),
            }
        } else {
            tc("review:reconcile.verifyOnChain", vcount, &[])
        };
        let name = backend.as_ref().map(model_display).unwrap_or_default();
        let mut v = crate::reports::ledger::common::action(
            tk,
            "rc-verify",
            Var::Outline,
            &I::GLOBE,
            &label,
            rc.verifying,
            !busy,
        )
        .help(tr("review:reconcile.verifyHint", &[("backend", &name)]));
        if !busy {
            v = v.on_press(on_verify);
        }
        let mut group = row().center_y().child(if backends.len() > 1 {
            v.corners([tk.r(0.375), 0.0, 0.0, tk.r(0.375)])
        } else {
            v
        });
        if backends.len() > 1 {
            let mut chev = crate::txdetail::widgets::icon_button(
                tk,
                "rc-verify-backend",
                Var::Outline,
                tk.r(2.0),
                &I::CHEVRON_DOWN,
                tk.r(1.0),
                t("review:reconcile.verifyBackend"),
            )
            .w(tk.r(2.0))
            .ml(-1.0)
            .corners([0.0, tk.r(0.375), tk.r(0.375), 0.0])
            .expanded(app.review.popup == Some(Popup::VerifyBackend));
            if !busy {
                chev = crate::ui::popup::trigger(
                    chev,
                    crate::ui::popup::Kind::Menu,
                    app.review.popup == Some(Popup::VerifyBackend),
                    |a: &mut App| toggle(a, Popup::VerifyBackend),
                );
            }
            group = group.child(chev);
            if app.review.popup == Some(Popup::VerifyBackend) {
                backend_menu(
                    tk,
                    layer,
                    &backends,
                    backend.as_ref().map(|b| s(b, "name")).unwrap_or_default(),
                );
            }
        }
        actions = actions.child(group);
    }
    if rc.verifying {
        let mut stop = kit::btn(
            tk,
            "rc-stop",
            Var::Ghost,
            Some(&I::SQUARE),
            &t("review:reconcile.stopVerify"),
            !rc.cancel_pending,
        );
        if !rc.cancel_pending {
            stop = stop.on_press(on_stop);
        }
        actions = actions.child(stop);
    }
    (vcount > 0 || rc.verifying).then_some(actions)
}

/// The workbench: entries on the left (33 rem), results on the right; stacked below `xl`.
fn addresses(app: &App, cx: &mut Cx<App>, tk: &Tk) -> El {
    let rc = &app.review.rc;
    let hidden = app.prefs.hide_sensitive();
    let summary = rc
        .report
        .as_ref()
        .map(|r| r["summary"].clone())
        .filter(Value::is_object);
    let busy = rc.checking || rc.verifying;
    let width = cx
        .layout_size("reconcile")
        .map(|[w, _]| w)
        .unwrap_or(1100.0);
    // Side by side while the page itself has room for both (the side panel narrows it as much
    // as a smaller window does): the entries' 33 rem and ~34 rem of results.
    let xl = width >= tk.r(33.0 + 34.0) + tk.card_gap() + 2.0 * tk.page_pad();
    // The results card's inner width (before its first layout: the page less its padding and,
    // side by side, the entries column).
    let fallback =
        width - 2.0 * tk.page_pad() - if xl { tk.r(33.0) + tk.card_gap() } else { 0.0 } - 2.0;
    let results_w = cx
        .layout_size("rc-results")
        .map(|[w, _]| w - 2.0)
        .filter(|w| *w > 0.0)
        .unwrap_or(fallback);
    // The empty results card matches the entries card's height beside it.
    let input_h = if xl {
        cx.layout_size("rc-input-section")
            .map(|[_, h]| h)
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let input = input_section(app, tk, hidden, busy, xl);
    let right = results_column(app, tk, hidden, summary.as_ref(), results_w, input_h);
    if xl {
        row()
            .w_full()
            .gap(tk.card_gap())
            .items_start()
            .child(input.w(tk.r(33.0)).shrink().min_w(0.0))
            .child(right.grow(1.0).w(0.0).min_w(0.0))
    } else {
        col()
            .w_full()
            .gap(tk.card_gap())
            .child(input.w_full())
            .child(right.w_full())
    }
}

fn model_display(b: &Value) -> String {
    let d = s(b, "display_name");
    if d.is_empty() { s(b, "name") } else { d }
}

fn backend_menu(tk: &Tk, layer: &mut Vec<El>, backends: &[Value], current: String) {
    let mut p =
        crate::ui::popup::Popup::menu("rc-verify-backend", |a: &mut App| a.review.popup = None);
    p.add(
        row().w_full().px(tk.r(0.5)).py(tk.r(0.375)).child(
            text(t("review:reconcile.verifyBackend"))
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        ),
    );
    for b in backends {
        let name = s(b, "name");
        let checked = name == current;
        let meta = [s(b, "chain"), s(b, "network")]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let n = name.clone();
        let mut r = row()
            .w_full()
            .h(tk.r(2.0))
            .pl(tk.r(2.0))
            .pr(tk.r(0.5))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.25))
            .hover(tk.accent)
            .focus_highlight(tk.accent)
            .role("menuitemradio")
            .checked(checked)
            .label(model_display(b))
            .focusable(true)
            .on_press(move |a: &mut App| {
                a.review.rc.backend = Some(n.clone());
                a.review.popup = None;
            });
        if checked {
            r = r.child(
                row()
                    .absolute(tk.r(0.5), (tk.r(2.0) - tk.r(0.875)) / 2.0)
                    .size(tk.r(0.875), tk.r(0.875))
                    .center()
                    .child(crate::ui::dot(tk.r(0.5), tk.fg)),
            );
        }
        r = r
            .child(
                text(model_display(b))
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(text(meta).font(tk.txs(400.0)).color(tk.muted_fg));
        p.item(
            r,
            format!("rc-backend-{name}"),
            model_display(b),
            true,
            checked,
        );
    }
    p.mount(
        tk,
        layer,
        col().w(tk.r(16.0)).padding(tk.r(0.25) + 1.0),
        |l| crate::shell::glass(tk, l),
        Placement::BottomEnd,
        4.0,
    );
}

fn input_section(app: &App, tk: &Tk, hidden: bool, busy: bool, side_by_side: bool) -> El {
    let rc = &app.review.rc;
    let entries = count_entries(&rc.input.text);
    let has_input = !rc.input.text.trim().is_empty() || rc.csv_text.is_some();
    let title = t("review:reconcile.inputTitle");
    let count = (entries > 0).then(|| {
        text(tc("review:reconcile.entryCount", entries, &[]))
            .font(tk.txs(400.0).features("'tnum' 1"))
            .color(tk.muted_fg)
    });
    let head = crate::reports::ledger::common::card_head(tk, title.clone(), None, count)
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border);
    let mut k = crate::txdetail::widgets::form_kit(tk);
    k.c.bg_base = Color::TRANSPARENT;
    k.c.hover = Color::TRANSPARENT;
    k.ty.ui = tk.txs(400.0).mono();
    let area = TextArea::new(&rc.input)
        .label(title.clone())
        .placeholder(t("review:reconcile.placeholder"))
        .rows(if side_by_side { 16 } else { 8 })
        .width(None)
        .spellcheck(false)
        .on_edit(|a: &mut App| &mut a.review.rc.input)
        .on_key(|a: &mut App, ev: &KeyEvent, _tx: &mut Tx| {
            if ev.key == Key::Enter && (ev.mods.cmd || ev.mods.ctrl) {
                on_check(a);
                return true;
            }
            false
        })
        .build(&k)
        .key("rc-input")
        .min_h(tk.r(if side_by_side { 18.0 } else { 10.0 }))
        .blur(if hidden { 3.5 } else { 0.0 });
    let mut sec = surface(tk)
        .key("rc-input-section")
        .role("region")
        .label(title)
        .clip_rounded(tk.r(0.75))
        .child(head)
        .child(col().w_full().px(tk.r(0.75)).py(tk.r(0.25)).child(area));
    if let Some(name) = &rc.csv_name {
        sec = sec.child(
            row().w_full().px(tk.card_pad()).pb(tk.r(0.75)).child(
                row()
                    .max_w(tk.r(30.0))
                    .px(tk.r(0.5))
                    .py(tk.r(0.25))
                    .gap(tk.r(0.5))
                    .center_y()
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border)
                    .fill(tk.muted.alpha(0.4))
                    .child(ic(&I::FILE_SPREADSHEET, tk.r(0.875), tk.muted_fg))
                    .child(
                        text(name.clone())
                            .font(tk.txs(500.0))
                            .color(tk.fg)
                            .truncate()
                            .min_w(0.0),
                    )
                    .child(
                        row()
                            .key("rc-remove-csv")
                            .radius(tk.r(0.125))
                            .role("button")
                            .label(t("review:reconcile.removeCsvAria"))
                            .focusable(true)
                            .hover_group()
                            .on_press(|a: &mut App| {
                                a.review.rc.csv_text = None;
                                a.review.rc.csv_name = None;
                            })
                            .child(ic(&I::X, tk.r(0.875), tk.muted_fg).group_color(tk.fg)),
                    ),
            ),
        );
    }
    let mut bar = row().w_full().gap(tk.r(0.5)).center_y();
    let mut import = kit::btn(
        tk,
        "rc-import-csv",
        Var::Ghost,
        Some(&I::FILE_SPREADSHEET),
        &t("review:reconcile.importCsv"),
        !busy,
    );
    if !busy {
        import = import.on_press_tx(import_csv);
    }
    bar = bar.child(import);
    if has_input {
        let mut clear = kit::btn(
            tk,
            "rc-clear",
            Var::Ghost,
            None,
            &t("review:reconcile.clear"),
            !busy,
        );
        if !busy {
            clear = clear.on_press(clear_input);
        }
        bar = bar.child(clear);
    }
    let enabled = has_input && !busy;
    let label = if rc.checking {
        t("review:reconcile.checking")
    } else {
        t("review:reconcile.checkOwnership")
    };
    let shortcut = if cfg!(target_os = "macos") {
        "⌘↩"
    } else {
        "Ctrl+Enter"
    };
    let mut check = crate::ui::button::button(
        tk,
        "rc-check",
        crate::ui::button::Variant::Default,
        crate::ui::button::Size::Sm,
        Some(&I::SEARCH),
        &label,
        enabled,
    )
    .shortcut_hint(shortcut.to_owned())
    .child(
        row()
            .h(tk.r(1.25))
            .min_w(tk.r(1.25))
            .px(tk.r(0.25))
            .center()
            .radius(tk.r(0.25))
            .fill(tk.primary_fg.alpha(0.15))
            .a11y_hidden()
            .child(
                text(shortcut)
                    .font(tk.tkbd())
                    .color(tk.primary_fg.alpha(0.8)),
            ),
    );
    if enabled {
        check = check.on_press(on_check);
    }
    bar = bar.child(el().grow(1.0)).child(check);
    // One footer: the actions, then where the check runs.
    sec.child(
        col()
            .w_full()
            .pl(tk.r(1.0))
            .pr(tk.r(0.75))
            .pt(tk.r(0.625))
            .pb(tk.r(0.75))
            .gap(tk.r(0.5))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .child(bar)
            .child(
                row()
                    .w_full()
                    .pl(tk.r(0.5))
                    .gap(tk.r(0.375))
                    .items_start()
                    .child(
                        ic(&I::SHIELD_CHECK, tk.r(0.875), tk.muted_fg)
                            .mt(1.0)
                            .a11y_hidden(),
                    )
                    .child(
                        text(t("review:reconcile.onDeviceNote"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            ),
    )
}

fn status_label_el(tk: &Tk, status: &str) -> El {
    row()
        .gap(tk.r(0.375))
        .center_y()
        .child(crate::ui::dot(tk.r(0.5), status_dot(tk, status)).a11y_hidden())
        .child(text(status_label(status)).font(tk.tsm(400.0)).color(tk.fg))
}

/// No results yet: Review's centred state in a card as tall as the entries beside it, the
/// three statuses as its legend.
fn empty_results(tk: &Tk, min_h: f32) -> El {
    let mut dl = col().w_full().mt(tk.r(0.75)).gap(tk.r(0.5)).role("list");
    for st in ["owned", "external", "unknown"] {
        dl = dl.child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_start()
                .role("listitem")
                .child(row().w(tk.r(6.5)).child(status_label_el(tk, st)))
                .child(
                    text(t(&format!("review:reconcile.empty.{st}")))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .grow(1.0)
                        .w(0.0)
                        .min_w(0.0),
                ),
        );
    }
    let body = col()
        .w_full()
        .max_w(tk.r(30.0))
        .items_center()
        .gap(tk.r(0.5))
        .child(
            row()
                .size(tk.r(3.0), tk.r(3.0))
                .radius(tk.r(1.5))
                .center()
                .fill(tk.muted)
                .mb(tk.r(0.5))
                .a11y_hidden()
                .child(ic(&I::SEARCH, tk.r(1.5), tk.muted_fg)),
        )
        .child(
            text(t("review:reconcile.empty.title"))
                .font(tk.tbase(600.0))
                .color(tk.fg)
                .align(rune::Align::Center)
                .role("heading")
                .level(2),
        )
        .child(dl);
    surface(tk)
        .key("rc-empty")
        .w_full()
        .min_h(min_h.max(tk.r(16.0)))
        .padding(tk.r(1.5))
        .center()
        .child(body)
}

fn results_column(
    app: &App,
    tk: &Tk,
    hidden: bool,
    summary: Option<&Value>,
    avail: f32,
    empty_h: f32,
) -> El {
    let rc = &app.review.rc;
    let mut c = col().w_full().gap(tk.r(0.75));
    if let Some(e) = &rc.error {
        c = c.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .radius(tk.r(0.5))
                .border(1.0, tk.destructive.alpha(0.3))
                .fill(tk.destructive.alpha(0.1))
                .role("alert")
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap(),
                ),
        );
    }
    let Some(sum) = summary else {
        return c.child(empty_results(tk, empty_h));
    };
    let rs = results(app);
    let n = |k: &str| num(sum, k) as usize;
    let mut filters: Vec<(&str, String, usize)> = vec![
        ("all", t("review:reconcile.filterAll"), n("total")),
        ("owned", status_label("owned"), n("owned")),
        ("external", status_label("external"), n("external")),
        ("unknown", status_label("unknown"), n("unknown")),
    ];
    if n("invalid") > 0 {
        filters.push(("invalid", status_label("invalid"), n("invalid")));
    }
    if n("unsupported") > 0 {
        filters.push(("unsupported", status_label("unsupported"), n("unsupported")));
    }
    let current = if rc.filter.is_empty() {
        "all"
    } else {
        rc.filter.as_str()
    };
    use crate::reports::ledger::common as lc;
    use crate::ui::table as tb;
    let segs = filters
        .iter()
        .map(|(id, label, count)| {
            let idv = id.to_string();
            let seg = lc::Seg::new(
                format!("rc-filter-{id}"),
                label.clone(),
                *id == current,
                move |a: &mut App| a.review.rc.filter = idv.clone(),
            )
            .count(count.to_string());
            if *id == "all" {
                seg
            } else {
                seg.dot(status_dot(tk, id))
            }
        })
        .collect();
    let group = lc::seg_tabs_wrap(tk, "rc-filters", t("common:actions.filter"), false, segs);
    let fk = {
        let mut k = crate::txdetail::widgets::form_kit(tk);
        k.ty.ui = tk.tsm(400.0);
        k
    };
    let search = crate::txdetail::widgets::field(
        tk,
        TextInput::new(&rc.query)
            .label(t("review:reconcile.filterPlaceholder"))
            .placeholder(t("review:reconcile.filterPlaceholder"))
            .height(tk.r(2.0))
            .width(None)
            .leading_icon(&I::SEARCH)
            .leading_icon_size(tk.r(0.875))
            .text_style(tk.tsm(400.0))
            .on_edit(|a: &mut App| &mut a.review.rc.query)
            .build(&fk)
            .key("rc-filter-query"),
    );
    let copied = app.clock < rc.copied_until;
    let mut copy = kit::btn(
        tk,
        "rc-copy-csv",
        Var::Outline,
        Some(&I::CLIPBOARD_COPY),
        &if copied {
            t("review:reconcile.copied")
        } else {
            t("review:reconcile.copyCsv")
        },
        !rs.is_empty(),
    );
    if !rs.is_empty() {
        copy = copy.on_press_tx(copy_csv);
    }
    let toolbar = row()
        .w_full()
        .px(tk.r(tb::EDGE))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(group)
        .child(el().grow(1.0))
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(row().w(tk.r(14.0)).child(search))
                .child(copy),
        );
    // Rows.
    let needle = rc.query.text.trim().to_lowercase();
    let filtered: Vec<(usize, &Value)> = rs
        .iter()
        .enumerate()
        .filter(|(_, r)| current == "all" || status_of(r) == current)
        .filter(|(_, r)| {
            needle.is_empty()
                || [
                    s(r, "input"),
                    owner_label(r),
                    branch_label(r),
                    s(r, "classification"),
                ]
                .join(" ")
                .to_lowercase()
                .contains(&needle)
        })
        .collect();
    // The table kit: a chevron slot on the card's inset, the input, its status, then the match
    // and classification sharing the rest; long text cut with its tooltip.
    let lead_w = tk.r(tb::EDGE) + tk.r(1.25);
    let cols = |a: El, b: El, c2: El, d: El, e: El| {
        row()
            .w_full()
            .items_start()
            .child(a.w(lead_w))
            .child(b.w(tk.r(16.0)).shrink().min_w(tk.r(8.0)))
            .child(c2.w(tk.r(7.5)))
            .child(d.grow(1.0).w(0.0).min_w(tk.r(7.0)))
            .child(e.grow(1.0).w(0.0).min_w(tk.r(7.0)).pr(tk.r(tb::EDGE)))
    };
    let th = |s: String| {
        row()
            .h(tk.r(tb::HEAD_H))
            .px(tk.r(tb::PX))
            .center_y()
            .min_w(0.0)
            .child(tb::th_text(tk, s))
    };
    let mut table = col().w_full().role("table").child(
        cols(
            el().h(tk.r(tb::HEAD_H)),
            th(t("review:reconcile.table.input")),
            th(t("common:field.status")),
            th(t("review:reconcile.table.match")),
            th(t("review:reconcile.table.classification")),
        )
        .fill(tk.muted.alpha(0.5))
        .role("row")
        .border_sides([1.0, 0.0, 1.0, 0.0], tk.border),
    );
    if filtered.is_empty() {
        table = table.child(tb::empty_text(tk, t("review:reconcile.table.noResults")));
    }
    let last = filtered.len().saturating_sub(1);
    for (pos, (index, r)) in filtered.into_iter().enumerate() {
        let key = format!("{}-{index}", s(r, "input"));
        let legs = r["legs"].as_array().cloned().unwrap_or_default();
        let has_legs = !legs.is_empty();
        let open = rc.expanded.contains(&key);
        let k2 = key.clone();
        let toggle_row = move |a: &mut App| {
            if !a.review.rc.expanded.remove(&k2) {
                a.review.rc.expanded.insert(k2.clone());
            }
        };
        let chevron = if has_legs {
            row()
                .key(format!("rc-legs-{index}"))
                .padding(2.0)
                .radius(tk.r(0.375))
                .hover(tk.accent)
                .role("button")
                .label(if open {
                    t("review:reconcile.hideLegsAria")
                } else {
                    t("review:reconcile.showLegsAria")
                })
                .expanded(open)
                .focusable(true)
                .on_press(toggle_row.clone())
                .child(ic(
                    if open {
                        &I::CHEVRON_DOWN
                    } else {
                        &I::CHEVRON_RIGHT
                    },
                    tk.r(0.875),
                    tk.muted_fg,
                ))
        } else {
            el()
        };
        let input = s(r, "input");
        let ty = format!(
            "{}{}",
            type_label(&s(r, "type")),
            if s(r, "chain").is_empty() {
                String::new()
            } else {
                format!(" · {}", s(r, "chain"))
            }
        );
        let copy_value = input.clone();
        let input_cell = col()
            .p_cell(tk)
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.25))
                    .center_y()
                    .hover_group()
                    .child(if hidden {
                        sensitive(input.clone(), tk.txs(400.0).mono(), tk.fg, true)
                            .single_line()
                            .truncate()
                            .min_w(0.0)
                    } else {
                        tb::cell_text(input.clone(), tk.txs(400.0).mono(), tk.fg)
                    })
                    .child(
                        crate::txdetail::widgets::icon_button(
                            tk,
                            format!("rc-copy-{index}"),
                            Var::Ghost,
                            tk.r(1.5),
                            &I::COPY,
                            tk.r(0.75),
                            t("review:reconcile.copyInputAria"),
                        )
                        .group_opacity(0.0, 1.0)
                        .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                            crate::settings::effects::copy(a, tx, copy_value.clone())
                        }),
                    ),
            )
            .child(tb::cell_text(ty, tk.txs(400.0), tk.muted_fg));
        let owner = owner_label(r);
        let branch = branch_label(r);
        let mut match_cell = col().p_cell(tk);
        if owner.is_empty() {
            match_cell = match_cell.child(tb::dash(tk));
        } else {
            match_cell = match_cell.child(tb::cell_text(owner, tk.tsm(500.0), tk.fg));
            if !branch.is_empty() {
                match_cell = match_cell.child(tb::cell_text(branch, tk.txs(400.0), tk.muted_fg));
            }
        }
        let mut class_cell = col().p_cell(tk).child(tb::cell_text(
            classification_label(&s(r, "classification")),
            tk.tsm(400.0),
            tk.fg,
        ));
        if !s(r, "note").is_empty() {
            class_cell = class_cell.child(tb::cell_text(s(r, "note"), tk.txs(400.0), tk.muted_fg));
        }
        let mut tr_el = cols(
            row().pl(tk.r(tb::EDGE) - 2.0).pt(tk.r(0.5)).child(chevron),
            input_cell,
            row().p_cell(tk).child(status_label_el(tk, status_of(r))),
            match_cell,
            class_cell,
        )
        .key(format!("rc-row-{index}"))
        .role("row")
        .hover(tk.muted.alpha(0.35));
        if pos != last || open {
            tr_el = tr_el.border_sides([0.0, 0.0, 1.0, 0.0], tk.border);
        }
        if open {
            tr_el = tr_el.fill(tk.muted.alpha(0.6)).hover(tk.muted.alpha(0.6));
        }
        if has_legs {
            tr_el = tr_el.cursor(rune::Cursor::Pointer).on_press(toggle_row);
        }
        table = table.child(tr_el);
        if has_legs && open {
            let mut legs_row = row()
                .w_full()
                .pl(lead_w)
                .pr(tk.r(tb::EDGE))
                .py(tk.r(0.75))
                .fill(tk.muted.alpha(0.25))
                .child(legs_breakdown(tk, &legs, hidden));
            if pos != last {
                legs_row = legs_row.border_sides([0.0, 0.0, 1.0, 0.0], tk.border);
            }
            table = table.child(legs_row);
        }
    }
    let mut foot = col()
        .w_full()
        .px(tk.r(tb::EDGE))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
    let checked = rc.checked_args.as_ref();
    let list_changed = checked.is_some_and(|c| {
        let trimmed = !rc.input.text.trim().is_empty();
        c["text"].as_str().unwrap_or("") != if trimmed { rc.input.text.as_str() } else { "" }
            || c["csv_text"].as_str().map(str::to_owned) != rc.csv_text
    });
    if list_changed {
        foot = foot.child(
            text(t("review:reconcile.listChanged"))
                .font(tk.txs(500.0))
                .color(kit::amber_text(tk))
                .wrap(),
        );
    }
    let verified = if sum["verified_on_chain"].as_bool() == Some(true) {
        t("review:reconcile.verifiedSuffix")
    } else {
        String::new()
    };
    foot = foot.child(
        text(tc(
            "review:reconcile.scanned",
            n("wallets_scanned"),
            &[
                ("index", &n("scan_to_index").to_string()),
                ("verified", &verified),
            ],
        ))
        .font(tk.txs(400.0))
        .color(tk.muted_fg)
        .wrap(),
    );
    let warnings = rc
        .report
        .as_ref()
        .map(|r| strs(&r["warnings"]))
        .unwrap_or_default();
    if !warnings.is_empty() {
        let ink = kit::amber_text(tk);
        let mut ul = col()
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .gap(tk.r(0.25))
            .radius(tk.r(0.5))
            .border(1.0, rune::color::hex("#f59e0b").unwrap().alpha(0.3))
            .fill(rune::color::hex("#f59e0b").unwrap().alpha(0.1))
            .role("list");
        for w in warnings {
            ul = ul.child(
                row()
                    .gap(tk.r(0.375))
                    .items_start()
                    .role("listitem")
                    .child(ic(&I::TRIANGLE_ALERT, tk.r(0.875), ink).mt(1.0))
                    .child(
                        text(w)
                            .font(tk.txs(400.0))
                            .color(ink)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        foot = foot.child(ul);
    }
    // shadcn's `overflow-x-auto` table container (its cells don't wrap): narrower than the
    // columns' least widths, the table scrolls sideways under the toolbar.
    let (table_w, overflow) = tb::fit(avail, lead_w + tk.r(8.0 + 7.5 + 7.0 + 7.0 + tb::EDGE));
    let table = tb::hscroll_if(
        "rc-results-scroll",
        t("review:reconcile.resultsTitle"),
        overflow,
        table.w(table_w),
    );
    c.child(
        surface(tk)
            .key("rc-results")
            .w_full()
            .role("region")
            .label(t("review:reconcile.resultsTitle"))
            .clip_rounded(tk.r(0.75))
            .child(toolbar)
            .child(table)
            .child(foot),
    )
}

trait Cell {
    fn p_cell(self, tk: &Tk) -> Self;
}
impl Cell for El {
    /// `TableCell`: `p-2 align-top`.
    fn p_cell(self, tk: &Tk) -> El {
        self.padding(tk.r(0.5))
    }
}

fn legs_breakdown(tk: &Tk, legs: &[Value], hidden: bool) -> El {
    let column = |title: String, side: &str| {
        let ls: Vec<&Value> = legs.iter().filter(|l| s(l, "side") == side).collect();
        let mut c = kit::inset(tk)
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .grow(1.0)
            .w(0.0)
            .child(
                row()
                    .gap(tk.r(0.25))
                    .child(text(title).font(tk.txs(500.0)).color(tk.muted_fg))
                    .child(
                        text(ls.len().to_string())
                            .font(tk.txs(500.0).features("'tnum' 1"))
                            .color(tk.muted_fg),
                    ),
            );
        if ls.is_empty() {
            return c.child(
                text("—")
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .py(tk.r(0.25)),
            );
        }
        for (i, l) in ls.iter().enumerate() {
            let label = if side == "input" {
                match l["outpoint"].as_str().filter(|o| !o.is_empty()) {
                    Some(o) if o.chars().count() > 17 => format!(
                        "{}…{}",
                        o.chars().take(10).collect::<String>(),
                        o.chars().skip(o.chars().count() - 6).collect::<String>()
                    ),
                    Some(o) => o.to_owned(),
                    None => t("review:reconcile.legs.input"),
                }
            } else {
                format!(
                    "#{}",
                    l["n"]
                        .as_i64()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "?".into())
                )
            };
            let right = if l["owned"].as_bool() == Some(true) {
                let b = s(l, "branch");
                text(format!(
                    "{}{}",
                    s(l, "wallet"),
                    if b.is_empty() {
                        String::new()
                    } else {
                        format!(" · {b}")
                    }
                ))
                .font(tk.txs(500.0))
                .color(kit::emerald_text(tk))
                .truncate()
            } else {
                text(t("review:reconcile.legs.external"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
            };
            let mut r = row()
                .w_full()
                .py(tk.r(0.25))
                .gap(tk.r(0.75))
                .center_y()
                .justify_between()
                .child(sensitive(label, tk.txs(400.0).mono(), tk.fg, hidden))
                .child(right);
            if i > 0 {
                r = r.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
            }
            c = c.child(r);
        }
        c
    };
    row()
        .w_full()
        .gap(tk.r(0.5))
        .items_start()
        .child(column(t("review:reconcile.legs.inputs"), "input"))
        .child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.muted_fg).self_center())
        .child(column(t("review:reconcile.legs.outputs"), "output"))
}
