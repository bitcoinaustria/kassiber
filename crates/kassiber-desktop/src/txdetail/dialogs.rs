//! Dialogs of the detail panel: Open explorer (ExplorerOpenDialog.tsx), Attach links
//! (AttachLinksDialog), Reuse evidence (TransactionEvidenceReuseDialog.tsx), the revert
//! confirmation, and the split payout card (TransactionSplitPayoutCard.tsx).

use rune::{Color, El, Tx, col, el, row, text};
use rune_kit::{TextArea, TextInput};
use serde_json::Value;

use super::actions::{attachment_id, parse_links, rail_label};
use super::draft::{self, opt_s};
use super::widgets::{select_menu, select_trigger, small_button, toggle_select};
use super::{Open, Select};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use crate::ui::popup::TriggerExt;

/// `explorerForTransaction` (lib/explorer.ts) with the explorer settings the web derives from the
/// backend list: a configured explorer for the row's layer, else the public mainnet fallback when
/// the setup and the user allow it (`graph::sources::explorer_for_record`).
pub fn explorer_target(app: &App, r: &Value) -> Option<(String, String)> {
    super::graph::sources::explorer_for_record(app, r)
        .ok()
        .map(|e| (e.label, e.url))
}

thread_local! {
    /// The window height the dialogs anchor against (`--kb-dialog-top: max(1rem, 8vh)`).
    static WINDOW_H: std::cell::Cell<f32> = const { std::cell::Cell::new(900.0) };
    /// The window width the dialogs centre in.
    static WINDOW_W: std::cell::Cell<f32> = const { std::cell::Cell::new(1440.0) };
}

/// `DialogContent` (ui/dialog.tsx): a faint `kb-glass-backdrop` (dark `black/22%`, light
/// `foreground/6%`), then the `kb-glass-dialog` surface (`background` at 80 % with a 12 px blur,
/// a 10 % hairline, a deep soft shadow), `max-w-[calc(100%-2rem)] sm:max-w-lg`, `rounded-[1rem]`,
/// `p-(--kb-card-padding)`, `gap-4`, centred horizontally at `top: max(1rem, 8vh)`, with the
/// `absolute top-3 right-3 size-7` close button.
fn dialog(tk: &Tk, layer: &mut Vec<El>, key: &str, width: f32, content: El, close: fn(&mut App)) {
    dialog_capped(tk, layer, key, width, tk.r(32.0), content, close);
}

