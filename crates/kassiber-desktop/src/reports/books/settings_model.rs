//! Book settings, the pure part: `ui.profiles.settings` parsed, the form's validation (the
//! custom tax-year date, the currency, the holding days), which pool scopes a method can use,
//! the `ui.profiles.update` arguments a form asks for, and the dry run's preview parsed
//! (docs/reference/tax.md "Book settings"). Everything here is unit-tested; the dialog in
//! `settings` draws it.
//!
//! The choices a book has (methods, pool scopes, which scopes each method has, tax-year presets,
//! time zones) always come from the daemon's answer, never from a list kept here, so a scope the
//! registry enables later (a generic book's `wallet`) shows up without a change.

use serde_json::{Map, Value, json};

use crate::format::Day;

/// One tax-year preset (`tax_year_presets`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub id: String,
    /// `MM-DD`.
    pub start: String,
    /// The registry's English name, the fallback when the UI has no copy for `id`.
    pub name: String,
}

/// `ui.profiles.settings` (and a dry run's `settings`): one book's settings and the choices its
/// country allows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub profile_id: String,
    pub workspace_id: String,
    pub label: String,
    pub is_current: bool,
    pub tax_country: String,
    pub preview: bool,
    pub fiat_currency: String,
    pub fiat_currency_locked: bool,
    pub long_term_days: Option<i64>,
    pub long_term_days_editable: bool,
    pub method: String,
    pub methods: Vec<String>,
    pub pool: String,
    pub pools: Vec<String>,
    /// (method, the scopes it has), in the daemon's order.
    pub method_pools: Vec<(String, Vec<String>)>,
    pub require_coarse_review: bool,
    pub rail_carrying_value: bool,
    pub tax_year_start: String,
    pub tax_year_start_default: String,
    pub tax_year_editable: bool,
    pub presets: Vec<Preset>,
    pub timezone: String,
    pub timezone_default: String,
    pub timezones: Vec<String>,
    pub calendar_is_default: bool,
}

fn st(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_owned))
        .collect()
}

impl Settings {
    pub fn parse(v: &Value) -> Option<Settings> {
        let profile_id = v["profile_id"].as_str()?.to_owned();
        Some(Settings {
            profile_id,
            workspace_id: st(&v["workspace_id"]),
            label: st(&v["label"]),
            is_current: v["is_current"].as_bool().unwrap_or(false),
            tax_country: v["tax_country"].as_str().unwrap_or("generic").to_owned(),
            preview: v["tax_country_preview"].as_bool().unwrap_or(false),
            fiat_currency: st(&v["fiat_currency"]),
            fiat_currency_locked: v["fiat_currency_locked"].as_bool().unwrap_or(false),
            long_term_days: v["tax_long_term_days"].as_i64(),
            long_term_days_editable: v["tax_long_term_days_editable"].as_bool().unwrap_or(false),
            method: st(&v["gains_algorithm"]),
            methods: strings(&v["gains_algorithms"]),
            pool: v["cost_basis_pool_scope"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("global")
                .to_owned(),
            pools: strings(&v["cost_basis_pool_scopes"]),
            method_pools: v["method_pool_scopes"]
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), strings(v))).collect())
                .unwrap_or_default(),
            require_coarse_review: v["require_coarse_review"].as_bool().unwrap_or(false),
            rail_carrying_value: v["bitcoin_rail_carrying_value"].as_bool().unwrap_or(true),
            tax_year_start: v["tax_year_start"].as_str().unwrap_or("01-01").to_owned(),
            tax_year_start_default: v["tax_year_start_default"]
                .as_str()
                .unwrap_or("01-01")
                .to_owned(),
            tax_year_editable: v["tax_year_editable"].as_bool().unwrap_or(false),
            presets: v["tax_year_presets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    Some(Preset {
                        id: p["id"].as_str()?.to_owned(),
                        start: p["start"].as_str()?.to_owned(),
                        name: st(&p["name"]),
                    })
                })
                .collect(),
            timezone: v["timezone"].as_str().unwrap_or("UTC").to_owned(),
            timezone_default: v["timezone_default"].as_str().unwrap_or("UTC").to_owned(),
            timezones: strings(&v["timezones"]),
            calendar_is_default: v["calendar_is_default"].as_bool().unwrap_or(true),
        })
    }

    /// The scopes `method` has in this book's country (`method_pool_scopes`); a method the
    /// daemon didn't describe has none of them.
    pub fn scopes_of(&self, method: &str) -> Vec<String> {
        self.method_pools
            .iter()
            .find(|(m, _)| m == method)
            .map(|(_, s)| s.clone())
            .unwrap_or_default()
    }
}

