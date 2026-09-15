//! The launcher panel: the island grows into a 660×512 sheet with a search
//! field and the matching applications, each row an icon, a name and a
//! subtitle. Typing re-ranks the list and the rows glide to their new
//! places instead of jumping; the selection follows on a spring.

use cairo::Context;

use crate::anim::{Quintic, Spring, Time};
use crate::apps::{self, App};
use crate::iconload::Icons;
use crate::island::{Font, Renderer};

pub const PANEL_W: f64 = 660.0;
pub const PANEL_H: f64 = 512.0;
pub const PANEL_R: f64 = 32.0;

/// The search field.
const FIELD_TOP: f64 = 46.0;
const FIELD_H: f64 = 46.0;
const FIELD_PAD: f64 = 22.0;
/// The list.
const LIST_TOP: f64 = 98.0;
const ROW_H: f64 = 56.0;
const ROW_PAD: f64 = 14.0;
const ICON: f64 = 38.0;
const ICON_GAP: f64 = 16.0;
pub const MAX_ROWS: usize = 7;

const ROW_SPRING: (f64, f64) = (0.34, 0.82);
const SEL_SPRING: (f64, f64) = (0.28, 0.78);
const FADE: f64 = 0.22;
/// the caret's blink (macOS runs at a bit under two a second)
const CARET_HZ: f64 = 1.6;
const CARET_DUTY: f64 = 0.55;

/// One row on screen: which app it shows, where it is heading and how far
/// it has faded in or out.
struct Row {
    id: String,
    app: App,
    /// its place in the list, as a spring so re-ranking glides
    slot: Spring,
    alpha: Quintic,
    /// when it appeared (its icon's little pop)
    born: Time,
    leaving: bool,
}

pub struct Launcher {
    all: Vec<App>,
    icons: Icons,
    pub query: String,
    rows: Vec<Row>,
    /// the selected row, by app id (survives re-ranking)
    selected: String,
    sel_slot: Spring,
    sel_alpha: Quintic,
    opened: Time,
}

impl Launcher {
    pub fn new(now: Time) -> Launcher {
        let mut l = Launcher {
            all: apps::scan(),
            icons: Icons::new(),
            query: String::new(),
            rows: Vec::new(),
            selected: String::new(),
            sel_slot: Spring::new(0.0, SEL_SPRING.0, SEL_SPRING.1),
            sel_alpha: Quintic::at_rest(0.0, FADE),
            opened: now,
        };
        l.refresh(now);
        l.sel_alpha.set(1.0, now);
        l
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.rows.iter().any(|r| r.slot.is_moving(now) || r.alpha.is_moving(now))
            || self.sel_slot.is_moving(now)
            || self.sel_alpha.is_moving(now)
            // the icons finish popping in
            || self.rows.iter().any(|r| now - r.born < 0.35)
            || now - self.opened < 0.8
    }

    /// The caret blinks, which is not worth a frame callback sixty times a
    /// second: the island wakes up for it at this instant instead.
    pub fn next_caret(&self, now: Time) -> Time {
        let period = 1.0 / CARET_HZ;
        let phase = (now * CARET_HZ).fract();
        now + period * (if phase < CARET_DUTY { CARET_DUTY - phase } else { 1.0 - phase }) + 0.005
    }

    fn caret_on(now: Time) -> bool {
        (now * CARET_HZ).fract() < CARET_DUTY
    }

    /// Re-rank for the current query: rows that stay glide to their new
    /// places, rows that go fade out where they are, new ones fade in.
    pub fn refresh(&mut self, now: Time) {
        let hits: Vec<App> = apps::search(&self.all, &self.query, MAX_ROWS).into_iter().cloned().collect();
        for (i, app) in hits.iter().enumerate() {
            match self.rows.iter_mut().find(|r| r.id == app.id) {
                Some(row) => {
                    row.leaving = false;
                    row.slot.set(i as f64, now);
                    row.alpha.set(1.0, now);
                    row.app = app.clone();
                }
                None => {
                    let mut slot = Spring::new(i as f64 + 0.35, ROW_SPRING.0, ROW_SPRING.1);
                    slot.set(i as f64, now);
                    let mut alpha = Quintic::at_rest(0.0, FADE);
                    alpha.set(1.0, now);
                    self.rows.push(Row { id: app.id.clone(), app: app.clone(), slot, alpha, born: now, leaving: false });
                }
            }
        }
        for row in &mut self.rows {
            if !hits.iter().any(|a| a.id == row.id) && !row.leaving {
                row.leaving = true;
                row.alpha.set(0.0, now);
            }
        }
        self.rows.retain(|r| !(r.leaving && !r.alpha.is_moving(now) && r.alpha.state(now).x <= 0.0));
        // keep the selection if it is still in the list, else take the top
        if !hits.iter().any(|a| a.id == self.selected) {
            self.selected = hits.first().map(|a| a.id.clone()).unwrap_or_default();
        }
        self.sync_selection(now);
    }

