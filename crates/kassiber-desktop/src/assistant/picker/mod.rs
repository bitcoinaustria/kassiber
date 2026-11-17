//! The composer's model picker (ProviderModelPicker.tsx, ModelPickerContent.tsx,
//! ModelPickerSidebar.tsx, ModelListRow.tsx, ComposerModelTraits.tsx, ProviderGlyph.tsx): a
//! quiet trigger in the composer toolbar, a provider rail with Favorites, ranked search across
//! every provider, the model list, the reasoning column, and a footer with speed and the
//! permission mode.
//!
//! Privacy: opening, hovering, searching and switching providers read only what the daemon
//! already has (`ai.providers.list`, and `cached_only` listings of providers that were checked
//! before). **Check models** (discovery, one provider) and acknowledging an off-device provider
//! run only from the user's press, through `checks.rs`.

pub mod brands;
pub mod data;
pub mod strings;
pub mod view;

use std::collections::HashMap;

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx};
use serde_json::json;

use crate::app::{App, Load, Reply};
use crate::assistant::model::Provider;
use crate::theme::Tk;
use data::{ModelRow, Snapshot};

pub use view::provider_glyph;

/// The rail's selection: the Favorites list or one provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    Favorites,
    Provider(String),
}

impl View {
    pub fn key(&self) -> String {
        match self {
            View::Favorites => "favorites".into(),
            View::Provider(p) => format!("provider:{p}"),
        }
    }
}

/// Where keyboard focus goes next inside the open picker (applied by remounting the target
/// with `autofocus`, keyed by `focus_gen`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Search,
    Effort,
    Rail,
}

#[derive(Default)]
pub struct State {
    /// Which composer's picker is open: `"page"` or `"panel"`.
    pub open: Option<&'static str>,
    /// The rail's view; `None` until resolved (the selection's provider, else the default).
    pub view: Option<View>,
    pub query: TextEdit,
    /// The highlighted row's model key; `None` falls back to the selection, then the first row.
    pub highlight: Option<String>,
    /// The highlight was last moved with the arrow keys (only then ArrowRight picks it).
    pub keyboard_highlight: bool,
    /// List models that cannot run the assistant too.
    pub show_all: bool,
    /// `assistantModelFavorites`, in starring order.
    pub favorites: Vec<(String, String)>,
    /// The footer's permission-mode menu.
    pub perm_open: bool,
    /// Why a pick was refused (an off-device provider needs an acknowledgement this build won't
    /// give).
    pub footer_error: Option<String>,
    pub focus: Focus,
    pub focus_gen: u32,
    /// Cached-only listings by provider (`ai.list_models` with `cached_only`).
    pub listings: HashMap<String, Snapshot>,
    /// The restore reads were sent for this provider list.
    pub restored: bool,
}

/// The providers the picker offers: enabled ones, one row per CLI locator.
pub fn providers(app: &App) -> Vec<Provider> {
    let Some(all) = app.assistant.providers.ready() else {
        return vec![];
    };
    let enabled: Vec<&Provider> = all.iter().filter(|p| p.enabled).collect();
    let preferred = app
        .assistant
        .selection
        .as_ref()
        .map(|(p, _)| p.clone())
        .or_else(|| all.iter().find(|p| p.is_default).map(|p| p.name.clone()));
    data::dedupe(&enabled, preferred.as_deref())
}

/// What the last check of `p` left: the cached listing, or a ready runtime check's models
/// (the newer one wins, as `useRestoreProviderModelChecks` decides).
pub fn snapshot(app: &App, p: &Provider) -> Option<Snapshot> {
    let listing = app.assistant.picker.listings.get(&p.name).cloned();
    let runtime = app
        .assistant
        .runtime
        .iter()
        .find(|(n, _)| *n == p.name)
        .and_then(|(_, v)| {
            let checked = v["checked_at"].as_str()?.to_owned();
            let st = v["providers"].as_array().and_then(|a| a.first());
            let models = st
                .filter(|s| s["state"] == "ready")
                .map(|s| ModelRow::parse_list(&s["models"]))
                .unwrap_or_default();
            Some(Snapshot {
                models,
                stale: v["stale"].as_bool().unwrap_or(false),
                error: v["error"]["message"].as_str().map(str::to_owned),
                checked_at: Some(checked),
            })
        });
    match (listing, runtime) {
        (Some(l), Some(r)) => Some(if r.checked_at >= l.checked_at { r } else { l }),
        (l, r) => l.or(r),
    }
}

