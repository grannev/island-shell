//! Light along the screen's edges. The island is one drop of the black
//! frame beyond the screen (`bezel`), and events flow out of it along the
//! frame: a notification's colour runs from the island along the top edge
//! towards the corners, a critical one goes round the whole screen, Touch ID
//! breathes blue around it while it waits and flashes green or shudders red
//! with the result.
//!
//! The light is drawn just inside the screen's edges, under the island; its
//! strength along an edge follows the distance from the island, so what the
//! eye reads as light travelling along the frame is a ring growing from the
//! island. The frame's black corners round it off.

use cairo::Context;

use crate::anim::Time;
use crate::island::Rect;

type Rgb = (f64, f64, f64);

/// How far the light reaches into the screen.
const DEPTH: f64 = 18.0;
/// The soft leading edge of a travelling front, and the trail it leaves.
const SOFT: f64 = 140.0;
const TRAIL: f64 = 320.0;
/// Touch ID's wave goes round in this long.
const BREATH_PERIOD: f64 = 2.2;
const BREATH_FADE: f64 = 0.35;

pub const TOUCH_ID_BLUE: Rgb = (0.40, 0.85, 1.0);
pub const SUCCESS_GREEN: Rgb = (0.20, 0.84, 0.35);
pub const FAILURE_RED: Rgb = (1.0, 0.23, 0.19);

/// A one-off wave of light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    /// from the island along the top edge to the corners (a notification)
    Top,
    /// from the island all the way round to the bottom (a critical one)
    Around,
    /// the whole outline lights up at once and fades (success)
    Flash,
    /// a short shake out of the island and back (a miss)
    Shudder,
    /// green rising from the bottom corners up the sides and draining
    /// away (the charger plugged in)
    Rise,
    /// a streak of light along the top edge, to the right (the next
    /// track) or the left (the previous one)
    StreakRight,
    StreakLeft,
}

impl Wave {
    fn duration(self) -> f64 {
        match self {
            Wave::Top => 1.15,
            Wave::Around => 1.7,
            Wave::Flash => 0.75,
            Wave::Shudder => 0.6,
            Wave::Rise => 1.8,
            Wave::StreakRight | Wave::StreakLeft => 0.7,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pulse {
    wave: Wave,
    colour: Rgb,
    start: Time,
}

/// A timer drawn as a line round the screen, clockwise from the island.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gauge {
    /// how far round, 0..1 (a countdown's time gone; a stopwatch's sweep
    /// through the minute)
    pub progress: f64,
    pub colour: Rgb,
    /// paused: the line dims
    pub running: bool,
    /// a stopwatch: a comet with a fading tail instead of a growing line
    pub comet: bool,
}

/// How far along the edges a low battery's red reaches from the corners.
const EMBER_REACH: f64 = 70.0;

/// How bright the gauge's line is, and its head.
const GAUGE_LINE: f64 = 0.5;
const GAUGE_HEAD: f64 = 0.5;
const GAUGE_HEAD_LEN: f64 = 48.0;
/// A comet's tail, as a share of the way round.
const COMET_TAIL: f64 = 0.22;

/// A volume or brightness level standing up one side of the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    /// the right edge (volume), else the left (brightness)
    pub right: bool,
    pub value: f64,
    /// how far it is shown, 0..1
    pub shown: f64,
    pub colour: Rgb,
}

#[derive(Default)]
pub struct Edge {
    pulses: Vec<Pulse>,
    gauge: Option<Gauge>,
    level: Option<Level>,
    /// the bottom corners smouldering red (low battery), how strongly
    ember: f64,
    /// the frame grown this much thicker (Focus / Do Not Disturb), and the
    /// screen's corner radius it rounds with
    border: (f64, f64),
    /// music's light along the bottom edge: the cover's colour, how bright
    ambient: Option<(Rgb, f64)>,
    /// a continuous wave (Touch ID waiting): its colour, since when, and
    /// when it began to fade out
    breath: Option<(Rgb, Time, Option<Time>)>,
}

impl Edge {
    /// Send a wave of light out of the island; a newer one of the same kind
    /// takes the place of the old.
    pub fn pulse(&mut self, wave: Wave, colour: Rgb, now: Time) {
        self.pulses.retain(|p| p.wave != wave);
        self.pulses.push(Pulse { wave, colour, start: now });
    }

    /// A wave that starts at `at` (later waves of the same kind may queue).
    pub fn schedule(&mut self, wave: Wave, colour: Rgb, at: Time) {
        self.pulses.push(Pulse { wave, colour, start: at });
    }

    /// Drop the waves that have not started yet (what they announced was
    /// called off).
    pub fn cancel_scheduled(&mut self, now: Time) {
        self.pulses.retain(|p| p.start <= now);
    }

    /// Keep a slow wave going round (Some), or let it fade (None).
    pub fn breathe(&mut self, colour: Option<Rgb>, now: Time) {
        match (colour, &mut self.breath) {
            (Some(c), None) => self.breath = Some((c, now, None)),
            (Some(c), Some(b)) => {
                b.0 = c;
                // back before it had faded: carry on
                b.2 = None;
            }
            (None, Some(b)) if b.2.is_none() => b.2 = Some(now),
            _ => {}
        }
    }

    /// Show (or take away) the timer's line; it is still between updates.
    pub fn set_gauge(&mut self, gauge: Option<Gauge>) {
        self.gauge = gauge;
    }

    /// A level up one side (still unless the island's springs move it).
    pub fn set_level(&mut self, level: Option<Level>) {
        self.level = level.filter(|l| l.shown > 0.005);
    }

    /// Light the bottom edge in the cover's colour (None: dark).
    pub fn set_ambient(&mut self, ambient: Option<(Rgb, f64)>) {
        self.ambient = ambient.filter(|a| a.1 > 0.005);
    }

