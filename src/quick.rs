//! The island's Control Center, laid out as Vorssaint's open island: a
//! title row whose actions show under the pointer, a row of cards (what is
//! playing, sound and display levels) and a rail of round shortcut tiles;
//! the sound page lists outputs, applications and the microphone; the
//! sections page is Vorssaint's gallery of the island's destinations.
use cairo::Context;

use crate::control::{App, Sink, SinkKind};
use crate::island::{Font, Renderer};
use crate::menu_style;

pub const WIDTH: f64 = 560.0;
/// the side inset of every page
const RAIL: f64 = 24.0;
const INNER: f64 = WIDTH - 2.0 * RAIL;
/// the title row
const HEADER_TOP: f64 = 12.0;
const HEADER_H: f64 = 36.0;
/// where a page's content starts, under the title row
const PAGE_TOP: f64 = HEADER_TOP + HEADER_H + 12.0;
const BOTTOM: f64 = 16.0;
/// between the card row and the rail
const ROW_GAP: f64 = 10.0;
/// the card row
const CARD_Y: f64 = PAGE_TOP;
const CARD_H: f64 = 96.0;
const CARD_R: f64 = 18.0;
/// the level card beside the music card
const LEVELS_W: f64 = 184.0;
const ART: f64 = CARD_H - 24.0;
const MEDIA_BUTTON: f64 = 36.0;
/// the shortcut rail: round wells over a one-line label
const RAIL_Y: f64 = CARD_Y + CARD_H + ROW_GAP;
const PER_ROW: usize = 5;
const SHORTCUT_GAP: f64 = 4.0;
const SHORTCUT_W: f64 = (INNER - (PER_ROW - 1) as f64 * SHORTCUT_GAP) / PER_ROW as f64;
const SHORTCUT_H: f64 = 74.0;
const SHORTCUT_R: f64 = 14.0;
const WELL_R: f64 = 20.0;
/// the main page
const RAIL_ROWS: usize = SHORTCUTS.len().div_ceil(PER_ROW);
pub const HEIGHT: f64 = RAIL_Y + RAIL_ROWS as f64 * SHORTCUT_H + (RAIL_ROWS - 1) as f64 * SHORTCUT_GAP + BOTTOM;
/// the title row's trailing button (collapse / close)
const CLOSE: f64 = 28.0;
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
/// the sections gallery: rows of square-ish tiles, as Vorssaint's
const SECTION_COLS: usize = 5;
const GALLERY_GAP: f64 = 8.0;
const SECTION_W: f64 = (INNER - (SECTION_COLS - 1) as f64 * GALLERY_GAP) / SECTION_COLS as f64;
const SECTION_H: f64 = 86.0;
const SECTION_R: f64 = 16.0;

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
    /// night mode, eased 0..1
    pub night: f64,
    /// the companion is out, eased 0..1
    pub mascot: f64,
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
    /// night mode (gammastep) is on
    pub night: bool,
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
    /// the gallery of destinations
    Sections,
}

/// A destination in the sections gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Control,
    Sound,
    Calendar,
    Notifications,
    Timer,
    Wifi,
    Bluetooth,
    Search,
    Files,
    Clipboard,
}

/// The gallery, in reading (and keyboard) order.
pub const SECTIONS: [Section; 10] = [
    Section::Control, Section::Sound, Section::Calendar, Section::Notifications, Section::Timer,
    Section::Wifi, Section::Bluetooth, Section::Search, Section::Files, Section::Clipboard,
];

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Control => "Управление",
            Section::Sound => "Звук",
            Section::Calendar => "Календарь",
            Section::Notifications => "Уведомления",
            Section::Timer => "Таймер",
            Section::Wifi => "Wi-Fi",
            Section::Bluetooth => "Bluetooth",
            Section::Search => "Поиск",
            Section::Files => "Файлы",
            Section::Clipboard => "Буфер",
        }
    }

    /// Its symbol's colour: the calendar red, the timer orange, the rest
    /// a soft white.
    fn tint(self) -> (f64, f64, f64, f64) {
        match self {
            Section::Calendar => (menu_style::RED.0, menu_style::RED.1, menu_style::RED.2, 1.0),
            Section::Timer => (menu_style::ORANGE.0, menu_style::ORANGE.1, menu_style::ORANGE.2, 1.0),
            _ => (1.0, 1.0, 1.0, 0.85),
        }
    }
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
    let top = PAGE_TOP;
    let sinks = top + CAPTION_H;
    let apps_caption = sinks + controls.sinks.len().max(1) as f64 * SINK_H + SECTION_GAP;
    let apps = apps_caption + CAPTION_H;
    let app_rows = controls.apps.len().min(MAX_APPS);
    let apps_h = if app_rows == 0 { SINK_H } else { app_rows as f64 * APP_H };
    let mic = apps + apps_h + SECTION_GAP + CAPTION_H;
    SoundPage { sinks, apps, app_rows, mic, height: mic + MIC_H + BOTTOM }
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
    /// a screenshot of the whole screen (once the menu has folded)
    Screenshot,
    /// start / stop a screen recording
    Record,
    /// night mode on the display module
    Night,
    /// the companion on the closed line, on or off
    Mascot,
    Lock,
    Sleep,
    Logout,
    Sink(usize),
    /// "Now playing": previous, play / pause, next
    MediaPrevious,
    MediaPlayPause,
    MediaNext,
    /// pause and take the player off the island until it plays again
    MediaClear,
    /// a destination in the sections gallery
    Section(Section),
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

