//! The clipboard, through the desktop's tools: `wl-copy` puts text on it,
//! and `cliphist` (fed by `wl-paste --watch cliphist store` in the sway
//! config) keeps its history, which the launcher shows under `clip:` /
//! `буфер:`. Every process here runs off the UI thread; `wl-copy` stays
//! behind to serve the selection, so it runs in its own systemd scope and
//! outlives a restart of the island.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

/// Put `text` on the clipboard.
pub fn copy(text: &str) {
    let text = text.to_string();
    std::thread::spawn(move || {
        let mut command = Command::new("wl-copy");
        command.arg("--").arg(&text);
        match crate::launch::spawn(&command) {
            Ok(mut child) => { let _ = child.wait(); }
            Err(e) => eprintln!("island-shell: wl-copy: {e}"),
        }
    });
}

/// Put history entry `id` back on the clipboard (`cliphist decode | wl-copy`,
/// so images keep their type).
pub fn restore(id: u64) {
    std::thread::spawn(move || {
        // the id is a number, the script fixed: nothing reaches the shell
        // but its own arguments
        let mut command = Command::new("sh");
        command.args(["-c", "cliphist decode \"$1\" | wl-copy", "sh", &id.to_string()]);
        match crate::launch::spawn(&command) {
            Ok(mut child) => { let _ = child.wait(); }
            Err(e) => eprintln!("island-shell: cliphist: {e}"),
        }
    });
}

/// One entry of the history.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: u64,
    /// the text (one line, trimmed), or a description of binary data
    pub preview: String,
    /// the image's format ("png") for images
    pub image: Option<String>,
    /// a small PNG of an image entry, made by the search worker
    pub thumb: Option<String>,
}

/// What `clip:` finds.
#[derive(Clone, Debug, PartialEq)]
pub enum History {
    Entries(Vec<Clip>),
    /// cliphist is not installed or not fed
    Missing,
}

/// Read the history (newest first), filtered by `query`. Blocks on
/// `cliphist list`: call off the UI thread.
pub fn history(query: &str, limit: usize) -> History {
    let Ok(out) = Command::new("cliphist").arg("list").stdin(Stdio::null()).stderr(Stdio::null()).output() else {
        return History::Missing;
    };
    if !out.status.success() {
        return History::Missing;
    }
    let query = query.to_lowercase();
    let entries = String::from_utf8_lossy(&out.stdout).lines().filter_map(parse_line)
        .filter(|clip| query.is_empty() || clip.preview.to_lowercase().contains(&query))
        .take(limit).collect();
    History::Entries(entries)
}

/// "12\tsome text" or "13\t[[ binary data 52 KiB png 800x600 ]]".
fn parse_line(line: &str) -> Option<Clip> {
    let (id, rest) = line.split_once('\t')?;
    let id = id.trim().parse().ok()?;
    let rest = rest.trim();
    if let Some(binary) = rest.strip_prefix("[[ binary data ").and_then(|b| b.strip_suffix(" ]]")) {
        // "52 KiB png 800x600"
        let words: Vec<&str> = binary.split_whitespace().collect();
        let (size, kind, dims) = match words.as_slice() {
            [n, unit, kind, dims, ..] => (format!("{n} {unit}"), kind.to_string(), dims.to_string()),
            [n, unit, kind] => (format!("{n} {unit}"), kind.to_string(), String::new()),
            _ => (binary.to_string(), String::new(), String::new()),
        };
        let image = ["png", "jpeg", "jpg", "gif", "webp", "bmp"].contains(&kind.as_str());
        let preview = if image {
            format!("Изображение {} · {size}", dims.replace('x', "×"))
        } else {
            format!("Данные {kind} · {size}")
        };
        return Some(Clip { id, preview: preview.replace("  ", " "), image: image.then_some(kind), thumb: None });
    }
    let preview: String = rest.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect();
    (!preview.is_empty()).then_some(Clip { id, preview, image: None, thumb: None })
}

/// A small PNG of image entry `id` under `$XDG_RUNTIME_DIR` (made once;
/// off the UI thread).
fn thumbnail(id: u64) -> Option<String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?.join("island-shell").join("clips");
    let path = dir.join(format!("{id}.png"));
    if !path.exists() {
        let out = Command::new("cliphist").arg("decode").arg(id.to_string())
            .stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        if !out.status.success() { return None; }
        let image = image::load_from_memory(&out.stdout).ok()?.thumbnail(128, 128);
        std::fs::create_dir_all(&dir).ok()?;
        image.save_with_format(&path, image::ImageFormat::Png).ok()?;
    }
    Some(path.to_string_lossy().into_owned())
}

/// `clip:` / `буфер:` asks for the history; the rest filters it.
pub fn query(text: &str) -> Option<&str> {
    let text = text.trim_start();
    ["clip:", "буфер:", "clip ", "буфер "].iter().find_map(|prefix| {
        let head = text.get(..prefix.len())?;
        (head.to_lowercase() == *prefix).then(|| text[prefix.len()..].trim())
    })
}

/// The history read on a worker thread: the newest query wins.
pub struct Search {
    generation: Arc<AtomicU64>,
    worker: crate::background::Worker<(u64, String), History>,
}

impl Search {
    pub fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let worker = crate::background::Worker::new("clipboard", move |(asked, query): (u64, String)| {
            if current.load(Ordering::Relaxed) != asked { return History::Entries(Vec::new()); }
            let mut history = history(&query, 60);
            if let History::Entries(entries) = &mut history {
                for clip in entries.iter_mut().filter(|c| c.image.is_some()).take(12) {
                    clip.thumb = thumbnail(clip.id);
                }
            }
            history
        });
        Self { generation, worker }
    }
    pub fn set_waker(&self, tx: smithay_client_toolkit::reexports::calloop::channel::Sender<()>) {
        self.worker.set_waker(tx);
    }
    pub fn request(&self, query: String) -> Option<u64> {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.worker.request((generation, query)).then_some(generation)
    }
    pub fn drain(&self) -> Vec<(u64, History)> {
        self.worker.drain().into_iter().map(|((generation, _), history)| (generation, history)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_lines_parse() {
        assert_eq!(parse_line("12\t  hello   world "), Some(Clip { id: 12, preview: "hello world".into(), image: None, thumb: None }));
        assert_eq!(parse_line("13\t[[ binary data 52 KiB png 800x600 ]]"),
            Some(Clip { id: 13, preview: "Изображение 800×600 · 52 KiB".into(), image: Some("png".into()), thumb: None }));
        assert_eq!(parse_line("14\t[[ binary data 3 KiB octet-stream ]]").map(|c| c.image), Some(None));
        assert_eq!(parse_line("garbage"), None);
        assert_eq!(query("clip: ssh"), Some("ssh"));
        assert_eq!(query("Буфер:"), Some(""));
        assert_eq!(query("clipboard"), None);
    }
}
