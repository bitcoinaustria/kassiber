//! Reconcile's tools mode (`components/reconcile/ToolCrossCheck.tsx`): a CoinTracking or
//! Blockpit export checked against the book, wallet by wallet, the tool and Kassiber side by
//! side. `ui.reports.cross_check` reads the file's bytes locally; nothing is imported.

use std::collections::{HashMap, VecDeque};

use rune::{Color, Cx, El, TextEdit, Tx, col, el, row, text};
use rune_kit::{Placement, TextInput};
use serde_json::{Value, json};

use super::kit::{self, Var};
use super::model::{num, s};
use super::{Popup, toggle};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, surface};

const PAGE: usize = 150;
const ALL: &str = "all";
const TOOLS: [&str; 2] = ["cointracking", "blockpit"];

#[derive(Default)]
pub struct State {
    /// The chosen export: its name and bytes (base64), never a path.
    pub file: Option<(String, String)>,
    pub tool: Option<&'static str>,
    pub file_error: Option<String>,
    pub year: Option<i64>,
    pub pair_key: String,
    pub view: &'static str,
    pub query: TextEdit,
    pub limit: usize,
    /// Comparisons by their args (`staleTime: Infinity`).
    cache: HashMap<String, Value>,
    errors: HashMap<String, String>,
    /// The last result shown, kept while another year's (or a refreshed book's) loads
    /// (`keepPreviousData`).
    shown: Option<Value>,
    /// The comparisons in flight: their args and the book refresh cycle they read.
    queue: VecDeque<(String, u64)>,
    /// The book refresh cycle (`app.book_generation`) the cache belongs to: a refresh (an
    /// exclusion, a journal run, a sync) changes what the comparison reads, so it reads again.
    generation: u64,
    pub years: Vec<i64>,
    pub exporting: bool,
    /// An exported workbook waiting for the save dialog.
    save: Option<String>,
}

/// Another book: the tools start over. Comparisons still out stay listed under a refresh cycle
/// no read has, so each answer pops its own entry and is dropped.
pub fn restart(app: &mut App) {
    let old = std::mem::take(&mut app.review.tools);
    st(app).queue = old.queue.into_iter().map(|(k, _)| (k, u64::MAX)).collect();
}

fn st(app: &mut App) -> &mut State {
    &mut app.review.tools
}

fn tool_name(id: &str) -> &'static str {
    match id {
        "cointracking" => "CoinTracking",
        _ => "Blockpit",
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `detectCrossCheckTool`: which tool wrote this CSV, from its header row.
fn detect(bytes: &[u8]) -> Option<&'static str> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(6144)]).into_owned();
    let head = head.trim_start_matches('\u{feff}');
    let first = head
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_lowercase();
    if ["integration name", "outgoing asset", "incoming asset"]
        .iter()
        .all(|c| first.contains(c))
    {
        return Some("blockpit");
    }
    if ["buy currency", "sell currency"]
        .iter()
        .all(|c| first.contains(c))
    {
        return Some("cointracking");
    }
    None
}

fn args(s: &State) -> Option<Value> {
    let (name, b64) = s.file.as_ref()?;
    let tool = s.tool?;
    let mut a = json!({ "tool": tool, "filename": name, "source_bytes_base64": b64 });
    if let Some(y) = s.year {
        a["year"] = json!(y);
    }
    Some(a)
}

fn key_of(a: &Value) -> String {
    format!("{}|{}|{}", a["tool"], a["filename"], a["year"])
        + &a["source_bytes_base64"]
            .as_str()
            .map(|b| b.len().to_string())
            .unwrap_or_default()
}

fn take_file(app: &mut App, name: String, bytes: Vec<u8>) {
    let s = st(app);
    s.file_error = None;
    s.tool = detect(&bytes);
    s.file = Some((name, base64(&bytes)));
    s.pair_key = ALL.into();
    s.view = "attention";
    s.cache.clear();
    s.errors.clear();
    s.shown = None;
    s.limit = PAGE;
}

fn choose_file(app: &mut App, tx: &mut Tx) {
    let _ = app;
    let options = rune::FileDialogOptions {
        title: t("review:reconcile.tools.chooseFile"),
        ..Default::default()
    }
    .filter("CSV", &["csv"]);
    tx.open_file(options, |a: &mut App, result, _tx| match result {
        Ok(Some(path)) => match std::fs::read(&path) {
            Ok(bytes) => take_file(
                a,
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                bytes,
            ),
            Err(e) => st(a).file_error = Some(e.to_string()),
        },
        Ok(None) => {}
        Err(e) => st(a).file_error = Some(e.to_string()),
    });
}

/// Reads the comparison its args name, once per args (local; no network).
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    if let Some(path) = st(app).save.take() {
        // `saveDaemonExport`: the owner picks where the workbook goes.
        let base = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("kassiber-cross-check.xlsx")
            .to_owned();
        let options = rune::FileDialogOptions {
            title: t("review:reconcile.tools.saveTitle"),
            file_name: Some(base.clone()),
            ..Default::default()
        }
        .filter("Excel", &["xlsx"]);
        tx.save_file(options, move |a: &mut App, result, _tx| {
            let (title, body, written) = match result {
                Ok(Some(dest)) => match std::fs::copy(&path, &dest) {
                    Ok(_) => (
                        t("review:reconcile.tools.saved"),
                        tr(
                            "review:reconcile.tools.savedTo",
                            &[(
                                "name",
                                &dest
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                            )],
                        ),
                        dest.to_string_lossy().into_owned(),
                    ),
                    Err(e) => {
                        a.toast(
                            ToastTone::Error,
                            &t("review:reconcile.tools.failedExport"),
                            Some(e.to_string()),
                            Some("rc-export"),
                        );
                        return;
                    }
                },
                _ => (
                    t("review:reconcile.tools.saved"),
                    base.clone(),
                    path.clone(),
                ),
            };
            // `toastopen`: Open / Show in Finder for the workbook just written.
            crate::toastopen::saved_toast(
                a,
                ToastTone::Success,
                &title,
                Some(body),
                Some("rc-export"),
                &written,
                false,
            );
        });
        changed = true;
    }
    if app.route.path() != "/reconcile" {
        return changed;
    }
    let Some(a) = args(&app.review.tools) else {
        return changed;
    };
    let k = key_of(&a);
    let generation = app.book_generation;
    if app.review.tools.generation != generation {
        let s = st(app);
        s.generation = generation;
        s.cache.clear();
        s.errors.clear();
        changed = true;
    }
    let s = &app.review.tools;
    if !s.cache.contains_key(&k)
        && !s.errors.contains_key(&k)
        && !s.queue.contains(&(k.clone(), generation))
        && app
            .send("ui.reports.cross_check", a, "review.tools.compare")
            .is_some()
    {
        st(app).queue.push_back((k, generation));
        changed = true;
    }
    changed
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "review.tools.compare" => {
            let Some((k, generation)) = st(app).queue.pop_front() else {
                return;
            };
            if generation != app.book_generation {
                // Read before a book refresh: the next poll compares again.
                return;
            }
            let s = st(app);
            match &r.error {
                Some(e) => {
                    s.errors.insert(k, e.clone());
                }
                None => {
                    if let Some(ys) = r.data["years"].as_array() {
                        s.years = ys.iter().filter_map(Value::as_i64).collect();
                    }
                    s.cache.insert(k, r.data.clone());
                    s.shown = Some(r.data.clone());
                    s.limit = PAGE;
                }
            }
        }
        "review.tools.export" => {
            st(app).exporting = false;
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Error,
                    &t("review:reconcile.tools.failedExport"),
                    Some(e.clone()),
                    Some("rc-export"),
                ),
                None => st(app).save = Some(s(&r.data, "file")),
            }
        }
        _ => {}
    }
}

