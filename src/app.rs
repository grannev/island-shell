//! The Wayland side: one layer-shell surface on the overlay layer, wl_shm
//! buffers drawn with cairo at the output's fractional scale, frame
//! callbacks driving the animations at the predicted presentation time,
//! pointer hover, and the event loop sources (control socket, sway IPC, the
//! clock timer).

use std::collections::VecDeque;
use std::time::Duration;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    delegate_registry,
    output::{OutputHandler, OutputState},
    presentation_time::{PresentTime, PresentationTimeHandler, PresentationTimeState},
    reexports::{
        calloop::{
            EventLoop, LoopHandle, RegistrationToken,
            channel,
            generic::Generic,
            timer::{TimeoutAction, Timer},
            Interest, Mode, PostAction,
        },
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Dispatch, Proxy, QueueHandle, WEnum,
            backend::ObjectId,
            globals::{GlobalList, registry_queue_init},
            protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
        },
        protocols::wp::{
            fractional_scale::v1::client::{
                wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
                wp_fractional_scale_v1::{self, WpFractionalScaleV1},
            },
            presentation_time::client::wp_presentation_feedback::{self, WpPresentationFeedback},
            viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};

use crate::anim::Time;
use crate::ctl::{Command, Server};
use crate::dbus::{self, PlayerCommand};
use crate::eq::{self, Cava};
use crate::island::{Config, Frame, Island, Rect, Renderer};
use crate::notify;
use crate::sway;

/// Logical size of the (mostly transparent) surface: wide enough for the
/// dodge and the spread, tall enough for the Touch ID pill and the liquid
/// stretch.
const SURFACE_W: u32 = 980;
/// tall enough for the launcher panel as well (the surface is transparent
/// and its input region only ever covers the island itself)
const SURFACE_H: u32 = 560;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
/// How often the sink's mute state is checked while something plays.
const VOLUME_POLL: Duration = Duration::from_secs(2);
/// how often the open player asks where the track is
const POSITION_POLL: Duration = Duration::from_secs(1);
/// shm buffers kept in flight (the compositor releases them a frame later)
const MAX_BUFFERS: usize = 4;

/// Animation time base: the compositor's presentation clock and, from its
/// feedback, a prediction of when the frame being drawn will be shown.
struct FrameClock {
    clk_id: u32,
    /// display refresh period, s
    refresh: f64,
    last_present: Option<Time>,
    /// how many periods after the next vblank our commits actually land
    /// (learned: the compositor renders one frame ahead)
    pipeline: i32,
    errors: VecDeque<i32>,
    /// predicted presentation time of every commit awaiting feedback
    pending: VecDeque<(ObjectId, Time)>,
    /// ISLAND_DEBUG=1: report the prediction error of every frame
    debug: bool,
}

impl FrameClock {
    const MARGIN: f64 = 0.002;

    fn new() -> Self {
        FrameClock {
            clk_id: 1,
            refresh: 1.0 / 60.0,
            last_present: None,
            pipeline: 1, // sway renders one frame ahead; corrected if not
            errors: VecDeque::new(),
            pending: VecDeque::new(),
            debug: std::env::var_os("ISLAND_DEBUG").is_some(),
        }
    }

    fn now(&self) -> Time {
        unsafe extern "C" {
            fn clock_gettime(clk: i32, ts: *mut [i64; 2]) -> i32;
        }
        let mut ts = [0i64; 2];
        unsafe {
            clock_gettime(self.clk_id as i32, &mut ts);
        }
        ts[0] as f64 + ts[1] as f64 * 1e-9
    }

    /// When will a frame committed right now be on screen? `steady`: we are
    /// inside the frame-callback cadence, where the compositor is one frame
    /// ahead (`pipeline`); a frame that wakes it from idle is shown sooner.
    fn predict(&self, steady: bool) -> Time {
        let now = self.now();
        match self.last_present {
            Some(lp) => {
                let n = ((now + Self::MARGIN - lp) / self.refresh).ceil().max(1.0);
                let pipeline = if steady { self.pipeline } else { 0 };
                lp + (n + pipeline as f64) * self.refresh
            }
            None => now + self.refresh,
        }
    }

    fn committed(&mut self, feedback: &WpPresentationFeedback, predicted: Time, steady: bool) {
        // only steady frames teach the pipeline depth
        self.pending.push_back((feedback.id(), if steady { predicted } else { f64::NAN }));
        while self.pending.len() > 8 {
            self.pending.pop_front();
        }
    }

