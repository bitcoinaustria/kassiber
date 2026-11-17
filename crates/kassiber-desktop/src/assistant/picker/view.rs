//! What the picker draws: the trigger (ProviderModelPicker + ComposerControl), the popover with
//! the provider rail (ModelPickerSidebar), search, provider header and model rows
//! (ModelPickerContent, ModelListRow), the reasoning column and the footer with speed and the
//! permission-mode menu (ComposerModelTraits), and the provider marks (ProviderGlyph).

use rune::{
    Color, Cx, El, Icon, Key, KeyEvent, Shadow, Side, Tx, col, el, icon, oklch, row, text,
    text_input,
};

use super::brands as B;
use super::data::{self, ModelRow};
use super::strings;
use super::{
    Focus, View, active_view, effort_options, effort_shown, groups, highlighted, items, providers,
    searching, selected,
};
use crate::app::App;
use crate::assistant::PermissionMode;
use crate::assistant::model::Provider;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use crate::ui::popup::TriggerExt;

static STAR_FILLED: Icon = Icon {
    name: "kb-star-filled",
    view: 24.0,
    body: r##"<path d="M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"##,
};

static ZAP_FILLED: Icon = Icon {
    name: "kb-zap-filled",
    view: 24.0,
    body: r##"<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"##,
};

// Tailwind v4 amber: the star, Fast, Full access and a failed check's warning.
fn amber(shade: u16) -> Color {
    match shade {
        400 => oklch(0.828, 0.189, 84.429),
        500 => oklch(0.769, 0.188, 70.08),
        600 => oklch(0.666, 0.179, 58.318),
        _ => oklch(0.555, 0.163, 48.998),
    }
}

/// `ProviderGlyph`: the provider's brand mark, else its initials. No status dot: where prompts
/// go is said in words in the header, and only a failed check marks the rail.
pub fn provider_glyph(tk: &Tk, p: &Provider, size: f32) -> El {
    let ink = if tk.dark { Color::WHITE } else { Color::BLACK };
    let mark: Option<El> = match data::icon_key(p) {
        Some("codex") => Some(icon(&B::OPENAI, size).color(ink)),
        Some("claude") => Some(icon(&B::CLAUDE, size)),
        Some("opencode") => Some(icon(
            if tk.dark {
                &B::OPENCODE_DARK
            } else {
                &B::OPENCODE_LIGHT
            },
            size,
        )),
        Some("copilot") => Some(icon(&B::GITHUB, size).color(ink)),
        Some("ollama") => Some(icon(&B::OLLAMA, size).color(ink)),
        Some("omlx") => Some(icon(&B::OMLX, size).color(ink)),
        _ => None,
    };
    let inner = mark.unwrap_or_else(|| {
        let s = (size * 0.5).max(tk.r(0.5));
        text(data::initials(&p.label()))
            .font(rune::TextStyle::new(s, 600.0).lh(s).tracking(-0.025 * s))
            .color(tk.fg.alpha(0.8))
    });
    row().size(size, size).center().child(inner).a11y_hidden()
}

/// `formatShortcut`: `⌘1` on macOS, `Ctrl+1` elsewhere.
fn shortcut(keys: &[&str]) -> String {
    let mac = cfg!(target_os = "macos");
    let parts: Vec<String> = keys
        .iter()
        .map(|k| match (*k, mac) {
            ("mod", true) => "⌘".into(),
            ("mod", false) => "Ctrl".into(),
            ("shift", true) => "⇧".into(),
            ("shift", false) => "Shift".into(),
            (k, _) => k.to_uppercase(),
        })
        .collect();
    if mac { parts.concat() } else { parts.join("+") }
}

fn permission_icon(m: PermissionMode) -> &'static Icon {
    match m {
        PermissionMode::Approval => &I::LOCK,
        PermissionMode::Auto => &I::SPARKLES,
        PermissionMode::FullAccess => &I::LOCK_OPEN,
    }
}

fn permission_key(m: PermissionMode) -> &'static str {
    m.id()
}

/// The selected model's provider and listed row (its display name, else its id).
fn selection_row(app: &App) -> Option<(Provider, ModelRow)> {
    let (sp, sm) = app.assistant.selection.as_ref()?;
    let g = groups(app);
    let grp = g.into_iter().find(|g| g.provider.name == *sp)?;
    let row = grp
        .models
        .iter()
        .find(|m| m.id == *sm)
        .cloned()
        .unwrap_or_else(|| ModelRow::bare(sm));
    Some((grp.provider, row))
}

// ---- Trigger -----------------------------------------------------------------------------------

