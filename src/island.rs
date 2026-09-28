//! The island itself: its state, the animated quantities derived from it and
//! the drawing. Everything here is in logical pixels; `Renderer` maps them to
//! the output's pixels through the cairo scale.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;

use cairo::Context;

use crate::anim::{Quintic, Spring, Time, inv_min_jerk};
use crate::activity;
use crate::icons;
use crate::art::Art;
use crate::editor;
use crate::launcher::{self, Launcher};
use crate::notice_center;
use crate::nvim::{self, Editor};
use crate::player;
use crate::quick;
use crate::touchid;

/// Height of the black box, logical px.
pub const BOX_H: f64 = 32.0;
/// Bottom corner radius at rest / when fully retracted into the edge.
const RADIUS: f64 = 14.0;
const RADIUS_HIDDEN: f64 = 22.0;
/// The concave fillets where the box meets the top edge of the screen.
pub const FILLET: f64 = 14.0;
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
const PILL_DUR: f64 = 0.35;
/// progress at which the line has dissolved and the pill content starts
const PILL_SWAP: f64 = 0.37;
const PILL_CLIMB: f64 = 30.0;
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
/// the roll when one notification replaces another
const ROLL_DUR: f64 = 0.44;
const PILL_W_DUR: f64 = 0.35;
const PILL_H_DUR: f64 = 0.35;
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
const BARS_W: f64 = 26.0;
const BARS_H: f64 = 14.0;
const DODGE_DUR: f64 = 0.25;
/// Hover: a spring with a little overshoot, like the CSS
/// cubic-bezier(0.2, 0.9, 0.3, 1.25) it replaces (peak ~250 ms in).
const HOVER_RESPONSE: f64 = 0.4;
const HOVER_DAMPING: f64 = 0.6;

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
}

impl Default for Config {
    fn default() -> Self {
        Config { smoothing: 0.6, liquid: 1.0, hover: true }
    }
}

