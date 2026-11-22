//! One message (ChatMessage.tsx): the user's bubble with its files, inline edit and collapse;
//! the assistant's work log (reasoning, tool calls, approvals), review cards, visuals, answer,
//! report facts, notices, provenance, loader, error and stop notes, and its hover actions.

use rune::{Color, El, Key, KeyEvent, TextEdit, Tx, col, row, text, text_input};
use serde_json::Value;

use super::{approval, md, tool, visual};
use crate::app::App;
use crate::assistant::model::{Msg, MsgStatus, ToolCall, ToolStatus};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// `bg-message`: the page background mixed 8 % toward white (dark), `#ebebed` (light).
fn message_fill(tk: &Tk) -> Color {
    rune::color::hex(if tk.dark { "#1e1e1e" } else { "#ebebed" }).unwrap_or(tk.muted)
}

/// A ghost `icon-xs` round action (`size-6 rounded-full`, a `size-3.5` glyph).
fn round_action(
    tk: &Tk,
    key: String,
    glyph: &'static rune::Icon,
    label: String,
    enabled: bool,
) -> El {
    let mut b = row()
        .key(key)
        .size(tk.r(1.5), tk.r(1.5))
        .center()
        .radius(999.0)
        .role("button")
        .label(label.clone())
        .child(ic(glyph, tk.r(0.875), tk.muted_fg).a11y_hidden());
    if enabled {
        b = b
            .hover(if tk.dark {
                tk.accent.alpha(0.5)
            } else {
                tk.accent
            })
            .help(label)
            .focusable(true);
    } else {
        b = b.disabled(true).opacity(0.5);
    }
    b
}

const COLLAPSE_CHARS: usize = 600;
const COLLAPSE_LINES: usize = 8;

