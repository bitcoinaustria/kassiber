//! The two-finger "swipe between pages" decision, apart from the view so it can be tested.
//!
//! It follows Safari and Chrome on macOS. A gesture is a candidate only when the platform
//! reports its phases (a trackpad's fingers touched down) with no modifier held. Its first
//! movement must be clearly horizontal to claim the gesture, so a vertical scroll stays a scroll.
//! It navigates once, when the fingers lift past [`COMMIT`] points of mostly horizontal travel.
//! Momentum never claims or navigates, and turning vertical cancels the swipe. Swiping right
//! (content moves right) goes back, swiping left goes forward.

/// Horizontal travel (points) that commits a swipe when the fingers lift.
pub const COMMIT: f32 = 96.0;
/// The first movement claims the gesture when it is at least this many times wider than tall.
/// It is stricter than the runtime's own axis pick (`|dx| > |dy|`), so a scroll that starts
/// slightly crooked stays a scroll.
pub const CLAIM_RATIO: f32 = 2.0;
/// A release commits only while travel stays at least this many times wider than tall.
pub const COMMIT_RATIO: f32 = 1.5;
/// Seconds without an event after which a tracked swipe counts as lost (the window lost the
/// pointer or focus and the end never came): it is dropped without navigating.
pub const STALE: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Began,
    Moved,
    Ended,
}

/// One wheel event as the decision needs it (from `rune::WheelEvent`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    pub dx: f32,
    pub dy: f32,
    pub phase: Phase,
    /// The content is coasting after the fingers lifted.
    pub momentum: bool,
    /// Shift, ⌘, Ctrl or Option is held.
    pub mods: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Swipe {
    /// No candidate gesture: unphased wheels, other gestures, momentum.
    #[default]
    Idle,
    /// The fingers touched down; the first movement decides.
    Armed { at: f64 },
    /// This gesture is a swipe: travel so far.
    Tracking { dx: f32, dy: f32, at: f64 },
}

/// What a wheel event did: whether the handler takes it (only the claiming delta matters; a
/// gesture the handler holds goes to it whatever it answers), and where to go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub take: bool,
    pub go: Option<Dir>,
}

impl Swipe {
    pub fn step(&mut self, i: Input, now: f64) -> Step {
        let mut out = Step::default();
        if i.momentum {
            // Coasting never starts or finishes a swipe; a swipe still tracked lost its end.
            if matches!(self, Swipe::Tracking { .. }) {
                *self = Swipe::Idle;
            }
            return out;
        }
        if i.phase == Phase::Began {
            *self = if i.mods {
                Swipe::Idle
            } else {
                Swipe::Armed { at: now }
            };
        }
        if i.dx != 0.0 || i.dy != 0.0 {
            *self = match *self {
                Swipe::Armed { .. } if i.dx.abs() >= CLAIM_RATIO * i.dy.abs() && i.dx != 0.0 => {
                    out.take = true;
                    Swipe::Tracking {
                        dx: i.dx,
                        dy: i.dy,
                        at: now,
                    }
                }
                Swipe::Armed { .. } => Swipe::Idle,
                Swipe::Tracking { dx, dy, .. } => {
                    let (dx, dy) = (dx + i.dx, dy + i.dy);
                    // Turned vertical: no swipe this gesture (the rest of it stays held, as a
                    // browser holds a gesture it has taken for history).
                    if dy.abs() > dx.abs() {
                        Swipe::Idle
                    } else {
                        Swipe::Tracking { dx, dy, at: now }
                    }
                }
                Swipe::Idle => Swipe::Idle,
            };
        }
        if i.phase == Phase::Ended {
            if let Swipe::Tracking { dx, dy, .. } = *self
                && dx.abs() >= COMMIT
                && dx.abs() >= COMMIT_RATIO * dy.abs()
            {
                out.go = Some(if dx > 0.0 { Dir::Back } else { Dir::Forward });
            }
            *self = Swipe::Idle;
        }
        out
    }

