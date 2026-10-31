//! The bar, in the island: what Waybar showed along the top, now two drops
//! of the island's black frame pressed into the top corners — the
//! workspaces in the left one; the details drawer (⋯ with disk, CPU and
//! memory), the volume, the keyboard layout, the battery and the tray in the
//! right one. Every element keeps its place; widths follow their content on
//! springs (Waybar's width-anim.sh, natively), digits roll, the focused
//! workspace's highlight slides between buttons.
//!
//! The bar holds state and draws; it never does I/O. What it shows arrives
//! from the island's workers (`set_*`), what is clicked goes back as an
//! `Action` for the app to carry out.

use std::collections::HashMap;

use cairo::Context;

use crate::anim::{Spring, Time};
use crate::island::{Font, Rect, Renderer};

/// The drops are as tall as the island.
pub const HEIGHT: f64 = 32.0;
/// Their free corner and the fillets where they leave an edge.
pub const RADIUS: f64 = 14.0;
pub const FILLET: f64 = 14.0;
/// Room inside a drop before its first item and after its last (the free
/// end also clears the rounded corner).
const LEAD: f64 = 4.0;
const TAIL: f64 = 10.0;
/// Items: their side padding (Waybar's 9 px; ⋯ 12 px; workspaces 11 px
/// with 1 px margins) and how their highlight sits in the drop.
const PAD: f64 = 9.0;
const PAD_DETAILS: f64 = 12.0;
const WS_PAD: f64 = 11.0;
const WS_MARGIN: f64 = 1.0;
const WASH_INSET: f64 = 5.0;
/// Widths change on this spring (no bounce: neighbours glide).
const WIDTH_SPRING: (f64, f64) = (0.3, 1.0);
/// The tray's icons.
pub const TRAY_ICON: f64 = 18.0;
const TRAY_SPACING: f64 = 6.0;

/// SF Symbols (private-use glyphs of SF Pro, as the menu bar draws them).
const ELLIPSIS: &str = "\u{100360}";
const DISK: &str = "\u{1009D8}";
const CPU: &str = "\u{1009D3}";
const MEMORY: &str = "\u{1009D6}";
/// How strongly a switch of workspace and an opening tray menu ripple the
/// frame's edge (a tugged island's full ripple is 1).
const SPLASH_WORKSPACE: f64 = 0.35;
const SPLASH_MENU: f64 = 0.3;
/// The drawer's graphs: how many readings (one a second), their size, the
/// gap before them, and their colours (CPU cyan, memory purple).
const HISTORY: usize = 30;
const GRAPH_W: f64 = 32.0;
const GRAPH_H: f64 = 14.0;
const GRAPH_GAP: f64 = 6.0;
const GRAPH_CPU: (f64, f64, f64) = (0x66 as f64 / 255.0, 0xD9 as f64 / 255.0, 1.0);
const GRAPH_MEMORY: (f64, f64, f64) = (0xBF as f64 / 255.0, 0x5A as f64 / 255.0, 0xF2 as f64 / 255.0);
/// A workspace's application icons: their size, the gap after its name,
/// between them, and how many at most.
pub const APP_ICON: f64 = 15.0;
const APP_ICON_GAP: f64 = 6.0;
const APP_ICON_SPACING: f64 = 3.0;
const APP_ICONS_MAX: usize = 4;
/// Between a drawn symbol and its text.
const SYMBOL_GAP: f64 = 5.0;

const MUTED: (f64, f64, f64) = (0x8F as f64 / 255.0, 0xCD as f64 / 255.0, 0xEF as f64 / 255.0);
/// Urgent workspaces.
const CRITICAL: (f64, f64, f64) = (1.0, 0x8C as f64 / 255.0, 0x42 as f64 / 255.0);

/// A workspace as sway reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workspace {
    pub name: String,
    pub focused: bool,
    pub urgent: bool,
    pub output: String,
    /// its windows' applications (app_id or class), each once
    pub apps: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    /// minutes to empty (or to full when charging)
    pub remaining: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub disk_free: String,
    pub cpu: u8,
    pub memory_gb: f64,
    pub memory_total_gb: f64,
}

/// A tray icon, decoded by a worker.
#[derive(Clone)]
pub struct TrayIcon {
    pub id: String,
    pub icon: Option<cairo::ImageSurface>,
    /// it has a menu of its own (dbusmenu), and a click opens it
    pub has_menu: bool,
    pub item_is_menu: bool,
    /// for its tooltip (with the tray's menus)
    #[allow(dead_code)]
    pub title: String,
}

/// What can be pointed at in the bar.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Workspace(String),
    Details,
    Disk,
    Cpu,
    Memory,
    Volume,
    Layout,
    Battery,
    Tray(String),
}

/// What a click or scroll on the bar asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Workspace(String),
    /// the next (+1) or previous (-1) workspace on this output
    CycleWorkspace(i32),
    ToggleDrawer,
    ToggleMute,
    /// a volume step, +1 or -1 (5 %)
    VolumeStep(i32),
    ToggleKeyboard,
    /// a tray item: activate, the secondary activation, or its menu, at
    /// the item's place on the surface
    TrayActivate(String, f64, f64),
    TraySecondary(String, f64, f64),
    TrayMenu(String, f64, f64),
    TrayScroll(String, i32),
    /// an entry of a tray item's menu was chosen
    MenuChoose(String, i32),
}

/// A tray item's menu, open as a sheet hanging from the right drop.
struct Menu {
    id: String,
    /// the pages: the menu, then each submenu opened from it
    pages: Vec<Vec<crate::tray::MenuEntry>>,
    opened: Time,
    grow: Spring,
    hovered: Option<usize>,
    /// where it was drawn, and its rows
    rect: Option<Rect>,
    rows: Vec<(Rect, Row)>,
    closing: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Back,
    Entry(usize),
}

const MENU_ROW: f64 = 32.0;
const MENU_SEPARATOR: f64 = 9.0;
const MENU_PAD: f64 = 6.0;
const MENU_MIN_W: f64 = 190.0;
const MENU_MAX_W: f64 = 320.0;
const MENU_R: f64 = 18.0;

pub struct Bar {
    pub workspaces: Vec<Workspace>,
    pub volume: Option<(f64, bool)>,
    pub layout: Option<String>,
    pub battery: Option<Battery>,
    pub details: Option<Details>,
    pub tray: Vec<TrayIcon>,
    /// the output it is on (whose workspaces it shows)
    pub output: Option<String>,
    drawer: bool,
    widths: HashMap<Item, Spring>,
    /// the focused workspace's highlight: where and how wide
    focus_x: Option<Spring>,
    focus_w: Option<Spring>,
    hovered: Option<Item>,
    washes: HashMap<Item, Spring>,
    /// drawn up into the edge (a fullscreen window): 0 shown .. 1 gone
    hide: Spring,
    /// where each item was drawn last, for hit testing
    slots: Vec<(Item, Rect)>,
    drops: [Option<Rect>; 2],
    menu: Option<Menu>,
    /// the speaker's, keyboard's and battery's symbols
    glyphs: crate::bar_icons::Glyphs,
    /// the icons of the workspaces' applications, by app id, at the
    /// output's scale (None: still loading or none to be had)
    pub app_icons: HashMap<String, Option<cairo::ImageSurface>>,
    /// the drawer's readings since it opened (CPU, memory; 0..1), and the
    /// graphs sliding left as each new one comes in
    history: std::collections::VecDeque<(f64, f64)>,
    history_scroll: Spring,
    /// something moved in a drop that should ripple the frame's edge: from
    /// where along it (x) and how strongly
    splash: Option<(f64, f64)>,
    /// the focused workspace as last drawn
    focused: Option<String>,
    /// a drop in the hand, and how far each one is stretched down as it
    /// jumps back
    tug: Option<DropTug>,
    stretch: [Spring; 2],
}

/// A drop held by the pointer: pulled down against a rubber band.
struct DropTug {
    side: usize,
    from: (f64, f64),
    engaged: bool,
    pull: f64,
    velocity: f64,
    at: Time,
}

/// How far a drop gives (it never reaches this), the pointer's travel
/// before a press becomes a pull, and the jelly it springs back on.
const TUG_DOWN: f64 = 26.0;
const TUG_SLOP: f64 = 6.0;
const TUG_RESPONSE: f64 = 0.46;
const TUG_DAMPING: f64 = 0.42;

