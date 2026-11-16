//! Provider checks the user starts from the assistant, gated exactly as the web gates them:
//!
//! - **Check models** (ProviderModelPicker `checkModels`): `ai.list_models` with `refresh` for
//!   that one provider, only from the button.
//! - Picking a model of an off-device provider never acknowledged (`selectModel`), or checking
//!   one, first asks (`window.confirm`, here an in-app alert dialog with Cancel and OK) and then
//!   sends `ai.providers.acknowledge`.
//! - **Use** / **Try again** on the model setup card (`ModelSetupCard` + `useAiProviderChecks`):
//!   switches a disabled agent on, refuses off-device checks while offline, asks before an
//!   unacknowledged one, runs `ai.provider_runtime.status` with `refresh`, and makes a ready
//!   agent the default and the chat's model.
//!
//! Mounting, opening the picker and rendering never contact a provider; nothing here runs while
//! AI features are off.

use std::collections::{HashMap, HashSet};

use rune::{El, col, row, text};
use serde_json::{Value, json};

use super::model::Provider;
use super::picker::data::{ModelRow, Snapshot};
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;

/// What an accepted confirmation goes on to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum After {
    Pick(String, String),
    Discover(String),
    SetUp(String),
}

#[derive(Default)]
pub struct State {
    /// The open confirmation: its message and what OK does.
    pub confirm: Option<(String, After)>,
    /// Acknowledgements in flight: request id → what follows.
    pub acks: Vec<(String, After)>,
    /// Model checks in flight (provider names) and their errors.
    pub checking: HashSet<String>,
    pub check_errors: HashMap<String, String>,
    pub check_ids: Vec<(String, String)>,
    /// The picker footer's acknowledgement error (`ackError`).
    pub ack_error: Option<String>,
    /// The setup card: the agent being started, the one whose check decides, its failure.
    pub starting: Option<String>,
    pub checked: Option<String>,
    pub failure: Option<(String, String)>,
    /// Runtime checks in flight and their refusals (`offline` / `declined`).
    pub runtime_pending: HashSet<String>,
    pub runtime_ids: Vec<(String, String)>,
    pub refusals: HashMap<String, &'static str>,
    pub enable_ids: Vec<(String, String)>,
}

fn find(app: &App, name: &str) -> Option<Provider> {
    app.assistant
        .providers
        .ready()?
        .iter()
        .find(|p| p.name == name)
        .cloned()
}

fn offline(app: &App) -> bool {
    app.read("ui.network.offline")
        .is_some_and(|v| v["offline"].as_bool() == Some(true))
}

fn take<T>(app: &App, list: &mut Vec<(String, T)>) -> Option<T> {
    let i = list
        .iter()
        .position(|(id, _)| !app.pending.contains_key(id))?;
    Some(list.remove(i).1)
}

/// `selectModel`: an off-device provider needs Kassiber's acknowledgement first.
pub fn pick(app: &mut App, provider: &str, model: &str) {
    let Some(p) = find(app, provider) else { return };
    if p.kind == "local" || p.acknowledged_at.is_some() {
        super::picker::commit_pick(app, provider, model);
        return;
    }
    let msg = tr(
        "assistant:modelPicker.remoteConfirm",
        &[("provider", &p.label())],
    );
    app.assistant.checks.confirm = Some((msg, After::Pick(provider.to_owned(), model.to_owned())));
}

/// **Check models** for one provider (`checkModels`).
pub fn check_models(app: &mut App, provider: &str) {
    if !app.prefs.ai_features || app.assistant.checks.checking.contains(provider) {
        return;
    }
    let Some(p) = find(app, provider) else { return };
    if p.kind != "local" && p.acknowledged_at.is_none() {
        let msg = tr(
            "assistant:modelPicker.remoteDiscoveryConfirm",
            &[("provider", &p.label())],
        );
        app.assistant.checks.confirm = Some((msg, After::Discover(provider.to_owned())));
        return;
    }
    run_discovery(app, &p);
}

fn run_discovery(app: &mut App, p: &Provider) {
    if !app.prefs.ai_features {
        return;
    }
    let s = &mut app.assistant.checks;
    s.checking.insert(p.name.clone());
    s.check_errors.remove(&p.name);
    // The request names the endpoint being checked, so an edited row is refused, not probed.
    if let Some(id) = app.send(
        "ai.list_models",
        json!({"provider": p.name, "base_url": p.base_url, "refresh": true}),
        "assistant.check_models",
    ) {
        app.assistant.checks.check_ids.push((id, p.name.clone()));
    }
}

