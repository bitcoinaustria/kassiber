//! The More actions menu (Edit, Copy wallet export, Full rescan, Remove) and the page's
//! dialogs: Edit connection, Copy wallet export material, Remove connection, and the import
//! rollback confirmation.

use rune::{Color, El, Icon, col, el, row, text};
use rune_kit::Placement;
use serde_json::Value;

use super::actions::{
    self, SCRIPT_TYPES, backend_label, btcpay_backends, can_clear_backend, detect_material,
    edit_kind, effective_backend_kind, live_backend_source, live_backends, provenance_routes,
    route_key, script_type_label, wallet_script_types,
};
use super::ui::{self, h};
use super::{CHANGE_ACK, CLEAR_BACKEND, DELETE_ACK, REVEAL_ACK, Select, s};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant};
use crate::ui::ic;
use crate::wallets::conn::Conn;

fn close_menu(a: &mut App) {
    a.wallets.detail.menu = false;
}

fn menu_item(
    tk: &Tk,
    key: &str,
    glyph: &'static Icon,
    label: &str,
    destructive: bool,
    enabled: bool,
) -> El {
    let ink = if destructive { tk.destructive } else { tk.fg };
    let icon_ink = if destructive {
        tk.destructive
    } else {
        tk.muted_fg
    };
    row()
        .key(key.to_owned())
        .w_full()
        .h(tk.r(0.375) * 2.0 + tk.r(1.25))
        .px(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(if destructive {
            tk.destructive.alpha(0.1)
        } else {
            tk.accent
        })
        .focus_highlight(if destructive {
            tk.destructive.alpha(0.1)
        } else {
            tk.accent
        })
        .role("menuitem")
        .label(label.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.5 })
        .child(ic(glyph, tk.r(1.0), icon_ink))
        .child(text(label.to_owned()).font(tk.tsm(400.0)).color(ink))
}

/// `DropdownMenuContent align="end" className="w-56"`.
pub fn menu(app: &App, tk: &Tk, layer: &mut Vec<El>, c: &Conn) {
    let has_descriptor = super::wallet_detail(app, c)
        .and_then(|w| w.get("descriptor"))
        .and_then(Value::as_bool)
        == Some(true);
    let exchange = super::exchange_kind(app, c).is_some();
    let busy = super::sync_running(app) || app.wallets.detail.core_testing;
    let width = tk.r(14.0);
    let inner = width - 2.0 * (tk.r(0.25) + 1.0);
    let mut p = crate::ui::popup::Popup::menu("wd-more", close_menu);
    let edit_l = t("common:actions.edit");
    p.item(
        menu_item(tk, "wd-menu-edit", &I::PENCIL, &edit_l, false, true)
            .on_press(actions::open_edit),
        "wd-menu-edit",
        edit_l,
        true,
        false,
    );
    if has_descriptor {
        let l = t("connections:detail.reveal.menuItem");
        p.item(
            menu_item(tk, "wd-menu-reveal", &I::COPY, &l, false, true)
                .on_press(actions::open_reveal),
            "wd-menu-reveal",
            l,
            true,
            false,
        );
    }
    if !exchange {
        let l = t("connections:detail.fullRescan");
        let mut item = menu_item(tk, "wd-menu-rescan", &I::ROTATE_CCW, &l, false, !busy);
        if !busy {
            // Network egress: only from this press.
            item = item.on_press(|a: &mut App| {
                close_menu(a);
                actions::sync(a, true);
            });
        }
        p.item(item, "wd-menu-rescan", l, !busy, false);
    }
    p.add(
        el().h(1.0)
            .mx(-tk.r(0.25))
            .my(tk.r(0.25))
            .w(inner + tk.r(0.5))
            .fill(tk.border),
    );
    let l = t("common:actions.remove");
    p.item(
        menu_item(tk, "wd-menu-remove", &I::TRASH_2, &l, true, true).on_press(actions::open_delete),
        "wd-menu-remove",
        l,
        true,
        false,
    );
    let list = col().w(inner).label(t("connections:detail.moreActions"));
    p.mount(
        tk,
        layer,
        list,
        |l| {
            crate::shell::glass(tk, col().min_h(0.0).padding(tk.r(0.25) + 1.0).child(l))
                .w(width)
                .key("wd-menu")
        },
        Placement::BottomEnd,
        4.0,
    );
}

pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>, c: &Conn) {
    if app.wallets.detail.edit.is_some() {
        edit(app, tk, layer, c);
    }
    if app.wallets.detail.reveal.is_some() {
        reveal(app, tk, layer, c);
    }
    if app.wallets.detail.delete.is_some() {
        delete(app, tk, layer, c);
    }
    if app.wallets.detail.rollback.is_some() {
        rollback(app, tk, layer);
    }
    crate::settings::certtrust::mount(app, tk, layer, &super::CERT_HOST);
}

pub(super) fn close_edit(a: &mut App) {
    a.wallets.detail.edit = None;
    a.wallets.detail.select = None;
    a.wallets.detail.date_picker = None;
}

const EDIT_DATE_PICKER: crate::ui::date_picker::Access = crate::ui::date_picker::Access {
    get: |a| a.wallets.detail.date_picker.as_ref(),
    get_mut: |a| &mut a.wallets.detail.date_picker,
};

fn set_edit_birthday(a: &mut App, iso: String) {
    if let Some(f) = a.wallets.detail.edit.as_mut() {
        f.birthday = rune::TextEdit::new(&iso);
        f.error = None;
    }
}

fn helper_tone(tk: &Tk, s: String, warn: bool) -> El {
    let ink = if warn {
        if tk.dark { h("#fcd34d") } else { h("#b45309") }
    } else if tk.dark {
        h("#6ee7b7")
    } else {
        h("#047857")
    };
    text(s).font(tk.txs(400.0)).color(ink).wrap().w_full()
}