impl Default for Bar {
    fn default() -> Self {
        Bar {
            workspaces: Vec::new(),
            volume: None,
            layout: None,
            battery: None,
            details: None,
            tray: Vec::new(),
            output: None,
            drawer: false,
            widths: HashMap::new(),
            focus_x: None,
            focus_w: None,
            hovered: None,
            washes: HashMap::new(),
            hide: Spring::new(0.0, 0.3, 1.0),
            slots: Vec::new(),
            drops: [None, None],
            menu: None,
            glyphs: Default::default(),
            app_icons: HashMap::new(),
            history: std::collections::VecDeque::new(),
            history_scroll: Spring::new(0.0, 0.35, 1.0),
            splash: None,
            focused: None,
            tug: None,
            stretch: [0; 2].map(|_| Spring::new(0.0, TUG_RESPONSE, TUG_DAMPING)),
        }
    }
}

/// What a frame of the bar covered and what takes the pointer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarFrame {
    pub painted: Vec<Rect>,
    pub input: Vec<Rect>,
}

/// One element laid out in a row: what it is, its full (target) width, and
/// how to paint it in its slot.
struct Cell {
    item: Item,
    width: f64,
}

impl Bar {
    /// The drops as last drawn (left, right): how far out and how tall.
    pub fn drop_sizes(&self) -> [Option<crate::outline::Drop>; 2] {
        self.drops.map(|d| d.filter(|r| r.w > 1.0 && r.h > 1.0).map(|r| crate::outline::Drop { w: r.w, h: r.h }))
    }

    pub fn drawer_open(&self) -> bool {
        self.drawer
    }

    pub fn set_drawer(&mut self, open: bool) {
        // the graphs start afresh each time it opens
        if open && !self.drawer {
            self.history.clear();
        }
        self.drawer = open;
    }

    /// Hide into the edge (a fullscreen window) or come back.
    pub fn set_hidden(&mut self, hidden: bool, now: Time) {
        self.hide.set(if hidden { 1.0 } else { 0.0 }, now);
    }

    /// A tray item's menu arrived: open it as a sheet under the item.
    /// A reading of the drawer's sampler: shown, and added to its graphs.
    pub fn push_details(&mut self, details: Details, now: Time) {
        let memory = if details.memory_total_gb > 0.0 { details.memory_gb / details.memory_total_gb } else { 0.0 };
        self.history.push_back((details.cpu as f64 / 100.0, memory.clamp(0.0, 1.0)));
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.history_scroll = Spring::new(1.0, 0.35, 1.0);
        self.history_scroll.set(0.0, now);
        self.details = Some(details);
    }

    /// A ripple the bar asks for since it was last asked: (x, strength).
    pub fn take_splash(&mut self) -> Option<(f64, f64)> {
        self.splash.take()
    }

    pub fn open_menu(&mut self, id: String, entries: Vec<crate::tray::MenuEntry>, now: Time) {
        // the menu drops out of the item: a light ripple runs from there
        if self.menu.is_none() {
            if let Some((_, r)) = self.slots.iter().find(|(i, _)| *i == Item::Tray(id.clone())) {
                self.splash = Some((r.x + r.w / 2.0, SPLASH_MENU));
            }
        }
        if entries.is_empty() {
            return;
        }
        let mut grow = Spring::new(0.0, 0.34, 0.78);
        grow.set(1.0, now);
        self.menu = Some(Menu { id, pages: vec![entries], opened: now, grow, hovered: None, rect: None, rows: Vec::new(), closing: false });
    }

    pub fn close_menu(&mut self, now: Time) {
        if let Some(m) = self.menu.as_mut() {
            m.closing = true;
            m.grow.set_with(0.0, 0.22, 1.0, now);
        }
    }

    /// Is a menu up (for the outside click that closes it)?
    pub fn menu_open(&self) -> bool {
        self.menu.as_ref().is_some_and(|m| !m.closing)
    }

    /// A row of the open menu chosen (`keyed`: from the keyboard, which lights
    /// the first row of a submenu it enters).
    fn choose_row(&mut self, row: Row, keyed: bool, now: Time) -> Option<Action> {
        let m = self.menu.as_mut()?;
        match row {
            Row::Back => {
                m.pages.pop();
                m.hovered = if keyed { first_row(m) } else { None };
                None
            }
            Row::Entry(i) => {
                let entry = m.pages.last()?.get(i)?.clone();
                if entry.separator || !entry.enabled {
                    return None;
                }
                if !entry.children.is_empty() {
                    m.pages.push(entry.children);
                    m.hovered = if keyed { first_row(m) } else { None };
                    return None;
                }
                let id = m.id.clone();
                self.close_menu(now);
                Some(Action::MenuChoose(id, entry.id))
            }
        }
    }

    /// A key while a menu is up: "next" / "prev" move the lit row,
    /// "enter" chooses it (or enters its submenu), "back" leaves a submenu,
    /// "escape" closes. Returns whether anything changed, and what was
    /// chosen.
    pub fn menu_key(&mut self, key: &str, now: Time) -> (bool, Option<Action>) {
        let Some(m) = self.menu.as_mut().filter(|m| !m.closing) else { return (false, None) };
        match key {
            "next" | "prev" => {
                let rows = selectable_rows(m);
                if rows.is_empty() {
                    return (false, None);
                }
                let at = m.hovered.and_then(|h| rows.iter().position(|r| *r == h));
                let n = rows.len();
                let next = match (at, key) {
                    (None, "next") => 0,
                    (None, _) => n - 1,
                    (Some(i), "next") => (i + 1) % n,
                    (Some(i), _) => (i + n - 1) % n,
                };
                m.hovered = Some(rows[next]);
                (true, None)
            }
            "enter" => {
                let Some(row) = m.hovered.and_then(|h| row_at(m, h)) else { return (false, None) };
                let action = self.choose_row(row, true, now);
                (true, action)
            }
            "back" if m.pages.len() > 1 => (true, self.choose_row(Row::Back, true, now)),
            "back" | "escape" => {
                self.close_menu(now);
                (true, None)
            }
            _ => (false, None),
        }
    }

    /// The pointer over the menu: which row is lit.
    pub fn menu_hover(&mut self, x: f64, y: f64) -> bool {
        let Some(m) = self.menu.as_mut() else { return false };
        let row = m.rows.iter().position(|(r, row)| inside(*r, x, y) && match row {
            Row::Entry(i) => m.pages.last().and_then(|p| p.get(*i)).is_some_and(|e| e.enabled && !e.separator),
            Row::Back => true,
        });
        let changed = row != m.hovered;
        m.hovered = row;
        changed
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.menu.as_ref().is_some_and(|m| m.grow.is_moving(now) || now - m.opened < 0.4)
            || self.hide.is_moving(now)
            || self.widths.values().any(|s| s.is_moving(now))
            || self.focus_x.as_ref().is_some_and(|s| s.is_moving(now))
            || self.focus_w.as_ref().is_some_and(|s| s.is_moving(now))
            || self.washes.values().any(|s| s.is_moving(now))
            || self.glyphs.is_animating(now)
            || (self.drawer && self.history_scroll.is_moving(now))
            || self.tug.is_some()
            || self.stretch.iter().any(|s| s.is_moving(now))
    }

    /// Take a drop in the hand (a left press on it, no menu open). What
    /// the press means waits: a pull, or a click on release.
    pub fn tug_press(&mut self, x: f64, y: f64, now: Time) -> bool {
        if self.menu.is_some() {
            return false;
        }
        let Some(side) = self.drops.iter().position(|d| d.is_some_and(|d| inside(Rect { h: HEIGHT, ..d }, x, y))) else { return false };
        self.tug = Some(DropTug { side, from: (x, y), engaged: false, pull: 0.0, velocity: 0.0, at: now });
        true
    }

    /// Whether a drop is held (the pointer belongs to the bar until let go).
    pub fn tugging(&self) -> bool {
        self.tug.is_some()
    }

    /// The held drop follows the pointer down. Returns whether it is a pull.
    pub fn tug_to(&mut self, x: f64, y: f64, now: Time) -> bool {
        let Some(t) = self.tug.as_mut() else { return false };
        let dy = y - t.from.1;
        if !t.engaged {
            if (x - t.from.0).hypot(dy) < TUG_SLOP {
                return false;
            }
            t.engaged = true;
        }
        let pull = crate::island::rubber(dy.max(0.0), TUG_DOWN);
        let dt = now - t.at;
        if dt > 0.004 {
            t.velocity = 0.5 * t.velocity + 0.5 * (pull - t.pull) / dt;
            t.at = now;
        }
        t.pull = pull;
        true
    }

