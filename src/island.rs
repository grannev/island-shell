//! The island itself: its state, the animated quantities derived from it and
//! the drawing. Everything here is in logical pixels; `Renderer` maps them to
//! the output's pixels through the cairo scale.

use std::cell::RefCell;
use std::collections::HashMap;

use cairo::Context;

use crate::anim::{Feel, Morph, Quintic, Spring, Time, inv_min_jerk, min_jerk};
use crate::activity;
use crate::icons;
use crate::art::Art;
use crate::editor;
use crate::launcher::{self, Launcher};
use crate::notice_center;
use crate::nvim::{self, Editor};
use crate::player;
use crate::quick;
use crate::timer_page;
use crate::floating;
use crate::tasks;
use crate::calendar::{self, Calendar};
use crate::touchid;

/// Height of the black box, logical px.
pub const BOX_H: f64 = 32.0;
/// Bottom corner radius at rest / when fully retracted into the edge.
const RADIUS: f64 = 14.0;
const RADIUS_HIDDEN: f64 = 22.0;
/// The concave fillets where the box meets the top edge of the screen.
pub const FILLET: f64 = 14.0;
/// How strongly a new notification ripples the frame's edge.
const SPLASH_NOTICE: f64 = 0.45;
const PAD: f64 = 16.0;
const PAD_HOVER: f64 = 34.0;
/// Extra horizontal padding when the box has fully sunk into the edge ("drop
/// meeting water" spread).
const SPREAD: f64 = 70.0;
/// How far the island moves left to make room for the ⋯ drawer.
pub const DODGE: f64 = 240.0;

pub const FONT: &str = "SF Pro Text Semi-Bold";
pub const FONT_PX: f64 = 16.0;
const CAPTION_PX: f64 = 15.33;
const FONT_REGULAR: &str = "SF Pro Text";
const SMALL_PX: f64 = 12.5;
const TINY_PX: f64 = 11.0;
const QUERY_PX: f64 = 21.0;

/// Silent playback (paused / muted) is shown this long before the title
/// slides away.
const SILENT_AFTER: f64 = 15.0;
/// Equaliser bands (cava's output width).
pub const EQ_BANDS: usize = crate::eq::BANDS;
/// easing of the bars towards the latest levels / down to flat when paused
const EQ_TAU: f64 = 0.045;
const EQ_TAU_SINK: f64 = 0.25;
/// Touch ID pill.
const PILL_H: f64 = 92.0;
const PILL_MIN_W: f64 = 212.0;
/// The box changes shape as Vorssaint's island does: growing, it swings a
/// little past its size (never more than SWING px) and settles back;
/// shrinking, it settles without a swing. Width and height keep their own
/// feel, and the opening itself (line → pill) moves with the width's.
const GROW_W: Feel = Feel { duration: 0.44, bounce: 0.25 };
const GROW_H: Feel = Feel { duration: 0.38, bounce: 0.22 };
const SHRINK_W: Feel = Feel { duration: 0.30, bounce: 0.0 };
const SHRINK_H: Feel = Feel { duration: 0.26, bounce: 0.0 };
const SWING: f64 = 12.0;
/// progress at which the line has dissolved and the pill content starts
const PILL_SWAP: f64 = 0.37;
const PILL_CLIMB: f64 = 30.0;
/// the size the pill's content arrives from
const ARRIVE_SCALE: f64 = 0.86;
const SHEET_SWAP: f64 = 0.30;
const PILL_GAP: f64 = 2.0;
const PULSE_PERIOD: f64 = 0.96;
/// the result: ridges → ring (0.45 s), then the mark draws on (0.3 s)
const RING_DUR: f64 = 0.45;
const MARK_DUR: f64 = 0.3;
const MARK_DELAY: f64 = 0.32;
const AUTH_OK_HOLD: f64 = 1.5;
const AUTH_FAIL_HOLD: f64 = 1.9;
const ICON_SIZE: f64 = 48.0;
/// Notifications: the pill is wider, the content sits on two lines beside
/// the app's icon, and it holds for 3.5 s (restarted per notification).
const NOTICE_HOLD: f64 = 3.5;
const NOTICE_MIN_W: f64 = 420.0;
/// the message wraps to at most two lines inside this column
const NOTICE_MAX_TEXT: f64 = 420.0;
const NOTICE_MAX_LINES: i32 = 2;
/// space above and below the content inside the pill
const NOTICE_PAD_V: f64 = 26.0;
const NOTICE_PAD_L: f64 = 22.0;
const NOTICE_PAD_R: f64 = 24.0;
const NOTICE_ICON: f64 = 40.0;
const NOTICE_GAP: f64 = 16.0;
const NOTICE_LINE_GAP: f64 = 2.0;
/// room under the message for a progress bar
const NOTICE_BAR: f64 = 14.0;
/// the roll when one notification replaces another
const ROLL_DUR: f64 = 0.44;
/// how far the content travels in the roll
const ROLL_TRAVEL: f64 = 46.0;
/// the launcher panel unfolding out of the island
const PANEL_DUR: f64 = 0.32;
/// the editor's chip sliding out beside the clock
const CHIP_DUR: f64 = 0.3;

const HIDE_DUR: f64 = 0.25;
const REVEAL_DUR: f64 = 0.3;
/// the island opening into the player, and the toast's life
const PLAYER_DUR: f64 = 0.34;
const PLAYER_R: f64 = 28.0;
/// how often a drag tells the player where it has got to
const SCRUB_INTERVAL: f64 = 0.25;
/// the progress line's swell and the controls' glow (response, damping);
/// both overshoot a little, so grabbing the playhead has a spring in it
const TRACK_SPRING: (f64, f64) = (0.3, 0.62);
const CONTROL_SPRING: (f64, f64) = (0.26, 0.7);
/// how far the line swells under the pointer, and in the hand
const TRACK_HOVER: f64 = 0.45;
const TOAST_HOLD: f64 = 4.0;
/// the gap between the clock and the compact bars
const BARS_GAP: f64 = 12.0;
/// The floating buttons' spring out (Vorssaint: 0.38 s, bounce 0.12) and
/// their quick way back in.
const FLOAT_RESPONSE: f64 = 0.38;
const FLOAT_DAMPING: f64 = 0.88;
const FLOAT_RETURN: f64 = 0.2;
const FLOAT_DELAY: f64 = 0.16;
/// Tugging the closed island: how far it gives (px) down, sideways and up
/// before the rubber band stops it — a tight one — and Apple's rubber-band
/// constant.
const TUG_DOWN: f64 = 34.0;
const TUG_SIDE: f64 = 26.0;
const TUG_UP: f64 = 5.0;
const TUG_RUBBER: f64 = 0.55;
/// The pointer has to move this far before a press becomes a tug.
const TUG_SLOP: f64 = 6.0;
/// Let go, it jumps back like jelly.
const TUG_RESPONSE: f64 = 0.46;
const TUG_DAMPING: f64 = 0.42;
/// The light round the frame waits this long for a pill to open.
const PILL_LIGHT_DELAY: f64 = 0.25;

/// How much thicker the frame is while Focus (Do Not Disturb) is on.
const FOCUS_FRAME: f64 = 4.0;

/// How far a downward tug pulls the frame's top edge down with it.
const TUG_SAG: f64 = 0.22;

/// How much of a sideways tug is a lean (the top lagging behind the
/// bottom) rather than a slide along the edge.
const TUG_LEAN: f64 = 0.4;

/// Below this the buttons are tucked wholly behind the page's edge.
const FLOAT_TUCKED: f64 = 0.085;
/// The narrowest the closed island's content gets (a bare resting capsule
/// is this plus the padding: 150 px).
const REST_CONTENT_W: f64 = 118.0;
const BARS_W: f64 = 26.0;
const BARS_H: f64 = 14.0;
/// Without the clock the closed island lays its activities out as
/// Vorssaint's wings: the song's cover and the timer's ring at the left end,
/// the bars and the timer's reading at the right, and the song's title in
/// the middle where the clock was. Around a title the shorter wing evens up
/// with the wider one (within WING_BALANCE), so the title stays centred.
const COVER: f64 = 20.0;
const COVER_R: f64 = 6.0;
const COVER_GAP: f64 = 10.0;
/// the timer's ring and its gap, when it moves to the left wing
const RING_LEN: f64 = 30.0;
/// the middle at its widest, while a song names itself there
const MIDDLE_MAX: f64 = 196.0;
/// the most room the shorter wing leaves to keep the middle centred
const WING_BALANCE: f64 = 64.0;
const DODGE_DUR: f64 = 0.25;
/// Hover: a spring with a little overshoot, like the CSS
/// cubic-bezier(0.2, 0.9, 0.3, 1.25) it replaces (peak ~250 ms in).
const HOVER_RESPONSE: f64 = 0.4;
const HOVER_DAMPING: f64 = 0.6;
fn slider_of(kind: activity::SliderKind) -> quick::Slider {
    match kind {
        activity::SliderKind::Brightness => quick::Slider::Brightness,
        activity::SliderKind::Volume => quick::Slider::Volume,
    }
}

/// how quickly a Control Center slider follows its value
const SLIDER_SPRING: f64 = 0.16;
/// the window for the logout button's confirming second press
const LOGOUT_CONFIRM: f64 = 3.0;

/// What the closed island shows at rest (`compact = clock music …`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compact {
    pub clock: bool,
    pub editor: bool,
    pub timer: bool,
    pub tasks: bool,
    pub record: bool,
    pub music: bool,
    pub unread: bool,
    pub dnd: bool,
    pub mic: bool,
}

impl Compact {
    pub const NAMES: [&'static str; 9] = ["clock", "editor", "timer", "tasks", "record", "music", "unread", "dnd", "mic"];
    pub const ALL: Compact = Compact {
        clock: true, editor: true, timer: true, tasks: true, record: true, music: true, unread: true, dnd: true, mic: true,
    };
    pub const NONE: Compact = Compact {
        clock: false, editor: false, timer: false, tasks: false, record: false, music: false, unread: false, dnd: false, mic: false,
    };

    fn slot(&mut self, name: &str) -> Option<&mut bool> {
        Some(match name {
            "clock" => &mut self.clock,
            "editor" | "nvim" => &mut self.editor,
            "timer" => &mut self.timer,
            "tasks" | "jobs" => &mut self.tasks,
            "record" | "recording" => &mut self.record,
            "music" | "bars" => &mut self.music,
            "unread" => &mut self.unread,
            "dnd" => &mut self.dnd,
            "mic" => &mut self.mic,
            _ => return None,
        })
    }

    pub fn get(&self, name: &str) -> Option<bool> {
        let mut copy = *self;
        copy.slot(name).map(|v| *v)
    }

    /// Turn one item on or off.
    pub fn set(&mut self, name: &str, on: bool) -> Result<(), String> {
        *self.slot(name).ok_or_else(|| format!("unknown item {name:?} (known: {})", Self::NAMES.join(", ")))? = on;
        Ok(())
    }

    /// "all", "none" or the items, separated by spaces or commas.
    pub fn parse(value: &str) -> Result<Compact, String> {
        let words: Vec<&str> = value.split([',', ' ']).filter(|w| !w.is_empty()).collect();
        match words.as_slice() {
            ["all"] => return Ok(Self::ALL),
            ["none"] | [] => return Ok(Self::NONE),
            _ => {}
        }
        let mut compact = Self::NONE;
        for word in words {
            compact.set(word, true)?;
        }
        Ok(compact)
    }

    pub fn describe(&self) -> String {
        if *self == Self::ALL { return "all".into(); }
        let names: Vec<&str> = Self::NAMES.iter().copied().filter(|n| self.get(n) == Some(true)).collect();
        if names.is_empty() { "none".into() } else { names.join(",") }
    }
}

/// Tunables (config file / `island-shell set`).
#[derive(Clone, Debug)]
pub struct Config {
    /// Corner smoothing 0..1 (0 = circular arcs, 0.6 ≈ Apple's continuous
    /// curvature). Limited by the room on the shorter edge of each corner.
    pub smoothing: f64,
    /// Velocity-dependent stretch of the box, 0 = rigid, 1 = the default
    /// "liquid" amount.
    pub liquid: f64,
    /// React to the pointer over the box (padding 16 → 34 on a spring).
    pub hover: bool,
    /// The monitor the island lives on: "auto" (the built-in panel, else
    /// the first output), "focused" (follows sway's focused output) or a
    /// connector name such as "eDP-1" (auto while it is unplugged).
    pub output: String,
    /// The clock, as a strftime(3) format.
    pub clock: String,
    /// The equaliser, the progress line and the "now playing" label.
    pub accent: (f64, f64, f64),
    /// Seconds a notification stays up.
    pub notice_hold: f64,
    /// Seconds the "now playing" toast stays up.
    pub toast_hold: f64,
    /// Seconds of silence (paused, muted, 0 %) before the bars fold away.
    pub silent_after: f64,
    /// How far the island moves left for the bar's ⋯ drawer, px.
    pub dodge: f64,
    /// Night mode's colour temperature, K.
    pub night_temp: u32,
    /// Where the calendar's weather is for: auto (the time zone's city),
    /// a city, "lat,lon", or off.
    pub weather: String,
    /// What the closed island shows at rest.
    pub compact: Compact,
    /// The timer ruler's ticks and the alarm when a countdown ends.
    pub timer_sound: bool,
    /// Radius of the screen's own corners, logical px (0 = square): the
    /// island's black runs on past the screen's edges and rounds them off
    /// like the lid around them.
    pub screen_corners: f64,
    /// Light along the screen's edges for notifications and Touch ID.
    pub edge_light: bool,
    /// Music lights the bottom edge in the cover's colours, with the bass
    /// (it redraws the edges while music plays).
    pub ambilight: bool,
    /// The bar (Waybar's elements) in drops of the frame in the top corners.
    pub bar: bool,
}

/// The X1 Titanium's lid corners, measured off a photo: ~55 photo px where
/// the 26 px bar is ~95.
pub const SCREEN_CORNERS: f64 = 16.0;

pub const CLOCK_FORMAT: &str = "%a %b %-d  %-I:%M %p";

impl Default for Config {
    fn default() -> Self {
        Config {
            smoothing: 0.6,
            liquid: 1.0,
            hover: true,
            output: "auto".into(),
            clock: CLOCK_FORMAT.into(),
            accent: crate::player::EQ_COLOR,
            notice_hold: NOTICE_HOLD,
            toast_hold: TOAST_HOLD,
            silent_after: SILENT_AFTER,
            dodge: DODGE,
            night_temp: 4500,
            weather: "auto".into(),
            compact: Compact::ALL,
            timer_sound: true,
            screen_corners: SCREEN_CORNERS,
            edge_light: true,
            ambilight: false,
            bar: false,
        }
    }
}

impl Config {
    pub const KEYS: [&'static str; 18] = [
        "smoothing", "liquid", "hover", "output", "clock", "accent",
        "notice_hold", "toast_hold", "silent_after", "dodge", "night_temp", "weather", "compact", "timer_sound",
        "screen_corners", "edge_light", "ambilight", "bar",
    ];

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        let number = || -> Result<f64, String> {
            match value {
                "on" | "true" | "yes" => Ok(1.0),
                "off" | "false" | "no" => Ok(0.0),
                v => v.trim_end_matches('s').parse().map_err(|_| format!("bad value {value:?} for {key}")),
            }
        };
        match key {
            "smoothing" => self.smoothing = number()?.clamp(0.0, 1.0),
            "liquid" => self.liquid = number()?.clamp(0.0, 4.0),
            "hover" => self.hover = number()? != 0.0,
            "output" => {
                if value.is_empty() || value.contains(char::is_whitespace) {
                    return Err(format!("bad output {value:?}: auto, focused or a name like eDP-1"));
                }
                self.output = value.into();
            }
            "clock" => {
                let value = value.trim_matches('"');
                if strftime(value).trim().is_empty() {
                    return Err(format!("clock format {value:?} gives an empty clock"));
                }
                self.clock = value.into();
            }
            "accent" => self.accent = parse_colour(value).ok_or_else(|| format!("bad colour {value:?}: #RRGGBB"))?,
            "notice_hold" => self.notice_hold = number()?.clamp(1.0, 60.0),
            "toast_hold" => self.toast_hold = number()?.clamp(1.0, 30.0),
            "silent_after" => self.silent_after = number()?.clamp(1.0, 3600.0),
            "dodge" => self.dodge = number()?.clamp(0.0, 1000.0),
            "night_temp" => {
                let kelvin: f64 = value.trim_end_matches(['K', 'k']).parse().map_err(|_| format!("bad value {value:?} for {key}"))?;
                self.night_temp = kelvin.clamp(1500.0, 6500.0) as u32;
            }
            "weather" => {
                let value = value.trim_matches('"');
                if value.is_empty() { return Err("weather: auto, off, a city or lat,lon".into()); }
                self.weather = value.into();
                crate::weather::configure(value);
            }
            "compact" => self.compact = Compact::parse(value.trim_matches('"'))?,
            "timer_sound" => self.timer_sound = number()? != 0.0,
            "screen_corners" => self.screen_corners = number()?.clamp(0.0, 80.0),
            "edge_light" => self.edge_light = number()? != 0.0,
            "ambilight" => self.ambilight = number()? != 0.0,
            "bar" => self.bar = number()? != 0.0,
            _ => return Err(format!("unknown setting {key:?} (known: {})", Self::KEYS.join(", "))),
        }
        Ok(())
    }

    pub fn describe(&self) -> String {
        let (r, g, b) = self.accent;
        let hex = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!(
            "smoothing={} liquid={} hover={} output={} clock={:?} accent=#{:02X}{:02X}{:02X} \
             notice_hold={} toast_hold={} silent_after={} dodge={} night_temp={} weather={:?} compact={} timer_sound={} screen_corners={} edge_light={} ambilight={} bar={}",
            self.smoothing, self.liquid, self.hover as u8, self.output, self.clock,
            hex(r), hex(g), hex(b), self.notice_hold, self.toast_hold, self.silent_after, self.dodge, self.night_temp, self.weather,
            self.compact.describe(), self.timer_sound as u8, self.screen_corners, self.edge_light as u8, self.ambilight as u8, self.bar as u8,
        )
    }

    /// Does the clock show seconds (and so need a tick every second)?
    pub fn clock_has_seconds(&self) -> bool {
        ["%S", "%T", "%r", "%s", "%X", "%c", "%+"].iter().any(|spec| self.clock.contains(spec))
    }
}

/// `#RRGGBB` / `RRGGBB` → 0..1 components.
fn parse_colour(value: &str) -> Option<(f64, f64, f64)> {
    let hex = value.trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// Why the island is out of sight; several may hold at once and it comes back
/// only when all are cleared.
#[derive(Clone, Copy, Debug, Default)]
pub struct HideReasons {
    /// mod+i
    pub user: bool,
    /// a fullscreen window has the focus (sway IPC)
    pub fullscreen: bool,
    /// Explicit show while a fullscreen window remains focused.
    pub fullscreen_override: bool,
}

impl HideReasons {
    fn any(self) -> bool {
        self.user || (self.fullscreen && !self.fullscreen_override)
    }
}

/// The track the island knows about, and where the player is in it (the
/// position is asked for now and then and runs on by itself in between).
#[derive(Clone, Debug)]
pub struct Song {
    pub now: crate::dbus::NowPlaying,
    position: f64,
    position_at: Time,
}

impl Song {
    fn new(np: &crate::dbus::NowPlaying, now: Time) -> Song {
        Song { now: np.clone(), position: 0.0, position_at: now }
    }

    pub fn playing(&self) -> bool {
        self.now.playing
    }

    /// Is this the same track, or has the player moved on?
    fn same_track(&self, np: &crate::dbus::NowPlaying) -> bool {
        self.now.title == np.title && self.now.artist == np.artist
    }

    pub fn position(&self, now: Time) -> f64 {
        let elapsed = if self.now.playing { now - self.position_at } else { 0.0 };
        let position = self.position + elapsed;
        if self.now.length > 0.0 { position.min(self.now.length) } else { position }
    }

    fn set_position(&mut self, position: f64, now: Time) {
        self.position = position;
        self.position_at = now;
    }

    fn view<'a>(&'a self, eq: &'a [f64], accent: (f64, f64, f64), pressed: Option<(player::Control, Time)>, scrub: Option<f64>, now: Time) -> player::View<'a> {
        player::View {
            accent,
            title: &self.now.title,
            artist: &self.now.artist,
            app: &self.now.app,
            art_url: &self.now.art_url,
            playing: self.now.playing,
            eq,
            // while the playhead is being dragged the line shows where the
            // pointer is, not where the player still is
            position: match scrub {
                Some(fraction) => fraction * self.now.length,
                None => self.position(now),
            },
            length: self.now.length,
            pressed,
            track_grow: 0.0,
            control_hover: [0.0; 4],
        }
    }
}

/// A notification on screen, with the instant it arrived (its icon's
/// entrance and the countdown ring run from there).
#[derive(Clone, Debug)]
pub struct Notice {
    pub note: crate::notify::Notification,
    pub since: Time,
}

/// A notification being swiped sideways (by two fingers or the mouse).
struct Swipe {
    /// the content's horizontal offset: set directly while held, a spring
    /// back to rest once let go
    offset: Spring,
    held: bool,
    /// fed by a touchpad / tilt wheel rather than a held button
    axis: bool,
    last: Time,
    /// px/s, smoothed, for a flick
    velocity: f64,
    /// a mouse press that has not moved far enough to be a swipe yet
    press: Option<f64>,
}

impl Swipe {
    fn new(now: Time) -> Self {
        Swipe { offset: Spring::new(0.0, 0.35, 0.82), held: true, axis: false, last: now, velocity: 0.0, press: None }
    }

    /// Follow the finger to `offset`.
    fn to(&mut self, offset: f64, now: Time) {
        let dt = now - self.last;
        let old = self.offset.value(now);
        if dt > 1e-4 {
            self.velocity = 0.6 * self.velocity + 0.4 * (offset - old) / dt;
        }
        self.offset = Spring::new(offset, 0.35, 0.82);
        self.last = now;
    }
}

/// What letting go of a notification gesture did.
#[derive(Clone, Debug, PartialEq)]
pub enum NoticeRelease {
    /// it never became a swipe: a click
    Click(NoticeClick),
    /// it was swiped away (the D-Bus ID to close, 0 for internal ones)
    Dismissed(u32),
    None,
}

/// How far (of the pill's width) or how fast a swipe must go to dismiss.
const SWIPE_DISMISS: f64 = 0.3;
const SWIPE_FLICK: f64 = 700.0;
/// a touchpad that stops sending without an explicit stop has let go
const SWIPE_IDLE: Time = 0.15;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeClick {
    Invoke(u32, String),
    Dismiss(u32),
    /// the reply field's send button
    SendReply,
    /// the rest of a pill being replied to: nothing to do
    Field,
}

/// A reply being typed into a notification (the app's inline-reply).
struct Reply {
    id: u32,
    text: String,
    since: Time,
    typed: Time,
}

/// A screen recording's course.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Recording {
    /// 3-2-1 in the pill (since)
    Countdown(Time),
    /// the pill folds away before capture starts (start at)
    Starting(Time),
    /// asked the recorder to start
    Launching,
    /// recording since, as this process
    Running(Time, u32),
    /// asked it to stop (it finishes the file)
    Stopping(Time),
}

/// What a key did to a reply being typed.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplyKey {
    None,
    /// send this text to the notification with this ID
    Send(u32, String),
    Cancel,
}

/// the reply field's band under the message, and the pill's width then
const REPLY_H: f64 = 52.0;
const NOTICE_REPLY_W: f64 = 460.0;

#[derive(Clone, Debug)]
struct LiveActivity {
    event: activity::Event,
    since: Time,
    updated_at: Time,
}

/// The running timer: a countdown, a Pomodoro phase or the stopwatch.
#[derive(Clone, Debug)]
struct Countdown {
    duration: f64,
    /// the time left as of `changed_at` (gone, for the stopwatch)
    remaining: f64,
    changed_at: Time,
    running: bool,
    kind: timer_page::Kind,
}

impl Countdown {
    fn new(duration: f64, now: Time) -> Self {
        Countdown::of(timer_page::Kind::Timer, duration, now)
    }

    fn of(kind: timer_page::Kind, duration: f64, now: Time) -> Self {
        let remaining = if kind == timer_page::Kind::Stopwatch { 0.0 } else { duration };
        Countdown { duration, remaining, changed_at: now, running: true, kind }
    }

    fn counts_up(&self) -> bool {
        self.kind == timer_page::Kind::Stopwatch
    }

    /// What it shows: the time left, or the stopwatch's time gone.
    fn reading(&self, now: Time) -> f64 {
        let passed = if self.running { now - self.changed_at } else { 0.0 };
        if self.counts_up() { self.remaining + passed } else { (self.remaining - passed).max(0.0) }
    }

    /// The ring: the share left, or the stopwatch's sweep through a minute.
    fn fraction(&self, now: Time) -> f64 {
        if self.counts_up() {
            (self.reading(now) % 60.0) / 60.0
        } else if self.duration <= 0.0 {
            0.0
        } else {
            self.reading(now) / self.duration
        }
    }

    /// A countdown's last minute.
    fn urgent(&self, now: Time) -> bool {
        !self.counts_up() && self.reading(now) <= 60.0
    }

    fn finished(&self, now: Time) -> bool {
        !self.counts_up() && self.running && self.reading(now) <= 0.0
    }

    fn pause(&mut self, now: Time) {
        self.remaining = self.reading(now);
        self.changed_at = now;
        self.running = false;
    }

    fn resume(&mut self, now: Time) {
        if self.counts_up() || self.remaining > 0.0 {
            self.changed_at = now;
            self.running = true;
        }
    }
}

/// A Pomodoro cycle: its plan and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cycle {
    plan: timer_page::Pomodoro,
    phase: timer_page::Phase,
    session: u32,
    /// the phase ran out: what waits to start (None: the cycle is over)
    waiting: Option<Option<(timer_page::Phase, u32)>>,
}

/// Touch ID: what the pill shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Auth {
    /// waiting for a finger (pulsing) since
    Waiting(Time),
    /// matched: green tick since, closes after `AUTH_OK_HOLD`
    Ok(Time),
    /// no match: red cross since, closes after `AUTH_FAIL_HOLD` unless a
    /// new attempt starts
    Fail(Time),
}

pub struct Island {
    /// 0 = fully shown, 1 = parked above the edge.
    hide: Quintic,
    /// 0 = centred, 1 = dodged 240 px left.
    dodge: Quintic,
    /// Pointer over the box: the padding springs (no tint — the box stays
    /// pure black).
    hover_pad: Spring,
    /// Now playing: the compact bars beside the clock (0 = clock alone),
    /// the toast that announces a track, and the player the island opens
    /// into while the pointer rests on it.
    reveal: Quintic,
    song: Option<Song>,
    toast: Option<Time>,
    player: Quintic,
    player_opened: Time,
    pressed: Option<(player::Control, Time)>,
    hovered_control: Option<player::Control>,
    /// the playhead is being dragged: where it is, and when the player was
    /// last told about it (the drag is not worth a D-Bus call per frame)
    scrub: Option<(f64, Time)>,
    /// the progress line swelling under the pointer / in the hand, and the
    /// three transport controls lighting up — all on springs
    track_grow: Spring,
    control_hover: [Spring; 4],
    art: Art,
    /// equaliser bands: the latest levels from cava and the eased heights
    eq_target: [f64; EQ_BANDS],
    eq: [f64; EQ_BANDS],
    eq_t: Time,
    /// playback nobody hears (paused, or the sink muted / at 0 %) since
    silent_since: Option<Time>,
    pub muted: bool,
    /// Touch ID: 0 = the line, 1 = the 92 px pill.
    pill: Morph,
    auth: Option<Auth>,
    /// when the current verification began (the pulse's time base)
    auth_start: Time,
    /// the last result shown, for undrawing it on a new attempt
    last_mark: Option<(touchid::Mark, (f64, f64, f64))>,
    /// Notifications: the one on screen, the one it replaced (while the
    /// roll runs) and one waiting for Touch ID to finish.
    notice: Option<Notice>,
    notice_prev: Option<Notice>,
    notice_queued: Option<crate::notify::Notification>,
    center: notice_center::Center,
    center_rect: Option<Rect>,
    notice_rect: Option<Rect>,
    notice_hovered: Option<usize>,
    notice_action_hover: [Spring; 2],
    /// the pill's progress, eased between a job's updates
    notice_progress: Spring,
    notice_swipe: Option<Swipe>,
    notice_reply: Option<Reply>,
    /// the pointer is on the notification's pill
    notice_pointer: bool,
    /// a swiped-away notification flying off while the pill folds
    notice_swiped: Option<(Notice, Spring, f64)>,
    /// a swipe that ended on its own (the touchpad went quiet): its
    /// notification for the app to close over D-Bus
    swiped_id: Option<u32>,
    pub dnd: bool,
    dnd_glow: Spring,
    roll: Quintic,
    /// the pill's size, eased when a new notification needs another (a
    /// long message wraps to two lines and the pill grows to fit)
    pill_w: Morph,
    pill_h: Morph,
    /// Short hardware and system activities share the pill but outrank
    /// ordinary notifications.  Repeated slider changes update in place.
    activity: Option<LiveActivity>,
    activity_retiring: Option<LiveActivity>,
    activity_queued: Option<activity::Event>,
    activity_value: Spring,
    activity_art: Art,
    /// A countdown remains compact beside the clock until it completes.
    timer: Option<Countdown>,
    timer_chip: Quintic,
    timer_w: f64,
    timer_rect: Option<Rect>,
    /// the timer's ring when the wings put it at the left end
    timer_mark_rect: Option<Rect>,
    timer_hovered: bool,
    timer_menu_rect: Option<Rect>,
    /// the timer page (Vorssaint's): its choices outlive its openings
    timer_page: timer_page::Page,
    /// a Pomodoro cycle in progress
    cycle: Option<Cycle>,
    /// sounds for the app to play (ruler ticks, the alarm)
    sounds: Vec<crate::sound::Sound>,
    network_menu_rect: Option<Rect>,
    network_panel: Option<crate::connections::Panel>,
    network_session: u64,
    timer_last: Option<Countdown>,
    /// jobs from `island-shell task|progress|run`: a ring beside the clock
    pub tasks: tasks::Tasks,
    tasks_chip: Quintic,
    tasks_w: f64,
    tasks_rect: Option<Rect>,
    tasks_pill_rect: Option<Rect>,
    /// the month sheet (a right click on the clock) and where it is
    calendar: Option<Calendar>,
    calendar_rect: Option<Rect>,
    clock_rect: Option<Rect>,
    /// who records from a microphone now: an orange dot at the line's end
    mic_users: Vec<String>,
    /// night mode's colour on the display module, eased
    night_glow: Spring,
    /// screen recording: its state, whether wf-recorder exists, the red
    /// chip beside the clock and its hit target, and requests for the app
    recording: Option<Recording>,
    pub recorder: bool,
    record_chip: Quintic,
    record_w: f64,
    record_rect: Option<Rect>,
    record_start: bool,
    record_stop: Option<u32>,
    /// when the last recording ran, for its length
    record_last: Option<Time>,
    mic_dot: Spring,
    quick_open: bool,
    quick_rect: Option<Rect>,
    quick_hover: Option<quick::Action>,
    quick_selected: usize,
    quick_hover_springs: HashMap<quick::Action, Spring>,
    /// the Control Center's page, and the page it is crossfading from
    quick_view: quick::View,
    quick_view_from: Option<(quick::View, Time)>,
    /// the environment the Control Center shows (volume, brightness,
    /// outputs, layout), with eased slider positions
    pub controls: quick::Controls,
    /// each slider's eased position
    slider_shown: HashMap<quick::Slider, Spring>,
    /// the slider held by the pointer
    quick_drag: Option<quick::Slider>,
    logout_armed_at: Option<Time>,
    /// when the Control Center opened (its rows arrive in a wave), when
    /// each control was last pressed and when its symbol last bounced
    quick_opened_at: Time,
    quick_pressed: HashMap<quick::Action, Time>,
    quick_bounced: HashMap<quick::Action, Time>,
    /// the sliders swell while held
    slider_grow: HashMap<quick::Slider, Spring>,
    quick_close_hover: Spring,
    quick_retiring_at: Option<Time>,
    /// the notification center was just closed: its content fades with the
    /// fold (or crossfades into the sheet that replaced it)
    center_retiring_at: Option<Time>,
    center_was_open: bool,
    /// the pill was last a sheet (30 px corners) rather than a capsule;
    /// remembered so a closing sheet keeps its corners while it folds
    sheet_corners: bool,
    quick_returned_at: Option<Time>,
    eq_rect: Option<Rect>,
    /// the song's cover at the left wing's end
    cover_rect: Option<Rect>,
    player_rect: Option<Rect>,
    eq_hovered: bool,
    editor_chip_rect: Option<Rect>,
    editor_chip_hovered: bool,
    /// the launcher: the panel's growth and its content
    panel: Quintic,
    pub launcher: Option<Launcher>,
    catalog: crate::apps::Catalog,
    launcher_icons: std::rc::Rc<crate::iconload::Icons>,
    file_search: std::rc::Rc<crate::files::Search>,
    clip_search: std::rc::Rc<crate::clipboard::Search>,
    /// Neovim (the companion plugin): the editor's state, the chip that
    /// shows it beside the clock, and the panel it opens into
    nvim: Option<Editor>,
    nvim_seen: Time,
    /// when the editor last wrote a file: the chip flashes for a moment
    nvim_saved: Option<Time>,
    chip: Quintic,
    chip_w: f64,
    /// Vorssaint's floating buttons beside an open page: how far out they
    /// are, which is lit, which was pressed, and the page they stand beside
    /// once they have arrived (for the pointer).
    floats: Spring,
    /// The closed island in the hand (see `tug_press`), and the springs
    /// that bring it back once let go.
    tug: Option<Tug>,
    /// light along the screen's edges (see `edge`)
    edge: crate::edge::Edge,
    /// the screen switching off into the island (see `curtain`)
    curtain: crate::curtain::Curtain,
    /// the volume / brightness level sliding out of the left edge
    side_hud: crate::side_hud::SideHud,
    /// Waybar's elements in the top corners' drops
    pub bar: crate::bar::Bar,
    /// the battery is low and not charging: the bottom corners smoulder
    low_battery: bool,
    /// the top edge as a membrane (pulled by a tug, rippling)
    membrane: crate::edge::Membrane,
    /// the frame thickened for Focus / Do Not Disturb, 0..1
    focus_frame: Spring,
    /// the surface's centre, as of the last frame drawn
    surface_cx: f64,
    /// the box as last drawn, and its corner radius (None: above the edge)
    box_geom: Option<(Rect, f64)>,
    /// a tugged island's top: its centre, how far either side its fillets
    /// end, and how far the edge is pulled down there
    tug_top: Option<(f64, f64, f64)>,
    tug_x: Spring,
    tug_y: Spring,
    /// A page folding away waits for its floating buttons to tuck in
    /// behind it: the fold starts here.
    fold_at: Time,
    float_lit: HashMap<floating::Button, Spring>,
    float_hover: Option<floating::Button>,
    float_pressed: Option<(floating::Button, Time)>,
    float_body: Option<floating::Body>,
    /// The clock and the bars at rest, as `compact` keeps or removes them.
    clock_chip: Quintic,
    music_chip: Quintic,
    /// The closed line, for a right click when the clock is not on it.
    line_rect: Option<Rect>,
    /// the clock's width, eased so the box follows its digits rolling
    clock_w: Option<Spring>,
    /// the middle between the wings, eased as titles change
    middle_w: Option<Spring>,
    /// the last song's title and cover, kept while its wings fold away
    wing_song: (String, String),
    editor_panel: Quintic,
    editor_opened: Time,
    editor_diagnostics: bool,
    // Keep the command sheet while it folds; clearing input must not reveal the buffer panel.
    editor_command: Option<nvim::Cmdline>,
    editor_output: Option<crate::editor_output::Output>,
    output_open: bool,
    output_view: bool,
    output_until: Time,
    command_seen: Time,
    editor_hover: Option<editor::Action>,
    editor_pressed: Option<(editor::Action, Time)>,
    /// the icon: fingerprint → ring, and the mark drawing itself on it
    ring: Quintic,
    mark: Quintic,
    pub hidden: HideReasons,
    pub dodged: bool,
    pub hovered: bool,
    pub clock: String,
    pub config: Config,
}

/// The closed island held by the pointer (or a finger).
#[derive(Clone, Copy, Debug)]
struct Tug {
    /// where the press landed
    from: (f64, f64),
    /// past the slop: it is a tug, not a click
    engaged: bool,
    /// how far the island gives now, and how fast (px, px/s)
    offset: (f64, f64),
    velocity: (f64, f64),
    at: Time,
}

/// Apple's rubber band: the island follows less the further it is pulled,
/// and never past `limit`.
pub(crate) fn rubber(d: f64, limit: f64) -> f64 {
    d.signum() * (1.0 - 1.0 / (d.abs() * TUG_RUBBER / limit + 1.0)) * limit
}

/// The pull that gives `offset` (to catch the island mid-wobble).
fn rubber_inverse(offset: f64, limit: f64) -> f64 {
    let k = (offset.abs() / limit).min(0.95);
    offset.signum() * limit / TUG_RUBBER * (1.0 / (1.0 - k) - 1.0)
}

/// How far the island gives for a pull of (`dx`, `dy`).
fn tug_offset(dx: f64, dy: f64) -> (f64, f64) {
    (rubber(dx, TUG_SIDE), rubber(dy, if dy > 0.0 { TUG_DOWN } else { TUG_UP }))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn union(self, o: Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect { x, y, w: (self.x + self.w).max(o.x + o.w) - x, h: (self.y + self.h).max(o.y + o.h) - y }
    }
}

/// What one frame drew, for the input / opaque regions and damage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// The whole painted area (box + fillets), `None` when above the edge.
    pub bounds: Option<Rect>,
    /// What takes the pointer: the box as it will be once it has finished
    /// opening, not as it is this frame. Changing the input region under a
    /// resting pointer makes the compositor drop it (an enter followed at
    /// once by a leave), and an island that grows for a third of a second
    /// would do that thirty times on the way.
    pub input: Option<Rect>,
    /// Fully opaque parts of the box.
    pub opaque: [Option<Rect>; 2],
}

/// The timer's colour: orange, green for a Pomodoro's breaks.
fn timer_colour(kind: timer_page::Kind) -> (f64, f64, f64) {
    match kind {
        timer_page::Kind::Pomodoro { phase: timer_page::Phase::ShortBreak | timer_page::Phase::LongBreak, .. } => crate::menu_style::GREEN,
        _ => crate::menu_style::ORANGE,
    }
}

/// Colour helpers.
fn rgb(hex: u32) -> (f64, f64, f64) {
    (((hex >> 16) & 0xff) as f64 / 255.0, ((hex >> 8) & 0xff) as f64 / 255.0, (hex & 0xff) as f64 / 255.0)
}

