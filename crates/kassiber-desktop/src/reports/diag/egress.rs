//! The network monitor (routes/Egress.tsx): the daemon's RAM-only outbound ledger,
//! `ui.egress.snapshot {limit: 1000}`, read on mount, every 4 s while open, and on "Refresh".
//! The snapshot is a local read of what the daemon already sent; nothing here makes egress.

use std::collections::{HashSet, VecDeque};

use rune::{Cx, El, TextEdit, Tx, col, el, row, text};
use serde_json::{Value, json};

use super::widgets::{self, BadgeVariant};
use crate::app::{App, Reply};
use crate::i18n::{has, t, tr};
use crate::icons as I;
use crate::theme::{Tk, Tone};
use crate::txdetail::widgets::Variant;
use crate::ui::ic;

const INTERVAL: f64 = 4.0;

#[derive(Default)]
pub struct State {
    pub snapshot: Option<Value>,
    pub error: Option<String>,
    pub query: TextEdit,
    pub expanded: HashSet<u64>,
    in_flight: VecDeque<()>,
    next_at: f64,
    mounted: bool,
}

impl State {
    pub fn unmount(&mut self) {
        self.mounted = false;
    }

    /// Leaving `/egress`: the search and expanded records are the route's `useState`s.
    pub fn leave(&mut self) {
        self.query = TextEdit::new("");
        self.expanded.clear();
        self.unmount();
    }
}

fn fetch(app: &mut App) {
    app.reports.diag.egress.in_flight.push_back(());
    app.reports.diag.egress.next_at = app.clock + INTERVAL;
    app.send(
        "ui.egress.snapshot",
        json!({"limit": 1000}),
        "reports.diag.egress.snapshot",
    );
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    // Behind the lock nothing is read; the unlock's redraw polls again.
    if app.locked {
        return false;
    }
    let e = &mut app.reports.diag.egress;
    let due = !e.mounted || (app.clock >= e.next_at && e.in_flight.is_empty());
    e.mounted = true;
    if due {
        fetch(app);
    }
    let wait = (app.reports.diag.egress.next_at - app.clock).clamp(0.05, 3600.0);
    tx.wake_after(std::time::Duration::from_secs_f64(wait));
    due
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag != "reports.diag.egress.snapshot" {
        return;
    }
    let e = &mut app.reports.diag.egress;
    e.in_flight.pop_front();
    if r.error_detail["code"] == crate::app::LOCKED_CODE {
        // Refused behind the lock: no error to show, the next poll runs after the unlock.
        e.next_at = 0.0;
        return;
    }
    match &r.error {
        Some(err) => e.error = Some(err.clone()),
        None => {
            e.error = None;
            e.snapshot = Some(r.data.clone());
        }
    }
}

