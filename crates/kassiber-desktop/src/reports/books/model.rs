//! The data behind Books: `ui.profiles.snapshot` parsed into book sets and books, the tax
//! country registry the pickers list (lib/taxCountries.generated.ts), method/region labels and
//! the pure helpers of Books.tsx and books-profile-settings.ts.

use serde_json::Value;

use crate::i18n::{Lang, lang, t};

/// One book (`Profile` in mocks/profiles.ts).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub tax_policy: String,
    pub fiat_currency: String,
    pub tax_country: Option<String>,
    pub gains_algorithm: Option<String>,
    pub pool_scope: Option<String>,
    pub allowed_pool_scopes: Option<Vec<String>>,
    pub accounts: String,
    pub wallets: String,
    pub last_opened: String,
    pub preview: bool,
}

impl Profile {
    /// `profile.taxCountry ?? "generic"`.
    pub fn country(&self) -> &str {
        self.tax_country.as_deref().unwrap_or("generic")
    }
}

/// One book set (`Workspace`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub currency: String,
    pub jurisdiction: String,
    pub created: String,
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Default)]
pub struct Profiles {
    pub workspaces: Vec<Workspace>,
    pub active_profile: String,
    pub active_workspace: String,
}

fn s(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

pub fn parse(p: &Value) -> Profiles {
    let workspaces = p["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| Workspace {
            id: s(&w["id"]),
            name: s(&w["name"]),
            currency: s(&w["currency"]),
            jurisdiction: s(&w["jurisdiction"]),
            created: s(&w["created"]),
            profiles: w["profiles"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| Profile {
                    id: s(&x["id"]),
                    name: s(&x["name"]),
                    tax_policy: s(&x["taxPolicy"]),
                    fiat_currency: s(&x["fiatCurrency"]),
                    tax_country: x["taxCountry"].as_str().map(str::to_owned),
                    gains_algorithm: x["gainsAlgorithm"].as_str().map(str::to_owned),
                    pool_scope: x["costBasisPoolScope"].as_str().map(str::to_owned),
                    allowed_pool_scopes: x["allowedCostBasisPoolScopes"].as_array().map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    }),
                    accounts: s(&x["accounts"]),
                    wallets: s(&x["wallets"]),
                    last_opened: s(&x["lastOpened"]),
                    preview: x["taxCountryPreview"].as_bool().unwrap_or(false),
                })
                .collect(),
        })
        .collect();
    Profiles {
        workspaces,
        active_profile: s(&p["activeProfileId"]),
        active_workspace: s(&p["activeWorkspaceId"]),
    }
}

impl Profiles {
    pub fn workspace(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }
    pub fn profile(&self, ws: &str, id: &str) -> Option<&Profile> {
        self.workspace(ws)
            .and_then(|w| w.profiles.iter().find(|p| p.id == id))
    }
    pub fn profile_count(&self) -> usize {
        self.workspaces.iter().map(|w| w.profiles.len()).sum()
    }
}

/// `BOOK_FILTER_THRESHOLD`: the filter field shows only above this many books.
pub const BOOK_FILTER_THRESHOLD: usize = 6;

/// `filterWorkspaces`: books whose name or tax policy match, under their set; a set whose own
/// name matches keeps all of its books.
pub fn filter_workspaces(workspaces: &[Workspace], query: &str) -> Vec<Workspace> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return workspaces.to_vec();
    }
    workspaces
        .iter()
        .map(|w| {
            if w.name.to_lowercase().contains(&needle) {
                w.clone()
            } else {
                Workspace {
                    profiles: w
                        .profiles
                        .iter()
                        .filter(|p| {
                            format!("{} {}", p.name, p.tax_policy)
                                .to_lowercase()
                                .contains(&needle)
                        })
                        .cloned()
                        .collect(),
                    ..w.clone()
                }
            }
        })
        .filter(|w| !w.profiles.is_empty())
        .collect()
}

