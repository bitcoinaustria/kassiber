//! The composer (components/ai-02.tsx): the frosted `kb-composer-glass` box with the chat's
//! files, the auto-growing text field, the toolbar (attach, model picker, send) and the starter
//! suggestions; plus the file card (ChatAttachmentCard.tsx) and the "About" focus chip
//! (AssistantFocusChip.tsx).
//!
//! Return or the send button sends (`sendPrompt`); while an answer streams the same button
//! queues the question and a Stop button ends the answer. Shift+Return breaks the line.
//!
//! Attachments and the staging-token rule: the web's renderer never names a file path (the
//! Tauri shell stages a picked or dropped file and hands the page an opaque token). Rune is one
//! native process with no untrusted renderer, and goes further: the file the user picked or
//! dropped is read here and its bytes are handed to the daemon in chunks
//! (`ui.chat.attachments.upload`, the web's path for pasted files), which keeps them in a private
//! temporary file and returns the staging token. The daemon and the model only ever see that
//! token; no path leaves this process, and nothing leaves the device.

use rune::{Color, Cx, El, Icon, Key, KeyEvent, Shadow, TextEdit, Tx, col, row, text, text_input};
use serde_json::json;

use super::model::{AttachmentView, ChatFile, FileState, Role};
use super::{chat, picker};
use crate::app::{App, ToastTone};
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// T3 Code's stop mark (ai-02.tsx `StopIcon`).
static STOP_MARK: Icon = Icon {
    name: "kb-stop-mark",
    view: 12.0,
    body: r##"<rect x="2" y="2" width="8" height="8" rx="1.5" fill="currentColor"/>"##,
};

/// T3 Code's send arrow (ai-02.tsx `SendArrowIcon`), a touch lighter than lucide's ArrowUp.
static SEND_ARROW: Icon = Icon {
    name: "kb-send-arrow",
    view: 14.0,
    body: r##"<path d="M7 11.5V2.5M7 2.5L3 6.5M7 2.5L11 6.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>"##,
};

/// `CHAT_ATTACHMENT_EXTENSIONS`.
pub const EXTENSIONS: [&str; 10] = [
    "csv", "tsv", "xlsx", "xlsm", "pdf", "png", "jpg", "jpeg", "webp", "gif",
];
/// `MAX_CHAT_ATTACHMENTS`.
pub const MAX_FILES: usize = 8;
/// `MAX_CHAT_UPLOAD_BYTES`.
const MAX_UPLOAD: usize = 25 * 1024 * 1024;
/// Below the daemon's request line limit once base64-encoded.
const CHUNK: usize = 512 * 1024;

/// `DEFAULT_COMPOSER_PROMPT_KEYS`.
pub const PROMPTS: [(&Icon, &str); 3] = [
    (&I::TRIANGLE_ALERT, "reviewQuarantine"),
    (&I::REFRESH_CW, "reprocessJournals"),
    (&I::FILE_SPREADSHEET, "prepareTaxExport"),
];

/// How a host shows the composer.
#[derive(Clone, Copy)]
pub struct Opts {
    /// `"page"` or `"panel"`: keys the field and the picker.
    pub host: &'static str,
    /// The empty page's hero: a taller box (`min-h-[96px]`) and `gap-4`.
    pub hero: bool,
    /// Starter questions under the box (`prompts` not emptied by the host).
    pub suggestions: bool,
    /// The closed bar's folded composer (`compact` without focus): one line, the send or stop
    /// button inside it, no toolbar.
    pub compact: bool,
}

/// A file being handed to the daemon.
pub struct Upload {
    pub key: String,
    pub bytes: Vec<u8>,
    pub offset: usize,
    pub upload_id: Option<String>,
    /// The book the file was chosen in; the daemon stages it for that book only.
    pub book: chat::Book,
}

fn ext(name: &str) -> String {
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// `chatAttachmentKind`.
pub fn kind_of(name: &str, staged: Option<&str>) -> Option<&'static str> {
    match staged {
        Some("spreadsheet") => return Some("spreadsheet"),
        Some("delimited") => return Some("delimited"),
        Some("pdf") => return Some("pdf"),
        Some("image") => return Some("image"),
        _ => {}
    }
    match ext(name).as_str() {
        "xlsx" | "xlsm" => Some("spreadsheet"),
        "csv" | "tsv" => Some("delimited"),
        "pdf" => Some("pdf"),
        "png" | "jpg" | "jpeg" | "webp" | "gif" => Some("image"),
        _ => None,
    }
}

