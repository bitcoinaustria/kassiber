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
