//! Now playing: the compact bars beside the clock, the "now playing" toast
//! when a track starts, and the player the island opens into when the
//! pointer rests on it — album art, title, a live equaliser, a progress
//! line you can scrub, and vector transport controls.

use std::f64::consts::PI;

use cairo::Context;

use crate::anim::Time;
use crate::art::Art;
use crate::island::{Font, Renderer, smoothstep};

pub const PLAYER_W: f64 = 430.0;
pub const PLAYER_H: f64 = 148.0;
const PAD: f64 = 20.0;
const ART: f64 = 80.0;
const ART_R: f64 = 16.0;
const TEXT_X: f64 = PAD + ART + 16.0;
const PROGRESS_Y: f64 = 82.0;
const PROGRESS_H: f64 = 3.0;
const CONTROLS_Y: f64 = 116.0;
const CONTROL_GAP: f64 = 52.0;

/// The toast that announces a track.
pub const TOAST_H: f64 = 92.0;
pub const TOAST_MIN_W: f64 = 380.0;
pub const TOAST_MAX_TEXT: f64 = 340.0;
const TOAST_ART: f64 = 48.0;
const TOAST_PAD: f64 = 22.0;

/// The equaliser's colour, as in the island's own bars.
pub const EQ_COLOR: (f64, f64, f64) = (0x66 as f64 / 255.0, 0xD9 as f64 / 255.0, 1.0);

/// What the pointer can hit in the open player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Previous,
    PlayPause,
    Next,
    /// a click on the progress line, as a fraction of the track
    Seek(f64),
}

/// Where the controls are, relative to the player's top-left corner.
fn control_centres(w: f64) -> [(Control, f64, f64); 3] {
    let cx = w / 2.0;
    [
        (Control::Previous, cx - CONTROL_GAP, CONTROLS_Y),
        (Control::PlayPause, cx, CONTROLS_Y),
        (Control::Next, cx + CONTROL_GAP, CONTROLS_Y),
    ]
}

/// Hit test in the player's own coordinates.
pub fn hit(w: f64, x: f64, y: f64) -> Option<Control> {
    for (control, cx, cy) in control_centres(w) {
        if (x - cx).hypot(y - cy) <= 22.0 {
            return Some(control);
        }
    }
    let track_w = w - TEXT_X - PAD;
    if x >= TEXT_X - 6.0 && x <= TEXT_X + track_w + 6.0 && (y - PROGRESS_Y).abs() <= 12.0 {
        return Some(Control::Seek(((x - TEXT_X) / track_w).clamp(0.0, 1.0)));
    }
    None
}

/// Everything the drawing needs about the track.
pub struct View<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub app: &'a str,
    pub art_url: &'a str,
    pub playing: bool,
    /// 0..1 per band, already eased
    pub eq: &'a [f64],
    pub position: f64,
    pub length: f64,
    /// a control the pointer is holding down, for the little squish
    pub pressed: Option<(Control, Time)>,
    pub hovered: Option<Control>,
}

