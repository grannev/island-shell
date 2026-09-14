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

    pub fn value(&self, now: Time) -> f64 {
        self.state(now).x
    }

    pub fn is_moving(&self, now: Time) -> bool {
        self.progress(now) < 1.0
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
            assert!((q.value(10.0 + s * 0.25) - min_jerk(s)).abs() < 1e-9);
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
    fn spring_settles_with_small_overshoot() {
        let mut s = Spring::new(0.0, 0.4, 0.6);
        s.set(1.0, 0.0);
        let peak = (0..200).map(|i| s.value(i as f64 * 0.005)).fold(0.0, f64::max);
        assert!(peak > 1.02 && peak < 1.15, "peak {peak}");
        assert!(!s.is_moving(3.0));
        assert!((s.value(3.0) - 1.0).abs() < 1e-3);
    }
}
