//! The Details, Classify and Pricing tabs (TransactionDetailsTab.tsx, TransactionClassifyTab.tsx,
//! TransactionPricingTab.tsx). All edits change the draft only; Save writes them.

use rune::{Color, El, Tx, col, el, row, text};
use rune_kit::{TextArea, TextInput};
use serde_json::Value;

use super::draft::{self, f, opt_s, s};
use super::widgets::{
    dirty_dot, fact, field_label, hint, select_menu, select_trigger, small_button, toggle_select,
    value,
};
use super::{Open, Select, Tab};
use crate::app::App;
use crate::format;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use crate::ui::popup::TriggerExt;

/// `grid grid-cols-2 gap-x-4 gap-y-3`.
fn grid2(tk: &Tk, cells: Vec<El>) -> El {
    let mut g = col().w_full().gap(tk.r(0.75));
    let mut it = cells.into_iter();
    while let Some(a) = it.next() {
        let b = it.next();
        let mut r = row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(a.grow(1.0));
        r = match b {
            Some(b) => r.child(b.grow(1.0)),
            None => r.child(el().grow(1.0)),
        };
        g = g.child(r);
    }
    g
}

// ---- Details (§4.1) ------------------------------------------------------------------------------

pub fn details(app: &App, tk: &Tk, o: &Open, r: &Value, layer: &mut Vec<El>) -> El {
    let hide = app.prefs.hide_sensitive();
    let dirt = o.dirty();
    let id_full = opt_s(r, "explorerId")
        .or_else(|| opt_s(r, "externalId"))
        .unwrap_or_else(|| s(r, "id").to_owned());
    let copy_full = id_full.clone();
    // Mono values are `font-mono text-xs`; the copy button sits in the label row (`Fact`).
    let txid = super::widgets::sens(
        tk,
        draft::short_txid(&id_full),
        tk.txs(500.0).mono(),
        tk.fg,
        hide,
    )
    .truncate();
    let copy_txid = copy_button(
        tk,
        "txd-copy-txid",
        &t("transactions:details.transactionId"),
        copy_full,
        true,
    );
    let price = if o.draft.pricing_source_kind.as_deref() == Some("manual_override")
        && !o.draft.manual_price.is_empty()
    {
        tr(
            "transactions:details.manualPerBtc",
            &[
                ("price", &o.draft.manual_price),
                ("currency", &o.draft.manual_currency),
            ],
        )
    } else if let Some(rt) = f(r, "rate").filter(|x| *x != 0.0) {
        tr(
            "transactions:details.perBtc",
            &[(
                "value",
                &format::fiat(rt, &super::tax::row_currency(Some(app), r)),
            )],
        )
    } else {
        t("transactions:details.priceMissing")
    };
    let source_type = opt_s(r, "type")
        .or_else(|| opt_s(r, "paymentMethod"))
        .unwrap_or_default();
    let source = format!(
        "{} · {}",
        opt_s(r, "account")
            .or_else(|| opt_s(r, "paymentMethod"))
            .unwrap_or_default(),
        type_label(&source_type)
    );
    let explorer: El = match super::dialogs::explorer_target(app, r) {
        Some((label, _)) => row()
            .key("txd-details-explorer")
            .gap(tk.r(0.25))
            .center_y()
            .role("link")
            .label(tr(
                "transactions:sourceRecord.openOn",
                &[("txid", &draft::short_txid(&id_full)), ("explorer", &label)],
            ))
            .focusable(true)
            .on_press(|a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.explorer_open = true;
                }
            })
            .child(text(label).font(tk.tsm(500.0)).color(tk.fg))
            .child(ic(&I::EXTERNAL_LINK, tk.r(0.75), tk.fg)),
        None => value(tk, t("transactions:sourceRecord.noExplorer")).color(tk.muted_fg),
    };
    let tags: El = if o.draft.tags.is_empty() {
        value(tk, t("transactions:details.tagsNone")).color(tk.muted_fg)
    } else {
        // `Badge variant="secondary" rounded-md`: px-2 py-0.5 text-xs font-medium.
        o.draft
            .tags
            .iter()
            .fold(row().gap(tk.r(0.25)).flex_wrap(), |acc, tag| {
                acc.child(
                    row()
                        .px(tk.r(0.5))
                        .py(tk.r(0.125))
                        .radius(tk.r(0.375))
                        .fill(tk.muted)
                        .child(super::widgets::sens(
                            tk,
                            tag.clone(),
                            tk.txs(500.0),
                            tk.fg,
                            hide,
                        )),
                )
            })
    };
    let txn_id = opt_s(r, "externalId").unwrap_or_default();
    let ext_differs =
        !txn_id.is_empty() && txn_id != opt_s(r, "explorerId").unwrap_or_else(|| txn_id.clone());
    let (ext_label, ext_hint, ext_value) = if ext_differs {
        (
            t("transactions:details.externalId"),
            Some(t("transactions:details.externalIdHint")),
            txn_id.clone(),
        )
    } else {
        (
            t("transactions:sourceRecord.kassiberRow"),
            None,
            s(r, "id").to_owned(),
        )
    };
    // The external id is shortened; the Kassiber row shows in full (it truncates to the column).
    let ext_text = if ext_differs {
        draft::short_txid(&ext_value)
    } else {
        ext_value.clone()
    };
    let ext = super::widgets::sens(
        tk,
        ext_text,
        tk.txs(500.0).mono(),
        tk.fg,
        hide && !ext_differs,
    )
    .truncate();
    let copy_ext = (!ext_differs)
        .then(|| copy_button(tk, "txd-copy-ext", &ext_label, ext_value.clone(), true));
    let note: El = if o.draft.note.trim().is_empty() {
        value(tk, t("transactions:details.noteNone")).color(tk.muted_fg)
    } else {
        super::widgets::sens(tk, o.draft.note.clone(), tk.tsm(500.0), tk.fg, hide)
            .max_lines(2)
            .wrap()
    };
    let cells = vec![
        super::widgets::fact_with(
            tk,
            &t("transactions:details.transactionId"),
            Some(t("transactions:details.transactionIdHint")),
            false,
            Some(copy_txid),
            txid,
        ),
        fact(
            tk,
            &t("transactions:details.priceAtTime"),
            Some(t("transactions:details.priceAtTimeHint")),
            false,
            super::widgets::sens(tk, price, tk.tsm(500.0), tk.fg, hide),
        ),
        fact(tk, &t("transactions:sourceRecord.source"), None, false, {
            let e = super::widgets::sens(tk, source.clone(), tk.tsm(500.0), tk.fg, hide).truncate();
            if hide {
                e
            } else {
                e.help_when_truncated(source)
            }
        }),
        fact(
            tk,
            &t("transactions:details.explorer"),
            None,
            false,
            explorer,
        ),
        fact(
            tk,
            &t("transactions:details.label"),
            None,
            dirt.label,
            value(tk, o.draft.label.clone()),
        ),
        fact(tk, &t("transactions:details.tags"), None, dirt.tags, tags),
        fact(
            tk,
            &t("transactions:details.included"),
            None,
            dirt.excluded,
            value(
                tk,
                if o.draft.excluded {
                    t("transactions:details.includedNo")
                } else {
                    t("transactions:details.includedYes")
                },
            ),
        ),
        super::widgets::fact_with(tk, &ext_label, ext_hint, false, copy_ext, ext),
    ];
    // `<dl kb-surface-inset grid grid-cols-2 gap-x-4 gap-y-3 p-3>`; Note spans both columns.
    let mut body = super::widgets::surface_inset(tk)
        .padding(tk.r(0.75))
        .gap(tk.r(0.75))
        .child(grid2(tk, cells))
        .child(fact(
            tk,
            &t("transactions:details.note"),
            Some(t("transactions:details.noteHint")),
            dirt.note,
            note,
        ));
    body = body.key("txd-details-grid");
    let _ = layer;
    col()
        .w_full()
        .gap(tk.r(0.75))
        .child(body)
        .children(super::rail::commercial(app, tk, o))
}