/// Draw the open player inside the box at (`x`, `y`, `w`, `h`).
#[allow(clippy::too_many_arguments)]
pub fn draw(cr: &Context, renderer: &Renderer, art: &Art, view: &View, x: f64, y: f64, w: f64, alpha: f64, now: Time, opened: Time) {
    if alpha <= 0.003 {
        return;
    }
    let t = now - opened;
    // the pieces settle in one after another as the island opens
    let stage = |delay: f64| {
        let k = smoothstep(((t - delay) / 0.28).clamp(0.0, 1.0));
        (alpha * k, 8.0 * (1.0 - k))
    };

    let (aa, ao) = stage(0.0);
    draw_art(cr, renderer, art, view, x + PAD, y + PAD + ao, ART, ART_R, aa);

    // title and artist
    let (ta, to) = stage(0.05);
    let bars_w = 30.0;
    let text_w = w - TEXT_X - PAD - bars_w - 12.0;
    let title = renderer.layout_ellipsized(cr, view.title, Font::Text, text_w, 1);
    let artist = renderer.layout_ellipsized(cr, if view.artist.is_empty() { view.app } else { view.artist }, Font::Small, text_w, 1);
    let (_, t_ext) = title.pixel_extents();
    let (_, a_ext) = artist.pixel_extents();
    cr.set_source_rgba(1.0, 1.0, 1.0, ta);
    cr.move_to(renderer.snap(x + TEXT_X + to), renderer.snap(y + 24.0 - t_ext.y() as f64));
    pangocairo::functions::show_layout(cr, &title);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.55 * ta);
    cr.move_to(renderer.snap(x + TEXT_X + to), renderer.snap(y + 24.0 + t_ext.height() as f64 + 2.0 - a_ext.y() as f64));
    pangocairo::functions::show_layout(cr, &artist);

    // the live equaliser, right of the text
    let (ba, _) = stage(0.1);
    bars(cr, renderer, view.eq, x + w - PAD - bars_w, y + 30.0, bars_w, 22.0, ba);

    // the progress line
    let (pa, po) = stage(0.12);
    let track_x = x + TEXT_X;
    let track_w = w - TEXT_X - PAD;
    let done = if view.length > 0.0 { (view.position / view.length).clamp(0.0, 1.0) } else { 0.0 };
    rounded(cr, track_x, y + PROGRESS_Y + po, track_w, PROGRESS_H, PROGRESS_H / 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.18 * pa);
    let _ = cr.fill();
    if done > 0.0 {
        rounded(cr, track_x, y + PROGRESS_Y + po, track_w * done, PROGRESS_H, PROGRESS_H / 2.0);
        cr.set_source_rgba(EQ_COLOR.0, EQ_COLOR.1, EQ_COLOR.2, 0.95 * pa);
        let _ = cr.fill();
    }
    if view.length > 0.0 {
        let elapsed = renderer.layout(cr, &clock(view.position), Font::Tiny);
        let left = renderer.layout(cr, &format!("-{}", clock((view.length - view.position).max(0.0))), Font::Tiny);
        let (_, e_ext) = elapsed.pixel_extents();
        let (_, l_ext) = left.pixel_extents();
        let ty = renderer.snap(y + PROGRESS_Y + 10.0 + po);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.45 * pa);
        cr.move_to(renderer.snap(track_x), ty - e_ext.y() as f64);
        pangocairo::functions::show_layout(cr, &elapsed);
        cr.move_to(renderer.snap(track_x + track_w - l_ext.width() as f64), ty - l_ext.y() as f64);
        pangocairo::functions::show_layout(cr, &left);
    }

    // transport controls
    let (ca, co) = stage(0.16);
    for (control, cx, cy) in control_centres(w) {
        let squish = match view.pressed {
            Some((c, since)) if c == control => 1.0 - 0.16 * (-(now - since) / 0.12).exp(),
            _ => 1.0,
        };
        // the control under the pointer sits on a soft disc
        if view.hovered == Some(control) {
            cr.new_path();
            cr.arc(x + cx, y + cy + co, 21.0, 0.0, 2.0 * PI);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.10 * ca);
            let _ = cr.fill();
        }
        let lit = if view.hovered == Some(control) { 1.0 } else { 0.85 };
        control_glyph(cr, control, view.playing, x + cx, y + cy + co, squish, ca * lit);
    }
}

