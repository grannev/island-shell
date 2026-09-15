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
    /// close the launcher if it is open (e.g. after launching)
    LauncherClose,
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
    Quit,
}

impl Command {
    pub fn parse(s: &str) -> Option<Command> {
        // the editor's messages carry JSON, which does not survive being
        // cut into words
        if let Some(json) = s.trim().strip_prefix("nvim ") {
            return Some(Command::Nvim(json.to_string()));
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
            ["set", key, value] => Command::Set(key.to_string(), value.to_string()),
            ["dump", path] => Command::Dump(path.to_string()),
            ["demo", app, rest @ ..] => {
                let rest = rest.join(" ");
                let (summary, body) = rest.split_once('|').unwrap_or((rest.as_str(), ""));
                Command::Demo(app.to_string(), summary.trim().to_string(), body.trim().to_string())
            }
            ["launcher"] => Command::Launcher,
            ["launcher-close"] => Command::LauncherClose,
            ["type", rest @ ..] => Command::Type(rest.join(" ")),
            ["key", name] => Command::Key(name.to_string()),
            ["scrub", value] => Command::Scrub(value.to_string()),
            ["quit"] => Command::Quit,
            _ => return None,
        })
    }
}

pub const USAGE: &str = "usage: island-shell [toggle|show|hide|dodge|undodge|dodge-toggle|launcher|status|set <key> <value>|quit]";

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    dir.join(format!("island-shell-{display}.sock"))
}

/// Client side: send one command, return the daemon's reply line.
pub fn send(cmd: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
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
                return Err(io::Error::new(io::ErrorKind::AddrInUse, "island-shell is already running"));
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
