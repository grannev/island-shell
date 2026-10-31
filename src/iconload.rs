//! Icons for the launcher: an icon name from a desktop entry resolved
//! through the user's icon theme and loaded as a cairo surface. PNGs are
//! read directly; SVG is rendered with resvg and XPM through ImageMagick,
//! both cached as PNG in `~/.cache/island-shell/icons` so it happens once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Size directories to try in a theme, largest first (we scale down,
/// never up); "" is for themes that drop the size level altogether.
const SIZES: [&str; 11] =
    ["256x256", "192x192", "128x128", "96x96", "64x64", "48x48", "scalable", "symbolic", "32x32", "512x512", ""];

/// Main-thread cache, shared by every launcher opening.
pub struct Icons {
    worker: crate::background::Worker<(String, i32), Option<crate::background::Pixels>>,
    pending: RefCell<HashSet<(String, i32)>>,
    ready: RefCell<HashMap<(String, i32), Option<cairo::ImageSurface>>>,
}

impl Icons {
    pub fn new() -> Self {
        // Construct the non-Send Cairo cache on the worker, not on the UI thread.
        let worker = crate::background::Worker::new("launcher-icons", {
            // The loader is thread-local so no Cairo object crosses threads.
            move |(name, px): (String, i32)| {
                thread_local! { static LOADER: Loader = Loader::new(); }
                LOADER.with(|loader| loader.get_scaled(&name, px).and_then(crate::background::Pixels::from_surface))
            }
        });
        Self { worker, pending: RefCell::new(HashSet::new()), ready: RefCell::new(HashMap::new()) }
    }
    pub fn set_waker(&self, tx: smithay_client_toolkit::reexports::calloop::channel::Sender<()>) {
        self.worker.set_waker(tx);
    }
    pub fn drain(&self) -> bool {
        let results = self.worker.drain();
        let changed = !results.is_empty();
        let mut ready = self.ready.borrow_mut();
        for (key, pixels) in results {
            self.pending.borrow_mut().remove(&key);
            if ready.len() >= 256 {
                if let Some(oldest) = ready.keys().next().cloned() {
                    ready.remove(&oldest);
                }
            }
            ready.insert(key, pixels.and_then(crate::background::Pixels::into_surface));
        }
        changed
    }
    pub fn get_scaled(&self, name: &str, px: i32) -> Option<cairo::ImageSurface> {
        let key = (name.to_string(), px.clamp(1, 512));
        if let Some(surface) = self.ready.borrow().get(&key) {
            return surface.clone();
        }
        if !self.pending.borrow().contains(&key) && self.worker.request(key.clone()) {
            self.pending.borrow_mut().insert(key);
        }
        None
    }
}

pub(crate) struct Loader {
    themes: Vec<String>,
    bases: Vec<PathBuf>,
    cache: RefCell<HashMap<String, Option<cairo::ImageSurface>>>,
    /// icons already scaled to the size the list draws them at: rescaling
    /// a 256 px PNG down to 57 px on every frame is the one thing in the
    /// launcher that actually costs something
    scaled: RefCell<HashMap<(String, i32), Option<cairo::ImageSurface>>>,
}

impl Loader {
    pub(crate) fn new() -> Loader {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut bases = vec![home.join(".local/share/icons"), home.join(".icons")];
        let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        bases.extend(system.split(':').filter(|s| !s.is_empty()).map(|d| PathBuf::from(d).join("icons")));

        // the GTK theme the desktop uses, then whatever it inherits, then
        // the standard fallbacks
        let mut themes = Vec::new();
        if let Some(theme) = gtk_theme(&home) {
            themes.push(theme);
        }
        let mut i = 0;
        while i < themes.len() {
            let inherits = index_inherits(&bases, &themes[i]);
            for t in inherits {
                if !themes.contains(&t) {
                    themes.push(t);
                }
            }
            i += 1;
        }
        for t in ["hicolor", "Adwaita"] {
            if !themes.iter().any(|x| x == t) {
                themes.push(t.into());
            }
        }
        Loader { themes, bases, cache: RefCell::new(HashMap::new()), scaled: RefCell::new(HashMap::new()) }
    }

    /// The icon for `name`, loaded and cached (`None` when there is none).
    pub fn get(&self, name: &str) -> Option<cairo::ImageSurface> {
        if let Some(hit) = self.cache.borrow().get(name) {
            return hit.clone();
        }
        let surface = self.resolve(name).and_then(|path| load(&path));
        if self.cache.borrow().len() >= 64 {
            self.cache.borrow_mut().clear();
        }
        self.cache.borrow_mut().insert(name.to_string(), surface.clone());
        surface
    }