/// `dialog` with its own `max-w` (`sm:max-w-lg` is 32 rem; `max-w-3xl!` 48 rem).
#[allow(clippy::too_many_arguments)]
fn dialog_capped(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    width: f32,
    cap: f32,
    content: El,
    close: fn(&mut App),
) {
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    layer.push(
        el().key(format!("{key}-scrim"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(wash)
            .on_press(close)
            .role("none"),
    );
    let top = crate::shell::dialog_top(tk, WINDOW_H.with(|h| h.get()));
    let (border, shadows) = if tk.dark {
        (
            Color::WHITE.alpha(0.08),
            vec![
                rune::Shadow {
                    y: 1.0,
                    color: Color::WHITE.alpha(0.04),
                    inset: true,
                    ..Default::default()
                },
                rune::Shadow {
                    y: 24.0,
                    blur: 72.0,
                    spread: -20.0,
                    color: Color::BLACK.alpha(0.9),
                    ..Default::default()
                },
            ],
        )
    } else {
        (
            tk.fg.alpha(0.1),
            vec![rune::Shadow {
                y: 24.0,
                blur: 64.0,
                spread: -24.0,
                color: Color::BLACK.alpha(0.65),
                ..Default::default()
            }],
        )
    };
    let close_btn = super::widgets::icon_button(
        tk,
        format!("{key}-close"),
        super::widgets::Variant::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None)
    .on_press(close);
    let w = width.min(cap).min(WINDOW_W.with(|w| w.get()) - tk.r(2.0));
    let mut surface = col()
        .key(key.to_owned())
        .role("dialog")
        .w(w)
        .padding(tk.r(1.25))
        .gap(tk.r(1.0))
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(6.0)
        .transition(crate::shell::dialog_enter())
        .border(1.0, border)
        .modal(true, close)
        .child(content)
        .child(close_btn);
    for sh in shadows {
        surface = surface.box_shadow(sh);
    }
    layer.push(crate::shell::centered(
        surface,
        WINDOW_W.with(|w| w.get()),
        w,
        top,
    ));
}

/// `DialogTitle`: `text-lg leading-none font-semibold`.
fn title(tk: &Tk, s: String) -> El {
    text(s).font(tk.tlg(600.0).lh(tk.r(1.125))).color(tk.fg)
}

/// `DialogDescription`: `text-sm text-muted-foreground`.
fn desc(tk: &Tk, s: String) -> El {
    text(s).font(tk.tsm(400.0)).color(tk.muted_fg).wrap()
}

pub fn mount(app: &App, tk: &Tk, o: &Open, layer: &mut Vec<El>) {
    let Some(r) = &o.record else { return };
    if app.shell.window_h > 0.0 {
        WINDOW_H.with(|h| h.set(app.shell.window_h));
    }
    if app.shell.window_w > 0.0 {
        WINDOW_W.with(|w| w.set(app.shell.window_w));
    }
    if o.explorer_open {
        explorer(app, tk, o, r, layer);
    }
    if o.links_open {
        links(app, tk, o, layer);
    }
    if let Some((reference, load)) = &o.explain {
        explanation(app, tk, reference, load, layer);
    }
    if o.reuse.is_some() && !o.busy.reuse {
        reuse(app, tk, o, layer);
    }
    if let Some((_, field, label)) = &o.revert_confirm {
        revert(tk, field.as_deref(), label, layer);
    }
}

fn explorer(app: &App, tk: &Tk, o: &Open, r: &Value, layer: &mut Vec<El>) {
    let target = explorer_target(app, r);
    let label = target
        .as_ref()
        .map(|t| t.0.clone())
        .unwrap_or_else(|| t("transactions:explorerDialog.fallbackExplorer"));
    let url = target.as_ref().map(|t| t.1.clone()).unwrap_or_default();
    let txn = draft::txid(r);
    let open_url = url.clone();
    let mut c = col()
        .gap(tk.r(0.75))
        .child(
            row()
                .size(tk.r(2.5), tk.r(2.5))
                .center()
                .radius(tk.r(1.25))
                .fill(tk.warn.bg)
                .child(ic(&I::SHIELD_ALERT, tk.r(1.25), tk.warn.text)),
        )
        .child(title(tk, t("transactions:explorerDialog.title")))
        .child(desc(
            tk,
            tr("transactions:explorerDialog.body", &[("explorer", &label)]),
        ))
        .child(
            col()
                .w_full()
                .padding(tk.r(0.5))
                .gap(tk.r(0.25))
                .radius(tk.r(0.375))
                .fill(tk.muted)
                .child(
                    text(txn)
                        .font(tk.txs(400.0).mono())
                        .color(tk.fg)
                        .wrap_anywhere(),
                )
                .children((!url.is_empty()).then(|| {
                    text(url)
                        .font(tk.txs(400.0).mono())
                        .color(tk.muted_fg)
                        .wrap_anywhere()
                })),
        );
    if let Some(e) = &o.explorer_error {
        c = c.child(
            text(e.clone())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert"),
        );
    }
    c = c.child(
        row()
            .w_full()
            .justify_end()
            .gap(tk.r(0.5))
            .child(
                crate::ui::outline_button(
                    tk,
                    "txd-explorer-cancel",
                    None,
                    &t("common:actions.cancel"),
                )
                .on_press(close_explorer),
            )
            .child(
                crate::ui::primary_button(
                    tk,
                    "txd-explorer-open",
                    Some(&I::EXTERNAL_LINK),
                    &t("transactions:explorerDialog.open"),
                )
                .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                    match tx.open_url(&open_url) {
                        Ok(()) => close_explorer(a),
                        Err(_) => {
                            if let Some(o) = a.detail.open.as_mut() {
                                o.explorer_error = Some(t("transactions:explorerDialog.openError"));
                            }
                        }
                    }
                }),
            ),
    );
    dialog(
        tk,
        layer,
        "txd-explorer-dialog",
        34.0 * tk.rem,
        c.label(t("transactions:explorerDialog.title")),
        close_explorer,
    );
}

fn close_explorer(a: &mut App) {
    if let Some(o) = a.detail.open.as_mut() {
        o.explorer_open = false;
    }
}

