//! The persisted preferences as data: the web's `kb.ui` fields the Rune app has, read
//! tolerantly (each field validated on its own; a missing or invalid one keeps its default, as
//! the web store's `merge` does), written deterministically, and the schema migrations.
//!
//! File shape (`schemaVersion` 1):
//! `{"schemaVersion": 1, "ui": {<kb.ui field names>}, "lastRoute": "/connections",
//!   "window": {"x", "y", "width", "height", "maximized"}}`.
//! No secrets, amounts, txids, xpubs, descriptors or paths: book-scoped entries are keyed by a
//! hash of the book's data root, routes keep only their first path segment.
//! `identity` (the Welcome gate) is kept without paths: an opened folder is named by its book key,
//! its path lives only in the approved-projects file (`onboarding::projects`).

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::app::{Currency, DEFAULT_SCALE, Privacy, ThemeMode};
use crate::i18n::Lang;
use crate::overview::holdings_model::HoldingsView;

pub const SCHEMA_VERSION: u64 = 1;

/// `appLockPolicy`.
#[derive(Clone, Debug, PartialEq)]
pub struct LockPolicy {
    pub auto_lock: bool,
    pub idle_minutes: u32,
    pub require_on_launch: bool,
    pub lock_on_close: bool,
    pub touch_id_unlock: bool,
}

impl Default for LockPolicy {
    fn default() -> Self {
        LockPolicy {
            auto_lock: false,
            idle_minutes: 5,
            require_on_launch: false,
            lock_on_close: false,
            touch_id_unlock: false,
        }
    }
}

/// One remembered column sort (`TableSort`): a key the table offers and a direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: String,
    pub desc: bool,
}

/// `TABLE_SORT_IDS` and the keys each table accepts.
pub const TABLES: [(&str, &[&str]); 3] = [
    (
        "wallets",
        &[
            "label",
            "kind",
            "transactions",
            "last",
            "taxFree",
            "balance",
        ],
    ),
    ("review", &["date"]),
    ("transactions", &["date", "amount"]),
];

/// The `kb.ui` fields the Rune app has (see the module docs for what is left out and why).
#[derive(Clone, Debug, PartialEq)]
pub struct UiPrefs {
    pub lang: Lang,
    pub currency: Currency,
    pub theme: ThemeMode,
    pub privacy: Privacy,
    pub hide_on_unlock: bool,
    pub clear_clipboard: bool,
    pub desktop_notifications: bool,
    /// `hideAmountsWhileScreenShared`, `excludeUnencryptedFromBackups` (Rune additions, osleaks).
    pub hide_on_screen_share: bool,
    pub exclude_unencrypted_backups: bool,
    pub scale: f32,
    /// `explorerSettings.publicFallbacksAllowed` (the user's choice; absent means allowed).
    pub explorer_fallbacks_allowed: bool,
    pub lock: LockPolicy,
    pub ai_features: bool,
    pub developer_tools: bool,
    pub alpha_visible: bool,
    pub nav_collapsed: bool,
    /// `assistantModelSelection`: (provider row name, model id).
    pub model_selection: Option<(String, String)>,
    pub model_favorites: Vec<(String, String)>,
    pub dock_position: &'static str,
    pub closed_bar: &'static str,
    pub assistant_panel_open: bool,
    pub right_panel_open: bool,
    pub side_panel_width: f32,
    pub permission_mode: &'static str,
    pub tool_profiles: BTreeMap<String, String>,
    /// `assistantChatPermissionModes`: saved chat id → the permission mode it last used.
    pub chat_modes: BTreeMap<String, &'static str>,
    /// `bookChartPeriods`: book key → period key (`auto`, `30days` … `all`, a year).
    pub book_periods: BTreeMap<String, String>,
    /// `tableSorts`: table → remembered sort (`None` = deliberately unsorted). Absent tables
    /// keep their own default.
    pub table_sorts: BTreeMap<&'static str, Option<Sort>>,
    /// `holdingsView`: Holdings by Source layout, order, filter and folding; the same for every
    /// book (`normalizeHoldingsView`).
    pub holdings_view: HoldingsView,
    /// Rune only (`overviewLayout`): which Overview widgets show and where; the same for every
    /// book. A stored layout of another version, or an invalid entry, keeps the defaults.
    pub overview_layout: crate::overview::grid::model::Layout,
    /// Rune only: the page to reopen (first path segment).
    pub last_route: Option<String>,
    /// `identity`: who setup made the app for, without paths (`onboarding::model::Identity`);
    /// None shows Welcome.
    pub identity: Option<crate::onboarding::model::Identity>,
    /// `firstSyncDone`: book identity key → its first sync finished.
    pub first_sync_done: BTreeMap<String, bool>,
}

