//! Evidence and connection requests (EvidenceRequestCard.tsx, WalletInputRequestCard.tsx and the
//! evidence half of AssistantSessionProvider): missing evidence is a local hand-off, never an
//! accounting approval.
//!
//! "Connect wallet" / "Import history" revalidate the request with the daemon first, then open
//! the add-connection dialog (`wallets::add`); a finished setup continues the review with a turn
//! naming the outcome. "Attach evidence" revalidates, then the native picker hands the file to
//! the daemon's staging (`internal.document_import.stage` with the review case), exactly as the
//! Tauri shell does; the model only ever sees the returned token. "Mark unavailable" continues
//! the review saying so.

use std::collections::{HashMap, HashSet};

use rune::{El, Tx, col, row, text};
use serde_json::{Value, json};

use super::chat::{self, Book, EvidenceTurn};
use super::model::{ChatFile, FileState, ToolCall, ToolStatus};
use crate::app::{App, Reply};
use crate::i18n::t;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button};

pub struct Pending {
    pub request: Value,
    pub origin: u64,
    pub revision: u64,
    pub outcome: Option<&'static str>,
    pub attachment: Option<String>,
}

#[derive(Default)]
pub struct State {
    /// `evidenceRequests`: status (and error) by request id.
    pub states: HashMap<String, (&'static str, Option<String>)>,
    /// The conversation generation each request was first seen in (`evidenceOriginsRef`).
    pub origins: HashMap<String, u64>,
    pub continued: HashSet<String>,
    pub pending: Option<Pending>,
    pub check_ids: Vec<(String, String)>,
    pub stage_ids: Vec<(String, String)>,
    /// The revalidated attach request waiting for the native picker (opened on the next poll).
    pub want_picker: Option<String>,
    /// The add-connection dialog opened for an evidence request: (request id, a save ran).
    pub dialog: Option<(String, bool)>,
    /// Connection request cards by call id: `idle` / `open` / `connected`.
    pub wallet_cards: HashMap<String, &'static str>,
    pub wallet_dialog: Option<(String, bool)>,
}

/// `evidenceRequest`: a finished, well-formed `request_input` packet.
pub fn parse(c: &ToolCall) -> Option<&Value> {
    if !["ui.review.request_input", "ui.source_funds.request_input"].contains(&c.name.as_str())
        || c.status != ToolStatus::Done
    {
        return None;
    }
    let env = c.result.as_ref()?;
    let d = &env["data"];
    let hex64 = |v: &Value| {
        v.as_str().is_some_and(|s| {
            s.len() == 64
                && s.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        })
    };
    let action = d["action"].as_str()?;
    let cases = d["cases"].as_array()?;
    let sof = c.name == "ui.source_funds.request_input";
    let prefix = if sof { "source_funds" } else { "quarantine" };
    let ok = env["kind"].as_str() == Some(c.name.as_str())
        && d["schema_version"].as_u64() == Some(1)
        && hex64(&d["request_id"])
        && d["workspace_id"].as_str().is_some_and(|s| !s.is_empty())
        && d["profile_id"].as_str().is_some_and(|s| !s.is_empty())
        && d["input_version"].as_u64().is_some()
        && ["connect_wallet", "import_history", "attach_evidence"].contains(&action)
        && !cases.is_empty()
        && cases.len() <= 20
        && (action != "attach_evidence" || cases.len() == 1)
        && (d["explanation"].is_null()
            || d["explanation"]
                .as_str()
                .is_some_and(|s| s.chars().count() <= 1000))
        && cases.iter().all(|i| {
            let tx = i["transaction_id"].as_str().unwrap_or("");
            !tx.is_empty()
                && i["case_id"].as_str() == Some(&format!("{prefix}:{tx}"))
                && i["reason"].is_string()
        });
    if !ok {
        return None;
    }
    if sof {
        let fine = d["domain"] == "source_funds"
            && cases.len() == 1
            && d["target_transaction"] == cases[0]["transaction_id"]
            && hex64(&d["review_fingerprint"])
            && d["recipe"].is_object()
            && d["recipe"].to_string().len() <= 16000;
        if !fine {
            return None;
        }
    } else if !d["domain"].is_null() {
        return None;
    }
    Some(d)
}

fn rid(req: &Value) -> String {
    req["request_id"].as_str().unwrap_or_default().to_owned()
}

fn book_of(req: &Value) -> Book {
    (
        req["workspace_id"].as_str().unwrap_or_default().to_owned(),
        req["profile_id"].as_str().unwrap_or_default().to_owned(),
    )
}

fn set(app: &mut App, id: &str, status: &'static str, error: Option<String>) {
    app.assistant
        .evidence
        .states
        .insert(id.to_owned(), (status, error));
}

pub fn status(app: &App, id: &str) -> (&'static str, Option<String>) {
    app.assistant
        .evidence
        .states
        .get(id)
        .cloned()
        .unwrap_or(("idle", None))
}

pub fn clear(app: &mut App) {
    let s = &mut app.assistant.evidence;
    s.states.clear();
    s.origins.clear();
    s.continued.clear();
    s.pending = None;
    s.want_picker = None;
    s.dialog = None;
}

/// `currentEvidence`: the request still belongs to this conversation.
fn current(app: &App, req: &Value) -> bool {
    let id = rid(req);
    app.assistant.evidence.origins.get(&id) == Some(&app.assistant.generation)
        && app.assistant.messages.iter().any(|m| {
            m.tool_calls
                .iter()
                .any(|c| parse(c).is_some_and(|d| rid(d) == id))
        })
}

fn active(app: &App, id: &str) -> bool {
    app.assistant
        .evidence
        .pending
        .as_ref()
        .is_some_and(|p| rid(&p.request) == id && p.origin == app.assistant.generation)
}

/// `openEvidenceRequest`: revalidate the current cases before any picker or setup.
pub fn open(app: &mut App, req: Value) {
    if app.assistant.streaming || !app.prefs.ai_features {
        return;
    }
    let id = rid(&req);
    let s = &app.assistant.evidence;
    if s.pending.is_some() || s.continued.contains(&id) {
        return;
    }
    if !current(app, &req) {
        set(app, &id, "stale", None);
        return;
    }
    let scope = json!({"workspace_id": req["workspace_id"], "profile_id": req["profile_id"]});
    let (kind, args) = if req["domain"] == "source_funds" {
        (
            "ui.source_funds.request_input",
            json!({"action": req["action"], "target_transaction": req["target_transaction"], "recipe": req["recipe"], "expected_review_fingerprint": req["review_fingerprint"], "explanation": req["explanation"], "expected_scope": scope}),
        )
    } else {
        let ids: Vec<Value> = req["cases"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| c["case_id"].clone())
            .collect();
        (
            "ui.review.request_input",
            json!({"action": req["action"], "case_ids": ids, "expected_input_version": req["input_version"], "explanation": req["explanation"], "expected_scope": scope}),
        )
    };
    app.assistant.evidence.pending = Some(Pending {
        request: req,
        origin: app.assistant.generation,
        revision: app.assistant.prompt_revision,
        outcome: None,
        attachment: None,
    });
    set(app, &id, "opening", None);
    if let Some(r) = app.send(kind, args, "assistant.evidence_check") {
        app.assistant.evidence.check_ids.push((r, id));
    }
}

fn fail(app: &mut App, id: &str, msg: String) {
    if active(app, id) {
        app.assistant.evidence.pending = None;
        set(app, id, "error", Some(msg));
    }
}

fn take(app: &App, list: &mut Vec<(String, String)>) -> Option<String> {
    let i = list
        .iter()
        .position(|(r, _)| !app.pending.contains_key(r))?;
    Some(list.remove(i).1)
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "assistant.evidence_check" => {
            let mut list = std::mem::take(&mut app.assistant.evidence.check_ids);
            let id = take(app, &mut list);
            app.assistant.evidence.check_ids = list;
            let Some(id) = id else { return };
            if !active(app, &id) {
                return;
            }
            if let Some(e) = &r.error {
                return fail(app, &id, e.clone());
            }
            if r.data["request_id"].as_str() != Some(id.as_str()) {
                return fail(app, &id, t("assistant:evidence.stale"));
            }
            let req = app
                .assistant
                .evidence
                .pending
                .as_ref()
                .map(|p| p.request.clone())
                .unwrap_or(Value::Null);
            if !current(app, &req) {
                return fail(app, &id, t("assistant:evidence.stale"));
            }
            match req["action"].as_str() {
                Some("attach_evidence") => app.assistant.evidence.want_picker = Some(id),
                Some("import_history") => {
                    crate::wallets::add::open(app, None);
                    app.assistant.evidence.dialog = Some((id, false));
                }
                _ => {
                    crate::wallets::add::open(app, Some("descriptor"));
                    app.assistant.evidence.dialog = Some((id, false));
                }
            }
        }
        "assistant.evidence_stage" => {
            let mut list = std::mem::take(&mut app.assistant.evidence.stage_ids);
            let id = take(app, &mut list);
            app.assistant.evidence.stage_ids = list;
            let Some(id) = id else { return };
            if !active(app, &id) {
                return;
            }
            if let Some(e) = &r.error {
                return fail(app, &id, e.clone());
            }
            let req = app
                .assistant
                .evidence
                .pending
                .as_ref()
                .map(|p| p.request.clone())
                .unwrap_or(Value::Null);
            let d = &r.data;
            let verified = d["attachment_id"].as_str().is_some_and(|s| !s.is_empty())
                && d["transaction_id"] == req["cases"][0]["transaction_id"];
            if !verified {
                return fail(app, &id, t("assistant:evidence.attachmentUnverified"));
            }
            let token = d["document_token"].as_str().unwrap_or_default().to_owned();
            let view = super::model::AttachmentView {
                filename: d["source"]["filename"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                kind: d["source"]["kind"].as_str().map(str::to_owned),
                size_bytes: d["source"]["size_bytes"].as_u64(),
            };
            complete(app, &id, "attached", Some((token, view)));
        }
        _ => {}
    }
}

/// `completeEvidence`: the file joins the chat (when there is room), then the review continues
/// on its own if nothing else is going on.
fn complete(
    app: &mut App,
    id: &str,
    outcome: &'static str,
    attachment: Option<(String, super::model::AttachmentView)>,
) {
    if !active(app, id) {
        return;
    }
    let req = app
        .assistant
        .evidence
        .pending
        .as_ref()
        .map(|p| p.request.clone())
        .unwrap_or(Value::Null);
    if !current(app, &req) {
        return;
    }
    let mut final_outcome = outcome;
    let mut kept = None;
    if let Some((token, view)) = attachment {
        let book = book_of(&req);
        let bound_ok = app.assistant.book.as_ref().is_none_or(|b| *b == book);
        let room = app.assistant.files.len() < super::composer::MAX_FILES;
        if bound_ok && (room || app.assistant.files.iter().any(|f| f.key == token)) {
            if !app.assistant.files.iter().any(|f| f.key == token) {
                app.assistant.files.push(ChatFile {
                    key: token.clone(),
                    view,
                    state: FileState::Ready,
                    book: Some(book.clone()),
                });
            }
            if app.assistant.book.is_none() {
                app.assistant.book = Some(book);
            }
            kept = Some(token);
        } else {
            let max = super::composer::MAX_FILES.to_string();
            app.toast(
                crate::app::ToastTone::Info,
                &crate::i18n::tr("assistant:composer.attachmentLimit.title", &[("max", &max)]),
                Some(t("assistant:evidence.attachedNotInChat")),
                Some("assistant-attachmentLimit"),
            );
            final_outcome = "attachedNotInChat";
        }
    }
    if let Some(p) = app.assistant.evidence.pending.as_mut() {
        p.outcome = Some(final_outcome);
        p.attachment = kept;
    }
    app.assistant.evidence.dialog = None;
    set(
        app,
        id,
        if outcome == "partial" {
            "partial"
        } else {
            "received"
        },
        None,
    );
    let (origin, revision) = app
        .assistant
        .evidence
        .pending
        .as_ref()
        .map(|p| (p.origin, p.revision))
        .unwrap_or((0, 0));
    let can_auto = origin == app.assistant.generation
        && revision == app.assistant.prompt_revision
        && !app.assistant.streaming
        && chat::has_model(app)
        && app.assistant.draft.text.trim().is_empty()
        && app.assistant.queued.is_empty();
    if can_auto {
        resume(app, req, false);
    }
}

/// `continueEvidenceRequest`: a turn naming what happened (or that the evidence is unavailable).
pub fn resume(app: &mut App, req: Value, unavailable: bool) {
    let id = rid(&req);
    let s = &app.assistant.evidence;
    let resumable = !s.continued.contains(&id)
        && s.pending
            .as_ref()
            .is_none_or(|p| rid(&p.request) == id && p.outcome.is_some());
    if !resumable {
        return;
    }
    if !current(app, &req) {
        set(app, &id, "stale", None);
        return;
    }
    if unavailable {
        app.assistant.evidence.pending = Some(Pending {
            request: req.clone(),
            origin: app.assistant.generation,
            revision: app.assistant.prompt_revision,
            outcome: Some("unavailable"),
            attachment: None,
        });
    }
    if !chat::has_model(app)
        || app.assistant.streaming
        || !app.assistant.queued.is_empty()
        || !app.assistant.draft.text.trim().is_empty()
    {
        set(
            app,
            &id,
            if unavailable {
                "unavailable"
            } else {
                "received"
            },
            None,
        );
        return;
    }
    let pending = app.assistant.evidence.pending.take();
    let outcome = if unavailable {
        "unavailable"
    } else {
        pending
            .as_ref()
            .and_then(|p| p.outcome)
            .unwrap_or("received")
    };
    let attachment = pending.and_then(|p| p.attachment);
    app.assistant.evidence.continued.insert(id.clone());
    set(app, &id, "continuing", None);
    let cases: Vec<&str> = req["cases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["case_id"].as_str())
        .collect();
    let key = if req["domain"] == "source_funds" {
        "assistant:evidence.sourceFundsContinuePrompt"
    } else {
        "assistant:evidence.continuePrompt"
    };
    let prompt = crate::i18n::tr(
        key,
        &[
            (
                "outcome",
                &t(&format!("assistant:evidence.outcome.{outcome}")),
            ),
            ("cases", &cases.join(", ")),
        ],
    );
    let context = (req["domain"] == "source_funds").then(|| {
        json!({"route": "/source-of-funds", "capabilities": ["source_funds"], "entity_type": "transaction", "entity_id": req["target_transaction"], "filters": {"source_funds_recipe": req["recipe"]}})
    });
    let base = app.assistant.messages.clone();
    let session = app.assistant.session_id.clone();
    chat::run_turn(
        app,
        &prompt,
        base,
        session,
        Some(EvidenceTurn {
            book: book_of(&req),
            context,
            attachment,
        }),
        None,
    );
}

/// Every frame: stamp new requests, open a requested picker, and follow the dialogs.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    let generation = app.assistant.generation;
    let ids: Vec<String> = app
        .assistant
        .messages
        .iter()
        .flat_map(|m| m.tool_calls.iter().filter_map(parse).map(rid))
        .collect();
    for id in ids {
        if let std::collections::hash_map::Entry::Vacant(e) =
            app.assistant.evidence.origins.entry(id)
        {
            e.insert(generation);
            changed = true;
        }
    }
    if let Some(id) = app.assistant.evidence.want_picker.take() {
        let req = app
            .assistant
            .evidence
            .pending
            .as_ref()
            .map(|p| p.request.clone())
            .unwrap_or(Value::Null);
        let mut options = rune::FileDialogOptions {
            title: "Exports, statements and images".into(),
            ..Default::default()
        };
        options = options.filter(
            "Exports, statements and images",
            &super::composer::EXTENSIONS,
        );
        tx.open_file(options, move |a: &mut App, result, _tx| match result {
            Ok(Some(path)) => stage(a, &id, &req, &path),
            _ => {
                if active(a, &id) {
                    a.assistant.evidence.pending = None;
                    set(a, &id, "idle", None);
                }
            }
        });
        changed = true;
    }
    let add_open = app.ov.add_open;
    let job = crate::wallets::add::submitting(app);
    let setup_failed = app.wallets.add.setup_error.is_some();
    if let Some((id, saw)) = app.assistant.evidence.dialog.clone() {
        if job && !saw {
            app.assistant.evidence.dialog = Some((id, true));
        } else if saw && !job && !setup_failed {
            complete(app, &id, "received", None);
            changed = true;
        } else if !add_open && !job {
            // Closed without a setup: back to where it was.
            if active(app, &id)
                && app
                    .assistant
                    .evidence
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.outcome.is_none())
            {
                app.assistant.evidence.pending = None;
                set(app, &id, "idle", None);
            }
            app.assistant.evidence.dialog = None;
            changed = true;
        }
    }
    if let Some((call, saw)) = app.assistant.evidence.wallet_dialog.clone() {
        if job && !saw {
            app.assistant.evidence.wallet_dialog = Some((call, true));
        } else if saw && !job && !setup_failed {
            app.assistant
                .evidence
                .wallet_cards
                .insert(call, "connected");
            app.assistant.evidence.wallet_dialog = None;
            changed = true;
        } else if !add_open && !job {
            if app.assistant.evidence.wallet_cards.get(&call) == Some(&"open") {
                app.assistant.evidence.wallet_cards.insert(call, "idle");
            }
            app.assistant.evidence.wallet_dialog = None;
            changed = true;
        }
    }
    changed
}

