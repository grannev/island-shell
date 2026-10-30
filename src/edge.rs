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
}

impl Wave {
    fn duration(self) -> f64 {
        match self {
            Wave::Top => 1.15,
            Wave::Around => 1.7,
            Wave::Flash => 0.75,
            Wave::Shudder => 0.6,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pulse {
    wave: Wave,
    colour: Rgb,
    start: Time,
}

#[derive(Default)]
pub struct Edge {
    pulses: Vec<Pulse>,
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
        if self.pulses.is_empty() && self.breath.is_none() {
            return Vec::new();
        }
        let far = (w / 2.0 + cx.abs().min(w)).hypot(h) + SOFT;
        let mut lit = false;
        for p in &self.pulses {
            let k = ((now - p.start) / p.wave.duration()).clamp(0.0, 1.0);
            let rise = smooth(k / 0.08);
            let stops: Vec<(f64, f64)> = match p.wave {
                Wave::Top => front(ease_out(k) * 0.64 * w, (1.0 - k).powf(1.4) * rise, 0.18),
                Wave::Around => front(ease_out(k) * far, (1.0 - k).powf(1.2) * rise, 0.22),
                Wave::Flash => vec![(0.0, rise * (1.0 - smooth((k - 0.15) / 0.85))), (far, rise * (1.0 - smooth((k - 0.15) / 0.85)))],
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
        if !lit {
            return Vec::new();
        }
        strips(w, h)
    }
}

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

/// Light the screen's edges in `colour`, its strength along the way given
/// by `stops` (distance from the island's top centre, strength). Each edge
/// is one composite: a gradient along it for the strength, through a
/// precomputed falloff into the screen. Returns whether anything was lit.
fn paint(cr: &Context, w: f64, h: f64, cx: f64, colour: Rgb, stops: &[(f64, f64)]) -> bool {
    if stops.iter().all(|s| s.1 <= 0.003) {
        return false;
    }
    let falloff = FALLOFF.with(|f| f.clone());
    let k = FALLOFF_N as f64 / DEPTH;
    // each edge: where it runs (start, direction, length), the distance
    // from the island of a point on it, and how its falloff is mapped (the
    // column runs along the edge's normal; the matrix must stay invertible)
    let edges: [(f64, f64, f64, f64, f64, Box<dyn Fn(f64) -> f64>, cairo::Matrix); 4] = [
        (0.0, 0.0, 1.0, 0.0, w, Box::new(move |t| (t - cx).abs()), cairo::Matrix::new(1.0, 0.0, 0.0, k, 0.0, 0.0)),
        (0.0, h, 1.0, 0.0, w, Box::new(move |t| (t - cx).hypot(h)), cairo::Matrix::new(1.0, 0.0, 0.0, -k, 0.0, k * h)),
        (0.0, 0.0, 0.0, 1.0, h, Box::new(move |t| cx.hypot(t)), cairo::Matrix::new(0.0, k, 1.0, 0.0, 0.0, 0.0)),
        (w, 0.0, 0.0, 1.0, h, Box::new(move |t| (w - cx).hypot(t)), cairo::Matrix::new(0.0, -k, 1.0, 0.0, 0.0, k * w)),
    ];
    let mut lit = false;
    for (x0, y0, dx, dy, len, distance, matrix) in edges {
        let n = (len / STEP).ceil() as usize;
        let samples: Vec<(f64, f64)> = (0..=n).map(|i| {
            let t = (i as f64 * STEP).min(len);
            (t, strength_at(stops, distance(t)))
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
