//! Pieces every Wallets screen draws: `ConnectionStatusPill`, `ConnectionAssetBadge`, and the
//! headless pump helper.

use rune::{Color, Cx, El, Shadow, el, row, text};

use super::conn::{Conn, status_dot, status_style};
use crate::app::App;
use crate::i18n::t;
use crate::logos;
use crate::theme::Tk;

/// `ConnectionStatusPill` (`rounded-md px-1.5 py-0.5 text-2xs font-medium ring-1 ring-inset`);
/// `synced` renders nothing when `hide_when_synced`.
pub fn status_pill(tk: &Tk, status: &str, hide_when_synced: bool) -> Option<El> {
    if hide_when_synced && status == "synced" {
        return None;
    }
    let (fill, ink, ring) = status_style(tk, status);
    let label = t(&format!("chrome:connectionStatus.{status}"));
    Some(
        row()
            .px(tk.r(0.375))
            .py(tk.r(0.125))
            .radius(tk.r(0.375))
            .fill(fill)
            .border(1.0, ring)
            .center_y()
            .child(text(label).font(tk.t2xs(500.0)).color(ink)),
    )
}

/// `ConnectionAssetBadge`: the rail logo in a bordered muted tile (`size-8`, `md` = `size-9`),
/// Lightning kinds on their brand tint, with an optional status dot. Hidden from AT.
pub fn asset_badge(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    c: &Conn,
    status: Option<&str>,
    md: bool,
) -> El {
    let h = |s: &str| rune::color::hex(s).expect("hex");
    let asset = c.asset_label();
    let core_ln = c.kind == "core-ln";
    let lnd = c.kind == "lnd";
    let lightning = core_ln || lnd || c.kind == "nwc" || c.kind == "phoenix";
    let liquid = asset == "LBTC";
    let side = tk.r(if md { 2.25 } else { 2.0 });
    let (fill, border) = if core_ln {
        (
            h("#09090b"),
            if tk.dark {
                h("#3f3f46")
            } else {
                h("#09090b").alpha(0.8)
            },
        )
    } else if lnd {
        if tk.dark {
            (h("#6f4cff").alpha(0.2), h("#8b72ff").alpha(0.4))
        } else {
            (h("#6f4cff").alpha(0.1), h("#6f4cff").alpha(0.3))
        }
    } else if lightning {
        if tk.dark {
            (h("#792EE5").alpha(0.2), h("#9f6af0").alpha(0.35))
        } else {
            (h("#792EE5").alpha(0.1), h("#792EE5").alpha(0.25))
        }
    } else if tk.dark {
        (tk.muted.alpha(0.55), tk.border.alpha(tk.border.a * 0.8))
    } else {
        (tk.muted.alpha(0.6), tk.border.alpha(tk.border.a * 0.7))
    };
    // The web pads and zooms each asset differently (`p-0.5 scale-115`, `p-0 scale-115`,
    // core-ln `scale-150`), so its Liquid disc and Lightning hexagon outgrow the bitcoin coin;
    // every asset's artwork is the bitcoin coin's size here.
    let bytes = if core_ln {
        logos::CORE_LIGHTNING
    } else if lnd {
        logos::LIGHTNING_LABS_SVG
    } else if lightning {
        logos::LIGHTNING
    } else if liquid {
        logos::LIQUID
    } else {
        logos::BITCOIN
    };
    let art = tk.r(if md {
        logos::role::ASSET_BADGE_MD
    } else {
        logos::role::ASSET_BADGE_SM
    });
    let img = logos::fitted(app, cx, logos::Logo::svg(bytes), art, side - 2.0);
    let mut b = row()
        .size(side, side)
        .center()
        .radius(tk.r(0.375))
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: if tk.dark {
                Color::BLACK.alpha(0.2)
            } else {
                h("#09090b").alpha(0.05)
            },
            ..Shadow::default()
        })
        .a11y_hidden()
        .child(img);
    if let Some(st) = status {
        // `absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full ring-2 ring-card`.
        let d = tk.r(0.625);
        b = b.child(
            el().absolute(side - d + tk.r(0.125), side - d + tk.r(0.125))
                .size(d + 4.0, d + 4.0)
                .radius((d + 4.0) / 2.0)
                .fill(tk.card)
                .child(
                    el().absolute(2.0, 2.0)
                        .size(d, d)
                        .radius(d / 2.0)
                        .fill(status_dot(tk, st)),
                ),
        );
    }
    b
}

/// Headless scenes: pump daemon replies until `done` (or 20 s).
pub fn pump_until(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !done(app) && std::time::Instant::now() < deadline {
        app.pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Headless scenes: pump until no request is in flight (or 20 s).
pub fn settle(app: &mut App) {
    pump_until(app, |a| a.pending.is_empty());
}
