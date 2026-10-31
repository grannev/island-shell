//! Notes, after Vorssaint's scratchpad: an open-island page whose top row
//! holds the pads as tabs and whose card holds the chosen pad's text, to
//! type into at once. Every pad is a Markdown file in ~/Documents/notes/
//! (named by when it was made); a tab is titled by its pad's first line.
//!
//! Typing is plain text editing: arrows (with Ctrl by word, with Shift
//! selecting), Home / End, Backspace / Delete (with Ctrl by word), Enter,
//! Tab, Ctrl+A / C / X / V through wl-copy / wl-paste, Ctrl+T a new pad,
//! Ctrl+Tab / Ctrl+PageDown the next pad (with Shift / PageUp the previous).
//! The mouse places the caret, drags a selection, and the wheel scrolls.
//!
//! Files are read, written and moved to the trash on a background worker;
//! a pad is saved 0.6 s after its last change (and when the sheet closes),
//! written to a temporary file and renamed over the old one. An empty pad
//! is never written, and one emptied is taken away when you leave it. The
//! hovered tab of the chosen pad shows a bin; a second press deletes the
//! pad (`gio trash`, so it can be brought back).

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use cairo::Context;

use crate::anim::{Glide, Spring, Time, min_jerk};
use crate::background::Worker;
use crate::island::{Font, Renderer};
use crate::menu_style;

pub const WIDTH: f64 = 560.0;
pub const HEIGHT: f64 = 392.0;
const RAIL: f64 = 24.0;
const HEADER_TOP: f64 = 12.0;
const HEADER_H: f64 = 36.0;
const TAB_H: f64 = 28.0;
const TAB_GAP: f64 = 6.0;
const TAB_PAD: f64 = 12.0;
const TAB_MAX: f64 = 150.0;
const TAB_MIN: f64 = 64.0;
const ROUND: f64 = 28.0;
const CARD_Y: f64 = HEADER_TOP + HEADER_H + 12.0;
const BOTTOM: f64 = 16.0;
const CARD_R: f64 = 18.0;
const CARD_PAD: f64 = 16.0;
const SAVE_AFTER: f64 = 0.6;
const BLINK: f64 = 0.53;
const SWITCH: f64 = 0.32;
const ARM_FOR: f64 = 3.0;
const BLUE: (f64, f64, f64) = menu_style::BLUE;

/// Where the pads live.
pub fn dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Documents").join("notes")
}

/// A pad's title: its first line with any Markdown heading marks taken off.
pub fn title_of(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line = line.trim_start_matches('#').trim();
    if line.is_empty() { "Новая заметка".into() } else { line.chars().take(40).collect() }
}

#[derive(Clone, Debug)]
pub struct Pad {
    pub path: PathBuf,
    pub text: String,
    /// when it last changed and has not been written since
    dirty_at: Option<Time>,
    /// the file exists on disk
    on_disk: bool,
}

/// Work for the file worker.
#[derive(Clone, Debug)]
enum Job {
    Load(PathBuf),
    Save(PathBuf, String),
    Trash(PathBuf),
    Paste,
}

#[derive(Debug)]
enum Done {
    Loaded(Vec<(PathBuf, String)>),
    Saved(Result<(), String>),
    Trashed,
    Pasted(Option<String>),
}

fn run(job: Job) -> Done {
    match job {
        Job::Load(dir) => {
            let mut pads = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "md" || e == "txt") && p.is_file())
                    .collect();
                paths.sort();
                for path in paths.into_iter().take(64) {
                    // pads are short; a huge file is not a pad
                    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2_000_000) { continue; }
                    if let Ok(text) = std::fs::read_to_string(&path) { pads.push((path, text)); }
                }
            }
            Done::Loaded(pads)
        }
        Job::Save(path, text) => Done::Saved(write_atomically(&path, &text).map_err(|e| e.to_string())),
        Job::Trash(path) => {
            let trashed = std::process::Command::new("gio").arg("trash").arg("--").arg(&path)
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                .status().is_ok_and(|s| s.success());
            if !trashed && path.exists() {
                // no trash to be had: keep it aside rather than lose it
                let aside = path.parent().map(|d| d.join(".trash")).unwrap_or_default();
                let _ = std::fs::create_dir_all(&aside);
                if let Some(name) = path.file_name() { let _ = std::fs::rename(&path, aside.join(name)); }
            }
            Done::Trashed
        }
        Job::Paste => {
            let out = std::process::Command::new("wl-paste").args(["--no-newline", "--type", "text/plain;charset=utf-8"])
                .stderr(std::process::Stdio::null()).output();
            let text = out.ok().filter(|o| o.status.success()).and_then(|o| String::from_utf8(o.stdout).ok());
            Done::Pasted(text.filter(|t| !t.is_empty()))
        }
    }
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// What a press lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Tab(usize),
    /// the chosen tab's bin
    Delete(usize),
    New,
    Close,
    /// the text, at a byte index
    Text(usize),
}

type Rect = (f64, f64, f64, f64);

fn inside(x: f64, y: f64, r: Rect) -> bool {
    x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
}

fn prev_char(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(j, _)| j)
}

