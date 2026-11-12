//! R4-14 refresh coordination. Pairing never owns the final commit.
use super::*;
use kassiber_transfers::{pairing, Scope, Sources};

#[derive(Clone, Copy)]
pub struct RefreshOptions {
    pub auto_pair: bool,
    pub skip_rebuild_when_current: bool,
    pub decimal_context: kassiber_pydecimal::Context,
}
impl Default for RefreshOptions {
    fn default() -> Self {
        Self {
            auto_pair: false,
            skip_rebuild_when_current: false,
            decimal_context: kassiber_pydecimal::Context::RP2,
        }
    }
}

/// Refresh a resolved profile. The resolver is invoked again under the writer
/// for every actual build. A stale projection commits before pairing; pairing
/// and a subsequent rebuild share one transaction, including the candidate cache.
#[allow(clippy::too_many_arguments)]
pub fn refresh_native<'a>(
    conn: &'a Connection,
    scope: &Scope<'_>,
    resolve: impl FnMut(&Connection) -> Result<Value, Refusal>,
    sources: &impl Sources,
    options: RefreshOptions,
    phase: &mut impl FnMut(&'static str),
    journal_progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
) -> Result<Committed<'a>, Refusal> {
    refresh_native_checked(
        conn,
        scope,
        resolve,
        sources,
        options,
        &mut |value| {
            phase(value);
            Ok(())
        },
        journal_progress,
        writes,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn refresh_native_checked<'a>(
    conn: &'a Connection,
    scope: &Scope<'_>,
    resolve: impl FnMut(&Connection) -> Result<Value, Refusal>,
    sources: &impl Sources,
    options: RefreshOptions,
    phase: &mut impl FnMut(&'static str) -> Result<(), Refusal>,
    journal_progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
) -> Result<Committed<'a>, Refusal> {
    let committed = refresh(
        conn,
        scope.profile_id,
        &mut NativePrebuild {
            resolve,
            sources,
            decimal_context: options.decimal_context,
        },
        &mut || {
            pairing::auto_pair_before_journals_captured(conn, scope, sources).map_err(|error| {
                match error {
                    pairing::AutomaticPairingError::App(error) => prebuild::app(error),
                    pairing::AutomaticPairingError::Sqlite(error) => error.into(),
                    pairing::AutomaticPairingError::Python { family, message } => {
                        Refusal::new(family, message)
                    }
                }
            })
        },
        Clocks {
            build_clock: &mut || sources.now_precise(conn).map_err(prebuild::app),
            now: &mut || sources.now(conn).map_err(prebuild::app),
            ids: &mut || sources.uuid(conn).map_err(prebuild::app),
        },
        options,
        Progress {
            phase,
            journal: journal_progress,
        },
        writes,
    )?;
    // Counted on the rebuilt projection: the pairing pass counted before its
    // own pairs were booked, and a run without it still follows the owner's
    // manual pairs and dismissals.
    let paired = kassiber_envelope::get(&committed.ordered_payload, "auto_pair").map(|summary| {
        kassiber_envelope::get(summary, "applied").is_some_and(kassiber_envelope::truthy)
    });
    if paired.unwrap_or(true) {
        pairing::recount_candidates(conn, scope, sources);
    }
    Ok(committed)
}

struct Progress<'a, P, J> {
    phase: &'a mut P,
    journal: &'a mut J,
}

// Pairing is injected only at this private seam for failure/rollback witnesses.
#[allow(clippy::too_many_arguments)]
fn refresh<'a>(
    conn: &'a Connection,
    profile_id: &str,
    prebuild: &mut impl Prebuild,
    pair: &mut impl FnMut() -> Result<kassiber_pyjson::Value, Refusal>,
    mut clocks: Clocks<'_>,
    options: RefreshOptions,
    progress: Progress<
        '_,
        impl FnMut(&'static str) -> Result<(), Refusal>,
        impl FnMut(&'static str),
    >,
    writes: &mut impl storage::WriteObserver,
) -> Result<Committed<'a>, Refusal> {
    require_idle(conn)?;
    let mut checkpoint = storage::DeferredCheckpoint::enter(conn)?;
    let mut payload = None;
    let mut ordered_payload = None;
    if options.auto_pair && !is_current(conn, profile_id)? {
        let first = build_in_transaction(
            conn,
            prebuild,
            reborrow(&mut clocks),
            progress.journal,
            writes,
            checkpoint,
            true,
        )?;
        payload = Some(first.payload);
        ordered_payload = Some(first.ordered_payload);
        checkpoint = first.checkpoint;
    }

    // This guard also covers callbacks and panics outside the rebuild itself.
    // A completed first build remains committed when the second part fails.
    let _transaction = Transaction(conn);
    let summary = if options.auto_pair {
        // Freshness phase observers may commit; invoke them only while idle.
        (progress.phase)("auto_pair")?;
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Some(match pair() {
            Ok(summary) => summary,
            Err(error) => {
                conn.execute_batch("ROLLBACK")?;
                skipped(&error)
            }
        })
    } else {
        None
    };
    writes.check()?;
    let pairs_applied = summary
        .as_ref()
        .and_then(|s| kassiber_envelope::get(s, "applied"))
        .is_some_and(kassiber_envelope::truthy);
    if payload.is_some() && !pairs_applied && is_current(conn, profile_id)? {
        commit_if_pending(conn)?;
    } else if payload.is_none()
        && options.skip_rebuild_when_current
        && !pairs_applied
        && is_current(conn, profile_id)?
    {
        commit_if_pending(conn)?;
        let quarantined: i64 = conn.query_row(
            "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id=?",
            [profile_id],
            |row| row.get(0),
        )?;
        payload = Some(json!({"rebuilt":false,"quarantined":quarantined}));
        ordered_payload = Some(kassiber_envelope::dict([
            ("rebuilt", kassiber_pyjson::Value::Bool(false)),
            (
                "quarantined",
                kassiber_pyjson::Value::Int(quarantined.into()),
            ),
        ]));
    } else {
        let reserve_writer = conn.is_autocommit();
        let rebuilt = build_in_transaction(
            conn,
            prebuild,
            clocks,
            progress.journal,
            writes,
            checkpoint,
            reserve_writer,
        )?;
        payload = Some(rebuilt.payload);
        ordered_payload = Some(rebuilt.ordered_payload);
        checkpoint = rebuilt.checkpoint;
    }
    let mut payload = payload.expect("a retained or rebuilt projection");
    let mut ordered_payload = ordered_payload.expect("a retained or rebuilt projection");
    if payload.get("rebuilt").is_none() {
        payload["rebuilt"] = json!(true);
        kassiber_envelope::set(
            &mut ordered_payload,
            "rebuilt",
            kassiber_pyjson::Value::Bool(true),
        );
    }
    if let Some(summary) = summary {
        payload["auto_pair"] = codec::to_json(&summary);
        kassiber_envelope::set(&mut ordered_payload, "auto_pair", summary);
    }
    Ok(Committed {
        payload,
        ordered_payload,
        checkpoint,
    })
}

