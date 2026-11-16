//! Book settings: one read and one change for every setting of any book,
//! with a dry run that reports what a change would do before it is applied
//! (`docs/reference/tax.md#book-settings`).
//!
//! The policy columns live on the `profiles` row. A book's own tax calendar
//! (its tax year's first day and its zone, where its country lets it
//! choose) is stored beside them under the per-book settings key
//! `tax.calendar.profile.<id>`, as other per-book policies are
//! (`freshness.policy.profile.<id>`): a book without the key has its
//! country's calendar, so existing books and their stored bytes are
//! unchanged. Every change goes through [`configure`], which validates the
//! whole result against the country registry, bumps the journal input
//! version when anything the journals read changes, and, as a dry run,
//! rolls its own writes back and returns the preview.
use super::*;
use kassiber_tax_countries::{book, BookCalendar, CalendarError, Country};

/// The per-book settings key of a book's own tax calendar.
pub fn calendar_key(profile_id: &str) -> String {
    format!("tax.calendar.profile.{profile_id}")
}

/// A book's stored choices: its tax year's first day and its zone, each
/// `None` for its country's.
type Stored = (Option<String>, Option<String>);

/// The stored choices of a book: its tax year's first day and its zone,
/// each `None` for its country's.
pub fn stored_calendar(conn: &Connection, profile_id: &str) -> Result<Stored> {
    read_calendar(conn, profile_id)?
}