fn next_char(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

fn prev_word(s: &str, i: usize) -> usize {
    let mut j = i;
    // spaces first, then the word
    while j > 0 && s[..j].chars().next_back().is_some_and(|c| !c.is_alphanumeric()) { j = prev_char(s, j); }
    while j > 0 && s[..j].chars().next_back().is_some_and(char::is_alphanumeric) { j = prev_char(s, j); }
    j
}

fn next_word(s: &str, i: usize) -> usize {
    let mut j = i;
    while j < s.len() && s[j..].chars().next().is_some_and(|c| !c.is_alphanumeric()) { j = next_char(s, j); }
    while j < s.len() && s[j..].chars().next().is_some_and(char::is_alphanumeric) { j = next_char(s, j); }
    j
}

fn line_start(s: &str, i: usize) -> usize {
    s[..i].rfind('\n').map_or(0, |j| j + 1)
}

fn line_end(s: &str, i: usize) -> usize {
    s[i..].find('\n').map_or(s.len(), |j| i + j)
}

pub struct Notes {
    dir: PathBuf,
    pub pads: Vec<Pad>,
    pub current: usize,
    loaded: bool,
    pub caret: usize,
    /// the other end of the selection, if any
    pub anchor: Option<usize>,
    /// the x the caret keeps while moving up and down
    goal_x: Option<f64>,
    scroll: Glide,
    /// when the caret last moved (it stays lit for a moment, then blinks)
    typed_at: Time,
    pub opened: Time,
    /// a pad switch: the one left, which way, and when
    switch: Option<(usize, f64, Time)>,
    armed: Option<(usize, Time)>,
    hover: Option<Hit>,
    hover_lit: Spring,
    /// the chosen tab's bin coming out under the pointer
    bin_lit: Spring,
    dragging: bool,
    worker: Worker<Job, Done>,
    pending_saves: usize,
    pub error: Option<String>,
    /// laid out at the last draw: the tabs' hit rects and the text
    tabs: RefCell<Vec<(Hit, Rect)>>,
    ctx: RefCell<Option<pango::Context>>,
    layout: RefCell<Option<(u64, pango::Layout)>>,
    version: u64,
    offline: bool,
}

impl Notes {
    pub fn new() -> Notes {
        Notes::in_dir(dir())
    }

    pub fn in_dir(dir: PathBuf) -> Notes {
        Notes {
            dir,
            pads: Vec::new(),
            current: 0,
            loaded: false,
            caret: 0,
            anchor: None,
            goal_x: None,
            scroll: Glide::new(0.0, 0.09),
            typed_at: 0.0,
            opened: 0.0,
            switch: None,
            armed: None,
            hover: None,
            hover_lit: Spring::new(0.0, 0.24, 0.83),
            bin_lit: Spring::new(0.0, 0.3, 0.9),
            dragging: false,
            worker: Worker::new("notes", run),
            pending_saves: 0,
            error: None,
            tabs: RefCell::new(Vec::new()),
            ctx: RefCell::new(None),
            layout: RefCell::new(None),
            version: 0,
            offline: false,
        }
    }

    /// A page for pictures and tests: these pads, no files.
    #[cfg(test)]
    pub fn offline(texts: &[&str], now: Time) -> Notes {
        let mut notes = Notes::in_dir(PathBuf::from("/nonexistent/notes"));
        notes.offline = true;
        notes.loaded = true;
        notes.pads = texts.iter().enumerate().map(|(i, t)| Pad {
            path: PathBuf::from(format!("/nonexistent/notes/{i}.md")), text: t.to_string(), dirty_at: None, on_disk: true,
        }).collect();
        notes.opened = now;
        notes
    }

    pub fn set_waker(&self, tx: smithay_client_toolkit::reexports::calloop::channel::Sender<()>) {
        self.worker.set_waker(tx);
    }

    /// The sheet opens: read the pads (the first time), the caret where it was.
    pub fn open(&mut self, now: Time) {
        self.opened = now;
        self.typed_at = now;
        self.armed = None;
        self.error = None;
        if !self.loaded && !self.offline {
            self.worker.request(Job::Load(self.dir.clone()));
        }
        if self.loaded && self.pads.is_empty() { self.new_pad(now); }
    }

    /// The sheet closes: whatever is unsaved goes now; an emptied pad goes away.
    pub fn close(&mut self, now: Time) {
        self.leave_pad(now);
        self.flush(now, true);
        self.dragging = false;
        self.hover = None;
    }

    fn text(&self) -> &str {
        self.pads.get(self.current).map_or("", |p| p.text.as_str())
    }

    fn changed(&mut self, now: Time) {
        self.version += 1;
        self.typed_at = now;
        self.goal_x = None;
        self.armed = None;
        if let Some(pad) = self.pads.get_mut(self.current) { pad.dirty_at = Some(now); }
    }

    fn new_name(&self) -> PathBuf {
        let stamp = crate::island::strftime("%Y-%m-%d %H.%M.%S");
        let mut path = self.dir.join(format!("{stamp}.md"));
        let mut n = 2;
        while self.pads.iter().any(|p| p.path == path) || (!self.offline && path.exists()) {
            path = self.dir.join(format!("{stamp} {n}.md"));
            n += 1;
        }
        path
    }

    pub fn new_pad(&mut self, now: Time) {
        // an empty pad already open is the new one
        if self.pads.get(self.current).is_some_and(|p| p.text.trim().is_empty()) { return; }
        self.leave_pad(now);
        let path = self.new_name();
        self.pads.push(Pad { path, text: String::new(), dirty_at: None, on_disk: false });
        let from = self.current;
        self.current = self.pads.len() - 1;
        self.switched(from, 1.0, now);
    }

    /// Leaving a pad that was emptied: it goes, file and all.
    fn leave_pad(&mut self, now: Time) {
        let Some(pad) = self.pads.get(self.current) else { return };
        if !pad.text.trim().is_empty() || self.pads.len() <= 1 && !pad.on_disk { return; }
        let pad = self.pads.remove(self.current);
        if pad.on_disk && !self.offline {
            let _ = self.worker.request(Job::Trash(pad.path));
        }
        self.current = self.current.min(self.pads.len().saturating_sub(1));
        self.version += 1;
        let _ = now;
    }

    fn switched(&mut self, from: usize, dir: f64, now: Time) {
        if from != self.current { self.switch = Some((from, dir, now)); }
        self.caret = self.text().len();
        self.anchor = None;
        self.goal_x = None;
        self.armed = None;
        self.typed_at = now;
        self.scroll.reset(0.0);
        self.version += 1;
        self.reveal_caret(now);
    }

    pub fn select(&mut self, index: usize, now: Time) {
        if index >= self.pads.len() || index == self.current { return; }
        let dir = if index > self.current { 1.0 } else { -1.0 };
        let target = self.pads[index].path.clone();
        self.leave_pad(now);
        let from = self.current;
        self.current = self.pads.iter().position(|p| p.path == target).unwrap_or(0);
        self.switched(from, dir, now);
    }

    fn cycle(&mut self, step: isize, now: Time) {
        if self.pads.len() < 2 { return; }
        let n = self.pads.len() as isize;
        let i = (self.current as isize + step).rem_euclid(n) as usize;
        self.select(i, now);
    }

    /// The bin on the chosen tab: the first press arms it, the second
    /// sends the pad to the trash.
    pub fn delete(&mut self, index: usize, now: Time) {
        if index >= self.pads.len() { return; }
        if !self.armed.is_some_and(|(i, t)| i == index && now - t < ARM_FOR) {
            self.armed = Some((index, now));
            return;
        }
        self.armed = None;
        let pad = self.pads.remove(index);
        if pad.on_disk && !self.offline {
            let _ = self.worker.request(Job::Trash(pad.path));
        }
        if self.pads.is_empty() {
            self.pads.push(Pad { path: self.new_name(), text: String::new(), dirty_at: None, on_disk: false });
        }
        let from = self.current;
        self.current = index.min(self.pads.len() - 1);
        self.switched(from.max(self.pads.len()), -1.0, now);
    }

    /// Write every pad whose quiet time is up (all of them when `all`).
    fn flush(&mut self, now: Time, all: bool) {
        for pad in &mut self.pads {
            let Some(at) = pad.dirty_at else { continue };
            if !all && now - at < SAVE_AFTER { continue; }
            pad.dirty_at = None;
            if pad.text.trim().is_empty() && !pad.on_disk { continue; }
            if self.offline { pad.on_disk = true; continue; }
            if self.worker.request(Job::Save(pad.path.clone(), pad.text.clone())) {
                pad.on_disk = true;
                self.pending_saves += 1;
            } else {
                // the queue is full: try again in a moment
                pad.dirty_at = Some(now);
            }
        }
    }

    /// Replies from the worker; true when the page changed.
    pub fn drain(&mut self, now: Time) -> bool {
        let mut changed = false;
        for (_, done) in self.worker.drain() {
            match done {
                Done::Loaded(found) => {
                    self.loaded = true;
                    // pads made while loading stay after the ones on disk
                    let fresh: Vec<Pad> = self.pads.drain(..).filter(|p| !p.text.is_empty()).collect();
                    self.pads = found.into_iter().map(|(path, text)| Pad { path, text, dirty_at: None, on_disk: true }).collect();
                    self.pads.extend(fresh);
                    if self.pads.is_empty() {
                        self.pads.push(Pad { path: self.new_name(), text: String::new(), dirty_at: None, on_disk: false });
                    }
                    self.current = self.pads.len() - 1;
                    self.caret = self.text().len();
                    self.anchor = None;
                    self.version += 1;
                    self.reveal_caret(now);
                    changed = true;
                }
                Done::Saved(result) => {
                    self.pending_saves = self.pending_saves.saturating_sub(1);
                    let error = result.err().map(|e| format!("Не удалось сохранить: {e}"));
                    changed |= error != self.error;
                    self.error = error;
                }
                Done::Trashed => {}
                Done::Pasted(Some(text)) => {
                    self.insert(&text, now);
                    changed = true;
                }
                Done::Pasted(None) => {}
            }
        }
        changed
    }

    /// Saves coming due; true when something changed.
    pub fn advance(&mut self, now: Time) -> bool {
        let due = self.pads.iter().any(|p| p.dirty_at.is_some_and(|t| now - t >= SAVE_AFTER));
        if due { self.flush(now, false); }
        if self.armed.is_some_and(|(_, t)| now - t >= ARM_FOR) {
            self.armed = None;
            return true;
        }
        false
    }

    /// The next save, the bin disarming, and (while `shown`) the caret's
    /// next blink.
    pub fn deadline(&self, now: Time, shown: bool) -> Option<Time> {
        let mut next: Option<Time> = None;
        let mut consider = |t: Time| next = Some(next.map_or(t, |n: Time| n.min(t)));
        for pad in &self.pads {
            if let Some(at) = pad.dirty_at { consider(at + SAVE_AFTER); }
        }
        if let Some((_, t)) = self.armed { consider(t + ARM_FOR); }
        if shown && self.anchor.is_none_or(|a| a == self.caret) {
            let first = self.typed_at + BLINK;
            if now < first {
                consider(first);
            } else {
                consider(first + (((now - first) / BLINK).floor() + 1.0) * BLINK);
            }
        }
        next.map(|t| t.max(now))
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.scroll.is_moving(now) || self.hover_lit.is_moving(now) || self.bin_lit.is_moving(now)
            || self.switch.is_some_and(|(_, _, t)| now - t < SWITCH)
            || now - self.opened < 0.5
    }

    #[cfg(test)]
    pub fn has_unsaved(&self) -> bool {
        self.pads.iter().any(|p| p.dirty_at.is_some())
    }

    fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.caret).then(|| (a.min(self.caret), a.max(self.caret)))
    }

    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else { self.anchor = None; return false };
        if let Some(pad) = self.pads.get_mut(self.current) { pad.text.replace_range(a..b, ""); }
        self.caret = a;
        self.anchor = None;
        true
    }

    pub fn insert(&mut self, text: &str, now: Time) {
        if self.pads.is_empty() { return; }
        self.delete_selection();
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let caret = self.caret.min(self.text().len());
        self.pads[self.current].text.insert_str(caret, &text);
        self.caret = caret + text.len();
        self.changed(now);
        self.reveal_caret(now);
    }

    fn move_to(&mut self, to: usize, shift: bool, now: Time) {
        if shift {
            if self.anchor.is_none() { self.anchor = Some(self.caret); }
        } else {
            self.anchor = None;
        }
        self.caret = to.min(self.text().len());
        self.typed_at = now;
        self.reveal_caret(now);
    }

    /// A key (names as the app maps them) or typed `text`. True when the
    /// page changed.
    pub fn key(&mut self, key: &str, text: Option<&str>, ctrl: bool, shift: bool, now: Time) -> bool {
        if self.pads.is_empty() { return false; }
        let s = self.text().to_string();
        let caret = self.caret.min(s.len());
        let keep_goal = matches!(key, "Up" | "Down" | "PageUp" | "PageDown") && !ctrl;
        if !keep_goal { self.goal_x = None; }
        match (key, ctrl) {
            ("Tab" | "PageDown", true) => self.cycle(if shift && key == "Tab" { -1 } else { 1 }, now),
            ("ISO_Left_Tab" | "PageUp", true) => self.cycle(-1, now),
            ("t", true) | ("n", true) => self.new_pad(now),
            ("a", true) => {
                self.anchor = Some(0);
                self.caret = s.len();
            }
            ("c" | "x", true) => {
                let (a, b) = self.selection().unwrap_or((0, s.len()));
                if b > a { crate::clipboard::copy(&s[a..b]); }
                if key == "x" && self.delete_selection() {
                    self.changed(now);
                }
            }
            ("v", true) => {
                if !self.offline { self.worker.request(Job::Paste); }
            }
            ("Left", _) => {
                let to = match self.selection() {
                    Some((a, _)) if !shift => a,
                    _ if ctrl => prev_word(&s, caret),
                    _ => prev_char(&s, caret),
                };
                self.move_to(to, shift, now);
            }
            ("Right", _) => {
                let to = match self.selection() {
                    Some((_, b)) if !shift => b,
                    _ if ctrl => next_word(&s, caret),
                    _ => next_char(&s, caret),
                };
                self.move_to(to, shift, now);
            }
            ("Home", _) => self.move_to(if ctrl { 0 } else { line_start(&s, caret) }, shift, now),
            ("End", _) => self.move_to(if ctrl { s.len() } else { line_end(&s, caret) }, shift, now),
            ("Up" | "Down" | "PageUp" | "PageDown", _) => {
                let lines = if key.starts_with("Page") { 8.0 } else { 1.0 };
                let dir = if key == "Up" || key == "PageUp" { -1.0 } else { 1.0 };
                let to = self.vertical(caret, dir * lines);
                self.move_to(to, shift, now);
            }
            ("BackSpace", _) => {
                if !self.delete_selection() {
                    let from = if ctrl { prev_word(&s, caret) } else { prev_char(&s, caret) };
                    if from == caret { return false; }
                    self.pads[self.current].text.replace_range(from..caret, "");
                    self.caret = from;
                }
                self.changed(now);
                self.reveal_caret(now);
            }
            ("Delete", _) => {
                if !self.delete_selection() {
                    let to = if ctrl { next_word(&s, caret) } else { next_char(&s, caret) };
                    if to == caret { return false; }
                    self.pads[self.current].text.replace_range(caret..to, "");
                }
                self.changed(now);
            }
            ("Return", false) => self.insert("\n", now),
            ("Tab", false) => self.insert("    ", now),
            _ => match text {
                Some(t) if !ctrl && !t.is_empty() && !t.chars().any(char::is_control) => self.insert(t, now),
                _ => return false,
            },
        }
        true
    }

    /// The byte index a line up or down from `caret` (`lines` signed).
    fn vertical(&mut self, caret: usize, lines: f64) -> usize {
        let s = self.text().to_string();
        let Some(layout) = self.text_layout() else {
            // no layout yet: by text lines
            let start = line_start(&s, caret);
            let column = s[start..caret].chars().count();
            let target = if lines < 0.0 {
                if start == 0 { return 0; }
                line_start(&s, start - 1)
            } else {
                let end = line_end(&s, caret);
                if end >= s.len() { return s.len(); }
                end + 1
            };
            let end = line_end(&s, target);
            return s[target..end].char_indices().nth(column).map_or(end, |(i, _)| target + i);
        };
        let pos = layout.index_to_pos(caret as i32);
        let scale = pango::SCALE as f64;
        let x = *self.goal_x.get_or_insert(pos.x() as f64 / scale);
        let line_h = (pos.height() as f64 / scale).max(1.0);
        let y = pos.y() as f64 / scale + line_h / 2.0 + lines * line_h;
        let (_, logical) = layout.extents();
        if y < 0.0 { return 0; }
        if y > logical.height() as f64 / scale { return s.len(); }
        index_at(&layout, &s, x, y)
    }

    fn text_layout(&self) -> Option<pango::Layout> {
        let mut cached = self.layout.borrow_mut();
        if let Some((v, l)) = cached.as_ref() {
            if *v == self.version { return Some(l.clone()); }
        }
        let ctx = self.ctx.borrow();
        let ctx = ctx.as_ref()?;
        let layout = pango::Layout::new(ctx);
        let mut desc = pango::FontDescription::from_string(Font::Body.face().0);
        desc.set_absolute_size(Font::Body.face().1 * pango::SCALE as f64);
        layout.set_font_description(Some(&desc));
        layout.set_width((TEXT_W * pango::SCALE as f64) as i32);
        layout.set_wrap(pango::WrapMode::WordChar);
        layout.set_spacing((3.0 * pango::SCALE as f64) as i32);
        layout.set_text(self.text());
        *cached = Some((self.version, layout.clone()));
        Some(layout)
    }

    fn text_height(&self) -> f64 {
        self.text_layout().map_or(0.0, |l| l.extents().1.height() as f64 / pango::SCALE as f64)
    }

    /// Scroll so the caret is in view.
    fn reveal_caret(&mut self, now: Time) {
        let Some(layout) = self.text_layout() else { return };
        let pos = layout.index_to_pos(self.caret.min(self.text().len()) as i32);
        let scale = pango::SCALE as f64;
        let (top, bottom) = (pos.y() as f64 / scale, (pos.y() + pos.height()) as f64 / scale);
        let view = TEXT_H;
        let target = self.scroll.target();
        let max = (self.text_height() - view).max(0.0);
        let want = if top < target { top - 6.0 } else if bottom > target + view { bottom - view + 6.0 } else { target };
        let want = want.clamp(0.0, max);
        if (want - target).abs() > 0.5 { self.scroll.set(want, now); }
    }

    pub fn scroll_by(&mut self, px: f64, now: Time) -> bool {
        let max = (self.text_height() - TEXT_H).max(0.0);
        let to = (self.scroll.target() + px).clamp(0.0, max);
        if (to - self.scroll.target()).abs() < 0.01 { return false; }
        self.scroll.set(to, now);
        true
    }

    pub fn hit(&self, x: f64, y: f64, now: Time) -> Option<Hit> {
        for (hit, rect) in self.tabs.borrow().iter() {
            if inside(x, y, *rect) { return Some(*hit); }
        }
        if inside(x, y, (RAIL, CARD_Y, WIDTH - 2.0 * RAIL, HEIGHT - CARD_Y - BOTTOM)) {
            let tx = x - RAIL - CARD_PAD;
            let ty = y - CARD_Y - CARD_PAD + self.scroll.value(now);
            let s = self.text();
            let index = self.text_layout().map_or(s.len(), |l| index_at(&l, s, tx, ty));
            return Some(Hit::Text(index));
        }
        None
    }

    pub fn pointer(&mut self, x: f64, y: f64, now: Time) -> bool {
        if self.dragging {
            if let Some(Hit::Text(i)) = self.hit(x, y.clamp(CARD_Y, HEIGHT - BOTTOM), now) {
                if i != self.caret {
                    self.caret = i;
                    self.typed_at = now;
                    self.reveal_caret(now);
                    return true;
                }
            }
            return false;
        }
        let hover = self.hit(x, y, now).filter(|h| !matches!(h, Hit::Text(_)));
        if hover == self.hover { return false; }
        // moving between the chosen tab and its bin is one hover
        let same = |h: Option<Hit>| match h { Some(Hit::Tab(i) | Hit::Delete(i)) => Some(i), _ => None };
        if same(hover).is_none() || same(hover) != same(self.hover) {
            self.hover_lit = Spring::new(0.0, 0.24, 0.83);
            if hover.is_some() { self.hover_lit.set(1.0, now); }
        }
        self.hover = hover;
        let bin = same(hover) == Some(self.current);
        self.bin_lit.set(if bin { 1.0 } else { 0.0 }, now);
        true
    }

    /// A press at (x, y) (relative to the page). Returns true when it asks
    /// for the page to close.
    pub fn press(&mut self, x: f64, y: f64, shift: bool, now: Time) -> bool {
        match self.hit(x, y, now) {
            Some(Hit::Tab(i)) => self.select(i, now),
            Some(Hit::Delete(i)) => self.delete(i, now),
            Some(Hit::New) => self.new_pad(now),
            Some(Hit::Close) => return true,
            Some(Hit::Text(i)) => {
                if shift {
                    if self.anchor.is_none() { self.anchor = Some(self.caret); }
                } else {
                    self.anchor = Some(i);
                }
                self.caret = i;
                self.goal_x = None;
                self.typed_at = now;
                self.dragging = true;
            }
            None => {}
        }
        false
    }

    pub fn release(&mut self) {
        self.dragging = false;
        if self.anchor == Some(self.caret) { self.anchor = None; }
    }

    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// Lay the header out: tabs (each up to TAB_MAX, sharing what room
    /// there is), then "+", and the collapse button at the end.
    fn layout_tabs(&self, cr: &Context, renderer: &Renderer, bin_k: f64) -> Vec<(Hit, Rect, String)> {
        let mid = HEADER_TOP + HEADER_H / 2.0;
        let room = WIDTH - 2.0 * RAIL - 2.0 * (ROUND + TAB_GAP) - 6.0;
        let titles: Vec<String> = self.pads.iter().map(|p| title_of(&p.text)).collect();
        let natural: Vec<f64> = titles.iter().map(|t| {
            renderer.layout(cr, t, Font::Label).pixel_extents().1.width() as f64 + 2.0 * TAB_PAD
        }).collect();
        let n = titles.len().max(1) as f64;
        let share = ((room - (n - 1.0) * TAB_GAP) / n).max(TAB_MIN);
        let mut x = RAIL;
        let mut out = Vec::new();
        for (i, title) in titles.into_iter().enumerate() {
            let mut w = natural[i].clamp(TAB_MIN, TAB_MAX).min(share.max(TAB_MIN));
            // the chosen tab makes room for its bin as the bin comes out
            if i == self.current { w = (natural[i] + 18.0 * bin_k).clamp(TAB_MIN, TAB_MAX + 18.0); }
            if x + w > RAIL + room + 0.5 { break; }
            out.push((Hit::Tab(i), (x, mid - TAB_H / 2.0, w, TAB_H), title));
            x += w + TAB_GAP;
        }
        out.push((Hit::New, (x, mid - ROUND / 2.0, ROUND, ROUND), String::new()));
        out.push((Hit::Close, (WIDTH - RAIL - ROUND, mid - ROUND / 2.0, ROUND, ROUND), String::new()));
        out
    }

    pub fn draw(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, alpha: f64, now: Time) {
        if alpha <= 0.002 { return; }
        if self.ctx.borrow().is_none() || self.ctx.borrow().as_ref().is_some_and(|c| !c.eq(renderer.pango_context())) {
            *self.ctx.borrow_mut() = Some(renderer.pango_context().clone());
            *self.layout.borrow_mut() = None;
        }
        let lit = self.hover_lit.value(now).clamp(0.0, 1.0);
        let arrive = |row: f64| {
            let k = crate::island::smoothstep((now - self.opened - 0.04 * row) / 0.3);
            (alpha * k, 8.0 * (1.0 - k))
        };
        // --- the header: tabs, "+", collapse ---
        let current = self.current;
        let bin_k = if self.armed.is_some_and(|(a, _)| a == current) {
            1.0
        } else {
            self.bin_lit.value(now).clamp(0.0, 1.0)
        };
        let tabs = self.layout_tabs(cr, renderer, bin_k);
        let (ha, hdy) = arrive(0.0);
        let mut rects = Vec::new();
        for (hit, (tx, ty, tw, th), title) in &tabs {
            let (tx, ty) = (x + tx, y + ty + hdy);
            let hovered = self.hover == Some(*hit);
            match hit {
                Hit::Tab(i) => {
                    let chosen = *i == self.current;
                    let armed = self.armed.is_some_and(|(a, _)| a == *i);
                    let wash = if chosen { 0.14 } else { 0.0 } + if hovered { 0.07 * lit } else { 0.0 };
                    if wash > 0.0 {
                        menu_style::rounded(cr, tx, ty, *tw, *th, th / 2.0);
                        cr.set_source_rgba(1.0, 1.0, 1.0, wash * ha);
                        let _ = cr.fill();
                    }
                    let bin = chosen && bin_k > 0.01;
                    let text_w = tw - 2.0 * TAB_PAD - 18.0 * if chosen { bin_k } else { 0.0 };
                    let lh = menu_style::line_height(cr, renderer, Font::Label);
                    let (ink, a) = if armed {
                        (menu_style::RED, 1.0)
                    } else if chosen {
                        ((1.0, 1.0, 1.0), 1.0)
                    } else {
                        ((1.0, 1.0, 1.0), menu_style::SECONDARY)
                    };
                    let label = if armed { "Удалить?" } else { title.as_str() };
                    menu_style::text(cr, renderer, label, tx + TAB_PAD, ty + (th - lh) / 2.0, text_w, Font::Label, ink, ha * a);
                    rects.push((*hit, (tx - x, ty - y - hdy, *tw, *th)));
                    if bin {
                        let (bx, by) = (tx + tw - TAB_PAD - 7.0, ty + th / 2.0);
                        let hot = self.hover == Some(Hit::Delete(*i)) || armed;
                        trash_symbol(cr, bx, by, if armed { menu_style::RED } else { (1.0, 1.0, 1.0) },
                            ha * bin_k * if hot { 1.0 } else { 0.55 });
                        // the bin's own target sits in front of the tab's
                        rects.insert(0, (Hit::Delete(*i), (bx - 9.0 - x, by - 11.0 - y - hdy, 18.0, 22.0)));
                    }
                }
                Hit::New => {
                    let (cx, cy) = (tx + tw / 2.0, ty + th / 2.0);
                    menu_style::disc(cr, cx, cy, tw / 2.0, (1.0, 1.0, 1.0, (0.075 + if hovered { 0.07 * lit } else { 0.0 }) * ha));
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.85 * ha);
                    cr.set_line_width(1.8);
                    cr.set_line_cap(cairo::LineCap::Round);
                    cr.move_to(cx - 5.5, cy);
                    cr.line_to(cx + 5.5, cy);
                    cr.move_to(cx, cy - 5.5);
                    cr.line_to(cx, cy + 5.5);
                    let _ = cr.stroke();
                    rects.push((*hit, (tx - x, ty - y - hdy, *tw, *th)));
                }
                Hit::Close => {
                    let h = if hovered { lit } else { 0.0 };
                    collapse(cr, tx, ty, *tw, h, ha);
                    rects.push((*hit, (tx - x, ty - y - hdy, *tw, *th)));
                }
                _ => {}
            }
        }
        *self.tabs.borrow_mut() = rects;

        // --- the card with the text ---
        let (ca, cdy) = arrive(1.0);
        let (cx, cy, cw, ch) = (x + RAIL, y + CARD_Y + cdy, WIDTH - 2.0 * RAIL, HEIGHT - CARD_Y - BOTTOM);
        menu_style::rounded(cr, cx, cy, cw, ch, CARD_R);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.065 * ca);
        let _ = cr.fill();
        if !self.loaded && !self.offline {
            menu_style::spinner(cr, cx + cw / 2.0, cy + ch / 2.0, 9.0, now, ca * 0.6);
            return;
        }
        let (tx, ty) = (cx + CARD_PAD, cy + CARD_PAD);
        cr.save().unwrap();
        cr.rectangle(cx, cy + 4.0, cw, ch - 8.0);
        cr.clip();
        // switching pads, the old text leaves the way it goes and the new
        // one comes in after it
        let switch = self.switch.filter(|(_, _, t)| now - t < SWITCH);
        let k = switch.map_or(1.0, |(_, _, t)| min_jerk(((now - t) / SWITCH).clamp(0.0, 1.0)));
        let dir = switch.map_or(0.0, |(_, d, _)| d);
        if let Some((from, _, _)) = switch {
            if let Some(old) = self.pads.get(from) {
                let a = ca * (1.0 - (k / 0.55).min(1.0));
                if a > 0.0 {
                    let layout = renderer.layout_ellipsized(cr, &old.text, Font::Body, TEXT_W, 14);
                    cr.set_source_rgba(1.0, 1.0, 1.0, a);
                    cr.move_to(renderer.snap(tx - 44.0 * dir * k), renderer.snap(ty));
                    pangocairo::functions::show_layout(cr, &layout);
                }
            }
        }
        let a = ca * if switch.is_some() { crate::island::smoothstep((k - 0.3) / 0.7) } else { 1.0 };
        let shift = 44.0 * dir * (1.0 - k);
        let scroll = self.scroll.value(now);
        let text = self.text();
        if text.is_empty() {
            menu_style::text(cr, renderer, "Начните писать…", tx + shift, ty, TEXT_W, Font::Body, (1.0, 1.0, 1.0), a * menu_style::TERTIARY);
        }
        if let Some(layout) = self.text_layout() {
            let ox = renderer.snap(tx + shift);
            let oy = renderer.snap(ty - scroll);
            // the selection, line by line
            if let Some((s0, s1)) = self.selection() {
                let mut iter = layout.iter();
                loop {
                    let line = iter.line_readonly();
                    let (top, bottom) = iter.line_yrange();
                    if let Some(line) = line {
                        let start = line.start_index() as usize;
                        let end = start + line.length() as usize;
                        if s1 > start && s0 <= end {
                            let ranges = line.x_ranges(s0.max(start) as i32, s1.min(end + 1) as i32);
                            for pair in ranges.chunks(2) {
                                if let [a0, a1] = pair {
                                    let scale = pango::SCALE as f64;
                                    let (rx0, rx1) = (*a0 as f64 / scale, *a1 as f64 / scale);
                                    cr.rectangle(ox + rx0, oy + top as f64 / scale, (rx1 - rx0).max(4.0), (bottom - top) as f64 / scale);
                                }
                            }
                        }
                    }
                    if !iter.next_line() { break; }
                }
                cr.set_source_rgba(BLUE.0, BLUE.1, BLUE.2, 0.38 * a);
                let _ = cr.fill();
            }
            cr.set_source_rgba(1.0, 1.0, 1.0, a);
            cr.move_to(ox, oy);
            pangocairo::functions::show_layout(cr, &layout);
            // the caret: lit after a move, then blinking
            let since = now - self.typed_at;
            let on = since < BLINK || ((since - BLINK) / BLINK).floor() as i64 % 2 == 1;
            if on && self.selection().is_none() && switch.is_none() {
                let pos = layout.index_to_pos(self.caret.min(text.len()) as i32);
                let scale = pango::SCALE as f64;
                let px = renderer.snap(ox + pos.x() as f64 / scale);
                let line_h = (pos.height() as f64 / scale).max(17.0);
                cr.rectangle(px - 1.0, oy + pos.y() as f64 / scale - 1.0, 2.0, line_h - 1.0);
                cr.set_source_rgba(BLUE.0, BLUE.1, BLUE.2, a);
                let _ = cr.fill();
            }
            let content = layout.extents().1.height() as f64 / pango::SCALE as f64;
            let max = (content - TEXT_H).max(0.0);
            cr.restore().unwrap();
            let top = (scroll / 24.0).clamp(0.0, 1.0);
            let bottom = ((max - scroll) / 24.0).clamp(0.0, 1.0);
            menu_style::rounded(cr, cx, cy, cw, ch, CARD_R);
            cr.clip();
            card_fades(cr, cx, cy, cw, ch, top * ca, bottom * ca);
            cr.reset_clip();
        } else {
            cr.restore().unwrap();
        }
        if let Some(error) = &self.error {
            let lh = menu_style::line_height(cr, renderer, Font::Footnote);
            menu_style::text(cr, renderer, error, cx + CARD_PAD, cy + ch - CARD_PAD - lh, cw - 2.0 * CARD_PAD, Font::Footnote,
                menu_style::RED, ca);
        }
    }
}