    /// The icon at exactly `px` × `px` device pixels, rendered once.
    pub(crate) fn get_scaled(&self, name: &str, px: i32) -> Option<cairo::ImageSurface> {
        let key = (name.to_string(), px);
        if let Some(hit) = self.scaled.borrow().get(&key) {
            return hit.clone();
        }
        let scaled = self.get(name).and_then(|source| {
            let target = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).ok()?;
            let cr = cairo::Context::new(&target).ok()?;
            let scale = px as f64 / source.width().max(source.height()) as f64;
            cr.translate(
                (px as f64 - source.width() as f64 * scale) / 2.0,
                (px as f64 - source.height() as f64 * scale) / 2.0,
            );
            cr.scale(scale, scale);
            cr.set_source_surface(&source, 0.0, 0.0).ok()?;
            if let Ok(pattern) = TryInto::<cairo::SurfacePattern>::try_into(cr.source()) {
                pattern.set_filter(cairo::Filter::Good);
            }
            cr.paint().ok()?;
            drop(cr);
            target.flush();
            Some(target)
        });
        if self.scaled.borrow().len() >= 256 {
            self.scaled.borrow_mut().clear();
        }
        self.scaled.borrow_mut().insert(key, scaled.clone());
        scaled
    }

    fn resolve(&self, name: &str) -> Option<PathBuf> {
        if name.starts_with('/') {
            let path = PathBuf::from(name);
            return path.exists().then_some(path);
        }
        for theme in &self.themes {
            for base in &self.bases {
                for size in SIZES {
                    let dir = if size.is_empty() { base.join(theme) } else { base.join(theme).join(size) };
                    for category in ["apps", "status", "devices", "mimetypes", "places", "actions"] {
                        for ext in ["png", "svg", "xpm"] {
                            let path = dir.join(category).join(format!("{name}.{ext}"));
                            if path.exists() {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        // the old pixmaps directory, and the name as given
        for dir in ["/usr/share/pixmaps", "/usr/local/share/pixmaps"] {
            for ext in ["png", "svg", "xpm"] {
                let path = Path::new(dir).join(format!("{name}.{ext}"));
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }
}

fn gtk_theme(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join(".config/gtk-3.0/settings.ini")).ok()?;
    text.lines().find_map(|l| l.trim().strip_prefix("gtk-icon-theme-name=").map(|v| v.trim().to_string()))
}

fn index_inherits(bases: &[PathBuf], theme: &str) -> Vec<String> {
    for base in bases {
        let Ok(text) = std::fs::read_to_string(base.join(theme).join("index.theme")) else { continue };
        if let Some(line) = text.lines().find_map(|l| l.trim().strip_prefix("Inherits=")) {
            return line.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
    }
    Vec::new()
}

/// Load a PNG straight, anything else through a rasterised copy in the
/// cache directory (rendered once).
pub(crate) fn load(path: &Path) -> Option<cairo::ImageSurface> {
    let png =
        if path.extension().and_then(|e| e.to_str()) == Some("png") { path.to_path_buf() } else { rasterise(path)? };
    let mut file = std::fs::File::open(&png).ok()?;
    cairo::ImageSurface::create_from_png(&mut file).ok()
}

/// Size the cached rasterisations are rendered at.
const RASTER: u32 = 128;

fn rasterise(path: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cache"))
        .join("island-shell")
        .join("icons");
    std::fs::create_dir_all(&dir).ok()?;
    let stem = path.file_stem()?.to_string_lossy().to_string();
    // Equal basenames in different themes are different icons. Also invalidate
    // a raster when its source is replaced, without scanning/deleting caches.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    if let Ok(metadata) = path.metadata() {
        metadata.len().hash(&mut hash);
        metadata.modified().ok().hash(&mut hash);
    }
    let out = dir.join(format!("{stem}-{:016x}-{RASTER}.png", hash.finish()));
    if out.exists() {
        return Some(out);
    }
    let ok = match path.extension().and_then(|e| e.to_str()) {
        Some("svg") => render_svg(path, &out),
        // XPM and anything else: ImageMagick, if it is around
        _ => Command::new("magick")
            .arg("-background")
            .arg("none")
            .arg(path)
            .args(["-resize", &format!("{RASTER}x{RASTER}")])
            .arg(&out)
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
    };
    (ok && out.exists()).then_some(out)
}

/// Render an SVG to a square PNG with resvg (no system library needed).
fn render_svg(path: &Path, out: &Path) -> bool {
    let Ok(data) = std::fs::read(path) else { return false };
    let mut options = resvg::usvg::Options::default();
    // Most icons are paths, not text. Loading every installed font for each
    // SVG used to take longer than a whole animation frame.
    if data.windows(5).any(|part| part == b"<text") {
        thread_local! {
            static FONTS: std::sync::Arc<resvg::usvg::fontdb::Database> = {
                let mut db = resvg::usvg::fontdb::Database::new();
                db.load_system_fonts();
                std::sync::Arc::new(db)
            };
        }
        options.fontdb = FONTS.with(Clone::clone);
    }
    options.resources_dir = path.parent().map(Path::to_path_buf);
    let Ok(tree) = resvg::usvg::Tree::from_data(&data, &options) else { return false };
    let size = tree.size();
    let scale = RASTER as f32 / size.width().max(size.height()).max(1.0);
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1)) else { return false };
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.save_png(out).is_ok()
}
