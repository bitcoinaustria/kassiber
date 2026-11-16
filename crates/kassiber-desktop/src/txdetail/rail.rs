//! The right rail (TransactionDetailAttachmentsPanel.tsx, TransactionEditHistoryPanel.tsx) and
//! the Commercial provenance panel (TransactionDetailCommercialPanel.tsx).

use rune::{El, Tx, col, el, row, text};
use rune_kit::TextInput;
use serde_json::Value;

use super::Open;
use super::actions::{attachment_id, rail_label};
use super::draft::{opt_s, s};
use super::widgets::{inset, muted_badge, small_button};
use crate::app::App;
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

fn detail_of(a: &Value) -> String {
    let is_url = super::actions::is_url(a);
    if is_url {
        let url = s(a, "url").to_owned();
        let label = rail_label(a);
        let host = url
            .split("://")
            .nth(1)
            .and_then(|r| r.split('/').next())
            .unwrap_or("")
            .to_owned();
        if url == label
            || (opt_s(a, "label").is_none()
                && (label == host || label.starts_with(&format!("{host} - "))))
        {
            String::new()
        } else {
            url
        }
    } else {
        let mut parts = vec![];
        if let Some(m) = opt_s(a, "media_type") {
            parts.push(m);
        }
        if let Some(sz) = a
            .get("size_bytes")
            .or_else(|| a.get("size"))
            .and_then(Value::as_f64)
        {
            parts.push(match sz {
                x if x >= 1_048_576.0 => format!("{:.1} MB", x / 1_048_576.0),
                x if x >= 1024.0 => format!("{:.1} KB", x / 1024.0),
                x => format!("{x} B"),
            });
        }
        if let Some(h) = opt_s(a, "sha256") {
            parts.push(format!("sha256 {}...", &h[..h.len().min(12)]));
        }
        if a.get("exists").and_then(Value::as_bool) == Some(false) {
            parts.push(t("transactions:attachment.missingFile"));
        }
        parts.join(" · ")
    }
}