    /// Thicken the frame by `width` (0 = as usual), concentric with the
    /// screen's corners of `radius`.
    pub fn set_border(&mut self, width: f64, radius: f64) {
        self.border = (width, radius);
    }

    /// The bottom corners' red glow (0 = none); it stands still.
    pub fn set_ember(&mut self, strength: f64) {
        self.ember = strength;
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.pulses.iter().any(|p| now - p.start < p.wave.duration()) || self.breath.is_some()
    }

    /// Paint the light for `now` on an output `w`×`h` whose corners have
    /// `radius`, with the island centred on `cx`. Returns the strips it
    /// painted (empty when dark).
    pub fn draw(&mut self, cr: &Context, w: f64, h: f64, cx: f64, now: Time) -> Vec<Rect> {
        self.pulses.retain(|p| now - p.start < p.wave.duration());
        if self.breath.is_some_and(|(_, _, end)| end.is_some_and(|e| now - e >= BREATH_FADE)) {
            self.breath = None;
        }
        if self.pulses.is_empty() && self.breath.is_none() && self.gauge.is_none() && self.level.is_none()
            && self.ember <= 0.0 && self.border.0 <= 0.05 && self.ambient.is_none() {
            return Vec::new();
        }
        let far = (w / 2.0 + cx.abs().min(w)).hypot(h) + SOFT;
        let mut lit = false;
        if self.ember > 0.0 {
            let ember = self.ember;
            lit |= paint_by(cr, w, h, FAILURE_RED, &|side, t| {
                // distance from the nearer bottom corner, along the edge
                let d = match side {
                    Side::Bottom => t.min(w - t),
                    Side::Left | Side::Right => h - t,
                    Side::Top => return 0.0,
                };
                if d > 4.0 * EMBER_REACH { 0.0 } else { ember * (-d / EMBER_REACH).exp() }
            });
        }
        if let Some((colour, strength)) = self.ambient {
            lit |= paint_by(cr, w, h, colour, &|side, t| match side {
                // brightest under the middle of the screen, as if lit from
                // behind it, and up the lower part of the sides
                Side::Bottom => strength * (0.55 + 0.45 * (std::f64::consts::PI * t / w).sin()),
                Side::Left | Side::Right => strength * 0.55 * ((t - 0.6 * h) / (0.4 * h)).clamp(0.0, 1.0),
                Side::Top => 0.0,
            });
        }
        if let Some(g) = self.gauge {
            lit |= paint_gauge(cr, w, h, cx, &g);
        }
        if let Some(l) = self.level {
            let top = h * (1.0 - l.value.clamp(0.0, 1.0));
            lit |= paint_by(cr, w, h, l.colour, &|side, t| {
                if side != if l.right { Side::Right } else { Side::Left } || t < top - STEP {
                    return 0.0;
                }
                // a line up from the bottom, brightest at its top
                let tip = (1.0 - (t - top).max(0.0) / GAUGE_HEAD_LEN).max(0.0);
                l.shown * (GAUGE_LINE + GAUGE_HEAD * tip)
            });
        }
        for p in &self.pulses {
            if p.wave == Wave::Rise {
                let k = ((now - p.start) / p.wave.duration()).clamp(0.0, 1.0);
                // up quickly, a moment full, then draining away
                let fill = if k < 0.35 { ease_out(k / 0.35) } else if k < 0.5 { 1.0 } else { 1.0 - smooth((k - 0.5) / 0.5) };
                let alpha = smooth(k / 0.06) * (1.0 - smooth((k - 0.75) / 0.25));
                let surface = h * 0.55 * fill;
                lit |= paint_by(cr, w, h, p.colour, &|side, t| match side {
                    Side::Bottom => 0.8 * alpha,
                    // the liquid's surface is soft, a little brighter
                    Side::Left | Side::Right => {
                        let depth = surface - (h - t);
                        if depth < -40.0 { 0.0 } else if depth < 0.0 { alpha * (1.0 + depth / 40.0) } else { alpha * 0.8 }
                    }
                    Side::Top => 0.0,
                });
                continue;
            }
            let k = ((now - p.start) / p.wave.duration()).clamp(0.0, 1.0);
            let rise = smooth(k / 0.08);
            let stops: Vec<(f64, f64)> = match p.wave {
                Wave::Top => front(ease_out(k) * 0.64 * w, (1.0 - k).powf(1.4) * rise, 0.18),
                Wave::Around => front(ease_out(k) * far, (1.0 - k).powf(1.2) * rise, 0.22),
                Wave::Flash => vec![(0.0, rise * (1.0 - smooth((k - 0.15) / 0.85))), (far, rise * (1.0 - smooth((k - 0.15) / 0.85)))],
                // drawn above, from the bottom corners
                Wave::Rise => continue,
                Wave::StreakRight | Wave::StreakLeft => {
                    let dir = if p.wave == Wave::StreakRight { 1.0 } else { -1.0 };
                    let at = cx + dir * (w / 2.0 + 120.0) * ease_out(k);
                    let alpha = (1.0 - k).powf(1.3) * rise;
                    lit |= paint_by(cr, w, h, p.colour, &|side, t| {
                        if side != Side::Top { return 0.0; }
                        // a head and a tail behind it, which never reaches
                        // back past the island
                        if (t - cx) * dir < 0.0 { return 0.0; }
                        let d = (t - at) * dir;
                        let shape = if d > 0.0 { (-(d / 60.0).powi(2)).exp() } else { (d / 150.0).exp() };
                        alpha * shape
                    });
                    continue;
                }
                Wave::Shudder => {
                    // out and back, twice, dying away
                    let reach = 360.0 * (std::f64::consts::PI * k * 2.0).sin().abs() * (1.0 - k);
                    front(reach, (1.0 - k) * rise, 0.25)
                }
            };
            lit |= paint(cr, w, h, cx, p.colour, &stops);
        }
        if let Some((colour, since, end)) = self.breath {
            let fade_in = smooth((now - since) / BREATH_FADE);
            let fade_out = end.map_or(1.0, |e| 1.0 - smooth((now - e) / BREATH_FADE));
            let phase = ((now - since) / BREATH_PERIOD).fract();
            let breathe = 0.55 + 0.25 * (std::f64::consts::TAU * (now - since) / BREATH_PERIOD).cos();
            let strength = fade_in * fade_out * breathe;
            // a steady faint rim, and a brighter wave going round on it
            let rim = 0.35 * strength;
            let wave: Vec<(f64, f64)> = front(ease_out(phase) * far, strength * (1.0 - phase * 0.6), 0.0)
                .into_iter().map(|(d, a)| (d, a.max(rim))).chain([(far + SOFT, rim)]).collect();
            lit |= paint(cr, w, h, cx, colour, &wave);
        }
        // the thicker frame goes over the light: the light comes out of it
        let (width, radius) = self.border;
        if width > 0.05 {
            cr.save().unwrap();
            cr.new_path();
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            cr.rectangle(0.0, 0.0, w, h);
            let inner = (radius - width).max(2.0);
            let (x0, y0, x1, y1) = (width, width, w - width, h - width);
            use std::f64::consts::{FRAC_PI_2, PI};
            cr.new_sub_path();
            cr.arc(x1 - inner, y0 + inner, inner, -FRAC_PI_2, 0.0);
            cr.arc(x1 - inner, y1 - inner, inner, 0.0, FRAC_PI_2);
            cr.arc(x0 + inner, y1 - inner, inner, FRAC_PI_2, PI);
            cr.arc(x0 + inner, y0 + inner, inner, PI, 1.5 * PI);
            cr.close_path();
            cr.set_source_rgb(0.0, 0.0, 0.0);
            let _ = cr.fill();
            cr.restore().unwrap();
            lit = true;
        }
        if !lit {
            return Vec::new();
        }
        strips(w, h)
    }
}

/// A point's way round the screen, clockwise from the island (0..perimeter).
fn way_round(side: Side, t: f64, w: f64, h: f64, cx: f64) -> f64 {
    let perimeter = 2.0 * (w + h);
    match side {
        Side::Top if t >= cx => t - cx,
        Side::Top => perimeter - (cx - t),
        Side::Right => (w - cx) + t,
        Side::Bottom => (w - cx) + h + (w - t),
        Side::Left => (w - cx) + h + w + (h - t),
    }
}

/// The timer's line: grown round from the island as far as its progress,
/// brighter at the head; or a stopwatch's comet.
fn paint_gauge(cr: &Context, w: f64, h: f64, cx: f64, g: &Gauge) -> bool {
    let perimeter = 2.0 * (w + h);
    let head = g.progress.clamp(0.0, 1.0) * perimeter;
    let dim = if g.running { 1.0 } else { 0.45 };
    paint_by(cr, w, h, g.colour, &|side, t| {
        let s = way_round(side, t, w, h, cx);
        let behind = if g.comet { (head - s).rem_euclid(perimeter) } else { head - s };
        let line = if g.comet {
            let k = 1.0 - behind / (COMET_TAIL * perimeter);
            if (0.0..=1.0).contains(&k) { GAUGE_LINE * k.powf(1.5) } else { 0.0 }
        } else if behind >= 0.0 {
            GAUGE_LINE
        } else {
            0.0
        };
        let tip = if behind >= -STEP && behind <= GAUGE_HEAD_LEN { GAUGE_HEAD * (1.0 - behind.max(0.0) / GAUGE_HEAD_LEN) } else { 0.0 };
        dim * (line + tip)
    })
}

/// The microphone's light: an orange LED in the frame's top-right corner,
/// in the black beyond the screen's rounded edge, with a little glow on
/// the screen beside it; `k` 0..1 is how far it is on. Returns its area.
pub fn mic_led(cr: &Context, w: f64, k: f64) -> Option<Rect> {
    if k <= 0.01 {
        return None;
    }
    let (x, y) = (w - LED_INSET, LED_INSET);
    let (r, g, b) = MIC_ORANGE;
    let halo = cairo::RadialGradient::new(x, y, 0.0, x, y, LED_HALO);
    halo.add_color_stop_rgba(0.0, r, g, b, 0.45 * k.min(1.0));
    halo.add_color_stop_rgba(1.0, r, g, b, 0.0);
    cr.save().unwrap();
    let _ = cr.set_source(&halo);
    cr.rectangle(x - LED_HALO, y - LED_HALO, 2.0 * LED_HALO, 2.0 * LED_HALO);
    let _ = cr.fill();
    cr.new_path();
    cr.arc(x, y, LED_R * k, 0.0, std::f64::consts::TAU);
    cr.set_source_rgb(r, g, b);
    let _ = cr.fill();
    cr.restore().unwrap();
    let left = x - LED_HALO - 1.0;
    Some(Rect { x: left, y: 0.0, w: w - left, h: y + LED_HALO + 1.0 })
}

/// iOS's microphone orange.
pub const MIC_ORANGE: Rgb = (1.0, 0.624, 0.039);
const LED_INSET: f64 = 4.2;
const LED_R: f64 = 2.6;
const LED_HALO: f64 = 11.0;

/// The thin strips along the four edges the light can reach.
pub fn strips(w: f64, h: f64) -> Vec<Rect> {
    let d = DEPTH + 2.0;
    vec![
        Rect { x: 0.0, y: 0.0, w, h: d },
        Rect { x: 0.0, y: h - d, w, h: d },
        Rect { x: 0.0, y: 0.0, w: d, h },
        Rect { x: w - d, y: 0.0, w: d, h },
    ]
}

/// A front of light at distance `at` from the island, of strength
/// `alpha`, leaving a trail of `tail` behind it: (distance, strength) stops.
fn front(at: f64, alpha: f64, tail: f64) -> Vec<(f64, f64)> {
    let behind = (at - TRAIL).max(0.0);
    let mut stops = vec![(0.0, alpha * tail)];
    if behind > 0.0 {
        stops.push((behind, alpha * tail));
    }
    stops.push((at, alpha));
    stops.push((at + SOFT, 0.0));
    stops
}

/// The screen's four edges.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// Light the screen's edges in `colour`, its strength along the way given
/// by `stops` (distance from the island's top centre, strength).
fn paint(cr: &Context, w: f64, h: f64, cx: f64, colour: Rgb, stops: &[(f64, f64)]) -> bool {
    if stops.iter().all(|s| s.1 <= 0.003) {
        return false;
    }
    paint_by(cr, w, h, colour, &|side, t| {
        let d = match side {
            Side::Top => (t - cx).abs(),
            Side::Bottom => (t - cx).hypot(h),
            Side::Left => cx.hypot(t),
            Side::Right => (w - cx).hypot(t),
        };
        strength_at(stops, d)
    })
}

/// Light the screen's edges in `colour`, its strength at a point given by
/// `strength(side, t)` (`t` from the left or top end of that edge). Each
/// edge is one composite: a gradient along it for the strength, through a
/// precomputed falloff into the screen. Returns whether anything was lit.
fn paint_by(cr: &Context, w: f64, h: f64, colour: Rgb, strength: &dyn Fn(Side, f64) -> f64) -> bool {
    let falloff = FALLOFF.with(|f| f.clone());
    let k = FALLOFF_N as f64 / DEPTH;
    // each edge: where it runs (start, direction, length), and how its
    // falloff is mapped (the column runs along the edge's normal; the
    // matrix must stay invertible)
    let edges = [
        (Side::Top, 0.0, 0.0, 1.0, 0.0, w, cairo::Matrix::new(1.0, 0.0, 0.0, k, 0.0, 0.0)),
        (Side::Bottom, 0.0, h, 1.0, 0.0, w, cairo::Matrix::new(1.0, 0.0, 0.0, -k, 0.0, k * h)),
        (Side::Left, 0.0, 0.0, 0.0, 1.0, h, cairo::Matrix::new(0.0, k, 1.0, 0.0, 0.0, 0.0)),
        (Side::Right, w, 0.0, 0.0, 1.0, h, cairo::Matrix::new(0.0, -k, 1.0, 0.0, 0.0, k * w)),
    ];
    let mut lit = false;
    for (side, x0, y0, dx, dy, len, matrix) in edges {
        let n = (len / STEP).ceil() as usize;
        let samples: Vec<(f64, f64)> = (0..=n).map(|i| {
            let t = (i as f64 * STEP).min(len);
            (t, strength(side, t))
        }).collect();
        let Some(first) = samples.iter().position(|s| s.1 > 0.003) else { continue };
        let last = samples.iter().rposition(|s| s.1 > 0.003).unwrap_or(first);
        let (lo, hi) = (first.saturating_sub(1), (last + 1).min(n));
        let (t0, t1) = (samples[lo].0, samples[hi].0.max(samples[lo].0 + 1.0));
        let gradient = cairo::LinearGradient::new(x0 + dx * t0, y0 + dy * t0, x0 + dx * t1, y0 + dy * t1);
        for (t, a) in &samples[lo..=hi] {
            gradient.add_color_stop_rgba(((t - t0) / (t1 - t0)).clamp(0.0, 1.0), colour.0, colour.1, colour.2, a.clamp(0.0, 1.0));
        }
        let mask = falloff.clone();
        mask.set_matrix(matrix);
        cr.save().unwrap();
        // only the lit stretch of the edge's band
        let (bx, by) = (x0 + dx * t0 - dy * if x0 > 0.0 { DEPTH } else { 0.0 }, y0 + dy * t0 - dx * if y0 > 0.0 { DEPTH } else { 0.0 });
        let (bw, bh) = if dx > 0.0 { (t1 - t0, DEPTH) } else { (DEPTH, t1 - t0) };
        cr.rectangle(bx, by, bw, bh);
        cr.clip();
        let _ = cr.set_source(&gradient);
        let _ = cr.mask(&mask);
        cr.restore().unwrap();
        lit = true;
    }
    lit
}

/// Spacing of the strength samples along an edge (logical px).
const STEP: f64 = 16.0;
/// Rows of the falloff image (over `DEPTH`).
const FALLOFF_N: usize = 48;

thread_local! {
    /// How the light dies away into the screen: a bright line at the edge
    /// over a wide soft glow (an A8 column, edge at row 0).
    static FALLOFF: cairo::SurfacePattern = {
        let mut image = cairo::ImageSurface::create(cairo::Format::A8, 1, FALLOFF_N as i32).expect("falloff");
        let stride = image.stride() as usize;
        {
            let mut data = image.data().expect("falloff data");
            for row in 0..FALLOFF_N {
                let d = (row as f64 + 0.5) * DEPTH / FALLOFF_N as f64;
                let a = 0.72 * (-d / 2.0).exp() + 0.30 * (-d / 7.0).exp();
                let edge_fade = 1.0 - (d / DEPTH).powi(3);
                data[row * stride] = (255.0 * (a * edge_fade).clamp(0.0, 1.0)).round() as u8;
            }
        }
        let pattern = cairo::SurfacePattern::create(&image);
        pattern.set_extend(cairo::Extend::Pad);
        // denser than the screen's pixels: nearest is as smooth, and much cheaper
        pattern.set_filter(cairo::Filter::Fast);
        pattern
    };
}

/// The strength at `d` from the island, between the stops.
fn strength_at(stops: &[(f64, f64)], d: f64) -> f64 {
    let Some(first) = stops.first() else { return 0.0 };
    if d <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let ((d0, a0), (d1, a1)) = (pair[0], pair[1]);
        if d <= d1 {
            return if d1 > d0 { a0 + (a1 - a0) * (d - d0) / (d1 - d0) } else { a1 };
        }
    }
    stops.last().map_or(0.0, |s| s.1)
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn ease_out(x: f64) -> f64 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(edge: &mut Edge, now: Time) -> (cairo::ImageSurface, Vec<Rect>) {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 400).unwrap();
        let rects = {
            let cr = Context::new(&surface).unwrap();
            edge.draw(&cr, 600.0, 400.0, 300.0, now)
        };
        (surface, rects)
    }

    fn alpha(surface: &mut cairo::ImageSurface, x: usize, y: usize) -> u8 {
        let stride = surface.stride() as usize;
        surface.data().unwrap()[y * stride + x * 4 + 3]
    }

    #[test]
    fn a_notification_runs_along_the_top_from_the_island() {
        let mut edge = Edge::default();
        let (_, dark) = render(&mut edge, 0.0);
        assert!(dark.is_empty(), "nothing to paint, nothing damaged");
        edge.pulse(Wave::Top, (0.2, 0.6, 1.0), 10.0);
        // early on: lit beside the island, not yet at the corner
        let (mut s, rects) = render(&mut edge, 10.15);
        assert_eq!(rects.len(), 4);
        assert!(alpha(&mut s, 330, 1) > 20, "{}", alpha(&mut s, 330, 1));
        assert!(alpha(&mut s, 20, 1) < alpha(&mut s, 330, 1));
        // the light stays at the edge
        assert_eq!(alpha(&mut s, 300, 200), 0);
        // later it has reached the corners and is fading; then it is gone
        let (mut s, _) = render(&mut edge, 10.6);
        assert!(alpha(&mut s, 40, 1) > 5);
        assert!(edge.is_animating(11.0));
        assert!(!edge.is_animating(11.3));
        let (_, rects) = render(&mut edge, 11.3);
        assert!(rects.is_empty());
    }

    #[test]
    fn a_critical_one_reaches_the_bottom() {
        let mut edge = Edge::default();
        edge.pulse(Wave::Around, (1.0, 0.2, 0.2), 0.0);
        let (mut s, _) = render(&mut edge, 0.9);
        assert!(alpha(&mut s, 300, 398) > 5);
    }

    #[test]
    fn a_timer_grows_round_the_screen_clockwise() {
        let mut edge = Edge::default();
        let gauge = |progress| Some(Gauge { progress, colour: (1.0, 0.6, 0.0), running: true, comet: false });
        // a quarter round (600x400: 2000 px): along the top right and
        // down the right edge, nothing on the left yet
        edge.set_gauge(gauge(0.25));
        let (mut s, rects) = render(&mut edge, 0.0);
        assert_eq!(rects.len(), 4);
        assert!(!edge.is_animating(0.0), "a still line costs no frames");
        assert!(alpha(&mut s, 500, 1) > 20 && alpha(&mut s, 598, 100) > 20);
        assert_eq!(alpha(&mut s, 100, 1), 0);
        assert_eq!(alpha(&mut s, 1, 200), 0);
        // nearly done: all the way round to the island's left
        edge.set_gauge(gauge(0.97));
        let (mut s, _) = render(&mut edge, 0.0);
        assert!(alpha(&mut s, 1, 200) > 20 && alpha(&mut s, 200, 1) > 20);
        // the gap left before the line meets its own start under the island
        assert_eq!(alpha(&mut s, 278, 1), 0);
        edge.set_gauge(None);
        let (_, rects) = render(&mut edge, 0.0);
        assert!(rects.is_empty());
    }

    #[test]
    fn scheduled_waves_wait_and_can_be_called_off() {
        let mut edge = Edge::default();
        edge.schedule(Wave::Flash, FAILURE_RED, 10.0);
        edge.schedule(Wave::Flash, FAILURE_RED, 11.0);
        edge.schedule(Wave::Flash, FAILURE_RED, 12.0);
        let (mut s, _) = render(&mut edge, 10.1);
        assert!(alpha(&mut s, 300, 1) > 20);
        // between the first two: dark, but still waiting for the next
        let (mut s, _) = render(&mut edge, 10.95);
        assert!(alpha(&mut s, 300, 1) < 10);
        assert!(edge.is_animating(10.95));
        edge.cancel_scheduled(11.05);
        assert!(!edge.is_animating(12.5), "the third was called off");
        assert!(edge.is_animating(11.5), "the started one plays out");
    }

    #[test]
    fn the_mic_led_sits_in_the_black_corner() {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 400).unwrap();
        let area = {
            let cr = Context::new(&surface).unwrap();
            assert!(mic_led(&cr, 600.0, 0.0).is_none());
            mic_led(&cr, 600.0, 1.0).expect("lit")
        };
        assert!(area.x + area.w <= 601.0 && area.y >= 0.0 && area.h < 20.0);
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let px = |x: usize, y: usize| &data[y * stride + x * 4..y * stride + x * 4 + 4];
        // premultiplied BGRA: an opaque orange centre
        assert_eq!(px(596, 4)[3], 255);
        assert!(px(596, 4)[2] > 240 && px(596, 4)[0] < 30);
    }

    #[test]
    fn levels_stand_up_their_side_and_embers_stay_in_the_bottom_corners() {
        let mut edge = Edge::default();
        edge.set_level(Some(Level { right: true, value: 0.5, shown: 1.0, colour: (1.0, 1.0, 1.0) }));
        let (mut s, _) = render(&mut edge, 0.0);
        assert!(alpha(&mut s, 599, 350) > 20, "the lower half of the right edge");
        assert_eq!(alpha(&mut s, 599, 100), 0);
        assert_eq!(alpha(&mut s, 0, 350), 0, "nothing on the left");
        edge.set_level(None);
        edge.set_ember(0.6);
        let (mut s, _) = render(&mut edge, 0.0);
        assert!(alpha(&mut s, 2, 398) > 20 && alpha(&mut s, 598, 398) > 20);
        assert_eq!(alpha(&mut s, 300, 399), 0);
        assert_eq!(alpha(&mut s, 0, 100), 0);
        assert!(!edge.is_animating(0.0), "a still glow costs no frames");
    }

    #[test]
    fn a_next_track_streaks_right_along_the_top() {
        let mut edge = Edge::default();
        edge.pulse(Wave::StreakRight, (1.0, 1.0, 1.0), 0.0);
        let (mut s, _) = render(&mut edge, 0.15);
        let right = alpha(&mut s, 450, 1);
        assert!(right > 20 && alpha(&mut s, 150, 1) < right / 4, "{right}");
        edge.set_ambient(Some(((1.0, 0.2, 0.2), 0.5)));
        let (mut s, _) = render(&mut edge, 1.0);
        assert!(alpha(&mut s, 300, 399) > 20 && alpha(&mut s, 300, 0) == 0);
    }

    #[test]
    fn focus_thickens_the_frame_evenly() {
        let mut edge = Edge::default();
        edge.set_border(4.0, 16.0);
        let (mut s, rects) = render(&mut edge, 0.0);
        assert_eq!(rects.len(), 4);
        for (x, y) in [(300, 2), (300, 397), (1, 200), (598, 200)] {
            assert_eq!(alpha(&mut s, x, y), 255, "{x},{y}");
        }
        assert_eq!(alpha(&mut s, 300, 6), 0);
        assert_eq!(alpha(&mut s, 6, 200), 0);
    }

    #[test]
    fn charging_rises_from_the_bottom_and_drains() {
        let mut edge = Edge::default();
        edge.pulse(Wave::Rise, SUCCESS_GREEN, 0.0);
        let (mut s, _) = render(&mut edge, 0.8);
        assert!(alpha(&mut s, 0, 300) > 20 && alpha(&mut s, 599, 300) > 20);
        assert_eq!(alpha(&mut s, 0, 60), 0, "it does not reach the top");
        let (_, rects) = render(&mut edge, 2.0);
        assert!(rects.is_empty());
    }

    #[test]
    fn way_round_is_continuous_clockwise() {
        let (w, h, cx) = (600.0, 400.0, 300.0);
        assert_eq!(way_round(Side::Top, 300.0, w, h, cx), 0.0);
        assert_eq!(way_round(Side::Top, 600.0, w, h, cx), way_round(Side::Right, 0.0, w, h, cx));
        assert_eq!(way_round(Side::Right, 400.0, w, h, cx), way_round(Side::Bottom, 600.0, w, h, cx));
        assert_eq!(way_round(Side::Bottom, 0.0, w, h, cx), way_round(Side::Left, 400.0, w, h, cx));
        assert_eq!(way_round(Side::Left, 0.0, w, h, cx), way_round(Side::Top, 0.0, w, h, cx));
        assert_eq!(way_round(Side::Top, 299.0, w, h, cx), 2.0 * (w + h) - 1.0);
    }

    #[test]
    fn touch_id_breathes_until_it_fades_out() {
        let mut edge = Edge::default();
        edge.breathe(Some(TOUCH_ID_BLUE), 0.0);
        let (mut s, _) = render(&mut edge, 1.0);
        assert!(alpha(&mut s, 1, 200) > 5 && alpha(&mut s, 300, 399) > 5);
        edge.breathe(None, 2.0);
        assert!(edge.is_animating(2.2));
        let (_, rects) = render(&mut edge, 2.0 + BREATH_FADE + 0.01);
        assert!(rects.is_empty() && !edge.is_animating(3.0));
    }
}

