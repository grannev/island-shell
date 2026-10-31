//! niri IPC (`$NIRI_SOCKET`): the questions the island asks sway (sway.rs),
//! answered from niri's state. Requests are JSON, one per connection (niri
//! answers one line, then the island closes); the event stream is a
//! connection that keeps sending one JSON event per line, read without
//! blocking in the event loop.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{json, Value};

fn socket_path() -> io::Result<String> {
    std::env::var("NIRI_SOCKET").map_err(|_| io::Error::new(io::ErrorKind::NotFound, "no NIRI_SOCKET"))
}

/// Whether the island runs under niri: niri puts its socket in the
/// environment of its clients and the user's services, and it answers. (A
/// session that ended can leave `NIRI_SOCKET` in systemd's environment,
/// pointing at nothing: sway's session that follows must not be taken for
/// niri.)
pub fn running() -> bool {
    socket_path().is_ok_and(|path| UnixStream::connect(path).is_ok())
}

fn other(message: String) -> io::Error {
    io::Error::other(message)
}

/// One request: the value inside niri's `{"Ok": {"<Kind>": value}}` reply
/// (`"Handled"` for actions).
fn request(request: &Value) -> io::Result<Value> {
    let stage = |what: &'static str| move |e: io::Error| io::Error::new(e.kind(), format!("niri {request} ({what}): {e}"));
    let mut stream = UnixStream::connect(socket_path()?).map_err(stage("connect"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut line = serde_json::to_vec(request).map_err(|e| other(e.to_string()))?;
    line.push(b'\n');
    stream.write_all(&line).map_err(stage("write"))?;
    stream.shutdown(Shutdown::Write).map_err(stage("shutdown"))?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).map_err(stage("read"))?;
    let reply: Value = serde_json::from_str(&reply).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    match reply.get("Ok") {
        Some(Value::Object(map)) => Ok(map.values().next().cloned().unwrap_or(Value::Null)),
        Some(value) => Ok(value.clone()),
        None => Err(other(format!("niri: {}", reply["Err"]))),
    }
}

fn action(action: Value) -> io::Result<()> {
    request(&json!({ "Action": action })).map(|_| ())
}

/// A workspace's label on the bar: its name, else its index on its output.
fn label(ws: &Value) -> String {
    ws["name"].as_str().map(str::to_string)
        .unwrap_or_else(|| ws["idx"].as_u64().unwrap_or_default().to_string())
}

/// niri's state as its event stream tells it: the stream starts with the
/// whole state and then sends changes, so the island answers its questions
/// from here instead of asking niri every time something moves (a window
/// going fullscreen sends a layout change every frame).
#[derive(Default)]
pub struct State {
    workspaces: Vec<Value>,
    windows: Vec<Value>,
    layouts: Value,
}

fn set_flag(items: &mut [Value], key: &str, pick: impl Fn(&Value) -> bool) {
    for item in items {
        let on = pick(item);
        item[key] = Value::Bool(on);
    }
}

impl State {
    fn apply(&mut self, name: &str, event: &Value) {
        let id = |v: &Value| v.as_u64();
        match name {
            "WorkspacesChanged" => self.workspaces = event["workspaces"].as_array().cloned().unwrap_or_default(),
            "WorkspaceActivated" => {
                let target = id(&event["id"]);
                let output = self.workspaces.iter().find(|w| id(&w["id"]) == target).map(|w| w["output"].clone());
                if let Some(output) = output {
                    for w in self.workspaces.iter_mut().filter(|w| w["output"] == output) {
                        w["is_active"] = Value::Bool(id(&w["id"]) == target);
                    }
                }
                if event["focused"] == Value::Bool(true) {
                    set_flag(&mut self.workspaces, "is_focused", |w| id(&w["id"]) == target);
                }
            }
            "WorkspaceUrgencyChanged" => {
                if let Some(w) = self.workspaces.iter_mut().find(|w| w["id"] == event["id"]) {
                    w["is_urgent"] = event["urgent"].clone();
                }
            }
            "WorkspaceActiveWindowChanged" => {
                if let Some(w) = self.workspaces.iter_mut().find(|w| w["id"] == event["workspace_id"]) {
                    w["active_window_id"] = event["active_window_id"].clone();
                }
            }
            "WindowsChanged" => self.windows = event["windows"].as_array().cloned().unwrap_or_default(),
            "WindowOpenedOrChanged" => {
                let window = event["window"].clone();
                if window["is_focused"] == Value::Bool(true) {
                    set_flag(&mut self.windows, "is_focused", |_| false);
                }
                match self.windows.iter_mut().find(|w| w["id"] == window["id"]) {
                    Some(old) => *old = window,
                    None => self.windows.push(window),
                }
            }
            "WindowClosed" => self.windows.retain(|w| w["id"] != event["id"]),
            "WindowFocusChanged" => {
                let target = id(&event["id"]);
                set_flag(&mut self.windows, "is_focused", |w| target.is_some() && id(&w["id"]) == target);
            }
            "WindowUrgencyChanged" => {
                if let Some(w) = self.windows.iter_mut().find(|w| w["id"] == event["id"]) {
                    w["is_urgent"] = event["urgent"].clone();
                }
            }
            "WindowLayoutsChanged" => {
                for change in event["changes"].as_array().map(Vec::as_slice).unwrap_or_default() {
                    if let Some(w) = self.windows.iter_mut().find(|w| w["id"] == change[0]) {
                        w["layout"] = change[1].clone();
                    }
                }
            }
            "KeyboardLayoutsChanged" => self.layouts = event["keyboard_layouts"].clone(),
            "KeyboardLayoutSwitched" => {
                if self.layouts.is_object() {
                    self.layouts["current_idx"] = event["idx"].clone();
                }
            }
            _ => {}
        }
    }

}

/// Questions and requests to niri. With the event stream's state (the
/// island's main loop) questions are answered from it; without it (another
/// thread) they are asked, one request per connection.
pub struct Query {
    state: Option<std::rc::Rc<std::cell::RefCell<State>>>,
}

impl Query {
    pub fn connect() -> io::Result<Query> {
        // fail early (and the same way sway's does) when niri is not there
        UnixStream::connect(socket_path()?)?;
        Ok(Query { state: None })
    }

    fn workspaces_now(&self) -> io::Result<Vec<Value>> {
        match &self.state {
            Some(state) => Ok(state.borrow().workspaces.clone()),
            None => Ok(request(&json!("Workspaces"))?.as_array().cloned().unwrap_or_default()),
        }
    }

    fn windows_now(&self) -> io::Result<Vec<Value>> {
        match &self.state {
            Some(state) => Ok(state.borrow().windows.clone()),
            None => Ok(request(&json!("Windows"))?.as_array().cloned().unwrap_or_default()),
        }
    }

    /// The focused output, and whether a fullscreen window covers `output`
    /// (None: the focused one), which is `size` logical px. niri's IPC has no
    /// fullscreen flag: a tiled window as large as its output is taken for
    /// one. (The size comes from the island, whose surface covers its
    /// output: niri can take a second to answer `Outputs`.)
    pub fn outputs(&mut self, output: Option<&str>, size: (u32, u32)) -> io::Result<(Option<String>, bool)> {
        let workspaces = self.workspaces_now()?;
        let focused_name = workspaces.iter().find(|w| w["is_focused"] == Value::Bool(true))
            .and_then(|w| w["output"].as_str()).map(str::to_string);
        let Some(name) = output.map(str::to_string).or_else(|| focused_name.clone()) else {
            return Ok((focused_name, false));
        };
        let active = workspaces.iter().find(|w| {
            w["output"].as_str() == Some(&name) && w["is_active"] == Value::Bool(true)
        });
        let Some(window_id) = active.and_then(|w| w["active_window_id"].as_u64()) else {
            return Ok((focused_name, false));
        };
        let windows = self.windows_now()?;
        let Some(window) = windows.iter().find(|w| w["id"].as_u64() == Some(window_id)) else {
            return Ok((focused_name, false));
        };
        let window_size = &window["layout"]["window_size"];
        let fullscreen = window["is_floating"] != Value::Bool(true) && size.0 > 0
            && (window_size[0].as_i64(), window_size[1].as_i64()) == (Some(size.0 as i64), Some(size.1 as i64));
        Ok((focused_name, fullscreen))
    }

    /// The active keyboard layout's name, when there is a choice of them.
    pub fn keyboard_layout(&mut self) -> io::Result<Option<String>> {
        let layouts = match &self.state {
            Some(state) if state.borrow().layouts.is_object() => state.borrow().layouts.clone(),
            _ => request(&json!("KeyboardLayouts"))?,
        };
        let names = layouts["names"].as_array().map(Vec::as_slice).unwrap_or_default();
        if names.len() < 2 {
            return Ok(None);
        }
        let current = layouts["current_idx"].as_u64().unwrap_or_default() as usize;
        Ok(names.get(current).and_then(Value::as_str).map(str::to_string))
    }

    /// Every workspace, for the bar, with the applications on it. niri
    /// keeps an empty workspace at the end of each output: empty ones show
    /// only while focused, as sway's do.
    pub fn workspaces(&mut self) -> io::Result<Vec<crate::bar::Workspace>> {
        let workspaces = self.workspaces_now()?;
        let windows = self.windows_now()?;
        Ok(bar_workspaces(&workspaces, &windows))
    }

    /// Shows the workspace the bar labels `name` (on the focused output
    /// first, since indices repeat across outputs).
    pub fn focus_workspace(&mut self, name: &str) -> io::Result<()> {
        let all = self.workspaces_now()?;
        let focused = all.iter().find(|w| w["is_focused"] == Value::Bool(true))
            .and_then(|w| w["output"].as_str()).unwrap_or_default().to_string();
        let matching = |w: &&Value| label(w) == name;
        let target = all.iter().filter(matching).find(|w| w["output"].as_str() == Some(focused.as_str()))
            .or_else(|| all.iter().find(matching));
        let Some(id) = target.and_then(|w| w["id"].as_u64()) else {
            return Err(other(format!("niri: no workspace {name}")));
        };
        action(json!({ "FocusWorkspace": { "reference": { "Id": id } } }))
    }

    pub fn next_layout(&mut self) -> io::Result<()> {
        action(json!({ "SwitchLayout": { "layout": "Next" } }))
    }

    pub fn quit(&mut self) -> io::Result<()> {
        action(json!({ "Quit": { "skip_confirmation": true } }))
    }
}

/// The bar's workspaces from niri's workspaces and windows.
fn bar_workspaces(workspaces: &[Value], windows: &[Value]) -> Vec<crate::bar::Workspace> {
    let mut all: Vec<&Value> = workspaces.iter().collect();
    all.sort_by_key(|w| (w["output"].as_str().unwrap_or_default().to_string(), w["idx"].as_u64().unwrap_or_default()));
    all.into_iter().filter_map(|w| {
        let id = w["id"].as_u64();
        // its windows in the order they stand: columns left to right,
        // floating ones last
        let mut mine: Vec<&Value> = windows.iter().filter(|win| win["workspace_id"].as_u64() == id).collect();
        mine.sort_by_key(|win| {
            let pos = &win["layout"]["pos_in_scrolling_layout"];
            (pos.is_null(), pos[0].as_u64().unwrap_or(u64::MAX), pos[1].as_u64().unwrap_or(0), win["id"].as_u64())
        });
        let mut apps: Vec<String> = Vec::new();
        for win in mine {
            if let Some(app) = win["app_id"].as_str().filter(|s| !s.is_empty()) {
                if !apps.iter().any(|a| a == app) {
                    apps.push(app.to_string());
                }
            }
        }
        let focused = w["is_focused"] == Value::Bool(true);
        if apps.is_empty() && !focused {
            return None;
        }
        Some(crate::bar::Workspace {
            name: label(w),
            focused,
            urgent: w["is_urgent"] == Value::Bool(true),
            output: w["output"].as_str().unwrap_or_default().to_string(),
            apps,
        })
    }).collect()
}

/// What changed, from one batch of events.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Changes {
    pub windows: bool,
    pub workspaces: bool,
    pub layout: bool,
}

