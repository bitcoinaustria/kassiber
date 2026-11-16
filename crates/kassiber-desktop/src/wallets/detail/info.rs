//! The Details tab: the connection details card, `ImportRunsPanel` (each file import of this
//! connection, with its two-step rollback), the Samourai card and the BTCPay provenance card.

use rune::{El, col, row, text};
use serde_json::Value;

use super::actions::{script_type_label, wallet_script_types};
use super::ui::{self, h};
use super::{count, f, s};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, button_surface};
use crate::ui::ic;
use crate::wallets::conn::Conn;

/// shadcn `Card` (default variant): `py-(--kb-card-padding) gap-4`; its header `border-b px-4`
/// with the title (`text-sm sm:text-base leading-none font-semibold`) and description.
fn default_card(tk: &Tk, title: El, desc: &str, content: Vec<El>, flush_bottom: bool) -> El {
    let header = col()
        .w_full()
        .px(tk.r(1.0))
        .pb(tk.card_pad())
        .gap(tk.r(0.375))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(title)
        .child(
            text(desc.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
    ui::card(tk)
        .pt(tk.card_pad())
        .pb(if flush_bottom { 0.0 } else { tk.card_pad() })
        .gap(tk.r(1.0))
        .child(header)
        .children(content)
}

fn sync_mode_label(v: &str) -> Option<String> {
    let k = match v {
        "backend_descriptor" => "backendDescriptor",
        "backend_addresses" => "backendAddresses",
        "backend_silent_payment" => "backendSilentPayment",
        "file_import" => "fileImport",
        "btcpay" => "btcpay",
        "not_configured" => "notConfigured",
        _ => return None,
    };
    Some(t(&format!("connections:detail.syncMode.{k}")))
}

/// `formatBackendDetail`: `name · kind (source)`.
fn backend_detail(b: Option<&Value>) -> String {
    let Some(name) = b.and_then(|b| s(b, "name")) else {
        return t("connections:detail.backendDetail.notConfigured");
    };
    let kind = b
        .and_then(|b| s(b, "kind"))
        .map(|k| format!(" · {k}"))
        .unwrap_or_default();
    let source = b
        .and_then(|b| s(b, "source"))
        .filter(|x| *x != "none")
        .map(|x| format!(" ({x})"))
        .unwrap_or_default();
    format!("{name}{kind}{source}")
}

pub fn panel(app: &App, tk: &Tk, c: &Conn, width: f32) -> Vec<El> {
    let mut out = vec![details(app, tk, c, width)];
    if let Some(p) = imports(app, tk) {
        out.push(p);
    }
    let wd = super::wallet_detail(app, c);
    if let Some(sam) = wd.and_then(|w| w.get("samourai")).filter(|x| x.is_object()) {
        out.push(samourai(app, tk, c, sam));
    }
    let routes = super::actions::provenance_routes(app, c);
    if !routes.is_empty() {
        out.push(provenance(tk, &routes));
    }
    out
}

fn details(app: &App, tk: &Tk, c: &Conn, width: f32) -> El {
    let wd = super::wallet_detail(app, c);
    let mode = super::sync_mode(app, c);
    let mut rows: Vec<El> = vec![];
    rows.push(ui::detail_row(
        tk,
        &t("connections:detail.connectionDetails.syncMode"),
        &sync_mode_label(&mode).unwrap_or_else(|| {
            if mode.is_empty() {
                "—".into()
            } else {
                mode.clone()
            }
        }),
        false,
        None,
    ));
    rows.push(ui::detail_row(
        tk,
        &t("connections:detail.connectionDetails.backend"),
        &backend_detail(wd.and_then(|w| w.get("backend"))),
        false,
        None,
    ));
    let account = wd.and_then(|w| w.get("account"));
    let acct = [
        account.and_then(|a| s(a, "code")),
        account.and_then(|a| s(a, "label")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    if !acct.is_empty() {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.connectionDetails.account"),
            &acct,
            false,
            None,
        ));
    }
    let chain_network = [
        wd.and_then(|w| s(w, "chain")),
        wd.and_then(|w| s(w, "network")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    if !chain_network.is_empty() {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.connectionDetails.network"),
            &chain_network,
            false,
            None,
        ));
    }
    let types = wallet_script_types(app, c);
    if !types.is_empty() {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.connectionDetails.scriptTypes"),
            &types
                .iter()
                .map(|v| script_type_label(v))
                .collect::<Vec<_>>()
                .join(" · "),
            false,
            None,
        ));
    }
    if wd
        .and_then(|w| w.get("descriptor"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        let status = t(
            if wd
                .and_then(|w| w.get("change_descriptor"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                "connections:detail.reveal.descriptorWithChange"
            } else {
                "connections:detail.reveal.descriptorOnly"
            },
        );
        let copy = button_surface(
            tk,
            row()
                .key("wd-reveal-inline")
                .size(tk.r(1.75), tk.r(1.75))
                .center(),
            Variant::Outline,
        )
        .role("button")
        .label(t("connections:detail.reveal.menuItem"))
        .help(t("connections:detail.reveal.menuItem"))
        .focusable(true)
        .on_press(super::actions::open_reveal)
        .child(ic(&I::COPY, tk.r(0.75), tk.fg));
        rows.push(
            row()
                .w_full()
                .min_w(0.0)
                .gap(tk.r(0.75))
                .center_y()
                .child(
                    text(t("connections:detail.connectionDetails.descriptorMaterial"))
                        .font(tk.txs(500.0))
                        .color(tk.muted_fg),
                )
                .child(
                    row().grow(1.0).min_w(0.0).justify_end().child(
                        text(status)
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .truncate(),
                    ),
                )
                .child(copy),
        );
    }
    if let Some(g) = c.gap {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.metric.gapLimit"),
            &count(g),
            false,
            None,
        ));
    }
    if let Some(b) = wd.and_then(|w| s(w, "birthday")) {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.edit.birthday"),
            &b.chars().take(10).collect::<String>(),
            false,
            None,
        ));
    }
    rows.push(ui::detail_row(
        tk,
        &t("connections:detail.connectionDetails.created"),
        &crate::format::short_date(wd.and_then(|w| s(w, "created_at"))),
        false,
        None,
    ));
    let copied = app.wallets.detail.is_copied("wd-copy-id", app.clock);
    rows.push(ui::detail_row(
        tk,
        &t("connections:detail.connectionDetails.kassiberId"),
        &c.id,
        true,
        Some(ui::copy_button(
            tk,
            "wd-copy-id",
            &c.id,
            None,
            false,
            copied,
        )),
    ));

    let edit = button_surface(
        tk,
        row()
            .key("wd-details-edit")
            .h(tk.r(1.75))
            .px(tk.r(0.5))
            .gap(tk.r(0.375))
            .center(),
        Variant::Ghost,
    )
    .role("button")
    .label(t("common:actions.edit"))
    .focusable(true)
    .on_press(super::actions::open_edit)
    .child(ic(&I::PENCIL, tk.r(0.875), tk.fg))
    .child(
        text(t("common:actions.edit"))
            .font(tk.txs(500.0))
            .color(tk.fg),
    );
    // `grid gap-x-10 gap-y-3 @3xl/wallet:grid-cols-2`, row-major.
    let two = width >= tk.r(48.0);
    let grid: El = if two {
        let mut g = col().w_full().gap(tk.r(0.75));
        let mut it = rows.into_iter();
        loop {
            let Some(a) = it.next() else { break };
            let b = it.next();
            let mut r = row()
                .w_full()
                .gap(tk.r(2.5))
                .center_y()
                .child(col().grow(1.0).min_w(0.0).child(a));
            r = r.child(col().grow(1.0).min_w(0.0).children(b));
            g = g.child(r);
        }
        g
    } else {
        col().w_full().gap(tk.r(0.75)).children(rows)
    };
    ui::card(tk)
        .key("wd-connection-details")
        .child(
            ui::strip(tk)
                .justify_between()
                .child(
                    text(t("connections:detail.connectionDetails.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(edit.my(-tk.r(0.25))),
        )
        .child(col().w_full().px(tk.r(1.0)).py(tk.r(0.75)).child(grid))
}

/// `ImportRunsPanel`: nothing when the connection has no file imports.
fn imports(app: &App, tk: &Tk) -> Option<El> {
    let batches = &app.wallets.detail.imports;
    if batches.is_empty() {
        return None;
    }
    let mut list = col().w_full().px(tk.r(1.0));
    let n = batches.len();
    for (i, b) in batches.iter().enumerate() {
        let name = s(b, "source_filename")
            .or_else(|| s(b, "source_format"))
            .unwrap_or("")
            .to_owned();
        let present = f(b, "rows_present").unwrap_or(0.0);
        let date = s(b, "imported_at")
            .map(super::coins::date_label)
            .unwrap_or_default();
        let mut title = row()
            .gap(tk.r(0.5))
            .center_y()
            .child(text(name).font(tk.tsm(500.0)).color(tk.fg).truncate());
        if b.get("column_map").is_some_and(|m| m.is_object()) {
            title = title.child(ui::badge_outline(
                tk,
                &t("connections:detail.imports.mapped"),
            ));
        }
        let batch = b.clone();
        let mut rb = button(
            tk,
            format!("wd-rollback-{i}"),
            Variant::Outline,
            Size::Sm,
            Some(&I::ROTATE_CCW),
            &t("connections:detail.imports.rollback"),
            present > 0.0,
        );
        if present > 0.0 {
            rb = rb.on_press(move |a: &mut App| super::actions::open_rollback(a, batch.clone()));
        }
        let mut item = row()
            .w_full()
            .py(tk.r(0.75))
            .gap(tk.r(0.5))
            .center_y()
            .justify_between()
            .child(
                col().min_w(0.0).child(title).child(
                    text(tc(
                        "connections:detail.imports.summary",
                        present as usize,
                        &[("date", &date)],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                ),
            )
            .child(rb);
        if i + 1 < n {
            item = item
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout();
        }
        list = list.child(item);
    }
    Some(default_card(
        tk,
        ui::card_title(tk, &t("connections:detail.imports.title")),
        &t("connections:detail.imports.description"),
        vec![list],
        false,
    ))
}

fn section_label(v: Option<&str>) -> String {
    match v {
        Some(k @ ("deposit" | "badbank" | "premix" | "postmix" | "ricochet")) => {
            t(&format!("connections:detail.samourai.sectionLabel.{k}"))
        }
        Some(other) => other.to_owned(),
        None => t("connections:detail.samourai.groupFallback"),
    }
}

fn samourai(app: &App, tk: &Tk, c: &Conn, m: &Value) -> El {
    let parent = s(m, "role") == Some("parent");
    let mut title = row()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(ui::card_title(tk, &t("connections:detail.samourai.title")));
    title = title.child(ui::badge_secondary(
        tk,
        &if parent {
            t("connections:detail.samourai.group")
        } else {
            section_label(s(m, "section"))
        },
    ));
    if m.get("privacy_boundary").and_then(Value::as_bool) == Some(true) {
        title = title.child(ui::badge_outline(
            tk,
            &t("connections:detail.samourai.privacyBoundary"),
        ));
    }
    if m.get("paynym").and_then(Value::as_bool) == Some(true) {
        title = title.child(ui::badge_outline(tk, "BIP47"));
    }
    let mut rows: Vec<El> = vec![];
    let source = match s(m, "source") {
        Some("source_set") => t("connections:detail.samourai.sourceLabel.sourceSet"),
        Some(x) => x.to_owned(),
        None => t("connections:detail.samourai.sourceLabel.imported"),
    };
    rows.push(ui::detail_row(
        tk,
        &t("connections:detail.samourai.source"),
        &source,
        false,
        None,
    ));
    rows.push(ui::detail_row(
        tk,
        &t("connections:detail.samourai.group"),
        s(m, "group_label").unwrap_or(&c.label),
        false,
        None,
    ));
    if !parent {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.section"),
            &section_label(s(m, "section")),
            false,
            None,
        ));
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.script"),
            s(m, "script_type").unwrap_or("—"),
            false,
            None,
        ));
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.root"),
            s(m, "root_path").unwrap_or("—"),
            true,
            None,
        ));
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.gap"),
            &f(m, "gap_limit")
                .or(c.gap)
                .map(count)
                .unwrap_or_else(|| "—".into()),
            false,
            None,
        ));
    } else {
        let secs = m
            .get("sections")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|x| section_label(x.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.sections"),
            &secs,
            false,
            None,
        ));
    }
    let conf = s(m, "mix_count_confidence");
    if let Some(v) = f(m, "minimum_mix_count").filter(|v| *v > 0.0) {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.minimumMixCount"),
            &tr(
                "connections:detail.samourai.minimumMixCountValue",
                &[
                    ("value", &count(v)),
                    (
                        "confidence",
                        &conf
                            .map(str::to_owned)
                            .unwrap_or_else(|| t("connections:detail.samourai.minimumConfidence")),
                    ),
                ],
            ),
            false,
            None,
        ));
    }
    if let Some(v) = f(m, "mix_count").filter(|v| *v > 0.0) {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.observedMixes"),
            &tr(
                "connections:detail.samourai.observedMixesValue",
                &[
                    ("value", &count(v)),
                    (
                        "confidence",
                        &conf
                            .map(str::to_owned)
                            .unwrap_or_else(|| t("connections:detail.samourai.importedConfidence")),
                    ),
                ],
            ),
            false,
            None,
        ));
    }
    if let Some(v) = f(m, "target_mix_count").filter(|v| *v > 0.0) {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.targetMixes"),
            &count(v),
            false,
            None,
        ));
    }
    if let Some(v) = f(m, "pool_denomination_sat").filter(|v| *v > 0.0) {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.pool"),
            &tr(
                "connections:detail.samourai.poolValue",
                &[("value", &count(v))],
            ),
            false,
            None,
        ));
    }
    let inv = app.wallets.detail.utxos.ready();
    if !parent {
        rows.push(ui::detail_row(
            tk,
            &t("connections:detail.samourai.coins"),
            &inv.and_then(super::hero::active_count)
                .map(count)
                .unwrap_or_else(|| "—".into()),
            false,
            None,
        ));
    }
    let mut grid = col().w_full().px(tk.r(1.0)).gap(tk.r(0.75));
    let mut it = rows.into_iter();
    while let Some(a) = it.next() {
        grid = grid.child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .child(col().grow(1.0).min_w(0.0).child(a))
                .child(col().grow(1.0).min_w(0.0).children(it.next())),
        );
    }
    let (amber_fill, amber_border, amber_ink) = if tk.dark {
        (
            h("#451a03").alpha(0.4),
            h("#78350f").alpha(0.6),
            h("#fde68a"),
        )
    } else {
        (h("#fffbeb"), h("#fcd34d"), h("#92400e"))
    };
    let box_ = |s: String, warn: bool| {
        let r = row()
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .radius(tk.r(0.375))
            .border_layout();
        if warn {
            r.fill(amber_fill)
                .border(1.0, amber_border)
                .child(text(s).font(tk.tsm(400.0)).color(amber_ink).wrap().w_full())
        } else {
            r.fill(tk.background).border(1.0, tk.border).child(
                text(s)
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .w_full(),
            )
        }
    };
    let mut foot = col()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout();
    if m.get("toxic_change").and_then(Value::as_bool) == Some(true) {
        foot = foot.child(box_(t("connections:detail.samourai.toxicChange"), true));
    }
    if s(m, "section") == Some("postmix") {
        foot = foot.child(box_(t("connections:detail.samourai.postmixNote"), false));
    }
    if inv
        .and_then(|i| i.get("freshness"))
        .and_then(|x| x.get("stale"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        foot = foot.child(box_(t("connections:detail.samourai.stale"), true));
    }
    let running = super::sync_running(app);
    let label = t(if running {
        "connections:detail.refreshing"
    } else {
        "connections:detail.samourai.refreshSource"
    });
    let mut refresh = button(
        tk,
        "wd-samourai-refresh",
        Variant::Outline,
        Size::Sm,
        Some(&I::REFRESH_CW),
        &label,
        !running,
    );
    if !running {
        refresh = refresh.on_press(|a: &mut App| super::actions::sync(a, false));
    }
    foot = foot.child(refresh);
    default_card(
        tk,
        title,
        &t("connections:detail.samourai.description"),
        vec![grid.pb(tk.r(0.25)), foot],
        true,
    )
}

