//! A wall clock for recorded sessions (plan 22, session replay), preloaded
//! into a recorded process and into its replay, never into a product build.
//!
//! With `KASSIBER_TEST_CLOCK_BASE_US` set, `gettimeofday` and `time` read
//! `base + n` microseconds, `n` counting the reads, until the process has
//! run two seconds; after that they also advance with the real elapsed time
//! beyond those two seconds, so a long-running process still sees time pass.
//! These are the calls SQLite's clock (`strftime('now')`, every book's
//! `created_at`) makes, so a short command writes the same times when it is
//! replayed, and so do the hashes a book keeps over them (a pair's
//! `source_row_hash` covers its `created_at`).
//!
//! `clock_gettime` stays real: Rust's `SystemTime` reads it, and code that
//! measures work with it (age's scrypt calibration doubles its cost until a
//! second passes) must see real time. Monotonic clocks are untouched. Linux
//! only.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::OnceLock;

type ClockGettime = unsafe extern "C" fn(libc::clockid_t, *mut libc::timespec) -> libc::c_int;

const GRACE_US: i64 = 2_000_000;

struct State {
    base_us: i64,
    started_monotonic_us: i64,
    real: ClockGettime,
}

static STATE: OnceLock<Option<State>> = OnceLock::new();
static READS: AtomicU64 = AtomicU64::new(0);
static LAST: AtomicI64 = AtomicI64::new(i64::MIN);

fn real_clock_gettime() -> ClockGettime {
    // SAFETY: RTLD_NEXT finds the next definition, libc's; the symbol has
    // clock_gettime's signature.
    unsafe {
        let symbol = libc::dlsym(libc::RTLD_NEXT, c"clock_gettime".as_ptr());
        assert!(!symbol.is_null(), "no clock_gettime behind the clock shim");
        std::mem::transmute::<*mut libc::c_void, ClockGettime>(symbol)
    }
}

fn micros(real: ClockGettime, clock: libc::clockid_t) -> i64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid timespec to write.
    unsafe { real(clock, &mut now) };
    now.tv_sec * 1_000_000 + now.tv_nsec / 1000
}

fn state() -> Option<&'static State> {
    STATE
        .get_or_init(|| {
            let base_us = std::env::var("KASSIBER_TEST_CLOCK_BASE_US")
                .ok()?
                .parse::<i64>()
                .ok()?;
            let real = real_clock_gettime();
            Some(State {
                base_us,
                started_monotonic_us: micros(real, libc::CLOCK_MONOTONIC),
                real,
            })
        })
        .as_ref()
}

/// The session's wall clock in microseconds, strictly increasing.
fn now_us(state: &State) -> i64 {
    let reads = READS.fetch_add(1, Ordering::SeqCst) as i64;
    let elapsed = micros(state.real, libc::CLOCK_MONOTONIC) - state.started_monotonic_us;
    let candidate = state.base_us + reads + (elapsed - GRACE_US).max(0);
    // Never step back when elapsed time starts to count.
    let previous = LAST.fetch_max(candidate, Ordering::SeqCst);
    candidate.max(previous.saturating_add(1))
}

/// The session's wall clock, or the real one without a recorded base.
fn wall_us() -> i64 {
    match state() {
        Some(state) => now_us(state),
        None => micros(real_clock_gettime(), libc::CLOCK_REALTIME),
    }
}

/// # Safety
/// As libc's `gettimeofday`.
#[no_mangle]
pub unsafe extern "C" fn gettimeofday(
    out: *mut libc::timeval,
    _zone: *mut libc::c_void,
) -> libc::c_int {
    let now = wall_us();
    if !out.is_null() {
        // SAFETY: the caller passes a valid timeval.
        unsafe {
            (*out).tv_sec = now.div_euclid(1_000_000);
            (*out).tv_usec = now.rem_euclid(1_000_000);
        }
    }
    0
}

/// # Safety
/// As libc's `time`.
#[no_mangle]
pub unsafe extern "C" fn time(out: *mut libc::time_t) -> libc::time_t {
    let now = wall_us().div_euclid(1_000_000);
    if !out.is_null() {
        // SAFETY: the caller passes a valid time_t.
        unsafe { *out = now };
    }
    now
}
