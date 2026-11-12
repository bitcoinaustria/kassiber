//! Opt-in wall-clock spans for performance work (`--features timing`).
//! Without the feature every span is a zero-sized no-op.

#[cfg(feature = "timing")]
mod imp {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    thread_local! {
        static SPANS: RefCell<Vec<(&'static str, Duration)>> = const { RefCell::new(Vec::new()) };
    }

    pub struct Span(&'static str, Instant);

    impl Drop for Span {
        fn drop(&mut self) {
            let elapsed = self.1.elapsed();
            SPANS.with(|spans| spans.borrow_mut().push((self.0, elapsed)));
        }
    }

    pub fn span(name: &'static str) -> Span {
        Span(name, Instant::now())
    }

    /// Recorded spans in completion order; clears the recorder.
    pub fn take() -> Vec<(&'static str, Duration)> {
        SPANS.with(|spans| std::mem::take(&mut *spans.borrow_mut()))
    }
}

#[cfg(feature = "timing")]
pub use imp::{span, take, Span};

/// A disabled span; dropping it does nothing.
#[cfg(not(feature = "timing"))]
pub struct Span;

#[cfg(not(feature = "timing"))]
impl Drop for Span {
    #[inline(always)]
    fn drop(&mut self) {}
}

#[cfg(not(feature = "timing"))]
#[inline(always)]
pub fn span(_name: &'static str) -> Span {
    Span
}
