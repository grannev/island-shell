//! The shelf, after Vorssaint's file tray: files dragged onto the island
//! wait here until they are dragged out again, as references (nothing is
//! copied). A drag that reaches the island unfolds the shelf with its drop
//! zone lit; letting go adds the files, which pop in one after another.
//! A tile opens its file on a click and can be dragged out to any
//! application (the shelf folds out of the way); the × on a hovered tile
//! takes it off. «Сжать» packs everything into a zip in ~/Downloads,
//! «Пути» copies the paths, «Очистить» (on a second press) empties it.
//!
//! The second tab is «Загрузки»: the newest files in ~/Downloads, with
//! downloads still running first (a ring round their icon).
//!
//! The list survives restarts in ~/.local/state/island-shell/shelf, one
//! path a line. Scanning ~/Downloads, zipping and saving the list run on
//! the page's worker.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cairo::Context;

use crate::anim::{Spring, Time};
use crate::background::Worker;
use crate::island::{Font, Renderer};
use crate::menu_style;

pub const WIDTH: f64 = 560.0;
const RAIL: f64 = 24.0;
const HEADER_TOP: f64 = 12.0;
const HEADER_H: f64 = 36.0;
const GRID_TOP: f64 = HEADER_TOP + HEADER_H + 12.0;
const BOTTOM: f64 = 16.0;
const COLS: usize = 6;
const GAP: f64 = 8.0;
const TILE_W: f64 = (WIDTH - 2.0 * RAIL - (COLS - 1) as f64 * GAP) / COLS as f64;
const TILE_H: f64 = 100.0;
const TILE_R: f64 = 16.0;
const ICON: f64 = 48.0;
const MAX_ROWS: usize = 3;
const ROUND: f64 = 28.0;
const SEG_W: f64 = 196.0;
const SEG_H: f64 = 30.0;
const ARM_FOR: f64 = 3.0;
const STATUS_FOR: f64 = 2.6;
const DRAG_SLOP: f64 = 6.0;
/// the newest downloads the second tab shows
const RECENT: usize = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Shelf,
    Downloads,
}

/// A file the page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub dir: bool,
    pub size: u64,
}

/// A download still running (from `downloads.rs`, kept by the island).
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    pub name: String,
    pub bytes: u64,
    pub speed: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Tab(Tab),
    /// a tile of the shown tab (running downloads first on «Загрузки»)
    Tile(usize),
    Remove(usize),
    Zip,
    Copy,
    Clear,
    Close,
}

#[derive(Clone, Debug)]
enum Job {
    Scan(PathBuf),
    Inspect(Vec<PathBuf>),
    Zip(Vec<PathBuf>, PathBuf),
    Save(PathBuf, Vec<PathBuf>),
}

#[derive(Debug)]
enum Done {
    Scanned(Vec<Entry>),
    Inspected(Vec<Entry>),
    Zipped(Result<PathBuf, String>),
    Saved,
}

fn entry(path: &Path) -> Option<Entry> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Entry { path: path.to_path_buf(), dir: meta.is_dir(), size: meta.len() })
}

fn run(job: Job) -> Done {
    match job {
        Job::Scan(dir) => {
            let mut found: Vec<(std::time::SystemTime, Entry)> = std::fs::read_dir(&dir).into_iter().flatten().flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    !name.starts_with('.') && !name.ends_with(".part") && !name.ends_with(".crdownload")
                })
                .filter_map(|e| {
                    let meta = e.metadata().ok()?;
                    // Firefox's empty placeholder beside a running download
                    if meta.is_file() && meta.len() == 0 { return None; }
                    Some((meta.modified().ok()?, Entry { path: e.path(), dir: meta.is_dir(), size: meta.len() }))
                })
                .collect();
            found.sort_by(|a, b| b.0.cmp(&a.0));
            Done::Scanned(found.into_iter().take(RECENT).map(|(_, e)| e).collect())
        }
        Job::Inspect(paths) => Done::Inspected(paths.iter().filter_map(|p| entry(p)).collect()),
        Job::Zip(items, out) => {
            let result = (|| -> Result<PathBuf, String> {
                // one zip run per folder the files sit in, names relative to
                // it, so the archive holds the files and not their paths
                let mut by_parent: Vec<(PathBuf, Vec<std::ffi::OsString>)> = Vec::new();
                for item in &items {
                    let (Some(parent), Some(name)) = (item.parent(), item.file_name()) else { continue };
                    match by_parent.iter_mut().find(|(p, _)| p == parent) {
                        Some((_, names)) => names.push(name.to_os_string()),
                        None => by_parent.push((parent.to_path_buf(), vec![name.to_os_string()])),
                    }
                }
                if let Some(dir) = out.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
                for (parent, names) in by_parent {
                    let status = std::process::Command::new("zip").current_dir(&parent).arg("-rqy").arg(&out).arg("--")
                        .args(&names).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status()
                        .map_err(|e| format!("zip: {e}"))?;
                    if !status.success() { return Err(format!("zip завершился с {status}")); }
                }
                Ok(out)
            })();
            Done::Zipped(result)
        }
        Job::Save(file, items) => {
            if let Some(dir) = file.parent() { let _ = std::fs::create_dir_all(dir); }
            let text: String = items.iter().map(|p| format!("{}\n", p.display())).collect();
            let tmp = file.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() { let _ = std::fs::rename(&tmp, &file); }
            Done::Saved
        }
    }
}