/// One provider's rows (`groupedRows` + the usability filter).
#[derive(Clone, Debug)]
pub struct Group {
    pub provider: Provider,
    pub models: Vec<ModelRow>,
    /// Models the filter takes out, shown or not.
    pub hidden: usize,
}

pub fn groups(app: &App) -> Vec<Group> {
    let sel = app.assistant.selection.as_ref();
    let favorites = &app.assistant.picker.favorites;
    let cli_default = crate::i18n::t("assistant:modelPicker.cliDefault");
    providers(app)
        .into_iter()
        .map(|p| {
            let queried = snapshot(app, &p).map(|s| s.models).unwrap_or_default();
            let mut models: Vec<ModelRow> = if !queried.is_empty() {
                queried
            } else {
                p.default_model.iter().map(|m| ModelRow::bare(m)).collect()
            };
            let cli = data::is_cli(&p);
            let hide_cli_default =
                cli && p.default_model.as_deref() == Some("default") && !models.is_empty();
            if let Some(d) = &p.default_model
                && !models.iter().any(|m| &m.id == d)
                && !hide_cli_default
            {
                models.insert(0, ModelRow::bare(d));
            }
            if let Some((sp, sm)) = sel
                && *sp == p.name
                && !sm.is_empty()
                && !(cli && sm == "default" && !models.is_empty())
                && !models.iter().any(|m| &m.id == sm)
            {
                models.insert(0, ModelRow::bare(sm));
            }
            if cli {
                for m in &mut models {
                    if m.id == "default" && m.display_name.is_none() {
                        m.display_name = Some(cli_default.clone());
                    }
                }
                return Group {
                    provider: p,
                    models,
                    hidden: 0,
                };
            }
            let kept = |m: &ModelRow| {
                data::runs_assistant(m)
                    || p.default_model.as_deref() == Some(m.id.as_str())
                    || sel.is_some_and(|(sp, sm)| *sp == p.name && *sm == m.id)
                    || favorites
                        .iter()
                        .any(|(fp, fm)| *fp == p.name && *fm == m.id)
            };
            let usable: Vec<ModelRow> = models.iter().filter(|m| kept(m)).cloned().collect();
            let hidden = models.len() - usable.len();
            Group {
                models: if app.assistant.picker.show_all {
                    models
                } else {
                    usable
                },
                provider: p,
                hidden,
            }
        })
        .collect()
}

/// A row of the list.
#[derive(Clone, Debug)]
pub struct Item {
    pub key: String,
    pub provider: Provider,
    pub model: ModelRow,
    pub posture: &'static str,
    pub favorite: bool,
}

fn item(app: &App, p: &Provider, m: &ModelRow) -> Item {
    let key = data::model_key(&p.name, &m.id);
    let favorite = app
        .assistant
        .picker
        .favorites
        .iter()
        .any(|(fp, fm)| *fp == p.name && *fm == m.id);
    Item {
        key,
        provider: p.clone(),
        posture: data::model_posture(p, m),
        model: m.clone(),
        favorite,
    }
}

fn group_items(app: &App, g: &Group) -> Vec<Item> {
    let favs = &app.assistant.picker.favorites;
    let is_fav = |m: &ModelRow| {
        favs.iter()
            .any(|(fp, fm)| *fp == g.provider.name && *fm == m.id)
    };
    let mut models = g.models.clone();
    models.sort_by(|a, b| is_fav(b).cmp(&is_fav(a)).then(data::compare_prefix(a, b)));
    let mut items: Vec<Item> = models.iter().map(|m| item(app, &g.provider, m)).collect();
    data::sort_by_posture(&mut items, |i| i.posture);
    items
}

