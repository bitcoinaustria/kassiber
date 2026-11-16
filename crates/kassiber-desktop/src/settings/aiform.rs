//! Add / edit AI provider wizard (AiProviderForm.tsx, aiProviderFormModel.ts,
//! aiProviderTest.ts): how the provider is reached and which preset, name / address / key,
//! the off-device acknowledgement, then the default model with Test connection.
//!
//! Nothing contacts the provider until Test connection (`ai.test_connection`), and saving a CLI
//! proxy reads its model list once (`ai.list_models refresh`): both only from those presses.
//! The API key field is masked and its value is sent only to the daemon, never logged.

use rune::{El, TextEdit, col, el, row, text};
use serde_json::{Value, json};

use super::controls::{
    Btn, PressIf, amber_soft, btn_lg, checkbox, copy_button, emerald, input, labelled, note,
};
use super::model::{auto_remote_reason, http_origin, is_loopback_http, is_native_locator};
use super::send;
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub display_name: &'static str,
    pub base_url: &'static str,
    pub kind: &'static str,
    pub default_model: &'static str,
    pub hint: &'static str,
    pub command: Option<&'static str>,
    pub key_required: bool,
    pub custom: bool,
    pub url_placeholder: Option<&'static str>,
}

#[allow(clippy::too_many_arguments)]
const fn pr(
    id: &'static str,
    label: &'static str,
    display_name: &'static str,
    base_url: &'static str,
    kind: &'static str,
    default_model: &'static str,
    hint: &'static str,
    command: Option<&'static str>,
) -> Preset {
    Preset {
        id,
        label,
        display_name,
        base_url,
        kind,
        default_model,
        hint,
        command,
        key_required: false,
        custom: false,
        url_placeholder: None,
    }
}

pub static PRESETS: [Preset; 14] = [
    pr(
        "ollama",
        "Ollama",
        "Ollama",
        "http://localhost:11434/v1",
        "local",
        "qwen3.6:35b",
        "ollama",
        Some("ollama serve"),
    ),
    pr(
        "lmstudio",
        "LM Studio",
        "LM Studio",
        "http://localhost:1234/v1",
        "local",
        "",
        "lmstudio",
        Some("lms server start"),
    ),
    pr(
        "vllm",
        "vLLM",
        "vLLM",
        "http://127.0.0.1:8000/v1",
        "local",
        "",
        "vllm",
        Some("vllm serve <model> --enable-auto-tool-choice --tool-call-parser hermes"),
    ),
    pr(
        "llamacpp",
        "llama.cpp",
        "llama.cpp",
        "http://127.0.0.1:8080/v1",
        "local",
        "",
        "llamacpp",
        Some("llama-server -m <model.gguf> --jinja"),
    ),
    pr(
        "omlx",
        "oMLX",
        "oMLX",
        "http://127.0.0.1:8000/v1",
        "local",
        "",
        "omlx",
        Some("omlx start"),
    ),
    Preset {
        key_required: true,
        ..pr(
            "openai",
            "OpenAI",
            "OpenAI",
            "https://api.openai.com/v1",
            "remote",
            "",
            "openai",
            None,
        )
    },
    Preset {
        key_required: true,
        ..pr(
            "openrouter",
            "OpenRouter",
            "OpenRouter",
            "https://openrouter.ai/api/v1",
            "remote",
            "",
            "openrouter",
            None,
        )
    },
    Preset {
        custom: true,
        ..pr(
            "openai-compatible",
            "OpenAI-compatible",
            "",
            "",
            "local",
            "",
            "custom",
            None,
        )
    },
    Preset {
        custom: true,
        ..pr(
            "openai-compatible-api",
            "OpenAI-compatible API",
            "",
            "",
            "remote",
            "",
            "customApi",
            None,
        )
    },
    pr(
        "claude-cli",
        "Claude CLI",
        "Claude CLI",
        "claude-cli://default",
        "remote",
        "default",
        "cli",
        None,
    ),
    pr(
        "codex-cli",
        "Codex CLI",
        "Codex CLI",
        "codex-cli://default",
        "remote",
        "default",
        "cli",
        None,
    ),
    pr(
        "opencode-cli",
        "OpenCode CLI",
        "OpenCode CLI",
        "opencode-cli://default",
        "remote",
        "",
        "cli",
        None,
    ),
    pr(
        "copilot-cli",
        "GitHub Copilot CLI",
        "GitHub Copilot CLI",
        "copilot-cli://default",
        "remote",
        "default",
        "cli",
        None,
    ),
    Preset {
        url_placeholder: Some("http://127.0.0.1:8317/v1"),
        ..pr(
            "cli-proxy",
            "CLI proxy",
            "CLI proxy",
            "",
            "cli_proxy",
            "",
            "cliProxy",
            None,
        )
    },
];

const ROUTES: [(&str, &[&str]); 3] = [
    (
        "local",
        &[
            "ollama",
            "lmstudio",
            "vllm",
            "llamacpp",
            "omlx",
            "openai-compatible",
        ],
    ),
    (
        "app",
        &[
            "claude-cli",
            "codex-cli",
            "opencode-cli",
            "copilot-cli",
            "cli-proxy",
        ],
    ),
    ("api", &["openai", "openrouter", "openai-compatible-api"]),
];

fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

fn route_for(id: &str) -> Option<&'static str> {
    ROUTES
        .iter()
        .find(|(_, ids)| ids.contains(&id))
        .map(|(r, _)| *r)
}

/// A test run and the values it tested (`target`) and listed (`listing`).
#[derive(Clone, Default)]
pub struct TestRun {
    /// "idle" | "running" | "ok" | "no_model" | "fail".
    pub state: &'static str,
    pub target: String,
    pub listing: String,
    pub data: Value,
    pub message: String,
}

#[derive(Default)]
pub struct AiForm {
    pub open: bool,
    pub initial: Option<Value>,
    pub name: TextEdit,
    pub base_url: TextEdit,
    pub api_key: TextEdit,
    pub key_origin: Option<String>,
    pub key_cleared: bool,
    pub model: TextEdit,
    pub kind: String,
    pub notes: TextEdit,
    pub remote_ack: bool,
    pub auto_remote: Option<&'static str>,
    pub preset: Option<&'static str>,
    pub route: Option<&'static str>,
    pub step: &'static str,
    pub api_format: &'static str,
    pub omit_store: bool,
    pub restrict: bool,
    pub settings_url: String,
    pub endpoint_reset: Option<&'static str>,
    pub advanced_open: bool,
    pub error: Option<String>,
    pub test: TestRun,
    /// The provider being saved, while its follow-up writes (key, proxy models) run.
    pub saving: Option<String>,
    pub save_key: Option<String>,
    pub save_proxy_models: bool,
}

fn te(s: &str) -> TextEdit {
    TextEdit::new(s)
}

pub fn open_add(app: &mut App) {
    app.settings.aiform = AiForm {
        open: true,
        kind: "local".into(),
        step: "provider",
        api_format: "auto",
        test: TestRun {
            state: "idle",
            ..Default::default()
        },
        ..AiForm::default()
    };
    super::subpage::opened(
        app,
        super::subpage::Kind::AiProvider,
        Some("set-ai-add".into()),
    );
}

