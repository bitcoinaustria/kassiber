//! Stage 1, `TargetStage`: what needs explaining — the amount, the planned-sale options and
//! exact reference, the paginated transaction picker with its filters, and the historical
//! inbound coverage section.

use rune::{Color, Cx, El, Icon, TextEdit, col, el, row, text};
use serde_json::Value;

use super::kit::{self, emerald, red, zinc};
use super::model;
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant};
use crate::ui::ic;

/// `StageHeader`: `border-b pb-4`, an `h2` (`text-lg font-semibold tracking-tight`), the lede.
pub fn stage_header(tk: &Tk, title: &str, lede: &str, actions: Option<El>) -> El {
    let texts = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(
            text(title.to_owned())
                .font(tk.tlg(600.0).tracking(-0.45))
                .color(tk.fg)
                .role("heading")
                .level(2),
        )
        .child(
            text(lede.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .max_w(tk.r(42.0)),
        );
    let mut r = row()
        .w_full()
        .pb(tk.r(1.0))
        .gap(tk.r(0.75))
        .items_start()
        .justify_between()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(texts);
    if let Some(a) = actions {
        r = r.child(a);
    }
    r
}

fn case(a: &mut App) -> &mut super::Case {
    a.provenance.sof.case.as_mut().expect("case")
}

fn edit_amount(a: &mut App) -> &mut TextEdit {
    &mut case(a).amount
}
fn edit_target(a: &mut App) -> &mut TextEdit {
    &mut case(a).target
}
fn edit_destination(a: &mut App) -> &mut TextEdit {
    &mut case(a).planned_destination
}
fn edit_note(a: &mut App) -> &mut TextEdit {
    &mut case(a).planned_note
}
fn edit_search(a: &mut App) -> &mut TextEdit {
    &mut case(a).search
}

pub fn toggle_open(a: &mut App, k: &str) {
    if let Some(c) = a.provenance.sof.case.as_mut()
        && !c.open.remove(k)
    {
        c.open.insert(k.to_owned());
    }
}

/// `PurposeButton`.
fn purpose(
    tk: &Tk,
    key: &str,
    active: bool,
    title: String,
    body: String,
    value: &'static str,
) -> El {
    col()
        .key(key.to_owned())
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .px(tk.r(0.75))
        .py(tk.r(0.75))
        .gap(tk.r(0.25))
        .radius(tk.r(0.375))
        .border(1.0, if active { tk.primary } else { tk.border })
        .border_layout()
        .fill(if active {
            tk.primary.alpha(0.05)
        } else {
            Color::TRANSPARENT
        })
        .hover(if active {
            tk.primary.alpha(0.05)
        } else {
            tk.muted.alpha(0.6)
        })
        .role("button")
        .label(title.clone())
        .checked(active)
        .focusable(true)
        .on_press(move |a: &mut App| case(a).purpose = value.to_owned())
        .child(text(title).font(tk.tsm(600.0)).color(tk.fg))
        .child(text(body).font(tk.txs(400.0)).color(tk.muted_fg).wrap())
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let sof = &app.provenance.sof;
    let c = sof.case.as_ref().expect("case");
    let ww = crate::reports::ledger::common::win_w(app, cx);
    let planned = c.purpose == "planned_exchange_sale";
    let target_label = t(if planned {
        "sourceFunds:setup.targetLabelPlanned"
    } else {
        "sourceFunds:setup.targetLabelExisting"
    });
    let amount_label = t(if planned {
        "sourceFunds:setup.amountLabelPlanned"
    } else {
        "sourceFunds:setup.amountLabelExisting"
    });
    let mut out = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(1.25))
        .child(stage_header(
            tk,
            &t("sourceFunds:workstation.whatNeedsExplaining"),
            &t("sourceFunds:workstation.everyDossierIsAnchoredTo"),
            None,
        ));
    if !c.selected_target().is_empty() {
        out = out.child(kit::field(
            tk,
            &amount_label,
            kit::input(
                tk,
                "sof-amount",
                &c.amount,
                &amount_label,
                &t("sourceFunds:case.fullAmount"),
                edit_amount,
                tk.r(2.25),
            ),
        ));
    }
    // `<details>`: planned sale or transaction reference.
    let open = c.is_open("target-options");
    let mut details = col()
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .child(kit::summary(
            tk,
            "sof-target-options",
            open,
            &t("sourceFunds:journey.targetOptions"),
            tk.tsm(400.0),
            tk.muted_fg,
            |a| toggle_open(a, "target-options"),
        ));
    if open {
        let mut inner = col().w_full().gap(tk.r(1.0)).pt(tk.r(1.0));
        let buttons = vec![
            purpose(
                tk,
                "sof-purpose-planned",
                planned,
                t("sourceFunds:purpose.planned.title"),
                t("sourceFunds:purpose.planned.body"),
                "planned_exchange_sale",
            ),
            purpose(
                tk,
                "sof-purpose-existing",
                !planned,
                t("sourceFunds:purpose.existing.title"),
                t("sourceFunds:purpose.existing.body"),
                "existing_transaction",
            ),
        ];
        inner = inner.child(if ww >= 768.0 {
            row().w_full().gap(tk.r(0.75)).children(buttons)
        } else {
            col().w_full().gap(tk.r(0.75)).children(buttons)
        });
        if planned {
            inner = inner.child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .items_start()
                    .px(tk.r(0.75))
                    .py(tk.r(0.625))
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border)
                    .border_layout()
                    .fill(tk.muted.alpha(0.3))
                    .child(
                        ic(&I::INFO, tk.r(1.0), tk.muted_fg.alpha(0.7))
                            .mt(2.0)
                            .a11y_hidden(),
                    )
                    .child(
                        text(t("sourceFunds:purpose.plannedHint"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .wrap()
                            .shrink(),
                    ),
            );
            let dest_l = t("sourceFunds:setup.exchangeBroker.label");
            let note_l = t("sourceFunds:setup.bankNote.label");
            let f1 = kit::field(
                tk,
                &dest_l,
                kit::input(
                    tk,
                    "sof-planned-dest",
                    &c.planned_destination,
                    &dest_l,
                    &t("sourceFunds:workstation.krakenBitpandaOtcDesk"),
                    edit_destination,
                    tk.r(2.25),
                ),
            );
            let f2 = kit::field(
                tk,
                &note_l,
                kit::input(
                    tk,
                    "sof-planned-note",
                    &c.planned_note,
                    &note_l,
                    &t("sourceFunds:setup.bankNote.placeholder"),
                    edit_note,
                    tk.r(2.25),
                ),
            );
            inner = inner.child(if ww >= 768.0 {
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .child(f1.w(220.0))
                    .child(f2.grow(1.0).w(0.0))
            } else {
                col().w_full().gap(tk.r(0.75)).child(f1).child(f2)
            });
        }
        let rl = t("sourceFunds:case.targetReference");
        inner = inner.child(kit::field(
            tk,
            &rl,
            kit::input(
                tk,
                "sof-target-ref",
                &c.target,
                &rl,
                &t("sourceFunds:case.targetReferenceHint"),
                edit_target,
                tk.r(2.25),
            ),
        ));
        details = details.child(inner);
    }
    out = out.child(details);
    if !c.selected_target().is_empty() && sof.resolved.error() {
        out = out.child(
            text(t("sourceFunds:case.targetUnavailable"))
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .role("alert"),
        );
    }
    out = out.child(picker(app, tk, layer, ww, &target_label));
    if c.next_cursor.is_some() {
        let label = t(if c.pages_loading {
            "sourceFunds:case.loading"
        } else {
            "sourceFunds:case.loadMore"
        });
        let b = kit::button(
            tk,
            "sof-load-more",
            Variant::Outline,
            Size::Default,
            None,
            &label,
            !c.pages_loading,
        );
        out = out.child(row().child(if c.pages_loading {
            b
        } else {
            b.on_press(super::load_more)
        }));
    }
    // Historical inbound coverage.
    let cov = sof.coverage.data();
    let summary = match cov {
        Some(d) => tr(
            "sourceFunds:coverage.summary",
            &[
                (
                    "amount",
                    &format!(
                        "{:.8}",
                        model::f(&d["totals"]["buckets"]["fully_traced"], "amount").unwrap_or(0.0)
                    ),
                ),
                (
                    "count",
                    &d["totals"]["tx_count"].as_u64().unwrap_or(0).to_string(),
                ),
            ],
        ),
        None => t("sourceFunds:coverage.summaryEmpty"),
    };
    let mut body = col().w_full().gap(tk.r(1.0));
    if let Some(h) = coverage_hero(app, tk, cov) {
        body = body.child(h);
    }
    body = body.child(coverage_panel(app, tk, cov, sof.coverage.loading()));
    out = out.child(optional_section(
        tk,
        "sof-coverage",
        c.show_coverage,
        &I::GIT_BRANCH,
        &t("sourceFunds:coverage.sectionTitle"),
        Some(summary),
        |a| case(a).show_coverage ^= true,
        body,
    ));
    out
}

