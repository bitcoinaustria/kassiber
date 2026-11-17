//! The node detail page as NodeConnectionDetail.tsx draws it: `NodeHeader`, the no-snapshot
//! card, `NodeOperatorHero`, the five tabs (Overview, Channels, Activity, Profitability,
//! Accounting) with their cards, and `ChannelDetailSheet`.
//!
//! Breakpoints follow the web's viewport media queries (`sm` 640, `md` 768, `xl` 1280, `2xl`
//! 1536 px of window width).

use rune::{Color, Cx, El, Icon, Shadow, TextStyle, Tx, col, el, row, text};

use super::Tab;
use super::model::{self as m, Channel, Forward, Node, Pill, Routing};
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::{MASK, Tk};
use crate::ui::{ic, surface};
use crate::wallets::conn::{Conn, kind_label};

/// Window width (viewport) for the web's media queries.
fn vw(app: &App, cx: &Cx<App>) -> f32 {
    cx.layout_size("sh-root")
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(if app.shell.window_w > 0.0 {
            app.shell.window_w
        } else {
            1440.0
        })
}

fn hidden(app: &App) -> bool {
    app.prefs.hide_sensitive()
}

/// Text the privacy modes blur (`.sensitive`).
fn sens(s: impl Into<String>, style: TextStyle, color: Color, hide: bool) -> El {
    crate::ui::sensitive(s, style, color, hide)
}

fn mono(tk: &Tk, size_rem: f32, line_rem: f32, w: f32) -> TextStyle {
    TextStyle::new(tk.r(size_rem), w)
        .lh(tk.r(line_rem))
        .mono()
        .features("'tnum' 1")
}

// ---- Buttons ---------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    Default,
    Outline,
}

/// shadcn `Button size="sm"`: `h-8 gap-1.5 rounded-md px-3 text-sm font-medium` (`px-2.5` with an
/// icon), a 1 rem icon.
fn button_sm(
    tk: &Tk,
    key: &str,
    variant: Variant,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
    spin: bool,
) -> El {
    use crate::ui::button as b;
    let v = match variant {
        Variant::Default => b::Variant::Default,
        Variant::Outline => b::Variant::Outline,
    };
    b::button_busy(tk, key, v, b::Size::Sm, spin, glyph, label, enabled)
}

/// The Refresh button (`onSync`, egress on press). `wide` keeps the header button as wide as its
/// longest label (`invisible` Refresh / Refreshing stacked in one grid cell).
#[allow(clippy::too_many_arguments)]
fn refresh_button(
    app: &App,
    tk: &Tk,
    key: &str,
    variant: Variant,
    c: &Conn,
    busy: bool,
    wide: bool,
    a11y: Option<String>,
) -> El {
    let label = if busy {
        t("connections:node.header.refreshing")
    } else {
        t("connections:node.header.refresh")
    };
    let mut b = button_sm(tk, key, variant, Some(&I::REFRESH_CW), &label, !busy, busy).busy(busy);
    if wide {
        let w = [
            t("connections:node.header.refresh"),
            t("connections:node.header.refreshing"),
        ]
        .iter()
        .map(|s| rune::text::measure(s, &[], tk.tsm(500.0), None).0)
        .fold(0.0f32, f32::max);
        // `h-8 gap-2` (pageHeaderActionClassName), px-2.5, the 1 rem icon, the widest label.
        b = b
            .gap(tk.r(0.5))
            .w(w + tk.r(1.0) + tk.r(0.5) + 2.0 * tk.r(0.625) + 2.0);
    }
    if let Some(n) = a11y {
        b = b.label(n);
    }
    let _ = app;
    let id = c.id.clone();
    if busy {
        b
    } else {
        b.on_press(move |a: &mut App| super::sync(a, &id))
    }
}

/// A link styled as an outline `sm` button (`Button asChild` around a router `Link`).
fn link_button(tk: &Tk, key: &str, glyph: &'static Icon, label: &str, to: String) -> El {
    button_sm(tk, key, Variant::Outline, Some(glyph), label, true, false)
        .role("link")
        .on_press(move |a: &mut App| a.navigate(&to))
}

fn transactions_route(c: &Conn) -> String {
    let enc: String = c
        .label
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("/transactions?wallet={enc}")
}

// ---- Cards -----------------------------------------------------------------------------------

/// `Card` (`kb-surface gap-4 py-5`) with a `CardHeader className="border-b px-4 pb-3"` (the
/// `[.border-b]:pb-5` rule wins over `pb-3`): title `text-sm sm:text-base font-semibold
/// leading-none` (with an icon `gap-2`), description `text-sm text-muted-foreground`, gap 1.5.
fn card(
    tk: &Tk,
    wide: bool,
    glyph: Option<&'static Icon>,
    title: El,
    description: Option<El>,
    content: Option<El>,
) -> El {
    let title_font = if wide { tk.tbase(600.0) } else { tk.tsm(600.0) }.lh(if wide {
        tk.r(1.0)
    } else {
        tk.r(0.875)
    });
    let mut t_row = row().gap(tk.r(0.5)).center_y().role("heading");
    if let Some(g) = glyph {
        t_row = t_row.child(ic(g, tk.r(1.0), tk.fg));
    }
    let _ = title_font;
    t_row = t_row.child(title);
    let mut head = col()
        .w_full()
        .px(tk.r(1.0))
        .pb(tk.card_pad())
        .gap(tk.r(0.375))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(t_row);
    if let Some(d) = description {
        head = head.child(d);
    }
    let mut c = surface(tk)
        .w_full()
        .min_w(0.0)
        .py(tk.card_pad())
        .gap(tk.r(1.0))
        .child(head);
    if let Some(body) = content {
        c = c.child(body);
    }
    c
}

fn card_title(tk: &Tk, wide: bool, s: &str) -> El {
    let f = if wide {
        tk.tbase(600.0).lh(tk.r(1.0))
    } else {
        tk.tsm(600.0).lh(tk.r(0.875))
    };
    text(s.to_owned()).font(f).color(tk.fg)
}

/// The grid rows of `CardHeader` place the description one `leading-none` title row plus
/// `gap-1.5` below the title's top; Rune's 1 rem title box sits 4 pt short of the browser's.
fn card_desc(tk: &Tk, s: String) -> El {
    text(s)
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .wrap()
        .mt(4.0)
}

/// `text-2xs font-medium uppercase tracking-wide text-muted-foreground` with an optional icon.
fn eyebrow(tk: &Tk, glyph: Option<&'static Icon>, s: &str) -> El {
    let mut r = row().gap(tk.r(0.5)).center_y().min_w(0.0);
    if let Some(g) = glyph {
        r = r.child(ic(g, tk.r(1.0), tk.muted_fg));
    }
    r.child(
        text(s.to_uppercase())
            .font(tk.t2xs(500.0).tracking(0.025 * tk.r(0.694)))
            .color(tk.muted_fg)
            .truncate(),
    )
}

/// `rounded-md border bg-background px-3 py-2.5`.
fn tile(tk: &Tk) -> El {
    col()
        .min_w(0.0)
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background)
        .px(tk.r(0.75))
        .py(tk.r(0.625))
}

/// `CompactStat`.
fn compact_stat(tk: &Tk, glyph: &'static Icon, label: &str, value: String, detail: String) -> El {
    tile(tk)
        .child(eyebrow(tk, Some(glyph), label))
        .child(
            text(value)
                .font(mono(tk, 0.875, 1.25, 600.0))
                .color(tk.fg)
                .truncate()
                .mt(tk.r(0.25)),
        )
        .child(
            text(detail)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .mt(tk.r(0.125)),
        )
}

/// A ring-inset status pill (`rounded-md px-1.5 py-0.5 text-2xs font-medium ring-1 ring-inset`).
fn ring_pill(tk: &Tk, p: Pill, glyph: Option<&'static Icon>, label: String) -> El {
    let mut r = row()
        .px(tk.r(0.375))
        .py(tk.r(0.125))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .fill(p.fill)
        .border(1.0, p.ring)
        .border_layout();
    if let Some(g) = glyph {
        r = r.child(ic(g, tk.r(0.875), p.ink));
    }
    r.child(text(label).font(tk.t2xs(500.0)).color(p.ink))
}

/// The gray count chip beside a card title (`px-2 py-1 text-2xs sm:text-xs`).
fn count_chip(tk: &Tk, wide: bool, n: usize) -> El {
    let p = m::gray(tk);
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.25))
        .radius(tk.r(0.375))
        .fill(p.fill)
        .border(1.0, p.ring)
        .border_layout()
        .child(
            text(n.to_string())
                .font(if wide { tk.txs(500.0) } else { tk.t2xs(500.0) })
                .color(p.ink),
        )
}

/// Badge `variant="outline" rounded-md text-2xs`.
fn outline_badge(tk: &Tk, s: String) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .child(text(s).font(tk.t2xs(500.0)).color(tk.fg))
}

/// An equal-column grid (`grid-cols-N gap-3`).
fn grid(tk: &Tk, cols: usize, items: Vec<El>) -> El {
    let gap = tk.r(0.75);
    let mut out = col().w_full().gap(gap);
    let mut it = items.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row().w_full().gap(gap).items_stretch();
        for _ in 0..cols {
            r = match it.next() {
                Some(e) => r.child(e.grow(1.0).min_w(0.0)),
                None => r.child(el().grow(1.0)),
            };
        }
        out = out.child(r);
    }
    out
}