fn favorite_items(app: &App, groups: &[Group]) -> Vec<Item> {
    let mut out = vec![];
    for (fp, fm) in &app.assistant.picker.favorites {
        for g in groups.iter().filter(|g| g.provider.name == *fp) {
            let m = g
                .models
                .iter()
                .find(|m| m.id == *fm)
                .cloned()
                .unwrap_or_else(|| ModelRow::bare(fm));
            out.push(item(app, &g.provider, &m));
        }
    }
    out
}

/// `initialModelPickerView`: Favorites when the current model is starred, else its provider,
/// else the default provider, else the first.
pub fn initial_view(app: &App, groups: &[Group]) -> View {
    let sel = app.assistant.selection.as_ref();
    if let Some((p, m)) = sel
        && app
            .assistant
            .picker
            .favorites
            .iter()
            .any(|(fp, fm)| fp == p && fm == m)
    {
        return View::Favorites;
    }
    let current = sel.and_then(|(p, _)| groups.iter().find(|g| g.provider.name == *p));
    current
        .or_else(|| groups.iter().find(|g| g.provider.is_default))
        .or(groups.first())
        .map(|g| View::Provider(g.provider.name.clone()))
        .unwrap_or(View::Favorites)
}

/// The view in force (a removed provider falls back to the first).
pub fn active_view(app: &App, groups: &[Group]) -> View {
    match &app.assistant.picker.view {
        None => initial_view(app, groups),
        Some(View::Favorites) => View::Favorites,
        Some(View::Provider(p)) if groups.iter().any(|g| g.provider.name == *p) => {
            View::Provider(p.clone())
        }
        Some(_) => groups
            .first()
            .map(|g| View::Provider(g.provider.name.clone()))
            .unwrap_or(View::Favorites),
    }
}

pub fn searching(app: &App) -> bool {
    !app.assistant.picker.query.text.trim().is_empty()
}

/// The rows in view: search results across every provider, the Favorites, or one provider's.
pub fn items(app: &App, groups: &[Group]) -> Vec<Item> {
    if searching(app) {
        let mut all: Vec<Item> = vec![];
        for it in groups
            .iter()
            .flat_map(|g| group_items(app, g))
            .chain(favorite_items(app, groups))
        {
            if !all.iter().any(|a| a.key == it.key) {
                all.push(it);
            }
        }
        data::sort_by_posture(&mut all, |i| i.posture);
        let q = app.assistant.picker.query.text.clone();
        let labelled: Vec<(String, Item)> =
            all.into_iter().map(|i| (i.provider.label(), i)).collect();
        return data::rank(labelled, &q, |(label, it)| data::Searchable {
            id: &it.model.id,
            display_name: it.model.display_name.as_deref(),
            owned_by: it.model.owned_by.as_deref(),
            source_provider: it.model.source_provider.as_deref(),
            provider_name: &it.provider.name,
            provider_display_name: label,
            favorite: it.favorite,
        })
        .into_iter()
        .map(|(_, it)| it)
        .collect();
    }
    match active_view(app, groups) {
        View::Favorites => favorite_items(app, groups),
        View::Provider(p) => groups
            .iter()
            .find(|g| g.provider.name == p)
            .map(|g| group_items(app, g))
            .unwrap_or_default(),
    }
}

/// The highlighted row's index (explicit, else the selection, else the first).
pub fn highlighted(app: &App, items: &[Item]) -> Option<usize> {
    let s = &app.assistant.picker;
    let sel = app
        .assistant
        .selection
        .as_ref()
        .map(|(p, m)| data::model_key(p, m));
    s.highlight
        .as_ref()
        .and_then(|h| items.iter().position(|i| &i.key == h))
        .or_else(|| sel.and_then(|k| items.iter().position(|i| i.key == k)))
        .or(if items.is_empty() { None } else { Some(0) })
}

/// Why the last check of `p` left no fresh listing: the error of a check run here, else the
/// cached listing's (stale or failed). Only ever the outcome of a check the user started.
pub fn provider_warning(app: &App, p: &Provider) -> Option<String> {
    if let Some(e) = app.assistant.checks.check_errors.get(&p.name) {
        return Some(e.clone());
    }
    snapshot(app, p)
        .filter(|s| s.stale || s.error.is_some())
        .map(|s| s.error.unwrap_or_default())
}

