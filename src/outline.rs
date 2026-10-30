//! The frame's inner edge, as one loop: where the black (the island, the
//! bar's drops, the frame beyond the screen) meets the picture. Light that
//! runs along the frame follows this loop, so it rounds the island and the
//! drops instead of passing under them.
//!
//! The loop starts at the middle of the island's lower edge and runs with
//! the picture on its right: out along the island's bottom, up its right
//! side onto the top edge, round the right drop (down its side, along its
//! bottom, onto the right edge), down, along the bottom, up the left edge,
//! round the left drop, back along the top and down the island's left side.
//! Each vertex has a radius; corners are circular arcs, convex (the light
//! outside them: the island's, the drops' free corners) or concave (inside:
//! fillets and the screen's own corners).

use std::f64::consts::FRAC_PI_2;

use cairo::Context;

use crate::island::Rect;

/// How far light reaches into the picture, and how it dies away.
pub const DEPTH: f64 = 18.0;
const STEP: f64 = 16.0;

#[derive(Clone, Copy, Debug)]
enum Piece {
    /// from (x, y) along the unit (dx, dy) for `len`; the picture lies along
    /// the normal (-dy, dx)
    Line { x: f64, y: f64, dx: f64, dy: f64, len: f64 },
    /// round (cx, cy) by `r` from angle `a0` through `sweep`; `outside`: the
    /// picture is outside the circle (a convex corner of the black)
    Arc { cx: f64, cy: f64, r: f64, a0: f64, sweep: f64, outside: bool },
}

impl Piece {
    fn len(&self) -> f64 {
        match *self {
            Piece::Line { len, .. } => len,
            Piece::Arc { r, sweep, .. } => r * sweep.abs(),
        }
    }
}

/// The island's box (as last drawn) and its corner radius, its fillets.
#[derive(Clone, Copy, Debug)]
pub struct Island {
    pub rect: Rect,
    pub radius: f64,
    pub fillet: f64,
}

/// A bar drop in a top corner: how far it stands out and how tall it is.
#[derive(Clone, Copy, Debug)]
pub struct Drop {
    pub w: f64,
    pub h: f64,
}

pub struct Outline {
    pieces: Vec<(Piece, f64)>,
    /// the loop's length
    pub len: f64,
    /// where the right screen edge begins going down (a notification's
    /// light reaches about this far, and a little further)
    pub right_edge: f64,
}

/// Drops' free corners and fillets.
const DROP_RADIUS: f64 = 14.0;
const DROP_FILLET: f64 = 14.0;