/// The quiet trigger (`ComposerControl`: `h-7 rounded-lg px-2 gap-1.5 text-sm font-medium`).
pub fn trigger(app: &App, tk: &Tk, host: &'static str) -> El {
    let a = &app.assistant;
    let open = a.picker.open == Some(host);
    let provs = providers(app);
    let current = a
        .selection
        .as_ref()
        .and_then(|(p, _)| provs.iter().find(|r| r.name == *p).cloned());
    let row_sel = selection_row(app);
    let label = match &a.selection {
        Some((p, m)) => {
            let pl = current
                .as_ref()
                .map(|c| c.label())
                .unwrap_or_else(|| p.clone());
            let ml = row_sel
                .as_ref()
                .and_then(|(_, r)| r.display_name.clone())
                .unwrap_or_else(|| m.clone());
            format!("{pl} · {ml}")
        }
        None if a.providers.ready().is_some_and(|_| provs.is_empty()) => {
            t("assistant:modelPicker.noProviderConfigured")
        }
        None => t("assistant:modelPicker.selectAModel"),
    };
    // Where prompts go, in words (the model's own route when it differs from its provider's).
    let posture_hint = current.as_ref().map(|c| {
        strings::disclosure(
            &row_sel
                .as_ref()
                .and_then(|(_, r)| data::model_where(c, r))
                .unwrap_or_else(|| data::where_prompts_go(c)),
        )
    });
    let show_effort = effort_shown(app);
    let effort_label = (show_effort && a.effort != "auto")
        .then(|| t(&format!("assistant:composer.effort.{}", a.effort)));
    let (sp, sm, _) = selected(app);
    let fast_on = data::fast_mode(sp.as_ref(), sm.as_ref()).0 && a.fast_mode;
    let full = a.permission == PermissionMode::FullAccess;
    let detail: Vec<String> = [
        effort_label
            .as_ref()
            .map(|e| format!("{}: {e}", t("assistant:composer.reasoningEffort"))),
        fast_on.then(|| t("assistant:composer.fastMode.fast")),
        full.then(|| {
            format!(
                "{}: {}",
                t("assistant:composer.permission.label"),
                t("assistant:composer.permission.full_access.label")
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    let detail = detail.join(", ");
    let mut aria = format!("{}: {label}", t("assistant:modelPicker.models"));
    if let Some(h) = &posture_hint {
        aria.push_str(&format!(" ({h})"));
    }
    if !detail.is_empty() {
        aria.push_str(&format!(", {detail}"));
    }
    let mut tip = label.clone();
    if let Some(h) = &posture_hint {
        tip.push_str(&format!(" · {h}"));
    }
    if !detail.is_empty() {
        tip.push_str(&format!(" · {detail}"));
    }
    tip.push_str(&format!("  {}", shortcut(&["mod", "shift", "m"])));
    if show_effort {
        tip.push_str(&format!(
            "  {} {}",
            t("assistant:composer.reasoning"),
            shortcut(&["mod", "shift", "k"])
        ));
    }
    let ink = if open { tk.fg } else { tk.muted_fg };
    let mut r = row()
        .key(format!("asst-picker-{host}"))
        .h(tk.r(1.75))
        .px(tk.r(0.5))
        .gap(tk.r(0.375))
        .center_y()
        .min_w(0.0)
        .shrink()
        .radius(tk.r(0.5))
        .fill(if open { tk.accent } else { Color::TRANSPARENT })
        .hover(tk.accent)
        .role("button")
        .label(aria)
        .help(tip)
        .expanded(open)
        .focusable(true)
        .on_press(move |s: &mut App| super::toggle(s, host));
    if let Some(c) = &current {
        r = r.child(provider_glyph(tk, c, tk.r(1.0)));
    }
    r = r.child(
        text(label)
            .font(tk.tsm(500.0).lh(tk.r(0.875)))
            .color(ink)
            .truncate()
            .min_w(0.0)
            .shrink(),
    );
    if effort_label.is_some() || fast_on {
        let mut traits = row().gap(tk.r(0.25)).center_y();
        if let Some(e) = effort_label {
            traits = traits.child(
                text(e)
                    .font(tk.tsm(500.0))
                    .color(tk.muted_fg)
                    .truncate()
                    .max_w(tk.r(6.0)),
            );
        }
        if fast_on {
            traits = traits.child(ic(&ZAP_FILLED, tk.r(0.875), amber(500)));
        }
        r = r.child(traits);
    }
    if full {
        let warn = if tk.dark { amber(400) } else { amber(600) };
        r = r.child(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .a11y_hidden()
                .child(ic(&I::LOCK_OPEN, tk.r(0.875), warn))
                .child(
                    text(t("assistant:composer.permission.full_access.label"))
                        .font(tk.txs(400.0))
                        .color(if tk.dark { amber(400) } else { amber(700) }),
                ),
        );
    }
    r.child(ic(&I::CHEVRON_DOWN, tk.r(0.875), ink.alpha(ink.a * 0.6)).a11y_hidden())
}

// ---- Popover -----------------------------------------------------------------------------------
//
// The owner's overhaul of the Tauri picker (ModelPickerContent / ModelPickerSidebar): the
// shared glass popover in the app's menu colours, a plain rail of provider marks with the app's
// selected-row fill (no status dots), a provider header that says in one quiet line where
// prompts go (no LOCAL / REMOTE pills) with an icon-only Check models in its corner, and rows
// that carry only what the listing knows. The capabilities stay the web's.

/// The rail's width and its buttons.
fn rail_w(tk: &Tk) -> f32 {
    tk.r(3.0)
}

/// What the window leaves for the picker's body above (or, flipped, below) its trigger.
fn body_height(cx: &Cx<App>, tk: &Tk, host: &str) -> f32 {
    let want = tk.r(24.0);
    let floor = tk.r(12.0);
    let win_h = cx.layout_size("sh-root").map(|[_, h]| h).unwrap_or(0.0);
    let Some((top, h)) = cx.window_layout_with(&format!("asst-picker-{host}"), |r| {
        ((r[1] / 4.0).round() * 4.0, (r[3] / 4.0).round() * 4.0)
    }) else {
        return want;
    };
    if win_h <= 0.0 {
        return want;
    }
    // The footer, the search row, the anchor gap and a margin from the window's edge.
    let chrome = tk.r(2.75) + 8.0 + tk.r(0.75);
    let above = top - crate::shell::window_top_inset() - chrome;
    let below = win_h - (top + h) - chrome;
    above.max(below).clamp(floor, want)
}

pub fn popover(app: &App, cx: &mut Cx<App>, tk: &Tk, host: &'static str, layer: &mut Vec<El>) {
    let window = cx
        .layout_size("sh-root")
        .map(|[w, _]| w)
        .unwrap_or(app.shell.window_w);
    let effort = effort_shown(app);
    let want = if effort { tk.r(40.0) } else { tk.r(30.0) };
    let w = if window > 0.0 {
        want.min(window - tk.r(2.0))
    } else {
        want
    };
    let g = groups(app);
    let list_items = items(app, &g);
    let mut body = row()
        .w_full()
        .h(body_height(cx, tk, host))
        .child(rail(app, tk, &g))
        .child(content(app, tk, &g, &list_items));
    if effort {
        body = body.child(effort_column(app, tk));
    }
    let panel = col()
        .key(format!("asst-pick-pop-{host}"))
        .w(w)
        .role("dialog")
        .label(t("assistant:modelPicker.models"))
        .child(body)
        .child(footer(app, tk));
    let surface = crate::shell::glass(tk, panel)
        .radius(tk.r(0.75))
        .clip_rounded(tk.r(0.75))
        .modal(true, super::close)
        .focus_return(format!("asst-picker-{host}"));
    layer.push(rune_kit::scrim(
        format!("asst-pick-scrim-{host}"),
        super::close,
    ));
    layer.push(crate::shell::anchored(
        tk,
        surface,
        &format!("asst-picker-{host}"),
        rune_kit::Placement::TopStart,
        8.0,
    ));
    if app.assistant.picker.perm_open {
        permission_menu(app, tk, layer);
    }
}

fn hairline(tk: &Tk) -> Color {
    tk.border.alpha(tk.border.a * 0.7)
}

fn warn_ink(tk: &Tk) -> Color {
    if tk.dark { amber(400) } else { amber(600) }
}

/// The rail: Favorites, a rule, then one mark per provider. The view in force wears the app's
/// selected-row fill; a provider whose last check failed carries a small warning glyph. Up and
/// Down switch the view, Right goes to the search field.
fn rail(app: &App, tk: &Tk, groups: &[super::Group]) -> El {
    let searching = searching(app);
    let active = active_view(app, groups).key();
    let s = &app.assistant.picker;
    let btn = tk.r(2.25);
    /// (view, label, where its prompts go, warning, glyph).
    type Tab = (View, String, Option<String>, Option<String>, El);
    let mut views: Vec<Tab> = vec![(
        View::Favorites,
        t("assistant:modelPicker.favorites"),
        None,
        None,
        el(),
    )];
    for g in groups {
        let p = &g.provider;
        let warning = super::provider_warning(app, p).map(|_| strings::check_failed());
        views.push((
            View::Provider(p.name.clone()),
            p.label(),
            Some(strings::disclosure(&data::where_prompts_go(p))),
            warning,
            provider_glyph(tk, p, tk.r(1.25)),
        ));
    }
    let mut list = col()
        .w_full()
        .items_center()
        .py(tk.r(0.375))
        .gap(tk.r(0.25));
    for (i, (v, name, disclosure, warning, mark)) in views.into_iter().enumerate() {
        let selected = !searching && v.key() == active;
        let content = if i == 0 {
            ic(
                if selected { &STAR_FILLED } else { &I::STAR },
                tk.r(1.0),
                if selected { tk.fg } else { tk.muted_fg },
            )
        } else {
            mark
        };
        let key = if selected && s.focus == Focus::Rail {
            format!("asst-pick-rail-{i}-g{}", s.focus_gen)
        } else {
            format!("asst-pick-rail-{i}")
        };
        let mut tip = name.clone();
        if let Some(w) = &warning {
            tip.push_str(&format!(" · {w}"));
        }
        let has_warning = warning.is_some();
        let description = [disclosure, warning]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(". ");
        let mut b = row()
            .key(key)
            .size(btn, btn)
            .center()
            .radius(tk.r(0.5))
            .fill(if selected {
                tk.row_active
            } else {
                Color::TRANSPARENT
            })
            .hover(if selected {
                tk.row_active
            } else {
                tk.row_hover
            })
            .focus_highlight(tk.row_hover)
            .role("tab")
            .label(name)
            .help(tip)
            .help_side(Side::Left)
            .checked(selected)
            .focusable(true)
            .focus_selected(selected || (searching && i == 0))
            .on_press(move |a: &mut App| super::select_view(a, v.clone()))
            .on_key(|a: &mut App, ev: &KeyEvent, _: &mut Tx| {
                if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt {
                    return false;
                }
                match ev.key {
                    Key::Down => super::rail_step(a, 1),
                    Key::Up => super::rail_step(a, -1),
                    Key::Right => super::focus(a, Focus::Search),
                    _ => return false,
                }
                true
            })
            .child(content);
        if !description.is_empty() {
            b = b.description(description);
        }
        if has_warning {
            let d = tk.r(0.875);
            b = b.child(
                row()
                    .absolute(btn - d - 1.0, btn - d - 1.0)
                    .size(d, d)
                    .center()
                    .radius(999.0)
                    .fill(tk.popover)
                    .inert()
                    .a11y_hidden()
                    .child(ic(&I::TRIANGLE_ALERT, tk.r(0.625), warn_ink(tk))),
            );
        }
        if selected && s.focus == Focus::Rail {
            b = b.autofocus();
        }
        list = list.child(b);
        if i == 0 {
            list = list.child(
                el().h(1.0)
                    .w(tk.r(1.5))
                    .my(tk.r(0.125))
                    .fill(hairline(tk))
                    .a11y_hidden(),
            );
        }
    }
    col()
        .key("asst-pick-rail")
        .w(rail_w(tk))
        .h_full()
        .border_sides([0.0, 1.0, 0.0, 0.0], hairline(tk))
        .border_layout()
        .role("tablist")
        .label(t("assistant:modelPicker.providers"))
        .focus_group()
        .child(
            col()
                .w_full()
                .h_full()
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

/// The icon-only **Check models**: the one action that lists a provider's models now
/// (`ai.list_models` with `refresh`, this provider only), never run on its own.
fn check_button(app: &App, tk: &Tk, p: &Provider) -> El {
    let checking = app.assistant.checks.checking.contains(&p.name);
    let label = if checking {
        t("assistant:modelPicker.checkingModels")
    } else {
        t("assistant:modelPicker.checkModels")
    };
    let enabled = !checking && app.assistant.checks.acks.is_empty() && app.prefs.ai_features;
    let glyph = ic(
        &I::REFRESH_CW,
        tk.r(0.875),
        if checking { tk.fg } else { tk.muted_fg },
    );
    let glyph = if checking {
        glyph.looping(rune::LoopSpec::rotation(1.0))
    } else {
        glyph
    };
    let name = p.name.clone();
    let b = row()
        .key("asst-pick-check")
        .size(tk.r(1.75), tk.r(1.75))
        .center()
        .radius(tk.r(0.375))
        .role("button")
        .label(label)
        .description(tr(
            "assistant:modelPicker.checkModelsHint",
            &[("provider", &p.label())],
        ))
        .help(format!(
            "{} · {}",
            t("assistant:modelPicker.checkModels"),
            tr(
                "assistant:modelPicker.checkModelsHint",
                &[("provider", &p.label())]
            )
        ))
        .child(glyph);
    if enabled {
        b.hover(tk.row_hover)
            .focusable(true)
            .on_press(move |s: &mut App| crate::assistant::checks::check_models(s, &name))
    } else {
        b.disabled(true).opacity(if checking { 1.0 } else { 0.5 })
    }
}

/// The small secondary Check models of the empty state.
fn check_button_text(app: &App, tk: &Tk, p: &Provider) -> El {
    let checking = app.assistant.checks.checking.contains(&p.name);
    let label = if checking {
        t("assistant:modelPicker.checkingModels")
    } else {
        t("assistant:modelPicker.checkModels")
    };
    let enabled = !checking && app.assistant.checks.acks.is_empty() && app.prefs.ai_features;
    let glyph = ic(&I::REFRESH_CW, tk.r(0.75), tk.muted_fg);
    let glyph = if checking {
        glyph.looping(rune::LoopSpec::rotation(1.0))
    } else {
        glyph
    };
    let name = p.name.clone();
    let b = row()
        .key("asst-pick-check-empty")
        .h(tk.r(1.75))
        .px(tk.r(0.625))
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, hairline(tk))
        .border_layout()
        .role("button")
        .label(label.clone())
        .help(tr(
            "assistant:modelPicker.checkModelsHint",
            &[("provider", &p.label())],
        ))
        .child(glyph)
        .child(text(label).font(tk.txs(500.0)).color(tk.fg));
    if enabled {
        b.hover(tk.row_hover)
            .focusable(true)
            .on_press(move |s: &mut App| crate::assistant::checks::check_models(s, &name))
    } else {
        b.disabled(true).opacity(if checking { 1.0 } else { 0.5 })
    }
}

/// The header over the list: the provider (its mark, name, where prompts go, Check models in
/// the corner, then what its last check left), or Favorites with the starring shortcut.
fn header(app: &App, tk: &Tk, view: &View, group: Option<&super::Group>) -> Option<El> {
    let name_font = tk.tsm(500.0).lh(tk.r(1.25));
    let line_font = tk.txs(400.0).lh(tk.r(1.0));
    let head = |mark: El, title: String, line: El, action: Option<El>| {
        let mut r = row()
            .w_full()
            .gap(tk.r(0.625))
            .center_y()
            .child(
                row()
                    .size(tk.r(2.0), tk.r(2.0))
                    .center()
                    .radius(tk.r(0.5))
                    .fill(tk.muted.alpha(if tk.dark { 0.6 } else { 0.8 }))
                    .a11y_hidden()
                    .child(mark),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        text(title)
                            .font(name_font)
                            .color(tk.fg)
                            .truncate()
                            .role("heading"),
                    )
                    .child(line),
            );
        if let Some(a) = action {
            r = r.child(a);
        }
        r
    };
    match (view, group) {
        (View::Favorites, _) => {
            let hint = tr(
                "assistant:modelPicker.favoriteShortcutHint",
                &[("shortcut", &shortcut(&["mod", "d"]))],
            );
            Some(
                col()
                    .key("asst-pick-head")
                    .w_full()
                    .px(tk.r(0.75))
                    .pt(tk.r(0.625))
                    .pb(tk.r(0.375))
                    .child(head(
                        ic(&STAR_FILLED, tk.r(1.0), amber(500)),
                        t("assistant:modelPicker.favorites"),
                        text(hint).font(line_font).color(tk.muted_fg).truncate(),
                        None,
                    )),
            )
        }
        (View::Provider(_), Some(g)) => {
            let p = &g.provider;
            let disclosure = strings::disclosure(&data::where_prompts_go(p));
            // The endpoint (or what a coding agent brokers) stays one hover away.
            let detail = if data::is_cli(p) {
                t("assistant:modelPicker.cliChatOnly")
            } else {
                p.base_url.clone()
            };
            let line = text(disclosure)
                .font(line_font)
                .color(tk.muted_fg)
                .truncate()
                .help(detail);
            let mut c = col()
                .key("asst-pick-head")
                .w_full()
                .px(tk.r(0.75))
                .pt(tk.r(0.625))
                .pb(tk.r(0.375))
                .child(head(
                    provider_glyph(tk, p, tk.r(1.25)),
                    p.label(),
                    line,
                    Some(check_button(app, tk, p)),
                ));
            let snap = super::snapshot(app, p);
            let note = |msg: String, ink: Color| {
                row()
                    .w_full()
                    .mt(tk.r(0.375))
                    .gap(tk.r(0.375))
                    .role("status")
                    .child(ic(&I::TRIANGLE_ALERT, tk.r(0.75), ink).mt(tk.r(0.125)))
                    .child(
                        text(msg)
                            .font(tk.t2xs(400.0).lh(tk.r(1.0)))
                            .color(ink)
                            .wrap()
                            .max_lines(2)
                            .grow(1.0)
                            .min_w(0.0),
                    )
            };
            if let Some(e) = app.assistant.checks.check_errors.get(&p.name) {
                c = c.child(note(e.clone(), tk.destructive));
            } else if let Some(sn) = &snap {
                if sn.stale {
                    c = c.child(note(
                        tr(
                            "assistant:modelPicker.staleModels",
                            &[("error", sn.error.as_deref().unwrap_or(""))],
                        ),
                        warn_ink(tk),
                    ));
                } else if let Some(e) = &sn.error {
                    c = c.child(note(e.clone(), tk.destructive));
                }
            }
            Some(c)
        }
        _ => None,
    }
}

/// The list column: search, the header, the rows, the usability filter.
fn content(app: &App, tk: &Tk, groups: &[super::Group], list_items: &[super::Item]) -> El {
    let s = &app.assistant.picker;
    let is_search = searching(app);
    let view = active_view(app, groups);
    let group = match &view {
        View::Provider(p) => groups.iter().find(|g| g.provider.name == *p),
        _ => None,
    };
    let hi = highlighted(app, list_items);
    let mut c = col().grow(1.0).min_w(0.0).h_full();
    // Search across every provider: a full-width row with a hairline under it.
    let mut input = text_input(&s.query, tk.tsm(400.0).lh(tk.r(1.5)))
        .key(format!(
            "asst-pick-search-{}",
            if s.focus == Focus::Search {
                s.focus_gen
            } else {
                0
            }
        ))
        .single_line()
        .grow(1.0)
        .min_w(0.0)
        .h(tk.r(1.5))
        .color(tk.fg)
        .placeholder(
            t("assistant:modelPicker.searchModels"),
            tk.muted_fg.alpha(0.7),
        )
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(t("assistant:modelPicker.searchModels"))
        .description(tr(
            "assistant:modelPicker.favoriteShortcutHint",
            &[("shortcut", &shortcut(&["mod", "d"]))],
        ))
        .expanded(true)
        .on_edit(|a: &mut App| &mut a.assistant.picker.query)
        .on_input_change(|a: &mut App, _: &mut Tx| a.assistant.picker.highlight = None)
        .on_input_key(|a: &mut App, ev: &KeyEvent, _: &mut Tx| super::search_key(a, ev))
        .on_submit(|_: &mut App, _: &mut Tx| {});
    if let Some(i) = hi {
        input = input.active_descendant(format!("asst-pick-row-{i}"));
    }
    if s.focus == Focus::Search {
        input = input.autofocus();
    }
    c = c.child(
        row()
            .w_full()
            .h(tk.r(2.75))
            .px(tk.r(0.75))
            .gap(tk.r(0.5))
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], hairline(tk))
            .border_layout()
            .child(ic(&I::SEARCH, tk.r(1.0), tk.muted_fg).a11y_hidden())
            .child(input),
    );
    if !is_search && let Some(h) = header(app, tk, &view, group) {
        c = c.child(h);
    }
    // The rows.
    let mut list = col()
        .key("asst-pick-list")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .role("listbox")
        .label(t("assistant:modelPicker.models"));
    if list_items.is_empty() {
        list = list.child(empty_state(app, tk, groups, &view, group, is_search));
    } else {
        let mut rows = col()
            .w_full()
            .gap(1.0)
            .px(tk.r(0.375))
            .pb(tk.r(0.375))
            .pt(if is_search { tk.r(0.375) } else { tk.r(0.125) });
        let sel = app
            .assistant
            .selection
            .as_ref()
            .map(|(p, m)| data::model_key(p, m));
        let fav_short = shortcut(&["mod", "d"]);
        for (i, it) in list_items.iter().enumerate() {
            rows = rows.child(model_row(
                tk,
                i,
                it,
                sel.as_deref() == Some(it.key.as_str()),
                hi == Some(i),
                is_search || view == View::Favorites,
                &fav_short,
            ));
        }
        list = list.child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(rows),
        );
    }
    c = c.child(list);
    // What the usability filter took out, with the way to show it.
    let filtered: usize = if is_search {
        groups.iter().map(|g| g.hidden).sum()
    } else {
        group.map(|g| g.hidden).unwrap_or(0)
    };
    if filtered > 0 {
        let all = s.show_all;
        let msg = if all {
            tc("assistant:modelPicker.unusableModelsShown", filtered, &[])
        } else {
            tc("assistant:modelPicker.unusableModelsHidden", filtered, &[])
        };
        c = c.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .px(tk.r(0.75))
                .py(tk.r(0.375))
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], hairline(tk))
                .border_layout()
                .child(
                    text(msg)
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    row()
                        .key("asst-pick-show-all")
                        .px(tk.r(0.375))
                        .py(tk.r(0.125))
                        .radius(tk.r(0.25))
                        .fill(if all {
                            tk.row_active
                        } else {
                            Color::TRANSPARENT
                        })
                        .hover(if all { tk.row_active } else { tk.row_hover })
                        .role("button")
                        .checked(all)
                        .label(t("assistant:modelPicker.showAllModels"))
                        .help(t("assistant:modelPicker.showAllModelsHint"))
                        .focusable(true)
                        .on_press(|a: &mut App| {
                            a.assistant.picker.show_all = !a.assistant.picker.show_all
                        })
                        .child(
                            text(t("assistant:modelPicker.showAllModels"))
                                .font(tk.t2xs(500.0))
                                .color(if all { tk.fg } else { tk.muted_fg }),
                        ),
                ),
        );
    }
    if let Some(e) = s
        .footer_error
        .as_ref()
        .or(app.assistant.checks.ack_error.as_ref())
    {
        c = c.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .border_sides([1.0, 0.0, 0.0, 0.0], hairline(tk))
                .border_layout()
                .role("status")
                .child(
                    text(e.clone())
                        .font(tk.t2xs(400.0))
                        .color(tk.destructive)
                        .wrap(),
                ),
        );
    }
    c
}

