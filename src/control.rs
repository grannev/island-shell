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

/// An application playing sound: all its streams under one slider.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    /// an icon-theme name (the stream's icon, else its binary)
    pub icon: String,
    /// the sink-input indices it owns
    pub streams: Vec<u32>,
    pub volume: f64,
    pub muted: bool,
}

/// What the menu shows; `None` where a reading failed (the control greys out).
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub volume: Option<(f64, bool)>,
    pub brightness: Option<f64>,
    pub sinks: Vec<Sink>,
    pub default_sink: String,
    pub layout: Option<String>,
    pub apps: Vec<App>,
    /// the default microphone's level and mute, and what it is called
    pub mic: Option<(f64, bool)>,
    pub mic_label: Option<String>,
    /// gammastep is tinting the screen
    pub night: bool,
}

pub enum Request {
    Refresh,
    Volume(f64),
    Brightness(f64),
    /// one application's streams
    AppVolume(Vec<u32>, f64),
    Mic(f64),
    MicMute,
    /// the speakers' mute
    ToggleMute,
    Sink(String),
    NextLayout,
    /// night mode on (at this colour temperature) or off
    Night(Option<u32>),
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
            let mut worker = Worker { compositor: None, muted: false };
            while let Ok(first) = rx.recv() {
                // a drag sends many values; only the latest of each matters
                let mut volume = None;
                let mut brightness = None;
                let mut mic = None;
                let mut apps: Vec<(Vec<u32>, f64)> = Vec::new();
                let mut refresh = false;
                let mut rest = Vec::new();
                for request in std::iter::once(first).chain(rx.try_iter()) {
                    match request {
                        Request::Volume(v) => volume = Some(v),
                        Request::Brightness(v) => brightness = Some(v),
                        Request::Mic(v) => mic = Some(v),
                        Request::AppVolume(streams, v) => match apps.iter_mut().find(|(s, _)| *s == streams) {
                            Some(slot) => slot.1 = v,
                            None => apps.push((streams, v)),
                        },
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
                if let Some(v) = mic {
                    let value = format!("{:.3}", v.clamp(0.0, 1.0));
                    run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SOURCE@", &value]);
                }
                for (streams, v) in apps {
                    let percent = format!("{}%", (v.clamp(0.0, 1.0) * 100.0).round());
                    for stream in streams {
                        run("pactl", &["set-sink-input-volume", &stream.to_string(), &percent]);
                    }
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
    compositor: Option<crate::compositor::Query>,
    /// the last read mute state: moving the slider unmutes, like any
    /// volume control
    muted: bool,
}

impl Worker {
    fn compositor(&mut self) -> Option<&mut crate::compositor::Query> {
        if self.compositor.is_none() {
            self.compositor = crate::compositor::Query::connect().ok();
        }
        self.compositor.as_mut()
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
                if let Some(compositor) = self.compositor() {
                    if compositor.next_layout().is_err() {
                        self.compositor = None;
                    }
                }
                true
            }
            Request::MicMute => {
                run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]);
                true
            }
            Request::ToggleMute => {
                run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
                true
            }
            Request::Night(kelvin) => {
                // the same gammastep as the mod+n script: a warm tint through
                // wlr-gamma-control while it runs; killing it restores the
                // screen. It lives in its own scope, past an island restart.
                run("pkill", &["-x", "gammastep"]);
                if let Some(kelvin) = kelvin {
                    let mut command = Command::new("gammastep");
                    command.args(["-O", &kelvin.to_string(), "-m", "wayland"]);
                    match crate::launch::spawn(&command) {
                        Ok(mut child) => { thread::spawn(move || { let _ = child.wait(); }); }
                        Err(e) => eprintln!("island-shell: gammastep: {e}"),
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
                if let Some(compositor) = self.compositor() {
                    let _ = compositor.exit();
                }
                false
            }
            Request::Refresh | Request::Volume(_) | Request::Brightness(_) | Request::Mic(_) | Request::AppVolume(..) => true,
        }
    }

    fn snapshot(&mut self) -> Snapshot {
        let layout = self.compositor().and_then(|c| c.keyboard_layout().ok().flatten());
        Snapshot {
            volume: crate::activity::read_volume(),
            brightness: backlight().and_then(|path| crate::activity::read_backlight(&path)),
            sinks: sinks(),
            default_sink: output("pactl", &["get-default-sink"]).unwrap_or_default(),
            layout,
            apps: output("pactl", &["-f", "json", "list", "sink-inputs"]).map(|json| parse_apps(&json)).unwrap_or_default(),
            mic: read_level("@DEFAULT_AUDIO_SOURCE@"),
            night: Command::new("pgrep").args(["-x", "gammastep"]).stdout(Stdio::null()).stderr(Stdio::null())
                .status().is_ok_and(|s| s.success()),
            mic_label: output("pactl", &["get-default-source"]).and_then(|name| {
                let json = output("pactl", &["-f", "json", "list", "sources"])?;
                source_label(&json, &name)
            }),
        }
    }
}

fn read_level(target: &str) -> Option<(f64, bool)> {
    let text = output("wpctl", &["get-volume", target])?;
    let value = text.split_whitespace().find_map(|word| word.parse::<f64>().ok())?;
    Some((value.clamp(0.0, 1.0), text.contains("MUTED")))
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

/// The applications playing now, one entry per application however many
/// streams it opened (a browser often has several).
pub fn parse_apps(json: &str) -> Vec<App> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut apps: Vec<App> = Vec::new();
    for stream in &list {
        let Some(index) = stream["index"].as_u64() else { continue };
        let props = &stream["properties"];
        let prop = |key: &str| props[key].as_str().filter(|s| !s.is_empty() && *s != "(null)").map(str::to_string);
        // event sounds (notification chimes) are not something to mix
        if prop("media.role").as_deref() == Some("event") {
            continue;
        }
        let binary = prop("application.process.binary");
        let name = prop("application.name").or_else(|| binary.clone()).unwrap_or_else(|| "Приложение".into());
        let icon = prop("application.icon_name").or(binary).unwrap_or_default();
        let levels: Vec<f64> = stream["volume"].as_object().map(|channels| {
            channels.values().filter_map(|c| c["value_percent"].as_str()?.trim_end_matches('%').parse::<f64>().ok()).collect()
        }).unwrap_or_default();
        let volume = (levels.iter().copied().fold(0.0, f64::max) / 100.0).clamp(0.0, 1.0);
        let muted = stream["mute"].as_bool().unwrap_or(false);
        match apps.iter_mut().find(|a| a.name == name) {
            Some(app) => {
                app.streams.push(index as u32);
                app.volume = app.volume.max(volume);
            }
            None => apps.push(App { name, icon, streams: vec![index as u32], volume, muted }),
        }
    }
    apps
}

/// The default microphone, named as a person would.
fn source_label(json: &str, name: &str) -> Option<String> {
    let serde_json::Value::Array(list) = serde_json::from_str::<serde_json::Value>(json).ok()? else { return None };
    let source = list.iter().find(|s| s["name"].as_str() == Some(name))?;
    if name.starts_with("alsa_input") {
        return Some("Встроенный микрофон".into());
    }
    source["description"].as_str().map(str::to_string)
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
        let mut worker = Worker { compositor: None, muted: false };
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
        // applications and the microphone: write back what they are at
        if let Some((mic, _)) = before.mic {
            let value = format!("{mic:.3}");
            run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SOURCE@", &value]);
            assert!((worker.snapshot().mic.unwrap().0 - mic).abs() < 0.011);
        }
        for app in &before.apps {
            let percent = format!("{}%", (app.volume * 100.0).round());
            for stream in &app.streams {
                run("pactl", &["set-sink-input-volume", &stream.to_string(), &percent]);
            }
        }
        let again = worker.snapshot();
        for (a, b) in before.apps.iter().zip(&again.apps) {
            assert_eq!(a.name, b.name);
            assert!((a.volume - b.volume).abs() < 0.011, "{} {} → {}", a.name, a.volume, b.volume);
        }
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

    #[test]
    fn streams_of_one_application_share_a_slider() {
        let json = r#"[
            {"index":1506,"mute":false,"volume":{"front-left":{"value_percent":"100%"},"front-right":{"value_percent":"100%"}},
             "properties":{"application.name":"Firefox","application.process.binary":"firefox-esr"}},
            {"index":1534,"mute":false,"volume":{"front-left":{"value_percent":"40%"}},
             "properties":{"application.name":"Firefox","application.process.binary":"firefox-esr"}},
            {"index":1600,"mute":true,"volume":{"mono":{"value_percent":"55%"}},
             "properties":{"application.name":"Telegram","application.icon_name":"telegram"}},
            {"index":1700,"mute":false,"volume":{"mono":{"value_percent":"80%"}},
             "properties":{"application.name":"sound","media.role":"event"}}
        ]"#;
        let apps = parse_apps(json);
        assert_eq!(apps.len(), 2);
        assert_eq!((apps[0].name.as_str(), apps[0].icon.as_str(), apps[0].streams.clone(), apps[0].volume),
            ("Firefox", "firefox-esr", vec![1506, 1534], 1.0));
        assert_eq!((apps[1].icon.as_str(), apps[1].muted, apps[1].volume), ("telegram", true, 0.55));
    }
}