fn mix(a: (f64, f64, f64), b: (f64, f64, f64), k: f64) -> (f64, f64, f64) {
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k)
}

/// How far the pill's content has faded in, from the morph's progress.
fn pill_alpha_of(q: f64) -> f64 {
    smoothstep((q - PILL_SWAP) / (1.0 - PILL_SWAP))
}

pub fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Island {
    pub fn new(config: Config) -> Self {
        let dnd = notice_center::load_dnd();
        crate::weather::configure(&config.weather);
        Island {
            hide: Quintic::at_rest(0.0, HIDE_DUR),
            dodge: Quintic::at_rest(0.0, DODGE_DUR),
            hover_pad: Spring::new(0.0, HOVER_RESPONSE, HOVER_DAMPING),
            reveal: Quintic::at_rest(0.0, REVEAL_DUR),
            song: None,
            toast: None,
            player: Quintic::at_rest(0.0, PLAYER_DUR),
            player_opened: 0.0,
            pressed: None,
            hovered_control: None,
            scrub: None,
            track_grow: Spring::new(0.0, TRACK_SPRING.0, TRACK_SPRING.1),
            control_hover: std::array::from_fn(|_| Spring::new(0.0, CONTROL_SPRING.0, CONTROL_SPRING.1)),
            art: Art::new(),
            eq_target: [0.0; EQ_BANDS],
            eq: [0.0; EQ_BANDS],
            eq_t: 0.0,
            silent_since: None,
            muted: false,
            pill: Morph::new(0.0, GROW_W, SHRINK_W, 0.03, 1e-3),
            auth: None,
            auth_start: 0.0,
            last_mark: None,
            notice: None,
            notice_prev: None,
            notice_queued: None,
            center: notice_center::Center::new(),
            center_rect: None,
            notice_rect: None,
            notice_hovered: None,
            notice_action_hover: std::array::from_fn(|_| Spring::new(0.0, 0.22, 0.85)),
            notice_progress: Spring::new(0.0, 0.35, 1.0),
            notice_swipe: None,
            notice_reply: None,
            notice_pointer: false,
            notice_swiped: None,
            swiped_id: None,
            dnd,
            dnd_glow: Spring::new(f64::from(dnd), 0.32, 0.9),
            roll: Quintic::at_rest(1.0, ROLL_DUR),
            pill_w: Morph::new(PILL_MIN_W, GROW_W, SHRINK_W, SWING, 0.25),
            pill_h: Morph::new(PILL_H, GROW_H, SHRINK_H, SWING, 0.25),
            activity: None,
            activity_retiring: None,
            activity_queued: None,
            activity_value: Spring::new(0.0, 0.28, 0.82),
            activity_art: Art::new(),
            timer: None,
            timer_chip: Quintic::at_rest(0.0, CHIP_DUR),
            timer_w: 0.0,
            timer_rect: None,
            timer_mark_rect: None,
            timer_hovered: false,
            timer_menu_rect: None,
            timer_page: timer_page::Page::new(),
            cycle: None,
            sounds: Vec::new(),
            network_menu_rect: None,
            network_panel: None,
            network_session: 0,
            timer_last: None,
            tasks: tasks::Tasks::default(),
            tasks_chip: Quintic::at_rest(0.0, CHIP_DUR),
            tasks_w: 0.0,
            tasks_rect: None,
            tasks_pill_rect: None,
            calendar: None,
            calendar_rect: None,
            clock_rect: None,
            mic_users: Vec::new(),
            night_glow: Spring::new(0.0, 0.35, 1.0),
            recording: None,
            recorder: crate::recorder::available(),
            record_chip: Quintic::at_rest(0.0, CHIP_DUR),
            record_w: 0.0,
            record_rect: None,
            record_start: false,
            record_stop: None,
            record_last: None,
            // under-damped: the dot swells a hair past full, then settles
            mic_dot: Spring::new(0.0, 0.32, 0.72),
            quick_open: false,
            quick_rect: None,
            quick_hover: None,
            quick_selected: 0,
            quick_hover_springs: HashMap::new(),
            quick_view: quick::View::Main,
            quick_view_from: None,
            controls: quick::Controls::default(),
            slider_shown: HashMap::new(),
            quick_drag: None,
            logout_armed_at: None,
            quick_opened_at: f64::NEG_INFINITY,
            quick_pressed: HashMap::new(),
            quick_bounced: HashMap::new(),
            slider_grow: HashMap::new(),
            quick_close_hover: Spring::new(0.0, 0.24, 0.83),
            quick_retiring_at: None,
            center_retiring_at: None,
            center_was_open: false,
            sheet_corners: false,
            quick_returned_at: None,
            eq_rect: None,
            cover_rect: None,
            player_rect: None,
            eq_hovered: false,
            editor_chip_rect: None,
            editor_chip_hovered: false,
            panel: Quintic::at_rest(0.0, PANEL_DUR),
            launcher: None,
            catalog: crate::apps::Catalog::new(),
            launcher_icons: std::rc::Rc::new(crate::iconload::Icons::new()),
            file_search: std::rc::Rc::new(crate::files::Search::new()),
            clip_search: std::rc::Rc::new(crate::clipboard::Search::new()),
            nvim: None,
            nvim_seen: 0.0,
            nvim_saved: None,
            chip: Quintic::at_rest(0.0, CHIP_DUR),
            chip_w: 0.0,
            floats: Spring::new(0.0, FLOAT_RESPONSE, FLOAT_DAMPING),
            tug: None,
            edge: crate::edge::Edge::default(),
            curtain: crate::curtain::Curtain::default(),
            side_hud: crate::side_hud::SideHud::default(),
            bar: crate::bar::Bar::default(),
            low_battery: false,
            membrane: crate::edge::Membrane::default(),
            focus_frame: Spring::new(0.0, 0.45, 0.8),
            surface_cx: 490.0,
            box_geom: None,
            tug_top: None,
            tug_x: Spring::new(0.0, TUG_RESPONSE, TUG_DAMPING),
            tug_y: Spring::new(0.0, TUG_RESPONSE, TUG_DAMPING),
            fold_at: 0.0,
            float_lit: HashMap::new(),
            float_hover: None,
            float_pressed: None,
            float_body: None,
            clock_chip: Quintic::at_rest(if config.compact.clock { 1.0 } else { 0.0 }, CHIP_DUR),
            music_chip: Quintic::at_rest(if config.compact.music { 1.0 } else { 0.0 }, CHIP_DUR),
            line_rect: None,
            clock_w: None,
            middle_w: None,
            wing_song: (String::new(), String::new()),
            editor_panel: Quintic::at_rest(0.0, PLAYER_DUR),
            editor_diagnostics: false,
            editor_command: None,
            editor_output: None,
            output_open: false,
            output_view: false,
            output_until: 0.0,
            command_seen: 0.0,
            editor_opened: 0.0,
            editor_hover: None,
            editor_pressed: None,
            ring: Quintic::at_rest(0.0, RING_DUR),
            mark: Quintic::at_rest(0.0, MARK_DUR),
            hidden: HideReasons::default(),
            dodged: false,
            hovered: false,
            clock: String::new(),
            config,
        }
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.hide.is_moving(now)
            || self.dodge.is_moving(now)
            || self.hover_pad.is_moving(now)
            || self.reveal.is_moving(now)
            || self.player.is_moving(now)
            || self.pill.is_moving(now)
            // the player's own content keeps moving while it is open
            || self.player.state(now).x > 0.0 && self.song.as_ref().is_some_and(|s| s.playing())
            || self.pressed.is_some_and(|(_, t)| now - t < 0.3)
            || self.editor_pressed.is_some_and(|(_, t)| now - t < 0.3)
            || self.scrub.is_some()
            || self.track_grow.is_moving(now)
            || self.control_hover.iter().any(|s| s.is_moving(now))
            // the small bars beside the clock are redrawn as cava feeds
            // them (~20 fps); the big ones glide with the display
            || self.eq_moving() && (self.pill.state(now).x > 0.0 || self.player.state(now).x > 0.0)
            || self.roll.is_moving(now)
            || self.pill_w.is_moving(now)
            || self.pill_h.is_moving(now)
            || self.activity_value.is_moving(now) && self.activity.is_some()
            || self.timer_menu_open() && self.timer_page.is_animating(now)
            || self.quick_hover_springs.values().any(|s| s.is_moving(now))
            || self.quick_open && self.slider_shown.values().any(|s| s.is_moving(now))
            || self.quick_open && (now - self.quick_opened_at < 0.6
                || self.quick_pressed.values().chain(self.quick_bounced.values()).any(|t| now - t < 1.0)
                || self.slider_grow.values().any(|s| s.is_moving(now)))
            || self.quick_view_from.is_some_and(|(_, since)| now - since < SHEET_SWAP)
            || self.quick_close_hover.is_moving(now)
            || self.dnd_glow.is_moving(now)
            || self.night_glow.is_moving(now)
            || self.quick_retiring_at.is_some_and(|since| now - since < SHEET_SWAP)
            || self.center_retiring_at.is_some_and(|since| now - since < SHEET_SWAP)
            || self.quick_returned_at.is_some_and(|since| now - since < SHEET_SWAP)
            || self.notice_action_hover.iter().any(|spring| spring.is_moving(now))
            || self.notice.is_some() && self.notice_progress.is_moving(now)
            || self.notice_swipe.as_ref().is_some_and(|s| s.held || s.offset.is_moving(now))
            || self.notice_swiped.is_some()
            || self.center.is_animating(now)
            || self.network_panel.as_ref().is_some_and(|p| p.is_animating(now))
            || self.timer_chip.is_moving(now)
            || self.tasks_chip.is_moving(now)
            || self.record_chip.is_moving(now)
            || self.mic_dot.is_moving(now)
            || self.calendar.as_ref().is_some_and(|c| c.is_animating(now)) && self.calendar_shown()
            || (self.tasks_chip.state(now).x > 0.0 || self.tasks_pill_shown()) && self.tasks.is_animating(now)
            || self.panel.is_moving(now)
            || self.chip.is_moving(now)
            || self.clock_chip.is_moving(now)
            || self.floats.is_moving(now)
            || self.tug.is_none() && (self.tug_x.is_moving(now) || self.tug_y.is_moving(now))
            || self.edge.is_animating(now)
            || self.curtain.is_animating(now)
            || self.membrane.is_animating(now)
            || self.focus_frame.is_moving(now)
            || self.side_hud.is_animating(now)
            || self.config.bar && self.bar.is_animating(now)
            || self.floats.target() > 0.5 && self.floats.value(now) < 0.5
            || self.float_lit.values().any(|s| s.is_moving(now))
            || self.float_pressed.is_some_and(|(_, t)| now - t < 0.5)
            || self.music_chip.is_moving(now)
            || self.clock_w.as_ref().is_some_and(|s| s.is_moving(now))
            || self.editor_panel.is_moving(now)
            || self.launcher.as_ref().is_some_and(|l| l.is_animating(now))
            // a macro being recorded keeps its dot breathing
            || self.nvim.as_ref().is_some_and(|e| !e.recording.is_empty()) && self.chip.state(now).x > 0.0
            // a write flashes the chip
            || self.nvim_saved.is_some_and(|t| now - t < editor::SAVE_FLASH) && self.chip.state(now).x > 0.0
            // the pill's content animates the whole time it is up
            || self.pill_open() && self.pill.state(now).x > 0.0
                && !self.activity.as_ref().is_some_and(|a| activity::takes_keyboard(&a.event) && now - a.since >= 0.5)
    }

    fn eq_moving(&self) -> bool {
        self.eq.iter().zip(&self.eq_target).any(|(a, b)| (a - b).abs() > 0.004)
    }

    /// Are the compact bars meant to show beside the clock?
    fn want_reveal(&self, now: Time) -> bool {
        self.song.is_some() && !self.dodged && !self.pill_open() && self.launcher.is_none() && !self.silent_expired(now)
    }

    /// Should the island be open as the player right now? A toast gives
    /// way to the pointer (you reached for it, after all); Touch ID and a
    /// notification do not.
    /// The pointer on the island opens the editor's panel when Neovim is
    /// the window in front; music keeps the pointer otherwise.
    fn want_editor_panel(&self, now: Time) -> bool {
        (self.editor_chip_hovered || (self.hovered && self.editor_panel.target() > 0.5)
            || self.output_open || self.nvim.as_ref().is_some_and(|e| e.cmdline.is_some()))
            && !self.timer_hovered
            && self.editor_live(now)
            && self.auth.is_none()
            && self.activity.is_none()
            && self.notice.is_none()
            && self.launcher.is_none()
            && !self.quick_open
            && !self.center.open
            && !self.hidden.any()
            && !self.dodged
    }

    fn want_player(&self) -> bool {
        (self.eq_hovered || (self.hovered && self.player.target() > 0.5) || self.scrub.is_some())
            && !self.timer_hovered
            && self.song.is_some()
            && self.nvim.as_ref().is_none_or(|e| e.cmdline.is_none())
            && self.editor_panel.target() < 0.5
            && self.auth.is_none()
            && self.activity.is_none()
            && self.notice.is_none()
            && self.launcher.is_none()
            && !self.quick_open
            && !self.hidden.any()
            && !self.dodged
    }

    /// Is anything using the pill (Touch ID owns it; a notification takes
    /// it when Touch ID does not)?
    fn pill_open(&self) -> bool {
        self.auth.is_some() || self.activity.is_some() || self.notice.is_some() || self.toast.is_some() || self.quick_open || self.center.open
    }

    fn silent_expired(&self, now: Time) -> bool {
        self.silent_since.is_some_and(|since| now - since >= self.config.silent_after)
    }

    /// Re-aim the movers after a state change.
    pub fn sync(&mut self, now: Time) {
        // the Control Center's "Now playing" follows the player
        let media = self.song.as_ref().map(|song| quick::Media {
            title: song.now.title.clone(),
            artist: song.now.artist.clone(),
            app: song.now.app.clone(),
            art_url: song.now.art_url.clone(),
            playing: song.now.playing,
        });
        if self.controls.media != media {
            self.controls.media = media;
        }
        // System events and panels rise on their own even when the compact
        // island is parked above the edge. Once they finish, it stays hidden.
        // put away by hand (mod+i), the island drops into the edge and sends
        // a ripple along it, as the drop meets the water; not when it goes
        // back under a fullscreen window after a pill, over a playing video
        if self.is_hidden() && self.hidden.user && self.hide.target() < 0.5 && self.config.edge_light {
            let cx = self.island_cx(now);
            self.membrane.ripple(cx, 0.8, now + 0.12);
        }
        self.hide.set(if self.is_hidden() { 1.0 } else { 0.0 }, now);
        self.dodge.set(if self.dodged { 1.0 } else { 0.0 }, now);
        // The compact island always breathes on ordinary hover, including
        // while music and an editor chip are present. Their own hit targets
        // still unfold their panels instead of adding extra padding.
        let hover = self.config.hover && self.hovered && !self.hidden.any()
            && !self.pill_open() && self.launcher.is_none()
            && !self.want_player() && !self.want_editor_panel(now);
        self.hover_pad.set(if hover { 1.0 } else { 0.0 }, now);
        // the progress line and the controls follow the pointer on springs
        let over_track = matches!(self.hovered_control, Some(player::Control::Seek(_)));
        let track = if self.scrub.is_some() {
            1.0
        } else if over_track && self.want_player() {
            TRACK_HOVER
        } else {
            0.0
        };
        self.track_grow.set(track, now);
        for (i, spring) in self.control_hover.iter_mut().enumerate() {
            let control = match i {
                0 => player::Control::Previous,
                1 => player::Control::PlayPause,
                2 => player::Control::Next,
                _ => player::Control::Clear,
            };
            let lit = self.hovered_control == Some(control) && self.scrub.is_none();
            spring.set(if lit { 1.0 } else { 0.0 }, now);
        }
        // the editor's chip rides beside the clock while Neovim is in front
        // the floating buttons come out once a page is open, and fall back
        // in quickly (no bounce) when it closes
        let folding = !self.pill_open() && self.launcher.is_none() && !self.want_player() && !self.want_editor_panel(now);
        if !folding && self.fold_at > now {
            // another sheet takes the box after all: no waiting
            self.fold_at = now;
            self.quick_retiring_at = self.quick_retiring_at.map(|t| t.min(now));
        }
        if self.floats_wanted() {
            // fresh, they wait for the page to have most of its size, so
            // they leave its sides rather than the compact line's
            let wait = if self.floats.value(now) < 0.05 { FLOAT_DELAY } else { 0.0 };
            self.floats.set_with(1.0, FLOAT_RESPONSE, FLOAT_DAMPING, now + wait);
        } else {
            // a page folding back to the line with its buttons out: they go
            // in first, and the page folds once they are behind its edge
            // (riding its shrinking sides they were left hanging in the air)
            let out = self.floats.target() > 0.5 && self.floats.value(now) > FLOAT_TUCKED;
            self.floats.set_with(0.0, FLOAT_RETURN, 1.0, now);
            if out && folding {
                self.fold_at = (1..=60).map(|i| now + i as f64 * 0.005)
                    .find(|t| self.floats.value(*t) <= FLOAT_TUCKED)
                    .unwrap_or(now + 0.3);
                // the page's content stays put until then, too
                self.quick_retiring_at = self.quick_retiring_at.map(|t| t.max(self.fold_at));
            }
            self.float_hover = None;
        }
        for (button, spring) in self.float_lit.iter_mut() {
            spring.set(if self.float_hover == Some(*button) { 1.0 } else { 0.0 }, now);
        }
        if let Some(button) = self.float_hover {
            self.float_lit.entry(button).or_insert_with(|| Spring::new(0.0, 0.32, 0.75)).set(1.0, now);
        }
        self.float_lit.retain(|_, s| s.target() > 0.0 || s.is_moving(now));
        let shown = self.config.compact;
        self.clock_chip.set(if shown.clock { 1.0 } else { 0.0 }, now);
        self.music_chip.set(if shown.music { 1.0 } else { 0.0 }, now);
        let chip = shown.editor && self.editor_live(now) && !self.hidden.any() && !self.dodged && !self.pill_open() && self.launcher.is_none();
        self.chip.set(if chip { 1.0 } else { 0.0 }, now);
        let timer = shown.timer && self.timer.is_some() && !self.hidden.any() && !self.dodged && self.launcher.is_none();
        for (action, spring) in self.quick_hover_springs.iter_mut() {
            if self.quick_hover != Some(*action) { spring.set(0.0, now); }
        }
        if let Some(action) = self.quick_hover.filter(|a| *a != quick::Action::Close) {
            self.quick_hover_springs.entry(action).or_insert_with(|| Spring::new(0.0, 0.24, 0.83)).set(1.0, now);
        }
        self.quick_hover_springs.retain(|_, s| s.target() > 0.0 || s.is_moving(now));
        self.quick_close_hover.set(if self.quick_hover == Some(quick::Action::Close) { 1.0 } else { 0.0 }, now);
        if !self.connections_open() {
            if let Some(panel) = &mut self.network_panel { panel.close(); }
            if !self.activity_retiring.as_ref().is_some_and(|a| matches!(a.event, activity::Event::WifiOpen | activity::Event::BluetoothOpen)) {
                self.network_panel = None;
            }
        }
        self.timer_chip.set(if timer { 1.0 } else { 0.0 }, now);
        let rec = shown.record && matches!(self.recording, Some(Recording::Running(..) | Recording::Stopping(_)))
            && !self.hidden.any() && !self.dodged && self.launcher.is_none();
        self.record_chip.set(if rec { 1.0 } else { 0.0 }, now);
        let jobs = shown.tasks && !self.tasks.is_empty() && !self.hidden.any() && !self.dodged && self.launcher.is_none();
        self.tasks_chip.set(if jobs { 1.0 } else { 0.0 }, now);
        let editor_panel = self.want_editor_panel(now);
        if editor_panel {
            self.editor_command = self.nvim.as_ref().and_then(|e| e.cmdline.clone());
            self.output_view = self.output_open;
        }
        if editor_panel && self.editor_panel.target() < 0.5 {
            self.editor_opened = now;
        }
        self.editor_panel.set(if editor_panel { 1.0 } else { 0.0 }, now);
        let player = self.want_player();
        if player {
            // reaching for the island answers the toast
            self.toast = None;
        }
        if player && self.player.target() < 0.5 {
            self.player_opened = now;
        }
        self.player.set(if player { 1.0 } else { 0.0 }, now);
        self.reveal.set(if self.want_reveal(now) { 1.0 } else { 0.0 }, now);
        let pill = self.pill_open() && !self.want_player();
        // closing, the fold may be waiting for the floating buttons
        self.pill.set(if pill { 1.0 } else { 0.0 }, if pill { now } else { now.max(self.fold_at) });
        self.panel.set(if self.launcher.is_some() { 1.0 } else { 0.0 }, now);
        let result = matches!(self.auth, Some(Auth::Ok(_) | Auth::Fail(_)));
        self.ring.set(if result { 1.0 } else { 0.0 }, now);
        if !result {
            self.mark.set(0.0, now);
        }
        if !self.song.as_ref().is_some_and(|s| s.playing()) {
            self.eq_target = [0.0; EQ_BANDS];
        }
    }

    /// Time-driven state changes: a pending track swap once the old text
    /// is in, the Touch ID result timing out, silence expiring. Returns
    /// whether anything changed (then redraw).
    pub fn advance(&mut self, now: Time) -> bool {
        let mut changed = false;
        self.center.advance(now);
        if self.output_open && now >= self.output_until && !self.hovered
            && self.nvim.as_ref().is_none_or(|e| e.cmdline.is_none()) {
            self.output_open = false;
            changed = true;
        }
        if self.nvim.as_ref().is_some_and(|e| e.cmdline.is_some()) && now - self.command_seen > 0.8 {
            self.nvim.as_mut().unwrap().cmdline = None;
            changed = true;
        }
        if let Some(launcher) = &mut self.launcher { changed |= launcher.advance(now); }
        changed |= self.side_hud.advance(now);
        if self.connections_open() {
            if let Some(panel) = &mut self.network_panel { changed |= panel.advance(now); }
        }
        if self.toast.is_some_and(|since| now - since >= self.config.toast_hold) {
            self.toast = None;
            changed = true;
        }
        if self.quick_retiring_at.is_some_and(|since| now - since >= SHEET_SWAP) {
            self.quick_retiring_at = None;
            changed = true;
        }
        if self.center_retiring_at.is_some_and(|since| now - since >= SHEET_SWAP && self.center_swapped())
            || self.center_retiring_at.is_some() && self.pill.state(now).x <= 0.0
        {
            self.center_retiring_at = None;
            changed = true;
        }
        if self.logout_armed_at.is_some_and(|since| now - since >= LOGOUT_CONFIRM) {
            self.logout_armed_at = None;
            changed = true;
        }
        if self.quick_view_from.is_some_and(|(_, since)| now - since >= SHEET_SWAP) {
            self.quick_view_from = None;
            changed = true;
        }
        if self.quick_returned_at.is_some_and(|since| now - since >= SHEET_SWAP) {
            self.quick_returned_at = None;
            changed = true;
        }
        match self.auth {
            Some(Auth::Ok(since)) if now - since >= AUTH_OK_HOLD => {
                self.auth = None;
                changed = true;
            }
            Some(Auth::Fail(since)) if now - since >= AUTH_FAIL_HOLD => {
                self.auth = None;
                changed = true;
            }
            // the mark starts drawing once the ring has (nearly) formed
            Some(Auth::Ok(since) | Auth::Fail(since)) if now - since >= MARK_DELAY => self.mark.set(1.0, now),
            _ => {}
        }
        if self.tasks.advance(now) {
            changed = true;
        }
        match self.recording {
            // Escape (or anything taking the pill) during 3-2-1 cancels it
            Some(Recording::Countdown(_)) if !self.activity.as_ref().is_some_and(|a| a.event == activity::Event::RecordCountdown) => {
                self.edge.cancel_scheduled(now);
                self.recording = None;
                changed = true;
            }
            Some(Recording::Countdown(since)) if now >= since + activity::RECORD_COUNTDOWN => {
                self.activity_retiring = self.activity.take();
                // capture once the pill has folded away
                self.recording = Some(Recording::Starting(now + 0.45));
                changed = true;
            }
            Some(Recording::Starting(at)) if now >= at => {
                self.recording = Some(Recording::Launching);
                self.record_start = true;
                changed = true;
            }
            _ => {}
        }
        // the jobs' pill has nothing left to show
        if self.tasks.is_empty() && self.activity.as_ref().is_some_and(|a| a.event == activity::Event::Tasks) {
            self.activity_retiring = self.activity.take();
            changed = true;
        }
        if self.activity.as_ref().is_some_and(|a| now - a.updated_at >= a.event.hold()) {
            self.activity_retiring = self.activity.take();
            changed = true;
        }
        if self.activity_retiring.is_some() && self.pill.state(now).x <= 0.0 {
            self.activity_retiring = None;
            self.activity_art.clear();
            if !self.connections_open() { self.network_panel = None; }
            if !self.calendar_open() { self.calendar = None; }
        }
        if self.timer.as_ref().is_some_and(|timer| timer.finished(now)) {
            self.timer_ran_out(now);
            changed = true;
        }
        // An activity that waited for Touch ID or the launcher gets the
        // pill before an ordinary notification.
        if self.auth.is_none() && self.launcher.is_none() && !self.center.open && self.activity.is_none() {
            if let Some(event) = self.activity_queued.take() {
                self.show_activity(event, now);
                changed = true;
            }
        }
        // a notification that waited for Touch ID or the launcher takes
        // the pill now
        if self.auth.is_none() && self.activity.is_none() && self.launcher.is_none() && !self.center.open {
            if let Some(note) = self.notice_queued.take() {
                self.show_notice(note, now);
                changed = true;
            }
        }
        // a notification in the hand (a reply being typed, the pointer on
        // it, a swipe) does not time out
        let held = self.held_notice().is_some();
        if !held && self.notice.as_ref().is_some_and(|n| now - n.since >= self.config.notice_hold) {
            self.notice = None;
            self.notice_prev = None;
            changed = true;
        }
        if self.notice_swipe.as_ref().is_some_and(|s| s.held && s.axis && now - s.last > SWIPE_IDLE) {
            if let Some(id) = self.release_swipe(now) { self.swiped_id = Some(id); }
            changed = true;
        }
        if self.notice_swipe.as_ref().is_some_and(|s| !s.held && !s.offset.is_moving(now)) {
            self.notice_swipe = None;
        }
        if let Some((_, fly, width)) = &self.notice_swiped {
            if fly.value(now).abs() >= *width || self.pill.state(now).x <= 0.0 {
                self.notice_swiped = None;
                changed = true;
            }
        }
        if self.notice_prev.is_some() && !self.roll.is_moving(now) {
            self.notice_prev = None;
        }
        if self.silent_expired(now) && self.reveal.state(now).x > 0.0 {
            changed = true; // sync retracts the bars
        }
        if changed {
            self.sync(now);
        }
        changed
    }

    /// The next instant `advance` has something to do.
    pub fn deadline(&self, now: Time) -> Option<Time> {
        let mut next: Option<Time> = None;
        let mut consider = |t: Time| next = Some(next.map_or(t, |n: Time| n.min(t)));
        if self.connections_open() {
            if let Some(deadline) = self.network_panel.as_ref().and_then(|p| p.deadline()) { consider(deadline); }
        }
        if self.output_open && !self.hovered && self.nvim.as_ref().is_none_or(|e| e.cmdline.is_none()) {
            consider(self.output_until.max(now + 0.01));
        }
        if self.nvim.as_ref().is_some_and(|e| e.cmdline.is_some()) { consider(self.command_seen + 0.81); }
        if let Some(since) = self.logout_armed_at { consider(since + LOGOUT_CONFIRM); }
        match self.auth {
            Some(Auth::Ok(since)) => consider(since + AUTH_OK_HOLD),
            Some(Auth::Fail(since)) => consider(since + AUTH_FAIL_HOLD),
            _ => {}
        }
        if let Some(n) = &self.notice {
            consider(n.since + self.config.notice_hold);
        }
        if let Some(a) = &self.activity {
            if a.event.hold().is_finite() {
                consider(a.updated_at + a.event.hold());
            }
        }
        if let Some(t) = self.side_hud.deadline() {
            consider(t);
        }
        if let Some(timer) = &self.timer {
            if timer.running && timer.counts_up() {
                // the stopwatch's next second
                if !self.hidden.any() {
                    consider(now + (1.0 - timer.reading(now).fract()).max(0.02));
                }
            } else if timer.running {
                let remaining = timer.reading(now);
                if remaining <= 0.06 {
                    consider(now + remaining.max(0.005));
                } else if !self.hidden.any() {
                    // A minutes-long ring barely moves in a second; only the
                    // final minute benefits from a 30 Hz sweep. Do not keep
                    // both a frame callback and a timer driving the same chip.
                    consider(now + if remaining <= 60.0 { 1.0 / 30.0 } else { remaining.fract().max(0.02) });
                }
            }
        }
        if let Some(l) = &self.launcher {
            consider(l.next_caret(now));
        }
        if let Some(at) = self.tasks.next_change(now) {
            consider(at);
        }
        match self.recording {
            Some(Recording::Countdown(since)) => consider(since + activity::RECORD_COUNTDOWN),
            Some(Recording::Starting(at)) => consider(at),
            // the time beside the dot ticks each second
            Some(Recording::Running(since, _)) if !self.hidden.any() => consider(now + 1.0 - (now - since).fract()),
            _ => {}
        }
        if self.tasks.turning() && (self.tasks_chip.state(now).x > 0.0 || self.tasks_pill_shown()) {
            consider(now + 1.0 / 30.0);
        }
        if let Some(since) = self.toast {
            consider(since + self.config.toast_hold);
        }
        if let Some(since) = self.silent_since {
            if now - since < self.config.silent_after && self.song.is_some() {
                consider(since + self.config.silent_after);
            }
        }
        next
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden.any() && self.auth.is_none() && self.activity.is_none()
            && self.notice.is_none() && (self.toast.is_none() || self.hidden.user)
            && self.launcher.is_none() && !self.quick_open && !self.center.open
    }

    pub fn toggle_visibility(&mut self, now: Time) {
        if self.hidden.any() {
            self.hidden.user = false;
            self.hidden.fullscreen_override = self.hidden.fullscreen;
        } else if self.hidden.fullscreen {
            self.hidden.fullscreen_override = false;
        } else {
            self.hidden.user = true;
        }
        self.sync(now);
    }

    pub fn show(&mut self, now: Time) {
        self.hidden.user = false;
        self.hidden.fullscreen_override = self.hidden.fullscreen;
        self.sync(now);
    }

    pub fn hide(&mut self, now: Time) {
        self.hidden.user = true;
        self.hidden.fullscreen_override = false;
        self.sync(now);
    }

    pub fn set_fullscreen(&mut self, fullscreen: bool, now: Time) {
        if self.hidden.fullscreen != fullscreen {
            self.hidden.fullscreen = fullscreen;
            self.hidden.fullscreen_override = false;
            self.sync(now);
        }
    }

    pub fn song(&self) -> Option<&Song> {
        self.song.as_ref()
    }

    /// One line for `island-shell status`.
    pub fn describe(&self, now: Time) -> String {
        format!(
            "song={:?} player={:.2} bars={:.2} pill={:.2} auth={:?} silent={} scrub={:?}",
            self.song.as_ref().map(|s| format!("{}{} — {}", if s.playing() { "" } else { "(paused) " }, s.now.artist, s.now.title)),
            self.player.state(now).x,
            self.reveal.state(now).x,
            self.pill.state(now).x,
            self.auth.map(|a| format!("{a:?}")).or_else(|| self.notice.as_ref().map(|n| format!("notice {:?}", n.note.app))),
            self.silent_since.map_or(0.0, |s| now - s),
            self.scrub.map(|(f, _)| (f * 100.0).round())
        ) + &self.activity.as_ref().map_or_else(String::new, |a| format!(" activity={:?}", a.event))
            + &self.launcher.as_ref().map_or_else(String::new, |l| format!(" launcher_rows={} files={}", l.result_count(), l.file_count()))
            + &if self.quick_open { " quick=1" } else { "" }
            + &if self.connections_open() { self.network_panel.as_ref().map_or_else(String::new, |p| p.describe()) } else { String::new() }
            + &self.timer.as_ref().map_or_else(String::new, |t| format!(" timer={:.0}s{}", t.reading(now), if t.running { "" } else { "(paused)" }))
            + &match &self.nvim {
            Some(e) => format!(
                " nvim={{{} {} {}:{} chip={:.2} panel={:.2}}}",
                e.mode().label(),
                if e.file.is_empty() { "[No Name]" } else { &e.file },
                e.line,
                e.col,
                self.chip.state(now).x,
                self.editor_panel.state(now).x
            ),
            None => String::new(),
        }
    }

    /// The active player changed (or went away). A track that starts
    /// announces itself with a toast, the way a notification arrives.
    pub fn set_player(&mut self, np: Option<&crate::dbus::NowPlaying>, now: Time) {
        match (np, &mut self.song) {
            (None, _) => {
                self.song = None;
                self.toast = None;
                self.art.clear();
            }
            (Some(np), Some(song)) if song.same_track(np) => {
                // play / pause, or fresher metadata for the same track
                let was_playing = song.now.playing;
                let position = song.position(now);
                song.now = np.clone();
                song.set_position(position, now);
                if np.playing && !was_playing && self.toast.is_none() {
                    self.announce(now);
                }
            }
            (Some(np), _) => {
                // another track: the art on the old path is stale
                self.art.clear();
                self.song = Some(Song::new(np, now));
                self.announce(now);
            }
        }
        self.update_silence(now);
        self.sync(now);
    }

    /// The user cleared the player: it folds away now (the watcher keeps it
    /// hidden until it plays again). Returns its bus name and whether it
    /// was playing, for the pause.
    pub fn clear_song(&mut self, now: Time) -> Option<(String, bool)> {
        let song = self.song.as_ref()?;
        let cleared = (song.now.bus_name.clone(), song.now.playing);
        self.scrub = None;
        self.eq_hovered = false;
        self.set_player(None, now);
        Some(cleared)
    }

    /// Show the "now playing" toast, unless the island has something more
    /// important on (or the player is already open under the pointer).
    fn announce(&mut self, now: Time) {
        if self.hidden.user || self.auth.is_some() || self.activity.is_some() || self.notice.is_some()
            || self.launcher.is_some() || self.quick_open || self.center.open || self.want_player() {
            return;
        }
        if self.song.as_ref().is_some_and(|s| s.playing()) {
            self.toast = Some(now);
            if self.hidden.fullscreen { self.hide.set_at(0.0, now); }
        }
    }

    /// The player's position, fetched over D-Bus.
    pub fn set_position(&mut self, position: f64, now: Time) {
        if let Some(song) = self.song.as_mut() {
            song.set_position(position, now);
        }
    }

    /// The bus name whose position is worth asking for right now.
    pub fn position_wanted(&self, now: Time) -> Option<String> {
        let song = self.song.as_ref()?;
        (self.player.state(now).x > 0.0 || self.toast.is_some()).then(|| song.now.bus_name.clone())
    }

    /// The sink's mute state changed.
    pub fn set_muted(&mut self, muted: bool, now: Time) {
        self.muted = muted;
        self.update_silence(now);
        self.sync(now);
    }

    fn update_silence(&mut self, now: Time) {
        let silent = self.song.as_ref().is_some_and(|s| !s.playing() || self.muted);
        match (silent, self.silent_since) {
            (true, None) => self.silent_since = Some(now),
            (false, Some(_)) => self.silent_since = None,
            _ => {}
        }
    }

    /// New band levels from cava (0..1). Whether they are worth a frame is
    /// another matter: the bars may be folded away (paused, dodged, behind
    /// a notification), and then nobody would see them move.
    pub fn set_eq(&mut self, levels: [f32; EQ_BANDS], now: Time) -> bool {
        if self.song.as_ref().is_some_and(|s| s.playing()) {
            self.eq_target = levels.map(|v| v as f64);
        }
        self.eq_visible(now)
    }

    fn eq_visible(&self, now: Time) -> bool {
        self.reveal.state(now).x > 0.01 || self.player.state(now).x > 0.01 || self.toast.is_some()
    }

    /// mod+d: the island unfolds into the launcher, or folds back. A
    /// hidden island shows the panel temporarily; a notification gives way.
    pub fn toggle_launcher(&mut self, now: Time) -> bool {
        self.quick_open = false;
        self.quick_hover = None;
        self.center.close();
        if self.launcher.is_some() {
            self.launcher = None;
        } else {
            // a hidden island stays hidden: the panel opens by itself and
            // takes the island's place back above the edge when it folds
            self.notice = None;
            self.notice_prev = None;
            self.activity = None;
            self.activity_queued = None;
            self.toast = None;
            self.catalog.refresh();
            self.launcher = Some(Launcher::new(now, self.catalog.apps.clone(), self.launcher_icons.clone(), self.file_search.clone(),
                self.clip_search.clone()));
        }
        self.sync(now);
        self.launcher.is_some()
    }

    /// The launcher, opened with a query already typed (`file:`, `clip:`).
    pub fn open_launcher_with(&mut self, query: &str, now: Time) {
        if self.launcher.is_none() {
            self.toggle_launcher(now);
        }
        if let Some(l) = self.launcher.as_mut() {
            l.push(query, now);
        }
        self.sync(now);
    }

    /// The ⋯ drawer (mod+o) needs the island to dodge; the panel cannot
    /// dodge with it, so it folds away first.
    pub fn set_dodged(&mut self, dodged: bool, now: Time) {
        self.dodged = dodged;
        self.launcher = None;
        self.quick_open = false;
        self.center.close();
        self.sync(now);
    }

    pub fn close_launcher(&mut self, now: Time) {
        if self.launcher.is_some() {
            self.launcher = None;
            self.sync(now);
        }
    }

    pub fn launcher_close_at(&self, cx: f64, px: f64, py: f64, now: Time) -> bool {
        if self.launcher.is_none() || self.panel.state(now).x < 0.95 { return false; }
        let (x, y) = self.box_origin(cx, now);
        Launcher::close_hit(px - x, py - y, launcher::PANEL_W)
    }

    pub fn set_asset_waker(&self, tx: smithay_client_toolkit::reexports::calloop::channel::Sender<()>) {
        self.catalog.set_waker(tx.clone());
        self.launcher_icons.set_waker(tx.clone());
        self.file_search.set_waker(tx.clone());
        self.clip_search.set_waker(tx.clone());
        crate::weather::set_waker(tx.clone());
        self.art.set_waker(tx.clone());
        self.activity_art.set_waker(tx);
    }

    pub fn drain_assets(&mut self, now: Time, scale: f64) -> bool {
        let catalog = self.catalog.drain();
        if catalog {
            // Warm visible icons before the user opens the launcher.
            for app in self.catalog.apps.iter().take(crate::launcher::VISIBLE_ROWS + 2) {
                if let Some(name) = &app.icon {
                    self.launcher_icons.get_scaled(name, (38.0 * scale).round() as i32);
                }
            }
            if let Some(launcher) = &mut self.launcher {
                launcher.set_apps(self.catalog.apps.clone(), now);
            }
        }
        let icons = self.launcher_icons.drain();
        let art = self.art.drain();
        let activity = self.activity_art.drain();
        // a forecast arrived: the open calendar shows it
        let mut files = crate::weather::take_fresh() && self.calendar_open();
        for (generation, reply) in self.file_search.drain() {
            if let Some(launcher) = &mut self.launcher {
                files |= launcher.receive_files(generation, reply, now);
            }
        }
        for (generation, history) in self.clip_search.drain() {
            if let Some(launcher) = &mut self.launcher {
                files |= launcher.receive_clips(generation, history, now);
            }
        }
        ((catalog || icons) && (self.launcher.is_some() || self.notice.is_some() || self.center.open)) || art || activity || files
    }

    /// The companion Neovim plugin sent its state.
    pub fn set_editor(&mut self, mut editor: Editor, now: Time) -> bool {
        if let Some(previous) = &self.nvim {
            if previous.server != editor.server && !editor.focused && self.editor_live(now) {
                return false;
            }
            if previous.server == editor.server {
                editor.cmdline = previous.cmdline.clone();
            }
            if previous.server != editor.server || previous.file != editor.file {
                self.editor_diagnostics = false;
                self.nvim_saved = None;
            }
        }
        let before = self.nvim.as_ref().map(|e| e.chip_state());
        let visible_before = self.chip.target() > 0.5;
        self.nvim_seen = now;
        let focused = editor.focused;
        let after = editor.chip_state();
        self.nvim = Some(editor);
        self.sync(now);
        // a frame is only worth it when what the island shows changed: the
        // plugin also reports cursor moves nobody can see from here
        let _ = focused;
        before.as_ref() != Some(&after) || visible_before != (self.chip.target() > 0.5) || self.editor_panel.state(now).x > 0.0
    }

    /// Acknowledgement means the command can actually be presented now.
    pub fn set_cmdline(&mut self, server: &str, command: Option<nvim::Cmdline>, now: Time) -> bool {
        if !self.nvim.as_ref().is_some_and(|e| e.server == server) { return false; }
        let available = self.editor_live(now) && !self.hidden.any() && !self.dodged
            && self.auth.is_none() && self.activity.is_none() && self.notice.is_none()
            && self.launcher.is_none() && self.toast.is_none() && !self.timer_hovered;
        if command.as_ref().is_some_and(|c| matches!(c.prefix.as_str(), ":" | "/" | "?"))
            && self.nvim.as_ref().is_some_and(|e| e.cmdline.is_none()) {
            self.output_open = false;
        }
        if command.is_none() && self.output_open {
            self.output_until = now + self.editor_output.as_ref().map_or(4.0, |o| o.hold());
        }
        self.nvim.as_mut().unwrap().cmdline = if available { command } else { None };
        self.command_seen = now;
        self.sync(now);
        available
    }

    pub fn accepts_result(&self, server: &str, now: Time) -> bool {
        self.nvim.as_ref().is_some_and(|e| e.server == server) && self.editor_live(now)
            && !self.hidden.any() && !self.dodged && self.auth.is_none()
            && self.activity.is_none() && self.launcher.is_none()
    }

    fn editor_height(&self) -> f64 {
        if self.output_view {
            crate::editor_output::HEIGHT + if self.editor_command.is_some() { 84.0 } else { 0.0 }
        } else if self.editor_command.is_some() { 84.0 } else { editor::PANEL_H }
    }

    pub fn set_result(&mut self, id: String, title: String, body: String, error: bool,
        append: bool, renderer: &Renderer, now: Time) {
        if append && self.editor_output.as_ref().is_some_and(|o| o.id == id) {
            let output = self.editor_output.as_mut().unwrap();
            output.append(&body, renderer);
            output.error |= error;
        } else {
            self.editor_output = Some(crate::editor_output::Output::new(id, title, body, error, renderer));
        }
        self.output_open = true;
        self.output_until = now + self.editor_output.as_ref().unwrap().hold();
        self.sync(now);
    }

    pub fn scroll_output(&mut self, rows: f64, now: Time) -> bool {
        if !self.output_open || !self.editor_panel_open(now) { return false; }
        if let Some(output) = &mut self.editor_output { output.scroll(rows); }
        self.output_until = now + 12.0;
        true
    }

    /// An editor went away. Only the instance that left is forgotten: two
    /// Neovims may be up, and the one quitting is rarely the one in front.
    pub fn editor_gone(&mut self, server: &str, now: Time) {
        if !server.is_empty() && self.nvim.as_ref().is_some_and(|e| e.server != server) {
            return;
        }
        self.nvim = None;
        self.nvim_saved = None;
        self.output_open = false;
        self.editor_output = None;
        self.editor_hover = None;
        self.editor_pressed = None;
        self.sync(now);
    }

    /// Is an editor connected, awake and in front?
    fn editor_live(&self, now: Time) -> bool {
        self.nvim.as_ref().is_some_and(|e| e.focused) && !nvim::stale(self.nvim_seen, now)
    }

    /// The editor wrote its buffer: no pill, the chip beside the clock
    /// flashes green for a moment instead.
    pub fn editor_saved(&mut self, server: &str, now: Time) {
        if !self.nvim.as_ref().is_some_and(|e| e.server == server) || !self.editor_live(now) {
            return;
        }
        self.nvim_saved = Some(now);
        self.sync(now);
    }

    /// A notification rises independently of the compact island and waits
    /// behind an active Touch ID.
    pub fn notify(&mut self, note: crate::notify::Notification, now: Time) {
        self.center.upsert(note.clone(), now);
        if self.dnd && !note.critical { return; }
        // Touch ID owns the pill, and the launcher owns the whole island:
        // the notification waits its turn
        if self.auth.is_some() || self.activity.is_some() || self.launcher.is_some() || self.center.open {
            self.notice_queued = Some(note);
            return;
        }
        self.show_notice(note, now);
    }

    fn show_notice(&mut self, note: crate::notify::Notification, now: Time) {
        // a pointer that was already resting where the pill appears does
        // not hold it; it has to move onto it
        if !self.notice.as_ref().is_some_and(|n| n.note.id == note.id) { self.notice_pointer = false; }
        // an update of the same job (LSP progress, say) takes the pill over
        // in place: no roll, the timer simply starts again
        if let Some(current) = self.notice.as_mut() {
            if (note.id != 0 && current.note.id == note.id)
                || (note.tag.is_some() && current.note.tag == note.tag) {
                if let Some(done) = note.progress { self.notice_progress.set(done, now); }
                current.note = note;
                current.since = now;
                self.notice_prev = None;
                self.sync(now);
                return;
            }
        }
        let fresh = self.notice.is_none();
        if self.notice.is_some() {
            // a newer one while the pill is up: the old rolls out through
            // the top, the new one in from below
            self.notice_prev = self.notice.take();
            self.roll.set_at(0.0, now);
            self.roll.set(1.0, now);
        }
        // the island swells out of the edge as a drop falls into water: a
        // ripple runs from it as the pill opens
        if fresh && self.config.edge_light {
            self.membrane.ripple(self.island_cx(now), SPLASH_NOTICE, now + PILL_LIGHT_DELAY * 0.5);
        }
        self.notice_progress = Spring::new(note.progress.unwrap_or(0.0), 0.35, 1.0);
        // its colour runs out of the island along the edge; a critical
        // one goes round the whole screen
        if self.config.edge_light {
            let (wave, kind) = if note.critical {
                (crate::edge::Wave::Around, crate::notify::AppKind::Critical)
            } else {
                (crate::edge::Wave::Top, note.kind)
            };
            // once the pill has opened, so the light leaves its lower edge
            // rather than slipping out from under the growing box
            self.edge.schedule(wave, icons::accent(kind), now + PILL_LIGHT_DELAY);
        }
        self.notice = Some(Notice { note, since: now });
        self.notice_hovered = None;
        for spring in &mut self.notice_action_hover { spring.set(0.0, now); }
        self.sync(now);
    }

    /// Dismiss what the pill shows (a click on it).
    pub fn dismiss_notice(&mut self, now: Time) -> bool {
        if self.auth.is_some() {
            return false;
        }
        if self.activity.is_some() {
            self.activity_retiring = self.activity.take();
            self.sync(now);
            return true;
        }
        if self.notice.is_some() {
            self.notice = None;
            self.notice_prev = None;
            self.notice_queued = None;
            self.sync(now);
            return true;
        }
        false
    }

    /// Present a live activity. Hardware feedback is immediate, so it
    /// takes the pill from a notification; Touch ID and the launcher still
    /// keep ownership until they close.
    pub fn live_activity(&mut self, event: activity::Event, now: Time) {
        // the battery's states light the frame's bottom corners
        match event {
            activity::Event::Charging { .. } => {
                self.low_battery = false;
                if self.config.edge_light {
                    self.edge.pulse(crate::edge::Wave::Rise, crate::edge::SUCCESS_GREEN, now);
                }
            }
            activity::Event::BatteryLow { .. } => self.low_battery = true,
            // the bar's battery: state only, no pill
            activity::Event::Battery { percent, charging, remaining } => {
                self.bar.battery = Some(crate::bar::Battery { percent, charging, remaining });
                if charging { self.low_battery = false; }
                return;
            }
            _ => {}
        }
        if let activity::Event::MicInUse { apps } = event {
            self.set_mic_users(apps, now);
            return;
        }
        if let activity::Event::MicMute { muted } = &event {
            self.controls.mic_muted = *muted;
            // the Control Center's own mic button shows it
            if self.quick_open {
                self.sync(now);
                return;
            }
        }
        // with the Control Center open its sliders show the change; no OSD
        // over it and none queued for after it closes
        if let activity::Event::Slider { kind, value, muted } = &event {
            if self.quick_open {
                if self.quick_drag != Some(slider_of(*kind)) {
                    self.set_control_level(slider_of(*kind), *value, now);
                    if *kind == activity::SliderKind::Volume { self.controls.muted = *muted; }
                }
                self.sync(now);
                return;
            }
            // otherwise the level slides out of the screen's left edge, as
            // on an iPhone; the island stays as it is
            self.side_hud.show(*kind, *value, *muted, now);
            // the bar's volume follows it
            match kind {
                activity::SliderKind::Volume => {
                    self.controls.volume = Some(*value);
                    self.controls.muted = *muted;
                }
                activity::SliderKind::Brightness => self.controls.brightness = Some(*value),
            }
            return;
        }
        if self.auth.is_some() || self.launcher.is_some() || self.center.open {
            self.activity_queued = Some(event);
            return;
        }
        self.show_activity(event, now);
    }

    fn show_activity(&mut self, event: activity::Event, now: Time) {
        // Connection attempts and password entry keep ownership of the sheet.
        if self.keyboard_panel_open() && !activity::takes_keyboard(&event) {
            self.activity_queued = Some(event);
            return;
        }
        self.activity_retiring = None;
        self.toast = None;
        self.scrub = None;
        if self.notice.is_some() {
            self.notice_queued = self.notice.take().map(|n| n.note);
            self.notice_prev = None;
        }
        if let activity::Event::Slider { value, .. } = &event {
            self.activity_value.set(*value, now);
        }
        // Key repeat must extend the hold without resetting opacity. Keep
        // the entrance even when alternating volume and brightness keys.
        if let Some(current) = &mut self.activity {
            if matches!(current.event, activity::Event::Slider { .. }) && matches!(event, activity::Event::Slider { .. }) {
                current.event = event;
                current.updated_at = now;
                self.sync(now);
                return;
            }
        }
        // the shutter blinks as the shot lands (after the capture: it is
        // never in the picture)
        if matches!(event, activity::Event::Screenshot { .. }) && self.config.edge_light {
            self.curtain.shutter(now);
        }
        if matches!(event, activity::Event::Screenshot { .. }) {
            self.activity_art.clear();
        }
        self.activity = Some(LiveActivity { event, since: now, updated_at: now });
        self.sync(now);
    }

    fn visible_activity(&self) -> Option<&LiveActivity> {
        self.activity.as_ref().or_else(|| {
            if self.auth.is_none() && self.notice.is_none() && self.toast.is_none() && self.launcher.is_none() {
                self.activity_retiring.as_ref()
            } else {
                None
            }
        })
    }

    pub fn keyboard_panel_open(&self) -> bool {
        self.quick_open || self.center.open || self.notice_reply.is_some()
            || self.activity.as_ref().is_some_and(|current| activity::takes_keyboard(&current.event))
    }

    pub fn close_activity_panel(&mut self, now: Time) -> bool {
        if self.notice_reply.is_some() {
            self.cancel_reply(now);
            return true;
        }
        if self.center.open && self.center.clear_reply(now) {
            return true;
        }
        if self.center.open {
            self.center.close();
            self.center_rect = None;
            self.sync(now);
            return true;
        }
        if self.quick_open {
            self.close_quick_menu(now);
            return true;
        }
        if !self.keyboard_panel_open() {
            return false;
        }
        self.activity_retiring = self.activity.take();
        self.sync(now);
        true
    }

    /// Start or control the compact countdown.
    pub fn timer_command(&mut self, command: &str, now: Time) -> Result<String, String> {
        match command {
            "menu" => {
                if self.auth.is_some() || self.launcher.is_some() {
                    return Err("close the current panel first".into());
                }
                self.hidden.user = false;
                self.live_activity(activity::Event::TimerMenu, now);
            }
            "pause" => {
                let timer = self.timer.as_mut().ok_or("no active timer")?;
                if timer.running { timer.pause(now); }
            }
            "resume" => {
                let timer = self.timer.as_mut().ok_or("no active timer")?;
                if !timer.running { timer.resume(now); }
            }
            "toggle" => {
                if !self.toggle_timer(now) {
                    return Err("no active timer".into());
                }
            }
            "cancel" | "stop" => {
                self.timer = None;
                self.cycle = None;
            }
            "status" => {
                return Ok(self.timer.as_ref().map_or_else(
                    || if self.cycle.is_some() { "waiting".to_string() } else { "stopped".to_string() },
                    |timer| format!("{:.0} {}{}", timer.reading(now).ceil(), if timer.running { "running" } else { "paused" },
                        match timer.kind {
                            timer_page::Kind::Timer => String::new(),
                            timer_page::Kind::Stopwatch => " stopwatch".into(),
                            timer_page::Kind::Pomodoro { phase, session, total } =>
                                format!(" pomodoro {} {session}/{total}", phase.title().to_lowercase()),
                        }),
                ));
            }
            "pomodoro" => {
                let plan = self.timer_page.pomodoro;
                self.start_phase(plan, timer_page::Phase::Focus, 1, now);
            }
            "stopwatch" => {
                self.cycle = None;
                self.timer = Some(Countdown::of(timer_page::Kind::Stopwatch, 0.0, now));
            }
            "next" => {
                let Some(Cycle { plan, waiting: Some(Some((phase, session))), .. }) = self.cycle else {
                    return Err("no Pomodoro phase waiting".into());
                };
                self.start_phase(plan, phase, session, now);
            }
            duration => {
                let seconds = activity::parse_duration(duration).ok_or_else(|| format!("bad duration {duration:?}"))?;
                self.cycle = None;
                self.timer = Some(Countdown::new(seconds, now));
            }
        }
        self.sync(now);
        Ok("ok".into())
    }

    /// Start a Pomodoro phase (the cycle's first, or the one waiting).
    fn start_phase(&mut self, plan: timer_page::Pomodoro, phase: timer_page::Phase, session: u32, now: Time) {
        self.cycle = Some(Cycle { plan, phase, session, waiting: None });
        let kind = timer_page::Kind::Pomodoro { phase, session, total: plan.sessions };
        self.timer = Some(Countdown::of(kind, plan.minutes(phase) as f64 * 60.0, now));
    }

    /// A countdown or a Pomodoro phase ran out: the alarm, a pill saying
    /// so, and a Pomodoro's next phase waiting to be started.
    fn timer_ran_out(&mut self, now: Time) {
        let Some(timer) = self.timer.take() else { return };
        // the line has closed round the screen: the frame flashes
        if self.config.edge_light {
            self.edge.pulse(crate::edge::Wave::Flash, timer_colour(timer.kind), now);
        }
        let event = match (timer.kind, self.cycle.as_mut()) {
            (timer_page::Kind::Pomodoro { phase, session, total }, Some(cycle)) => {
                let next = cycle.plan.after(phase, session);
                cycle.waiting = Some(next);
                let title = format!("{} окончен", phase.title());
                let detail = match next {
                    Some((next, n)) => format!("Далее: {} · {} мин · {n} из {total}", next.title().to_lowercase(), cycle.plan.minutes(next)),
                    None => format!("Помодоро завершён · {total} из {total}"),
                };
                activity::Event::TimerDone { title, detail }
            }
            _ => activity::Event::timer_done(),
        };
        if self.config.timer_sound {
            self.sounds.push(crate::sound::Sound::Alarm);
        }
        // an open timer page shows what follows instead of a pill
        if !self.timer_menu_open() {
            self.live_activity(event, now);
        }
    }

    /// The session as the timer page shows it.
    fn timer_session(&self, now: Time) -> timer_page::Session {
        match (&self.timer, &self.cycle) {
            (Some(t), _) => timer_page::Session::Active {
                kind: t.kind, reading: t.reading(now), running: t.running, urgent: t.urgent(now),
            },
            (None, Some(Cycle { plan, phase, waiting: Some(Some((next, n))), .. })) => timer_page::Session::Next {
                done: *phase, next: *next, session: *n, total: plan.sessions, minutes: plan.minutes(*next),
            },
            (None, Some(Cycle { plan, waiting: Some(None), .. })) => timer_page::Session::Finished { total: plan.sessions },
            _ => timer_page::Session::Setup,
        }
    }

    /// Sounds for the app to play since it last asked.
    pub fn take_sounds(&mut self) -> Vec<crate::sound::Sound> {
        std::mem::take(&mut self.sounds)
    }

    /// The ruler moved under the person's hand: a tick, if sounds are on.
    fn timer_ticks(&mut self) {
        if self.timer_page.take_ticks() && self.config.timer_sound {
            self.sounds.push(crate::sound::Sound::Tick);
        }
    }

    pub fn timer_page_open(&self) -> bool {
        self.auth.is_none() && self.timer_menu_open()
    }

    /// Carry out what the timer page asked for.
    fn run_timer_page(&mut self, command: timer_page::Command, now: Time) {
        use timer_page::Command;
        match command {
            Command::Timer(minutes) => {
                self.cycle = None;
                self.timer = Some(Countdown::new(minutes as f64 * 60.0, now));
            }
            Command::Pomodoro(plan) => self.start_phase(plan, timer_page::Phase::Focus, 1, now),
            Command::Stopwatch => {
                self.cycle = None;
                self.timer = Some(Countdown::of(timer_page::Kind::Stopwatch, 0.0, now));
            }
            Command::Toggle => { self.toggle_timer(now); }
            Command::Next => { let _ = self.timer_command("next", now); }
            Command::Cancel => {
                self.timer = None;
                self.cycle = None;
            }
            Command::Close => {
                self.close_activity_panel(now);
                self.return_to_quick_menu(now);
            }
        }
        self.sync(now);
    }

    fn timer_page_origin(&self) -> Option<(f64, f64)> {
        self.timer_menu_rect.map(|r| (r.x, r.y))
    }

    /// The held ruler follows the pointer, even outside the page.
    pub fn timer_drag_to(&mut self, px: f64, now: Time) -> bool {
        let Some((x, _)) = self.timer_page_origin() else { return false };
        if !self.timer_page.dragging() { return false; }
        let moved = self.timer_page.drag_to(px - x, now);
        self.timer_ticks();
        moved
    }

    pub fn timer_release(&mut self, px: f64, now: Time) -> bool {
        let Some((x, _)) = self.timer_page_origin() else { return false };
        let released = self.timer_page.release(px - x, now);
        self.timer_ticks();
        released
    }

    pub fn scroll_timer(&mut self, px: f64, py: f64, delta: f64, discrete: bool, now: Time) -> bool {
        if !self.timer_page_open() { return false; }
        let Some((x, y)) = self.timer_page_origin() else { return false };
        let session = self.timer_session(now);
        let taken = self.timer_page.scroll(&session, px - x, py - y, delta, discrete, now);
        self.timer_ticks();
        if taken { self.sync(now); }
        taken
    }

    pub fn timer_key(&mut self, key: &str, now: Time) -> bool {
        if !self.timer_page_open() { return false; }
        let session = self.timer_session(now);
        if let Some(command) = self.timer_page.key(&session, key, now) {
            self.run_timer_page(command, now);
        }
        self.timer_ticks();
        self.sync(now);
        true
    }

    /// Start a recording (after 3-2-1) or stop the one running.
    pub fn toggle_recording(&mut self, now: Time) -> Result<&'static str, &'static str> {
        if self.recording.is_none() && !self.recorder {
            // installed since the island started? (a few stats along PATH)
            self.recorder = crate::recorder::available();
        }
        match self.recording {
            None if !self.recorder => {
                // say so instead of doing nothing
                self.show_activity(activity::Event::RecorderMissing, now);
                Err("wf-recorder is not installed (sudo sh scripts/03-install-screen-recorder.sh)")
            }
            None => {
                self.recording = Some(Recording::Countdown(now));
                self.show_activity(activity::Event::RecordCountdown, now);
                if !self.activity.as_ref().is_some_and(|a| a.event == activity::Event::RecordCountdown) {
                    self.recording = None;
                    return Err("busy");
                }
                // the frame flashes red with each count; the flashes are
                // over before capture starts (the recorder films the
                // overlay too, so nothing stays lit while it records)
                if self.config.edge_light {
                    for count in 0..3 {
                        self.edge.schedule(crate::edge::Wave::Flash, crate::edge::FAILURE_RED, now + count as f64 * activity::RECORD_COUNTDOWN / 3.0);
                    }
                }
                Ok("counting down")
            }
            Some(Recording::Countdown(_) | Recording::Starting(_)) => {
                if self.activity.as_ref().is_some_and(|a| a.event == activity::Event::RecordCountdown) {
                    self.activity_retiring = self.activity.take();
                }
                self.edge.cancel_scheduled(now);
                self.recording = None;
                self.sync(now);
                Ok("cancelled")
            }
            Some(Recording::Running(_, pid)) => {
                self.record_stop = Some(pid);
                self.recording = Some(Recording::Stopping(now));
                // the take is over: a red wave leaves the island
                if self.config.edge_light {
                    self.edge.pulse(crate::edge::Wave::Top, crate::edge::FAILURE_RED, now);
                }
                self.sync(now);
                Ok("stopping")
            }
            Some(Recording::Launching | Recording::Stopping(_)) => Ok("busy"),
        }
    }

    fn recording_seconds(&self, now: Time) -> Option<u64> {
        match self.recording {
            Some(Recording::Running(since, _)) => Some((now - since).max(0.0) as u64),
            Some(Recording::Stopping(at)) => self.record_last.map(|since| (at - since).max(0.0) as u64),
            _ => None,
        }
    }

    /// The app starts the recorder when 3-2-1 is over.
    pub fn take_record_start(&mut self) -> bool {
        std::mem::take(&mut self.record_start)
    }

    /// The app stops the recorder (SIGINT to this process).
    pub fn take_record_stop(&mut self) -> Option<u32> {
        self.record_stop.take()
    }

    /// The recorder reported.
    pub fn recorder_event(&mut self, event: crate::recorder::Event, now: Time) {
        match event {
            crate::recorder::Event::Started { pid } => {
                self.recording = Some(Recording::Running(now, pid));
                self.record_last = Some(now);
            }
            crate::recorder::Event::Finished { path, thumb, .. } => {
                let seconds = self.record_last.map_or(0, |since| (now - since).max(0.0) as u64);
                self.recording = None;
                self.record_last = None;
                if let Some(path) = path {
                    self.activity_art.clear();
                    self.live_activity(activity::Event::Recording { path, thumb, seconds }, now);
                }
            }
        }
        self.sync(now);
    }

    /// A click on the red dot stops the recording.
    pub fn click_record_at(&mut self, px: f64, py: f64, now: Time) -> bool {
        if !self.record_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h) { return false; }
        let _ = self.toggle_recording(now);
        true
    }

    /// A job request from the control socket.
    pub fn task(&mut self, request: tasks::Request, now: Time) -> Result<String, String> {
        let outcome = self.tasks.apply(request, now)?;
        let reply = match outcome {
            tasks::Outcome::Listed(text) => return Ok(text),
            tasks::Outcome::Ended { announce: true } => {
                self.show_tasks(now);
                "ok".into()
            }
            _ => "ok".into(),
        };
        self.sync(now);
        Ok(reply)
    }

    /// The running jobs' owner processes (to notice one that is gone).
    pub fn task_owners(&self) -> Vec<(String, u32)> {
        self.tasks.owners()
    }

    /// A job's owner exited without ending it.
    pub fn task_orphaned(&mut self, id: &str, now: Time) {
        if let Some(tasks::Outcome::Ended { announce }) = self.tasks.orphaned(id, now) {
            if announce { self.show_tasks(now); }
            self.sync(now);
        }
    }

    fn tasks_pill_shown(&self) -> bool {
        self.visible_activity().is_some_and(|a| a.event == activity::Event::Tasks)
    }

    /// Unfold the jobs (or keep them unfolded a while longer).
    fn show_tasks(&mut self, now: Time) {
        if let Some(current) = self.activity.as_mut().filter(|a| a.event == activity::Event::Tasks) {
            current.updated_at = now;
            return;
        }
        self.live_activity(activity::Event::Tasks, now);
    }

    /// A click on the jobs' ring unfolds them; a click on the unfolded
    /// pill folds it back.
    pub fn click_tasks_at(&mut self, px: f64, py: f64, now: Time) -> bool {
        let inside = |r: Option<Rect>| r.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h);
        if inside(self.tasks_pill_rect) && self.activity.as_ref().is_some_and(|a| a.event == activity::Event::Tasks) {
            self.activity_retiring = self.activity.take();
            self.sync(now);
            return true;
        }
        if !inside(self.tasks_rect) { return false; }
        self.show_tasks(now);
        self.sync(now);
        true
    }

    /// The pointer resting on the unfolded jobs keeps them open.
    pub fn hold_tasks_hover(&mut self, px: f64, py: f64, now: Time) {
        let Some(r) = self.tasks_pill_rect else { return };
        if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
            if let Some(current) = self.activity.as_mut().filter(|a| a.event == activity::Event::Tasks) {
                current.updated_at = now;
            }
        }
    }

    /// The applications recording from a microphone changed.
    pub fn set_mic_users(&mut self, apps: Vec<String>, now: Time) {
        self.mic_dot.set(if apps.is_empty() { 0.0 } else { 1.0 }, now);
        self.mic_users = apps;
        self.sync(now);
    }

    /// Unfold the month (a right click on the clock, `island-shell calendar`).
    pub fn open_calendar(&mut self, now: Time) {
        if self.calendar_open() || self.auth.is_some() || self.launcher.is_some() { return; }
        if self.center.open {
            self.center.close();
            self.center_rect = None;
        }
        if self.quick_open { self.close_quick_menu(now); }
        self.calendar = Some(Calendar::new(now));
        self.show_activity(activity::Event::CalendarOpen, now);
        // a sheet that had to wait leaves nothing half-made
        if !self.calendar_open() { self.calendar = None; }
    }

    pub fn calendar_open(&self) -> bool {
        self.activity.as_ref().is_some_and(|a| a.event == activity::Event::CalendarOpen)
    }

    fn calendar_shown(&self) -> bool {
        self.visible_activity().is_some_and(|a| a.event == activity::Event::CalendarOpen)
    }

    /// Is (px, py) on the compact clock?
    /// Without a clock at rest, any free part of the closed island stands in for it.
    pub fn clock_at(&self, px: f64, py: f64) -> bool {
        let inside = |r: &Rect| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h;
        if !self.accepts_quick_menu_click() { return false; }
        if self.config.compact.clock { return self.clock_rect.is_some_and(|r| inside(&r)); }
        self.line_rect.is_some_and(|r| inside(&r))
            && ![self.eq_rect, self.cover_rect, self.timer_rect, self.timer_mark_rect, self.tasks_rect, self.record_rect,
                self.editor_chip_rect].iter().flatten().any(inside)
    }

    /// The timer's chip: its reading, and its ring when the wings part them.
    fn on_timer(&self, px: f64, py: f64) -> bool {
        [self.timer_rect, self.timer_mark_rect].iter().flatten().any(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h)
    }

    /// The song on the closed island: its bars, and its cover in the wings.
    fn on_music(&self, px: f64, py: f64) -> bool {
        [self.eq_rect, self.cover_rect].iter().flatten().any(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h)
    }

    pub fn set_calendar_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        if !self.calendar_open() { return false; }
        let (Some(r), Some(c)) = (self.calendar_rect, self.calendar.as_mut()) else { return false };
        c.pointer(px - r.x, py - r.y, now)
    }

    /// A click while the month is open: its controls act, the rest of the
    /// sheet just takes the click.
    pub fn click_calendar(&mut self, px: f64, py: f64, left: bool, now: Time) -> bool {
        if !self.calendar_open() { return false; }
        let Some(r) = self.calendar_rect else { return true };
        if !left { return true; }
        let close = self.calendar.as_mut().is_some_and(|c| c.click(px - r.x, py - r.y, now));
        if close {
            self.activity_retiring = self.activity.take();
        }
        self.sync(now);
        true
    }

    /// The wheel over the open month turns the months.
    pub fn scroll_calendar(&mut self, px: f64, now: Time) -> bool {
        if !self.calendar_open() { return false; }
        if let Some(c) = &mut self.calendar { c.scroll(px, now); }
        self.sync(now);
        true
    }

    pub fn calendar_key(&mut self, key: &str, now: Time) -> bool {
        if !self.calendar_open() { return false; }
        let taken = self.calendar.as_mut().is_some_and(|c| c.key(key, now));
        if taken { self.sync(now); }
        true
    }

    /// A left click on the compact timer pauses or resumes it.
    pub fn toggle_timer(&mut self, now: Time) -> bool {
        let Some(timer) = self.timer.as_mut() else { return false };
        if timer.running { timer.pause(now); } else { timer.resume(now); }
        self.sync(now);
        true
    }

    pub fn click_timer_at(&mut self, px: f64, py: f64, now: Time) -> bool {
        if self.pill_open() || self.launcher.is_some() || self.hidden.any() {
            return false;
        }
        if !self.on_timer(px, py) {
            return false;
        }
        if !self.toggle_timer(now) {
            self.live_activity(activity::Event::TimerMenu, now);
        }
        true
    }

    /// Right click opens controls; left click on the running chip pauses.
    pub fn timer_menu_at(&mut self, px: f64, py: f64, now: Time) -> bool {
        if self.pill_open() || self.launcher.is_some() || self.hidden.any() {
            return false;
        }
        if !self.on_timer(px, py) {
            return false;
        }
        self.live_activity(activity::Event::TimerMenu, now);
        true
    }

    pub fn click_timer_menu(&mut self, px: f64, py: f64, now: Time) -> bool {
        if self.auth.is_some() || !self.activity.as_ref().is_some_and(|a| a.event == activity::Event::TimerMenu) {
            return false;
        }
        if let Some(r) = self.timer_menu_rect {
            let session = self.timer_session(now);
            if let Some(command) = self.timer_page.press(&session, px - r.x, py - r.y, now) {
                self.run_timer_page(command, now);
            }
            self.sync(now);
        }
        true
    }

    pub fn set_timer_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        let session = self.timer_session(now);
        let changed = match self.timer_menu_rect.filter(|_| self.timer_page_open()) {
            Some(r) => self.timer_page.pointer(&session, px - r.x, py - r.y, now),
            None => self.timer_page.leave(now),
        };
        self.timer_ticks();
        let hovered = self.on_timer(px, py);
        if hovered == self.timer_hovered {
            return changed;
        }
        self.timer_hovered = hovered;
        true
    }

    pub fn clear_timer_hover(&mut self, now: Time) -> bool {
        let changed = self.timer_page.leave(now) || self.timer_hovered;
        self.timer_hovered = false;
        changed
    }

    fn connections_open(&self) -> bool {
        self.auth.is_none() && self.activity.as_ref().is_some_and(|a| matches!(a.event,
            activity::Event::WifiOpen | activity::Event::BluetoothOpen))
    }

    pub fn open_connections(&mut self, kind: crate::network::Kind,
        tx: smithay_client_toolkit::reexports::calloop::channel::Sender<crate::network::Message>, now: Time) {
        if self.auth.is_some() { return; }
        if self.connections_open() && self.network_panel.as_ref().is_some_and(|p| p.kind == kind) {
            self.close_activity_panel(now);
            return;
        }
        self.hidden.user = false;
        self.quick_open = false;
        self.launcher = None;
        self.network_session += 1;
        self.network_panel = Some(crate::connections::Panel::new(kind, self.network_session, tx, now));
        self.live_activity(if kind == crate::network::Kind::Wifi { activity::Event::WifiOpen } else { activity::Event::BluetoothOpen }, now);
    }

    pub fn network_message(&mut self, message: crate::network::Message, now: Time) -> bool {
        if !self.connections_open() { return false; }
        self.network_panel.as_mut().is_some_and(|p| p.receive(message, now))
    }

    pub fn set_network_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        if !self.connections_open() { return false; }
        let Some(rect) = self.network_menu_rect else { return false };
        self.network_panel.as_mut().is_some_and(|p| p.pointer(px - rect.x, py - rect.y, now))
    }

    pub fn clear_network_hover(&mut self, now: Time) -> bool {
        self.network_panel.as_mut().is_some_and(|p| p.leave(now))
    }

    pub fn click_network_panel(&mut self, px: f64, py: f64, activate: bool, now: Time) -> bool {
        if !self.connections_open() { return false; }
        if !activate { return true; }
        if let (Some(rect), Some(panel)) = (self.network_menu_rect, &mut self.network_panel) {
            if panel.click(px - rect.x, py - rect.y, now) {
                self.close_activity_panel(now);
                self.return_to_quick_menu(now);
            }
        }
        true
    }

    /// Typing into the reply field of an open notification in the center.
    pub fn center_reply_key(&mut self, key: &str, text: Option<&str>, ctrl: bool, now: Time) -> Option<notice_center::Action> {
        if !self.center.open { return None; }
        self.center.reply_key(key, text, ctrl, now)
    }

    pub fn network_key(&mut self, key: &str, text: Option<&str>, ctrl: bool, now: Time) -> bool {
        if !self.connections_open() { return false; }
        if let Some(panel) = &mut self.network_panel { panel.key(key, text, ctrl, now); }
        true
    }

    pub fn scroll_network(&mut self, rows: f64, now: Time) -> bool {
        if !self.connections_open() { return false; }
        if let Some(panel) = &mut self.network_panel { panel.scroll_by(rows, now); }
        true
    }

    /// fprintd started a verification.
    pub fn auth_started(&mut self, now: Time) {
        // a password prompt outranks the launcher
        self.launcher = None;
        self.quick_open = false;
        self.quick_retiring_at = None;
        self.center.close();
        if let Some(activity) = self.activity.take() {
            if !activity::takes_keyboard(&activity.event) { self.activity_queued = Some(activity.event); }
        }
        self.activity_retiring = None;
        // a new attempt while the pill is still up (after a miss): back to
        // waiting, no close / reopen
        if self.auth.is_none() {
            self.auth_start = now;
        }
        self.auth = Some(Auth::Waiting(now));
        self.sync(now);
    }

    /// fprintd VerifyStatus.
    pub fn auth_status(&mut self, status: &str, done: bool, now: Time) {
        if self.auth.is_none() {
            return;
        }
        let result = match status {
            "verify-match" => Some(true),
            "verify-no-match" => Some(false),
            // retry hints: keep waiting
            s if s.starts_with("verify-retry") || s.starts_with("verify-swipe") || s.starts_with("verify-finger") => None,
            _ if done => Some(false),
            _ => None,
        };
        match result {
            Some(true) => self.auth = Some(Auth::Ok(now)),
            Some(false) => self.auth = Some(Auth::Fail(now)),
            None => return,
        }
        if self.config.edge_light {
            use crate::edge::{Wave, FAILURE_RED, SUCCESS_GREEN};
            let (wave, colour) = if result == Some(true) { (Wave::Flash, SUCCESS_GREEN) } else { (Wave::Shudder, FAILURE_RED) };
            self.edge.pulse(wave, colour, now);
        }
        self.sync(now);
    }

    pub fn update_clock(&mut self) -> bool {
        let text = strftime(&self.config.clock);
        if text != self.clock {
            self.clock = text;
            true
        } else {
            false
        }
    }

    /// Draw one frame on `cr` (already scaled to logical px), the island
    /// centred on `cx`, for the instant `now` (when the frame will be shown).
    pub fn draw(&mut self, cr: &Context, renderer: &Renderer, cx: f64, now: Time) -> Frame {
        renderer.motion.set_time(now);
        self.surface_cx = cx;
        // equaliser easing: quick to follow the music, slow to sink when
        // paused
        let dt = (now - self.eq_t).clamp(0.0, 0.5);
        self.eq_t = now;
        let playing = self.song.as_ref().is_some_and(|s| s.playing());
        let tau = if playing { EQ_TAU } else { EQ_TAU_SINK };
        let k = 1.0 - (-dt / tau).exp();
        for (h, t) in self.eq.iter_mut().zip(&self.eq_target) {
            *h += (*t - *h) * k;
        }

        let moving = self.is_animating(now);
        let hide = self.hide.state(now);
        let dodge = self.dodge.state(now);
        let hover_pad = self.hover_pad.value(now);
        let reveal = self.reveal.state(now).x * self.music_chip.state(now).x;
        let q = self.pill.state(now).x;
        // the pill, the player, the editor's panel and the launcher are the
        // shapes the box can take besides the plain line; they never overlap
        let p = self.panel.state(now).x;
        let pl = self.player.state(now).x;
        let ed = self.editor_panel.state(now).x;
        // the spread lags behind the slide — most of it happens while the box
        // is already mostly gone, like a drop flattening on water; it is a
        // function of the position so that show and hide trace the same path
        let spread_k = inv_min_jerk(hide.x).powf(2.2);

        // the panel stands on its own: while it is open the island's own
        // hiding and dodging do not move it, so mod+d works with the island
        // parked above the edge and the panel folds back out of sight
        let cx = cx - self.config.dodge * dodge.x * (1.0 - p);
        let dy = -BOX_H * hide.x * (1.0 - p);
        if dy <= -(BOX_H + FILLET) && p <= 0.0 {
            self.box_geom = None;
            self.tug_top = None;
            return Frame::default();
        }
        // in the hand (or wobbling back) the closed line gives a little:
        // it slides along the edge and leans, stretches down like a drop
        // about to fall, or squashes into the edge; any other shape the box
        // takes shakes it off
        let line_k = ((1.0 - q) * (1.0 - p) * (1.0 - pl) * (1.0 - ed)).clamp(0.0, 1.0);
        let (tug_x, tug_y) = self.tug_now(now);
        let (tug_x, tug_y) = (tug_x * line_k, tug_y * line_k);
        let tugged = tug_x.abs() > 0.01 || tug_y.abs() > 0.01;
        // the outline hangs from the edge; the content rides in its middle
        let edge = dy;
        let cx = cx + tug_x * (1.0 - TUG_LEAN / 2.0);
        let dy = dy + tug_y / 2.0;
        let pad = PAD + (PAD_HOVER - PAD) * hover_pad + SPREAD * spread_k;
        let radius = RADIUS + (RADIUS_HIDDEN - RADIUS) * hide.x;
        if self.center.open {
            self.center_retiring_at = None;
        } else if self.center_was_open {
            self.center_retiring_at = Some(now.max(self.fold_at));
        }
        self.center_was_open = self.center.open;
        // a sheet keeps its 30 px corners until another kind of pill takes
        // the box; switching to a capsule's radius the instant a sheet
        // closed ballooned the folding sheet into an oval
        if self.quick_open || self.center.open || self.visible_activity().is_some_and(|a| activity::takes_keyboard(&a.event)) {
            self.sheet_corners = true;
        } else if self.auth.is_some() || self.notice.is_some() || self.toast.is_some() || self.visible_activity().is_some() {
            self.sheet_corners = false;
        }
        let pill_radius = if self.sheet_corners { 30.0 } else { self.pill_h.state(now).x / 2.0 };
        let radius = radius
            + (pill_radius - radius) * q
            + (PLAYER_R - radius) * pl
            + (editor::PANEL_R - radius) * ed
            + (launcher::PANEL_R - radius) * p;

        // "liquid": the box stretches along its direction of travel in
        // proportion to its speed (px/s) and relaxes as it stops
        let liquid = self.config.liquid;
        let stretch_y = liquid * 0.02 * (hide.v * BOX_H).abs();
        let stretch_x = liquid * 0.01 * (dodge.v * self.config.dodge).abs();

        // --- the line: the clock, with the bars beating beside it while
        // something plays (the track's name lives in the toast and in the
        // player now, not here) ---
        let clock = renderer.layout(cr, &self.clock, Font::Text);
        let (_, clock_ext) = clock.pixel_extents();
        let clock_target = clock_ext.width() as f64;
        let clock_w = match &mut self.clock_w {
            Some(spring) => {
                spring.set(clock_target, now);
                spring.value(now)
            }
            None => {
                self.clock_w = Some(Spring::new(clock_target, crate::motion::DURATION, 1.0));
                clock_target
            }
        };
        let clock_h = clock_ext.height() as f64;
        let clock_k = self.clock_chip.state(now).x;
        let clock_len = clock_k * clock_w;
        let bars_len = reveal * (BARS_GAP + BARS_W);
        // Neovim's mode chip grows out on the other side of the clock
        let chip_k = self.chip.state(now).x;
        if let Some(e) = &self.nvim {
            self.chip_w = editor::chip_width(cr, renderer, e);
        }
        let chip_len = chip_k * (self.chip_w + editor::GAP);
        let timer_k = self.timer_chip.state(now).x;
        if self.timer.is_some() { self.timer_last = self.timer.clone(); }
        if let Some(timer) = self.timer.as_ref().or(self.timer_last.as_ref()) {
            self.timer_w = activity::timer_width(cr, renderer, timer.reading(now));
        }
        let timer_len = timer_k * (self.timer_w + 10.0);
        let tasks_k = self.tasks_chip.state(now).x;
        if !self.tasks.is_empty() {
            self.tasks_w = self.tasks.chip_width(cr, renderer, now);
        }
        let tasks_len = tasks_k * (self.tasks_w + 10.0);
        let record_k = self.record_chip.state(now).x;
        let record_time = self.recording_seconds(now).map(|s| format!("{}:{:02}", s / 60, s % 60));
        if let Some(text) = &record_time {
            let reserved: String = text.chars().map(|c| if c.is_ascii_digit() { '8' } else { c }).collect();
            self.record_w = 16.0 + renderer.layout(cr, &reserved, Font::Small).pixel_extents().1.width() as f64;
        }
        let record_len = record_k * (self.record_w + 10.0);
        self.record_rect = None;
        self.tasks_rect = None;
        self.tasks_pill_rect = None;
        self.calendar_rect = None;
        self.clock_rect = None;
        self.line_rect = None;
        self.timer_rect = None;
        self.timer_mark_rect = None;
        self.cover_rect = None;
        self.timer_menu_rect = None;
        self.network_menu_rect = None;
        self.quick_rect = None;
        self.center_rect = None;
        self.notice_rect = None;
        self.eq_rect = None;
        self.player_rect = None;
        self.editor_chip_rect = None;
        let shown = self.config.compact;
        let unread_len = if shown.unread && self.center.unread() > 0 { 18.0 } else { 0.0 };
        let dnd_len = if shown.dnd && self.dnd { 21.0 } else { 0.0 };
        let mic_k = if shown.mic { self.mic_dot.value(now).clamp(0.0, 1.2) } else { 0.0 };
        let mic_len = mic_k.min(1.0) * 16.0;
        // --- without the clock: the wings (see COVER) ---
        let wing = 1.0 - clock_k;
        if let Some(song) = &self.song {
            if self.wing_song.0 != song.now.title || self.wing_song.1 != song.now.art_url {
                self.wing_song = (song.now.title.clone(), song.now.art_url.clone());
            }
        }
        let cover_k = wing * reveal;
        let cover_len = cover_k * (COVER + COVER_GAP);
        let ring_len = wing * timer_k * RING_LEN;
        let timer_len = timer_len - ring_len;
        let title_fit = if self.wing_song.0.is_empty() {
            REST_CONTENT_W
        } else {
            let w = renderer.layout(cr, &self.wing_song.0, Font::Small).pixel_extents().1.width() as f64;
            (w + 28.0).clamp(REST_CONTENT_W, MIDDLE_MAX)
        };
        let title_fit = match &mut self.middle_w {
            Some(spring) => {
                spring.set(title_fit, now);
                spring.value(now)
            }
            None => {
                self.middle_w = Some(Spring::new(title_fit, crate::motion::DURATION, 1.0));
                title_fit
            }
        };
        let middle_len = clock_len + wing * (REST_CONTENT_W + (title_fit - REST_CONTENT_W) * reveal);
        let left_len = chip_len + cover_len + ring_len;
        let right_len = timer_len + tasks_len + record_len + bars_len + unread_len + dnd_len + mic_len;
        // the shorter wing keeps its items at its end and leaves the room
        // beside the middle, so a song's title stays centred; only a little
        // room, though: a wide editor chip must not leave a long empty wing
        let balance = wing * reveal;
        let left_pad = balance * (right_len - left_len).clamp(0.0, WING_BALANCE);
        let right_pad = balance * (left_len - right_len).clamp(0.0, WING_BALANCE);
        let content_w = left_len + left_pad + middle_len + right_pad + right_len;
        // without the clock the island keeps a resting width of its own,
        // a short black capsule, rather than shrinking to its padding
        let line_w = content_w.max(REST_CONTENT_W) + 2.0 * pad;

        // --- the pill: Touch ID, a live activity, or a notification ---
        let activity_size = self.center.open.then_some((notice_center::WIDTH, self.center.height()))
            .or_else(|| self.quick_open.then_some((quick::WIDTH, quick::height(self.quick_view, &self.controls)))).or_else(|| self.visible_activity().map(|a| {
            if matches!(a.event, activity::Event::WifiOpen | activity::Event::BluetoothOpen) {
                self.network_panel.as_ref().map_or(a.event.size(), |p| (crate::connections::WIDTH, p.height()))
            } else if a.event == activity::Event::Tasks {
                self.tasks.pill_size()
            } else if a.event == activity::Event::CalendarOpen {
                self.calendar.as_ref().map_or(a.event.size(), |c| (calendar::WIDTH, c.height()))
            } else if a.event == activity::Event::TimerMenu {
                (timer_page::WIDTH, self.timer_page.height(&self.timer_session(now)))
            } else { a.event.size() }
        }));
        let notice_size = self.notice.as_ref().map(|n| {
            let (w, h) = renderer.notice_size(cr, &n.note);
            // replying, the action buttons give way to the reply field
            if self.notice_reply.as_ref().is_some_and(|r| r.id == n.note.id) {
                let actions = if n.note.actions.iter().any(|(key, _)| key != "default") { 39.0 } else { 0.0 };
                (w.max(NOTICE_REPLY_W), h - actions + REPLY_H)
            } else {
                (w, h)
            }
        });
        let toast_size = self.song.as_ref().filter(|_| self.toast.is_some()).map(|song| {
            let view = song.view(&self.eq, self.config.accent, None, None, now);
            (player::toast_width(cr, renderer, &view), player::TOAST_H)
        });
        let (w_target, h_target) = match (self.auth.is_some(), activity_size.or(notice_size).or(toast_size)) {
            (true, _) => (PILL_MIN_W, PILL_H),
            (false, Some(size)) => size,
            (false, None) => (self.pill_w.state(now).x, self.pill_h.state(now).x),
        };
        if q <= 0.0 {
            // closed: be at the size the next pill will need, without
            // animating there (and without restarting the tween every
            // frame — that kept the island redrawing forever)
            if (self.pill_w.state(now).x - w_target).abs() > 0.01 {
                self.pill_w.set_at(w_target, now);
            }
            if (self.pill_h.state(now).x - h_target).abs() > 0.01 {
                self.pill_h.set_at(h_target, now);
            }
        } else {
            self.pill_w.set(w_target, now);
            self.pill_h.set(h_target, now);
        }
        let pill_w = self.pill_w.state(now).x;
        let pill_h = self.pill_h.state(now).x;
        let line_h = BOX_H;
        let bw = line_w
            + (pill_w - line_w) * q
            + (player::PLAYER_W - line_w) * pl
            + (editor::PANEL_W - line_w) * ed
            + (launcher::PANEL_W - line_w) * p
            + stretch_x;
        let bh = line_h
            + (pill_h - line_h) * q
            + (player::PLAYER_H - line_h) * pl
            + (self.editor_height() - line_h) * ed
            + (launcher::PANEL_H - line_h) * p
            + stretch_y
            - stretch_x * 0.15
            + tug_y / 2.0;
        // squashed it widens; stretched it thins (a drop keeps its volume),
        // but only its outline, into the padding: the content keeps its room
        let bw = if tug_y < 0.0 { bw - 1.6 * tug_y } else { bw };
        let thin = (0.22 * tug_y.max(0.0)).min(7.0);
        let (mut x0, mut x1, mut y1) = (cx - bw / 2.0, cx + bw / 2.0, dy + bh);
        if !moving && !tugged {
            // crisp edges at rest; while moving they glide sub-pixel
            x0 = renderer.snap(x0);
            x1 = renderer.snap(x1);
            y1 = renderer.snap(y1);
        }
        let r = radius.min((x1 - x0) / 2.0).min((y1 - dy) / 2.0);
        // the light along the frame comes out of the box as it is now
        self.box_geom = (y1 > 0.5).then(|| (Rect { x: x0, y: edge.max(0.0), w: x1 - x0, h: y1 - edge.max(0.0) }, r));
        if pl > 0.05 && self.player.target() > 0.5 && self.song.is_some() {
            self.player_rect = Some(Rect { x: x0, y: dy, w: x1 - x0, h: y1 - dy });
        }

        self.tug_top = None;
        if tugged {
            // the top lags behind the bottom: a shear about the middle;
            // the fillets draw out into a neck as it is pulled down, from
            // the frame's edge as the pull has bent it (the membrane, drawn
            // with the frame marks, runs level over the island and leaves
            // the fillets' ends flat, so they meet without a corner)
            cr.save().unwrap();
            let mid = (edge + y1) / 2.0;
            let k = TUG_LEAN * tug_x / (y1 - edge).max(1.0);
            let sag = if self.config.edge_light { TUG_SAG * tug_y.max(0.0) } else { 0.0 };
            let top = edge + sag;
            let fillet = FILLET + 0.45 * tug_y.max(0.0);
            cr.transform(cairo::Matrix::new(1.0, 0.0, k, 1.0, -k * mid, 0.0));
            self.trace_notch_with(cr, x0 + thin, x1 - thin, top, y1, r, fillet);
            cr.restore().unwrap();
            self.tug_top = Some(((x0 + x1) / 2.0 + k * (top - mid), (x1 - x0) / 2.0 - thin + fillet, sag));
        } else {
            self.trace_notch(cr, x0, x1, dy, y1, r);
        }
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.fill();
        // Low battery is intentionally an OSD, not a system notification:
        // the whole pill takes on the warning colour.
        if self.auth.is_none() && self.visible_activity().is_some_and(|a| a.event.is_low_battery()) {
            self.trace_notch(cr, x0, x1, dy, y1, r);
            cr.set_source_rgba(0.42, 0.015, 0.02, 0.82 * pill_alpha_of(q));
            let _ = cr.fill();
        }
        // a critical notification puts a breathing rim around the pill
        if let Some(n) = self.notice.as_ref().filter(|n| n.note.critical) {
            let c = icons::accent(n.note.kind);
            let breathe = 0.45 + 0.35 * (0.5 - 0.5 * (2.0 * std::f64::consts::PI * (now - n.since) / 1.1).cos());
            self.trace_notch(cr, x0 + 0.75, x1 - 0.75, dy, y1 - 0.75, r);
            cr.set_line_width(1.5);
            cr.set_source_rgba(c.0, c.1, c.2, breathe * pill_alpha_of(q));
            let _ = cr.stroke();
        }
        // the floating buttons drip out of the sides of an open page
        let float_k = self.floats.value(now);
        let float_page = floating::Body { x0, x1, top: dy, bottom: y1 };
        self.float_body = None;
        // going in, a button tucked behind the edge is gone: the page may
        // fold shorter than the column, and a tucked drop would hang below
        let tucked = self.floats.target() < 0.5 && float_k <= FLOAT_TUCKED;
        if float_k > 0.001 && !tucked {
            for button in floating::ALL {
                let look = floating::Look {
                    progress: float_k,
                    lit: self.float_lit.get(&button).map_or(0.0, |s| s.value(now)),
                    selected: self.float_selected(button),
                    pressed: self.float_pressed.filter(|(b, _)| *b == button).map_or(f64::INFINITY, |(_, t)| now - t),
                };
                floating::draw(cr, button, float_page, &look, 1.0);
            }
            if float_k > 0.95 && self.floats.target() > 0.5 {
                self.float_body = Some(float_page);
            }
        }
        // everything inside stays inside (the pill content climbs in from
        // below the box): a pixel-aligned rectangle, which cairo clips for
        // free, is enough — the content never reaches the corners
        cr.save().unwrap();
        if q > 0.001 && q < 0.999 {
            // while the pill morphs, its content may still be larger than
            // the box: clip to the real outline so nothing pokes past the
            // rounded corners (a mask only for these few frames)
            self.trace_notch(cr, x0, x1, dy, y1, r);
        } else {
            let (cx0, cy0) = (renderer.floor(x0), renderer.floor(dy));
            cr.rectangle(cx0, cy0, renderer.ceil(x1) - cx0, renderer.ceil(y1) - cy0);
        }
        cr.clip();

        // the line's content rides at the top of the box and dissolves
        // while the pill opens (first third), the pill's content climbs in
        // from below and fades in over the rest
        let line_alpha = (1.0 - smoothstep(q / PILL_SWAP)) * (1.0 - smoothstep(pl / 0.45))
            * (1.0 - smoothstep(ed / 0.45))
            * if self.hidden.any() && self.launcher.is_none() { 0.0 } else { 1.0 };
        let pill_alpha = pill_alpha_of(q);
        let sheet_k = self.quick_retiring_at.map_or(1.0, |since| min_jerk(((now - since) / SHEET_SWAP).clamp(0.0, 1.0)));
        if line_alpha > 0.95 && !self.pill_open() {
            self.line_rect = Some(Rect { x: x0, y: dy, w: x1 - x0, h: y1 - dy });
        }
        if line_alpha > 0.0 {
            let content_x = cx - content_w / 2.0;
            if chip_len > 1.0 {
                if chip_k > 0.95 && line_alpha > 0.95 && ed < 0.05 && self.nvim.is_some() {
                    self.editor_chip_rect = Some(Rect { x: content_x, y: dy, w: chip_len, h: line_h });
                }
                if let Some(e) = &self.nvim {
                    cr.save().unwrap();
                    cr.rectangle(content_x, dy, chip_len, line_h);
                    cr.clip();
                    let saved = self.nvim_saved.map(|t| now - t).filter(|t| *t < editor::SAVE_FLASH);
                    editor::draw_chip(cr, renderer, e, content_x, dy + line_h / 2.0, self.chip_w, line_alpha * chip_k, saved, now);
                    cr.restore().unwrap();
                }
            }
            // the left wing: the song's cover, then the timer's ring
            let cover_x = content_x + chip_len;
            let ring_x = cover_x + cover_len;
            if cover_len > 0.5 {
                if cover_k > 0.95 && line_alpha > 0.95 && pl < 0.05 && self.song.is_some() {
                    self.cover_rect = Some(Rect { x: cover_x - 5.0, y: dy, w: COVER + 10.0, h: line_h });
                }
                cr.save().unwrap();
                cr.rectangle(cover_x, dy, cover_len, line_h);
                cr.clip();
                // it opens from its middle, as the clock closes to its own
                let room = cover_len * COVER / (COVER + COVER_GAP);
                let mut x = cover_x + (room - COVER) / 2.0;
                if cover_k > 0.999 {
                    x = renderer.snap(x);
                }
                let y = renderer.snap(dy + (line_h - COVER) / 2.0);
                player::art_tile(cr, renderer, &self.art, &self.wing_song.1, x, y, COVER, COVER_R, line_alpha * cover_k * cover_k);
                cr.restore().unwrap();
            }
            let slot_x = ring_x + ring_len + left_pad;
            if self.song.is_some() || reveal > 0.001 {
                // the song names itself in the middle, where the clock was
                let k = wing * reveal;
                if k > 0.01 && !self.wing_song.0.is_empty() {
                    cr.save().unwrap();
                    cr.rectangle(slot_x, dy, middle_len, line_h);
                    cr.clip();
                    let layout = renderer.layout(cr, "Ag", Font::Small);
                    let (_, ext) = layout.pixel_extents();
                    let inner = (middle_len - 24.0).max(4.0);
                    let ty = renderer.snap(dy + line_h / 2.0 - ext.height() as f64 / 2.0 - ext.y() as f64);
                    crate::motion::text(cr, renderer, "wing-title", &self.wing_song.0, slot_x + 12.0, ty + 3.0 * (1.0 - k),
                        inner, 0.5, Font::Small, (1.0, 1.0, 1.0), 0.86 * line_alpha * smoothstep((k - 0.45) / 0.55));
                    cr.restore().unwrap();
                }
            }
            let clock_room = slot_x + (middle_len - clock_len) / 2.0;
            let clock_x = renderer.snap(clock_room - clock_ext.x() as f64);
            let ty = renderer.snap(dy + (line_h - clock_h) / 2.0 - clock_ext.y() as f64);
            if clock_k > 0.001 {
                // leaving, the clock shrinks toward its middle as its room closes
                cr.save().unwrap();
                cr.rectangle(clock_room, dy, clock_len, line_h);
                cr.clip();
                let squeeze = (clock_w - clock_len) / 2.0;
                crate::motion::text(cr, renderer, "clock", &self.clock, clock_x - squeeze, ty,
                    clock_w.max(clock_target) + clock_ext.x() as f64 + 8.0, 0.0, Font::Text, (1.0, 1.0, 1.0), line_alpha * clock_k * clock_k);
                cr.restore().unwrap();
            }
            if line_alpha > 0.95 && clock_k > 0.95 && !self.pill_open() {
                self.clock_rect = Some(Rect { x: clock_x + clock_ext.x() as f64 - 6.0, y: dy, w: clock_w + 12.0, h: line_h });
            }
            let mut after_clock = clock_x + clock_ext.x() as f64 + clock_len + (middle_len - clock_len) / 2.0 + right_pad;
            if timer_len > 0.5 {
                if self.timer.is_some() && line_alpha > 0.95 && timer_k > 0.95 && !self.pill_open() {
                    let w = self.timer_w + 8.0 - 30.0 * wing;
                    self.timer_rect = Some(Rect { x: after_clock + 6.0, y: dy, w, h: line_h });
                    if wing > 0.95 {
                        self.timer_mark_rect = Some(Rect { x: ring_x - 6.0, y: dy, w: 32.0, h: line_h });
                    }
                }
                if let Some(timer) = self.timer.as_ref().or(self.timer_last.as_ref()) {
                    let cy = dy + line_h / 2.0;
                    // the ring crosses to the left wing as the clock leaves
                    let ring_at = after_clock + 10.0 + (ring_x - after_clock - 10.0) * wing;
                    cr.save().unwrap();
                    cr.rectangle(ring_at - 2.0, dy, 24.0 * timer_k, line_h);
                    cr.clip();
                    // crossing over the leaving clock it dims, so the two never tangle
                    let crossing = 1.0 - 0.85 * (std::f64::consts::PI * wing).sin();
                    activity::draw_timer_ring(cr, ring_at, cy, timer.urgent(now), timer.fraction(now), timer.running,
                        line_alpha * timer_k * crossing);
                    cr.restore().unwrap();
                    cr.save().unwrap();
                    cr.rectangle(after_clock, dy, timer_len, line_h);
                    cr.clip();
                    let reading_x = after_clock + 10.0 + 30.0 * (1.0 - wing);
                    activity::draw_timer_reading(cr, renderer, reading_x, cy, timer.reading(now), timer.urgent(now), wing,
                        line_alpha * timer_k);
                    cr.restore().unwrap();
                }
                after_clock += timer_len;
            }
            if tasks_len > 0.5 {
                if !self.tasks.is_empty() && line_alpha > 0.95 && tasks_k > 0.95 && !self.pill_open() {
                    self.tasks_rect = Some(Rect { x: after_clock + 6.0, y: dy, w: self.tasks_w + 8.0, h: line_h });
                }
                cr.save().unwrap();
                cr.rectangle(after_clock, dy, tasks_len, line_h);
                cr.clip();
                self.tasks.draw_chip(cr, renderer, after_clock + 10.0, dy + line_h / 2.0, line_alpha * tasks_k, now);
                cr.restore().unwrap();
                after_clock += tasks_len;
            }
            if record_len > 0.5 {
                if record_k > 0.95 && line_alpha > 0.95 && !self.pill_open() {
                    self.record_rect = Some(Rect { x: after_clock + 4.0, y: dy, w: self.record_w + 10.0, h: line_h });
                }
                cr.save().unwrap();
                cr.rectangle(after_clock, dy, record_len, line_h);
                cr.clip();
                // the red dot, and the recording's length beside it
                let (mx, my) = (after_clock + 15.0, dy + line_h / 2.0);
                let red = crate::menu_style::RED;
                let stopping = matches!(self.recording, Some(Recording::Stopping(_)));
                cr.new_path();
                cr.arc(mx, my, 5.0, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(red.0, red.1, red.2, line_alpha * record_k * if stopping { 0.5 } else { 1.0 });
                let _ = cr.fill();
                if let Some(text) = &record_time {
                    let layout = renderer.layout(cr, text, Font::Small);
                    let (_, ext) = layout.pixel_extents();
                    crate::motion::text(cr, renderer, "record-time", text, mx + 11.0 - ext.x() as f64,
                        my - ext.height() as f64 / 2.0 - ext.y() as f64, ext.width() as f64 + 30.0, 0.0, Font::Small,
                        red, line_alpha * record_k);
                }
                cr.restore().unwrap();
                after_clock += record_len;
            }
            if reveal > 0.0 {
                // the bars grow out of the clock's right edge
                let bx = after_clock + BARS_GAP * reveal;
                if reveal > 0.95 && line_alpha > 0.95 && pl < 0.05 && self.song.is_some() {
                    self.eq_rect = Some(Rect { x: bx - 5.0, y: dy, w: BARS_W + 10.0, h: line_h });
                }
                cr.save().unwrap();
                cr.rectangle(bx, dy, BARS_W * reveal, line_h);
                cr.clip();
                player::bars(cr, renderer, &self.eq, self.config.accent, bx, dy + (line_h - BARS_H) / 2.0, BARS_W, BARS_H, line_alpha * reveal);
                cr.restore().unwrap();
            }
            if unread_len > 0.0 {
                let bx = after_clock + bars_len + 13.0;
                cr.arc(bx, dy + line_h / 2.0, 2.7, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(0.40, 0.82, 1.0, line_alpha);
                let _ = cr.fill();
            }
            if dnd_len > 0.0 {
                let mx = after_clock + bars_len + unread_len + 12.0;
                let my = dy + line_h / 2.0;
                cr.save().unwrap();
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                cr.arc(mx, my, 5.5, 0.0, std::f64::consts::TAU);
                cr.new_sub_path();
                cr.arc(mx + 3.0, my - 2.0, 5.2, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(0.72, 0.64, 1.0, line_alpha);
                let _ = cr.fill();
                cr.restore().unwrap();
            }
            if mic_len > 0.3 {
                // the privacy dot swells in (a touch past full) and shrinks away
                let mx = after_clock + bars_len + unread_len + dnd_len + 10.5;
                let orange = crate::menu_style::ORANGE;
                cr.new_path();
                cr.arc(mx, dy + line_h / 2.0, 3.3 * mic_k, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(orange.0, orange.1, orange.2, line_alpha * mic_k.min(1.0));
                let _ = cr.fill();
            }
        }


        // arriving, the pill's content grows into place from 0.86 of its
        // size around the box's top middle as it climbs and fades in
        // (Vorssaint's arrival); leaving, it shrinks back the same way
        let arrive = ARRIVE_SCALE + (1.0 - ARRIVE_SCALE) * pill_alpha;
        let arriving = pill_alpha > 0.0 && arrive < 0.9999;
        if arriving {
            cr.save().unwrap();
            cr.translate(cx, dy);
            cr.scale(arrive, arrive);
            cr.translate(-cx, -dy);
        }
        if pill_alpha > 0.0 {
            if let Some(auth) = self.auth {
                let (result, caption, since) = match auth {
                    Auth::Waiting(since) => (None, "Touch ID", since),
                    Auth::Ok(since) => (Some((touchid::Mark::Check, crate::menu_style::GREEN)), "Touch ID", since),
                    Auth::Fail(since) => (Some((touchid::Mark::Cross, crate::menu_style::RED)), "Повторите попытку", since),
                };
                if result.is_some() {
                    self.last_mark = result;
                }
                // the breathing runs from the start of the verification; the
                // result colour takes over as the ring forms (and lets go
                // again as it splits back into the fingerprint)
                let pulse_t = now - self.auth_start;
                let morph = self.ring.state(now).x;
                let ripple = |i: usize| pulse_colour(pulse_t - 0.07 * i as f64);
                let shown = result.or(self.last_mark);
                let kind = shown.map(|(k, _)| k);
                let colour = match shown {
                    Some((_, c)) => mix(pulse_colour(pulse_t), c, smoothstep(morph / 0.6)),
                    None => pulse_colour(pulse_t),
                };
                let t = now - since;
                // success: a little bounce as the ring fills; a miss: the
                // icon shakes its head
                let scale = match auth {
                    Auth::Ok(_) if t > 0.25 => 1.0 + 0.14 * (-(t - 0.25) / 0.12).exp() * (2.0 * std::f64::consts::PI * (t - 0.25) / 0.35).sin(),
                    _ => 1.0,
                };
                let shake = match auth {
                    Auth::Fail(_) => 7.0 * (-t / 0.18).exp() * (2.0 * std::f64::consts::PI * t / 0.13).sin(),
                    _ => 0.0,
                };
                // the fill comes up under the last stretch of the morph; on
                // the way back (a new attempt) it leaves with the mark
                let mark = self.mark.state(now).x;
                let mut fill = ((morph - 0.6) / 0.4).clamp(0.0, 1.0);
                if result.is_none() {
                    fill *= mark;
                }
                let icon = touchid::IconState {
                    morph,
                    fill,
                    mark,
                    kind,
                    colour,
                    ridge_colour: Some(&ripple),
                    alpha: pill_alpha,
                    scale,
                    shake,
                };
                let cap_l = renderer.layout(cr, caption, Font::Caption);
                let (_, c_ext) = cap_l.pixel_extents();
                let total = ICON_SIZE + PILL_GAP + c_ext.height() as f64;
                let climb = (1.0 - pill_alpha) * PILL_CLIMB;
                let top = dy + (bh - total) / 2.0 + climb;
                touchid::draw(cr, cx, top + ICON_SIZE / 2.0, ICON_SIZE, &icon);
                let cxp = renderer.snap(cx - c_ext.width() as f64 / 2.0 - c_ext.x() as f64);
                let cy = renderer.snap(top + ICON_SIZE + PILL_GAP - c_ext.y() as f64);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.8 * pill_alpha);
                cr.move_to(cxp, cy);
                pangocairo::functions::show_layout(cr, &cap_l);
            } else if self.center.open {
                let climb = (1.0 - pill_alpha) * PILL_CLIMB;
                let x = cx - notice_center::WIDTH / 2.0;
                let enter = 8.0 * (1.0 - sheet_k);
                if pill_alpha > 0.95 && sheet_k > 0.9 {
                    self.center_rect = Some(Rect { x, y: dy + climb + enter, w: notice_center::WIDTH, h: self.center.height() });
                }
                self.center.draw(cr, renderer, &self.launcher_icons, x, dy + climb + enter, pill_alpha * sheet_k, now);
            } else if self.quick_open {
                let climb = (1.0 - pill_alpha) * PILL_CLIMB;
                let arrive = self.quick_returned_at.map_or(1.0, |since| min_jerk(((now - since) / SHEET_SWAP).clamp(0.0, 1.0)));
                let enter = 8.0 * (1.0 - arrive);
                if pill_alpha > 0.95 && arrive > 0.9 {
                    self.quick_rect = Some(Rect { x: cx - quick::WIDTH / 2.0, y: dy + climb + enter, w: quick::WIDTH,
                        h: quick::height(self.quick_view, &self.controls) });
                }
                let springs = &self.quick_hover_springs;
                let hover = |action: quick::Action| springs.get(&action).map_or(0.0, |s| s.value(now));
                let (shown, grown, icons) = (&self.slider_shown, &self.slider_grow, &self.launcher_icons);
                let level = |slider: quick::Slider| shown.get(&slider).map_or(0.0, |s| s.value(now).clamp(0.0, 1.0));
                let grow = |slider: quick::Slider| grown.get(&slider).map_or(0.0, |s| s.value(now));
                let icon = |name: &str, px: i32| if name.is_empty() { None } else { icons.get_scaled(name, px) };
                let (pressed_at, bounced_at) = (&self.quick_pressed, &self.quick_bounced);
                let pressed = |action: quick::Action| pressed_at.get(&action).map_or(f64::INFINITY, |t| now - t);
                let bounced = |action: quick::Action| bounced_at.get(&action).map_or(f64::INFINITY, |t| now - t);
                let frame = quick::Frame {
                    status: self.quick_status(now),
                    controls: &self.controls,
                    level: &level,
                    grow: &grow,
                    icon: &icon,
                    art: &self.art,
                    hover: &hover,
                    logout_armed: self.logout_armed_at.is_some(),
                    now,
                    opened: self.quick_opened_at,
                    pressed: &pressed,
                    bounced: &bounced,
                };
                let (qx, qy) = (cx - quick::WIDTH / 2.0, dy + climb + enter);
                let close = self.quick_close_hover.value(now);
                match self.quick_view_from {
                    // a page switch: the old page lifts away as the new one settles
                    Some((from, since)) if now - since < SHEET_SWAP => {
                        let k = min_jerk(((now - since) / SHEET_SWAP).clamp(0.0, 1.0));
                        quick::draw(cr, renderer, qx, qy - 8.0 * k, pill_alpha * arrive * (1.0 - k), from, close, &frame);
                        quick::draw(cr, renderer, qx, qy + 8.0 * (1.0 - k), pill_alpha * arrive * k, self.quick_view, close, &frame);
                    }
                    _ => quick::draw(cr, renderer, qx, qy, pill_alpha * arrive, self.quick_view, close, &frame),
                }
            } else if let Some(activity) = self.visible_activity().cloned() {
                let climb = (1.0 - pill_alpha) * PILL_CLIMB;
                let enter = 8.0 * (1.0 - sheet_k);
                let content_alpha = pill_alpha * sheet_k;
                if activity.event == activity::Event::TimerMenu {
                    if self.timer_page.opened != activity.since {
                        self.timer_page.open(activity.since);
                    }
                    let session = self.timer_session(now);
                    let mx = cx - timer_page::WIDTH / 2.0;
                    if content_alpha > 0.9 {
                        let h = self.timer_page.height(&session);
                        self.timer_menu_rect = Some(Rect { x: mx, y: dy + climb + enter, w: timer_page::WIDTH, h });
                    }
                    self.timer_page.draw(cr, renderer, mx, dy + climb + enter, &session, content_alpha, now);
                }
                if activity.event == activity::Event::CalendarOpen {
                    let (w, x) = (calendar::WIDTH, cx - calendar::WIDTH / 2.0);
                    if content_alpha > 0.9 {
                        self.calendar_rect = Some(Rect { x, y: dy + climb + enter, w, h: bh });
                    }
                    if let Some(c) = &self.calendar {
                        c.draw(cr, renderer, x, dy + climb + enter, bh, content_alpha, now);
                    }
                }
                if activity.event == activity::Event::Tasks {
                    if content_alpha > 0.9 {
                        self.tasks_pill_rect = Some(Rect { x: x0, y: dy, w: x1 - x0, h: bh });
                    }
                    self.tasks.draw_pill(cr, renderer, x0, dy + climb + enter, x1 - x0, content_alpha, activity.since, now);
                }
                if matches!(activity.event, activity::Event::WifiOpen | activity::Event::BluetoothOpen) {
                    let (w, _) = activity.event.size();
                    if content_alpha > 0.9 {
                        self.network_menu_rect = Some(Rect { x: cx - w / 2.0, y: dy + climb + enter, w, h: bh });
                    }
                    if let Some(panel) = &mut self.network_panel {
                        panel.draw(cr, renderer, cx - w / 2.0, dy + climb + enter, bh, content_alpha, now);
                    }
                }
                activity::draw_pill(
                    cr,
                    renderer,
                    &self.activity_art,
                    &activity.event,
                    self.activity_value.value(now),
                    x0,
                    dy + climb + enter,
                    x1 - x0,
                    bh,
                    content_alpha,
                    activity.since,
                    now,
                );
            } else if let Some(since) = self.toast {
                if let Some(song) = &self.song {
                    let view = song.view(&self.eq, self.config.accent, None, None, now);
                    let climb = (1.0 - pill_alpha) * PILL_CLIMB;
                    player::draw_toast(cr, renderer, &self.art, &view, x0, dy + climb, x1 - x0, bh, pill_alpha, since, now);
                }
            } else {
                // a notification: the icon with its countdown on the left,
                // the app's name and the message beside it. A newer one
                // rolls the old content out through the top and itself in
                // from below.
                let roll = self.roll.state(now).x;
                if let Some(prev) = &self.notice_prev {
                    self.draw_notice(cr, renderer, prev, cx, dy, bh, pill_alpha * (1.0 - roll).powf(0.8), -ROLL_TRAVEL * roll, now);
                }
                if let Some(notice) = &self.notice {
                    let climb = (1.0 - pill_alpha) * PILL_CLIMB + ROLL_TRAVEL * (1.0 - roll);
                    let (_, _, w, _) = renderer.notice_layouts(cr, &notice.note);
                    if pill_alpha > 0.95 && roll > 0.95 {
                        self.notice_rect = Some(Rect { x: cx - w / 2.0, y: dy, w, h: bh });
                    }
                    // in the hand, the content follows sideways and thins out
                    let shift = self.notice_swipe.as_ref().map_or(0.0, |s| s.offset.value(now));
                    let fade = 1.0 - (shift.abs() / w).clamp(0.0, 1.0).powf(1.3);
                    cr.save().unwrap();
                    cr.translate(shift, 0.0);
                    self.draw_notice(cr, renderer, notice, cx, dy, bh, pill_alpha * roll.powf(0.7) * fade, climb, now);
                    cr.restore().unwrap();
                }
                if let Some((notice, fly, width)) = &self.notice_swiped {
                    // swiped away: it carries on out while the pill folds
                    let shift = fly.value(now);
                    let fade = 1.0 - (shift.abs() / width).clamp(0.0, 1.0).powf(0.8);
                    cr.save().unwrap();
                    cr.translate(shift, 0.0);
                    self.draw_notice(cr, renderer, notice, cx, dy, bh, pill_alpha * fade, 0.0, now);
                    cr.restore().unwrap();
                }
            }
        }
        if arriving {
            cr.restore().unwrap();
        }
        if sheet_k < 1.0 && pill_alpha > 0.0 {
            let hover = |_: quick::Action| 0.0;
            let never = |_: quick::Action| f64::INFINITY;
            let shown = &self.slider_shown;
            let level = |slider: quick::Slider| shown.get(&slider).map_or(0.0, |s| s.value(now).clamp(0.0, 1.0));
            let no_grow = |_: quick::Slider| 0.0;
            let no_icon = |_: &str, _: i32| None;
            let frame = quick::Frame {
                status: self.quick_status(now),
                controls: &self.controls,
                level: &level,
                grow: &no_grow,
                icon: &no_icon,
                art: &self.art,
                hover: &hover,
                logout_armed: false,
                now,
                opened: self.quick_opened_at,
                pressed: &never,
                bounced: &never,
            };
            quick::draw(cr, renderer, cx - quick::WIDTH / 2.0, dy - 8.0 * sheet_k,
                pill_alpha * (1.0 - sheet_k), self.quick_view, 0.0, &frame);
        }
        if let Some(since) = self.center_retiring_at.filter(|_| pill_alpha > 0.0) {
            // closing: the content sinks and fades with the fold, the mirror
            // of its arrival; replaced by another sheet: a short crossfade
            let (alpha, shift) = if self.center_swapped() {
                let k = min_jerk(((now - since) / SHEET_SWAP).clamp(0.0, 1.0));
                (pill_alpha * (1.0 - k), -8.0 * k)
            } else {
                (pill_alpha, (1.0 - pill_alpha) * PILL_CLIMB)
            };
            if alpha > 0.002 {
                self.center.draw(cr, renderer, &self.launcher_icons, cx - notice_center::WIDTH / 2.0, dy + shift, alpha, now);
            }
        }
        if pl > 0.0 {
            if let Some(song) = &self.song {
                let mut view = song.view(&self.eq, self.config.accent, self.pressed, self.scrub.map(|(f, _)| f), now);
                view.track_grow = self.track_grow.state(now).x.max(0.0);
                view.control_hover = std::array::from_fn(|i| self.control_hover[i].state(now).x.max(0.0));
                let alpha = smoothstep((pl - 0.3) / 0.6);
                player::draw(cr, renderer, &self.art, &view, x0, dy, x1 - x0, alpha, now, self.player_opened);
            }
        }
        if ed > 0.0 {
            if let Some(e) = &self.nvim {
                if self.output_view {
                    if let Some(output) = &self.editor_output {
                        output.draw(cr, renderer, x0, dy, x1 - x0, smoothstep((ed - 0.3) / 0.6));
                    }
                }
                if !self.output_view || self.editor_command.is_some() { editor::draw(
                    cr,
                    renderer,
                    e,
                    x0,
                    dy + if self.output_view { crate::editor_output::HEIGHT } else { 0.0 },
                    x1 - x0,
                    smoothstep((ed - 0.3) / 0.6),
                    self.editor_hover,
                    self.editor_pressed,
                    self.editor_diagnostics,
                    self.editor_command.as_ref(),
                    now,
                    self.editor_opened,
                ); }
            }
        }
        if let Some(l) = &self.launcher {
            // the sheet's content comes up once the box has most of its size
            l.draw(cr, renderer, x0, dy, x1 - x0, smoothstep((p - 0.35) / 0.5), now);
        }
        cr.restore().unwrap();

        let top = edge.min(dy).max(0.0);
        let mut bounds = Rect { x: x0 - FILLET, y: top, w: x1 - x0 + 2.0 * FILLET, h: y1 - top };
        if tugged {
            // the leaning top and the drawn-out fillets reach further out
            let reach = TUG_LEAN * tug_x.abs() + 0.45 * tug_y.max(0.0) + 2.0;
            bounds.x -= reach;
            bounds.w += 2.0 * reach;
        }
        if float_k > 0.001 {
            bounds.x -= floating::REACH;
            bounds.w += 2.0 * floating::REACH;
            bounds.h = bounds.h.max(floating::bottom(float_page) + 2.0 - top);
        }
        // the input region covers whichever is larger, the box now or the
        // box it is growing into
        let (target_w, target_h) = self.target_size(line_w, line_h, now);
        let input_w = (x1 - x0).max(target_w);
        let input_h = (y1 - dy).max(target_h);
        let mut input = Rect {
            x: cx - input_w / 2.0 - FILLET,
            y: top,
            w: input_w + 2.0 * FILLET,
            h: (dy + input_h - top).max(0.0),
        };
        if self.floats.target() > 0.5 {
            let page = floating::Body { x0: cx - input_w / 2.0, x1: cx + input_w / 2.0, top: dy, bottom: dy + input_h };
            input.x -= floating::REACH;
            input.w += 2.0 * floating::REACH;
            input.h = input.h.max(floating::bottom(page) + 2.0 - top);
        }
        // opaque: the box minus its curved corners (a T shape); none while
        // tugged — the leaning, thinned outline leaves parts of that box
        // clear, and the compositor fills a declared-opaque clear pixel black
        let opaque = if tugged { [None, None] } else { [
            (y1 - r > top).then(|| Rect { x: x0, y: top, w: x1 - x0, h: y1 - r - top }),
            (x1 - x0 > 2.0 * r).then(|| Rect { x: x0 + r, y: (y1 - r).max(top), w: x1 - x0 - 2.0 * r, h: r.min(y1 - top) }),
        ] };
        Frame { bounds: Some(bounds), input: Some(input), opaque }
    }

    /// One notification's content inside the pill: the app's icon with the
    /// countdown ring, its name and the message.
    #[allow(clippy::too_many_arguments)]
    fn draw_notice(&self, cr: &Context, renderer: &Renderer, notice: &Notice, cx: f64, dy: f64, bh: f64, alpha: f64, offset: f64, now: Time) {
        if alpha <= 0.001 {
            return;
        }
        let grouped = alpha < 0.999;
        if grouped { cr.save().unwrap(); cr.push_group(); }
        let opacity = alpha;
        let alpha = 1.0;
        let note = &notice.note;
        let t = now - notice.since;
        let (app, msg, width, _) = renderer.notice_layouts(cr, note);
        let x0 = cx - width / 2.0;
        let icon_cx = x0 + NOTICE_PAD_L + NOTICE_ICON / 2.0;
        let reply = self.notice_reply.as_ref().filter(|r| r.id == note.id);
        let actions: Vec<_> = if reply.is_some() { Vec::new() } else {
            note.actions.iter().filter(|(key, _)| key != "default").take(2).collect()
        };
        let mid = if reply.is_some() {
            dy + (bh - REPLY_H) / 2.0 + offset + 4.0
        } else {
            dy + bh / 2.0 + offset - if actions.is_empty() { 0.0 } else { 14.0 }
        };
        if !note.leading_image().is_some_and(|name| crate::notify::draw_media(cr, &self.launcher_icons, name,
            icon_cx, mid, NOTICE_ICON, renderer.scale(), alpha)) {
            icons::draw(cr, note.kind, icon_cx, mid, NOTICE_ICON, t, alpha);
        }
        if note.icon.is_some() {
            if let Some(image) = &note.image {
                let image_x = x0 + width - NOTICE_PAD_R - 36.0;
                let _ = crate::notify::draw_media(cr, &self.launcher_icons, image,
                    image_x, mid, 72.0, renderer.scale(), alpha);
            }
        }
        match note.progress {
            // a job reports how far it has got instead of how long the pill
            // still has to live
            Some(_) => icons::progress_ring(cr, note.kind, icon_cx, mid, NOTICE_ICON / 2.0 + 6.0,
                self.notice_progress.value(now).clamp(0.0, 1.0), alpha * smoothstep(t / 0.3)),
            // held (a reply, the pointer on it), its time stands still: full
            None => {
                let left = if self.held_notice() == Some(note.id) { 1.0 } else { 1.0 - (t / self.config.notice_hold).clamp(0.0, 1.0) };
                icons::countdown(cr, note.kind, icon_cx, mid, NOTICE_ICON / 2.0 + 6.0, left, alpha * smoothstep(t / 0.3))
            }
        }

        let (_, app_ext) = app.pixel_extents();
        let (_, msg_ext) = msg.pixel_extents();
        let bar = if note.progress.is_some() { NOTICE_BAR } else { 0.0 };
        let total = app_ext.height() as f64 + NOTICE_LINE_GAP + msg_ext.height() as f64 + bar;
        let tx = renderer.snap(x0 + NOTICE_PAD_L + NOTICE_ICON + NOTICE_GAP);
        let ty = mid - total / 2.0;
        // the lines settle in a hair after the icon, each a touch later
        let slide = |d: f64| 6.0 * (1.0 - smoothstep(((t - d) / 0.4).clamp(0.0, 1.0)));
        let accent = icons::accent(note.kind);
        cr.set_source_rgba(accent.0, accent.1, accent.2, alpha);
        cr.move_to(tx + slide(0.06), renderer.snap(ty - app_ext.y() as f64));
        pangocairo::functions::show_layout(cr, &app);
        cr.set_source_rgba(1.0, 1.0, 1.0, alpha * 0.95);
        cr.move_to(tx + slide(0.12), renderer.snap(ty + app_ext.height() as f64 + NOTICE_LINE_GAP - msg_ext.y() as f64));
        pangocairo::functions::show_layout(cr, &msg);
        if note.progress.is_some() {
            // a download's bar under the message, in the app's colour
            let done = self.notice_progress.value(now).clamp(0.0, 1.0);
            let (bx, by) = (tx + slide(0.18), ty + total - 4.0);
            let bw = x0 + width - NOTICE_PAD_R - tx;
            crate::menu_style::rounded(cr, bx, by, bw, 4.0, 2.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * alpha);
            let _ = cr.fill();
            if done > 0.0 {
                crate::menu_style::rounded(cr, bx, by, (bw * done).max(4.0), 4.0, 2.0);
                cr.set_source_rgba(accent.0, accent.1, accent.2, alpha);
                let _ = cr.fill();
            }
        }
        if let Some(reply) = reply {
            self.draw_reply(cr, renderer, note, reply, tx, x0 + width - NOTICE_PAD_R, dy + bh - REPLY_H + offset, alpha, now);
        }
        for (index, (_, title)) in actions.iter().enumerate() {
            let bx = x0 + NOTICE_PAD_L + 55.0 + index as f64 * 148.0;
            let by = dy + bh - 39.0 + offset;
            cr.new_path();
            cr.arc(bx + 118.0, by + 15.0, 15.0, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
            cr.arc(bx + 15.0, by + 15.0, 15.0, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();
            cr.set_source_rgba(1.0, 1.0, 1.0, (0.12 + 0.15 * self.notice_action_hover[index].value(now)) * alpha);
            let _ = cr.fill();
            let label = renderer.layout_ellipsized(cr, title, Font::Small, 113.0, 1);
            let (_, ink) = label.pixel_extents();
            cr.set_source_rgba(1.0, 1.0, 1.0, alpha * 0.95);
            cr.move_to(renderer.snap(bx + (133.0 - ink.width() as f64) / 2.0 - ink.x() as f64),
                renderer.snap(by + (30.0 - ink.height() as f64) / 2.0 - ink.y() as f64));
            pangocairo::functions::show_layout(cr, &label);
        }
        if grouped {
            cr.pop_group_to_source().unwrap();
            let _ = cr.paint_with_alpha(opacity);
            cr.restore().unwrap();
        }
    }

    /// The reply field: an iMessage capsule with a caret and the send
    /// button, rising in under the message.
    #[allow(clippy::too_many_arguments)]
    fn draw_reply(&self, cr: &Context, renderer: &Renderer, note: &crate::notify::Notification, reply: &Reply,
        x0: f64, x1: f64, top: f64, alpha: f64, now: Time) {
        let k = smoothstep((now - reply.since) / 0.3);
        let rect = (x0, top + 4.0 + 6.0 * (1.0 - k), x1 - x0, crate::menu_style::REPLY_FIELD_H);
        crate::menu_style::reply_field(cr, renderer, rect, &reply.text, note.reply.as_deref().unwrap_or("Ответ"),
            reply.typed, now, alpha * k);
    }

    /// Start typing a reply to the notification on show (its app takes
    /// inline replies). The field takes the keyboard.
    pub fn begin_reply(&mut self, now: Time) -> bool {
        let Some(notice) = self.notice.as_ref().filter(|n| n.note.reply.is_some()) else { return false };
        self.notice_reply = Some(Reply { id: notice.note.id, text: String::new(), since: now, typed: now });
        self.notice_swipe = None;
        self.sync(now);
        true
    }

    pub fn replying(&self) -> bool {
        self.notice_reply.is_some()
    }

    /// Typing into the reply: text, BackSpace, Ctrl+BackSpace, Ctrl+U,
    /// Return sends, Escape gives up.
    pub fn reply_key(&mut self, key: &str, text: Option<&str>, ctrl: bool, now: Time) -> ReplyKey {
        let Some(reply) = self.notice_reply.as_mut() else { return ReplyKey::None };
        match key {
            "Return" => {
                if reply.text.trim().is_empty() { return ReplyKey::None; }
                return self.send_reply(now).map_or(ReplyKey::None, |(id, text)| ReplyKey::Send(id, text));
            }
            "Escape" => {
                self.cancel_reply(now);
                return ReplyKey::Cancel;
            }
            "BackSpace" if ctrl => {
                let trimmed = reply.text.trim_end().len();
                let cut = reply.text[..trimmed].rfind(char::is_whitespace).map_or(0, |i| i + 1);
                reply.text.truncate(cut);
            }
            "BackSpace" => { reply.text.pop(); }
            "u" if ctrl => reply.text.clear(),
            _ => {
                if let Some(text) = text.filter(|t| !ctrl && t.chars().all(|c| !c.is_control())) {
                    if reply.text.chars().count() < 2000 { reply.text.push_str(text); }
                }
            }
        }
        reply.typed = now;
        self.sync(now);
        ReplyKey::None
    }

    /// Send what was typed: the pill goes, its ID and the text return.
    pub fn send_reply(&mut self, now: Time) -> Option<(u32, String)> {
        let reply = self.notice_reply.take()?;
        if reply.text.trim().is_empty() {
            self.notice_reply = Some(reply);
            return None;
        }
        self.notice = None;
        self.notice_prev = None;
        self.sync(now);
        Some((reply.id, reply.text))
    }

    fn cancel_reply(&mut self, now: Time) {
        if self.notice_reply.take().is_some() {
            // back to an ordinary notification, with its full time again
            if let Some(notice) = self.notice.as_mut() { notice.since = now; }
            self.sync(now);
        }
    }

    /// Is the player open (so clicks go to its controls)?
    pub fn player_open(&self, now: Time) -> bool {
        self.player.state(now).x > 0.5 && self.song.is_some()
    }

    /// Music commands are available only over the compact equaliser or the
    /// visible player panel. The clock and the editor chip belong to the
    /// island's shortcut menu instead.
    pub fn accepts_player_clicks(&self) -> bool {
        self.song.is_some() && !self.pill_open() && self.launcher.is_none()
    }

    pub fn music_area_at(&self, px: f64, py: f64) -> bool {
        if !self.accepts_player_clicks() { return false; }
        self.on_music(px, py) || self.player_rect.is_some_and(|r| {
            px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
        })
    }

    /// A normal click opens shortcuts unless a modal panel owns the input.
    pub fn accepts_quick_menu_click(&self) -> bool {
        !self.hidden.any() && self.auth.is_none() && self.launcher.is_none()
            && !self.keyboard_panel_open() && !self.is_scrubbing()
    }

    pub fn quick_menu_open(&self) -> bool { self.quick_open }

    /// Has another sheet taken the box from the closing notification
    /// center (a crossfade rather than a fold)?
    fn center_swapped(&self) -> bool {
        self.quick_open || self.launcher.is_some() || self.activity.is_some() || self.notice.is_some() || self.auth.is_some()
    }

    fn quick_status(&self, now: Time) -> quick::Status {
        quick::Status {
            unread: self.center.unread(),
            dnd: self.dnd_glow.value(now),
            timer: self.timer.as_ref().map(|t| (t.reading(now), t.running)),
            recording: self.recorder.then_some(self.recording.is_some()),
            night: self.night_glow.value(now),
        }
    }

    pub fn notification_id(&self) -> Option<u32> { self.notice.as_ref().map(|n| n.note.id).filter(|id| *id != 0) }

    pub fn set_dnd(&mut self, enabled: bool, now: Time) {
        if self.dnd == enabled { return; }
        self.dnd = enabled;
        self.dnd_glow.set(f64::from(enabled), now);
        // the moon bounces as Focus switches
        self.quick_bounced.insert(quick::Action::Dnd, now);
        notice_center::save_dnd(enabled);
        if enabled && self.notice.as_ref().is_some_and(|notice| !notice.note.critical) {
            self.notice = None;
            self.notice_prev = None;
            self.notice_queued = None;
        }
        self.sync(now);
    }

    /// Two-finger (or tilt-wheel) sideways scroll over the notification:
    /// the content follows. `stop` is the touchpad's finger lift. Returns
    /// whether the gesture was the notification's, and the D-Bus ID to
    /// close if it was swiped away.
    pub fn swipe_notice(&mut self, px: f64, py: f64, dx: f64, stop: bool, now: Time) -> (bool, Option<u32>) {
        let over = self.notice_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h);
        let active = self.notice_swipe.as_ref().is_some_and(|s| s.held && s.axis);
        if !active && (!over || dx == 0.0 || self.notice.is_none()) {
            return (false, None);
        }
        if !active {
            let mut swipe = Swipe::new(now);
            swipe.axis = true;
            self.notice_swipe = Some(swipe);
        }
        if dx != 0.0 {
            if let Some(swipe) = self.notice_swipe.as_mut() {
                // the content moves the way scrolled content would
                let to = swipe.offset.value(now) - dx;
                swipe.to(to, now);
            }
        }
        let dismissed = if stop { self.release_swipe(now) } else { None };
        self.sync(now);
        (true, dismissed)
    }

    /// A mouse press on the notification's body: maybe a click, maybe the
    /// start of a swipe — decided by how far it moves. Action buttons
    /// keep acting on the press.
    pub fn press_notice(&mut self, px: f64, py: f64, now: Time) -> bool {
        let Some(r) = self.notice_rect else { return false };
        if px < r.x || px > r.x + r.w || py < r.y || py > r.y + r.h || self.notice.is_none() {
            return false;
        }
        if self.notice_action_index_at(px, py).is_some() || self.notice_reply.is_some() {
            return false;
        }
        let mut swipe = Swipe::new(now);
        swipe.press = Some(px);
        self.notice_swipe = Some(swipe);
        true
    }

    /// The held mouse moved: past a few pixels it is a swipe.
    pub fn drag_notice(&mut self, px: f64, now: Time) -> bool {
        let Some(swipe) = self.notice_swipe.as_mut().filter(|s| s.held && !s.axis) else { return false };
        let Some(x0) = swipe.press else { return false };
        let dx = px - x0;
        if swipe.offset.value(now) == 0.0 && dx.abs() < 6.0 {
            return true;
        }
        swipe.to(dx, now);
        true
    }

    /// The mouse let go of the notification.
    pub fn release_notice(&mut self, px: f64, py: f64, now: Time) -> NoticeRelease {
        let Some(swipe) = self.notice_swipe.as_ref().filter(|s| s.held && !s.axis) else { return NoticeRelease::None };
        if swipe.offset.value(now) == 0.0 {
            self.notice_swipe = None;
            return self.notice_click_at(px, py).map_or(NoticeRelease::None, NoticeRelease::Click);
        }
        match self.release_swipe(now) {
            Some(id) => NoticeRelease::Dismissed(id),
            None => NoticeRelease::None,
        }
    }

    /// Let go: far or fast enough and the notification flies off (its ID
    /// is returned to close it), otherwise it springs back.
    fn release_swipe(&mut self, now: Time) -> Option<u32> {
        let mut swipe = self.notice_swipe.take()?;
        let width = self.notice_rect.map_or(NOTICE_MIN_W, |r| r.w);
        let offset = swipe.offset.value(now);
        let flick = swipe.velocity.abs() > SWIPE_FLICK && swipe.velocity.signum() == offset.signum() && offset.abs() > 16.0;
        if offset.abs() > width * SWIPE_DISMISS || flick {
            let notice = self.notice.take()?;
            let id = notice.note.id;
            let mut fly = Spring::new(offset, 0.28, 1.0);
            fly.set(offset.signum() * (width + 60.0), now);
            self.notice_swiped = Some((notice, fly, width));
            self.notice_prev = None;
            self.notice_rect = None;
            self.sync(now);
            return Some(id);
        }
        swipe.held = false;
        swipe.offset.set(0.0, now);
        self.notice_swipe = Some(swipe);
        self.sync(now);
        None
    }

    /// A swipe that ended by itself since the last look.
    pub fn take_swiped(&mut self) -> Option<u32> {
        self.swiped_id.take()
    }

    pub fn notice_click_at(&self, px: f64, py: f64) -> Option<NoticeClick> {
        let rect = self.notice_rect?;
        if px < rect.x || px > rect.x + rect.w || py < rect.y || py > rect.y + rect.h { return None; }
        let note = &self.notice.as_ref()?.note;
        if note.id == 0 { return None; }
        if self.notice_reply.is_some() {
            // only the send button acts; the rest of the pill is the field
            let (cx, cy) = (rect.x + rect.w - NOTICE_PAD_R - 18.0, rect.y + rect.h - REPLY_H + 4.0 + 18.0);
            let send = (px - cx).powi(2) + (py - cy).powi(2) <= 18.0 * 18.0;
            return Some(if send { NoticeClick::SendReply } else { NoticeClick::Field });
        }
        if let Some(index) = self.notice_action_index_at(px, py) {
            let (key, _) = note.actions.iter().filter(|(key, _)| key != "default").nth(index)?;
            return Some(NoticeClick::Invoke(note.id, key.clone()));
        }
        note.actions.iter().find(|(key, _)| key == "default")
            .map_or(Some(NoticeClick::Dismiss(note.id)), |(key, _)| Some(NoticeClick::Invoke(note.id, key.clone())))
    }

    fn notice_action_index_at(&self, px: f64, py: f64) -> Option<usize> {
        let rect = self.notice_rect?;
        if py < rect.y + rect.h - 39.0 || py > rect.y + rect.h - 9.0 { return None; }
        let note = &self.notice.as_ref()?.note;
        note.actions.iter().filter(|(key, _)| key != "default").take(2).enumerate().find_map(|(index, _)| {
            let bx = rect.x + NOTICE_PAD_L + 55.0 + index as f64 * 148.0;
            (px >= bx && px <= bx + 133.0).then_some(index)
        })
    }

    pub fn set_notice_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        let over = self.notice_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h);
        if over != self.notice_pointer {
            self.notice_pointer = over;
            // let go, it has its full time again
            if !over {
                if let Some(notice) = self.notice.as_mut() { notice.since = now; }
            }
        }
        let hovered = self.notice_action_index_at(px, py);
        if self.notice_hovered == hovered { return false; }
        self.notice_hovered = hovered;
        for (index, spring) in self.notice_action_hover.iter_mut().enumerate() {
            spring.set(if hovered == Some(index) { 1.0 } else { 0.0 }, now);
        }
        true
    }

    /// The notification the user has in hand: a reply being typed into it,
    /// the pointer resting on it, a swipe under way. It neither times out
    /// here nor expires at the server.
    pub fn held_notice(&self) -> Option<u32> {
        let notice = self.notice.as_ref()?;
        let held = self.notice_reply.as_ref().is_some_and(|r| r.id == notice.note.id) || self.notice_pointer
            || self.notice_swipe.as_ref().is_some_and(|s| s.held);
        held.then_some(notice.note.id)
    }

    pub fn notification_closed(&mut self, id: u32, now: Time) {
        self.center.closed(id);
        // a reply being typed stays until it is sent or given up
        if self.notice_reply.as_ref().is_some_and(|r| r.id == id) {
            self.sync(now);
            return;
        }
        if self.notice.as_ref().is_some_and(|n| n.note.id == id) {
            self.notice = None;
            self.notice_prev = None;
        }
        if self.notice_queued.as_ref().is_some_and(|n| n.id == id) { self.notice_queued = None; }
        self.sync(now);
    }

    pub fn open_notification_center(&mut self, now: Time) -> bool {
        if self.auth.is_some() { return false; }
        self.quick_open = false;
        self.launcher = None;
        self.notice = None;
        self.notice_prev = None;
        self.activity = None;
        self.center.open(now);
        self.sync(now);
        true
    }

    pub fn set_center_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        if !self.center.open { return false; }
        let Some(rect) = self.center_rect else { return false };
        self.center.pointer(px - rect.x, py - rect.y, now)
    }

    pub fn clear_center_hover(&mut self, now: Time) -> bool {
        self.center.pointer(-1.0, -1.0, now)
    }

    pub fn click_notification_center(&mut self, px: f64, py: f64, activate: bool, now: Time) -> Option<notice_center::Action> {
        if !self.center.open { return None; }
        let action = if activate {
            self.center_rect.map_or(notice_center::Action::None, |rect| self.center.click(px - rect.x, py - rect.y, now))
        } else { notice_center::Action::None };
        self.sync(now);
        Some(action)
    }

    /// Two fingers sideways over the notification center's cards.
    pub fn swipe_center(&mut self, px: f64, py: f64, dx: f64, stop: bool, now: Time) -> Option<notice_center::Action> {
        if !self.center.open { return None; }
        let rect = self.center_rect?;
        let (taken, action) = self.center.swipe_axis(px - rect.x, py - rect.y, dx, stop, now);
        if !taken { return None; }
        self.sync(now);
        Some(action.unwrap_or(notice_center::Action::None))
    }

    /// A mouse press on a card: decided on release (click or swipe).
    pub fn press_center(&mut self, px: f64, py: f64, now: Time) -> bool {
        if !self.center.open { return false; }
        let Some(rect) = self.center_rect else { return false };
        self.center.press(px - rect.x, py - rect.y, now)
    }

    pub fn drag_center(&mut self, px: f64, now: Time) -> bool {
        if !self.center.open { return false; }
        let Some(rect) = self.center_rect else { return false };
        self.center.drag(px - rect.x, now)
    }

    pub fn release_center(&mut self, px: f64, py: f64, now: Time) -> Option<notice_center::Action> {
        if !self.center.open { return None; }
        let rect = self.center_rect?;
        let action = self.center.release(px - rect.x, py - rect.y, now);
        self.sync(now);
        action
    }

    /// What a card swipe that ended on its own did.
    pub fn take_center_action(&mut self) -> Option<notice_center::Action> {
        self.center.take_pending()
    }

    pub fn scroll_notification_center(&mut self, rows: f64, now: Time) -> bool {
        if !self.center.open { return false; }
        self.center.scroll(rows, now);
        true
    }

    /// Is a page open that the floating buttons stand beside?
    fn floats_wanted(&self) -> bool {
        self.auth.is_none() && self.launcher.is_none()
            && (self.quick_open || self.center.open || self.calendar_open() || self.timer_menu_open())
    }

    fn timer_menu_open(&self) -> bool {
        self.activity.as_ref().is_some_and(|a| a.event == activity::Event::TimerMenu)
    }

    /// Is the button's page the one showing?
    pub fn float_selected(&self, button: floating::Button) -> bool {
        match button {
            floating::Button::Sections => self.sections_open(),
            floating::Button::Search => false,
            floating::Button::Timer => self.timer_menu_open(),
            floating::Button::Notifications => self.center.open,
            floating::Button::Sound => self.quick_open && self.quick_view == quick::View::Sound,
            floating::Button::Calendar => self.calendar_open(),
        }
    }

    /// The floating button under (px, py), once the buttons have arrived.
    pub fn float_at(&self, px: f64, py: f64) -> Option<floating::Button> {
        floating::hit(self.float_body?, px, py)
    }

    pub fn set_float_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        let hover = self.float_at(px, py);
        if hover == self.float_hover { return false; }
        self.float_hover = hover;
        self.sync(now);
        true
    }

    /// A floating button was pressed: it dips; the page it leads to opens
    /// in place of the one showing (the island morphs between them).
    pub fn press_float(&mut self, button: floating::Button, now: Time) {
        self.float_pressed = Some((button, now));
        if self.float_selected(button) { return; }
        self.leave_pages(now);
        match button {
            floating::Button::Search => {
                self.toggle_launcher(now);
            }
            floating::Button::Timer => {
                let _ = self.timer_command("menu", now);
            }
            floating::Button::Notifications => {
                self.open_notification_center(now);
            }
            floating::Button::Sound => {
                self.show_quick_menu(now);
                self.quick_view = quick::View::Sound;
            }
            floating::Button::Sections => self.open_sections(now),
            floating::Button::Calendar => self.open_calendar(now),
        }
        self.sync(now);
    }

    /// Close whichever page is open, for another to take its place.
    fn leave_pages(&mut self, now: Time) {
        if self.center.open {
            self.center.close();
            self.center_rect = None;
        }
        if self.quick_open {
            // its content fades as the next page's comes in
            self.close_quick_menu(now);
        }
        if self.activity.as_ref().is_some_and(|a| activity::takes_keyboard(&a.event)) {
            self.activity = None;
        }
    }

    pub fn sections_open(&self) -> bool {
        self.quick_open && self.quick_view == quick::View::Sections
    }

    /// The sections gallery: the Control Center's sheet on its gallery page.
    pub fn open_sections(&mut self, now: Time) {
        self.open_quick_menu_shortcut(now);
        self.quick_view = quick::View::Sections;
        self.sync(now);
    }

    pub fn open_quick_menu(&mut self, now: Time) -> bool {
        if !self.accepts_quick_menu_click() { return false; }
        self.show_quick_menu(now);
        true
    }

    /// A keyboard shortcut may summon the sheet while the compact island is
    /// parked above a fullscreen window, just like the launcher.
    pub fn open_quick_menu_shortcut(&mut self, now: Time) -> bool {
        if self.auth.is_some() { return false; }
        self.launcher = None;
        let from_center = self.center.open;
        self.center.close();
        self.show_quick_menu(now);
        if from_center {
            // crossfade with the notification center as it leaves
            self.quick_returned_at = Some(now);
        }
        true
    }

    fn show_quick_menu(&mut self, now: Time) {
        self.quick_open = true;
        self.quick_retiring_at = None;
        self.quick_hover = None;
        self.quick_selected = 0;
        self.quick_view = quick::View::Main;
        self.quick_view_from = None;
        self.quick_drag = None;
        self.logout_armed_at = None;
        self.quick_opened_at = now;
        self.quick_pressed.clear();
        self.quick_bounced.clear();
        self.eq_hovered = false;
        self.editor_chip_hovered = false;
        self.notice = None;
        self.notice_prev = None;
        self.activity = None;
        self.activity_queued = None;
        self.toast = None;
        self.sync(now);
    }

    /// Explicit close buttons in child sheets return to the shortcut grid.
    /// Escape and an outside click still dismiss the entire island menu.
    pub fn return_to_quick_menu(&mut self, now: Time) {
        if self.auth.is_some() { return; }
        self.activity_retiring = None;
        self.network_panel = None;
        self.open_quick_menu_shortcut(now);
        self.quick_returned_at = Some(now);
    }

    pub fn close_quick_menu(&mut self, now: Time) {
        if self.quick_open {
            self.quick_open = false;
            self.quick_retiring_at = Some(now);
            self.quick_returned_at = None;
            self.quick_rect = None;
            self.quick_hover = None;
            self.sync(now);
        }
    }

    fn quick_hit(&self, px: f64, py: f64) -> Option<quick::Hit> {
        self.quick_rect.and_then(|r| quick::hit(self.quick_view, &self.controls, px - r.x, py - r.y))
    }

    pub fn quick_action_at(&self, px: f64, py: f64) -> Option<quick::Action> {
        match self.quick_hit(px, py)? {
            quick::Hit::Action(action) => Some(action),
            quick::Hit::Slider(..) => None,
        }
    }

    /// A press in the Control Center: an action, or a slider grabbed at
    /// the pointer (its value jumps there, as in macOS).
    pub fn quick_press(&mut self, px: f64, py: f64, now: Time) -> Option<quick::Hit> {
        let hit = self.quick_hit(px, py)?;
        match hit {
            quick::Hit::Slider(slider, value) => {
                self.quick_drag = Some(slider);
                self.slider_grow.entry(slider).or_insert_with(|| Spring::new(0.0, 0.3, 0.62)).set(1.0, now);
                self.set_control_level(slider, value, now);
            }
            quick::Hit::Action(action) => self.tap(action, now),
        }
        Some(hit)
    }

    /// The held slider follows the pointer, even outside the menu.
    pub fn quick_drag_to(&mut self, px: f64, now: Time) -> Option<(quick::Slider, f64)> {
        let slider = self.quick_drag?;
        let rect = self.quick_rect?;
        let value = quick::slider_fraction(slider, &self.controls, px - rect.x);
        self.set_control_level(slider, value, now);
        Some((slider, value))
    }

    pub fn quick_release(&mut self, now: Time) -> bool {
        let Some(slider) = self.quick_drag.take() else { return false };
        if let Some(spring) = self.slider_grow.get_mut(&slider) { spring.set(0.0, now); }
        true
    }

    /// A control was pressed: it dips; toggles bounce their symbol.
    fn tap(&mut self, action: quick::Action, now: Time) {
        self.quick_pressed.insert(action, now);
        if matches!(action, quick::Action::Layout) {
            self.quick_bounced.insert(action, now);
        }
    }

    /// The keyboard layout changed (from the menu or anywhere else).
    pub fn set_layout(&mut self, layout: String, now: Time) {
        if self.controls.layout.as_deref() == Some(layout.as_str()) { return; }
        let known = self.controls.layout.is_some();
        self.controls.layout = Some(layout);
        if known { self.quick_bounced.insert(quick::Action::Layout, now); }
        self.sync(now);
    }

    /// Move a slider's eased position (a fresh one starts where it is).
    fn show_level(&mut self, slider: quick::Slider, value: f64, now: Time) {
        self.slider_shown.entry(slider).or_insert_with(|| Spring::new(value, SLIDER_SPRING, 1.0)).set(value, now);
    }

    fn set_control_level(&mut self, slider: quick::Slider, value: f64, now: Time) {
        match slider {
            quick::Slider::Volume => {
                self.controls.volume = Some(value);
                self.controls.muted = false;
            }
            quick::Slider::Brightness => self.controls.brightness = Some(value),
            quick::Slider::App(i) => {
                if let Some(app) = self.controls.apps.get_mut(i) {
                    app.volume = value;
                    app.muted = false;
                }
            }
            quick::Slider::Mic => self.controls.mic = Some(value),
        }
        self.show_level(slider, value, now);
    }

    /// A fresh reading of the environment for the Control Center.
    pub fn set_controls(&mut self, snapshot: crate::control::Snapshot, now: Time) {
        // the held slider is the truth until it is let go
        let held = self.quick_drag;
        if held != Some(quick::Slider::Volume) {
            if let Some((value, muted)) = snapshot.volume {
                self.set_control_level(quick::Slider::Volume, value, now);
                self.controls.muted = muted;
            } else {
                self.controls.volume = None;
            }
        }
        if held != Some(quick::Slider::Brightness) {
            match snapshot.brightness {
                Some(value) => self.set_control_level(quick::Slider::Brightness, value, now),
                None => self.controls.brightness = None,
            }
        }
        // while an application's slider is held its row must not move
        if !matches!(held, Some(quick::Slider::App(_))) {
            self.controls.apps = snapshot.apps;
            for i in 0..self.controls.apps.len() {
                let value = self.controls.apps[i].volume;
                self.show_level(quick::Slider::App(i), value, now);
            }
        }
        if held != Some(quick::Slider::Mic) {
            self.controls.mic = snapshot.mic.map(|(value, _)| value);
            self.controls.mic_muted = snapshot.mic.is_some_and(|(_, muted)| muted);
            if let Some((value, _)) = snapshot.mic { self.show_level(quick::Slider::Mic, value, now); }
        }
        self.controls.mic_label = snapshot.mic_label;
        if self.controls.night != snapshot.night {
            self.controls.night = snapshot.night;
            self.night_glow.set(if snapshot.night { 1.0 } else { 0.0 }, now);
        }
        self.controls.sinks = snapshot.sinks;
        self.controls.default_sink = snapshot.default_sink;
        if let Some(layout) = snapshot.layout { self.set_layout(layout, now); }
        self.sync(now);
    }

    /// Night mode on or off at once (the worker confirms); returns the
    /// new state.
    pub fn toggle_night(&mut self, now: Time) -> bool {
        self.controls.night = !self.controls.night;
        self.night_glow.set(if self.controls.night { 1.0 } else { 0.0 }, now);
        self.quick_bounced.insert(quick::Action::Night, now);
        self.sync(now);
        self.controls.night
    }

    /// Mute or unmute the microphone at once (the worker confirms).
    pub fn toggle_mic(&mut self, now: Time) {
        self.controls.mic_muted = !self.controls.mic_muted;
        self.quick_bounced.insert(quick::Action::MicMute, now);
        self.sync(now);
    }

    /// Is the sound page showing (its rows are refreshed while it is)?
    pub fn sound_page_open(&self) -> bool {
        self.quick_open && self.quick_view == quick::View::Sound
    }

    /// Switch the Control Center's page with a short crossfade.
    pub fn quick_show(&mut self, view: quick::View, now: Time) {
        if self.quick_view == view { return; }
        self.quick_view_from = Some((self.quick_view, now));
        self.quick_view = view;
        self.quick_hover = None;
        self.quick_selected = 0;
        self.sync(now);
    }

    /// Logging out takes a second press within a few seconds; returns
    /// whether this press confirms it.
    pub fn arm_logout(&mut self, now: Time) -> bool {
        if self.logout_armed_at.is_some_and(|since| now - since < LOGOUT_CONFIRM) {
            self.logout_armed_at = None;
            return true;
        }
        self.logout_armed_at = Some(now);
        self.quick_bounced.insert(quick::Action::Logout, now);
        self.sync(now);
        false
    }

    /// The actions the keyboard walks through on the current page.
    fn quick_keys(&self) -> Vec<quick::Action> {
        match self.quick_view {
            quick::View::Main => {
                // the music card's buttons come first, as the card does
                let media = self.controls.media.is_some().then_some([
                    quick::Action::MediaPrevious, quick::Action::MediaPlayPause, quick::Action::MediaNext,
                    quick::Action::MediaClear]);
                media.into_iter().flatten()
                    .chain(quick::ACTIONS.iter().copied()
                        .filter(|a| *a != quick::Action::Outputs || !self.controls.sinks.is_empty()))
                    .collect()
            }
            quick::View::Sound => (0..self.controls.sinks.len()).map(quick::Action::Sink)
                .chain(self.controls.mic.is_some().then_some(quick::Action::MicMute)).collect(),
            quick::View::Sections => quick::SECTIONS.iter().map(|s| quick::Action::Section(*s)).collect(),
        }
    }

    pub fn quick_key(&mut self, key: &str, now: Time) -> Option<quick::Action> {
        if !self.quick_open { return None; }
        let keys = self.quick_keys();
        if keys.is_empty() { return None; }
        self.quick_selected = self.quick_selected.min(keys.len() - 1);
        match key {
            "next" => self.quick_selected = (self.quick_selected + 1) % keys.len(),
            "prev" => self.quick_selected = (self.quick_selected + keys.len() - 1) % keys.len(),
            "enter" => {
                let action = keys[self.quick_selected];
                self.tap(action, now);
                return Some(action);
            }
            _ => {}
        }
        self.quick_hover = Some(keys[self.quick_selected]);
        self.sync(now);
        None
    }

    pub fn set_quick_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        let hovered = self.quick_action_at(px, py);
        if self.quick_hover == hovered { return false; }
        self.quick_hover = hovered;
        if let Some(action) = hovered {
            if let Some(i) = self.quick_keys().iter().position(|a| *a == action) { self.quick_selected = i; }
        }
        self.sync(now);
        true
    }

    /// A press on the closed island: maybe a click, maybe a tug — the
    /// pointer's next moves decide. Only the plain line can be tugged.
    pub fn tug_press(&mut self, px: f64, py: f64, now: Time) -> bool {
        if !self.tuggable(px, py) {
            return false;
        }
        let offset = self.tug_now(now);
        self.tug = Some(Tug { from: (px, py), engaged: false, offset, velocity: (0.0, 0.0), at: now });
        // caught mid-wobble: it stays where it is, in the hand
        let o = (px - rubber_inverse(offset.0, TUG_SIDE), py - rubber_inverse(offset.1, if offset.1 > 0.0 { TUG_DOWN } else { TUG_UP }));
        if offset.0.abs() > 3.0 || offset.1.abs() > 3.0 {
            self.tug = Some(Tug { from: o, engaged: true, offset, velocity: (0.0, 0.0), at: now });
        }
        true
    }

    /// Can the plain line be taken in the hand at this point?
    pub fn tuggable(&self, px: f64, py: f64) -> bool {
        !self.pill_open() && self.launcher.is_none() && !self.hidden.any() && self.auth.is_none()
            && self.player.target() < 0.5 && self.editor_panel.target() < 0.5
            && self.line_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h)
    }

    /// The held island follows the pointer, against the rubber band.
    /// Returns whether it is being tugged (the press is no click then).
    pub fn tug_to(&mut self, px: f64, py: f64, now: Time) -> bool {
        let Some(tug) = self.tug.as_mut() else { return false };
        let (dx, dy) = (px - tug.from.0, py - tug.from.1);
        if !tug.engaged {
            if dx.hypot(dy) < TUG_SLOP {
                return false;
            }
            tug.engaged = true;
        }
        let offset = tug_offset(dx, dy);
        let dt = now - tug.at;
        if dt > 0.004 {
            // smoothed, so a last jittery event does not set the throw
            let v = ((offset.0 - tug.offset.0) / dt, (offset.1 - tug.offset.1) / dt);
            tug.velocity = (0.5 * tug.velocity.0 + 0.5 * v.0, 0.5 * tug.velocity.1 + 0.5 * v.1);
            tug.at = now;
        }
        tug.offset = offset;
        true
    }

    /// Let go: a tug jumps back like jelly. Returns whether it was a tug
    /// (false: the press was a click after all).
    pub fn tug_release(&mut self, now: Time) -> bool {
        let Some(tug) = self.tug.take() else { return false };
        if !tug.engaged {
            return false;
        }
        // a pointer that rested before letting go throws nothing
        let fresh = now - tug.at < 0.08;
        let (vx, vy) = if fresh { tug.velocity } else { (0.0, 0.0) };
        self.tug_x.release(tug.offset.0, vx.clamp(-1500.0, 1500.0), 0.0, now);
        self.tug_y.release(tug.offset.1, vy.clamp(-1500.0, 1500.0), 0.0, now);
        // the edge it pulled snaps back, and a ripple runs to the corners
        if self.config.edge_light {
            let strength = (tug.offset.1.max(0.0) / TUG_DOWN).max(tug.offset.0.abs() / TUG_SIDE * 0.5);
            let cx = self.island_cx(now) + tug.offset.0;
            self.membrane.ripple(cx, strength, now);
        }
        true
    }

    /// How far the island gives right now (held, or wobbling back).
    fn tug_now(&self, now: Time) -> (f64, f64) {
        match &self.tug {
            Some(t) if t.engaged => t.offset,
            _ => (self.tug_x.value(now), self.tug_y.value(now)),
        }
    }

    /// Does a finger landing here hold something to drag at once (a
    /// slider, the playhead, the timer's ruler, a notification to swipe),
    /// rather than wait to see whether it taps or scrolls?
    pub fn touch_grabs(&self, cx: f64, px: f64, py: f64, now: Time) -> bool {
        if self.quick_open && matches!(self.quick_hit(px, py), Some(quick::Hit::Slider(..))) {
            return true;
        }
        if matches!(self.control_at(cx, px, py, now), Some(player::Control::Seek(_))) {
            return true;
        }
        if self.timer_menu_open() {
            if let Some(r) = self.timer_menu_rect {
                let session = self.timer_session(now);
                if self.timer_page.hit(&session, px - r.x, py - r.y) == Some(timer_page::Part::Ruler) {
                    return true;
                }
            }
        }
        self.notice.is_some() && self.notice_reply.is_none()
            && self.notice_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h)
            && self.notice_action_index_at(px, py).is_none()
    }

    /// A finger cannot hover: a tap on the bars (or the editor's chip)
    /// does what resting the pointer there would, and opens its panel.
    pub fn touch_reveals(&self, px: f64, py: f64, now: Time) -> bool {
        let chip = self.editor_chip_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h);
        (self.accepts_player_clicks() && self.on_music(px, py) && !self.player_open(now))
            || (chip && !self.editor_panel_open(now))
    }

    /// Is a panel open that a finger revealed (the player, the editor's)?
    /// It stays until a touch lands outside the island.
    pub fn touch_revealed(&self, now: Time) -> bool {
        self.hovered && (self.want_player() || self.want_editor_panel(now))
    }

    /// Sideways over the bars or the open player, a finger changes tracks.
    pub fn touch_swipes_tracks(&self, px: f64, py: f64) -> bool {
        self.music_area_at(px, py)
    }

    pub fn set_eq_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        let hovered = self.on_music(px, py);
        if self.eq_hovered == hovered { return false; }
        self.eq_hovered = hovered;
        self.sync(now);
        true
    }

    pub fn set_editor_chip_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        let hovered = self.editor_chip_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h);
        if self.editor_chip_hovered == hovered { return false; }
        self.editor_chip_hovered = hovered;
        self.sync(now);
        true
    }

    /// Is the editor's panel open (so clicks go to it)?
    pub fn editor_panel_open(&self, now: Time) -> bool {
        self.editor_panel.state(now).x > 0.5 && self.nvim.is_some()
    }

    /// What the pointer is over in the editor's panel.
    pub fn editor_action_at(&self, cx: f64, px: f64, py: f64, now: Time) -> Option<editor::Action> {
        if !self.editor_panel_open(now) {
            return None;
        }
        let e = self.nvim.as_ref()?;
        let (x0, y0) = self.box_origin(cx, now);
        if self.output_view {
            return (px - x0 > editor::PANEL_W - 52.0 && py - y0 < 36.0)
                .then_some(editor::Action::CloseOutput);
        }
        if self.editor_command.is_some() { return None; }
        editor::hit(e, editor::PANEL_W, px - x0, py - y0, self.editor_diagnostics)
    }

    pub fn set_editor_hover(&mut self, action: Option<editor::Action>) -> bool {
        if action != self.editor_hover {
            self.editor_hover = action;
            return true;
        }
        false
    }

    /// A click in the editor's panel: what to ask Neovim to do.
    pub fn press_editor(&mut self, action: editor::Action, now: Time) -> Option<(String, String)> {
        if action == editor::Action::CloseOutput {
            self.output_open = false;
            self.sync(now);
            return None;
        }
        if action == editor::Action::Diagnostics {
            self.editor_diagnostics = !self.editor_diagnostics;
            self.editor_hover = None;
            return None;
        }
        let e = self.nvim.as_ref()?;
        self.editor_pressed = Some((action, now));
        let expr = match action {
            editor::Action::Buffer(id) => format!("execute('buffer {id}')"),
            editor::Action::Diagnostics | editor::Action::CloseOutput => unreachable!(),
            editor::Action::Diagnostic(index) => {
                let d = e.diagnostic_items.get(index)?;
                format!("luaeval('require(\"island\").jump({}, {}, {})')", d.buffer, d.line, d.col)
            },
            editor::Action::Save => "execute('silent! write')".to_string(),
            editor::Action::Format => "luaeval('(pcall(vim.lsp.buf.format) or true) and 1')".to_string(),
            editor::Action::CloseBuffer => "execute('silent! bdelete')".to_string(),
        };
        Some((e.server.clone(), expr))
    }
    pub fn control_at(&self, cx: f64, px: f64, py: f64, now: Time) -> Option<player::Control> {
        if !self.player_open(now) {
            return None;
        }
        let (x0, y0) = self.box_origin(cx, now);
        player::hit(player::PLAYER_W, px - x0, py - y0)
    }

    /// Remember the pointer's position over the player for the hover and
    /// the press feedback. A `Seek` hover is kept as a marker that the
    /// pointer is over the progress line (the line swells for it).
    pub fn set_hovered_control(&mut self, control: Option<player::Control>) -> bool {
        let same = match (self.hovered_control, control) {
            (Some(player::Control::Seek(_)), Some(player::Control::Seek(_))) => true,
            (a, b) => a == b,
        };
        if !same {
            self.hovered_control = control;
            return true;
        }
        false
    }

    pub fn is_scrubbing(&self) -> bool {
        self.scrub.is_some()
    }

    /// The pointer moved while dragging the playhead: follow it, and tell
    /// the player now and then so the sound follows too.
    pub fn scrub_to(&mut self, fraction: f64, now: Time) -> Option<(String, String, crate::dbus::PlayerCommand)> {
        let (_, last_sent) = self.scrub?;
        self.scrub = Some((fraction, last_sent));
        let song = self.song.as_ref()?;
        if song.now.length <= 0.0 || song.now.track_id.is_empty() || now - last_sent < SCRUB_INTERVAL {
            return None;
        }
        self.scrub = Some((fraction, now));
        let command = crate::dbus::PlayerCommand::Seek(fraction * song.now.length);
        Some((song.now.bus_name.clone(), song.now.track_id.clone(), command))
    }

    /// The button came up: land the playhead where it was left.
    pub fn end_scrub(&mut self, now: Time) -> Option<(String, String, crate::dbus::PlayerCommand)> {
        let (fraction, _) = self.scrub.take()?;
        let song = self.song.as_mut()?;
        let length = song.now.length;
        if length <= 0.0 || song.now.track_id.is_empty() {
            self.sync(now);
            return None;
        }
        song.set_position(fraction * length, now);
        let out = Some((song.now.bus_name.clone(), song.now.track_id.clone(), crate::dbus::PlayerCommand::Seek(fraction * length)));
        self.sync(now);
        out
    }

    /// Where a point in surface coordinates falls on the progress line.
    pub fn scrub_fraction(&self, cx: f64, px: f64, now: Time) -> f64 {
        let (x0, _) = self.box_origin(cx, now);
        player::track_fraction(player::PLAYER_W, px - x0)
    }

    pub fn press_control(&mut self, control: player::Control, now: Time) -> Option<(String, String, crate::dbus::PlayerCommand)> {
        let song = self.song.as_ref()?;
        self.pressed = Some((control, now));
        let command = match control {
            player::Control::Previous => { self.track_streak(false, now); crate::dbus::PlayerCommand::Previous }
            player::Control::PlayPause => crate::dbus::PlayerCommand::PlayPause,
            player::Control::Next => { self.track_streak(true, now); crate::dbus::PlayerCommand::Next }
            // the island's own business, see `clear_song`
            player::Control::Clear => return None,
            player::Control::Seek(fraction) => {
                if song.now.length <= 0.0 || song.now.track_id.is_empty() {
                    return None;
                }
                // a press on the line starts a drag: the playhead follows
                // the pointer until the button comes up
                self.scrub = Some((fraction, now));
                crate::dbus::PlayerCommand::Seek(fraction * song.now.length)
            }
        };
        let song = self.song.as_ref()?;
        Some((song.now.bus_name.clone(), song.now.track_id.clone(), command))
    }

    /// Where the box's top-left corner is right now, for hit testing.
    pub fn box_origin(&self, cx: f64, now: Time) -> (f64, f64) {
        let p = self.panel.state(now).x;
        let pl = self.player.state(now).x;
        let ed = self.editor_panel.state(now).x;
        let cx = cx - self.config.dodge * self.dodge.state(now).x * (1.0 - p);
        let w = launcher::PANEL_W * p + player::PLAYER_W * pl + editor::PANEL_W * ed;
        (cx - w / 2.0, -BOX_H * self.hide.state(now).x * (1.0 - p))
    }

    /// The size the box is heading for: whichever shape is being opened,
    /// or the plain line when none is.
    fn target_size(&self, line_w: f64, line_h: f64, now: Time) -> (f64, f64) {
        let mut size = (line_w, line_h);
        let mut take = |k: f64, w: f64, h: f64| {
            if k > 0.5 {
                size = (size.0.max(w), size.1.max(h));
            }
        };
        take(self.pill.target(), self.pill_w.state(now).x, self.pill_h.state(now).x);
        take(self.player.target(), player::PLAYER_W, player::PLAYER_H);
        take(self.editor_panel.target(), editor::PANEL_W, self.editor_height());
        take(self.panel.target(), launcher::PANEL_W, launcher::PANEL_H);
        size
    }

    /// Light along the screen's edges, on an output `w`×`h` with the
    /// island's surface centred on `cx`; painted under the island. Returns
    /// the strips it painted.
    pub fn draw_edges(&mut self, cr: &Context, w: f64, h: f64, cx: f64, now: Time) -> Vec<Rect> {
        // the light comes out of the island as it was last drawn, and rounds
        // the bar's drops
        self.edge.set_island(self.box_geom.filter(|_| !self.hidden.any() || self.pill.state(now).x > 0.05));
        self.edge.set_drops(if self.config.bar { self.bar.drop_sizes() } else { [None, None] });
        // Touch ID breathes round the screen while it waits for a finger
        let waiting = matches!(self.auth, Some(Auth::Waiting(_))) && self.config.edge_light;
        // the light begins once Touch ID's pill has grown
        self.edge.breathe(waiting.then_some(crate::edge::TOUCH_ID_BLUE), (now).max(self.auth_start + PILL_LIGHT_DELAY));
        // a running timer is a line round the screen
        let gauge = self.timer.as_ref().filter(|_| self.config.edge_light).map(|t| crate::edge::Gauge {
            progress: if t.counts_up() { t.fraction(now) } else { 1.0 - t.fraction(now) },
            colour: timer_colour(t.kind),
            running: t.running,
            comet: t.counts_up(),
        });
        self.edge.set_gauge(gauge);
        self.edge.set_ember(if self.low_battery && self.config.edge_light { 0.55 } else { 0.0 });
        // music: the bottom edge in the cover's colour, breathing with the bass
        let ambient = self.song.as_ref().filter(|s| s.playing() && self.config.ambilight).map(|song| {
            let colour = self.art.colour(&song.now.art_url).unwrap_or(self.config.accent);
            let bass = self.eq.first().copied().unwrap_or(0.0).clamp(0.0, 1.0);
            (colour, 0.12 + 0.5 * bass)
        });
        self.edge.set_ambient(ambient);
        // Focus: the frame gathers the screen in a little
        self.focus_frame.set(if self.dnd && self.config.edge_light { 1.0 } else { 0.0 }, now);
        self.edge.set_border(FOCUS_FRAME * self.focus_frame.value(now).max(0.0), self.config.screen_corners);
        let cx = cx - self.config.dodge * self.dodge.state(now).x * (1.0 - self.panel.state(now).x);
        let mut painted = self.edge.draw(cr, w, h, cx, self.config.screen_corners, now);
        painted.extend(self.curtain.draw_shutter(cr, w, h, self.config.screen_corners, now));
        // the screen closing into the island (or coming out of it): over
        // the light, under the island
        let island = self.line_rect.unwrap_or(Rect { x: cx - 75.0, y: 0.0, w: 150.0, h: BOX_H });
        if let Some(r) = self.curtain.draw(cr, w, h, self.config.screen_corners, island, now) {
            painted.push(r);
        }
        painted
    }

    /// Marks in the frame beyond the screen, drawn over its corners: the
    /// microphone's orange light. Returns what they cover.
    pub fn draw_frame_marks(&mut self, cr: &Context, renderer: &Renderer, w: f64, h: f64, now: Time) -> Vec<Rect> {
        // the top edge: pulled down round a tugged island, rippling
        let (cx, attach, sag) = self.tug_top.unwrap_or((self.island_cx(now), 0.0, 0.0));
        // the edge's ripples run along it and along what hangs from it —
        // the island's and the bar's drops' edges — to the screen's sides
        // (a tugged island is sheared off its box: the pull alone shows)
        let shore = crate::edge::Shore {
            corner: self.config.screen_corners,
            smoothing: self.config.smoothing,
            drops: if self.config.bar { self.bar.drop_sizes() } else { [None, None] },
            island: self.box_geom.filter(|_| self.tug_top.is_none()).map(|(b, r)| (b, r, FILLET)),
        };
        let mut marks = self.membrane.draw(cr, w, &shore, cx, attach, sag, now);
        marks.extend(crate::edge::mic_led(cr, w, self.mic_dot.value(now).clamp(0.0, 1.2)));
        marks.extend(self.side_hud.draw(cr, renderer, h, now));
        marks
    }

    /// Where the island's centre is on the surface now (dodging moves it).
    fn island_cx(&self, now: Time) -> f64 {
        self.surface_cx - self.config.dodge * self.dodge.state(now).x * (1.0 - self.panel.state(now).x)
    }

    /// A track skipped from the island: a streak of the music's colour runs
    /// along the top edge the way it went.
    pub fn track_streak(&mut self, forward: bool, now: Time) {
        if !self.config.edge_light || self.song.is_none() {
            return;
        }
        let colour = self.song.as_ref().and_then(|s| self.art.colour(&s.now.art_url)).unwrap_or(self.config.accent);
        let wave = if forward { crate::edge::Wave::StreakRight } else { crate::edge::Wave::StreakLeft };
        self.edge.pulse(wave, colour, now);
    }

    /// The bar's drops in the top corners, on a surface `w` wide; nothing
    /// when the bar is off.
    pub fn draw_bar(&mut self, cr: &Context, renderer: &Renderer, w: f64, now: Time) -> crate::bar::BarFrame {
        if !self.config.bar {
            return crate::bar::BarFrame::default();
        }
        // what the Control Center's worker reads is what the bar shows
        self.bar.volume = self.controls.volume.map(|v| (v, self.controls.muted));
        self.bar.layout = self.controls.layout.as_deref().map(|l| l.trim_end_matches(" (US)").to_string());
        // a fullscreen window: up into the edge with the island
        self.bar.set_hidden(self.hidden.fullscreen, now);
        let frame = self.bar.draw(cr, renderer, w, self.config.smoothing, now);
        // a workspace switched or a tray menu opened: a ripple from there
        if let Some((x, strength)) = self.bar.take_splash() {
            if self.config.edge_light && !self.hidden.fullscreen {
                self.membrane.ripple(x, strength, now);
            }
        }
        frame
    }

    /// Switch the screen off into the island (before locking or sleeping).
    pub fn switch_off(&mut self, now: Time) {
        self.curtain.close(now);
    }

    /// Let the screen out of the island again (unlocked, woken).
    pub fn switch_on(&mut self, now: Time) {
        self.curtain.open(now);
    }

    /// Has the screen gone all the way into the island?
    pub fn switched_off(&self, now: Time) -> bool {
        self.curtain.is_off(now)
    }

    /// The notch outline, traced counter-clockwise: the concave fillets at
    /// the top are right turns, the convex corners at the bottom left turns
    /// (`Corner::trace` mirrored).
    fn trace_notch(&self, cr: &Context, x0: f64, x1: f64, y0: f64, y1: f64, r: f64) {
        self.trace_notch_with(cr, x0, x1, y0, y1, r, FILLET);
    }

    /// The notch outline with fillets of radius `fillet`.
    #[allow(clippy::too_many_arguments)]
    fn trace_notch_with(&self, cr: &Context, x0: f64, x1: f64, y0: f64, y1: f64, r: f64, fillet: f64) {
        crate::drop::notch(cr, x0, x1, y0, y1, r, fillet, self.config.smoothing);
    }
}