/// `formatAttachmentSize`: `42 B`, `48 kB`, `1.2 MB` (decimal units, Intl's short unit names).
pub fn format_size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let units = ["byte", "kB", "MB"];
    let (mut v, mut u) = (bytes as f64, 0);
    while v >= 1000.0 && u < units.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    let digits = if u == 0 || v >= 10.0 { 0 } else { 1 };
    let mut n = format!("{v:.digits$}");
    if crate::i18n::lang() == crate::i18n::Lang::De {
        n = n.replace('.', ",");
    }
    format!("{n} {}", units[u])
}

/// The composer: box, then the suggestion chips below it.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, o: Opts, layer: &mut Vec<El>) -> El {
    let a = &app.assistant;
    let trimmed = a.draft.text.trim();
    let mut surface = glass(
        tk,
        col()
            .key(format!("asst-composer-{}", o.host))
            .w_full()
            .radius(tk.r(1.0))
            .min_h(if o.hero { 96.0 } else { 72.0 }),
    )
    .focus_within_ring(2.0, 2.0, tk.ring.alpha(0.55))
    .on_drop_files(|s: &mut App, files: &[std::path::PathBuf], _tx: &mut Tx| {
        s.assistant.thread_drop = false;
        attach_paths(s, files);
    })
    .on_drop_hover(|s: &mut App, over: bool, _tx: &mut Tx| s.assistant.thread_drop = over);
    if !a.files.is_empty() {
        let mut files = row()
            .w_full()
            .flex_wrap()
            .gap(tk.r(0.375))
            .px(tk.r(0.75))
            .pt(tk.r(0.75))
            .center_y();
        for f in &a.files {
            let note =
                (f.state == FileState::Sent).then(|| t("assistant:composer.attachmentInChat"));
            let remove = (f.state != FileState::Uploading).then(|| f.key.clone());
            files = files.child(attachment_card(
                tk,
                &f.view,
                f.state == FileState::Uploading,
                note,
                remove,
            ));
        }
        surface = surface.child(files);
    }
    if o.compact {
        surface = surface.min_h(52.0);
        let button = if app.assistant.streaming {
            stop_button(tk, o.host)
        } else {
            send_button(app, tk, o.host)
        };
        surface = surface.child(
            row()
                .w_full()
                .grow(1.0)
                .center_y()
                .child(field(app, tk, o))
                .child(button.mr(tk.r(0.625))),
        );
    } else {
        surface = surface.child(field(app, tk, o));
        surface = surface.child(toolbar(app, cx, tk, o, layer));
    }
    if a.thread_drop {
        surface = surface.child(
            row()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .gap(tk.r(0.5))
                .radius(tk.r(1.0))
                .border(2.0, tk.ring.alpha(0.6))
                .fill(tk.background.alpha(0.85))
                .inert()
                .child(ic(&I::PAPERCLIP, tk.r(1.0), tk.fg))
                .child(
                    text(t("assistant:composer.dropToAttach"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                ),
        );
    }
    let mut c = col()
        .w_full()
        .gap(if o.hero { tk.r(1.0) } else { tk.r(0.75) })
        .child(surface);
    let show_suggestions = o.suggestions && !a.streaming;
    let _ = trimmed;
    if show_suggestions {
        c = c.child(suggestions(tk, o.host));
    }
    c
}

/// `.kb-composer-glass`: the card (dark: card mixed 8 % toward the foreground) at the glass
/// opacity over a 12 px backdrop blur; light: a black 8 % hairline and a 1 px drop; dark: a
/// white 5 % hairline and a 3 % top highlight.
pub fn glass(tk: &Tk, el: El) -> El {
    if tk.dark {
        let surface = rune::color::hex("#292929").unwrap_or(tk.card);
        el.fill(surface.alpha(0.8))
            .backdrop_blur(12.0)
            .box_shadow(Shadow {
                spread: 1.0,
                color: Color::WHITE.alpha(0.05),
                ..Shadow::default()
            })
            .box_shadow(Shadow {
                y: 1.0,
                color: Color::WHITE.alpha(0.03),
                inset: true,
                ..Shadow::default()
            })
    } else {
        el.fill(tk.card.alpha(0.8))
            .backdrop_blur(12.0)
            .box_shadow(Shadow {
                spread: 1.0,
                color: Color::BLACK.alpha(0.08),
                ..Shadow::default()
            })
            .box_shadow(Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..Shadow::default()
            })
    }
}

