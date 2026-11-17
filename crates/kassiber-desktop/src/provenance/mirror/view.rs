//! The page (`PrivacyMirrorPayloadView`, `ExposureFinding`, `FindingList`, `ScreenSkeleton`,
//! `ScreenNotice`). Tailwind spacing maps to rem through `tk.r`, as everywhere in the app.

use rune::{Color, Cx, El, col, el, row, text};
use serde_json::Value;

use super::{ask, finding_tab, group, headline, investigate, is_open, personal, refresh, toggle};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::overview::grid::stat::{self, Stat};
use crate::provenance::chain::model::Workspace;
use crate::provenance::chain::results::t_or;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Variant};
use crate::ui::table::{self as tb, Tone};
use crate::ui::{ic, surface};

fn readable(code: &str) -> String {
    code.replace('_', " ")
}

/// `formatUiNumber`: grouped in the UI language.
fn num(n: u64) -> String {
    crate::format::count_in(n as usize, crate::i18n::lang())
}

/// `.kb-surface-inset`: the small-radius tile nested inside a card.
fn inset(tk: &Tk) -> El {
    col()
        .w_full()
        .radius(tk.r(0.5))
        .fill(tk.card)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
}

/// `ul.list-disc.pl-4.space-y-1.text-xs.text-muted-foreground`: the disc hangs in the padding.
fn bullets(tk: &Tk, items: Vec<String>) -> El {
    let small = tk.txs(400.0);
    col()
        .w_full()
        .gap(tk.r(0.25))
        .children(items.into_iter().map(|s| {
            row()
                .w_full()
                .items_start()
                .child(
                    row()
                        .w(tk.r(1.0))
                        .justify_end()
                        .pr(tk.r(0.375))
                        .child(text("•").font(small).color(tk.muted_fg))
                        .a11y_hidden(),
                )
                .child(
                    text(s)
                        .font(small)
                        .color(tk.muted_fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                )
        }))
}

/// The amber of a warning (the kit's warning ink).
fn warn_ink(tk: &Tk) -> Color {
    tb::tone(tk, Tone::Warning).1
}

/// The chevron at the end of a disclosure's summary.
fn chevron(tk: &Tk, open: bool) -> El {
    ic(
        if open {
            &I::CHEVRON_DOWN
        } else {
            &I::CHEVRON_RIGHT
        },
        tk.r(1.0),
        tk.muted_fg,
    )
    .a11y_hidden()
}

/// A disclosure's summary row (`pm-<k>-summary`): `content`, then the chevron.
fn disclosure(tk: &Tk, k: &str, open: bool, label: String, content: El, pad: [f32; 2]) -> El {
    let key = k.to_owned();
    row()
        .key(format!("pm-{k}-summary"))
        .w_full()
        .min_w(0.0)
        .px(pad[0])
        .py(pad[1])
        .gap(tk.r(0.75))
        .center_y()
        .role("button")
        .label(label)
        .expanded(open)
        .focusable(true)
        .hover(tk.muted.alpha(0.35))
        .focus_highlight(tk.muted.alpha(0.35))
        .on_press(move |a: &mut App| toggle(a, &key))
        .child(content.grow(1.0).min_w(0.0))
        .child(chevron(tk, open))
}

/// The glyph of a finding's category.
fn category_glyph(category: &str) -> &'static rune::Icon {
    match category {
        "linkage" => &I::LINK_2,
        "structure" => &I::WORKFLOW,
        "pattern" => &I::ACTIVITY,
        "attribution" => &I::TAG,
        _ => &I::EYE,
    }
}

fn is_context(f: &Value) -> bool {
    matches!(
        f.get("relevance").and_then(Value::as_str),
        Some("received_context" | "nearby_context")
    )
}

/// A personal finding the analysis rates a warning. Severity shows as the leading tile's tone
/// and glyph; the kind of evidence is the chip's words.
fn is_warning(f: &Value) -> bool {
    !is_context(f) && f.get("severity").and_then(Value::as_str) == Some("warning")
}

fn statements(f: &Value) -> Vec<String> {
    ["assumptions", "limitations"]
        .iter()
        .flat_map(|k| {
            f.get(*k)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|s| {
            s.as_str()
                .map(|s| t_or(&format!("privacyMirror:statement.{s}"), s))
        })
        .collect()
}

/// What a finding (or a group of repeats) is: the severity tile, its title over
/// "category · relevance", and the evidence chip.
fn finding_head(tk: &Tk, f: &Value, warn: bool, count: Option<String>) -> (String, El) {
    let g = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let (code, relevance, authority, category) =
        (g("code"), g("relevance"), g("authority"), g("category"));
    let title = t_or(&format!("privacyMirror:findingTitle.{code}"), &g("title"));
    let small = tk.txs(400.0);
    let lead = if warn {
        tb::lead_tile(tk, &I::TRIANGLE_ALERT, Tone::Warning)
    } else {
        tb::lead_tile(tk, category_glyph(&category), Tone::Neutral)
    };
    let mut name = row().w_full().min_w(0.0).gap(tk.r(0.5)).center_y().child(
        text(title.clone())
            .font(tk.tsm(600.0))
            .color(tk.fg)
            .wrap()
            .min_w(0.0)
            .role("heading")
            .level(3),
    );
    if let Some(c) = count {
        name = name.child(
            text(c)
                .font(small.features("'tnum' 1"))
                .color(tk.muted_fg)
                .single_line(),
        );
    }
    let meta = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .center_y()
        .flex_wrap()
        .child(
            text(t(&format!("privacyMirror:category.{category}")))
                .font(small)
                .color(tk.muted_fg),
        )
        .child(text("·").font(small).color(tk.muted_fg).a11y_hidden())
        .child(
            text(t(&format!("privacyMirror:relevance.{relevance}")))
                .font(small)
                .color(tk.muted_fg),
        );
    let head = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.75))
        .center_y()
        .child(lead)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.125))
                .child(name)
                .child(meta),
        )
        .child(tb::chip(
            tk,
            Tone::Neutral,
            None,
            t_or(
                &format!("privacyMirror:authority.{authority}"),
                &readable(&authority),
            ),
        ));
    (title, head)
}

