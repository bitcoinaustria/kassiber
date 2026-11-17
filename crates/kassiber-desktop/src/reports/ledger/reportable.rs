//! The Ledger entries tab (components/kb/JournalReportableEntries.tsx over the "journal-events"
//! `ReviewDataTable`): the description and Process journals, five metric filters, then the
//! processed entries worklist with search, a date sort, the active-filter strip and pages of 8.

use rune::{Color, Cx, El, TextEdit, col, row, text};
use rune_kit::TextInput;
use serde_json::Value;

use super::common::{self as c, Tone, sens, tnum};
use super::journals::{entry_type_label, humanize};
use crate::app::{App, Load};
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::{MASK, Tk};
use crate::txdetail::widgets::{self as w, Variant};
use crate::ui::{ic, surface};

pub const LIMIT: usize = 500;
const PAGE: usize = 8;

/// The worklist's own state (search, metric filter, sort, page).
pub struct TableUi {
    pub search: TextEdit,
    pub metric: Option<String>,
    /// Date order: descending (the remembered default) or ascending.
    pub desc: bool,
    pub page: usize,
}

impl Default for TableUi {
    fn default() -> Self {
        TableUi {
            search: TextEdit::new(""),
            metric: None,
            desc: true,
            page: 0,
        }
    }
}

/// A `ReviewTableRow` as `reportableEntryToRow` builds it.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: String,
    pub date: String,
    pub account: String,
    pub event: String,
    pub source: String,
    pub amount: String,
    pub basis: String,
    pub impact: String,
    pub ready: bool,
    pub evidence: String,
    pub next_action: String,
    pub filters: Vec<&'static str>,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn opt_f(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// `stripJournalMarkers`: drops the engine's `at_…=` tokens.
