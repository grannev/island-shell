//! Connection operations run off the Wayland thread. Replies carry both a
//! panel lifetime and a request number, so a late scan cannot reopen a panel.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::blocking::{Connection, Proxy, connection::Builder};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wifi,
    Bluetooth,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub interface: String,
    pub profile: Option<String>,
    pub connected: bool,
    pub paired: bool,
    pub security: String,
    pub signal: u8,
    pub battery: Option<u8>,
    pub icon: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub powered: bool,
    pub devices: Vec<Device>,
}

pub enum Action {
    Refresh,
    Scan,
    Power(bool),
    Connect(Device, Option<String>),
    Disconnect(Device),
    /// remove a saved network's profiles, or unpair a device
    Forget(Device),
}

#[derive(Clone, Copy, PartialEq)]
pub enum Input {
    None,
    Pin,
    Passkey,
}

pub struct Prompt {
    pub text: String,
    pub code: String,
    pub input: Input,
    pub reply: Option<mpsc::Sender<Option<String>>>,
}

pub enum Update {
    Snapshot(Snapshot),
    Finished(Result<Snapshot, String>),
    Prompt(Prompt),
    PromptDismissed,
}

pub struct Message {
    pub session: u64,
    pub request: u64,
    pub update: Update,
}

#[derive(Clone)]
pub struct Job {
    pub session: u64,
    pub request: u64,
    pub cancel: Arc<AtomicBool>,
    pub tx: Sender<Message>,
}

