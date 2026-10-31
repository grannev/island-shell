//! Downloads into ~/Downloads, watched with inotify on a thread of its own
//! (it sleeps in `read` while nothing downloads). Browsers write a partial
//! file beside the one they mean — Firefox `name.part`, Chromium
//! `name.crdownload` — and rename it when the transfer ends; a partial that
//! goes away without its final file was cancelled.
//!
//! While partial files exist their sizes are read twice a second for the
//! byte count and the speed. Neither browser says how big the file will
//! be, so a download is an indeterminate ring with "12,4 МБ · 2,1 МБ/с".
//! A partial left behind by an earlier session is only reported once it
//! grows.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use smithay_client_toolkit::reexports::calloop::channel::Sender;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Started { key: String, name: String },
    Progress { key: String, bytes: u64, speed: f64 },
    Finished { key: String, path: PathBuf },
    Cancelled { key: String },
}

const SUFFIXES: [&str; 2] = [".part", ".crdownload"];

pub fn dir() -> PathBuf {
    std::env::var_os("XDG_DOWNLOAD_DIR").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads")
    })
}

/// The file a partial becomes: its name without the browser's suffix.
pub fn final_of(partial: &Path) -> Option<PathBuf> {
    let name = partial.file_name()?.to_str()?;
    let stem = SUFFIXES.iter().find_map(|s| name.strip_suffix(s))?;
    Some(partial.with_file_name(stem))
}

/// A name for the user: Chromium's "Unconfirmed 123" is no name yet.
fn display_name(partial: &Path) -> String {
    let name = final_of(partial).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    if name.is_empty() || name.starts_with("Unconfirmed ") { "Загрузка".into() } else { name }
}

/// "12,4 МБ" in Russian units, with a decimal comma.
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    let (value, unit) = if b >= 1e9 { (b / 1e9, "ГБ") } else if b >= 1e6 { (b / 1e6, "МБ") } else if b >= 1e3 { (b / 1e3, "КБ") } else { (b, "Б") };
    let text = if value >= 100.0 || unit == "Б" { format!("{value:.0}") } else { format!("{value:.1}") };
    format!("{} {unit}", text.replace('.', ","))
}

struct Partial {
    key: String,
    first: u64,
    bytes: u64,
    at: Instant,
    speed: f64,
    reported: bool,
}

fn partials(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().filter_map(|e| {
        let path = e.path();
        let name = path.file_name()?.to_str()?;
        if !SUFFIXES.iter().any(|s| name.ends_with(s)) { return None; }
        let len = e.metadata().ok().filter(|m| m.is_file())?.len();
        Some((path, len))
    }).collect()
}

/// One look at the folder: what started, grew, ended.
fn scan(dir: &Path, known: &mut HashMap<PathBuf, Partial>, next_key: &mut u64, out: &mut Vec<Event>) {
    let now = Instant::now();
    let found = partials(dir);
    let gone: Vec<PathBuf> = known.keys().filter(|p| !found.iter().any(|(f, _)| f == *p)).cloned().collect();
    let fresh: Vec<&(PathBuf, u64)> = found.iter().filter(|(p, _)| !known.contains_key(p)).collect();
    for path in gone {
        let partial = known.remove(&path).unwrap();
        // Chromium renames "Unconfirmed 123.crdownload" to the real name
        // mid-way: the same download under a new partial
        if let Some((new_path, len)) = fresh.iter().find(|(p, _)| !known.contains_key(p) && path.extension() == p.extension()
            && path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Unconfirmed "))) {
            if partial.reported {
                out.push(Event::Started { key: partial.key.clone(), name: display_name(new_path) });
            }
            known.insert(new_path.clone(), Partial { bytes: *len, ..partial });
            continue;
        }
        if !partial.reported { continue; }
        match final_of(&path).filter(|f| std::fs::metadata(f).is_ok_and(|m| m.len() > 0)) {
            Some(done) => out.push(Event::Finished { key: partial.key, path: done }),
            None => out.push(Event::Cancelled { key: partial.key }),
        }
    }
    for (path, len) in found {
        match known.get_mut(&path) {
            None => {
                *next_key += 1;
                known.insert(path, Partial { key: format!("download:{next_key}"), first: len, bytes: len, at: now, speed: 0.0, reported: false });
            }
            Some(p) => {
                let dt = now.duration_since(p.at).as_secs_f64();
                if dt >= 0.25 {
                    let rate = len.saturating_sub(p.bytes) as f64 / dt;
                    p.speed = if p.speed == 0.0 { rate } else { 0.6 * p.speed + 0.4 * rate };
                    p.at = now;
                }
                let grew = len != p.bytes;
                p.bytes = len;
                if !p.reported && len > p.first {
                    p.reported = true;
                    out.push(Event::Started { key: p.key.clone(), name: display_name(&path) });
                }
                if p.reported && (grew || dt >= 0.25) {
                    out.push(Event::Progress { key: p.key.clone(), bytes: len, speed: p.speed });
                }
            }
        }
    }
}

