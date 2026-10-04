//! The island's Control Center: shortcut modules, display and sound
//! sliders with the audio output picker, the keyboard layout and the
//! session buttons — reached from the compact island.
use cairo::Context;

use crate::activity::SliderKind;
use crate::control::{Sink, SinkKind};
use crate::island::{Font, Renderer};
use crate::menu_style::{self, RAIL};

pub const WIDTH: f64 = 440.0;
const INNER: f64 = WIDTH - 2.0 * RAIL;
const GAP: f64 = 8.0;
const TILE_W: f64 = (INNER - GAP) / 2.0;
const TILE_H: f64 = 56.0;
const MODULE_GAP: f64 = 10.0;
const DISPLAY_Y: f64 = menu_style::CONTENT_TOP + 3.0 * TILE_H + 2.0 * GAP + MODULE_GAP;
const MODULE_H: f64 = 62.0;
const SOUND_Y: f64 = DISPLAY_Y + MODULE_H + MODULE_GAP;
const BOTTOM_Y: f64 = SOUND_Y + MODULE_H + MODULE_GAP;
const BOTTOM_H: f64 = 70.0;
const SMALL_W: f64 = (INNER - 3.0 * GAP) / 4.0;
pub const HEIGHT: f64 = BOTTOM_Y + BOTTOM_H + RAIL;
/// the sliders: inset in their module, below its title
const SLIDER_X: f64 = RAIL + 12.0;
const SLIDER_W: f64 = INNER - 24.0;
const SLIDER_DY: f64 = 32.0;
const SLIDER_H: f64 = 22.0;
/// the output picker's rows
const SINK_H: f64 = 48.0;

/// What the tiles' second lines report.
#[derive(Clone, Copy, Default)]
pub struct Status {
    pub unread: usize,
    /// Do Not Disturb, eased 0..1
    pub dnd: f64,
    /// a countdown's remaining seconds and whether it is running
    pub timer: Option<(f64, bool)>,
}

/// The environment as the menu last read it.
#[derive(Clone, Debug, Default)]
pub struct Controls {
    pub volume: Option<f64>,
    pub muted: bool,
    pub brightness: Option<f64>,
    pub sinks: Vec<Sink>,
    pub default_sink: String,
    pub layout: Option<String>,
}

impl Controls {
    pub fn current_sink(&self) -> Option<&Sink> {
        self.sinks.iter().find(|s| s.name == self.default_sink)
    }
}

/// Which page of the menu is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Main,
    Outputs,
}

/// Everything a frame of the menu needs besides geometry.
pub struct Frame<'a> {
    pub status: Status,
    pub controls: &'a Controls,
    /// eased slider positions
    pub volume: f64,
    pub brightness: f64,
    /// how lit each control is (hover / keyboard selection)
    pub hover: &'a dyn Fn(Action) -> f64,
    /// the logout button is waiting for its confirming second press
    pub logout_armed: bool,
    /// the frame's time and when the menu opened (rows arrive in a wave)
    pub now: f64,
    pub opened: f64,
    /// seconds since a control was pressed (it dips and springs back) and
    /// since its symbol last bounced
    pub pressed: &'a dyn Fn(Action) -> f64,
    pub bounced: &'a dyn Fn(Action) -> f64,
    /// how far each slider has swollen in the hand (brightness, volume)
    pub grow: (f64, f64),
}

impl Frame<'_> {
    /// A row of the main page arriving: its opacity and how far below its
    /// place it still is.
    fn arrival(&self, row: usize, alpha: f64) -> (f64, f64) {
        let k = crate::island::smoothstep((self.now - self.opened - 0.035 * row as f64) / 0.3);
        (alpha * k, 10.0 * (1.0 - k))
    }

    /// Keys for this opening's labels: a fresh menu just shows its text.
    fn key(&self, name: &str) -> String {
        format!("quick{:.4}:{name}", self.opened)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Wifi,
    Bluetooth,
    Timer,
    Launcher,
    Notifications,
    Dnd,
    Outputs,
    Layout,
    Lock,
    Sleep,
    Logout,
    Sink(usize),
    Back,
    Close,
}