impl Outline {
    /// The loop on a screen `w`×`h` with corners `corner`, the island (or
    /// a point `cx` on the top edge when it is up in the edge) and the
    /// drops (left, right).
    pub fn new(w: f64, h: f64, corner: f64, island: Option<Island>, cx: f64, drops: [Option<Drop>; 2]) -> Outline {
        let mut v: Vec<(f64, f64, f64)> = Vec::new();
        let mut right_edge_vertex = 0;
        match island {
            Some(i) if i.rect.w > 1.0 && i.rect.h > 1.0 => {
                let (x0, x1, y1) = (i.rect.x, i.rect.x + i.rect.w, i.rect.y + i.rect.h);
                v.push((x0 + i.rect.w / 2.0, y1, 0.0));
                v.push((x1, y1, i.radius));
                v.push((x1, 0.0, i.fillet));
            }
            _ => v.push((cx, 0.0, 0.0)),
        }
        match drops[1] {
            Some(d) => {
                v.push((w - d.w, 0.0, DROP_FILLET));
                v.push((w - d.w, d.h, DROP_RADIUS));
                v.push((w, d.h, DROP_FILLET));
            }
            None => v.push((w, 0.0, corner)),
        }
        right_edge_vertex = right_edge_vertex.max(v.len() - 1);
        v.push((w, h, corner));
        v.push((0.0, h, corner));
        match drops[0] {
            Some(d) => {
                v.push((0.0, d.h, DROP_FILLET));
                v.push((d.w, d.h, DROP_RADIUS));
                v.push((d.w, 0.0, DROP_FILLET));
            }
            None => v.push((0.0, 0.0, corner)),
        }
        if let Some(i) = island.filter(|i| i.rect.w > 1.0 && i.rect.h > 1.0) {
            let (x0, y1) = (i.rect.x, i.rect.y + i.rect.h);
            v.push((x0, 0.0, i.fillet));
            v.push((x0, y1, i.radius));
        }
        let n = v.len();
        // each vertex's corner, as far as its neighbours leave it room
        let radius: Vec<f64> = (0..n).map(|i| {
            let (px, py, _) = v[(i + n - 1) % n];
            let (x, y, r) = v[i];
            let (nx, ny, _) = v[(i + 1) % n];
            r.min((x - px).hypot(y - py) / 2.0).min((nx - x).hypot(ny - y) / 2.0).max(0.0)
        }).collect();
        let mut pieces = Vec::new();
        let mut s = 0.0;
        let mut right_edge = 0.0;
        for i in 0..n {
            let (x, y, _) = v[i];
            let (nx, ny, _) = v[(i + 1) % n];
            let len = (nx - x).hypot(ny - y);
            if len < 1e-9 {
                continue;
            }
            let (dx, dy) = ((nx - x) / len, (ny - y) / len);
            // the straight stretch between this corner and the next
            let (r0, r1) = (radius[i], radius[(i + 1) % n]);
            let line = Piece::Line { x: x + dx * r0, y: y + dy * r0, dx, dy, len: (len - r0 - r1).max(0.0) };
            if i == right_edge_vertex {
                right_edge = s;
            }
            s += push(&mut pieces, line, s);
            // then the next vertex's corner
            let j = (i + 1) % n;
            if r1 > 0.0 {
                let (ax, ay, _) = v[(j + 1) % n];
                let olen = (ax - nx).hypot(ay - ny);
                if olen < 1e-9 {
                    continue;
                }
                let (ox, oy) = ((ax - nx) / olen, (ay - ny) / olen);
                let (ccx, ccy) = (nx - dx * r1 + ox * r1, ny - dy * r1 + oy * r1);
                let (p0x, p0y) = (nx - dx * r1, ny - dy * r1);
                let (p1x, p1y) = (nx + ox * r1, ny + oy * r1);
                let a0 = (p0y - ccy).atan2(p0x - ccx);
                let a1 = (p1y - ccy).atan2(p1x - ccx);
                let mut sweep = a1 - a0;
                while sweep > std::f64::consts::PI { sweep -= 2.0 * std::f64::consts::PI; }
                while sweep < -std::f64::consts::PI { sweep += 2.0 * std::f64::consts::PI; }
                // the picture is on the right of travel (normal (-dy, dx)):
                // a centre on that side means the light lies inside
                let inside = (ccx - p0x) * -dy + (ccy - p0y) * dx > 0.0;
                s += push(&mut pieces, Piece::Arc { cx: ccx, cy: ccy, r: r1, a0, sweep, outside: !inside }, s);
            }
        }
        Outline { pieces, len: s, right_edge }
    }

    /// Light the loop in `colour`, its strength at `s` along it given by
    /// `strength`. Returns the rectangles it lit.
    pub fn paint(&self, cr: &Context, colour: (f64, f64, f64), strength: &dyn Fn(f64) -> f64) -> Vec<Rect> {
        let mut lit = Vec::new();
        for &(piece, s0) in &self.pieces {
            match piece {
                Piece::Line { x, y, dx, dy, len } => {
                    if let Some(r) = paint_line(cr, x, y, dx, dy, len, s0, colour, strength) {
                        lit.push(r);
                    }
                }
                Piece::Arc { cx, cy, r, a0, sweep, outside } => {
                    let s = strength(s0 + r * sweep.abs() / 2.0);
                    if s > 0.003 {
                        lit.push(paint_arc(cr, cx, cy, r, a0, sweep, outside, colour, s));
                    }
                }
            }
        }
        lit
    }
}

