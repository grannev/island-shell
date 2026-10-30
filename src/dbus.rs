//! D-Bus watchers on their own threads (zbus, blocking API): the active
//! MPRIS player on the session bus, and fprintd's verification signals on
//! the system bus. Events are pushed into the event loop through a calloop
//! channel.

use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

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
    /// the player's own name ("Telegram", "Firefox"…)
    pub app: String,
    pub playing: bool,
    pub artist: String,
    pub title: String,
    /// `mpris:artUrl`, usually a file:// path
    pub art_url: String,
    /// `mpris:trackid`, needed to seek
    pub track_id: String,
    /// track length in seconds, 0 when the player does not say
    pub length: f64,
}

pub enum Event {
    /// The active player changed (None: nobody is playing or paused).
    Player(Option<NowPlaying>),
    /// fprintd started a verification.
    AuthStarted,
    /// fprintd VerifyStatus: (status, done).
    AuthStatus(String, bool),
    /// where the active player is in the track, in seconds
    Position(f64),
    /// the lock screen the island asked for has gone (or never came)
    Unlocked,
}

#[derive(Clone, Copy, Debug)]
pub enum PlayerCommand {
    PlayPause,
    Pause,
    Next,
    Previous,
    /// jump to this position, in seconds
    Seek(f64),
}

/// Send a control to the player; the call runs on a throwaway thread so a
/// frozen player never stalls the island.
pub fn control(bus_name: &str, track_id: &str, cmd: PlayerCommand) {
    let name = bus_name.to_string();
    let track = track_id.to_string();
    thread::spawn(move || {
        let run = || -> zbus::Result<()> {
            let conn = Connection::session()?;
            let proxy = Proxy::new(&conn, name.as_str(), MPRIS_PATH, PLAYER_IFACE)?;
            match cmd {
                PlayerCommand::PlayPause => proxy.call_noreply("PlayPause", &()),
                PlayerCommand::Pause => proxy.call_noreply("Pause", &()),
                PlayerCommand::Next => proxy.call_noreply("Next", &()),
                PlayerCommand::Previous => proxy.call_noreply("Previous", &()),
                PlayerCommand::Seek(secs) => {
                    let path = zbus::zvariant::ObjectPath::try_from(track.as_str())
                        .map_err(|e| zbus::Error::Variant(e.into()))?;
                    proxy.call_noreply("SetPosition", &(path, (secs * 1e6) as i64))
                }
            }
        };
        if let Err(e) = run() {
            eprintln!("island-shell: player control: {e}");
        }
    });
}

/// A player the user cleared off the island: it stays hidden until it
/// plays again after the pause has taken.
struct Cleared {
    name: String,
    /// seen paused (or stopped) since it was cleared
    paused: bool,
    at: Instant,
}

static CLEARED: Mutex<Vec<Cleared>> = Mutex::new(Vec::new());
/// the next look at the players also clears every one that is not playing
static SWEEP: Mutex<bool> = Mutex::new(false);
/// a player that ignores Pause is let back after this long
const PAUSE_GRACE: Duration = Duration::from_secs(3);

/// Pause the player and take it (and any other paused one) off the
/// island, without quitting it; it comes back when it plays again.
pub fn clear_player(bus_name: &str, playing: bool) {
    if let Ok(mut cleared) = CLEARED.lock() {
        cleared.retain(|c| c.name != bus_name);
        cleared.push(Cleared { name: bus_name.to_string(), paused: !playing, at: Instant::now() });
    }
    if let Ok(mut sweep) = SWEEP.lock() {
        *sweep = true;
    }
    if playing {
        control(bus_name, "", PlayerCommand::Pause);
    }
}

/// Is this player cleared off the island right now?
fn cleared_hides(cleared: &mut Vec<Cleared>, name: &str, playing: bool, sweep: bool) -> bool {
    let Some(i) = cleared.iter().position(|c| c.name == name) else {
        if sweep && !playing {
            cleared.push(Cleared { name: name.to_string(), paused: true, at: Instant::now() });
            return true;
        }
        return false;
    };
    if !playing {
        cleared[i].paused = true;
        return true;
    }
    // playing again once the pause had taken (or Pause was ignored)
    if cleared[i].paused || cleared[i].at.elapsed() > PAUSE_GRACE {
        cleared.remove(i);
        return false;
    }
    true
}