/// `bookInitials`: the first letters of the first two words, upper-cased; "?" for none.
pub fn initials(name: &str) -> String {
    let s: String = name
        .split_whitespace()
        .filter_map(|p| p.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if s.is_empty() { "?".into() } else { s }
}

/// `formatWorkspaceMeta`: currency · jurisdiction · since <created>.
pub fn workspace_meta(w: &Workspace, include_created: bool) -> String {
    let since = (include_created && !w.created.is_empty())
        .then(|| crate::i18n::tr("onboarding:books.meta.since", &[("date", &w.created)]));
    [
        Some(w.currency.clone()),
        Some(w.jurisdiction.clone()),
        since,
    ]
    .into_iter()
    .flatten()
    .filter(|x| !x.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

// ---- Tax country registry --------------------------------------------------------------------

/// A country's long-term holding rule (`long_term` in the registry).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LongTerm {
    /// A day count the book chooses (generic).
    Days,
    /// Held more than one year.
    MoreThanOneYear,
    /// No long-term distinction.
    Never,
}

/// The registry entry fields the Books pickers and the book settings dialog need.
pub struct Country {
    pub code: &'static str,
    pub default_method: &'static str,
    pub default_pool_scope: &'static str,
    pub methods: &'static [&'static str],
    pub pool_scopes: &'static [&'static str],
    pub preview: bool,
    pub name_en: &'static str,
    pub name_de: &'static str,
    /// The fixed currency; None where the book chooses.
    pub fiat: Option<&'static str>,
    pub long_term: LongTerm,
    pub timezone: &'static str,
    /// `MM-DD`.
    pub tax_year_start: &'static str,
    pub tax_year_editable: bool,
}

#[allow(clippy::too_many_arguments)]
const fn c(
    code: &'static str,
    (name_en, name_de): (&'static str, &'static str),
    preview: bool,
    methods: &'static [&'static str],
    default_method: &'static str,
    pool_scopes: &'static [&'static str],
    default_pool_scope: &'static str,
    fiat: Option<&'static str>,
    long_term: LongTerm,
    (timezone, tax_year_start, tax_year_editable): (&'static str, &'static str, bool),
) -> Country {
    Country {
        code,
        default_method,
        default_pool_scope,
        methods,
        pool_scopes,
        preview,
        name_en,
        name_de,
        fiat,
        long_term,
        timezone,
        tax_year_start,
        tax_year_editable,
    }
}

/// The tax country registry (`crates/tax-countries/src/lib.rs` `COUNTRIES`), in picker order.
/// The settings dialog takes a book's own choices from `ui.profiles.settings`; this copy names
/// the regions and seeds a new book or a region switch (no daemon kind lists the registry).
pub const COUNTRIES: &[Country] = &[
    c(
        "generic",
        ("Generic", "Allgemein"),
        false,
        &["FIFO", "HIFO", "LIFO", "LOFO", "MOVING_AVERAGE"],
        "FIFO",
        &["global"],
        "global",
        None,
        LongTerm::Days,
        ("UTC", "01-01", true),
    ),
    c(
        "at",
        ("Austria", "Österreich"),
        false,
        &[
            "FIFO",
            "HIFO",
            "LIFO",
            "LOFO",
            "MOVING_AVERAGE",
            "MOVING_AVERAGE_AT",
        ],
        "MOVING_AVERAGE_AT",
        &["global"],
        "global",
        Some("EUR"),
        LongTerm::Never,
        ("Europe/Vienna", "01-01", false),
    ),
    c(
        "us",
        ("United States", "Vereinigte Staaten"),
        true,
        &["FIFO", "HIFO", "LIFO", "LOFO"],
        "FIFO",
        &["global", "global_then_wallet", "wallet"],
        "wallet",
        Some("USD"),
        LongTerm::MoreThanOneYear,
        ("UTC", "01-01", false),
    ),
    c(
        "de",
        ("Germany", "Deutschland"),
        true,
        &["FIFO", "MOVING_AVERAGE_FIFO_PERIOD"],
        "FIFO",
        &["wallet"],
        "wallet",
        Some("EUR"),
        LongTerm::MoreThanOneYear,
        ("Europe/Berlin", "01-01", false),
    ),
    c(
        "ch",
        ("Switzerland", "Schweiz"),
        true,
        &["FIFO", "MOVING_AVERAGE"],
        "FIFO",
        &["global"],
        "global",
        Some("CHF"),
        LongTerm::Never,
        ("Europe/Zurich", "01-01", false),
    ),
    c(
        "ie",
        ("Ireland", "Irland"),
        true,
        &["FIFO"],
        "FIFO",
        &["global"],
        "global",
        Some("EUR"),
        LongTerm::Never,
        ("Europe/Dublin", "01-01", false),
    ),
    c(
        "gb",
        ("United Kingdom", "Vereinigtes Königreich"),
        true,
        &["UK_SHARE_MATCHING"],
        "UK_SHARE_MATCHING",
        &["global"],
        "global",
        Some("GBP"),
        LongTerm::Never,
        ("Europe/London", "04-06", false),
    ),
    c(
        "es",
        ("Spain", "Spanien"),
        true,
        &["FIFO"],
        "FIFO",
        &["global"],
        "global",
        Some("EUR"),
        LongTerm::Never,
        ("Europe/Madrid", "01-01", false),
    ),
    c(
        "jp",
        ("Japan", "Japan"),
        true,
        &["JP_MOVING_AVERAGE", "JP_TOTAL_AVERAGE"],
        "JP_TOTAL_AVERAGE",
        &["global"],
        "global",
        Some("JPY"),
        LongTerm::Never,
        ("Asia/Tokyo", "01-01", false),
    ),
    c(
        "au",
        ("Australia", "Australien"),
        true,
        &["FIFO"],
        "FIFO",
        &["global"],
        "global",
        Some("AUD"),
        LongTerm::MoreThanOneYear,
        ("Australia/Sydney", "07-01", false),
    ),
    c(
        "nz",
        ("New Zealand", "Neuseeland"),
        true,
        &["FIFO"],
        "FIFO",
        &["global"],
        "global",
        Some("NZD"),
        LongTerm::Never,
        ("Pacific/Auckland", "04-01", false),
    ),
    c(
        "in",
        ("India", "Indien"),
        true,
        &["FIFO"],
        "FIFO",
        &["global"],
        "global",
        Some("INR"),
        LongTerm::Never,
        ("Asia/Kolkata", "04-01", false),
    ),
    c(
        "za",
        ("South Africa", "Südafrika"),
        true,
        &["FIFO"],
        "FIFO",
        &["global"],
        "global",
        Some("ZAR"),
        LongTerm::Never,
        ("Africa/Johannesburg", "03-01", false),
    ),
];