#[cfg(test)]
mod gallery {
    use super::*;

    /// Frames of each wave on a whole (scaled-down) screen.
    #[test]
    #[ignore = "writes target/gallery/edge-*.png; set ISLAND_GALLERY=1"]
    fn edge_filmstrips() {
        if std::env::var_os("ISLAND_GALLERY").is_none() { return; }
        let (w, h, scale) = (1504.0, 1002.0, 0.5);
        let shots: [(&str, Box<dyn Fn(&mut Edge)>, f64); 5] = [
            ("top", Box::new(|e: &mut Edge| e.pulse(Wave::Top, (0.165, 0.67, 0.93), 0.0)), Wave::Top.duration()),
            ("around", Box::new(|e: &mut Edge| e.pulse(Wave::Around, FAILURE_RED, 0.0)), Wave::Around.duration()),
            ("flash", Box::new(|e: &mut Edge| e.pulse(Wave::Flash, SUCCESS_GREEN, 0.0)), Wave::Flash.duration()),
            ("shudder", Box::new(|e: &mut Edge| e.pulse(Wave::Shudder, FAILURE_RED, 0.0)), Wave::Shudder.duration()),
            ("breath", Box::new(|e: &mut Edge| e.breathe(Some(TOUCH_ID_BLUE), 0.0)), BREATH_PERIOD),
        ];
        for (name, start, length) in shots {
            let frames = 6;
            let (fw, fh) = (w * scale, 140.0 * scale);
            let out = cairo::ImageSurface::create(cairo::Format::ARgb32, fw as i32, (fh * frames as f64) as i32 + 4 * frames).unwrap();
            let cr = Context::new(&out).unwrap();
            let mut edge = Edge::default();
            start(&mut edge);
            for n in 0..frames {
                let now = length * n as f64 / frames as f64 + 0.02;
                cr.save().unwrap();
                cr.translate(0.0, n as f64 * (fh + 4.0));
                cr.rectangle(0.0, 0.0, fw, fh);
                cr.clip();
                cr.scale(scale, scale);
                cr.set_source_rgb(0.10, 0.17, 0.22);
                cr.rectangle(0.0, 0.0, w, h);
                cr.fill().unwrap();
                edge.draw(&cr, w, h, w / 2.0, now);
                crate::bezel::paint(&cr, w, h, 16.0);
                cr.set_source_rgb(0.0, 0.0, 0.0);
                cr.rectangle(w / 2.0 - 100.0, 0.0, 200.0, 32.0);
                cr.fill().unwrap();
                cr.restore().unwrap();
            }
            drop(cr);
            std::fs::create_dir_all("target/gallery").unwrap();
            out.write_to_png(&mut std::fs::File::create(format!("target/gallery/edge-{name}.png")).unwrap()).unwrap();
        }
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    #[test]
    #[ignore = "timing; run with --release --ignored --nocapture"]
    fn edge_paint_cost() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 2256, 1504).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        for (name, wave) in [("top", Wave::Top), ("around", Wave::Around), ("flash", Wave::Flash)] {
            let mut edge = Edge::default();
            edge.pulse(wave, (0.2, 0.6, 1.0), 0.0);
            let started = std::time::Instant::now();
            let n = 60;
            for i in 0..n {
                edge.draw(&cr, 1504.0, 1002.0, 752.0, wave.duration() * i as f64 / n as f64);
            }
            eprintln!("{name}: {:.2} ms/frame", started.elapsed().as_secs_f64() * 1e3 / n as f64);
        }
        let mut edge = Edge::default();
        edge.breathe(Some(TOUCH_ID_BLUE), 0.0);
        let started = std::time::Instant::now();
        for i in 0..60 { edge.draw(&cr, 1504.0, 1002.0, 752.0, i as f64 / 60.0); }
        eprintln!("breath: {:.2} ms/frame", started.elapsed().as_secs_f64() * 1e3 / 60.0);
    }
}

