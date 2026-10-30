//! The tray: the island is the session's `org.kde.StatusNotifierWatcher`
//! (as Waybar was — the same properties and signals, so scripts that count
//! `RegisteredStatusNotifierItems` keep working) and its only host. Items
//! register with the watcher; a host thread reads their icons and titles,
//! follows `NewIcon` / `NewStatus` / `NewTitle`, calls `Activate`,
//! `SecondaryActivate`, `ContextMenu` and `Scroll` for the bar, and reads
//! their menus (`com.canonical.dbusmenu`) for the island's menu sheets.
//! Everything here runs on its own threads; the bar gets snapshots.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::{OwnedValue, Value};
use zbus::MatchRule;

use crate::background::Pixels;
use crate::bar_sources::BarEvent;

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";

/// A tray item as the bar shows it.
pub struct TrayItem {
    pub id: String,
    pub title: String,
    pub icon: Option<Pixels>,
    /// it has a dbusmenu
    pub has_menu: bool,
    /// a click shows its menu rather than activating it
    pub item_is_menu: bool,
}

/// One entry of an item's menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    /// a check or radio mark: Some(on)
    pub toggle: Option<bool>,
    pub children: Vec<MenuEntry>,
}

/// What the island asks of the tray.
pub enum Request {
    Activate { id: String, x: i32, y: i32 },
    Secondary { id: String, x: i32, y: i32 },
    ContextMenu { id: String, x: i32, y: i32 },
    Scroll { id: String, delta: i32 },
    /// read the item's menu (it comes back as `BarEvent::Menu`)
    Menu { id: String },
    /// an entry of the item's menu was chosen
    MenuClicked { id: String, entry: i32 },
}

enum HostMsg {
    Registered(String, String),
    /// a bus name (unique or well-known) changed owner; `gone` when it left
    Owner { name: String, gone: bool },
    /// an item's sender signalled a change
    Changed(String),
    Request(Request),
}

struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
    host: mpsc::Sender<HostMsg>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher", spawn = false)]
