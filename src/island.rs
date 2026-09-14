//! The island itself: its state, the animated quantities derived from it and
//! the drawing. Everything here is in logical pixels; `Renderer` maps them to
//! the output's pixels through the cairo scale.

use std::f64::consts::{FRAC_PI_2, PI};
use std::time::{Duration, Instant};

use cairo::Context;

use crate::anim::{Ease, Tween, inv_min_jerk};

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

const HIDE_DUR: Duration = Duration::from_millis(250);
const DODGE_DUR: Duration = Duration::from_millis(250);
const HOVER_DUR: Duration = Duration::from_millis(420);
const HOVER_BG_DUR: Duration = Duration::from_millis(250);
const HOVER_EASE: Ease = Ease::Bezier(0.2, 0.9, 0.3, 1.25);

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
    hide: Tween,
    /// 0 = centred, 1 = dodged 240 px left.
    dodge: Tween,
    /// Pointer over the box: padding (with the springy overshoot) and the
    /// background tint move on their own clocks like the CSS transitions did.
    hover_pad: Tween,
    hover_bg: Tween,
    pub hidden: HideReasons,
    pub dodged: bool,
    pub hovered: bool,
    pub clock: String,
}

/// Geometry of one frame, for the input region and hit testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Island {
    pub fn new() -> Self {
        Island {
            hide: Tween::fixed(0.0, HIDE_DUR, Ease::MinJerk),
            dodge: Tween::fixed(0.0, DODGE_DUR, Ease::MinJerk),
            hover_pad: Tween::fixed(0.0, HOVER_DUR, HOVER_EASE),
            hover_bg: Tween::fixed(0.0, HOVER_BG_DUR, Ease::EASE),
            hidden: HideReasons::default(),
            dodged: false,
            hovered: false,
            clock: String::new(),
        }
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.hide.is_moving(now)
            || self.dodge.is_moving(now)
            || self.hover_pad.is_moving(now)
            || self.hover_bg.is_moving(now)
    }

    /// Re-aim the tweens after a state change.
    pub fn sync(&mut self, now: Instant) {
        self.hide.set(if self.hidden.any() { 1.0 } else { 0.0 }, now);
        self.dodge.set(if self.dodged { 1.0 } else { 0.0 }, now);
        // no hover feedback while hidden: the box is not there to point at
        let hover = self.hovered && !self.hidden.any();
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
    /// centred on `cx`. Returns the box rectangle (logical px) or `None` when
    /// the box is entirely above the edge.
    pub fn draw(&self, cr: &Context, renderer: &Renderer, cx: f64, now: Instant) -> Option<BoxRect> {
        let hide = self.hide.value(now); // 0..1, already eased
        // the spread lags behind the slide — most of it happens while the box
        // is already mostly gone, like a drop flattening on water; it is a
        // function of the position so that show and hide trace the same path
        let spread_k = inv_min_jerk(hide).powf(2.2);
        let hover_pad = self.hover_pad.value(now);
        let hover_bg = self.hover_bg.value(now);
        let dodge = self.dodge.value(now);

        let cx = cx - DODGE * dodge;
        let dy = -BOX_H * hide;
        if dy <= -(BOX_H + FILLET) {
            return None;
        }
        let pad = PAD + (PAD_HOVER - PAD) * hover_pad + SPREAD * spread_k;
        let radius = RADIUS + (RADIUS_HIDDEN - RADIUS) * hide;

        let layout = renderer.layout(cr, &self.clock);
        let (_, logical) = layout.pixel_extents();
        let tw = logical.width() as f64;
        let th = logical.height() as f64;

        let bw = tw + 2.0 * pad;
        let x0 = cx - bw / 2.0;
        let x1 = cx + bw / 2.0;
        let y1 = dy + BOX_H;
        let r = radius.min(bw / 2.0).min(BOX_H / 2.0);

        // the notch: concave fillets at the top, convex corners at the bottom
        cr.new_path();
        cr.move_to(x0 - FILLET, dy);
        cr.arc(x0 - FILLET, dy + FILLET, FILLET, -FRAC_PI_2, 0.0);
        cr.line_to(x0, y1 - r);
        cr.arc_negative(x0 + r, y1 - r, r, PI, FRAC_PI_2);
        cr.line_to(x1 - r, y1);
        cr.arc_negative(x1 - r, y1 - r, r, FRAC_PI_2, 0.0);
        cr.line_to(x1, dy + FILLET);
        cr.arc(x1 + FILLET, dy + FILLET, FILLET, PI, 3.0 * FRAC_PI_2);
        cr.close_path();
        let tint = 10.0 / 255.0 * hover_bg; // #000 -> #0A0A0A
        cr.set_source_rgb(tint, tint, tint);
        let _ = cr.fill();

        // the clock, centred, its origin snapped to whole output pixels so the
        // glyphs never land between them while the box is moving
        let tx = renderer.snap(cx - tw / 2.0 - logical.x() as f64);
        let ty = renderer.snap(dy + (BOX_H - th) / 2.0 - logical.y() as f64);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(tx, ty);
        pangocairo::functions::show_layout(cr, &layout);

        Some(BoxRect { x: x0 - FILLET, y: dy.max(0.0), w: bw + 2.0 * FILLET, h: y1 - dy.max(0.0) })
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