pub fn open_edit(app: &mut App, name: &str) {
    let Some(p) = super::ai::providers(app)
        .into_iter()
        .find(|p| p["name"].as_str() == Some(name))
    else {
        return;
    };
    let s = |k: &str| p[k].as_str().unwrap_or("").to_owned();
    let fmt = match p["api_format"].as_str() {
        Some("responses") => "responses",
        Some("chat_completions") => "chat_completions",
        _ => "auto",
    };
    let omit = p["omit_store_field"].as_bool() == Some(true);
    app.settings.aiform = AiForm {
        open: true,
        name: te(p["display_name"].as_str().unwrap_or(&s("name"))),
        base_url: te(&s("base_url")),
        model: te(&s("default_model")),
        kind: p["kind"].as_str().unwrap_or("local").to_owned(),
        notes: te(&s("notes")),
        step: "connect",
        api_format: fmt,
        omit_store: omit,
        restrict: p["agent_access"].as_str() == Some("restricted"),
        settings_url: s("base_url").trim().to_owned(),
        advanced_open: fmt != "auto" || omit,
        test: TestRun {
            state: "idle",
            ..Default::default()
        },
        initial: Some(p),
        ..AiForm::default()
    };
    super::subpage::opened(
        app,
        super::subpage::Kind::AiProvider,
        Some(format!("set-ai-card-{name}-edit")),
    );
}

/// Scene helper: pick a route and preset as the user would.
pub fn scene_pick(app: &mut App, route: &'static str, id: &str) {
    app.settings.aiform.route = Some(route);
    if let Some(p) = preset(id) {
        apply_preset(app, p);
    }
}

fn apply_preset(app: &mut App, p: &'static Preset) {
    let f = &mut app.settings.aiform;
    f.route = route_for(p.id);
    f.error = None;
    f.api_key = te("");
    f.key_origin = None;
    f.key_cleared = false;
    f.api_format = "auto";
    f.omit_store = false;
    f.advanced_open = false;
    f.test = TestRun {
        state: "idle",
        ..Default::default()
    };
    f.endpoint_reset = None;
    f.settings_url = p.base_url.trim().to_owned();
    f.preset = Some(p.id);
    f.name = te(p.display_name);
    f.base_url = te(p.base_url);
    f.kind = p.kind.to_owned();
    f.auto_remote = None;
    f.model = te(p.default_model);
    f.notes = te(&if p.custom {
        String::new()
    } else if p.kind == "local" {
        tr("settings:aiProvider.presetNoteLocal", &[("label", p.label)])
    } else {
        tr(
            "settings:aiProvider.presetNoteRemote",
            &[("label", p.label)],
        )
    });
    f.remote_ack = false;
}

fn editing(f: &AiForm) -> bool {
    f.initial.is_some()
}

/// `syncEndpointSettings`.
fn sync_endpoint(f: &mut AiForm) -> bool {
    let current = f.base_url.text.trim().to_owned();
    if f.settings_url.is_empty() {
        if !current.is_empty() {
            f.settings_url = current;
        }
        return false;
    }
    if current == f.settings_url {
        return false;
    }
    f.settings_url = current.clone();
    f.test = TestRun {
        state: "idle",
        ..Default::default()
    };
    let lost = if f.omit_store {
        Some("exemption")
    } else if f.api_format != "auto" {
        Some("format")
    } else {
        None
    };
    f.api_format = "auto";
    f.omit_store = false;
    let shown = lost.filter(|_| !is_native_locator(&current));
    f.endpoint_reset = shown;
    shown.is_some()
}

/// `guardKeyOrigin`.
fn guard_key(f: &mut AiForm) -> bool {
    if f.api_key.text.trim().is_empty() {
        return false;
    }
    let origin = http_origin(&f.base_url.text);
    match &f.key_origin {
        None => {
            if origin.is_some() {
                f.key_origin = origin;
            }
            false
        }
        Some(k) if Some(k) == origin.as_ref() => false,
        Some(_) => {
            f.api_key = te("");
            f.key_origin = None;
            f.key_cleared = true;
            true
        }
    }
}

/// `applyUrlPosture`.
fn url_posture(f: &mut AiForm) -> String {
    match auto_remote_reason(&f.kind, &f.base_url.text) {
        Some(r) => {
            f.kind = "remote".into();
            f.auto_remote = Some(r);
            f.remote_ack = false;
            "remote".into()
        }
        None => f.kind.clone(),
    }
}

fn ack_needed(f: &AiForm, posture: &str) -> bool {
    posture != "local"
        && f.initial
            .as_ref()
            .is_none_or(|i| i["kind"].as_str() == Some("local") || i["acknowledged_at"].is_null())
}

fn is_cli_url(f: &AiForm) -> bool {
    is_native_locator(&f.base_url.text)
}

fn format_to_save(f: &AiForm) -> Option<&'static str> {
    if is_cli_url(f) || f.kind == "cli_proxy" {
        None
    } else {
        Some(f.api_format)
    }
}

fn omit_to_save(f: &AiForm) -> Option<bool> {
    if is_cli_url(f) || f.kind == "cli_proxy" {
        None
    } else {
        Some(f.omit_store)
    }
}

fn test_fingerprint(f: &AiForm) -> String {
    json!({"provider": f.initial.as_ref().map(|i| i["name"].clone()), "baseUrl": f.base_url.text.trim(), "apiKey": f.api_key.text.trim(), "model": f.model.text.trim(), "apiFormat": f.api_format, "omit": f.omit_store}).to_string()
}

fn listing_fingerprint(f: &AiForm) -> String {
    json!({"provider": f.initial.as_ref().map(|i| i["name"].clone()), "baseUrl": f.base_url.text.trim(), "apiKey": f.api_key.text.trim()}).to_string()
}

fn key_required(f: &AiForm) -> bool {
    f.preset
        .and_then(preset)
        .is_some_and(|p| p.key_required && f.base_url.text.trim() == p.base_url)
}

fn step_ids(f: &AiForm) -> Vec<&'static str> {
    let mut v = vec![];
    if !editing(f) {
        v.push("provider");
    }
    v.push("connect");
    if f.kind != "local" {
        v.push("privacy");
    }
    v.push("check");
    v
}

/// `handleTest` (egress: only from Test connection).
fn handle_test(app: &mut App) {
    let f = &mut app.settings.aiform;
    f.error = None;
    if guard_key(f) || sync_endpoint(f) {
        return;
    }
    let test_kind = url_posture(f);
    let needs = ack_needed(f, &test_kind);
    if needs && (!f.remote_ack || test_kind != f.kind) {
        f.error = Some(t("settings:aiProvider.errorAckRequiredTest"));
        f.step = "privacy";
        return;
    }
    let url = f.base_url.text.trim().to_owned();
    let (target, listing) = (test_fingerprint(f), listing_fingerprint(f));
    if url.is_empty() {
        f.test = TestRun {
            state: "fail",
            target,
            listing,
            message: t("settings:aiProvider.errorBaseUrlRequired"),
            ..Default::default()
        };
        return;
    }
    if !url.starts_with("http://") && !url.starts_with("https://") && !is_native_locator(&url) {
        f.test = TestRun {
            state: "fail",
            target,
            listing,
            message: t("settings:aiProvider.errorUrlScheme"),
            ..Default::default()
        };
        return;
    }
    let mut args = json!({"base_url": url, "kind": test_kind});
    if !f.api_key.text.trim().is_empty() {
        args["api_key"] = json!(f.api_key.text.trim());
    }
    if !f.model.text.trim().is_empty() {
        args["model"] = json!(f.model.text.trim());
    }
    if let Some(i) = &f.initial {
        args["provider"] = i["name"].clone();
    }
    if let Some(fmt) = format_to_save(f) {
        args["api_format"] = json!(fmt);
    }
    if let Some(o) = omit_to_save(f) {
        args["omit_store_field"] = json!(o);
    }
    if needs {
        args["acknowledged"] = json!(true);
    }
    f.test = TestRun {
        state: "running",
        target,
        listing,
        ..Default::default()
    };
    send(app, "ai.test_connection", args, "settings.aiform_test");
}