/// One line, and the way out where there is one (Check models, Show all models).
fn empty_state(
    app: &App,
    tk: &Tk,
    groups: &[super::Group],
    view: &View,
    group: Option<&super::Group>,
    searching: bool,
) -> El {
    let (title, hint) = empty_message(app, groups, view, group, searching);
    let mut e = col()
        .key("asst-pick-empty")
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(2.5))
        .gap(tk.r(0.625))
        .items_center()
        .role("status");
    if *view == View::Favorites && !searching {
        e = e.child(ic(&I::STAR, tk.r(1.25), tk.muted_fg.alpha(0.6)));
    }
    e = e.child(
        text(title)
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .align(rune::Align::Center)
            .wrap()
            .max_w(tk.r(18.0)),
    );
    if let Some(h) = hint {
        e = e.child(
            text(h)
                .font(tk.txs(400.0).lh(tk.r(1.0)))
                .color(tk.muted_fg.alpha(0.8))
                .wrap()
                .align(rune::Align::Center)
                .max_w(tk.r(16.0)),
        );
    }
    let no_models = !searching && group.is_some_and(|g| g.models.is_empty() && g.hidden == 0);
    if let (true, Some(g)) = (no_models, group) {
        e = e.child(check_button_text(app, tk, &g.provider));
    }
    e
}

