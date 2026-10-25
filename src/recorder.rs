//! Screen recording with `wf-recorder`: the island counts down 3-2-1, the
//! recording runs while a red dot and its time sit beside the clock, a
//! click stops it, and the finished video flies into the island as a
//! thumbnail (its first frame, by `ffmpeg`). Videos go to
//! `~/Videos/Screencasts`.
//!
//! The recorder is the island's own helper (not a user application): it is
//! stopped with SIGINT so the file is finished properly, and a restart of
//! the island ends it the same way. Screenshots from the Control Center
//! start here too.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use smithay_client_toolkit::reexports::calloop::channel::Sender;

/// What the recorder thread reports.
#[derive(Debug)]
pub enum Event {
    /// wf-recorder runs, as this process
    Started { pid: u32 },
    /// it ended: the file (when there is one) and a frame of it
    Finished { path: Option<PathBuf>, thumb: Option<PathBuf> },
}

/// Take a screenshot of the whole screen once the island's menu has folded
/// out of the picture: the person's own screenshot script when there is one
/// (it saves, copies and chimes), else the island's grim wrapper. Both hand
/// the file back to the island for the thumbnail. Runs in a user scope, off
/// the UI thread.
pub fn screenshot(after: Duration) {
    thread::spawn(move || {
        thread::sleep(after);
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let own = home.join(".config/sway/scripts/screenshot.sh");
        let program = if own.is_file() { own } else { home.join(".local/bin/island-screenshot") };
        let mut command = Command::new("sh");
        command.arg(&program);
        match crate::launch::spawn(&command) {
            Ok(mut child) => { let _ = child.wait(); }
            Err(e) => eprintln!("island-shell: screenshot: {e}"),
        }
    });
}

/// Is wf-recorder installed?
pub fn available() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("wf-recorder").is_file()))
}

fn directory() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let videos = std::env::var_os("XDG_VIDEOS_DIR").map(PathBuf::from).filter(|d| d != &home).unwrap_or_else(|| home.join("Videos"));
    Some(videos.join("Screencasts"))
}

/// Start recording `output` (a sway output name) on a thread; events come
/// back through `tx`.
pub fn start(output: Option<String>, tx: Sender<Event>) {
    std::thread::spawn(move || {
        let Some(dir) = directory() else {
            let _ = tx.send(Event::Finished { path: None, thumb: None });
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}.mp4", crate::island::strftime("%Y-%m-%d_%H-%M-%S")));
        let mut command = Command::new("wf-recorder");
        if let Some(output) = &output { command.args(["-o", output]); }
        command.arg("-f").arg(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                eprintln!("island-shell: wf-recorder: {e}");
                let _ = tx.send(Event::Finished { path: None, thumb: None });
                return;
            }
        };
        let _ = tx.send(Event::Started { pid: child.id() });
        // SIGINT is how we stop it: wf-recorder then finishes the file
        if let Ok(status) = child.wait() {
            if !status.success() && status.code().is_some() {
                eprintln!("island-shell: wf-recorder exited with {status}");
            }
        }
        let exists = path.metadata().is_ok_and(|m| m.len() > 0);
        let thumb = exists.then(|| thumbnail(&path)).flatten();
        let _ = tx.send(Event::Finished { path: exists.then_some(path), thumb });
    });
}

/// Stop the recording (SIGINT to wf-recorder, which finishes the file).
pub fn stop(pid: u32) {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    const SIGINT: i32 = 2;
    // SAFETY: plain syscall on a pid we spawned and still wait for
    unsafe { kill(pid as i32, SIGINT); }
}

/// The first frame as a PNG under `$XDG_RUNTIME_DIR` (on the recorder's
/// thread).
fn thumbnail(video: &Path) -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?.join("island-shell");
    std::fs::create_dir_all(&dir).ok()?;
    let thumb = dir.join(format!("{}.png", video.file_stem()?.to_string_lossy()));
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-ss", "0.3", "-i"]).arg(video)
        .args(["-frames:v", "1", "-vf", "scale=480:-2"]).arg(&thumb)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status().is_ok_and(|s| s.success());
    (ok && thumb.exists()).then_some(thumb)
}
