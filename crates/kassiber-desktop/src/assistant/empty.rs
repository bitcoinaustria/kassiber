//! The assistant's one empty state, shared by the page (/assistant), the maximized side panel
//! and the docked side panel: a centred column with the greeting, one muted line, the model
//! chooser (ModelSetupCard.tsx, while no model is chosen) and the composer directly under it,
//! then any recent chats. The page and the maximized panel draw it identically; the docked
//! panel draws the same structure at its own width with a smaller greeting.
//!
//! A deliberate change from the Tauri app (owner feedback): the web's panel put a different
//! empty hint and the chooser at the bottom above a docked composer, and its page used another
//! layout; here all three look the same.

use rune::{Cx, El, col, row, text};

use super::composer::{self, Opts};
use super::picker;
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button};
use crate::ui::ic;

/// The empty column's width (`max-w-2xl`): the chooser and the composer share it.
pub const COLUMN: f32 = 40.0;

/// Where the empty state is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// The Assistant page.
    Page,
    /// The side panel, maximized over the page column: drawn exactly like the page.
    Maximized,
    /// The side panel docked beside the page.
    Docked,
}

impl Host {
    fn composer(self) -> &'static str {
        if self == Host::Page { "page" } else { "panel" }
    }
}

/// The empty state's group: greeting, line, chooser, composer, error and recent chats. The host
/// centres it vertically in its own space.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, host: Host, layer: &mut Vec<El>) -> El {
    let a = &app.assistant;
    let docked = host == Host::Docked;
    let has_model = a.selection.as_ref().is_some_and(|(_, m)| !m.is_empty());
    let mut c = col()
        .key(format!("asst-empty-{}", host.composer()))
        .w_full()
        .max_w(tk.r(COLUMN))
        .gap(if docked { tk.r(1.25) } else { tk.r(1.5) })
        .items_center();

    let size = if docked { tk.r(1.25) } else { tk.r(1.875) };
    let line_h = if docked { tk.r(1.75) } else { tk.r(2.25) };
    let heading = text(t("assistant:page.heading"))
        .font(
            rune::TextStyle::new(size, if docked { 500.0 } else { 400.0 })
                .lh(line_h)
                .tracking(-0.025 * size),
        )
        .color(tk.fg)
        .align(rune::Align::Center)
        .wrap()
        .role("heading");
    let line = text(t("assistant:panel.emptyBody"))
        .font(tk.tsm(400.0).lh(tk.r(1.25)))
        .color(tk.muted_fg)
        .align(rune::Align::Center)
        .wrap();
    c = c.child(
        col()
            .w_full()
            .max_w(tk.r(32.0))
            .gap(tk.r(0.5))
            .items_center()
            .child(heading)
            .child(line),
    );

    if !has_model && let Some(card) = model_setup_card(app, tk) {
        c = c.child(card);
    }

    let mut box_col = col().w_full();
    if let Some(chip) = composer::focus_chip(app, tk, true) {
        box_col = box_col.child(chip.mb(tk.r(0.5)));
    }
    box_col = box_col.child(composer::view(
        app,
        cx,
        tk,
        Opts {
            host: host.composer(),
            hero: !docked,
            suggestions: has_model,
            compact: false,
        },
        layer,
    ));
    c = c.child(box_col);

    if let Some(e) = &a.error {
        c = c.child(
            row()
                .w_full()
                .radius(tk.r(0.375))
                .border(1.0, tk.destructive.alpha(0.3))
                .border_layout()
                .fill(tk.destructive.alpha(0.1))
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .role("alert")
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap(),
                ),
        );
    }
    if !a.recent.is_empty() {
        c = c.child(super::page::recent_chats(app, tk));
    }
    c
}