/// The selected provider, its listed row, and whether a check listed it.
pub fn selected(app: &App) -> (Option<Provider>, Option<ModelRow>, bool) {
    let Some((sp, sm)) = &app.assistant.selection else {
        return (None, None, false);
    };
    let Some(p) = providers(app).into_iter().find(|p| p.name == *sp) else {
        return (None, None, false);
    };
    let listed = snapshot(app, &p).and_then(|s| s.models.into_iter().find(|m| m.id == *sm));
    let known = listed.is_some();
    (Some(p), listed, known)
}

/// Whether the selected model takes a reasoning effort (`useSupportedReasoningEffort`).
pub fn effort_shown(app: &App) -> bool {
    let (p, m, _) = selected(app);
    data::supports_effort(p.as_ref(), m.as_ref())
}

/// The levels the selected model advertises (lowest first), and whether a check listed it.
pub fn effort_options(app: &App) -> (Vec<String>, bool) {
    let (p, m, known) = selected(app);
    let adv = data::advertised_efforts(p.as_ref(), m.as_ref());
    (
        data::KNOWN_EFFORTS
            .iter()
            .filter(|e| adv.iter().any(|a| a == *e))
            .map(|s| s.to_string())
            .collect(),
        known,
    )
}

// ---- Actions -----------------------------------------------------------------------------------

/// Opens the picker over `host`'s composer, the search field focused (or the reasoning levels).
pub fn open(app: &mut App, host: &'static str, to_effort: bool) {
    let g = groups(app);
    let view = if !g.is_empty() {
        Some(initial_view(app, &g))
    } else {
        None
    };
    let s = &mut app.assistant.picker;
    s.open = Some(host);
    s.view = view;
    s.query = TextEdit::new("");
    s.query.focused = true;
    s.highlight = None;
    s.keyboard_highlight = false;
    s.footer_error = None;
    s.perm_open = false;
    s.focus = if to_effort {
        Focus::Effort
    } else {
        Focus::Search
    };
    s.focus_gen += 1;
}

pub fn close(app: &mut App) {
    let s = &mut app.assistant.picker;
    s.open = None;
    s.perm_open = false;
    s.footer_error = None;
}

pub fn toggle(app: &mut App, host: &'static str) {
    if app.assistant.picker.open == Some(host) {
        close(app);
    } else {
        open(app, host, false);
    }
}

pub fn focus(app: &mut App, f: Focus) {
    app.assistant.picker.focus = f;
    app.assistant.picker.focus_gen += 1;
    app.assistant.picker.query.focused = f == Focus::Search;
}

/// `selectView`: a rail press clears the search and returns focus to it.
pub fn select_view(app: &mut App, v: View) {
    let s = &mut app.assistant.picker;
    s.view = Some(v);
    s.query = TextEdit::new("");
    s.highlight = None;
    focus(app, Focus::Search);
}

/// `selectModel`: an off-device provider needs Kassiber's acknowledgement first (`checks::pick`
/// asks, acknowledges, then commits). A pick moves on to the new model's levels, or closes.
pub fn pick(app: &mut App, provider: &str, model: &str) {
    if !providers(app).iter().any(|p| p.name == provider) {
        return;
    }
    crate::assistant::checks::pick(app, provider, model);
}

/// The pick itself, once the provider may be used.
pub fn commit_pick(app: &mut App, provider: &str, model: &str) {
    app.assistant.selection = Some((provider.to_owned(), model.to_owned()));
    app.assistant.picker.footer_error = None;
    let (opts, _) = effort_options(app);
    if !opts.is_empty() && app.assistant.effort != "auto" && !opts.contains(&app.assistant.effort) {
        app.assistant.effort = "auto".into();
    }
    if app.assistant.picker.open.is_none() {
        return;
    }
    if effort_shown(app) {
        focus(app, Focus::Effort);
    } else {
        close(app);
    }
}

/// `useFastModeSupport(selection).supported`.
pub fn fast_supported(app: &App) -> bool {
    let (p, m, _) = selected(app);
    data::fast_mode(p.as_ref(), m.as_ref()).0
}

pub fn toggle_favorite(app: &mut App, provider: &str, model: &str) {
    let favs = &mut app.assistant.picker.favorites;
    if let Some(i) = favs.iter().position(|(p, m)| p == provider && m == model) {
        favs.remove(i);
    } else {
        favs.push((provider.to_owned(), model.to_owned()));
    }
}

