use kassiber_journals::{process::Committed, Refusal};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

#[cfg(not(feature = "test-harness"))]
pub(super) fn process<'a>(
    conn: &'a Connection,
    workspace: Option<&Value>,
    profile: Option<&Value>,
    auto_pair: bool,
    progress: &mut impl FnMut(&'static str),
) -> Result<Committed<'a>, Refusal> {
    if auto_pair {
        return kassiber_journals::service::refresh(
            conn,
            workspace,
            profile,
            kassiber_journals::process::RefreshOptions {
                auto_pair,
                ..Default::default()
            },
            progress,
        );
    }
    kassiber_journals::service::process(conn, workspace, profile, progress)
}

#[cfg(feature = "test-harness")]
pub(super) fn process<'a>(
    conn: &'a Connection,
    workspace: Option<&Value>,
    profile: Option<&Value>,
    auto_pair: bool,
    progress: &mut impl FnMut(&'static str),
) -> Result<Committed<'a>, Refusal> {
    if std::env::var("KASSIBER_RUST_TEST_JOURNAL_FAIL_PAIRED_STORE").as_deref() == Ok("1") {
        conn.execute_batch("CREATE TEMP TRIGGER IF NOT EXISTS refresh_failure BEFORE UPDATE OF last_processed_at ON profiles WHEN NEW.last_processed_at IS NOT NULL AND EXISTS(SELECT 1 FROM custody_component_economic_terms WHERE term_kind='transaction_pair') BEGIN SELECT RAISE(ABORT,'second rebuild failed'); END")?;
    }
    let observed = std::cell::Cell::new(0);
    let mut progress = |phase| {
        if phase == "complete"
            && std::env::var("KASSIBER_RUST_TEST_JOURNAL_TRACE_WRITES").as_deref() == Ok("1")
        {
            eprintln!(
                "{}",
                serde_json::json!({"phase":phase,"writes":observed.get()})
            );
        }
        progress(phase);
    };
    if auto_pair {
        return kassiber_journals::service::refresh_with_sources(
            conn,
            workspace,
            profile,
            &Sources,
            kassiber_journals::process::RefreshOptions {
                auto_pair,
                ..Default::default()
            },
            &mut progress,
            &mut Writes(&observed),
        );
    }
    kassiber_journals::service::process_with_sources(
        conn,
        workspace,
        profile,
        &Sources,
        &mut progress,
        &mut Writes(&observed),
    )
}

#[cfg(feature = "test-harness")]
pub(super) struct Sources;
#[cfg(feature = "test-harness")]
impl kassiber_transfers::Sources for Sources {
    fn uuid(&self, conn: &Connection) -> Result<String, kassiber_envelope::AppError> {
        if std::env::var("KASSIBER_RUST_TEST_JOURNAL_IDS").as_deref() == Ok("sequential") {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(1);
            return Ok(format!(
                "00000000-0000-4000-8000-{:012x}",
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        }
        kassiber_transfers::SystemSources.uuid(conn)
    }
    fn now(&self, conn: &Connection) -> Result<String, kassiber_envelope::AppError> {
        std::env::var("KASSIBER_RUST_TEST_JOURNAL_TIME")
            .map_or_else(|_| kassiber_transfers::SystemSources.now(conn), Ok)
    }
    fn now_precise(&self, conn: &Connection) -> Result<String, kassiber_envelope::AppError> {
        std::env::var("KASSIBER_RUST_TEST_JOURNAL_TIME")
            .map_or_else(|_| kassiber_transfers::SystemSources.now_precise(conn), Ok)
    }
}

#[cfg(feature = "test-harness")]
struct Writes<'a>(&'a std::cell::Cell<usize>);
#[cfg(feature = "test-harness")]
impl kassiber_journals::storage::WriteObserver for Writes<'_> {
    fn wrote(&mut self, table: &'static str) -> kassiber_store::rusqlite::Result<()> {
        self.0.set(self.0.get() + 1);
        let selected = |name| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                == Some(self.0.get())
        };
        if selected("KASSIBER_RUST_TEST_JOURNAL_FAIL_AFTER_WRITE") {
            return Err(kassiber_store::rusqlite::Error::ExecuteReturnedResults);
        }
        if selected("KASSIBER_RUST_TEST_JOURNAL_PAUSE_AFTER_WRITE") {
            use std::io::Write;
            println!(
                "{}",
                serde_json::json!({"paused_at": self.0.get(), "table": table})
            );
            std::io::stdout().flush().expect("journal test kill point");
            loop {
                std::thread::park();
            }
        }
        Ok(())
    }
}