/// The waiting fingerprint breathes through three blues (the reference
/// cycled 12 colours at 80 ms; this is the same path, continuous).
fn pulse_colour(t: Time) -> (f64, f64, f64) {
    let k = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * t / PULSE_PERIOD).cos();
    let (a, b, c) = (rgb(0x3A6F90), rgb(0x66D9FF), rgb(0xD9F6FF));
    if k < 0.5 { mix(a, b, k * 2.0) } else { mix(b, c, k * 2.0 - 1.0) }
}

/// The local time through strftime(3), e.g. "Mon Sep 15  1:05 PM" for the
/// default format. The process never calls setlocale, so names stay in the
/// C locale like the old island's clock.
pub fn strftime(format: &str) -> String {
    strftime_at(format, -1)
}

/// `strftime` for the Unix time `at` (now when negative), in local time.
pub fn strftime_at(format: &str, at: i64) -> String {
    #[repr(C)]
    struct Tm {
        tm_sec: i32, tm_min: i32, tm_hour: i32, tm_mday: i32, tm_mon: i32, tm_year: i32,
        tm_wday: i32, tm_yday: i32, tm_isdst: i32, tm_gmtoff: i64, tm_zone: *const u8,
    }
    unsafe extern "C" {
        fn time(t: *mut i64) -> i64;
        fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
        fn strftime(s: *mut u8, max: usize, format: *const std::ffi::c_char, tm: *const Tm) -> usize;
    }
    let Ok(format) = std::ffi::CString::new(format) else { return String::new() };
    let mut t = 0i64;
    let mut tm = Tm {
        tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 1, tm_mon: 0, tm_year: 0,
        tm_wday: 0, tm_yday: 0, tm_isdst: 0, tm_gmtoff: 0, tm_zone: std::ptr::null(),
    };
    let mut out = [0u8; 256];
    let len = unsafe {
        if at < 0 { time(&mut t); } else { t = at; }
        localtime_r(&t, &mut tm);
        strftime(out.as_mut_ptr(), out.len(), format.as_ptr(), &tm)
    };
    String::from_utf8_lossy(&out[..len]).into_owned()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    /// SF Pro Text Semibold 16 px: the clock and the song
    Text,
    /// the pill's caption, same face, 15.33 px
    Caption,
    /// subtitles in the launcher's list, 12.5 px
    Small,
    /// the player's timestamps, 11 px
    Tiny,
    /// the launcher's search field, 21 px
    Query,
    // Sheet typography (Apple's text styles, SF Pro Text):
    /// sheet titles, Semibold 17
    Title,
    /// row titles, Semibold 15
    Headline,
    /// running text, Regular 15
    Body,
    /// secondary lines, Regular 13
    Subhead,
    /// buttons and segments, Medium 14
    Label,
    /// fine print, Regular 12
    Footnote,
    /// the timer page's big reading, Regular 50, tabular digits
    Clock,
    /// the timer ruler's chosen time, Regular 28, tabular digits
    ClockSmall,
    /// the bar's items, Medium 16 (Waybar's face)
    Bar,
    /// the bar's focused workspace, Black 16
    BarBold,
    /// the percentage inside the bar's battery, Bold 10.5
    BatteryDigits,
}

