//! Native Wi-Fi / Bluetooth sheets, including their interaction and motion.

use crate::anim::{Spring, Time};
use crate::island::{Font, Renderer, smoothstep};
use crate::network::{self, Action, Device, Input, Kind, Message, Snapshot, Update};
use cairo::Context;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const WIDTH: f64 = 480.0;
pub const HEIGHT: f64 = 482.0;
const LIST_Y: f64 = 132.0;
const ROW_H: f64 = 52.0;
const VISIBLE: usize = 5;

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
    scroll: Spring,
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
            scroll: Spring::new(0.0, 0.3, 1.0),
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
        if self.prompt.is_some() { 420.0 } else { LIST_Y + self.rows().len().clamp(3, VISIBLE) as f64 * ROW_H + 90.0 }
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
        if contains(x, y, 426.0, 24.0, 32.0, 32.0) {
            return Some(Hit::Close);
        }
        if self.prompt.is_some() {
            if contains(x, y, 40.0, 306.0, 194.0, 38.0) {
                return Some(Hit::Cancel);
            }
            if contains(x, y, 246.0, 306.0, 194.0, 38.0) && self.can_submit() {
                return Some(Hit::Submit);
            }
            if matches!(self.prompt, Some(Prompt::Wifi(_))) && contains(x, y, 375.0, 225.0, 62.0, 42.0) {
                return Some(Hit::Reveal);
            }
            return None;
        }
        if self.busy && !self.scanning {
            return None;
        }
        if contains(x, y, 369.0, 25.0, 46.0, 28.0) && self.loaded {
            return Some(Hit::Power);
        }
        if contains(x, y, 22.0, 86.0, 436.0, 34.0) {
            return Some(Hit::Tab(if x < WIDTH / 2.0 { 0 } else { 1 }));
        }
        if contains(x, y, 22.0, self.frame_height - 54.0, 436.0, 34.0)
            && (self.snapshot.powered || self.error.is_some())
        {
            return Some(Hit::Search);
        }
        if self.snapshot.powered
            && contains(x, y, 22.0, LIST_Y, 436.0, (self.frame_height - 90.0 - LIST_Y).max(0.0))
            && now - self.changed > 0.18
        {
            let offset = self.scroll.value(now);
            for row in self.rows() {
                let pos = self.slots.get(&row.id).map_or(0.0, |s| s.value(now));
                if contains(x, y, 22.0, LIST_Y + (pos - offset) * ROW_H, 436.0, ROW_H - 2.0) {
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
                    self.scroll = Spring::new(0.0, 0.3, 1.0);
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
                        self.scroll = Spring::new(0.0, 0.3, 1.0);
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
        rounded(cr, 22.0, 22.0, 36.0, 36.0, 12.0);
        cr.set_source_rgb(0.10, 0.52, 0.97);
        let _ = cr.fill();
        symbol(cr, self.kind, 40.0, 40.0, 1.0, 1.0);
        label(
            cr,
            renderer,
            if self.kind == Kind::Wifi { "Wi-Fi" } else { "Bluetooth" },
            72.0,
            19.0,
            Font::Text,
            278.0,
            1.0,
        );
        let subtitle = if !self.loaded {
            "Загружаем устройства…"
        } else if self.snapshot.powered {
            "Включён"
        } else {
            "Выключен"
        };
        label(cr, renderer, subtitle, 72.0, 43.0, Font::Tiny, 275.0, 0.5);
        let power = self.power.value(now).clamp(0.0, 1.0);
        rounded(cr, 369.0, 28.0, 42.0, 24.0, 12.0);
        cr.set_source_rgb(0.20 * (1.0 - power) + 0.10 * power, 0.20 + 0.28 * power, 0.22 + 0.74 * power);
        let _ = cr.fill();
        cr.new_path();
        cr.arc(381.0 + 18.0 * power, 40.0, 9.0 + self.lit(Hit::Power, now) * 0.5, 0.0, std::f64::consts::TAU);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.92);
        let _ = cr.fill();
        button(cr, renderer, "×", 426.0, 24.0, 32.0, 32.0, self.lit(Hit::Close, now), false, true);

        if self.prompt.is_some() {
            self.draw_prompt(cr, renderer, now);
        } else {
            rounded(cr, 22.0, 86.0, 436.0, 34.0, 11.0);
            cr.set_source_rgb(0.11, 0.11, 0.12);
            let _ = cr.fill();
            rounded(cr, 25.0 + self.tab_slide.value(now) * 216.0, 89.0, 214.0, 28.0, 8.0);
            cr.set_source_rgb(0.25, 0.25, 0.27);
            let _ = cr.fill();
            rounded(cr, 25.5 + self.tab_slide.value(now) * 216.0, 89.5, 213.0, 27.0, 7.5);
            cr.set_line_width(1.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.11);
            let _ = cr.stroke();
            let tabs = if self.kind == Kind::Wifi {
                ["Все сети", "Сохранённые"]
            } else {
                ["Мои устройства", "Поиск устройств"]
            };
            for (index, text) in tabs.iter().enumerate() {
                centered(
                    cr,
                    renderer,
                    text,
                    24.0 + index as f64 * 216.0,
                    88.0,
                    216.0,
                    30.0,
                    Font::Small,
                    if self.tab == index { 1.0 } else { 0.50 + self.lit(Hit::Tab(index), now) * 0.25 },
                );
            }
            let rows = self.rows();
            let fade = smoothstep((now - self.changed) / 0.24);
            if fade < 1.0 && !self.old_rows.is_empty() {
                cr.save().unwrap();
                cr.rectangle(20.0, LIST_Y, 440.0, (height - 90.0 - LIST_Y).max(0.0));
                cr.clip();
                for (i, d) in self.old_rows.iter().enumerate() {
                    self.draw_row(cr, renderer, d, LIST_Y + i as f64 * ROW_H - fade * 8.0, 1.0 - fade, now);
                }
                cr.restore().unwrap();
            }
            if rows.is_empty() || !self.snapshot.powered {
                let (title, hint) = if !self.loaded {
                    ("Загрузка…", "Получаем состояние адаптера")
                } else if !self.snapshot.powered {
                    ("Адаптер выключен", "Включите переключатель вверху")
                } else if self.busy {
                    ("Ищем поблизости", "Список обновится автоматически")
                } else if self.kind == Kind::Bluetooth && self.tab == 0 {
                    ("Пока нет устройств", "Откройте вкладку поиска для сопряжения")
                } else if self.kind == Kind::Bluetooth {
                    ("Устройства не найдены", "Включите на устройстве режим сопряжения")
                } else {
                    ("Сети не найдены", "Нажмите кнопку поиска ниже")
                };
                rounded(cr, 22.0, LIST_Y, 436.0, (height - 90.0 - LIST_Y).max(0.0), 16.0);
                cr.set_source_rgb(0.13, 0.13, 0.14);
                let _ = cr.fill();
                rounded(cr, 22.5, LIST_Y + 0.5, 435.0, (height - 91.0 - LIST_Y).max(0.0), 15.5);
                cr.set_line_width(1.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.06);
                let _ = cr.stroke();
                let center = LIST_Y + (height - 90.0 - LIST_Y) / 2.0;
                let appear = if self.old_rows.is_empty() { 1.0 } else { fade };
                symbol(cr, self.kind, WIDTH / 2.0, center - 24.0, 1.8, 0.3 * appear);
                centered(cr, renderer, title, 22.0, center + 10.0, 436.0, 25.0, Font::Text, 0.85 * appear);
                centered(cr, renderer, hint, 22.0, center + 40.0, 436.0, 24.0, Font::Small, 0.45 * appear);
            } else {
                cr.save().unwrap();
                cr.rectangle(20.0, LIST_Y, 440.0, (height - 90.0 - LIST_Y).max(0.0));
                cr.clip();
                for (i, d) in rows.iter().enumerate() {
                    let slot = self.slots.get(&d.id).map_or(i as f64, |s| s.value(now));
                    let ry = LIST_Y + (slot - self.scroll.value(now)) * ROW_H + (1.0 - fade) * 8.0;
                    if ry + ROW_H < LIST_Y || ry > LIST_Y + ROW_H * VISIBLE as f64 {
                        continue;
                    }
                    let appear = self.born.get(&d.id).map_or(1.0, |t| smoothstep((now - t) / 0.25));
                    self.draw_row(cr, renderer, d, ry, fade * appear, now);
                }
                cr.restore().unwrap();
                if rows.len() > VISIBLE {
                    let track = ROW_H * VISIBLE as f64;
                    let thumb = track * VISIBLE as f64 / rows.len() as f64;
                    let sy = LIST_Y + (track - thumb) * self.scroll.value(now) / (rows.len() - VISIBLE) as f64;
                    rounded(cr, 465.0, sy, 3.0, thumb, 1.5);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
                    let _ = cr.fill();
                }
            }
            let hint = if let Some(error) = &self.error {
                error.as_str()
            } else if self.busy {
                &self.status
            } else if self.kind == Kind::Wifi {
                "Клик по подключённой сети — отключить"
            } else {
                "Клик по подключённому устройству — отключить"
            };
            if self.error.is_some() {
                cr.set_source_rgb(1.0, 0.42, 0.38);
            } else {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.42);
            }
            let l = renderer.layout_ellipsized(cr, hint, Font::Tiny, 430.0, 2);
            cr.move_to(renderer.snap(22.0), renderer.snap(height - 85.0));
            pangocairo::functions::show_layout(cr, &l);
            let search = if self.scanning {
                "Остановить поиск"
            } else if self.busy {
                "Подождите…"
            } else if self.kind == Kind::Wifi {
                "Найти сети"
            } else {
                "Искать устройства"
            };
            button(
                cr,
                renderer,
                search,
                22.0,
                height - 54.0,
                436.0,
                34.0,
                self.lit(Hit::Search, now),
                false,
                (self.snapshot.powered || self.error.is_some()) && (!self.busy || self.scanning),
            );
            if self.busy {
                spinner(cr, 41.0, height - 37.0, now);
            }
        }
        if grouped {
            cr.pop_group_to_source().unwrap();
            let _ = cr.paint_with_alpha(alpha);
        }
        cr.restore().unwrap();
        cr.new_path();
    }

    fn draw_row(&self, cr: &Context, renderer: &Renderer, d: &Device, y: f64, alpha: f64, now: Time) {
        let hover = self.lit(Hit::Row(d.id.clone()), now);
        rounded(cr, 22.0, y, 436.0, ROW_H - 3.0, 12.0);
        if d.connected {
            cr.set_source_rgba(0.14 + 0.05 * hover, 0.22 + 0.05 * hover, 0.34 + 0.06 * hover, alpha);
        } else {
            cr.set_source_rgba(0.13 + 0.06 * hover, 0.13 + 0.06 * hover, 0.14 + 0.06 * hover, alpha);
        }
        let _ = cr.fill();
        rounded(cr, 22.5, y + 0.5, 435.0, ROW_H - 4.0, 11.5);
        cr.set_line_width(1.0);
        cr.set_source_rgba(if d.connected { 0.32 } else { 1.0 }, if d.connected { 0.66 } else { 1.0 },
            1.0, alpha * (if d.connected { 0.28 } else { 0.06 } + hover * 0.10));
        let _ = cr.stroke();
        if self.kind == Kind::Wifi {
            symbol(cr, Kind::Wifi, 43.0, y + 24.0, 0.86, alpha * if d.connected { 1.0 } else { 0.7 });
        } else {
            device_symbol(cr, &d.icon, 43.0, y + 24.0, alpha * if d.connected { 1.0 } else { 0.7 });
        }
        label(cr, renderer, &d.name, 67.0, y + 5.0, Font::Small, 252.0, alpha * 0.95);
        let detail = if self.kind == Kind::Wifi {
            format!(
                "{} · {}%{}",
                if d.connected {
                    "Подключено"
                } else if d.paired {
                    "Сохранено"
                } else {
                    "Доступно"
                },
                d.signal,
                if d.security.is_empty() { " · открытая" } else { " · защищённая" }
            )
        } else if d.name == crate::network::UNNAMED_BLUETOOTH {
            format!("{} · имя не передано", d.interface)
        } else {
            format!(
                "{}{}",
                if d.connected {
                    "Подключено"
                } else if d.paired {
                    "Сопряжено"
                } else {
                    "Новое устройство"
                },
                d.battery.map_or(String::new(), |v| format!(" · {v}%"))
            )
        };
        label(cr, renderer, &detail, 67.0, y + 27.0, Font::Tiny, 270.0, alpha * 0.43);
        let action = if d.connected {
            "Отключить"
        } else if self.kind == Kind::Bluetooth && !d.paired {
            "Сопрячь"
        } else {
            "Подключить"
        };
        rounded(cr, 341.0, y + 10.0, 101.0, 29.0, 9.0);
        if d.connected {
            cr.set_source_rgba(0.26, 0.54, 0.89, alpha * (0.25 + hover * 0.20));
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, alpha * (0.07 + hover * 0.12));
        }
        let _ = cr.fill();
        centered(cr, renderer, action, 342.0, y + 10.0, 99.0, 29.0, Font::Tiny, alpha * (0.66 + hover * 0.34));
    }

    fn draw_prompt(&self, cr: &Context, renderer: &Renderer, now: Time) {
        let Some(prompt) = &self.prompt else { return };
        let k = smoothstep((now - self.prompt_since) / 0.24);
        cr.save().unwrap();
        cr.translate(0.0, (1.0 - k) * 7.0);
        if k < 0.999 { cr.push_group(); }
        rounded(cr, 22.0, 95.0, 436.0, 262.0, 17.0);
        cr.set_source_rgb(0.13, 0.13, 0.14);
        let _ = cr.fill();
        rounded(cr, 22.5, 95.5, 435.0, 261.0, 16.5);
        cr.set_line_width(1.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.07);
        let _ = cr.stroke();
        let (title, description, code, field) = match prompt {
            Prompt::Wifi(d) => (d.name.as_str(), "Введите пароль сети Wi-Fi", "", true),
            Prompt::Pair(p) => ("Сопряжение", p.text.as_str(), p.code.as_str(), p.input != Input::None),
        };
        label(cr, renderer, title, 40.0, 115.0, Font::Text, 400.0, 1.0);
        label(cr, renderer, description, 40.0, 152.0, Font::Small, 400.0, 0.55);
        if field {
            rounded(cr, 40.0, 225.0, 400.0, 42.0, 11.0);
            cr.set_source_rgb(0.085, 0.085, 0.10);
            let _ = cr.fill();
            rounded(cr, 40.5, 225.5, 399.0, 41.0, 10.5);
            cr.set_source_rgba(0.24, 0.60, 1.0, 0.8);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let text = if matches!(prompt, Prompt::Wifi(_)) && !self.reveal {
                "•".repeat(self.input.chars().count())
            } else {
                self.input.clone()
            };
            let l = renderer.layout(cr, "", Font::Small).copy();
            l.set_text(&text);
            let width = l.pixel_extents().1.width() as f64;
            cr.save().unwrap();
            cr.rectangle(53.0, 232.0, 333.0, 26.0);
            cr.clip();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.move_to(renderer.snap(54.0 - (width - 319.0).max(0.0)), renderer.snap(237.0));
            pangocairo::functions::show_layout(cr, &l);
            let blink =
                if now - self.input_since < 0.7 { 1.0 } else { 0.5 + 0.5 * ((now - self.input_since) * 5.0).cos() };
            cr.new_path();
            cr.rectangle(55.0 + width.min(319.0), 237.0, 1.2, 17.0);
            cr.set_source_rgba(0.4, 0.7, 1.0, blink);
            let _ = cr.fill();
            cr.restore().unwrap();
            if matches!(prompt, Prompt::Wifi(_)) {
                centered(
                    cr,
                    renderer,
                    if self.reveal { "Скрыть" } else { "Показать" },
                    375.0,
                    225.0,
                    62.0,
                    42.0,
                    Font::Tiny,
                    0.55 + 0.45 * self.lit(Hit::Reveal, now),
                );
            }
        } else if !code.is_empty() {
            centered(cr, renderer, code, 40.0, 215.0, 400.0, 52.0, Font::Query, 1.0);
        }
        button(cr, renderer, "Отмена", 40.0, 306.0, 194.0, 38.0, self.lit(Hit::Cancel, now), false, true);
        button(
            cr,
            renderer,
            if matches!(prompt, Prompt::Wifi(_)) { "Подключиться" } else { "Подтвердить" },
            246.0,
            306.0,
            194.0,
            38.0,
            self.lit(Hit::Submit, now),
            true,
            self.can_submit(),
        );
        label(cr, renderer, "Enter — подтвердить · Escape — закрыть", 22.0, 371.0, Font::Tiny, 436.0, 0.36);
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
fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}
fn label(cr: &Context, renderer: &Renderer, text: &str, x: f64, y: f64, font: Font, w: f64, alpha: f64) {
    let l = renderer.layout_ellipsized(cr, text, font, w, 1);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.move_to(renderer.snap(x), renderer.snap(y));
    pangocairo::functions::show_layout(cr, &l);
    cr.new_path();
}
fn centered(cr: &Context, renderer: &Renderer, text: &str, x: f64, y: f64, w: f64, h: f64, font: Font, alpha: f64) {
    let l = renderer.layout_ellipsized(cr, text, font, w - 6.0, 1);
    let (_, e) = l.pixel_extents();
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.move_to(
        renderer.snap(x + (w - e.width() as f64) / 2.0 - e.x() as f64),
        renderer.snap(y + (h - e.height() as f64) / 2.0 - e.y() as f64),
    );
    pangocairo::functions::show_layout(cr, &l);
    cr.new_path();
}
fn button(
    cr: &Context,
    renderer: &Renderer,
    text: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    hover: f64,
    accent: bool,
    enabled: bool,
) {
    let a = if enabled { 1.0 } else { 0.32 };
    if text == "×" {
        crate::menu_style::close(cr, x, y, w, hover, a);
    } else {
        let kind = if accent { crate::menu_style::ButtonKind::Primary } else { crate::menu_style::ButtonKind::Neutral };
        crate::menu_style::button(cr, renderer, text, (x, y, w, h), Font::Small, kind, hover, a);
    }
}
fn spinner(cr: &Context, x: f64, y: f64, now: Time) {
    cr.save().unwrap();
    cr.new_path();
    cr.set_line_width(1.6);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.arc(x, y, 6.0, now * 5.0, now * 5.0 + 4.3);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.65);
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
            rounded(cr, x + dx - 2.0, y - 1.0, 4.0, 9.0, 2.0);
            let _ = cr.fill();
        }
    } else if icon.contains("keyboard") {
        rounded(cr, x - 10.0, y - 6.0, 20.0, 13.0, 3.0);
        let _ = cr.stroke();
        for row in 0..2 {
            for col in 0..4 {
                cr.rectangle(x - 7.0 + col as f64 * 4.0, y - 3.0 + row as f64 * 4.0, 1.7, 1.7);
            }
        }
        let _ = cr.fill();
    } else if icon.contains("mouse") {
        rounded(cr, x - 6.0, y - 9.0, 12.0, 19.0, 6.0);
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
            rounded(&cr, x, y, WIDTH, height, 30.0);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.fill().unwrap();
            p.draw(&cr, &renderer, x, y, height, 1.0, 11.0);
        }
        if std::env::var_os("ISLAND_CONNECTION_PREVIEW").is_some() {
            surface.write_to_png(&mut std::fs::File::create("target/connection-sheets.png").unwrap()).unwrap();
        }
    }
}