// ---- Tax year -----------------------------------------------------------------------------------

/// Days in each month of a common year (2001): 29 February is not a day every year has.
const MONTH_DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// A tax year's first day as (month, day) from `MM-DD`, or None when it isn't a day every year
/// has (the daemon's `parse_start`: two-digit month and day, a dash, never `02-29`).
pub fn parse_start(text: &str) -> Option<(u32, u32)> {
    let t = text.trim();
    let b = t.as_bytes();
    if b.len() != 5 || b[2] != b'-' || !b[..2].iter().chain(&b[3..]).all(u8::is_ascii_digit) {
        return None;
    }
    let m: u32 = t[..2].parse().ok()?;
    let d: u32 = t[3..].parse().ok()?;
    ((1..=12).contains(&m) && d >= 1 && d <= MONTH_DAYS[m as usize - 1]).then_some((m, d))
}

/// What a person typed as a custom start, made `MM-DD`: `7-1`, `07/01` and `0701` are read as
/// 1 July. None when it can't be a day every year has.
pub fn normalize_start(text: &str) -> Option<String> {
    let t = text.trim();
    // Digits and separators only, so the byte slices below fall on character boundaries.
    if !t
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'/' | b'.'))
    {
        return None;
    }
    let parts: Vec<&str> = t.split(['-', '/', '.']).collect();
    let (m, d) = match parts.as_slice() {
        [m, d] if !m.is_empty() && !d.is_empty() && m.len() <= 2 && d.len() <= 2 => {
            (m.to_string(), d.to_string())
        }
        [md] if md.len() == 4 => (md[..2].to_string(), md[2..].to_string()),
        _ => return None,
    };
    let s = format!("{m:0>2}-{d:0>2}");
    parse_start(&s).map(|_| s)
}

/// The tax-year picker's choice: a preset by id, or a custom date.
#[derive(Clone, Debug, PartialEq)]
pub enum YearChoice {
    Preset(String),
    Custom,
}

/// The choice that shows a stored start: the preset with that start, else a custom date.
pub fn choice_for(start: &str, presets: &[Preset]) -> YearChoice {
    presets
        .iter()
        .find(|p| p.start == start)
        .map(|p| YearChoice::Preset(p.id.clone()))
        .unwrap_or(YearChoice::Custom)
}

/// The start a choice asks for: the preset's, or the custom field's text made `MM-DD`
/// (`Err` when that text isn't a valid date).
pub fn chosen_start(choice: &YearChoice, presets: &[Preset], custom: &str) -> Result<String, ()> {
    match choice {
        YearChoice::Preset(id) => presets
            .iter()
            .find(|p| &p.id == id)
            .map(|p| p.start.clone())
            .ok_or(()),
        YearChoice::Custom => normalize_start(custom).ok_or(()),
    }
}

/// The tax year of `start` that contains `today`: its first and last day and its name, the
/// calendar year of its first day (`2025`), labelled `2025-26` when it doesn't start on
/// 1 January (`kassiber_tax_countries::years`).
pub fn year_containing(start: (u32, u32), today: Day) -> (Day, Day, String) {
    let (m, d) = start;
    let y = if (today.m, today.d) >= (m, d) {
        today.y
    } else {
        today.y - 1
    };
    let first = Day { y, m, d };
    let last = Day::from_epoch_days(Day { y: y + 1, m, d }.epoch_days() - 1);
    (first, last, year_label(y, start))
}

/// `2025` for a calendar year, `2025-26` otherwise.
pub fn year_label(y: i32, start: (u32, u32)) -> String {
    if start == (1, 1) {
        y.to_string()
    } else {
        format!("{y}-{:02}", (y + 1).rem_euclid(100))
    }
}

/// Today in UTC.
pub fn today() -> Day {
    Day::from_epoch_days((crate::format::now_secs() / 86_400.0).floor() as i64)
}

// ---- Currency and holding period ------------------------------------------------------------------