const FONT_MEDIUM: &str = "SF Pro Text Medium";

impl Font {
    const ALL: [Font; 16] = [
        Font::Text, Font::Caption, Font::Small, Font::Tiny, Font::Query,
        Font::Title, Font::Headline, Font::Body, Font::Subhead, Font::Label, Font::Footnote,
        Font::Clock, Font::ClockSmall, Font::Bar, Font::BarBold, Font::BatteryDigits,
    ];

    fn face(self) -> (&'static str, f64) {
        match self {
            Font::Text => (FONT, FONT_PX),
            Font::Caption => (FONT, CAPTION_PX),
            Font::Small => (FONT_REGULAR, SMALL_PX),
            Font::Tiny => (FONT_REGULAR, TINY_PX),
            Font::Query => (FONT, QUERY_PX),
            Font::Title => (FONT, 17.0),
            Font::Headline => (FONT, 15.0),
            Font::Body => (FONT_REGULAR, 15.0),
            Font::Subhead => (FONT_REGULAR, 13.0),
            Font::Label => (FONT_MEDIUM, 14.0),
            Font::Footnote => (FONT_REGULAR, 12.0),
            Font::Clock => (FONT_REGULAR, 50.0),
            Font::ClockSmall => (FONT_REGULAR, 28.0),
            Font::Bar => (FONT_MEDIUM, 16.0),
            Font::BarBold => ("SF Pro Text Black", 16.0),
            Font::BatteryDigits => ("SF Pro Text Bold", 10.5),
        }
    }
}

