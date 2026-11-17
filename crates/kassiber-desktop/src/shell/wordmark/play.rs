//! When a mark plays (KassiberWordmark.tsx's `useAnimatedPaths`), as a state machine the poll
//! steps on the frame clock instead of a layout effect and `requestAnimationFrame`.
//!
//! - A reveal (`oneShot`) plays when its mark mounts with `animate` on, or `animate` turns on
//!   again; a remount (React's `key`) plays it afresh. It plays once: finished, it waits for
//!   `animate` to go off. Under reduced motion, or in a hidden window, it is skipped (the mark
//!   shows at rest) and counts as played; hiding the window mid-play ends it at rest.
//! - The working loop plays while mounted, pauses while the window is hidden and resumes where it
//!   was; under reduced motion the K rests.

use super::grid::{Frame, Motion};

/// What the view draws at a mark site now: `None` while the site is not on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Want {
    /// A different key remounts the mark (React's `key`), which replays its reveal.
    pub key: u64,
    /// `animate` (a working mark always animates).
    pub animate: bool,
    pub motion: Motion,
}

/// What the window and the OS say about motion right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// `document.visibilityState === "hidden"`: minimized or fully covered.
    pub hidden: bool,
    pub reduced: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Play {
    /// Fixed for the whole play: a reveal switched mid-play takes effect from the next one.
    motion: Motion,
    /// Clock (seconds) the play started at, shifted on resume so `elapsed` skips the pause.
    start: f64,
    /// Elapsed ms when the window hid the working loop.
    paused: Option<f64>,
}

/// One site's mark.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slot {
    mounted: Option<u64>,
    /// The reveal played since `animate` last turned on (`finishedRef`).
    finished: bool,
    play: Option<Play>,
    /// The frame drawn now; None at rest.
    frame: Option<Frame>,
}

/// What a step did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    /// The drawn frame changed: the view runs again.
    pub changed: bool,
    /// A reveal finished (played, skipped or cut short): `onRevealEnd`.
    pub ended: bool,
    /// Clock (seconds) to step again at; None when nothing plays. A time already past means the
    /// next frame.
    pub wake: Option<f64>,
}

/// While the window hides a working loop, how often to look whether it shows again.
pub const HIDDEN_RECHECK: f64 = 0.25;

impl Slot {
    /// The frame to draw: None at rest.
    pub fn frame(&self) -> Option<&Frame> {
        self.frame.as_ref()
    }

    /// Whether `want` is a mount this slot has not seen yet, so the view draws the first frame
    /// itself rather than one frame of the mark at rest before the poll starts the play (the
    /// web starts it in a layout effect, before paint).
    pub fn pending(&self, want: &Want, env: Env) -> bool {
        if !want.animate || env.reduced || (env.hidden && want.motion.one_shot()) {
            return false;
        }
        match self.mounted {
            Some(key) if key == want.key => {
                self.play.is_none() && !(want.motion.one_shot() && self.finished)
            }
            _ => true,
        }
    }

    pub fn playing(&self) -> bool {
        self.play.is_some()
    }

    /// Brings the slot up to clock `now` (seconds) for what the view draws there.
    pub fn step(&mut self, want: Option<Want>, now: f64, env: Env) -> Step {
        let before = self.frame.clone();
        let mut ended = false;
        match want {
            None => *self = Slot::default(),
            Some(w) => {
                if self.mounted != Some(w.key) {
                    *self = Slot {
                        mounted: Some(w.key),
                        ..Slot::default()
                    };
                }
                ended = self.advance(w, now, env);
            }
        }
        let wake = self.play.as_ref().map(|p| match p.paused {
            Some(_) => now + HIDDEN_RECHECK,
            None => {
                let elapsed = (now - p.start) * 1000.0;
                super::grid::next_change(p.motion, elapsed).map_or(now, |ms| p.start + ms / 1000.0)
            }
        });
        Step {
            changed: self.frame != before,
            ended,
            wake,
        }
    }

