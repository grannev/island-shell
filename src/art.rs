//! Album art: `mpris:artUrl` points at a file most of the time (Telegram
//! and browsers write a temporary JPEG or PNG), which is decoded once and
//! kept as a cairo surface at the size the island draws it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

/// Decoded art, already square and scaled to the requested device pixels.
pub struct Art {
    cache: RefCell<HashMap<(String, i32), Option<cairo::ImageSurface>>>,
}

impl Art {
    pub fn new() -> Art {
        Art { cache: RefCell::new(HashMap::new()) }
    }

    /// The art behind `url` at `px` × `px` device pixels, cropped square
    /// from the middle. `None` for a URL we cannot read (a remote one, or
    /// a file that has gone).
    pub fn get(&self, url: &str, px: i32) -> Option<cairo::ImageSurface> {
        let key = (url.to_string(), px);
        if let Some(hit) = self.cache.borrow().get(&key) {
            return hit.clone();
        }
        let surface = local_path(url).and_then(|path| load(&path, px));
        self.cache.borrow_mut().insert(key, surface.clone());
        surface
    }

    /// Forget everything (a new track's art usually lands on the same
    /// temporary path, so the cache must not answer for the old one).
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
    }
}

fn local_path(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("file://")?;
    // percent-decoding: art paths carry spaces and non-ASCII often enough
    let mut out = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let path = PathBuf::from(String::from_utf8(out).ok()?);
    path.exists().then_some(path)
}

fn load(path: &std::path::Path, px: i32) -> Option<cairo::ImageSurface> {
    let image = image::open(path).ok()?;
    let (w, h) = (image.width(), image.height());
    let side = w.min(h);
    let image = image
        .crop_imm((w - side) / 2, (h - side) / 2, side, side)
        .resize_exact(px as u32, px as u32, image::imageops::FilterType::CatmullRom)
        .to_rgba8();
    // cairo wants premultiplied BGRA
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).ok()?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().ok()?;
        for (y, row) in image.rows().enumerate() {
            for (x, pixel) in row.enumerate() {
                let [r, g, b, a] = pixel.0;
                let (r, g, b) = (mul(r, a), mul(g, a), mul(b, a));
                let o = y * stride + x * 4;
                data[o] = b;
                data[o + 1] = g;
                data[o + 2] = r;
                data[o + 3] = a;
            }
        }
    }
    surface.mark_dirty();
    Some(surface)
}

fn mul(c: u8, a: u8) -> u8 {
    ((c as u16 * a as u16 + 127) / 255) as u8
}
