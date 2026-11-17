//! The main window's frame between launches (Tauri: lib.rs `restore_main_window_frame`).
//!
//! Tauri restores the user's last frame (macOS: AppKit frame autosave; Windows/Linux: the
//! window-state plugin with size, position and maximized), refitted to the connected displays,
//! and otherwise opens at `first_launch_window_size`: 82 % × 88 % of the display, at most
//! 1440 × 960, at least the 980 × 700 minimum, centred. Full screen is never restored: Tauri
//! leaves it out on Windows/Linux and AppKit never records a full-screen frame.
//!
//! Rune: the frame is read from the window's `WindowHandle::placement` (sampled while it runs
//! and once more as the window closes or the app quits) and applied through a window factory
//! (`rune::run_windows`), which is the only place the connected displays are known before the
//! window opens: `WindowOptions::position` / `physical_size` and `App::maximized`.

use std::cell::{Cell, RefCell};

use serde_json::{Value, json};

/// `tauri.conf.json`'s `minWidth` / `minHeight`.
pub const MIN_SIZE: (f32, f32) = (980.0, 700.0);
/// Rune's own default when no display is known (headless and tooling runs keep it).
pub const FALLBACK_SIZE: (f32, f32) = (1440.0, 900.0);

/// The last normal (not maximized, minimized or full-screen) frame and whether the window was
/// maximized: position in physical desktop pixels (the window's outer top-left, the space
/// `WindowOptions::position` takes), size in points (content).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl Frame {
    pub fn read(v: &Value) -> Option<Frame> {
        let int = |k: &str| {
            v[k].as_i64()
                .filter(|n| n.unsigned_abs() < 1 << 24)
                .map(|n| n as i32)
        };
        let size = |k: &str| {
            v[k].as_f64()
                .filter(|n| n.is_finite() && *n >= 100.0 && *n <= 16384.0)
                .map(|n| n as f32)
        };
        Some(Frame {
            x: int("x")?,
            y: int("y")?,
            width: size("width")?,
            height: size("height")?,
            maximized: v["maximized"].as_bool() == Some(true),
        })
    }

    pub fn write(&self) -> Value {
        json!({"x": self.x, "y": self.y, "width": self.width.round() as i64, "height": self.height.round() as i64, "maximized": self.maximized})
    }
}

thread_local! {
    /// The main window's handle (set on the UI thread each time the native window is created).
    static HANDLE: RefCell<Option<rune::WindowHandle>> = const { RefCell::new(None) };
    /// App clock of the last sample (sampling asks AppKit/winit several things; twice a second
    /// is plenty to follow moves and resizes).
    static SAMPLED_AT: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
}

/// `App::on_window`: remember the handle.
pub fn remember_handle(handle: &rune::WindowHandle) {
    HANDLE.with(|h| *h.borrow_mut() = Some(handle.clone()));
}

/// The main window's handle, once the native window exists (the wordmark asks if it's hidden).
pub fn handle() -> Option<rune::WindowHandle> {
    HANDLE.with(|h| h.borrow().clone())
}

/// The frame to remember now, from the live window and the frame remembered so far. None
/// before a window exists (headless runs never have one) or where positions are hidden
/// (Wayland).
pub fn sample(prev: Option<Frame>, clock: Option<f64>) -> Option<Frame> {
    if let Some(now) = clock {
        if now - SAMPLED_AT.with(Cell::get) < 0.5 {
            return prev;
        }
        SAMPLED_AT.with(|c| c.set(now));
    }
    let p = HANDLE.with(|h| h.borrow().as_ref().and_then(rune::WindowHandle::placement))?;
    Some(from_placement(&p, prev))
}

/// What a placement says to remember. A maximized window keeps the frame it restores to (the
/// system's record on Windows, else the last normal frame seen); a minimized or full-screen
/// window changes nothing.
pub fn from_placement(p: &rune::WindowPlacement, prev: Option<Frame>) -> Frame {
    let scale = if p.scale.is_finite() && p.scale > 0.0 {
        p.scale
    } else {
        1.0
    };
    let normal = |(x, y): (i32, i32), (w, h): (u32, u32)| Frame {
        x,
        y,
        width: (f64::from(w) / scale) as f32,
        height: (f64::from(h) / scale) as f32,
        maximized: false,
    };
    if p.full_screen || p.minimized {
        if let Some(prev) = prev {
            return prev;
        }
        return Frame {
            maximized: p.maximized,
            ..normal(p.position, p.content_size)
        };
    }
    if p.maximized {
        let base = match (p.restore, prev) {
            (Some((pos, size)), _) => normal(pos, size),
            (None, Some(prev)) => prev,
            (None, None) => normal(p.position, p.content_size),
        };
        return Frame {
            maximized: true,
            ..base
        };
    }
    normal(p.position, p.content_size)
}