impl Job {
    fn send(&self, update: Update) {
        if !self.cancel.load(Ordering::Relaxed) {
            let _ = self.tx.send(Message { session: self.session, request: self.request, update });
        }
    }
    fn canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

pub fn spawn(kind: Kind, action: Action, job: Job) {
    thread::spawn(move || {
        let result = match kind {
            Kind::Wifi => wifi_action(action, &job),
            Kind::Bluetooth => bluetooth_action(action, &job),
        };
        if result.is_err() && kind == Kind::Wifi && !job.canceled() {
            if let Ok(snapshot) = wifi_snapshot(false) {
                job.send(Update::Snapshot(snapshot));
            }
        }
        job.send(Update::Finished(result.map_err(friendly_error)));
    });
}

fn friendly_error(error: String) -> String {
    let lower = error.to_lowercase();
    if lower.contains("notready") || lower.contains("rfkill") {
        "Адаптер выключен или заблокирован. Включите его и повторите поиск.".into()
    } else if lower.contains("authentication") || lower.contains("secrets") || lower.contains("password") {
        "Не удалось подтвердить подключение. Проверьте пароль или код и повторите.".into()
    } else if lower.contains("permission") || lower.contains("not authorized") {
        "Система не разрешила это действие. Проверьте разрешения NetworkManager / Bluetooth.".into()
    } else {
        error.lines().next().unwrap_or("Не удалось выполнить действие").chars().take(200).collect()
    }
}

fn system_bus() -> Result<Connection, String> {
    Builder::system()
        .map_err(|e| e.to_string())?
        .method_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

fn nmcli(args: &[&str]) -> Result<String, String> {
    let output = Command::new("nmcli")
        .env("LC_ALL", "C")
        .args(["--wait", "15"])
        .args(args)
        .output()
        .map_err(|e| format!("NetworkManager: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim_end().to_string())
}

pub fn fields(line: &str) -> Vec<String> {
    let mut result = vec![String::new()];
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            result.last_mut().unwrap().push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ':' {
            result.push(String::new());
        } else {
            result.last_mut().unwrap().push(ch);
        }
    }
    if escaped {
        result.last_mut().unwrap().push('\\');
    }
    result
}

type Properties = HashMap<String, OwnedValue>;
type Settings = HashMap<String, Properties>;
fn string(props: &Properties, name: &str) -> String {
    props.get(name).and_then(|v| <&str>::try_from(v).ok()).unwrap_or("").to_string()
}
fn boolean(props: &Properties, name: &str) -> bool {
    props.get(name).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

fn wifi_profiles() -> Result<HashMap<String, String>, String> {
    let conn = system_bus()?;
    let settings = Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .map_err(|e| e.to_string())?;
    let paths: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).map_err(|e| e.to_string())?;
    let mut profiles = HashMap::new();
    for path in paths {
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .map_err(|e| e.to_string())?;
        // GetSettings never requests secrets.
        let Ok(data): Result<Settings, _> = proxy.call("GetSettings", &()) else { continue };
        if let (Some(wireless), Some(connection)) = (data.get("802-11-wireless"), data.get("connection")) {
            let Some(value) = wireless.get("ssid").and_then(|v| v.try_clone().ok()) else { continue };
            if let Ok(bytes) = Vec::<u8>::try_from(value) {
                profiles.insert(String::from_utf8_lossy(&bytes).into_owned(), string(connection, "uuid"));
            }
        }
    }
    Ok(profiles)
}

fn wifi_snapshot(rescan: bool) -> Result<Snapshot, String> {
    let powered = nmcli(&["radio", "wifi"])? == "enabled";
    if !powered {
        return Ok(Snapshot { powered, devices: vec![] });
    }
    let profiles = wifi_profiles()?;
    let output = nmcli(&[
        "-t",
        "-f",
        "IN-USE,SSID,SIGNAL,SECURITY,DEVICE",
        "device",
        "wifi",
        "list",
        "--rescan",
        if rescan { "yes" } else { "no" },
    ])?;
    let mut devices = parse_wifi(&output, &profiles);
    devices.extend(saved_out_of_range(&devices, &profiles));
    Ok(Snapshot { powered, devices })
}

/// Saved networks the scan did not find: they stay listed among the saved
/// ones (and can be forgotten) with no interface and no signal.
fn saved_out_of_range(found: &[Device], profiles: &HashMap<String, String>) -> Vec<Device> {
    let mut saved: Vec<Device> = profiles
        .iter()
        .filter(|(ssid, _)| !found.iter().any(|d| &d.name == *ssid))
        .map(|(ssid, uuid)| Device {
            id: format!("saved:{ssid}"),
            name: ssid.clone(),
            interface: String::new(),
            profile: Some(uuid.clone()),
            connected: false,
            paired: true,
            security: String::new(),
            signal: 0,
            battery: None,
            icon: "wifi".into(),
        })
        .collect();
    saved.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    saved
}

/// Delete every saved profile of the network `ssid` (NetworkManager may
/// hold several for one name).
fn wifi_forget(ssid: &str) -> Result<(), String> {
    let conn = system_bus()?;
    let settings = Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .map_err(|e| e.to_string())?;
    let paths: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).map_err(|e| e.to_string())?;
    let mut removed = 0;
    for path in paths {
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .map_err(|e| e.to_string())?;
        let Ok(data): Result<Settings, _> = proxy.call("GetSettings", &()) else { continue };
        let Some(value) = data.get("802-11-wireless").and_then(|w| w.get("ssid")).and_then(|v| v.try_clone().ok()) else {
            continue;
        };
        if Vec::<u8>::try_from(value).is_ok_and(|bytes| bytes == ssid.as_bytes()) {
            proxy.call::<_, _, ()>("Delete", &()).map_err(|e| e.to_string())?;
            removed += 1;
        }
    }
    if removed == 0 { Err(format!("Сеть «{ssid}» не сохранена")) } else { Ok(()) }
}

fn parse_wifi(output: &str, profiles: &HashMap<String, String>) -> Vec<Device> {
    let mut devices: Vec<Device> = vec![];
    for line in output.lines() {
        let f = fields(line);
        if f.len() < 5 || f[1].is_empty() {
            continue;
        }
        let row = Device {
            id: format!("{}:{}", f[4], f[1]),
            name: f[1].clone(),
            interface: f[4].clone(),
            profile: profiles.get(&f[1]).cloned(),
            connected: f[0] == "*",
            paired: profiles.contains_key(&f[1]),
            security: if f[3] == "--" { String::new() } else { f[3].clone() },
            signal: f[2].parse().unwrap_or(0),
            battery: None,
            icon: "wifi".into(),
        };
        if let Some(old) = devices.iter_mut().find(|d| d.id == row.id) {
            if row.connected || (!old.connected && row.signal > old.signal) {
                *old = row;
            }
        } else {
            devices.push(row);
        }
    }
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.signal.cmp(&a.signal)).then(a.name.cmp(&b.name)));
    devices
}

