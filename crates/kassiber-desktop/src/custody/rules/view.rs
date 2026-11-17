//! The rules sheet (shadcn `Sheet`, right side, `sm:max-w-lg`): linking rules with their
//! switches and a confirmed delete, saved views that open the suggestions they filter, and the
//! "Create linking rule" dialog with its selects.

use rune::{Color, Cx, El, Shadow, col, row, text};
use rune_kit::{Placement, TextInput};
use serde_json::Value;

use super::{PAIR_KIND_OPTIONS, PAIR_POLICY_OPTIONS, WALLET_KIND_OPTIONS};
use crate::app::{App, ThemeMode};
use crate::custody::kit::{self, Var};
use crate::i18n::{Lang, t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::form_kit;
use crate::ui::ic;

fn rule_name(r: &Value) -> String {
    r["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t("review:swap.rules.unnamed"))
}

/// shadcn `Switch`, pressing it toggles the rule.
fn switch(tk: &Tk, key: String, checked: bool, label: String) -> El {
    crate::ui::switch(tk, &key, checked, &label)
}

pub fn sheet(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    let st = &app.custody.rules;
    if st.open {
        sheet_view(app, cx, tk, layer);
    }
    if st.create_open {
        create_dialog(app, tk, layer);
    }
}

fn sheet_view(app: &App, _cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    let st = &app.custody.rules;
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let w = tk.r(32.0).min(win_w - tk.r(3.0));
    // `bg-background/60 backdrop-blur-xs` over the window.
    layer.push(
        crate::shell::window_backdrop("tp-rules-scrim", tk.background.alpha(0.6))
            .backdrop_blur(2.0)
            .on_press(super::close)
            .role("none"),
    );

    // Rules.
    let new_btn = kit::btn35(
        tk,
        "tp-rules-new",
        Var::Outline,
        Some(&I::PLUS),
        &t("review:swap.rules.new"),
        true,
    )
    .on_press(super::open_create);
    let mut rules = col()
        .w_full()
        .gap(tk.r(0.5))
        .role("region")
        .label(t("review:swap.rules.title"))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .justify_between()
                .child(
                    text(t("review:swap.rules.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .role("heading"),
                )
                .child(new_btn),
        )
        .child(
            text(t("review:swap.rules.explain"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    let list = st.rules.clone().unwrap_or_default();
    if st.rules_loading && st.rules.is_none() {
        rules = rules.child(
            text(t("review:swap.table.loading"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .role("status"),
        );
    } else if list.is_empty() {
        rules = rules.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(1.0))
                .justify_center()
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_dash(4.0, 3.0)
                .child(
                    text(t("review:swap.rules.empty"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else {
        let mut ul = col()
            .w_full()
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .clip_rounded(tk.r(0.375))
            .role("list");
        for (i, r) in list.iter().enumerate() {
            let id = model_s(r, "id");
            let name = rule_name(r);
            let enabled = r["enabled"].as_bool().unwrap_or(false);
            let (id1, id2, id3) = (id.clone(), id.clone(), id.clone());
            let mut li = row()
                .key(format!("tp-rule-{id}"))
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .role("listitem")
                .child(
                    col()
                        .min_w(0.0)
                        .grow(1.0)
                        .w(0.0)
                        .child(
                            text(name.clone())
                                .font(tk.tsm(500.0))
                                .color(tk.fg)
                                .truncate(),
                        )
                        .child(
                            text(super::rule_sentence(r))
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .wrap(),
                        ),
                )
                .child(
                    switch(
                        tk,
                        format!("tp-rule-switch-{id}"),
                        enabled,
                        tr("review:swap.rules.toggleNamedAria", &[("name", &name)]),
                    )
                    .on_press(move |a: &mut App| super::toggle_rule(a, &id1, enabled)),
                );
            if st.confirm_delete.as_deref() == Some(id.as_str()) {
                li = li.child(
                    row()
                        .gap(tk.r(0.25))
                        .center_y()
                        .child(
                            destructive_btn(
                                tk,
                                format!("tp-rule-delete-yes-{id}"),
                                &t("review:swap.rules.deleteConfirm"),
                            )
                            .on_press(move |a: &mut App| super::delete_rule(a, &id2)),
                        )
                        .child(
                            kit::btn(
                                tk,
                                format!("tp-rule-delete-no-{id}"),
                                Var::Ghost,
                                None,
                                &t("common:actions.cancel"),
                                true,
                            )
                            .on_press(|a: &mut App| super::ask_delete(a, None)),
                        ),
                );
            } else {
                li = li.child(
                    kit::icon_btn(
                        tk,
                        format!("tp-rule-delete-{id}"),
                        Var::Ghost,
                        tk.r(2.0),
                        &I::TRASH_2,
                        tk.r(0.875),
                        tr("review:swap.rules.deleteNamedAria", &[("name", &name)]),
                    )
                    .on_press(move |a: &mut App| super::ask_delete(a, Some(id3.clone()))),
                );
            }
            if i > 0 {
                ul = ul.child(kit::rule(tk));
            }
            ul = ul.child(li);
        }
        rules = rules.child(ul);
    }
    let mut body = col()
        .w_full()
        .px(tk.r(1.0))
        .pb(tk.r(1.5))
        .gap(tk.r(1.5))
        .child(rules);
    // Views saved earlier still filter the suggestions; new ones are not made here.
    let views: Vec<Value> = st
        .transfer_views
        .iter()
        .chain(st.swap_views.iter())
        .cloned()
        .collect();
    if !views.is_empty() {
        let mut ul = col()
            .w_full()
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .clip_rounded(tk.r(0.375))
            .role("list");
        for (i, v) in views.iter().enumerate() {
            let id = model_s(v, "id");
            let name = model_s(v, "name");
            let (id1, id2) = (id.clone(), id.clone());
            let li = row()
                .key(format!("tp-view-{id}"))
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .role("listitem")
                .child(ic(&I::STAR, tk.r(0.875), tk.muted_fg).a11y_hidden())
                .child(
                    row()
                        .key(format!("tp-view-open-{id}"))
                        .min_w(0.0)
                        .grow(1.0)
                        .w(0.0)
                        .radius(tk.r(0.125))
                        .role("button")
                        .label(name.clone())
                        .focusable(true)
                        .on_press(move |a: &mut App| super::open_view(a, &id1))
                        .child(
                            text(name.clone())
                                .font(tk.tsm(500.0))
                                .color(tk.fg)
                                .truncate(),
                        ),
                )
                .child(
                    kit::icon_btn(
                        tk,
                        format!("tp-view-delete-{id}"),
                        Var::Ghost,
                        tk.r(2.0),
                        &I::TRASH_2,
                        tk.r(0.875),
                        tr("review:swap.savedView.deleteAria", &[("name", &name)]),
                    )
                    .on_press(move |a: &mut App| super::delete_view(a, &id2)),
                );
            if i > 0 {
                ul = ul.child(kit::rule(tk));
            }
            ul = ul.child(li);
        }
        body = body.child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .role("region")
                .label(t("review:swap.savedView.title"))
                .child(
                    text(t("review:swap.savedView.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .role("heading"),
                )
                .child(
                    text(t("review:swap.savedView.explain"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(ul),
        );
    }
    if let Some(e) = &st.error {
        body = body.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    let header = col()
        .w_full()
        .padding(tk.r(1.0))
        .pr(tk.r(2.5))
        .gap(tk.r(0.375))
        .child(
            text(t("review:swap.rules.sheetTitle"))
                .font(tk.tbase(600.0))
                .color(tk.fg)
                .role("heading"),
        )
        .child(
            text(t("review:swap.rules.sheetDescription"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    let close = row()
        .key("tp-rules-close")
        .size(tk.r(1.25), tk.r(1.25))
        .center()
        .radius(tk.r(0.125))
        .opacity(0.7)
        .role("button")
        .label(t("common:actions.close"))
        .focusable(true)
        .on_press(super::close)
        .child(ic(&I::X, tk.r(1.0), tk.fg))
        .pin(None, Some(tk.r(1.0)), Some(tk.r(1.0)), None);
    let surface = col()
        .key("tp-rules-sheet")
        .w(w)
        .h(win_h)
        .gap(tk.r(1.0))
        .fill(tk.popover)
        .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
        .box_shadow(Shadow {
            y: 10.0,
            blur: 15.0,
            spread: -3.0,
            color: Color::BLACK.alpha(0.05),
            ..Default::default()
        })
        .role("dialog")
        .label(t("review:swap.rules.sheetTitle"))
        .description(t("review:swap.rules.sheetDescription"))
        .modal(true, super::close)
        .transition(crate::shell::sheet_enter(tk))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(header)
        .child(body)
        .child(close);
    // `SheetContent side="right"` as its own layer root, sliding in as the web's does.
    layer.push(surface.absolute((win_w - w).max(0.0), 0.0));
}

fn model_s(v: &Value, k: &str) -> String {
    v[k].as_str().map(str::to_owned).unwrap_or_default()
}

/// `Button variant="destructive" size="sm"` (`h-8`).
fn destructive_btn(tk: &Tk, key: String, label: &str) -> El {
    let fill = if tk.dark {
        tk.destructive.alpha(0.6)
    } else {
        tk.destructive
    };
    row()
        .key(key)
        .h(tk.r(2.0))
        .px(tk.r(0.75))
        .center()
        .radius(tk.r(0.375))
        .fill(fill)
        .hover(fill.alpha(0.9))
        .role("button")
        .label(label.to_owned())
        .focusable(true)
        .child(
            text(label.to_owned())
                .font(tk.tsm(500.0))
                .color(Color::WHITE),
        )
}

// ---- The create dialog -------------------------------------------------------------------------

/// A shadcn `Select` (`SelectTrigger` `w-full h-9`, `SelectContent` with checked items).
#[allow(clippy::too_many_arguments)]
fn select(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    label: &str,
    current: &str,
    options: Vec<(String, String)>,
    choose: fn(&mut App, &str),
) -> El {
    let shown = options
        .iter()
        .find(|(v, _)| v == current)
        .map(|(_, l)| l.clone())
        .unwrap_or_default();
    let open = app.custody.rules.select.as_deref() == Some(key);
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (Color::TRANSPARENT, tk.accent)
    };
    let k = key.to_owned();
    let trigger = row()
        .key(key.to_owned())
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .border_layout()
        .fill(fill)
        .hover(hover)
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Default::default()
        })
        .role("combobox")
        .label(label.to_owned())
        .a11y_value(shown.clone())
        .focusable(true)
        .child(text(shown).font(tk.tsm(400.0)).color(tk.fg))
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    let trigger = crate::ui::popup::trigger(
        trigger,
        crate::ui::popup::Kind::Listbox,
        open,
        move |a: &mut App| {
            let next = if a.custody.rules.select.as_deref() == Some(k.as_str()) {
                None
            } else {
                Some(k.clone())
            };
            super::set_select(a, next);
        },
    );
    if open {
        let mut p = crate::ui::popup::Popup::listbox(key, |a: &mut App| super::set_select(a, None));
        let list = col()
            .min_w(tk.r(12.0))
            .padding(tk.r(0.25))
            .items_stretch()
            .label(label.to_owned());
        for (value, text_label) in options {
            let checked = value == current;
            let val = value.clone();
            let mut item = row()
                .key(format!("{key}-opt-{value}"))
                .min_h(tk.r(2.0))
                .pl(tk.r(0.5))
                .pr(tk.r(2.0))
                .gap(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.25))
                .role("option")
                .checked(checked)
                .label(text_label.clone())
                .focusable(true)
                .focus_selected(checked)
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .on_press(move |a: &mut App| {
                    super::set_select(a, None);
                    choose(a, &val);
                })
                .child(text(text_label.clone()).font(tk.tsm(400.0)).color(tk.fg));
            if checked {
                item = item.child(
                    row()
                        .absolute(0.0, 0.0)
                        .w_full()
                        .h(tk.r(2.0))
                        .justify_end()
                        .pr(tk.r(0.5))
                        .center_y()
                        .child(ic(&I::CHECK, tk.r(1.0), tk.fg)),
                );
            }
            p.item(
                item,
                format!("{key}-opt-{value}"),
                text_label,
                true,
                checked,
            );
        }
        let p = p.max_h(tk.r(22.0));
        p.mount(
            tk,
            layer,
            list,
            |l| kit::menu_surface(tk, col().min_h(0.0).child(l)),
            Placement::BottomStart,
            4.0,
        );
    }
    trigger
}

/// A shadcn `Label` above a field (`space-y-1`).
fn labelled(tk: &Tk, label: &str, field: El) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .items_start()
        .gap(tk.r(0.25))
        .child(
            text(label.to_owned())
                .font(tk.tsm(500.0).lh(tk.r(0.875)))
                .color(tk.fg),
        )
        .child(field)
}

fn input<F>(
    tk: &Tk,
    edit: &rune::TextEdit,
    key: &str,
    label: &str,
    placeholder: &str,
    on_edit: F,
) -> El
where
    F: for<'a> Fn(&'a mut App) -> &'a mut rune::TextEdit + 'static,
{
    let k = form_kit(tk);
    let i = TextInput::new(edit)
        .label(label.to_owned())
        .placeholder(placeholder.to_owned())
        .height(tk.r(2.25))
        .width(None)
        .text_style(tk.tsm(400.0))
        .on_edit(on_edit);
    crate::txdetail::widgets::field(tk, i.build(&k).key(key.to_owned()))
        .radius(tk.r(0.375))
        .w_full()
}

fn asset_options() -> Vec<(String, String)> {
    vec![
        ("any".into(), t("review:swap.createRule.any")),
        ("BTC".into(), t("review:swap.createRule.assetBtc")),
        ("LBTC".into(), t("review:swap.createRule.assetLbtc")),
    ]
}

fn kind_options() -> Vec<(String, String)> {
    let mut v = vec![("any".to_owned(), t("review:swap.createRule.any"))];
    v.extend(WALLET_KIND_OPTIONS.iter().map(|k| {
        (
            k.to_string(),
            t(&format!("review:swap.createRule.walletKind.{k}")),
        )
    }));
    v
}

fn create_dialog(app: &App, tk: &Tk, out: &mut Vec<El>) {
    let st = &app.custody.rules;
    // A select's list is placed after the dialog, so its trigger is laid out before it.
    let mut pops: Vec<El> = vec![];
    let layer = &mut pops;
    let f = &st.form;
    let half = |a: El, b: El| {
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .child(col().w(0.0).grow(1.0).min_w(0.0).child(a))
            .child(col().w(0.0).grow(1.0).min_w(0.0).child(b))
    };
    let name = labelled(
        tk,
        &t("review:swap.createRule.nameLabel"),
        input(
            tk,
            &f.name,
            "tp-rule-name",
            &t("review:swap.createRule.nameLabel"),
            &t("review:swap.createRule.namePlaceholder"),
            |a| &mut a.custody.rules.form.name,
        ),
    );
    let out_asset = labelled(
        tk,
        &t("review:swap.createRule.outAsset"),
        select(
            app,
            tk,
            layer,
            "tp-rule-out-asset",
            &t("review:swap.createRule.outAsset"),
            &f.out_asset,
            asset_options(),
            |a, v| a.custody.rules.form.out_asset = v.to_owned(),
        ),
    );
    let in_asset = labelled(
        tk,
        &t("review:swap.createRule.inAsset"),
        select(
            app,
            tk,
            layer,
            "tp-rule-in-asset",
            &t("review:swap.createRule.inAsset"),
            &f.in_asset,
            asset_options(),
            |a, v| a.custody.rules.form.in_asset = v.to_owned(),
        ),
    );
    let out_kind = labelled(
        tk,
        &t("review:swap.createRule.outWalletKind"),
        select(
            app,
            tk,
            layer,
            "tp-rule-out-kind",
            &t("review:swap.createRule.outWalletKind"),
            &f.out_kind,
            kind_options(),
            |a, v| a.custody.rules.form.out_kind = v.to_owned(),
        ),
    );
    let in_kind = labelled(
        tk,
        &t("review:swap.createRule.inWalletKind"),
        select(
            app,
            tk,
            layer,
            "tp-rule-in-kind",
            &t("review:swap.createRule.inWalletKind"),
            &f.in_kind,
            kind_options(),
            |a, v| a.custody.rules.form.in_kind = v.to_owned(),
        ),
    );
    let max_fee = labelled(
        tk,
        &t("review:swap.createRule.maxFee"),
        input(
            tk,
            &f.max_fee,
            "tp-rule-max-fee",
            &t("review:swap.createRule.maxFee"),
            &t("review:swap.createRule.maxFeePlaceholder"),
            |a| &mut a.custody.rules.form.max_fee,
        ),
    );
    let conf = labelled(
        tk,
        &t("review:swap.createRule.minConfidence"),
        select(
            app,
            tk,
            layer,
            "tp-rule-confidence",
            &t("review:swap.createRule.minConfidence"),
            &f.min_confidence,
            vec![
                ("strong".into(), t("review:swap.createRule.minStrong")),
                ("exact".into(), t("review:swap.createRule.minExact")),
            ],
            |a, v| a.custody.rules.form.min_confidence = v.to_owned(),
        ),
    );
    let kinds: Vec<(String, String)> = PAIR_KIND_OPTIONS
        .iter()
        .map(|k| {
            (
                k.to_string(),
                t(&format!(
                    "review:transfersPage.kind.{}",
                    crate::custody::link_model::kind_key(Some(k))
                )),
            )
        })
        .collect();
    let kind = labelled(
        tk,
        &t("review:swap.createRule.kind"),
        select(
            app,
            tk,
            layer,
            "tp-rule-kind",
            &t("review:swap.createRule.kind"),
            &f.kind,
            kinds,
            |a, v| a.custody.rules.form.kind = v.to_owned(),
        ),
    );
    let policies: Vec<(String, String)> = PAIR_POLICY_OPTIONS
        .iter()
        .map(|p| {
            (
                p.to_string(),
                t(&format!(
                    "review:transfersPage.policy.{}",
                    if *p == "taxable" {
                        "taxable"
                    } else {
                        "carrying"
                    }
                )),
            )
        })
        .collect();
    let policy = labelled(
        tk,
        &t("review:swap.createRule.policy"),
        select(
            app,
            tk,
            layer,
            "tp-rule-policy",
            &t("review:swap.createRule.policy"),
            &f.policy,
            policies,
            |a, v| a.custody.rules.form.policy = v.to_owned(),
        ),
    );
    let fields = col()
        .w_full()
        .gap(tk.r(0.75))
        .child(name)
        .child(half(out_asset, in_asset))
        .child(half(out_kind, in_kind))
        .child(half(max_fee, conf))
        .child(half(kind, policy));
    let mut create = if st.creating {
        kit::btn_busy(
            tk,
            "tp-rule-create",
            Var::Default,
            true,
            None,
            &t("review:swap.createRule.createButton"),
            false,
        )
        .h(tk.r(2.25))
        .px(tk.r(1.0))
    } else {
        kit::btn_md(
            tk,
            "tp-rule-create",
            Var::Default,
            None,
            &t("review:swap.createRule.createButton"),
            true,
        )
    };
    if !st.creating {
        create = create.on_press(super::create);
    }
    let footer = kit::dialog_footer(tk)
        .child(
            kit::btn_md(
                tk,
                "tp-rule-cancel",
                Var::Outline,
                None,
                &t("common:actions.cancel"),
                true,
            )
            .on_press(super::close_create),
        )
        .child(create);
    let c = col()
        .w_full()
        .gap(tk.r(1.0))
        .label(t("review:swap.createRule.title"))
        .child(kit::dialog_header(
            tk,
            t("review:swap.createRule.title"),
            Some(t("review:swap.createRule.description")),
        ))
        .child(fields)
        .child(footer);
    kit::dialog(app, tk, out, "tp-rule-dialog", 32.0, c, super::close_create);
    out.extend(pops);
}

// ---- Scenes ------------------------------------------------------------------------------------

fn open_sheet(a: &mut App) {
    a.navigate("/transfers");
    crate::custody::prime(a);
    super::open(a);
    crate::custody::settle(a);
}

pub const SCENES: &[crate::app::Scene] = &[
    ("custody-rules", open_sheet),
    ("custody-rules-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        open_sheet(a);
    }),
    ("custody-rules-de", |a| {
        a.prefs.lang = Lang::De;
        crate::i18n::set_lang(Lang::De);
        open_sheet(a);
    }),
    ("custody-rules-new", |a| {
        open_sheet(a);
        super::open_create(a);
    }),
    ("custody-rules-new-kind", |a| {
        open_sheet(a);
        super::open_create(a);
        super::set_select(a, Some("tp-rule-kind".into()));
    }),
    ("custody-rules-confirm", |a| {
        open_sheet(a);
        let id = a
            .custody
            .rules
            .rules
            .as_ref()
            .and_then(|r| r.first())
            .and_then(|r| r["id"].as_str())
            .map(str::to_owned);
        super::ask_delete(a, id);
    }),
];