/// What an event (its name) means to the island.
fn changes_of(event: &str, changes: &mut Changes) {
    match event {
        "WorkspacesChanged" | "WorkspaceActivated" | "WorkspaceUrgencyChanged" => {
            changes.workspaces = true;
            changes.windows = true;
        }
        "WorkspaceActiveWindowChanged" | "WindowFocusChanged" | "WindowLayoutsChanged" => {
            changes.windows = true;
        }
        "WindowsChanged" | "WindowOpenedOrChanged" | "WindowClosed" | "WindowUrgencyChanged" => {
            changes.windows = true;
            changes.workspaces = true;
        }
        "KeyboardLayoutsChanged" | "KeyboardLayoutSwitched" => changes.layout = true,
        _ => {}
    }
}

/// The event stream: a non-blocking connection polled in the event loop,
/// keeping the state the query answers from.
pub struct Events {
    stream: UnixStream,
    buf: Vec<u8>,
    state: std::rc::Rc<std::cell::RefCell<State>>,
}

impl AsFd for Events {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.stream.as_fd()
    }
}

impl Events {
    /// Subscribes, and from now on `query` answers from the stream's state.
    pub fn subscribe(query: &mut Query) -> io::Result<Events> {
        let mut stream = UnixStream::connect(socket_path()?)?;
        stream.write_all(b"\"EventStream\"\n")?;
        stream.shutdown(Shutdown::Write)?;
        // the reply, then events: read the reply a byte at a time so no
        // event is swallowed into a buffer that is thrown away
        let mut reply = Vec::new();
        let mut byte = [0u8; 1];
        while stream.read(&mut byte)? == 1 && byte[0] != b'\n' {
            reply.push(byte[0]);
        }
        let reply: Value = serde_json::from_slice(&reply).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if reply.get("Ok").is_none() {
            return Err(other(format!("niri event stream: {}", reply["Err"])));
        }
        // the stream starts with the whole state: read it now, so the
        // first questions already have answers
        stream.set_read_timeout(Some(Duration::from_millis(500)))?;
        let state = std::rc::Rc::new(std::cell::RefCell::new(State::default()));
        let mut events = Events { stream, buf: Vec::new(), state: state.clone() };
        let mut reader = BufReader::new(events.stream.try_clone()?);
        for _ in 0..3 {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.is_empty() {
                break;
            }
            events.take_line(line.as_bytes());
        }
        // anything read ahead stays with the reader: keep it
        events.buf.extend_from_slice(reader.buffer());
        events.stream.set_read_timeout(None)?;
        events.stream.set_nonblocking(true)?;
        query.state = Some(state);
        Ok(events)
    }