#[cfg(test)]
mod gauge_gallery {
    use super::*;

    #[test]
    #[ignore = "writes target/gallery/edge-gauge.png; set ISLAND_GALLERY=1"]
    fn gauge_frames() {
        if std::env::var_os("ISLAND_GALLERY").is_none() { return; }
        let (w, h, scale) = (1504.0, 1002.0, 0.3);
        let shots = [(0.15, false, true), (0.5, false, true), (0.9, false, false), (0.4, true, true)];
        let (fw, fh) = (w * scale, h * scale);
        let out = cairo::ImageSurface::create(cairo::Format::Rgb24, (fw * 2.0 + 6.0) as i32, (fh * 2.0 + 6.0) as i32).unwrap();
        let cr = Context::new(&out).unwrap();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().unwrap();
        for (i, (progress, comet, running)) in shots.into_iter().enumerate() {
            let mut edge = Edge::default();
            let colour = if i == 2 { crate::menu_style::GREEN } else { crate::menu_style::ORANGE };
            edge.set_gauge(Some(Gauge { progress, colour, running, comet }));
            cr.save().unwrap();
            cr.translate((i % 2) as f64 * (fw + 6.0), (i / 2) as f64 * (fh + 6.0));
            cr.scale(scale, scale);
            cr.rectangle(0.0, 0.0, w, h);
            cr.clip();
            cr.set_source_rgb(0.10, 0.17, 0.22);
            cr.paint().unwrap();
            edge.draw(&cr, w, h, w / 2.0, 0.0);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.rectangle(w / 2.0 - 100.0, 0.0, 200.0, 32.0);
            cr.fill().unwrap();
            cr.restore().unwrap();
        }
        drop(cr);
        std::fs::create_dir_all("target/gallery").unwrap();
        out.write_to_png(&mut std::fs::File::create("target/gallery/edge-gauge.png").unwrap()).unwrap();
    }
}