fn links(app: &App, tk: &Tk, o: &Open, layer: &mut Vec<El>) {
    use super::widgets::{Size, Variant, button, field, field_label, form_kit};
    let mut k = form_kit(tk);
    // `font-mono text-xs` (TextArea draws in the kit's `ui` type).
    k.ty.ui = tk.txs(400.0).mono();
    let (urls, errors) = parse_links(&app.detail.links.text);
    // `Textarea min-h-32 font-mono text-xs`.
    let area = field(
        tk,
        TextArea::new(&app.detail.links)
            .label(t("transactions:attachments.linksDialog.urls"))
            .placeholder(t("transactions:attachments.linksDialog.placeholder"))
            .rows(6)
            .width(None)
            .on_edit(|a: &mut App| &mut a.detail.links)
            .build(&k)
            .key("txd-links-input")
            .min_h(tk.r(8.0)),
    );
    // The status line (`flex justify-between gap-2 text-xs`): counts, then the invalid lines in
    // amber (`ml-1`), the error at the end.
    let mut status = row().gap(tk.r(0.25)).role("status").child(
        text(if urls.is_empty() {
            t("transactions:attachments.linksDialog.noUrls")
        } else {
            tc(
                "transactions:attachments.linksDialog.urlsReady",
                urls.len(),
                &[],
            )
        })
        .font(tk.txs(400.0))
        .color(tk.muted_fg),
    );
    if !errors.is_empty() {
        let amber = if tk.dark {
            rune::color::hex("#fbbf24").unwrap()
        } else {
            rune::color::hex("#d97706").unwrap()
        };
        status = status.child(
            text(tc(
                "transactions:attachments.linksDialog.invalidLines",
                errors.len(),
                &[],
            ))
            .font(tk.txs(400.0))
            .color(amber),
        );
    }
    let mut status_row = row()
        .w_full()
        .gap(tk.r(0.5))
        .justify_between()
        .center_y()
        .child(status);
    if let Some(e) = &o.links_error {
        status_row = status_row.child(
            text(e.clone())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap()
                .shrink()
                .min_w(0.0),
        );
    }
    let busy = o.busy.links > 0;
    let submit_label = if busy {
        t("transactions:attachments.linksDialog.attaching")
    } else if urls.len() > 1 {
        tr(
            "transactions:attachments.linksDialog.attachMany",
            &[("count", &urls.len().to_string())],
        )
    } else {
        t("transactions:attachments.linksDialog.attachOne")
    };
    let ok = !busy && !urls.is_empty() && errors.is_empty();
    let mut submit = button(
        tk,
        "txd-links-submit",
        Variant::Default,
        Size::Default,
        None,
        &submit_label,
        ok,
    );
    if ok {
        submit = submit.on_press(super::actions::attach_links);
    }
    // `DialogHeader gap-2`, the field `grid gap-2`, `DialogFooter sm:justify-end gap-2`.
    let c = col()
        .w_full()
        .gap(tk.r(1.0))
        .label(t("transactions:attachments.linksDialog.title"))
        .child(
            col()
                .gap(tk.r(0.5))
                .child(title(tk, t("transactions:attachments.linksDialog.title")))
                .child(desc(
                    tk,
                    t("transactions:attachments.linksDialog.description"),
                )),
        )
        .child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .child(field_label(
                    tk,
                    &t("transactions:attachments.linksDialog.urls"),
                    None,
                    false,
                ))
                .child(area)
                .child(status_row),
        )
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(tk.r(0.5))
                .child(
                    button(
                        tk,
                        "txd-links-cancel",
                        Variant::Outline,
                        Size::Default,
                        None,
                        &t("common:actions.cancel"),
                        !busy,
                    )
                    .on_press(close_links),
                )
                .child(submit),
        );
    dialog(tk, layer, "txd-links-dialog", 32.0 * tk.rem, c, close_links);
}

fn close_links(a: &mut App) {
    if let Some(o) = a.detail.open.as_mut() {
        o.links_open = false;
    }
}

