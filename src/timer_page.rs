//! The timer page, laid out as Vorssaint's: three modes named in the title
//! row (Timer, Pomodoro, Stopwatch, the chosen one underlined in orange)
//! with an orange Start capsule; under them a ruler of orange ticks, one per
//! minute up to three hours, that the pointer drags, the wheel or two
//! fingers turn, or a click jumps along, beside the chosen time in large
//! digits. The Pomodoro adds a row of its breaks and sessions. While a
//! session runs, the title row names its phase and the body holds iOS's
//! round Cancel and Pause buttons beside the large reading.
//!
//! The island owns the running session; this page holds the choices
//! (kept while the island runs) and reports what the person asked for.
use std::collections::HashMap;

use cairo::Context;

use crate::anim::{Spring, Time};
use crate::island::{Font, Renderer};
use crate::menu_style;

pub const WIDTH: f64 = 480.0;
const RAIL: f64 = 24.0;
const INNER: f64 = WIDTH - 2.0 * RAIL;
const HEADER_TOP: f64 = 12.0;
const HEADER_H: f64 = 36.0;
const PAGE_TOP: f64 = HEADER_TOP + HEADER_H + 12.0;
const BOTTOM: f64 = 18.0;
/// the ruler: labels on top, ticks, the pointer under them
const RULER_H: f64 = 80.0;
/// room for the chosen time beside the ruler
const CLOCK_W: f64 = 124.0;
const RULER_W: f64 = INNER - CLOCK_W - 12.0;
/// px per minute along the ruler
pub const SPACING: f64 = 14.0;
pub const MAX_MINUTES: u32 = 180;
/// the Pomodoro's breaks and sessions
const SETTINGS_Y: f64 = PAGE_TOP + RULER_H + 10.0;
const SETTINGS_H: f64 = 38.0;
/// a running session: the round buttons and the reading
const ACTIVE_H: f64 = 72.0;
const ROUND: f64 = 52.0;
/// the Start capsule
const START_H: f64 = menu_style::SMALL_BUTTON_H;

type Rect = (f64, f64, f64, f64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Timer,
    Pomodoro,
    Stopwatch,
}

pub const MODES: [Mode; 3] = [Mode::Timer, Mode::Pomodoro, Mode::Stopwatch];

impl Mode {
    fn title(self) -> &'static str {
        match self {
            Mode::Timer => "Таймер",
            Mode::Pomodoro => "Помодоро",
            Mode::Stopwatch => "Секундомер",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn title(self) -> &'static str {
        match self {
            Phase::Focus => "Фокус",
            Phase::ShortBreak => "Перерыв",
            Phase::LongBreak => "Длинный перерыв",
        }
    }
}

/// A Pomodoro cycle's plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pomodoro {
    pub focus: u32,
    pub short: u32,
    pub long: u32,
    /// a long break after every this many sessions
    pub interval: u32,
    pub sessions: u32,
}

impl Default for Pomodoro {
    fn default() -> Self {
        Pomodoro { focus: 25, short: 5, long: 15, interval: 4, sessions: 4 }
    }
}

impl Pomodoro {
    pub fn minutes(&self, phase: Phase) -> u32 {
        match phase {
            Phase::Focus => self.focus,
            Phase::ShortBreak => self.short,
            Phase::LongBreak => self.long,
        }
    }

    /// What follows `phase` of session `n`: the next phase and its session,
    /// or None when the last session's focus ends the cycle.
    pub fn after(&self, phase: Phase, n: u32) -> Option<(Phase, u32)> {
        match phase {
            Phase::Focus if n >= self.sessions => None,
            Phase::Focus if n % self.interval.max(1) == 0 => Some((Phase::LongBreak, n)),
            Phase::Focus => Some((Phase::ShortBreak, n)),
            _ => Some((Phase::Focus, n + 1)),
        }
    }
}

/// What a running session is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Timer,
    Stopwatch,
    Pomodoro { phase: Phase, session: u32, total: u32 },
}

/// The session as the page shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Session {
    /// nothing runs: the modes, the ruler and Start
    Setup,
    /// a countdown or the stopwatch, running or paused; `reading` is the
    /// time left (or gone, for the stopwatch)
    Active { kind: Kind, reading: f64, running: bool, urgent: bool },
    /// a Pomodoro phase ended and the next waits to be started
    Next { done: Phase, next: Phase, session: u32, total: u32, minutes: u32 },
    /// the Pomodoro's last focus ended
    Finished { total: u32 },
}

/// What the person asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// a countdown of so many minutes
    Timer(u32),
    Pomodoro(Pomodoro),
    Stopwatch,
    /// pause or resume
    Toggle,
    /// start the waiting Pomodoro phase
    Next,
    /// end the session (or the finished cycle)
    Cancel,
    /// close the page
    Close,
}

/// The Pomodoro's adjustable readouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Setting {
    Short,
    Long,
    Interval,
    Sessions,
}

const SETTINGS: [Setting; 4] = [Setting::Short, Setting::Long, Setting::Interval, Setting::Sessions];