/// The text field: `text-base leading-relaxed px-4 pt-4 pb-1`, growing up to 176 px.
fn field(app: &App, tk: &Tk, o: Opts) -> El {
    let style = rune::TextStyle::new(tk.r(1.0), 400.0).lh(tk.r(1.625));
    let host = o.host;
    let input = text_input(&app.assistant.draft, style)
        .key(format!("asst-input-{host}"))
        .w_full()
        .color(tk.fg)
        .placeholder(t("assistant:composer.placeholder"), tk.muted_fg.alpha(0.45))
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(t("assistant:composer.placeholder"))
        .on_edit(|s: &mut App| &mut s.assistant.draft)
        .on_input_change(|s: &mut App, _tx: &mut Tx| {
            s.assistant.recall = None;
            // `setDraft`: a parked handoff stays only while the draft has text.
            if s.assistant.draft.text.trim().is_empty() {
                s.assistant.parked = None;
            }
        })
        // Return sends (or queues); a refused send keeps the text. Shift+Return breaks the line.
        .on_submit(move |s: &mut App, _tx: &mut Tx| submit_from(s, host))
        .on_input_key(|s: &mut App, ev: &KeyEvent, _tx: &mut Tx| recall_key(s, ev));
    let grow = rune::auto_grow(input, 1, 8).max_h(176.0);
    if o.compact {
        // `pr-14 pl-4 pt-3.5 pb-0`; the button sits in the right padding.
        return col()
            .min_w(0.0)
            .grow(1.0)
            .pl(tk.r(1.0))
            .pr(tk.r(0.5))
            .pt(tk.r(0.875))
            .pb(tk.r(0.875))
            .child(grow);
    }
    col()
        .w_full()
        .min_h(0.0)
        .grow(1.0)
        .px(tk.r(1.0))
        .pt(tk.r(1.0))
        .pb(tk.r(0.25))
        .child(grow)
}

/// A send from the closed bar opens the panel so the answer is in view; the panel's composer
/// takes focus when the panel was not already showing the assistant (useAssistantComposer).
fn submit_from(s: &mut App, host: &str) {
    if host == "bar" {
        let focus = !s.assistant.tab_open;
        super::open_panel(s);
        if focus {
            s.assistant.draft.focused = true;
        }
    }
    chat::submit(s);
}

/// ArrowUp/ArrowDown recall of this conversation's earlier prompts (composerPromptHistory.ts):
/// only from an empty field or an active recall, and only at the text's edge.
fn recall_key(s: &mut App, ev: &KeyEvent) -> bool {
    let up = match ev.key {
        Key::Up => true,
        Key::Down => false,
        _ => return false,
    };
    if ev.mods.shift || ev.mods.alt || ev.mods.cmd || ev.mods.ctrl {
        return false;
    }
    let a = &mut s.assistant;
    if a.files.iter().any(|f| f.state != FileState::Sent) {
        return false;
    }
    if a.recall.is_none() && !a.draft.text.is_empty() {
        return false;
    }
    let caret = a.draft.focus;
    let at_edge = if up {
        !a.draft.text[..caret.min(a.draft.text.len())].contains('\n')
    } else {
        !a.draft.text[caret.min(a.draft.text.len())..].contains('\n')
    };
    if !at_edge {
        return false;
    }
    let prompts: Vec<String> = a
        .messages
        .iter()
        .filter(|m| m.role == Role::User && !m.content.trim().is_empty())
        .map(|m| m.content.clone())
        .collect();
    if prompts.is_empty() {
        return false;
    }
    let next = match (a.recall, up) {
        (None, true) => Some(prompts.len() - 1),
        (None, false) => return false,
        (Some(0), true) => return false,
        (Some(i), true) => Some(i - 1),
        (Some(i), false) if i + 1 < prompts.len() => Some(i + 1),
        // Past the newest: back to an empty draft.
        (Some(_), false) => None,
    };
    let text = next.map(|i| prompts[i].clone()).unwrap_or_default();
    let mut e = TextEdit::new(&text);
    e.focused = true;
    e.set_caret(text.len());
    a.draft = e;
    a.recall = next;
    a.parked = None;
    true
}