/// The top edge of the frame as a membrane: pulled down after a tugged
/// island, and carrying ripples out to the corners (a let-go tug, the
/// island dropping into the edge), whose radius quivers when they arrive.
#[derive(Default)]
pub struct Membrane {
    ripples: Vec<Ripple>,
}

#[derive(Clone, Copy, Debug)]
struct Ripple {
    cx: f64,
    strength: f64,
    start: Time,
}

/// How fast a ripple runs along the edge (px/s), how long it lives, its
/// height at full strength and its width.
const RIPPLE_SPEED: f64 = 1300.0;
const RIPPLE_LIFE: f64 = 1.1;
const RIPPLE_HEIGHT: f64 = 9.0;
const RIPPLE_WIDTH: f64 = 70.0;
/// How wide the pulled-down membrane is round the island.
const SAG_WIDTH: f64 = 240.0;
/// Samples along the edge.
const MEMBRANE_STEP: f64 = 6.0;

impl Membrane {
    /// Send ripples out from `cx` (0..1 strength), starting at `at`.
    pub fn ripple(&mut self, cx: f64, strength: f64, at: Time) {
        if strength > 0.02 {
            self.ripples.push(Ripple { cx, strength: strength.min(1.0), start: at });
        }
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.ripples.iter().any(|r| now - r.start < RIPPLE_LIFE)
    }