/// Pango setup shared by all drawing: fonts, hinting for the fractional
/// scale, one pango context and a cache of shaped layouts (shaping the
/// same clock string sixty times a second would cost more than drawing).
pub struct Renderer {
    scale: f64,
    fonts: Vec<pango::FontDescription>,
    ctx: pango::Context,
    cache: RefCell<LayoutCache>,
    /// labels whose text changes animate (motion.rs)
    pub motion: crate::motion::Registry,
}

type LayoutKey = (Font, String, i32, i32);
const LAYOUT_CACHE_LIMIT: usize = 512;

#[derive(Default)]
struct LayoutCache {
    entries: HashMap<LayoutKey, (pango::Layout, u64)>,
    clock: u64,
}

impl LayoutCache {
    fn get(&mut self, key: &LayoutKey) -> Option<pango::Layout> {
        self.clock += 1;
        let (layout, touched) = self.entries.get_mut(key)?;
        *touched = self.clock;
        Some(layout.clone())
    }
    fn insert(&mut self, key: LayoutKey, layout: pango::Layout) {
        if self.entries.len() >= LAYOUT_CACHE_LIMIT {
            let oldest = self.entries.iter().min_by_key(|(_, (_, touched))| *touched).map(|(key, _)| key.clone());
            if let Some(oldest) = oldest { self.entries.remove(&oldest); }
        }
        self.clock += 1;
        self.entries.insert(key, (layout, self.clock));
    }
}