/// Where the window opens: physical position and size, and whether maximized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch {
    pub position: (i32, i32),
    pub physical_size: (u32, u32),
    pub maximized: bool,
}

/// `first_launch_window_size`: most of the display, capped, never below the minimum, never
/// larger than the display.
pub fn first_launch_size((area_w, area_h): (f64, f64), (min_w, min_h): (f64, f64)) -> (f64, f64) {
    let fit = |area: f64, share: f64, min: f64, max: f64| {
        (area * share).min(max).max(min).min(area).round()
    };
    (
        fit(area_w, 0.82, min_w, 1440.0),
        fit(area_h, 0.88, min_h, 960.0),
    )
}

fn rect(d: &rune::Display) -> (f64, f64, f64, f64) {
    (
        f64::from(d.position.0),
        f64::from(d.position.1),
        f64::from(d.size.0),
        f64::from(d.size.1),
    )
}

fn overlap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64 {
    let w = (a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0);
    let h = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

/// The frame to open with on these displays: the saved one refitted to the display it is
/// mostly on (clamped to its size and moved fully onto it), or the first-launch frame centred
/// on the primary display when there is none or it lands on no connected display. None when
/// there are no displays (the platform places the window).
pub fn launch_frame(
    saved: Option<Frame>,
    displays: &[rune::Display],
    min: (f32, f32),
) -> Option<Launch> {
    let primary = displays
        .iter()
        .find(|d| d.primary)
        .or_else(|| displays.first())?;
    let scale_of = |d: &rune::Display| {
        if d.scale.is_finite() && d.scale > 0.0 {
            d.scale
        } else {
            1.0
        }
    };
    let (min_w, min_h) = (f64::from(min.0), f64::from(min.1));
    let fitted = saved.and_then(|f| {
        let on = displays
            .iter()
            .map(|d| {
                let s = scale_of(d);
                (
                    d,
                    overlap(
                        (
                            f64::from(f.x),
                            f64::from(f.y),
                            f64::from(f.width) * s,
                            f64::from(f.height) * s,
                        ),
                        rect(d),
                    ),
                )
            })
            .filter(|(_, area)| *area > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(d, _)| d)?;
        let s = scale_of(on);
        let (dx, dy, dw, dh) = rect(on);
        // Points, within the display and not below the minimum (unless the display is smaller).
        let w = f64::from(f.width).max(min_w).min(dw / s);
        let h = f64::from(f.height).max(min_h).min(dh / s);
        let (pw, ph) = ((w * s).round(), (h * s).round());
        let x = f64::from(f.x).min(dx + dw - pw).max(dx);
        let y = f64::from(f.y).min(dy + dh - ph).max(dy);
        Some(Launch {
            position: (x as i32, y as i32),
            physical_size: (pw as u32, ph as u32),
            maximized: f.maximized,
        })
    });
    Some(fitted.unwrap_or_else(|| {
        let s = scale_of(primary);
        let (dx, dy, dw, dh) = rect(primary);
        let (w, h) = first_launch_size((dw / s, dh / s), (min_w, min_h));
        let (pw, ph) = ((w * s).round(), (h * s).round());
        Launch {
            position: (
                (dx + ((dw - pw) / 2.0).max(0.0)) as i32,
                (dy + ((dh - ph) / 2.0).max(0.0)) as i32,
            ),
            physical_size: (pw as u32, ph as u32),
            maximized: saved.is_some_and(|f| f.maximized),
        }
    }))
}

/// Runs the app: tooling runs exactly as before (`App::run`), a real launch through a window
/// factory that opens the window at its restored frame.
pub fn run(app: rune::App<crate::app::App>) {
    if super::file::tooling_run() {
        return app.run();
    }
    let saved = super::launch_window();
    rune::run_windows(move |displays| {
        let place = launch_frame(saved, displays, MIN_SIZE);
        let app = if place.is_some_and(|p| p.maximized) {
            app.maximized(true)
        } else {
            app
        };
        let mut spec = rune::WindowSpec::from_app("main", app);
        if let Some(p) = place {
            spec.options.position = Some(p.position);
            spec.options.physical_size = Some(p.physical_size);
        }
        vec![spec]
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(position: (i32, i32), size: (u32, u32), scale: f64, primary: bool) -> rune::Display {
        rune::Display {
            id: None,
            name: None,
            position,
            size,
            scale,
            primary,
        }
    }

    fn placement(position: (i32, i32), size: (u32, u32), scale: f64) -> rune::WindowPlacement {
        rune::WindowPlacement {
            position,
            content_position: position,
            content_size: size,
            scale,
            maximized: false,
            minimized: false,
            full_screen: false,
            restore: None,
            display: None,
        }
    }

    #[test]
    fn first_launch_matches_tauri() {
        assert_eq!(
            first_launch_size((1512.0, 982.0), (980.0, 700.0)),
            (1240.0, 864.0)
        );
        assert_eq!(
            first_launch_size((2560.0, 1440.0), (980.0, 700.0)),
            (1440.0, 960.0)
        );
        assert_eq!(
            first_launch_size((1024.0, 768.0), (980.0, 700.0)),
            (980.0, 700.0)
        );
        assert_eq!(
            first_launch_size((800.0, 600.0), (980.0, 700.0)),
            (800.0, 600.0)
        );
        // Centred on the primary display, in physical pixels.
        let ds = [display((0, 0), (3024, 1964), 2.0, true)];
        let l = launch_frame(None, &ds, MIN_SIZE).unwrap();
        assert_eq!(l.physical_size, (2480, 1728));
        assert_eq!(l.position, (272, 118));
        assert!(!l.maximized);
    }

    #[test]
    fn a_saved_frame_reopens_where_it_was() {
        let ds = [
            display((0, 0), (3024, 1964), 2.0, true),
            display((3024, 0), (1920, 1080), 1.0, false),
        ];
        let f = Frame {
            x: 3100,
            y: 50,
            width: 1200.0,
            height: 800.0,
            maximized: false,
        };
        let l = launch_frame(Some(f), &ds, MIN_SIZE).unwrap();
        assert_eq!((l.position, l.physical_size), ((3100, 50), (1200, 800)));
        let f = Frame {
            x: 100,
            y: 100,
            width: 1100.0,
            height: 750.0,
            maximized: true,
        };
        let l = launch_frame(Some(f), &ds, MIN_SIZE).unwrap();
        assert_eq!(
            (l.position, l.physical_size, l.maximized),
            ((100, 100), (2200, 1500), true)
        );
    }

    #[test]
    fn a_saved_frame_is_clamped_to_its_display() {
        let ds = [display((0, 0), (1920, 1080), 1.0, true)];
        // Hanging off the right and bottom, and larger than the screen.
        let f = Frame {
            x: 1500,
            y: 900,
            width: 3000.0,
            height: 400.0,
            maximized: false,
        };
        let l = launch_frame(Some(f), &ds, MIN_SIZE).unwrap();
        assert_eq!(
            l.physical_size,
            (1920, 700),
            "width to the display, height up to the minimum"
        );
        assert_eq!(l.position, (0, 380));
    }

    #[test]
    fn a_frame_on_a_disconnected_display_falls_back_to_the_primary() {
        let ds = [display((0, 0), (1920, 1080), 1.0, true)];
        let f = Frame {
            x: 5000,
            y: 0,
            width: 1200.0,
            height: 800.0,
            maximized: true,
        };
        let l = launch_frame(Some(f), &ds, MIN_SIZE).unwrap();
        assert_eq!(l.physical_size, (1440, 950));
        assert_eq!(l.position, (240, 65));
        assert!(l.maximized, "the maximized state still applies");
        assert_eq!(launch_frame(Some(f), &[], MIN_SIZE), None);
    }

    #[test]
    fn placements_keep_the_normal_frame() {
        let p = placement((10, 20), (2400, 1600), 2.0);
        let normal = from_placement(&p, None);
        assert_eq!(
            normal,
            Frame {
                x: 10,
                y: 20,
                width: 1200.0,
                height: 800.0,
                maximized: false
            }
        );
        let mut max = placement((0, 0), (3024, 1900), 2.0);
        max.maximized = true;
        assert_eq!(
            from_placement(&max, Some(normal)),
            Frame {
                maximized: true,
                ..normal
            }
        );
        // Windows: the system's restore record wins.
        max.restore = Some(((30, 40), (2000, 1400)));
        assert_eq!(
            from_placement(&max, Some(normal)),
            Frame {
                x: 30,
                y: 40,
                width: 1000.0,
                height: 700.0,
                maximized: true
            }
        );
        let mut fs = placement((0, 0), (3024, 1964), 2.0);
        fs.full_screen = true;
        assert_eq!(
            from_placement(&fs, Some(normal)),
            normal,
            "full screen is never recorded"
        );
        let mut min = placement((0, 0), (1, 1), 2.0);
        min.minimized = true;
        assert_eq!(from_placement(&min, Some(normal)), normal);
    }

    #[test]
    fn frames_read_tolerantly() {
        let f = Frame {
            x: -1200,
            y: 30,
            width: 1280.0,
            height: 820.0,
            maximized: true,
        };
        assert_eq!(Frame::read(&f.write()), Some(f));
        assert_eq!(
            Frame::read(&json!({"x": 1, "y": 2, "width": 5, "height": 900})),
            None
        );
        assert_eq!(
            Frame::read(&json!({"x": "1", "y": 2, "width": 1000, "height": 900})),
            None
        );
        assert_eq!(Frame::read(&json!(null)), None);
    }
}