pub const SIDE_PANEL_DEFAULT_WIDTH: f32 = 600.0;
pub const SIDE_PANEL_MIN_WIDTH: f32 = 480.0;
pub const SIDE_PANEL_MAX_WIDTH: f32 = 1680.0;
const MAX_NAME: usize = 200;
const MAX_FAVORITES: usize = 200;
const MAX_PROFILES: usize = 200;
/// `ASSISTANT_CHAT_PERMISSION_MODES_LIMIT`.
pub const MAX_CHAT_MODES: usize = 200;
pub const MAX_BOOKS: usize = 64;

impl Default for UiPrefs {
    fn default() -> Self {
        UiPrefs {
            lang: Lang::En,
            currency: Currency::Btc,
            theme: ThemeMode::Dark,
            privacy: Privacy::Normal,
            hide_on_unlock: false,
            clear_clipboard: true,
            desktop_notifications: false,
            hide_on_screen_share: true,
            exclude_unencrypted_backups: true,
            scale: DEFAULT_SCALE,
            explorer_fallbacks_allowed: true,
            lock: LockPolicy::default(),
            ai_features: true,
            developer_tools: false,
            alpha_visible: true,
            nav_collapsed: false,
            model_selection: None,
            model_favorites: vec![],
            dock_position: "right",
            closed_bar: "hidden",
            assistant_panel_open: false,
            right_panel_open: false,
            side_panel_width: SIDE_PANEL_DEFAULT_WIDTH,
            permission_mode: "auto",
            tool_profiles: BTreeMap::new(),
            chat_modes: BTreeMap::new(),
            book_periods: BTreeMap::new(),
            table_sorts: BTreeMap::new(),
            holdings_view: HoldingsView::default(),
            overview_layout: Default::default(),
            last_route: None,
            identity: None,
            first_sync_done: BTreeMap::new(),
        }
    }
}

/// A book identity key (`bookIdentityKey`): a data root's hash, or `id:<workspace>/<profile>`.
fn is_identity_key(k: &str) -> bool {
    is_book_key(k) || (k.starts_with("id:") && k.len() > 3 && k.chars().count() <= 2 * MAX_NAME + 4)
}

// ---- Normalizers (the web store's) ----------------------------------------------------------------

/// `normalizeAppScale`: 0.05 steps within 0.8–1.2; anything else is the default.
pub fn normalize_scale(v: &Value) -> Option<f32> {
    let x = v.as_f64().filter(|x| x.is_finite())?;
    let stepped = (x / 0.05).round() * 0.05;
    Some(((stepped.clamp(0.8, 1.2) * 100.0).round() / 100.0) as f32)
}

/// `normalizeSidePanelWidth`.
pub fn normalize_width(v: &Value) -> Option<f32> {
    let x = v.as_f64().filter(|x| x.is_finite())?;
    Some(
        (x as f32)
            .clamp(SIDE_PANEL_MIN_WIDTH, SIDE_PANEL_MAX_WIDTH)
            .round(),
    )
}

/// `normalizePeriodParam` for stored keys: the canonical key, or None.
pub fn normalize_period(s: &str) -> Option<String> {
    let s = s.trim().to_lowercase();
    Some(match s.as_str() {
        "auto" | "automatic" => "auto".into(),
        "30days" | "30d" => "30days".into(),
        "ytd" => "ytd".into(),
        "1year" | "3m" | "6m" | "12m" | "1y" => "1year".into(),
        "4years" | "4y" | "5y" => "4years".into(),
        "10years" | "10y" | "15y" => "10years".into(),
        "all" | "max" => "all".into(),
        y if y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()) => y.to_owned(),
        _ => return None,
    })
}

fn name(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.chars().count() <= MAX_NAME)
        .map(str::to_owned)
}

fn pair(v: &Value) -> Option<(String, String)> {
    Some((name(&v["provider"])?, name(&v["model"])?))
}