/// A three-letter currency code, upper-cased; None for anything else.
pub fn normalize_currency(text: &str) -> Option<String> {
    let t = text.trim();
    (t.len() == 3 && t.chars().all(|c| c.is_ascii_alphabetic())).then(|| t.to_ascii_uppercase())
}

/// Whole days, 0 or more (at most 100 years).
pub fn parse_days(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    t.parse::<i64>().ok().filter(|d| (0..=36_500).contains(d))
}

// ---- Pool scopes ------------------------------------------------------------------------------------

/// Whether a pool scope can be chosen with the form's method.
#[derive(Clone, Debug, PartialEq)]
pub enum ScopeState {
    Available,
    /// The country offers the scope, but the chosen method has no form of it.
    MethodUnsupported,
    /// The country's rules don't offer the scope (shown, not choosable).
    RegionUnsupported,
}

/// Every pool scope the app knows, in display order.
pub const ALL_SCOPES: [&str; 3] = ["global", "wallet", "global_then_wallet"];

/// `scope_states`, then every other known scope as `RegionUnsupported`: the choice is always
/// shown, with the reason a scope can't be picked here.
pub fn all_scope_states(choices: &Settings, method: &str) -> Vec<(String, ScopeState)> {
    let mut out = scope_states(choices, method);
    for s in ALL_SCOPES {
        if !out.iter().any(|(x, _)| x == s) {
            out.push((s.to_owned(), ScopeState::RegionUnsupported));
        }
    }
    out
}

/// The country's default scope when its rules allow exactly one (the legal one); None where
/// the book may choose.
pub fn legal_scope(choices: &Settings) -> Option<&str> {
    match choices.pools.as_slice() {
        [only] => Some(only.as_str()),
        _ => None,
    }
}

/// Each scope the country offers (`cost_basis_pool_scopes`, in its order) and whether `method`
/// can use it (`method_pool_scopes`).
pub fn scope_states(choices: &Settings, method: &str) -> Vec<(String, ScopeState)> {
    let has = choices.scopes_of(method);
    choices
        .pools
        .iter()
        .map(|s| {
            (
                s.clone(),
                if has.contains(s) {
                    ScopeState::Available
                } else {
                    ScopeState::MethodUnsupported
                },
            )
        })
        .collect()
}

/// The scope to keep after the method changes: the current one if the method has it, else the
/// method's first.
pub fn scope_after_method(choices: &Settings, method: &str, current: &str) -> String {
    let has = choices.scopes_of(method);
    if has.iter().any(|s| s == current) {
        current.to_owned()
    } else {
        has.first().cloned().unwrap_or_else(|| current.to_owned())
    }
}

// ---- The change -----------------------------------------------------------------------------------

/// What the form holds, validated.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    pub country: String,
    pub currency: Option<String>,
    pub long_term_days: Option<i64>,
    pub method: String,
    pub pool: String,
    pub require_coarse_review: bool,
    pub rail_carrying_value: bool,
    /// `MM-DD`; None leaves the calendar to the daemon (a region switch).
    pub tax_year_start: Option<String>,
    pub timezone: Option<String>,
}

/// The `ui.profiles.update` arguments that change `base` into `v`, without `profile_id` and
/// `dry_run`; an empty map when nothing changes. A region switch always names the method and
/// pool scope with it (the region's defaults, or what was chosen), as the web's payload does,
/// and leaves the calendar to the daemon, which resets one the new region doesn't allow.
pub fn changes(base: &Settings, v: &Values) -> Map<String, Value> {
    let mut m = Map::new();
    let switched = v.country != base.tax_country;
    if switched {
        m.insert("tax_country".into(), json!(v.country));
        m.insert("gains_algorithm".into(), json!(v.method));
        m.insert("cost_basis_pool_scope".into(), json!(v.pool));
    } else {
        if v.method != base.method {
            m.insert("gains_algorithm".into(), json!(v.method));
        }
        if v.pool != base.pool {
            m.insert("cost_basis_pool_scope".into(), json!(v.pool));
        }
        if let Some(s) = &v.tax_year_start
            && *s != base.tax_year_start
        {
            m.insert("tax_year_start".into(), json!(s));
        }
        if let Some(z) = &v.timezone
            && *z != base.timezone
        {
            m.insert("timezone".into(), json!(z));
        }
        if let Some(d) = v.long_term_days
            && Some(d) != base.long_term_days
        {
            m.insert("tax_long_term_days".into(), json!(d));
        }
    }
    if let Some(c) = &v.currency
        && !c.eq_ignore_ascii_case(&base.fiat_currency)
    {
        m.insert("fiat_currency".into(), json!(c));
    }
    if v.require_coarse_review != base.require_coarse_review {
        m.insert(
            "require_coarse_review".into(),
            json!(v.require_coarse_review),
        );
    }
    if v.rail_carrying_value != base.rail_carrying_value {
        m.insert(
            "bitcoin_rail_carrying_value".into(),
            json!(v.rail_carrying_value),
        );
    }
    m
}