fn edit(app: &App, tk: &Tk, layer: &mut Vec<El>, c: &Conn) {
    // Open select lists mount above the dialog (after it), or its modal would cover them.
    let mut popups = Vec::new();
    let Some(e) = app.wallets.detail.edit.as_ref() else {
        return;
    };
    let enc = super::encrypted(app);
    let kind = edit_kind(c);
    let detail = super::wallet_detail(app, c);
    let backend_name = detail
        .and_then(|w| w.get("backend"))
        .and_then(|b| s(b, "name"))
        .map(str::to_owned);
    let chain = super::wallet_chain(app, c);
    let mut items: Vec<El> = vec![];
    items.push(ui::dialog_header(
        tk,
        &t("connections:detail.edit.title"),
        &t(if enc {
            "connections:detail.edit.descriptionEncrypted"
        } else {
            "connections:detail.edit.descriptionPlaintext"
        }),
    ));
    let label_name = t("connections:detail.edit.label");
    items.push(ui::group(
        tk,
        vec![
            ui::label(tk, &label_name),
            ui::input(
                tk,
                "wd-edit-label",
                &e.label,
                &label_name,
                "",
                |a| {
                    &mut a
                        .wallets
                        .detail
                        .edit
                        .get_or_insert_with(Default::default)
                        .label
                },
                false,
            ),
        ],
    ));
    if live_backend_source(app, c) && kind != Some("btcpay") {
        let opts = live_backends(app, c);
        let mut choices: Vec<(String, String)> = vec![(
            String::new(),
            match &backend_name {
                Some(n) => tr(
                    "connections:detail.edit.keepCurrentBackendNamed",
                    &[("name", n)],
                ),
                None => t("connections:detail.edit.keepCurrentBackend"),
            },
        )];
        if can_clear_backend(app, c) {
            choices.push((
                CLEAR_BACKEND.into(),
                t("connections:detail.edit.useDefaultBitcoinBackend"),
            ));
        }
        for b in &opts {
            let mut l = backend_label(b);
            if let Some(n) = s(b, "network") {
                l.push_str(&format!(" · {n}"));
            }
            if b.get("is_default").and_then(Value::as_bool) == Some(true) {
                l.push_str(&t("connections:detail.edit.backendOptionDefault"));
            }
            choices.push((s(b, "name").unwrap_or("").to_owned(), l));
        }
        let name = t("connections:detail.edit.syncBackend");
        let current = choices
            .iter()
            .find(|(v, _)| *v == e.backend)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        let open = app.wallets.detail.select == Some(Select::Backend);
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                crate::ui::popup::trigger(
                    ui::select_trigger(tk, "wd-edit-backend", &name, &current, open),
                    crate::ui::popup::Kind::Listbox,
                    open,
                    |a: &mut App| {
                        a.wallets.detail.select =
                            if a.wallets.detail.select == Some(Select::Backend) {
                                None
                            } else {
                                Some(Select::Backend)
                            }
                    },
                ),
                ui::helper(
                    tk,
                    &t(if chain == "liquid" {
                        "connections:detail.edit.liquidBackendHelper"
                    } else {
                        "connections:detail.edit.backendHelper"
                    }),
                ),
            ],
        ));
        if open {
            select_menu(tk, &mut popups, "wd-edit-backend", choices, &e.backend);
        }
    }
    if kind == Some("descriptor") {
        let name = t("connections:detail.edit.descriptorOrXpub");
        let mut g = vec![
            ui::label(tk, &name),
            ui::textarea(
                tk,
                "wd-edit-material",
                &e.material,
                &name,
                &t("connections:detail.edit.materialPlaceholder"),
                tk.txs(400.0).mono(),
                tk.r(6.0),
                |a| {
                    &mut a
                        .wallets
                        .detail
                        .edit
                        .get_or_insert_with(Default::default)
                        .material
                },
            ),
        ];
        let (dk, dl, dh) = detect_material(&e.material.text);
        if e.material.text.trim().is_empty() {
            g.push(ui::helper(tk, &t("connections:detail.edit.materialHelper")));
        } else {
            let msg = match &dh {
                Some(hint) => tr(
                    "connections:detail.edit.detectedWithHint",
                    &[("label", &dl), ("hint", hint)],
                ),
                None => tr("connections:detail.edit.detected", &[("label", &dl)]),
            };
            g.push(helper_tone(tk, msg, dk == "unknown"));
        }
        if dk == "bare-xpub" || !wallet_script_types(app, c).is_empty() {
            let mut boxes = col().w_full().gap(tk.r(0.375));
            for v in SCRIPT_TYPES {
                let on = e.script_types.iter().any(|x| x == v);
                let lbl = script_type_label(v);
                boxes = boxes.child(ui::checkbox_row(
                    tk,
                    &format!("wd-edit-st-{v}"),
                    on,
                    true,
                    &lbl,
                    text(lbl.clone()).font(tk.tsm(400.0)).color(tk.fg),
                    move |a: &mut App| {
                        if let Some(f) = a.wallets.detail.edit.as_mut() {
                            if let Some(i) = f.script_types.iter().position(|x| x == v) {
                                f.script_types.remove(i);
                            } else {
                                f.script_types.push(v.to_owned());
                            }
                        }
                    },
                ));
            }
            g.push(
                col()
                    .w_full()
                    .padding(tk.r(0.75))
                    .gap(tk.r(0.5))
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border)
                    .border_layout()
                    .fill(tk.muted.alpha(0.2))
                    .child(ui::label(
                        tk,
                        &t("connections:detail.edit.scriptTypesLabel"),
                    ))
                    .child(boxes)
                    .child(ui::helper(
                        tk,
                        &t("connections:detail.edit.scriptTypesHelper"),
                    )),
            );
        }
        items.push(ui::group(tk, g));
    }
    let backend_kind = effective_backend_kind(app, c, e);
    if kind == Some("descriptor") || backend_kind.as_deref() == Some("bitcoinrpc") {
        let name = t("connections:detail.edit.birthday");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                crate::ui::date_picker::render(
                    app,
                    tk,
                    &mut popups,
                    EDIT_DATE_PICKER,
                    "wd-edit-birthday",
                    &name,
                    &e.birthday.text,
                    !e.pending,
                    true,
                    set_edit_birthday,
                ),
                ui::helper(
                    tk,
                    &t(if backend_kind.as_deref() == Some("bitcoinrpc") {
                        "connections:detail.edit.birthdayCoreHelper"
                    } else {
                        "connections:detail.edit.birthdayHelper"
                    }),
                ),
            ],
        ));
    }
    if kind == Some("descriptor") {
        let name = t("connections:detail.edit.gapLimit");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                ui::input(
                    tk,
                    "wd-edit-gap",
                    &e.gap,
                    &name,
                    "40",
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .edit
                            .get_or_insert_with(Default::default)
                            .gap
                    },
                    false,
                ),
                ui::helper(tk, &t("connections:detail.edit.gapLimitHelper")),
            ],
        ));
    }
    if kind == Some("btcpay") {
        let mut choices: Vec<(String, String)> = vec![(
            String::new(),
            t("connections:detail.edit.keepCurrentInstance"),
        )];
        for b in btcpay_backends(app) {
            let mut l = backend_label(&b);
            if b.get("is_default").and_then(Value::as_bool) == Some(true) {
                l.push_str(&t("connections:detail.edit.backendOptionDefault"));
            }
            choices.push((s(&b, "name").unwrap_or("").to_owned(), l));
        }
        let name = t("connections:detail.edit.btcpayInstance");
        let current = choices
            .iter()
            .find(|(v, _)| *v == e.backend)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        let open = app.wallets.detail.select == Some(Select::BtcpayInstance);
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                crate::ui::popup::trigger(
                    ui::select_trigger(tk, "wd-edit-instance", &name, &current, open),
                    crate::ui::popup::Kind::Listbox,
                    open,
                    |a: &mut App| {
                        a.wallets.detail.select =
                            if a.wallets.detail.select == Some(Select::BtcpayInstance) {
                                None
                            } else {
                                Some(Select::BtcpayInstance)
                            }
                    },
                ),
            ],
        ));
        if open {
            select_menu(tk, &mut popups, "wd-edit-instance", choices, &e.backend);
        }
        let n1 = t("connections:detail.edit.btcpayStoreId");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &n1),
                ui::input(
                    tk,
                    "wd-edit-store",
                    &e.store_id,
                    &n1,
                    &t("connections:detail.edit.storeIdPlaceholder"),
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .edit
                            .get_or_insert_with(Default::default)
                            .store_id
                    },
                    false,
                ),
            ],
        ));
        let n2 = t("connections:detail.edit.btcpayWallet");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &n2),
                ui::input(
                    tk,
                    "wd-edit-payment",
                    &e.payment_method,
                    &n2,
                    &t("connections:detail.edit.paymentMethodPlaceholder"),
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .edit
                            .get_or_insert_with(Default::default)
                            .payment_method
                    },
                    false,
                ),
            ],
        ));
    }
    if kind == Some("file-wallet") {
        let name = t("connections:detail.edit.sourceFile");
        let label = c.label.clone();
        let browse = ui::btn(
            tk,
            "wd-edit-browse",
            Variant::Outline,
            Size::Default,
            None,
            &t("connections:detail.edit.browse"),
            true,
        )
        .on_press_tx(move |_a: &mut App, tx: &mut rune::Tx| {
            let title = tr(
                "connections:detail.edit.selectExportFileTitle",
                &[("label", &label)],
            );
            tx.open_file(
                rune::FileDialogOptions {
                    title,
                    ..Default::default()
                },
                |a: &mut App, result, _tx| {
                    if let (Ok(Some(p)), Some(f)) = (result, a.wallets.detail.edit.as_mut()) {
                        f.source_file = rune::TextEdit::new(p.to_string_lossy().into_owned());
                    }
                },
            );
        });
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .child(ui::input(
                        tk,
                        "wd-edit-source",
                        &e.source_file,
                        &name,
                        &t("connections:detail.edit.sourceFilePlaceholder"),
                        |a| {
                            &mut a
                                .wallets
                                .detail
                                .edit
                                .get_or_insert_with(Default::default)
                                .source_file
                        },
                        false,
                    ))
                    .child(browse),
            ],
        ));
    }
    let routes = provenance_routes(app, c);
    if !routes.is_empty() {
        let mut list = col().w_full().gap(tk.r(0.5));
        for (i, r) in routes.iter().enumerate() {
            let key = route_key(r);
            let removed = e.clear_provenance || e.removed_routes.contains(&key);
            let sub = format!(
                "{} · {} · {}",
                s(r, "backend").unwrap_or(""),
                s(r, "store_id").unwrap_or(""),
                s(r, "payment_method_id").unwrap_or("")
            );
            let content = col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(t("connections:detail.edit.keepProvenanceRoute"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    text(sub.clone())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate(),
                );
            let k2 = key.clone();
            list = list.child(
                ui::checkbox_row(
                    tk,
                    &format!("wd-edit-route-{i}"),
                    !removed,
                    !e.clear_provenance,
                    &format!("{} {sub}", t("connections:detail.edit.keepProvenanceRoute")),
                    content,
                    move |a: &mut App| {
                        if let Some(f) = a.wallets.detail.edit.as_mut() {
                            if let Some(p) = f.removed_routes.iter().position(|x| *x == k2) {
                                f.removed_routes.remove(p);
                            } else {
                                f.removed_routes.push(k2.clone());
                            }
                        }
                    },
                )
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border.alpha(tk.border.a * 0.6))
                .border_layout(),
            );
        }
        let clear_content = col()
            .gap(tk.r(0.125))
            .child(
                text(t("connections:detail.edit.clearProvenance"))
                    .font(tk.tsm(400.0))
                    .color(tk.fg),
            )
            .child(
                text(tc(
                    "connections:detail.edit.clearProvenanceHelper",
                    routes.len(),
                    &[],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
            );
        items.push(
            col()
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                .border_layout()
                .child(
                    col()
                        .gap(tk.r(0.25))
                        .child(
                            text(t("connections:detail.edit.provenanceRoutes"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        )
                        .child(ui::helper(
                            tk,
                            &t("connections:detail.edit.provenanceRoutesHelper"),
                        )),
                )
                .child(list)
                .child(ui::checkbox_row(
                    tk,
                    "wd-edit-clear-routes",
                    e.clear_provenance,
                    true,
                    &t("connections:detail.edit.clearProvenance"),
                    clear_content,
                    |a: &mut App| {
                        if let Some(f) = a.wallets.detail.edit.as_mut() {
                            f.clear_provenance = !f.clear_provenance;
                            if f.clear_provenance {
                                f.removed_routes.clear();
                            }
                        }
                    },
                )),
        );
    }
    let dep = col()
        .gap(tk.r(0.125))
        .grow(1.0)
        .min_w(0.0)
        .child(
            text(t("connections:detail.edit.deprecated"))
                .font(tk.tsm(400.0))
                .color(tk.fg),
        )
        .child(
            text(t("connections:detail.edit.deprecatedHelper"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
    items.push(
        col()
            .w_full()
            .padding(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, tk.border.alpha(tk.border.a * 0.7))
            .border_layout()
            .child(ui::checkbox_row(
                tk,
                "wd-edit-deprecated",
                e.deprecated,
                true,
                &t("connections:detail.edit.deprecated"),
                dep,
                |a: &mut App| {
                    if let Some(f) = a.wallets.detail.edit.as_mut() {
                        f.deprecated = !f.deprecated;
                    }
                },
            )),
    );
    if enc {
        let name = t("connections:detail.edit.passphrase");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                ui::input(
                    tk,
                    "wd-edit-pass",
                    &e.passphrase,
                    &name,
                    "",
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .edit
                            .get_or_insert_with(Default::default)
                            .passphrase
                    },
                    true,
                ),
            ],
        ));
    } else {
        let name = t("connections:detail.edit.plaintextChallenge");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                ui::input_submit(
                    tk,
                    "wd-edit-ack",
                    &e.ack,
                    &name,
                    CHANGE_ACK,
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .edit
                            .get_or_insert_with(Default::default)
                            .ack
                    },
                    false,
                    actions::submit_edit,
                ),
            ],
        ));
    }
    if let Some(err) = &e.error {
        items.push(ui::error_line(tk, err));
    }
    let save = ui::btn(
        tk,
        "wd-edit-save",
        Variant::Default,
        Size::Default,
        None,
        &t(if e.pending {
            "connections:detail.edit.saving"
        } else {
            "common:actions.save"
        }),
        !e.pending,
    )
    .on_press(actions::submit_edit);
    items.push(ui::dialog_footer(
        tk,
        vec![
            ui::btn(
                tk,
                "wd-edit-cancel",
                Variant::Outline,
                Size::Default,
                None,
                &t("common:actions.cancel"),
                true,
            )
            .on_press(close_edit),
            save,
        ],
    ));
    let width = tk.r(32.0);
    ui::dialog(
        app,
        tk,
        layer,
        "wd-edit",
        width,
        &t("connections:detail.edit.title"),
        col().w_full().gap(tk.r(1.0)).children(items),
        close_edit,
    );
    layer.extend(popups);
}

