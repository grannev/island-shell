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

/// How far along the edges a low battery's red reaches from the corners.
const EMBER_REACH: f64 = 70.0;

/// The island's fillets where it hangs from the screen's edge.
const ISLAND_FILLET: f64 = crate::menu_style::tight(14.0);

#[derive(Default)]
pub struct Edge {
    pulses: Vec<Pulse>,
    /// the island as last drawn: the light comes out of it
    island: Option<(Rect, f64)>,
    /// the bar's drops in the top corners (left, right): the light rounds
    /// them
    drops: [Option<crate::outline::Drop>; 2],
    /// the bottom corners smouldering red (low battery), how strongly
    ember: f64,
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

    /// The island's box and corner radius as drawn (None: not showing).
    pub fn set_island(&mut self, island: Option<(Rect, f64)>) {
        self.island = island;
    }

    /// The bar's drops as last drawn (None: not there).
    pub fn set_drops(&mut self, drops: [Option<crate::outline::Drop>; 2]) {
        self.drops = drops;
    }

    /// Light the bottom edge in the cover's colour (None: dark).
    pub fn set_ambient(&mut self, ambient: Option<(Rgb, f64)>) {
        self.ambient = ambient.filter(|a| a.1 > 0.005);
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
    pub fn draw(&mut self, cr: &Context, w: f64, h: f64, cx: f64, radius: f64, now: Time) -> Vec<Rect> {
        let r = radius.clamp(0.0, w.min(h) / 2.0);
        self.pulses.retain(|p| now - p.start < p.wave.duration());
        if self.breath.is_some_and(|(_, _, end)| end.is_some_and(|e| now - e >= BREATH_FADE)) {
            self.breath = None;
        }
        if self.pulses.is_empty() && self.breath.is_none()
            && self.ember <= 0.0 && self.ambient.is_none() {
            return Vec::new();
        }
        // the frame's inner edge, round the island and the bar's drops
        let island = self.island.map(|(rect, radius)| crate::outline::Island { rect, radius, fillet: ISLAND_FILLET });
        let outline = crate::outline::Outline::new(w, h, r, island, cx, self.drops);
        let half = outline.len / 2.0;
        let far = half + SOFT;
        // a notification's light runs past the corners a little way down
        let top_reach = outline.right_edge + 0.25 * h;
        let lead = island.map_or(0.0, |i| i.rect.w / 2.0 + i.rect.h);
        let mut lit = false;
        let mut painted: Vec<Rect> = Vec::new();
        // distance from the island along the loop, either way
        let both_ways = |s: f64| s.min(outline.len - s);
        if self.ember > 0.0 {
            let ember = self.ember;
            lit |= paint_by(cr, w, h, r, FAILURE_RED, &|side, t| {
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
            lit |= paint_by(cr, w, h, r, colour, &|side, t| match side {
                // brightest under the middle of the screen, as if lit from
                // behind it, and up the lower part of the sides
                Side::Bottom => strength * (0.55 + 0.45 * (std::f64::consts::PI * t / w).sin()),
                Side::Left | Side::Right => strength * 0.55 * ((t - 0.6 * h) / (0.4 * h)).clamp(0.0, 1.0),
                Side::Top => 0.0,
            });
        }
        for p in &self.pulses {
            if p.wave == Wave::Rise {
                let k = ((now - p.start) / p.wave.duration()).clamp(0.0, 1.0);
                // up quickly, a moment full, then draining away
                let fill = if k < 0.35 { ease_out(k / 0.35) } else if k < 0.5 { 1.0 } else { 1.0 - smooth((k - 0.5) / 0.5) };
                let alpha = smooth(k / 0.06) * (1.0 - smooth((k - 0.75) / 0.25));
                let surface = h * 0.55 * fill;
                lit |= paint_by(cr, w, h, r, p.colour, &|side, t| match side {
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
                Wave::Top => front(ease_out(k) * top_reach, (1.0 - k).powf(1.4) * rise, 0.18),
                Wave::Around => front(ease_out(k) * far, (1.0 - k).powf(1.2) * rise, 0.22),
                Wave::Flash => vec![(0.0, rise * (1.0 - smooth((k - 0.15) / 0.85))), (far, rise * (1.0 - smooth((k - 0.15) / 0.85)))],
                // drawn above, from the bottom corners
                Wave::Rise => continue,
                Wave::StreakRight | Wave::StreakLeft => {
                    let right = p.wave == Wave::StreakRight;
                    let at = (w / 2.0 + lead + 120.0) * ease_out(k);
                    let alpha = (1.0 - k).powf(1.3) * rise;
                    let len = outline.len;
                    painted.extend(outline.paint(cr, p.colour, &|s| {
                        // only its own way round, from the island
                        let along = if right { s } else { len - s };
                        if along > len / 2.0 { return 0.0; }
                        // a head and a tail behind it
                        let d = along - at;
                        let shape = if d > 0.0 { (-(d / 60.0).powi(2)).exp() } else { (d / 150.0).exp() };
                        alpha * shape
                    }));
                    continue;
                }
                Wave::Shudder => {
                    // out and back, twice, dying away
                    let reach = (lead + 260.0) * (std::f64::consts::PI * k * 2.0).sin().abs() * (1.0 - k);
                    front(reach, (1.0 - k) * rise, 0.25)
                }
            };
            if stops.iter().any(|s| s.1 > 0.003) {
                painted.extend(outline.paint(cr, p.colour, &|s| strength_at(&stops, both_ways(s))));
            }
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
            painted.extend(outline.paint(cr, colour, &|s| strength_at(&wave, both_ways(s))));
        }
        if lit {
            painted.extend(strips(w, h));
        }
        painted
    }
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
/// Light the screen's edges in `colour`, its strength at a point given by
/// `strength(side, t)` (`t` from the left or top end of that edge). Each
/// edge is one composite: a gradient along it for the strength, through a
/// precomputed falloff into the screen. Returns whether anything was lit.
fn paint_by(cr: &Context, w: f64, h: f64, r: f64, colour: Rgb, strength: &dyn Fn(Side, f64) -> f64) -> bool {
    let falloff_mask = FALLOFF.with(|f| f.clone());
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
        lit = true;
        // the straight stretch stops where the screen's corners round off
        let (c0, c1) = (t0.max(r), t1.min(len - r));
        if c1 <= c0 {
            continue;
        }
        let mask = falloff_mask.clone();
        mask.set_matrix(matrix);
        cr.save().unwrap();
        // only the lit stretch of the edge's band
        let (bx, by) = (x0 + dx * c0 - dy * if x0 > 0.0 { DEPTH } else { 0.0 }, y0 + dy * c0 - dx * if y0 > 0.0 { DEPTH } else { 0.0 });
        let (bw, bh) = if dx > 0.0 { (c1 - c0, DEPTH) } else { (DEPTH, c1 - c0) };
        cr.rectangle(bx, by, bw, bh);
        cr.clip();
        let _ = cr.set_source(&gradient);
        let _ = cr.mask(&mask);
        cr.restore().unwrap();
    }
    // the corners: the light follows the screen's rounded edge, dying away
    // inwards from the arc as it does from a straight edge
    if r > 0.5 {
        let corners = [
            ((0.0, 0.0), (r, r), (Side::Top, 0.0), (Side::Left, 0.0)),
            ((w - r, 0.0), (w - r, r), (Side::Top, w), (Side::Right, 0.0)),
            ((0.0, h - r), (r, h - r), (Side::Bottom, 0.0), (Side::Left, h)),
            ((w - r, h - r), (w - r, h - r), (Side::Bottom, w), (Side::Right, h)),
        ];
        for ((sx, sy), (ccx, ccy), (a, ta), (b, tb)) in corners {
            let s = strength(a, ta).max(strength(b, tb));
            if s <= 0.003 {
                continue;
            }
            let gradient = cairo::RadialGradient::new(ccx, ccy, 0.0, ccx, ccy, r);
            gradient.set_extend(cairo::Extend::Pad);
            // from the centre (deepest) out to the arc (the edge)
            let steps = 12;
            for i in (0..=steps).rev() {
                let d = r * i as f64 / steps as f64;
                gradient.add_color_stop_rgba((r - d) / r, colour.0, colour.1, colour.2, (s * falloff(d)).clamp(0.0, 1.0));
            }
            cr.save().unwrap();
            cr.rectangle(sx, sy, r, r);
            cr.clip();
            let _ = cr.set_source(&gradient);
            let _ = cr.paint();
            cr.restore().unwrap();
            lit = true;
        }
    }
    lit
}

/// How the light dies away `d` px into the screen: a bright line at the
/// edge over a wide soft glow.
fn falloff(d: f64) -> f64 {
    if d >= DEPTH {
        return 0.0;
    }
    let a = 0.72 * (-d / 2.0).exp() + 0.30 * (-d / 7.0).exp();
    (a * (1.0 - (d / DEPTH).powi(3))).clamp(0.0, 1.0)
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
                data[row * stride] = (255.0 * falloff(d)).round() as u8;
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
            edge.draw(&cr, 600.0, 400.0, 300.0, 16.0, now)
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
        assert!(!rects.is_empty());
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
    fn the_light_leaves_the_middle_of_the_islands_lower_edge() {
        let mut edge = Edge::default();
        // a 100x40 island hanging in the middle of a 600x400 screen
        edge.set_island(Some((Rect { x: 250.0, y: 0.0, w: 100.0, h: 40.0 }, 14.0)));
        edge.pulse(Wave::Top, (0.2, 0.6, 1.0), 10.0);
        // first it runs out along the island's bottom, the screen still dark
        let (mut s, rects) = render(&mut edge, 10.06);
        assert!(alpha(&mut s, 330, 41) > 20, "along the island's bottom: {}", alpha(&mut s, 330, 41));
        assert_eq!(alpha(&mut s, 550, 1), 0, "not along the frame yet");
        assert!(rects.iter().any(|r| r.x < 250.0 && r.y + r.h > 40.0), "the island's surround is damaged");
        // then up its sides and out along the top edge
        let (mut s, _) = render(&mut edge, 10.3);
        assert!(alpha(&mut s, 247, 10) > 10, "up the island's side");
        assert!(alpha(&mut s, 450, 1) > 20, "on along the frame");
    }

    #[test]
    fn a_critical_one_reaches_the_bottom() {
        let mut edge = Edge::default();
        edge.pulse(Wave::Around, (1.0, 0.2, 0.2), 0.0);
        let (mut s, _) = render(&mut edge, 0.9);
        assert!(alpha(&mut s, 300, 398) > 5);
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
    fn embers_stay_in_the_bottom_corners() {
        let mut edge = Edge::default();
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
                edge.draw(&cr, w, h, w / 2.0, 16.0, now);
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
                edge.draw(&cr, 1504.0, 1002.0, 752.0, 16.0, wave.duration() * i as f64 / n as f64);
            }
            eprintln!("{name}: {:.2} ms/frame", started.elapsed().as_secs_f64() * 1e3 / n as f64);
        }
        let mut edge = Edge::default();
        edge.breathe(Some(TOUCH_ID_BLUE), 0.0);
        let started = std::time::Instant::now();
        for i in 0..60 { edge.draw(&cr, 1504.0, 1002.0, 752.0, 16.0, i as f64 / 60.0); }
        eprintln!("breath: {:.2} ms/frame", started.elapsed().as_secs_f64() * 1e3 / 60.0);
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
const RIPPLE_LIFE: f64 = 1.4;
const RIPPLE_HEIGHT: f64 = 9.0;
const RIPPLE_WIDTH: f64 = 70.0;
/// How wide the pulled-down membrane is round the island.
const SAG_WIDTH: f64 = 170.0;
/// What bounces back off the screen's side, and how soon it is spent (px).
const REBOUND: f64 = 0.55;
const REBOUND_RUN: f64 = 70.0;
/// How much a corner swells when a full crest hits it (px), from how soon
/// before (s, the crest's shoulder arriving) and for how long.
const IMPACT: f64 = 12.0;
const IMPACT_LEAD: f64 = 0.04;
const IMPACT_LIFE: f64 = 0.5;

/// What hangs from the top edge, which the membrane's ripples ride: the
/// bar's drops in the top corners and the island between them, as drawn.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shore {
    /// the screen's corners (where no drop is)
    pub corner: f64,
    /// the drops' and the island's corner smoothing
    pub smoothing: f64,
    pub drops: [Option<crate::outline::Drop>; 2],
    /// the island: its box (hanging from the top), corners and fillets
    pub island: Option<(Rect, f64, f64)>,
}

/// The edge the ripples ride, from the screen's left side to its right,
/// sampled every `SHORE_STEP` px of its length: each sample's point, its
/// normal (out of the black, into the screen) and its distance along.
struct ShoreLine {
    points: Vec<(f64, f64, f64, f64, f64)>,
    /// how far along each side's corner (its middle) is
    walls: [f64; 2],
    len: f64,
}

/// Samples along the membrane's edge (px of its length).
const SHORE_STEP: f64 = 3.0;

impl Shore {
    /// The edge from the left side to the right, the corners where it meets
    /// the sides swollen by `grow`.
    fn line(&self, cr: &Context, w: f64, grow: [f64; 2]) -> ShoreLine {
        use crate::outline::Drop;
        let sm = self.smoothing;
        let (df, dr) = (crate::bar::FILLET, crate::bar::RADIUS);
        // a drop lifted into the edge (a fullscreen window) has its top
        // fillet above the screen
        let top = |d: Drop| (d.h - crate::bar::HEIGHT).min(0.0);
        let mut v: Vec<(f64, f64, f64, f64)> = Vec::new();
        // the left side: down the wall a little, round its corner (a drop's
        // fillet or the screen's), along the drop and up its side
        let wall_r = |d: Option<Drop>, g: f64| d.map_or((self.corner + g, 0.0), |_| (df + g, sm));
        let (lr, ls) = wall_r(self.drops[0], grow[0]);
        let run = |r: f64, s: f64| 2.0 * (1.0 + s) * r + 2.0;
        let lb = self.drops[0].map_or(0.0, |d| d.h);
        v.push((0.0, lb + run(lr, ls), 0.0, 0.0));
        v.push((0.0, lb, lr, ls));
        let wall_points = [(0.0, lb), (w, self.drops[1].map_or(0.0, |d| d.h))];
        let mut left_end = 0.0;
        if let Some(d) = self.drops[0] {
            v.push((d.w, d.h, dr, sm));
            v.push((d.w, top(d), df, sm));
            left_end = d.w;
        }
        let right_start = self.drops[1].map_or(w, |d| w - d.w);
        if let Some((b, r, f)) = self.island {
            let (x0, x1, y1) = (b.x, b.x + b.w, b.y + b.h);
            // (only clear of the drops: squeezed between them it is skipped)
            if y1 > 1.0 && x0 - 2.0 * f > left_end && x1 + 2.0 * f < right_start {
                v.push((x0, b.y, f, sm));
                v.push((x0, y1, r, sm));
                v.push((x1, y1, r, sm));
                v.push((x1, b.y, f, sm));
            }
        }
        let (rr, rs) = wall_r(self.drops[1], grow[1]);
        if let Some(d) = self.drops[1] {
            v.push((w - d.w, top(d), df, sm));
            v.push((w - d.w, d.h, dr, sm));
        }
        let rb = wall_points[1].1;
        v.push((w, rb, rr, rs));
        v.push((w, rb + run(rr, rs), 0.0, 0.0));
        // flattened by cairo, so it is the very curve the black was drawn by
        cr.save().unwrap();
        cr.identity_matrix();
        crate::drop::trace_open(cr, &v);
        let flat: Vec<(f64, f64)> = cr.copy_path_flat().map(|p| p.iter().filter_map(|seg| match seg {
            cairo::PathSegment::MoveTo((x, y)) | cairo::PathSegment::LineTo((x, y)) => Some((x, y)),
            _ => None,
        }).collect()).unwrap_or_default();
        cr.new_path();
        cr.restore().unwrap();
        // evenly along it, each sample's normal from the stretch it is on
        let mut points = Vec::new();
        let mut s = 0.0;
        let mut next = 0.0;
        for pair in flat.windows(2) {
            let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
            let len = (x1 - x0).hypot(y1 - y0);
            if len < 1e-6 {
                continue;
            }
            let (dx, dy) = ((x1 - x0) / len, (y1 - y0) / len);
            while next <= s + len {
                let k = next - s;
                points.push((x0 + dx * k, y0 + dy * k, -dy, dx, next));
                next += SHORE_STEP;
            }
            s += len;
        }
        if let (Some(&(x, y)), Some(last)) = (flat.last(), points.last().copied()) {
            if s - last.4 > 0.01 {
                points.push((x, y, last.2, last.3, s));
            }
        }
        let near = |(px, py): (f64, f64)| points.iter().min_by(|a, b| {
            let da = (a.0 - px).hypot(a.1 - py);
            let db = (b.0 - px).hypot(b.1 - py);
            da.total_cmp(&db)
        }).map_or(0.0, |p| p.4);
        ShoreLine { walls: [near(wall_points[0]), near(wall_points[1])], len: s, points }
    }
}

impl ShoreLine {
    /// How far along the edge a ripple sent from `x` on the top starts:
    /// where the edge passes over `x` (under the island when it is there).
    fn source(&self, x: f64) -> f64 {
        self.points.iter().min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs())).map_or(0.0, |p| p.4)
    }
}

impl Membrane {
    /// Send ripples out from `cx` (0..1 strength), starting at `at`.
    pub fn ripple(&mut self, cx: f64, strength: f64, at: Time) {
        if strength > 0.02 {
            self.ripples.push(Ripple { cx, strength: strength.min(1.0), start: at });
        }
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.ripples.iter().any(|r| now - r.start < RIPPLE_LIFE + IMPACT_LIFE)
    }

    /// How far the edge is pushed out at `s` along it by a ripple sent from
    /// `from` along it, whose wall on that side is `wall` away. A crest runs
    /// into the screen's side as into a wall: it piles up against it, the
    /// little that bounces back dies within a stretch, and what goes on
    /// round the corner is swallowed there.
    fn ripple_at(r: &Ripple, from: f64, wall: f64, s: f64, now: Time) -> f64 {
        let t = now - r.start;
        if !(0.0..RIPPLE_LIFE).contains(&t) {
            return 0.0;
        }
        let d = (s - from).abs();
        let front = RIPPLE_SPEED * t;
        // a leading crest and a smaller one after it, dying away
        let fade = (1.0 - t / RIPPLE_LIFE).powf(1.2) * (1.0 - 0.25 * (d / 900.0).min(1.0));
        let bump = |d: f64, at: f64, k: f64| k * (-((d - at) / RIPPLE_WIDTH).powi(2)).exp();
        let crests = |d: f64| bump(d, front, 1.0) + bump(d, front - 2.6 * RIPPLE_WIDTH, 0.45);
        // its height goes into the corner's swell over the last stretch
        // before it, so nothing is cut off half way round the corner
        let swallowed = smooth((wall - d) / SWALLOW);
        let past = (front - wall).max(0.0);
        let rebound = if d <= wall { REBOUND * (-past / REBOUND_RUN).exp() * crests(2.0 * wall - d) } else { 0.0 };
        r.strength * RIPPLE_HEIGHT * fade * (crests(d) + rebound) * swallowed
    }

    /// How much a ripple sent from `from` swells the corner `wall` away
    /// along the edge: a crest slams into it, it bulges and shudders,
    /// settling.
    fn impact(r: &Ripple, wall: f64, now: Time) -> f64 {
        let arrives = r.start + wall / RIPPLE_SPEED;
        let t = now - arrives;
        if !(-IMPACT_LEAD..IMPACT_LIFE).contains(&t) || arrives - r.start >= RIPPLE_LIFE {
            return 0.0;
        }
        // as strong as the crest that hits
        let age = arrives - r.start;
        let fade = ((1.0 - age / RIPPLE_LIFE).powf(1.2) * (1.0 - 0.25 * (wall / 900.0).min(1.0))).sqrt();
        let swell = if t < 0.0 {
            smooth(1.0 + t / IMPACT_LEAD)
        } else {
            (-t / 0.16).exp() * (0.7 + 0.3 * (std::f64::consts::TAU * t / 0.1).cos())
        };
        r.strength * fade * IMPACT * swell
    }

    /// Paint the membrane (black, over everything) for a screen `w` wide:
    /// pulled down `sag` px round `cx`, plus the ripples. Returns what it
    /// covered.
    /// (`attach`: how far either side of `cx` the edge runs level, pulled
    /// down by the full `sag`: the island's fillets end there.)
    /// (`shore`: what hangs from the top edge. The ripples run along one
    /// edge from one side of the screen to the other — the drops' and the
    /// island's lower edges, round their corners and fillets, and the top
    /// between them — and end against the corners where it meets the
    /// screen's sides, which swell as they hit.)
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, cr: &Context, w: f64, shore: &Shore, cx: f64, attach: f64, sag: f64, now: Time) -> Vec<Rect> {
        self.ripples.retain(|r| now - r.start < RIPPLE_LIFE + IMPACT_LIFE);
        if self.ripples.is_empty() && sag < 0.05 {
            return Vec::new();
        }
        // where each ripple starts along the edge and how far its walls are
        let line = shore.line(cr, w, [0.0, 0.0]);
        let walls = |line: &ShoreLine| -> Vec<(Ripple, f64, [f64; 2])> {
            self.ripples.iter().map(|r| {
                let from = line.source(r.cx);
                (*r, from, [(from - line.walls[0]).max(0.0), (line.walls[1] - from).max(0.0)])
            }).collect()
        };
        let grow = {
            let ripples = walls(&line);
            [0, 1].map(|side| ripples.iter().map(|(r, _, wall)| Self::impact(r, wall[side], now)).sum::<f64>())
        };
        let line = if grow.iter().any(|g| *g > 0.05) { shore.line(cr, w, grow) } else { line };
        let ripples = walls(&line);
        let depth: Vec<f64> = line.points.iter().map(|&(x, y, _, ny, s)| {
            // the tug's pull: the top edge, level over the island, easing
            // back up
            let out = ((x - cx).abs() - attach).max(0.0);
            let pull = if y.abs() < 0.5 && ny > 0.9 { sag * (-(out / SAG_WIDTH).powi(2)).exp() } else { 0.0 };
            let wave: f64 = ripples.iter().map(|(r, from, wall)| {
                Self::ripple_at(r, *from, if s < *from { wall[0] } else { wall[1] }, s, now)
            }).sum();
            // nothing at the very ends, down the sides
            let ends = smooth(s.min(line.len - s) / END_TAPER);
            (pull + wave) * ends
        }).collect();
        let mut covered = Vec::new();
        cr.save().unwrap();
        cr.set_source_rgb(0.0, 0.0, 0.0);
        // each stretch that moves: black from just inside the edge out to
        // where it has been pushed
        let n = depth.len();
        let mut i = 0;
        while i < n {
            if depth[i] < 0.05 {
                i += 1;
                continue;
            }
            let a = i.saturating_sub(1);
            while i < n && depth[i] >= 0.05 {
                i += 1;
            }
            let b = i.min(n - 1);
            let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
            cr.new_path();
            for k in a..=b {
                let (x, y, nx, ny, _) = line.points[k];
                let (px, py) = (x - nx * INSIDE, y - ny * INSIDE);
                cr.line_to(px, py);
                (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
            }
            for k in (a..=b).rev() {
                let (x, y, nx, ny, _) = line.points[k];
                let (px, py) = (x + nx * depth[k], y + ny * depth[k]);
                cr.line_to(px, py);
                (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
            }
            cr.close_path();
            let _ = cr.fill();
            covered.push(Rect { x: x0 - 2.0, y: (y0 - 2.0).max(0.0), w: x1 - x0 + 4.0, h: y1 - y0.max(0.0) + 4.0 });
        }
        // the corners at the sides, swollen by a crest slamming into them
        for (side, g) in grow.into_iter().enumerate() {
            if g < 0.05 {
                continue;
            }
            let (wall_x, base, r, smoothing) = match (side, shore.drops[side]) {
                (0, Some(d)) => (0.0, d.h, crate::bar::FILLET + g, shore.smoothing),
                (_, Some(d)) => (w, d.h, crate::bar::FILLET + g, shore.smoothing),
                (0, None) => (0.0, 0.0, shore.corner + g, 0.0),
                (_, None) => (w, 0.0, shore.corner + g, 0.0),
            };
            let sign = if side == 0 { 1.0 } else { -1.0 };
            let x = |dx: f64| wall_x + sign * dx;
            let run = 2.0 * (1.0 + smoothing) * r + 1.0;
            crate::drop::trace(cr, &[
                (x(run), base, 0.0),
                (x(0.0), base, r),
                (x(0.0), base + run, 0.0),
                (x(-1.0), base + run, 0.0),
                (x(-1.0), base - 1.0, 0.0),
                (x(run), base - 1.0, 0.0),
            ], smoothing);
            let _ = cr.fill();
            covered.push(Rect { x: if side == 0 { 0.0 } else { w - run - 1.0 }, y: (base - 2.0).max(0.0), w: run + 1.0, h: run + 3.0 });
        }
        cr.restore().unwrap();
        covered
    }
}

/// How far inside the black the membrane's fill begins (so it never leaves
/// a seam along the edge), over how much before a side's corner a crest is
/// swallowed and over how much of the edge's ends it fades to nothing.
const INSIDE: f64 = 1.0;
const SWALLOW: f64 = 28.0;
const END_TAPER: f64 = 10.0;

#[cfg(test)]
mod membrane_tests {
    use super::*;
    use crate::outline::Drop;

    const BARE: Shore = Shore { corner: 16.0, smoothing: 0.0, drops: [None, None], island: None };

    /// An island 250..350 down to 32, drops 120 and 150 wide, 32 tall.
    fn bar() -> Shore {
        Shore {
            corner: 16.0,
            smoothing: 0.6,
            drops: [Some(Drop { w: 120.0, h: 32.0 }), Some(Drop { w: 150.0, h: 32.0 })],
            island: Some((Rect { x: 250.0, y: 0.0, w: 100.0, h: 32.0 }, 14.0, 14.0)),
        }
    }

    fn render(m: &mut Membrane, shore: &Shore, sag: f64, now: Time) -> (cairo::ImageSurface, Vec<Rect>) {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 100).unwrap();
        let covered = {
            let cr = Context::new(&surface).unwrap();
            m.draw(&cr, 600.0, shore, 300.0, 0.0, sag, now)
        };
        (surface, covered)
    }

    fn alpha(surface: &mut cairo::ImageSurface, x: usize, y: usize) -> u8 {
        let stride = surface.stride() as usize;
        surface.data().unwrap()[y * stride + x * 4 + 3]
    }

    fn alpha_after(m: &mut Membrane, sag: f64, now: Time, x: usize, y: usize) -> (u8, Vec<Rect>) {
        let (mut s, covered) = render(m, &BARE, sag, now);
        (alpha(&mut s, x, y), covered)
    }

    /// Frames over `t0..t1`, 1 ms apart: does any have (x, y) black?
    fn ever(m: &mut Membrane, shore: &Shore, t0: Time, t1: Time, x: usize, y: usize) -> bool {
        (0..((t1 - t0) * 1000.0) as usize).any(|i| alpha(&mut render(m, shore, 0.0, t0 + i as f64 / 1000.0).0, x, y) > 250)
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
    fn a_ripple_rides_round_every_turn_of_the_edge() {
        let mut m = Membrane::default();
        let shore = bar();
        m.ripple(300.0, 1.0, 0.0);
        // under the island, along its lower edge
        assert!(ever(&mut m, &shore, 0.0, 0.1, 330, 35), "along the island");
        // down the island's side, beside it
        assert!(ever(&mut m, &shore, 0.0, 0.2, 353, 22), "down the island's side");
        // along the top between the island and the drop: never gone flat
        // round the turns — every px along it is crossed by the crest
        for x in [366, 380, 420, 440] {
            assert!(ever(&mut m, &shore, 0.0, 0.4, x, 3), "along the top at {x}");
        }
        // down the drop's side and along its lower edge
        assert!(ever(&mut m, &shore, 0.0, 0.5, 447, 20), "down the drop's side");
        assert!(ever(&mut m, &shore, 0.0, 0.6, 520, 35), "along the drop");
        // nothing far below the edge, ever
        assert!(!ever(&mut m, &shore, 0.0, 0.6, 520, 60));
    }

    #[test]
    fn a_ripple_ends_against_the_corner_at_the_screens_side() {
        let mut m = Membrane::default();
        let shore = bar();
        m.ripple(300.0, 1.0, 0.0);
        // it reaches the side at full height, and the drop's fillet there
        // swells as it hits
        assert!(ever(&mut m, &shore, 0.2, 0.7, 597, 40), "the crest reaches the side");
        assert!(ever(&mut m, &shore, 0.2, 0.7, 599, 44), "the fillet bulges");
        // nothing comes back across the drop, and the corner settles
        assert!(!ever(&mut m, &shore, 0.75, 0.9, 480, 34), "spent against the wall");
        assert!(!ever(&mut m, &shore, 1.0, 1.2, 597, 47), "the corner settles");
    }

    #[test]
    fn a_ripple_runs_out_to_the_bare_corners_and_they_swell() {
        let mut m = Membrane::default();
        m.ripple(300.0, 1.0, 0.0);
        assert!(m.is_animating(0.5));
        // after 0.1 s the crest is ~130 px out
        assert!(alpha_after(&mut m, 0.0, 0.1, 430, 3).0 > 200);
        assert!(alpha_after(&mut m, 0.0, 0.1, 300, 3).0 < 100);
        // the corners, ~300 px away: black where the screen's arc was open
        assert!(ever(&mut m, &BARE, 0.2, 0.35, 5, 5));
        assert!(!ever(&mut m, &BARE, 1.0, 1.2, 5, 5), "and back");
        assert!(!m.is_animating(RIPPLE_LIFE + IMPACT_LIFE + 0.01));
    }
}

#[cfg(test)]
mod corner_look {
    use super::*;
    #[test]
    #[ignore = "writes target/gallery/edge-corner.png"]
    fn corner_closeup() {
        let (w, h) = (1504.0, 1002.0);
        let out = cairo::ImageSurface::create(cairo::Format::Rgb24, 600, 300).unwrap();
        let cr = Context::new(&out).unwrap();
        for (i, wave) in [Some(Wave::Flash), None].into_iter().enumerate() {
            let mut edge = Edge::default();
            match wave { Some(w) => edge.pulse(w, TOUCH_ID_BLUE, 0.0), None => edge.breathe(Some(TOUCH_ID_BLUE), 0.0) }
            cr.save().unwrap();
            cr.translate(i as f64 * 300.0, 0.0);
            cr.rectangle(0.0, 0.0, 300.0, 300.0);
            cr.clip();
            cr.scale(1.5 * 4.0, 1.5 * 4.0);
            cr.set_source_rgb(0.10, 0.17, 0.22);
            cr.paint().unwrap();
            edge.draw(&cr, w, h, w / 2.0, 16.0, 0.12);
            crate::bezel::paint(&cr, w, h, 16.0);
            cr.restore().unwrap();
        }
        drop(cr);
        out.write_to_png(&mut std::fs::File::create("target/gallery/edge-corner.png").unwrap()).unwrap();
    }
}

#[cfg(test)]
mod island_light_gallery {
    use super::*;
    #[test]
    #[ignore = "writes target/gallery/edge-from-island.png"]
    fn from_island() {
        let (w, h, scale) = (1504.0, 1002.0, std::env::var("EDGE_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.6));
        let island = Rect { x: 542.0, y: 0.0, w: 420.0, h: 92.0 };
        let (fw, fh) = (w * scale, 200.0 * scale);
        let times = [0.05, 0.12, 0.2, 0.32, 0.5, 0.75];
        let out = cairo::ImageSurface::create(cairo::Format::Rgb24, fw as i32, ((fh + 3.0) * times.len() as f64) as i32).unwrap();
        let cr = Context::new(&out).unwrap();
        for (i, t) in times.into_iter().enumerate() {
            let mut edge = Edge::default();
            edge.set_island(Some((island, 30.0)));
            edge.pulse(Wave::Top, (0.165, 0.67, 0.93), 0.0);
            cr.save().unwrap();
            cr.translate(0.0, i as f64 * (fh + 3.0));
            cr.rectangle(0.0, 0.0, fw, fh);
            cr.clip();
            cr.scale(scale, scale);
            cr.set_source_rgb(0.10, 0.17, 0.22);
            cr.paint().unwrap();
            edge.draw(&cr, w, h, w / 2.0, 16.0, t);
            // the island over it
            let (x0, x1, b, r) = (island.x, island.x + island.w, island.h, 30.0);
            cr.new_path();
            cr.move_to(x0 - 14.0, 0.0);
            cr.arc(x0 - 14.0, 14.0, 14.0, -0.5 * std::f64::consts::PI, 0.0);
            cr.arc_negative(x0 + r, b - r, r, std::f64::consts::PI, 0.5 * std::f64::consts::PI);
            cr.arc_negative(x1 - r, b - r, r, 0.5 * std::f64::consts::PI, 0.0);
            cr.arc(x1 + 14.0, 14.0, 14.0, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.fill().unwrap();
            crate::bezel::paint(&cr, w, h, 16.0);
            cr.restore().unwrap();
        }
        drop(cr);
        out.write_to_png(&mut std::fs::File::create("target/gallery/edge-from-island.png").unwrap()).unwrap();
    }
}

#[cfg(test)]
mod membrane_cost {
    use super::*;

    /// What a frame of a running ripple costs at 1.5×, with the bar's drops
    /// and the island (`cargo test --release membrane_frame_cost -- --ignored --nocapture`).
    #[test]
    #[ignore = "a timing, not a check"]
    fn membrane_frame_cost() {
        use crate::outline::Drop;
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 2256, 200).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        let shore = Shore {
            corner: 16.0,
            smoothing: 0.6,
            drops: [Some(Drop { w: 140.0, h: 32.0 }), Some(Drop { w: 420.0, h: 32.0 })],
            island: Some((Rect { x: 640.0, y: 0.0, w: 224.0, h: 32.0 }, 14.0, 14.0)),
        };
        let mut m = Membrane::default();
        m.ripple(752.0, 1.0, 0.0);
        let mut times = Vec::new();
        for i in 0..110 {
            let now = i as f64 / 60.0;
            let t = std::time::Instant::now();
            m.draw(&cr, 1504.0, &shore, 752.0, 0.0, 0.0, now);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        println!("membrane frame: median={:.3}ms p95={:.3}ms max={:.3}ms", times[times.len() / 2], times[times.len() * 95 / 100], times[times.len() - 1]);
    }
}

#[cfg(test)]
mod edge_cost {
    use super::*;

    /// What a frame of light round the whole screen costs at 1.5×
    /// (`cargo test --release edge_frame_cost -- --ignored --nocapture`).
    #[test]
    #[ignore = "a timing, not a check"]
    fn edge_frame_cost() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 2256, 1504).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        let mut edge = Edge::default();
        edge.set_island(Some((Rect { x: 640.0, y: 0.0, w: 224.0, h: 32.0 }, 14.0)));
        edge.set_drops([Some(crate::outline::Drop { w: 140.0, h: 32.0 }), Some(crate::outline::Drop { w: 420.0, h: 32.0 })]);
        edge.breathe(Some(TOUCH_ID_BLUE), 0.0);
        let mut times = Vec::new();
        for i in 0..60 {
            let now = 1.0 + i as f64 / 60.0;
            let t = std::time::Instant::now();
            edge.draw(&cr, 1504.0, 1002.0, 752.0, 16.0, now);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        println!("edge frame: median={:.3}ms p95={:.3}ms max={:.3}ms", times[30], times[57], times[59]);
    }
}

#[cfg(test)]
mod wave_tail_look {
    use super::*;

    /// A notification's wave dying away round the island's and the right
    /// drop's turns, close up.
    #[test]
    #[ignore = "writes target/gallery/wave-tail.png"]
    fn wave_tail() {
        let (w, h) = (1504.0, 1002.0);
        let times = [0.25, 0.55, 0.85, 1.1];
        let (cw, ch, zoom) = (300.0, 60.0, 3.0);
        let out = cairo::ImageSurface::create(cairo::Format::Rgb24, (cw * zoom) as i32, (ch * zoom * times.len() as f64) as i32).unwrap();
        let cr = Context::new(&out).unwrap();
        for (row, t) in times.into_iter().enumerate() {
            let mut edge = Edge::default();
            edge.set_island(Some((Rect { x: 640.0, y: 0.0, w: 224.0, h: 32.0 }, 14.0)));
            edge.set_drops([None, Some(crate::outline::Drop { w: 420.0, h: 32.0 })]);
            edge.pulse(Wave::Top, TOUCH_ID_BLUE, 0.0);
            cr.save().unwrap();
            cr.translate(0.0, row as f64 * ch * zoom);
            cr.rectangle(0.0, 0.0, cw * zoom, ch * zoom - 3.0);
            cr.clip();
            cr.scale(zoom, zoom);
            cr.translate(-(864.0 - 40.0), 0.0);
            cr.set_source_rgb(0.10, 0.17, 0.22);
            cr.paint().unwrap();
            edge.draw(&cr, w, h, 752.0, 16.0, t);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            crate::drop::notch(&cr, 640.0, 864.0, 0.0, 32.0, 14.0, 14.0, 0.0);
            let _ = cr.fill();
            crate::drop::corner(&cr, w, true, 420.0, 32.0, 14.0, 14.0, 0.0);
            let _ = cr.fill();
            cr.restore().unwrap();
        }
        drop(cr);
        out.write_to_png(&mut std::fs::File::create("target/gallery/wave-tail.png").unwrap()).unwrap();
    }
}