fn move_highlight(app: &mut App, delta: i32) {
    let g = groups(app);
    let items = items(app, &g);
    if items.is_empty() {
        return;
    }
    let n = items.len() as i32;
    let start = highlighted(app, &items)
        .map(|i| i as i32)
        .unwrap_or(if delta > 0 { -1 } else { 0 });
    let next = ((start + delta) % n + n) % n;
    app.assistant.picker.highlight = Some(items[next as usize].key.clone());
    app.assistant.picker.keyboard_highlight = true;
}

fn pick_index(app: &mut App, i: usize) {
    let g = groups(app);
    let items = items(app, &g);
    if let Some(it) = items.get(i) {
        let (p, m) = (it.provider.name.clone(), it.model.id.clone());
        pick(app, &p, &m);
    }
}

fn switch_provider(app: &mut App, step: i32) {
    let g = groups(app);
    let mut rail = vec![View::Favorites];
    rail.extend(g.iter().map(|g| View::Provider(g.provider.name.clone())));
    let cur = rail
        .iter()
        .position(|v| *v == active_view(app, &g))
        .map(|i| i as i32)
        .unwrap_or(-1);
    let n = rail.len() as i32;
    let next = if cur < 0 {
        0
    } else {
        ((cur + step) % n + n) % n
    };
    select_view(app, rail[next as usize].clone());
}

/// Up/Down on the rail: the next view, keeping the focus on the rail (the search clears).
pub fn rail_step(app: &mut App, step: i32) {
    let g = groups(app);
    let mut rail = vec![View::Favorites];
    rail.extend(g.iter().map(|g| View::Provider(g.provider.name.clone())));
    let cur = if searching(app) {
        -1
    } else {
        rail.iter()
            .position(|v| *v == active_view(app, &g))
            .map(|i| i as i32)
            .unwrap_or(-1)
    };
    let n = rail.len() as i32;
    let next = if cur < 0 {
        0
    } else {
        (cur + step).clamp(0, n - 1)
    };
    let s = &mut app.assistant.picker;
    s.view = Some(rail[next as usize].clone());
    s.query = TextEdit::new("");
    s.highlight = None;
    focus(app, Focus::Rail);
}

/// The character a chord carries, however the platform reports it.
fn chord_char(k: &Key) -> Option<char> {
    match k {
        Key::Command(c) => Some(c.to_ascii_lowercase()),
        Key::Text(s) => s.chars().next().map(|c| c.to_ascii_lowercase()),
        _ => None,
    }
}

/// The search field's keys (ModelPickerContent `handleSearchKeyDown` + the picker's shortcuts:
/// ⌘1…9 pick, ⌘D stars, ⌘⇧↑/↓ switch provider).
pub fn search_key(app: &mut App, ev: &KeyEvent) -> bool {
    let m = ev.mods;
    if (m.cmd || m.ctrl) && !m.alt {
        if m.shift && matches!(ev.key, Key::Up | Key::Down) {
            switch_provider(app, if ev.key == Key::Down { 1 } else { -1 });
            return true;
        }
        if m.shift {
            return false;
        }
        match chord_char(&ev.key) {
            Some('d') => {
                let g = groups(app);
                let items = items(app, &g);
                if let Some(it) = highlighted(app, &items).and_then(|i| items.get(i)) {
                    let (p, mm) = (it.provider.name.clone(), it.model.id.clone());
                    toggle_favorite(app, &p, &mm);
                }
                true
            }
            Some(c @ '1'..='9') => {
                pick_index(app, c as usize - '1' as usize);
                true
            }
            _ => false,
        }
    } else if m.alt {
        false
    } else {
        match ev.key {
            Key::Down => move_highlight(app, 1),
            Key::Up => move_highlight(app, -1),
            Key::Enter => {
                let g = groups(app);
                let items = items(app, &g);
                if let Some(i) = highlighted(app, &items) {
                    pick_index(app, i);
                }
            }
            Key::Right
                if !m.shift
                    && effort_shown(app)
                    && app.assistant.picker.query.focus
                        >= app.assistant.picker.query.text.len() =>
            {
                let g = groups(app);
                let items = items(app, &g);
                let sel = app
                    .assistant
                    .selection
                    .as_ref()
                    .map(|(p, mm)| data::model_key(p, mm));
                let s = &app.assistant.picker;
                match highlighted(app, &items).and_then(|i| items.get(i)) {
                    Some(it)
                        if Some(&it.key) != sel.as_ref()
                            && s.keyboard_highlight
                            && s.highlight.is_some() =>
                    {
                        let (p, mm) = (it.provider.name.clone(), it.model.id.clone());
                        pick(app, &p, &mm);
                    }
                    _ => focus(app, Focus::Effort),
                }
            }
            Key::Left if !m.shift && app.assistant.picker.query.text.is_empty() => {
                focus(app, Focus::Rail)
            }
            Key::Tab if m.shift && !searching(app) => focus(app, Focus::Rail),
            _ => return false,
        }
        true
    }
}

