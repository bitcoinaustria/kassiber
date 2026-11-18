//! Book settings: every setting of one book, any book, in one dialog, and every change shown
//! as a dry run before anything is written (docs/reference/tax.md "Book settings";
//! `ui.profiles.settings`, `ui.profiles.update {dry_run}` in docs/reference/daemon.md).
//!
//! Sections: Book (name), Country and currency, Tax year (presets, a custom date, the time
//! zone; locked with the reason where the country has a legal year), Cost basis (method and
//! pool scope, each scope explained and disabled with the reason the method gives), Holding
//! period, Review options (daily-rate review, BTC/L-BTC carrying value).
//!
//! The flow, all local daemon calls:
//! 1. open → `ui.profiles.settings {profile_id}` (the form waits for it);
//! 2. a region switch → a dry run with the region and its defaults, whose `settings` are the
//!    new region's choices (methods, scopes, currency, calendar) until saved;
//! 3. "Review changes" → the dry run of exactly what changed; its `changes` and consequences
//!    (journals to reprocess, years regrouped, held rows, filed reports) are shown in words;
//! 4. "Apply changes" → the same request without `dry_run`, then `ui.profiles.rename` if the
//!    name changed too.
//!
//! The Tauri dialog (Books.tsx `RenameProfileDialog`) has name, region, method and scope only,
//! with a region-switch confirmation; this one replaces that confirmation with the preview.

use rune::{El, Key, KeyEvent, TextEdit, Tx, col, el, row, text};
use rune_kit::TextInput;
use serde_json::{Map, Value, json};

use super::kit::{
    self, dialog_header, error_box, labelled, note, preview_badge, section, select_menu,
    select_trigger,
};
use super::model::{self, LongTerm};
use super::settings_model::{self as sm, Consequence, Preview, ScopeState, Settings, YearChoice};
use super::{Dialog, error_text};
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, field, form_kit, spinner};
use crate::ui::ic;

/// The custom-date choice's option value in the tax-year select.
const CUSTOM: &str = "__custom__";

/// The dialog's form (`app.reports.books.form`).
#[derive(Default)]
pub struct Form {
    pub profile_id: String,
    pub ws: String,
    /// The saved settings (`ui.profiles.settings`); None while they load.
    pub base: Option<Settings>,
    pub load_error: Option<String>,
    /// A switched region's choices (the dry run's `settings`), and the region they are for.
    pub choices: Option<Settings>,
    pub choices_for: String,
    pub country: String,
    pub currency: TextEdit,
    pub days: TextEdit,
    pub method: String,
    pub pool: String,
    pub year: Option<YearChoice>,
    pub custom: TextEdit,
    pub zone: String,
    pub coarse: bool,
    pub rails: bool,
    /// Fields that failed validation: `name`, `currency`, `days`, `custom`.
    pub invalid: Vec<&'static str>,
    /// The dry run being reviewed, with the change it was made for.
    pub preview: Option<Preview>,
    pub pending: Map<String, Value>,
    /// The new name, when it changed.
    pub rename: Option<String>,
}

fn form(a: &mut App) -> &mut Form {
    &mut a.reports.books.form
}

fn te(s: &str) -> TextEdit {
    TextEdit::new(s)
}

/// Opens Book settings for `profile_id` in the set `ws` (`name` seeds the name field until the
/// settings arrive). The read is local.
pub fn open(app: &mut App, ws: &str, profile_id: &str, name: &str) {
    let b = &mut app.reports.books;
    b.popup = None;
    b.error = None;
    b.submitting = false;
    let mut n = TextEdit::new(name);
    n.focused = true;
    b.name = n;
    b.form = Form {
        profile_id: profile_id.to_owned(),
        ws: ws.to_owned(),
        ..Form::default()
    };
    b.dialog = Some(Dialog::Settings {
        ws: ws.to_owned(),
        profile_id: profile_id.to_owned(),
    });
    if let Some(s) = b.cache.get(profile_id).cloned() {
        seed(app, s);
    }
    app.send(
        "ui.profiles.settings",
        json!({"profile_id": profile_id}),
        "reports.books.form_settings",
    );
}

/// What the form shows for saved settings `s`, field by field.
#[derive(Clone, PartialEq)]
struct Seeded {
    country: String,
    currency: String,
    days: String,
    method: String,
    pool: String,
    custom: String,
    year: YearChoice,
    zone: String,
    coarse: bool,
    rails: bool,
}

fn seeded(s: &Settings) -> Seeded {
    let year = sm::choice_for(&s.tax_year_start, &s.presets);
    Seeded {
        country: s.tax_country.clone(),
        currency: s.fiat_currency.clone(),
        days: s.long_term_days.map(|d| d.to_string()).unwrap_or_default(),
        method: s.method.clone(),
        pool: s.pool.clone(),
        custom: if year == YearChoice::Custom {
            s.tax_year_start.clone()
        } else {
            String::new()
        },
        year,
        zone: s.timezone.clone(),
        coarse: s.require_coarse_review,
        rails: s.rail_carrying_value,
    }
}

/// Fills the form from the saved settings. A later read (e.g. after the CLI changed the book)
/// refreshes every field the user has not edited since the last read; edited fields stay, so a
/// re-read never overwrites edits and a review never proposes reverting an outside change.
fn seed(app: &mut App, s: Settings) {
    let f = form(app);
    let new = seeded(&s);
    let old = f.base.as_ref().map(seeded);
    // A field takes the new value when there was no earlier read or it still shows the old one.
    let first = old.is_none();
    let untouched = |shown: bool| first || shown;
    if untouched(old.as_ref().is_some_and(|o| f.country == o.country)) {
        f.country = new.country.clone();
    }
    if untouched(old.as_ref().is_some_and(|o| f.currency.text == o.currency)) {
        f.currency = te(&new.currency);
    }
    if untouched(old.as_ref().is_some_and(|o| f.days.text == o.days)) {
        f.days = te(&new.days);
    }
    if untouched(old.as_ref().is_some_and(|o| f.method == o.method)) {
        f.method = new.method.clone();
    }
    if untouched(old.as_ref().is_some_and(|o| f.pool == o.pool)) {
        f.pool = new.pool.clone();
    }
    if untouched(
        old.as_ref()
            .is_some_and(|o| f.year.as_ref() == Some(&o.year) && f.custom.text == o.custom),
    ) {
        f.year = Some(new.year.clone());
        f.custom = te(&new.custom);
    }
    if untouched(old.as_ref().is_some_and(|o| f.zone == o.zone)) {
        f.zone = new.zone.clone();
    }
    if untouched(old.as_ref().is_some_and(|o| f.coarse == o.coarse)) {
        f.coarse = new.coarse;
    }
    if untouched(old.as_ref().is_some_and(|o| f.rails == o.rails)) {
        f.rails = new.rails;
    }
    f.base = Some(s);
    f.load_error = None;
}