/// The toolbar: attach, the model picker, then send (`min-h-[44px] px-2.5 pb-2.5 gap-1`).
fn toolbar(app: &App, cx: &mut Cx<App>, tk: &Tk, o: Opts, layer: &mut Vec<El>) -> El {
    let attach = row()
        .key(format!("asst-attach-{}", o.host))
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(999.0)
        .hover(if tk.dark {
            tk.accent.alpha(0.5)
        } else {
            tk.accent
        })
        .role("button")
        .label(t("assistant:composer.attach"))
        .help(t("assistant:composer.attach"))
        .focusable(true)
        .on_press_tx(|s: &mut App, tx: &mut Tx| attach_file(s, tx))
        .child(ic(&I::PLUS, tk.r(1.0), tk.muted_fg));
    if app.assistant.picker.open == Some(o.host) {
        picker::mount(app, cx, tk, o.host, layer);
    }
    let pick = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .center_y()
        .child(picker::trigger(app, tk, o.host));
    let streaming = app.assistant.streaming;
    let mut actions = row().gap(tk.r(0.5)).center_y();
    if streaming {
        actions = actions.child(stop_button(tk, o.host));
    }
    if !streaming || !app.assistant.draft.text.trim().is_empty() {
        actions = actions.child(send_button(app, tk, o.host));
    }
    row()
        .w_full()
        .min_h(44.0)
        .px(tk.r(0.625))
        .pb(tk.r(0.625))
        .gap(tk.r(0.25))
        .center_y()
        .child(attach)
        .child(pick)
        .child(actions)
}

/// The round Stop button (`bg-destructive/90 text-white`).
fn stop_button(tk: &Tk, host: &str) -> El {
    let label = t("assistant:composer.stopGenerating");
    row()
        .key(format!("asst-stop-{host}"))
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(999.0)
        .fill(tk.destructive.alpha(0.9))
        .hover(tk.destructive)
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: tk.destructive.alpha(0.24),
            ..Shadow::default()
        })
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .on_press(chat::abort)
        .child(ic(&STOP_MARK, 12.0, Color::WHITE))
}

/// The round send button (`bg-primary`, a faint foreground tint while there is nothing to
/// send). While an answer streams it queues (`ListPlus`). Named by why it is held, else by what
/// it does; the tooltip adds the keys.
fn send_button(app: &App, tk: &Tk, host: &str) -> El {
    let blocked = chat::send_blocked_reason(app);
    let can = chat::can_submit(app);
    let queue = can && app.assistant.streaming;
    let label = blocked.clone().unwrap_or_else(|| {
        t(if queue {
            "assistant:composer.queueMessage"
        } else {
            "assistant:composer.sendMessage"
        })
    });
    let tip = blocked.clone().unwrap_or_else(|| {
        let (send, newline) = if cfg!(target_os = "macos") {
            ("↩", "⇧↩")
        } else {
            ("Enter", "Shift+Enter")
        };
        format!(
            "{label}  {send}   {}  {newline}",
            t("assistant:composer.newLine")
        )
    });
    let mut b = row()
        .key(format!("asst-send-{host}"))
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(999.0)
        .role("button")
        .label(label)
        .help(tip);
    if can {
        b = b
            .fill(tk.primary)
            .hover(tk.primary.alpha(0.9))
            .box_shadow(Shadow {
                y: 1.0,
                blur: 2.0,
                color: tk.primary.alpha(0.24),
                ..Shadow::default()
            })
            .focusable(true)
            .on_press({
                let host = host.to_owned();
                move |s: &mut App| submit_from(s, &host)
            })
            .child(if queue {
                ic(&I::LIST_PLUS, tk.r(1.0), tk.primary_fg)
            } else {
                ic(&SEND_ARROW, 14.0, tk.primary_fg)
            });
    } else {
        b = b
            .fill(tk.fg.alpha(0.12))
            .disabled(true)
            .child(ic(&SEND_ARROW, 14.0, tk.muted_fg));
    }
    b
}

