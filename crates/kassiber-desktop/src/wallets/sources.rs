//! The Coverage card on a wallet connection's page (SourceCoverageCard.tsx;
//! docs/reference/desktop.md#sources-live-sync-and-file-snapshots): live sync or file snapshot,
//! how far the source reaches, whether it is current, and the one action that fixes it.
//!
//! It reads `ui.sources.coverage` for this wallet only, a local read. Check file contents hashes
//! the file the wallet remembers importing, and only on that press. Nothing here contacts a
//! network: Sync now and Import newer export are the page's existing actions. A statement ("no
//! activity after", "account closed on") closes the coverage gap from its date on; it creates
//! no transactions, balances or tax facts.

use rune::{Color, El, Key, KeyEvent, TextEdit, Tx, col, row, text};
use serde_json::{Value, json};

use super::coverage;
use super::detail::Read;
use super::detail::ui::{self as dui, amber};
use crate::app::{App, Reply};
use crate::i18n::{has, t, tc, tr};
use crate::icons as I;
use crate::overview::add_connection::Source;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button};
use crate::wallets::conn::Conn;

/// This card's state (`app.wallets.sources`); a book switch starts it over.
#[derive(Default)]
pub struct State {
    /// The wallet the read belongs to.
    pub wallet: String,
    pub read: Read<Value>,
    /// The book refresh cycle the read follows (`App::book_generation`).
    pub generation: u64,
    /// Check file contents: in flight, and its answer (`same` | `different` | `unknown`).
    pub checking: bool,
    pub file_check: Option<String>,
    /// Import newer export was pressed: the coverage then and the import count then; the next
    /// read after a successful import says what it added.
    pub before: Option<(Value, u64)>,
    pub import_result: Option<String>,
    pub attest: Option<AttestForm>,
    pub clear: Option<ClearForm>,
}

/// The statement dialog.
pub struct AttestForm {
    /// `no_activity_after` | `closed_on`.
    pub kind: &'static str,
    pub date: TextEdit,
    pub note: TextEdit,
    pub error: Option<String>,
    pub pending: bool,
}

#[derive(Default)]
pub struct ClearForm {
    pub error: Option<String>,
    pub pending: bool,
}

// ---- Model -----------------------------------------------------------------------------------

/// This wallet's coverage, from its read.
fn coverage_of<'a>(st: &'a State, id: &str) -> Option<&'a Value> {
    st.read.ready()?["sources"]
        .as_array()?
        .iter()
        .find(|c| c["wallet_id"].as_str() == Some(id))
}

/// The parser for Import newer export: the catalog's ready file format whose stored format the
/// wallet names (its kind matching, or a generic wallet), else the one its kind selects.
pub fn file_source(c: &Conn) -> Option<&'static Source> {
    use crate::wallets::add::{is_history_source, known_wallet_import_source, meta::meta};
    let fmt = c.source_format.as_deref().filter(|f| !f.is_empty());
    crate::overview::catalog::SOURCES
        .iter()
        .find(|s| {
            let m = meta(s.id);
            !s.planned
                && is_history_source(s)
                && fmt.is_some()
                && m.source_format == fmt
                && (m.wallet_kind.is_none()
                    || m.wallet_kind == Some(c.kind.as_str())
                    || c.kind == "custom"
                    || c.kind == "csv")
        })
        .or_else(|| known_wallet_import_source(Some(c.kind.as_str())))
}

/// The "how to get a newer export" line's key: the provider's hint, else the generic one for a
/// file source.
pub fn refresh_hint_key(src: Option<&Source>, is_file: bool) -> Option<String> {
    let hinted = src
        .and_then(|s| crate::wallets::add::meta::refresh_hint_key(s.id))
        .map(|k| format!("connections:coverage.refreshHints.{k}"));
    hinted.or_else(|| is_file.then(|| "connections:coverage.refreshHints.generic".to_owned()))
}

/// The covered window line.
pub fn window_text(c: &Value) -> String {
    let from = coverage::format_day(c["covered_from"].as_str());
    match (from, coverage::format_day(c["covered_through"].as_str())) {
        (Some(from), Some(through)) => tr(
            "connections:coverage.card.window",
            &[("from", &from), ("through", &through)],
        ),
        (None, Some(through)) => tr(
            "connections:coverage.card.windowThrough",
            &[("through", &through)],
        ),
        _ => t("connections:coverage.card.windowNone"),
    }
}