/// `OptionalSection`: a `kb-surface-inset` collapsible whose trigger names it and summarizes.
#[allow(clippy::too_many_arguments)]
pub fn optional_section(
    tk: &Tk,
    key: &str,
    open: bool,
    glyph: &'static Icon,
    title: &str,
    summary: Option<String>,
    toggle: fn(&mut App),
    body: El,
) -> El {
    let mut texts = col()
        .min_w(0.0)
        .shrink()
        .child(text(title.to_owned()).font(tk.tsm(600.0)).color(tk.fg));
    if let Some(s) = &summary {
        texts = texts.child(
            text(s.clone())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate(),
        );
    }
    let trigger = row()
        .key(key.to_owned())
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .role("button")
        .label(match &summary {
            Some(s) => format!("{title} {s}"),
            None => title.to_owned(),
        })
        .expanded(open)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(toggle)
        .child(
            row()
                .min_w(0.0)
                .shrink()
                .gap(tk.r(0.5))
                .center_y()
                .child(ic(glyph, tk.r(1.0), tk.fg).a11y_hidden())
                .child(texts),
        )
        .child(
            ic(
                if open {
                    &I::CHEVRON_UP
                } else {
                    &I::CHEVRON_DOWN
                },
                tk.r(1.0),
                tk.muted_fg,
            )
            .a11y_hidden(),
        );
    let mut s = kit::inset(tk).key(format!("{key}-section")).child(trigger);
    if open {
        s = s.child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(body),
        );
    }
    s
}

