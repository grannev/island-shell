//! The island's Control Center: shortcut modules, display and sound
//! sliders with the audio output picker, the keyboard layout and the
//! session buttons — reached from the compact island.
use cairo::Context;

use crate::control::{App, Sink, SinkKind};
use crate::island::{Font, Renderer};
use crate::menu_style::{self, RAIL};

pub const WIDTH: f64 = 440.0;
const INNER: f64 = WIDTH - 2.0 * RAIL;
const GAP: f64 = 8.0;
const TILE_W: f64 = (INNER - GAP) / 2.0;
const TILE_H: f64 = 56.0;
const MODULE_GAP: f64 = 10.0;
/// "Now playing" sits under the tiles while something plays; the modules
/// below it move down by its height (see `shift`)
const MEDIA_Y: f64 = menu_style::CONTENT_TOP + 3.0 * TILE_H + 2.0 * GAP + MODULE_GAP;
const MEDIA_H: f64 = 72.0;
const MEDIA_ART: f64 = 48.0;
const MEDIA_BUTTON: f64 = 38.0;
const DISPLAY_Y: f64 = MEDIA_Y;
const MODULE_H: f64 = 62.0;
const SOUND_Y: f64 = DISPLAY_Y + MODULE_H + MODULE_GAP;
const BOTTOM_Y: f64 = SOUND_Y + MODULE_H + MODULE_GAP;
const BOTTOM_H: f64 = 70.0;
const SMALL_W: f64 = (INNER - 4.0 * GAP) / 5.0;
/// the main page without "Now playing"
pub const HEIGHT: f64 = BOTTOM_Y + BOTTOM_H + RAIL;
/// the sliders: inset in their module, below its title
const SLIDER_X: f64 = RAIL + 12.0;
const SLIDER_W: f64 = INNER - 24.0;
const SLIDER_DY: f64 = 32.0;
const SLIDER_H: f64 = 22.0;
/// the sound page: section captions, rows and their compact sliders
const CAPTION_H: f64 = 24.0;
const SECTION_GAP: f64 = 14.0;
const SINK_H: f64 = 44.0;
const APP_H: f64 = 54.0;
const MIC_H: f64 = 56.0;
const MAX_APPS: usize = 4;
const ROW_SLIDER_X: f64 = RAIL + 52.0;
const ROW_SLIDER_W: f64 = INNER - 52.0 - 14.0;
const ROW_SLIDER_DY: f64 = 32.0;
const ROW_SLIDER_H: f64 = 14.0;

/// What the tiles' second lines report.
#[derive(Clone, Copy, Default)]
pub struct Status {
    pub unread: usize,
    /// Do Not Disturb, eased 0..1
    pub dnd: f64,
    /// a countdown's remaining seconds and whether it is running
    pub timer: Option<(f64, bool)>,
    /// screen recording: None without wf-recorder, else whether one runs
    pub recording: Option<bool>,
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
    pub apps: Vec<App>,
    pub mic: Option<f64>,
    pub mic_muted: bool,
    pub mic_label: Option<String>,
    /// what is playing (kept by the island from MPRIS)
    pub media: Option<Media>,
}

/// The track "Now playing" shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    pub title: String,
    pub artist: String,
    pub app: String,
    pub art_url: String,
    pub playing: bool,
}

/// How far the modules under the tiles move down for "Now playing".
fn shift(controls: &Controls) -> f64 {
    if controls.media.is_some() { MEDIA_H + MODULE_GAP } else { 0.0 }
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
    /// outputs, applications and the microphone
    Sound,
}

/// The sliders the menu can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slider {
    Brightness,
    Volume,
    /// an application on the sound page (index into `Controls::apps`)
    App(usize),
    Mic,
}

/// Where the sound page's parts sit (y of each, relative to the menu).
struct SoundPage {
    sinks: f64,
    apps: f64,
    app_rows: usize,
    mic: f64,
    height: f64,
}

fn sound_page(controls: &Controls) -> SoundPage {
    let top = menu_style::CONTENT_TOP;
    let sinks = top + CAPTION_H;
    let apps_caption = sinks + controls.sinks.len().max(1) as f64 * SINK_H + SECTION_GAP;
    let apps = apps_caption + CAPTION_H;
    let app_rows = controls.apps.len().min(MAX_APPS);
    let apps_h = if app_rows == 0 { SINK_H } else { app_rows as f64 * APP_H };
    let mic = apps + apps_h + SECTION_GAP + CAPTION_H;
    SoundPage { sinks, apps, app_rows, mic, height: mic + MIC_H + RAIL }
}