fn wifi_action(action: Action, job: &Job) -> Result<Snapshot, String> {
    match action {
        Action::Refresh => {}
        Action::Scan => {
            job.send(Update::Snapshot(wifi_snapshot(false)?));
            return wifi_snapshot(true);
        }
        Action::Power(on) => {
            nmcli(&["radio", "wifi", if on { "on" } else { "off" }])?;
        }
        Action::Disconnect(device) => {
            nmcli(&["device", "disconnect", &device.interface])?;
        }
        Action::Forget(device) => wifi_forget(&device.name)?,
        Action::Connect(device, password) => {
            if let Some(uuid) = device.profile.as_ref().filter(|_| password.is_none()) {
                nmcli(&["connection", "up", "uuid", uuid, "ifname", &device.interface])?;
            } else {
                wifi_new_connection(&device, password.as_deref(), job)?;
            }
        }
    }
    wifi_snapshot(false)
}

fn wifi_new_connection(device: &Device, password: Option<&str>, job: &Job) -> Result<(), String> {
    if device.security.contains("802.1X") || device.security.contains("EAP") {
        return Err("Корпоративная сеть: сначала настройте сертификат и профиль в NetworkManager.".into());
    }
    let conn = system_bus()?;
    let nm = Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .map_err(|e| e.to_string())?;
    let path: OwnedObjectPath = nm.call("GetDeviceByIpIface", &(&device.interface,)).map_err(|e| e.to_string())?;
    let access_point = wifi_access_point(&conn, &path, &device.name)?;
    let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::from([
        (
            "connection",
            HashMap::from([("id", Value::from(device.name.as_str())), ("type", Value::from("802-11-wireless"))]),
        ),
        (
            "802-11-wireless",
            HashMap::from([
                ("ssid", Value::from(device.name.as_bytes().to_vec())),
                ("mode", Value::from("infrastructure")),
            ]),
        ),
        ("ipv4", HashMap::from([("method", Value::from("auto"))])),
        ("ipv6", HashMap::from([("method", Value::from("auto"))])),
    ]);
    if !device.security.is_empty() {
        let secret = password.filter(|s| !s.is_empty()).ok_or("Введите пароль сети")?;
        let (key_mgmt, key) = if device.security.contains("WEP") {
            ("none", "wep-key0")
        } else if device.security.contains("WPA3") && !device.security.contains("WPA2") {
            ("sae", "psk")
        } else {
            ("wpa-psk", "psk")
        };
        settings.insert(
            "802-11-wireless-security",
            HashMap::from([("key-mgmt", Value::from(key_mgmt)), (key, Value::from(secret))]),
        );
    }
    // Secrets go straight to NetworkManager over D-Bus: never argv, a shell,
    // temporary files, the control socket, or the application log.
    let root = ObjectPath::try_from("/").unwrap();
    let active: OwnedObjectPath = if let Some(uuid) = &device.profile {
        let profiles = Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager/Settings",
            "org.freedesktop.NetworkManager.Settings",
        )
        .map_err(|e| e.to_string())?;
        let saved: OwnedObjectPath = profiles.call("GetConnectionByUuid", &(uuid,)).map_err(|e| e.to_string())?;
        let profile = Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            saved.clone(),
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .map_err(|e| e.to_string())?;
        let mut original: Settings = profile.call("GetSettings", &()).map_err(|e| e.to_string())?;
        if let Some(security) = settings.remove("802-11-wireless-security") {
            let owned = security
                .into_iter()
                .map(|(key, value)| value.try_to_owned().map(|v| (key.to_string(), v)))
                .collect::<Result<Properties, _>>()
                .map_err(|e| e.to_string())?;
            original.insert("802-11-wireless-security".into(), owned);
        }
        profile.call::<_, _, ()>("Update", &(original,)).map_err(|e| e.to_string())?;
        nm.call("ActivateConnection", &(saved, path, root)).map_err(|e| e.to_string())?
    } else {
        let (_, active): (OwnedObjectPath, OwnedObjectPath) =
            nm.call("AddAndActivateConnection", &(settings, path, access_point)).map_err(|e| e.to_string())?;
        active
    };
    let activation =
        Proxy::new(&conn, "org.freedesktop.NetworkManager", active, "org.freedesktop.NetworkManager.Connection.Active")
            .map_err(|e| e.to_string())?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(40) {
        let state: u32 = activation.get_property("State").map_err(|e| e.to_string())?;
        if state == 2 {
            return Ok(());
        }
        if state == 4 {
            return Err("Подключение прервано. Проверьте пароль и доступность сети.".into());
        }
        if job.canceled() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err("Сеть не ответила вовремя. Проверьте пароль и повторите подключение.".into())
}