fn push(pieces: &mut Vec<(Piece, f64)>, piece: Piece, s: f64) -> f64 {
    let len = piece.len();
    if len > 1e-6 {
        pieces.push((piece, s));
    }
    len
}

/// A straight stretch: a gradient along it for the strength, through the
/// falloff turned towards the picture; only its lit part.
#[allow(clippy::too_many_arguments)]
fn paint_line(cr: &Context, x: f64, y: f64, dx: f64, dy: f64, len: f64, s0: f64, colour: (f64, f64, f64), strength: &dyn Fn(f64) -> f64) -> Option<Rect> {
    let n = (len / STEP).ceil().max(1.0) as usize;
    let samples: Vec<(f64, f64)> = (0..=n).map(|i| {
        let t = (i as f64 * len / n as f64).min(len);
        (t, strength(s0 + t))
    }).collect();
    let first = samples.iter().position(|s| s.1 > 0.003)?;
    let last = samples.iter().rposition(|s| s.1 > 0.003).unwrap_or(first);
    let (lo, hi) = (first.saturating_sub(1), (last + 1).min(n));
    let (t0, t1) = (samples[lo].0, samples[hi].0.max(samples[lo].0 + 1.0).min(len.max(samples[lo].0 + 1.0)));
    let gradient = cairo::LinearGradient::new(x + dx * t0, y + dy * t0, x + dx * t1, y + dy * t1);
    for (t, a) in &samples[lo..=hi] {
        gradient.add_color_stop_rgba(((t - t0) / (t1 - t0)).clamp(0.0, 1.0), colour.0, colour.1, colour.2, a.clamp(0.0, 1.0));
    }
    // the falloff column runs along the normal (-dy, dx), into the picture
    let (nx, ny) = (-dy, dx);
    let k = FALLOFF_N as f64 / DEPTH;
    let matrix = cairo::Matrix::new(dx, k * nx, dy, k * ny, -(dx * x + dy * y), -k * (nx * x + ny * y));
    let mask = FALLOFF.with(|f| f.clone());
    mask.set_matrix(matrix);
    // the band: the lit stretch, DEPTH into the picture
    let (ax, ay) = (x + dx * t0, y + dy * t0);
    let (bx, by) = (x + dx * t1 + nx * DEPTH, y + dy * t1 + ny * DEPTH);
    let band = Rect { x: ax.min(bx), y: ay.min(by), w: (ax - bx).abs(), h: (ay - by).abs() };
    cr.save().unwrap();
    cr.rectangle(band.x, band.y, band.w, band.h);
    cr.clip();
    let _ = cr.set_source(&gradient);
    let _ = cr.mask(&mask);
    cr.restore().unwrap();
    Some(band)
}

/// A corner: the light dying away from the arc, outside it (convex) or
/// inside it (concave), in the corner's quarter.
#[allow(clippy::too_many_arguments)]
fn paint_arc(cr: &Context, cx: f64, cy: f64, r: f64, a0: f64, sweep: f64, outside: bool, colour: (f64, f64, f64), s: f64) -> Rect {
    let reach = if outside { r + DEPTH } else { r };
    let gradient = cairo::RadialGradient::new(cx, cy, 0.0, cx, cy, reach);
    let steps = 10;
    if outside {
        gradient.add_color_stop_rgba(0.0, colour.0, colour.1, colour.2, s * falloff(0.0));
        for i in 0..=steps {
            let d = DEPTH * i as f64 / steps as f64;
            gradient.add_color_stop_rgba((r + d) / reach, colour.0, colour.1, colour.2, (s * falloff(d)).clamp(0.0, 1.0));
        }
    } else {
        for i in (0..=steps).rev() {
            let d = r * i as f64 / steps as f64;
            gradient.add_color_stop_rgba((r - d) / reach.max(1e-6), colour.0, colour.1, colour.2, (s * falloff(d)).clamp(0.0, 1.0));
        }
    }
    // the quarter the corner turns through
    let mid = a0 + sweep / 2.0;
    let (sx, sy) = (mid.cos().signum(), mid.sin().signum());
    let quarter = Rect {
        x: if sx > 0.0 { cx } else { cx - reach },
        y: if sy > 0.0 { cy } else { cy - reach },
        w: reach,
        h: reach,
    };
    let _ = FRAC_PI_2;
    cr.save().unwrap();
    cr.rectangle(quarter.x, quarter.y, quarter.w, quarter.h);
    cr.clip();
    let _ = cr.set_source(&gradient);
    let _ = cr.paint();
    cr.restore().unwrap();
    quarter
}