fn pick(v: &Value, options: &[&'static str]) -> Option<&'static str> {
    let s = v.as_str()?;
    options.iter().copied().find(|o| *o == s)
}

// ---- Reading ------------------------------------------------------------------------------------

impl UiPrefs {
    /// Reads a migrated (v1) document: every field validated alone.
    pub fn read(doc: &Value) -> UiPrefs {
        let ui = &doc["ui"];
        let d = UiPrefs::default();
        let bool_or = |k: &str, def: bool| ui[k].as_bool().unwrap_or(def);
        let lock_v = &ui["appLockPolicy"];
        let lb = |k: &str, def: bool| lock_v[k].as_bool().unwrap_or(def);
        let lock = LockPolicy {
            auto_lock: lb("autoLockWhenIdle", false),
            idle_minutes: lock_v["idleMinutes"]
                .as_u64()
                .filter(|m| (1..=1440).contains(m))
                .map_or(5, |m| m as u32),
            require_on_launch: lb("requirePassphraseOnLaunch", false),
            lock_on_close: lb("lockOnWindowClose", false),
            touch_id_unlock: lb("touchIdUnlock", false),
        };
        // `restoredPrivacy`: the mode (or the legacy boolean), raised to "hide amounts" at launch
        // when the user asked amounts to start hidden.
        let hide_on_unlock = ui["hideAmountsOnUnlock"].as_bool() == Some(true);
        let mut privacy = match ui["privacyMode"].as_str() {
            Some("normal") => Privacy::Normal,
            Some("hideAmounts") => Privacy::HideAmounts,
            Some("hideAll") => Privacy::HideAll,
            _ if ui["hideSensitive"].as_bool() == Some(true) => Privacy::HideAmounts,
            _ => d.privacy,
        };
        if hide_on_unlock && privacy == Privacy::Normal {
            privacy = Privacy::HideAmounts;
        }
        let mut favorites: Vec<(String, String)> = Vec::new();
        for f in ui["assistantModelFavorites"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(pair)
        {
            if !favorites.contains(&f) && favorites.len() < MAX_FAVORITES {
                favorites.push(f);
            }
        }
        let tool_profiles = ui["assistantProviderToolProfiles"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(p, _)| !p.is_empty() && p.chars().count() <= MAX_NAME)
            .filter_map(|(p, v)| {
                Some((p.clone(), pick(v, &["full", "scoped", "core"])?.to_owned()))
            })
            .take(MAX_PROFILES)
            .collect();
        let book_periods = ui["bookChartPeriods"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, _)| is_book_key(k))
            .filter_map(|(k, v)| Some((k.clone(), normalize_period(v.as_str()?)?)))
            .take(MAX_BOOKS)
            .collect();
        let mut table_sorts = BTreeMap::new();
        if let Some(sorts) = ui["tableSorts"].as_object() {
            for (table, keys) in TABLES {
                match sorts.get(table) {
                    Some(Value::Null) => {
                        table_sorts.insert(table, None);
                    }
                    Some(s) => {
                        let key = s["key"].as_str().filter(|k| keys.contains(k));
                        let dir = s["direction"]
                            .as_str()
                            .filter(|d| matches!(*d, "asc" | "desc"));
                        if let (Some(key), Some(dir)) = (key, dir) {
                            table_sorts.insert(
                                table,
                                Some(Sort {
                                    key: key.to_owned(),
                                    desc: dir == "desc",
                                }),
                            );
                        }
                    }
                    None => {}
                }
            }
        }
        UiPrefs {
            lang: match ui["lang"].as_str() {
                Some("de") => Lang::De,
                Some("en") => Lang::En,
                _ => d.lang,
            },
            currency: match ui["currency"].as_str() {
                Some("eur") => Currency::Fiat,
                Some("btc") => Currency::Btc,
                _ => d.currency,
            },
            theme: match ui["theme"].as_str() {
                Some("system") => ThemeMode::System,
                Some("light") => ThemeMode::Light,
                Some("dark") => ThemeMode::Dark,
                _ => d.theme,
            },
            privacy,
            hide_on_unlock,
            clear_clipboard: bool_or("clearClipboard", d.clear_clipboard),
            // Added without a version bump in the web: only an explicit `true` opts in.
            desktop_notifications: ui["desktopNotifications"].as_bool() == Some(true),
            hide_on_screen_share: bool_or("hideAmountsWhileScreenShared", d.hide_on_screen_share),
            exclude_unencrypted_backups: bool_or(
                "excludeUnencryptedFromBackups",
                d.exclude_unencrypted_backups,
            ),
            scale: normalize_scale(&ui["appScale"]).unwrap_or(d.scale),
            explorer_fallbacks_allowed: ui["explorerSettings"]["publicFallbacksAllowed"].as_bool()
                != Some(false),
            lock,
            ai_features: bool_or("aiFeaturesEnabled", d.ai_features),
            developer_tools: bool_or("developerToolsEnabled", d.developer_tools),
            alpha_visible: bool_or("preAlphaBannerVisible", d.alpha_visible),
            nav_collapsed: ui["navCollapsed"].as_bool() == Some(true),
            model_selection: pair(&ui["assistantModelSelection"]),
            model_favorites: favorites,
            dock_position: pick(&ui["assistantDockPosition"], &["left", "center", "right"])
                .unwrap_or(d.dock_position),
            closed_bar: pick(
                &ui["assistantClosedBar"],
                &["composer", "launcher", "hidden"],
            )
            .unwrap_or(d.closed_bar),
            assistant_panel_open: ui["assistantPanelOpen"].as_bool() == Some(true),
            // Saved before the panel held more than the assistant: open exactly when it was.
            right_panel_open: ui["rightPanelOpen"]
                .as_bool()
                .unwrap_or(ui["assistantPanelOpen"].as_bool() == Some(true)),
            side_panel_width: normalize_width(&ui["sidePanelWidth"]).unwrap_or(d.side_panel_width),
            permission_mode: pick(
                &ui["assistantDefaultPermissionMode"],
                &["approval", "auto", "full_access"],
            )
            .unwrap_or(d.permission_mode),
            tool_profiles,
            chat_modes: ui["assistantChatPermissionModes"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(id, _)| !id.is_empty() && id.chars().count() <= MAX_NAME)
                .filter_map(|(id, v)| {
                    Some((id.clone(), pick(v, &["approval", "auto", "full_access"])?))
                })
                .take(MAX_PROFILES)
                .collect(),
            book_periods,
            table_sorts,
            holdings_view: HoldingsView::from_json(&ui["holdingsView"]),
            overview_layout: crate::overview::grid::model::Layout::from_json(&ui["overviewLayout"]),
            last_route: doc["lastRoute"].as_str().and_then(route_root),
            identity: crate::onboarding::model::Identity::from_json(&ui["identity"]),
            first_sync_done: ui["firstSyncDone"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, v)| is_identity_key(k) && v.as_bool() == Some(true))
                .map(|(k, _)| (k.clone(), true))
                .take(MAX_BOOKS)
                .collect(),
        }
    }

    /// The `ui` object and `lastRoute` this state writes (keys in a fixed order).
    pub fn write(&self, doc: &mut Map<String, Value>) {
        let mut ui = Map::new();
        let s = |v: &str| Value::String(v.to_owned());
        ui.insert("lang".into(), s(self.lang.code()));
        ui.insert(
            "currency".into(),
            s(if self.currency == Currency::Fiat {
                "eur"
            } else {
                "btc"
            }),
        );
        ui.insert(
            "theme".into(),
            s(match self.theme {
                ThemeMode::System => "system",
                ThemeMode::Light => "light",
                ThemeMode::Dark => "dark",
            }),
        );
        ui.insert(
            "privacyMode".into(),
            s(match self.privacy {
                Privacy::Normal => "normal",
                Privacy::HideAmounts => "hideAmounts",
                Privacy::HideAll => "hideAll",
            }),
        );
        ui.insert("hideAmountsOnUnlock".into(), self.hide_on_unlock.into());
        ui.insert("clearClipboard".into(), self.clear_clipboard.into());
        ui.insert(
            "desktopNotifications".into(),
            self.desktop_notifications.into(),
        );
        ui.insert(
            "hideAmountsWhileScreenShared".into(),
            self.hide_on_screen_share.into(),
        );
        ui.insert(
            "excludeUnencryptedFromBackups".into(),
            self.exclude_unencrypted_backups.into(),
        );
        ui.insert(
            "appScale".into(),
            json!((f64::from(self.scale) * 100.0).round() / 100.0),
        );
        ui.insert(
            "explorerSettings".into(),
            json!({"publicFallbacksAllowed": self.explorer_fallbacks_allowed}),
        );
        let l = &self.lock;
        ui.insert(
            "appLockPolicy".into(),
            json!({
                "autoLockWhenIdle": l.auto_lock,
                "idleMinutes": l.idle_minutes,
                "requirePassphraseOnLaunch": l.require_on_launch,
                "lockOnWindowClose": l.lock_on_close,
                "touchIdUnlock": l.touch_id_unlock,
            }),
        );
        ui.insert("aiFeaturesEnabled".into(), self.ai_features.into());
        ui.insert("developerToolsEnabled".into(), self.developer_tools.into());
        ui.insert("preAlphaBannerVisible".into(), self.alpha_visible.into());
        ui.insert("navCollapsed".into(), self.nav_collapsed.into());
        ui.insert(
            "assistantModelSelection".into(),
            self.model_selection
                .as_ref()
                .map_or(Value::Null, |(p, m)| json!({"provider": p, "model": m})),
        );
        ui.insert(
            "assistantModelFavorites".into(),
            Value::Array(
                self.model_favorites
                    .iter()
                    .map(|(p, m)| json!({"provider": p, "model": m}))
                    .collect(),
            ),
        );
        ui.insert("assistantDockPosition".into(), s(self.dock_position));
        ui.insert("assistantClosedBar".into(), s(self.closed_bar));
        ui.insert(
            "assistantPanelOpen".into(),
            self.assistant_panel_open.into(),
        );
        ui.insert("rightPanelOpen".into(), self.right_panel_open.into());
        ui.insert(
            "sidePanelWidth".into(),
            json!(self.side_panel_width.round() as i64),
        );
        ui.insert(
            "assistantDefaultPermissionMode".into(),
            s(self.permission_mode),
        );
        ui.insert(
            "assistantProviderToolProfiles".into(),
            json!(self.tool_profiles),
        );
        ui.insert(
            "assistantChatPermissionModes".into(),
            json!(self.chat_modes),
        );
        ui.insert("bookChartPeriods".into(), json!(self.book_periods));
        let mut sorts = Map::new();
        for (table, sort) in &self.table_sorts {
            let v = sort.as_ref().map_or(
                Value::Null,
                |s| json!({"key": s.key, "direction": if s.desc { "desc" } else { "asc" }}),
            );
            sorts.insert((*table).to_owned(), v);
        }
        ui.insert("tableSorts".into(), Value::Object(sorts));
        ui.insert(
            "identity".into(),
            self.identity.as_ref().map_or(Value::Null, |i| i.to_json()),
        );
        ui.insert("firstSyncDone".into(), json!(self.first_sync_done));
        ui.insert("holdingsView".into(), self.holdings_view.to_json());
        ui.insert("overviewLayout".into(), self.overview_layout.to_json());
        doc.insert("ui".into(), Value::Object(ui));
        match &self.last_route {
            Some(r) => doc.insert("lastRoute".into(), s(r)),
            None => doc.remove("lastRoute"),
        };
    }
}

/// `db#<16 hex>`: the book key the per-book maps use (a hash, so no path is stored).
pub fn is_book_key(k: &str) -> bool {
    k.strip_prefix("db#")
        .is_some_and(|h| h.len() == 16 && h.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The book key of a data root: FNV-1a 64 of its path, as `db#<hex>` (`bookIdentityKey`).
pub fn book_key(data_root: Option<&std::path::Path>) -> String {
    let text = data_root
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("db#{h:016x}")
}

/// The first segment of a route path (`/connections/abc?x` → `/connections`): ids, queries and
/// fragments (which can name transactions) never reach the file.
pub fn route_root(route: &str) -> Option<String> {
    let path = route.split(['?', '#']).next()?;
    let seg = path.strip_prefix('/')?.split('/').next()?;
    (!seg.is_empty() && seg.len() <= 40 && seg.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
        .then(|| format!("/{seg}"))
}

// ---- Migrations ---------------------------------------------------------------------------------

/// What loading made of the file.
#[derive(Debug, Default, PartialEq)]
pub struct Migrated {
    /// The document in the current schema (an object; `{}` when unusable).
    pub doc: Value,
    /// The file is not in the current schema: write the migrated one back.
    pub rewrite: bool,
    /// A newer app wrote it: read what we know, never write over it.
    pub read_only: bool,
}

/// One migration per schema version, from that version to the next (`MIGRATIONS[v]` takes a
/// version-`v` document). Add one and bump `SCHEMA_VERSION` to change the shape.
const MIGRATIONS: [fn(Value) -> Value; SCHEMA_VERSION as usize] = [from_v0];

/// v0: a `kb.ui`-shaped object without `schemaVersion` (flat, or zustand's
/// `{"state": {...}, "version": n}` envelope). Applies the web's own legacy merges (the hide
/// boolean → privacy mode, the two old panel widths → one, `overviewChartPeriods` →
/// `bookChartPeriods`) and drops what never belongs on disk (`notifications`, `identity`).
fn from_v0(raw: Value) -> Value {
    let mut ui = match raw.get("state") {
        Some(Value::Object(s)) => s.clone(),
        _ => raw.as_object().cloned().unwrap_or_default(),
    };
    if !ui.contains_key("privacyMode")
        && ui.get("hideSensitive").and_then(Value::as_bool) == Some(true)
    {
        ui.insert("privacyMode".into(), "hideAmounts".into());
    }
    if !ui.contains_key("sidePanelWidth") {
        let legacy = ["detailPanelWidth", "assistantPanelWidth"]
            .iter()
            .filter_map(|k| ui.get(*k).and_then(Value::as_f64))
            .filter(|x| x.is_finite())
            .reduce(f64::max);
        if let Some(w) = legacy {
            ui.insert("sidePanelWidth".into(), json!(w));
        }
    }
    if !ui.contains_key("bookChartPeriods")
        && let Some(p) = ui.get("overviewChartPeriods").cloned()
    {
        ui.insert("bookChartPeriods".into(), p);
    }
    for gone in [
        "notifications",
        "identity",
        "hideSensitive",
        "detailPanelWidth",
        "assistantPanelWidth",
        "overviewChartPeriods",
        "assistantChatPermissionModes",
        "sourceFundsDrafts",
        "firstSyncDone",
    ] {
        ui.remove(gone);
    }
    let last_route = ui.remove("lastRoute");
    let mut doc = json!({"schemaVersion": 1, "ui": Value::Object(ui)});
    if let Some(r) = last_route {
        doc["lastRoute"] = r;
    }
    doc
}

/// Brings any file's JSON to the current schema.
pub fn migrate(raw: Option<Value>) -> Migrated {
    let Some(raw) = raw.filter(Value::is_object) else {
        return Migrated {
            doc: json!({}),
            rewrite: false,
            read_only: false,
        };
    };
    // `!(v >= n)` style: a missing or non-numeric version is the oldest.
    let version = raw
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if version > SCHEMA_VERSION {
        return Migrated {
            doc: raw,
            rewrite: false,
            read_only: true,
        };
    }
    let mut doc = raw;
    for step in &MIGRATIONS[version as usize..] {
        doc = step(doc);
    }
    if !doc["ui"].is_object() {
        doc["ui"] = json!({});
    }
    Migrated {
        doc,
        rewrite: version < SCHEMA_VERSION,
        read_only: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(p: &UiPrefs) -> UiPrefs {
        let mut doc = Map::new();
        doc.insert("schemaVersion".into(), json!(SCHEMA_VERSION));
        p.write(&mut doc);
        UiPrefs::read(&Value::Object(doc))
    }

    #[test]
    fn defaults_when_nothing_is_stored() {
        assert_eq!(UiPrefs::read(&json!({})), UiPrefs::default());
        assert_eq!(UiPrefs::read(&migrate(None).doc), UiPrefs::default());
        let d = UiPrefs::default();
        assert_eq!(
            (
                d.theme,
                d.scale,
                d.clear_clipboard,
                d.ai_features,
                d.developer_tools
            ),
            (ThemeMode::Dark, 1.0, true, true, false)
        );
    }

    #[test]
    fn every_field_round_trips() {
        let mut p = UiPrefs {
            lang: Lang::De,
            currency: Currency::Fiat,
            theme: ThemeMode::Light,
            privacy: Privacy::HideAll,
            hide_on_unlock: true,
            clear_clipboard: false,
            desktop_notifications: true,
            hide_on_screen_share: false,
            exclude_unencrypted_backups: false,
            scale: 1.15,
            explorer_fallbacks_allowed: false,
            lock: LockPolicy {
                auto_lock: true,
                idle_minutes: 15,
                require_on_launch: true,
                lock_on_close: true,
                touch_id_unlock: true,
            },
            ai_features: false,
            developer_tools: true,
            alpha_visible: false,
            nav_collapsed: true,
            model_selection: Some(("ollama".into(), "qwen3.6:35b".into())),
            model_favorites: vec![("claude".into(), "default".into())],
            dock_position: "center",
            closed_bar: "launcher",
            assistant_panel_open: true,
            right_panel_open: true,
            side_panel_width: 720.0,
            permission_mode: "approval",
            tool_profiles: [("ollama".to_owned(), "core".to_owned())].into(),
            chat_modes: [("chat-1".to_owned(), "full_access")].into(),
            book_periods: [(
                book_key(Some(std::path::Path::new("/b"))),
                "2024".to_owned(),
            )]
            .into(),
            table_sorts: BTreeMap::new(),
            holdings_view: HoldingsView {
                layout: crate::overview::holdings_model::Layout::Trend,
                sort: crate::overview::holdings_model::Sort::Name,
                empty: crate::overview::holdings_model::Empty::Show,
                rails: vec![crate::overview::holdings_model::Group::Liquid],
                limit: 0,
            },
            overview_layout: {
                use crate::overview::grid::model::{Layout, Size, WidgetId};
                let mut l = Layout::default();
                l.add(WidgetId::Assets, Size::Small);
                l.remove(WidgetId::Stats);
                l.set_size(WidgetId::Chart, Size::Large);
                l.move_to(WidgetId::TaxYear, 0);
                l
            },
            last_route: Some("/connections".into()),
            identity: Some(crate::onboarding::model::Identity {
                name: "Private".into(),
                workspace: "My Books".into(),
                profile: "Private".into(),
                country: "AT".into(),
                encrypted: true,
                imported: Some(book_key(Some(std::path::Path::new("/b")))),
                ..Default::default()
            }),
            first_sync_done: [("id:My Books/Private".to_owned(), true)].into(),
        };
        p.table_sorts.insert(
            "wallets",
            Some(Sort {
                key: "balance".into(),
                desc: true,
            }),
        );
        p.table_sorts.insert("transactions", None);
        p.table_sorts.insert(
            "review",
            Some(Sort {
                key: "date".into(),
                desc: false,
            }),
        );
        assert_eq!(roundtrip(&p), p);
    }

    #[test]
    fn invalid_fields_fall_back_alone() {
        let doc = json!({"schemaVersion": 1, "ui": {
            "theme": "purple", "lang": "fr", "currency": 3, "appScale": 7, "privacyMode": "everything",
            "clearClipboard": "yes", "desktopNotifications": "true", "navCollapsed": 1,
            "sidePanelWidth": 99999, "assistantDockPosition": "top", "assistantClosedBar": null,
            "assistantDefaultPermissionMode": "root", "assistantModelSelection": {"provider": "", "model": "m"},
            "assistantModelFavorites": [{"provider": "a", "model": "b"}, {"provider": "a", "model": "b"}, 5, {"provider": "x"}],
            "assistantProviderToolProfiles": {"a": "full", "b": "everything", "": "core"},
            "appLockPolicy": {"autoLockWhenIdle": true, "idleMinutes": 0},
            "bookChartPeriods": {"db#0123456789abcdef": "1y", "db#0123456789abcde0": "forever", "/home/me": "all"},
            "tableSorts": {"wallets": {"key": "bogus", "direction": "asc"}, "transactions": {"key": "amount", "direction": "desc"}, "other": null},
            "developerToolsEnabled": true,
            "holdingsView": {"layout": "grouped", "sort": "activity", "rails": [], "limit": 3, "empty": "hide"},
            "overviewLayout": {"version": 2, "tiles": [{"id": "flows", "size": "large"}, {"id": "chart", "size": "small"}, {"id": "ticker", "size": "medium"}, {"id": "flows", "size": "medium"}, 7]},
            "unknownField": {"deep": [1, 2, 3]},
        }, "lastRoute": "/transactions?tx=abc", "window": "nope"});
        let p = UiPrefs::read(&doc);
        let d = UiPrefs::default();
        assert_eq!(
            (p.theme, p.lang, p.currency, p.privacy),
            (d.theme, d.lang, d.currency, d.privacy)
        );
        assert_eq!(
            p.scale, 1.2,
            "out-of-range scales clamp, as normalizeAppScale does"
        );
        assert!(p.clear_clipboard && !p.desktop_notifications && !p.nav_collapsed);
        assert_eq!(p.side_panel_width, SIDE_PANEL_MAX_WIDTH);
        assert_eq!(
            (p.dock_position, p.closed_bar, p.permission_mode),
            ("right", "hidden", "auto")
        );
        assert_eq!(p.model_selection, None);
        assert_eq!(p.model_favorites, vec![("a".to_owned(), "b".to_owned())]);
        assert_eq!(
            p.tool_profiles,
            [("a".to_owned(), "full".to_owned())].into()
        );
        assert_eq!(
            p.lock,
            LockPolicy {
                auto_lock: true,
                ..LockPolicy::default()
            }
        );
        assert_eq!(
            p.book_periods,
            [("db#0123456789abcdef".to_owned(), "1year".to_owned())].into()
        );
        assert_eq!(p.table_sorts.len(), 1);
        assert_eq!(
            p.table_sorts["transactions"],
            Some(Sort {
                key: "amount".into(),
                desc: true
            })
        );
        assert!(p.developer_tools);
        // Each holdings field alone: a dropped layout, an emptied rail filter and an unknown
        // limit take their defaults; the valid sort and empty mode stay.
        let hv = &p.holdings_view;
        assert_eq!(
            (hv.layout, hv.sort, hv.empty, hv.limit),
            (
                d.holdings_view.layout,
                crate::overview::holdings_model::Sort::Activity,
                crate::overview::holdings_model::Empty::Hide,
                8
            )
        );
        assert_eq!(hv.rails, d.holdings_view.rails);
        // The overview layout tile by tile: the valid one stays, a size the widget does not
        // come in becomes its default, unknown widgets and repeats are dropped.
        {
            use crate::overview::grid::model::{Size, WidgetId};
            let l = &p.overview_layout;
            assert_eq!(l.len(), 2);
            assert_eq!(l.size_of(WidgetId::Flows), Some(Size::Large));
            assert_eq!(
                l.size_of(WidgetId::Chart),
                Some(WidgetId::Chart.default_size())
            );
        }
        // The free grid's document (version 1) keeps what was shown, in reading order.
        {
            use crate::overview::grid::model::WidgetId;
            let old = json!({"ui": {"overviewLayout": {"version": 1, "widgets": [
                {"id": "flows", "x": 0, "y": 60, "w": 12, "h": 8, "visible": true},
                {"id": "stats", "x": 0, "y": 0, "w": 12, "h": 4, "visible": false},
                {"id": "recent", "x": 0, "y": 10, "w": 4, "h": 20, "visible": true}]}}});
            let l = UiPrefs::read(&old).overview_layout;
            assert!(!l.contains(WidgetId::Stats));
            assert!(l.index_of(WidgetId::Recent) < l.index_of(WidgetId::Flows));
        }
        assert_eq!(
            UiPrefs::read(&json!({"ui": {"overviewLayout": "grid"}})).overview_layout,
            d.overview_layout
        );
        assert_eq!(p.last_route.as_deref(), Some("/transactions"));
    }

    #[test]
    fn hide_on_unlock_raises_privacy_at_launch() {
        let p =
            UiPrefs::read(&json!({"ui": {"privacyMode": "normal", "hideAmountsOnUnlock": true}}));
        assert_eq!(p.privacy, Privacy::HideAmounts);
        let p =
            UiPrefs::read(&json!({"ui": {"privacyMode": "hideAll", "hideAmountsOnUnlock": true}}));
        assert_eq!(p.privacy, Privacy::HideAll);
    }

    #[test]
    fn right_panel_follows_the_legacy_assistant_flag() {
        assert!(UiPrefs::read(&json!({"ui": {"assistantPanelOpen": true}})).right_panel_open);
        assert!(
            !UiPrefs::read(&json!({"ui": {"assistantPanelOpen": true, "rightPanelOpen": false}}))
                .right_panel_open
        );
    }

    #[test]
    fn routes_keep_only_their_first_segment() {
        assert_eq!(
            route_root("/connections/7a8a-uuid").as_deref(),
            Some("/connections")
        );
        assert_eq!(
            route_root("/transactions?tx=deadbeef#t").as_deref(),
            Some("/transactions")
        );
        assert_eq!(route_root("/"), None);
        assert_eq!(route_root("overview"), None);
        assert_eq!(route_root("/Overview"), None);
    }

    #[test]
    fn book_keys_hash_the_path() {
        let k = book_key(Some(std::path::Path::new("/Users/me/book/data")));
        assert!(is_book_key(&k));
        assert!(!k.contains("Users"));
        assert_ne!(k, book_key(Some(std::path::Path::new("/other"))));
    }

    #[test]
    fn migrates_a_kb_ui_blob() {
        // The web store's localStorage envelope, from before privacy modes and one panel width.
        let raw = json!({"state": {
            "theme": "light", "hideSensitive": true, "detailPanelWidth": 700, "assistantPanelWidth": 520,
            "overviewChartPeriods": {"db#0123456789abcdef": "all"},
            "notifications": [{"title": "Sync failed", "body": "secret-ish"}],
            "identity": {"name": "My Books", "importedProject": {"dataRoot": "/Users/me/x"}},
        }, "version": 2});
        let m = migrate(Some(raw));
        assert!(m.rewrite && !m.read_only);
        assert_eq!(m.doc["schemaVersion"], 1);
        assert!(
            m.doc["ui"].get("notifications").is_none() && m.doc["ui"].get("identity").is_none()
        );
        let p = UiPrefs::read(&m.doc);
        assert_eq!(
            (p.theme, p.privacy, p.side_panel_width),
            (ThemeMode::Light, Privacy::HideAmounts, 700.0)
        );
        assert_eq!(p.book_periods["db#0123456789abcdef"], "all");
    }

    #[test]
    fn current_files_load_without_rewrite_and_newer_ones_stay_untouched() {
        let m = migrate(Some(json!({"schemaVersion": 1, "ui": {"theme": "light"}})));
        assert!(!m.rewrite && !m.read_only);
        assert_eq!(UiPrefs::read(&m.doc).theme, ThemeMode::Light);
        let m = migrate(Some(
            json!({"schemaVersion": 99, "ui": {"theme": "light", "newThing": 1}}),
        ));
        assert!(m.read_only && !m.rewrite);
        assert_eq!(UiPrefs::read(&m.doc).theme, ThemeMode::Light);
        // Not an object, or a broken `ui`: defaults.
        assert_eq!(migrate(Some(json!([1, 2]))).doc, json!({}));
        assert_eq!(
            UiPrefs::read(&migrate(Some(json!({"schemaVersion": 1, "ui": 5}))).doc),
            UiPrefs::default()
        );
    }

    #[test]
    fn written_files_hold_no_paths_or_ids() {
        let mut p = UiPrefs::default();
        p.book_periods.insert(
            book_key(Some(std::path::Path::new("/Users/me/secret-book"))),
            "all".into(),
        );
        p.last_route = route_root("/connections/0f6c-wallet-id");
        let mut doc = Map::new();
        p.write(&mut doc);
        let text = serde_json::to_string(&doc).unwrap();
        assert!(
            !text.contains("secret-book")
                && !text.contains("wallet-id")
                && !text.contains("/Users")
        );
    }
}
