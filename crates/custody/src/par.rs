//! Order-preserving parallel map for pure per-item work.
//!
//! Callers consume the results in item order and stop where a sequential loop
//! would stop, so evaluating later items early changes nothing: the work is
//! pure, and a panic is only resumed when its item is reached.
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::thread;

/// Items per worker below which the work stays on the calling thread.
const MIN_CHUNK: usize = 64;
/// Upper bound on workers, so a large machine is not monopolized.
const MAX_WORKERS: usize = 1;

/// The outcome of one item: its value, or the panic it raised.
pub(crate) type Outcome<R> = thread::Result<R>;

/// `f` over `items`, in order, using up to [`MAX_WORKERS`] scoped threads.
/// Falls back to the calling thread when threads are unavailable.
pub(crate) fn map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<Outcome<R>> {
    let run = |chunk: &[T]| -> Vec<Outcome<R>> {
        chunk
            .iter()
            .map(|item| catch_unwind(AssertUnwindSafe(|| f(item))))
            .collect()
    };
    if items.len() / MIN_CHUNK <= 1 {
        return run(items);
    }
    static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let cores = *CORES.get_or_init(|| thread::available_parallelism().map_or(1, usize::from));
    let workers = cores.min(MAX_WORKERS).min(items.len() / MIN_CHUNK);
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

/// The item's value, resuming its panic as the sequential loop would raise it.
pub(crate) fn unwrap<R>(outcome: Outcome<R>) -> R {
    outcome.unwrap_or_else(|panic| resume_unwind(panic))
}