/// Everything a frame of the menu needs besides geometry.
pub struct Frame<'a> {
    pub status: Status,
    pub controls: &'a Controls,
    /// eased slider positions, and how far each has swollen in the hand
    pub level: &'a dyn Fn(Slider) -> f64,
    pub grow: &'a dyn Fn(Slider) -> f64,
    /// an icon-theme icon at a size in device pixels (loaded off-thread)
    pub icon: &'a dyn Fn(&str, i32) -> Option<cairo::ImageSurface>,
    /// album art (loaded off-thread)
    pub art: &'a crate::art::Art,
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
    MicMute,
    Layout,
    /// start / stop a screen recording
    Record,
    Lock,
    Sleep,
    Logout,
    Sink(usize),
    /// "Now playing": previous, play / pause, next
    MediaPrevious,
    MediaPlayPause,
    MediaNext,
    Back,
    Close,
}

/// What a press lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Action(Action),
    /// a slider, and where along it (0..1)
    Slider(Slider, f64),
}

/// Keyboard order of the main page.
pub const ACTIONS: [Action; 12] = [
    Action::Wifi, Action::Bluetooth, Action::Timer, Action::Launcher, Action::Notifications, Action::Dnd,
    Action::Outputs, Action::Layout, Action::Record, Action::Lock, Action::Sleep, Action::Logout,
];
const TILES: [Action; 6] = [Action::Wifi, Action::Bluetooth, Action::Timer, Action::Launcher, Action::Notifications, Action::Dnd];
const SMALL: [Action; 5] = [Action::Layout, Action::Record, Action::Lock, Action::Sleep, Action::Logout];

pub fn height(view: View, controls: &Controls) -> f64 {
    match view {
        View::Main => HEIGHT + shift(controls),
        View::Sound => sound_page(controls).height,
    }
}

fn tile(i: usize) -> (f64, f64) {
    (RAIL + (i % 2) as f64 * (TILE_W + GAP), menu_style::CONTENT_TOP + (i / 2) as f64 * (TILE_H + GAP))
}

fn small(i: usize) -> f64 {
    RAIL + i as f64 * (SMALL_W + GAP)
}

/// The output button on the sound module's title line.
fn outputs_button(controls: &Controls) -> (f64, f64, f64, f64) {
    (RAIL + INNER - 12.0 - 170.0, SOUND_Y + shift(controls) + 4.0, 170.0, 24.0)
}

/// The transport buttons' centres in "Now playing".
const MEDIA_ACTIONS: [Action; 3] = [Action::MediaPrevious, Action::MediaPlayPause, Action::MediaNext];

fn media_button(i: usize) -> (f64, f64) {
    (RAIL + INNER - 10.0 - MEDIA_BUTTON / 2.0 - (2 - i) as f64 * MEDIA_BUTTON, MEDIA_Y + MEDIA_H / 2.0)
}

fn inside(x: f64, y: f64, rect: (f64, f64, f64, f64)) -> bool {
    x >= rect.0 && x <= rect.0 + rect.2 && y >= rect.1 && y <= rect.1 + rect.3
}

/// Where along `slider` the menu-relative `x` falls (0..1); also used
/// while dragging.
pub fn slider_fraction(slider: Slider, x: f64) -> f64 {
    let (sx, sw) = match slider {
        Slider::Brightness | Slider::Volume => (SLIDER_X, SLIDER_W),
        Slider::App(_) | Slider::Mic => (ROW_SLIDER_X, ROW_SLIDER_W),
    };
    ((x - sx) / sw).clamp(0.0, 1.0)
}