/// A native `<select>`'s option list, under its trigger.
fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    options: Vec<(String, String)>,
    current: &str,
) {
    let close = |a: &mut App| a.wallets.detail.select = None;
    let mut p = crate::ui::popup::Popup::listbox(anchor, close);
    let list = col()
        .w(tk.r(26.5))
        .padding(tk.r(0.25))
        .label(anchor.to_owned());
    for (v, label) in options {
        let checked = v == current;
        let val = v.clone();
        p.item(
            row()
                .w_full()
                .min_h(tk.r(2.0))
                .pr(tk.r(0.5))
                .gap(tk.r(0.375))
                .center_y()
                .radius(tk.r(0.25))
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .role("option")
                .checked(checked)
                .label(label.clone())
                .focusable(true)
                .focus_selected(checked)
                .on_press(move |a: &mut App| {
                    a.wallets.detail.select = None;
                    if let Some(f) = a.wallets.detail.edit.as_mut() {
                        f.backend = val.clone();
                    }
                })
                .child(row().w(tk.r(1.5)).center().child(if checked {
                    ic(&I::CHECK, tk.r(0.875), tk.fg)
                } else {
                    el().size(1.0, 1.0)
                }))
                .child(
                    text(label.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .grow(1.0),
                ),
            format!("{anchor}-opt-{v}"),
            label,
            true,
            checked,
        );
    }
    let p = p.max_h(tk.r(20.0));
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
        Placement::BottomStart,
        4.0,
    );
}