pub fn attachments(app: &App, tk: &Tk, o: &Open) -> El {
    let hide = app.prefs.hide_sensitive();
    let list = o.attachments.clone().unwrap_or_default();
    let reuse_ok = app
        .snapshot()
        .is_some_and(|sn| sn.txs.len() + sn.activity_txs.len() > 1);
    // `kb-surface-inset p-3`; header `mb-2 justify-between gap-2`: Paperclip (1 rem, muted), the
    // title (`text-sm font-semibold`, `gap-2`) and its InfoHint; the count (`text-xs`) and the
    // reuse button (`outline icon size-7`, Copy 0.875 rem) at the end.
    let title = t("transactions:attachments.title");
    let mut tail = row().gap(tk.r(0.375)).center_y().child(
        text(list.len().to_string())
            .font(tk.txs(400.0).features("'tnum' 1"))
            .color(tk.muted_fg),
    );
    if reuse_ok {
        tail = tail.child(
            super::widgets::icon_button(
                tk,
                "txd-reuse",
                super::widgets::Variant::Outline,
                tk.r(1.75),
                &I::COPY,
                tk.r(0.875),
                t("transactions:attachments.reuseEvidenceAria"),
            )
            .on_press(|a: &mut App| {
                let c = super::dialogs::reuse_candidates(a);
                super::actions::open_reuse(a, &c);
            }),
        );
    }
    let head = row()
        .w_full()
        .mb(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(ic(&I::PAPERCLIP, tk.r(1.0), tk.muted_fg))
                .child(text(title.clone()).font(tk.tsm(600.0)).color(tk.fg))
                .child(super::widgets::hint_named(
                    tk,
                    title.clone(),
                    t("transactions:attachments.emptyBody"),
                    tk.fg,
                )),
        )
        .child(tail);
    let mut c = super::widgets::surface_inset(tk)
        .key("txd-attachments")
        .padding(tk.r(0.75))
        .child(head);
    let mut items = col().w_full().gap(tk.r(0.375));
    for a in &list {
        let id = attachment_id(a);
        let label = rail_label(a);
        let is_url = super::actions::is_url(a);
        let renaming = o.rename.as_ref().filter(|r| r.0 == id);
        let row_el: El = if let Some((_, err)) = renaming {
            let k = tk.kit();
            let input = TextInput::new(&app.detail.rename)
                .label(tr(
                    "transactions:attachments.linkTextAria",
                    &[("name", &label)],
                ))
                .height(tk.r(1.75))
                .text_style(tk.txs(400.0))
                .on_edit(|a: &mut App| &mut a.detail.rename)
                .on_submit(|a: &mut App, _: &mut Tx| super::actions::commit_rename(a))
                .on_key(|a: &mut App, ev: &rune::KeyEvent, _: &mut Tx| {
                    if matches!(ev.key, rune::Key::Escape) {
                        if let Some(o) = a.detail.open.as_mut() {
                            o.rename = None;
                        }
                        return true;
                    }
                    false
                })
                .build(&k)
                .key("txd-rename-input")
                .grow(1.0);
            let mut r = col().w_full().gap(tk.r(0.25)).child(
                row()
                    .w_full()
                    .gap(tk.r(0.25))
                    .center_y()
                    .child(input)
                    .child(
                        super::widgets::icon_btn(
                            tk,
                            "txd-rename-save",
                            &I::CHECK,
                            t("transactions:attachments.saveLinkText"),
                            tk.r(1.75),
                        )
                        .on_press(super::actions::commit_rename),
                    )
                    .child(
                        super::widgets::icon_btn(
                            tk,
                            "txd-rename-cancel",
                            &I::X,
                            t("transactions:attachments.cancelEdit"),
                            tk.r(1.75),
                        )
                        .on_press(|a: &mut App| {
                            if let Some(o) = a.detail.open.as_mut() {
                                o.rename = None;
                            }
                        }),
                    ),
            );
            if let Some(e) = err {
                r = r.child(
                    text(e.clone())
                        .font(tk.t2xs(400.0))
                        .color(tk.destructive)
                        .role("alert"),
                );
            }
            r
        } else {
            let open_id = id.clone();
            let detail = detail_of(a);
            let mut info = col()
                .grow(1.0)
                .min_w(0.0)
                .key(format!("txd-att-open-{id}"))
                .role("link")
                .label(label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| super::actions::open_attachment(a, &open_id))
                .child({
                    let e = super::widgets::sens(tk, label.clone(), tk.txs(500.0), tk.fg, hide)
                        .truncate();
                    if hide {
                        e
                    } else {
                        e.help_when_truncated(label.clone())
                    }
                });
            if !detail.is_empty() {
                let e = super::widgets::sens(tk, detail.clone(), tk.t2xs(400.0), tk.muted_fg, hide)
                    .truncate();
                info = info.child(if hide {
                    e
                } else {
                    e.help_when_truncated(detail)
                });
            }
            if hide {
                info = info.help(t("transactions:attachments.detailHidden"));
            }
            let tile = row()
                .size(tk.r(1.5), tk.r(1.5))
                .center()
                .radius(tk.r(0.375))
                .fill(tk.muted)
                .child(ic(
                    if is_url { &I::LINK_2 } else { &I::FILE_TEXT },
                    tk.r(0.875),
                    tk.muted_fg,
                ));
            let mut r = row()
                .w_full()
                .min_w(0.0)
                .gap(tk.r(0.5))
                .center_y()
                .px(tk.r(0.5))
                .py(tk.r(0.375))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .fill(tk.background)
                .child(tile)
                .child(info);
            if opt_s(a, "copied_from_attachment_id").is_some() {
                r = r.child(muted_badge(tk, &t("transactions:attachments.reused")));
            }
            if is_url && !hide {
                let (rid, cur) = (id.clone(), label.clone());
                r = r.child(
                    super::widgets::icon_btn(
                        tk,
                        format!("txd-att-rename-{id}"),
                        &I::PENCIL,
                        tr(
                            "transactions:attachments.editLinkTextAria",
                            &[("name", &label)],
                        ),
                        tk.r(1.5),
                    )
                    .on_press(move |a: &mut App| super::actions::start_rename(a, &rid, &cur)),
                );
            }
            let rm = id.clone();
            let rm_label = if hide {
                t("transactions:attachments.removeAttachment")
            } else {
                tr("transactions:attachments.removeNamed", &[("name", &label)])
            };
            r.child(
                super::widgets::icon_btn(
                    tk,
                    format!("txd-att-remove-{id}"),
                    &I::X,
                    rm_label,
                    tk.r(1.5),
                )
                .on_press(move |a: &mut App| super::actions::remove_attachment(a, &rm)),
            )
        };
        items = items.child(row_el);
    }
    if !list.is_empty() {
        c = c.child(items.mb(tk.r(0.5)));
    }
    if let Some(e) = &o.attach_error {
        c = c.child(
            text(e.clone())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert")
                .mb(tk.r(0.5)),
        );
    }
    // `grid grid-cols-2 gap-1.5` of outline `sm` buttons with 0.875 rem icons.
    let picking = o.busy.attach;
    let mut attach = super::widgets::button_with(
        tk,
        "txd-attach-files",
        super::widgets::Variant::Outline,
        super::widgets::Size::Sm,
        Some(&I::FILE_TEXT),
        Some(tk.r(0.875)),
        &if picking {
            t("transactions:attachments.picking")
        } else {
            t("transactions:attachments.attachFiles")
        },
        !picking,
    )
    .grow(1.0)
    .justify_center();
    if !picking {
        attach = attach.on_press_tx(super::actions::attach_file);
    }
    let links = super::widgets::button_with(
        tk,
        "txd-attach-links",
        super::widgets::Variant::Outline,
        super::widgets::Size::Sm,
        Some(&I::LINK_2),
        Some(tk.r(0.875)),
        &t("transactions:attachments.attachLinks"),
        true,
    )
    .grow(1.0)
    .justify_center()
    .on_press(|a: &mut App| {
        if let Some(o) = a.detail.open.as_mut() {
            o.links_open = true;
            o.links_error = None;
        }
        a.detail.links.focused = true;
    });
    c.child(row().w_full().gap(tk.r(0.375)).child(attach).child(links))
}