/// The detail lines under the window (last sync, imports, hybrid and file notes, the user's
/// statement, its gap and the scan warnings), each with whether it is a warning.
pub fn facts(c: &Value, now: f64) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mode = coverage::mode(c);
    let live = matches!(mode, "live" | "hybrid");
    if live
        && let Some(when) = c["live"]["last_sync_at"]
            .as_str()
            .and_then(|ts| crate::overview::l10n::relative(ts, now))
    {
        out.push((
            if c["live"]["last_sync_status"] == "error" {
                t("connections:coverage.state.syncFailed")
            } else {
                tr("connections:coverage.card.lastSync", &[("when", &when)])
            },
            false,
        ));
    }
    let imports = c["file"]["import_count"].as_u64().unwrap_or(0) as usize;
    if imports > 0 {
        let date = coverage::format_day(c["file"]["last_import_at"].as_str())
            .unwrap_or_else(|| "—".into());
        out.push((
            tc(
                "connections:coverage.card.imports",
                imports,
                &[("date", &date)],
            ),
            false,
        ));
    }
    if mode == "hybrid" {
        out.push((
            match coverage::format_day(c["live"]["api_history_limit"].as_str()) {
                Some(date) => tr("connections:coverage.card.hybridNote", &[("date", &date)]),
                None => t("connections:coverage.card.hybridNoteNoLimit"),
            },
            false,
        ));
    }
    if mode == "file" {
        out.push((t("connections:coverage.card.fileNote"), false));
    }
    if let Some(a) = c.get("attestation").filter(|a| a.is_object()) {
        let date = coverage::format_day(a["date"].as_str()).unwrap_or_default();
        let mut line = tr(
            if a["kind"] == "closed_on" {
                "connections:coverage.card.attestedClosed"
            } else {
                "connections:coverage.card.attestedNoActivity"
            },
            &[("date", &date)],
        );
        if let Some(note) = a["note"].as_str().filter(|n| !n.is_empty()) {
            line.push_str(" — ");
            line.push_str(note);
        }
        out.push((line, false));
    }
    if let Some(gap) = c.get("attestation_gap").filter(|g| g.is_object()) {
        let d = |k: &str| coverage::format_day(gap[k].as_str()).unwrap_or_default();
        out.push((
            tr(
                "connections:coverage.card.attestationGap",
                &[("from", &d("from")), ("through", &d("through"))],
            ),
            true,
        ));
    }
    for w in c["scan_warnings"].as_array().into_iter().flatten() {
        if let Some(code) = w.as_str() {
            out.push((scan_warning(code), false));
        }
    }
    out
}

/// The statement request.
pub fn attest_args(wallet: &str, f: &AttestForm) -> Value {
    json!({
        "wallet": wallet,
        "kind": f.kind,
        "date": f.date.text.trim(),
        "note": f.note.text.trim(),
    })
}

/// `scanWarningText`.
pub fn scan_warning(code: &str) -> String {
    let key = format!("connections:coverage.scanWarnings.{code}");
    if has(&key) {
        t(&key)
    } else {
        tr("connections:coverage.scanWarnings.other", &[("code", code)])
    }
}

/// `importResultText`: the import a user just ran, judged from coverage before and after it.
pub fn import_result(before: &Value, after: &Value) -> String {
    let n = |v: &Value, k: &str| v["file"][k].as_u64();
    let new_run = n(after, "import_count").unwrap_or(0) > n(before, "import_count").unwrap_or(0);
    let added = if new_run {
        n(after, "last_run_rows_inserted").unwrap_or(0)
    } else {
        0
    };
    if added == 0 {
        return t("connections:coverage.card.importNothingNew");
    }
    let present = n(after, "last_run_rows_read").map(|r| r.saturating_sub(added));
    let imported = crate::format::count(added as usize);
    match (
        present,
        coverage::format_day(after["covered_through"].as_str()),
    ) {
        (Some(p), Some(date)) => tr(
            "connections:coverage.card.importResult",
            &[
                ("imported", &imported),
                ("present", &crate::format::count(p as usize)),
                ("date", &date),
            ],
        ),
        (p, _) => tr(
            "connections:coverage.card.importResultNoDate",
            &[
                ("imported", &imported),
                ("present", &crate::format::count(p.unwrap_or(0) as usize)),
            ],
        ),
    }
}