/// What the finding says, once per card.
fn finding_detail(tk: &Tk, f: &Value) -> El {
    let g = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    text(t_or(
        &format!("privacyMirror:findingDetail.{}", g("code")),
        &g("detail"),
    ))
    .font(tk.tsm(400.0))
    .color(tk.muted_fg)
    .wrap()
    .max_w(tk.r(56.0))
}

/// One finding's own line: what it affects, its one visible action (Investigate), and — folded
/// behind the line — its assumptions and limits and the assistant's explanation.
fn finding_row(app: &App, tk: &Tk, f: &Value, explain: bool, index: Option<usize>) -> El {
    let id = f.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
    let category = f
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let small = tk.txs(400.0);
    let affected = f
        .get("affected_output_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let basis = statements(f);
    let k = format!("basis-{id}");
    let foldable = !basis.is_empty() || explain;
    let open = foldable && is_open(app, &k);
    let mut what = row().min_w(0.0).gap(tk.r(0.5)).center_y();
    if let Some(i) = index {
        what = what.child(
            text(format!("{}", i + 1))
                .font(small.mono())
                .color(tk.muted_fg)
                .w(tk.r(1.5))
                .a11y_hidden(),
        );
    }
    let basis_label = t("privacyMirror:findingBasis");
    let line = if affected > 0 {
        tc("privacyMirror:affectedOutputs", affected as usize, &[])
    } else {
        basis_label.clone()
    };
    what = what.child(
        text(line.clone())
            .font(tk.tsm(400.0))
            .color(if affected > 0 { tk.fg } else { tk.muted_fg })
            .wrap()
            .min_w(0.0),
    );
    let left = if foldable {
        let key = k.clone();
        row()
            .key(format!("pm-{k}-summary"))
            .grow(1.0)
            .min_w(0.0)
            .min_h(tk.r(2.0))
            .px(tk.r(0.5))
            .ml(-tk.r(0.5))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .role("button")
            .label(format!("{line} · {basis_label}"))
            .expanded(open)
            .focusable(true)
            .hover(tk.muted.alpha(0.35))
            .focus_highlight(tk.muted.alpha(0.35))
            .on_press(move |a: &mut App| toggle(a, &key))
            .child(chevron(tk, open))
            .child(what)
    } else {
        row()
            .grow(1.0)
            .min_w(0.0)
            .min_h(tk.r(2.0))
            .center_y()
            .child(what)
    };
    let inv = f.get("investigation").cloned().unwrap_or(Value::Null);
    let tab = finding_tab(&category);
    let inv2 = inv.clone();
    let investigate_btn = w::button_with(
        tk,
        format!("pm-inv-{id}"),
        Variant::Outline,
        w::Size::Sm,
        Some(&I::ARROW_UP_RIGHT),
        Some(tk.r(0.875)),
        &t("privacyMirror:investigate"),
        true,
    )
    .on_press(move |a: &mut App| investigate(a, &inv, Workspace::Graph, tab));
    let mut c = col().w_full().child(
        row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .child(left)
            .child(investigate_btn),
    );
    if open {
        let mut body = col()
            .w_full()
            .pl(tk.r(if index.is_some() { 2.875 } else { 0.875 }))
            .pt(tk.r(0.375))
            .pb(tk.r(0.25))
            .gap(tk.r(0.5));
        if !basis.is_empty() {
            body = body
                .child(text(basis_label).font(tk.txs(500.0)).color(tk.muted_fg))
                .child(bullets(tk, basis));
        }
        if explain {
            let asking = app.provenance.mirror.asking || app.assistant.streaming;
            // Keyed `…-ask`: it sends the projection to the assistant (an AI request), so audits skip it.
            let b = w::button_with(
                tk,
                format!("pm-explain-{id}-ask"),
                Variant::Ghost,
                w::Size::Sm,
                Some(&I::SPARKLES),
                Some(tk.r(0.875)),
                &t("privacyMirror:explain"),
                !asking,
            );
            body = body.child(row().ml(-tk.r(0.5)).child(if asking {
                b
            } else {
                b.on_press(move |a: &mut App| ask(a, Some(&inv2)))
            }));
        }
        c = c.child(body);
    }
    c
}

/// A card's surface: a page card, or the inset tile inside a disclosure.
fn card(tk: &Tk, nested: bool) -> El {
    if nested {
        inset(tk).clip_rounded(tk.r(0.5))
    } else {
        surface(tk).w_full().clip_rounded(tk.r(0.75))
    }
}

/// `ExposureFinding`: one finding as a card.
fn finding(app: &App, tk: &Tk, f: &Value, nested: bool, explain: bool) -> El {
    let (title, head) = finding_head(tk, f, is_warning(f), None);
    card(tk, nested)
        .padding(tk.r(1.0))
        .gap(tk.r(0.625))
        .role("article")
        .label(title)
        .child(head)
        .child(finding_detail(tk, f).pl(tk.r(2.25)))
        .child(finding_row(app, tk, f, explain, None).pl(tk.r(2.25)))
}

/// `FindingList`: repeated rows fold into one disclosure; evidence and counts stay per finding.
fn finding_list(app: &App, tk: &Tk, findings: &[Value], nested: bool, explain: bool) -> Vec<El> {
    group(findings)
        .into_iter()
        .map(|g| {
            if g.len() == 1 {
                return finding(app, tk, &g[0], nested, explain);
            }
            let first = &g[0];
            let k = format!(
                "group-{}",
                first.get("id").and_then(Value::as_str).unwrap_or("")
            );
            let open = is_open(app, &k);
            let count = tc("privacyMirror:repeatedFindings", g.len(), &[]);
            let (title, head) =
                finding_head(tk, first, g.iter().any(is_warning), Some(count.clone()));
            let mut d = card(tk, nested)
                .role("article")
                .label(title.clone())
                .child(disclosure(
                    tk,
                    &k,
                    open,
                    format!("{title} {count}"),
                    head,
                    [tk.r(1.0), tk.r(0.75)],
                ));
            if open {
                let mut body = col()
                    .w_full()
                    .px(tk.r(1.0))
                    .pt(tk.r(0.75))
                    .pb(tk.r(0.5))
                    .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                    .border_layout()
                    .child(finding_detail(tk, first).pb(tk.r(0.5)));
                for (i, f) in g.iter().enumerate() {
                    body = body.child(
                        finding_row(app, tk, f, explain, Some(i))
                            .py(tk.r(0.25))
                            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
                            .border_layout(),
                    );
                }
                d = d.child(body);
            }
            d
        })
        .collect()
}

/// The body under a disclosure's summary.
fn disclosure_body(tk: &Tk) -> El {
    col()
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
}

/// `screenShellClassName` on the app's spacing scale (`Tk::page_pad`, `Tk::section_gap`).
fn shell(tk: &Tk) -> El {
    col()
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background)
}