fn state_file() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local").join("state")
    });
    base.join("island-shell").join("shelf")
}

/// `file://` URI of a path, percent-encoded (RFC 8089).
pub fn uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The paths in a `text/uri-list` (comments and non-file URIs skipped).
pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|line| {
        let rest = line.strip_prefix("file://")?;
        // file://host/path: only the local host
        let rest = if rest.starts_with('/') { rest } else { &rest[rest.find('/')?..] };
        let bytes = rest.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
    }).collect()
}

/// The icon-theme icon for a file, by its extension.
pub fn icon_name(entry: &Entry) -> &'static str {
    if entry.dir { return "folder"; }
    let ext = entry.path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "heic" | "avif" => "image-x-generic",
        "pdf" => "application-pdf",
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" | "zst" => "package-x-generic",
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "opus" | "aac" => "audio-x-generic",
        "mp4" | "mkv" | "webm" | "avi" | "mov" => "video-x-generic",
        "doc" | "docx" | "odt" | "rtf" => "x-office-document",
        "xls" | "xlsx" | "ods" | "csv" => "x-office-spreadsheet",
        "ppt" | "pptx" | "odp" => "x-office-presentation",
        "html" | "htm" => "text-html",
        "deb" => "application-x-deb",
        "iso" | "img" => "media-optical",
        "exe" | "msi" | "appimage" | "sh" => "application-x-executable",
        _ => "text-x-generic",
    }
}

fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)
        .is_some_and(|e| matches!(e.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"))
}

/// What the island lends the page for one frame.
pub struct Look<'a> {
    /// an icon-theme icon at a size in device pixels (loaded off-thread)
    pub icon: &'a dyn Fn(&str, i32) -> Option<cairo::ImageSurface>,
    /// a picture's thumbnail, square, at a size in device pixels
    pub thumb: &'a dyn Fn(&Path, i32) -> Option<cairo::ImageSurface>,
    pub active: &'a [Active],
}

type Rect = (f64, f64, f64, f64);

fn inside(x: f64, y: f64, r: Rect) -> bool {
    x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
}

pub struct Shelf {
    pub items: Vec<Entry>,
    pub downloads: Vec<Entry>,
    pub tab: Tab,
    tab_slide: Spring,
    pub opened: Time,
    hover: Option<Hit>,
    lit: HashMap<String, Spring>,
    /// a drag hangs over the page; how lit the drop zone is
    pub drop_over: bool,
    drop_lit: Spring,
    /// when each tile arrived (it pops in)
    arrived: HashMap<PathBuf, Time>,
    removed: Vec<(Entry, usize, Time)>,
    armed_clear: Option<Time>,
    pressed: Option<(Hit, f64, f64, Time)>,
    status: Option<(String, bool, Time)>,
    pub zipping: bool,
    worker: Worker<Job, Done>,
    state: Option<PathBuf>,
    scanned: bool,
    rects: RefCell<Vec<(Hit, Rect)>>,
}

impl Shelf {
    pub fn new() -> Shelf {
        let state = state_file();
        // a few lines read once at start, never while painting
        let items = std::fs::read_to_string(&state).unwrap_or_default().lines()
            .filter(|l| !l.trim().is_empty()).map(PathBuf::from)
            .map(|p| Entry { dir: false, size: 0, path: p }).collect::<Vec<_>>();
        let shelf = Shelf::with(items, Some(state));
        let paths: Vec<PathBuf> = shelf.items.iter().map(|e| e.path.clone()).collect();
        if !paths.is_empty() { shelf.worker.request(Job::Inspect(paths)); }
        shelf
    }

    fn with(items: Vec<Entry>, state: Option<PathBuf>) -> Shelf {
        Shelf {
            items,
            downloads: Vec::new(),
            tab: Tab::Shelf,
            tab_slide: Spring::new(0.0, 0.3, 0.86),
            opened: 0.0,
            hover: None,
            lit: HashMap::new(),
            drop_over: false,
            drop_lit: Spring::new(0.0, 0.28, 0.8),
            arrived: HashMap::new(),
            removed: Vec::new(),
            armed_clear: None,
            pressed: None,
            status: None,
            zipping: false,
            worker: Worker::new("shelf", run),
            state,
            scanned: false,
            rects: RefCell::new(Vec::new()),
        }
    }

    /// For pictures and tests: these entries, nothing saved.
    #[cfg(test)]
    pub fn offline(items: Vec<Entry>, downloads: Vec<Entry>) -> Shelf {
        let mut shelf = Shelf::with(items, None);
        shelf.downloads = downloads;
        shelf.scanned = true;
        shelf
    }

    pub fn set_waker(&self, tx: smithay_client_toolkit::reexports::calloop::channel::Sender<()>) {
        self.worker.set_waker(tx);
    }

    pub fn open(&mut self, tab: Tab, now: Time) {
        self.opened = now;
        self.armed_clear = None;
        self.pressed = None;
        self.status = None;
        self.show(tab, now);
        self.tab_slide = Spring::new(if tab == Tab::Shelf { 0.0 } else { 1.0 }, 0.3, 0.86);
    }

    pub fn show(&mut self, tab: Tab, now: Time) {
        self.tab = tab;
        self.tab_slide.set(if tab == Tab::Shelf { 0.0 } else { 1.0 }, now);
        if tab == Tab::Downloads { self.rescan(); }
    }