fn close_reveal(a: &mut App) {
    a.wallets.detail.reveal = None;
}

fn reveal(app: &App, tk: &Tk, layer: &mut Vec<El>, c: &Conn) {
    let Some(r) = app.wallets.detail.reveal.as_ref() else {
        return;
    };
    let enc = super::encrypted(app);
    let change = super::wallet_detail(app, c)
        .and_then(|w| w.get("change_descriptor"))
        .and_then(Value::as_bool)
        == Some(true);
    let mut items: Vec<El> = vec![ui::dialog_header(
        tk,
        &t("connections:detail.reveal.title"),
        &tr(
            if enc {
                "connections:detail.reveal.descriptionEncrypted"
            } else {
                "connections:detail.reveal.descriptionPlaintext"
            },
            &[("label", &c.label)],
        ),
    )];
    if change {
        items.push(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.muted.alpha(0.3))
                .child(
                    text(t("connections:detail.reveal.includesChange"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    if enc {
        let name = t("connections:detail.reveal.passphrase");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                ui::input(
                    tk,
                    "wd-reveal-pass",
                    &r.passphrase,
                    &name,
                    "",
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .reveal
                            .get_or_insert_with(Default::default)
                            .passphrase
                    },
                    true,
                ),
                ui::helper(tk, &t("connections:detail.reveal.helper")),
            ],
        ));
    } else {
        let name = t("connections:detail.reveal.plaintextChallenge");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                ui::input_submit(
                    tk,
                    "wd-reveal-ack",
                    &r.ack,
                    &name,
                    REVEAL_ACK,
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .reveal
                            .get_or_insert_with(Default::default)
                            .ack
                    },
                    false,
                    actions::submit_reveal,
                ),
                ui::helper(tk, &t("connections:detail.reveal.plaintextHelper")),
            ],
        ));
    }
    if let Some(e) = &r.error {
        items.push(ui::error_line(tk, e));
    }
    if r.copied {
        items.push(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(ic(&I::CIRCLE_CHECK, tk.r(1.0), ui::emerald_text(tk)))
                .child(
                    text(t("connections:detail.reveal.copiedInline"))
                        .font(tk.tsm(400.0))
                        .color(ui::emerald_text(tk)),
                ),
        );
    }
    let label = t(if r.copied {
        "connections:detail.reveal.copied"
    } else if r.pending {
        "connections:detail.reveal.copying"
    } else {
        "connections:detail.reveal.copy"
    });
    let enabled = !r.pending && !r.copied;
    let mut copy = ui::btn(
        tk,
        "wd-reveal-copy",
        Variant::Default,
        Size::Default,
        Some(if r.copied { &I::CIRCLE_CHECK } else { &I::COPY }),
        &label,
        enabled,
    );
    if enabled {
        copy = copy.on_press(actions::submit_reveal);
    }
    items.push(ui::dialog_footer(
        tk,
        vec![
            ui::btn(
                tk,
                "wd-reveal-cancel",
                Variant::Outline,
                Size::Default,
                None,
                &t("common:actions.cancel"),
                true,
            )
            .on_press(close_reveal),
            copy,
        ],
    ));
    ui::dialog(
        app,
        tk,
        layer,
        "wd-reveal",
        tk.r(32.0),
        &t("connections:detail.reveal.title"),
        col().w_full().gap(tk.r(1.0)).children(items),
        close_reveal,
    );
}