/// The page header: whose view this is, what the read is doing, and the page's actions.
fn header(app: &App, tk: &Tk, investigation: Option<&Value>) -> El {
    let s = &app.provenance.mirror;
    let mut left = row()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .center_y()
        .flex_wrap()
        .child(ic(&I::EYE, tk.r(0.875), tk.muted_fg).a11y_hidden())
        .child(
            text(t("privacyMirror:observer"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        );
    if s.loading {
        left = left.child(progress(app, tk).ml(tk.r(0.5)));
    }
    let mut actions = row().gap(tk.r(0.5)).center_y();
    if let Some(investigation) = investigation {
        let asking = s.asking || app.assistant.streaming;
        if app.prefs.ai_features {
            let b = w::button_with(
                tk,
                "pm-ask",
                Variant::Outline,
                w::Size::Sm,
                Some(&I::SPARKLES),
                Some(tk.r(0.875)),
                &t("privacyMirror:ask"),
                !asking,
            );
            actions = actions.child(if asking {
                b
            } else {
                b.on_press(|a: &mut App| ask(a, None))
            });
        }
        let inv = investigation.clone();
        actions = actions.child(
            w::button_with(
                tk,
                "pm-workbench",
                Variant::Outline,
                w::Size::Sm,
                Some(&I::ARROW_UP_RIGHT),
                Some(tk.r(0.875)),
                &t("privacyMirror:workbench"),
                true,
            )
            .on_press(move |a: &mut App| investigate(a, &inv, Workspace::Graph, "findings")),
        );
    }
    actions = actions.child(refresh_button(tk, s.loading));
    row()
        .w_full()
        .min_h(tk.r(2.0))
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .flex_wrap()
        .child(left)
        .child(actions)
}

/// What the read is doing: the local analysis runs for seconds, so the line says so and counts.
fn progress(app: &App, tk: &Tk) -> El {
    let s = &app.provenance.mirror;
    let secs = s
        .started
        .map(|at| (app.clock - at).max(0.0).floor() as u64)
        .unwrap_or(0);
    let label = t("chainAnalysis:running");
    let mut r = row()
        .key("pm-progress")
        .gap(tk.r(0.375))
        .center_y()
        .role("status")
        .label(label.clone())
        .child(w::spinner(tk, tk.r(0.875)).a11y_hidden())
        .child(text(label).font(tk.tsm(400.0)).color(tk.muted_fg));
    if secs >= 2 {
        r = r.child(
            text(format!("{secs} s"))
                .font(tk.txs(400.0).features("'tnum' 1"))
                .color(tk.muted_fg)
                .a11y_hidden(),
        );
    }
    r
}

/// A failed first read (or a report this build cannot show): the centred state the app's
/// empty pages use, with the daemon's message and the refresh the title asks for.
fn notice(app: &App, tk: &Tk, body: String) -> El {
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let free = (win_h - crate::shell::window_top_inset() - tk.r(10.0)).max(tk.r(14.0));
    let ink = warn_ink(tk);
    shell(tk).key("pm-page").child(header(app, tk, None)).child(
        col()
            .key("pm-unavailable")
            .w_full()
            .min_h(free)
            .center()
            .role("alert")
            .child(
                col()
                    .w_full()
                    .max_w(tk.r(32.0))
                    .items_center()
                    .gap(tk.r(0.5))
                    .child(
                        row()
                            .size(tk.r(3.0), tk.r(3.0))
                            .radius(tk.r(1.5))
                            .center()
                            .fill(ink.alpha(0.12))
                            .mb(tk.r(0.5))
                            .child(ic(&I::TRIANGLE_ALERT, tk.r(1.5), ink))
                            .a11y_hidden(),
                    )
                    .child(
                        text(t("privacyMirror:unavailable.title"))
                            .font(tk.tbase(600.0))
                            .color(tk.fg)
                            .align(rune::Align::Center)
                            .role("heading")
                            .level(2),
                    )
                    .child(
                        text(body)
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .wrap()
                            .align(rune::Align::Center),
                    ),
            ),
    )
}

/// A skeleton bar.
fn bone(tk: &Tk, w: f32, h: f32) -> El {
    el().w(w).h(h).radius(tk.r(0.375)).fill(tk.muted.alpha(0.7))
}

/// The first read: the page's own layout with bars where the answer will be (the headline
/// card, the count tiles, the findings, the folded sections), under the live header.
fn skeleton(app: &App, cx: &Cx<App>, tk: &Tk) -> El {
    let folded = |w: f32| {
        surface(tk).w_full().px(tk.r(1.0)).py(tk.r(0.875)).child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .child(bone(tk, tk.r(w), tk.r(0.875)))
                .child(bone(tk, tk.r(1.0), tk.r(1.0))),
        )
    };
    let finding = |w: f32| {
        surface(tk).w_full().px(tk.r(1.0)).py(tk.r(0.75)).child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .center_y()
                .child(bone(tk, tk.r(1.5), tk.r(1.5)))
                .child(
                    col()
                        .grow(1.0)
                        .gap(tk.r(0.375))
                        .child(bone(tk, tk.r(w), tk.r(0.875)))
                        .child(bone(tk, tk.r(w * 0.7), tk.r(0.625))),
                )
                .child(bone(tk, tk.r(6.0), tk.r(1.25))),
        )
    };
    let tiles = stat::row_of(
        app,
        cx,
        tk,
        "pm-stats",
        t("privacyMirror:coverage.title"),
        vec![
            Stat::loading(
                "pm-stat-attention",
                &I::FINGERPRINT,
                t("privacyMirror:groups.attention"),
            ),
            Stat::loading(
                "pm-stat-context",
                &I::NETWORK,
                t("privacyMirror:groups.context"),
            ),
            Stat::loading(
                "pm-stat-examined",
                &I::LIST_CHECKS,
                t("privacyMirror:coverage.title"),
            ),
        ],
    );
    shell(tk).key("pm-page").child(header(app, tk, None)).child(
        col()
            .key("pm-loading")
            .w_full()
            .gap(tk.r(0.75))
            .busy(true)
            .label(t("chainAnalysis:running"))
            .child(
                surface(tk)
                    .w_full()
                    .padding(tk.card_pad())
                    .gap(tk.r(0.625))
                    .child(
                        row()
                            .w_full()
                            .justify_between()
                            .child(bone(tk, tk.r(16.0), tk.r(1.25)))
                            .child(bone(tk, tk.r(8.0), tk.r(1.25))),
                    )
                    .child(bone(tk, tk.r(22.0), tk.r(0.75))),
            )
            .child(tiles)
            .child(bone(tk, tk.r(7.0), tk.r(0.875)).mt(tk.r(0.5)))
            .child(finding(13.0))
            .child(finding(10.0))
            .child(folded(12.0))
            .child(folded(9.0)),
    )
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, _layer: &mut Vec<El>) -> El {
    let s = &app.provenance.mirror;
    let Some(p) = s.payload.as_ref() else {
        if s.loading || s.mounted_for.is_none() {
            return skeleton(app, cx, tk);
        }
        return notice(
            app,
            tk,
            s.error
                .clone()
                .unwrap_or_else(|| t("privacyMirror:unavailable.body")),
        );
    };
    if p.get("payload_schema_version").and_then(Value::as_u64) != Some(2) {
        return notice(
            app,
            tk,
            s.error
                .clone()
                .unwrap_or_else(|| t("privacyMirror:unavailable.body")),
        );
    }
    payload_view(app, cx, tk, p)
}

