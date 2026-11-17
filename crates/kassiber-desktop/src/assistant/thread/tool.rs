//! The work log's pieces: reasoning panes (ChatReasoning.tsx), the collapsed "Used N tools" row
//! (ChainOfThought), and a tool call's card (ChatToolCall.tsx over ai-elements/tool.tsx) with its
//! one-line result summary (summarizeToolResult, overviewToolSummary.ts).

use rune::{Color, El, col, row, text};
use serde_json::Value;

use super::{is_open, md, toggle};
use crate::app::App;
use crate::assistant::model::{ToolCall, ToolStatus};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// `ChainOfThoughtHeader`: `inline-flex gap-1.5 py-0.5 text-xs font-medium muted`, an icon, the
/// label and a chevron that turns while open.
pub fn cot_header(
    tk: &Tk,
    key: String,
    glyph: &'static rune::Icon,
    label: El,
    open: bool,
    spin: bool,
) -> El {
    let mut icon = ic(glyph, tk.r(0.75), tk.muted_fg);
    if spin {
        icon = icon.looping(rune::LoopSpec::rotation(0.9));
    }
    let k = key.clone();
    row()
        .key(format!("{key}-trigger"))
        .min_w(0.0)
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .py(tk.r(0.125))
        .role("button")
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| toggle(a, &k, false))
        .child(icon.a11y_hidden())
        .child(label.min_w(0.0))
        .child(
            ic(
                if open {
                    &I::CHEVRON_UP
                } else {
                    &I::CHEVRON_DOWN
                },
                tk.r(0.75),
                tk.muted_fg,
            )
            .a11y_hidden(),
        )
}

/// One reasoning pane ("Thoughts" / "Thinking…"), collapsed by default; open, its markdown sits
/// behind a left rule (`mt-1 border-l pl-3`).
pub fn reasoning(app: &App, tk: &Tk, key: &str, content: &str, active: bool) -> El {
    let open = is_open(app, key, false);
    let label = if active {
        format!("{}...", t("assistant:message.thinking"))
    } else if let Some((_, secs)) = app.assistant.thread.thought.get(key) {
        // Watched while it was written: how long it took (`message.thoughtFor`).
        let d = if *secs > 0 {
            format!("{secs}s")
        } else {
            "<1s".to_owned()
        };
        crate::i18n::tr("assistant:message.thoughtFor", &[("duration", &d)])
    } else {
        t("assistant:message.thoughts")
    };
    let shown = if active {
        // `ThinkingHeaderLabel`: the word pulses (`thinking-pulse` 2.2 s, 0.55–1) and three dots
        // blink in turn (`thinking-dot` 1.4 s, 0.2 s apart) in a 1.25 em box.
        let mut dots = row().w(tk.r(0.75) * 1.25).a11y_hidden();
        for i in 0..3 {
            dots = dots.child(
                text(".")
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg)
                    .looping(rune::LoopSpec::opacity(1.4, 0.2 * i as f64, 0.15, 1.0)),
            );
        }
        row()
            .min_w(0.0)
            .center_y()
            .child(
                text(t("assistant:message.thinking"))
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg)
                    .truncate()
                    .looping(rune::LoopSpec::opacity(2.2, 0.0, 0.55, 1.0)),
            )
            .child(dots)
    } else {
        text(label.clone())
            .font(tk.txs(500.0))
            .color(tk.muted_fg)
            .truncate()
    };
    let header = cot_header(tk, key.to_owned(), &I::SPARKLES, shown, open, false).label(label);
    let mut c = col()
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .mb(tk.r(0.5))
        .child(header);
    if open {
        c = c.child(
            col()
                .w_full()
                .mt(tk.r(0.25))
                .py(2.0)
                .pl(tk.r(0.75))
                .border_sides([0.0, 0.0, 0.0, 1.0], tk.border.alpha(tk.border.a * 0.7))
                .child(md::view(app, tk, key, content, md::Look::Reasoning)),
        );
    }
    c
}

pub fn is_active(c: &ToolCall) -> bool {
    matches!(c.status, ToolStatus::Running | ToolStatus::Pending)
}