/// The stored choices, or why the stored value cannot be read (the outer
/// error is the database's).
fn read_calendar(conn: &Connection, profile_id: &str) -> Result<Result<Stored>> {
    let Some(raw) = setting(conn, &calendar_key(profile_id))? else {
        return Ok(Ok((None, None)));
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(Err(AppError::new(
            "internal_error",
            "The book's stored tax calendar is not valid JSON",
        )));
    };
    let field = |key: &str| {
        parsed
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    Ok(Ok((field("tax_year_start"), field("timezone"))))
}

/// A book's calendar where the book must stay readable and repairable (its
/// settings, `profiles get`, a settings change): a stored choice the
/// country no longer allows, or a value that cannot be read, is reported
/// rather than refused. Every reader that computes with the calendar (the
/// journals, the reports) goes through [`calendar`] and fails closed.
pub(crate) struct Readable {
    /// The calendar in force: the stored choice, or the country's when it
    /// cannot be used.
    pub book: BookCalendar,
    /// What is stored (`None` halves are the country's; both `None` when
    /// the value cannot be read).
    pub stored: Stored,
    /// `calendar_invalid`: the refusal's `code`, `message` and `hint` and
    /// the stored `tax_year_start` and `timezone`, when the choice cannot
    /// be used.
    pub invalid: Option<Value>,
}

pub(crate) fn readable(conn: &Connection, profile_id: &str, country: &Country) -> Result<Readable> {
    let report = |error: AppError, stored: &Stored| {
        let text = |value: &Option<String>| value.clone().map(s).unwrap_or(Value::Null);
        dict([
            ("code", s(error.code)),
            ("message", s(error.message)),
            ("hint", error.hint.map(s).unwrap_or(Value::Null)),
            ("tax_year_start", text(&stored.0)),
            ("timezone", text(&stored.1)),
        ])
    };
    let stored = match read_calendar(conn, profile_id)? {
        Ok(stored) => stored,
        Err(error) => {
            let stored = (None, None);
            return Ok(Readable {
                book: country.default_calendar(),
                invalid: Some(report(error, &stored)),
                stored,
            });
        }
    };
    Ok(
        match country.book_calendar(stored.0.as_deref(), stored.1.as_deref()) {
            Ok(book) => Readable {
                book,
                stored,
                invalid: None,
            },
            Err(error) => Readable {
                book: country.default_calendar(),
                invalid: Some(report(calendar_error(error), &stored)),
                stored,
            },
        },
    )
}

fn calendar_error(error: CalendarError) -> AppError {
    AppError::new(error.code(), error.to_string())
        .with_hint(error.hint())
        .with_details(dict([("reason", s(error.code()))]))
}

/// The tax calendar of the book `profile` (a `profiles` row): its country's
/// with its stored choices. A stored choice its country does not allow
/// fails closed with the registry's code.
pub fn calendar(conn: &Connection, profile: &Value) -> Result<BookCalendar> {
    let country = profiles::tax_country(profile)?;
    let (start, zone) = stored_calendar(conn, &text(profile, "id"))?;
    country
        .book_calendar(start.as_deref(), zone.as_deref())
        .map_err(calendar_error)
}

/// The tax calendar of the book `profile_id`.
pub fn calendar_of(conn: &Connection, profile_id: &str) -> Result<BookCalendar> {
    let profile = one(conn, "SELECT * FROM profiles WHERE id=?", [profile_id])?;
    calendar(conn, &profile)
}

/// The keys the journals add to the profile the tax engine request reads
/// (`kassiber_custody::ledger_request::config::BOOK_TAX_YEAR_START`,
/// `BOOK_TIMEZONE` and `BOOK_WALLET_ORDER`): only the calendar choices the
/// book stored, and a `global_then_wallet` book's wallet order, so every
/// other book's request keeps its bytes. The request builder validates them.
pub fn engine_profile_keys(
    conn: &Connection,
    profile_id: &str,
) -> Result<Vec<(&'static str, String)>> {
    let (start, zone) = stored_calendar(conn, profile_id)?;
    let mut keys = Vec::new();
    if let Some(start) = start {
        keys.push(("tax_year_start", start));
    }
    if let Some(zone) = zone {
        keys.push(("tax_timezone", zone));
    }
    if let Some(order) = wallet_allocation_order(conn, profile_id)? {
        keys.push(("wallet_allocation_order", order));
    }
    Ok(keys)
}

/// A `global_then_wallet` book's wallet ids by when they were added (then
/// by id), as a JSON array: the order the United States' 2025 allocation
/// fills its wallets in, which a rename does not change
/// (`kassiber_custody::ledger_request::config::BOOK_WALLET_ORDER`). `None`
/// for every other book, so its request keeps its bytes.
fn wallet_allocation_order(conn: &Connection, profile_id: &str) -> Result<Option<String>> {
    let scope: Option<String> = conn
        .query_row(
            "SELECT cost_basis_pool_scope FROM profiles WHERE id=?",
            [profile_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?
        .flatten();
    let scope = scope.unwrap_or_default();
    if kassiber_pytext::strip(&scope).to_lowercase() != "global_then_wallet" {
        return Ok(None);
    }
    let mut statement = conn
        .prepare("SELECT id FROM wallets WHERE profile_id=? ORDER BY created_at, id")
        .map_err(sql_error)?;
    let ids = statement
        .query_map([profile_id], |row| row.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    Ok(Some(serde_json::Value::from(ids).to_string()))
}

/// Stores `book` for the book `profile_id` of `country`: only what differs
/// from the country's calendar, and no key at all when nothing does.
fn store_calendar(
    conn: &Connection,
    profile_id: &str,
    country: &Country,
    book: BookCalendar,
) -> Result<()> {
    let key = calendar_key(profile_id);
    let mut stored = serde_json::Map::new();
    if book.tax_year_start != country.tax_year_start {
        stored.insert("tax_year_start".into(), book.start_text().into());
    }
    if book.timezone != country.timezone {
        stored.insert("timezone".into(), book.timezone.into());
    }
    if stored.is_empty() {
        conn.execute("DELETE FROM settings WHERE key=?", [&key])
            .map_err(sql_error)?;
        return Ok(());
    }
    set_setting(conn, &key, &serde_json::Value::Object(stored).to_string())
}

/// Removes the stored calendars of books that no longer exist.
pub(crate) fn forget_deleted(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM settings WHERE key LIKE 'tax.calendar.profile.%' AND substr(key, 22) NOT IN (SELECT id FROM profiles)",
        [],
    )
    .map_err(sql_error)?;
    Ok(())
}

/// Copies `source`'s stored calendar to the new book `target` when both
/// have the same country (a copied book's settings).
pub(crate) fn copy_calendar(conn: &Connection, source: &str, target: &str) -> Result<()> {
    let source_profile = one(conn, "SELECT * FROM profiles WHERE id=?", [source])?;
    let target_profile = one(conn, "SELECT * FROM profiles WHERE id=?", [target])?;
    if text(&source_profile, "tax_country") != text(&target_profile, "tax_country") {
        return Ok(());
    }
    if let Some(raw) = setting(conn, &calendar_key(source))? {
        set_setting(conn, &calendar_key(target), &raw)?;
    }
    Ok(())
}

/// The calendar a change asks for: `None` keeps the stored choice,
/// `"default"` (or an empty string) asks for the country's.
fn requested(args: &Value, key: &str) -> Result<Option<Option<String>>> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(text)) => {
            let text = text.as_str().map(str::trim).unwrap_or_default();
            Ok(Some(
                (!text.is_empty() && !text.eq_ignore_ascii_case("default"))
                    .then(|| text.to_owned()),
            ))
        }
        Some(_) => Err(AppError::new(
            "validation",
            format!("{key} must be a string"),
        )),
    }
}