/// `handleSubmit`.
fn submit(app: &mut App) {
    let f = &mut app.settings.aiform;
    f.error = None;
    if guard_key(f) {
        f.step = "connect";
        f.error = Some(t("settings:aiProvider.errorKeyForAnotherAddress"));
        return;
    }
    if sync_endpoint(f) {
        f.step = "connect";
        f.error = Some(t("settings:aiProvider.errorSettingsResetForAddress"));
        return;
    }
    if url_posture(f) != f.kind
        || (f.auto_remote.is_some() && !f.remote_ack && ack_needed(f, &f.kind.clone()))
    {
        f.step = "privacy";
        f.error = Some(t("settings:aiProvider.errorAckRequired"));
        return;
    }
    if !editing(f) && key_required(f) && f.api_key.text.trim().is_empty() {
        f.step = "connect";
        let label = f.preset.and_then(preset).map(|p| p.label).unwrap_or("");
        f.error = Some(tr(
            "settings:aiProvider.errorApiKeyRequired",
            &[("label", label)],
        ));
        return;
    }
    if f.name.text.trim().is_empty() {
        f.step = "connect";
        f.error = Some(t("settings:aiProvider.errorNameRequired"));
        return;
    }
    let kind = f.kind.clone();
    let needs = ack_needed(f, &kind);
    if kind == "local" && is_cli_url(f) {
        f.error = Some(t("settings:aiProvider.errorCliLocalPosture"));
        return;
    }
    if needs && !f.remote_ack {
        f.step = "privacy";
        f.error = Some(t("settings:aiProvider.errorAckRequired"));
        return;
    }
    let tested_ok = f.test.state == "ok" && f.test.target == test_fingerprint(f);
    let detected = tested_ok
        .then(|| {
            f.test.data["detected_api_format"]
                .as_str()
                .filter(|v| *v == "responses" || *v == "chat_completions")
                .map(str::to_owned)
        })
        .flatten();
    let key = f.api_key.text.trim().to_owned();
    let url = f.base_url.text.trim().to_owned();
    let proxy = kind == "cli_proxy";
    let access = if f.restrict { "restricted" } else { "full" };
    let mut args = json!({
        "display_name": f.name.text.trim(),
        "base_url": url,
        "kind": kind,
        "agent_access": access,
    });
    let name = match &f.initial {
        Some(i) => {
            let n = i["name"].as_str().unwrap_or("").to_owned();
            args["name"] = json!(n);
            args["default_model"] = if f.model.text.trim().is_empty() {
                Value::Null
            } else {
                json!(f.model.text.trim())
            };
            args["notes"] = if f.notes.text.trim().is_empty() {
                Value::Null
            } else {
                json!(f.notes.text.trim())
            };
            if let Some(o) = omit_to_save(f) {
                args["omit_store_field"] = json!(o);
            }
            match format_to_save(f) {
                Some(fmt) => args["api_format"] = json!(fmt),
                None => {
                    args["api_format"] = json!("auto");
                    if !i["detected_api_format"].is_null() {
                        args["clear"] = json!(["detected_api_format"]);
                    }
                }
            }
            f.save_proxy_models =
                proxy && (!key.is_empty() || url != i["base_url"].as_str().unwrap_or("").trim());
            n
        }
        None => {
            let n = f.name.text.trim().to_lowercase();
            args["name"] = json!(f.name.text.trim());
            if !f.model.text.trim().is_empty() {
                args["default_model"] = json!(f.model.text.trim());
            }
            if !f.notes.text.trim().is_empty() {
                args["notes"] = json!(f.notes.text.trim());
            }
            if let Some(fmt) = format_to_save(f) {
                args["api_format"] = json!(fmt);
            }
            if let Some(o) = omit_to_save(f) {
                args["omit_store_field"] = json!(o);
            }
            f.save_proxy_models = proxy;
            n
        }
    };
    if needs {
        args["acknowledged"] = json!(true);
    }
    if let Some(d) = detected {
        args["detected_api_format"] = json!(d);
    }
    f.saving = Some(name);
    f.save_key = (!key.is_empty()).then_some(key);
    let kind_ = if editing(f) {
        "ai.providers.update"
    } else {
        "ai.providers.create"
    };
    send(app, kind_, args, "settings.aiform_save");
}

fn finish_save(app: &mut App) {
    let f = &mut app.settings.aiform;
    let name = f.saving.clone().unwrap_or_default();
    if let Some(k) = f.save_key.take() {
        send(
            app,
            "ai.providers.set_api_key",
            json!({"name": name, "api_key": k}),
            "settings.aiform_key",
        );
        return;
    }
    if std::mem::take(&mut app.settings.aiform.save_proxy_models) {
        let url = app.settings.aiform.base_url.text.trim().to_owned();
        send(
            app,
            "ai.list_models",
            json!({"provider": name, "base_url": url, "refresh": true}),
            "settings.aiform_proxy_models",
        );
        return;
    }
    app.settings.aiform.saving = None;
    app.settings.aiform.open = false;
    send(
        app,
        "ai.providers.list",
        Value::Null,
        "settings.ai_providers",
    );
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let f = &mut app.settings.aiform;
    match tag {
        "settings.aiform_test" => {
            if f.test.state != "running" {
                return;
            }
            match &r.error {
                Some(e) => {
                    f.test.state = "fail";
                    f.test.message = e.clone();
                }
                None => {
                    let skipped = r.data["probe"]["status"].as_str() == Some("skipped");
                    let reason = r.data["probe"]["reason"].as_str().unwrap_or("");
                    f.test.state = if skipped
                        && (reason == "no_model"
                            || ["model_not_listed", "not_a_chat_model", "unknown_family"]
                                .contains(&reason))
                    {
                        "no_model"
                    } else {
                        "ok"
                    };
                    f.test.data = r.data.clone();
                    if f.test.state == "ok" && r.data["store_accepted"].as_bool() == Some(false) {
                        f.advanced_open = true;
                    }
                }
            }
        }
        "settings.aiform_save" | "settings.aiform_key" => match &r.error {
            Some(e) => {
                f.error = Some(e.clone());
                f.saving = None;
            }
            None => finish_save(app),
        },
        "settings.aiform_proxy_models" => finish_save(app),
        _ => {}
    }
}

fn go_next(app: &mut App) {
    let f = &mut app.settings.aiform;
    f.error = None;
    match f.step {
        "provider" => {
            if f.preset.is_none() {
                f.error = Some(t("settings:aiProvider.errorChooseProvider"));
            } else {
                f.step = "connect";
            }
        }
        "connect" => {
            let url = f.base_url.text.trim().to_owned();
            if f.name.text.trim().is_empty() {
                f.error = Some(t("settings:aiProvider.errorNameRequired"));
                return;
            }
            if url.is_empty() {
                f.error = Some(t("settings:aiProvider.errorBaseUrlRequired"));
                return;
            }
            if !url.starts_with("http://")
                && !url.starts_with("https://")
                && !is_native_locator(&url)
            {
                f.error = Some(t("settings:aiProvider.errorUrlScheme"));
                return;
            }
            if guard_key(f) || sync_endpoint(f) {
                return;
            }
            let next = url_posture(f);
            if !editing(f) && key_required(f) && f.api_key.text.trim().is_empty() {
                let label = f.preset.and_then(preset).map(|p| p.label).unwrap_or("");
                f.error = Some(tr(
                    "settings:aiProvider.errorApiKeyRequired",
                    &[("label", label)],
                ));
                return;
            }
            f.step = if next != "local" { "privacy" } else { "check" };
        }
        "privacy" => {
            if ack_needed(f, &f.kind.clone()) && !f.remote_ack {
                f.error = Some(t("settings:aiProvider.errorAckRequiredNext"));
            } else {
                f.step = "check";
            }
        }
        _ => {}
    }
}

