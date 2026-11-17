//! The full Assistant page (routes/Assistant.tsx): the toolbar (Incognito marker, message
//! count, Export, More actions with Incognito, History and Clear chat), then either the shared
//! empty state (`empty.rs`: greeting, model chooser, composer, recent chats) or the conversation
//! with the composer docked under it. Also RecentChats.tsx and ChatHistoryPanel.tsx.

use rune::{Align, Cx, El, Side, Tx, col, row, text};

use super::Menu;
use super::composer::{self, Opts};
use super::model::{MsgStatus, Role};
use crate::app::{App, Load};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button};
use crate::ui::ic;
use crate::ui::popup::TriggerExt;

/// `max-w-3xl`.
const COLUMN: f32 = 48.0;

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let a = &app.assistant;
    let has_messages = !a.messages.is_empty();
    let viewport = cx
        .layout_size("app-main")
        .map(|[_, h]| h)
        .unwrap_or(app.shell.window_h - 47.0)
        .max(0.0);
    let mut page = col()
        .key("asst-page")
        .w_full()
        .fill(tk.background)
        .role("main");
    if has_messages {
        page = page.h(viewport);
    } else {
        page = page.min_h(viewport).pb(tk.r(1.0));
    }
    page = page.child(toolbar(app, tk, layer));
    super::checks::dialog(app, tk, layer);
    let mut section = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .role("region")
        .label(t("assistant:page.conversationLabel"));
    if has_messages {
        section = section.child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(super::thread::view(app, cx, tk, true, layer)),
        );
    } else {
        // The shared empty state (the maximized side panel draws the same; it shows the error).
        section = section.child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(480.0)
                .px(tk.r(1.5))
                .py(tk.r(1.5))
                .center()
                .child(super::empty::view(
                    app,
                    cx,
                    tk,
                    super::empty::Host::Page,
                    layer,
                )),
        );
    }
    if let (true, Some(e)) = (has_messages, &a.error) {
        section = section.child(centered(
            col()
                .w_full()
                .max_w(tk.r(COLUMN))
                .mt(tk.r(0.75))
                .px(tk.r(1.25))
                .child(
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
                ),
        ));
    }
    if has_messages {
        // The composer docked under the thread (`sticky bottom-0`, a fade from the background).
        let mut dock = col()
            .w_full()
            .max_w(tk.r(COLUMN))
            .mt(tk.r(0.75))
            .px(tk.r(1.25))
            .pt(tk.r(1.5))
            .pb(tk.r(0.75));
        if let Some(chip) = composer::focus_chip(app, tk, false) {
            dock = dock.child(chip.mb(tk.r(0.5)));
        }
        dock = dock.child(composer::view(
            app,
            cx,
            tk,
            Opts {
                host: "page",
                hero: false,
                suggestions: false,
                compact: false,
            },
            layer,
        ));
        section = section.child(centered(dock));
    }
    page.child(section)
}

/// `mx-auto`: a capped column centered in the full width.
pub fn centered(inner: El) -> El {
    row().w_full().justify_center().child(inner)
}