fn reborrow<'a>(clocks: &'a mut Clocks<'_>) -> Clocks<'a> {
    Clocks {
        build_clock: &mut *clocks.build_clock,
        now: &mut *clocks.now,
        ids: &mut *clocks.ids,
    }
}

fn commit_if_pending(conn: &Connection) -> Result<(), Refusal> {
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT")?;
    }
    Ok(())
}

fn skipped(error: &Refusal) -> kassiber_pyjson::Value {
    use kassiber_envelope::{dict, s};
    use kassiber_pyjson::Value as Py;
    let application = error.family == "AppError";
    dict([
        ("enabled", Py::Bool(true)),
        ("applied", Py::Int(0.into())),
        ("rules_applied", Py::Int(0.into())),
        ("bulk_exact_applied", Py::Int(0.into())),
        ("skipped", Py::Bool(true)),
        (
            "error",
            dict([
                (
                    "code",
                    s(if application {
                        error.code.as_deref().unwrap_or("app_error")
                    } else {
                        "auto_pair_failed"
                    }),
                ),
                (
                    "message",
                    s("Automatic pairing was skipped; journals were still processed."),
                ),
                (
                    "retryable",
                    Py::Bool(if application {
                        error.retryable.unwrap_or(false)
                    } else {
                        true
                    }),
                ),
            ]),
        ),
    ])
}

fn is_current(conn: &Connection, profile_id: &str) -> Result<bool, Refusal> {
    use kassiber_store::rusqlite::{types::Value as Sql, OptionalExtension};
    let row = conn.query_row(
        "SELECT last_processed_at,last_processed_tx_count,journal_input_version,last_processed_input_version,(SELECT COUNT(*) FROM transactions WHERE profile_id=p.id AND excluded=0),required_journal_semantics_version,journal_semantics_version FROM profiles p WHERE id=?",
        [profile_id],
        |row| Ok((row.get::<_, Sql>(0)?,row.get::<_, Sql>(1)?,row.get::<_, Sql>(2)?,row.get::<_, Sql>(3)?,row.get::<_, i64>(4)?,row.get::<_, Sql>(5)?,row.get::<_, Sql>(6)?)),
    ).optional()?;
    let Some((at, count, input, processed, active, required, built)) = row else {
        return Ok(false);
    };
    let scalar = |value: Sql| match value {
        Sql::Null => Value::Null,
        Sql::Text(s) => Value::String(s),
        Sql::Integer(n) => json!(n),
        Sql::Real(n) => json!(n),
        Sql::Blob(b) => codec::to_json(&kassiber_pyjson::Value::Bytes(b)),
    };
    // Python coerces all marker integers before evaluating its truth expression.
    let count = crate::integer::integer_or_zero(&scalar(count))?;
    let input = crate::integer::integer_or_zero(&scalar(input))?;
    let processed = crate::integer::integer_or_zero(&scalar(processed))?;
    // Python compares unbounded integers: zero requires nothing, otherwise
    // the built version must be the current one and reach the requirement.
    let required = crate::integer::integer_or_zero(&scalar(required))?;
    let built = crate::integer::integer_or_zero(&scalar(built))?;
    let version = kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION;
    let current_semantics = required == json!(0)
        || (built == json!(version)
            && required
                .as_i64()
                .map_or(required.to_string().starts_with('-'), |required| {
                    required <= version
                }));
    Ok(crate::truth(&scalar(at))
        && count == json!(active)
        && input == processed
        && current_semantics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_pairing_retains_application_codes_even_for_internal_errors() {
        for (family, code, retryable, expected_code, expected_retryable) in [
            ("AppError", "validation", false, "validation", false),
            ("AppError", "internal_error", false, "internal_error", false),
            ("AppError", "internal_error", true, "internal_error", true),
            ("OperationalError", "", false, "auto_pair_failed", true),
        ] {
            let mut refusal = Refusal::new(family, "private error text");
            refusal.code = Some(code.into());
            refusal.retryable = Some(retryable);
            let result = skipped(&refusal);
            assert_eq!(
                kassiber_pyjson::dumps(&result, &Default::default()).unwrap(),
                format!(
                    "{{\"enabled\": true, \"applied\": 0, \"rules_applied\": 0, \"bulk_exact_applied\": 0, \"skipped\": true, \"error\": {{\"code\": \"{expected_code}\", \"message\": \"Automatic pairing was skipped; journals were still processed.\", \"retryable\": {expected_retryable}}}}}"
                ),
            );
        }
    }
}