    /// Let go: a pulled drop jumps back and ripples the edge from under
    /// it. Returns whether it was a pull (false: a click after all).
    pub fn tug_release(&mut self, now: Time) -> bool {
        let Some(t) = self.tug.take() else { return false };
        if !t.engaged {
            return false;
        }
        let v = if now - t.at < 0.08 { t.velocity.clamp(-1500.0, 1500.0) } else { 0.0 };
        self.stretch[t.side].release(t.pull, v, 0.0, now);
        if let Some(d) = self.drops[t.side] {
            self.splash = Some((d.x + d.w / 2.0, (t.pull / TUG_DOWN).min(1.0)));
        }
        true
    }

    /// How far a drop is stretched down now.
    fn stretched(&self, side: usize, now: Time) -> f64 {
        match &self.tug {
            Some(t) if t.engaged && t.side == side => t.pull,
            _ => self.stretch[side].value(now).max(-4.0),
        }
    }

    /// The item under a point of the surface, if the bar is there.
    pub fn item_at(&self, x: f64, y: f64) -> Option<Item> {
        self.slots.iter().find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(i, _)| i.clone())
    }

    /// Is the point over one of the drops (or the menu hanging from one)?
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.drops.iter().flatten().any(|r| inside(*r, x, y))
            || self.menu.as_ref().and_then(|m| m.rect).is_some_and(|r| inside(r, x, y))
    }

    /// The pointer moved over the bar (None: it left).
    pub fn hover(&mut self, item: Option<Item>, now: Time) -> bool {
        if self.hovered == item {
            return false;
        }
        if let Some(old) = self.hovered.take() {
            if let Some(s) = self.washes.get_mut(&old) { s.set(0.0, now); }
        }
        if let Some(new) = &item {
            self.washes.entry(new.clone()).or_insert_with(|| Spring::new(0.0, 0.18, 1.0)).set(1.0, now);
        }
        self.hovered = item;
        true
    }

    /// A press on the bar: what it asks for. `button`: 0x110 left, 0x111
    /// right, 0x112 middle.
    pub fn press(&mut self, x: f64, y: f64, button: u32, now: Time) -> Option<Action> {
        // the menu first: a row chosen, a submenu entered, the way back
        if let Some(m) = self.menu.as_mut().filter(|m| !m.closing) {
            if m.rect.is_some_and(|r| inside(r, x, y)) {
                let row = m.rows.iter().find(|(r, _)| inside(*r, x, y)).map(|(_, row)| row.clone());
                return row.and_then(|row| self.choose_row(row, false, now));
            }
            // anywhere else on the bar: the menu folds first
            self.close_menu(now);
        }
        let item = self.item_at(x, y)?;
        let (cx, cy) = self.slots.iter().find(|(i, _)| *i == item).map(|(_, r)| (r.x + r.w / 2.0, r.y + r.h)).unwrap_or((x, y));
        Some(match (item, button) {
            (Item::Workspace(name), _) => Action::Workspace(name),
            (Item::Details | Item::Disk | Item::Cpu | Item::Memory, _) => Action::ToggleDrawer,
            (Item::Volume, _) => Action::ToggleMute,
            (Item::Layout, _) => Action::ToggleKeyboard,
            (Item::Battery, _) => return None,
            (Item::Tray(id), 0x111) => Action::TrayMenu(id, cx, cy),
            (Item::Tray(id), 0x112) => Action::TraySecondary(id, cx, cy),
            (Item::Tray(id), _) => Action::TrayActivate(id, cx, cy),
        })
    }

    /// A scroll on the bar (`dy` > 0: down).
    pub fn scroll(&self, x: f64, y: f64, dy: f64) -> Option<Action> {
        match self.item_at(x, y)? {
            Item::Workspace(_) => Some(Action::CycleWorkspace(if dy > 0.0 { 1 } else { -1 })),
            Item::Volume => Some(Action::VolumeStep(if dy > 0.0 { -1 } else { 1 })),
            Item::Tray(id) => Some(Action::TrayScroll(id, if dy > 0.0 { 1 } else { -1 })),
            _ => None,
        }
    }

    fn my_workspaces(&self) -> impl Iterator<Item = &Workspace> {
        let output = self.output.clone();
        self.workspaces.iter().filter(move |w| output.as_ref().is_none_or(|o| &w.output == o))
    }

    /// The workspace after (or before) the focused one on this output.
    pub fn cycled(&self, step: i32) -> Option<String> {
        let mine: Vec<&Workspace> = self.my_workspaces().collect();
        let at = mine.iter().position(|w| w.focused)? as i32;
        let n = mine.len() as i32;
        mine.get((at + step).rem_euclid(n) as usize).map(|w| w.name.clone())
    }

    /// The right drop's cells, left to right.
    fn right_cells(&self, cr: &Context, renderer: &Renderer) -> Vec<(Cell, Option<String>)> {
        let text_w = |s: &str| renderer.layout(cr, s, Font::Bar).pixel_extents().1.width() as f64;
        let mut cells = Vec::new();
        // the drawer opens to the left of ⋯, which stays by the volume
        if let Some(d) = &self.details {
            for (item, text) in [
                (Item::Disk, format!("{DISK} {}", d.disk_free)),
                (Item::Cpu, format!("{CPU} {}%", d.cpu)),
                (Item::Memory, format!("{MEMORY} {:.1}GB", d.memory_gb)),
            ] {
                let graph = if item == Item::Disk { 0.0 } else { GRAPH_GAP + GRAPH_W };
                let width = if self.drawer { text_w(&text) + graph + 2.0 * PAD } else { 0.0 };
                cells.push((Cell { item, width }, Some(text)));
            }
        }
        cells.push((Cell { item: Item::Details, width: text_w(ELLIPSIS) + 2.0 * PAD_DETAILS }, Some(ELLIPSIS.to_string())));
        // the drawn symbols, their text after them
        if let Some((value, _)) = self.volume {
            let text = format!("{}%", (value * 100.0).round() as u32);
            let width = crate::bar_icons::SPEAKER_W + SYMBOL_GAP + text_w(&text) + 2.0 * PAD;
            cells.push((Cell { item: Item::Volume, width }, Some(text)));
        }
        if let Some(layout) = &self.layout {
            let width = crate::bar_icons::KEYBOARD_W + SYMBOL_GAP + text_w(layout) + 2.0 * PAD;
            cells.push((Cell { item: Item::Layout, width }, Some(layout.clone())));
        }
        if self.battery.is_some() {
            let width = self.glyphs.battery_target_w() + 2.0 * PAD;
            cells.push((Cell { item: Item::Battery, width }, Some(String::new())));
        }
        for icon in &self.tray {
            cells.push((Cell { item: Item::Tray(icon.id.clone()), width: TRAY_ICON + TRAY_SPACING }, None));
        }
        if !self.tray.is_empty() {
            // the tray module's own padding round its icons
            if let Some(last) = cells.last_mut() { last.0.width += 2.0 * PAD - TRAY_SPACING; }
        }
        cells
    }

    /// The icons shown after a workspace's name (only ones that loaded).
    fn shown_apps<'a>(&'a self, w: &'a Workspace) -> impl Iterator<Item = &'a cairo::ImageSurface> {
        w.apps.iter().filter_map(|a| self.app_icons.get(a).and_then(Option::as_ref)).take(APP_ICONS_MAX)
    }

    /// The room a workspace's icons take after its name.
    fn apps_w(&self, w: &Workspace) -> f64 {
        let n = self.shown_apps(w).count();
        if n == 0 { 0.0 } else { APP_ICON_GAP + n as f64 * APP_ICON + (n - 1) as f64 * APP_ICON_SPACING }
    }

    /// The left drop's cells: the workspaces.
    fn left_cells(&self, cr: &Context, renderer: &Renderer) -> Vec<(Cell, Option<String>)> {
        self.my_workspaces().map(|w| {
            let font = if w.focused { Font::BarBold } else { Font::Bar };
            let width = renderer.layout(cr, &w.name, font).pixel_extents().1.width() as f64 + self.apps_w(w) + 2.0 * WS_PAD + 2.0 * WS_MARGIN;
            (Cell { item: Item::Workspace(w.name.clone()), width }, Some(w.name.clone()))
        }).collect()
    }

    /// Each cell's animated width now (a new one grows in from nothing).
    fn animate(&mut self, cells: &[(Cell, Option<String>)], now: Time) -> Vec<f64> {
        cells.iter().map(|(cell, _)| {
            let spring = self.widths.entry(cell.item.clone()).or_insert_with(|| Spring::new(0.0, WIDTH_SPRING.0, WIDTH_SPRING.1));
            spring.set(cell.width, now);
            spring.value(now).max(0.0)
        }).collect()
    }

    /// Paint both drops on a surface `w` wide.
    pub fn draw(&mut self, cr: &Context, renderer: &Renderer, w: f64, smoothing: f64, now: Time) -> BarFrame {
        if let Some((value, muted)) = self.volume {
            self.glyphs.volume(value, muted, now);
        }
        if let Some(b) = self.battery {
            self.glyphs.battery(b.percent, b.charging, now);
        }
        let left = self.left_cells(cr, renderer);
        let right = self.right_cells(cr, renderer);
        let left_w = self.animate(&left, now);
        let right_w = self.animate(&right, now);
        // forget the springs of what is gone
        let present: Vec<Item> = left.iter().chain(&right).map(|(c, _)| c.item.clone()).collect();
        self.widths.retain(|k, _| present.contains(k));
        self.washes.retain(|k, s| present.contains(k) && (s.target() > 0.0 || s.value(now) > 0.001));

        let lift = HEIGHT * self.hide.value(now).clamp(0.0, 1.0);
        let mut frame = BarFrame::default();
        self.slots.clear();
        self.drops = [None, None];
        if lift >= HEIGHT + FILLET {
            return frame;
        }
        let y = -lift;
        cr.save().unwrap();
        cr.translate(0.0, y);

        // the left drop: the workspaces
        let content: f64 = left_w.iter().sum();
        if content > 0.5 {
            let dw = LEAD + content + TAIL;
            // pulled down it stretches, its buttons riding half way
            let pull = self.stretched(0, now);
            crate::drop::corner(cr, w, false, dw, HEIGHT + pull, RADIUS, FILLET, smoothing);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            let _ = cr.fill();
            self.drops[0] = Some(Rect { x: 0.0, y: 0.0, w: dw, h: HEIGHT + pull + y });
            cr.save().unwrap();
            cr.translate(0.0, pull / 2.0);
            let mut x = LEAD;
            let mut focus = None;
            for ((cell, text), slot_w) in left.iter().zip(&left_w) {
                let ws = self.my_workspaces().find(|ws| Item::Workspace(ws.name.clone()) == cell.item).cloned();
                let slot = Rect { x: x + WS_MARGIN, y: WASH_INSET - 1.0, w: (slot_w - 2.0 * WS_MARGIN).max(0.0), h: HEIGHT - 2.0 * (WASH_INSET - 1.0) };
                if ws.as_ref().is_some_and(|w| w.focused) {
                    focus = Some(slot);
                }
                self.wash(cr, &cell.item, slot, now);
                if let (Some(text), Some(ws)) = (text, ws) {
                    let font = if ws.focused { Font::BarBold } else { Font::Bar };
                    let colour = if ws.urgent { CRITICAL } else { (1.0, 1.0, 1.0) };
                    let alpha = if ws.focused || ws.urgent { 1.0 } else { 0.6 };
                    // urgent: an orange wash under it
                    if ws.urgent {
                        rounded(cr, slot, 4.0);
                        cr.set_source_rgba(CRITICAL.0, CRITICAL.1, CRITICAL.2, 0.18);
                        let _ = cr.fill();
                    }
                    // the name, then its applications' icons
                    let icons: Vec<cairo::ImageSurface> = self.shown_apps(&ws).cloned().collect();
                    let apps_w = self.apps_w(&ws);
                    clipped(cr, Rect { x, y: 0.0, w: *slot_w, h: HEIGHT }, || {
                        let tw = renderer.layout(cr, text, font).pixel_extents().1.width() as f64;
                        let start = x + (slot_w - tw - apps_w) / 2.0;
                        centred_text(cr, renderer, text, font, start + tw / 2.0, colour, alpha);
                        let mut ix = start + tw + APP_ICON_GAP;
                        for icon in &icons {
                            cr.save().unwrap();
                            cr.translate(renderer.snap(ix), renderer.snap((HEIGHT - APP_ICON) / 2.0));
                            cr.scale(APP_ICON / icon.width() as f64, APP_ICON / icon.height() as f64);
                            let _ = cr.set_source_surface(icon, 0.0, 0.0);
                            let _ = cr.paint_with_alpha(if ws.focused { 1.0 } else { 0.7 });
                            cr.restore().unwrap();
                            ix += APP_ICON + APP_ICON_SPACING;
                        }
                    });
                }
                self.slots.push((cell.item.clone(), Rect { x, y: y.max(0.0), w: *slot_w, h: HEIGHT + y }));
                x += slot_w;
            }
            // the focused workspace's highlight slides from button to button
            if let Some(f) = focus {
                let fx = self.focus_x.get_or_insert_with(|| Spring::new(f.x, 0.32, 0.82));
                fx.set(f.x, now);
                // another workspace: the pill's arrival ripples the edge
                let name = self.workspaces.iter().find(|w| w.focused).map(|w| w.name.clone());
                if self.focused.is_some() && name != self.focused {
                    self.splash = Some((f.x + f.w / 2.0, SPLASH_WORKSPACE));
                }
                self.focused = name;
                let fw = self.focus_w.get_or_insert_with(|| Spring::new(f.w, 0.32, 0.82));
                fw.set(f.w, now);
                let r = Rect { x: fx.value(now), y: f.y, w: fw.value(now).max(0.0), h: f.h };
                rounded(cr, r, 4.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
                let _ = cr.fill();
            }
            cr.restore().unwrap();
            frame.painted.push(Rect { x: 0.0, y: 0.0, w: dw + FILLET * 5.0, h: (HEIGHT + TUG_DOWN + FILLET * 5.0 + y).max(0.0) });
            frame.input.push(Rect { x: 0.0, y: 0.0, w: dw, h: (HEIGHT + pull.max(0.0) + y).max(0.0) });
        }

        // the right drop: details, volume, layout, battery, tray
        let content: f64 = right_w.iter().sum();
        if content > 0.5 {
            let dw = TAIL + content + LEAD;
            let pull = self.stretched(1, now);
            crate::drop::corner(cr, w, true, dw, HEIGHT + pull, RADIUS, FILLET, smoothing);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            let _ = cr.fill();
            self.drops[1] = Some(Rect { x: w - dw, y: 0.0, w: dw, h: HEIGHT + pull + y });
            cr.save().unwrap();
            cr.translate(0.0, pull / 2.0);
            let mut x = w - LEAD - content;
            for ((cell, text), slot_w) in right.iter().zip(&right_w) {
                let slot = Rect { x, y: WASH_INSET - 1.0, w: *slot_w, h: HEIGHT - 2.0 * (WASH_INSET - 1.0) };
                self.wash(cr, &cell.item, slot, now);
                clipped(cr, Rect { x, y: 0.0, w: *slot_w, h: HEIGHT }, || match (&cell.item, text) {
                    (Item::Tray(id), _) => {
                        if let Some(icon) = self.tray.iter().find(|t| &t.id == id).and_then(|t| t.icon.clone()) {
                            let (ix, iy) = (x + (slot_w - TRAY_ICON) / 2.0, (HEIGHT - TRAY_ICON) / 2.0);
                            cr.save().unwrap();
                            cr.translate(renderer.snap(ix), renderer.snap(iy));
                            cr.scale(TRAY_ICON / icon.width() as f64, TRAY_ICON / icon.height() as f64);
                            let _ = cr.set_source_surface(&icon, 0.0, 0.0);
                            let _ = cr.paint();
                            cr.restore().unwrap();
                        }
                    }
                    (item, Some(text)) => {
                        let pad = if *item == Item::Details { PAD_DETAILS } else { PAD };
                        let colour = self.colour_of(item);
                        // a drawn symbol first, then the text after it
                        let cy = HEIGHT / 2.0;
                        let symbol = match item {
                            Item::Volume => {
                                self.glyphs.draw_speaker(cr, x + pad, cy, colour, 1.0, now);
                                crate::bar_icons::SPEAKER_W + SYMBOL_GAP
                            }
                            Item::Layout => {
                                crate::bar_icons::draw_keyboard(cr, renderer, x + pad, cy, colour, 1.0);
                                crate::bar_icons::KEYBOARD_W + SYMBOL_GAP
                            }
                            Item::Battery => {
                                if let Some(b) = self.battery {
                                    self.glyphs.draw_battery(cr, renderer, x + pad, cy, b.percent, 1.0, now);
                                }
                                0.0
                            }
                            _ => 0.0,
                        };
                        // the CPU's and memory's last half minute
                        if self.drawer && matches!(item, Item::Cpu | Item::Memory) {
                            let (colour, pick): ((f64, f64, f64), fn(&(f64, f64)) -> f64) = if *item == Item::Cpu {
                                (GRAPH_CPU, |s| s.0)
                            } else {
                                (GRAPH_MEMORY, |s| s.1)
                            };
                            let values: Vec<f64> = self.history.iter().map(pick).collect();
                            let r = Rect { x: x + slot_w - PAD - GRAPH_W, y: (HEIGHT - GRAPH_H) / 2.0, w: GRAPH_W, h: GRAPH_H };
                            graph(cr, r, &values, self.history_scroll.value(now).clamp(0.0, 1.0), colour);
                        }
                        if !text.is_empty() {
                            let key = format!("bar {item:?}");
                            let lh = renderer.layout(cr, text, Font::Bar).pixel_extents().1;
                            crate::motion::text(cr, renderer, &key, text, x + pad + symbol, (HEIGHT - lh.height() as f64) / 2.0 - lh.y() as f64,
                                (slot_w - pad - symbol).max(4.0) + 40.0, 0.0, Font::Bar, colour, 1.0);
                        }
                    }
                    _ => {}
                });
                self.slots.push((cell.item.clone(), Rect { x, y: y.max(0.0), w: *slot_w, h: HEIGHT + y }));
                x += slot_w;
            }
            cr.restore().unwrap();
            frame.painted.push(Rect { x: w - dw - FILLET * 5.0, y: 0.0, w: dw + FILLET * 5.0, h: (HEIGHT + TUG_DOWN + FILLET * 5.0 + y).max(0.0) });
            frame.input.push(Rect { x: w - dw, y: 0.0, w: dw, h: (HEIGHT + pull.max(0.0) + y).max(0.0) });
            // the battery's time, in a small pill under it while it is hovered
            let k = self.washes.get(&Item::Battery).map_or(0.0, |s| s.value(now).clamp(0.0, 1.0));
            if let (true, Some(b), Some((_, slot))) = (k > 0.01, self.battery, self.slots.iter().find(|(i, _)| *i == Item::Battery)) {
                let text = battery_tip(b);
                let layout = renderer.layout(cr, &text, Font::Small);
                let (_, ext) = layout.pixel_extents();
                let (tw, th) = (ext.width() as f64 + 24.0, 26.0);
                let tx = (slot.x + slot.w / 2.0 - tw / 2.0).min(w - tw - 6.0);
                let ty = HEIGHT + 8.0 - 4.0 * (1.0 - k);
                let tip = Rect { x: tx, y: ty, w: tw, h: th };
                rounded(cr, tip, th / 2.0);
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.92 * k);
                let _ = cr.fill();
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.9 * k);
                cr.move_to(renderer.snap(tx + 12.0 - ext.x() as f64), renderer.snap(ty + (th - ext.height() as f64) / 2.0 - ext.y() as f64));
                pangocairo::functions::show_layout(cr, &layout);
                frame.painted.push(Rect { x: tx - 2.0, y: 0.0, w: tw + 4.0, h: ty + th + 2.0 });
            }
        }
        self.draw_menu(cr, renderer, w, smoothing, now, &mut frame);
        cr.restore().unwrap();
        frame.input.retain(|r| r.h > 0.5);
        frame
    }

    /// The tray item's menu: a sheet hanging from the right drop's lower
    /// edge under the item, fillets where it leaves it, as the island hangs
    /// from the screen's edge.
    fn draw_menu(&mut self, cr: &Context, renderer: &Renderer, w: f64, smoothing: f64, now: Time, frame: &mut BarFrame) {
        let Some(m) = self.menu.as_mut() else { return };
        let k = m.grow.value(now).max(0.0);
        if m.closing && k <= 0.01 {
            self.menu = None;
            return;
        }
        let page = m.pages.last().cloned().unwrap_or_default();
        let back = m.pages.len() > 1;
        let label_w = page.iter().map(|e| renderer.layout(cr, &e.label, Font::Body).pixel_extents().1.width() as f64).fold(0.0, f64::max);
        let mw = (label_w + 70.0).clamp(MENU_MIN_W, MENU_MAX_W);
        let content: f64 = if back { MENU_ROW } else { 0.0 }
            + page.iter().map(|e| if e.separator { MENU_SEPARATOR } else { MENU_ROW }).sum::<f64>();
        let full_h = (content + 2.0 * MENU_PAD).min(520.0);
        // under the item, inside the drop's span
        let anchor = self.slots.iter().find(|(i, _)| *i == Item::Tray(m.id.clone())).map(|(_, r)| r.x + r.w / 2.0).unwrap_or(w - 40.0);
        let drop_x = self.drops[1].map_or(w - mw, |d| d.x);
        let x1 = (anchor + mw / 2.0).min(w - 2.0);
        let x0 = (x1 - mw).max(drop_x + RADIUS + FILLET);
        let x1 = x0 + mw;
        let y0 = HEIGHT;
        let h = full_h * k;
        crate::drop::notch(cr, x0, x1, y0, y0 + h.max(0.5), MENU_R.min(h / 2.0).max(0.0), FILLET * k.min(1.0), smoothing);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.fill();
        let rect = Rect { x: x0, y: y0, w: mw, h };
        m.rect = Some(rect);
        frame.painted.push(Rect { x: x0 - FILLET - 2.0, y: 0.0, w: mw + 2.0 * FILLET + 4.0, h: y0 + full_h * 1.1 + 4.0 });
        frame.input.push(rect);
        // the rows come in once it has most of its height
        let alpha = ((k - 0.5) / 0.4).clamp(0.0, 1.0);
        m.rows.clear();
        let mut y = y0 + MENU_PAD;
        let mut rows: Vec<(Row, f64)> = Vec::new();
        if back { rows.push((Row::Back, MENU_ROW)); }
        for (i, e) in page.iter().enumerate() {
            rows.push((Row::Entry(i), if e.separator { MENU_SEPARATOR } else { MENU_ROW }));
        }
        cr.save().unwrap();
        cr.rectangle(x0, y0, mw, h);
        cr.clip();
        for (n, (row, rh)) in rows.into_iter().enumerate() {
            let r = Rect { x: x0 + 6.0, y, w: mw - 12.0, h: rh };
            if alpha > 0.002 {
                if m.hovered == Some(n) {
                    rounded(cr, r, 9.0);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.12 * alpha);
                    let _ = cr.fill();
                }
                match &row {
                    Row::Back => menu_label(cr, renderer, "‹ Назад", r, true, alpha * 0.7),
                    Row::Entry(i) => {
                        let e = &page[*i];
                        if e.separator {
                            cr.rectangle(r.x + 8.0, r.y + rh / 2.0 - 0.5, r.w - 16.0, 1.0);
                            cr.set_source_rgba(1.0, 1.0, 1.0, 0.12 * alpha);
                            let _ = cr.fill();
                        } else {
                            menu_label(cr, renderer, &e.label, r, e.enabled, alpha);
                            if e.toggle == Some(true) {
                                // a check mark before it
                                let (cx, cy) = (r.x + 14.0, r.y + rh / 2.0);
                                cr.new_path();
                                cr.move_to(cx - 4.5, cy + 0.5);
                                cr.line_to(cx - 1.2, cy + 3.8);
                                cr.line_to(cx + 5.0, cy - 3.8);
                                cr.set_line_width(1.8);
                                cr.set_line_cap(cairo::LineCap::Round);
                                cr.set_source_rgba(1.0, 1.0, 1.0, 0.9 * alpha);
                                let _ = cr.stroke();
                            }
                            if !e.children.is_empty() {
                                let chevron = renderer.layout(cr, "›", Font::Body);
                                let (_, ext) = chevron.pixel_extents();
                                cr.set_source_rgba(1.0, 1.0, 1.0, 0.5 * alpha);
                                cr.move_to(renderer.snap(r.x + r.w - 12.0 - ext.width() as f64), renderer.snap(r.y + (rh - ext.height() as f64) / 2.0 - ext.y() as f64));
                                pangocairo::functions::show_layout(cr, &chevron);
                            }
                        }
                    }
                }
            }
            m.rows.push((r, row));
            y += rh;
        }
        cr.restore().unwrap();
    }

    fn colour_of(&self, item: &Item) -> (f64, f64, f64) {
        match item {
            Item::Volume if self.volume.is_some_and(|(_, m)| m) => MUTED,
            _ => (1.0, 1.0, 1.0),
        }
    }

    /// The hover wash under an item.
    fn wash(&self, cr: &Context, item: &Item, slot: Rect, now: Time) {
        let k = self.washes.get(item).map_or(0.0, |s| s.value(now).clamp(0.0, 1.0));
        if k > 0.002 && slot.w > 1.0 {
            rounded(cr, slot, 5.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.10 * k);
            let _ = cr.fill();
        }
    }
}

