//! The touchscreen: one finger on the island becomes the pointer events it
//! already understands. A finger that lands on something to drag (a
//! slider, the playhead, the timer's ruler, a notification) holds it at
//! once; anywhere else the island waits to see what the finger does — a
//! tap is a click, a long press the right button, and a drag scrolls the
//! content under it, which keeps gliding after a flick and slows down the
//! way iOS lists do.

use std::collections::VecDeque;

use crate::anim::Time;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Past this the finger is moving, not tapping (logical px).
const SLOP: f64 = 10.0;
/// Held this long without moving: the right button.
pub const LONG_PRESS: f64 = 0.45;
/// How quickly a flick dies away (iOS's normal deceleration, ~0.998/ms).
const FLING_TAU: f64 = 0.325;
/// Slower flicks just stop where the finger left the content (px/s).
const FLING_MIN: f64 = 250.0;
const FLING_STOP: f64 = 12.0;
/// The finger's speed is measured over this much of its recent path.
const VELOCITY_WINDOW: f64 = 0.1;
/// A sideways flick lends a swipe this much of its speed (s).
const KICK: f64 = 0.08;
const KICK_MAX: f64 = 160.0;

/// What the island should be told.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    /// the finger is over this point (a pointer would be resting there)
    Hover(f64, f64),
    Press(u32, f64, f64),
    Motion(f64, f64),
    Release(u32, f64, f64),
    /// pointer-axis values: positive scrolls further down / right, so the
    /// content moves with the finger; `stop` is the finger's lift
    Scroll { x: f64, y: f64, dx: f64, dy: f64, stop: bool },
    /// a drag ended: the whole way the finger went
    Swiped { dx: f64, dy: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// down, not moved yet: a tap, a long press or the start of a drag
    Pending,
    /// holding a control
    Grab,
    /// scrolling, locked to one axis
    Pan { sideways: bool },
    /// the long press fired; the rest of the touch is ignored
    Done,
}

struct Finger {
    id: i32,
    start: (f64, f64),
    at: (f64, f64),
    mode: Mode,
    /// a tap only rests the pointer here (no click)
    reveal: bool,
    since: Time,
    path: VecDeque<(Time, f64, f64)>,
}

impl Finger {
    /// The finger's speed over its recent path (px/s).
    fn velocity(&self) -> (f64, f64) {
        let (Some(first), Some(last)) = (self.path.front(), self.path.back()) else { return (0.0, 0.0) };
        let dt = last.0 - first.0;
        if dt < 0.008 {
            return (0.0, 0.0);
        }
        ((last.1 - first.1) / dt, (last.2 - first.2) / dt)
    }
}

/// Content still gliding after a flick.
struct Fling {
    at: (f64, f64),
    /// in pointer-axis units per second
    v: f64,
    sideways: bool,
    last: Time,
}

#[derive(Default)]
pub struct Touch {
    finger: Option<Finger>,
    fling: Option<Fling>,
}

impl Touch {
    /// A finger lands. `grab`: the point holds something to drag;
    /// `reveal`: a tap there should only rest the pointer on it.
    pub fn down(&mut self, id: i32, x: f64, y: f64, grab: bool, reveal: bool, now: Time) -> Vec<Out> {
        // a second finger is not a gesture the island has
        if self.finger.is_some() {
            return Vec::new();
        }
        // a finger on gliding content catches it
        self.fling = None;
        let mode = if grab { Mode::Grab } else { Mode::Pending };
        self.finger = Some(Finger { id, start: (x, y), at: (x, y), mode, reveal, since: now, path: VecDeque::from([(now, x, y)]) });
        let mut out = vec![Out::Hover(x, y)];
        if grab {
            out.push(Out::Press(BTN_LEFT, x, y));
        }
        out
    }

    pub fn motion(&mut self, id: i32, x: f64, y: f64, now: Time) -> Vec<Out> {
        let Some(f) = self.finger.as_mut().filter(|f| f.id == id) else { return Vec::new() };
        f.path.push_back((now, x, y));
        while f.path.len() > 2 && f.path.front().is_some_and(|p| now - p.0 > VELOCITY_WINDOW) {
            f.path.pop_front();
        }
        let (px, py) = f.at;
        let out = match f.mode {
            Mode::Grab => vec![Out::Motion(x, y)],
            Mode::Pending => {
                let (dx, dy) = (x - f.start.0, y - f.start.1);
                if dx.hypot(dy) < SLOP {
                    return Vec::new();
                }
                // the content starts moving from here, without a jump
                f.mode = Mode::Pan { sideways: dx.abs() > dy.abs() };
                Vec::new()
            }
            Mode::Pan { sideways } => {
                let (dx, dy) = if sideways { (px - x, 0.0) } else { (0.0, py - y) };
                vec![Out::Scroll { x: f.start.0, y: f.start.1, dx, dy, stop: false }]
            }
            Mode::Done => Vec::new(),
        };
        f.at = (x, y);
        out
    }