// ---- Reads and writes ------------------------------------------------------------------------

/// The card's read for `wallet` (another wallet starts the card over).
pub fn read(app: &mut App, wallet: &str) {
    if app.wallets.sources.wallet != wallet {
        app.wallets.sources = State {
            wallet: wallet.to_owned(),
            ..State::default()
        };
    }
    app.wallets.sources.generation = app.book_generation;
    app.send(
        "ui.sources.coverage",
        json!({"wallet": wallet}),
        "wallets.sources.read",
    );
}

/// Every frame on a wallet's page: read when the card has none for it, and after each book
/// refresh cycle (an import, a sync or a statement re-reads the book).
pub fn poll(app: &mut App, c: &Conn) -> bool {
    let st = &app.wallets.sources;
    if st.wallet != c.id || st.generation != app.book_generation {
        read(app, &c.id);
        return true;
    }
    false
}

fn check_file(a: &mut App) {
    let wallet = a.wallets.sources.wallet.clone();
    if a.wallets.sources.checking || wallet.is_empty() {
        return;
    }
    a.wallets.sources.checking = true;
    a.wallets.sources.file_check = None;
    a.send(
        "ui.sources.coverage",
        json!({"wallet": wallet, "check_file": true}),
        "wallets.sources.check",
    );
}

/// Import newer export: the history import into this wallet, on its parser when known.
fn import(a: &mut App, c: &Conn) {
    if let Some(cov) = coverage_of(&a.wallets.sources, &c.id).cloned() {
        a.wallets.sources.before = Some((cov, a.wallets.imports_done));
    }
    a.wallets.sources.import_result = None;
    crate::wallets::add::open_history_on(a, c.id.clone(), file_source(c));
}

fn open_attest(a: &mut App) {
    let wallet = a.wallets.sources.wallet.clone();
    let cov = coverage_of(&a.wallets.sources, &wallet).cloned();
    let current = cov
        .as_ref()
        .and_then(|c| c.get("attestation"))
        .filter(|x| x.is_object());
    // A statement closes the gap only from the covered end, so that day is the default.
    let date = current
        .and_then(|x| x["date"].as_str())
        .map(str::to_owned)
        .or_else(|| coverage::day(cov.as_ref().and_then(|c| c["covered_through"].as_str())))
        .unwrap_or_default();
    a.wallets.sources.attest = Some(AttestForm {
        kind: if current.is_some_and(|x| x["kind"] == "closed_on") {
            "closed_on"
        } else {
            "no_activity_after"
        },
        date: TextEdit::new(&date),
        note: TextEdit::new(current.and_then(|x| x["note"].as_str()).unwrap_or("")),
        error: None,
        pending: false,
    });
}

fn close_attest(a: &mut App) {
    a.wallets.sources.attest = None;
}

fn close_clear(a: &mut App) {
    a.wallets.sources.clear = None;
}

fn submit_attest(a: &mut App) {
    let wallet = a.wallets.sources.wallet.clone();
    let Some(f) = a.wallets.sources.attest.as_mut() else {
        return;
    };
    if f.pending || f.date.text.trim().is_empty() {
        return;
    }
    f.pending = true;
    f.error = None;
    let args = attest_args(&wallet, f);
    a.send("ui.sources.attest_coverage", args, "wallets.sources.attest");
}

