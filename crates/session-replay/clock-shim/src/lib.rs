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
//! `clock_gettime`'s realtime clocks read `base + real elapsed time`, so the
//! calendar is the recording's (a balance history "to today", a certificate's
//! validity) while durations stay real: code that measures work with
//! `SystemTime` (age's scrypt calibration doubles its cost until a second
//! passes) still sees time pass at the real rate. A replay of a session
//! recorded without the shared clock passes its recorded start as
//! `KASSIBER_REPLAY_REALTIME_BASE_US`; then `gettimeofday` and `time` read
//! that realtime too. Monotonic clocks are untouched. Linux only.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::OnceLock;

type ClockGettime = unsafe extern "C" fn(libc::clockid_t, *mut libc::timespec) -> libc::c_int;

const GRACE_US: i64 = 2_000_000;

struct State {
    base_us: i64,
    /// `gettimeofday` and `time` count reads (the shared SQLite clock);
    /// otherwise they read the virtual realtime.
    frozen: bool,
    started_monotonic_ns: i128,
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

fn nanos(real: ClockGettime, clock: libc::clockid_t) -> i128 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid timespec to write.
    unsafe { real(clock, &mut now) };
    i128::from(now.tv_sec) * 1_000_000_000 + i128::from(now.tv_nsec)
}

fn micros(real: ClockGettime, clock: libc::clockid_t) -> i64 {
    (nanos(real, clock) / 1000) as i64
}

fn state() -> Option<&'static State> {
    STATE
        .get_or_init(|| {
            let base = |name: &str| std::env::var(name).ok()?.parse::<i64>().ok();
            let (base_us, frozen) = match base("KASSIBER_TEST_CLOCK_BASE_US") {
                Some(base_us) => (base_us, true),
                None => (base("KASSIBER_REPLAY_REALTIME_BASE_US")?, false),
            };
            let real = real_clock_gettime();
            Some(State {
                base_us,
                frozen,
                started_monotonic_ns: nanos(real, libc::CLOCK_MONOTONIC),
                real,
            })
        })
        .as_ref()
}

/// Real time elapsed since the clock started, in nanoseconds.
fn elapsed_ns(state: &State) -> i128 {
    nanos(state.real, libc::CLOCK_MONOTONIC) - state.started_monotonic_ns
}

/// The virtual realtime: the session's base plus real elapsed time.
fn realtime_ns(state: &State) -> i128 {
    i128::from(state.base_us) * 1000 + elapsed_ns(state)
}

/// The session's wall clock in microseconds, strictly increasing.
fn now_us(state: &State) -> i64 {
    if !state.frozen {
        return (realtime_ns(state) / 1000) as i64;
    }
    let reads = READS.fetch_add(1, Ordering::SeqCst) as i64;
    let elapsed = (elapsed_ns(state) / 1000) as i64;
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
/// As libc's `clock_gettime`.
#[no_mangle]
pub unsafe extern "C" fn clock_gettime(
    clock: libc::clockid_t,
    out: *mut libc::timespec,
) -> libc::c_int {
    let realtime = matches!(
        clock,
        libc::CLOCK_REALTIME | libc::CLOCK_REALTIME_COARSE | libc::CLOCK_TAI
    );
    match state().filter(|_| realtime) {
        Some(state) => {
            let now = realtime_ns(state);
            if !out.is_null() {
                // SAFETY: the caller passes a valid timespec.
                unsafe {
                    (*out).tv_sec = now.div_euclid(1_000_000_000) as libc::time_t;
                    (*out).tv_nsec = now.rem_euclid(1_000_000_000) as libc::c_long;
                }
            }
            0
        }
        // SAFETY: as the caller's contract, passed on to libc's.
        None => unsafe { real_clock_gettime()(clock, out) },
    }
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