#[cfg(feature = "test-harness")]
struct RefreshWrites<'a> {
    writes: Writes<'a>,
    committed: &'a std::cell::Cell<bool>,
}
#[cfg(feature = "test-harness")]
impl kassiber_journals::storage::WriteObserver for RefreshWrites<'_> {
    fn wrote(&mut self, table: &'static str) -> kassiber_store::rusqlite::Result<()> {
        if self.committed.get()
            && std::env::var("KASSIBER_RUST_TEST_JOURNAL_FAIL_SECOND_BUILD").as_deref() == Ok("1")
        {
            return Err(kassiber_store::rusqlite::Error::ExecuteReturnedResults);
        }
        self.writes.wrote(table)
    }
}

pub(super) struct Refresh<'a> {
    pub cancel: &'a kassiber_wallets::scan::Cancellation,
    pub consent: Option<&'a kassiber_wallets::freshness::native::Consent>,
    pub context: Value,
    pub deadline: std::time::Instant,
    #[cfg(feature = "test-harness")]
    pub test_passphrase: Option<&'a str>,
}

impl Refresh<'_> {
    fn check(&self, conn: &Connection) -> Result<(), kassiber_envelope::AppError> {
        kassiber_wallets::scan::Control::check(self.cancel, self.deadline)?;
        if let Some(consent) = self.consent {
            consent.check()?;
        }
        if kassiber_setup::context::snapshot(conn)? != self.context {
            return Err(kassiber_envelope::AppError::new(
                "stale_context",
                "Freshness selection changed",
            ));
        }
        Ok(())
    }
}

struct CheckedWrites<'a, W, C> {
    inner: W,
    check: C,
    _cancel: &'a kassiber_wallets::scan::Cancellation,
}
impl<
        W: kassiber_journals::storage::WriteObserver,
        C: Fn() -> Result<(), kassiber_envelope::AppError>,
    > kassiber_journals::storage::WriteObserver for CheckedWrites<'_, W, C>
{
    fn phase(&mut self, phase: &'static str) {
        self.inner.phase(phase);
        #[cfg(feature = "test-harness")]
        if phase == "complete"
            && std::env::var("KASSIBER_RUST_TEST_JOURNAL_CANCEL_BEFORE_COMMIT").as_deref()
                == Ok("1")
        {
            self._cancel.cancel();
        }
    }
    fn check(&mut self) -> Result<(), Refusal> {
        (self.check)().map_err(Into::into)
    }
    fn wrote(&mut self, table: &'static str) -> kassiber_store::rusqlite::Result<()> {
        if (self.check)().is_err() {
            return Err(kassiber_store::rusqlite::Error::ExecuteReturnedResults);
        }
        self.inner.wrote(table)
    }
}

