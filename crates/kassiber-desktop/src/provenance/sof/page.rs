//! The journey frame (routes/source-funds/index.tsx): the scope gate, the step rail, the case
//! strip, the stage's alerts, the stage surface and the Back / forward footer.

use rune::{Cx, El, col, el, row, text};

use super::kit::{self, amber};
use super::{Stage, model};
use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant};
use crate::ui::ic;

/// `screenShellClassName mx-auto max-w-6xl`.
fn shell(tk: &Tk) -> El {
    col()
        .w_full()
        .max_w(tk.r(72.0))
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background)
}

fn centered(tk: &Tk, inner: El) -> El {
    row()
        .w_full()
        .justify_center()
        .fill(tk.background)
        .child(inner)
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let sof = &app.provenance.sof;
    // The scope gate: no case before the canonical book scope is known.
    let ready = matches!(&sof.scope, Some(Ok(_))) && sof.case.is_some();
    if !ready {
        let msg = t(if matches!(&sof.scope, Some(Err(_))) {
            "sourceFunds:case.scopeError"
        } else {
            "sourceFunds:case.loading"
        });
        return centered(
            tk,
            col()
                .w_full()
                .padding(tk.r(1.25))
                .role("status")
                .child(text(msg).font(tk.tsm(400.0)).color(tk.fg)),
        );
    }
    let c = sof.case.as_ref().expect("case");
    let stage = super::stage(app);
    let position = stage.index();
    let selected_tx_id = super::selected_tx_id(app);
    let can_export = super::can_export(app);

    // The step rail: `ol grid grid-cols-2 gap-2 sm:grid-cols-4`.
    let mut steps = row()
        .w_full()
        .gap(tk.r(0.5))
        .role("list")
        .label(t("sourceFunds:header.title"));
    for (i, s) in Stage::ALL.iter().enumerate() {
        let active = *s == stage;
        let enabled = !(i > 0 && selected_tx_id.is_empty() || *s == Stage::Export && !can_export);
        let label = t(&format!("sourceFunds:journey.{}", s.id()));
        let num = row()
            .size(tk.r(1.5), tk.r(1.5))
            .center()
            .radius(tk.r(0.75))
            .border(1.0, tk.border)
            .border_layout();
        let num = if i < position {
            num.child(ic(&I::CHECK, tk.r(0.75), tk.fg))
        } else {
            num.child(
                text((i + 1).to_string())
                    .font(tk.txs(400.0))
                    .color(if active { tk.fg } else { tk.muted_fg }),
            )
        };
        let st = *s;
        let mut b = row()
            .key(format!("sof-step-{}", s.id()))
            .grow(1.0)
            .w(0.0)
            .min_w(0.0)
            .px(tk.r(0.75))
            .py(tk.r(0.75))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.5))
            .border(
                1.0,
                if active {
                    tk.primary.alpha(0.4)
                } else {
                    rune::Color::TRANSPARENT
                },
            )
            .border_layout()
            .role("button")
            .label(if i < position {
                label.clone()
            } else {
                format!("{} {label}", i + 1)
            })
            .checked(active)
            .focusable(enabled)
            .disabled(!enabled)
            .child(num.a11y_hidden())
            .child(
                text(label)
                    .font(tk.tsm(if active { 600.0 } else { 400.0 }))
                    .color(if active { tk.fg } else { tk.muted_fg })
                    .truncate(),
            );
        if active {
            b = b.fill(tk.primary.alpha(0.05));
        } else if enabled {
            b = b.hover(tk.muted);
        }
        if enabled {
            b = b.on_press(move |a: &mut App| super::go_stage(a, st));
        } else {
            b = b.opacity(0.4);
        }
        steps = steps.child(b);
    }

    let mut page = shell(tk)
        .key("sof-page")
        .child(
            text(t("sourceFunds:journey.description"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .max_w(tk.r(48.0)),
        )
        .child(steps);

    // The case strip (every stage after the first).
    if stage != Stage::Target {
        let tx = super::selected_tx(app);
        let report = super::report(app);
        let pick = |k: &str| {
            tx.map(|t| model::s(t, k).to_owned())
                .filter(|s| !s.is_empty())
        };
        let title = pick("note")
            .or_else(|| pick("description"))
            .or_else(|| pick("wallet"))
            .or_else(|| {
                report
                    .map(|r| model::s(&r["target"], "wallet").to_owned())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| t("sourceFunds:case.targetSection"));
        let amount = report
            .and_then(|r| model::f(&r["target"], "required_amount"))
            .or_else(|| tx.and_then(|t| model::f(t, "amount")));
        let asset = tx
            .map(|t| model::s(t, "asset"))
            .filter(|s| !s.is_empty())
            .unwrap_or("BTC");
        let status = status_key(app);
        let hidden = app.prefs.hide_sensitive();
        page = page.child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .center_y()
                .justify_between()
                .radius(tk.r(0.5))
                .fill(tk.muted.alpha(0.3))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(title).font(tk.tsm(500.0)).color(tk.fg).truncate())
                        .child(
                            row()
                                .child(crate::ui::sensitive(
                                    model::format_btc(amount, asset),
                                    tk.txs(400.0),
                                    tk.muted_fg,
                                    hidden,
                                ))
                                .child(
                                    text(format!(
                                        " · {}",
                                        t(&format!("sourceFunds:case.{status}"))
                                    ))
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg)
                                    .wrap()
                                    .shrink(),
                                ),
                        ),
                )
                .child(
                    kit::button(
                        tk,
                        "sof-change",
                        Variant::Ghost,
                        Size::Sm,
                        None,
                        &t("sourceFunds:journey.changeTarget"),
                        true,
                    )
                    .on_press(|a: &mut App| super::go_stage(a, Stage::Target)),
                ),
        );
    }
    if super::context(app).is_some_and(|c| c["scope_truncated"].as_bool() == Some(true)) {
        page = page.child(
            text(t("sourceFunds:case.scopeTruncated"))
                .font(tk.tsm(400.0))
                .color(kit::ink(tk, &amber()))
                .wrap()
                .role("status"),
        );
    }
    if !c.selected_target().is_empty() && sof.preview.error() {
        page = page.child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .gap(tk.r(0.5))
                .radius(tk.r(0.5))
                .border(1.0, tk.destructive.alpha(0.3))
                .border_layout()
                .role("alert")
                .child(
                    text(t("sourceFunds:journey.targetError"))
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap(),
                )
                .child(
                    kit::button(
                        tk,
                        "sof-retry",
                        Variant::Outline,
                        Size::Sm,
                        None,
                        &t("sourceFunds:journey.retry"),
                        true,
                    )
                    .on_press(super::retry_preview),
                ),
        );
    }
    if stage == Stage::Disclose {
        if sof.diagram.error() {
            page = page.child(
                col()
                    .w_full()
                    .padding(tk.r(1.0))
                    .gap(tk.r(0.5))
                    .radius(tk.r(0.5))
                    .border(1.0, tk.border)
                    .border_layout()
                    .role("alert")
                    .child(
                        text(t("sourceFunds:journey.previewError"))
                            .font(tk.tsm(400.0))
                            .color(tk.fg)
                            .wrap(),
                    )
                    .child(
                        kit::button(
                            tk,
                            "sof-retry-diagram",
                            Variant::Outline,
                            Size::Sm,
                            None,
                            &t("sourceFunds:journey.retry"),
                            true,
                        )
                        .on_press(super::retry_diagram),
                    ),
            );
        } else if sof.diagram.fetching() {
            page = page.child(
                text(t("sourceFunds:case.loading"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .role("status"),
            );
        }
        if super::report(app).is_some() && !super::exportable(app) {
            let a = amber();
            page = page.child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .padding(tk.r(1.0))
                    .center_y()
                    .justify_between()
                    .flex_wrap()
                    .radius(tk.r(0.5))
                    .border(1.0, a.c500.alpha(0.3))
                    .border_layout()
                    .fill(a.c500.alpha(0.05))
                    .role("status")
                    .child(
                        text(t("sourceFunds:case.exportBlocked"))
                            .font(tk.tsm(400.0))
                            .color(tk.fg)
                            .wrap()
                            .shrink(),
                    )
                    .child(
                        kit::button(
                            tk,
                            "sof-resolve",
                            Variant::Outline,
                            Size::Sm,
                            None,
                            &t("sourceFunds:journey.resolveQuestions"),
                            true,
                        )
                        .on_press(|a: &mut App| super::go_stage(a, Stage::Trace)),
                    ),
            );
        }
    }
    if super::export_error(app) {
        page = page.child(
            text(t("sourceFunds:case.exportError"))
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .role("alert"),
        );
    }

    let body = match stage {
        Stage::Target => super::target::view(app, cx, tk, layer),
        Stage::Trace => super::trace::view(app, cx, tk, layer),
        Stage::Disclose => super::disclose::disclose(app, cx, tk, layer),
        Stage::Export => super::disclose::export(app, tk),
    };
    page = page.child(
        crate::ui::surface(tk)
            .key(format!("sof-stage-{}", stage.id()))
            .w_full()
            .min_w(0.0)
            .padding(tk.card_pad())
            .role("region")
            .label(t(&format!("sourceFunds:journey.{}", stage.id())))
            .child(body),
    );

    // The footer: Back, and the stage's forward action.
    let mut footer = row()
        .w_full()
        .pt(tk.r(1.0))
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout();
    let back = kit::button(
        tk,
        "sof-back",
        Variant::Ghost,
        Size::Default,
        Some(&I::ARROW_LEFT),
        &t("sourceFunds:journey.back"),
        position > 0,
    );
    footer = footer.child(if position > 0 {
        back.on_press(super::back)
    } else {
        back
    });
    if stage != Stage::Export {
        let cur = super::current(app);
        let enabled = match stage {
            Stage::Target => !selected_tx_id.is_empty() && !sof.resolved.fetching(),
            Stage::Trace => cur,
            _ => cur && super::preview_ready(app),
        };
        let label = t(match stage {
            Stage::Disclose => "sourceFunds:journey.approvePreview",
            Stage::Trace => "sourceFunds:journey.openPreview",
            _ => "sourceFunds:journey.traceTarget",
        });
        footer = footer.child(forward_button(tk, &label, enabled));
    } else {
        footer = footer.child(el());
    }
    page = page.child(footer);
    centered(tk, page)
}