    /// Read ~/Downloads again (the tab opened, a download ended).
    pub fn rescan(&mut self) {
        if self.state.is_some() || !self.scanned {
            self.scanned = self.worker.request(Job::Scan(crate::downloads::dir())) || self.scanned;
        }
    }

    fn save(&self) {
        if let Some(state) = &self.state {
            self.worker.request(Job::Save(state.clone(), self.items.iter().map(|e| e.path.clone()).collect()));
        }
    }

    /// Files dropped on the island (or a fresh archive): new ones join the
    /// end, popping in one after another. Returns how many were new.
    pub fn add(&mut self, paths: Vec<PathBuf>, now: Time) -> usize {
        let mut fresh = Vec::new();
        for path in paths {
            if self.items.iter().any(|e| e.path == path) || fresh.contains(&path) { continue; }
            fresh.push(path);
        }
        for (i, path) in fresh.iter().enumerate() {
            self.arrived.insert(path.clone(), now + 0.06 * i as f64);
            self.items.push(Entry { path: path.clone(), dir: false, size: 0 });
        }
        if !fresh.is_empty() {
            if self.state.is_some() { self.worker.request(Job::Inspect(fresh.clone())); }
            self.save();
            self.tab = Tab::Shelf;
            self.tab_slide.set(0.0, now);
        }
        fresh.len()
    }

    pub fn remove(&mut self, index: usize, now: Time) {
        if index >= self.items.len() { return; }
        let entry = self.items.remove(index);
        self.removed.push((entry, index, now));
        self.save();
    }

    fn clear(&mut self, now: Time) {
        if !self.armed_clear.is_some_and(|t| now - t < ARM_FOR) {
            self.armed_clear = Some(now);
            return;
        }
        self.armed_clear = None;
        for (i, e) in self.items.drain(..).enumerate() { self.removed.push((e, i, now)); }
        self.save();
    }

    /// The tiles of the shown tab.
    fn shown_count(&self, active: usize) -> usize {
        match self.tab {
            Tab::Shelf => self.items.len(),
            Tab::Downloads => active + self.downloads.len(),
        }
    }

    fn rows(&self) -> usize {
        let n = match self.tab { Tab::Shelf => self.items.len(), Tab::Downloads => self.downloads.len() + 1 };
        n.div_ceil(COLS).clamp(1, MAX_ROWS)
    }

    /// The page's height: a row for the empty drop zone, up to three of tiles.
    pub fn target_height(&self) -> f64 {
        let rows = self.rows() as f64;
        GRID_TOP + rows * TILE_H + (rows - 1.0) * GAP + BOTTOM
    }

    /// The file a tile stands for.
    pub fn path_of(&self, index: usize, active: usize) -> Option<PathBuf> {
        match self.tab {
            Tab::Shelf => self.items.get(index).map(|e| e.path.clone()),
            Tab::Downloads => index.checked_sub(active).and_then(|i| self.downloads.get(i)).map(|e| e.path.clone()),
        }
    }

    fn status(&mut self, text: &str, error: bool, now: Time) {
        self.status = Some((text.into(), error, now));
    }

    /// Replies from the worker; true when the page changed. A finished
    /// archive is returned to be put on the shelf.
    pub fn drain(&mut self, now: Time) -> (bool, Option<Result<PathBuf, String>>) {
        let mut changed = false;
        let mut zipped = None;
        for (_, done) in self.worker.drain() {
            match done {
                Done::Scanned(found) => {
                    changed |= found != self.downloads;
                    self.downloads = found;
                }
                Done::Inspected(found) => {
                    for e in found {
                        if let Some(item) = self.items.iter_mut().find(|i| i.path == e.path) { *item = e; }
                    }
                    changed = true;
                }
                Done::Zipped(result) => {
                    self.zipping = false;
                    match &result {
                        Ok(_) => self.status("Архив готов", false, now),
                        Err(e) => self.status(&format!("Не удалось сжать: {e}"), true, now),
                    }
                    zipped = Some(result);
                    changed = true;
                }
                Done::Saved => {}
            }
        }
        (changed, zipped)
    }

    pub fn advance(&mut self, now: Time) -> bool {
        let mut changed = false;
        if self.armed_clear.is_some_and(|t| now - t >= ARM_FOR) {
            self.armed_clear = None;
            changed = true;
        }
        if self.status.as_ref().is_some_and(|(_, _, t)| now - t >= STATUS_FOR) {
            self.status = None;
            changed = true;
        }
        let before = self.removed.len();
        self.removed.retain(|(_, _, t)| now - t < 0.3);
        changed | (before != self.removed.len())
    }

    pub fn deadline(&self) -> Option<Time> {
        let mut next: Option<Time> = None;
        let mut consider = |t: Time| next = Some(next.map_or(t, |n: Time| n.min(t)));
        if let Some(t) = self.armed_clear { consider(t + ARM_FOR); }
        if let Some((_, _, t)) = &self.status { consider(t + STATUS_FOR); }
        next
    }

    pub fn is_animating(&self, now: Time) -> bool {
        now - self.opened < 0.6
            || self.tab_slide.is_moving(now) || self.drop_lit.is_moving(now)
            || self.lit.values().any(|s| s.is_moving(now))
            || self.arrived.values().any(|t| now - t < 0.6)
            || !self.removed.is_empty()
            || self.pressed.is_some_and(|(_, _, _, t)| now - t < 0.4)
            || self.zipping
    }

