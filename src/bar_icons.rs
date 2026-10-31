//! The bar's status symbols drawn as vectors (no font glyphs): the speaker
//! whose waves light one by one with the level and are struck through when
//! muted, the keyboard, and iOS's battery — a rounded body filled to the
//! charge with the percentage cut out of it, its cap, and a bolt beside it
//! while it charges. Each one's motion lives in `Glyphs`, on springs.

use cairo::Context;

use crate::anim::{Spring, Time};
use crate::island::{Font, Renderer};

pub type Rgb = (f64, f64, f64);

/// How wide each symbol is (logical px), at rest.
pub const SPEAKER_W: f64 = 19.5;
pub const KEYBOARD_W: f64 = 20.0;
pub const ELLIPSIS_W: f64 = 14.0;
/// The battery: its body, the cap after a gap, and the bolt's room.
const BODY_W: f64 = 25.5;
const BODY_H: f64 = 12.5;
const BODY_R: f64 = 4.0;
const CAP_GAP: f64 = 1.0;
const CAP_W: f64 = 1.6;
const CAP_H: f64 = 4.4;
const BOLT_W: f64 = 9.0;
pub const BATTERY_W: f64 = BODY_W + CAP_GAP + CAP_W;

/// iOS's battery colours: low (≤ 20 %) red, charging green.
pub const BATTERY_LOW: Rgb = (1.0, 0x3B as f64 / 255.0, 0x30 as f64 / 255.0);
pub const BATTERY_CHARGING: Rgb = (0x34 as f64 / 255.0, 0xC7 as f64 / 255.0, 0x59 as f64 / 255.0);
/// What iOS leaves of a wave that is not lit.
const WAVE_DIM: f64 = 0.28;

/// The symbols' motion.
pub struct Glyphs {
    /// each wave of the speaker, lit 0..1
    waves: [Spring; 3],
    /// the mute stroke drawn across, 0..1
    slash: Spring,
    /// the battery's charge as filled, 0..1
    level: Spring,
    /// the bolt beside it, 0..1
    charge: Spring,
    /// the fill's colour: 0 white, 1 its tint
    tint: Spring,
    known: bool,
}

impl Default for Glyphs {
    fn default() -> Self {
        Glyphs {
            waves: [0; 3].map(|_| Spring::new(0.0, 0.28, 0.55)),
            slash: Spring::new(0.0, 0.3, 0.8),
            level: Spring::new(0.0, 0.6, 1.0),
            charge: Spring::new(0.0, 0.34, 0.6),
            tint: Spring::new(0.0, 0.4, 1.0),
            known: false,
        }
    }
}

impl Glyphs {
    /// Follow the volume (0..1, muted).
    pub fn volume(&mut self, value: f64, muted: bool, now: Time) {
        let lit = |i: usize| if muted { 0.0 } else if value > [0.0, 0.33, 0.66][i] { 1.0 } else { 0.0 };
        for (i, w) in self.waves.iter_mut().enumerate() {
            w.set(lit(i), now);
        }
        self.slash.set(if muted { 1.0 } else { 0.0 }, now);
    }

    /// Follow the battery.
    pub fn battery(&mut self, percent: u8, charging: bool, now: Time) {
        let level = percent.min(100) as f64 / 100.0;
        if !self.known {
            // its first reading is where it is, not a fill from empty
            self.level = Spring::new(level, 0.6, 1.0);
            self.known = true;
        }
        self.level.set(level, now);
        self.charge.set(if charging { 1.0 } else { 0.0 }, now);
        self.tint.set(if charging || percent <= 20 { 1.0 } else { 0.0 }, now);
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.waves.iter().any(|w| w.is_moving(now))
            || [&self.slash, &self.level, &self.charge, &self.tint].iter().any(|s| s.is_moving(now))
    }

    /// The battery's width (the bolt makes room for itself; the cell's
    /// width spring carries the change).
    pub fn battery_target_w(&self) -> f64 {
        BATTERY_W + BOLT_W * self.charge.target()
    }

