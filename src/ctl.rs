//! Control socket: `island-shell <command>` talks to the running daemon over
//! a Unix socket in `$XDG_RUNTIME_DIR` (one line in, one line back), which is
//! what the sway bindings and the ⋯ drawer script call.

use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Toggle,
    Show,
    Hide,
    Dodge,
    Undodge,
    DodgeToggle,
    Status,
    /// `set <key> <value>`: change a tunable live (see island::Config)
    Set(String, String),
    /// `dump <file.png>`: render the current frame to a PNG (debugging)
    Dump(String),
    /// `demo <app> <summary> <body>`: fabricate a notification (debugging)
    Demo(String, String, String),
    /// mod+d: unfold into the launcher panel, or fold back
    Launcher,
    /// Open the four shortcuts normally revealed by holding the island.
    Quick,
    /// Open the native notification history sheet.
    Notifications,
    /// `dnd <on|off|toggle|status>` controls quiet notifications.
    Dnd(String),
    /// close the launcher if it is open (e.g. after launching)
    LauncherClose,
    /// Close modal menus, like a click on the transparent outside region.
    Dismiss,
    /// type into the launcher's search field (debugging)
    Type(String),
    /// `key <escape|return|up|down|backspace>`: a key for the launcher
    /// (debugging, and handy from a script)
    Key(String),
    /// `scrub <0..1|end>`: drag the playhead from a script, exactly as the
    /// pointer does — repeated calls move it, "end" lands it
    Scrub(String),
    /// `nvim <json>`: a message from the companion Neovim plugin
    Nvim(String),
    /// A live activity supplied explicitly (also used by screenshot hooks).
    Activity(crate::activity::Event),
    /// `timer <duration|pause|resume|toggle|cancel|status>`.
    Timer(String),
    /// `task …` / `progress …`: a job shown as a live activity
    Task(crate::tasks::Request),
    /// start (after 3-2-1) or stop a screen recording
    Record,
    Quit,
}

impl Command {
    pub fn parse(s: &str) -> Option<Command> {
        // the editor's messages carry JSON, which does not survive being
        // cut into words
        if let Some(json) = s.trim().strip_prefix("nvim ") {
            return Some(Command::Nvim(json.to_string()));
        }
        // a value may hold spaces (a clock format): keep them as typed
        if let Some((key, value)) = s.trim().strip_prefix("set ").and_then(|rest| rest.trim_start().split_once(char::is_whitespace)) {
            let value = value.trim();
            return (!value.is_empty()).then(|| Command::Set(key.to_string(), value.to_string()));
        }
        if let Some(path) = s.trim().strip_prefix("screenshot ") {
            return (!path.is_empty()).then(|| Command::Activity(crate::activity::Event::Screenshot { path: PathBuf::from(path) }));
        }
        let words: Vec<&str> = s.split_whitespace().collect();
        Some(match words.as_slice() {
            ["toggle"] => Command::Toggle,
            ["show"] => Command::Show,
            ["hide"] => Command::Hide,
            ["dodge"] => Command::Dodge,
            ["undodge"] => Command::Undodge,
            ["dodge-toggle"] => Command::DodgeToggle,
            ["status"] => Command::Status,
            ["dump", path] => Command::Dump(path.to_string()),
            ["demo", app, rest @ ..] => {
                let rest = rest.join(" ");
                let (summary, body) = rest.split_once('|').unwrap_or((rest.as_str(), ""));
                Command::Demo(app.to_string(), summary.trim().to_string(), body.trim().to_string())
            }
            ["launcher"] => Command::Launcher,
            ["quick"] | ["menu"] => Command::Quick,
            ["notifications"] => Command::Notifications,
            ["dnd", action] => Command::Dnd(action.to_string()),
            ["dnd"] => Command::Dnd("toggle".into()),
            ["launcher-close"] => Command::LauncherClose,
            ["dismiss"] => Command::Dismiss,
            ["type", rest @ ..] => Command::Type(rest.join(" ")),
            ["key", name] => Command::Key(name.to_string()),
            ["scrub", value] => Command::Scrub(value.to_string()),
            ["volume", value, tail @ ..] => Command::Activity(crate::activity::Event::Slider {
                kind: crate::activity::SliderKind::Volume,
                value: value.parse::<f64>().ok()?.clamp(0.0, 1.0),
                muted: tail.contains(&"muted"),
            }),
            ["brightness", value] => Command::Activity(crate::activity::Event::Slider {
                kind: crate::activity::SliderKind::Brightness,
                value: value.parse::<f64>().ok()?.clamp(0.0, 1.0),
                muted: false,
            }),
            ["battery", percent, "charging"] => Command::Activity(crate::activity::Event::Charging {
                percent: percent.parse::<u8>().ok()?.min(100),
            }),
            ["battery", percent, "low"] => Command::Activity(crate::activity::Event::BatteryLow {
                percent: percent.parse::<u8>().ok()?.min(100),
            }),
            ["bluetooth", battery, name @ ..] if !name.is_empty() => Command::Activity(crate::activity::Event::Bluetooth {
                name: name.join(" "),
                battery: (battery != &"-").then(|| battery.parse::<u8>().ok().map(|p| p.min(100))).flatten(),
            }),
            ["wifi", ssid @ ..] if !ssid.is_empty() => Command::Activity(crate::activity::Event::Wifi { ssid: ssid.join(" ") }),
            ["timer", action] => Command::Timer(action.to_string()),
            ["timer"] => Command::Timer("menu".into()),
            ["timer", "start", duration] => Command::Timer(duration.to_string()),
            ["wifi"] => Command::Activity(crate::activity::Event::WifiOpen),
            ["calendar"] => Command::Activity(crate::activity::Event::CalendarOpen),
            ["record"] => Command::Record,
            ["mic", "muted"] => Command::Activity(crate::activity::Event::MicMute { muted: true }),
            ["mic", "on"] => Command::Activity(crate::activity::Event::MicMute { muted: false }),
            ["bluetooth"] => Command::Activity(crate::activity::Event::BluetoothOpen),
            ["task", rest @ ..] => Command::Task(crate::tasks::parse(rest, false)?),
            ["progress", rest @ ..] => Command::Task(crate::tasks::parse(rest, true)?),
            ["quit"] => Command::Quit,
            _ => return None,
        })
    }
}