    pub fn set_drop_over(&mut self, over: bool, now: Time) {
        self.drop_over = over;
        self.drop_lit.set(if over { 1.0 } else { 0.0 }, now);
    }

    pub fn hit(&self, x: f64, y: f64) -> Option<Hit> {
        self.rects.borrow().iter().find(|(_, r)| inside(x, y, *r)).map(|(h, _)| *h)
    }

    fn key(hit: Hit) -> String {
        match hit {
            Hit::Remove(i) => format!("tile{i}"),
            other => format!("{other:?}"),
        }
    }

    pub fn pointer(&mut self, x: f64, y: f64, now: Time) -> bool {
        let hover = self.hit(x, y);
        if hover == self.hover { return false; }
        self.hover = hover;
        for (k, s) in self.lit.iter_mut() {
            if hover.map(Shelf::key).as_deref() != Some(k.as_str()) { s.set(0.0, now); }
        }
        if let Some(h) = hover {
            let key = match h { Hit::Remove(i) => Shelf::key(Hit::Tile(i)), h => Shelf::key(h) };
            self.lit.entry(key).or_insert_with(|| Spring::new(0.0, 0.24, 0.83)).set(1.0, now);
        }
        self.lit.retain(|_, s| s.target() > 0.0 || s.is_moving(now));
        true
    }

    fn lit(&self, hit: Hit, now: Time) -> f64 {
        self.lit.get(&Shelf::key(hit)).map_or(0.0, |s| s.value(now).clamp(0.0, 1.0))
    }

    pub fn press(&mut self, x: f64, y: f64, now: Time) {
        self.pressed = self.hit(x, y).map(|h| (h, x, y, now));
    }

    /// A press moving: past a few pixels on a tile it becomes a drag of
    /// that tile's file (the island starts it and folds away).
    pub fn drag_from(&mut self, x: f64, y: f64, active: usize) -> Option<PathBuf> {
        let (hit, px, py, _) = self.pressed?;
        let Hit::Tile(i) = hit else { return None };
        if (x - px).hypot(y - py) < DRAG_SLOP { return None; }
        self.pressed = None;
        self.path_of(i, active)
    }

    /// The press let go where it started: the click. What it asks of the island.
    pub fn release(&mut self, x: f64, y: f64, active: usize, now: Time) -> Release {
        let Some((hit, _, _, _)) = self.pressed.take() else { return Release::None };
        if self.hit(x, y) != Some(hit) { return Release::None; }
        if hit != Hit::Clear { self.armed_clear = None; }
        match hit {
            Hit::Tab(tab) => { self.show(tab, now); Release::None }
            Hit::Tile(i) => match self.path_of(i, active) {
                Some(path) => Release::Open(path),
                None => Release::None,
            },
            Hit::Remove(i) => { self.remove(i, now); Release::None }
            Hit::Zip => {
                if self.items.is_empty() || self.zipping { return Release::None; }
                self.zipping = true;
                let stamp = crate::island::strftime("%Y-%m-%d %H.%M.%S");
                let out = crate::downloads::dir().join(format!("Полка {stamp}.zip"));
                if self.state.is_some() {
                    self.worker.request(Job::Zip(self.items.iter().map(|e| e.path.clone()).collect(), out));
                }
                Release::Zipping
            }
            Hit::Copy => {
                let paths: Vec<String> = self.items.iter().map(|e| e.path.display().to_string()).collect();
                if paths.is_empty() { return Release::None; }
                self.status(if paths.len() == 1 { "Путь скопирован" } else { "Пути скопированы" }, false, now);
                Release::Copy(paths.join("\n"))
            }
            Hit::Clear => { self.clear(now); Release::None }
            Hit::Close => Release::Close,
        }
    }

