//! The conversation engine: `AssistantSessionProvider` + `useAiChatStream` on `app.assistant`.
//!
//! A turn is one streaming `ai.chat` request. Its progress records (`ai.chat.status`, `.delta`,
//! `.tool_call`, `.tool_result`, `.tool_consent_required`) update the running assistant message
//! through `stream::apply_record`; the terminal record ends it. Stop sends `ai.chat.cancel` for
//! the running request and drops its late records. Approvals answer with `ai.tool_call.consent`;
//! a permission-mode change while an answer runs goes out as `ai.chat.permission_mode`.
//!
//! Book binding (docs/reference/ai.md): every turn names the book it was asked in
//! (`expected_scope`); a conversation belongs to the book of its first turn, file or reopened
//! chat, and opening another book starts a fresh one. Queued questions, parked handoffs and pins
//! carry the book they were made in and never run in another.
//!
//! Every AI kind here is sent only from the user's own action (Enter, Send, Approve, Stop, a
//! permission change while an answer runs), and never while AI features are off.

use std::collections::HashMap;

use rune::TextEdit;
use serde_json::{Value, json};

use super::model::{AttachmentView, FileState, Msg, MsgStatus, Role};
use super::stream;
use super::think::ThinkParser;
use super::{Focus, PermissionMode};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc};

/// The open book's ids (`AssistantBookScope`): workspace, profile.
pub type Book = (String, String);

/// `bookNow().scope`: the open book, once its ids are known.
pub fn book_now(app: &App) -> Option<Book> {
    let p = app.read("ui.profiles.snapshot")?;
    let w = p.get("activeWorkspaceId")?.as_str()?.to_owned();
    let pr = p.get("activeProfileId")?.as_str()?.to_owned();
    Some((w, pr))
}

pub fn scope_json(b: &Book) -> Value {
    json!({"workspace_id": b.0, "profile_id": b.1})
}

/// `sameBook(made, now)`: an unknown open book (ids loading) does not count as another.
fn same_book(made: &Book, now: Option<&Book>) -> bool {
    now.is_none_or(|n| n == made)
}

/// The running turn (`useAiChatStream` refs).
pub struct Turn {
    pub request_id: String,
    pub assistant_id: String,
    pub user_id: String,
    pub parser: ThinkParser,
    pub observed: HashMap<String, (String, String)>,
    /// `persist === false` (an Incognito turn without a saved chat).
    pub no_persist: bool,
    pub provider: String,
    pub model: String,
    /// A record arrived: the daemon started it.
    pub started: bool,
    pub prompt: String,
    /// A page's handoff or an evidence continuation: its text is the page's, not the user's.
    pub generated: bool,
    /// Tokens this turn showed for the first time.
    pub new_files: Vec<String>,
}

/// `QueuedPrompt`: `pin` is the item the tray showed when it was submitted (pinned or open)
/// and `context` the whole screen context it showed (`tray::build`), or a handoff's page.
#[derive(Clone)]
pub struct Queued {
    pub prompt: String,
    pub pin: Option<Focus>,
    pub context: Value,
    pub handoff: bool,
    pub book: Book,
}

/// `ParkedHandoff`: a page's prompt waiting in the composer for a model.
#[derive(Clone, Debug)]
pub struct Parked {
    pub context: Option<Value>,
    pub book: Book,
}

/// What a submitted question was asked under (`asked`).
pub struct Asked {
    pub pin: Option<Focus>,
    pub context: Value,
    pub handoff: bool,
    pub book: Book,
}

/// An evidence continuation's own turn.
pub struct EvidenceTurn {
    pub book: Book,
    pub context: Option<Value>,
    pub attachment: Option<String>,
}

/// Requests whose reply the engine needs to match (replies carry no id; the one that just
/// finished is the one no longer pending).
#[derive(Default)]
pub struct InFlight {
    pub consents: Vec<(String, String, String, String)>,
    pub modes: Vec<(String, String)>,
}

/// The one of `ids` whose request is no longer pending: the reply being handled now.
fn take_finished<T>(app: &App, list: &mut Vec<(String, T)>) -> Option<T> {
    let i = list
        .iter()
        .position(|(id, _)| !app.pending.contains_key(id))?;
    Some(list.remove(i).1)
}

// ---- Readiness ---------------------------------------------------------------------------------

/// `effortSeeded`: the providers (and so the selected provider's stored default) are loaded.
pub fn effort_seeded(app: &App) -> bool {
    matches!(app.assistant.providers, crate::app::Load::Ready(_))
        || app.assistant.providers_failed()
}

/// `ready`: sends wait for the effort seed and the open book's ids.
pub fn ready(app: &App) -> bool {
    effort_seeded(app) && book_now(app).is_some()
}

/// `sendBlockedReason`.
pub fn send_blocked_reason(app: &App) -> Option<String> {
    if !effort_seeded(app) {
        return Some(t("assistant:composer.loadingProviderSettings"));
    }
    if book_now(app).is_none() {
        return Some(t("assistant:composer.loadingBook"));
    }
    app.assistant
        .files
        .iter()
        .any(|f| f.state == FileState::Uploading)
        .then(|| t("assistant:composer.attachmentUploading"))
}

pub fn has_model(app: &App) -> bool {
    app.assistant
        .selection
        .as_ref()
        .is_some_and(|(_, m)| !m.is_empty())
}

