//! `AddConnectionDialog`'s catalog types (the web's `ConnectionSource`, generated into
//! catalog.rs from ui-tauri/src/lib/connectionCatalog.tsx) and the Overview's entry points.
//! The dialog itself, both steps and every setup form, lives in `crate::wallets::add`; the
//! Overview mounts it through `dialog` / `on_key` below.

use rune::{Cx, El, Icon, KeyEvent, Tx};

use crate::app::App;
use crate::i18n::{has, t};
use crate::theme::Tk;

/// A catalog category. `label` is a translation key; read it through [`Category::label`].
pub struct Category {
    pub id: &'static str,
    pub(super) label: &'static str,
    pub icon: &'static Icon,
}

impl Category {
    pub fn label(&self) -> String {
        t(self.label)
    }
}

/// Catalog copy that is either a brand, product or format name shown as written in every
/// language (`Name`) or a translation key (`Key`).
#[derive(Clone, Copy)]
pub enum Label {
    Name(&'static str),
    Key(&'static str),
}

impl Label {
    pub fn text(self) -> String {
        match self {
            Label::Name(s) => s.to_owned(),
            Label::Key(k) => t(k),
        }
    }
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

/// A catalog source. Its copy lives in the `connections:catalog` bundle: `description`,
/// `path_label` and `details` are translation keys (`details` the key of an array) and `title`
/// / `format_label` are keys unless they are names; read them through the methods below so
/// every renderer shows the active language.
pub struct Source {
    pub id: &'static str,
    pub(super) title: Label,
    pub(super) description: &'static str,
    pub category: &'static str,
    pub planned: bool,
    pub dev_only: bool,
    pub(super) path_label: Option<&'static str>,
    pub(super) format_label: Option<Label>,
    pub(super) details: &'static str,
    pub art: Art,
    #[allow(dead_code)]
    pub size: Size,
    pub frame: Frame,
    pub docs_href: Option<&'static str>,
}

impl Source {
    pub fn title(&self) -> String {
        self.title.text()
    }
    pub fn description(&self) -> String {
        t(self.description)
    }
    pub fn path_label(&self) -> Option<String> {
        self.path_label.map(t)
    }
    pub fn format_label(&self) -> Option<String> {
        self.format_label.map(Label::text)
    }
    /// The setup notes, in bundle order (an i18next array flattens to `key.0`, `key.1`, …).
    pub fn details(&self) -> Vec<String> {
        (0..)
            .map(|i| format!("{}.{i}", self.details))
            .take_while(|k| has(k))
            .map(|k| t(&k))
            .collect()
    }
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    crate::wallets::add::on_key(app, ev, tx)
}

pub fn dialog(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    crate::wallets::add::dialog(app, cx, tk, layer);
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{CATEGORIES, SOURCES};
    use super::*;
    use crate::i18n::{Lang, missing, set_lang};

    /// Every catalog string resolves in both languages without falling back, the German copy
    /// has as many setup notes as the English, and keyed copy is actually translated.
    #[test]
    fn catalog_copy_is_translated() {
        let render = |lang| {
            set_lang(lang);
            let cats: Vec<String> = CATEGORIES.iter().map(Category::label).collect();
            let srcs: Vec<_> = SOURCES
                .iter()
                .map(|s| {
                    (
                        s.title(),
                        s.description(),
                        s.path_label(),
                        s.format_label(),
                        s.details(),
                    )
                })
                .collect();
            (cats, srcs)
        };
        let (en_cats, en) = render(Lang::En);
        let (de_cats, de) = render(Lang::De);
        set_lang(Lang::En);
        let catalog_misses: Vec<String> = missing()
            .into_iter()
            .filter(|k| k.contains("connections:catalog"))
            .collect();
        assert!(catalog_misses.is_empty(), "{catalog_misses:?}");
        assert_eq!(en_cats[0], "Wallets");
        assert_eq!(de_cats[4], "Börsen");
        for ((s, e), d) in SOURCES.iter().zip(&en).zip(&de) {
            assert!(!e.1.starts_with("connections:"), "{}: {}", s.id, e.1);
            assert!(!e.4.is_empty(), "{}: no setup notes", s.id);
            assert_eq!(e.4.len(), d.4.len(), "{}: setup note count", s.id);
            assert_ne!(e.1, d.1, "{}: description not translated", s.id);
            if let Label::Key(_) = s.title {
                assert_ne!(e.0, d.0, "{}: title not translated", s.id);
            } else {
                assert_eq!(e.0, d.0, "{}: a name is the same in every language", s.id);
            }
        }
        let address_list = SOURCES.iter().position(|s| s.id == "address-list").unwrap();
        assert_eq!(
            en[address_list].1,
            "Watch a flat list of individual addresses — e.g. a pre-HD Bitcoin Core keypool."
        );
        assert!(
            de[address_list]
                .1
                .starts_with("Beobachte eine flache Liste")
        );
    }
}
