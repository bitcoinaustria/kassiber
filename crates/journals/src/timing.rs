#[cfg(feature = "timing")]
mod imp {
    use std::{cell::RefCell, time::Instant};

    #[derive(serde::Serialize)]
    pub struct Measurement {
        name: &'static str,
        wall_seconds: f64,
        cpu_seconds: Option<f64>,
    }

    thread_local! {
        static CLOCK: RefCell<Option<fn() -> f64>> = const { RefCell::new(None) };
        static SPANS: RefCell<Vec<Measurement>> = const { RefCell::new(Vec::new()) };
    }

    pub fn set_cpu_clock(clock: fn() -> f64) {
        CLOCK.with(|slot| *slot.borrow_mut() = Some(clock));
    }

    pub struct Span {
        name: &'static str,
        start: Instant,
        cpu: Option<(fn() -> f64, f64)>,
    }

    pub fn span(name: &'static str) -> Span {
        Span {
            name,
            start: Instant::now(),
            cpu: CLOCK.with(|slot| slot.borrow().map(|clock| (clock, clock()))),
        }
    }

    impl Drop for Span {
        fn drop(&mut self) {
            let measurement = Measurement {
                name: self.name,
                wall_seconds: self.start.elapsed().as_secs_f64(),
                cpu_seconds: self.cpu.map(|(clock, start)| clock() - start),
            };
            SPANS.with(|spans| spans.borrow_mut().push(measurement));
        }
    }

    pub fn take() -> Vec<Measurement> {
        SPANS.with(|spans| std::mem::take(&mut *spans.borrow_mut()))
    }
}

#[cfg(feature = "timing")]
pub use imp::*;

#[cfg(not(feature = "timing"))]
pub use kassiber_custody::timing::span;