pub fn hit(view: View, controls: &Controls, x: f64, y: f64) -> Option<Hit> {
    let (cx, cy, size) = menu_style::close_rect(WIDTH);
    if inside(x, y, (cx - 6.0, cy - 6.0, size + 12.0, size + 12.0)) {
        return Some(Hit::Action(Action::Close));
    }
    if view == View::Sound {
        if inside(x, y, (RAIL - 6.0, cy - 6.0, 110.0, size + 12.0)) {
            return Some(Hit::Action(Action::Back));
        }
        if !(RAIL..RAIL + INNER).contains(&x) {
            return None;
        }
        let page = sound_page(controls);
        let row = ((y - page.sinks) / SINK_H).floor();
        if row >= 0.0 && (row as usize) < controls.sinks.len() {
            return Some(Hit::Action(Action::Sink(row as usize)));
        }
        let row = ((y - page.apps) / APP_H).floor();
        // a row's slider takes the press anywhere right of its icon
        if row >= 0.0 && (row as usize) < page.app_rows && x >= ROW_SLIDER_X - 8.0 {
            let app = Slider::App(row as usize);
            return Some(Hit::Slider(app, slider_fraction(app, x)));
        }
        if controls.mic.is_some() && (page.mic..page.mic + MIC_H).contains(&y) {
            return Some(if x < ROW_SLIDER_X - 8.0 {
                Hit::Action(Action::MicMute)
            } else {
                Hit::Slider(Slider::Mic, slider_fraction(Slider::Mic, x))
            });
        }
        return None;
    }
    for (i, action) in TILES.iter().enumerate() {
        let (tx, ty) = tile(i);
        if inside(x, y, (tx, ty, TILE_W, TILE_H)) {
            return Some(Hit::Action(*action));
        }
    }
    if controls.media.is_some() {
        for (i, action) in MEDIA_ACTIONS.iter().enumerate() {
            let (cx, cy) = media_button(i);
            if (x - cx).abs() <= MEDIA_BUTTON / 2.0 && (y - cy).abs() <= 22.0 {
                return Some(Hit::Action(*action));
            }
        }
    }
    if !controls.sinks.is_empty() && inside(x, y, outputs_button(controls)) {
        return Some(Hit::Action(Action::Outputs));
    }
    let dy = shift(controls);
    for (slider, top, present) in [
        (Slider::Brightness, DISPLAY_Y + dy, controls.brightness.is_some()),
        (Slider::Volume, SOUND_Y + dy, controls.volume.is_some()),
    ] {
        // the whole lower part of the module grabs the slider
        if present && inside(x, y, (RAIL, top + SLIDER_DY - 8.0, INNER, SLIDER_H + 16.0)) {
            return Some(Hit::Slider(slider, slider_fraction(slider, x)));
        }
    }
    for (i, action) in SMALL.iter().enumerate() {
        if inside(x, y, (small(i), BOTTOM_Y + dy, SMALL_W, BOTTOM_H)) {
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
        "record" => {
            // a ring around a dot (screen recording)
            cr.arc(0.0, 0.0, 7.5, 0.0, tau);
            let _ = cr.stroke();
            cr.arc(0.0, 0.0, 4.0, 0.0, tau);
            let _ = cr.fill();
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
        View::Sound => draw_sound(cr, renderer, x, y, alpha, frame),
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

    // what is playing, then the modules below it
    let media_rows = usize::from(controls.media.is_some());
    if let Some(media) = &controls.media {
        let (alpha, rise) = frame.arrival(3, base_alpha);
        draw_media(cr, renderer, x, y + MEDIA_Y + rise, alpha, frame, media);
    }
    let y = y + shift(controls);

    // Display
    let (alpha, rise) = frame.arrival(3 + media_rows, base_alpha);
    module(cr, renderer, "Дисплей", x, y + DISPLAY_Y + rise, alpha);
    let present = controls.brightness.is_some();
    slider(cr, x + SLIDER_X, y + DISPLAY_Y + SLIDER_DY + rise, SLIDER_W, SLIDER_H, (frame.level)(Slider::Brightness),
        "sun", 1.0, alpha * if present { 1.0 } else { 0.35 }, (frame.grow)(Slider::Brightness));

    // Sound, with the output it plays through
    let (alpha, rise) = frame.arrival(4 + media_rows, base_alpha);
    let y = y + rise;
    module(cr, renderer, "Звук", x, y + SOUND_Y, alpha);
    if let Some(sink) = controls.current_sink() {
        let (bx, by, bw, bh) = outputs_button(controls);
        let by = by - shift(controls);
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
        if controls.muted { 0.0 } else { (frame.level)(Slider::Volume) }, symbol, (frame.level)(Slider::Volume),
        alpha * if present { 1.0 } else { 0.35 }, (frame.grow)(Slider::Volume));
    let y = y - rise;

    // the keyboard layout and the session
    let (alpha, rise) = frame.arrival(5 + media_rows, base_alpha);
    for (i, action) in SMALL.iter().enumerate() {
        let sx = x + small(i);
        let sy = y + BOTTOM_Y + rise;
        let h = (frame.hover)(*action).clamp(0.0, 1.0);
        let recording = *action == Action::Record && status.recording == Some(true);
        let armed = *action == Action::Logout && frame.logout_armed || recording;
        let missing = *action == Action::Record && status.recording.is_none();
        let alpha = if missing { alpha * 0.4 } else { alpha };
        let press = menu_style::press_scale((frame.pressed)(*action));
        cr.save().unwrap();
        cr.translate(sx + SMALL_W / 2.0, sy + BOTTOM_H / 2.0);
        cr.scale(press, press);
        cr.translate(-(sx + SMALL_W / 2.0), -(sy + BOTTOM_H / 2.0));
        menu_style::group(cr, sx, sy, SMALL_W, BOTTOM_H, h, alpha);
        let (name, label) = match action {
            Action::Layout => ("keyboard", controls.layout.as_deref().map(crate::control::layout_label)
                .unwrap_or_else(|| "Раскладка".into())),
            Action::Record => ("record", if recording { "Стоп".into() } else { "Запись".into() }),
            Action::Lock => ("lock", "Блокировка".into()),
            Action::Sleep => ("sleep", "Сон".into()),
            _ => ("logout", if frame.logout_armed { "Точно?".into() } else { "Выйти".into() }),
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
        crate::motion::text(cr, renderer, &frame.key(&format!("small{i}")), &label, sx + 1.0, sy + 44.0 + (18.0 - lh) / 2.0,
            SMALL_W - 2.0, 0.5, Font::Footnote, colour, alpha * if armed { 1.0 } else { 0.85 });
        cr.restore().unwrap();
    }
}

/// "Now playing": the art, the title over the artist, and previous /
/// play-pause / next; a new track's title rises in, a pressed button dips.
fn draw_media(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, frame: &Frame, media: &Media) {
    if alpha <= 0.002 {
        return;
    }
    menu_style::group(cr, x + RAIL, y, INNER, MEDIA_H, 0.0, alpha);
    let (ax, ay) = (x + RAIL + 12.0, y + (MEDIA_H - MEDIA_ART) / 2.0);
    crate::player::art_tile(cr, renderer, frame.art, &media.art_url, ax, ay, MEDIA_ART, 10.0, alpha);
    let tx = ax + MEDIA_ART + 12.0;
    let tw = x + media_button(0).0 - MEDIA_BUTTON / 2.0 - 6.0 - tx;
    let title_h = menu_style::line_height(cr, renderer, Font::Headline);
    let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
    let top = y + (MEDIA_H - title_h - sub_h) / 2.0;
    let second = if media.artist.is_empty() { media.app.clone() } else { media.artist.clone() };
    crate::motion::text(cr, renderer, &frame.key("media-title"), &media.title, tx, top, tw, 0.0, Font::Headline,
        (1.0, 1.0, 1.0), alpha);
    crate::motion::text(cr, renderer, &frame.key("media-artist"), &second, tx, top + title_h, tw, 0.0, Font::Subhead,
        (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
    for (i, action) in MEDIA_ACTIONS.iter().enumerate() {
        let (cx, cy) = media_button(i);
        let (cx, cy) = (x + cx, y - MEDIA_Y + cy);
        let lit = (frame.hover)(*action).clamp(0.0, 1.0);
        if lit > 0.0 {
            menu_style::well(cr, cx, cy, 17.0, None, 0.8 * lit * alpha);
        }
        let control = match action {
            Action::MediaPrevious => crate::player::Control::Previous,
            Action::MediaPlayPause => crate::player::Control::PlayPause,
            _ => crate::player::Control::Next,
        };
        let size = if control == crate::player::Control::PlayPause { 0.78 } else { 0.62 };
        let press = menu_style::press_scale((frame.pressed)(*action));
        crate::player::control_glyph(cr, control, media.playing, cx, cy, size * press, alpha * (0.85 + 0.15 * lit));
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

/// A thin slider for a list row (no symbol inside); grows when held.
fn row_slider(cr: &Context, x: f64, y: f64, w: f64, value: f64, alpha: f64, grow: f64, colour: (f64, f64, f64)) {
    let swell = 4.0 * grow.clamp(0.0, 1.2);
    let (x, y, w, h) = (x - swell * 0.5, y - swell * 0.5, w + swell, ROW_SLIDER_H + swell);
    menu_style::rounded(cr, x, y, w, h, h / 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.14 * alpha);
    let _ = cr.fill();
    if value <= 0.001 {
        return;
    }
    let fill = h + (w - h) * value.clamp(0.0, 1.0);
    menu_style::rounded(cr, x, y, fill, h, h / 2.0);
    cr.set_source_rgba(colour.0, colour.1, colour.2, 0.94 * alpha);
    let _ = cr.fill();
}

fn caption(cr: &Context, renderer: &Renderer, text: &str, x: f64, y: f64, alpha: f64) {
    menu_style::label(cr, renderer, text, x + RAIL + 4.0, y + 4.0, INNER, Font::Footnote, menu_style::SECONDARY * alpha);
}

fn checkmark(cr: &Context, cx: f64, cy: f64, alpha: f64) {
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

/// The microphone: a capsule and stand; struck through when muted.
pub fn mic(cr: &Context, cx: f64, cy: f64, muted: bool, alpha: f64) {
    cr.save().unwrap();
    stroke_style(cr, 1.8, alpha);
    menu_style::rounded(cr, cx - 3.2, cy - 8.5, 6.4, 11.0, 3.2);
    let _ = cr.fill();
    cr.new_path();
    cr.arc(cx, cy - 1.0, 6.0, 0.0, std::f64::consts::PI);
    cr.move_to(cx, cy + 5.0);
    cr.line_to(cx, cy + 8.0);
    let _ = cr.stroke();
    if muted {
        cr.set_line_width(2.0);
        cr.move_to(cx - 8.0, cy - 8.5);
        cr.line_to(cx + 8.0, cy + 8.5);
        let _ = cr.stroke();
    }
    cr.restore().unwrap();
    cr.new_path();
}

fn draw_sound(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, frame: &Frame) {
    let controls = frame.controls;
    let back = (frame.hover)(Action::Back);
    menu_style::chevron(cr, x + RAIL + 5.0, y + menu_style::HEADER_MID, 7.0, std::f64::consts::PI, alpha * (0.8 + 0.2 * back));
    menu_style::text(cr, renderer, "Назад", x + RAIL + 16.0,
        y + menu_style::HEADER_MID - menu_style::line_height(cr, renderer, Font::Body) / 2.0, 80.0, Font::Body,
        menu_style::BLUE, alpha * (0.85 + 0.15 * back));
    menu_style::centred(cr, renderer, "Звук", (x + 110.0, y + menu_style::HEADER_MID - 12.0, WIDTH - 220.0, 24.0),
        Font::Title, (1.0, 1.0, 1.0), alpha);
    let page = sound_page(controls);
    let body_h = menu_style::line_height(cr, renderer, Font::Body);
    let label_h = menu_style::line_height(cr, renderer, Font::Label);

    // where the sound goes
    caption(cr, renderer, "Вывод", x, y + page.sinks - CAPTION_H, alpha);
    let sinks = &controls.sinks;
    let top = y + page.sinks;
    menu_style::group(cr, x + RAIL, top, INNER, sinks.len().max(1) as f64 * SINK_H, 0.0, alpha);
    if sinks.is_empty() {
        menu_style::centred(cr, renderer, "Нет устройств вывода", (x + RAIL, top, INNER, SINK_H), Font::Body,
            (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
    }
    for (i, sink) in sinks.iter().enumerate() {
        let ry = top + i as f64 * SINK_H;
        if i > 0 {
            menu_style::separator(cr, renderer, x + RAIL + 52.0, x + RAIL + INNER, ry, alpha);
        }
        menu_style::highlight(cr, x + RAIL, ry, INNER, SINK_H, (frame.hover)(Action::Sink(i)), alpha);
        let current = sink.name == controls.default_sink;
        let (wx, wy) = (x + RAIL + 12.0 + 14.0, ry + SINK_H / 2.0);
        let press = menu_style::press_scale((frame.pressed)(Action::Sink(i)));
        menu_style::transformed(cr, wx, wy, press * menu_style::bounce_scale((frame.bounced)(Action::Sink(i))), 0.0, || {
            menu_style::well(cr, wx, wy, 14.0, current.then_some(menu_style::BLUE), alpha);
            glyph(cr, sink_glyph(sink.kind), wx, wy, 1.0, (1.0, 1.0, 1.0, alpha));
        });
        menu_style::label(cr, renderer, &sink.label, x + RAIL + 52.0, ry + (SINK_H - body_h) / 2.0, INNER - 52.0 - 44.0,
            Font::Body, alpha);
        if current {
            checkmark(cr, x + RAIL + INNER - 22.0, ry + SINK_H / 2.0, alpha);
        }
    }

    // the applications playing, each with its own level
    caption(cr, renderer, "Приложения", x, y + page.apps - CAPTION_H, alpha);
    let top = y + page.apps;
    if page.app_rows == 0 {
        menu_style::group(cr, x + RAIL, top, INNER, SINK_H, 0.0, alpha);
        menu_style::centred(cr, renderer, "Ничего не воспроизводится", (x + RAIL, top, INNER, SINK_H), Font::Body,
            (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
    } else {
        menu_style::group(cr, x + RAIL, top, INNER, page.app_rows as f64 * APP_H, 0.0, alpha);
    }
    for (i, app) in controls.apps.iter().take(page.app_rows).enumerate() {
        let ry = top + i as f64 * APP_H;
        if i > 0 {
            menu_style::separator(cr, renderer, x + ROW_SLIDER_X, x + RAIL + INNER, ry, alpha);
        }
        let (ix, iy, size) = (x + RAIL + 12.0, ry + (APP_H - 28.0) / 2.0, 28.0);
        let px = (size * renderer.scale()).round() as i32;
        match (frame.icon)(&app.icon, px) {
            Some(surface) => {
                cr.save().unwrap();
                cr.translate(ix, iy);
                cr.scale(size / surface.width() as f64, size / surface.height() as f64);
                let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                let _ = cr.paint_with_alpha(alpha);
                cr.restore().unwrap();
            }
            None => {
                menu_style::well(cr, ix + size / 2.0, iy + size / 2.0, size / 2.0, None, alpha);
                glyph(cr, "speaker", ix + size / 2.0, iy + size / 2.0, 1.0, (1.0, 1.0, 1.0, alpha));
            }
        }
        let level = (frame.level)(Slider::App(i));
        let percent = format!("{}%", (level * 100.0).round());
        menu_style::label(cr, renderer, &app.name, x + ROW_SLIDER_X, ry + 8.0, ROW_SLIDER_W - 50.0, Font::Label, alpha);
        crate::motion::text(cr, renderer, &format!("quick{:.4}:app{i}", frame.opened), &percent,
            x + ROW_SLIDER_X, ry + 8.0 + (label_h - menu_style::line_height(cr, renderer, Font::Footnote)) / 2.0,
            ROW_SLIDER_W, 1.0, Font::Footnote, (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
        row_slider(cr, x + ROW_SLIDER_X, ry + ROW_SLIDER_DY, ROW_SLIDER_W, if app.muted { 0.0 } else { level },
            alpha * if app.muted { 0.5 } else { 1.0 }, (frame.grow)(Slider::App(i)), (1.0, 1.0, 1.0));
    }

    // the microphone
    caption(cr, renderer, "Микрофон", x, y + page.mic - CAPTION_H, alpha);
    let top = y + page.mic;
    menu_style::group(cr, x + RAIL, top, INNER, MIC_H, 0.0, alpha);
    let present = controls.mic.is_some();
    let muted = controls.mic_muted;
    let (wx, wy) = (x + RAIL + 12.0 + 16.0, top + MIC_H / 2.0);
    let lit = (frame.hover)(Action::MicMute);
    let press = menu_style::press_scale((frame.pressed)(Action::MicMute));
    menu_style::transformed(cr, wx, wy, press * menu_style::bounce_scale((frame.bounced)(Action::MicMute)), 0.0, || {
        menu_style::well(cr, wx, wy, 16.0, muted.then_some(menu_style::RED), alpha * if present { 1.0 } else { 0.4 });
        if !muted && lit > 0.0 {
            menu_style::well(cr, wx, wy, 16.0, Some((0.3, 0.3, 0.32)), alpha * lit * 0.5);
        }
        mic(cr, wx, wy, muted, alpha);
    });
    let name = controls.mic_label.as_deref().unwrap_or(if present { "Микрофон" } else { "Нет микрофона" });
    menu_style::label(cr, renderer, name, x + ROW_SLIDER_X, top + 8.0, ROW_SLIDER_W - 50.0, Font::Label, alpha);
    if present {
        let level = (frame.level)(Slider::Mic);
        let status = if muted { "Выкл.".to_string() } else { format!("{}%", (level * 100.0).round()) };
        crate::motion::text(cr, renderer, &format!("quick{:.4}:mic", frame.opened), &status,
            x + ROW_SLIDER_X, top + 8.0 + (label_h - menu_style::line_height(cr, renderer, Font::Footnote)) / 2.0,
            ROW_SLIDER_W, 1.0, Font::Footnote, if muted { menu_style::RED } else { (1.0, 1.0, 1.0) },
            if muted { alpha } else { menu_style::SECONDARY * alpha });
        row_slider(cr, x + ROW_SLIDER_X, top + ROW_SLIDER_DY, ROW_SLIDER_W, if muted { 0.0 } else { level },
            alpha * if muted { 0.5 } else { 1.0 }, (frame.grow)(Slider::Mic), (1.0, 1.0, 1.0));
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
            apps: vec![App { name: "Firefox".into(), icon: "firefox".into(), streams: vec![1, 2], volume: 0.8, muted: false }],
            mic: Some(0.7),
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
        let (bx, by, bw, _) = outputs_button(&c);
        assert_eq!(hit(View::Main, &c, bx + bw - 10.0, by + 10.0), Some(Hit::Action(Action::Outputs)));
        assert_eq!(hit(View::Main, &c, SLIDER_X, SOUND_Y + SLIDER_DY + 10.0), Some(Hit::Slider(Slider::Volume, 0.0)));
        assert_eq!(hit(View::Main, &c, SLIDER_X + SLIDER_W, DISPLAY_Y + SLIDER_DY + 10.0),
            Some(Hit::Slider(Slider::Brightness, 1.0)));
        assert_eq!(hit(View::Main, &c, WIDTH - 37.0, 40.0), Some(Hit::Action(Action::Close)));
        // with something playing, its buttons take their place and the
        // modules below move down by the module's height
        let mut playing = c.clone();
        playing.media = Some(Media { title: "Says".into(), playing: true, ..Media::default() });
        for (i, action) in MEDIA_ACTIONS.iter().enumerate() {
            let (cx, cy) = media_button(i);
            assert_eq!(hit(View::Main, &playing, cx, cy), Some(Hit::Action(*action)));
        }
        let dy = MEDIA_H + MODULE_GAP;
        assert_eq!(hit(View::Main, &playing, small(0) + 30.0, BOTTOM_Y + dy + 30.0), Some(Hit::Action(Action::Layout)));
        assert_eq!(height(View::Main, &playing), HEIGHT + dy);
        assert_eq!(hit(View::Main, &c, WIDTH / 2.0, HEIGHT - 8.0), None);
        let page = sound_page(&c);
        assert_eq!(hit(View::Sound, &c, 200.0, page.sinks + SINK_H + 10.0), Some(Hit::Action(Action::Sink(1))));
        assert_eq!(hit(View::Sound, &c, 40.0, 40.0), Some(Hit::Action(Action::Back)));
        assert_eq!(hit(View::Sound, &c, ROW_SLIDER_X + ROW_SLIDER_W, page.apps + 40.0), Some(Hit::Slider(Slider::App(0), 1.0)));
        assert_eq!(hit(View::Sound, &c, 40.0, page.mic + 20.0), Some(Hit::Action(Action::MicMute)));
        assert_eq!(hit(View::Sound, &c, ROW_SLIDER_X, page.mic + 30.0), Some(Hit::Slider(Slider::Mic, 0.0)));
        assert_eq!(page.height, height(View::Sound, &c));
        let without = Controls { volume: None, ..controls() };
        assert_eq!(hit(View::Main, &without, SLIDER_X + 50.0, SOUND_Y + SLIDER_DY + 10.0), None);
    }
}