/// `grid-cols-[minmax(0,1fr)_minmax(360px,0.8fr)] gap-3` at `xl`, stacked below.
fn split(tk: &Tk, xl: bool, a: El, b: El) -> El {
    if xl {
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .child(a.grow(1.0).min_w(0.0))
            .child(b.grow(0.8).min_w(360.0))
    } else {
        col().w_full().gap(tk.r(0.75)).child(a).child(b)
    }
}

/// A liquidity bar (`flex h-N overflow-hidden rounded-full bg-muted`, emerald then sky).
fn bar(tk: &Tk, height: f32, local_pct: f32, remote_pct: f32, aria: String) -> El {
    row()
        .w_full()
        .h(height)
        .radius(height / 2.0)
        .fill(tk.muted)
        .clip_rounded(height / 2.0)
        .role("img")
        .label(aria)
        .child(
            el().h_full()
                .w_frac(local_pct.clamp(0.0, 100.0) / 100.0)
                .fill(m::bar_local(tk)),
        )
        .child(
            el().h_full()
                .w_frac(remote_pct.clamp(0.0, 100.0) / 100.0)
                .fill(m::bar_remote(tk)),
        )
}

/// `DetailRow`: `flex items-center gap-3 text-sm`, the label `text-xs font-medium muted`, the
/// value right-aligned and truncated (`font-mono text-xs` when `mono`), a copy button for
/// string values with `copy`.
fn detail_row(app: &App, tk: &Tk, key: &str, label: &str, value: El, copy: Option<String>) -> El {
    let mut r = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.75))
        .center_y()
        .child(
            text(label.to_owned())
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        )
        .child(row().grow(1.0).min_w(0.0).justify_end().child(value));
    if let Some(v) = copy {
        r = r.child(copy_button(app, tk, key, v));
    }
    r
}

fn dv(tk: &Tk, s: impl Into<String>, mono_font: bool) -> El {
    let f = if mono_font {
        mono(tk, 0.75, 1.0, 400.0)
    } else {
        tk.tsm(400.0)
    };
    text(s.into()).font(f).color(tk.fg).truncate()
}

fn dv_sens(tk: &Tk, s: impl Into<String>, mono_font: bool, hide: bool) -> El {
    let f = if mono_font {
        mono(tk, 0.75, 1.0, 400.0)
    } else {
        tk.tsm(400.0)
    };
    sens(s, f, tk.fg, hide).truncate()
}

/// `CopyButton` (outline `icon-xs`, `Copy` → `Check` for 1.1 s, named Copy / Copied).
fn copy_button(app: &App, tk: &Tk, key: &str, value: String) -> El {
    let copied = app
        .wallets
        .node
        .copied
        .as_ref()
        .is_some_and(|(k, _)| k == key);
    let label = if copied {
        t("chrome:copyButton.copied")
    } else {
        t("chrome:copyButton.copy")
    };
    let k = key.to_owned();
    let glyph = if copied {
        ic(
            &I::CHECK,
            tk.r(0.75),
            if tk.dark {
                rune::color::hex("#34d399").unwrap()
            } else {
                rune::color::hex("#059669").unwrap()
            },
        )
    } else {
        ic(&I::COPY, tk.r(0.75), tk.fg)
    };
    crate::ui::outline_surface(
        tk,
        row()
            .key(format!("node-copy-{key}"))
            .size(tk.r(1.5), tk.r(1.5))
            .center()
            .radius(tk.r(0.375)),
    )
    .role("button")
    .label(label)
    .focusable(true)
    .on_press_tx(move |a: &mut App, tx: &mut Tx| {
        crate::osleaks::clipboard::copy(a, tx, value.clone());
        a.wallets.node.copied = Some((k.clone(), a.clock + 1.1));
    })
    .child(glyph)
}

/// `ConnectionAssetBadge size="md"` (LND's lightning-labs.svg on its violet tile included).
fn node_badge(app: &App, cx: &Cx<App>, tk: &Tk, c: &Conn) -> El {
    crate::wallets::common::asset_badge(app, cx, tk, c, None, true)
}

// ---- Page ------------------------------------------------------------------------------------

pub fn page(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    c: &Conn,
    node: Option<Node>,
) -> El {
    let st = &app.wallets.node;
    let snapshot_loading = node.is_none() && st.fetching;
    let busy = st.fetching || snapshot_loading;
    let w = vw(app, cx);
    let sm = w >= 640.0;
    let shell = col()
        .key("wallets-node")
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background);
    let header = header(app, cx, tk, c, node.as_ref(), busy, sm);
    let Some(node) = node else {
        let title = t(if snapshot_loading {
            "connections:node.loadingSnapshot.title"
        } else {
            "connections:node.noSnapshot.title"
        });
        let desc = if snapshot_loading {
            t("connections:node.loadingSnapshot.description")
        } else if let Some(e) = &st.error {
            tr(
                "connections:node.noSnapshot.errorDescription",
                &[("message", e)],
            )
        } else {
            t("connections:node.noSnapshot.description")
        };
        let body = row()
            .w_full()
            .px(tk.r(1.0))
            .pt(tk.r(1.0))
            .gap(tk.r(0.5))
            .items_start()
            .child(refresh_button(
                app,
                tk,
                "node-empty-refresh",
                Variant::Outline,
                c,
                busy,
                false,
                None,
            ));
        return shell.child(header).child(card(
            tk,
            sm,
            None,
            card_title(tk, sm, &title),
            Some(card_desc(tk, desc)),
            Some(body),
        ));
    };
    let xl = w >= 1280.0;
    let mut out = shell.child(header).child(hero(app, tk, c, &node, busy, w));
    out = out.child(tabs(tk, sm, st.tab));
    let content = match st.tab {
        Tab::Overview => col()
            .w_full()
            .gap(tk.r(0.75))
            .child(metrics(app, tk, &node, w))
            .child(split(
                tk,
                xl,
                activity_card(tk, sm, xl, &node),
                accounting_card(tk, sm, c, &node),
            )),
        Tab::Channels => col()
            .w_full()
            .gap(tk.r(0.75))
            .child(channels_card(app, cx, tk, sm, &node, layer)),
        Tab::Activity => {
            let mut col_ = col()
                .w_full()
                .gap(tk.r(0.75))
                .child(activity_card(tk, sm, xl, &node));
            if let Some(f) = node.forwards.as_ref().filter(|f| !f.is_empty()) {
                col_ = col_.child(forwards_card(app, cx, tk, sm, f));
            }
            col_
        }
        Tab::Profitability => {
            let mut col_ = col().w_full().gap(tk.r(0.75));
            if let Some(r) = &node.routing {
                col_ = col_.child(routing_card(app, tk, sm, w, r));
            }
            col_.child(economics_card(app, cx, tk, sm, &node.channels))
        }
        Tab::Accounting => col().w_full().gap(tk.r(0.75)).child(split(
            tk,
            xl,
            accounting_card(tk, sm, c, &node),
            details_cards(app, tk, sm, c, &node),
        )),
    };
    // `<Tabs className="gap-3">`: the panel sits 0.75 rem under the list.
    out.child(content.role("tabpanel").label(tab_label(st.tab)))
}

/// `NodeHeader`.
fn header(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    c: &Conn,
    node: Option<&Node>,
    busy: bool,
    sm: bool,
) -> El {
    let back = crate::ui::button::icon_button(
        tk,
        "node-back",
        crate::ui::button::Variant::Outline,
        tk.r(2.0),
        &I::ARROW_LEFT,
        tk.r(1.0),
        t("connections:node.header.backToWallets"),
    )
    .role("link")
    .on_press(|a: &mut App| a.navigate("/connections"));
    let dot = || {
        text("·")
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .a11y_hidden()
    };
    let mut sub = row()
        .mt(tk.r(0.25))
        .gap(tk.r(0.5))
        .center_y()
        .min_w(0.0)
        .child(
            text(kind_label(&c.kind))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate(),
        );
    if let Some(n) = node {
        if !n.alias.is_empty() {
            sub = sub.child(dot()).child(
                text(n.alias.clone())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .truncate(),
            );
        }
        if !n.network.is_empty() {
            sub = sub.child(dot()).child(
                text(n.network.clone())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .truncate(),
            );
        }
    }
    if c.status != "synced"
        && let Some(p) = crate::wallets::common::status_pill(tk, &c.status, true)
    {
        sub = sub.child(dot()).child(p);
    }
    let title_font = if sm {
        TextStyle::new(tk.r(1.5), 600.0).lh(tk.r(2.0))
    } else {
        tk.txl(600.0)
    }
    .tracking(-0.025 * tk.r(1.5));
    let mut left = row()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.75))
        .center_y()
        .child(back);
    if sm {
        left = left.child(node_badge(app, cx, tk, c));
    }
    left = left.child(
        col()
            .min_w(0.0)
            .child(
                text(c.label.clone())
                    .font(title_font)
                    .color(tk.fg)
                    .truncate()
                    .role("heading"),
            )
            .child(sub),
    );
    let refresh_label = if busy {
        t("connections:node.header.refreshing")
    } else {
        t("connections:node.header.refresh")
    };
    let a11y = tr(
        "connections:node.header.refreshAction",
        &[("action", &refresh_label), ("label", &c.label)],
    );
    row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(left)
        .child(refresh_button(
            app,
            tk,
            "node-header-refresh",
            Variant::Outline,
            c,
            busy,
            true,
            Some(a11y),
        ))
}

