//! The custody steps of the store's ordered compatibility pass
//! (`kassiber_store::custody_hooks`): Python's
//! `refresh_legacy_authored_components`, `mark_legacy_pair_fee_reviews`'
//! validator decision and `backfill_legacy_componentless_review_relations`.
//! The product binary installs them with [`install`].
use super::legacy_refresh::{
    activate_native_review, atomic, domain, link, migrate_pair_groups, pair_rows, python_int,
    python_int_or_zero, record_issue, resolve_issues, supersede, Error, NativeReview,
    RefreshResult,
};
use super::{component_rows as components, component_store as store, pair_spec as spec, *};
use kassiber_envelope::{set, truthy};
use kassiber_pyjson::BigInt;
use kassiber_store::custody_hooks::{CustodyMigrations, MigrationClock};

struct Migrations;

/// Install the custody migrations for every migrating open in this process.
pub fn install() {
    kassiber_store::custody_hooks::install(&Migrations);
}

impl CustodyMigrations for Migrations {
    fn refresh_legacy_authored_components(
        &self,
        conn: &Connection,
        clock: MigrationClock<'_>,
    ) -> kassiber_store::Result<()> {
        let sources = Clock(clock.pinned_timestamp().map(str::to_owned));
        sql::capture_sqlite(|| refresh_legacy_authored_components(conn, &sources))
            .map(|_| ())
            .map_err(|e| store_error("refresh_legacy_authored_components", e))
    }
    fn requires_legacy_pair_fee_review(
        &self,
        conn: &Connection,
        component_id: &str,
    ) -> kassiber_store::Result<bool> {
        requires_legacy_pair_fee_review(conn, component_id)
            .map_err(|e| store_error("mark_legacy_pair_fee_reviews", Error::App(e)))
    }
}

/// A failure Python would raise out of `open_db`: the store rolls the step
/// back and refuses the open. Busy and storage errors keep their kind.
fn store_error(step: &str, error: Error) -> kassiber_store::Error {
    let code = match error {
        Error::Sqlite(error) => return kassiber_store::map_sqlite_error(error),
        Error::App(error) => {
            if matches!(error.code.as_str(), "database_busy" | "database_error") {
                return kassiber_store::Error::new(
                    if error.code == "database_busy" {
                        kassiber_store::ErrorKind::DatabaseBusy
                    } else {
                        kassiber_store::ErrorKind::DatabaseError
                    },
                    "database operation failed",
                )
                .retryable(error.retryable);
            }
            error.code
        }
        Error::Python { family, .. } => family.to_owned(),
    };
    kassiber_store::Error::new(
        kassiber_store::ErrorKind::UnsupportedSchemaLayout,
        "a legacy custody migration could not complete; the book was left unchanged",
    )
    .details(serde_json::json!({
        "migration_step": step,
        "reason": "custody_migration_failed",
        "error_code": code,
    }))
}

/// Python's clocks during the migration: `now_iso()` (whole seconds) for
/// migration issues and `datetime.now().isoformat()` (microseconds when not
/// zero) in the component layer. A pinned instant serves both.
struct Clock(Option<String>);

impl Sources for Clock {
    fn uuid(&self, _: &Connection) -> Result<String> {
        Err(AppError::new(
            "internal_error",
            "legacy migration requires deterministic identities",
        ))
    }
    fn now(&self, conn: &Connection) -> Result<String> {
        match &self.0 {
            Some(pinned) => Ok(pinned.clone()),
            None => SystemSources.now(conn),
        }
    }
    fn now_precise(&self, conn: &Connection) -> Result<String> {
        match &self.0 {
            Some(pinned) => Ok(isoformat_micros(pinned)),
            None => SystemSources.now_precise(conn),
        }
    }
}

/// The same instant as `datetime.isoformat()` writes it: a millisecond
/// fraction widens to microseconds; a zero fraction is omitted.
fn isoformat_micros(pinned: &str) -> String {
    let Some(body) = pinned.strip_suffix('Z') else {
        return pinned.to_owned();
    };
    match body.split_once('.') {
        Some((seconds, fraction)) if fraction.bytes().all(|b| b.is_ascii_digit()) => {
            let micros = format!("{fraction:0<6}");
            if micros.bytes().all(|b| b == b'0') {
                format!("{seconds}Z")
            } else {
                format!("{seconds}.{}Z", &micros[..6])
            }
        }
        _ => pinned.to_owned(),
    }
}