    pub fn up(&mut self, id: i32, now: Time) -> Vec<Out> {
        let Some(f) = self.finger.take_if(|f| f.id == id) else { return Vec::new() };
        let (x, y) = f.at;
        match f.mode {
            Mode::Grab => vec![Out::Release(BTN_LEFT, x, y)],
            Mode::Pending if f.reveal => Vec::new(),
            Mode::Pending => vec![Out::Press(BTN_LEFT, f.start.0, f.start.1), Out::Release(BTN_LEFT, f.start.0, f.start.1)],
            Mode::Pan { sideways } => {
                let swiped = Out::Swiped { dx: x - f.start.0, dy: y - f.start.1 };
                // the finger may have rested before it lifted
                let fresh = f.path.back().is_some_and(|p| now - p.0 < VELOCITY_WINDOW);
                let (vx, vy) = if fresh { f.velocity() } else { (0.0, 0.0) };
                let v = -if sideways { vx } else { vy };
                let (sx, sy) = f.start;
                if sideways {
                    // a swipe decides on lift: a flick carries it a bit further
                    let kick = (v * KICK).clamp(-KICK_MAX, KICK_MAX);
                    return vec![Out::Scroll { x: sx, y: sy, dx: kick, dy: 0.0, stop: true }, swiped];
                }
                if v.abs() >= FLING_MIN {
                    self.fling = Some(Fling { at: (sx, sy), v, sideways, last: now });
                }
                vec![swiped]
            }
            Mode::Done => Vec::new(),
        }
    }

    /// The compositor took the touch over: let go of whatever is held.
    pub fn cancel(&mut self) -> Vec<Out> {
        self.fling = None;
        let Some(f) = self.finger.take() else { return Vec::new() };
        match f.mode {
            Mode::Grab => vec![Out::Release(BTN_LEFT, f.at.0, f.at.1)],
            Mode::Pan { sideways: true } => vec![Out::Scroll { x: f.start.0, y: f.start.1, dx: 0.0, dy: 0.0, stop: true }],
            _ => Vec::new(),
        }
    }

    /// The finger has rested since `down`: the right button, if it has
    /// not moved.
    pub fn long_press(&mut self, id: i32, now: Time) -> Vec<Out> {
        let Some(f) = self.finger.as_mut().filter(|f| f.id == id) else { return Vec::new() };
        if f.mode != Mode::Pending || now - f.since < LONG_PRESS - 0.02 {
            return Vec::new();
        }
        f.mode = Mode::Done;
        let (x, y) = f.start;
        vec![Out::Press(BTN_RIGHT, x, y), Out::Release(BTN_RIGHT, x, y)]
    }

    /// Is a finger down?
    pub fn active(&self) -> bool {
        self.finger.is_some()
    }

    /// Is this the finger the island follows?
    pub fn holds(&self, id: i32) -> bool {
        self.finger.as_ref().is_some_and(|f| f.id == id)
    }

    /// Is this finger still undecided (a long press may come)?
    pub fn pending(&self, id: i32) -> bool {
        self.finger.as_ref().is_some_and(|f| f.id == id && f.mode == Mode::Pending)
    }

    pub fn flinging(&self) -> bool {
        self.fling.is_some()
    }

    /// The glide for this frame: the exact distance an exponentially
    /// slowing flick covers since the last one.
    pub fn step(&mut self, now: Time) -> Option<Out> {
        let fling = self.fling.as_mut()?;
        let dt = (now - fling.last).clamp(0.0, 0.1);
        fling.last = now;
        let decay = (-dt / FLING_TAU).exp();
        let d = fling.v * FLING_TAU * (1.0 - decay);
        fling.v *= decay;
        let (x, y) = fling.at;
        let out = if fling.sideways {
            Out::Scroll { x, y, dx: d, dy: 0.0, stop: false }
        } else {
            Out::Scroll { x, y, dx: 0.0, dy: d, stop: false }
        };
        if fling.v.abs() < FLING_STOP {
            self.fling = None;
        }
        Some(out)
    }