/// The book a request names: `profile_id` (any book of any book set, as
/// the desktop and the assistant name it), else the CLI's `profile` and
/// `workspace` selection or the open book.
pub fn select_book(conn: &Connection, args: &Value) -> Result<(Value, Value)> {
    let Some(id) = get(args, "profile_id").filter(|v| !matches!(v, Value::Null)) else {
        return references::scope(conn, args);
    };
    let id = match id {
        Value::Str(text) => text.as_str().unwrap_or_default().trim().to_owned(),
        _ => return Err(AppError::new("validation", "profile_id must be a string")),
    };
    let found = rows(conn, "SELECT * FROM profiles WHERE id=?", [id.as_str()])?;
    let Some(profile) = found.into_iter().next() else {
        return Err(AppError::new("validation", "Book not found.")
            .with_hint("Choose an existing book.")
            .with_details(dict([("profile_id", s(id))])));
    };
    let workspace = one(
        conn,
        "SELECT * FROM workspaces WHERE id=?",
        [text(&profile, "workspace_id")],
    )?;
    Ok((workspace, profile))
}

/// The settings a book has and may choose: one projection the CLI, the
/// desktop and the assistant read. A stored calendar the country no longer
/// allows is shown as stored, with `calendar_invalid` saying why, so the
/// book can still be opened and reset (`default`).
pub fn settings(conn: &Connection, profile: &Value) -> Result<Value> {
    let country = profiles::tax_country(profile)?;
    let id = text(profile, "id");
    let readable = readable(conn, &id, country)?;
    let book = readable.book;
    let current = setting(conn, "context_profile")?.is_some_and(|current| current == id);
    let method = text(profile, "gains_algorithm");
    let scope = kassiber_pytext::strip(&text(profile, "cost_basis_pool_scope")).to_lowercase();
    let strings = |items: &[&str]| Value::List(items.iter().map(|item| s(*item)).collect());
    let mut projection = dict([
        ("profile_id", s(id.as_str())),
        (
            "workspace_id",
            get(profile, "workspace_id").cloned().unwrap_or(Value::Null),
        ),
        (
            "label",
            get(profile, "label").cloned().unwrap_or(Value::Null),
        ),
        ("is_current", Value::Bool(current)),
        ("tax_country", s(country.code)),
        ("tax_country_preview", Value::Bool(country.preview)),
        (
            "fiat_currency",
            get(profile, "fiat_currency")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "fiat_currency_locked",
            Value::Bool(matches!(
                country.fiat,
                kassiber_tax_countries::Fiat::Fixed(_)
            )),
        ),
        (
            "tax_long_term_days",
            profiles::effective_long_term_days(profile),
        ),
        (
            "tax_long_term_days_editable",
            Value::Bool(matches!(
                country.long_term,
                kassiber_tax_countries::LongTerm::ProfileDays { .. }
            )),
        ),
        ("gains_algorithm", s(method.as_str())),
        ("gains_algorithms", strings(country.methods)),
        ("cost_basis_pool_scope", s(scope.as_str())),
        ("cost_basis_pool_scopes", strings(country.pool_scopes)),
        (
            "method_pool_scopes",
            dict(country.methods.iter().map(|method| {
                (
                    *method,
                    Value::List(
                        country
                            .pool_scopes
                            .iter()
                            .filter(|scope| {
                                kassiber_tax_countries::method_allows_pool_scope(method, scope)
                            })
                            .map(|scope| s(*scope))
                            .collect(),
                    ),
                )
            })),
        ),
        (
            "require_coarse_review",
            Value::Bool(flag(profile, "require_coarse_review")),
        ),
        (
            "bitcoin_rail_carrying_value",
            Value::Bool(flag(profile, "bitcoin_rail_carrying_value")),
        ),
        (
            "tax_year_start",
            s(match (&readable.invalid, &readable.stored.0) {
                (Some(_), Some(stored)) => stored.clone(),
                _ => book.start_text(),
            }),
        ),
        (
            "tax_year_start_default",
            s(book::format_start(country.tax_year_start)),
        ),
        ("tax_year_editable", Value::Bool(country.tax_year_editable)),
        (
            "tax_year_presets",
            Value::List(if country.tax_year_editable {
                kassiber_tax_countries::TAX_YEAR_PRESETS
                    .iter()
                    .map(|preset| {
                        dict([
                            ("id", s(preset.id)),
                            ("start", s(book::format_start(preset.start))),
                            ("name", s(preset.name_en)),
                        ])
                    })
                    .collect()
            } else {
                Vec::new()
            }),
        ),
        (
            "timezone",
            s(match (&readable.invalid, &readable.stored.1) {
                (Some(_), Some(stored)) => stored.clone(),
                _ => book.timezone.to_owned(),
            }),
        ),
        ("timezone_default", s(country.timezone)),
        ("timezones", strings(&country.book_zones())),
        (
            "calendar_is_default",
            Value::Bool(readable.invalid.is_none() && book.is_default(country)),
        ),
    ]);
    // Only a book whose stored calendar cannot be used has the key.
    if let Some(invalid) = readable.invalid {
        replace(&mut projection, "calendar_invalid", invalid);
    }
    Ok(projection)
}