/// The shortcut rail, in reading (and keyboard) order.
const SHORTCUTS: [Action; 14] = [
    Action::Wifi, Action::Bluetooth, Action::Timer, Action::Notifications, Action::Dnd, Action::Night, Action::Mascot,
    Action::Launcher, Action::Screenshot, Action::Record, Action::Layout, Action::Lock, Action::Sleep, Action::Logout,
];
/// Keyboard order of the main page (the transport buttons come first
/// while something plays, see the island's `quick_keys`).
pub const ACTIONS: [Action; 15] = [
    Action::Outputs, Action::Wifi, Action::Bluetooth, Action::Timer, Action::Notifications, Action::Dnd, Action::Night,
    Action::Mascot, Action::Launcher, Action::Screenshot, Action::Record, Action::Layout, Action::Lock, Action::Sleep, Action::Logout,
];

pub fn height(view: View, controls: &Controls) -> f64 {
    match view {
        View::Main => HEIGHT,
        View::Sound => sound_page(controls).height,
        View::Sections => sections_height(),
    }
}

fn section_tile(i: usize) -> Rect {
    let (row, col) = (i / SECTION_COLS, i % SECTION_COLS);
    (RAIL + col as f64 * (SECTION_W + GALLERY_GAP), PAGE_TOP + row as f64 * (SECTION_H + GALLERY_GAP), SECTION_W, SECTION_H)
}

fn sections_height() -> f64 {
    let rows = SECTIONS.len().div_ceil(SECTION_COLS);
    PAGE_TOP + rows as f64 * SECTION_H + (rows - 1) as f64 * GALLERY_GAP + BOTTOM
}

/// A section tile's centre (for the island's tests).
#[cfg(test)]
pub fn section_centre(section: Section) -> (f64, f64) {
    let i = SECTIONS.iter().position(|s| *s == section).unwrap();
    let (x, y, w, h) = section_tile(i);
    (x + w / 2.0, y + h / 2.0)
}

type Rect = (f64, f64, f64, f64);

/// A shortcut tile; a short last row is centred, as Vorssaint's rails are.
fn shortcut(i: usize) -> Rect {
    let (row, col) = (i / PER_ROW, i % PER_ROW);
    let in_row = (SHORTCUTS.len() - row * PER_ROW).min(PER_ROW);
    let lead = (PER_ROW - in_row) as f64 * (SHORTCUT_W + SHORTCUT_GAP) / 2.0;
    (RAIL + lead + col as f64 * (SHORTCUT_W + SHORTCUT_GAP), RAIL_Y + row as f64 * (SHORTCUT_H + SHORTCUT_GAP), SHORTCUT_W, SHORTCUT_H)
}

/// A shortcut's centre and the action it holds (for the island's tests).
#[cfg(test)]
pub fn shortcut_centre(i: usize) -> (Action, f64, f64) {
    let (x, y, w, h) = shortcut(i);
    (SHORTCUTS[i], x + w / 2.0, y + h / 2.0)
}

fn music_card() -> Rect {
    (RAIL, CARD_Y, INNER - LEVELS_W - ROW_GAP, CARD_H)
}

/// The card a level lives in: beside the music card both share one slim
/// card; alone, each has a card of its own.
fn level_card(slider: Slider, media: bool) -> Rect {
    if media {
        return (RAIL + INNER - LEVELS_W, CARD_Y, LEVELS_W, CARD_H);
    }
    let half = (INNER - ROW_GAP) / 2.0;
    let x = if slider == Slider::Volume { RAIL } else { RAIL + half + ROW_GAP };
    (x, CARD_Y, half, CARD_H)
}

/// A level's label line and its slider.
struct Level {
    label: Rect,
    slider: Rect,
}

fn level(slider: Slider, media: bool) -> Level {
    let (cx, cy, cw, ch) = level_card(slider, media);
    if media {
        let top = cy + 10.0 + if slider == Slider::Volume { 0.0 } else { 42.0 };
        Level { label: (cx + 14.0, top, cw - 28.0, 16.0), slider: (cx + 14.0, top + 20.0, cw - 28.0, 16.0) }
    } else {
        Level { label: (cx + 16.0, cy + 14.0, cw - 32.0, 22.0), slider: (cx + 16.0, cy + ch - 16.0 - 26.0, cw - 32.0, 26.0) }
    }
}

/// The output picker: the volume's label line.
fn outputs_button(controls: &Controls) -> Rect {
    level(Slider::Volume, controls.media.is_some()).label
}

/// The title row's trailing button.
fn close_button() -> Rect {
    (WIDTH - RAIL - CLOSE + 4.0, HEADER_TOP + (HEADER_H - CLOSE) / 2.0, CLOSE, CLOSE)
}

/// The sound page's way back: a chevron and its title.
fn back_button() -> Rect {
    (RAIL - 6.0, HEADER_TOP, 120.0, HEADER_H)
}

/// The transport buttons' centres in the music card.
/// (The clear button sits apart, at the card's right edge.)
const MEDIA_ACTIONS: [Action; 4] = [Action::MediaPrevious, Action::MediaPlayPause, Action::MediaNext, Action::MediaClear];