    /// How far the edge is pushed down at `x` by the ripples.
    fn ripples_at(&self, x: f64, now: Time) -> f64 {
        self.ripples.iter().map(|r| {
            let t = now - r.start;
            if !(0.0..RIPPLE_LIFE).contains(&t) {
                return 0.0;
            }
            let d = (x - r.cx).abs();
            let front = RIPPLE_SPEED * t;
            // a leading crest and a smaller one after it, dying away
            let fade = (1.0 - t / RIPPLE_LIFE).powi(2) * (1.0 - 0.4 * (d / 900.0).min(1.0));
            let bump = |at: f64, k: f64| k * (-((d - at) / RIPPLE_WIDTH).powi(2)).exp();
            r.strength * RIPPLE_HEIGHT * fade * (bump(front, 1.0) + bump(front - 2.6 * RIPPLE_WIDTH, 0.45))
        }).sum()
    }

    /// How much a top corner's radius has grown: it quivers as a ripple
    /// arrives at `corner_x`.
    fn quiver(&self, corner_x: f64, now: Time) -> f64 {
        self.ripples.iter().map(|r| {
            let arrives = r.start + (corner_x - r.cx).abs() / RIPPLE_SPEED;
            let t = now - arrives;
            if !(0.0..0.45).contains(&t) {
                return 0.0;
            }
            r.strength * 3.5 * (-t / 0.14).exp() * (std::f64::consts::TAU * t / 0.11).sin().abs()
        }).sum()
    }

