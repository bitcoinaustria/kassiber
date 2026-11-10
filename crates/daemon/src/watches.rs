//! `daemon_chain_analysis_watches`: the unlocked encrypted book's background
//! pass. It runs authorized source batches and due local watch evaluation on
//! its own connection, then emits one generic at-least-once wake-up event;
//! inbox identities never leave the database. Lock, restore, passphrase
//! rotation and shutdown stop it before the book closes.
use super::Daemon;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::WatchWorker;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(5);
/// An authorized source read has an eight-second inactivity timeout.
const STOP_WAIT: Duration = Duration::from_secs(10);

pub(crate) struct Thread {
    stop: Arc<AtomicBool>,
    done: Arc<(Mutex<bool>, Condvar)>,
}

impl Thread {
    fn finished(&self) -> bool {
        *self.done.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn run(mut worker: Box<dyn WatchWorker>, output: super::Output, stop: &AtomicBool) {
    let stopped = || stop.load(Ordering::SeqCst);
    while !stopped() {
        // No source IDs, error text or watch titles enter operational output.
        if let Ok(ids) = worker.pass(&stopped) {
            if !ids.is_empty() && !stopped() {
                if output
                    .event(
                        "ui.chain_analysis.watches.changed",
                        dict([("local_only", Value::Bool(true))]),
                    )
                    .is_err()
                {
                    return;
                }
                if !stopped() {
                    let _ = worker.delivered(&ids);
                }
            }
        }
        #[cfg(feature = "test-kinds")]
        mark_pass();
        let mut waited = Duration::ZERO;
        while waited < POLL && !stopped() {
            std::thread::sleep(Duration::from_millis(50));
            waited += Duration::from_millis(50);
        }
    }
}

/// Tests wait for completed passes instead of a timeout when nothing reaches
/// the source: append one line per finished pass to the named file.
#[cfg(feature = "test-kinds")]
fn mark_pass() {
    if let Some(path) = std::env::var_os("KASSIBER_RUST_TEST_WATCH_PASS_LOG") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = file.write_all(b"pass\n");
        }
    }
}

impl Daemon {
    /// `start_worker`: idempotent; only an unlocked keyed book gets a worker.
    pub(crate) fn start_watch_worker(&mut self) {
        #[cfg(feature = "test-kinds")]
        if std::env::var("KASSIBER_RUST_TEST_NO_WATCH_WORKER").as_deref() == Ok("1") {
            return;
        }
        if self.watch_worker.as_ref().is_some_and(|t| !t.finished()) {
            return;
        }
        let Some(book) = self.book.as_mut() else {
            return;
        };
        let Ok(Some(worker)) = book.chain_analysis_watch_worker() else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let (flag, signal, output) = (Arc::clone(&stop), Arc::clone(&done), self.output.clone());
        let spawned = std::thread::Builder::new()
            .name("local-evidence-watches".into())
            .spawn(move || {
                run(worker, output, &flag);
                *signal.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
                signal.1.notify_all();
            });
        if spawned.is_ok() {
            self.watch_worker = Some(Thread { stop, done });
        }
    }

    /// `stop_worker`: wait for a bounded source read to observe the stop.
    pub(crate) fn stop_watch_worker(&mut self, require_stopped: bool) -> Result<(), AppError> {
        let Some(thread) = self.watch_worker.as_ref() else {
            return Ok(());
        };
        thread.stop.store(true, Ordering::SeqCst);
        let guard = thread.done.0.lock().unwrap_or_else(|e| e.into_inner());
        let (guard, _) = thread
            .done
            .1
            .wait_timeout_while(guard, STOP_WAIT, |done| !*done)
            .unwrap_or_else(|e| e.into_inner());
        if !*guard {
            drop(guard);
            if require_stopped {
                return Err(AppError::new(
                    "project_operation_in_progress",
                    "Local evidence work is stopping; try again",
                )
                .retryable(true));
            }
            return Ok(());
        }
        drop(guard);
        self.watch_worker = None;
        Ok(())
    }
}