/// **Use** / **Try again** on the setup card (`use`).
pub fn set_up(app: &mut App, name: &str) {
    if !app.prefs.ai_features || app.assistant.checks.starting.is_some() {
        return;
    }
    let Some(p) = find(app, name) else { return };
    let s = &mut app.assistant.checks;
    s.starting = Some(name.to_owned());
    s.failure = None;
    if !p.enabled {
        // A switched-off provider is never contacted, so it is switched on first.
        if let Some(id) = app.send(
            "ai.providers.update",
            json!({"name": name, "enabled": true}),
            "assistant.setup_enable",
        ) {
            app.assistant.checks.enable_ids.push((id, name.to_owned()));
        }
        return;
    }
    check_runtime(app, &p);
}

/// `check(row)` of `useAiProviderChecks`: offline refusal, acknowledgement, then the check.
fn check_runtime(app: &mut App, p: &Provider) {
    app.assistant.checks.refusals.remove(&p.name);
    if offline(app) && p.kind != "local" {
        app.assistant
            .checks
            .refusals
            .insert(p.name.clone(), "offline");
        app.assistant.checks.starting = None;
        return;
    }
    if p.kind != "local" && p.acknowledged_at.is_none() {
        let msg = tc("settings:ai.checkConfirm", 1, &[("names", &p.label())]);
        app.assistant.checks.confirm = Some((msg, After::SetUp(p.name.clone())));
        return;
    }
    run_runtime(app, p);
}

fn run_runtime(app: &mut App, p: &Provider) {
    if !app.prefs.ai_features {
        app.assistant.checks.starting = None;
        return;
    }
    app.assistant.checks.runtime_pending.insert(p.name.clone());
    let args = if super::model::is_cli_locator(&p.base_url) {
        json!({"provider": p.name, "base_url": p.base_url, "refresh": true})
    } else {
        // An HTTP provider's check is its model list.
        json!({"provider": p.name, "base_url": p.base_url, "refresh": true})
    };
    let kind = if super::model::is_cli_locator(&p.base_url) {
        "ai.provider_runtime.status"
    } else {
        "ai.list_models"
    };
    if let Some(id) = app.send(kind, args, "assistant.setup_check") {
        app.assistant.checks.runtime_ids.push((id, p.name.clone()));
    }
    app.assistant.checks.starting = None;
    app.assistant.checks.checked = Some(p.name.clone());
}

/// The confirmation's OK.
pub fn accept(app: &mut App) {
    let Some((_, after)) = app.assistant.checks.confirm.take() else {
        return;
    };
    let name = match &after {
        After::Pick(p, _) | After::Discover(p) | After::SetUp(p) => p.clone(),
    };
    if !app.prefs.ai_features {
        return;
    }
    app.assistant.checks.ack_error = None;
    if let Some(id) = app.send(
        "ai.providers.acknowledge",
        json!({"name": name}),
        "assistant.ack",
    ) {
        app.assistant.checks.acks.push((id, after));
    }
}