pub(super) fn explicit<'a>(
    conn: &'a Connection,
    context: &kassiber_runtime::store::BookContext,
    auto_pair: bool,
    cancel: &kassiber_wallets::scan::Cancellation,
    progress: &mut dyn FnMut(&Value) -> Result<(), kassiber_envelope::AppError>,
) -> Result<Committed<'a>, kassiber_envelope::AppError> {
    use kassiber_envelope::{dict, get, py_str, s};
    let control = Refresh {
        cancel,
        consent: None,
        context: dict([
            ("workspace_id", s(context.workspace_id.as_str())),
            ("workspace_label", s(context.workspace_label.as_str())),
            ("profile_id", s(context.profile_id.as_str())),
            ("profile_label", s(context.profile_label.as_str())),
        ]),
        // Python's direct desktop action has no time limit; only cancellation,
        // retirement and a context change stop it.
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(u64::from(u32::MAX)),
        #[cfg(feature = "test-harness")]
        test_passphrase: None,
    };
    let output_error = std::cell::RefCell::new(None);
    let check = || {
        if let Some(error) = output_error.borrow().clone() {
            return Err(error);
        }
        control.check(conn)
    };
    check()?;
    #[cfg(feature = "test-harness")]
    if std::env::var("KASSIBER_RUST_TEST_JOURNAL_FAIL_PAIRED_STORE").as_deref() == Ok("1") {
        conn.execute_batch("CREATE TEMP TRIGGER IF NOT EXISTS refresh_failure BEFORE UPDATE OF last_processed_at ON profiles WHEN NEW.last_processed_at IS NOT NULL AND EXISTS(SELECT 1 FROM custody_component_economic_terms WHERE term_kind='transaction_pair') BEGIN SELECT RAISE(ABORT,'second rebuild failed'); END")
            .map_err(|error| kassiber_journals::service::application_error(error.into()))?;
    }
    #[cfg(feature = "test-harness")]
    let sources = Sources;
    #[cfg(not(feature = "test-harness"))]
    let sources = kassiber_transfers::SystemSources;
    #[cfg(feature = "test-harness")]
    let observed = std::cell::Cell::new(0);
    #[cfg(feature = "test-harness")]
    let inner = Writes(&observed);
    #[cfg(not(feature = "test-harness"))]
    let inner = ();
    let mut writes = CheckedWrites {
        inner,
        check,
        _cancel: cancel,
    };
    let profile = py_str(get(&control.context, "profile_id").unwrap_or(&Value::Null));
    let mut emit = |phase| {
        if phase == "complete" || check().is_ok() {
            if let Err(error) = progress(&dict([("phase", s(phase))])) {
                *output_error.borrow_mut() = Some(error);
            }
        }
        // Hold the worker at the named phase until the request is cancelled
        // or the test creates the release file. A fixed pause let a test
        // process slowed by machine load act after the worker had moved on.
        #[cfg(feature = "test-harness")]
        if std::env::var("KASSIBER_RUST_TEST_JOURNAL_WAIT_PHASE").as_deref() == Ok(phase) {
            let release = std::env::var_os("KASSIBER_RUST_TEST_JOURNAL_WAIT_RELEASE")
                .map(std::path::PathBuf::from);
            while check().is_ok() && !release.as_deref().is_some_and(std::path::Path::exists) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    };
    let emit = std::cell::RefCell::new(&mut emit);
    let result = if profile.is_empty() {
        kassiber_journals::service::process_with_sources(
            conn,
            None,
            None,
            &sources,
            &mut |phase| (emit.borrow_mut())(phase),
            &mut writes,
        )
    } else {
        kassiber_journals::service::refresh_with_sources_checked(
            conn,
            &profile,
            &sources,
            kassiber_journals::process::RefreshOptions {
                auto_pair,
                ..Default::default()
            },
            &mut |phase| {
                check()?;
                (emit.borrow_mut())(phase);
                check().map_err(Into::into)
            },
            &mut |phase| (emit.borrow_mut())(phase),
            &mut writes,
        )
    };
    match result {
        Ok(result) => Ok(result),
        Err(error) => {
            check()?;
            Err(kassiber_journals::service::application_error(error))
        }
    }
}

