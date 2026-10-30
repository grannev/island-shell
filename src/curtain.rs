//! The screen switching off into the island, like an old television: the
//! picture collapses up into a band along the top edge, the band narrows
//! into the island, and the frame's black has closed over everything else.
//! Waking, the island lets the screen out again the other way round.
//!
//! The curtain is black over the screen with a rounded window cut out of it
//! (the picture still showing); the island is drawn over it.

use cairo::Context;

use crate::anim::{min_jerk, Time};
use crate::island::Rect;

const CLOSE: f64 = 0.6;
const OPEN: f64 = 0.65;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Closing(Time),
    Off,
    Opening(Time),
}

#[derive(Default)]
pub struct Curtain {
    state: Option<State>,
}

impl Curtain {
    /// Switch the screen off into the island.
    pub fn close(&mut self, now: Time) {
        if !matches!(self.state, Some(State::Closing(_) | State::Off)) {
            self.state = Some(State::Closing(now));
        }
    }

    /// Let the screen out of the island again.
    pub fn open(&mut self, now: Time) {
        if matches!(self.state, Some(State::Closing(_) | State::Off)) {
            self.state = Some(State::Opening(now));
        }
    }

    /// Has the screen gone all the way into the island?
    pub fn is_off(&self, now: Time) -> bool {
        match self.state {
            Some(State::Off) => true,
            Some(State::Closing(since)) => now - since >= CLOSE,
            _ => false,
        }
    }

    pub fn is_animating(&self, now: Time) -> bool {
        match self.state {
            Some(State::Closing(since)) => now - since < CLOSE + 0.05,
            Some(State::Opening(since)) => now - since < OPEN + 0.05,
            _ => false,
        }
    }

    /// Paint the curtain on an output `w`×`h` whose corners have `radius`;
    /// the screen closes into `island`. Returns what it covered.
    pub fn draw(&mut self, cr: &Context, w: f64, h: f64, radius: f64, island: Rect, now: Time) -> Option<Rect> {
        let window = match self.state? {
            State::Closing(since) => {
                let t = (now - since) / CLOSE;
                if t >= 1.0 {
                    self.state = Some(State::Off);
                }
                closing(w, h, radius, island, t.clamp(0.0, 1.0))
            }
            State::Off => None,
            State::Opening(since) => {
                let t = (now - since) / OPEN;
                if t >= 1.0 {
                    self.state = None;
                    // one last frame: wipe the curtain away
                    return Some(Rect { x: 0.0, y: 0.0, w, h });
                }
                opening(w, h, radius, island, t.clamp(0.0, 1.0))
            }
        };
        cr.save().unwrap();
        cr.new_path();
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        cr.rectangle(0.0, 0.0, w, h);
        if let Some((r, corner)) = window {
            rounded(cr, r, corner);
        }
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.fill();
        // the picture's last light lingers on the collapsing edge
        if let Some((r, corner)) = window {
            let glow = afterglow(r, h);
            if glow > 0.01 {
                rounded(cr, Rect { x: r.x + 0.75, y: r.y - 1.0, w: (r.w - 1.5).max(0.0), h: r.h + 0.25 }, corner);
                cr.set_line_width(1.5);
                cr.set_source_rgba(0.85, 0.95, 1.0, glow);
                let _ = cr.stroke();
            }
        }
        cr.restore().unwrap();
        Some(Rect { x: 0.0, y: 0.0, w, h })
    }
}

/// The window still showing the picture while it closes: first its bottom
/// rises to a band under the top edge, then the band narrows into the
/// island. (Rect, corner radius.)
fn closing(w: f64, h: f64, radius: f64, island: Rect, t: f64) -> Option<(Rect, f64)> {
    let band = island.h.max(1.0);
    let v = min_jerk((t / 0.55).clamp(0.0, 1.0));
    let u = min_jerk(((t - 0.45) / 0.55).clamp(0.0, 1.0));
    let bottom = h + (band - h) * v;
    let x0 = island.x * u;
    let x1 = w + (island.x + island.w - w) * u;
    (t < 1.0).then(|| (Rect { x: x0, y: 0.0, w: x1 - x0, h: bottom }, radius.min(band / 2.0)))
}

/// Waking: the band widens out of the island, then the picture drops down
/// from it, a little past the bottom and back.
fn opening(w: f64, h: f64, radius: f64, island: Rect, t: f64) -> Option<(Rect, f64)> {
    let band = island.h.max(1.0);
    let u = min_jerk((t / 0.45).clamp(0.0, 1.0));
    let v = ease_out_back(((t - 0.35) / 0.65).clamp(0.0, 1.0));
    let x0 = island.x * (1.0 - u);
    let x1 = island.x + island.w + (w - island.x - island.w) * u;
    let bottom = band + (h - band) * v;
    Some((Rect { x: x0, y: 0.0, w: x1 - x0, h: bottom.min(h + 40.0) }, radius.min(band / 2.0).max(radius * v.min(1.0))))
}

/// How bright the collapsing edge glows: while the picture is a thin band.
fn afterglow(r: Rect, h: f64) -> f64 {
    let thin = 1.0 - (r.h / h).clamp(0.0, 1.0);
    0.45 * thin * thin
}

fn ease_out_back(x: f64) -> f64 {
    let c = 1.2;
    1.0 + (c + 1.0) * (x - 1.0).powi(3) + c * (x - 1.0).powi(2)
}