fn source_badge(tk: &Tk, source: &str, label: &str) -> El {
    let (glyph, tone): (&'static rune::Icon, crate::theme::Tone) = match source {
        "ai_tool" => (&I::BOT, tk.sky),
        "gui" => (&I::MONITOR, tk.good),
        _ => (&I::SQUARE_TERMINAL, crate::ui::neutral_tone(tk)),
    };
    row()
        .h(tk.r(1.25))
        .px(tk.r(0.375))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .fill(tone.bg)
        .border(1.0, tone.border)
        .child(ic(glyph, tk.r(0.625), tone.text))
        .child(text(label.to_owned()).font(tk.t2xs(500.0)).color(tone.text))
}

fn family_badge(tk: &Tk, fam: &str) -> El {
    let tone = match fam {
        "pricing" => tk.warn,
        "tax" => crate::theme::Tone {
            bg: rune::color::hex("#8b5cf6").unwrap().alpha(0.12),
            border: rune::color::hex("#8b5cf6").unwrap().alpha(0.3),
            text: rune::color::hex(if tk.dark { "#c4b5fd" } else { "#6d28d9" }).unwrap(),
            solid: rune::color::hex("#8b5cf6").unwrap(),
        },
        _ => crate::ui::neutral_tone(tk),
    };
    row()
        .h(tk.r(1.25))
        .px(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .fill(tone.bg)
        .border(1.0, tone.border)
        .child(text(fam.to_owned()).font(tk.t2xs(500.0)).color(tone.text))
}

pub fn history(app: &App, tk: &Tk, o: &Open) -> El {
    let hide = app.prefs.hide_sensitive();
    let h = o.history.clone().unwrap_or(Value::Null);
    let events = h["events"].as_array().cloned().unwrap_or_default();
    let stale = h["stale"]["edit_count"].as_u64().unwrap_or(0) as usize;
    let open = o.history_open;
    // `<details kb-surface-inset>`: the summary (`p-3 justify-between gap-2`) names the panel
    // (History 1 rem, `text-sm font-semibold`), then the stale and event counts (only when there
    // are any) and a chevron that turns as it opens.
    let mut summary = row()
        .key("txd-history-toggle")
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .role("button")
        .expanded(open)
        .label(t("transactions:history.panelTitle"))
        .focusable(true)
        .on_press(|a: &mut App| {
            if let Some(o) = a.detail.open.as_mut() {
                o.history_open = !o.history_open;
            }
        })
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .grow(1.0)
                .child(ic(&I::HISTORY, tk.r(1.0), tk.muted_fg))
                .child(
                    text(t("transactions:history.panelTitle"))
                        .font(tk.tsm(600.0))
                        .color(tk.fg),
                ),
        );
    let mut ends = row().gap(tk.r(0.375)).center_y();
    if stale > 0 {
        ends = ends.child(
            super::widgets::badge(
                tk,
                &stale.to_string(),
                tk.warn.text,
                tk.warn.bg,
                tk.warn.border,
            )
            .help(tc("transactions:history.staleCount", stale, &[])),
        );
    }
    if !events.is_empty() {
        ends = ends.child(
            row()
                .px(tk.r(0.5))
                .py(tk.r(0.125))
                .radius(tk.r(0.375))
                .fill(tk.muted)
                .child(
                    text(events.len().to_string())
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                ),
        );
    }
    summary = summary.child(ends.child(ic(
        if open {
            &I::CHEVRON_DOWN
        } else {
            &I::CHEVRON_RIGHT
        },
        tk.r(0.875),
        tk.muted_fg,
    )));
    let shell = super::widgets::surface_inset(tk)
        .key("txd-history")
        .child(summary);
    if !open {
        return shell;
    }
    // The body: `border-t p-3`.
    let mut c = col()
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
    if stale > 0 {
        let when = h["stale"]["latest_changed_at"]
            .as_str()
            .map(|w| format::short_date(Some(w)))
            .unwrap_or_else(|| t("transactions:history.reportsNeedFreshState"));
        let busy = o.busy.process || app.busy.processing;
        let mut btn = small_button(
            tk,
            "txd-history-process",
            Some(&I::REFRESH_CW),
            &t("transactions:history.process"),
            true,
            !busy,
        );
        if !busy {
            btn = btn.on_press(super::actions::process_journals);
        }
        c = c.child(
            row()
                .w_full()
                .padding(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.375))
                .fill(tk.warn.bg)
                .border(1.0, tk.warn.border)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(tc("transactions:history.staleCount", stale, &[]))
                                .font(tk.txs(600.0))
                                .color(tk.warn.text),
                        )
                        .child(
                            row()
                                .gap(tk.r(0.25))
                                .center_y()
                                .child(ic(&I::CLOCK, tk.r(0.75), tk.muted_fg))
                                .child(text(when).font(tk.t2xs(400.0)).color(tk.muted_fg)),
                        ),
                )
                .child(btn),
        );
    }
    // Loading and empty: `rounded-md border border-dashed bg-muted/30 p-3 text-xs`.
    let dashed = |s: String| {
        row()
            .w_full()
            .padding(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_dash(3.0, 2.0)
            .fill(tk.muted.alpha(0.3))
            .child(text(s).font(tk.txs(400.0)).color(tk.muted_fg))
    };
    if o.history.is_none() {
        return shell.child(c.child(dashed(t("transactions:history.loading"))));
    }
    if events.is_empty() {
        return shell.child(c.child(dashed(t("transactions:history.emptyTimeline"))));
    }
    let now = format::now_secs();
    for e in events {
        let eid = s(&e, "id").to_owned();
        let expanded = o.expanded.contains(&eid);
        let toggle_id = eid.clone();
        let when = s(&e, "changed_at");
        let rel = format::relative(when, now).unwrap_or_else(|| "Unknown".into());
        let mut head = row()
            .key(format!("txd-hist-{eid}"))
            .w_full()
            .gap(tk.r(0.375))
            .center_y()
            .flex_wrap()
            .role("button")
            .expanded(expanded)
            .label(s(&e, "summary").to_owned())
            .focusable(true)
            .on_press(move |a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    if let Some(i) = o.expanded.iter().position(|x| *x == toggle_id) {
                        o.expanded.remove(i);
                    } else {
                        o.expanded.push(toggle_id.clone());
                    }
                }
            })
            .child(ic(
                if expanded {
                    &I::CHEVRON_DOWN
                } else {
                    &I::CHEVRON_RIGHT
                },
                tk.r(0.75),
                tk.muted_fg,
            ))
            .child(source_badge(tk, s(&e, "source"), s(&e, "source_label")));
        for fam in e["families"].as_array().cloned().unwrap_or_default() {
            head = head.child(family_badge(tk, fam.as_str().unwrap_or("")));
        }
        if e["report_anchor"]["stale_for_reports"].as_bool() == Some(true) {
            head = head.child(super::widgets::badge(
                tk,
                &t("transactions:history.staleReports"),
                tk.warn.text,
                tk.warn.bg,
                tk.warn.border,
            ));
        }
        let rev_id = eid.clone();
        let all_label = t("transactions:history.confirmRevertAll");
        head = head
            .child(el().grow(1.0))
            .child(
                text(rel)
                    .font(tk.t2xs(400.0))
                    .color(tk.muted_fg)
                    .help(format::short_date(Some(when))),
            )
            .child(
                super::widgets::icon_btn(
                    tk,
                    format!("txd-revert-{eid}"),
                    &I::ROTATE_CCW,
                    t("transactions:history.revertEditAria"),
                    tk.r(1.5),
                )
                .on_press(move |a: &mut App| {
                    if let Some(o) = a.detail.open.as_mut() {
                        o.revert_confirm = Some((rev_id.clone(), None, all_label.clone()));
                    }
                }),
            );
        let mut item = col()
            .w_full()
            .gap(tk.r(0.25))
            .py(tk.r(0.375))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .child(head)
            .child(
                super::widgets::sens(tk, s(&e, "summary").to_owned(), tk.txs(400.0), tk.fg, hide)
                    .wrap(),
            );
        if let Some(rsn) = opt_s(&e, "reason") {
            item = item.child(text(rsn).font(tk.t2xs(400.0)).color(tk.muted_fg).wrap());
        }
        if expanded {
            for fld in e["fields"].as_array().cloned().unwrap_or_default() {
                let label = s(&fld, "label").to_owned();
                let field_id = s(&fld, "field").to_owned();
                let (ev, lb) = (eid.clone(), label.clone());
                let mut diff = row().gap(tk.r(0.25)).flex_wrap();
                for add in fld["diff"]["added"].as_array().cloned().unwrap_or_default() {
                    diff = diff.child(
                        text(format!("+{}", add.as_str().unwrap_or("")))
                            .font(tk.t2xs(500.0))
                            .color(tk.good.text),
                    );
                }
                for rm in fld["diff"]["removed"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    diff = diff.child(
                        text(format!("-{}", rm.as_str().unwrap_or("")))
                            .font(tk.t2xs(500.0))
                            .color(tk.alert.text),
                    );
                }
                let boxv = |cap: String, v: &str| {
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .padding(tk.r(0.375))
                        .radius(tk.r(0.25))
                        .fill(tk.muted.alpha(0.4))
                        .child(text(cap).font(tk.t2xs(500.0)).color(tk.muted_fg))
                        .child(
                            super::widgets::sens(tk, v.to_owned(), tk.txs(400.0), tk.fg, hide)
                                .wrap(),
                        )
                };
                let mut f = col()
                    .w_full()
                    .gap(tk.r(0.25))
                    .padding(tk.r(0.375))
                    .child(
                        row()
                            .w_full()
                            .gap(tk.r(0.375))
                            .center_y()
                            .child(family_badge(tk, s(&fld, "family")))
                            .child(
                                text(label.clone())
                                    .font(tk.txs(500.0))
                                    .color(tk.fg)
                                    .grow(1.0),
                            )
                            .child(
                                super::widgets::icon_btn(
                                    tk,
                                    format!("txd-revert-{eid}-{field_id}"),
                                    &I::ROTATE_CCW,
                                    tr(
                                        "transactions:history.revertFieldAria",
                                        &[("label", &label)],
                                    ),
                                    tk.r(1.5),
                                )
                                .on_press(move |a: &mut App| {
                                    if let Some(o) = a.detail.open.as_mut() {
                                        o.revert_confirm =
                                            Some((ev.clone(), Some(field_id.clone()), lb.clone()));
                                    }
                                }),
                            ),
                    )
                    .child(
                        row()
                            .w_full()
                            .gap(tk.r(0.375))
                            .child(boxv(
                                t("transactions:history.before"),
                                s(&fld, "before_label"),
                            ))
                            .child(boxv(
                                t("transactions:history.after"),
                                s(&fld, "after_label"),
                            )),
                    )
                    .child(diff);
                if fld["redacted"].as_bool() == Some(true) {
                    f = f.child(
                        text(t("transactions:history.redacted"))
                            .font(tk.t2xs(400.0))
                            .color(tk.muted_fg),
                    );
                }
                item = item.child(f);
            }
        }
        c = c.child(item);
    }
    shell.child(c)
}