impl Watcher {
    fn register_status_notifier_item(&self, #[zbus(header)] header: zbus::message::Header<'_>, service: &str) {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        // a path means "on my own connection"; a name means at the usual path
        let (bus, path) = if service.starts_with('/') {
            (sender, service.to_string())
        } else {
            (service.to_string(), "/StatusNotifierItem".to_string())
        };
        let _ = self.host.send(HostMsg::Registered(bus, path));
    }

    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().map(|i| i.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

/// Start the watcher and the host (once the watcher's name is free: while
/// Waybar holds it, the tray waits). Returns where to send requests.
pub fn spawn(ui: Sender<BarEvent>, icon_px: i32) -> mpsc::Sender<Request> {
    let (host_tx, host_rx) = mpsc::channel::<HostMsg>();
    let (requests, request_rx) = mpsc::channel::<Request>();
    {
        let host_tx = host_tx.clone();
        thread::Builder::new().name("tray-requests".into()).spawn(move || {
            for request in request_rx {
                if host_tx.send(HostMsg::Request(request)).is_err() { return; }
            }
        }).expect("spawn tray-requests");
    }
    let items = Arc::new(Mutex::new(Vec::new()));
    let watcher_items = items.clone();
    let watcher_host = host_tx.clone();
    thread::Builder::new().name("tray".into()).spawn(move || {
        let conn = loop {
            let watcher = Watcher { items: watcher_items.clone(), host: watcher_host.clone() };
            match zbus::blocking::connection::Builder::session()
                .and_then(|b| b.serve_at(WATCHER_PATH, watcher))
                .and_then(|b| b.name(WATCHER))
                .and_then(|b| b.build())
            {
                Ok(conn) => break conn,
                Err(_) => thread::sleep(Duration::from_secs(5)),
            }
        };
        listen(&conn, host_tx.clone());
        host(conn, items, host_rx, ui, icon_px);
    }).expect("spawn tray");
    requests
}

/// Item signals and owner changes, fed to the host.
fn listen(conn: &Connection, host: mpsc::Sender<HostMsg>) {
    let conn = conn.clone();
    thread::Builder::new().name("tray-signals".into()).spawn(move || {
        let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return };
        let rules = [
            MatchRule::builder().msg_type(Type::Signal).interface(ITEM).map(|b| b.build()),
            MatchRule::builder().msg_type(Type::Signal).interface("org.freedesktop.DBus")
                .and_then(|b| b.member("NameOwnerChanged")).map(|b| b.build()),
        ];
        for rule in rules.into_iter().flatten() {
            let _ = dbus.add_match_rule(rule);
        }
        for msg in MessageIterator::from(&conn) {
            let Ok(msg) = msg else { continue };
            let header = msg.header();
            let member = header.member().map(|m| m.to_string()).unwrap_or_default();
            if member == "NameOwnerChanged" {
                if let Ok((name, _old, new)) = msg.body().deserialize::<(String, String, String)>() {
                    let _ = host.send(HostMsg::Owner { name, gone: new.is_empty() });
                }
            } else if header.interface().is_some_and(|i| i.as_str() == ITEM) {
                if let Some(sender) = header.sender() {
                    let _ = host.send(HostMsg::Changed(sender.to_string()));
                }
            }
        }
    }).expect("spawn tray-signals");
}

struct Known {
    bus: String,
    path: String,
    /// the unique name behind `bus`, to match signals
    owner: String,
    menu: Option<String>,
    item_is_menu: bool,
}

fn host(conn: Connection, items: Arc<Mutex<Vec<String>>>, rx: mpsc::Receiver<HostMsg>, ui: Sender<BarEvent>, icon_px: i32) {
    let loader = crate::iconload::Loader::new();
    let mut known: Vec<(String, Known)> = Vec::new();
    let mut cache: HashMap<String, TrayItem> = HashMap::new();
    let emit = |conn: &Connection, member: &str, id: &str| {
        let _ = conn.emit_signal(None::<&str>, WATCHER_PATH, WATCHER, member, &(id,));
    };
    let publish = |known: &[(String, Known)], cache: &mut HashMap<String, TrayItem>, ui: &Sender<BarEvent>| {
        let list: Vec<TrayItem> = known.iter().filter_map(|(id, _)| cache.get(id)).map(|t| TrayItem {
            id: t.id.clone(),
            title: t.title.clone(),
            icon: t.icon.clone(),
            has_menu: t.has_menu,
            item_is_menu: t.item_is_menu,
        }).collect();
        let _ = ui.send(BarEvent::Tray(list));
    };
    for msg in rx {
        match msg {
            HostMsg::Registered(bus, path) => {
                let id = format!("{bus}{path}");
                if known.iter().any(|(k, _)| *k == id) {
                    continue;
                }
                let owner = if bus.starts_with(':') { bus.clone() } else {
                    zbus::blocking::fdo::DBusProxy::new(&conn).ok()
                        .and_then(|d| d.get_name_owner(bus.as_str().try_into().ok()?).ok())
                        .map(|o| o.to_string()).unwrap_or_else(|| bus.clone())
                };
                known.push((id.clone(), Known { bus, path, owner, menu: None, item_is_menu: false }));
                if let Ok(mut list) = items.lock() { list.push(id.clone()); }
                emit(&conn, "StatusNotifierItemRegistered", &id);
                if let Some(item) = read_item(&conn, &id, &mut known, &loader, icon_px) {
                    cache.insert(id, item);
                }
                publish(&known, &mut cache, &ui);
            }
            HostMsg::Owner { name, gone: true } => {
                let gone: Vec<String> = known.iter().filter(|(_, k)| k.bus == name || k.owner == name).map(|(id, _)| id.clone()).collect();
                for id in gone {
                    known.retain(|(k, _)| *k != id);
                    cache.remove(&id);
                    if let Ok(mut list) = items.lock() { list.retain(|k| *k != id); }
                    emit(&conn, "StatusNotifierItemUnregistered", &id);
                }
                publish(&known, &mut cache, &ui);
            }
            HostMsg::Owner { .. } => {}
            HostMsg::Changed(sender) => {
                // signals burst (icon, then status…): settle, then read once
                thread::sleep(Duration::from_millis(40));
                let ids: Vec<String> = known.iter().filter(|(_, k)| k.owner == sender || k.bus == sender).map(|(id, _)| id.clone()).collect();
                for id in ids {
                    if let Some(item) = read_item(&conn, &id, &mut known, &loader, icon_px) {
                        cache.insert(id, item);
                    }
                }
                publish(&known, &mut cache, &ui);
            }
            HostMsg::Request(request) => serve(&conn, &known, request, &ui),
        }
    }
}

fn proxy<'a>(conn: &Connection, bus: &'a str, path: &'a str, iface: &'a str) -> Option<zbus::blocking::Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(conn)
        .destination(bus).ok()?
        .path(path).ok()?
        .interface(iface).ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build().ok()
}

