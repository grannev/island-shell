//! The Touch ID icon, drawn as vectors so that it can transform: the
//! fingerprint's ridges converge into a ring, the ring fills green or red,
//! and a checkmark or a cross draws itself on top (checkmark.circle.fill /
//! xmark.circle.fill), with a scale bounce on success and a shake on a
//! miss — like the Touch ID prompt on a Mac.

use std::f64::consts::PI;

use cairo::Context;

use crate::anim::min_jerk;

/// One ridge of the fingerprint: an elliptical arc, radius in icon units
/// (the icon is 48 units tall), angles in degrees increasing clockwise
/// (270 = top).
struct Ridge {
    r: f64,
    a1: f64,
    a2: f64,
}

/// A stylised fingerprint after SF Symbols' touchid: nested arches over
/// the top, a whorl in the middle, fragments at the bottom.
const RIDGES: [Ridge; 9] = [
    Ridge { r: 3.6, a1: 215.0, a2: 520.0 },
    Ridge { r: 7.6, a1: 130.0, a2: 445.0 },
    Ridge { r: 11.6, a1: 150.0, a2: 405.0 },
    Ridge { r: 11.6, a1: 430.0, a2: 470.0 },
    Ridge { r: 15.6, a1: 120.0, a2: 425.0 },
    Ridge { r: 15.6, a1: 450.0, a2: 480.0 },
    Ridge { r: 19.6, a1: 150.0, a2: 395.0 },
    Ridge { r: 19.6, a1: 410.0, a2: 455.0 },
    Ridge { r: 23.4, a1: 175.0, a2: 365.0 },
];
/// vertical stretch of the fingerprint
const ASPECT: f64 = 1.12;
/// the ring the ridges collapse into (checkmark.circle radius)
const RING_R: f64 = 21.5;
const RIDGE_WIDTH: f64 = 2.5;
const MARK_WIDTH: f64 = 3.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    Check,
    Cross,
}

/// Everything that varies between frames.
pub struct IconState<'a> {
    /// 0 = fingerprint, 1 = ring
    pub morph: f64,
    /// 0..1 the ring's fill
    pub fill: f64,
    /// 0..1 how much of the mark has drawn itself
    pub mark: f64,
    pub kind: Option<Mark>,
    /// ridge / ring colour (the fingerprint's pulse or the result colour)
    pub colour: (f64, f64, f64),
    /// colour of the ridge `i` while breathing (overrides `colour`)
    pub ridge_colour: Option<&'a dyn Fn(usize) -> (f64, f64, f64)>,
    pub alpha: f64,
    /// extra scale (the bounce) and horizontal offset (the shake)
    pub scale: f64,
    pub shake: f64,
}

/// Points of the marks in icon units, centred.
const CHECK: [(f64, f64); 3] = [(-9.5, 0.5), (-3.2, 6.8), (10.0, -7.0)];
const CROSS_A: [(f64, f64); 2] = [(-7.0, -7.0), (7.0, 7.0)];
const CROSS_B: [(f64, f64); 2] = [(7.0, -7.0), (-7.0, 7.0)];

/// Trace the first `k` (0..1) of a polyline's length.
fn trace_partial(cr: &Context, pts: &[(f64, f64)], k: f64) {
    if k <= 0.0 {
        return;
    }
    let total: f64 = pts.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum();
    let mut left = total * k.min(1.0);
    cr.move_to(pts[0].0, pts[0].1);
    for w in pts.windows(2) {
        let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let len = (dx * dx + dy * dy).sqrt();
        if left >= len {
            cr.line_to(w[1].0, w[1].1);
            left -= len;
        } else {
            let f = left / len;
            cr.line_to(w[0].0 + dx * f, w[0].1 + dy * f);
            break;
        }
    }
}

/// Draw the icon centred on (`cx`, `cy`), `size` = height in px of the
/// fingerprint (48 units).
pub fn draw(cr: &Context, cx: f64, cy: f64, size: f64, st: &IconState) {
    let unit = size / 48.0;
    cr.save().unwrap();
    cr.translate(cx + st.shake, cy);
    cr.scale(unit * st.scale, unit * st.scale);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    if st.fill > 0.0 {
        cr.arc(0.0, 0.0, RING_R, 0.0, 2.0 * PI);
        cr.set_source_rgba(st.colour.0, st.colour.1, st.colour.2, st.alpha * st.fill);
        let _ = cr.fill();
    }

    // ridges: each converges into the full ring, the outer ones first
    for (i, ridge) in RIDGES.iter().enumerate() {
        let delay = 0.06 * (RIDGES.len() - 1 - i) as f64;
        let m = min_jerk(((st.morph - delay) / (1.0 - 0.06 * (RIDGES.len() - 1) as f64)).clamp(0.0, 1.0));
        let r = ridge.r + (RING_R - ridge.r) * m;
        let a1 = ridge.a1.to_radians();
        let a2 = (ridge.a2 + (ridge.a1 + 360.0 - ridge.a2) * m).to_radians();
        let aspect = ASPECT + (1.0 - ASPECT) * m;
        cr.save().unwrap();
        cr.scale(1.0, aspect);
        cr.new_path();
        cr.arc(0.0, 0.0, r, a1, a2);
        cr.restore().unwrap();
        let c = match st.ridge_colour {
            Some(f) if m < 1.0 => {
                let rc = f(i);
                (rc.0 + (st.colour.0 - rc.0) * m, rc.1 + (st.colour.1 - rc.1) * m, rc.2 + (st.colour.2 - rc.2) * m)
            }
            _ => st.colour,
        };
        cr.set_source_rgba(c.0, c.1, c.2, st.alpha);
        cr.set_line_width(RIDGE_WIDTH);
        let _ = cr.stroke();
    }

    // the mark draws itself on the filled ring
    if st.mark > 0.0 {
        cr.set_source_rgba(1.0, 1.0, 1.0, st.alpha);
        cr.set_line_width(MARK_WIDTH);
        match st.kind {
            Some(Mark::Check) => {
                cr.new_path();
                trace_partial(cr, &CHECK, st.mark);
                let _ = cr.stroke();
            }
            Some(Mark::Cross) => {
                cr.new_path();
                trace_partial(cr, &CROSS_A, (st.mark * 1.6).min(1.0));
                let _ = cr.stroke();
                cr.new_path();
                trace_partial(cr, &CROSS_B, ((st.mark - 0.375) * 1.6).clamp(0.0, 1.0));
                let _ = cr.stroke();
            }
            None => {}
        }
    }
    cr.restore().unwrap();
}
