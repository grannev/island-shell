//! The launcher panel: the island grows into a 660×512 sheet with a search
//! field and the matching applications, each row an icon, a name and a
//! subtitle. Typing re-ranks the list and the rows glide to their new
//! places instead of jumping; the selection follows on a spring.

use cairo::Context;
use std::{rc::Rc, sync::Arc};

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
/// rows that fit in the list's viewport
pub const VISIBLE_ROWS: usize = 7;
/// how many matches are kept at all (far more than anyone scrolls through)
const MAX_MATCHES: usize = 200;
/// the list fades out at its edges instead of showing a scrollbar
const EDGE_FADE: f64 = 22.0;
/// keep this many rows visible past the selection when scrolling to it
const SCROLL_MARGIN: f64 = 1.0;

const ROW_SPRING: (f64, f64) = (0.34, 0.82);
const SEL_SPRING: (f64, f64) = (0.28, 0.78);
const SCROLL_SPRING: (f64, f64) = (0.3, 0.95);
const FADE: f64 = 0.22;
/// the caret's blink (macOS runs at a bit under two a second)
const CARET_HZ: f64 = 1.6;
const CARET_DUTY: f64 = 0.55;
const FILE_DEBOUNCE: f64 = 0.18;
const EDGE_LIMIT: f64 = 28.0;
const EDGE_RELEASE: f64 = 0.10;
const EDGE_SETTLE: f64 = 0.85;

#[derive(Clone, Copy)]
struct EdgePull {
    from: f64,
    peak: f64,
    since: Time,
}

#[derive(Clone)]
enum Entry {
    Application(App),
    File(crate::files::FileHit),
}

impl Entry {
    fn id(&self) -> String {
        match self { Self::Application(a) => a.id.clone(), Self::File(f) => format!("file:{:?}", f.path) }
    }
    fn name(&self) -> &str { match self { Self::Application(a) => &a.name, Self::File(f) => &f.name } }
    fn subtitle(&self) -> &str { match self { Self::Application(a) => &a.subtitle, Self::File(f) => &f.subtitle } }
    fn icon(&self) -> Option<&str> { match self { Self::Application(a) => a.icon.as_deref(), Self::File(_) => None } }
}

/// One row on screen: which app it shows, where it is heading and how far
/// it has faded in or out.
struct Row {
    id: String,
    entry: Entry,
    /// its place in the list, as a spring so re-ranking glides
    slot: Spring,
    alpha: Quintic,
    /// when it appeared (its icon's little pop)
    born: Time,
    leaving: bool,
}

pub struct Launcher {
    all: Arc<Vec<App>>,
    icons: Rc<Icons>,
    file_search: Rc<crate::files::Search>,
    files: Vec<crate::files::FileHit>,
    file_request: Option<u64>,
    file_due: Option<Time>,
    file_note: Option<String>,
    pub query: String,
    rows: Vec<Row>,
    /// the selected row, by app id (survives re-ranking)
    selected: String,
    sel_slot: Spring,
    sel_alpha: Quintic,
    /// how far the list is scrolled, in rows
    scroll: Spring,
    close_hover: Spring,
    edge_pull: Option<EdgePull>,
    matches: usize,
    opened: Time,
}

impl Launcher {
    pub fn new(now: Time, all: Arc<Vec<App>>, icons: Rc<Icons>, file_search: Rc<crate::files::Search>) -> Launcher {
        let mut l = Launcher {
            all,
            icons,
            file_search,
            files: Vec::new(),
            file_request: None,
            file_due: None,
            file_note: None,
            query: String::new(),
            rows: Vec::new(),
            selected: String::new(),
            sel_slot: Spring::new(0.0, SEL_SPRING.0, SEL_SPRING.1),
            sel_alpha: Quintic::at_rest(0.0, FADE),
            scroll: Spring::new(0.0, SCROLL_SPRING.0, SCROLL_SPRING.1),
            close_hover: Spring::new(0.0, 0.24, 0.85),
            edge_pull: None,
            matches: 0,
            opened: now,
        };
        l.refresh(now);
        l.sel_alpha.set(1.0, now);
        l
    }

    pub fn set_apps(&mut self, apps: Arc<Vec<App>>, now: Time) {
        self.all = apps;
        self.refresh(now);
    }

