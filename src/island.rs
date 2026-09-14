//! The island itself: its state, the animated quantities derived from it and
//! the drawing. Everything here is in logical pixels; `Renderer` maps them to
//! the output's pixels through the cairo scale.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;

use cairo::Context;

use crate::anim::{Quintic, Spring, Time, inv_min_jerk};
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

const SONG_MAX_CHARS: usize = 42;
/// Silent playback (paused / muted) is shown this long before the title
/// slides away.
const SILENT_AFTER: f64 = 15.0;
/// Equaliser: four 3 px bars in a 36 px slot at the start of the line.
pub const EQ_BANDS: usize = crate::eq::BANDS;
const EQ_W: f64 = 36.0;
const EQ_X0: f64 = 6.0;
const EQ_STEP: f64 = 7.0;
const EQ_BAR_W: f64 = 3.0;
const EQ_BOTTOM: f64 = 6.0;
const EQ_MIN_H: f64 = 2.0;
const EQ_MAX_H: f64 = 11.5;
const EQ_COLOR: (f64, f64, f64) = (0x66 as f64 / 255.0, 0xD9 as f64 / 255.0, 1.0);
/// easing of the bars towards the latest levels / down to flat when paused
const EQ_TAU: f64 = 0.045;
const EQ_TAU_SINK: f64 = 0.25;
/// Touch ID pill.
const PILL_H: f64 = 92.0;
const PILL_R: f64 = 44.0;
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

const HIDE_DUR: f64 = 0.25;
const REVEAL_DUR: f64 = 0.3;
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
}

impl HideReasons {
    fn any(self) -> bool {
        self.user || self.fullscreen
    }
}

/// A track shown in the island.
#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub bus_name: String,
    /// "Artist — Title", cut to 42 characters
    pub text: String,
    pub playing: bool,
}