/// Ask the player where it is, on a thread — the island never blocks on a
/// D-Bus call of its own.
pub fn spawn_position(bus_name: &str, tx: Sender<Event>) {
    let name = bus_name.to_string();
    thread::spawn(move || {
        if let Some(position) = position(&name) {
            let _ = tx.send(Event::Position(position));
        }
    });
}

/// Where the player is in the track, in seconds (a property, so it has to
/// be asked for; nobody signals it).
pub fn position(bus_name: &str) -> Option<f64> {
    let conn = Connection::session().ok()?;
    let proxy = Proxy::new(&conn, bus_name, MPRIS_PATH, PLAYER_IFACE).ok()?;
    let us: i64 = proxy.get_property("Position").ok()?;
    Some(us as f64 / 1e6)
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
    let art_url = meta.get("mpris:artUrl").and_then(|v| str_of(v)).unwrap_or_default();
    let track_id = meta.get("mpris:trackid").and_then(|v| path_of(v)).unwrap_or_default();
    let length = meta.get("mpris:length").and_then(|v| int_of(v)).unwrap_or(0) as f64 / 1e6;
    Some(NowPlaying {
        bus_name: name.to_string(),
        app: player_label(name),
        playing,
        artist,
        title,
        art_url,
        track_id,
        length,
    })
}

fn int_of(v: &Value<'_>) -> Option<i64> {
    match v {
        Value::I64(n) => Some(*n),
        Value::U64(n) => Some(*n as i64),
        Value::I32(n) => Some(*n as i64),
        Value::U32(n) => Some(*n as i64),
        Value::F64(n) => Some(*n as i64),
        Value::Value(inner) => int_of(inner),
        _ => None,
    }
}

/// `mpris:trackid` should be an object path but some players send a string.
fn path_of(v: &Value<'_>) -> Option<String> {
    match v {
        Value::ObjectPath(p) => Some(p.to_string()),
        Value::Str(s) => Some(s.to_string()),
        Value::Value(inner) => path_of(inner),
        _ => None,
    }
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
        let sweep = SWEEP.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or(false);
        for name in self.names() {
            let Some(mut np) = read_player(&self.conn, &name) else { continue };
            let hidden = CLEARED.lock().map(|mut c| cleared_hides(&mut c, &name, np.playing, sweep)).unwrap_or(false);
            if hidden {
                continue;
            }
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
                    if new.is_empty() {
                        // gone: a new one under that name starts afresh
                        if let Ok(mut cleared) = CLEARED.lock() {
                            cleared.retain(|c| c.name != name);
                        }
                    } else {
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

/// After the island has locked or suspended: wait (on a thread) for the lock
/// screen to come and go, then tell the island to let the screen out. A
/// locker that never shows up within a few seconds counts as gone, so the
/// screen is never left switched off.
pub fn watch_unlock(tx: Sender<Event>) {
    thread::spawn(move || {
        let locker = || {
            std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|entry| {
                std::fs::read_to_string(entry.path().join("comm"))
                    .is_ok_and(|comm| matches!(comm.trim(), "hyprlock" | "swaylock"))
            })
        };
        let started = Instant::now();
        let mut seen = false;
        while started.elapsed() < Duration::from_secs(6) {
            if locker() {
                seen = true;
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
        while seen && locker() {
            thread::sleep(Duration::from_millis(250));
        }
        let _ = tx.send(Event::Unlocked);
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleared_player_returns_when_it_plays_again() {
        let mut cleared = vec![Cleared { name: "a".into(), paused: false, at: Instant::now() }];
        // the pause has not taken yet: still hidden
        assert!(cleared_hides(&mut cleared, "a", true, false));
        assert!(cleared_hides(&mut cleared, "a", false, false));
        assert!(cleared_hides(&mut cleared, "a", false, false));
        // played again: back, and no longer cleared
        assert!(!cleared_hides(&mut cleared, "a", true, false));
        assert!(cleared.is_empty());
        // others are left alone, unless a sweep takes the paused ones
        assert!(!cleared_hides(&mut cleared, "b", false, false));
        assert!(!cleared_hides(&mut cleared, "c", true, true));
        assert!(cleared_hides(&mut cleared, "b", false, true));
        assert!(cleared_hides(&mut cleared, "b", false, false));
    }

    #[test]
    fn a_player_that_ignores_pause_comes_back() {
        let mut cleared = vec![Cleared { name: "a".into(), paused: false, at: Instant::now() - PAUSE_GRACE * 2 }];
        assert!(!cleared_hides(&mut cleared, "a", true, false));
    }
}
