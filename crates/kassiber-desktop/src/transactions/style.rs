//! Tailwind v4 colours and the style maps the table uses (components/transactions/model.ts:
//! `transactionStatusStyles`, `transactionFlowStyles`, `pricingSourceStyles`), so badges and
//! chips draw exactly the web's light and dark classes.

use rune::{Color, oklch};

use crate::theme::Tk;

/// A Tailwind v4 palette colour by family and shade.
pub fn tw(family: &str, shade: u16) -> Color {
    let (l, c, h) = match (family, shade) {
        ("emerald", 50) => (0.979, 0.021, 166.113),
        ("emerald", 300) => (0.845, 0.143, 164.978),
        ("emerald", 400) => (0.765, 0.177, 163.223),
        ("emerald", 600) => (0.596, 0.145, 163.225),
        ("emerald", 700) => (0.508, 0.118, 165.612),
        ("emerald", 900) => (0.378, 0.077, 168.94),
        ("amber", 50) => (0.987, 0.022, 95.277),
        ("amber", 400) => (0.828, 0.189, 84.429),
        ("amber", 500) => (0.769, 0.188, 70.08),
        ("amber", 600) => (0.666, 0.179, 58.318),
        ("amber", 700) => (0.555, 0.163, 48.998),
        ("amber", 900) => (0.414, 0.112, 45.904),
        ("red", 50) => (0.971, 0.013, 17.38),
        ("red", 300) => (0.808, 0.114, 19.571),
        ("red", 400) => (0.704, 0.191, 22.216),
        ("red", 600) => (0.577, 0.245, 27.325),
        ("red", 700) => (0.505, 0.213, 27.518),
        ("red", 900) => (0.396, 0.141, 25.723),
        ("sky", 50) => (0.977, 0.013, 236.62),
        ("sky", 300) => (0.828, 0.111, 230.318),
        ("sky", 400) => (0.746, 0.16, 232.661),
        ("sky", 600) => (0.588, 0.158, 241.966),
        ("sky", 700) => (0.5, 0.134, 242.749),
        ("sky", 900) => (0.391, 0.09, 240.876),
        ("blue", 50) => (0.97, 0.014, 254.604),
        ("blue", 400) => (0.707, 0.165, 254.624),
        ("blue", 600) => (0.546, 0.245, 262.881),
        ("blue", 700) => (0.488, 0.243, 264.376),
        ("blue", 900) => (0.379, 0.146, 265.522),
        ("zinc", 50) => (0.985, 0.0, 0.0),
        ("zinc", 300) => (0.871, 0.006, 286.286),
        ("zinc", 500) => (0.552, 0.016, 285.938),
        ("zinc", 700) => (0.37, 0.013, 285.805),
        ("zinc", 800) => (0.274, 0.006, 286.033),
        ("indigo", 50) => (0.962, 0.018, 272.314),
        ("indigo", 300) => (0.785, 0.115, 274.713),
        ("indigo", 400) => (0.673, 0.182, 276.935),
        ("indigo", 600) => (0.511, 0.262, 276.966),
        ("indigo", 700) => (0.457, 0.24, 277.023),
        ("indigo", 900) => (0.359, 0.144, 278.697),
        ("orange", 50) => (0.98, 0.016, 73.684),
        ("orange", 300) => (0.837, 0.128, 66.29),
        ("orange", 400) => (0.75, 0.183, 55.934),
        ("orange", 600) => (0.646, 0.222, 41.116),
        ("orange", 700) => (0.553, 0.195, 38.402),
        ("orange", 900) => (0.408, 0.123, 38.172),
        ("violet", 50) => (0.969, 0.016, 293.756),
        ("violet", 300) => (0.811, 0.111, 293.571),
        ("violet", 400) => (0.702, 0.183, 293.541),
        ("violet", 600) => (0.541, 0.281, 293.009),
        ("violet", 700) => (0.491, 0.27, 292.581),
        ("violet", 900) => (0.38, 0.189, 293.745),
        ("teal", 50) => (0.984, 0.014, 180.72),
        ("teal", 300) => (0.855, 0.138, 181.071),
        ("teal", 600) => (0.6, 0.118, 184.704),
        ("teal", 700) => (0.511, 0.096, 186.391),
        ("teal", 900) => (0.386, 0.063, 188.416),
        ("gray", 50) => (0.985, 0.002, 247.839),
        ("gray", 400) => (0.707, 0.022, 261.325),
        ("gray", 500) => (0.551, 0.027, 264.364),
        ("gray", 600) => (0.446, 0.03, 256.802),
        ("gray", 800) => (0.278, 0.033, 256.848),
        _ => (0.5, 0.0, 0.0),
    };
    oklch(l, c, h)
}