    fn results(&self) -> Vec<&Row> {
        let mut rows: Vec<&Row> = self.rows.iter().filter(|r| !r.leaving).collect();
        rows.sort_by(|a, b| a.slot.target().partial_cmp(&b.slot.target()).unwrap_or(std::cmp::Ordering::Equal));
        rows
    }

    fn sync_selection(&mut self, now: Time) {
        let target = self.rows.iter().find(|r| r.id == self.selected).map(|r| r.slot.target());
        match target {
            Some(slot) => {
                self.sel_slot.set(slot, now);
                self.sel_alpha.set(1.0, now);
            }
            None => self.sel_alpha.set(0.0, now),
        }
    }

    /// Move the selection by `delta` rows (wrapping).
    pub fn move_selection(&mut self, delta: i32, now: Time) {
        let rows = self.results();
        if rows.is_empty() {
            return;
        }
        let current = rows.iter().position(|r| r.id == self.selected).unwrap_or(0) as i32;
        let n = rows.len() as i32;
        let next = (current + delta).rem_euclid(n) as usize;
        self.selected = rows[next].id.clone();
        self.sync_selection(now);
    }

    pub fn select_index(&mut self, index: usize, now: Time) {
        if let Some(row) = self.results().get(index) {
            let id = row.id.clone();
            if id != self.selected {
                self.selected = id;
                self.sync_selection(now);
            }
        }
    }

    pub fn push(&mut self, text: &str, now: Time) {
        self.query.push_str(text);
        self.refresh(now);
    }

    pub fn backspace(&mut self, now: Time) {
        self.query.pop();
        self.refresh(now);
    }

    /// Ctrl+U: wipe the query.
    pub fn clear(&mut self, now: Time) {
        self.query.clear();
        self.refresh(now);
    }

    /// Launch what is selected; `false` when nothing matches.
    pub fn launch(&self) -> bool {
        match self.rows.iter().find(|r| r.id == self.selected) {
            Some(row) => {
                apps::launch(&row.app);
                true
            }
            None => false,
        }
    }

    /// Which row is at `y` inside the panel (for the pointer).
    pub fn row_at(&self, panel_top: f64, y: f64) -> Option<usize> {
        let rel = y - (panel_top + LIST_TOP);
        if rel < 0.0 {
            return None;
        }
        let index = (rel / ROW_H) as usize;
        (index < self.results().len()).then_some(index)
    }

    /// Draw the panel's content inside the box at (`x`, `y`, `w`), faded in
    /// by `alpha` (the panel's own growth is the island's business).
    pub fn draw(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, w: f64, alpha: f64, now: Time) {
        if alpha <= 0.002 {
            return;
        }
        let t = now - self.opened;
        // the search field
        let fx = x + FIELD_PAD;
        let fw = w - 2.0 * FIELD_PAD;
        let fy = y + FIELD_TOP;
        let intro = |delay: f64| {
            let k = crate::island::smoothstep(((t - delay) / 0.3).clamp(0.0, 1.0));
            (alpha * k, 10.0 * (1.0 - k))
        };
        let (fa, fo) = intro(0.0);
        rounded(cr, fx, fy + fo, fw, FIELD_H, FIELD_H / 2.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.09 * fa);
        let _ = cr.fill();

        // the magnifying glass (SF Symbols, in the same face as the text)
        let glass = renderer.layout(cr, "\u{1002AB}", Font::Text);
        let (_, g_ext) = glass.pixel_extents();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.45 * fa);
        cr.move_to(renderer.snap(fx + 18.0), renderer.snap(fy + fo + (FIELD_H - g_ext.height() as f64) / 2.0 - g_ext.y() as f64));
        pangocairo::functions::show_layout(cr, &glass);

        let text_x = fx + 18.0 + g_ext.width() as f64 + 12.0;
        // the caret sits ahead of the placeholder, not on top of it
        let placeholder_shift = 10.0;
        let (query, placeholder) = if self.query.is_empty() {
            ("Search".to_string(), true)
        } else {
            (self.query.clone(), false)
        };
        let q_layout = renderer.layout_ellipsized(cr, &query, Font::Query, fw - 90.0, 1);
        let (_, q_ext) = q_layout.pixel_extents();
        let q_y = renderer.snap(fy + fo + (FIELD_H - q_ext.height() as f64) / 2.0 - q_ext.y() as f64);
        cr.set_source_rgba(1.0, 1.0, 1.0, if placeholder { 0.35 * fa } else { fa });
        cr.move_to(renderer.snap(text_x + if placeholder { placeholder_shift } else { 0.0 }), q_y);
        pangocairo::functions::show_layout(cr, &q_layout);

        // the caret, blinking on the macOS cadence
        if Self::caret_on(now) {
            let cx = renderer.snap(text_x + if placeholder { 0.0 } else { q_ext.width() as f64 } + 1.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.85 * fa);
            cr.rectangle(cx, renderer.snap(fy + fo + 11.0), 2.0, FIELD_H - 22.0);
            let _ = cr.fill();
        }

