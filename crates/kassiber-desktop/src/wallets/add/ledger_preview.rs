//! components/kb/GenericLedgerPreview.tsx: the read-only preview of a generic-ledger import in
//! the Add connection dialog.
//!
//! On a chosen file it reads `ui.wallets.ledger_preview` (`{filename, source_bytes_base64}`, a
//! local parse: nothing is persisted, nothing leaves the machine) and shows what would import:
//! the "N of M rows ready" badge, an error-count badge, the recognised columns, up to five
//! normalised rows (date, type, amount, value; amounts blurred by the privacy modes), and the
//! first three row-numbered problems. Unrecognised columns steer to the template instead.
//! `blocks_submit` is the web's `onBlockSubmitChange` value: the dialog's import button stays
//! disabled until the preview is confident, error-free and has rows.

use std::collections::VecDeque;

use rune::{Color, El, col, el, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::{Lang, lang, t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, sensitive};

/// The file the preview reads (`GenericLedgerPreviewSource`).
#[derive(Clone, Debug)]
pub struct Source {
    pub filename: String,
    pub bytes_b64: String,
    /// False for a browser-picked file the daemon cannot read by path: previewable, not importable.
    pub importable: bool,
}

#[derive(Default)]
pub struct State {
    pub source: Option<Source>,
    /// Bumped on every new source; a reply for an older one is dropped (the query key changed).
    generation: u64,
    /// Generations of the requests in flight, in send order.
    in_flight: VecDeque<u64>,
    pub data: Option<Value>,
    pub error: Option<String>,
}

impl State {
    fn fetching(&self) -> bool {
        self.in_flight.contains(&self.generation)
    }
}

/// The dialog opens afresh: previews still out keep their entries and the counter its count,
/// so their answers are dropped instead of taken for the next file's.
pub fn restart(old: State) -> State {
    State {
        generation: old.generation,
        in_flight: old.in_flight,
        ..State::default()
    }
}

/// Stores `source` and starts its preview read (None clears it).
pub fn set_source(app: &mut App, source: Option<Source>) {
    let st = &mut app.wallets.add.ledger;
    st.generation += 1;
    st.data = None;
    st.error = None;
    st.source = source.clone();
    let Some(src) = source else { return };
    let generation = st.generation;
    let args = json!({ "filename": src.filename, "source_bytes_base64": src.bytes_b64 });
    if app
        .send("ui.wallets.ledger_preview", args, "wallets.ledger.preview")
        .is_some()
    {
        app.wallets.add.ledger.in_flight.push_back(generation);
    }
}

/// `blocksSubmit`: not importable, still reading, failed, no data, unrecognised columns, any
/// row error, or nothing mapped.
pub fn blocks_submit(app: &App) -> bool {
    let st = &app.wallets.add.ledger;
    let Some(src) = &st.source else { return false };
    let Some(d) = st.data.as_ref() else {
        return true;
    };
    !src.importable
        || st.fetching()
        || st.error.is_some()
        || d.get("confident").and_then(Value::as_bool) == Some(false)
        || num(d, "errors") > 0.0
        || num(d, "mapped") <= 0.0
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag != "wallets.ledger.preview" {
        return;
    }
    let st = &mut app.wallets.add.ledger;
    let Some(generation) = st.in_flight.pop_front() else {
        return;
    };
    if generation != st.generation {
        return;
    }
    match &r.error {
        Some(e) => {
            st.error = Some(e.clone());
            st.data = None;
        }
        None => {
            st.error = None;
            st.data = Some(r.data.clone());
        }
    }
}

/// Headless runs: wait for the preview read.
pub fn prime(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.wallets.add.ledger.fetching() && std::time::Instant::now() < deadline {
        app.pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty())
}

/// `formatUiNumber(num, {maximumFractionDigits: 8})`: en `1,234.015`, de-AT `1 234,015`.
fn format_number(v: f64, lang: Lang) -> String {
    let s = format!("{:.8}", v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let frac = frac.trim_end_matches('0');
    let (group, dec) = match lang {
        Lang::En => (',', '.'),
        Lang::De => ('\u{a0}', ','),
    };
    let mut g = String::new();
    // de-AT groups plain numbers only from five digits (`1234`, `12 345`).
    let min = if lang == Lang::De { 5 } else { 4 };
    for (i, ch) in int.chars().enumerate() {
        if int.len() >= min && i > 0 && (int.len() - i) % 3 == 0 {
            g.push(group);
        }
        g.push(ch);
    }
    let sign = if v < 0.0 && (int != "0" || !frac.is_empty()) {
        "-"
    } else {
        ""
    };
    if frac.is_empty() {
        format!("{sign}{g}")
    } else {
        format!("{sign}{g}{dec}{frac}")
    }
}

/// `formatAssetAmount`: a number with its asset (BTC when absent), or the raw text.
fn format_asset_amount(value: Option<&str>, asset: Option<&str>, lang: Lang) -> String {
    let Some(v) = value else { return String::new() };
    match v.trim().parse::<f64>() {
        Ok(n) if n.is_finite() => format!("{} {}", format_number(n, lang), asset.unwrap_or("BTC")),
        _ => v.to_owned(),
    }
}

/// `formatDate`: `{year: numeric, month: short, day: numeric}` in the UI locale, or the raw text.
fn format_date(iso: Option<&str>) -> String {
    let Some(v) = iso else { return String::new() };
    match crate::format::Day::parse(v) {
        Some(d) => d.long_ui(),
        None => v.to_owned(),
    }
}

/// Badge (`rounded-full px-2 py-0.5 text-xs font-medium`), filled.
fn badge(tk: &Tk, label: String, fill: Color, ink: Color) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.75))
        .fill(fill)
        .center_y()
        .child(text(label).font(tk.txs(500.0)).color(ink))
}

