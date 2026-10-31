//! App icons for notifications, drawn as vectors so they can move: each is
//! a rounded "app tile" in the app's colour with a white glyph on it, and
//! each has its own little entrance — the paper plane flies in, the gear
//! turns into place, the bell rings, the envelope's flap falls shut, the
//! alert shakes its head.

use std::f64::consts::PI;

use cairo::Context;

use crate::anim::min_jerk;
use crate::notify::AppKind;

/// Tile size in icon units; everything here is drawn in a 44×44 box
/// centred on the origin and scaled to the size the pill wants.
const UNIT: f64 = 44.0;
const TILE: f64 = 40.0;
// menu_style::tight(11.0); this file is also built by examples/icons
const TILE_R: f64 = 11.0 * 0.5;

fn rgb(hex: u32) -> (f64, f64, f64) {
    (((hex >> 16) & 0xff) as f64 / 255.0, ((hex >> 8) & 0xff) as f64 / 255.0, (hex & 0xff) as f64 / 255.0)
}

/// The app's accent colour (the tile's, and the countdown ring's).
pub fn accent(kind: AppKind) -> (f64, f64, f64) {
    match kind {
        AppKind::Telegram => rgb(0x2AABEE),
        AppKind::System => rgb(0x8A94A6),
        AppKind::Firefox => rgb(0xFF7139),
        AppKind::Discord => rgb(0x5865F2),
        AppKind::Mail => rgb(0x3B82F6),
        AppKind::Bell => rgb(0x66D9FF),
        AppKind::Critical => rgb(0xFF3B30),
        AppKind::Editor => rgb(0x57A143),
    }
}

fn tile_gradient(kind: AppKind) -> ((f64, f64, f64), (f64, f64, f64)) {
    let top = accent(kind);
    let bottom = match kind {
        AppKind::Telegram => rgb(0x1E96D6),
        AppKind::System => rgb(0x6B7385),
        AppKind::Firefox => rgb(0xE8443A),
        AppKind::Discord => rgb(0x4650D8),
        AppKind::Mail => rgb(0x1D5FD8),
        AppKind::Bell => rgb(0x3FA9D6),
        AppKind::Critical => rgb(0xD92B20),
        AppKind::Editor => rgb(0x3E7A2E),
    };
    (top, bottom)
}

/// A rounded square with continuous curvature (the same smoothing the
/// island's own corners use, done here with Béziers directly).
fn tile_path(cr: &Context, half: f64, r: f64) {
    let k = 0.46 * r; // squircle-ish handle length
    cr.new_path();
    cr.move_to(-half + r, -half);
    cr.line_to(half - r, -half);
    cr.curve_to(half - r + k, -half, half, -half + r - k, half, -half + r);
    cr.line_to(half, half - r);
    cr.curve_to(half, half - r + k, half - r + k, half, half - r, half);
    cr.line_to(-half + r, half);
    cr.curve_to(-half + r - k, half, -half, half - r + k, -half, half - r);
    cr.line_to(-half, -half + r);
    cr.curve_to(-half, -half + r - k, -half + r - k, -half, -half + r, -half);
    cr.close_path();
}

/// Draw the icon centred on (`cx`, `cy`), `size` px tall, `t` seconds
/// since it appeared (its entrance), `alpha` its opacity.
pub fn draw(cr: &Context, kind: AppKind, cx: f64, cy: f64, size: f64, t: f64, alpha: f64) {
    let scale = size / UNIT;
    // every icon pops in with the same spring, then plays its own gesture
    let s = (t.max(0.0) / 0.42).clamp(0.0, 1.0);
    let over = 1.0 + 0.16 * (-t.max(0.0) / 0.14).exp() * (2.0 * PI * t.max(0.0) / 0.32).sin();
    let pop = (min_jerk(s) * over).max(0.0);
    if pop * scale < 1e-3 {
        return; // a zero scale is not an invertible matrix
    }
    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.scale(scale * pop, scale * pop);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    let (top, bottom) = tile_gradient(kind);
    let grad = cairo::LinearGradient::new(0.0, -TILE / 2.0, 0.0, TILE / 2.0);
    grad.add_color_stop_rgba(0.0, top.0, top.1, top.2, alpha);
    grad.add_color_stop_rgba(1.0, bottom.0, bottom.1, bottom.2, alpha);
    tile_path(cr, TILE / 2.0, TILE_R);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();

    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    match kind {
        AppKind::Telegram => plane(cr, t, alpha),
        AppKind::System => gear(cr, t, top, alpha),
        AppKind::Firefox => flame(cr, t, alpha),
        AppKind::Discord => clyde(cr, t, top, alpha),
        AppKind::Mail => envelope(cr, t, bottom, alpha),
        AppKind::Bell => bell(cr, t),
        AppKind::Critical => alert(cr, t),
        AppKind::Editor => neovim(cr, t),
    }
    cr.restore().unwrap();
}

