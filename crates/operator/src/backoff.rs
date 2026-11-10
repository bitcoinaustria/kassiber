//! `kassiber/secrets/auth_backoff.py`: persisted retry smoothing for
//! passphrase attempts, shared by the daemon and the broker through
//! `config/auth_backoff.json`.
//!
//! The deadline is UTC epoch time so it survives restarts. It is not an
//! adversarial rate limit (SQLCipher's KDF is), and a loaded deadline is
//! clamped to the maximum so a clock correction cannot lock anyone out.

use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::{dumps, BigInt, Key, Options, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const AUTH_FAILURES_BEFORE_BACKOFF: u64 = 3;
pub const AUTH_BACKOFF_BASE_SECONDS: f64 = 5.0;
pub const AUTH_BACKOFF_MAX_SECONDS: f64 = 30.0;
pub const AUTH_BACKOFF_FILENAME: &str = "auth_backoff.json";

#[derive(Debug, Default)]
struct Counters {
    failures: u64,
    locked_until: f64,
}

/// `AuthAttemptBackoff(state_path)`.
#[derive(Debug, Default)]
pub struct AuthAttemptBackoff {
    state: Mutex<Counters>,
    path: Option<PathBuf>,
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

impl AuthAttemptBackoff {
    pub fn new(path: Option<PathBuf>) -> Self {
        AuthAttemptBackoff {
            state: Mutex::new(Counters::default()),
            path,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Counters> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// `check(scope)`: refuse while a deadline is pending.
    pub fn check(&self, scope: &str) -> Result<(), AppError> {
        let now = now();
        let mut retry_after;
        {
            let mut counters = self.lock();
            self.load(&mut counters);
            retry_after = counters.locked_until - now;
            if retry_after <= 0.0 {
                if counters.locked_until != 0.0 {
                    counters.locked_until = 0.0;
                    self.persist(&counters);
                }
                return Ok(());
            }
            if retry_after > AUTH_BACKOFF_MAX_SECONDS {
                retry_after = AUTH_BACKOFF_MAX_SECONDS;
                counters.locked_until = now + retry_after;
                self.persist(&counters);
            }
        }
        // max(1, int(retry_after + 0.999)), truncating toward zero.
        let seconds = ((retry_after + 0.999) as i64).max(1);
        Err(AppError::new(
            "local_auth_rate_limited",
            "too many failed passphrase attempts",
        )
        .with_details(dict([
            ("scope", s(scope)),
            ("throttle", s("database")),
            ("retry_after_seconds", Value::Int(BigInt::from(seconds))),
        ]))
        .with_hint("Wait before trying the passphrase again.")
        .retryable(true))
    }

    /// `record_success()`.
    pub fn record_success(&self) {
        let mut counters = self.lock();
        counters.failures = 0;
        counters.locked_until = 0.0;
        self.persist(&counters);
    }

    /// `record_failure()`.
    pub fn record_failure(&self) {
        let now = now();
        let mut counters = self.lock();
        self.load(&mut counters);
        counters.failures += 1;
        if counters.failures < AUTH_FAILURES_BEFORE_BACKOFF {
            self.persist(&counters);
            return;
        }
        let exponent = counters.failures - AUTH_FAILURES_BEFORE_BACKOFF;
        let delay = if exponent >= 63 {
            AUTH_BACKOFF_MAX_SECONDS
        } else {
            (AUTH_BACKOFF_BASE_SECONDS * (1u64 << exponent) as f64).min(AUTH_BACKOFF_MAX_SECONDS)
        };
        counters.locked_until = now + delay;
        self.persist(&counters);
    }

    /// `_load_locked`: an unreadable or malformed file leaves the in-memory
    /// counters as they were.
    fn load(&self, counters: &mut Counters) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return;
        };
        let Ok(payload) = kassiber_pytext::loads(text) else {
            return;
        };
        let Value::Dict(_) = payload else {
            return;
        };
        let field = |key: &str| kassiber_envelope::get(&payload, key);
        let Some(failures) = python_int(field("failures"), 0) else {
            return;
        };
        let Some(locked_until) = python_float(field("locked_until")) else {
            return;
        };
        counters.failures = failures.max(0) as u64;
        // max(0.0, x) keeps 0.0 unless x is greater (NaN included).
        counters.locked_until = if locked_until > 0.0 {
            locked_until
        } else {
            0.0
        };
    }

    /// `_persist_locked`: best effort; an OS error is ignored.
    fn persist(&self, counters: &Counters) {
        let Some(path) = &self.path else {
            return;
        };
        if counters.failures == 0 && counters.locked_until <= 0.0 {
            let _ = std::fs::remove_file(path);
            return;
        }
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let payload = Value::Dict(vec![
            (
                Key::from("failures"),
                Value::Int(BigInt::from(counters.failures)),
            ),
            (
                Key::from("locked_until"),
                Value::Float(counters.locked_until),
            ),
        ]);
        let Ok(mut text) = dumps(&payload, &Options::new().sort_keys(true)) else {
            return;
        };
        text.push('\n');
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        // Python writes and renames without syncing; a deadline that a power
        // loss could roll back would reopen the throttle, so flush both.
        if write_synced(&temporary, text.as_bytes()).is_ok()
            && std::fs::rename(&temporary, path).is_ok()
        {
            if let Some(parent) = path.parent() {
                let _ = sync_directory(parent);
            }
        }
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    #[cfg(test)]
    tests::SYNCED.with(|synced| synced.borrow_mut().push(path.to_owned()));
    file.sync_all()
}

/// `sync_all` is `F_FULLFSYNC` on macOS. Directory syncs are a no-op on Windows.
fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    tests::SYNCED.with(|synced| synced.borrow_mut().push(path.to_owned()));
    #[cfg(not(windows))]
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}

/// `int(value)` for a JSON value, `default` when missing; `None` where
/// Python raises `TypeError`/`ValueError`.
fn python_int(value: Option<&Value>, default: i64) -> Option<i64> {
    match value {
        None => Some(default),
        Some(Value::Bool(b)) => Some(i64::from(*b)),
        Some(Value::Int(n)) => Some(i64::try_from(n.clone()).unwrap_or(i64::MAX)),
        Some(Value::Float(f)) if f.is_finite() => Some(f.trunc() as i64),
        Some(Value::Str(text)) => text
            .as_str()
            .and_then(|text| text.trim().replace('_', "").parse::<i64>().ok()),
        _ => None,
    }
}

/// `float(value)` for a JSON value, `0.0` when missing.
fn python_float(value: Option<&Value>) -> Option<f64> {
    match value {
        None => Some(0.0),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        Some(Value::Int(n)) => n.to_string().parse::<f64>().ok(),
        Some(Value::Float(f)) => Some(*f),
        Some(Value::Str(text)) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_float(text).ok()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Files and directories this test thread flushed, in order.
        pub(super) static SYNCED: std::cell::RefCell<Vec<PathBuf>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    #[test]
    fn persisted_counters_are_synced_before_and_after_the_rename() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config").join(AUTH_BACKOFF_FILENAME);
        let backoff = AuthAttemptBackoff::new(Some(path.clone()));
        SYNCED.with(|synced| synced.borrow_mut().clear());
        backoff.record_failure();
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        assert_eq!(
            SYNCED.with(|synced| synced.take()),
            vec![PathBuf::from(temporary), root.path().join("config")]
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"failures\": 1, \"locked_until\": 0.0}\n"
        );
    }

    #[test]
    fn three_failures_start_a_persisted_backoff() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config").join(AUTH_BACKOFF_FILENAME);
        let backoff = AuthAttemptBackoff::new(Some(path.clone()));
        backoff.check("operator_unlock").unwrap();
        backoff.record_failure();
        backoff.record_failure();
        backoff.check("operator_unlock").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "{\"failures\": 2, \"locked_until\": 0.0}\n");
        backoff.record_failure();
        let error = backoff.check("operator_unlock").unwrap_err();
        assert_eq!(error.code, "local_auth_rate_limited");
        assert!(error.retryable);
        let details = error.details.unwrap();
        assert_eq!(
            kassiber_envelope::get(&details, "retry_after_seconds"),
            Some(&Value::Int(BigInt::from(5)))
        );
        // Another process sees the same deadline.
        let other = AuthAttemptBackoff::new(Some(path.clone()));
        assert!(other.check("operator_admin").is_err());
        backoff.record_success();
        assert!(!path.exists());
        // An instance keeps what it loaded while the file is gone, as Python's.
        assert!(other.check("operator_admin").is_err());
        AuthAttemptBackoff::new(Some(path))
            .check("operator_admin")
            .unwrap();
    }

    #[test]
    fn a_far_future_deadline_is_clamped() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(AUTH_BACKOFF_FILENAME);
        std::fs::write(&path, "{\"failures\": 9, \"locked_until\": 1e300}").unwrap();
        let error = AuthAttemptBackoff::new(Some(path.clone()))
            .check("scope")
            .unwrap_err();
        let details = error.details.unwrap();
        assert_eq!(
            kassiber_envelope::get(&details, "retry_after_seconds"),
            Some(&Value::Int(BigInt::from(30)))
        );
        let rewritten = kassiber_pytext::loads(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(matches!(
            kassiber_envelope::get(&rewritten, "locked_until"),
            Some(Value::Float(f)) if *f < now() + 31.0
        ));
    }

    #[test]
    fn delays_double_to_the_maximum() {
        let backoff = AuthAttemptBackoff::new(None);
        let mut seen = Vec::new();
        for _ in 0..7 {
            backoff.record_failure();
            let locked = backoff.lock().locked_until;
            seen.push(if locked == 0.0 {
                0
            } else {
                (locked - now()).round() as i64
            });
        }
        assert_eq!(seen, [0, 0, 5, 10, 20, 30, 30]);
    }
}