/// `HeroStat`.
fn hero_stat(
    tk: &Tk,
    glyph: &'static Icon,
    label: &str,
    value: String,
    detail: String,
    tone: Color,
    hide: bool,
) -> El {
    tile(tk)
        .child(eyebrow(tk, Some(glyph), label))
        .child(
            sens(value, mono(tk, 1.0, 1.5, 600.0), tone, hide)
                .truncate()
                .mt(tk.r(0.25)),
        )
        .child(
            sens(detail, tk.txs(400.0), tk.muted_fg, hide)
                .truncate()
                .mt(tk.r(0.125)),
        )
}

/// `NodeOperatorHero`.
fn hero(app: &App, tk: &Tk, c: &Conn, n: &Node, busy: bool, w: f32) -> El {
    let hide = hidden(app);
    // Valued in the book's currency at its market rate, like the wallet detail hero.
    let (cur, price) = crate::wallets::detail::fiat(app);
    let active = n.active_count();
    let cap = n.total_capacity_sat.max(0.0);
    let local_pct = if cap > 0.0 {
        (n.total_local_balance_sat / cap * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let remote_pct = if cap > 0.0 {
        (n.total_remote_balance_sat / cap * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let inbound = if cap > 0.0 {
        n.total_remote_balance_sat / cap
    } else {
        0.0
    };
    let outbound = if cap > 0.0 {
        n.total_local_balance_sat / cap
    } else {
        0.0
    };
    let signal = if active == 0 {
        "noActiveChannels"
    } else if inbound < 0.15 {
        "lowReceive"
    } else if outbound < 0.15 {
        "lowSend"
    } else {
        "balanced"
    };
    let tone = match signal {
        "balanced" => m::emerald(tk),
        "noActiveChannels" => m::red(tk),
        _ => m::amber(tk),
    };
    let net_sat = n.routing.as_ref().map(|r| r.net_profit_sat).unwrap_or(0.0);
    // Badge secondary `rounded-md`: `px-2 py-0.5 text-xs font-medium`, secondary fill.
    let operator = row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .fill(tk.muted)
        .border(1.0, Color::TRANSPARENT)
        .border_layout()
        .child(
            text(t("connections:node.hero.operatorView"))
                .font(tk.txs(500.0))
                .color(tk.fg),
        );
    let signal_pill = row()
        .px(tk.r(0.5))
        .py(tk.r(0.25))
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .fill(tone.fill)
        .border(1.0, tone.ring)
        .border_layout()
        .child(ic(&I::GAUGE, tk.r(0.875), tone.ink))
        .child(
            text(t(&format!("connections:node.hero.signal.{signal}")))
                .font(tk.txs(500.0))
                .color(tone.ink),
        );
    let line = tr(
        "connections:node.hero.snapshotLine",
        &[
            ("channels", &m::count(active as f64)),
            ("forwards", &m::count(n.forward_total())),
            ("peers", &m::count(n.peer_count)),
        ],
    );
    let badges = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(operator)
        .child(signal_pill)
        .child(text(line).font(tk.txs(400.0)).color(tk.muted_fg));
    let net_tone = if net_sat > 0.0 {
        m::ink_emerald(tk)
    } else if net_sat < 0.0 {
        m::ink_red(tk)
    } else {
        tk.muted_fg
    };
    let stats = vec![
        hero_stat(
            tk,
            &I::ZAP,
            &t("connections:node.hero.canSend"),
            m::sat(n.total_local_balance_sat),
            m::fiat(n.total_local_balance_sat / 1e8 * price, &cur),
            m::ink_emerald(tk),
            hide,
        ),
        hero_stat(
            tk,
            &I::CABLE,
            &t("connections:node.hero.canReceive"),
            m::sat(n.total_remote_balance_sat),
            t("connections:node.hero.remoteCapacity"),
            m::ink_sky(tk),
            hide,
        ),
        hero_stat(
            tk,
            &I::COINS,
            &t("connections:node.hero.onchainReserve"),
            m::sat(n.onchain_balance_sat),
            m::fiat(n.onchain_balance_sat / 1e8 * price, &cur),
            tk.fg,
            hide,
        ),
        hero_stat(
            tk,
            &I::TRENDING_UP,
            &t("connections:node.hero.netRouting"),
            m::sat_signed(net_sat),
            m::fiat(net_sat / 1e8 * price, &cur),
            net_tone,
            hide,
        ),
    ];
    let cols = if w >= 1536.0 {
        4
    } else if w >= 768.0 {
        2
    } else {
        1
    };
    let share = |pct: f32, ink: Color, key: &str| {
        let s = format!("{}% {}", pct.round(), t(key));
        if hide {
            sens(s, tk.txs(400.0).features("'tnum' 1"), tk.muted_fg, true)
        } else {
            row()
                .child(
                    text(format!("{}%", pct.round()))
                        .font(tk.txs(400.0).features("'tnum' 1"))
                        .color(ink),
                )
                .child(
                    text(format!(" {}", t(key)))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                )
        }
    };
    let liquidity = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(bar(
            tk,
            tk.r(0.75),
            local_pct as f32,
            remote_pct as f32,
            tr(
                "connections:node.hero.liquidityAria",
                &[
                    ("local", &m::count(n.total_local_balance_sat)),
                    ("remote", &m::count(n.total_remote_balance_sat)),
                ],
            ),
        ))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .justify_between()
                .flex_wrap()
                .child(share(
                    local_pct as f32,
                    m::ink_emerald(tk),
                    "connections:node.hero.localShare",
                ))
                .child(share(
                    remote_pct as f32,
                    m::ink_sky(tk),
                    "connections:node.hero.remoteShare",
                ))
                .child(sens(
                    tr("connections:node.hero.capacity", &[("value", &m::sat(cap))]),
                    tk.txs(400.0).features("'tnum' 1"),
                    tk.muted_fg,
                    hide,
                )),
        );
    let left = col()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(1.0))
        .child(badges)
        .child(grid(tk, cols, stats))
        .child(liquidity);
    let actions = col()
        .gap(tk.r(0.5))
        .justify_center()
        .child(refresh_button(
            app,
            tk,
            "node-hero-refresh",
            Variant::Default,
            c,
            busy,
            false,
            None,
        ))
        .child(link_button(
            tk,
            "node-hero-transactions",
            &I::RECEIPT_TEXT,
            &t("connections:node.hero.viewTransactions"),
            transactions_route(c),
        ))
        .child(link_button(
            tk,
            "node-hero-reports",
            &I::ARROW_RIGHT,
            &t("connections:node.hero.openReports"),
            "/reports".into(),
        ));
    // `CardContent grid gap-4 px-4 py-4 xl:grid-cols-[minmax(0,1fr)_220px]` inside the card's py-5.
    let content = if w >= 1280.0 {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_stretch()
            .child(left)
            .child(actions.w(220.0))
    } else {
        col()
            .w_full()
            .gap(tk.r(1.0))
            .child(left)
            .child(actions.items_start())
    };
    surface(tk)
        .w_full()
        .py(tk.card_pad())
        .child(content.px(tk.r(1.0)).py(tk.r(1.0)))
}

fn tab_label(tab: Tab) -> String {
    t(&format!("connections:node.tabs.{}", tab.key()))
}

/// `TabsList className="grid h-auto w-full grid-cols-2 gap-1 p-1 sm:grid-cols-5"`.
fn tabs(tk: &Tk, sm: bool, current: Tab) -> El {
    let mut items = Vec::new();
    for tab in Tab::ALL {
        let active = tab == current;
        let (fill, border) = if !active {
            (Color::TRANSPARENT, Color::TRANSPARENT)
        } else if tk.dark {
            (tk.input.alpha(tk.input.a * 0.3), tk.input)
        } else {
            (tk.background, Color::TRANSPARENT)
        };
        let ink = if active || !tk.dark {
            tk.fg
        } else {
            tk.muted_fg
        };
        let mut b = row()
            .key(format!("node-tab-{}", tab.key()))
            .h(tk.r(1.75) + 2.0)
            .px(tk.r(0.5))
            .center()
            .radius(tk.r(0.375))
            .fill(fill)
            .border(1.0, border)
            .border_layout()
            .role("tab")
            .checked(active)
            .label(tab_label(tab))
            .focusable(true)
            .focus_selected(active)
            .on_press(move |a: &mut App| {
                a.wallets.node.tab = tab;
                a.wallets.node.open_channel = None;
            })
            .child(text(tab_label(tab)).font(tk.tsm(500.0)).color(ink));
        if active {
            b = b.box_shadow(Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..Default::default()
            });
        }
        items.push(b);
    }
    let cols = if sm { 5 } else { 2 };
    let mut list = col()
        .key("node-tabs")
        .w_full()
        .padding(tk.r(0.25))
        .gap(tk.r(0.25))
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .role("tablist")
        .label(
            Tab::ALL
                .iter()
                .map(|t| tab_label(*t))
                .collect::<Vec<_>>()
                .join(" "),
        )
        .focus_group();
    let mut it = items.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row().w_full().gap(tk.r(0.25));
        for _ in 0..cols {
            r = match it.next() {
                Some(e) => r.child(e.grow(1.0).min_w(0.0)),
                None => r.child(el().grow(1.0)),
            };
        }
        list = list.child(r);
    }
    list
}