/// What a press lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Action(Action),
    /// a slider, and where along it (0..1)
    Slider(SliderKind, f64),
}

/// Keyboard order of the main page.
pub const ACTIONS: [Action; 11] = [
    Action::Wifi, Action::Bluetooth, Action::Timer, Action::Launcher, Action::Notifications, Action::Dnd,
    Action::Outputs, Action::Layout, Action::Lock, Action::Sleep, Action::Logout,
];
const TILES: [Action; 6] = [Action::Wifi, Action::Bluetooth, Action::Timer, Action::Launcher, Action::Notifications, Action::Dnd];
const SMALL: [Action; 4] = [Action::Layout, Action::Lock, Action::Sleep, Action::Logout];

pub fn height(view: View, controls: &Controls) -> f64 {
    match view {
        View::Main => HEIGHT,
        View::Outputs => menu_style::CONTENT_TOP + controls.sinks.len().max(1) as f64 * SINK_H + RAIL,
    }
}

fn tile(i: usize) -> (f64, f64) {
    (RAIL + (i % 2) as f64 * (TILE_W + GAP), menu_style::CONTENT_TOP + (i / 2) as f64 * (TILE_H + GAP))
}

fn small(i: usize) -> f64 {
    RAIL + i as f64 * (SMALL_W + GAP)
}

/// The output button on the sound module's title line.
fn outputs_button() -> (f64, f64, f64, f64) {
    (RAIL + INNER - 12.0 - 170.0, SOUND_Y + 4.0, 170.0, 24.0)
}

fn inside(x: f64, y: f64, rect: (f64, f64, f64, f64)) -> bool {
    x >= rect.0 && x <= rect.0 + rect.2 && y >= rect.1 && y <= rect.1 + rect.3
}

/// Where along a slider `x` falls (0..1); also used while dragging.
pub fn slider_fraction(x: f64) -> f64 {
    ((x - SLIDER_X) / SLIDER_W).clamp(0.0, 1.0)
}

pub fn hit(view: View, controls: &Controls, x: f64, y: f64) -> Option<Hit> {
    let (cx, cy, size) = menu_style::close_rect(WIDTH);
    if inside(x, y, (cx - 6.0, cy - 6.0, size + 12.0, size + 12.0)) {
        return Some(Hit::Action(Action::Close));
    }
    if view == View::Outputs {
        if inside(x, y, (RAIL - 6.0, cy - 6.0, 110.0, size + 12.0)) {
            return Some(Hit::Action(Action::Back));
        }
        let row = ((y - menu_style::CONTENT_TOP) / SINK_H).floor();
        let within = (RAIL..RAIL + INNER).contains(&x) && row >= 0.0 && (row as usize) < controls.sinks.len();
        return within.then(|| Hit::Action(Action::Sink(row as usize)));
    }
    for (i, action) in TILES.iter().enumerate() {
        let (tx, ty) = tile(i);
        if inside(x, y, (tx, ty, TILE_W, TILE_H)) {
            return Some(Hit::Action(*action));
        }
    }
    if !controls.sinks.is_empty() && inside(x, y, outputs_button()) {
        return Some(Hit::Action(Action::Outputs));
    }
    for (kind, top, present) in [
        (SliderKind::Brightness, DISPLAY_Y, controls.brightness.is_some()),
        (SliderKind::Volume, SOUND_Y, controls.volume.is_some()),
    ] {
        // the whole lower part of the module grabs the slider
        if present && inside(x, y, (RAIL, top + SLIDER_DY - 8.0, INNER, SLIDER_H + 16.0)) {
            return Some(Hit::Slider(kind, slider_fraction(x)));
        }
    }
    for (i, action) in SMALL.iter().enumerate() {
        if inside(x, y, (small(i), BOTTOM_Y, SMALL_W, BOTTOM_H)) {
            return Some(Hit::Action(*action));
        }
    }
    None
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
        _ => {}
    }
    cr.restore().unwrap();
}