impl Renderer {
    pub fn new(scale: f64) -> Self {
        let fonts = Font::ALL
            .iter()
            .map(|font| {
                let (face, px) = font.face();
                let mut desc = pango::FontDescription::from_string(face);
                desc.set_absolute_size(px * pango::SCALE as f64);
                desc
            })
            .collect();
        // the context takes the scale from a scratch surface; frames keep
        // the same transform, so it stays valid
        let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).expect("scratch surface");
        let cr = Context::new(&scratch).expect("scratch context");
        cr.scale(scale, scale);
        let ctx = pangocairo::functions::create_context(&cr);
        let mut opts = cairo::FontOptions::new().expect("font options");
        opts.set_hint_metrics(cairo::HintMetrics::On);
        opts.set_hint_style(cairo::HintStyle::Slight);
        opts.set_antialias(cairo::Antialias::Gray);
        pangocairo::functions::context_set_font_options(&ctx, Some(&opts));
        ctx.set_round_glyph_positions(true);
        let renderer = Renderer { scale, fonts, ctx, cache: RefCell::new(LayoutCache::default()), motion: Default::default() };
        // Resolve common font faces at startup/scale changes, not on the
        // first visible frame of a newly opened panel.
        for font in Font::ALL {
            renderer.layout(&cr, "Aa Яя 0123456789", font).pixel_extents();
        }
        renderer
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Round a logical coordinate to a whole output pixel.
    pub fn snap(&self, v: f64) -> f64 {
        (v * self.scale).round() / self.scale
    }

    pub fn floor(&self, v: f64) -> f64 {
        (v * self.scale).floor() / self.scale
    }

    pub fn ceil(&self, v: f64) -> f64 {
        (v * self.scale).ceil() / self.scale
    }

    /// The two lines of a notification, each ellipsised to the pill's text
    /// column, and the width the pill needs for them.
    pub fn notice_layouts(&self, cr: &Context, note: &crate::notify::Notification) -> (pango::Layout, pango::Layout, f64, f64) {
        let app = self.layout_ellipsized(cr, &note.app, Font::Caption, NOTICE_MAX_TEXT, 1);
        let msg = self.layout_ellipsized(cr, &note.message(), Font::Text, NOTICE_MAX_TEXT, NOTICE_MAX_LINES);
        // the summary stays semibold and white, the body after it is
        // lighter and a touch dimmer (the way iOS sets a sender apart)
        if !note.summary.is_empty() && !note.body.is_empty() {
            let attrs = pango::AttrList::new();
            let start = (note.summary.len() + " — ".len()) as u32;
            let mut weight = pango::AttrInt::new_weight(pango::Weight::Normal);
            weight.set_start_index(start);
            attrs.insert(weight);
            let mut alpha = pango::AttrInt::new_foreground_alpha(0xD000);
            alpha.set_start_index(start);
            attrs.insert(alpha);
            msg.set_attributes(Some(&attrs));
        }
        let text_w = (app.pixel_extents().1.width() as f64).max(msg.pixel_extents().1.width() as f64);
        let image_extra = if note.icon.is_some() && note.image.is_some() { 88.0 } else { 0.0 };
        let w = NOTICE_PAD_L + NOTICE_ICON + NOTICE_GAP + text_w + NOTICE_PAD_R + image_extra;
        let text_h = app.pixel_extents().1.height() as f64 + NOTICE_LINE_GAP + msg.pixel_extents().1.height() as f64
            + if note.progress.is_some() { NOTICE_BAR } else { 0.0 };
        let action_extra = if note.actions.iter().any(|(key, _)| key != "default") { 39.0 } else { 0.0 };
        let h = (text_h + 2.0 * NOTICE_PAD_V).max(NOTICE_ICON + 2.0 * NOTICE_PAD_V - 6.0) + action_extra;
        (app, msg, w.max(NOTICE_MIN_W), h.max(PILL_H))
    }

    /// The pill's size for this notification.
    pub fn notice_size(&self, cr: &Context, note: &crate::notify::Notification) -> (f64, f64) {
        let (_, _, w, h) = self.notice_layouts(cr, note);
        (w, h)
    }

    /// A layout wrapped to at most `lines` lines inside `max` logical px,
    /// the last one cut with "…".
    pub fn layout_ellipsized(&self, cr: &Context, text: &str, font: Font, max: f64, lines: i32) -> pango::Layout {
        // Widths during a morph differ every frame, often by less than one
        // device pixel. Do not turn floating point noise into cache misses.
        let width = (self.snap(max.max(0.0)) * pango::SCALE as f64).round() as i32;
        let key = (font, text.to_string(), width, lines);
        if let Some(l) = self.cache.borrow_mut().get(&key) {
            return l;
        }
        let layout = self.layout(cr, text, font).copy();
        layout.set_width(width);
        layout.set_wrap(pango::WrapMode::WordChar);
        // a negative height means "this many lines" when ellipsizing
        layout.set_height(-lines);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        self.cache.borrow_mut().insert(key, layout.clone());
        layout
    }

    pub fn output_layout(&self, text: &str) -> pango::Layout {
        let layout = pango::Layout::new(&self.ctx);
        let mut font = pango::FontDescription::from_string("monospace");
        font.set_absolute_size(13.0 * pango::SCALE as f64);
        layout.set_font_description(Some(&font));
        layout.set_width(((editor::PANEL_W - 48.0) * pango::SCALE as f64) as i32);
        layout.set_wrap(pango::WrapMode::WordChar);
        layout.set_text(text);
        layout
    }

    /// A layout for `text`, glyph positions rounded to output pixels. (Not
    /// switched to fractional positions while moving: unhinted metrics give
    /// a text about 1 px narrower and shorter, and the swap at the end of a
    /// move read as the text changing size.)
    pub fn layout(&self, _cr: &Context, text: &str, font: Font) -> pango::Layout {
        let key = (font, text.to_string(), -1, 0);
        let mut cache = self.cache.borrow_mut();
        if let Some(l) = cache.get(&key) {
            return l;
        }
        let layout = pango::Layout::new(&self.ctx);
        layout.set_font_description(Some(&self.fonts[font as usize]));
        if matches!(font, Font::Clock | Font::ClockSmall) {
            // digits of one width, so a running clock never shifts
            let attrs = pango::AttrList::new();
            attrs.insert(pango::AttrFontFeatures::new("tnum"));
            layout.set_attributes(Some(&attrs));
        }
        layout.set_text(text);
        cache.insert(key, layout.clone());
        layout
    }
}

#[cfg(test)]
mod tests {
    /// Settle `island` and paint it over a desktop-coloured background into
    /// target/gallery/<name>.png, cropped to what it drew.
    fn gallery_shot(name: &str, island: &mut Island, t: Time) {
        gallery_shot_after(name, island, t, 2.0);
    }