    fn presented(&mut self, feedback: &WpPresentationFeedback, actual: Time, refresh_ns: u32) {
        if refresh_ns > 0 {
            self.refresh = refresh_ns as f64 * 1e-9;
        }
        self.last_present = Some(actual);
        if let Some(i) = self.pending.iter().position(|(id, _)| *id == feedback.id()) {
            let (_, predicted) = self.pending.remove(i).unwrap();
            if predicted.is_nan() {
                return;
            }
            let err = ((actual - predicted) / self.refresh).round() as i32;
            if self.debug {
                eprintln!("frame: predicted {:+.2} ms off (pipeline {})", (actual - predicted) * 1e3, self.pipeline);
            }
            self.errors.push_back(err);
            if self.errors.len() > 3 {
                self.errors.pop_front();
            }
            // a consistent miss means the pipeline is deeper (or shallower)
            // than assumed: adjust once, then watch again
            if self.errors.len() == 3 && err != 0 && self.errors.iter().all(|e| *e == err) {
                self.pipeline = (self.pipeline + err).clamp(0, 3);
                self.errors.clear();
            }
        }
    }

    fn discarded(&mut self, feedback: &WpPresentationFeedback) {
        self.pending.retain(|(id, _)| *id != feedback.id());
    }
}

pub struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    presentation: PresentationTimeState,
    pool: SlotPool,
    layer: LayerSurface,
    viewport: WpViewport,
    _fractional: Option<WpFractionalScaleV1>,
    qh: QueueHandle<App>,
    loop_handle: LoopHandle<'static, App>,

    /// output pixels per logical pixel (wp_fractional_scale)
    scale: f64,
    width: u32,
    height: u32,
    configured: bool,
    frame_pending: bool,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    modifiers: Modifiers,
    kbd_focus: bool,
    /// where the pointer is, in logical px inside the surface
    pointer_at: (f64, f64),
    last_frame: Option<Frame>,
    /// the whole picture, always up to date
    canvas: Option<cairo::ImageSurface>,
    /// shm buffers, reused once the compositor releases them; each remembers
    /// where its content lags behind the canvas
    buffers: Vec<(Buffer, Option<Rect>)>,
    clock: FrameClock,
    /// the presentation time of the last frame we drew
    drawn_for: Option<Time>,

    pub island: Island,
    renderer: Renderer,
    sway_query: Option<sway::Query>,
    /// the equaliser feed, alive only while a player is playing
    cava: Option<Cava>,
    eq_tx: channel::Sender<[f32; eq::BANDS]>,
    /// the timer for the island's next time-driven change
    deadline: Option<(Time, RegistrationToken)>,
    volume_poll: Option<RegistrationToken>,
    /// while the player is open its position is asked for every second
    position_poll: Option<RegistrationToken>,
    dbus_tx: channel::Sender<dbus::Event>,
    pub exit: bool,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<App>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let presentation = PresentationTimeState::bind(&globals, &qh);
    let viewporter: WpViewporter = bind_global(&globals, &qh, 1)?;
    let fractional_mgr: Option<WpFractionalScaleManagerV1> = bind_global(&globals, &qh, 1).ok();
    if fractional_mgr.is_none() {
        eprintln!("island-shell: wp_fractional_scale_v1 not offered, rendering at scale 1");
    }

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("island-shell"), None);
    layer.set_anchor(Anchor::TOP);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(SURFACE_W, SURFACE_H);
    let viewport = viewporter.get_viewport(layer.wl_surface(), &qh, ());
    let fractional = fractional_mgr.map(|m| m.get_fractional_scale(layer.wl_surface(), &qh, ()));
    // an empty input region until the first frame says where the box is
    let empty = Region::new(&compositor)?;
    layer.set_input_region(Some(empty.wl_region()));
    layer.commit();

    let pool = SlotPool::new((SURFACE_W * SURFACE_H * 4 * 2) as usize, &shm)?;

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue).insert(loop_handle.clone())?;

    // D-Bus watchers (MPRIS players, fprintd) and the equaliser feed
    let (dbus_tx, dbus_rx) = channel::channel::<dbus::Event>();
    dbus::spawn_player_watch(dbus_tx.clone());
    dbus::spawn_auth_watch(dbus_tx.clone());
    let (eq_tx, eq_rx) = channel::channel::<[f32; eq::BANDS]>();
    let (note_tx, note_rx) = channel::channel::<notify::Notification>();
    notify::spawn_watch(note_tx);

    let mut island = Island::new(load_config());
    island.update_clock();

    let sway_query = match sway::Query::connect() {
        Ok(q) => Some(q),
        Err(e) => {
            eprintln!("island-shell: sway ipc unavailable ({e}); no fullscreen auto-hide");
            None
        }
    };

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        shm,
        presentation,
        pool,
        layer,
        viewport,
        _fractional: fractional,
        qh: qh.clone(),
        loop_handle: loop_handle.clone(),
        scale: 1.0,
        width: SURFACE_W,
        height: SURFACE_H,
        configured: false,
        frame_pending: false,
        pointer: None,
        keyboard: None,
        modifiers: Modifiers::default(),
        kbd_focus: false,
        pointer_at: (0.0, 0.0),
        last_frame: None,
        canvas: None,
        buffers: Vec::new(),
        clock: FrameClock::new(),
        drawn_for: None,
        island,
        renderer: Renderer::new(1.0),
        sway_query,
        cava: None,
        eq_tx,
        deadline: None,
        volume_poll: None,
        position_poll: None,
        dbus_tx,
        exit: false,
    };

    loop_handle.insert_source(dbus_rx, |event, _, app: &mut App| {
        let channel::Event::Msg(event) = event else { return };
        let now = app.frame_time();
        match event {
            dbus::Event::Player(np) => app.island.set_player(np.as_ref(), now),
            dbus::Event::AuthStarted => app.island.auth_started(now),
            dbus::Event::AuthStatus(status, done) => app.island.auth_status(&status, done, now),
            dbus::Event::Position(position) => app.island.set_position(position, now),
        }
        app.after_change();
    })?;
    loop_handle.insert_source(eq_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(levels) = event {
            app.island.set_eq(levels);
            app.redraw();
        }
    })?;
    loop_handle.insert_source(note_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(note) = event {
            let now = app.frame_time();
            app.island.notify(note, now);
            app.after_change();
        }
    })?;

    // control socket
    let server = Server::bind()?;
    loop_handle.insert_source(
        Generic::new(server, Interest::READ, Mode::Level),
        |_, server, app: &mut App| {
            server.accept_all(|cmd| app.handle_command(cmd));
            Ok(PostAction::Continue)
        },
    )?;

    // sway events: hide behind a fullscreen window
    if app.sway_query.is_some() {
        match sway::Events::subscribe(&["window", "workspace"]) {
            Ok(events) => {
                loop_handle.insert_source(
                    Generic::new(events, Interest::READ, Mode::Level),
                    |_, events, app: &mut App| {
                        // SAFETY: the stream is only read from, never removed here
                        let events = unsafe { events.get_mut() };
                        match events.drain() {
                            Ok(list) => {
                                let relevant = list.iter().any(|(_, change)| {
                                    matches!(change.as_str(), "fullscreen_mode" | "focus" | "close" | "new" | "move")
                                });
                                if relevant {
                                    app.check_fullscreen();
                                }
                                Ok(PostAction::Continue)
                            }
                            Err(e) => {
                                eprintln!("island-shell: sway ipc: {e}");
                                Ok(PostAction::Remove)
                            }
                        }
                    },
                )?;
                app.check_fullscreen();
            }
            Err(e) => eprintln!("island-shell: sway subscribe failed: {e}"),
        }
    }

    // the clock: fire just after every minute boundary
    loop_handle.insert_source(Timer::from_duration(until_next_minute()), |_, _, app: &mut App| {
        if app.island.update_clock() {
            app.redraw();
        }
        TimeoutAction::ToDuration(until_next_minute())
    })?;

    while !app.exit {
        event_loop.dispatch(None, &mut app)?;
    }
    Ok(())
}

