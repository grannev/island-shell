//! Bounded, private notification history and its native island sheet.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use cairo::Context;
use serde::{Deserialize, Serialize};

use crate::anim::{Glide, Spring, Time, min_jerk};
use crate::icons;
use crate::iconload::Icons;
use crate::island::{Font, Renderer};
use crate::menu_style::{self, ButtonKind, RAIL};
use crate::notify::Notification;

pub const WIDTH: f64 = 480.0;
/// a full list and a notification's details share one height
pub const HEIGHT: f64 = ROW_TOP + ROWS as f64 * ROW_H + LIST_BOTTOM;
const LIMIT: usize = 50;
const ROWS: usize = 4;
const ROW_TOP: f64 = menu_style::CONTENT_TOP;
pub const ROW_H: f64 = 72.0;
/// below the last card: the sheet's bottom rail less the cards' own gap
const LIST_BOTTOM: f64 = 16.0;
const ROW_MOTION: Time = 0.42;
/// the wheel's glide (see anim::Glide)
const SCROLL_TAU: Time = 0.08;
const INDICATOR_HOLD: Time = 0.8;
const EXIT_MOTION: Time = 0.32;
const VIEW_MOTION: Time = 0.28;

struct Leaving {
    entry: Entry,
    count: usize,
    unread: usize,
    child: bool,
    row: f64,
    since: Time,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum RowKey { Entry(u64), Group(String) }

struct ViewRow<'a> {
    key: RowKey,
    entry: &'a Entry,
    count: usize,
    unread: usize,
    child: bool,
}

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
    /// the list position in rows (fractional: the wheel glides, it does
    /// not step); rows and leaving rows live in list coordinates
    scroll: Glide,
    scrolled_at: Time,
    detail: Option<u64>,
    view_transition: Option<(Option<u64>, Time)>,
    detail_scroll: f64,
    expanded: HashSet<String>,
    hovered: Option<Hit>,
    row_hover: HashMap<RowKey, Spring>,
    button_hover: [Spring; 5],
    row_from: HashMap<RowKey, f64>,
    row_motion_at: Time,
    born_at: HashMap<u64, Time>,
    leaving: Vec<Leaving>,
    writer: Option<mpsc::Sender<Vec<Entry>>>,
}

fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(||
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(base.join("island-shell/notifications.json"))
}