/// `MetricCard` (`Card gap-2.5 py-4`, `CardContent space-y-2 px-4`).
fn metric_card(tk: &Tk, glyph: &'static Icon, label: &str, value: El, detail: String) -> El {
    surface(tk).min_w(0.0).py(tk.r(1.0)).child(
        col()
            .w_full()
            .px(tk.r(1.0))
            .gap(tk.r(0.5))
            .child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(ic(glyph, tk.r(1.0), tk.muted_fg))
                    .child(
                        text(label.to_owned())
                            .font(tk.txs(500.0))
                            .color(tk.muted_fg),
                    ),
            )
            .child(value)
            .child(text(detail).font(tk.txs(400.0)).color(tk.muted_fg).wrap()),
    )
}

/// `NodeMetrics`.
fn metrics(app: &App, tk: &Tk, n: &Node, w: f32) -> El {
    let hide = hidden(app);
    // Valued in the book's currency at its market rate, like the wallet detail hero.
    let (cur, price) = crate::wallets::detail::fiat(app);
    let value_font = tk
        .txl(600.0)
        .tracking(-0.025 * tk.r(1.25))
        .features("'tnum' 1");
    let local_btc = n.total_local_balance_sat / 1e8;
    let onchain_btc = n.onchain_balance_sat / 1e8;
    let total_ln = n.total_local_balance_sat + n.total_remote_balance_sat;
    let items = vec![
        metric_card(
            tk,
            &I::ZAP,
            &t("connections:node.metrics.localBalance"),
            sens(m::btc(local_btc), value_font, tk.fg, hide),
            tr(
                "connections:node.metrics.localBalanceDetail",
                &[
                    ("sat", &m::count(n.total_local_balance_sat)),
                    ("eur", &m::fiat(local_btc * price, &cur)),
                ],
            ),
        ),
        metric_card(
            tk,
            &I::CABLE,
            &t("connections:node.metrics.inboundLiquidity"),
            sens(m::sat(n.total_remote_balance_sat), value_font, tk.fg, hide),
            tr(
                "connections:node.metrics.totalCapacity",
                &[("capacity", &m::sat(n.total_capacity_sat))],
            ),
        ),
        metric_card(
            tk,
            &I::SERVER,
            &t("connections:node.metrics.channels"),
            text(format!("{} / {}", n.active_count(), n.channels.len()))
                .font(value_font)
                .color(tk.fg),
            tr(
                "connections:node.metrics.channelsDetail",
                &[
                    ("peers", &format!("{}", n.peer_count)),
                    ("closed", &n.closed_channels.len().to_string()),
                ],
            ),
        ),
        metric_card(
            tk,
            &I::COINS,
            &t("connections:node.metrics.onChain"),
            sens(m::btc(onchain_btc), value_font, tk.fg, hide),
            if total_ln > 0.0 {
                tr(
                    "connections:node.metrics.localInChannels",
                    &[(
                        "percent",
                        &format!("{}", (n.total_local_balance_sat / total_ln * 100.0).round()),
                    )],
                )
            } else {
                t("connections:node.metrics.noChannelLiquidity")
            },
        ),
    ];
    let cols = if w >= 1280.0 {
        4
    } else if w >= 640.0 {
        2
    } else {
        1
    };
    grid(tk, cols, items)
}

/// `NodeActivityCard`.
fn activity_card(tk: &Tk, sm: bool, xl: bool, n: &Node) -> El {
    let paid = n.paid_invoice_count.unwrap_or(0.0);
    let invoices = n.invoice_count.unwrap_or(paid);
    let completed = n.completed_payment_count.unwrap_or(0.0);
    let payments = n.payment_count.unwrap_or(completed);
    let exceptions = n.failed_payment_count.unwrap_or(0.0) + n.expired_invoice_count.unwrap_or(0.0);
    let items = vec![
        compact_stat(
            tk,
            &I::ARROW_DOWN_RIGHT,
            &t("connections:node.activity.invoices"),
            format!("{} / {}", m::count(paid), m::count(invoices)),
            t("connections:node.activity.paid"),
        ),
        compact_stat(
            tk,
            &I::ARROW_UP_RIGHT,
            &t("connections:node.activity.payments"),
            format!("{} / {}", m::count(completed), m::count(payments)),
            t("connections:node.activity.completed"),
        ),
        compact_stat(
            tk,
            &I::REPEAT,
            &t("connections:node.activity.forwards"),
            m::count(n.forward_total()),
            n.routing
                .as_ref()
                .map(|r| r.window_label.clone())
                .unwrap_or_else(|| t("connections:node.activity.snapshotWindow")),
        ),
        compact_stat(
            tk,
            &I::CIRCLE_X,
            &t("connections:node.activity.exceptions"),
            m::count(exceptions),
            t("connections:node.activity.failedExpired"),
        ),
    ];
    let cols = if xl {
        4
    } else if sm {
        2
    } else {
        1
    };
    card(
        tk,
        sm,
        Some(&I::ACTIVITY),
        card_title(tk, sm, &t("connections:node.activity.title")),
        Some(card_desc(tk, t("connections:node.activity.description"))),
        Some(
            col()
                .w_full()
                .px(tk.r(1.0))
                .pt(tk.r(1.0))
                .child(grid(tk, cols, items)),
        ),
    )
}

/// `AccountingReadinessCard`.
fn accounting_card(tk: &Tk, sm: bool, c: &Conn, n: &Node) -> El {
    let imported = c.transaction_count.unwrap_or(0.0);
    let booked = n.paid_invoice_count.unwrap_or(0.0);
    let events = booked
        + n.completed_payment_count.unwrap_or(0.0)
        + n.routing.as_ref().map(|r| r.forward_count).unwrap_or(0.0);
    let items = vec![
        compact_stat(
            tk,
            &I::RECEIPT_TEXT,
            &t("connections:node.accounting.imported"),
            m::count(imported),
            t("connections:node.accounting.transactions"),
        ),
        compact_stat(
            tk,
            &I::SHIELD_CHECK,
            &t("connections:node.accounting.bookedInvoices"),
            m::count(booked),
            t("connections:node.accounting.finalized"),
        ),
        compact_stat(
            tk,
            &I::ACTIVITY,
            &t("connections:node.accounting.nodeEvents"),
            m::count(events),
            t("connections:node.accounting.auditTrail"),
        ),
    ];
    let buttons = row()
        .gap(tk.r(0.5))
        .flex_wrap()
        .child(link_button(
            tk,
            "node-accounting-transactions",
            &I::RECEIPT_TEXT,
            &t("connections:node.accounting.viewTransactions"),
            transactions_route(c),
        ))
        .child(link_button(
            tk,
            "node-accounting-journals",
            &I::CLIPBOARD_LIST,
            &t("connections:node.accounting.openJournals"),
            "/journals".into(),
        ));
    card(
        tk,
        sm,
        Some(&I::CLIPBOARD_LIST),
        card_title(tk, sm, &t("connections:node.accounting.title")),
        Some(card_desc(tk, t("connections:node.accounting.description"))),
        Some(
            col()
                .w_full()
                .px(tk.r(1.0))
                .pt(tk.r(1.0))
                .gap(tk.r(0.75))
                .child(grid(tk, if sm { 3 } else { 1 }, items))
                .child(buttons),
        ),
    )
}

/// `RoutingSummary`.
fn routing_card(app: &App, tk: &Tk, sm: bool, w: f32, r: &Routing) -> El {
    let hide = hidden(app);
    // Valued in the book's currency at its market rate, like the wallet detail hero.
    let (cur, price) = crate::wallets::detail::fiat(app);
    let stat = |label: &str, value: String, detail: Option<String>, tone: Color| {
        let mut c = col()
            .min_w(0.0)
            .gap(tk.r(0.25))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_layout()
            .fill(tk.background)
            .px(tk.r(0.75))
            .py(tk.r(0.625))
            .child(
                text(label.to_uppercase())
                    .font(tk.t2xs(500.0).tracking(0.025 * tk.r(0.694)))
                    .color(tk.muted_fg),
            )
            .child(sens(value, mono(tk, 0.875, 1.25, 600.0), tone, hide));
        if let Some(d) = detail {
            c = c.child(sens(
                d,
                tk.txs(400.0).features("'tnum' 1"),
                tk.muted_fg,
                hide,
            ));
        }
        c
    };
    let profit_tone = if r.net_profit_sat > 0.0 {
        m::ink_emerald(tk)
    } else if r.net_profit_sat < 0.0 {
        m::ink_red(tk)
    } else {
        tk.muted_fg
    };
    let items = vec![
        stat(
            &t("connections:node.routing.revenue"),
            m::sat_signed(r.routing_revenue_sat),
            None,
            m::ink_emerald(tk),
        ),
        stat(
            &t("connections:node.routing.paymentFees"),
            m::sat_signed(-r.payment_cost_sat),
            None,
            m::ink_red(tk),
        ),
        stat(
            &t("connections:node.routing.rebalanceFees"),
            m::sat_signed(-r.rebalance_cost_sat),
            None,
            m::ink_red(tk),
        ),
        stat(
            &t("connections:node.routing.onChainCosts"),
            m::sat_signed(-r.onchain_cost_sat),
            None,
            m::ink_red(tk),
        ),
        stat(
            &t("connections:node.routing.netProfit"),
            m::sat_signed(r.net_profit_sat),
            Some(m::fiat(r.net_profit_sat / 1e8 * price, &cur)),
            profit_tone,
        ),
    ];
    let cols = if w >= 1280.0 {
        5
    } else if sm {
        2
    } else {
        1
    };
    let desc = tr(
        "connections:node.routing.description",
        &[
            ("window", &r.window_label),
            ("forwards", &m::count(r.forward_count)),
            ("payments", &m::count(r.payment_count)),
            ("rebalances", &m::count(r.rebalance_count)),
        ],
    );
    card(
        tk,
        sm,
        Some(&I::TRENDING_UP),
        card_title(tk, sm, &t("connections:node.routing.title")),
        Some(card_desc(tk, desc)),
        Some(
            col()
                .w_full()
                .px(tk.r(1.0))
                .pt(tk.r(1.0))
                .child(grid(tk, cols, items)),
        ),
    )
}