/// "Used N tools": a run of routine calls folded into one row; open, their cards behind a rule.
pub fn group(app: &App, tk: &Tk, key: &str, calls: &[&ToolCall]) -> El {
    let open = is_open(app, key, false);
    let active = calls.iter().find(|c| is_active(c));
    let n = calls.len();
    let mut label = row().min_w(0.0).gap(tk.r(0.375)).center_y().child(
        text(tc(
            if active.is_some() {
                "assistant:message.toolGroupRunning"
            } else {
                "assistant:message.toolGroup"
            },
            n,
            &[],
        ))
        .font(tk.txs(500.0))
        .color(tk.muted_fg)
        .truncate(),
    );
    if let Some(a) = active {
        label = label.child(
            text(a.name.clone())
                .font(tk.t2xs(400.0).mono())
                .color(tk.muted_fg)
                .truncate(),
        );
    }
    let name = tc(
        if active.is_some() {
            "assistant:message.toolGroupRunning"
        } else {
            "assistant:message.toolGroup"
        },
        n,
        &[],
    );
    let header = cot_header(
        tk,
        key.to_owned(),
        if active.is_some() {
            &I::LOADER_CIRCLE
        } else {
            &I::WRENCH
        },
        label,
        open,
        active.is_some(),
    )
    .label(name);
    let mut c = col()
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .mb(tk.r(0.5))
        .child(header);
    if open {
        let mut list = col()
            .w_full()
            .mt(tk.r(0.25))
            .gap(tk.r(0.25))
            .py(2.0)
            .pl(tk.r(0.75))
            .border_sides([0.0, 0.0, 0.0, 1.0], tk.border.alpha(tk.border.a * 0.7));
        for call in calls {
            list = list.child(card(app, tk, call));
        }
        c = c.child(list);
    }
    c
}

fn state_label(s: ToolStatus) -> &'static str {
    // ai-elements/tool.tsx hardcodes these in English.
    match s {
        ToolStatus::Pending => "Pending",
        ToolStatus::AwaitingConsent => "Awaiting consent",
        ToolStatus::Running => "Running",
        ToolStatus::Done => "Done",
        ToolStatus::Denied => "Denied",
        ToolStatus::Error => "Error",
    }
}

fn state_icon(tk: &Tk, s: ToolStatus) -> El {
    let emerald = rune::color::hex("#059669").unwrap_or(tk.good.solid);
    let amber = rune::color::hex("#d97706").unwrap_or(tk.warn.solid);
    let (g, c): (&'static rune::Icon, Color) = match s {
        ToolStatus::Pending => (&I::WRENCH, tk.muted_fg),
        ToolStatus::AwaitingConsent => (&I::SHIELD_CHECK, tk.primary),
        ToolStatus::Running => (&I::LOADER_CIRCLE, tk.muted_fg),
        ToolStatus::Done => (&I::CIRCLE_CHECK, emerald),
        ToolStatus::Denied => (&I::SHIELD_ALERT, amber),
        ToolStatus::Error => (&I::CIRCLE_X, tk.destructive),
    };
    let mut e = ic(g, tk.r(0.75), c).a11y_hidden();
    if s == ToolStatus::Running {
        e = e.looping(rune::LoopSpec::rotation(0.9));
    }
    e
}

/// A status label (`secondary`, or the destructive ink for errors), uppercase.
fn badge(tk: &Tk, label: &str, destructive: bool) -> El {
    let fg = if destructive {
        tk.destructive
    } else {
        tk.muted_fg
    };
    text(label.to_uppercase()).font(tk.t3xs(500.0)).color(fg)
}