const TEXT_W: f64 = WIDTH - 2.0 * RAIL - 2.0 * CARD_PAD;
const TEXT_H: f64 = HEIGHT - CARD_Y - BOTTOM - 2.0 * CARD_PAD;

/// The byte index nearest (x, y) in the layout's coordinates.
fn index_at(layout: &pango::Layout, s: &str, x: f64, y: f64) -> usize {
    let scale = pango::SCALE as f64;
    let (_, index, trailing) = layout.xy_to_index((x * scale) as i32, (y.max(0.0) * scale) as i32);
    let mut i = (index.max(0) as usize).min(s.len());
    for _ in 0..trailing.max(0) { i = next_char(s, i); }
    // a click past a line's end lands before its newline, not after it
    i.min(s.len())
}

/// The card's text dissolving into its top and bottom while there is more.
fn card_fades(cr: &Context, x: f64, y: f64, w: f64, h: f64, top: f64, bottom: f64) {
    let depth = 22.0;
    for (amount, from, to) in [(top, y, y + depth), (bottom, y + h, y + h - depth)] {
        if amount <= 0.0 { continue; }
        // into the card's own grey on black
        let g = cairo::LinearGradient::new(0.0, from, 0.0, to);
        g.add_color_stop_rgba(0.0, 0.065, 0.065, 0.065, amount);
        g.add_color_stop_rgba(1.0, 0.065, 0.065, 0.065, 0.0);
        let _ = cr.set_source(&g);
        cr.rectangle(x, from.min(to), w, depth);
        let _ = cr.fill();
    }
}