/// Ask for the next frame callback on `layer`'s surface.
fn surface_frame(layer: &LayerSurface, qh: &QueueHandle<App>) {
    let surface = layer.wl_surface();
    surface.frame(qh, FrameCallbackData(surface.clone()));
    layer.commit();
}

fn read_muted() -> bool {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    if let Ok(v) = std::fs::read_to_string(dir.join("waybar-volume")) {
        let v = v.trim();
        return v == "muted" || v == "0";
    }
    let out = |args: &[&str]| {
        std::process::Command::new("pactl").args(args).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    };
    let muted = out(&["get-sink-mute", "@DEFAULT_SINK@"]).is_some_and(|s| s.contains("yes"));
    let zero = out(&["get-sink-volume", "@DEFAULT_SINK@"]).is_some_and(|s| s.split_whitespace().any(|w| w == "0%"));
    muted || zero
}

fn until_next_minute() -> Duration {
    let t = crate::island::libc_localtime();
    Duration::from_secs(60 - t.sec as u64) + Duration::from_millis(20)
}

fn config_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("island-shell").join("config")
}

/// `key = value` lines, `#` comments (see island::Config).
fn load_config() -> Config {
    let mut config = Config::default();
    let Ok(text) = std::fs::read_to_string(config_path()) else { return config };
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            eprintln!("island-shell: config: cannot parse {line:?}");
            continue;
        };
        if let Err(e) = config.set(key.trim(), value.trim()) {
            eprintln!("island-shell: config: {e}");
        }
    }
    config
}

fn bind_global<I>(globals: &GlobalList, qh: &QueueHandle<App>, version: u32) -> Result<I, Box<dyn std::error::Error>>
where
    I: Proxy + 'static,
    App: Dispatch<I, ()>,
{
    Ok(globals.bind::<I, App, ()>(qh, version..=version, ())?)
}

