//! Live activities: short, native OSDs fed by the desktop environment and
//! the compact countdown that lives beside the clock.  Watchers only read
//! the environment and send events; all timing and drawing stays in the
//! island's event loop.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use cairo::Context;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::art::Art;
use crate::island::{Font, Renderer, smoothstep};

pub const PILL_H: f64 = 92.0;
pub const SLIDER_H: f64 = 82.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderKind {
    Volume,
    Brightness,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Slider { kind: SliderKind, value: f64, muted: bool },
    Charging { percent: u8 },
    BatteryLow { percent: u8 },
    /// the battery's state, for the bar (no pill of its own): percent,
    /// charging, minutes to empty (or full)
    Battery { percent: u8, charging: bool, remaining: Option<u32> },
    Bluetooth { name: String, battery: Option<u8> },
    Wifi { ssid: String },
    Screenshot { path: PathBuf },
    /// a countdown or a Pomodoro phase ran out: what ended, what follows
    TimerDone { title: String, detail: String },
    TimerMenu,
    WifiOpen,
    BluetoothOpen,
    /// the jobs from `island-shell task|progress|run`, unfolded
    Tasks,
    /// the month, from a right click on the clock
    CalendarOpen,
    /// the notes page (`src/notes.rs`)
    NotesOpen,
    /// the default microphone was muted or unmuted
    MicMute { muted: bool },
    /// the applications recording from a microphone now (state for the
    /// privacy dot, never a pill of its own)
    MicInUse { apps: Vec<String> },
    /// 3-2-1 before a screen recording starts
    RecordCountdown,
    /// the record button was pressed without wf-recorder installed
    RecorderMissing,
    /// night mode was switched (from the keyboard or a script)
    NightMode { on: bool },
    /// a screen recording was saved (its first frame, its length)
    Recording { path: PathBuf, thumb: Option<PathBuf>, seconds: u64 },
}

/// The countdown before a recording, in seconds.
pub const RECORD_COUNTDOWN: f64 = 3.0;

impl Event {
    /// A countdown ran out.
    pub fn timer_done() -> Event {
        Event::TimerDone { title: "Таймер".into(), detail: "Время вышло".into() }
    }

    pub fn size(&self) -> (f64, f64) {
        match self {
            Event::Slider { .. } => (340.0, SLIDER_H),
            // state for the bar, never a pill
            Event::Battery { .. } => (270.0, PILL_H),
            Event::Charging { .. } => (270.0, PILL_H),
            Event::BatteryLow { .. } => (310.0, PILL_H),
            Event::Bluetooth { .. } => (370.0, PILL_H),
            Event::Wifi { .. } => (390.0, PILL_H),
            Event::Screenshot { .. } => (390.0, 106.0),
            Event::TimerDone { .. } => (330.0, PILL_H),
            // sized by the island from the timer page's state
            Event::TimerMenu => (crate::timer_page::WIDTH, 158.0),
            // sized by the island from its jobs (tasks::Tasks::pill_size)
            Event::Tasks => (crate::tasks::PILL_W, PILL_H),
            // sized by the island from the month shown
            Event::CalendarOpen => (crate::calendar::WIDTH, 352.0),
            Event::NotesOpen => (crate::notes::WIDTH, crate::notes::HEIGHT),
            Event::MicMute { .. } => (300.0, PILL_H),
            Event::MicInUse { .. } => (0.0, 0.0),
            Event::RecordCountdown => (300.0, PILL_H),
            Event::RecorderMissing => (470.0, PILL_H),
            Event::NightMode { .. } => (300.0, PILL_H),
            Event::Recording { .. } => (400.0, 96.0),
            Event::WifiOpen | Event::BluetoothOpen => (crate::connections::WIDTH, crate::connections::HEIGHT),
        }
    }

    pub fn hold(&self) -> f64 {
        match self {
            Event::Slider { .. } => 1.45,
            Event::Battery { .. } => 0.0,
            Event::Charging { .. } => 2.8,
            Event::BatteryLow { .. } => 5.0,
            Event::Bluetooth { .. } | Event::Wifi { .. } => 3.2,
            Event::Screenshot { .. } => 4.0,
            Event::TimerDone { .. } => 4.0,
            Event::Tasks => 4.5,
            Event::CalendarOpen | Event::NotesOpen => f64::INFINITY,
            Event::MicMute { .. } => 1.8,
            Event::MicInUse { .. } => 0.0,
            Event::RecordCountdown => RECORD_COUNTDOWN,
            Event::RecorderMissing => 6.0,
            Event::NightMode { .. } => 1.8,
            Event::Recording { .. } => 4.5,
            Event::TimerMenu => f64::INFINITY,
            Event::WifiOpen | Event::BluetoothOpen => f64::INFINITY,
        }
    }

    pub fn is_low_battery(&self) -> bool {
        matches!(self, Event::BatteryLow { .. })
    }
}

pub fn takes_keyboard(event: &Event) -> bool {
    matches!(event, Event::TimerMenu | Event::WifiOpen | Event::BluetoothOpen | Event::CalendarOpen | Event::NotesOpen
        | Event::RecordCountdown)
}

