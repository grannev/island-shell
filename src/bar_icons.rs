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
pub const SPEAKER_W: f64 = 19.0;
pub const KEYBOARD_W: f64 = 19.0;
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

    /// The speaker from `x`, centred on `cy`.
    pub fn draw_speaker(&self, cr: &Context, x: f64, cy: f64, colour: Rgb, alpha: f64, now: Time) {
        let (r, g, b) = colour;
        cr.save().unwrap();
        cr.new_path();
        // the body and its cone, one rounded shape
        cr.set_line_join(cairo::LineJoin::Round);
        cr.set_line_width(1.3);
        cr.move_to(x + 1.0, cy - 2.9);
        cr.line_to(x + 4.0, cy - 2.9);
        cr.line_to(x + 8.0, cy - 6.6);
        cr.line_to(x + 8.0, cy + 6.6);
        cr.line_to(x + 4.0, cy + 2.9);
        cr.line_to(x + 1.0, cy + 2.9);
        cr.close_path();
        cr.set_source_rgba(r, g, b, alpha);
        let _ = cr.fill_preserve();
        let _ = cr.stroke();
        // the waves, lit by the level (a lit one swells out a little)
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_width(1.6);
        for (i, w) in self.waves.iter().enumerate() {
            let k = w.value(now).clamp(-0.2, 1.3);
            let lit = WAVE_DIM + (1.0 - WAVE_DIM) * k.clamp(0.0, 1.0);
            let fade = 1.0 - self.slash.value(now).clamp(0.0, 1.0) * (1.0 - WAVE_DIM);
            let radius = [3.9, 7.0, 10.1][i] * (0.92 + 0.08 * k);
            let sweep = [0.95, 0.88, 0.82][i];
            cr.new_path();
            cr.arc(x + 7.4, cy, radius, -sweep, sweep);
            cr.set_source_rgba(r, g, b, alpha * (lit * fade).min(1.0));
            let _ = cr.stroke();
        }
        // muted: a stroke drawn across it, cut out of what it crosses
        let s = self.slash.value(now).clamp(0.0, 1.05);
        if s > 0.01 {
            let (x0, y0, x1, y1) = (x + 1.5, cy - 7.5, x + 16.0, cy + 7.5);
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

/// The keyboard from `x`, centred on `cy`: a filled body, its keys cut out.
pub fn draw_keyboard(cr: &Context, renderer: &Renderer, x: f64, cy: f64, colour: Rgb, alpha: f64) {
    let (w, h) = (KEYBOARD_W, 12.6);
    let (x, y) = (renderer.snap(x), renderer.snap(cy - h / 2.0));
    cr.save().unwrap();
    rounded(cr, x, y, w, h, 2.8);
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
    let _ = cr.fill();
    cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
    let key = |kx: f64, ky: f64, kw: f64| {
        rounded(cr, renderer.snap(kx), renderer.snap(ky), kw, 1.9, 0.5);
        let _ = cr.fill();
    };
    // two rows of keys, the second shifted by half a key, and the space bar
    for i in 0..6 {
        key(x + 2.2 + i as f64 * 2.6, y + 2.2, 1.9);
    }
    for i in 0..5 {
        key(x + 3.5 + i as f64 * 2.6, y + 5.0, 1.9);
    }
    key(x + 5.0, y + 8.3, w - 10.0);
    cr.restore().unwrap();
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
