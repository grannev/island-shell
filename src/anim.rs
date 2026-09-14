//! Time-based interpolation: every visible motion of the island is a `Tween`
//! sampled at the compositor's frame rate, not a fixed table of frames.

use std::time::{Duration, Instant};

/// Minimum-jerk profile 10t³ − 15t⁴ + 6t⁵: starts from rest, accelerates,
/// decelerates and stops dead — no overshoot (the reference island's curve).
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    MinJerk,
    /// CSS `cubic-bezier(x1, y1, x2, y2)`; y outside 0..1 overshoots.
    Bezier(f64, f64, f64, f64),
}

impl Ease {
    /// CSS `ease`.
    pub const EASE: Ease = Ease::Bezier(0.25, 0.1, 0.25, 1.0);

    pub fn apply(self, t: f64) -> f64 {
        match self {
            Ease::MinJerk => min_jerk(t),
            Ease::Bezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t.clamp(0.0, 1.0)),
        }
    }
}

fn bezier_axis(p1: f64, p2: f64, s: f64) -> f64 {
    // cubic with P0 = 0, P3 = 1
    let u = 1.0 - s;
    3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s
}

/// y(x) of a CSS cubic-bezier timing function: solve x(s) = x for the
/// parameter s by bisection, then evaluate y(s).
fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if bezier_axis(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier_axis(y1, y2, 0.5 * (lo + hi))
}

/// A scalar moving from `from` to `to` over `dur` with an easing curve.
/// Retargeting mid-flight restarts from the current value, so a toggle
/// pressed during a move turns the island around where it is.
#[derive(Clone, Debug)]
pub struct Tween {
    from: f64,
    to: f64,
    start: Instant,
    dur: Duration,
    ease: Ease,
}

impl Tween {
    pub fn fixed(value: f64, dur: Duration, ease: Ease) -> Self {
        Tween { from: value, to: value, start: Instant::now() - dur, dur, ease }
    }

    /// Progress of the current move, 0..1 in time (before easing).
    pub fn progress(&self, now: Instant) -> f64 {
        if self.dur.is_zero() {
            return 1.0;
        }
        let t = now.saturating_duration_since(self.start).as_secs_f64() / self.dur.as_secs_f64();
        t.clamp(0.0, 1.0)
    }

    pub fn value(&self, now: Instant) -> f64 {
        let k = self.ease.apply(self.progress(now));
        self.from + (self.to - self.from) * k
    }

    pub fn is_moving(&self, now: Instant) -> bool {
        self.from != self.to && self.progress(now) < 1.0
    }

    /// Start moving towards `to` from wherever the value is now; a no-op if
    /// that is already the target.
    pub fn set(&mut self, to: f64, now: Instant) {
        if self.to == to {
            return;
        }
        self.from = self.value(now);
        self.to = to;
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
    }

    #[test]
    fn bezier_is_identity_for_linear() {
        for i in 0..=10 {
            let x = i as f64 / 10.0;
            assert!((cubic_bezier(0.0, 0.0, 1.0, 1.0, x) - x).abs() < 1e-6);
        }
    }

    #[test]
    fn hover_curve_overshoots() {
        let e = Ease::Bezier(0.2, 0.9, 0.3, 1.25);
        let peak = (0..100).map(|i| e.apply(i as f64 / 100.0)).fold(0.0, f64::max);
        assert!(peak > 1.0 && peak < 1.2);
    }
}
