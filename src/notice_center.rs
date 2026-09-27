//! Bounded, private notification history and its native island sheet.

use std::collections::VecDeque;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use cairo::Context;
use serde::{Deserialize, Serialize};

use crate::anim::{Spring, Time, min_jerk};
use crate::icons;
use crate::island::{Font, Renderer};
use crate::notify::Notification;

pub const WIDTH: f64 = 480.0;
pub const HEIGHT: f64 = 430.0;
const LIMIT: usize = 50;
const ROWS: usize = 4;
const ROW_TOP: f64 = 72.0;
const ROW_H: f64 = 72.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub serial: u64,
    pub note: Notification,
    pub received: u64,
    pub read: bool,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Close,
    Dismiss(u32),
    Invoke(u32, String),
    DismissMany(Vec<u32>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit { Close, Clear, Back, Row(usize), Remove(usize), DetailRemove, Invoke(usize) }

pub struct Center {
    entries: VecDeque<Entry>,
    next_serial: u64,
    pub open: bool,
    pub opened_at: Time,
    offset: usize,
    detail: Option<u64>,
    detail_scroll: f64,
    hovered: Option<Hit>,
    row_hover: [Spring; ROWS],
    button_hover: [Spring; 3],
    writer: Option<mpsc::Sender<Vec<Entry>>>,
}

fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(||
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(base.join("island-shell/notifications.json"))
}

fn save(path: &PathBuf, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
    }
    let encoded = serde_json::to_vec(entries)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&tmp)?;
    file.write_all(&encoded)?;
    file.sync_data()?;
    std::fs::rename(tmp, path)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Center {
    pub fn new() -> Self {
        if cfg!(test) { return Self::with_entries(VecDeque::new(), None); }
        let path = state_path();
        let entries: VecDeque<Entry> = path.as_ref().and_then(|path| std::fs::read(path).ok())
            .and_then(|data| serde_json::from_slice::<Vec<Entry>>(&data).ok())
            .unwrap_or_default().into_iter().take(LIMIT).map(|mut e| { e.active = false; e }).collect();
        let writer = path.map(|path| {
            let (tx, rx) = mpsc::channel::<Vec<Entry>>();
            std::thread::Builder::new().name("notification-history".into()).spawn(move || {
                while let Ok(mut entries) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() { entries = newer; }
                    if let Err(error) = save(&path, &entries) { eprintln!("island-shell: notification history: {error}"); }
                }
            }).expect("spawn history writer");
            tx
        });
        Self::with_entries(entries, writer)
    }

    fn with_entries(entries: VecDeque<Entry>, writer: Option<mpsc::Sender<Vec<Entry>>>) -> Self {
        let next_serial = entries.iter().map(|e| e.serial).max().unwrap_or(0).saturating_add(1);
        Self {
            entries, next_serial, open: false, opened_at: 0.0, offset: 0,
            detail: None, detail_scroll: 0.0, hovered: None,
            row_hover: std::array::from_fn(|_| Spring::new(0.0, 0.22, 0.85)),
            button_hover: std::array::from_fn(|_| Spring::new(0.0, 0.22, 0.85)),
            writer,
        }
    }

    fn persist(&self) {
        if let Some(tx) = &self.writer { let _ = tx.send(self.entries.iter().cloned().collect()); }
    }

    pub fn unread(&self) -> usize { self.entries.iter().filter(|e| !e.read).count() }
    pub fn len(&self) -> usize { self.entries.len() }

    pub fn upsert(&mut self, mut note: Notification) {
        if note.transient { return; }
        note.app = note.app.chars().take(100).collect();
        note.summary = note.summary.chars().take(240).collect();
        note.body = note.body.chars().take(2000).collect();
        let existing = self.entries.iter().position(|e| e.active && note.id != 0 && e.note.id == note.id)
            .or_else(|| note.tag.as_ref().and_then(|tag| self.entries.iter().position(|e| e.note.tag.as_ref() == Some(tag))));
        if let Some(index) = existing {
            let mut entry = self.entries.remove(index).unwrap();
            entry.note = note;
            entry.received = now_secs();
            entry.read = false;
            entry.active = entry.note.id != 0;
            self.entries.push_front(entry);
        } else {
            self.entries.push_front(Entry { serial: self.next_serial, active: note.id != 0,
                note, received: now_secs(), read: false });
            self.next_serial = self.next_serial.saturating_add(1);
        }
        self.entries.truncate(LIMIT);
        self.offset = self.offset.min(self.entries.len().saturating_sub(ROWS));
        self.persist();
    }

    pub fn closed(&mut self, id: u32) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.active && e.note.id == id) {
            entry.active = false;
            self.persist();
        }
    }

    pub fn open(&mut self, now: Time) {
        self.open = true;
        self.opened_at = now;
        self.offset = 0;
        self.detail = None;
        self.hovered = None;
    }

    pub fn close(&mut self) { self.open = false; self.detail = None; self.hovered = None; }

    pub fn is_animating(&self, now: Time) -> bool {
        self.open && (now - self.opened_at < 0.5 || self.row_hover.iter().any(|s| s.is_moving(now))
            || self.button_hover.iter().any(|s| s.is_moving(now)))
    }

    fn hit(&self, x: f64, y: f64) -> Option<Hit> {
        if !(0.0..WIDTH).contains(&x) || !(0.0..HEIGHT).contains(&y) { return None; }
        if x >= WIDTH - 52.0 && y < 58.0 { return Some(Hit::Close); }
        if self.detail.is_some() {
            if x < 86.0 && y < 58.0 { return Some(Hit::Back); }
            if x >= WIDTH - 110.0 && y >= HEIGHT - 68.0 { return Some(Hit::DetailRemove); }
            if y >= HEIGHT - 68.0 && x >= 24.0 && x < WIDTH - 120.0 {
                return Some(Hit::Invoke(((x - 24.0) / 154.0).floor() as usize));
            }
            return None;
        }
        if x >= WIDTH - 143.0 && y < 58.0 { return Some(Hit::Clear); }
        if y >= ROW_TOP && y < ROW_TOP + ROW_H * ROWS as f64 {
            let row = ((y - ROW_TOP) / ROW_H) as usize;
            if self.entries.get(self.offset + row).is_some() {
                return Some(if x >= WIDTH - 55.0 { Hit::Remove(row) } else { Hit::Row(row) });
            }
        }
        None
    }

    pub fn pointer(&mut self, x: f64, y: f64, now: Time) -> bool {
        let hit = self.hit(x, y);
        if self.hovered == hit { return false; }
        self.hovered = hit;
        for (i, spring) in self.row_hover.iter_mut().enumerate() {
            spring.set(if matches!(hit, Some(Hit::Row(j) | Hit::Remove(j)) if i == j) { 1.0 } else { 0.0 }, now);
        }
        for (i, spring) in self.button_hover.iter_mut().enumerate() {
            spring.set(if matches!((i, hit), (0, Some(Hit::Close)) | (1, Some(Hit::Clear | Hit::Back)) | (2, Some(Hit::DetailRemove | Hit::Invoke(_)))) { 1.0 } else { 0.0 }, now);
        }
        true
    }

    pub fn scroll(&mut self, rows: f64) -> bool {
        if self.detail.is_some() {
            let old = self.detail_scroll;
            self.detail_scroll = (old + rows * 32.0).clamp(0.0, 600.0);
            return old != self.detail_scroll;
        }
        let old = self.offset;
        self.offset = (self.offset as f64 + rows).round().clamp(0.0, self.entries.len().saturating_sub(ROWS) as f64) as usize;
        old != self.offset
    }

    pub fn click(&mut self, x: f64, y: f64) -> Action {
        match self.hit(x, y) {
            Some(Hit::Close) => { self.close(); Action::Close }
            Some(Hit::Clear) => {
                let ids = self.entries.iter().filter(|e| e.active).map(|e| e.note.id).collect();
                self.entries.clear(); self.offset = 0; self.persist(); Action::DismissMany(ids)
            }
            Some(Hit::Back) => { self.detail = None; self.detail_scroll = 0.0; Action::None }
            Some(Hit::Row(row)) => {
                if let Some(entry) = self.entries.get_mut(self.offset + row) {
                    entry.read = true;
                    self.detail = Some(entry.serial);
                    self.detail_scroll = 0.0;
                    self.persist();
                }
                Action::None
            }
            Some(Hit::Remove(row)) => self.remove(self.offset + row),
            Some(Hit::DetailRemove) => {
                let index = self.entries.iter().position(|e| Some(e.serial) == self.detail);
                self.detail = None;
                index.map_or(Action::None, |i| self.remove(i))
            }
            Some(Hit::Invoke(index)) => {
                let Some(entry) = self.entries.iter().find(|e| Some(e.serial) == self.detail) else { return Action::None };
                if !entry.active { return Action::None; }
                let actions = visible_actions(&entry.note);
                actions.get(index).map_or(Action::None, |(key, _)| Action::Invoke(entry.note.id, key.clone()))
            }
            None => Action::None,
        }
    }

    fn remove(&mut self, index: usize) -> Action {
        let Some(entry) = self.entries.remove(index) else { return Action::None };
        self.offset = self.offset.min(self.entries.len().saturating_sub(ROWS));
        self.persist();
        if entry.active { Action::Dismiss(entry.note.id) } else { Action::None }
    }

    pub fn draw(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, now: Time) {
        let title = if self.detail.is_some() { "Уведомление" } else { "Уведомления" };
        let title_x = if self.detail.is_some() { x + 121.0 } else { x + 24.0 };
        label(cr, renderer, title, title_x, y + 17.0, WIDTH - (title_x - x) - 62.0, Font::Text, alpha);
        if self.detail.is_some() {
            button(cr, renderer, "‹ Назад", x + 22.0, y + 8.0, 85.0, 42.0, alpha, self.button_hover[1].value(now));
        } else {
            let unread = self.unread();
            let count = if unread > 0 { format!("{unread} новых") } else { format!("{} всего", self.len()) };
            label(cr, renderer, &count, x + 25.0, y + 47.0, 200.0, Font::Small, alpha * 0.55);
            if !self.entries.is_empty() {
                button(cr, renderer, "Очистить", x + WIDTH - 145.0, y + 9.0, 91.0, 38.0, alpha, self.button_hover[1].value(now));
            }
        }
        button(cr, renderer, "×", x + WIDTH - 49.0, y + 10.0, 32.0, 32.0, alpha, self.button_hover[0].value(now));
        if let Some(serial) = self.detail {
            self.draw_detail(cr, renderer, x, y, alpha, serial, now);
        } else {
            self.draw_list(cr, renderer, x, y, alpha, now);
        }
    }

    fn draw_list(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, now: Time) {
        if self.entries.is_empty() {
            label(cr, renderer, "Пока пусто", x + 24.0, y + 162.0, WIDTH - 48.0, Font::Text, alpha * 0.88);
            label(cr, renderer, "Новые сообщения появятся здесь", x + 24.0, y + 194.0, WIDTH - 48.0, Font::Small, alpha * 0.48);
            return;
        }
        cr.save().unwrap();
        cr.rectangle(x + 14.0, y + ROW_TOP, WIDTH - 28.0, ROW_H * ROWS as f64);
        cr.clip();
        for (row, entry) in self.entries.iter().skip(self.offset).take(ROWS).enumerate() {
            let delay = row as f64 * 0.04;
            let k = min_jerk(((now - self.opened_at - delay) / 0.38).clamp(0.0, 1.0));
            let top = y + ROW_TOP + row as f64 * ROW_H + (1.0 - k) * 15.0;
            let opacity = alpha * k;
            let hover = self.row_hover[row].value(now);
            rounded(cr, x + 16.0, top + 2.0, WIDTH - 32.0, ROW_H - 5.0, 15.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, (0.055 + 0.085 * hover) * opacity);
            let _ = cr.fill();
            icons::draw(cr, entry.note.kind, x + 43.0, top + 35.0, 39.0, (now - self.opened_at - delay).max(0.0), opacity);
            label(cr, renderer, &entry.note.app, x + 70.0, top + 10.0, 250.0, Font::Caption, opacity * 0.91);
            label(cr, renderer, &entry.note.message(), x + 70.0, top + 35.0, WIDTH - 145.0, Font::Small, opacity * 0.63);
            if !entry.read {
                cr.arc(x + WIDTH - 72.0, top + 23.0, 3.0, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(0.40, 0.82, 1.0, opacity);
                let _ = cr.fill();
            }
            label(cr, renderer, "×", x + WIDTH - 47.0, top + 19.0, 24.0, Font::Text, opacity * (0.43 + 0.35 * hover));
        }
        cr.restore().unwrap();
        if self.entries.len() > ROWS {
            let text = format!("{}–{} из {}  ·  прокрутка", self.offset + 1, (self.offset + ROWS).min(self.entries.len()), self.entries.len());
            label(cr, renderer, &text, x + 24.0, y + HEIGHT - 32.0, WIDTH - 48.0, Font::Small, alpha * 0.43);
        } else {
            label(cr, renderer, "Нажмите, чтобы прочитать", x + 24.0, y + HEIGHT - 32.0, WIDTH - 48.0, Font::Small, alpha * 0.43);
        }
    }

    fn draw_detail(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, serial: u64, now: Time) {
        let Some(entry) = self.entries.iter().find(|e| e.serial == serial) else { return };
        icons::draw(cr, entry.note.kind, x + 49.0, y + 102.0, 48.0, (now - self.opened_at).max(0.0), alpha);
        label(cr, renderer, &entry.note.app, x + 83.0, y + 77.0, WIDTH - 108.0, Font::Caption, alpha * 0.65);
        label(cr, renderer, &entry.note.summary, x + 83.0, y + 103.0, WIDTH - 108.0, Font::Text, alpha);
        cr.save().unwrap();
        cr.rectangle(x + 24.0, y + 149.0, WIDTH - 48.0, 197.0);
        cr.clip();
        let text = renderer.layout(cr, &entry.note.body, Font::Caption).copy();
        text.set_width(((WIDTH - 48.0) * pango::SCALE as f64) as i32);
        text.set_wrap(pango::WrapMode::WordChar);
        cr.set_source_rgba(1.0, 1.0, 1.0, alpha * 0.78);
        cr.move_to(renderer.snap(x + 24.0), renderer.snap(y + 150.0 - self.detail_scroll));
        pangocairo::functions::show_layout(cr, &text);
        cr.restore().unwrap();
        for (index, (_, text)) in visible_actions(&entry.note).iter().enumerate() {
            button(cr, renderer, text, x + 24.0 + index as f64 * 154.0, y + HEIGHT - 62.0, 144.0, 42.0, alpha, self.button_hover[2].value(now));
        }
        button(cr, renderer, "Удалить", x + WIDTH - 112.0, y + HEIGHT - 62.0, 88.0, 42.0, alpha, self.button_hover[2].value(now));
    }
}