fn empty_message(
    app: &App,
    groups: &[super::Group],
    view: &View,
    group: Option<&super::Group>,
    searching: bool,
) -> (String, Option<String>) {
    let favs = !app.assistant.picker.favorites.is_empty();
    if groups.is_empty() && !favs {
        return (t("assistant:modelPicker.noProviders"), None);
    }
    if searching {
        return (t("assistant:modelPicker.noMatchingModels"), None);
    }
    if *view == View::Favorites {
        return (
            t("assistant:modelPicker.noFavorites"),
            Some(tr(
                "assistant:modelPicker.noFavoritesHint",
                &[("shortcut", &shortcut(&["mod", "d"]))],
            )),
        );
    }
    if !app.assistant.picker.show_all && group.is_some_and(|g| g.hidden > 0) {
        return (
            t("assistant:modelPicker.noUsableModels"),
            Some(t("assistant:modelPicker.noUsableModelsHint")),
        );
    }
    // One line: the button under it says what to do.
    (t("assistant:modelPicker.noModels"), None)
}

/// A model row: the name with what the listing knows (a proxy prefix, a route off the device
/// when it differs from the provider's, Fast), the provider or id line, then the current
/// model's check, the highlighted row's ⌘-digit and the star (shown on hover, on the
/// highlighted row, and always once starred).
fn model_row(
    tk: &Tk,
    i: usize,
    it: &super::Item,
    selected: bool,
    highlighted: bool,
    show_provider: bool,
    fav_short: &str,
) -> El {
    let m = &it.model;
    let name = m
        .display_name
        .clone()
        .or_else(|| m.prefix.as_ref().and(m.base_model.clone()))
        .unwrap_or_else(|| m.id.clone());
    let show_id = name != m.id;
    let mut title = row().gap(tk.r(0.375)).center_y().min_w(0.0).child(
        text(name.clone())
            .font(tk.tsm(400.0).lh(tk.r(1.25)))
            .color(tk.fg)
            .truncate()
            .min_w(0.0)
            .shrink(),
    );
    if let Some(prefix) = &m.prefix {
        title = title.child(
            row()
                .px(tk.r(0.25))
                .radius(tk.r(0.25))
                .fill(tk.muted)
                .help(tr(
                    "assistant:modelPicker.prefixTitle",
                    &[("prefix", prefix)],
                ))
                .child(
                    text(prefix.clone())
                        .font(tk.t2xs(400.0).lh(tk.r(1.0)).mono())
                        .color(tk.muted_fg),
                ),
        );
    }
    // A model whose prompts go elsewhere than its provider's (an Ollama cloud model): say so.
    let elsewhere = data::model_where(&it.provider, m).map(|w| strings::disclosure(&w));
    if let Some(d) = &elsewhere {
        let tip = m
            .privacy_reason
            .clone()
            .map(|r| format!("{d} · {r}"))
            .unwrap_or_else(|| d.clone());
        let glyph = if it.posture == "local" {
            &I::HARD_DRIVE
        } else {
            &I::CLOUD
        };
        title = title.child(
            row()
                .center()
                .help(tip)
                .child(ic(glyph, tk.r(0.75), tk.muted_fg)),
        );
    }
    let fast = m.supports_fast_mode == Some(true);
    if fast {
        let tip = m
            .fast_mode_description
            .clone()
            .unwrap_or_else(|| t("assistant:composer.fastMode.hint"));
        title = title.child(row().center().help(tip).child(ic(
            &ZAP_FILLED,
            tk.r(0.75),
            amber(500).alpha(0.85),
        )));
    }
    let mut left = col().grow(1.0).min_w(0.0).child(title);
    let sub = tk.txs(400.0).lh(tk.r(1.0));
    if show_provider {
        let mut line = row()
            .gap(tk.r(0.375))
            .center_y()
            .min_w(0.0)
            .child(provider_glyph(tk, &it.provider, tk.r(0.75)))
            .child(
                text(it.provider.label())
                    .font(sub)
                    .color(tk.muted_fg)
                    .truncate(),
            );
        if show_id {
            line = line.child(
                text(format!("· {}", m.id))
                    .font(sub.mono())
                    .color(tk.muted_fg.alpha(0.8))
                    .truncate()
                    .min_w(0.0),
            );
        }
        left = left.child(line);
    } else if show_id {
        left = left.child(
            text(m.id.clone())
                .font(sub.mono())
                .color(tk.muted_fg.alpha(0.8))
                .truncate(),
        );
    }
    let mut right = row().gap(tk.r(0.25)).center_y();
    if highlighted && i < 9 {
        right = right.child(
            text(shortcut(&["mod", &(i + 1).to_string()]))
                .font(tk.t2xs(500.0))
                .color(tk.muted_fg.alpha(0.8))
                .a11y_hidden()
                .mr(tk.r(0.125)),
        );
    }
    if selected {
        right = right.child(ic(&I::CHECK, tk.r(1.0), tk.fg));
    }
    let fav_label = if it.favorite {
        t("assistant:modelPicker.removeFavorite")
    } else {
        t("assistant:modelPicker.addFavorite")
    };
    let (fp, fm) = (it.provider.name.clone(), m.id.clone());
    let mut star = row()
        .key(format!("asst-pick-star-{i}"))
        .size(tk.r(1.5), tk.r(1.5))
        .mr(-tk.r(0.25))
        .center()
        .radius(tk.r(0.375))
        .hover(tk.row_hover)
        .role("button")
        .label(fav_label.clone())
        .help(format!("{fav_label} ({fav_short})"))
        .checked(it.favorite)
        .on_press(move |a: &mut App| super::toggle_favorite(a, &fp, &fm))
        .child(if it.favorite {
            ic(&STAR_FILLED, tk.r(0.875), amber(500))
        } else {
            ic(&I::STAR, tk.r(0.875), tk.muted_fg)
        });
    if !it.favorite && !highlighted {
        star = star.group_opacity(0.0, 1.0);
    }
    right = right.child(star);
    let key = it.key.clone();
    let (pp, pm) = (it.provider.name.clone(), m.id.clone());
    let label = if show_provider {
        format!("{name} · {}", it.provider.label())
    } else {
        name.clone()
    };
    let mut r = row()
        .key(format!("asst-pick-row-{i}"))
        .w_full()
        .min_h(tk.r(2.25))
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .fill(if highlighted {
            tk.accent
        } else {
            Color::TRANSPARENT
        })
        .hover_group()
        .role("option")
        .label(label)
        .checked(highlighted)
        .on_hover_change(move |a: &mut App, on: bool, _: &mut Tx| {
            if on && a.assistant.picker.highlight.as_deref() != Some(key.as_str()) {
                a.assistant.picker.highlight = Some(key.clone());
                a.assistant.picker.keyboard_highlight = false;
            }
        })
        .on_press(move |a: &mut App| super::pick(a, &pp, &pm))
        .child(left)
        .child(right);
    if let Some(d) = elsewhere {
        r = r.description(d);
    }
    if selected {
        r = r.focus_selected(true);
    }
    r
}