/// A count's noun, from the counted phrase ("281 owned outputs" → "Owned outputs").
fn noun(phrase: &str, count: &str) -> String {
    let rest = phrase.replacen(count, "", 1);
    let rest = rest.trim();
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Opens disclosure `k` (a tile's press) and brings it into view.
fn open_section(a: &mut App, k: &str, reveal: &str) {
    let s = &mut a.provenance.mirror;
    s.open.insert(k.to_owned());
    s.reveal = Some(reveal.to_owned());
}

fn payload_view(app: &App, cx: &Cx<App>, tk: &Tk, p: &Value) -> El {
    let s = &app.provenance.mirror;
    let summary = p.get("summary").cloned().unwrap_or(Value::Null);
    let coverage = p.get("coverage").cloned().unwrap_or(Value::Null);
    let findings: Vec<Value> = p
        .get("findings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let head = headline(&summary, &findings);
    let attention: Vec<Value> = findings.iter().filter(|f| personal(f)).cloned().collect();
    let context: Vec<Value> = findings.iter().filter(|f| !personal(f)).cloned().collect();
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let small = tk.txs(400.0);
    let investigation = p.get("investigation").cloned().unwrap_or(Value::Null);

    let mut page = shell(tk)
        .key("pm-page")
        .child(header(app, tk, Some(&investigation)));
    if let Some(e) = &s.error {
        let (bg, _, ring) = tb::tone(tk, Tone::Warning);
        page = page.child(
            row()
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.625))
                .items_start()
                .radius(tk.r(0.5))
                .border(1.0, ring)
                .border_layout()
                .fill(bg)
                .role("alert")
                .child(
                    ic(&I::TRIANGLE_ALERT, tk.r(1.0), warn_ink(tk))
                        .mt(tk.r(0.125))
                        .a11y_hidden(),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.125))
                        .child(
                            text(t("privacyMirror:refreshFailed"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg)
                                .wrap(),
                        )
                        .child(
                            text(e.clone())
                                .font(tk.tsm(400.0))
                                .color(tk.muted_fg)
                                .wrap(),
                        ),
                ),
        );
    }
    if let Some(e) = &s.action_error {
        page = page.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .role("alert"),
        );
    }

    // The answer: one headline, how far the local evidence reaches, and what limits it.
    let status = coverage
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    let title = t(&format!("privacyMirror:headline.{head}"));
    let status_tone = if status == "complete" {
        Tone::Neutral
    } else {
        Tone::Warning
    };
    let mut card = surface(tk).w_full().padding(tk.card_pad()).child(
        row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .justify_between()
            .flex_wrap()
            .child(
                text(title.clone())
                    .font(tk.tlg(600.0))
                    .color(tk.fg)
                    .wrap()
                    .role("heading")
                    .level(2)
                    .min_w(0.0),
            )
            .child(tb::chip(
                tk,
                status_tone,
                None,
                t(&format!("privacyMirror:coverage.status.{status}")),
            )),
    );
    // Only headlines that need a caveat get a body; the others would repeat themselves.
    if head == "nothing_found" || head == "unavailable" {
        card = card.child(
            text(t(&format!("privacyMirror:headlineBody.{head}")))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .mt(tk.r(0.25))
                .max_w(tk.r(48.0)),
        );
    }
    let outputs = n(&summary, "owned_output_count");
    let (examined, available) = (
        n(&summary, "analyzed_transaction_count"),
        n(&summary, "local_transaction_count"),
    );
    let outputs_line = tc("privacyMirror:population.outputs", outputs as usize, &[]);
    let examined_line = tr(
        "privacyMirror:population.transactions",
        &[("examined", &num(examined)), ("available", &num(available))],
    );
    // The tiles below carry the population; without them (nothing local) the line says it.
    if available == 0 {
        card = card.child(
            text(format!("{outputs_line} · {examined_line}"))
                .font(small.features("'tnum' 1"))
                .color(tk.muted_fg)
                .mt(tk.r(0.5))
                .wrap(),
        );
    }
    let gaps = n(&coverage, "missing_nodes") > 0
        || n(&coverage, "stale_nodes") > 0
        || n(&coverage, "conflicting_nodes") > 0;
    let truncated = coverage
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut limits = String::new();
    if gaps || truncated {
        if gaps {
            limits = tr(
                "privacyMirror:coverage.gaps",
                &[
                    ("missing", &n(&coverage, "missing_nodes").to_string()),
                    ("stale", &n(&coverage, "stale_nodes").to_string()),
                    (
                        "conflicting",
                        &n(&coverage, "conflicting_nodes").to_string(),
                    ),
                ],
            );
        }
        if truncated {
            if gaps {
                limits.push_str(" · ");
            }
            limits.push_str(&t("privacyMirror:coverage.truncated"));
        }
        card = card.child(
            row()
                .w_full()
                .mt(tk.r(0.5))
                .gap(tk.r(0.375))
                .items_start()
                .child(
                    ic(&I::TRIANGLE_ALERT, tk.r(0.875), warn_ink(tk))
                        .mt(tk.r(0.0625))
                        .a11y_hidden(),
                )
                .child(
                    text(limits.clone())
                        .font(small)
                        .color(tk.muted_fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    if head == "no_local_evidence" || head == "no_owned_outputs" {
        card = card.child(
            row().mt(tk.r(0.75)).child(
                w::button(
                    tk,
                    "pm-connect",
                    Variant::Outline,
                    w::Size::Sm,
                    None,
                    &t("privacyMirror:connect"),
                    true,
                )
                .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.875), tk.fg))
                .on_press(|a: &mut App| crate::shell::go(a, "/connections")),
            ),
        );
    }
    page = page.child(card.key("pm-summary").role("region").label(title));

    // The counts at a glance. A tile is drawn for what the report holds; a risk tile only
    // when there is one.
    if available > 0 {
        let mut tiles: Vec<Stat> = vec![];
        if !attention.is_empty() {
            let groups = group(&attention);
            let first = &groups[0][0];
            let mut line = t_or(
                &format!(
                    "privacyMirror:findingTitle.{}",
                    first.get("code").and_then(Value::as_str).unwrap_or("")
                ),
                first.get("title").and_then(Value::as_str).unwrap_or(""),
            );
            if groups.len() > 1 {
                line.push_str(&format!(" +{}", groups.len() - 1));
            }
            let mut tile = Stat::new(
                "pm-stat-attention",
                &I::FINGERPRINT,
                t("privacyMirror:groups.attention"),
                num(attention.len() as u64),
            )
            .line(line)
            .button(t("privacyMirror:groups.attention"), |a: &mut App| {
                a.provenance.mirror.reveal = Some("pm-attention".into())
            });
            if attention.iter().any(is_warning) {
                tile = tile.ink(warn_ink(tk)).glyph_ink(warn_ink(tk));
            }
            tiles.push(tile);
        }
        if !context.is_empty() {
            tiles.push(
                Stat::new(
                    "pm-stat-context",
                    &I::NETWORK,
                    t("privacyMirror:groups.context"),
                    num(context.len() as u64),
                )
                .button(t("privacyMirror:groups.context"), |a: &mut App| {
                    open_section(a, "context", "pm-context")
                }),
            );
        }
        tiles.push(
            Stat::new(
                "pm-stat-outputs",
                &I::COINS,
                noun(&outputs_line, &outputs.to_string()),
                num(outputs),
            )
            .line(tc(
                "privacyMirror:population.domains",
                n(&summary, "domain_count") as usize,
                &[],
            )),
        );
        tiles.push(
            Stat::new(
                "pm-stat-examined",
                &I::LIST_CHECKS,
                t("privacyMirror:coverage.title"),
                format!("{} / {}", num(examined), num(available)),
            )
            .button(t("privacyMirror:coverage.title"), |a: &mut App| {
                open_section(a, "coverage", "pm-coverage")
            }),
        );
        if n(&coverage, "missing_nodes") > 0 {
            tiles.push(
                Stat::new(
                    "pm-stat-missing",
                    &I::TRIANGLE_ALERT,
                    t("chainAnalysis:coverageMissing"),
                    num(n(&coverage, "missing_nodes")),
                )
                .ink(warn_ink(tk))
                .glyph_ink(warn_ink(tk))
                .line(t(&format!("privacyMirror:coverage.status.{status}")))
                .button(t("chainAnalysis:coverageMissing"), |a: &mut App| {
                    open_section(a, "coverage", "pm-coverage")
                }),
            );
        }
        page = page.child(stat::row_of(
            app,
            cx,
            tk,
            "pm-stats",
            t("privacyMirror:coverage.title"),
            tiles,
        ));
    }

    if !attention.is_empty() {
        let heading = row()
            .mt(tk.r(0.5))
            .gap(tk.r(0.375))
            .center_y()
            .role("heading")
            .level(2)
            .label(format!(
                "{} {}",
                t("privacyMirror:groups.attention"),
                attention.len()
            ))
            .child(
                text(t("privacyMirror:groups.attention"))
                    .font(tk.tsm(600.0))
                    .color(tk.fg),
            )
            .child(
                text(attention.len().to_string())
                    .font(tk.txs(500.0).features("'tnum' 1"))
                    .color(tk.muted_fg),
            );
        page = page.child(
            col()
                .key("pm-attention")
                .w_full()
                .gap(tk.r(0.5))
                .child(heading)
                .children(finding_list(
                    app,
                    tk,
                    &attention,
                    false,
                    app.prefs.ai_features,
                )),
        );
    }
    if !context.is_empty() {
        let open = is_open(app, "context");
        let label = format!(
            "{} {} {}",
            t("privacyMirror:groups.context"),
            context.len(),
            t("privacyMirror:groups.contextNote")
        );
        let content = row()
            .gap(tk.r(0.75))
            .center_y()
            .child(tb::lead_tile(tk, &I::NETWORK, Tone::Neutral))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(tk.r(0.125))
                    .child(
                        row()
                            .gap(tk.r(0.375))
                            .center_y()
                            .child(
                                text(t("privacyMirror:groups.context"))
                                    .font(tk.tsm(600.0))
                                    .color(tk.fg),
                            )
                            .child(
                                text(context.len().to_string())
                                    .font(tk.txs(500.0).features("'tnum' 1"))
                                    .color(tk.muted_fg),
                            ),
                    )
                    .child(
                        text(t("privacyMirror:groups.contextNote"))
                            .font(small)
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
            );
        let mut d = surface(tk)
            .key("pm-context")
            .w_full()
            .clip_rounded(tk.r(0.75))
            .child(disclosure(
                tk,
                "context",
                open,
                label,
                content,
                [tk.r(1.0), tk.r(0.75)],
            ));
        if open {
            d = d.child(disclosure_body(tk).children(finding_list(app, tk, &context, true, false)));
        }
        page = page.child(d);
    }
    page = page.child(coverage_section(app, tk, &summary, &coverage, p));

    // Footer: the workbench tools that live elsewhere (`Button variant="link" size="sm"`).
    let inv_psbt = investigation.clone();
    let inv_ds = investigation;
    let link = |k: &str, glyph: &'static rune::Icon, label: String| {
        w::button_with(
            tk,
            k.to_owned(),
            Variant::Ghost,
            w::Size::Sm,
            Some(glyph),
            Some(tk.r(0.875)),
            &label,
            true,
        )
        .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.75), tk.muted_fg).a11y_hidden())
    };
    page.child(
        row()
            .key("pm-footer")
            .w_full()
            .pb(tk.r(0.5))
            .gap(tk.r(0.5))
            .flex_wrap()
            .center_y()
            .child(
                text(t("privacyMirror:toolsElsewhere"))
                    .font(small)
                    .color(tk.muted_fg)
                    .mr(tk.r(0.25)),
            )
            .child(
                link("pm-psbt", &I::FILE_SEARCH, t("privacyMirror:checkSpend")).on_press(
                    move |a: &mut App| investigate(a, &inv_psbt, Workspace::Psbt, "findings"),
                ),
            )
            .child(
                link("pm-datasets", &I::TAGS, t("privacyMirror:datasets")).on_press(
                    move |a: &mut App| investigate(a, &inv_ds, Workspace::Datasets, "findings"),
                ),
            ),
    )
}