/// Fill, text and (inset ring or border) colour of a chip or badge.
#[derive(Clone, Copy)]
pub struct Chip {
    pub bg: Color,
    pub fg: Color,
    pub ring: Color,
}

/// `bg-{f}-50 text-{f}-700 ring-{f}-600/20 dark:bg-{f}-900/{dark_bg} dark:text-{f}-{dark_fg}
/// dark:ring-{f}-400/20` — the shape every status and pricing style shares.
fn ringed(tk: &Tk, f: &str, dark_bg: f32, dark_fg: u16, light_ring: (u16, f32)) -> Chip {
    if tk.dark {
        Chip {
            bg: tw(f, 900).alpha(dark_bg),
            fg: tw(f, dark_fg),
            ring: tw(f, 400).alpha(0.2),
        }
    } else {
        Chip {
            bg: tw(f, 50),
            fg: tw(f, 700),
            ring: tw(f, light_ring.0).alpha(light_ring.1),
        }
    }
}

/// `transactionStatusStyles`.
pub fn status_chip(tk: &Tk, status: super::model::Status) -> Chip {
    use super::model::Status;
    match status {
        Status::Completed => ringed(tk, "emerald", 0.3, 400, (600, 0.2)),
        Status::Pending => ringed(tk, "amber", 0.3, 400, (600, 0.2)),
        Status::Failed => ringed(tk, "red", 0.3, 400, (600, 0.2)),
        Status::Review => ringed(tk, "blue", 0.3, 400, (700, 0.1)),
    }
}

/// `pricingSourceStyles` by the row's pricing style key.
pub fn pricing_chip(tk: &Tk, style: &str) -> Chip {
    match style {
        "source" => ringed(tk, "emerald", 0.3, 400, (600, 0.2)),
        "wallet" => ringed(tk, "indigo", 0.25, 300, (600, 0.2)),
        "btcpay" => ringed(tk, "orange", 0.25, 300, (600, 0.2)),
        "override" => ringed(tk, "violet", 0.3, 300, (600, 0.2)),
        "missing" => ringed(tk, "amber", 0.3, 400, (600, 0.2)),
        _ => ringed(tk, "sky", 0.3, 400, (600, 0.2)),
    }
}

/// `transactionFlowStyles`: the 2 rem flow tile (border, fill, icon colour).
pub fn flow_chip(tk: &Tk, flow: super::model::Flow) -> Chip {
    use super::model::Flow;
    let (f, dark_bg_shade, dark_bg, dark_fg, border_shade) = match flow {
        Flow::Incoming => ("emerald", 900, 0.25, 300, 600),
        Flow::Outgoing => ("red", 900, 0.25, 300, 600),
        Flow::Transfer => ("zinc", 800, 0.7, 300, 500),
        Flow::Swap => ("sky", 900, 0.25, 300, 600),
        Flow::Layer => ("teal", 900, 0.25, 300, 600),
    };
    if tk.dark {
        Chip {
            bg: tw(f, dark_bg_shade).alpha(dark_bg),
            fg: tw(f, dark_fg),
            ring: tw(f, border_shade).alpha(0.2),
        }
    } else {
        Chip {
            bg: tw(f, 50),
            fg: tw(f, 700),
            ring: tw(f, border_shade).alpha(0.2),
        }
    }
}

/// The row amount's tone: `text-emerald-700 dark:text-emerald-300` / red, else muted.
pub fn amount_color(tk: &Tk, flow: super::model::Flow) -> Color {
    use super::model::Flow;
    match flow {
        Flow::Incoming => tw("emerald", if tk.dark { 300 } else { 700 }),
        Flow::Outgoing => tw("red", if tk.dark { 300 } else { 700 }),
        _ => tk.muted_fg,
    }
}

/// The coarse-price badge: `bg-amber-500/10 text-amber-700 dark:text-amber-400`.
pub fn coarse_chip(tk: &Tk) -> Chip {
    Chip {
        bg: tw("amber", 500).alpha(0.1),
        fg: tw("amber", if tk.dark { 400 } else { 700 }),
        ring: Color::TRANSPARENT,
    }
}
