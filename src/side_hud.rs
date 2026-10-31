//! The volume / brightness level, out of the screen's left edge: a drop of
//! the same black frame as the island, hanging from the left edge (concave
//! fillets where it meets it) half way up the screen's upper half, as the
//! island hangs from the top. It is the island's old level pill turned on
//! its side: the symbol at the top, a vertical slider rising from the
//! bottom, the percentage under it. It grows out of the edge on a spring,
//! swells a touch with each change, and draws back in a moment after the
//! last one. It takes no input.

use cairo::Context;

use crate::activity::SliderKind;
use crate::anim::{Spring, Time};
use crate::island::{Font, Rect, Renderer};

/// How far it stands out from the edge, how tall it is, its outer corners
/// and the fillets where it meets the edge.
const WIDTH: f64 = 56.0;
const HEIGHT: f64 = 220.0;
const RADIUS: f64 = crate::menu_style::tight(24.0);
const FILLET: f64 = crate::menu_style::tight(14.0);
/// The slider inside: its width and where it runs.
const TRACK_W: f64 = 6.0;
const TRACK_TOP: f64 = 50.0;
const TRACK_BOTTOM: f64 = 40.0;
/// How long it stays after the last change.
pub const HOLD: f64 = 1.45;
/// A change swells it this much (px), dying away this fast.
const SWELL: f64 = 3.0;
const SWELL_TAU: f64 = 0.16;

pub struct SideHud {
    kind: SliderKind,
    muted: bool,
    /// how far out it is, 0..1 (a little past 1 while settling)
    out: Spring,
    value: Spring,
    updated: Time,
    shown: bool,
}

impl Default for SideHud {
    fn default() -> Self {
        SideHud {
            kind: SliderKind::Volume,
            muted: false,
            out: Spring::new(0.0, 0.42, 0.7),
            value: Spring::new(0.0, 0.2, 1.0),
            updated: f64::NEG_INFINITY,
            shown: false,
        }
    }
}

impl SideHud {
    /// A level changed: out it comes (or stays), and follows it.
    pub fn show(&mut self, kind: SliderKind, value: f64, muted: bool, now: Time) {
        let fresh = !self.shown || self.out.value(now) < 0.05;
        if fresh || kind != self.kind {
            // a new level starts where it is, not from the last one's
            self.value = Spring::new(value.clamp(0.0, 1.0), 0.2, 1.0);
        }
        self.kind = kind;
        self.muted = muted;
        self.value.set(value.clamp(0.0, 1.0), now);
        self.out.set(1.0, now);
        self.updated = now;
        self.shown = true;
    }

    /// Time to go? Returns whether anything changed.
    pub fn advance(&mut self, now: Time) -> bool {
        if self.shown && now - self.updated >= HOLD {
            self.shown = false;
            self.out.set(0.0, now);
            return true;
        }
        false
    }

    /// When it is due to slide away.
    pub fn deadline(&self) -> Option<Time> {
        self.shown.then_some(self.updated + HOLD)
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.out.is_moving(now) || self.value.is_moving(now) || (self.shown && now - self.updated < 5.0 * SWELL_TAU)
    }