fn clock(seconds: f64) -> String {
    let s = seconds.ceil().max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn stroke_style(cr: &Context, width: f64, alpha: f64) {
    cr.new_path();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_width(width);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
}

/// The small glyphs: speaker, sun, outputs, keyboard and session buttons.
fn glyph(cr: &Context, name: &str, cx: f64, cy: f64, level: f64, rgba: (f64, f64, f64, f64)) {
    cr.save().unwrap();
    cr.translate(cx, cy);
    stroke_style(cr, 1.8, 1.0);
    cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
    let tau = std::f64::consts::TAU;
    match name {
        "sun" => {
            cr.arc(0.0, 0.0, 3.6, 0.0, tau);
            let _ = cr.fill();
            for i in 0..8 {
                let a = i as f64 * tau / 8.0;
                cr.move_to(6.2 * a.cos(), 6.2 * a.sin());
                cr.line_to(8.2 * a.cos(), 8.2 * a.sin());
            }
            let _ = cr.stroke();
        }
        "speaker" | "speaker-muted" => {
            cr.move_to(-8.0, -2.8);
            cr.line_to(-5.0, -2.8);
            cr.line_to(-0.8, -6.5);
            cr.line_to(-0.8, 6.5);
            cr.line_to(-5.0, 2.8);
            cr.line_to(-8.0, 2.8);
            cr.close_path();
            let _ = cr.fill();
            if name == "speaker-muted" {
                cr.move_to(3.0, -3.0);
                cr.line_to(8.0, 2.0);
                cr.move_to(8.0, -3.0);
                cr.line_to(3.0, 2.0);
                let _ = cr.stroke();
            } else {
                for (i, r) in [3.6, 6.8].into_iter().enumerate() {
                    if level > i as f64 * 0.5 + 0.01 {
                        cr.new_sub_path();
                        cr.arc(0.0, 0.0, r, -0.85, 0.85);
                        let _ = cr.stroke();
                    }
                }
            }
        }
        "headphones" => {
            cr.arc(0.0, 1.0, 7.0, std::f64::consts::PI, tau);
            let _ = cr.stroke();
            for dx in [-7.0, 7.0] {
                menu_style::rounded(cr, dx - 2.2, 0.0, 4.4, 7.5, 2.0);
                let _ = cr.fill();
            }
        }
        "display" => {
            menu_style::rounded(cr, -8.0, -6.0, 16.0, 10.5, 2.0);
            let _ = cr.stroke();
            cr.move_to(-3.5, 7.5);
            cr.line_to(3.5, 7.5);
            let _ = cr.stroke();
        }
        "bluetooth" => {
            cr.set_line_width(1.6);
            cr.move_to(-4.5, -4.5);
            cr.line_to(4.5, 4.0);
            cr.line_to(0.0, 8.0);
            cr.line_to(0.0, -8.0);
            cr.line_to(4.5, -4.0);
            cr.line_to(-4.5, 4.5);
            let _ = cr.stroke();
        }
        "keyboard" => {
            menu_style::rounded(cr, -9.0, -6.0, 18.0, 12.0, 2.5);
            let _ = cr.stroke();
            for row in 0..2 {
                for col in 0..4 {
                    cr.rectangle(-6.2 + col as f64 * 3.6, -3.3 + row as f64 * 3.3, 1.6, 1.6);
                }
            }
            let _ = cr.fill();
            cr.move_to(-3.5, 3.4);
            cr.line_to(3.5, 3.4);
            let _ = cr.stroke();
        }
        "lock" => {
            menu_style::rounded(cr, -6.0, -1.5, 12.0, 9.5, 2.2);
            let _ = cr.fill();
            cr.new_path();
            cr.arc(0.0, -2.0, 3.8, std::f64::consts::PI, 0.0);
            let _ = cr.stroke();
        }
        "sleep" => {
            // moon.zzz: an outlined crescent and a small z
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            cr.arc(-2.0, 1.5, 6.5, 0.0, tau);
            cr.new_sub_path();
            cr.arc(1.8, -2.2, 5.6, 0.0, tau);
            let _ = cr.fill();
            cr.set_fill_rule(cairo::FillRule::Winding);
            cr.set_line_width(1.4);
            cr.move_to(4.6, -9.0);
            cr.line_to(8.4, -9.0);
            cr.line_to(4.6, -5.4);
            cr.line_to(8.4, -5.4);
            let _ = cr.stroke();
        }
        "logout" => {
            // a door frame and an arrow leaving it
            cr.move_to(-1.0, -7.5);
            cr.line_to(-7.0, -7.5);
            cr.line_to(-7.0, 7.5);
            cr.line_to(-1.0, 7.5);
            let _ = cr.stroke();
            cr.move_to(-2.5, 0.0);
            cr.line_to(8.0, 0.0);
            cr.move_to(4.5, -3.5);
            cr.line_to(8.0, 0.0);
            cr.line_to(4.5, 3.5);
            let _ = cr.stroke();
        }
        _ => {}
    }
    cr.restore().unwrap();
    cr.new_path();
}

fn sink_glyph(kind: SinkKind) -> &'static str {
    match kind {
        SinkKind::Speaker | SinkKind::Other => "speaker",
        SinkKind::Headphones => "headphones",
        SinkKind::Display => "display",
        SinkKind::Bluetooth => "headphones",
    }
}