/// Read an item's state; None when it is passive (Waybar hid those too) or
/// cannot be read.
fn read_item(conn: &Connection, id: &str, known: &mut [(String, Known)], loader: &crate::iconload::Loader, px: i32) -> Option<TrayItem> {
    let (_, k) = known.iter_mut().find(|(k, _)| k == id)?;
    let p = proxy(conn, &k.bus, &k.path, ITEM)?;
    let status: String = p.get_property("Status").unwrap_or_else(|_| "Active".into());
    if status == "Passive" {
        return None;
    }
    let title: String = p.get_property("Title").unwrap_or_default();
    k.menu = p.get_property::<zbus::zvariant::OwnedObjectPath>("Menu").ok().map(|m| m.to_string()).filter(|m| m != "/");
    k.item_is_menu = p.get_property("ItemIsMenu").unwrap_or(false);
    let theme_path: String = p.get_property("IconThemePath").unwrap_or_default();
    let name: String = p.get_property(if status == "NeedsAttention" { "AttentionIconName" } else { "IconName" }).unwrap_or_default();
    let pixmaps: Vec<(i32, i32, Vec<u8>)> = p.get_property(if status == "NeedsAttention" { "AttentionIconPixmap" } else { "IconPixmap" }).unwrap_or_default();
    let icon = icon_by_name(loader, &name, &theme_path, px).or_else(|| icon_from_pixmaps(&pixmaps, px)).and_then(Pixels::from_surface);
    Some(TrayItem { id: id.to_string(), title, icon, has_menu: k.menu.is_some(), item_is_menu: k.item_is_menu })
}

fn icon_by_name(loader: &crate::iconload::Loader, name: &str, theme_path: &str, px: i32) -> Option<cairo::ImageSurface> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return crate::iconload::load(std::path::Path::new(name)).and_then(|s| scaled(&s, px));
    }
    if !theme_path.is_empty() {
        let base = std::path::Path::new(theme_path);
        for candidate in [base.join(format!("{name}.png")), base.join(format!("{name}.svg"))] {
            if let Some(s) = crate::iconload::load(&candidate) {
                return scaled(&s, px);
            }
        }
    }
    loader.get_scaled(name, px)
}

/// The pixmap closest to `px` (preferring larger), converted from the
/// protocol's ARGB in network byte order to cairo's premultiplied pixels.
fn icon_from_pixmaps(pixmaps: &[(i32, i32, Vec<u8>)], px: i32) -> Option<cairo::ImageSurface> {
    let best = pixmaps.iter().filter(|(w, h, d)| *w > 0 && *h > 0 && d.len() >= (*w * *h * 4) as usize)
        .min_by_key(|(w, _, _)| if *w >= px { (*w - px, 0) } else { (px - *w, 1) })?;
    let (w, h, data) = best;
    scaled(&argb_surface(*w, *h, data)?, px)
}