fn run_export(app: &mut App) {
    let Some(a) = args(&app.review.tools) else {
        return;
    };
    if app
        .send("ui.reports.export_cross_check", a, "review.tools.export")
        .is_some()
    {
        st(app).exporting = true;
    }
}

/// `ExportFormatMark`: the tool's logo in a white bordered tile, its artwork (not its canvas)
/// filling the padded box.
fn mark(app: &App, cx: &Cx<App>, tk: &Tk, tool: &str, side: f32, pad: f32) -> El {
    let inner = side - 2.0 * pad - 2.0;
    let logo = if tool == "cointracking" {
        crate::logos::Logo::svg(crate::logos::COINTRACKING)
    } else {
        crate::logos::Logo::raster(crate::logos::BLOCKPIT)
    };
    row()
        .size(side, side)
        .padding(pad)
        .center()
        .radius(tk.r(0.375))
        .fill(Color::WHITE)
        .border(1.0, tk.border)
        .clip_rounded(tk.r(0.375))
        .a11y_hidden()
        .child(crate::logos::fitted(app, cx, logo, inner, inner))
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.review.tools;
    let mut sec = surface(tk)
        .key("rc-tools")
        .w_full()
        .min_h(tk.r(32.0))
        .role("region")
        .label(t("review:reconcile.modes.tools"))
        .clip_rounded(tk.r(0.75));
    let Some((name, _)) = &s.file else {
        // The empty picker: Review's centred state on the page, no card around it.
        let mut logos = row().gap(tk.r(0.5)).center_y();
        for tool in TOOLS {
            logos = logos.child(mark(app, cx, tk, tool, tk.r(2.5), tk.r(0.375)));
        }
        logos = logos
            .child(ic(&I::ARROW_LEFT_RIGHT, tk.r(1.25), tk.muted_fg).mx(tk.r(0.25)))
            .child(
                row()
                    .size(tk.r(2.5), tk.r(2.5))
                    .center()
                    .radius(tk.r(0.375))
                    .fill(tk.primary)
                    .child(text("K").font(tk.tbase(600.0)).color(tk.primary_fg)),
            );
        let mut c = col()
            .w_full()
            .max_w(tk.r(32.0))
            .gap(tk.r(0.5))
            .items_center()
            .child(logos.a11y_hidden().mb(tk.r(0.75)))
            .child(
                text(t("review:reconcile.tools.empty.title"))
                    .font(tk.tbase(600.0))
                    .color(tk.fg)
                    .align(rune::Align::Center)
                    .role("heading")
                    .level(2),
            )
            .child(
                text(t("review:reconcile.tools.empty.body"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .align(rune::Align::Center),
            )
            .child(
                kit::btn(
                    tk,
                    "rc-tools-choose",
                    Var::Default,
                    Some(&I::FILE_UP),
                    &t("review:reconcile.tools.chooseFile"),
                    true,
                )
                .mt(tk.r(0.75))
                .on_press_tx(choose_file),
            );
        if let Some(e) = &s.file_error {
            c = c.child(
                text(e.clone())
                    .font(tk.txs(400.0))
                    .color(tk.destructive)
                    .role("alert"),
            );
        }
        let free = crate::reports::ledger::common::free_h(app, tk) - tk.r(3.0);
        return col()
            .key("rc-tools")
            .w_full()
            .min_h(free)
            .padding(tk.r(1.5))
            .center()
            .role("region")
            .label(t("review:reconcile.modes.tools"))
            .child(c);
    };
    let a = args(s);
    let k = a.as_ref().map(key_of);
    let result = k.as_ref().and_then(|k| s.cache.get(k)).or(s.shown.as_ref());
    let error = k.as_ref().and_then(|k| s.errors.get(k));
    let fetching = k
        .as_ref()
        .is_some_and(|k| s.queue.iter().any(|(q, _)| q == k));
    let tool_label = s.tool.map(tool_name).unwrap_or("");
    // The file bar.
    let mut file_btn = row()
        .key("rc-tools-file")
        .h(tk.r(2.0))
        .max_w(tk.r(30.0))
        .pl(tk.r(0.25))
        .pr(tk.r(0.625))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.background.alpha(0.7))
        .hover(tk.muted.alpha(0.6))
        .role("button")
        .label(format!("{name} {}", t("review:reconcile.tools.changeFile")))
        .focusable(true)
        .on_press_tx(choose_file);
    file_btn = match s.tool {
        Some(tool) => file_btn.child(mark(app, cx, tk, tool, tk.r(1.5), tk.r(0.125))),
        None => file_btn.child(ic(&I::FILE_UP, tk.r(1.0), tk.muted_fg).ml(tk.r(0.25))),
    };
    file_btn = file_btn
        .child(
            text(name.clone())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .truncate()
                .min_w(0.0),
        )
        .child(
            text(t("review:reconcile.tools.changeFile"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    let export_enabled = result.is_some() && !fetching && !s.exporting && error.is_none();
    let mut export = kit::btn(
        tk,
        "rc-tools-export",
        Var::Outline,
        Some(&I::DOWNLOAD),
        &if s.exporting {
            t("review:reconcile.tools.exporting")
        } else {
            t("review:reconcile.tools.export")
        },
        export_enabled,
    );
    if export_enabled {
        export = export.on_press(run_export);
    }
    let bar = row()
        .w_full()
        .px(tk.r(crate::ui::table::EDGE))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(file_btn)
        .child(el().grow(1.0))
        .child(year_picker(app, tk, layer))
        .child(export);
    sec = sec.child(bar);
    let Some(tool) = s.tool else {
        // `ToolQuestion`.
        let mut btns = row().gap(tk.r(0.5)).justify_center().flex_wrap();
        for option in TOOLS {
            btns = btns.child(
                crate::txdetail::widgets::button_surface(
                    tk,
                    row()
                        .key(format!("rc-tools-pick-{option}"))
                        .h(tk.r(2.25))
                        .px(tk.r(0.75))
                        .gap(tk.r(0.5))
                        .center_y(),
                    Var::Outline,
                )
                .role("button")
                .label(tool_name(option))
                .focusable(true)
                .on_press(move |a: &mut App| st(a).tool = Some(option))
                .child(mark(app, cx, tk, option, tk.r(1.25), 1.0))
                .child(text(tool_name(option)).font(tk.tsm(500.0)).color(tk.fg)),
            );
        }
        return sec.child(
            col()
                .w_full()
                .grow(1.0)
                .padding(tk.r(1.5))
                .gap(tk.r(0.75))
                .items_center()
                .justify_center()
                .child(
                    text(t("review:reconcile.tools.whichTool"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(btns),
        );
    };
    if let Some(e) = error {
        let mut c = col()
            .w_full()
            .grow(1.0)
            .padding(tk.r(1.5))
            .gap(tk.r(0.5))
            .items_center()
            .justify_center()
            .role("alert")
            .child(
                row()
                    .size(tk.r(3.0), tk.r(3.0))
                    .radius(tk.r(1.5))
                    .center()
                    .fill(tk.warn.solid.alpha(0.12))
                    .mb(tk.r(0.5))
                    .a11y_hidden()
                    .child(ic(&I::TRIANGLE_ALERT, tk.r(1.5), tk.warn.solid)),
            )
            .child(
                text(t("review:reconcile.tools.failed"))
                    .font(tk.tbase(600.0))
                    .color(tk.fg)
                    .role("heading")
                    .level(2),
            )
            .child(
                text(e.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .max_w(tk.r(32.0))
                    .wrap()
                    .align(rune::Align::Center)
                    .mb(tk.r(0.5)),
            );
        for option in TOOLS.iter().copied().filter(|o| *o != tool) {
            c = c.child(
                crate::txdetail::widgets::button_surface(
                    tk,
                    row()
                        .key(format!("rc-tools-readas-{option}"))
                        .h(tk.r(2.0))
                        .px(tk.r(0.625))
                        .gap(tk.r(0.375))
                        .center_y(),
                    Var::Outline,
                )
                .role("button")
                .label(tr(
                    "review:reconcile.tools.readAs",
                    &[("tool", tool_name(option))],
                ))
                .focusable(true)
                .on_press(move |a: &mut App| st(a).tool = Some(option))
                .child(mark(app, cx, tk, option, tk.r(1.25), 1.0))
                .child(
                    text(tr(
                        "review:reconcile.tools.readAs",
                        &[("tool", tool_name(option))],
                    ))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
                ),
            );
        }
        return sec.child(c);
    }
    let Some(result) = result else {
        return sec.child(
            col()
                .w_full()
                .padding(tk.r(1.25))
                .gap(tk.r(0.75))
                .busy(true)
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(kit::spinner(tk, tk.r(1.0)))
                        .child(
                            text(t("review:reconcile.tools.running"))
                                .font(tk.tsm(400.0))
                                .color(tk.muted_fg),
                        ),
                )
                .child(kit::skeleton(tk, None, tk.r(3.5)))
                .child(kit::skeleton(tk, None, tk.r(16.0))),
        );
    };
    sec.child(comparison(app, cx, tk, result, tool_label, fetching))
}

/// The year picker (`YearPicker allowAll`).
fn year_picker(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.review.tools;
    let years = &s.years;
    let value = s.year;
    let index = value.and_then(|v| years.iter().position(|y| *y == v));
    let older: Option<Option<i64>> = match value {
        None => years.first().map(|y| Some(*y)),
        Some(_) => index.and_then(|i| years.get(i + 1)).map(|y| Some(*y)),
    };
    let newer: Option<Option<i64>> = match index {
        Some(i) if i > 0 => Some(Some(years[i - 1])),
        Some(0) => Some(None),
        _ => None,
    };
    let all = t("transactions:dashboard.export.yearPicker.all");
    let label = value.map(|v| v.to_string()).unwrap_or_else(|| all.clone());
    let arrow = |key: &str,
                 glyph: &'static rune::Icon,
                 name: String,
                 target: Option<Option<i64>>,
                 left: bool| {
        let mut b = row()
            .key(key.to_owned())
            .size(tk.r(2.0) - 2.0, tk.r(2.0) - 2.0)
            .center()
            .border_sides(
                if left {
                    [0.0, 1.0, 0.0, 0.0]
                } else {
                    [0.0, 0.0, 0.0, 1.0]
                },
                tk.border,
            )
            .role("button")
            .label(name)
            .focusable(target.is_some())
            .disabled(target.is_none())
            .opacity(if target.is_some() { 1.0 } else { 0.35 })
            .child(ic(glyph, tk.r(1.0), tk.muted_fg));
        if let Some(y) = target {
            b = b
                .hover(tk.muted.alpha(0.6))
                .on_press(move |a: &mut App| st(a).year = y);
        }
        b
    };
    let open = app.review.popup == Some(Popup::Year);
    let center = row()
        .key("rc-tools-year")
        .h(tk.r(2.0) - 2.0)
        .min_w(tk.r(6.0))
        .px(tk.r(0.75))
        .gap(tk.r(0.375))
        .center()
        .hover(tk.muted.alpha(0.6))
        .role("button")
        .label(tr(
            "transactions:dashboard.export.yearPicker.label",
            &[("year", &label)],
        ))
        .expanded(open)
        .focusable(true)
        .on_press(|a: &mut App| toggle(a, Popup::Year))
        .child(
            text(label)
                .font(tk.tsm(500.0).features("'tnum' 1"))
                .color(tk.fg),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(0.875), tk.muted_fg));
    if open {
        let pick = |key: String, label: String, checked: bool, y: Option<i64>, full: bool| {
            let mut b = row()
                .key(key)
                .py(tk.r(0.375))
                .px(tk.r(0.5))
                .justify_center()
                .radius(tk.r(0.375))
                .role("radio")
                .checked(checked)
                .label(label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| {
                    st(a).year = y;
                    a.review.popup = None;
                });
            b = if full { b.w_full() } else { b.grow(1.0).w(0.0) };
            b = if checked {
                b.fill(tk.primary)
            } else {
                b.hover(tk.muted.alpha(0.6))
            };
            b.child(
                text(label)
                    .font(tk.tsm(400.0).features("'tnum' 1"))
                    .color(if checked { tk.primary_fg } else { tk.fg }),
            )
        };
        let mut body = col()
            .w(tk.r(15.0))
            .padding(tk.r(0.5))
            .gap(tk.r(0.25))
            .child(
                pick(
                    "rc-year-all".into(),
                    all.clone(),
                    value.is_none(),
                    None,
                    true,
                )
                .mb(tk.r(0.125)),
            );
        let mut grid = col()
            .w_full()
            .gap(tk.r(0.25))
            .role("radiogroup")
            .label(t("transactions:dashboard.export.yearPicker.title"));
        for chunk in years.chunks(4) {
            let mut r = row().w_full().gap(tk.r(0.25));
            for y in chunk {
                r = r.child(pick(
                    format!("rc-year-{y}"),
                    y.to_string(),
                    value == Some(*y),
                    Some(*y),
                    false,
                ));
            }
            for _ in chunk.len()..4 {
                r = r.child(el().grow(1.0).w(0.0));
            }
            grid = grid.child(r);
        }
        body = body.child(grid);
        let surface = crate::shell::glass(tk, body)
            .modal(true, |a: &mut App| a.review.popup = None)
            .focus_return("rc-tools-year".to_owned());
        layer.push(crate::shell::anchored(
            tk,
            surface,
            "rc-tools-year",
            Placement::Bottom,
            4.0,
        ));
    }
    row()
        .h(tk.r(2.0))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.background.alpha(0.7))
        .clip_rounded(tk.r(0.375))
        .child(arrow(
            "rc-year-older",
            &I::CHEVRON_LEFT,
            t("transactions:dashboard.export.yearPicker.older"),
            older,
            true,
        ))
        .child(center)
        .child(arrow(
            "rc-year-newer",
            &I::CHEVRON_RIGHT,
            t("transactions:dashboard.export.yearPicker.newer"),
            newer,
            false,
        ))
}

fn comparison(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    result: &Value,
    tool_label: &str,
    loading: bool,
) -> El {
    let s = &app.review.tools;
    let pairs: Vec<Value> = result["pairs"].as_array().cloned().unwrap_or_default();
    let pair_key = if s.pair_key.is_empty() {
        ALL
    } else {
        s.pair_key.as_str()
    };
    let selected = pairs.iter().find(|p| s_(p, "key") == pair_key);
    let state_of = |key: &str| {
        pairs
            .iter()
            .find(|p| s_(p, "key") == key)
            .map(|p| s_(p, "state"))
            .unwrap_or_default()
    };
    let rows: Vec<Value> = result["rows"].as_array().cloned().unwrap_or_default();
    let scoped: Vec<&Value> = rows
        .iter()
        .filter(|r| match selected {
            Some(p) => s_(r, "pair") == s_(p, "key"),
            None => state_of(&s_(r, "pair")) == "paired",
        })
        .collect();
    let attention: Vec<&Value> = scoped
        .iter()
        .copied()
        .filter(|r| s_(r, "status") != "matched")
        .collect();
    let matched = scoped.len() - attention.len();
    let needle = s.query.text.trim().to_lowercase();
    let view = if s.view.is_empty() {
        "attention"
    } else {
        s.view
    };
    let listed: Vec<&Value> = match view {
        "attention" => attention.clone(),
        "matched" => scoped
            .iter()
            .copied()
            .filter(|r| s_(r, "status") == "matched")
            .collect(),
        _ => scoped.clone(),
    }
    .into_iter()
    .filter(|r| row_matches(r, &needle))
    .collect();
    let width = cx.layout_size("rc-tools").map(|[w, _]| w).unwrap_or(1100.0);
    let wide = width >= tk.r(48.0);
    let wallets = wallet_list(app, tk, &pairs, pair_key, tool_label, wide);
    let mut top = col()
        .w_full()
        .px(tk.r(crate::ui::table::EDGE))
        .pt(tk.r(1.25))
        .pb(tk.r(0.75))
        .gap(tk.r(0.75))
        .child(headline(
            tk, result, selected, &attention, &pairs, tool_label, loading,
        ));
    // Views and search.
    // The views on the page's segmented track (Transfers, Journals, the Reconcile modes).
    let mut views = row()
        .h(tk.r(2.0))
        .padding(tk.r(0.125))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .role("radiogroup")
        .label(t("review:reconcile.tools.views.label"));
    for (key, count) in [
        ("attention", attention.len()),
        ("matched", matched),
        ("all", scoped.len()),
    ] {
        let on = view == key;
        let mut b = row()
            .key(format!("rc-view-{key}"))
            .h(tk.r(1.75))
            .px(tk.r(0.75))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .cursor(rune::Cursor::Pointer)
            .role("radio")
            .checked(on)
            .label(format!(
                "{} {}",
                t(&format!("review:reconcile.tools.views.{key}")),
                count
            ))
            .focusable(true)
            .on_press(move |a: &mut App| {
                st(a).view = key;
                st(a).limit = PAGE;
            });
        b = if on {
            b.fill(tk.card).box_shadow(rune::Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.08),
                ..Default::default()
            })
        } else {
            b.hover(tk.background.alpha(0.5))
        };
        let chip = row()
            .px(tk.r(0.375))
            .radius(999.0)
            .fill(if on { tk.muted } else { Color::TRANSPARENT })
            .a11y_hidden()
            .child(
                text(crate::format::count(count))
                    .font(tk.t2xs(400.0).features("'tnum' 1"))
                    .color(if on { tk.fg } else { tk.muted_fg }),
            );
        b = b
            .child(
                text(t(&format!("review:reconcile.tools.views.{key}")))
                    .font(tk.tsm(500.0))
                    .color(if on { tk.fg } else { tk.muted_fg }),
            )
            .child(chip);
        views = views.child(b);
    }
    let fk = crate::txdetail::widgets::form_kit(tk);
    let search = crate::txdetail::widgets::field(
        tk,
        TextInput::new(&s.query)
            .label(t("review:reconcile.tools.search"))
            .placeholder(t("review:reconcile.tools.search"))
            .height(tk.r(2.0))
            .width(None)
            .leading_icon(&I::SEARCH)
            .leading_icon_size(tk.r(0.875))
            .text_style(tk.tsm(400.0))
            .on_edit(|a: &mut App| &mut a.review.tools.query)
            .on_change(|a: &mut App, _tx: &mut Tx| a.review.tools.limit = PAGE)
            .build(&fk)
            .key("rc-tools-search"),
    );
    top = top.child(
        row()
            .w_full()
            .gap(tk.r(0.5))
            .center_y()
            .flex_wrap()
            .child(views)
            .child(row().grow(1.0).min_w(tk.r(10.0)).child(search)),
    );
    let mut main = col().min_w(0.0).grow(1.0).w(0.0).child(top);
    let limit = if s.limit == 0 { PAGE } else { s.limit };
    let empty = if view == "attention" && needle.is_empty() {
        t("review:reconcile.tools.nothingToReview")
    } else {
        t("review:reconcile.tools.noRows")
    };
    main = main.child(row_list(
        app,
        cx,
        tk,
        &listed[..listed.len().min(limit)],
        &s_(result, "book_currency"),
        tool_label,
        empty,
    ));
    if listed.len() > limit {
        main = main.child(
            row()
                .w_full()
                .py(tk.r(0.75))
                .justify_center()
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    kit::btn(
                        tk,
                        "rc-tools-more",
                        Var::Ghost,
                        None,
                        &tc(
                            "review:reconcile.tools.showMore",
                            PAGE.min(listed.len() - limit),
                            &[],
                        ),
                        true,
                    )
                    .on_press(move |a: &mut App| st(a).limit = limit + PAGE),
                ),
        );
    }
    if wide {
        row()
            .w_full()
            .grow(1.0)
            .items_stretch()
            .child(wallets.w(tk.r(15.0)))
            .child(main)
    } else {
        col().w_full().grow(1.0).child(wallets).child(main)
    }
}

fn s_(v: &Value, k: &str) -> String {
    s(v, k)
}

fn row_matches(r: &Value, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let k = &r["kassiber"];
    let tl = &r["tool"];
    [
        s(k, "wallet"),
        s(k, "txid"),
        s(k, "classification"),
        s(k, "amount"),
        s(tl, "source"),
        s(tl, "txid"),
        s(tl, "type"),
        s(tl, "amount"),
    ]
    .iter()
    .any(|v| !v.is_empty() && v.to_lowercase().contains(needle))
}

fn wallet_list(app: &App, tk: &Tk, pairs: &[Value], selected: &str, tool: &str, wide: bool) -> El {
    let _ = app;
    let issues: usize = pairs
        .iter()
        .filter(|p| s(p, "state") == "paired")
        .map(|p| num(p, "issues") as usize)
        .sum();
    let amber = |n: usize| {
        text(crate::format::count(n))
            .font(tk.txs(500.0).features("'tnum' 1"))
            .color(kit::amber_text(tk))
    };
    let mut entries: Vec<(String, String, String, Option<&Value>, usize)> = vec![(
        ALL.into(),
        t("review:reconcile.tools.allWallets"),
        String::new(),
        None,
        issues,
    )];
    for p in pairs {
        let names = p["tool_names"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let title = p["wallet"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| names.clone());
        let detail = match s(p, "state").as_str() {
            "paired" => format!("{tool}: {names}"),
            "kassiber_only" => tr("review:reconcile.tools.notInTool", &[("tool", tool)]),
            _ => tr("review:reconcile.tools.notInKassiber", &[("tool", tool)]),
        };
        entries.push((
            s(p, "key"),
            title,
            detail,
            Some(p),
            num(p, "issues") as usize,
        ));
    }
    let mut ul = if wide {
        col().w_full().gap(2.0)
    } else {
        row().gap(tk.r(0.25))
    };
    for (key, title, detail, pair, count) in entries {
        let on = key == selected;
        let unpaired = pair.is_some_and(|p| s(p, "state") != "paired");
        let badge = if unpaired {
            ic(&I::INFO, tk.r(1.0), tk.muted_fg)
        } else if count > 0 {
            amber(count)
        } else {
            // Nothing to flag: no mark (signal only what differs).
            el()
        };
        let mut text_col = col().min_w(0.0).grow(1.0).child(
            text(title.clone())
                .font(tk.tsm(if unpaired { 400.0 } else { 500.0 }))
                .color(if unpaired { tk.muted_fg } else { tk.fg })
                .truncate(),
        );
        if !detail.is_empty() {
            text_col = text_col.child(
                text(detail)
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .truncate(),
            );
        }
        let k = key.clone();
        let mut b = row()
            .key(format!(
                "rc-wallet-{}",
                key.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
            ))
            .px(tk.r(0.625))
            .py(tk.r(0.375))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.375))
            .role("button")
            .checked(on)
            .label(title)
            .focusable(true)
            .on_press(move |a: &mut App| {
                st(a).pair_key = k.clone();
                st(a).limit = PAGE;
            })
            .child(text_col)
            .child(badge);
        b = if on {
            b.fill(tk.muted.alpha(0.6))
        } else {
            b.hover(tk.muted.alpha(0.35))
        };
        b = if wide {
            b.w_full()
        } else {
            b.max_w(tk.r(16.0))
        };
        ul = ul.child(b);
    }
    let mut nav = col()
        .fill(tk.muted.alpha(0.25))
        .role("navigation")
        .label(t("review:reconcile.tools.wallets"));
    if wide {
        nav = nav
            .padding(tk.r(0.75))
            .border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
            .child(
                text(t("review:reconcile.tools.wallets"))
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg)
                    .px(tk.r(0.625))
                    .pb(tk.r(0.375)),
            );
    } else {
        nav = nav
            .w_full()
            .padding(tk.r(0.5))
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border);
    }
    nav.child(ul)
}