/// `Suggestions`: `flex-wrap justify-center gap-2` pills that put the prompt in the field.
fn suggestions(tk: &Tk, host: &'static str) -> El {
    let mut r = row().w_full().flex_wrap().justify_center().gap(tk.r(0.5));
    for (i, (glyph, key)) in PROMPTS.iter().enumerate() {
        let label = t(&format!("assistant:prompts.{key}.text"));
        let prompt_key = format!("assistant:prompts.{key}.prompt");
        r = r.child(
            row()
                .key(format!("asst-suggest-{host}-{i}"))
                .px(tk.r(0.75))
                .py(tk.r(0.375))
                .gap(tk.r(0.5))
                .center_y()
                .radius(999.0)
                .fill(tk.muted.alpha(if tk.dark { 0.4 } else { 0.5 }))
                .border(1.0, tk.border.alpha(tk.border.a * 0.6))
                .border_layout()
                .hover(tk.muted)
                .role("button")
                .label(label.clone())
                .focusable(true)
                .on_press(move |s: &mut App| fill_prompt(s, &t(&prompt_key)))
                .child(ic(glyph, tk.r(1.0), tk.muted_fg))
                .child(text(label).font(tk.tsm(400.0)).color(tk.muted_fg)),
        );
    }
    r
}

/// A suggestion or launcher row: the prompt replaces the draft and the field takes focus.
pub fn fill_prompt(s: &mut App, prompt: &str) {
    let mut e = TextEdit::new(prompt);
    e.focused = true;
    e.set_caret(prompt.len());
    s.assistant.draft = e;
    s.assistant.recall = None;
    s.assistant.parked = None;
}

/// `AssistantFocusChip`: "About" and the pinned item, removable.
pub fn focus_chip(app: &App, tk: &Tk, centered: bool) -> Option<El> {
    let f = app.assistant.focus.as_ref()?;
    let remove = tr("assistant:panel.removeFocus", &[("label", &f.label)]);
    let chip = row()
        .min_w(0.0)
        .gap(tk.r(0.25))
        .center_y()
        .radius(999.0)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .fill(tk.muted.alpha(0.6))
        .py(tk.r(0.125))
        .pr(tk.r(0.125))
        .pl(tk.r(0.5))
        .child(
            text(f.label.clone())
                .font(tk.txs(500.0))
                .color(tk.fg)
                .truncate(),
        )
        .child(
            row()
                .key("asst-focus-remove")
                .size(tk.r(1.0), tk.r(1.0))
                .center()
                .radius(999.0)
                .hover(tk.muted)
                .role("button")
                .label(remove)
                .focusable(true)
                .on_press(|s: &mut App| s.assistant.focus = None)
                .child(ic(&I::X, tk.r(0.75), tk.muted_fg)),
        );
    let mut r = row()
        .w_full()
        .gap(tk.r(0.375))
        .px(tk.r(0.25))
        .center_y()
        .child(
            text(t("assistant:panel.focusLabel"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        )
        .child(chip);
    if centered {
        r = r.justify_center();
    }
    Some(r)
}

/// `ChatAttachmentCard`: a type tile, the name, the type and size (or `note`), and a remove
/// button on hover when `remove` names the file.
pub fn attachment_card(
    tk: &Tk,
    f: &AttachmentView,
    uploading: bool,
    note: Option<String>,
    remove: Option<String>,
) -> El {
    let glyph: &'static Icon = match kind_of(&f.filename, f.kind.as_deref()) {
        Some("spreadsheet" | "delimited") => &I::FILE_SPREADSHEET,
        Some("pdf") => &I::FILE_TEXT,
        Some("image") => &I::FILE_IMAGE,
        _ => &I::FILE,
    };
    let details = if uploading {
        t("assistant:composer.attachmentUploading")
    } else {
        note.unwrap_or_else(|| {
            let mut parts = vec![ext(&f.filename).to_uppercase()];
            if let Some(n) = f.size_bytes.filter(|n| *n > 0) {
                parts.push(format_size(n));
            }
            parts.retain(|p| !p.is_empty());
            parts.join(" · ")
        })
    };
    let mut tile = row()
        .size(tk.r(2.25), tk.r(2.25))
        .center()
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .child(ic(glyph, tk.r(1.0), tk.muted_fg));
    if uploading {
        tile = tile.child(
            row()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .fill(tk.background.alpha(0.7))
                .child(crate::txdetail::widgets::spinner(tk, tk.r(1.0))),
        );
    }
    let key = format!(
        "asst-file-{}",
        remove.clone().unwrap_or_else(|| f.filename.clone())
    );
    let mut card = row()
        .key(key.clone())
        .max_w(tk.r(16.0))
        .min_w(0.0)
        .gap(tk.r(0.625))
        .center_y()
        .radius(tk.r(0.75))
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .fill(tk.background.alpha(0.6))
        .padding(tk.r(0.375))
        .pr(tk.r(0.75))
        .hover_group()
        .role("group")
        .label(f.filename.clone())
        .child(tile.a11y_hidden())
        .child(
            col()
                .min_w(0.0)
                .child(
                    text(f.filename.clone())
                        .font(tk.txs(500.0))
                        .color(tk.fg)
                        .truncate()
                        .help(f.filename.clone()),
                )
                .child(
                    text(details)
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg)
                        .truncate(),
                ),
        );
    if let Some(k) = remove {
        let named = tr(
            "assistant:composer.removeAttachmentNamed",
            &[("filename", &f.filename)],
        );
        card = card.child(
            row()
                .key(format!("{key}-remove"))
                .absolute(0.0, 0.0)
                .size(20.0, 20.0)
                .offset(-6.0, -6.0)
                .center()
                .radius(10.0)
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.background)
                .box_shadow(Shadow {
                    y: 1.0,
                    blur: 2.0,
                    color: Color::BLACK.alpha(0.05),
                    ..Shadow::default()
                })
                .group_opacity(0.0, 1.0)
                .role("button")
                .label(named)
                .help(t("assistant:composer.removeAttachment"))
                .focusable(true)
                .on_press(move |s: &mut App| s.assistant.files.retain(|f| f.key != k))
                .child(ic(&I::X, tk.r(0.75), tk.muted_fg)),
        );
    }
    card
}