    pub fn result_count(&self) -> usize { self.matches }

    pub fn file_count(&self) -> usize { self.files.len() }

    fn query_changed(&mut self, now: Time) {
        self.edge_pull = None;
        self.file_search.cancel();
        self.file_request = None;
        self.files.clear();
        self.file_note = None;
        self.file_due = (crate::files::query(&self.query).0.chars().count() >= 2).then_some(now + FILE_DEBOUNCE);
        self.refresh(now);
    }

    pub fn advance(&mut self, now: Time) -> bool {
        if !self.file_due.is_some_and(|due| now >= due) { return false; }
        self.file_due = None;
        self.file_request = self.file_search.request(crate::files::query(&self.query).0.to_string());
        if self.file_request.is_none() { self.file_note = Some("Поиск занят · повторите запрос".into()); }
        true
    }

    pub fn receive_files(&mut self, generation: u64, reply: crate::files::Reply, now: Time) -> bool {
        if self.file_request != Some(generation) { return false; }
        self.file_request = None;
        self.files = reply.files;
        self.file_note = reply.note;
        self.refresh(now);
        true
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.rows.iter().any(|r| r.slot.is_moving(now) || r.alpha.is_moving(now))
            || self.sel_slot.is_moving(now)
            || self.sel_alpha.is_moving(now)
            || self.scroll.is_moving(now)
            || self.close_hover.is_moving(now)
            || self.edge_pull.is_some_and(|pull| now - pull.since < EDGE_SETTLE)
            // the icons finish popping in
            || self.rows.iter().any(|r| now - r.born < 0.35)
            || now - self.opened < 0.8
    }

    /// The caret blinks, which is not worth a frame callback sixty times a
    /// second: the island wakes up for it at this instant instead.
    pub fn next_caret(&self, now: Time) -> Time {
        let period = 1.0 / CARET_HZ;
        let phase = (now * CARET_HZ).fract();
        let caret = now + period * (if phase < CARET_DUTY { CARET_DUTY - phase } else { 1.0 - phase }) + 0.005;
        self.file_due.map_or(caret, |due| due.min(caret))
    }

    fn caret_on(now: Time) -> bool {
        (now * CARET_HZ).fract() < CARET_DUTY
    }

    /// Re-rank for the current query: rows that stay glide to their new
    /// places, rows that go fade out where they are, new ones fade in.
    pub fn refresh(&mut self, now: Time) {
        let (query, files_only) = crate::files::query(&self.query);
        let mut hits: Vec<Entry> = if files_only { Vec::new() } else {
            apps::search(&self.all, query, MAX_MATCHES).into_iter().cloned().map(Entry::Application).collect()
        };
        if !self.files.is_empty() {
            let other_apps = hits.split_off(hits.len().min(5));
            hits.extend(self.files.iter().cloned().map(Entry::File));
            hits.extend(other_apps);
        }
        let ids: Vec<_> = hits.iter().map(Entry::id).collect();
        let present: std::collections::HashSet<_> = ids.iter().map(String::as_str).collect();
        self.matches = hits.len();
        for (i, entry) in hits.iter().enumerate() {
            let id = &ids[i];
            match self.rows.iter_mut().find(|r| &r.id == id) {
                Some(row) => {
                    row.leaving = false;
                    row.slot.set(i as f64, now);
                    row.alpha.set(1.0, now);
                    row.entry = entry.clone();
                }
                None => {
                    let mut slot = Spring::new(i as f64 + 0.35, ROW_SPRING.0, ROW_SPRING.1);
                    slot.set(i as f64, now);
                    let mut alpha = Quintic::at_rest(0.0, FADE);
                    alpha.set(1.0, now);
                    self.rows.push(Row { id: id.clone(), entry: entry.clone(), slot, alpha, born: now, leaving: false });
                }
            }
        }
        for row in &mut self.rows {
            if !present.contains(row.id.as_str()) && !row.leaving {
                row.leaving = true;
                row.alpha.set(0.0, now);
            }
        }
        self.rows.retain(|r| !(r.leaving && !r.alpha.is_moving(now) && r.alpha.state(now).x <= 0.0));
        // keep the selection if it is still in the list, else take the top
        if !present.contains(self.selected.as_str()) {
            self.selected = ids.first().cloned().unwrap_or_default();
            self.scroll.set(0.0, now);
        }
        self.sync_selection(now);
        // a shorter list may leave the scroll past its end
        let clamped = self.scroll.target().clamp(0.0, self.max_scroll());
        if (clamped - self.scroll.target()).abs() > 1e-6 {
            self.scroll.set(clamped, now);
        }
        self.scroll_to_selection(now);
    }