/// Start the environment readers.  Missing optional tools simply disable
/// their watcher; the rest of the island remains fully functional.
pub fn spawn_watchers(tx: Sender<Event>) {
    spawn_audio(tx.clone());
    spawn_power(tx.clone());
    spawn_network(tx);
}

fn spawn_audio(tx: Sender<Event>) {
    thread::spawn(move || {
        let mut previous = read_volume();
        let mut mic_muted = read_mic_muted();
        let mut mic_users = read_mic_users();
        if !mic_users.is_empty() {
            let _ = tx.send(Event::MicInUse { apps: mic_users.clone() });
        }
        loop {
            let child = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
            let Ok(mut child) = child else {
                thread::sleep(Duration::from_secs(5));
                continue;
            };
            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    // a recording starting or stopping, a microphone muted
                    if line.contains(" source") {
                        let users = read_mic_users();
                        if users != mic_users {
                            mic_users = users;
                            let _ = tx.send(Event::MicInUse { apps: mic_users.clone() });
                        }
                        let muted = read_mic_muted();
                        if muted.is_some() && muted != mic_muted {
                            // only a change is news (not the state at start)
                            if mic_muted.is_some() {
                                let _ = tx.send(Event::MicMute { muted: muted.unwrap_or(false) });
                            }
                            mic_muted = muted;
                        }
                        continue;
                    }
                    if !line.contains("sink") && !line.contains("server") {
                        continue;
                    }
                    let Some(current) = read_volume() else { continue };
                    if previous != Some(current) {
                        previous = Some(current);
                        let _ = tx.send(Event::Slider { kind: SliderKind::Volume, value: current.0, muted: current.1 });
                    }
                }
            }
            let _ = child.wait();
            thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Is the default microphone muted?
fn read_mic_muted() -> Option<bool> {
    let out = Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).contains("MUTED"))
}

/// The applications recording from a microphone right now: live capture
/// streams, leaving out what listens to the speakers (cava, visualisers:
/// sink monitors) and level meters.
fn read_mic_users() -> Vec<String> {
    let Ok(out) = Command::new("pactl").args(["-f", "json", "list", "source-outputs"]).output() else { return Vec::new() };
    let Ok(streams) = serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout) else { return Vec::new() };
    let monitors: Vec<u64> = Command::new("pactl").args(["list", "short", "sources"]).output().ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.split('\t').nth(1).is_some_and(|n| n.ends_with(".monitor")))
            .filter_map(|l| l.split('\t').next()?.parse().ok()).collect())
        .unwrap_or_default();
    mic_users(&streams, &monitors)
}

fn mic_users(streams: &[serde_json::Value], monitors: &[u64]) -> Vec<String> {
    let mut apps: Vec<String> = streams.iter().filter_map(|stream| {
        let props = stream.get("properties")?;
        let prop = |key: &str| props.get(key).and_then(|v| v.as_str());
        if stream.get("corked").and_then(|v| v.as_bool()) == Some(true)
            || prop("stream.capture.sink") == Some("true")
            || prop("media.name").is_some_and(|n| n.eq_ignore_ascii_case("peak detect"))
            || stream.get("source").and_then(|v| v.as_u64()).is_some_and(|s| monitors.contains(&s)) {
            return None;
        }
        let name = prop("application.name").or(prop("application.process.binary")).or(prop("node.name"))?;
        Some(name.chars().take(40).collect())
    }).collect();
    apps.sort();
    apps.dedup();
    apps
}

pub fn read_volume() -> Option<(f64, bool)> {
    let out = Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SINK@"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let value = text.split_whitespace().find_map(|word| word.parse::<f64>().ok())?;
    Some((value.clamp(0.0, 1.0), text.contains("MUTED")))
}

fn spawn_power(tx: Sender<Event>) {
    thread::spawn(move || {
        let backlight = first_device("/sys/class/backlight");
        let battery = first_battery();
        let mut last_light = backlight.as_deref().and_then(read_backlight);
        let mut last_battery = battery.as_deref().and_then(read_battery);
        let mut power_tick = 0;
        if let Some((percent, false)) = last_battery.filter(|(percent, _)| *percent <= 20) {
            let _ = tx.send(Event::BatteryLow { percent });
        }
        // the bar's battery: sent when it changes (the minutes included)
        let mut last_state = None;
        let state = |path: &Path, tx: &Sender<Event>, last: &mut Option<(u8, bool, Option<u32>)>| {
            let Some((percent, charging)) = read_battery(path) else { return };
            let now = (percent, charging, read_remaining(path, charging));
            if *last != Some(now) {
                *last = Some(now);
                let _ = tx.send(Event::Battery { percent, charging, remaining: now.2 });
            }
        };
        if let Some(path) = battery.as_deref() {
            state(path, &tx, &mut last_state);
        }
        loop {
            thread::sleep(Duration::from_millis(80));
            if let Some(path) = backlight.as_deref() {
                if let Some(value) = read_backlight(path) {
                    if last_light.is_some_and(|old| (old - value).abs() >= 0.004) {
                        let _ = tx.send(Event::Slider { kind: SliderKind::Brightness, value, muted: false });
                    }
                    last_light = Some(value);
                }
            }
            // Keep key feedback responsive without reading the battery
            // at the backlight's rate.
            power_tick = (power_tick + 1) % 25;
            if power_tick != 0 {
                continue;
            }
            let Some(path) = battery.as_deref() else { continue };
            state(path, &tx, &mut last_state);
            let Some(current) = read_battery(path) else { continue };
            if let Some((old_percent, old_charging)) = last_battery {
                if current.1 && !old_charging {
                    let _ = tx.send(Event::Charging { percent: current.0 });
                }
                let crossed = [20, 10, 5].into_iter().any(|level| old_percent > level && current.0 <= level);
                if crossed && !current.1 {
                    let _ = tx.send(Event::BatteryLow { percent: current.0 });
                }
            }
            last_battery = Some(current);
        }
    });
}