/// Control Center's slider: a thick capsule whose white fill is the level,
/// with the symbol riding inside at the left.
#[allow(clippy::too_many_arguments)]
fn slider(cr: &Context, x: f64, y: f64, w: f64, h: f64, value: f64, symbol: &str, level: f64, alpha: f64, grow: f64) {
    // held, it swells a little about its centre line
    let swell = 6.0 * grow.clamp(0.0, 1.2);
    let (x, y, w, h) = (x - swell * 0.5, y - swell * 0.5, w + swell, h + swell);
    menu_style::rounded(cr, x, y, w, h, h / 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.14 * alpha);
    let _ = cr.fill();
    // the fill never shrinks below a circle, so the symbol stays seated
    let fill = h + (w - h) * value.clamp(0.0, 1.0);
    menu_style::rounded(cr, x, y, fill, h, h / 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.94 * alpha);
    let _ = cr.fill();
    glyph(cr, symbol, x + h / 2.0 + 1.0, y + h / 2.0, level, (0.0, 0.0, 0.0, 0.62 * alpha));
}

fn module(cr: &Context, renderer: &Renderer, title: &str, x: f64, y: f64, alpha: f64) {
    if alpha <= 0.002 {
        return;
    }
    menu_style::group(cr, x + RAIL, y, INNER, MODULE_H, 0.0, alpha);
    menu_style::label(cr, renderer, title, x + RAIL + 12.0, y + 8.0, 160.0, Font::Label, alpha);
}

/// The menu at `x`, `y`; `view_k` fades a page in or out during a switch.
pub fn draw(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, view: View, close_hover: f64, frame: &Frame) {
    if alpha <= 0.002 {
        return;
    }
    let (cx, cy, size) = menu_style::close_rect(WIDTH);
    match view {
        View::Main => {
            menu_style::header(cr, renderer, x, y, WIDTH, None, "Пункт управления", None, 60.0, alpha);
            draw_main(cr, renderer, x, y, alpha, frame);
        }
        View::Outputs => draw_outputs(cr, renderer, x, y, alpha, frame),
    }
    menu_style::close(cr, x + cx, y + cy, size, close_hover, alpha);
}