/// `get_component(...)["validation"]["issues"]` names the legacy pair fee
/// blocker.
fn requires_legacy_pair_fee_review(c: &Connection, id: &str) -> Result<bool> {
    let component = super::legacy_refresh::component(c, id)?;
    Ok(store::list(&value(&component, "validation"), "issues")
        .iter()
        .any(|issue| text(issue, "code") == "legacy_pair_fee_allocation_review_required"))
}

/// `refresh_legacy_authored_components`: deleted rows, then connected pair
/// groups, then payouts, in one savepoint. Domain refusals become durable
/// migration issues (journal quarantines); anything else rolls back.
fn refresh_legacy_authored_components(c: &Connection, sources: &dyn Sources) -> RefreshResult<()> {
    let pending = rows(
        c,
        "SELECT (SELECT COUNT(*) FROM transaction_pairs WHERE component_id IS NULL)
          + (SELECT COUNT(*) FROM direct_swap_payouts WHERE component_id IS NULL)
          + (SELECT COUNT(*) FROM custody_authored_migration_issues WHERE resolved_at IS NULL) AS pending",
        [],
    )?;
    if !truthy(&value(&pending[0], "pending")) {
        return Ok(());
    }
    atomic(c, "custody_authored_migration", || {
        migrate_deleted_rows(c, sources)?;
        migrate_pair_groups(c, None, sources)?;
        migrate_payouts(c, sources)?;
        Ok(())
    })
}

const PAYOUT_FIELDS: &[&str] = &[
    "id",
    "workspace_id",
    "profile_id",
    "out_transaction_id",
    "kind",
    "policy",
    "payout_asset",
    "payout_amount",
    "payout_occurred_at",
    "payout_fiat_value",
    "payout_external_id",
    "counterparty",
    "notes",
    "swap_fee_msat",
    "swap_fee_kind",
    "out_amount",
    "deleted_at",
    "created_at",
];

/// `_payout_rows`: payouts joined to their observed outbound.
fn payout_rows(c: &Connection, include_deleted: bool) -> Result<Vec<Value>> {
    rows(
        c,
        &format!(
            "SELECT p.*, 'direct_swap_payouts' AS legacy_table,
            o.asset AS out_asset,o.amount AS out_tx_amount,o.fee AS out_fee,
            o.amount_includes_fee AS out_amount_includes_fee,o.direction AS out_direction,
            o.occurred_at AS out_occurred_at,o.raw_json AS out_raw_json,o.wallet_id AS out_wallet_id,
            ow.kind AS out_wallet_kind,ow.config_json AS out_wallet_config_json
            FROM direct_swap_payouts p JOIN transactions o ON o.id=p.out_transaction_id
            JOIN wallets ow ON ow.id=o.wallet_id {}
            ORDER BY p.profile_id,p.created_at,p.id",
            if include_deleted {
                ""
            } else {
                "WHERE p.deleted_at IS NULL"
            }
        ),
        [],
    )
}

fn empty(v: &Value) -> bool {
    matches!(v, Value::Null) || *v == s("")
}

/// `str(row["component_id"] or "")`.
fn linked_id(row: &Value) -> String {
    let id = value(row, "component_id");
    if truthy(&id) {
        py_str(&id)
    } else {
        String::new()
    }
}

/// `_canonical_payload`: the payout's fiat value in its canonical text.
fn source_hash(row: &Value, fields: &[&str]) -> RefreshResult<String> {
    let mut payload = project(row, fields);
    if fields.contains(&"payout_fiat_value") {
        let canonical = canonical_float(&value(row, "payout_fiat_value"))?;
        set(&mut payload, "payout_fiat_value", canonical);
    }
    Ok(store::hash(&payload, false)?)
}