/// The label-row copy button (`rounded-sm text-muted-foreground`, Copy 0.75 rem, no padding).
fn copy_button(tk: &Tk, key: &str, label: &str, payload: String, enabled: bool) -> El {
    let name = tr("transactions:infoHint.copy", &[("label", label)]);
    let mut b = row()
        .key(key.to_owned())
        .center()
        .radius(tk.r(0.25))
        .role("button")
        .label(name.clone())
        .help(name)
        .focusable(true)
        .child(ic(&I::COPY, tk.r(0.75), tk.muted_fg));
    if enabled {
        b = b.on_press_tx(move |a: &mut App, tx: &mut Tx| {
            crate::settings::effects::copy(a, tx, payload.clone())
        });
    } else {
        b = b.disabled(true).opacity(0.4);
    }
    b
}

/// `typeLabel(sourceType)`.
pub fn type_label(ty: &str) -> String {
    let key = match ty {
        "Hard fork" | "Hardfork" | "hardfork" => "hardFork".to_owned(),
        "LN invoice" => "lnInvoice".into(),
        "LN payment" => "lnPayment".into(),
        "Channel open" => "channelOpen".into(),
        "Channel close" => "channelClose".into(),
        other => {
            let mut c = other.chars();
            c.next()
                .map(|f| f.to_lowercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
                .replace(' ', "")
        }
    };
    let full = format!("transactions:type.{key}");
    if crate::i18n::has(&full) {
        t(&full)
    } else {
        ty.to_owned()
    }
}

// ---- Classify (§4.2) ------------------------------------------------------------------------------

fn add_tag(a: &mut App, tag: &str) {
    let tag = tag.trim().trim_end_matches(',').trim().to_owned();
    let Some(o) = a.detail.open.as_mut() else {
        return;
    };
    if tag.is_empty() || o.draft.tags.iter().any(|x| x.eq_ignore_ascii_case(&tag)) {
        return;
    }
    o.draft.tags.push(tag);
}

fn commit_tag_input(a: &mut App) {
    let v = a.detail.tag.text.clone();
    add_tag(a, &v);
    a.detail.tag = rune::TextEdit::new("");
    a.detail.tag.focused = true;
}

pub fn classify(app: &App, tk: &Tk, o: &Open, r: &Value, layer: &mut Vec<El>) -> El {
    let hide = app.prefs.hide_sensitive();
    let k = super::widgets::form_kit(tk);
    let dirt = o.dirty();
    // `grid gap-4 @md:grid-cols-2`; each field `grid gap-2` (label over a w-fit select).
    let label_sel = col()
        .items_start()
        .gap(tk.r(0.5))
        .child(field_label(
            tk,
            &t("transactions:classify.label"),
            None,
            dirt.label,
        ))
        .child(
            select_trigger(
                tk,
                "txd-label",
                &t("transactions:classify.label"),
                &draft::label_text(&o.draft.label),
                true,
            )
            .select_trigger(o.select == Some(Select::Label), |a: &mut App| {
                toggle_select(a, Select::Label)
            }),
        );
    if o.select == Some(Select::Label) {
        let opts = draft::LABELS
            .iter()
            .map(|(c, k)| (c.to_string(), t(k)))
            .collect();
        select_menu(tk, layer, "txd-label", opts, &o.draft.label, |a, v| {
            if let Some(o) = a.detail.open.as_mut() {
                o.draft.label = v.to_owned();
            }
        });
    }
    let review_sel = col()
        .items_start()
        .gap(tk.r(0.5))
        .child(super::widgets::field_label_ink(
            tk,
            &t("transactions:classify.reviewStatus"),
            Some(t("transactions:classify.reviewStatusHint")),
            dirt.review_status,
            tk.muted_fg,
        ))
        .child(
            select_trigger(
                tk,
                "txd-review",
                &t("transactions:classify.reviewStatus"),
                &t(draft::status_key(&o.draft.review_status)),
                true,
            )
            .select_trigger(o.select == Some(Select::Review), |a: &mut App| {
                toggle_select(a, Select::Review)
            }),
        );
    if o.select == Some(Select::Review) {
        let opts = draft::REVIEW_STATUSES
            .iter()
            .map(|c| (c.to_string(), t(draft::status_key(c))))
            .collect();
        select_menu(
            tk,
            layer,
            "txd-review",
            opts,
            &o.draft.review_status,
            |a, v| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.draft.review_status = v.to_owned();
                }
            },
        );
    }
    // Tags.
    // `rounded-md border bg-background p-2`: the chips (`mb-2 min-h-8 flex-wrap gap-1.5`), then
    // the input and its `+` button (`flex gap-2`).
    let mut chips = row()
        .w_full()
        .min_h(tk.r(2.0))
        .mb(tk.r(0.5))
        .gap(tk.r(0.375))
        .flex_wrap()
        .items_start();
    if o.draft.tags.is_empty() {
        chips = chips.child(
            text(t("transactions:classify.noTagsYet"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .px(tk.r(0.25))
                .py(tk.r(0.25)),
        );
    }
    for (i, tag) in o.draft.tags.iter().enumerate() {
        let tg = tag.clone();
        chips = chips.child(
            row()
                .key(format!("txd-tag-{i}"))
                .px(tk.r(0.5))
                .py(tk.r(0.25))
                .gap(tk.r(0.25))
                .center_y()
                .radius(tk.r(0.375))
                .fill(tk.muted)
                .role("button")
                .label(tr("transactions:classify.removeTagAria", &[("tag", tag)]))
                .focusable(true)
                .on_press(move |a: &mut App| {
                    if let Some(o) = a.detail.open.as_mut() {
                        o.draft.tags.retain(|x| *x != tg);
                    }
                })
                .child(super::widgets::sens(
                    tk,
                    tag.clone(),
                    tk.txs(500.0),
                    tk.fg,
                    hide,
                ))
                .child(ic(&I::X, tk.r(0.75), tk.muted_fg)),
        );
    }
    let input = TextInput::new(&app.detail.tag)
        .placeholder(t("transactions:classify.addTagPlaceholder"))
        .label(t("transactions:classify.addTagAria"))
        .height(tk.r(2.25))
        .text_style(tk.tsm(400.0))
        .on_edit(|a: &mut App| &mut a.detail.tag)
        .on_change(|a: &mut App, _tx: &mut Tx| {
            if a.detail.tag.text.ends_with(',') {
                commit_tag_input(a);
            }
        })
        .on_submit(|a: &mut App, _tx: &mut Tx| commit_tag_input(a))
        .build(&k)
        .key("txd-tag-input")
        .grow(1.0);
    let input = super::widgets::field(tk, input);
    let add = super::widgets::icon_button(
        tk,
        "txd-tag-add",
        super::widgets::Variant::Outline,
        tk.r(2.25),
        &I::PLUS,
        tk.r(1.0),
        t("transactions:classify.addTagAria"),
    )
    .on_press(commit_tag_input);
    let mut sugg = row().gap(tk.r(0.375)).flex_wrap();
    for (code, key) in draft::TAG_SUGGESTIONS
        .iter()
        .filter(|(c, _)| !o.draft.tags.iter().any(|x| x.eq_ignore_ascii_case(c)))
        .take(7)
    {
        let name = key.map(t).unwrap_or_else(|| code.to_string());
        let code = code.to_string();
        sugg = sugg.child(
            row()
                .key(format!("txd-sugg-{code}"))
                .px(tk.r(0.5))
                .py(tk.r(0.25))
                .center_y()
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .hover(tk.muted)
                .role("button")
                .label(tr(
                    "transactions:classify.suggestionPrefix",
                    &[("tag", &name)],
                ))
                .focusable(true)
                .on_press(move |a: &mut App| add_tag(a, &code))
                .child(
                    text(tr(
                        "transactions:classify.suggestionPrefix",
                        &[("tag", &name)],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                ),
        );
    }
    // "(press <kbd>t</kbd> anywhere to focus)" (`text-xs font-normal text-muted-foreground`).
    let hint_src = t("transactions:classify.tagsFocusHint");
    let mut hint = row().gap(tk.r(0.25)).center_y();
    for (i, part) in hint_src.split("<0>").enumerate() {
        let (k_part, rest) = if i > 0 {
            part.split_once("</0>").unwrap_or(("", part))
        } else {
            ("", part)
        };
        if !k_part.is_empty() {
            hint = hint.child(super::widgets::kbd(tk, k_part));
        }
        if !rest.trim().is_empty() {
            hint = hint.child(
                text(rest.trim().to_owned())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
        }
    }
    let tags = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(field_label(
                    tk,
                    &t("transactions:classify.tags"),
                    None,
                    dirt.tags,
                ))
                .child(hint),
        )
        .child(
            col()
                .w_full()
                .padding(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .fill(tk.background)
                .child(chips)
                .child(row().w_full().gap(tk.r(0.5)).child(input).child(add)),
        )
        .child(sugg);
    // Note.
    // `Textarea min-h-28`.
    let note = TextArea::new(&app.detail.note)
        .placeholder(t("transactions:classify.notePlaceholder"))
        .rows(4)
        .width(None)
        .on_edit(|a: &mut App| &mut a.detail.note)
        .on_change(|a: &mut App, _: &mut Tx| sync_text_fields(a))
        .build(&k)
        .key("txd-note");
    let note = if hide { note.blur(3.5) } else { note };
    let note = super::widgets::field(tk, note.min_h(tk.r(7.0)));
    let mut body = col()
        .key("txd-classify")
        .w_full()
        .gap(tk.r(1.0))
        .child(
            row()
                .w_full()
                .gap(tk.r(1.0))
                .child(label_sel.grow(1.0).min_w(0.0))
                .child(review_sel.grow(1.0).min_w(0.0)),
        )
        .child(tags)
        .child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .child(field_label(
                    tk,
                    &t("transactions:classify.note"),
                    None,
                    dirt.note,
                ))
                .child(note),
        );
    // Only the Transactions page wires loan marking (`onMarkLoan`); Overview's host does not.
    if super::host_loans(app)
        && let Some(loans) = loan_role(app, tk, o, r)
    {
        body = body.child(loans);
    }
    body
}

/// Copies the text fields into the draft (each field's `on_change`).
pub fn sync_text_fields(app: &mut App) {
    let note = app.detail.note.text.clone();
    let (cur, price, val, src) = (
        app.detail.cur.text.clone(),
        app.detail.price.text.clone(),
        app.detail.value.text.clone(),
        app.detail.source.text.clone(),
    );
    if let Some(o) = app.detail.open.as_mut()
        && o.record.is_some()
    {
        o.draft.note = note;
        o.draft.manual_currency = cur;
        o.draft.manual_price = price;
        o.draft.manual_value = val;
        o.draft.manual_source = src;
    }
}

pub fn loan_of(app: &App, id: &str) -> Option<Value> {
    app.detail
        .loans
        .as_ref()?
        .iter()
        .find(|m| s(m, "transaction_id") == id)
        .cloned()
}

fn loan_role(app: &App, tk: &Tk, o: &Open, r: &Value) -> Option<El> {
    let incoming = draft::f(r, "amountSat").unwrap_or(0.0) >= 0.0;
    let mark = loan_of(app, s(r, "id"));
    let role = mark.as_ref().map(|m| s(m, "role").to_owned());
    let mut items: Vec<(&str, Option<&'static str>)> = vec![];
    match &role {
        Some(rl) => {
            if !incoming && rl != "loan_principal_repaid" {
                items.push((
                    "transactions:table.row.collateral.changePrincipalRepaid",
                    Some("principal-repaid"),
                ));
            }
            if incoming && rl != "loan_principal_received" {
                items.push((
                    "transactions:table.row.collateral.changePrincipalReceived",
                    Some("principal-received"),
                ));
            }
            items.push(("transactions:table.row.collateral.unmark", None));
        }
        None if incoming => {
            items.push((
                "transactions:table.row.collateral.markPrincipalReceived",
                Some("principal-received"),
            ));
            items.push((
                "transactions:table.row.collateral.markReturned",
                Some("returned"),
            ));
        }
        None => {
            items.push((
                "transactions:table.row.collateral.markPrincipalRepaid",
                Some("principal-repaid"),
            ));
            items.push((
                "transactions:table.row.collateral.markCollateral",
                Some("collateral"),
            ));
        }
    }
    let busy = o.busy.loan || o.saving;
    let mut btns = row()
        .w_full()
        .padding(tk.r(0.5))
        .gap(tk.r(0.375))
        .flex_wrap()
        .radius(tk.r(0.375))
        .border(1.0, tk.border);
    for (i, (key, as_)) in items.into_iter().enumerate() {
        let a2 = as_;
        let mut b = small_button(tk, format!("txd-loan-{i}"), None, &t(key), true, !busy);
        if !busy {
            b = b.on_press(move |a: &mut App| super::actions::loan_mark(a, a2));
        }
        btns = btns.child(b);
    }
    Some(
        col()
            .w_full()
            .gap(tk.r(0.375))
            .child(field_label(
                tk,
                &t("transactions:classify.loanRole"),
                Some(t("transactions:classify.loanRoleHint")),
                false,
            ))
            .child(btns),
    )
}

// ---- Pricing (§4.3) --------------------------------------------------------------------------------

fn choose_pricing(a: &mut App, value: &str) {
    let Some(o) = a.detail.open.as_mut() else {
        return;
    };
    let current = draft::pricing_choice(
        o.draft.pricing_source_kind.as_deref(),
        &o.draft.pricing_quality,
    );
    if current == value {
        return;
    }
    let base_choice = draft::pricing_choice(
        o.base.pricing_source_kind.as_deref(),
        &o.base.pricing_quality,
    );
    if value == base_choice {
        o.draft.pricing_source_kind = o.base.pricing_source_kind.clone();
        o.draft.pricing_quality = o.base.pricing_quality.clone();
    } else if let Some((_, _, kind, quality)) = draft::PRICING_CHOICES.iter().find(|c| c.0 == value)
    {
        o.draft.pricing_source_kind = kind.map(str::to_owned);
        o.draft.pricing_quality = quality.to_string();
    }
}

pub fn set_manual_exact(a: &mut App) {
    if let Some(o) = a.detail.open.as_mut() {
        o.draft.pricing_source_kind = Some("manual_override".into());
        o.draft.pricing_quality = "exact".into();
        o.tab = Tab::Pricing;
    }
    a.detail.price.focused = true;
}

fn on_price(a: &mut App, _tx: &mut Tx) {
    let amount = a
        .detail
        .open
        .as_ref()
        .and_then(|o| o.record.as_ref())
        .map(draft::amount_btc)
        .unwrap_or(0.0);
    let v = match draft::parse_decimal(&a.detail.price.text) {
        Some(p) if amount > 0.0 => format!("{:.2}", p * amount),
        _ => String::new(),
    };
    a.detail.value = rune::TextEdit::new(v);
    if let Some(o) = a.detail.open.as_mut() {
        o.draft.pricing_source_kind = Some("manual_override".into());
        o.draft.pricing_quality = "exact".into();
    }
    sync_text_fields(a);
}

fn on_value(a: &mut App, _tx: &mut Tx) {
    let amount = a
        .detail
        .open
        .as_ref()
        .and_then(|o| o.record.as_ref())
        .map(draft::amount_btc)
        .unwrap_or(0.0);
    let p = match draft::parse_decimal(&a.detail.value.text) {
        Some(v) if amount > 0.0 => format!("{:.2}", v / amount),
        _ => String::new(),
    };
    let mut e = rune::TextEdit::new(p);
    e.focused = false;
    a.detail.price = e;
    if let Some(o) = a.detail.open.as_mut() {
        o.draft.pricing_source_kind = Some("manual_override".into());
        o.draft.pricing_quality = "exact".into();
    }
    sync_text_fields(a);
}

pub fn pricing(app: &App, tk: &Tk, o: &Open, r: &Value) -> El {
    let hide = app.prefs.hide_sensitive();
    let k = super::widgets::form_kit(tk);
    let dirt = o.dirty();
    let choice = draft::pricing_choice(
        o.draft.pricing_source_kind.as_deref(),
        &o.draft.pricing_quality,
    );
    // `grid gap-3 @sm:grid-cols-2 @3xl:grid-cols-4` against the tab column's width.
    // The tab panel's laid-out width; the maximized two-column layout keeps the tabs in a 28.75 rem
    // column, so the panel width alone does not say it.
    let measured = super::view::tab_width();
    let col_w = if measured > 0.0 {
        measured
    } else {
        app.detail.width - 2.0 * tk.r(1.5)
    };
    let per_row = if col_w >= tk.r(48.0) {
        4
    } else if col_w >= tk.r(24.0) {
        2
    } else {
        1
    };
    let mut cards_list: Vec<El> = Vec::new();
    for (v, key, _, _) in draft::PRICING_CHOICES {
        let selected = choice == v;
        // `flex items-start gap-2.5 rounded-md border p-3`, selected `border-primary bg-accent`;
        // the radio: `mt-0.5 size-4 rounded-full border` with a `size-2` dot.
        cards_list.push(
            row()
                .key(format!("txd-pricing-{v}"))
                .min_w(0.0)
                .grow(1.0)
                .padding(tk.r(0.75))
                .gap(tk.r(0.625))
                .items_start()
                .radius(tk.r(0.375))
                .border(1.0, if selected { tk.primary } else { tk.border })
                .fill(if selected {
                    tk.accent
                } else {
                    Color::TRANSPARENT
                })
                .hover(if selected {
                    tk.accent
                } else {
                    tk.muted.alpha(0.5)
                })
                .role("radio")
                .checked(selected)
                .label(t(&format!("transactions:pricingOption.{key}.label")))
                .focusable(true)
                .on_press(move |a: &mut App| {
                    if !selected {
                        choose_pricing(a, v)
                    }
                })
                .child(
                    row()
                        .size(tk.r(1.0), tk.r(1.0))
                        .mt(tk.r(0.125))
                        .radius(tk.r(0.5))
                        .center()
                        .border(
                            1.0,
                            if selected {
                                tk.primary
                            } else {
                                tk.muted_fg.alpha(0.4)
                            },
                        )
                        .child(if selected {
                            el().size(tk.r(0.5), tk.r(0.5))
                                .radius(tk.r(0.25))
                                .fill(tk.primary)
                        } else {
                            el()
                        }),
                )
                .child(
                    col()
                        .min_w(0.0)
                        .grow(1.0)
                        .gap(tk.r(0.25))
                        .child(
                            text(t(&format!("transactions:pricingOption.{key}.label")))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        )
                        .child(
                            text(t(&format!("transactions:pricingOption.{key}.description")))
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .wrap(),
                        ),
                ),
        );
    }
    let mut cards = col()
        .w_full()
        .gap(tk.r(0.75))
        .role("radiogroup")
        .label(t("transactions:pricing.sourceAria"));
    let mut it = cards_list.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row().w_full().gap(tk.r(0.75)).items_stretch();
        for _ in 0..per_row {
            r = r.child(it.next().unwrap_or_else(|| el().grow(1.0)));
        }
        cards = cards.child(r);
    }
    let mut body = col()
        .key("txd-pricing")
        .w_full()
        .gap(tk.r(1.0))
        .child(cards);
    let amount = draft::amount_btc(r);
    if o.draft.pricing_source_kind.as_deref() == Some("manual_override") {
        let field =
            |label: &str, edit: &rune::TextEdit, key: &'static str, ph: Option<String>| -> El {
                let mut i = TextInput::new(edit)
                    .label(label.to_owned())
                    .height(tk.r(2.25))
                    .text_style(tk.tsm(400.0));
                if let Some(p) = ph {
                    i = i.placeholder(p);
                }
                let built = match key {
                    "txd-manual-currency" => i.on_edit(|a: &mut App| &mut a.detail.cur).on_change(
                        |a: &mut App, _: &mut Tx| {
                            let up: String =
                                a.detail.cur.text.to_uppercase().chars().take(3).collect();
                            if up != a.detail.cur.text {
                                let mut e = rune::TextEdit::new(up);
                                e.focused = true;
                                a.detail.cur = e;
                            }
                            sync_text_fields(a);
                        },
                    ),
                    "txd-manual-price" => i
                        .on_edit(|a: &mut App| &mut a.detail.price)
                        .on_change(on_price),
                    "txd-manual-value" => i
                        .on_edit(|a: &mut App| &mut a.detail.value)
                        .on_change(on_value),
                    _ => i
                        .on_edit(|a: &mut App| &mut a.detail.source)
                        .on_change(|a: &mut App, _: &mut Tx| sync_text_fields(a)),
                }
                .build(&k)
                .key(key);
                super::widgets::field(tk, built)
            };
        let labeled = |label: &str, input: El| {
            col()
                .min_w(0.0)
                .gap(tk.r(0.5))
                .child(field_label(tk, label, None, false))
                .child(input)
        };
        // `grid gap-3 rounded-md border bg-muted/50 p-3`.
        let badge_tone = (
            if tk.dark {
                rune::color::hex("#fcd34d").unwrap()
            } else {
                rune::color::hex("#b45309").unwrap()
            },
            rune::color::hex("#78350f")
                .unwrap()
                .alpha(if tk.dark { 0.25 } else { 0.0 }),
            rune::color::hex("#d97706").unwrap().alpha(0.3),
        );
        let editor = col()
            .key("txd-manual")
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .fill(tk.muted.alpha(0.5))
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .center_y()
                    .justify_between()
                    .flex_wrap()
                    .child(
                        col()
                            .child(
                                row()
                                    .gap(tk.r(0.375))
                                    .center_y()
                                    .child(
                                        text(t("transactions:pricing.manualOverride"))
                                            .font(tk.tsm(500.0))
                                            .color(tk.fg),
                                    )
                                    .children(dirt.pricing.then(|| dirty_dot(tk)))
                                    .child(hint(
                                        tk,
                                        &t("transactions:pricing.manualOverride"),
                                        t("transactions:pricing.manualOverrideHint"),
                                    )),
                            )
                            .child(
                                text(tr(
                                    "transactions:pricing.calculatedFromAmount",
                                    &[("amount", &draft::btc_amount(amount))],
                                ))
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg),
                            ),
                    )
                    .child(super::widgets::badge(
                        tk,
                        &draft::source_kind_label("manual_override"),
                        badge_tone.0,
                        badge_tone.1,
                        badge_tone.2,
                    )),
            )
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .child(
                        labeled(
                            &t("transactions:pricing.currency"),
                            field(
                                &t("transactions:pricing.currency"),
                                &app.detail.cur,
                                "txd-manual-currency",
                                None,
                            ),
                        )
                        .w(100.0),
                    )
                    .child(
                        labeled(
                            &t("transactions:pricing.pricePerBtc"),
                            field(
                                &t("transactions:pricing.pricePerBtc"),
                                &app.detail.price,
                                "txd-manual-price",
                                Some(t("transactions:pricing.manualPricePlaceholder")),
                            ),
                        )
                        .grow(1.0),
                    )
                    .child(
                        labeled(
                            &t("transactions:pricing.totalValue"),
                            field(
                                &t("transactions:pricing.totalValue"),
                                &app.detail.value,
                                "txd-manual-value",
                                Some(t("transactions:pricing.manualValuePlaceholder")),
                            ),
                        )
                        .grow(1.0),
                    ),
            )
            .child(
                col()
                    .gap(tk.r(0.5))
                    .child(field_label(
                        tk,
                        &t("transactions:pricing.evidenceSource"),
                        Some(t("transactions:pricing.evidenceHint")),
                        false,
                    ))
                    .child(
                        field(
                            &t("transactions:pricing.evidenceSource"),
                            &app.detail.source,
                            "txd-manual-source",
                            Some(t("transactions:pricing.evidencePlaceholder")),
                        )
                        .blur(if hide { 3.5 } else { 0.0 }),
                    )
                    .child(
                        text(
                            t("transactions:pricing.attachHint")
                                .replace("<0>", "")
                                .replace("</0>", ""),
                        )
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                    ),
            );
        body = body.child(editor);
    }
    let now = app.snapshot().map(|s| s.rate()).filter(|x| *x > 0.0);
    let imported = f(r, "rate")
        .filter(|x| *x != 0.0)
        .map(|v| {
            tr(
                "transactions:pricing.perBtc",
                &[(
                    "value",
                    &format::fiat(v, &super::tax::row_currency(Some(app), r)),
                )],
            )
        })
        .unwrap_or_else(|| t("transactions:pricing.importedPriceNone"));
    let spot = now
        .map(|v| {
            tr(
                "transactions:pricing.perBtc",
                &[(
                    "value",
                    &format::fiat(
                        v,
                        &app.snapshot()
                            .map(|s| s.fiat_currency())
                            .unwrap_or_else(|| "EUR".into()),
                    ),
                )],
            )
        })
        .unwrap_or_else(|| t("transactions:pricing.spotNowUnknown"));
    // Two `DetailField`s (`rounded-md border bg-background p-3`; label row `mb-1`).
    let detail_field = |label: String, hint_text: String, v: String| -> El {
        col()
            .min_w(0.0)
            .grow(1.0)
            .padding(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .fill(tk.background)
            .child(
                row()
                    .mb(tk.r(0.25))
                    .gap(tk.r(0.375))
                    .center_y()
                    .child(
                        text(label.to_uppercase())
                            .font(tk.t2xs(500.0))
                            .color(tk.muted_fg),
                    )
                    .child(hint(tk, &label, hint_text)),
            )
            .child({
                let e = super::widgets::sens(tk, v.clone(), tk.tsm(500.0), tk.fg, hide).truncate();
                if hide { e } else { e.help_when_truncated(v) }
            })
    };
    body = body.child(
        row()
            .w_full()
            .gap(tk.r(0.75))
            .child(detail_field(
                t("transactions:pricing.importedPrice"),
                t("transactions:pricing.importedPriceHint"),
                imported,
            ))
            .child(detail_field(
                t("transactions:pricing.spotNow"),
                t("transactions:pricing.spotNowHint"),
                spot,
            )),
    );
    let provider = opt_s(r, "pricingProvider");
    let pair = opt_s(r, "pricingPair");
    let quality = o.draft.pricing_quality.clone();
    let has_provenance = provider.is_some() || pair.is_some();
    // `onOpenMarketDataSettings`: the Transactions and Review pages wire it.
    let host_settings = super::host_loans(app) || super::review_host(app).is_some();
    if has_provenance {
        let coarse = quality == "coarse_fallback";
        let title = if choice == "fmv_provider" {
            t("transactions:pricing.rateCacheSource")
        } else {
            t("transactions:pricing.priceSource")
        };
        let gran = opt_s(r, "pricingGranularity").unwrap_or_default();
        let pair_g = match (&pair, gran.is_empty()) {
            (Some(p), false) => format!("{p} · {gran}"),
            (Some(p), true) => p.clone(),
            (None, false) => gran.clone(),
            (None, true) => t("transactions:pricing.pairGranularityUnknown"),
        };
        let (moment_label, moment) = price_moment(r);
        let amber = rune::color::hex("#f59e0b").unwrap();
        let amber_icon = if tk.dark {
            rune::color::hex("#fbbf24").unwrap()
        } else {
            rune::color::hex("#d97706").unwrap()
        };
        // `rounded-md border bg-background p-3`, coarse `border-amber-500/40 bg-amber-500/10`.
        let mut head_l = row().gap(tk.r(0.375)).center_y();
        if coarse {
            head_l = head_l.child(ic(&I::TRIANGLE_ALERT, tk.r(0.75), amber_icon));
        }
        head_l = head_l.child(
            text(title.to_uppercase())
                .font(tk.t2xs(500.0))
                .color(tk.muted_fg),
        );
        let cell = |label: String, v: String| {
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(text(label).font(tk.txs(500.0)).color(tk.fg))
                .child(text(v).font(tk.txs(400.0)).color(tk.muted_fg))
        };
        let mut boxed = col()
            .key("txd-provenance")
            .w_full()
            .padding(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, if coarse { amber.alpha(0.4) } else { tk.border })
            .fill(if coarse {
                amber.alpha(0.1)
            } else {
                tk.background
            })
            .child(
                row()
                    .w_full()
                    .mb(tk.r(0.5))
                    .gap(tk.r(0.5))
                    .center_y()
                    .justify_between()
                    .child(head_l)
                    .child(super::widgets::badge(
                        tk,
                        &draft::quality_label(&quality),
                        tk.fg,
                        Color::TRANSPARENT,
                        tk.border,
                    )),
            )
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .child(cell(
                        t("transactions:pricing.provider"),
                        draft::provider_label(provider.as_deref().unwrap_or("")),
                    ))
                    .child(cell(t("transactions:pricing.pairGranularity"), pair_g))
                    .child(cell(moment_label, moment)),
            );
        if let Some(m) = opt_s(r, "pricingMethod") {
            boxed = boxed.child(
                text(tr("transactions:pricing.method", &[("method", &m)]))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .mt(tk.r(0.5)),
            );
        }
        if coarse || quality == "provider_sample" {
            let (title, body_text) = if coarse {
                (
                    t("transactions:pricing.fallbackOnly"),
                    t("transactions:pricing.fallbackOnlyBody"),
                )
            } else {
                (
                    t("transactions:pricing.providerSample"),
                    t("transactions:pricing.providerSampleBody"),
                )
            };
            let mut buttons = row().gap(tk.r(0.5)).flex_wrap().child(
                super::widgets::button_with(
                    tk,
                    "txd-exact-manual",
                    super::widgets::Variant::Outline,
                    super::widgets::Size::Sm,
                    Some(&I::SAVE),
                    Some(tk.r(0.875)),
                    &t("transactions:pricing.exactManualPrice"),
                    true,
                )
                .on_press(set_manual_exact),
            );
            if host_settings {
                buttons = buttons.child(
                    super::widgets::button_with(
                        tk,
                        "txd-rate-settings",
                        super::widgets::Variant::Outline,
                        super::widgets::Size::Sm,
                        Some(&I::SLIDERS_HORIZONTAL),
                        Some(tk.r(0.875)),
                        &t("transactions:pricing.openRateSettings"),
                        true,
                    )
                    .on_press(open_rate_settings),
                );
            }
            // `mt-3 flex-wrap justify-between gap-3 rounded-md border bg-background/70 p-3`.
            boxed = boxed.child(
                row()
                    .w_full()
                    .mt(tk.r(0.75))
                    .padding(tk.r(0.75))
                    .gap(tk.r(0.75))
                    .center_y()
                    .justify_between()
                    .flex_wrap()
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border)
                    .fill(tk.background.alpha(0.7))
                    // The text keeps its own width (`min-w-0`, no `flex-1`), so in a narrow
                    // panel it fills the line and the buttons wrap below it.
                    .child(
                        col()
                            .w_full()
                            .min_w(0.0)
                            .gap(tk.r(0.25))
                            .child(text(title).font(tk.txs(500.0)).color(tk.fg))
                            .child(
                                text(body_text)
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg)
                                    .wrap(),
                            ),
                    )
                    .child(buttons),
            );
        }
        body = body.child(boxed);
    } else if quality == "exact" {
        // `rounded-md border bg-emerald-500/10 p-3 text-xs text-muted-foreground`.
        body = body.child(
            col()
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.25))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .fill(rune::color::hex("#10b981").unwrap().alpha(0.1))
                .child(
                    text(t("transactions:pricing.exactEvidence"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(
                    text(t("transactions:pricing.exactEvidenceBody"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        );
    }
    let q_pricing = super::header::quarantine(app, o, r).is_some_and(|q| q.target == Tab::Pricing);
    if draft::pricing_missing(&o.draft, r) && !has_provenance && !q_pricing {
        let amber = rune::color::hex("#f59e0b").unwrap();
        let mut buttons = row().gap(tk.r(0.5)).mt(tk.r(0.75)).flex_wrap();
        if host_settings {
            buttons = buttons.child(
                super::widgets::button_with(
                    tk,
                    "txd-rate-settings-2",
                    super::widgets::Variant::Outline,
                    super::widgets::Size::Sm,
                    Some(&I::SLIDERS_HORIZONTAL),
                    Some(tk.r(0.875)),
                    &t("transactions:pricing.openRateSettings"),
                    true,
                )
                .on_press(open_rate_settings),
            );
        }
        buttons = buttons.child(
            super::widgets::button_with(
                tk,
                "txd-manual-exact-2",
                super::widgets::Variant::Outline,
                super::widgets::Size::Sm,
                Some(&I::SAVE),
                Some(tk.r(0.875)),
                &t("transactions:pricing.manualExactPrice"),
                true,
            )
            .on_press(set_manual_exact),
        );
        body = body.child(
            row()
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.5))
                .items_start()
                .radius(tk.r(0.375))
                .border(1.0, amber.alpha(0.4))
                .fill(amber.alpha(0.1))
                .child(
                    ic(
                        &I::TRIANGLE_ALERT,
                        tk.r(1.0),
                        if tk.dark {
                            rune::color::hex("#fbbf24").unwrap()
                        } else {
                            rune::color::hex("#d97706").unwrap()
                        },
                    )
                    .mt(tk.r(0.125)),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(t("transactions:pricing.noCachedSpot"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        )
                        .child(
                            text(t("transactions:pricing.noCachedSpotBody"))
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .wrap()
                                .mt(tk.r(0.25)),
                        )
                        .child(buttons),
                ),
        );
    }
    body
}

fn open_rate_settings(a: &mut App) {
    super::close(a);
    a.navigate("/settings#market");
}

/// `pricingPriceMoment`: daily rows show the trading day (Kraken OHLCVT CSV days end at the stamp).
fn price_moment(r: &Value) -> (String, String) {
    let ts = opt_s(r, "pricingTimestamp");
    if s(r, "pricingGranularity") == "daily" {
        let day = ts.as_deref().and_then(format::Day::parse).map(|d| {
            let shift =
                s(r, "pricingProvider") == "kraken-csv" && s(r, "pricingMethod") == "ohlcvt_csv";
            if shift {
                format::Day::from_epoch_days(d.epoch_days() - 1)
            } else {
                d
            }
        });
        return (
            t("transactions:pricePoint.tradingDay"),
            day.map(|d| d.iso()).unwrap_or_else(|| "—".into()),
        );
    }
    (
        t("transactions:pricePoint.timestamp"),
        format::short_date(ts.as_deref()),
    )
}