/// `canSubmit`.
pub fn can_submit(app: &App) -> bool {
    !app.assistant.draft.text.trim().is_empty()
        && has_model(app)
        && send_blocked_reason(app).is_none()
        && app.prefs.ai_features
}

/// The default permission mode for a new chat (Settings → AI).
pub fn default_mode(app: &App) -> PermissionMode {
    match app.settings.prefs.permission_mode {
        "approval" => PermissionMode::Approval,
        "full_access" => PermissionMode::FullAccess,
        _ => PermissionMode::Auto,
    }
}

// ---- Notices -----------------------------------------------------------------------------------

fn notice_book_switched(app: &mut App, returned: bool, cleared: bool) {
    let body = if returned && cleared {
        "assistant:queue.bookSwitched.bodyBoth"
    } else if cleared {
        "assistant:queue.bookSwitched.bodyCleared"
    } else {
        "assistant:queue.bookSwitched.bodyReturned"
    };
    app.toast(
        ToastTone::Warning,
        &t("assistant:queue.bookSwitched.title"),
        Some(t(body)),
        Some("assistant-book-switched"),
    );
}

fn entries(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_owned());
            }
            cur.clear();
        } else {
            if !cur.is_empty() {
                cur.push('\n');
            }
            cur.push_str(line);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

/// `draftHoldsPrompt`: the prompt is in the draft as whole entries.
fn draft_holds(draft: &str, prompt: &str) -> bool {
    let have = entries(draft);
    let want = entries(prompt);
    want.is_empty()
        || (0..have.len()).any(|s| {
            want.iter()
                .enumerate()
                .all(|(o, e)| have.get(s + o) == Some(e))
        })
}

/// `returnToDraft`: queued text that can no longer run goes back to the composer, once each.
pub fn return_to_draft(app: &mut App, prompts: &[String]) {
    let current = app.assistant.draft.text.trim().to_owned();
    let missing: Vec<&String> = prompts
        .iter()
        .filter(|p| !draft_holds(&current, p))
        .collect();
    if missing.is_empty() {
        return;
    }
    let mut parts: Vec<String> = vec![];
    if !current.is_empty() {
        parts.push(current);
    }
    parts.extend(missing.into_iter().cloned());
    replace_draft(app, &parts.join("\n\n"), false);
}

/// `replaceDraft`: new text in place of the draft (a parked handoff goes with it).
pub fn replace_draft(app: &mut App, text: &str, focus: bool) {
    let mut e = TextEdit::new(text);
    e.focused = focus || app.assistant.draft.focused;
    e.set_caret(text.len());
    app.assistant.draft = e;
    app.assistant.parked = None;
    app.assistant.recall = None;
}

fn refuse_in_other_book(app: &mut App, prompt: &str, generated: bool) {
    if !generated {
        return_to_draft(app, &[prompt.to_owned()]);
    }
    notice_book_switched(app, !generated, generated);
}

/// `refuseBookLoading`: not taken before the open book's ids are known.
pub fn refuse_book_loading(app: &mut App, prompt: Option<&str>) {
    if let Some(p) = prompt {
        return_to_draft(app, &[p.to_owned()]);
    }
    let body = if prompt.is_some() {
        "assistant:queue.bookLoading.bodyReturned"
    } else {
        "assistant:queue.bookLoading.body"
    };
    app.toast(
        ToastTone::Warning,
        &t("assistant:queue.bookLoading.title"),
        Some(t(body)),
        Some("assistant-book-loading"),
    );
}

// ---- Sending -----------------------------------------------------------------------------------

/// The composer's submit (Enter or the send button): the trimmed draft, cleared once taken.
pub fn submit(app: &mut App) {
    if !can_submit(app) {
        return;
    }
    let text = app.assistant.draft.text.trim().to_owned();
    if send_prompt(app, &text, None) {
        replace_draft(app, "", true);
    }
}

/// `sendPrompt`: sent, queued or parked (true), or refused (false, the text stays).
pub fn send_prompt(app: &mut App, prompt: &str, explicit: Option<Parked>) -> bool {
    let trimmed = prompt.trim().to_owned();
    if trimmed.is_empty() || !app.prefs.ai_features {
        return false;
    }
    let now = book_now(app);
    if let Some(h) = &explicit
        && !same_book(&h.book, now.as_ref())
    {
        notice_book_switched(app, false, true);
        return false;
    }
    if !has_model(app) {
        // No model yet: a page's request waits in the composer, without the pin.
        let Some(h) = explicit else { return false };
        app.assistant.focus = None;
        let page = h
            .context
            .clone()
            .or_else(|| super::screen::current_page(app));
        app.assistant.draft = TextEdit::new(prompt);
        app.assistant.parked = page.map(|context| Parked {
            context: Some(context),
            book: h.book,
        });
        return true;
    }
    let mut handoff = explicit;
    if handoff.is_none()
        && let Some(parked) = app.assistant.parked.take()
    {
        if !same_book(&parked.book, now.as_ref()) {
            replace_draft(app, "", false);
            notice_book_switched(app, false, true);
            return false;
        }
        handoff = Some(parked);
    }
    let book = match &handoff {
        Some(h) => h.book.clone(),
        None => match now.clone() {
            Some(b) => b,
            None => {
                refuse_book_loading(app, Some(&trimmed));
                return false;
            }
        },
    };
    app.assistant.prompt_revision += 1;
    // What the tray shows now is what this question shares, however long it waits in the queue.
    let (pin, context) = match &handoff {
        Some(h) => (
            None,
            h.context
                .clone()
                .unwrap_or_else(|| super::screen::current(app)),
        ),
        None => {
            let shared = super::tray::build(app);
            (shared.item, shared.screen)
        }
    };
    let item = Queued {
        prompt: trimmed,
        pin,
        context,
        handoff: handoff.is_some(),
        book,
    };
    if handoff.is_some() {
        app.assistant.focus = None;
    }
    if app.assistant.streaming || !ready(app) {
        app.assistant.queued.push(item);
        return true;
    }
    dispatch(app, item)
}

fn dispatch(app: &mut App, item: Queued) -> bool {
    let base = app.assistant.messages.clone();
    let session = app.assistant.session_id.clone();
    run_turn(
        app,
        &item.prompt,
        base,
        session,
        None,
        Some(Asked {
            pin: item.pin,
            context: item.context,
            handoff: item.handoff,
            book: item.book,
        }),
    )
}

/// The pin to send now: none once the open book is another.
fn live_pin(app: &App, pin: Option<&Focus>) -> Option<Focus> {
    let now = book_now(app);
    pin.filter(|p| same_book(&p.book, now.as_ref())).cloned()
}

/// A turn's screen context: a page request's or evidence continuation's own (`page`), else what
/// the tray showed when the question was asked (`asked`: the page with the item's entity).
/// `item` says whether that item is still of the open book (`None`: no item). The flag says the
/// item is this turn's context, the only case in which the daemon reads it for the model
/// (`read_pinned_entity`); an item of another book is dropped, never sent.
fn turn_context(page: Option<Value>, asked: Value, item: Option<bool>) -> (Value, bool) {
    match (page, item) {
        (Some(page), _) => (page, false),
        (None, Some(true)) => (asked, true),
        (None, Some(false)) => {
            let mut c = asked;
            if let Some(m) = c.as_object_mut() {
                m.remove("entity_type");
                m.remove("entity_id");
            }
            (c, false)
        }
        (None, None) => (asked, false),
    }
}

/// `runTurn`: one chat turn against an explicit conversation base and saved chat.
pub fn run_turn(
    app: &mut App,
    prompt: &str,
    base: Vec<Msg>,
    session: Option<String>,
    evidence: Option<EvidenceTurn>,
    asked: Option<Asked>,
) -> bool {
    if !app.prefs.ai_features || app.assistant.streaming {
        return false;
    }
    let Some((provider, model)) = app
        .assistant
        .selection
        .clone()
        .filter(|(_, m)| !m.is_empty())
    else {
        return false;
    };
    let generated = evidence.is_some() || asked.as_ref().is_some_and(|a| a.handoff);
    let turn_book = match (&evidence, &asked) {
        (Some(e), _) => Some(e.book.clone()),
        (None, Some(a)) => Some(a.book.clone()),
        (None, None) => book_now(app),
    };
    let Some(turn_book) = turn_book else {
        refuse_book_loading(app, (!generated).then_some(prompt));
        return false;
    };
    let base: Vec<Msg> = base
        .into_iter()
        .filter(|m| m.role != Role::System)
        .collect();
    let mut sent: Vec<(String, AttachmentView, Option<Book>)> = app
        .assistant
        .files
        .iter()
        .filter(|f| f.state != FileState::Uploading)
        .map(|f| (f.key.clone(), f.view.clone(), f.book.clone()))
        .collect();
    if let Some(tok) = evidence.as_ref().and_then(|e| e.attachment.clone())
        && !sent.iter().any(|(k, _, _)| *k == tok)
    {
        sent.push((tok, AttachmentView::default(), Some(turn_book.clone())));
    }
    let bound = app.assistant.book.clone();
    let carried = !base.is_empty() || session.is_some() || !sent.is_empty();
    let now = book_now(app);
    if !same_book(&turn_book, now.as_ref())
        || (carried && bound.as_ref() != Some(&turn_book))
        || sent
            .iter()
            .any(|(_, _, b)| b.as_ref().is_some_and(|b| *b != turn_book))
    {
        refuse_in_other_book(app, prompt, generated);
        return false;
    }
    if bound.is_none() {
        app.assistant.book = Some(turn_book.clone());
    }
    app.assistant.turn_revision = app.assistant.prompt_revision;
    app.assistant.turn_generation = app.assistant.generation;
    let mut messages: Vec<Value> = base
        .iter()
        .map(|m| json!({"role": role_str(m.role), "content": m.content}))
        .collect();
    messages.push(json!({"role": "user", "content": prompt}));
    let seed = app.assistant.seed_pending && session.is_none();
    app.assistant.seed_pending = false;
    let seed_notices: Vec<Value> = if seed {
        base.iter()
            .enumerate()
            .filter(|(_, m)| m.role == Role::Assistant && !m.notices.is_empty())
            .map(|(i, m)| json!({"index": i, "notices": m.notices}))
            .collect()
    } else {
        vec![]
    };
    let new_files: Vec<(String, AttachmentView)> = sent
        .iter()
        .filter(|(k, _, _)| !app.assistant.shown.contains(k))
        .map(|(k, v, _)| (k.clone(), v.clone()))
        .collect();
    for (k, _) in &new_files {
        app.assistant.shown.insert(k.clone());
    }
    // A file a sent message already carried stays with the chat, marked "In this chat".
    for f in app
        .assistant
        .files
        .iter_mut()
        .filter(|f| new_files.iter().any(|(k, _)| *k == f.key))
    {
        f.state = FileState::Sent;
    }
    let effort = app.assistant.effort.clone();
    let fast = app.assistant.fast_mode && super::picker::fast_supported(app);
    let options = if effort == "auto" && !fast {
        Value::Null
    } else {
        let mut o = serde_json::Map::new();
        if effort != "auto" {
            o.insert("reasoning_effort".into(), json!(effort));
        }
        if fast {
            o.insert("fast_mode".into(), json!(true));
        }
        Value::Object(o)
    };
    let tool_profile = app
        .settings
        .prefs
        .tool_profiles
        .get(&provider)
        .cloned()
        .unwrap_or_else(|| "full".into());
    // A turn without a submitted question (an evidence continuation, an edited message) shares
    // what the tray shows now.
    let (asked_ctx, asked_item) = match &asked {
        Some(a) => (a.context.clone(), a.pin.clone()),
        None => {
            let shared = super::tray::build(app);
            (shared.screen, shared.item)
        }
    };
    let item_live = asked_item
        .as_ref()
        .map(|p| live_pin(app, Some(p)).is_some());
    let page_ctx = evidence
        .as_ref()
        .and_then(|e| e.context.clone())
        .or_else(|| {
            asked
                .as_ref()
                .filter(|a| a.handoff)
                .map(|a| a.context.clone())
        });
    let (screen, read_pin) = turn_context(page_ctx, asked_ctx, item_live);
    let no_persist = app.assistant.incognito && session.is_none();
    let mut args = json!({
        "provider": provider,
        "model": model,
        "messages": messages,
        "tools_enabled": true,
        "tool_profile": tool_profile,
        "system_prompt_kind": "kassiber",
        "expected_scope": scope_json(&turn_book),
        "permission_mode": app.assistant.permission.id(),
        "persist": if no_persist { json!(false) } else { json!("auto") },
        "screen_context": screen,
    });
    if read_pin {
        args["read_pinned_entity"] = json!(true);
    }
    if !options.is_null() {
        args["options"] = options;
    }
    if let Some(s) = &session {
        args["session_id"] = json!(s);
    }
    if seed {
        args["seed_history"] = json!(true);
        if !seed_notices.is_empty() {
            args["seed_notices"] = json!(seed_notices);
        }
    }
    if !sent.is_empty() {
        // Tokens only, never a path; `new` marks the files this message first sends.
        let list: Vec<Value> = sent
            .iter()
            .map(|(k, _, _)| {
                if new_files.iter().any(|(n, _)| n == k) {
                    json!({"token": k, "new": true})
                } else {
                    json!({"token": k})
                }
            })
            .collect();
        args["attachments"] = json!(list);
    }
    // `send`: the user's message and the pending answer, then the stream.
    let user_id = stream::make_id("user");
    let assistant_id = stream::make_id("answer");
    let mut user = Msg::new(user_id.clone(), Role::User, prompt);
    if evidence.is_none() {
        user.attachments = new_files.iter().map(|(_, v)| v.clone()).collect();
    }
    let mut answer = Msg::new(assistant_id.clone(), Role::Assistant, "");
    answer.status = MsgStatus::Pending;
    answer.activity_label = Some("Preparing chat".into());
    let a = &mut app.assistant;
    a.error = None;
    a.pending_consents.clear();
    a.messages.push(user);
    a.messages.push(answer);
    a.synthetic = false;
    let Some(request_id) = app.send("ai.chat", args, "assistant.chat") else {
        // No daemon: the turn ends at once with the transport's error.
        fail_answer(
            app,
            &assistant_id,
            "stream_failed",
            &t("assistant:message.chatFailed"),
        );
        return false;
    };
    // A streaming answer is not a foreground read: it does not hold the route progress line.
    app.busy.reads = app.busy.reads.saturating_sub(1);
    app.assistant.streaming = true;
    app.assistant.turn = Some(Turn {
        request_id,
        assistant_id,
        user_id,
        parser: ThinkParser::default(),
        observed: HashMap::new(),
        no_persist,
        provider,
        model,
        started: false,
        prompt: prompt.to_owned(),
        generated,
        new_files: new_files.into_iter().map(|(k, _)| k).collect(),
    });
    app.assistant.thread.anchor_new_turn = true;
    true
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
    }
}