pub fn country(code: &str) -> Option<&'static Country> {
    let c = code.trim().to_lowercase();
    COUNTRIES.iter().find(|x| x.code == c)
}

/// `isPreviewTaxCountry`.
pub fn is_preview(code: &str) -> bool {
    country(code).is_some_and(|c| c.preview)
}

/// `PICKER_ORDER`.
const PICKER_ORDER: [&str; 10] = [
    "FIFO",
    "LIFO",
    "HIFO",
    "LOFO",
    "MOVING_AVERAGE",
    "MOVING_AVERAGE_AT",
    "MOVING_AVERAGE_FIFO_PERIOD",
    "UK_SHARE_MATCHING",
    "JP_TOTAL_AVERAGE",
    "JP_MOVING_AVERAGE",
];

/// A method's place in the picker (`PICKER_ORDER`); a method the copy doesn't know yet sorts
/// after the known ones.
pub fn picker_rank(method: &str) -> usize {
    PICKER_ORDER
        .iter()
        .position(|m| *m == method)
        .unwrap_or(PICKER_ORDER.len())
}

/// `gainsAlgorithmsFor`: the default first, then the allowed lot methods in picker order; a
/// country with Austria's moving average doesn't offer the plain one.
pub fn methods_for(code: &str) -> Vec<String> {
    let Some(c) = country(code) else {
        return vec![];
    };
    let mut out = vec![c.default_method.to_owned()];
    let has_at = c.methods.contains(&"MOVING_AVERAGE_AT");
    for m in PICKER_ORDER {
        if m != c.default_method && c.methods.contains(&m) && !(m == "MOVING_AVERAGE" && has_at) {
            out.push(m.to_owned());
        }
    }
    out
}

/// `defaultGainsAlgorithm` / `GAINS_ALGORITHM_DEFAULTS`.
pub fn default_method(code: &str) -> String {
    country(code)
        .map(|c| c.default_method)
        .unwrap_or("FIFO")
        .to_owned()
}

/// `defaultPoolScope`.
pub fn default_pool_scope(code: &str) -> String {
    country(code)
        .map(|c| c.default_pool_scope)
        .unwrap_or("global")
        .to_owned()
}

/// `regionLabel`: Austria and generic keep the onboarding wording; others the registry name in
/// the UI language.
pub fn region_label(code: &str) -> String {
    match code {
        "at" => t("onboarding:books.region.at"),
        "generic" => t("onboarding:books.region.generic"),
        _ => match country(code) {
            Some(c) => if lang() == Lang::De {
                c.name_de
            } else {
                c.name_en
            }
            .to_owned(),
            None => code.to_owned(),
        },
    }
}