impl kassiber_wallets::freshness::execution::Handler for Refresh<'_> {
    fn supports(&self, kind: &str) -> bool {
        kind == "journal_refresh"
    }

    fn run(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut kassiber_wallets::freshness::execution::Progress<'_>,
    ) -> Result<Value, kassiber_wallets::freshness::execution::Failure> {
        use kassiber_envelope::{dict, get, py_str, s, set, truthy};
        use kassiber_wallets::freshness::execution::Failure;
        self.check(conn)?;
        progress.emit(conn, &dict([("phase", s("journal_refresh"))]))?;
        self.check(conn)?;
        #[cfg(feature = "test-harness")]
        if let Ok(wait) = std::env::var("KASSIBER_RUST_TEST_JOURNAL_WAIT_MS") {
            let until = std::time::Instant::now()
                + std::time::Duration::from_millis(wait.parse::<u64>().unwrap_or(0).min(5000));
            while std::time::Instant::now() < until {
                self.check(conn)?;
                progress.check(conn)?;
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let payload = get(job, "payload").unwrap_or(&Value::Null);
        let profile = py_str(get(job, "profile_id").unwrap_or(&Value::Null));
        #[cfg(feature = "test-harness")]
        let sources = Sources;
        #[cfg(not(feature = "test-harness"))]
        let sources = kassiber_transfers::SystemSources;
        #[cfg(feature = "test-harness")]
        let guard = kassiber_journals::service::ReplayGuard::capture(conn);
        let last_phase = std::cell::Cell::new("admission");
        let paired_phase = std::cell::Cell::new(false);
        #[cfg(feature = "test-harness")]
        let committed = std::cell::Cell::new(false);
        #[cfg(feature = "test-harness")]
        let observed = std::cell::Cell::new(0);
        #[cfg(feature = "test-harness")]
        let mut writes = RefreshWrites {
            writes: Writes(&observed),
            committed: &committed,
        };
        #[cfg(not(feature = "test-harness"))]
        let mut writes = ();
        let result = kassiber_journals::service::refresh_with_sources_checked(
            conn,
            &profile,
            &sources,
            kassiber_journals::process::RefreshOptions {
                auto_pair: get(payload, "auto_pair").is_some_and(truthy),
                skip_rebuild_when_current: get(payload, "skip_rebuild_when_current")
                    .is_some_and(truthy),
                ..Default::default()
            },
            &mut |phase| {
                self.check(conn)?;
                progress.emit(conn, &dict([("phase", s(phase))]))?;
                paired_phase.set(phase == "auto_pair");
                last_phase.set(phase);
                self.check(conn)?;
                Ok(())
            },
            &mut |phase| {
                last_phase.set(phase);
                #[cfg(feature = "test-harness")]
                if phase == "complete" {
                    committed.set(true);
                }
            },
            &mut writes,
        )
        .map_err(|error| {
            #[cfg(feature = "test-harness")]
            {
                let replay_safe = last_phase.get() != "complete" && guard.unchanged(conn);
                hybrid::record(&error, last_phase.get(), replay_safe);
            }
            if error.family == "AppError" {
                Failure::App(kassiber_journals::service::application_error(error))
            } else if let Some(code) = error.sqlite_code {
                Failure::Database(kassiber_store::rusqlite::Error::SqliteFailure(
                    kassiber_store::rusqlite::ffi::Error::new(code),
                    Some(error.message.clone()),
                ))
            } else {
                let module = if [
                    "DatabaseError",
                    "OperationalError",
                    "IntegrityError",
                    "ProgrammingError",
                ]
                .contains(&error.family.as_str())
                {
                    "sqlite3"
                } else {
                    "builtins"
                };
                Failure::Unexpected {
                    message: error.message.clone(),
                    class: format!("{module}.{}", error.family),
                }
            }
        });
        #[cfg(feature = "test-harness")]
        let result = match result {
            Err(error)
                if last_phase.get() != "complete"
                    && guard.unchanged(conn)
                    && hybrid::enabled()
                    && hybrid::replayable(&error) =>
            {
                self.check(conn)?;
                progress.check(conn)?;
                return hybrid::replay(self, conn, job, progress, paired_phase.get());
            }
            result => result,
        };
        let result = result?;
        let mut payload = result.ordered_payload.clone();
        drop(result);
        set(&mut payload, "status", s("synced"));
        Ok(payload)
    }
}

#[cfg(feature = "test-harness")]
mod hybrid;

/// `journals quarantine groups` (`handlers.quarantine_groups`): the scope
/// first, then the `--limit` bounds, then the rollup or one group's page.
#[allow(clippy::type_complexity)]
pub(super) fn quarantine_groups_command(
    conn: &Connection,
    args: &Value,
    scope: &dyn Fn(
        Option<&Value>,
        Option<&Value>,
    ) -> Result<(String, String), kassiber_envelope::AppError>,
) -> Result<Value, kassiber_envelope::AppError> {
    use kassiber_envelope::{get, AppError};
    use kassiber_journals::quarantine::groups;
    let (_, profile_id) = scope(get(args, "workspace"), get(args, "profile"))?;
    let group_id = match get(args, "group_id") {
        Some(Value::Str(text)) => Some(kassiber_envelope::py_str(&Value::Str(text.clone()))),
        _ => None,
    };
    let cursor = match get(args, "cursor") {
        Some(Value::Str(text)) => Some(text.clone()),
        _ => None,
    };
    let limit = match get(args, "limit") {
        Some(Value::Int(n)) => {
            let maximum = if group_id.as_deref().is_some_and(|g| !g.is_empty()) {
                groups::MAX_MEMBER_LIMIT
            } else {
                groups::MAX_GROUP_LIMIT
            };
            let bounded = i64::try_from(n).ok().filter(|n| (1..=maximum).contains(n));
            Some(bounded.ok_or_else(|| {
                AppError::new(
                    "validation",
                    format!("--limit must be between 1 and {maximum}"),
                )
            })?)
        }
        _ => None,
    };
    groups::quarantine_groups(
        conn,
        &profile_id,
        group_id.as_deref(),
        cursor.as_ref(),
        limit,
    )
}

/// `ui.journals.quarantine.resolve`: one audited price override or exclusion,
/// then an optional plain rebuild, mirroring Python's
/// `_quarantine_resolution_payload` with its `default_source` (`"gui"` for the
/// desktop, `"ai_tool"` for the assistant).
pub(super) fn quarantine_resolve(
    conn: &Connection,
    scope: &dyn Fn() -> Result<(String, String), kassiber_envelope::AppError>,
    args: &Value,
    source: &str,
) -> Result<Value, kassiber_envelope::AppError> {
    use kassiber_envelope::{dict, get, py_str, s, AppError};
    use kassiber_pyjson::Key;
    const KIND: &str = "ui.journals.quarantine.resolve";
    let Value::Dict(fields) = args else {
        unreachable!("the daemon coerces arguments to an object")
    };
    let allowed = [
        "transaction",
        "group_id",
        "action",
        "fiat_rate",
        "fiat_value",
        "reason",
        "reprocess",
    ];
    let mut unknown: Vec<Value> = fields
        .iter()
        .filter_map(|(key, _)| match key {
            Key::Str(name) if !name.as_str().is_some_and(|n| allowed.contains(&n)) => {
                Some(Value::Str(name.clone()))
            }
            _ => None,
        })
        .collect();
    if !unknown.is_empty() {
        unknown.sort_by(|a, b| match (a, b) {
            (Value::Str(a), Value::Str(b)) => a.code_points().cmp(b.code_points()),
            _ => std::cmp::Ordering::Equal,
        });
        return Err(
            AppError::new("validation", format!("{KIND} received unsupported fields"))
                .with_details(dict([("unknown", Value::List(unknown))])),
        );
    }
    let present = |key: &str| get(args, key).filter(|v| !matches!(v, Value::Null));
    let stripped = |key: &str| match get(args, key) {
        Some(Value::Str(text)) => {
            let text = kassiber_pytext::strip_py(text);
            text.code_points()
                .next()
                .is_some()
                .then(|| py_str(&Value::Str(text)))
        }
        _ => None,
    };
    let transaction_arg = present("transaction");
    let group_arg = present("group_id");
    if transaction_arg.is_none() == group_arg.is_none() {
        return Err(AppError::new(
            "validation",
            format!("{KIND} requires exactly one of transaction or group_id"),
        ));
    }
    let transaction = match transaction_arg {
        None => None,
        Some(_) => Some(stripped("transaction").ok_or_else(|| {
            AppError::new("validation", format!("{KIND} requires a transaction"))
        })?),
    };
    let group_id = match group_arg {
        None => None,
        Some(_) => Some(stripped("group_id").ok_or_else(|| {
            AppError::new(
                "validation",
                format!("{KIND} group_id must be a non-empty string"),
            )
        })?),
    };
    let action = match get(args, "action") {
        Some(Value::Str(text)) if text.is("price_override") => "price_override",
        Some(Value::Str(text)) if text.is("exclude") => "exclude",
        Some(value @ (Value::List(_) | Value::Dict(_))) => {
            return Err(AppError::new(
                "internal_error",
                format!("unhashable type: '{}'", kassiber_envelope::type_name(value)),
            ))
        }
        _ => {
            return Err(
                AppError::new("validation", format!("{KIND} action is invalid")).with_details(
                    dict([(
                        "allowed",
                        Value::List(vec![s("price_override"), s("exclude")]),
                    )]),
                ),
            )
        }
    };
    let reason = stripped("reason")
        .ok_or_else(|| AppError::new("validation", format!("{KIND} requires an audit reason")))?;
    let reprocess = match get(args, "reprocess") {
        None => true,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => {
            return Err(AppError::new(
                "validation",
                format!("{KIND} reprocess must be boolean"),
            ))
        }
    };
    // Python resolves the active book, a root group's root and the
    // membership before the price checks.
    let (workspace_id, profile_id) = scope()?;
    let (workspace_id, profile_id) = (workspace_id.as_str(), profile_id.as_str());
    let transaction = match &group_id {
        // A root group acts on its root; what waits on it clears (or not) as
        // a side effect of the reprocess, reported below.
        // `transaction.strip()` applies to the root id too.
        Some(group) => kassiber_pytext::strip(
            &kassiber_journals::quarantine::groups::root_transaction_for_group(
                conn, profile_id, group,
            )?,
        )
        .to_owned(),
        None => transaction.expect("exactly one target"),
    };
    let membership_before = kassiber_journals::quarantine::groups::membership(conn, profile_id)?;
    let (rate, total) = (present("fiat_rate"), present("fiat_value"));
    if action == "price_override" {
        if rate.is_none() && total.is_none() {
            return Err(AppError::new(
                "validation",
                "price_override requires fiat_rate or fiat_value from reviewed evidence",
            ));
        }
        if rate.is_some() && total.is_some() {
            return Err(AppError::new(
                "validation",
                "price_override accepts either fiat_rate or fiat_value, not both",
            ));
        }
    } else if rate.is_some() || total.is_some() {
        return Err(AppError::new(
            "validation",
            "exclude does not accept fiat_rate or fiat_value",
        ));
    }

    let resolution = kassiber_journals::cli::resolve(
        conn,
        kassiber_journals::cli::Scope {
            workspace_id,
            profile_id,
        },
        &kassiber_journals::cli::Resolution {
            action: if action == "exclude" {
                kassiber_journals::cli::Action::Exclude
            } else {
                kassiber_journals::cli::Action::PriceOverride
            },
            transaction: &transaction,
            fiat_rate: rate,
            fiat_value: total,
            source,
            reason: &reason,
        },
    )?;
    let transaction_id = py_str(get(&resolution, "transaction_id").unwrap_or(&Value::Null));
    let sql =
        |error: kassiber_store::rusqlite::Error| AppError::new("internal_error", error.to_string());
    let journal_process = if reprocess {
        let committed = process(conn, None, None, false, &mut |_| {})
            .map_err(kassiber_journals::service::application_error)?;
        let payload = committed.ordered_payload.clone();
        committed
            .after_reply()
            .map_err(kassiber_journals::service::application_error)?;
        payload
    } else {
        Value::Null
    };
    let remaining = conn
        .query_row(
            "SELECT reason, detail_json, created_at FROM journal_quarantines WHERE transaction_id = ?",
            [transaction_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, kassiber_store::rusqlite::types::Value>(0)?,
                    row.get::<_, kassiber_store::rusqlite::types::Value>(1)?,
                    row.get::<_, kassiber_store::rusqlite::types::Value>(2)?,
                ))
            },
        )
        .map(Some)
        .or_else(|error| match error {
            kassiber_store::rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(sql(other)),
        })?;
    let scalar = |value: kassiber_store::rusqlite::types::Value| {
        use kassiber_store::rusqlite::types::Value as Sql;
        match value {
            Sql::Null => Value::Null,
            Sql::Integer(n) => Value::Int(n.into()),
            Sql::Real(n) => Value::Float(n),
            Sql::Text(text) => Value::Str(text.as_str().into()),
            Sql::Blob(bytes) => Value::Bytes(bytes),
        }
    };
    let remaining = remaining.map(|(reason, detail, created)| {
        use kassiber_store::rusqlite::types::Value as Sql;
        let text = match &detail {
            Sql::Null => Some("{}".to_owned()),
            Sql::Text(text) if text.is_empty() => Some("{}".to_owned()),
            Sql::Text(text) => Some(text.clone()),
            Sql::Blob(bytes) if bytes.is_empty() => Some("{}".to_owned()),
            Sql::Blob(bytes) => String::from_utf8(bytes.clone()).ok(),
            _ => None,
        };
        let detail = match text.map(|t| kassiber_pytext::loads(&t)) {
            Some(Ok(value @ Value::Dict(_))) => value,
            _ => dict::<&str>([]),
        };
        dict([
            ("reason", scalar(reason)),
            ("detail", detail),
            ("created_at", scalar(created)),
        ])
    });
    let effect = kassiber_journals::quarantine::groups::resolution_effect(
        conn,
        profile_id,
        &transaction_id,
        &membership_before,
        reprocess,
    )?;
    let mut payload = vec![
        ("transaction_id", s(transaction_id.as_str())),
        (
            "group_id",
            group_id.map_or(Value::Null, |group| s(group.as_str())),
        ),
        ("action", s(action)),
        ("resolution", resolution),
        ("reprocessed", Value::Bool(reprocess)),
        ("journal_process", journal_process),
        ("cleared", Value::Bool(remaining.is_none())),
        ("remaining_quarantine", remaining.unwrap_or(Value::Null)),
    ];
    payload.extend(effect);
    Ok(dict(payload))
}