fn draw_main(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, frame: &Frame) {
    let status = frame.status;
    let controls = frame.controls;
    let dnd = status.dnd.clamp(0.0, 1.0);
    let base_alpha = alpha;
    for (i, action) in TILES.iter().enumerate() {
        let (alpha, rise) = frame.arrival(i / 2, base_alpha);
        if alpha <= 0.002 {
            continue;
        }
        let (tx, ty) = tile(i);
        let (tx, ty) = (x + tx, y + ty + rise);
        let h = (frame.hover)(*action).clamp(0.0, 1.0);
        let press = menu_style::press_scale((frame.pressed)(*action));
        cr.save().unwrap();
        cr.translate(tx + TILE_W / 2.0, ty + TILE_H / 2.0);
        cr.scale(press, press);
        cr.translate(-(tx + TILE_W / 2.0), -(ty + TILE_H / 2.0));
        menu_style::group(cr, tx, ty, TILE_W, TILE_H, h, alpha);
        let accent = match action {
            Action::Wifi | Action::Bluetooth => Some(menu_style::BLUE),
            Action::Timer => Some(menu_style::ORANGE),
            Action::Notifications => Some(menu_style::RED),
            _ => None,
        };
        let (wx, wy) = (tx + 12.0 + 16.0, ty + TILE_H / 2.0);
        menu_style::well(cr, wx, wy, 16.0, accent, alpha);
        if *action == Action::Dnd && dnd > 0.0 {
            menu_style::well(cr, wx, wy, 16.0, Some(menu_style::PURPLE), alpha * dnd);
        }
        let bounce = menu_style::bounce_scale((frame.bounced)(*action));
        icon(cr, *action, wx, wy, alpha, (0.7 + 0.03 * h) * bounce);
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
            _ => unreachable!(),
        };
        two_lines(cr, renderer, title, &subtitle, tx + 56.0, ty, TILE_W - 64.0, TILE_H, alpha,
            &frame.key(&format!("{action:?}")));
        cr.restore().unwrap();
    }

    // Display
    let (alpha, rise) = frame.arrival(3, base_alpha);
    module(cr, renderer, "Дисплей", x, y + DISPLAY_Y + rise, alpha);
    let present = controls.brightness.is_some();
    slider(cr, x + SLIDER_X, y + DISPLAY_Y + SLIDER_DY + rise, SLIDER_W, SLIDER_H, frame.brightness, "sun", 1.0,
        alpha * if present { 1.0 } else { 0.35 }, frame.grow.0);

    // Sound, with the output it plays through
    let (alpha, rise) = frame.arrival(4, base_alpha);
    let y = y + rise;
    module(cr, renderer, "Звук", x, y + SOUND_Y, alpha);
    if let Some(sink) = controls.current_sink() {
        let (bx, by, bw, bh) = outputs_button();
        let lit = (frame.hover)(Action::Outputs).clamp(0.0, 1.0);
        let label_w = renderer.layout(cr, &sink.label, Font::Subhead).pixel_extents().1.width() as f64;
        let text_w = label_w.min(bw - 30.0);
        let right = x + bx + bw;
        menu_style::chevron(cr, right - 5.0, y + by + bh / 2.0, 4.0, 0.0, alpha * (0.55 + 0.45 * lit));
        crate::motion::text(cr, renderer, &frame.key("sink"), &sink.label, right - 16.0 - (bw - 30.0),
            y + by + (bh - menu_style::line_height(cr, renderer, Font::Subhead)) / 2.0, bw - 30.0, 1.0, Font::Subhead,
            (1.0, 1.0, 1.0), alpha * (menu_style::SECONDARY + (1.0 - menu_style::SECONDARY) * lit));
        glyph(cr, sink_glyph(sink.kind), right - 30.0 - text_w, y + by + bh / 2.0, 1.0,
            (1.0, 1.0, 1.0, alpha * (menu_style::SECONDARY + 0.4 * lit)));
    }
    let present = controls.volume.is_some();
    let symbol = if controls.muted { "speaker-muted" } else { "speaker" };
    slider(cr, x + SLIDER_X, y + SOUND_Y + SLIDER_DY, SLIDER_W, SLIDER_H,
        if controls.muted { 0.0 } else { frame.volume }, symbol, frame.volume, alpha * if present { 1.0 } else { 0.35 },
        frame.grow.1);
    let y = y - rise;

    // the keyboard layout and the session
    let (alpha, rise) = frame.arrival(5, base_alpha);
    for (i, action) in SMALL.iter().enumerate() {
        let sx = x + small(i);
        let sy = y + BOTTOM_Y + rise;
        let h = (frame.hover)(*action).clamp(0.0, 1.0);
        let armed = *action == Action::Logout && frame.logout_armed;
        let press = menu_style::press_scale((frame.pressed)(*action));
        cr.save().unwrap();
        cr.translate(sx + SMALL_W / 2.0, sy + BOTTOM_H / 2.0);
        cr.scale(press, press);
        cr.translate(-(sx + SMALL_W / 2.0), -(sy + BOTTOM_H / 2.0));
        menu_style::group(cr, sx, sy, SMALL_W, BOTTOM_H, h, alpha);
        let (name, label) = match action {
            Action::Layout => ("keyboard", controls.layout.as_deref().map(crate::control::layout_label)
                .unwrap_or_else(|| "Раскладка".into())),
            Action::Lock => ("lock", "Блокировка".into()),
            Action::Sleep => ("sleep", "Сон".into()),
            _ => ("logout", if armed { "Точно?".into() } else { "Выйти".into() }),
        };
        let wy = sy + 25.0;
        let since = (frame.bounced)(*action);
        let (scale, angle) = match action {
            // asking to confirm, the symbol shakes its head
            Action::Logout => (1.0, menu_style::wiggle_angle(since)),
            _ => (menu_style::bounce_scale(since), 0.0),
        };
        menu_style::transformed(cr, sx + SMALL_W / 2.0, wy, scale, angle, || {
            menu_style::well(cr, sx + SMALL_W / 2.0, wy, 16.0, armed.then_some(menu_style::RED), alpha);
            glyph(cr, name, sx + SMALL_W / 2.0, wy, 1.0, (1.0, 1.0, 1.0, alpha));
        });
        let colour = if armed { menu_style::RED } else { (1.0, 1.0, 1.0) };
        let lh = menu_style::line_height(cr, renderer, Font::Footnote);
        crate::motion::text(cr, renderer, &frame.key(&format!("small{i}")), &label, sx + 4.0, sy + 44.0 + (18.0 - lh) / 2.0,
            SMALL_W - 8.0, 0.5, Font::Footnote, colour, alpha * if armed { 1.0 } else { 0.85 });
        cr.restore().unwrap();
    }
}