/// `accountingMethodLabel`: every method the locale names, UK share matching included (the
/// web's Books.tsx shows its raw token; the key exists, so this names it); an unknown method
/// shows its token.
pub fn method_label(method: &str) -> String {
    let up = method.to_uppercase();
    const KEYED: [&str; 10] = [
        "MOVING_AVERAGE_AT",
        "MOVING_AVERAGE",
        "MOVING_AVERAGE_FIFO_PERIOD",
        "FIFO",
        "LIFO",
        "HIFO",
        "LOFO",
        "UK_SHARE_MATCHING",
        "JP_TOTAL_AVERAGE",
        "JP_MOVING_AVERAGE",
    ];
    if KEYED.contains(&up.as_str()) {
        t(&format!("onboarding:books.method.{up}"))
    } else {
        method.to_owned()
    }
}

/// The precise name of a pool scope (`whole book`, `per wallet`, the US 2025 change); a scope
/// this copy has no words for yet shows its registry token.
pub fn pool_scope_label(scope: &str) -> String {
    match scope {
        "global" => t("onboarding:books.settings.pool.global.label"),
        "wallet" => t("onboarding:books.settings.pool.wallet.label"),
        "global_then_wallet" => t("onboarding:books.settings.pool.global_then_wallet.label"),
        other => other.to_owned(),
    }
}

/// The one-line explanation of a pool scope; None for one this copy has no words for.
pub fn pool_scope_help(scope: &str) -> Option<String> {
    match scope {
        "global" | "wallet" | "global_then_wallet" => {
            Some(t(&format!("onboarding:books.settings.pool.{scope}.help")))
        }
        _ => None,
    }
}

const MONTHS_LONG_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_LONG_DE: [&str; 12] = [
    "Jänner",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

/// A day of the year in words: en `1 July`, de `1. Juli`.
pub fn day_of_year(m: u32, d: u32) -> String {
    let i = (m.clamp(1, 12) - 1) as usize;
    if lang() == Lang::De {
        format!("{d}. {}", MONTHS_LONG_DE[i])
    } else {
        format!("{d} {}", MONTHS_LONG_EN[i])
    }
}

/// A date in words: en `1 July 2026`, de `1. Juli 2026`.
pub fn long_date(day: crate::format::Day) -> String {
    format!("{} {}", day_of_year(day.m, day.d), day.y)
}

/// `1 July` / `1. Juli` for an `MM-DD` start; the text itself when it isn't one.
pub fn start_label(start: &str) -> String {
    match super::settings_model::parse_start(start) {
        Some((m, d)) => day_of_year(m, d),
        None => start.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_put_the_default_first_and_drop_plain_average_beside_austrias() {
        assert_eq!(
            methods_for("at"),
            vec!["MOVING_AVERAGE_AT", "FIFO", "LIFO", "HIFO", "LOFO"]
        );
        assert_eq!(
            methods_for("generic"),
            vec!["FIFO", "LIFO", "HIFO", "LOFO", "MOVING_AVERAGE"]
        );
        assert_eq!(
            methods_for("jp"),
            vec!["JP_TOTAL_AVERAGE", "JP_MOVING_AVERAGE"]
        );
    }

    #[test]
    fn filter_keeps_a_matching_set_whole_and_drops_empty_sets() {
        let ws = vec![
            Workspace {
                id: "a".into(),
                name: "Private".into(),
                profiles: vec![Profile {
                    name: "Main".into(),
                    tax_policy: "Generic - FIFO".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Workspace {
                id: "b".into(),
                name: "Company".into(),
                profiles: vec![
                    Profile {
                        name: "Ops".into(),
                        tax_policy: "Austria".into(),
                        ..Default::default()
                    },
                    Profile {
                        name: "Treasury".into(),
                        tax_policy: "Generic".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ];
        assert_eq!(filter_workspaces(&ws, "  ").len(), 2);
        let f = filter_workspaces(&ws, "austria");
        assert_eq!(
            (f.len(), f[0].profiles.len(), f[0].profiles[0].name.as_str()),
            (1, 1, "Ops")
        );
        assert_eq!(filter_workspaces(&ws, "comp")[0].profiles.len(), 2);
        assert!(filter_workspaces(&ws, "zzz").is_empty());
    }

    #[test]
    fn initials_take_two_words() {
        assert_eq!(initials("Full Accounting"), "FA");
        assert_eq!(initials("exchange reconciliation book"), "ER");
        assert_eq!(initials("  "), "?");
    }
}