/// The battery's tooltip: how long it lasts, or until it is full.
fn battery_tip(b: Battery) -> String {
    let time = |m: u32| if m >= 60 { format!("{} ч {:02} мин", m / 60, m % 60) } else { format!("{m} мин") };
    match (b.charging, b.remaining) {
        (true, Some(m)) if b.percent < 100 => format!("Заряжается · до полной {}", time(m)),
        (true, _) => format!("Заряжается · {}%", b.percent),
        (false, Some(m)) => format!("Осталось {}", time(m)),
        (false, None) => format!("Батарея {}%", b.percent),
    }
}

/// The open page's row at index `k` as the menu lays them out: the way
/// back first (in a submenu), then its entries.
fn row_at(m: &Menu, k: usize) -> Option<Row> {
    let back = m.pages.len() > 1;
    match (back, k) {
        (true, 0) => Some(Row::Back),
        (true, k) => Some(Row::Entry(k - 1)),
        (false, k) => Some(Row::Entry(k)),
    }.filter(|r| match r {
        Row::Entry(i) => m.pages.last().is_some_and(|p| *i < p.len()),
        Row::Back => true,
    })
}

/// The rows a key can light: the way back, and enabled entries.
fn selectable_rows(m: &Menu) -> Vec<usize> {
    let back = m.pages.len() > 1;
    let entries = m.pages.last().map_or(&[][..], |p| &p[..]);
    let offset = back as usize;
    (back.then_some(0).into_iter()).chain(entries.iter().enumerate()
        .filter(|(_, e)| e.enabled && !e.separator).map(|(i, _)| i + offset)).collect()
}