/// One field's change in a preview.
fn change(field: &str, from: Value, to: Value) -> Value {
    dict([("field", s(field)), ("from", from), ("to", to)])
}

/// The keys of a change: the profile columns `profiles.set` takes and the
/// book calendar's two.
pub const CHANGE_KEYS: &[&str] = &[
    "label",
    "fiat_currency",
    "tax_country",
    "tax_long_term_days",
    "gains_algorithm",
    "cost_basis_pool_scope",
    "require_coarse_review",
    "bitcoin_rail_carrying_value",
    "tax_year_start",
    "timezone",
];

/// Changes any of a book's settings (`profiles.configure`, and
/// `ui.profiles.update`). `args` names the book (`profile`, `workspace`)
/// and the settings to change; `dry_run` (a boolean) applies nothing and
/// returns what the change would do. The calendar is validated against the
/// resulting country: a region switch that the stored calendar no longer
/// fits resets it to the new country's, which the result reports.
pub fn configure(conn: &Connection, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let dry_run = match get(args, "dry_run") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(_) => {
            return Err(AppError::new("validation", "dry_run must be a boolean"));
        }
    };
    if CHANGE_KEYS.iter().all(|key| optional(args, key).is_none()) {
        return Err(
            AppError::new("validation", "No book setting change was provided.")
                .with_hint(format!("Pass one or more of: {}", CHANGE_KEYS.join(", "))),
        );
    }
    let (_, before) = select_book(conn, args)?;
    let id = text(&before, "id");
    let before_settings = settings(conn, &before)?;
    conn.execute_batch("SAVEPOINT book_settings_change")
        .map_err(sql_error)?;
    let outcome = apply(conn, args, &before, sources);
    let outcome = match outcome {
        Ok(result) if dry_run => {
            let after = one(conn, "SELECT * FROM profiles WHERE id=?", [id.as_str()])?;
            preview(conn, &before, &before_settings, &after, result)
        }
        other => other,
    };
    if dry_run || outcome.is_err() {
        conn.execute_batch("ROLLBACK TO book_settings_change; RELEASE book_settings_change")
            .map_err(sql_error)?;
    } else {
        conn.execute_batch("RELEASE book_settings_change")
            .map_err(sql_error)?;
    }
    outcome
}