/// Which composer a shortcut aims at: the page's on `/assistant`, else the panel's.
fn host_for_shortcut(app: &App) -> Option<&'static str> {
    if crate::assistant::owns(app.route.path()) {
        Some("page")
    } else if crate::assistant::assistant_in_view(app) {
        Some("panel")
    } else {
        None
    }
}

/// Every frame: the local restore reads, the seeded selection, and dropping a level the
/// selected model does not take.
pub fn poll(app: &mut App, _tx: &mut Tx) -> bool {
    let mut changed = false;
    let ready = matches!(app.assistant.providers, Load::Ready(_));
    if !ready {
        app.assistant.picker.restored = false;
        return false;
    }
    if !app.assistant.picker.restored && app.daemon.is_some() {
        app.assistant.picker.restored = true;
        // `useRestoreProviderModelChecks`: display reads of earlier checks, never a probe.
        let checked: Vec<(String, String)> = app
            .assistant
            .providers
            .ready()
            .into_iter()
            .flatten()
            .filter(|p| p.enabled && p.last_checked_at.is_some())
            .map(|p| (p.name.clone(), p.base_url.clone()))
            .collect();
        for (name, url) in checked {
            app.send(
                "ai.list_models",
                json!({"provider": name, "base_url": url, "cached_only": true}),
                "assistant.picker.models",
            );
        }
        changed = true;
    }
    // A remembered selection whose provider was switched off falls back to the default.
    if let Some((sp, _)) = &app.assistant.selection
        && app
            .assistant
            .providers
            .ready()
            .is_some_and(|all| all.iter().any(|p| p.name == *sp && !p.enabled))
    {
        app.assistant.selection = None;
        changed = true;
    }
    // Seed a selection from the default provider's saved default model, or the first usable
    // model a previous check listed; a CLI's "default" yields to its first listed model.
    let provs = providers(app);
    let fallback = provs
        .iter()
        .find(|p| p.is_default)
        .or(provs.first())
        .cloned();
    match app.assistant.selection.clone() {
        Some((sp, sm)) if sm == "default" => {
            if let Some(p) = provs.iter().find(|p| p.name == sp && data::is_cli(p))
                && let Some(first) = snapshot(app, p)
                    .and_then(|s| s.models.into_iter().next())
                    .filter(|m| m.id != "default")
            {
                app.assistant.selection = Some((sp, first.id));
                changed = true;
            }
        }
        None => {
            if let Some(p) = fallback {
                if let Some(d) = &p.default_model {
                    app.assistant.selection = Some((p.name.clone(), d.clone()));
                    changed = true;
                } else if let Some(m) = snapshot(app, &p).and_then(|s| {
                    if data::is_cli(&p) {
                        s.models.into_iter().next()
                    } else {
                        s.models.into_iter().find(data::runs_assistant)
                    }
                }) {
                    app.assistant.selection = Some((p.name.clone(), m.id));
                    changed = true;
                }
            }
        }
        _ => {}
    }
    // `useSupportedReasoningEffort`: a model known not to take one drops back to Auto.
    if app.assistant.effort != "auto" {
        let (p, m, known) = selected(app);
        let resolved = app.assistant.selection.is_none()
            || p.is_none()
            || p.as_ref().is_some_and(data::provider_supports_effort)
            || known;
        if resolved && !data::supports_effort(p.as_ref(), m.as_ref()) {
            app.assistant.effort = "auto".into();
            changed = true;
        }
    }
    if app.assistant.picker.open.is_some() && !app.prefs.ai_features {
        close(app);
        changed = true;
    }
    changed
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag != "assistant.picker.models" || r.error.is_some() {
        return;
    }
    let Some(name) = r.data["provider"].as_str() else {
        return;
    };
    let snap = Snapshot {
        models: ModelRow::parse_list(&r.data["models"]),
        stale: r.data["stale"].as_bool().unwrap_or(false),
        error: r.data["error"]["message"].as_str().map(str::to_owned),
        checked_at: r.data["checked_at"].as_str().map(str::to_owned),
    };
    // An unchecked endpoint contributes nothing.
    if snap.checked_at.is_some() {
        app.assistant.picker.listings.insert(name.to_owned(), snap);
    }
}