/// Telegram: the paper plane flies in from the lower left and settles.
fn plane(cr: &Context, t: f64, alpha: f64) {
    let k = 1.0 - min_jerk((t / 0.55).clamp(0.0, 1.0));
    cr.save().unwrap();
    cr.translate(-13.0 * k, 9.0 * k);
    cr.rotate(-0.5 * k);
    cr.new_path();
    cr.move_to(-11.0, -0.6);
    cr.line_to(11.5, -8.5);
    cr.line_to(6.0, 9.5);
    cr.line_to(0.6, 4.0);
    cr.line_to(-2.4, 8.2);
    cr.line_to(-2.9, 2.4);
    cr.close_path();
    let _ = cr.fill();
    // the fold: the darker underside of the wing
    cr.new_path();
    cr.move_to(-2.9, 2.4);
    cr.line_to(11.5, -8.5);
    cr.line_to(0.6, 4.0);
    cr.close_path();
    cr.set_source_rgba(0.80, 0.92, 1.0, alpha);
    let _ = cr.fill();
    cr.restore().unwrap();
}

/// System: the gear turns a sixth of a turn into place.
fn gear(cr: &Context, t: f64, hub: (f64, f64, f64), alpha: f64) {
    let turn = (1.0 - min_jerk((t / 0.6).clamp(0.0, 1.0))) * PI / 3.0;
    cr.save().unwrap();
    cr.rotate(turn);
    let teeth = 7;
    let (r_out, r_in) = (12.2, 8.4);
    let p = 2.0 * PI / teeth as f64;
    cr.new_path();
    for i in 0..teeth {
        let a = i as f64 * p;
        cr.arc(0.0, 0.0, r_out, a, a + 0.40 * p);
        cr.arc(0.0, 0.0, r_in, a + 0.56 * p, a + p);
    }
    cr.close_path();
    let _ = cr.fill();
    // the hub, in the tile's own colour
    cr.new_path();
    cr.arc(0.0, 0.0, 4.0, 0.0, 2.0 * PI);
    cr.set_source_rgba(hub.0, hub.1, hub.2, alpha);
    let _ = cr.fill();
    cr.restore().unwrap();
}

/// Firefox: a flame that unwinds as it appears.
fn flame(cr: &Context, t: f64, alpha: f64) {
    let k = 1.0 - min_jerk((t / 0.6).clamp(0.0, 1.0));
    cr.save().unwrap();
    cr.rotate(-0.55 * k);
    cr.new_path();
    cr.move_to(1.5, -13.0);
    cr.curve_to(6.0, -8.5, 8.6, -5.0, 8.2, -0.5);
    cr.curve_to(7.8, 5.0, 4.5, 11.5, -1.0, 11.8);
    cr.curve_to(-6.5, 12.1, -10.0, 8.0, -9.4, 2.8);
    cr.curve_to(-9.0, -0.6, -6.6, -1.8, -5.6, -3.4);
    cr.curve_to(-4.6, -5.0, -5.0, -6.6, -6.0, -7.8);
    cr.curve_to(-2.6, -7.4, -0.6, -9.4, 1.5, -13.0);
    cr.close_path();
    let _ = cr.fill();
    // the hot core
    cr.new_path();
    cr.move_to(1.2, -3.6);
    cr.curve_to(4.6, -0.4, 4.6, 3.4, 2.2, 6.2);
    cr.curve_to(-0.6, 9.4, -5.2, 7.4, -4.8, 3.0);
    cr.curve_to(-4.5, 0.2, -1.4, -0.6, 1.2, -3.6);
    cr.close_path();
    let hot = rgb(0xFFD166);
    cr.set_source_rgba(hot.0, hot.1, hot.2, alpha);
    let _ = cr.fill();
    cr.restore().unwrap();
}