/// The choices the form offers now: the saved book's, or the switched region's (the dry run's,
/// else the registry copy's until it answers).
pub fn choices(f: &Form) -> Option<Settings> {
    let base = f.base.as_ref()?;
    if f.country == base.tax_country {
        return Some(base.clone());
    }
    if let Some(c) = f.choices.as_ref().filter(|c| c.tax_country == f.country) {
        return Some(c.clone());
    }
    Some(fallback_choices(base, &f.country))
}

/// A switched region's choices from the registry copy, until its dry run answers: every
/// method with every scope (the dry run on Review checks the pair).
fn fallback_choices(base: &Settings, code: &str) -> Settings {
    let Some(c) = model::country(code) else {
        return base.clone();
    };
    let pools: Vec<String> = c.pool_scopes.iter().map(|s| s.to_string()).collect();
    Settings {
        profile_id: base.profile_id.clone(),
        workspace_id: base.workspace_id.clone(),
        label: base.label.clone(),
        is_current: base.is_current,
        tax_country: c.code.to_owned(),
        preview: c.preview,
        fiat_currency: c
            .fiat
            .map(str::to_owned)
            .unwrap_or_else(|| base.fiat_currency.clone()),
        fiat_currency_locked: c.fiat.is_some(),
        long_term_days: if c.long_term == LongTerm::Days {
            base.long_term_days.or(Some(365))
        } else {
            None
        },
        long_term_days_editable: c.long_term == LongTerm::Days,
        method: c.default_method.to_owned(),
        methods: c.methods.iter().map(|s| s.to_string()).collect(),
        pool: c.default_pool_scope.to_owned(),
        method_pools: c
            .methods
            .iter()
            .map(|m| (m.to_string(), pools.clone()))
            .collect(),
        pools,
        require_coarse_review: base.require_coarse_review,
        rail_carrying_value: base.rail_carrying_value,
        tax_year_start: c.tax_year_start.to_owned(),
        tax_year_start_default: c.tax_year_start.to_owned(),
        tax_year_editable: c.tax_year_editable,
        presets: vec![],
        timezone: c.timezone.to_owned(),
        timezone_default: c.timezone.to_owned(),
        timezones: vec![c.timezone.to_owned()],
        calendar_is_default: true,
    }
}

/// The methods the picker lists: the saved method first while on the saved region (a legacy
/// one may be outside the list), then the region's in the app's picker order; Austria's moving
/// average hides the plain one unless the book uses it.
pub fn method_options(f: &Form, c: &Settings) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let saved = f
        .base
        .as_ref()
        .filter(|b| b.tax_country == f.country)
        .map(|b| b.method.clone());
    if let Some(m) = saved.clone().filter(|m| !m.is_empty()) {
        out.push(m);
    }
    let has_at = c.methods.iter().any(|m| m == "MOVING_AVERAGE_AT");
    let mut ordered: Vec<&String> = c.methods.iter().collect();
    ordered.sort_by_key(|m| model::picker_rank(m));
    for m in ordered {
        let hidden = m == "MOVING_AVERAGE" && has_at && saved.as_deref() != Some("MOVING_AVERAGE");
        if !hidden && !out.contains(m) {
            out.push(m.clone());
        }
    }
    out
}

pub fn set_country(app: &mut App, code: &str) {
    app.reports.books.error = None;
    let f = form(app);
    let Some(base) = f.base.clone() else { return };
    f.preview = None;
    f.country = code.to_owned();
    if code == base.tax_country {
        f.method = base.method.clone();
        f.pool = base.pool.clone();
        f.choices = None;
        f.choices_for.clear();
        return;
    }
    f.method = model::default_method(code);
    f.pool = model::default_pool_scope(code);
    f.choices = None;
    f.choices_for = code.to_owned();
    // The region's own choices (methods, scopes, currency, calendar) as the daemon sees them:
    // a dry run writes nothing.
    let mut m = Map::new();
    m.insert("tax_country".into(), json!(code));
    m.insert("gains_algorithm".into(), json!(f.method));
    m.insert("cost_basis_pool_scope".into(), json!(f.pool));
    let args = sm::update_args(&base.profile_id, &m, true, &Value::Null);
    app.send("ui.profiles.update", args, "reports.books.form_choices");
}

fn set_method(app: &mut App, m: &str) {
    app.reports.books.error = None;
    let f = &app.reports.books.form;
    let Some(c) = choices(f) else { return };
    let pool = sm::scope_after_method(&c, m, &f.pool);
    let f = form(app);
    f.method = m.to_owned();
    f.pool = pool;
}

fn set_pool(app: &mut App, s: &str) {
    let f = &app.reports.books.form;
    let Some(c) = choices(f) else { return };
    if sm::scope_states(&c, &f.method)
        .iter()
        .any(|(x, st)| x == s && *st == ScopeState::Available)
    {
        app.reports.books.error = None;
        form(app).pool = s.to_owned();
    }
}

/// Picks a tax-year preset by id (or `__custom__`), as the select does.
pub fn choose_year(app: &mut App, v: &str) {
    set_year(app, v);
}

fn set_year(app: &mut App, v: &str) {
    app.reports.books.error = None;
    let f = form(app);
    f.invalid.retain(|x| *x != "custom");
    if v == CUSTOM {
        if f.year != Some(YearChoice::Custom) {
            let start = f
                .base
                .as_ref()
                .map(|b| b.tax_year_start.clone())
                .unwrap_or_default();
            f.custom = te(&start);
            f.custom.focused = true;
        }
        f.year = Some(YearChoice::Custom);
    } else {
        f.year = Some(YearChoice::Preset(v.to_owned()));
    }
}

fn set_zone(app: &mut App, z: &str) {
    app.reports.books.error = None;
    form(app).zone = z.to_owned();
}

