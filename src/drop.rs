//! One shape language for everything that hangs from the screen's edges —
//! the island, the side level, the bar's corner drops: drops of the same
//! black frame. A drop is an axis-aligned outline whose vertices each carry a
//! corner radius; a corner turning one way rounds convexly, the other way it
//! is a concave fillet where the drop leaves an edge. Every corner has
//! continuous curvature (`Corner`, the island's smoothing).

use std::f64::consts::FRAC_PI_2;

use cairo::Context;

/// A vertex of a drop's outline: its point and its corner radius (0: sharp).
pub type Vertex = (f64, f64, f64);

/// How far beyond a fillet an outline runs along the edge it leaves (only
/// so the fillet has room for its smoothing).
const RUN: f64 = 4.0;

/// Trace a closed, axis-aligned outline through `vertices`, rounding each
/// by its radius with `smoothing` (0 = circular arcs). Each corner may take
/// at most half of each edge it shares.
pub fn trace(cr: &Context, vertices: &[Vertex], smoothing: f64) {
    trace_with(cr, vertices.len(), |i| (vertices[i], smoothing), true);
}

/// Trace an open, axis-aligned outline through `vertices`, each rounded by
/// its own (radius, smoothing) as `trace` rounds a closed one; its two ends
/// stay where they are.
pub fn trace_open(cr: &Context, vertices: &[(f64, f64, f64, f64)]) {
    let n = vertices.len();
    trace_with(cr, n, |i| {
        let (x, y, r, s) = vertices[i];
        ((x, y, if i == 0 || i + 1 == n { 0.0 } else { r }), s)
    }, false);
}

fn trace_with(cr: &Context, n: usize, vertex: impl Fn(usize) -> (Vertex, f64), closed: bool) {
    cr.new_path();
    let mut first = true;
    for i in 0..n {
        let ((px, py, _), _) = vertex((i + n - 1) % n);
        let ((x, y, r), smoothing) = vertex(i);
        let ((nx, ny, _), _) = vertex((i + 1) % n);
        let (lin, lout) = ((x - px).hypot(y - py), (nx - x).hypot(ny - y));
        if r <= 0.0 || lin < 1e-9 || lout < 1e-9 {
            if first { cr.move_to(x, y) } else { cr.line_to(x, y) }
            first = false;
            continue;
        }
        let r = r.min(lin / 2.0).min(lout / 2.0);
        let corner = Corner::new(r, smoothing, lin.min(lout) / 2.0);
        let (dx, dy) = ((x - px) / lin, (y - py) / lin);
        let (ox, oy) = ((nx - x) / lout, (ny - y) / lout);
        let (sx, sy) = (x - dx * corner.p, y - dy * corner.p);
        if first { cr.move_to(sx, sy) } else { cr.line_to(sx, sy) }
        first = false;
        // the corner's frame arrives along +x and turns to +y; a turn the
        // other way is the mirror
        corner.trace(cr, x, y, dy.atan2(dx), dx * oy - dy * ox < 0.0);
    }
    if closed {
        cr.close_path();
    }
}

/// The island: hanging from the top edge at `y0` between `x0` and `x1`,
/// down to `y1`; fillets `fillet` where it leaves the edge, corners `r`.
#[allow(clippy::too_many_arguments)]
pub fn notch(cr: &Context, x0: f64, x1: f64, y0: f64, y1: f64, r: f64, fillet: f64, smoothing: f64) {
    trace(cr, &[
        (x0 - RUN * fillet, y0, 0.0),
        (x0, y0, fillet),
        (x0, y1, r),
        (x1, y1, r),
        (x1, y0, fillet),
        (x1 + RUN * fillet, y0, 0.0),
    ], smoothing);
}

/// A drop on the left edge, standing out to `w` between `y0` and `y1`.
pub fn left(cr: &Context, w: f64, y0: f64, y1: f64, r: f64, fillet: f64, smoothing: f64) {
    trace(cr, &[
        (0.0, y0 - RUN * fillet, 0.0),
        (0.0, y0, fillet),
        (w, y0, r),
        (w, y1, r),
        (0.0, y1, fillet),
        (0.0, y1 + RUN * fillet, 0.0),
    ], smoothing);
}

/// A drop in a top corner of a screen `screen_w` wide: `w` out from the
/// side edge (left when `right` is false), `h` down from the top; a fillet
/// where it leaves the top edge and one where it leaves the side, its free
/// corner rounded by `r`.
#[allow(clippy::too_many_arguments)]
pub fn corner(cr: &Context, screen_w: f64, right: bool, w: f64, h: f64, r: f64, fillet: f64, smoothing: f64) {
    let x = |dx: f64| if right { screen_w - dx } else { dx };
    trace(cr, &[
        (x(w + RUN * fillet), 0.0, 0.0),
        (x(w), 0.0, fillet),
        (x(w), h, r),
        (x(0.0), h, fillet),
        (x(0.0), h + RUN * fillet, 0.0),
        (x(0.0), 0.0, 0.0),
    ], smoothing);
}