/// The confirmation's Cancel (Escape, the scrim): nothing is acknowledged or contacted.
pub fn cancel(app: &mut App) {
    if let Some((_, After::SetUp(name))) = app.assistant.checks.confirm.take() {
        app.assistant.checks.refusals.insert(name, "declined");
        app.assistant.checks.starting = None;
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "assistant.ack" => {
            let mut list = std::mem::take(&mut app.assistant.checks.acks);
            let after = take(app, &mut list);
            app.assistant.checks.acks = list;
            let Some(after) = after else { return };
            if let Some(e) = &r.error {
                app.assistant.checks.ack_error = Some(e.clone());
                if let After::SetUp(n) = &after {
                    app.assistant.checks.failure = Some((n.clone(), e.clone()));
                    app.assistant.checks.starting = None;
                }
                return;
            }
            // The rows now carry the acknowledgement (`providersQuery.refetch()`).
            super::load_providers(app);
            match after {
                After::Pick(p, m) => super::picker::commit_pick(app, &p, &m),
                After::Discover(p) => {
                    if let Some(row) = find(app, &p) {
                        run_discovery(app, &row);
                    }
                }
                After::SetUp(p) => {
                    if let Some(row) = find(app, &p) {
                        run_runtime(app, &row);
                    }
                }
            }
        }
        "assistant.check_models" => {
            let mut list = std::mem::take(&mut app.assistant.checks.check_ids);
            let name = take(app, &mut list);
            app.assistant.checks.check_ids = list;
            let Some(name) = name else { return };
            app.assistant.checks.checking.remove(&name);
            match &r.error {
                Some(e) => {
                    app.assistant.checks.check_errors.insert(name, e.clone());
                }
                None => store_listing(app, &name, &r.data),
            }
        }
        "assistant.setup_enable" => {
            let mut list = std::mem::take(&mut app.assistant.checks.enable_ids);
            let name = take(app, &mut list);
            app.assistant.checks.enable_ids = list;
            let Some(name) = name else { return };
            match &r.error {
                Some(e) => {
                    app.assistant.checks.failure = Some((name, e.clone()));
                    app.assistant.checks.starting = None;
                }
                None => {
                    super::load_providers(app);
                    if let Some(mut p) = find(app, &name) {
                        p.enabled = true;
                        check_runtime(app, &p);
                    }
                }
            }
        }
        "assistant.setup_check" => {
            let mut list = std::mem::take(&mut app.assistant.checks.runtime_ids);
            let name = take(app, &mut list);
            app.assistant.checks.runtime_ids = list;
            let Some(name) = name else { return };
            app.assistant.checks.runtime_pending.remove(&name);
            super::load_providers(app);
            if let Some(e) = &r.error {
                app.assistant.checks.failure = Some((name.clone(), e.clone()));
                app.assistant.checks.checked = None;
                return;
            }
            if r.kind == "ai.provider_runtime.status" {
                app.assistant.runtime.retain(|(n, _)| *n != name);
                app.assistant.runtime.push((name.clone(), r.data.clone()));
                let st = r.data["providers"]
                    .as_array()
                    .and_then(|a| a.first())
                    .cloned()
                    .unwrap_or(Value::Null);
                if st["state"] == "ready" {
                    let listing = json!({"provider": name, "models": st["models"], "checked_at": r.data["checked_at"], "stale": r.data["stale"], "error": r.data["error"]});
                    store_listing(app, &name, &listing);
                } else {
                    app.assistant.picker.listings.remove(&name);
                }
            } else {
                store_listing(app, &name, &r.data);
            }
            // Only this check counts: a ready, fresh result makes the agent the chat's model.
            if app.assistant.checks.checked.as_deref() == Some(name.as_str()) {
                app.assistant.checks.checked = None;
                let stale = r.data["stale"].as_bool() == Some(true);
                let ready = r.kind != "ai.provider_runtime.status"
                    || r.data["providers"][0]["state"] == "ready";
                if ready && !stale {
                    let _ = app.send(
                        "ai.providers.set_default",
                        json!({"name": name}),
                        "assistant.set_default",
                    );
                    let model = find(app, &name)
                        .and_then(|p| p.default_model)
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "default".into());
                    app.assistant.selection = Some((name, model));
                }
            }
        }
        "assistant.set_default" => super::load_providers(app),
        _ => {}
    }
}

fn store_listing(app: &mut App, name: &str, d: &Value) {
    let snap = Snapshot {
        models: ModelRow::parse_list(&d["models"]),
        stale: d["stale"].as_bool().unwrap_or(false),
        error: d["error"]["message"].as_str().map(str::to_owned),
        checked_at: d["checked_at"].as_str().map(str::to_owned),
    };
    app.assistant.picker.listings.insert(name.to_owned(), snap);
}

/// The setup card's status line for an agent: (text, is error), or nothing.
pub fn setup_status(app: &App, name: &str) -> Option<(String, bool)> {
    let s = &app.assistant.checks;
    if s.runtime_pending.contains(name) || s.starting.as_deref() == Some(name) {
        return Some((t("settings:ai.status.checking"), false));
    }
    if let Some((n, e)) = &s.failure
        && n == name
    {
        return Some((e.clone(), true));
    }
    if s.refusals.get(name) == Some(&"offline") {
        return Some((t("settings:ai.status.offline"), false));
    }
    None
}

pub fn busy(app: &App, name: &str) -> bool {
    let s = &app.assistant.checks;
    s.runtime_pending.contains(name)
        || s.starting.is_some()
        || s.acks
            .iter()
            .any(|(_, a)| matches!(a, After::SetUp(n) if n == name))
}

/// The confirmation as an alert dialog (`window.confirm`): the message, Cancel and OK.
pub fn dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let Some((msg, _)) = &app.assistant.checks.confirm else {
        return;
    };
    use crate::settings::controls::{Btn, btn_lg};
    let c = col()
        .w_full()
        .gap(tk.r(1.0))
        .role("alertdialog")
        .label(msg.clone())
        .child(text(msg.clone()).font(tk.tsm(400.0)).color(tk.fg).wrap())
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .justify_end()
                .child(
                    btn_lg(
                        tk,
                        "asst-confirm-cancel",
                        &t("common:actions.cancel"),
                        Btn::Outline,
                        true,
                    )
                    .on_press(cancel),
                )
                .child(
                    btn_lg(tk, "asst-confirm-ok", "OK", Btn::Primary, true)
                        .autofocus()
                        .on_press(accept),
                ),
        );
    crate::settings::controls::dialog(
        app,
        tk,
        layer,
        "asst-confirm-dialog",
        tk.r(26.0),
        c,
        cancel,
        true,
    );
}

/// Escape closes the confirmation (Cancel).
pub fn on_escape(app: &mut App) -> bool {
    if app.assistant.checks.confirm.is_some() {
        cancel(app);
        return true;
    }
    false
}