fn media_button(i: usize) -> (f64, f64) {
    let (x, y, w, h) = music_card();
    let row = y + h - 12.0 - MEDIA_BUTTON / 2.0;
    if i == 3 {
        return (x + w - 12.0 - MEDIA_BUTTON / 2.0 + 4.0, row);
    }
    let text_x = x + 12.0 + ART + 14.0;
    (text_x + MEDIA_BUTTON / 2.0 - 8.0 + i as f64 * (MEDIA_BUTTON + 4.0), row)
}

fn inside(x: f64, y: f64, rect: Rect) -> bool {
    x >= rect.0 && x <= rect.0 + rect.2 && y >= rect.1 && y <= rect.1 + rect.3
}

/// Where along `slider` the menu-relative `x` falls (0..1); also used
/// while dragging.
pub fn slider_fraction(slider: Slider, controls: &Controls, x: f64) -> f64 {
    let (sx, sw) = match slider {
        Slider::Brightness | Slider::Volume => {
            let (sx, _, sw, _) = level(slider, controls.media.is_some()).slider;
            (sx, sw)
        }
        Slider::App(_) | Slider::Mic => (ROW_SLIDER_X, ROW_SLIDER_W),
    };
    ((x - sx) / sw).clamp(0.0, 1.0)
}

pub fn hit(view: View, controls: &Controls, x: f64, y: f64) -> Option<Hit> {
    let (cx, cy, cw, ch) = close_button();
    if inside(x, y, (cx - 6.0, cy - 6.0, cw + 12.0, ch + 12.0)) {
        return Some(Hit::Action(Action::Close));
    }
    if view == View::Sections {
        return SECTIONS.iter().enumerate()
            .find(|(i, _)| inside(x, y, section_tile(*i)))
            .map(|(_, section)| Hit::Action(Action::Section(*section)));
    }
    if view == View::Sound {
        if inside(x, y, back_button()) {
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
            return Some(Hit::Slider(app, slider_fraction(app, controls, x)));
        }
        if controls.mic.is_some() && (page.mic..page.mic + MIC_H).contains(&y) {
            return Some(if x < ROW_SLIDER_X - 8.0 {
                Hit::Action(Action::MicMute)
            } else {
                Hit::Slider(Slider::Mic, slider_fraction(Slider::Mic, controls, x))
            });
        }
        return None;
    }
    for (i, action) in SHORTCUTS.iter().enumerate() {
        if inside(x, y, shortcut(i)) {
            return Some(Hit::Action(*action));
        }
    }
    let media = controls.media.is_some();
    if media {
        for (i, action) in MEDIA_ACTIONS.iter().enumerate() {
            let (cx, cy) = media_button(i);
            if (x - cx).abs() <= MEDIA_BUTTON / 2.0 + 2.0 && (y - cy).abs() <= MEDIA_BUTTON / 2.0 + 4.0 {
                return Some(Hit::Action(*action));
            }
        }
    }
    if !controls.sinks.is_empty() && inside(x, y, outputs_button(controls)) {
        return Some(Hit::Action(Action::Outputs));
    }
    for (slider, present) in [(Slider::Volume, controls.volume.is_some()), (Slider::Brightness, controls.brightness.is_some())] {
        let (sx, sy, sw, sh) = level(slider, media).slider;
        // a little room around the slider grabs it too
        if present && inside(x, y, (sx - 8.0, sy - 8.0, sw + 16.0, sh + 16.0)) {
            return Some(Hit::Slider(slider, slider_fraction(slider, controls, x)));
        }
    }
    None
}

pub fn icon(cr: &Context, action: Action, x: f64, y: f64, alpha: f64, scale: f64) {
    menu_style::stamp(cr, || format!("icon {action:?} {scale}"), x, y, 16.0 * scale + 3.0, (1.0, 1.0, 1.0, alpha), |cr| {
        cr.scale(scale, scale);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.set_line_width(2.2);
        icon_body(cr, action);
    });
}

fn icon_body(cr: &Context, action: Action) {
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
            // moon.fill: a disc with a bite taken from its upper right
            cr.save().unwrap();
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            cr.rectangle(-20.0, -20.0, 40.0, 40.0);
            cr.arc(6.5, -5.5, 8.5, 0.0, std::f64::consts::TAU);
            cr.clip();
            cr.arc(0.0, 0.0, 10.5, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            cr.restore().unwrap();
        }
        _ => {}
    }
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
pub fn glyph(cr: &Context, name: &str, cx: f64, cy: f64, level: f64, rgba: (f64, f64, f64, f64)) {
    // the level only matters in steps of half (the speaker's waves)
    let steps = (level * 2.0 + 0.98).floor();
    menu_style::stamp(cr, || format!("glyph {name} {steps}"), cx, cy, 15.0, rgba, |cr| {
        stroke_style(cr, 1.8, 1.0);
        glyph_body(cr, name, level);
    });
}