/// `_migrate_deleted_legacy_rows`: author, activate and immediately
/// supersede (at the row's deletion time) a component for each deleted row.
fn migrate_deleted_rows(c: &Connection, sources: &dyn Sources) -> RefreshResult<()> {
    for (rows, term_kind, fields) in [
        (pair_rows(c, true)?, "transaction_pair", spec::FIELDS),
        (payout_rows(c, true)?, "direct_swap_payout", PAYOUT_FIELDS),
    ] {
        for row in rows {
            if empty(&value(&row, "deleted_at")) {
                continue;
            }
            let linked = linked_id(&row);
            if !linked.is_empty() {
                match super::legacy_refresh::component(c, &linked) {
                    Ok(component) if text(&component, "state") == "superseded" => {
                        resolve_issues(c, std::slice::from_ref(&row), sources)?;
                        continue;
                    }
                    Ok(_) => {}
                    // A delayed replay may name a component that is not
                    // present; the canonical path records the precise issue.
                    Err(error) if domain(&Error::App(error.clone())) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            let migrated = atomic(c, "custody_deleted_item", || {
                let hash = source_hash(&row, fields)?;
                let mut plan = if term_kind == "transaction_pair" {
                    super::legacy_refresh::group_spec(std::slice::from_ref(&row), false)?
                } else {
                    payout_spec(&row, &hash)?
                };
                if !linked.is_empty() {
                    spec::retarget(&mut plan, &linked);
                    supersede(
                        c,
                        &linked,
                        "replace staged deleted legacy review",
                        None,
                        sources,
                    )?;
                }
                let id = activate_native_review(
                    c,
                    &row,
                    plan,
                    &NativeReview {
                        term_kind,
                        source_hash: &hash,
                        change_reason: "migrate deleted legacy custody review",
                        evidence: dict([
                            ("legacy_table", value(&row, "legacy_table")),
                            ("legacy_source_id", s(py_str(&value(&row, "id")))),
                        ]),
                    },
                    sources,
                )?;
                supersede(
                    c,
                    &id,
                    "migrate deleted legacy custody review",
                    Some(&value(&row, "deleted_at")),
                    sources,
                )?;
                link(c, std::slice::from_ref(&row), &id)?;
                Ok(())
            });
            match migrated {
                Ok(()) => resolve_issues(c, std::slice::from_ref(&row), sources)?,
                Err(error) if domain(&error) => {
                    record_issue(c, std::slice::from_ref(&row), &error, sources)?
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

/// `_migrate_legacy_payouts`: each active payout becomes one activated
/// conversion; an effective active link is unchanged.
fn migrate_payouts(c: &Connection, sources: &dyn Sources) -> RefreshResult<()> {
    let payouts = payout_rows(c, false)?;
    atomic(c, "custody_payout_consolidation", || {
        for row in &payouts {
            let row = std::slice::from_ref(row);
            match migrate_payout(c, &row[0], sources) {
                Ok(()) => resolve_issues(c, row, sources)?,
                Err(error) if domain(&error) => record_issue(c, row, &error, sources)?,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    })
}

fn migrate_payout(c: &Connection, row: &Value, sources: &dyn Sources) -> RefreshResult<()> {
    let linked = linked_id(row);
    if !linked.is_empty() {
        let component = super::legacy_refresh::component(c, &linked)?;
        if text(&component, "effective_state") == "active" {
            return Ok(());
        }
    }
    atomic(c, "custody_payout_item", || {
        let hash = source_hash(row, PAYOUT_FIELDS)?;
        let mut plan = payout_spec(row, &hash)?;
        // Python then tries `_append_exact_native_residual` and
        // `_append_source_residual`. Neither can bind anything here: the
        // first needs the outbound's external id, which `_payout_rows` does
        // not select, and the second skips conversions, which every payout
        // spec is.
        if !linked.is_empty() {
            spec::retarget(&mut plan, &linked);
            supersede(
                c,
                &linked,
                "replace staged legacy direct payout",
                None,
                sources,
            )?;
        }
        let id = activate_native_review(
            c,
            row,
            plan,
            &NativeReview {
                term_kind: "direct_swap_payout",
                source_hash: &hash,
                change_reason: "migrate legacy direct payout",
                evidence: dict([
                    ("legacy_table", s("direct_swap_payouts")),
                    ("legacy_source_id", s(py_str(&value(row, "id")))),
                ]),
            },
            sources,
        )?;
        link(c, std::slice::from_ref(row), &id)?;
        Ok(())
    })
}

fn big(n: BigInt) -> Value {
    Value::Int(n)
}

/// `_payout_occurred_at`: the payout time canonicalized, if any.
fn payout_occurred_at(row: &Value) -> RefreshResult<Value> {
    let raw = value(row, "payout_occurred_at");
    if empty(&raw) {
        return Ok(Value::Null);
    }
    Ok(s(parse_legacy_timestamp(&raw)?))
}

/// `_payout_spec`.
fn payout_spec(row: &Value, source_hash: &str) -> RefreshResult<Value> {
    let out_asset = spec::asset(&value(row, "out_asset"));
    let payout_asset = spec::asset(&value(row, "payout_asset"));
    // `row_boundary_amounts(...).principal_msat` of the outbound.
    let amount = python_int_or_zero(&value(row, "out_tx_amount"))?;
    let fee = python_int_or_zero(&value(row, "out_fee"))?;
    let principal = if kassiber_pytext::lower(kassiber_pytext::strip(&direction(row))) == "outbound"
        && truthy(&value(row, "out_amount_includes_fee"))
    {
        amount - fee
    } else {
        amount
    };
    let explicit = value(row, "out_amount");
    let reviewed = if empty(&explicit) {
        principal
    } else {
        python_int(&explicit)?
    };
    let payout_amount = python_int(&value(row, "payout_amount"))?;
    let zero = BigInt::default();
    let physical_source = if reviewed > zero {
        reviewed.clone()
    } else {
        zero.clone()
    };
    let physical_payout = if payout_amount > zero {
        payout_amount
    } else {
        zero
    };
    let profile = py_str(&value(row, "profile_id"));
    let review = py_str(&value(row, "id"));
    let id = spec::stable(&["direct_swap_payout", &profile, &review, source_hash]);
    let source_leg = spec::stable(&[&id, "leg", "source"]);
    let target_leg = spec::stable(&[&id, "leg", "target"]);
    let out = dict([
        ("transaction_id", value(row, "out_transaction_id")),
        ("wallet_id", value(row, "out_wallet_id")),
        ("wallet_kind", value(row, "out_wallet_kind")),
        ("config_json", value(row, "out_wallet_config_json")),
        ("raw_json", value(row, "out_raw_json")),
        ("asset", s(&out_asset)),
    ]);
    let mut source = spec::leg(
        &out,
        &source_leg,
        "source",
        0,
        value(row, "out_occurred_at"),
        true,
        0,
    );
    set(&mut source, "amount_msat", big(physical_source.clone()));
    set(
        &mut source,
        "valuation_amount",
        big(physical_source.clone()),
    );
    let when = match payout_occurred_at(row)? {
        Value::Null => value(row, "out_occurred_at"),
        when => when,
    };
    let mut target = spec::leg(
        &dict([("asset", s(&payout_asset))]),
        &target_leg,
        "retained",
        0,
        when,
        true,
        0,
    );
    set(&mut target, "rail", s("untracked"));
    set(&mut target, "chain", Value::Null);
    set(&mut target, "network", Value::Null);
    set(&mut target, "amount_msat", big(physical_payout.clone()));
    set(
        &mut target,
        "valuation_amount",
        big(physical_source.clone()),
    );
    set(&mut target, "transaction_id", Value::Null);
    set(&mut target, "wallet_id", Value::Null);
    set(
        &mut target,
        "location_ref",
        s(format!("legacy-payout:{review}")),
    );
    let mut allocation = spec::allocation(
        spec::stable(&[&id, "allocation", "0"]),
        &source_leg,
        &target_leg,
        0,
        0,
    );
    set(&mut allocation, "source_amount_msat", big(physical_source));
    set(&mut allocation, "sink_amount_msat", big(physical_payout));
    Ok(dict([
        ("component_id", s(&id)),
        (
            "lineage_id",
            s(spec::stable(&[
                "direct_swap_payout",
                &profile,
                &review,
                "lineage",
            ])),
        ),
        ("component_type", s("swap")),
        ("conservation_mode", s("conversion")),
        ("conversion_policy", s(py_str(&value(row, "policy")))),
        ("conversion_reviewed", Value::Bool(true)),
        ("legs", Value::List(vec![source, target])),
        ("allocations", Value::List(vec![allocation])),
        ("reviewed_source_amount_msat", big(reviewed)),
    ]))
}

fn direction(row: &Value) -> String {
    let raw = value(row, "out_direction");
    if truthy(&raw) {
        py_str(&raw)
    } else {
        String::new()
    }
}

/// `_legacy_terms`: the one economic-terms row of a spec without its own.
pub(super) fn legacy_terms(
    row: &Value,
    plan: &Value,
    term_kind: &str,
    source_hash: &str,
) -> RefreshResult<Value> {
    let legs = store::list(plan, "legs");
    let review = py_str(&value(row, "id"));
    let payout_amount = value(row, "payout_amount");
    Ok(Value::List(vec![dict([
        (
            "id",
            s(spec::stable(&[
                &text(plan, "component_id"),
                "term",
                term_kind,
                &review,
            ])),
        ),
        ("source_leg_id", value(&legs[0], "id")),
        ("target_leg_id", value(&legs[1], "id")),
        ("term_kind", s(term_kind)),
        ("legacy_source_id", s(&review)),
        ("source_row_hash", s(source_hash)),
        ("review_kind", s(py_str(&value(row, "kind")))),
        ("tax_policy", s(py_str(&value(row, "policy")))),
        (
            "reviewed_source_amount_msat",
            value(plan, "reviewed_source_amount_msat"),
        ),
        ("swap_fee_msat", value(row, "swap_fee_msat")),
        ("swap_fee_kind", value(row, "swap_fee_kind")),
        ("confidence_at_review", value(row, "confidence_at_pair")),
        ("review_source", value(row, "pair_source")),
        ("review_notes", value(row, "notes")),
        ("payout_asset", value(row, "payout_asset")),
        (
            "payout_amount_msat",
            if matches!(payout_amount, Value::Null) {
                Value::Null
            } else {
                big(python_int(&payout_amount)?)
            },
        ),
        ("payout_occurred_at", payout_occurred_at(row)?),
        (
            "payout_fiat_value_exact",
            canonical_float(&value(row, "payout_fiat_value"))?,
        ),
        ("payout_external_id", value(row, "payout_external_id")),
        ("counterparty", value(row, "counterparty")),
    ])]))
}

/// `_canonical_float`: a stored REAL as `format(value, ".17g")` (17
/// significant digits round-trip it), anything else as `str(Decimal(str(v)))`.
fn canonical_float(v: &Value) -> RefreshResult<Value> {
    match v {
        Value::Null => Ok(Value::Null),
        _ if *v == s("") => Ok(Value::Null),
        Value::Float(f) => Ok(s(format_g17(*f))),
        _ => {
            let text = py_str(v);
            let invalid = || Error::Python {
                family: "InvalidOperation",
                message: "[<class 'decimal.ConversionSyntax'>]".into(),
            };
            // Decimal() strips whitespace and takes `_` between digits.
            let text = kassiber_pytext::strip(&text);
            let chars: Vec<char> = text.chars().collect();
            for (i, c) in chars.iter().enumerate() {
                if *c == '_'
                    && !(i > 0
                        && i + 1 < chars.len()
                        && is_digit(chars[i - 1])
                        && is_digit(chars[i + 1]))
                {
                    return Err(invalid());
                }
            }
            let text = text.replace('_', "");
            if let Some(special) = special_decimal(&text) {
                return Ok(s(special));
            }
            let decimal: kassiber_pydecimal::Decimal = text.parse().map_err(|_| invalid())?;
            Ok(s(decimal.to_string()))
        }
    }
}

/// `str(Decimal(text))` for the non-finite spellings Decimal accepts:
/// `[sign](inf|infinity|nan[digits]|snan[digits])`, case-insensitive.
fn special_decimal(text: &str) -> Option<String> {
    let (sign, body) = match text.as_bytes().first() {
        Some(b'-') => ("-", &text[1..]),
        Some(b'+') => ("", &text[1..]),
        _ => ("", text),
    };
    let lower = body.to_ascii_lowercase();
    if lower == "inf" || lower == "infinity" {
        return Some(format!("{sign}Infinity"));
    }
    let (name, payload) = match lower.strip_prefix("snan") {
        Some(rest) => ("sNaN", rest),
        None => ("NaN", lower.strip_prefix("nan")?),
    };
    if !payload.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let payload = payload.trim_start_matches('0');
    Some(format!("{sign}{name}{payload}"))
}

/// CPython's `format(value, ".17g")`.
pub(super) fn format_g17(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf" } else { "inf" }.into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.into();
    }
    // 17 significant digits, correctly rounded, as d.dddddddddddddddde<exp>.
    let scientific = format!("{value:.16e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("exponent");
    let exponent: i32 = exponent.parse().expect("exponent digits");
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if negative { "-" } else { "" };
    if (-4..17).contains(&exponent) {
        let text = if exponent >= 0 {
            let split = exponent as usize + 1;
            format!("{}.{}", &digits[..split], &digits[split..])
        } else {
            format!("0.{}{}", "0".repeat((-exponent - 1) as usize), digits)
        };
        let text = text.trim_end_matches('0').trim_end_matches('.');
        format!("{sign}{text}")
    } else {
        let fraction = digits[1..].trim_end_matches('0');
        let mantissa = if fraction.is_empty() {
            digits[..1].to_owned()
        } else {
            format!("{}.{}", &digits[..1], fraction)
        };
        format!(
            "{sign}{mantissa}e{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.unsigned_abs()
        )
    }
}

fn validation(value: &Value) -> Error {
    Error::App(
        AppError::new(
            "validation",
            format!("Invalid occurred_at/date value '{}'", py_str(value)),
        )
        .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z"),
    )
}

/// `time_utils.parse_legacy_timestamp`: canonical UTC for the timestamp forms
/// older reviews stored verbatim, or the validation refusal.
pub(super) fn parse_legacy_timestamp(value: &Value) -> RefreshResult<String> {
    let raw = if truthy(value) {
        py_str(value)
    } else {
        String::new()
    };
    let raw = kassiber_pytext::strip(&raw).to_owned();
    let upper = kassiber_pytext::upper(&raw);
    let Some(parts) = legacy_match(&upper) else {
        return match components::timestamp(&s(&raw)) {
            Ok(text) => Ok(py_str(&text)),
            Err(error) => rfc2822(&raw).ok_or(Error::App(error)),
        };
    };
    // datetime.fromisoformat reads ASCII digits only.
    if [parts.year, parts.month, parts.day]
        .into_iter()
        .chain(
            [
                parts.hour,
                parts.minute,
                parts.second,
                parts.fraction,
                parts.zone,
            ]
            .into_iter()
            .flatten(),
        )
        .any(|part| !part.is_ascii())
    {
        return Err(validation(value));
    }
    let mut text = format!("{}-{}-{}", parts.year, parts.month, parts.day);
    if let Some(hour) = parts.hour {
        text += &format!(
            "T{hour}:{}:{}",
            parts.minute.unwrap_or("00"),
            parts.second.unwrap_or("00")
        );
        if let Some(fraction) = parts.fraction {
            let fraction: String = fraction.chars().take(6).collect();
            text += &format!(".{fraction:0<6}");
        }
        match parts.zone {
            Some("Z" | "UTC" | "GMT") => text += "+00:00",
            Some(zone) => {
                let digits = zone[1..].replace(':', "");
                let minutes = if digits.len() > 2 {
                    &digits[2..digits.len().min(4)]
                } else {
                    ""
                };
                text += &format!(
                    "{}{}:{}",
                    &zone[..1],
                    &digits[..2],
                    if minutes.is_empty() { "00" } else { minutes }
                );
            }
            None => {}
        }
    }
    components::timestamp(&s(&text))
        .map(|v| py_str(&v))
        .map_err(|_| validation(value))
}

struct LegacyParts<'a> {
    year: &'a str,
    month: &'a str,
    day: &'a str,
    hour: Option<&'a str>,
    minute: Option<&'a str>,
    second: Option<&'a str>,
    fraction: Option<&'a str>,
    zone: Option<&'a str>,
}

fn is_digit(c: char) -> bool {
    kassiber_pytext::decimal_value(c as u32).is_some()
}

/// `_LEGACY_TIMESTAMP.fullmatch`. Every optional part is taken greedily:
/// no shorter choice could let the remainder match.
fn legacy_match(text: &str) -> Option<LegacyParts<'_>> {
    let mut at = 0;
    let digits = |at: &mut usize, n: usize| -> Option<&str> {
        let start = *at;
        let mut end = start;
        for _ in 0..n {
            let c = text[end..].chars().next().filter(|c| is_digit(*c))?;
            end += c.len_utf8();
        }
        *at = end;
        Some(&text[start..end])
    };
    let optional = |at: &mut usize, c: char| {
        if text[*at..].starts_with(c) {
            *at += c.len_utf8();
        }
    };
    let year = digits(&mut at, 4)?;
    optional(&mut at, '-');
    let month = digits(&mut at, 2)?;
    optional(&mut at, '-');
    let day = digits(&mut at, 2)?;
    let mut parts = LegacyParts {
        year,
        month,
        day,
        hour: None,
        minute: None,
        second: None,
        fraction: None,
        zone: None,
    };
    if at == text.len() {
        return Some(parts);
    }
    if !(text[at..].starts_with('T') || text[at..].starts_with(' ')) {
        return None;
    }
    at += 1;
    parts.hour = Some(digits(&mut at, 2)?);
    let mut minute_at = at;
    optional(&mut minute_at, ':');
    if let Some(minute) = digits(&mut minute_at, 2) {
        at = minute_at;
        parts.minute = Some(minute);
        let mut second_at = at;
        optional(&mut second_at, ':');
        if let Some(second) = digits(&mut second_at, 2) {
            at = second_at;
            parts.second = Some(second);
            if text[at..].starts_with(['.', ',']) {
                let start = at + 1;
                let mut end = start;
                while let Some(c) = text[end..].chars().next().filter(|c| is_digit(*c)) {
                    end += c.len_utf8();
                }
                if end > start {
                    parts.fraction = Some(&text[start..end]);
                    at = end;
                }
            }
        }
    }
    while let Some(c) = text[at..]
        .chars()
        .next()
        .filter(|c| kassiber_pytext::is_space(*c as u32))
    {
        at += c.len_utf8();
    }
    let rest = &text[at..];
    if rest.is_empty() {
        return Some(parts);
    }
    if ["Z", "UTC", "GMT"].contains(&rest) {
        parts.zone = Some(rest);
        return Some(parts);
    }
    let mut zone_at = at;
    if !(rest.starts_with('+') || rest.starts_with('-')) {
        return None;
    }
    zone_at += 1;
    digits(&mut zone_at, 2)?;
    if zone_at < text.len() {
        let mut minutes_at = zone_at;
        optional(&mut minutes_at, ':');
        digits(&mut minutes_at, 2)?;
        zone_at = minutes_at;
    }
    if zone_at != text.len() {
        return None;
    }
    parts.zone = Some(rest);
    Some(parts)
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// `_parse_rfc2822`: `[Day, ]DD Mon YYYY HH:MM[:SS] zone` with a zone that
/// fixes the instant, as canonical UTC.
fn rfc2822(raw: &str) -> Option<String> {
    let upper = kassiber_pytext::upper(raw);
    let chars: Vec<char> = upper.chars().collect();
    let mut at = 0;
    let space = |c: char| kassiber_pytext::is_space(c as u32);
    let skip_spaces = |at: &mut usize, required: bool| -> bool {
        let start = *at;
        while *at < chars.len() && space(chars[*at]) {
            *at += 1;
        }
        !required || *at > start
    };
    let take_digits = |at: &mut usize, min: usize, max: usize| -> Option<u32> {
        let start = *at;
        let mut value = 0u32;
        while *at < chars.len() && *at - start < max {
            let Some(d) = kassiber_pytext::decimal_value(chars[*at] as u32) else {
                break;
            };
            value = value * 10 + d;
            *at += 1;
        }
        (*at - start >= min).then_some(value)
    };
    // Optional weekday followed by a comma.
    for day in ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"] {
        let candidate: Vec<char> = day.chars().collect();
        if chars.len() >= at + 4 && chars[at..at + 3] == candidate[..] && chars[at + 3] == ',' {
            at += 4;
            skip_spaces(&mut at, false);
            break;
        }
    }
    let day = take_digits(&mut at, 1, 2)?;
    if !skip_spaces(&mut at, true) {
        return None;
    }
    let month_text: String = chars.get(at..at + 3)?.iter().collect();
    let month = MONTHS.iter().position(|m| *m == month_text)? as u32 + 1;
    at += 3;
    if !skip_spaces(&mut at, true) {
        return None;
    }
    let year = take_digits(&mut at, 4, 4)?;
    if !skip_spaces(&mut at, true) {
        return None;
    }
    let hour = take_digits(&mut at, 2, 2)?;
    if chars.get(at) != Some(&':') {
        return None;
    }
    at += 1;
    let minute = take_digits(&mut at, 2, 2)?;
    let mut second = 0;
    if chars.get(at) == Some(&':') {
        let mut second_at = at + 1;
        if let Some(value) = take_digits(&mut second_at, 2, 2) {
            second = value;
            at = second_at;
        }
    }
    if !skip_spaces(&mut at, true) {
        return None;
    }
    let zone: String = chars[at..].iter().collect();
    let offset = match zone.as_str() {
        "GMT" | "UTC" | "UT" | "Z" => 0i64,
        _ => {
            let sign = match zone.chars().next()? {
                '+' => 1,
                '-' => -1,
                _ => return None,
            };
            let digits: Vec<u32> = zone
                .chars()
                .skip(1)
                .filter(|c| *c != ':')
                .map(|c| kassiber_pytext::decimal_value(c as u32))
                .collect::<Option<_>>()?;
            let colon = zone[1..].matches(':').count();
            if digits.len() != 4 || colon > 1 || (colon == 1 && zone.chars().nth(3) != Some(':')) {
                return None;
            }
            sign * i64::from((digits[0] * 10 + digits[1]) * 60 + digits[2] * 10 + digits[3])
        }
    };
    let leap = |y: u32| y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let lengths = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year == 0
        || day == 0
        || day > lengths[month as usize - 1]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    // Python's datetime with a fixed offset, astimezone(UTC).
    if offset.abs() >= 24 * 60 {
        return None;
    }
    let days =
        kassiber_pytext::zone::days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    let seconds =
        days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second)
            - offset * 60;
    let (y, m, d) = kassiber_pytext::zone::civil_from_days(seconds.div_euclid(86_400));
    if !(1..=9999).contains(&y) {
        return None;
    }
    let clock = seconds.rem_euclid(86_400);
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    ))
}

fn s(v: impl AsRef<str>) -> Value {
    kassiber_envelope::s(v.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_g17_matches_cpython() {
        for (value, expected) in [
            (123.456_789_012_345_67, "123.45678901234567"),
            (0.1, "0.10000000000000001"),
            (1e16, "10000000000000000"),
            (1e17, "1e+17"),
            (1.5e-5, "1.5e-05"),
            (6.666666666666666e-7, "6.666666666666666e-07"),
            (5e-324, "4.9406564584124654e-324"),
            (123456789012345678.0, "1.2345678901234568e+17"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (0.0001, "0.0001"),
            (-2.5, "-2.5"),
            (10.0, "10"),
            (1e300, "1.0000000000000001e+300"),
        ] {
            assert_eq!(format_g17(value), expected, "{value}");
        }
    }

    #[test]
    fn legacy_timestamps_canonicalize_like_python() {
        for (raw, expected) in [
            ("2025-03-01T09:00:30Z", "2025-03-01T09:00:30Z"),
            (" 2025-03-01t09:00:30z ", "2025-03-01T09:00:30Z"),
            ("20250301T090030Z", "2025-03-01T09:00:30Z"),
            ("2025-03-01 09:00:30 UTC", "2025-03-01T09:00:30Z"),
            ("2025-03-01T10:00:30+0100", "2025-03-01T09:00:30Z"),
            ("2025-03-01T10:00:30.123456789+01", "2025-03-01T09:00:30Z"),
            ("2025-03-01", "2025-03-01T00:00:00Z"),
            ("Sat, 01 Mar 2025 10:00:30 +0100", "2025-03-01T09:00:30Z"),
            ("1 Mar 2025 09:00 GMT", "2025-03-01T09:00:00Z"),
        ] {
            assert_eq!(parse_legacy_timestamp(&s(raw)).unwrap(), expected, "{raw}");
        }
        for raw in [
            "01.09.2025",
            "2025-13-01T00:00:00Z",
            "yesterday",
            "2025-03-01T09:00:30 CET",
        ] {
            let Err(Error::App(error)) = parse_legacy_timestamp(&s(raw)) else {
                panic!("{raw} must be refused");
            };
            assert_eq!(error.code, "validation", "{raw}");
        }
    }
}