#[allow(clippy::too_many_arguments)]
fn two_lines(cr: &Context, renderer: &Renderer, title: &str, subtitle: &str, x: f64, y: f64, w: f64, h: f64, alpha: f64, key: &str) {
    let title_h = menu_style::line_height(cr, renderer, Font::Headline);
    let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
    let top = y + (h - title_h - sub_h) / 2.0;
    menu_style::label(cr, renderer, title, x, top, w, Font::Headline, alpha);
    crate::motion::text(cr, renderer, key, subtitle, x, top + title_h, w, 0.0, Font::Subhead, (1.0, 1.0, 1.0),
        menu_style::SECONDARY * alpha);
}

fn draw_outputs(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, frame: &Frame) {
    let back = (frame.hover)(Action::Back);
    menu_style::chevron(cr, x + RAIL + 5.0, y + menu_style::HEADER_MID, 7.0, std::f64::consts::PI, alpha * (0.8 + 0.2 * back));
    menu_style::text(cr, renderer, "Назад", x + RAIL + 16.0,
        y + menu_style::HEADER_MID - menu_style::line_height(cr, renderer, Font::Body) / 2.0, 80.0, Font::Body,
        menu_style::BLUE, alpha * (0.85 + 0.15 * back));
    menu_style::centred(cr, renderer, "Вывод звука", (x + 110.0, y + menu_style::HEADER_MID - 12.0, WIDTH - 220.0, 24.0),
        Font::Title, (1.0, 1.0, 1.0), alpha);
    let sinks = &frame.controls.sinks;
    let top = y + menu_style::CONTENT_TOP;
    menu_style::group(cr, x + RAIL, top, INNER, sinks.len().max(1) as f64 * SINK_H, 0.0, alpha);
    if sinks.is_empty() {
        menu_style::centred(cr, renderer, "Нет устройств вывода", (x + RAIL, top, INNER, SINK_H), Font::Body,
            (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
    }
    let body_h = menu_style::line_height(cr, renderer, Font::Body);
    for (i, sink) in sinks.iter().enumerate() {
        let ry = top + i as f64 * SINK_H;
        if i > 0 {
            menu_style::separator(cr, renderer, x + RAIL + 52.0, x + RAIL + INNER, ry, alpha);
        }
        menu_style::highlight(cr, x + RAIL, ry, INNER, SINK_H, (frame.hover)(Action::Sink(i)), alpha);
        let current = sink.name == frame.controls.default_sink;
        let (wx, wy) = (x + RAIL + 12.0 + 14.0, ry + SINK_H / 2.0);
        menu_style::well(cr, wx, wy, 14.0, current.then_some(menu_style::BLUE), alpha);
        glyph(cr, sink_glyph(sink.kind), wx, wy, 1.0, (1.0, 1.0, 1.0, alpha));
        menu_style::label(cr, renderer, &sink.label, x + RAIL + 52.0, ry + (SINK_H - body_h) / 2.0, INNER - 52.0 - 44.0,
            Font::Body, alpha);
        if current {
            let (cx, cy) = (x + RAIL + INNER - 22.0, ry + SINK_H / 2.0);
            cr.new_path();
            cr.move_to(cx - 5.5, cy + 0.5);
            cr.line_to(cx - 1.5, cy + 4.5);
            cr.line_to(cx + 6.0, cy - 4.5);
            cr.set_line_width(2.2);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_join(cairo::LineJoin::Round);
            cr.set_source_rgba(menu_style::BLUE.0, menu_style::BLUE.1, menu_style::BLUE.2, alpha);
            let _ = cr.stroke();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls() -> Controls {
        Controls {
            volume: Some(0.4),
            brightness: Some(0.6),
            sinks: vec![
                Sink { name: "a".into(), label: "Динамики".into(), kind: SinkKind::Speaker },
                Sink { name: "b".into(), label: "AirPods".into(), kind: SinkKind::Bluetooth },
            ],
            default_sink: "b".into(),
            ..Controls::default()
        }
    }

    #[test]
    fn every_control_is_hit_inside_its_target_only() {
        let c = controls();
        for (i, action) in TILES.iter().enumerate() {
            let (x, y) = tile(i);
            assert_eq!(hit(View::Main, &c, x + 20.0, y + 20.0), Some(Hit::Action(*action)));
        }
        for (i, action) in SMALL.iter().enumerate() {
            assert_eq!(hit(View::Main, &c, small(i) + 30.0, BOTTOM_Y + 30.0), Some(Hit::Action(*action)));
        }
        let (bx, by, bw, _) = outputs_button();
        assert_eq!(hit(View::Main, &c, bx + bw - 10.0, by + 10.0), Some(Hit::Action(Action::Outputs)));
        assert_eq!(hit(View::Main, &c, SLIDER_X, SOUND_Y + SLIDER_DY + 10.0), Some(Hit::Slider(SliderKind::Volume, 0.0)));
        assert_eq!(hit(View::Main, &c, SLIDER_X + SLIDER_W, DISPLAY_Y + SLIDER_DY + 10.0),
            Some(Hit::Slider(SliderKind::Brightness, 1.0)));
        assert_eq!(hit(View::Main, &c, WIDTH - 37.0, 40.0), Some(Hit::Action(Action::Close)));
        assert_eq!(hit(View::Main, &c, WIDTH / 2.0, HEIGHT - 8.0), None);
        assert_eq!(hit(View::Outputs, &c, 200.0, menu_style::CONTENT_TOP + SINK_H + 10.0), Some(Hit::Action(Action::Sink(1))));
        assert_eq!(hit(View::Outputs, &c, 40.0, 40.0), Some(Hit::Action(Action::Back)));
        let without = Controls { volume: None, ..controls() };
        assert_eq!(hit(View::Main, &without, SLIDER_X + 50.0, SOUND_Y + SLIDER_DY + 10.0), None);
    }
}