fn submit_clear(a: &mut App) {
    let wallet = a.wallets.sources.wallet.clone();
    let Some(f) = a.wallets.sources.clear.as_mut() else {
        return;
    };
    if f.pending {
        return;
    }
    f.pending = true;
    f.error = None;
    a.send(
        "ui.sources.attest_coverage",
        json!({"wallet": wallet, "clear": true}),
        "wallets.sources.clear",
    );
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "wallets.sources.read" => {
            let st = &mut app.wallets.sources;
            st.read = match &r.error {
                Some(e) => Read::Failed(e.clone()),
                None => Read::Ready(r.data.clone()),
            };
            // The first read after a successful import judges it against the press.
            let imports = app.wallets.imports_done;
            let st = &mut app.wallets.sources;
            if let Some((before, at)) = st.before.clone()
                && imports > at
                && let Some(after) = coverage_of(st, &st.wallet).cloned()
            {
                st.import_result = Some(import_result(&before, &after));
                st.before = None;
            }
        }
        "wallets.sources.check" => {
            let st = &mut app.wallets.sources;
            st.checking = false;
            st.file_check = Some(
                r.error
                    .is_none()
                    .then(|| r.data["sources"][0]["file"]["source_file_check"].as_str())
                    .flatten()
                    .unwrap_or("unknown")
                    .to_owned(),
            );
        }
        "wallets.sources.attest" | "wallets.sources.clear" => match &r.error {
            Some(e) => {
                let e = if e.trim().is_empty() {
                    t("connections:coverage.attestDialog.failed")
                } else {
                    e.clone()
                };
                let st = &mut app.wallets.sources;
                if let Some(f) = st.attest.as_mut().filter(|_| tag.ends_with("attest")) {
                    f.pending = false;
                    f.error = Some(e);
                } else if let Some(f) = st.clear.as_mut() {
                    f.pending = false;
                    f.error = Some(e);
                }
            }
            None => {
                app.wallets.sources.attest = None;
                app.wallets.sources.clear = None;
                // Coverage and the status derived from it change: the card and the shell read
                // again with the refresh cycle.
                app.refresh_reads();
            }
        },
        _ => {}
    }
}

/// Escape closes the innermost dialog.
pub fn on_key(app: &mut App, ev: &KeyEvent) -> bool {
    if !matches!(ev.key, Key::Escape) {
        return false;
    }
    if app.wallets.sources.clear.is_some() {
        close_clear(app);
        return true;
    }
    if app.wallets.sources.attest.is_some() {
        close_attest(app);
        return true;
    }
    false
}

// ---- View ------------------------------------------------------------------------------------

fn small(tk: &Tk, s: String, color: Color) -> El {
    text(s).font(tk.txs(400.0)).color(color).wrap().w_full()
}

