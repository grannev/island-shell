//! sway IPC: a subscription to window / workspace events (non-blocking, fed
//! into the event loop) plus a second, blocking connection for queries — the
//! island hides while its output shows a fullscreen window.

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::process::Command;

use serde_json::Value;

const MAGIC: &[u8; 6] = b"i3-ipc";
const RUN_COMMAND: u32 = 0;
const GET_INPUTS: u32 = 100;
const GET_WORKSPACES: u32 = 1;
const GET_OUTPUTS: u32 = 3;
const SUBSCRIBE: u32 = 2;
const GET_TREE: u32 = 4;
const EVENT_FLAG: u32 = 0x8000_0000;

fn socket_path() -> io::Result<String> {
    if let Ok(p) = std::env::var("SWAYSOCK") {
        return Ok(p);
    }
    let out = Command::new("sway").arg("--get-socketpath").output()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no sway socket"));
    }
    Ok(p)
}

fn write_msg(stream: &mut UnixStream, kind: u32, payload: &[u8]) -> io::Result<()> {
    let mut msg = Vec::with_capacity(14 + payload.len());
    msg.extend_from_slice(MAGIC);
    msg.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    msg.extend_from_slice(&kind.to_ne_bytes());
    msg.extend_from_slice(payload);
    stream.write_all(&msg)
}

fn read_msg(stream: &mut UnixStream) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 14];
    stream.read_exact(&mut header)?;
    if &header[..6] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad ipc magic"));
    }
    let len = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
    let kind = u32::from_ne_bytes(header[10..14].try_into().unwrap());
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    Ok((kind, payload))
}

/// Blocking request / reply connection.
pub struct Query {
    stream: UnixStream,
}

impl Query {
    pub fn connect() -> io::Result<Query> {
        Ok(Query { stream: UnixStream::connect(socket_path()?)? })
    }

    fn request(&mut self, kind: u32, payload: &str) -> io::Result<Value> {
        write_msg(&mut self.stream, kind, payload.as_bytes())?;
        let (_, body) = read_msg(&mut self.stream)?;
        serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Run a sway command (as `swaymsg` would).
    pub fn command(&mut self, command: &str) -> io::Result<()> {
        let reply = self.request(RUN_COMMAND, command)?;
        let ok = reply.as_array().is_some_and(|all| all.iter().all(|r| r["success"] == Value::Bool(true)));
        if ok { Ok(()) } else { Err(io::Error::other(format!("sway: {reply}"))) }
    }

    /// The active layout of the keyboard that has a choice of layouts
    /// (sway keeps them in step across keyboards).
    pub fn keyboard_layout(&mut self) -> io::Result<Option<String>> {
        let inputs = self.request(GET_INPUTS, "")?;
        let keyboards: Vec<&Value> = inputs.as_array().map(|all| {
            all.iter()
                .filter(|i| i["type"] == "keyboard")
                .filter(|i| i["xkb_layout_names"].as_array().is_some_and(|names| names.len() > 1))
                .collect()
        }).unwrap_or_default();
        let keyboard = keyboards.iter().find(|i| i["identifier"].as_str().is_some_and(|id| id.to_lowercase().contains("keyboard")))
            .or(keyboards.first());
        Ok(keyboard.and_then(|k| k["xkb_active_layout_name"].as_str().map(str::to_string)))
    }

    /// sway's focused output, and whether a fullscreen window covers the
    /// workspace visible on `output` (the island's; None: the focused one).
    pub fn outputs(&mut self, output: Option<&str>) -> io::Result<(Option<String>, bool)> {
        let workspaces = self.request(GET_WORKSPACES, "")?;
        let workspaces = workspaces.as_array().map(Vec::as_slice).unwrap_or_default();
        let focused = workspaces.iter().find(|w| w["focused"] == Value::Bool(true));
        let focused_output = focused.and_then(|w| w["output"].as_str().map(str::to_string));
        let shown = match output {
            Some(name) => workspaces
                .iter()
                .find(|w| w["visible"] == Value::Bool(true) && w["output"] == name)
                .or(focused.filter(|w| w["output"] == name)),
            None => focused,
        };
        let Some(shown) = shown.and_then(|w| w["name"].as_str().map(str::to_string)) else {
            return Ok((focused_output, false));
        };
        let tree = self.request(GET_TREE, "")?;
        // the workspace node itself reports fullscreen_mode 1: only its
        // descendants count
        let fullscreen = find_workspace(&tree, &shown).is_some_and(|ws| children(ws).any(has_fullscreen));
        Ok((focused_output, fullscreen))
    }
}

/// The part of an output that other layer surfaces' exclusive zones leave
/// free (the bar above, an on-screen keyboard below), relative to the
/// output, and the output's own logical width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Usable {
    pub x: i32,
    pub y: i32,
    pub output_w: i32,
}

