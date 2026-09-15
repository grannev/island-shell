//! Neovim: the island doubles as the editor's status line. A Neovim that
//! runs our companion plugin (`nvim/island.lua`) pushes its state into the
//! island's control socket and the island can call back into it over its
//! RPC socket. Instances that do not carry the token the plugin reads from
//! `~/.config/island-shell/nvim-token` are ignored, so a plain Neovim
//! never shows up here.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use serde_json::Value;

use crate::anim::Time;

pub const PROTOCOL: &str = "island.nvim/1";

/// How long an editor may stay quiet before the island forgets it (the
/// plugin sends a heartbeat with every state; a killed Neovim stops).
pub const STALE_AFTER: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    Replace,
    Command,
    Terminal,
    Other,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s.chars().next().unwrap_or('n') {
            'i' => Mode::Insert,
            'v' | 'V' | '\u{16}' => Mode::Visual,
            'R' => Mode::Replace,
            'c' => Mode::Command,
            't' => Mode::Terminal,
            'n' => Mode::Normal,
            _ => Mode::Other,
        }
    }

    /// The letter on the chip, and its colour.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "N",
            Mode::Insert => "I",
            Mode::Visual => "V",
            Mode::Replace => "R",
            Mode::Command => "C",
            Mode::Terminal => "T",
            Mode::Other => "·",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::Replace => "REPLACE",
            Mode::Command => "COMMAND",
            Mode::Terminal => "TERMINAL",
            Mode::Other => "",
        }
    }

    pub fn colour(self) -> (f64, f64, f64) {
        let hex = match self {
            Mode::Normal => 0x66D9FF,
            Mode::Insert => 0x34C759,
            Mode::Visual => 0xFFB340,
            Mode::Replace => 0xFF3B30,
            Mode::Command => 0xBF5AF2,
            Mode::Terminal => 0x8A94A6,
            Mode::Other => 0x8A94A6,
        };
        (((hex >> 16) & 0xff) as f64 / 255.0, ((hex >> 8) & 0xff) as f64 / 255.0, (hex & 0xff) as f64 / 255.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    pub errors: u32,
    pub warnings: u32,
    pub info: u32,
    pub hints: u32,
}

impl Diagnostics {
    pub fn any(&self) -> bool {
        self.errors + self.warnings + self.info + self.hints > 0
    }
}

/// One entry of the editor's buffer list.
#[derive(Clone, Debug, PartialEq)]
pub struct Buffer {
    pub id: i64,
    pub name: String,
    pub lang: String,
    pub modified: bool,
    pub current: bool,
}

/// Everything one Neovim tells the island about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Editor {
    /// the RPC socket to call back into
    pub server: String,
    pub focused: bool,
    pub mode: String,
    pub file: String,
    /// the directory the file sits in, shortened with ~
    pub dir: String,
    pub lang: String,
    pub modified: bool,
    pub line: u32,
    pub col: u32,
    pub lines: u32,
    pub percent: u32,
    pub encoding: String,
    pub indent: String,
    pub diagnostics: Diagnostics,
    pub branch: String,
    pub added: u32,
    pub removed: u32,
    /// the register a macro is being recorded into
    pub recording: String,
    /// search matches: (current, total)
    pub search: Option<(u32, u32)>,
    pub buffers: Vec<Buffer>,
    pub cwd: String,
}

impl Editor {
    pub fn mode(&self) -> Mode {
        Mode::parse(&self.mode)
    }

    /// The parts the collapsed island shows: a change here is worth a frame.
    pub fn chip_state(&self) -> (Mode, bool, Diagnostics, String, Option<(u32, u32)>, String) {
        (self.mode(), self.modified, self.diagnostics.clone(), self.recording.clone(), self.search, self.file.clone())
    }
}

/// A message the plugin sent us.
#[derive(Clone, Debug)]
pub enum Message {
    /// full state (the plugin sends it whenever something changes)
    State(Box<Editor>),
    /// the editor went away (its own socket path, so one instance leaving
    /// does not silence another)
    Gone(String),
    /// something to show in a pill: kind, title, body, progress 0..1
    Toast { kind: ToastKind, title: String, body: String, progress: Option<f64> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Saved,
    Done,
    Error,
    Warning,
    Progress,
    Git,
    Info,
}

impl ToastKind {
    fn parse(s: &str) -> ToastKind {
        match s {
            "saved" => ToastKind::Saved,
            "done" => ToastKind::Done,
            "error" => ToastKind::Error,
            "warn" | "warning" => ToastKind::Warning,
            "progress" => ToastKind::Progress,
            "git" => ToastKind::Git,
            _ => ToastKind::Info,
        }
    }
}

/// The token the plugin has to quote. Created on first run, 0600.
pub fn token() -> String {
    let path = token_path();
    if let Ok(text) = std::fs::read_to_string(&path) {
        let token = text.trim().to_string();
        if !token.is_empty() {
            return token;
        }
    }
    let token = fresh_token();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, format!("{token}\n")).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
    token
}