/// `SourceCoverageView`, above the exchange sync card and the notices. Nothing until the read
/// answered with this wallet.
pub fn card(app: &App, tk: &Tk, c: &Conn) -> Option<El> {
    let st = &app.wallets.sources;
    if st.wallet != c.id {
        return None;
    }
    let cov = coverage_of(st, &c.id)?;
    let mode = coverage::mode(cov);
    let live = matches!(mode, "live" | "hybrid");
    let file = matches!(mode, "file" | "hybrid");
    let state = coverage::state(cov);
    let (_, ink) = super::list::coverage_glyph(tk, coverage::tone(cov));
    let mode_key = format!("connections:coverage.modeLong.{mode}");
    let state_key = format!("connections:coverage.state.{state}");
    let head = row()
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.25))
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .flex_wrap()
                        .child(
                            text(t("connections:coverage.card.title"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg)
                                .role("heading")
                                .level(2),
                        )
                        .child(
                            text(if has(&mode_key) {
                                t(&mode_key)
                            } else {
                                mode.to_owned()
                            })
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                        ),
                )
                .child(small(tk, window_text(cov), tk.muted_fg)),
        )
        .child(
            text(if has(&state_key) {
                t(&state_key)
            } else {
                state.to_owned()
            })
            .font(tk.txs(500.0))
            .color(ink),
        );
    let mut body = col().w_full().gap(tk.r(0.75)).child(head);
    let lines = facts(cov, crate::format::now_secs());
    if !lines.is_empty() {
        let (_, _, warn_ink) = amber(tk);
        let mut list = col().w_full().gap(tk.r(0.25)).role("list");
        for (line, warn) in lines {
            list = list
                .child(small(tk, line, if warn { warn_ink } else { tk.muted_fg }).role("listitem"));
        }
        body = body.child(list);
    }
    if cov["file"]["source_file_changed"] == true {
        // Metadata only (size, modification time): advisory, not proof.
        let (fill, _, ink) = amber(tk);
        body = body.child(
            col()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .fill(fill)
                .child(small(tk, t("connections:coverage.card.fileChanged"), ink)),
        );
    }
    if cov["file"]["source_file_remembered"] == true {
        let mut check = button(
            tk,
            "wd-coverage-check",
            Variant::Outline,
            Size::Xs,
            None,
            &t("connections:coverage.card.checkFile"),
            !st.checking,
        )
        .busy(st.checking);
        if !st.checking {
            check = check.on_press(check_file);
        }
        let mut r = row().gap(tk.r(0.5)).center_y().flex_wrap().child(check);
        if let Some(result) = &st.file_check {
            let key = format!("connections:coverage.card.fileCheck.{result}");
            let key = if has(&key) {
                key
            } else {
                "connections:coverage.card.fileCheck.unknown".to_owned()
            };
            r = r.child(
                text(t(&key))
                    .font(tk.txs(500.0))
                    .color(tk.fg)
                    .wrap()
                    .role("status"),
            );
        }
        body = body.child(r);
    }
    if let Some(result) = &st.import_result {
        body = body.child(
            text(result.clone())
                .font(tk.txs(500.0))
                .color(tk.fg)
                .wrap()
                .w_full()
                .role("status"),
        );
    }
    let action = cov["action"].as_str().unwrap_or("");
    let mut actions = row().gap(tk.r(0.5)).center_y().flex_wrap();
    if live {
        let syncing = super::detail::sync_running(app);
        let mut b = button(
            tk,
            "wd-coverage-sync",
            if action == "sync" {
                Variant::Default
            } else {
                Variant::Outline
            },
            Size::Sm,
            Some(&I::REFRESH_CW),
            &t("connections:coverage.card.syncNow"),
            !syncing,
        )
        .busy(syncing);
        if !syncing {
            // The page's own Refresh (network egress, only ever from this press).
            b = b.on_press(|a: &mut App| super::detail::sync_now(a));
        }
        actions = actions.child(b);
    }
    if file {
        let primary = action == "import_newer_file" || cov["file"]["source_file_changed"] == true;
        let conn = c.clone();
        actions = actions.child(
            button(
                tk,
                "wd-coverage-import",
                if primary {
                    Variant::Default
                } else {
                    Variant::Outline
                },
                Size::Sm,
                Some(&I::FILE_UP),
                &t(if mode == "hybrid" {
                    "connections:coverage.card.importOlder"
                } else {
                    "connections:coverage.card.importNewer"
                }),
                true,
            )
            .on_press(move |a: &mut App| import(a, &conn)),
        );
    }
    let attested = cov.get("attestation").is_some_and(Value::is_object);
    if mode != "manual" {
        actions = actions.child(
            button(
                tk,
                "wd-coverage-attest",
                Variant::Ghost,
                Size::Sm,
                Some(&I::CALENDAR_DAYS),
                &t(if attested {
                    "connections:coverage.card.attestEdit"
                } else {
                    "connections:coverage.card.attest"
                }),
                true,
            )
            .on_press(open_attest),
        );
    }
    if attested {
        actions = actions.child(
            button(
                tk,
                "wd-coverage-clear",
                Variant::Ghost,
                Size::Sm,
                None,
                &t("connections:coverage.card.attestClear"),
                true,
            )
            .on_press(|a: &mut App| a.wallets.sources.clear = Some(ClearForm::default())),
        );
    }
    body = body.child(actions);
    let src = file_source(c);
    if file && let Some(key) = refresh_hint_key(src, file) {
        let mut hint = row()
            .w_full()
            .gap(tk.r(0.25))
            .flex_wrap()
            .child(
                text(format!("{}:", t("connections:coverage.card.howToRefresh")))
                    .font(tk.t2xs(500.0))
                    .color(tk.muted_fg),
            )
            .child(text(t(&key)).font(tk.t2xs(400.0)).color(tk.muted_fg).wrap());
        if let Some((href, title)) = src.and_then(|s| Some((s.docs_href?, s.title))) {
            let label = tr(
                "connections:coverage.card.providerGuide",
                &[("provider", title)],
            );
            hint = hint.child(
                row()
                    .key("wd-coverage-guide")
                    .role("link")
                    .label(label.clone())
                    .focusable(true)
                    .on_press_tx(move |_a: &mut App, tx: &mut Tx| {
                        let _ = tx.open_url(href);
                    })
                    .child(text(label).font(tk.t2xs(400.0)).color(tk.fg)),
            );
        }
        body = body.child(hint);
    }
    Some(
        dui::card(tk)
            .key("wd-coverage")
            .role("region")
            .label(t("connections:coverage.card.title"))
            .child(col().w_full().padding(tk.r(1.0)).child(body)),
    )
}