/// `ModelPickerEffortColumn`: the selected model's name over its levels, Auto first.
fn effort_column(app: &App, tk: &Tk) -> El {
    let a = &app.assistant;
    let s = &a.picker;
    let (levels, known) = effort_options(app);
    let mut shown: Vec<String> = vec!["auto".into()];
    shown.extend(levels.iter().cloned());
    if !shown.contains(&a.effort) {
        shown.push(a.effort.clone());
    }
    let model_label = selection_row(app)
        .map(|(_, r)| r.display_name.unwrap_or(r.id))
        .unwrap_or_default();
    let mut group = col()
        .key("asst-pick-effort")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(tk.r(0.125))
        .px(tk.r(0.5))
        .pt(tk.r(0.5))
        .pb(tk.r(0.375))
        .role("radiogroup")
        .label(t("assistant:composer.reasoningEffort"));
    let n = shown.len();
    for (i, e) in shown.iter().enumerate() {
        let sel = *e == a.effort;
        let label = t(&format!("assistant:composer.effort.{e}"));
        let key = if sel && s.focus == Focus::Effort {
            format!("asst-pick-level-{i}-g{}", s.focus_gen)
        } else {
            format!("asst-pick-level-{i}")
        };
        let ev = e.clone();
        let list = shown.clone();
        let mut r = row()
            .key(key)
            .w_full()
            .h(tk.r(2.0))
            .px(tk.r(0.5))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.25))
            .fill(if sel {
                tk.row_active
            } else {
                Color::TRANSPARENT
            })
            .hover(if sel { tk.row_active } else { tk.row_hover })
            .focus_highlight(tk.row_hover)
            .role("radio")
            .checked(sel)
            .label(if e == "auto" {
                format!("{label} {}", t("assistant:composer.effortDefault"))
            } else {
                label.clone()
            })
            .focusable(true)
            .focus_selected(sel)
            .on_press(move |a: &mut App| {
                a.assistant.effort = ev.clone();
                a.assistant.effort_chosen = true;
                super::focus(a, Focus::Effort);
            })
            .on_key(move |a: &mut App, k: &KeyEvent, _: &mut Tx| {
                if k.mods.cmd || k.mods.ctrl || k.mods.alt {
                    return false;
                }
                let next = match k.key {
                    Key::Down => (i + 1).min(n - 1),
                    Key::Up => i.saturating_sub(1),
                    Key::Home => 0,
                    Key::End => n - 1,
                    Key::Left => {
                        super::focus(a, Focus::Search);
                        return true;
                    }
                    _ => return false,
                };
                a.assistant.effort = list[next].clone();
                a.assistant.effort_chosen = true;
                super::focus(a, Focus::Effort);
                true
            })
            .child(
                text(label)
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if e == "auto" {
            r = r.help(t("assistant:composer.effortAutoHint"));
        }
        if sel {
            r = r.child(ic(&I::CHECK, tk.r(0.875), tk.fg));
            if s.focus == Focus::Effort {
                r = r.autofocus();
            }
        } else if e == "auto" {
            r = r.child(
                text(t("assistant:composer.effortDefault"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg.alpha(0.7)),
            );
        }
        group = group.child(r);
    }
    if levels.is_empty() && !known {
        group = group.child(
            text(t("assistant:composer.effortLevelsUnknown"))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg.alpha(0.8))
                .wrap()
                .px(tk.r(0.5))
                .pt(tk.r(0.25)),
        );
    }
    col()
        .w(tk.r(10.0))
        .h_full()
        .border_sides([0.0, 0.0, 0.0, 1.0], hairline(tk))
        .border_layout()
        .child(
            col().w_full().child(
                row()
                    .w_full()
                    .h(tk.r(2.75))
                    .px(tk.r(0.75))
                    .gap(tk.r(0.5))
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], hairline(tk))
                    .border_layout()
                    .child(
                        text(t("assistant:composer.reasoning"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                    )
                    .child(
                        row().grow(1.0).min_w(0.0).justify_end().child(
                            text(model_label.clone())
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .truncate()
                                .help(model_label),
                        ),
                    ),
            ),
        )
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(group),
        )
}