pub fn strip_markers(d: &str) -> String {
    d.split_whitespace()
        .filter(|tok| {
            let Some(rest) = tok.strip_prefix("at_") else {
                return true;
            };
            let Some((name, _)) = rest.split_once('=') else {
                return true;
            };
            !(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `formatMsatAmount`: whole sats in the UI grouping, `sats` and the asset.
pub fn msat_amount(msat: f64, asset: &str) -> String {
    let sats = (msat.abs() / 1000.0).round() as usize;
    let sign = if msat < 0.0 { "-" } else { "" };
    format!("{sign}{} sats {asset}", crate::format::count(sats))
        .trim()
        .to_owned()
}

fn short_id(v: &str) -> String {
    if v.chars().count() > 12 {
        format!("{}...", v.chars().take(10).collect::<String>())
    } else {
        v.to_owned()
    }
}

fn pricing_label(e: &Value) -> String {
    let (source, quality) = (s(&e["pricingSourceKind"]), s(&e["pricingQuality"]));
    if source.is_empty() && quality.is_empty() {
        return t("journals:reportable.pricing.label");
    }
    match source {
        "manual_override" => t("journals:reportable.pricing.manual"),
        "source_price" => t("journals:reportable.pricing.source"),
        "rate_cache" => t("journals:reportable.pricing.cached"),
        "fmv_provider" => t(if quality == "provider_sample" {
            "journals:reportable.pricing.providerSample"
        } else {
            "journals:reportable.pricing.providerPrice"
        }),
        _ => {
            let label = humanize(if source.is_empty() { "pricing" } else { source });
            if quality.is_empty() {
                label
            } else {
                tr(
                    "journals:reportable.pricing.qualified",
                    &[("source", &label), ("quality", &humanize(quality))],
                )
            }
        }
    }
}

/// `reportableEntryMetricFilterIds`.
pub fn metric_ids(kind: &str) -> Vec<&'static str> {
    match kind {
        "acquisition" => vec!["acquisitions"],
        "disposal" => vec!["disposals"],
        "income" => vec!["income"],
        "fee" | "transfer_fee" => vec!["fees"],
        "neutral_swap" | "transfer_in" | "transfer_out" => vec!["neutral"],
        _ => vec![],
    }
}

/// `reportableEntryToRow`.
/// `cur` is the book's currency: the event's `costBasisEur`, `proceedsEur`, `fiatValueEur` and
/// `gainLossEur` are stored journal amounts in it (the `Eur` names are historical).
pub fn to_row(e: &Value, summary: &Value, cur: &str) -> Row {
    let money = |v: f64| crate::format::fiat(v, cur);
    let needs = summary["needsJournals"].as_bool().unwrap_or(false);
    let kennzahl = e["atKennzahl"].as_i64();
    let at_label = match (kennzahl, e["atCategory"].as_str()) {
        (Some(k), _) => tr(
            "journals:reportable.atKennzahl",
            &[("kennzahl", &k.to_string())],
        ),
        (None, Some(cat)) if !cat.is_empty() => entry_type_label(cat),
        _ => String::new(),
    };
    let kind = s(&e["entryType"]);
    let type_label = entry_type_label(kind);
    let desc = strip_markers(s(&e["description"]));
    let desc = desc.trim();
    let event = if desc.is_empty() {
        type_label
    } else {
        tr(
            "journals:reportable.eventTitle",
            &[("type", &type_label), ("description", desc)],
        )
    };
    let (basis_v, proceeds) = (opt_f(&e["costBasisEur"]), opt_f(&e["proceedsEur"]));
    let basis = if basis_v.is_some() || proceeds.is_some() {
        [
            basis_v.map(|v| tr("journals:reportable.basis.amount", &[("value", &money(v))])),
            proceeds.map(|v| {
                tr(
                    "journals:reportable.basis.proceeds",
                    &[("value", &money(v))],
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    } else if opt_f(&e["fiatValueEur"]).is_some_and(|v| v != 0.0) {
        tr(
            "journals:reportable.basis.value",
            &[("value", &money(e["fiatValueEur"].as_f64().unwrap_or(0.0)))],
        )
    } else {
        t("journals:reportable.basis.noFiatImpact")
    };
    let evidence = if needs {
        s(&summary["freshnessReason"]).to_owned()
    } else if let Some(k) = kennzahl {
        tr(
            "journals:reportable.evidence.austrianMapping",
            &[("kennzahl", &k.to_string())],
        )
    } else if !s(&e["pricingSourceKind"]).is_empty() {
        tr(
            "journals:reportable.evidence.pricedBy",
            &[("source", &pricing_label(e))],
        )
    } else if kind == "transfer_in" || kind == "transfer_out" {
        t("journals:reportable.evidence.transferMatching")
    } else {
        t("journals:reportable.evidence.journalProcessing")
    };
    let next_action = if needs {
        t("journals:reportable.nextAction.needsJournals")
    } else if kennzahl.is_some() {
        t("journals:reportable.nextAction.austrianMapping")
    } else {
        t("journals:reportable.nextAction.reports")
    };
    let id_src = [
        s(&e["transactionExternalId"]),
        s(&e["transactionId"]),
        s(&e["id"]),
    ]
    .into_iter()
    .find(|x| !x.is_empty())
    .unwrap_or("");
    let date_src = [s(&e["occurredAt"]), s(&e["createdAt"])]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or("");
    let account = [s(&e["accountLabel"]), s(&e["account"]), s(&e["wallet"])]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or("")
        .to_owned();
    Row {
        id: short_id(id_src),
        date: if date_src.is_empty() {
            t("journals:review.unknownDate")
        } else {
            date_src.chars().take(10).collect()
        },
        account,
        event,
        source: [s(&e["wallet"]).to_owned(), at_label]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
        amount: msat_amount(e["quantityMsat"].as_f64().unwrap_or(0.0), s(&e["asset"])),
        basis,
        impact: money(opt_f(&e["gainLossEur"]).unwrap_or(0.0)),
        ready: !needs,
        evidence,
        next_action,
        filters: metric_ids(kind),
    }
}

/// `reportableEntryMetrics`: (label, value, tone, filter id, filter label).
pub fn metrics(summary: &Value) -> Vec<(&'static str, f64, Tone, &'static str, &'static str)> {
    let types: Vec<(String, f64)> = summary["entryTypes"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| (s(&e["type"]).to_owned(), e["count"].as_f64().unwrap_or(0.0)))
                .collect()
        })
        .unwrap_or_default();
    let count = |ks: &[&str]| {
        types
            .iter()
            .filter(|(k, _)| ks.contains(&k.as_str()))
            .map(|(_, n)| n)
            .sum::<f64>()
    };
    let needs = summary["needsJournals"].as_bool().unwrap_or(false);
    let (acq, dis, inc, fee, neu) = (
        count(&["acquisition"]),
        count(&["disposal"]),
        count(&["income"]),
        count(&["fee", "transfer_fee"]),
        count(&["neutral_swap", "transfer_in", "transfer_out"]),
    );
    let some = |n: f64, tone: Tone| if n > 0.0 { tone } else { Tone::Neutral };
    vec![
        (
            "Acquisitions",
            acq,
            if needs {
                Tone::Warning
            } else {
                some(acq, Tone::Good)
            },
            "acquisitions",
            "Acquisitions",
        ),
        (
            "Disposals",
            dis,
            some(dis, Tone::Warning),
            "disposals",
            "Disposals",
        ),
        ("Income", inc, some(inc, Tone::Good), "income", "Income"),
        ("Fees", fee, some(fee, Tone::Warning), "fees", "Fees"),
        (
            "Neutral",
            neu,
            some(neu, Tone::Good),
            "neutral",
            "Neutral entries",
        ),
    ]
}

fn matches(r: &Row, metric: Option<&str>, q: &str, owner: &str) -> bool {
    if let Some(m) = metric
        && !r.filters.contains(&m)
    {
        return false;
    }
    if q.is_empty() {
        return true;
    }
    let status = if r.ready { "Ready" } else { "Needs review" };
    let priority = if r.ready { "Low" } else { "Medium" };
    // The row fields in `ReviewDataTable`'s order (`rowKey` and the transaction action are unset).
    let hay = [
        r.id.as_str(),
        "",
        "",
        &r.date,
        &r.account,
        &r.event,
        &r.source,
        &r.amount,
        &r.basis,
        &r.impact,
        status,
        priority,
        owner,
        &r.evidence,
        &r.next_action,
    ];
    hay.join(" ").to_lowercase().contains(q)
}

fn filtered(rows: &[Row], ui: &TableUi, owner: &str) -> Vec<Row> {
    let q = ui.search.text.trim().to_lowercase();
    let mut out: Vec<Row> = rows
        .iter()
        .filter(|r| matches(r, ui.metric.as_deref(), &q, owner))
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        if ui.desc {
            b.date.cmp(&a.date)
        } else {
            a.date.cmp(&b.date)
        }
    });
    out
}

// ---- View ---------------------------------------------------------------------------------

/// A metric filter's glyph.
fn metric_glyph(id: &str) -> &'static rune::Icon {
    match id {
        "acquisitions" => &I::ARROW_DOWN_LEFT,
        "disposals" => &I::ARROW_UP_RIGHT,
        "income" => &I::COINS,
        "fees" => &I::RECEIPT,
        _ => &I::ARROW_LEFT_RIGHT,
    }
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    use crate::ui::table as tb;
    let _ = layer;
    let data = match &app.reports.ledger.journals.events {
        Load::Idle | Load::Loading => {
            return c::screen_skeleton(tk, tk.r(13.0), c::win_w(app, cx) >= 1536.0).padding(0.0);
        }
        Load::Failed(e) => {
            let msg = if e.is_empty() {
                t("journals:reportable.unavailable.fallback")
            } else {
                e.clone()
            };
            return c::centered(
                tk,
                "lg-rep-unavailable",
                &I::TRIANGLE_ALERT,
                tk.alert.solid,
                t("journals:reportable.unavailable.title"),
                msg,
                None,
                c::free_h(app, tk),
            )
            .role("alert");
        }
        Load::Ready(d) => d,
    };
    let summary = &data["summary"];
    let owner = summary["profile"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| t("journals:quarantine.ownerFallback"));
    let cur = crate::format::book_currency(app);
    let rows: Vec<Row> = data["events"]
        .as_array()
        .map(|a| a.iter().map(|e| to_row(e, summary, &cur)).collect())
        .unwrap_or_default();
    let ui = &app.reports.ledger.journals.table;
    let shown = filtered(&rows, ui, &owner);
    let wide = c::win_w(app, cx) >= 1024.0;
    let needs = summary["needsJournals"].as_bool().unwrap_or(false);

    // What the tab lists (Process journals sits in the page header, on both tabs).
    let lede = text(t("journals:reportable.description"))
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .wrap()
        .max_w(tk.r(48.0));

    // Metric filters on the stat strip: a press filters the list; the chosen one stays raised.
    let ms = metrics(summary);
    let mut tiles = Vec::new();
    for (label, value, _tone, id, _) in ms.iter() {
        let active = ui.metric.as_deref() == Some(*id);
        let id_s = id.to_string();
        // Colour only for the stale state (acquisitions wait on a journal run).
        let ink = if needs && *id == "acquisitions" {
            tk.warn.solid
        } else {
            tk.fg
        };
        let mut m = col()
            .key(format!("lg-rep-metric-{id}"))
            .w(0.0)
            .grow(1.0)
            .min_w(0.0)
            .padding(tk.r(1.0))
            .gap(tk.r(0.5))
            .role("button")
            .checked(active)
            .label(tr("journals:review.metricAria.filter", &[("label", label)]))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |a: &mut App| {
                let t = &mut a.reports.ledger.journals.table;
                t.metric = Some(id_s.clone());
                t.page = 0;
            })
            .child(
                row()
                    .min_w(0.0)
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(
                        ic(
                            metric_glyph(id),
                            tk.r(1.0),
                            if active { tk.fg } else { tk.muted_fg },
                        )
                        .a11y_hidden(),
                    )
                    .child(
                        text(label.to_string())
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .truncate()
                            .min_w(0.0),
                    ),
            )
            .child(c::stat_value(
                tk,
                crate::format::count(*value as usize),
                ink,
                false,
            ));
        m = if active {
            m.fill(tk.muted.alpha(0.6))
        } else {
            m.hover(tk.muted.alpha(0.35))
        };
        tiles.push(m);
    }
    let strip = c::stat_strip(
        tk,
        "lg-rep-metrics",
        t("journals:ledger.tabs.reportable"),
        tiles,
    );

    // Card header: title and its line, the search on the right (below it when narrow).
    let k = w::form_kit(tk);
    let search = w::field(
        tk,
        TextInput::new(&ui.search)
            .placeholder(t("journals:reportable.searchPlaceholder"))
            .label(t("journals:reportable.searchPlaceholder"))
            .leading_icon(&I::SEARCH)
            .leading_icon_size(tk.r(0.875))
            .height(tk.r(2.0))
            .text_style(tk.tsm(400.0))
            .on_edit(|a: &mut App| &mut a.reports.ledger.journals.table.search)
            .on_change(|a: &mut App, _| a.reports.ledger.journals.table.page = 0)
            .build(&k)
            .key("lg-rep-search"),
    )
    .radius(tk.r(0.375));
    let detail = summary["freshnessReason"].as_str().unwrap_or("");
    let desc = tr(
        "journals:review.tableDescription.detail",
        &[("rows", &shown.len().to_string()), ("detail", detail)],
    );
    let mut card = surface(tk)
        .key("lg-rep-card")
        .w_full()
        .role("region")
        .label(t("journals:reportable.tableTitle"))
        .clip_rounded(tk.r(0.75));
    if wide {
        card = card.child(c::card_head(
            tk,
            t("journals:reportable.tableTitle"),
            Some(desc),
            Some(search.w(tk.r(20.0))),
        ));
    } else {
        card = card
            .child(c::card_head(
                tk,
                t("journals:reportable.tableTitle"),
                Some(desc),
                None,
            ))
            .child(
                row()
                    .w_full()
                    .px(tk.r(tb::EDGE))
                    .pb(tk.r(0.75))
                    .child(search.w_full()),
            );
    }

    // Active filters.
    let search_on = !ui.search.text.is_empty();
    if ui.metric.is_some() || search_on {
        let mut strip = row()
            .w_full()
            .px(tk.r(tb::EDGE))
            .pb(tk.r(0.75))
            .gap(tk.r(0.5))
            .center_y()
            .flex_wrap()
            .child(
                text(t("journals:review.filters.active"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
        if let Some(m) = ms.iter().find(|m| Some(m.3) == ui.metric.as_deref()) {
            let label = m.4.to_owned();
            strip = strip.child(
                row()
                    .key("lg-rep-clear-metric")
                    .h(tk.r(1.75))
                    .px(tk.r(0.5))
                    .gap(tk.r(0.375))
                    .center_y()
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border)
                    .border_layout()
                    .fill(tk.background)
                    .hover(tk.muted)
                    .role("button")
                    .label(tr(
                        "journals:review.filters.clearMetricAria",
                        &[("label", &label)],
                    ))
                    .focusable(true)
                    .on_press(|a: &mut App| {
                        a.reports.ledger.journals.table.metric = None;
                        a.reports.ledger.journals.table.page = 0;
                    })
                    .child(text(label).font(tk.txs(400.0)).color(tk.fg))
                    .child(ic(&I::X, tk.r(0.75), tk.fg)),
            );
        }
        strip = strip.child(
            w::button(
                tk,
                "lg-rep-clear",
                Variant::Ghost,
                w::Size::Xs,
                None,
                &t("journals:review.filters.clearAll"),
                true,
            )
            .on_press(|a: &mut App| {
                let t = &mut a.reports.ledger.journals.table;
                t.metric = None;
                t.search = TextEdit::new("");
                t.page = 0;
            }),
        );
        card = card.child(strip);
    }

    // The table, on the kit: Event and Evidence share what the figures and the date leave.
    let hidden = app.prefs.hide_sensitive();
    let pages = shown.len().div_ceil(PAGE).max(1);
    let page = ui.page.min(pages - 1);
    let slice = &shown[(page * PAGE).min(shown.len())..((page + 1) * PAGE).min(shown.len())];
    let (sm5, xs, mono) = (tb::num_style(tk), tk.txs(400.0), tk.txs(400.0).mono());
    let masked = |s: &str| {
        if hidden {
            MASK.to_owned()
        } else {
            s.to_owned()
        }
    };
    let labels = [
        t("journals:review.column.event"),
        t("journals:review.column.evidence"),
        t("journals:review.column.amountBasis"),
        t("journals:review.column.impact"),
        t("common:field.date"),
    ];
    let mut content = [
        0.0f32,
        0.0,
        c::tw(&labels[2], tk.tsm(500.0)),
        c::tw(&labels[3], tk.tsm(500.0)),
        c::tw(&labels[4], tk.tsm(500.0)) + tk.r(0.25) + tk.r(0.875),
    ];
    for r in slice {
        content[2] = content[2]
            .max(c::tw(&masked(&r.amount), sm5).max(c::tw(&masked(&r.basis), tb::sub_style(tk))));
        content[3] = content[3].max(c::tw(&masked(&r.impact), sm5));
        content[4] = content[4].max(c::tw(&r.date, tnum(tk.tsm(400.0))));
    }
    let px = 2.0 * tk.r(tb::PX);
    let edge = tk.r(tb::EDGE) - tk.r(tb::PX);
    let mut widths = [0.0f32; 5];
    for k in 2..5 {
        widths[k] = (content[k] + px).ceil();
    }
    widths[4] += edge;
    let fixed: f32 = widths.iter().sum::<f32>() + edge;
    let (event_min, evidence_min) = (tk.r(16.0), tk.r(11.0));
    let avail = c::avail(cx, "lg-rep-table", c::page_w(app, cx, tk) - 2.0);
    let (table_w, overflow) = tb::fit(avail, fixed + event_min + evidence_min);
    let rest = table_w - fixed;
    widths[0] = (rest * 0.58).max(event_min) + edge;
    widths[1] = rest + edge - widths[0];
    let right = [false, false, true, true, true];
    let pad = |k: usize, cell: El| match k {
        0 => cell.pl(tk.r(tb::EDGE)),
        4 => cell.pr(tk.r(tb::EDGE)),
        _ => cell,
    };
    let sort = tb::sort_label(
        tk,
        "lg-rep-sort",
        &labels[4],
        t("common:field.date"),
        Some(!ui.desc),
        true,
    )
    .on_press(|a: &mut App| {
        let t = &mut a.reports.ledger.journals.table;
        t.desc = !t.desc;
    });
    let header = tb::head(tk).children((0..5).map(|k| {
        let content = if k == 4 {
            sort.clone()
        } else {
            tb::th_text(tk, labels[k].clone())
        };
        pad(k, tb::th(tk, Some(widths[k]), right[k], content))
    }));
    let mut rows_el: Vec<El> = Vec::new();
    for (i, r) in slice.iter().enumerate() {
        // The table kit's leading slot: a plain entry glyph, or the amber clock while the
        // entry still waits (no standing "ready" badge).
        let square = if r.ready {
            tb::lead_glyph(tk, &I::FILE_TEXT, tk.muted_fg)
        } else {
            tb::lead_tile(tk, &I::CLOCK_3, tb::Tone::Warning)
        };
        let event = row()
            .w_full()
            .min_w(0.0)
            .gap(tk.r(tb::LEAD_GAP))
            .items_start()
            .child(square)
            .child(
                col()
                    .min_w(0.0)
                    .grow(1.0)
                    .w(0.0)
                    .child(tb::cell_text(r.event.clone(), tk.tsm(500.0), tk.fg))
                    .child(
                        row()
                            .w_full()
                            .min_w(0.0)
                            .gap(tk.r(0.375))
                            .center_y()
                            .child(
                                text(r.id.clone())
                                    .font(mono)
                                    .color(tk.muted_fg)
                                    .single_line(),
                            )
                            .child(text("·").font(xs).color(tk.muted_fg))
                            .child(if hidden {
                                sens(r.account.clone(), xs, tk.muted_fg, true)
                                    .single_line()
                                    .truncate()
                                    .min_w(0.0)
                            } else {
                                tb::cell_text(r.account.clone(), xs, tk.muted_fg)
                            }),
                    ),
            );
        let evidence = col()
            .w_full()
            .min_w(0.0)
            .child(tb::cell_text(r.source.clone(), tk.tsm(400.0), tk.fg))
            .child(tb::cell_text(r.evidence.clone(), xs, tk.muted_fg));
        let amount = col()
            .items_end()
            .child(tb::num(tk, masked(&r.amount), tk.fg, hidden))
            .child(tb::num_sub(tk, masked(&r.basis), hidden));
        let impact = tb::num(tk, masked(&r.impact), impact_ink(tk, &r.impact), hidden);
        let date = text(r.date.clone())
            .font(tnum(tk.tsm(400.0)))
            .color(tk.muted_fg)
            .single_line();
        let cells: Vec<El> = [event, evidence, amount, impact, date]
            .into_iter()
            .enumerate()
            .map(|(k, c)| pad(k, tb::td(tk, Some(widths[k]), right[k], tb::PY, c)))
            .collect();
        rows_el.push(
            tb::tr(tk, i + 1 == slice.len())
                .items_stretch()
                .children(cells)
                .key(format!("lg-rep-row-{i}")),
        );
    }
    if rows_el.is_empty() {
        rows_el.push(tb::empty_text(tk, t("journals:reportable.empty")));
    }
    card = card.child(tb::hscroll_if(
        "lg-rep-table",
        t("journals:reportable.tableTitle"),
        overflow,
        col().w(table_w).child(header).children(rows_el),
    ));

    // Pagination footer.
    let range = if shown.is_empty() {
        t("journals:review.pagination.noRows")
    } else {
        tr(
            "journals:review.pagination.range",
            &[
                ("from", &(page * PAGE + 1).to_string()),
                ("to", &((page + 1) * PAGE).min(shown.len()).to_string()),
                ("total", &shown.len().to_string()),
            ],
        )
    };
    let nav = |key: &str, glyph: &'static rune::Icon, label: String, enabled: bool, delta: i32| {
        let b = w::icon_button(
            tk,
            key.to_owned(),
            Variant::Outline,
            tk.r(1.75),
            glyph,
            tk.r(1.0),
            label,
        )
        .disabled(!enabled)
        .focusable(enabled)
        .opacity(if enabled { 1.0 } else { 0.5 });
        if enabled {
            b.on_press(move |a: &mut App| {
                let t = &mut a.reports.ledger.journals.table;
                t.page = (t.page.min(pages - 1) as i32 + delta).clamp(0, pages as i32 - 1) as usize;
            })
        } else {
            b
        }
    };
    let footer = row()
        .w_full()
        .px(tk.r(tb::EDGE))
        .py(tk.r(0.625))
        .center_y()
        .justify_between()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(text(range).font(tnum(tk.txs(400.0))).color(tk.muted_fg))
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(nav(
                    "lg-rep-prev",
                    &I::CHEVRON_LEFT,
                    t("journals:review.pagination.previous"),
                    page > 0,
                    -1,
                ))
                .child(
                    text(tr(
                        "journals:review.pagination.page",
                        &[
                            ("current", &(page + 1).to_string()),
                            ("count", &pages.to_string()),
                        ],
                    ))
                    .font(tnum(tk.txs(400.0)))
                    .color(tk.muted_fg),
                )
                .child(nav(
                    "lg-rep-next",
                    &I::CHEVRON_RIGHT,
                    t("journals:review.pagination.next"),
                    page + 1 < pages,
                    1,
                )),
        );
    card = card.child(footer);

    col()
        .w_full()
        .gap(tk.r(0.75))
        .child(lede)
        .child(strip)
        .child(card)
        .fill(Color::TRANSPARENT)
}

/// `impactToneClass`, signal only: a loss in red, everything else in the foreground (the web's
/// emerald on every gain read as a wall of "all good").
fn impact_ink(tk: &Tk, impact: &str) -> Color {
    if impact.trim().starts_with('-') {
        tk.alert.solid
    } else {
        tk.fg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Basis, proceeds, value and impact are the book's currency, not euros.
    #[test]
    fn ledger_rows_render_in_the_book_currency() {
        let summary = serde_json::json!({"needsJournals": false});
        let sell = serde_json::json!({
            "entryType": "sell", "occurredAt": "2025-03-01T10:00:00Z", "asset": "BTC",
            "quantityMsat": -1_000_000_000.0, "costBasisEur": 700.0, "proceedsEur": 950.5,
            "gainLossEur": 250.5,
        });
        let r = to_row(&sell, &summary, "USD");
        assert!(
            r.basis.contains("$700.00") && r.basis.contains("$950.50"),
            "{}",
            r.basis
        );
        assert_eq!(r.impact, "$250.50");
        let income = serde_json::json!({"entryType": "income", "fiatValueEur": 42.0});
        let r = to_row(&income, &summary, "GBP");
        assert!(r.basis.contains("£42.00"), "{}", r.basis);
        assert!(!r.basis.contains('€') && !r.impact.contains('€'));
    }

    #[test]
    fn markers_are_stripped() {
        assert_eq!(
            strip_markers("Synced at_regime=neu from at_pool=x core"),
            "Synced from core"
        );
        assert_eq!(strip_markers("at_Bad=1 keep"), "at_Bad=1 keep");
    }

    #[test]
    fn metric_ids_follow_the_model() {
        assert_eq!(metric_ids("transfer_fee"), vec!["fees"]);
        assert_eq!(metric_ids("transfer_in"), vec!["neutral"]);
        assert!(metric_ids("other").is_empty());
    }

    #[test]
    fn msat_amounts_round_to_sats() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(msat_amount(-7_352_650_000.0, "BTC"), "-7,352,650 sats BTC");
        assert_eq!(msat_amount(1_499.0, "BTC"), "1 sats BTC");
    }

    #[test]
    fn short_ids() {
        assert_eq!(short_id("373b15e0a627488e80c3"), "373b15e0a6...");
        assert_eq!(short_id("abc"), "abc");
    }
}