fn go_back(app: &mut App) {
    let f = &mut app.settings.aiform;
    f.error = None;
    let ids = step_ids(f);
    let i = ids.iter().position(|s| *s == f.step).unwrap_or(0);
    if i > 0 {
        f.step = ids[i - 1];
    }
}

/// A selectable card (`ring-1 ring-border`, chosen = `ring-2 ring-primary bg-primary/5`).
fn choice(tk: &Tk, key: String, checked: bool, label: String) -> El {
    row()
        .key(key)
        .w_full()
        .gap(tk.r(0.75))
        .radius(tk.r(0.5))
        .fill(if checked {
            tk.primary.alpha(0.05)
        } else {
            tk.card
        })
        .border(
            if checked { 2.0 } else { 1.0 },
            if checked { tk.primary } else { tk.border },
        )
        .hover(if checked {
            tk.primary.alpha(0.05)
        } else {
            tk.muted.alpha(0.6)
        })
        .role("radio")
        .checked(checked)
        .label(label)
        .focusable(true)
}

fn stepper(tk: &Tk, f: &AiForm) -> El {
    let ids = step_ids(f);
    let cur = ids.iter().position(|s| *s == f.step).unwrap_or(0);
    let summary = |id: &str| -> Option<String> {
        match id {
            "provider" => f.preset.and_then(preset).map(|p| {
                if p.custom {
                    t(if p.kind == "local" {
                        "settings:aiProvider.presetCustom"
                    } else {
                        "settings:aiProvider.presetCustomApi"
                    })
                } else {
                    p.label.to_owned()
                }
            }),
            "connect" => Some(f.name.text.trim().to_owned()).filter(|s| !s.is_empty()),
            "privacy" => Some(t(&format!("settings:aiProvider.postureName.{}", f.kind))),
            _ => Some(f.model.text.trim().to_owned()).filter(|s| !s.is_empty()),
        }
    };
    let mut track = row()
        .w_full()
        .gap(tk.r(0.25))
        .padding(tk.r(0.25))
        .radius(tk.r(0.75))
        .fill(tk.background)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .role("navigation")
        .label(t("onboarding:stepper.ariaLabel"));
    for (i, id) in ids.iter().enumerate() {
        let done = i < cur;
        let active = i == cur;
        let label = t(&format!("settings:aiProvider.step.{id}"));
        let name = match (done, summary(id)) {
            (true, Some(s)) => format!("{label}: {s}"),
            _ => label.clone(),
        };
        let badge = row()
            .size(tk.r(1.25), tk.r(1.25))
            .center()
            .radius(tk.r(0.625))
            .fill(if done {
                tk.primary
            } else if active {
                tk.primary.alpha(0.1)
            } else {
                tk.card
            })
            .border(
                1.0,
                if done {
                    tk.primary
                } else if active {
                    tk.primary.alpha(0.3)
                } else {
                    tk.border
                },
            )
            .a11y_hidden()
            .child(if done {
                ic(&I::CHECK, tk.r(0.875), tk.primary_fg)
            } else {
                text((i + 1).to_string())
                    .font(tk.txs(500.0))
                    .color(if active { tk.fg } else { tk.muted_fg })
            });
        let target = *id;
        let mut b = row()
            .key(format!("set-aif-step-{id}"))
            .grow(1.0)
            .min_w(0.0)
            .gap(tk.r(0.5))
            .center_y()
            .px(tk.r(0.625))
            .py(tk.r(0.5))
            .radius(tk.r(0.5))
            .role("button")
            .label(name)
            .disabled(!done)
            .child(badge)
            .child(
                text(label)
                    .font(tk.tsm(500.0))
                    .color(if active { tk.fg } else { tk.muted_fg })
                    .truncate(),
            );
        if active {
            b = b.fill(tk.card).border(1.0, tk.border);
        }
        if done {
            b = b
                .focusable(true)
                .hover(tk.card)
                .on_press(move |a: &mut App| {
                    a.settings.aiform.error = None;
                    a.settings.aiform.step = target;
                });
        }
        track = track.child(b);
    }
    track
}