impl Setting {
    fn title(self) -> &'static str {
        match self {
            Setting::Short => "Перерыв",
            Setting::Long => "Длинный",
            Setting::Interval => "Длинный после",
            Setting::Sessions => "Сессий",
        }
    }

    /// The values a click steps through.
    fn presets(self) -> &'static [u32] {
        match self {
            Setting::Short => &[3, 5, 10, 15],
            Setting::Long => &[10, 15, 20, 30],
            Setting::Interval => &[2, 3, 4, 6],
            Setting::Sessions => &[2, 4, 6, 8],
        }
    }

    fn range(self) -> (u32, u32) {
        match self {
            Setting::Short | Setting::Long => (1, 60),
            Setting::Interval | Setting::Sessions => (1, 24),
        }
    }

    fn get(self, p: &Pomodoro) -> u32 {
        match self {
            Setting::Short => p.short,
            Setting::Long => p.long,
            Setting::Interval => p.interval,
            Setting::Sessions => p.sessions,
        }
    }

    fn set(self, p: &mut Pomodoro, value: u32) {
        let (lo, hi) = self.range();
        let value = value.clamp(lo, hi);
        match self {
            Setting::Short => p.short = value,
            Setting::Long => p.long = value,
            Setting::Interval => p.interval = value,
            Setting::Sessions => p.sessions = value,
        }
    }

    fn text(self, p: &Pomodoro) -> String {
        match self {
            Setting::Short | Setting::Long => format!("{} мин", self.get(p)),
            Setting::Interval => format!("{}-й", self.get(p)),
            Setting::Sessions => self.get(p).to_string(),
        }
    }
}

/// A control of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Mode(Mode),
    Start,
    Close,
    Ruler,
    Setting(Setting),
    /// pause / resume, or start the next phase
    Primary,
    /// cancel the session
    Cancel,
}

struct Drag {
    x0: f64,
    value0: f64,
    moved: bool,
}

pub struct Page {
    pub mode: Mode,
    /// the countdown's minutes on the ruler
    pub minutes: u32,
    /// the Pomodoro's plan (its focus is the ruler in that mode)
    pub pomodoro: Pomodoro,
    /// the ruler's shown minute, eased toward the chosen one (and following
    /// the pointer while it drags)
    ruler: Spring,
    drag: Option<Drag>,
    /// the part of a minute two fingers have turned so far
    scroll_rest: f64,
    hover: Option<Part>,
    lit: HashMap<Part, Spring>,
    pressed: Option<(Part, Time)>,
    /// the orange line under the chosen mode: its x and width
    underline: Option<(Spring, Spring)>,
    /// where the mode words and the Start capsule were last drawn
    mode_rects: [Rect; 3],
    start_rect: Rect,
    /// when the chosen minute last changed (the pointer nods), and how many
    /// ticks the sound has not heard yet
    ticked: Time,
    ticks: u32,
    pub opened: Time,
}

impl Default for Page {
    fn default() -> Self {
        Page::new()
    }
}

impl Page {
    pub fn new() -> Self {
        let mut mode_rects = [(0.0, HEADER_TOP, 0.0, HEADER_H); 3];
        // until the first frame measures them
        let mut x = RAIL - 8.0;
        for (i, w) in [72.0, 96.0, 108.0].into_iter().enumerate() {
            mode_rects[i] = (x, HEADER_TOP, w, HEADER_H);
            x += w + 2.0;
        }
        Page {
            mode: Mode::Timer,
            minutes: 15,
            pomodoro: Pomodoro::default(),
            ruler: Spring::new(15.0, 0.22, 0.9),
            drag: None,
            scroll_rest: 0.0,
            hover: None,
            lit: HashMap::new(),
            pressed: None,
            underline: None,
            mode_rects,
            start_rect: (WIDTH - RAIL - 160.0, HEADER_TOP + (HEADER_H - START_H) / 2.0, 80.0, START_H),
            ticked: f64::NEG_INFINITY,
            ticks: 0,
            opened: f64::NEG_INFINITY,
        }
    }

    /// The page opens (again): it forgets the pointer, keeps the choices.
    pub fn open(&mut self, now: Time) {
        self.opened = now;
        self.drag = None;
        self.hover = None;
        self.pressed = None;
        self.scroll_rest = 0.0;
        for spring in self.lit.values_mut() {
            *spring = Spring::new(0.0, 0.25, 1.0);
        }
        let chosen = self.chosen() as f64;
        self.ruler = Spring::new(chosen, 0.22, 0.9);
    }

    /// The minutes the ruler holds in this mode.
    pub fn chosen(&self) -> u32 {
        match self.mode {
            Mode::Pomodoro => self.pomodoro.focus,
            _ => self.minutes,
        }
    }

    fn uses_ruler(&self) -> bool {
        self.mode != Mode::Stopwatch
    }

    /// Choose `minute` on the ruler; a change ticks.
    fn choose(&mut self, minute: f64, now: Time) -> bool {
        let minute = minute.round().clamp(1.0, MAX_MINUTES as f64) as u32;
        if minute == self.chosen() {
            return false;
        }
        match self.mode {
            Mode::Pomodoro => self.pomodoro.focus = minute,
            _ => self.minutes = minute,
        }
        if self.drag.is_none() {
            self.ruler.set(minute as f64, now);
        }
        self.ticked = now;
        self.ticks += 1;
        true
    }

