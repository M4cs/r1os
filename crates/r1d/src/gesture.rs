//! Turns raw side-button edges and wheel ticks into what the shell acts on.
//!
//! Pure state machine (no I/O, no clock): callers pass `Instant`s in, so it's unit-testable.
//!
//! Button:
//!   - `Button { down }` edges are always forwarded at once, so the UI can react instantly (and
//!     push-to-talk can start capturing on the down edge).
//!   - Released before `hold_ms`: a tap, but only reported after `double_ms` with no second press,
//!     so a quick second press can become a `DoubleTap` instead.
//!   - Held for `hold_ms`: `HoldStart`, then `HoldEnd { held_ms }` on release.
//!   - Tap, then press and hold: `Tap` followed by `HoldStart` (tap-and-hold is not a gesture).
//!   - With a triple press set up (Settings › Controls; [`Gestures::set_triple`]), a double tap waits `double_ms`
//!     for a third press: `TripleTap` if it comes, `DoubleTap` if not. Off by default, so a double tap is instant.
//!
//! Wheel: one tick per detent (`-1` = KEY_UP, `+1` = KEY_DOWN). Ticks within `wheel_guard_ms` of
//! a button edge are dropped and counted: on a test R1 the wheel produced 1–3 ticks at the moment
//! of a button press (docs/open-questions.md Q12).

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raw {
    Wheel(i32),
    Button(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureKind {
    Tap,
    DoubleTap,
    /// Three quick presses: a shortcut, when one is set (r1d's `triple_press` setting).
    TripleTap,
    HoldStart,
    HoldEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Out {
    Wheel(i32),
    Button(bool),
    Gesture { kind: GestureKind, held_ms: Option<u64> },
    /// A wheel tick dropped by the guard window (for diagnostics only).
    WheelSuppressed(i32),
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub hold: Duration,
    pub double: Duration,
    pub wheel_guard: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hold: Duration::from_millis(350),
            double: Duration::from_millis(280),
            wheel_guard: Duration::from_millis(120),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// First press is down. `hold_fired` once `HoldStart` went out.
    Down { at: Instant, hold_fired: bool },
    /// First press was short; waiting to see whether a second press follows.
    WaitSecond { up_at: Instant },
    /// Second press is down (a double tap if it's released before `hold`).
    SecondDown { at: Instant },
    /// Two presses, and a triple press is set up: waiting for a third.
    WaitThird { up_at: Instant },
    /// Third press is down (a triple tap if it's released before `hold`).
    ThirdDown { at: Instant },
}

#[derive(Debug)]
pub struct Gestures {
    cfg: Config,
    state: State,
    last_edge: Option<Instant>,
    /// Waits for a third press after a double tap (a triple-press shortcut is set).
    triple: bool,
}

impl Gestures {
    pub fn new(cfg: Config) -> Self {
        Self { cfg, state: State::Idle, last_edge: None, triple: false }
    }

    /// A triple press is set up (or not any more): double taps wait to see.
    pub fn set_triple(&mut self, on: bool) {
        self.triple = on;
    }

    /// Feeds one raw input received at `now`.
    pub fn input(&mut self, raw: Raw, now: Instant) -> Vec<Out> {
        let mut out = self.tick(now);
        match raw {
            Raw::Wheel(delta) => {
                let guarded = self.last_edge.is_some_and(|t| now.saturating_duration_since(t) < self.cfg.wheel_guard);
                out.push(if guarded { Out::WheelSuppressed(delta) } else { Out::Wheel(delta) });
            }
            Raw::Button(down) => {
                if self.edge_is_duplicate(down) {
                    return out;
                }
                self.last_edge = Some(now);
                out.push(Out::Button(down));
                self.button(down, now, &mut out);
            }
        }
        out
    }

    /// Fires timers due at `now` (hold threshold, double-tap window).
    pub fn tick(&mut self, now: Instant) -> Vec<Out> {
        let mut out = Vec::new();
        match self.state {
            State::Down { at, hold_fired: false } if now >= at + self.cfg.hold => {
                out.push(gesture(GestureKind::HoldStart, None));
                self.state = State::Down { at, hold_fired: true };
            }
            State::WaitSecond { up_at } if now >= up_at + self.cfg.double => {
                out.push(gesture(GestureKind::Tap, None));
                self.state = State::Idle;
            }
            State::SecondDown { at } if now >= at + self.cfg.hold => {
                // Tap, then press and hold: the first press was a tap; this one is a hold.
                out.push(gesture(GestureKind::Tap, None));
                out.push(gesture(GestureKind::HoldStart, None));
                self.state = State::Down { at, hold_fired: true };
            }
            State::WaitThird { up_at } if now >= up_at + self.cfg.double => {
                out.push(gesture(GestureKind::DoubleTap, None));
                self.state = State::Idle;
            }
            State::ThirdDown { at } if now >= at + self.cfg.hold => {
                // A double tap, then press and hold.
                out.push(gesture(GestureKind::DoubleTap, None));
                out.push(gesture(GestureKind::HoldStart, None));
                self.state = State::Down { at, hold_fired: true };
            }
            _ => {}
        }
        out
    }

    /// When `tick` next needs calling, if a timer is pending.
    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            State::Idle | State::Down { hold_fired: true, .. } => None,
            State::Down { at, hold_fired: false } | State::SecondDown { at } | State::ThirdDown { at } => Some(at + self.cfg.hold),
            State::WaitSecond { up_at } | State::WaitThird { up_at } => Some(up_at + self.cfg.double),
        }
    }

    /// A down while already down (or an up while up) can't happen physically; drop it rather than
    /// confuse the state machine (e.g. if a reader restarts mid-press).
    fn edge_is_duplicate(&self, down: bool) -> bool {
        let is_down = matches!(self.state, State::Down { .. } | State::SecondDown { .. } | State::ThirdDown { .. });
        is_down == down
    }

    fn button(&mut self, down: bool, now: Instant, out: &mut Vec<Out>) {
        self.state = match (self.state, down) {
            (State::Idle, true) => State::Down { at: now, hold_fired: false },
            (State::WaitSecond { .. }, true) => State::SecondDown { at: now },
            (State::Down { at, hold_fired: true }, false) => {
                out.push(gesture(GestureKind::HoldEnd, Some(ms(now - at))));
                State::Idle
            }
            (State::Down { hold_fired: false, .. }, false) => State::WaitSecond { up_at: now },
            (State::SecondDown { .. }, false) if self.triple => State::WaitThird { up_at: now },
            (State::SecondDown { .. }, false) => {
                out.push(gesture(GestureKind::DoubleTap, None));
                State::Idle
            }
            (State::WaitThird { .. }, true) => State::ThirdDown { at: now },
            (State::ThirdDown { .. }, false) => {
                out.push(gesture(GestureKind::TripleTap, None));
                State::Idle
            }
            (s, _) => s,
        };
    }
}

fn gesture(kind: GestureKind, held_ms: Option<u64>) -> Out {
    Out::Gesture { kind, held_ms }
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Clock {
        t0: Instant,
        g: Gestures,
        out: Vec<Out>,
    }

    impl Clock {
        fn new() -> Self {
            Self { t0: Instant::now(), g: Gestures::new(Config::default()), out: Vec::new() }
        }
        fn at(&self, ms: u64) -> Instant {
            self.t0 + Duration::from_millis(ms)
        }
        fn press(&mut self, ms: u64, down: bool) {
            let o = self.g.input(Raw::Button(down), self.at(ms));
            self.out.extend(o);
        }
        fn wheel(&mut self, ms: u64, d: i32) {
            let o = self.g.input(Raw::Wheel(d), self.at(ms));
            self.out.extend(o);
        }
        fn tick(&mut self, ms: u64) {
            let o = self.g.tick(self.at(ms));
            self.out.extend(o);
        }
        fn gestures(&self) -> Vec<(GestureKind, Option<u64>)> {
            self.out
                .iter()
                .filter_map(|o| match o {
                    Out::Gesture { kind, held_ms } => Some((*kind, *held_ms)),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn tap_is_reported_after_the_double_window() {
        let mut c = Clock::new();
        c.press(0, true);
        c.press(150, false);
        c.tick(300);
        assert!(c.gestures().is_empty(), "too early: a second press could still come");
        c.tick(150 + 280);
        assert_eq!(c.gestures(), vec![(GestureKind::Tap, None)]);
        assert_eq!(c.g.deadline(), None);
    }

    #[test]
    fn double_tap() {
        let mut c = Clock::new();
        c.press(0, true);
        c.press(120, false);
        c.press(250, true);
        c.press(360, false);
        c.tick(2000);
        assert_eq!(c.gestures(), vec![(GestureKind::DoubleTap, None)]);
    }

    #[test]
    fn triple_tap_only_when_set_up() {
        // Off (the default): three presses are a double tap, then a tap.
        let mut c = Clock::new();
        for (t, d) in [(0, true), (100, false), (200, true), (300, false), (400, true), (500, false)] {
            c.press(t, d);
        }
        c.tick(2000);
        assert_eq!(c.gestures(), vec![(GestureKind::DoubleTap, None), (GestureKind::Tap, None)]);
        // On: a triple tap; and a double tap waits the window, then comes.
        let mut c = Clock::new();
        c.g.set_triple(true);
        for (t, d) in [(0, true), (100, false), (200, true), (300, false), (400, true), (500, false)] {
            c.press(t, d);
        }
        assert_eq!(c.gestures(), vec![(GestureKind::TripleTap, None)]);
        let mut c = Clock::new();
        c.g.set_triple(true);
        c.press(0, true);
        c.press(100, false);
        c.press(200, true);
        c.press(300, false);
        c.tick(500);
        assert!(c.gestures().is_empty(), "a third press could still come");
        c.tick(300 + 280);
        assert_eq!(c.gestures(), vec![(GestureKind::DoubleTap, None)]);
    }

    #[test]
    fn hold_reports_start_then_duration() {
        let mut c = Clock::new();
        c.press(0, true);
        assert_eq!(c.g.deadline(), Some(c.at(350)));
        c.tick(350);
        c.press(1500, false);
        assert_eq!(c.gestures(), vec![(GestureKind::HoldStart, None), (GestureKind::HoldEnd, Some(1500))]);
    }

    #[test]
    fn hold_start_fires_from_the_next_input_too() {
        // No tick in between: the wheel event at 400 ms must still fire the overdue HoldStart first.
        let mut c = Clock::new();
        c.press(0, true);
        c.wheel(400, 1);
        assert_eq!(c.out[1], Out::Gesture { kind: GestureKind::HoldStart, held_ms: None });
        assert_eq!(c.out[2], Out::Wheel(1));
    }

    #[test]
    fn tap_then_hold() {
        let mut c = Clock::new();
        c.press(0, true);
        c.press(100, false);
        c.press(200, true);
        c.tick(550);
        c.press(1000, false);
        assert_eq!(
            c.gestures(),
            vec![(GestureKind::Tap, None), (GestureKind::HoldStart, None), (GestureKind::HoldEnd, Some(800))]
        );
    }

    #[test]
    fn button_edges_are_forwarded_immediately() {
        let mut c = Clock::new();
        c.press(0, true);
        assert_eq!(c.out, vec![Out::Button(true)]);
    }

    #[test]
    fn wheel_ticks_next_to_a_button_edge_are_suppressed() {
        let mut c = Clock::new();
        c.wheel(0, 1);
        c.press(1000, true);
        c.wheel(1000, -1); // same instant as the press: a ghost tick
        c.wheel(1100, -1); // still inside the 120 ms guard
        c.wheel(1200, 1); // outside it
        let wheel: Vec<_> = c.out.iter().filter(|o| matches!(o, Out::Wheel(_) | Out::WheelSuppressed(_))).collect();
        assert_eq!(
            wheel,
            vec![&Out::Wheel(1), &Out::WheelSuppressed(-1), &Out::WheelSuppressed(-1), &Out::Wheel(1)]
        );
    }

    #[test]
    fn duplicate_edges_are_ignored() {
        let mut c = Clock::new();
        c.press(0, false); // up while idle
        c.press(10, true);
        c.press(20, true); // down while down
        assert_eq!(c.out, vec![Out::Button(true)]);
    }
}