fn glyph_body(cr: &Context, name: &str, level: f64) {
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
        "calendar" => {
            // a page with a binding: the top band filled, two rings, days
            menu_style::rounded(cr, -8.5, -7.0, 17.0, 15.5, 3.0);
            let _ = cr.stroke();
            cr.rectangle(-8.5, -6.0, 17.0, 3.6);
            let _ = cr.fill();
            cr.move_to(-4.0, -9.5);
            cr.line_to(-4.0, -6.5);
            cr.move_to(4.0, -9.5);
            cr.line_to(4.0, -6.5);
            let _ = cr.stroke();
            for (dx, dy) in [(-4.0, 0.5), (0.0, 0.5), (4.0, 0.5), (-4.0, 4.2), (0.0, 4.2)] {
                cr.rectangle(dx - 1.0, dy - 1.0, 2.0, 2.0);
            }
            let _ = cr.fill();
        }
        "sunset" => {
            // night mode: half a sun sinking behind the horizon
            cr.arc(0.0, 3.0, 5.0, std::f64::consts::PI, tau);
            let _ = cr.fill();
            for i in 1..4 {
                let a = std::f64::consts::PI + i as f64 * std::f64::consts::PI / 4.0;
                cr.move_to(7.6 * a.cos(), 3.0 + 7.6 * a.sin());
                cr.line_to(9.8 * a.cos(), 3.0 + 9.8 * a.sin());
            }
            cr.move_to(-9.5, 3.0);
            cr.line_to(9.5, 3.0);
            cr.move_to(-5.5, 7.0);
            cr.line_to(5.5, 7.0);
            let _ = cr.stroke();
        }
        "grid" => {
            // the sections gallery: four rounded squares
            for (dx, dy) in [(-7.5, -7.5), (1.0, -7.5), (-7.5, 1.0), (1.0, 1.0)] {
                menu_style::rounded(cr, dx, dy, 6.5, 6.5, 2.0);
                let _ = cr.fill();
            }
        }
        "sliders" => {
            // the Control Center: two switches, one on and one off
            for (y, knob) in [(-4.5, 5.5), (4.5, -5.5)] {
                menu_style::rounded(cr, -10.0, y - 3.8, 20.0, 7.6, 3.8);
                let _ = cr.stroke();
                cr.arc(knob, y, 2.4, 0.0, tau);
                let _ = cr.fill();
            }
        }
        "folder" => {
            // a folder with its tab
            cr.move_to(-9.5, -6.5);
            cr.line_to(-3.5, -6.5);
            cr.line_to(-1.5, -4.0);
            cr.line_to(9.5, -4.0);
            cr.line_to(9.5, 7.5);
            cr.line_to(-9.5, 7.5);
            cr.close_path();
            let _ = cr.stroke();
            cr.move_to(-9.5, -1.0);
            cr.line_to(9.5, -1.0);
            let _ = cr.stroke();
        }
        "clipboard" => {
            // a board with its clip and two lines of text
            menu_style::rounded(cr, -7.5, -7.5, 15.0, 17.0, 2.5);
            let _ = cr.stroke();
            menu_style::rounded(cr, -3.5, -9.5, 7.0, 4.0, 1.5);
            let _ = cr.fill();
            cr.move_to(-3.5, 0.0);
            cr.line_to(3.5, 0.0);
            cr.move_to(-3.5, 4.0);
            cr.line_to(1.5, 4.0);
            let _ = cr.stroke();
        }
        "screenshot" => {
            // a viewfinder: four corners around the screen, a dot in it
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                cr.move_to(sx * 9.0, sy * 3.5);
                cr.line_to(sx * 9.0, sy * 7.0);
                cr.line_to(sx * 5.0, sy * 7.0);
            }
            let _ = cr.stroke();
            cr.arc(0.0, 0.0, 2.2, 0.0, tau);
            let _ = cr.fill();
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

/// A card of the open island: a faint white wash, no outline.
fn card(cr: &Context, rect: Rect, r: f64, alpha: f64) {
    menu_style::rounded(cr, rect.0, rect.1, rect.2, rect.3, r);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.065 * alpha);
    let _ = cr.fill();
}