/// Discord: the face bounces in with a squash.
fn clyde(cr: &Context, t: f64, eyes: (f64, f64, f64), alpha: f64) {
    let squash = 1.0 + 0.18 * (-t / 0.16).exp() * (2.0 * PI * t / 0.3).cos();
    cr.save().unwrap();
    cr.scale(squash, 2.0 - squash);
    cr.translate(0.0, -0.5);
    cr.new_path();
    cr.move_to(-12.5, 5.0);
    cr.curve_to(-12.8, -4.0, -10.0, -9.5, -6.0, -10.6);
    cr.curve_to(-2.0, -11.7, 2.0, -11.7, 6.0, -10.6);
    cr.curve_to(10.0, -9.5, 12.8, -4.0, 12.5, 5.0);
    // the two feet with the notch between them
    cr.curve_to(10.5, 8.6, 8.6, 10.4, 6.6, 10.4);
    cr.curve_to(5.0, 10.4, 4.2, 9.0, 3.6, 7.6);
    cr.curve_to(1.2, 8.2, -1.2, 8.2, -3.6, 7.6);
    cr.curve_to(-4.2, 9.0, -5.0, 10.4, -6.6, 10.4);
    cr.curve_to(-8.6, 10.4, -10.5, 8.6, -12.5, 5.0);
    cr.close_path();
    let _ = cr.fill();
    cr.set_source_rgba(eyes.0, eyes.1, eyes.2, alpha);
    for x in [-4.6, 4.6] {
        cr.save().unwrap();
        cr.translate(x, -1.0);
        cr.scale(1.0, 1.35);
        cr.new_path();
        cr.arc(0.0, 0.0, 2.5, 0.0, 2.0 * PI);
        let _ = cr.fill();
        cr.restore().unwrap();
    }
    cr.restore().unwrap();
}

/// Mail: the flap falls shut behind the letter.
fn envelope(cr: &Context, t: f64, flap: (f64, f64, f64), alpha: f64) {
    let open = 1.0 - min_jerk((t / 0.5).clamp(0.0, 1.0));
    let (w, h, r) = (12.5, 9.0, 2.4);
    cr.new_path();
    cr.move_to(-w + r, -h);
    cr.line_to(w - r, -h);
    cr.curve_to(w, -h, w, -h, w, -h + r);
    cr.line_to(w, h - r);
    cr.curve_to(w, h, w, h, w - r, h);
    cr.line_to(-w + r, h);
    cr.curve_to(-w, h, -w, h, -w, h - r);
    cr.line_to(-w, -h + r);
    cr.curve_to(-w, -h, -w, -h, -w + r, -h);
    cr.close_path();
    let _ = cr.fill();
    // the flap: a V that lifts while it opens, in the tile's colour
    cr.save().unwrap();
    cr.set_source_rgba(flap.0, flap.1, flap.2, alpha);
    cr.set_line_width(2.1);
    cr.new_path();
    cr.move_to(-w + 1.8, -h + 1.6);
    cr.line_to(0.0, 2.4 - 9.5 * open);
    cr.line_to(w - 1.8, -h + 1.6);
    let _ = cr.stroke();
    cr.restore().unwrap();
}

/// A bell that rings: it swings, damped, as it arrives.
fn bell(cr: &Context, t: f64) {
    let swing = 0.42 * (-t / 0.35).exp() * (2.0 * PI * t / 0.26).sin();
    cr.save().unwrap();
    cr.translate(0.0, -11.0);
    cr.rotate(swing);
    cr.translate(0.0, 11.0);
    cr.new_path();
    cr.move_to(-9.5, 5.0);
    cr.curve_to(-7.5, 3.0, -7.2, 0.5, -7.2, -2.0);
    cr.curve_to(-7.2, -7.0, -4.0, -10.0, 0.0, -10.0);
    cr.curve_to(4.0, -10.0, 7.2, -7.0, 7.2, -2.0);
    cr.curve_to(7.2, 0.5, 7.5, 3.0, 9.5, 5.0);
    cr.close_path();
    let _ = cr.fill();
    cr.new_path();
    cr.arc(0.0, 8.0, 2.6, 0.0, 2.0 * PI);
    let _ = cr.fill();
    cr.restore().unwrap();
}