    /// A tracked swipe whose last event is older than [`STALE`] is dropped. Returns whether it was.
    pub fn expire(&mut self, now: f64) -> bool {
        let at = match *self {
            Swipe::Armed { at } | Swipe::Tracking { at, .. } => at,
            Swipe::Idle => return false,
        };
        if now - at > STALE {
            *self = Swipe::Idle;
            return true;
        }
        false
    }

    /// While a swipe is tracked: its direction and how far toward committing it is (1 = a
    /// release now navigates).
    pub fn progress(&self) -> Option<(Dir, f32)> {
        match *self {
            Swipe::Tracking { dx, dy, .. } if dx != 0.0 => {
                let dir = if dx > 0.0 { Dir::Back } else { Dir::Forward };
                let p = (dx.abs() / COMMIT).min(1.0);
                // Too vertical to commit yet: it shows short of the mark.
                let p = if dx.abs() >= COMMIT_RATIO * dy.abs() {
                    p
                } else {
                    p.min(0.99)
                };
                Some((dir, p))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(dx: f32, dy: f32, phase: Phase) -> Input {
        Input {
            dx,
            dy,
            phase,
            momentum: false,
            mods: false,
        }
    }

    /// A phased gesture: began, `moves`, ended. Returns every navigation it made.
    fn gesture(s: &mut Swipe, moves: &[(f32, f32)]) -> Vec<Dir> {
        let mut t = 0.0;
        let mut go = vec![];
        let mut run = |s: &mut Swipe, i: Input, t: &mut f64| {
            *t += 0.01;
            go.extend(s.step(i, *t).go);
        };
        run(s, ev(0.0, 0.0, Phase::Began), &mut t);
        for &(dx, dy) in moves {
            run(s, ev(dx, dy, Phase::Moved), &mut t);
        }
        run(s, ev(0.0, 0.0, Phase::Ended), &mut t);
        go
    }

    #[test]
    fn a_horizontal_swipe_past_the_mark_navigates_once_on_release() {
        let mut s = Swipe::default();
        assert_eq!(gesture(&mut s, &[(20.0, 1.0); 6]), vec![Dir::Back]);
        assert_eq!(s, Swipe::Idle);
        assert_eq!(gesture(&mut s, &[(-20.0, 0.0); 6]), vec![Dir::Forward]);
    }

    #[test]
    fn nothing_navigates_before_the_fingers_lift() {
        let mut s = Swipe::default();
        s.step(ev(0.0, 0.0, Phase::Began), 0.0);
        for i in 0..20 {
            assert_eq!(
                s.step(ev(30.0, 0.0, Phase::Moved), 0.01 * i as f64).go,
                None
            );
        }
        assert_eq!(s.progress(), Some((Dir::Back, 1.0)));
    }

    #[test]
    fn a_short_swipe_does_not_navigate() {
        let mut s = Swipe::default();
        assert!(gesture(&mut s, &[(20.0, 0.0); 4]).is_empty());
        // Past the mark and back again: released short of it.
        assert!(gesture(&mut s, &[(60.0, 0.0), (60.0, 0.0), (-80.0, 0.0)]).is_empty());
    }

    #[test]
    fn only_a_clearly_horizontal_first_movement_claims_the_gesture() {
        let mut s = Swipe::default();
        s.step(ev(0.0, 0.0, Phase::Began), 0.0);
        // Zero-delta events decide nothing.
        assert!(!s.step(ev(0.0, 0.0, Phase::Moved), 0.0).take);
        assert!(s.step(ev(4.0, 1.0, Phase::Moved), 0.0).take);
        let mut s = Swipe::default();
        s.step(ev(0.0, 0.0, Phase::Began), 0.0);
        // Wider than tall, but not clearly: the page scrolls as before.
        assert!(!s.step(ev(3.0, 2.0, Phase::Moved), 0.0).take);
        assert_eq!(s, Swipe::Idle);
        // A vertical start never becomes a swipe later in the same gesture.
        assert!(gesture(&mut s, &[(0.0, 10.0), (200.0, 0.0)]).is_empty());
        // The claiming delta is the only one that asks to be taken.
        let mut s = Swipe::default();
        s.step(ev(0.0, 0.0, Phase::Began), 0.0);
        assert!(s.step(ev(10.0, 0.0, Phase::Moved), 0.0).take);
        assert!(!s.step(ev(10.0, 0.0, Phase::Moved), 0.0).take);
    }

    #[test]
    fn turning_vertical_cancels_the_swipe() {
        let mut s = Swipe::default();
        assert!(gesture(&mut s, &[(10.0, 0.0), (0.0, 40.0), (200.0, 0.0)]).is_empty());
        // Diagonal travel past the mark is not a swipe either.
        assert!(gesture(&mut s, &[(10.0, 0.0), (90.0, 90.0)]).is_empty());
    }

    #[test]
    fn momentum_never_claims_or_navigates() {
        let mut s = Swipe::default();
        assert_eq!(
            gesture(&mut s, &[(50.0, 0.0), (50.0, 0.0)]),
            vec![Dir::Back]
        );
        // The coast after it: began, moves and its own end, all momentum.
        for phase in [Phase::Began, Phase::Moved, Phase::Moved, Phase::Ended] {
            let st = s.step(
                Input {
                    dx: 40.0,
                    dy: 0.0,
                    phase,
                    momentum: true,
                    mods: false,
                },
                1.0,
            );
            assert_eq!(st, Step::default());
        }
        assert_eq!(s, Swipe::Idle);
        // Momentum arriving while a swipe is tracked means its end was lost: dropped.
        s.step(ev(0.0, 0.0, Phase::Began), 2.0);
        s.step(ev(150.0, 0.0, Phase::Moved), 2.0);
        s.step(
            Input {
                dx: 10.0,
                dy: 0.0,
                phase: Phase::Moved,
                momentum: true,
                mods: false,
            },
            2.1,
        );
        assert_eq!(s.step(ev(0.0, 0.0, Phase::Ended), 2.2).go, None);
    }

    #[test]
    fn unphased_wheels_and_modifiers_never_swipe() {
        // A mouse's horizontal wheel or Shift + wheel: no Began, so never armed.
        let mut s = Swipe::default();
        for _ in 0..20 {
            assert_eq!(s.step(ev(30.0, 0.0, Phase::Moved), 0.0), Step::default());
        }
        // A trackpad swipe with a modifier held.
        let mut s = Swipe::default();
        s.step(
            Input {
                dx: 0.0,
                dy: 0.0,
                phase: Phase::Began,
                momentum: false,
                mods: true,
            },
            0.0,
        );
        assert!(!s.step(ev(200.0, 0.0, Phase::Moved), 0.0).take);
        assert_eq!(s.step(ev(0.0, 0.0, Phase::Ended), 0.0).go, None);
    }

    #[test]
    fn a_lost_swipe_expires_without_navigating() {
        let mut s = Swipe::default();
        s.step(ev(0.0, 0.0, Phase::Began), 0.0);
        s.step(ev(150.0, 0.0, Phase::Moved), 0.1);
        assert!(!s.expire(0.5));
        assert!(s.expire(0.1 + STALE + 0.01));
        assert_eq!(s, Swipe::Idle);
        assert_eq!(s.progress(), None);
        // An end arriving after that navigates nothing.
        assert_eq!(s.step(ev(0.0, 0.0, Phase::Ended), 2.0).go, None);
    }

    #[test]
    fn progress_reports_direction_and_share_of_the_mark() {
        let mut s = Swipe::default();
        s.step(ev(0.0, 0.0, Phase::Began), 0.0);
        s.step(ev(-COMMIT / 2.0, 0.0, Phase::Moved), 0.0);
        assert_eq!(s.progress(), Some((Dir::Forward, 0.5)));
        s.step(ev(-COMMIT, 0.0, Phase::Moved), 0.0);
        assert_eq!(s.progress(), Some((Dir::Forward, 1.0)));
    }
}
