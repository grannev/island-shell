//! Album art: `mpris:artUrl` points at a file most of the time (Telegram
//! and browsers write a temporary JPEG or PNG), which is decoded once and
//! kept as a cairo surface at the size the island draws it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Decoded art, already square and scaled to the requested device pixels.
pub struct Art {
    cache: RefCell<HashMap<(String, i32), Option<cairo::ImageSurface>>>,
    pending: RefCell<HashSet<(String, i32)>>,
    generation: Cell<u64>,
    worker: crate::background::Worker<(u64, String, i32), Option<crate::background::Pixels>>,
}

impl Art {
    pub fn new() -> Art {
        Art {
            cache: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashSet::new()),
            generation: Cell::new(0),
            worker: crate::background::Worker::new("image-decode", |(_, url, px): (u64, String, i32)| {
                local_path(&url).and_then(|path| load(&path, px)).and_then(crate::background::Pixels::from_surface)
            }),
        }
    }

    pub fn set_waker(&self, tx: smithay_client_toolkit::reexports::calloop::channel::Sender<()>) {
        self.worker.set_waker(tx);
    }

    pub fn drain(&self) -> bool {
        let mut changed = false;
        for ((generation, url, px), pixels) in self.worker.drain() {
            if generation != self.generation.get() {
                continue;
            }
            let key = (url, px);
            self.pending.borrow_mut().remove(&key);
            let mut cache = self.cache.borrow_mut();
            if cache.len() >= 32 {
                if let Some(old) = cache.keys().next().cloned() {
                    cache.remove(&old);
                }
            }
            cache.insert(key, pixels.and_then(crate::background::Pixels::into_surface));
            changed = true;
        }
        changed
    }

    /// The art behind `url` at `px` × `px` device pixels, cropped square
    /// from the middle. `None` for a URL we cannot read (a remote one, or
    /// a file that has gone).
    pub fn get(&self, url: &str, px: i32) -> Option<cairo::ImageSurface> {
        let key = (url.to_string(), px.clamp(1, 1024));
        if let Some(hit) = self.cache.borrow().get(&key) {
            return hit.clone();
        }
        if !self.pending.borrow().contains(&key) && self.worker.request((self.generation.get(), key.0.clone(), key.1)) {
            self.pending.borrow_mut().insert(key);
        }
        None
    }

    /// Forget everything (a new track's art usually lands on the same
    /// temporary path, so the cache must not answer for the old one).
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
        self.pending.borrow_mut().clear();
        self.generation.set(self.generation.get().wrapping_add(1));
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clearing_art_rejects_late_results_and_coalesces_requests() {
        let art = Art::new();
        for _ in 0..20 {
            assert!(art.get("file:///missing-old-art.png", 32).is_none());
        }
        assert_eq!(art.pending.borrow().len(), 1);
        art.clear();
        art.get("file:///missing-current-art.png", 32);
        let start = std::time::Instant::now();
        while !art.pending.borrow().is_empty() && start.elapsed().as_secs() < 2 {
            art.drain();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(art.pending.borrow().is_empty());
        let cache = art.cache.borrow();
        assert_eq!(cache.len(), 1);
        assert!(cache.contains_key(&("file:///missing-current-art.png".into(), 32)));
    }
}