/// Pixels in the protocol's ARGB (network byte order) as a cairo surface.
fn argb_surface(w: i32, h: i32, data: &[u8]) -> Option<cairo::ImageSurface> {
    let (w, h) = (&w, &h);
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, *w, *h).ok()?;
    let stride = surface.stride() as usize;
    {
        let mut out = surface.data().ok()?;
        for y in 0..*h as usize {
            for x in 0..*w as usize {
                let i = (y * *w as usize + x) * 4;
                let (a, r, g, b) = (data[i] as u32, data[i + 1] as u32, data[i + 2] as u32, data[i + 3] as u32);
                let o = y * stride + x * 4;
                out[o] = (b * a / 255) as u8;
                out[o + 1] = (g * a / 255) as u8;
                out[o + 2] = (r * a / 255) as u8;
                out[o + 3] = a as u8;
            }
        }
    }
    surface.mark_dirty();
    Some(surface)
}

fn scaled(source: &cairo::ImageSurface, px: i32) -> Option<cairo::ImageSurface> {
    let target = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).ok()?;
    let cr = cairo::Context::new(&target).ok()?;
    let k = px as f64 / source.width().max(source.height()).max(1) as f64;
    cr.translate((px as f64 - source.width() as f64 * k) / 2.0, (px as f64 - source.height() as f64 * k) / 2.0);
    cr.scale(k, k);
    cr.set_source_surface(source, 0.0, 0.0).ok()?;
    cr.paint().ok()?;
    drop(cr);
    target.flush();
    Some(target)
}

fn serve(conn: &Connection, known: &[(String, Known)], request: Request, ui: &Sender<BarEvent>) {
    let find = |id: &str| known.iter().find(|(k, _)| k == id).map(|(_, k)| k);
    match request {
        Request::Activate { id, x, y } | Request::Secondary { id, x, y } | Request::ContextMenu { id, x, y }
            if find(&id).is_none() => { let _ = (x, y); }
        Request::Activate { id, x, y } => call(conn, find(&id), "Activate", &(x, y)),
        Request::Secondary { id, x, y } => call(conn, find(&id), "SecondaryActivate", &(x, y)),
        Request::ContextMenu { id, x, y } => call(conn, find(&id), "ContextMenu", &(x, y)),
        Request::Scroll { id, delta } => call(conn, find(&id), "Scroll", &(delta, "vertical")),
        Request::Menu { id } => {
            let Some(k) = find(&id) else { return };
            let Some(menu) = &k.menu else { return };
            let Some(p) = proxy(conn, &k.bus, menu, MENU) else { return };
            let _ = p.call::<_, _, bool>("AboutToShow", &(0i32,));
            let layout: zbus::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))> =
                p.call("GetLayout", &(0i32, -1i32, Vec::<String>::new()));
            if let Ok((_, root)) = layout {
                let entry = parse_entry(root.0, &root.1, &root.2);
                let _ = ui.send(BarEvent::Menu(id, entry.children));
            }
        }
        Request::MenuClicked { id, entry } => {
            let Some(k) = find(&id) else { return };
            let Some(menu) = &k.menu else { return };
            let Some(p) = proxy(conn, &k.bus, menu, MENU) else { return };
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
            let _ = p.call_noreply("Event", &(entry, "clicked", Value::from(0i32), stamp));
        }
    }
}

fn call<B: serde::Serialize + zbus::zvariant::DynamicType>(conn: &Connection, k: Option<&Known>, method: &str, body: &B) {
    let Some(k) = k else { return };
    if let Some(p) = proxy(conn, &k.bus, &k.path, ITEM) {
        let _ = p.call_noreply(method, body);
    }
}