fn close_delete(a: &mut App) {
    a.wallets.detail.delete = None;
}

fn delete(app: &App, tk: &Tk, layer: &mut Vec<El>, c: &Conn) {
    let Some(d) = app.wallets.detail.delete.as_ref() else {
        return;
    };
    let enc = super::encrypted(app);
    let challenge = t(if enc {
        "connections:detail.delete.challengePassphrase"
    } else {
        "connections:detail.delete.challengePlaintext"
    });
    let mut items: Vec<El> = vec![ui::dialog_header(
        tk,
        &t("connections:detail.delete.title"),
        &tr(
            "connections:detail.delete.description",
            &[("label", &c.label), ("challenge", &challenge)],
        ),
    )];
    if enc {
        let name = t("connections:detail.delete.passphrase");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                ui::input(
                    tk,
                    "wd-delete-pass",
                    &d.passphrase,
                    &name,
                    "",
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .delete
                            .get_or_insert_with(Default::default)
                            .passphrase
                    },
                    true,
                ),
            ],
        ));
    } else {
        let name = t("connections:detail.delete.plaintextChallenge");
        items.push(ui::group(
            tk,
            vec![
                ui::label(tk, &name),
                ui::input(
                    tk,
                    "wd-delete-ack",
                    &d.ack,
                    &name,
                    DELETE_ACK,
                    |a| {
                        &mut a
                            .wallets
                            .detail
                            .delete
                            .get_or_insert_with(Default::default)
                            .ack
                    },
                    false,
                ),
            ],
        ));
    }
    let name = t("connections:detail.delete.connectionLabel");
    items.push(ui::group(
        tk,
        vec![
            ui::label(tk, &name),
            ui::input_submit(
                tk,
                "wd-delete-label",
                &d.confirm,
                &name,
                &c.label,
                |a| {
                    &mut a
                        .wallets
                        .detail
                        .delete
                        .get_or_insert_with(Default::default)
                        .confirm
                },
                false,
                actions::submit_delete,
            ),
        ],
    ));
    if let Some(e) = &d.error {
        items.push(ui::error_line(tk, e));
    }
    let mut remove = ui::destructive_button(
        tk,
        "wd-delete-submit",
        &t(if d.pending {
            "connections:detail.delete.removing"
        } else {
            "common:actions.remove"
        }),
        !d.pending,
    );
    if !d.pending {
        remove = remove.on_press(actions::submit_delete);
    }
    items.push(ui::dialog_footer(
        tk,
        vec![
            ui::btn(
                tk,
                "wd-delete-cancel",
                Variant::Outline,
                Size::Default,
                None,
                &t("common:actions.cancel"),
                true,
            )
            .on_press(close_delete),
            remove,
        ],
    ));
    ui::dialog(
        app,
        tk,
        layer,
        "wd-delete",
        tk.r(32.0),
        &t("connections:detail.delete.title"),
        col().w_full().gap(tk.r(1.0)).children(items),
        close_delete,
    );
}