    fn results(&self) -> Vec<&Row> {
        let mut rows: Vec<&Row> = self.rows.iter().filter(|r| !r.leaving).collect();
        rows.sort_by(|a, b| a.slot.target().partial_cmp(&b.slot.target()).unwrap_or(std::cmp::Ordering::Equal));
        rows
    }

    /// The furthest the list can be scrolled, in rows.
    fn max_scroll(&self) -> f64 {
        (self.matches as f64 - VISIBLE_ROWS as f64).max(0.0)
    }

    /// Scroll by `rows` (the wheel).
    pub fn scroll_by(&mut self, rows: f64, now: Time) -> bool {
        if rows.abs() < 1e-6 { return false; }
        let wanted = self.scroll.target() + rows;
        let target = wanted.clamp(0.0, self.max_scroll());
        let excess = wanted - target;
        if excess.abs() > 1e-6 {
            self.pull_edge(excess.signum(), excess.abs(), now);
        }
        if (target - self.scroll.target()).abs() < 1e-6 {
            return excess.abs() > 1e-6;
        }
        self.scroll.set(target, now);
        true
    }

    fn edge_offset(&self, now: Time) -> f64 {
        let Some(pull) = self.edge_pull else { return 0.0 };
        let elapsed = (now - pull.since).max(0.0);
        if elapsed < EDGE_RELEASE {
            let k = crate::anim::min_jerk(elapsed / EDGE_RELEASE);
            pull.from + (pull.peak - pull.from) * k
        } else if elapsed < EDGE_SETTLE {
            let t = (elapsed - EDGE_RELEASE) / 0.10;
            pull.peak * (1.0 + t) * (-t).exp()
        } else { 0.0 }
    }

    /// A resisted pull, shared by keys and the wheel. Keeping the current
    /// rendered offset as the start makes rapid repeated input continuous.
    fn pull_edge(&mut self, direction: f64, strength: f64, now: Time) {
        let from = self.edge_offset(now);
        let impulse = -direction.signum() * 18.0 * strength.clamp(0.35, 2.5);
        let wanted = from + impulse;
        let peak = EDGE_LIMIT * wanted / (EDGE_LIMIT + wanted.abs());
        self.edge_pull = Some(EdgePull { from, peak, since: now });
    }