/// The album art, or the app's initial when the player publishes none.
fn draw_art(cr: &Context, renderer: &Renderer, art: &Art, view: &View, x: f64, y: f64, size: f64, radius: f64, alpha: f64) {
    if alpha <= 0.003 {
        return;
    }
    let device = (size * renderer.scale()).round().max(1.0) as i32;
    cr.save().unwrap();
    rounded(cr, x, y, size, size, radius);
    cr.clip();
    match art.get(view.art_url, device) {
        Some(surface) => {
            cr.save().unwrap();
            cr.translate(x, y);
            cr.scale(1.0 / renderer.scale(), 1.0 / renderer.scale());
            let _ = cr.set_source_surface(&surface, 0.0, 0.0);
            let _ = cr.paint_with_alpha(alpha);
            cr.restore().unwrap();
        }
        None => {
            // no art: a soft plate with a music note
            let grad = cairo::LinearGradient::new(x, y, x, y + size);
            grad.add_color_stop_rgba(0.0, 0.22, 0.24, 0.28, alpha);
            grad.add_color_stop_rgba(1.0, 0.13, 0.14, 0.17, alpha);
            let _ = cr.set_source(&grad);
            let _ = cr.paint();
            note(cr, x + size / 2.0, y + size / 2.0, size * 0.42, alpha * 0.8);
        }
    }
    cr.restore().unwrap();
    // a hairline so a dark cover does not melt into the island
    rounded(cr, x, y, size, size, radius);
    cr.set_line_width(1.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.10 * alpha);
    let _ = cr.stroke();
}

/// A music note, for art we do not have.
fn note(cr: &Context, cx: f64, cy: f64, size: f64, alpha: f64) {
    let s = size / 24.0;
    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.scale(s, s);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.set_line_width(2.6);
    cr.new_path();
    cr.move_to(-4.0, 8.0);
    cr.line_to(-4.0, -9.0);
    cr.line_to(10.0, -12.0);
    cr.line_to(10.0, 5.0);
    let _ = cr.stroke();
    for (x, y) in [(-7.0, 8.0), (7.0, 5.0)] {
        cr.new_path();
        cr.save().unwrap();
        cr.translate(x, y);
        cr.scale(1.25, 1.0);
        cr.arc(0.0, 0.0, 3.4, 0.0, 2.0 * PI);
        cr.restore().unwrap();
        let _ = cr.fill();
    }
    cr.restore().unwrap();
}

/// Four bars, bottoms on a common line — the same shape the collapsed
/// island shows beside the clock.
pub fn bars(cr: &Context, renderer: &Renderer, levels: &[f64], x: f64, y: f64, w: f64, h: f64, alpha: f64) {
    if alpha <= 0.003 {
        return;
    }
    let n = levels.len().max(1);
    let bar = 3.0;
    let step = (w - bar) / (n - 1).max(1) as f64;
    cr.set_source_rgba(EQ_COLOR.0, EQ_COLOR.1, EQ_COLOR.2, alpha);
    for (i, level) in levels.iter().enumerate() {
        let bh = (2.0 + (h - 2.0) * level.clamp(0.0, 1.0)).min(h);
        let bx = renderer.snap(x + i as f64 * step);
        rounded(cr, bx, y + h - bh, bar, bh, bar / 2.0);
        let _ = cr.fill();
    }
}

/// Previous / play-pause / next, drawn rather than typed so they can be
/// nudged when pressed.
fn control_glyph(cr: &Context, control: Control, playing: bool, cx: f64, cy: f64, scale: f64, alpha: f64) {
    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.scale(scale, scale);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    match control {
        Control::PlayPause if playing => {
            for dx in [-5.0, 2.5] {
                rounded(cr, dx, -10.0, 4.6, 20.0, 1.8);
                let _ = cr.fill();
            }
        }
        Control::PlayPause => {
            cr.new_path();
            cr.move_to(-6.0, -11.0);
            cr.line_to(11.0, 0.0);
            cr.line_to(-6.0, 11.0);
            cr.close_path();
            let _ = cr.fill();
        }
        Control::Previous | Control::Next => {
            let dir = if control == Control::Next { 1.0 } else { -1.0 };
            for offset in [-8.5, 0.5] {
                cr.new_path();
                cr.move_to(dir * (offset + 8.0), 0.0);
                cr.line_to(dir * offset, -8.0);
                cr.line_to(dir * offset, 8.0);
                cr.close_path();
                let _ = cr.fill();
            }
            rounded(cr, dir * 9.0 - 1.6, -8.0, 3.2, 16.0, 1.4);
            let _ = cr.fill();
        }
        Control::Seek(_) => {}
    }
    cr.restore().unwrap();
}

