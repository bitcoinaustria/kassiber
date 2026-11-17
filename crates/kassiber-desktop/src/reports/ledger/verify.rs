//! The journal reproducibility check (components/kb/JournalVerifyDialog.tsx,
//! docs/reference/replay-bundle.md): `ui.journals.verify {limit: 20}` recomputes the journals in
//! a private in-memory copy of the book and compares them with the stored ones. It runs only
//! when the user opens the dialog (Check reproducibility), reads the book, writes nothing and
//! contacts no network.

use rune::{Color, El, Shadow, col, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, icon_button, spinner};

pub const LIMIT: u64 = 20;
pub const TAG: &str = "reports.ledger.verify";

#[derive(Default)]
pub struct Verify {
    pub open: bool,
    /// Requests sent and not yet answered: only the newest opening's answer is shown.
    inflight: u32,
    pub report: Option<Value>,
    pub error: Option<String>,
}

impl Verify {
    pub fn pending(&self) -> bool {
        self.open && self.inflight > 0
    }
}

/// Opening the dialog runs the check (the web's `useEffect` on `open`).
pub fn open(app: &mut App) {
    let v = &mut app.reports.ledger.journals.verify;
    v.open = true;
    v.report = None;
    v.error = None;
    if app
        .send("ui.journals.verify", json!({"limit": LIMIT}), TAG)
        .is_some()
    {
        app.reports.ledger.journals.verify.inflight += 1;
    }
}

/// Closing resets the mutation; a late answer is dropped.
pub fn close(app: &mut App) {
    let v = &mut app.reports.ledger.journals.verify;
    v.open = false;
    v.report = None;
    v.error = None;
}

pub fn on_reply(app: &mut App, r: &Reply) {
    let v = &mut app.reports.ledger.journals.verify;
    v.inflight = v.inflight.saturating_sub(1);
    if !v.open || v.inflight > 0 {
        return;
    }
    match &r.error {
        Some(e) => v.error = Some(e.clone()),
        None => v.report = Some(r.data.clone()),
    }
}

/// `VerifyReport`'s headline.
pub fn headline(report: &Value) -> String {
    match report.get("status").and_then(Value::as_str).unwrap_or("") {
        "identical" => t("journals:ledger.verify.status.identical"),
        "different" => tc(
            "journals:ledger.verify.status.different",
            report.get("differing").and_then(Value::as_u64).unwrap_or(0) as usize,
            &[],
        ),
        "stale" | "not_processed" => t("journals:ledger.verify.status.stale"),
        "no_transactions" => t("journals:ledger.verify.status.noTransactions"),
        _ => tr(
            "journals:ledger.verify.status.refused",
            &[(
                "message",
                report
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )],
        ),
    }
}