fn close_rollback(a: &mut App) {
    a.wallets.detail.rollback = None;
}

fn rollback(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let Some(r) = app.wallets.detail.rollback.as_ref() else {
        return;
    };
    let n = r
        .plan
        .as_ref()
        .and_then(|p| super::f(p, "transactions_to_delete"))
        .or_else(|| super::f(&r.batch, "rows_present"))
        .unwrap_or(0.0) as usize;
    let file = s(&r.batch, "source_filename")
        .or_else(|| s(&r.batch, "source_format"))
        .unwrap_or("")
        .to_owned();
    let mut items: Vec<El> = vec![ui::dialog_header(
        tk,
        &t("connections:detail.imports.confirmTitle"),
        &tc(
            "connections:detail.imports.confirmBody",
            n,
            &[("file", &file)],
        ),
    )];
    if let Some(also) = r
        .plan
        .as_ref()
        .and_then(|p| p.get("also_removed"))
        .and_then(Value::as_object)
        .filter(|m| !m.is_empty())
    {
        let list = also
            .iter()
            .map(|(k, v)| {
                format!(
                    "{} {k}",
                    v.as_f64()
                        .map(|x| format!("{}", x as i64))
                        .unwrap_or_else(|| v.to_string())
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        items.push(
            text(tr(
                "connections:detail.imports.alsoRemoved",
                &[("items", &list)],
            ))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .wrap()
            .w_full(),
        );
    }
    if let Some(e) = &r.error {
        items.push(ui::error_line(tk, e));
    }
    let enabled = !r.pending && !r.plan_pending && r.plan.is_some();
    let mut ok = ui::destructive_button(
        tk,
        "wd-rollback-confirm",
        &t(if r.plan_pending {
            "connections:detail.imports.loadingPlan"
        } else {
            "connections:detail.imports.confirmAction"
        }),
        enabled,
    );
    if enabled {
        ok = ok.on_press(actions::confirm_rollback);
    }
    items.push(ui::dialog_footer(
        tk,
        vec![
            ui::btn(
                tk,
                "wd-rollback-cancel",
                Variant::Ghost,
                Size::Default,
                None,
                &t("connections:detail.imports.cancel"),
                true,
            )
            .on_press(close_rollback),
            ok,
        ],
    ));
    ui::dialog(
        app,
        tk,
        layer,
        "wd-rollback",
        tk.r(32.0),
        &t("connections:detail.imports.confirmTitle"),
        col().w_full().gap(tk.r(1.0)).children(items),
        close_rollback,
    );
}

#[allow(dead_code)]
fn _c() -> Color {
    Color::TRANSPARENT
}