/// The title row: the page's title, and at its end a faint ellipsis that
/// becomes the collapse button under the pointer (a title, not a toolbar).
fn header(cr: &Context, renderer: &Renderer, x: f64, y: f64, title: &str, lead: f64, close_hover: f64, alpha: f64) {
    let title_h = menu_style::line_height(cr, renderer, Font::Title);
    menu_style::text(cr, renderer, title, x + RAIL + lead, y + HEADER_TOP + (HEADER_H - title_h) / 2.0, 260.0, Font::Title,
        (1.0, 1.0, 1.0), alpha);
    let (bx, by, bw, bh) = close_button();
    let (cx, cy) = (x + bx + bw / 2.0, y + by + bh / 2.0);
    let lit = close_hover.clamp(0.0, 1.0);
    if lit > 0.0 {
        menu_style::rounded(cr, x + bx, y + by, bw, bh, 8.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.09 * lit * alpha);
        let _ = cr.fill();
    }
    // the dots gather into the chevron
    let dots = 1.0 - lit;
    if dots > 0.01 {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.35 * dots * alpha);
        for i in -1..=1 {
            cr.new_path();
            cr.arc(cx + i as f64 * 5.0 * dots, cy, 1.4, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }
    if lit > 0.01 {
        menu_style::chevron(cr, cx, cy + 1.5, 5.0 * (0.6 + 0.4 * lit), -std::f64::consts::FRAC_PI_2, lit * alpha);
    }
}

/// The menu at `x`, `y`.
pub fn draw(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, view: View, close_hover: f64, frame: &Frame) {
    if alpha <= 0.002 {
        return;
    }
    match view {
        View::Main => {
            header(cr, renderer, x, y, "Управление", 0.0, close_hover, alpha);
            draw_main(cr, renderer, x, y, alpha, frame);
        }
        View::Sound => {
            let back = (frame.hover)(Action::Back).clamp(0.0, 1.0);
            let (bx, by, _, bh) = back_button();
            menu_style::chevron(cr, x + bx + 12.0, y + by + bh / 2.0, 6.5, std::f64::consts::PI, alpha * (0.7 + 0.3 * back));
            header(cr, renderer, x, y, "Звук", 18.0, close_hover, alpha);
            draw_sound(cr, renderer, x, y, alpha, frame);
        }
        View::Sections => {
            header(cr, renderer, x, y, "Разделы", 0.0, close_hover, alpha);
            for (i, section) in SECTIONS.iter().enumerate() {
                let (alpha, rise) = frame.arrival(i / SECTION_COLS, alpha);
                if alpha > 0.002 {
                    draw_section(cr, renderer, offset(section_tile(i), x, y + rise), *section, alpha, frame);
                }
            }
        }
    }
}

/// A gallery tile: a faint rounded plate (lit under the pointer or the
/// keyboard), the section's symbol in its colour, its name, and for the
/// timer and the notifications a line of their state.
fn draw_section(cr: &Context, renderer: &Renderer, rect: Rect, section: Section, alpha: f64, frame: &Frame) {
    let action = Action::Section(section);
    let lit = (frame.hover)(action).clamp(0.0, 1.0);
    let (tx, ty, tw, th) = rect;
    let (cx, cy) = (tx + tw / 2.0, ty + th / 2.0);
    let press = menu_style::press_scale((frame.pressed)(action));
    menu_style::transformed(cr, cx, cy, press, 0.0, || {
        menu_style::rounded(cr, tx, ty, tw, th, SECTION_R);
        cr.set_source_rgba(1.0, 1.0, 1.0, (0.045 + 0.075 * lit) * alpha);
        let _ = cr.fill();
        menu_style::rounded(cr, tx + 0.5, ty + 0.5, tw - 1.0, th - 1.0, SECTION_R - 0.5);
        cr.set_line_width(1.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, (0.04 + 0.5 * lit) * alpha);
        let _ = cr.stroke();
        // the symbol, drawn white and then poured into its colour
        let tint = section.tint();
        cr.push_group();
        section_symbol(cr, section, cx, ty + 27.0);
        let symbol = cr.pop_group().unwrap();
        cr.set_source_rgba(tint.0, tint.1, tint.2, tint.3 * alpha);
        let _ = cr.mask(&symbol);
        let state = match section {
            Section::Notifications if frame.status.unread > 0 => Some(format!("{} новых", frame.status.unread.min(99))),
            Section::Timer => frame.status.timer.map(|(left, running)| {
                if running { clock(left) } else { format!("‖ {}", clock(left)) }
            }),
            _ => None,
        };
        let title_y = ty + if state.is_some() { 48.0 } else { 54.0 };
        let line = menu_style::line_height(cr, renderer, Font::Footnote);
        menu_style::centred(cr, renderer, section.title(), (tx, title_y, tw, line), Font::Footnote, (1.0, 1.0, 1.0), alpha);
        if let Some(state) = state {
            menu_style::centred(cr, renderer, &state, (tx, title_y + line + 1.0, tw, line), Font::Tiny,
                (1.0, 1.0, 1.0), alpha * menu_style::SECONDARY);
        }
    });
}

/// A section's symbol at (cx, cy), in white.
fn section_symbol(cr: &Context, section: Section, cx: f64, cy: f64) {
    let white = (1.0, 1.0, 1.0, 1.0);
    match section {
        Section::Control => glyph(cr, "sliders", cx, cy, 1.0, white),
        Section::Sound => menu_style::transformed(cr, cx, cy, 1.35, 0.0, || glyph(cr, "speaker", cx + 1.5, cy, 1.0, white)),
        Section::Calendar => glyph(cr, "calendar", cx, cy, 1.0, white),
        Section::Notifications => icon(cr, Action::Notifications, cx, cy, 1.0, 0.8),
        Section::Timer => icon(cr, Action::Timer, cx, cy, 1.0, 0.8),
        Section::Wifi => icon(cr, Action::Wifi, cx, cy, 1.0, 0.8),
        Section::Bluetooth => icon(cr, Action::Bluetooth, cx, cy, 1.0, 0.75),
        Section::Search => icon(cr, Action::Launcher, cx, cy, 1.0, 0.8),
        Section::Files => glyph(cr, "folder", cx, cy, 1.0, white),
        Section::Clipboard => glyph(cr, "clipboard", cx, cy, 1.0, white),
    }
}

fn draw_main(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, frame: &Frame) {
    let controls = frame.controls;
    let media = controls.media.is_some();
    // the cards, then the rail's two rows arrive in a wave
    let (card_alpha, rise) = frame.arrival(0, alpha);
    if let Some(m) = &controls.media {
        draw_media(cr, renderer, x, y + rise, card_alpha, frame, m);
    }
    if media {
        card(cr, offset(level_card(Slider::Volume, true), x, y + rise), CARD_R, card_alpha);
    } else {
        for slider in [Slider::Volume, Slider::Brightness] {
            card(cr, offset(level_card(slider, false), x, y + rise), CARD_R, card_alpha);
        }
    }
    draw_level(cr, renderer, x, y + rise, card_alpha, frame, Slider::Volume);
    draw_level(cr, renderer, x, y + rise, card_alpha, frame, Slider::Brightness);
    for (i, action) in SHORTCUTS.iter().enumerate() {
        let (alpha, rise) = frame.arrival(1 + i / PER_ROW, alpha);
        if alpha > 0.002 {
            draw_shortcut(cr, renderer, offset(shortcut(i), x, y + rise), *action, i, alpha, frame);
        }
    }
}

fn offset(rect: Rect, x: f64, y: f64) -> Rect {
    (rect.0 + x, rect.1 + y, rect.2, rect.3)
}

/// A level: its name (and for sound, the output, which opens the sound
/// page), and the slider.
fn draw_level(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, frame: &Frame, slider: Slider) {
    let controls = frame.controls;
    let media = controls.media.is_some();
    let part = level(slider, media);
    let (lx, ly, lw, lh) = offset(part.label, x, y);
    let font = if media { Font::Footnote } else { Font::Headline };
    let line = menu_style::line_height(cr, renderer, font);
    let (name, present) = match slider {
        Slider::Volume => ("Звук", controls.volume.is_some()),
        _ => ("Дисплей", controls.brightness.is_some()),
    };
    let name_w = renderer.layout(cr, name, font).pixel_extents().1.width() as f64;
    menu_style::text(cr, renderer, name, lx, ly + (lh - line) / 2.0, lw, font, (1.0, 1.0, 1.0),
        alpha * if media { menu_style::SECONDARY } else { 1.0 });
    if slider == Slider::Volume {
        if let Some(sink) = controls.current_sink() {
            // the output, right-aligned after the name, with a chevron
            let lit = (frame.hover)(Action::Outputs).clamp(0.0, 1.0);
            let sub = Font::Footnote;
            let sub_h = menu_style::line_height(cr, renderer, sub);
            let room = lw - name_w - 12.0 - 14.0;
            let label_w = renderer.layout(cr, &sink.label, sub).pixel_extents().1.width() as f64;
            let text_w = label_w.min(room).max(0.0);
            let right = lx + lw;
            menu_style::chevron(cr, right - 3.0, ly + lh / 2.0, 3.5, 0.0, alpha * (0.45 + 0.55 * lit));
            if room > 24.0 {
                crate::motion::text(cr, renderer, &frame.key("sink"), &sink.label, right - 12.0 - room, ly + (lh - sub_h) / 2.0,
                    room, 1.0, sub, (1.0, 1.0, 1.0), alpha * (menu_style::SECONDARY + (1.0 - menu_style::SECONDARY) * lit));
                if !media {
                    glyph(cr, sink_glyph(sink.kind), right - 24.0 - text_w, ly + lh / 2.0, 1.0,
                        (1.0, 1.0, 1.0, alpha * (menu_style::SECONDARY + 0.4 * lit)));
                }
            }
        }
    }
    let (sx, sy, sw, sh) = offset(part.slider, x, y);
    let (symbol, value) = match slider {
        Slider::Volume if controls.muted => ("speaker-muted", 0.0),
        Slider::Volume => ("speaker", (frame.level)(Slider::Volume)),
        _ => ("sun", (frame.level)(Slider::Brightness)),
    };
    let fade = if present { 1.0 } else { 0.35 };
    if media {
        thin_slider(cr, sx, sy, sw, sh, value, symbol, (frame.level)(slider), alpha * fade, (frame.grow)(slider));
    } else {
        self::slider(cr, sx, sy, sw, sh, value, symbol, (frame.level)(slider), alpha * fade, (frame.grow)(slider));
    }
}

/// The slim card's slider: the symbol stands before the track.
#[allow(clippy::too_many_arguments)]
fn thin_slider(cr: &Context, x: f64, y: f64, w: f64, h: f64, value: f64, symbol: &str, level: f64, alpha: f64, grow: f64) {
    glyph(cr, symbol, x + 8.0, y + h / 2.0, level, (1.0, 1.0, 1.0, 0.85 * alpha));
    let track = x + 24.0;
    row_slider_sized(cr, track, y + (h - 8.0) / 2.0, w - 24.0, 8.0, value, alpha, grow, (1.0, 1.0, 1.0));
}

/// A shortcut tile: a round well over a one-line label; on, the well takes
/// its colour. The label reads the tile's state when it has one.
#[allow(clippy::too_many_arguments)]
fn draw_shortcut(cr: &Context, renderer: &Renderer, rect: Rect, action: Action, i: usize, alpha: f64, frame: &Frame) {
    let status = frame.status;
    let controls = frame.controls;
    let (tx, ty, tw, th) = rect;
    let lit = (frame.hover)(action).clamp(0.0, 1.0);
    let missing = action == Action::Record && status.recording.is_none();
    let alpha = if missing { alpha * 0.4 } else { alpha };
    let dnd = status.dnd.clamp(0.0, 1.0);
    let night = status.night.clamp(0.0, 1.0);
    let recording = status.recording == Some(true);
    let armed = action == Action::Logout && frame.logout_armed;
    // how "on" the tile is, and its colour then
    let (on, accent) = match action {
        Action::Timer => (if status.timer.is_some() { 1.0 } else { 0.0 }, menu_style::ORANGE),
        Action::Notifications => (if status.unread > 0 { 1.0 } else { 0.0 }, menu_style::RED),
        Action::Dnd => (dnd, menu_style::PURPLE),
        Action::Night => (night, menu_style::ORANGE),
        Action::Mascot => (status.mascot.clamp(0.0, 1.0), crate::mascot::COLOUR),
        Action::Record => (if recording { 1.0 } else { 0.0 }, menu_style::RED),
        Action::Logout => (if armed { 1.0 } else { 0.0 }, menu_style::RED),
        Action::Wifi | Action::Bluetooth => (1.0, menu_style::BLUE),
        _ => (0.0, menu_style::BLUE),
    };
    // hovered, the tile lifts a touch under a light wash; pressed, it dips
    let press = menu_style::press_scale((frame.pressed)(action)) * (1.0 + 0.022 * lit);
    let (cx, cy) = (tx + tw / 2.0, ty + th / 2.0);
    menu_style::transformed(cr, cx, cy, press, 0.0, || {
        if lit > 0.0 {
            menu_style::rounded(cr, tx, ty, tw, th, SHORTCUT_R);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.09 * lit * alpha);
            let _ = cr.fill();
        }
        let (wx, wy) = (cx, ty + 4.0 + WELL_R);
        let since = (frame.bounced)(action);
        let (scale, angle) = match action {
            // asking to confirm, the symbol shakes its head
            Action::Logout => (1.0, menu_style::wiggle_angle(since)),
            _ => (menu_style::bounce_scale(since), 0.0),
        };
        menu_style::transformed(cr, wx, wy, scale, angle, || {
            menu_style::disc(cr, wx, wy, WELL_R, (1.0, 1.0, 1.0, 0.075 * (1.0 - on) * alpha));
            menu_style::disc(cr, wx, wy, WELL_R, (accent.0, accent.1, accent.2, on * alpha));
            match action {
                Action::Wifi | Action::Bluetooth | Action::Timer | Action::Launcher | Action::Notifications | Action::Dnd => {
                    icon(cr, action, wx, wy, alpha, 0.68);
                }
                Action::Night => glyph(cr, "sunset", wx, wy, 1.0, (1.0, 1.0, 1.0, alpha)),
                Action::Mascot => {
                    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
                    crate::mascot::symbol(cr, wx, wy + 0.5, 0.82);
                }
                Action::Record => glyph(cr, "record", wx, wy, 1.0, (1.0, 1.0, 1.0, alpha)),
                Action::Screenshot => glyph(cr, "screenshot", wx, wy, 1.0, (1.0, 1.0, 1.0, alpha)),
                Action::Layout => glyph(cr, "keyboard", wx, wy, 1.0, (1.0, 1.0, 1.0, alpha)),
                Action::Lock => glyph(cr, "lock", wx, wy, 1.0, (1.0, 1.0, 1.0, alpha)),
                Action::Sleep => glyph(cr, "sleep", wx, wy, 1.0, (1.0, 1.0, 1.0, alpha)),
                _ => glyph(cr, "logout", wx, wy, 1.0, (1.0, 1.0, 1.0, alpha)),
            }
        });
        let label = match action {
            Action::Wifi => "Wi-Fi".to_string(),
            Action::Bluetooth => "Bluetooth".to_string(),
            Action::Timer => match status.timer {
                Some((left, true)) => clock(left),
                Some((left, false)) => format!("‖ {}", clock(left)),
                None => "Таймер".to_string(),
            },
            Action::Notifications => match status.unread {
                0 => "Уведомления".to_string(),
                n => format!("{} новых", n.min(99)),
            },
            Action::Dnd => "Тишина".to_string(),
            Action::Night => "Ночь".to_string(),
            Action::Mascot => "Друг".to_string(),
            Action::Launcher => "Поиск".to_string(),
            Action::Record => if recording { "Стоп".to_string() } else { "Запись".to_string() },
            Action::Screenshot => "Скриншот".to_string(),
            Action::Layout => controls.layout.as_deref().map(crate::control::layout_label).unwrap_or_else(|| "Раскладка".into()),
            Action::Lock => "Блокировка".to_string(),
            Action::Sleep => "Сон".to_string(),
            _ => if armed { "Точно?".to_string() } else { "Выйти".to_string() },
        };
        let colour = match action {
            Action::Timer if status.timer.is_some() => menu_style::ORANGE,
            Action::Logout if armed => menu_style::RED,
            Action::Record if recording => menu_style::RED,
            _ => (1.0, 1.0, 1.0),
        };
        let lh = menu_style::line_height(cr, renderer, Font::Footnote);
        crate::motion::text(cr, renderer, &frame.key(&format!("shortcut{i}")), &label, tx + 2.0, ty + 4.0 + 2.0 * WELL_R + 6.0 + (16.0 - lh) / 2.0,
            tw - 4.0, 0.5, Font::Footnote, colour, alpha * if colour == (1.0, 1.0, 1.0) { 0.85 } else { 1.0 });
    });
}