// ---- Commercial provenance (§5.4) ---------------------------------------------------------------

fn compact(id: &str) -> String {
    if id.chars().count() > 24 {
        let head: String = id.chars().take(12).collect();
        let tail: String = id
            .chars()
            .rev()
            .take(10)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}...{tail}")
    } else {
        id.to_owned()
    }
}

/// Hidden entirely when the row has no BTCPay match.
pub fn commercial(app: &App, tk: &Tk, o: &Open) -> Option<El> {
    let data = o.commercial.as_ref()?;
    let matches = data["btcpay"].as_array().cloned().unwrap_or_default();
    if matches.is_empty() {
        return None;
    }
    let _ = app;
    let mut c = inset(tk).key("txd-commercial").child(
        text(t("transactions:commercial.title"))
            .font(tk.txs(600.0))
            .color(tk.fg),
    );
    // Freshness note from the least current store (never synced < error < stale < fresh).
    let mut fresh = data["btcpay_freshness"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    fresh.sort_by_key(|e| {
        if e["never_synced"].as_bool() == Some(true) {
            0
        } else if !e["last_error_code"].is_null() {
            1
        } else if e["stale"].as_bool() == Some(true) {
            2
        } else {
            3
        }
    });
    if let Some(fr) = fresh.first() {
        let when = format::short_date(fr["last_success_at"].as_str());
        let (key, warn) = if fr["never_synced"].as_bool() == Some(true) {
            ("transactions:commercial.freshness.never", true)
        } else if !fr["last_error_code"].is_null() {
            ("transactions:commercial.freshness.failed", true)
        } else if fr["stale"].as_bool() == Some(true) {
            ("transactions:commercial.freshness.stale", true)
        } else {
            ("transactions:commercial.freshness.asOf", false)
        };
        c = c.child(
            text(tr(key, &[("when", &when)]))
                .font(tk.t2xs(400.0))
                .color(if warn { tk.warn.text } else { tk.muted_fg })
                .wrap(),
        );
    }
    for (i, m) in matches.iter().enumerate() {
        let link_rec = &m["link"];
        let payment = m.get("payment").filter(|p| p.is_object());
        let payout = payment.is_some_and(|p| s(p, "record_type") == "payout");
        let reviewed = s(link_rec, "state") == "reviewed";
        let pay_id = payment
            .and_then(|p| opt_s(p, "payment_id"))
            .or_else(|| opt_s(link_rec, "payment_id"))
            .map(|x| compact(&x))
            .unwrap_or_else(|| t("transactions:commercial.linked"));
        let mut rows = col().w_full().gap(tk.r(0.25)).child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .child(
                    text(t(if payout {
                        "transactions:commercial.btcpayPayout"
                    } else {
                        "transactions:commercial.btcpayPayment"
                    }))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .grow(1.0),
                )
                .child(text(pay_id).font(tk.txs(500.0).mono()).color(if reviewed {
                    tk.fg
                } else {
                    tk.muted_fg
                })),
        );
        let inv_key = if payout {
            "transactions:commercial.refundedInvoice"
        } else if payment.is_some() {
            "transactions:commercial.paidInvoice"
        } else {
            "transactions:commercial.invoice"
        };
        let inv_id = m
            .get("invoice")
            .and_then(|v| opt_s(v, "invoice_id"))
            .or_else(|| opt_s(link_rec, "invoice_id"))
            .unwrap_or_else(|| t("transactions:commercial.unknown"));
        rows = rows.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .child(
                    text(t(inv_key))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .grow(1.0),
                )
                .child(text(inv_id).font(tk.txs(500.0).mono()).color(tk.fg)),
        );
        if let Some(pr) = m.get("payment_request").filter(|v| v.is_object()) {
            rows = rows.child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .child(
                        text(t("transactions:commercial.paymentRequest"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .grow(1.0),
                    )
                    .child(
                        text(opt_s(pr, "label").unwrap_or_else(|| s(pr, "id").to_owned()))
                            .font(tk.txs(500.0))
                            .color(tk.fg),
                    ),
            );
        }
        if let Some(or) = m.get("origin").filter(|v| v.is_object()) {
            let kind = opt_s(or, "kind").unwrap_or_else(|| "unknown".into());
            let camel: String = kind
                .split('_')
                .enumerate()
                .map(|(i, w)| {
                    if i == 0 {
                        w.to_owned()
                    } else {
                        w[..1].to_uppercase() + &w[1..]
                    }
                })
                .collect();
            let key = format!("transactions:commercial.originLabel.{camel}");
            let mut lbl = if crate::i18n::has(&key) {
                t(&key)
            } else {
                t("transactions:commercial.originLabel.unknown")
            };
            if let Some(l) = opt_s(or, "label") {
                lbl = format!("{lbl} · {l}");
            }
            rows = rows.child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .child(
                        text(t("transactions:commercial.origin"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .grow(1.0),
                    )
                    .child(text(lbl).font(tk.txs(500.0)).color(tk.fg)),
            );
        }
        let mut badges = row().gap(tk.r(0.25)).flex_wrap();
        if !reviewed && let Some(ls) = opt_s(link_rec, "state") {
            badges = badges.child(muted_badge(
                tk,
                &t(&format!("transactions:commercial.linkState.{ls}")),
            ));
        }
        if let Some(rs) = opt_s(link_rec, "reconciliation_state").filter(|x| x != "unreviewed") {
            badges = badges.child(muted_badge(
                tk,
                &t(&format!("transactions:commercial.reconciliationState.{rs}")),
            ));
        }
        if let Some(ck) = opt_s(link_rec, "commercial_kind") {
            badges = badges.child(muted_badge(
                tk,
                &t(&format!("transactions:commercial.commercialKind.{ck}")),
            ));
        }
        rows = rows.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .child(
                    text(t("transactions:commercial.reconciliation"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .grow(1.0),
                )
                .child(badges),
        );
        // Review actions.
        let link = s(link_rec, "id").to_owned();
        let busy = o.busy.commercial;
        if reviewed {
            let l2 = link.clone();
            let mut b = small_button(
                tk,
                format!("txd-com-reopen-{i}"),
                None,
                &t("transactions:commercial.review.reopen"),
                true,
                !busy,
            );
            if !busy {
                b = b.on_press(move |a: &mut App| {
                    super::actions::commercial_review(a, &l2, "suggested", None)
                });
            }
            rows = rows
                .child(
                    text(t("transactions:commercial.review.reopenEffect"))
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(row().child(b));
        } else {
            let default = if payout {
                if payment.is_some_and(|p| s(p, "origin_kind") == "refund") {
                    "refund"
                } else {
                    "expense"
                }
            } else {
                "income"
            };
            let kind = o.commercial_kind.clone().unwrap_or_else(|| default.into());
            let opts: Vec<&str> = if payout {
                vec!["refund", "expense", "transfer", "none"]
            } else {
                vec!["income", "transfer", "none"]
            };
            let mut sel = row()
                .gap(tk.r(0.25))
                .flex_wrap()
                .role("radiogroup")
                .label(t("transactions:commercial.review.kindLabel"));
            for k in opts {
                let k2 = k.to_owned();
                sel = sel.child(
                    super::widgets::badge(
                        tk,
                        &t(&format!("transactions:commercial.commercialKind.{k}")),
                        if kind == k { tk.primary_fg } else { tk.fg },
                        if kind == k { tk.primary } else { tk.background },
                        tk.border,
                    )
                    .key(format!("txd-com-kind-{i}-{k}"))
                    .role("radio")
                    .checked(kind == k)
                    .focusable(true)
                    .on_press(move |a: &mut App| {
                        if let Some(o) = a.detail.open.as_mut() {
                            o.commercial_kind = Some(k2.clone());
                        }
                    }),
                );
            }
            let (l2, l3, k2) = (link.clone(), link.clone(), kind.clone());
            let mut confirm = small_button(
                tk,
                format!("txd-com-confirm-{i}"),
                Some(&I::CHECK),
                &t("transactions:commercial.review.confirm"),
                true,
                !busy,
            );
            let mut reject = small_button(
                tk,
                format!("txd-com-reject-{i}"),
                None,
                &t("transactions:commercial.review.reject"),
                false,
                !busy,
            );
            if !busy {
                confirm = confirm.on_press(move |a: &mut App| {
                    super::actions::commercial_review(a, &l2, "reviewed", Some(&k2))
                });
                reject = reject.on_press(move |a: &mut App| {
                    super::actions::commercial_review(a, &l3, "rejected", None)
                });
            }
            let effect = if kind == "none" {
                t("transactions:commercial.review.effectKeepKind")
            } else {
                tr(
                    "transactions:commercial.review.effectKind",
                    &[(
                        "kind",
                        &t(&format!("transactions:commercial.commercialKind.{kind}")),
                    )],
                )
            };
            rows = rows
                .child(sel)
                .child(text(effect).font(tk.t2xs(400.0)).color(tk.muted_fg).wrap())
                .child(row().gap(tk.r(0.375)).child(confirm).child(reject));
        }
        c = c.child(
            rows.pt(tk.r(0.25))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border),
        );
    }
    if let Some(e) = &o.commercial_error {
        c = c.child(
            text(e.clone())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    Some(c)
}
