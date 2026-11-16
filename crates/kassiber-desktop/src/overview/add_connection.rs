//! `AddConnectionDialog`'s catalog types (the web's `ConnectionSource`, generated into
//! catalog.rs from ui-tauri/src/lib/connectionCatalog.tsx) and the Overview's entry points.
//! The dialog itself, both steps and every setup form, lives in `crate::wallets::add`; the
//! Overview mounts it through `dialog` / `on_key` below.

use rune::{Cx, El, Icon, KeyEvent, Tx};

use crate::app::App;
use crate::theme::Tk;

pub struct Category {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static Icon,
}

/// A source's artwork: a logo file (with its extension, and whether the web flattens it to a
/// silhouette with `brightness-0 dark:invert`), an inline lettered badge, or a lucide icon.
// `Icon` and `None` are what tools/connection-catalog/gen.py emits for a source with an icon or
// no artwork; today's catalog (catalog.rs) has neither, but a regenerated one may.
#[allow(dead_code)]
pub enum Art {
    File(&'static [u8], &'static str, bool),
    Badge {
        fill: &'static str,
        ink: &'static str,
        label: &'static str,
    },
    Icon(&'static Icon),
    None,
}

/// The web's per-logo size class (`size-N`, `h-N w-auto`), kept as generated. The desktop draws
/// every logo at its role's size instead (`crate::logos::role`), so it is not read.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Size {
    Natural,
    Square(f32),
    Height(f32),
}

/// The artwork frame's fill when the source overrides it.
#[derive(Clone, Copy)]
pub enum Frame {
    Default,
    Fill(&'static str),
}

pub struct Source {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub planned: bool,
    pub dev_only: bool,
    pub path_label: Option<&'static str>,
    pub format_label: Option<&'static str>,
    pub details: &'static [&'static str],
    pub art: Art,
    #[allow(dead_code)]
    pub size: Size,
    pub frame: Frame,
    pub docs_href: Option<&'static str>,
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    crate::wallets::add::on_key(app, ev, tx)
}

pub fn dialog(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    crate::wallets::add::dialog(app, cx, tk, layer);
}