        // the selection, gliding between rows
        let sel_alpha = self.sel_alpha.state(now).x * alpha;
        if sel_alpha > 0.002 {
            let sy = y + LIST_TOP + self.sel_slot.state(now).x * ROW_H;
            rounded(cr, x + FIELD_PAD - 6.0, sy + 2.0, w - 2.0 * (FIELD_PAD - 6.0), ROW_H - 4.0, 13.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.10 * sel_alpha);
            let _ = cr.fill();
        }

        // the rows
        for row in &self.rows {
            let slot = row.slot.state(now).x;
            let row_alpha = row.alpha.state(now).x * alpha;
            if row_alpha <= 0.004 {
                continue;
            }
            // each row arrives a beat after the one above it
            let (ra, ro) = intro(0.06 + 0.045 * slot.max(0.0));
            let ry = y + LIST_TOP + slot * ROW_H + ro;
            self.draw_row(cr, renderer, row, x, ry, w, row_alpha * (ra / alpha).min(1.0), now);
        }

        if self.results().is_empty() {
            let text = renderer.layout(cr, "No results", Font::Text);
            let (_, e) = text.pixel_extents();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.4 * alpha);
            cr.move_to(renderer.snap(x + w / 2.0 - e.width() as f64 / 2.0), renderer.snap(y + LIST_TOP + 30.0));
            pangocairo::functions::show_layout(cr, &text);
        }
    }

    fn draw_row(&self, cr: &Context, renderer: &Renderer, row: &Row, x: f64, y: f64, w: f64, alpha: f64, now: Time) {
        let icon_x = x + FIELD_PAD + ROW_PAD - 6.0;
        let icon_y = y + (ROW_H - ICON) / 2.0;
        // the icon pops in like a notification's
        let age = now - row.born;
        let pop = 0.86 + 0.14 * crate::island::smoothstep((age / 0.3).clamp(0.0, 1.0));
        // the icon comes pre-scaled to the device pixels it covers, so a
        // frame only has to blit it
        let device = (ICON * renderer.scale()).round().max(1.0);
        match row.app.icon.as_deref().and_then(|name| self.icons.get_scaled(name, device as i32)) {
            Some(surface) => {
                let scale = pop / renderer.scale();
                cr.save().unwrap();
                cr.translate(icon_x + ICON / 2.0, icon_y + ICON / 2.0);
                cr.scale(scale, scale);
                let _ = cr.set_source_surface(&surface, -(surface.width() as f64) / 2.0, -(surface.height() as f64) / 2.0);
                let _ = cr.paint_with_alpha(alpha);
                cr.restore().unwrap();
            }
            None => monogram(cr, renderer, &row.app.name, icon_x, icon_y, ICON * pop, alpha),
        }

        let text_x = renderer.snap(icon_x + ICON + ICON_GAP);
        let max_w = w - (text_x - x) - FIELD_PAD;
        let name = renderer.layout_ellipsized(cr, &row.app.name, Font::Text, max_w, 1);
        let (_, n_ext) = name.pixel_extents();
        if row.app.subtitle.is_empty() {
            cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
            cr.move_to(text_x, renderer.snap(y + (ROW_H - n_ext.height() as f64) / 2.0 - n_ext.y() as f64));
            pangocairo::functions::show_layout(cr, &name);
        } else {
            let sub = renderer.layout_ellipsized(cr, &row.app.subtitle, Font::Small, max_w, 1);
            let (_, s_ext) = sub.pixel_extents();
            let total = n_ext.height() as f64 + 1.0 + s_ext.height() as f64;
            let top = y + (ROW_H - total) / 2.0;
            cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
            cr.move_to(text_x, renderer.snap(top - n_ext.y() as f64));
            pangocairo::functions::show_layout(cr, &name);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.5 * alpha);
            cr.move_to(text_x, renderer.snap(top + n_ext.height() as f64 + 1.0 - s_ext.y() as f64));
            pangocairo::functions::show_layout(cr, &sub);
        }
    }
}

/// An app with no icon of its own gets a monogram tile.
fn monogram(cr: &Context, renderer: &Renderer, name: &str, x: f64, y: f64, size: f64, alpha: f64) {
    let letter: String = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    // a stable colour per name
    let hash = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let hue = (hash % 360) as f64;
    let (r, g, b) = hsl(hue, 0.5, 0.55);
    rounded(cr, x, y, size, size, size * 0.28);
    cr.set_source_rgba(r, g, b, alpha);
    let _ = cr.fill();
    let layout = renderer.layout(cr, &letter, Font::Text);
    let (_, e) = layout.pixel_extents();
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.move_to(renderer.snap(x + (size - e.width() as f64) / 2.0), renderer.snap(y + (size - e.height() as f64) / 2.0 - e.y() as f64));
    pangocairo::functions::show_layout(cr, &layout);
}

fn hsl(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_path();
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.arc(x + w - r, y + r, r, 1.5 * std::f64::consts::PI, 2.0 * std::f64::consts::PI);
    cr.arc(x + w - r, y + h - r, r, 0.0, 0.5 * std::f64::consts::PI);
    cr.arc(x + r, y + h - r, r, 0.5 * std::f64::consts::PI, std::f64::consts::PI);
    cr.close_path();
}