impl Song {
    pub fn from_player(np: &crate::dbus::NowPlaying) -> Song {
        let mut text = if np.artist.is_empty() { np.title.clone() } else { format!("{} — {}", np.artist, np.title) };
        if text.chars().count() > SONG_MAX_CHARS {
            text = text.chars().take(SONG_MAX_CHARS - 1).collect::<String>() + "…";
        }
        Song { bus_name: np.bus_name.clone(), text, playing: np.playing }
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
    /// Now playing: the song text slides out from behind the divider
    /// (0 = clock only, 1 = fully out).
    reveal: Quintic,
    song: Option<Song>,
    /// a track change waits for the current text to retract first
    song_pending: Option<Option<Song>>,
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

fn smoothstep(x: f64) -> f64 {
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
            song_pending: None,
            eq_target: [0.0; EQ_BANDS],
            eq: [0.0; EQ_BANDS],
            eq_t: 0.0,
            silent_since: None,
            muted: false,
            pill: Quintic::at_rest(0.0, PILL_DUR),
            auth: None,
            auth_start: 0.0,
            last_mark: None,
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
            || self.pill.is_moving(now)
            || self.eq_moving()
            // the pill's content animates the whole time it is up
            || self.auth.is_some() && self.pill.state(now).x > 0.0
    }

    fn eq_moving(&self) -> bool {
        self.eq.iter().zip(&self.eq_target).any(|(a, b)| (a - b).abs() > 0.004)
    }

    /// Is the song text meant to be out?
    fn want_reveal(&self, now: Time) -> bool {
        self.song.is_some()
            && self.song_pending.is_none()
            && !self.dodged
            && self.auth.is_none()
            && !self.silent_expired(now)
    }

    fn silent_expired(&self, now: Time) -> bool {
        self.silent_since.is_some_and(|since| now - since >= SILENT_AFTER)
    }

    /// Re-aim the movers after a state change.
    pub fn sync(&mut self, now: Time) {
        self.hide.set(if self.hidden.any() { 1.0 } else { 0.0 }, now);
        self.dodge.set(if self.dodged { 1.0 } else { 0.0 }, now);
        // no hover feedback while hidden: the box is not there to point at
        let hover = self.config.hover && self.hovered && !self.hidden.any() && self.auth.is_none();
        self.hover_pad.set(if hover { 1.0 } else { 0.0 }, now);
        self.reveal.set(if self.want_reveal(now) { 1.0 } else { 0.0 }, now);
        self.pill.set(if self.auth.is_some() { 1.0 } else { 0.0 }, now);
        let result = matches!(self.auth, Some(Auth::Ok(_) | Auth::Fail(_)));
        self.ring.set(if result { 1.0 } else { 0.0 }, now);
        if !result {
            self.mark.set(0.0, now);
        }
        if !self.song.as_ref().is_some_and(|s| s.playing) {
            self.eq_target = [0.0; EQ_BANDS];
        }
    }

    /// Time-driven state changes: a pending track swap once the old text
    /// is in, the Touch ID result timing out, silence expiring. Returns
    /// whether anything changed (then redraw).
    pub fn advance(&mut self, now: Time) -> bool {
        let mut changed = false;
        if self.song_pending.is_some() && (!self.reveal.is_moving(now) && self.reveal.state(now).x == 0.0 || self.song.is_none()) {
            self.song = self.song_pending.take().unwrap();
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
        if self.silent_expired(now) && self.reveal.state(now).x > 0.0 && self.song_pending.is_none() {
            changed = true; // sync retracts it
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
        match self.auth {
            Some(Auth::Ok(since)) => consider(since + AUTH_OK_HOLD),
            Some(Auth::Fail(since)) => consider(since + AUTH_FAIL_HOLD),
            _ => {}
        }
        if let Some(since) = self.silent_since {
            if now - since < SILENT_AFTER && self.song.is_some() {
                consider(since + SILENT_AFTER);
            }
        }
        next
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden.any()
    }

    pub fn song(&self) -> Option<&Song> {
        self.song.as_ref()
    }

    /// One line for `island-shell status`.
    pub fn describe(&self, now: Time) -> String {
        format!(
            "song={:?} pending={} reveal={:.2} pill={:.2} auth={:?} silent={}",
            self.song.as_ref().map(|s| format!("{}{}", if s.playing { "" } else { "(paused) " }, s.text)),
            self.song_pending.is_some(),
            self.reveal.state(now).x,
            self.pill.state(now).x,
            self.auth,
            self.silent_since.map_or(0.0, |s| now - s)
        )
    }

    /// The active player changed (or went away).
    pub fn set_player(&mut self, np: Option<&crate::dbus::NowPlaying>, now: Time) {
        let new = np.map(Song::from_player);
        match (&mut self.song, new) {
            (None, new) => {
                self.song = new;
                self.song_pending = None;
            }
            (Some(cur), Some(new)) if cur.text == new.text => {
                // play / pause or another player with the same track
                cur.playing = new.playing;
                cur.bus_name = new.bus_name;
                self.song_pending = None;
            }
            (Some(_), new) => {
                // another track: the old text slides away first, then the
                // new one slides in (`advance` does the swap)
                self.song_pending = Some(new);
            }
        }
        self.update_silence(now);
        self.sync(now);
    }

    /// The sink's mute state changed.
    pub fn set_muted(&mut self, muted: bool, now: Time) {
        self.muted = muted;
        self.update_silence(now);
        self.sync(now);
    }

    fn update_silence(&mut self, now: Time) {
        let silent = self.song.as_ref().is_some_and(|s| !s.playing || self.muted);
        match (silent, self.silent_since) {
            (true, None) => self.silent_since = Some(now),
            (false, Some(_)) => self.silent_since = None,
            _ => {}
        }
    }

    /// New band levels from cava (0..1).
    pub fn set_eq(&mut self, levels: [f32; EQ_BANDS]) {
        if self.song.as_ref().is_some_and(|s| s.playing) {
            self.eq_target = levels.map(|v| v as f64);
        }
    }

    /// fprintd started a verification.
    pub fn auth_started(&mut self, now: Time) {
        if self.hidden.any() {
            return; // never shows while hidden
        }
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
        let playing = self.song.as_ref().is_some_and(|s| s.playing);
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
        // the spread lags behind the slide — most of it happens while the box
        // is already mostly gone, like a drop flattening on water; it is a
        // function of the position so that show and hide trace the same path
        let spread_k = inv_min_jerk(hide.x).powf(2.2);

        let cx = cx - DODGE * dodge.x;
        let dy = -BOX_H * hide.x;
        if dy <= -(BOX_H + FILLET) {
            return Frame::default();
        }
        let pad = PAD + (PAD_HOVER - PAD) * hover_pad + SPREAD * spread_k;
        let radius = RADIUS + (RADIUS_HIDDEN - RADIUS) * hide.x;
        let radius = radius + (PILL_R - radius) * q;

        // "liquid": the box stretches along its direction of travel in
        // proportion to its speed (px/s) and relaxes as it stops
        let liquid = self.config.liquid;
        let stretch_y = liquid * 0.02 * (hide.v * BOX_H).abs();
        let stretch_x = liquid * 0.01 * (dodge.v * DODGE).abs();

        // --- the line: [eq][gap][song][gap]|[gap] clock ---
        let clock = renderer.layout(cr, &self.clock, Font::Text);
        let (_, clock_ext) = clock.pixel_extents();
        let clock_w = clock_ext.width() as f64;
        let clock_h = clock_ext.height() as f64;
        let group = self.song.as_ref().filter(|_| reveal > 0.0).map(|song| {
            let title = renderer.layout(cr, &song.text, Font::Text);
            let divider = renderer.layout(cr, "|", Font::Text);
            let gap2 = renderer.layout(cr, "  ", Font::Text).pixel_extents().1.width() as f64;
            let gap3 = renderer.layout(cr, "   ", Font::Text).pixel_extents().1.width() as f64;
            let title_w = title.pixel_extents().1.width() as f64;
            let div_w = divider.pixel_extents().1.width() as f64;
            let len = EQ_W + gap2 + title_w + gap3 + div_w + gap3;
            (title, divider, gap2, gap3, title_w, div_w, len)
        });
        let group_len = group.as_ref().map_or(0.0, |g| g.6);
        let line_w = clock_w + reveal * group_len + 2.0 * pad;

        // --- the Touch ID pill ---
        let pill_w = PILL_MIN_W;
        let line_h = BOX_H;
        let bw = line_w + (pill_w - line_w) * q + stretch_x;
        let bh = line_h + (PILL_H - line_h) * q + stretch_y - stretch_x * 0.15;
        let (mut x0, mut x1, mut y1) = (cx - bw / 2.0, cx + bw / 2.0, dy + bh);
        if !moving {
            // crisp edges at rest; while moving they glide sub-pixel
            x0 = renderer.snap(x0);
            x1 = renderer.snap(x1);
            y1 = renderer.snap(y1);
        }
        let r = radius.min((x1 - x0) / 2.0).min((y1 - dy) / 2.0);

        self.trace_notch(cr, x0, x1, dy, y1, r);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.fill();
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
        let line_alpha = 1.0 - smoothstep(q / PILL_SWAP);
        let pill_alpha = smoothstep((q - PILL_SWAP) / (1.0 - PILL_SWAP));
        if line_alpha > 0.0 {
            let content_w = clock_w + reveal * group_len;
            let clock_x = renderer.snap(cx + content_w / 2.0 - clock_w - clock_ext.x() as f64);
            let ty = renderer.snap(dy + (line_h - clock_h) / 2.0 - clock_ext.y() as f64);
            cr.set_source_rgba(1.0, 1.0, 1.0, line_alpha);
            cr.move_to(clock_x, ty);
            pangocairo::functions::show_layout(cr, &clock);

            if let Some((title, divider, gap2, gap3, title_w, div_w, len)) = group {
                // the group is anchored to the clock and clipped to what has
                // emerged from behind the divider
                let group_x = clock_x - len;
                cr.save().unwrap();
                cr.rectangle(clock_x - reveal * len, dy, reveal * len, line_h);
                cr.clip();
                let mut x = group_x;
                // equaliser: four 3 px bars, bottoms 6 px above the line's
                // bottom, 2..11.5 px tall
                let bottom = dy + line_h - EQ_BOTTOM;
                cr.set_source_rgba(EQ_COLOR.0, EQ_COLOR.1, EQ_COLOR.2, line_alpha);
                for (i, level) in self.eq.iter().enumerate() {
                    let h = EQ_MIN_H + (EQ_MAX_H - EQ_MIN_H) * level;
                    let bx = renderer.snap(x + EQ_X0 + i as f64 * EQ_STEP);
                    cr.rectangle(bx, bottom - h, EQ_BAR_W, h);
                }
                let _ = cr.fill();
                x += EQ_W + gap2;
                cr.set_source_rgba(1.0, 1.0, 1.0, line_alpha);
                cr.move_to(renderer.snap(x), ty);
                pangocairo::functions::show_layout(cr, &title);
                x += title_w + gap3;
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.5 * line_alpha);
                cr.move_to(renderer.snap(x), ty);
                pangocairo::functions::show_layout(cr, &divider);
                let _ = div_w;
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
            }
        }
        cr.restore().unwrap();

        let top = dy.max(0.0);
        let bounds = Rect { x: x0 - FILLET, y: top, w: x1 - x0 + 2.0 * FILLET, h: y1 - top };
        // opaque: the box minus its curved corners (a T shape)
        let opaque = [
            (y1 - r > top).then(|| Rect { x: x0, y: top, w: x1 - x0, h: y1 - r - top }),
            (x1 - x0 > 2.0 * r).then(|| Rect { x: x0 + r, y: (y1 - r).max(top), w: x1 - x0 - 2.0 * r, h: r.min(y1 - top) }),
        ];
        Frame { bounds: Some(bounds), opaque }
    }

    /// Hit test for clicks: the song controls only make sense on the line.
    pub fn accepts_player_clicks(&self) -> bool {
        self.song.is_some() && self.auth.is_none()
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
}

/// Pango setup shared by all drawing: fonts, hinting for the fractional
/// scale, one pango context and a cache of shaped layouts (shaping the
/// same clock string sixty times a second would cost more than drawing).
pub struct Renderer {
    scale: f64,
    text: pango::FontDescription,
    caption: pango::FontDescription,
    ctx: pango::Context,
    cache: RefCell<HashMap<(Font, String), pango::Layout>>,
}

impl Renderer {
    pub fn new(scale: f64) -> Self {
        let mut text = pango::FontDescription::from_string(FONT);
        text.set_absolute_size(FONT_PX * pango::SCALE as f64);
        let mut caption = pango::FontDescription::from_string(FONT);
        caption.set_absolute_size(CAPTION_PX * pango::SCALE as f64);
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
        Renderer { scale, text, caption, ctx, cache: RefCell::new(HashMap::new()) }
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

    /// A layout for `text`, glyph positions rounded to output pixels. (Not
    /// switched to fractional positions while moving: unhinted metrics give
    /// a text about 1 px narrower and shorter, and the swap at the end of a
    /// move read as the text changing size.)
    pub fn layout(&self, _cr: &Context, text: &str, font: Font) -> pango::Layout {
        let key = (font, text.to_string());
        let mut cache = self.cache.borrow_mut();
        if let Some(l) = cache.get(&key) {
            return l.clone();
        }
        if cache.len() > 64 {
            cache.clear();
        }
        let layout = pango::Layout::new(&self.ctx);
        layout.set_font_description(Some(match font {
            Font::Text => &self.text,
            Font::Caption => &self.caption,
        }));
        layout.set_text(text);
        cache.insert(key, layout.clone());
        layout
    }
}