/// The open-island page's end-of-row control: a faint ⋯ that gathers into
/// the collapse chevron under the pointer.
fn collapse(cr: &Context, x: f64, y: f64, size: f64, lit: f64, alpha: f64) {
    let (cx, cy) = (x + size / 2.0, y + size / 2.0);
    if lit > 0.0 {
        menu_style::rounded(cr, x, y, size, size, 8.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.09 * lit * alpha);
        let _ = cr.fill();
    }
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

/// trash: a lid with a handle over a tapered can with two ribs.
pub fn trash_symbol(cr: &Context, cx: f64, cy: f64, colour: (f64, f64, f64), alpha: f64) {
    cr.save().unwrap();
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
    cr.set_line_width(1.4);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.new_path();
    cr.move_to(cx - 5.0, cy - 3.5);
    cr.line_to(cx + 5.0, cy - 3.5);
    cr.move_to(cx - 1.8, cy - 3.5);
    cr.line_to(cx - 1.4, cy - 5.3);
    cr.line_to(cx + 1.4, cy - 5.3);
    cr.line_to(cx + 1.8, cy - 3.5);
    cr.move_to(cx - 3.8, cy - 2.0);
    cr.line_to(cx - 3.2, cy + 5.2);
    cr.line_to(cx + 3.2, cy + 5.2);
    cr.line_to(cx + 3.8, cy - 2.0);
    cr.move_to(cx - 1.2, cy - 0.5);
    cr.line_to(cx - 1.0, cy + 3.4);
    cr.move_to(cx + 1.2, cy - 0.5);
    cr.line_to(cx + 1.0, cy + 3.4);
    let _ = cr.stroke();
    cr.restore().unwrap();
}

/// note.text: a page with its corner folded and three lines.
pub fn symbol(cr: &Context, cx: f64, cy: f64) {
    cr.save().unwrap();
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    let (w, h) = (17.0, 21.0);
    let (x0, y0) = (cx - w / 2.0, cy - h / 2.0);
    menu_style::rounded(cr, x0, y0, w, h, 4.0);
    let _ = cr.stroke();
    for (i, len) in [11.0, 11.0, 7.0].iter().enumerate() {
        let ly = y0 + 6.0 + i as f64 * 4.5;
        cr.move_to(x0 + 3.5, ly);
        cr.line_to(x0 + 3.5 + len - 1.0, ly);
    }
    let _ = cr.stroke();
    cr.restore().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_edits_moves_and_selects() {
        let mut n = Notes::offline(&["hello world"], 0.0);
        n.caret = 11;
        n.key("", Some("!"), false, false, 1.0);
        assert_eq!(n.pads[0].text, "hello world!");
        n.key("BackSpace", None, true, false, 1.1);
        assert_eq!(n.pads[0].text, "hello ");
        n.key("Return", None, false, false, 1.2);
        n.key("", Some("ещё"), false, false, 1.3);
        assert_eq!(n.pads[0].text, "hello \nещё");
        n.key("Left", None, false, true, 1.4);
        n.key("Left", None, false, true, 1.5);
        assert_eq!(n.selection(), Some(("hello \nе".len(), "hello \nещё".len())));
        n.key("", Some("x"), false, false, 1.6);
        assert_eq!(n.pads[0].text, "hello \nеx");
        n.key("Home", None, false, false, 1.7);
        assert_eq!(n.caret, "hello \n".len());
        n.key("Up", None, false, false, 1.8);
        assert_eq!(n.caret, 0);
        n.key("a", None, true, false, 1.9);
        n.key("Delete", None, false, false, 2.0);
        assert_eq!(n.pads[0].text, "");
    }

    #[test]
    fn titles_come_from_the_first_line() {
        assert_eq!(title_of("\n# Покупки\n- хлеб"), "Покупки");
        assert_eq!(title_of("   "), "Новая заметка");
    }

    #[test]
    fn edits_save_after_a_pause_and_an_emptied_pad_goes() {
        let mut n = Notes::offline(&["one", "two"], 0.0);
        n.current = 1;
        n.caret = 3;
        n.key("", Some("!"), false, false, 1.0);
        assert!(n.has_unsaved());
        assert_eq!(n.deadline(1.1, false), Some(1.0 + SAVE_AFTER));
        n.advance(1.0 + SAVE_AFTER);
        assert!(!n.has_unsaved());
        // emptied and left, the pad is taken away
        n.key("a", None, true, false, 2.0);
        n.key("BackSpace", None, false, false, 2.1);
        n.select(0, 2.2);
        assert_eq!(n.pads.len(), 1);
        assert_eq!(n.pads[0].text, "one");
        // a new pad, and a second "new" while it is empty does nothing
        n.new_pad(3.0);
        n.new_pad(3.1);
        assert_eq!(n.pads.len(), 2);
        assert_eq!(n.current, 1);
        n.key("Tab", None, true, false, 3.2);
        assert_eq!((n.pads.len(), n.current), (1, 0), "the empty one went as it was left");
    }

    #[test]
    fn the_bin_wants_a_second_press() {
        let mut n = Notes::offline(&["a", "b", "c"], 0.0);
        n.current = 1;
        n.delete(1, 1.0);
        assert_eq!(n.pads.len(), 3);
        n.delete(1, 1.5);
        assert_eq!(n.pads.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["a", "c"]);
        // a press long after the first starts over
        n.delete(0, 2.0);
        n.delete(0, 2.0 + ARM_FOR + 0.1);
        assert_eq!(n.pads.len(), 2);
    }
}