// ---- Tables ----------------------------------------------------------------------------------

/// A table column: relative width (grow) and alignment.
#[derive(Clone, Copy)]
struct ColSpec {
    grow: f32,
    right: bool,
}

/// shadcn's `Table` container (`overflow-x-auto`) around an auto-layout table: the columns
/// share the card's width, and below `least` (about the cells' least widths) the table keeps
/// that width and scrolls sideways.
fn scroll_table(cx: &Cx<App>, tk: &Tk, key: &str, label: String, least: f32, table: El) -> El {
    let fallback = cx
        .layout_size("wallets-node")
        .map(|[w, _]| w - 2.0 * tk.page_pad() - 2.0)
        .unwrap_or(least);
    let w = crate::ui::table::width(key, least, fallback);
    crate::ui::table::hscroll(key, label, least, table.w(w))
}

/// The header row: the table kit's band and labels.
fn thead(tk: &Tk, cols: &[ColSpec], labels: &[String]) -> El {
    let mut r = crate::ui::table::head(tk)
        .h(tk.r(crate::ui::table::HEAD_H))
        .center_y();
    for (c, l) in cols.iter().zip(labels) {
        let mut cell = row()
            .grow(c.grow)
            .w(0.0)
            .min_w(0.0)
            .px(tk.r(0.5))
            .center_y();
        if c.right {
            cell = cell.justify_end();
        }
        r = r.child(cell.child(crate::ui::table::th_text(tk, l.clone())));
    }
    r
}

/// A body row (the table kit's rule and hover), cells `p-2 align-top`.
fn trow(tk: &Tk, cols: &[ColSpec], cells: Vec<El>, last: bool) -> El {
    let mut r = crate::ui::table::tr(tk, last).items_start();
    for (c, e) in cols.iter().zip(cells) {
        let mut cell = col().grow(c.grow).w(0.0).min_w(0.0).padding(tk.r(0.5));
        if c.right {
            cell = cell.items_end();
        }
        r = r.child(cell.child(e));
    }
    r.role("row")
}

