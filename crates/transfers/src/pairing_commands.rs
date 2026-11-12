//! Transport options and transaction ownership around the shared pairing service.
use super::*;
use kassiber_envelope::{set, truthy};

fn option(args: &Value, name: &str) -> Option<String> {
    get(args, name).filter(|v| truthy(v)).map(py_str)
}

fn bounded_integer(args: &Value, name: &str, default: i64) -> Result<i64> {
    let n = get(args, name)
        .map(integer)
        .transpose()?
        .unwrap_or(default.into());
    // These are comparison bounds, never quantities written to the book. A
    // bound beyond i64 covers every persisted amount and RFC3339 instant.
    Ok(i64::try_from(n.clone()).unwrap_or_else(|_| if n < 0.into() { i64::MIN } else { i64::MAX }))
}

fn float(value: &Value) -> Result<f64> {
    match value {
        Value::Float(n) => Ok(*n),
        Value::Int(n) => n
            .to_string()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| AppError::new("internal_error", "int too large to convert to float")),
        Value::Bool(n) => Ok(f64::from(u8::from(*n))),
        Value::Str(t) => {
            kassiber_pytext::parse_float(t.as_str().unwrap_or_default()).map_err(|_| {
                AppError::new(
                    "internal_error",
                    format!(
                        "could not convert string to float: {}",
                        kassiber_envelope::py_repr(value)
                    ),
                )
            })
        }
        v => Err(AppError::new(
            "internal_error",
            format!(
                "float() argument must be a string or a real number, not '{}'",
                kassiber_envelope::type_name(v)
            ),
        )),
    }
}

pub(super) fn execute(
    c: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let mut options = pairing::PairingOptions::default();
    options.matching.time_window_seconds = bounded_integer(args, "time_window_seconds", 86400)?;
    options.matching.time_window_seconds_overflow = get(args, "time_window_seconds")
        .map(integer)
        .transpose()?
        .is_some_and(|n| n.to_string().parse::<f64>().is_ok_and(|f| !f.is_finite()));
    options.matching.fee_pct_max = get(args, "fee_pct_max")
        .map(float)
        .transpose()?
        .unwrap_or(0.01);
    options.matching.fee_sats_min = bounded_integer(args, "fee_sats_min", 2500)?;
    options.matching.now = sources.now_precise(c)?;
    options.confidence = option(args, "confidence");
    options.asset_pair = option(args, "asset_pair");
    options.route_pair = option(args, "route_pair");
    options.method = option(args, "method");
    options.candidate_type = option(args, "candidate_type");
    options.raw_pair_filters = Some(project(args, &["asset_pair", "route_pair"]));
    options.authored_source = Some(option(args, "authored_source").unwrap_or_else(|| "cli".into()));
    if operation == "transfers.suggest" {
        return pairing::suggestions(c, scope, &options);
    }
    let run = || {
        if operation == "transfers.bulk-pair" {
            pairing::bulk_pair(c, scope, &options, None, sources)
        } else {
            pairing::apply_rules(c, scope, &options, None, sources)
        }
    };
    if get(args, "dry_run").is_some_and(truthy) {
        kassiber_store::begin_write_savepoint(c, "transfers_pairing_preview")
            .map_err(sql::error)?;
        let result = run();
        c.execute_batch("ROLLBACK TO SAVEPOINT transfers_pairing_preview; RELEASE SAVEPOINT transfers_pairing_preview").map_err(sql::error)?;
        let mut result = result?;
        set(&mut result, "dry_run", Value::Bool(true));
        Ok(result)
    } else {
        // The library leaves successful work pending for journal callers. This
        // service boundary commits only when it owns the outer transaction.
        sql::atomic(c, run)
    }
}