impl App {
    fn handle_command(&mut self, cmd: Command) -> String {
        let now = self.frame_time();
        match cmd {
            Command::Toggle => self.island.hidden.user = !self.island.hidden.user,
            Command::Show => self.island.hidden.user = false,
            Command::Hide => self.island.hidden.user = true,
            Command::Dodge => self.island.set_dodged(true, now),
            Command::Undodge => self.island.set_dodged(false, now),
            Command::DodgeToggle => {
                let dodged = self.island.dodged;
                self.island.set_dodged(!dodged, now);
            }
            Command::Status => {
                return format!(
                    "{} {} scale={} {}x{} refresh={:.3}ms pipeline={} buffers={} {} {}",
                    if self.island.is_hidden() { "hidden" } else { "shown" },
                    if self.island.dodged { "dodged" } else { "centred" },
                    self.scale,
                    self.width,
                    self.height,
                    self.clock.refresh * 1e3,
                    self.clock.pipeline,
                    self.buffers.len(),
                    self.island.config.describe(),
                    self.island.describe(now)
                );
            }
            Command::Set(key, value) => {
                if let Err(e) = self.island.config.set(&key, &value) {
                    return format!("error: {e}");
                }
            }
            Command::Launcher => {
                let open = self.island.toggle_launcher(now);
                self.set_keyboard_focus(open);
                self.after_change();
                return if open { "open".into() } else { "closed".into() };
            }
            Command::LauncherClose => {
                self.island.close_launcher(now);
                self.set_keyboard_focus(false);
                self.after_change();
                return "ok".into();
            }
            Command::Type(text) => {
                if let Some(l) = self.island.launcher.as_mut() {
                    l.push(&text, now);
                    self.after_change();
                    return "ok".into();
                }
                return "error: the launcher is not open".into();
            }
            Command::Key(name) => {
                let keysym = match name.as_str() {
                    "escape" => Keysym::Escape,
                    "return" | "enter" => Keysym::Return,
                    "up" => Keysym::Up,
                    "down" => Keysym::Down,
                    "backspace" => Keysym::BackSpace,
                    _ => return format!("error: unknown key {name:?}"),
                };
                self.launcher_key(&KeyEvent { time: 0, raw_code: 0, keysym, utf8: None });
                return "ok".into();
            }
            Command::Scrub(value) => {
                let sent = if value == "end" {
                    self.island.end_scrub(now)
                } else {
                    match value.parse::<f64>() {
                        Ok(fraction) => {
                            let fraction = fraction.clamp(0.0, 1.0);
                            if self.island.is_scrubbing() {
                                self.island.scrub_to(fraction, now)
                            } else {
                                self.island.press_control(crate::player::Control::Seek(fraction), now)
                            }
                        }
                        Err(_) => return format!("error: bad fraction {value:?}"),
                    }
                };
                if let Some((bus, track, command)) = sent {
                    dbus::control(&bus, &track, command);
                }
                self.after_change();
                return "ok".into();
            }
            Command::Demo(app, summary, body) => {
                let note = notify::fabricate(&app, &summary, &body);
                self.island.notify(note, now);
                self.after_change();
                return "ok".into();
            }
            Command::Dump(path) => {
                return match self.dump(&path) {
                    Ok(()) => "ok".into(),
                    Err(e) => format!("error: {e}"),
                };
            }
            Command::Quit => {
                self.exit = true;
                return "bye".into();
            }
        }
        self.island.sync(now);
        self.after_change();
        "ok".into()
    }

    fn check_fullscreen(&mut self) {
        let Some(q) = self.sway_query.as_mut() else { return };
        match q.fullscreen_focused() {
            Ok(fs) => {
                if fs != self.island.hidden.fullscreen {
                    self.island.hidden.fullscreen = fs;
                    let now = self.frame_time();
                    self.island.sync(now);
                    self.redraw();
                }
            }
            Err(e) => {
                eprintln!("island-shell: sway query: {e}");
                self.sway_query = None;
            }
        }
    }