pub fn token_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("island-shell")
        .join("nvim-token")
}

fn fresh_token() -> String {
    // 16 bytes of the kernel's randomness, hex (read exactly: /dev/urandom
    // never reaches an end)
    let mut bytes = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| {
        use std::io::Read;
        f.read_exact(&mut bytes)
    });
    match read.is_ok().then_some(bytes) {
        Some(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        None => {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            format!("{:016x}{:08x}", t.as_nanos() as u64, std::process::id())
        }
    }
}

/// Parse one `nvim <json>` line. `None` when it is not for us (wrong
/// protocol, or a Neovim without our plugin trying its luck).
pub fn parse(json: &str, token: &str) -> Option<Message> {
    let value: Value = serde_json::from_str(json).ok()?;
    if value["protocol"].as_str() != Some(PROTOCOL) || value["token"].as_str() != Some(token) {
        return None;
    }
    match value["event"].as_str()? {
        "state" => Some(Message::State(Box::new(editor_from(&value)))),
        "gone" => Some(Message::Gone(value["server"].as_str().unwrap_or_default().to_string())),
        "toast" => Some(Message::Toast {
            kind: ToastKind::parse(value["kind"].as_str().unwrap_or("info")),
            title: value["title"].as_str().unwrap_or_default().to_string(),
            body: value["body"].as_str().unwrap_or_default().to_string(),
            progress: value["progress"].as_f64(),
        }),
        _ => None,
    }
}

fn editor_from(v: &Value) -> Editor {
    let str_of = |key: &str| v[key].as_str().unwrap_or_default().to_string();
    let u32_of = |key: &str| v[key].as_u64().unwrap_or(0) as u32;
    let diagnostics = Diagnostics {
        errors: v["diagnostics"]["errors"].as_u64().unwrap_or(0) as u32,
        warnings: v["diagnostics"]["warnings"].as_u64().unwrap_or(0) as u32,
        info: v["diagnostics"]["info"].as_u64().unwrap_or(0) as u32,
        hints: v["diagnostics"]["hints"].as_u64().unwrap_or(0) as u32,
    };
    let buffers = v["buffers"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|b| Buffer {
                    id: b["id"].as_i64().unwrap_or(0),
                    name: b["name"].as_str().unwrap_or_default().to_string(),
                    lang: b["lang"].as_str().unwrap_or_default().to_string(),
                    modified: b["modified"].as_bool().unwrap_or(false),
                    current: b["current"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    let search = match (v["search"]["current"].as_u64(), v["search"]["total"].as_u64()) {
        (Some(c), Some(t)) if t > 0 => Some((c as u32, t as u32)),
        _ => None,
    };
    Editor {
        server: str_of("server"),
        focused: v["focused"].as_bool().unwrap_or(true),
        mode: str_of("mode"),
        file: str_of("file"),
        dir: str_of("dir"),
        lang: str_of("lang"),
        modified: v["modified"].as_bool().unwrap_or(false),
        line: u32_of("line"),
        col: u32_of("col"),
        lines: u32_of("lines"),
        percent: u32_of("percent"),
        encoding: str_of("encoding"),
        indent: str_of("indent"),
        diagnostics,
        branch: str_of("branch"),
        added: u32_of("added"),
        removed: u32_of("removed"),
        recording: str_of("recording"),
        search,
        buffers,
        cwd: str_of("cwd"),
    }
}

/// Call back into the editor: `nvim --server <socket> --remote-expr <expr>`
/// on a thread, so a busy editor never stalls a frame.
pub fn call(server: &str, expr: &str) {
    if server.is_empty() {
        return;
    }
    let (server, expr) = (server.to_string(), expr.to_string());
    thread::spawn(move || {
        let status = Command::new("nvim")
            .args(["--server", &server, "--remote-expr", &expr])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Err(e) = status {
            eprintln!("island-shell: nvim call: {e}");
        }
    });
}

/// Time helper: has this editor gone quiet?
pub fn stale(last_seen: Time, now: Time) -> bool {
    now - last_seen > STALE_AFTER
}
