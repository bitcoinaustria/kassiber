//! One behaviour for every dropdown menu and select list: Radix `DropdownMenu` and `Select` as
//! the web configures them (components/ui/dropdown-menu.tsx, select.tsx; no call site sets
//! `loop`). Call sites keep drawing their own trigger, rows and surface (each was matched to its
//! Tauri pixels); this module adds what the Radix primitives do around them:
//!
//! - **Trigger** ([`trigger`]): `aria-expanded`; Enter/Space/press toggle; ArrowDown opens a
//!   menu, ArrowDown/ArrowUp open a select (Radix `OPEN_KEYS`); it records whether the keyboard
//!   opened it.
//! - **Initial focus**: a select focuses its selected option (else the first enabled one); a menu
//!   opened from the keyboard focuses its first enabled item; a menu opened by the pointer
//!   focuses the list itself, so no row lights until the arrows or the pointer move (Radix
//!   focuses the content).
//! - **Keys** (on the surface, before any area's `on_key`): ArrowUp/Down move without wrapping,
//!   Home/End and PageUp/PageDown jump to the ends, typed letters search (Radix typeahead: a
//!   1 s buffer, a repeated letter cycles), disabled rows are skipped, Tab is held (Radix
//!   prevents it), Escape closes only this list and hands focus back to its trigger.
//! - **Structure**: exactly one `menu`/`listbox` (the surface drops Rune's implicit modal
//!   `dialog`), focus trapped, the page inert behind a scrim that closes on an outside press.
//! - **Fit**: the surface is at most the room the window leaves on its side (it flips when the
//!   other side has more), the list scrolls inside it and keeps the focused row in view.
//! - **Submenus** ([`sub_trigger`], [`Popup::sub`], [`Popup::mount_sub`]): open on hover and
//!   ArrowRight/Enter (focus on the sub's first row), ArrowLeft closes the sub and refocuses its
//!   trigger; the sub content is a non-modal layer beside a [`sub_anchor`] in its trigger, fitted
//!   to the window, so the parent menu stays live under it.
//! - **Non-modal menus** ([`trigger_nonmodal`]) and picks that re-render their trigger
//!   ([`refocus`]) keep focus where Radix puts it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rune::{El, Key, KeyEvent, ScrollSpec, Tx, col};
use rune_kit::Placement;

use crate::app::App;
use crate::theme::Tk;

/// Which Radix primitive a popup stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// `DropdownMenu`: role `menu`, items `menuitem`/`menuitemcheckbox`/`menuitemradio`.
    Menu,
    /// `Select` (and a native `<select>`'s popup): role `listbox`, items `option`.
    Listbox,
}

impl Kind {
    fn role(self) -> &'static str {
        match self {
            Kind::Menu => "menu",
            Kind::Listbox => "listbox",
        }
    }
}

thread_local! {
    /// Whether the last popup opened from the keyboard (Kobalte's / Radix's keyboard gesture).
    static BY_KEYS: Cell<bool> = const { Cell::new(false) };
    /// Typeahead: the search so far and the clock time of its last key.
    static SEARCH: RefCell<(String, f64)> = const { RefCell::new((String::new(), f64::NEG_INFINITY)) };
    /// ArrowLeft just closed a submenu under the pointer: the hover its trigger sees again as
    /// the submenu's layer goes doesn't reopen it (Radix waits for the pointer to move).
    static HOLD_HOVER: Cell<bool> = const { Cell::new(false) };
}

/// Record how a popup is being opened; [`trigger`] calls it, a bespoke opener (a context menu,
/// a shortcut) may too.
pub fn note_open(by_keys: bool) {
    BY_KEYS.with(|c| c.set(by_keys));
    SEARCH.with(|s| s.borrow_mut().0.clear());
}

fn opened_by_keys() -> bool {
    BY_KEYS.with(|c| c.get())
}