    /// Did the chosen minute change since the sound last asked?
    pub fn take_ticks(&mut self) -> bool {
        std::mem::take(&mut self.ticks) > 0
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn height(&self, session: &Session) -> f64 {
        match session {
            Session::Setup if self.mode == Mode::Pomodoro => SETTINGS_Y + SETTINGS_H + BOTTOM,
            Session::Setup => PAGE_TOP + RULER_H + BOTTOM,
            _ => PAGE_TOP + ACTIVE_H + BOTTOM,
        }
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.ruler.is_moving(now)
            || self.lit.values().any(|s| s.is_moving(now))
            || self.underline.as_ref().is_some_and(|(x, w)| x.is_moving(now) || w.is_moving(now))
            || now - self.ticked < 0.4
            || self.pressed.is_some_and(|(_, t)| now - t < 0.5)
            || now - self.opened < 0.6
    }

    // --- geometry ---

    fn close_rect() -> Rect {
        let s = menu_style::CLOSE_SIZE;
        (WIDTH - RAIL - s + 4.0, HEADER_TOP + (HEADER_H - s) / 2.0, s, s)
    }

    fn ruler_rect() -> Rect {
        (RAIL, PAGE_TOP, RULER_W, RULER_H)
    }

    fn setting_rect(i: usize) -> Rect {
        let w = INNER / SETTINGS.len() as f64;
        (RAIL + i as f64 * w, SETTINGS_Y, w - 6.0, SETTINGS_H)
    }

    fn cancel_rect() -> Rect {
        (RAIL, PAGE_TOP + (ACTIVE_H - ROUND) / 2.0, ROUND, ROUND)
    }

    fn primary_rect() -> Rect {
        (RAIL + ROUND + 14.0, PAGE_TOP + (ACTIVE_H - ROUND) / 2.0, ROUND, ROUND)
    }

    /// The control under the page-relative point.
    pub fn hit(&self, session: &Session, x: f64, y: f64) -> Option<Part> {
        let inside = |r: Rect, pad: f64| x >= r.0 - pad && x <= r.0 + r.2 + pad && y >= r.1 - pad && y <= r.1 + r.3 + pad;
        if inside(Self::close_rect(), 4.0) {
            return Some(Part::Close);
        }
        match session {
            Session::Setup => {
                if let Some(i) = (0..3).find(|i| inside(self.mode_rects[*i], 0.0)) {
                    return Some(Part::Mode(MODES[i]));
                }
                if inside(self.start_rect, 3.0) {
                    return Some(Part::Start);
                }
                if self.uses_ruler() && inside(Self::ruler_rect(), 0.0) {
                    return Some(Part::Ruler);
                }
                if self.mode == Mode::Pomodoro {
                    if let Some(i) = (0..SETTINGS.len()).find(|i| inside(Self::setting_rect(*i), 0.0)) {
                        return Some(Part::Setting(SETTINGS[i]));
                    }
                }
                None
            }
            Session::Active { .. } | Session::Next { .. } => {
                if inside(Self::primary_rect(), 2.0) {
                    return Some(Part::Primary);
                }
                inside(Self::cancel_rect(), 2.0).then_some(Part::Cancel)
            }
            Session::Finished { .. } => inside(Self::cancel_rect(), 2.0).then_some(Part::Cancel),
        }
    }

    /// A part's centre, page-relative (for the island's tests).
    #[cfg(test)]
    pub fn centre(&self, part: Part) -> (f64, f64) {
        let r = match part {
            Part::Close => Self::close_rect(),
            Part::Start => self.start_rect,
            Part::Ruler => Self::ruler_rect(),
            Part::Primary => Self::primary_rect(),
            Part::Cancel => Self::cancel_rect(),
            Part::Mode(mode) => self.mode_rects[MODES.iter().position(|m| *m == mode).unwrap()],
            Part::Setting(setting) => Self::setting_rect(SETTINGS.iter().position(|s| *s == setting).unwrap()),
        };
        (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
    }

    // --- input ---

    /// The pointer moved over the page (or dragged the ruler).
    pub fn pointer(&mut self, session: &Session, x: f64, y: f64, now: Time) -> bool {
        if self.drag.is_some() {
            return self.drag_to(x, now);
        }
        let hover = self.hit(session, x, y);
        if hover == self.hover {
            return false;
        }
        self.hover = hover;
        self.light(now);
        true
    }

    pub fn leave(&mut self, now: Time) -> bool {
        if self.hover.is_none() || self.drag.is_some() {
            return false;
        }
        self.hover = None;
        self.light(now);
        true
    }

    fn light(&mut self, now: Time) {
        let hover = self.hover;
        if let Some(part) = hover {
            self.lit.entry(part).or_insert_with(|| Spring::new(0.0, 0.25, 1.0));
        }
        for (part, spring) in self.lit.iter_mut() {
            spring.set(if Some(*part) == hover { 1.0 } else { 0.0 }, now);
        }
    }

    fn lit(&self, part: Part, now: Time) -> f64 {
        self.lit.get(&part).map_or(0.0, |s| s.value(now).clamp(0.0, 1.0))
    }

    /// A press: the ruler starts a drag; buttons answer with a command.
    pub fn press(&mut self, session: &Session, x: f64, y: f64, now: Time) -> Option<Command> {
        let part = self.hit(session, x, y)?;
        self.pressed = Some((part, now));
        match part {
            Part::Ruler => {
                self.drag = Some(Drag { x0: x, value0: self.chosen() as f64, moved: false });
                None
            }
            Part::Mode(mode) => {
                self.select_mode(mode, now);
                None
            }
            Part::Setting(setting) => {
                let current = setting.get(&self.pomodoro);
                let presets = setting.presets();
                let next = presets.iter().copied().find(|v| *v > current).unwrap_or(presets[0]);
                setting.set(&mut self.pomodoro, next);
                self.ticked = now;
                None
            }
            Part::Start => Some(self.start()),
            Part::Close => Some(Command::Close),
            Part::Primary => Some(match session {
                Session::Next { .. } => Command::Next,
                _ => Command::Toggle,
            }),
            Part::Cancel => Some(Command::Cancel),
        }
    }

    fn start(&self) -> Command {
        match self.mode {
            Mode::Timer => Command::Timer(self.minutes),
            Mode::Pomodoro => Command::Pomodoro(self.pomodoro),
            Mode::Stopwatch => Command::Stopwatch,
        }
    }

    fn select_mode(&mut self, mode: Mode, now: Time) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        // the ruler slides to this mode's minutes
        self.ruler.set(self.chosen() as f64, now);
    }

    /// The held ruler follows the pointer: dragging left brings later
    /// minutes under the pointer, as a tape would.
    pub fn drag_to(&mut self, x: f64, now: Time) -> bool {
        let Some(drag) = &mut self.drag else { return false };
        if (x - drag.x0).abs() > 3.0 {
            drag.moved = true;
        }
        if !drag.moved {
            return false;
        }
        let value = (drag.value0 - (x - drag.x0) / SPACING).clamp(1.0, MAX_MINUTES as f64);
        self.ruler.set(value, now);
        self.choose(value, now);
        true
    }

    /// Let go: a click without a drag jumps to the minute under it; either
    /// way the ruler settles on a whole minute.
    pub fn release(&mut self, x: f64, now: Time) -> bool {
        let Some(drag) = self.drag.take() else { return false };
        if !drag.moved {
            let (rx, _, rw, _) = Self::ruler_rect();
            let offset = x - (rx + rw / 2.0);
            self.choose(self.chosen() as f64 + offset / SPACING, now);
        }
        self.ruler.set(self.chosen() as f64, now);
        true
    }

    /// The wheel or two fingers over the page: a notch is a minute; a
    /// touchpad turns the ruler by its own spacing. Over a Pomodoro
    /// readout it changes that readout instead.
    pub fn scroll(&mut self, session: &Session, x: f64, y: f64, px: f64, discrete: bool, now: Time) -> bool {
        if *session != Session::Setup {
            return false;
        }
        let steps = if discrete {
            px.signum()
        } else {
            self.scroll_rest += px / SPACING;
            let whole = self.scroll_rest.trunc();
            self.scroll_rest -= whole;
            whole
        };
        if let Some(Part::Setting(setting)) = self.hit(session, x, y) {
            if steps != 0.0 {
                let value = setting.get(&self.pomodoro) as f64 + steps;
                setting.set(&mut self.pomodoro, value.max(1.0) as u32);
                self.ticked = now;
                self.ticks += 1;
            }
            return true;
        }
        if !self.uses_ruler() {
            return false;
        }
        if steps != 0.0 {
            self.choose(self.chosen() as f64 + steps, now);
        }
        true
    }

    /// Arrows move a minute, Tab changes the mode, Return starts (or
    /// pauses, or starts the next phase).
    pub fn key(&mut self, session: &Session, key: &str, now: Time) -> Option<Command> {
        match (session, key) {
            (Session::Setup, "Left" | "Down") if self.uses_ruler() => {
                self.choose(self.chosen() as f64 - 1.0, now);
                None
            }
            (Session::Setup, "Right" | "Up") if self.uses_ruler() => {
                self.choose(self.chosen() as f64 + 1.0, now);
                None
            }
            (Session::Setup, "Tab") => {
                let i = MODES.iter().position(|m| *m == self.mode).unwrap_or(0);
                self.select_mode(MODES[(i + 1) % MODES.len()], now);
                None
            }
            (Session::Setup, "Return") => Some(self.start()),
            (Session::Active { .. }, "Return" | "space") => Some(Command::Toggle),
            (Session::Next { .. }, "Return") => Some(Command::Next),
            (Session::Finished { .. }, "Return") => Some(Command::Cancel),
            _ => None,
        }
    }

    // --- drawing ---

    pub fn draw(&mut self, cr: &Context, renderer: &Renderer, x: f64, y: f64, session: &Session, alpha: f64, now: Time) {
        if alpha <= 0.002 {
            return;
        }
        // the rows arrive in a wave, as the Control Center's do
        let opened = self.opened;
        let wave = |row: f64| {
            let k = crate::island::smoothstep((now - opened - 0.04 * row) / 0.3);
            (alpha * k, 8.0 * (1.0 - k))
        };
        let (a0, r0) = wave(0.0);
        menu_style::close(cr, x + Self::close_rect().0, y + Self::close_rect().1 + r0, menu_style::CLOSE_SIZE,
            self.lit(Part::Close, now), a0);
        match *session {
            Session::Setup => {
                self.draw_modes(cr, renderer, x, y + r0, a0, now);
                self.draw_start(cr, renderer, x, y + r0, a0, now);
                let (a1, r1) = wave(1.0);
                if self.uses_ruler() {
                    self.draw_ruler(cr, renderer, x, y + r1, a1, now);
                } else {
                    let (rx, ry, rw, rh) = Self::ruler_rect();
                    menu_style::text(cr, renderer, "Отсчёт с нуля. Пауза и отмена —", x + rx, y + r1 + ry + rh / 2.0 - 19.0,
                        rw, Font::Subhead, (1.0, 1.0, 1.0), a1 * menu_style::SECONDARY);
                    menu_style::text(cr, renderer, "в любой момент.", x + rx, y + r1 + ry + rh / 2.0 + 1.0,
                        rw, Font::Subhead, (1.0, 1.0, 1.0), a1 * menu_style::SECONDARY);
                }
                let chosen = if self.uses_ruler() { clock(self.chosen() as f64 * 60.0) } else { "0:00".to_string() };
                let (rx, ry, rw, _) = Self::ruler_rect();
                self.draw_reading(cr, renderer, &chosen, Font::ClockSmall, x + rx + rw + 12.0, y + r1 + ry + 45.0,
                    CLOCK_W, 1.0, menu_style::ORANGE, a1, &format!("timer-page-chosen{:.4}", self.opened));
                if self.mode == Mode::Pomodoro {
                    let (a2, r2) = wave(2.0);
                    for (i, setting) in SETTINGS.iter().enumerate() {
                        self.draw_setting(cr, renderer, x, y + r2, i, *setting, a2, now);
                    }
                }
            }
            Session::Active { kind, reading, running, urgent } => {
                let (title, place) = match kind {
                    Kind::Timer => ("Таймер".to_string(), None),
                    Kind::Stopwatch => ("Секундомер".to_string(), None),
                    Kind::Pomodoro { phase, session, total } => (phase.title().to_string(), Some(format!("{session} из {total}"))),
                };
                let title = if running { title } else { format!("{title} · пауза") };
                self.draw_phase(cr, renderer, x, y + r0, &title, place.as_deref(), a0);
                let (a1, r1) = wave(1.0);
                self.draw_round(cr, x, y + r1, Part::Cancel, a1, now);
                self.draw_round(cr, x, y + r1, Part::Primary, a1, now);
                round_symbol(cr, x + Self::primary_rect().0, y + r1 + Self::primary_rect().1,
                    if running { "pause" } else { "play" }, a1);
                // the last minute of a countdown turns red
                let colour = if urgent { menu_style::RED } else { menu_style::ORANGE };
                self.draw_big(cr, renderer, &clock(reading), x, y + r1, colour, a1, kind);
            }
            Session::Next { done, next, session, total, minutes } => {
                let place = format!("{session} из {total}");
                self.draw_phase(cr, renderer, x, y + r0, &format!("{} окончен", done.title()), Some(&place), a0);
                let (a1, r1) = wave(1.0);
                self.draw_round(cr, x, y + r1, Part::Cancel, a1, now);
                self.draw_round(cr, x, y + r1, Part::Primary, a1, now);
                round_symbol(cr, x + Self::primary_rect().0, y + r1 + Self::primary_rect().1, "play", a1);
                let (px, py, _, _) = Self::primary_rect();
                menu_style::text(cr, renderer, &format!("Начать: {}", next.title().to_lowercase()), x + px + ROUND + 14.0,
                    y + r1 + py + ROUND / 2.0 - 9.0, 150.0, Font::Subhead, (1.0, 1.0, 1.0), a1 * menu_style::SECONDARY);
                self.draw_big(cr, renderer, &clock(minutes as f64 * 60.0), x, y + r1, menu_style::ORANGE, a1 * 0.5,
                    Kind::Pomodoro { phase: next, session, total });
            }
            Session::Finished { total } => {
                self.draw_phase(cr, renderer, x, y + r0, "Помодоро завершён", Some(&format!("{total} из {total}")), a0);
                let (a1, r1) = wave(1.0);
                self.draw_round(cr, x, y + r1, Part::Cancel, a1, now);
                let (cx, cy, _, _) = Self::cancel_rect();
                menu_style::text(cr, renderer, "Готово", x + cx + ROUND + 14.0, y + r1 + cy + ROUND / 2.0 - 9.0, 150.0,
                    Font::Subhead, (1.0, 1.0, 1.0), a1 * menu_style::SECONDARY);
            }
        }
    }

    /// The three modes, the chosen one white over an orange line that
    /// slides between them.
    fn draw_modes(&mut self, cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, now: Time) {
        let mut mx = RAIL - 8.0;
        let line = menu_style::line_height(cr, renderer, Font::Label);
        for (i, mode) in MODES.iter().enumerate() {
            let w = renderer.layout(cr, mode.title(), Font::Label).pixel_extents().1.width() as f64 + 16.0;
            self.mode_rects[i] = (mx, HEADER_TOP, w, HEADER_H);
            let selected = *mode == self.mode;
            let lit = self.lit(Part::Mode(*mode), now);
            let a = if selected { 1.0 } else { 0.5 + 0.3 * lit };
            menu_style::text(cr, renderer, mode.title(), x + mx + 8.0, y + HEADER_TOP + (HEADER_H - line) / 2.0 - 1.0, w,
                Font::Label, (1.0, 1.0, 1.0), alpha * a);
            mx += w + 2.0;
        }
        let i = MODES.iter().position(|m| *m == self.mode).unwrap_or(0);
        let (ux, _, uw, _) = self.mode_rects[i];
        let (tx, tw) = (ux + 8.0, uw - 16.0);
        let (sx, sw) = self.underline.get_or_insert_with(|| (Spring::new(tx, 0.3, 0.8), Spring::new(tw, 0.3, 0.8)));
        sx.set(tx, now);
        sw.set(tw, now);
        let (lx, lw) = (sx.value(now), sw.value(now));
        let o = menu_style::ORANGE;
        menu_style::rounded(cr, x + lx, y + HEADER_TOP + HEADER_H / 2.0 + line / 2.0 + 3.0, lw, 2.0, 1.0);
        cr.set_source_rgba(o.0, o.1, o.2, alpha);
        let _ = cr.fill();
    }

    /// Start: the warm (orange-tinted) button.
    fn draw_start(&mut self, cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, now: Time) {
        let tw = renderer.layout(cr, "Старт", Font::Label).pixel_extents().1.width() as f64;
        let w = tw + 28.0;
        let close = Self::close_rect();
        self.start_rect = (close.0 - 10.0 - w, HEADER_TOP + (HEADER_H - START_H) / 2.0, w, START_H);
        let (sx, sy, sw, sh) = self.start_rect;
        let lit = self.lit(Part::Start, now);
        let press = menu_style::press_scale(self.pressed.filter(|(p, _)| *p == Part::Start).map_or(f64::INFINITY, |(_, t)| now - t));
        menu_style::transformed(cr, x + sx + sw / 2.0, y + sy + sh / 2.0, press, 0.0, || {
            menu_style::button(cr, renderer, "Старт", (x + sx, y + sy, sw, sh), menu_style::ButtonKind::Warm, lit, alpha);
        });
    }

    /// The ruler: a tick per minute, longer-lit up to the chosen one, a
    /// label every five, fading at both ends, the pointer under the middle.
    fn draw_ruler(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, now: Time) {
        let (rx, ry, rw, rh) = Self::ruler_rect();
        let (rx, ry) = (x + rx, y + ry);
        let mid = rx + rw / 2.0;
        let shown = self.ruler.value(now);
        let chosen = self.chosen();
        let edge = (rw * 0.12).max(1.0);
        let o = menu_style::ORANGE;
        let held = self.lit(Part::Ruler, now);
        cr.save().unwrap();
        cr.rectangle(rx, ry - 2.0, rw, rh + 4.0);
        cr.clip();
        let first = ((shown - rw / 2.0 / SPACING).floor() as i64 - 2).max(1) as u32;
        let last = ((shown + rw / 2.0 / SPACING).ceil() as i64 + 2).min(MAX_MINUTES as i64) as u32;
        for minute in first..=last {
            let tx = mid + (minute as f64 - shown) * SPACING;
            let fade = ((tx - rx).min(rx + rw - tx) / edge).clamp(0.0, 1.0);
            if fade <= 0.0 {
                continue;
            }
            let strength = if minute <= chosen { 1.0 } else { 0.35 + 0.1 * held };
            let a = alpha * fade * strength;
            menu_style::rounded(cr, tx - 2.0, ry + 26.0, 4.0, 38.0, 2.0);
            cr.set_source_rgba(o.0, o.1, o.2, a);
            let _ = cr.fill();
            if minute % 5 == 0 || minute == 1 {
                let label = ruler_label(minute);
                let layout = renderer.layout(cr, &label, Font::Label);
                let (_, ext) = layout.pixel_extents();
                cr.set_source_rgba(o.0, o.1, o.2, a);
                cr.move_to(renderer.snap(tx - ext.width() as f64 / 2.0 - ext.x() as f64), renderer.snap(ry - ext.y() as f64 + 2.0));
                pangocairo::functions::show_layout(cr, &layout);
                cr.new_path();
            }
        }
        cr.restore().unwrap();
        // the pointer nods as each minute passes under it
        let nod = 1.0 + 0.35 * (-(now - self.ticked) / 0.09).exp() * if now >= self.ticked { 1.0 } else { 0.0 };
        let (px, py) = (mid, ry + 70.0);
        cr.new_path();
        cr.move_to(px, py);
        cr.line_to(px + 6.0 * nod, py + 10.0 * nod);
        cr.line_to(px - 6.0 * nod, py + 10.0 * nod);
        cr.close_path();
        cr.set_source_rgba(o.0, o.1, o.2, alpha);
        let _ = cr.fill();
    }

    /// A Pomodoro readout: its name small and dim, its value in orange.
    #[allow(clippy::too_many_arguments)]
    fn draw_setting(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, i: usize, setting: Setting, alpha: f64, now: Time) {
        let (sx, sy, sw, sh) = Self::setting_rect(i);
        let lit = self.lit(Part::Setting(setting), now);
        if lit > 0.0 {
            menu_style::rounded(cr, x + sx - 6.0, y + sy, sw + 6.0, sh, 8.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.07 * lit * alpha);
            let _ = cr.fill();
        }
        menu_style::text(cr, renderer, setting.title(), x + sx, y + sy + 2.0, sw - 4.0, Font::Tiny, (1.0, 1.0, 1.0),
            alpha * menu_style::SECONDARY);
        let value = setting.text(&self.pomodoro);
        let o = menu_style::ORANGE;
        menu_style::text(cr, renderer, &value, x + sx, y + sy + 17.0, sw - 16.0, Font::Label, o, alpha);
        let vw = renderer.layout(cr, &value, Font::Label).pixel_extents().1.width() as f64;
        menu_style::chevron(cr, x + sx + vw + 8.0, y + sy + 26.0, 3.5, std::f64::consts::FRAC_PI_2, alpha * menu_style::SECONDARY);
    }

    /// The running session's title: its phase in orange, its place beside.
    #[allow(clippy::too_many_arguments)]
    fn draw_phase(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, title: &str, place: Option<&str>, alpha: f64) {
        let line = menu_style::line_height(cr, renderer, Font::Headline);
        let ty = y + HEADER_TOP + (HEADER_H - line) / 2.0;
        menu_style::text(cr, renderer, title, x + RAIL, ty, 300.0, Font::Headline, menu_style::ORANGE, alpha);
        if let Some(place) = place {
            let w = renderer.layout(cr, title, Font::Headline).pixel_extents().1.width() as f64;
            let sub = menu_style::line_height(cr, renderer, Font::Subhead);
            menu_style::text(cr, renderer, place, x + RAIL + w + 8.0, ty + (line - sub) / 2.0 + 1.0, 120.0, Font::Subhead,
                (1.0, 1.0, 1.0), alpha * menu_style::SECONDARY);
        }
    }

    /// iOS's round buttons: Cancel grey, the primary one orange.
    fn draw_round(&self, cr: &Context, x: f64, y: f64, part: Part, alpha: f64, now: Time) {
        let (bx, by, bw, _) = if part == Part::Cancel { Self::cancel_rect() } else { Self::primary_rect() };
        let (cx, cy) = (x + bx + bw / 2.0, y + by + bw / 2.0);
        let lit = self.lit(part, now);
        let press = menu_style::press_scale(self.pressed.filter(|(p, _)| *p == part).map_or(f64::INFINITY, |(_, t)| now - t));
        menu_style::transformed(cr, cx, cy, press * (1.0 + 0.03 * lit), 0.0, || {
            cr.new_path();
            cr.arc(cx, cy, bw / 2.0, 0.0, std::f64::consts::TAU);
            if part == Part::Cancel {
                cr.set_source_rgba(1.0, 1.0, 1.0, alpha * (0.14 + 0.06 * lit));
                let _ = cr.fill();
                // a cross
                cr.set_line_width(2.4);
                cr.set_line_cap(cairo::LineCap::Round);
                cr.set_source_rgba(1.0, 1.0, 1.0, alpha * 0.9);
                for (dx, dy) in [(1.0, 1.0), (1.0, -1.0)] {
                    cr.move_to(cx - 7.0 * dx, cy - 7.0 * dy);
                    cr.line_to(cx + 7.0 * dx, cy + 7.0 * dy);
                }
                let _ = cr.stroke();
            } else {
                let o = menu_style::ORANGE;
                cr.set_source_rgba(o.0, o.1, o.2, alpha * (0.28 + 0.08 * lit));
                let _ = cr.fill();
            }
        });
    }

    /// The large reading, right-aligned in the body.
    #[allow(clippy::too_many_arguments)]
    fn draw_big(&self, cr: &Context, renderer: &Renderer, text: &str, x: f64, y: f64, colour: (f64, f64, f64), alpha: f64, kind: Kind) {
        let right = x + WIDTH - RAIL;
        // the widest the reading gets, so ticking digits never move it
        let reserved: String = text.chars().map(|c| if c.is_ascii_digit() { '8' } else { c }).collect();
        let w = renderer.layout(cr, &reserved, Font::Clock).pixel_extents().1.width() as f64;
        let key = format!("timer-page-big{:.4}{}", self.opened, matches!(kind, Kind::Stopwatch));
        self.draw_reading(cr, renderer, text, Font::Clock, right - w, y + PAGE_TOP + ACTIVE_H / 2.0, w, 0.0, colour, alpha, &key);
    }

    /// Digits through the rolling text motion, vertically centred on `cy`.
    #[allow(clippy::too_many_arguments)]
    fn draw_reading(&self, cr: &Context, renderer: &Renderer, text: &str, font: Font, x: f64, cy: f64, w: f64, align: f64,
        colour: (f64, f64, f64), alpha: f64, key: &str) {
        let layout = renderer.layout(cr, "0", font);
        let (_, ext) = layout.pixel_extents();
        let ty = renderer.snap(cy - ext.height() as f64 / 2.0 - ext.y() as f64);
        crate::motion::text(cr, renderer, key, text, x, ty, w, align, font, colour, alpha);
    }
}

/// A play or pause symbol on a round button whose box starts at (bx, by).
fn round_symbol(cr: &Context, bx: f64, by: f64, name: &str, alpha: f64) {
    let (cx, cy) = (bx + ROUND / 2.0, by + ROUND / 2.0);
    let o = menu_style::ORANGE;
    cr.new_path();
    cr.set_source_rgba(o.0, o.1, o.2, alpha);
    if name == "pause" {
        for dx in [-5.5, 2.5] {
            menu_style::rounded(cr, cx + dx, cy - 9.0, 4.0, 18.0, 1.6);
            let _ = cr.fill();
        }
    } else {
        cr.move_to(cx - 5.5, cy - 10.0);
        cr.line_to(cx + 10.0, cy);
        cr.line_to(cx - 5.5, cy + 10.0);
        cr.close_path();
        let _ = cr.fill();
    }
}

/// The ruler's label for a minute: plain minutes up to an hour, then hours
/// ("1 ч") and hours with minutes ("1:05").
fn ruler_label(minute: u32) -> String {
    match (minute / 60, minute % 60) {
        (0, m) => m.to_string(),
        (h, 0) => format!("{h} ч"),
        (h, m) => format!("{h}:{m:02}"),
    }
}

/// A clock reading: m:ss, or h:mm:ss from an hour on.
pub fn clock(seconds: f64) -> String {
    let s = seconds.ceil().max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pomodoro_alternates_focus_and_breaks_and_ends_after_its_sessions() {
        let p = Pomodoro { focus: 25, short: 5, long: 15, interval: 2, sessions: 3 };
        assert_eq!(p.after(Phase::Focus, 1), Some((Phase::ShortBreak, 1)));
        assert_eq!(p.after(Phase::ShortBreak, 1), Some((Phase::Focus, 2)));
        assert_eq!(p.after(Phase::Focus, 2), Some((Phase::LongBreak, 2)));
        assert_eq!(p.after(Phase::LongBreak, 2), Some((Phase::Focus, 3)));
        assert_eq!(p.after(Phase::Focus, 3), None);
    }

    #[test]
    fn the_ruler_drags_like_a_tape_ticks_per_minute_and_a_click_jumps() {
        let mut page = Page::new();
        page.open(0.0);
        let (rx, ry, rw, rh) = Page::ruler_rect();
        let (mx, my) = (rx + rw / 2.0, ry + rh / 2.0);
        assert_eq!(page.press(&Session::Setup, mx, my, 1.0), None);
        // dragging left by three minutes' spacing brings later minutes in
        page.drag_to(mx - 3.0 * SPACING, 1.1);
        assert_eq!(page.minutes, 18);
        assert!(page.take_ticks());
        assert!(!page.take_ticks());
        page.release(mx - 3.0 * SPACING, 1.2);
        // a click right of the middle jumps that many minutes on
        page.press(&Session::Setup, mx + 2.0 * SPACING, my, 2.0);
        page.release(mx + 2.0 * SPACING, 2.0);
        assert_eq!(page.minutes, 20);
        // never past three hours or under a minute
        for _ in 0..400 { page.key(&Session::Setup, "Right", 3.0); }
        assert_eq!(page.minutes, MAX_MINUTES);
        for _ in 0..400 { page.key(&Session::Setup, "Left", 3.0); }
        assert_eq!(page.minutes, 1);
    }

    #[test]
    fn the_wheel_turns_a_minute_a_notch_and_a_touchpad_by_its_spacing() {
        let mut page = Page::new();
        let (rx, ry, rw, rh) = Page::ruler_rect();
        let (mx, my) = (rx + rw / 2.0, ry + rh / 2.0);
        assert!(page.scroll(&Session::Setup, mx, my, 40.0, true, 1.0));
        assert_eq!(page.minutes, 16);
        page.scroll(&Session::Setup, mx, my, SPACING * 0.6, false, 1.1);
        assert_eq!(page.minutes, 16, "part of a minute waits for the rest");
        page.scroll(&Session::Setup, mx, my, SPACING * 0.6, false, 1.2);
        assert_eq!(page.minutes, 17);
        assert!(!page.scroll(&Session::Active { kind: Kind::Timer, reading: 60.0, running: true, urgent: false },
            mx, my, 40.0, true, 1.3));
    }

    #[test]
    fn modes_choose_what_start_starts_and_the_pomodoro_keeps_its_own_minutes() {
        let mut page = Page::new();
        assert_eq!(page.key(&Session::Setup, "Return", 0.0), Some(Command::Timer(15)));
        page.key(&Session::Setup, "Tab", 0.0);
        assert_eq!(page.mode, Mode::Pomodoro);
        page.key(&Session::Setup, "Right", 0.0);
        assert_eq!(page.pomodoro.focus, 26);
        assert_eq!(page.minutes, 15);
        assert!(matches!(page.key(&Session::Setup, "Return", 0.0), Some(Command::Pomodoro(p)) if p.focus == 26));
        page.key(&Session::Setup, "Tab", 0.0);
        assert_eq!(page.key(&Session::Setup, "Return", 0.0), Some(Command::Stopwatch));
        assert!(page.height(&Session::Setup) < Page { mode: Mode::Pomodoro, ..Page::new() }.height(&Session::Setup));
    }

    #[test]
    fn a_running_session_offers_cancel_and_pause_and_a_waiting_phase_offers_its_start() {
        let mut page = Page::new();
        let active = Session::Active { kind: Kind::Timer, reading: 90.0, running: true, urgent: false };
        let centre = |r: Rect| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        let (x, y) = centre(Page::primary_rect());
        assert_eq!(page.press(&active, x, y, 1.0), Some(Command::Toggle));
        let next = Session::Next { done: Phase::Focus, next: Phase::ShortBreak, session: 1, total: 4, minutes: 5 };
        assert_eq!(page.press(&next, x, y, 2.0), Some(Command::Next));
        let (x, y) = centre(Page::cancel_rect());
        assert_eq!(page.press(&active, x, y, 3.0), Some(Command::Cancel));
        let (x, y) = centre(Page::close_rect());
        assert_eq!(page.press(&Session::Setup, x, y, 4.0), Some(Command::Close));
    }

    #[test]
    fn labels_read_minutes_then_hours() {
        assert_eq!(ruler_label(45), "45");
        assert_eq!(ruler_label(60), "1 ч");
        assert_eq!(ruler_label(95), "1:35");
        assert_eq!(clock(150.0 * 60.0), "2:30:00");
    }
}