/// "Review changes": validates the form, then asks the daemon what the change would do. Only a
/// new name (no setting) is renamed at once: it changes nothing the journals read.
pub fn review(app: &mut App) {
    if app.reports.books.submitting {
        return;
    }
    let label = app.reports.books.name.text.trim().to_owned();
    let f = &app.reports.books.form;
    let Some(base) = f.base.clone() else { return };
    let Some(c) = choices(f) else { return };
    let switched = f.country != base.tax_country;
    let mut invalid: Vec<&'static str> = Vec::new();
    if label.is_empty() {
        invalid.push("name");
    }
    let currency = if c.fiat_currency_locked {
        None
    } else {
        let v = sm::normalize_currency(&f.currency.text);
        if v.is_none() {
            invalid.push("currency");
        }
        v
    };
    let days = if c.long_term_days_editable && !switched {
        let v = sm::parse_days(&f.days.text);
        if v.is_none() {
            invalid.push("days");
        }
        v
    } else {
        None
    };
    let start = if c.tax_year_editable && !switched {
        match f
            .year
            .as_ref()
            .map(|y| sm::chosen_start(y, &c.presets, &f.custom.text))
        {
            Some(Ok(s)) => Some(s),
            Some(Err(())) => {
                invalid.push("custom");
                None
            }
            None => None,
        }
    } else {
        None
    };
    let zone = (!switched && c.timezones.len() > 1).then(|| f.zone.clone());
    let values = sm::Values {
        country: f.country.clone(),
        currency,
        long_term_days: days,
        method: f.method.clone(),
        pool: f.pool.clone(),
        require_coarse_review: f.coarse,
        rail_carrying_value: f.rails,
        tax_year_start: start,
        timezone: zone,
    };
    let rename = (label != base.label && !label.is_empty()).then(|| label.clone());
    let b = &mut app.reports.books;
    b.form.invalid = invalid.clone();
    if !invalid.is_empty() {
        b.error = Some(t(match invalid[0] {
            "name" => "onboarding:books.renameProfile.errorEmptyName",
            "currency" => "onboarding:books.settings.currency.invalid",
            "days" => "onboarding:books.settings.holding.invalid",
            _ => "onboarding:books.settings.taxYear.customInvalid",
        }));
        return;
    }
    b.error = None;
    let changes = sm::changes(&base, &values);
    b.form.rename = rename.clone();
    if changes.is_empty() {
        match rename {
            Some(label) => send(
                app,
                "ui.profiles.rename",
                json!({"profile_id": base.profile_id, "label": label}),
                "reports.books.rename_profile",
            ),
            None => super::dialogs::close(app),
        }
        return;
    }
    app.reports.books.form.pending = changes.clone();
    let args = sm::update_args(&base.profile_id, &changes, true, &app.expected_scope());
    send(
        app,
        "ui.profiles.update",
        args,
        "reports.books.form_preview",
    );
}

/// "Apply changes": the reviewed change, for real.
pub fn apply(app: &mut App) {
    if app.reports.books.submitting {
        return;
    }
    let f = &app.reports.books.form;
    let Some(base) = f.base.as_ref() else { return };
    let id = base.profile_id.clone();
    let args = sm::update_args(&id, &f.pending, false, &app.expected_scope());
    app.reports.books.then_rename = f.rename.clone().map(|l| (id, l));
    send(app, "ui.profiles.update", args, "reports.books.update");
}

/// Back from the preview to the form.
pub fn back(app: &mut App) {
    if app.reports.books.submitting {
        return;
    }
    app.reports.books.error = None;
    form(app).preview = None;
}

fn send(app: &mut App, kind: &str, args: Value, tag: &'static str) {
    app.reports.books.error = None;
    if app.send(kind, args, tag).is_some() {
        app.reports.books.submitting = true;
    }
}

/// A refusal in plain words: the typed codes get their own copy, anything else the daemon's
/// message (else the generic one).
fn refusal(r: &Reply) -> String {
    match r.error_detail["code"].as_str().and_then(sm::error_key) {
        Some(k) => t(k),
        None => error_text(r, "onboarding:books.renameProfile.errorGeneric"),
    }
}

/// This dialog's replies; true when the tag was one of them.
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) -> bool {
    match tag {
        "reports.books.form_settings" => {
            let mine =
                r.data["profile_id"].as_str() == Some(app.reports.books.form.profile_id.as_str());
            match (&r.error, Settings::parse(&r.data)) {
                (None, Some(s)) => {
                    app.reports
                        .books
                        .cache
                        .insert(s.profile_id.clone(), s.clone());
                    if mine {
                        seed(app, s);
                    }
                }
                _ => {
                    if app.reports.books.form.base.is_none() {
                        form(app).load_error =
                            Some(error_text(r, "onboarding:books.settings.loadError"));
                    }
                }
            }
            true
        }
        "reports.books.form_choices" => {
            let p = Preview::parse(&r.data);
            if r.error.is_none()
                && let Some(after) = p.after
            {
                let f = form(app);
                if after.tax_country == f.choices_for && after.tax_country == f.country {
                    f.choices = Some(after);
                }
            }
            true
        }
        "reports.books.form_preview" => {
            app.reports.books.submitting = false;
            if r.error.is_some() {
                app.reports.books.error = Some(refusal(r));
                return true;
            }
            form(app).preview = Some(Preview::parse(&r.data));
            true
        }
        _ => false,
    }
}

/// A refused apply keeps the preview's step but shows why.
pub fn apply_failed(app: &mut App, r: &Reply) {
    app.reports.books.error = Some(refusal(r));
}

/// After an applied change: the book's settings cache is re-read, and a book whose journals
/// now wait for reprocessing is remembered until it is opened.
pub fn applied(app: &mut App) {
    let f = &app.reports.books.form;
    let id = f.profile_id.clone();
    if f.preview
        .as_ref()
        .is_some_and(|p| p.reprocess && p.processed)
    {
        app.reports.books.stale.insert(id.clone());
    }
    if !id.is_empty() {
        app.send(
            "ui.profiles.settings",
            json!({"profile_id": id}),
            "reports.books.settings_read",
        );
    }
}

// ---- View -----------------------------------------------------------------------------------

fn bad(app: &App, which: &str) -> bool {
    app.reports.books.form.invalid.contains(&which)
}