/// A right-turn corner with continuous curvature: a circular arc of
/// reduced sweep between two cubic Béziers that take the curvature from
/// zero on the straight edge up to 1/r and back (Figma's corner smoothing,
/// after "Desperately seeking squircles"). `smoothing` 0 = plain arc.
pub(crate) struct Corner {
    r: f64,
    /// distance from the corner point along each edge where the curve begins
    p: f64,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    arc_len: f64,
    arc_sweep: f64,
}

impl Corner {
    pub(crate) fn new(r: f64, smoothing: f64, budget: f64) -> Corner {
        // room for the smoothing: p = (1 + s) r must fit the edge
        let s = smoothing.min((budget / r - 1.0).max(0.0)).clamp(0.0, 1.0);
        let p = (1.0 + s) * r;
        let arc_sweep = FRAC_PI_2 * (1.0 - s);
        let arc_len = (arc_sweep / 2.0).sin() * r * std::f64::consts::SQRT_2;
        let alpha = (FRAC_PI_2 - arc_sweep) / 2.0;
        let p3p4 = r * (alpha / 2.0).tan();
        let beta = FRAC_PI_2 / 2.0 * s;
        let c = p3p4 * beta.cos();
        let d = c * beta.tan();
        let b = ((p - arc_len - c - d) / 3.0).max(0.0);
        let a = 2.0 * b;
        Corner { r, p, a, b, c, d, arc_len, arc_sweep }
    }

    /// Trace the corner whose point is (`x`, `y`); in the local frame the
    /// path arrives along +x and leaves along +y (a right turn), `rot`
    /// turns that frame and `mirror` flips it into a left turn.
    pub(crate) fn trace(&self, cr: &Context, x: f64, y: f64, rot: f64, mirror: bool) {
        cr.save().unwrap();
        cr.translate(x, y);
        cr.rotate(rot);
        if mirror {
            cr.scale(1.0, -1.0);
        }
        let (r, p) = (self.r, self.p);
        if self.arc_sweep >= FRAC_PI_2 - 1e-9 {
            cr.arc(-r, r, r, -FRAC_PI_2, 0.0);
        } else {
            let (a, b, c, d) = (self.a, self.b, self.c, self.d);
            let x0 = -p;
            cr.curve_to(x0 + a, 0.0, x0 + a + b, 0.0, x0 + a + b + c, d);
            let half = self.arc_sweep / 2.0;
            cr.arc(-r, r, r, -FRAC_PI_2 / 2.0 - half, -FRAC_PI_2 / 2.0 + half);
            let (sx, sy) = (x0 + a + b + c + self.arc_len, d + self.arc_len);
            cr.curve_to(sx + d, sy + c, sx + d, sy + b + c, sx + d, sy + a + b + c);
        }
        cr.restore().unwrap();
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_of(draw: impl Fn(&Context)) -> impl Fn(usize, usize) -> u8 {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 400, 200).unwrap();
        {
            let cr = Context::new(&surface).unwrap();
            draw(&cr);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.fill().unwrap();
        }
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap().to_vec();
        move |x, y| data[y * stride + x * 4 + 3]
    }

    #[test]
    fn the_notch_is_the_islands_outline() {
        let a = alpha_of(|cr| notch(cr, 100.0, 300.0, 0.0, 32.0, 14.0, 14.0, 0.6));
        assert_eq!(a(200, 16), 255, "the box");
        assert_eq!(a(200, 40), 0, "below it");
        assert_eq!(a(95, 1), 255, "the fillet by the edge");
        assert_eq!(a(87, 12), 0, "under the fillet's curve");
        assert_eq!(a(101, 31), 0, "the rounded bottom corner");
        assert_eq!(a(50, 1), 0, "the edge further out");
    }

    #[test]
    fn a_corner_drop_fills_the_corner_with_fillets_both_ways() {
        let a = alpha_of(|cr| corner(cr, 400.0, false, 120.0, 32.0, 14.0, 14.0, 0.6));
        assert_eq!(a(1, 1), 255);
        assert_eq!(a(60, 16), 255);
        assert_eq!(a(125, 1), 255, "fillet leaving the top edge");
        assert_eq!(a(1, 36), 255, "fillet leaving the left edge");
        assert_eq!(a(12, 44), 0, "under it");
        assert_eq!(a(119, 31), 0, "the free corner is round");
        assert_eq!(a(200, 5), 0);
        let b = alpha_of(|cr| corner(cr, 400.0, true, 120.0, 32.0, 14.0, 14.0, 0.6));
        assert_eq!(b(398, 1), 255);
        assert_eq!(b(274, 1), 255);
        assert_eq!(b(398, 36), 255);
        assert_eq!(b(281, 31), 0);
    }
}