pub const USAGE: &str = concat!(
    "usage: island-shell [toggle|show|hide|dodge|undodge|dodge-toggle|quick|notifications|dnd <on|off|toggle|status>|launcher|wifi|bluetooth|calendar|record|",
    "timer <duration|pause|resume|toggle|cancel|status>|screenshot <file>|",
    "task [--pid=N] start|set|done|fail|cancel <id> …|task list|progress <title> <n>%|run [--title T] [--] <command>|",
    "status|set <key> <value>|quit]",
);

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    dir.join(format!("island-shell-{display}.sock"))
}

/// Client side: send one command, return the daemon's reply (one line,
/// or several for a listing; the daemon closes the connection after it).
pub fn send(cmd: &str) -> io::Result<String> {
    use std::io::Read;
    let mut stream = UnixStream::connect(socket_path())?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply.trim_end().to_string())
}

/// Server side: a non-blocking listener; `accept_all` drains pending
/// connections and answers each with the string the handler returns.
pub struct Server {
    listener: UnixListener,
}

impl Server {
    pub fn bind() -> io::Result<Server> {
        let path = socket_path();
        // a stale socket from a crashed daemon: take it over if nobody answers
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                // not a failure worth a restart loop from systemd: the island
                // is up, just not this process
                eprintln!("island-shell: already running");
                std::process::exit(0);
            }
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Server { listener })
    }

    pub fn accept_all(&self, mut handle: impl FnMut(Command) -> String) {
        loop {
            let (stream, _) = match self.listener.accept() {
                Ok(c) => c,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) => {
                    eprintln!("island-shell: control socket: {e}");
                    return;
                }
            };
            // the client writes its line right after connecting; a short
            // blocking read is fine here
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(200)));
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let reply = match Command::parse(&line) {
                Some(cmd) => handle(cmd),
                None => format!("error: unknown command {:?}", line.trim()),
            };
            let mut stream = reader.into_inner();
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.write_all(b"\n");
        }
    }
}

impl AsFd for Server {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.listener.as_fd()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(socket_path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_keeps_spaces_inside_the_value() {
        assert!(matches!(
            Command::parse("set clock %a %b %-d  %H:%M"),
            Some(Command::Set(key, value)) if key == "clock" && value == "%a %b %-d  %H:%M"
        ));
        assert!(matches!(Command::parse("set hover 0"), Some(Command::Set(key, value)) if key == "hover" && value == "0"));
        assert!(Command::parse("set hover").is_none());
    }

    #[test]
    fn parses_live_activity_commands() {
        assert_eq!(
            Command::parse("volume 0.42 muted"),
            Some(Command::Activity(crate::activity::Event::Slider {
                kind: crate::activity::SliderKind::Volume,
                value: 0.42,
                muted: true,
            }))
        );
        assert_eq!(
            Command::parse("wifi A network with spaces"),
            Some(Command::Activity(crate::activity::Event::Wifi {
                ssid: "A network with spaces".into(),
            }))
        );
        assert_eq!(
            Command::parse("screenshot /tmp/A shot.png"),
            Some(Command::Activity(crate::activity::Event::Screenshot {
                path: PathBuf::from("/tmp/A shot.png"),
            }))
        );
        assert_eq!(Command::parse("timer start 25m"), Some(Command::Timer("25m".into())));
        assert!(matches!(Command::parse("progress --pid=9 Сборка 40%"), Some(Command::Task(r)) if r.owner == Some(9)));
        assert!(Command::parse("task frobnicate x").is_none());
    }
}