#[allow(clippy::too_many_arguments)]
fn input(
    app: &App,
    tk: &Tk,
    key: &str,
    label: &str,
    edit: &TextEdit,
    placeholder: Option<&str>,
    invalid: bool,
    width: Option<f32>,
    on_edit: fn(&mut App) -> &mut TextEdit,
    which: &'static str,
) -> El {
    let busy = app.reports.books.submitting;
    let mut i = TextInput::new(edit)
        .label(label.to_owned())
        .height(tk.r(2.25))
        .text_style(tk.tsm(400.0))
        .invalid(invalid)
        .disabled(busy);
    if let Some(p) = placeholder {
        i = i.placeholder(p.to_owned());
    }
    let built = i
        .on_edit(on_edit)
        .on_change(move |a: &mut App, _: &mut Tx| {
            a.reports.books.form.invalid.retain(|x| *x != which);
            a.reports.books.error = None;
        })
        .on_submit(|a: &mut App, _: &mut Tx| review(a))
        .build(&form_kit(tk))
        .key(key.to_owned());
    let built = match width {
        Some(w) => built.w(w),
        None => built.w_full(),
    };
    field(tk, built)
}

fn sel_open(app: &App, which: &str) -> bool {
    app.reports
        .books
        .popup
        .as_deref()
        .and_then(|p| p.strip_prefix("sel:"))
        == Some(which)
}

fn toggle_sel(a: &mut App, which: &str) {
    let k = format!("sel:{which}");
    a.reports.books.popup = if a.reports.books.popup.as_deref() == Some(k.as_str()) {
        None
    } else {
        Some(k)
    };
}

/// A full-width select field: label, trigger, its listbox when open.
#[allow(clippy::too_many_arguments)]
fn select(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    which: &'static str,
    label: &str,
    value: &str,
    badge: bool,
    options: Vec<(String, String, bool)>,
    current: &str,
    enabled: bool,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) -> El {
    let key = format!("books-set-{which}");
    let trig = select_trigger(
        tk,
        &key,
        label,
        value,
        badge.then(|| preview_badge(tk)),
        enabled,
        true,
    );
    let open = sel_open(app, which) && enabled;
    let trig = crate::ui::popup::trigger(
        trig,
        crate::ui::popup::Kind::Listbox,
        open,
        move |a: &mut App| {
            if !a.reports.books.submitting {
                toggle_sel(a, which)
            }
        },
    );
    if open {
        select_menu(tk, layer, &key, tk.r(20.0), options, current, choose);
    }
    labelled(tk, label, trig)
}

/// A read-only fact: `text-sm` in a muted box (a locked setting).
fn fact(tk: &Tk, key: &str, label: &str, value: &str) -> El {
    labelled(
        tk,
        label,
        row()
            .key(key.to_owned())
            .w_full()
            .min_h(tk.r(2.25))
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .fill(tk.muted.alpha(0.25))
            .role("text")
            .label(format!("{label}: {value}"))
            .child(
                text(value.to_owned())
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .wrap()
                    .w_full()
                    .a11y_hidden(),
            ),
    )
}

/// The preset's name in the UI language; the registry's English name for a preset this copy
/// doesn't know yet.
pub fn preset_label(p: &sm::Preset) -> String {
    let key = format!("onboarding:books.settings.taxYear.presets.{}", p.id);
    match p.id.as_str() {
        "calendar" | "uk" | "april" | "july" | "march" => t(&key),
        _ => {
            if p.name.is_empty() {
                model::start_label(&p.start)
            } else {
                p.name.clone()
            }
        }
    }
}

/// "Current tax year 2026-27: 1 July 2026 – 30 June 2027" for an `MM-DD` start.
fn year_example(start: &str) -> Option<String> {
    let s = sm::parse_start(start)?;
    let (a, b, label) = sm::year_containing(s, sm::today());
    Some(tr(
        "onboarding:books.settings.taxYear.example",
        &[
            ("label", &label),
            ("from", &model::long_date(a)),
            ("to", &model::long_date(b)),
        ],
    ))
}

/// The day before `start`: a tax year's last day, in words.
fn year_end(start: &str) -> String {
    match sm::parse_start(start) {
        Some(s) => {
            let (_, last, _) = sm::year_containing(
                s,
                crate::format::Day {
                    y: 2001,
                    m: s.0,
                    d: s.1,
                },
            );
            model::day_of_year(last.m, last.d)
        }
        None => String::new(),
    }
}