/// ⌘⇧M toggles the picker, ⌘⇧K opens it on the reasoning levels (or moves there), Escape
/// closes the innermost of the permission menu and the picker.
pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let s = &app.assistant.picker;
    if matches!(ev.key, Key::Escape) {
        if s.perm_open {
            app.assistant.picker.perm_open = false;
            return true;
        }
        if s.open.is_some() {
            close(app);
            return true;
        }
        return false;
    }
    let m = ev.mods;
    if !(m.cmd || m.ctrl) || !m.shift || m.alt {
        return false;
    }
    let Some(c) = chord_char(&ev.key).filter(|c| *c == 'm' || *c == 'k') else {
        return false;
    };
    // Scoped to the composer: its field has focus, or its picker is open.
    if !(app.assistant.draft.focused || s.open.is_some()) {
        return false;
    }
    let Some(host) = s.open.or_else(|| host_for_shortcut(app)) else {
        return false;
    };
    if c == 'm' {
        toggle(app, host);
    } else if s.open == Some(host) {
        let f = if effort_shown(app) {
            Focus::Effort
        } else {
            Focus::Search
        };
        focus(app, f);
    } else {
        let to_effort = effort_shown(app);
        open(app, host, to_effort);
    }
    true
}

/// The trigger in the composer toolbar.
pub fn trigger(app: &App, tk: &Tk, host: &'static str) -> El {
    view::trigger(app, tk, host)
}

/// The open picker's popover, anchored to `trigger(…, host)`.
pub fn mount(app: &App, cx: &mut Cx<App>, tk: &Tk, host: &'static str, layer: &mut Vec<El>) {
    view::popover(app, cx, tk, host, layer);
}

fn scene(a: &mut App, selection: Option<(&str, &str)>) {
    a.assistant.selection = selection.map(|(p, m)| (p.to_owned(), m.to_owned()));
    a.navigate("/assistant");
    crate::assistant::prime(a);
    open(a, "page", false);
}

/// SYNTHETIC: what a Check models of Ollama could have left (no provider is contacted): chat
/// models, one routed to Ollama's cloud, one with Fast, and an embedding model the usability
/// filter hides.
fn synthetic_listing(a: &mut App) {
    let row = |id: &str| ModelRow::bare(id);
    let mut cloud = row("gpt-oss:120b-cloud");
    cloud.privacy_posture = Some("remote".into());
    cloud.privacy_reason = Some("Ollama Cloud runs this model on ollama.com".into());
    let mut fast = row("qwen3:4b");
    fast.display_name = Some("Qwen3 4B".into());
    let models = vec![
        row("qwen3.6:35b"),
        row("gpt-oss:20b"),
        row("llama3.3:70b"),
        fast,
        cloud,
        row("nomic-embed-text"),
    ];
    a.assistant.picker.listings.insert(
        "ollama".into(),
        Snapshot {
            models,
            stale: false,
            error: None,
            checked_at: Some("2026-11-16T08:00:00Z".into()),
        },
    );
}

fn scene_with_models(a: &mut App) {
    synthetic_listing(a);
    scene(a, Some(("ollama", "qwen3.6:35b")));
}