/// Applies a change inside the caller's savepoint.
fn apply(conn: &Connection, args: &Value, before: &Value, sources: &dyn Sources) -> Result<Value> {
    let id = text(before, "id");
    let start = requested(args, "tax_year_start")?;
    let zone = requested(args, "timezone")?;
    // A stored choice the country no longer allows (or cannot be read) does
    // not block the change that repairs it: `default` resets it, and any
    // other change must still yield a calendar the country allows.
    let previous = readable(conn, &id, profiles::tax_country(before)?)?;
    let before_calendar = previous.book;
    let before_invalid = previous.invalid.is_some();
    // Both halves unset yet invalid: the stored value could not be read.
    let unreadable = before_invalid && previous.stored == (None, None);
    let (stored_start, stored_zone) = previous.stored;
    let mut columns = dict::<&str>([
        (
            "workspace",
            get(before, "workspace_id").cloned().unwrap_or(Value::Null),
        ),
        ("profile", s(id.as_str())),
    ]);
    let mut any_column = false;
    for key in &CHANGE_KEYS[..8] {
        if let Some(value) = optional(args, key) {
            replace(&mut columns, key, value.clone());
            any_column = true;
        }
    }
    let result = if any_column {
        profiles::execute(conn, "profiles.set", &columns, sources)?
    } else {
        Value::Null
    };
    let after = one(conn, "SELECT * FROM profiles WHERE id=?", [id.as_str()])?;
    let country = profiles::tax_country(&after)?;
    let country_changed = text(before, "tax_country") != text(&after, "tax_country");
    // `profiles set` drops a stored calendar the new country does not allow.
    // An unreadable stored value is dropped by any change.
    let mut reset = get(&result, "tax_calendar_reset").is_some() || unreadable;
    let (stored_start, stored_zone) = if reset {
        (None, None)
    } else {
        (stored_start, stored_zone)
    };
    // A choice not asked for keeps the stored one if the (new) country
    // still allows it; a region switch resets one it does not.
    let keep = |asked: Option<Option<String>>, stored: Option<String>| match asked {
        Some(value) => (value, true),
        None => (stored, false),
    };
    let (start, start_asked) = keep(start, stored_start.clone());
    let (zone, zone_asked) = keep(zone, stored_zone.clone());
    let book = match country.book_calendar(start.as_deref(), zone.as_deref()) {
        Ok(book) => book,
        Err(error) => {
            let asked_failed = match &error {
                CalendarError::InvalidStart(_) | CalendarError::StartLocked { .. } => start_asked,
                CalendarError::UnknownZone(_) | CalendarError::ZoneLocked { .. } => zone_asked,
            };
            if asked_failed || !country_changed {
                return Err(calendar_error(error));
            }
            // The stored choice does not fit the new country.
            reset = true;
            let start = if start_asked { start.as_deref() } else { None };
            let zone = if zone_asked { zone.as_deref() } else { None };
            country.book_calendar(start, zone).map_err(calendar_error)?
        }
    };
    store_calendar(conn, &id, country, book)?;
    let bumped = text(before, "journal_input_version") != text(&after, "journal_input_version");
    if (before_invalid || before_calendar != book) && !bumped {
        // The tax years the journals stored are cut differently now.
        conn.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?",[id.as_str()]).map_err(sql_error)?;
    }
    // `profiles get`'s projection after both writes, with what `profiles
    // set` reported beside it.
    let set_result = result;
    let (w, p) = references::scope(conn, &columns)?;
    let mut result = profiles::details_of(conn, &w, &p)?;
    if let Some(holds) = get(&set_result, "price_currency_holds") {
        replace(&mut result, "price_currency_holds", holds.clone());
    }
    if reset {
        replace(&mut result, "tax_calendar_reset", Value::Bool(true));
    }
    Ok(result)
}