impl Config {
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let v: f64 = match value {
            "on" | "true" => 1.0,
            "off" | "false" => 0.0,
            v => v.parse().map_err(|_| format!("bad value {value:?}"))?,
        };
        match key {
            "smoothing" => self.smoothing = v.clamp(0.0, 1.0),
            "liquid" => self.liquid = v.clamp(0.0, 4.0),
            "hover" => self.hover = v != 0.0,
            _ => return Err(format!("unknown setting {key:?}")),
        }
        Ok(())
    }

    pub fn describe(&self) -> String {
        format!("smoothing={} liquid={} hover={}", self.smoothing, self.liquid, self.hover as u8)
    }
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

    fn view<'a>(&'a self, eq: &'a [f64], pressed: Option<(player::Control, Time)>, scrub: Option<f64>, now: Time) -> player::View<'a> {
        player::View {
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
            control_hover: [0.0; 3],
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeClick {
    Invoke(u32, String),
    Dismiss(u32),
}

#[derive(Clone, Debug)]
struct LiveActivity {
    event: activity::Event,
    since: Time,
    updated_at: Time,
}

#[derive(Clone, Debug)]
struct Countdown {
    duration: f64,
    remaining: f64,
    changed_at: Time,
    running: bool,
}

impl Countdown {
    fn new(duration: f64, now: Time) -> Self {
        Countdown { duration, remaining: duration, changed_at: now, running: true }
    }

    fn remaining(&self, now: Time) -> f64 {
        if self.running { (self.remaining - (now - self.changed_at)).max(0.0) } else { self.remaining }
    }

    fn fraction(&self, now: Time) -> f64 {
        if self.duration <= 0.0 { 0.0 } else { self.remaining(now) / self.duration }
    }
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
    control_hover: [Spring; 3],
    art: Art,
    /// equaliser bands: the latest levels from cava and the eased heights
    eq_target: [f64; EQ_BANDS],
    eq: [f64; EQ_BANDS],
    eq_t: Time,
    /// playback nobody hears (paused, or the sink muted / at 0 %) since
    silent_since: Option<Time>,
    pub muted: bool,
    /// Touch ID: 0 = the line, 1 = the 92 px pill.
    pill: Quintic,
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
    pub dnd: bool,
    roll: Quintic,
    /// the pill's size, eased when a new notification needs another (a
    /// long message wraps to two lines and the pill grows to fit)
    pill_w: Quintic,
    pill_h: Quintic,
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
    timer_hovered: bool,
    timer_menu_rect: Option<Rect>,
    timer_menu_hover: Option<&'static str>,
    network_menu_rect: Option<Rect>,
    network_panel: Option<crate::connections::Panel>,
    network_session: u64,
    timer_button_hover: [Spring; 7],
    timer_button_enabled: [Spring; 7],
    timer_last: Option<Countdown>,
    quick_open: bool,
    quick_rect: Option<Rect>,
    quick_hover: Option<quick::Action>,
    quick_selected: usize,
    quick_hover_springs: [Spring; 6],
    quick_close_hover: Spring,
    eq_rect: Option<Rect>,
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
    /// Neovim (the companion plugin): the editor's state, the chip that
    /// shows it beside the clock, and the panel it opens into
    nvim: Option<Editor>,
    nvim_seen: Time,
    /// when the editor last wrote a file: the chip flashes for a moment
    nvim_saved: Option<Time>,
    chip: Quintic,
    chip_w: f64,
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
            pill: Quintic::at_rest(0.0, PILL_DUR),
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
            dnd: notice_center::load_dnd(),
            roll: Quintic::at_rest(1.0, ROLL_DUR),
            pill_w: Quintic::at_rest(PILL_MIN_W, PILL_W_DUR),
            pill_h: Quintic::at_rest(PILL_H, PILL_H_DUR),
            activity: None,
            activity_retiring: None,
            activity_queued: None,
            activity_value: Spring::new(0.0, 0.28, 0.82),
            activity_art: Art::new(),
            timer: None,
            timer_chip: Quintic::at_rest(0.0, CHIP_DUR),
            timer_w: 0.0,
            timer_rect: None,
            timer_hovered: false,
            timer_menu_rect: None,
            timer_menu_hover: None,
            network_menu_rect: None,
            network_panel: None,
            network_session: 0,
            timer_button_hover: std::array::from_fn(|_| Spring::new(0.0, 0.25, 1.0)),
            timer_button_enabled: std::array::from_fn(|i| Spring::new(if i == 4 || i == 5 { 0.0 } else { 1.0 }, 0.25, 1.0)),
            timer_last: None,
            quick_open: false,
            quick_rect: None,
            quick_hover: None,
            quick_selected: 0,
            quick_hover_springs: std::array::from_fn(|_| Spring::new(0.0, 0.24, 0.83)),
            quick_close_hover: Spring::new(0.0, 0.24, 0.83),
            eq_rect: None,
            player_rect: None,
            eq_hovered: false,
            editor_chip_rect: None,
            editor_chip_hovered: false,
            panel: Quintic::at_rest(0.0, PANEL_DUR),
            launcher: None,
            catalog: crate::apps::Catalog::new(),
            launcher_icons: std::rc::Rc::new(crate::iconload::Icons::new()),
            file_search: std::rc::Rc::new(crate::files::Search::new()),
            nvim: None,
            nvim_seen: 0.0,
            nvim_saved: None,
            chip: Quintic::at_rest(0.0, CHIP_DUR),
            chip_w: 0.0,
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
            || self.timer_button_hover.iter().chain(&self.timer_button_enabled).any(|s| s.is_moving(now))
            || self.quick_hover_springs.iter().any(|s| s.is_moving(now))
            || self.quick_close_hover.is_moving(now)
            || self.notice_action_hover.iter().any(|spring| spring.is_moving(now))
            || self.center.is_animating(now)
            || self.network_panel.as_ref().is_some_and(|p| p.is_animating(now))
            || self.timer_chip.is_moving(now)
            || self.panel.is_moving(now)
            || self.chip.is_moving(now)
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
        self.silent_since.is_some_and(|since| now - since >= SILENT_AFTER)
    }

    /// Re-aim the movers after a state change.
    pub fn sync(&mut self, now: Time) {
        // System events and panels rise on their own even when the compact
        // island is parked above the edge. Once they finish, it stays hidden.
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
                _ => player::Control::Next,
            };
            let lit = self.hovered_control == Some(control) && self.scrub.is_none();
            spring.set(if lit { 1.0 } else { 0.0 }, now);
        }
        // the editor's chip rides beside the clock while Neovim is in front
        let chip = self.editor_live(now) && !self.hidden.any() && !self.dodged && !self.pill_open() && self.launcher.is_none();
        self.chip.set(if chip { 1.0 } else { 0.0 }, now);
        let timer = self.timer.is_some() && !self.hidden.any() && !self.dodged && self.launcher.is_none();
        for (i, (command, ..)) in activity::TIMER_BUTTONS.iter().enumerate() {
            let enabled = !matches!(*command, "toggle" | "cancel") || self.timer.is_some();
            self.timer_button_hover[i].set(if enabled && self.timer_menu_hover == Some(command) { 1.0 } else { 0.0 }, now);
            self.timer_button_enabled[i].set(if enabled { 1.0 } else { 0.0 }, now);
        }
        for (i, spring) in self.quick_hover_springs.iter_mut().enumerate() {
            spring.set(if self.quick_hover == Some(quick::ACTIONS[i]) { 1.0 } else { 0.0 }, now);
        }
        self.quick_close_hover.set(if self.quick_hover == Some(quick::Action::Close) { 1.0 } else { 0.0 }, now);
        if !self.connections_open() {
            if let Some(panel) = &mut self.network_panel { panel.close(); }
            if !self.activity_retiring.as_ref().is_some_and(|a| matches!(a.event, activity::Event::WifiOpen | activity::Event::BluetoothOpen)) {
                self.network_panel = None;
            }
        }
        self.timer_chip.set(if timer { 1.0 } else { 0.0 }, now);
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
        self.pill.set(if self.pill_open() && !self.want_player() { 1.0 } else { 0.0 }, now);
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
        if self.connections_open() {
            if let Some(panel) = &mut self.network_panel { changed |= panel.advance(now); }
        }
        if self.toast.is_some_and(|since| now - since >= TOAST_HOLD) {
            self.toast = None;
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
        if self.activity.as_ref().is_some_and(|a| now - a.updated_at >= a.event.hold()) {
            self.activity_retiring = self.activity.take();
            changed = true;
        }
        if self.activity_retiring.is_some() && self.pill.state(now).x <= 0.0 {
            self.activity_retiring = None;
            self.activity_art.clear();
            if !self.connections_open() { self.network_panel = None; }
        }
        if self.timer.as_ref().is_some_and(|timer| timer.running && timer.remaining(now) <= 0.0) {
            self.timer = None;
            self.live_activity(activity::Event::TimerDone, now);
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
        if self.notice.as_ref().is_some_and(|n| now - n.since >= NOTICE_HOLD) {
            self.notice = None;
            self.notice_prev = None;
            changed = true;
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
        match self.auth {
            Some(Auth::Ok(since)) => consider(since + AUTH_OK_HOLD),
            Some(Auth::Fail(since)) => consider(since + AUTH_FAIL_HOLD),
            _ => {}
        }
        if let Some(n) = &self.notice {
            consider(n.since + NOTICE_HOLD);
        }
        if let Some(a) = &self.activity {
            if a.event.hold().is_finite() {
                consider(a.updated_at + a.event.hold());
            }
        }
        if let Some(timer) = &self.timer {
            if timer.running {
                let remaining = timer.remaining(now);
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
        if let Some(since) = self.toast {
            consider(since + TOAST_HOLD);
        }
        if let Some(since) = self.silent_since {
            if now - since < SILENT_AFTER && self.song.is_some() {
                consider(since + SILENT_AFTER);
            }
        }
        next
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden.any() && self.auth.is_none() && self.activity.is_none()
            && self.notice.is_none() && self.launcher.is_none() && !self.center.open
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
            + &self.timer.as_ref().map_or_else(String::new, |t| format!(" timer={:.0}s{}", t.remaining(now), if t.running { "" } else { "(paused)" }))
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

    /// Show the "now playing" toast, unless the island has something more
    /// important on (or the player is already open under the pointer).
    fn announce(&mut self, now: Time) {
        if self.hidden.any() || self.auth.is_some() || self.activity.is_some() || self.notice.is_some() || self.launcher.is_some() || self.want_player() {
            return;
        }
        if self.song.as_ref().is_some_and(|s| s.playing()) {
            self.toast = Some(now);
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
            self.launcher = Some(Launcher::new(now, self.catalog.apps.clone(), self.launcher_icons.clone(), self.file_search.clone()));
        }
        self.sync(now);
        self.launcher.is_some()
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

    pub fn set_asset_waker(&self, tx: smithay_client_toolkit::reexports::calloop::channel::Sender<()>) {
        self.catalog.set_waker(tx.clone());
        self.launcher_icons.set_waker(tx.clone());
        self.file_search.set_waker(tx.clone());
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
        let mut files = false;
        for (generation, reply) in self.file_search.drain() {
            if let Some(launcher) = &mut self.launcher {
                files |= launcher.receive_files(generation, reply, now);
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
        // an update of the same job (LSP progress, say) takes the pill over
        // in place: no roll, the timer simply starts again
        if let Some(current) = self.notice.as_mut() {
            if (note.id != 0 && current.note.id == note.id)
                || (note.tag.is_some() && current.note.tag == note.tag) {
                current.note = note;
                current.since = now;
                self.notice_prev = None;
                self.sync(now);
                return;
            }
        }
        if self.notice.is_some() {
            // a newer one while the pill is up: the old rolls out through
            // the top, the new one in from below
            self.notice_prev = self.notice.take();
            self.roll.set_at(0.0, now);
            self.roll.set(1.0, now);
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
        self.quick_open || self.center.open || self.activity.as_ref().is_some_and(|current| activity::takes_keyboard(&current.event))
    }

    pub fn close_activity_panel(&mut self, now: Time) -> bool {
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
                if timer.running {
                    timer.remaining = timer.remaining(now);
                    timer.changed_at = now;
                    timer.running = false;
                }
            }
            "resume" => {
                let timer = self.timer.as_mut().ok_or("no active timer")?;
                if !timer.running && timer.remaining > 0.0 {
                    timer.changed_at = now;
                    timer.running = true;
                }
            }
            "toggle" => {
                if !self.toggle_timer(now) {
                    return Err("no active timer".into());
                }
            }
            "cancel" | "stop" => self.timer = None,
            "status" => {
                return Ok(self.timer.as_ref().map_or_else(
                    || "stopped".to_string(),
                    |timer| format!("{:.0} {}", timer.remaining(now).ceil(), if timer.running { "running" } else { "paused" }),
                ));
            }
            "pomodoro" => self.timer = Some(Countdown::new(25.0 * 60.0, now)),
            duration => {
                let seconds = activity::parse_duration(duration).ok_or_else(|| format!("bad duration {duration:?}"))?;
                self.timer = Some(Countdown::new(seconds, now));
            }
        }
        self.sync(now);
        Ok("ok".into())
    }

    /// A left click on the compact timer pauses or resumes it.
    pub fn toggle_timer(&mut self, now: Time) -> bool {
        let Some(timer) = self.timer.as_mut() else { return false };
        if timer.running {
            timer.remaining = timer.remaining(now);
            timer.running = false;
        } else if timer.remaining > 0.0 {
            timer.running = true;
        }
        timer.changed_at = now;
        self.sync(now);
        true
    }

    pub fn click_timer_at(&mut self, px: f64, py: f64, now: Time) -> bool {
        if self.pill_open() || self.launcher.is_some() || self.hidden.any() {
            return false;
        }
        let Some(rect) = self.timer_rect else { return false };
        if px < rect.x || px > rect.x + rect.w || py < rect.y || py > rect.y + rect.h {
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
        if !self.timer_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h) {
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
            if let Some(command) = activity::timer_menu_hit(px - r.x, py - r.y) {
                if command == "close" {
                    self.activity_retiring = self.activity.take();
                } else if self.timer_command(command, now).is_ok() && !matches!(command, "toggle" | "cancel") {
                    self.activity_retiring = self.activity.take();
                }
                self.sync(now);
            }
        }
        true
    }

    pub fn set_timer_hover(&mut self, px: f64, py: f64) -> bool {
        let menu_hover = self.timer_menu_rect.and_then(|r| activity::timer_menu_hit(px - r.x, py - r.y));
        let changed = self.timer_menu_hover != menu_hover;
        self.timer_menu_hover = menu_hover;
        let hovered = self.timer_rect.is_some_and(|rect| {
            px >= rect.x && px <= rect.x + rect.w && py >= rect.y && py <= rect.y + rect.h
        });
        if hovered == self.timer_hovered {
            return changed;
        }
        self.timer_hovered = hovered;
        true
    }

    pub fn clear_timer_hover(&mut self) -> bool {
        let changed = self.timer_hovered || self.timer_menu_hover.is_some();
        self.timer_menu_hover = None;
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
            if panel.click(px - rect.x, py - rect.y, now) { self.close_activity_panel(now); }
        }
        true
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
        self.sync(now);
    }

    pub fn update_clock(&mut self) -> bool {
        let text = clock_text();
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
        let reveal = self.reveal.state(now).x;
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
        let cx = cx - DODGE * dodge.x * (1.0 - p);
        let dy = -BOX_H * hide.x * (1.0 - p);
        if dy <= -(BOX_H + FILLET) && p <= 0.0 {
            return Frame::default();
        }
        let pad = PAD + (PAD_HOVER - PAD) * hover_pad + SPREAD * spread_k;
        let radius = RADIUS + (RADIUS_HIDDEN - RADIUS) * hide.x;
        let pill_radius = if self.quick_open || self.center.open || self.visible_activity().is_some_and(|a| activity::takes_keyboard(&a.event)) {
            30.0
        } else {
            self.pill_h.state(now).x / 2.0
        };
        let radius = radius
            + (pill_radius - radius) * q
            + (PLAYER_R - radius) * pl
            + (editor::PANEL_R - radius) * ed
            + (launcher::PANEL_R - radius) * p;

        // "liquid": the box stretches along its direction of travel in
        // proportion to its speed (px/s) and relaxes as it stops
        let liquid = self.config.liquid;
        let stretch_y = liquid * 0.02 * (hide.v * BOX_H).abs();
        let stretch_x = liquid * 0.01 * (dodge.v * DODGE).abs();

        // --- the line: the clock, with the bars beating beside it while
        // something plays (the track's name lives in the toast and in the
        // player now, not here) ---
        let clock = renderer.layout(cr, &self.clock, Font::Text);
        let (_, clock_ext) = clock.pixel_extents();
        let clock_w = clock_ext.width() as f64;
        let clock_h = clock_ext.height() as f64;
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
            self.timer_w = activity::timer_width(cr, renderer, timer.remaining(now));
        }
        let timer_len = timer_k * (self.timer_w + 10.0);
        self.timer_rect = None;
        self.timer_menu_rect = None;
        self.network_menu_rect = None;
        self.quick_rect = None;
        self.center_rect = None;
        self.notice_rect = None;
        self.eq_rect = None;
        self.player_rect = None;
        self.editor_chip_rect = None;
        let unread_len = if self.center.unread() > 0 { 18.0 } else { 0.0 };
        let dnd_len = if self.dnd { 17.0 } else { 0.0 };
        let line_w = chip_len + clock_w + timer_len + bars_len + unread_len + dnd_len + 2.0 * pad;

        // --- the pill: Touch ID, a live activity, or a notification ---
        let activity_size = self.center.open.then_some((notice_center::WIDTH, notice_center::HEIGHT))
            .or_else(|| self.quick_open.then_some((quick::WIDTH, quick::HEIGHT))).or_else(|| self.visible_activity().map(|a| {
            if matches!(a.event, activity::Event::WifiOpen | activity::Event::BluetoothOpen) {
                self.network_panel.as_ref().map_or(a.event.size(), |p| (crate::connections::WIDTH, p.height()))
            } else { a.event.size() }
        }));
        let notice_size = self.notice.as_ref().map(|n| renderer.notice_size(cr, &n.note));
        let toast_size = self.song.as_ref().filter(|_| self.toast.is_some()).map(|song| {
            let view = song.view(&self.eq, None, None, now);
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
            - stretch_x * 0.15;
        let (mut x0, mut x1, mut y1) = (cx - bw / 2.0, cx + bw / 2.0, dy + bh);
        if !moving {
            // crisp edges at rest; while moving they glide sub-pixel
            x0 = renderer.snap(x0);
            x1 = renderer.snap(x1);
            y1 = renderer.snap(y1);
        }
        let r = radius.min((x1 - x0) / 2.0).min((y1 - dy) / 2.0);
        if pl > 0.05 && self.player.target() > 0.5 && self.song.is_some() {
            self.player_rect = Some(Rect { x: x0, y: dy, w: x1 - x0, h: y1 - dy });
        }

        self.trace_notch(cr, x0, x1, dy, y1, r);
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
        // everything inside stays inside (the pill content climbs in from
        // below the box): a pixel-aligned rectangle, which cairo clips for
        // free, is enough — the content never reaches the corners
        cr.save().unwrap();
        let (cx0, cy0) = (renderer.floor(x0), renderer.floor(dy));
        cr.rectangle(cx0, cy0, renderer.ceil(x1) - cx0, renderer.ceil(y1) - cy0);
        cr.clip();

        // the line's content rides at the top of the box and dissolves
        // while the pill opens (first third), the pill's content climbs in
        // from below and fades in over the rest
        let line_alpha = (1.0 - smoothstep(q / PILL_SWAP)) * (1.0 - smoothstep(pl / 0.45))
            * (1.0 - smoothstep(ed / 0.45))
            * if self.hidden.any() && self.launcher.is_none() { 0.0 } else { 1.0 };
        let pill_alpha = pill_alpha_of(q);
        if line_alpha > 0.0 {
            let content_w = chip_len + clock_w + timer_len + bars_len + unread_len + dnd_len;
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
            let clock_x = renderer.snap(content_x + chip_len - clock_ext.x() as f64);
            let ty = renderer.snap(dy + (line_h - clock_h) / 2.0 - clock_ext.y() as f64);
            cr.set_source_rgba(1.0, 1.0, 1.0, line_alpha);
            cr.move_to(clock_x, ty);
            pangocairo::functions::show_layout(cr, &clock);
            let mut after_clock = clock_x + clock_ext.x() as f64 + clock_w;
            if timer_len > 0.5 {
                if self.timer.is_some() && line_alpha > 0.95 && timer_k > 0.95 && !self.pill_open() {
                    self.timer_rect = Some(Rect { x: after_clock + 6.0, y: dy, w: self.timer_w + 8.0, h: line_h });
                }
                cr.save().unwrap();
                cr.rectangle(after_clock, dy, timer_len, line_h);
                cr.clip();
                if let Some(timer) = self.timer.as_ref().or(self.timer_last.as_ref()) {
                    activity::draw_timer(
                        cr,
                        renderer,
                        after_clock + 10.0,
                        dy + line_h / 2.0,
                        timer.remaining(now),
                        timer.fraction(now),
                        timer.running,
                        line_alpha * timer_k,
                    );
                }
                cr.restore().unwrap();
                after_clock += timer_len;
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
                player::bars(cr, renderer, &self.eq, bx, dy + (line_h - BARS_H) / 2.0, BARS_W, BARS_H, line_alpha * reveal);
                cr.restore().unwrap();
            }
            if unread_len > 0.0 {
                let bx = after_clock + bars_len + 13.0;
                cr.arc(bx, dy + line_h / 2.0, 2.7, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(0.40, 0.82, 1.0, line_alpha);
                let _ = cr.fill();
            }
            if self.dnd {
                let mx = after_clock + bars_len + unread_len + 8.0;
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
        }


        if pill_alpha > 0.0 {
            if let Some(auth) = self.auth {
                let (result, caption, since) = match auth {
                    Auth::Waiting(since) => (None, "Touch ID", since),
                    Auth::Ok(since) => (Some((touchid::Mark::Check, rgb(0x34C759))), "Touch ID", since),
                    Auth::Fail(since) => (Some((touchid::Mark::Cross, rgb(0xFF3B30))), "Try again", since),
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
                if pill_alpha > 0.95 {
                    self.center_rect = Some(Rect { x, y: dy + climb, w: notice_center::WIDTH, h: notice_center::HEIGHT });
                }
                self.center.draw(cr, renderer, &self.launcher_icons, x, dy + climb, pill_alpha, now);
            } else if self.quick_open {
                let climb = (1.0 - pill_alpha) * PILL_CLIMB;
                if pill_alpha > 0.95 {
                    self.quick_rect = Some(Rect { x: cx - quick::WIDTH / 2.0, y: dy + climb, w: quick::WIDTH, h: quick::HEIGHT });
                }
                quick::draw(cr, renderer, cx - quick::WIDTH / 2.0, dy + climb, pill_alpha,
                    std::array::from_fn(|i| self.quick_hover_springs[i].value(now)), self.quick_close_hover.value(now), self.center.unread(), self.dnd);
            } else if let Some(activity) = self.visible_activity().cloned() {
                let climb = (1.0 - pill_alpha) * PILL_CLIMB;
                if activity.event == activity::Event::TimerMenu {
                    let mx = cx - 195.0;
                    if pill_alpha > 0.95 {
                        self.timer_menu_rect = Some(Rect { x: mx, y: dy + climb, w: 390.0, h: 184.0 });
                    }
                    activity::draw_timer_menu(cr, renderer, mx, dy + climb,
                        self.timer.as_ref().map(|t| (t.remaining(now), t.running)), &std::array::from_fn(|i| self.timer_button_hover[i].value(now).clamp(0.0, 1.0)),
                        &std::array::from_fn(|i| self.timer_button_enabled[i].value(now).clamp(0.0, 1.0)), pill_alpha, activity.since, now);
                }
                if matches!(activity.event, activity::Event::WifiOpen | activity::Event::BluetoothOpen) {
                    let (w, _) = activity.event.size();
                    if pill_alpha > 0.95 {
                        self.network_menu_rect = Some(Rect { x: cx - w / 2.0, y: dy + climb, w, h: bh });
                    }
                    if let Some(panel) = &mut self.network_panel {
                        panel.draw(cr, renderer, cx - w / 2.0, dy + climb, bh, pill_alpha, now);
                    }
                }
                activity::draw_pill(
                    cr,
                    renderer,
                    &self.activity_art,
                    &activity.event,
                    self.activity_value.value(now),
                    x0,
                    dy + climb,
                    x1 - x0,
                    bh,
                    pill_alpha,
                    activity.since,
                    now,
                );
            } else if let Some(since) = self.toast {
                if let Some(song) = &self.song {
                    let view = song.view(&self.eq, None, None, now);
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
                    if pill_alpha > 0.95 && roll > 0.95 {
                        let (_, _, w, _) = renderer.notice_layouts(cr, &notice.note);
                        self.notice_rect = Some(Rect { x: cx - w / 2.0, y: dy, w, h: bh });
                    }
                    self.draw_notice(cr, renderer, notice, cx, dy, bh, pill_alpha * roll.powf(0.7), climb, now);
                }
            }
        }
        if pl > 0.0 {
            if let Some(song) = &self.song {
                let mut view = song.view(&self.eq, self.pressed, self.scrub.map(|(f, _)| f), now);
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

        let top = dy.max(0.0);
        let bounds = Rect { x: x0 - FILLET, y: top, w: x1 - x0 + 2.0 * FILLET, h: y1 - top };
        // the input region covers whichever is larger, the box now or the
        // box it is growing into
        let (target_w, target_h) = self.target_size(line_w, line_h, now);
        let input_w = (x1 - x0).max(target_w);
        let input_h = (y1 - dy).max(target_h);
        let input = Rect {
            x: cx - input_w / 2.0 - FILLET,
            y: top,
            w: input_w + 2.0 * FILLET,
            h: (dy + input_h - top).max(0.0),
        };
        // opaque: the box minus its curved corners (a T shape)
        let opaque = [
            (y1 - r > top).then(|| Rect { x: x0, y: top, w: x1 - x0, h: y1 - r - top }),
            (x1 - x0 > 2.0 * r).then(|| Rect { x: x0 + r, y: (y1 - r).max(top), w: x1 - x0 - 2.0 * r, h: r.min(y1 - top) }),
        ];
        Frame { bounds: Some(bounds), input: Some(input), opaque }
    }

    /// One notification's content inside the pill: the app's icon with the
    /// countdown ring, its name and the message.
    #[allow(clippy::too_many_arguments)]
    fn draw_notice(&self, cr: &Context, renderer: &Renderer, notice: &Notice, cx: f64, dy: f64, bh: f64, alpha: f64, offset: f64, now: Time) {
        if alpha <= 0.001 {
            return;
        }
        let note = &notice.note;
        let t = now - notice.since;
        let (app, msg, width, _) = renderer.notice_layouts(cr, note);
        let x0 = cx - width / 2.0;
        let icon_cx = x0 + NOTICE_PAD_L + NOTICE_ICON / 2.0;
        let actions: Vec<_> = note.actions.iter().filter(|(key, _)| key != "default").take(2).collect();
        let mid = dy + bh / 2.0 + offset - if actions.is_empty() { 0.0 } else { 14.0 };
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
            Some(done) => icons::progress_ring(cr, note.kind, icon_cx, mid, NOTICE_ICON / 2.0 + 6.0, done, alpha * smoothstep(t / 0.3)),
            None => icons::countdown(cr, note.kind, icon_cx, mid, NOTICE_ICON / 2.0 + 6.0, 1.0 - (t / NOTICE_HOLD).clamp(0.0, 1.0), alpha * smoothstep(t / 0.3)),
        }

        let (_, app_ext) = app.pixel_extents();
        let (_, msg_ext) = msg.pixel_extents();
        let total = app_ext.height() as f64 + NOTICE_LINE_GAP + msg_ext.height() as f64;
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
        self.eq_rect.or(self.player_rect).is_some_and(|r| {
            px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
        })
    }

    /// A normal click opens shortcuts unless a modal panel owns the input.
    pub fn accepts_quick_menu_click(&self) -> bool {
        !self.hidden.any() && self.auth.is_none() && self.launcher.is_none()
            && !self.keyboard_panel_open() && !self.is_scrubbing()
    }

    pub fn quick_menu_open(&self) -> bool { self.quick_open }

    pub fn notification_id(&self) -> Option<u32> { self.notice.as_ref().map(|n| n.note.id).filter(|id| *id != 0) }

    pub fn set_dnd(&mut self, enabled: bool, now: Time) {
        if self.dnd == enabled { return; }
        self.dnd = enabled;
        notice_center::save_dnd(enabled);
        if enabled && self.notice.as_ref().is_some_and(|notice| !notice.note.critical) {
            self.notice = None;
            self.notice_prev = None;
            self.notice_queued = None;
        }
        self.sync(now);
    }

    pub fn notice_click_at(&self, px: f64, py: f64) -> Option<NoticeClick> {
        let rect = self.notice_rect?;
        if px < rect.x || px > rect.x + rect.w || py < rect.y || py > rect.y + rect.h { return None; }
        let note = &self.notice.as_ref()?.note;
        if note.id == 0 { return None; }
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
        let hovered = self.notice_action_index_at(px, py);
        if self.notice_hovered == hovered { return false; }
        self.notice_hovered = hovered;
        for (index, spring) in self.notice_action_hover.iter_mut().enumerate() {
            spring.set(if hovered == Some(index) { 1.0 } else { 0.0 }, now);
        }
        true
    }

    pub fn notification_closed(&mut self, id: u32, now: Time) {
        self.center.closed(id);
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

    pub fn scroll_notification_center(&mut self, rows: f64) -> bool {
        self.center.open && self.center.scroll(rows)
    }

    pub fn open_quick_menu(&mut self, now: Time) -> bool {
        if !self.accepts_quick_menu_click() { return false; }
        self.quick_open = true;
        self.quick_hover = None;
        self.quick_selected = 0;
        self.eq_hovered = false;
        self.editor_chip_hovered = false;
        self.notice = None;
        self.notice_prev = None;
        self.activity = None;
        self.activity_queued = None;
        self.toast = None;
        self.sync(now);
        true
    }

    pub fn close_quick_menu(&mut self, now: Time) {
        if self.quick_open {
            self.quick_open = false;
            self.quick_rect = None;
            self.quick_hover = None;
            self.sync(now);
        }
    }

    pub fn quick_action_at(&self, px: f64, py: f64) -> Option<quick::Action> {
        self.quick_rect.and_then(|r| quick::hit(px - r.x, py - r.y))
    }

    pub fn quick_key(&mut self, key: &str, now: Time) -> Option<quick::Action> {
        if !self.quick_open { return None; }
        match key {
            "next" => self.quick_selected = (self.quick_selected + 1) % quick::ACTIONS.len(),
            "prev" => self.quick_selected = (self.quick_selected + quick::ACTIONS.len() - 1) % quick::ACTIONS.len(),
            "enter" => return Some(quick::ACTIONS[self.quick_selected]),
            _ => {}
        }
        self.quick_hover = Some(quick::ACTIONS[self.quick_selected]);
        self.sync(now);
        None
    }

    pub fn set_quick_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        let hovered = self.quick_action_at(px, py);
        if self.quick_hover == hovered { return false; }
        self.quick_hover = hovered;
        if let Some(action) = hovered {
            if let Some(i) = quick::ACTIONS.iter().position(|a| *a == action) { self.quick_selected = i; }
        }
        self.sync(now);
        true
    }

    pub fn set_eq_hover(&mut self, px: f64, py: f64, now: Time) -> bool {
        let hovered = self.eq_rect.is_some_and(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h);
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
            player::Control::Previous => crate::dbus::PlayerCommand::Previous,
            player::Control::PlayPause => crate::dbus::PlayerCommand::PlayPause,
            player::Control::Next => crate::dbus::PlayerCommand::Next,
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
        let cx = cx - DODGE * self.dodge.state(now).x * (1.0 - p);
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

    /// The notch outline, traced counter-clockwise: the concave fillets at
    /// the top are right turns, the convex corners at the bottom left turns
    /// (`Corner::trace` mirrored).
    fn trace_notch(&self, cr: &Context, x0: f64, x1: f64, y0: f64, y1: f64, r: f64) {
        let s = self.config.smoothing;
        let h = y1 - y0;
        // the vertical edge is shared by a fillet and a corner: each may
        // smooth over at most its half
        let fillet = Corner::new(FILLET, s, h / 2.0);
        let corner = Corner::new(r, s, (h / 2.0).min((x1 - x0) / 2.0));
        cr.new_path();
        cr.move_to(x0 - fillet.p, y0);
        fillet.trace(cr, x0, y0, 0.0, false);
        cr.line_to(x0, y1 - corner.p);
        corner.trace(cr, x0, y1, FRAC_PI_2, true);
        cr.line_to(x1 - corner.p, y1);
        corner.trace(cr, x1, y1, 0.0, true);
        cr.line_to(x1, y0 + fillet.p);
        fillet.trace(cr, x1, y0, -FRAC_PI_2, false);
        cr.close_path();
    }
}

/// The waiting fingerprint breathes through three blues (the reference
/// cycled 12 colours at 80 ms; this is the same path, continuous).
fn pulse_colour(t: Time) -> (f64, f64, f64) {
    let k = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * t / PULSE_PERIOD).cos();
    let (a, b, c) = (rgb(0x3A6F90), rgb(0x66D9FF), rgb(0xD9F6FF));
    if k < 0.5 { mix(a, b, k * 2.0) } else { mix(b, c, k * 2.0 - 1.0) }
}

/// A right-turn corner with continuous curvature: a circular arc of
/// reduced sweep between two cubic Béziers that take the curvature from
/// zero on the straight edge up to 1/r and back (Figma's corner smoothing,
/// after "Desperately seeking squircles"). `smoothing` 0 = plain arc.
struct Corner {
    r: f64,
    /// distance from the corner point along each edge where the curve begins
    p: f64,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    arc_len: f64,
    arc_sweep: f64,
}

impl Corner {
    fn new(r: f64, smoothing: f64, budget: f64) -> Corner {
        // room for the smoothing: p = (1 + s) r must fit the edge
        let s = smoothing.min((budget / r - 1.0).max(0.0)).clamp(0.0, 1.0);
        let p = (1.0 + s) * r;
        let arc_sweep = FRAC_PI_2 * (1.0 - s);
        let arc_len = (arc_sweep / 2.0).sin() * r * std::f64::consts::SQRT_2;
        let alpha = (FRAC_PI_2 - arc_sweep) / 2.0;
        let p3p4 = r * (alpha / 2.0).tan();
        let beta = FRAC_PI_2 / 2.0 * s;
        let c = p3p4 * beta.cos();
        let d = c * beta.tan();
        let b = ((p - arc_len - c - d) / 3.0).max(0.0);
        let a = 2.0 * b;
        Corner { r, p, a, b, c, d, arc_len, arc_sweep }
    }

    /// Trace the corner whose point is (`x`, `y`); in the local frame the
    /// path arrives along +x and leaves along +y (a right turn), `rot`
    /// turns that frame and `mirror` flips it into a left turn.
    fn trace(&self, cr: &Context, x: f64, y: f64, rot: f64, mirror: bool) {
        cr.save().unwrap();
        cr.translate(x, y);
        cr.rotate(rot);
        if mirror {
            cr.scale(1.0, -1.0);
        }
        let (r, p) = (self.r, self.p);
        if self.arc_sweep >= FRAC_PI_2 - 1e-9 {
            cr.arc(-r, r, r, -FRAC_PI_2, 0.0);
        } else {
            let (a, b, c, d) = (self.a, self.b, self.c, self.d);
            let x0 = -p;
            cr.curve_to(x0 + a, 0.0, x0 + a + b, 0.0, x0 + a + b + c, d);
            let half = self.arc_sweep / 2.0;
            cr.arc(-r, r, r, -FRAC_PI_2 / 2.0 - half, -FRAC_PI_2 / 2.0 + half);
            let (sx, sy) = (x0 + a + b + c + self.arc_len, d + self.arc_len);
            cr.curve_to(sx + d, sy + c, sx + d, sy + b + c, sx + d, sy + a + b + c);
        }
        cr.restore().unwrap();
    }
}

/// `%a %b %-d  %-I:%M %p` in the local time zone, e.g. "Mon Sep 15  1:05 PM".
pub fn clock_text() -> String {
    let now = libc_localtime();
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let hour12 = match now.hour % 12 {
        0 => 12,
        h => h,
    };
    let ampm = if now.hour < 12 { "AM" } else { "PM" };
    format!(
        "{} {} {}  {}:{:02} {}",
        DAYS[now.wday as usize], MONTHS[now.mon as usize], now.mday, hour12, now.min, ampm
    )
}

pub struct LocalTime {
    pub sec: u32,
    pub min: u32,
    pub hour: u32,
    pub mday: u32,
    pub mon: u32,
    pub wday: u32,
}

/// Broken-down local time through libc (no chrono needed for one clock).
pub fn libc_localtime() -> LocalTime {
    unsafe extern "C" {
        fn time(t: *mut i64) -> i64;
        fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
    }
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const u8,
    }
    let mut t = 0i64;
    let mut tm = Tm {
        tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 1, tm_mon: 0, tm_year: 0,
        tm_wday: 0, tm_yday: 0, tm_isdst: 0, tm_gmtoff: 0, tm_zone: std::ptr::null(),
    };
    unsafe {
        time(&mut t);
        localtime_r(&t, &mut tm);
    }
    LocalTime {
        sec: tm.tm_sec as u32,
        min: tm.tm_min as u32,
        hour: tm.tm_hour as u32,
        mday: tm.tm_mday as u32,
        mon: tm.tm_mon as u32,
        wday: tm.tm_wday as u32,
    }
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
}

/// Pango setup shared by all drawing: fonts, hinting for the fractional
/// scale, one pango context and a cache of shaped layouts (shaping the
/// same clock string sixty times a second would cost more than drawing).
pub struct Renderer {
    scale: f64,
    text: pango::FontDescription,
    caption: pango::FontDescription,
    small: pango::FontDescription,
    tiny: pango::FontDescription,
    query: pango::FontDescription,
    ctx: pango::Context,
    cache: RefCell<LayoutCache>,
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
        let mut text = pango::FontDescription::from_string(FONT);
        text.set_absolute_size(FONT_PX * pango::SCALE as f64);
        let mut caption = pango::FontDescription::from_string(FONT);
        caption.set_absolute_size(CAPTION_PX * pango::SCALE as f64);
        let mut small = pango::FontDescription::from_string(FONT_REGULAR);
        small.set_absolute_size(SMALL_PX * pango::SCALE as f64);
        let mut tiny = pango::FontDescription::from_string(FONT_REGULAR);
        tiny.set_absolute_size(TINY_PX * pango::SCALE as f64);
        let mut query = pango::FontDescription::from_string(FONT);
        query.set_absolute_size(QUERY_PX * pango::SCALE as f64);
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
        let renderer = Renderer { scale, text, caption, small, tiny, query, ctx, cache: RefCell::new(LayoutCache::default()) };
        // Resolve common font faces at startup/scale changes, not on the
        // first visible frame of a newly opened panel.
        for font in [Font::Text, Font::Caption, Font::Small, Font::Tiny, Font::Query] {
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
        let text_h = app.pixel_extents().1.height() as f64 + NOTICE_LINE_GAP + msg.pixel_extents().1.height() as f64;
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
        layout.set_font_description(Some(match font {
            Font::Text => &self.text,
            Font::Caption => &self.caption,
            Font::Small => &self.small,
            Font::Tiny => &self.tiny,
            Font::Query => &self.query,
        }));
        layout.set_text(text);
        cache.insert(key, layout.clone());
        layout
    }
}

#[cfg(test)]
mod tests {
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
        for (i, action) in quick::ACTIONS.iter().enumerate() {
            let x = r.x + 38.0 + (i % 2) as f64 * 207.0;
            let y = r.y + 84.0 + (i / 2) as f64 * 80.0;
            assert_eq!(island.quick_action_at(x, y), Some(*action));
        }
        island.set_quick_hover(r.x + 38.0, r.y + 84.0, 10.7);
        assert!(island.quick_hover_springs[0].value(10.8) > 0.0);
        island.close_quick_menu(10.8);
        assert!(!island.keyboard_panel_open());
        assert_eq!(island.quick_action_at(r.x + 38.0, r.y + 84.0), None);
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
    fn rapid_osd_updates_keep_the_entrance_and_extend_the_hold() {
        let mut island = Island::new(Config::default());
        for i in 0..60 {
            let time = 10.0 + i as f64 * 0.05;
            island.live_activity(activity::Event::Slider {
                kind: if i % 2 == 0 { activity::SliderKind::Volume } else { activity::SliderKind::Brightness },
                value: i as f64 / 60.0,
                muted: false,
            }, time);
            island.advance(time);
            let current = island.activity.as_ref().unwrap();
            assert_eq!(current.since, 10.0, "key repeat restarted the entrance");
            assert_eq!(current.updated_at, time);
            assert_eq!(island.pill.target(), 1.0);
        }
        island.advance(14.0);
        assert!(island.activity.is_some());
        island.advance(14.5);
        assert!(island.activity.is_none());
        assert!(island.activity_retiring.is_some(), "keep content during the closing animation");
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

        island.live_activity(activity::Event::TimerDone, 20.0);
        assert!(island.activity.is_some());
        assert_eq!(island.hide.target(), 0.0);
        island.advance(20.0 + activity::Event::TimerDone.hold() + 0.01);
        assert!(island.activity.is_none());
        assert_eq!(island.hide.target(), 1.0);

        assert!(island.toggle_launcher(30.0));
        assert_eq!(island.hide.target(), 0.0);
        island.close_launcher(31.0);
        assert!(island.is_hidden());
        assert_eq!(island.hide.target(), 1.0);
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
        assert_eq!(island.activity_queued, Some(activity::Event::TimerDone));
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
        assert!(island.click_timer_menu(rect.x + 235.0, rect.y + 88.0, 11.0));
        assert_eq!(island.timer_command("status", 11.0).unwrap(), "1500 running");
        assert!(!island.pill_open());
        island.draw(&cr, &renderer, 490.0, 11.5);
        let rect = island.timer_rect.unwrap();
        assert!(island.click_timer_at(rect.x + 5.0, rect.y + 16.0, 11.5));
        assert!(!island.timer.as_ref().unwrap().running);
        island.timer_command("menu", 12.0).unwrap();
        island.draw(&cr, &renderer, 490.0, 13.0);
        let rect = island.timer_menu_rect.unwrap();
        island.click_timer_menu(rect.x + 245.0, rect.y + 138.0, 13.0);
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

    #[test]
    fn timer_hover_eases_in_and_out_without_restarting_the_panel() {
        let mut island = Island::new(Config::default());
        island.timer_command("menu", 10.0).unwrap();
        island.timer_menu_rect = Some(Rect { x: 0.0, y: 0.0, w: 390.0, h: 184.0 });
        island.set_timer_hover(50.0, 80.0);
        island.sync(11.0);
        assert_eq!(island.timer_button_hover[0].value(11.0), 0.0);
        let value = island.timer_button_hover[0].value(11.05);
        assert!(value > 0.0 && value < 1.0);
        island.clear_timer_hover();
        island.sync(11.05);
        assert!((island.timer_button_hover[0].value(11.05) - value).abs() < 1e-9);
        assert!(island.timer_button_hover[0].value(12.0) < 0.001);
        assert_eq!(island.activity.as_ref().unwrap().since, 10.0);
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
}