fn answer_mut<'a>(app: &'a mut App, id: &str) -> Option<&'a mut Msg> {
    app.assistant.messages.iter_mut().find(|m| m.id == id)
}

fn fail_answer(app: &mut App, id: &str, code: &str, message: &str) {
    app.assistant.error = Some(message.to_owned());
    if let Some(m) = answer_mut(app, id) {
        m.status = MsgStatus::Error;
        m.error_code = Some(code.to_owned());
        m.error_message = Some(message.to_owned());
    }
}

// ---- The stream --------------------------------------------------------------------------------

/// A progress record of a running request (`onRecord`).
pub fn on_progress(app: &mut App, tag: &str, envelope: &Value) {
    if tag != "assistant.chat" {
        return;
    }
    let rid = envelope["request_id"].as_str().unwrap_or("");
    let Some(turn) = app.assistant.turn.as_mut().filter(|t| t.request_id == rid) else {
        return;
    };
    turn.started = true;
    if let Some(kind) = stream::completed_mutation(&mut turn.observed, envelope) {
        on_mutation(app, &kind);
    }
    if envelope["kind"].as_str() == Some("ai.chat.tool_consent_required")
        && let Some(req) = stream::consent_request(&envelope["data"])
    {
        let list = &mut app.assistant.pending_consents;
        list.retain(|c| c.call_id != req.call_id);
        list.push(req);
    }
    if let Some(id) = stream::settled_call(envelope) {
        app.assistant.pending_consents.retain(|c| c.call_id != id);
    }
    let Some(turn) = app.assistant.turn.as_mut() else {
        return;
    };
    let id = turn.assistant_id.clone();
    let mut parser = std::mem::take(&mut turn.parser);
    if let Some(m) = answer_mut(app, &id) {
        stream::apply_record(m, envelope, &mut parser, false);
    }
    if let Some(turn) = app.assistant.turn.as_mut() {
        turn.parser = parser;
    }
}