/// Every loaded row except the current one: `(id, "txnId · date · wallet")`.
pub fn reuse_candidates(app: &App) -> Vec<(String, String)> {
    let current = app
        .detail
        .open
        .as_ref()
        .and_then(|o| o.id().map(str::to_owned))
        .unwrap_or_default();
    let Some(sn) = app.snapshot() else {
        return vec![];
    };
    let mut seen = std::collections::HashSet::new();
    sn.txs
        .iter()
        .chain(sn.activity_txs.iter().rev())
        .filter(|t| t.id != current && seen.insert(t.id.clone()))
        .take(200)
        .map(|t| {
            // `transactionOptionLabel`: the full `txnId`, date and wallet.
            let txn = if t.external_id.is_empty() {
                t.id.clone()
            } else {
                t.external_id.clone()
            };
            let date = crate::format::tx_date(&t.date, t.conf > 0.0);
            (t.id.clone(), format!("{txn} · {date} · {}", t.account))
        })
        .collect()
}

fn reuse(app: &App, tk: &Tk, o: &Open, layer: &mut Vec<El>) {
    let hide = app.prefs.hide_sensitive();
    let Some(re) = &o.reuse else { return };
    let cands = reuse_candidates(app);
    let current = re.source.clone().unwrap_or_default();
    let current_label = cands
        .iter()
        .find(|c| c.0 == current)
        .map(|c| c.1.clone())
        .unwrap_or_else(|| t("transactions:evidenceReuse.chooseTransaction"));
    // The open list mounts above the dialog (after it), or its modal would cover it.
    let mut popups = Vec::new();
    if o.select == Some(Select::ReuseSource) {
        select_menu(
            tk,
            &mut popups,
            "txd-reuse-source",
            cands.clone(),
            &current,
            super::actions::pick_reuse_source,
        );
    }
    let mut list = col().w_full().gap(tk.r(0.25));
    match &re.list {
        None => {
            list = list.child(
                row()
                    .w_full()
                    .px(tk.r(0.5))
                    .py(tk.r(1.5))
                    .justify_center()
                    .child(
                        text(t("transactions:evidenceReuse.loading"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                    ),
            )
        }
        // `px-2 py-6 text-center text-sm text-muted-foreground`.
        Some(l) if l.is_empty() => {
            list = list.child(
                row()
                    .w_full()
                    .px(tk.r(0.5))
                    .py(tk.r(1.5))
                    .justify_center()
                    .child(
                        text(t("transactions:evidenceReuse.noEvidence"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                    ),
            )
        }
        Some(l) => {
            for a in l {
                let id = attachment_id(a);
                let checked = re.selected.contains(&id);
                let is_url = super::actions::is_url(a);
                let tid = id.clone();
                list = list.child(
                    row()
                        .key(format!("txd-reuse-item-{id}"))
                        .w_full()
                        .padding(tk.r(0.5))
                        .gap(tk.r(0.5))
                        .center_y()
                        .radius(tk.r(0.375))
                        .border(1.0, if checked { tk.primary } else { tk.border })
                        .role("checkbox")
                        .checked(checked)
                        .label(rail_label(a))
                        .focusable(true)
                        .on_press(move |a: &mut App| {
                            if let Some(r) = a.detail.open.as_mut().and_then(|o| o.reuse.as_mut()) {
                                if let Some(i) = r.selected.iter().position(|x| *x == tid) {
                                    r.selected.remove(i);
                                } else {
                                    r.selected.push(tid.clone());
                                }
                            }
                        })
                        .child(
                            row()
                                .size(14.0, 14.0)
                                .center()
                                .radius(3.0)
                                .border(1.0, tk.input)
                                .fill(if checked { tk.primary } else { tk.background })
                                .child(if checked {
                                    ic(&I::CHECK, 10.0, tk.primary_fg)
                                } else {
                                    el()
                                }),
                        )
                        .child(ic(
                            if is_url { &I::LINK_2 } else { &I::FILE_TEXT },
                            tk.r(0.875),
                            tk.muted_fg,
                        ))
                        .child(
                            super::widgets::sens(tk, rail_label(a), tk.txs(500.0), tk.fg, hide)
                                .grow(1.0)
                                .truncate(),
                        ),
                );
            }
        }
    }
    let ok = !re.selected.is_empty() && !o.busy.reuse;
    let mut copy = super::widgets::button(
        tk,
        "txd-reuse-copy",
        super::widgets::Variant::Default,
        super::widgets::Size::Default,
        Some(&I::CHECK),
        &t("transactions:evidenceReuse.copyEvidence"),
        ok,
    );
    if ok {
        copy = copy.on_press(super::actions::copy_evidence);
    }
    // Header (`gap-2`), then `space-y-4`: the source picker (`space-y-1.5`, a `text-2xs
    // font-semibold uppercase tracking-wide` caption) and the evidence box (`rounded-md border`;
    // a `border-b px-3 py-2` header, `max-h-72 p-2` list), then the footer.
    let c = col()
        .w_full()
        .gap(tk.r(1.0))
        .label(t("transactions:evidenceReuse.title"))
        .child(
            col()
                .gap(tk.r(0.5))
                .child(title(tk, t("transactions:evidenceReuse.title")))
                .child(desc(tk, t("transactions:evidenceReuse.description"))),
        )
        .child(
            col()
                .w_full()
                .gap(tk.r(1.0))
                .child(
                    col()
                        .w_full()
                        .gap(tk.r(0.375))
                        .child(
                            text(t("transactions:evidenceReuse.sourceTransaction").to_uppercase())
                                .font(tk.t2xs(600.0).tracking(0.25))
                                .color(tk.muted_fg),
                        )
                        .child(
                            select_trigger(
                                tk,
                                "txd-reuse-source",
                                &t("transactions:evidenceReuse.sourceTransaction"),
                                &current_label,
                                true,
                            )
                            .w_full()
                            .select_trigger(o.select == Some(Select::ReuseSource), |a: &mut App| {
                                toggle_select(a, Select::ReuseSource)
                            })
                            .blur(if hide { 3.5 } else { 0.0 }),
                        ),
                )
                .child(
                    col()
                        .w_full()
                        .radius(tk.r(0.375))
                        .border(1.0, tk.border)
                        .child(
                            row()
                                .w_full()
                                .px(tk.r(0.75))
                                .py(tk.r(0.5))
                                .gap(tk.r(0.5))
                                .center_y()
                                .justify_between()
                                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                                .child(
                                    row()
                                        .gap(tk.r(0.5))
                                        .center_y()
                                        .child(ic(&I::REPEAT_2, tk.r(1.0), tk.muted_fg))
                                        .child(
                                            text(t("transactions:evidenceReuse.evidenceToCopy"))
                                                .font(tk.tsm(500.0))
                                                .color(tk.fg),
                                        ),
                                )
                                .child(super::widgets::badge(
                                    tk,
                                    &tr(
                                        "transactions:evidenceReuse.selectedCount",
                                        &[("count", &re.selected.len().to_string())],
                                    ),
                                    tk.fg,
                                    Color::TRANSPARENT,
                                    tk.border,
                                )),
                        )
                        .child(
                            col()
                                .w_full()
                                .max_h(tk.r(18.0))
                                .padding(tk.r(0.5))
                                .child(list)
                                .scroll(rune::ScrollSpec {
                                    stick_bottom: false,
                                }),
                        ),
                ),
        )
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(tk.r(0.5))
                .child(
                    super::widgets::button(
                        tk,
                        "txd-reuse-cancel",
                        super::widgets::Variant::Outline,
                        super::widgets::Size::Default,
                        None,
                        &t("common:actions.cancel"),
                        true,
                    )
                    .on_press(close_reuse),
                )
                .child(copy),
        );
    dialog(tk, layer, "txd-reuse-dialog", 34.0 * tk.rem, c, close_reuse);
    layer.extend(popups);
}

fn close_reuse(a: &mut App) {
    if let Some(o) = a.detail.open.as_mut() {
        o.reuse = None;
        o.select = None;
    }
}

fn revert(tk: &Tk, field: Option<&str>, label: &str, layer: &mut Vec<El>) {
    let _ = field;
    let c = col()
        .gap(tk.r(0.875))
        .label(t("transactions:history.revertEditAria"))
        .child(title(tk, t("transactions:history.revertEditAria")))
        .child(desc(
            tk,
            tr(
                "transactions:history.confirmRevertField",
                &[("label", label)],
            ),
        ))
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(tk.r(0.5))
                .child(
                    crate::ui::outline_button(
                        tk,
                        "txd-revert-cancel",
                        None,
                        &t("common:actions.cancel"),
                    )
                    .on_press(close_revert),
                )
                .child(
                    crate::ui::primary_button(
                        tk,
                        "txd-revert-confirm",
                        Some(&I::ROTATE_CCW),
                        &t("common:actions.confirm"),
                    )
                    .on_press(|a: &mut App| {
                        let Some((ev, fld, lbl)) = a
                            .detail
                            .open
                            .as_ref()
                            .and_then(|o| o.revert_confirm.clone())
                        else {
                            return;
                        };
                        super::actions::revert(a, &ev, fld.as_deref(), &lbl);
                    }),
                ),
        );
    dialog(
        tk,
        layer,
        "txd-revert-dialog",
        28.0 * tk.rem,
        c,
        close_revert,
    );
}

fn close_revert(a: &mut App) {
    if let Some(o) = a.detail.open.as_mut() {
        o.revert_confirm = None;
    }
}

// ---- Split payout card (§5.5; its copy lives under `transactions:splitPayout`) -----------------

pub fn split_card(app: &App, tk: &Tk, o: &Open, r: &Value, layer: &mut Vec<El>) -> Option<El> {
    // `isSplitTransferQuarantine`: the Review page's reason when it hosts the sheet.
    let reason = super::header::quarantine_reason(app, o, r)?;
    if !reason.to_lowercase().contains("transfer_fee_implausible") {
        return None;
    }
    let k = tk.kit();
    let asset = opt_s(r, "asset").unwrap_or_else(|| "BTC".into());
    let outbound = draft::amount_btc(r);
    let (out_err, pay_err) = super::actions::split_errors(app, outbound);
    let input = |edit: &rune::TextEdit, key: &'static str, label: String, ph: String| -> El {
        let i = TextInput::new(edit)
            .label(label.clone())
            .placeholder(ph)
            .height(tk.r(2.0))
            .text_style(tk.tsm(400.0));
        let i = match key {
            "txd-split-out" => i.on_edit(|a: &mut App| &mut a.detail.split_out),
            "txd-split-payout" => i.on_edit(|a: &mut App| &mut a.detail.split_payout),
            "txd-split-counterparty" => i.on_edit(|a: &mut App| &mut a.detail.split_counterparty),
            _ => i.on_edit(|a: &mut App| &mut a.detail.split_notes),
        };
        col()
            .gap(tk.r(0.25))
            .grow(1.0)
            .min_w(0.0)
            .child(text(label).font(tk.txs(500.0)).color(tk.fg))
            .child(i.build(&k).key(key))
    };
    let mut out_field = input(
        &app.detail.split_out,
        "txd-split-out",
        tr("transactions:splitPayout.outLabel", &[("asset", &asset)]),
        format!("{outbound}"),
    );
    out_field = out_field.child(
        text(tr(
            "transactions:splitPayout.outboundTotal",
            &[("amount", &outbound.to_string()), ("asset", &asset)],
        ))
        .font(tk.t2xs(400.0))
        .color(tk.muted_fg),
    );
    if let Some(e) = &out_err {
        out_field = out_field.child(text(e.clone()).font(tk.t2xs(400.0)).color(tk.destructive));
    }
    let mut pay_field = input(
        &app.detail.split_payout,
        "txd-split-payout",
        tr(
            "transactions:splitPayout.payoutLabel",
            &[("asset", &o.split_asset)],
        ),
        "0.00000000".into(),
    );
    if let Some(e) = &pay_err {
        pay_field = pay_field.child(text(e.clone()).font(tk.t2xs(400.0)).color(tk.destructive));
    }
    if o.select == Some(Select::SplitAsset) {
        select_menu(
            tk,
            layer,
            "txd-split-asset",
            ["BTC", "LBTC", "LNBTC"]
                .iter()
                .map(|x| (x.to_string(), x.to_string()))
                .collect(),
            &o.split_asset,
            |a, v| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.split_asset = v.to_owned();
                }
            },
        );
    }
    let policy_label = |p: &str| {
        t(if p == "taxable" {
            "transactions:splitPayout.policyTaxable"
        } else {
            "transactions:splitPayout.policyCarrying"
        })
    };
    if o.select == Some(Select::SplitPolicy) {
        select_menu(
            tk,
            layer,
            "txd-split-policy",
            vec![
                ("carrying-value".into(), policy_label("carrying-value")),
                ("taxable".into(), policy_label("taxable")),
            ],
            &o.split_policy,
            |a, v| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.split_policy = v.to_owned();
                }
            },
        );
    }
    let valid = out_err.is_none()
        && pay_err.is_none()
        && !app.detail.split_out.text.trim().is_empty()
        && !app.detail.split_payout.text.trim().is_empty();
    let ok = valid && !o.busy.split;
    let mut submit = small_button(
        tk,
        "txd-split-submit",
        None,
        &t(if o.busy.split {
            "common:state.saving"
        } else {
            "transactions:splitPayout.submit"
        }),
        true,
        ok,
    );
    if ok {
        submit = submit.on_press(super::actions::split_submit);
    }
    let c = col()
        .key("txd-split")
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.625))
        .radius(tk.r(0.375))
        .border(1.0, tk.warn.border)
        .fill(tk.warn.bg);
    let payout_asset = t("transactions:splitPayout.payoutAsset");
    let tax_policy = t("transactions:splitPayout.taxPolicy");
    let mut c = c
        .child(
            text(t("transactions:splitPayout.title"))
                .font(tk.tsm(600.0))
                .color(tk.fg),
        )
        .child(
            text(tr("transactions:splitPayout.body", &[("asset", &asset)]))
                .font(tk.txs(400.0))
                .color(tk.fg)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(out_field)
                .child(
                    col()
                        .gap(tk.r(0.25))
                        .grow(1.0)
                        .child(text(payout_asset.clone()).font(tk.txs(500.0)).color(tk.fg))
                        .child(
                            select_trigger(
                                tk,
                                "txd-split-asset",
                                &payout_asset,
                                &o.split_asset,
                                true,
                            )
                            .select_trigger(o.select == Some(Select::SplitAsset), |a: &mut App| {
                                toggle_select(a, Select::SplitAsset)
                            }),
                        ),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(pay_field)
                .child(
                    col()
                        .gap(tk.r(0.25))
                        .grow(1.0)
                        .child(text(tax_policy.clone()).font(tk.txs(500.0)).color(tk.fg))
                        .child(
                            select_trigger(
                                tk,
                                "txd-split-policy",
                                &tax_policy,
                                &policy_label(&o.split_policy),
                                true,
                            )
                            .select_trigger(
                                o.select == Some(Select::SplitPolicy),
                                |a: &mut App| toggle_select(a, Select::SplitPolicy),
                            ),
                        ),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .child(input(
                    &app.detail.split_counterparty,
                    "txd-split-counterparty",
                    t("transactions:splitPayout.counterparty"),
                    t("transactions:splitPayout.counterpartyPlaceholder"),
                ))
                .child(input(
                    &app.detail.split_notes,
                    "txd-split-notes",
                    t("transactions:splitPayout.notes"),
                    String::new(),
                )),
        )
        .child(submit)
        .child(
            text(t("transactions:splitPayout.carryingHint"))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg),
        );
    if let Some(e) = &o.split_error {
        c = c.child(
            text(e.clone())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    if let Some(m) = &o.split_msg {
        c = c.child(text(m.clone()).font(tk.txs(400.0)).color(tk.good.text));
    }
    Some(c)
}

/// `CapitalGainExplanation`'s dialog: `max-w-3xl! overflow-y-auto`, the title, the description and
/// the explanation body (the Reports page's renderer, `reports::report::extra::explanation_body`).
fn explanation(
    app: &App,
    tk: &Tk,
    reference: &Value,
    load: &crate::app::Load<Value>,
    layer: &mut Vec<El>,
) {
    use crate::app::Load;
    let body = match load {
        Load::Idle | Load::Loading => desc(tk, t("journals:explanation.loading")).color(tk.fg),
        Load::Failed(e) => desc(
            tk,
            if e.is_empty() {
                t("journals:explanation.unavailable")
            } else {
                e.clone()
            },
        )
        .color(tk.fg)
        .role("alert"),
        Load::Ready(d) => crate::reports::report::extra::explanation_body(app, tk, d, reference),
    };
    let max_h = (WINDOW_H.with(|h| h.get())
        - 2.0 * crate::shell::dialog_top(tk, WINDOW_H.with(|h| h.get()))
        - tk.r(8.0))
    .max(tk.r(12.0));
    let c = col()
        .w_full()
        .gap(tk.r(0.75))
        .label(t("journals:explanation.title"))
        .child(title(tk, t("journals:explanation.title")))
        .child(desc(tk, t("journals:explanation.description")))
        .child(
            col()
                .key("txd-explain-body")
                .w_full()
                .max_h(max_h)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        );
    dialog_capped(
        tk,
        layer,
        "txd-explain-dialog",
        tk.r(48.0),
        tk.r(48.0),
        c,
        |a: &mut App| {
            if let Some(o) = a.detail.open.as_mut() {
                o.explain = None;
            }
        },
    );
}