fn first_device(base: &str) -> Option<PathBuf> {
    std::fs::read_dir(base).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir())
}

fn first_battery() -> Option<PathBuf> {
    std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|path| read_trim(&path.join("type")).as_deref() == Some("Battery"))
}

fn read_trim(path: &Path) -> Option<String> {
    Some(std::fs::read_to_string(path).ok()?.trim().to_string())
}

pub fn read_backlight(path: &Path) -> Option<f64> {
    let current = read_trim(&path.join("brightness"))?.parse::<f64>().ok()?;
    let max = read_trim(&path.join("max_brightness"))?.parse::<f64>().ok()?;
    (max > 0.0).then_some((current / max).clamp(0.0, 1.0))
}

/// Minutes to empty (or to full while charging), from energy and power (as
/// Waybar's bar-battery.sh worked it out).
fn read_remaining(path: &Path, charging: bool) -> Option<u32> {
    let num = |name: &str| read_trim(&path.join(name))?.parse::<f64>().ok();
    let power = num("power_now")?;
    if power <= 0.0 {
        return None;
    }
    let now = num("energy_now")?;
    let energy = if charging { num("energy_full")? - now } else { now };
    Some((energy * 60.0 / power).max(0.0) as u32)
}

fn read_battery(path: &Path) -> Option<(u8, bool)> {
    let percent = read_trim(&path.join("capacity"))?.parse::<u8>().ok()?.min(100);
    let status = read_trim(&path.join("status")).unwrap_or_default();
    Some((percent, matches!(status.as_str(), "Charging" | "Full")))
}

fn spawn_network(tx: Sender<Event>) {
    spawn_wifi(tx.clone());
    spawn_bluetooth(tx);
}

fn spawn_wifi(tx: Sender<Event>) {
    thread::spawn(move || {
        let mut wifi = active_wifi();
        loop {
            let child = Command::new("nmcli").arg("monitor").stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
            let Ok(mut child) = child else {
                thread::sleep(Duration::from_secs(5));
                continue;
            };
            if let Some(stdout) = child.stdout.take() {
                for _ in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let next = active_wifi();
                    if next == wifi {
                        continue;
                    }
                    let ssid = next.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "Отключено".into());
                    let _ = tx.send(Event::Wifi { ssid });
                    wifi = next;
                }
            }
            let _ = child.wait();
            thread::sleep(Duration::from_secs(1));
        }
    });
}

fn spawn_bluetooth(tx: Sender<Event>) {
    thread::spawn(move || {
        let mut previous = connected_bluetooth();
        loop {
            let child = Command::new("dbus-monitor")
                .args(["--system", "type='signal',sender='org.bluez'"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = child else {
                thread::sleep(Duration::from_secs(5));
                continue;
            };
            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    // dbus-monitor prints a header followed by the signal's
                    // fields; query BlueZ once per signal, not once per line.
                    if !line.starts_with("signal time=") {
                        continue;
                    }
                    let current = connected_bluetooth();
                    for (address, name) in &current {
                        if !previous.contains_key(address) {
                            let _ =
                                tx.send(Event::Bluetooth { name: name.clone(), battery: bluetooth_battery(address) });
                        }
                    }
                    previous = current;
                }
            }
            let _ = child.wait();
            thread::sleep(Duration::from_secs(1));
        }
    });
}

fn active_wifi() -> Option<String> {
    let out = Command::new("nmcli").args(["-t", "-f", "ACTIVE,SSID", "device", "wifi", "list"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|line| line.strip_prefix("yes:").map(unescape_nmcli))
}

fn unescape_nmcli(value: &str) -> String {
    value.replace("\\:", ":").replace("\\\\", "\\")
}

fn connected_bluetooth() -> HashMap<String, String> {
    let Ok(out) = Command::new("bluetoothctl").args(["devices", "Connected"]).output() else {
        return HashMap::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("Device ")?;
            let (address, name) = line.split_once(' ')?;
            Some((address.to_string(), name.to_string()))
        })
        .collect()
}