/// `ToolInput` / `ToolOutput`: a labelled section (`border-t px-2 py-1.5`), plain text in a
/// muted pill or JSON in a scrolling `pre`.
fn section(tk: &Tk, label: &str, body: &str, plain: bool, error: bool) -> El {
    let head = text(label.to_uppercase())
        .font(tk.t3xs(500.0))
        .color(tk.muted_fg)
        .mb(2.0);
    let content = if plain {
        row()
            .w_full()
            .radius(tk.r(0.375))
            .fill(tk.muted.alpha(0.55))
            .px(tk.r(0.375))
            .py(2.0)
            .child(
                text(body.to_owned())
                    .font(tk.txs(400.0).lh(tk.r(0.75) * 1.375))
                    .color(tk.muted_fg)
                    .wrap(),
            )
    } else {
        super::nested_scroll(col().w_full().max_h(tk.r(if error { 8.0 } else { 7.0 })))
            .radius(tk.r(0.375))
            .fill(if error {
                tk.destructive.alpha(0.1)
            } else {
                tk.muted.alpha(0.55)
            })
            .px(tk.r(0.375))
            .py(2.0)
            .child(
                text(body.to_owned())
                    .font(tk.t3xs(400.0).mono())
                    .color(if error { tk.destructive } else { tk.muted_fg })
                    .wrap_anywhere(),
            )
    };
    col()
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
        .child(head)
        .child(content)
}