fn signed(v: &str) -> String {
    if !v.is_empty() && !v.starts_with('-') && v.parse::<f64>().map(|n| n != 0.0).unwrap_or(true) {
        format!("+{v}")
    } else {
        v.to_owned()
    }
}

fn headline(
    tk: &Tk,
    result: &Value,
    pair: Option<&Value>,
    attention: &[&Value],
    pairs: &[Value],
    tool: &str,
    loading: bool,
) -> El {
    let count = |st: &str| attention.iter().filter(|r| s(r, "status") == st).count();
    let mut parts: Vec<(&str, usize)> = vec![
        ("differ", count("different")),
        ("onlyTool", count("only_tool")),
        ("onlyKassiber", count("only_kassiber")),
        ("excluded", count("excluded_in_kassiber")),
        ("unreadable", count("not_comparable")),
    ];
    if pair.is_none() {
        parts.push((
            "walletsNotInTool",
            pairs
                .iter()
                .filter(|p| s(p, "state") == "kassiber_only")
                .count(),
        ));
        parts.push((
            "walletsNotInKassiber",
            pairs
                .iter()
                .filter(|p| s(p, "state") == "tool_only")
                .count(),
        ));
    }
    let detail: Vec<String> = parts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| {
            tc(
                &format!("review:reconcile.tools.parts.{k}"),
                *n,
                &[("tool", tool)],
            )
        })
        .collect();
    let unpaired = pair.filter(|p| s(p, "state") != "paired");
    let nothing = result["rows"].as_array().is_none_or(Vec::is_empty);
    let tone = if nothing || unpaired.is_some() {
        "gap"
    } else if !attention.is_empty() {
        "attention"
    } else if !detail.is_empty() {
        "gap"
    } else {
        "agrees"
    };
    let title = if nothing {
        t("review:reconcile.tools.headline.nothingCompared")
    } else if let Some(p) = unpaired {
        if s(p, "state") == "kassiber_only" {
            tr(
                "review:reconcile.tools.headline.walletNotInTool",
                &[("wallet", &s(p, "wallet")), ("tool", tool)],
            )
        } else {
            let names = p["tool_names"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            tr(
                "review:reconcile.tools.headline.walletNotInKassiber",
                &[("wallet", &names), ("tool", tool)],
            )
        }
    } else if !attention.is_empty() {
        tc(
            "review:reconcile.tools.headline.attention",
            attention.len(),
            &[],
        )
    } else if !detail.is_empty() {
        t("review:reconcile.tools.headline.rowsAgree")
    } else {
        tr("review:reconcile.tools.headline.agrees", &[("tool", tool)])
    };
    let sub = if nothing {
        tr(
            "review:reconcile.tools.headline.nothingComparedDetail",
            &[("tool", tool)],
        )
    } else if let Some(p) = unpaired {
        tc(
            "review:reconcile.tools.headline.walletRows",
            num(p, "rows") as usize,
            &[],
        )
    } else if !detail.is_empty() {
        detail.join(" · ")
    } else {
        tc(
            "review:reconcile.tools.headline.compared",
            num(&result["counts"], "matched") as usize,
            &[],
        )
    };
    let amber = rune::color::hex("#f59e0b").unwrap();
    let (border, fill, glyph, ink): (Color, Color, &'static rune::Icon, Color) = match tone {
        "agrees" => (
            tk.border,
            tk.muted.alpha(0.4),
            &I::CIRCLE_CHECK,
            tk.muted_fg,
        ),
        "attention" => (
            amber.alpha(0.4),
            amber.alpha(0.1),
            &I::TRIANGLE_ALERT,
            rune::color::hex("#d97706").unwrap(),
        ),
        _ => (tk.border, tk.muted.alpha(0.4), &I::INFO, tk.muted_fg),
    };
    let mut box_ = row()
        .key("rc-tools-headline")
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .items_start()
        .radius(tk.r(0.5))
        .border(1.0, border)
        .fill(fill)
        .role("status")
        .child(ic(glyph, tk.r(1.25), ink).mt(tk.r(0.125)))
        .child(
            col()
                .min_w(0.0)
                .grow(1.0)
                .child(text(title).font(tk.tsm(500.0)).color(tk.fg).wrap())
                .child(
                    text(sub)
                        .font(tk.txs(400.0).lh(tk.r(1.25)))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        );
    if loading {
        box_ = box_.child(kit::spinner(tk, tk.r(1.0)));
    }
    let mut c = col().w_full().gap(tk.r(0.375)).child(box_);
    if pair.is_none() {
        for ins in result["insights"].as_array().into_iter().flatten() {
            let code = s(ins, "code");
            let glyph: &'static rune::Icon = match code.as_str() {
                "clock_offset" => &I::CLOCK,
                "journals_stale" => &I::TRIANGLE_ALERT,
                "tool_other_assets" | "kassiber_other_assets" => &I::INFO,
                _ => continue,
            };
            let p = &ins["params"];
            let hours = p["hours"]
                .as_f64()
                .map(|h| signed(&super::model::trim_num(h)))
                .unwrap_or_default();
            let n = num(p, "count") as usize;
            let assets = s(p, "assets");
            let line = tc(
                &format!("review:reconcile.tools.notes.{code}"),
                n,
                &[("tool", tool), ("hours", &hours), ("assets", &assets)],
            );
            c = c.child(
                row()
                    .px(tk.r(0.25))
                    .gap(tk.r(0.5))
                    .items_start()
                    .child(ic(glyph, tk.r(0.875), tk.muted_fg).mt(tk.r(0.125)))
                    .child(
                        text(line)
                            .font(tk.txs(400.0).lh(tk.r(1.25)))
                            .color(tk.muted_fg)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
    }
    c
}

fn fmt_date(v: &str) -> String {
    match crate::format::Day::parse(v) {
        Some(d) => d.long_ui(),
        None => v.to_owned(),
    }
}

fn fmt_time(v: &str) -> String {
    // `timeStyle: "short"` in the UI locale (UTC, as the headless web runs).
    let t = v.get(11..16).unwrap_or("");
    if t.len() != 5 {
        return String::new();
    }
    match crate::i18n::lang() {
        crate::i18n::Lang::De => t.to_owned(),
        crate::i18n::Lang::En => {
            let h: u32 = t[..2].parse().unwrap_or(0);
            let m = &t[3..];
            let (h12, ap) = match h {
                0 => (12, "AM"),
                1..=11 => (h, "AM"),
                12 => (12, "PM"),
                _ => (h - 12, "PM"),
            };
            format!("{h12}:{m}\u{202f}{ap}")
        }
    }
}

fn short_txid(v: &str) -> String {
    if v.chars().count() > 20 {
        format!(
            "{}…{}",
            v.chars().take(8).collect::<String>(),
            v.chars().skip(v.chars().count() - 6).collect::<String>()
        )
    } else {
        v.to_owned()
    }
}

fn row_list(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    rows: &[&Value],
    currency: &str,
    tool: &str,
    empty: String,
) -> El {
    let width = cx
        .layout_size("rc-tools-rows")
        .map(|[w, _]| w)
        .unwrap_or(900.0);
    let wide = width >= tk.r(42.0);
    // Edge to edge under the views (no card inside the card), on the table kit's inset.
    let mut c = col()
        .key("rc-tools-rows")
        .w_full()
        .min_h(tk.r(10.0))
        .max_h(tk.r(48.0))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
    let grid = |a: El, b: El, cc: El, d: El, e: El| {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(a.w(tk.r(6.5)))
            .child(b.grow(1.0).w(0.0).min_w(0.0))
            .child(cc.grow(1.0).w(0.0).min_w(0.0))
            .child(d.w(tk.r(13.0)).min_w(tk.r(8.0)))
            .child(e.w(tk.r(1.75)))
    };
    if wide {
        // The table kit's compact header band (the rows are `text-xs`).
        let h = |s: String| crate::ui::table::th_text_compact(tk, s);
        c = c.child(
            grid(
                h(t("review:reconcile.tools.columns.date")),
                h(tool.to_owned()),
                h("Kassiber".into()),
                h(t("review:reconcile.tools.columns.difference")),
                el(),
            )
            .h(tk.r(crate::ui::table::HEAD_H_COMPACT))
            .center_y()
            .px(tk.r(crate::ui::table::EDGE))
            .fill(tk.muted.alpha(0.5))
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .a11y_hidden(),
        );
    }
    if rows.is_empty() {
        return c.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(3.0))
                .justify_center()
                .child(text(empty).font(tk.tsm(400.0)).color(tk.muted_fg)),
        );
    }
    let mut ul = col()
        .w_full()
        .role("list")
        .label(t("review:reconcile.tools.rows"));
    for (i, r) in rows.iter().enumerate() {
        let mut li = row_el(app, tk, r, currency, tool, wide, &grid)
            .key(format!("rc-tools-row-{}", s(r, "id")));
        if i > 0 {
            li = li.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
        }
        ul = ul.child(li);
    }
    c.child(ul.scroll(rune::ScrollSpec {
        stick_bottom: false,
    }))
}

fn dot_color(tk: &Tk, status: &str) -> Color {
    let h = |x: &str| rune::color::hex(x).unwrap();
    let _ = tk;
    match status {
        "excluded_in_kassiber" => h("#94a3b8"),
        // Matched rows carry no dot (signal only what differs); the slot keeps the dates aligned.
        "matched" => Color::TRANSPARENT,
        "not_comparable" => h("#f43f5e"),
        _ => h("#f59e0b"),
    }
}

fn row_el(
    app: &App,
    tk: &Tk,
    r: &Value,
    currency: &str,
    tool: &str,
    wide: bool,
    grid: &dyn Fn(El, El, El, El, El) -> El,
) -> El {
    let _ = app;
    let flagged: Vec<String> = r["differences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| s(d, "severity") == "tax")
        .map(|d| s(d, "field"))
        .collect();
    let k = &r["kassiber"];
    let tl = &r["tool"];
    let when = if !s(k, "occurred_at").is_empty() {
        s(k, "occurred_at")
    } else {
        s(tl, "occurred_at")
    };
    let book_ref = k.is_object().then(|| s(k, "ref"));
    let classification = |v: &str| {
        v.split('+')
            .map(|p| {
                let key = format!("review:reconcile.tools.classification.{p}");
                if crate::i18n::has(&key) {
                    t(&key)
                } else {
                    p.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let mut date = col()
        .gap(0.0)
        .child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(crate::ui::dot(tk.r(0.5), dot_color(tk, &s(r, "status"))))
                .child(
                    text(fmt_date(&when))
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.fg),
                ),
        )
        .child(
            text(fmt_time(&when))
                .font(tk.txs(400.0).features("'tnum' 1"))
                .color(tk.muted_fg)
                .pl(tk.r(0.875)),
        );
    if !wide && let Some(rf) = book_ref.clone() {
        date = row()
            .w_full()
            .gap(tk.r(0.5))
            .child(date)
            .child(el().grow(1.0))
            .child(
                kit::link(
                    tk,
                    format!("rc-open-{}", s(r, "id")),
                    &t("review:reconcile.tools.openTransaction"),
                    true,
                )
                .on_press(move |a: &mut App| a.navigate(&format!("/transactions?tx={rf}"))),
            );
    }
    let tool_side = side(
        tk,
        tool,
        &tr("review:reconcile.tools.missingIn", &[("side", tool)]),
        &s(tl, "type"),
        flagged.iter().any(|f| f == "type"),
        &s(tl, "amount"),
        &s(tl, "asset"),
        &s(tl, "direction"),
        flagged.iter().any(|f| f == "amount" || f == "fee"),
        &if tl.is_object() {
            format!("{} · #{}", s(tl, "source"), s(tl, "row"))
        } else {
            String::new()
        },
        &s(tl, "txid"),
        tl.is_object(),
        false,
        wide,
    );
    let book_txid = if k.is_object() && s(k, "txid") != s(tl, "txid") {
        s(k, "txid")
    } else {
        String::new()
    };
    let book_side = side(
        tk,
        "Kassiber",
        &tr("review:reconcile.tools.missingIn", &[("side", "Kassiber")]),
        &if k.is_object() {
            classification(&s(k, "classification"))
        } else {
            String::new()
        },
        flagged.iter().any(|f| f == "type"),
        &s(k, "amount"),
        &s(k, "asset"),
        &s(k, "direction"),
        flagged.iter().any(|f| f == "amount" || f == "fee"),
        &s(k, "wallet"),
        &book_txid,
        k.is_object(),
        k["excluded"].as_bool() == Some(true),
        wide,
    );
    let issue = issue(tk, r, currency, tool);
    let open = match book_ref {
        Some(rf) if wide => crate::txdetail::widgets::icon_button(
            tk,
            format!("rc-open-{}", s(r, "id")),
            Var::Ghost,
            tk.r(1.75),
            &I::ARROW_UP_RIGHT,
            tk.r(0.875),
            t("review:reconcile.tools.openTransaction"),
        )
        .on_press(move |a: &mut App| a.navigate(&format!("/transactions?tx={rf}"))),
        _ => el(),
    };
    if wide {
        grid(date, tool_side, book_side, issue, open)
            .px(tk.r(crate::ui::table::EDGE))
            .py(tk.r(0.625))
            .hover(tk.muted.alpha(0.35))
            .role("listitem")
    } else {
        col()
            .w_full()
            .px(tk.r(crate::ui::table::EDGE))
            .py(tk.r(0.625))
            .gap(tk.r(0.375))
            .role("listitem")
            .child(date)
            .child(tool_side)
            .child(book_side)
            .child(issue)
    }
}

#[allow(clippy::too_many_arguments)]
fn side(
    tk: &Tk,
    label: &str,
    missing: &str,
    ty: &str,
    type_differs: bool,
    amount: &str,
    asset: &str,
    direction: &str,
    amount_differs: bool,
    where_: &str,
    txid: &str,
    present: bool,
    excluded: bool,
    wide: bool,
) -> El {
    let differs = kit::amber_text(tk);
    let mut body = col().min_w(0.0).grow(1.0);
    if present {
        let mut top = row().gap(tk.r(0.5)).items_end().flex_wrap();
        let mut amt = row().gap(tk.r(0.125)).center_y();
        if direction == "inbound" {
            amt = amt.child(ic(
                &I::ARROW_DOWN_LEFT,
                tk.r(0.75),
                rune::color::hex("#059669").unwrap(),
            ));
        } else if !direction.is_empty() {
            amt = amt.child(ic(
                &I::ARROW_UP_RIGHT,
                tk.r(0.75),
                rune::color::hex("#e11d48").unwrap(),
            ));
        }
        if !amount.is_empty() {
            amt = amt.child(
                text(format!("{amount} {asset}"))
                    .font(
                        tk.txs(if amount_differs { 500.0 } else { 400.0 })
                            .features("'tnum' 1"),
                    )
                    .color(if amount_differs { differs } else { tk.fg }),
            );
        }
        top = top.child(amt);
        if !ty.is_empty() {
            top = top.child(
                text(ty.to_owned())
                    .font(tk.txs(if type_differs { 500.0 } else { 400.0 }))
                    .color(if type_differs { differs } else { tk.muted_fg }),
            );
        }
        let mut bottom = row().gap(tk.r(0.375)).center_y().min_w(0.0).child(
            text(where_.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .min_w(0.0),
        );
        if excluded {
            bottom = bottom.child(
                row()
                    .px(tk.r(0.25))
                    .radius(tk.r(0.125))
                    .border(1.0, tk.border)
                    .child(
                        text(t("review:reconcile.tools.excludedBadge"))
                            .font(tk.px(10.0, 400.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        if !txid.is_empty() {
            bottom = bottom.child(
                text(short_txid(txid))
                    .font(tk.px(11.0, 400.0).mono())
                    .color(tk.muted_fg)
                    .help(txid.to_owned()),
            );
        }
        body = body.child(top).child(bottom);
    } else {
        body = body.child(
            text(missing.to_owned())
                .font(tk.txs(400.0).italic())
                .color(tk.muted_fg),
        );
    }
    if wide {
        body
    } else {
        row()
            .w_full()
            .gap(tk.r(0.5))
            .items_start()
            .child(
                text(label.to_owned())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .w(tk.r(5.5))
                    .truncate(),
            )
            .child(body)
    }
}

fn issue(tk: &Tk, r: &Value, currency: &str, tool: &str) -> El {
    let tag = |s: String, tone: &str| {
        let fg = match tone {
            "tax" => {
                if tk.dark {
                    rune::color::hex("#fde68a").unwrap()
                } else {
                    rune::color::hex("#92400e").unwrap()
                }
            }
            "warn" => kit::amber_text(tk),
            _ => tk.muted_fg,
        };
        text(s).font(tk.txs(400.0)).color(fg)
    };
    let mut c = row().min_w(0.0).gap(tk.r(0.25)).items_start().flex_wrap();
    let status = s(r, "status");
    match status.as_str() {
        "not_comparable" => {
            c = c
                .child(tag(t("review:reconcile.tools.issue.unreadable"), "tax"))
                .child(
                    text(s(r, "reason"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .max_lines(2)
                        .wrap()
                        .help(s(r, "reason")),
                );
        }
        // The row's amber dot already flags it; the words say which side is missing.
        "only_tool" => c = c.child(tag(t("review:reconcile.tools.issue.onlyTool"), "warn")),
        "only_kassiber" => {
            c = c.child(tag(
                tr(
                    "review:reconcile.tools.issue.onlyKassiber",
                    &[("tool", tool)],
                ),
                "warn",
            ))
        }
        "excluded_in_kassiber" => {
            c = c.child(tag(t("review:reconcile.tools.issue.excluded"), "info"))
        }
        _ => {
            if status == "matched" {
                c = c.child(tag(t("review:reconcile.tools.issue.matched"), "plain"));
            }
            for d in r["differences"].as_array().into_iter().flatten() {
                let field = s(d, "field");
                let label = t(&format!("review:reconcile.tools.diff.{field}"));
                let delta = s(d, "delta");
                let txt = if field == "time" {
                    if s(d, "severity") == "tax" {
                        t("review:reconcile.tools.diff.taxYear")
                    } else {
                        format!(
                            "{label} {}",
                            tr(
                                "review:reconcile.tools.diff.minutes",
                                &[("minutes", &signed(&delta))]
                            )
                        )
                    }
                } else if field == "value" && !delta.is_empty() {
                    let v: f64 = delta.parse().unwrap_or(0.0);
                    let m = if currency.is_empty() {
                        delta.clone()
                    } else if v > 0.0 {
                        format!("+{}", crate::format::fiat(v, currency))
                    } else {
                        crate::format::fiat(v, currency)
                    };
                    format!("{label} {m}")
                } else if !delta.is_empty() {
                    format!("{label} {}", signed(&delta))
                } else {
                    label
                };
                c = c.child(tag(
                    txt,
                    if s(d, "severity") == "tax" {
                        "tax"
                    } else {
                        "info"
                    },
                ));
            }
            if s(r, "match") == "probable" {
                c = c.child(tag(t("review:reconcile.tools.probable"), "info"));
            }
        }
    }
    c
}

/// Scenes: a CoinTracking export as if picked, compared now (a local read of its bytes).
pub fn scene_load(app: &mut App, name: &str, bytes: &[u8]) {
    take_file(app, name.to_owned(), bytes.to_vec());
    if let Some(a) = args(&app.review.tools) {
        let k = key_of(&a);
        if app
            .send("ui.reports.cross_check", a, "review.tools.compare")
            .is_some()
        {
            let generation = app.book_generation;
            st(app).generation = generation;
            st(app).queue.push_back((k, generation));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(matched: u64) -> Reply {
        Reply {
            kind: "ui.reports.cross_check".into(),
            data: json!({ "matched": matched }),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    /// A comparison read before a book refresh (an exclusion in Transactions, a journal run)
    /// is not kept: the page compares again against the refreshed book.
    #[test]
    fn a_comparison_from_before_a_book_refresh_is_read_again() {
        let mut app = crate::app::tests::quiet_app();
        let before = app.book_generation;
        let s = st(&mut app);
        s.file = Some(("export.csv".into(), "AA==".into()));
        s.tool = Some("cointracking");
        let k = key_of(&args(s).unwrap());
        s.queue.push_back((k.clone(), before));
        app.book_generation += 1;
        on_reply(&mut app, "review.tools.compare", &reply(1));
        assert!(
            !app.review.tools.cache.contains_key(&k),
            "the old book's result"
        );
        let generation = app.book_generation;
        st(&mut app).queue.push_back((k.clone(), generation));
        on_reply(&mut app, "review.tools.compare", &reply(2));
        assert_eq!(app.review.tools.cache[&k]["matched"], 2);
    }

    /// Another book while a comparison is out: its answer is dropped, and the new book's own
    /// comparison of the same file still lands.
    #[test]
    fn a_comparison_from_before_a_book_switch_is_dropped() {
        let mut app = crate::app::tests::quiet_app();
        let generation = app.book_generation;
        let k = "k".to_owned();
        st(&mut app).queue.push_back((k.clone(), generation));
        restart(&mut app);
        st(&mut app).queue.push_back((k.clone(), generation));
        on_reply(&mut app, "review.tools.compare", &reply(1));
        assert!(
            !app.review.tools.cache.contains_key(&k),
            "the old book's result"
        );
        on_reply(&mut app, "review.tools.compare", &reply(2));
        assert_eq!(app.review.tools.cache[&k]["matched"], 2);
    }
}
