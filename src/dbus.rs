//! D-Bus watchers on their own threads (zbus, blocking API): the active
//! MPRIS player on the session bus, and fprintd's verification signals on
//! the system bus. Events are pushed into the event loop through a calloop
//! channel.

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::blocking::{Connection, MessageIterator, Proxy, fdo::DBusProxy};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{MatchRule, message::Type};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

/// What the island shows for the active player.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub bus_name: String,
    pub playing: bool,
    pub artist: String,
    pub title: String,
}

pub enum Event {
    /// The active player changed (None: nobody is playing or paused).
    Player(Option<NowPlaying>),
    /// fprintd started a verification.
    AuthStarted,
    /// fprintd VerifyStatus: (status, done).
    AuthStatus(String, bool),
}

#[derive(Clone, Copy, Debug)]
pub enum PlayerCommand {
    PlayPause,
    Next,
    Previous,
}

/// Send a control to the player; the call runs on a throwaway thread so a
/// frozen player never stalls the island.
pub fn control(bus_name: &str, cmd: PlayerCommand) {
    let name = bus_name.to_string();
    thread::spawn(move || {
        let method = match cmd {
            PlayerCommand::PlayPause => "PlayPause",
            PlayerCommand::Next => "Next",
            PlayerCommand::Previous => "Previous",
        };
        let run = || -> zbus::Result<()> {
            let conn = Connection::session()?;
            let proxy = Proxy::new(&conn, name.as_str(), MPRIS_PATH, PLAYER_IFACE)?;
            proxy.call_noreply(method, &())
        };
        if let Err(e) = run() {
            eprintln!("island-shell: player {method}: {e}");
        }
    });
}

pub fn spawn_player_watch(tx: Sender<Event>) {
    thread::Builder::new()
        .name("mpris".into())
        .spawn(move || {
            if let Err(e) = player_loop(&tx) {
                eprintln!("island-shell: mpris watch: {e}");
            }
        })
        .expect("spawn mpris thread");
}

/// Playback status and metadata of one player; `None` when it is stopped
/// or gone.
fn read_player(conn: &Connection, name: &str) -> Option<NowPlaying> {
    let proxy = Proxy::new(conn, name, MPRIS_PATH, PLAYER_IFACE).ok()?;
    let status: String = proxy.get_property("PlaybackStatus").ok()?;
    let playing = match status.as_str() {
        "Playing" => true,
        "Paused" => false,
        _ => return None,
    };
    let meta: HashMap<String, OwnedValue> = proxy.get_property("Metadata").unwrap_or_default();
    let title = meta.get("xesam:title").and_then(|v| str_of(v)).unwrap_or_default();
    let artist = meta
        .get("xesam:artist")
        .and_then(|v| match &**v {
            Value::Array(a) => Some(a.iter().filter_map(str_of).collect::<Vec<_>>().join(", ")),
            v => str_of(v),
        })
        .unwrap_or_default();
    Some(NowPlaying { bus_name: name.to_string(), playing, artist, title })
}

fn str_of(v: &Value<'_>) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        Value::Value(inner) => str_of(inner),
        _ => None,
    }
}

/// A player's name as a fallback title (Telegram publishes no title for
/// the first track it plays).
fn player_label(bus_name: &str) -> String {
    let short = bus_name.strip_prefix(MPRIS_PREFIX).unwrap_or(bus_name);
    let short = short.split(".instance").next().unwrap_or(short);
    match short {
        "TelegramDesktop" => "Telegram".into(),
        s if s.starts_with("firefox") => "Firefox".into(),
        "" => "Player".into(),
        s => s.into(),
    }
}

struct Players {
    conn: Connection,
    dbus: DBusProxy<'static>,
    /// unique name → well-known name, for signals
    owners: HashMap<String, String>,
    /// well-known name → when it last changed (a tie-breaker)
    recent: HashMap<String, u64>,
    stamp: u64,
    last: Option<NowPlaying>,
}

impl Players {
    fn names(&mut self) -> Vec<String> {
        let mut names = Vec::new();
        if let Ok(list) = self.dbus.list_names() {
            for n in list {
                let n = n.to_string();
                if n.starts_with(MPRIS_PREFIX) {
                    if let Ok(owner) = self.dbus.get_name_owner(n.as_str().try_into().unwrap()) {
                        self.owners.insert(owner.to_string(), n.clone());
                    }
                    names.push(n);
                }
            }
        }
        names
    }