    /// Paint it on a screen `h` tall; returns what it covered.
    pub fn draw(&self, cr: &Context, renderer: &Renderer, h: f64, now: Time) -> Option<Rect> {
        let out = self.out.value(now).max(0.0);
        if out <= 0.005 && !self.shown {
            return None;
        }
        let swell = if self.shown { SWELL * (-(now - self.updated).max(0.0) / SWELL_TAU).exp() } else { 0.0 };
        // it grows out of the edge and opens up as it comes, overshooting
        // a touch on its spring
        let w = WIDTH * out + swell;
        let tall = HEIGHT * (0.45 + 0.55 * out.min(1.05)) + 2.0 * swell;
        let mid = h / 4.0;
        let (y0, y1) = (mid - tall / 2.0, mid + tall / 2.0);
        let k = (w / (FILLET + RADIUS)).min(1.0);
        let (f, r) = (FILLET * k, RADIUS * k);
        cr.save().unwrap();
        crate::drop::left(cr, w, y0, y1, r, f, 0.6);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.fill();
        // the content comes in once it has most of its size
        let alpha = ((out - 0.55) / 0.4).clamp(0.0, 1.0);
        if alpha > 0.002 {
            let cx = w / 2.0;
            let value = if self.muted { 0.0 } else { self.value.value(now).clamp(0.0, 1.0) };
            crate::activity::draw_osd_symbol(cr, renderer, &self.kind, cx, y0 + 26.0, alpha, self.muted, value);
            // the slider, rising from the bottom
            let (tx, ty0, ty1) = (cx - TRACK_W / 2.0, y0 + TRACK_TOP, y1 - TRACK_BOTTOM);
            capsule(cr, tx, ty0, TRACK_W, ty1 - ty0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.18 * alpha);
            let _ = cr.fill();
            let fill = (ty1 - ty0) * value;
            if fill > 0.5 {
                capsule(cr, tx, ty1 - fill.max(TRACK_W), TRACK_W, fill.max(TRACK_W));
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.95 * alpha);
                let _ = cr.fill();
            }
            // the percentage, its digits rolling as in Control Center
            let text = format!("{}%", (value * 100.0).round());
            crate::motion::text(cr, renderer, "side-level", &text, 0.0, y1 - 29.0, w, 0.5, Font::Small, (1.0, 1.0, 1.0), 0.62 * alpha);
        }
        cr.restore().unwrap();
        Some(Rect { x: 0.0, y: mid - HEIGHT / 2.0 - FILLET - SWELL - 4.0, w: WIDTH * 1.1 + SWELL + 4.0, h: HEIGHT + 2.0 * (FILLET + SWELL) + 8.0 })
    }
}

fn capsule(cr: &Context, x: f64, y: f64, w: f64, h: f64) {
    let r = w / 2.0;
    cr.new_path();
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 2.0 * std::f64::consts::PI);
    cr.arc(x + r, y + h - r, r, 0.0, std::f64::consts::PI);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(hud: &SideHud, now: Time) -> (cairo::ImageSurface, Option<Rect>) {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 200, 600).unwrap();
        let covered = {
            let cr = Context::new(&surface).unwrap();
            hud.draw(&cr, &Renderer::new(1.0), 1000.0, now)
        };
        (surface, covered)
    }

    fn alpha(surface: &mut cairo::ImageSurface, x: usize, y: usize) -> u8 {
        let stride = surface.stride() as usize;
        surface.data().unwrap()[y * stride + x * 4 + 3]
    }

    #[test]
    fn it_slides_out_of_the_left_edge_in_the_upper_half_and_away() {
        let mut hud = SideHud::default();
        assert!(render(&hud, 0.0).1.is_none(), "nothing until a level changes");
        hud.show(SliderKind::Volume, 0.5, false, 10.0);
        let (mut s, covered) = render(&hud, 10.6);
        let covered = covered.unwrap();
        assert!(covered.x == 0.0 && covered.x + covered.w < 70.0, "small, at the left edge");
        // a black drop on the edge a quarter of the way down (1000 tall),
        // the slider inside it filled below its middle and empty above
        assert_eq!(alpha(&mut s, 2, 250), 255);
        assert_eq!(alpha(&mut s, 2, 250 - 110 - 20), 0, "above it, the screen");
        let cx = (WIDTH / 2.0) as usize;
        let lit = |s: &mut cairo::ImageSurface, y: usize| { let stride = s.stride() as usize; s.data().unwrap()[y * stride + cx * 4 + 1] };
        let (ty0, ty1) = (250.0 - 110.0 + TRACK_TOP, 250.0 + 110.0 - TRACK_BOTTOM);
        let mid = ((ty0 + ty1) / 2.0) as usize;
        assert!(lit(&mut s, mid + 20) > 200, "filled below the middle");
        assert!(lit(&mut s, mid - 20) < 80, "empty above it");
        // a key held: each change extends the stay
        hud.show(SliderKind::Volume, 0.6, false, 11.0);
        assert!(!hud.advance(11.0 + HOLD - 0.1));
        assert!(hud.advance(11.0 + HOLD + 0.001));
        assert_eq!(hud.deadline(), None);
        // gone off the edge
        assert!(!hud.is_animating(15.0));
        assert!(render(&hud, 15.0).1.is_none());
    }

    #[test]
    fn brightness_after_volume_starts_from_its_own_level() {
        let mut hud = SideHud::default();
        hud.show(SliderKind::Volume, 0.9, false, 10.0);
        hud.show(SliderKind::Brightness, 0.2, false, 10.3);
        assert!((hud.value.value(10.3) - 0.2).abs() < 1e-9);
    }
}