/// A menu or select trigger as Radix wires it. `el` is the call site's styled trigger (key,
/// role, label and look already set) without a press handler; `toggle` opens or closes the
/// popup. Adds `aria-expanded`, the press, and ArrowDown (menus) / ArrowDown+ArrowUp (selects)
/// opening it.
pub fn trigger(el: El, kind: Kind, open: bool, toggle: impl Fn(&mut App) + Clone + 'static) -> El {
    let t = toggle.clone();
    el.expanded(open)
        .on_press_tx(move |a: &mut App, tx: &mut Tx| {
            note_open(tx.by_keys());
            t(a);
        })
        .on_key(move |a: &mut App, ev: &KeyEvent, _tx: &mut Tx| {
            if open || ev.mods.cmd || ev.mods.alt || ev.mods.ctrl {
                return false;
            }
            if opens_on(kind, &ev.key) {
                note_open(true);
                toggle(a);
                return true;
            }
            false
        })
}

/// [`trigger`] as builder methods: `.menu_trigger(open, toggle)` where a call site had
/// `.expanded(open).on_press(toggle)`.
pub trait TriggerExt: Sized {
    fn menu_trigger(self, open: bool, toggle: impl Fn(&mut App) + Clone + 'static) -> El;
    fn select_trigger(self, open: bool, toggle: impl Fn(&mut App) + Clone + 'static) -> El;
}

impl TriggerExt for El {
    fn menu_trigger(self, open: bool, toggle: impl Fn(&mut App) + Clone + 'static) -> El {
        trigger(self, Kind::Menu, open, toggle)
    }
    fn select_trigger(self, open: bool, toggle: impl Fn(&mut App) + Clone + 'static) -> El {
        trigger(self, Kind::Listbox, open, toggle)
    }
}

/// [`trigger`] for a non-modal menu (`DropdownMenu modal={false}`): Rune's autofocus only
/// lands inside a modal layer, so opening focuses the list itself (pointer) or its first row
/// `first_key` (keyboard), as Radix moves focus into the content either way.
pub fn trigger_nonmodal(
    el: El,
    kind: Kind,
    open: bool,
    list_key: impl Into<String>,
    first_key: impl Into<String>,
    toggle: impl Fn(&mut App) + Clone + 'static,
) -> El {
    let (list, first) = (list_key.into(), first_key.into());
    let (f2, t2) = (first.clone(), toggle.clone());
    el.expanded(open)
        .on_press_tx(move |a: &mut App, tx: &mut Tx| {
            let keys = tx.by_keys();
            note_open(keys);
            t2(a);
            if !open {
                tx.focus(if keys { f2.clone() } else { list.clone() }, keys);
            }
        })
        .on_key(move |a: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            if open || ev.mods.cmd || ev.mods.alt || ev.mods.ctrl || !opens_on(kind, &ev.key) {
                return false;
            }
            note_open(true);
            toggle(a);
            tx.focus(first.clone(), true);
            true
        })
}

/// Hands focus back to `anchor` after a pick whose effect re-renders the trigger (a filter
/// count on it, a route change): the list's own `focus_return` can miss the trigger then, so
/// this asks again by key. Call from the row's press handler after choosing. (A trigger that
/// remounts later, as its row re-lays out on a reply, needs a stable key on its wrapper.)
pub fn refocus(tx: &mut Tx, anchor: &str) {
    let ring = tx.by_keys();
    tx.focus(anchor.to_owned(), ring);
}

/// The keys that open a closed trigger besides Enter and Space: Radix DropdownMenu's
/// `ArrowDown`, Radix Select's `ArrowDown` and `ArrowUp`.
pub fn opens_on(kind: Kind, key: &Key) -> bool {
    match kind {
        Kind::Menu => *key == Key::Down,
        Kind::Listbox => matches!(key, Key::Down | Key::Up),
    }
}

/// One focusable row as the key handler sees it.
#[derive(Clone, Debug)]
struct Item {
    key: String,
    text: String,
    enabled: bool,
    selected: bool,
}

type Close = Rc<dyn Fn(&mut App)>;

/// An open popup's rows and rules. Build it, add rows in order with [`Popup::item`] (and
/// non-focusable content with [`Popup::add`]), then [`Popup::mount`] it, or take the list and
/// key handler with [`Popup::finish`] for a bespoke surface.
pub struct Popup {
    kind: Kind,
    anchor: String,
    close: Close,
    items: Vec<Item>,
    /// Rows and other content in order; rows carry their index in `items`.
    children: Vec<(Option<usize>, El)>,
    list_key: String,
    /// A submenu: ArrowLeft closes it and refocuses this sub trigger.
    parent: Option<(String, Close)>,
    focus_list: bool,
    /// [`Popup::prepare`] already gave a row the initial focus.
    prepared_focus: bool,
    /// The scroll viewport's own cap (a `max-h-*` on the web's content), under the window fit.
    max_h: Option<f32>,
}

impl Popup {
    pub fn new(kind: Kind, anchor: impl Into<String>, close: impl Fn(&mut App) + 'static) -> Self {
        let anchor = anchor.into();
        let list_key = format!("{anchor}-menu");
        Popup {
            kind,
            anchor,
            close: Rc::new(close),
            items: Vec::new(),
            children: Vec::new(),
            list_key,
            parent: None,
            focus_list: true,
            prepared_focus: false,
            max_h: None,
        }
    }

