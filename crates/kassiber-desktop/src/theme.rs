//! Kassiber's design tokens (ui-tauri/src/styles/globals.css) as Rune colours and type.
//!
//! Sizes follow the web app's rem scale: the root is 16 px × the UI scale (1.0 by default, see
//! `app::DEFAULT_SCALE`), so `text-sm` (0.875 rem) is 14 pt at the default scale, and every rem
//! value scales with it.

use rune::color::hex;
use rune::{Color, TextStyle, oklch, oklcha};
use rune_kit::{Kit, PaletteSeed};

fn h(s: &str) -> Color {
    hex(s).expect("valid hex colour")
}

/// Tailwind v4 status colours used across the shell and the dashboard.
#[derive(Clone, Copy, Debug)]
pub struct Tone {
    /// Pill/badge fill (the `/10`–`/15` tints).
    pub bg: Color,
    /// Pill border (the `/25`–`/35` tints).
    pub border: Color,
    /// Text on the tinted fill.
    pub text: Color,
    /// Solid accent (dots, chart marks).
    pub solid: Color,
}

// One spacing scale for every screen, in rem (owner request, 2026-11: the Overview was a little
// cramped, the other screens a little loose; both converge here). It replaces the web's
// `--kb-page-gutter` 1.25 / `--kb-page-gap` 0.75 and the Overview's 0.75 inset and gutter.
// Read it through `Tk::page_pad`, `card_gap`, `section_gap` and `card_pad`.
/// The page's inset.
pub const PAGE_PAD: f32 = 1.0;
/// Between sibling cards and tiles, both axes.
pub const CARD_GAP: f32 = 0.875;
/// Between a page's header row and its first card (and between later page blocks).
pub const SECTION_GAP: f32 = 0.875;
/// A card's inner padding.
pub const CARD_PAD: f32 = 1.0;

#[derive(Clone, Debug)]
pub struct Tk {
    pub dark: bool,
    /// Root font size in points (16 × app scale).
    pub rem: f32,
    // Surfaces.
    pub background: Color,
    pub chrome: Color,
    pub card: Color,
    pub popover: Color,
    pub muted: Color,
    pub accent: Color,
    // Ink.
    pub fg: Color,
    pub muted_fg: Color,
    pub primary: Color,
    pub primary_fg: Color,
    pub destructive: Color,
    // Lines.
    pub border: Color,
    pub input: Color,
    pub ring: Color,
    // Sidebar.
    pub sidebar_fg: Color,
    pub sidebar_muted_fg: Color,
    pub sidebar_border: Color,
    pub sidebar_control: Color,
    pub row_hover: Color,
    pub row_active: Color,
    // Brand.
    pub kb_accent: Color,
    pub kb_accent_text: Color,
    // Status tones.
    pub good: Tone,
    pub warn: Tone,
    pub alert: Tone,
    pub info: Tone,
    pub sky: Tone,
    // Chart.
    pub chart_btc: Color,
    pub chart_value: Color,
    pub chart_basis: Color,
    pub chart_price: Color,
    pub flow_in: Color,
    pub flow_out: Color,
    pub flow_move: Color,
    pub flow_fee: Color,
    pub glass_backdrop: Color,
}

