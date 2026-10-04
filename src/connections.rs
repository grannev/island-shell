//! Native Wi-Fi / Bluetooth sheets, including their interaction and motion.

use crate::anim::{Glide, Spring, Time};
use crate::island::{Font, Renderer, smoothstep};
use crate::menu_style::{self, ButtonKind, RAIL};
use crate::network::{self, Action, Device, Input, Kind, Message, Snapshot, Update};
use cairo::Context;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const WIDTH: f64 = 480.0;
const TABS_Y: f64 = menu_style::CONTENT_TOP;
const TABS_H: f64 = 32.0;
const LIST_Y: f64 = TABS_Y + TABS_H + 12.0;
pub const ROW_H: f64 = 52.0;
const VISIBLE: usize = 5;
const SCROLL_TAU: f64 = 0.08;
/// below the list: a status line, the search button and the bottom rail
const FOOTER: f64 = 24.0 + menu_style::BUTTON_H + RAIL;
pub const HEIGHT: f64 = LIST_Y + VISIBLE as f64 * ROW_H + FOOTER;
const PROMPT_Y: f64 = menu_style::CONTENT_TOP;
const PROMPT_FIELD_Y: f64 = PROMPT_Y + 84.0;
const PROMPT_BUTTONS_Y: f64 = PROMPT_FIELD_Y + 40.0 + 22.0;
const PROMPT_H: f64 = PROMPT_BUTTONS_Y + menu_style::BUTTON_H + 18.0 - PROMPT_Y;
const TOGGLE_X: f64 = WIDTH - RAIL - menu_style::CLOSE_SIZE - 12.0 - 46.0;
const INNER: f64 = WIDTH - 2.0 * RAIL;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Hit {
    Close,
    Power,
    Tab(usize),
    Search,
    Row(String),
    Cancel,
    Submit,
    Reveal,
}

enum Prompt {
    Wifi(Device),
    Pair(network::Prompt),
}

pub struct Panel {
    pub kind: Kind,
    session: u64,
    request: u64,
    tx: Sender<Message>,
    cancel: Arc<AtomicBool>,
    closed: bool,
    snapshot: Snapshot,
    loaded: bool,
    busy: bool,
    scanning: bool,
    status: String,
    error: Option<String>,
    tab: usize,
    tab_slide: Spring,
    /// the list position in rows; the wheel glides it (anim::Glide)
    scroll: Glide,
    hovered: Option<Hit>,
    hover: HashMap<Hit, Spring>,
    slots: HashMap<String, Spring>,
    born: HashMap<String, Time>,
    old_rows: Vec<Device>,
    changed: Time,
    prompt: Option<Prompt>,
    prompt_since: Time,
    input: String,
    reveal: bool,
    input_since: Time,
    power: Spring,
    connecting: Option<Device>,
    retry_password: Option<String>,
    refreshing: bool,
    next_refresh: Time,
    frame_height: f64,
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.close();
    }
}

impl Panel {
    pub fn new(kind: Kind, session: u64, tx: Sender<Message>, now: Time) -> Self {
        let mut panel = Self::idle(kind, session, tx, now);
        panel.start(Action::Refresh, now);
        panel
    }

    /// A settled sheet with fixed devices and no background work (gallery).
    #[cfg(test)]
    pub(crate) fn preview(kind: Kind, devices: Vec<Device>, prompt: Option<usize>, now: Time) -> Self {
        let (tx, _) = smithay_client_toolkit::reexports::calloop::channel::channel();
        let mut panel = Self::idle(kind, 1, tx, now);
        panel.set_snapshot(Snapshot { powered: true, devices }, now);
        panel.power.set(1.0, now);
        if let Some(index) = prompt {
            panel.prompt = Some(match kind {
                Kind::Wifi => Prompt::Wifi(panel.snapshot.devices[index].clone()),
                Kind::Bluetooth => Prompt::Wifi(panel.snapshot.devices[index].clone()),
            });
            panel.prompt_since = now;
            panel.input = "password".into();
        }
        panel
    }

    fn idle(kind: Kind, session: u64, tx: Sender<Message>, now: Time) -> Self {
        Self {
            kind,
            session,
            request: 0,
            tx,
            cancel: Arc::new(AtomicBool::new(false)),
            closed: false,
            snapshot: Snapshot { powered: false, devices: vec![] },
            loaded: false,
            busy: false,
            scanning: false,
            status: String::new(),
            error: None,
            tab: 0,
            tab_slide: Spring::new(0.0, 0.32, 0.86),
            scroll: Glide::new(0.0, SCROLL_TAU),
            hovered: None,
            hover: HashMap::new(),
            slots: HashMap::new(),
            born: HashMap::new(),
            old_rows: vec![],
            changed: now,
            prompt: None,
            prompt_since: now,
            input: String::new(),
            reveal: false,
            input_since: now,
            power: Spring::new(0.0, 0.25, 1.0),
            connecting: None,
            retry_password: None,
            refreshing: false,
            next_refresh: now + 4.0,
            frame_height: 378.0,
        }
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(Prompt::Pair(prompt)) = &mut self.prompt {
            if let Some(reply) = prompt.reply.take() {
                let _ = reply.send(None);
            }
        }
    }

    fn rows(&self) -> Vec<Device> {
        self.snapshot
            .devices
            .iter()
            .filter(|d| match self.kind {
                Kind::Wifi => self.tab == 0 || d.paired,
                Kind::Bluetooth => {
                    if self.tab == 0 {
                        d.paired
                    } else {
                        !d.paired
                    }
                }
            })
            .cloned()
            .collect()
    }

