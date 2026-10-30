//! Time-based interpolation: every visible motion of the island is sampled
//! at the moment the frame will actually be on screen (see `FrameClock` in
//! app.rs), not at a fixed table of frames. Times are seconds on the
//! compositor's presentation clock.

pub type Time = f64;

/// Minimum-jerk profile 10t³ − 15t⁴ + 6t⁵ from rest to rest.
pub fn min_jerk(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (10.0 + t * (-15.0 + 6.0 * t))
}

/// Inverse of `min_jerk` on 0..1 (monotonic, so a bisection does).
pub fn inv_min_jerk(y: f64) -> f64 {
    let y = y.clamp(0.0, 1.0);
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if min_jerk(mid) < y {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Position, velocity and acceleration of a mover at one instant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub x: f64,
    pub v: f64,
    pub a: f64,
}

/// A minimum-jerk move that may start in motion: the quintic through the
/// current position, velocity and acceleration that arrives at the target
/// at rest. From rest it is exactly 10t³ − 15t⁴ + 6t⁵ (the reference
/// island's profile); a toggle pressed mid-flight turns the island around
/// without any jolt (position, speed and acceleration all stay continuous).
#[derive(Clone, Debug)]
pub struct Quintic {
    x0: f64,
    v0: f64, // per unit of τ = t / dur
    a0: f64, // per unit of τ²
    c3: f64,
    c4: f64,
    c5: f64,
    target: f64,
    start: Time,
    dur: f64,
}

impl Quintic {
    pub fn at_rest(value: f64, dur: f64) -> Self {
        let mut q = Quintic { x0: value, v0: 0.0, a0: 0.0, c3: 0.0, c4: 0.0, c5: 0.0, target: value, start: 0.0, dur };
        q.retarget(State { x: value, v: 0.0, a: 0.0 }, value, 0.0);
        q
    }

    fn retarget(&mut self, from: State, target: f64, now: Time) {
        let t = self.dur;
        let d = target - from.x;
        let v = from.v * t;
        let a = from.a * t * t;
        self.x0 = from.x;
        self.v0 = v;
        self.a0 = a;
        self.c3 = 10.0 * d - 6.0 * v - 1.5 * a;
        self.c4 = -15.0 * d + 8.0 * v + 1.5 * a;
        self.c5 = 6.0 * d - 3.0 * v - 0.5 * a;
        self.target = target;
        self.start = now;
    }

    pub fn target(&self) -> f64 {
        self.target
    }

    /// Progress in time, 0..1.
    pub fn progress(&self, now: Time) -> f64 {
        if self.dur <= 0.0 {
            return 1.0;
        }
        ((now - self.start) / self.dur).clamp(0.0, 1.0)
    }

    pub fn state(&self, now: Time) -> State {
        let s = self.progress(now);
        if s >= 1.0 {
            return State { x: self.target, v: 0.0, a: 0.0 };
        }
        let (c3, c4, c5) = (self.c3, self.c4, self.c5);
        let x = self.x0 + s * (self.v0 + s * (0.5 * self.a0 + s * (c3 + s * (c4 + s * c5))));
        let dv = self.v0 + s * (self.a0 + s * (3.0 * c3 + s * (4.0 * c4 + s * 5.0 * c5)));
        let da = self.a0 + s * (6.0 * c3 + s * (12.0 * c4 + s * 20.0 * c5));
        State { x, v: dv / self.dur, a: da / (self.dur * self.dur) }
    }

    pub fn is_moving(&self, now: Time) -> bool {
        self.progress(now) < 1.0
    }

    /// Jump to `value` without motion (the start of a fresh move). The
    /// mover is at rest afterwards: `is_moving` is false.
    pub fn set_at(&mut self, value: f64, now: Time) {
        self.retarget(State { x: value, v: 0.0, a: 0.0 }, value, now);
        self.start = now - self.dur; // already arrived
    }

    /// Start moving towards `to` from the current state of motion; a no-op
    /// if that is already the target.
    pub fn set(&mut self, to: f64, now: Time) {
        if self.target == to {
            return;
        }
        let from = self.state(now);
        self.retarget(from, to, now);
    }
}

/// A damped spring (SwiftUI's `response` / `dampingFraction`): the natural
/// motion for anything that follows input — a pointer entering and leaving
/// the box at its edge retargets the spring, never restarts it.
#[derive(Clone, Debug)]
pub struct Spring {
    omega: f64, // undamped angular frequency
    zeta: f64,  // damping ratio, < 1 (underdamped) overshoots a little
    x0: f64,
    v0: f64,
    target: f64,
    start: Time,
}

impl Spring {
    pub fn new(value: f64, response: f64, damping: f64) -> Self {
        Spring { omega: 2.0 * std::f64::consts::PI / response, zeta: damping, x0: value, v0: 0.0, target: value, start: 0.0 }
    }

    pub fn target(&self) -> f64 {
        self.target
    }

    pub fn state(&self, now: Time) -> State {
        let t = (now - self.start).max(0.0);
        let (w, z) = (self.omega, self.zeta);
        let dx = self.x0 - self.target;
        let decay = (-z * w * t).exp();
        if z < 1.0 {
            let wd = w * (1.0 - z * z).sqrt();
            let a = dx;
            let b = (self.v0 + z * w * dx) / wd;
            let (s, c) = (wd * t).sin_cos();
            let x = decay * (a * c + b * s);
            let v = decay * ((b * wd - z * w * a) * c - (a * wd + z * w * b) * s);
            let acc = -2.0 * z * w * v - w * w * x;
            State { x: self.target + x, v, a: acc }
        } else {
            // critically damped
            let a = dx;
            let b = self.v0 + w * dx;
            let x = decay * (a + b * t);
            let v = decay * (b - w * (a + b * t));
            let acc = -2.0 * w * v - w * w * x;
            State { x: self.target + x, v, a: acc }
        }
    }

    pub fn value(&self, now: Time) -> f64 {
        self.state(now).x
    }

    /// Settled within a thousandth of the unit range.
    pub fn is_moving(&self, now: Time) -> bool {
        let s = self.state(now);
        (s.x - self.target).abs() > 1e-3 || s.v.abs() > 1e-2
    }

    pub fn set(&mut self, to: f64, now: Time) {
        if self.target == to {
            return;
        }
        let s = self.state(now);
        self.x0 = s.x;
        self.v0 = s.v;
        self.target = to;
        self.start = now;
    }

    /// Let go of something held by hand: from `x`, moving at `v`, the
    /// spring takes it to `to` with its own feel.
    pub fn release(&mut self, x: f64, v: f64, to: f64, now: Time) {
        self.x0 = x;
        self.v0 = v;
        self.target = to;
        self.start = now;
    }

    /// Retarget with a different feel (a quick, unbouncy way back), from
    /// wherever the motion is now.
    pub fn set_with(&mut self, to: f64, response: f64, damping: f64, now: Time) {
        if self.target == to {
            return;
        }
        let s = self.state(now);
        self.x0 = s.x;
        self.v0 = s.v;
        self.target = to;
        self.start = now;
        self.omega = 2.0 * std::f64::consts::PI / response;
        self.zeta = damping;
    }
}

/// A scroll position: an exponential approach to its target, the way a
/// scroll view eases out. Unlike a spring from rest it starts moving at
/// once, so every wheel notch reads as a push, not a step; notches that
/// arrive while it is moving extend the same glide (the position never
/// jumps, only the target moves on).
#[derive(Clone, Debug)]
pub struct Glide {
    x0: f64,
    target: f64,
    start: Time,
    tau: f64,
}

impl Glide {
    /// `tau`: the time constant — 95 % of the way after 3 τ.
    pub fn new(value: f64, tau: f64) -> Self {
        Glide { x0: value, target: value, start: 0.0, tau }
    }

    pub fn target(&self) -> f64 {
        self.target
    }

    pub fn value(&self, now: Time) -> f64 {
        let t = (now - self.start).max(0.0);
        self.target + (self.x0 - self.target) * (-t / self.tau).exp()
    }

    pub fn is_moving(&self, now: Time) -> bool {
        (self.value(now) - self.target).abs() > 1e-3
    }

    /// Glide to `to` from wherever it is now.
    pub fn set(&mut self, to: f64, now: Time) {
        if self.target == to {
            return;
        }
        self.x0 = self.value(now);
        self.target = to;
        self.start = now;
    }

    /// Glide to `to` with another time constant (keys glide a little
    /// longer than the wheel).
    pub fn set_with(&mut self, to: f64, tau: f64, now: Time) {
        self.x0 = self.value(now);
        self.target = to;
        self.start = now;
        self.tau = tau;
    }

    /// Jump without motion (a list that was rebuilt).
    pub fn reset(&mut self, value: f64) {
        self.x0 = value;
        self.target = value;
    }
}

/// One feel of a [`Morph`]: SwiftUI's perceptual duration and bounce.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feel {
    pub duration: f64,
    pub bounce: f64,
}