/// "Now playing": the art, the title over the artist, and previous /
/// play-pause / next under them; a new track's title rises in, a pressed
/// button dips.
fn draw_media(cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, frame: &Frame, media: &Media) {
    if alpha <= 0.002 {
        return;
    }
    let (mx, my, mw, mh) = offset(music_card(), x, y);
    card(cr, (mx, my, mw, mh), CARD_R, alpha);
    let (ax, ay) = (mx + 12.0, my + (mh - ART) / 2.0);
    crate::player::art_tile(cr, renderer, frame.art, &media.art_url, ax, ay, ART, 14.0, alpha);
    let tx = ax + ART + 14.0;
    let tw = mx + mw - 12.0 - tx;
    let title_h = menu_style::line_height(cr, renderer, Font::Headline);
    let top = my + 14.0;
    let second = if media.artist.is_empty() { media.app.clone() } else { media.artist.clone() };
    crate::motion::text(cr, renderer, &frame.key("media-title"), &media.title, tx, top, tw, 0.0, Font::Headline,
        (1.0, 1.0, 1.0), alpha);
    crate::motion::text(cr, renderer, &frame.key("media-artist"), &second, tx, top + title_h, tw, 0.0, Font::Footnote,
        (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
    for (i, action) in MEDIA_ACTIONS.iter().enumerate() {
        let (cx, cy) = media_button(i);
        let (cx, cy) = (x + cx, y + cy);
        let lit = (frame.hover)(*action).clamp(0.0, 1.0);
        if *action == Action::MediaClear {
            // quieter than the transport: a small plate that is always there
            let press = menu_style::press_scale((frame.pressed)(*action));
            menu_style::well(cr, cx, cy, (11.0 + 3.0 * lit) * press, None, (0.35 + 0.45 * lit) * alpha);
            crate::player::control_glyph(cr, crate::player::Control::Clear, media.playing, cx, cy, 0.9 * press,
                alpha * (0.55 + 0.4 * lit));
            continue;
        }
        if lit > 0.0 {
            menu_style::well(cr, cx, cy, MEDIA_BUTTON / 2.0, None, 0.8 * lit * alpha);
        }
        let control = match action {
            Action::MediaPrevious => crate::player::Control::Previous,
            Action::MediaPlayPause => crate::player::Control::PlayPause,
            _ => crate::player::Control::Next,
        };
        let size = if control == crate::player::Control::PlayPause { 0.74 } else { 0.58 };
        let press = menu_style::press_scale((frame.pressed)(*action));
        crate::player::control_glyph(cr, control, media.playing, cx, cy, size * press, alpha * (0.85 + 0.15 * lit));
    }
}

/// A thin slider for a list row (no symbol inside); grows when held.
fn row_slider(cr: &Context, x: f64, y: f64, w: f64, value: f64, alpha: f64, grow: f64, colour: (f64, f64, f64)) {
    row_slider_sized(cr, x, y, w, ROW_SLIDER_H, value, alpha, grow, colour);
}

#[allow(clippy::too_many_arguments)]
fn row_slider_sized(cr: &Context, x: f64, y: f64, w: f64, h: f64, value: f64, alpha: f64, grow: f64, colour: (f64, f64, f64)) {
    let swell = 4.0 * grow.clamp(0.0, 1.2);
    let (x, y, w, h) = (x - swell * 0.5, y - swell * 0.5, w + swell, h + swell);
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
        for (i, action) in SHORTCUTS.iter().enumerate() {
            let (x, y, w, h) = shortcut(i);
            assert!(x >= RAIL - 0.01 && x + w <= RAIL + INNER + 0.01, "tile {i} inside the page");
            assert_eq!(hit(View::Main, &c, x + w / 2.0, y + h / 2.0), Some(Hit::Action(*action)));
        }
        let (bx, by, bw, bh) = outputs_button(&c);
        assert_eq!(hit(View::Main, &c, bx + bw - 10.0, by + bh / 2.0), Some(Hit::Action(Action::Outputs)));
        for media in [false, true] {
            let mut c = c.clone();
            if media { c.media = Some(Media { title: "Says".into(), playing: true, ..Media::default() }); }
            for slider in [Slider::Volume, Slider::Brightness] {
                let (sx, sy, sw, sh) = level(slider, media).slider;
                assert_eq!(hit(View::Main, &c, sx, sy + sh / 2.0), Some(Hit::Slider(slider, 0.0)));
                assert_eq!(hit(View::Main, &c, sx + sw, sy + sh / 2.0), Some(Hit::Slider(slider, 1.0)));
            }
            assert_eq!(height(View::Main, &c), HEIGHT);
        }
        let (cx, cy, cw, ch) = close_button();
        assert_eq!(hit(View::Main, &c, cx + cw / 2.0, cy + ch / 2.0), Some(Hit::Action(Action::Close)));
        // with something playing, the music card's buttons answer
        let mut playing = c.clone();
        playing.media = Some(Media { title: "Says".into(), playing: true, ..Media::default() });
        for (i, action) in MEDIA_ACTIONS.iter().enumerate() {
            let (cx, cy) = media_button(i);
            assert_eq!(hit(View::Main, &playing, cx, cy), Some(Hit::Action(*action)));
        }
        assert_eq!(hit(View::Main, &c, WIDTH / 2.0, HEIGHT - 4.0), None);
        let page = sound_page(&c);
        assert_eq!(hit(View::Sound, &c, 200.0, page.sinks + SINK_H + 10.0), Some(Hit::Action(Action::Sink(1))));
        assert_eq!(hit(View::Sound, &c, 40.0, HEADER_TOP + HEADER_H / 2.0), Some(Hit::Action(Action::Back)));
        assert_eq!(hit(View::Sound, &c, ROW_SLIDER_X + ROW_SLIDER_W, page.apps + 40.0), Some(Hit::Slider(Slider::App(0), 1.0)));
        assert_eq!(hit(View::Sound, &c, 40.0, page.mic + 20.0), Some(Hit::Action(Action::MicMute)));
        assert_eq!(hit(View::Sound, &c, ROW_SLIDER_X, page.mic + 30.0), Some(Hit::Slider(Slider::Mic, 0.0)));
        assert_eq!(page.height, height(View::Sound, &c));
        let without = Controls { volume: None, ..controls() };
        let (sx, sy, _, sh) = level(Slider::Volume, false).slider;
        assert_eq!(hit(View::Main, &without, sx + 50.0, sy + sh / 2.0), None);
    }
}
