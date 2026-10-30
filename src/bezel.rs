//! The black beyond the screen. The island's surface covers the whole
//! output, and the island is one drop of a frame that runs on past the
//! screen's edges; for now only its corners show, rounding the screen off
//! the way the lid around it is rounded.

use cairo::Context;

use crate::island::Rect;

/// The squares at the output's corners the frame paints into (empty when
/// the corners are square).
pub fn corners(w: f64, h: f64, radius: f64) -> Vec<Rect> {
    let r = radius.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return Vec::new();
    }
    [(0.0, 0.0), (w - r, 0.0), (0.0, h - r), (w - r, h - r)]
        .into_iter()
        .map(|(x, y)| Rect { x, y, w: r, h: r })
        .collect()
}

/// Paint the frame's corners: black outside a quarter circle, so a
/// corner reads as the screen's own edge rather than something on it.
/// Painted over whatever is there: what lights the screen inside the arc
/// (an edge's light, the curtain) stays. The caller wipes the squares
/// before each frame's drawing, so the antialiased edge never thickens.
pub fn paint(cr: &Context, w: f64, h: f64, radius: f64) {
    let r = radius.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return;
    }
    cr.save().ok();
    cr.set_operator(cairo::Operator::Over);
    cr.set_source_rgb(0.0, 0.0, 0.0);
    cr.new_path();
    // each corner: from the edge, around the arc, out along the other edge
    // and back through the screen's very corner
    let arcs = [
        ((r, r), (0.0, 0.0), std::f64::consts::PI),
        ((w - r, r), (w, 0.0), 1.5 * std::f64::consts::PI),
        ((w - r, h - r), (w, h), 0.0),
        ((r, h - r), (0.0, h), 0.5 * std::f64::consts::PI),
    ];
    for ((cx, cy), (px, py), start) in arcs {
        cr.move_to(px, py);
        let (sx, sy) = (cx + r * start.cos(), cy + r * start.sin());
        cr.line_to(sx, sy);
        cr.arc(cx, cy, r, start, start + 0.5 * std::f64::consts::PI);
        cr.close_path();
    }
    let _ = cr.fill();
    cr.restore().ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_are_black_outside_the_arc_and_clear_inside() {
        let (w, h, scale) = (200, 120, 1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, (w as f64 * scale) as i32, (h as f64 * scale) as i32).unwrap();
        {
            let cr = Context::new(&surface).unwrap();
            cr.scale(scale, scale);
            paint(&cr, w as f64, h as f64, 16.0);
        }
        let stride = surface.stride() as usize;
        let data = surface.take_data().unwrap();
        let alpha = |x: f64, y: f64| data[(y * scale) as usize * stride + (x * scale) as usize * 4 + 3];
        for (x, y) in [(0.5, 0.5), (199.5, 0.5), (0.5, 119.5), (199.5, 119.5)] {
            assert_eq!(alpha(x, y), 255, "corner pixel at {x},{y}");
        }
        // inside the arc and along the edges between corners: nothing
        for (x, y) in [(10.0, 10.0), (100.0, 0.5), (0.5, 60.0), (190.0, 110.0), (100.0, 60.0)] {
            assert_eq!(alpha(x, y), 0, "pixel at {x},{y}");
        }
        // the diagonal crosses the arc at r(1 - 1/√2) ≈ 4.7 px
        assert!(alpha(3.5, 3.5) > 200 && alpha(6.0, 6.0) < 40);
        let mut edge = Vec::new();
        for i in 0..20 {
            edge.push(alpha(4.0 + i as f64 * 0.05, 4.0 + i as f64 * 0.05));
        }
        assert!(edge.windows(2).all(|p| p[0] >= p[1]), "{edge:?}");
    }

    #[test]
    fn what_lights_the_screen_inside_the_arc_stays() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 100, 100).unwrap();
        {
            let cr = Context::new(&surface).unwrap();
            // a light (or the curtain) already painted into the corner
            cr.set_source_rgba(0.2, 0.6, 1.0, 1.0);
            cr.paint().unwrap();
            paint(&cr, 100.0, 100.0, 16.0);
        }
        let stride = surface.stride() as usize;
        let data = surface.take_data().unwrap();
        let px = |x: usize, y: usize| &data[y * stride + x * 4..y * stride + x * 4 + 4];
        assert_eq!(px(10, 10), &[255, 153, 51, 255], "inside the arc: still lit");
        assert_eq!(px(0, 0), &[0, 0, 0, 255], "the very corner: the frame");
    }

    #[test]
    fn square_corners_paint_nothing() {
        assert!(corners(100.0, 100.0, 0.0).is_empty());
        assert_eq!(corners(100.0, 60.0, 16.0)[3], Rect { x: 84.0, y: 44.0, w: 16.0, h: 16.0 });
    }
}