    fn gallery_shot_after(name: &str, island: &mut Island, t: Time, settle: f64) {
        let scale = 1.5;
        let renderer = Renderer::new(scale);
        let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 960).unwrap();
        let ctx = Context::new(&scratch).unwrap();
        ctx.scale(scale, scale);
        for n in 0..(settle * 60.0) as usize {
            let now = t + n as f64 / 60.0;
            island.drain_assets(now, scale);
            island.advance(now);
            island.draw(&ctx, &renderer, 490.0, now);
        }
        let end = t + settle;
        island.advance(end);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 960).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(scale, scale);
        let gradient = cairo::LinearGradient::new(0.0, 0.0, 980.0, 640.0);
        gradient.add_color_stop_rgb(0.0, 0.10, 0.16, 0.22);
        gradient.add_color_stop_rgb(1.0, 0.26, 0.42, 0.48);
        cr.set_source(&gradient).unwrap();
        cr.paint().unwrap();
        let frame = island.draw(&cr, &renderer, 490.0, end);
        drop(cr);
        let b = frame.bounds.unwrap_or(Rect { x: 0.0, y: 0.0, w: 980.0, h: 640.0 });
        let (x0, x1) = (((b.x - 24.0) * scale).max(0.0) as i32, ((b.x + b.w + 24.0) * scale).min(1470.0) as i32);
        let y1 = ((b.y + b.h + 24.0) * scale).min(960.0) as i32;
        let out = cairo::ImageSurface::create(cairo::Format::ARgb32, x1 - x0, y1).unwrap();
        let oc = Context::new(&out).unwrap();
        oc.set_source_surface(&surface, -x0 as f64, 0.0).unwrap();
        oc.paint().unwrap();
        drop(oc);
        std::fs::create_dir_all("target/gallery").unwrap();
        out.write_to_png(&mut std::fs::File::create(format!("target/gallery/{name}.png")).unwrap()).unwrap();
    }

    fn menu_style_top() -> f64 { crate::menu_style::CONTENT_TOP }

    pub(super) fn gallery_song(playing: bool) -> crate::dbus::NowPlaying {
        crate::dbus::NowPlaying {
            bus_name: "gallery".into(), app: "Firefox".into(), playing, artist: "Nils Frahm".into(),
            title: "Says".into(), art_url: String::new(), track_id: "gallery".into(), length: 534.0,
        }
    }

    pub(super) fn gallery_island_pub() -> Island { gallery_island() }

    fn gallery_island() -> Island {
        let mut island = Island::new(Config::default());
        island.clock = "Mon Oct 5  9:41 AM".into();
        island.sync(10.0);
        island
    }

    /// Frames of `island` from `t` every `step` seconds, side by side, into
    /// target/gallery/<name>.png (motion review).
    fn filmstrip(name: &str, island: &mut Island, t: Time, frames: usize, step: f64) {
        filmstrip_with(name, island, &Renderer::new(1.0), t, frames, step);
    }

    fn filmstrip_with(name: &str, island: &mut Island, renderer: &Renderer, t: Time, frames: usize, step: f64) {
        let (cw, ch) = (760.0, 560.0);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, (cw * frames as f64) as i32, ch as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.set_source_rgb(0.18, 0.30, 0.36);
        cr.paint().unwrap();
        for n in 0..frames {
            let now = t + n as f64 * step;
            island.advance(now);
            cr.save().unwrap();
            cr.rectangle(n as f64 * cw, 0.0, cw, ch);
            cr.clip();
            island.draw(&cr, renderer, n as f64 * cw + cw / 2.0, now);
            cr.restore().unwrap();
        }
        drop(cr);
        std::fs::create_dir_all("target/gallery").unwrap();
        surface.write_to_png(&mut std::fs::File::create(format!("target/gallery/{name}.png")).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "writes target/gallery/motion-*.png; set ISLAND_GALLERY=1"]
    fn text_motion_filmstrips() {
        if std::env::var_os("ISLAND_GALLERY").is_none() { return; }
        let mut island = gallery_island();
        island.clock = "Mon Oct 5  9:59 AM".into();
        island.sync(10.0);
        let renderer = Renderer::new(1.0);
        filmstrip_with("motion-clock-before", &mut island, &renderer, 10.0, 1, 0.0);
        island.clock = "Mon Oct 5  10:00 AM".into();
        filmstrip_with("motion-clock", &mut island, &renderer, 10.5, 10, 0.045);

        let mut island = gallery_island();
        island.open_quick_menu(10.0);
        island.set_controls(crate::control::Snapshot {
            volume: Some((0.45, false)), brightness: Some(0.7), layout: Some("Russian".into()),
            ..Default::default()
        }, 10.0);
        filmstrip("motion-quick-open", &mut island, 10.0, 10, 0.05);
        island.set_dnd(true, 11.0);
        island.set_layout("English (US)".into(), 11.0);
        filmstrip("motion-quick-toggle", &mut island, 11.0, 10, 0.045);
    }

    #[test]
    #[ignore = "writes target/gallery/tug-*.png; set ISLAND_GALLERY=1"]
    fn tug_filmstrips() {
        if std::env::var_os("ISLAND_GALLERY").is_none() { return; }
        let pull = |name: &str, to: (f64, f64)| {
            let mut island = gallery_island();
            // at rest first, with the strip's own renderer
            filmstrip("tug-rest", &mut island, 10.0, 30, 1.0 / 60.0);
            let line = island.line_rect.expect("the closed line");
            let from = (line.x + line.w / 2.0, line.y + line.h / 2.0);
            assert!(island.tug_press(from.0, from.1, 11.0));
            for i in 1..=10 {
                let k = i as f64 / 10.0;
                island.tug_to(from.0 + to.0 * k, from.1 + to.1 * k, 11.0 + k * 0.15);
            }
            filmstrip(&format!("{name}-held"), &mut island, 11.2, 1, 0.0);
            island.tug_release(11.2);
            filmstrip(name, &mut island, 11.2, 14, 2.0 / 60.0);
        };
        pull("tug-down", (0.0, 160.0));
        pull("tug-side", (220.0, 30.0));
    }

    #[test]
    #[ignore = "writes target/gallery/close-*.png; set ISLAND_GALLERY=1"]
    fn close_filmstrips() {
        if std::env::var_os("ISLAND_GALLERY").is_none() { return; }
        let mut island = gallery_island();
        for n in 0..3u32 {
            let mut note = crate::notify::fabricate(["Telegram", "Firefox", "notify-send"][n as usize], "Message", "Body");
            note.id = n + 1;
            island.center.upsert(note, 5.0);
        }
        island.open_notification_center(10.0);
        gallery_shot_after("close-center-open", &mut island, 10.0, 1.0);
        island.close_activity_panel(11.0);
        filmstrip("close-center", &mut island, 11.0, 12, 2.0 / 60.0);

        let mut island = gallery_island();
        island.song = Some(Song { now: gallery_song(true), position: 0.0, position_at: 10.0 });
        island.reveal.set_at(1.0, 10.0);
        island.timer_command("15m", 10.0).unwrap();
        island.sync(10.0);
        island.advance(12.0);
        island.config.compact.clock = false;
        island.sync(12.0);
        filmstrip("wings-part", &mut island, 12.0, 10, 3.0 / 60.0);

        let mut island = gallery_island();
        island.open_quick_menu(10.0);
        gallery_shot_after("close-quick-open", &mut island, 10.0, 1.0);
        island.close_activity_panel(11.0);
        filmstrip("close-quick", &mut island, 11.0, 12, 2.0 / 60.0);

        // the floating buttons drip out as the menu opens, and back in
        let mut island = gallery_island();
        island.open_quick_menu(10.0);
        filmstrip("floats-open", &mut island, 10.0, 12, 2.5 / 60.0);
        island.close_quick_menu(12.0);
        filmstrip("floats-close", &mut island, 12.0, 14, 1.5 / 60.0);

        let mut island = gallery_island();
        island.open_notification_center(10.0);
        filmstrip("open-center", &mut island, 10.0, 12, 2.0 / 60.0);
        island.advance(11.0);
        island.open_quick_menu_shortcut(11.0);
        filmstrip("swap-center-quick", &mut island, 11.0, 12, 2.0 / 60.0);

        let mut island = gallery_island();
        island.open_calendar(10.0);
        island.calendar = Some(Calendar::at(calendar::Date { year: 2026, month: 10, day: 5 }, 10.0));
        island.advance(11.0);
        island.calendar.as_mut().unwrap().key("PageDown", 12.0);
        filmstrip("calendar-month", &mut island, 12.0, 12, 4.0 / 60.0);
        island.calendar.as_mut().unwrap().key("PageUp", 13.0);
        filmstrip("calendar-month-back", &mut island, 13.0, 12, 4.0 / 60.0);
    }

    /// Every inner surface, settled, at the display's 1.5 scale:
    /// `ISLAND_GALLERY=1 cargo test gallery -- --ignored --nocapture`.
    #[test]
    #[ignore = "writes target/gallery/*.png; set ISLAND_GALLERY=1"]
    fn gallery() {
        if std::env::var_os("ISLAND_GALLERY").is_none() { return; }
        use crate::network::{Device, Kind};
        let device = |id: &str, name: &str, paired: bool, security: &str, signal: u8, icon: &str| Device {
            id: id.into(), name: name.into(), interface: "wlan0".into(), profile: paired.then(|| format!("uuid-{id}")),
            connected: false, paired, security: security.into(), signal, battery: None, icon: icon.into(),
        };

        let mut island = gallery_island();
        island.song = Some(Song { now: gallery_song(true), position: 141.0, position_at: 10.0 });
        island.reveal.set_at(1.0, 10.0);
        island.set_eq([0.7, 0.4, 0.9, 0.5], 10.0);
        gallery_shot("01-compact-music", &mut island, 10.0);
        island.config.compact.clock = false;
        island.sync(10.0);
        gallery_shot("01b-compact-music-no-clock", &mut island, 11.0);
        island.timer_command("15m", 13.0).unwrap();
        island.sync(13.0);
        gallery_shot("01d-wings-music-timer", &mut island, 13.0);
        let mut timer_only = gallery_island();
        timer_only.config.compact.clock = false;
        timer_only.sync(10.0);
        timer_only.timer_command("15m", 10.0).unwrap();
        timer_only.sync(10.0);
        gallery_shot("01e-wings-timer", &mut timer_only, 10.0);
        let mut island = gallery_island();
        island.song = Some(Song { now: gallery_song(true), position: 141.0, position_at: 10.0 });
        island.reveal.set_at(1.0, 10.0);
        island.set_eq([0.7, 0.4, 0.9, 0.5], 10.0);
        island.config.compact.clock = false;
        island.sync(10.0);
        island.advance(11.0);
        island.config.compact = Compact::NONE;
        island.sync(11.0);
        gallery_shot("01c-compact-resting", &mut island, 12.0);
        island.config.compact = Compact::ALL;
        island.sync(12.0);
        island.advance(13.0);

        let controls = crate::control::Snapshot {
            volume: Some((0.45, false)),
            brightness: Some(0.7),
            sinks: vec![
                crate::control::Sink { name: "spk".into(), label: "Динамики".into(), kind: crate::control::SinkKind::Speaker },
                crate::control::Sink { name: "bt".into(), label: "AirPods Pro".into(), kind: crate::control::SinkKind::Bluetooth },
                crate::control::Sink { name: "hdmi".into(), label: "HDMI / DisplayPort 1".into(), kind: crate::control::SinkKind::Display },
            ],
            default_sink: "bt".into(),
            layout: Some("Russian".into()),
            apps: vec![
                crate::control::App { name: "Firefox".into(), icon: "firefox-esr".into(), streams: vec![1, 2], volume: 0.8, muted: false },
                crate::control::App { name: "Telegram".into(), icon: "org.telegram.desktop".into(), streams: vec![3], volume: 0.35, muted: false },
            ],
            mic: Some((0.7, false)),
            mic_label: Some("Встроенный микрофон".into()),
            night: true,
        };
        let mut island = gallery_island();
        island.open_quick_menu(10.0);
        island.set_controls(controls.clone(), 10.0);
        gallery_shot("02-quick-menu", &mut island, 10.0);
        {
            let mut playing = gallery_island();
            playing.song = Some(Song { now: gallery_song(true), position: 141.0, position_at: 10.0 });
            playing.open_quick_menu(10.0);
            playing.set_controls(controls.clone(), 10.0);
            playing.quick_hover = Some(quick::Action::MediaPlayPause);
            gallery_shot("02e-quick-menu-playing", &mut playing, 10.0);
        }
        island.arm_logout(12.0);
        island.quick_hover = Some(quick::Action::Sleep);
        island.sync(12.0);
        gallery_shot_after("02b-quick-menu-armed", &mut island, 12.0, 0.6);
        island.close_quick_menu(13.0);
        island.recorder = true;
        island.recorder_event(crate::recorder::Event::Started { pid: 1 }, 13.0);
        island.open_quick_menu(14.0);
        gallery_shot("02f-quick-menu-recording", &mut island, 14.0);
        island.quick_show(quick::View::Sound, 13.0);
        for _ in 0..100 {
            island.drain_assets(13.0, 1.5);
            std::thread::sleep(std::time::Duration::from_millis(5));
            let renderer = Renderer::new(1.5);
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
            let ctx = Context::new(&scratch).unwrap();
            ctx.scale(1.5, 1.5);
            island.draw(&ctx, &renderer, 490.0, 13.0);
        }
        gallery_shot("02c-quick-sound", &mut island, 13.0);
        island.toggle_mic(16.0);
        island.controls.apps.clear();
        gallery_shot_after("02d-quick-sound-muted", &mut island, 16.0, 1.0);
        let mut sections = gallery_island();
        sections.timer_command("15m", 10.0).unwrap();
        sections.open_sections(10.0);
        sections.quick_key("next", 10.0);
        gallery_shot("02g-sections", &mut sections, 10.0);


        let mut island = gallery_island();
        island.timer_command("menu", 10.0).unwrap();
        gallery_shot("03-timer-menu", &mut island, 10.0);
        island.timer_page.mode = timer_page::Mode::Pomodoro;
        gallery_shot("03b-timer-pomodoro", &mut island, 12.0);
        island.timer_page.mode = timer_page::Mode::Stopwatch;
        gallery_shot("03c-timer-stopwatch", &mut island, 14.0);
        let mut island = gallery_island();
        island.timer_page.pomodoro.focus = 1;
        island.timer_command("pomodoro", 0.0).unwrap();
        island.timer_command("menu", 0.0).unwrap();
        gallery_shot("03d-timer-pomodoro-next", &mut island, 61.0);
        let mut island = gallery_island();
        island.timer_page.pomodoro.focus = 1;
        island.timer_command("pomodoro", 0.0).unwrap();
        gallery_shot_after("03e-timer-pomodoro-done", &mut island, 60.5, 1.0);

        let mut island = gallery_island();
        island.timer_command("25m", 10.0).unwrap();
        island.timer_command("menu", 10.0).unwrap();
        gallery_shot("04-timer-running-menu", &mut island, 10.0);

        let mut island = gallery_island();
        island.timer_command("25m", 10.0).unwrap();
        gallery_shot("05-timer-compact", &mut island, 10.0);

        {
            let mut island = gallery_island();
            island.recorder = true;
            island.toggle_recording(10.0).unwrap();
            gallery_shot_after("05l-record-countdown", &mut island, 10.0, 1.2);
            let mut missing = gallery_island();
            missing.show_activity(activity::Event::RecorderMissing, 10.0);
            gallery_shot_after("05k-record-missing", &mut missing, 10.0, 1.0);
            let mut island = gallery_island();
            island.recorder_event(crate::recorder::Event::Started { pid: 1 }, 8.0);
            gallery_shot("05m-recording", &mut island, 10.0);
            island.recorder_event(crate::recorder::Event::Finished {
                path: Some("/tmp/2026-10-05_13-40-00.mp4".into()), thumb: None }, 52.0);
            gallery_shot("05n-recording-saved", &mut island, 52.0);
        }

        {
            let mut island = gallery_island();
            island.set_mic_users(vec!["Telegram Desktop".into()], 9.0);
            island.set_dnd(true, 9.0);
            gallery_shot("05j-mic-in-use", &mut island, 10.0);
            island.live_activity(activity::Event::MicMute { muted: true }, 11.0);
            gallery_shot_after("05k-mic-muted", &mut island, 11.0, 1.0);
        }

        {
            let mut island = gallery_island();
            island.open_calendar(10.0);
            island.calendar = Some(Calendar::at(calendar::Date { year: 2026, month: 10, day: 5 }, 10.0));
            gallery_shot("05f-calendar", &mut island, 10.0);
            crate::weather::inject(r#"{"current":{"temperature_2m":11.6,"weather_code":2,"is_day":1},
                "daily":{"time":["2026-10-05","2026-10-06","2026-10-07","2026-10-08","2026-10-09","2026-10-10","2026-10-11"],
                "weather_code":[2,61,3,0,95,71,45],
                "temperature_2m_max":[14.2,9.8,8.0,12.5,15.1,1.0,6.0],"temperature_2m_min":[5.1,3.6,2.0,1.5,7.7,-4.2,0.5]}}"#, "Самара");
            island.calendar = Some(Calendar::at(calendar::Date { year: 2026, month: 10, day: 5 }, 10.0));
            gallery_shot("05f2-calendar-weather", &mut island, 10.0);
            island.calendar.as_mut().unwrap().key("Down", 12.0);
            island.calendar.as_mut().unwrap().key("Right", 12.0);
            island.calendar.as_mut().unwrap().pointer(200.0, 230.0, 12.0);
            gallery_shot("05g-calendar-selected", &mut island, 12.0);
            island.calendar.as_mut().unwrap().key("PageDown", 15.0);
            gallery_shot_after("05h-calendar-switching", &mut island, 15.0, 0.12);
        }

        {
            use crate::tasks::{Op, Request};
            let req = |id: &str, op| Request { id: id.into(), op, owner: None };
            let mut island = gallery_island();
            island.task(req("build", Op::Start { title: "cargo build".into(), after: 0.0 }), 4.0).unwrap();
            gallery_shot_after("05b-task-running", &mut island, 10.0, 1.0);
            island.task(req("copy", Op::Start { title: "Копирование фото".into(), after: 0.0 }), 10.0).unwrap();
            island.task(req("copy", Op::Set { progress: Some(0.4), detail: Some("412 из 1030 файлов".into()) }), 10.0).unwrap();
            gallery_shot_after("05c-task-progress", &mut island, 10.0, 1.0);
            island.task(req("backup", Op::Start { title: "Резервная копия".into(), after: 0.0 }), 11.0).unwrap();
            island.task(req("backup", Op::Fail { detail: Some("диск отключён".into()) }), 11.5).unwrap();
            island.click_tasks_at(-1.0, -1.0, 11.5);
            island.show_tasks(12.0);
            gallery_shot_after("05d-task-pill", &mut island, 12.0, 1.0);
            island.task(req("build", Op::Done { detail: None }), 20.0).unwrap();
            let mut done = gallery_island();
            done.task(req("build", Op::Start { title: "cargo build".into(), after: 0.0 }), 1.0).unwrap();
            done.task(req("build", Op::Done { detail: None }), 1.5).unwrap();
            gallery_shot_after("05e-task-done", &mut done, 1.5, 1.0);
        }

        let wifi = || vec![
            Device { connected: true, ..device("home", "Home Wi-Fi", true, "WPA2", 82, "") },
            device("studio", "Studio · 5 GHz", true, "WPA3", 64, ""),
            device("guest", "Guest", false, "", 40, ""),
            device("long", "Neighbourhood network with a long name", false, "WPA2", 22, ""),
        ];
        let mut island = gallery_island();
        island.network_panel = Some(crate::connections::Panel::preview(Kind::Wifi, wifi(), None, 10.0));
        island.live_activity(activity::Event::WifiOpen, 10.0);
        gallery_shot("06-wifi", &mut island, 10.0);
        if let Some(panel) = island.network_panel.as_mut() { panel.pointer(200.0, 120.0 + 52.0 + 20.0, 12.0); }
        gallery_shot("06b-wifi-hover", &mut island, 12.0);
        // a saved network: its bin beside Connect, then armed by a first press
        if let Some(panel) = island.network_panel.as_mut() { panel.pointer(200.0, 120.0 + 52.0 * 1.5 + 0.0, 14.0); }
        gallery_shot("06c-wifi-forget", &mut island, 14.0);
        if let Some(panel) = island.network_panel.as_mut() {
            if let Some((x, y)) = panel.forget_centre("studio", 16.0) {
                panel.pointer(x, y, 16.0);
                panel.click(x, y, 16.0);
            }
        }
        gallery_shot_after("06d-wifi-forget-armed", &mut island, 16.0, 1.0);

        let mut island = gallery_island();
        island.network_panel = Some(crate::connections::Panel::preview(Kind::Wifi, wifi(), Some(1), 10.0));
        island.live_activity(activity::Event::WifiOpen, 10.0);
        gallery_shot("07-wifi-password", &mut island, 10.0);

        let mut island = gallery_island();
        let bt = vec![
            Device { connected: true, battery: Some(84), ..device("a", "AirPods Pro", true, "", 0, "audio-headphones") },
            device("k", "Magic Keyboard", true, "", 0, "input-keyboard"),
            device("m", "MX Master 3S", true, "", 0, "input-mouse"),
        ];
        island.network_panel = Some(crate::connections::Panel::preview(Kind::Bluetooth, bt, None, 10.0));
        island.live_activity(activity::Event::BluetoothOpen, 10.0);
        gallery_shot("08-bluetooth", &mut island, 10.0);

        let mut island = gallery_island();
        for (n, (app, summary, body)) in [
            ("Telegram", "Alice", "Are we still on for tonight? I booked the table for eight."),
            ("Telegram", "Bob", "Sent you the photos"),
            ("Firefox", "Download complete", "island-shell.tar.gz"),
            ("notify-send", "Backup finished", "412 files, 1.2 GB"),
        ].into_iter().enumerate() {
            let mut note = crate::notify::fabricate(app, summary, body);
            note.id = n as u32 + 1;
            island.center.upsert(note, 5.0 + n as f64);
        }
        island.open_notification_center(10.0);
        gallery_shot("09-notification-center", &mut island, 10.0);
        {
            let mut scrolled = gallery_island();
            for n in 0..7 {
                let apps = ["Telegram", "Firefox", "Discord", "notify-send", "Thunderbird", "Slack", "Steam"];
                let mut note = crate::notify::fabricate(apps[n as usize], &format!("Message {n}"), "Body text");
                note.id = 100 + n;
                scrolled.center.upsert(note, 5.0);
            }
            scrolled.open_notification_center(10.0);
            gallery_shot_after("09c-notification-scroll-settled", &mut scrolled, 10.0, 1.0);
            scrolled.center.scroll(1.5, 11.0);
            gallery_shot_after("09d-notification-scroll-moving", &mut scrolled, 11.0, 0.06);
        }
        island.center.click(90.0, menu_style_top() + 12.0, 13.0);
        gallery_shot("09b-notification-detail", &mut island, 13.0);
        {
            let mut replying = gallery_island();
            let mut note = crate::notify::fabricate("Telegram", "Alice", "Are we still on for tonight? I booked the table for eight.");
            note.id = 40;
            note.actions = vec![("default".into(), "Открыть".into()), ("inline-reply".into(), "Ответить".into())];
            note.reply = Some("Сообщение".into());
            replying.center.upsert(note, 5.0);
            replying.open_notification_center(10.0);
            replying.center.click(90.0, menu_style_top() + 12.0, 11.0);
            for key in ["Д", "а", ",", " ", "в", " ", "8"] { replying.center_reply_key("", Some(key), false, 12.0); }
            gallery_shot_after("09e-notification-detail-reply", &mut replying, 12.0, 1.0);
            let sent = replying.center_reply_key("Return", None, false, 13.0);
            assert_eq!(sent, Some(notice_center::Action::Reply(40, "Да, в 8".into())));
            replying.center.closed(40);
            gallery_shot_after("09f-notification-detail-sent", &mut replying, 13.0, 0.8);
        }

        let mut island = gallery_island();
        island.notify(crate::notify::fabricate("Telegram", "Alice", "Are we still on for tonight? I booked the table for eight, and the place is near the river."), 10.0);
        gallery_shot("10-notification", &mut island, 10.0);

        let mut island = gallery_island();
        let mut note = crate::notify::fabricate("Firefox", "Загрузка", "island-shell-0.9.tar.gz — 12 из 30 МБ");
        note.id = 31;
        note.progress = Some(0.4);
        island.notify(note, 10.0);
        gallery_shot("10b-notification-progress", &mut island, 10.0);

        let mut island = gallery_island();
        let mut note = crate::notify::fabricate("Telegram", "Alice", "Are we still on for tonight?");
        note.id = 32;
        note.actions = vec![("default".into(), "Open".into()), ("inline-reply".into(), "Ответить".into())];
        note.reply = Some("Сообщение".into());
        island.notify(note, 10.0);
        gallery_shot("10c-notification-reply-button", &mut island, 10.0);
        island.begin_reply(12.0);
        for key in ["Д", "а", ",", " ", "в", " ", "8"] { island.reply_key("", Some(key), false, 12.5); }
        gallery_shot_after("10d-notification-replying", &mut island, 12.5, 1.0);

        let mut island = gallery_island();
        island.song = Some(Song { now: gallery_song(true), position: 141.0, position_at: 10.0 });
        island.reveal.set_at(1.0, 10.0);
        island.set_eq([0.7, 0.4, 0.9, 0.5], 10.0);
        let renderer = Renderer::new(1.5);
        let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let ctx = Context::new(&scratch).unwrap();
        ctx.scale(1.5, 1.5);
        island.draw(&ctx, &renderer, 490.0, 10.1);
        let eq = island.eq_rect.unwrap();
        island.hovered = true;
        island.set_eq_hover(eq.x + eq.w / 2.0, eq.y + 10.0, 10.2);
        gallery_shot("11-player", &mut island, 10.2);

        let mut island = gallery_island();
        island.set_player(Some(&gallery_song(true)), 10.0);
        gallery_shot_after("12-now-playing-toast", &mut island, 10.0, 1.2);

        let mut island = gallery_island();
        island.live_activity(activity::Event::Slider { kind: activity::SliderKind::Volume, value: 0.65, muted: false }, 10.0);
        gallery_shot_after("13-volume", &mut island, 10.0, 0.9);

        let mut island = gallery_island();
        island.live_activity(activity::Event::Charging { percent: 72 }, 10.0);
        gallery_shot("14-charging", &mut island, 10.0);

        let mut island = gallery_island();
        let ready = std::time::Instant::now();
        while island.catalog.apps.is_empty() && ready.elapsed().as_secs() < 5 {
            island.drain_assets(9.0, 1.5);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        island.toggle_launcher(10.0);
        for _ in 0..200 {
            island.drain_assets(10.0, 1.5);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        gallery_shot("15-launcher", &mut island, 10.0);
        island.launcher.as_mut().unwrap().push("2*(17+5)^2", 12.0);
        gallery_shot("15b-launcher-calc", &mut island, 12.0);
        island.launcher.as_mut().unwrap().clear(14.0);
        island.launcher.as_mut().unwrap().push("5 миль в км", 14.0);
        gallery_shot("15c-launcher-convert", &mut island, 14.0);
        island.launcher.as_mut().unwrap().clear(16.0);
        island.launcher.as_mut().unwrap().push("clip:", 16.0);
        for _ in 0..200 {
            island.drain_assets(16.0, 1.5);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        gallery_shot("15d-launcher-clipboard", &mut island, 16.0);

        let mut island = gallery_island();
        let editor = Editor {
            server: "gallery".into(), focused: true, mode: "n".into(), file: "island.rs".into(),
            dir: "~/it/coding/island-shell/src".into(), lang: "rust".into(), modified: true,
            line: 1690, col: 9, lines: 3480, percent: 48, encoding: "utf-8".into(), indent: "4 spaces".into(),
            branch: "main".into(), added: 42, removed: 7, cwd: "~/it/coding/island-shell".into(),
            buffers: vec![
                nvim::Buffer { id: 1, name: "island.rs".into(), lang: "rust".into(), modified: true, current: true },
                nvim::Buffer { id: 2, name: "app.rs".into(), lang: "rust".into(), modified: false, current: false },
                nvim::Buffer { id: 3, name: "README.md".into(), lang: "markdown".into(), modified: false, current: false },
            ],
            ..Editor::default()
        };
        island.set_editor(editor, 10.0);
        island.draw(&ctx, &renderer, 490.0, 10.5);
        let chip = island.editor_chip_rect.unwrap();
        island.hovered = true;
        island.set_editor_chip_hover(chip.x + chip.w / 2.0, chip.y + 10.0, 10.6);
        gallery_shot("16-editor", &mut island, 10.6);

        let mut island = gallery_island();
        island.auth_started(10.0);
        gallery_shot("17-touch-id", &mut island, 10.0);
    }

    fn shown_notice(id: u32) -> (Island, Rect) {
        let mut island = Island::new(Config::default());
        let mut note = crate::notify::fabricate("Telegram", "Alice", "Hello there");
        note.id = id;
        note.actions = vec![("default".into(), "Open".into())];
        island.notify(note, 10.0);
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        for n in 0..40 { island.draw(&cr, &renderer, 490.0, 10.0 + n as f64 / 60.0); }
        let rect = island.notice_rect.expect("the pill is up");
        (island, rect)
    }

    #[test]
    fn a_short_swipe_springs_back_and_a_long_one_dismisses() {
        let (mut island, r) = shown_notice(7);
        let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        // two fingers, a little way: back it goes
        assert_eq!(island.swipe_notice(x, y, -30.0, false, 11.0), (true, None));
        assert_eq!(island.swipe_notice(x, y, 0.0, true, 11.02), (true, None));
        assert!(island.notice.is_some());
        // most of the way: it is gone and its ID comes back to be closed
        for n in 0..10 { island.swipe_notice(x, y, -20.0, false, 12.0 + n as f64 * 0.05); }
        assert_eq!(island.swipe_notice(x, y, 0.0, true, 12.6), (true, Some(7)));
        assert!(island.notice.is_none() && island.notice_swiped.is_some());

        // the mouse: a press that does not move is a click…
        let (mut island, r) = shown_notice(8);
        let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        assert!(island.press_notice(x, y, 11.0));
        assert!(island.drag_notice(x + 3.0, 11.05));
        assert_eq!(island.release_notice(x + 3.0, y, 11.1), NoticeRelease::Click(NoticeClick::Invoke(8, "default".into())));
        // …a slow drag past a third of the pill is a swipe
        assert!(island.press_notice(x, y, 12.0));
        for n in 1..=20 { island.drag_notice(x + n as f64 * 9.0, 12.0 + n as f64 * 0.1); }
        assert_eq!(island.release_notice(x + 180.0, y, 14.1), NoticeRelease::Dismissed(8));
    }

    #[test]
    fn a_swiped_card_leaves_the_notification_center_the_way_it_went() {
        let mut island = Island::new(Config::default());
        for (n, app) in ["Telegram", "Firefox"].into_iter().enumerate() {
            let mut note = crate::notify::fabricate(app, "Message", "Body");
            note.id = n as u32 + 1;
            island.center.upsert(note, 5.0);
        }
        island.open_notification_center(10.0);
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        for n in 0..60 { island.draw(&cr, &renderer, 490.0, 10.0 + n as f64 / 60.0); }
        let r = island.center_rect.expect("the center is up");
        let (x, y) = (r.x + 200.0, r.y + crate::menu_style::CONTENT_TOP + 20.0);
        for n in 0..10 { island.swipe_center(x, y, -20.0, false, 12.0 + n as f64 * 0.05); }
        let action = island.swipe_center(x, y, 0.0, true, 12.6).expect("the swipe was the center's");
        // the newest card (Firefox, id 2) went
        assert_eq!(action, notice_center::Action::Dismiss(2));
        assert_eq!(island.center.len(), 1);
    }

    #[test]
    fn an_inline_reply_is_typed_in_the_pill_and_sent_by_return() {
        let mut island = Island::new(Config::default());
        let mut note = crate::notify::fabricate("Telegram", "Alice", "Are we still on?");
        note.id = 21;
        note.actions = vec![("default".into(), "Open".into()), ("inline-reply".into(), "Ответить".into())];
        note.reply = Some("Сообщение".into());
        island.notify(note, 10.0);
        assert!(island.begin_reply(10.5));
        assert!(island.keyboard_panel_open());
        for key in ["Д", "а", "!"] { island.reply_key("", Some(key), false, 11.0); }
        island.reply_key("BackSpace", None, false, 11.1);
        // the pill waits for the reply however long it takes: the server is
        // asked to hold it, and even the app closing it does not take it
        assert_eq!(island.held_notice(), Some(21));
        island.notification_closed(21, 15.0);
        island.advance(10.0 + 60.0);
        assert!(island.notice.is_some() && island.replying());
        assert_eq!(island.reply_key("Return", None, false, 70.1), ReplyKey::Send(21, "Да".into()));
        assert!(island.notice.is_none() && !island.replying());

        // Escape gives the notification back, with its time restarted
        let mut note = crate::notify::fabricate("Telegram", "Bob", "Hi");
        note.id = 22;
        note.reply = Some("Сообщение".into());
        island.notify(note, 80.0);
        island.begin_reply(80.5);
        assert_eq!(island.reply_key("Escape", None, false, 81.0), ReplyKey::Cancel);
        assert!(island.notice.as_ref().is_some_and(|n| n.since == 81.0) && !island.replying());
        assert_eq!(island.held_notice(), None, "given back, it expires again");
    }

    #[test]
    fn the_pointer_on_a_notification_holds_it() {
        let (mut island, r) = shown_notice(30);
        island.set_notice_hover(r.x + r.w / 2.0, r.y + r.h / 2.0, 11.0);
        assert_eq!(island.held_notice(), Some(30));
        island.advance(30.0);
        assert!(island.notice.is_some(), "it waits while the pointer rests on it");
        island.set_notice_hover(-1.0, -1.0, 30.0);
        assert_eq!(island.held_notice(), None);
        island.advance(30.0 + island.config.notice_hold - 0.1);
        assert!(island.notice.is_some(), "let go, it has its full time again");
        island.advance(30.0 + island.config.notice_hold + 0.1);
        assert!(island.notice.is_none());
    }

    #[test]
    fn config_reads_every_key_and_rejects_nonsense() {
        let mut config = Config::default();
        assert_eq!(strftime(CLOCK_FORMAT), strftime(&config.clock));
        config.set("clock", "%H:%M").unwrap();
        assert_eq!(strftime(&config.clock).len(), 5);
        assert!(!config.clock_has_seconds());
        config.set("clock", "\"%H:%M:%S\"").unwrap();
        assert_eq!(config.clock, "%H:%M:%S");
        assert!(config.clock_has_seconds());
        assert!(config.set("clock", "").is_err());
        config.set("accent", "#FF9500").unwrap();
        assert_eq!(config.accent, (1.0, 149.0 / 255.0, 0.0));
        assert!(config.set("accent", "orange").is_err());
        config.set("notice_hold", "6s").unwrap();
        assert_eq!(config.notice_hold, 6.0);
        config.set("silent_after", "0").unwrap();
        assert_eq!(config.silent_after, 1.0);
        config.set("output", "HDMI-A-1").unwrap();
        assert!(config.set("output", "two words").is_err());
        config.set("hover", "off").unwrap();
        assert!(!config.hover);
        assert!(config.set("nonsense", "1").unwrap_err().contains("known:"));
        assert!(config.describe().contains("accent=#FF9500"));
        assert!(config.describe().contains("clock=\"%H:%M:%S\""));
    }

    #[test]
    fn silence_and_holds_follow_the_config() {
        let mut island = Island::new(Config::default());
        island.config.set("silent_after", "3").unwrap();
        island.silent_since = Some(10.0);
        assert!(!island.silent_expired(12.9));
        assert!(island.silent_expired(13.0));
    }

    use super::*;

    #[test]
    fn text_cache_is_bounded_and_does_not_flush_frequently_used_labels() {
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 2, 2).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        let clock = renderer.layout(&cr, "Clock", Font::Text);
        for n in 0..800 {
            assert_eq!(renderer.layout(&cr, "Clock", Font::Text), clock);
            renderer.layout(&cr, &format!("Other {n}"), Font::Text);
        }
        assert!(renderer.cache.borrow().entries.len() <= LAYOUT_CACHE_LIMIT);
        assert_eq!(renderer.layout_ellipsized(&cr, "Stable", Font::Text, 200.0, 1),
            renderer.layout_ellipsized(&cr, "Stable", Font::Text, 200.00001, 1));
    }

    #[test]
    fn steady_countdown_uses_deadlines_instead_of_continuous_frame_callbacks() {
        let mut island = Island::new(Config::default());
        island.timer_command("25m", 10.0).unwrap();
        island.advance(12.0);
        assert!(!island.is_animating(12.0));
        let next = island.deadline(12.25).unwrap();
        assert!(next > 12.5 && next <= 13.01);
    }

    #[test]
    fn notice_clicks_use_default_action_and_explicit_buttons() {
        let mut island = Island::new(Config::default());
        let mut note = crate::notify::fabricate("System", "Message", "Body");
        note.id = 42;
        note.actions = vec![("default".into(), "Open".into()), ("reply".into(), "Reply".into())];
        island.notify(note, 1.0);
        island.notice_rect = Some(Rect { x: 100.0, y: 0.0, w: 420.0, h: 130.0 });
        assert_eq!(island.notice_click_at(150.0, 40.0), Some(NoticeClick::Invoke(42, "default".into())));
        assert_eq!(island.notice_click_at(100.0 + NOTICE_PAD_L + 65.0, 100.0), Some(NoticeClick::Invoke(42, "reply".into())));
        assert_eq!(island.notice_click_at(90.0, 40.0), None);
    }

    #[test]
    fn dnd_keeps_normal_notifications_in_history_but_shows_critical() {
        let mut island = Island::new(Config::default());
        island.set_dnd(true, 0.0);
        let mut normal = crate::notify::fabricate("System", "normal", "body");
        normal.id = 1;
        island.notify(normal, 1.0);
        assert!(island.notice.is_none());
        assert_eq!(island.center.len(), 1);
        let mut critical = crate::notify::fabricate("Critical", "urgent", "body");
        critical.id = 2;
        island.notify(critical, 2.0);
        assert!(island.notice.as_ref().is_some_and(|notice| notice.note.critical));
        island.set_dnd(false, 3.0);
    }

    #[test]
    #[ignore = "Read-only offscreen performance benchmark against installed desktop entries"]
    fn animation_benchmark() {
        use std::time::Instant;
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        let mut island = Island::new(Config::default());
        island.update_clock();
        let ready = Instant::now();
        while island.catalog.apps.is_empty() && ready.elapsed().as_secs() < 5 {
            island.drain_assets(9.0, 1.5);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!island.catalog.apps.is_empty(), "benchmark needs real desktop entries");
        for cycle in 0..3 {
            let now = 10.0 + cycle as f64 * 3.0;
            let start = Instant::now();
            island.toggle_launcher(now);
            let open_ms = start.elapsed().as_secs_f64() * 1000.0;
            let mut frames = Vec::new();
            for n in 0..90 {
                let t = now + n as f64 / 60.0;
                let start = Instant::now();
                island.drain_assets(t, 1.5);
                cr.set_operator(cairo::Operator::Clear);
                cr.paint().unwrap();
                cr.set_operator(cairo::Operator::Over);
                island.advance(t);
                island.draw(&cr, &renderer, 490.0, t);
                frames.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            frames.sort_by(f64::total_cmp);
            eprintln!("launcher cycle={cycle} open={open_ms:.3}ms median={:.3}ms p95={:.3}ms max={:.3}ms >16ms={}",
                frames[45], frames[85], frames[89], frames.iter().filter(|v| **v > 16.667).count());
            island.close_launcher(now + 2.0);
            island.advance(now + 2.9);
        }
        for (label, event) in [
            ("timer", activity::Event::TimerMenu),
            ("volume", activity::Event::Slider { kind: activity::SliderKind::Volume, value: 0.65, muted: false }),
        ] {
            island.live_activity(event, 30.0);
            let start = Instant::now();
            for n in 0..60 {
                let t = 30.0 + n as f64 / 60.0;
                island.advance(t);
                island.draw(&cr, &renderer, 490.0, t);
            }
            eprintln!("{label} average={:.3}ms", start.elapsed().as_secs_f64() * 1000.0 / 60.0);
            island.close_activity_panel(31.0);
            island.advance(32.0);
        }
    }

    #[test]
    fn quick_menu_unfolds_and_routes_each_shortcut_without_launching_on_release() {
        let mut island = Island::new(Config::default());
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        assert!(island.open_quick_menu(10.0));
        assert!(island.keyboard_panel_open());
        island.draw(&cr, &renderer, 490.0, 10.7);
        let r = island.quick_rect.expect("menu should accept pointer input after unfolding");
        for i in 0..12 {
            let (action, x, y) = quick::shortcut_centre(i);
            assert_eq!(island.quick_action_at(r.x + x, r.y + y), Some(action));
        }
        let (first, x, y) = quick::shortcut_centre(0);
        assert_eq!(first, quick::Action::Wifi);
        island.set_quick_hover(r.x + x, r.y + y, 10.7);
        assert!(island.quick_hover_springs[&quick::Action::Wifi].value(10.8) > 0.0);
        island.close_quick_menu(10.8);
        assert!(!island.keyboard_panel_open());
        assert_eq!(island.quick_action_at(r.x + x, r.y + y), None);
    }

    #[test]
    fn quick_shortcut_temporarily_reveals_a_hidden_island() {
        let mut island = Island::new(Config::default());
        island.set_fullscreen(true, 1.0);
        assert!(island.is_hidden());
        assert!(!island.open_quick_menu(1.1));
        assert!(island.open_quick_menu_shortcut(1.1));
        assert!(island.quick_menu_open());
        assert!(!island.is_hidden());
        assert_eq!(island.hide.target(), 0.0);
        island.close_quick_menu(2.0);
        assert!(island.is_hidden());
        assert_eq!(island.hide.target(), 1.0);
    }

    #[test]
    fn timer_close_returns_to_quick_menu_but_escape_dismisses_it() {
        let mut island = Island::new(Config::default());
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        island.live_activity(activity::Event::TimerMenu, 10.0);
        island.draw(&cr, &renderer, 490.0, 10.8);
        let rect = island.timer_menu_rect.expect("timer controls should be interactive");
        let (x, y) = island.timer_page.centre(timer_page::Part::Close);
        assert!(island.click_timer_menu(rect.x + x, rect.y + y, 10.9));
        assert!(island.quick_menu_open());
        assert!(island.quick_returned_at.is_some());
        assert!(island.close_activity_panel(11.2));
        assert!(!island.quick_menu_open());
    }

    #[test]
    fn launcher_close_target_leads_back_to_quick_menu() {
        let mut island = Island::new(Config::default());
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        assert!(island.toggle_launcher(20.0));
        island.draw(&cr, &renderer, 490.0, 20.8);
        let (x, y) = island.box_origin(490.0, 20.8);
        assert!(island.launcher_close_at(490.0, x + launcher::PANEL_W - 32.0, y + 27.0, 20.8));
        island.close_launcher(20.8);
        island.return_to_quick_menu(20.8);
        assert!(island.launcher.is_none());
        assert!(island.quick_menu_open());
    }

    #[test]
    fn quick_menu_crossfades_into_notification_center() {
        let mut island = Island::new(Config::default());
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        assert!(island.open_quick_menu(10.0));
        island.draw(&cr, &renderer, 490.0, 10.6);
        island.close_quick_menu(10.7);
        assert!(island.open_notification_center(10.7));
        assert!(island.quick_retiring_at.is_some());
        assert!(island.is_animating(10.8));
        island.draw(&cr, &renderer, 490.0, 10.8);
        assert!(island.center_rect.is_none());
        island.advance(11.1);
        island.draw(&cr, &renderer, 490.0, 11.1);
        assert!(island.center_rect.is_some());
        assert!(island.quick_retiring_at.is_none());
    }

    #[test]
    fn quiet_mode_changes_without_closing_quick_menu() {
        let mut island = Island::new(Config::default());
        assert!(island.open_quick_menu(1.0));
        island.set_dnd(true, 1.1);
        assert!(island.quick_menu_open());
        assert!(island.dnd);
        island.set_dnd(false, 1.2);
        assert!(island.quick_menu_open());
    }

    #[test]
    fn music_and_editor_open_only_from_their_compact_hit_targets() {
        let mut island = Island::new(Config::default());
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 840).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        island.song = Some(Song { now: crate::dbus::NowPlaying {
            bus_name: "test".into(), app: "Player".into(), playing: true, artist: "A".into(),
            title: "T".into(), art_url: String::new(), track_id: "test".into(), length: 120.0,
        }, position: 0.0, position_at: 10.0 });
        island.reveal.set_at(1.0, 10.0);
        island.sync(10.0);
        island.draw(&cr, &renderer, 490.0, 10.5);
        let eq = island.eq_rect.unwrap();
        assert!(island.music_area_at(eq.x + eq.w / 2.0, eq.y + 10.0));
        assert!(!island.music_area_at(eq.x - 25.0, eq.y + 10.0));
        island.hovered = true;
        island.sync(10.5);
        assert_eq!(island.player.target(), 0.0);
        assert_eq!(island.hover_pad.target(), 1.0);
        let width_before_panel = island.hover_pad.value(10.6);
        assert!(width_before_panel > 0.0 && width_before_panel < 1.0);
        island.set_eq_hover(eq.x + eq.w / 2.0, eq.y + 10.0, 10.6);
        assert_eq!(island.player.target(), 1.0);
        assert_eq!(island.hover_pad.target(), 0.0);
        assert!((island.hover_pad.value(10.6) - width_before_panel).abs() < 1e-9);
        island.set_eq_hover(-1.0, -1.0, 10.7);
        assert_eq!(island.player.target(), 1.0, "moving into the panel keeps it open");
        island.draw(&cr, &renderer, 490.0, 10.75);
        let player = island.player_rect.expect("opening player has its own input region");
        assert!(island.music_area_at(player.x + player.w / 2.0, player.y + player.h / 2.0));
        assert!(!island.music_area_at(player.x - 20.0, player.y + 10.0));
        island.hovered = false;
        island.sync(10.8);
        assert_eq!(island.player.target(), 0.0);

        island.song = None;
        island.player.set_at(0.0, 11.0);
        island.set_editor(Editor { server: "test".into(), focused: true, file: "main.rs".into(), ..Editor::default() }, 11.0);
        island.draw(&cr, &renderer, 490.0, 11.5);
        let chip = island.editor_chip_rect.unwrap();
        island.hovered = true;
        island.sync(11.5);
        assert_eq!(island.editor_panel.target(), 0.0);
        assert_eq!(island.hover_pad.target(), 1.0);
        let width_before_editor = island.hover_pad.value(11.6);
        island.set_editor_chip_hover(chip.x + chip.w / 2.0, chip.y + 10.0, 11.6);
        assert_eq!(island.editor_panel.target(), 1.0);
        assert_eq!(island.hover_pad.target(), 0.0);
        assert!((island.hover_pad.value(11.6) - width_before_editor).abs() < 1e-9);
        island.set_editor_chip_hover(-1.0, -1.0, 11.7);
        assert_eq!(island.editor_panel.target(), 1.0);
        island.hovered = false;
        island.sync(11.8);
        assert_eq!(island.editor_panel.target(), 0.0);
    }

    #[test]
    fn level_changes_slide_out_at_the_side_and_leave_the_island_alone() {
        let mut island = Island::new(Config::default());
        for i in 0..60 {
            let time = 10.0 + i as f64 * 0.05;
            island.live_activity(activity::Event::Slider {
                kind: if i % 2 == 0 { activity::SliderKind::Volume } else { activity::SliderKind::Brightness },
                value: i as f64 / 60.0,
                muted: false,
            }, time);
            island.advance(time);
            assert!(island.activity.is_none(), "no pill in the island");
            assert_eq!(island.pill.target(), 0.0);
            assert_eq!(island.deadline(time), Some(time + crate::side_hud::HOLD), "each change extends the stay");
        }
        assert!(island.is_animating(13.0));
        island.advance(14.5);
        assert!(island.deadline(14.5).is_none());
        assert!(!island.is_animating(17.0));
    }

    #[test]
    fn fullscreen_events_rise_without_restoring_the_compact_island() {
        let mut island = Island::new(Config::default());
        island.set_fullscreen(true, 10.0);
        assert!(island.is_hidden());
        assert_eq!(island.hide.target(), 1.0);

        island.notify(crate::notify::fabricate("System", "Message", "Body"), 11.0);
        assert!(island.notice.is_some());
        assert!(!island.is_hidden());
        assert_eq!(island.hide.target(), 0.0);
        island.advance(11.0 + NOTICE_HOLD + 0.01);
        assert!(island.notice.is_none());
        assert!(island.is_hidden());
        assert_eq!(island.hide.target(), 1.0);

        island.live_activity(activity::Event::timer_done(), 20.0);
        assert!(island.activity.is_some());
        assert_eq!(island.hide.target(), 0.0);
        island.advance(20.0 + activity::Event::timer_done().hold() + 0.01);
        assert!(island.activity.is_none());
        assert_eq!(island.hide.target(), 1.0);

        assert!(island.toggle_launcher(30.0));
        assert_eq!(island.hide.target(), 0.0);
        island.close_launcher(31.0);
        assert!(island.is_hidden());
        assert_eq!(island.hide.target(), 1.0);
    }

    #[test]
    fn now_playing_toast_remains_fully_visible_over_fullscreen() {
        let mut island = Island::new(Config::default());
        let track = crate::dbus::NowPlaying {
            bus_name: "test".into(), app: "Player".into(), playing: true,
            artist: "Artist".into(), title: "Video".into(), art_url: String::new(),
            track_id: "video".into(), length: 60.0,
        };
        island.set_fullscreen(true, 10.0);
        island.set_player(Some(&track), 10.1);
        assert!(island.toast.is_some());
        assert!(!island.is_hidden());
        assert_eq!(island.hide.target(), 0.0);
        assert_eq!(island.hide.state(10.1).x, 0.0);
        island.advance(10.1 + TOAST_HOLD + 0.01);
        assert!(island.is_hidden());
        assert_eq!(island.hide.target(), 1.0);

        let mut entering_fullscreen = Island::new(Config::default());
        entering_fullscreen.set_player(Some(&track), 20.0);
        entering_fullscreen.set_fullscreen(true, 20.1);
        assert!(entering_fullscreen.toast.is_some());
        assert_eq!(entering_fullscreen.hide.target(), 0.0);

        let mut manually_hidden = Island::new(Config::default());
        manually_hidden.hide(30.0);
        manually_hidden.set_player(Some(&track), 30.1);
        assert!(manually_hidden.toast.is_none());
    }

    #[test]
    fn manual_show_temporarily_overrides_fullscreen_hide() {
        let mut island = Island::new(Config::default());
        island.set_fullscreen(true, 10.0);
        island.toggle_visibility(11.0);
        assert!(!island.is_hidden());
        assert_eq!(island.hide.target(), 0.0);
        island.toggle_visibility(12.0);
        assert!(island.is_hidden());
        island.show(13.0);
        assert!(!island.is_hidden());
        island.set_fullscreen(false, 14.0);
        assert!(!island.hidden.fullscreen_override);
        island.set_fullscreen(true, 15.0);
        assert!(island.is_hidden());
    }

    #[test]
    fn timer_completion_waits_for_touch_id_and_invalid_durations_are_rejected() {
        let mut island = Island::new(Config::default());
        for duration in ["inf", "NaN", "-1", "1e100"] {
            assert!(island.timer_command(duration, 10.0).is_err());
        }
        island.timer_command("1s", 10.0).unwrap();
        island.auth_started(10.1);
        island.advance(11.1);
        assert!(island.activity.is_none());
        assert_eq!(island.activity_queued, Some(activity::Event::timer_done()));
    }

    #[test]
    fn timer_menu_can_start_pause_and_cancel_with_the_pointer() {
        let mut island = Island::new(Config::default());
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 980, 240).unwrap();
        let cr = Context::new(&surface).unwrap();
        let renderer = Renderer::new(1.0);
        island.timer_command("menu", 10.0).unwrap();
        for i in 0..60 { island.draw(&cr, &renderer, 490.0, 10.0 + i as f64 / 60.0); }
        let rect = island.timer_menu_rect.unwrap();
        let (x, y) = island.timer_page.centre(timer_page::Part::Start);
        assert!(island.click_timer_menu(rect.x + x, rect.y + y, 11.0));
        assert_eq!(island.timer_command("status", 11.0).unwrap(), "900 running");
        // the page stays, now showing the running timer
        assert!(matches!(island.timer_session(11.0), timer_page::Session::Active { .. }));
        assert!(island.close_activity_panel(11.1));
        for i in 0..60 { island.draw(&cr, &renderer, 490.0, 11.1 + i as f64 / 60.0); }
        let rect = island.timer_rect.unwrap();
        assert!(island.click_timer_at(rect.x + 5.0, rect.y + 16.0, 12.5));
        assert!(!island.timer.as_ref().unwrap().running);
        island.timer_command("menu", 13.0).unwrap();
        for i in 0..60 { island.draw(&cr, &renderer, 490.0, 13.0 + i as f64 / 60.0); }
        let rect = island.timer_menu_rect.unwrap();
        let (x, y) = island.timer_page.centre(timer_page::Part::Cancel);
        island.click_timer_menu(rect.x + x, rect.y + y, 14.0);
        assert!(island.timer.is_none());
    }

    #[test]
    fn polished_activities_render_at_fractional_scale() {
        let scale = 1.5;
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 780, 1170).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(scale, scale);
        cr.set_source_rgb(0.12, 0.13, 0.15);
        cr.paint().unwrap();
        let renderer = Renderer::new(scale);
        let cases = [
            (Some(activity::Event::Slider { kind: activity::SliderKind::Volume, value: 0.65, muted: false }), None, 0.0),
            (Some(activity::Event::Slider { kind: activity::SliderKind::Brightness, value: 0.8, muted: false }), None, 110.0),
            (Some(activity::Event::Slider { kind: activity::SliderKind::Volume, value: 0.4, muted: true }), None, 220.0),
            (None, Some("menu"), 330.0),
            (None, None, 550.0),
            (None, Some("25m"), 620.0),
            (None, Some("pause"), 690.0),
        ];
        for (event, command, y) in cases {
            let mut island = Island::new(Config::default());
            island.clock = "Thu Sep 17  12:30 AM".into();
            island.sync(10.0);
            if let Some(event) = event { island.live_activity(event, 10.0); }
            if let Some(command) = command {
                if command == "pause" { island.timer_command("25m", 10.0).unwrap(); }
                island.timer_command(command, 10.0).unwrap();
            }
            // Resolve geometry on a scratch surface, then paint a settled
            // frame onto the contact sheet with the real notch renderer.
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 780, 360).unwrap();
            let ctx = Context::new(&scratch).unwrap();
            ctx.scale(scale, scale);
            for i in 0..60 { island.draw(&ctx, &renderer, 260.0, 10.0 + i as f64 / 60.0); }
            cr.save().unwrap();
            cr.translate(0.0, y);
            island.draw(&cr, &renderer, 260.0, 11.0);
            cr.restore().unwrap();
        }
        if std::env::var_os("ISLAND_ACTIVITY_PREVIEW").is_some() {
            let mut file = std::fs::File::create("target/activity-polish.png").unwrap();
            surface.write_to_png(&mut file).unwrap();
        }
    }

    #[test]
    fn countdown_pauses_and_resumes_without_losing_time() {
        let mut island = Island::new(Config::default());
        assert_eq!(island.timer_command("10s", 100.0).unwrap(), "ok");
        assert_eq!(island.timer_command("status", 104.0).unwrap(), "6 running");
        island.timer_command("pause", 104.0).unwrap();
        assert_eq!(island.timer_command("status", 109.0).unwrap(), "6 paused");
        island.timer_command("resume", 109.0).unwrap();
        assert_eq!(island.timer_command("status", 111.0).unwrap(), "4 running");
        island.timer_command("cancel", 111.0).unwrap();
        assert_eq!(island.timer_command("status", 111.0).unwrap(), "stopped");
    }

    #[test]
    fn escape_closes_timer_menu_without_stopping_countdown() {
        let mut island = Island::new(Config::default());
        island.timer_command("10m", 100.0).unwrap();
        island.live_activity(activity::Event::TimerMenu, 100.0);
        assert!(island.keyboard_panel_open());
        assert!(island.close_activity_panel(101.0));
        assert!(!island.keyboard_panel_open());
        assert_eq!(island.timer_command("status", 101.0).unwrap(), "599 running");
    }

    /// Draw the island for `frames` frames from `from` (rects and the page
    /// are laid out by drawing).
    #[test]
    fn the_closed_island_gives_against_a_tight_rubber_band_and_jumps_back() {
        let mut island = Island::new(Config::default());
        settle(&mut island, 10.0, 30);
        let line = island.line_rect.expect("the closed line");
        let (x, y) = (line.x + line.w / 2.0, line.y + line.h / 2.0);
        // a press that does not move is a click
        assert!(island.tug_press(x, y, 11.0));
        assert!(!island.tug_to(x + 3.0, y + 2.0, 11.05));
        assert!(!island.tug_release(11.1));
        // a long pull gives only so much, the further the less
        assert!(island.tug_press(x, y, 12.0));
        assert!(island.tug_to(x, y + 60.0, 12.05));
        let near = island.tug_now(12.05).1;
        assert!(island.tug_to(x, y + 600.0, 12.1));
        let far = island.tug_now(12.1).1;
        assert!(near > 10.0 && far > near && far < TUG_DOWN, "{near} {far}");
        assert!(island.tug_to(x - 600.0, y - 600.0, 12.15));
        let (side, up) = island.tug_now(12.15);
        assert!(side < -15.0 && side > -TUG_SIDE && up < 0.0 && up > -TUG_UP, "{side} {up}");
        // let go, it overshoots a little and comes to rest where it was
        assert!(island.tug_to(x, y + 600.0, 12.2));
        assert!(island.tug_release(12.2));
        assert!(island.is_animating(12.25));
        let low = (0..120).map(|i| island.tug_now(12.2 + i as f64 / 120.0).1).fold(f64::MAX, f64::min);
        assert!(low < -1.0 && low > -0.4 * TUG_DOWN, "a jelly overshoot: {low}");
        assert!(island.tug_now(15.0).1.abs() < 0.05 && !island.is_animating(15.0));
        // only the plain line: an open page is not a handle
        island.open_quick_menu(16.0);
        settle(&mut island, 16.0, 40);
        assert!(!island.tug_press(x, y, 17.0));
    }

    #[test]
    fn a_tugged_island_declares_no_opaque_box() {
        let mut island = Island::new(Config::default());
        settle(&mut island, 10.0, 30);
        let line = island.line_rect.expect("the closed line");
        let (x, y) = (line.x + line.w / 2.0, line.y + line.h / 2.0);
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 300).unwrap();
        let cr = Context::new(&surface).unwrap();
        assert!(island.draw(&cr, &renderer, 490.0, 11.0).opaque.iter().any(Option::is_some));
        island.tug_press(x, y, 11.0);
        island.tug_to(x + 200.0, y + 100.0, 11.1);
        assert!(island.draw(&cr, &renderer, 490.0, 11.1).opaque.iter().all(Option::is_none));
        island.tug_release(11.1);
        let rest = island.draw(&cr, &renderer, 490.0, 14.0);
        assert!(rest.opaque.iter().any(Option::is_some), "back at rest it is opaque again");
    }

    #[test]
    fn rubber_inverse_undoes_the_band() {
        for d in [-300.0, -40.0, 5.0, 80.0] {
            assert!((rubber_inverse(rubber(d, TUG_DOWN), TUG_DOWN) - d).abs() < 1e-6);
        }
    }

    fn settle(island: &mut Island, from: Time, frames: usize) {
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 960).unwrap();
        let cr = Context::new(&surface).unwrap();
        for n in 0..frames {
            let now = from + n as f64 / 60.0;
            island.advance(now);
            island.draw(&cr, &renderer, 490.0, now);
        }
    }

    #[test]
    fn the_timer_page_starts_from_its_ruler_and_ticks_as_the_ruler_turns() {
        let mut island = Island::new(Config::default());
        island.timer_command("menu", 10.0).unwrap();
        settle(&mut island, 10.0, 60);
        assert!(island.timer_page_open());
        let r = island.timer_menu_rect.expect("the page");
        // two fingers turn the ruler a few minutes; each change ticks once
        let (px, py) = (r.x + 120.0, r.y + 100.0);
        assert!(island.scroll_timer(px, py, 3.0 * timer_page::SPACING, false, 11.0));
        assert_eq!(island.take_sounds(), vec![crate::sound::Sound::Tick]);
        assert!(island.timer_key("Return", 11.1));
        assert_eq!(island.timer_command("status", 11.1).unwrap(), "1080 running");
        // the page now offers Cancel and Pause; Return pauses
        assert!(matches!(island.timer_session(11.2), timer_page::Session::Active { running: true, .. }));
        island.timer_key("Return", 11.2);
        assert!(!island.timer.as_ref().unwrap().running);
        island.config.timer_sound = false;
        island.timer_key("Escape", 11.3);
        island.timer_command("cancel", 11.3).unwrap();
        island.scroll_timer(px, py, 40.0, true, 11.4);
        assert!(island.take_sounds().is_empty(), "sounds can be switched off");
    }

    #[test]
    fn a_pomodoro_phase_ends_with_the_alarm_and_waits_for_the_next() {
        let mut island = Island::new(Config::default());
        island.timer_page.pomodoro = timer_page::Pomodoro { focus: 1, short: 1, long: 2, interval: 2, sessions: 2 };
        island.timer_command("pomodoro", 0.0).unwrap();
        island.advance(61.0);
        assert!(island.timer.is_none());
        assert_eq!(island.take_sounds(), vec![crate::sound::Sound::Alarm]);
        assert!(matches!(&island.activity.as_ref().unwrap().event,
            activity::Event::TimerDone { title, .. } if title == "Фокус окончен"));
        assert!(matches!(island.timer_session(61.0), timer_page::Session::Next { next: timer_page::Phase::ShortBreak, .. }));
        island.timer_command("next", 62.0).unwrap();
        assert!(island.timer_command("status", 62.0).unwrap().ends_with("pomodoro перерыв 1/2"));
        island.advance(123.0);
        island.timer_command("next", 124.0).unwrap();
        island.advance(185.0);
        // the last focus ends the cycle
        assert_eq!(island.timer_session(185.0), timer_page::Session::Finished { total: 2 });
        // the stopwatch counts up and never runs out
        island.timer_command("stopwatch", 200.0).unwrap();
        island.advance(500.0);
        assert_eq!(island.timer_command("status", 500.0).unwrap(), "300 running stopwatch");
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;
    fn editor(server: &str, focused: bool) -> Editor {
        Editor { server: server.into(), focused, file: "main.c".into(), mode: "c".into(), ..Editor::default() }
    }
    fn command() -> Option<nvim::Cmdline> {
        Some(nvim::Cmdline { prefix: ":".into(), text: "write".into(), pos: 5 })
    }
    #[test]
    fn save_flash_follows_only_the_visible_editor_and_current_file() {
        let mut island = Island::new(Config::default());
        island.set_editor(editor("one", true), 10.0);
        island.editor_saved("two", 10.1);
        assert_eq!(island.nvim_saved, None);
        island.editor_saved("one", 10.2);
        assert_eq!(island.nvim_saved, Some(10.2));
        island.set_editor(Editor { file: "other.c".into(), ..editor("one", true) }, 10.3);
        assert_eq!(island.nvim_saved, None);
        island.set_editor(editor("one", false), 10.4);
        island.editor_saved("one", 10.5);
        assert_eq!(island.nvim_saved, None);
    }
    #[test]
    fn command_is_scoped_to_focused_instance_and_expires() {
        let mut island = Island::new(Config::default());
        island.set_editor(editor("one", true), 1.0);
        assert!(!island.set_cmdline("two", command(), 1.1));
        assert!(island.set_cmdline("one", command(), 1.1));
        assert!(island.want_editor_panel(1.2));
        island.set_editor(editor("two", false), 1.2);
        assert_eq!(island.nvim.as_ref().unwrap().server, "one");
        island.set_editor(editor("one", true), 1.3);
        assert!(island.nvim.as_ref().unwrap().cmdline.is_some());
        island.advance(2.0);
        assert!(island.nvim.as_ref().unwrap().cmdline.is_none());
    }
    #[test]
    fn completed_command_folds_without_revealing_buffer_panel() {
        let mut island = Island::new(Config::default());
        island.set_editor(editor("one", true), 1.0);
        island.set_cmdline("one", command(), 1.1);
        assert_eq!(island.editor_height(), 84.0);
        let open = 1.1 + PLAYER_DUR;
        assert_eq!(island.editor_panel.state(open).x, 1.0);
        island.set_cmdline("one", None, open);
        assert_eq!(island.editor_panel.target(), 0.0);
        assert_eq!(island.editor_height(), 84.0);
        assert!(island.editor_command.is_some());
        let mut previous = 84.0;
        for step in 0..=20 {
            let now = open + PLAYER_DUR * step as f64 / 20.0;
            let height = BOX_H + (island.editor_height() - BOX_H) * island.editor_panel.state(now).x;
            assert!(height <= previous + 1e-8);
            previous = height;
        }
        assert!((previous - BOX_H).abs() < 1e-8);
        let finished = open + PLAYER_DUR;
        island.notify(crate::notify::fabricate("Neovim", ":echo", "done"), finished);
        island.dismiss_notice(finished + 1.0);
        assert!(!island.want_editor_panel(finished + 1.0));
        assert_eq!(island.editor_panel.target(), 0.0);
        island.hovered = true;
        island.sync(finished + 1.1);
        assert_eq!(island.editor_panel.target(), 0.0);
        island.editor_chip_hovered = true;
        island.sync(finished + 1.2);
        assert_eq!(island.editor_panel.target(), 1.0);
        assert_eq!(island.editor_height(), editor::PANEL_H);
        assert!(island.editor_command.is_none());
    }
    #[test]
    fn results_require_a_visible_focused_editor() {
        let mut island = Island::new(Config::default());
        island.set_editor(editor("one", true), 1.0);
        assert!(island.accepts_result("one", 1.1));
        assert!(!island.accepts_result("two", 1.1));
        island.hidden.user = true;
        assert!(!island.accepts_result("one", 1.1));
        island.hidden.user = false;
        island.auth_started(1.2);
        assert!(!island.accepts_result("one", 1.3));
    }
    #[test]
    fn hidden_island_and_authentication_return_command_to_terminal() {
        let mut island = Island::new(Config::default());
        island.set_editor(editor("one", true), 1.0);
        island.hidden.user = true;
        assert!(!island.set_cmdline("one", command(), 1.1));
        island.hidden.user = false;
        island.auth_started(1.2);
        assert!(!island.set_cmdline("one", command(), 1.3));
    }

    #[test]
    fn compact_items_parse_and_describe() {
        assert_eq!(Compact::parse("all"), Ok(Compact::ALL));
        assert_eq!(Compact::parse("none"), Ok(Compact::NONE));
        let some = Compact::parse("music, timer mic").unwrap();
        assert!(some.music && some.timer && some.mic && !some.clock && !some.dnd);
        assert_eq!(some.describe(), "timer,music,mic");
        assert!(Compact::parse("clock weather").is_err());
        let mut config = Config::default();
        config.set("compact", "timer,music").unwrap();
        assert!(!config.compact.clock);
        assert!(config.describe().contains("compact=timer,music"));
    }

    #[test]
    fn without_the_clock_the_island_rests_as_a_short_capsule_and_right_click_opens_the_month() {
        let mut config = Config::default();
        config.compact.clock = false;
        let mut island = Island::new(config);
        island.advance(1.0);
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 960).unwrap();
        let cr = Context::new(&surface).unwrap();
        island.draw(&cr, &renderer, 490.0, 1.0);
        let line = island.line_rect.expect("the closed line");
        assert!((line.w - (REST_CONTENT_W + 2.0 * PAD)).abs() < 1.0, "resting width {}", line.w);
        assert!(island.clock_rect.is_none());
        assert!(island.clock_at(line.x + line.w / 2.0, line.y + 10.0));
    }

    #[test]
    fn without_the_clock_activities_part_into_wings_around_the_songs_title() {
        let mut config = Config::default();
        config.compact.clock = false;
        let mut island = Island::new(config);
        island.song = Some(Song { now: super::tests::gallery_song(true), position: 0.0, position_at: 1.0 });
        island.reveal.set_at(1.0, 1.0);
        island.timer_command("15m", 1.0).unwrap();
        island.sync(1.0);
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 960).unwrap();
        let cr = Context::new(&surface).unwrap();
        for n in 0..90 {
            let now = 1.0 + n as f64 / 60.0;
            island.advance(now);
            island.draw(&cr, &renderer, 490.0, now);
        }
        let (cover, mark) = (island.cover_rect.expect("the cover"), island.timer_mark_rect.expect("the ring"));
        let (eq, reading) = (island.eq_rect.expect("the bars"), island.timer_rect.expect("the reading"));
        assert!(cover.x + cover.w < 490.0 && mark.x + mark.w < 490.0, "cover and ring on the left");
        assert!(eq.x > 490.0 && reading.x > 490.0, "bars and reading on the right");
        let line = island.line_rect.unwrap();
        assert!((line.x + line.w / 2.0 - 490.0).abs() < 1.0, "the middle stays centred");
        // the cover is the song's, the ring the timer's, the middle the island's
        let at = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (x, y) = at(cover);
        assert!(island.music_area_at(x, y));
        assert!(!island.clock_at(x, y));
        let (x, y) = at(mark);
        assert!(island.click_timer_at(x, y, 3.0));
        assert!(!island.timer.as_ref().unwrap().running, "the ring pauses the timer");
        assert!(island.clock_at(490.0, line.y + 10.0));
    }

    #[test]
    fn the_sections_gallery_opens_from_its_floating_button_and_lists_destinations() {
        let mut island = Island::new(Config::default());
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 960).unwrap();
        let cr = Context::new(&surface).unwrap();
        let settle = |island: &mut Island, from: Time| {
            for n in 0..90 {
                let now = from + n as f64 / 60.0;
                island.advance(now);
                island.draw(&cr, &renderer, 490.0, now);
            }
        };
        assert!(island.open_quick_menu(1.0));
        settle(&mut island, 1.0);
        island.press_float(floating::Button::Sections, 3.0);
        settle(&mut island, 3.0);
        assert!(island.sections_open());
        assert!(island.float_selected(floating::Button::Sections));
        let rect = island.quick_rect.expect("the gallery");
        let (x, y) = quick::section_centre(quick::Section::Calendar);
        assert_eq!(island.quick_action_at(rect.x + x, rect.y + y), Some(quick::Action::Section(quick::Section::Calendar)));
        assert_eq!(island.quick_keys().len(), quick::SECTIONS.len());
        assert!((rect.h - quick::height(quick::View::Sections, &island.controls)).abs() < 1.0);
    }

    #[test]
    fn floating_buttons_arrive_beside_an_open_page_and_lead_to_other_pages() {
        let mut island = Island::new(Config::default());
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1470, 960).unwrap();
        let cr = Context::new(&surface).unwrap();
        let settle = |island: &mut Island, from: Time| {
            for n in 0..90 {
                let now = from + n as f64 / 60.0;
                island.advance(now);
                island.draw(&cr, &renderer, 490.0, now);
            }
        };
        settle(&mut island, 1.0);
        assert!(island.float_body.is_none(), "no buttons beside the compact island");
        assert!(island.open_quick_menu(3.0));
        settle(&mut island, 3.0);
        let body = island.float_body.expect("the buttons arrived");
        let at = |button: floating::Button| {
            (0..400).flat_map(|y| (0..980).map(move |x| (x as f64, y as f64)))
                .find(|(x, y)| floating::hit(body, *x, *y) == Some(button)).unwrap()
        };
        let (x, y) = at(floating::Button::Calendar);
        assert_eq!(island.float_at(x, y), Some(floating::Button::Calendar));
        island.press_float(floating::Button::Calendar, 5.0);
        assert!(island.calendar_open() && !island.quick_menu_open());
        assert!(island.float_selected(floating::Button::Calendar));
        settle(&mut island, 5.0);
        island.press_float(floating::Button::Notifications, 7.0);
        assert!(island.center.open && !island.calendar_open());
        settle(&mut island, 7.0);
        island.press_float(floating::Button::Sound, 9.0);
        assert!(island.quick_menu_open() && island.float_selected(floating::Button::Sound) && !island.center.open);
        settle(&mut island, 9.0);
        island.close_quick_menu(11.0);
        settle(&mut island, 11.0);
        assert!(island.float_body.is_none() && island.floats.value(12.5) < 0.01, "they fall back in");
    }
}