fn bluetooth_battery(address: &str) -> Option<u8> {
    let out = Command::new("bluetoothctl").args(["info", address]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|line| line.contains("Battery Percentage"))?;
    if let Some((_, decimal)) = line.rsplit_once('(') {
        return decimal.trim_end_matches(')').trim().parse::<u8>().ok();
    }
    line.split_whitespace().find_map(|word| word.strip_prefix("0x").and_then(|hex| u8::from_str_radix(hex, 16).ok()))
}

/// Parse `25m`, `1h30m`, `90s`, or a bare number of seconds.
pub fn parse_duration(text: &str) -> Option<f64> {
    if let Ok(seconds) = text.parse::<f64>() {
        return (seconds.is_finite() && seconds > 0.0 && seconds <= 604800.0).then_some(seconds);
    }
    let mut total = 0.0;
    let mut number = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            number.push(ch);
            continue;
        }
        let value = number.parse::<f64>().ok()?;
        number.clear();
        total += value
            * match ch {
                'h' => 3600.0,
                'm' => 60.0,
                's' => 1.0,
                _ => return None,
            };
    }
    if !number.is_empty() || !total.is_finite() || total <= 0.0 || total > 604800.0 { None } else { Some(total) }
}

pub fn timer_width(cr: &Context, renderer: &Renderer, remaining: f64) -> f64 {
    // Reserve digit cells so a second ticking over never moves the clock.
    let text: String = timer_text(remaining).chars().map(|ch| if ch.is_ascii_digit() { '8' } else { ch }).collect();
    24.0 + 6.0 + renderer.layout(cr, &text, Font::Small).pixel_extents().1.width() as f64
}

/// The countdown's colour: orange, red in its last minute.
fn timer_colour(urgent: bool) -> (f64, f64, f64) {
    if urgent { (1.0, 0.23, 0.19) } else { (1.0, 0.62, 0.12) }
}

/// The timer chip's ring, 20 px wide from `x`: the time left as an arc,
/// with pause bars inside while it is paused.
pub fn draw_timer_ring(cr: &Context, x: f64, cy: f64, urgent: bool, fraction: f64, running: bool, alpha: f64) {
    cr.save().unwrap();
    cr.set_line_cap(cairo::LineCap::Round);
    let colour = timer_colour(urgent);
    cr.set_line_width(2.4);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.18 * alpha);
    cr.new_sub_path();
    cr.arc(x + 10.0, cy, 8.0, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
    cr.new_sub_path();
    cr.arc(
        x + 10.0,
        cy,
        8.0,
        -std::f64::consts::FRAC_PI_2,
        -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * fraction.clamp(0.0, 1.0),
    );
    let _ = cr.stroke();
    if !running {
        cr.set_line_width(1.5);
        for dx in [-1.8, 1.8] {
            cr.move_to(x + 10.0 + dx, cy - 3.0);
            cr.line_to(x + 10.0 + dx, cy + 3.0);
        }
        let _ = cr.stroke();
    }
    cr.restore().unwrap();
}

/// The timer chip's reading from `x`; `tint` 0..1 takes it from white to
/// the countdown's own colour (the wings wear it, as Vorssaint's strip does).
pub fn draw_timer_reading(cr: &Context, renderer: &Renderer, x: f64, cy: f64, seconds: f64, urgent: bool, tint: f64, alpha: f64) {
    // the seconds roll (down as a countdown counts down)
    let label = timer_text(seconds);
    let text = renderer.layout(cr, &label, Font::Small);
    let (_, ext) = text.pixel_extents();
    let c = timer_colour(urgent);
    let t = tint.clamp(0.0, 1.0);
    let colour = (1.0 + (c.0 - 1.0) * t, 1.0 + (c.1 - 1.0) * t, 1.0 + (c.2 - 1.0) * t);
    crate::motion::text(cr, renderer, "timer", &label, x - ext.x() as f64,
        cy - ext.height() as f64 / 2.0 - ext.y() as f64, ext.width() as f64 + 30.0, 0.0, Font::Small,
        colour, (0.92 + 0.08 * t) * alpha);
}

