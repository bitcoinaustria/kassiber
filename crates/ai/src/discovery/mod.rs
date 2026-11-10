//! Provider and model discovery: the RAM-only last-good cache
//! (`kassiber/ai/discovery_cache.py`) and model-metadata sanitising.

pub mod metadata;

use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;

use crate::clock::checked_at;
use crate::pyval::{opt, s};

/// The discovery time-to-live.
pub const TTL: Duration = Duration::from_secs(60);

/// `DiscoverySnapshot`.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub value: Value,
    pub checked_at: String,
    pub stale: bool,
    /// `{code, message}` of the failure a stale snapshot stands in for (a
    /// saved check reads back whatever dict was stored).
    pub error: Option<Value>,
}

impl Snapshot {
    /// `metadata()`: `checked_at`, `stale` and `error`.
    pub fn metadata(&self) -> Vec<(&'static str, Value)> {
        vec![
            ("checked_at", s(&self.checked_at)),
            ("stale", Value::Bool(self.stale)),
            ("error", self.error.clone().unwrap_or(Value::Null)),
        ]
    }
}

/// The metadata of "never checked": what a display read reports on a cold
/// cache.
pub fn unchecked_metadata() -> Vec<(&'static str, Value)> {
    vec![
        ("checked_at", opt(None::<&str>)),
        ("stale", Value::Bool(false)),
        ("error", Value::Null),
    ]
}

struct Entry {
    snapshot: Snapshot,
    expires: Instant,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    refreshing: HashSet<String>,
    /// Cold-cache failures, for the waiters of that same probe only.
    failures: HashMap<String, AppError>,
}

/// `ProviderDiscoveryCache`: one probe per key at a time, shared by its
/// waiters; the last good result is kept and served stale after a failure.
pub struct Cache {
    ttl: Duration,
    state: Mutex<State>,
    changed: Condvar,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(TTL)
    }
}

/// `_safe_error(exc)`.
fn safe_error(error: &AppError) -> Value {
    let code = if error.code.is_empty() {
        "ai_unavailable"
    } else {
        error.code.as_str()
    };
    dict([("code", s(code)), ("message", s(&error.message))])
}