/// `Button variant="ghost" size="icon-sm"` with `RefreshCw` (`animate-spin` while fetching).
fn refresh_button(tk: &Tk, refreshing: bool) -> El {
    let label = t("privacyMirror:refresh");
    let mut glyph = ic(&I::REFRESH_CW, tk.r(1.0), tk.muted_fg);
    if refreshing {
        glyph = glyph.looping(rune::LoopSpec::rotation(1.0));
    }
    let b = w::button_surface(
        tk,
        row().key("pm-refresh").size(tk.r(2.0), tk.r(2.0)).center(),
        Variant::Ghost,
    )
    .role("button")
    .label(label.clone())
    .help(label)
    .child(glyph);
    if refreshing {
        b.disabled(true).busy(true).opacity(0.5)
    } else {
        b.focusable(true).on_press(refresh)
    }
}

/// A sub-disclosure inside "What was examined".
fn sub_disclosure(tk: &Tk, k: &str, open: bool, label: String) -> El {
    col()
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(disclosure(
            tk,
            k,
            open,
            label.clone(),
            text(label).font(tk.tsm(500.0)).color(tk.fg),
            [tk.r(1.0), tk.r(0.625)],
        ))
}

/// "What was examined": population, the per-check coverage, stop reasons, the conditional
/// interpretations and the report's assumptions.
fn coverage_section(app: &App, tk: &Tk, summary: &Value, coverage: &Value, p: &Value) -> El {
    let open = is_open(app, "coverage");
    let small = tk.txs(400.0);
    let label = t("privacyMirror:coverage.title");
    let domains = summary
        .get("domain_count")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let content = row()
        .gap(tk.r(0.75))
        .center_y()
        .child(tb::lead_tile(tk, &I::LIST_CHECKS, Tone::Neutral))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.125))
                .child(text(label.clone()).font(tk.tsm(600.0)).color(tk.fg))
                .child(
                    text(tc("privacyMirror:population.domains", domains, &[]))
                        .font(small)
                        .color(tk.muted_fg),
                ),
        );
    let mut d = surface(tk)
        .key("pm-coverage")
        .w_full()
        .clip_rounded(tk.r(0.75))
        .child(disclosure(
            tk,
            "coverage",
            open,
            label,
            content,
            [tk.r(1.0), tk.r(0.75)],
        ));
    if !open {
        return d;
    }
    // The checks, on the table kit: what ran, how much of it, and a chip only where it fell short.
    let checks = coverage
        .get("checks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut list = col()
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .role("table")
        .label(t("privacyMirror:coverage.title"));
    for (i, c) in checks.iter().enumerate() {
        let code = c.get("code").and_then(Value::as_str).unwrap_or("");
        let status = c.get("status").and_then(Value::as_str).unwrap_or("");
        let mut left = col().min_w(0.0).grow(1.0).gap(tk.r(0.125)).child(
            text(t_or(
                &format!("privacyMirror:checks.{code}"),
                &readable(code),
            ))
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap(),
        );
        if let Some(r) = c.get("reason").and_then(Value::as_str) {
            left = left.child(
                text(t_or(&format!("privacyMirror:reason.{r}"), &readable(r)))
                    .font(small)
                    .color(tk.muted_fg)
                    .wrap(),
            );
        }
        let count = format!(
            "{} / {}",
            num(c.get("evaluated").and_then(Value::as_u64).unwrap_or(0)),
            num(c.get("eligible").and_then(Value::as_u64).unwrap_or(0))
        );
        let status_label = t(&format!("privacyMirror:checkStatus.{status}"));
        let state = match status {
            "complete" => text(status_label)
                .font(small)
                .color(tk.muted_fg)
                .single_line(),
            "partial" | "bounded" => tb::chip(tk, Tone::Warning, None, status_label),
            _ => tb::chip(tk, Tone::Neutral, None, status_label),
        };
        let mut r = row()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(tb::PY))
            .gap(tk.r(0.75))
            .center_y()
            .role("row")
            .child(left)
            .child(
                text(count)
                    .font(tk.tsm(500.0).features("'tnum' 1"))
                    .color(tk.fg)
                    .single_line(),
            )
            .child(row().w(tk.r(7.5)).justify_end().child(state));
        if i > 0 {
            r = r
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout();
        }
        list = list.child(r);
    }
    d = d.child(list);
    let reasons: Vec<String> = coverage
        .get("stopped_reasons")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|code| {
                    let k = format!("privacyMirror:reason.{code}");
                    if crate::i18n::has(&k) {
                        t(&k)
                    } else {
                        t_or(
                            &format!(
                                "privacyMirror:reason.{}",
                                code.strip_prefix("entropy_").unwrap_or(code)
                            ),
                            &readable(code),
                        )
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if !reasons.is_empty() {
        d = d.child(
            col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(bullets(tk, reasons)),
        );
    }
    let entropy = p.get("entropy").cloned().unwrap_or(Value::Null);
    let results: Vec<Value> = entropy
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !results.is_empty() {
        let eopen = is_open(app, "entropy");
        let evaluated = entropy
            .get("evaluated")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let mut e = sub_disclosure(
            tk,
            "entropy",
            eopen,
            tc("privacyMirror:entropy.title", evaluated, &[]),
        );
        if eopen {
            let mut body = col().w_full().px(tk.r(1.0)).pb(tk.r(0.75)).child(
                text(t("privacyMirror:entropy.body"))
                    .font(small)
                    .color(tk.muted_fg)
                    .wrap()
                    .pb(tk.r(0.5)),
            );
            for (i, r) in results.iter().enumerate() {
                let inv = r.get("investigation").cloned().unwrap_or(Value::Null);
                body = body.child(
                    row()
                        .w_full()
                        .py(tk.r(0.25))
                        .gap(tk.r(0.75))
                        .center_y()
                        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
                        .border_layout()
                        .child(
                            text(format!("{}", i + 1))
                                .font(small.mono())
                                .color(tk.muted_fg)
                                .w(tk.r(1.5))
                                .a11y_hidden(),
                        )
                        .child(
                            text(entropy_label(r))
                                .font(tk.tsm(400.0))
                                .color(tk.fg)
                                .wrap()
                                .grow(1.0)
                                .min_w(0.0),
                        )
                        .child(
                            w::button(
                                tk,
                                format!("pm-ent-{i}"),
                                Variant::Ghost,
                                w::Size::Sm,
                                None,
                                &t("privacyMirror:entropy.inspect"),
                                true,
                            )
                            .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.75), tk.fg))
                            .on_press(move |a: &mut App| {
                                investigate(a, &inv, Workspace::Graph, "entropy")
                            }),
                        ),
                );
            }
            let omitted = entropy.get("omitted").and_then(Value::as_u64).unwrap_or(0) as usize;
            if omitted > 0 {
                body = body.child(
                    text(tc("privacyMirror:entropy.omitted", omitted, &[]))
                        .font(small)
                        .color(tk.muted_fg)
                        .wrap()
                        .pt(tk.r(0.5)),
                );
            }
            e = e.child(body);
        }
        d = d.child(e);
    }
    let aopen = is_open(app, "assumptions");
    let mut a = sub_disclosure(tk, "assumptions", aopen, t("privacyMirror:assumptions"));
    if aopen {
        let items = p
            .get("assumptions")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| t_or(&format!("privacyMirror:statement.{s}"), s))
                    .collect()
            })
            .unwrap_or_default();
        a = a.child(
            col()
                .w_full()
                .px(tk.r(1.0))
                .pb(tk.r(0.75))
                .child(bullets(tk, items)),
        );
    }
    d.child(a)
}