fn visible_actions(note: &Notification) -> Vec<&(String, String)> {
    let mut actions: Vec<_> = note.actions.iter().filter(|(key, _)| key == "default").take(1).collect();
    actions.extend(note.actions.iter().filter(|(key, _)| key != "default").take(1));
    actions
}

fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

fn label(cr: &Context, renderer: &Renderer, text: &str, x: f64, y: f64, width: f64, font: Font, alpha: f64) {
    let layout = renderer.layout(cr, text, font).copy();
    layout.set_width((width * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.move_to(renderer.snap(x), renderer.snap(y));
    pangocairo::functions::show_layout(cr, &layout);
}

fn button(cr: &Context, renderer: &Renderer, text: &str, x: f64, y: f64, w: f64, h: f64, alpha: f64, hover: f64) {
    rounded(cr, x, y, w, h, h / 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, (0.085 + 0.12 * hover) * alpha);
    let _ = cr.fill();
    label(cr, renderer, text, x + 12.0, y + 10.0, w - 24.0, Font::Small, alpha * (0.72 + 0.26 * hover));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn center() -> Center { Center::with_entries(VecDeque::new(), None) }
    #[test]
    fn history_replaces_active_id_and_keeps_only_fifty() {
        let mut center = center();
        for id in 1..=52 {
            let mut note = crate::notify::fabricate("System", &id.to_string(), "body");
            note.id = id;
            center.upsert(note);
        }
        assert_eq!(center.len(), LIMIT);
        let mut replacement = crate::notify::fabricate("System", "updated", "body");
        replacement.id = 52;
        center.upsert(replacement);
        assert_eq!(center.len(), LIMIT);
        assert_eq!(center.entries.front().unwrap().note.summary, "updated");
        center.closed(52);
        assert!(!center.entries.front().unwrap().active);
    }

    #[test]
    fn reading_and_removing_a_notification_updates_unread_state() {
        let mut center = center();
        let mut note = crate::notify::fabricate("System", "title", "body");
        note.id = 5;
        note.actions = vec![("default".into(), "Открыть".into())];
        center.upsert(note);
        center.open(0.0);
        assert_eq!(center.unread(), 1);
        assert_eq!(center.click(90.0, ROW_TOP + 12.0), Action::None);
        assert_eq!(center.unread(), 0);
        assert_eq!(center.click(35.0, HEIGHT - 40.0), Action::Invoke(5, "default".into()));
        assert_eq!(center.click(WIDTH - 40.0, HEIGHT - 40.0), Action::Dismiss(5));
        assert_eq!(center.len(), 0);
    }
}