impl Feel {
    /// How far past its target this feel swings, as a share of the travel.
    pub fn overshoot(self) -> f64 {
        if self.bounce <= 0.0 {
            return 0.0;
        }
        let z = 1.0 - self.bounce;
        (-std::f64::consts::PI * z / (1.0 - z * z).sqrt()).exp()
    }

    /// This feel with only as much bounce as keeps a swing over `travel`
    /// within `limit`.
    pub fn limited(self, travel: f64, limit: f64) -> Feel {
        if self.bounce <= 0.0 || travel <= 0.0 || travel * self.overshoot() <= limit {
            return self;
        }
        // overshoot = exp(-πζ/√(1-ζ²)) solved for ζ at limit / travel
        let s = (limit.max(0.01) / travel).ln();
        let z = -s / (std::f64::consts::PI.powi(2) + s * s).sqrt();
        Feel { bounce: 1.0 - z, ..self }
    }
}

/// The island's change of shape, as Vorssaint moves its island: growing it
/// swings a little past its size and settles back, shrinking it settles
/// without a swing; a swing never passes `limit` (in the value's own units)
/// and the value lands exactly on its target once within `settle` of it, so
/// edges snapped at rest stay put.
#[derive(Clone, Debug)]
pub struct Morph {
    spring: Spring,
    grow: Feel,
    shrink: Feel,
    limit: f64,
    settle: f64,
}

