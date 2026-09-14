//! The island itself: its state, the animated quantities derived from it and
//! the drawing. Everything here is in logical pixels; `Renderer` maps them to
//! the output's pixels through the cairo scale.

use std::f64::consts::FRAC_PI_2;

use cairo::Context;

use crate::anim::{Quintic, Spring, Time, inv_min_jerk};

/// Height of the black box, logical px.
pub const BOX_H: f64 = 32.0;
/// Bottom corner radius at rest / when fully retracted into the edge.
const RADIUS: f64 = 14.0;
const RADIUS_HIDDEN: f64 = 22.0;
/// The concave fillets where the box meets the top edge of the screen.
pub const FILLET: f64 = 14.0;
const PAD: f64 = 16.0;
const PAD_HOVER: f64 = 34.0;
/// Extra horizontal padding when the box has fully sunk into the edge ("drop
/// meeting water" spread).
const SPREAD: f64 = 70.0;
/// How far the island moves left to make room for the ⋯ drawer.
pub const DODGE: f64 = 240.0;

pub const FONT: &str = "SF Pro Text Semi-Bold";
pub const FONT_PX: f64 = 16.0;

const HIDE_DUR: f64 = 0.25;
const DODGE_DUR: f64 = 0.25;
/// Hover: a spring with a little overshoot, like the CSS
/// cubic-bezier(0.2, 0.9, 0.3, 1.25) it replaces (peak ~250 ms in).
const HOVER_RESPONSE: f64 = 0.4;
const HOVER_DAMPING: f64 = 0.6;
const HOVER_BG_DUR: f64 = 0.25;

/// Tunables (config file / `island-shell set`).
#[derive(Clone, Debug)]
pub struct Config {
    /// Corner smoothing 0..1 (0 = circular arcs, 0.6 ≈ Apple's continuous
    /// curvature). Limited by the room on the shorter edge of each corner.
    pub smoothing: f64,
    /// Velocity-dependent stretch of the box, 0 = rigid, 1 = the default
    /// "liquid" amount.
    pub liquid: f64,
    /// React to the pointer over the box (padding 16 → 34 on a spring, a
    /// touch of grey).
    pub hover: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { smoothing: 0.6, liquid: 1.0, hover: true }
    }
}

impl Config {
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let v: f64 = match value {
            "on" | "true" => 1.0,
            "off" | "false" => 0.0,
            v => v.parse().map_err(|_| format!("bad value {value:?}"))?,
        };
        match key {
            "smoothing" => self.smoothing = v.clamp(0.0, 1.0),
            "liquid" => self.liquid = v.clamp(0.0, 4.0),
            "hover" => self.hover = v != 0.0,
            _ => return Err(format!("unknown setting {key:?}")),
        }
        Ok(())
    }

    pub fn describe(&self) -> String {
        format!("smoothing={} liquid={} hover={}", self.smoothing, self.liquid, self.hover as u8)
    }
}

/// Why the island is out of sight; several may hold at once and it comes back
/// only when all are cleared.
#[derive(Clone, Copy, Debug, Default)]
pub struct HideReasons {
    /// mod+i
    pub user: bool,
    /// a fullscreen window has the focus (sway IPC)
    pub fullscreen: bool,
}

impl HideReasons {
    fn any(self) -> bool {
        self.user || self.fullscreen
    }
}

pub struct Island {
    /// 0 = fully shown, 1 = parked above the edge.
    hide: Quintic,
    /// 0 = centred, 1 = dodged 240 px left.
    dodge: Quintic,
    /// Pointer over the box: the padding springs, the background tint eases.
    hover_pad: Spring,
    hover_bg: Quintic,
    pub hidden: HideReasons,
    pub dodged: bool,
    pub hovered: bool,
    pub clock: String,
    pub config: Config,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn union(self, o: Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect { x, y, w: (self.x + self.w).max(o.x + o.w) - x, h: (self.y + self.h).max(o.y + o.h) - y }
    }
}

/// What one frame drew, for the input / opaque regions and damage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// The whole painted area (box + fillets), `None` when above the edge.
    pub bounds: Option<Rect>,
    /// Fully opaque parts of the box.
    pub opaque: [Option<Rect>; 2],
}

impl Island {
    pub fn new(config: Config) -> Self {
        Island {
            hide: Quintic::at_rest(0.0, HIDE_DUR),
            dodge: Quintic::at_rest(0.0, DODGE_DUR),
            hover_pad: Spring::new(0.0, HOVER_RESPONSE, HOVER_DAMPING),
            hover_bg: Quintic::at_rest(0.0, HOVER_BG_DUR),
            hidden: HideReasons::default(),
            dodged: false,
            hovered: false,
            clock: String::new(),
            config,
        }
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.hide.is_moving(now)
            || self.dodge.is_moving(now)
            || self.hover_pad.is_moving(now)
            || self.hover_bg.is_moving(now)
    }