/// The `ui.profiles.update` request: the book, the changes, the dry-run flag, and, for the open
/// book, its `expected_scope` (a switch made elsewhere while the dialog is open then refuses the
/// change with `stale_context`). Another book's settings go without one: the daemon refuses a
/// `profile_id` other than the scope's.
pub fn update_args(
    profile_id: &str,
    changes: &Map<String, Value>,
    dry_run: bool,
    expected_scope: &Value,
) -> Value {
    let mut m = changes.clone();
    m.insert("profile_id".into(), json!(profile_id));
    if dry_run {
        m.insert("dry_run".into(), json!(true));
    }
    let named = |k: &str| {
        expected_scope[k]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
    };
    if named("workspace_id")
        && expected_scope["profile_id"].as_str() == Some(profile_id)
        && !profile_id.trim().is_empty()
    {
        m.insert("expected_scope".into(), expected_scope.clone());
    }
    Value::Object(m)
}

// ---- The dry run ------------------------------------------------------------------------------------

/// One changed field.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub field: String,
    pub from: Value,
    pub to: Value,
}

/// A dry run's answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub changes: Vec<Change>,
    pub reprocess: bool,
    pub processed: bool,
    pub transactions: u64,
    pub filed_reports: u64,
    pub readiness: Vec<String>,
    /// Transactions held for a price in the new currency, and the currencies they have now.
    pub held: u64,
    pub held_currencies: Vec<String>,
    pub calendar_reset: bool,
    pub after: Option<Settings>,
}

impl Preview {
    pub fn parse(v: &Value) -> Preview {
        let n = |v: &Value| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0)
        };
        Preview {
            changes: v["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    Some(Change {
                        field: c["field"].as_str()?.to_owned(),
                        from: c["from"].clone(),
                        to: c["to"].clone(),
                    })
                })
                .collect(),
            reprocess: v["journals"]["reprocess"].as_bool().unwrap_or(false),
            processed: v["journals"]["processed"].as_bool().unwrap_or(false),
            transactions: n(&v["journals"]["transactions"]),
            filed_reports: n(&v["filed_reports"]),
            readiness: strings(&v["readiness"]),
            held: n(&v["price_currency_holds"]["transactions"]),
            held_currencies: v["price_currency_holds"]["by_currency"]
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default(),
            calendar_reset: v["tax_calendar_reset"].as_bool().unwrap_or(false),
            after: Settings::parse(&v["settings"]),
        }
    }

    pub fn has(&self, flag: &str) -> bool {
        self.readiness.iter().any(|r| r == flag)
    }
}

/// The plain-language consequences a preview lists, in order, as (key, count, args) for the
/// view to translate: journals, regrouped years, holds, filed reports, a reset calendar.
#[derive(Clone, Debug, PartialEq)]
pub enum Consequence {
    /// Journals were processed and go stale until reprocessed: `transactions` rows.
    Reprocess(u64),
    /// Journals were never processed: they use the new settings when they first run.
    FirstRun,
    Regrouped,
    Held(u64, Vec<String>),
    Filed(u64),
    CalendarReset,
}

pub fn consequences(p: &Preview) -> Vec<Consequence> {
    let mut out = Vec::new();
    if p.reprocess {
        out.push(if p.processed {
            Consequence::Reprocess(p.transactions)
        } else {
            Consequence::FirstRun
        });
    }
    if p.has("tax_years_regrouped") {
        out.push(Consequence::Regrouped);
    }
    if p.held > 0 || p.has("price_currency_mismatch_holds") {
        out.push(Consequence::Held(p.held, p.held_currencies.clone()));
    }
    if p.filed_reports > 0 && (p.has("filed_reports_may_be_impacted") || p.reprocess) {
        out.push(Consequence::Filed(p.filed_reports));
    }
    if p.calendar_reset {
        out.push(Consequence::CalendarReset);
    }
    out
}