impl Tk {
    pub fn new(dark: bool, scale: f32) -> Tk {
        let rem = 16.0 * scale;
        if dark {
            Tk {
                dark,
                rem,
                background: oklch(0.145, 0.0, 0.0),
                chrome: oklch(0.18, 0.0, 0.0),
                card: oklch(0.205, 0.0, 0.0),
                popover: oklch(0.205, 0.0, 0.0),
                muted: oklch(0.269, 0.0, 0.0),
                accent: oklch(0.269, 0.0, 0.0),
                fg: oklch(0.985, 0.0, 0.0),
                muted_fg: oklch(0.708, 0.0, 0.0),
                primary: oklch(0.922, 0.0, 0.0),
                primary_fg: oklch(0.205, 0.0, 0.0),
                destructive: oklch(0.704, 0.191, 22.216),
                border: oklcha(1.0, 0.0, 0.0, 0.10),
                input: oklcha(1.0, 0.0, 0.0, 0.15),
                ring: oklcha(0.985, 0.0, 0.0, 0.55),
                sidebar_fg: h("#f4f4f5"),
                sidebar_muted_fg: oklch(0.708, 0.0, 0.0),
                sidebar_border: h("#27272a"),
                sidebar_control: oklcha(1.0, 0.0, 0.0, 0.08),
                row_hover: oklcha(1.0, 0.0, 0.0, 0.06),
                row_active: oklcha(1.0, 0.0, 0.0, 0.12),
                kb_accent: h("#ff3341"),
                kb_accent_text: h("#ff3341"),
                good: Tone {
                    bg: h("#10b981").alpha(0.10),
                    border: h("#10b981").alpha(0.25),
                    text: h("#6ee7b7"),
                    solid: h("#34d399"),
                },
                warn: Tone {
                    bg: h("#f59e0b").alpha(0.10),
                    border: h("#f59e0b").alpha(0.25),
                    text: h("#fcd34d"),
                    solid: h("#fbbf24"),
                },
                alert: Tone {
                    bg: h("#ef4444").alpha(0.10),
                    border: h("#ef4444").alpha(0.25),
                    text: h("#fca5a5"),
                    solid: h("#f87171"),
                },
                info: Tone {
                    bg: h("#3b82f6").alpha(0.12),
                    border: h("#3b82f6").alpha(0.25),
                    text: h("#93c5fd"),
                    solid: h("#60a5fa"),
                },
                sky: Tone {
                    bg: h("#0ea5e9").alpha(0.12),
                    border: h("#0ea5e9").alpha(0.25),
                    text: h("#7dd3fc"),
                    solid: h("#0ea5e9"),
                },
                chart_btc: h("#f6a21a"),
                chart_value: h("#38bdf8"),
                chart_basis: h("#50c695"),
                chart_price: h("#94a3b8"),
                flow_in: h("#34d399"),
                flow_out: h("#f87171"),
                flow_move: h("#38bdf8"),
                flow_fee: h("#a1a1aa"),
                glass_backdrop: Color::BLACK.alpha(0.22),
            }
        } else {
            let fg = oklch(0.21, 0.006, 285.885);
            Tk {
                dark,
                rem,
                background: oklch(0.992, 0.0, 0.0),
                chrome: oklch(0.985, 0.0, 0.0),
                card: oklch(1.0, 0.0, 0.0),
                popover: oklch(1.0, 0.0, 0.0),
                muted: oklch(0.967, 0.001, 286.375),
                accent: oklch(0.967, 0.001, 286.375),
                fg,
                muted_fg: oklch(0.552, 0.016, 285.938),
                primary: fg,
                primary_fg: oklch(0.985, 0.0, 0.0),
                destructive: oklch(0.577, 0.245, 27.325),
                border: oklch(0.92, 0.004, 286.32),
                input: oklch(0.871, 0.006, 286.286),
                ring: oklcha(0.21, 0.006, 285.885, 0.45),
                sidebar_fg: fg,
                sidebar_muted_fg: oklch(0.552, 0.016, 285.938),
                sidebar_border: oklch(0.92, 0.004, 286.32),
                sidebar_control: oklcha(0.21, 0.006, 285.885, 0.05),
                row_hover: oklcha(0.21, 0.006, 285.885, 0.04),
                row_active: oklcha(0.21, 0.006, 285.885, 0.07),
                kb_accent: h("#e3000f"),
                kb_accent_text: h("#d1000e"),
                good: Tone {
                    bg: h("#10b981").alpha(0.10),
                    border: h("#10b981").alpha(0.25),
                    text: h("#047857"),
                    solid: h("#059669"),
                },
                warn: Tone {
                    bg: h("#f59e0b").alpha(0.10),
                    border: h("#f59e0b").alpha(0.25),
                    text: h("#b45309"),
                    solid: h("#d97706"),
                },
                alert: Tone {
                    bg: h("#ef4444").alpha(0.10),
                    border: h("#ef4444").alpha(0.25),
                    text: h("#b91c1c"),
                    solid: h("#dc2626"),
                },
                info: Tone {
                    bg: h("#3b82f6").alpha(0.12),
                    border: h("#3b82f6").alpha(0.25),
                    text: h("#1d4ed8"),
                    solid: h("#2563eb"),
                },
                sky: Tone {
                    bg: h("#0ea5e9").alpha(0.12),
                    border: h("#0ea5e9").alpha(0.25),
                    text: h("#0369a1"),
                    solid: h("#0ea5e9"),
                },
                chart_btc: h("#f7931a"),
                chart_value: h("#0284c7"),
                chart_basis: h("#2fae79"),
                chart_price: h("#64748b"),
                flow_in: h("#059669"),
                flow_out: h("#dc2626"),
                flow_move: h("#0284c7"),
                flow_fee: h("#71717a"),
                glass_backdrop: fg.alpha(0.06),
            }
        }
    }