/// `ModelPickerFooter`: Standard | Fast where the model offers it, then the permission mode.
fn footer(app: &App, tk: &Tk) -> El {
    let a = &app.assistant;
    let mut f = row()
        .key("asst-pick-footer")
        .w_full()
        .min_h(tk.r(2.75))
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .flex_wrap()
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], hairline(tk))
        .border_layout();
    let (sp, sm, _) = selected(app);
    let (fast, desc, runtime) = data::fast_mode(sp.as_ref(), sm.as_ref());
    if fast {
        let hint = if runtime == Some("claude") {
            t("assistant:composer.fastMode.claudeHint")
        } else {
            desc.unwrap_or_else(|| t("assistant:composer.fastMode.hint"))
        };
        let on = a.fast_mode;
        let seg = |fast_seg: bool| {
            let checked = on == fast_seg;
            let label = if fast_seg {
                t("assistant:composer.fastMode.fast")
            } else {
                t("assistant:composer.fastMode.standard")
            };
            let mut b = row()
                .key(format!("asst-pick-speed-{fast_seg}"))
                .h(tk.r(1.5))
                .px(tk.r(0.5))
                .gap(tk.r(0.25))
                .center()
                .radius(tk.r(0.375))
                .role("radio")
                .checked(checked)
                .label(label.clone())
                .focusable(true)
                .focus_selected(checked)
                .on_press(move |s: &mut App| s.assistant.fast_mode = fast_seg);
            if checked {
                b = b.fill(tk.background).box_shadow(Shadow {
                    y: 1.0,
                    blur: 2.0,
                    color: Color::BLACK.alpha(0.05),
                    ..Shadow::default()
                });
            }
            if fast_seg {
                b = b.child(ic(
                    &ZAP_FILLED,
                    tk.r(0.75),
                    if checked { tk.fg } else { tk.muted_fg },
                ));
                if !on {
                    b = b.help(hint.clone());
                }
            }
            b.child(text(label).font(tk.txs(400.0)).color(if checked {
                tk.fg
            } else {
                tk.muted_fg
            }))
        };
        f = f.child(
            row()
                .padding(tk.r(0.125))
                .radius(tk.r(0.5))
                .fill(tk.muted)
                .role("radiogroup")
                .label(t("assistant:composer.fastMode.speed"))
                .focus_group()
                .child(seg(false))
                .child(seg(true)),
        );
        if on {
            f = f.child(
                text(hint)
                    .font(tk.t2xs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .min_w(0.0),
            );
        }
    }
    let mode = a.permission;
    let warn = mode == PermissionMode::FullAccess;
    let ink = if warn {
        if tk.dark { amber(400) } else { amber(700) }
    } else if a.picker.perm_open {
        tk.fg
    } else {
        tk.muted_fg
    };
    let title = t("assistant:composer.permission.label");
    let label = t(&format!(
        "assistant:composer.permission.{}.label",
        permission_key(mode)
    ));
    f.child(row().grow(1.0)).child(
        row()
            .key("asst-pick-perm")
            .h(tk.r(1.75))
            .px(tk.r(0.5))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .fill(if a.picker.perm_open {
                tk.row_active
            } else {
                Color::TRANSPARENT
            })
            .hover(if a.picker.perm_open {
                tk.row_active
            } else {
                tk.row_hover
            })
            .role("button")
            .label(format!("{title}: {label}"))
            .help(t(&format!(
                "assistant:composer.permission.{}.description",
                permission_key(mode)
            )))
            .focusable(true)
            .menu_trigger(a.picker.perm_open, |s: &mut App| {
                s.assistant.picker.perm_open = !s.assistant.picker.perm_open
            })
            .child(ic(permission_icon(mode), tk.r(0.875), ink))
            .child(text(label).font(tk.txs(400.0)).color(ink))
            .child(ic(&I::CHEVRON_UP, tk.r(0.75), ink.alpha(ink.a * 0.7))),
    )
}