/// A page's first entry a key can light (not the way back).
fn first_row(m: &Menu) -> Option<usize> {
    selectable_rows(m).into_iter().find(|k| *k > 0 || m.pages.len() == 1)
}

/// A small graph of `values` (0..1, oldest first, one per step) in `r`,
/// the newest at its right edge; `scroll` (1 → 0) slides the newest in.
fn graph(cr: &Context, r: Rect, values: &[f64], scroll: f64, colour: (f64, f64, f64)) {
    let step = r.w / (HISTORY - 1) as f64;
    cr.save().unwrap();
    // its floor
    rounded(cr, r, 3.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.06);
    let _ = cr.fill_preserve();
    cr.clip();
    if !values.is_empty() {
        let n = values.len();
        let at = |i: usize| {
            let x = r.x + r.w - (n - 1 - i) as f64 * step + scroll * step;
            (x, r.y + r.h - 1.0 - values[i].clamp(0.0, 1.0) * (r.h - 2.0))
        };
        cr.new_path();
        for i in 0..n {
            let (x, y) = at(i);
            if i == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
        }
        if n == 1 {
            let (x, y) = at(0);
            cr.line_to(x - step, y);
        }
        let line = cr.copy_path().ok();
        // the area under it, fading down
        let (x_last, _) = at(n - 1);
        let (x_first, _) = if n == 1 { (x_last - step, 0.0) } else { at(0) };
        cr.line_to(x_last, r.y + r.h);
        cr.line_to(x_first, r.y + r.h);
        cr.close_path();
        let fill = cairo::LinearGradient::new(0.0, r.y, 0.0, r.y + r.h);
        fill.add_color_stop_rgba(0.0, colour.0, colour.1, colour.2, 0.45);
        fill.add_color_stop_rgba(1.0, colour.0, colour.1, colour.2, 0.05);
        let _ = cr.set_source(&fill);
        let _ = cr.fill();
        if let Some(line) = line {
            cr.new_path();
            cr.append_path(&line);
            cr.set_line_width(1.3);
            cr.set_line_join(cairo::LineJoin::Round);
            cr.set_source_rgba(colour.0, colour.1, colour.2, 1.0);
            let _ = cr.stroke();
        }
    }
    cr.restore().unwrap();
}