// ---- Attaching -------------------------------------------------------------------------------

/// `attachFile`: the native picker, then the daemon stages the file for this book.
pub fn attach_file(app: &mut App, tx: &mut Tx) {
    if !app.prefs.ai_features {
        return;
    }
    if chat::book_now(app).is_none() {
        chat::refuse_book_loading(app, None);
        return;
    }
    if app.assistant.files.len() >= MAX_FILES {
        limit_toast(app);
        return;
    }
    let mut options = rune::FileDialogOptions {
        title: t("assistant:composer.attach"),
        ..Default::default()
    };
    options = options.filter("Kassiber", &EXTENSIONS);
    tx.open_file(options, |a: &mut App, result, _tx| {
        if let Ok(Some(path)) = result {
            attach_paths(a, &[path]);
        }
    });
}

fn limit_toast(app: &mut App) {
    let max = MAX_FILES.to_string();
    app.toast(
        ToastTone::Info,
        &tr("assistant:composer.attachmentLimit.title", &[("max", &max)]),
        Some(tr(
            "assistant:composer.attachmentLimit.body",
            &[("max", &max)],
        )),
        Some("assistant-attachmentLimit"),
    );
}

/// A picked file's bytes, read only up to `limit` (+1 to tell it is over): a multi-gigabyte
/// pick is refused without loading it. `None`: larger than `limit`. Only regular files.
fn read_bounded(p: &std::path::Path, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let file = std::fs::File::open(p)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    if meta.len() > limit as u64 {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    // The file can grow between the size check and the read.
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() <= limit).then_some(bytes))
}

/// Files picked or dropped: each is read and handed to the daemon one after another, shown as an
/// uploading card meanwhile.
pub fn attach_paths(app: &mut App, paths: &[std::path::PathBuf]) {
    if !app.prefs.ai_features {
        return;
    }
    // The files belong to the book open at the user's action.
    let Some(book) = chat::book_now(app) else {
        chat::refuse_book_loading(app, None);
        return;
    };
    let mut unsupported = false;
    for p in paths {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !EXTENSIONS.contains(&ext(&name).as_str()) {
            unsupported = true;
            continue;
        }
        if app.assistant.files.len() >= MAX_FILES {
            limit_toast(app);
            break;
        }
        let bytes = match read_bounded(p, MAX_UPLOAD) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                app.toast(
                    ToastTone::Warning,
                    &t("assistant:composer.attachmentTooLarge.title"),
                    Some(tr(
                        "assistant:composer.attachmentTooLarge.bodyNamed",
                        &[("filename", &name)],
                    )),
                    Some("assistant-attachmentTooLarge"),
                );
                continue;
            }
            Err(_) => {
                app.toast(
                    ToastTone::Warning,
                    &t("assistant:composer.attachmentFailed"),
                    Some(name.clone()),
                    Some("assistant-attachment-failed"),
                );
                continue;
            }
        };
        if bytes.is_empty() {
            continue;
        }
        app.assistant.next_file += 1;
        let key = format!("upload-{}", app.assistant.next_file);
        let view = AttachmentView {
            kind: kind_of(&name, None).map(str::to_owned),
            size_bytes: Some(bytes.len() as u64),
            filename: name,
        };
        app.assistant.files.push(ChatFile {
            key: key.clone(),
            view,
            state: FileState::Uploading,
            book: Some(book.clone()),
        });
        app.assistant.uploads.push(Upload {
            key,
            bytes,
            offset: 0,
            upload_id: None,
            book: book.clone(),
        });
    }
    if unsupported {
        app.toast(
            ToastTone::Warning,
            &t("assistant:composer.attachmentUnsupported.title"),
            Some(t("assistant:composer.attachmentUnsupported.body")),
            Some("assistant-attachmentUnsupported"),
        );
    }
    start_queue(app);
}