fn radio(tk: &Tk, key: &str, label: String, checked: bool, pick: &'static str) -> El {
    let mut r = row()
        .key(key.to_owned())
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .role("radio")
        .checked(checked)
        .label(label.clone())
        .focusable(true)
        .on_press(move |a: &mut App| {
            if let Some(f) = a.wallets.sources.attest.as_mut() {
                f.kind = pick;
            }
        })
        .child(
            row()
                .size(tk.r(1.0), tk.r(1.0))
                .center()
                .radius(tk.r(0.5))
                .border(1.0, if checked { tk.primary } else { tk.input })
                .border_layout()
                .child(if checked {
                    row()
                        .size(tk.r(0.5), tk.r(0.5))
                        .radius(tk.r(0.25))
                        .fill(tk.primary)
                } else {
                    row()
                }),
        )
        .child(text(label).font(tk.tsm(400.0)).color(tk.fg));
    if !checked {
        r = r.hover(tk.muted.alpha(0.4));
    }
    r
}

/// The statement and clear dialogs.
pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let st = &app.wallets.sources;
    let wallet_label = coverage_of(st, &st.wallet)
        .and_then(|c| c["wallet"].as_str())
        .unwrap_or("")
        .to_owned();
    if let Some(f) = &st.attest {
        fn form(a: &mut App) -> &mut AttestForm {
            a.wallets.sources.attest.get_or_insert_with(|| AttestForm {
                kind: "no_activity_after",
                date: TextEdit::new(""),
                note: TextEdit::new(""),
                error: None,
                pending: false,
            })
        }
        let date_label = t("connections:coverage.attestDialog.date");
        let note_label = t("connections:coverage.attestDialog.note");
        let mut items = vec![
            dui::dialog_header(
                tk,
                &t("connections:coverage.attestDialog.title"),
                &tr(
                    "connections:coverage.attestDialog.body",
                    &[("wallet", &wallet_label)],
                ),
            ),
            dui::group(
                tk,
                vec![
                    dui::label(tk, &t("connections:coverage.attestDialog.kind")),
                    col()
                        .w_full()
                        .gap(tk.r(0.375))
                        .role("radiogroup")
                        .label(t("connections:coverage.attestDialog.kind"))
                        .child(radio(
                            tk,
                            "wd-attest-kind-none",
                            t("connections:coverage.attestDialog.kindNoActivity"),
                            f.kind == "no_activity_after",
                            "no_activity_after",
                        ))
                        .child(radio(
                            tk,
                            "wd-attest-kind-closed",
                            t("connections:coverage.attestDialog.kindClosed"),
                            f.kind == "closed_on",
                            "closed_on",
                        )),
                ],
            ),
            dui::group(
                tk,
                vec![
                    dui::label(tk, &date_label),
                    dui::input_submit(
                        tk,
                        "wd-attest-date",
                        &f.date,
                        &date_label,
                        "YYYY-MM-DD",
                        |a| &mut form(a).date,
                        false,
                        submit_attest,
                    ),
                ],
            ),
            dui::group(
                tk,
                vec![
                    dui::label(tk, &note_label),
                    dui::input_submit(
                        tk,
                        "wd-attest-note",
                        &f.note,
                        &note_label,
                        &t("connections:coverage.attestDialog.notePlaceholder"),
                        |a| &mut form(a).note,
                        false,
                        submit_attest,
                    ),
                ],
            ),
        ];
        if let Some(e) = &f.error {
            items.push(dui::error_line(tk, e));
        }
        let ok = !f.pending && !f.date.text.trim().is_empty();
        let mut confirm = dui::btn(
            tk,
            "wd-attest-confirm",
            Variant::Default,
            Size::Default,
            None,
            &t("connections:coverage.attestDialog.confirm"),
            ok,
        );
        if ok {
            confirm = confirm.on_press(submit_attest);
        }
        items.push(dui::dialog_footer(
            tk,
            vec![
                dui::btn(
                    tk,
                    "wd-attest-cancel",
                    Variant::Outline,
                    Size::Default,
                    None,
                    &t("connections:coverage.attestDialog.cancel"),
                    true,
                )
                .on_press(close_attest),
                confirm,
            ],
        ));
        dui::dialog(
            app,
            tk,
            layer,
            "wd-attest",
            tk.r(32.0),
            &t("connections:coverage.attestDialog.title"),
            col().w_full().gap(tk.r(1.0)).children(items),
            close_attest,
        );
    }
    if let Some(f) = &st.clear {
        let mut items = vec![dui::dialog_header(
            tk,
            &t("connections:coverage.attestDialog.clearTitle"),
            &tr(
                "connections:coverage.attestDialog.clearBody",
                &[("wallet", &wallet_label)],
            ),
        )];
        if let Some(e) = &f.error {
            items.push(dui::error_line(tk, e));
        }
        let mut confirm = dui::destructive_button(
            tk,
            "wd-attest-clear-confirm",
            &t("connections:coverage.attestDialog.clearConfirm"),
            !f.pending,
        );
        if !f.pending {
            confirm = confirm.on_press(submit_clear);
        }
        items.push(dui::dialog_footer(
            tk,
            vec![
                dui::btn(
                    tk,
                    "wd-attest-clear-cancel",
                    Variant::Outline,
                    Size::Default,
                    None,
                    &t("connections:coverage.attestDialog.cancel"),
                    true,
                )
                .on_press(close_clear),
                confirm,
            ],
        ));
        dui::dialog(
            app,
            tk,
            layer,
            "wd-attest-clear",
            tk.r(28.0),
            &t("connections:coverage.attestDialog.clearTitle"),
            col().w_full().gap(tk.r(1.0)).children(items),
            close_clear,
        );
    }
}

