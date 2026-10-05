//! The calendar sheet: a right click on the clock unfolds the month, as
//! in Notification Center's calendar. Today is a red disc; a click selects
//! a day and the footer says which day it is and how far away; ‹ › (or the
//! wheel, or PageUp / PageDown) turn the months, and the grid slides the
//! way the months go. Arrow keys move the selection by a day or a week,
//! crossing into the next month when they run off this one; Home and
//! «Сегодня» come back to today.

use std::collections::HashMap;

use cairo::Context;

use crate::anim::{Spring, Time, min_jerk};
use crate::island::{Font, Renderer, smoothstep};
use crate::menu_style::{self, ButtonKind, RAIL};

pub const WIDTH: f64 = 372.0;
const WEEKDAYS_MID: f64 = 82.0;
const GRID_TOP: f64 = 96.0;
const ROW_H: f64 = 40.0;
const DAY_R: f64 = 17.0;
const FOOTER_H: f64 = 34.0;
const SWITCH: Time = 0.36;
/// the wheel turns a month per notch (or per this much touchpad travel)
const WHEEL_STEP: f64 = 52.0;
const TODAY_W: f64 = 92.0;

const MONTHS: [&str; 12] = ["Январь", "Февраль", "Март", "Апрель", "Май", "Июнь", "Июль", "Август", "Сентябрь",
    "Октябрь", "Ноябрь", "Декабрь"];
const MONTHS_OF: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября",
    "октября", "ноября", "декабря"];
const WEEKDAYS: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];
const WEEKDAY_NAMES: [&str; 7] = ["Понедельник", "Вторник", "Среда", "Четверг", "Пятница", "Суббота", "Воскресенье"];

/// A civil date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// Today in local time.
    pub fn today() -> Date {
        let text = crate::island::strftime("%Y %m %d");
        let mut parts = text.split(' ').map(|p| p.parse::<i64>().unwrap_or(1));
        let (year, month, day) = (parts.next().unwrap_or(1970), parts.next().unwrap_or(1), parts.next().unwrap_or(1));
        Date { year: year as i32, month: month as u32, day: day as u32 }
    }

    /// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    fn days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    fn from_days(z: i64) -> Date {
        let z = z + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        Date { year: (yoe + era * 400 + i64::from(month <= 2)) as i32, month, day }
    }

    /// Monday 0 … Sunday 6.
    fn weekday(self) -> usize {
        (self.days() + 3).rem_euclid(7) as usize
    }

    fn plus(self, days: i64) -> Date {
        Date::from_days(self.days() + days)
    }
}