impl Query {
    /// Where the free area of `output` (None: the focused one) starts:
    /// sway sizes its visible workspace to exactly that area.
    pub fn usable(&mut self, output: Option<&str>) -> io::Result<Option<Usable>> {
        let outputs = self.request(GET_OUTPUTS, "")?;
        let outputs = outputs.as_array().map(Vec::as_slice).unwrap_or_default();
        let out = match output {
            Some(name) => outputs.iter().find(|o| o["name"] == name),
            None => outputs.iter().find(|o| o["focused"] == Value::Bool(true)),
        };
        let Some(out) = out else { return Ok(None) };
        let workspaces = self.request(GET_WORKSPACES, "")?;
        let ws = workspaces.as_array().and_then(|all| all.iter().find(|w| w["name"] == out["current_workspace"]));
        let Some(ws) = ws else { return Ok(None) };
        Ok(usable_from(&out["rect"], &ws["rect"]))
    }
}

fn usable_from(output: &Value, workspace: &Value) -> Option<Usable> {
    let int = |v: &Value, k: &str| v[k].as_i64().map(|n| n as i32);
    Some(Usable {
        x: int(workspace, "x")? - int(output, "x")?,
        y: int(workspace, "y")? - int(output, "y")?,
        output_w: int(output, "width")?,
    })
}

fn find_workspace<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    if node["type"] == "workspace" && node["name"] == name {
        return Some(node);
    }
    node["nodes"].as_array()?.iter().find_map(|n| find_workspace(n, name))
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    ["nodes", "floating_nodes"].into_iter().filter_map(move |k| node[k].as_array()).flatten()
}

fn has_fullscreen(node: &Value) -> bool {
    matches!(node["fullscreen_mode"].as_u64(), Some(1 | 2)) || children(node).any(has_fullscreen)
}

/// Event subscription: a non-blocking stream to poll in the event loop.
pub struct Events {
    stream: UnixStream,
    buf: Vec<u8>,
}

impl AsFd for Events {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.stream.as_fd()
    }
}

impl Events {
    pub fn subscribe(events: &[&str]) -> io::Result<Events> {
        let mut stream = UnixStream::connect(socket_path()?)?;
        let payload = serde_json::to_string(events).unwrap();
        write_msg(&mut stream, SUBSCRIBE, payload.as_bytes())?;
        let (_, reply) = read_msg(&mut stream)?;
        if !reply.starts_with(b"{\"success\": true") && !reply.starts_with(b"{\"success\":true") {
            return Err(io::Error::other(format!("subscribe failed: {}", String::from_utf8_lossy(&reply))));
        }
        stream.set_nonblocking(true)?;
        Ok(Events { stream, buf: Vec::new() })
    }

    /// Read what is available; returns the "change" field of every complete
    /// event (the event type is in the high bits, `kind & 0x7f`).
    pub fn drain(&mut self) -> io::Result<Vec<(u32, String)>> {
        let mut chunk = [0u8; 8192];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "sway ipc closed")),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let mut out = Vec::new();
        while self.buf.len() >= 14 {
            let len = u32::from_ne_bytes(self.buf[6..10].try_into().unwrap()) as usize;
            let kind = u32::from_ne_bytes(self.buf[10..14].try_into().unwrap());
            if self.buf.len() < 14 + len {
                break;
            }
            let body: Vec<u8> = self.buf.drain(..14 + len).skip(14).collect();
            if kind & EVENT_FLAG != 0 {
                let change = serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|v| v["change"].as_str().map(str::to_string))
                    .unwrap_or_default();
                out.push((kind & 0x7f, change));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_area_is_relative_to_its_output() {
        let output = serde_json::json!({"x": 1504, "y": 0, "width": 1504, "height": 1002});
        let workspace = serde_json::json!({"x": 1504, "y": 26, "width": 1504, "height": 676});
        assert_eq!(usable_from(&output, &workspace), Some(Usable { x: 0, y: 26, output_w: 1504 }));
    }
}