impl Cache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Forgets every snapshot and failure (a provider changed).
    pub fn invalidate(&self) {
        let mut state = self.lock();
        state.entries.clear();
        state.failures.clear();
    }

    /// The last snapshot for `key`, expired or not, without ever probing.
    pub fn peek(&self, key: &str) -> Option<Snapshot> {
        self.lock()
            .entries
            .get(key)
            .map(|entry| entry.snapshot.clone())
    }

    /// `seed(key, snapshot)`: hold a saved snapshot without probing, already
    /// expired. `peek` reports it and the next `get` still probes; a failure
    /// there returns it marked stale, as for any earlier result.
    pub fn seed(&self, key: &str, snapshot: Snapshot) {
        let mut state = self.lock();
        if !state.entries.contains_key(key) {
            state.entries.insert(
                key.to_owned(),
                Entry {
                    snapshot,
                    expires: Instant::now(),
                },
            );
        }
    }

    /// The snapshot for `key`, probing with `fetch` on a cold or expired
    /// entry or on `refresh`. Concurrent callers of one key share a probe.
    pub fn get(
        &self,
        key: &str,
        refresh: bool,
        fetch: impl FnOnce() -> Result<Value, AppError>,
    ) -> Result<Snapshot, AppError> {
        {
            let mut state = self.lock();
            if let Some(entry) = state.entries.get(key) {
                if !refresh && entry.expires > Instant::now() {
                    return Ok(entry.snapshot.clone());
                }
            }
            if state.refreshing.contains(key) {
                while state.refreshing.contains(key) {
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                if let Some(entry) = state.entries.get(key) {
                    return Ok(entry.snapshot.clone());
                }
                if let Some(failure) = state.failures.get(key) {
                    // A cold probe just failed: share its outcome rather
                    // than turning one slow failure into N.
                    return Err(failure.clone());
                }
            }
            state.failures.remove(key);
            state.refreshing.insert(key.to_owned());
        }
        struct Done<'a>(&'a Cache, &'a str);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                let mut state = self.0.lock();
                state.refreshing.remove(self.1);
                self.0.changed.notify_all();
            }
        }
        let _done = Done(self, key);
        match fetch() {
            Ok(value) => {
                let snapshot = Snapshot {
                    value,
                    checked_at: checked_at(),
                    stale: false,
                    error: None,
                };
                self.lock().entries.insert(
                    key.to_owned(),
                    Entry {
                        snapshot: snapshot.clone(),
                        expires: Instant::now() + self.ttl,
                    },
                );
                Ok(snapshot)
            }
            Err(error) => {
                let mut state = self.lock();
                state.failures.insert(key.to_owned(), error.clone());
                if let Some(previous) = state.entries.get(key) {
                    let snapshot = Snapshot {
                        value: previous.snapshot.value.clone(),
                        checked_at: checked_at(),
                        stale: true,
                        error: Some(safe_error(&error)),
                    };
                    state.entries.insert(
                        key.to_owned(),
                        Entry {
                            snapshot: snapshot.clone(),
                            expires: Instant::now() + self.ttl,
                        },
                    );
                    return Ok(snapshot);
                }
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    // `tests/test_ai_discovery_cache.py`.
    #[test]
    fn single_flight_and_last_good_fallback() {
        let cache = Arc::new(Cache::new(Duration::from_secs(60)));
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(3));
        let threads: Vec<_> = (0..3)
            .map(|_| {
                let (cache, calls, barrier) = (cache.clone(), calls.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    cache
                        .get("models", false, || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(100));
                            Ok(Value::List(vec![s("m")]))
                        })
                        .unwrap()
                })
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(results
            .iter()
            .all(|r| r.value == Value::List(vec![s("m")]) && !r.stale));
        let stale = cache
            .get("models", true, || {
                Err(AppError::new("ai_unavailable", "down"))
            })
            .unwrap();
        assert!(stale.stale);
        assert_eq!(stale.value, Value::List(vec![s("m")]));
        assert_eq!(
            stale.error,
            Some(dict([
                ("code", s("ai_unavailable")),
                ("message", s("down"))
            ]))
        );
        assert_eq!(cache.peek("models"), Some(stale));
        assert_eq!(cache.peek("other"), None);
        cache.invalidate();
        assert_eq!(cache.peek("models"), None);
    }

    #[test]
    fn a_seeded_snapshot_is_reported_but_never_spares_a_probe() {
        let cache = Cache::new(Duration::from_secs(60));
        let saved = Snapshot {
            value: Value::List(vec![s("old")]),
            checked_at: "2026-11-01T00:00:00Z".into(),
            stale: false,
            error: None,
        };
        cache.seed("models", saved.clone());
        assert_eq!(cache.peek("models"), Some(saved.clone()));
        // An entry this session wins over a later seed.
        cache.seed(
            "models",
            Snapshot {
                value: Value::Null,
                ..saved.clone()
            },
        );
        assert_eq!(cache.peek("models"), Some(saved));
        // The next get still probes; its failure reports the seed as stale.
        let calls = AtomicUsize::new(0);
        let stale = cache
            .get("models", false, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(AppError::new("ai_unavailable", "down"))
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(stale.stale);
        assert_eq!(stale.value, Value::List(vec![s("old")]));
    }

    #[test]
    fn cold_probe_failure_is_shared_with_waiters() {
        let cache = Arc::new(Cache::new(Duration::from_secs(60)));
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(3));
        let threads: Vec<_> = (0..3)
            .map(|_| {
                let (cache, calls, barrier) = (cache.clone(), calls.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    cache.get("k", false, || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(150));
                        Err(AppError::new("ai_unavailable", "slow failure"))
                    })
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap().unwrap_err().message, "slow failure");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stale_failure_is_not_replayed_to_a_later_call_and_the_ttl_expires() {
        let cache = Cache::new(Duration::from_millis(30));
        assert!(cache
            .get("k", false, || Err(AppError::new("ai_unavailable", "x")))
            .is_err());
        let fresh = cache.get("k", false, || Ok(s("ok"))).unwrap();
        assert_eq!(fresh.value, s("ok"));
        let cached = cache
            .get("k", false, || panic!("probed a warm entry"))
            .unwrap();
        assert_eq!(cached, fresh);
        std::thread::sleep(Duration::from_millis(40));
        let mut probed = false;
        cache
            .get("k", false, || {
                probed = true;
                Ok(s("again"))
            })
            .unwrap();
        assert!(probed, "an expired entry probes again");
        // A probe that panics releases its key.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.get("p", false, || panic!("boom"))
        }));
        assert_eq!(
            cache.get("p", false, || Ok(s("ok"))).unwrap().value,
            s("ok")
        );
    }
}