fn country_section(app: &App, tk: &Tk, layer: &mut Vec<El>, f: &Form, c: &Settings) -> El {
    let busy = app.reports.books.submitting;
    let regions = model::COUNTRIES
        .iter()
        .map(|x| (x.code.to_owned(), model::region_label(x.code), x.preview))
        .collect();
    let mut fields = vec![select(
        app,
        tk,
        layer,
        "region",
        &t("onboarding:books.renameProfile.regionLabel"),
        &model::region_label(&f.country),
        model::is_preview(&f.country),
        regions,
        &f.country,
        !busy,
        set_country,
    )];
    if model::is_preview(&f.country) {
        let body = format!(
            "{}: {}",
            t("common:taxCountry.previewBadge"),
            t("common:taxCountry.previewNotice")
        );
        fields.push(
            col()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .role("note")
                .label(body.clone())
                .child(
                    text(body)
                        .font(tk.txs(400.0).lh(tk.r(1.25)))
                        .color(tk.fg)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    let cur_label = t("onboarding:essentials.fiatCurrency");
    if c.fiat_currency_locked {
        fields.push(fact(
            tk,
            "books-set-currency-fixed",
            &cur_label,
            &tr(
                "onboarding:books.settings.currency.locked",
                &[
                    ("country", &model::region_label(&f.country)),
                    ("currency", &c.fiat_currency),
                ],
            ),
        ));
    } else {
        fields.push(labelled(
            tk,
            &cur_label,
            input(
                app,
                tk,
                "books-set-currency",
                &cur_label,
                &f.currency,
                Some("EUR"),
                bad(app, "currency"),
                Some(tk.r(7.0)),
                |a| &mut a.reports.books.form.currency,
                "currency",
            ),
        ));
        fields.push(note(tk, &t("onboarding:books.settings.currency.note")));
    }
    section(
        tk,
        "books-set-sec-country",
        &t("onboarding:books.settings.sections.country"),
        None,
        false,
        fields,
    )
}

fn tax_year_section(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    f: &Form,
    c: &Settings,
    switched: bool,
) -> El {
    let busy = app.reports.books.submitting;
    let region = model::region_label(&f.country);
    let mut fields: Vec<El> = Vec::new();
    let start_label = t("onboarding:books.settings.taxYear.startLabel");
    let zone_label = t("onboarding:books.settings.taxYear.zoneLabel");
    if c.tax_year_editable && !switched {
        let mut opts: Vec<(String, String, bool)> = c
            .presets
            .iter()
            .map(|p| (p.id.clone(), preset_label(p), false))
            .collect();
        opts.push((
            CUSTOM.into(),
            t("onboarding:books.settings.taxYear.custom"),
            false,
        ));
        let (cur, shown) = match &f.year {
            Some(YearChoice::Preset(id)) => (
                id.clone(),
                c.presets
                    .iter()
                    .find(|p| &p.id == id)
                    .map(preset_label)
                    .unwrap_or_default(),
            ),
            _ => (CUSTOM.into(), t("onboarding:books.settings.taxYear.custom")),
        };
        fields.push(select(
            app,
            tk,
            layer,
            "year",
            &start_label,
            &shown,
            false,
            opts,
            &cur,
            !busy,
            set_year,
        ));
        let start = match &f.year {
            Some(YearChoice::Custom) => {
                let lbl = t("onboarding:books.settings.taxYear.customLabel");
                fields.push(labelled(
                    tk,
                    &lbl,
                    input(
                        app,
                        tk,
                        "books-set-custom",
                        &lbl,
                        &f.custom,
                        Some(&t("onboarding:books.settings.taxYear.customPlaceholder")),
                        bad(app, "custom"),
                        Some(tk.r(7.0)),
                        |a| &mut a.reports.books.form.custom,
                        "custom",
                    ),
                ));
                let hint = if bad(app, "custom") {
                    t("onboarding:books.settings.taxYear.customInvalid")
                } else {
                    t("onboarding:books.settings.taxYear.customHint")
                };
                fields.push(note(tk, &hint));
                sm::normalize_start(&f.custom.text)
            }
            Some(y) => sm::chosen_start(y, &c.presets, "").ok(),
            None => None,
        };
        if c.timezones.len() > 1 {
            let zones = c
                .timezones
                .iter()
                .map(|z| (z.clone(), z.clone(), false))
                .collect();
            fields.push(select(
                app,
                tk,
                layer,
                "zone",
                &zone_label,
                &f.zone,
                false,
                zones,
                &f.zone,
                !busy,
                set_zone,
            ));
            fields.push(note(tk, &t("onboarding:books.settings.taxYear.zoneNote")));
        }
        if let Some(ex) = start.as_deref().and_then(year_example) {
            fields.push(
                text(ex)
                    .key("books-set-year-example")
                    .font(tk.txs(500.0))
                    .color(tk.fg)
                    .wrap()
                    .w_full()
                    .role("status"),
            );
        }
    } else {
        let start = &c.tax_year_start;
        let span = tr(
            "onboarding:books.settings.taxYear.span",
            &[
                ("from", &model::start_label(start)),
                ("to", &year_end(start)),
            ],
        );
        fields.push(fact(
            tk,
            "books-set-year-fixed",
            &start_label,
            &format!("{span} · {}", c.timezone),
        ));
        let why = if switched {
            t("onboarding:books.settings.afterSwitch")
        } else {
            tr(
                "onboarding:books.settings.taxYear.locked",
                &[("country", &region)],
            )
        };
        fields.push(note(tk, &why));
    }
    section(
        tk,
        "books-set-sec-year",
        &t("onboarding:books.settings.sections.taxYear"),
        None,
        false,
        fields,
    )
}

/// One pool-scope choice: the radio, its name, its one-line explanation, and why it can't be
/// chosen with this method.
#[allow(clippy::too_many_arguments)]
fn scope_option(
    app: &App,
    tk: &Tk,
    scope: &str,
    state: &ScopeState,
    selected: bool,
    method: &str,
    country: &str,
    legal: bool,
) -> El {
    let busy = app.reports.books.submitting;
    let enabled = *state == ScopeState::Available && !busy;
    let label = model::pool_scope_label(scope);
    let mut texts = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.125))
        .a11y_hidden()
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .child(
                    text(label.clone())
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .wrap()
                        .shrink(),
                )
                .children(legal.then(|| {
                    crate::ui::tag(
                        tk,
                        tk.muted_fg,
                        None,
                        t("onboarding:books.settings.pool.default"),
                    )
                })),
        );
    let mut desc = Vec::new();
    if let Some(h) = model::pool_scope_help(scope) {
        texts = texts.child(
            text(h.clone())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
        desc.push(h);
    }
    if *state == ScopeState::MethodUnsupported {
        let why = tr(
            "onboarding:books.settings.pool.unsupported",
            &[("method", &model::method_label(method))],
        );
        texts = texts.child(
            text(why.clone())
                .font(tk.txs(400.0))
                .color(kit::amber(tk))
                .wrap()
                .w_full(),
        );
        desc.push(why);
    }
    if *state == ScopeState::RegionUnsupported {
        let why = tr(
            "onboarding:books.settings.pool.regionUnsupported",
            &[("country", &model::region_label(country))],
        );
        texts = texts.child(
            text(why.clone())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
        desc.push(why);
    }
    let dot = {
        let ring = row()
            .size(tk.r(1.0), tk.r(1.0))
            .center()
            .radius(tk.r(0.5))
            .border(1.0, if selected { tk.primary } else { tk.input })
            .a11y_hidden();
        if selected {
            ring.child(
                el().size(tk.r(0.5), tk.r(0.5))
                    .radius(tk.r(0.25))
                    .fill(tk.primary),
            )
        } else {
            ring
        }
    };
    let s = scope.to_owned();
    let mut r = row()
        .key(format!("books-set-pool-{scope}"))
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.625))
        .gap(tk.r(0.75))
        .items_start()
        .radius(tk.r(0.375))
        .border(
            1.0,
            if selected {
                tk.primary.alpha(0.6)
            } else {
                tk.border
            },
        )
        .role("radio")
        .checked(selected)
        .label(label)
        .description(desc.join(" "))
        .focusable(enabled || selected)
        .focus_selected(selected)
        .disabled(!enabled && !selected)
        .child(row().pt(tk.r(0.125)).child(dot))
        .child(texts);
    if enabled {
        r = r
            .hover(tk.muted.alpha(0.35))
            .on_press(move |a: &mut App| set_pool(a, &s));
    } else if !selected {
        r = r.opacity(0.6);
    }
    r
}