/// `ChannelEconomicsCard`.
fn economics_card(app: &App, cx: &Cx<App>, tk: &Tk, sm: bool, channels: &[Channel]) -> El {
    let hide = hidden(app);
    let mut rows: Vec<&Channel> = channels.iter().collect();
    rows.sort_by(|a, b| {
        b.earned_routing_sat
            .unwrap_or(0.0)
            .total_cmp(&a.earned_routing_sat.unwrap_or(0.0))
    });
    let cols = [
        ColSpec {
            grow: 2.2,
            right: false,
        },
        ColSpec {
            grow: 1.0,
            right: true,
        },
        ColSpec {
            grow: 1.0,
            right: true,
        },
        ColSpec {
            grow: 1.0,
            right: true,
        },
        ColSpec {
            grow: 1.0,
            right: true,
        },
    ];
    let labels = ["peer", "forwards", "earned", "openCost", "state"]
        .map(|k| t(&format!("connections:node.channelEconomics.{k}")));
    let mut table = col()
        .w_full()
        .role("table")
        .child(thead(tk, &cols, &labels));
    if rows.is_empty() {
        table = table.child(
            row().w_full().padding(tk.r(0.5)).justify_center().child(
                text(t("connections:node.channelEconomics.empty"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    let n = rows.len();
    for (i, ch) in rows.into_iter().enumerate() {
        let earned = ch.earned_routing_sat.unwrap_or(0.0);
        let covers = earned >= m::DEFAULT_OPEN_COST_SAT;
        let peer = col()
            .min_w(0.0)
            .child(
                text(ch.peer_alias.clone())
                    .font(tk.tsm(500.0))
                    .color(tk.fg)
                    .truncate(),
            )
            .child(
                text(ch.short_channel_id.clone().unwrap_or_else(|| ch.id.clone()))
                    .font(if sm {
                        mono(tk, 0.75, 1.0, 400.0)
                    } else {
                        mono(tk, 0.694, 1.0, 400.0)
                    })
                    .color(tk.muted_fg)
                    .truncate(),
            );
        let cells = vec![
            peer,
            text(m::count(ch.forward_count.unwrap_or(0.0)))
                .font(mono(tk, 0.875, 1.25, 400.0))
                .color(tk.fg),
            sens(m::sat(earned), mono(tk, 0.875, 1.25, 400.0), tk.fg, hide),
            ring_pill(
                tk,
                if covers {
                    m::emerald(tk)
                } else {
                    m::neutral(tk)
                },
                None,
                t(if covers {
                    "connections:node.channelEconomics.covered"
                } else {
                    "connections:node.channelEconomics.notYet"
                }),
            ),
            ring_pill(
                tk,
                m::state_tone(tk, &ch.state),
                None,
                m::state_label(&ch.state),
            ),
        ];
        table = table.child(trow(tk, &cols, cells, i + 1 == n));
    }
    let desc = tr(
        "connections:node.channelEconomics.description",
        &[("cost", &m::count(m::DEFAULT_OPEN_COST_SAT))],
    );
    let table = scroll_table(
        cx,
        tk,
        "node-economics-table",
        t("connections:node.channelEconomics.title"),
        tk.r(34.0),
        table,
    );
    card(
        tk,
        sm,
        Some(&I::TRENDING_UP),
        card_title(tk, sm, &t("connections:node.channelEconomics.title")),
        Some(card_desc(tk, desc)),
        Some(table),
    )
}

/// `ChannelsCard`.
fn channels_card(app: &App, cx: &Cx<App>, tk: &Tk, sm: bool, n: &Node, layer: &mut Vec<El>) -> El {
    let hide = hidden(app);
    let closed_only = n.channels.is_empty() && !n.closed_channels.is_empty();
    let show_closed = app.wallets.node.show_closed.unwrap_or(closed_only);
    let mut sorted: Vec<&Channel> = n.channels.iter().collect();
    sorted.sort_by(|a, b| b.capacity_sat.total_cmp(&a.capacity_sat));
    let mut closed: Vec<&Channel> = n.closed_channels.iter().collect();
    closed.sort_by(|a, b| b.capacity_sat.total_cmp(&a.capacity_sat));
    let visible: Vec<&Channel> = if show_closed {
        sorted.iter().chain(closed.iter()).copied().collect()
    } else {
        sorted.clone()
    };
    let mut title = row()
        .gap(tk.r(0.5))
        .center_y()
        .child(card_title(tk, sm, &t("connections:node.channels.title")))
        .child(count_chip(tk, sm, n.channels.len()));
    let _ = &mut title;
    let mut head = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .flex_wrap()
        .child(title);
    if !closed.is_empty() {
        let v = m::count(closed.len() as f64);
        let label = if show_closed {
            tr("connections:node.channels.hideClosed", &[("value", &v)])
        } else {
            tr("connections:node.channels.showClosed", &[("value", &v)])
        };
        // `variant="ghost" size="sm" className="text-xs"`.
        let b = row()
            .key("node-toggle-closed")
            .h(tk.r(2.0))
            .px(tk.r(0.75))
            .center()
            .radius(tk.r(0.375))
            .hover(if tk.dark {
                tk.accent.alpha(0.5)
            } else {
                tk.accent
            })
            .role("button")
            .label(label.clone())
            .focusable(true)
            .on_press(move |a: &mut App| a.wallets.node.show_closed = Some(!show_closed))
            .child(text(label).font(tk.txs(500.0)).color(tk.fg));
        head = head.child(b);
    }
    let cols = [
        ColSpec {
            grow: 2.0,
            right: false,
        },
        ColSpec {
            grow: 2.4,
            right: false,
        },
        ColSpec {
            grow: 0.9,
            right: true,
        },
        ColSpec {
            grow: 1.0,
            right: true,
        },
        ColSpec {
            grow: 0.9,
            right: true,
        },
    ];
    let body = if visible.is_empty() {
        col().w_full().px(tk.card_pad()).py(tk.r(2.0)).child(
            text(t("connections:node.channels.empty"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        )
    } else {
        let labels = ["peer", "balance", "capacity", "feePolicy", "state"]
            .map(|k| t(&format!("connections:node.channels.{k}")));
        let mut table = col()
            .w_full()
            .role("table")
            .child(thead(tk, &cols, &labels));
        let len = visible.len();
        for (i, ch) in visible.iter().enumerate() {
            table = table.child(channel_row(
                tk,
                sm,
                hide,
                ch,
                n.total_capacity_sat,
                &cols,
                i + 1 == len,
            ));
        }
        scroll_table(
            cx,
            tk,
            "node-channels-table",
            t("connections:node.channels.title"),
            tk.r(42.0),
            table,
        )
    };
    // The card: header row (title + toggle) and description, then the flush table.
    let header = col()
        .w_full()
        .px(tk.r(1.0))
        .pb(tk.card_pad())
        .gap(tk.r(0.375))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(head)
        .child(card_desc(tk, t("connections:node.channels.description")));
    if let Some(id) = &app.wallets.node.open_channel
        && let Some(ch) = sorted.iter().chain(closed.iter()).find(|c| &c.id == id)
    {
        channel_sheet(app, tk, ch, n.total_capacity_sat, layer);
    }
    surface(tk)
        .w_full()
        .min_w(0.0)
        .py(tk.card_pad())
        .gap(tk.r(1.0))
        .child(header)
        .child(body)
}

/// `ChannelRow`: the whole row opens the sheet; the in-cell button names it for keyboards.
fn channel_row(
    tk: &Tk,
    sm: bool,
    hide: bool,
    ch: &Channel,
    total_cap: f64,
    cols: &[ColSpec],
    last: bool,
) -> El {
    let small = if sm { tk.txs(400.0) } else { tk.t2xs(400.0) };
    let small_mono = if sm {
        mono(tk, 0.75, 1.0, 400.0)
    } else {
        mono(tk, 0.694, 1.0, 400.0)
    };
    let capacity = ch.capacity_sat.max(1.0);
    let local_pct = (ch.local_balance_sat / capacity * 100.0).clamp(0.0, 100.0) as f32;
    let remote_pct = (ch.remote_balance_sat / capacity * 100.0).clamp(0.0, 100.0) as f32;
    let share = if total_cap > 0.0 {
        (ch.capacity_sat / total_cap * 100.0).round()
    } else {
        0.0
    };
    let show_share = ch.state == "active" || ch.state == "inactive";
    let id = ch.id.clone();
    let mut name = row().gap(tk.r(0.375)).center_y().min_w(0.0).child(
        text(ch.peer_alias.clone())
            .font(tk.tsm(500.0))
            .color(tk.fg)
            .truncate(),
    );
    if ch.is_private {
        name = name.child(outline_badge(tk, t("connections:node.channels.private")));
    }
    if !ch.is_initiator {
        name = name.child(outline_badge(
            tk,
            t("connections:node.channels.remoteOpened"),
        ));
    }
    let mut ident = ch
        .peer_pubkey
        .as_deref()
        .map(m::pubkey)
        .unwrap_or_else(|| t("connections:node.channels.unannouncedPeer"));
    if let Some(s) = &ch.short_channel_id {
        ident.push_str(&format!(" · {s}"));
    }
    let open_id = id.clone();
    let peer = col()
        .key(format!("node-channel-{id}"))
        .min_w(0.0)
        .gap(tk.r(0.25))
        .radius(tk.r(0.375))
        .role("button")
        .label(tr(
            "connections:node.channels.openDetail",
            &[("peer", &ch.peer_alias)],
        ))
        .focusable(true)
        .on_press(move |a: &mut App| a.wallets.node.open_channel = Some(open_id.clone()))
        .child(name)
        .child(sens(ident, small_mono, tk.muted_fg, hide).truncate());
    let amounts = if hide {
        row().w_full().child(sens(
            format!(
                "{} {} {} {}",
                m::sat(ch.local_balance_sat),
                t("connections:node.channels.localLabel"),
                m::sat(ch.remote_balance_sat),
                t("connections:node.channels.remoteLabel")
            ),
            small_mono,
            tk.muted_fg,
            true,
        ))
    } else {
        let part = |v: f64, ink: Color, key: &str| {
            row()
                .child(text(m::sat(v)).font(small_mono).color(ink))
                .child(
                    text(format!(" {}", t(key)))
                        .font(small_mono)
                        .color(tk.muted_fg),
                )
        };
        row()
            .w_full()
            .justify_between()
            .child(part(
                ch.local_balance_sat,
                m::ink_emerald(tk),
                "connections:node.channels.localLabel",
            ))
            .child(part(
                ch.remote_balance_sat,
                m::ink_sky(tk),
                "connections:node.channels.remoteLabel",
            ))
    };
    let balance = col()
        .w_full()
        .gap(tk.r(0.25))
        .child(bar(
            tk,
            tk.r(0.5),
            local_pct,
            remote_pct,
            tr(
                "connections:node.channels.balanceAria",
                &[
                    ("local", &m::count(ch.local_balance_sat)),
                    ("capacity", &m::count(ch.capacity_sat)),
                ],
            ),
        ))
        .child(amounts);
    let mut cap = col().items_end().child(sens(
        m::sat(ch.capacity_sat),
        mono(tk, 0.875, 1.25, 400.0),
        tk.fg,
        hide,
    ));
    if show_share {
        cap = cap.child(
            text(tr(
                "connections:node.channels.shareOfNode",
                &[("percent", &format!("{share}"))],
            ))
            .font(small)
            .color(tk.muted_fg),
        );
    }
    let mut fee = col().items_end();
    if ch.base_fee_msat.is_none() && ch.fee_rate_ppm.is_none() {
        fee = fee.child(text("—").font(tk.txs(400.0)).color(tk.muted_fg));
    } else {
        let ppm = ch.fee_rate_ppm.map(m::count).unwrap_or_else(|| "—".into());
        fee = fee
            .child(
                text(tr("connections:node.channels.ppm", &[("value", &ppm)]))
                    .font(mono(tk, 0.875, 1.25, 400.0))
                    .color(tk.fg),
            )
            .child(
                text(tr(
                    "connections:node.channels.baseMsat",
                    &[("value", &m::count(ch.base_fee_msat.unwrap_or(0.0)))],
                ))
                .font(small)
                .color(tk.muted_fg),
            );
    }
    if let Some(fc) = ch.forward_count.filter(|v| *v != 0.0) {
        fee = fee.child(
            text(tr(
                "connections:node.channels.forwardsEarned",
                &[
                    ("value", &m::count(fc)),
                    ("earned", &m::sat(ch.earned_routing_sat.unwrap_or(0.0))),
                ],
            ))
            .font(small)
            .color(tk.muted_fg)
            .mt(tk.r(0.125)),
        );
    }
    // A chip only for a channel that needs a look; an active one just says so.
    let mut state = col().items_end().child(if ch.state == "active" {
        text(m::state_label(&ch.state))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
    } else {
        ring_pill(
            tk,
            m::state_tone(tk, &ch.state),
            None,
            m::state_label(&ch.state),
        )
    });
    if let Some(closed) = &ch.closed_at {
        state = state.child(
            text(format!(
                "{} · {}",
                m::close_kind_label(ch.close_kind.as_deref()),
                m::short_date(Some(closed))
            ))
            .font(small)
            .color(tk.muted_fg)
            .mt(tk.r(0.125)),
        );
    }
    let row_id = id.clone();
    trow(tk, cols, vec![peer, balance, cap, fee, state], last)
        .key(format!("node-channel-row-{id}"))
        .hover(tk.muted.alpha(0.45))
        .on_press(move |a: &mut App| a.wallets.node.open_channel = Some(row_id.clone()))
}

/// `ForwardsCard`.
fn forwards_card(app: &App, cx: &Cx<App>, tk: &Tk, sm: bool, forwards: &[Forward]) -> El {
    let hide = hidden(app);
    let mut sorted: Vec<&Forward> = forwards.iter().collect();
    sorted.sort_by(|a, b| {
        crate::format::parse_ts(&b.occurred_at)
            .unwrap_or(0.0)
            .total_cmp(&crate::format::parse_ts(&a.occurred_at).unwrap_or(0.0))
    });
    let cols = [
        ColSpec {
            grow: 1.0,
            right: false,
        },
        ColSpec {
            grow: 2.4,
            right: false,
        },
        ColSpec {
            grow: 1.0,
            right: true,
        },
        ColSpec {
            grow: 0.9,
            right: true,
        },
        ColSpec {
            grow: 1.1,
            right: true,
        },
    ];
    let labels = ["time", "route", "amount", "fee", "status"]
        .map(|k| t(&format!("connections:node.forwards.{k}")));
    let small = if sm { tk.txs(400.0) } else { tk.t2xs(400.0) };
    let mut table = col()
        .w_full()
        .role("table")
        .child(thead(tk, &cols, &labels));
    let n = sorted.len();
    for (i, f) in sorted.iter().enumerate() {
        let mut time = col().child(
            text(m::short_date(Some(&f.occurred_at)))
                .font(tk.tsm(400.0))
                .color(tk.fg),
        );
        if let Some(r) = m::relative(Some(&f.occurred_at)) {
            time = time.child(text(r).font(small).color(tk.muted_fg));
        }
        let leg = |glyph: &'static Icon, ink: Color, alias: &str, scid: &Option<String>| {
            let mut r = row()
                .gap(tk.r(0.375))
                .center_y()
                .min_w(0.0)
                .child(ic(glyph, tk.r(0.75), ink))
                .child(
                    text(alias.to_owned())
                        .font(tk.txs(500.0))
                        .color(tk.fg)
                        .truncate(),
                );
            if let Some(s) = scid {
                r = r.child(sens(
                    s.clone(),
                    mono(tk, 0.694, 1.0, 400.0),
                    tk.muted_fg,
                    hide,
                ));
            }
            r
        };
        let route = col()
            .min_w(0.0)
            .gap(tk.r(0.125))
            .child(leg(
                &I::ARROW_DOWN_RIGHT,
                m::arrow_in(tk),
                &f.in_peer_alias,
                &f.in_short_channel_id,
            ))
            .child(leg(
                &I::ARROW_UP_RIGHT,
                m::arrow_out(tk),
                &f.out_peer_alias,
                &f.out_short_channel_id,
            ));
        let amount = sens(
            m::sat((f.amount_in_msat / 1000.0).round()),
            mono(tk, 0.875, 1.25, 400.0),
            tk.fg,
            hide,
        );
        let fee_text = if f.status == "settled" && f.fee_msat > 0.0 {
            if f.fee_msat < 1000.0 {
                tr(
                    "connections:node.forwards.feeMsat",
                    &[("value", &m::count(f.fee_msat))],
                )
            } else {
                tr(
                    "connections:node.forwards.feeSat",
                    &[("value", &m::count((f.fee_msat / 1000.0).round()))],
                )
            }
        } else {
            "—".into()
        };
        let fee = sens(
            fee_text,
            mono(tk, 0.875, 1.25, 400.0),
            if f.status == "settled" {
                m::ink_emerald(tk)
            } else {
                tk.muted_fg
            },
            hide,
        );
        let glyph: &'static Icon = match f.status.as_str() {
            "settled" => &I::ARROW_DOWN_RIGHT,
            "failed" => &I::CIRCLE_X,
            _ => &I::ARROW_UP_RIGHT,
        };
        let mut status = col().items_end().child(ring_pill(
            tk,
            m::forward_tone(tk, &f.status),
            Some(glyph),
            m::forward_status_label(&f.status),
        ));
        if let Some(r) = &f.failure_reason {
            status = status.child(
                text(r.clone())
                    .font(small)
                    .color(tk.muted_fg)
                    .mt(tk.r(0.125)),
            );
        }
        table = table.child(trow(
            tk,
            &cols,
            vec![time, route, amount, fee, status],
            i + 1 == n,
        ));
    }
    let title = row()
        .gap(tk.r(0.5))
        .center_y()
        .child(card_title(tk, sm, &t("connections:node.forwards.title")))
        .child(count_chip(tk, sm, n));
    let table = scroll_table(
        cx,
        tk,
        "node-forwards-table",
        t("connections:node.forwards.title"),
        tk.r(38.0),
        table,
    );
    card(
        tk,
        sm,
        Some(&I::REPEAT),
        title,
        Some(card_desc(tk, t("connections:node.forwards.description"))),
        Some(table),
    )
}

/// `NodeDetailsCard`: Node identity, then Sync status.
fn details_cards(app: &App, tk: &Tk, sm: bool, c: &Conn, n: &Node) -> El {
    let hide = hidden(app);
    let mut id_rows = col().w_full().px(tk.r(1.0)).pt(tk.r(1.0)).gap(tk.r(0.75));
    id_rows = id_rows
        .child(detail_row(
            app,
            tk,
            "alias",
            &t("connections:node.identity.alias"),
            dv(tk, n.alias.clone(), false),
            None,
        ))
        .child(detail_row(
            app,
            tk,
            "pubkey",
            &t("connections:node.identity.publicKey"),
            dv_sens(tk, n.pubkey.clone(), true, hide),
            None,
        ))
        .child(detail_row(
            app,
            tk,
            "network",
            &t("connections:node.identity.network"),
            dv(tk, n.network.clone(), false),
            None,
        ));
    if let Some(v) = &n.implementation_version {
        id_rows = id_rows.child(detail_row(
            app,
            tk,
            "impl",
            &t("connections:node.identity.implementation"),
            dv(tk, v.clone(), false),
            None,
        ));
    }
    if let Some(h) = n.block_height {
        id_rows = id_rows.child(detail_row(
            app,
            tk,
            "height",
            &t("connections:node.identity.blockHeight"),
            dv(tk, m::count(h), true),
            None,
        ));
    }
    id_rows = id_rows
        .child(detail_row(
            app,
            tk,
            "kind",
            &t("connections:node.identity.connectionKind"),
            dv(tk, c.kind.clone(), true),
            None,
        ))
        .child(detail_row(
            app,
            tk,
            "kassiber-id",
            &t("connections:node.identity.kassiberId"),
            dv(tk, c.id.clone(), true),
            Some(c.id.clone()),
        ));
    let identity = card(
        tk,
        sm,
        Some(&I::EARTH),
        card_title(tk, sm, &t("connections:node.identity.title")),
        Some(card_desc(tk, t("connections:node.identity.description"))),
        Some(id_rows),
    );
    let mut sync_rows = col().w_full().px(tk.r(1.0)).pt(tk.r(1.0)).gap(tk.r(0.75));
    sync_rows = sync_rows
        .child(detail_row(
            app,
            tk,
            "status",
            &t("connections:node.syncStatus.status"),
            dv(tk, c.status.clone(), true),
            None,
        ))
        .child(detail_row(
            app,
            tk,
            "last",
            &t("connections:node.syncStatus.lastRefresh"),
            dv(tk, c.last.clone(), false),
            None,
        ));
    if let Some(v) = &c.last_sync_at {
        sync_rows = sync_rows.child(detail_row(
            app,
            tk,
            "last-sync",
            &t("connections:node.syncStatus.lastSyncAt"),
            dv(tk, m::short_date(Some(v)), true),
            None,
        ));
    }
    if let Some(v) = &c.last_transaction_at {
        sync_rows = sync_rows.child(detail_row(
            app,
            tk,
            "last-tx",
            &t("connections:node.syncStatus.lastTransaction"),
            dv(tk, m::short_date(Some(v)), true),
            None,
        ));
    }
    if let Some(v) = c.transaction_count {
        sync_rows = sync_rows.child(detail_row(
            app,
            tk,
            "imported",
            &t("connections:node.syncStatus.importedTransactions"),
            dv(tk, m::count(v), false),
            None,
        ));
    }
    let sync = card(
        tk,
        sm,
        None,
        card_title(tk, sm, &t("connections:node.syncStatus.title")),
        Some(card_desc(tk, t("connections:node.syncStatus.description"))),
        Some(sync_rows),
    );
    col().w_full().gap(tk.r(0.75)).child(identity).child(sync)
}

// ---- Channel sheet ---------------------------------------------------------------------------

fn sheet_heading(tk: &Tk, s: &str) -> El {
    text(s.to_uppercase())
        .font(tk.t2xs(500.0).tracking(0.025 * tk.r(0.694)))
        .color(tk.muted_fg)
        .role("heading")
}

/// `ChannelDetailSheet` / `ChannelDetailBody`: the right sheet (`sm:max-w-md`, popover fill,
/// `border-l`) under the title bar, over a `bg-background/60 backdrop-blur-xs` overlay.
fn channel_sheet(app: &App, tk: &Tk, ch: &Channel, total_cap: f64, layer: &mut Vec<El>) {
    let hide = hidden(app);
    let close = |a: &mut App| a.wallets.node.open_channel = None;
    let capacity = ch.capacity_sat.max(1.0);
    let local_pct = (ch.local_balance_sat / capacity * 100.0) as f32;
    let remote_pct = (ch.remote_balance_sat / capacity * 100.0) as f32;
    let show_share = ch.state == "active" || ch.state == "inactive";
    let share =
        (show_share && total_cap > 0.0).then(|| (ch.capacity_sat / total_cap * 100.0).round());
    let tone = m::state_tone(tk, &ch.state);
    let title = row()
        .gap(tk.r(0.5))
        .center_y()
        .min_w(0.0)
        .role("heading")
        .child(
            row()
                .size(tk.r(1.75), tk.r(1.75))
                .center()
                .radius(tk.r(0.375))
                .fill(tone.fill)
                .border(1.0, tone.ring)
                .border_layout()
                .a11y_hidden()
                .child(ic(&I::ZAP, tk.r(0.875), tone.ink)),
        )
        .child(
            text(ch.peer_alias.clone())
                .font(tk.tbase(600.0))
                .color(tk.fg)
                .truncate(),
        );
    let mut desc = format!(
        "{} · {}",
        m::state_label(&ch.state),
        t(if ch.is_initiator {
            "connections:node.channelDetail.locallyInitiated"
        } else {
            "connections:node.channelDetail.remoteOpened"
        })
    );
    if ch.is_private {
        desc.push_str(&format!(
            " · {}",
            t("connections:node.channelDetail.private")
        ));
    }
    let head = col()
        .w_full()
        .padding(tk.r(1.0))
        .gap(tk.r(0.375))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(title)
        .child(text(desc).font(tk.tsm(400.0)).color(tk.muted_fg).wrap());

    let amounts = if hide {
        row().w_full().child(sens(
            format!(
                "{}  {}",
                tr(
                    "connections:node.channelDetail.localBalance",
                    &[("value", &m::sat(ch.local_balance_sat))]
                ),
                tr(
                    "connections:node.channelDetail.remoteBalance",
                    &[("value", &m::sat(ch.remote_balance_sat))]
                )
            ),
            mono(tk, 0.75, 1.0, 400.0),
            tk.fg,
            true,
        ))
    } else {
        row()
            .w_full()
            .justify_between()
            .child(
                text(tr(
                    "connections:node.channelDetail.localBalance",
                    &[("value", &m::sat(ch.local_balance_sat))],
                ))
                .font(mono(tk, 0.75, 1.0, 400.0))
                .color(m::ink_emerald(tk)),
            )
            .child(
                text(tr(
                    "connections:node.channelDetail.remoteBalance",
                    &[("value", &m::sat(ch.remote_balance_sat))],
                ))
                .font(mono(tk, 0.75, 1.0, 400.0))
                .color(m::ink_sky(tk)),
            )
    };
    let mut cap_line = row().w_full().justify_between().child(
        text(tr(
            "connections:node.channelDetail.capacity",
            &[("capacity", &m::sat(ch.capacity_sat))],
        ))
        .font(tk.txs(400.0))
        .color(tk.muted_fg),
    );
    if let Some(s) = share {
        cap_line = cap_line.child(
            text(tr(
                "connections:node.channelDetail.shareOfNode",
                &[("percent", &format!("{s}"))],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    let balance = col()
        .w_full()
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background)
        .px(tk.r(0.75))
        .py(tk.r(0.75))
        .child(bar(
            tk,
            tk.r(0.625),
            local_pct,
            remote_pct,
            tr(
                "connections:node.channels.balanceAria",
                &[
                    ("local", &m::count(ch.local_balance_sat)),
                    ("capacity", &m::count(ch.capacity_sat)),
                ],
            ),
        ))
        .child(amounts)
        .child(cap_line);

    let section = |heading: &str, rows: Vec<El>| {
        let mut c = col()
            .w_full()
            .gap(tk.r(0.625))
            .child(sheet_heading(tk, heading));
        for r in rows {
            c = c.child(r);
        }
        c
    };
    let dr = |key: &str, label: &str, value: El, copy: Option<String>| {
        detail_row(app, tk, &format!("sheet-{key}"), label, value, copy)
    };

    let mut peer_rows = vec![dr(
        "alias",
        &t("connections:node.channelDetail.alias"),
        dv(tk, ch.peer_alias.clone(), false),
        None,
    )];
    peer_rows.push(match &ch.peer_pubkey {
        Some(pk) => dr(
            "pubkey",
            &t("connections:node.channelDetail.publicKey"),
            dv_sens(tk, pk.clone(), true, hide),
            None,
        ),
        None => dr(
            "pubkey",
            &t("connections:node.channelDetail.publicKey"),
            text(t("connections:node.channelDetail.publicKeyHidden"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .truncate(),
            None,
        ),
    });
    if ch.is_private {
        peer_rows.push(dr(
            "visibility",
            &t("connections:node.channelDetail.visibility"),
            dv(
                tk,
                t("connections:node.channelDetail.privateVisibility"),
                false,
            ),
            None,
        ));
    }

    let mut chan_rows = vec![match &ch.short_channel_id {
        Some(s) => dr(
            "scid",
            &t("connections:node.channelDetail.shortChannelId"),
            dv(tk, s.clone(), true),
            Some(s.clone()),
        ),
        None => dr(
            "scid",
            &t("connections:node.channelDetail.shortChannelId"),
            dv(
                tk,
                t("connections:node.channelDetail.shortChannelIdPending"),
                false,
            ),
            None,
        ),
    }];
    if let Some(op) = &ch.funding_outpoint {
        let mut v = row().grow(1.0).gap(tk.r(0.25)).center_y().min_w(0.0).child(
            sens(op.clone(), mono(tk, 0.75, 1.0, 400.0), tk.fg, hide)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        );
        if let Some(href) = m::explorer_href(Some(op)) {
            // Opens the system browser on press (the web's `<a target="_blank">`): a user action.
            v = v.child(
                row()
                    .key("node-sheet-explorer")
                    .role("link")
                    .label(t("connections:node.channelDetail.openFundingTx"))
                    .help(t("connections:node.channelDetail.openFundingTx"))
                    .focusable(true)
                    .on_press_tx(move |_: &mut App, tx: &mut Tx| {
                        let _ = tx.open_url(&href);
                    })
                    .child(ic(&I::EXTERNAL_LINK, tk.r(0.75), tk.muted_fg)),
            );
        }
        chan_rows.push(dr(
            "outpoint",
            &t("connections:node.channelDetail.fundingOutpoint"),
            v,
            None,
        ));
    }
    let with_rel = |ts: &str, rel: Option<String>| {
        format!(
            "{}{}",
            m::short_date(Some(ts)),
            rel.map(|r| format!(" · {r}")).unwrap_or_default()
        )
    };
    if let Some(o) = &ch.opened_at {
        chan_rows.push(dr(
            "opened",
            &t("connections:node.channelDetail.opened"),
            dv(tk, with_rel(o, m::relative(Some(o))), true),
            None,
        ));
    }
    if let Some(cl) = &ch.closed_at {
        let kind = ch
            .close_kind
            .as_deref()
            .map(|k| format!(" · {}", m::close_kind_label(Some(k))))
            .unwrap_or_default();
        chan_rows.push(dr(
            "closed",
            &t("connections:node.channelDetail.closed"),
            dv(tk, format!("{}{kind}", m::short_date(Some(cl))), true),
            None,
        ));
    }
    if let Some(la) = &ch.last_activity_at {
        chan_rows.push(dr(
            "activity",
            &t("connections:node.channelDetail.lastActivity"),
            dv(tk, with_rel(la, m::relative(Some(la))), true),
            None,
        ));
    }
    if let Some(hc) = ch.htlc_count {
        chan_rows.push(dr(
            "htlcs",
            &t("connections:node.channelDetail.inFlightHtlcs"),
            dv(tk, m::count(hc), false),
            None,
        ));
    }

    let mut routing_rows = vec![
        dr(
            "fee-rate",
            &t("connections:node.channelDetail.feeRate"),
            dv(
                tk,
                tr(
                    "connections:node.channelDetail.feeRateValue",
                    &[("value", &m::count(ch.fee_rate_ppm.unwrap_or(0.0)))],
                ),
                false,
            ),
            None,
        ),
        dr(
            "base-fee",
            &t("connections:node.channelDetail.baseFee"),
            dv(
                tk,
                tr(
                    "connections:node.channelDetail.baseFeeValue",
                    &[("value", &m::count(ch.base_fee_msat.unwrap_or(0.0)))],
                ),
                false,
            ),
            None,
        ),
    ];
    if let Some(fc) = ch.forward_count {
        routing_rows.push(dr(
            "forwards",
            &t("connections:node.channelDetail.forwardsWindow"),
            dv(tk, m::count(fc), false),
            None,
        ));
    }
    if let Some(e) = ch.earned_routing_sat {
        routing_rows.push(dr(
            "earned",
            &t("connections:node.channelDetail.earnedRouting"),
            dv(tk, m::sat(e), true),
            None,
        ));
    }

    let body = col()
        .w_full()
        .px(tk.r(1.0))
        .pb(tk.r(1.5))
        .gap(tk.r(1.0))
        .child(balance)
        .child(section(
            &t("connections:node.channelDetail.peer"),
            peer_rows,
        ))
        .child(section(
            &t("connections:node.channelDetail.channel"),
            chan_rows,
        ))
        .child(section(
            &t("connections:node.channelDetail.routing"),
            routing_rows,
        ));

    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let width = (win_w - tk.r(3.0)).min(if win_w >= 640.0 { tk.r(28.0) } else { f32::MAX });
    let close_btn = row()
        .key("node-sheet-close")
        .absolute(width - tk.r(1.0) - tk.r(1.0) - 1.0, tk.r(1.0))
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .opacity(0.7)
        .radius(2.0)
        .role("button")
        .label("Close")
        .focusable(true)
        .on_press(close)
        .child(ic(&I::X, tk.r(1.0), tk.fg));
    let rim = if tk.dark {
        Color::WHITE.alpha(0.06)
    } else {
        Color::BLACK.alpha(0.04)
    };
    let panel = col()
        .key("node-channel-sheet")
        .role("dialog")
        .label(ch.peer_alias.clone())
        .w(width)
        .fill(tk.popover)
        .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
        .border_layout()
        .box_shadow(Shadow {
            y: 10.0,
            blur: 15.0,
            spread: -3.0,
            color: Color::BLACK.alpha(0.05),
            ..Default::default()
        })
        .box_shadow(Shadow {
            y: if tk.dark { -1.0 } else { 1.0 },
            color: rim,
            inset: true,
            ..Default::default()
        })
        .transition(crate::shell::sheet_enter(tk))
        .modal(true, close)
        .child(
            col()
                .key("node-sheet-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(col().w_full().gap(tk.r(1.0)).child(head).child(body)),
        )
        .child(close_btn);
    layer.push(
        crate::shell::window_backdrop("node-sheet-overlay", tk.background.alpha(0.6))
            .backdrop_blur(4.0)
            .on_press(close)
            .a11y_hidden(),
    );
    // Its own layer root, pinned right below the title bar, so it slides in and out itself.
    layer.push(panel.pin(
        None,
        Some(crate::shell::window_top_inset()),
        Some(0.0),
        Some(0.0),
    ));
    let _ = MASK;
}