    /// Render the island as it is right now into a PNG, off screen.
    fn dump(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.clock.now();
        self.island.advance(t);
        let bw = (self.width as f64 * self.scale).round() as i32;
        let bh = (self.height as f64 * self.scale).round() as i32;
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, bw, bh)?;
        {
            let cr = cairo::Context::new(&surface)?;
            cr.scale(self.scale, self.scale);
            self.island.draw(&cr, &self.renderer, self.width as f64 / 2.0, t);
        }
        let mut file = std::fs::File::create(path)?;
        surface.write_to_png(&mut file)?;
        Ok(())
    }

    /// The launcher needs the keyboard; nothing else does.
    fn set_keyboard_focus(&mut self, want: bool) {
        if want == self.kbd_focus {
            return;
        }
        self.kbd_focus = want;
        self.layer.set_keyboard_interactivity(if want {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        });
        self.layer.commit();
    }

    /// One key press (or repeat) while the launcher is open.
    fn launcher_key(&mut self, event: &KeyEvent) {
        let now = self.frame_time();
        let ctrl = self.modifiers.ctrl;
        let Some(l) = self.island.launcher.as_mut() else { return };
        match event.keysym {
            Keysym::Escape => {
                self.island.close_launcher(now);
                self.set_keyboard_focus(false);
            }
            Keysym::Return | Keysym::KP_Enter => {
                let launched = l.launch();
                self.island.close_launcher(now);
                self.set_keyboard_focus(false);
                if !launched {
                    eprintln!("island-shell: nothing to launch");
                }
            }
            Keysym::Down | Keysym::Tab => l.move_selection(1, now),
            Keysym::Up | Keysym::ISO_Left_Tab => l.move_selection(-1, now),
            Keysym::BackSpace => l.backspace(now),
            Keysym::u | Keysym::U if ctrl => l.clear(now),
            _ => match event.utf8.as_deref() {
                // printable text only (no control characters)
                Some(text) if !text.is_empty() && !text.chars().any(|c| c.is_control()) => l.push(text, now),
                _ => {}
            },
        }
        self.after_change();
    }

    /// Presentation time of the next frame we will draw.
    fn frame_time(&self) -> Time {
        self.clock.predict(self.frame_pending)
    }

    /// After the island's state changed: keep the helpers in step (cava
    /// and the mute poll while a song is shown), arm the timer for the
    /// next time-driven change, and draw.
    fn after_change(&mut self) {
        // whoever closed the launcher (a key, Touch ID, the compositor),
        // the keyboard goes back when it is gone
        let wants_keyboard = self.island.launcher.is_some();
        self.set_keyboard_focus(wants_keyboard);
        let has_song = self.island.song().is_some();
        if has_song && self.cava.is_none() {
            match Cava::start(self.eq_tx.clone()) {
                Ok(c) => self.cava = Some(c),
                Err(e) => eprintln!("island-shell: cava: {e}"),
            }
        } else if !has_song && self.cava.is_some() {
            self.cava = None;
        }
        let want_poll = has_song;
        if want_poll && self.volume_poll.is_none() {
            self.poll_volume();
            let token = self
                .loop_handle
                .insert_source(Timer::from_duration(VOLUME_POLL), |_, _, app: &mut App| {
                    app.poll_volume();
                    TimeoutAction::ToDuration(VOLUME_POLL)
                })
                .expect("volume timer");
            self.volume_poll = Some(token);
        } else if !want_poll {
            if let Some(token) = self.volume_poll.take() {
                self.loop_handle.remove(token);
            }
        }
        // the player's position: asked for while it is on screen
        let now = self.clock.now();
        match self.island.position_wanted(now) {
            Some(bus) if self.position_poll.is_none() => {
                dbus::spawn_position(&bus, self.dbus_tx.clone());
                let token = self
                    .loop_handle
                    .insert_source(Timer::from_duration(POSITION_POLL), |_, _, app: &mut App| {
                        let now = app.clock.now();
                        match app.island.position_wanted(now) {
                            Some(bus) => {
                                dbus::spawn_position(&bus, app.dbus_tx.clone());
                                TimeoutAction::ToDuration(POSITION_POLL)
                            }
                            None => {
                                app.position_poll = None;
                                TimeoutAction::Drop
                            }
                        }
                    })
                    .expect("position timer");
                self.position_poll = Some(token);
            }
            None => {
                if let Some(token) = self.position_poll.take() {
                    self.loop_handle.remove(token);
                }
            }
            _ => {}
        }
        self.arm_deadline();
        self.redraw();
    }

    /// Schedule `advance` for the island's next deadline.
    fn arm_deadline(&mut self) {
        let now = self.clock.now();
        let next = self.island.deadline(now);
        if self.deadline.as_ref().map(|(t, _)| *t) == next {
            return;
        }
        if let Some((_, token)) = self.deadline.take() {
            self.loop_handle.remove(token);
        }
        if let Some(t) = next {
            let delay = Duration::from_secs_f64((t - now).max(0.0)) + Duration::from_millis(5);
            let token = self
                .loop_handle
                .insert_source(Timer::from_duration(delay), |_, _, app: &mut App| {
                    app.deadline = None;
                    let now = app.frame_time();
                    app.island.advance(now);
                    app.after_change();
                    TimeoutAction::Drop
                })
                .expect("deadline timer");
            self.deadline = Some((t, token));
        }
    }

    /// Is the sink muted or at 0 %? The bar's volume script keeps a state
    /// file; pactl is the fallback.
    fn poll_volume(&mut self) {
        let muted = read_muted();
        if muted != self.island.muted {
            let now = self.frame_time();
            self.island.set_muted(muted, now);
            self.arm_deadline();
            self.redraw();
        }
    }

    /// Present a new frame now unless one is already on its way.
    pub fn redraw(&mut self) {
        if self.configured && !self.frame_pending {
            self.draw(false);
        }
    }

    /// Draw and commit a frame; `steady` when called from the frame callback.
    fn draw(&mut self, steady: bool) {
        let started = std::time::Instant::now();
        let t = self.clock.predict(steady);
        // one frame per refresh: the compositor hands out frame callbacks
        // more generously than the display can show them, and a second
        // frame for the same vblank is work nobody ever sees
        if self.drawn_for.is_some_and(|prev| t <= prev + self.clock.refresh * 0.5) {
            if !self.frame_pending {
                surface_frame(&self.layer, &self.qh);
                self.frame_pending = true;
            }
            return;
        }
        self.drawn_for = Some(t);
        if self.island.advance(t) {
            self.arm_deadline();
        }
        let bw = (self.width as f64 * self.scale).round() as i32;
        let bh = (self.height as f64 * self.scale).round() as i32;
        let stride = bw * 4;
        let whole = Rect { x: 0.0, y: 0.0, w: self.width as f64, h: self.height as f64 };

        if !self.canvas.as_ref().is_some_and(|c| c.width() == bw && c.height() == bh) {
            self.canvas = Some(cairo::ImageSurface::create(cairo::Format::ARgb32, bw, bh).expect("cairo surface"));
            self.buffers.clear();
            self.last_frame = None;
        }
        let canvas = self.canvas.clone().unwrap();
        let (frame, changed) = {
            let cr = cairo::Context::new(&canvas).expect("cairo context");
            cr.scale(self.scale, self.scale);
            // wipe where the previous frame painted (the rest is still clear)
            let stale = match self.last_frame {
                None => Some(whole),
                Some(prev) => prev.bounds,
            };
            if let Some(r) = stale {
                cr.set_operator(cairo::Operator::Clear);
                cr.rectangle(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0);
                let _ = cr.fill();
                cr.set_operator(cairo::Operator::Over);
            }
            let t_paint = std::time::Instant::now();
            let frame = self.island.draw(&cr, &self.renderer, self.width as f64 / 2.0, t);
            if self.clock.debug {
                eprintln!("paint: {:.0} us", t_paint.elapsed().as_secs_f64() * 1e6);
            }
            let changed = match (stale, frame.bounds) {
                (Some(a), Some(b)) => Some(a.union(b)),
                (a, b) => a.or(b),
            };
            (frame, changed)
        };
        canvas.flush();
        let t_copy = std::time::Instant::now();

        // every buffer now lags behind the canvas in `changed` too
        for (_, lag) in &mut self.buffers {
            *lag = match (*lag, changed) {
                (Some(a), Some(b)) => Some(a.union(b)),
                (a, b) => a.or(b),
            };
        }
        // a free buffer (released by the compositor), or a new one — but
        // never more than a small pool: if the compositor is holding them
        // all, drop this frame instead of allocating another megabyte
        let idx = match self.buffers.iter().position(|(b, _)| self.pool.canvas(b).is_some()) {
            Some(i) => i,
            None if self.buffers.len() < MAX_BUFFERS => {
                let (buffer, _) = self.pool.create_buffer(bw, bh, stride, wl_shm::Format::Argb8888).expect("shm buffer");
                self.buffers.push((buffer, Some(whole)));
                self.buffers.len() - 1
            }
            None => {
                // try again on the next frame callback
                if self.clock.debug {
                    eprintln!("stall: no free buffer");
                }
                if !self.frame_pending {
                    surface_frame(&self.layer, &self.qh);
                    self.frame_pending = true;
                }
                return;
            }
        };
        let lag = self.buffers[idx].1.take();
        let damage = lag.map(|d| {
            let s = self.scale;
            let x0 = ((d.x - 1.0) * s).floor().max(0.0) as usize;
            let y0 = ((d.y - 1.0) * s).floor().max(0.0) as usize;
            let x1 = (((d.x + d.w + 1.0) * s).ceil() as usize).min(bw as usize);
            let y1 = (((d.y + d.h + 1.0) * s).ceil() as usize).min(bh as usize);
            (x0, y0, x1, y1)
        });
        if let Some((x0, y0, x1, y1)) = damage {
            if x1 > x0 && y1 > y0 {
                let shm_canvas = self.pool.canvas(&self.buffers[idx].0).expect("free buffer");
                let cairo_stride = canvas.stride() as usize;
                canvas
                    .with_data(|src| {
                        for y in y0..y1 {
                            let (a, b) = (y * cairo_stride + x0 * 4, y * cairo_stride + x1 * 4);
                            let (c, d) = (y * stride as usize + x0 * 4, y * stride as usize + x1 * 4);
                            shm_canvas[c..d].copy_from_slice(&src[a..b]);
                        }
                    })
                    .expect("cairo surface data");
            }
        }
        if self.clock.debug {
            if let Some((x0, y0, x1, y1)) = damage {
                eprintln!("copy: {:.0} us ({}x{})", t_copy.elapsed().as_secs_f64() * 1e6, x1 - x0, y1 - y0);
            }
        }
        let buffer = &self.buffers[idx].0;

        let surface = self.layer.wl_surface();
        // input only over the box (the rest of the surface is empty air),
        // opaque where the box is solid, damage where anything changed
        if self.last_frame.map(|f| f.bounds) != Some(frame.bounds) {
            let region = Region::new(&self.compositor).expect("wl_region");
            if let Some(r) = frame.bounds {
                region.add(r.x.floor() as i32, r.y.floor() as i32, r.w.ceil() as i32 + 1, r.h.ceil() as i32 + 1);
            }
            self.layer.set_input_region(Some(region.wl_region()));
        }
        if self.last_frame.map(|f| f.opaque) != Some(frame.opaque) {
            let region = Region::new(&self.compositor).expect("wl_region");
            for r in frame.opaque.iter().flatten() {
                // inward rounding: only whole covered pixels are opaque
                let (x0, y0) = (r.x.ceil() as i32, r.y.ceil() as i32);
                let (x1, y1) = ((r.x + r.w).floor() as i32, (r.y + r.h).floor() as i32);
                if x1 > x0 && y1 > y0 {
                    region.add(x0, y0, x1 - x0, y1 - y0);
                }
            }
            self.layer.set_opaque_region(Some(region.wl_region()));
        }
        if let Some((x0, y0, x1, y1)) = damage {
            surface.damage_buffer(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32);
        }
        self.last_frame = Some(frame);

        self.viewport.set_source(0.0, 0.0, bw as f64, bh as f64);
        self.viewport.set_destination(self.width as i32, self.height as i32);
        if self.island.is_animating(t) {
            surface.frame(&self.qh, FrameCallbackData(surface.clone()));
            self.frame_pending = true;
        }
        if let Ok(feedback) = self.presentation.feedback(surface, &self.qh) {
            self.clock.committed(&feedback, t, steady);
        }
        buffer.attach_to(surface).expect("attach");
        self.layer.commit();
        if self.clock.debug {
            eprintln!("draw: {:.0} us steady={}", started.elapsed().as_secs_f64() * 1e6, steady as u8);
        }
    }

    fn set_scale(&mut self, scale: f64) {
        if (scale - self.scale).abs() < 1e-6 {
            return;
        }
        self.scale = scale;
        self.renderer = Renderer::new(scale);
        self.last_frame = None;
        self.redraw();
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _factor: i32) {
        // integer scale from wl_output: the fractional protocol supersedes it
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _time: u32) {
        self.frame_pending = false;
        self.draw(true);
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl PresentationTimeHandler for App {
    fn presentation_time_state(&mut self) -> &mut PresentationTimeState {
        &mut self.presentation
    }

    #[allow(clippy::too_many_arguments)]
    fn presented(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        feedback: &WpPresentationFeedback,
        _: &wl_surface::WlSurface,
        _: Vec<wl_output::WlOutput>,
        time: PresentTime,
        refresh: u32,
        _seq: u64,
        _flags: WEnum<wp_presentation_feedback::Kind>,
    ) {
        self.clock.clk_id = time.clk_id;
        let actual = time.tv_sec as f64 + time.tv_nsec as f64 * 1e-9;
        self.clock.presented(feedback, actual, refresh);
    }

    fn discarded(&mut self, _: &Connection, _: &QueueHandle<Self>, feedback: &WpPresentationFeedback, _: &wl_surface::WlSurface) {
        self.clock.discarded(feedback);
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        let (w, h) = configure.new_size;
        if w != 0 {
            self.width = w;
        }
        if h != 0 {
            self.height = h;
        }
        self.configured = true;
        self.last_frame = None;
        // draw straight away (the first frame, or a new size); a pending
        // frame callback will draw again, which is harmless
        self.draw(false);
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let handle = self.loop_handle.clone();
            self.keyboard = self
                .seat_state
                .get_keyboard_with_repeat(qh, &seat, None, handle, Box::new(|app: &mut App, _kbd, event| app.launcher_key(&event)))
                .map_err(|e| eprintln!("island-shell: keyboard: {e}"))
                .ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        let mut changed = false;
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    self.island.hovered = true;
                    self.pointer_at = event.position;
                    changed = true;
                }
                PointerEventKind::Leave { .. } => {
                    self.island.hovered = false;
                    if !self.island.is_scrubbing() {
                        self.island.set_hovered_control(None);
                    }
                    changed = true;
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_at = event.position;
                    if self.hover_launcher_row() {
                        changed = true;
                    }
                    let now = self.clock.predict(false);
                    // dragging the playhead: the pointer keeps its grab
                    // even when it wanders off the island
                    if self.island.is_scrubbing() {
                        let fraction = self.island.scrub_fraction(self.width as f64 / 2.0, event.position.0, now);
                        if let Some((bus, track, command)) = self.island.scrub_to(fraction, now) {
                            dbus::control(&bus, &track, command);
                        }
                        changed = true;
                        continue;
                    }
                    let control = self.island.control_at(self.width as f64 / 2.0, event.position.0, event.position.1, now);
                    if self.island.set_hovered_control(control) {
                        changed = true;
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if button == BTN_LEFT && self.island.is_scrubbing() {
                        let now = self.clock.predict(false);
                        if let Some((bus, track, command)) = self.island.end_scrub(now) {
                            dbus::control(&bus, &track, command);
                        }
                        changed = true;
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    let now = self.clock.predict(false);
                    // inside the launcher a click launches the row under
                    // the pointer
                    if self.island.launcher.is_some() {
                        self.pointer_at = event.position;
                        self.hover_launcher_row();
                        if button == BTN_LEFT {
                            if let Some(l) = self.island.launcher.as_ref() {
                                l.launch();
                            }
                            self.island.close_launcher(now);
                            self.set_keyboard_focus(false);
                        }
                        changed = true;
                        continue;
                    }
                    // inside the open player the buttons and the progress
                    // line take the click
                    if let Some(control) = self.island.control_at(self.width as f64 / 2.0, event.position.0, event.position.1, now) {
                        if button == BTN_LEFT {
                            if let Some((bus, track, command)) = self.island.press_control(control, now) {
                                dbus::control(&bus, &track, command);
                            }
                            changed = true;
                            continue;
                        }
                    }
                    // a click on a notification dismisses it
                    if self.island.dismiss_notice(now) {
                        changed = true;
                        continue;
                    }
                    let cmd = match button {
                        BTN_LEFT => Some(PlayerCommand::PlayPause),
                        BTN_RIGHT => Some(PlayerCommand::Next),
                        BTN_MIDDLE => Some(PlayerCommand::Previous),
                        _ => None,
                    };
                    if let Some(cmd) = cmd {
                        self.player_control(cmd);
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let v = if vertical.discrete != 0 { vertical.discrete as f64 } else { vertical.absolute };
                    let now = self.clock.predict(false);
                    match self.island.launcher.as_mut() {
                        // inside the launcher the wheel scrolls the list
                        Some(l) => {
                            let rows = if vertical.discrete != 0 { v } else { v / 40.0 };
                            if l.scroll_by(rows, now) {
                                changed = true;
                            }
                        }
                        // on the line it skips tracks (as on the old touch target)
                        None if v < 0.0 => self.player_control(PlayerCommand::Next),
                        None if v > 0.0 => self.player_control(PlayerCommand::Previous),
                        None => {}
                    }
                }
            }
        }
        if changed {
            let now = self.frame_time();
            self.island.sync(now);
            self.redraw();
        }
    }
}

impl App {
    /// Point the launcher's selection at the row under the pointer.
    fn hover_launcher_row(&mut self) -> bool {
        let now = self.clock.predict(false);
        let (bx, by) = self.island.box_origin(self.width as f64 / 2.0, now);
        let (px, py) = self.pointer_at;
        let Some(l) = self.island.launcher.as_mut() else { return false };
        if px < bx || px > bx + crate::launcher::PANEL_W {
            return false;
        }
        match l.row_at(by, py, now) {
            Some(index) => {
                l.select_index(index, now);
                true
            }
            None => false,
        }
    }

    fn player_control(&self, cmd: PlayerCommand) {
        if self.island.accepts_player_clicks() {
            if let Some(song) = self.island.song() {
                dbus::control(&song.now.bus_name, &song.now.track_id, cmd);
            }
        }
    }
}

impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {
        // the compositor took the keyboard away (another window, a lock):
        // the launcher has no business staying open without it
        let now = self.frame_time();
        if self.island.launcher.is_some() {
            self.island.close_launcher(now);
            self.after_change();
        }
    }

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.launcher_key(&event);
    }

    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.launcher_key(&event);
    }

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.modifiers = modifiers;
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch<WpViewporter, ()> for App {
    fn event(_: &mut Self, _: &WpViewporter, _: <WpViewporter as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpViewport, ()> for App {
    fn event(_: &mut Self, _: &WpViewport, _: <WpViewport as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpFractionalScaleManagerV1, ()> for App {
    fn event(_: &mut Self, _: &WpFractionalScaleManagerV1, _: <WpFractionalScaleManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpFractionalScaleV1, ()> for App {
    fn event(app: &mut Self, _: &WpFractionalScaleV1, event: wp_fractional_scale_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            app.set_scale(scale as f64 / 120.0);
        }
    }
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(App);