    /// Paint the membrane (black, over everything) for a screen `w` wide
    /// whose corners have `radius`: pulled down `sag` px round `cx`, plus the
    /// ripples. Returns what it covered.
    pub fn draw(&mut self, cr: &Context, w: f64, radius: f64, cx: f64, sag: f64, now: Time) -> Vec<Rect> {
        self.ripples.retain(|r| now - r.start < RIPPLE_LIFE);
        let mut covered = Vec::new();
        let n = (w / MEMBRANE_STEP).ceil() as usize;
        let ys: Vec<(f64, f64)> = (0..=n).map(|i| {
            let x = (i as f64 * MEMBRANE_STEP).min(w);
            let pull = sag * (-((x - cx) / SAG_WIDTH).powi(2)).exp();
            (x, pull + self.ripples_at(x, now))
        }).collect();
        let deepest = ys.iter().map(|p| p.1).fold(0.0, f64::max);
        cr.save().unwrap();
        cr.set_source_rgb(0.0, 0.0, 0.0);
        if deepest > 0.15 {
            cr.new_path();
            cr.move_to(0.0, 0.0);
            for &(x, y) in &ys {
                cr.line_to(x, y);
            }
            cr.line_to(w, 0.0);
            cr.close_path();
            let _ = cr.fill();
            covered.push(Rect { x: 0.0, y: 0.0, w, h: deepest + 2.0 });
        }
        // the top corners quiver as a ripple reaches them
        for (corner_x, sign) in [(0.0, 1.0), (w, -1.0)] {
            let grow = self.quiver(corner_x, now);
            if grow < 0.05 {
                continue;
            }
            let r = radius + grow;
            let (ax, ay) = (corner_x + sign * r, r);
            cr.new_path();
            cr.move_to(corner_x, 0.0);
            cr.line_to(corner_x, ay);
            if sign > 0.0 {
                cr.arc(ax, ay, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            } else {
                cr.arc_negative(ax, ay, r, 0.0, -0.5 * std::f64::consts::PI);
            }
            cr.close_path();
            let _ = cr.fill();
            covered.push(Rect { x: if sign > 0.0 { 0.0 } else { w - r - 1.0 }, y: 0.0, w: r + 1.0, h: r + 1.0 });
        }
        cr.restore().unwrap();
        covered
    }
}

#[cfg(test)]
mod membrane_tests {
    use super::*;