    /// Stop gliding (the content went away).
    pub fn stop(&mut self) {
        self.fling = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_clicks_where_it_landed() {
        let mut t = Touch::default();
        assert_eq!(t.down(1, 100.0, 20.0, false, false, 0.0), vec![Out::Hover(100.0, 20.0)]);
        // a little wobble is still a tap
        assert!(t.motion(1, 104.0, 22.0, 0.05).is_empty());
        assert_eq!(t.up(1, 0.1), vec![Out::Press(BTN_LEFT, 100.0, 20.0), Out::Release(BTN_LEFT, 100.0, 20.0)]);
        assert!(!t.active());
    }

    #[test]
    fn a_reveal_tap_only_rests_the_pointer() {
        let mut t = Touch::default();
        assert_eq!(t.down(1, 10.0, 10.0, false, true, 0.0), vec![Out::Hover(10.0, 10.0)]);
        assert!(t.up(1, 0.1).is_empty());
    }

    #[test]
    fn a_grab_presses_at_once_and_follows() {
        let mut t = Touch::default();
        assert_eq!(t.down(1, 50.0, 50.0, true, false, 0.0), vec![Out::Hover(50.0, 50.0), Out::Press(BTN_LEFT, 50.0, 50.0)]);
        assert_eq!(t.motion(1, 52.0, 50.0, 0.01), vec![Out::Motion(52.0, 50.0)]);
        assert_eq!(t.up(1, 0.02), vec![Out::Release(BTN_LEFT, 52.0, 50.0)]);
    }

    #[test]
    fn a_drag_scrolls_the_content_with_the_finger() {
        let mut t = Touch::default();
        t.down(1, 100.0, 300.0, false, false, 0.0);
        // past the slop it locks to the vertical axis without a jump
        assert!(t.motion(1, 101.0, 280.0, 0.02).is_empty());
        // the finger goes up 30 px: the list scrolls down by as much
        assert_eq!(t.motion(1, 108.0, 250.0, 0.04), vec![Out::Scroll { x: 100.0, y: 300.0, dx: 0.0, dy: 30.0, stop: false }]);
        // it rested before lifting: no glide
        assert_eq!(t.up(1, 0.5), vec![Out::Swiped { dx: 8.0, dy: -50.0 }]);
        assert!(!t.flinging());
    }

    #[test]
    fn a_flick_glides_and_slows_down() {
        let mut t = Touch::default();
        t.down(1, 100.0, 400.0, false, false, 0.0);
        let mut y = 400.0;
        for i in 1..=6 {
            y -= 20.0;
            t.motion(1, 100.0, y, i as f64 * 0.016);
        }
        t.up(1, 0.1);
        assert!(t.flinging());
        let mut total = 0.0;
        let mut last = f64::MAX;
        let mut now = 0.1;
        while t.flinging() {
            now += 1.0 / 60.0;
            let Some(Out::Scroll { dy, .. }) = t.step(now) else { panic!("a glide scrolls") };
            assert!(dy > 0.0 && dy <= last);
            last = dy;
            total += dy;
        }
        // ~1250 px/s carries the list about v·τ further
        assert!((total - 1250.0 * FLING_TAU).abs() < 40.0, "{total}");
        // a new finger catches it
        t.up(1, 0.0);
        t.fling = Some(Fling { at: (0.0, 0.0), v: 900.0, sideways: false, last: 0.0 });
        t.down(2, 0.0, 0.0, false, false, 0.0);
        assert!(!t.flinging());
    }

    #[test]
    fn a_sideways_swipe_stops_on_lift_with_a_kick() {
        let mut t = Touch::default();
        t.down(1, 100.0, 40.0, false, false, 0.0);
        t.motion(1, 120.0, 41.0, 0.016);
        assert_eq!(t.motion(1, 140.0, 42.0, 0.032), vec![Out::Scroll { x: 100.0, y: 40.0, dx: -20.0, dy: 0.0, stop: false }]);
        let out = t.up(1, 0.04);
        let Out::Scroll { dx, stop: true, .. } = out[0] else { panic!("{out:?}") };
        assert!(dx < 0.0 && dx >= -KICK_MAX);
        assert_eq!(out[1], Out::Swiped { dx: 40.0, dy: 2.0 });
    }

    #[test]
    fn a_long_press_is_the_right_button() {
        let mut t = Touch::default();
        t.down(1, 30.0, 12.0, false, false, 0.0);
        assert!(t.long_press(1, 0.2).is_empty());
        assert_eq!(t.long_press(1, LONG_PRESS), vec![Out::Press(BTN_RIGHT, 30.0, 12.0), Out::Release(BTN_RIGHT, 30.0, 12.0)]);
        assert!(t.motion(1, 90.0, 12.0, 0.6).is_empty());
        assert!(t.up(1, 0.7).is_empty());
        // a finger that moved first never long-presses
        t.down(2, 0.0, 0.0, false, false, 1.0);
        t.motion(2, 0.0, 30.0, 1.1);
        assert!(t.long_press(2, 1.0 + LONG_PRESS).is_empty());
    }

    #[test]
    fn a_second_finger_is_ignored() {
        let mut t = Touch::default();
        t.down(1, 0.0, 0.0, false, false, 0.0);
        assert!(t.down(2, 50.0, 50.0, false, false, 0.01).is_empty());
        assert!(t.up(2, 0.02).is_empty());
        assert!(t.active());
    }
}
