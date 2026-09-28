//! The desktop notification service. D-Bus method handling stays on zbus's
//! executor; the Wayland thread receives small, ordered events through calloop.

use std::collections::HashMap;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::blocking::connection::Builder;
use zbus::zvariant::{OwnedValue, Value};
use zbus::interface;

use crate::notify::{self, Notification};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

#[derive(Clone, Debug)]
pub enum Event {
    Upsert(Notification),
    Closed(u32),
}

#[derive(Clone, Debug)]
pub enum Command {
    Action { id: u32, key: String },
    Dismiss(u32),
    PlaySound { critical: bool },
    Wake,
    EmitClosed { id: u32, reason: u32 },
}

struct Active {
    actions: Vec<(String, String)>,
    resident: bool,
    expires: Option<Instant>,
}

#[derive(Default)]
struct State {
    next_id: u32,
    active: HashMap<u32, Active>,
}

struct Service {
    state: Arc<Mutex<State>>,
    dnd: Arc<AtomicBool>,
    ui: Sender<Event>,
    commands: mpsc::Sender<Command>,
}

fn hint_byte(value: &Value<'_>) -> Option<u8> {
    match value {
        Value::U8(value) => Some(*value),
        Value::Value(inner) => hint_byte(inner),
        _ => None,
    }
}

fn hint_bool(value: &Value<'_>) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Value(inner) => hint_bool(inner),
        _ => None,
    }
}

fn inner_value<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    match value { Value::Value(inner) => inner_value(inner), other => other }
}

fn hint_string(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Str(value) => Some(value.to_string()),
        Value::Value(inner) => hint_string(inner),
        _ => None,
    }
}

fn media_name(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 1024 { return None; }
    if let Some(path) = value.strip_prefix("file://") {
        if !path.starts_with('/') { return None; }
        let mut decoded = Vec::with_capacity(path.len());
        let mut bytes = path.bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                let hi = (bytes.next()? as char).to_digit(16)?;
                let lo = (bytes.next()? as char).to_digit(16)?;
                decoded.push((hi * 16 + lo) as u8);
            } else { decoded.push(byte); }
        }
        return String::from_utf8(decoded).ok();
    }
    (!value.contains("://")).then(|| value.to_string())
}

fn image_bytes(value: &Value<'_>) -> Option<image::RgbaImage> {
    let Value::Structure(data) = inner_value(value) else { return None };
    let fields = data.fields();
    if fields.len() != 7 { return None; }
    let int = |index| match inner_value(&fields[index]) { Value::I32(n) => Some(*n), _ => None };
    let flag = match inner_value(&fields[3]) { Value::Bool(b) => *b, _ => return None };
    let (width, height, stride, bits, channels) = (int(0)?, int(1)?, int(2)?, int(4)?, int(5)?);
    if !(1..=1024).contains(&width) || !(1..=1024).contains(&height) || bits != 8
        || channels != if flag { 4 } else { 3 } || stride < width * channels
        || stride > 1024 * 4 + 4096 { return None; }
    let owned = OwnedValue::try_from(&fields[6]).ok()?;
    let bytes = Vec::<u8>::try_from(owned).ok()?;
    let needed = (height as usize - 1).checked_mul(stride as usize)?
        .checked_add(width as usize * channels as usize)?;
    if bytes.len() < needed || bytes.len() > 5_000_000 { return None; }
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height as usize {
        for col in 0..width as usize {
            let offset = row * stride as usize + col * channels as usize;
            pixels.extend_from_slice(&bytes[offset..offset + 3]);
            pixels.push(if flag { bytes[offset + 3] } else { 255 });
        }
    }
    image::RgbaImage::from_raw(width as u32, height as u32, pixels)
}

fn save_image(image: &image::RgbaImage, id: u32) -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".local/state"));
    let dir = base.join("island-shell/images");
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).ok()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_nanos();
    let path = dir.join(format!("{}-{id}-{stamp}.png", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).ok()?;
    image::DynamicImage::ImageRgba8(image.clone()).write_to(&mut file, image::ImageFormat::Png).ok()?;
    Some(path.to_string_lossy().into_owned())
}