/// The transaction picker (`kb-surface-inset overflow-hidden`).
fn picker(app: &App, tk: &Tk, layer: &mut Vec<El>, ww: f32, target_label: &str) -> El {
    let sof = &app.provenance.sof;
    let c = sof.case.as_ref().expect("case");
    let filters_open = c.show_filters;
    let wide = ww >= 1024.0;
    let left = col()
        .min_w(0.0)
        .child(
            text(target_label.to_owned())
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(
            text(tr(
                "sourceFunds:case.loadedTransactions",
                &[("count", &c.pages.len().to_string())],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    let search_l = t("sourceFunds:workstation.searchTxidWalletNote");
    let search = kit::search_input(
        tk,
        "sof-search",
        &c.search,
        &search_l,
        &search_l,
        edit_search,
    );
    let mut bar = row()
        .w_full()
        .gap(tk.r(0.5))
        .child(if filters_open || !wide {
            col().grow(1.0).min_w(220.0).child(search)
        } else {
            col().w(220.0).child(search)
        })
        .child(
            kit::button(
                tk,
                "sof-filters",
                Variant::Outline,
                Size::Sm,
                Some(&I::SLIDERS_HORIZONTAL),
                &t("sourceFunds:filters.title"),
                true,
            )
            .expanded(filters_open)
            .on_press(|a: &mut App| case(a).show_filters ^= true),
        );
    if c.filters.active() || !c.search.text.is_empty() {
        bar = bar.child(
            kit::button(
                tk,
                "sof-filters-clear",
                Variant::Outline,
                Size::Sm,
                Some(&I::X),
                &t("sourceFunds:filters.clear"),
                true,
            )
            .on_press(|a: &mut App| {
                let c = case(a);
                c.search = TextEdit::new("");
                c.filters = model::Filters::default();
            }),
        );
    }
    let mut right = col().gap(tk.r(0.75)).child(bar);
    if filters_open {
        right = right.child(filter_grid(app, tk, layer, ww));
    }
    let head = if wide {
        let right = if filters_open {
            right.grow(1.0).w(0.0).min_w(0.0)
        } else {
            right
        };
        row()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.75))
            .center_y()
            .justify_between()
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout()
            .child(left.shrink())
            .child(right)
    } else {
        col()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.75))
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout()
            .child(left)
            .child(right.w_full())
    };
    let mut list = kit::inset(tk)
        .key("sof-picker")
        .clip_rounded(tk.r(0.5))
        .child(head);
    if c.pages_error {
        list = list.child(
            text(t("sourceFunds:case.transactionsError"))
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .padding(tk.r(0.75))
                .role("alert"),
        );
    }
    let md = ww >= 768.0;
    if md {
        // The table kit's header band and labels.
        use crate::ui::table as tb;
        list = list.child(
            tb::head(tk)
                .h(tk.r(tb::HEAD_H))
                .px(tk.r(0.75))
                .gap(tk.r(0.75))
                .center_y()
                .child(row().grow(1.0).w(0.0).min_w(0.0).child(tb::th_text(
                    tk,
                    t("sourceFunds:transactionRow.header.transaction"),
                )))
                .child(
                    row()
                        .w(150.0)
                        .justify_end()
                        .child(tb::th_text(tk, t("sourceFunds:sourceOrGap.amount"))),
                )
                .child(
                    row()
                        .w(160.0)
                        .child(tb::th_text(tk, t("sourceFunds:fallback.wallet"))),
                )
                .child(
                    row()
                        .w(130.0)
                        .justify_end()
                        .child(tb::th_text(tk, t("sourceFunds:transactionRow.header.flow"))),
                )
                .child(el().w(44.0 - tk.r(0.75)).a11y_hidden()),
        );
    }
    let body = if c.pages.is_empty() {
        col()
            .w_full()
            .padding(tk.r(0.75))
            .child(kit::empty_state(tk, t("sourceFunds:setup.noMatches")))
    } else {
        let selected = c.selected_target();
        let hidden = app.prefs.hide_sensitive();
        let mut rows = col().w_full();
        let n = c.pages.len();
        for (i, r) in c.pages.iter().enumerate() {
            rows = rows.child(target_row(
                tk,
                r,
                model::tx_ref(r) == selected,
                md,
                hidden,
                i + 1 == n,
            ));
        }
        rows
    };
    list.child(
        col()
            .w_full()
            .max_h(430.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .child(body),
    )
}

fn filter_grid(app: &App, tk: &Tk, layer: &mut Vec<El>, ww: f32) -> El {
    let c = app.provenance.sof.case.as_ref().expect("case");
    let fl = &c.filters;
    let o = |v: &str, l: String| (v.to_owned(), l, false);
    let direction = vec![
        o("all", t("sourceFunds:filters.direction.all")),
        o("incoming", t("sourceFunds:flow.incoming")),
        o("outgoing", t("sourceFunds:flow.outgoing")),
        o("transfer", t("sourceFunds:flow.transfer")),
        o("swap", t("sourceFunds:linkType.swap")),
    ];
    let date = vec![
        o("all", t("sourceFunds:filters.date.all")),
        o("today", t("sourceFunds:filters.date.today")),
        o("yesterday", t("sourceFunds:filters.date.yesterday")),
        o("7days", t("sourceFunds:filters.date.last7days")),
        o("30days", t("sourceFunds:filters.date.last30days")),
        o("older", t("sourceFunds:filters.date.older")),
    ];
    let status = vec![
        o("all", t("sourceFunds:filters.status.all")),
        o("confirmed", t("sourceFunds:filters.status.confirmed")),
        o("pending", t("sourceFunds:filters.status.pending")),
        o("review", t("sourceFunds:filters.status.review")),
    ];
    let mut network = vec![o("all", t("sourceFunds:filters.network.all"))];
    network.extend(
        ["on-chain", "lightning", "liquid", "exchange"]
            .iter()
            .map(|n| o(n, (*n).to_owned())),
    );
    let mut asset = vec![o("all", t("sourceFunds:filters.asset.all"))];
    asset.extend(["BTC", "LBTC"].iter().map(|n| o(n, (*n).to_owned())));
    let mut wallet = vec![o("all", t("sourceFunds:filters.wallet.all"))];
    let names: Vec<String> = match app.provenance.sof.wallets.data() {
        Some(d) => model::unique_sorted(
            model::arr(d, "wallets")
                .iter()
                .map(|w| model::s(w, "label").to_owned()),
        ),
        None => model::unique_sorted(c.pages.iter().map(model::tx_wallet)),
    };
    wallet.extend(names.into_iter().map(|n| (n.clone(), n, false)));
    let h = tk.r(2.0);
    let items = vec![
        kit::select(
            app,
            tk,
            layer,
            "f-flow",
            &t("sourceFunds:filters.direction.ariaLabel"),
            direction,
            &fl.flow,
            "",
            h,
            |a, v| case(a).filters.flow = v.to_owned(),
        ),
        kit::select(
            app,
            tk,
            layer,
            "f-date",
            &t("sourceFunds:filters.date.ariaLabel"),
            date,
            &fl.date,
            "",
            h,
            |a, v| case(a).filters.date = v.to_owned(),
        ),
        kit::select(
            app,
            tk,
            layer,
            "f-status",
            &t("sourceFunds:filters.status.ariaLabel"),
            status,
            &fl.status,
            "",
            h,
            |a, v| case(a).filters.status = v.to_owned(),
        ),
        kit::select(
            app,
            tk,
            layer,
            "f-network",
            &t("sourceFunds:filters.network.ariaLabel"),
            network,
            &fl.network,
            "",
            h,
            |a, v| case(a).filters.network = v.to_owned(),
        ),
        kit::select(
            app,
            tk,
            layer,
            "f-asset",
            &t("sourceFunds:filters.asset.ariaLabel"),
            asset,
            &fl.asset,
            "",
            h,
            |a, v| case(a).filters.asset = v.to_owned(),
        ),
    ];
    let wallet_sel = kit::select(
        app,
        tk,
        layer,
        "f-wallet",
        &t("sourceFunds:filters.wallet.ariaLabel"),
        wallet,
        &fl.wallet,
        "",
        h,
        |a, v| case(a).filters.wallet = v.to_owned(),
    );
    let gap = tk.r(0.5);
    let grid = if ww >= 1280.0 {
        // `xl:grid-cols-5`, the wallet select spanning two columns.
        col()
            .w_full()
            .gap(gap)
            .child(kit::grid(tk, 5, gap, items))
            .child(
                row()
                    .w_full()
                    .gap(gap)
                    .child(col().grow(2.0).w(0.0).child(wallet_sel))
                    .child(el().grow(3.0).w(0.0)),
            )
    } else {
        let mut all = items;
        all.push(wallet_sel);
        kit::grid(tk, if ww >= 640.0 { 2 } else { 1 }, gap, all)
    };
    col()
        .w_full()
        .pt(tk.r(0.75))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(grid)
}

/// The flow tones of a picker row: (pill fill, pill border, ink).
fn flow_tone(tk: &Tk, flow: &str) -> (Color, Color, Color) {
    match flow {
        "incoming" => {
            let e = emerald();
            (
                if tk.dark { e.c900.alpha(0.25) } else { e.c50 },
                e.c600.alpha(0.2),
                kit::ink(tk, &e),
            )
        }
        "outgoing" => {
            let r = red();
            (
                if tk.dark { r.c900.alpha(0.25) } else { r.c50 },
                r.c600.alpha(0.2),
                kit::ink(tk, &r),
            )
        }
        _ => {
            let z = zinc();
            (
                if tk.dark { z.c800.alpha(0.7) } else { z.c50 },
                z.c500.alpha(0.2),
                if tk.dark { z.c300 } else { z.c700 },
            )
        }
    }
}

/// `TransactionTargetRow`: the select button and the eye that opens the detail panel.
fn target_row(tk: &Tk, r: &Value, active: bool, md: bool, hidden: bool, last: bool) -> El {
    let flow = model::tx_flow(r);
    let glyph: &'static Icon = match flow {
        "incoming" => &I::ARROW_DOWN_RIGHT,
        "outgoing" => &I::ARROW_UP_RIGHT,
        _ => &I::ARROW_LEFT_RIGHT,
    };
    let (fill, border, ink) = flow_tone(tk, flow);
    let amount_ink = match flow {
        "incoming" => kit::ink(tk, &emerald()),
        "outgoing" => kit::ink(tk, &red()),
        _ => tk.muted_fg,
    };
    let txid = model::first(r, &["external_id", "externalId", "id"]).to_owned();
    let desc = {
        let d = model::first(r, &["counter", "description", "note"]);
        if !d.is_empty() {
            d.to_owned()
        } else if !txid.is_empty() {
            txid.clone()
        } else {
            t("sourceFunds:transactionRow.fallbackDescription")
        }
    };
    let desc_name = desc.clone();
    let id = model::tx_ref(r);
    let flow_label = t(&format!("sourceFunds:flow.{flow}"));
    // The table kit's leading slot: the direction glyph on its tinted 1.5 rem tile.
    use crate::ui::table as tb;
    let icon_box = row()
        .size(tk.r(tb::LEAD), tk.r(tb::LEAD))
        .center()
        .radius(tk.r(0.375))
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .a11y_hidden()
        .child(ic(glyph, tk.r(0.875), ink));
    let meta = row()
        .gap(tk.r(0.5))
        .flex_wrap()
        .child(
            text(model::s(r, "asset").to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        )
        .child(
            text(model::short_id(&txid))
                .font(tk.txs(400.0).mono())
                .color(tk.muted_fg),
        );
    let main = row()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .gap(tk.r(tb::LEAD_GAP))
        .items_start()
        .child(icon_box)
        .child(
            col()
                .min_w(0.0)
                .shrink()
                .gap(tk.r(0.25))
                .child(tb::cell_text(desc, tk.tsm(500.0), tk.fg))
                .child(meta),
        );
    let amount = tb::num(tk, model::tx_signed_amount(r), amount_ink, hidden);
    // The direction is the leading tile; the flow column says it in words, without a second badge.
    let flow_col = col()
        .w(130.0)
        .gap(tk.r(0.25))
        .items_end()
        .child(text(flow_label).font(tk.tsm(400.0)).color(tk.fg))
        .child(tb::num_sub(tk, model::tx_date(r), false));
    let row_name = format!(
        "{} {} {} {} {} {} {}",
        desc_name,
        model::s(r, "asset"),
        model::short_id(&txid),
        model::tx_signed_amount(r),
        model::tx_wallet(r),
        t(&format!("sourceFunds:flow.{flow}")),
        model::tx_date(r)
    );
    let sel_id = id.clone();
    let mut button = row()
        .key(format!("sof-row-{id}"))
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .gap(tk.r(0.75))
        .center_y()
        .role("button")
        .label(row_name)
        .checked(active)
        .focusable(true)
        .on_press(move |a: &mut App| case(a).target = TextEdit::new(&sel_id));
    button = if md {
        button
            .child(main)
            .child(row().w(150.0).justify_end().child(amount))
            .child(row().w(160.0).min_w(0.0).child(tb::cell_text(
                model::tx_wallet(r),
                tk.tsm(400.0),
                tk.muted_fg,
            )))
            .child(flow_col)
    } else {
        button.child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .child(main)
                .child(amount)
                .child(
                    text(format!(
                        "{}{}",
                        t("sourceFunds:transactionRow.walletMobile"),
                        model::tx_wallet(r)
                    ))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
                ),
        )
    };
    let open_id = id.clone();
    let eye_l = t("sourceFunds:workstation.viewTransactionDetails");
    let eye = row()
        .key(format!("sof-row-eye-{id}"))
        .w(44.0)
        .h_full()
        .center()
        .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
        .border_layout()
        .hover(tk.muted.alpha(0.45))
        .role("button")
        .label(eye_l)
        .help(t("sourceFunds:workstation.viewDetails"))
        .focusable(true)
        .on_press(move |a: &mut App| super::open_tx(a, &open_id))
        .child(ic(&I::EYE, tk.r(1.0), tk.muted_fg));
    let mut r = row()
        .w_full()
        .items_stretch()
        .hover(tk.muted.alpha(0.35))
        .child(button)
        .child(eye);
    if active {
        // The table kit's selected row.
        r = r.fill(tk.muted.alpha(0.6)).hover(tk.muted.alpha(0.6));
    }
    if !last {
        r = r
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout();
    }
    r
}