/// `invalidateDaemonQueriesForMutation`: a change the assistant made shows on every screen.
fn on_mutation(app: &mut App, _kind: &str) {
    app.refresh_reads();
}

/// The terminal `ai.chat` record.
fn on_chat_reply(app: &mut App, r: &Reply) {
    // The send took this one back off the foreground count; `handle` just took it again.
    app.busy.reads += 1;
    let Some(turn) = app.assistant.turn.as_ref() else {
        return;
    };
    // Another request's late terminal (a stopped turn): this turn is still pending.
    if app.pending.contains_key(&turn.request_id) {
        return;
    }
    let turn = app.assistant.turn.take().expect("turn");
    app.assistant.streaming = false;
    app.assistant.pending_consents.clear();
    let mut parser = turn.parser;
    if let Some(e) = &r.error {
        let code = r.error_detail["code"]
            .as_str()
            .unwrap_or("unknown_error")
            .to_owned();
        let refused = code == "stale_context" && !turn.started;
        if refused {
            // Nothing ran: drop the turn it never had.
            app.assistant
                .messages
                .retain(|m| m.id != turn.user_id && m.id != turn.assistant_id);
            for k in &turn.new_files {
                app.assistant.shown.remove(k);
                for f in app.assistant.files.iter_mut().filter(|f| f.key == *k) {
                    f.state = FileState::Ready;
                }
            }
            refuse_in_other_book(app, &turn.prompt, turn.generated);
            return;
        }
        if code == "not_found" {
            app.assistant.session_id = None;
        }
        fail_answer(app, &turn.assistant_id, &code, e);
        return;
    }
    let d = &r.data;
    let finish = d["finish_reason"].as_str().map(str::to_owned);
    let tail = parser.flush();
    if !turn.no_persist {
        app.assistant.session_id = d["session_id"].as_str().map(str::to_owned);
    }
    if let Some(m) = answer_mut(app, &turn.assistant_id) {
        m.content.push_str(&tail.content);
        stream::append_thinking(m, &tail.thinking);
        m.status = stream::terminal_status(finish.as_deref(), false);
        m.finish_reason = finish;
        m.provider = Some(
            d["provider"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or(turn.provider),
        );
        m.model = Some(d["model"].as_str().map(str::to_owned).unwrap_or(turn.model));
        m.provenance = (!d["provenance"].is_null()).then(|| d["provenance"].clone());
        for n in stream::notices(&d["provenance"]["notices"]) {
            if !m.notices.contains(&n) {
                m.notices.push(n);
            }
        }
    }
    // Files whose staging grant ended: the turn went on without them.
    let expired: Vec<String> = d["expired_attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    if !expired.is_empty() {
        let names: Vec<String> = app
            .assistant
            .files
            .iter()
            .filter(|f| expired.contains(&f.key))
            .map(|f| f.view.filename.clone())
            .collect();
        app.assistant.files.retain(|f| !expired.contains(&f.key));
        let n = expired.len();
        app.toast(
            ToastTone::Warning,
            &tc("assistant:composer.attachmentExpired.title", n, &[]),
            Some(tc(
                "assistant:composer.attachmentExpired.body",
                n,
                &[("filenames", &names.join(", "))],
            )),
            Some("assistant-attachment-expired"),
        );
    }
}

/// `cancelActiveRequest`: `ai.chat.cancel` for the running request; late records are dropped.
fn cancel_active(app: &mut App) {
    let Some(turn) = app.assistant.turn.take() else {
        return;
    };
    app.assistant.streaming = false;
    let _ = app.send(
        "ai.chat.cancel",
        json!({"target_request_id": turn.request_id}),
        "assistant.cancel",
    );
    app.assistant.cancelled_turn = Some(turn.assistant_id);
}

/// Stop (`abort`): the running answer ends here, marked stopped by the user.
pub fn abort(app: &mut App) {
    app.assistant.draft.focused = true;
    cancel_active(app);
    app.assistant.pending_consents.clear();
    if let Some(id) = app.assistant.cancelled_turn.take()
        && let Some(m) = answer_mut(app, &id)
    {
        m.status = MsgStatus::Cancelled;
    }
}

/// `reset()` of the stream: the running turn is cancelled and detached; an empty conversation.
fn reset_stream(app: &mut App) {
    cancel_active(app);
    app.assistant.cancelled_turn = None;
    let a = &mut app.assistant;
    a.messages.clear();
    a.streaming = false;
    a.error = None;
    a.pending_consents.clear();
    a.session_id = None;
    a.synthetic = false;
    // A fresh thread, keeping the column widths already laid out.
    a.thread = super::thread::State {
        width: a.thread.width,
        ..Default::default()
    };
}

/// `loadConversation`: the thread shows `messages`, bound to `session`.
pub fn load_messages(app: &mut App, messages: Vec<Msg>, session: Option<String>) {
    if app.assistant.turn.is_some() {
        return;
    }
    let a = &mut app.assistant;
    a.messages = messages;
    a.streaming = false;
    a.error = None;
    a.pending_consents.clear();
    a.session_id = session;
    a.synthetic = false;
    // A fresh thread, keeping the column widths already laid out.
    a.thread = super::thread::State {
        width: a.thread.width,
        ..Default::default()
    };
}

fn clear_evidence(app: &mut App) {
    app.assistant.generation += 1;
    super::evidence::clear(app);
}

/// `startFreshConversation`: the Incognito choice stays; the mode returns to the default.
pub fn start_fresh(app: &mut App) {
    clear_evidence(app);
    app.assistant.seed_pending = false;
    app.assistant.files.clear();
    app.assistant.uploads.clear();
    app.assistant.upload_busy = false;
    app.assistant.shown.clear();
    reset_stream(app);
    app.assistant.permission = default_mode(app);
    app.assistant.book = None;
    app.assistant.recent_requested = false;
}

/// `clearChat` (Clear chat, New chat).
pub fn clear_chat(app: &mut App) {
    app.assistant.menu = None;
    app.assistant.queued.clear();
    app.assistant.focus = None;
    start_fresh(app);
}

// ---- Approvals and the permission mode ---------------------------------------------------------

/// `sendConsent`: answered once; the row leaves at once.
pub fn send_consent(app: &mut App, call_id: &str, decision: &str) {
    if !app.prefs.ai_features {
        return;
    }
    // A SYNTHETIC scene has no running request to answer: its rows take the answer as a
    // recorded one would, so the scene shows what follows; nothing is sent.
    if app.assistant.synthetic
        && app.assistant.turn.is_none()
        && app
            .assistant
            .pending_consents
            .iter()
            .any(|c| c.call_id == call_id)
    {
        app.assistant
            .pending_consents
            .retain(|c| c.call_id != call_id);
        for m in app.assistant.messages.iter_mut() {
            stream::apply_consent_response(m, call_id, decision, true, None);
        }
        if app.assistant.pending_consents.is_empty() {
            app.assistant.draft.focused = true;
        }
        return;
    }
    let Some(target) = app.assistant.turn.as_ref().map(|t| t.request_id.clone()) else {
        return;
    };
    if !app
        .assistant
        .pending_consents
        .iter()
        .any(|c| c.call_id == call_id)
    {
        return;
    }
    app.assistant
        .pending_consents
        .retain(|c| c.call_id != call_id);
    // The row leaves; focus moves on to the next approval's row, else the composer.
    if app.assistant.pending_consents.is_empty() {
        app.assistant.draft.focused = true;
    }
    let args = json!({"target_request_id": target, "call_id": call_id, "decision": decision});
    if let Some(id) = app.send("ai.tool_call.consent", args, "assistant.consent") {
        app.assistant.in_flight.consents.push((
            id,
            call_id.to_owned(),
            decision.to_owned(),
            target,
        ));
    }
}

fn on_consent_reply(app: &mut App, r: &Reply) {
    let mut list: Vec<(String, (String, String, String))> = app
        .assistant
        .in_flight
        .consents
        .drain(..)
        .map(|(id, c, d, t)| (id, (c, d, t)))
        .collect();
    let found = take_finished(app, &mut list);
    app.assistant.in_flight.consents = list
        .into_iter()
        .map(|(id, (c, d, t))| (id, c, d, t))
        .collect();
    let Some((call_id, decision, target)) = found else {
        return;
    };
    // Answered for a turn that has since ended or been left.
    let Some(turn) = app
        .assistant
        .turn
        .as_ref()
        .filter(|t| t.request_id == target)
    else {
        return;
    };
    let aid = turn.assistant_id.clone();
    if let Some(e) = &r.error {
        let code = r.error_detail["code"]
            .as_str()
            .unwrap_or("consent_failed")
            .to_owned();
        app.assistant.error = Some(e.clone());
        if let Some(m) = answer_mut(app, &aid) {
            stream::apply_consent_response(m, &call_id, &decision, false, Some(&code));
        }
        return;
    }
    if r.data["recorded"].as_bool() == Some(false) {
        let reason = r.data["reason"].as_str().unwrap_or("not_found").to_owned();
        app.assistant.error = Some(format!("Could not record tool consent: {reason}"));
        if let Some(m) = answer_mut(app, &aid) {
            stream::apply_consent_response(m, &call_id, &decision, false, Some(&reason));
        }
        return;
    }
    if let Some(m) = answer_mut(app, &aid) {
        stream::apply_consent_response(m, &call_id, &decision, true, None);
    }
}

/// `choosePermissionMode`: a running answer switches too; otherwise the next turn names it.
pub fn set_permission_mode(app: &mut App, mode: PermissionMode) {
    if mode == app.assistant.permission {
        return;
    }
    app.assistant.permission = mode;
    if !app.prefs.ai_features {
        return;
    }
    let Some(target) = app.assistant.turn.as_ref().map(|t| t.request_id.clone()) else {
        return;
    };
    let args = json!({"target_request_id": target, "permission_mode": mode.id()});
    if let Some(id) = app.send("ai.chat.permission_mode", args, "assistant.permission") {
        app.assistant.in_flight.modes.push((id, target));
    }
}

fn on_mode_reply(app: &mut App, r: &Reply) {
    let mut list = std::mem::take(&mut app.assistant.in_flight.modes);
    let found = take_finished(app, &mut list);
    app.assistant.in_flight.modes = list;
    let Some(target) = found else { return };
    if app
        .assistant
        .turn
        .as_ref()
        .is_none_or(|t| t.request_id != target)
    {
        return;
    }
    if r.error.is_some() {
        app.toast(
            ToastTone::Warning,
            &t("assistant:composer.permission.notApplied.title"),
            Some(t("assistant:composer.permission.notApplied.body")),
            Some("assistant-permission-mode-not-applied"),
        );
        return;
    }
    let released: Vec<String> = r.data["released_call_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    app.assistant
        .pending_consents
        .retain(|c| !released.contains(&c.call_id));
}

// ---- Branch and edit ---------------------------------------------------------------------------

/// The user and assistant turns with text, as a saved chat keeps them.
fn kept(messages: &[Msg]) -> Vec<Msg> {
    messages
        .iter()
        .filter(|m| matches!(m.role, Role::User | Role::Assistant) && !m.content.is_empty())
        .map(|m| {
            let mut n = Msg::new(stream::make_id("msg"), m.role, m.content.clone());
            n.notices = m.notices.clone();
            n
        })
        .collect()
}

/// `branchFromMessage`: a fresh, unsaved conversation up to and including `id`.
pub fn branch(app: &mut App, id: &str) {
    if app.assistant.streaming {
        return;
    }
    let Some(i) = app.assistant.messages.iter().position(|m| m.id == id) else {
        return;
    };
    let entries = kept(&app.assistant.messages[..=i]);
    if entries.is_empty() {
        return;
    }
    clear_evidence(app);
    app.assistant.queued.clear();
    app.assistant.seed_pending = true;
    load_messages(app, entries, None);
}

/// `editUserMessage(id, text)`: rewind to just before the prompt and regenerate from the edit,
/// as a new unsaved chat (the original stays in History).
pub fn edit(app: &mut App, id: &str, next: &str) {
    if app.assistant.streaming {
        return;
    }
    let Some(i) = app.assistant.messages.iter().position(|m| m.id == id) else {
        return;
    };
    if app.assistant.messages[i].role != Role::User {
        return;
    }
    let trimmed = next.trim().to_owned();
    if trimmed.is_empty() || !has_model(app) {
        return;
    }
    clear_evidence(app);
    let prior = kept(&app.assistant.messages[..i]);
    app.assistant.queued.clear();
    app.assistant.seed_pending = true;
    app.assistant.thread.editing = None;
    load_messages(app, prior.clone(), None);
    if !ready(app) {
        let Some(book) = book_now(app) else {
            refuse_book_loading(app, Some(&trimmed));
            return;
        };
        let shared = super::tray::build(app);
        let item = Queued {
            prompt: trimmed,
            pin: shared.item,
            context: shared.screen,
            handoff: false,
            book,
        };
        app.assistant.queued = vec![item];
        return;
    }
    run_turn(app, &trimmed, prior, None, None, None);
}

// ---- Saved chats -------------------------------------------------------------------------------

/// `resumeSession` after its read: a fresh conversation bound to the saved chat and its book.
pub fn resumed(app: &mut App, messages: Vec<Msg>, id: Option<String>) {
    // Another book opened (or the chat was cleared) meanwhile.
    let now = book_now(app);
    let Some(book) = app.assistant.resume_book.take() else {
        return;
    };
    if app.assistant.generation != app.assistant.resume_generation
        || !same_book(&book, now.as_ref())
    {
        return;
    }
    let a = &mut app.assistant;
    a.queued.clear();
    a.incognito = false;
    a.files.clear();
    a.shown.clear();
    a.focus = None;
    a.seed_pending = false;
    replace_draft(app, "", false);
    let mode = id
        .as_ref()
        .and_then(|i| app.assistant.chat_modes.get(i).copied())
        .unwrap_or_else(|| default_mode(app));
    app.assistant.permission = mode;
    load_messages(app, messages, id);
    app.assistant.book = Some(book);
}

// ---- Every frame -------------------------------------------------------------------------------

/// The provider's effort default seeds the session when it is selected; a level the user picked
/// stays theirs (`useProviderDefaultEffort`, without per-model nearest-level fitting).
fn seed_effort(app: &mut App) -> bool {
    let Some((provider, _)) = app.assistant.selection.clone() else {
        return false;
    };
    let book = book_now(app);
    let key = (provider.clone(), book);
    if app.assistant.effort_seed_for.as_ref() == Some(&key) {
        return false;
    }
    let Some(list) = app.assistant.providers.ready() else {
        return false;
    };
    let default = list.iter().find(|p| p.name == provider).and_then(|p| {
        p.raw["default_reasoning_effort"]
            .as_str()
            .map(str::to_owned)
    });
    let book_changed = app
        .assistant
        .effort_seed_for
        .as_ref()
        .is_some_and(|(_, b)| *b != key.1);
    if book_changed {
        app.assistant.effort_chosen = false;
    }
    if !app.assistant.effort_chosen || app.assistant.effort == "auto" {
        app.assistant.effort = default.unwrap_or_else(|| "auto".into());
        app.assistant.effort_chosen = false;
    }
    app.assistant.effort_seed_for = Some(key);
    true
}

/// Queue, book binding, the kill switch and the remembered screen. True when anything changed.
pub fn poll(app: &mut App) -> bool {
    let mut changed = false;
    // AI features off unmounts the session: the running answer stops and the chat is gone.
    if !app.prefs.ai_features {
        if app.assistant.turn.is_some()
            || !app.assistant.messages.is_empty()
            || !app.assistant.queued.is_empty()
            || !app.assistant.files.is_empty()
        {
            app.assistant.queued.clear();
            app.assistant.focus = None;
            app.assistant.incognito = false;
            start_fresh(app);
            changed = true;
        }
        return changed;
    }
    // The page the assistant answers about, while the user is somewhere else.
    if app.route.path() != "/assistant" {
        let (path, search) = super::screen::current_location(app);
        let ctx = super::screen::context_for(&path, &search);
        if app.assistant.last_screen.as_ref() != Some(&ctx) {
            app.assistant.last_screen = Some(ctx);
        }
    }
    if let Some(id) = app.assistant.session_id.clone() {
        let mode = app.assistant.permission;
        if app.assistant.chat_modes.get(&id) != Some(&mode) {
            app.assistant.chat_modes.insert(id, mode);
        }
    }
    changed |= seed_effort(app);
    changed |= super::tray::poll(app);
    let now = book_now(app);
    // A conversation belongs to the book it started in.
    if let (Some(bound), Some(n)) = (app.assistant.book.clone(), now.as_ref())
        && bound != *n
    {
        let had_file = !app.assistant.files.is_empty();
        start_fresh(app);
        if had_file {
            app.toast(
                ToastTone::Warning,
                &t("assistant:queue.bookSwitched.title"),
                Some(t("assistant:queue.bookSwitched.bodyAttachment")),
                Some("assistant-book-switched-attachment"),
            );
        }
        changed = true;
    }
    if let Some(n) = now.as_ref() {
        if app.assistant.focus.as_ref().is_some_and(|f| f.book != *n) {
            app.assistant.focus = None;
            changed = true;
        }
        if app.assistant.parked.as_ref().is_some_and(|p| p.book != *n) {
            replace_draft(app, "", false);
            notice_book_switched(app, false, true);
            changed = true;
        }
    }
    // The queue sends its next question once the answer ended and the book is ready.
    if !app.assistant.streaming && !app.assistant.queued.is_empty() && ready(app) && has_model(app)
    {
        let stale: Vec<Queued> = app
            .assistant
            .queued
            .iter()
            .filter(|q| !same_book(&q.book, now.as_ref()))
            .cloned()
            .collect();
        if !stale.is_empty() {
            app.assistant
                .queued
                .retain(|q| same_book(&q.book, now.as_ref()));
            let own: Vec<String> = stale
                .iter()
                .filter(|q| !q.handoff)
                .map(|q| q.prompt.clone())
                .collect();
            return_to_draft(app, &own);
            notice_book_switched(app, !own.is_empty(), own.len() < stale.len());
        } else {
            let next = app.assistant.queued.remove(0);
            dispatch(app, next);
        }
        changed = true;
    }
    changed
}

/// This module's replies.
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "assistant.chat" => on_chat_reply(app, r),
        "assistant.consent" => on_consent_reply(app, r),
        "assistant.permission" => on_mode_reply(app, r),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returned_prompts_are_added_once_as_whole_entries() {
        assert!(draft_holds("a\n\nb", "b"));
        assert!(!draft_holds("ab", "b"));
        assert!(draft_holds("x\n\nfirst\nline\n\ny", "first\nline"));
        assert_eq!(
            entries("one\n \n\ntwo\n"),
            vec!["one".to_owned(), "two".to_owned()]
        );
    }

    #[test]
    fn only_an_item_that_is_the_turns_context_is_read() {
        let page = json!({"route": "/review"});
        let asked = json!({"route": "/overview", "entity_type": "transaction", "entity_id": "t"});
        assert_eq!(
            turn_context(None, asked.clone(), Some(true)),
            (asked.clone(), true)
        );
        // A page request or evidence continuation brings its own context: no pinned read.
        assert_eq!(
            turn_context(Some(page.clone()), asked.clone(), Some(true)),
            (page, false)
        );
        // An item of another book is not sent.
        assert_eq!(
            turn_context(None, asked.clone(), Some(false)),
            (json!({"route": "/overview"}), false)
        );
        let plain = json!({"route": "/overview"});
        assert_eq!(turn_context(None, plain.clone(), None), (plain, false));
    }

    #[test]
    fn an_unknown_open_book_is_not_another() {
        let b = ("w".to_owned(), "p".to_owned());
        assert!(same_book(&b, None));
        assert!(same_book(&b, Some(&b)));
        assert!(!same_book(&b, Some(&("w".to_owned(), "q".to_owned()))));
    }
}