fn inside(r: Rect, x: f64, y: f64) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

/// A menu row's label, after the room for a check mark.
fn menu_label(cr: &Context, renderer: &Renderer, text: &str, r: Rect, enabled: bool, alpha: f64) {
    let layout = renderer.layout_ellipsized(cr, text, Font::Body, r.w - 44.0, 1);
    let (_, ext) = layout.pixel_extents();
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha * if enabled { 0.95 } else { 0.35 });
    cr.move_to(renderer.snap(r.x + 28.0), renderer.snap(r.y + (r.h - ext.height() as f64) / 2.0 - ext.y() as f64));
    pangocairo::functions::show_layout(cr, &layout);
}

fn rounded(cr: &Context, r: Rect, radius: f64) {
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(r.x + r.w - radius, r.y + radius, radius, -FRAC_PI_2, 0.0);
    cr.arc(r.x + r.w - radius, r.y + r.h - radius, radius, 0.0, FRAC_PI_2);
    cr.arc(r.x + radius, r.y + r.h - radius, radius, FRAC_PI_2, PI);
    cr.arc(r.x + radius, r.y + radius, radius, PI, 1.5 * PI);
    cr.close_path();
}

fn clipped(cr: &Context, r: Rect, draw: impl FnOnce()) {
    if r.w <= 0.5 {
        return;
    }
    cr.save().unwrap();
    cr.rectangle(r.x, r.y, r.w, r.h);
    cr.clip();
    draw();
    cr.restore().unwrap();
}