fn play_sound(critical: bool) {
    let Some(home) = std::env::var_os("HOME") else { return };
    let (file, volume) = if critical {
        (".local/share/sounds/win7/stereo/Windows_Critical_Stop.wav", "65536")
    } else {
        (".local/share/sounds/ios/TriTone.mp3", "49152")
    };
    let path = std::path::PathBuf::from(home).join(file);
    if !path.is_file() { return; }
    let _ = std::process::Command::new("paplay")
        .arg(format!("--volume={volume}"))
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[interface(name = "org.freedesktop.Notifications", spawn = false)]
impl Service {
    fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["actions", "body", "icon-static", "persistence"]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(&self, app_name: String, replaces_id: u32, app_icon: String, summary: String,
        body: String, actions: Vec<String>, hints: HashMap<String, OwnedValue>, expire_timeout: i32) -> u32 {
        let critical = hints.get("urgency").and_then(|v| hint_byte(v)).is_some_and(|v| v >= 2);
        let transient = hints.get("transient").and_then(|v| hint_bool(v)).unwrap_or(false);
        let resident = hints.get("resident").and_then(|v| hint_bool(v)).unwrap_or(false);
        let silent = hints.get("suppress-sound").and_then(|v| hint_bool(v)).unwrap_or(false)
            || (self.dnd.load(Ordering::Relaxed) && !critical)
            || ["telegram", "thunderbird"].iter().any(|name| app_name.to_ascii_lowercase().contains(name));
        let base_kind = notify::classify(&app_name, &app_icon, false);
        let parsed_actions: Vec<_> = actions.chunks_exact(2).take(12)
            .map(|pair| (pair[0].clone(), pair[1].clone())).collect();
        let timeout = if expire_timeout == 0 { None } else {
            let millis = if expire_timeout < 0 { if critical { 0 } else { 5000 } } else { expire_timeout as u64 };
            if millis == 0 { None } else { Some(Instant::now() + Duration::from_millis(millis.clamp(100, 86_400_000))) }
        };
        let mut state = self.state.lock().unwrap();
        let id = if replaces_id != 0 && state.active.contains_key(&replaces_id) {
            replaces_id
        } else {
            loop {
                state.next_id = state.next_id.wrapping_add(1).max(1);
                if !state.active.contains_key(&state.next_id) { break state.next_id; }
            }
        };
        state.active.insert(id, Active { actions: parsed_actions.clone(), resident, expires: timeout });
        drop(state);
        let icon = media_name(&app_icon).or_else(|| hints.get("desktop-entry")
            .and_then(|value| hint_string(value)).and_then(|name| media_name(name.trim_end_matches(".desktop"))));
        let image = ["image-data", "image_data", "icon_data"].iter().find_map(|key| hints.get(*key))
            .and_then(|value| image_bytes(value)).and_then(|image| save_image(&image, id))
            .or_else(|| ["image-path", "image_path"].iter().find_map(|key| hints.get(*key))
                .and_then(|value| hint_string(value)).and_then(|name| media_name(&name)));
        let note = Notification {
            id,
            app: notify::pretty_app(&app_name, base_kind),
            summary: notify::plain(&summary),
            body: notify::plain(&body),
            kind: if critical { notify::AppKind::Critical } else { base_kind },
            critical,
            tag: None,
            progress: None,
            actions: parsed_actions,
            transient,
            icon,
            image,
        };
        let _ = self.ui.send(Event::Upsert(note));
        if !silent { let _ = self.commands.send(Command::PlaySound { critical }); }
        let _ = self.commands.send(Command::Wake);
        id
    }

    fn close_notification(&self, id: u32) -> zbus::fdo::Result<()> {
        if self.state.lock().unwrap().active.remove(&id).is_none() {
            return Err(zbus::fdo::Error::Failed("unknown notification ID".into()));
        }
        let _ = self.commands.send(Command::EmitClosed { id, reason: 3 });
        Ok(())
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("island-shell".into(), "island-shell".into(), env!("CARGO_PKG_VERSION").into(), "1.3".into())
    }

    #[zbus(signal)]
    async fn notification_closed(emitter: &zbus::object_server::SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &zbus::object_server::SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
}

fn emit_closed(conn: &zbus::blocking::Connection, ui: &Sender<Event>, id: u32, reason: u32) {
    if let Err(error) = conn.emit_signal(None::<&str>, PATH, NAME, "NotificationClosed", &(id, reason)) {
        eprintln!("island-shell: NotificationClosed: {error}");
    }
    let _ = ui.send(Event::Closed(id));
}

/// Start owning the notification name. If another daemon still owns it,
/// retry without weakening the rest of the island.
pub fn spawn(ui: Sender<Event>, dnd: Arc<AtomicBool>) -> mpsc::Sender<Command> {
    let (commands, rx) = mpsc::channel();
    let tx = commands.clone();
    std::thread::Builder::new().name("notifications".into()).spawn(move || {
        let state = Arc::new(Mutex::new(State::default()));
        let conn = loop {
            let service = Service { state: state.clone(), dnd: dnd.clone(), ui: ui.clone(), commands: tx.clone() };
            match Builder::session().and_then(|builder| builder.serve_at(PATH, service))
                .and_then(|builder| builder.name(NAME)).and_then(Builder::build) {
                Ok(conn) => break conn,
                Err(error) => {
                    eprintln!("island-shell: notification name unavailable: {error}");
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        };
        loop {
            let deadline = state.lock().unwrap().active.values().filter_map(|a| a.expires).min();
            let wait = deadline.map_or(Duration::from_secs(3600), |t| t.saturating_duration_since(Instant::now()));
            match rx.recv_timeout(wait) {
                Ok(Command::Wake) => {},
                Ok(Command::PlaySound { critical }) => play_sound(critical),
                Ok(Command::EmitClosed { id, reason }) => emit_closed(&conn, &ui, id, reason),
                Ok(Command::Dismiss(id)) => {
                    if state.lock().unwrap().active.remove(&id).is_some() { emit_closed(&conn, &ui, id, 2); }
                }
                Ok(Command::Action { id, key }) => {
                    let mut state = state.lock().unwrap();
                    let valid = state.active.get(&id).is_some_and(|a| a.actions.iter().any(|(k, _)| k == &key));
                    let close = valid && !state.active.get(&id).is_some_and(|a| a.resident);
                    if close { state.active.remove(&id); }
                    drop(state);
                    if valid {
                        if let Err(error) = conn.emit_signal(None::<&str>, PATH, NAME, "ActionInvoked", &(id, key.as_str())) {
                            eprintln!("island-shell: ActionInvoked: {error}");
                        }
                        if close { emit_closed(&conn, &ui, id, 2); }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {},
            }
            let now = Instant::now();
            let expired: Vec<_> = {
                let mut state = state.lock().unwrap();
                let ids: Vec<_> = state.active.iter().filter_map(|(id, a)| a.expires.is_some_and(|t| t <= now).then_some(*id)).collect();
                for id in &ids { state.active.remove(id); }
                ids
            };
            for id in expired { emit_closed(&conn, &ui, id, 1); }
        }
    }).expect("spawn notification server");
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hint_values_support_nested_variants() {
        assert_eq!(hint_byte(&Value::U8(2)), Some(2));
        assert_eq!(hint_bool(&Value::Bool(true)), Some(true));
    }

    #[test]
    fn image_hint_validates_dimensions_and_preserves_alpha() {
        let data = Value::from((2i32, 1i32, 8i32, true, 8i32, 4i32,
            vec![255u8, 0, 0, 128, 0, 0, 255, 255]));
        let image = image_bytes(&data).unwrap();
        assert_eq!(image.dimensions(), (2, 1));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 128]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 0, 255, 255]);
        let invalid = Value::from((0i32, 1i32, 8i32, true, 8i32, 4i32, vec![0u8; 8]));
        assert!(image_bytes(&invalid).is_none());
        assert_eq!(media_name("file:///tmp/a%20b.png").as_deref(), Some("/tmp/a b.png"));
    }

    #[test]
    #[ignore = "run inside dbus-run-session to avoid taking over the live notification name"]
    fn owns_name_replaces_and_closes_notifications() {
        let (ui, _events) = smithay_client_toolkit::reexports::calloop::channel::channel();
        let commands = spawn(ui, Arc::new(AtomicBool::new(false)));
        let conn = zbus::blocking::Connection::session().unwrap();
        let proxy = zbus::blocking::Proxy::new(&conn, NAME, PATH, NAME).unwrap();
        let capabilities: Vec<String> = (0..100).find_map(|_| {
            let result = proxy.call("GetCapabilities", &()).ok();
            if result.is_none() { std::thread::sleep(Duration::from_millis(20)); }
            result
        }).expect("notification service should own the private bus name");
        assert!(capabilities.iter().any(|c| c == "actions"));
        let server: (String, String, String, String) = proxy.call("GetServerInformation", &()).unwrap();
        assert_eq!(server.0, "island-shell");
        let empty_hints = HashMap::<String, OwnedValue>::new();
        let id: u32 = proxy.call("Notify", &("Tester", 0u32, "", "first", "body", vec!["default", "Open"], &empty_hints, 0i32)).unwrap();
        assert_ne!(id, 0);
        let replacement: u32 = proxy.call("Notify", &("Tester", id, "", "second", "body", Vec::<String>::new(), &empty_hints, 0i32)).unwrap();
        assert_eq!(replacement, id);
        let next: u32 = proxy.call("Notify", &("Tester", 0u32, "", "third", "", Vec::<String>::new(), &empty_hints, 0i32)).unwrap();
        assert_ne!(next, id);
        let _: () = proxy.call("CloseNotification", &(id,)).unwrap();
        commands.send(Command::Dismiss(next)).unwrap();
    }
}