fn month_length(year: i32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A month: year and month (1..=12).
type Month = (i32, u32);

fn shift(month: Month, by: i32) -> Month {
    let index = month.0 * 12 + month.1 as i32 - 1 + by;
    (index.div_euclid(12), index.rem_euclid(12) as u32 + 1)
}

/// The rows a month's grid needs (weeks start on Monday).
fn rows(month: Month) -> usize {
    let lead = Date { year: month.0, month: month.1, day: 1 }.weekday();
    (lead + month_length(month.0, month.1) as usize).div_ceil(7)
}

/// "через 3 дня", "5 дней назад", "сегодня"…
fn relative(days: i64) -> String {
    let plural = |n: i64| {
        let n = n.abs();
        match (n % 10, n % 100) {
            (1, r) if r != 11 => "день",
            (2..=4, r) if !(12..=14).contains(&r) => "дня",
            _ => "дней",
        }
    };
    match days {
        0 => "сегодня".into(),
        1 => "завтра".into(),
        -1 => "вчера".into(),
        2.. => format!("через {days} {}", plural(days)),
        _ => format!("{} {} назад", -days, plural(days)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    Close,
    Previous,
    Next,
    Today,
    Day(Date),
}

pub struct Calendar {
    today: Date,
    shown: Month,
    selected: Date,
    /// the month the grid is leaving, which way, and since when
    switch: Option<(Month, f64, Time)>,
    opened: Time,
    hover: Option<Hit>,
    springs: HashMap<Hit, Spring>,
    wheel: f64,
    wheeled: Time,
    /// the day last picked (its disc pops)
    picked: Option<(Date, Time)>,
}

impl Calendar {
    pub fn new(now: Time) -> Calendar {
        let today = Date::today();
        Calendar::at(today, now)
    }

    pub fn at(today: Date, now: Time) -> Calendar {
        Calendar {
            today, shown: (today.year, today.month), selected: today, switch: None, opened: now, hover: None,
            springs: HashMap::new(), wheel: 0.0, wheeled: 0.0, picked: None,
        }
    }

    pub fn height(&self) -> f64 {
        self.height_for(self.shown)
    }

    fn height_for(&self, month: Month) -> f64 {
        GRID_TOP + rows(month) as f64 * ROW_H + 6.0 + FOOTER_H + 16.0
    }

    pub fn is_animating(&self, now: Time) -> bool {
        now - self.opened < 0.6
            || self.switch.is_some_and(|(_, _, since)| now - since < SWITCH)
            || self.springs.values().any(|s| s.is_moving(now))
            || self.picked.is_some_and(|(_, at)| now - at < 0.9)
    }

    /// Show `month`, sliding the grid in from the side it lies on.
    fn show(&mut self, month: Month, now: Time) {
        if month == self.shown { return; }
        let dir = if month > self.shown { 1.0 } else { -1.0 };
        self.switch = Some((self.shown, dir, now));
        self.shown = month;
        self.hover = None;
    }

    fn select(&mut self, day: Date, now: Time) {
        self.selected = day;
        self.picked = Some((day, now));
        self.show((day.year, day.month), now);
    }

    fn hit(&self, x: f64, y: f64) -> Option<Hit> {
        if !(0.0..WIDTH).contains(&x) || !(0.0..self.height()).contains(&y) { return None; }
        let near = |cx: f64, cy: f64| (x - cx).powi(2) + (y - cy).powi(2) <= 18.0 * 18.0;
        let (close_x, close_y, size) = menu_style::close_rect(WIDTH);
        let mid = close_y + size / 2.0;
        if near(close_x + size / 2.0, mid) { return Some(Hit::Close); }
        if near(close_x - 8.0 - size / 2.0, mid) { return Some(Hit::Next); }
        if near(close_x - 8.0 - size - 6.0 - size / 2.0, mid) { return Some(Hit::Previous); }
        let footer = GRID_TOP + rows(self.shown) as f64 * ROW_H + 6.0;
        if y >= footer && y < footer + FOOTER_H && x >= WIDTH - RAIL - TODAY_W && !self.at_today() {
            return Some(Hit::Today);
        }
        let col_w = (WIDTH - 2.0 * RAIL) / 7.0;
        if y >= GRID_TOP && y < footer - 6.0 && x >= RAIL && x < WIDTH - RAIL {
            let (col, row) = (((x - RAIL) / col_w) as i64, ((y - GRID_TOP) / ROW_H) as i64);
            let first = Date { year: self.shown.0, month: self.shown.1, day: 1 };
            let day = first.plus(row * 7 + col - first.weekday() as i64);
            if (day.year, day.month) == self.shown { return Some(Hit::Day(day)); }
        }
        None
    }

    fn at_today(&self) -> bool {
        self.selected == self.today && self.shown == (self.today.year, self.today.month)
    }

    pub fn pointer(&mut self, x: f64, y: f64, now: Time) -> bool {
        let hit = self.hit(x, y);
        if hit == self.hover { return false; }
        self.hover = hit;
        for (key, spring) in self.springs.iter_mut() {
            if Some(*key) != hit { spring.set(0.0, now); }
        }
        if let Some(hit) = hit {
            self.springs.entry(hit).or_insert_with(|| Spring::new(0.0, 0.2, 1.0)).set(1.0, now);
        }
        self.springs.retain(|_, s| s.target() > 0.0 || s.is_moving(now));
        true
    }

    /// A click; true when it closes the sheet.
    pub fn click(&mut self, x: f64, y: f64, now: Time) -> bool {
        match self.hit(x, y) {
            Some(Hit::Close) => return true,
            Some(Hit::Previous) => self.show(shift(self.shown, -1), now),
            Some(Hit::Next) => self.show(shift(self.shown, 1), now),
            Some(Hit::Today) => self.select(self.today, now),
            Some(Hit::Day(day)) => self.select(day, now),
            None => {}
        }
        false
    }

    /// The wheel (pixels, down positive) turns the months.
    pub fn scroll(&mut self, px: f64, now: Time) -> bool {
        if now - self.wheeled > 0.4 { self.wheel = 0.0; }
        self.wheeled = now;
        self.wheel += px;
        let steps = (self.wheel / WHEEL_STEP).trunc();
        if steps == 0.0 { return false; }
        self.wheel -= steps * WHEEL_STEP;
        self.show(shift(self.shown, steps as i32), now);
        true
    }

    /// Left / Right a day, Up / Down a week, PageUp / PageDown a month,
    /// Home today. Returns whether the key was taken.
    pub fn key(&mut self, key: &str, now: Time) -> bool {
        let day = match key {
            "Left" => self.selected.plus(-1),
            "Right" => self.selected.plus(1),
            "Up" => self.selected.plus(-7),
            "Down" => self.selected.plus(7),
            "Home" => self.today,
            "PageUp" | "PageDown" => {
                let month = shift(self.shown, if key == "PageUp" { -1 } else { 1 });
                let day = self.selected.day.min(month_length(month.0, month.1));
                Date { year: month.0, month: month.1, day }
            }
            _ => return false,
        };
        self.select(day, now);
        true
    }

    /// `h` is the sheet's height as it is now (it eases between months
    /// of five and six weeks); the footer rides its bottom edge.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, h: f64, alpha: f64, now: Time) {
        let hover = |hit: Hit| self.springs.get(&hit).map_or(0.0, |s| s.value(now).clamp(0.0, 1.0));
        let key = format!("calendar{:.4}", self.opened);
        // the header: a red tile with today's number, the month and year,
        // ‹ › and the close control
        let tile_y = y + menu_style::HEADER_MID - menu_style::TILE / 2.0;
        menu_style::tile(cr, x + RAIL, tile_y, menu_style::TILE, menu_style::RED, alpha);
        menu_style::centred(cr, renderer, &self.today.day.to_string(), (x + RAIL, tile_y, menu_style::TILE, menu_style::TILE),
            Font::Headline, (1.0, 1.0, 1.0), alpha);
        let title_h = menu_style::line_height(cr, renderer, Font::Title);
        let title = format!("{} {}", MONTHS[self.shown.1 as usize - 1], self.shown.0);
        crate::motion::text(cr, renderer, &format!("{key}:title"), &title, x + RAIL + menu_style::TILE + 12.0,
            y + menu_style::HEADER_MID - title_h / 2.0, 190.0, 0.0, Font::Title, (1.0, 1.0, 1.0), alpha);
        let (close_x, close_y, size) = menu_style::close_rect(WIDTH);
        menu_style::close(cr, x + close_x, y + close_y, size, hover(Hit::Close), alpha);
        for (hit, cx, angle) in [
            (Hit::Next, close_x - 8.0 - size / 2.0, 0.0),
            (Hit::Previous, close_x - 8.0 - size - 6.0 - size / 2.0, std::f64::consts::PI),
        ] {
            let lit = hover(hit);
            cr.new_path();
            cr.arc(x + cx, y + close_y + size / 2.0, size / 2.0, 0.0, std::f64::consts::TAU);
            cr.set_source_rgba(1.0, 1.0, 1.0, (0.14 + 0.08 * lit) * alpha);
            let _ = cr.fill();
            menu_style::chevron(cr, x + cx + if angle == 0.0 { 1.0 } else { -1.0 }, y + close_y + size / 2.0, 9.0, angle,
                (0.62 + 0.38 * lit) * alpha);
        }

        // the weekdays, the weekend dimmer
        let col_w = (WIDTH - 2.0 * RAIL) / 7.0;
        for (i, name) in WEEKDAYS.iter().enumerate() {
            let a = if i >= 5 { menu_style::TERTIARY } else { menu_style::SECONDARY };
            menu_style::centred(cr, renderer, name, (x + RAIL + i as f64 * col_w, y + WEEKDAYS_MID - 10.0, col_w, 20.0),
                Font::Footnote, (1.0, 1.0, 1.0), a * alpha);
        }

        // the grid: on a month switch the old one slides out the way the
        // months go and the new one follows
        let grid_h = rows(self.shown) as f64 * ROW_H;
        cr.save().unwrap();
        cr.rectangle(x + 4.0, y + GRID_TOP - 4.0, WIDTH - 8.0, grid_h.max(self.switch.map_or(0.0, |(m, ..)| rows(m) as f64 * ROW_H)) + 8.0);
        cr.clip();
        match self.switch {
            Some((from, dir, since)) if now - since < SWITCH => {
                let k = min_jerk(((now - since) / SWITCH).clamp(0.0, 1.0));
                let travel = 44.0;
                self.draw_month(cr, renderer, from, x - dir * travel * k, y, alpha * (1.0 - smoothstep(k / 0.7)), &hover, now);
                self.draw_month(cr, renderer, self.shown, x + dir * travel * (1.0 - k), y, alpha * smoothstep((k - 0.15) / 0.85),
                    &hover, now);
            }
            _ => self.draw_month(cr, renderer, self.shown, x, y, alpha, &hover, now),
        }
        cr.restore().unwrap();

        // the footer: the selected day, how far it is, and the way back
        // while the month switches, the sheet eases to the new number of
        // weeks and the footer rides its edge; otherwise it stays put
        // (also while the sheet unfolds)
        let bottom = match self.switch {
            Some((from, _, since)) if now - since < SWITCH + 0.4 => {
                let (a, b) = (self.height_for(from), self.height());
                h.clamp(a.min(b), a.max(b))
            }
            _ => self.height(),
        };
        let footer = y + bottom - 16.0 - FOOTER_H;
        let sel = self.selected;
        let line = format!("{}, {} {} · {}", WEEKDAY_NAMES[sel.weekday()], sel.day, MONTHS_OF[sel.month as usize - 1],
            relative(sel.days() - self.today.days()));
        let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
        crate::motion::text(cr, renderer, &format!("{key}:footer"), &line, x + RAIL + 4.0, footer + (FOOTER_H - sub_h) / 2.0,
            WIDTH - 2.0 * RAIL - TODAY_W - 4.0, 0.0, Font::Subhead, (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
        if !self.at_today() {
            menu_style::button(cr, renderer, "Сегодня",
                (x + WIDTH - RAIL - TODAY_W, footer + (FOOTER_H - menu_style::SMALL_BUTTON_H) / 2.0, TODAY_W, menu_style::SMALL_BUTTON_H),
                ButtonKind::Plain, hover(Hit::Today), alpha);
        }
    }

    /// One month's days. Today keeps its red disc, another selected day
    /// gets a white one, a hovered day a light wash; weekends are dimmer;
    /// the rows arrive in a wave when the sheet opens.
    #[allow(clippy::too_many_arguments)]
    fn draw_month(&self, cr: &Context, renderer: &Renderer, month: Month, x: f64, y: f64, alpha: f64,
        hover: &dyn Fn(Hit) -> f64, now: Time) {
        if alpha <= 0.002 { return; }
        let col_w = (WIDTH - 2.0 * RAIL) / 7.0;
        let first = Date { year: month.0, month: month.1, day: 1 };
        let lead = first.weekday();
        for day in 1..=month_length(month.0, month.1) {
            let date = Date { year: month.0, month: month.1, day };
            let cell = lead + day as usize - 1;
            let (col, row) = (cell % 7, cell / 7);
            let wave = smoothstep((now - self.opened - 0.03 * row as f64) / 0.32);
            let a = alpha * wave;
            if a <= 0.002 { continue; }
            let cx = x + RAIL + (col as f64 + 0.5) * col_w;
            let cy = y + GRID_TOP + row as f64 * ROW_H + ROW_H / 2.0 + 4.0 * (1.0 - wave);
            let is_today = date == self.today;
            let is_selected = date == self.selected;
            let pop = self.picked.filter(|(d, _)| *d == date).map_or(1.0, |(_, at)| menu_style::bounce_scale(now - at));
            let lit = hover(Hit::Day(date));
            if lit > 0.0 && !is_today && !is_selected {
                menu_style::well(cr, cx, cy, DAY_R, None, 0.7 * lit * a);
            }
            let (font, colour, text_a) = if is_today {
                menu_style::well(cr, cx, cy, DAY_R * pop, Some(menu_style::RED), a);
                (Font::Headline, (1.0, 1.0, 1.0), 1.0)
            } else if is_selected {
                menu_style::well(cr, cx, cy, DAY_R * pop, Some((1.0, 1.0, 1.0)), a);
                (Font::Headline, (0.0, 0.0, 0.0), 1.0)
            } else {
                (Font::Body, (1.0, 1.0, 1.0), if col >= 5 { menu_style::SECONDARY } else { menu_style::PRIMARY })
            };
            menu_style::centred(cr, renderer, &day.to_string(), (cx - DAY_R, cy - DAY_R, 2.0 * DAY_R, 2.0 * DAY_R), font,
                colour, text_a * a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i32, month: u32, day: u32) -> Date { Date { year, month, day } }

    #[test]
    fn dates_and_weekdays() {
        assert_eq!(d(1970, 1, 1).days(), 0);
        assert_eq!(d(2026, 10, 5).weekday(), 0, "a Monday");
        assert_eq!(d(2000, 2, 29).plus(1), d(2000, 3, 1));
        assert_eq!(d(2026, 12, 31).plus(1), d(2027, 1, 1));
        assert_eq!(Date::from_days(d(1969, 7, 20).days()), d(1969, 7, 20));
        assert_eq!(month_length(2028, 2), 29);
        assert_eq!(month_length(2100, 2), 28);
        assert_eq!(shift((2026, 1), -1), (2025, 12));
        assert_eq!(shift((2026, 11), 3), (2027, 2));
        // February 2021 starts on a Monday and fills exactly four weeks
        assert_eq!(rows((2021, 2)), 4);
        assert_eq!(rows((2026, 3)), 6);
        assert_eq!(relative(3), "через 3 дня");
        assert_eq!(relative(11), "через 11 дней");
        assert_eq!(relative(-21), "21 день назад");
        assert_eq!(relative(-1), "вчера");
    }

    #[test]
    fn keys_and_clicks_move_the_selection_across_months() {
        let mut c = Calendar::at(d(2026, 10, 5), 0.0);
        assert!(c.key("Up", 1.0));
        assert_eq!((c.selected, c.shown), (d(2026, 9, 28), (2026, 9)));
        assert!(c.switch.is_some_and(|(from, dir, _)| from == (2026, 10) && dir < 0.0));
        c.key("PageDown", 2.0);
        assert_eq!(c.selected, d(2026, 10, 28));
        c.key("Home", 3.0);
        assert!(c.at_today());
        // the first Thursday of October 2026 sits in the first row
        let col_w = (WIDTH - 2.0 * RAIL) / 7.0;
        assert_eq!(c.hit(RAIL + 3.5 * col_w, GRID_TOP + ROW_H / 2.0), Some(Hit::Day(d(2026, 10, 1))));
        assert_eq!(c.hit(RAIL + 0.5 * col_w, GRID_TOP + ROW_H / 2.0), None, "September's tail is blank");
        assert!(!c.click(RAIL + 3.5 * col_w, GRID_TOP + ROW_H / 2.0, 4.0));
        assert_eq!(c.selected, d(2026, 10, 1));
        assert!(c.scroll(60.0, 5.0));
        assert_eq!(c.shown, (2026, 11));
        let (close_x, close_y, size) = menu_style::close_rect(WIDTH);
        assert!(c.click(close_x + size / 2.0, close_y + size / 2.0, 6.0));
    }
}