/// `UserMessage`: files, the bubble (`max-w-[80%] rounded-2xl bg-message p-3 text-sm`), and
/// Show full message / Edit under it.
pub fn user(app: &App, tk: &Tk, m: &Msg, page: bool) -> El {
    let th = &app.assistant.thread;
    let key = format!("asst-msg-{}", m.id);
    let max_w = |w: f32| w * if page { 0.8 } else { 0.82 };
    let width = super::width(app, page);
    if let Some((id, edit)) = &th.editing
        && id == &m.id
    {
        let can_confirm = !edit.text.trim().is_empty();
        let field = text_input(edit, tk.tsm(400.0))
            .key(format!("{key}-edit"))
            .w_full()
            .min_h(60.0)
            .color(tk.fg)
            .caret(tk.fg, tk.primary.alpha(0.3))
            .label(t("assistant:message.edit"))
            .on_edit(|s: &mut App| &mut s.assistant.thread.editing.as_mut().expect("editing").1)
            // Enter confirms (Shift+Enter breaks the line), regenerating from the edit.
            .on_submit(|s: &mut App, _tx: &mut Tx| confirm_edit(s))
            .on_input_key(|s: &mut App, ev: &KeyEvent, _tx: &mut Tx| {
                if matches!(ev.key, Key::Escape) {
                    s.assistant.thread.editing = None;
                    return true;
                }
                false
            });
        return col().key(key).w_full().items_end().gap(tk.r(0.25)).child(
            col()
                .w_full()
                .max_w(width * if page { 0.72 } else { 0.82 })
                .child(
                    row()
                        .w_full()
                        .radius(tk.r(1.0))
                        .fill(message_fill(tk))
                        .border(1.0, tk.input)
                        .border_layout()
                        .px(tk.r(0.75))
                        .py(tk.r(0.5))
                        .child(field),
                )
                .child(
                    row()
                        .w_full()
                        .mt(tk.r(0.25))
                        .gap(tk.r(0.375))
                        .justify_end()
                        .child({
                            let b = round_action(
                                tk,
                                format!("asst-msg-{}-confirm", m.id),
                                &I::CHECK,
                                t("assistant:message.confirmEdit"),
                                can_confirm,
                            );
                            if can_confirm {
                                b.on_press(confirm_edit)
                            } else {
                                b
                            }
                        })
                        .child(
                            round_action(
                                tk,
                                format!("asst-msg-{}-cancel", m.id),
                                &I::X,
                                t("assistant:message.cancelEdit"),
                                true,
                            )
                            .on_press(|s: &mut App| s.assistant.thread.editing = None),
                        ),
                ),
        );
    }
    let can_collapse = m.content.chars().count() > COLLAPSE_CHARS
        || m.content.split('\n').count() > COLLAPSE_LINES;
    let expanded_key = format!("{key}-full");
    let collapsed = can_collapse && !super::is_open(app, &expanded_key, false);
    let mut c = col()
        .key(key.clone())
        .w_full()
        .items_end()
        .gap(tk.r(0.25))
        .hover_group();
    if !m.attachments.is_empty() {
        let mut files = row()
            .max_w(max_w(width))
            .flex_wrap()
            .justify_end()
            .gap(tk.r(0.375));
        for f in &m.attachments {
            files = files.child(crate::assistant::composer::attachment_card(
                tk, f, false, None, None,
            ));
        }
        c = c.child(files);
    }
    let mut body = text(m.content.clone())
        .font(tk.tsm(400.0))
        .color(tk.fg)
        .wrap()
        .selectable(true);
    let mut bubble = col()
        .max_w(max_w(width))
        .radius(tk.r(1.0))
        .fill(message_fill(tk))
        .padding(tk.r(0.75));
    if collapsed {
        body = body.max_h(tk.r(11.0));
        bubble = bubble.child(
            col()
                .w_full()
                .max_h(tk.r(11.0))
                .clip_rounded(0.0)
                .clip_fade_linear(0.0, 0.0, 0.0, tk.r(1.75))
                .child(body),
        );
    } else {
        bubble = bubble.child(body);
    }
    c = c.child(bubble);
    let can_edit = page && !m.content.is_empty();
    if can_collapse || can_edit {
        let mut actions = row().gap(tk.r(0.375)).center_y();
        if can_collapse {
            let label = if collapsed {
                t("assistant:message.showFullMessage")
            } else {
                t("assistant:message.showLess")
            };
            let k = expanded_key.clone();
            actions = actions.child(
                row()
                    .key(format!("{key}-collapse"))
                    .h(tk.r(1.5))
                    .px(tk.r(0.375))
                    .center_y()
                    .radius(tk.r(0.375))
                    .hover(if tk.dark {
                        tk.accent.alpha(0.5)
                    } else {
                        tk.accent
                    })
                    .role("button")
                    .expanded(!collapsed)
                    .label(label.clone())
                    .focusable(true)
                    .on_press(move |a: &mut App| super::toggle(a, &k, false))
                    .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg)),
            );
        }
        if can_edit {
            let id = m.id.clone();
            let content = m.content.clone();
            actions = actions.child(
                round_action(
                    tk,
                    format!("{key}-edit-btn"),
                    &I::PENCIL,
                    t("assistant:message.edit"),
                    true,
                )
                .group_opacity(0.0, 1.0)
                .on_press(move |a: &mut App| {
                    let mut e = TextEdit::new(&content);
                    e.focused = true;
                    e.set_caret(content.len());
                    a.assistant.thread.editing = Some((id.clone(), e));
                }),
            );
        }
        c = c.child(actions);
    }
    c
}