fn wifi_access_point(conn: &Connection, device: &OwnedObjectPath, ssid: &str) -> Result<OwnedObjectPath, String> {
    let wireless = Proxy::new(
        conn,
        "org.freedesktop.NetworkManager",
        device.clone(),
        "org.freedesktop.NetworkManager.Device.Wireless",
    )
    .map_err(|e| e.to_string())?;
    let paths: Vec<OwnedObjectPath> = wireless.call("GetAccessPoints", &()).map_err(|e| e.to_string())?;
    let mut best: Option<(u8, OwnedObjectPath)> = None;
    for path in paths {
        let ap = Proxy::new(
            conn,
            "org.freedesktop.NetworkManager",
            path.clone(),
            "org.freedesktop.NetworkManager.AccessPoint",
        )
        .map_err(|e| e.to_string())?;
        let Ok(name): Result<Vec<u8>, _> = ap.get_property("Ssid") else { continue };
        if name != ssid.as_bytes() {
            continue;
        }
        let strength: u8 = ap.get_property("Strength").unwrap_or(0);
        if best.as_ref().is_none_or(|(s, _)| strength > *s) {
            best = Some((strength, path));
        }
    }
    best.map(|(_, path)| path).ok_or_else(|| "Сеть больше не видна. Обновите список и повторите.".into())
}

type Objects = HashMap<OwnedObjectPath, HashMap<String, Properties>>;
fn bluez_objects(conn: &Connection) -> Result<Objects, String> {
    Proxy::new(conn, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager")
        .map_err(|e| e.to_string())?
        .call("GetManagedObjects", &())
        .map_err(|e| e.to_string())
}

fn bluetooth_snapshot(conn: &Connection) -> Result<Snapshot, String> {
    let objects = bluez_objects(conn)?;
    let adapters: Vec<_> = objects.values().filter_map(|v| v.get("org.bluez.Adapter1")).collect();
    if adapters.is_empty() {
        return Err("Bluetooth-адаптер не найден".into());
    }
    let powered = adapters.iter().any(|p| boolean(p, "Powered"));
    let mut devices = vec![];
    for (path, interfaces) in objects {
        let Some(p) = interfaces.get("org.bluez.Device1") else { continue };
        let name = bluetooth_name(p);
        let battery =
            interfaces.get("org.bluez.Battery1").and_then(|b| b.get("Percentage")).and_then(|v| u8::try_from(v).ok());
        devices.push(Device {
            id: path.to_string(),
            name,
            interface: string(p, "Address"),
            profile: None,
            connected: boolean(p, "Connected"),
            paired: boolean(p, "Paired"),
            security: String::new(),
            signal: 0,
            battery,
            icon: string(p, "Icon"),
        });
    }
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then((a.name == UNNAMED_BLUETOOTH).cmp(&(b.name == UNNAMED_BLUETOOTH)))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.cmp(&b.id))
    });
    Ok(Snapshot { powered, devices })
}

