//! `review_workflow._tax_effects`: per-year tax figures of a review state as
//! the reports would read them. The state is stored inside a savepoint that is
//! always rolled back, so the report readers (Vienna tax years, the Kennzahl
//! mapping, report blockers) produce the figures and neither a preview nor the
//! comparison before an apply leaves a trace.
use crate::review_workflow::Error;
use crate::{persistence, report_blockers, JournalState};
use kassiber_custody::codec;
use kassiber_envelope::{dict, get, py_str, s, AppError};
use kassiber_pydecimal::Decimal;
use kassiber_pyjson::{BigInt, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::review_delta;
use kassiber_transfers::review_support::sql;
use kassiber_transfers::{Scope, Sources};

pub(crate) const MAX_TAX_YEARS: usize = 10;

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

/// `tax_estimate.austrian_indicative_tax`, the fields a review keeps.
fn indicative_tax(
    year: i64,
    amounts: &[(i64, BigInt)],
    blocker_ids: &[String],
    rows_before: i64,
) -> Value {
    let estimate = kassiber_reports::tax_estimate::austrian_indicative_tax(
        year,
        amounts,
        blocker_ids,
        rows_before,
    );
    dict(
        [
            "status",
            "special_rate_base_eur_cents",
            "estimated_tax_eur_cents",
            "withheld_by",
        ]
        .map(|key| (key, get(&estimate, key).cloned().unwrap_or(Value::Null))),
    )
}

fn years(conn: &Connection, current: &Value, austrian: bool) -> Result<(Vec<Value>, usize), Error> {
    let id = py_str(get(current, "id").unwrap_or(&Value::Null));
    let zone = kassiber_reports::country::zone(&field(current, "tax_country").to_lowercase());
    let totals = kassiber_reports::review_figures::year_totals(conn, &id, zone)?;
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
            let estimate = indicative_tax(*year, &amounts, &blocker_ids, before);
            let mut kennzahlen = Vec::new();
            for (code, amount) in &amounts {
                if *amount != BigInt::default() {
                    kennzahlen.push((code.to_string(), Value::Int(amount.clone())));
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
    // Austria's Kennzahlen and indicative tax need its EUR figures.
    let austrian = kassiber_tax_countries::lookup(&country)
        .is_some_and(|entry| entry.rules == kassiber_tax_countries::Rules::Austria)
        && currency == "EUR";
    kassiber_store::begin_write_savepoint(conn, "review_tax_preview").map_err(sql::error)?;
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