    /// The speaker from `x`, centred on `cy` (SF Symbols' speaker.wave.3.fill:
    /// one smooth body and cone, three even arcs).
    pub fn draw_speaker(&self, cr: &Context, x: f64, cy: f64, colour: Rgb, alpha: f64, now: Time) {
        let (r, g, b) = colour;
        cr.save().unwrap();
        rounded_polygon(cr, &[
            (x, cy - 3.0, 1.2),
            (x + 3.6, cy - 3.0, 0.9),
            (x + 8.0, cy - 7.0, 1.6),
            (x + 8.0, cy + 7.0, 1.6),
            (x + 3.6, cy + 3.0, 0.9),
            (x, cy + 3.0, 1.2),
        ]);
        cr.set_source_rgba(r, g, b, alpha);
        let _ = cr.fill();
        // the waves, lit by the level (a lit one swells out a little)
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_width(1.75);
        for (i, w) in self.waves.iter().enumerate() {
            let k = w.value(now).clamp(-0.2, 1.3);
            let lit = WAVE_DIM + (1.0 - WAVE_DIM) * k.clamp(0.0, 1.0);
            let fade = 1.0 - self.slash.value(now).clamp(0.0, 1.0) * (1.0 - WAVE_DIM);
            let radius = [4.7, 7.9, 11.1][i] * (0.94 + 0.06 * k);
            let sweep = [0.74, 0.8, 0.84][i];
            cr.new_path();
            cr.arc(x + 7.0, cy, radius, -sweep, sweep);
            cr.set_source_rgba(r, g, b, alpha * (lit * fade).min(1.0));
            let _ = cr.stroke();
        }
        // muted: a stroke drawn across it, cut out of what it crosses
        let s = self.slash.value(now).clamp(0.0, 1.05);
        if s > 0.01 {
            let (x0, y0, x1, y1) = (x + 1.5, cy - 8.0, x + 17.0, cy + 8.0);
            let (ex, ey) = (x0 + (x1 - x0) * s, y0 + (y1 - y0) * s);
            cr.new_path();
            cr.move_to(x0, y0);
            cr.line_to(ex, ey);
            cr.set_line_width(4.2);
            cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
            let _ = cr.stroke_preserve();
            cr.set_line_width(1.6);
            cr.set_source_rgba(r, g, b, alpha);
            let _ = cr.stroke();
        }
        cr.restore().unwrap();
    }

    /// The battery from `x`, centred on `cy`: the charge `percent` written
    /// in it (cut out of the fill, as iOS does).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_battery(&self, cr: &Context, renderer: &Renderer, x: f64, cy: f64, percent: u8, alpha: f64, now: Time) {
        let level = self.level.value(now).clamp(0.0, 1.0);
        let tint = self.tint.value(now).clamp(0.0, 1.0);
        let charge = self.charge.value(now).clamp(0.0, 1.2);
        let colour = mix((1.0, 1.0, 1.0), if self.charge.target() > 0.5 { BATTERY_CHARGING } else { BATTERY_LOW }, tint);
        let y = renderer.snap(cy - BODY_H / 2.0);
        let x = renderer.snap(x);
        cr.save().unwrap();
        // the body: grey, filled to the charge
        rounded(cr, x, y, BODY_W, BODY_H, BODY_R);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.36 * alpha);
        let _ = cr.fill_preserve();
        cr.save().unwrap();
        cr.clip();
        cr.rectangle(x, y, BODY_W * level, BODY_H);
        cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
        let _ = cr.fill();
        cr.restore().unwrap();
        // the cap
        cr.new_path();
        let (cx0, cy0) = (x + BODY_W + CAP_GAP, cy - CAP_H / 2.0);
        cr.move_to(cx0, cy0);
        cr.arc(cx0 + CAP_W - 1.4, cy0 + 1.4, 1.4, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(cx0 + CAP_W - 1.4, cy0 + CAP_H - 1.4, 1.4, 0.0, std::f64::consts::FRAC_PI_2);
        cr.line_to(cx0, cy0 + CAP_H);
        cr.close_path();
        cr.set_source_rgba(1.0, 1.0, 1.0, (0.36 + 0.64 * (level >= 0.995) as u8 as f64) * alpha);
        let _ = cr.fill();
        // the percentage, cut out of it (the drop behind is black)
        let text = percent.to_string();
        let layout = renderer.layout(cr, &text, Font::BatteryDigits);
        let (_, ext) = layout.pixel_extents();
        let tx = x + (BODY_W - ext.width() as f64) / 2.0 - ext.x() as f64;
        let ty = y + (BODY_H - ext.height() as f64) / 2.0 - ext.y() as f64;
        cr.move_to(renderer.snap(tx), renderer.snap(ty));
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
        pangocairo::functions::show_layout(cr, &layout);
        // charging: a bolt beside it, springing in
        if charge > 0.01 {
            let bx = x + BATTERY_W + 2.6;
            cr.translate(bx + 3.0, cy);
            cr.scale(charge, charge);
            bolt(cr);
            cr.set_source_rgba(1.0, 1.0, 1.0, alpha * charge.min(1.0));
            let _ = cr.fill();
        }
        cr.restore().unwrap();
    }
}