    fn take_line(&mut self, line: &[u8]) -> Option<String> {
        // `{"EventName": {...}}`
        let Ok(Value::Object(event)) = serde_json::from_slice::<Value>(line) else { return None };
        let (name, body) = event.into_iter().next()?;
        self.state.borrow_mut().apply(&name, &body);
        Some(name)
    }

    /// Reads what has come and says what changed.
    pub fn drain(&mut self) -> io::Result<Changes> {
        let mut chunk = [0u8; 16384];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "niri ipc closed")),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let mut changes = Changes::default();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            if let Some(name) = self.take_line(&line) {
                changes_of(&name, &mut changes);
            }
        }
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_names_else_indices() {
        assert_eq!(label(&json!({"name": "web", "idx": 2})), "web");
        assert_eq!(label(&json!({"name": null, "idx": 3})), "3");
    }

    #[test]
    fn state_follows_the_stream() {
        let mut state = State::default();
        state.apply("WorkspacesChanged", &json!({"workspaces": [
            {"id": 1, "idx": 1, "name": null, "output": "eDP-1", "is_active": true, "is_focused": true, "is_urgent": false, "active_window_id": 10},
            {"id": 2, "idx": 2, "name": null, "output": "eDP-1", "is_active": false, "is_focused": false, "is_urgent": false, "active_window_id": null},
        ]}));
        state.apply("WindowsChanged", &json!({"windows": [
            {"id": 10, "app_id": "Alacritty", "workspace_id": 1, "is_focused": true, "is_floating": false,
             "layout": {"pos_in_scrolling_layout": [1, 1], "window_size": [700, 900]}},
        ]}));
        state.apply("WindowOpenedOrChanged", &json!({"window":
            {"id": 11, "app_id": "firefox", "workspace_id": 2, "is_focused": true, "is_floating": false,
             "layout": {"pos_in_scrolling_layout": [1, 1], "window_size": [700, 900]}}}));
        state.apply("WorkspaceActivated", &json!({"id": 2, "focused": true}));
        let bar = bar_workspaces(&state.workspaces, &state.windows);
        assert_eq!(bar.iter().map(|w| (w.name.as_str(), w.focused, w.apps.clone())).collect::<Vec<_>>(),
            vec![("1", false, vec!["Alacritty".to_string()]), ("2", true, vec!["firefox".to_string()])]);
        assert_eq!(state.windows.iter().filter(|w| w["is_focused"] == Value::Bool(true)).count(), 1);
        state.apply("WindowLayoutsChanged", &json!({"changes": [[11, {"window_size": [1504, 1002]}]]}));
        assert_eq!(state.windows[1]["layout"]["window_size"], json!([1504, 1002]));
        state.apply("WindowClosed", &json!({"id": 10}));
        assert_eq!(bar_workspaces(&state.workspaces, &state.windows).len(), 1);
        state.apply("KeyboardLayoutsChanged", &json!({"keyboard_layouts": {"names": ["English (US)", "Russian"], "current_idx": 0}}));
        state.apply("KeyboardLayoutSwitched", &json!({"idx": 1}));
        assert_eq!(state.layouts["current_idx"], json!(1));
    }

    #[test]
    fn events_say_what_changed() {
        let mut changes = Changes::default();
        changes_of("KeyboardLayoutSwitched", &mut changes);
        assert_eq!(changes, Changes { layout: true, ..Changes::default() });
        changes_of("WindowClosed", &mut changes);
        assert_eq!(changes, Changes { windows: true, workspaces: true, layout: true });
        let mut quiet = Changes::default();
        changes_of("OverviewOpenedOrClosed", &mut quiet);
        assert_eq!(quiet, Changes::default());
    }
}