    /// Caps the scrolling list's height (the web's `max-h-*` on the content); the window fit
    /// still applies under it. Put it here, not on the list container: a capped container
    /// inside the viewport would clip its rows instead of scrolling them.
    pub fn max_h(mut self, h: f32) -> Self {
        self.max_h = Some(h);
        self
    }

    /// The list's key (default `{anchor}-menu`).
    pub fn list_key(mut self, key: impl Into<String>) -> Self {
        self.list_key = key.into();
        self
    }

    pub fn menu(anchor: impl Into<String>, close: impl Fn(&mut App) + 'static) -> Self {
        Self::new(Kind::Menu, anchor, close)
    }

    pub fn listbox(anchor: impl Into<String>, close: impl Fn(&mut App) + 'static) -> Self {
        Self::new(Kind::Listbox, anchor, close)
    }

    /// A submenu of the menu opened from `anchor`, opened by the sub trigger keyed
    /// `trigger_key`: Escape still closes the whole menu (`close`) and refocuses `anchor`,
    /// ArrowLeft runs `close_sub` and refocuses the sub trigger. Its rows don't take focus as it
    /// opens on hover; [`sub_trigger`] focuses the first on ArrowRight.
    pub fn sub(
        kind: Kind,
        anchor: impl Into<String>,
        trigger_key: impl Into<String>,
        close: impl Fn(&mut App) + 'static,
        close_sub: impl Fn(&mut App) + 'static,
    ) -> Self {
        let trigger_key = trigger_key.into();
        let mut p = Self::new(kind, anchor, close).list_key(format!("{trigger_key}-list"));
        p.parent = Some((trigger_key, Rc::new(close_sub)));
        p.focus_list = false;
        p
    }

    /// A focusable row: `el` is the call site's styled row (its press handler picks and closes);
    /// `text` is what typeahead matches. Disabled rows are skipped by the keys and the pointer;
    /// `selected` is the select's value (or the checked radio), where a select opens.
    pub fn item(
        &mut self,
        el: El,
        key: impl Into<String>,
        text: impl Into<String>,
        enabled: bool,
        selected: bool,
    ) {
        let key = key.into();
        self.children.push((
            Some(self.items.len()),
            el.key(key.clone())
                .focusable(enabled)
                .disabled(!enabled)
                .focus_selected(selected && enabled),
        ));
        self.items.push(Item {
            key,
            text: text.into(),
            enabled,
            selected,
        });
    }

    /// [`Popup::item`] for a list the call site lays out itself (rows inside nested columns, a
    /// header above them): returns the row ready to place; finish with [`Popup::finish_custom`].
    /// The initial focus goes to the selected row (a select's value), or a keyboard-opened
    /// menu's first enabled row.
    pub fn prepare(
        &mut self,
        el: El,
        key: impl Into<String>,
        text: impl Into<String>,
        enabled: bool,
        selected: bool,
    ) -> El {
        let key = key.into();
        let first = !self.items.iter().any(|i| i.enabled);
        self.items.push(Item {
            key: key.clone(),
            text: text.into(),
            enabled,
            selected,
        });
        let el = el
            .key(key)
            .focusable(enabled)
            .disabled(!enabled)
            .focus_selected(selected && enabled);
        let take = enabled
            && !self.prepared_focus
            && self.parent.is_none()
            && match self.kind {
                Kind::Listbox => selected,
                Kind::Menu => first && opened_by_keys(),
            };
        if take {
            self.prepared_focus = true;
            el.autofocus()
        } else {
            el
        }
    }