pub fn prime(app: &mut App) {
    app.reports.diag.egress.mounted = true;
    fetch(app);
    super::super::pump_until(app, |a| a.reports.diag.egress.in_flight.is_empty());
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn destination(r: &Value) -> String {
    match r["port"].as_u64().filter(|p| *p != 0) {
        Some(p) => format!("{}:{p}", s(r, "host")),
        None => s(r, "host").to_owned(),
    }
}

fn subsystem(name: &str) -> String {
    let k = format!("review:egress.subsystem.{name}");
    if has(&k) { t(&k) } else { name.to_owned() }
}

fn format_bytes(v: &Value) -> String {
    let b = v.as_f64().unwrap_or(0.0);
    if !b.is_finite() || b <= 0.0 {
        "0 B".into()
    } else {
        super::logstore::format_bytes(b as usize)
    }
}

fn db_label(db: &Value) -> String {
    if db["sqlite_plaintext_header"].as_bool() == Some(true) {
        t("review:egress.db.plaintext")
    } else if db["encrypted_like"].as_bool() == Some(true) {
        t("review:egress.db.ciphertextLike")
    } else if s(db, "classification") == "missing" {
        t("review:egress.db.missing")
    } else if s(db, "classification") == "unreadable" {
        t("review:egress.db.unreadable")
    } else {
        t("review:egress.db.unknown")
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Ink {
    Good,
    Bad,
    Neutral,
}

fn db_tone(db: &Value) -> Ink {
    if db["sqlite_plaintext_header"].as_bool() == Some(true) {
        Ink::Bad
    } else if db["encrypted_like"].as_bool() == Some(true) {
        Ink::Good
    } else {
        Ink::Neutral
    }
}

fn emerald(tk: &Tk) -> rune::Color {
    rune::color::hex(if tk.dark { "#6ee7b7" } else { "#047857" }).unwrap_or(tk.fg)
}

/// `DetailRow`: `grid grid-cols-[120px_1fr] gap-3`, the value optionally mono and toned.
fn detail(tk: &Tk, label: String, value: String, tone: Ink, mono: bool) -> El {
    let ink = match tone {
        Ink::Good => emerald(tk),
        Ink::Bad => tk.destructive,
        Ink::Neutral => tk.fg,
    };
    let style = if mono {
        tk.txs(400.0).mono()
    } else {
        tk.tsm(400.0)
    };
    row()
        .w_full()
        .gap(tk.r(0.75))
        .child(
            text(label)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .w(120.0)
                .wrap(),
        )
        .child(
            text(value)
                .font(style)
                .color(ink)
                .grow(1.0)
                .min_w(0.0)
                .wrap_anywhere()
                .wrap(),
        )
}

fn status_badge(tk: &Tk, r: &Value) -> El {
    match s(r, "allowlist_status") {
        "unexpected" => widgets::tone_badge(
            tk,
            &widgets::destructive_tone(tk),
            Some((&I::TRIANGLE_ALERT, tk.destructive)),
            &t("review:egress.status.unexpected"),
            false,
        ),
        "unknown" => widgets::badge(
            tk,
            BadgeVariant::Outline,
            &t("review:egress.status.unknown"),
        ),
        _ => {
            let label = if r["user_allowlisted"].as_bool() == Some(true) {
                t("review:egress.status.user")
            } else {
                t("review:egress.status.builtIn")
            };
            let tone = Tone {
                bg: rune::Color::TRANSPARENT,
                border: tk.border,
                text: tk.fg,
                solid: tk.fg,
            };
            widgets::tone_badge(
                tk,
                &tone,
                Some((&I::CIRCLE_CHECK, rune::color::hex("#059669").unwrap())),
                &label,
                false,
            )
        }
    }
}

fn allowlist_detail(r: &Value) -> String {
    let status = match s(r, "allowlist_status") {
        "expected" => {
            if r["user_allowlisted"].as_bool() == Some(true) {
                t("review:egress.status.user")
            } else {
                t("review:egress.status.builtIn")
            }
        }
        other => t(&format!("review:egress.status.{other}")),
    };
    [
        status,
        s(r, "allowlist_label").to_owned(),
        s(r, "allowlist_source").to_owned(),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// A cell `w` of the table's width; the first and last keep the kit's edge inset (the card
/// head's) so the columns line up with it.
fn cell(i: usize, w: f32, total: f32, e: El) -> El {
    let edge = 24.0;
    row()
        .w(w * total)
        .min_w(0.0)
        .pl(if i == 0 { edge } else { 10.8 })
        .pr(if i == 5 { edge } else { 10.8 })
        .py(7.2)
        .center_y()
        .child(e)
}

/// The auto table layout: each column's max-content width (measured: the kit's `text-sm`
/// header labels, the cells' mono and badge text) as fractions of their sum, and that sum (the
/// table's least width before it scrolls sideways).
fn columns(tk: &Tk, rows: &[&Value]) -> ([f32; 6], f32) {
    let m = |s: &str, style: rune::TextStyle| rune::text::measure(s, &[], style, None).0;
    let mono = |s: &str| m(s, tk.txs(400.0).mono());
    let xs = |s: &str| m(s, tk.txs(500.0));
    let pad = 21.6;
    let edge = |i: usize| if i == 0 || i == 5 { 24.0 - 10.8 } else { 0.0 };
    let heads = [
        "review:egress.table.time",
        "review:egress.table.subsystem",
        "review:egress.table.destination",
        "review:egress.table.operation",
        "review:egress.table.bytesOut",
        "review:egress.table.allowlist",
    ];
    let mut w: [f32; 6] =
        std::array::from_fn(|i| m(&t(heads[i]), tk.tsm(500.0)) + pad + edge(i) + 2.0);
    if rows.is_empty() {
        let sum: f32 = w.iter().sum();
        return (w.map(|x| x / sum), sum);
    }
    for r in rows {
        let mut dest = mono(&destination(r));
        if r["via_proxy"].as_bool() == Some(true) {
            dest += tk.r(0.5) + mono(&t("review:egress.table.proxy"));
        }
        let mut op = mono(s(r, "operation"));
        if !s(r, "method").is_empty() {
            op += tk.r(0.5) + m(s(r, "method"), tk.txs(400.0));
        }
        let status = match s(r, "allowlist_status") {
            "unexpected" => t("review:egress.status.unexpected"),
            "unknown" => t("review:egress.status.unknown"),
            _ if r["user_allowlisted"].as_bool() == Some(true) => t("review:egress.status.user"),
            _ => t("review:egress.status.builtIn"),
        };
        let cells = [
            tk.r(0.875) + tk.r(0.25) * 3.0 + mono(&super::time::clock_time(s(r, "ts"))),
            xs(&capitalize(&subsystem(s(r, "subsystem")))) + tk.r(1.0) + 2.0,
            dest,
            op,
            mono(&format_bytes(&r["bytes_out"])),
            xs(&status) + tk.r(1.0) + tk.r(0.25) + tk.r(0.75) + 2.0,
        ];
        for i in 0..6 {
            w[i] = w[i].max(cells[i] + pad + edge(i) + 2.0);
        }
    }
    let sum: f32 = w.iter().sum();
    (w.map(|x| x / sum), sum)
}

fn table(app: &App, tk: &Tk, records: &[Value], filtered: &[&Value], total_w: f32) -> El {
    let e = &app.reports.diag.egress;
    let (cols, _) = columns(tk, filtered);
    let th = |i: usize, key: &str, right: bool| {
        // The table kit's header label: sentence case, `text-sm` muted.
        let r = cell(i, cols[i], total_w, crate::ui::table::th_text(tk, t(key)))
            .h(tk.r(crate::ui::table::HEAD_H))
            .py(0.0);
        if right { r.justify_end() } else { r }
    };
    let mut c = col().w_full().child(
        crate::ui::table::head(tk)
            .child(th(0, "review:egress.table.time", false))
            .child(th(1, "review:egress.table.subsystem", false))
            .child(th(2, "review:egress.table.destination", false))
            .child(th(3, "review:egress.table.operation", false))
            .child(th(4, "review:egress.table.bytesOut", true))
            .child(th(5, "review:egress.table.allowlist", false)),
    );
    if filtered.is_empty() {
        let msg = if records.is_empty() {
            t("review:egress.empty")
        } else {
            t("review:egress.table.noRows")
        };
        // Review's centred empty state, inside the table.
        let glyph = if records.is_empty() {
            &I::GLOBE
        } else {
            &I::SEARCH
        };
        return c.child(
            col()
                .key("eg-empty")
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(2.5))
                .items_center()
                .gap(tk.r(0.75))
                .child(
                    row()
                        .size(tk.r(3.0), tk.r(3.0))
                        .radius(tk.r(1.5))
                        .center()
                        .fill(tk.muted)
                        .a11y_hidden()
                        .child(ic(glyph, tk.r(1.5), tk.muted_fg)),
                )
                .child(text(msg).font(tk.tsm(500.0)).color(tk.fg).role("status")),
        );
    }
    let n = filtered.len();
    for (i, r) in filtered.iter().enumerate() {
        let id = r["id"].as_u64().unwrap_or(0);
        let open = e.expanded.contains(&id);
        let dest = destination(r);
        let bad = s(r, "allowlist_status") == "unexpected";
        let toggle = tr("review:egress.details.toggle", &[("destination", &dest)]);
        let time_btn = row()
            .key(format!("eg-row-{id}"))
            .h(tk.r(1.5))
            .px(tk.r(0.25))
            .gap(tk.r(0.25))
            .center_y()
            .radius(tk.r(0.125))
            .hover(tk.muted)
            .role("button")
            .expanded(open)
            .label(toggle.clone())
            .help(toggle)
            .focusable(true)
            .on_press(move |a: &mut App| {
                let x = &mut a.reports.diag.egress.expanded;
                if !x.remove(&id) {
                    x.insert(id);
                }
            })
            .child(widgets::chevron(tk, open, tk.r(0.875)))
            .child(
                text(super::time::clock_time(s(r, "ts")))
                    .font(tk.txs(400.0).mono())
                    .color(tk.muted_fg),
            );
        let mut dest_el = row().gap(tk.r(0.5)).child(
            text(dest.clone()).font(tk.txs(400.0).mono()).color(if bad {
                tk.destructive
            } else {
                tk.fg
            }),
        );
        if r["via_proxy"].as_bool() == Some(true) {
            dest_el = dest_el.child(
                text(t("review:egress.table.proxy"))
                    .font(tk.txs(400.0).mono())
                    .color(tk.muted_fg),
            );
        }
        let mut op = row().gap(tk.r(0.5)).child(
            text(s(r, "operation").to_owned())
                .font(tk.txs(400.0).mono())
                .color(if bad { tk.destructive } else { tk.fg }),
        );
        if !s(r, "method").is_empty() {
            op = op.child(
                text(s(r, "method").to_owned())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
        }
        let last = i + 1 == n && !open;
        let mut tr_row = crate::ui::table::tr(tk, last)
            .child(cell(0, cols[0], total_w, time_btn))
            .child(cell(
                1,
                cols[1],
                total_w,
                widgets::badge(
                    tk,
                    BadgeVariant::Outline,
                    &capitalize(&subsystem(s(r, "subsystem"))),
                ),
            ))
            .child(cell(2, cols[2], total_w, dest_el))
            .child(cell(3, cols[3], total_w, op))
            .child(
                cell(
                    4,
                    cols[4],
                    total_w,
                    text(format_bytes(&r["bytes_out"]))
                        .font(tk.txs(400.0).mono())
                        .color(if bad { tk.destructive } else { tk.fg }),
                )
                .justify_end(),
            )
            .child(cell(5, cols[5], total_w, status_badge(tk, r)));
        if bad {
            tr_row = tr_row.fill(tk.destructive.alpha(0.1));
        }
        c = c.child(tr_row);
        if open {
            c = c.child(details(tk, r, &dest, i + 1 == n));
        }
    }
    c
}

/// CSS `text-transform: capitalize`: every word's first letter upper case.
fn capitalize(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn details(tk: &Tk, r: &Value, dest: &str, last: bool) -> El {
    let section = |title: String| {
        col()
            .grow(1.0)
            .min_w(0.0)
            .padding(tk.r(0.75))
            .gap(tk.r(0.75))
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .fill(tk.background.alpha(0.7))
            .child(
                text(title.to_uppercase())
                    .font(tk.txs(600.0))
                    .color(tk.muted_fg),
            )
    };
    let op = if s(r, "method").is_empty() {
        s(r, "operation").to_owned()
    } else {
        format!("{} {}", s(r, "operation"), s(r, "method"))
    };
    let meta = section(t("review:egress.details.capturedMetadata")).child(
        col()
            .w_full()
            .gap(tk.r(0.5))
            .child(detail(
                tk,
                t("review:egress.details.recordId"),
                format!("#{}", r["id"]),
                Ink::Neutral,
                true,
            ))
            .child(detail(
                tk,
                t("review:egress.details.fullTime"),
                s(r, "ts").to_owned(),
                Ink::Neutral,
                true,
            ))
            .child(detail(
                tk,
                t("review:egress.table.destination"),
                dest.to_owned(),
                Ink::Neutral,
                true,
            ))
            .child(detail(
                tk,
                t("review:egress.table.subsystem"),
                subsystem(s(r, "subsystem")),
                Ink::Neutral,
                false,
            ))
            .child(detail(
                tk,
                t("review:egress.table.operation"),
                op,
                Ink::Neutral,
                true,
            ))
            .child(detail(
                tk,
                t("review:egress.details.scheme"),
                if s(r, "scheme").is_empty() {
                    t("review:egress.details.none")
                } else {
                    s(r, "scheme").to_owned()
                },
                Ink::Neutral,
                true,
            ))
            .child(detail(
                tk,
                t("review:egress.table.bytesOut"),
                format_bytes(&r["bytes_out"]),
                Ink::Neutral,
                true,
            ))
            .child(detail(
                tk,
                t("review:egress.details.proxy"),
                if r["via_proxy"].as_bool() == Some(true) {
                    t("review:egress.details.yes")
                } else {
                    t("review:egress.details.no")
                },
                Ink::Neutral,
                false,
            ))
            .child(detail(
                tk,
                t("review:egress.details.allowlist"),
                allowlist_detail(r),
                Ink::Neutral,
                false,
            )),
    );
    let stored = section(t("review:egress.details.storedRecord")).child(
        col()
            .w_full()
            .max_h(tk.r(20.0))
            .padding(tk.r(0.75))
            .radius(tk.r(0.375))
            .fill(tk.muted.alpha(0.6))
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .child(
                text(serde_json::to_string_pretty(r).unwrap_or_default())
                    .font(tk.txs(400.0).mono())
                    .color(tk.fg)
                    .wrap(),
            ),
    );
    let not_captured = row()
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_dash(3.0, 2.0)
        .fill(tk.background.alpha(0.7))
        .child(detail(
            tk,
            t("review:egress.details.notCaptured"),
            t("review:egress.details.notCapturedValue"),
            Ink::Neutral,
            false,
        ));
    col()
        .w_full()
        .px(tk.card_pad())
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .fill(tk.muted.alpha(0.2))
        .border_sides([0.0, 0.0, if last { 0.0 } else { 1.0 }, 0.0], tk.border)
        .child(row().w_full().gap(tk.r(0.75)).child(meta).child(stored))
        .child(not_captured)
}

fn matches(r: &Value, needle: &str) -> bool {
    let id = r["id"].to_string();
    let port = r["port"]
        .as_u64()
        .filter(|p| *p != 0)
        .map(|p| p.to_string())
        .unwrap_or_default();
    [
        id.as_str(),
        s(r, "host"),
        port.as_str(),
        s(r, "subsystem"),
        s(r, "operation"),
        s(r, "method"),
        s(r, "allowlist_label"),
        s(r, "allowlist_source"),
    ]
    .iter()
    .filter(|v| !v.is_empty())
    .any(|v| v.to_lowercase().contains(needle))
}

/// The shell's cached `ui.network.offline` read (the title bar's indicator keeps it current):
/// (offline mode on, KASSIBER_NO_EGRESS set). No read of its own.
fn offline(app: &App) -> (bool, bool) {
    app.read("ui.network.offline")
        .map(|v| (v["offline"] == true, v["environment_blocked"] == true))
        .unwrap_or((false, false))
}

/// One destination (`host:port`) of the loaded records: its subsystems, count, bytes and
/// whether any of its requests fell outside the allowlist.
struct Dest {
    dest: String,
    host: String,
    subsystems: Vec<String>,
    records: u64,
    bytes: f64,
    unexpected: u64,
}

fn destinations(records: &[Value]) -> Vec<Dest> {
    let mut out: Vec<Dest> = Vec::new();
    for r in records {
        let dest = destination(r);
        let i = match out.iter().position(|d| d.dest == dest) {
            Some(i) => i,
            None => {
                out.push(Dest {
                    dest: dest.clone(),
                    host: s(r, "host").to_owned(),
                    subsystems: vec![],
                    records: 0,
                    bytes: 0.0,
                    unexpected: 0,
                });
                out.len() - 1
            }
        };
        let d = &mut out[i];
        d.records += 1;
        d.bytes += r["bytes_out"].as_f64().unwrap_or(0.0).max(0.0);
        if s(r, "allowlist_status") == "unexpected" {
            d.unexpected += 1;
        }
        let sub = s(r, "subsystem").to_owned();
        if !sub.is_empty() && !d.subsystems.contains(&sub) {
            d.subsystems.push(sub);
        }
    }
    // Unexpected destinations first, then the busiest.
    out.sort_by(|a, b| {
        b.unexpected
            .min(1)
            .cmp(&a.unexpected.min(1))
            .then(b.records.cmp(&a.records))
            .then(a.dest.cmp(&b.dest))
    });
    out
}

/// An aside card on the kit: its heading and rows.
fn aside_card(tk: &Tk, key: &str, glyph: &'static rune::Icon, title: String, body: El) -> El {
    widgets::surface(tk)
        .key(key.to_owned())
        .w_full()
        .clip_rounded(tk.r(0.75))
        .role("region")
        .label(title.clone())
        .child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .pt(tk.r(0.875))
                .pb(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .child(ic(glyph, tk.r(1.0), tk.muted_fg).a11y_hidden())
                .child(
                    text(title)
                        .font(tk.tsm(600.0))
                        .color(tk.fg)
                        .role("heading")
                        .level(2),
                ),
        )
        .child(body)
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, _layer: &mut Vec<El>) -> El {
    use crate::overview::grid::stat::{Stat, row_of};
    let e = &app.reports.diag.egress;
    let snap = e.snapshot.clone().unwrap_or(Value::Null);
    let records: Vec<Value> = snap["records"].as_array().cloned().unwrap_or_default();
    let needle = e.query.text.trim().to_lowercase();
    let filtered: Vec<&Value> = records
        .iter()
        .filter(|r| needle.is_empty() || matches(r, &needle))
        .collect();
    let db = snap["db_header"].clone();
    let fetching = !e.in_flight.is_empty();

    // ---- Toolbar: the filter on the left, Refresh (the page's one action) on the right.
    let k = crate::txdetail::widgets::form_kit(tk);
    let mut query = rune_kit::TextInput::new(&e.query)
        .label(t("review:egress.queryPlaceholder"))
        .placeholder(t("review:egress.queryPlaceholder"))
        .height(tk.r(2.25))
        .text_style(tk.tsm(400.0))
        .leading_icon(&I::SEARCH);
    if !e.query.text.is_empty() {
        query = query.clear_button();
    }
    let query = query
        .on_edit(|a: &mut App| &mut a.reports.diag.egress.query)
        .build(&k)
        .key("eg-query".to_owned())
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .w(tk.r(20.0))
        .min_w(tk.r(9.0))
        .shrink();
    let refresh = {
        let g = ic(&I::REFRESH_CW, tk.r(1.0), tk.fg);
        let g = if fetching {
            g.looping(rune::LoopSpec::rotation(1.0))
        } else {
            g
        };
        let mut b = crate::txdetail::widgets::button_surface(
            tk,
            row()
                .key("eg-refresh")
                .h(tk.r(2.0))
                .px(tk.r(0.75))
                .gap(tk.r(0.5))
                .center(),
            Variant::Outline,
        )
        .role("button")
        .label(t("review:egress.refresh"))
        .focusable(!fetching)
        .disabled(fetching)
        .opacity(if fetching { 0.5 } else { 1.0 })
        .child(g)
        .child(
            text(t("review:egress.refresh"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        );
        if !fetching {
            b = b.on_press(fetch);
        }
        b
    };
    let left = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .role("toolbar")
        .label(t("review:egress.queryPlaceholder"))
        .child(query);
    let mut page = widgets::shell(tk).child(crate::reports::ledger::common::page_header(
        tk,
        left,
        Some(refresh),
    ));
    if let Some(err) = &e.error {
        page = page.child(
            row()
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.5))
                .items_start()
                .radius(tk.r(0.5))
                .border(1.0, tk.destructive.alpha(0.3))
                .fill(tk.destructive.alpha(0.1))
                .role("alert")
                .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), tk.destructive))
                .child(
                    text(tr("review:egress.loadFailed", &[("error", err)]))
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap()
                        .grow(1.0)
                        .w(0.0),
                ),
        );
    }

    // ---- Stat tiles: what was recorded, then signals only (offline mode, unexpected
    // destinations, a database header that isn't ciphertext). "0 unexpected" and
    // "ciphertext-like" stay in the table's badges and the aside.
    let total = snap["summary"]["total_records"].as_u64().unwrap_or(0);
    let unexpected = snap["summary"]["unexpected"].as_u64().unwrap_or(0);
    let dests = destinations(&records);
    let bytes: f64 = dests.iter().map(|d| d.bytes).sum();
    let since = snap["started_at"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(super::time::clock_time);
    let mut stats = Vec::new();
    let (off, env) = offline(app);
    if off || env {
        let line = if off {
            t("chrome:network.offlineMode.on")
        } else {
            t("chrome:network.offlineMode.environment")
        };
        stats.push(
            Stat::new(
                "eg-offline",
                &I::WIFI_OFF,
                t("chrome:network.indicator.offlineMode"),
                if off {
                    t("chrome:network.health.offline")
                } else {
                    "KASSIBER_NO_EGRESS".to_owned()
                },
            )
            .line(line.clone())
            .help(line),
        );
    }
    let mut recorded = Stat::new(
        "eg-metric-total",
        &I::ACTIVITY,
        t("review:egress.metric.total"),
        crate::format::count(total as usize),
    )
    .line(format!(
        "{}: {}",
        t("review:egress.table.bytesOut"),
        format_bytes(&json!(bytes))
    ));
    if let Some(since) = &since {
        recorded = recorded.help(format!("{} · {since}", t("review:egress.subtitle")));
    }
    stats.push(recorded);
    let mut kinds: Vec<String> = Vec::new();
    for d in &dests {
        for x in &d.subsystems {
            let x = capitalize(&subsystem(x));
            if !kinds.contains(&x) {
                kinds.push(x);
            }
        }
    }
    stats.push(
        Stat::new(
            "eg-metric-dest",
            &I::GLOBE,
            t("review:egress.table.destination"),
            crate::format::count(dests.len()),
        )
        .line(if kinds.is_empty() {
            t("review:egress.subsystems.empty")
        } else {
            kinds.join(", ")
        }),
    );
    if unexpected > 0 {
        let first = dests
            .iter()
            .filter(|d| d.unexpected > 0)
            .map(|d| d.dest.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let mut st = Stat::new(
            "eg-metric-unexpected",
            &I::TRIANGLE_ALERT,
            t("review:egress.metric.unexpected"),
            crate::format::count(unexpected as usize),
        )
        .ink(tk.destructive)
        .glyph_ink(tk.destructive);
        st = if first.is_empty() {
            st.line(t("review:egress.status.unexpected"))
        } else {
            st.line_with(first, Some(tk.destructive), false)
        };
        stats.push(st);
    }
    match db_tone(&db) {
        Ink::Good => {}
        tone => {
            let ink = if tone == Ink::Bad {
                tk.destructive
            } else {
                tk.warn.solid
            };
            stats.push(
                Stat::new(
                    "eg-metric-db",
                    &I::DATABASE,
                    t("review:egress.metric.dbHeader"),
                    db_label(&db),
                )
                .ink(ink)
                .glyph_ink(ink)
                .line(t("review:egress.db.title")),
            );
        }
    }
    page = page.child(row_of(
        app,
        cx,
        tk,
        "eg-metrics",
        t("chrome:shell.extras.egress"),
        stats,
    ));

    // ---- The ledger card: head with its signals (allowlist incomplete, rows evicted), the
    // table on the kit, rows unfolding into their details.
    let mut signals = row().gap(tk.r(0.5)).center_y().flex_wrap();
    if snap["allowlist_complete"].as_bool() == Some(false) {
        signals = signals.child(widgets::tone_badge(
            tk,
            &Tone {
                bg: rune::Color::TRANSPARENT,
                border: tk.border,
                text: tk.fg,
                solid: tk.fg,
            },
            Some((&I::LOCK_KEYHOLE, tk.fg)),
            &t("review:egress.db.incompleteAllowlist"),
            false,
        ));
    }
    if snap["gap"].as_bool() == Some(true) {
        signals = signals.child(widgets::badge(
            tk,
            BadgeVariant::Outline,
            &t("review:egress.gap"),
        ));
    }
    if fetching {
        signals = signals.child(crate::reports::ledger::common::inline_status(
            tk,
            t("review:activity.refreshing"),
        ));
    }
    let head = crate::reports::ledger::common::card_head(
        tk,
        t("review:egress.subtitle"),
        Some(t("review:egress.notMonitored")),
        Some(signals),
    );
    // `lg:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]`: the aside beside the ledger from `lg`.
    let win = rune::memo::layout_size("sh-root")
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(if app.shell.window_w > 0.0 {
            app.shell.window_w
        } else {
            1440.0
        });
    // Beside the ledger only where the table keeps its columns next to it; otherwise the three
    // cards sit in a row under it (stacked in a narrow window).
    let lg = win >= 1720.0;
    let md = win >= 1100.0;
    let nav = if crate::shell::nav_folded(app, cx) {
        tk.r(3.0)
    } else {
        tk.r(16.0)
    };
    let aside_w = tk.r(22.0);
    let avail = win
        - nav
        - tk.r(0.375)
        - 2.0
        - 2.0 * tk.page_pad()
        - 2.0
        - if lg { tk.card_gap() + aside_w } else { 0.0 };
    let least = columns(tk, &filtered).1.max(760.0);
    let (table_w, overflow) = crate::ui::table::fit(avail, least);
    let grid = table(app, tk, &records, &filtered, table_w).w(table_w);
    let ledger = widgets::surface(tk)
        .key("eg-ledger")
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .clip_rounded(tk.r(0.75))
        .child(head)
        .child(crate::ui::table::hscroll_if(
            "eg-table-scroll",
            t("chrome:shell.extras.egress"),
            overflow,
            grid,
        ));

    // ---- The aside: requests per destination (a press filters the ledger to it), per
    // subsystem, and the database header proof.
    let mut dl = col().w_full().pb(tk.r(0.5)).role("list");
    if dests.is_empty() {
        dl = dl.child(
            text(t("review:egress.empty"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .px(tk.r(1.0))
                .pb(tk.r(0.5)),
        );
    }
    for d in dests.iter().take(12) {
        let bad = d.unexpected > 0;
        let host = d.host.clone();
        let subs = d
            .subsystems
            .iter()
            .map(|x| capitalize(&subsystem(x)))
            .collect::<Vec<_>>()
            .join(", ");
        dl = dl.child(
            row()
                .key(format!("eg-dest-{}", d.dest))
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.375))
                .gap(tk.r(0.75))
                .center_y()
                .hover(tk.muted.alpha(0.5))
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(format!(
                    "{} · {} · {}",
                    d.dest,
                    subs,
                    t("review:egress.queryPlaceholder")
                ))
                .focusable(true)
                .on_press(move |a: &mut App| {
                    a.reports.diag.egress.query = TextEdit::new(host.as_str())
                })
                .child(
                    col()
                        .grow(1.0)
                        .w(0.0)
                        .min_w(0.0)
                        .child(
                            text(d.dest.clone())
                                .font(tk.txs(400.0).mono())
                                .color(if bad { tk.destructive } else { tk.fg })
                                .single_line()
                                .truncate()
                                .min_w(0.0),
                        )
                        .child(
                            text(subs)
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .single_line()
                                .truncate()
                                .min_w(0.0),
                        ),
                )
                .child(
                    text(format!("{} / {}", d.records, format_bytes(&json!(d.bytes))))
                        .font(tk.txs(400.0).mono())
                        .color(tk.muted_fg)
                        .single_line(),
                ),
        );
    }
    let mut subs = col().w_full().px(tk.r(1.0)).pb(tk.r(0.875)).gap(tk.r(0.5));
    let by: Vec<(String, Value)> = snap["summary"]["by_subsystem"]
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    if by.is_empty() {
        subs = subs.child(
            text(t("review:egress.subsystems.empty"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        );
    } else {
        for (name, item) in by {
            subs = subs.child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .center_y()
                    .justify_between()
                    .child(
                        text(capitalize(&subsystem(&name)))
                            .font(tk.tsm(400.0))
                            .color(tk.fg),
                    )
                    .child(
                        text(format!(
                            "{} / {}",
                            item["records"].as_u64().unwrap_or(0),
                            format_bytes(&item["bytes_out"])
                        ))
                        .font(tk.txs(400.0).mono())
                        .color(tk.muted_fg),
                    ),
            );
        }
    }
    let db_body = col()
        .w_full()
        .px(tk.r(1.0))
        .pb(tk.r(0.875))
        .gap(tk.r(0.5))
        .child(detail(
            tk,
            t("review:egress.db.classification"),
            db_label(&db),
            db_tone(&db),
            false,
        ))
        .child(detail(
            tk,
            t("review:egress.db.prefix"),
            if s(&db, "prefix_hex").is_empty() {
                t("review:egress.db.none")
            } else {
                s(&db, "prefix_hex").to_owned()
            },
            Ink::Neutral,
            true,
        ));
    let cards = [
        aside_card(
            tk,
            "eg-destinations",
            &I::GLOBE,
            t("review:egress.table.destination"),
            dl,
        ),
        aside_card(
            tk,
            "eg-subsystems",
            &I::LAYOUT_DASHBOARD,
            t("review:egress.subsystems.title"),
            subs,
        ),
        aside_card(
            tk,
            "eg-db",
            &I::DATABASE,
            t("review:egress.db.title"),
            db_body,
        ),
    ];
    let aside = if lg {
        col()
            .w(aside_w)
            .min_w(aside_w)
            .gap(tk.card_gap())
            .children(cards)
    } else if md {
        let [a, b, c] = cards;
        row()
            .w_full()
            .gap(tk.card_gap())
            .items_start()
            .child(a.grow(1.3).w(0.0))
            .child(b.grow(1.0).w(0.0))
            .child(c.grow(1.0).w(0.0))
    } else {
        col().w_full().gap(tk.card_gap()).children(cards)
    };
    let _ = el;
    if lg {
        page.child(
            row()
                .w_full()
                .gap(tk.card_gap())
                .items_start()
                .child(ledger)
                .child(aside),
        )
    } else {
        page.child(
            col()
                .w_full()
                .gap(tk.card_gap())
                .child(ledger.w_full())
                .child(aside),
        )
    }
}

/// SYNTHETIC ledger rows for the `reports-egress-sample` scene (the disposable book records
/// no egress and the scene never makes any to fill it): representative rows of each
/// subsystem, one destination outside the allowlist, one via a proxy. Not real traffic.
pub fn inject_sample(app: &mut App) {
    let now = crate::format::now_secs();
    let at = |ago: f64| super::logstore::iso_from_secs(now - ago);
    let e = &mut app.reports.diag.egress;
    let mut snap = e.snapshot.clone().unwrap_or(json!({}));
    let rec = |id: u64,
               ago: f64,
               sub: &str,
               host: &str,
               port: u64,
               scheme: &str,
               op: &str,
               method: Option<&str>,
               bytes: u64,
               proxy: bool,
               status: &str,
               label: Option<&str>| {
        json!({"id": id, "ts": at(ago), "subsystem": sub, "host": host, "port": port, "scheme": scheme, "operation": op, "method": method, "bytes_out": bytes, "via_proxy": proxy, "allowlist_status": status, "allowlist_label": label, "allowlist_source": label.map(|_| "built-in"), "user_allowlisted": false})
    };
    let records = vec![
        rec(
            7,
            4.0,
            "sync",
            "198.51.100.7",
            50002,
            "ssl",
            "electrum.subscribe",
            None,
            2048,
            true,
            "unexpected",
            None,
        ),
        rec(
            6,
            9.0,
            "sync",
            "electrum.example.org",
            50002,
            "ssl",
            "electrum.history",
            None,
            6144,
            false,
            "expected",
            Some("Wallet backend"),
        ),
        rec(
            5,
            15.0,
            "sync",
            "electrum.example.org",
            50002,
            "ssl",
            "electrum.headers",
            None,
            512,
            false,
            "expected",
            Some("Wallet backend"),
        ),
        rec(
            4,
            42.0,
            "pricing",
            "api.coingecko.com",
            443,
            "https",
            "rates.latest",
            Some("GET"),
            412,
            false,
            "expected",
            Some("CoinGecko"),
        ),
        rec(
            3,
            120.0,
            "pricing",
            "api.coingecko.com",
            443,
            "https",
            "rates.history",
            Some("GET"),
            655,
            false,
            "expected",
            Some("CoinGecko"),
        ),
        rec(
            2,
            300.0,
            "ai",
            "127.0.0.1",
            11434,
            "http",
            "chat.completions",
            Some("POST"),
            3280,
            false,
            "expected",
            Some("Local AI provider"),
        ),
    ];
    snap["records"] = json!(records);
    snap["started_at"] = json!(at(3_600.0));
    snap["allowlist_complete"] = json!(true);
    snap["summary"] = json!({"total_records": 6, "unexpected": 1, "update": 0, "by_subsystem": {"ai": {"records": 1, "bytes_out": 3280}, "pricing": {"records": 2, "bytes_out": 1067}, "sync": {"records": 3, "bytes_out": 8704}}});
    snap["db_header"] = json!({"exists": true, "classification": "encrypted", "encrypted_like": true, "sqlite_plaintext_header": false, "prefix_hex": "8f2a91c4d07e33b5"});
    e.snapshot = Some(snap);
    e.expanded.insert(4);
    // Keep the injected rows on screen (no 4 s refetch replacing them).
    e.next_at = f64::INFINITY;
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_poll_refused_behind_the_lock_does_not_wedge_the_monitor() {
        let mut app = crate::app::tests::quiet_app();
        app.locked = true;
        // `prime` waits until every poll it sent is answered: the lock's refusal answers it.
        super::prime(&mut app);
        assert!(app.reports.diag.egress.in_flight.is_empty());
        assert!(
            app.reports.diag.egress.error.is_none(),
            "a refusal behind the lock is not an error to show"
        );
        assert_eq!(
            app.reports.diag.egress.next_at, 0.0,
            "the next poll runs right after the unlock"
        );
    }

    use super::*;

    #[test]
    fn destinations_group_by_host_and_port_unexpected_first() {
        let r = |host: &str, port: u64, sub: &str, bytes: u64, status: &str| json!({"host": host, "port": port, "subsystem": sub, "bytes_out": bytes, "allowlist_status": status});
        let rows = vec![
            r("a.example", 443, "pricing", 100, "expected"),
            r("a.example", 443, "pricing", 50, "expected"),
            r("b.example", 50002, "sync", 10, "unexpected"),
            r("a.example", 80, "update", 5, "expected"),
        ];
        let d = destinations(&rows);
        assert_eq!(
            d.iter().map(|d| d.dest.as_str()).collect::<Vec<_>>(),
            ["b.example:50002", "a.example:443", "a.example:80"]
        );
        assert_eq!(
            (d[1].records, d[1].bytes as u64, d[1].unexpected),
            (2, 150, 0)
        );
        assert_eq!(d[0].unexpected, 1);
        assert_eq!(d[0].host, "b.example");
    }

    #[test]
    fn capitalize_is_css_capitalize() {
        assert_eq!(capitalize("price feed"), "Price Feed");
        assert_eq!(capitalize("AI"), "AI");
        assert_eq!(capitalize(""), "");
    }
}
