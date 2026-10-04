//! Native shortcuts reached from the compact island.
use cairo::Context;

use crate::island::{Font, Renderer};
use crate::menu_style;

pub const WIDTH: f64 = 440.0;
const PAD: f64 = menu_style::RAIL;
const GAP: f64 = 10.0;
const TILE_W: f64 = (WIDTH - PAD * 2.0 - GAP) / 2.0;
const TILE_H: f64 = 64.0;
pub const HEIGHT: f64 = menu_style::CONTENT_TOP + 3.0 * TILE_H + 2.0 * GAP + PAD;

/// What the tiles' second lines report.
#[derive(Clone, Copy, Default)]
pub struct Status {
    pub unread: usize,
    /// Do Not Disturb, eased 0..1
    pub dnd: f64,
    /// a countdown's remaining seconds and whether it is running
    pub timer: Option<(f64, bool)>,
}

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
    (PAD + (i % 2) as f64 * (TILE_W + GAP), menu_style::CONTENT_TOP + (i / 2) as f64 * (TILE_H + GAP))
}

pub fn hit(x: f64, y: f64) -> Option<Action> {
    let (cx, cy, size) = menu_style::close_rect(WIDTH);
    if (cx - 6.0..cx + size + 6.0).contains(&x) && (cy - 6.0..cy + size + 6.0).contains(&y) {
        return Some(Action::Close);
    }
    ACTIONS.iter().enumerate().find_map(|(i, action)| {
        let (tx, ty) = tile(i);
        (x >= tx && x <= tx + TILE_W && y >= ty && y <= ty + TILE_H).then_some(*action)
    })
}

fn icon(cr: &Context, action: Action, x: f64, y: f64, alpha: f64, scale: f64) {
    cr.save().unwrap();
    cr.translate(x, y);
    cr.scale(scale, scale);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_width(2.2);
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

fn clock(seconds: f64) -> String {
    let s = seconds.ceil().max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// The menu: a header and six Control Center-style modules — a round
/// icon well, a title and a status line.
#[allow(clippy::too_many_arguments)]
pub fn draw(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, hover: [f64; 6], close_hover: f64, status: Status) {
    menu_style::header(cr, renderer, x, y, WIDTH, None, "Быстрый доступ", None, 60.0, alpha);
    let (cx, cy, size) = menu_style::close_rect(WIDTH);
    menu_style::close(cr, x + cx, y + cy, size, close_hover, alpha);

    let dnd = status.dnd.clamp(0.0, 1.0);
    for (i, action) in ACTIONS.iter().enumerate() {
        let (tx, ty) = tile(i);
        let (tx, ty) = (x + tx, y + ty);
        let h = hover[i].clamp(0.0, 1.0);
        menu_style::group(cr, tx, ty, TILE_W, TILE_H, h, alpha);
        let accent = match action {
            Action::Wifi | Action::Bluetooth => Some(menu_style::BLUE),
            Action::Timer => Some(menu_style::ORANGE),
            Action::Notifications => Some(menu_style::RED),
            Action::Launcher => None,
            Action::Dnd => None,
            Action::Close => unreachable!(),
        };
        let (wx, wy) = (tx + 14.0 + 18.0, ty + TILE_H / 2.0);
        menu_style::well(cr, wx, wy, 18.0, accent, alpha);
        if *action == Action::Dnd && dnd > 0.0 {
            menu_style::well(cr, wx, wy, 18.0, Some(menu_style::PURPLE), alpha * dnd);
        }
        icon(cr, *action, wx, wy, alpha, 0.78 + 0.03 * h);
        let (title, subtitle) = match action {
            Action::Wifi => ("Wi-Fi", "Сети".to_string()),
            Action::Bluetooth => ("Bluetooth", "Устройства".to_string()),
            Action::Timer => ("Таймер", match status.timer {
                Some((left, true)) => clock(left),
                Some((left, false)) => format!("Пауза · {}", clock(left)),
                None => "Выкл.".to_string(),
            }),
            Action::Launcher => ("Лаунчер", "Приложения".to_string()),
            Action::Notifications => ("Уведомления", match status.unread {
                0 => "Нет новых".to_string(),
                n => format!("{} новых", n.min(99)),
            }),
            Action::Dnd => ("Тишина", if dnd > 0.5 { "Вкл.".to_string() } else { "Выкл.".to_string() }),
            Action::Close => unreachable!(),
        };
        let text_x = tx + 64.0;
        let text_w = TILE_W - 64.0 - 10.0;
        let title_h = menu_style::line_height(cr, renderer, Font::Headline);
        let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
        let top = ty + (TILE_H - title_h - sub_h) / 2.0;
        menu_style::label(cr, renderer, title, text_x, top, text_w, Font::Headline, alpha);
        menu_style::label(cr, renderer, &subtitle, text_x, top + title_h, text_w, Font::Subhead, menu_style::SECONDARY * alpha);
    }
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