    /// The list for rows from [`Popup::prepare`]: `container` already holds them; it gets the
    /// role, key and focus group (and the focus itself when no row took it).
    pub fn finish_custom(self, container: El) -> (El, KeyHandler) {
        let mut list = name(
            self.kind,
            container
                .key(self.list_key.clone())
                .role(self.kind.role())
                .focus_group(),
            &self.anchor,
            self.parent.as_ref(),
        );
        if !self.prepared_focus && self.focus_list {
            list = list.focusable(true).no_focus_ring().autofocus();
        }
        let handler = KeyHandler {
            anchor: self.anchor,
            list: self.list_key,
            items: Rc::new(self.items),
            close: self.close,
            parent: self.parent,
        };
        (list, handler)
    }

    /// Content that isn't a row: a label, a separator, a hint.
    pub fn add(&mut self, el: El) {
        self.children.push((None, el));
    }

    /// The row that takes focus as the popup opens: the selected option of a select (else its
    /// first enabled one), a keyboard-opened menu's first enabled item, else none (the list).
    fn initial(&self) -> Option<usize> {
        if self.parent.is_some() {
            return None;
        }
        initial_focus(self.kind, &self.flags(), opened_by_keys())
    }

    fn flags(&self) -> Vec<(bool, bool)> {
        self.items.iter().map(|i| (i.enabled, i.selected)).collect()
    }

    /// The finished list (`container` with the role, rows and initial focus; it scrolls) and the
    /// key handler to put on the surface (the modal element focus stays inside).
    pub fn finish(self, container: El) -> (El, KeyHandler) {
        let initial = self.initial();
        let list_key = self.list_key;
        let children = self.children.into_iter().map(|(i, el)| {
            if i.is_some() && i == initial {
                el.autofocus()
            } else {
                el
            }
        });
        let mut list = name(
            self.kind,
            container
                .key(list_key.clone())
                .role(self.kind.role())
                .focus_group(),
            &self.anchor,
            self.parent.as_ref(),
        )
        .children(children);
        if initial.is_none() && self.focus_list {
            // Radix focuses the content when the pointer opened a menu: no row lights.
            list = list.focusable(true).no_focus_ring().autofocus();
        }
        let handler = KeyHandler {
            anchor: self.anchor,
            list: list_key,
            items: Rc::new(self.items),
            close: self.close,
            parent: self.parent,
        };
        (list, handler)
    }