impl Morph {
    pub fn new(value: f64, grow: Feel, shrink: Feel, limit: f64, settle: f64) -> Self {
        Morph { spring: Spring::new(value, shrink.duration, 1.0), grow, shrink, limit, settle }
    }

    pub fn target(&self) -> f64 {
        self.spring.target
    }

    pub fn state(&self, now: Time) -> State {
        let s = self.spring.state(now);
        if (s.x - self.spring.target).abs() < self.settle && s.v.abs() < self.settle * 20.0 {
            State { x: self.spring.target, v: 0.0, a: 0.0 }
        } else {
            s
        }
    }

    pub fn is_moving(&self, now: Time) -> bool {
        self.state(now).x != self.spring.target
    }

    /// Move to `to` with the feel its direction calls for.
    pub fn set(&mut self, to: f64, now: Time) {
        if self.spring.target == to {
            return;
        }
        let from = self.state(now).x;
        let feel = if to > from { self.grow } else { self.shrink };
        let feel = feel.limited((to - from).abs(), self.limit);
        self.spring.set_with(to, feel.duration, 1.0 - feel.bounce, now);
    }

    /// Be at `value` at once.
    pub fn set_at(&mut self, value: f64, now: Time) {
        self.spring = Spring { x0: value, v0: 0.0, target: value, start: now, ..self.spring.clone() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_jerk_endpoints() {
        assert_eq!(min_jerk(0.0), 0.0);
        assert_eq!(min_jerk(1.0), 1.0);
        assert!((min_jerk(0.5) - 0.5).abs() < 1e-9);
        assert!((inv_min_jerk(min_jerk(0.3)) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn quintic_from_rest_is_min_jerk() {
        let mut q = Quintic::at_rest(0.0, 0.25);
        q.set(1.0, 10.0);
        for i in 0..=10 {
            let s = i as f64 / 10.0;
            assert!((q.state(10.0 + s * 0.25).x - min_jerk(s)).abs() < 1e-9);
        }
        assert!(!q.is_moving(10.3));
    }

    #[test]
    fn quintic_reversal_is_continuous() {
        let mut q = Quintic::at_rest(0.0, 0.25);
        q.set(1.0, 0.0);
        let before = q.state(0.1);
        q.set(0.0, 0.1);
        let after = q.state(0.1);
        assert!((before.x - after.x).abs() < 1e-9);
        assert!((before.v - after.v).abs() < 1e-9);
        assert!((before.a - after.a).abs() < 1e-6);
        let end = q.state(0.35);
        assert!((end.x - 0.0).abs() < 1e-9 && end.v.abs() < 1e-9);
    }

    #[test]
    fn glide_moves_at_once_and_accumulates_without_jumps() {
        let mut g = Glide::new(0.0, 0.08);
        g.set(1.0, 10.0);
        // an ease-out: a third of the way after a fifth of the time
        assert!(g.value(10.04) > 0.35);
        let before = g.value(10.05);
        g.set(g.target() + 1.0, 10.05);
        assert!((g.value(10.05) - before).abs() < 1e-12);
        assert!(g.value(10.06) > before);
        assert!(!g.is_moving(11.0));
        assert!((g.value(11.0) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn spring_settles_with_small_overshoot() {
        let mut s = Spring::new(0.0, 0.4, 0.6);
        s.set(1.0, 0.0);
        let peak = (0..200).map(|i| s.value(i as f64 * 0.005)).fold(0.0, f64::max);
        assert!(peak > 1.02 && peak < 1.15, "peak {peak}");
        assert!(!s.is_moving(3.0));
        assert!((s.value(3.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn morph_swings_growing_settles_shrinking_and_lands_exactly() {
        let grow = Feel { duration: 0.44, bounce: 0.25 };
        let shrink = Feel { duration: 0.30, bounce: 0.0 };
        let mut m = Morph::new(150.0, grow, shrink, 12.0, 0.25);
        m.set(1500.0, 0.0);
        let peak = (0..400).map(|i| m.state(i as f64 * 0.005).x).fold(0.0, f64::max);
        assert!(peak > 1500.5 && peak <= 1512.5, "a long travel swings at most the limit: {peak}");
        assert!(!m.is_moving(2.0) && m.state(2.0).x == 1500.0);
        m.set(150.0, 2.0);
        let low = (0..400).map(|i| m.state(2.0 + i as f64 * 0.005).x).fold(f64::MAX, f64::min);
        assert!(low >= 150.0, "shrinking never passes its size: {low}");
        assert!(!m.is_moving(3.0) && m.state(3.0).x == 150.0);
    }
}
