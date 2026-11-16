//! The workbench page (`ChainAnalysisWorkbench`): header with Tools and the assistant handoff,
//! the error/notice banners, the three workspaces, the watches, saved investigations and
//! acquisition sections, the query form, the executed-query strip, the graph/table frame with
//! its inspector, and the Results card.

use rune::{Color, Cx, El, Shadow, col, el, row, text};
use rune_kit::Placement;
use serde_json::Value;

use super::kit::{self, ca, field, input, opt, select};
use super::model::{self, Sel, Workspace};
use super::{graph, inspector, panels, results, st};
use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Variant};
use crate::ui::{ic, sensitive};

fn win_w(app: &App, cx: &Cx<App>) -> f32 {
    let w = cx
        .layout_size("sh-root")
        .map(|[w, _]| w)
        .unwrap_or(app.shell.window_w);
    if w > 0.0 { w } else { 1440.0 }
}

/// A grid of `cols` equal columns; items carry their column span.
fn grid(cols: usize, gap: f32, items: Vec<(El, usize)>) -> El {
    let mut c = col().w_full().gap(gap);
    let mut line = row().w_full().gap(gap).items_end();
    let mut used = 0;
    for (e, span) in items {
        let span = span.min(cols);
        if used + span > cols {
            if used < cols {
                line = line.child(el().w(0.0).grow((cols - used) as f32));
            }
            c = c.child(line);
            line = row().w_full().gap(gap).items_end();
            used = 0;
        }
        line = line.child(e.w(0.0).grow(span as f32).min_w(0.0));
        used += span;
    }
    if used > 0 {
        if used < cols {
            line = line.child(el().w(0.0).grow((cols - used) as f32));
        }
        c = c.child(line);
    }
    c
}