    /// Mounts the popup in `layer`: a scrim, then `surface(list)` anchored to the trigger, at
    /// most as tall as the room on its side, with the list scrolling inside. `container` is the
    /// call site's list column (width, padding, gap); `surface` wraps the scrolling list in its
    /// look (glass, padding, a header) and returns it.
    pub fn mount(
        self,
        tk: &Tk,
        layer: &mut Vec<El>,
        container: El,
        surface: impl FnOnce(El) -> El,
        placement: Placement,
        gap: f32,
    ) {
        let anchor = self.anchor.clone();
        let close = self.close.clone();
        let c2 = close.clone();
        layer.push(rune_kit::scrim(
            format!("{anchor}-scrim"),
            move |a: &mut App| c2(a),
        ));
        let max_h = self.max_h;
        let (list, keys) = self.finish(container);
        // The viewport shrinks to the room the window leaves (its `shrink` zeroes only its own
        // minimum width; the list inside keeps the call site's).
        let mut list = col()
            .key(format!("{anchor}-scroll"))
            .shrink()
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(list);
        if let Some(h) = max_h {
            list = list.max_h(h);
        }
        let s = keys.attach(surface(list).min_h(0.0));
        let s = s
            .modal(true, move |a: &mut App| close(a))
            .focus_return(anchor.clone());
        layer.push(crate::shell::anchored(tk, s, &anchor, placement, gap));
    }
}

/// Where a submenu's content attaches: a 1 pt wide strip inside its sub trigger row, starting
/// `dy` below the row's top (Radix puts the content's first row level with the trigger, so this
/// is minus the content's padding and border, plus any measured offset). Add it as a child of
/// the row; [`Popup::mount_sub`] anchors to it. The strip runs far down: when the content
/// would overflow the window's bottom, Rune flips its alignment to the anchor's end, which then
/// lies below the window, so the window clamp leaves it shifted up just enough, as Floating
/// UI's `shift` (Radix) does, instead of hanging it above the trigger.
pub fn sub_anchor(trigger_key: &str, dy: f32) -> El {
    rune::el()
        .key(format!("{trigger_key}-sub-anchor"))
        .absolute(0.0, dy)
        .size(1.0, SUB_ANCHOR_H)
        .a11y_hidden()
}

const SUB_ANCHOR_H: f32 = 8192.0;

impl Popup {
    /// Mounts a submenu (built with [`Popup::sub`]) as its own non-modal layer beside its
    /// trigger's [`sub_anchor`], after the parent menu's layer: the parent stays live and in
    /// the tree, and the window fit (flip, then shift 8 pt inside) applies as Radix's collision
    /// handling does. `side` is where it opens (Left beside a menu at the window's right edge),
    /// `gap` the distance from the trigger's edge (negative overlaps).
    pub fn mount_sub(
        self,
        tk: &Tk,
        layer: &mut Vec<El>,
        container: El,
        surface: impl FnOnce(El) -> El,
        side: rune::Side,
        gap: f32,
    ) {
        let trigger = self
            .parent
            .as_ref()
            .map(|(k, _)| k.clone())
            .unwrap_or_default();
        let max_h = self.max_h;
        let (list, keys) = self.finish(container);
        let body = match max_h {
            Some(h) => col()
                .shrink()
                .min_h(0.0)
                .max_h(h)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
            None => list,
        };
        let s = keys.attach(surface(body));
        let s = s
            .anchored(
                format!("{trigger}-sub-anchor"),
                side,
                rune::Align::Start,
                gap,
            )
            .instant_changes()
            .motion_intent(rune::MotionIntent::CUT)
            .transition(crate::shell::popover_enter(tk, side, rune::Align::Start));
        layer.push(s);
    }
}

/// Radix's `DropdownMenuContent` is `aria-labelledby` its trigger (a submenu's, its sub
/// trigger): the list is named after it unless the call site labelled it.
fn name(kind: Kind, list: El, anchor: &str, parent: Option<&(String, Close)>) -> El {
    match kind {
        Kind::Menu => {
            list.labelled_by(parent.map(|(k, _)| k.as_str()).unwrap_or(anchor).to_owned())
        }
        Kind::Listbox => list,
    }
}

/// The keyboard of an open popup, attached to its surface with [`KeyHandler::attach`].
#[derive(Clone)]
pub struct KeyHandler {
    anchor: String,
    list: String,
    items: Rc<Vec<Item>>,
    close: Close,
    parent: Option<(String, Close)>,
}

impl KeyHandler {
    /// Puts the key handler on `surface` and keeps it out of the accessibility tree (no extra
    /// `dialog` around the list). `surface` must be an element around the list, not the list
    /// styled in place (that would lose its role and key): use [`KeyHandler::attach_to_list`].
    pub fn attach(self, surface: El) -> El {
        surface
            .role("none")
            .on_key(move |a: &mut App, ev: &KeyEvent, tx: &mut Tx| self.key(a, ev, tx))
    }

    /// Puts the key handler on an element that is both the list and its surface (a shell menu
    /// whose glass is drawn on the list itself): its key and `menu` role stay.
    pub fn attach_to_list(self, list: El) -> El {
        list.on_key(move |a: &mut App, ev: &KeyEvent, tx: &mut Tx| self.key(a, ev, tx))
    }

    /// The trigger focus returns to when the popup closes.
    pub fn anchor(&self) -> &str {
        &self.anchor
    }