/// The dry run's answer: every field that would change, whether journals
/// must be reprocessed, the rows a currency change would hold, and the
/// readiness effects.
fn preview(
    conn: &Connection,
    before: &Value,
    before_settings: &Value,
    after: &Value,
    applied: Value,
) -> Result<Value> {
    let after_settings = settings(conn, after)?;
    let mut changes = Vec::new();
    for key in [
        "label",
        "tax_country",
        "fiat_currency",
        "tax_long_term_days",
        "gains_algorithm",
        "cost_basis_pool_scope",
        "require_coarse_review",
        "bitcoin_rail_carrying_value",
        "tax_year_start",
        "timezone",
    ] {
        let from = get(before_settings, key).cloned().unwrap_or(Value::Null);
        let to = get(&after_settings, key).cloned().unwrap_or(Value::Null);
        if from != to {
            changes.push(change(key, from, to));
        }
    }
    let version = |profile: &Value| text(profile, "journal_input_version");
    let reprocess = version(before) != version(after);
    let id = text(before, "id");
    let count = |sql: &str| -> Result<i64> {
        conn.query_row(sql, [id.as_str()], |row| row.get(0))
            .map_err(sql_error)
    };
    let transactions =
        count("SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0")?;
    let processed = !matches!(get(before, "last_processed_at"), None | Some(Value::Null));
    let filed = count(
        "SELECT COUNT(*) FROM filed_report_snapshots WHERE profile_id=? AND report_state='filed'",
    )?;
    let year_changed = changes.iter().any(|change| {
        matches!(
            get(change, "field"),
            Some(Value::Str(field)) if matches!(field.as_str(), Some("tax_year_start" | "timezone" | "tax_country"))
        )
    });
    let holds = get(&applied, "price_currency_holds").cloned();
    let held = holds
        .as_ref()
        .and_then(|holds| get(holds, "transactions"))
        .map(py_str)
        .and_then(|count| count.parse::<i64>().ok())
        .unwrap_or(0);
    let mut readiness = Vec::new();
    if reprocess && processed {
        readiness.push(s("journals_stale_until_reprocessed"));
    }
    if held > 0 {
        readiness.push(s("price_currency_mismatch_holds"));
    }
    if reprocess && filed > 0 {
        readiness.push(s("filed_reports_may_be_impacted"));
    }
    if year_changed && !changes.is_empty() {
        readiness.push(s("tax_years_regrouped"));
    }
    let mut preview = dict([
        ("dry_run", Value::Bool(true)),
        ("profile_id", s(id.as_str())),
        ("changes", Value::List(changes)),
        (
            "journals",
            dict([
                ("reprocess", Value::Bool(reprocess)),
                ("processed", Value::Bool(processed)),
                ("transactions", Value::Int(transactions.into())),
            ]),
        ),
        ("filed_reports", Value::Int(filed.into())),
        ("readiness", Value::List(readiness)),
        ("settings", after_settings),
    ]);
    if let Some(holds) = holds {
        replace(&mut preview, "price_currency_holds", holds);
    }
    if get(&applied, "tax_calendar_reset").is_some() {
        replace(&mut preview, "tax_calendar_reset", Value::Bool(true));
    }
    Ok(preview)
}