/// One conditional-interpretation row's text: the exact count, a lower bound when the search
/// stopped early, else the reason (or status) it could not be evaluated.
pub fn entropy_label(r: &Value) -> String {
    let status = r.get("status").and_then(Value::as_str).unwrap_or("");
    let as_text = |k: &str| {
        r.get(k).and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        })
    };
    let (count, lower) = (
        as_text("interpretation_count"),
        as_text("interpretation_count_lower_bound"),
    );
    if status == "exact" && count.is_some() {
        return tr(
            "privacyMirror:entropy.interpretations",
            &[("countText", &count.unwrap_or_default())],
        );
    }
    if ["budget_exhausted", "timeout", "model_bounded"].contains(&status)
        && count.is_none()
        && lower.is_some()
    {
        return tr(
            "privacyMirror:entropy.lowerBound",
            &[("countText", &lower.unwrap_or_default())],
        );
    }
    let code = r
        .get("reason")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(status);
    t_or(&format!("privacyMirror:reason.{code}"), &readable(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entropy_rows_pick_count_bound_or_reason() {
        let exact = entropy_label(&json!({"status": "exact", "interpretation_count": "3"}));
        assert!(exact.contains('3'), "{exact}");
        let bound = entropy_label(
            &json!({"status": "timeout", "interpretation_count": null, "interpretation_count_lower_bound": "40"}),
        );
        assert!(bound.contains("40"), "{bound}");
        // An exact status without a count falls through to the status text (no reason key).
        assert_eq!(entropy_label(&json!({"status": "exact"})), "exact");
        assert_eq!(
            entropy_label(&json!({"status": "unsupported", "reason": "some_new_reason"})),
            "some new reason"
        );
    }
}
