//! The compositor the island runs under: sway (SwayFX) through sway.rs, or
//! niri (started with `startniri`) through niri.rs. Everything the island
//! asks of the compositor or tells it goes through here.

use std::io;
use std::os::fd::{AsFd, BorrowedFd};

use crate::{niri, sway};

pub use crate::sway::Usable;

/// Requests to the compositor.
pub enum Query {
    Sway(sway::Query),
    Niri(niri::Query),
}

impl Query {
    pub fn connect() -> io::Result<Query> {
        if niri::running() {
            niri::Query::connect().map(Query::Niri)
        } else {
            sway::Query::connect().map(Query::Sway)
        }
    }

    /// The focused output, and whether a fullscreen window covers `output`
    /// (None: the focused one), whose logical size is `size`.
    pub fn outputs(&mut self, output: Option<&str>, size: (u32, u32)) -> io::Result<(Option<String>, bool)> {
        match self {
            Query::Sway(q) => q.outputs(output),
            Query::Niri(q) => q.outputs(output, size),
        }
    }

    pub fn workspaces(&mut self) -> io::Result<Vec<crate::bar::Workspace>> {
        match self {
            Query::Sway(q) => q.workspaces(),
            Query::Niri(q) => q.workspaces(),
        }
    }

    pub fn keyboard_layout(&mut self) -> io::Result<Option<String>> {
        match self {
            Query::Sway(q) => q.keyboard_layout(),
            Query::Niri(q) => q.keyboard_layout(),
        }
    }

    /// Where the free area of `output` starts. niri does not say: Ok(None),
    /// and the island works it out from its own bar reserve.
    pub fn usable(&mut self, output: Option<&str>) -> io::Result<Option<Usable>> {
        match self {
            Query::Sway(q) => q.usable(output),
            Query::Niri(_) => Ok(None),
        }
    }

    pub fn is_niri(&self) -> bool {
        matches!(self, Query::Niri(_))
    }

    /// Shows the workspace the bar labels `name`.
    pub fn focus_workspace(&mut self, name: &str) -> io::Result<()> {
        match self {
            Query::Sway(q) => q.command(&format!("workspace \"{}\"", name.replace('"', "\\\""))),
            Query::Niri(q) => q.focus_workspace(name),
        }
    }

    /// The keyboard's next layout.
    pub fn next_layout(&mut self) -> io::Result<()> {
        match self {
            Query::Sway(q) => q.command("input type:keyboard xkb_switch_layout next"),
            Query::Niri(q) => q.next_layout(),
        }
    }

    /// Ends the session.
    pub fn exit(&mut self) -> io::Result<()> {
        match self {
            Query::Sway(q) => q.command("exit"),
            Query::Niri(q) => q.quit(),
        }
    }
}

/// What a batch of events changed for the island.
pub use crate::niri::Changes;

/// The compositor's events, polled in the event loop.
pub enum Events {
    Sway(sway::Events),
    Niri(niri::Events),
}

impl AsFd for Events {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            Events::Sway(e) => e.as_fd(),
            Events::Niri(e) => e.as_fd(),
        }
    }
}

impl Events {
    /// Subscribes; under niri, `query` answers from the stream from now on.
    pub fn subscribe(query: &mut Query) -> io::Result<Events> {
        match query {
            Query::Sway(_) => sway::Events::subscribe(&["window", "workspace", "input"]).map(Events::Sway),
            Query::Niri(q) => niri::Events::subscribe(q).map(Events::Niri),
        }
    }

    pub fn drain(&mut self) -> io::Result<Changes> {
        match self {
            Events::Sway(e) => Ok(sway_changes(&e.drain()?)),
            Events::Niri(e) => e.drain(),
        }
    }
}

/// sway's events (type, change) in the island's terms.
fn sway_changes(list: &[(u32, String)]) -> Changes {
    Changes {
        windows: list.iter().any(|(_, change)| {
            matches!(change.as_str(), "fullscreen_mode" | "focus" | "close" | "new" | "move")
        }),
        // a workspace event (type 0), or a window opened, closed or moved
        // (type 3): the bar's buttons and the applications on them
        workspaces: list.iter().any(|(kind, change)| {
            *kind == 0 || (*kind == 3 && matches!(change.as_str(), "new" | "close" | "move"))
        }),
        layout: list.iter().any(|(_, change)| change == "xkb_layout"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sway_events_in_the_islands_terms() {
        let changes = sway_changes(&[(3, "title".into()), (4, "xkb_layout".into())]);
        assert_eq!(changes, Changes { layout: true, ..Changes::default() });
        let changes = sway_changes(&[(3, "new".into())]);
        assert_eq!(changes, Changes { windows: true, workspaces: true, layout: false });
        let changes = sway_changes(&[(0, "focus".into())]);
        assert_eq!(changes, Changes { windows: true, workspaces: true, layout: false });
    }
}