/// Critical: an exclamation mark that shakes its head.
fn alert(cr: &Context, t: f64) {
    let shake = 2.4 * (-t / 0.2).exp() * (2.0 * PI * t / 0.12).sin();
    cr.save().unwrap();
    cr.translate(shake, 0.0);
    cr.new_path();
    cr.move_to(-2.1, -11.0);
    cr.line_to(2.1, -11.0);
    cr.line_to(1.5, 3.0);
    cr.line_to(-1.5, 3.0);
    cr.close_path();
    let _ = cr.fill();
    cr.new_path();
    cr.arc(0.0, 8.2, 2.5, 0.0, 2.0 * PI);
    let _ = cr.fill();
    cr.restore().unwrap();
}

/// Neovim: the two strokes of its mark, drawn in as they arrive.
fn neovim(cr: &Context, t: f64) {
    let k = min_jerk((t / 0.5).clamp(0.0, 1.0));
    cr.save().unwrap();
    // the left bar and the diagonal, as in the Neovim mark
    cr.new_path();
    cr.move_to(-11.0, -11.0);
    cr.line_to(-4.6, -11.0);
    cr.line_to(-4.6, 11.0 * (2.0 * k - 1.0).clamp(-1.0, 1.0).max(-1.0));
    cr.line_to(-11.0, 11.0);
    cr.close_path();
    let _ = cr.fill();
    if k > 0.35 {
        let g = ((k - 0.35) / 0.65).clamp(0.0, 1.0);
        cr.new_path();
        cr.move_to(-4.6, -11.0);
        cr.line_to(-4.6 + 15.6 * g, -11.0 + 22.0 * g);
        cr.line_to(11.0 * g.max(0.35), 11.0);
        cr.line_to(11.0 * g.max(0.35), -11.0 + 4.0);
        cr.close_path();
        let _ = cr.fill();
    }
    // the right bar
    if k > 0.6 {
        cr.new_path();
        cr.move_to(4.6, -11.0 + 6.0);
        cr.line_to(11.0, -11.0);
        cr.line_to(11.0, 11.0);
        cr.line_to(4.6, 11.0);
        cr.close_path();
        let _ = cr.fill();
    }
    cr.restore().unwrap();
}

/// The countdown ring around the icon: it empties clockwise from the top
/// over the notification's life.
pub fn countdown(cr: &Context, kind: AppKind, cx: f64, cy: f64, radius: f64, left: f64, alpha: f64) {
    if left <= 0.0 || alpha <= 0.0 {
        return;
    }
    let c = accent(kind);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_width(2.0);
    cr.new_path();
    cr.arc(cx, cy, radius, 0.0, 2.0 * PI);
    cr.set_source_rgba(c.0, c.1, c.2, 0.18 * alpha);
    let _ = cr.stroke();
    cr.new_path();
    cr.arc(cx, cy, radius, -PI / 2.0, -PI / 2.0 + 2.0 * PI * left.clamp(0.0, 1.0));
    cr.set_source_rgba(c.0, c.1, c.2, 0.9 * alpha);
    let _ = cr.stroke();
}

/// A ring that fills as a job gets on with it (LSP indexing, a build), the
/// same size as the countdown so one can take over from the other.
pub fn progress_ring(cr: &Context, kind: AppKind, cx: f64, cy: f64, radius: f64, done: f64, alpha: f64) {
    let c = accent(kind);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_width(2.0);
    cr.new_path();
    cr.arc(cx, cy, radius, 0.0, 2.0 * PI);
    cr.set_source_rgba(c.0, c.1, c.2, 0.18 * alpha);
    let _ = cr.stroke();
    let done = done.clamp(0.0, 1.0);
    if done > 0.0 {
        cr.new_path();
        cr.arc(cx, cy, radius, -PI / 2.0, -PI / 2.0 + 2.0 * PI * done);
        cr.set_source_rgba(c.0, c.1, c.2, 0.95 * alpha);
        let _ = cr.stroke();
    }
}