    pub fn height(&self) -> f64 {
        if self.prompt.is_some() {
            PROMPT_Y + PROMPT_H + RAIL
        } else {
            LIST_Y + self.rows().len().clamp(3, VISIBLE) as f64 * ROW_H + FOOTER
        }
    }

    fn list_height(&self) -> f64 {
        (self.frame_height - FOOTER - LIST_Y).max(0.0)
    }

    pub fn describe(&self) -> String {
        format!(
            " network={:?} tab={} rows={} busy={} loaded={} error={}",
            self.kind,
            self.tab,
            self.rows().len(),
            self.busy as u8,
            self.loaded as u8,
            self.error.is_some() as u8
        )
    }

    fn update_slots(&mut self, now: Time) {
        for (index, row) in self.rows().iter().enumerate() {
            self.born.entry(row.id.clone()).or_insert(now);
            let slot = self.slots.entry(row.id.clone()).or_insert_with(|| Spring::new(index as f64 + 0.14, 0.3, 1.0));
            slot.set(index as f64, now);
        }
        let max = self.rows().len().saturating_sub(VISIBLE) as f64;
        self.scroll.set(self.scroll.target().min(max), now);
    }

    fn start(&mut self, action: Action, now: Time) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.request += 1;
        self.busy = true;
        self.refreshing = false;
        self.scanning = matches!(action, Action::Scan);
        self.connecting = if let Action::Connect(device, _) = &action { Some(device.clone()) } else { None };
        self.error = None;
        self.status = match &action {
            Action::Refresh => "Обновляем список…".into(),
            Action::Scan => "Идёт поиск поблизости…".into(),
            Action::Power(on) => if *on { "Включаем…" } else { "Выключаем…" }.into(),
            Action::Connect(d, _) => {
                if self.kind == Kind::Bluetooth && !d.paired {
                    format!("Сопряжение с {}…", d.name)
                } else {
                    format!("Подключаем {}…", d.name)
                }
            }
            Action::Disconnect(d) => format!("Отключаем {}…", d.name),
        };
        self.hover_at(None, now);
        network::spawn(
            self.kind,
            action,
            network::Job {
                session: self.session,
                request: self.request,
                cancel: self.cancel.clone(),
                tx: self.tx.clone(),
            },
        );
    }

    pub fn receive(&mut self, message: Message, now: Time) -> bool {
        if self.closed || message.session != self.session || message.request != self.request {
            return false;
        }
        match message.update {
            Update::Snapshot(snapshot) => self.set_snapshot(snapshot, now),
            Update::Finished(result) => {
                self.busy = false;
                self.refreshing = false;
                self.next_refresh = now + 4.0;
                self.scanning = false;
                self.prompt = None;
                self.input.clear();
                match result {
                    Ok(snapshot) => {
                        self.set_snapshot(snapshot, now);
                        self.retry_password = None;
                        self.status = "Список обновлён".into();
                    }
                    Err(error) => {
                        self.retry_password = self
                            .connecting
                            .as_ref()
                            .filter(|d| self.kind == Kind::Wifi && !d.security.is_empty())
                            .map(|d| d.id.clone());
                        self.loaded = true;
                        self.error = Some(error);
                        self.status.clear();
                    }
                }
            }
            Update::Prompt(prompt) => {
                self.prompt = Some(Prompt::Pair(prompt));
                self.prompt_since = now;
                self.input.clear();
                self.input_since = now;
                self.reveal = false;
            }
            Update::PromptDismissed => {
                self.prompt = None;
                self.input.clear();
            }
        }
        true
    }

    fn set_snapshot(&mut self, snapshot: Snapshot, now: Time) {
        self.snapshot = snapshot;
        self.loaded = true;
        self.power.set(if self.snapshot.powered { 1.0 } else { 0.0 }, now);
        self.update_slots(now);
    }

    pub fn is_animating(&self, now: Time) -> bool {
        !self.closed
            && (self.busy
                || self.prompt.is_some()
                || now - self.changed < 0.3
                || self.hover.values().any(|s| s.is_moving(now))
                || self.slots.values().any(|s| s.is_moving(now))
                || self.born.values().any(|t| now - t < 0.3)
                || self.tab_slide.is_moving(now)
                || self.scroll.is_moving(now)
                || self.power.is_moving(now))
    }

    pub fn deadline(&self) -> Option<Time> {
        (!self.closed && !self.busy && !self.refreshing && self.prompt.is_none()).then_some(self.next_refresh)
    }

    pub fn advance(&mut self, now: Time) -> bool {
        if !self.deadline().is_some_and(|deadline| now >= deadline) {
            return false;
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.request += 1;
        self.refreshing = true;
        network::spawn(
            self.kind,
            Action::Refresh,
            network::Job {
                session: self.session,
                request: self.request,
                cancel: self.cancel.clone(),
                tx: self.tx.clone(),
            },
        );
        true
    }

    fn hit(&self, x: f64, y: f64, now: Time) -> Option<Hit> {
        let (cx, cy, size) = menu_style::close_rect(WIDTH);
        if contains(x, y, cx - 4.0, cy - 4.0, size + 8.0, size + 8.0) {
            return Some(Hit::Close);
        }
        if self.prompt.is_some() {
            let half = (INNER - 36.0 - 10.0) / 2.0;
            if contains(x, y, RAIL + 18.0, PROMPT_BUTTONS_Y, half, menu_style::BUTTON_H) {
                return Some(Hit::Cancel);
            }
            if contains(x, y, RAIL + 18.0 + half + 10.0, PROMPT_BUTTONS_Y, half, menu_style::BUTTON_H) && self.can_submit() {
                return Some(Hit::Submit);
            }
            if matches!(self.prompt, Some(Prompt::Wifi(_))) && contains(x, y, WIDTH - RAIL - 18.0 - 86.0, PROMPT_FIELD_Y, 86.0, 40.0) {
                return Some(Hit::Reveal);
            }
            return None;
        }
        if self.busy && !self.scanning {
            return None;
        }
        if contains(x, y, TOGGLE_X - 4.0, 22.0, 54.0, 36.0) && self.loaded {
            return Some(Hit::Power);
        }
        if contains(x, y, RAIL, TABS_Y, INNER, TABS_H) {
            return Some(Hit::Tab(if x < WIDTH / 2.0 { 0 } else { 1 }));
        }
        if contains(x, y, RAIL, self.frame_height - RAIL - menu_style::BUTTON_H, INNER, menu_style::BUTTON_H)
            && (self.snapshot.powered || self.error.is_some())
        {
            return Some(Hit::Search);
        }
        if self.snapshot.powered && contains(x, y, RAIL, LIST_Y, INNER, self.list_height()) && now - self.changed > 0.18 {
            let offset = self.scroll.value(now);
            for row in self.rows() {
                let pos = self.slots.get(&row.id).map_or(0.0, |s| s.value(now));
                if contains(x, y, RAIL, LIST_Y + (pos - offset) * ROW_H, INNER, ROW_H) {
                    return Some(Hit::Row(row.id));
                }
            }
        }
        None
    }

    fn hover_at(&mut self, hit: Option<Hit>, now: Time) -> bool {
        if self.hovered == hit {
            return false;
        }
        if let Some(old) = self.hovered.take() {
            self.hover.entry(old).or_insert_with(|| Spring::new(0.0, 0.24, 1.0)).set(0.0, now);
        }
        if let Some(next) = &hit {
            self.hover.entry(next.clone()).or_insert_with(|| Spring::new(0.0, 0.24, 1.0)).set(1.0, now);
        }
        self.hovered = hit;
        true
    }

    pub fn pointer(&mut self, x: f64, y: f64, now: Time) -> bool {
        self.hover_at(self.hit(x, y, now), now)
    }
    pub fn leave(&mut self, now: Time) -> bool {
        self.hover_at(None, now)
    }
    fn lit(&self, hit: Hit, now: Time) -> f64 {
        self.hover.get(&hit).map_or(0.0, |s| s.value(now).clamp(0.0, 1.0))
    }

    /// Returns true only for the explicit close button. Other clicks are
    /// consumed by the sheet, including its whitespace and disabled controls.
    pub fn click(&mut self, x: f64, y: f64, now: Time) -> bool {
        let Some(hit) = self.hit(x, y, now) else { return false };
        match hit {
            Hit::Close => return true,
            Hit::Power => self.start(Action::Power(!self.snapshot.powered), now),
            Hit::Tab(index) => {
                if index != self.tab {
                    self.old_rows = self.rows();
                    self.changed = now;
                    self.tab = index;
                    self.tab_slide.set(index as f64, now);
                    self.scroll = Glide::new(0.0, SCROLL_TAU);
                    self.slots.clear();
                    self.update_slots(now);
                    self.hover_at(None, now);
                    if self.kind == Kind::Bluetooth && index == 1 && self.snapshot.powered {
                        self.start(Action::Scan, now);
                    } else if self.scanning {
                        self.start(Action::Refresh, now);
                    }
                }
            }
            Hit::Search => {
                if self.scanning {
                    self.start(Action::Refresh, now);
                } else {
                    if self.kind == Kind::Bluetooth && self.tab != 1 {
                        self.old_rows = self.rows();
                        self.changed = now;
                        self.tab = 1;
                        self.tab_slide.set(1.0, now);
                        self.scroll = Glide::new(0.0, SCROLL_TAU);
                        self.slots.clear();
                        self.update_slots(now);
                    }
                    self.start(if self.snapshot.powered { Action::Scan } else { Action::Refresh }, now);
                }
            }
            Hit::Row(id) => {
                if let Some(device) = self.snapshot.devices.iter().find(|d| d.id == id).cloned() {
                    if device.connected {
                        self.start(Action::Disconnect(device), now);
                    } else if self.kind == Kind::Wifi
                        && !device.security.is_empty()
                        && (device.profile.is_none() || self.retry_password.as_ref() == Some(&device.id))
                    {
                        self.prompt = Some(Prompt::Wifi(device));
                        self.prompt_since = now;
                        self.input.clear();
                        self.input_since = now;
                        self.reveal = false;
                        self.cancel.store(true, Ordering::Relaxed);
                        self.request += 1;
                        self.busy = false;
                        self.scanning = false;
                        self.refreshing = false;
                    } else {
                        self.start(Action::Connect(device, None), now);
                    }
                }
            }
            Hit::Cancel => {
                if let Some(Prompt::Pair(prompt)) = self.prompt.take() {
                    if let Some(reply) = prompt.reply {
                        let _ = reply.send(None);
                    }
                    self.start(Action::Refresh, now);
                }
                self.prompt = None;
                self.input.clear();
            }
            Hit::Submit => self.submit(now),
            Hit::Reveal => self.reveal = !self.reveal,
        }
        false
    }

    fn can_submit(&self) -> bool {
        match &self.prompt {
            Some(Prompt::Wifi(d)) => {
                if d.security.contains("WEP") {
                    !self.input.is_empty()
                } else {
                    (8..=63).contains(&self.input.len())
                        || (self.input.len() == 64 && self.input.chars().all(|c| c.is_ascii_hexdigit()))
                }
            }
            Some(Prompt::Pair(p)) => {
                p.reply.is_some()
                    && match p.input {
                        Input::None => true,
                        Input::Pin => (1..=16).contains(&self.input.len()),
                        Input::Passkey => {
                            (1..=6).contains(&self.input.len()) && self.input.chars().all(|c| c.is_ascii_digit())
                        }
                    }
            }
            None => false,
        }
    }

    fn submit(&mut self, now: Time) {
        if !self.can_submit() {
            return;
        }
        match self.prompt.take() {
            Some(Prompt::Wifi(device)) => {
                let secret = std::mem::take(&mut self.input);
                self.start(Action::Connect(device, Some(secret)), now);
            }
            Some(Prompt::Pair(p)) => {
                if let Some(reply) = p.reply {
                    let _ = reply.send(Some(std::mem::take(&mut self.input)));
                }
            }
            None => {}
        }
    }

    pub fn key(&mut self, key: &str, text: Option<&str>, ctrl: bool, now: Time) {
        if self.prompt.is_none() {
            match key {
                "Down" => self.scroll_by(1.0, now),
                "Up" => self.scroll_by(-1.0, now),
                _ => {}
            }
            return;
        }
        match key {
            "Return" => self.submit(now),
            "BackSpace" => {
                self.input.pop();
                self.input_since = now;
            }
            _ if ctrl && text.is_some_and(|t| t.eq_ignore_ascii_case("u")) => {
                self.input.clear();
                self.input_since = now;
            }
            _ if !ctrl => {
                if let Some(text) = text.filter(|t| !t.chars().any(char::is_control)) {
                    if self.input.len() + text.len() <= 128 {
                        self.input.push_str(text);
                        self.input_since = now;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn scroll_by(&mut self, rows: f64, now: Time) {
        if self.prompt.is_some() {
            return;
        }
        let max = self.rows().len().saturating_sub(VISIBLE) as f64;
        self.scroll.set((self.scroll.target() + rows).clamp(0.0, max), now);
        self.hover_at(None, now);
    }

    pub fn draw(&mut self, cr: &Context, renderer: &Renderer, x: f64, y: f64, height: f64, alpha: f64, now: Time) {
        self.frame_height = height;
        cr.save().unwrap();
        cr.translate(x, y);
        // A group guarantees one coherent fade for text, icons and controls.
        let grouped = alpha < 0.999;
        if grouped { cr.push_group(); }
        let subtitle = if !self.loaded {
            "Загрузка…"
        } else if self.snapshot.powered {
            "Включён"
        } else {
            "Выключен"
        };
        let kind = self.kind;
        let glyph = move |cr: &Context, cx: f64, cy: f64| symbol(cr, kind, cx, cy + 0.5, 1.05, 1.0);
        menu_style::header_keyed(cr, renderer, 0.0, 0.0, WIDTH, Some((menu_style::BLUE, &glyph)),
            if self.kind == Kind::Wifi { "Wi-Fi" } else { "Bluetooth" }, Some(subtitle), WIDTH - TOGGLE_X + 8.0, 1.0,
            &format!("net{}:subtitle", self.session));
        menu_style::toggle(cr, TOGGLE_X, menu_style::HEADER_MID - 14.0, self.power.value(now),
            self.lit(Hit::Power, now), if self.loaded { 1.0 } else { 0.4 });
        let (cx, cy, size) = menu_style::close_rect(WIDTH);
        menu_style::close(cr, cx, cy, size, self.lit(Hit::Close, now), 1.0);

        if self.prompt.is_some() {
            self.draw_prompt(cr, renderer, now);
        } else {
            let tabs = if self.kind == Kind::Wifi { ["Все сети", "Сохранённые"] } else { ["Мои устройства", "Поиск"] };
            menu_style::segmented(cr, renderer, (RAIL, TABS_Y, INNER, TABS_H), &tabs, self.tab_slide.value(now),
                &[self.lit(Hit::Tab(0), now), self.lit(Hit::Tab(1), now)], 1.0);
            let rows = self.rows();
            let list_h = self.list_height();
            let fade = smoothstep((now - self.changed) / 0.24);
            if rows.is_empty() || !self.snapshot.powered {
                let (title, hint) = if !self.loaded {
                    ("Загрузка…", "Получаем состояние адаптера")
                } else if !self.snapshot.powered {
                    ("Адаптер выключен", "Включите переключатель вверху")
                } else if self.busy {
                    ("Ищем поблизости", "Список обновится автоматически")
                } else if self.kind == Kind::Bluetooth && self.tab == 0 {
                    ("Пока нет устройств", "Откройте поиск, чтобы создать пару")
                } else if self.kind == Kind::Bluetooth {
                    ("Устройства не найдены", "Включите на устройстве режим сопряжения")
                } else {
                    ("Сети не найдены", "Нажмите «Найти сети» ниже")
                };
                menu_style::group(cr, RAIL, LIST_Y, INNER, list_h, 0.0, 1.0);
                let centre = LIST_Y + list_h / 2.0;
                let appear = if self.old_rows.is_empty() { 1.0 } else { fade };
                symbol(cr, self.kind, WIDTH / 2.0, centre - 26.0, 1.6, menu_style::TERTIARY * appear);
                menu_style::centred(cr, renderer, title, (RAIL, centre - 6.0, INNER, 22.0), Font::Headline,
                    (1.0, 1.0, 1.0), appear);
                menu_style::centred(cr, renderer, hint, (RAIL, centre + 16.0, INNER, 20.0), Font::Subhead,
                    (1.0, 1.0, 1.0), menu_style::SECONDARY * appear);
            } else {
                let shown = rows.len().clamp(1, VISIBLE) as f64 * ROW_H;
                menu_style::group(cr, RAIL, LIST_Y, INNER, shown.min(list_h), 0.0, 1.0);
                cr.save().unwrap();
                menu_style::rounded(cr, RAIL, LIST_Y, INNER, shown.min(list_h), menu_style::GROUP_R);
                cr.clip();
                if fade < 1.0 && !self.old_rows.is_empty() {
                    for (i, d) in self.old_rows.iter().enumerate() {
                        self.draw_row(cr, renderer, d, LIST_Y + i as f64 * ROW_H - fade * 8.0, 1.0 - fade, i > 0, now);
                    }
                }
                let offset = self.scroll.value(now);
                for (i, d) in rows.iter().enumerate() {
                    let slot = self.slots.get(&d.id).map_or(i as f64, |s| s.value(now));
                    let ry = LIST_Y + (slot - offset) * ROW_H + (1.0 - fade) * 8.0;
                    if ry + ROW_H < LIST_Y || ry > LIST_Y + ROW_H * VISIBLE as f64 {
                        continue;
                    }
                    let appear = self.born.get(&d.id).map_or(1.0, |t| smoothstep((now - t) / 0.25));
                    self.draw_row(cr, renderer, d, ry, fade * appear, slot - offset > 0.5, now);
                }
                cr.restore().unwrap();
                if rows.len() > VISIBLE {
                    let track = ROW_H * VISIBLE as f64 - 12.0;
                    let thumb = track * VISIBLE as f64 / rows.len() as f64;
                    let sy = LIST_Y + 6.0 + (track - thumb) * offset / (rows.len() - VISIBLE) as f64;
                    menu_style::rounded(cr, WIDTH - RAIL - 6.0, sy, 3.0, thumb, 1.5);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.28);
                    let _ = cr.fill();
                }
            }
            // a status line only when there is something to say
            let status_y = LIST_Y + list_h + 5.0;
            if let Some(error) = &self.error {
                menu_style::text(cr, renderer, error, RAIL + 2.0, status_y, INNER - 4.0, Font::Footnote, menu_style::RED, 1.0);
            } else if self.busy && !self.status.is_empty() {
                menu_style::label(cr, renderer, &self.status, RAIL + 2.0, status_y, INNER - 4.0, Font::Footnote, menu_style::SECONDARY);
            }
            let search = if self.scanning {
                "Остановить поиск"
            } else if self.busy {
                "Подождите…"
            } else if self.kind == Kind::Wifi {
                "Найти сети"
            } else {
                "Искать устройства"
            };
            let enabled = (self.snapshot.powered || self.error.is_some()) && (!self.busy || self.scanning);
            let by = height - RAIL - menu_style::BUTTON_H;
            menu_style::button(cr, renderer, search, (RAIL, by, INNER, menu_style::BUTTON_H), ButtonKind::Neutral,
                self.lit(Hit::Search, now), if enabled { 1.0 } else { 0.4 });
            if self.busy {
                menu_style::spinner(cr, RAIL + 22.0, by + menu_style::BUTTON_H / 2.0, 7.0, now, 1.0);
            }
        }
        if grouped {
            cr.pop_group_to_source().unwrap();
            let _ = cr.paint_with_alpha(alpha);
        }
        cr.restore().unwrap();
        cr.new_path();
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(&self, cr: &Context, renderer: &Renderer, d: &Device, y: f64, alpha: f64, divider: bool, now: Time) {
        if alpha <= 0.002 {
            return;
        }
        let hover = self.lit(Hit::Row(d.id.clone()), now);
        let text_x = RAIL + 58.0;
        if divider {
            menu_style::separator(cr, renderer, text_x, WIDTH - RAIL, y, alpha);
        }
        menu_style::highlight(cr, RAIL, y, INNER, ROW_H, hover, alpha);
        let mid = y + ROW_H / 2.0;
        let well_x = RAIL + 14.0 + 15.0;
        menu_style::well(cr, well_x, mid, 15.0, d.connected.then_some(menu_style::BLUE), alpha);
        if self.kind == Kind::Wifi {
            symbol(cr, Kind::Wifi, well_x, mid + 0.5, 0.78, alpha);
        } else {
            device_symbol(cr, &d.icon, well_x, mid, alpha);
        }
        let detail = if self.kind == Kind::Wifi {
            if d.connected {
                "Подключено".to_string()
            } else if d.paired {
                "Сохранённая сеть".to_string()
            } else if d.security.is_empty() {
                "Открытая сеть".to_string()
            } else {
                format!("Защищённая · {}", d.security)
            }
        } else if d.name == crate::network::UNNAMED_BLUETOOTH {
            format!("{} · имя не передано", d.interface)
        } else if d.connected {
            "Подключено".to_string()
        } else if d.paired {
            "Не подключено".to_string()
        } else {
            "Новое устройство".to_string()
        };
        let action = if d.connected {
            "Отключить"
        } else if self.kind == Kind::Bluetooth && !d.paired {
            "Создать пару"
        } else {
            "Подключить"
        };
        let action_w = renderer.layout(cr, action, Font::Label).pixel_extents().1.width() as f64 + 24.0;
        let trailing_w = action_w.max(64.0);
        let text_w = WIDTH - RAIL - 14.0 - trailing_w - 8.0 - text_x;
        let title_h = menu_style::line_height(cr, renderer, Font::Body);
        let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
        let top = mid - (title_h + sub_h) / 2.0;
        menu_style::label(cr, renderer, &d.name, text_x, top, text_w, Font::Body, alpha);
        // "Подключение…" → "Подключено" reads as a change, not a jump
        crate::motion::text(cr, renderer, &format!("net{}:{}", self.session, d.id), &detail, text_x, top + title_h, text_w,
            0.0, Font::Subhead, (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
        // at rest the row shows its state; under the pointer, what a click does
        let right = WIDTH - RAIL - 14.0;
        let rest = alpha * (1.0 - hover);
        if rest > 0.01 {
            if self.kind == Kind::Wifi {
                strength(cr, right - 10.0, mid, d.signal, rest);
                if !d.security.is_empty() {
                    lock(cr, right - 34.0, mid, rest * menu_style::SECONDARY);
                }
            } else if let Some(battery) = d.battery {
                let s = format!("{battery}%");
                let w = renderer.layout(cr, &s, Font::Subhead).pixel_extents().1.width() as f64;
                menu_style::label(cr, renderer, &s, right - w, mid - sub_h / 2.0, w + 2.0, Font::Subhead, menu_style::SECONDARY * rest);
            }
        }
        if hover > 0.01 {
            let h = 26.0;
            let kind = if d.connected { ButtonKind::Neutral } else { ButtonKind::Primary };
            menu_style::button(cr, renderer, action, (right - action_w, mid - h / 2.0, action_w, h), kind, 0.0, alpha * hover);
        }
    }

    fn draw_prompt(&self, cr: &Context, renderer: &Renderer, now: Time) {
        let Some(prompt) = &self.prompt else { return };
        let k = smoothstep((now - self.prompt_since) / 0.24);
        cr.save().unwrap();
        cr.translate(0.0, (1.0 - k) * 7.0);
        if k < 0.999 { cr.push_group(); }
        menu_style::group(cr, RAIL, PROMPT_Y, INNER, PROMPT_H, 0.0, 1.0);
        let (title, description, code, field) = match prompt {
            Prompt::Wifi(d) => (d.name.as_str(), "Введите пароль сети Wi-Fi", "", true),
            Prompt::Pair(p) => ("Создание пары", p.text.as_str(), p.code.as_str(), p.input != Input::None),
        };
        let inset = RAIL + 18.0;
        let inner = INNER - 36.0;
        menu_style::label(cr, renderer, title, inset, PROMPT_Y + 18.0, inner, Font::Headline, 1.0);
        let wrapped = renderer.layout_ellipsized(cr, description, Font::Subhead, inner, 2);
        cr.set_source_rgba(1.0, 1.0, 1.0, menu_style::SECONDARY);
        cr.move_to(renderer.snap(inset), renderer.snap(PROMPT_Y + 42.0));
        pangocairo::functions::show_layout(cr, &wrapped);
        cr.new_path();
        if field {
            let fy = PROMPT_FIELD_Y;
            menu_style::field(cr, inset, fy, inner, 40.0, true, 1.0);
            let text = if matches!(prompt, Prompt::Wifi(_)) && !self.reveal {
                "•".repeat(self.input.chars().count())
            } else {
                self.input.clone()
            };
            let l = renderer.layout(cr, "", Font::Body).copy();
            l.set_text(&text);
            let (_, logical) = l.pixel_extents();
            let width = logical.width() as f64;
            let room = inner - 24.0 - 86.0;
            cr.save().unwrap();
            cr.rectangle(inset + 10.0, fy + 4.0, room + 4.0, 32.0);
            cr.clip();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            let ty = fy + (40.0 - logical.height() as f64) / 2.0;
            cr.move_to(renderer.snap(inset + 12.0 - (width - room).max(0.0)), renderer.snap(ty));
            pangocairo::functions::show_layout(cr, &l);
            let blink =
                if now - self.input_since < 0.7 { 1.0 } else { 0.5 + 0.5 * ((now - self.input_since) * 5.0).cos() };
            cr.new_path();
            cr.rectangle(inset + 13.0 + width.min(room), fy + 10.0, 2.0, 20.0);
            cr.set_source_rgba(menu_style::BLUE.0, menu_style::BLUE.1, menu_style::BLUE.2, blink);
            let _ = cr.fill();
            cr.restore().unwrap();
            if matches!(prompt, Prompt::Wifi(_)) {
                menu_style::button(cr, renderer, if self.reveal { "Скрыть" } else { "Показать" },
                    (inset + inner - 86.0, fy, 86.0, 40.0), ButtonKind::Plain, self.lit(Hit::Reveal, now), 1.0);
            }
        } else if !code.is_empty() {
            menu_style::centred(cr, renderer, code, (inset, PROMPT_FIELD_Y - 6.0, inner, 52.0), Font::Query, (1.0, 1.0, 1.0), 1.0);
        }
        let half = (inner - 10.0) / 2.0;
        menu_style::button(cr, renderer, "Отмена", (inset, PROMPT_BUTTONS_Y, half, menu_style::BUTTON_H),
            ButtonKind::Neutral, self.lit(Hit::Cancel, now), 1.0);
        menu_style::button(
            cr,
            renderer,
            if matches!(prompt, Prompt::Wifi(_)) { "Подключиться" } else { "Подтвердить" },
            (inset + half + 10.0, PROMPT_BUTTONS_Y, half, menu_style::BUTTON_H),
            ButtonKind::Primary,
            self.lit(Hit::Submit, now),
            if self.can_submit() { 1.0 } else { 0.4 },
        );
        if k < 0.999 {
            cr.pop_group_to_source().unwrap();
            let _ = cr.paint_with_alpha(k);
        }
        cr.restore().unwrap();
    }
}

fn contains(x: f64, y: f64, bx: f64, by: f64, w: f64, h: f64) -> bool {
    x >= bx && x <= bx + w && y >= by && y <= by + h
}
/// Wi-Fi strength: three arcs, as many lit as the signal earns.
fn strength(cr: &Context, cx: f64, cy: f64, signal: u8, alpha: f64) {
    let lit = if signal >= 67 { 3 } else if signal >= 34 { 2 } else { 1 };
    cr.save().unwrap();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_width(1.8);
    for (i, r) in [3.2, 6.6, 10.0].into_iter().enumerate() {
        cr.new_path();
        cr.arc(cx, cy + 5.0, r, 3.92, 5.50);
        cr.set_source_rgba(1.0, 1.0, 1.0, alpha * if i < lit { 0.9 } else { 0.22 });
        let _ = cr.stroke();
    }
    cr.new_path();
    cr.arc(cx, cy + 5.0, 1.2, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha * 0.9);
    let _ = cr.fill();
    cr.restore().unwrap();
}

/// A small padlock.
fn lock(cr: &Context, cx: f64, cy: f64, alpha: f64) {
    cr.save().unwrap();
    cr.new_path();
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    menu_style::rounded(cr, cx - 4.5, cy - 1.0, 9.0, 7.5, 1.8);
    let _ = cr.fill();
    cr.new_path();
    cr.set_line_width(1.5);
    cr.arc(cx, cy - 1.5, 3.0, std::f64::consts::PI, 0.0);
    let _ = cr.stroke();
    cr.restore().unwrap();
}

fn symbol(cr: &Context, kind: Kind, x: f64, y: f64, scale: f64, alpha: f64) {
    cr.save().unwrap();
    cr.translate(x, y);
    cr.scale(scale, scale);
    cr.new_path();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_width(1.7);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    if kind == Kind::Wifi {
        for r in [10.0, 6.5, 3.0] {
            cr.new_sub_path();
            cr.arc(0.0, 5.0, r, 3.90, 5.52);
        }
        let _ = cr.stroke();
        cr.new_path();
        cr.arc(0.0, 5.0, 1.1, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    } else {
        cr.move_to(0.0, -9.0);
        cr.line_to(5.0, -4.0);
        cr.line_to(-5.0, 5.0);
        cr.move_to(0.0, -9.0);
        cr.line_to(0.0, 9.0);
        cr.line_to(5.0, 4.0);
        cr.line_to(-5.0, -5.0);
        let _ = cr.stroke();
    }
    cr.restore().unwrap();
}
fn device_symbol(cr: &Context, icon: &str, x: f64, y: f64, alpha: f64) {
    cr.save().unwrap();
    cr.new_path();
    cr.set_line_width(1.6);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    if icon.contains("head") || icon.contains("audio") {
        cr.arc(x, y, 8.0, std::f64::consts::PI, std::f64::consts::TAU);
        let _ = cr.stroke();
        for dx in [-8.0, 8.0] {
            menu_style::rounded(cr, x + dx - 2.0, y - 1.0, 4.0, 9.0, 2.0);
            let _ = cr.fill();
        }
    } else if icon.contains("keyboard") {
        menu_style::rounded(cr, x - 10.0, y - 6.0, 20.0, 13.0, 3.0);
        let _ = cr.stroke();
        for row in 0..2 {
            for col in 0..4 {
                cr.rectangle(x - 7.0 + col as f64 * 4.0, y - 3.0 + row as f64 * 4.0, 1.7, 1.7);
            }
        }
        let _ = cr.fill();
    } else if icon.contains("mouse") {
        menu_style::rounded(cr, x - 6.0, y - 9.0, 12.0, 19.0, 6.0);
        let _ = cr.stroke();
        cr.move_to(x, y - 5.0);
        cr.line_to(x, y - 1.0);
        let _ = cr.stroke();
    } else {
        symbol(cr, Kind::Bluetooth, x, y, 0.8, alpha);
    }
    cr.restore().unwrap();
    cr.new_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, paired: bool) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            interface: "wlan0".into(),
            profile: None,
            connected: false,
            paired,
            security: "WPA2".into(),
            signal: 78,
            battery: None,
            icon: "audio-headphones".into(),
        }
    }
    fn panel(kind: Kind) -> Panel {
        let (tx, _) = smithay_client_toolkit::reexports::calloop::channel::channel();
        let mut p = Panel::idle(kind, 42, tx, 10.0);
        p.set_snapshot(
            Snapshot {
                powered: true,
                devices: vec![device("home", "Home Wi-Fi", true), device("guest", "Guest", false)],
            },
            10.0,
        );
        p
    }

    #[test]
    fn late_results_cannot_replace_newer_search_or_reopen_closed_sheet() {
        let mut p = panel(Kind::Wifi);
        let snapshot = Snapshot { powered: false, devices: vec![] };
        p.request = 3;
        assert!(!p.receive(Message { session: 42, request: 2, update: Update::Snapshot(snapshot.clone()) }, 11.0));
        assert!(!p.receive(Message { session: 41, request: 3, update: Update::Snapshot(snapshot.clone()) }, 11.0));
        assert!(p.snapshot.powered);
        p.close();
        assert!(!p.receive(Message { session: 42, request: 3, update: Update::Snapshot(snapshot) }, 12.0));
        assert!(p.cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn hover_interpolates_and_long_lists_remain_reachable() {
        let mut p = panel(Kind::Wifi);
        p.pointer(440.0, 30.0, 11.0);
        assert_eq!(p.lit(Hit::Close, 11.0), 0.0);
        assert!(p.lit(Hit::Close, 11.04) > 0.0 && p.lit(Hit::Close, 11.04) < 1.0);
        let before = p.lit(Hit::Close, 11.04);
        p.leave(11.04);
        assert!((p.lit(Hit::Close, 11.04) - before).abs() < 1e-9);
        assert!(p.lit(Hit::Close, 12.0) < 0.001);
        p.set_snapshot(
            Snapshot { powered: true, devices: (0..12).map(|i| device(&i.to_string(), "Network", false)).collect() },
            12.0,
        );
        p.scroll_by(50.0, 13.0);
        p.frame_height = p.height();
        assert_eq!(p.hit(100.0, LIST_Y + ROW_H * 4.0 + 10.0, 14.0), Some(Hit::Row("11".into())));
    }

    #[test]
    fn secured_wifi_opens_a_masked_prompt_and_pairing_is_rejected_on_close() {
        let mut p = panel(Kind::Wifi);
        p.click(100.0, LIST_Y + ROW_H + 10.0, 11.0);
        assert!(matches!(p.prompt, Some(Prompt::Wifi(_))));
        assert!(!p.reveal);
        p.key("", Some("пароль123"), false, 12.0);
        p.key("BackSpace", None, false, 12.1);
        assert_eq!(p.input, "пароль12");
        assert!(p.can_submit());
        let (tx, rx) = std::sync::mpsc::channel();
        p.prompt = Some(Prompt::Pair(network::Prompt {
            text: "Confirm".into(),
            code: "123456".into(),
            input: Input::None,
            reply: Some(tx),
        }));
        p.close();
        assert_eq!(rx.recv().unwrap(), None);
    }

    #[test]
    fn bluetooth_tabs_split_paired_devices_from_discovery() {
        let mut p = panel(Kind::Bluetooth);
        assert_eq!(p.rows().len(), 1);
        assert!(p.rows()[0].paired);
        p.tab = 1;
        assert_eq!(p.rows().len(), 1);
        assert!(!p.rows()[0].paired);
    }

    #[test]
    fn connection_sheets_render_at_fractional_scale() {
        let scale = 1.5;
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1530, 1530).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(scale, scale);
        cr.set_source_rgb(0.08, 0.09, 0.11);
        cr.paint().unwrap();
        let renderer = Renderer::new(scale);
        for index in 0..4 {
            let kind = if index % 2 == 0 { Kind::Wifi } else { Kind::Bluetooth };
            let mut p = panel(kind);
            p.snapshot.devices = if kind == Kind::Wifi {
                vec![
                    device("h", "Home Wi-Fi", true),
                    device("s", "Studio · 5 GHz", true),
                    device("g", "Guest", false),
                    device("n", "Neighbourhood network with a long name", false),
                ]
            } else {
                vec![
                    device("a", "AirPods Pro", true),
                    device("k", "Magic Keyboard", true),
                    device("m", "MX Master 3S", true),
                    device("b", "Beoplay H95", false),
                ]
            };
            p.snapshot.devices[0].connected = true;
            if kind == Kind::Bluetooth {
                p.snapshot.devices[0].battery = Some(84);
                p.snapshot.devices[1].icon = "input-keyboard".into();
                p.snapshot.devices[2].icon = "input-mouse".into();
            } else {
                p.snapshot.devices[2].security.clear();
            }
            p.update_slots(10.0);
            p.power.set(1.0, 10.0);
            if index == 2 {
                p.prompt = Some(Prompt::Wifi(p.snapshot.devices[1].clone()));
                p.prompt_since = 10.0;
                p.input = "preview-password".into();
            }
            if index == 3 {
                p.tab = 1;
                p.tab_slide.set(1.0, 10.0);
                p.slots.clear();
                p.update_slots(10.0);
                p.busy = true;
                p.scanning = true;
                p.status = "Идёт поиск поблизости…".into();
            }
            let x = 20.0 + (index % 2) as f64 * 500.0;
            let y = 12.0 + (index / 2) as f64 * 500.0;
            let height = p.height();
            menu_style::rounded(&cr, x, y, WIDTH, height, 30.0);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.fill().unwrap();
            p.draw(&cr, &renderer, x, y, height, 1.0, 11.0);
        }
        if std::env::var_os("ISLAND_CONNECTION_PREVIEW").is_some() {
            surface.write_to_png(&mut std::fs::File::create("target/connection-sheets.png").unwrap()).unwrap();
        }
    }
}