pub const UNNAMED_BLUETOOTH: &str = "Устройство без имени";

fn bluetooth_name(properties: &Properties) -> String {
    // BlueZ's initial Alias often is the address with dashes. Respect a
    // user's alias, but prefer the real remote Name over that placeholder.
    for key in ["Alias", "Name"] {
        let candidate = string(properties, key);
        let name = candidate.trim();
        let parts: Vec<_> = name.split([':', '-']).collect();
        let address =
            parts.len() == 6 && parts.iter().all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit()));
        if !name.is_empty() && !address {
            return name.to_string();
        }
    }
    // Do not invent a product name from a random/private advertising address.
    UNNAMED_BLUETOOTH.into()
}

fn bluetooth_action(action: Action, job: &Job) -> Result<Snapshot, String> {
    let conn = system_bus()?;
    match action {
        Action::Refresh => {}
        Action::Scan | Action::Power(_) => {
            let objects = bluez_objects(&conn)?;
            let mut adapters: Vec<_> =
                objects.iter().filter_map(|(path, v)| v.get("org.bluez.Adapter1").map(|p| (path, p))).collect();
            adapters.sort_by(|a, b| boolean(b.1, "Powered").cmp(&boolean(a.1, "Powered")).then(a.0.cmp(b.0)));
            let (path, _) = adapters.first().ok_or("Bluetooth-адаптер не найден")?;
            let adapter =
                Proxy::new(&conn, "org.bluez", (*path).clone(), "org.bluez.Adapter1").map_err(|e| e.to_string())?;
            if let Action::Power(on) = action {
                for (path, _) in adapters {
                    Proxy::new(&conn, "org.bluez", path.clone(), "org.bluez.Adapter1")
                        .map_err(|e| e.to_string())?
                        .set_property("Powered", on)
                        .map_err(|e| e.to_string())?;
                }
            } else {
                adapter.call::<_, _, ()>("StartDiscovery", &()).map_err(|e| e.to_string())?;
                let started = Instant::now();
                // Allow multiple inquiry cycles for delayed remote names.
                while !job.canceled() && started.elapsed() < Duration::from_secs(30) {
                    match bluetooth_snapshot(&conn) {
                        Ok(snapshot) => job.send(Update::Snapshot(snapshot)),
                        Err(_) => break,
                    }
                    for _ in 0..8 {
                        if job.canceled() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
                // Discovery is a lease owned by this bus connection. Release
                // only our lease, including when the user closes the panel.
                let _ = adapter.call::<_, _, ()>("StopDiscovery", &());
            }
        }
        Action::Disconnect(device) => {
            Proxy::new(&conn, "org.bluez", device.id, "org.bluez.Device1")
                .map_err(|e| e.to_string())?
                .call::<_, _, ()>("Disconnect", &())
                .map_err(|e| e.to_string())?;
        }
        Action::Connect(device, _) => {
            connect_bluetooth(&device, job)?;
        }
        Action::Forget(device) => {
            // the adapter that knows the device removes it (and its pairing
            // keys); a connected device is disconnected on the way
            let adapter = Proxy::new(&conn, "org.bluez", device.id.as_str(), "org.bluez.Device1")
                .map_err(|e| e.to_string())?
                .get_property::<OwnedObjectPath>("Adapter")
                .map_err(|e| e.to_string())?;
            let path = ObjectPath::try_from(device.id.as_str()).map_err(|e| e.to_string())?;
            Proxy::new(&conn, "org.bluez", adapter, "org.bluez.Adapter1")
                .map_err(|e| e.to_string())?
                .call::<_, _, ()>("RemoveDevice", &(path,))
                .map_err(|e| e.to_string())?;
        }
    }
    bluetooth_snapshot(&conn)
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    Rejected(String),
    Canceled(String),
}

struct Agent {
    job: Job,
    device: String,
}
impl Agent {
    fn ask(&self, path: &ObjectPath<'_>, text: &str, code: String, input: Input) -> Result<String, AgentError> {
        if path.as_str() != self.device {
            return Err(AgentError::Rejected("Unexpected device".into()));
        }
        let (tx, rx) = mpsc::channel();
        self.job.send(Update::Prompt(Prompt { text: text.into(), code, input, reply: Some(tx) }));
        let started = Instant::now();
        while !self.job.canceled() && started.elapsed() < Duration::from_secs(90) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Some(answer)) => return Ok(answer),
                Ok(None) => return Err(AgentError::Rejected("Rejected by user".into())),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        Err(AgentError::Canceled("Pairing canceled".into()))
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}
    fn cancel(&self) {
        self.job.send(Update::PromptDismissed);
    }
    fn request_pin_code(&self, device: ObjectPath<'_>) -> Result<String, AgentError> {
        self.ask(&device, "Введите PIN-код устройства", String::new(), Input::Pin)
    }
    fn request_passkey(&self, device: ObjectPath<'_>) -> Result<u32, AgentError> {
        let value = self.ask(&device, "Введите код устройства (6 цифр)", String::new(), Input::Passkey)?;
        value.parse::<u32>().ok().filter(|v| *v <= 999999).ok_or_else(|| AgentError::Rejected("Invalid passkey".into()))
    }
    fn request_confirmation(&self, device: ObjectPath<'_>, passkey: u32) -> Result<(), AgentError> {
        self.ask(&device, "Код совпадает с кодом на устройстве?", format!("{passkey:06}"), Input::None).map(|_| ())
    }
    fn request_authorization(&self, device: ObjectPath<'_>) -> Result<(), AgentError> {
        self.ask(&device, "Разрешить сопряжение с этим устройством?", String::new(), Input::None).map(|_| ())
    }
    fn authorize_service(&self, device: ObjectPath<'_>, _uuid: &str) -> Result<(), AgentError> {
        if device.as_str() == self.device && !self.job.canceled() {
            Ok(())
        } else {
            Err(AgentError::Rejected("Unexpected device".into()))
        }
    }
    fn display_pin_code(&self, _device: ObjectPath<'_>, pincode: &str) {
        self.job.send(Update::Prompt(Prompt {
            text: "Введите этот код на Bluetooth-клавиатуре".into(),
            code: pincode.into(),
            input: Input::None,
            reply: None,
        }));
    }
    fn display_passkey(&self, device: ObjectPath<'_>, passkey: u32, _entered: u16) {
        self.display_pin_code(device, &format!("{passkey:06}"));
    }
}

fn connect_bluetooth(device: &Device, job: &Job) -> Result<(), String> {
    let path = ObjectPath::try_from("/org/islandshell/PairingAgent").unwrap();
    let agent = Agent { job: job.clone(), device: device.id.clone() };
    let conn = Builder::system()
        .map_err(|e| e.to_string())?
        .method_timeout(Duration::from_secs(100))
        .serve_at(path.clone(), agent)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let manager = Proxy::new(&conn, "org.bluez", "/org/bluez", "org.bluez.AgentManager1").map_err(|e| e.to_string())?;
    manager.call::<_, _, ()>("RegisterAgent", &(path.clone(), "KeyboardDisplay")).map_err(|e| e.to_string())?;
    let proxy = Proxy::new(&conn, "org.bluez", device.id.as_str(), "org.bluez.Device1").map_err(|e| e.to_string())?;
    // The agent applies only to this application, never becomes the system
    // default, and never accepts a code on the user's behalf.
    let finished = Arc::new(AtomicBool::new(false));
    let (done, canceled, device_path) = (finished.clone(), job.cancel.clone(), device.id.clone());
    let watchdog = thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            if canceled.load(Ordering::Relaxed) {
                if let Ok(bus) = system_bus() {
                    if let Ok(p) = Proxy::new(&bus, "org.bluez", device_path.as_str(), "org.bluez.Device1") {
                        let _ = p.call::<_, _, ()>("CancelPairing", &());
                    }
                }
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    let mut result =
        if device.paired { Ok(()) } else { proxy.call::<_, _, ()>("Pair", &()).map_err(|e| e.to_string()) };
    finished.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    if result.is_err() {
        let _ = proxy.call::<_, _, ()>("CancelPairing", &());
    }
    if result.is_ok() && !job.canceled() {
        // Keep this application's agent alive until profile connection has
        // completed, including AuthorizeService requests following pairing.
        result = proxy
            .call::<_, _, ()>("Connect", &())
            .map_err(|e| e.to_string())
            .or_else(|e| if e.contains("AlreadyConnected") { Ok(()) } else { Err(e) });
    }
    let _ = manager.call::<_, _, ()>("UnregisterAgent", &(path,));
    job.send(Update::PromptDismissed);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bluetooth_names_preserve_aliases_but_never_show_address_placeholders_as_names() {
        let mut properties = Properties::new();
        let text = |s: &str| OwnedValue::try_from(Value::from(s)).unwrap();
        properties.insert("Alias".into(), text("4B-15-58-C8-90-ED"));
        assert_eq!(bluetooth_name(&properties), UNNAMED_BLUETOOTH);
        properties.insert("Name".into(), text("WH-1000XM5"));
        assert_eq!(bluetooth_name(&properties), "WH-1000XM5");
        properties.insert("Alias".into(), text("Рабочие наушники"));
        assert_eq!(bluetooth_name(&properties), "Рабочие наушники");
        properties.insert("Alias".into(), text("4b:15:58:c8:90:ed"));
        assert_eq!(bluetooth_name(&properties), "WH-1000XM5");
        properties.insert("Name".into(), text("  "));
        assert_eq!(bluetooth_name(&properties), UNNAMED_BLUETOOTH);
    }

    #[test]
    fn pairing_confirmation_waits_for_the_user_and_can_be_rejected() {
        let (tx, rx) = smithay_client_toolkit::reexports::calloop::channel::channel();
        let job = Job { session: 7, request: 3, cancel: Arc::new(AtomicBool::new(false)), tx };
        let worker = thread::spawn(move || {
            let agent = Agent { job, device: "/org/bluez/hci0/dev_00_00_00_00_00_01".into() };
            agent.request_confirmation(ObjectPath::try_from(agent.device.as_str()).unwrap(), 123456)
        });
        let message = rx.recv().unwrap();
        let Update::Prompt(prompt) = message.update else { panic!("missing confirmation") };
        assert_eq!(prompt.code, "123456");
        assert!(!worker.is_finished());
        prompt.reply.unwrap().send(None).unwrap();
        assert!(matches!(worker.join().unwrap(), Err(AgentError::Rejected(_))));
    }

    #[test]
    #[ignore = "Briefly scans for Bluetooth devices, then releases discovery; never pairs"]
    fn live_discovery_and_agent_cleanup() {
        let bus = system_bus().unwrap();
        let objects = bluez_objects(&bus).unwrap();
        let (path, props) = objects
            .iter()
            .find_map(|(p, v)| v.get("org.bluez.Adapter1").filter(|v| boolean(v, "Powered")).map(|v| (p, v)))
            .expect("powered adapter");
        let was_scanning = boolean(props, "Discovering");
        let (tx, rx) = smithay_client_toolkit::reexports::calloop::channel::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job { session: 1, request: 1, cancel: cancel.clone(), tx };
        let agent_path = ObjectPath::try_from("/org/islandshell/TestAgent").unwrap();
        let conn = Builder::system()
            .unwrap()
            .serve_at(agent_path.clone(), Agent { job: job.clone(), device: String::new() })
            .unwrap()
            .build()
            .unwrap();
        let manager = Proxy::new(&conn, "org.bluez", "/org/bluez", "org.bluez.AgentManager1").unwrap();
        manager.call::<_, _, ()>("RegisterAgent", &(agent_path.clone(), "KeyboardDisplay")).unwrap();
        manager.call::<_, _, ()>("UnregisterAgent", &(agent_path,)).unwrap();
        let stopper = thread::spawn(move || {
            thread::sleep(Duration::from_secs(1));
            cancel.store(true, Ordering::Relaxed);
        });
        bluetooth_action(Action::Scan, &job).expect("Bluetooth discovery");
        stopper.join().unwrap();
        assert!(rx.try_recv().is_ok(), "scan must deliver at least one snapshot");
        let adapter = Proxy::new(&bus, "org.bluez", path.clone(), "org.bluez.Adapter1").unwrap();
        let scanning: bool = adapter.get_property("Discovering").unwrap();
        if !was_scanning {
            assert!(!scanning, "discovery leaked after cancellation");
        }
    }
    #[test]
    #[ignore = "Reads the real adapters; does not scan, connect, disconnect or change power"]
    fn live_read_only_inventory() {
        let wifi = wifi_snapshot(false).expect("NetworkManager inventory");
        if let Some(device) = wifi.devices.iter().find(|d| d.connected) {
            let conn = system_bus().unwrap();
            let nm = Proxy::new(
                &conn,
                "org.freedesktop.NetworkManager",
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.NetworkManager",
            )
            .unwrap();
            let path: OwnedObjectPath = nm.call("GetDeviceByIpIface", &(&device.interface,)).unwrap();
            wifi_access_point(&conn, &path, &device.name).expect("visible access point resolves without activating it");
        }
        eprintln!(
            "Wi-Fi: {} networks, {} saved profiles",
            wifi.devices.len(),
            wifi.devices.iter().filter(|d| d.paired).count()
        );
        let bt = bluetooth_snapshot(&system_bus().unwrap()).expect("BlueZ inventory");
        eprintln!("Bluetooth: {} devices, powered={}", bt.devices.len(), bt.powered);
    }
    #[test]
    fn wifi_scan_preserves_active_ap_and_escaped_ssids_without_truncation() {
        let profiles = HashMap::from([("Cafe: North".into(), "uuid".into())]);
        let text = "*:Cafe\\: North:30:WPA2:wlan0\n:Cafe\\: North:99:WPA2:wlan0\n:Guest:50:--:wlan0\n:A:10::wlan0\n:B:20::wlan0\n:C:30::wlan0\n:D:40::wlan0\n:E:50::wlan0";
        let rows = parse_wifi(text, &profiles);
        assert_eq!(rows.len(), 7);
        assert!(rows[0].connected);
        assert_eq!(rows[0].signal, 30);
        assert_eq!(rows[0].profile.as_deref(), Some("uuid"));
        assert!(rows.iter().find(|d| d.name == "Guest").unwrap().security.is_empty());
        // a saved network out of range is still listed, to be forgotten
        let profiles = HashMap::from([("Cafe: North".into(), "uuid".into()), ("Home".into(), "home".into())]);
        let away = saved_out_of_range(&rows, &profiles);
        assert_eq!(away.len(), 1);
        assert!(away[0].paired && away[0].interface.is_empty() && away[0].profile.as_deref() == Some("home"));
    }
}