fn pretty(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        _ => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

/// The reverse call a finished change names (`data.undo`): only read to say it can be undone.
fn undoable(c: &ToolCall) -> bool {
    c.status == ToolStatus::Done
        && c.result
            .as_ref()
            .and_then(|r| r["data"]["undo"]["tool"].as_str())
            .is_some_and(|s| !s.is_empty())
}

/// `localToolReference`: a transaction the result points at.
pub fn local_reference(c: &ToolCall) -> Option<String> {
    let r = c.result.as_ref()?;
    let refv = &r["data"]["local_reference"];
    (refv["route"].as_str() == Some("/transactions"))
        .then(|| {
            refv["transaction"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .flatten()
}

/// A tool call's card: header (state icon, name, badge, chevron), the approval / undo line, the
/// open-transaction link, and — open — its arguments, summary or result, and reason or error.
pub fn card(app: &App, tk: &Tk, call: &ToolCall) -> El {
    if let Some(e) = super::approval::evidence_card(app, tk, call) {
        return e;
    }
    if let Some(e) = super::approval::wallet_request_card(app, tk, call) {
        return e;
    }
    let key = format!("asst-tool-{}", call.call_id);
    let open = is_open(app, &key, false);
    let err = call.status == ToolStatus::Error;
    let k2 = key.clone();
    let header = row()
        .key(format!("{key}-header"))
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .px(tk.r(0.5))
        .py(tk.r(0.25))
        .hover(tk.muted.alpha(0.45))
        .role("button")
        .label(format!("{} {}", call.name, state_label(call.status)))
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| toggle(a, &k2, false))
        .child(state_icon(tk, call.status))
        .child(
            text(call.name.clone())
                .font(tk.t2xs(400.0).mono())
                .color(tk.fg)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(badge(tk, state_label(call.status), err))
        .child(
            ic(
                if open {
                    &I::CHEVRON_UP
                } else {
                    &I::CHEVRON_DOWN
                },
                tk.r(0.75),
                tk.muted_fg,
            )
            .a11y_hidden(),
        );
    let mut c = col()
        .key(key.clone())
        .w_full()
        .min_w(0.0)
        .radius(tk.r(0.375))
        .border(
            1.0,
            if err {
                tk.destructive.alpha(0.35)
            } else {
                tk.border.alpha(tk.border.a * 0.6)
            },
        )
        .border_layout()
        .fill(if err {
            tk.destructive.alpha(0.05)
        } else {
            tk.muted.alpha(0.2)
        })
        .clip_rounded(tk.r(0.375))
        .child(header);
    let approval_key = match call.approval.as_deref() {
        Some("auto") => Some("assistant:tool.approval.auto"),
        Some("full_access") => Some("assistant:tool.approval.full_access"),
        Some("session") => Some("assistant:tool.approval.session"),
        _ => None,
    };
    let undo = undoable(call);
    if approval_key.is_some() || undo {
        let mut line = row()
            .w_full()
            .flex_wrap()
            .gap(tk.r(0.5))
            .center_y()
            .px(tk.r(0.5))
            .pb(tk.r(0.375));
        if let Some(k) = approval_key {
            let amber = if tk.dark {
                rune::color::hex("#fbbf24")
            } else {
                rune::color::hex("#d97706")
            }
            .unwrap_or(tk.warn.solid);
            line = line.child(
                row()
                    .gap(tk.r(0.25))
                    .center_y()
                    .radius(999.0)
                    .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                    .border_layout()
                    .fill(tk.background.alpha(0.6))
                    .px(tk.r(0.375))
                    .py(1.0)
                    .child(
                        ic(
                            &I::SHIELD_CHECK,
                            tk.r(0.75),
                            if call.approval.as_deref() == Some("full_access") {
                                amber
                            } else {
                                tk.muted_fg
                            },
                        )
                        .a11y_hidden(),
                    )
                    .child(text(t(k)).font(tk.t2xs(400.0)).color(tk.muted_fg)),
            );
        }
        if undo {
            line = line.child(
                row()
                    .gap(tk.r(0.25))
                    .center_y()
                    .child(ic(&I::UNDO_2, tk.r(0.75), tk.muted_fg).a11y_hidden())
                    .child(
                        text(t("assistant:tool.undoHint"))
                            .font(tk.t2xs(400.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        c = c.child(line);
    }
    if let Some(tx_id) = local_reference(call) {
        c = c.child(
            row()
                .key(format!("{key}-ref"))
                .mx(tk.r(0.25))
                .mb(tk.r(0.25))
                .h(tk.r(1.75))
                .px(tk.r(0.5))
                .gap(tk.r(0.375))
                .center_y()
                .radius(tk.r(0.375))
                .hover(if tk.dark {
                    tk.accent.alpha(0.5)
                } else {
                    tk.accent
                })
                .role("button")
                .label(t("assistant:tool.openReference"))
                .focusable(true)
                .on_press(move |a: &mut App| super::open_transaction(a, &tx_id))
                .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.875), tk.fg).a11y_hidden())
                .child(
                    text(t("assistant:tool.openReference"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                ),
        );
    }
    if open {
        if call.arguments.as_object().is_some_and(|o| !o.is_empty()) {
            c = c.child(section(
                tk,
                "Arguments",
                &pretty(&call.arguments),
                false,
                false,
            ));
        }
        let summary = summarize(call.result.as_ref());
        if let Some(s) = &summary {
            c = c.child(section(
                tk,
                &t("assistant:tool.summaryLabel"),
                s,
                true,
                false,
            ));
        } else if let Some(r) = call.result.as_ref().filter(|r| !r.is_null()) {
            c = c.child(section(
                tk,
                &t("assistant:tool.detailsLabel"),
                &pretty(r),
                r.is_string(),
                false,
            ));
        }
        match (&call.reason, call.status) {
            (Some(r), ToolStatus::Error | ToolStatus::Denied) => {
                c = c.child(section(tk, "Error", r, false, true))
            }
            (Some(r), _) => {
                c = c.child(section(
                    tk,
                    &t("assistant:tool.reasonLabel"),
                    r,
                    true,
                    false,
                ))
            }
            _ => {}
        }
    }
    c
}

fn arr_len(v: &Value, k: &str) -> usize {
    v[k].as_array().map_or(0, Vec::len)
}

fn n(v: &Value) -> usize {
    v.as_f64().map_or(0, |f| f.max(0.0) as usize)
}

/// `summarizeToolResult`: one line for the result kinds the web summarizes, else `None`.
pub fn summarize(result: Option<&Value>) -> Option<String> {
    let env = result?;
    let kind = env["kind"].as_str().filter(|k| !k.is_empty())?;
    let d = env.get("data").filter(|d| d.is_object())?;
    if d["status"].as_str() == Some("unavailable")
        && d["reason"]
            .as_str()
            .is_some_and(|r| r.starts_with("journals_"))
    {
        return Some(t("assistant:tool.journalsUnavailable"));
    }
    Some(match kind {
        "ui.review.cases" => tc(
            if d["next_cursor"].is_null() || d.get("next_cursor").is_none() {
                "assistant:review.cases.last"
            } else {
                "assistant:review.cases.more"
            },
            arr_len(d, "cases"),
            &[],
        ),
        "ui.review.plan" => {
            let a = super::approval::review_artifact(d)?;
            tc(
                "assistant:review.planSummary",
                arr_len(a, "operations"),
                &[
                    ("before", &n(&a["before"]["quarantine_count"]).to_string()),
                    ("after", &n(&a["after"]["quarantine_count"]).to_string()),
                ],
            )
        }
        "ui.review.apply" | "ui.review.receipt" => {
            let r = super::approval::review_receipt(d)?;
            tc(
                "assistant:review.receipt.verified",
                arr_len(r, "operations"),
                &[],
            )
        }
        "ui.overview.snapshot" => overview_summary(d),
        "ui.visuals.render" => {
            let source = d["visual"]["source"].as_str().unwrap_or("");
            let key = format!("assistant:visual.source.{source}");
            let label = if crate::i18n::has(&key) {
                t(&key)
            } else {
                source.to_owned()
            };
            tc(
                "assistant:tool.visual.summary",
                arr_len(d, "rows"),
                &[("source", &label)],
            )
        }
        "load_tools" => {
            let areas: Vec<&str> = d["areas"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            tc(
                "assistant:tool.loadTools.summary",
                arr_len(d, "added_tools"),
                &[("areas", &areas.join(", "))],
            )
        }
        "ui.workspace.health" => {
            let books_set = d["workspace"]["label"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("assistant:tool.workspaceHealth.noBooksSet"));
            let books = d["profile"]["label"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("assistant:tool.workspaceHealth.noBook"));
            let journals = d["journals"]["status"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("assistant:tool.workspaceHealth.statusUnknown"));
            let reports = if d["reports"]["ready"].as_bool() == Some(true) {
                t("assistant:tool.workspaceHealth.reportsReady")
            } else {
                t("assistant:tool.workspaceHealth.reportsNotReady")
            };
            tr(
                "assistant:tool.workspaceHealth.summary",
                &[
                    ("booksSet", &books_set),
                    ("books", &books),
                    ("journals", &journals),
                    ("reports", &reports),
                ],
            )
        }
        "ui.next_actions" => {
            let titles: Vec<&str> = d["suggestions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["title"].as_str())
                .collect();
            if titles.is_empty() {
                t("assistant:tool.nextActions.none")
            } else {
                tc(
                    "assistant:tool.nextActions.suggestions",
                    titles.len(),
                    &[("titles", &titles.join(", "))],
                )
            }
        }
        "ui.wallets.list" => {
            let wallets = tc(
                "assistant:tool.walletsList.wallets",
                arr_len(d, "wallets"),
                &[],
            );
            let labels: Vec<&str> = d["wallets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|w| w["label"].as_str())
                .take(4)
                .collect();
            if labels.is_empty() {
                tr(
                    "assistant:tool.walletsList.withoutLabels",
                    &[("wallets", &wallets)],
                )
            } else {
                tr(
                    "assistant:tool.walletsList.withLabels",
                    &[("wallets", &wallets), ("labels", &labels.join(", "))],
                )
            }
        }
        "ui.wallets.import_attachment.plan"
        | "ui.wallets.import_attachment.apply"
        | "ui.wallets.import_document.plan"
        | "ui.wallets.import_document.apply" => {
            let doc = kind.contains("document");
            let apply = kind.ends_with("apply");
            let rows = &d["rows"];
            let count = n(if doc && apply {
                &rows["imported"]
            } else {
                &rows["to_import"]
            });
            let target = if d["target"].is_object() {
                &d["target"]
            } else {
                &d["wallet"]
            };
            let wallet = target["label"].as_str().unwrap_or("—").to_owned();
            let base = if doc {
                "assistant:tool.importDocument"
            } else {
                "assistant:tool.importAttachment"
            };
            if apply {
                tc(&format!("{base}.applied"), count, &[("wallet", &wallet)])
            } else if d["ready"].as_bool() == Some(true) {
                tc(&format!("{base}.ready"), count, &[("wallet", &wallet)])
            } else {
                tr(&format!("{base}.blocked"), &[("wallet", &wallet)])
            }
        }
        "ui.wallets.read_document" => {
            let s = &d["summary"];
            tc(
                "assistant:tool.readDocument.summary",
                n(&s["rows"]),
                &[
                    ("ready", &n(&s["ready"]).to_string()),
                    ("held", &n(&s["quarantined"]).to_string()),
                ],
            )
        }
        "ui.imports.rollback" => tc(
            "assistant:tool.importRollback.done",
            n(&d["transactions_deleted"]),
            &[],
        ),
        "ui.backends.list" => {
            let backends = tc(
                "assistant:tool.backendsList.backends",
                arr_len(d, "backends"),
                &[],
            );
            match d["summary"]["default_backend"].as_str() {
                Some(def) => tr(
                    "assistant:tool.backendsList.withDefault",
                    &[("backends", &backends), ("default", def)],
                ),
                None => tr(
                    "assistant:tool.backendsList.withoutDefault",
                    &[("backends", &backends)],
                ),
            }
        }
        "ui.journals.quarantine" => tc(
            "assistant:tool.quarantine.summary",
            n(&d["summary"]["count"]),
            &[],
        ),
        "ui.journals.transfers.list" => {
            let entries = n(&d["summary"]["journal_transfer_entries"]);
            let pairs = tc(
                "assistant:tool.transfersList.pairs",
                n(&d["summary"]["manual_pairs"]),
                &[],
            );
            tc(
                "assistant:tool.transfersList.summary",
                entries,
                &[("pairs", &pairs), ("transferEntries", &entries.to_string())],
            )
        }
        "ui.rates.summary" => tc(
            "assistant:tool.ratesSummary.pairs",
            arr_len(d, "pairs"),
            &[],
        ),
        "ui.rates.coverage" => {
            let missing = n(&d["summary"]["missing_price_transactions"]);
            tc(
                "assistant:tool.ratesCoverage.summary",
                missing,
                &[
                    ("missing", &missing.to_string()),
                    (
                        "coverable",
                        &n(&d["summary"]["cache_coverable_missing"]).to_string(),
                    ),
                ],
            )
        }
        "ui.report.blockers" => {
            if d["ready"].as_bool() == Some(true) {
                t("assistant:tool.reportBlockers.ready")
            } else {
                tc(
                    "assistant:tool.reportBlockers.blockers",
                    arr_len(d, "blockers"),
                    &[],
                )
            }
        }
        "ui.audit.changes_since_last_answer" => {
            if d["status"].as_str() == Some("baseline_required") {
                t("assistant:tool.changesSince.baselineRequired")
            } else {
                let tx = n(&d["counts_since"]["transactions"]);
                let state = if d["changed"].as_bool() == Some(true) {
                    t("assistant:tool.changesSince.changed")
                } else {
                    t("assistant:tool.changesSince.unchanged")
                };
                tc(
                    "assistant:tool.changesSince.summary",
                    tx,
                    &[
                        ("state", &state),
                        ("txCount", &tx.to_string()),
                        (
                            "journalCount",
                            &n(&d["counts_since"]["journal_entries"]).to_string(),
                        ),
                    ],
                )
            }
        }
        "ui.maintenance.settings" | "ui.maintenance.configure" => {
            let base = if kind.ends_with("settings") {
                "assistant:tool.maintenanceSettings"
            } else {
                "assistant:tool.maintenanceConfigure"
            };
            t(&format!(
                "{base}.{}",
                if d["settings"]["auto_sync_before_report_reads"].as_bool() == Some(true) {
                    "enabled"
                } else {
                    "disabled"
                }
            ))
        }
        "ui.maintenance.run" => {
            if d["ready"].as_bool() == Some(true) {
                t("assistant:tool.maintenanceRun.ready")
            } else {
                tc(
                    "assistant:tool.maintenanceRun.blockers",
                    arr_len(d, "blockers"),
                    &[],
                )
            }
        }
        "ui.transactions.list" => tc(
            "assistant:tool.transactionsList.summary",
            arr_len(d, "txs"),
            &[],
        ),
        "ui.transactions.search" => tc(
            "assistant:tool.transactionsSearch.summary",
            arr_len(d, "txs"),
            &[],
        ),
        "ui.transactions.extremes" => tr(
            "assistant:tool.transactionsExtremes.summary",
            &[
                (
                    "largest",
                    &tc(
                        "assistant:tool.transactionsExtremes.largest",
                        arr_len(d, "largest"),
                        &[],
                    ),
                ),
                (
                    "smallest",
                    &tc(
                        "assistant:tool.transactionsExtremes.smallest",
                        arr_len(d, "smallest"),
                        &[],
                    ),
                ),
            ],
        ),
        "ui.reports.summary" => tr(
            "assistant:tool.reportsSummary.summary",
            &[
                (
                    "activeTransactions",
                    &tc(
                        "assistant:tool.reportsSummary.activeTransactions",
                        n(&d["metrics"]["active_transactions"]),
                        &[],
                    ),
                ),
                (
                    "assetFlowRows",
                    &tc(
                        "assistant:tool.reportsSummary.assetFlowRows",
                        arr_len(d, "asset_flow"),
                        &[],
                    ),
                ),
                (
                    "walletFlowRows",
                    &tc(
                        "assistant:tool.reportsSummary.walletFlowRows",
                        arr_len(d, "wallet_flow"),
                        &[],
                    ),
                ),
                (
                    "transferPairs",
                    &tc(
                        "assistant:tool.reportsSummary.transferPairs",
                        arr_len(d, "transfer_pairs"),
                        &[],
                    ),
                ),
            ],
        ),
        "ui.reports.balance_sheet" => tr(
            "assistant:tool.balanceSheet.summary",
            &[
                (
                    "balanceRows",
                    &tc(
                        "assistant:tool.balanceSheet.balanceRows",
                        arr_len(d, "rows"),
                        &[],
                    ),
                ),
                (
                    "assetTotals",
                    &tc(
                        "assistant:tool.balanceSheet.assetTotals",
                        arr_len(d, "totals_by_asset"),
                        &[],
                    ),
                ),
            ],
        ),
        "ui.reports.portfolio_summary" => tr(
            "assistant:tool.portfolioSummary.summary",
            &[
                (
                    "holdingRows",
                    &tc(
                        "assistant:tool.portfolioSummary.holdingRows",
                        arr_len(d, "rows"),
                        &[],
                    ),
                ),
                (
                    "assetTotals",
                    &tc(
                        "assistant:tool.portfolioSummary.assetTotals",
                        arr_len(d, "totals_by_asset"),
                        &[],
                    ),
                ),
            ],
        ),
        "ui.reports.tax_summary" => tr(
            "assistant:tool.taxSummary.summary",
            &[
                (
                    "taxRows",
                    &tc("assistant:tool.taxSummary.taxRows", arr_len(d, "rows"), &[]),
                ),
                (
                    "years",
                    &tc(
                        "assistant:tool.taxSummary.years",
                        arr_len(d, "available_years"),
                        &[],
                    ),
                ),
            ],
        ),
        "ui.reports.balance_history" => {
            let interval = d["filters"]["interval"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("assistant:tool.balanceHistory.intervalFallback"));
            tc(
                "assistant:tool.balanceHistory.buckets",
                arr_len(d, "rows"),
                &[("interval", &interval)],
            )
        }
        _ => return None,
    })
}

/// `overviewSnapshotSummary`.
fn overview_summary(d: &Value) -> String {
    let fiat = &d["fiat"];
    let comp = &fiat["completeness"];
    let basis_complete = comp["cost_basis_complete"]
        .as_bool()
        .or_else(|| comp["costBasisComplete"].as_bool())
        .unwrap_or(true);
    let state = comp["state"].as_str().unwrap_or("complete");
    let market_missing = comp["market_rate_missing"]
        .as_bool()
        .or_else(|| comp["marketRateMissing"].as_bool())
        .unwrap_or(false);
    let eur_missing = market_missing
        && fiat["fiatCurrency"]
            .as_str()
            .unwrap_or("EUR")
            .eq_ignore_ascii_case("EUR");
    let mut parts = vec![
        tc(
            "assistant:tool.overviewSnapshot.connections",
            arr_len(d, "connections"),
            &[],
        ),
        tc(
            "assistant:tool.overviewSnapshot.recentTransactions",
            arr_len(d, "txs"),
            &[],
        ),
    ];
    if let (Some(p), false) = (d["priceEur"].as_f64(), eur_missing) {
        // `priceEur` really is the BTC-EUR rate (the label reads `BTC/EUR`), whatever the book.
        parts.push(tr(
            "assistant:tool.overviewSnapshot.priceEur",
            &[("value", &crate::format::fiat(p, "EUR"))],
        ));
    } else if eur_missing {
        parts.push(t("assistant:tool.overviewSnapshot.noMarketRate"));
    }
    if let Some(r) = fiat["eurRealizedYTD"].as_f64() {
        let key = if basis_complete {
            "assistant:tool.overviewSnapshot.realizedYtd"
        } else {
            "assistant:tool.overviewSnapshot.realizedYtdIncomplete"
        };
        // Realized gain/loss is a sum of journal amounts, in the book's own currency.
        let book = fiat["fiatCurrency"]
            .as_str()
            .map(|c| c.trim().to_uppercase())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "EUR".into());
        parts.push(tr(key, &[("value", &crate::format::fiat(r, &book))]));
    }
    if !basis_complete {
        parts.push(t(if state == "stale" {
            "assistant:tool.overviewSnapshot.basisStale"
        } else {
            "assistant:tool.overviewSnapshot.basisIncomplete"
        }));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summaries_follow_the_web() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(
            summarize(Some(
                &json!({"kind": "ui.journals.quarantine", "data": {"summary": {"count": 3}}})
            ))
            .as_deref(),
            Some("3 quarantined transactions.")
        );
        assert_eq!(
            summarize(Some(
                &json!({"kind": "ui.report.blockers", "data": {"ready": true}})
            ))
            .as_deref(),
            Some("Reports are ready.")
        );
        assert_eq!(summarize(Some(&json!({"kind": "ui.rates.coverage", "data": {"status": "unavailable", "reason": "journals_stale"}}))).as_deref(), Some("Figures withheld: journals need processing first."));
        assert_eq!(
            summarize(Some(&json!({"kind": "something.else", "data": {}}))),
            None
        );
        assert_eq!(summarize(Some(&json!("text"))), None);
    }

    /// A USD book's realized YTD reads in dollars; the BTC/EUR price stays the EUR quote.
    #[test]
    fn overview_summary_uses_the_book_currency_for_realized() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let d = json!({"priceEur": 80_000.0, "fiat": {"fiatCurrency": "USD", "eurRealizedYTD": 1234.5}});
        let s = overview_summary(&d);
        assert!(s.contains("realized YTD $1,234.50"), "{s}");
        assert!(s.contains("BTC/EUR €\u{a0}80.000,00"), "{s}");
    }

    #[test]
    fn undo_and_reference_are_read_only_hints() {
        let mut c = ToolCall {
            call_id: "c".into(),
            name: "ui.transactions.metadata.update".into(),
            arguments: json!({}),
            kind_class: "mutating".into(),
            needs_consent: false,
            status: ToolStatus::Done,
            approval: Some("auto".into()),
            result: Some(
                json!({"kind": "x", "data": {"undo": {"tool": "ui.transactions.metadata.update", "arguments": {}}, "local_reference": {"route": "/transactions", "transaction": "t1"}}}),
            ),
            reason: None,
            review_preview: None,
            segment_id: None,
        };
        assert!(undoable(&c));
        assert_eq!(local_reference(&c).as_deref(), Some("t1"));
        c.status = ToolStatus::Error;
        assert!(!undoable(&c));
    }
}