fn prune_saved_images(entries: &VecDeque<Entry>, state: &PathBuf) {
    let dir = state.with_file_name("images");
    let keep: HashSet<_> = entries.iter().filter_map(|entry| entry.note.image.as_ref()).collect();
    let Ok(files) = std::fs::read_dir(dir) else { return };
    for file in files.flatten() {
        let path = file.path();
        if path.extension().is_some_and(|ext| ext == "png")
            && path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.') )
            && file.file_type().is_ok_and(|kind| kind.is_file())
            && !keep.contains(&path.to_string_lossy().into_owned()) {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn load_dnd() -> bool {
    if cfg!(test) { return false; }
    state_path().and_then(|path| std::fs::read_to_string(path.with_file_name("dnd")).ok())
        .is_some_and(|value| value.trim() == "on")
}

pub fn save_dnd(enabled: bool) {
    if cfg!(test) { return; }
    let Some(path) = state_path().map(|path| path.with_file_name("dnd")) else { return };
    if let Some(parent) = path.parent() {
        if std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent).is_err() { return; }
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(path) {
        let _ = file.write_all(if enabled { b"on\n" } else { b"off\n" });
    }
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
        if let Some(path) = path.as_ref() { prune_saved_images(&entries, path); }
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
            entries, next_serial, open: false, opened_at: 0.0, scroll: Glide::new(0.0, SCROLL_TAU), scrolled_at: f64::NEG_INFINITY,
            detail: None, view_transition: None, detail_scroll: 0.0, expanded: HashSet::new(), hovered: None,
            row_hover: HashMap::new(),
            button_hover: std::array::from_fn(|_| Spring::new(0.0, 0.22, 0.85)),
            row_from: HashMap::new(), row_motion_at: 0.0,
            born_at: HashMap::new(), leaving: Vec::new(),
            writer,
        }
    }

    fn persist(&self) {
        if let Some(tx) = &self.writer { let _ = tx.send(self.entries.iter().cloned().collect()); }
    }

    pub fn unread(&self) -> usize { self.entries.iter().filter(|e| !e.read).count() }
    pub fn len(&self) -> usize { self.entries.len() }

    pub fn height(&self) -> f64 {
        if self.detail.is_some() { HEIGHT } else { self.list_height() }
    }

    fn list_height(&self) -> f64 {
        if self.entries.is_empty() { return ROW_TOP + ROW_H * 3.0 + LIST_BOTTOM; }
        let top = self.scroll.target();
        let visible = self.leaving.iter().map(|row| (row.row - top).ceil().max(0.0) as usize + 1).max().unwrap_or(0);
        let moving = self.row_from.values().map(|row| (row - top).ceil().max(0.0) as usize + 1).max().unwrap_or(0);
        ROW_TOP + ROW_H * self.rows().len().max(visible).max(moving).clamp(1, ROWS) as f64 + LIST_BOTTOM
    }

    fn rows(&self) -> Vec<ViewRow<'_>> {
        let mut groups: HashMap<&str, Vec<&Entry>> = HashMap::new();
        for entry in &self.entries {
            groups.entry(&entry.note.app).or_default().push(entry);
        }
        let mut seen = HashSet::new();
        let mut rows = Vec::with_capacity(self.entries.len() + groups.len());
        for entry in &self.entries {
            let app = entry.note.app.as_str();
            if !seen.insert(app) { continue; }
            let group = &groups[app];
            if group.len() == 1 {
                rows.push(ViewRow { key: RowKey::Entry(entry.serial), entry, count: 1,
                    unread: usize::from(!entry.read), child: false });
            } else {
                rows.push(ViewRow { key: RowKey::Group(app.to_string()), entry,
                    count: group.len(), unread: group.iter().filter(|e| !e.read).count(), child: false });
                if self.expanded.contains(app) {
                    rows.extend(group.iter().map(|entry| ViewRow {
                        key: RowKey::Entry(entry.serial), entry, count: 1,
                        unread: usize::from(!entry.read), child: true,
                    }));
                }
            }
        }
        rows
    }

    fn max_scroll(&self) -> f64 {
        self.rows().len().saturating_sub(ROWS) as f64
    }

    /// Keep the list within its rows after it shrank, gliding back.
    fn clamp_scroll(&mut self, now: Time) {
        let max = self.max_scroll();
        if self.scroll.target() > max { self.scroll.set(max, now); }
    }

    /// Is a row (in list coordinates) inside the viewport?
    fn in_view(&self, row: f64) -> bool {
        let top = self.scroll.target();
        row > top - 1.0 && row < top + ROWS as f64
    }

    pub fn upsert(&mut self, mut note: Notification, now: Time) {
        if note.transient { return; }
        note.app = note.app.chars().take(100).collect();
        note.summary = note.summary.chars().take(240).collect();
        note.body = note.body.chars().take(2000).collect();
        let existing = self.entries.iter().position(|e| e.active && note.id != 0 && e.note.id == note.id)
            .or_else(|| note.tag.as_ref().and_then(|tag| self.entries.iter().position(|e| e.note.tag.as_ref() == Some(tag))));
        let reorder = existing != Some(0);
        if self.open && reorder { self.capture_rows(now); }
        if let Some(index) = existing {
            let mut entry = self.entries.remove(index).unwrap();
            entry.note = note;
            entry.received = now_secs();
            entry.read = false;
            entry.active = entry.note.id != 0;
            self.entries.push_front(entry);
        } else {
            if self.open { self.born_at.insert(self.next_serial, now); }
            self.entries.push_front(Entry { serial: self.next_serial, active: note.id != 0,
                note, received: now_secs(), read: false });
            self.next_serial = self.next_serial.saturating_add(1);
        }
        self.entries.truncate(LIMIT);
        if self.open && reorder { self.scroll.set(0.0, now); } else { self.clamp_scroll(now); }
        self.persist();
    }

    fn row_position(&self, key: &RowKey, target: f64, now: Time) -> f64 {
        let k = min_jerk(((now - self.row_motion_at) / ROW_MOTION).clamp(0.0, 1.0));
        self.row_from.get(key).map_or(target, |from| from + (target - from) * k)
    }

    fn capture_rows(&mut self, now: Time) {
        let positions: HashMap<_, _> = self.rows().iter().enumerate().map(|(i, row)| {
            (row.key.clone(), self.row_position(&row.key, i as f64, now))
        }).collect();
        self.row_from = positions;
        self.row_motion_at = now;
    }

    pub fn advance(&mut self, now: Time) {
        self.leaving.retain(|row| now - row.since < EXIT_MOTION);
        self.born_at.retain(|_, since| now - *since < ROW_MOTION);
        if now - self.row_motion_at >= ROW_MOTION { self.row_from.clear(); }
        if self.view_transition.is_some_and(|(_, since)| now - since >= VIEW_MOTION) {
            self.view_transition = None;
        }
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
        self.scroll.reset(0.0);
        self.row_hover.clear();
        self.detail = None;
        self.view_transition = None;
        self.expanded.clear();
        self.hovered = None;
        self.row_from.clear();
        self.born_at.clear();
        self.leaving.clear();
    }

    pub fn close(&mut self) {
        self.open = false; self.detail = None; self.view_transition = None; self.hovered = None;
        self.row_from.clear(); self.born_at.clear(); self.leaving.clear();
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.open && (now - self.opened_at < 0.5 || self.row_hover.values().any(|s| s.is_moving(now))
            || self.scroll.is_moving(now) || now - self.scrolled_at < INDICATOR_HOLD + 0.3
            || self.button_hover.iter().any(|s| s.is_moving(now))
            || !self.leaving.is_empty() || !self.born_at.is_empty()
            || !self.row_from.is_empty()
            || self.view_transition.is_some_and(|(_, since)| now - since < VIEW_MOTION))
    }

    fn hit(&self, x: f64, y: f64) -> Option<Hit> {
        if !(0.0..WIDTH).contains(&x) || !(0.0..self.height()).contains(&y) { return None; }
        if x >= WIDTH - 54.0 && y < 58.0 { return Some(Hit::Close); }
        if self.detail.is_some() {
            if x < 107.0 && y < 58.0 { return Some(Hit::Back); }
            if x >= WIDTH - 110.0 && y >= HEIGHT - 68.0 { return Some(Hit::DetailRemove); }
            if y >= HEIGHT - 68.0 && x >= 22.0 && x < WIDTH - 120.0 {
                return Some(Hit::Invoke(((x - 22.0) / 154.0).floor() as usize));
            }
            return None;
        }
        if x >= WIDTH - 156.0 && x < WIDTH - 62.0 && y < 58.0 { return Some(Hit::Clear); }
        if y >= ROW_TOP && y < ROW_TOP + ROW_H * ROWS as f64 {
            let index = ((y - ROW_TOP) / ROW_H + self.scroll.target()).floor();
            // the gap between two cards belongs to neither
            let within = (y - ROW_TOP) / ROW_H + self.scroll.target() - index;
            if index >= 0.0 && within * ROW_H < ROW_H - 6.0 && self.rows().get(index as usize).is_some() {
                let row = index as usize;
                return Some(if x >= WIDTH - 55.0 { Hit::Remove(row) } else { Hit::Row(row) });
            }
        }
        None
    }

    pub fn pointer(&mut self, x: f64, y: f64, now: Time) -> bool {
        let hit = self.hit(x, y);
        if self.hovered == hit { return false; }
        self.hovered = hit;
        self.set_row_hover(now);
        for (i, spring) in self.button_hover.iter_mut().enumerate() {
            spring.set(if matches!((i, hit),
                (0, Some(Hit::Close)) | (1, Some(Hit::Clear | Hit::Back))
                | (2, Some(Hit::Invoke(0))) | (3, Some(Hit::Invoke(1)))
                | (4, Some(Hit::DetailRemove))) { 1.0 } else { 0.0 }, now);
        }
        true
    }

    /// Light the card under the pointer, whichever row it has scrolled to.
    fn set_row_hover(&mut self, now: Time) {
        let lit = match self.hovered {
            Some(Hit::Row(index) | Hit::Remove(index)) => self.rows().get(index).map(|view| view.key.clone()),
            _ => None,
        };
        for (key, spring) in self.row_hover.iter_mut() {
            if Some(key) != lit.as_ref() { spring.set(0.0, now); }
        }
        if let Some(key) = lit {
            self.row_hover.entry(key).or_insert_with(|| Spring::new(0.0, 0.22, 0.85)).set(1.0, now);
        }
        self.row_hover.retain(|_, spring| spring.target() > 0.0 || spring.is_moving(now));
    }

    /// The scroll indicator's opacity: up while scrolling, gone a moment
    /// after the list comes to rest.
    fn indicator(&self, now: Time) -> f64 {
        let since = now - self.scrolled_at;
        if self.scroll.is_moving(now) || since < INDICATOR_HOLD { 1.0 }
        else { 1.0 - ((since - INDICATOR_HOLD) / 0.3).clamp(0.0, 1.0) }
    }

    /// Scroll by `rows` (fractional): the list glides, it does not step.
    pub fn scroll(&mut self, rows: f64, now: Time) -> bool {
        if self.detail.is_some() {
            let old = self.detail_scroll;
            self.detail_scroll = (old + rows * ROW_H).clamp(0.0, 600.0);
            return old != self.detail_scroll;
        }
        let next = (self.scroll.target() + rows).clamp(0.0, self.max_scroll());
        if (next - self.scroll.target()).abs() < 1e-9 { return false; }
        self.scroll.set_with(next, SCROLL_TAU, now);
        self.scrolled_at = now;
        true
    }

    pub fn click(&mut self, x: f64, y: f64, now: Time) -> Action {
        match self.hit(x, y) {
            Some(Hit::Close) => { self.close(); Action::Close }
            Some(Hit::Clear) => {
                let ids = self.entries.iter().filter(|e| e.active).map(|e| e.note.id).collect();
                self.capture_rows(now);
                let leaving: Vec<_> = self.rows().iter().filter_map(|view| {
                    let row = *self.row_from.get(&view.key)?;
                    self.in_view(row).then(|| Leaving {
                        entry: view.entry.clone(), count: view.count, unread: view.unread,
                        child: view.child, row, since: now,
                    })
                }).collect();
                self.leaving.extend(leaving);
                self.entries.clear(); self.scroll.set(0.0, now); self.persist(); Action::DismissMany(ids)
            }
            Some(Hit::Back) => {
                self.view_transition = Some((self.detail, now));
                self.detail = None;
                self.detail_scroll = 0.0;
                Action::None
            }
            Some(Hit::Row(row)) => {
                if let Some(view) = self.rows().get(row) {
                    if view.count > 1 {
                        let app = view.entry.note.app.clone();
                        self.capture_rows(now);
                        if self.expanded.contains(&app) {
                            let leaving: Vec<_> = self.rows().iter().filter(|view| view.child && view.entry.note.app == app)
                                .filter_map(|view| {
                                    let row = *self.row_from.get(&view.key)?;
                                    self.in_view(row).then(|| Leaving {
                                        entry: view.entry.clone(), count: 1, unread: view.unread,
                                        child: true, row, since: now,
                                    })
                                }).collect();
                            self.leaving.extend(leaving);
                            self.expanded.remove(&app);
                        } else {
                            self.expanded.insert(app.clone());
                            for entry in self.entries.iter().filter(|entry| entry.note.app == app) {
                                self.born_at.insert(entry.serial, now);
                            }
                        }
                        self.clamp_scroll(now);
                        return Action::None;
                    }
                    let serial = view.entry.serial;
                    if let Some(entry) = self.entries.iter_mut().find(|entry| entry.serial == serial) {
                        entry.read = true;
                        self.view_transition = Some((None, now));
                        self.detail = Some(serial);
                        self.detail_scroll = 0.0;
                        self.persist();
                    }
                }
                Action::None
            }
            Some(Hit::Remove(row)) => {
                let rows = self.rows();
                let Some(view) = rows.get(row) else { return Action::None };
                if view.count > 1 { self.remove_group(&view.entry.note.app.clone(), now) }
                else { self.remove_serial(view.entry.serial, now) }
            }
            Some(Hit::DetailRemove) => {
                let index = self.entries.iter().position(|e| Some(e.serial) == self.detail);
                self.detail = None;
                index.map_or(Action::None, |i| self.remove(i, now))
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

    fn remove(&mut self, index: usize, now: Time) -> Action {
        let serial = match self.entries.get(index) { Some(entry) => entry.serial, None => return Action::None };
        self.capture_rows(now);
        let view = self.rows().into_iter().find(|row| row.key == RowKey::Entry(serial)).map(|view| {
            let row = self.row_from.get(&view.key).copied();
            (view.entry.clone(), view.count, view.unread, view.child, row)
        });
        let Some(entry) = self.entries.remove(index) else { return Action::None };
        if self.open && self.detail.is_none() {
            if let Some((_, count, unread, child, Some(row))) = view {
                if self.in_view(row) {
                    self.leaving.push(Leaving { entry: entry.clone(), count, unread, child, row, since: now });
                }
            }
        }
        self.born_at.remove(&entry.serial);
        self.clamp_scroll(now);
        self.persist();
        if entry.active { Action::Dismiss(entry.note.id) } else { Action::None }
    }

    fn remove_serial(&mut self, serial: u64, now: Time) -> Action {
        self.entries.iter().position(|entry| entry.serial == serial)
            .map_or(Action::None, |index| self.remove(index, now))
    }

    fn remove_group(&mut self, app: &str, now: Time) -> Action {
        self.capture_rows(now);
        let leaving: Vec<_> = self.rows().iter().filter(|view| view.entry.note.app == app).filter_map(|view| {
            let row = *self.row_from.get(&view.key)?;
            self.in_view(row).then(|| Leaving {
                entry: view.entry.clone(), count: view.count, unread: view.unread,
                child: view.child, row, since: now,
            })
        }).collect();
        self.leaving.extend(leaving);
        let ids = self.entries.iter().filter(|entry| entry.note.app == app && entry.active)
            .map(|entry| entry.note.id).collect();
        self.entries.retain(|entry| entry.note.app != app);
        self.expanded.remove(app);
        self.clamp_scroll(now);
        self.persist();
        Action::DismissMany(ids)
    }

    pub fn draw(&self, cr: &Context, renderer: &Renderer, media: &Icons, x: f64, y: f64, alpha: f64, now: Time) {
        if let Some((previous, since)) = self.view_transition {
            let k = min_jerk(((now - since) / VIEW_MOTION).clamp(0.0, 1.0));
            if k < 1.0 {
                self.draw_view_composited(cr, renderer, media, x, y - 9.0 * k, alpha * (1.0 - k), previous, now);
                self.draw_view_composited(cr, renderer, media, x, y + 9.0 * (1.0 - k), alpha * k, self.detail, now);
                return;
            }
        }
        self.draw_view_composited(cr, renderer, media, x, y, alpha, self.detail, now);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_view_composited(&self, cr: &Context, renderer: &Renderer, media: &Icons,
        x: f64, y: f64, opacity: f64, detail: Option<u64>, now: Time) {
        if opacity <= 0.001 { return; }
        let grouped = opacity < 0.999;
        if grouped {
            cr.save().unwrap();
            cr.rectangle(x, y, WIDTH, if detail.is_some() { HEIGHT } else { self.list_height() });
            cr.clip();
            cr.push_group();
        }
        self.draw_view(cr, renderer, media, x, y, detail, now);
        if grouped {
            cr.pop_group_to_source().unwrap();
            let _ = cr.paint_with_alpha(opacity);
            cr.restore().unwrap();
        }
    }

    fn draw_view(&self, cr: &Context, renderer: &Renderer, media: &Icons,
        x: f64, y: f64, detail: Option<u64>, now: Time) {
        if detail.is_some() {
            // a navigation bar: back on the left, the title centred
            let back = self.button_hover[1].value(now);
            menu_style::chevron(cr, x + RAIL + 5.0, y + menu_style::HEADER_MID, 7.0, std::f64::consts::PI,
                (0.8 + 0.2 * back).min(1.0));
            menu_style::text(cr, renderer, "Назад", x + RAIL + 16.0,
                y + menu_style::HEADER_MID - menu_style::line_height(cr, renderer, Font::Body) / 2.0,
                80.0, Font::Body, menu_style::BLUE, 0.85 + 0.15 * back);
            menu_style::centred(cr, renderer, "Уведомление", (x + 110.0, y + menu_style::HEADER_MID - 12.0, WIDTH - 220.0, 24.0),
                Font::Title, (1.0, 1.0, 1.0), 1.0);
        } else {
            let unread = self.unread();
            let count = match (unread, self.len()) {
                (0, 0) => "Нет уведомлений".to_string(),
                (0, n) => format!("Всего {n}"),
                (n, _) => format!("{n} новых"),
            };
            menu_style::header(cr, renderer, x, y, WIDTH, None, "Уведомления", Some(&count), 170.0, 1.0);
            if !self.entries.is_empty() {
                menu_style::button(cr, renderer, "Очистить",
                    (x + WIDTH - 154.0, y + menu_style::HEADER_MID - menu_style::SMALL_BUTTON_H / 2.0, 92.0, menu_style::SMALL_BUTTON_H),
                    ButtonKind::Neutral, self.button_hover[1].value(now), 1.0);
            }
        }
        let (cx, cy, size) = menu_style::close_rect(WIDTH);
        menu_style::close(cr, x + cx, y + cy, size, self.button_hover[0].value(now), 1.0);
        if let Some(serial) = detail {
            self.draw_detail(cr, renderer, media, x, y, 1.0, serial, now);
        } else {
            self.draw_list(cr, renderer, media, x, y, self.list_height(), 1.0, now);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_list(&self, cr: &Context, renderer: &Renderer, media: &Icons,
        x: f64, y: f64, height: f64, alpha: f64, now: Time) {
        let rows = self.rows();
        if rows.is_empty() {
            let appear = if self.leaving.is_empty() { 1.0 } else {
                min_jerk(((now - self.row_motion_at - 0.12) / 0.28).clamp(0.0, 1.0))
            };
            let a = alpha * appear;
            let (top, h) = (y + ROW_TOP, ROW_H * 3.0 - 6.0);
            menu_style::group(cr, x + RAIL, top, WIDTH - 2.0 * RAIL, h, 0.0, a);
            let centre = top + h / 2.0;
            menu_style::well(cr, x + WIDTH / 2.0, centre - 26.0, 22.0, Some(menu_style::RED), a);
            cr.save().unwrap();
            cr.translate(x + WIDTH / 2.0, centre - 27.0);
            cr.scale(0.85, 0.85);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_join(cairo::LineJoin::Round);
            cr.set_line_width(2.2);
            cr.set_source_rgba(1.0, 1.0, 1.0, a);
            cr.move_to(-8.0, 8.0);
            cr.curve_to(-6.0, 5.0, -6.0, -1.0, -6.0, -4.0);
            cr.arc(0.0, -4.0, 6.0, std::f64::consts::PI, 0.0);
            cr.curve_to(6.0, -1.0, 6.0, 5.0, 8.0, 8.0);
            cr.close_path();
            let _ = cr.stroke();
            cr.new_path();
            cr.arc(0.0, 11.0, 1.5, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            cr.restore().unwrap();
            menu_style::centred(cr, renderer, "Пока пусто", (x, centre + 4.0, WIDTH, 22.0), Font::Headline, (1.0, 1.0, 1.0), a);
            menu_style::centred(cr, renderer, "Новые уведомления появятся здесь", (x, centre + 26.0, WIDTH, 20.0),
                Font::Subhead, (1.0, 1.0, 1.0), menu_style::SECONDARY * a);
        }
        cr.save().unwrap();
        // the viewport ends where the last card does (cards keep a 6 px gap)
        cr.rectangle(x + 14.0, y + ROW_TOP, WIDTH - 28.0, ROW_H * ROWS as f64 - 6.0);
        cr.clip();
        let scroll = self.scroll.value(now);
        for (index, view) in rows.iter().enumerate() {
            let position = self.row_position(&view.key, index as f64, now) - scroll;
            if position < -1.0 || position > ROWS as f64 { continue; }
            let delay = (index as f64 - self.scroll.target()).clamp(0.0, ROWS as f64 - 1.0) * 0.04;
            let opening = min_jerk(((now - self.opened_at - delay) / 0.38).clamp(0.0, 1.0));
            let born = if view.count > 1 { None } else { self.born_at.get(&view.entry.serial).copied() };
            let entrance = born.map_or(1.0, |since| min_jerk(((now - since) / ROW_MOTION).clamp(0.0, 1.0)));
            let fade = born.map_or(1.0, |since| min_jerk(((now - since) / 0.26).clamp(0.0, 1.0)));
            let top = y + ROW_TOP + position * ROW_H + (1.0 - opening) * 15.0 + (1.0 - entrance) * 20.0;
            let opacity = alpha * opening * fade;
            let hover = self.row_hover.get(&view.key).map_or(0.0, |spring| spring.value(now));
            self.draw_row(cr, renderer, media, view.entry, view.count, view.unread, view.child,
                x, top, opacity, hover, born.map_or(now - self.opened_at, |since| now - since));
        }
        for leaving in &self.leaving {
            let k = min_jerk(((now - leaving.since) / EXIT_MOTION).clamp(0.0, 1.0));
            let top = y + ROW_TOP + (leaving.row - scroll) * ROW_H - k * 9.0;
            self.draw_row(cr, renderer, media, &leaving.entry, leaving.count, leaving.unread, leaving.child,
                x + k * 25.0, top, alpha * (1.0 - k), 0.0, now - self.opened_at);
        }
        // cards slide under soft edges where there is more to scroll to
        let max = self.max_scroll();
        menu_style::edge_fades(cr, x, y + ROW_TOP, WIDTH, ROW_H * ROWS as f64 - 6.0,
            (scroll / 0.4).clamp(0.0, 1.0), ((max - scroll) / 0.4).clamp(0.0, 1.0), 22.0);
        cr.restore().unwrap();
        let shown = self.indicator(now);
        if rows.len() > ROWS && shown > 0.0 {
            // an overlay scroll indicator, shown while the list moves
            let track = ROW_H * ROWS as f64 - 18.0;
            let thumb = (track * ROWS as f64 / rows.len() as f64).max(24.0);
            let at = (scroll / max).clamp(0.0, 1.0);
            menu_style::rounded(cr, x + WIDTH - 13.0, y + ROW_TOP + 6.0 + (track - thumb) * at, 3.0, thumb, 1.5);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.35 * alpha * shown);
            let _ = cr.fill();
        }
        let _ = height;
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(&self, cr: &Context, renderer: &Renderer, media: &Icons, entry: &Entry, count: usize,
        unread: usize, child: bool, x: f64, top: f64, alpha: f64, hover: f64, icon_elapsed: Time) {
        if alpha <= 0.001 { return; }
        let grouped = alpha < 0.999;
        if grouped {
            cr.save().unwrap();
            cr.rectangle(x + 14.0, top, WIDTH - 28.0, ROW_H);
            cr.clip();
            cr.push_group();
        }
        let opacity = alpha;
        let alpha = 1.0;
        let indent = if child { 18.0 } else { 0.0 };
        let (card_x, card_w, card_h) = (x + RAIL + indent, WIDTH - 2.0 * RAIL - indent, ROW_H - 6.0);
        menu_style::group(cr, card_x, top, card_w, card_h, hover, alpha);
        let mid = top + card_h / 2.0;
        let icon_cx = card_x + 12.0 + 18.0;
        if !entry.note.leading_image().is_some_and(|name| crate::notify::draw_media(cr, media, name,
            icon_cx, mid, 36.0, renderer.scale(), alpha)) {
            icons::draw(cr, entry.note.kind, icon_cx, mid, 36.0, icon_elapsed.max(0.0), alpha);
        }
        let text_x = card_x + 12.0 + 36.0 + 12.0;
        let right = card_x + card_w - 14.0;
        let title_h = menu_style::line_height(cr, renderer, Font::Headline);
        let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
        let line1 = mid - (title_h + sub_h) / 2.0;
        // the trailing column: when, and whether it is new; × under the pointer
        let when = age(now_secs().saturating_sub(entry.received));
        let when_w = renderer.layout(cr, &when, Font::Footnote).pixel_extents().1.width() as f64;
        let rest = 1.0 - hover.clamp(0.0, 1.0);
        let foot_h = menu_style::line_height(cr, renderer, Font::Footnote);
        menu_style::label(cr, renderer, &when, right - when_w, line1 + (title_h - foot_h) / 2.0, when_w + 2.0,
            Font::Footnote, menu_style::SECONDARY * alpha * rest);
        if unread > 0 {
            cr.new_path();
            cr.arc(right - when_w - 9.0, line1 + title_h / 2.0, 3.5, 0.0, std::f64::consts::TAU);
            cr.set_source_rgba(menu_style::BLUE.0, menu_style::BLUE.1, menu_style::BLUE.2, alpha * rest);
            let _ = cr.fill();
        }
        if hover > 0.01 {
            menu_style::close(cr, right - 22.0, mid - 11.0, 22.0, 0.0, alpha * hover);
        }
        let title_w = right - when_w - 22.0 - text_x;
        if count > 1 {
            let name_w = (renderer.layout(cr, &entry.note.app, Font::Headline).pixel_extents().1.width() as f64)
                .min(title_w - 52.0);
            menu_style::label(cr, renderer, &entry.note.app, text_x, line1, name_w + 2.0, Font::Headline, alpha);
            let bw = menu_style::badge(cr, renderer, &count.to_string(), text_x + name_w + 8.0, line1 + title_h / 2.0,
                menu_style::GRAY, alpha);
            let open = self.expanded.contains(&entry.note.app);
            menu_style::chevron(cr, text_x + name_w + 8.0 + bw + 11.0, line1 + title_h / 2.0, 4.5,
                if open { -std::f64::consts::FRAC_PI_2 } else { std::f64::consts::FRAC_PI_2 },
                alpha * (0.5 + 0.4 * hover));
            menu_style::label(cr, renderer, &entry.note.message(), text_x, line1 + title_h, right - 28.0 - text_x,
                Font::Subhead, menu_style::SECONDARY * alpha);
        } else {
            // sender first, the message below; the app's name only fills in
            let note = &entry.note;
            let title = if note.summary.is_empty() { note.app.as_str() } else { note.summary.as_str() };
            let body = if !note.body.is_empty() { note.body.as_str() } else if !note.summary.is_empty() { note.app.as_str() } else { "" };
            menu_style::label(cr, renderer, title, text_x, line1, title_w, Font::Headline, alpha);
            menu_style::label(cr, renderer, body, text_x, line1 + title_h, right - 28.0 - text_x,
                Font::Subhead, menu_style::SECONDARY * alpha);
        }
        if grouped {
            cr.pop_group_to_source().unwrap();
            let _ = cr.paint_with_alpha(opacity);
            cr.restore().unwrap();
        }
    }

    fn draw_detail(&self, cr: &Context, renderer: &Renderer, media: &Icons, x: f64, y: f64, alpha: f64, serial: u64, now: Time) {
        let Some(entry) = self.entries.iter().find(|e| e.serial == serial) else { return };
        let top = y + menu_style::CONTENT_TOP;
        if !entry.note.leading_image().is_some_and(|name| crate::notify::draw_media(cr, media, name,
            x + RAIL + 22.0, top + 22.0, 44.0, renderer.scale(), alpha)) {
            icons::draw(cr, entry.note.kind, x + RAIL + 22.0, top + 22.0, 44.0, (now - self.opened_at).max(0.0), alpha);
        }
        let tx = x + RAIL + 44.0 + 12.0;
        let tw = WIDTH - RAIL - (tx - x);
        let title_h = menu_style::line_height(cr, renderer, Font::Headline);
        let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
        let line1 = top + 22.0 - (title_h + sub_h) / 2.0;
        let when = age(now_secs().saturating_sub(entry.received));
        menu_style::label(cr, renderer, &entry.note.summary, tx, line1, tw, Font::Headline, alpha);
        menu_style::label(cr, renderer, &format!("{} · {}", entry.note.app, when), tx, line1 + title_h, tw,
            Font::Subhead, menu_style::SECONDARY * alpha);
        let body_top = top + 58.0;
        let body_h = HEIGHT - menu_style::BUTTON_H - RAIL - 14.0 - body_top + y;
        menu_style::group(cr, x + RAIL, body_top, WIDTH - 2.0 * RAIL, body_h, 0.0, alpha);
        cr.save().unwrap();
        cr.rectangle(x + RAIL + 14.0, body_top + 10.0, WIDTH - 2.0 * RAIL - 28.0, body_h - 20.0);
        cr.clip();
        let body_y = if let Some(image) = &entry.note.image {
            let _ = crate::notify::draw_media(cr, media, image, x + WIDTH / 2.0,
                body_top + 70.0, 118.0, renderer.scale(), alpha);
            body_top + 138.0
        } else { body_top + 14.0 };
        let text = renderer.layout(cr, &entry.note.body, Font::Body).copy();
        text.set_width(((WIDTH - 2.0 * RAIL - 28.0) * pango::SCALE as f64) as i32);
        text.set_wrap(pango::WrapMode::WordChar);
        cr.set_source_rgba(1.0, 1.0, 1.0, alpha * 0.9);
        cr.move_to(renderer.snap(x + RAIL + 14.0), renderer.snap(body_y - self.detail_scroll));
        pangocairo::functions::show_layout(cr, &text);
        cr.restore().unwrap();
        let by = y + HEIGHT - RAIL - menu_style::BUTTON_H;
        for (index, (_, text)) in visible_actions(&entry.note).iter().enumerate() {
            let kind = if index == 0 { ButtonKind::Primary } else { ButtonKind::Neutral };
            menu_style::button(cr, renderer, text, (x + RAIL + index as f64 * 154.0, by, 144.0, menu_style::BUTTON_H),
                kind, self.button_hover[2 + index].value(now), alpha);
        }
        menu_style::button(cr, renderer, "Удалить", (x + WIDTH - 110.0, by, 88.0, menu_style::BUTTON_H),
            ButtonKind::Destructive, self.button_hover[4].value(now), alpha);
    }
}

/// How long ago, the way Notification Center says it.
fn age(seconds: u64) -> String {
    match seconds {
        0..60 => "сейчас".into(),
        60..3600 => format!("{} мин", seconds / 60),
        3600..86400 => format!("{} ч", seconds / 3600),
        86400..172800 => "вчера".into(),
        _ => format!("{} дн.", seconds / 86400),
    }
}

fn visible_actions(note: &Notification) -> Vec<&(String, String)> {
    let mut actions: Vec<_> = note.actions.iter().filter(|(key, _)| key == "default").take(1).collect();
    actions.extend(note.actions.iter().filter(|(key, _)| key != "default").take(1));
    actions
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
            center.upsert(note, id as f64);
        }
        assert_eq!(center.len(), LIMIT);
        let mut replacement = crate::notify::fabricate("System", "updated", "body");
        replacement.id = 52;
        center.upsert(replacement, 53.0);
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
        center.upsert(note, 0.0);
        center.open(0.0);
        assert_eq!(center.unread(), 1);
        assert_eq!(center.click(90.0, ROW_TOP + 12.0, 0.5), Action::None);
        assert_eq!(center.unread(), 0);
        assert_eq!(center.view_transition, Some((None, 0.5)));
        assert!(center.is_animating(0.6));
        assert_eq!(center.click(35.0, HEIGHT - 40.0, 0.6), Action::Invoke(5, "default".into()));
        assert_eq!(center.click(WIDTH - 40.0, HEIGHT - 40.0, 0.7), Action::Dismiss(5));
        assert_eq!(center.len(), 0);
    }

    #[test]
    fn same_app_collapses_and_children_keep_individual_read_state() {
        let mut center = center();
        for (id, app) in [(1, "Mail"), (2, "Chat"), (3, "Mail")] {
            let mut note = crate::notify::fabricate(app, &format!("Message {id}"), "body");
            note.id = id;
            center.upsert(note, id as f64);
        }
        center.open(10.0);
        let rows = center.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].entry.note.app, "Mail");
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[0].unread, 2);
        assert_eq!(rows[1].entry.note.app, "Chat");
        assert_eq!(center.height(), ROW_TOP + 2.0 * ROW_H + LIST_BOTTOM);
        assert_eq!(center.click(90.0, ROW_TOP + 12.0, 10.5), Action::None);
        assert_eq!(center.rows().len(), 4);
        assert_eq!(center.height(), HEIGHT);
        assert!(center.rows()[1].child);
        assert!(center.born_at.contains_key(&center.rows()[1].entry.serial));
        assert!(center.detail.is_none());
        assert_eq!(center.click(90.0, ROW_TOP + ROW_H + 12.0, 11.0), Action::None);
        assert!(center.detail.is_some());
        assert_eq!(center.unread(), 2);
        assert_eq!(center.rows()[0].unread, 1);
        assert_eq!(center.click(40.0, 25.0, 11.2), Action::None);
        assert_eq!(center.click(90.0, ROW_TOP + 12.0, 11.5), Action::None);
        assert_eq!(center.rows().len(), 2);
        assert_eq!(center.height(), HEIGHT);
        assert_eq!(center.leaving.iter().filter(|row| row.child).count(), 2);
        center.advance(11.5 + ROW_MOTION + 0.01);
        assert_eq!(center.height(), ROW_TOP + 2.0 * ROW_H + LIST_BOTTOM);
    }

    #[test]
    fn fractional_wheel_reaches_the_last_child_of_an_expanded_group() {
        let mut center = center();
        for id in 1..=5 {
            let mut note = crate::notify::fabricate("Telegram", &format!("Message {id}"), "body");
            note.id = id;
            center.upsert(note, id as f64);
        }
        center.open(10.0);
        assert_eq!(center.click(90.0, ROW_TOP + 12.0, 10.5), Action::None);
        assert_eq!(center.rows().len(), 6);
        // a quarter of a row is a quarter of a row: no rounding to steps
        assert!(center.scroll(0.25, 11.0));
        assert_eq!(center.scroll.target(), 0.25);
        let early = center.scroll.value(11.02);
        assert!(early > 0.05 && early < 0.25, "the glide starts at once: {early}");
        for step in 2..=12 { center.scroll(0.25, 11.0 + step as f64 * 0.02); }
        assert_eq!(center.scroll.target(), 2.0, "clamped at the end of the list");
        assert!(!center.scroll(0.25, 11.4));
        assert_eq!(center.rows()[2 + ROWS - 1].entry.note.summary, "Message 1");
        assert_eq!(center.click(90.0, ROW_TOP + (ROWS as f64 - 1.0) * ROW_H + 12.0, 12.0), Action::None);
        assert_eq!(center.detail.and_then(|serial| center.entries.iter().find(|entry| entry.serial == serial))
            .map(|entry| entry.note.summary.as_str()), Some("Message 1"));
    }

    #[test]
    fn removing_a_group_dismisses_only_that_apps_active_notifications() {
        let mut center = center();
        for (id, app) in [(1, "Mail"), (2, "Chat"), (3, "Mail")] {
            let mut note = crate::notify::fabricate(app, &id.to_string(), "body");
            note.id = id;
            center.upsert(note, id as f64);
        }
        center.open(10.0);
        assert_eq!(center.click(WIDTH - 40.0, ROW_TOP + 12.0, 10.5), Action::DismissMany(vec![3, 1]));
        assert_eq!(center.len(), 1);
        assert_eq!(center.rows()[0].entry.note.app, "Chat");
    }

    #[test]
    fn incoming_and_removed_rows_interpolate_without_jumping() {
        let mut center = center();
        for id in 1..=2 {
            let mut note = crate::notify::fabricate(&format!("App{id}"), &id.to_string(), "body");
            note.id = id;
            center.upsert(note, id as f64);
        }
        center.open(10.0);
        let previous_front = center.entries.front().unwrap().serial;
        let mut incoming = crate::notify::fabricate("App3", "incoming", "body");
        incoming.id = 3;
        center.upsert(incoming, 11.0);
        assert!(center.born_at.contains_key(&center.entries.front().unwrap().serial));
        assert_eq!(center.row_position(&RowKey::Entry(previous_front), 1.0, 11.0), 0.0);
        assert!((center.row_position(&RowKey::Entry(previous_front), 1.0, 11.0 + ROW_MOTION) - 1.0).abs() < 1e-9);
        assert!(center.is_animating(11.1));
        center.advance(11.5);
        assert!(center.born_at.is_empty());

        assert_eq!(center.click(WIDTH - 40.0, ROW_TOP + 16.0, 12.0), Action::Dismiss(3));
        assert_eq!(center.leaving.len(), 1);
        assert_eq!(center.row_position(&RowKey::Entry(previous_front), 0.0, 12.0), 1.0);
        assert!(center.row_position(&RowKey::Entry(previous_front), 0.0, 12.0 + ROW_MOTION).abs() < 1e-9);
        center.advance(12.0 + EXIT_MOTION);
        assert!(center.leaving.is_empty());
    }

    #[test]
    #[ignore = "set ISLAND_CENTER_DUMP to render the detail sheet for visual inspection"]
    fn detail_sheet_renders_at_fractional_scale() {
        let path = std::env::var("ISLAND_CENTER_DUMP").expect("ISLAND_CENTER_DUMP path");
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 720, 645).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.paint().unwrap();
        cr.scale(1.5, 1.5);
        let mut center = center();
        let mut note = crate::notify::fabricate("System", "Проверка центра", "Текст уведомления должен читаться без наложений.");
        note.id = 1;
        note.actions = vec![("default".into(), "Открыть".into())];
        center.upsert(note, 0.0);
        center.open(0.0);
        center.click(90.0, ROW_TOP + 12.0, 0.5);
        center.draw(&cr, &renderer, &Icons::new(), 0.0, 0.0, 1.0, 1.0);
        surface.write_to_png(&mut std::fs::File::create(path).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "set ISLAND_CENTER_DUMP to inspect grouped emoji entrance"]
    fn grouped_emoji_entrance_renders_at_fractional_scale() {
        let path = std::env::var("ISLAND_CENTER_DUMP").expect("ISLAND_CENTER_DUMP path");
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 720, 645).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        let mut center = center();
        for (id, summary) in [(1, "Первое 🎁"), (2, "Второе 🎉")] {
            let mut note = crate::notify::fabricate("Telegram", summary, "Текст 🎁 после раскрытия");
            note.id = id;
            center.upsert(note, id as f64);
        }
        center.open(0.0);
        center.click(90.0, ROW_TOP + 12.0, 0.6);
        center.draw(&cr, &renderer, &Icons::new(), 0.0, 0.0, 1.0, 0.72);
        surface.write_to_png(&mut std::fs::File::create(path).unwrap()).unwrap();
    }
}