    /// Re-aim the movers after a state change.
    pub fn sync(&mut self, now: Time) {
        self.hide.set(if self.hidden.any() { 1.0 } else { 0.0 }, now);
        self.dodge.set(if self.dodged { 1.0 } else { 0.0 }, now);
        // no hover feedback while hidden: the box is not there to point at
        let hover = self.config.hover && self.hovered && !self.hidden.any();
        let h = if hover { 1.0 } else { 0.0 };
        self.hover_pad.set(h, now);
        self.hover_bg.set(h, now);
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden.any()
    }

    pub fn update_clock(&mut self) -> bool {
        let text = clock_text();
        if text != self.clock {
            self.clock = text;
            true
        } else {
            false
        }
    }

    /// Draw one frame on `cr` (already scaled to logical px), the island
    /// centred on `cx`, for the instant `now` (when the frame will be shown).
    pub fn draw(&self, cr: &Context, renderer: &Renderer, cx: f64, now: Time) -> Frame {
        let moving = self.is_animating(now);
        let hide = self.hide.state(now);
        let dodge = self.dodge.state(now);
        let hover_pad = self.hover_pad.value(now);
        let hover_bg = self.hover_bg.value(now);
        // the spread lags behind the slide — most of it happens while the box
        // is already mostly gone, like a drop flattening on water; it is a
        // function of the position so that show and hide trace the same path
        let spread_k = inv_min_jerk(hide.x).powf(2.2);

        let cx = cx - DODGE * dodge.x;
        let dy = -BOX_H * hide.x;
        if dy <= -(BOX_H + FILLET) {
            return Frame::default();
        }
        let pad = PAD + (PAD_HOVER - PAD) * hover_pad + SPREAD * spread_k;
        let radius = RADIUS + (RADIUS_HIDDEN - RADIUS) * hide.x;

        // "liquid": the box stretches along its direction of travel in
        // proportion to its speed (px/s) and relaxes as it stops
        let liquid = self.config.liquid;
        let stretch_y = liquid * 0.02 * (hide.v * BOX_H).abs();
        let stretch_x = liquid * 0.01 * (dodge.v * DODGE).abs();

        let layout = renderer.layout(cr, &self.clock);
        let (_, logical) = layout.pixel_extents();
        let tw = logical.width() as f64;
        let th = logical.height() as f64;

        let bw = tw + 2.0 * pad + stretch_x;
        let bh = BOX_H + stretch_y - stretch_x * 0.15;
        let (mut x0, mut x1, mut y1) = (cx - bw / 2.0, cx + bw / 2.0, dy + bh);
        if !moving {
            // crisp edges at rest; while moving they glide sub-pixel
            x0 = renderer.snap(x0);
            x1 = renderer.snap(x1);
            y1 = renderer.snap(y1);
        }
        let r = radius.min((x1 - x0) / 2.0).min((y1 - dy) / 2.0);

        self.trace_notch(cr, x0, x1, dy, y1, r);
        let tint = 10.0 / 255.0 * hover_bg; // #000 -> #0A0A0A
        cr.set_source_rgb(tint, tint, tint);
        let _ = cr.fill();

        // the clock, centred, its origin on whole output pixels: the glyphs
        // are always rendered the same way (hinted metrics), so the text
        // never changes shape between motion and rest
        let tx = renderer.snap(cx - tw / 2.0 - logical.x() as f64);
        let ty = renderer.snap(dy + (y1 - dy - th) / 2.0 - logical.y() as f64);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(tx, ty);
        pangocairo::functions::show_layout(cr, &layout);

        let top = dy.max(0.0);
        let bounds = Rect { x: x0 - FILLET, y: top, w: x1 - x0 + 2.0 * FILLET, h: y1 - top };
        // opaque: the box minus its curved corners (a T shape)
        let opaque = [
            (y1 - r > top).then(|| Rect { x: x0, y: top, w: x1 - x0, h: y1 - r - top }),
            (x1 - x0 > 2.0 * r).then(|| Rect { x: x0 + r, y: (y1 - r).max(top), w: x1 - x0 - 2.0 * r, h: r.min(y1 - top) }),
        ];
        Frame { bounds: Some(bounds), opaque }
    }