    /// Bring the selection into view, keeping a row of context where there
    /// is one.
    fn scroll_to_selection(&mut self, now: Time) {
        let Some(slot) = self.rows.iter().find(|r| r.id == self.selected).map(|r| r.slot.target()) else { return };
        let first = self.scroll.target();
        let last = first + VISIBLE_ROWS as f64 - 1.0;
        let margin = if self.matches > VISIBLE_ROWS + 1 { SCROLL_MARGIN } else { 0.0 };
        let target = if slot < first + margin {
            slot - margin
        } else if slot > last - margin {
            slot - VISIBLE_ROWS as f64 + 1.0 + margin
        } else {
            return;
        };
        self.scroll.set(target.clamp(0.0, self.max_scroll()), now);
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

    /// Move the selection by `delta` rows without wrapping at either edge.
    pub fn move_selection(&mut self, delta: i32, now: Time) {
        let rows = self.results();
        if rows.is_empty() {
            return;
        }
        let current = rows.iter().position(|r| r.id == self.selected).unwrap_or(0) as i32;
        let n = rows.len() as i32;
        let wanted = current + delta;
        if wanted < 0 || wanted >= n {
            self.scroll_to_selection(now);
            self.pull_edge(delta.signum() as f64, 1.0, now);
            return;
        }
        let next_id = rows[wanted as usize].id.clone();
        self.selected = next_id;
        self.sync_selection(now);
        // a shorter list may leave the scroll past its end
        let clamped = self.scroll.target().clamp(0.0, self.max_scroll());
        if (clamped - self.scroll.target()).abs() > 1e-6 {
            self.scroll.set(clamped, now);
        }
        self.scroll_to_selection(now);
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
        self.query_changed(now);
    }

    pub fn backspace(&mut self, now: Time) {
        self.query.pop();
        self.query_changed(now);
    }

    /// Delete one Unicode word (or a run of punctuation) before the caret.
    pub fn delete_previous_word(&mut self, now: Time) {
        let end = self.query.trim_end_matches(char::is_whitespace).len();
        let Some(last) = self.query[..end].chars().next_back() else {
            if !self.query.is_empty() { self.query.clear(); self.query_changed(now); }
            return;
        };
        let word = |ch: char| ch.is_alphanumeric() || ch == '_';
        let start = self.query[..end].char_indices().rev()
            .take_while(|(_, ch)| !ch.is_whitespace() && word(*ch) == word(last))
            .last().map_or(end, |(index, _)| index);
        self.query.truncate(start);
        self.query_changed(now);
    }

    pub fn close_hit(x: f64, y: f64, width: f64) -> bool {
        x >= width - 53.0 && x <= width - 13.0 && (7.0..49.0).contains(&y)
    }

    pub fn set_close_hover(&mut self, x: f64, y: f64, now: Time) -> bool {
        let hovered = Self::close_hit(x, y, PANEL_W);
        if (self.close_hover.target() > 0.5) == hovered { return false; }
        self.close_hover.set(f64::from(hovered), now);
        true
    }

    /// Ctrl+U: wipe the query.
    pub fn clear(&mut self, now: Time) {
        self.query.clear();
        self.query_changed(now);
    }

    /// Launch what is selected; `false` when nothing matches.
    pub fn launch(&self) -> bool {
        match self.rows.iter().find(|r| !r.leaving && r.id == self.selected) {
            Some(row) => {
                match &row.entry {
                    Entry::Application(app) => { apps::launch(app); true }
                    Entry::File(file) => crate::files::open(&file.path),
                }
            }
            None => false,
        }
    }

    /// Which row is at `y` inside the panel (for the pointer).
    pub fn row_at(&self, panel_top: f64, y: f64, now: Time) -> Option<usize> {
        let rel = y - (panel_top + LIST_TOP);
        if rel < 0.0 || rel > VISIBLE_ROWS as f64 * ROW_H {
            return None;
        }
        let index = (rel / ROW_H + self.scroll.state(now).x).floor();
        (index >= 0.0 && (index as usize) < self.matches).then_some(index as usize)
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
        let close = self.close_hover.value(now).clamp(0.0, 1.0);
        rounded(cr, x + w - 49.0, y + 10.0, 34.0, 34.0, 17.0);
        cr.set_source_rgba(0.16 + 0.08 * close, 0.16 + 0.08 * close, 0.17 + 0.08 * close, alpha);
        let _ = cr.fill();
        cr.set_line_width(1.8);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_source_rgba(1.0, 1.0, 1.0, (0.62 + 0.36 * close) * alpha);
        cr.move_to(x + w - 39.0, y + 20.0);
        cr.line_to(x + w - 25.0, y + 34.0);
        cr.move_to(x + w - 25.0, y + 20.0);
        cr.line_to(x + w - 39.0, y + 34.0);
        let _ = cr.stroke();
        rounded(cr, fx, fy + fo, fw, FIELD_H, FIELD_H / 2.0);
        cr.set_source_rgba(0.15, 0.15, 0.17, fa);
        let _ = cr.fill();
        rounded(cr, fx + 0.5, fy + fo + 0.5, fw - 1.0, FIELD_H - 1.0, (FIELD_H - 1.0) / 2.0);
        cr.set_line_width(1.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.11 * fa);
        let _ = cr.stroke();

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
            ("Приложения и файлы".to_string(), true)
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
            cr.set_source_rgba(0.40, 0.80, 1.0, 0.95 * fa);
            cr.rectangle(cx, renderer.snap(fy + fo + 11.0), 2.0, FIELD_H - 22.0);
            let _ = cr.fill();
        }

        // the list scrolls inside its own viewport; there is no scrollbar,
        // the rows simply fade out at the top and bottom edges
        let scroll = self.scroll.state(now).x;
        let view_top = y + LIST_TOP;
        let view_h = VISIBLE_ROWS as f64 * ROW_H;
        cr.save().unwrap();
        cr.rectangle(x, view_top, w, view_h);
        cr.clip();
        let edge = self.edge_offset(now);
        cr.save().unwrap();
        cr.translate(0.0, edge);

        // the selection, gliding between rows
        let sel_alpha = self.sel_alpha.state(now).x * alpha;
        if sel_alpha > 0.002 {
            let sy = view_top + (self.sel_slot.state(now).x - scroll) * ROW_H;
            rounded(cr, x + FIELD_PAD - 6.0, sy + 2.0, w - 2.0 * (FIELD_PAD - 6.0), ROW_H - 4.0, 13.0);
            cr.set_source_rgba(0.16, 0.20, 0.26, sel_alpha);
            let _ = cr.fill();
            rounded(cr, x + FIELD_PAD - 5.5, sy + 2.5, w - 2.0 * (FIELD_PAD - 6.0) - 1.0, ROW_H - 5.0, 12.5);
            cr.set_line_width(1.0);
            cr.set_source_rgba(0.42, 0.77, 1.0, 0.23 * sel_alpha);
            let _ = cr.stroke();
        }

        // the rows
        for row in &self.rows {
            let slot = row.slot.state(now).x;
            let row_alpha = row.alpha.state(now).x * alpha;
            if row_alpha <= 0.004 {
                continue;
            }
            let place = slot - scroll;
            if place < -1.2 || place > VISIBLE_ROWS as f64 + 0.2 {
                continue; // outside the viewport: nothing to draw
            }
            // each row arrives a beat after the one above it
            let (ra, ro) = intro(0.06 + 0.045 * place.max(0.0));
            let ry = view_top + place * ROW_H + ro;
            self.draw_row(cr, renderer, row, x, ry, w, row_alpha * (ra / alpha).min(1.0), now);
        }
        cr.restore().unwrap();

        // the fades: black over the panel's own black, so rows dissolve
        // into the edges of the viewport when there is more to scroll to
        let fade_top = (scroll / 0.6).clamp(0.0, 1.0);
        let fade_bottom = ((self.max_scroll() - scroll) / 0.6).clamp(0.0, 1.0);
        if fade_top > 0.0 {
            let g = cairo::LinearGradient::new(0.0, view_top, 0.0, view_top + EDGE_FADE);
            g.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, fade_top);
            g.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
            let _ = cr.set_source(&g);
            cr.rectangle(x, view_top, w, EDGE_FADE);
            let _ = cr.fill();
        }
        if fade_bottom > 0.0 {
            let g = cairo::LinearGradient::new(0.0, view_top + view_h - EDGE_FADE, 0.0, view_top + view_h);
            g.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
            g.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, fade_bottom);
            let _ = cr.set_source(&g);
            cr.rectangle(x, view_top + view_h - EDGE_FADE, w, EDGE_FADE);
            let _ = cr.fill();
        }
        cr.restore().unwrap();

        if self.matches == 0 {
            let text = renderer.layout(cr, if self.file_due.is_some() || self.file_request.is_some() {
                "Ищем файлы…"
            } else { "Ничего не найдено" }, Font::Text);
            let (_, e) = text.pixel_extents();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.4 * alpha);
            cr.move_to(renderer.snap(x + w / 2.0 - e.width() as f64 / 2.0), renderer.snap(y + LIST_TOP + 30.0));
            pangocairo::functions::show_layout(cr, &text);
        }
        let footer = if self.file_due.is_some() || self.file_request.is_some() {
            "Поиск файлов…"
        } else if let Some(note) = &self.file_note { note }
        else { "file: — только файлы и каталоги · Enter — открыть" };
        let hint = renderer.layout_ellipsized(cr, footer, Font::Tiny, w - 2.0 * FIELD_PAD, 1);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.36 * alpha);
        cr.move_to(renderer.snap(x + FIELD_PAD), renderer.snap(y + 494.0));
        pangocairo::functions::show_layout(cr, &hint);
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
        match row.entry.icon().and_then(|name| self.icons.get_scaled(name, device as i32)) {
            Some(surface) => {
                let scale = pop / renderer.scale();
                cr.save().unwrap();
                cr.translate(icon_x + ICON / 2.0, icon_y + ICON / 2.0);
                cr.scale(scale, scale);
                let _ = cr.set_source_surface(&surface, -(surface.width() as f64) / 2.0, -(surface.height() as f64) / 2.0);
                let _ = cr.paint_with_alpha(alpha);
                cr.restore().unwrap();
            }
            None => match &row.entry {
                Entry::File(file) => file_icon(cr, file.directory, icon_x, icon_y, ICON * pop, alpha),
                Entry::Application(app) => monogram(cr, renderer, &app.name, icon_x, icon_y, ICON * pop, alpha),
            },
        }

        let text_x = renderer.snap(icon_x + ICON + ICON_GAP);
        let max_w = w - (text_x - x) - FIELD_PAD;
        let name = renderer.layout_ellipsized(cr, row.entry.name(), Font::Text, max_w, 1);
        let (_, n_ext) = name.pixel_extents();
        if row.entry.subtitle().is_empty() {
            cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
            cr.move_to(text_x, renderer.snap(y + (ROW_H - n_ext.height() as f64) / 2.0 - n_ext.y() as f64));
            pangocairo::functions::show_layout(cr, &name);
        } else {
            let sub = renderer.layout_ellipsized(cr, row.entry.subtitle(), Font::Small, max_w, 1);
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

impl Drop for Launcher {
    fn drop(&mut self) { self.file_search.cancel(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher() -> Launcher {
        Launcher::new(0.0, Arc::new(Vec::new()), Rc::new(Icons::new()), Rc::new(crate::files::Search::new()))
    }
    fn reply(name: &str) -> crate::files::Reply {
        crate::files::Reply { files: vec![crate::files::FileHit {
            path: std::path::Path::new("/tmp").join(name), name: name.into(), subtitle: "Файл · ~/Documents".into(), directory: false,
        }], note: None }
    }

    #[test]
    fn file_search_debounces_and_clearing_removes_the_deadline() {
        let mut l = launcher();
        l.push("r", 1.0);
        assert!(l.file_due.is_none());
        l.push("e", 1.1);
        assert!((l.file_due.unwrap() - 1.28).abs() < 1e-9);
        assert!(!l.advance(1.2));
        l.push("ad", 1.21);
        assert!((l.file_due.unwrap() - 1.39).abs() < 1e-9);
        l.clear(1.3);
        assert!(l.file_due.is_none());
        assert!(!l.advance(10.0));
    }

    #[test]
    fn late_file_replies_cannot_replace_a_new_query_or_reactivate_old_rows() {
        let mut l = launcher();
        l.push("file:notes", 1.0);
        l.file_due = None;
        l.file_request = Some(7);
        assert!(!l.receive_files(6, reply("old.txt"), 1.2));
        assert!(l.receive_files(7, reply("notes.txt"), 1.3));
        assert_eq!(l.result_count(), 1);
        assert_eq!(l.file_count(), 1);
        let selected = l.selected.clone();
        l.set_apps(Arc::new(Vec::new()), 1.4);
        assert_eq!(l.selected, selected);
        l.clear(1.5);
        assert_eq!(l.file_count(), 0);
        assert!(!l.receive_files(7, reply("notes.txt"), 1.6));
        assert!(!l.launch(), "a fading-out file must not be opened");
    }

    #[test]
    fn keyboard_edges_pull_without_wrapping_and_settle_back() {
        let mut l = launcher();
        l.files = ["one", "two", "three"].into_iter().map(|name| crate::files::FileHit {
            path: std::path::Path::new("/tmp").join(name), name: name.into(),
            subtitle: String::new(), directory: false,
        }).collect();
        l.refresh(1.0);
        let first = l.selected.clone();
        l.move_selection(-1, 1.1);
        assert_eq!(l.selected, first);
        assert!(l.edge_offset(1.2) > 8.0);
        let ongoing = l.edge_offset(1.22);
        l.move_selection(-1, 1.22);
        assert!((l.edge_offset(1.22) - ongoing).abs() < 1e-6);
        assert!(l.edge_offset(1.32) > ongoing);
        assert!(l.edge_offset(2.2).abs() < 0.01);
        l.move_selection(1, 1.5);
        l.move_selection(1, 1.6);
        let last = l.selected.clone();
        assert_ne!(last, first);
        l.move_selection(1, 1.7);
        assert_eq!(l.selected, last);
        assert!(l.edge_offset(1.8) < -8.0);
        assert!(l.is_animating(1.8));
    }

    #[test]
    fn wheel_uses_the_same_resisted_pull_at_both_edges() {
        let mut l = launcher();
        l.files = (0..12).map(|i| crate::files::FileHit {
            path: std::path::Path::new("/tmp").join(format!("item-{i}")),
            name: format!("item-{i}"), subtitle: String::new(), directory: false,
        }).collect();
        l.refresh(1.0);
        assert!(l.scroll_by(-1.0, 1.1));
        assert_eq!(l.scroll.target(), 0.0);
        assert!(l.edge_offset(1.2) > 0.0);
        let first_pull = l.edge_pull.unwrap().peak;
        assert!(l.scroll_by(-1.0, 1.2));
        let second_pull = l.edge_pull.unwrap().peak;
        assert!(second_pull > first_pull && second_pull < EDGE_LIMIT);
        assert!(l.scroll_by(20.0, 2.0));
        assert_eq!(l.scroll.target(), l.max_scroll());
        assert!(l.edge_offset(2.1) < 0.0);
        assert!(l.scroll_by(1.0, 2.2));
        assert_eq!(l.scroll.target(), l.max_scroll());
        assert!(l.edge_offset(2.3) < 0.0);
    }

    #[test]
    fn control_backspace_removes_unicode_words_and_path_segments() {
        let mut l = launcher();
        l.push("Привет мир /tmp/файл", 1.0);
        l.delete_previous_word(1.1);
        assert_eq!(l.query, "Привет мир /tmp/");
        l.delete_previous_word(1.2);
        assert_eq!(l.query, "Привет мир /tmp");
        l.delete_previous_word(1.3);
        assert_eq!(l.query, "Привет мир /");
        l.delete_previous_word(1.4);
        assert_eq!(l.query, "Привет мир ");
        l.delete_previous_word(1.5);
        assert_eq!(l.query, "Привет ");
        l.delete_previous_word(1.6);
        assert!(l.query.is_empty());
    }

    #[test]
    fn launcher_close_target_stays_in_the_header() {
        assert!(Launcher::close_hit(PANEL_W - 32.0, 27.0, PANEL_W));
        assert!(!Launcher::close_hit(PANEL_W - 32.0, FIELD_TOP + 20.0, PANEL_W));
    }
}

fn file_icon(cr: &Context, directory: bool, x: f64, y: f64, size: f64, alpha: f64) {
    cr.save().unwrap();
    cr.translate(x + (ICON - size) / 2.0, y + (ICON - size) / 2.0);
    cr.scale(size / 38.0, size / 38.0);
    if directory {
        rounded(cr, 2.0, 7.0, 16.0, 11.0, 3.0);
        cr.set_source_rgba(0.19, 0.55, 0.85, alpha);
        let _ = cr.fill();
        rounded(cr, 2.0, 12.0, 34.0, 22.0, 4.0);
        cr.set_source_rgba(0.28, 0.69, 1.0, alpha);
        let _ = cr.fill();
    } else {
        cr.new_path();
        cr.move_to(8.0, 2.0);
        cr.line_to(23.0, 2.0);
        cr.line_to(31.0, 10.0);
        cr.line_to(31.0, 35.0);
        cr.line_to(8.0, 35.0);
        cr.close_path();
        cr.set_source_rgba(0.84, 0.88, 0.94, alpha);
        let _ = cr.fill();
        cr.move_to(23.0, 2.0);
        cr.line_to(23.0, 10.0);
        cr.line_to(31.0, 10.0);
        cr.set_source_rgba(0.43, 0.53, 0.65, alpha);
        let _ = cr.fill();
        cr.set_line_width(1.8);
        cr.set_line_cap(cairo::LineCap::Round);
        for y in [17.0, 22.0, 27.0] {
            cr.move_to(13.0, y);
            cr.line_to(26.0, y);
        }
        let _ = cr.stroke();
    }
    cr.restore().unwrap();
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