/// The picker open over the page composer, no model chosen.
pub fn scene_open(a: &mut App) {
    scene(a, None)
}
/// Open with ollama · qwen3.6:35b selected (the Tauri capture's state).
pub fn scene_model(a: &mut App) {
    scene(a, Some(("ollama", "qwen3.6:35b")))
}
/// A coding agent that takes reasoning levels: the column beside the list, a level chosen.
pub fn scene_effort(a: &mut App) {
    scene(a, Some(("claude", "default")));
    a.assistant.effort = "high".into();
    focus(a, Focus::Effort);
}
/// The footer's permission-mode menu open.
pub fn scene_permission(a: &mut App) {
    scene(a, Some(("ollama", "qwen3.6:35b")));
    a.assistant.picker.perm_open = true;
}
/// A query ranking across every provider.
pub fn scene_search(a: &mut App) {
    scene(a, Some(("ollama", "qwen3.6:35b")));
    let mut e = TextEdit::new("de");
    e.focused = true;
    e.set_caret(2);
    a.assistant.picker.query = e;
}
/// SYNTHETIC: starred models under Favorites (the favorites store is empty on a fresh book).
pub fn scene_favorites(a: &mut App) {
    a.assistant.picker.favorites = vec![
        ("ollama".into(), "qwen3.6:35b".into()),
        ("claude".into(), "default".into()),
    ];
    scene(a, Some(("ollama", "qwen3.6:35b")));
}
pub fn scene_de(a: &mut App) {
    a.prefs.lang = crate::i18n::Lang::De;
    scene_with_models(a);
}
pub fn scene_light(a: &mut App) {
    a.prefs.theme = crate::app::ThemeMode::Light;
    scene_with_models(a);
}
/// SYNTHETIC: a provider with models (a cached listing; nothing is checked).
pub fn scene_models(a: &mut App) {
    scene_with_models(a);
}
/// SYNTHETIC: the models listing, a row highlighted by the keyboard (its star and ⌘-digit).
pub fn scene_highlight(a: &mut App) {
    scene_with_models(a);
    a.assistant.picker.highlight = Some(data::model_key("ollama", "gpt-oss:20b"));
    a.assistant.picker.keyboard_highlight = true;
}
/// An off-device provider's header: where its prompts go, in words.
pub fn scene_remote(a: &mut App) {
    scene(a, None);
    let remote = providers(a)
        .into_iter()
        .find(|p| data::posture(&p.kind) != "local")
        .map(|p| p.name);
    if let Some(p) = remote {
        a.assistant.picker.view = Some(View::Provider(p));
    }
}
/// SYNTHETIC: Ollama's last check failed (the rail's warning glyph and the header's line).
pub fn scene_warning(a: &mut App) {
    scene(a, None);
    a.assistant.checks.check_errors.insert(
        "ollama".into(),
        "Could not reach http://localhost:11434/v1: connection refused.".into(),
    );
    a.assistant.picker.view = Some(View::Provider("ollama".into()));
}
/// SYNTHETIC: searching across providers with a listing.
pub fn scene_search_models(a: &mut App) {
    scene_with_models(a);
    let mut e = TextEdit::new("gpt");
    e.focused = true;
    e.set_caret(3);
    a.assistant.picker.query = e;
}
/// The picker in the docked side panel's composer (on the Overview), and maximized.
pub fn scene_panel(a: &mut App) {
    synthetic_listing(a);
    a.assistant.selection = Some(("ollama".into(), "qwen3.6:35b".into()));
    a.navigate("/overview");
    a.prefs.right_panel_open = true;
    a.assistant.tab_open = true;
    crate::assistant::prime(a);
    open(a, "panel", false);
}
pub fn scene_panel_max(a: &mut App) {
    scene_panel(a);
    a.detail.maximized = true;
}
/// The closed trigger with its traits: a reasoning level, and Full access named in the toolbar.
pub fn scene_trigger_traits(a: &mut App) {
    a.assistant.selection = Some(("claude".into(), "default".into()));
    a.assistant.effort = "high".into();
    a.assistant.permission = crate::assistant::PermissionMode::FullAccess;
    a.navigate("/assistant");
    crate::assistant::prime(a);
}
