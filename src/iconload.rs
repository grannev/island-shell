//! Icons for the launcher: an icon name from a desktop entry resolved
//! through the user's icon theme and loaded as a cairo surface. PNGs are
//! read directly; SVG is rendered with resvg and XPM through ImageMagick,
//! both cached as PNG in `~/.cache/island-shell/icons` so it happens once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Size directories to try in a theme, largest first (we scale down,
/// never up); "" is for themes that drop the size level altogether.
const SIZES: [&str; 11] =
    ["256x256", "192x192", "128x128", "96x96", "64x64", "48x48", "scalable", "symbolic", "32x32", "512x512", ""];

pub struct Icons {
    themes: Vec<String>,
    bases: Vec<PathBuf>,
    cache: RefCell<HashMap<String, Option<cairo::ImageSurface>>>,
}

impl Icons {
    pub fn new() -> Icons {
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
        Icons { themes, bases, cache: RefCell::new(HashMap::new()) }
    }

    /// The icon for `name`, loaded and cached (`None` when there is none).
    pub fn get(&self, name: &str) -> Option<cairo::ImageSurface> {
        if let Some(hit) = self.cache.borrow().get(name) {
            return hit.clone();
        }
        let surface = self.resolve(name).and_then(|path| load(&path));
        self.cache.borrow_mut().insert(name.to_string(), surface.clone());
        surface
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
                    for ext in ["png", "svg", "xpm"] {
                        let path = dir.join("apps").join(format!("{name}.{ext}"));
                        if path.exists() {
                            return Some(path);
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
    text.lines()
        .find_map(|l| l.trim().strip_prefix("gtk-icon-theme-name=").map(|v| v.trim().to_string()))
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
fn load(path: &Path) -> Option<cairo::ImageSurface> {
    let png = if path.extension().and_then(|e| e.to_str()) == Some("png") {
        path.to_path_buf()
    } else {
        rasterise(path)?
    };
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
    let out = dir.join(format!("{stem}-{RASTER}.png"));
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
    options.fontdb_mut().load_system_fonts();
    let Ok(tree) = resvg::usvg::Tree::from_data(&data, &options) else { return false };
    let size = tree.size();
    let scale = RASTER as f32 / size.width().max(size.height()).max(1.0);
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1)) else { return false };
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.save_png(out).is_ok()
}