    /// Pick the player to show: playing before paused, then the one that
    /// changed most recently.
    fn active(&mut self) -> Option<NowPlaying> {
        let mut best: Option<(bool, u64, NowPlaying)> = None;
        for name in self.names() {
            let Some(mut np) = read_player(&self.conn, &name) else { continue };
            if np.title.is_empty() && np.artist.is_empty() {
                // metadata not published yet: give the player a moment
                for _ in 0..6 {
                    thread::sleep(Duration::from_millis(250));
                    match read_player(&self.conn, &name) {
                        Some(n) if !n.title.is_empty() || !n.artist.is_empty() => {
                            np = n;
                            break;
                        }
                        Some(n) => np = n,
                        None => break,
                    }
                }
                if np.title.is_empty() && np.artist.is_empty() {
                    np.title = player_label(&name);
                }
            }
            let stamp = self.recent.get(&name).copied().unwrap_or(0);
            let better = match &best {
                None => true,
                Some((playing, s, _)) => (np.playing, stamp) > (*playing, *s),
            };
            if better {
                best = Some((np.playing, stamp, np));
            }
        }
        best.map(|(_, _, np)| np)
    }

    fn touch(&mut self, unique: &str) {
        self.stamp += 1;
        if let Some(name) = self.owners.get(unique).cloned() {
            self.recent.insert(name, self.stamp);
        }
    }

    fn publish(&mut self, tx: &Sender<Event>) {
        let now = self.active();
        if now != self.last {
            self.last = now.clone();
            let _ = tx.send(Event::Player(now));
        }
    }
}

fn player_loop(tx: &Sender<Event>) -> zbus::Result<()> {
    let conn = Connection::session()?;
    let dbus = DBusProxy::new(&conn)?;
    let props = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .arg(0, PLAYER_IFACE)?
        .build();
    let names = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg0ns(MPRIS_PREFIX.trim_end_matches('.'))?
        .build();
    let iter = MessageIterator::from(&conn);
    dbus.add_match_rule(props)?;
    dbus.add_match_rule(names)?;

    let mut players = Players { conn: conn.clone(), dbus, owners: HashMap::new(), recent: HashMap::new(), stamp: 0, last: None };
    players.publish(tx);

    for msg in iter {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        match header.member().map(|m| m.as_str()) {
            Some("PropertiesChanged") => {
                if let Some(sender) = header.sender() {
                    players.touch(sender.as_str());
                }
                // a burst of changes (track switch) arrives as several
                // signals: settle briefly, then read once
                thread::sleep(Duration::from_millis(50));
                players.publish(tx);
            }
            Some("NameOwnerChanged") => {
                if let Ok((name, _old, new)) = msg.body().deserialize::<(String, String, String)>() {
                    if !new.is_empty() {
                        players.owners.insert(new, name.clone());
                        players.stamp += 1;
                        players.recent.insert(name, players.stamp);
                    }
                }
                players.publish(tx);
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn spawn_auth_watch(tx: Sender<Event>) {
    thread::Builder::new()
        .name("fprintd".into())
        .spawn(move || {
            if let Err(e) = auth_loop(&tx) {
                eprintln!("island-shell: fprintd watch: {e}");
            }
        })
        .expect("spawn fprintd thread");
}

fn auth_loop(tx: &Sender<Event>) -> zbus::Result<()> {
    let conn = Connection::system()?;
    let rule = MatchRule::builder().msg_type(Type::Signal).interface("net.reactivated.Fprint.Device")?.build();
    let iter = MessageIterator::for_match_rule(rule, &conn, Some(16))?;
    for msg in iter {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        match header.member().map(|m| m.as_str()) {
            Some("VerifyFingerSelected") => {
                let _ = tx.send(Event::AuthStarted);
            }
            Some("VerifyStatus") => {
                if let Ok((status, done)) = msg.body().deserialize::<(String, bool)>() {
                    let _ = tx.send(Event::AuthStatus(status, done));
                }
            }
            _ => {}
        }
    }
    Ok(())
}