/// Watch the downloads folder; events go to `tx`. Without the folder (or
/// inotify) there is simply no watcher.
pub fn spawn(tx: Sender<Event>) {
    let dir = dir();
    let Ok(cdir) = CString::new(dir.as_os_str().as_bytes()) else { return };
    let _ = std::thread::Builder::new().name("downloads".into()).spawn(move || {
        // SAFETY: plain inotify calls on a descriptor this thread owns.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 { return; }
        let mask = libc::IN_CREATE | libc::IN_MOVED_TO | libc::IN_MOVED_FROM | libc::IN_DELETE | libc::IN_CLOSE_WRITE;
        if unsafe { libc::inotify_add_watch(fd, cdir.as_ptr(), mask) } < 0 {
            unsafe { libc::close(fd) };
            return;
        }
        let mut known = HashMap::new();
        let mut next_key = 0u64;
        let mut buf = [0u8; 4096];
        scan(&dir, &mut known, &mut next_key, &mut Vec::new());
        loop {
            // asleep until the folder changes; twice a second while
            // something downloads
            let timeout = if known.is_empty() { -1 } else { 500 };
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let ready = unsafe { libc::poll(&mut pfd, 1, timeout) };
            if ready < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted { continue; }
                break;
            }
            if ready > 0 {
                let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 { break; }
            }
            let mut events = Vec::new();
            scan(&dir, &mut known, &mut next_key, &mut events);
            for event in events {
                if tx.send(event).is_err() {
                    unsafe { libc::close(fd) };
                    return;
                }
            }
        }
        unsafe { libc::close(fd) };
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_that_grows_then_becomes_its_file_is_a_download() {
        let dir = std::env::temp_dir().join(format!("island-downloads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut known = HashMap::new();
        let mut key = 0;
        let mut out = Vec::new();
        // a stale partial from before: never reported until it grows
        std::fs::write(dir.join("old.iso.part"), b"x").unwrap();
        scan(&dir, &mut known, &mut key, &mut out);
        assert!(out.is_empty());
        std::fs::write(dir.join("film.mkv"), b"").unwrap();
        std::fs::write(dir.join("film.mkv.part"), b"abc").unwrap();
        scan(&dir, &mut known, &mut key, &mut out);
        std::fs::write(dir.join("film.mkv.part"), b"abcdef").unwrap();
        scan(&dir, &mut known, &mut key, &mut out);
        assert!(matches!(&out[0], Event::Started { name, .. } if name == "film.mkv"), "{out:?}");
        out.clear();
        std::fs::rename(dir.join("film.mkv.part"), dir.join("film.mkv")).unwrap();
        // the stale one is deleted unreported: no word about it
        std::fs::remove_file(dir.join("old.iso.part")).unwrap();
        scan(&dir, &mut known, &mut key, &mut out);
        assert_eq!(out, vec![Event::Finished { key: "download:2".into(), path: dir.join("film.mkv") }]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_read_in_russian_units() {
        assert_eq!(size_text(512), "512 Б");
        assert_eq!(size_text(12_400_000), "12,4 МБ");
        assert_eq!(size_text(1_500_000_000), "1,5 ГБ");
    }
}