// ---- Coverage ---------------------------------------------------------------------------------

fn bucket_bar(name: &str) -> Color {
    match name {
        "fully_traced" => emerald().c500,
        "attested" => kit::sky().c500,
        "unattributed" => kit::orange().c500,
        "in_review" => kit::amber().c500,
        "untraced" => kit::rose().c500,
        _ => Color::TRANSPARENT,
    }
}

fn bucket_tone(tk: &Tk, name: &str) -> Color {
    match name {
        "fully_traced" => kit::ink(tk, &emerald()),
        "attested" => kit::ink(tk, &kit::sky()),
        "unattributed" => kit::ink(tk, &kit::orange()),
        "in_review" => kit::ink(tk, &kit::amber()),
        "untraced" => kit::ink(tk, &kit::rose()),
        _ => tk.muted_fg,
    }
}

/// `TracedCoverageHero`.
fn coverage_hero(app: &App, tk: &Tk, cov: Option<&Value>) -> Option<El> {
    let cov = cov?;
    let totals = &cov["totals"];
    let count = totals["tx_count"].as_u64().unwrap_or(0);
    if count == 0 {
        return None;
    }
    let total = model::f(totals, "amount").unwrap_or(0.0);
    let pct = |n: &str| {
        if total > 0.0 {
            model::f(&totals["buckets"][n], "amount").unwrap_or(0.0) / total * 100.0
        } else {
            0.0
        }
    };
    let mut legend = row().gap(tk.r(1.0)).flex_wrap();
    for name in model::COVERAGE_BUCKETS {
        if *name != "fully_traced"
            && model::f(&totals["buckets"][*name], "amount").unwrap_or(0.0) <= 0.0
        {
            continue;
        }
        let sw = if *name == "not_classified" {
            tk.muted_fg.alpha(0.4)
        } else {
            bucket_bar(name)
        };
        legend = legend.child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(
                    el().size(tk.r(0.625), tk.r(0.625))
                        .radius(2.0)
                        .fill(sw)
                        .a11y_hidden(),
                )
                .child(
                    text(t(&format!("sourceFunds:coverageBucket.{name}")))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                )
                .child(
                    text(format!("{:.1}%", pct(name)))
                        .font(tk.txs(500.0))
                        .color(bucket_tone(tk, name)),
                ),
        );
    }
    let mut bar = row()
        .w_full()
        .h(tk.r(0.625))
        .radius(tk.r(0.3125))
        .clip_rounded(tk.r(0.3125))
        .fill(tk.muted);
    for name in model::COVERAGE_BUCKETS {
        let p = pct(name);
        if p > 0.0 {
            let fill = if *name == "not_classified" {
                tk.muted_fg.alpha(0.4)
            } else {
                bucket_bar(name)
            };
            bar = bar.child(
                el().w_frac((p / 100.0) as f32)
                    .h_full()
                    .fill(fill)
                    .help(format!(
                        "{}: {p:.1}%",
                        t(&format!("sourceFunds:coverageBucket.{name}"))
                    )),
            );
        }
    }
    let mut body = col()
        .w_full()
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .center_y()
                .justify_between()
                .flex_wrap()
                .child(
                    col()
                        .child(
                            text(t("sourceFunds:workstation.inboundHistoryTraced").to_uppercase())
                                .font(tk.txs(400.0).tracking(0.3))
                                .color(tk.muted_fg),
                        )
                        .child(
                            row()
                                .mt(2.0)
                                .gap(tk.r(0.5))
                                .items_end()
                                .child(
                                    text(format!("{:.1}%", pct("fully_traced")))
                                        .font(kit::tnum(tk.px(tk.r(1.875), 600.0).mono()))
                                        .color(kit::ink(tk, &emerald())),
                                )
                                .child(
                                    text(tr(
                                        "sourceFunds:case.fullyTraced",
                                        &[("count", &count.to_string())],
                                    ))
                                    .font(tk.tsm(400.0))
                                    .color(tk.muted_fg),
                                ),
                        ),
                )
                .child(legend),
        )
        .child(bar.mt(tk.r(0.75)))
        .child(
            text(tr(
                "sourceFunds:case.attested",
                &[("percent", &format!("{:.1}", pct("attested")))],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap()
            .mt(tk.r(0.5)),
        );
    if cov["truncation"]["truncated"].as_bool() == Some(true) {
        body = body.child(
            text(tr(
                "sourceFunds:case.partialCoverage",
                &[
                    ("shown", &count.to_string()),
                    (
                        "total",
                        &cov["truncation"]["inbound_total_count"]
                            .as_u64()
                            .unwrap_or(0)
                            .to_string(),
                    ),
                ],
            ))
            .font(tk.txs(400.0))
            .color(kit::ink(tk, &kit::amber()))
            .mt(tk.r(0.25)),
        );
    }
    let _ = app;
    Some(
        crate::ui::surface(tk)
            .w_full()
            .padding(tk.r(1.0))
            .child(body),
    )
}

/// `CoveragePanel`: six buckets, amount and share each.
fn coverage_panel(app: &App, tk: &Tk, cov: Option<&Value>, loading: bool) -> El {
    let Some(cov) = cov else {
        return kit::empty_state(
            tk,
            t(if loading {
                "sourceFunds:workstation.computingCoverage"
            } else {
                "sourceFunds:coverage.noInbound"
            }),
        );
    };
    let totals = &cov["totals"];
    let count = totals["tx_count"].as_u64().unwrap_or(0);
    if count == 0 {
        return kit::empty_state(tk, t("sourceFunds:coverage.noInbound"));
    }
    let total = model::f(totals, "amount").unwrap_or(0.0);
    let hidden = app.prefs.hide_sensitive();
    let mut out = col().w_full().gap(tk.r(0.75));
    if cov["truncation"]["truncated"].as_bool() == Some(true) {
        let raw = tr(
            "sourceFunds:coverage.truncated",
            &[
                ("shown", &count.to_string()),
                (
                    "total",
                    &cov["truncation"]["inbound_total_count"]
                        .as_u64()
                        .unwrap_or(0)
                        .to_string(),
                ),
                (
                    "notClassified",
                    &cov["truncation"]["not_classified_count"]
                        .as_u64()
                        .unwrap_or(0)
                        .to_string(),
                ),
            ],
        );
        let plain = raw.replace("<0>", "").replace("</0>", "");
        let a = kit::amber();
        out = out.child(
            kit::notice_box(tk, kit::notice(tk, &a, a.c900, a.c100)).child(
                text(plain)
                    .font(tk.txs(400.0))
                    .color(if tk.dark { a.c100 } else { a.c900 })
                    .wrap(),
            ),
        );
    }
    let mut cells = vec![];
    for name in model::COVERAGE_BUCKETS {
        let b = &totals["buckets"][*name];
        let amount = model::f(b, "amount").unwrap_or(0.0);
        let n = b["tx_count"].as_u64().unwrap_or(0);
        let pct = if total > 0.0 {
            amount / total * 100.0
        } else {
            0.0
        };
        cells.push(
            col()
                .gap(tk.r(0.25))
                .child(
                    text(t(&format!("sourceFunds:coverageBucket.{name}")).to_uppercase())
                        .font(tk.txs(400.0).tracking(0.3))
                        .color(tk.fg.alpha(0.7)),
                )
                .child(crate::ui::sensitive(
                    format!("{amount:.8}"),
                    tk.tlg(600.0),
                    bucket_tone(tk, name),
                    hidden,
                ))
                .child(
                    text(format!("{n} tx · {pct:.1}%"))
                        .font(tk.txs(400.0))
                        .color(tk.fg.alpha(0.8)),
                ),
        );
    }
    let ww = app.shell.window_w;
    out.child(kit::grid(
        tk,
        if ww >= 1024.0 || ww == 0.0 { 6 } else { 3 },
        tk.r(1.0),
        cells,
    ))
}