    /// The handler alone, for a surface that holds more than the list (a search field).
    pub fn key(&self, a: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let m = ev.mods;
        if ev.key == Key::Escape {
            (self.close)(a);
            tx.return_focus(self.anchor.clone(), true);
            return true;
        }
        if m.cmd || m.ctrl || m.alt {
            return false;
        }
        let flags: Vec<bool> = self.items.iter().map(|i| i.enabled).collect();
        let focused = tx.focused().map(str::to_owned);
        let cur = focused
            .as_deref()
            .and_then(|f| self.items.iter().position(|i| i.key == f));
        // Keys for something else in the surface (a field, a submenu's rows) stay theirs.
        if cur.is_none() && focused.as_deref().is_some_and(|f| f != self.list) {
            return false;
        }
        match &ev.key {
            Key::Tab => true,
            Key::Left if self.parent.is_some() => {
                let (k, close_sub) = self.parent.as_ref().unwrap();
                HOLD_HOVER.with(|h| h.set(true));
                close_sub(a);
                tx.focus(k.clone(), true);
                true
            }
            Key::Down | Key::Up | Key::Home | Key::End | Key::PageUp | Key::PageDown => {
                if let Some(i) = step(&flags, cur, &ev.key) {
                    let k = self.items[i].key.clone();
                    tx.scroll_into_view(k.clone());
                    tx.focus(k, true);
                }
                true
            }
            Key::Text(t) if is_search_char(t) => {
                let now = tx.clock();
                let search = SEARCH.with(|s| {
                    let mut s = s.borrow_mut();
                    if now - s.1 > 1.0 {
                        s.0.clear();
                    }
                    // A space starts nothing (it presses the row); within a search it's a letter.
                    if t == " " && s.0.is_empty() {
                        return None;
                    }
                    s.0.push_str(t);
                    s.1 = now;
                    Some(s.0.clone())
                });
                let Some(search) = search else { return false };
                let texts: Vec<(&str, bool)> = self
                    .items
                    .iter()
                    .map(|i| (i.text.as_str(), i.enabled))
                    .collect();
                if let Some(i) = typeahead(&texts, cur, &search) {
                    let k = self.items[i].key.clone();
                    tx.scroll_into_view(k.clone());
                    tx.focus(k, true);
                }
                true
            }
            _ => false,
        }
    }
}

fn is_search_char(t: &str) -> bool {
    let mut cs = t.chars();
    matches!((cs.next(), cs.next()), (Some(c), None) if !c.is_control())
}

/// A sub trigger row (`DropdownMenuSubTrigger`): hovering it or pressing it opens the sub, and
/// ArrowRight/Enter open it with focus on its first row (`first_key`). `el` is the styled row.
pub fn sub_trigger(
    el: El,
    open: bool,
    first_key: impl Into<String>,
    open_sub: impl Fn(&mut App) + Clone + 'static,
) -> El {
    let first = first_key.into();
    let (o1, o2, o3) = (open_sub.clone(), open_sub.clone(), open_sub);
    el.expanded(open)
        .on_hover_change(move |a: &mut App, inside: bool, _tx: &mut Tx| {
            let held = HOLD_HOVER.with(|h| h.replace(false));
            if inside && !held {
                o1(a);
            }
        })
        .on_press(move |a: &mut App| o2(a))
        .on_key(move |a: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            if matches!(ev.key, Key::Right | Key::Enter) || ev.key == Key::Text(" ".into()) {
                o3(a);
                note_open(true);
                tx.focus(first.clone(), true);
                return true;
            }
            false
        })
}

/// Where focus starts: `rows` are (enabled, selected).
pub fn initial_focus(kind: Kind, rows: &[(bool, bool)], by_keys: bool) -> Option<usize> {
    let first = rows.iter().position(|r| r.0);
    match kind {
        Kind::Listbox => rows.iter().position(|r| r.0 && r.1).or(first),
        Kind::Menu if by_keys => first,
        Kind::Menu => None,
    }
}

/// The row an arrow, Home/End or Page key moves to from `cur` (`None`: the list itself has
/// focus), over rows that are `enabled`. No wrapping: at an end it stays.
pub fn step(enabled: &[bool], cur: Option<usize>, key: &Key) -> Option<usize> {
    let first = enabled.iter().position(|e| *e)?;
    let last = enabled.iter().rposition(|e| *e)?;
    match key {
        Key::Home | Key::PageUp => Some(first),
        Key::End | Key::PageDown => Some(last),
        Key::Down => match cur {
            None => Some(first),
            Some(i) => (i + 1..enabled.len()).find(|j| enabled[*j]).or(Some(i)),
        },
        Key::Up => match cur {
            None => Some(last),
            Some(i) => (0..i).rev().find(|j| enabled[*j]).or(Some(i)),
        },
        _ => cur,
    }
}