    /// Lay out and draw the page at (x, y).
    pub fn draw(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, h: f64, alpha: f64, look: &Look, now: Time) {
        if alpha <= 0.002 { return; }
        let mut rects: Vec<(Hit, Rect)> = Vec::new();
        let arrive = |row: f64| {
            let k = crate::island::smoothstep((now - self.opened - 0.04 * row) / 0.3);
            (alpha * k, 8.0 * (1.0 - k))
        };
        let (ha, hdy) = arrive(0.0);
        let mid = HEADER_TOP + HEADER_H / 2.0;
        // --- the tabs ---
        let seg = (RAIL, mid - SEG_H / 2.0, SEG_W, SEG_H);
        let lit = [self.lit(Hit::Tab(Tab::Shelf), now), self.lit(Hit::Tab(Tab::Downloads), now)];
        menu_style::segmented(cr, renderer, (x + seg.0, y + seg.1 + hdy, seg.2, seg.3), &["Полка", "Загрузки"],
            self.tab_slide.value(now).clamp(0.0, 1.0), &lit, ha);
        rects.push((Hit::Tab(Tab::Shelf), (seg.0, seg.1, seg.2 / 2.0, seg.3)));
        rects.push((Hit::Tab(Tab::Downloads), (seg.0 + seg.2 / 2.0, seg.1, seg.2 / 2.0, seg.3)));
        // --- actions at the end of the row, then the collapse control ---
        let close = (WIDTH - RAIL - ROUND, mid - ROUND / 2.0, ROUND, ROUND);
        collapse(cr, x + close.0, y + close.1 + hdy, ROUND, self.lit(Hit::Close, now), ha);
        rects.push((Hit::Close, close));
        let actions_k = (1.0 - self.tab_slide.value(now)).clamp(0.0, 1.0) * if self.items.is_empty() { 0.0 } else { 1.0 };
        if let Some((text, error, since)) = &self.status {
            let k = crate::island::smoothstep((now - since) / 0.2) * (1.0 - crate::island::smoothstep((now - since - STATUS_FOR + 0.3) / 0.3));
            let lh = menu_style::line_height(cr, renderer, Font::Subhead);
            let colour = if *error { menu_style::RED } else { menu_style::GREEN };
            let w = renderer.layout(cr, text, Font::Subhead).pixel_extents().1.width() as f64;
            menu_style::text(cr, renderer, text, x + close.0 - 12.0 - w, y + mid - lh / 2.0 + hdy + 3.0 * (1.0 - k), 260.0,
                Font::Subhead, colour, ha * k);
        } else if actions_k > 0.01 {
            let mut bx = close.0 - 10.0;
            for hit in [Hit::Clear, Hit::Copy, Hit::Zip] {
                bx -= ROUND;
                let rect = (bx, mid - ROUND / 2.0, ROUND, ROUND);
                let lit = self.lit(hit, now);
                let armed = hit == Hit::Clear && self.armed_clear.is_some();
                let pressed = self.pressed.filter(|(h, ..)| *h == hit).map_or(1.0, |(.., t)| menu_style::press_scale(now - t));
                let (cx, cy) = (x + rect.0 + ROUND / 2.0, y + rect.1 + ROUND / 2.0 + hdy);
                menu_style::transformed(cr, cx, cy, pressed, if armed { menu_style::wiggle_angle(now - self.armed_clear.unwrap()) } else { 0.0 }, || {
                    let fill = if armed { (menu_style::RED.0, menu_style::RED.1, menu_style::RED.2, 0.9 * ha * actions_k) }
                        else { (1.0, 1.0, 1.0, (0.075 + 0.07 * lit) * ha * actions_k) };
                    menu_style::disc(cr, cx, cy, ROUND / 2.0, fill);
                    let a = ha * actions_k * if self.zipping && hit == Hit::Zip { 0.4 } else { 0.9 };
                    match hit {
                        Hit::Zip => archive_symbol(cr, cx, cy, a),
                        Hit::Copy => copy_symbol(cr, cx, cy, a),
                        _ => crate::notes::trash_symbol(cr, cx, cy, (1.0, 1.0, 1.0), a),
                    }
                });
                rects.push((hit, rect));
                bx -= 8.0;
            }
        }

        // --- the grid ---
        let (ga, gdy) = arrive(1.0);
        let gx = x + RAIL;
        let gy = y + GRID_TOP + gdy;
        let gw = WIDTH - 2.0 * RAIL;
        let gh = h - GRID_TOP - BOTTOM;
        cr.save().unwrap();
        cr.rectangle(x, y + GRID_TOP - 4.0, WIDTH, gh + 8.0);
        cr.clip();
        let drop = self.drop_lit.value(now).clamp(0.0, 1.0);
        let slide = self.tab_slide.value(now);
        // the tabs slide sideways past each other
        for (tab, offset) in [(Tab::Shelf, -slide), (Tab::Downloads, 1.0 - slide)] {
            if offset.abs() >= 0.999 { continue; }
            let dx = offset * 60.0;
            let a = ga * (1.0 - offset.abs()).powi(2);
            match tab {
                Tab::Shelf => self.draw_shelf(cr, renderer, gx + dx, gy, gw, gh, a, drop, look, &mut rects, offset.abs() < 0.5, now),
                Tab::Downloads => self.draw_downloads(cr, renderer, gx + dx, gy, gw, gh, a, look, &mut rects, offset.abs() < 0.5, now),
            }
        }
        cr.restore().unwrap();
        *self.rects.borrow_mut() = rects;
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_shelf(&self, cr: &Context, renderer: &Renderer, gx: f64, gy: f64, gw: f64, gh: f64, alpha: f64, drop: f64,
        look: &Look, rects: &mut Vec<(Hit, Rect)>, active: bool, now: Time) {
        // the drop zone: a dashed outline, lit blue while a drag hangs over
        if self.items.is_empty() || drop > 0.01 {
            let blue = menu_style::BLUE;
            let k = if self.items.is_empty() { 1.0 } else { drop };
            menu_style::rounded(cr, gx, gy, gw, gh, TILE_R);
            cr.set_source_rgba(blue.0, blue.1, blue.2, 0.12 * drop * alpha * k);
            let _ = cr.fill_preserve();
            cr.set_dash(&[6.0, 5.0], now * 18.0 * drop);
            cr.set_line_width(1.5);
            let grey = 0.22 * (1.0 - drop);
            cr.set_source_rgba(grey + blue.0 * drop, grey + blue.1 * drop, grey + blue.2 * drop, (0.9) * alpha * k);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            if self.items.is_empty() {
                let (cx, cy) = (gx + gw / 2.0, gy + gh / 2.0 - 10.0);
                let lift = 3.0 * drop;
                tray_symbol(cr, cx, cy - lift, (1.0, 1.0, 1.0), alpha * (0.55 + 0.45 * drop));
                let lh = menu_style::line_height(cr, renderer, Font::Subhead);
                let text = if drop > 0.5 { "Отпустите, чтобы положить на полку" } else { "Перетащите сюда файлы" };
                menu_style::centred(cr, renderer, text, (gx, cy + 22.0, gw, lh), Font::Subhead, (1.0, 1.0, 1.0),
                    alpha * menu_style::SECONDARY);
                return;
            }
        }
        for (i, entry) in self.items.iter().enumerate() {
            let rect = tile_rect(i);
            let (tx, ty) = (gx + rect.0, gy + rect.1);
            if ty > gy + gh { break; }
            let born = self.arrived.get(&entry.path).map_or(1.0, |t| ((now - t) / 0.45).clamp(0.0, 1.0));
            let pop = if born < 1.0 { pop_scale(born) } else { 1.0 };
            let a = alpha * crate::island::smoothstep(born / 0.4);
            let lit = self.lit(Hit::Tile(i), now);
            let pressed = self.pressed.filter(|(h, ..)| *h == Hit::Tile(i)).map_or(1.0, |(.., t)| menu_style::press_scale(now - t));
            menu_style::transformed(cr, tx + TILE_W / 2.0, ty + TILE_H / 2.0, pop * pressed * (1.0 + 0.02 * lit), 0.0, || {
                draw_tile(cr, renderer, tx, ty, entry, None, lit, a, look);
                if lit > 0.01 {
                    // the × to take it off the shelf, top left of the icon
                    let (rx, ry) = (tx + (TILE_W - ICON) / 2.0 - 2.0, ty + 8.0);
                    let hot = self.hover == Some(Hit::Remove(i));
                    menu_style::disc(cr, rx, ry, 9.0, (0.32, 0.32, 0.34, a * lit));
                    cr.set_source_rgba(1.0, 1.0, 1.0, a * lit * if hot { 1.0 } else { 0.8 });
                    cr.set_line_width(1.6);
                    cr.set_line_cap(cairo::LineCap::Round);
                    cr.move_to(rx - 3.0, ry - 3.0);
                    cr.line_to(rx + 3.0, ry + 3.0);
                    cr.move_to(rx + 3.0, ry - 3.0);
                    cr.line_to(rx - 3.0, ry + 3.0);
                    let _ = cr.stroke();
                }
            });
            if active {
                // page coordinates, as the tile's own rect below
                let (rx, ry) = (RAIL + rect.0 + (TILE_W - ICON) / 2.0 - 2.0, GRID_TOP + rect.1 + 8.0);
                rects.push((Hit::Remove(i), (rx - 11.0, ry - 11.0, 22.0, 22.0)));
                rects.push((Hit::Tile(i), (RAIL + rect.0, GRID_TOP + rect.1, TILE_W, TILE_H)));
            }
        }
        // tiles taken off shrink away where they were
        for (entry, index, since) in &self.removed {
            let k = ((now - since) / 0.25).clamp(0.0, 1.0);
            let rect = tile_rect(*index);
            let (tx, ty) = (gx + rect.0, gy + rect.1);
            menu_style::transformed(cr, tx + TILE_W / 2.0, ty + TILE_H / 2.0, 1.0 - 0.4 * k, 0.0, || {
                draw_tile(cr, renderer, tx, ty, entry, None, 0.0, alpha * (1.0 - k), look);
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_downloads(&self, cr: &Context, renderer: &Renderer, gx: f64, gy: f64, gw: f64, gh: f64, alpha: f64,
        look: &Look, rects: &mut Vec<(Hit, Rect)>, active: bool, now: Time) {
        let running = look.active.len();
        if running == 0 && self.downloads.is_empty() {
            let lh = menu_style::line_height(cr, renderer, Font::Subhead);
            menu_style::centred(cr, renderer, "В «Загрузках» пока пусто", (gx, gy + gh / 2.0 - lh / 2.0, gw, lh), Font::Subhead,
                (1.0, 1.0, 1.0), alpha * menu_style::SECONDARY);
            return;
        }
        for i in 0..self.shown_count(running) {
            let rect = tile_rect(i);
            let (tx, ty) = (gx + rect.0, gy + rect.1);
            if ty + 4.0 > gy + gh { break; }
            let lit = self.lit(Hit::Tile(i), now);
            if let Some(a) = look.active.get(i) {
                let entry = Entry { path: PathBuf::from(&a.name), dir: false, size: a.bytes };
                draw_tile(cr, renderer, tx, ty, &entry, Some((a, now)), 0.0, alpha, look);
            } else if let Some(entry) = self.downloads.get(i - running) {
                let pressed = self.pressed.filter(|(h, ..)| *h == Hit::Tile(i)).map_or(1.0, |(.., t)| menu_style::press_scale(now - t));
                menu_style::transformed(cr, tx + TILE_W / 2.0, ty + TILE_H / 2.0, pressed * (1.0 + 0.02 * lit), 0.0, || {
                    draw_tile(cr, renderer, tx, ty, entry, None, lit, alpha, look);
                });
                if active { rects.push((Hit::Tile(i), (RAIL + rect.0, GRID_TOP + rect.1, TILE_W, TILE_H))); }
            }
        }
    }
}

/// What a click asks of the island.
#[derive(Clone, Debug, PartialEq)]
pub enum Release {
    None,
    Open(PathBuf),
    Copy(String),
    Zipping,
    Close,
}

fn tile_rect(i: usize) -> Rect {
    let (row, col) = (i / COLS, i % COLS);
    (col as f64 * (TILE_W + GAP), row as f64 * (TILE_H + GAP), TILE_W, TILE_H)
}

/// A pop: up past full size and back (a springy arrival).
fn pop_scale(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let s = 1.0 - (-6.5 * t).exp() * (std::f64::consts::TAU * 1.1 * t).cos();
    0.4 + 0.6 * s
}

#[allow(clippy::too_many_arguments)]
fn draw_tile(cr: &Context, renderer: &Renderer, tx: f64, ty: f64, entry: &Entry, running: Option<(&Active, Time)>,
    lit: f64, alpha: f64, look: &Look) {
    if alpha <= 0.002 { return; }
    if lit > 0.0 {
        menu_style::rounded(cr, tx, ty, TILE_W, TILE_H, TILE_R);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.09 * lit * alpha);
        let _ = cr.fill();
    }
    let (ix, iy) = (tx + (TILE_W - ICON) / 2.0, ty + 8.0);
    let px = (ICON * renderer.scale()).round() as i32;
    let thumb = (!entry.dir && is_image(&entry.path)).then(|| (look.thumb)(&entry.path, px)).flatten();
    let surface = thumb.clone().or_else(|| (look.icon)(icon_name(entry), px));
    match surface {
        Some(s) => {
            cr.save().unwrap();
            if thumb.is_some() {
                menu_style::rounded(cr, ix + 3.0, iy + 3.0, ICON - 6.0, ICON - 6.0, 8.0);
                cr.clip();
            }
            let k = ICON / s.width().max(1) as f64;
            cr.translate(renderer.snap(ix), renderer.snap(iy));
            cr.scale(k, k);
            let _ = cr.set_source_surface(&s, 0.0, 0.0);
            let _ = cr.paint_with_alpha(alpha);
            cr.restore().unwrap();
        }
        None => page_symbol(cr, ix + ICON / 2.0, iy + ICON / 2.0, alpha * 0.8),
    }
    let name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| entry.path.display().to_string());
    let lh = menu_style::line_height(cr, renderer, Font::Footnote);
    let ny = iy + ICON + 6.0;
    match running {
        Some((a, now)) => {
            // a running download: a ring turning round its icon, and its bytes
            let (cx, cy) = (ix + ICON / 2.0, iy + ICON / 2.0);
            menu_style::spinner(cr, cx, cy, ICON / 2.0 + 2.0, now, alpha);
            menu_style::centred(cr, renderer, &name, (tx, ny, TILE_W, lh), Font::Footnote, (1.0, 1.0, 1.0), alpha);
            let detail = crate::downloads::size_text(a.bytes);
            menu_style::centred(cr, renderer, &detail, (tx, ny + lh, TILE_W, lh), Font::Footnote, menu_style::BLUE, alpha);
        }
        None => {
            let layout = renderer.layout_ellipsized(cr, &name, Font::Footnote, TILE_W - 8.0, 2);
            let (_, ext) = layout.pixel_extents();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9 * alpha);
            layout.set_alignment(pango::Alignment::Center);
            cr.move_to(renderer.snap(tx + 4.0 + (TILE_W - 8.0 - layout.width() as f64 / pango::SCALE as f64) / 2.0), renderer.snap(ny));
            let _ = ext;
            pangocairo::functions::show_layout(cr, &layout);
            cr.new_path();
        }
    }
}

fn stroke_style(cr: &Context, alpha: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.set_line_width(1.6);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
}

/// archivebox: a lid over a box with a slot.
fn archive_symbol(cr: &Context, cx: f64, cy: f64, alpha: f64) {
    stroke_style(cr, alpha);
    menu_style::rounded(cr, cx - 7.0, cy - 6.0, 14.0, 4.0, 1.2);
    let _ = cr.stroke();
    cr.move_to(cx - 6.0, cy - 2.0);
    cr.line_to(cx - 6.0, cy + 6.0);
    cr.line_to(cx + 6.0, cy + 6.0);
    cr.line_to(cx + 6.0, cy - 2.0);
    cr.move_to(cx - 2.0, cy + 1.0);
    cr.line_to(cx + 2.0, cy + 1.0);
    let _ = cr.stroke();
}

/// doc.on.doc: two pages, one behind the other.
fn copy_symbol(cr: &Context, cx: f64, cy: f64, alpha: f64) {
    stroke_style(cr, alpha);
    menu_style::rounded(cr, cx - 3.5, cy - 4.5, 9.0, 11.0, 2.0);
    let _ = cr.stroke();
    cr.move_to(cx - 6.0, cy + 3.0);
    cr.line_to(cx - 6.0, cy - 5.0);
    cr.curve_to(cx - 6.0, cy - 6.5, cx - 5.5, cy - 7.0, cx - 4.0, cy - 7.0);
    cr.line_to(cx + 2.0, cy - 7.0);
    let _ = cr.stroke();
}

/// tray: an open box seen from the front, for the empty drop zone and the
/// sections gallery.
pub fn tray_symbol(cr: &Context, cx: f64, cy: f64, colour: (f64, f64, f64), alpha: f64) {
    cr.save().unwrap();
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.move_to(cx - 11.0, cy + 1.0);
    cr.line_to(cx - 7.5, cy - 8.0);
    cr.line_to(cx + 7.5, cy - 8.0);
    cr.line_to(cx + 11.0, cy + 1.0);
    cr.line_to(cx + 11.0, cy + 7.0);
    cr.curve_to(cx + 11.0, cy + 8.5, cx + 10.0, cy + 9.5, cx + 8.5, cy + 9.5);
    cr.line_to(cx - 8.5, cy + 9.5);
    cr.curve_to(cx - 10.0, cy + 9.5, cx - 11.0, cy + 8.5, cx - 11.0, cy + 7.0);
    cr.close_path();
    cr.move_to(cx - 11.0, cy + 1.0);
    cr.line_to(cx - 4.5, cy + 1.0);
    cr.curve_to(cx - 4.0, cy + 4.0, cx + 4.0, cy + 4.0, cx + 4.5, cy + 1.0);
    cr.line_to(cx + 11.0, cy + 1.0);
    let _ = cr.stroke();
    cr.restore().unwrap();
}

/// arrow.down.circle, for the gallery's «Загрузки».
pub fn download_symbol(cr: &Context, cx: f64, cy: f64) {
    cr.save().unwrap();
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.new_path();
    cr.arc(cx, cy, 10.5, 0.0, std::f64::consts::TAU);
    cr.move_to(cx, cy - 5.0);
    cr.line_to(cx, cy + 5.0);
    cr.move_to(cx - 4.0, cy + 1.0);
    cr.line_to(cx, cy + 5.0);
    cr.line_to(cx + 4.0, cy + 1.0);
    let _ = cr.stroke();
    cr.restore().unwrap();
}

/// A plain page, for files without a theme icon.
fn page_symbol(cr: &Context, cx: f64, cy: f64, alpha: f64) {
    stroke_style(cr, alpha);
    cr.move_to(cx - 12.0, cy - 17.0);
    cr.line_to(cx + 5.0, cy - 17.0);
    cr.line_to(cx + 13.0, cy - 9.0);
    cr.line_to(cx + 13.0, cy + 17.0);
    cr.line_to(cx - 12.0, cy + 17.0);
    cr.close_path();
    let _ = cr.stroke();
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn e(p: &str) -> Entry {
        Entry { path: PathBuf::from(p), dir: false, size: 1 }
    }

    #[test]
    fn uri_lists_round_trip() {
        let path = PathBuf::from("/home/u/Мои файлы/a b#1.pdf");
        let list = format!("# from a file manager\r\n{}\r\nhttps://example.com/x\r\n", uri(&path));
        assert_eq!(parse_uri_list(&list), vec![path]);
        assert_eq!(parse_uri_list("file://localhost/tmp/x"), vec![PathBuf::from("/tmp/x")]);
    }

    #[test]
    fn drops_add_new_files_once_and_the_bin_wants_a_second_press() {
        let mut shelf = Shelf::offline(vec![e("/a")], Vec::new());
        assert_eq!(shelf.add(vec![PathBuf::from("/a"), PathBuf::from("/b"), PathBuf::from("/b")], 1.0), 1);
        assert_eq!(shelf.items.len(), 2);
        shelf.clear(2.0);
        assert_eq!(shelf.items.len(), 2);
        shelf.clear(2.5);
        assert!(shelf.items.is_empty());
    }

    #[test]
    fn a_tile_pressed_and_moved_becomes_a_drag() {
        let mut shelf = Shelf::offline(vec![e("/a/one.txt")], Vec::new());
        shelf.rects.borrow_mut().push((Hit::Tile(0), (24.0, 60.0, TILE_W, TILE_H)));
        shelf.press(40.0, 80.0, 1.0);
        assert_eq!(shelf.drag_from(42.0, 81.0, 0), None);
        assert_eq!(shelf.drag_from(52.0, 90.0, 0), Some(PathBuf::from("/a/one.txt")));
        // a press let go in place is a click that opens the file
        shelf.press(40.0, 80.0, 2.0);
        assert_eq!(shelf.release(40.0, 80.0, 0, 2.1), Release::Open(PathBuf::from("/a/one.txt")));
    }
}
#[cfg(test)]
mod hit_tests {
    use super::*;

    /// The × sits on the tile's icon, where it is drawn: a click on it
    /// takes the file off (it once landed on the tile and opened it).
    #[test]
    fn the_x_on_a_tile_takes_it_off_and_the_tile_opens() {
        let entry = |p: &str| Entry { path: p.into(), dir: false, size: 1 };
        let mut shelf = Shelf::offline(vec![entry("/a/one.txt"), entry("/a/two.txt")], Vec::new());
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 900, 600).unwrap();
        let cr = Context::new(&surface).unwrap();
        let none = |_: &str, _: i32| None;
        let no_thumb = |_: &Path, _: i32| None;
        let look = Look { icon: &none, thumb: &no_thumb, active: &[] };
        shelf.open(Tab::Shelf, 0.0);
        shelf.draw(&cr, &renderer, 0.0, 0.0, 200.0, 1.0, &look, 1.0);
        let (rx, ry) = (RAIL + (TILE_W - ICON) / 2.0 - 2.0, GRID_TOP + 8.0);
        shelf.press(rx, ry, 1.2);
        assert_eq!(shelf.release(rx, ry, 0, 1.3), Release::None);
        assert_eq!(shelf.items.iter().map(|e| e.path.to_str().unwrap()).collect::<Vec<_>>(), ["/a/two.txt"]);
        shelf.draw(&cr, &renderer, 0.0, 0.0, 200.0, 1.0, &look, 2.0);
        let (tx, ty) = (RAIL + TILE_W / 2.0, GRID_TOP + TILE_H - 20.0);
        shelf.press(tx, ty, 2.1);
        assert_eq!(shelf.release(tx, ty, 0, 2.2), Release::Open("/a/two.txt".into()));
    }
}