pub fn view(app: &App, tk: &Tk) -> Option<El> {
    let st = &app.wallets.add.ledger;
    st.source.as_ref()?;
    let fetching = st.fetching();
    if fetching && st.data.is_none() {
        // `flex items-center gap-2 rounded-md border border-dashed px-3 py-2 text-xs`.
        return Some(
            row()
                .key("ledger-preview-loading")
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .busy(true)
                .child(
                    ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.muted_fg)
                        .looping(rune::LoopSpec::rotation(0.9))
                        .a11y_hidden(),
                )
                .child(
                    text(t("connections:add.genericLedger.preview.loading"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    if let Some(e) = &st.error {
        let msg = if e.trim().is_empty() {
            t("connections:add.genericLedger.preview.failed")
        } else {
            e.clone()
        };
        return Some(
            row()
                .key("ledger-preview-error")
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.375))
                .border(1.0, tk.destructive.alpha(0.4))
                .border_layout()
                .fill(tk.destructive.alpha(0.05))
                .role("alert")
                .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), tk.destructive).a11y_hidden())
                .child(
                    text(msg)
                        .font(tk.txs(400.0))
                        .color(tk.destructive)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    let d = st.data.as_ref()?;
    if d.get("confident").and_then(Value::as_bool) == Some(false) {
        // `border-amber-500/40 bg-amber-500/5 text-amber-700 dark:text-amber-400`.
        let amber = rune::color::hex("#f59e0b").expect("hex");
        let ink = rune::color::hex(if tk.dark { "#fbbf24" } else { "#b45309" }).expect("hex");
        return Some(
            col()
                .key("ledger-preview-unrecognized")
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.25))
                .radius(tk.r(0.375))
                .border(1.0, amber.alpha(0.4))
                .border_layout()
                .fill(amber.alpha(0.05))
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), ink).a11y_hidden())
                        .child(
                            text(t("connections:add.genericLedger.preview.notRecognized"))
                                .font(tk.txs(500.0))
                                .color(ink),
                        ),
                )
                .child(
                    text(t("connections:add.genericLedger.preview.notRecognizedHint"))
                        .font(tk.txs(400.0))
                        .color(ink)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    let lang = lang();
    let hidden = app.prefs.hide_sensitive();
    let mapped = num(d, "mapped") as usize;
    let rows_read = num(d, "rows_read") as usize;
    let errors = num(d, "errors") as usize;
    let mut badges = row()
        .w_full()
        .flex_wrap()
        .gap(tk.r(0.5))
        .center_y()
        .child(badge(
            tk,
            tr(
                "connections:add.genericLedger.preview.ready",
                &[
                    ("mapped", &mapped.to_string()),
                    ("rows", &rows_read.to_string()),
                ],
            ),
            tk.muted,
            tk.fg,
        ));
    if errors > 0 {
        let fill = if tk.dark {
            tk.destructive.alpha(0.6)
        } else {
            tk.destructive
        };
        badges = badges.child(badge(
            tk,
            tc(
                "connections:add.genericLedger.preview.errorCount",
                errors,
                &[],
            ),
            fill,
            Color::WHITE,
        ));
    }
    let mut out = col()
        .key("ledger-preview")
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .child(badges);
    let detected: Vec<&str> = d
        .get("detected")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("column").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if !detected.is_empty() {
        out = out.child(
            text(tr(
                "connections:add.genericLedger.preview.detected",
                &[("columns", &detected.join(", "))],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap()
            .w_full(),
        );
    }
    let rows: Vec<&Value> = d
        .get("preview")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(5).collect())
        .unwrap_or_default();
    if !rows.is_empty() {
        out = out.child(table(tk, &rows, lang, hidden));
    }
    let problems: Vec<&Value> = d
        .get("problems")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    if !problems.is_empty() {
        let mut list = col().w_full().gap(tk.r(0.125)).role("list");
        for p in problems.iter().take(3) {
            let row_no = p
                .get("row")
                .map(|r| {
                    r.as_f64()
                        .map(|n| (n as i64).to_string())
                        .unwrap_or_else(|| r.to_string())
                })
                .unwrap_or_default();
            let message = s(p, "message").unwrap_or("");
            list = list.child(
                text(tr(
                    "connections:add.genericLedger.preview.problemRow",
                    &[("row", &row_no), ("message", message)],
                ))
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .wrap()
                .w_full()
                .role("listitem"),
            );
        }
        if problems.len() > 3 {
            list = list.child(
                text(tc(
                    "connections:add.genericLedger.preview.more",
                    problems.len() - 3,
                    &[],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .role("listitem"),
            );
        }
        out = out.child(list);
    }
    Some(out)
}

/// The rows table (`ScrollArea max-h-44` around a shadcn `Table`, every cell `text-xs`).
fn table(tk: &Tk, rows: &[&Value], lang: Lang, hidden: bool) -> El {
    // Auto table layout: each column takes a share roughly in proportion to its content.
    const SHARE: [f32; 4] = [1.15, 0.8, 1.25, 0.9];
    let head_cell = |label: String, i: usize, right: bool| {
        let mut c = row()
            .grow(SHARE[i])
            .min_w(0.0)
            .h(tk.r(2.5))
            .px(tk.r(0.5))
            .center_y()
            .role("columnheader");
        if right {
            c = c.justify_end();
        }
        c.child(text(label).font(tk.txs(500.0)).color(tk.fg))
    };
    let head = row()
        .w_full()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .role("row")
        .child(head_cell(
            t("connections:add.genericLedger.preview.colDate"),
            0,
            false,
        ))
        .child(head_cell(
            t("connections:add.genericLedger.preview.colKind"),
            1,
            false,
        ))
        .child(head_cell(
            t("connections:add.genericLedger.preview.colAmount"),
            2,
            true,
        ))
        .child(head_cell(
            t("connections:add.genericLedger.preview.colValue"),
            3,
            true,
        ));
    let cell = |i: usize, right: bool, content: El| {
        let mut c = row()
            .grow(SHARE[i])
            .min_w(0.0)
            .padding(tk.r(0.5))
            .center_y()
            .role("cell");
        if right {
            c = c.justify_end();
        }
        c.child(content)
    };
    let mut body = col().w_full().role("rowgroup");
    let n = rows.len();
    for (idx, r) in rows.iter().enumerate() {
        let kind = s(r, "kind")
            .or_else(|| s(r, "direction"))
            .unwrap_or("")
            .to_owned();
        let amount = format_asset_amount(s(r, "amount"), s(r, "asset"), lang);
        let value = match s(r, "fiat_value") {
            Some(v) => match s(r, "fiat_currency") {
                Some(c) => format!("{v} {c}"),
                None => v.to_owned(),
            },
            None => String::new(),
        };
        let mut line = row()
            .key(format!("ledger-preview-row-{idx}"))
            .w_full()
            .hover(tk.muted.alpha(0.5))
            .role("row")
            .child(cell(
                0,
                false,
                text(format_date(s(r, "occurred_at")))
                    .font(tk.txs(400.0))
                    .color(tk.fg)
                    .truncate(),
            ))
            .child(cell(
                1,
                false,
                text(kind).font(tk.txs(400.0)).color(tk.muted_fg).truncate(),
            ))
            .child(cell(
                2,
                true,
                sensitive(amount, tk.txs(400.0).mono(), tk.fg, hidden).truncate(),
            ))
            .child(cell(
                3,
                true,
                if value.is_empty() {
                    el()
                } else {
                    sensitive(value, tk.txs(400.0), tk.muted_fg, hidden).truncate()
                },
            ));
        if idx + 1 < n {
            line = line
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout();
        }
        body = body.child(line);
    }
    col()
        .key("ledger-preview-table")
        .w_full()
        .max_h(tk.r(11.0))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(col().w_full().role("table").child(head).child(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file's preview is still out when the dialog is closed and opened on another file: the
    /// old preview never shows for the new file, whose own still lands.
    #[test]
    fn a_preview_from_before_reopening_is_dropped() {
        let mut app = crate::app::tests::quiet_app();
        let file = |name: &str| Source {
            filename: name.into(),
            bytes_b64: "AA==".into(),
            importable: true,
        };
        let reply = |rows: u64| Reply {
            kind: "ui.wallets.ledger_preview".into(),
            data: json!({ "rows": rows }),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        crate::wallets::add::open(&mut app, Some("generic-ledger"));
        set_source(&mut app, Some(file("a.csv")));
        crate::wallets::add::open(&mut app, Some("generic-ledger"));
        set_source(&mut app, Some(file("b.csv")));
        on_reply(&mut app, "wallets.ledger.preview", &reply(1));
        assert!(app.wallets.add.ledger.data.is_none(), "a.csv's preview");
        assert!(app.wallets.add.ledger.fetching());
        on_reply(&mut app, "wallets.ledger.preview", &reply(2));
        assert_eq!(app.wallets.add.ledger.data, Some(json!({ "rows": 2 })));
    }

    #[test]
    fn numbers_trim_and_group_per_locale() {
        assert_eq!(format_number(0.015, Lang::En), "0.015");
        assert_eq!(format_number(0.015, Lang::De), "0,015");
        assert_eq!(format_number(1234.5, Lang::En), "1,234.5");
        assert_eq!(format_number(1234.5, Lang::De), "1234,5");
        assert_eq!(format_number(12345.0, Lang::De), "12\u{a0}345");
    }

    #[test]
    fn amounts_fall_back_to_btc_and_raw_text() {
        assert_eq!(
            format_asset_amount(Some("0.01500000"), None, Lang::En),
            "0.015 BTC"
        );
        assert_eq!(
            format_asset_amount(Some("abc"), Some("BTC"), Lang::En),
            "abc"
        );
        assert_eq!(format_asset_amount(None, None, Lang::En), "");
    }
}