/// Radix's `getNextMatch`: the next enabled row after `cur` whose text starts with `search`
/// (case-insensitive), wrapping round; a search of one letter repeated cycles through the
/// rows starting with it. `None` when nothing else matches.
pub fn typeahead(rows: &[(&str, bool)], cur: Option<usize>, search: &str) -> Option<usize> {
    let chars: Vec<char> = search.chars().collect();
    let repeated = chars.len() > 1 && chars.iter().all(|c| *c == chars[0]);
    let needle = if repeated {
        chars[0].to_string()
    } else {
        search.to_owned()
    }
    .to_lowercase();
    let cands: Vec<usize> = (0..rows.len()).filter(|i| rows[*i].1).collect();
    if cands.is_empty() {
        return None;
    }
    let start = cur
        .and_then(|c| cands.iter().position(|i| *i == c))
        .unwrap_or(0);
    let mut wrapped: Vec<usize> = cands[start..]
        .iter()
        .chain(cands[..start].iter())
        .copied()
        .collect();
    if needle.chars().count() == 1 {
        wrapped.retain(|i| Some(*i) != cur);
    }
    let next = wrapped
        .into_iter()
        .find(|i| rows[*i].0.to_lowercase().starts_with(&needle))?;
    (Some(next) != cur).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_clamp_and_skip_disabled() {
        let en = [true, false, true, true, false];
        assert_eq!(step(&en, None, &Key::Down), Some(0));
        assert_eq!(step(&en, None, &Key::Up), Some(3));
        assert_eq!(step(&en, Some(0), &Key::Down), Some(2));
        assert_eq!(step(&en, Some(2), &Key::Up), Some(0));
        // No wrap at either end.
        assert_eq!(step(&en, Some(3), &Key::Down), Some(3));
        assert_eq!(step(&en, Some(0), &Key::Up), Some(0));
        assert_eq!(step(&en, Some(2), &Key::Home), Some(0));
        assert_eq!(step(&en, Some(0), &Key::End), Some(3));
        assert_eq!(step(&en, Some(0), &Key::PageDown), Some(3));
        assert_eq!(step(&[false, false], None, &Key::Down), None);
    }

    #[test]
    fn initial_focus_follows_radix() {
        let rows = [(false, false), (true, false), (true, true)];
        assert_eq!(initial_focus(Kind::Listbox, &rows, false), Some(2));
        assert_eq!(
            initial_focus(Kind::Listbox, &[(true, false), (true, false)], false),
            Some(0)
        );
        assert_eq!(initial_focus(Kind::Menu, &rows, true), Some(1));
        assert_eq!(initial_focus(Kind::Menu, &rows, false), None);
        // A disabled selected option isn't focused.
        assert_eq!(
            initial_focus(Kind::Listbox, &[(true, false), (false, true)], false),
            Some(0)
        );
    }

    #[test]
    fn typeahead_matches_radix() {
        let rows = [
            ("Apple", true),
            ("Banana", true),
            ("Blueberry", false),
            ("Cherry", true),
            ("Bread", true),
        ];
        assert_eq!(typeahead(&rows, None, "b"), Some(1));
        assert_eq!(typeahead(&rows, Some(1), "b"), Some(4)); // skips the disabled Blueberry
        assert_eq!(typeahead(&rows, Some(4), "b"), Some(1)); // wraps round
        assert_eq!(typeahead(&rows, Some(1), "bb"), Some(4)); // a repeated letter cycles
        assert_eq!(typeahead(&rows, Some(0), "br"), Some(4));
        assert_eq!(typeahead(&rows, Some(4), "bre"), None); // already there
        assert_eq!(typeahead(&rows, Some(0), "z"), None);
        assert_eq!(typeahead(&rows, None, "CH"), Some(3));
    }

    #[test]
    fn opening_keys() {
        assert!(opens_on(Kind::Menu, &Key::Down));
        assert!(!opens_on(Kind::Menu, &Key::Up));
        assert!(opens_on(Kind::Listbox, &Key::Up));
        assert!(!opens_on(Kind::Listbox, &Key::Left));
    }
}