fn timer_text(remaining: f64) -> String {
    let seconds = remaining.ceil().max(0.0) as u64;
    let (minutes, seconds) = (seconds / 60, seconds % 60);
    if minutes >= 60 {
        format!("{}:{:02}:{:02}", minutes / 60, minutes % 60, seconds)
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_pill(
    cr: &Context,
    renderer: &Renderer,
    art: &Art,
    event: &Event,
    value: f64,
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
    alpha: f64,
    since: f64,
    now: f64,
) {
    let enter = smoothstep((now - since) / 0.35) * alpha;
    let cy = y0 + h / 2.0;
    match event {
        Event::Battery { .. } => {}
        Event::Slider { kind, muted, value: target } => {
            let label = match kind {
                SliderKind::Volume if *muted => "Без звука",
                SliderKind::Volume => "Громкость",
                SliderKind::Brightness => "Яркость",
            };
            draw_osd_symbol(cr, renderer, kind, x0 + 35.0, y0 + 29.0, enter, *muted, *target);
            let title = renderer.layout(cr, label, Font::Caption);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.94 * enter);
            cr.move_to(renderer.snap(x0 + 57.0), renderer.snap(y0 + 19.0));
            pangocairo::functions::show_layout(cr, &title);
            // the percentage counts like Control Center's: its digits roll
            crate::motion::text(cr, renderer, &format!("osd{since:.4}"), &format!("{}%", (target * 100.0).round()),
                x0 + 26.0, y0 + 20.0, w - 52.0, 1.0, Font::Small, (1.0, 1.0, 1.0), 0.62 * enter);
            let (sx, sy, sw, sh) = (x0 + 26.0, y0 + 53.0, w - 52.0, 6.0);
            round_rect(cr, sx, sy, sw, sh, sh / 2.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.18 * enter);
            let _ = cr.fill();
            let fill = sw * if *muted { 0.0 } else { value.clamp(0.0, 1.0) };
            if fill > 0.5 {
                round_rect(cr, sx, sy, fill, sh, sh / 2.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.95 * enter);
                let _ = cr.fill();
            }
        }
        Event::Charging { percent } => {
            draw_battery(cr, x0 + 51.0, cy, *percent, true, enter);
            draw_two_lines(
                cr,
                renderer,
                "Зарядка",
                &format!("Заряжено {percent}%"),
                x0 + 91.0,
                cy,
                enter,
                crate::menu_style::GREEN,
            );
        }
        Event::BatteryLow { percent } => {
            draw_battery(cr, x0 + 51.0, cy, *percent, false, enter);
            draw_two_lines(
                cr,
                renderer,
                "Низкий заряд",
                &format!("Осталось {percent}%"),
                x0 + 91.0,
                cy,
                enter,
                crate::menu_style::RED,
            );
        }
        Event::Bluetooth { name, battery } => {
            draw_headphones(cr, x0 + 49.0, cy, enter);
            let detail = battery.map_or_else(|| "Подключено".to_string(), |p| format!("Подключено · {p}%"));
            draw_two_lines(cr, renderer, name, &detail, x0 + 88.0, cy, enter, crate::menu_style::BLUE);
        }
        Event::Wifi { ssid } => {
            draw_wifi(cr, x0 + 48.0, cy, enter);
            draw_two_lines(cr, renderer, "Wi-Fi", ssid, x0 + 88.0, cy, enter, crate::menu_style::TEAL);
        }
        Event::Screenshot { path } => {
            let size = 68.0;
            let ix = x0 + 18.0;
            let iy = cy - size / 2.0;
            let pop = 0.75 + 0.25 * smoothstep((now - since) / 0.4);
            cr.save().unwrap();
            round_rect(cr, ix + size * (1.0 - pop) / 2.0, iy + size * (1.0 - pop) / 2.0, size * pop, size * pop, 12.0);
            cr.clip();
            let url = format!("file://{}", path.to_string_lossy());
            if let Some(surface) = art.get(&url, (size * renderer.scale()).round() as i32) {
                cr.save().unwrap();
                cr.translate(ix, iy);
                cr.scale(size / surface.width() as f64, size / surface.height() as f64);
                cr.set_source_surface(&surface, 0.0, 0.0).ok();
                let _ = cr.paint_with_alpha(enter);
                cr.restore().unwrap();
            } else {
                cr.set_source_rgba(0.18, 0.18, 0.20, enter);
                let _ = cr.paint();
            }
            cr.restore().unwrap();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Снимок экрана");
            draw_two_lines(cr, renderer, "Снимок сохранён", name, x0 + 104.0, cy, enter, crate::menu_style::TEAL);
        }
        Event::TimerDone { title, detail } => {
            draw_clock(cr, x0 + 50.0, cy, enter);
            draw_two_lines(cr, renderer, title, detail, x0 + 90.0, cy, enter, crate::menu_style::ORANGE);
        }
        Event::TimerMenu => {}
        Event::WifiOpen | Event::BluetoothOpen => {}
        // drawn by the island, which holds the jobs and the calendar
        Event::Tasks | Event::CalendarOpen | Event::NotesOpen | Event::MicInUse { .. } => {}
        Event::NightMode { on } => {
            // the moon bounces in a warm (or a plain) well
            let (mx, o) = (x0 + 49.0, crate::menu_style::ORANGE);
            let colour = if *on { o } else { (1.0, 1.0, 1.0) };
            crate::menu_style::well(cr, mx, cy, 21.0, Some(colour), enter * if *on { 0.25 } else { 0.12 });
            let s = 1.5 * crate::menu_style::bounce_scale(now - since - 0.05);
            crate::menu_style::transformed(cr, mx, cy, s, 0.0, || {
                cr.save().unwrap();
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                cr.new_path();
                cr.arc(mx, cy, 6.0, 0.0, std::f64::consts::TAU);
                cr.new_sub_path();
                cr.arc(mx + 3.4, cy - 2.6, 5.2, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(colour.0, colour.1, colour.2, enter);
                let _ = cr.fill();
                cr.restore().unwrap();
            });
            draw_two_lines(cr, renderer, "Ночной режим", if *on { "Включён" } else { "Выключен" }, x0 + 88.0, cy, enter,
                if *on { o } else { crate::menu_style::TEAL });
        }
        Event::RecordCountdown => {
            // a red ring empties around the digit, which rolls down
            let t = now - since;
            let left = (RECORD_COUNTDOWN - t).max(0.0);
            let (rx, red) = (x0 + 49.0, crate::menu_style::RED);
            crate::menu_style::well(cr, rx, cy, 22.0, Some(red), enter * 0.18);
            cr.save().unwrap();
            cr.set_line_width(3.0);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_source_rgba(red.0, red.1, red.2, enter);
            cr.new_path();
            let share = left / RECORD_COUNTDOWN;
            cr.arc(rx, cy, 20.5, -std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * share);
            let _ = cr.stroke();
            cr.restore().unwrap();
            let digit = format!("{}", left.ceil().max(1.0) as u32);
            let h = crate::menu_style::line_height(cr, renderer, Font::Title);
            crate::motion::text(cr, renderer, &format!("record{since:.4}"), &digit, rx - 20.0, cy - h / 2.0, 40.0, 0.5,
                Font::Title, (1.0, 1.0, 1.0), enter);
            draw_two_lines(cr, renderer, "Запись экрана", "Весь экран", x0 + 88.0, cy, enter, red);
        }
        Event::RecorderMissing => {
            // the record ring, dim, and what to run
            let (rx, red) = (x0 + 49.0, crate::menu_style::RED);
            crate::menu_style::well(cr, rx, cy, 22.0, Some(red), enter * 0.18);
            crate::quick::glyph(cr, "record", rx, cy, 1.0, (red.0, red.1, red.2, enter));
            draw_two_lines_in(cr, renderer, "Запись экрана: нужен wf-recorder",
                "sudo sh scripts/03-install-screen-recorder.sh", x0 + 88.0, cy, w - 112.0, enter, red);
        }
        Event::Recording { thumb, seconds, path } => {
            // the first frame, 16:9, pops in like a screenshot's
            let (w, h) = (112.0, 63.0);
            let (ix, iy) = (x0 + 18.0, cy - h / 2.0);
            let pop = 0.75 + 0.25 * smoothstep((now - since) / 0.4);
            cr.save().unwrap();
            cr.translate(ix + w / 2.0, cy);
            cr.scale(pop, pop);
            cr.translate(-(ix + w / 2.0), -cy);
            round_rect(cr, ix, iy, w, h, 10.0);
            cr.clip();
            let surface = thumb.as_ref().and_then(|t| art.get(&format!("file://{}", t.to_string_lossy()), (w * renderer.scale()).round() as i32));
            match surface {
                Some(surface) => {
                    // cover the frame, keeping the picture's proportions
                    let k = (w / surface.width() as f64).max(h / surface.height() as f64);
                    cr.translate(ix + (w - surface.width() as f64 * k) / 2.0, iy + (h - surface.height() as f64 * k) / 2.0);
                    cr.scale(k, k);
                    cr.set_source_surface(&surface, 0.0, 0.0).ok();
                    let _ = cr.paint_with_alpha(enter);
                }
                None => {
                    cr.set_source_rgba(0.18, 0.18, 0.20, enter);
                    let _ = cr.paint();
                }
            }
            cr.restore().unwrap();
            let length = format!("{}:{:02}", seconds / 60, seconds % 60);
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            draw_two_lines(cr, renderer, "Запись сохранена", &format!("{length} · {name}"), x0 + 146.0, cy, enter,
                crate::menu_style::RED);
        }
        Event::MicMute { muted } => {
            // the symbol bounces as it is struck through or freed
            let s = 1.45 * crate::menu_style::bounce_scale(now - since - 0.05);
            let colour = if *muted { crate::menu_style::RED } else { crate::menu_style::ORANGE };
            crate::menu_style::well(cr, x0 + 49.0, cy, 21.0, Some(colour), enter * 0.22);
            crate::menu_style::transformed(cr, x0 + 49.0, cy, s, 0.0, || {
                crate::quick::mic(cr, x0 + 49.0, cy, *muted, enter);
            });
            let detail = if *muted { "Выключен" } else { "Включён" };
            draw_two_lines(cr, renderer, "Микрофон", detail, x0 + 88.0, cy, enter, colour);
        }
    }
}

fn draw_two_lines(
    cr: &Context,
    renderer: &Renderer,
    title: &str,
    detail: &str,
    x: f64,
    cy: f64,
    alpha: f64,
    accent: (f64, f64, f64),
) {
    draw_two_lines_in(cr, renderer, title, detail, x, cy, 255.0, alpha, accent);
}

/// Two lines as above, `width` wide.
#[allow(clippy::too_many_arguments)]
fn draw_two_lines_in(
    cr: &Context,
    renderer: &Renderer,
    title: &str,
    detail: &str,
    x: f64,
    cy: f64,
    width: f64,
    alpha: f64,
    accent: (f64, f64, f64),
) {
    let title = renderer.layout_ellipsized(cr, title, Font::Caption, width, 1);
    let detail = renderer.layout_ellipsized(cr, detail, Font::Small, width, 1);
    cr.set_source_rgba(accent.0, accent.1, accent.2, alpha);
    cr.move_to(x, cy - 23.0);
    pangocairo::functions::show_layout(cr, &title);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.72 * alpha);
    cr.move_to(x, cy + 3.0);
    pangocairo::functions::show_layout(cr, &detail);
}

pub fn draw_osd_symbol(
    cr: &Context,
    renderer: &Renderer,
    kind: &SliderKind,
    cx: f64,
    cy: f64,
    alpha: f64,
    muted: bool,
    value: f64,
) {
    // Use the same SF Symbols speaker as the user's menu bar. Fall back
    // to native vectors when the SF Pro private-use glyph is unavailable.
    if *kind == SliderKind::Volume {
        let glyph = if muted || value <= 0.0 {
            "\u{1002a3}"
        } else if value < 0.34 {
            "\u{1002a5}"
        } else if value < 0.67 {
            "\u{1002a7}"
        } else {
            "\u{1002a9}"
        };
        let layout = renderer.layout(cr, glyph, Font::Text);
        if layout.unknown_glyphs_count() == 0 {
            let (ink, _) = layout.pixel_extents();
            cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
            cr.move_to(
                renderer.snap(cx - ink.width() as f64 / 2.0 - ink.x() as f64),
                renderer.snap(cy - ink.height() as f64 / 2.0 - ink.y() as f64),
            );
            pangocairo::functions::show_layout(cr, &layout);
            return;
        }
    }
    draw_symbol(cr, kind, cx, cy, 21.0, alpha, muted, value);
}

fn draw_symbol(cr: &Context, kind: &SliderKind, cx: f64, cy: f64, size: f64, alpha: f64, muted: bool, value: f64) {
    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.scale(size / 24.0, size / 24.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    match kind {
        SliderKind::Volume => {
            // Rounded speaker body, with an optically balanced cone.
            cr.move_to(-9.0, -3.5);
            cr.line_to(-5.5, -3.5);
            cr.line_to(-0.7, -7.7);
            cr.curve_to(0.1, -8.4, 1.0, -8.0, 1.0, -7.0);
            cr.line_to(1.0, 7.0);
            cr.curve_to(1.0, 8.0, 0.1, 8.4, -0.7, 7.7);
            cr.line_to(-5.5, 3.5);
            cr.line_to(-9.0, 3.5);
            cr.curve_to(-10.0, 3.5, -10.5, 3.0, -10.5, 2.0);
            cr.line_to(-10.5, -2.0);
            cr.curve_to(-10.5, -3.0, -10.0, -3.5, -9.0, -3.5);
            cr.close_path();
            let _ = cr.fill();
            if muted || value <= 0.0 {
                cr.move_to(5.0, -3.0);
                cr.line_to(10.0, 3.0);
                cr.move_to(10.0, -3.0);
                cr.line_to(5.0, 3.0);
                let _ = cr.stroke();
            } else {
                // Each arc has its own subpath: cairo otherwise connects
                // them with a diagonal across the sound waves.
                cr.new_sub_path();
                cr.arc(1.2, 0.0, 5.0, -0.8, 0.8);
                let _ = cr.stroke();
                if value > 0.45 {
                    cr.new_sub_path();
                    cr.arc(1.2, 0.0, 9.0, -0.82, 0.82);
                    let _ = cr.stroke();
                }
            }
        }
        SliderKind::Brightness => {
            cr.new_sub_path();
            cr.arc(0.0, 0.0, 4.3, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            for i in 0..8 {
                let a = i as f64 * std::f64::consts::FRAC_PI_4;
                cr.move_to(a.cos() * 7.4, a.sin() * 7.4);
                cr.line_to(a.cos() * 10.0, a.sin() * 10.0);
            }
            let _ = cr.stroke();
        }
    }
    cr.restore().unwrap();
}

fn draw_battery(cr: &Context, cx: f64, cy: f64, percent: u8, charging: bool, alpha: f64) {
    cr.set_line_width(2.2);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.85 * alpha);
    round_rect(cr, cx - 22.0, cy - 12.0, 40.0, 24.0, 6.0);
    let _ = cr.stroke();
    round_rect(cr, cx + 20.0, cy - 5.0, 4.0, 10.0, 2.0);
    let _ = cr.fill();
    let fill = 34.0 * percent as f64 / 100.0;
    if fill > 0.0 {
        round_rect(cr, cx - 19.0, cy - 9.0, fill, 18.0, 4.0);
        if charging {
            cr.set_source_rgba(0.20, 0.78, 0.35, alpha);
        } else {
            cr.set_source_rgba(1.0, 0.23, 0.19, alpha);
        }
        let _ = cr.fill();
    }
    if charging {
        cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
        cr.move_to(cx + 2.0, cy - 15.0);
        cr.line_to(cx - 7.0, cy + 1.0);
        cr.line_to(cx, cy + 1.0);
        cr.line_to(cx - 3.0, cy + 15.0);
        cr.line_to(cx + 9.0, cy - 4.0);
        cr.line_to(cx + 2.0, cy - 4.0);
        cr.close_path();
        let _ = cr.fill();
    }
}

fn draw_headphones(cr: &Context, cx: f64, cy: f64, alpha: f64) {
    cr.new_path();
    cr.set_source_rgba(0.38, 0.72, 1.0, alpha);
    cr.set_line_width(4.0);
    cr.arc(cx, cy + 1.0, 18.0, std::f64::consts::PI, std::f64::consts::TAU);
    let _ = cr.stroke();
    for side in [-1.0, 1.0] {
        round_rect(cr, cx + side * 18.0 - 5.0, cy, 10.0, 22.0, 5.0);
        let _ = cr.fill();
    }
}

fn draw_wifi(cr: &Context, cx: f64, cy: f64, alpha: f64) {
    cr.new_path();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_source_rgba(0.40, 0.85, 1.0, alpha);
    cr.set_line_width(3.2);
    for (r, start) in [(24.0, 3.83), (16.0, 3.88), (8.0, 4.02)] {
        cr.new_sub_path();
        cr.arc(cx, cy + 13.0, r, start, 5.59);
        let _ = cr.stroke();
    }
    cr.arc(cx, cy + 13.0, 2.3, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

fn draw_clock(cr: &Context, cx: f64, cy: f64, alpha: f64) {
    cr.new_path();
    cr.set_source_rgba(1.0, 0.62, 0.12, alpha);
    cr.set_line_width(3.0);
    cr.arc(cx, cy, 20.0, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    cr.move_to(cx, cy);
    cr.line_to(cx, cy - 11.0);
    cr.move_to(cx, cy);
    cr.line_to(cx + 9.0, cy + 5.0);
    let _ = cr.stroke();
}

fn round_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = crate::menu_style::corner(r, w, h);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microphone_users_leave_out_speaker_listeners_and_meters() {
        let streams: Vec<serde_json::Value> = serde_json::from_str(r#"[
            {"source": 59, "corked": false, "properties": {"media.name": "cava", "node.name": "cava", "stream.capture.sink": "true"}},
            {"source": 60, "corked": false, "properties": {"application.name": "Telegram Desktop"}},
            {"source": 60, "corked": false, "properties": {"application.name": "Telegram Desktop"}},
            {"source": 60, "corked": true, "properties": {"application.name": "Paused"}},
            {"source": 61, "corked": false, "properties": {"application.name": "obs"}},
            {"source": 60, "corked": false, "properties": {"application.name": "pavucontrol", "media.name": "Peak detect"}}
        ]"#).unwrap();
        assert_eq!(mic_users(&streams, &[61]), vec!["Telegram Desktop".to_string()]);
    }

    #[test]
    fn wifi_icon_does_not_stroke_the_previous_text_position() {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 180, 100).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.move_to(2.0, 10.0);
        draw_wifi(&cr, 140.0, 50.0, 1.0);
        drop(cr);
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        for y in 0..100 {
            for x in 0..80 {
                assert_eq!(data[y * stride + x * 4 + 3], 0);
            }
        }
    }

    #[test]
    fn durations_accept_compound_units() {
        assert_eq!(parse_duration("25m"), Some(1500.0));
        assert_eq!(parse_duration("1h30m"), Some(5400.0));
        assert_eq!(parse_duration("90"), Some(90.0));
        assert_eq!(parse_duration("nope"), None);
    }

    #[test]
    fn every_activity_draws_offscreen() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 500, 1080).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        let renderer = Renderer::new(1.0);
        let art = Art::new();
        let events = [
            Event::Slider { kind: SliderKind::Volume, value: 0.6, muted: false },
            Event::Slider { kind: SliderKind::Brightness, value: 0.8, muted: false },
            Event::Charging { percent: 72 },
            Event::BatteryLow { percent: 10 },
            Event::Bluetooth { name: "Headphones".into(), battery: Some(84) },
            Event::Wifi { ssid: "Island".into() },
            Event::Screenshot { path: PathBuf::from("/does/not/exist.png") },
            Event::timer_done(),
        ];
        for (index, event) in events.into_iter().enumerate() {
            let y = index as f64 * 120.0 + 7.0;
            round_rect(&cr, 20.0, y, 430.0, 106.0, 42.0);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.fill().unwrap();
            cr.save().unwrap();
            draw_pill(&cr, &renderer, &art, &event, 0.65, 20.0, y, 430.0, 106.0, 1.0, 1.0, 1.4);
            cr.restore().unwrap();
        }
        round_rect(&cr, 20.0, 980.0, 220.0, 32.0, 14.0);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.fill().unwrap();
        draw_timer_ring(&cr, 32.0, 996.0, true, 0.7, true, 1.0);
        draw_timer_reading(&cr, &renderer, 62.0, 996.0, 42.0, true, 0.0, 1.0);
        surface.flush();
        if std::env::var_os("ISLAND_ACTIVITY_PREVIEW").is_some() {
            let mut file = std::fs::File::create("target/activity-preview.png").unwrap();
            surface.write_to_png(&mut file).unwrap();
        }
    }
}
