//! Native shortcuts reached from the compact island.
use cairo::Context;

use crate::island::{Font, Renderer};

pub const WIDTH: f64 = 440.0;
pub const HEIGHT: f64 = 324.0;
const PAD: f64 = 18.0;
const GAP: f64 = 10.0;
const TILE_W: f64 = (WIDTH - PAD * 2.0 - GAP) / 2.0;
const TILE_H: f64 = 70.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Wifi,
    Bluetooth,
    Timer,
    Launcher,
    Notifications,
    Dnd,
    Close,
}

pub const ACTIONS: [Action; 6] = [Action::Wifi, Action::Bluetooth, Action::Timer, Action::Launcher, Action::Notifications, Action::Dnd];

fn tile(i: usize) -> (f64, f64) {
    (PAD + (i % 2) as f64 * (TILE_W + GAP), 64.0 + (i / 2) as f64 * (TILE_H + GAP))
}

pub fn hit(x: f64, y: f64) -> Option<Action> {
    if (WIDTH - 49.0..WIDTH - 12.0).contains(&x) && (12.0..47.0).contains(&y) {
        return Some(Action::Close);
    }
    ACTIONS.iter().enumerate().find_map(|(i, action)| {
        let (tx, ty) = tile(i);
        (x >= tx && x <= tx + TILE_W && y >= ty && y <= ty + TILE_H).then_some(*action)
    })
}

fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