/// `name (count), …` of a `by_wallet` / `by_year` object.
pub fn groups(v: Option<&Value>) -> String {
    v.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, n)| {
                    format!(
                        "{k} ({})",
                        n.as_u64()
                            .map(|x| x.to_string())
                            .unwrap_or_else(|| n.to_string())
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// One difference line: `table · change · key · columns`.
pub fn difference_line(d: &Value) -> String {
    let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("");
    let mut line = format!("{} · {} · {}", s("table"), s("change"), s("key"));
    let cols: Vec<&str> = d
        .get("columns")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| c.get("column").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if !cols.is_empty() {
        line.push_str(&format!(" · {}", cols.join(", ")));
    }
    line
}

fn muted(tk: &Tk, s: String) -> El {
    text(s)
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .wrap()
        .w_full()
}

fn report_view(tk: &Tk, report: &Value) -> Vec<El> {
    let mut out = vec![
        text(headline(report))
            .font(tk.tsm(500.0))
            .color(tk.fg)
            .wrap()
            .w_full(),
    ];
    let different = report.get("status").and_then(Value::as_str) == Some("different");
    if !different {
        return out;
    }
    if report.get("deterministic").and_then(Value::as_bool) == Some(false) {
        out.push(muted(tk, t("journals:ledger.verify.nondeterministic")));
    }
    let wallets = groups(report.get("by_wallet"));
    if !wallets.is_empty() {
        out.push(
            text(tr(
                "journals:ledger.verify.byWallet",
                &[("wallets", &wallets)],
            ))
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap()
            .w_full(),
        );
    }
    let years = groups(report.get("by_year"));
    if !years.is_empty() {
        out.push(
            text(tr("journals:ledger.verify.byYear", &[("years", &years)]))
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap()
                .w_full(),
        );
    }
    let diffs = report
        .get("differences")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut list = col().w_full().gap(tk.r(0.25)).role("list");
    for d in &diffs {
        list = list.child(
            text(difference_line(d))
                .font(tk.txs(400.0).mono())
                .color(tk.fg)
                .wrap_anywhere()
                .w_full()
                .role("listitem"),
        );
    }
    out.push(list);
    if report.get("truncated").and_then(Value::as_bool) == Some(true) {
        out.push(muted(
            tk,
            tr(
                "journals:ledger.verify.truncated",
                &[("count", &diffs.len().to_string())],
            ),
        ));
    }
    out.push(muted(tk, t("journals:ledger.verify.nextStep")));
    out
}

/// The dialog (`DialogContent className="max-w-2xl"`).
pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let v = &app.reports.ledger.journals.verify;
    if !v.open {
        return;
    }
    let key = "lg-verify";
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    layer.push(
        crate::shell::window_backdrop(format!("{key}-scrim"), wash)
            .on_press(close)
            .role("none"),
    );
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
    let top = crate::shell::dialog_top(tk, win_h);
    let width = (win_w - tk.r(2.0)).min(tk.r(42.0));
    let (border, shadows) = if tk.dark {
        (
            Color::WHITE.alpha(0.08),
            vec![
                Shadow {
                    y: 1.0,
                    color: Color::WHITE.alpha(0.04),
                    inset: true,
                    ..Default::default()
                },
                Shadow {
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
            vec![Shadow {
                y: 24.0,
                blur: 64.0,
                spread: -24.0,
                color: Color::BLACK.alpha(0.65),
                ..Default::default()
            }],
        )
    };
    let header = col()
        .w_full()
        .gap(tk.r(0.5))
        .pr(tk.r(1.5))
        .child(
            text(t("journals:ledger.verify.title"))
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .role("heading")
                .level(2),
        )
        .child(
            text(t("journals:ledger.verify.description"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
    // `max-h-[60vh] space-y-3 overflow-y-auto text-sm`, a live status region.
    let mut status = col()
        .key(format!("{key}-status"))
        .w_full()
        .gap(tk.r(0.75))
        .role("status");
    if v.pending() {
        status = status.child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(spinner(tk, tk.r(1.0)).a11y_hidden())
                .child(
                    text(t("journals:ledger.verify.running"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    if let Some(e) = &v.error {
        status = status.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .w_full(),
        );
    }
    if let Some(r) = &v.report {
        status = status.children(report_view(tk, r));
    }
    let status = col()
        .key(format!("{key}-body"))
        .w_full()
        .max_h(win_h * 0.6)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(status);
    let footer = row().w_full().justify_end().child(
        button(
            tk,
            format!("{key}-done"),
            Variant::Default,
            Size::Default,
            None,
            &t("common:actions.close"),
            true,
        )
        .on_press(close),
    );
    let close_btn = icon_button(
        tk,
        format!("{key}-close"),
        Variant::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None)
    .on_press(close);
    let max_h = win_h - top - tk.r(1.0);
    let body = col()
        .key(format!("{key}-scroll"))
        .w_full()
        .max_h(max_h - 2.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .padding(tk.r(1.25))
                .gap(tk.r(1.0))
                .child(header)
                .child(status)
                .child(footer),
        );
    let mut surface = col()
        .key(key.to_owned())
        .role("dialog")
        .label(t("journals:ledger.verify.title"))
        .description(t("journals:ledger.verify.description"))
        .w(width)
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(6.0)
        .transition(crate::shell::dialog_enter())
        .border(1.0, border)
        .border_layout()
        .clip_rounded(tk.r(1.0))
        .modal(true, close)
        .focus_return("lg-verify-open".to_owned())
        .child(body)
        .child(close_btn);
    for sh in shadows {
        surface = surface.box_shadow(sh);
    }
    layer.push(crate::shell::centered(surface, win_w, width, top));
}

/// Scene: the dialog with a report as the daemon would return it (synthetic).
pub fn scene_report(app: &mut App, report: Value) {
    let v = &mut app.reports.ledger.journals.verify;
    v.open = true;
    v.inflight = 0;
    v.error = None;
    v.report = Some(report);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headlines_follow_the_status() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(
            headline(&json!({"status": "identical"})),
            "The stored journals are exactly what the current inputs produce."
        );
        assert_eq!(
            headline(&json!({"status": "different", "differing": 1})),
            "1 stored row differs from a recomputation."
        );
        assert_eq!(
            headline(&json!({"status": "different", "differing": 3})),
            "3 stored rows differ from a recomputation."
        );
        assert_eq!(
            headline(&json!({"status": "not_processed"})),
            headline(&json!({"status": "stale"}))
        );
        assert_eq!(
            headline(&json!({"status": "refused", "error": {"message": "locked"}})),
            "Recomputing the journals was refused: locked"
        );
    }

    #[test]
    fn difference_lines_and_groups() {
        let d = json!({"table": "journal_entries", "change": "changed", "key": "tx1:0", "columns": [{"column": "fiat_value"}, {"column": "gain"}]});
        assert_eq!(
            difference_line(&d),
            "journal_entries · changed · tx1:0 · fiat_value, gain"
        );
        assert_eq!(
            difference_line(&json!({"table": "t", "change": "missing", "key": "k"})),
            "t · missing · k"
        );
        assert_eq!(
            groups(Some(&json!({"2024": 2, "2025": 1}))),
            "2024 (2), 2025 (1)"
        );
        assert_eq!(groups(None), "");
    }
}