/// An attention line (`role="status"`): the amber triangle and its sentence.
fn note(tk: &Tk, body: String) -> El {
    row()
        .w_full()
        .gap(tk.r(0.5))
        .items_start()
        .role("status")
        .child(ic(&I::TRIANGLE_ALERT, tk.r(0.875), kit::amber(tk)).mt(tk.r(0.125)))
        .child(
            text(body)
                .font(tk.txs(400.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

/// The centred state of a page with no result yet (Review's empty state): a tinted round
/// glyph (or the spinner while the query runs), a title and one muted sentence.
fn hero(app: &App, cx: &Cx<App>, tk: &Tk, busy: bool) -> El {
    let win_h = cx
        .layout_size("sh-root")
        .map(|[_, h]| h)
        .filter(|h| *h > 0.0)
        .unwrap_or(app.shell.window_h);
    let free = (win_h - crate::shell::window_top_inset() - tk.r(10.0)).max(tk.r(14.0));
    let badge = if busy {
        w::spinner(tk, tk.r(1.25))
    } else {
        ic(&I::NETWORK, tk.r(1.25), tk.muted_fg)
    };
    let (title, body) = if busy {
        (t("chainAnalysis:running"), String::new())
    } else {
        (t("chainAnalysis:workbench.graph"), t("chainAnalysis:empty"))
    };
    let mut c = col()
        .key("ca-empty")
        .w_full()
        .min_h(free)
        .center()
        .gap(tk.r(0.5))
        .child(
            row()
                .size(tk.r(3.0), tk.r(3.0))
                .center()
                .radius(tk.r(1.5))
                .fill(tk.muted.alpha(0.6))
                .mb(tk.r(0.5))
                .a11y_hidden()
                .child(badge),
        )
        .child(
            text(title)
                .font(tk.tbase(600.0))
                .color(tk.fg)
                .role(if busy { "status" } else { "heading" })
                .level(2),
        );
    if !body.is_empty() {
        c = c.child(
            text(body)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .align(rune::Align::Center)
                .max_w(tk.r(32.0)),
        );
    }
    c
}

/// Graph | Table: one segmented control (the pressed side filled).
fn view_toggle(tk: &Tk, table: bool) -> El {
    let side = |k: &'static str, label: String, on: bool, to: bool| {
        let e = row()
            .key(k)
            .h(tk.r(1.5))
            .px(tk.r(0.625))
            .center()
            .radius(tk.r(0.3125))
            .role("button")
            .label(label.clone())
            .focusable(true)
            .checked(on)
            .child(
                text(label)
                    .font(tk.txs(500.0))
                    .color(if on { tk.fg } else { tk.muted_fg }),
            );
        if on {
            e.fill(tk.background).border(1.0, tk.border)
        } else {
            e.hover(tk.background.alpha(0.6))
                .on_press(move |a: &mut App| st(a).table = to)
        }
    };
    row()
        .padding(2.0)
        .gap(2.0)
        .radius(tk.r(0.4375))
        .fill(tk.muted)
        .child(side(
            "ca-view-graph",
            t("chainAnalysis:graphView"),
            !table,
            false,
        ))
        .child(side("ca-view-table", t("chainAnalysis:table"), table, true))
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let shell = col()
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background);
    if app.provenance.chain.mounted_for.is_none() {
        return shell.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .role("status")
                .child(w::spinner(tk, tk.r(0.875)))
                .child(
                    text(t("chainAnalysis:scopeLoading"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    let s = &app.provenance.chain;
    let mut page = shell.child(header(app, cx, tk, layer));
    if let Some(e) = &s.error {
        page = page.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.625))
                .gap(tk.r(0.625))
                .items_start()
                .radius(tk.r(0.5))
                .border(1.0, tk.destructive.alpha(0.3))
                .border_layout()
                .fill(tk.destructive.alpha(0.06))
                .role("alert")
                .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), tk.destructive).mt(tk.r(0.125)))
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    crate::shell::transient::dismiss(
                        tk,
                        "ca-err-dismiss",
                        tk.muted_fg,
                        |a: &mut App| st(a).error = None,
                    )
                    .mt(-4.0)
                    .mr(-4.0),
                ),
        );
    }
    if let Some(n) = &s.notice {
        // The × is an owner request over the web (the next action or leaving clears it too).
        page = page.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.625))
                .gap(tk.r(0.5))
                .items_start()
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.card)
                .role("status")
                .child({
                    let msg = text(n.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap()
                        .w_full();
                    // `toastopen`: the export's Open / Show in Finder under its "saved" notice.
                    let mut c = rune::col().grow(1.0).min_w(0.0).gap(tk.r(0.375)).child(msg);
                    if !s.export_actions.is_empty() && *n == t("chainAnalysis:exportSaved") {
                        c = c.child(crate::toastopen::links(
                            tk,
                            &s.export_actions,
                            |a| format!("ca-export-{}", a.id),
                            tk.muted_fg,
                            tk.fg,
                        ));
                    }
                    c
                })
                .child(
                    crate::shell::transient::dismiss(
                        tk,
                        "ca-notice-dismiss",
                        tk.muted_fg,
                        |a: &mut App| st(a).notice = None,
                    )
                    .mt(-4.0)
                    .mr(-4.0),
                ),
        );
    }
    match s.workspace {
        Workspace::Psbt => return page.child(super::psbt::view(app, tk, layer)),
        Workspace::Datasets => return page.child(super::datasets::view(app, tk, layer)),
        Workspace::Graph => {}
    }
    if kit::is_open(app, "filters") {
        page = page.child(filters(app, cx, tk, layer));
    }
    if kit::is_open(app, "inbox") {
        page = page.child(
            section(tk, "ca-inbox", &t("chainAnalysis:watch.inbox"), |a| {
                st(a).open.remove("inbox");
            })
            .child(panels::inbox(app, tk)),
        );
    }
    if s.show_saved {
        page = page.child(
            section(tk, "ca-saved", &t("chainAnalysis:case.title"), |a| {
                st(a).show_saved = false
            })
            .child(panels::saved(app, tk, layer)),
        );
    }
    if s.show_acquire {
        page = page.child(
            section(tk, "ca-acquire", &t("chainAnalysis:acquire.title"), |a| {
                st(a).show_acquire = false
            })
            .child(panels::acquisition(app, tk, layer))
            .child(panels::sources(app, tk, layer)),
        );
    }
    let busy = super::running(app);
    let Some(r) = s.result.clone() else {
        return page.child(hero(app, cx, tk, busy));
    };
    if s.refresh_failed {
        page = page.child(note(tk, t("chainAnalysis:refreshFailed")));
    }
    if s.historical {
        page = page.child(note(tk, t("chainAnalysis:case.historical")));
    }
    if s.query.key() != r.query.key() {
        page = page.child(note(tk, t("chainAnalysis:queryChanged")));
    }
    // The frame's head band: what was executed on the left; the view switch and the one
    // action of the result (Watch) on the right. The previous result stays up while a new
    // query runs, the band saying so.
    let mut left = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.75))
        .flex_wrap()
        .center_y();
    if busy {
        left = left.child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .role("status")
                .label(t("chainAnalysis:running"))
                .child(w::spinner(tk, tk.r(0.75)))
                .child(
                    text(t("chainAnalysis:running"))
                        .font(tk.px(11.0, 500.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    left = left.child(strip(
        tk,
        &r.query,
        r.coverage(),
        r.nodes.len(),
        r.edges.len(),
    ));
    let band = row()
        .w_full()
        .min_h(tk.r(2.75))
        .px(tk.r(0.75))
        .py(tk.r(0.375))
        .gap(tk.r(0.75))
        .items_start()
        .fill(tk.muted.alpha(0.3))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(left.min_h(tk.r(2.0)))
        .child(
            panels::watch_action(
                app,
                tk,
                layer,
                &format!("result:{}", r.query.key()),
                r.query.clone(),
                None,
                false,
            )
            .max_w(tk.r(30.0)),
        )
        .child(
            row()
                .h(tk.r(2.0))
                .center_y()
                .child(view_toggle(tk, s.table)),
        );
    let inspecting = s.sel.as_ref().is_some_and(|x| {
        if x.edge {
            r.edge(&x.id).is_some()
        } else {
            r.node(&x.id).is_some()
        }
    });
    let main = if s.table {
        table(app, tk, &r)
    } else if r.nodes.is_empty() {
        col()
            .w_full()
            .min_h(tk.r(20.0))
            .padding(tk.r(2.0))
            .gap(tk.r(0.75))
            .center()
            .child(
                row()
                    .size(tk.r(3.0), tk.r(3.0))
                    .center()
                    .radius(tk.r(1.5))
                    .fill(tk.muted.alpha(0.6))
                    .a11y_hidden()
                    .child(ic(&I::NETWORK, tk.r(1.25), tk.muted_fg)),
            )
            .child(
                text(t("chainAnalysis:noNodes"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .align(rune::Align::Center)
                    .max_w(tk.r(28.0)),
            )
    } else {
        graph::view(app, cx, tk)
    };
    let mut legend = row()
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .gap(tk.r(1.0))
        .flex_wrap()
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
    for (item, color, dashed) in [
        ("physical", ca::physical(), false),
        ("custody", ca::custody(), true),
        ("hypothesis", ca::hypothesis(), true),
    ] {
        legend = legend.child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(kit::swatch(color, dashed, 24.0))
                .child(
                    text(t(&format!("chainAnalysis:{item}")))
                        .font(tk.px(11.0, 400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    if !inspecting && !r.nodes.is_empty() {
        legend = legend.child(el().grow(1.0)).child(
            text(t("chainAnalysis:select"))
                .font(tk.px(11.0, 400.0))
                .color(tk.muted_fg),
        );
    }
    let mut body = row()
        .w_full()
        .items_start()
        .child(col().grow(1.0).min_w(0.0).child(main).child(legend));
    if inspecting && let Some(i) = inspector::view(app, tk, layer) {
        body = body.child(i);
    }
    let frame = crate::ui::surface(tk)
        .w_full()
        .clip_rounded(tk.r(0.75))
        .role("region")
        .label(t("chainAnalysis:workbench.graph"))
        .child(band)
        .child(body);
    page.child(frame).child(results::view(app, tk, layer))
}

/// A closable section: a card with its title bar and the X.
fn section(tk: &Tk, k: &str, title: &str, close: fn(&mut App)) -> El {
    crate::ui::surface(tk)
        .key(k.to_owned())
        .w_full()
        .role("region")
        .label(title.to_owned())
        .child(
            row()
                .w_full()
                .pl(tk.r(1.0))
                .pr(tk.r(0.5))
                .py(tk.r(0.375))
                .center_y()
                .justify_between()
                .child(
                    text(title.to_owned())
                        .font(tk.tsm(600.0))
                        .color(tk.fg)
                        .role("heading")
                        .level(2),
                )
                .child(
                    w::icon_button(
                        tk,
                        format!("{k}-close"),
                        Variant::Ghost,
                        tk.r(2.0),
                        &I::X,
                        tk.r(1.0),
                        t("chainAnalysis:dismiss"),
                    )
                    .on_press(close),
                ),
        )
}

/// The page header: the query (starting point, Run, Advanced) as the one primary entry, then
/// the assistant hand-off and the Tools menu. Unread watch events show as the only chip.
fn header(app: &App, cx: &Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.provenance.chain;
    let wide = win_w(app, cx) >= 1180.0;
    let mut bar = row().w_full().min_h(tk.r(2.25)).gap(tk.r(0.5)).center_y();
    if s.workspace != Workspace::Graph {
        bar = bar
            .child(
                w::button(
                    tk,
                    "ca-back",
                    Variant::Ghost,
                    w::Size::Sm,
                    Some(&I::ARROW_LEFT),
                    &t("chainAnalysis:backToInvestigation"),
                    true,
                )
                .on_press(|a: &mut App| st(a).workspace = Workspace::Graph),
            )
            .child(el().grow(1.0));
    } else {
        let busy = super::running(app);
        let run_label = t(if busy {
            "chainAnalysis:running"
        } else {
            "chainAnalysis:run"
        });
        let run = w::button_with(
            tk,
            "ca-run",
            Variant::Default,
            w::Size::Sm,
            Some(&I::PLAY),
            Some(tk.r(0.875)),
            &run_label,
            !busy,
        )
        .h(tk.r(2.25))
        .busy(busy);
        let open = kit::is_open(app, "filters");
        let n = s.query.active_filter_count();
        let label = t("chainAnalysis:filters");
        let mut adv = w::button_surface(
            tk,
            row()
                .key("filters-summary")
                .h(tk.r(2.25))
                .px(tk.r(0.625))
                .gap(tk.r(0.375))
                .center(),
            Variant::Outline,
        )
        .role("button")
        .label(if n > 0 {
            format!("{label} ({n})")
        } else {
            label.clone()
        })
        .expanded(open)
        .focusable(true)
        .on_press(|a: &mut App| kit::toggle_open(a, "filters"))
        .child(ic(&I::SLIDERS_HORIZONTAL, tk.r(0.875), tk.fg));
        if wide {
            adv = adv.child(text(label).font(tk.tsm(500.0)).color(tk.fg));
        }
        if n > 0 {
            adv = adv.child(
                row()
                    .min_w(tk.r(1.125))
                    .h(tk.r(1.125))
                    .px(tk.r(0.25))
                    .center()
                    .radius(tk.r(0.5625))
                    .fill(tk.muted)
                    .child(
                        text(n.to_string())
                            .font(tk.px(10.0, 600.0).features("'tnum' 1"))
                            .color(tk.fg),
                    )
                    .a11y_hidden(),
            );
        }
        bar = bar
            .child(
                input(
                    tk,
                    app,
                    "q.subject",
                    &t("chainAnalysis:subject"),
                    &t("chainAnalysis:subjectPlaceholder"),
                    true,
                    tk.r(2.25),
                    true,
                    Some(super::on_subject),
                )
                .grow(1.0)
                .min_w(tk.r(10.0)),
            )
            .child(if busy { run } else { run.on_press(super::run) })
            .child(adv);
        let unread = s
            .inbox
            .events
            .as_ref()
            .and_then(|e| e.get("unread_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if unread > 0 {
            let label = format!("{} ({unread})", t("chainAnalysis:watch.inbox"));
            bar = bar.child(
                w::button_surface(
                    tk,
                    row()
                        .key("ca-inbox-chip")
                        .h(tk.r(2.25))
                        .px(tk.r(0.625))
                        .gap(tk.r(0.375))
                        .center(),
                    Variant::Outline,
                )
                .role("button")
                .label(label)
                .focusable(true)
                .on_press(|a: &mut App| {
                    st(a).open.insert("inbox".into());
                })
                .child(ic(&I::BELL, tk.r(0.875), kit::amber(tk)))
                .child(
                    text(unread.to_string())
                        .font(tk.tsm(600.0).features("'tnum' 1"))
                        .color(kit::amber(tk)),
                ),
            );
        }
        if s.result.is_some() {
            let enabled = app.prefs.ai_features && !s.asking && !app.assistant.streaming;
            let b = if wide {
                w::button_with(
                    tk,
                    "ca-ask",
                    Variant::Outline,
                    w::Size::Sm,
                    Some(&I::SPARKLES),
                    Some(tk.r(0.875)),
                    &t("chainAnalysis:ask"),
                    enabled,
                )
                .h(tk.r(2.25))
                .busy(s.asking)
            } else {
                let b = w::icon_button(
                    tk,
                    "ca-ask",
                    Variant::Outline,
                    tk.r(2.25),
                    &I::SPARKLES,
                    tk.r(0.875),
                    t("chainAnalysis:ask"),
                )
                .help(t("chainAnalysis:ask"))
                .busy(s.asking);
                if enabled {
                    b
                } else {
                    b.opacity(0.5).disabled(true)
                }
            };
            bar = bar.child(if enabled { b.on_press(super::ask) } else { b });
        }
    }
    let open = s.popup.as_deref() == Some("ca-tools");
    let tools = w::button_surface(
        tk,
        row()
            .key("ca-tools")
            .h(tk.r(2.25))
            .px(tk.r(0.625))
            .gap(tk.r(0.375))
            .center(),
        Variant::Outline,
    )
    .role("button")
    .label(t("chainAnalysis:tools"))
    .focusable(true)
    .child(ic(&I::WRENCH, tk.r(0.875), tk.fg))
    .child(
        text(t("chainAnalysis:tools"))
            .font(tk.tsm(500.0))
            .color(tk.fg),
    )
    .child(ic(&I::CHEVRON_DOWN, tk.r(0.875), tk.muted_fg));
    let tools =
        crate::ui::popup::trigger(tools, crate::ui::popup::Kind::Menu, open, |a: &mut App| {
            let p = &mut st(a).popup;
            *p = if p.as_deref() == Some("ca-tools") {
                None
            } else {
                Some("ca-tools".into())
            };
        });
    if open {
        tools_menu(app, tk, layer);
    }
    bar.child(tools)
}

fn tools_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let s = &app.provenance.chain;
    let export_ok = s.workspace == Workspace::Graph && s.result.is_some();
    let close = |a: &mut App| st(a).popup = None;
    let mut p = crate::ui::popup::Popup::menu("ca-tools", close).list_key("ca-tools-list");
    let mut item = |k: &str, label: String, enabled: bool, f: fn(&mut App)| {
        let mut r = row()
            .w_full()
            .px(tk.r(0.5))
            .py(tk.r(0.375))
            .radius(tk.r(0.25))
            .role("menuitem")
            .label(label.clone())
            .child(text(label.clone()).font(tk.tsm(400.0)).color(tk.fg));
        if enabled {
            r = r
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .on_press(move |a: &mut App| {
                    st(a).popup = None;
                    f(a);
                });
        } else {
            r = r.opacity(0.5);
        }
        p.item(r, k, label, enabled, false);
    };
    item(
        "ca-tool-psbt",
        t("chainAnalysis:workbench.psbt"),
        true,
        |a| st(a).workspace = Workspace::Psbt,
    );
    item(
        "ca-tool-datasets",
        t("chainAnalysis:workbench.datasets"),
        true,
        |a| st(a).workspace = Workspace::Datasets,
    );
    item(
        "ca-tool-watches",
        t("chainAnalysis:watch.inbox"),
        true,
        |a| {
            let s = st(a);
            s.workspace = Workspace::Graph;
            s.open.insert("inbox".into());
        },
    );
    item("ca-tool-saved", t("chainAnalysis:case.title"), true, |a| {
        let s = st(a);
        s.workspace = Workspace::Graph;
        s.show_saved = true;
        s.show_acquire = false;
    });
    item(
        "ca-tool-acquire",
        t("chainAnalysis:acquire.title"),
        true,
        panels::show_acquire,
    );
    item(
        "ca-tool-json",
        t("chainAnalysis:exportJson"),
        export_ok,
        |a| st(a).export = Some("json"),
    );
    item(
        "ca-tool-csv",
        t("chainAnalysis:exportCsv"),
        export_ok,
        |a| st(a).export = Some("csv"),
    );
    item("ca-tool-mirror", t("chainAnalysis:privacy"), true, |a| {
        crate::shell::go(a, "/privacy-mirror")
    });
    let shadow = if tk.dark {
        Shadow {
            y: 18.0,
            blur: 44.0,
            spread: -18.0,
            color: Color::BLACK.alpha(0.8),
            ..Shadow::default()
        }
    } else {
        Shadow {
            y: 16.0,
            blur: 40.0,
            spread: -18.0,
            color: Color::BLACK.alpha(0.55),
            ..Shadow::default()
        }
    };
    let surface = |l: El| {
        col()
            .min_h(0.0)
            .child(l)
            .padding(tk.r(0.25) + 1.0)
            .radius(tk.r(0.375))
            .fill(tk.popover)
            .border(1.0, tk.fg.alpha(0.10))
            .box_shadow(shadow)
            .key("ca-tools-menu")
            .transition(crate::shell::popover_enter(
                tk,
                rune::Side::Bottom,
                rune::Align::End,
            ))
    };
    p.mount(
        tk,
        layer,
        col().min_w(tk.r(8.0)),
        surface,
        Placement::BottomEnd,
        4.0,
    );
}

/// `ExecutedQueryStrip`: the boundary facts of the executed query, and (only when it is not
/// within observed coverage) the coverage chip that opens the Coverage results.
fn strip(tk: &Tk, q: &model::Query, coverage: &Value, nodes: usize, edges: usize) -> El {
    use crate::ui::table as tb;
    let (relations, hypotheses) = q.effective_layers();
    let style = tk.px(11.0, 400.0);
    let mut r = row()
        .gap(tk.r(0.75))
        .flex_wrap()
        .center_y()
        .role("group")
        .label(t("chainAnalysis:resultQuery"))
        .child(
            text(t(&format!("chainAnalysis:{}", q.observer)))
                .font(tk.px(11.0, 600.0))
                .color(tk.fg)
                .help(t("chainAnalysis:observer")),
        );
    r = r.child(
        text(format!(
            "{nodes} {} · {edges} {}",
            t("chainAnalysis:nodes"),
            t("chainAnalysis:edges")
        ))
        .font(style.features("'tnum' 1"))
        .color(tk.muted_fg),
    );
    if relations {
        r = r.child(
            text(t("chainAnalysis:relations"))
                .font(style)
                .color(tk.muted_fg),
        );
    }
    if hypotheses {
        r = r.child(
            text(t("chainAnalysis:hypotheses"))
                .font(style)
                .color(tk.muted_fg),
        );
    }
    let domain: Vec<String> = [q.chain.clone(), q.network.clone()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if !domain.is_empty() {
        r = r.child(
            text(domain.join(" / "))
                .font(style.mono())
                .color(tk.muted_fg),
        );
    }
    let complete = coverage
        .get("complete")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let label = t(
        if coverage
            .get("budget_exhausted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "chainAnalysis:budgetReached"
        } else if complete {
            "chainAnalysis:complete"
        } else {
            "chainAnalysis:incomplete"
        },
    );
    let chip = if complete {
        text(label.clone())
            .font(style)
            .color(tk.muted_fg)
            .px(tk.r(0.25))
            .py(2.0)
            .radius(6.0)
            .hover(tk.muted.alpha(0.6))
    } else {
        tb::chip(tk, tb::Tone::Warning, None, &label)
    };
    r.child(
        chip.key("ca-coverage-chip")
            .role("button")
            .label(label)
            .focusable(true)
            .on_press(|a: &mut App| st(a).tab = "coverage"),
    )
}

/// The Advanced controls of the query (the header's disclosure).
fn filters(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.provenance.chain;
    let q = &s.query;
    let c = crate::ui::surface(tk)
        .key("ca-filters")
        .w_full()
        .padding(tk.r(1.0))
        .role("group")
        .label(t("chainAnalysis:filters"));
    let cols = if win_w(app, cx) >= 1024.0 { 6 } else { 3 };
    let mut items: Vec<(El, usize)> = vec![];
    if q.mode != "overview" {
        items.push((
            field(
                tk,
                &t("chainAnalysis:direction"),
                select(
                    app,
                    tk,
                    layer,
                    "q-direction",
                    &t("chainAnalysis:direction"),
                    &q.direction,
                    ["both", "backward", "forward"]
                        .iter()
                        .map(|d| opt(*d, t(&format!("chainAnalysis:{d}"))))
                        .collect(),
                    true,
                    32.0,
                    |a: &mut App, v: &str| st(a).query.direction = v.to_owned(),
                ),
            ),
            1,
        ));
    }
    items.push((
        field(
            tk,
            &t("chainAnalysis:observer"),
            select(
                app,
                tk,
                layer,
                "q-observer",
                &t("chainAnalysis:observer"),
                &q.observer,
                ["owner", "public", "disclosed"]
                    .iter()
                    .map(|d| opt(*d, t(&format!("chainAnalysis:{d}"))))
                    .collect(),
                true,
                32.0,
                |a: &mut App, v: &str| st(a).query.observer = v.to_owned(),
            ),
        ),
        1,
    ));
    items.push((
        field(
            tk,
            &t("chainAnalysis:mode"),
            select(
                app,
                tk,
                layer,
                "q-mode",
                &t("chainAnalysis:mode"),
                &q.mode,
                ["overview", "trace", "path"]
                    .iter()
                    .map(|d| opt(*d, t(&format!("chainAnalysis:{d}"))))
                    .collect(),
                true,
                32.0,
                |a: &mut App, v: &str| st(a).query.mode = v.to_owned(),
            ),
        ),
        1,
    ));
    if q.mode == "path" {
        items.push((
            field(
                tk,
                &t("chainAnalysis:target"),
                input(
                    tk,
                    app,
                    "q.target",
                    &t("chainAnalysis:target"),
                    &t("chainAnalysis:targetPlaceholder"),
                    true,
                    32.0,
                    true,
                    Some(super::on_filters),
                ),
            ),
            if cols == 6 { 3 } else { 2 },
        ));
    }
    items.push((
        field(
            tk,
            &t("chainAnalysis:chain"),
            select(
                app,
                tk,
                layer,
                "q-chain",
                &t("chainAnalysis:chain"),
                q.chain.as_deref().unwrap_or(""),
                vec![
                    opt("", t("chainAnalysis:all")),
                    opt("bitcoin", "Bitcoin"),
                    opt("liquid", "Liquid"),
                ],
                true,
                32.0,
                |a: &mut App, v: &str| st(a).query.chain = (!v.is_empty()).then(|| v.to_owned()),
            ),
        ),
        1,
    ));
    let mut nets = vec![opt("", t("chainAnalysis:all"))];
    nets.extend(
        ["main", "test", "signet", "regtest"]
            .iter()
            .map(|n| opt(*n, *n)),
    );
    items.push((
        field(
            tk,
            &t("chainAnalysis:network"),
            select(
                app,
                tk,
                layer,
                "q-network",
                &t("chainAnalysis:network"),
                q.network.as_deref().unwrap_or(""),
                nets,
                true,
                32.0,
                |a: &mut App, v: &str| st(a).query.network = (!v.is_empty()).then(|| v.to_owned()),
            ),
        ),
        1,
    ));
    for (k, label) in [
        ("q.depth", "depth"),
        ("q.node_limit", "nodeLimit"),
        ("q.edge_limit", "edgeLimit"),
    ] {
        items.push((
            field(
                tk,
                &t(&format!("chainAnalysis:{label}")),
                input(
                    tk,
                    app,
                    k,
                    &t(&format!("chainAnalysis:{label}")),
                    "",
                    false,
                    32.0,
                    true,
                    Some(super::on_filters),
                ),
            ),
            1,
        ));
    }
    items.push((
        field(
            tk,
            &t("chainAnalysis:minAmount"),
            input(
                tk,
                app,
                "q.min",
                &t("chainAnalysis:minAmount"),
                "",
                true,
                32.0,
                true,
                Some(super::on_filters),
            ),
        ),
        1,
    ));
    for (k, label) in [("q.start", "start"), ("q.end", "end")] {
        items.push((
            field(
                tk,
                &t(&format!("chainAnalysis:{label}")),
                input(
                    tk,
                    app,
                    k,
                    &t(&format!("chainAnalysis:{label}")),
                    "YYYY-MM-DDTHH:MM",
                    false,
                    32.0,
                    true,
                    Some(super::on_filters),
                ),
            ),
            1,
        ));
    }
    let check = |k: &str, on: bool, label: String, f: fn(&mut App)| {
        row()
            .gap(8.0)
            .center_y()
            .child(crate::settings::controls::checkbox(tk, k, on, &label, f))
            .child(text(label).font(tk.txs(400.0)).color(tk.fg))
    };
    c.child(grid(cols, 12.0, items)).child(
        row()
            .mt(12.0)
            .gap(20.0)
            .flex_wrap()
            .child(check(
                "q-relations",
                q.include_relations,
                t("chainAnalysis:relations"),
                |a| {
                    let q = &mut st(a).query;
                    q.include_relations = !q.include_relations;
                },
            ))
            .child(check(
                "q-hypotheses",
                q.include_hypotheses,
                t("chainAnalysis:hypotheses"),
                |a| {
                    let q = &mut st(a).query;
                    q.include_hypotheses = !q.include_hypotheses;
                },
            )),
    )
}

/// The Table view: kind, identity, amount and network per node.
fn table(app: &App, tk: &Tk, r: &model::AResult) -> El {
    let hide = app.prefs.hide_sensitive();
    let sel = app.provenance.chain.sel.as_ref();
    use crate::ui::table as tb;
    let cell = |e: El, grow: f32| e.w(0.0).grow(grow).min_w(0.0);
    // The table kit's compact header (the cells are `text-xs`).
    let head = |s: String| {
        row()
            .h(tk.r(tb::HEAD_H_COMPACT))
            .px(tk.r(tb::PX))
            .center_y()
            .child(tb::th_text_compact(tk, s))
    };
    let pad = tk.r(tb::PX);
    let mut rows = col().w_full();
    for n in &r.nodes {
        let selected = sel.is_some_and(|x| !x.edge && x.id == n.id);
        let id = n.id.clone();
        let ident = n.identity().to_owned();
        rows = rows.child(
            tb::tr(tk, false)
                .items_start()
                .fill(if selected {
                    tk.muted.alpha(0.6)
                } else {
                    Color::TRANSPARENT
                })
                .child(cell(
                    text(n.kind.clone())
                        .font(tk.txs(400.0))
                        .color(tk.fg)
                        .padding(pad),
                    0.45,
                ))
                .child(cell(
                    col().padding(pad).child(
                        kit::underlined(&ident, tk.txs(400.0).mono(), tk.fg)
                            .wrap_anywhere()
                            .key(format!("ca-row-{}", n.id))
                            .role("button")
                            .label(ident.clone())
                            .focusable(true)
                            .on_press(move |a: &mut App| super::select(a, Sel::node(id.clone()))),
                    ),
                    6.6,
                ))
                .child(cell(
                    col().padding(pad).items_end().child(sensitive(
                        model::format_amount(n.amount_msat.as_deref(), n.asset.as_deref()),
                        tk.txs(400.0).features("'tnum' 1"),
                        tk.fg,
                        hide,
                    )),
                    1.6,
                ))
                .child(cell(
                    tb::cell_text(format!("{} / {}", n.chain, n.network), tk.txs(400.0), tk.fg)
                        .padding(pad),
                    1.3,
                )),
        );
    }
    col()
        .w_full()
        .max_h(520.0)
        .child(
            tb::head(tk)
                .child(cell(head(t("chainAnalysis:kind")), 0.45))
                .child(cell(head(t("chainAnalysis:id")), 6.6))
                .child(cell(head(t("chainAnalysis:amount")).justify_end(), 1.6))
                .child(cell(head(t("chainAnalysis:network")), 1.3)),
        )
        .child(
            col()
                .w_full()
                .max_h(520.0 - 40.0)
                .child(rows)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .key("ca-table"),
        )
}