    /// The notch outline, traced counter-clockwise: the concave fillets at
    /// the top are right turns, the convex corners at the bottom left turns
    /// (`Corner::trace` mirrored).
    fn trace_notch(&self, cr: &Context, x0: f64, x1: f64, y0: f64, y1: f64, r: f64) {
        let s = self.config.smoothing;
        let h = y1 - y0;
        // the vertical edge is shared by a fillet and a corner: each may
        // smooth over at most its half
        let fillet = Corner::new(FILLET, s, h / 2.0);
        let corner = Corner::new(r, s, (h / 2.0).min((x1 - x0) / 2.0));
        cr.new_path();
        cr.move_to(x0 - fillet.p, y0);
        fillet.trace(cr, x0, y0, 0.0, false);
        cr.line_to(x0, y1 - corner.p);
        corner.trace(cr, x0, y1, FRAC_PI_2, true);
        cr.line_to(x1 - corner.p, y1);
        corner.trace(cr, x1, y1, 0.0, true);
        cr.line_to(x1, y0 + fillet.p);
        fillet.trace(cr, x1, y0, -FRAC_PI_2, false);
        cr.close_path();
    }
}

/// A right-turn corner with continuous curvature: a circular arc of
/// reduced sweep between two cubic Béziers that take the curvature from
/// zero on the straight edge up to 1/r and back (Figma's corner smoothing,
/// after "Desperately seeking squircles"). `smoothing` 0 = plain arc.
struct Corner {
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
    fn new(r: f64, smoothing: f64, budget: f64) -> Corner {
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
    fn trace(&self, cr: &Context, x: f64, y: f64, rot: f64, mirror: bool) {
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

/// `%a %b %-d  %-I:%M %p` in the local time zone, e.g. "Mon Sep 15  1:05 PM".
pub fn clock_text() -> String {
    let now = libc_localtime();
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let hour12 = match now.hour % 12 {
        0 => 12,
        h => h,
    };
    let ampm = if now.hour < 12 { "AM" } else { "PM" };
    format!(
        "{} {} {}  {}:{:02} {}",
        DAYS[now.wday as usize], MONTHS[now.mon as usize], now.mday, hour12, now.min, ampm
    )
}

pub struct LocalTime {
    pub sec: u32,
    pub min: u32,
    pub hour: u32,
    pub mday: u32,
    pub mon: u32,
    pub wday: u32,
}

/// Broken-down local time through libc (no chrono needed for one clock).
pub fn libc_localtime() -> LocalTime {
    unsafe extern "C" {
        fn time(t: *mut i64) -> i64;
        fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
    }
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const u8,
    }
    let mut t = 0i64;
    let mut tm = Tm {
        tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 1, tm_mon: 0, tm_year: 0,
        tm_wday: 0, tm_yday: 0, tm_isdst: 0, tm_gmtoff: 0, tm_zone: std::ptr::null(),
    };
    unsafe {
        time(&mut t);
        localtime_r(&t, &mut tm);
    }
    LocalTime {
        sec: tm.tm_sec as u32,
        min: tm.tm_min as u32,
        hour: tm.tm_hour as u32,
        mday: tm.tm_mday as u32,
        mon: tm.tm_mon as u32,
        wday: tm.tm_wday as u32,
    }
}

/// Pango setup shared by all drawing: font, hinting for the fractional scale.
pub struct Renderer {
    scale: f64,
    font: pango::FontDescription,
}

impl Renderer {
    pub fn new(scale: f64) -> Self {
        let mut font = pango::FontDescription::from_string(FONT);
        font.set_absolute_size(FONT_PX * pango::SCALE as f64);
        Renderer { scale, font }
    }

    /// Round a logical coordinate to a whole output pixel.
    pub fn snap(&self, v: f64) -> f64 {
        (v * self.scale).round() / self.scale
    }

    /// A layout for `text`, glyph positions rounded to output pixels. (Not
    /// switched to fractional positions while moving: unhinted metrics give
    /// a text about 1 px narrower and shorter, and the swap at the end of a
    /// move read as the text changing size.)
    pub fn layout(&self, cr: &Context, text: &str) -> pango::Layout {
        let layout = pangocairo::functions::create_layout(cr);
        let ctx = layout.context();
        let mut opts = cairo::FontOptions::new().expect("font options");
        opts.set_hint_metrics(cairo::HintMetrics::On);
        opts.set_hint_style(cairo::HintStyle::Slight);
        opts.set_antialias(cairo::Antialias::Gray);
        pangocairo::functions::context_set_font_options(&ctx, Some(&opts));
        ctx.set_round_glyph_positions(true);
        layout.set_font_description(Some(&self.font));
        layout.set_text(text);
        layout
    }
}