/// The journal CLI leaves (`cli/main.py`'s `journals` branch) other than
/// `process` and `quarantine groups`; `args` is the parsed namespace.
/// `None` for any other kind.
#[allow(clippy::type_complexity)]
pub(super) fn cli_command(
    conn: &Connection,
    kind: &str,
    args: &Value,
    scope: &dyn Fn(
        Option<&Value>,
        Option<&Value>,
    ) -> Result<(String, String), kassiber_envelope::AppError>,
) -> Option<Result<Value, kassiber_envelope::AppError>> {
    use kassiber_envelope::{dict, get, s, AppError};
    use kassiber_journals::cli;
    const KINDS: [&str; 9] = [
        "journals.list",
        "journals.transfers.list",
        "journals.events.list",
        "journals.events.get",
        "journals.quarantined",
        "journals.quarantine.show",
        "journals.quarantine.clear",
        "journals.quarantine.resolve.price-override",
        "journals.quarantine.resolve.exclude",
    ];
    if !KINDS.contains(&kind) {
        return None;
    }
    let arg = |name: &str| get(args, name).filter(|value| !matches!(value, Value::Null));
    let text = |name: &str| match arg(name) {
        Some(Value::Str(text)) => text.as_str().map(str::to_owned),
        _ => None,
    };
    let int = |name: &str| match arg(name) {
        Some(Value::Int(n)) => Some(n.clone()),
        _ => None,
    };
    let (workspace, profile) = (arg("workspace"), arg("profile"));
    Some((|| -> Result<Value, AppError> {
        let (workspace_id, profile_id) = scope(workspace, profile)?;
        let ids = cli::Scope {
            workspace_id: &workspace_id,
            profile_id: &profile_id,
        };
        let transaction = text("transaction").unwrap_or_default();
        match kind {
            "journals.list" => {
                let limit = int("limit");
                let cursor = text("cursor");
                let (records, meta) = cli::entries(conn, ids, limit.as_ref(), cursor.as_deref())?;
                let mut fields = vec![("records".to_owned(), records)];
                fields.extend(meta);
                Ok(dict(fields.iter().map(|(k, v)| (k.as_str(), v.clone()))))
            }
            "journals.transfers.list" => cli::transfer_audit(conn, ids),
            "journals.events.list" => {
                let limit = int("limit");
                let fields = [
                    "wallet",
                    "account",
                    "asset",
                    "entry_type",
                    "start",
                    "end",
                    "cursor",
                ]
                .map(text);
                let [wallet, account, asset, entry_type, start, end, cursor] = &fields;
                cli::events(
                    conn,
                    ids,
                    &cli::EventFilters {
                        wallet: wallet.as_deref(),
                        account: account.as_deref(),
                        asset: asset.as_deref(),
                        entry_type: entry_type.as_deref(),
                        start: start.as_deref(),
                        end: end.as_deref(),
                        cursor: cursor.as_deref(),
                        limit: limit.as_ref(),
                    },
                )
            }
            "journals.events.get" => cli::event(conn, ids, &text("event_id").unwrap_or_default()),
            "journals.quarantined" => cli::quarantines(conn, ids),
            "journals.quarantine.show" => cli::quarantine_show(conn, ids, &transaction),
            "journals.quarantine.clear" => cli::quarantine_clear(conn, ids, &transaction),
            _ => {
                let action = if kind.ends_with(".exclude") {
                    cli::Action::Exclude
                } else {
                    cli::Action::PriceOverride
                };
                let (rate, total) = (arg("fiat_rate"), arg("fiat_value"));
                let reason = match action {
                    cli::Action::Exclude => "Resolved quarantine by excluding transaction",
                    cli::Action::PriceOverride => {
                        "Resolved quarantine with manual pricing override"
                    }
                };
                let group_id = text("group_id");
                let reprocess = matches!(arg("reprocess"), Some(Value::Bool(true)));
                if group_id.is_none() && !reprocess {
                    return cli::resolve(
                        conn,
                        ids,
                        &cli::Resolution {
                            action,
                            transaction: &transaction,
                            fiat_rate: rate,
                            fiat_value: total,
                            source: "cli",
                            reason,
                        },
                    );
                }
                // `resolve_quarantine_with_effect`: a `root:` group acts on its
                // root, and the effect on rows waiting on it is reported.
                let transaction = match &group_id {
                    Some(group) => {
                        kassiber_journals::quarantine::groups::root_transaction_for_group(
                            conn,
                            &profile_id,
                            group,
                        )?
                    }
                    None => transaction,
                };
                let before = kassiber_journals::quarantine::groups::membership(conn, &profile_id)?;
                let resolution = cli::resolve(
                    conn,
                    ids,
                    &cli::Resolution {
                        action,
                        transaction: &transaction,
                        fiat_rate: rate,
                        fiat_value: total,
                        source: "cli",
                        reason,
                    },
                )?;
                let Value::Dict(mut payload) = resolution else {
                    unreachable!("a resolution is a record")
                };
                let resolved = kassiber_envelope::py_str(
                    get(&Value::Dict(payload.clone()), "transaction_id").unwrap_or(&Value::Null),
                );
                payload.push(("group_id".into(), group_id.map_or(Value::Null, s)));
                payload.push(("reprocessed".into(), Value::Bool(reprocess)));
                if reprocess {
                    let committed = process(conn, workspace, profile, false, &mut |_| {})
                        .map_err(kassiber_journals::service::application_error)?;
                    let journal = committed.ordered_payload.clone();
                    committed
                        .after_reply()
                        .map_err(kassiber_journals::service::application_error)?;
                    payload.push(("journal_process".into(), journal));
                    payload.retain(|(key, _)| *key != "note".into());
                    use kassiber_store::rusqlite::OptionalExtension;
                    let remaining: Option<kassiber_store::rusqlite::types::Value> = conn
                        .query_row(
                            "SELECT reason FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
                            [profile_id.as_str(), resolved.as_str()],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
                    payload.push(("cleared".into(), Value::Bool(remaining.is_none())));
                    payload.push((
                        "remaining_reason".into(),
                        match remaining {
                            None => Value::Null,
                            Some(kassiber_store::rusqlite::types::Value::Text(text)) => s(text),
                            Some(kassiber_store::rusqlite::types::Value::Integer(n)) => {
                                Value::Int(n.into())
                            }
                            Some(kassiber_store::rusqlite::types::Value::Real(n)) => {
                                Value::Float(n)
                            }
                            Some(kassiber_store::rusqlite::types::Value::Blob(bytes)) => {
                                Value::Bytes(bytes)
                            }
                            Some(kassiber_store::rusqlite::types::Value::Null) => Value::Null,
                        },
                    ));
                }
                let effect = kassiber_journals::quarantine::groups::resolution_effect(
                    conn,
                    &profile_id,
                    &resolved,
                    &before,
                    reprocess,
                )?;
                payload.extend(effect.into_iter().map(|(key, value)| (key.into(), value)));
                Ok(Value::Dict(payload))
            }
        }
    })())
}