fn cost_basis_section(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    f: &Form,
    c: &Settings,
    first: bool,
) -> El {
    let busy = app.reports.books.submitting;
    let methods = method_options(f, c);
    let opts = methods
        .iter()
        .map(|m| (m.clone(), model::method_label(m), false))
        .collect();
    let mut fields = vec![select(
        app,
        tk,
        layer,
        "method",
        &t("onboarding:books.renameProfile.methodLabel"),
        &model::method_label(&f.method),
        false,
        opts,
        &f.method,
        !busy && methods.len() > 1,
        set_method,
    )];
    if f.country == "at" {
        fields.push(note(
            tk,
            &t("onboarding:books.renameProfile.austrianMethodNote"),
        ));
    }
    // Every pool is shown; the ones the country's rules don't offer say so, and a country with
    // one legal pool marks it as the default.
    let states = sm::all_scope_states(c, &f.method);
    let legal = sm::legal_scope(c).map(str::to_owned);
    let pool_label = t("onboarding:books.settings.pool.label");
    let mut group = col()
        .key("books-set-pool")
        .w_full()
        .gap(tk.r(0.5))
        .role("radiogroup")
        .label(pool_label.clone())
        .focus_group();
    for (s, st) in &states {
        group = group.child(scope_option(
            app,
            tk,
            s,
            st,
            *s == f.pool,
            &f.method,
            &f.country,
            legal.as_deref() == Some(s.as_str()),
        ));
    }
    fields.push(labelled(tk, &pool_label, group));
    fields.push(note(tk, &t("onboarding:books.settings.pool.perCoin")));
    section(
        tk,
        "books-set-sec-basis",
        &t("onboarding:books.settings.sections.costBasis"),
        None,
        first,
        fields,
    )
}

fn holding_section(app: &App, tk: &Tk, f: &Form, c: &Settings, switched: bool) -> El {
    let label = t("onboarding:essentials.longTermDays");
    let region = model::region_label(&f.country);
    let mut fields = Vec::new();
    if c.long_term_days_editable && !switched {
        fields.push(labelled(
            tk,
            &label,
            input(
                app,
                tk,
                "books-set-days",
                &label,
                &f.days,
                Some("365"),
                bad(app, "days"),
                Some(tk.r(7.0)),
                |a| &mut a.reports.books.form.days,
                "days",
            ),
        ));
        fields.push(note(tk, &t("onboarding:books.settings.holding.note")));
    } else {
        let rule = match model::country(&f.country).map(|x| x.long_term) {
            Some(LongTerm::MoreThanOneYear) => tr(
                "onboarding:books.settings.holding.oneYear",
                &[("country", &region)],
            ),
            Some(LongTerm::Days) => tc(
                "onboarding:books.settings.holding.days",
                c.long_term_days.unwrap_or(365).max(0) as usize,
                &[],
            ),
            _ => tr(
                "onboarding:books.settings.holding.never",
                &[("country", &region)],
            ),
        };
        fields.push(
            text(rule)
                .key("books-set-holding-rule")
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap()
                .w_full(),
        );
        if switched && c.long_term_days_editable {
            fields.push(note(tk, &t("onboarding:books.settings.afterSwitch")));
        }
    }
    section(
        tk,
        "books-set-sec-holding",
        &t("onboarding:books.settings.sections.holding"),
        None,
        false,
        fields,
    )
}

/// A switch row: label and description, the switch at the end.
#[allow(clippy::too_many_arguments)]
fn switch_row(
    app: &App,
    tk: &Tk,
    key: &str,
    label: &str,
    desc: &str,
    on: bool,
    flip: fn(&mut App),
) -> El {
    let busy = app.reports.books.submitting;
    let sw = crate::ui::switch(tk, &format!("{key}-switch"), on, label)
        .description(desc.to_owned())
        .disabled(busy)
        .opacity(if busy { 0.5 } else { 1.0 });
    let sw = if busy {
        sw
    } else {
        sw.on_press(move |a: &mut App| flip(a))
    };
    row()
        .key(key.to_owned())
        .w_full()
        .gap(tk.r(1.0))
        .items_start()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.25))
                .a11y_hidden()
                .child(
                    text(label.to_owned())
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .wrap()
                        .w_full(),
                )
                .child(
                    text(desc.to_owned())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        )
        .child(row().pt(tk.r(0.125)).child(sw))
}

fn review_section(app: &App, tk: &Tk, f: &Form) -> El {
    let fields = vec![
        switch_row(
            app,
            tk,
            "books-set-coarse",
            &t("settings:marketData.coarse.reviewToggleLabel"),
            &t("settings:marketData.coarse.reviewToggleDescription"),
            f.coarse,
            |a| {
                a.reports.books.error = None;
                a.reports.books.form.coarse = !a.reports.books.form.coarse;
            },
        ),
        switch_row(
            app,
            tk,
            "books-set-rails",
            &t("settings:marketData.bitcoinRails.toggleLabel"),
            &t("settings:marketData.bitcoinRails.toggleDescription"),
            f.rails,
            |a| {
                a.reports.books.error = None;
                a.reports.books.form.rails = !a.reports.books.form.rails;
            },
        ),
    ];
    section(
        tk,
        "books-set-sec-review",
        &t("onboarding:books.settings.sections.review"),
        None,
        false,
        fields,
    )
}

#[allow(clippy::too_many_arguments)]
fn footer(
    app: &App,
    tk: &Tk,
    secondary: &str,
    on_secondary: fn(&mut App),
    primary: &str,
    on_primary: fn(&mut App),
    primary_enabled: bool,
) -> El {
    let busy = app.reports.books.submitting;
    let mut submit = button(
        tk,
        "books-settings-submit",
        Variant::Default,
        Size::Default,
        None,
        primary,
        !busy && primary_enabled,
    )
    .on_press(move |a: &mut App| on_primary(a));
    if busy {
        submit = submit.child(spinner(tk, tk.r(0.875)));
    }
    row()
        .w_full()
        .pt(tk.r(0.25))
        .gap(tk.r(0.5))
        .justify_end()
        .child(
            button(
                tk,
                "books-settings-cancel",
                Variant::Outline,
                Size::Default,
                None,
                secondary,
                !busy,
            )
            .on_press(move |a: &mut App| {
                if !a.reports.books.submitting {
                    on_secondary(a);
                }
            }),
        )
        .child(submit)
}

