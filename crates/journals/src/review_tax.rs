//! `review_workflow._tax_effects`: per-year tax figures of a review state as
//! the reports would read them. The state is stored inside a savepoint that is
//! always rolled back, so the report readers (Vienna tax years, the Kennzahl
//! mapping, report blockers) produce the figures and neither a preview nor the
//! comparison before an apply leaves a trace.
use crate::review_workflow::Error;
use crate::{persistence, report_blockers, JournalState};
use kassiber_custody::codec;
use kassiber_envelope::{dict, get, py_str, s, AppError};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{BigInt, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::review_delta;
use kassiber_transfers::review_support::sql;
use kassiber_transfers::{Scope, Sources};

pub(crate) const MAX_TAX_YEARS: usize = 10;
const OLD_RULES_BLOCKER: &str = "crypto_rows_before_2022_03";
const E1KV_FIRST_TAX_YEAR: i64 = 2022;

fn profile(conn: &Connection, scope: &Scope<'_>) -> Result<Value, Error> {
    Ok(sql::rows(
        conn,
        "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
        [scope.profile_id, scope.workspace_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review profile was not found"))?)
}

fn field(profile: &Value, key: &str) -> String {
    match get(profile, key) {
        Some(value) if kassiber_envelope::truthy(value) => {
            kassiber_pytext::strip(&py_str(value)).to_owned()
        }
        _ => String::new(),
    }
}

fn money(value: &Decimal) -> Result<Value, Error> {
    review_delta::money(value)
        .map(s)
        .map_err(|_| AppError::new("internal_error", "[<class 'decimal.InvalidOperation'>]").into())
}

/// The report blockers of the stored state, by the reports' own rules. They
/// are read for the active book; previewing another book withholds the
/// indicative figure instead of borrowing the active book's barriers.
fn report_blocker_ids(conn: &Connection, profile: &Value) -> Result<Vec<String>, Error> {
    use kassiber_workspace::readiness;
    let (context, active) = readiness::scope(conn, None)?;
    let health = readiness::health(conn, &context, active.as_ref())?;
    let coverage = kassiber_rates::coverage(conn, &dict([("limit", Value::Int(10.into()))]))?;
    let snapshot = report_blockers::snapshot(conn, health, coverage)?;
    let active = get(&snapshot, "health")
        .and_then(|health| get(health, "profile"))
        .cloned()
        .unwrap_or(Value::Null);
    if field(&active, "id") != py_str(get(profile, "id").unwrap_or(&Value::Null)) {
        return Ok(vec!["report_scope_not_active".into()]);
    }
    let mut ids: Vec<String> = match get(&snapshot, "blockers") {
        Some(Value::List(items)) => items
            .iter()
            .filter(|item| matches!(item, Value::Dict(_)))
            .map(|item| match get(item, "id") {
                Some(id) if kassiber_envelope::truthy(id) => py_str(id),
                _ => "report_blocker".into(),
            })
            .collect(),
        _ => Vec::new(),
    };
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// `tax_estimate.austrian_indicative_tax`, the fields a review keeps: the
/// 27.5 % Sondersteuersatz over KZ 172 + KZ 174 - KZ 176, withheld by any
/// report blocker and by rows dated before the crypto Kennzahlen.
fn indicative_tax(
    year: i64,
    amounts: &[(i64, Value)],
    blocker_ids: &[String],
    rows_before: usize,
) -> Result<Value, Error> {
    let cents = |codes: &[i64]| -> BigInt {
        amounts
            .iter()
            .filter(|(code, _)| codes.contains(code))
            .map(|(_, amount)| match amount {
                Value::Int(amount) => amount.clone(),
                _ => BigInt::default(),
            })
            .sum()
    };
    let base = cents(&[172, 174]) - cents(&[176]);
    let applies = year >= E1KV_FIRST_TAX_YEAR;
    let mut blockers: Vec<String> = blocker_ids.to_vec();
    if applies && rows_before > 0 {
        blockers.push(OLD_RULES_BLOCKER.into());
    }
    blockers.retain(|id| !id.is_empty());
    blockers.sort();
    blockers.dedup();
    let estimate = if applies && blockers.is_empty() {
        let positive = base.clone().max(BigInt::default());
        let product = positive
            .to_string()
            .parse::<Decimal>()
            .and_then(|base| {
                base.mul(
                    &"0.275".parse().expect("the special rate"),
                    &Context::new(32, Rounding::HalfEven),
                )
            })
            .and_then(|tax| tax.quantize(0, &Context::new(32, Rounding::HalfUp)))
            .and_then(|tax| tax.to_fixed_places(0, Rounding::Down))
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
        Value::Int(
            product
                .parse()
                .map_err(|_| AppError::new("internal_error", "invalid estimate"))?,
        )
    } else {
        Value::Null
    };
    let status = if !applies {
        "not_applicable"
    } else if !blockers.is_empty() {
        "withheld"
    } else {
        "indicative"
    };
    Ok(dict([
        ("status", s(status)),
        (
            "special_rate_base_eur_cents",
            if applies {
                Value::Int(base)
            } else {
                Value::Null
            },
        ),
        ("estimated_tax_eur_cents", estimate),
        (
            "withheld_by",
            Value::List(blockers.into_iter().map(s).collect()),
        ),
    ]))
}

fn years(conn: &Connection, current: &Value, austrian: bool) -> Result<(Vec<Value>, usize), Error> {
    let id = py_str(get(current, "id").unwrap_or(&Value::Null));
    let vienna = field(current, "tax_country").to_lowercase() == "at";
    let totals = kassiber_reports::review_figures::year_totals(conn, &id, vienna)?;
    let skip = totals.len().saturating_sub(MAX_TAX_YEARS);
    let blocker_ids = if austrian && totals.len() > skip {
        report_blocker_ids(conn, current)?
    } else {
        Vec::new()
    };
    let mut result = Vec::new();
    for (year, [realized, income]) in totals.iter().skip(skip) {
        let mut item = dict([
            ("year", Value::Int((*year).into())),
            ("realized_gain_loss", money(realized)?),
            ("income", money(income)?),
        ]);
        if austrian && (2009..=2100).contains(year) {
            let (amounts, before) =
                kassiber_reports::review_figures::kennzahl_summary(conn, current, *year)?;
            let estimate = indicative_tax(*year, &amounts, &blocker_ids, before)?;
            let mut kennzahlen = Vec::new();
            for (code, amount) in &amounts {
                if kassiber_envelope::truthy(amount) {
                    kennzahlen.push((code.to_string(), amount.clone()));
                }
            }
            kassiber_envelope::set(&mut item, "kennzahlen", dict(kennzahlen));
            kassiber_envelope::set(&mut item, "indicative_tax", estimate);
        }
        result.push(item);
    }
    Ok((result, skip))
}

/// Per-year figures of `state`: realized gain/loss and income and, for an
/// Austrian EUR book, its Kennzahlen and the indicative tax.
pub(crate) fn tax_effects(
    conn: &Connection,
    scope: &Scope<'_>,
    state: &JournalState<'_>,
    sources: &dyn Sources,
) -> Result<Value, Error> {
    let current = profile(conn, scope)?;
    let country = field(&current, "tax_country").to_lowercase();
    let currency = field(&current, "fiat_currency").to_uppercase();
    let austrian = country == "at" && currency == "EUR";
    conn.execute_batch("SAVEPOINT review_tax_preview")
        .map_err(sql::error)?;
    let result = (|| {
        persistence::store_ledger_state(
            conn,
            &codec::to_json(&current),
            state,
            &sources.now(conn)?,
            &mut || sources.uuid(conn).map_err(crate::Refusal::from),
            &mut (),
        )?;
        let stored = profile(conn, scope)?;
        years(conn, &stored, austrian)
    })();
    conn.execute_batch(
        "ROLLBACK TO SAVEPOINT review_tax_preview; RELEASE SAVEPOINT review_tax_preview",
    )
    .map_err(sql::error)?;
    let (years, omitted) = result?;
    Ok(dict([
        ("fiat_currency", s(currency)),
        ("jurisdiction", if austrian { s("AT") } else { Value::Null }),
        ("years", Value::List(years)),
        ("years_omitted", omitted.into()),
    ]))
}