/// A dbusmenu layout node: (id, properties, children as variants).
fn parse_entry(id: i32, props: &HashMap<String, OwnedValue>, children: &[OwnedValue]) -> MenuEntry {
    let text = |key: &str| props.get(key).and_then(|v| <&str>::try_from(&**v).ok().map(str::to_string));
    let flag = |key: &str| props.get(key).and_then(|v| bool::try_from(&**v).ok());
    let int = |key: &str| props.get(key).and_then(|v| i32::try_from(&**v).ok());
    // mnemonics: "_File" → "File", "__" → "_"
    let label = text("label").unwrap_or_default().replace("__", "\u{1}").replace('_', "").replace('\u{1}', "_");
    let toggle = text("toggle-type").filter(|t| !t.is_empty()).map(|_| int("toggle-state") == Some(1));
    let kids = children.iter().filter_map(|child| {
        let structure = zbus::zvariant::Structure::try_from(match &**child { Value::Value(inner) => (**inner).try_clone().ok()?, v => v.try_clone().ok()? }).ok()?;
        let fields = structure.fields();
        let id = i32::try_from(fields.first()?).ok()?;
        let props: HashMap<String, OwnedValue> = match fields.get(1)? {
            // a{sv}: each value is itself a variant; unwrap it
            Value::Dict(d) => d.iter().filter_map(|(k, v)| {
                let v = match v { Value::Value(inner) => (**inner).try_clone().ok()?, v => v.try_clone().ok()? };
                Some((<&str>::try_from(k).ok()?.to_string(), OwnedValue::try_from(v).ok()?))
            }).collect(),
            _ => HashMap::new(),
        };
        // hidden entries are not shown at all
        if props.get("visible").and_then(|v| bool::try_from(&**v).ok()) == Some(false) {
            return None;
        }
        let grand: Vec<OwnedValue> = match fields.get(2)? {
            Value::Array(a) => a.iter().filter_map(|v| OwnedValue::try_from(v.try_clone().ok()?).ok()).collect(),
            _ => Vec::new(),
        };
        Some(parse_entry(id, &props, &grand))
    }).collect();
    MenuEntry {
        id,
        label,
        enabled: flag("enabled").unwrap_or(true),
        separator: text("type").as_deref() == Some("separator"),
        toggle,
        children: kids,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixmaps_become_premultiplied_cairo_pixels() {
        // one opaque red pixel and one half-transparent white, ARGB bytes
        let data = vec![255, 255, 0, 0, 128, 255, 255, 255];
        let mut s = argb_surface(2, 1, &data).unwrap();
        let d = s.data().unwrap();
        assert_eq!(&d[0..4], &[0, 0, 255, 255]);
        assert_eq!(&d[4..8], &[128, 128, 128, 128]);
        drop(d);
        assert!(icon_from_pixmaps(&[(2, 1, data)], 18).is_some_and(|s| s.width() == 18));
    }

    #[test]
    fn menu_labels_drop_their_mnemonics() {
        let mut props = HashMap::new();
        props.insert("label".to_string(), OwnedValue::try_from(Value::from("_Open __file")).unwrap());
        let e = parse_entry(3, &props, &[]);
        assert_eq!(e.label, "Open _file");
        assert!(e.enabled && !e.separator && e.toggle.is_none());
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Reads a running item's menu (set TRAY_ITEM=":1.38/StatusNotifierItem").
    #[test]
    #[ignore = "talks to a live tray item"]
    fn read_a_live_menu() {
        let Ok(item) = std::env::var("TRAY_ITEM") else { return };
        let (bus, path) = item.split_at(item.find('/').unwrap());
        let conn = Connection::session().unwrap();
        let p = proxy(&conn, bus, path, ITEM).unwrap();
        let menu: zbus::zvariant::OwnedObjectPath = p.get_property("Menu").unwrap();
        let title: String = p.get_property("Title").unwrap_or_default();
        let m = proxy(&conn, bus, menu.as_str(), MENU).unwrap();
        let (_, root): (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)) = m.call("GetLayout", &(0i32, -1i32, Vec::<String>::new())).unwrap();
        let entry = parse_entry(root.0, &root.1, &root.2);
        fn show(e: &MenuEntry, depth: usize) {
            eprintln!("{}[{}] {:?}{}{}", "  ".repeat(depth), e.id, e.label, if e.separator { " ---" } else { "" }, if e.enabled { "" } else { " (off)" });
            for c in &e.children { show(c, depth + 1); }
        }
        eprintln!("{title}:");
        show(&entry, 0);
        assert!(!entry.children.is_empty());
    }
}
