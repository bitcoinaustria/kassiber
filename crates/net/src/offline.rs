//! Revision-bound preference cache. Never infer freshness from a TTL.
use crate::Offline;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct OfflineCache {
    reader: Arc<dyn Fn() -> Offline + Send + Sync>,
    revision: Arc<dyn Fn() -> Option<u64> + Send + Sync>,
    cached: Arc<Mutex<Option<(u64, Offline)>>>,
}

impl OfflineCache {
    /// `revision` must cheaply read the current, non-reused preference generation
    /// across every writer/process, returning None on unavailable/ambiguous state.
    /// A timestamp alone is insufficient. The reader and generation source must
    /// implement one coherent publication protocol. Without that protocol use
    /// Gate::new with a fresh reader instead; this cache is an opt-in integration.
    pub fn new(
        reader: impl Fn() -> Offline + Send + Sync + 'static,
        revision: impl Fn() -> Option<u64> + Send + Sync + 'static,
    ) -> Self {
        Self {
            reader: Arc::new(reader),
            revision: Arc::new(revision),
            cached: Arc::new(Mutex::new(None)),
        }
    }

    /// Explicit invalidation for preference replacement, lock/switch or errors.
    pub fn invalidate(&self) {
        if let Ok(mut cached) = self.cached.lock() {
            *cached = None;
        }
    }

    pub fn read(&self) -> Offline {
        let Ok(mut cached) = self.cached.lock() else {
            return Offline::Unavailable;
        };
        let Some(revision) = (self.revision)() else {
            *cached = None;
            return Offline::Unavailable;
        };
        if let Some((previous, state)) = *cached {
            if previous == revision {
                return state;
            }
        }
        let state = (self.reader)();
        if (self.revision)() != Some(revision) {
            *cached = None;
            return Offline::Unavailable;
        }
        *cached = Some((revision, state));
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    #[test]
    fn cache_requires_fresh_revision_and_invalidates_on_change_or_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let revision = Arc::new(AtomicU64::new(1));
        let current = revision.clone();
        let state = revision.clone();
        let cache = OfflineCache::new(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                if state.load(Ordering::SeqCst) == 2 {
                    Offline::Offline
                } else {
                    Offline::Online
                }
            },
            move || match current.load(Ordering::SeqCst) {
                0 => None,
                n => Some(n),
            },
        );
        assert_eq!(cache.read(), Offline::Online);
        assert_eq!(cache.read(), Offline::Online);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        revision.store(2, Ordering::SeqCst);
        assert_eq!(cache.read(), Offline::Offline);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        revision.store(0, Ordering::SeqCst);
        assert_eq!(cache.read(), Offline::Unavailable);
        revision.store(2, Ordering::SeqCst);
        assert_eq!(cache.read(), Offline::Offline);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        cache.invalidate();
        assert_eq!(cache.read(), Offline::Offline);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn preference_change_during_load_fails_closed() {
        let revision = Arc::new(AtomicU64::new(1));
        let changed = revision.clone();
        let cache = OfflineCache::new(
            move || {
                changed.fetch_add(1, Ordering::SeqCst);
                Offline::Online
            },
            move || Some(revision.load(Ordering::SeqCst)),
        );
        assert_eq!(cache.read(), Offline::Unavailable);
        assert_eq!(cache.read(), Offline::Unavailable);
    }
}