/// The keyboard from `x`, centred on `cy` (SF Symbols' keyboard.fill): a
/// filled body with three rows cut out of it — seven keys, six between
/// them, and the space bar between two.
pub fn draw_keyboard(cr: &Context, renderer: &Renderer, x: f64, cy: f64, colour: Rgb, alpha: f64) {
    let (w, h) = (KEYBOARD_W, 13.5);
    let (x, y) = (renderer.snap(x), renderer.snap(cy - h / 2.0));
    cr.save().unwrap();
    rounded(cr, x, y, w, h, 3.0);
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
    let _ = cr.fill();
    cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
    let (kw, kh, pitch, margin) = (1.8, 1.8, 2.5, 1.6);
    let key = |kx: f64, ky: f64, width: f64| {
        rounded(cr, kx, ky, width, kh, 0.5);
        let _ = cr.fill();
    };
    for i in 0..7 {
        key(x + margin + i as f64 * pitch, y + 2.4, kw);
    }
    for i in 0..6 {
        key(x + margin + pitch / 2.0 + i as f64 * pitch, y + 5.8, kw);
    }
    key(x + margin, y + 9.2, kw);
    key(x + margin + pitch, y + 9.2, w - 2.0 * (margin + pitch));
    key(x + w - margin - kw, y + 9.2, kw);
    cr.restore().unwrap();
}

/// The drawer's handle from `x`, centred on `cy`: three dots.
pub fn draw_ellipsis(cr: &Context, x: f64, cy: f64, colour: Rgb, alpha: f64) {
    cr.save().unwrap();
    cr.new_path();
    for i in 0..3 {
        let cx = x + 1.7 + i as f64 * 5.3;
        cr.new_sub_path();
        cr.arc(cx, cy, 1.7, 0.0, std::f64::consts::TAU);
    }
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
    let _ = cr.fill();
    cr.restore().unwrap();
}

/// A closed polygon through `(x, y, radius)` points, each corner rounded
/// smoothly (a curve tangent to both of its edges).
fn rounded_polygon(cr: &Context, points: &[(f64, f64, f64)]) {
    let n = points.len();
    cr.new_path();
    for i in 0..n {
        let (px, py, _) = points[(i + n - 1) % n];
        let (vx, vy, r) = points[i];
        let (nx, ny, _) = points[(i + 1) % n];
        let (ax, ay) = (px - vx, py - vy);
        let (bx, by) = (nx - vx, ny - vy);
        let (la, lb) = (ax.hypot(ay), bx.hypot(by));
        let (ax, ay, bx, by) = (ax / la, ay / la, bx / lb, by / lb);
        // where the curve leaves each edge: r / tan(half the corner's angle)
        let angle = (ax * bx + ay * by).clamp(-1.0, 1.0).acos();
        let t = (r / (angle / 2.0).tan()).min(la / 2.0).min(lb / 2.0);
        let (p1x, p1y) = (vx + ax * t, vy + ay * t);
        let (p2x, p2y) = (vx + bx * t, vy + by * t);
        if i == 0 { cr.move_to(p1x, p1y) } else { cr.line_to(p1x, p1y) }
        let k = 0.55;
        cr.curve_to(p1x + (vx - p1x) * k, p1y + (vy - p1y) * k, p2x + (vx - p2x) * k, p2y + (vy - p2y) * k, p2x, p2y);
    }
    cr.close_path();
}

/// A bolt round the origin, about 7 × 11.
fn bolt(cr: &Context) {
    cr.new_path();
    cr.move_to(0.9, -5.6);
    cr.line_to(-3.4, 0.9);
    cr.line_to(-0.2, 0.9);
    cr.line_to(-0.9, 5.6);
    cr.line_to(3.4, -0.9);
    cr.line_to(0.2, -0.9);
    cr.close_path();
}

fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

fn mix(a: Rgb, b: Rgb, k: f64) -> Rgb {
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k)
}