/// A work-log item (`workLogItems`): reasoning, one call that needs attention, or a run of
/// routine calls.
enum Item<'a> {
    Thinking(String, &'a str, usize),
    Call(&'a ToolCall),
    Tools(String, Vec<&'a ToolCall>),
}

fn needs_attention(c: &ToolCall) -> bool {
    matches!(
        c.status,
        ToolStatus::AwaitingConsent | ToolStatus::Error | ToolStatus::Denied
    ) || (c.status == ToolStatus::Pending && c.needs_consent)
        || matches!(
            c.approval.as_deref(),
            Some("auto" | "full_access" | "session")
        )
        || ([
            "ui.review.request_input",
            "ui.source_funds.request_input",
            "ui.wallets.request_input",
        ]
        .contains(&c.name.as_str())
            && c.status == ToolStatus::Done)
        || super::action::shows_applied(c)
}

/// `chatMessageRounds` + `workLogItems`.
/// The reasoning pane still being written (`ChatReasoning`'s active segment): the last round's,
/// while the answer streams and has no text yet. Its key.
pub(crate) fn active_thinking(m: &Msg) -> Option<String> {
    let streaming = matches!(m.status, MsgStatus::Streaming | MsgStatus::Pending);
    if !streaming || !m.content.is_empty() {
        return None;
    }
    let items = work_log(m);
    let n_rounds = items
        .iter()
        .filter_map(|i| {
            if let Item::Thinking(_, _, r) = i {
                Some(*r)
            } else {
                None
            }
        })
        .max()
        .map_or(0, |r| r + 1);
    items.iter().find_map(|i| match i {
        Item::Thinking(id, _, r) if *r + 1 == n_rounds => Some(format!("asst-think-{id}")),
        _ => None,
    })
}

fn work_log(m: &Msg) -> Vec<Item<'_>> {
    let segs: Vec<_> = m
        .thinking_segments
        .iter()
        .filter(|s| !s.content.is_empty())
        .collect();
    let mut rounds: Vec<(String, &str, Vec<&ToolCall>)> = if segs.is_empty() {
        if m.tool_calls.is_empty() {
            vec![]
        } else {
            vec![(format!("{}-round", m.id), "", m.tool_calls.iter().collect())]
        }
    } else {
        let mut r: Vec<_> = segs
            .iter()
            .map(|s| {
                (
                    s.id.clone(),
                    s.content.as_str(),
                    m.tool_calls
                        .iter()
                        .filter(|c| c.segment_id.as_deref() == Some(s.id.as_str()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let known: Vec<&str> = segs.iter().map(|s| s.id.as_str()).collect();
        let untagged: Vec<&ToolCall> = m
            .tool_calls
            .iter()
            .filter(|c| {
                c.segment_id
                    .as_deref()
                    .is_none_or(|id| !known.contains(&id))
            })
            .collect();
        let first = &mut r[0].2;
        let mut merged = untagged;
        merged.append(first);
        *first = merged;
        r
    };
    rounds.retain(|(_, th, calls)| !th.is_empty() || !calls.is_empty());
    let mut items: Vec<Item> = vec![];
    for (i, (id, th, calls)) in rounds.into_iter().enumerate() {
        if !th.is_empty() {
            items.push(Item::Thinking(id, th, i));
        }
        for c in calls {
            if needs_attention(c) {
                items.push(Item::Call(c));
                continue;
            }
            match items.last_mut() {
                Some(Item::Tools(_, list)) => list.push(c),
                _ => items.push(Item::Tools(format!("tools:{}", c.call_id), vec![c])),
            }
        }
    }
    items
}

/// `splitReviewCheckpoint`: a trailing ```json `{"review_checkpoint": …}` fence is shown as
/// "Continuation details", not prose.
pub fn split_checkpoint(content: &str) -> (&str, Option<&str>) {
    let start = match content.rfind("\n```json") {
        Some(i) => i + 1,
        None if content.starts_with("```json") => 0,
        None => return (content, None),
    };
    if content.len() - start > 16_384 {
        return (content, None);
    }
    let tail = &content[start + 7..];
    let Some(nl) = tail.find('\n') else {
        return (content, None);
    };
    if !tail[..nl].trim().is_empty() {
        return (content, None);
    }
    let rest = &tail[nl + 1..];
    let Some(end) = rest.rfind("\n```") else {
        return (content, None);
    };
    if !rest[end + 4..].trim().is_empty() {
        return (content, None);
    }
    let packet = &rest[..end];
    let ok = serde_json::from_str::<Value>(packet).ok().is_some_and(|v| {
        v.as_object().is_some_and(|o| o.len() == 1)
            && v["review_checkpoint"]
                .as_object()
                .is_some_and(|cp| !cp.is_empty() && cp.iter().all(|(k, f)| checkpoint_field(k, f)))
    });
    if ok {
        (&content[..start], Some(packet))
    } else {
        (content, None)
    }
}

fn checkpoint_field(k: &str, f: &Value) -> bool {
    match k {
        "workspace_id" | "profile_id" => f.as_str().is_some_and(|s| !s.is_empty()),
        "next_cursor" | "unapplied_plan_digest" => f.is_null() || f.is_string(),
        "input_version" => f.as_u64().is_some(),
        "receipt_ids" | "last_page_transaction_ids" => {
            let limit = if k == "receipt_ids" { 32 } else { 100 };
            f.as_array().is_some_and(|a| {
                a.len() <= limit && a.iter().all(|i| i.as_str().is_some_and(|s| !s.is_empty()))
            })
        }
        _ => false,
    }
}

/// `collectDeterministicFacts`: report figures the tools returned, as up to four small cards.
fn facts(m: &Msg) -> Vec<(String, String, Option<String>)> {
    let mut out = vec![];
    let mut seen: Vec<String> = vec![];
    let sat = |v: &Value| {
        format!(
            "{} sat",
            crate::format::count(v.as_f64().unwrap_or(0.0).max(0.0) as usize)
        )
    };
    for c in &m.tool_calls {
        let Some(env) = c.result.as_ref() else {
            continue;
        };
        let Some(kind) = env["kind"].as_str() else {
            continue;
        };
        let d = &env["data"];
        if !d.is_object() || seen.iter().any(|k| k == kind) {
            continue;
        }
        seen.push(kind.to_owned());
        let n = |v: &Value| v.as_f64().unwrap_or(0.0).max(0.0) as usize;
        match kind {
            "ui.report.blockers" => {
                let titles: Vec<&str> = d["blockers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|b| b["title"].as_str())
                    .take(2)
                    .collect();
                let label = if d["ready"].as_bool() == Some(true) {
                    t("assistant:facts.ready")
                } else {
                    tc(
                        "assistant:facts.blockers",
                        d["blockers"].as_array().map_or(0, Vec::len),
                        &[],
                    )
                };
                out.push((
                    t("assistant:facts.reportReadiness"),
                    label,
                    Some(titles.join(", ")).filter(|s| !s.is_empty()),
                ));
            }
            "ui.reports.summary" => {
                let first = d["asset_flow"].get(0);
                out.push((
                    t("assistant:facts.summary"),
                    tc(
                        "assistant:facts.activeTx",
                        n(&d["metrics"]["active_transactions"]),
                        &[],
                    ),
                    first.map(|f| {
                        tr(
                            "assistant:facts.assetFlow",
                            &[
                                (
                                    "asset",
                                    f["asset"].as_str().unwrap_or(&t("assistant:facts.asset")),
                                ),
                                ("inbound", &sat(&f["inbound_amount_sat"])),
                                ("outbound", &sat(&f["outbound_amount_sat"])),
                            ],
                        )
                    }),
                ));
            }
            "ui.reports.balance_sheet" | "ui.reports.portfolio_summary" => {
                let first = d["totals_by_asset"].get(0);
                out.push((
                    t(if kind == "ui.reports.balance_sheet" {
                        "assistant:facts.balanceSheet"
                    } else {
                        "assistant:facts.portfolio"
                    }),
                    tc(
                        "assistant:facts.assetTotals",
                        d["totals_by_asset"].as_array().map_or(0, Vec::len),
                        &[],
                    ),
                    first.map(|f| {
                        tr(
                            "assistant:facts.assetTotalDetail",
                            &[
                                (
                                    "asset",
                                    f["asset"].as_str().unwrap_or(&t("assistant:facts.asset")),
                                ),
                                ("quantity", &sat(&f["quantity_sat"])),
                            ],
                        )
                    }),
                ));
            }
            "ui.reports.tax_summary" => {
                let gl = d["rows"].get(0).and_then(|r| r["gain_loss"].as_f64());
                out.push((
                    t("assistant:facts.taxSummary"),
                    tc("assistant:facts.rows", n(&d["summary"]["row_count"]), &[]),
                    gl.map(|g| {
                        tr(
                            "assistant:facts.gainLoss",
                            &[("value", &visual::number(g, 2, 2))],
                        )
                    }),
                ));
            }
            "ui.rates.coverage" => out.push((
                t("assistant:facts.rateCoverage"),
                tc(
                    "assistant:facts.missingPrices",
                    n(&d["summary"]["missing_price_transactions"]),
                    &[],
                ),
                Some(tc(
                    "assistant:facts.coverableFromCache",
                    n(&d["summary"]["cache_coverable_missing"]),
                    &[],
                )),
            )),
            "ui.audit.changes_since_last_answer" => {
                let label = if d["status"].as_str() == Some("baseline_required") {
                    t("assistant:facts.baselineRequired")
                } else if d["changed"].as_bool() == Some(true) {
                    t("assistant:facts.changedSinceBaseline")
                } else {
                    t("assistant:facts.noChangesSinceBaseline")
                };
                out.push((t("assistant:facts.changeAudit"), label, None));
            }
            "ui.maintenance.run" => out.push((
                t("assistant:facts.maintenance"),
                if d["ready"].as_bool() == Some(true) {
                    t("assistant:facts.reportsReady")
                } else {
                    tc(
                        "assistant:facts.blockers",
                        d["blockers"].as_array().map_or(0, Vec::len),
                        &[],
                    )
                },
                d["sync_mode"]
                    .as_str()
                    .map(|m| tr("assistant:facts.syncMode", &[("mode", m)])),
            )),
            _ => {}
        }
    }
    out.truncate(4);
    out
}

/// `shortTime`: `Nov 14, 04:42 PM` / `14. Nov., 16:42` (UTC).
fn short_time(ts: &str) -> String {
    let Some(d) = crate::format::Day::parse(ts) else {
        return ts.to_owned();
    };
    let hm = ts.get(11..16).unwrap_or("00:00");
    let h: u32 = hm.get(0..2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mm = hm.get(3..5).unwrap_or("00");
    if crate::i18n::lang() == crate::i18n::Lang::De {
        format!("{}, {h:02}:{mm}", d.month_day_ui())
    } else {
        let (h12, ap) = match h {
            0 => (12, "AM"),
            1..=11 => (h, "AM"),
            12 => (12, "PM"),
            _ => (h - 12, "PM"),
        };
        format!("{}, {h12:02}:{mm}\u{202f}{ap}", d.month_day_ui())
    }
}

/// `provenanceParts`.
fn provenance(m: &Msg) -> Option<String> {
    let p = m.provenance.as_ref()?;
    let kinds: Vec<&str> = m
        .tool_calls
        .iter()
        .filter_map(|c| c.result.as_ref()?.get("kind")?.as_str())
        .collect();
    let n = |v: &Value| v.as_f64().map(|f| f.max(0.0) as usize);
    let mut parts = vec![];
    if let Some(used) = p["tools_used"].as_array().filter(|a| !a.is_empty()) {
        parts.push(tc("assistant:provenance.localTools", used.len(), &[]));
    }
    if !kinds.contains(&"ui.reports.summary")
        && let Some(c) = n(&p["active_transactions"])
    {
        parts.push(tc("assistant:provenance.activeTx", c, &[]));
    }
    if !kinds.contains(&"ui.journals.quarantine")
        && let Some(c) = n(&p["quarantines"])
    {
        parts.push(tc("assistant:provenance.quarantine", c, &[]));
    }
    if !kinds.contains(&"ui.rates.coverage")
        && let Some(c) = n(&p["missing_price_transactions"])
    {
        parts.push(tc("assistant:provenance.missingPrices", c, &[]));
    }
    let at = p["journals_processed_at"].as_str();
    if p["auto_journal_processed"].as_bool() == Some(true) {
        parts.push(match at {
            Some(a) => tr(
                "assistant:provenance.journalsRefreshedAt",
                &[("time", &short_time(a))],
            ),
            None => t("assistant:provenance.journalsRefreshed"),
        });
    } else if let Some(a) = at {
        parts.push(tr(
            "assistant:provenance.journalsAt",
            &[("time", &short_time(a))],
        ));
    }
    if p["auto_sync_attempted"].as_bool() == Some(true) {
        parts.push(t(if p["auto_sync_ok"].as_bool() == Some(false) {
            "assistant:provenance.syncFailed"
        } else {
            "assistant:provenance.syncChecked"
        }));
    }
    let r = &p["privacy_receipt"];
    if r.is_object() {
        parts.push(t(if r["remote_provider"].as_bool() == Some(true) {
            "assistant:provenance.remoteProvider"
        } else {
            "assistant:provenance.localProvider"
        }));
        if let Some(c) = n(&r["advertised_tool_count"]) {
            parts.push(tc("assistant:provenance.advertisedTools", c, &[]));
        }
        if let Some(c) = n(&r["tools_denied"]).filter(|c| *c > 0) {
            parts.push(tc("assistant:provenance.deniedTools", c, &[]));
        }
        if r["cross_book_data_disclosed"].as_bool() == Some(true) {
            parts.push(t("assistant:provenance.crossBookRead"));
        }
        if let Some(c) = n(&r["egress_records"]).filter(|c| *c > 0) {
            parts.push(tr(
                "assistant:provenance.egressReceipt",
                &[
                    ("records", &c.to_string()),
                    (
                        "bytes",
                        &crate::format::count(n(&r["egress_bytes_out"]).unwrap_or(0)),
                    ),
                ],
            ));
        }
    }
    if let Some(g) = p["generated_at"].as_str() {
        parts.push(tr(
            "assistant:provenance.answeredAt",
            &[("time", &short_time(g))],
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The assistant's message.
/// Read aloud (`speechSynthesis` in the web): the system voice on macOS (`say`, local only).
fn read_aloud(app: &App, tk: &Tk, key: &str, content: &str) -> El {
    if !cfg!(target_os = "macos") {
        return rune::el();
    }
    let speaking = app
        .assistant
        .speaking
        .as_ref()
        .is_some_and(|(k, _)| k == key);
    let label = t(if speaking {
        "assistant:message.stopReading"
    } else {
        "assistant:message.readAloud"
    });
    let (k, text_) = (key.to_owned(), content.to_owned());
    let glyph: &'static rune::Icon = if speaking { &I::SQUARE } else { &I::VOLUME_2 };
    round_action(tk, format!("{key}-speak"), glyph, label, true)
        .on_press(move |a: &mut App| crate::assistant::toggle_speech(a, &k, &text_))
}

/// `confirmEditing`: the edited prompt regenerates the conversation from there.
fn confirm_edit(s: &mut App) {
    let Some((id, e)) = s.assistant.thread.editing.take() else {
        return;
    };
    let trimmed = e.text.trim().to_owned();
    if trimmed.is_empty() {
        s.assistant.thread.editing = Some((id, e));
        return;
    }
    crate::assistant::chat::edit(s, &id, &trimmed);
}

pub fn assistant(app: &App, tk: &Tk, m: &Msg, _page: bool, layer: &mut Vec<El>) -> El {
    let key = format!("asst-msg-{}", m.id);
    let streaming = matches!(m.status, MsgStatus::Streaming | MsgStatus::Pending);
    let has_answer = !m.content.is_empty();
    let items = work_log(m);
    let n_rounds = items
        .iter()
        .filter_map(|i| {
            if let Item::Thinking(_, _, r) = i {
                Some(*r)
            } else {
                None
            }
        })
        .max()
        .map_or(0, |r| r + 1);
    let has_thinking = items.iter().any(|i| matches!(i, Item::Thinking(..)));
    let has_tools = !m.tool_calls.is_empty();
    // Tailwind's `sm:` is a viewport query (640 px), not the column's width: the panel's
    // visuals lay out as wide as the page's.
    let wide = app.shell.window_w <= 0.0 || app.shell.window_w >= 640.0;
    let mut c = col()
        .key(key.clone())
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.25))
        .py(2.0)
        .hover_group();
    for item in &items {
        c = c.child(match item {
            Item::Thinking(id, content, round) => tool::reasoning(
                app,
                tk,
                &format!("asst-think-{id}"),
                content,
                streaming && !has_answer && *round + 1 == n_rounds,
            ),
            Item::Call(call) => {
                let pending = (call.status == ToolStatus::AwaitingConsent)
                    .then(|| {
                        app.assistant
                            .pending_consents
                            .iter()
                            .find(|r| r.call_id == call.call_id)
                    })
                    .flatten();
                match pending {
                    Some(r) => approval::pending_row(app, tk, r, layer),
                    None => col()
                        .w_full()
                        .min_w(0.0)
                        .mb(tk.r(0.5))
                        .child(tool::card(app, tk, call)),
                }
            }
            Item::Tools(id, calls) => tool::group(app, tk, &format!("asst-group-{id}"), calls),
        });
    }
    // Review plans and receipts, once each.
    let mut seen: Vec<String> = vec![];
    let applied: Vec<String> = m
        .tool_calls
        .iter()
        .filter_map(approval::review_result)
        .filter(|(plan, _)| !plan)
        .filter_map(|(_, r)| r["artifact_digest"].as_str().map(str::to_owned))
        .collect();
    for (plan, v) in m.tool_calls.iter().filter_map(approval::review_result) {
        let id = if plan {
            format!("plan:{}", v["digest"])
        } else {
            format!("receipt:{}", v["id"])
        };
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        c = c.child(if plan {
            approval::proposal_card(
                app,
                tk,
                v,
                v["digest"]
                    .as_str()
                    .is_some_and(|d| applied.iter().any(|a| a == d)),
                wide,
            )
        } else {
            approval::receipt_card(app, tk, v)
        });
    }
    for v in m.tool_calls.iter().filter_map(visual::parse) {
        c = c.child(visual::card(app, tk, &v, wide));
    }
    for (id, source) in visual::pending(&m.tool_calls) {
        c = c.child(visual::skeleton(tk, &id, source.as_deref()));
    }
    let last_review = m
        .tool_calls
        .iter()
        .rfind(|c| c.name.starts_with("ui.review."));
    let review_pending =
        last_review.filter(|c| matches!(c.status, ToolStatus::Pending | ToolStatus::Running));
    if let Some(r) = review_pending {
        let k = match r.name.as_str() {
            "ui.review.plan" => "assistant:review.progress.plan",
            "ui.review.apply" => "assistant:review.progress.apply",
            _ => "assistant:review.progress.read",
        };
        c = c.child(
            text(t(k))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .my(tk.r(0.5))
                .role("status"),
        );
    }
    if let Some(f) =
        last_review.filter(|c| c.name == "ui.review.apply" && c.status == ToolStatus::Error)
    {
        c = c.child(
            row()
                .w_full()
                .my(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, tk.destructive.alpha(0.3))
                .border_layout()
                .padding(tk.r(0.75))
                .role("status")
                .child(
                    text(format!(
                        "{} {}",
                        t("assistant:review.failed"),
                        f.reason.clone().unwrap_or_default()
                    ))
                    .font(tk.tsm(400.0))
                    .color(tk.destructive)
                    .wrap(),
                ),
        );
    }
    if has_answer {
        let (prose, packet) = split_checkpoint(&m.content);
        let mut answer =
            col()
                .w_full()
                .min_w(0.0)
                .child(md::view(app, tk, &key, prose, md::Look::Answer));
        if has_thinking || has_tools {
            answer = answer.mt(tk.r(1.0));
        }
        if let Some(pk) = packet {
            let dk = format!("{key}-continuation");
            let open = super::is_open(app, &dk, false);
            let dk2 = dk.clone();
            let mut d = col().w_full().mt(tk.r(0.75)).child(
                row()
                    .key(format!("{dk}-summary"))
                    .gap(tk.r(0.25))
                    .py(tk.r(0.5))
                    .center_y()
                    .role("button")
                    .expanded(open)
                    .label(t("assistant:review.continuationDetails"))
                    .focusable(true)
                    .on_press(move |a: &mut App| super::toggle(a, &dk2, false))
                    .child(
                        ic(
                            if open {
                                &I::CHEVRON_DOWN
                            } else {
                                &I::CHEVRON_RIGHT
                            },
                            tk.r(0.75),
                            tk.muted_fg,
                        )
                        .a11y_hidden(),
                    )
                    .child(
                        text(t("assistant:review.continuationDetails"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg),
                    ),
            );
            if open {
                d = d.child(
                    super::nested_scroll(col().w_full().max_h(tk.r(12.0)))
                        .radius(tk.r(0.375))
                        .fill(tk.muted)
                        .padding(tk.r(0.75))
                        .child(
                            text(pk.to_owned())
                                .font(tk.txs(400.0).mono())
                                .color(tk.muted_fg)
                                .wrap_anywhere(),
                        ),
                );
            }
            answer = answer.child(d);
        }
        c = c.child(answer);
        let f = facts(m);
        if !f.is_empty() {
            let cards: Vec<El> = f
                .into_iter()
                .map(|(source, label, detail)| {
                    let mut card = col()
                        .grow(1.0)
                        .w(0.0)
                        .min_w(0.0)
                        .radius(tk.r(0.375))
                        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                        .border_layout()
                        .fill(tk.muted.alpha(0.2))
                        .px(tk.r(0.75))
                        .py(tk.r(0.5))
                        .child(
                            text(source.to_uppercase())
                                .font(tk.t2xs(500.0))
                                .color(tk.muted_fg),
                        )
                        .child(text(label).font(tk.tsm(500.0)).color(tk.fg).mt(2.0));
                    if let Some(d) = detail {
                        card = card.child(
                            text(d)
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .mt(2.0)
                                .wrap(),
                        );
                    }
                    card
                })
                .collect();
            let mut grid = col().w_full().mt(tk.r(0.75)).gap(tk.r(0.5));
            let per = if wide { 2 } else { 1 };
            let mut it = cards.into_iter().peekable();
            while it.peek().is_some() {
                let mut r = row().w_full().gap(tk.r(0.5));
                for _ in 0..per {
                    r = r.child(it.next().unwrap_or_else(|| rune::el().grow(1.0).w(0.0)));
                }
                grid = grid.child(r);
            }
            c = c.child(grid);
        }
    }
    if !m.notices.is_empty() {
        let mut ul = col().w_full().mt(tk.r(0.5)).gap(tk.r(0.25));
        for n in &m.notices {
            let k = format!("assistant:message.notice.{n}");
            ul = ul.child(
                row()
                    .gap(tk.r(0.375))
                    .center_y()
                    .role("status")
                    .child(
                        ic(
                            &I::ZAP,
                            tk.r(0.875),
                            rune::color::hex("#f59e0b").unwrap_or(tk.warn.solid),
                        )
                        .a11y_hidden(),
                    )
                    .child(
                        text(if crate::i18n::has(&k) {
                            t(&k)
                        } else {
                            n.clone()
                        })
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                    ),
            );
        }
        c = c.child(ul);
    }
    if let Some(p) = provenance(m) {
        c = c.child(
            text(p)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .mt(tk.r(0.5)),
        );
    }
    let awaiting = m
        .tool_calls
        .iter()
        .any(|c| c.status == ToolStatus::AwaitingConsent);
    if review_pending.is_none() && !awaiting && !has_answer && !has_thinking && streaming {
        let label = m
            .activity_label
            .clone()
            .unwrap_or_else(|| t("assistant:message.generating"));
        c = c.child(
            row()
                .mt(tk.r(0.25))
                .gap(tk.r(0.5))
                .center_y()
                .role("status")
                // ChatLoader hardcodes this English prefix.
                .label(format!("Assistant status: {label}"))
                .child(crate::txdetail::widgets::spinner(tk, tk.r(0.875)).a11y_hidden())
                .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg)),
        );
    }
    if m.status == MsgStatus::Error {
        let mut line = row().flex_wrap().gap(tk.r(0.5)).center_y().child(
            text(
                m.error_message
                    .clone()
                    .unwrap_or_else(|| t("assistant:message.chatFailed")),
            )
            .font(tk.tsm(400.0))
            .color(tk.destructive)
            .wrap(),
        );
        if let Some(code) = &m.error_code {
            line = line.child(
                row()
                    .radius(tk.r(0.375))
                    .fill(tk.destructive.alpha(0.1))
                    .px(tk.r(0.375))
                    .py(2.0)
                    .child(
                        text(code.to_uppercase())
                            .font(tk.t2xs(400.0).mono())
                            .color(tk.destructive),
                    ),
            );
        }
        c = c.child(line);
    }
    if m.status == MsgStatus::Cancelled {
        c = c.child(
            text(t("assistant:message.stoppedByUser"))
                .font(tk.txs(400.0).italic())
                .color(tk.muted_fg)
                .mt(tk.r(0.25)),
        );
    }
    if has_answer && !streaming {
        let copy_key = format!("{key}-copy");
        let copied = app
            .assistant
            .thread
            .copied
            .as_ref()
            .is_some_and(|(k, until)| *k == copy_key && *until > app.clock);
        let content = m.content.clone();
        let mut actions = row()
            .mt(tk.r(0.375))
            .gap(tk.r(0.375))
            .center_y()
            .group_opacity(0.0, 1.0)
            .child(
                round_action(
                    tk,
                    copy_key.clone(),
                    if copied { &I::CHECK } else { &I::COPY },
                    t(if copied {
                        "assistant:message.copied"
                    } else {
                        "assistant:message.copy"
                    }),
                    true,
                )
                .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                    crate::settings::effects::copy(a, tx, content.clone());
                    super::mark_copied(a, tx, &copy_key);
                }),
            );
        actions = actions.child(read_aloud(app, tk, &key, &m.content));
        let id = m.id.clone();
        actions = actions.child(
            round_action(
                tk,
                format!("{key}-branch"),
                &I::SPLIT,
                t("assistant:message.branch"),
                true,
            )
            .on_press(move |a: &mut App| crate::assistant::chat::branch(a, &id)),
        );
        c = c.child(actions);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_split_only_when_valid() {
        let ok = "Done.\n```json\n{\"review_checkpoint\": {\"workspace_id\": \"w\", \"input_version\": 3}}\n```\n";
        assert_eq!(
            split_checkpoint(ok),
            (
                "Done.\n",
                Some("{\"review_checkpoint\": {\"workspace_id\": \"w\", \"input_version\": 3}}")
            )
        );
        let other = "Done.\n```json\n{\"foo\": 1}\n```";
        assert_eq!(split_checkpoint(other), (other, None));
        assert_eq!(split_checkpoint("plain"), ("plain", None));
    }

    #[test]
    fn short_times_follow_the_language() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(
            short_time("2026-11-14T16:42:00Z"),
            "Nov 14, 04:42\u{202f}PM"
        );
    }
}
