//! Order-preserving parallel map for pure work (after `kassiber-custody`'s
//! `par` module).
//!
//! Callers consume the results in item order and stop where a sequential
//! loop would stop, so evaluating later items early changes nothing: the
//! work is pure, and a panic is only resumed when its item is reached.
//! The engine keeps no state between calls; these threads end before the
//! call returns.

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::thread;

/// Upper bound on workers, so a large machine is not monopolized.
const MAX_WORKERS: usize = 8;

thread_local! {
    /// Set inside [`sequential`]: every map of this thread stays on it.
    static SEQUENTIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` with every parallel map it reaches on the calling thread. The
/// results are the same; a caller that counts process CPU time saves the
/// workers' overhead.
pub fn sequential<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SEQUENTIAL.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(SEQUENTIAL.with(|flag| flag.replace(true)));
    f()
}

/// The outcome of one item: its value, or the panic it raised.
pub(crate) type Outcome<R> = thread::Result<R>;

/// `f` over `items`, in order, using up to [`MAX_WORKERS`] scoped threads
/// with at least `min_chunk` items each. Falls back to the calling thread
/// when one worker would do or threads are unavailable.
pub(crate) fn map<T: Sync, R: Send>(
    items: &[T],
    min_chunk: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<Outcome<R>> {
    let run = |chunk: &[T]| -> Vec<Outcome<R>> {
        chunk
            .iter()
            .map(|item| catch_unwind(AssertUnwindSafe(|| f(item))))
            .collect()
    };
    let workers = workers(items.len(), min_chunk);
    if workers <= 1 {
        return run(items);
    }
    let size = items.len().div_ceil(workers);
    thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(size)
            .map(|chunk| {
                let run = &run;
                (
                    chunk,
                    thread::Builder::new()
                        .spawn_scoped(scope, move || run(chunk))
                        .ok(),
                )
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|(chunk, handle)| match handle {
                Some(handle) => handle.join().unwrap_or_else(|panic| resume_unwind(panic)),
                None => run(chunk),
            })
            .collect()
    })
}

/// `f` over contiguous chunks of `items`, one per worker, in order, with
/// at least `min_chunk` items each; one chunk on the calling thread when
/// one worker would do or threads are unavailable.
pub(crate) fn chunks<T: Sync, R: Send>(
    items: &[T],
    min_chunk: usize,
    f: impl Fn(&[T]) -> R + Sync,
) -> Vec<Outcome<R>> {
    let workers = workers(items.len(), min_chunk);
    if workers <= 1 {
        return vec![catch_unwind(AssertUnwindSafe(|| f(items)))];
    }
    let size = items.len().div_ceil(workers);
    let chunks: Vec<&[T]> = items.chunks(size).collect();
    map(&chunks, 1, |chunk| f(chunk))
}

/// How many workers `len` items of at least `min_chunk` each keep busy.
fn workers(len: usize, min_chunk: usize) -> usize {
    if SEQUENTIAL.with(std::cell::Cell::get) {
        return 1;
    }
    static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let cores = *CORES.get_or_init(|| thread::available_parallelism().map_or(1, usize::from));
    cores.min(MAX_WORKERS).min(len / min_chunk.max(1)).max(1)
}

/// The item's value, resuming its panic as the sequential loop would raise it.
pub(crate) fn unwrap<R>(outcome: Outcome<R>) -> R {
    outcome.unwrap_or_else(|panic| resume_unwind(panic))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_item_order_and_panics_wait_for_their_item() {
        let items: Vec<usize> = (0..5_000).collect();
        let doubled: Vec<usize> = map(&items, 16, |&i| i * 2)
            .into_iter()
            .map(unwrap)
            .collect();
        assert_eq!(doubled, items.iter().map(|i| i * 2).collect::<Vec<_>>());
        let joined: Vec<usize> = chunks(&items, 16, |chunk| chunk.to_vec())
            .into_iter()
            .flat_map(unwrap)
            .collect();
        assert_eq!(joined, items);
        // A panic in a later item does not surface before an earlier error.
        let outcomes = map(&items, 16, |&i| {
            if i == 4_000 {
                panic!("late");
            }
            if i == 10 {
                Err(i)
            } else {
                Ok(i)
            }
        });
        let first_error = outcomes.into_iter().map(unwrap).find_map(Result::err);
        assert_eq!(first_error, Some(10));
    }

    #[test]
    fn sequential_maps_stay_on_the_calling_thread_and_keep_results() {
        let items: Vec<usize> = (0..5_000).collect();
        let caller = thread::current().id();
        let threads = |items: &[usize]| -> Vec<thread::ThreadId> {
            map(items, 16, |_| thread::current().id())
                .into_iter()
                .map(unwrap)
                .collect()
        };
        let inside = sequential(|| {
            assert_eq!(chunks(&items, 16, |chunk| chunk.len()).len(), 1);
            threads(&items)
        });
        assert!(inside.iter().all(|id| *id == caller));
        // The scope ends with `sequential`, also when it unwinds.
        let _ = catch_unwind(|| sequential(|| panic!("inside")));
        assert!(!SEQUENTIAL.with(std::cell::Cell::get));
        if workers(items.len(), 16) > 1 {
            assert!(threads(&items).iter().any(|id| *id != caller));
        }
        let doubled = sequential(|| map(&items, 16, |&i| i * 2));
        let doubled: Vec<usize> = doubled.into_iter().map(unwrap).collect();
        assert_eq!(doubled, items.iter().map(|i| i * 2).collect::<Vec<_>>());
    }
}