/// A changed value in words.
pub fn value_text(field: &str, v: &Value) -> String {
    match v {
        Value::Null => "—".into(),
        Value::Bool(b) => t(if *b {
            "onboarding:books.settings.on"
        } else {
            "onboarding:books.settings.off"
        }),
        _ => {
            let s = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            match field {
                "tax_country" => model::region_label(&s),
                "gains_algorithm" => model::method_label(&s),
                "cost_basis_pool_scope" => model::pool_scope_label(&s),
                "tax_year_start" => model::start_label(&s),
                "tax_long_term_days" => s
                    .parse::<usize>()
                    .map(|n| tc("onboarding:books.settings.holding.days", n, &[]))
                    .unwrap_or(s),
                _ => s,
            }
        }
    }
}

/// A changed field's name.
pub fn field_name(field: &str) -> String {
    match field {
        "label"
        | "tax_country"
        | "fiat_currency"
        | "tax_long_term_days"
        | "gains_algorithm"
        | "cost_basis_pool_scope"
        | "require_coarse_review"
        | "bitcoin_rail_carrying_value"
        | "tax_year_start"
        | "timezone" => t(&format!("onboarding:books.settings.fields.{field}")),
        other => other.to_owned(),
    }
}

/// A consequence in words, and whether it needs attention (amber) rather than information.
pub fn consequence_text(
    c: &Consequence,
    base: &Settings,
    after: Option<&Settings>,
) -> (String, bool) {
    match c {
        Consequence::Reprocess(n) => (
            tc(
                "onboarding:books.settings.preview.reprocess",
                *n as usize,
                &[],
            ),
            false,
        ),
        Consequence::FirstRun => (t("onboarding:books.settings.preview.firstRun"), false),
        Consequence::Regrouped => {
            let start = after
                .map(|a| a.tax_year_start.clone())
                .unwrap_or_else(|| base.tax_year_start.clone());
            (
                tr(
                    "onboarding:books.settings.preview.regrouped",
                    &[("start", &model::start_label(&start))],
                ),
                false,
            )
        }
        Consequence::Held(n, from) => {
            let to = after.map(|a| a.fiat_currency.clone()).unwrap_or_default();
            (
                tc(
                    "onboarding:books.settings.preview.held",
                    *n as usize,
                    &[("from", &from.join(", ")), ("to", &to)],
                ),
                true,
            )
        }
        Consequence::Filed(n) => (
            tc("onboarding:books.settings.preview.filed", *n as usize, &[]),
            true,
        ),
        Consequence::CalendarReset => {
            let country = after
                .map(|a| model::region_label(&a.tax_country))
                .unwrap_or_default();
            (
                tr(
                    "onboarding:books.settings.preview.calendarReset",
                    &[("country", &country)],
                ),
                false,
            )
        }
    }
}

fn preview_body(tk: &Tk, f: &Form, base: &Settings, p: &Preview) -> Vec<El> {
    let mut out = Vec::new();
    let mut rows: Vec<(String, String, String)> = Vec::new();
    if let Some(n) = &f.rename {
        rows.push(("label".into(), base.label.clone(), n.clone()));
    }
    for c in &p.changes {
        rows.push((
            c.field.clone(),
            value_text(&c.field, &c.from),
            value_text(&c.field, &c.to),
        ));
    }
    let heading = t("onboarding:books.settings.preview.changesHeading");
    let mut list = col()
        .key("books-set-changes")
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .clip_rounded(tk.r(0.5))
        .role("list")
        .label(heading.clone());
    for (i, (field, from, to)) in rows.iter().enumerate() {
        let name = field_name(field);
        let r = row()
            .key(format!("books-set-change-{field}"))
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .gap(tk.r(0.75))
            .center_y()
            .role("listitem")
            .label(tr(
                "onboarding:books.settings.preview.changeA11y",
                &[("field", &name), ("from", from), ("to", to)],
            ))
            .child(
                text(name)
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg)
                    .w(tk.r(9.5))
                    .wrap()
                    .a11y_hidden(),
            )
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(tk.r(0.5))
                    .center_y()
                    .a11y_hidden()
                    .child(
                        text(from.clone())
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .wrap()
                            .shrink(),
                    )
                    .child(ic(&I::ARROW_RIGHT, tk.r(0.875), tk.muted_fg))
                    .child(
                        text(to.clone())
                            .font(tk.tsm(500.0))
                            .color(tk.fg)
                            .wrap()
                            .shrink(),
                    ),
            );
        list = list.child(if i > 0 {
            r.border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
        } else {
            r
        });
    }
    out.push(labelled(tk, &heading, list));
    let cons = sm::consequences(p);
    let what = t("onboarding:books.settings.preview.consequencesHeading");
    let mut items = col()
        .key("books-set-consequences")
        .w_full()
        .gap(tk.r(0.5))
        .role("list")
        .label(what.clone());
    if cons.is_empty() {
        let s = t("onboarding:books.settings.preview.nothingElse");
        items = items.child(
            text(s.clone())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full()
                .role("listitem")
                .label(s),
        );
    }
    for c in &cons {
        let (s, warn) = consequence_text(c, base, p.after.as_ref());
        let (glyph, ink) = if warn {
            (&I::TRIANGLE_ALERT, kit::amber(tk))
        } else {
            (&I::INFO, tk.muted_fg)
        };
        items = items.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .role("listitem")
                .label(s.clone())
                .child(
                    row()
                        .pt(tk.r(0.125))
                        .a11y_hidden()
                        .child(ic(glyph, tk.r(1.0), ink)),
                )
                .child(
                    text(s)
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0)
                        .a11y_hidden(),
                ),
        );
    }
    out.push(labelled(tk, &what, items));
    out
}