/// The `locale` key of a typed refusal (`method_scope_unsupported`, `tax_year_start_invalid`,
/// …); None for codes the dialog shows with the daemon's own message.
pub fn error_key(code: &str) -> Option<&'static str> {
    Some(match code {
        "method_scope_unsupported" => "onboarding:books.settings.errors.methodScopeUnsupported",
        "tax_year_start_invalid" => "onboarding:books.settings.errors.taxYearStartInvalid",
        "tax_year_start_locked" => "onboarding:books.settings.errors.taxYearStartLocked",
        "timezone_unknown" => "onboarding:books.settings.errors.timezoneUnknown",
        "timezone_locked" => "onboarding:books.settings.errors.timezoneLocked",
        "stale_context" => "onboarding:books.settings.errors.bookSwitched",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generic() -> Settings {
        Settings::parse(&json!({
            "profile_id": "p1", "workspace_id": "w1", "label": "Main", "is_current": true,
            "tax_country": "generic", "tax_country_preview": false,
            "fiat_currency": "EUR", "fiat_currency_locked": false,
            "tax_long_term_days": 365, "tax_long_term_days_editable": true,
            "gains_algorithm": "FIFO", "gains_algorithms": ["FIFO", "HIFO", "LIFO", "LOFO", "MOVING_AVERAGE"],
            "cost_basis_pool_scope": "global", "cost_basis_pool_scopes": ["global", "wallet"],
            "method_pool_scopes": {"FIFO": ["global", "wallet"], "MOVING_AVERAGE": ["global"], "HIFO": ["global", "wallet"]},
            "require_coarse_review": false, "bitcoin_rail_carrying_value": true,
            "tax_year_start": "01-01", "tax_year_start_default": "01-01", "tax_year_editable": true,
            "tax_year_presets": [
                {"id": "calendar", "start": "01-01", "name": "1 January (calendar year)"},
                {"id": "uk", "start": "04-06", "name": "6 April (UK-style)"},
                {"id": "april", "start": "04-01", "name": "1 April"},
                {"id": "july", "start": "07-01", "name": "1 July"},
                {"id": "march", "start": "03-01", "name": "1 March"}
            ],
            "timezone": "UTC", "timezone_default": "UTC", "timezones": ["UTC", "Europe/Vienna"],
            "calendar_is_default": true
        }))
        .unwrap()
    }

    fn values(s: &Settings) -> Values {
        Values {
            country: s.tax_country.clone(),
            currency: Some(s.fiat_currency.clone()),
            long_term_days: s.long_term_days,
            method: s.method.clone(),
            pool: s.pool.clone(),
            require_coarse_review: s.require_coarse_review,
            rail_carrying_value: s.rail_carrying_value,
            tax_year_start: Some(s.tax_year_start.clone()),
            timezone: Some(s.timezone.clone()),
        }
    }

    #[test]
    fn custom_start_dates_are_days_every_year_has() {
        assert_eq!(parse_start("07-01"), Some((7, 1)));
        assert_eq!(parse_start("12-31"), Some((12, 31)));
        assert_eq!(parse_start("02-28"), Some((2, 28)));
        for bad in [
            "02-29", "02-30", "04-31", "13-01", "00-10", "01-00", "7-1", "07/01", "0701", "",
            "ab-cd", "07-01x",
        ] {
            assert_eq!(parse_start(bad), None, "{bad}");
        }
    }

    #[test]
    fn custom_start_input_is_forgiving_but_still_valid() {
        assert_eq!(normalize_start(" 7-1 ").as_deref(), Some("07-01"));
        assert_eq!(normalize_start("07/01").as_deref(), Some("07-01"));
        assert_eq!(normalize_start("4.6").as_deref(), Some("04-06"));
        assert_eq!(normalize_start("0301").as_deref(), Some("03-01"));
        assert_eq!(normalize_start("2-29"), None);
        assert_eq!(normalize_start("31-12"), None);
        assert_eq!(normalize_start("7"), None);
        assert_eq!(normalize_start("123-1"), None);
        // A four-byte character is one "len 4" token; it must not be sliced.
        assert_eq!(normalize_start("📅"), None);
        assert_eq!(normalize_start("é1-1"), None);
    }

    #[test]
    fn presets_and_custom_dates_pick_the_start() {
        let s = generic();
        assert_eq!(
            choice_for("04-06", &s.presets),
            YearChoice::Preset("uk".into())
        );
        assert_eq!(choice_for("10-01", &s.presets), YearChoice::Custom);
        assert_eq!(
            chosen_start(&YearChoice::Preset("july".into()), &s.presets, ""),
            Ok("07-01".into())
        );
        assert_eq!(
            chosen_start(&YearChoice::Custom, &s.presets, "10-1"),
            Ok("10-01".into())
        );
        assert_eq!(
            chosen_start(&YearChoice::Custom, &s.presets, "02-29"),
            Err(())
        );
        assert_eq!(
            chosen_start(&YearChoice::Preset("gone".into()), &s.presets, ""),
            Err(())
        );
    }

    #[test]
    fn the_current_tax_year_is_named_by_its_first_day() {
        let (a, b, l) = year_containing(
            (7, 1),
            Day {
                y: 2026,
                m: 11,
                d: 16,
            },
        );
        assert_eq!(
            (a.iso(), b.iso(), l.as_str()),
            ("2026-07-01".into(), "2027-06-30".into(), "2026-27")
        );
        let (a, b, l) = year_containing(
            (7, 1),
            Day {
                y: 2026,
                m: 3,
                d: 1,
            },
        );
        assert_eq!(
            (a.iso(), b.iso(), l.as_str()),
            ("2025-07-01".into(), "2026-06-30".into(), "2025-26")
        );
        let (a, b, l) = year_containing(
            (1, 1),
            Day {
                y: 2026,
                m: 11,
                d: 16,
            },
        );
        assert_eq!(
            (a.iso(), b.iso(), l.as_str()),
            ("2026-01-01".into(), "2026-12-31".into(), "2026")
        );
        // A 1 March year ends on the last day of February, leap or not.
        let (_, b, _) = year_containing(
            (3, 1),
            Day {
                y: 2027,
                m: 5,
                d: 1,
            },
        );
        assert_eq!(b.iso(), "2028-02-29");
        assert_eq!(year_label(2099, (4, 6)), "2099-00");
    }

    #[test]
    fn currency_and_days_validate() {
        assert_eq!(normalize_currency(" usd ").as_deref(), Some("USD"));
        assert_eq!(normalize_currency("EURO"), None);
        assert_eq!(normalize_currency("€"), None);
        assert_eq!(parse_days("0"), Some(0));
        assert_eq!(parse_days(" 365 "), Some(365));
        assert_eq!(parse_days("-1"), None);
        assert_eq!(parse_days("1.5"), None);
        assert_eq!(parse_days(""), None);
        assert_eq!(parse_days("99999"), None);
    }

    #[test]
    fn scopes_follow_the_registry_and_the_method() {
        let s = generic();
        assert_eq!(
            scope_states(&s, "FIFO"),
            vec![
                ("global".into(), ScopeState::Available),
                ("wallet".into(), ScopeState::Available)
            ]
        );
        assert_eq!(
            scope_states(&s, "MOVING_AVERAGE"),
            vec![
                ("global".into(), ScopeState::Available),
                ("wallet".into(), ScopeState::MethodUnsupported)
            ]
        );
        // A method the daemon didn't describe can use none of them.
        assert!(
            scope_states(&s, "LOFO")
                .iter()
                .all(|(_, st)| *st == ScopeState::MethodUnsupported)
        );
        assert_eq!(scope_after_method(&s, "MOVING_AVERAGE", "wallet"), "global");
        assert_eq!(scope_after_method(&s, "HIFO", "wallet"), "wallet");
        // Only what the country offers is listed: a single-scope country shows one.
        let mut one = s.clone();
        one.pools = vec!["global".into()];
        assert_eq!(scope_states(&one, "FIFO").len(), 1);
        // The form still shows every scope, the ones the country doesn't offer disabled, and
        // names the single allowed one as the legal default.
        assert_eq!(
            all_scope_states(&one, "FIFO"),
            vec![
                ("global".into(), ScopeState::Available),
                ("wallet".into(), ScopeState::RegionUnsupported),
                ("global_then_wallet".into(), ScopeState::RegionUnsupported),
            ]
        );
        assert_eq!(legal_scope(&one), Some("global"));
        assert_eq!(legal_scope(&s), None);
    }

    #[test]
    fn changes_name_only_what_differs() {
        let s = generic();
        assert!(changes(&s, &values(&s)).is_empty());
        let mut v = values(&s);
        v.tax_year_start = Some("07-01".into());
        v.require_coarse_review = true;
        v.currency = Some("usd".into());
        let m = changes(&s, &v);
        assert_eq!(
            Value::Object(m),
            json!({"tax_year_start": "07-01", "require_coarse_review": true, "fiat_currency": "usd"})
        );
        // The same currency in another case is no change.
        let mut v = values(&s);
        v.currency = Some("eur".into());
        assert!(changes(&s, &v).is_empty());
    }

    #[test]
    fn a_region_switch_names_method_and_scope_and_leaves_the_calendar() {
        let s = generic();
        let mut v = values(&s);
        v.country = "at".into();
        v.method = "MOVING_AVERAGE_AT".into();
        v.pool = "global".into();
        v.tax_year_start = Some("07-01".into());
        v.long_term_days = Some(100);
        let m = changes(&s, &v);
        assert_eq!(
            Value::Object(m),
            json!({"tax_country": "at", "gains_algorithm": "MOVING_AVERAGE_AT", "cost_basis_pool_scope": "global"})
        );
    }

    #[test]
    fn update_args_carry_the_scope_only_for_the_open_book() {
        let mut m = Map::new();
        m.insert("tax_year_start".into(), json!("07-01"));
        let scope = json!({"workspace_id": "w1", "profile_id": "p1"});
        assert_eq!(
            update_args("p1", &m, true, &scope),
            json!({"tax_year_start": "07-01", "profile_id": "p1", "dry_run": true, "expected_scope": scope})
        );
        // Another book: the daemon would refuse a profile_id other than the scope's.
        assert_eq!(
            update_args("p2", &m, false, &scope),
            json!({"tax_year_start": "07-01", "profile_id": "p2"})
        );
        assert_eq!(
            update_args(
                "p1",
                &m,
                false,
                &json!({"workspace_id": null, "profile_id": "p1"})
            ),
            json!({"tax_year_start": "07-01", "profile_id": "p1"})
        );
    }

    #[test]
    fn a_dry_run_reads_as_plain_consequences() {
        let p = Preview::parse(&json!({
            "dry_run": true,
            "changes": [{"field": "tax_year_start", "from": "01-01", "to": "07-01"}],
            "journals": {"reprocess": true, "processed": true, "transactions": 1125},
            "filed_reports": 2,
            "readiness": ["journals_stale_until_reprocessed", "filed_reports_may_be_impacted", "tax_years_regrouped"],
            "settings": {"profile_id": "p1", "tax_year_start": "07-01"}
        }));
        assert_eq!(p.changes.len(), 1);
        assert_eq!(
            p.after.as_ref().map(|s| s.tax_year_start.as_str()),
            Some("07-01")
        );
        assert_eq!(
            consequences(&p),
            vec![
                Consequence::Reprocess(1125),
                Consequence::Regrouped,
                Consequence::Filed(2)
            ]
        );

        let p = Preview::parse(&json!({
            "changes": [{"field": "fiat_currency", "from": "EUR", "to": "USD"}],
            "journals": {"reprocess": true, "processed": false, "transactions": 4},
            "filed_reports": 0,
            "readiness": ["price_currency_mismatch_holds"],
            "price_currency_holds": {"transactions": 4, "by_currency": {"EUR": 4}},
            "tax_calendar_reset": true
        }));
        assert_eq!(
            consequences(&p),
            vec![
                Consequence::FirstRun,
                Consequence::Held(4, vec!["EUR".into()]),
                Consequence::CalendarReset
            ]
        );
        assert!(consequences(&Preview::default()).is_empty());
    }

    #[test]
    fn typed_refusals_map_to_plain_messages() {
        assert!(error_key("method_scope_unsupported").is_some());
        assert!(error_key("tax_year_start_invalid").is_some());
        assert!(error_key("timezone_locked").is_some());
        assert_eq!(error_key("validation"), None);
    }
}