fn icon(cr: &Context, action: Action, x: f64, y: f64, alpha: f64, scale: f64) {
    cr.save().unwrap();
    cr.translate(x, y);
    cr.scale(scale, scale);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_width(2.1);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    match action {
        Action::Wifi => {
            for (r, a, b) in [(12.0, 3.83, 5.59), (8.0, 3.88, 5.54), (4.0, 4.02, 5.40)] {
                cr.new_sub_path();
                cr.arc(0.0, 6.0, r, a, b);
                let _ = cr.stroke();
            }
            cr.arc(0.0, 6.0, 1.4, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        Action::Bluetooth => {
            cr.move_to(0.0, -13.0);
            cr.line_to(8.0, -6.0);
            cr.line_to(-7.0, 7.0);
            cr.move_to(-7.0, -7.0);
            cr.line_to(8.0, 6.0);
            cr.line_to(0.0, 13.0);
            cr.line_to(0.0, -13.0);
            let _ = cr.stroke();
        }
        Action::Timer => {
            cr.arc(0.0, 2.0, 10.0, 0.0, std::f64::consts::TAU);
            cr.move_to(0.0, 2.0);
            cr.line_to(0.0, -4.0);
            cr.move_to(0.0, 2.0);
            cr.line_to(5.0, 5.0);
            cr.move_to(-3.0, -12.0);
            cr.line_to(3.0, -12.0);
            let _ = cr.stroke();
        }
        Action::Launcher => {
            cr.arc(-2.0, -2.0, 8.0, 0.0, std::f64::consts::TAU);
            cr.move_to(4.0, 4.0);
            cr.line_to(11.0, 11.0);
            let _ = cr.stroke();
        }
        Action::Notifications => {
            cr.move_to(-8.0, 8.0);
            cr.curve_to(-6.0, 5.0, -6.0, -1.0, -6.0, -4.0);
            cr.arc(0.0, -4.0, 6.0, std::f64::consts::PI, 0.0);
            cr.curve_to(6.0, -1.0, 6.0, 5.0, 8.0, 8.0);
            cr.close_path();
            let _ = cr.stroke();
            cr.arc(0.0, 11.0, 1.5, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        Action::Dnd => {
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            cr.arc(0.0, 0.0, 11.0, 0.0, std::f64::consts::TAU);
            cr.new_sub_path();
            cr.arc(5.0, -4.0, 10.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        Action::Close => {}
    }
    cr.restore().unwrap();
}

pub fn draw(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, hover: [f64; 6], close_hover: f64, unread: usize, dnd_level: f64) {
    let title = renderer.layout(cr, "Быстрый доступ", Font::Text);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.97 * alpha);
    cr.move_to(renderer.snap(x + 22.0), renderer.snap(y + 18.0));
    pangocairo::functions::show_layout(cr, &title);
    rounded(cr, x + WIDTH - 48.0, y + 14.0, 30.0, 30.0, 15.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, (0.10 + close_hover * 0.12) * alpha);
    let _ = cr.fill();
    cr.set_source_rgba(1.0, 1.0, 1.0, (0.58 + close_hover * 0.42) * alpha);
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.move_to(x + WIDTH - 39.0, y + 23.0);
    cr.line_to(x + WIDTH - 27.0, y + 35.0);
    cr.move_to(x + WIDTH - 27.0, y + 23.0);
    cr.line_to(x + WIDTH - 39.0, y + 35.0);
    let _ = cr.stroke();

    for (i, action) in ACTIONS.iter().enumerate() {
        let (tx, ty) = tile(i);
        let (tx, ty) = (x + tx, y + ty);
        let h = hover[i].clamp(0.0, 1.0);
        let on = if *action == Action::Dnd { dnd_level.clamp(0.0, 1.0) } else { 0.0 };
        rounded(cr, tx, ty, TILE_W, TILE_H, 17.0);
        cr.set_source_rgba(0.13 + 0.15 * on, 0.13 + 0.07 * on, 0.14 + 0.28 * on,
            (0.98 + 0.02 * h) * alpha);
        let _ = cr.fill();
        rounded(cr, tx + 0.5, ty + 0.5, TILE_W - 1.0, TILE_H - 1.0, 16.5);
        cr.set_line_width(1.0);
        cr.set_source_rgba(0.72 + 0.18 * on, 0.70 + 0.06 * on, 0.80 + 0.18 * on,
            (0.08 + 0.11 * h + 0.25 * on) * alpha);
        let _ = cr.stroke();
        let accent = match action {
            Action::Wifi => (0.11, 0.57, 0.97),
            Action::Bluetooth => (0.13, 0.48, 0.91),
            Action::Timer => (0.95, 0.57, 0.12),
            Action::Launcher => (0.43, 0.43, 0.49),
            Action::Notifications => (0.38, 0.71, 0.94),
            Action::Dnd => (0.36 + 0.19 * on, 0.36 + 0.07 * on, 0.40 + 0.51 * on),
            Action::Close => unreachable!(),
        };
        rounded(cr, tx + 14.0, ty + 15.0, 40.0, 40.0, 12.0);
        cr.set_source_rgba(accent.0, accent.1, accent.2, alpha);
        let _ = cr.fill();
        icon(cr, *action, tx + 34.0, ty + 35.0, alpha, 1.0 + 0.045 * h);
        let label = match action {
            Action::Wifi => "Wi-Fi",
            Action::Bluetooth => "Bluetooth",
            Action::Timer => "Таймер",
            Action::Launcher => "Лаунчер",
            Action::Notifications => "Уведомления",
            Action::Dnd => "Тишина",
            Action::Close => unreachable!(),
        };
        let layout = renderer.layout(cr, label, Font::Caption);
        cr.set_source_rgba(1.0, 1.0, 1.0, (0.87 + 0.13 * h) * alpha);
        cr.move_to(renderer.snap(tx + 66.0), renderer.snap(ty + 25.0));
        pangocairo::functions::show_layout(cr, &layout);
        if *action == Action::Notifications && unread > 0 {
            let count = renderer.layout(cr, &unread.min(99).to_string(), Font::Small);
            cr.set_source_rgba(0.46, 0.82, 1.0, alpha);
            cr.move_to(renderer.snap(tx + TILE_W - 22.0), renderer.snap(ty + 5.0));
            pangocairo::functions::show_layout(cr, &count);
        }
    }
    let hint = renderer.layout(cr, "Esc — закрыть", Font::Small);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.4 * alpha);
    cr.move_to(renderer.snap(x + 23.0), renderer.snap(y + HEIGHT - 23.0));
    pangocairo::functions::show_layout(cr, &hint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_and_close_are_hit_only_inside_their_targets() {
        for (i, action) in ACTIONS.iter().enumerate() {
            let (x, y) = tile(i);
            assert_eq!(hit(x + 20.0, y + 20.0), Some(*action));
        }
        assert_eq!(hit(WIDTH - 33.0, 29.0), Some(Action::Close));
        assert_eq!(hit(WIDTH / 2.0, HEIGHT - 8.0), None);
    }
}
