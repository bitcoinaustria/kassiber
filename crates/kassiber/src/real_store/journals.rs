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
struct Sources;
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
