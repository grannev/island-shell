//! The quick menu's controls over the environment: volume, brightness, the
//! audio output, the keyboard layout and the session. Everything runs on
//! one worker thread, so the UI never waits for a process; slider drags
//! are coalesced to the latest value before anything is written.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use smithay_client_toolkit::reexports::calloop::channel::Sender;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SinkKind {
    Speaker,
    Headphones,
    Display,
    Bluetooth,
    Other,
}

/// An audio output the menu can switch to.
#[derive(Clone, Debug, PartialEq)]
pub struct Sink {
    pub name: String,
    pub label: String,
    pub kind: SinkKind,
}

/// What the menu shows; `None` where a reading failed (the control greys out).
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub volume: Option<(f64, bool)>,
    pub brightness: Option<f64>,
    pub sinks: Vec<Sink>,
    pub default_sink: String,
    pub layout: Option<String>,
}

pub enum Request {
    Refresh,
    Volume(f64),
    Brightness(f64),
    Sink(String),
    NextLayout,
    Lock,
    Suspend,
    Logout,
}

/// Start the worker; snapshots come back through `tx`.
pub fn spawn(tx: Sender<Snapshot>) -> mpsc::Sender<Request> {
    let (requests, rx) = mpsc::channel::<Request>();
    thread::Builder::new()
        .name("controls".into())
        .spawn(move || {
            let mut worker = Worker { sway: None, muted: false };
            while let Ok(first) = rx.recv() {
                // a drag sends many values; only the latest of each matters
                let mut volume = None;
                let mut brightness = None;
                let mut refresh = false;
                let mut rest = Vec::new();
                for request in std::iter::once(first).chain(rx.try_iter()) {
                    match request {
                        Request::Volume(v) => volume = Some(v),
                        Request::Brightness(v) => brightness = Some(v),
                        Request::Refresh => refresh = true,
                        other => rest.push(other),
                    }
                }
                if let Some(v) = volume {
                    worker.set_volume(v);
                }
                if let Some(v) = brightness {
                    set_brightness(v);
                }
                for request in rest {
                    refresh |= worker.run(request);
                }
                if refresh {
                    let snapshot = worker.snapshot();
                    worker.muted = snapshot.volume.is_some_and(|(_, muted)| muted);
                    if tx.send(snapshot).is_err() {
                        break;
                    }
                }
            }
        })
        .expect("spawn controls worker");
    requests
}

struct Worker {
    sway: Option<crate::sway::Query>,
    /// the last read mute state: moving the slider unmutes, like any
    /// volume control
    muted: bool,
}

impl Worker {
    fn sway(&mut self) -> Option<&mut crate::sway::Query> {
        if self.sway.is_none() {
            self.sway = crate::sway::Query::connect().ok();
        }
        self.sway.as_mut()
    }

    fn set_volume(&mut self, value: f64) {
        let value = format!("{:.3}", value.clamp(0.0, 1.0));
        run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &value]);
        if self.muted {
            run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
            self.muted = false;
        }
    }

    /// Returns whether the menu should be refreshed afterwards.
    fn run(&mut self, request: Request) -> bool {
        match request {
            Request::Sink(name) => {
                run("pactl", &["set-default-sink", &name]);
                true
            }
            Request::NextLayout => {
                if let Some(sway) = self.sway() {
                    if sway.command("input type:keyboard xkb_switch_layout next").is_err() {
                        self.sway = None;
                    }
                }
                true
            }
            Request::Lock => {
                run("loginctl", &["lock-session"]);
                false
            }
            Request::Suspend => {
                run("systemctl", &["suspend"]);
                false
            }
            Request::Logout => {
                if let Some(sway) = self.sway() {
                    let _ = sway.command("exit");
                }
                false
            }
            Request::Refresh | Request::Volume(_) | Request::Brightness(_) => true,
        }
    }

    fn snapshot(&mut self) -> Snapshot {
        let layout = self.sway().and_then(|sway| sway.keyboard_layout().ok().flatten());
        Snapshot {
            volume: crate::activity::read_volume(),
            brightness: backlight().and_then(|path| crate::activity::read_backlight(&path)),
            sinks: sinks(),
            default_sink: output("pactl", &["get-default-sink"]).unwrap_or_default(),
            layout,
        }
    }
}

fn run(program: &str, args: &[&str]) {
    let status = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    if let Err(e) = status {
        eprintln!("island-shell: {program}: {e}");
    }
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn backlight() -> Option<PathBuf> {
    std::fs::read_dir("/sys/class/backlight").ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir())
}