/// The primary button with its trailing arrow (`<Button>{label}<ArrowRight/></Button>`).
fn forward_button(tk: &Tk, label: &str, enabled: bool) -> El {
    let b = crate::txdetail::widgets::button_surface(
        tk,
        row()
            .key("sof-forward")
            .h(tk.r(2.25))
            .px(tk.r(0.75))
            .gap(tk.r(0.5))
            .center(),
        Variant::Default,
    )
    .role("button")
    .label(label.to_owned())
    .focusable(enabled)
    .disabled(!enabled)
    .child(
        text(label.to_owned())
            .font(tk.tsm(500.0))
            .color(tk.primary_fg),
    )
    .child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.primary_fg));
    if enabled {
        b.on_press(super::forward)
    } else {
        b.opacity(0.5)
    }
}

/// The case strip's status (`selectTarget`, `targetUnavailable`, …).
fn status_key(app: &App) -> &'static str {
    let sof = &app.provenance.sof;
    let Some(c) = &sof.case else {
        return "selectTarget";
    };
    if c.selected_target().is_empty() {
        "selectTarget"
    } else if sof.resolved.error()
        || (sof.resolved.success() && super::selected_tx_id(app).is_empty())
    {
        "targetUnavailable"
    } else if sof.preview.error() {
        "reviewUnavailable"
    } else if sof.preview.fetching() || super::report(app).is_none() {
        "loading"
    } else if super::exportable(app) {
        "exportable"
    } else {
        "needsEvidence"
    }
}
