//! Assistant (AiProvidersSettingsPanel.tsx, AiProviderCard.tsx, useAiProviderChecks.ts,
//! AssistantPermissionSettings.tsx, ChatHistorySettings.tsx, ExternalAgentsSettings.tsx).
//!
//! Privacy contract: opening the section contacts no provider. Each provider's last check is
//! read with `cached_only` (never a probe); a check runs only from Check / Check all / Set up,
//! after the off-device acknowledgement, and offline mode refuses non-local providers first.

use std::collections::HashMap;

use rune::{Color, El, col, el, row, text};
use serde_json::{Value, json};

use super::controls::{
    Btn, PressIf, amber, amber_soft, btn, command_line, copy_button, desc, emerald, group, opt,
    para, select, srow, srow_a, switch,
};
use super::model::{self, is_native_locator};
use super::send;
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

#[derive(Clone, Debug, Default)]
pub struct Check {
    pub runtime: Option<Value>,
    pub models: Vec<Value>,
    pub reachable: bool,
    pub models_listed: bool,
    pub checked_at: Option<String>,
    pub stale: bool,
    pub pending: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct AiUi {
    pub checks: HashMap<String, Check>,
    pub refusals: HashMap<String, &'static str>,
    pub updating: Option<String>,
    pub update_error: Option<String>,
    pub confirming_clear: bool,
    /// Providers to check once their acknowledgements land.
    pub after_ack: Vec<String>,
    pub ack_error: Option<String>,
    /// Providers awaiting the off-device confirmation.
    pub pending_confirm: Vec<String>,
}

#[derive(Default)]
pub struct Agents {
    pub draft_grant: Option<String>,
    pub draft_access: Option<String>,
    pub next_live: f64,
    pub error: bool,
    pub session_error: Option<String>,
    pub existing_lease: bool,
}

pub fn providers(app: &App) -> Vec<Value> {
    app.read("ai.providers.list")
        .and_then(|d| d["providers"].as_array())
        .cloned()
        .unwrap_or_default()
}

fn find(app: &App, name: &str) -> Option<Value> {
    providers(app)
        .into_iter()
        .find(|p| p["name"].as_str() == Some(name))
}

fn name_of(p: &Value) -> String {
    p["display_name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(p["name"].as_str().unwrap_or(""))
        .to_owned()
}

fn enabled(p: &Value) -> bool {
    p["enabled"].as_bool() != Some(false)
}

fn offline(app: &App) -> bool {
    app.read("ui.network.offline")
        .is_some_and(|v| v["offline"].as_bool() == Some(true))
}

pub fn load(app: &mut App) {
    send(
        app,
        "ai.providers.list",
        Value::Null,
        "settings.ai_providers",
    );
    send(
        app,
        "ui.chat.history.configure",
        json!({}),
        "settings.chat_config",
    );
    send(
        app,
        "ui.chat.sessions.list",
        json!({"limit": 200}),
        "settings.chat_sessions",
    );
    send(
        app,
        "ui.agent_access.status",
        Value::Null,
        "settings.agent_access",
    );
    send(app, "status", Value::Null, "settings.status");
    send(app, "ui.network.offline", Value::Null, "settings.offline");
}

/// The daemon's last check of each provider (`cached_only`: never a probe).
fn read_cached(app: &mut App) {
    if !app.prefs.ai_features {
        return;
    }
    for p in providers(app) {
        let name = p["name"].as_str().unwrap_or("").to_owned();
        let url = p["base_url"].as_str().unwrap_or("").to_owned();
        let args = json!({"provider": name, "base_url": url, "cached_only": true});
        if is_native_locator(&url) {
            send(
                app,
                "ai.provider_runtime.status",
                args,
                "settings.ai_runtime_cached",
            );
        } else {
            send(app, "ai.list_models", args, "settings.ai_models_cached");
        }
    }
}

pub fn agent_session_active(app: &App) -> bool {
    app.read("ui.agent_access.status")
        .is_some_and(|s| s["session"]["active"].as_bool() == Some(true))
}

fn apply_runtime(c: &mut Check, data: &Value) {
    let rt = data["providers"].get(0).cloned().filter(|v| !v.is_null());
    c.models = if rt
        .as_ref()
        .is_some_and(|r| r["state"].as_str() == Some("ready"))
    {
        rt.as_ref()
            .and_then(|r| r["models"].as_array().cloned())
            .unwrap_or_default()
    } else {
        vec![]
    };
    c.runtime = rt;
    c.models_listed = false;
    c.checked_at = data["checked_at"].as_str().map(str::to_owned);
    c.stale = data["stale"].as_bool() == Some(true);
    if c.stale {
        c.error = data["error"]["message"].as_str().map(str::to_owned);
    }
}

fn apply_models(c: &mut Check, data: &Value, cli: bool) {
    let has = data["checked_at"].as_str().is_some();
    c.models = data["models"].as_array().cloned().unwrap_or_default();
    c.checked_at = data["checked_at"].as_str().map(str::to_owned);
    c.stale = data["stale"].as_bool() == Some(true);
    if cli {
        // A model list newer than the runtime check: readiness unknown.
        c.models_listed = c
            .runtime
            .as_ref()
            .is_none_or(|r| r["state"].as_str() != Some("ready"));
    } else {
        c.reachable = has && !c.stale;
    }
    if c.stale {
        c.error = data["error"]["message"].as_str().map(str::to_owned);
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let provider = r.data["provider"].as_str().unwrap_or("").to_owned();
    match tag {
        "settings.ai_providers" => read_cached(app),
        "settings.ai_runtime_cached" if r.error.is_none() => {
            apply_runtime(app.settings.ai.checks.entry(provider).or_default(), &r.data)
        }
        "settings.ai_models_cached" if r.error.is_none() => {
            if r.data["checked_at"].as_str().is_some() {
                apply_models(
                    app.settings.ai.checks.entry(provider).or_default(),
                    &r.data,
                    false,
                );
            }
        }
        "settings.ai_check_runtime" | "settings.ai_check_models" => {
            // The reply names its provider; a failed one is matched to the pending check.
            let name = if provider.is_empty() {
                app.settings
                    .ai
                    .checks
                    .iter()
                    .find(|(_, c)| c.pending)
                    .map(|(n, _)| n.clone())
                    .unwrap_or_default()
            } else {
                provider
            };
            let cli = tag == "settings.ai_check_runtime";
            let c = app.settings.ai.checks.entry(name).or_default();
            c.pending = false;
            match &r.error {
                Some(e) => c.error = Some(e.clone()),
                None => {
                    c.error = None;
                    if cli {
                        apply_runtime(c, &r.data)
                    } else {
                        apply_models(c, &r.data, false)
                    }
                }
            }
            send(
                app,
                "ai.providers.list",
                Value::Null,
                "settings.ai_providers_after",
            );
        }
        "settings.ai_ack" => {
            if let Some(e) = &r.error {
                app.settings.ai.ack_error = Some(e.clone());
            }
            if !app.settings.is_busy("settings.ai_ack") {
                let names = std::mem::take(&mut app.settings.ai.after_ack);
                let failed = app.settings.ai.ack_error.take();
                send(
                    app,
                    "ai.providers.list",
                    Value::Null,
                    "settings.ai_providers_after",
                );
                for n in names {
                    match &failed {
                        Some(e) => {
                            app.settings.ai.checks.entry(n).or_default().error = Some(e.clone())
                        }
                        None => run_check(app, &n),
                    }
                }
            }
        }
        "settings.ai_update"
        | "settings.ai_set_default"
        | "settings.ai_delete"
        | "settings.ai_move_key" => {
            app.settings.ai.updating = None;
            if let Some(e) = &r.error {
                app.settings.ai.update_error = Some(e.clone());
            }
            send(
                app,
                "ai.providers.list",
                Value::Null,
                "settings.ai_providers_after",
            );
        }
        "settings.ai_setup_enable" => {
            app.settings.ai.updating = None;
            match &r.error {
                Some(e) => app.settings.ai.update_error = Some(e.clone()),
                None => {
                    send(
                        app,
                        "ai.providers.list",
                        Value::Null,
                        "settings.ai_providers_after",
                    );
                    if !provider.is_empty() {
                        check_many(app, vec![provider]);
                    }
                }
            }
        }
        "settings.chat_set" => {
            send(
                app,
                "ui.chat.history.configure",
                json!({}),
                "settings.chat_config",
            );
            send(
                app,
                "ui.chat.sessions.list",
                json!({"limit": 200}),
                "settings.chat_sessions",
            );
        }
        "settings.chat_clear" => {
            send(
                app,
                "ui.chat.sessions.list",
                json!({"limit": 200}),
                "settings.chat_sessions",
            );
        }
        "settings.agent_configure" => {
            app.settings.agents.error = r.error.is_some();
            send(
                app,
                "ui.agent_access.status",
                Value::Null,
                "settings.agent_access",
            );
        }
        "settings.agent_unlock" | "settings.agent_lock" | "settings.agent_pairing" => {
            app.settings.agents.session_error = r.error.clone().map(|e| {
                r.error_detail["hint"]
                    .as_str()
                    .map(|h| format!("{e} {h}"))
                    .unwrap_or(e)
            });
            if tag == "settings.agent_unlock" {
                app.settings.agents.existing_lease =
                    r.data["session"]["existing_lease"].as_bool() == Some(true);
            }
            send(
                app,
                "ui.agent_access.status",
                Value::Null,
                "settings.agent_access",
            );
        }
        "settings.agent_access" if r.data["mcp_enabled"].as_bool() == Some(true) => {
            app.settings.agents.draft_grant = None;
            app.settings.agents.draft_access = None;
        }
        _ => {}
    }
}

/// Starts one provider's check (egress: CLI runtime probe or HTTP model discovery).
fn run_check(app: &mut App, name: &str) {
    let Some(p) = find(app, name) else { return };
    let url = p["base_url"].as_str().unwrap_or("").to_owned();
    let c = app.settings.ai.checks.entry(name.to_owned()).or_default();
    c.pending = true;
    c.error = None;
    let args = json!({"provider": name, "base_url": url, "refresh": true});
    if is_native_locator(&url) {
        send(
            app,
            "ai.provider_runtime.status",
            args,
            "settings.ai_check_runtime",
        );
    } else {
        send(app, "ai.list_models", args, "settings.ai_check_models");
    }
}

/// `checkMany`: enabled providers only; offline refuses non-local ones; off-device providers
/// never acknowledged ask first.
pub fn check_many(app: &mut App, names: Vec<String>) {
    if !app.prefs.ai_features {
        return;
    }
    let off = offline(app);
    let rows: Vec<Value> = names
        .iter()
        .filter_map(|n| find(app, n))
        .filter(enabled)
        .collect();
    for n in &names {
        app.settings.ai.refusals.remove(n);
    }
    let mut reachable = vec![];
    for p in rows {
        let n = p["name"].as_str().unwrap_or("").to_owned();
        if off && p["kind"].as_str() != Some("local") {
            app.settings.ai.refusals.insert(n, "offline");
        } else {
            reachable.push(p);
        }
    }
    let unack: Vec<&Value> = reachable
        .iter()
        .filter(|p| p["kind"].as_str() != Some("local") && p["acknowledged_at"].is_null())
        .collect();
    if !unack.is_empty() {
        let names_str = unack
            .iter()
            .map(|p| name_of(p))
            .collect::<Vec<_>>()
            .join(", ");
        let msg = tc(
            "settings:ai.checkConfirm",
            unack.len(),
            &[("names", &names_str)],
        );
        let all: Vec<String> = reachable
            .iter()
            .map(|p| p["name"].as_str().unwrap_or("").to_owned())
            .collect();
        // Acknowledged-or-local ones run at once; the rest after the confirmation.
        let unack_names: Vec<String> = unack
            .iter()
            .map(|p| p["name"].as_str().unwrap_or("").to_owned())
            .collect();
        for n in all.iter().filter(|n| !unack_names.contains(n)) {
            run_check(app, n);
        }
        app.settings.ai.pending_confirm = unack_names.clone();
        super::dialogs::confirm(
            app,
            msg,
            super::dialogs::Confirmed::CheckProviders(unack_names),
        );
        return;
    }
    for p in reachable {
        run_check(app, p["name"].as_str().unwrap_or(""));
    }
}

/// The confirmation was accepted: acknowledge each, then check them.
pub fn acknowledge_and_check(app: &mut App, names: Vec<String>) {
    app.settings.ai.pending_confirm.clear();
    app.settings.ai.after_ack = names.clone();
    app.settings.ai.ack_error = None;
    for n in names {
        send(
            app,
            "ai.providers.acknowledge",
            json!({"name": n}),
            "settings.ai_ack",
        );
    }
}

/// `setUp`: a switched-off built-in is switched on first, then checked.
fn set_up(app: &mut App, name: &str) {
    let Some(p) = find(app, name) else { return };
    if !enabled(&p) {
        app.settings.ai.updating = Some(name.to_owned());
        app.settings.ai.update_error = None;
        send(
            app,
            "ai.providers.update",
            json!({"name": name, "enabled": true}),
            "settings.ai_setup_enable",
        );
        return;
    }
    check_many(app, vec![name.to_owned()]);
}

fn update(app: &mut App, name: &str, args: Value) {
    app.settings.ai.updating = Some(name.to_owned());
    app.settings.ai.update_error = None;
    let mut a = args;
    a["name"] = json!(name);
    send(app, "ai.providers.update", a, "settings.ai_update");
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Ready,
    Attention,
    Error,
    Muted,
}

fn tone_color(tk: &Tk, t_: Tone) -> Color {
    match t_ {
        Tone::Ready => emerald(tk),
        Tone::Attention => amber(tk),
        Tone::Error => tk.destructive,
        Tone::Muted => tk.muted_fg,
    }
}

/// The card's one-line status and the detail line under it.
fn status(app: &App, p: &Value, c: &Check) -> (Tone, String, Option<String>) {
    let name = p["name"].as_str().unwrap_or("");
    let cli = is_native_locator(p["base_url"].as_str().unwrap_or(""));
    if c.pending {
        return (Tone::Muted, t("settings:ai.status.checking"), None);
    }
    if app.settings.ai.refusals.get(name) == Some(&"offline") {
        return (Tone::Attention, t("settings:ai.status.offline"), None);
    }
    let err = |tone: Tone| {
        if c.error.is_some() && !c.stale {
            Tone::Error
        } else {
            tone
        }
    };
    if cli && c.models_listed && c.error.is_none() {
        return (
            if c.models.is_empty() {
                Tone::Muted
            } else {
                Tone::Ready
            },
            tc("settings:ai.status.modelsListed", c.models.len(), &[]),
            None,
        );
    }
    if cli {
        let Some(rt) = &c.runtime else {
            return (
                err(Tone::Muted),
                if c.error.is_some() {
                    t("settings:ai.status.error")
                } else {
                    t("settings:ai.status.notChecked")
                },
                None,
            );
        };
        let version = model::version_label(rt["version"].as_str());
        let message = rt["message"]
            .as_str()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_owned);
        let (tone, text_, detail) = match rt["state"].as_str() {
            Some("ready") => (
                Tone::Ready,
                version
                    .map(|v| tr("settings:ai.status.readyWithVersion", &[("version", &v)]))
                    .unwrap_or_else(|| t("settings:ai.status.ready")),
                None,
            ),
            Some("missing_executable") => {
                (Tone::Error, t("settings:ai.status.notInstalled"), message)
            }
            Some("authentication_required") => (
                Tone::Attention,
                version
                    .map(|v| {
                        tr(
                            "settings:ai.status.signInNeededWithVersion",
                            &[("version", &v)],
                        )
                    })
                    .unwrap_or_else(|| t("settings:ai.status.signInNeeded")),
                message,
            ),
            _ => (Tone::Error, t("settings:ai.status.error"), message),
        };
        return (err(tone), text_, detail);
    }
    if c.reachable {
        return (
            err(Tone::Ready),
            tc("settings:ai.status.reachable", c.models.len(), &[]),
            None,
        );
    }
    if c.error.is_some() {
        return (err(Tone::Muted), t("settings:ai.status.unreachable"), None);
    }
    (Tone::Muted, t("settings:ai.status.notChecked"), None)
}

/// Brand glyph stand-in (`ProviderGlyph` draws vendor marks): a lucide glyph per route.
fn glyph(tk: &Tk, p: &Value, dim: bool) -> El {
    let url = p["base_url"].as_str().unwrap_or("");
    let g = if is_native_locator(url) {
        &I::SQUARE_TERMINAL
    } else if p["kind"].as_str() == Some("local") {
        &I::LAPTOP
    } else {
        &I::KEY_ROUND
    };
    ic(g, tk.r(1.25), if dim { tk.fg.alpha(0.5) } else { tk.fg }).a11y_hidden()
}

fn card(app: &App, tk: &Tk, layer: &mut Vec<El>, p: &Value) -> El {
    let pname = p["name"].as_str().unwrap_or("").to_owned();
    let name = name_of(p);
    let url = p["base_url"].as_str().unwrap_or("").to_owned();
    let cli = is_native_locator(&url);
    let on = enabled(p);
    let is_default = p["is_default"].as_bool() == Some(true);
    let check = app
        .settings
        .ai
        .checks
        .get(&pname)
        .cloned()
        .unwrap_or_default();
    let (tone, status_text, detail) = status(app, p, &check);
    let key = format!("set-ai-card-{pname}");
    let open = super::is_open(app, &key, false);
    let updating = app.settings.ai.updating.as_deref() == Some(pname.as_str());
    let where_ = t(if cli {
        "settings:ai.where.cli".into()
    } else {
        format!(
            "settings:ai.where.{}",
            p["kind"].as_str().unwrap_or("local")
        )
    }
    .as_str());
    let model_label = p["default_model"]
        .as_str()
        .filter(|m| !m.is_empty())
        .map(|m| {
            if cli && m == "default" {
                t("assistant:modelPicker.cliDefault")
            } else {
                m.to_owned()
            }
        });
    let sub = [
        is_default.then(|| t("settings:ai.default")),
        Some(where_),
        model_label.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let k = key.clone();
    let head_btn = row()
        .key(format!("{key}-toggle"))
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.75))
        .center_y()
        .px(tk.r(0.5))
        .py(tk.r(0.25))
        .ml(-tk.r(0.5))
        .radius(tk.r(0.375))
        .hover(tk.muted.alpha(0.4))
        .role("button")
        .expanded(open)
        .label(tr("settings:ai.showDetails", &[("name", &name)]))
        .focusable(true)
        .on_press(move |a: &mut App| super::toggle_open(a, &k, false))
        .child(
            ic(
                if open {
                    &I::CHEVRON_DOWN
                } else {
                    &I::CHEVRON_RIGHT
                },
                tk.r(1.0),
                tk.muted_fg,
            )
            .a11y_hidden(),
        )
        .child(glyph(tk, p, !on))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .opacity(if on { 1.0 } else { 0.7 })
                .child(
                    text(name.clone())
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .truncate(),
                )
                .child(text(sub).font(tk.tsm(400.0)).color(tk.muted_fg).truncate()),
        )
        .child(
            text(status_text)
                .font(tk.tsm(400.0))
                .color(tone_color(tk, tone))
                .truncate()
                .max_w(tk.r(18.0)),
        );
    let pn = pname.clone();
    let sw_enabled = !updating && !(is_default && on);
    let mut sw = switch(
        tk,
        format!("{key}-enabled"),
        on,
        &tr("settings:ai.enableProvider", &[("name", &name)]),
        sw_enabled,
        move |a, v| update(a, &pn, json!({"enabled": v})),
    );
    if is_default && on {
        sw = sw.help(t("settings:ai.defaultCannotDisable"));
    }
    let mut c = col()
        .w_full()
        .key(key.clone())
        .role("group")
        .label(name.clone())
        .child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.75))
                .center_y()
                .child(head_btn)
                .child(sw),
        );
    if !open {
        return c;
    }
    // Details.
    let mut d = col()
        .w_full()
        .gap(tk.r(0.75))
        .pl(tk.r(4.75))
        .pr(tk.r(1.0))
        .py(tk.r(0.75))
        .fill(tk.muted.alpha(0.2))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout();
    let ago = model::checked_ago(check.checked_at.as_deref(), crate::format::now_secs());
    if (detail.is_some() || ago.is_some()) && !check.pending {
        let line = [
            detail.clone(),
            ago.map(|a| tr("settings:ai.status.checkedAgo", &[("ago", &a)])),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        d = d.child(
            text(line.replace('`', ""))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    let refused = app.settings.ai.refusals.contains_key(&pname);
    if let Some(e) = check.error.as_ref().filter(|_| !check.pending && !refused) {
        let s = if check.stale {
            format!("{} {}", t("settings:ai.status.stale"), e)
        } else {
            e.clone()
        };
        d = d.child(
            text(s.replace('`', ""))
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap(),
        );
    }
    if p["agent_access"].as_str() == Some("restricted") {
        d = d.child(
            text(t("settings:ai.restrictedAccessNote"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    if !cli && p["omit_store_field"].as_bool() == Some(true) {
        d = d.child(
            text(t("settings:ai.omitStoreNote"))
                .font(tk.tsm(400.0))
                .color(amber_soft(tk))
                .wrap(),
        );
    }
    if cli
        && check
            .models
            .iter()
            .any(|m| m["supports_fast_mode"].as_bool() == Some(true))
    {
        d = d.child(
            text(t("settings:ai.fastModeNote"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    let dl = |label: String, value: El| {
        row()
            .w_full()
            .gap(tk.r(1.5))
            .center_y()
            .child(
                text(label)
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .w(tk.r(9.0)),
            )
            .child(value.grow(1.0).min_w(0.0))
    };
    let mut grid = col().w_full().gap(tk.r(0.5));
    if !cli {
        let fmt = match (p["api_format"].as_str(), p["detected_api_format"].as_str()) {
            (Some(f @ ("responses" | "chat_completions")), _) => Some(tr(
                "settings:ai.apiFormat.set",
                &[(
                    "format",
                    &t(&format!("settings:aiProvider.apiFormatName.{f}")),
                )],
            )),
            (_, Some(f @ ("responses" | "chat_completions"))) => Some(tr(
                "settings:ai.apiFormat.detected",
                &[(
                    "format",
                    &t(&format!("settings:aiProvider.apiFormatName.{f}")),
                )],
            )),
            (Some("auto"), _) => Some(t("settings:ai.apiFormat.auto")),
            _ => None,
        };
        let mut v = row()
            .gap(tk.r(0.25))
            .child(text(url.clone()).font(tk.tsm(400.0)).color(tk.fg).wrap());
        if let Some(f) = fmt {
            v = v.child(
                text(format!("· {f}"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .help(t("settings:ai.apiFormatTitle")),
            );
        }
        grid = grid.child(dl(t("settings:ai.address"), v));
    }
    grid = grid.child(dl(
        t("settings:ai.defaultModel"),
        text(model_label.unwrap_or_else(|| "-".into()))
            .font(tk.tsm(400.0))
            .color(tk.fg),
    ));
    let efforts = model::effort_options(p, &check.models);
    let pn = pname.clone();
    let effort = select(
        app,
        tk,
        layer,
        &format!("{key}-effort"),
        &t("settings:ai.defaultEffort"),
        p["default_reasoning_effort"].as_str().unwrap_or("auto"),
        std::iter::once(opt("auto", t("assistant:composer.effort.auto")))
            .chain(
                efforts
                    .iter()
                    .map(|e| opt(*e, t(&format!("assistant:composer.effort.{e}")))),
            )
            .collect(),
        !updating,
        None,
        move |a, v| {
            update(
                a,
                &pn,
                if v == "auto" {
                    json!({"clear": ["default_reasoning_effort"]})
                } else {
                    json!({"default_reasoning_effort": v})
                },
            )
        },
    );
    grid = grid.child(dl(t("settings:ai.defaultEffort"), row().child(effort)));
    let profile = app
        .settings
        .prefs
        .tool_profiles
        .get(&pname)
        .cloned()
        .unwrap_or_else(|| "full".into());
    let pn = pname.clone();
    let tools = select(
        app,
        tk,
        layer,
        &format!("{key}-tools"),
        &t("settings:ai.toolProfile"),
        &profile,
        ["full", "scoped", "core"]
            .iter()
            .map(|v| super::controls::Opt {
                value: (*v).into(),
                label: t(&format!("assistant:composer.toolProfile.{v}.label")),
                title: Some(t(&format!(
                    "assistant:composer.toolProfile.{v}.description"
                ))),
                disabled: false,
            })
            .collect(),
        true,
        None,
        move |a, v| {
            if v == "full" {
                a.settings.prefs.tool_profiles.remove(&pn);
            } else {
                a.settings
                    .prefs
                    .tool_profiles
                    .insert(pn.clone(), v.to_owned());
            }
        },
    );
    grid = grid.child(dl(t("settings:ai.toolProfile"), row().child(tools)));
    let key_text = if cli {
        t("settings:ai.signInCli")
    } else if p["has_api_key"].as_bool() == Some(true) {
        format!(
            "{} · {}",
            model::secret_store_label(p["secret_ref"]["store_id"].as_str().unwrap_or("")),
            model::secret_state_label(p["secret_ref"]["state"].as_str().unwrap_or(""))
        )
    } else {
        t("settings:ai.noKey")
    };
    grid = grid.child(dl(
        t(if cli {
            "settings:ai.signIn"
        } else {
            "settings:ai.keyStorage"
        }),
        text(key_text).font(tk.tsm(400.0)).color(tk.fg).wrap(),
    ));
    d = d.child(grid);
    // Actions.
    let blocked = if !app.prefs.ai_features {
        Some(t("settings:ai.status.aiOff"))
    } else if !on {
        Some(t("settings:ai.status.switchedOff"))
    } else {
        None
    };
    let can_check = !check.pending && blocked.is_none();
    let pn = pname.clone();
    let mut actions = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .pt(tk.r(0.25))
        .child(
            btn(
                tk,
                format!("{key}-check"),
                &t(if p["kind"].as_str() == Some("cli_proxy") {
                    "settings:ai.refreshModels"
                } else {
                    "settings:ai.check"
                }),
                Btn::Outline,
                can_check,
            )
            .label(tr("settings:ai.checkProvider", &[("name", &name)]))
            .help(blocked.unwrap_or_else(|| tr("settings:ai.checkHint", &[("name", &name)])))
            .press_if(can_check, move |a| check_many(a, vec![pn.clone()])),
        );
    if !is_default && on {
        let pn = pname.clone();
        let busy = app.settings.is_busy("settings.ai_set_default");
        actions = actions.child(
            btn(
                tk,
                format!("{key}-default"),
                &t("settings:ai.setAsDefault"),
                Btn::Outline,
                !busy,
            )
            .press_if(!busy, move |a| {
                send(
                    a,
                    "ai.providers.set_default",
                    json!({"name": pn}),
                    "settings.ai_set_default",
                );
            }),
        );
    }
    let pn = pname.clone();
    actions = actions.child(
        btn(
            tk,
            format!("{key}-edit"),
            &t("settings:ai.edit"),
            Btn::Outline,
            true,
        )
        .label(tr("settings:ai.editProvider", &[("name", &name)]))
        .on_press(move |a: &mut App| super::aiform::open_edit(a, &pn)),
    );
    let policy = app
        .read("ai.providers.list")
        .map(|d| d["secret_store_policy"]["default"].clone())
        .unwrap_or(Value::Null);
    let native = policy["native_store_id"].as_str().map(str::to_owned);
    let moving = app.settings.is_busy("settings.ai_move_key");
    let store = p["secret_ref"]["store_id"].as_str().unwrap_or("");
    if let Some(native) = native.filter(|_| {
        policy["native_available"].as_bool() == Some(true)
            && store == "sqlcipher_inline"
            && p["has_api_key"].as_bool() == Some(true)
    }) {
        // Moves the key into the OS keychain: outside the disposable book (wired, not pressed).
        let pn = pname.clone();
        let label = tr(
            "settings:ai.moveKeyToNative",
            &[("store", model::secret_store_label(&native))],
        );
        actions = actions.child(
            btn(
                tk,
                format!("{key}-move-native"),
                &label,
                Btn::Outline,
                !moving,
            )
            .press_if(!moving, move |a| {
                send(
                    a,
                    "ai.providers.move_api_key",
                    json!({"name": pn, "store_id": native}),
                    "settings.ai_move_key",
                );
            }),
        );
    }
    if !store.is_empty() && store != "sqlcipher_inline" {
        let pn = pname.clone();
        let ok = !moving && p["has_api_key"].as_bool() == Some(true);
        actions = actions.child(
            btn(
                tk,
                format!("{key}-move-db"),
                &t("settings:ai.moveKeyToDatabase"),
                Btn::Outline,
                ok,
            )
            .press_if(ok, move |a| {
                send(
                    a,
                    "ai.providers.move_api_key",
                    json!({"name": pn, "store_id": "sqlcipher_inline"}),
                    "settings.ai_move_key",
                );
            }),
        );
    }
    let pn = pname.clone();
    let del_ok = !is_default && !app.settings.is_busy("settings.ai_delete");
    actions = actions.child(el().grow(1.0)).child(
        btn(
            tk,
            format!("{key}-remove"),
            &t("settings:ai.remove"),
            Btn::Outline,
            del_ok,
        )
        .label(tr("settings:ai.deleteProvider", &[("name", &name)]))
        .press_if(del_ok, move |a| {
            let n = find(a, &pn).map(|p| name_of(&p)).unwrap_or_default();
            super::dialogs::confirm(
                a,
                tr("settings:ai.deleteConfirm", &[("name", &n)]),
                super::dialogs::Confirmed::DeleteProvider(pn.clone()),
            );
        }),
    );
    d = d.child(actions);
    c = c.child(d);
    c
}

/// `NotSetUpProviders`.
fn not_set_up(app: &App, tk: &Tk, rows: &[Value]) -> El {
    let key = "set-ai-notsetup";
    let open = super::is_open(app, key, false);
    let mut c = col().w_full().child(
        row()
            .key(key)
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(0.75))
            .gap(tk.r(0.75))
            .center_y()
            .hover(tk.muted.alpha(0.4))
            .role("button")
            .expanded(open)
            .focusable(true)
            .on_press(move |a: &mut App| super::toggle_open(a, key, false))
            .child(
                ic(
                    if open {
                        &I::CHEVRON_DOWN
                    } else {
                        &I::CHEVRON_RIGHT
                    },
                    tk.r(1.0),
                    tk.muted_fg,
                )
                .a11y_hidden(),
            )
            .child(
                text(tc("settings:ai.notSetUp.title", rows.len(), &[]))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
    );
    if !open {
        return c;
    }
    let blocked = (!app.prefs.ai_features).then(|| t("settings:ai.status.aiOff"));
    for p in rows {
        let pname = p["name"].as_str().unwrap_or("").to_owned();
        let name = name_of(p);
        let check = app
            .settings
            .ai
            .checks
            .get(&pname)
            .cloned()
            .unwrap_or_default();
        let pending = check.pending || app.settings.ai.updating.as_deref() == Some(pname.as_str());
        let url = p["base_url"].as_str().unwrap_or("");
        let where_ = t(if is_native_locator(url) {
            "settings:ai.where.cli".into()
        } else {
            format!(
                "settings:ai.where.{}",
                p["kind"].as_str().unwrap_or("local")
            )
        }
        .as_str());
        let mut info = col()
            .grow(1.0)
            .min_w(0.0)
            .child(
                text(name.clone())
                    .font(tk.tsm(500.0))
                    .color(tk.fg)
                    .truncate(),
            )
            .child(
                text(where_)
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .truncate(),
            );
        if let Some(e) = &check.error {
            info = info.child(
                text(e.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.destructive)
                    .wrap()
                    .role("alert"),
            );
        }
        let pn = pname.clone();
        let del_ok =
            !app.settings.is_busy("settings.ai_delete") && p["is_default"].as_bool() != Some(true);
        let trash = crate::txdetail::widgets::button_surface(
            tk,
            row()
                .key(format!("set-ai-ns-del-{pname}"))
                .size(tk.r(2.0), tk.r(2.0))
                .center(),
            crate::txdetail::widgets::Variant::Ghost,
        )
        .role("button")
        .label(tr("settings:ai.notSetUp.remove", &[("name", &name)]))
        .focusable(del_ok)
        .disabled(!del_ok)
        .group_opacity(0.0, 1.0)
        .child(ic(&I::TRASH_2, tk.r(1.0), tk.muted_fg))
        .press_if(del_ok, move |a| {
            let n = find(a, &pn).map(|p| name_of(&p)).unwrap_or_default();
            super::dialogs::confirm(
                a,
                tr("settings:ai.deleteConfirm", &[("name", &n)]),
                super::dialogs::Confirmed::DeleteProvider(pn.clone()),
            );
        });
        let pn = pname.clone();
        let ok = blocked.is_none() && !pending;
        let mut setup = btn(
            tk,
            format!("set-ai-ns-setup-{pname}"),
            &t(if pending {
                "settings:ai.status.checking"
            } else {
                "settings:ai.notSetUp.setUp"
            }),
            Btn::Outline,
            ok,
        )
        .label(tr("settings:ai.notSetUp.setUpAria", &[("name", &name)]))
        .press_if(ok, move |a| set_up(a, &pn));
        if let Some(b) = &blocked {
            setup = setup.help(b.clone());
        }
        c = c.child(
            row()
                .hover_group()
                .w_full()
                .pl(tk.r(2.75))
                .pr(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.75))
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(glyph(tk, p, false))
                .child(info)
                .child(trash)
                .child(setup),
        );
    }
    c
}

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ai_on = app.prefs.ai_features;
    let features = srow_a(
        app,
        tk,
        t("settings:ai.featuresLabel"),
        desc(tk, t("settings:ai.featuresDescription")),
        vec![switch(
            tk,
            "set-ai-features",
            ai_on,
            &t("settings:ai.featuresAria"),
            true,
            |a, v| {
                a.prefs.ai_features = v;
                if v {
                    read_cached(a);
                }
            },
        )],
        Some("assistantFeatures"),
    );
    let mode = app.settings.prefs.permission_mode;
    let perm = srow_a(
        app,
        tk,
        t("settings:ai.permissionMode.label"),
        desc(
            tk,
            t(&format!("assistant:composer.permission.{mode}.description")),
        ),
        vec![select(
            app,
            tk,
            layer,
            "set-ai-permission",
            &t("settings:ai.permissionMode.label"),
            mode,
            ["approval", "auto", "full_access"]
                .iter()
                .map(|m| opt(*m, t(&format!("assistant:composer.permission.{m}.label"))))
                .collect(),
            true,
            None,
            |a, v| {
                a.settings.prefs.permission_mode = match v {
                    "approval" => "approval",
                    "full_access" => "full_access",
                    _ => "auto",
                }
            },
        )],
        Some("permissionMode"),
    );
    let mut first = group().row(features).row(perm);
    for r in chat_history(app, tk, layer) {
        first = first.row(r);
    }

    // Models.
    let list = providers(app);
    let enabled_names: Vec<String> = list
        .iter()
        .filter(|p| enabled(p))
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect();
    let any_pending = app.settings.ai.checks.values().any(|c| c.pending)
        || app.settings.is_busy("settings.ai_ack");
    let can_all = ai_on && !any_pending && !enabled_names.is_empty();
    let check_all = btn(
        tk,
        "set-ai-check-all",
        &t("settings:ai.checkAll"),
        Btn::Outline,
        can_all,
    )
    .help(if ai_on {
        t("settings:ai.checkAllHint")
    } else {
        t("settings:ai.status.aiOff")
    })
    .press_if(can_all, move |a| check_many(a, enabled_names.clone()));
    let add = btn(
        tk,
        "set-ai-add",
        &t("settings:ai.addProvider"),
        Btn::Outline,
        true,
    )
    .on_press(|a: &mut App| super::aiform::open_add(a));
    let mut models = group()
        .anchor("aiModels")
        .title(t("settings:ai.modelsHeading"))
        .footer(t("settings:ai.checkPrivacy"))
        .action(check_all)
        .action(add);
    if let Some(w) = app
        .read("ai.providers.list")
        .and_then(|d| d["secret_store_policy"]["default"]["warning"].as_str())
    {
        models = models.row(para(tk, w.to_owned(), amber_soft(tk)));
    }
    if let Some(e) = &app.settings.ai.update_error {
        models = models.row(para(tk, e.clone(), tk.destructive).role("alert"));
    }
    let loaded = app.read("ai.providers.list").is_some();
    if !loaded && app.settings.is_busy("settings.ai_providers") {
        models = models.row(para(tk, t("settings:ai.loadingProviders"), tk.muted_fg));
    } else if !loaded {
        models = models.row(para(tk, t("settings:ai.loadError"), tk.destructive));
    } else if list.is_empty() {
        models = models.row(para(tk, t("settings:ai.noProviders"), tk.muted_fg));
    } else {
        let (in_use, not_set): (Vec<Value>, Vec<Value>) = list
            .into_iter()
            .partition(|p| p["set_up"].as_bool() != Some(false));
        for p in &in_use {
            models = models.row(card(app, tk, layer, p));
        }
        if !not_set.is_empty() {
            models = models.row(not_set_up(app, tk, &not_set));
        }
    }
    col()
        .w_full()
        .gap(tk.r(1.5))
        .child(first.build(app, tk))
        .child(models.build(app, tk))
        .child(agents(app, tk, layer))
}

fn chat_history(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let cfg = app
        .read("ui.chat.history.configure")
        .cloned()
        .unwrap_or(Value::Null);
    let mode = cfg["history"].as_str().unwrap_or("auto").to_owned();
    let enabled = cfg["history_enabled"].as_bool() == Some(true);
    let enc = cfg["database_encrypted"].as_bool() == Some(true);
    let line = if enabled {
        if enc {
            "settings:chatHistory.effectiveEncrypted"
        } else {
            "settings:chatHistory.effectiveOnUnencrypted"
        }
    } else if enc {
        "settings:chatHistory.effectiveNotStoredEncrypted"
    } else if mode == "auto" {
        "settings:chatHistory.effectiveNotStoredAuto"
    } else {
        "settings:chatHistory.effectiveNotStored"
    };
    let count = app
        .read("ui.chat.sessions.list")
        .and_then(|s| s["sessions"].as_array())
        .map(Vec::len)
        .unwrap_or(0);
    let sel = select(
        app,
        tk,
        layer,
        "set-chat-history",
        &t("settings:chatHistory.storeLabel"),
        &mode,
        ["auto", "on", "off"]
            .iter()
            .map(|v| super::controls::Opt {
                value: (*v).into(),
                label: t(&format!("settings:chatHistory.modeLabel.{v}")),
                title: Some(t(&format!("settings:chatHistory.mode.{v}"))),
                disabled: false,
            })
            .collect(),
        true,
        None,
        |a, v| {
            send(
                a,
                "ui.chat.history.configure",
                json!({"history": v}),
                "settings.chat_set",
            );
        },
    );
    let controls = if app.settings.ai.confirming_clear {
        vec![
            btn(
                tk,
                "set-chat-delete-all",
                &t("settings:chatHistory.deleteAll"),
                Btn::Destructive,
                true,
            )
            .on_press(|a: &mut App| {
                send(
                    a,
                    "ui.chat.sessions.clear",
                    json!({}),
                    "settings.chat_clear",
                );
                crate::assistant::forget_all_sessions(a);
                a.settings.ai.confirming_clear = false;
            }),
            btn(
                tk,
                "set-chat-cancel",
                &t("common:actions.cancel"),
                Btn::Ghost,
                true,
            )
            .on_press(|a: &mut App| a.settings.ai.confirming_clear = false),
        ]
    } else {
        vec![
            btn(
                tk,
                "set-chat-clear",
                &t("settings:chatHistory.clearStored"),
                Btn::Outline,
                count > 0,
            )
            .press_if(count > 0, |a| a.settings.ai.confirming_clear = true),
        ]
    };
    // With no saved chats the count row has nothing to act on: its storage note joins the
    // setting's own line instead (signal, not a "0 saved chats" row).
    if count == 0 && !app.settings.ai.confirming_clear {
        let note = if enabled {
            format!("{} {}", t(line), t("settings:chatHistory.description"))
        } else {
            t(line)
        };
        return vec![srow_a(
            app,
            tk,
            t("settings:chatHistory.storeLabel"),
            desc(tk, note),
            vec![sel],
            Some("chatHistory"),
        )];
    }
    vec![
        srow_a(
            app,
            tk,
            t("settings:chatHistory.storeLabel"),
            desc(tk, t(line)),
            vec![sel],
            Some("chatHistory"),
        ),
        srow(
            app,
            tk,
            tc("settings:chatHistory.storedCount", count, &[]),
            desc(tk, t("settings:chatHistory.description")),
            controls,
        ),
    ]
}

/// `ExternalAgentsSettings`.
fn agents(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ai_on = app.prefs.ai_features;
    let access = app.read("ui.agent_access.status");
    let recorded_on = access.is_some_and(|a| a["mcp_enabled"].as_bool() == Some(true));
    let on = ai_on && recorded_on;
    let recorded = access.is_some_and(|a| a["access_recorded"].as_bool() == Some(true));
    let ag = &app.settings.agents;
    let grant = if recorded_on {
        access
            .and_then(|a| a["grant"].as_str())
            .unwrap_or("read")
            .to_owned()
    } else {
        ag.draft_grant.clone().unwrap_or_else(|| {
            if recorded {
                access
                    .and_then(|a| a["grant"].as_str())
                    .unwrap_or("read")
                    .to_owned()
            } else {
                "accounting_decisions".into()
            }
        })
    };
    let agent_access = if recorded_on {
        access
            .and_then(|a| a["agent_access"].as_str())
            .unwrap_or("restricted")
            .to_owned()
    } else {
        ag.draft_access.clone().unwrap_or_else(|| {
            if recorded {
                access
                    .and_then(|a| a["agent_access"].as_str())
                    .unwrap_or("restricted")
                    .to_owned()
            } else {
                "full".into()
            }
        })
    };
    let configuring = app.settings.is_busy("settings.agent_configure");
    let choice_ok = ai_on && access.is_some() && !configuring;
    let (g2, a2) = (grant.clone(), agent_access.clone());
    let sw = switch(
        tk,
        "set-agents",
        on,
        &t("settings:ai.agentsAria"),
        ai_on && access.is_some() && !configuring,
        move |a, v| {
            let ai = a.prefs.ai_features;
            let args = if v {
                json!({"mcp_enabled": true, "ai_features_enabled": ai, "grant": g2, "agent_access": a2})
            } else {
                json!({"mcp_enabled": false, "ai_features_enabled": ai})
            };
            send(
                a,
                "ui.agent_access.configure",
                args,
                "settings.agent_configure",
            );
        },
    );
    let mut g = group()
        .anchor("externalAgents")
        .title(t("settings:ai.agentsLabel"))
        .row(srow(
            app,
            tk,
            t("settings:ai.agentsAllowLabel"),
            desc(tk, t("settings:ai.agentsDescription")),
            vec![sw],
        ));
    if ai_on && access.is_some() {
        g = g.footer(t(if grant == "read" {
            "settings:ai.agentsAdminNever"
        } else {
            "settings:ai.agentsHostApproval"
        }));
    }
    if ag.error {
        g = g.row(para(tk, t("settings:ai.agentsError"), tk.destructive));
    }
    if ai_on && access.is_some() {
        let grant_key = |v: &str| match v {
            "operator" => "Operator",
            "accounting_decisions" => "Decisions",
            _ => "Read",
        };
        let sel = select(
            app,
            tk,
            layer,
            "set-agents-grant",
            &t("settings:ai.agentsGrantLabel"),
            &grant,
            ["read", "operator", "accounting_decisions"]
                .iter()
                .map(|v| opt(*v, t(&format!("settings:ai.agentsGrant{}", grant_key(v)))))
                .collect(),
            choice_ok,
            None,
            move |a, v| {
                if recorded_on {
                    send(
                        a,
                        "ui.agent_access.configure",
                        json!({"grant": v}),
                        "settings.agent_configure",
                    );
                } else {
                    a.settings.agents.draft_grant = Some(v.to_owned());
                }
            },
        );
        g = g.row(srow(
            app,
            tk,
            t("settings:ai.agentsGrantLabel"),
            desc(
                tk,
                t(&format!("settings:ai.agentsGrant{}Hint", grant_key(&grant))),
            ),
            vec![sel],
        ));
        g = g.row(srow(
            app,
            tk,
            t("settings:ai.agentsRestrictLabel"),
            desc(tk, t("settings:ai.agentsRestrictHint")),
            vec![switch(
                tk,
                "set-agents-restrict",
                agent_access == "restricted",
                &t("settings:ai.agentsRestrictLabel"),
                choice_ok,
                move |a, v| {
                    let next = if v { "restricted" } else { "full" };
                    if recorded_on {
                        send(
                            a,
                            "ui.agent_access.configure",
                            json!({"agent_access": next}),
                            "settings.agent_configure",
                        );
                    } else {
                        a.settings.agents.draft_access = Some(next.to_owned());
                    }
                },
            )],
        ));
    }
    let status = app.read("status");
    let book = status.and_then(|s| {
        Some((
            s["data_root"].as_str()?,
            s["current_workspace"].as_str()?,
            s["current_profile"].as_str()?,
        ))
    });
    if let (true, Some((root, ws, prof))) = (on, book) {
        let launcher: Vec<String> = match super::desktop::inspect(app)
            .ok()
            .map(|s| s.target_path)
            .filter(|p| !p.is_empty())
        {
            Some(target) => {
                let file = std::path::Path::new(&target)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if file == "kassiber" || file == "kassiber.exe" {
                    vec![target]
                } else {
                    vec![target, "--cli".into()]
                }
            }
            None => vec!["kassiber".into()],
        };
        let args: Vec<String> = [
            "--data-root",
            root,
            "mcp",
            "serve",
            "--workspace",
            ws,
            "--profile",
            prof,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // The demo's agents read the demo's own agent settings (`crate::demo`).
        let (agent_cmd, agent_json) = match crate::demo::agent_env(app) {
            Some(env) => (
                crate::demo::claude_command(&launcher, &args, env),
                crate::demo::json_config(&launcher, &args, env),
            ),
            None => (
                model::claude_code_command(&launcher, &args),
                model::mcp_json_config(&launcher, &args),
            ),
        };
        g = g.row(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.5))
                .center_y()
                .child(col().grow(1.0).min_w(0.0).child(command_line(
                    app,
                    tk,
                    "set-agents-cmd",
                    &agent_cmd,
                )))
                .child(
                    row()
                        .gap(tk.r(0.125))
                        .center_y()
                        .child(text("JSON").font(tk.txs(400.0)).color(tk.muted_fg))
                        .child(copy_button(
                            app,
                            tk,
                            "set-agents-json",
                            agent_json,
                            t("settings:ai.agentsCopyJson"),
                        )),
                ),
        );
        if let Some(note) = crate::demo::agents_note(app, tk) {
            g = g.row(note);
        }
    }
    let session = access.map(|a| a["session"].clone()).unwrap_or(Value::Null);
    let live = app
        .read("ui.agent_access.status")
        .filter(|_| agent_session_active(app))
        .map(|a| a["session"].clone())
        .unwrap_or(session.clone());
    let active = session["active"].as_bool() == Some(true);
    let busy = [
        "settings.agent_unlock",
        "settings.agent_lock",
        "settings.agent_pairing",
    ]
    .iter()
    .any(|t_| app.settings.is_busy(t_));
    if (on && session["needed"].as_bool() == Some(true)) || active {
        let mins =
            (session["idle_timeout_seconds"].as_f64().unwrap_or(900.0) / 60.0).round() as u64;
        let d = if active {
            tr(
                "settings:ai.agentsSessionActive",
                &[("minutes", &mins.to_string())],
            )
        } else if ag.existing_lease {
            t("settings:ai.agentsSessionExisting")
        } else {
            t("settings:ai.agentsSessionLocked")
        };
        let b = btn(
            tk,
            "set-agents-session",
            &t(if active {
                "settings:ai.agentsSessionLock"
            } else {
                "settings:ai.agentsSessionUnlock"
            }),
            Btn::Outline,
            !busy,
        )
        .press_if(!busy, move |a| {
            if active {
                send(
                    a,
                    "ui.agent_access.lock",
                    Value::Null,
                    "settings.agent_lock",
                );
            } else {
                send(
                    a,
                    "ui.agent_access.unlock",
                    Value::Null,
                    "settings.agent_unlock",
                );
            }
        });
        g = g.row(srow(
            app,
            tk,
            t("settings:ai.agentsSessionLabel"),
            desc(tk, d),
            vec![b],
        ));
    }
    let peers: Vec<Value> = live["agents"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a["state"].as_str() != Some("denied"))
        .collect();
    if active && peers.is_empty() {
        g = g.row(para(tk, t("settings:ai.agentsSessionWaiting"), tk.muted_fg));
    }
    if active {
        for peer in peers {
            let id = peer["id"].as_str().unwrap_or("").to_owned();
            let pid = peer["pid"].as_u64().unwrap_or(0);
            let nm = peer["label"]
                .as_str()
                .map(|l| format!("{l} ({pid})"))
                .unwrap_or_else(|| pid.to_string());
            let pending = peer["state"].as_str() == Some("pending");
            let label = if pending {
                tr("settings:ai.agentsPairingRequest", &[("name", &nm)])
            } else {
                tc(
                    "settings:ai.agentsPairingAllowed",
                    peer["calls"].as_u64().unwrap_or(0) as usize,
                    &[("name", &nm)],
                )
            };
            let mut controls = vec![];
            if pending {
                let i = id.clone();
                controls.push(
                    btn(
                        tk,
                        format!("set-agents-allow-{id}"),
                        &t("settings:ai.agentsPairingAllow"),
                        Btn::Primary,
                        !busy,
                    )
                    .press_if(!busy, move |a| {
                        send(
                            a,
                            "ui.agent_access.pairing",
                            json!({"session_id": i, "allow": true}),
                            "settings.agent_pairing",
                        );
                    }),
                );
            }
            let i = id.clone();
            controls.push(
                btn(
                    tk,
                    format!("set-agents-deny-{id}"),
                    &t(if pending {
                        "settings:ai.agentsPairingDeny"
                    } else {
                        "settings:ai.agentsPairingRemove"
                    }),
                    Btn::Outline,
                    !busy,
                )
                .press_if(!busy, move |a| {
                    send(
                        a,
                        "ui.agent_access.pairing",
                        json!({"session_id": i, "allow": false}),
                        "settings.agent_pairing",
                    );
                }),
            );
            g = g.row(srow(app, tk, label, None, controls));
        }
    }
    if let Some(e) = ag.session_error.as_ref().filter(|_| on || active) {
        g = g.row(para(
            tk,
            format!("{} {e}", t("settings:ai.agentsSessionError")),
            tk.destructive,
        ));
    }
    let built = g.build(app, tk);
    if ai_on { built } else { built.opacity(0.6) }
}