/// Starts the upload queue unless a chunk is in flight: re-sending the pending chunk (same id
/// and offset) would fail the daemon's offset check and abort a valid upload.
fn start_queue(app: &mut App) {
    if !app.assistant.uploads.is_empty() && !app.assistant.upload_busy {
        send_chunk(app);
    }
}

/// Sends the next chunk of the first queued upload.
fn send_chunk(app: &mut App) {
    let Some(u) = app.assistant.uploads.first() else {
        app.assistant.upload_busy = false;
        return;
    };
    if !app.prefs.ai_features {
        app.assistant.uploads.clear();
        app.assistant
            .files
            .retain(|f| f.state != FileState::Uploading);
        app.assistant.upload_busy = false;
        return;
    }
    let scope = chat::scope_json(&u.book);
    let end = (u.offset + CHUNK).min(u.bytes.len());
    let chunk = base64(&u.bytes[u.offset..end]);
    let mut args = match &u.upload_id {
        None => {
            let name = app
                .assistant
                .files
                .iter()
                .find(|f| f.key == u.key)
                .map(|f| f.view.filename.clone())
                .unwrap_or_default();
            json!({"filename": name, "total_bytes": u.bytes.len(), "expected_scope": scope})
        }
        Some(id) => json!({"upload_id": id, "expected_scope": scope}),
    };
    args["offset"] = json!(u.offset);
    args["chunk_base64"] = json!(chunk);
    app.assistant.upload_busy = true;
    let key = u.key.clone();
    app.assistant.upload_sent.push_back(key);
    app.send("ui.chat.attachments.upload", args, "assistant.upload");
}