/// The add / edit provider wizard as a Settings sub-page (was a dialog over the modal): the
/// stepper under the heading, the step's fields, Back / Next / Save in the footer bar.
pub fn page(app: &App, tk: &Tk) -> super::subpage::Parts {
    let f = &app.settings.aiform;
    let edit = editing(f);
    let busy = f.saving.is_some();
    let ids = step_ids(f);
    let idx = ids.iter().position(|s| *s == f.step).unwrap_or(0);
    let last = idx == ids.len() - 1;
    let body = match f.step {
        "provider" => provider_step(app, tk, f),
        "privacy" => privacy_step(tk, f),
        "check" => check_step(app, tk, f),
        _ => connect_step(app, tk, f),
    };
    let needs_ack = ack_needed(f, &f.kind);
    let can_submit = !busy && (!needs_ack || f.remote_ack);
    let mut actions = vec![];
    // The first step's secondary action leaves the page (as the section's Back does); later
    // steps step back.
    actions.push(if idx == 0 {
        super::subpage::cancel_button(tk, "set-aif-cancel", !busy)
    } else {
        btn_lg(
            tk,
            "set-aif-back",
            &t("settings:aiProvider.back"),
            Btn::Ghost,
            !busy,
        )
        .press_if(!busy, go_back)
    });
    if edit {
        if !last {
            actions.push(
                btn_lg(
                    tk,
                    "set-aif-next",
                    &t("settings:aiProvider.next"),
                    Btn::Outline,
                    true,
                )
                .on_press(|a: &mut App| go_next(a)),
            );
        }
        actions.push(
            btn_lg(
                tk,
                "set-aif-save",
                &t("common:actions.save"),
                Btn::Primary,
                can_submit,
            )
            .press_if(can_submit, submit),
        );
    } else if last {
        actions.push(
            btn_lg(
                tk,
                "set-aif-add",
                &t("settings:aiProvider.addProviderButton"),
                Btn::Primary,
                can_submit,
            )
            .press_if(can_submit, submit),
        );
    } else {
        let ok = !(f.step == "provider" && f.preset.is_none());
        actions.push(
            btn_lg(
                tk,
                "set-aif-next",
                &t("settings:aiProvider.next"),
                Btn::Primary,
                ok,
            )
            .press_if(ok, go_next),
        );
    }
    let error = f.error.clone().map(|e| {
        text(e)
            .font(tk.txs(400.0))
            .color(tk.destructive)
            .role("alert")
            .grow(1.0)
            .min_w(0.0)
            .wrap()
    });
    // The side rail sums up what the wizard has so far: the provider, where its prompts go and
    // its address (an owner request; the web's dialog has no rail).
    let mut summary: Vec<El> = vec![];
    if let Some(reach) = f
        .route
        .map(|r| t(&format!("settings:aiProvider.route.{r}.reach")))
    {
        summary.push(text(reach).font(tk.tsm(400.0)).color(tk.muted_fg).wrap());
    }
    if !f.kind.is_empty() {
        let posture = t(&format!("settings:aiProvider.postureName.{}", f.kind));
        let ink = if f.kind == "local" {
            super::controls::emerald(tk)
        } else {
            super::controls::amber(tk)
        };
        summary.push(text(posture).font(tk.tsm(500.0)).color(ink));
    }
    if !f.base_url.text.trim().is_empty() {
        summary.push(
            text(f.base_url.text.trim().to_owned())
                .font(tk.txs(400.0).mono())
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    let name = f.name.text.trim();
    let side = if summary.is_empty() {
        vec![]
    } else {
        vec![super::subpage::side_card(
            tk,
            "set-aif-summary",
            Some(if name.is_empty() {
                t("settings:aiProvider.step.provider")
            } else {
                name.to_owned()
            }),
            summary,
        )]
    };
    super::subpage::Parts {
        aside: None,
        side,
        title: t(if edit {
            "settings:aiProvider.editTitle"
        } else {
            "settings:aiProvider.addTitle"
        }),
        description: Some(t("settings:aiProvider.wizardDescription")),
        head_extra: Some(stepper(tk, f)),
        body: col().w_full().gap(tk.r(1.0)).child(body),
        footer_left: error,
        actions,
        scroll_key: format!("set-aif-body-{}", f.step),
    }
}

fn provider_step(app: &App, tk: &Tk, f: &AiForm) -> El {
    let mut routes = col()
        .w_full()
        .gap(tk.r(0.5))
        .role("radiogroup")
        .label(t("settings:aiProvider.routeLabel"));
    for (id, _) in ROUTES {
        let checked = f.route == Some(id);
        let glyph = match id {
            "local" => &I::LAPTOP,
            "app" => &I::SQUARE_TERMINAL,
            _ => &I::KEY_ROUND,
        };
        let posture_local = id == "local";
        let badge_tone = if posture_local { &tk.good } else { &tk.warn };
        let posture = if posture_local { "local" } else { "remote" };
        let tint = rune::color::hex(if posture_local { "#10b981" } else { "#f59e0b" }).unwrap();
        let badge = row()
            .px(tk.r(0.375))
            .py(tk.r(0.125))
            .center_y()
            .radius(tk.r(0.5))
            .fill(tint.alpha(0.15))
            .help(t(&format!("settings:aiProvider.route.{id}.reach")))
            .child(
                text(t(&format!("assistant:modelPicker.posture.{posture}")).to_uppercase())
                    .font(tk.t3xs(500.0).lh(tk.r(0.625)).tracking(0.25))
                    .color(badge_tone.text),
            );
        routes = routes.child(
            choice(
                tk,
                format!("set-aif-route-{id}"),
                checked,
                t(&format!("settings:aiProvider.route.{id}.title")),
            )
            .padding(tk.r(0.75))
            .items_start()
            .on_press(move |a: &mut App| {
                let f = &mut a.settings.aiform;
                f.route = Some(id);
                if f.preset.is_some_and(|p| route_for(p) != Some(id)) {
                    f.preset = None;
                }
            })
            .child(
                row()
                    .size(tk.r(2.0), tk.r(2.0))
                    .center()
                    .radius(tk.r(0.375))
                    .fill(tk.muted)
                    .a11y_hidden()
                    .child(ic(glyph, tk.r(1.0), tk.fg)),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(tk.r(0.125))
                    .child(
                        row()
                            .gap(tk.r(0.5))
                            .center_y()
                            .child(
                                text(t(&format!("settings:aiProvider.route.{id}.title")))
                                    .font(tk.tsm(500.0))
                                    .color(tk.fg),
                            )
                            .child(badge),
                    )
                    .child(
                        text(t(&format!("settings:aiProvider.route.{id}.description")))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
            )
            .child(
                row()
                    .size(tk.r(1.0), tk.r(1.0))
                    .mt(tk.r(0.25))
                    .center()
                    .radius(tk.r(0.5))
                    .fill(if checked {
                        tk.primary
                    } else {
                        rune::Color::TRANSPARENT
                    })
                    .border(1.0, if checked { tk.primary } else { tk.border })
                    .a11y_hidden()
                    .child(if checked {
                        ic(&I::CHECK, tk.r(0.75), tk.primary_fg)
                    } else {
                        el()
                    }),
            ),
        );
    }
    let mut c = col().w_full().gap(tk.r(1.0)).child(routes);
    if let Some(route) = f.route {
        let ids = ROUTES
            .iter()
            .find(|(r, _)| *r == route)
            .map(|(_, ids)| *ids)
            .unwrap_or(&[]);
        let mut grid = col()
            .w_full()
            .gap(tk.r(0.5))
            .role("radiogroup")
            .label(t(&format!("settings:aiProvider.route.{route}.which")));
        let mut pair = row().w_full().gap(tk.r(0.5));
        for (i, id) in ids.iter().enumerate() {
            let p = preset(id).unwrap();
            let checked = f.preset == Some(p.id);
            let label = if p.custom {
                t(if p.kind == "local" {
                    "settings:aiProvider.presetCustom"
                } else {
                    "settings:aiProvider.presetCustomApi"
                })
            } else {
                p.label.to_owned()
            };
            let glyph = if p.custom {
                &I::PLUG
            } else if p.kind == "local" {
                &I::LAPTOP
            } else if is_native_locator(p.base_url) {
                &I::SQUARE_TERMINAL
            } else {
                &I::KEY_ROUND
            };
            let mut card = choice(tk, format!("set-aif-preset-{id}"), checked, label.clone())
                .grow(1.0)
                .min_w(0.0)
                .px(tk.r(0.75))
                .py(tk.r(0.625))
                .gap(tk.r(0.625))
                .center_y()
                .on_press(move |a: &mut App| apply_preset(a, p))
                .child(ic(glyph, tk.r(1.0), tk.muted_fg).a11y_hidden())
                .child(
                    text(label)
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                );
            if checked {
                card = card.child(ic(&I::CHECK, tk.r(0.875), tk.primary));
            }
            pair = pair.child(card);
            if i % 2 == 1 || i == ids.len() - 1 {
                if ids.len() % 2 == 1 && i == ids.len() - 1 {
                    pair = pair.child(el().grow(1.0));
                }
                grid = grid.child(std::mem::replace(&mut pair, row().w_full().gap(tk.r(0.5))));
            }
        }
        c = c.child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .child(
                    text(t(&format!("settings:aiProvider.route.{route}.which")))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(grid),
        );
        if let Some(p) = f.preset.and_then(preset) {
            let mut hint = col().w_full().gap(tk.r(0.375)).child(
                text(tr(
                    &format!("settings:aiProvider.presetHint.{}", p.hint),
                    &[("label", p.label)],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
            );
            if let Some(cmd) = p.command {
                hint = hint.child(
                    row()
                        .w_full()
                        .gap(tk.r(0.5))
                        .center_y()
                        .radius(tk.r(0.375))
                        .fill(tk.muted.alpha(0.6))
                        .pl(tk.r(0.625))
                        .pr(tk.r(0.25))
                        .py(tk.r(0.25))
                        .child(
                            text(cmd)
                                .font(tk.txs(400.0).mono())
                                .color(tk.fg)
                                .wrap()
                                .grow(1.0)
                                .min_w(0.0),
                        )
                        .child(copy_button(
                            app,
                            tk,
                            "set-aif-copy-cmd",
                            cmd.to_owned(),
                            t("settings:aiProvider.copyCommand"),
                        )),
                );
            }
            c = c.child(hint);
        }
    }
    c
}

fn connect_step(_app: &App, tk: &Tk, f: &AiForm) -> El {
    let edit = editing(f);
    let kr = key_required(f);
    let mut c = col().w_full().gap(tk.r(1.0));
    c = c.child(
        col()
            .w_full()
            .gap(tk.r(0.5))
            .child(labelled(
                tk,
                &t("settings:aiProvider.nameLabel"),
                input(
                    tk,
                    &f.name,
                    "set-aif-name",
                    &t("settings:aiProvider.nameLabel"),
                    "oMLX",
                    false,
                    true,
                    tk.r(2.25),
                    false,
                    |a: &mut App| &mut a.settings.aiform.name,
                ),
            ))
            .child(
                text(t("settings:aiProvider.nameHint"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            ),
    );
    let ph = f
        .preset
        .and_then(preset)
        .and_then(|p| p.url_placeholder)
        .unwrap_or("http://127.0.0.1:8000/v1 or claude-cli://default");
    // The web applies posture, key origin and endpoint rules on blur; here on Next, Test and
    // Save, and the notes below say why something changed.
    let mut url = col().w_full().gap(tk.r(0.5)).child(labelled(
        tk,
        &t("settings:aiProvider.urlLabel"),
        input(
            tk,
            &f.base_url,
            "set-aif-url",
            &t("settings:aiProvider.urlLabel"),
            ph,
            false,
            true,
            tk.r(2.25),
            false,
            |a: &mut App| &mut a.settings.aiform.base_url,
        ),
    ));
    let auto_shown = f.auto_remote.is_some()
        && f.kind == "remote"
        && match f.auto_remote {
            Some("cli") => is_native_locator(&f.base_url.text),
            _ => {
                (f.base_url.text.trim().starts_with("http://")
                    || f.base_url.text.trim().starts_with("https://"))
                    && !is_loopback_http(&f.base_url.text)
            }
        };
    if let (Some(r), true) = (f.auto_remote, auto_shown) {
        url = url.child(
            note(
                tk,
                &I::INFO,
                t(&format!("settings:aiProvider.autoRemote.{r}")),
                amber_soft(tk),
            )
            .role("status"),
        );
    }
    if let Some(r) = f.endpoint_reset {
        url = url.child(note(
            tk,
            &I::INFO,
            t(if r == "exemption" {
                "settings:aiProvider.exemptionResetForAddress"
            } else {
                "settings:aiProvider.formatResetForAddress"
            }),
            amber_soft(tk),
        ));
    }
    c = c.child(url);
    let key_hint = t(if kr {
        "settings:aiProvider.apiKeyRequiredHint"
    } else if f.kind == "local" {
        "settings:aiProvider.apiKeyLocalHint"
    } else {
        "settings:aiProvider.apiKeyRemoteHint"
    });
    let key_ph = if edit
        && f.initial
            .as_ref()
            .is_some_and(|i| i["has_api_key"].as_bool() == Some(true))
    {
        t("settings:aiProvider.apiKeyPlaceholderKeep")
    } else {
        "sk-…".into()
    };
    let key_field = input(
        tk,
        &f.api_key,
        "set-aif-key",
        &t("settings:aiProvider.apiKeyLabel"),
        &key_ph,
        true,
        true,
        tk.r(2.25),
        false,
        |a: &mut App| &mut a.settings.aiform.api_key,
    );
    let mut key = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .child(
                    text(t("settings:aiProvider.apiKeyLabel"))
                        .font(tk.tsm(500.0).lh(tk.r(0.875)))
                        .color(tk.fg),
                )
                .child(text(key_hint).font(tk.txs(400.0)).color(tk.muted_fg)),
        )
        .child(key_field)
        .child(note(
            tk,
            &I::KEY_ROUND,
            t("settings:aiProvider.apiKeyStorageHint"),
            tk.muted_fg,
        ));
    if f.key_cleared {
        key = key.child(note(
            tk,
            &I::INFO,
            t("settings:aiProvider.keyClearedForAddress"),
            amber_soft(tk),
        ));
    }
    c = c.child(key);
    // Advanced: posture, API format, store field, notes.
    let cli = is_cli_url(f);
    let proxy = f.kind == "cli_proxy";
    let summary = if cli || proxy {
        t(&format!("settings:aiProvider.postureName.{}", f.kind))
    } else {
        tr(
            "settings:aiProvider.advancedSummary",
            &[(
                "format",
                &t(&format!("settings:aiProvider.apiFormat.{}", f.api_format)),
            )],
        )
    };
    let open = f.advanced_open;
    let mut adv = col()
        .w_full()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .child(
            row()
                .key("set-aif-advanced")
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.375))
                .hover(tk.muted.alpha(0.4))
                .role("button")
                .expanded(open)
                .focusable(true)
                .on_press(|a: &mut App| {
                    a.settings.aiform.advanced_open = !a.settings.aiform.advanced_open
                })
                .child(
                    ic(
                        if open {
                            &I::CHEVRON_DOWN
                        } else {
                            &I::CHEVRON_RIGHT
                        },
                        tk.r(0.875),
                        tk.muted_fg,
                    )
                    .a11y_hidden(),
                )
                .child(
                    text(t("settings:aiProvider.advancedLabel"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(el().grow(1.0))
                .child(
                    text(summary)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate(),
                ),
        );
    if open {
        let mut inner = col()
            .w_full()
            .gap(tk.r(0.75))
            .px(tk.r(0.75))
            .py(tk.r(0.625))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .border_layout();
        let mut posture = col().w_full().gap(tk.r(0.375)).child(
            text(t("settings:aiProvider.postureLabel"))
                .font(tk.txs(500.0))
                .color(tk.fg),
        );
        if !proxy {
            let mut seg = row()
                .w_full()
                .gap(tk.r(0.375))
                .role("group")
                .label(t("settings:aiProvider.postureLabel"));
            for k in ["local", "remote", "tee"] {
                let on = f.kind == k;
                seg = seg.child(
                    row()
                        .key(format!("set-aif-kind-{k}"))
                        .grow(1.0)
                        .px(tk.r(0.5))
                        .py(tk.r(0.375))
                        .center()
                        .radius(tk.r(0.375))
                        .border(1.0, if on { tk.primary } else { tk.border })
                        .fill(if on { tk.primary } else { tk.background })
                        .hover(if on { tk.primary } else { tk.muted })
                        .role("button")
                        .checked(on)
                        .label(t(&format!("settings:aiProvider.posture.{k}")))
                        .focusable(true)
                        .on_press(move |a: &mut App| {
                            a.settings.aiform.kind = k.to_owned();
                            a.settings.aiform.auto_remote = None;
                        })
                        .child(
                            text(t(&format!("settings:aiProvider.posture.{k}")).to_uppercase())
                                .font(tk.txs(500.0).tracking(0.3))
                                .color(if on { tk.primary_fg } else { tk.muted_fg }),
                        ),
                );
            }
            posture = posture.child(seg);
        }
        posture = posture.child(
            text(t(&format!("settings:aiProvider.kindHint.{}", f.kind)))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
        inner = inner.child(posture);
        if !cli && !proxy {
            let mut fmts = col()
                .w_full()
                .gap(tk.r(0.375))
                .role("radiogroup")
                .label(t("settings:aiProvider.apiFormatLabel"));
            for fmt in ["auto", "responses", "chat_completions"] {
                let on = f.api_format == fmt;
                fmts = fmts.child(
                    row()
                        .key(format!("set-aif-fmt-{fmt}"))
                        .w_full()
                        .gap(tk.r(0.5))
                        .items_start()
                        .px(tk.r(0.625))
                        .py(tk.r(0.5))
                        .radius(tk.r(0.375))
                        .border(1.0, if on { tk.primary } else { tk.border })
                        .fill(if on {
                            tk.primary.alpha(0.05)
                        } else {
                            rune::Color::TRANSPARENT
                        })
                        .hover(tk.muted.alpha(0.4))
                        .role("radio")
                        .checked(on)
                        .label(t(&format!("settings:aiProvider.apiFormat.{fmt}")))
                        .focusable(true)
                        .on_press(move |a: &mut App| {
                            let f = &mut a.settings.aiform;
                            sync_endpoint(f);
                            f.endpoint_reset = None;
                            f.api_format = fmt;
                        })
                        .child(
                            row()
                                .size(tk.r(0.875), tk.r(0.875))
                                .mt(2.0)
                                .center()
                                .radius(tk.r(0.4375))
                                .border(1.0, if on { tk.primary } else { tk.input })
                                .child(if on {
                                    el().size(tk.r(0.4375), tk.r(0.4375))
                                        .radius(tk.r(0.22))
                                        .fill(tk.primary)
                                } else {
                                    el()
                                }),
                        )
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(tk.r(0.125))
                                .child(
                                    text(t(&format!("settings:aiProvider.apiFormat.{fmt}")))
                                        .font(tk.txs(500.0))
                                        .color(tk.fg),
                                )
                                .child(
                                    text(t(&format!("settings:aiProvider.apiFormatHint.{fmt}")))
                                        .font(tk.txs(400.0))
                                        .color(tk.muted_fg)
                                        .wrap(),
                                ),
                        ),
                );
            }
            inner = inner
                .child(
                    text(t("settings:aiProvider.apiFormatLabel"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(fmts);
            let saved = f
                .initial
                .as_ref()
                .filter(|i| i["base_url"].as_str().unwrap_or("").trim() == f.base_url.text.trim())
                .and_then(|i| i["detected_api_format"].as_str());
            if let Some(d) = saved {
                inner = inner.child(
                    text(tr(
                        "settings:aiProvider.apiFormatLastDetected",
                        &[(
                            "format",
                            &t(&format!("settings:aiProvider.apiFormatName.{d}")),
                        )],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                );
            }
            inner = inner.child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .items_start()
                    .px(tk.r(0.625))
                    .py(tk.r(0.5))
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border)
                    .child(checkbox(
                        tk,
                        "set-aif-omit",
                        f.omit_store,
                        &t("settings:aiProvider.omitStoreLabel"),
                        |a| {
                            let f = &mut a.settings.aiform;
                            sync_endpoint(f);
                            f.endpoint_reset = None;
                            f.omit_store = !f.omit_store;
                        },
                    ))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(tk.r(0.125))
                            .child(
                                text(t("settings:aiProvider.omitStoreLabel"))
                                    .font(tk.txs(500.0))
                                    .color(tk.fg),
                            )
                            .child(
                                text(t("settings:aiProvider.omitStoreHint"))
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg)
                                    .wrap(),
                            ),
                    ),
            );
        }
        inner = inner.child(
            col()
                .w_full()
                .gap(tk.r(0.375))
                .child(
                    text(t("settings:aiProvider.notesLabel"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(input(
                    tk,
                    &f.notes,
                    "set-aif-notes",
                    &t("settings:aiProvider.notesLabel"),
                    &t("settings:aiProvider.notesPlaceholder"),
                    false,
                    true,
                    tk.r(2.25),
                    false,
                    |a: &mut App| &mut a.settings.aiform.notes,
                )),
        );
        adv = adv.child(inner);
    }
    c.child(adv)
}

fn privacy_step(tk: &Tk, f: &AiForm) -> El {
    let amber = rune::color::hex("#f59e0b").unwrap();
    let title = t(if f.kind == "tee" {
        "settings:aiProvider.privacy.titleTee"
    } else {
        "settings:aiProvider.privacy.title"
    });
    let name = if f.name.text.trim().is_empty() {
        f.preset
            .and_then(preset)
            .map(|p| p.label.to_owned())
            .unwrap_or_default()
    } else {
        f.name.text.trim().to_owned()
    };
    let title = title.replace("{{name}}", &name);
    let check_li = |s: String| {
        row()
            .w_full()
            .gap(tk.r(0.5))
            .items_start()
            .child(ic(&I::CHECK, tk.r(0.875), emerald(tk)).mt(1.0))
            .child(
                text(s)
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
            )
    };
    let cli = is_native_locator(&f.base_url.text) || f.kind == "cli_proxy";
    let mut c = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_start()
                .padding(tk.r(0.75))
                .radius(tk.r(0.5))
                .border(1.0, amber.alpha(0.4))
                .fill(amber.alpha(0.1))
                .child(ic(&I::SHIELD_ALERT, tk.r(1.0), amber_soft(tk)).mt(2.0))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.375))
                        .child(text(title).font(tk.tsm(500.0)).color(amber_soft(tk)).wrap())
                        .child(
                            text(t("settings:aiProvider.remoteWarning"))
                                .font(tk.txs(400.0))
                                .color(amber_soft(tk))
                                .wrap(),
                        ),
                ),
        )
        .child(
            col()
                .w_full()
                .gap(tk.r(0.375))
                .child(check_li(t("settings:aiProvider.privacy.onlyWhenAsked")))
                .child(check_li(t("settings:aiProvider.privacy.noSecrets")))
                .child(check_li(t(if cli {
                    "settings:aiProvider.privacy.cliTerms"
                } else {
                    "settings:aiProvider.privacy.retention"
                }))),
        );
    if ack_needed(f, &f.kind) {
        c = c.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .padding(tk.r(0.75))
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .child(checkbox(
                    tk,
                    "set-aif-ack",
                    f.remote_ack,
                    &t("settings:aiProvider.remoteAck"),
                    |a| {
                        a.settings.aiform.remote_ack = !a.settings.aiform.remote_ack;
                        a.settings.aiform.error = None;
                    },
                ))
                .child(
                    text(t("settings:aiProvider.remoteAck"))
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    } else {
        c = c.child(
            text(t("settings:aiProvider.privacy.acknowledged"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    c.child(
        row()
            .w_full()
            .gap(tk.r(0.5))
            .items_start()
            .padding(tk.r(0.75))
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .child(checkbox(
                tk,
                "set-aif-restrict",
                f.restrict,
                &t("settings:aiProvider.restrictAccessLabel"),
                |a| a.settings.aiform.restrict = !a.settings.aiform.restrict,
            ))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(tk.r(0.125))
                    .child(
                        text(t("settings:aiProvider.restrictAccessLabel"))
                            .font(tk.tsm(500.0))
                            .color(tk.fg),
                    )
                    .child(
                        text(t("settings:aiProvider.restrictAccessHint"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
            ),
    )
}

fn check_step(app: &App, tk: &Tk, f: &AiForm) -> El {
    let proxy = f.kind == "cli_proxy";
    let busy = f.saving.is_some();
    let disabled_reason = (editing(f)
        && f.initial
            .as_ref()
            .is_some_and(|i| i["enabled"].as_bool() == Some(false)))
    .then(|| t("settings:aiProvider.errorTestDisabled"));
    let running = f.test.state == "running" && f.test.target == test_fingerprint(f);
    let ok = !busy && disabled_reason.is_none();
    let test_btn = super::controls::btn(
        tk,
        "set-aif-test",
        &t(if proxy {
            "settings:aiProvider.testDiscover"
        } else {
            "settings:aiProvider.testConnection"
        }),
        Btn::Outline,
        ok,
    )
    .h(tk.r(2.25))
    .help(disabled_reason.unwrap_or_else(|| t("settings:aiProvider.testHint")))
    .press_if(ok, handle_test);
    let test_btn = if running {
        test_btn
            .child(ic(&I::LOADER_CIRCLE, tk.r(0.875), tk.fg).looping(rune::LoopSpec::rotation(0.9)))
    } else {
        test_btn
    };
    let current = f.test.state != "idle" && f.test.target == test_fingerprint(f);
    let suggestions: Vec<String> = if !is_cli_url(f)
        && (f.test.state == "ok" || f.test.state == "no_model")
        && f.test.listing == listing_fingerprint(f)
    {
        f.test.data["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| {
                m["unsupported_reason"].is_null() && m["supports_tools"].as_bool() != Some(false)
            })
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect()
    } else {
        vec![]
    };
    let mut c = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(
            text(t("settings:aiProvider.defaultModelLabel"))
                .font(tk.tsm(500.0).lh(tk.r(0.875)))
                .color(tk.fg),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .child(
                    input(
                        tk,
                        &f.model,
                        "set-aif-model",
                        &t("settings:aiProvider.defaultModelLabel"),
                        "qwen3.6:35b",
                        false,
                        true,
                        tk.r(2.25),
                        false,
                        |a: &mut App| &mut a.settings.aiform.model,
                    )
                    .grow(1.0)
                    .min_w(0.0),
                )
                .child(test_btn),
        );
    if suggestions.is_empty() {
        c = c.child(
            text(t(if proxy {
                "settings:aiProvider.checkHintCliProxy"
            } else {
                "settings:aiProvider.checkHint"
            }))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap(),
        );
    } else {
        // The web offers these through a `<datalist>`; Rune has none, so they are chips.
        let mut chips = row().w_full().gap(tk.r(0.25)).flex_wrap();
        for id in suggestions.iter().take(24) {
            let m = id.clone();
            chips = chips.child(
                super::controls::btn(tk, format!("set-aif-model-{id}"), id, Btn::Ghost, true)
                    .h(tk.r(1.5))
                    .on_press(move |a: &mut App| {
                        a.settings.aiform.model = TextEdit::new(m.clone())
                    }),
            );
        }
        c = c.child(chips).child(
            text(tc(
                "settings:aiProvider.modelSuggestions",
                suggestions.len(),
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    let mut out = col().w_full().gap(tk.r(1.0)).child(c);
    if current {
        let d = &f.test.data;
        let mut r = col()
            .w_full()
            .gap(tk.r(0.25))
            .padding(tk.r(0.75))
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .fill(tk.muted.alpha(0.2))
            .role("status");
        let n = d["model_count"].as_u64().unwrap_or(0) as usize;
        match f.test.state {
            "running" => {
                r = r.child(
                    text(t("settings:aiProvider.testing"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                )
            }
            "fail" => {
                r = r.child(
                    text(tr(
                        "settings:aiProvider.testFailed",
                        &[("message", &f.test.message)],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.destructive)
                    .wrap(),
                )
            }
            "no_model" if proxy => {
                r = r.child(note(
                    tk,
                    &I::CIRCLE_CHECK,
                    tc("settings:aiProvider.discovered", n, &[]),
                    emerald(tk),
                ));
                let unusable = d["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| !m["unsupported_reason"].is_null())
                    .count();
                if unusable > 0 {
                    r = r.child(
                        text(tc("settings:aiProvider.discoveredUnusable", unusable, &[]))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .pl(tk.r(1.25)),
                    );
                }
                if d["models_saved"].as_bool() == Some(true) {
                    r = r.child(
                        text(t("settings:aiProvider.discoveredSaved"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .pl(tk.r(1.25)),
                    );
                }
                match d["probe"]["reason"].as_str() {
                    Some(reason @ ("model_not_listed" | "not_a_chat_model" | "unknown_family")) => {
                        let key = match reason {
                            "model_not_listed" => "settings:aiProvider.testModelNotListed",
                            "not_a_chat_model" => "settings:aiProvider.testModelNotChat",
                            _ => "settings:aiProvider.testModelUnknownFamily",
                        };
                        r = r.child(note(
                            tk,
                            &I::TRIANGLE_ALERT,
                            tr(
                                key,
                                &[("model", d["probe"]["model"].as_str().unwrap_or(""))],
                            ),
                            amber_soft(tk),
                        ));
                    }
                    _ => {
                        r = r.child(
                            text(t("settings:aiProvider.discoveredChoose"))
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .pl(tk.r(1.25)),
                        )
                    }
                }
            }
            "no_model" => {
                r = r
                    .child(note(
                        tk,
                        &I::TRIANGLE_ALERT,
                        tc("settings:aiProvider.testNoReply", n, &[]),
                        amber_soft(tk),
                    ))
                    .child(
                        text(t("settings:aiProvider.testNoModel"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .pl(tk.r(1.25))
                            .wrap(),
                    );
            }
            _ if d["check_kind"].as_str() == Some("binary_presence") => {
                r = r.child(
                    text(t("settings:aiProvider.testOkBinary"))
                        .font(tk.txs(400.0))
                        .color(emerald(tk)),
                )
            }
            _ => {
                r = r.child(note(
                    tk,
                    &I::CIRCLE_CHECK,
                    tc("settings:aiProvider.testOkConnected", n, &[]),
                    emerald(tk),
                ));
                let li = |s: String| {
                    text(s)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .pl(tk.r(1.25))
                        .wrap()
                };
                if let Some(m) = d["probe"]["model"]
                    .as_str()
                    .filter(|_| d["probe"]["status"].as_str() == Some("ok"))
                {
                    r = r.child(li(tr(
                        "settings:aiProvider.testReplyChecked",
                        &[("model", m)],
                    )));
                }
                if let Some(fmt) = d["probe"]["api_format"]
                    .as_str()
                    .filter(|_| d["probe"]["status"].as_str() == Some("ok"))
                {
                    r = r.child(li(tr(
                        "settings:aiProvider.testFormatUsed",
                        &[(
                            "format",
                            &t(&format!("settings:aiProvider.apiFormatName.{fmt}")),
                        )],
                    )));
                }
                if d["models_saved"].as_bool() == Some(true) {
                    r = r.child(li(t("settings:aiProvider.discoveredSaved")));
                }
                if let Some(fmt) = d["detected_api_format"].as_str() {
                    let key = if d["api_format"].as_str() == Some("auto") {
                        "settings:aiProvider.testFormatDetected"
                    } else {
                        "settings:aiProvider.testFormatSet"
                    };
                    r = r.child(li(tr(
                        key,
                        &[(
                            "format",
                            &t(&format!("settings:aiProvider.apiFormatName.{fmt}")),
                        )],
                    )));
                }
                if d["tools_accepted"].as_bool() == Some(true) {
                    r = r.child(li(t("settings:aiProvider.testToolsAccepted")));
                }
                if d["store_accepted"].as_bool() == Some(false) {
                    r = r.child(note(
                        tk,
                        &I::SHIELD_ALERT,
                        t("settings:aiProvider.testStoreRefused"),
                        amber_soft(tk),
                    ));
                }
                if d["store_accepted"].is_null() && omit_to_save(f) == Some(true) {
                    r = r.child(note(
                        tk,
                        &I::SHIELD_ALERT,
                        t("settings:aiProvider.testStoreOmitted"),
                        amber_soft(tk),
                    ));
                }
                if d["tools_accepted"].as_bool() == Some(false) {
                    r = r.child(note(
                        tk,
                        &I::SHIELD_ALERT,
                        t("settings:aiProvider.testToolsRefused"),
                        amber_soft(tk),
                    ));
                }
            }
        }
        out = out.child(r);
    }
    let _ = app;
    out
}