#[cfg(test)]
mod tug_membrane_look {
    use super::*;
    #[test]
    #[ignore = "writes target/gallery/tug-membrane.png"]
    fn tug_membrane() {
        let renderer = Renderer::new(3.0);
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, 1200, 300).unwrap();
        let cr = Context::new(&surface).unwrap();
        let mut island = tests::gallery_island_pub();
        for (i, (dx, dy)) in [(0.0, 200.0), (160.0, 120.0)].into_iter().enumerate() {
            let mut t = 10.0;
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 1200, 300).unwrap();
            let sc = Context::new(&scratch).unwrap();
            sc.scale(1.5, 1.5);
            for _ in 0..40 { island.advance(t); island.draw(&sc, &renderer, 200.0, t); t += 1.0 / 60.0; }
            let line = island.line_rect.unwrap();
            let (x, y) = (line.x + line.w / 2.0, line.y + line.h / 2.0);
            island.tug_press(x, y, t);
            island.tug_to(x + dx, y + dy, t + 0.1);
            cr.save().unwrap();
            cr.translate(i as f64 * 600.0, 0.0);
            cr.rectangle(0.0, 0.0, 600.0, 300.0);
            cr.clip();
            cr.scale(1.5, 1.5);
            cr.set_source_rgb(0.15, 0.32, 0.38);
            cr.paint().unwrap();
            island.draw(&cr, &renderer, 200.0, t + 0.1);
            island.draw_frame_marks(&cr, &renderer, 400.0, 200.0, t + 0.1);
            cr.restore().unwrap();
            island.tug_release(t + 0.1);
            for _ in 0..120 { t += 1.0 / 60.0; island.advance(t); }
        }
        drop(cr);
        surface.write_to_png(&mut std::fs::File::create("target/gallery/tug-membrane.png").unwrap()).unwrap();
    }
}

#[cfg(test)]
mod bar_look {
    use super::*;
    #[test]
    #[ignore = "writes target/gallery/bar.png"]
    fn bar_look() {
        let renderer = Renderer::new(1.5);
        let (w, h) = (1504.0, 380.0);
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, (w * 1.5) as i32, (h * 1.5 * 2.0) as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        for (row, drawer) in [false, true].into_iter().enumerate() {
            let mut island = tests::gallery_island_pub();
            island.config.bar = true;
            island.bar.workspaces = ["1", "2", "3", "10"].iter().map(|n| crate::bar::Workspace {
                name: n.to_string(), focused: *n == "2", urgent: *n == "10", output: "eDP-1".into(),
            }).collect();
            island.controls.volume = Some(0.5);
            island.controls.layout = Some("English (US)".into());
            island.bar.battery = Some(crate::bar::Battery { percent: 84, charging: false, remaining: None });
            island.bar.details = Some(crate::bar::Details { disk_free: "118G".into(), cpu: 7, memory_gb: 3.4 });
            island.bar.set_drawer(drawer);
            island.bar.tray = vec![crate::bar::TrayIcon { id: "vpn".into(), icon: None, has_menu: true, item_is_menu: false, title: "VPN".into() }];
            if drawer {
                use crate::tray::MenuEntry;
                let e = |id, label: &str, enabled| MenuEntry { id, label: label.into(), enabled, ..MenuEntry::default() };
                let sep = MenuEntry { separator: true, ..MenuEntry::default() };
                let mut checked = e(8, "Start at login", true);
                checked.toggle = Some(true);
                let mut more = e(9, "Servers", true);
                more.children = vec![e(10, "Auto", true)];
                island.bar.open_menu("vpn".into(), vec![e(7, "Show AmneziaVPN", true), sep.clone(), e(5, "Connect", false),
                    e(4, "Disconnect", true), checked, more, sep, e(2, "Visit Website", true), e(1, "Quit AmneziaVPN", true)], 10.0);
            }
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 2256, 300).unwrap();
            let sc = Context::new(&scratch).unwrap();
            sc.scale(1.5, 1.5);
            let mut t = 10.0;
            for _ in 0..90 { island.advance(t); island.draw(&sc, &renderer, w / 2.0, t); island.draw_bar(&sc, &renderer, w, t); t += 1.0 / 60.0; }
            cr.save().unwrap();
            cr.translate(0.0, row as f64 * h * 1.5);
            cr.rectangle(0.0, 0.0, w * 1.5, h * 1.5);
            cr.clip();
            cr.scale(1.5, 1.5);
            let g = cairo::LinearGradient::new(0.0, 0.0, w, h);
            g.add_color_stop_rgb(0.0, 0.10, 0.25, 0.32);
            g.add_color_stop_rgb(1.0, 0.18, 0.35, 0.40);
            cr.set_source(&g).unwrap();
            cr.paint().unwrap();
            island.draw(&cr, &renderer, w / 2.0, t);
            island.draw_bar(&cr, &renderer, w, t);
            crate::bezel::paint(&cr, w, 1002.0, 16.0);
            cr.restore().unwrap();
        }
        drop(cr);
        surface.write_to_png(&mut std::fs::File::create("target/gallery/bar.png").unwrap()).unwrap();
    }
}

#[cfg(test)]
mod light_round_drops {
    use super::*;
    #[test]
    #[ignore = "writes target/gallery/light-drops.png"]
    fn light_round_drops() {
        let renderer = Renderer::new(1.5);
        let (w, h) = (1504.0, 1002.0);
        let rows = [10.35, 10.55, 10.8];
        let (fh, scale) = (110.0, 0.8);
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, (w * scale) as i32, ((fh * scale + 4.0) * rows.len() as f64) as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        for (i, t) in rows.into_iter().enumerate() {
            let mut island = tests::gallery_island_pub();
            island.config.bar = true;
            island.bar.workspaces = ["1", "2", "3"].iter().map(|n| crate::bar::Workspace { name: n.to_string(), focused: *n == "1", urgent: false, output: "eDP-1".into() }).collect();
            island.controls.volume = Some(0.3);
            island.controls.layout = Some("English".into());
            island.bar.battery = Some(crate::bar::Battery { percent: 50, charging: false, remaining: None });
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 2256, 300).unwrap();
            let sc = Context::new(&scratch).unwrap();
            sc.scale(1.5, 1.5);
            let mut now = 9.0;
            for _ in 0..60 { island.advance(now); island.draw_edges(&sc, w, h, w / 2.0, now); island.draw(&sc, &renderer, w / 2.0, now); island.draw_bar(&sc, &renderer, w, now); now += 1.0 / 60.0; }
            island.edge.pulse(crate::edge::Wave::Top, (0.165, 0.67, 0.93), 10.0);
            cr.save().unwrap();
            cr.translate(0.0, i as f64 * (fh * scale + 4.0));
            cr.rectangle(0.0, 0.0, w * scale, fh * scale);
            cr.clip();
            cr.scale(scale, scale);
            cr.set_source_rgb(0.10, 0.17, 0.22);
            cr.paint().unwrap();
            island.draw_edges(&cr, w, h, w / 2.0, t);
            island.draw(&cr, &renderer, w / 2.0, t);
            island.draw_bar(&cr, &renderer, w, t);
            crate::bezel::paint(&cr, w, h, 16.0);
            cr.restore().unwrap();
        }
        drop(cr);
        surface.write_to_png(&mut std::fs::File::create("target/gallery/light-drops.png").unwrap()).unwrap();
    }
}

#[cfg(test)]
mod ripple_look {
    use super::*;
    #[test]
    #[ignore = "writes target/gallery/ripple-drops.png"]
    fn ripple_drops() {
        let renderer = Renderer::new(1.5);
        let (w, h) = (1504.0, 1002.0);
        // out along the edges, then into the screen's sides (~0.58 s)
        let rows = [0.12, 0.3, 0.52, 0.58, 0.62, 0.7, 0.9];
        let (fh, scale) = (70.0, 0.8);
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, (w * scale) as i32, ((fh * scale + 4.0) * rows.len() as f64) as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        for (i, t) in rows.into_iter().enumerate() {
            let mut island = tests::gallery_island_pub();
            island.config.bar = true;
            island.bar.workspaces = ["1", "2", "3"].iter().map(|n| crate::bar::Workspace { name: n.to_string(), focused: *n == "1", urgent: false, output: "eDP-1".into() }).collect();
            island.controls.volume = Some(0.3);
            island.controls.layout = Some("English".into());
            island.bar.battery = Some(crate::bar::Battery { percent: 50, charging: false, remaining: None });
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 2256, 300).unwrap();
            let sc = Context::new(&scratch).unwrap();
            sc.scale(1.5, 1.5);
            let mut now = 9.0;
            for _ in 0..60 { island.advance(now); island.draw(&sc, &renderer, w / 2.0, now); island.draw_bar(&sc, &renderer, w, now); now += 1.0 / 60.0; }
            island.membrane.ripple(w / 2.0, 1.0, 10.0);
            cr.save().unwrap();
            cr.translate(0.0, i as f64 * (fh * scale + 4.0));
            cr.rectangle(0.0, 0.0, w * scale, fh * scale);
            cr.clip();
            cr.scale(scale, scale);
            cr.set_source_rgb(0.15, 0.32, 0.38);
            cr.paint().unwrap();
            island.draw(&cr, &renderer, w / 2.0, 10.0 + t);
            island.draw_bar(&cr, &renderer, w, 10.0 + t);
            island.draw_frame_marks(&cr, &renderer, w, h, 10.0 + t);
            cr.restore().unwrap();
        }
        drop(cr);
        surface.write_to_png(&mut std::fs::File::create("target/gallery/ripple-drops.png").unwrap()).unwrap();
        // the right side close up, as the crest hits it
        let times = [0.3, 0.36, 0.42, 0.5, 0.55, 0.58, 0.61, 0.65, 0.72, 0.85];
        let (cw, ch, zoom) = (150.0, 70.0, 3.0);
        let close = cairo::ImageSurface::create(cairo::Format::Rgb24, (cw * zoom * times.len() as f64) as i32, (2.0 * ch * zoom + 3.0) as i32).unwrap();
        let cr = Context::new(&close).unwrap();
        for (i, t) in times.into_iter().enumerate() {
            let mut island = tests::gallery_island_pub();
            island.config.bar = true;
            island.controls.volume = Some(0.3);
            island.bar.battery = Some(crate::bar::Battery { percent: 50, charging: false, remaining: None });
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 2256, 300).unwrap();
            let sc = Context::new(&scratch).unwrap();
            let mut now = 9.0;
            for _ in 0..60 { island.advance(now); island.draw(&sc, &renderer, w / 2.0, now); island.draw_bar(&sc, &renderer, w, now); now += 1.0 / 60.0; }
            island.membrane.ripple(w / 2.0, 1.0, 10.0);
            // the top row: the right side; the bottom: the drop's inner turn
            let turn = island.bar.drop_sizes()[1].map_or(w, |d| w - d.w);
            for (row, x) in [w - cw, turn - cw / 2.0].into_iter().enumerate() {
                cr.save().unwrap();
                cr.translate(i as f64 * cw * zoom, row as f64 * (ch * zoom + 3.0));
                cr.rectangle(0.0, 0.0, cw * zoom - 3.0, ch * zoom);
                cr.clip();
                cr.scale(zoom, zoom);
                cr.translate(-x, 0.0);
                cr.set_source_rgb(0.15, 0.32, 0.38);
                cr.paint().unwrap();
                island.draw_bar(&cr, &renderer, w, 10.0 + t);
                island.draw_frame_marks(&cr, &renderer, w, h, 10.0 + t);
                cr.restore().unwrap();
            }
        }
        drop(cr);
        close.write_to_png(&mut std::fs::File::create("target/gallery/ripple-wall.png").unwrap()).unwrap();
    }
}