/// `PermissionModeMenu`'s menu (`side="top" align="end" w-72`): each mode with its description,
/// Full access in a warning tone.
fn permission_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let cur = app.assistant.permission;
    let mut p = crate::ui::popup::Popup::menu("asst-pick-perm", |s: &mut App| {
        s.assistant.picker.perm_open = false
    })
    .list_key("asst-pick-perm-menu");
    p.add(
        row().w_full().px(tk.r(0.5)).py(tk.r(0.375)).child(
            text(t("assistant:composer.permission.label"))
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        ),
    );
    for m in PermissionMode::ALL {
        let k = permission_key(m);
        let checked = m == cur;
        let label = t(&format!("assistant:composer.permission.{k}.label"));
        let desc = t(&format!("assistant:composer.permission.{k}.description"));
        let glyph_ink = if m == PermissionMode::FullAccess {
            if tk.dark { amber(400) } else { amber(600) }
        } else {
            tk.muted_fg
        };
        let mut indicator = row().w(tk.r(0.875)).h(tk.r(1.25)).center().a11y_hidden();
        if checked {
            indicator = indicator.child(crate::ui::dot(tk.r(0.5), tk.fg));
        }
        let item = row()
            .w_full()
            .py(tk.r(0.5))
            .pl(tk.r(0.5))
            .pr(tk.r(0.5))
            .gap(tk.r(0.625))
            .radius(tk.r(0.25))
            .hover(tk.accent)
            .focus_highlight(tk.accent)
            .role("menuitemradio")
            .checked(checked)
            .focus_selected(checked)
            .label(format!("{label} {desc}"))
            .focusable(true)
            .on_press(move |s: &mut App| {
                crate::assistant::chat::set_permission_mode(s, m);
                s.assistant.picker.perm_open = false;
            })
            .child(indicator)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(tk.r(0.125))
                    .child(
                        row()
                            .gap(tk.r(0.375))
                            .center_y()
                            .child(ic(permission_icon(m), tk.r(0.875), glyph_ink))
                            .child(text(label.clone()).font(tk.tsm(500.0)).color(tk.fg)),
                    )
                    .child(
                        text(desc)
                            .font(tk.txs(400.0).lh(tk.r(1.0)))
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
            );
        p.item(item, format!("asst-pick-perm-{k}"), label, true, checked);
    }
    p.mount(
        tk,
        layer,
        col()
            .w(tk.r(18.0))
            .padding(tk.r(0.25))
            .label(t("assistant:composer.permission.label")),
        |l| crate::shell::glass(tk, l),
        rune_kit::Placement::TopEnd,
        4.0,
    );
}