    /// Returns whether a reveal ended.
    fn advance(&mut self, w: Want, now: f64, env: Env) -> bool {
        if !w.animate {
            self.finished = false;
            self.play = None;
            self.frame = None;
            return false;
        }
        let one_shot = w.motion.one_shot();
        if self.play.is_none() {
            if one_shot && self.finished {
                return false;
            }
            if one_shot && (env.reduced || env.hidden) {
                self.finished = true;
                return true;
            }
            if env.reduced {
                return false;
            }
            self.play = Some(Play {
                motion: w.motion,
                start: now,
                paused: None,
            });
        }
        let Some(play) = self.play.as_mut() else {
            return false;
        };
        if one_shot {
            let elapsed = (now - play.start) * 1000.0;
            let Motion::Reveal(r) = play.motion else {
                unreachable!()
            };
            if env.reduced || env.hidden || elapsed >= r.ms() {
                self.play = None;
                self.frame = None;
                self.finished = true;
                return true;
            }
            self.frame = Some(play.motion.frame(elapsed));
        } else if env.reduced {
            self.play = None;
            self.frame = None;
        } else if env.hidden {
            if play.paused.is_none() {
                play.paused = Some((now - play.start) * 1000.0);
            }
        } else {
            if let Some(ms) = play.paused.take() {
                play.start = now - ms / 1000.0;
            }
            self.frame = Some(play.motion.frame((now - play.start) * 1000.0));
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::grid::{Reveal, WORKING_CYCLE_MS, reveal, working};
    use super::*;

    const DECRYPT: Want = Want {
        key: 0,
        animate: true,
        motion: Motion::Reveal(Reveal::Decrypt),
    };
    const WORKING: Want = Want {
        key: 0,
        animate: true,
        motion: Motion::Working,
    };
    const SHOWN: Env = Env {
        hidden: false,
        reduced: false,
    };
    const HIDDEN: Env = Env {
        hidden: true,
        reduced: false,
    };
    const REDUCED: Env = Env {
        hidden: false,
        reduced: true,
    };

    #[test]
    fn a_reveal_plays_once_on_mount_then_rests_without_waking() {
        let mut s = Slot::default();
        assert!(s.pending(&DECRYPT, SHOWN));
        let st = s.step(Some(DECRYPT), 10.0, SHOWN);
        assert!(st.changed && !st.ended);
        assert_eq!(s.frame(), Some(&reveal(Reveal::Decrypt, 0.0)));
        assert!(
            st.wake.is_some_and(|w| (w - 10.07).abs() < 1e-6),
            "the first flicker tick: {:?}",
            st.wake
        );
        let st = s.step(Some(DECRYPT), 10.5, SHOWN);
        assert_eq!(s.frame(), Some(&reveal(Reveal::Decrypt, 500.0)));
        assert!(st.changed && st.wake.is_some());
        let st = s.step(Some(DECRYPT), 11.4, SHOWN);
        assert_eq!(
            st,
            Step {
                changed: true,
                ended: true,
                wake: None
            }
        );
        assert_eq!(s.frame(), None);
        // At rest: further polls change nothing and ask for no frame.
        assert_eq!(s.step(Some(DECRYPT), 20.0, SHOWN), Step::default());
        assert!(!s.pending(&DECRYPT, SHOWN));
    }

    #[test]
    fn a_new_key_or_animate_turning_on_again_replays() {
        let mut s = Slot::default();
        s.step(Some(DECRYPT), 0.0, SHOWN);
        s.step(Some(DECRYPT), 2.0, SHOWN);
        let again = Want { key: 1, ..DECRYPT };
        assert!(s.pending(&again, SHOWN));
        s.step(Some(again), 3.0, SHOWN);
        assert!(s.playing());
        s.step(Some(again), 5.0, SHOWN);
        s.step(
            Some(Want {
                animate: false,
                ..again
            }),
            6.0,
            SHOWN,
        );
        s.step(Some(again), 7.0, SHOWN);
        assert!(s.playing(), "animate on again replays");
        // Unmounted and mounted again: a fresh play.
        s.step(Some(again), 9.0, SHOWN);
        s.step(None, 9.5, SHOWN);
        s.step(Some(again), 10.0, SHOWN);
        assert!(s.playing());
    }

    #[test]
    fn hiding_ends_a_reveal_once_and_showing_does_not_replay_it() {
        let mut s = Slot::default();
        s.step(Some(DECRYPT), 0.0, SHOWN);
        let st = s.step(Some(DECRYPT), 0.3, HIDDEN);
        assert_eq!(
            st,
            Step {
                changed: true,
                ended: true,
                wake: None
            }
        );
        assert_eq!(s.step(Some(DECRYPT), 0.6, SHOWN), Step::default());
    }

    #[test]
    fn a_reveal_mounted_hidden_or_under_reduced_motion_is_skipped_and_ends() {
        for env in [HIDDEN, REDUCED] {
            let mut s = Slot::default();
            assert!(!s.pending(&DECRYPT, env));
            assert_eq!(
                s.step(Some(DECRYPT), 0.0, env),
                Step {
                    changed: false,
                    ended: true,
                    wake: None
                }
            );
            // Motion allowed again later: the finished reveal does not replay.
            assert_eq!(s.step(Some(DECRYPT), 1.0, SHOWN), Step::default());
        }
    }

    #[test]
    fn the_working_loop_pauses_while_hidden_and_resumes_where_it_was() {
        let mut s = Slot::default();
        s.step(Some(WORKING), 0.0, SHOWN);
        s.step(Some(WORKING), 0.7, SHOWN);
        assert_eq!(s.frame(), Some(&working(700.0)));
        let st = s.step(Some(WORKING), 1.0, HIDDEN);
        assert!(!st.changed);
        assert_eq!(
            st.wake,
            Some(1.0 + HIDDEN_RECHECK),
            "only a slow visibility check while hidden"
        );
        s.step(Some(WORKING), 30.0, HIDDEN);
        s.step(Some(WORKING), 40.0, SHOWN);
        assert_eq!(
            s.frame(),
            Some(&working(1000.0)),
            "picks up at 1 s in, where it hid"
        );
        // It loops for as long as it is mounted, and unmounting stops it.
        s.step(Some(WORKING), 40.0 + 2.0 * WORKING_CYCLE_MS / 1000.0, SHOWN);
        assert_eq!(s.frame(), Some(&working(1000.0)));
        assert_eq!(s.step(None, 50.0, SHOWN).wake, None);
        assert!(!s.playing());
    }

    #[test]
    fn reduced_motion_rests_the_working_k_and_starts_it_fresh_after() {
        let mut s = Slot::default();
        assert_eq!(s.step(Some(WORKING), 0.0, REDUCED), Step::default());
        assert_eq!(s.frame(), None);
        s.step(Some(WORKING), 1.0, SHOWN);
        assert_eq!(s.frame(), Some(&working(0.0)));
        let st = s.step(Some(WORKING), 1.5, REDUCED);
        assert!(st.changed && st.wake.is_none());
        assert_eq!(s.frame(), None);
    }

    #[test]
    fn a_reveal_switched_mid_play_applies_from_the_next_play() {
        let mut s = Slot::default();
        s.step(Some(DECRYPT), 0.0, SHOWN);
        let assemble = Want {
            motion: Motion::Reveal(Reveal::Assemble),
            ..DECRYPT
        };
        s.step(Some(assemble), 0.5, SHOWN);
        assert_eq!(s.frame(), Some(&reveal(Reveal::Decrypt, 500.0)));
    }
}