fn centred_text(cr: &Context, renderer: &Renderer, text: &str, font: Font, cx: f64, colour: (f64, f64, f64), alpha: f64) {
    let layout = renderer.layout(cr, text, font);
    let (_, ext) = layout.pixel_extents();
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
    cr.move_to(renderer.snap(cx - ext.width() as f64 / 2.0 - ext.x() as f64), renderer.snap((HEIGHT - ext.height() as f64) / 2.0 - ext.y() as f64));
    pangocairo::functions::show_layout(cr, &layout);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> Bar {
        let mut bar = Bar::default();
        bar.workspaces = ["1", "2", "3", "10"].iter().map(|n| Workspace {
            name: n.to_string(), focused: *n == "3", urgent: false, output: "eDP-1".into(), apps: Vec::new(),
        }).collect();
        bar.output = Some("eDP-1".into());
        bar.volume = Some((0.5, false));
        bar.layout = Some("English".into());
        bar.battery = Some(Battery { percent: 84, charging: false, remaining: None });
        bar.details = Some(Details { disk_free: "120G".into(), cpu: 7, memory_gb: 3.2, memory_total_gb: 16.0 });
        bar
    }

    fn settle(bar: &mut Bar, from: Time) -> BarFrame {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1504, 120).unwrap();
        let cr = Context::new(&surface).unwrap();
        let renderer = Renderer::new(1.0);
        let mut frame = BarFrame::default();
        for i in 0..90 { frame = bar.draw(&cr, &renderer, 1504.0, 0.6, from + i as f64 / 60.0); }
        frame
    }

    #[test]
    fn two_corner_drops_hold_the_left_and_right_groups() {
        let mut bar = bar();
        let frame = settle(&mut bar, 10.0);
        assert_eq!(frame.input.len(), 2);
        let (l, r) = (frame.input[0], frame.input[1]);
        assert_eq!((l.x, l.y, l.h), (0.0, 0.0, HEIGHT));
        assert!((r.x + r.w - 1504.0).abs() < 1e-9 && r.h == HEIGHT);
        assert!(l.w < 300.0 && r.w < 500.0, "content-sized: {} {}", l.w, r.w);
        // the workspaces left, in order; the right group with ⋯ first and
        // the battery last
        let names: Vec<Item> = bar.slots.iter().map(|(i, _)| i.clone()).collect();
        assert_eq!(&names[..4], &["1", "2", "3", "10"].map(|n| Item::Workspace(n.into())));
        assert!(names[4..].contains(&Item::Details));
        assert_eq!(names.last(), Some(&Item::Battery));
        assert!(bar.contains(5.0, 10.0) && bar.contains(1500.0, 10.0) && !bar.contains(752.0, 10.0));
    }

    #[test]
    fn clicks_and_scrolls_ask_for_actions() {
        let mut bar = bar();
        settle(&mut bar, 10.0);
        let at = |bar: &Bar, item: Item| bar.slots.iter().find(|(i, _)| *i == item).map(|(_, r)| (r.x + r.w / 2.0, 16.0)).unwrap();
        let (x, y) = at(&bar, Item::Workspace("10".into()));
        assert_eq!(bar.press(x, y, 0x110, 11.0), Some(Action::Workspace("10".into())));
        assert_eq!(bar.scroll(x, y, 1.0), Some(Action::CycleWorkspace(1)));
        assert_eq!(bar.cycled(1).as_deref(), Some("10"));
        assert_eq!(bar.cycled(-1).as_deref(), Some("2"));
        let (x, y) = at(&bar, Item::Volume);
        assert_eq!(bar.press(x, y, 0x110, 11.0), Some(Action::ToggleMute));
        assert_eq!(bar.scroll(x, y, -1.0), Some(Action::VolumeStep(1)));
        let (x, y) = at(&bar, Item::Details);
        assert_eq!(bar.press(x, y, 0x110, 11.0), Some(Action::ToggleDrawer));
    }

    #[test]
    fn the_drawer_grows_the_right_drop_leftwards() {
        let mut bar = bar();
        let closed = settle(&mut bar, 10.0).input[1];
        bar.set_drawer(true);
        let open = settle(&mut bar, 12.0).input[1];
        assert!(open.w > closed.w + 100.0, "{} → {}", closed.w, open.w);
        assert!((open.x + open.w - 1504.0).abs() < 1e-9, "still pressed to the right edge");
        assert!(bar.slots.iter().any(|(i, r)| *i == Item::Cpu && r.w > 20.0));
    }

    #[test]
    fn a_tray_menu_hangs_from_the_right_drop_and_chooses() {
        use crate::tray::MenuEntry;
        let mut bar = bar();
        bar.tray = vec![TrayIcon { id: ":1.5/StatusNotifierItem".into(), icon: None, has_menu: true, item_is_menu: false, title: "T".into() }];
        settle(&mut bar, 10.0);
        let entry = |id, label: &str| MenuEntry { id, label: label.into(), enabled: true, ..MenuEntry::default() };
        let mut sub = entry(3, "More");
        sub.children = vec![entry(4, "Deep")];
        bar.open_menu(":1.5/StatusNotifierItem".into(), vec![entry(1, "Open"), MenuEntry { separator: true, ..MenuEntry::default() }, sub], 12.0);
        assert!(bar.menu_open());
        settle(&mut bar, 12.0);
        let rect = bar.menu.as_ref().unwrap().rect.unwrap();
        assert!(rect.y == HEIGHT && rect.x + rect.w <= 1504.0 && rect.h > 60.0);
        let row = |bar: &Bar, n: usize| { let r = bar.menu.as_ref().unwrap().rows[n].0; (r.x + 20.0, r.y + r.h / 2.0) };
        // into the submenu, and its entry chosen
        let (x, y) = row(&bar, 2);
        assert_eq!(bar.press(x, y, 0x110, 13.0), None);
        settle(&mut bar, 13.0);
        let (x, y) = row(&bar, 1);
        assert_eq!(bar.press(x, y, 0x110, 14.0), Some(Action::MenuChoose(":1.5/StatusNotifierItem".into(), 4)));
        assert!(!bar.menu_open());
        settle(&mut bar, 14.0);
        assert!(bar.menu.is_none(), "folded away");
    }

    #[test]
    fn a_workspace_switch_and_a_tray_menu_ripple_the_edge() {
        let mut bar = bar();
        bar.tray = vec![TrayIcon { id: "vpn".into(), icon: None, has_menu: true, item_is_menu: false, title: "VPN".into() }];
        settle(&mut bar, 10.0);
        assert_eq!(bar.take_splash(), None, "showing up is not a splash");
        for ws in &mut bar.workspaces { ws.focused = ws.name == "1"; }
        settle(&mut bar, 12.0);
        let (x, k) = bar.take_splash().expect("the pill moved");
        let one = bar.slots.iter().find(|(i, _)| *i == Item::Workspace("1".into())).unwrap().1;
        assert!(x > one.x && x < one.x + one.w && k > 0.0 && k < 1.0);
        assert_eq!(bar.take_splash(), None, "once");
        bar.open_menu("vpn".into(), Vec::new(), 14.0);
        let (x, _) = bar.take_splash().expect("the menu opened");
        assert!(x > 1504.0 - 60.0);
    }

    #[test]
    fn a_drop_pulled_down_stretches_and_jumps_back() {
        let mut bar = bar();
        settle(&mut bar, 10.0);
        let (x, y) = bar.slots.iter().find(|(i, _)| *i == Item::Volume).map(|(_, r)| (r.x + r.w / 2.0, 16.0)).unwrap();
        // a press and release in place is a click after all
        assert!(bar.tug_press(x, y, 11.0));
        assert!(!bar.tug_release(11.05));
        // pulled down it gives against its band, never past it
        assert!(bar.tug_press(x, y, 12.0));
        assert!(bar.tug_to(x, y + 200.0, 12.1));
        let frame = settle_once(&mut bar, 12.1);
        let d = bar.drop_sizes()[1].unwrap();
        assert!(d.h > HEIGHT + 10.0 && d.h < HEIGHT + TUG_DOWN, "{}", d.h);
        assert!(frame.input.iter().any(|r| r.h > HEIGHT + 10.0));
        // let go: back, and a ripple from under it
        assert!(bar.tug_release(12.2));
        assert!(bar.take_splash().is_some());
        settle(&mut bar, 13.0);
        assert!((bar.drop_sizes()[1].unwrap().h - HEIGHT).abs() < 0.5);
        assert!(!bar.is_animating(15.0));
    }

    fn settle_once(bar: &mut Bar, now: Time) -> BarFrame {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1504, 120).unwrap();
        let cr = Context::new(&surface).unwrap();
        bar.draw(&cr, &Renderer::new(1.0), 1504.0, 0.6, now)
    }

    #[test]
    fn a_tray_menu_follows_the_keys() {
        use crate::tray::MenuEntry;
        let mut bar = bar();
        let e = |id, label: &str, enabled| MenuEntry { id, label: label.into(), enabled, ..MenuEntry::default() };
        let mut sub = e(3, "More", true);
        sub.children = vec![e(4, "Inner", true)];
        let sep = MenuEntry { separator: true, ..MenuEntry::default() };
        bar.open_menu("vpn".into(), vec![e(1, "Open", true), sep, e(2, "Off", false), sub], 10.0);
        // down skips the separator and the disabled row, and wraps
        assert!(bar.menu_key("next", 10.1).0);
        assert_eq!(bar.menu.as_ref().unwrap().hovered, Some(0));
        bar.menu_key("next", 10.1);
        assert_eq!(bar.menu.as_ref().unwrap().hovered, Some(3));
        bar.menu_key("next", 10.1);
        assert_eq!(bar.menu.as_ref().unwrap().hovered, Some(0));
        bar.menu_key("prev", 10.1);
        // enter on a submenu goes in, its first entry lit (after the way back)
        assert_eq!(bar.menu_key("enter", 10.2), (true, None));
        assert_eq!(bar.menu.as_ref().unwrap().pages.len(), 2);
        assert_eq!(bar.menu.as_ref().unwrap().hovered, Some(1));
        // back comes out; enter on an entry chooses it and folds the menu
        bar.menu_key("back", 10.3);
        assert_eq!(bar.menu.as_ref().unwrap().pages.len(), 1);
        bar.menu_key("next", 10.3);
        bar.menu_key("next", 10.3);
        assert_eq!(bar.menu_key("enter", 10.4), (true, Some(Action::MenuChoose("vpn".into(), 1))));
        assert!(!bar.menu_open());
        // escape closes
        bar.open_menu("vpn".into(), vec![e(1, "Open", true)], 11.0);
        bar.menu_key("escape", 11.1);
        assert!(!bar.menu_open());
    }

    #[test]
    fn a_workspace_shows_its_applications_icons() {
        let mut bar = bar();
        settle(&mut bar, 10.0);
        let width = |bar: &Bar| bar.slots.iter().find(|(i, _)| *i == Item::Workspace("2".into())).unwrap().1.w;
        let plain = width(&bar);
        bar.workspaces[1].apps = vec!["foot".into(), "firefox".into()];
        // still loading: no room taken yet
        settle(&mut bar, 12.0);
        assert!((width(&bar) - plain).abs() < 0.5);
        let icon = cairo::ImageSurface::create(cairo::Format::ARgb32, 22, 22).unwrap();
        bar.app_icons.insert("foot".into(), Some(icon.clone()));
        bar.app_icons.insert("firefox".into(), Some(icon));
        settle(&mut bar, 14.0);
        let grown = width(&bar) - plain;
        assert!((grown - (APP_ICON_GAP + 2.0 * APP_ICON + APP_ICON_SPACING)).abs() < 0.5, "{grown}");
    }

    #[test]
    fn the_batterys_tip_says_how_long() {
        assert_eq!(battery_tip(Battery { percent: 84, charging: false, remaining: Some(192) }), "Осталось 3 ч 12 мин");
        assert_eq!(battery_tip(Battery { percent: 40, charging: true, remaining: Some(45) }), "Заряжается · до полной 45 мин");
        assert_eq!(battery_tip(Battery { percent: 100, charging: true, remaining: None }), "Заряжается · 100%");
    }

    #[test]
    fn fullscreen_draws_it_up_into_the_edge() {
        let mut bar = bar();
        settle(&mut bar, 10.0);
        bar.set_hidden(true, 12.0);
        let frame = settle(&mut bar, 12.0);
        assert!(frame.input.is_empty());
        assert!(!bar.contains(5.0, 10.0));
    }
}