fn provenance(tk: &Tk, routes: &[Value]) -> El {
    let cell = |s: &str, head: bool| {
        row().grow(1.0).min_w(0.0).px(tk.r(0.5)).child(
            text(s.to_owned())
                .font(tk.tsm(if head { 500.0 } else { 400.0 }))
                .color(tk.fg)
                .truncate(),
        )
    };
    let mut table = col().w_full();
    table = table.child(
        row()
            .w_full()
            .h(tk.r(2.5))
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout()
            .child(cell(
                &t("connections:detail.btcpayProvenance.instance"),
                true,
            ))
            .child(cell(&t("connections:detail.btcpayProvenance.store"), true))
            .child(cell(
                &t("connections:detail.btcpayProvenance.paymentMethod"),
                true,
            )),
    );
    for (i, r) in routes.iter().enumerate() {
        let mut tr_ = row()
            .w_full()
            .min_h(tk.r(2.3))
            .center_y()
            .hover(tk.muted.alpha(0.5))
            .child(cell(s(r, "backend").unwrap_or(""), false))
            .child(cell(s(r, "store_id").unwrap_or(""), false))
            .child(cell(s(r, "payment_method_id").unwrap_or(""), false));
        if i + 1 < routes.len() {
            tr_ = tr_
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout();
        }
        table = table.child(tr_);
    }
    // The footer's <strong>/<em>/<code> markup reads as plain text here.
    let footer = t("connections:detail.btcpayProvenance.footer")
        .replace("<strong>", "")
        .replace("</strong>", "")
        .replace("<em>", "")
        .replace("</em>", "")
        .replace("<code>", "")
        .replace("</code>", "");
    default_card(
        tk,
        ui::card_title(tk, &t("connections:detail.btcpayProvenance.title")),
        &t("connections:detail.btcpayProvenance.description"),
        vec![
            table,
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.625))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(
                    text(footer)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        ],
        true,
    )
}