// ---- Scenes ----------------------------------------------------------------------------------

/// SYNTHETIC: a file source behind, with a statement gap, a remembered file that changed and
/// a scan warning, on the open wallet's card (nothing is written to the book).
pub fn scene_sample(a: &mut App, attest: bool) {
    let id = a.wallets.detail.wallet.clone();
    let label = crate::wallets::conn::find(a, &id)
        .map(|c| c.label)
        .unwrap_or_default();
    let cov = json!({
        "wallet_id": id, "wallet": label, "kind": "river", "completeness_class": "file",
        "source_mode": "file", "covered_from": "2024-01-03", "covered_through": "2026-06-30",
        "freshness_state": "behind", "reason": "file_ends_before_period_end",
        "action": "import_newer_file", "needs_action": true,
        "live": {"last_sync_at": null, "last_sync_status": null, "last_sync_error_code": null,
                 "api_history_limit": null},
        "file": {"source_format": "river_csv", "import_count": 2,
                 "last_import_at": "2026-07-02T09:00:00Z", "first_row_at": "2024-01-03",
                 "last_row_at": "2026-06-30", "last_run_rows_read": 40,
                 "last_run_rows_inserted": 12, "source_file_changed": true,
                 "source_file_check": null, "source_file_remembered": true},
        "transactions": {"count": 52, "first_at": "2024-01-03", "last_at": "2026-06-30"},
        "attestation": {"kind": "no_activity_after", "date": "2026-09-30",
                        "note": "Moved everything to cold storage",
                        "attested_at": "2026-10-01T08:00:00Z"},
        "attestation_gap": {"from": "2026-06-30", "through": "2026-09-30"},
        "scan_warnings": ["undated_transactions"],
    });
    let st = &mut a.wallets.sources;
    st.wallet = id;
    st.generation = a.book_generation;
    st.read = Read::Ready(json!({"sources": [cov]}));
    st.file_check = Some("different".into());
    if attest {
        open_attest(a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cov(v: Value) -> Value {
        let mut base = json!({
            "wallet_id": "w1", "wallet": "River", "source_mode": "file",
            "freshness_state": "current", "reason": "", "action": "none", "needs_action": false,
            "live": {}, "file": {"import_count": 0}, "scan_warnings": [],
        });
        for (k, x) in v.as_object().unwrap() {
            base[k] = x.clone();
        }
        base
    }

    #[test]
    fn window_and_facts_follow_the_mode() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let c = cov(json!({"covered_from": "2024-01-03", "covered_through": "2026-06-30"}));
        assert_eq!(window_text(&c), "Covers Jan 3, 2024 – Jun 30, 2026");
        assert_eq!(window_text(&cov(json!({}))), "No dated history yet");
        let lines: Vec<String> = facts(&c, 0.0).into_iter().map(|(l, _)| l).collect();
        assert_eq!(lines, [t("connections:coverage.card.fileNote")]);
        let hybrid = cov(json!({
            "source_mode": "hybrid",
            "live": {"api_history_limit": "2025-01-01", "last_sync_at": null},
            "file": {"import_count": 2, "last_import_at": "2026-07-02T09:00:00Z"},
            "attestation_gap": {"from": "2026-06-30", "through": "2026-09-30"},
            "scan_warnings": ["never_synced", "made_up"],
        }));
        let f = facts(&hybrid, 0.0);
        assert_eq!(f[0].0, "2 imports, last Jul 2, 2026");
        assert!(f[1].0.contains("before Jan 1, 2025"), "{}", f[1].0);
        assert!(f[2].1, "the statement gap is a warning");
        assert_eq!(f[3].0, "Never synced yet.");
        assert_eq!(f[4].0, "Coverage warning: made_up");
    }

    #[test]
    fn an_import_is_judged_against_the_coverage_before_it() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let before = cov(json!({"file": {"import_count": 1}}));
        let after = cov(json!({
            "covered_through": "2026-09-30",
            "file": {"import_count": 2, "last_run_rows_read": 40, "last_run_rows_inserted": 12},
        }));
        assert_eq!(
            import_result(&before, &after),
            "12 new rows, 28 already present, now covers through Sep 30, 2026."
        );
        assert_eq!(
            import_result(&before, &before),
            "Nothing new — every row in this file is already present."
        );
        assert_eq!(scan_warning("never_synced"), "Never synced yet.");
        assert_eq!(scan_warning("odd"), "Coverage warning: odd");
    }

    #[test]
    fn attestation_requests_carry_the_trimmed_fields() {
        let f = AttestForm {
            kind: "closed_on",
            date: TextEdit::new(" 2026-09-30 "),
            note: TextEdit::new(" moved "),
            error: None,
            pending: false,
        };
        assert_eq!(
            attest_args("w1", &f),
            json!({"wallet": "w1", "kind": "closed_on", "date": "2026-09-30", "note": "moved"})
        );
    }

    #[test]
    fn refresh_hints_name_the_provider_or_fall_back() {
        let river = crate::overview::catalog::SOURCES
            .iter()
            .find(|s| s.id == "river");
        assert_eq!(
            refresh_hint_key(river, true).as_deref(),
            Some("connections:coverage.refreshHints.river")
        );
        assert_eq!(
            refresh_hint_key(None, true).as_deref(),
            Some("connections:coverage.refreshHints.generic")
        );
        assert_eq!(refresh_hint_key(None, false), None);
    }

    #[test]
    fn the_file_source_follows_the_stored_format_then_the_kind() {
        let c = Conn {
            kind: "river".into(),
            source_format: Some("river_csv".into()),
            ..Conn::default()
        };
        assert_eq!(file_source(&c).map(|s| s.id), Some("river"));
        let generic = Conn {
            kind: "custom".into(),
            source_format: Some("strike_csv".into()),
            ..Conn::default()
        };
        assert_eq!(file_source(&generic).map(|s| s.id), Some("strike"));
        let none = Conn {
            kind: "descriptor".into(),
            ..Conn::default()
        };
        assert!(file_source(&none).is_none());
    }
}