/// Mounts the dialog (Book settings, or its preview step).
pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let f = &app.reports.books.form;
    let b = &app.reports.books;
    let mut inner: Vec<El> = Vec::new();
    let key = "books-settings";
    let width = tk.r(36.0);
    let Some(base) = f.base.clone() else {
        let title = t("onboarding:books.renameProfile.title");
        let desc = t("onboarding:books.settings.description");
        let mut body = col()
            .w_full()
            .gap(tk.r(1.0))
            .child(dialog_header(tk, &title, &desc));
        body = match &f.load_error {
            Some(e) => body.child(error_box(tk, e)).child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .justify_end()
                    .child(
                        button(
                            tk,
                            "books-settings-cancel",
                            Variant::Outline,
                            Size::Default,
                            None,
                            &t("common:actions.cancel"),
                            true,
                        )
                        .on_press(super::dialogs::close),
                    )
                    .child(
                        button(
                            tk,
                            "books-settings-retry",
                            Variant::Default,
                            Size::Default,
                            None,
                            &t("common:actions.retry"),
                            true,
                        )
                        .on_press(|a: &mut App| {
                            let (ws, id, name) = (
                                a.reports.books.form.ws.clone(),
                                a.reports.books.form.profile_id.clone(),
                                a.reports.books.name.text.clone(),
                            );
                            open(a, &ws, &id, &name);
                        }),
                    ),
            ),
            None => body.child(
                row()
                    .key("books-settings-loading")
                    .w_full()
                    .py(tk.r(2.0))
                    .gap(tk.r(0.5))
                    .justify_center()
                    .center_y()
                    .role("status")
                    .label(t("common:state.loading"))
                    .child(spinner(tk, tk.r(1.0)))
                    .child(
                        text(t("common:state.loading"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .a11y_hidden(),
                    ),
            ),
        };
        kit::dialog_scroll(
            app,
            tk,
            layer,
            key,
            &title,
            &desc,
            width,
            body,
            super::dialogs::close,
        );
        return;
    };
    let Some(c) = choices(f) else { return };
    let switched = f.country != base.tax_country;
    if let Some(p) = &f.preview {
        let title = t("onboarding:books.settings.preview.title");
        let desc = tr(
            "onboarding:books.settings.preview.description",
            &[("name", &base.label)],
        );
        let mut body = col()
            .w_full()
            .gap(tk.r(1.0))
            .child(dialog_header(tk, &title, &desc));
        for e in preview_body(tk, f, &base, p) {
            body = body.child(e);
        }
        if let Some(e) = &b.error {
            body = body.child(error_box(tk, e));
        }
        body = body.child(footer(
            app,
            tk,
            &t("common:actions.back"),
            back,
            &t("onboarding:books.settings.preview.apply"),
            apply,
            true,
        ));
        kit::dialog_scroll(
            app,
            tk,
            layer,
            key,
            &title,
            &desc,
            width,
            body,
            super::dialogs::close,
        );
        return;
    }
    let title = t("onboarding:books.renameProfile.title");
    let desc = t("onboarding:books.settings.description");
    // Wide enough, the sections sit in two columns (the book, its country and tax year; the cost
    // basis, holding period and review options), so the form fits a screen instead of a long
    // scroll; a narrow window keeps one column.
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let wide = win_w >= tk.r(66.0);
    let width = if wide { tk.r(62.0) } else { width };
    let mut body = col()
        .w_full()
        .gap(tk.r(1.25))
        .child(dialog_header(tk, &title, &desc));
    let name_label = t("onboarding:books.renameProfile.nameLabel");
    // The name needs no section heading of its own ("Book" over "Book name").
    let name_field = col().key("books-set-sec-book").w_full().child(labelled(
        tk,
        &name_label,
        input(
            app,
            tk,
            "books-set-name",
            &name_label,
            &b.name,
            None,
            bad(app, "name"),
            None,
            |a| &mut a.reports.books.name,
            "name",
        ),
    ));
    let left = col()
        .gap(tk.r(1.25))
        .child(name_field)
        .child(country_section(app, tk, &mut inner, f, &c))
        .child(tax_year_section(app, tk, &mut inner, f, &c, switched));
    if wide {
        let right = col()
            .gap(tk.r(1.25))
            .child(cost_basis_section(app, tk, &mut inner, f, &c, true))
            .child(holding_section(app, tk, f, &c, switched))
            .child(review_section(app, tk, f));
        body = body.child(
            row()
                .key("books-set-columns")
                .w_full()
                .gap(tk.r(2.0))
                .items_start()
                .child(left.grow(1.0).w(0.0).min_w(0.0))
                .child(right.grow(1.0).w(0.0).min_w(0.0)),
        );
    } else {
        body = body.child(left.w_full()).child(
            col()
                .w_full()
                .gap(tk.r(1.25))
                .child(cost_basis_section(app, tk, &mut inner, f, &c, false))
                .child(holding_section(app, tk, f, &c, switched))
                .child(review_section(app, tk, f)),
        );
    }
    if let Some(e) = &b.error {
        body = body.child(error_box(tk, e));
    }
    let loading_choices = switched
        && f.choices
            .as_ref()
            .is_none_or(|x| x.tax_country != f.country);
    body = body.child(footer(
        app,
        tk,
        &t("common:actions.cancel"),
        super::dialogs::close,
        &t("onboarding:books.settings.reviewChanges"),
        review,
        !loading_choices,
    ));
    kit::dialog_scroll(
        app,
        tk,
        layer,
        key,
        &title,
        &desc,
        width,
        body,
        super::dialogs::close,
    );
    layer.extend(inner);
}

/// Escape steps back from the preview before it closes the dialog.
pub fn on_key(app: &mut App, ev: &KeyEvent) -> bool {
    if !matches!(app.reports.books.dialog, Some(Dialog::Settings { .. }))
        || app.reports.books.popup.is_some()
    {
        return false;
    }
    match ev.key {
        Key::Escape
            if app.reports.books.form.preview.is_some() && !app.reports.books.submitting =>
        {
            back(app);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod reseed_tests {
    use super::*;

    fn saved(method: &str) -> Settings {
        Settings {
            profile_id: "p".into(),
            tax_country: "generic".into(),
            fiat_currency: "USD".into(),
            method: method.into(),
            pool: "global".into(),
            tax_year_start: "01-01".into(),
            timezone: "UTC".into(),
            ..Default::default()
        }
    }

    /// A fresh read after an outside change (the CLI set HIFO) refreshes untouched controls, so
    /// a later review doesn't propose reverting it; a control the user edited keeps their value.
    #[test]
    fn a_fresh_read_refreshes_untouched_fields_only() {
        let mut app = crate::app::tests::quiet_app();
        seed(&mut app, saved("FIFO"));
        seed(&mut app, saved("HIFO"));
        assert_eq!(form(&mut app).method, "HIFO");
        form(&mut app).method = "LIFO".into();
        form(&mut app).pool = "wallet".into();
        let mut later = saved("HIFO");
        later.pool = "global".into();
        later.fiat_currency = "GBP".into();
        seed(&mut app, later);
        let f = form(&mut app);
        assert_eq!((f.method.as_str(), f.pool.as_str()), ("LIFO", "wallet"));
        assert_eq!(f.currency.text, "GBP");
    }
}