/// The picked file goes to the daemon's staging for the review case (the shell's own request;
/// the model and the conversation only ever get the token it returns).
fn stage(app: &mut App, id: &str, req: &Value, path: &std::path::Path) {
    if !active(app, id) {
        return;
    }
    let mut args = json!({
        "source_file": path.to_string_lossy(),
        "purpose": "chat_attachment",
        "expected_scope": {"workspace_id": req["workspace_id"], "profile_id": req["profile_id"]},
        "review_case_id": req["cases"][0]["case_id"],
    });
    if req["domain"] == "source_funds" {
        args["review_recipe"] = req["recipe"].clone();
        args["expected_review_fingerprint"] = req["review_fingerprint"].clone();
    }
    if let Some(r) = app.send(
        "internal.document_import.stage",
        args,
        "assistant.evidence_stage",
    ) {
        app.assistant.evidence.stage_ids.push((r, id.to_owned()));
    }
}

/// `EvidenceRequestCard`.
pub fn card(app: &App, tk: &Tk, c: &ToolCall) -> Option<El> {
    let d = parse(c)?.clone();
    let id = rid(&d);
    let action = d["action"].as_str().unwrap_or("").to_owned();
    let (st, err) = status(app, &id);
    let waiting = ["received", "partial", "unavailable"].contains(&st);
    let blocked = app.assistant.streaming
        || ["opening", "continuing", "stale"].contains(&st)
        || !app.prefs.ai_features;
    let other_active = app.assistant.evidence.states.iter().any(|(k, (v, _))| {
        *k != id && ["opening", "received", "partial", "unavailable"].contains(v)
    });
    let enabled = !(blocked || other_active || (waiting && app.assistant.streaming));
    let explanation = d["explanation"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t(&format!("assistant:evidence.why.{action}")));
    let key = format!("asst-evidence-{}", c.call_id);
    let label = if waiting {
        t("assistant:evidence.resume")
    } else {
        t(&format!("assistant:evidence.action.{action}"))
    };
    let d2 = d.clone();
    let mut act = button(
        tk,
        format!("{key}-act"),
        Variant::Outline,
        Size::Sm,
        None,
        &label,
        enabled,
    );
    if enabled {
        act = act.on_press(move |a: &mut App| {
            if waiting {
                resume(a, d2.clone(), false)
            } else {
                open(a, d2.clone())
            }
        });
    }
    let mut line = row()
        .mt(tk.r(0.75))
        .flex_wrap()
        .gap(tk.r(0.75))
        .center_y()
        .child(act);
    if !waiting && !["continuing", "stale"].contains(&st) {
        let can = !(blocked || other_active);
        let d3 = d.clone();
        let mut un = row()
            .key(format!("{key}-unavailable"))
            .role("button")
            .label(t("assistant:evidence.unavailable"))
            .child(
                text(t("assistant:evidence.unavailable"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
        if can {
            un = un
                .focusable(true)
                .on_press(move |a: &mut App| resume(a, d3.clone(), true));
        } else {
            un = un.disabled(true).opacity(0.5);
        }
        line = line.child(un);
    }
    let status_text = err.unwrap_or_else(|| t(&format!("assistant:evidence.status.{st}")));
    Some(
        col()
            .key(key)
            .w_full()
            .my(tk.r(0.75))
            .pl(tk.r(1.0))
            .border_sides([0.0, 0.0, 0.0, 2.0], tk.border)
            .role("region")
            .label(t("assistant:evidence.label"))
            .child(
                text(t(&format!("assistant:evidence.missing.{action}")))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            )
            .child(
                text(explanation)
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .mt(tk.r(0.25)),
            )
            .child(line)
            .child(
                text(status_text)
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .mt(tk.r(0.5))
                    .role("status"),
            ),
    )
}

/// The connection request card's action: the add-connection dialog on that source.
pub fn open_wallet_request(app: &mut App, call_id: &str, source: &str) {
    crate::wallets::add::open(app, Some(source));
    app.assistant
        .evidence
        .wallet_cards
        .insert(call_id.to_owned(), "open");
    app.assistant.evidence.wallet_dialog = Some((call_id.to_owned(), false));
}

pub fn wallet_card_state(app: &App, call_id: &str) -> &'static str {
    app.assistant
        .evidence
        .wallet_cards
        .get(call_id)
        .copied()
        .unwrap_or("idle")
}