fn rounded(cr: &Context, r: Rect, radius: f64) {
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(r.x + r.w - radius, r.y + radius, radius, -FRAC_PI_2, 0.0);
    cr.arc(r.x + r.w - radius, r.y + r.h - radius, radius, 0.0, FRAC_PI_2);
    cr.arc(r.x + radius, r.y + r.h - radius, radius, FRAC_PI_2, PI);
    cr.arc(r.x + radius, r.y + radius, radius, PI, 1.5 * PI);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISLAND: Rect = Rect { x: 250.0, y: 0.0, w: 100.0, h: 32.0 };

    fn alpha_at(curtain: &mut Curtain, now: Time, x: usize, y: usize) -> u8 {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 400).unwrap();
        {
            let cr = Context::new(&surface).unwrap();
            curtain.draw(&cr, 600.0, 400.0, 16.0, ISLAND, now);
        }
        let stride = surface.stride() as usize;
        surface.data().unwrap()[y * stride + x * 4 + 3]
    }

    #[test]
    fn the_screen_closes_up_then_into_the_island_and_opens_again() {
        let mut c = Curtain::default();
        assert_eq!(alpha_at(&mut c, 0.0, 300, 300), 0, "no curtain at rest");
        c.close(1.0);
        // early: the bottom is already dark, the top still shows
        assert_eq!(alpha_at(&mut c, 1.0 + 0.25, 300, 390), 255);
        assert_eq!(alpha_at(&mut c, 1.0 + 0.25, 300, 20), 0);
        // a band under the top edge, still the screen's width; then narrowing
        assert_eq!(alpha_at(&mut c, 1.0 + 0.3, 300, 200), 255);
        assert_eq!(alpha_at(&mut c, 1.0 + 0.3, 20, 10), 0);
        assert_eq!(alpha_at(&mut c, 1.0 + 0.45, 20, 10), 255);
        assert_eq!(alpha_at(&mut c, 1.0 + 0.45, 300, 10), 0);
        assert!(!c.is_off(1.5) && c.is_off(1.0 + CLOSE));
        // off: black everywhere (the island is drawn over it)
        assert_eq!(alpha_at(&mut c, 2.0, 20, 10), 255);
        assert_eq!(alpha_at(&mut c, 2.0, 300, 16), 255);
        assert!(!c.is_animating(2.0));
        c.open(3.0);
        assert!(c.is_animating(3.1));
        // the band first, then the picture drops
        assert_eq!(alpha_at(&mut c, 3.3, 20, 10), 0);
        assert_eq!(alpha_at(&mut c, 3.3, 300, 300), 255);
        assert_eq!(alpha_at(&mut c, 3.0 + OPEN - 0.01, 300, 300), 0);
        // done: gone (one last frame wipes it)
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 400).unwrap();
        let cr = Context::new(&surface).unwrap();
        assert!(c.draw(&cr, 600.0, 400.0, 16.0, ISLAND, 3.0 + OPEN + 0.01).is_some());
        assert!(c.draw(&cr, 600.0, 400.0, 16.0, ISLAND, 3.0 + OPEN + 0.02).is_none());
        drop(cr);
        let _ = surface.data();
    }
}

#[cfg(test)]
mod gallery {
    use super::*;

    #[test]
    #[ignore = "writes target/gallery/curtain.png; set ISLAND_GALLERY=1"]
    fn curtain_frames() {
        if std::env::var_os("ISLAND_GALLERY").is_none() { return; }
        let (w, h, scale) = (1504.0, 1002.0, 0.18);
        let island = Rect { x: 660.0, y: 0.0, w: 184.0, h: 32.0 };
        let (fw, fh) = (w * scale, h * scale);
        let cols = 6;
        let times: Vec<(bool, f64)> = (0..cols).map(|i| (true, CLOSE * i as f64 / (cols - 1) as f64))
            .chain((0..cols).map(|i| (false, OPEN * i as f64 / (cols - 1) as f64))).collect();
        let out = cairo::ImageSurface::create(cairo::Format::Rgb24, ((fw + 4.0) * cols as f64) as i32, ((fh + 4.0) * 2.0) as i32).unwrap();
        let cr = Context::new(&out).unwrap();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().unwrap();
        for (i, (closing, t)) in times.into_iter().enumerate() {
            let mut c = Curtain::default();
            if closing { c.close(0.0) } else { c.state = Some(State::Opening(0.0)) }
            cr.save().unwrap();
            cr.translate((i % cols) as f64 * (fw + 4.0), (i / cols) as f64 * (fh + 4.0));
            cr.scale(scale, scale);
            cr.rectangle(0.0, 0.0, w, h);
            cr.clip();
            // a "picture": a gradient wallpaper
            let g = cairo::LinearGradient::new(0.0, 0.0, w, h);
            g.add_color_stop_rgb(0.0, 0.16, 0.42, 0.62);
            g.add_color_stop_rgb(1.0, 0.55, 0.30, 0.55);
            cr.set_source(&g).unwrap();
            cr.paint().unwrap();
            c.draw(&cr, w, h, 16.0, island, t.min(if closing { CLOSE - 0.001 } else { OPEN - 0.001 }));
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.rectangle(island.x, island.y, island.w, island.h);
            cr.fill().unwrap();
            cr.restore().unwrap();
        }
        drop(cr);
        std::fs::create_dir_all("target/gallery").unwrap();
        out.write_to_png(&mut std::fs::File::create("target/gallery/curtain.png").unwrap()).unwrap();
    }
}