/// Write the panel's brightness directly (the video group may), never
/// all the way to black.
fn set_brightness(value: f64) {
    let Some(path) = backlight() else { return };
    let Some(max) = std::fs::read_to_string(path.join("max_brightness")).ok().and_then(|s| s.trim().parse::<f64>().ok()) else {
        return;
    };
    let raw = (value.clamp(0.01, 1.0) * max).round().max(1.0) as u64;
    if let Err(e) = std::fs::write(path.join("brightness"), raw.to_string()) {
        eprintln!("island-shell: brightness: {e}");
    }
}

/// The outputs worth offering: sinks with a port that is not known to be
/// unplugged, named the way a person would.
fn sinks() -> Vec<Sink> {
    let Some(json) = output("pactl", &["-f", "json", "list", "sinks"]) else { return Vec::new() };
    parse_sinks(&json)
}

pub fn parse_sinks(json: &str) -> Vec<Sink> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    list.iter()
        .filter_map(|sink| {
            let name = sink["name"].as_str()?.to_string();
            let description = sink["description"].as_str().unwrap_or(&name).to_string();
            let ports = sink["ports"].as_array().cloned().unwrap_or_default();
            let unplugged = !ports.is_empty()
                && ports.iter().all(|p| p["availability"].as_str() == Some("not available"));
            if unplugged {
                return None;
            }
            let port = sink["active_port"].as_str().unwrap_or("").to_lowercase();
            let (kind, label) = if name.starts_with("bluez_output") {
                (SinkKind::Bluetooth, description)
            } else if port.contains("headphone") || port.contains("headset") {
                (SinkKind::Headphones, "Наушники".to_string())
            } else if port.contains("speaker") {
                (SinkKind::Speaker, "Динамики".to_string())
            } else if port.contains("hdmi") || port.contains("displayport") {
                let n: String = port.chars().filter(|c| c.is_ascii_digit()).collect();
                (SinkKind::Display, format!("HDMI / DisplayPort {n}").trim_end().to_string())
            } else {
                (SinkKind::Other, description)
            };
            Some(Sink { name, label, kind })
        })
        .collect()
}

/// A layout name as the menu says it.
pub fn layout_label(name: &str) -> String {
    match name {
        "English (US)" | "English (UK)" | "English" => "English".into(),
        "Russian" => "Русская".into(),
        "Ukrainian" => "Українська".into(),
        "German" => "Deutsch".into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talks to the real session: writes back the current volume and
    /// brightness and switches the layout forward and back again.
    #[test]
    #[ignore = "touches the live session; ISLAND_CONTROL_TEST=1"]
    fn worker_round_trips_against_the_session() {
        if std::env::var_os("ISLAND_CONTROL_TEST").is_none() { return; }
        let mut worker = Worker { sway: None, muted: false };
        let before = worker.snapshot();
        eprintln!("{before:?}");
        let (volume, muted) = before.volume.expect("volume");
        worker.muted = muted;
        if !muted { worker.set_volume(volume); }
        set_brightness(before.brightness.expect("brightness"));
        let after = worker.snapshot();
        assert!((after.volume.unwrap().0 - volume).abs() < 0.011);
        assert!((after.brightness.unwrap() - before.brightness.unwrap()).abs() < 0.011);
        assert!(!after.sinks.is_empty() && after.sinks.iter().any(|s| s.name == after.default_sink));
        let layout = before.layout.clone().expect("layout");
        worker.run(Request::NextLayout);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let switched = worker.snapshot().layout.expect("layout");
        worker.run(Request::NextLayout);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_ne!(switched, layout);
        assert_eq!(worker.snapshot().layout, Some(layout));
    }

    #[test]
    fn sinks_skip_unplugged_ports_and_read_as_people_say() {
        let json = r#"[
            {"name":"alsa_output.hdmi3","description":"Controller HDMI 3","active_port":"[Out] HDMI3",
             "ports":[{"name":"[Out] HDMI3","availability":"not available"}]},
            {"name":"alsa_output.speaker","description":"Controller Speaker","active_port":"[Out] Speaker",
             "ports":[{"name":"[Out] Speaker","availability":"availability unknown"}]},
            {"name":"bluez_output.AA.1","description":"AirPods Pro","active_port":"headset-output",
             "ports":[{"name":"headset-output","availability":"available"}]}
        ]"#;
        let sinks = parse_sinks(json);
        assert_eq!(sinks.len(), 2);
        assert_eq!((sinks[0].kind, sinks[0].label.as_str()), (SinkKind::Speaker, "Динамики"));
        assert_eq!((sinks[1].kind, sinks[1].label.as_str()), (SinkKind::Bluetooth, "AirPods Pro"));
        assert_eq!(layout_label("Russian"), "Русская");
    }
}