    fn alpha_after(m: &mut Membrane, sag: f64, now: Time, x: usize, y: usize) -> (u8, Vec<Rect>) {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 100).unwrap();
        let covered = {
            let cr = Context::new(&surface).unwrap();
            m.draw(&cr, 600.0, 16.0, 300.0, sag, now)
        };
        let stride = surface.stride() as usize;
        (surface.data().unwrap()[y * stride + x * 4 + 3], covered)
    }

    #[test]
    fn a_pulled_edge_sags_round_the_island_only() {
        let mut m = Membrane::default();
        let (a, covered) = alpha_after(&mut m, 0.0, 0.0, 300, 1);
        assert_eq!(a, 0);
        assert!(covered.is_empty(), "at rest it is nothing");
        assert_eq!(alpha_after(&mut m, 8.0, 0.0, 300, 5).0, 255);
        assert_eq!(alpha_after(&mut m, 8.0, 0.0, 300, 10).0, 0);
        assert!(alpha_after(&mut m, 8.0, 0.0, 20, 2).0 < 30, "far from the island the edge stays put");
    }

    #[test]
    fn a_ripple_runs_out_to_the_corners_and_they_quiver() {
        let mut m = Membrane::default();
        m.ripple(300.0, 1.0, 0.0);
        assert!(m.is_animating(0.5));
        // after 0.1 s the crest is ~130 px out
        assert!(alpha_after(&mut m, 0.0, 0.1, 430, 3).0 > 200);
        assert!(alpha_after(&mut m, 0.0, 0.1, 300, 3).0 < 100);
        // the corners: 300 px away, reached at ~0.23 s
        let (_, covered) = alpha_after(&mut m, 0.0, 0.3, 0, 0);
        assert!(covered.iter().any(|r| r.x == 0.0 && r.w > 16.0 && r.h > 16.0 && r.w < 30.0));
        assert!(!m.is_animating(RIPPLE_LIFE + 0.01));
    }
}