/// Rows of the falloff image (over `DEPTH`).
const FALLOFF_N: usize = 48;

/// How the light dies away `d` px into the picture: a bright line at the
/// edge over a wide soft glow.
pub fn falloff(d: f64) -> f64 {
    if d >= DEPTH {
        return 0.0;
    }
    let a = 0.72 * (-d / 2.0).exp() + 0.30 * (-d / 7.0).exp();
    (a * (1.0 - (d / DEPTH).powi(3))).clamp(0.0, 1.0)
}

thread_local! {
    static FALLOFF: cairo::SurfacePattern = {
        let mut image = cairo::ImageSurface::create(cairo::Format::A8, 1, FALLOFF_N as i32).expect("falloff");
        let stride = image.stride() as usize;
        {
            let mut data = image.data().expect("falloff data");
            for row in 0..FALLOFF_N {
                let d = (row as f64 + 0.5) * DEPTH / FALLOFF_N as f64;
                data[row * stride] = (255.0 * falloff(d)).round() as u8;
            }
        }
        let pattern = cairo::SurfacePattern::create(&image);
        pattern.set_extend(cairo::Extend::Pad);
        pattern.set_filter(cairo::Filter::Fast);
        pattern
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(outline: &Outline, strength: &dyn Fn(f64) -> f64) -> impl Fn(usize, usize) -> u8 + use<> {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 400).unwrap();
        {
            let cr = Context::new(&surface).unwrap();
            outline.paint(&cr, (0.2, 0.6, 1.0), strength);
        }
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap().to_vec();
        move |x, y| data[y * stride + x * 4 + 3]
    }

    fn screen() -> Outline {
        Outline::new(600.0, 400.0, 16.0,
            Some(Island { rect: Rect { x: 250.0, y: 0.0, w: 100.0, h: 32.0 }, radius: 14.0, fillet: 14.0 }),
            300.0, [Some(Drop { w: 120.0, h: 32.0 }), Some(Drop { w: 150.0, h: 32.0 })])
    }

    #[test]
    fn the_loop_rounds_the_island_and_both_drops() {
        let o = screen();
        // roughly: the island's half, the top, round the drops, the screen
        assert!(o.len > 1900.0 && o.len < 2.0 * (600.0 + 400.0) + 600.0, "{}", o.len);
        let all = lit(&o, &|_| 1.0);
        assert!(all(300, 33) > 100, "under the island");
        assert!(all(352, 10) > 100, "beside the island");
        assert!(all(400, 1) > 100, "the top edge between them");
        assert!(all(448, 10) > 100, "beside the right drop");
        assert!(all(520, 34) > 100, "under the right drop");
        assert!(all(598, 200) > 100, "the right edge");
        assert!(all(60, 34) > 100, "under the left drop");
        assert_eq!(all(520, 10), 0, "the drop itself is black, not lit");
        assert_eq!(all(300, 16), 0, "nor the island");
        assert_eq!(all(300, 200), 0, "the middle of the picture");
    }

    #[test]
    fn a_wave_from_the_island_reaches_the_drop_before_the_edge_below_it() {
        let o = screen();
        // light out to 220 px along the loop, both ways from the island
        let near = lit(&o, &|s| if s.min(o.len - s) < 220.0 { 1.0 } else { 0.0 });
        assert!(near(448, 10) > 100, "down the right drop's side");
        assert_eq!(near(598, 200), 0, "not yet down the right edge");
        assert!(o.right_edge > 250.0 && o.right_edge < 500.0, "{}", o.right_edge);
    }
}