    /// rem → points at the current scale.
    pub fn r(&self, rem: f32) -> f32 {
        rem * self.rem
    }

    /// The page's inset from the content panel's edges (every screen, the Overview included).
    pub fn page_pad(&self) -> f32 {
        self.r(PAGE_PAD)
    }
    /// The space between sibling cards and tiles, across and down.
    pub fn card_gap(&self) -> f32 {
        self.r(CARD_GAP)
    }
    /// The space between a page's blocks: its header row and the first card, and below.
    pub fn section_gap(&self) -> f32 {
        self.r(SECTION_GAP)
    }
    /// A card's (and an Overview tile's) inner padding.
    pub fn card_pad(&self) -> f32 {
        self.r(CARD_PAD)
    }

    fn ty(&self, rem: f32, weight: f32, line_rem: f32) -> TextStyle {
        TextStyle::new(self.r(rem), weight).lh(self.r(line_rem))
    }
    /// `text-3xs` (0.625 rem).
    pub fn t3xs(&self, w: f32) -> TextStyle {
        self.ty(0.625, w, 0.875)
    }
    /// `text-2xs` (0.694 rem).
    pub fn t2xs(&self, w: f32) -> TextStyle {
        self.ty(0.694, w, 1.0)
    }
    /// `text-xs` (0.75 rem / 1 rem).
    pub fn txs(&self, w: f32) -> TextStyle {
        self.ty(0.75, w, 1.0)
    }
    /// `text-sm` (0.875 rem / 1.25 rem).
    pub fn tsm(&self, w: f32) -> TextStyle {
        self.ty(0.875, w, 1.25)
    }
    /// `text-base` (1 rem / 1.5 rem).
    pub fn tbase(&self, w: f32) -> TextStyle {
        self.ty(1.0, w, 1.5)
    }
    /// `text-lg` (1.125 rem / 1.75 rem).
    pub fn tlg(&self, w: f32) -> TextStyle {
        self.ty(1.125, w, 1.75)
    }
    /// `text-xl` (1.25 rem / 1.75 rem).
    pub fn txl(&self, w: f32) -> TextStyle {
        self.ty(1.25, w, 1.75)
    }
    /// `<kbd>` hints (ui/kbd.tsx: `font-sans text-xs font-medium`): the one place the web's
    /// DM Sans actually applies, since index.html's body stack wins everywhere else.
    pub fn tkbd(&self) -> TextStyle {
        self.txs(500.0).family(rune::Family::Named("DM Sans 9pt"))
    }
    /// A fixed point size, for chart labels the web draws in px.
    pub fn px(&self, size: f32, w: f32) -> TextStyle {
        TextStyle::new(size, w)
    }

    /// The rune-kit tokens (menus, popovers, switches, tables) seeded from Kassiber's palette.
    pub fn kit(&self) -> Kit {
        let mut k = Kit::from_palette(PaletteSeed {
            bg: self.background,
            fg: self.fg,
            accent: self.primary,
        });
        k.c.page = self.background;
        k.c.bg_base = self.card;
        k.c.surface_float = self.popover;
        k.c.text_base = self.fg;
        k.c.text_strong = self.fg;
        k.c.text_muted = self.muted_fg;
        k.c.border_base = self.border;
        k.c.border_weak = self.border;
        k.c.hover = self.row_hover;
        k.c.pressed = self.row_active;
        k.radius.sm = self.r(0.25);
        k.radius.md = self.r(0.375);
        k.radius.lg = self.r(0.5);
        k
    }
}

pub const MASK: &str = "••••";