/// The toast that announces a track: art, the app's name, the title and
/// the artist, with the bars beating on the right.
#[allow(clippy::too_many_arguments)]
pub fn draw_toast(cr: &Context, renderer: &Renderer, art: &Art, view: &View, x: f64, y: f64, w: f64, h: f64, alpha: f64, since: Time, now: Time) {
    if alpha <= 0.003 {
        return;
    }
    let t = now - since;
    let stage = |delay: f64| {
        let k = smoothstep(((t - delay) / 0.3).clamp(0.0, 1.0));
        (alpha * k, 10.0 * (1.0 - k))
    };
    let (aa, ao) = stage(0.0);
    let art_y = y + (h - TOAST_ART) / 2.0;
    draw_art(cr, renderer, art, view, x + TOAST_PAD, art_y + ao, TOAST_ART, 11.0, aa);

    let bars_w = 26.0;
    let text_x = x + TOAST_PAD + TOAST_ART + 16.0;
    let text_w = (x + w - TOAST_PAD - bars_w - 14.0) - text_x;
    let (ta, to) = stage(0.06);
    let label = renderer.layout_ellipsized(cr, &format!("{} · Now Playing", view.app), Font::Caption, text_w, 1);
    let title = renderer.layout_ellipsized(
        cr,
        &if view.artist.is_empty() { view.title.to_string() } else { format!("{} — {}", view.title, view.artist) },
        Font::Text,
        text_w,
        1,
    );
    let (_, l_ext) = label.pixel_extents();
    let (_, t_ext) = title.pixel_extents();
    let total = l_ext.height() as f64 + 2.0 + t_ext.height() as f64;
    let top = y + (h - total) / 2.0;
    cr.set_source_rgba(EQ_COLOR.0, EQ_COLOR.1, EQ_COLOR.2, ta);
    cr.move_to(renderer.snap(text_x + to), renderer.snap(top - l_ext.y() as f64));
    pangocairo::functions::show_layout(cr, &label);
    cr.set_source_rgba(1.0, 1.0, 1.0, ta);
    cr.move_to(renderer.snap(text_x + to), renderer.snap(top + l_ext.height() as f64 + 2.0 - t_ext.y() as f64));
    pangocairo::functions::show_layout(cr, &title);

    let (ba, _) = stage(0.12);
    bars(cr, renderer, view.eq, x + w - TOAST_PAD - bars_w, y + h / 2.0 - 9.0, bars_w, 18.0, ba);
}

/// The width a toast needs for this track.
pub fn toast_width(cr: &Context, renderer: &Renderer, view: &View) -> f64 {
    let label = renderer.layout_ellipsized(cr, &format!("{} · Now Playing", view.app), Font::Caption, TOAST_MAX_TEXT, 1);
    let title = renderer.layout_ellipsized(
        cr,
        &if view.artist.is_empty() { view.title.to_string() } else { format!("{} — {}", view.title, view.artist) },
        Font::Text,
        TOAST_MAX_TEXT,
        1,
    );
    let text = (label.pixel_extents().1.width() as f64).max(title.pixel_extents().1.width() as f64);
    (TOAST_PAD + TOAST_ART + 16.0 + text + 14.0 + 26.0 + TOAST_PAD).max(TOAST_MIN_W)
}

/// m:ss
fn clock(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_path();
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.arc(x + w - r, y + r, r, 1.5 * PI, 2.0 * PI);
    cr.arc(x + w - r, y + h - r, r, 0.0, 0.5 * PI);
    cr.arc(x + r, y + h - r, r, 0.5 * PI, PI);
    cr.close_path();
}