/// `runtimeSummary` of a cached check: (headline key under `settings:ai.status`, tone is error,
/// detail).
fn runtime_status(app: &App, name: &str) -> Option<(String, bool, Option<String>)> {
    let (_, data) = app.assistant.runtime.iter().find(|(n, _)| n == name)?;
    let status = data["providers"].as_array()?.first()?;
    let detail = status["message"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Some(match status["state"].as_str()? {
        "ready" => return None,
        "missing_executable" => ("notInstalled".into(), true, detail),
        "authentication_required" => ("signInNeeded".into(), false, detail),
        _ => ("error".into(), true, detail),
    })
}

/// `ModelSetupCard`: the coding agents as compact rows with **Use** (switch on if needed,
/// confirm an off-device agent, check it, then make it the chat's model), and a quiet footer
/// row that opens Settings for local models, servers and API keys. Nothing is contacted until
/// the user presses Use.
pub fn model_setup_card(app: &App, tk: &Tk) -> Option<El> {
    if !app.prefs.ai_features {
        return None;
    }
    let providers = app.assistant.providers.ready()?;
    let agents: Vec<_> = providers.iter().filter(|p| p.is_cli()).collect();
    if agents.is_empty() {
        return None;
    }
    let border = tk.border.alpha(tk.border.a * 0.7);
    let mut card = col()
        .key("asst-setup")
        .w_full()
        .radius(tk.r(0.75))
        .border(1.0, border)
        .border_layout()
        .fill(tk.card)
        .clip_rounded(tk.r(0.75))
        .role("region")
        .label(t("assistant:modelSetup.title"));
    card = card.child(
        col()
            .w_full()
            .px(tk.r(1.0))
            .pt(tk.r(0.875))
            .pb(tk.r(0.5))
            .gap(tk.r(0.125))
            .child(
                text(t("assistant:modelSetup.title"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg)
                    .role("heading"),
            )
            .child(
                text(t("assistant:modelSetup.body"))
                    .font(tk.txs(400.0).lh(tk.r(1.125)))
                    .color(tk.muted_fg)
                    .wrap(),
            ),
    );
    let mut list = col().w_full().px(tk.r(0.375)).pb(tk.r(0.375)).role("list");
    for (i, p) in agents.iter().enumerate() {
        let name = p.label();
        let mut info = col().grow(1.0).min_w(0.0).gap(tk.r(0.0625)).child(
            text(name.clone())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .truncate(),
        );
        let status = runtime_status(app, &p.name);
        let pending = super::checks::busy(app, &p.name)
            && app
                .assistant
                .checks
                .starting
                .as_deref()
                .is_none_or(|n| n == p.name);
        // "Checking…", "Offline", or this agent's failure; else what its last check found.
        let line = match super::checks::setup_status(app, &p.name) {
            Some(l) => Some(l),
            None => status.as_ref().map(|(key, error, detail)| {
                let mut line = t(&format!("settings:ai.status.{key}"));
                if let Some(d) = detail {
                    line = format!("{line} · {}", d.replace('`', ""));
                }
                (line, *error)
            }),
        };
        if let Some((line, error)) = line {
            info = info.child(
                text(line)
                    .font(tk.txs(400.0))
                    .color(if error { tk.destructive } else { tk.muted_fg })
                    .wrap()
                    .role("status"),
            );
        }
        let label = if status.is_some() {
            t("assistant:modelSetup.retry")
        } else {
            t("assistant:modelSetup.use")
        };
        let enabled = !pending
            && app.assistant.checks.starting.is_none()
            && app.assistant.checks.runtime_pending.is_empty()
            && app.prefs.ai_features;
        let pname = p.name.clone();
        let mut use_btn = button(
            tk,
            format!("asst-setup-use-{i}"),
            Variant::Outline,
            Size::Sm,
            None,
            &label,
            enabled,
        )
        .label(tr("assistant:modelSetup.useAria", &[("name", &name)]));
        if enabled {
            use_btn = use_btn.on_press(move |s: &mut App| super::checks::set_up(s, &pname));
        }
        // The logo on a quiet tile, like the provider rows in Settings.
        let tile = row()
            .size(tk.r(2.0), tk.r(2.0))
            .center()
            .radius(tk.r(0.5))
            .fill(tk.muted.alpha(if tk.dark { 0.6 } else { 0.8 }))
            .a11y_hidden()
            .child(picker::provider_glyph(tk, p, tk.r(1.125)));
        list = list.child(
            row()
                .key(format!("asst-setup-row-{i}"))
                .w_full()
                .gap(tk.r(0.75))
                .px(tk.r(0.625))
                .py(tk.r(0.5))
                .radius(tk.r(0.5))
                .center_y()
                .role("listitem")
                .label(name)
                .child(tile)
                .child(info)
                .child(use_btn),
        );
    }
    card = card.child(list);
    let settings_l = t("assistant:modelSetup.openSettings");
    card = card.child(
        row()
            .key("asst-setup-settings")
            .w_full()
            .gap(tk.r(0.5))
            .px(tk.r(1.0))
            .py(tk.r(0.625))
            .center_y()
            .fill(tk.muted.alpha(if tk.dark { 0.25 } else { 0.4 }))
            .hover(tk.muted.alpha(if tk.dark { 0.45 } else { 0.7 }))
            .border_sides([1.0, 0.0, 0.0, 0.0], border)
            .border_layout()
            .role("button")
            .label(settings_l.clone())
            .help(t("assistant:modelSetup.other"))
            .focusable(true)
            .on_press(|s: &mut App| crate::settings::open_section(s, "assistant-ai"))
            .child(ic(&I::SETTINGS, tk.r(0.875), tk.muted_fg))
            .child(
                text(t("assistant:modelSetup.other"))
                    .font(tk.txs(400.0).lh(tk.r(1.125)))
                    .color(tk.muted_fg)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(
                row()
                    .gap(tk.r(0.125))
                    .center_y()
                    .child(text(settings_l).font(tk.txs(500.0)).color(tk.fg))
                    .child(ic(&I::CHEVRON_RIGHT, tk.r(0.875), tk.muted_fg)),
            ),
    );
    Some(card)
}