/// The full-width toolbar: status on the left, Export and More actions on the right.
fn toolbar(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let a = &app.assistant;
    let has_messages = !a.messages.is_empty();
    let mut left = row().grow(1.0).min_w(0.0).gap(tk.r(0.5)).center_y();
    if a.incognito {
        // A standing marker while nothing is stored; pressing it turns Incognito off.
        let title = [
            t("assistant:page.incognitoActiveTitle"),
            incognito_scope(app).unwrap_or_default(),
            t("assistant:page.incognitoTurnOff"),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        left = left.child(
            row()
                .key("asst-incognito-marker")
                .gap(tk.r(0.375))
                .center_y()
                .radius(999.0)
                .fill(tk.primary.alpha(0.1))
                .hover(tk.primary.alpha(0.15))
                .px(tk.r(0.5))
                .py(tk.r(0.125))
                .role("button")
                .label(t("assistant:page.incognito"))
                .help(title)
                .focusable(true)
                .on_press(|s: &mut App| s.assistant.incognito = false)
                .child(ic(&I::EYE_OFF, tk.r(0.875), tk.primary))
                .child(
                    text(t("assistant:page.incognito"))
                        .font(tk.txs(500.0))
                        .color(tk.primary),
                ),
        );
    }
    if has_messages {
        if a.streaming {
            let label = if a.queued.is_empty() {
                t("assistant:page.generating")
            } else {
                tc("assistant:page.generatingQueued", a.queued.len(), &[])
            };
            left = left
                .child(row().child(text(label).font(tk.txs(500.0)).color(tk.primary).truncate()));
        } else {
            left = left.child(
                text(tc("assistant:page.messageCount", a.messages.len(), &[]))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .truncate(),
            );
        }
    }
    let mut bar = row()
        .w_full()
        .px(tk.r(1.5))
        .pt(tk.r(1.0))
        .pb(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_end()
        .child(left);
    if has_messages {
        bar = bar.child(
            button(
                tk,
                "asst-export",
                Variant::Ghost,
                Size::Sm,
                Some(&I::DOWNLOAD),
                &t("assistant:page.exportChat"),
                true,
            )
            .on_press_tx(|s: &mut App, tx: &mut Tx| export_chat(s, tx)),
        );
    }
    let more_open = matches!(a.menu, Some(Menu::More | Menu::History));
    bar = bar.child(
        ghost_icon(
            tk,
            "asst-more",
            &I::ELLIPSIS,
            t("assistant:page.moreActions"),
        )
        .menu_trigger(more_open, |s: &mut App| {
            s.assistant.menu = if matches!(s.assistant.menu, Some(Menu::More | Menu::History)) {
                None
            } else {
                Some(Menu::More)
            }
        }),
    );
    if more_open {
        more_menu(app, tk, layer);
    }
    bar
}

/// A ghost `icon-sm` button (`size-8`, a `size-4` glyph in the foreground).
fn ghost_icon(tk: &Tk, key: &str, glyph: &'static rune::Icon, label: String) -> El {
    row()
        .key(key.to_owned())
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(tk.r(0.375))
        .hover(if tk.dark {
            tk.accent.alpha(0.5)
        } else {
            tk.accent
        })
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .child(ic(glyph, tk.r(1.0), tk.fg))
}

/// `useIncognitoScope`: what the selected provider itself keeps, said next to Incognito.
fn incognito_scope(app: &App) -> Option<String> {
    let (provider, _) = app.assistant.selection.as_ref()?;
    let providers = app.assistant.providers.ready()?;
    let p = providers.iter().find(|p| &p.name == provider)?;
    let key = if p.kind == "local" {
        "local"
    } else if p.base_url.starts_with("copilot-cli://") {
        "copilot"
    } else if p.base_url.starts_with("opencode-cli://") && cfg!(target_os = "windows") {
        "opencodeWindows"
    } else if p.is_cli() {
        "cli"
    } else {
        "remote"
    };
    Some(t(&format!("assistant:page.incognitoScope.{key}")))
}

/// The More actions menu (`align="end" min-w-52`).
fn more_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let a = &app.assistant;
    let mut pp = crate::ui::popup::Popup::menu("asst-more", |s: &mut App| s.assistant.menu = None)
        .list_key("asst-more-menu");
    let mut list = col()
        .min_w(tk.r(13.0))
        .padding(tk.r(0.25))
        .label(t("assistant:page.moreActions"));
    if a.session_id.is_none() {
        let title = [
            t("assistant:page.incognitoTitle"),
            incognito_scope(app).unwrap_or_default(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        let mut item = crate::shell::menu_row(
            tk,
            "asst-menu-incognito",
            Some(&I::EYE_OFF),
            &t("assistant:page.incognito"),
            None,
        )
        .role("menuitemcheckbox")
        .checked(a.incognito)
        .help(title)
        // Keeps the menu open so the toggle state is visible.
        .on_press(|s: &mut App| s.assistant.incognito = !s.assistant.incognito);
        if a.incognito {
            item = item.child(ic(&I::CHECK, tk.r(1.0), tk.primary));
        }
        list = list.child(pp.prepare(
            item,
            "asst-menu-incognito",
            t("assistant:page.incognito"),
            true,
            false,
        ));
    }
    let history_open = a.menu == Some(Menu::History);
    let hist_l = t("assistant:history.trigger");
    let mut hist =
        crate::shell::menu_row(tk, "asst-menu-history", Some(&I::HISTORY), &hist_l, None)
            .child(ic(&I::CHEVRON_RIGHT, tk.r(1.0), tk.muted_fg));
    if history_open {
        hist = hist.fill(tk.accent);
    }
    // `DropdownMenuSub`: hover, a press, ArrowRight or Enter open it.
    let hist =
        crate::ui::popup::sub_trigger(hist, history_open, "asst-history-open-0", |s: &mut App| {
            if s.assistant.menu != Some(Menu::History) {
                super::open_history(s);
            }
        });
    list = list.child(pp.prepare(hist, "asst-menu-history", hist_l, true, false));
    if !a.messages.is_empty() {
        let l = t("assistant:page.clearChat");
        list = list.child(crate::shell::menu_sep(tk)).child(pp.prepare(
            destructive_item(tk, "asst-menu-clear", &l).on_press(super::reset),
            "asst-menu-clear",
            l,
            true,
            false,
        ));
    }
    layer.push(rune_kit::scrim(
        "asst-more-scrim".to_owned(),
        |s: &mut App| s.assistant.menu = None,
    ));
    let (list, keys) = pp.finish_custom(list);
    let surface = keys
        .attach_to_list(crate::shell::glass(tk, list))
        .modal(true, |s: &mut App| s.assistant.menu = None)
        .focus_return("asst-more");
    layer.push(crate::shell::anchored(
        tk,
        surface,
        "asst-more",
        rune_kit::Placement::BottomEnd,
        4.0,
    ));
    if history_open {
        history_submenu(app, tk, layer);
    }
}

/// `DropdownMenuItem variant="destructive"`.
pub fn destructive_item(tk: &Tk, key: &str, label: &str) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.destructive.alpha(if tk.dark { 0.2 } else { 0.1 }))
        .focus_highlight(tk.destructive.alpha(if tk.dark { 0.2 } else { 0.1 }))
        .role("menuitem")
        .label(label.to_owned())
        .focusable(true)
        .child(ic(&I::TRASH_2, tk.r(1.0), tk.destructive))
        .child(
            text(label.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .grow(1.0),
        )
}

/// `ChatHistoryPanel`'s submenu (`max-h-96 w-80`): saved chats with resume and delete.
fn history_submenu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let a = &app.assistant;
    let mut pp = crate::ui::popup::Popup::sub(
        crate::ui::popup::Kind::Menu,
        "asst-more",
        "asst-menu-history",
        |s: &mut App| s.assistant.menu = None,
        |s: &mut App| s.assistant.menu = Some(Menu::More),
    )
    .list_key("asst-history-menu");
    let mut list = col()
        .w(tk.r(20.0))
        .padding(tk.r(0.25))
        .label(t("assistant:history.savedChats"));
    list = list
        .child(crate::shell::menu_label(
            tk,
            &t("assistant:history.savedChats"),
        ))
        .child(crate::shell::menu_sep(tk));
    let note = |s: String| {
        row()
            .w_full()
            .px(tk.r(0.5))
            .py(tk.r(0.75))
            .child(text(s).font(tk.tsm(400.0)).color(tk.muted_fg).wrap())
    };
    match &a.sessions {
        Load::Loading | Load::Idle => list = list.child(note(t("assistant:history.loading"))),
        Load::Failed(_) => list = list.child(note(t("assistant:history.disabled"))),
        Load::Ready(l) if l.sessions.is_empty() => {
            list = list.child(note(if l.history_enabled {
                t("assistant:history.empty")
            } else {
                t("assistant:history.disabled")
            }))
        }
        Load::Ready(l) => {
            let mut rows = col().w_full();
            for (i, s) in l.sessions.iter().enumerate() {
                let current = a.session_id.as_deref() == Some(s.id.as_str());
                let title = if current {
                    format!("{} {}", s.title, t("assistant:history.current"))
                } else {
                    s.title.clone()
                };
                let meta = tr(
                    "assistant:history.entryMeta",
                    &[
                        ("date", &medium_datetime(&s.updated_at)),
                        (
                            "messages",
                            &tc(
                                "assistant:history.messageCount",
                                s.message_count as usize,
                                &[],
                            ),
                        ),
                    ],
                );
                let id = s.id.clone();
                let del = s.id.clone();
                let resume_l = title.clone();
                let resume = col()
                    .grow(1.0)
                    .min_w(0.0)
                    .role("menuitem")
                    .label(title.clone())
                    .focusable(!a.streaming)
                    .disabled(a.streaming)
                    .opacity(if a.streaming { 0.5 } else { 1.0 })
                    .on_press(move |s: &mut App| super::resume(s, &id))
                    .child(text(title).font(tk.tsm(500.0)).color(tk.fg).truncate())
                    .child(text(meta).font(tk.txs(400.0)).color(tk.muted_fg));
                let resume = pp.prepare(
                    resume,
                    format!("asst-history-open-{i}"),
                    resume_l,
                    !a.streaming,
                    current,
                );
                let trash = row()
                    .key(format!("asst-history-del-{i}"))
                    .size(tk.r(1.75), tk.r(1.75))
                    .center()
                    .radius(tk.r(0.375))
                    .hover(if tk.dark {
                        tk.accent.alpha(0.5)
                    } else {
                        tk.accent
                    })
                    .role("button")
                    .label(tr("assistant:history.deleteChat", &[("title", &s.title)]))
                    .focusable(true)
                    .on_press(move |s: &mut App| super::delete_session(s, &del))
                    .child(ic(&I::TRASH_2, tk.r(0.875), tk.muted_fg));
                rows = rows.child(
                    row()
                        .key(format!("asst-history-row-{i}"))
                        .w_full()
                        .gap(tk.r(0.5))
                        .px(tk.r(0.5))
                        .py(tk.r(0.375))
                        .radius(tk.r(0.25))
                        .hover(tk.accent)
                        .child(resume)
                        .child(trash),
                );
            }
            list = list.child(
                col()
                    .w_full()
                    .max_h(tk.r(24.0) - tk.r(3.0))
                    .scroll(rune::ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(rows),
            );
        }
    }
    if let Some(e) = &a.resume_error {
        list = list.child(
            row().w_full().px(tk.r(0.5)).py(tk.r(0.5)).child(
                text(e.clone())
                    .font(tk.txs(400.0))
                    .color(tk.destructive)
                    .wrap(),
            ),
        );
    }
    let (list, keys) = pp.finish_custom(list);
    // Non-modal beside the modal More menu (Radix's sub content), so the parent stays live.
    let surface = keys.attach_to_list(crate::shell::glass(tk, list));
    layer.push(
        surface
            .anchored("asst-menu-history", Side::Left, Align::Start, 4.0)
            .transition(crate::shell::popover_enter(tk, Side::Left, Align::Start)),
    );
}

/// `toLocaleString({dateStyle: "medium", timeStyle: "short"})`: `Nov 14, 2026, 4:42 PM` /
/// `14.11.2026, 16:42` (UTC; this app has no time zone database).
pub fn medium_datetime(ts: &str) -> String {
    let Some(day) = crate::format::Day::parse(ts) else {
        return ts.to_owned();
    };
    let hm = ts.get(11..16).unwrap_or("00:00");
    let (h, m) = (
        hm.get(0..2)
            .and_then(|h| h.parse::<u32>().ok())
            .unwrap_or(0),
        hm.get(3..5).unwrap_or("00"),
    );
    if crate::i18n::lang() == crate::i18n::Lang::De {
        format!("{:02}.{:02}.{}, {h:02}:{m}", day.d, day.m, day.y)
    } else {
        let (h12, ap) = match h {
            0 => (12, "AM"),
            1..=11 => (h, "AM"),
            12 => (12, "PM"),
            _ => (h - 12, "PM"),
        };
        format!("{}, {h12}:{m}\u{202f}{ap}", day.long_ui())
    }
}

/// `RecentChats`: the three most recent saved chats, one click from continuing.
pub fn recent_chats(app: &App, tk: &Tk) -> El {
    let a = &app.assistant;
    let mut list = col().w_full();
    for (i, s) in a.recent.iter().enumerate() {
        let id = s.id.clone();
        let date = crate::format::Day::parse(&s.updated_at)
            .map(|d| d.month_day_ui())
            .unwrap_or_else(|| s.updated_at.clone());
        list = list.child(
            row()
                .key(format!("asst-recent-{i}"))
                .w_full()
                .gap(tk.r(0.625))
                .center_y()
                .radius(tk.r(0.5))
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .hover(tk.muted)
                .role("button")
                .label(s.title.clone())
                .focusable(!a.streaming)
                .disabled(a.streaming)
                .on_press(move |s: &mut App| super::resume(s, &id))
                .child(ic(&I::HISTORY, tk.r(0.875), tk.muted_fg))
                .child(
                    text(s.title.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(text(date).font(tk.txs(400.0)).color(tk.muted_fg)),
        );
    }
    col()
        .w_full()
        .max_w(tk.r(28.0))
        .child(
            row().w_full().mb(tk.r(0.375)).px(tk.r(0.75)).child(
                text(t("assistant:page.recentChats"))
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg),
            ),
        )
        .child(list)
}

/// `saveChatExport`: the transcript as Markdown, to a file the user names. Local only.
fn export_chat(app: &mut App, tx: &mut Tx) {
    if app.assistant.messages.is_empty() {
        return;
    }
    let now = crate::format::now_secs();
    let day = crate::format::Day::from_epoch_days((now / 86_400.0).floor() as i64);
    let secs = now.rem_euclid(86_400.0) as u64;
    let iso = format!(
        "{}T{:02}:{:02}:{:02}.{:03}Z",
        day.iso(),
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        ((now.fract()) * 1000.0) as u64
    );
    let markdown = export_markdown(app, &iso);
    let options = rune::FileDialogOptions {
        title: "Save chat export".into(),
        file_name: Some(format!("kassiber-chat-{}.md", day.iso())),
        ..Default::default()
    }
    .filter("Markdown", &["md"]);
    tx.save_file(options, move |_a: &mut App, result, _tx| {
        if let Ok(Some(path)) = result {
            // `toastopen`: owner request over the web (which confirms nothing): "File saved"
            // with Open / Show in Finder once the transcript is written.
            if std::fs::write(&path, &markdown).is_ok() {
                crate::toastopen::file_saved(_a, &path, "chat-export");
            }
        }
    });
}

/// `buildChatExportMarkdown`.
pub fn export_markdown(app: &App, iso: &str) -> String {
    let json = |v: &serde_json::Value| {
        let s = v.to_string();
        if s.chars().count() > 4000 {
            format!("{}...<truncated>", s.chars().take(4000).collect::<String>())
        } else {
            s
        }
    };
    let transcript: Vec<String> = app
        .assistant
        .messages
        .iter()
        .map(|m| {
            let status = match m.status {
                MsgStatus::Done => String::new(),
                MsgStatus::Pending => " (pending)".into(),
                MsgStatus::Streaming => " (streaming)".into(),
                MsgStatus::Error => " (error)".into(),
                MsgStatus::Cancelled => " (cancelled)".into(),
            };
            let header = format!(
                "## {}{status}",
                if m.role == Role::User {
                    "You"
                } else {
                    "Kassiber"
                }
            );
            let body = if !m.content.is_empty() {
                m.content.clone()
            } else if let Some(e) = &m.error_message {
                e.clone()
            } else if m.status == MsgStatus::Cancelled {
                "Stopped by user.".into()
            } else {
                String::new()
            };
            let mut tools = String::new();
            if !m.tool_calls.is_empty() {
                let mut lines = vec![String::new(), "Tools:".into()];
                for c in &m.tool_calls {
                    let st = format!("{:?}", c.status).to_lowercase();
                    lines.push(format!(
                        "- {}: {}",
                        c.name,
                        if st == "awaitingconsent" {
                            "awaiting_consent".into()
                        } else {
                            st
                        }
                    ));
                    if c.arguments.as_object().is_some_and(|o| !o.is_empty()) {
                        lines.push(format!("  Arguments: {}", json(&c.arguments)));
                    }
                    if let Some(r) = c.result.as_ref().filter(|r| !r.is_null()) {
                        lines.push(format!("  Result: {}", json(r)));
                    }
                    if let Some(r) = &c.reason {
                        lines.push(format!("  Reason: {r}"));
                    }
                }
                tools = lines.join("\n");
            }
            let prov = m
                .provenance
                .as_ref()
                .map(|p| ["".to_owned(), "Provenance:".into(), json(p)].join("\n"))
                .unwrap_or_default();
            format!("{header}\n\n{body}{tools}{prov}")
        })
        .collect();
    format!(
        "# Kassiber chat export\n\nExported: {iso}\n\n{}\n",
        transcript.join("\n\n---\n\n")
    )
}