#[cfg(test)]
mod symbols_look {
    use super::*;

    fn symbols_image() -> cairo::ImageSurface {
        let states: [(f64, bool, u8, bool); 6] = [
            (0.2, false, 84, false), (0.5, false, 50, false), (0.9, false, 100, false),
            (0.5, true, 15, false), (0.7, false, 60, true), (0.0, false, 7, false),
        ];
        let (w, h, zoom) = (330.0, 34.0, 3.0);
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, (w * zoom) as i32, (h * zoom * states.len() as f64) as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        for (row, (volume, muted, percent, charging)) in states.into_iter().enumerate() {
            // (each its own renderer: rolling digits remember their text)
            let renderer = Renderer::new(1.5);
            let mut bar = Bar::default();
            bar.volume = Some((volume, muted));
            bar.layout = Some("English".into());
            bar.battery = Some(Battery { percent, charging, remaining: None });
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 60).unwrap();
            let sc = Context::new(&scratch).unwrap();
            let mut t = 10.0;
            for _ in 0..120 { bar.draw(&sc, &renderer, w, 0.6, t); t += 1.0 / 60.0; }
            cr.save().unwrap();
            cr.translate(0.0, row as f64 * h * zoom);
            cr.rectangle(0.0, 0.0, w * zoom, h * zoom - 2.0);
            cr.clip();
            cr.scale(zoom, zoom);
            cr.set_source_rgb(0.18, 0.35, 0.40);
            cr.paint().unwrap();
            bar.draw(&cr, &renderer, w, 0.6, t);
            cr.restore().unwrap();
        }
        drop(cr);
        surface
    }

    #[test]
    #[ignore = "writes target/gallery/bar-symbols.png"]
    fn symbols() {
        std::fs::create_dir_all("target/gallery").unwrap();
        symbols_image().write_to_png(&mut std::fs::File::create("target/gallery/bar-symbols.png").unwrap()).unwrap();
    }

    #[test]
    fn symbols_look_as_before() {
        crate::golden::check("bar-symbols", &symbols_image());
    }
}

#[cfg(test)]
mod tug_look {
    use super::*;

    fn pulled_drop_image() -> cairo::ImageSurface {
        let renderer = Renderer::new(1.5);
        let (w, h, zoom) = (420.0, 70.0, 2.0);
        let pulls = [0.0, 30.0, 90.0, 400.0];
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, (w * zoom) as i32, (h * zoom * pulls.len() as f64) as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        for (row, pull) in pulls.into_iter().enumerate() {
            let mut bar = Bar::default();
            bar.volume = Some((0.6, false));
            bar.layout = Some("English".into());
            bar.battery = Some(Battery { percent: 72, charging: false, remaining: None });
            let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 100).unwrap();
            let sc = Context::new(&scratch).unwrap();
            let mut t = 10.0;
            for _ in 0..90 { bar.draw(&sc, &renderer, w, 0.6, t); t += 1.0 / 60.0; }
            if pull > 0.0 {
                let x = w - 60.0;
                bar.tug_press(x, 16.0, t);
                bar.tug_to(x, 16.0 + pull, t + 0.05);
            }
            cr.save().unwrap();
            cr.translate(0.0, row as f64 * h * zoom);
            cr.rectangle(0.0, 0.0, w * zoom, h * zoom - 2.0);
            cr.clip();
            cr.scale(zoom, zoom);
            cr.set_source_rgb(0.18, 0.35, 0.40);
            cr.paint().unwrap();
            bar.draw(&cr, &renderer, w, 0.6, t + 0.05);
            cr.restore().unwrap();
        }
        drop(cr);
        surface
    }

    #[test]
    #[ignore = "writes target/gallery/bar-tug.png"]
    fn pulled_drop() {
        std::fs::create_dir_all("target/gallery").unwrap();
        pulled_drop_image().write_to_png(&mut std::fs::File::create("target/gallery/bar-tug.png").unwrap()).unwrap();
    }

    #[test]
    fn pulled_drop_look_as_before() {
        crate::golden::check("bar-tug", &pulled_drop_image());
    }
}

#[cfg(test)]
mod workspace_apps_look {
    use super::*;

    #[test]
    #[ignore = "writes target/gallery/bar-workspaces.png"]
    fn workspaces_with_apps() {
        let renderer = Renderer::new(1.5);
        let loader = crate::iconload::Loader::new();
        let (w, h, zoom) = (300.0, 40.0, 3.0);
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, (w * zoom) as i32, (h * zoom) as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        let mut bar = Bar::default();
        let ws = |name: &str, focused, apps: &[&str]| Workspace { name: name.into(), focused, apps: apps.iter().map(|a| a.to_string()).collect(), ..Default::default() };
        bar.workspaces = vec![ws("1", false, &["firefox-esr"]), ws("2", true, &["Alacritty", "org.telegram.desktop"]), ws("3", false, &[])];
        for app in ["firefox-esr", "Alacritty", "org.telegram.desktop"] {
            let name = if app == "Alacritty" { "Alacritty" } else { app };
            bar.app_icons.insert(app.into(), loader.get_scaled(name, (APP_ICON * 1.5) as i32));
        }
        let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 600, 100).unwrap();
        let sc = Context::new(&scratch).unwrap();
        let mut t = 10.0;
        for _ in 0..90 { bar.draw(&sc, &renderer, 1504.0, 0.6, t); t += 1.0 / 60.0; }
        cr.scale(zoom, zoom);
        cr.set_source_rgb(0.18, 0.35, 0.40);
        cr.paint().unwrap();
        bar.draw(&cr, &renderer, 1504.0, 0.6, t);
        drop(cr);
        surface.write_to_png(&mut std::fs::File::create("target/gallery/bar-workspaces.png").unwrap()).unwrap();
    }
}

#[cfg(test)]
mod drawer_look {
    use super::*;

    fn drawer_graphs_image() -> cairo::ImageSurface {
        let renderer = Renderer::new(1.5);
        let (w, h, zoom) = (620.0, 36.0, 2.5);
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, (w * zoom) as i32, (h * zoom) as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        let mut bar = Bar::default();
        bar.volume = Some((0.6, false));
        bar.battery = Some(Battery { percent: 72, charging: false, remaining: None });
        bar.set_drawer(true);
        let mut t = 10.0;
        for i in 0..24 {
            let cpu = (8.0 + 30.0 * ((i as f64) * 0.7).sin().abs() + if i == 15 { 50.0 } else { 0.0 }) as u8;
            bar.push_details(Details { disk_free: "118G".into(), cpu, memory_gb: 5.0 + i as f64 * 0.08, memory_total_gb: 16.0 }, t);
            t += 1.0;
        }
        let scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, 900, 100).unwrap();
        let sc = Context::new(&scratch).unwrap();
        for _ in 0..90 { bar.draw(&sc, &renderer, w, 0.6, t); t += 1.0 / 60.0; }
        cr.scale(zoom, zoom);
        cr.set_source_rgb(0.18, 0.35, 0.40);
        cr.paint().unwrap();
        bar.draw(&cr, &renderer, w, 0.6, t);
        drop(cr);
        surface
    }

    #[test]
    #[ignore = "writes target/gallery/bar-drawer.png"]
    fn drawer_graphs() {
        std::fs::create_dir_all("target/gallery").unwrap();
        drawer_graphs_image().write_to_png(&mut std::fs::File::create("target/gallery/bar-drawer.png").unwrap()).unwrap();
    }

    #[test]
    fn drawer_graphs_look_as_before() {
        crate::golden::check("bar-drawer", &drawer_graphs_image());
    }
}