/// The daemon's answer to a chunk: the next chunk, the staged file, or why it failed.
pub fn on_upload_reply(app: &mut App, r: &crate::app::Reply) {
    let sent = app.assistant.upload_sent.pop_front();
    // A chunk of a file the chat let go of (cleared meanwhile): the queue's first file is
    // another one, whose own chunk is already out, or the queue is empty.
    if app.assistant.uploads.first().map(|u| &u.key) != sent.as_ref() {
        if app.assistant.uploads.is_empty() {
            app.assistant.upload_busy = false;
        }
        return;
    }
    let key = app.assistant.uploads[0].key.clone();
    if let Some(e) = &r.error {
        app.assistant.uploads.remove(0);
        app.assistant.files.retain(|f| f.key != key);
        app.toast(
            ToastTone::Warning,
            &t("assistant:composer.attachmentFailed"),
            Some(e.clone()),
            Some("assistant-attachment-failed"),
        );
    } else if r.data["complete"].as_bool() == Some(true) {
        let u = app.assistant.uploads.remove(0);
        let token = r.data["document_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        // `stageAttachment`: a file of another book (switched meanwhile), or one already in the
        // chat, is not listed; the first file binds a fresh conversation to its book.
        let bound = app.assistant.book.clone();
        if bound.as_ref().is_some_and(|b| *b != u.book)
            || chat::book_now(app).is_some_and(|b| b != u.book)
        {
            app.assistant.files.retain(|f| f.key != u.key);
            send_chunk(app);
            return;
        }
        if app
            .assistant
            .files
            .iter()
            .any(|f| f.key == token && f.state != FileState::Uploading)
        {
            app.assistant.files.retain(|f| f.key != u.key);
            send_chunk(app);
            return;
        }
        if bound.is_none() {
            app.assistant.book = Some(u.book.clone());
        }
        if let Some(f) = app.assistant.files.iter_mut().find(|f| f.key == u.key) {
            f.state = FileState::Ready;
            if !token.is_empty() {
                f.key = token;
            }
            if let Some(kind) = r.data["source"]["kind"].as_str() {
                f.view.kind = Some(kind.to_owned());
            }
        }
    } else {
        let u = &mut app.assistant.uploads[0];
        u.upload_id = r.data["upload_id"]
            .as_str()
            .map(str::to_owned)
            .or(u.upload_id.take());
        u.offset = (u.offset + CHUNK).min(u.bytes.len());
    }
    send_chunk(app);
}

/// Standard base64 (RFC 4648, padded).
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads stop at the limit: a file over it is refused, one at it is read whole, and
    /// something that is not a regular file (a directory here) is an error.
    #[test]
    fn attachment_reads_are_bounded() {
        let dir = std::env::temp_dir().join(format!("kassiber-attach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let at = dir.join("at.csv");
        let over = dir.join("over.csv");
        std::fs::write(&at, [7u8; 16]).unwrap();
        std::fs::write(&over, [7u8; 17]).unwrap();
        assert_eq!(read_bounded(&at, 16).unwrap().unwrap().len(), 16);
        assert!(read_bounded(&over, 16).unwrap().is_none());
        assert!(read_bounded(&dir, 16).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn queue(app: &mut App, key: &str, name: &str) {
        let book: chat::Book = ("w".into(), "p".into());
        app.assistant.files.push(ChatFile {
            key: key.into(),
            view: AttachmentView {
                filename: name.into(),
                kind: None,
                size_bytes: Some(3),
            },
            state: FileState::Uploading,
            book: Some(book.clone()),
        });
        app.assistant.uploads.push(Upload {
            key: key.into(),
            bytes: b"abc".to_vec(),
            offset: 0,
            upload_id: None,
            book,
        });
        send_chunk(app);
    }

    /// A chunk of the only queued upload in flight (a second file was refused): starting the
    /// queue again sends nothing, so the daemon never sees a duplicate offset.
    #[test]
    fn a_chunk_in_flight_is_not_sent_twice() {
        let mut app = crate::app::tests::quiet_app();
        queue(&mut app, "a", "a.csv");
        assert!(app.assistant.upload_busy);
        let sent = app.pending.len();
        start_queue(&mut app);
        assert_eq!(app.pending.len(), sent, "no duplicate chunk");
    }

    fn staged(token: &str) -> crate::app::Reply {
        crate::app::Reply {
            kind: "ui.chat.attachments.upload".into(),
            data: serde_json::json!({"complete": true, "document_token": token}),
            error: None,
            error_detail: serde_json::Value::Null,
            elapsed: Default::default(),
        }
    }

    /// Clearing the chat while file A's last chunk is out, then attaching B: A's answer must not
    /// hand A's staged document to B's card; B's own answer does.
    #[test]
    fn an_abandoned_uploads_answer_does_not_land_on_the_next_file() {
        let mut app = crate::app::tests::quiet_app();
        app.prefs.ai_features = true;
        queue(&mut app, "upload-1", "a.csv");
        chat::start_fresh(&mut app);
        queue(&mut app, "upload-2", "b.csv");
        on_upload_reply(&mut app, &staged("doc-a"));
        let f = &app.assistant.files;
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].key.as_str(), f[0].state),
            ("upload-2", FileState::Uploading)
        );
        on_upload_reply(&mut app, &staged("doc-b"));
        let f = &app.assistant.files[0];
        assert_eq!(
            (f.key.as_str(), f.view.filename.as_str(), f.state),
            ("doc-b", "b.csv", FileState::Ready)
        );
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn kinds_follow_the_extension() {
        assert_eq!(kind_of("a.XLSX", None), Some("spreadsheet"));
        assert_eq!(kind_of("a.csv", None), Some("delimited"));
        assert_eq!(kind_of("a.jpeg", None), Some("image"));
        assert_eq!(kind_of(".csv", None), None);
        assert_eq!(kind_of("a.txt", Some("pdf")), Some("pdf"));
    }

    #[test]
    fn sizes_read_like_intl() {
        assert_eq!(format_size(42), "42 B");
        assert_eq!(format_size(48_213), "48 kB");
        assert_eq!(format_size(1_204_000), "1.2 MB");
    }
}
