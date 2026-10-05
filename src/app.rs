//! The Wayland side: one layer-shell surface on the overlay layer, wl_shm
//! buffers drawn with cairo at the output's fractional scale, frame
//! callbacks driving the animations at the predicted presentation time,
//! pointer hover, and the event loop sources (control socket, sway IPC, the
//! clock timer).

use std::collections::VecDeque;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region, SurfaceData},
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
use crate::activity;
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
/// tall enough for the launcher panel and the tallest sheet (the Control
/// Center with "Now playing", the Sound page with four applications); the
/// surface is transparent and its input region only ever covers the
/// island itself
const SURFACE_H: u32 = 640;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyClick { Menu, PlayPause, Next, Previous, None }

fn body_click(button: u32, over_music: bool, can_open_menu: bool) -> BodyClick {
    if over_music {
        match button {
            BTN_LEFT => BodyClick::PlayPause,
            BTN_RIGHT => BodyClick::Next,
            BTN_MIDDLE => BodyClick::Previous,
            _ => BodyClick::None,
        }
    } else if button == BTN_LEFT && can_open_menu {
        BodyClick::Menu
    } else {
        BodyClick::None
    }
}
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

/// A single transparent pixel stretched over the output. Its input region
/// excludes the island; it exists only while a modal menu is open.
struct DismissSurface {
    layer: LayerSurface,
    viewport: WpViewport,
    buffer: Buffer,
    size: (u32, u32),
    input_hole: std::cell::Cell<Option<Option<(i32, i32, i32, i32)>>>,
}

#[derive(Default)]
struct FrameStats {
    count: u64,
    slow: u64,
    total: f64,
    worst: f64,
}

impl FrameStats {
    fn record(&mut self, seconds: f64, refresh: f64) {
        self.count += 1;
        self.slow += u64::from(seconds > refresh);
        self.total += seconds;
        self.worst = self.worst.max(seconds);
    }
    fn describe(&self) -> String {
        format!(" frame_cpu_ms={:.2}/{:.2} slow_frames={}/{}",
            self.total * 1000.0 / self.count.max(1) as f64, self.worst * 1000.0, self.slow, self.count)
    }
}

impl Drop for DismissSurface {
    fn drop(&mut self) {
        self.viewport.destroy();
    }
}

fn dismiss_hole(input: Rect, output_width: u32, surface_width: u32) -> (i32, i32, i32, i32) {
    let x = (output_width as f64 - surface_width as f64) / 2.0 + input.x;
    (x.floor() as i32, input.y.floor() as i32, input.w.ceil() as i32 + 1, input.h.ceil() as i32 + 1)
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
    layer_shell: LayerShell,
    viewporter: WpViewporter,
    dismiss: Option<DismissSurface>,
    viewport: WpViewport,
    fractional: Option<WpFractionalScaleV1>,
    fractional_mgr: Option<WpFractionalScaleManagerV1>,
    /// the output the island's surface was created on (None: not chosen yet,
    /// or every output is gone)
    bound_output: Option<wl_output::WlOutput>,
    /// sway's focused output, for `output = focused`
    focused_output: Option<String>,
    clock_timer: Option<RegistrationToken>,
    qh: QueueHandle<App>,
    loop_handle: LoopHandle<'static, App>,

    /// output pixels per logical pixel (wp_fractional_scale)
    scale: f64,
    width: u32,
    height: u32,
    configured: bool,
    frame_pending: bool,
    frame_stats: FrameStats,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    modifiers: Modifiers,
    kbd_interactivity: KeyboardInteractivity,
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
    /// the token the Neovim plugin has to quote
    nvim_token: String,

    pub island: Island,
    renderer: Renderer,
    sway_query: Option<sway::Query>,
    /// the equaliser feed, alive only while a player is playing
    cava: Option<Cava>,
    cava_start: crate::background::Worker<(), Result<Cava, String>>,
    cava_pending: bool,
    cava_retry: std::time::Instant,
    volume_read: crate::background::Worker<(), bool>,
    volume_pending: bool,
    /// the timer for the island's next time-driven change
    deadline: Option<(Time, RegistrationToken)>,
    volume_poll: Option<RegistrationToken>,
    /// while the player is open its position is asked for every second
    position_poll: Option<RegistrationToken>,
    dbus_tx: channel::Sender<dbus::Event>,
    network_tx: channel::Sender<crate::network::Message>,
    notification_commands: std::sync::mpsc::Sender<crate::notification_server::Command>,
    notification_dnd: Arc<AtomicBool>,
    /// the Control Center's worker (volume, brightness, outputs, layout,
    /// session) and whether the menu was open at the last change
    controls: std::sync::mpsc::Sender<crate::control::Request>,
    /// screen recordings report through here
    recorder_tx: channel::Sender<crate::recorder::Event>,
    quick_was_open: bool,
    /// refreshes the sound page's applications while it is open
    sound_poll: Option<RegistrationToken>,
    task_poll: Option<RegistrationToken>,
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

    // no output yet: the outputs' names arrive with the first dispatch,
    // and the island then moves to the one the config asks for
    let (layer, viewport, fractional) = create_island_surface(
        &compositor, &layer_shell, &viewporter, fractional_mgr.as_ref(), &qh, None, KeyboardInteractivity::None,
    );

    let pool = SlotPool::new((SURFACE_W * SURFACE_H * 4 * 2) as usize, &shm)?;

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue).insert(loop_handle.clone())?;

    // D-Bus watchers (MPRIS players, fprintd) and the equaliser feed
    let (dbus_tx, dbus_rx) = channel::channel::<dbus::Event>();
    dbus::spawn_player_watch(dbus_tx.clone());
    dbus::spawn_auth_watch(dbus_tx.clone());
    let (eq_tx, eq_rx) = channel::channel::<[f32; eq::BANDS]>();
    let (note_tx, note_rx) = channel::channel::<crate::notification_server::Event>();
    let notification_dnd = Arc::new(AtomicBool::new(crate::notice_center::load_dnd()));
    let notification_commands = crate::notification_server::spawn(note_tx, notification_dnd.clone());
    let (activity_tx, activity_rx) = channel::channel::<activity::Event>();
    activity::spawn_watchers(activity_tx.clone());
    let (network_tx, network_rx) = channel::channel::<crate::network::Message>();
    let (control_tx, control_rx) = channel::channel::<crate::control::Snapshot>();
    let controls = crate::control::spawn(control_tx);
    // the island starts knowing the environment (night mode, the layout)
    let _ = controls.send(crate::control::Request::Refresh);
    let (recorder_tx, recorder_rx) = channel::channel::<crate::recorder::Event>();

    let mut island = Island::new(load_config());
    island.update_clock();
    let (asset_tx, asset_rx) = channel::channel::<()>();
    island.set_asset_waker(asset_tx.clone());
    let eq_sender = eq_tx.clone();
    let cava_start = crate::background::Worker::new("audio-start", move |_| Cava::start(eq_sender.clone()).map_err(|e| e.to_string()));
    cava_start.set_waker(asset_tx.clone());
    let volume_read = crate::background::Worker::new("audio-state", |_| read_muted());
    volume_read.set_waker(asset_tx);

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
        layer_shell,
        viewporter,
        dismiss: None,
        viewport,
        fractional,
        fractional_mgr,
        bound_output: None,
        focused_output: None,
        clock_timer: None,
        qh: qh.clone(),
        loop_handle: loop_handle.clone(),
        scale: 1.0,
        width: SURFACE_W,
        height: SURFACE_H,
        configured: false,
        frame_pending: false,
        frame_stats: FrameStats::default(),
        pointer: None,
        keyboard: None,
        modifiers: Modifiers::default(),
        kbd_interactivity: KeyboardInteractivity::None,
        pointer_at: (0.0, 0.0),
        last_frame: None,
        canvas: None,
        buffers: Vec::new(),
        clock: FrameClock::new(),
        drawn_for: None,
        nvim_token: crate::nvim::token(),
        island,
        renderer: Renderer::new(1.0),
        sway_query,
        cava: None,
        cava_start,
        cava_pending: false,
        cava_retry: std::time::Instant::now(),
        volume_read,
        volume_pending: false,
        deadline: None,
        volume_poll: None,
        position_poll: None,
        dbus_tx,
        network_tx,
        notification_commands,
        notification_dnd,
        controls,
        recorder_tx,
        quick_was_open: false,
        sound_poll: None,
        task_poll: None,
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
    loop_handle.insert_source(asset_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(()) = event {
            let now = app.frame_time();
            let mut changed = app.island.drain_assets(now, app.scale);
            for (_, result) in app.cava_start.drain() {
                app.cava_pending = false;
                match result {
                    Ok(cava) if app.island.song().is_some() => app.cava = Some(cava),
                    Ok(_) => {},
                    Err(error) => {
                        eprintln!("island-shell: cava: {error}");
                        app.cava_retry = std::time::Instant::now() + Duration::from_secs(5);
                    }
                }
                changed = true;
            }
            for (_, muted) in app.volume_read.drain() {
                app.volume_pending = false;
                if app.island.song().is_some() && muted != app.island.muted {
                    app.island.set_muted(muted, now);
                    changed = true;
                }
            }
            if changed { app.after_change(); }
        }
    })?;
    // A very fast worker may finish before its waker was attached.
    app.island.drain_assets(app.frame_time(), app.scale);
    loop_handle.insert_source(eq_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(levels) = event {
            let now = app.frame_time();
            if app.island.set_eq(levels, now) {
                app.redraw();
            }
        }
    })?;
    loop_handle.insert_source(note_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(event) = event {
            let now = app.frame_time();
            match event {
                crate::notification_server::Event::Upsert(note) => app.island.notify(note, now),
                crate::notification_server::Event::Closed(id) => app.island.notification_closed(id, now),
            }
            app.after_change();
        }
    })?;
    loop_handle.insert_source(activity_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(event) = event {
            let now = app.frame_time();
            if let activity::Event::Slider { kind: activity::SliderKind::Volume, muted, .. } = &event {
                app.island.set_muted(*muted, now);
            }
            app.island.live_activity(event, now);
            app.after_change();
        }
    })?;

    loop_handle.insert_source(recorder_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(event) = event {
            let now = app.frame_time();
            app.island.recorder_event(event, now);
            app.after_change();
        }
    })?;

    loop_handle.insert_source(control_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(snapshot) = event {
            let now = app.frame_time();
            app.island.set_controls(snapshot, now);
            app.after_change();
        }
    })?;
    if let Some(layout) = app.sway_query.as_mut().and_then(|q| q.keyboard_layout().ok().flatten()) {
        app.island.controls.layout = Some(layout);
    }

    loop_handle.insert_source(network_rx, |event, _, app: &mut App| {
        if let channel::Event::Msg(message) = event {
            let now = app.frame_time();
            if app.island.network_message(message, now) { app.after_change(); }
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
        match sway::Events::subscribe(&["window", "workspace", "input"]) {
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
                                if list.iter().any(|(_, change)| change == "xkb_layout") {
                                    app.layout_changed();
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

    app.arm_clock();

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

/// Turn an editor toast into the notification the pill already knows how
/// to show.
fn editor_toast(kind: crate::nvim::ToastKind, title: String, body: String, progress: Option<f64>) -> notify::Notification {
    use crate::nvim::ToastKind;
    let critical = kind == ToastKind::Error;
    let app = match kind {
        ToastKind::Done => "Done",
        ToastKind::Error => "Error",
        ToastKind::Warning => "Warning",
        ToastKind::Git => "Git",
        _ => "Neovim",
    };
    // `vim.notify` hands us its own title, which is often just the editor's
    // name: no point saying it twice
    let (summary, body) = if title == app || title.is_empty() { (body, String::new()) } else { (title, body) };
    notify::Notification {
        id: 0,
        app: app.to_string(),
        summary,
        body,
        kind: if critical { crate::notify::AppKind::Critical } else { crate::notify::AppKind::Editor },
        critical,
        tag: Some(match kind {
            ToastKind::Progress => "nvim.progress".into(),
            _ => "nvim".into(),
        }),
        progress,
        actions: Vec::new(),
        transient: false,
        reply: None,
        icon: None,
        image: None,
    }
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

/// How often the open sound page re-reads the playing applications.
const SOUND_POLL: Duration = Duration::from_millis(1000);
/// how often running jobs' owners are checked
const TASK_POLL: Duration = Duration::from_millis(1000);

/// Logical pixels a notch of the wheel scrolls a list: about one row, as
/// GTK and browsers do for a list this size.
const WHEEL_PX: f64 = 52.0;

/// One axis event as a scroll distance in logical px: wheels by notch
/// (high-resolution wheels in fractions of one), touchpads 1:1.
fn wheel_pixels(discrete: i32, value120: i32, absolute: f64) -> f64 {
    if value120 != 0 {
        value120 as f64 / 120.0 * WHEEL_PX
    } else if discrete != 0 {
        discrete as f64 * WHEEL_PX
    } else {
        absolute
    }
}

/// Just past the next second, or the next minute, boundary.
fn until_next_tick(seconds: bool) -> Duration {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let to_second = Duration::from_secs(1) - Duration::from_nanos(since.subsec_nanos() as u64);
    if seconds {
        return to_second + Duration::from_millis(5);
    }
    let sec: u64 = crate::island::strftime("%S").parse().unwrap_or(0);
    Duration::from_secs(59u64.saturating_sub(sec)) + to_second + Duration::from_millis(20)
}

/// The island's layer surface, its viewport and fractional-scale object, on
/// `output` (None: the compositor's choice).
fn create_island_surface(
    compositor: &CompositorState,
    layer_shell: &LayerShell,
    viewporter: &WpViewporter,
    fractional_mgr: Option<&WpFractionalScaleManagerV1>,
    qh: &QueueHandle<App>,
    output: Option<&wl_output::WlOutput>,
    keyboard: KeyboardInteractivity,
) -> (LayerSurface, WpViewport, Option<WpFractionalScaleV1>) {
    let surface = compositor.create_surface(qh);
    let layer = layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("island-shell"), output);
    layer.set_anchor(Anchor::TOP);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(keyboard);
    layer.set_size(SURFACE_W, SURFACE_H);
    let viewport = viewporter.get_viewport(layer.wl_surface(), qh, ());
    let fractional = fractional_mgr.map(|m| m.get_fractional_scale(layer.wl_surface(), qh, ()));
    // an empty input region until the first frame says where the box is
    if let Ok(empty) = Region::new(compositor) {
        layer.set_input_region(Some(empty.wl_region()));
    }
    layer.commit();
    (layer, viewport, fractional)
}

/// Is this connector a laptop's own panel?
fn is_builtin(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|prefix| name.starts_with(prefix))
}

/// The output `want` names among `outputs` (name, in announcement order):
/// "focused" → `focused`, a connector name → that one; anything missing,
/// and "auto", falls back to the built-in panel, else the first output.
fn choose_output<'a>(outputs: &'a [String], want: &str, focused: Option<&str>) -> Option<&'a String> {
    let named = |name: &str| outputs.iter().find(|n| n.as_str() == name);
    let wanted = match want {
        "auto" => None,
        "focused" => focused.and_then(named),
        name => named(name),
    };
    wanted.or_else(|| outputs.iter().find(|n| is_builtin(n))).or_else(|| outputs.first())
}

fn config_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("island-shell").join("config")
}

/// A `#` starts a comment at the beginning of a line, or as a separate
/// word after a value; `accent = #66D9FF` keeps its colour.
fn strip_comment(line: &str) -> &str {
    let line = line.trim();
    if line.starts_with('#') {
        return "";
    }
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate().skip(1) {
        let separate = bytes[i - 1].is_ascii_whitespace() && bytes.get(i + 1).is_none_or(u8::is_ascii_whitespace);
        if b == b'#' && separate {
            return line[..i].trim_end();
        }
    }
    line
}

/// `key = value` lines, `#` comments (see island::Config).
fn load_config() -> Config {
    let mut config = Config::default();
    let Ok(text) = std::fs::read_to_string(config_path()) else { return config };
    for line in text.lines() {
        let line = strip_comment(line);
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
            Command::Dismiss => self.dismiss_menus(),
            Command::Toggle => self.island.toggle_visibility(now),
            Command::Show => self.island.show(now),
            Command::Hide => self.island.hide(now),
            Command::Dodge => self.island.set_dodged(true, now),
            Command::Undodge => self.island.set_dodged(false, now),
            Command::DodgeToggle => {
                let dodged = self.island.dodged;
                self.island.set_dodged(!dodged, now);
            }
            Command::Status => {
                return format!(
                    "{} {} on={} scale={} {}x{} refresh={:.3}ms pipeline={} buffers={} dismiss={:?} {} {}",
                    if self.island.is_hidden() { "hidden" } else { "shown" },
                    if self.island.dodged { "dodged" } else { "centred" },
                    self.bound_output_name().unwrap_or_else(|| "-".into()),
                    self.scale,
                    self.width,
                    self.height,
                    self.clock.refresh * 1e3,
                    self.clock.pipeline,
                    self.buffers.len(),
                    self.dismiss.as_ref().map(|d| d.size),
                    self.island.config.describe(),
                    self.island.describe(now) + &self.frame_stats.describe()
                );
            }
            Command::Set(key, value) => {
                if let Err(e) = self.island.config.set(&key, &value) {
                    return format!("error: {e}");
                }
                match key.as_str() {
                    "output" => self.outputs_changed(),
                    "clock" => {
                        self.island.update_clock();
                        self.arm_clock();
                    }
                    _ => {}
                }
            }
            Command::Launcher => {
                let open = self.island.toggle_launcher(now);
                self.set_keyboard_focus(open);
                self.after_change();
                return if open { "open".into() } else { "closed".into() };
            }
            Command::Quick => {
                if self.island.quick_menu_open() {
                    self.island.close_quick_menu(now);
                } else {
                    self.island.open_quick_menu_shortcut(now);
                }
                self.after_change();
                return "ok".into();
            }
            Command::Notifications => {
                let open = self.island.open_notification_center(now);
                self.set_keyboard_focus(open);
                self.after_change();
                return if open { "open".into() } else { "unavailable".into() };
            }
            Command::Dnd(action) => {
                let enabled = match action.as_str() {
                    "on" => true,
                    "off" => false,
                    "toggle" => !self.island.dnd,
                    "status" => return if self.island.dnd { "on" } else { "off" }.into(),
                    _ => return "error: expected on, off, toggle or status".into(),
                };
                self.island.set_dnd(enabled, now);
                self.notification_dnd.store(enabled, Ordering::Relaxed);
                self.after_change();
                return if enabled { "on" } else { "off" }.into();
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
                if self.island.replying() {
                    self.island.reply_key("", Some(&text), false, now);
                    self.after_change();
                    return "ok".into();
                }
                return "error: nothing takes text".into();
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
            Command::Nvim(json) => {
                match crate::nvim::parse(&json, &self.nvim_token) {
                    Some(crate::nvim::Message::State(editor)) => {
                        if self.island.set_editor(*editor, now) {
                            self.after_change();
                        }
                    }
                    Some(crate::nvim::Message::Result { server, kind, title, body, id, append }) => {
                        if !self.island.accepts_result(&server, now) { return "fallback".into(); }
                        self.island.set_result(id, title, body, kind == crate::nvim::ToastKind::Error, append, &self.renderer, now);
                        self.after_change();
                        return "ok: result".into();
                    }
                    Some(crate::nvim::Message::Cmdline { server, command }) => {
                        let visible = self.island.set_cmdline(&server, command, now);
                        self.after_change();
                        return if visible { "ok: cmdline" } else { "fallback" }.into();
                    }
                    Some(crate::nvim::Message::Gone(server)) => {
                        self.island.editor_gone(&server, now);
                        self.after_change();
                    }
                    Some(crate::nvim::Message::Toast { server, kind, title, body, progress }) => {
                        // a write is not news worth a pill: the chip flashes
                        if kind == crate::nvim::ToastKind::Saved {
                            self.island.editor_saved(&server, now);
                        } else {
                            self.island.notify(editor_toast(kind, title, body, progress), now);
                        }
                        self.after_change();
                    }
                    // not ours: a Neovim without the companion plugin, or
                    // one quoting the wrong token
                    None => return "error: not an island.nvim message".into(),
                }
                return "ok".into();
            }
            Command::Activity(event) => {
                if let activity::Event::Slider { kind: activity::SliderKind::Volume, muted, .. } = &event {
                    self.island.set_muted(*muted, now);
                }
                match event {
                    activity::Event::WifiOpen => self.island.open_connections(crate::network::Kind::Wifi, self.network_tx.clone(), now),
                    activity::Event::BluetoothOpen => self.island.open_connections(crate::network::Kind::Bluetooth, self.network_tx.clone(), now),
                    activity::Event::CalendarOpen => self.island.open_calendar(now),
                    _ => self.island.live_activity(event, now),
                }
                self.after_change();
                return "ok".into();
            }
            Command::Night(state) => {
                let on = self.island.controls.night;
                let want = match state.as_str() {
                    "status" => return if on { "on".into() } else { "off".into() },
                    "on" => true,
                    "off" => false,
                    _ => !on,
                };
                if want != on {
                    self.island.toggle_night(now);
                    let _ = self.controls.send(crate::control::Request::Night(want.then_some(self.island.config.night_temp)));
                }
                if !self.island.quick_menu_open() {
                    self.island.live_activity(activity::Event::NightMode { on: want }, now);
                }
                self.after_change();
                return if want { "on".into() } else { "off".into() };
            }
            Command::Record => {
                let reply = match self.island.toggle_recording(now) {
                    Ok(state) => state.to_string(),
                    Err(error) => return format!("error: {error}"),
                };
                self.after_change();
                return reply;
            }
            Command::Task(request) => {
                let reply = match self.island.task(request, now) {
                    Ok(reply) => reply,
                    Err(error) => return format!("error: {error}"),
                };
                self.after_change();
                return reply;
            }
            Command::Timer(command) => {
                let reply = match self.island.timer_command(&command, now) {
                    Ok(reply) => reply,
                    Err(error) => return format!("error: {error}"),
                };
                self.after_change();
                return reply;
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
        let island_output = self.bound_output_name();
        let Some(q) = self.sway_query.as_mut() else { return };
        match q.outputs(island_output.as_deref()) {
            Ok((focused, fs)) => {
                if focused != self.focused_output {
                    self.focused_output = focused;
                    if self.island.config.output == "focused" && self.rebind_output() {
                        // a different output shows a different workspace
                        return self.check_fullscreen();
                    }
                }
                if fs != self.island.hidden.fullscreen {
                    let now = self.frame_time();
                    self.island.set_fullscreen(fs, now);
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

    /// Modal menus own the keyboard until dismissed.
    fn set_keyboard_focus(&mut self, want: bool) {
        let next = if !want {
            KeyboardInteractivity::None
        } else if self.island.launcher.is_some() {
            KeyboardInteractivity::Exclusive
        } else if self.island.keyboard_panel_open() {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::OnDemand
        };
        if next == self.kbd_interactivity {
            return;
        }
        self.kbd_interactivity = next;
        self.layer.set_keyboard_interactivity(next);
        self.layer.commit();
    }

    /// One key press (or repeat) while the launcher is open.
    fn launcher_key(&mut self, event: &KeyEvent) {
        let now = self.frame_time();
        let ctrl = self.modifiers.ctrl;
        if self.island.launcher.is_none() {
            if event.keysym == Keysym::Escape {
                if self.island.close_activity_panel(now) { self.after_change(); }
            } else if self.island.replying() {
                let key = match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => "Return",
                    Keysym::BackSpace => "BackSpace",
                    Keysym::u | Keysym::U if ctrl => "u",
                    _ => "",
                };
                if let crate::island::ReplyKey::Send(id, text) = self.island.reply_key(key, event.utf8.as_deref(), ctrl, now) {
                    let _ = self.notification_commands.send(crate::notification_server::Command::Reply { id, text });
                }
                self.after_change();
            } else if self.island.calendar_open() {
                let key = match event.keysym {
                    Keysym::Left => "Left",
                    Keysym::Right => "Right",
                    Keysym::Up => "Up",
                    Keysym::Down => "Down",
                    Keysym::Home => "Home",
                    Keysym::Page_Up => "PageUp",
                    Keysym::Page_Down => "PageDown",
                    _ => "",
                };
                if self.island.calendar_key(key, now) { self.after_change(); }
            } else if self.island.quick_menu_open() {
                let key = match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => "enter",
                    Keysym::Down | Keysym::Right | Keysym::Tab => "next",
                    Keysym::Up | Keysym::Left | Keysym::ISO_Left_Tab => "prev",
                    _ => "",
                };
                if let Some(action) = self.island.quick_key(key, now) {
                    self.activate_quick_action(action, now);
                }
                self.after_change();
            } else {
                let key = match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => "Return",
                    Keysym::BackSpace => "BackSpace",
                    Keysym::Up => "Up",
                    Keysym::Down => "Down",
                    Keysym::u | Keysym::U if ctrl => "u",
                    _ => "",
                };
                if let Some(action) = self.island.center_reply_key(key, event.utf8.as_deref(), ctrl, now) {
                    self.handle_notification_action(action, now);
                    self.after_change();
                    return;
                }
                let key = if key == "u" { "" } else { key };
                if self.island.network_key(key, event.utf8.as_deref(), ctrl, now) { self.after_change(); }
            }
            return;
        }
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
            Keysym::BackSpace if ctrl => l.delete_previous_word(now),
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
    /// The keyboard layout may have changed (sway reports it once per
    /// keyboard): the Control Center's layout button follows it.
    fn layout_changed(&mut self) {
        let Some(layout) = self.sway_query.as_mut().and_then(|q| q.keyboard_layout().ok().flatten()) else { return };
        if self.island.controls.layout.as_deref() == Some(layout.as_str()) {
            return;
        }
        let now = self.frame_time();
        self.island.set_layout(layout, now);
        self.after_change();
    }

    fn after_change(&mut self) {
        // the Control Center reads the environment each time it opens
        let quick = self.island.quick_menu_open();
        if quick && !self.quick_was_open {
            let _ = self.controls.send(crate::control::Request::Refresh);
        }
        self.quick_was_open = quick;
        // the sound page follows applications starting and stopping
        let sound = self.island.sound_page_open();
        if sound && self.sound_poll.is_none() {
            let tx = self.controls.clone();
            self.sound_poll = self.loop_handle.insert_source(Timer::from_duration(SOUND_POLL), move |_, _, _: &mut App| {
                let _ = tx.send(crate::control::Request::Refresh);
                TimeoutAction::ToDuration(SOUND_POLL)
            }).ok();
        } else if !sound {
            if let Some(token) = self.sound_poll.take() { self.loop_handle.remove(token); }
        }
        // the screen recorder starts after 3-2-1 and stops on request
        if self.island.take_record_start() {
            crate::recorder::start(self.bound_output_name(), self.recorder_tx.clone());
        }
        if let Some(pid) = self.island.take_record_stop() {
            crate::recorder::stop(pid);
        }
        // a job whose owner process is gone ends as interrupted
        let owned = !self.island.task_owners().is_empty();
        if owned && self.task_poll.is_none() {
            self.task_poll = self.loop_handle.insert_source(Timer::from_duration(TASK_POLL), |_, _, app: &mut App| {
                let now = app.clock.predict(false);
                let gone: Vec<String> = app.island.task_owners().into_iter()
                    .filter(|(_, pid)| !std::path::Path::new(&format!("/proc/{pid}")).exists())
                    .map(|(id, _)| id).collect();
                for id in &gone { app.island.task_orphaned(id, now); }
                if !gone.is_empty() { app.after_change(); }
                if app.island.task_owners().is_empty() {
                    app.task_poll = None;
                    TimeoutAction::Drop
                } else {
                    TimeoutAction::ToDuration(TASK_POLL)
                }
            }).ok();
        }
        // whoever closed the launcher (a key, Touch ID, the compositor),
        // the keyboard goes back when it is gone
        // a reply takes the keyboard even over a fullscreen window
        let wants_keyboard = self.island.launcher.is_some() || self.island.replying()
            || (!self.island.is_hidden() && self.island.keyboard_panel_open());
        self.set_keyboard_focus(wants_keyboard);
        self.sync_dismiss_surface(wants_keyboard);
        let has_song = self.island.song().is_some();
        if has_song && self.cava.is_none() && !self.cava_pending && std::time::Instant::now() >= self.cava_retry {
            self.cava_pending = self.cava_start.request(());
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
        if !self.volume_pending { self.volume_pending = self.volume_read.request(()); }
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
        // a touchpad swipe that ended without a finger-lift event
        if let Some(id) = self.island.take_swiped() {
            self.close_notification(id);
        }
        if let Some(action) = self.island.take_center_action() {
            let now = self.clock.predict(false);
            self.handle_notification_action(action, now);
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
        if self.last_frame.map(|f| f.input) != Some(frame.input) {
            let region = Region::new(&self.compositor).expect("wl_region");
            if let Some(r) = frame.input {
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
        self.update_dismiss_region();

        self.viewport.set_source(0.0, 0.0, bw as f64, bh as f64);
        self.viewport.set_destination(self.width as i32, self.height as i32);
        if self.island.is_animating(t) || self.renderer.motion.moving(t) {
            surface.frame(&self.qh, FrameCallbackData(surface.clone()));
            self.frame_pending = true;
        }
        if let Ok(feedback) = self.presentation.feedback(surface, &self.qh) {
            self.clock.committed(&feedback, t, steady);
        }
        buffer.attach_to(surface).expect("attach");
        self.layer.commit();
        self.frame_stats.record(started.elapsed().as_secs_f64(), self.clock.refresh);
        if self.clock.debug {
            eprintln!("draw: {:.0} us steady={}", started.elapsed().as_secs_f64() * 1e6, steady as u8);
        }
    }

    /// (Re)start the clock's timer: every minute, or every second when the
    /// format shows seconds.
    fn arm_clock(&mut self) {
        if let Some(token) = self.clock_timer.take() {
            self.loop_handle.remove(token);
        }
        let seconds = self.island.config.clock_has_seconds();
        self.clock_timer = self
            .loop_handle
            .insert_source(Timer::from_duration(until_next_tick(seconds)), |_, _, app: &mut App| {
                if app.island.update_clock() {
                    app.redraw();
                }
                TimeoutAction::ToDuration(until_next_tick(app.island.config.clock_has_seconds()))
            })
            .ok();
    }

    fn output_name(&self, output: &wl_output::WlOutput) -> Option<String> {
        self.output_state.info(output).and_then(|info| info.name)
    }

    fn bound_output_name(&self) -> Option<String> {
        self.bound_output.as_ref().and_then(|output| self.output_name(output))
    }

    /// The output the config asks for among the ones that exist now, other
    /// than `gone` (sctk still lists an output while reporting it destroyed).
    fn pick_output(&self, gone: Option<&wl_output::WlOutput>) -> Option<wl_output::WlOutput> {
        let outputs: Vec<(wl_output::WlOutput, String)> = self
            .output_state
            .outputs()
            .filter(|output| Some(output) != gone)
            .filter_map(|output| self.output_name(&output).map(|name| (output, name)))
            .collect();
        let names: Vec<String> = outputs.iter().map(|(_, name)| name.clone()).collect();
        let chosen = choose_output(&names, &self.island.config.output, self.focused_output.as_deref())?;
        outputs.into_iter().find(|(_, name)| name == chosen).map(|(output, _)| output)
    }

    /// Outputs came, went or changed, or the config names another one:
    /// move the island if needed, then look at what that output shows.
    fn outputs_changed(&mut self) {
        self.outputs_changed_without(None);
    }

    fn outputs_changed_without(&mut self, gone: Option<&wl_output::WlOutput>) {
        self.rebind_output_without(gone);
        self.check_fullscreen();
    }

    /// Recreate the island's surface on the chosen output if it is not
    /// there already. Everything the island shows lives in `Island`, so the
    /// animations carry on; only the Wayland objects are new. Returns
    /// whether it moved.
    fn rebind_output(&mut self) -> bool {
        self.rebind_output_without(None)
    }

    fn rebind_output_without(&mut self, gone: Option<&wl_output::WlOutput>) -> bool {
        let target = self.pick_output(gone);
        // with no outputs at all there is nowhere to go: wait for one
        if target.is_none() || target == self.bound_output {
            return false;
        }
        let (layer, viewport, fractional) = create_island_surface(
            &self.compositor, &self.layer_shell, &self.viewporter, self.fractional_mgr.as_ref(),
            &self.qh, target.as_ref(), self.kbd_interactivity,
        );
        self.viewport.destroy();
        if let Some(old) = self.fractional.take() {
            old.destroy();
        }
        self.layer = layer;
        self.viewport = viewport;
        self.fractional = fractional;
        self.bound_output = target;
        if self.island.config.output != "auto" || self.clock.debug {
            eprintln!("island-shell: on output {}", self.bound_output_name().unwrap_or_default());
        }
        // the old surface's frame callback and buffers went with it
        self.configured = false;
        self.frame_pending = false;
        self.last_frame = None;
        self.drawn_for = None;
        self.buffers.clear();
        self.width = SURFACE_W;
        self.height = SURFACE_H;
        // no leave event arrives for a destroyed surface
        if self.island.hovered {
            self.pointer_left();
        }
        // the outside-click catcher covers the island's output
        if self.dismiss.take().is_some() {
            self.sync_dismiss_surface(true);
        }
        true
    }

    /// The pointer is no longer over the island (it left, or the surface
    /// under it was replaced).
    fn pointer_left(&mut self) {
        let now = self.clock.predict(false);
        self.island.hovered = false;
        self.island.clear_timer_hover();
        self.island.clear_network_hover(now);
        self.island.set_quick_hover(-1.0, -1.0, now);
        self.island.clear_center_hover(now);
        self.island.set_notice_hover(-1.0, -1.0, now);
        self.island.set_eq_hover(-1.0, -1.0, now);
        self.island.set_editor_chip_hover(-1.0, -1.0, now);
        self.island.set_editor_hover(None);
        if !self.island.is_scrubbing() {
            self.island.set_hovered_control(None);
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

    fn sync_dismiss_surface(&mut self, want: bool) {
        if !want {
            // Destruction immediately returns input to the desktop; the
            // island itself can still play its normal closing animation.
            self.dismiss = None;
            return;
        }
        if self.dismiss.is_some() { return; }
        let output = self.bound_output.clone().or_else(|| {
            self.layer.wl_surface().data::<SurfaceData<()>>().and_then(|data| data.outputs().next())
        });
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh, surface, Layer::Overlay, Some("island-dismiss"), output.as_ref(),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        let viewport = self.viewporter.get_viewport(layer.wl_surface(), &self.qh, ());
        let (buffer, pixels) = match self.pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) {
            Ok(buffer) => buffer,
            Err(e) => { eprintln!("island-shell: outside-click surface: {e}"); return; }
        };
        pixels.fill(0);
        layer.commit(); // Wait for the initial configure before attaching.
        self.dismiss = Some(DismissSurface { layer, viewport, buffer, size: (0, 0), input_hole: std::cell::Cell::new(None) });
    }

    fn update_dismiss_region(&self) {
        let Some(dismiss) = &self.dismiss else { return };
        let (w, h) = dismiss.size;
        if w == 0 || h == 0 { return; }
        let hole = self.last_frame.and_then(|frame| frame.input).map(|rect| dismiss_hole(rect, w, self.width));
        if dismiss.input_hole.get() == Some(hole) { return; }
        dismiss.input_hole.set(Some(hole));
        let region = Region::new(&self.compositor).expect("wl_region");
        region.add(0, 0, w as i32, h as i32);
        if let Some((x, y, width, height)) = hole {
            region.subtract(x, y, width, height);
        }
        dismiss.layer.set_input_region(Some(region.wl_region()));
        dismiss.layer.commit();
    }

    fn dismiss_menus(&mut self) {
        let now = self.frame_time();
        self.island.close_launcher(now);
        self.island.close_activity_panel(now);
        self.after_change();
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
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.outputs_changed();
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.outputs_changed();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.bound_output.as_ref() == Some(&output) {
            // the compositor closes the surface too; do not wait for it
            self.bound_output = None;
        }
        self.outputs_changed_without(Some(&output));
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if layer.wl_surface() == self.layer.wl_surface() {
            // its output went away (or was disabled): move to another one
            // rather than exit, and wait for one if none is left
            self.bound_output = None;
            self.configured = false;
            self.outputs_changed();
        } else if self.dismiss.as_ref().is_some_and(|d| d.layer.wl_surface() == layer.wl_surface()) {
            self.dismiss_menus();
        }
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        if layer.wl_surface() != self.layer.wl_surface() {
            if let Some(dismiss) = self.dismiss.as_mut().filter(|d| d.layer.wl_surface() == layer.wl_surface()) {
                let (w, h) = configure.new_size;
                if w == 0 || h == 0 { return; }
                let first = dismiss.size == (0, 0);
                dismiss.size = (w, h);
                dismiss.input_hole.set(None);
                dismiss.viewport.set_destination(w as i32, h as i32);
                if first {
                    dismiss.buffer.attach_to(dismiss.layer.wl_surface()).expect("unused dismiss buffer");
                    dismiss.layer.wl_surface().damage_buffer(0, 0, 1, 1);
                }
                self.update_dismiss_region();
            }
            return;
        }
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
            if self.dismiss.as_ref().is_some_and(|d| &event.surface == d.layer.wl_surface()) {
                if matches!(event.kind, PointerEventKind::Press { .. }) {
                    self.dismiss_menus();
                    changed = true;
                }
                continue;
            }
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    self.island.hovered = true;
                    self.pointer_at = event.position;
                    self.island.set_timer_hover(event.position.0, event.position.1);
                    self.island.set_network_hover(event.position.0, event.position.1, self.clock.predict(false));
                    self.island.set_quick_hover(event.position.0, event.position.1, self.clock.predict(false));
                    self.island.set_center_hover(event.position.0, event.position.1, self.clock.predict(false));
                    self.island.set_notice_hover(event.position.0, event.position.1, self.clock.predict(false));
                    self.island.set_eq_hover(event.position.0, event.position.1, self.clock.predict(false));
                    self.island.set_editor_chip_hover(event.position.0, event.position.1, self.clock.predict(false));
                    changed = true;
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer_left();
                    changed = true;
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_at = event.position;
                    if self.island.set_calendar_hover(event.position.0, event.position.1, self.clock.predict(false)) {
                        changed = true;
                    }
                    self.island.hold_tasks_hover(event.position.0, event.position.1, self.clock.predict(false));
                    if self.island.set_timer_hover(event.position.0, event.position.1) {
                        changed = true;
                    }
                    if self.island.set_network_hover(event.position.0, event.position.1, self.clock.predict(false)) {
                        changed = true;
                    }
                    if self.island.set_quick_hover(event.position.0, event.position.1, self.clock.predict(false)) {
                        changed = true;
                    }
                    if self.island.set_center_hover(event.position.0, event.position.1, self.clock.predict(false)) {
                        changed = true;
                    }
                    if self.island.set_notice_hover(event.position.0, event.position.1, self.clock.predict(false)) {
                        changed = true;
                    }
                    if self.island.set_eq_hover(event.position.0, event.position.1, self.clock.predict(false)) {
                        changed = true;
                    }
                    if self.island.set_editor_chip_hover(event.position.0, event.position.1, self.clock.predict(false)) {
                        changed = true;
                    }
                    if self.hover_launcher_row() {
                        changed = true;
                    }
                    let now = self.clock.predict(false);
                    // a notification (or a card) held by the mouse follows it sideways
                    if self.island.drag_notice(event.position.0, now) || self.island.drag_center(event.position.0, now) {
                        changed = true;
                        continue;
                    }
                    // a Control Center slider in the hand follows the pointer
                    if let Some((kind, value)) = self.island.quick_drag_to(event.position.0, now) {
                        self.set_level(kind, value);
                        changed = true;
                        continue;
                    }
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
                    let action = self.island.editor_action_at(self.width as f64 / 2.0, event.position.0, event.position.1, now);
                    if self.island.set_editor_hover(action) {
                        changed = true;
                    }
                    let control = self.island.control_at(self.width as f64 / 2.0, event.position.0, event.position.1, now);
                    if self.island.set_hovered_control(control) {
                        changed = true;
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if button == BTN_LEFT && self.island.quick_release(self.clock.predict(false)) {
                        changed = true;
                    }
                    if button == BTN_LEFT {
                        let now = self.clock.predict(false);
                        let (x, y) = event.position;
                        match self.island.release_notice(x, y, now) {
                            crate::island::NoticeRelease::Click(click) => self.notice_click(click, now),
                            crate::island::NoticeRelease::Dismissed(id) => self.close_notification(id),
                            crate::island::NoticeRelease::None => {}
                        }
                        if let Some(action) = self.island.release_center(x, y, now) {
                            self.handle_notification_action(action, now);
                        }
                        changed = true;
                    }
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
                        let over_row = self.hover_launcher_row();
                        if button == BTN_LEFT && self.island.launcher_close_at(self.width as f64 / 2.0, event.position.0, event.position.1, now) {
                            self.island.close_launcher(now);
                            self.island.return_to_quick_menu(now);
                        } else if button == BTN_LEFT && over_row {
                            if let Some(l) = self.island.launcher.as_ref() {
                                l.launch();
                            }
                            self.island.close_launcher(now);
                            self.set_keyboard_focus(false);
                        }
                        changed = true;
                        continue;
                    }
                    if button == BTN_LEFT && self.island.press_center(event.position.0, event.position.1, now) {
                        changed = true;
                        continue;
                    }
                    if let Some(action) = self.island.click_notification_center(event.position.0, event.position.1, button == BTN_LEFT, now) {
                        self.handle_notification_action(action, now);
                        changed = true;
                        continue;
                    }
                    if self.island.quick_menu_open() {
                        if button == BTN_LEFT {
                            match self.island.quick_press(event.position.0, event.position.1, now) {
                                Some(crate::quick::Hit::Action(action)) => self.activate_quick_action(action, now),
                                Some(crate::quick::Hit::Slider(kind, value)) => self.set_level(kind, value),
                                None => {}
                            }
                        }
                        changed = true;
                        continue;
                    }
                    // The compact countdown keeps its own small hit target,
                    // even when music or an editor can open from the rest
                    // of the island.
                    if button == BTN_LEFT && self.island.click_timer_menu(event.position.0, event.position.1, now) {
                        changed = true;
                        continue;
                    }
                    if button == BTN_RIGHT && self.island.timer_menu_at(event.position.0, event.position.1, now) {
                        changed = true;
                        continue;
                    }
                    if self.island.click_network_panel(event.position.0, event.position.1, button == BTN_LEFT, now) {
                        changed = true;
                        continue;
                    }
                    if self.island.click_calendar(event.position.0, event.position.1, button == BTN_LEFT, now) {
                        changed = true;
                        continue;
                    }
                    // a right click on the clock unfolds the month
                    if button == BTN_RIGHT && self.island.clock_at(event.position.0, event.position.1) {
                        self.island.open_calendar(now);
                        changed = true;
                        continue;
                    }
                    if button == BTN_LEFT && self.island.click_timer_at(event.position.0, event.position.1, now) {
                        changed = true;
                        continue;
                    }
                    if button == BTN_LEFT && self.island.click_record_at(event.position.0, event.position.1, now) {
                        changed = true;
                        continue;
                    }
                    if button == BTN_LEFT && self.island.click_tasks_at(event.position.0, event.position.1, now) {
                        changed = true;
                        continue;
                    }
                    // inside the editor's panel the rows and buttons act
                    if let Some(action) = self.island.editor_action_at(self.width as f64 / 2.0, event.position.0, event.position.1, now) {
                        if button == BTN_LEFT {
                            if let Some((server, expr)) = self.island.press_editor(action, now) {
                                crate::nvim::call(&server, &expr);
                            }
                            changed = true;
                            continue;
                        }
                    }
                    // inside the open player the buttons and the progress
                    // line take the click
                    if button == BTN_LEFT {
                        if let Some(control) = self.island.control_at(self.width as f64 / 2.0, event.position.0, event.position.1, now) {
                            if let Some((bus, track, command)) = self.island.press_control(control, now) {
                                dbus::control(&bus, &track, command);
                            }
                            changed = true;
                            continue;
                        }
                    }
                    if self.island.editor_panel_open(now) {
                        changed = true;
                        continue;
                    }
                    if button == BTN_LEFT {
                        // the body may be a click or the start of a swipe:
                        // the release decides
                        if self.island.press_notice(event.position.0, event.position.1, now) {
                            changed = true;
                            continue;
                        }
                        if let Some(click) = self.island.notice_click_at(event.position.0, event.position.1) {
                            self.notice_click(click, now);
                            changed = true;
                            continue;
                        }
                    }
                    // a click on a notification dismisses it
                    let dismissed_id = self.island.notification_id();
                    if self.island.dismiss_notice(now) {
                        if let Some(id) = dismissed_id {
                            let _ = self.notification_commands.send(crate::notification_server::Command::Dismiss(id));
                        }
                        changed = true;
                        continue;
                    }
                    let music = self.island.music_area_at(event.position.0, event.position.1);
                    let menu = self.island.accepts_quick_menu_click() && self.pointer_inside_island(event.position);
                    match body_click(button, music, menu) {
                        BodyClick::Menu => self.open_quick_menu(now),
                        BodyClick::PlayPause => self.player_control(PlayerCommand::PlayPause),
                        BodyClick::Next => self.player_control(PlayerCommand::Next),
                        BodyClick::Previous => self.player_control(PlayerCommand::Previous),
                        BodyClick::None => {}
                    }
                    changed = true;
                }
                PointerEventKind::Axis { vertical, horizontal, .. } => {
                    changed = true;
                    // two fingers sideways over a notification swipe it
                    if horizontal.absolute != 0.0 || horizontal.discrete != 0 || horizontal.stop {
                        let now = self.clock.predict(false);
                        let dx = wheel_pixels(horizontal.discrete, horizontal.value120, horizontal.absolute);
                        let (x, y) = event.position;
                        let (taken, dismissed) = self.island.swipe_notice(x, y, dx, horizontal.stop, now);
                        if let Some(id) = dismissed { self.close_notification(id); }
                        if taken { continue; }
                        if let Some(action) = self.island.swipe_center(x, y, dx, horizontal.stop, now) {
                            self.handle_notification_action(action, now);
                            continue;
                        }
                        if vertical.absolute == 0.0 && vertical.discrete == 0 { continue; }
                    }
                    // a notch is a push of WHEEL_PX; a touchpad moves 1:1
                    let px = wheel_pixels(vertical.discrete, vertical.value120, vertical.absolute);
                    let v = if vertical.discrete != 0 { vertical.discrete as f64 } else { vertical.absolute };
                    let now = self.clock.predict(false);
                    if self.island.scroll_notification_center(px / crate::notice_center::ROW_H, now) {
                        // the card under a resting pointer changed
                        let (x, y) = self.pointer_at;
                        self.island.set_center_hover(x, y, now);
                        continue;
                    }
                    if self.island.scroll_calendar(px, now) {
                        continue;
                    }
                    if self.island.scroll_network(px / crate::connections::ROW_H, now) {
                        changed = true;
                        continue;
                    }
                    if self.island.scroll_output(if vertical.discrete != 0 { v } else { v / 40.0 }, now) {
                        continue;
                    }
                    let music = self.island.music_area_at(event.position.0, event.position.1);
                    match self.island.launcher.as_mut() {
                        // inside the launcher the wheel scrolls the list
                        Some(l) => {
                            if l.scroll_by(px / crate::launcher::ROW_H, now) {
                                changed = true;
                            }
                        }
                        // on the line it skips tracks (as on the old touch target)
                        None if v < 0.0 && music => self.player_control(PlayerCommand::Next),
                        None if v > 0.0 && music => self.player_control(PlayerCommand::Previous),
                        None => {}
                    }
                }
            }
        }
        if changed {
            let now = self.frame_time();
            self.island.sync(now);
            self.after_change();
        }
    }
}

impl App {
    fn pointer_inside_island(&self, position: (f64, f64)) -> bool {
        self.last_frame.and_then(|frame| frame.bounds).is_some_and(|r| {
            position.0 >= r.x && position.0 <= r.x + r.w
                && position.1 >= r.y && position.1 <= r.y + r.h
        })
    }

    fn open_quick_menu(&mut self, now: Time) {
        if self.island.dodged {
            // Close Waybar's details drawer before opening the island menu.
            self.island.set_dodged(false, now);
            std::thread::spawn(|| {
                let _ = std::process::Command::new("pkill").args(["-RTMIN+9", "-x", "waybar"]).status();
            });
        }
        self.island.open_quick_menu(now);
    }

    fn activate_quick_action(&mut self, action: crate::quick::Action, now: Time) {
        use crate::control::Request;
        use crate::quick::{Action, View};
        // shortcuts to other sheets and the session leave the menu; its own
        // controls (Do Not Disturb, the layout, the output page) keep it open
        if matches!(action, Action::Wifi | Action::Bluetooth | Action::Timer | Action::Launcher
            | Action::Notifications | Action::Lock | Action::Sleep | Action::Close)
        {
            self.island.close_quick_menu(now);
        }
        match action {
            Action::Night => {
                let on = self.island.toggle_night(now);
                let _ = self.controls.send(Request::Night(on.then_some(self.island.config.night_temp)));
            }
            Action::Record => {
                // 3-2-1 needs the pill, so the menu goes first
                self.island.close_quick_menu(now);
                let _ = self.island.toggle_recording(now);
            }
            Action::MediaPrevious | Action::MediaPlayPause | Action::MediaNext => {
                let control = match action {
                    Action::MediaPrevious => crate::player::Control::Previous,
                    Action::MediaPlayPause => crate::player::Control::PlayPause,
                    _ => crate::player::Control::Next,
                };
                if let Some((bus, track, command)) = self.island.press_control(control, now) {
                    dbus::control(&bus, &track, command);
                }
            }
            Action::Wifi => self.island.open_connections(crate::network::Kind::Wifi, self.network_tx.clone(), now),
            Action::Bluetooth => self.island.open_connections(crate::network::Kind::Bluetooth, self.network_tx.clone(), now),
            Action::Timer => { let _ = self.island.timer_command("menu", now); }
            Action::Launcher => { self.island.toggle_launcher(now); }
            Action::Notifications => { self.island.open_notification_center(now); }
            Action::Dnd => {
                let enabled = !self.island.dnd;
                self.island.set_dnd(enabled, now);
                self.notification_dnd.store(enabled, Ordering::Relaxed);
            }
            Action::Outputs => {
                self.island.quick_show(View::Sound, now);
                let _ = self.controls.send(Request::Refresh);
            }
            Action::MicMute => {
                self.island.toggle_mic(now);
                let _ = self.controls.send(Request::MicMute);
            }
            Action::Back => self.island.quick_show(View::Main, now),
            Action::Sink(index) => {
                if let Some(sink) = self.island.controls.sinks.get(index).cloned() {
                    self.island.controls.default_sink = sink.name.clone();
                    let _ = self.controls.send(Request::Sink(sink.name));
                }
                self.island.quick_show(View::Main, now);
            }
            Action::Layout => { let _ = self.controls.send(Request::NextLayout); }
            Action::Lock => { let _ = self.controls.send(Request::Lock); }
            Action::Sleep => { let _ = self.controls.send(Request::Suspend); }
            Action::Logout => {
                if self.island.arm_logout(now) {
                    self.island.close_quick_menu(now);
                    let _ = self.controls.send(Request::Logout);
                }
            }
            Action::Close => {}
        }
    }

    fn notice_click(&mut self, click: crate::island::NoticeClick, now: Time) {
        match click {
            // an app that takes inline replies: the pill opens a field
            crate::island::NoticeClick::Invoke(_, key) if key == "inline-reply" => {
                self.island.begin_reply(now);
            }
            crate::island::NoticeClick::Invoke(id, key) => {
                let _ = self.notification_commands.send(crate::notification_server::Command::Action { id, key });
            }
            crate::island::NoticeClick::SendReply => {
                if let Some((id, text)) = self.island.send_reply(now) {
                    let _ = self.notification_commands.send(crate::notification_server::Command::Reply { id, text });
                }
            }
            crate::island::NoticeClick::Field => {}
            crate::island::NoticeClick::Dismiss(id) => {
                self.island.dismiss_notice(now);
                let _ = self.notification_commands.send(crate::notification_server::Command::Dismiss(id));
            }
        }
    }

    /// Close a notification the user dismissed (swiped or clicked away).
    fn close_notification(&self, id: u32) {
        if id != 0 {
            let _ = self.notification_commands.send(crate::notification_server::Command::Dismiss(id));
        }
    }

    /// Send a slider's new value to the worker (it keeps only the latest).
    fn set_level(&self, slider: crate::quick::Slider, value: f64) {
        use crate::control::Request;
        use crate::quick::Slider;
        let request = match slider {
            Slider::Volume => Request::Volume(value),
            Slider::Brightness => Request::Brightness(value),
            Slider::Mic => Request::Mic(value),
            Slider::App(i) => match self.island.controls.apps.get(i) {
                Some(app) => Request::AppVolume(app.streams.clone(), value),
                None => return,
            },
        };
        let _ = self.controls.send(request);
    }

    fn handle_notification_action(&mut self, action: crate::notice_center::Action, now: Time) {
        use crate::notice_center::Action;
        use crate::notification_server::Command;
        match action {
            Action::Invoke(id, key) => { let _ = self.notification_commands.send(Command::Action { id, key }); }
            Action::Dismiss(id) => { let _ = self.notification_commands.send(Command::Dismiss(id)); }
            Action::DismissMany(ids) => {
                for id in ids { let _ = self.notification_commands.send(Command::Dismiss(id)); }
            }
            Action::Reply(id, text) => { let _ = self.notification_commands.send(Command::Reply { id, text }); }
            Action::Close => self.island.return_to_quick_menu(now),
            Action::None => {}
        }
    }

    /// Point the launcher's selection at the row under the pointer.
    fn hover_launcher_row(&mut self) -> bool {
        let now = self.clock.predict(false);
        let (bx, by) = self.island.box_origin(self.width as f64 / 2.0, now);
        let (px, py) = self.pointer_at;
        let Some(l) = self.island.launcher.as_mut() else { return false };
        let close_changed = l.set_close_hover(px - bx, py - by, now);
        if px < bx || px > bx + crate::launcher::PANEL_W {
            return close_changed;
        }
        match l.row_at(by, py, now) {
            Some(index) => {
                l.select_index(index, now);
                true
            }
            None => close_changed,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_choice_prefers_the_config_then_the_builtin_panel() {
        let outputs: Vec<String> = ["HDMI-A-1", "eDP-1", "DP-2"].map(String::from).to_vec();
        let pick = |want: &str, focused: Option<&str>| choose_output(&outputs, want, focused).map(String::as_str);
        assert_eq!(pick("auto", Some("DP-2")), Some("eDP-1"));
        assert_eq!(pick("DP-2", None), Some("DP-2"));
        // a named output that is unplugged falls back like auto
        assert_eq!(pick("DP-9", None), Some("eDP-1"));
        assert_eq!(pick("focused", Some("HDMI-A-1")), Some("HDMI-A-1"));
        assert_eq!(pick("focused", None), Some("eDP-1"));
        let external: Vec<String> = ["DP-2", "HDMI-A-1"].map(String::from).to_vec();
        assert_eq!(choose_output(&external, "auto", None).map(String::as_str), Some("DP-2"));
        assert_eq!(choose_output(&[], "auto", None), None);
    }

    #[test]
    fn comments_leave_colours_alone() {
        assert_eq!(strip_comment("# smoothing = 1"), "");
        assert_eq!(strip_comment("#smoothing = 1"), "");
        assert_eq!(strip_comment("liquid = 1   # stretch"), "liquid = 1");
        assert_eq!(strip_comment("accent = #66D9FF  # cyan"), "accent = #66D9FF");
        assert_eq!(strip_comment("accent = #66D9FF"), "accent = #66D9FF");
        assert_eq!(strip_comment("clock = %H:%M #"), "clock = %H:%M");
    }

    #[test]
    fn clock_tick_lands_just_past_a_boundary() {
        assert!(until_next_tick(true) <= Duration::from_millis(1005));
        assert!(until_next_tick(false) <= Duration::from_millis(60_020));
    }

    #[test]
    fn music_buttons_are_scoped_to_the_music_area() {
        assert_eq!(body_click(BTN_LEFT, false, true), BodyClick::Menu);
        assert_eq!(body_click(BTN_RIGHT, false, true), BodyClick::None);
        assert_eq!(body_click(BTN_MIDDLE, false, true), BodyClick::None);
        assert_eq!(body_click(BTN_LEFT, true, true), BodyClick::PlayPause);
        assert_eq!(body_click(BTN_RIGHT, true, true), BodyClick::Next);
        assert_eq!(body_click(BTN_MIDDLE, true, true), BodyClick::Previous);
        assert_eq!(body_click(BTN_LEFT, false, false), BodyClick::None);
    }

    #[test]
    fn ordinary_clicks_do_not_interrupt_modal_panels_or_toggle_an_open_launcher() {
        let mut island = Island::new(Config::default());
        assert!(island.accepts_quick_menu_click());
        island.timer_command("menu", 10.0).unwrap();
        assert!(!island.accepts_quick_menu_click());
        island.close_activity_panel(11.0);
        assert!(island.accepts_quick_menu_click());
        assert!(island.toggle_launcher(12.0));
        assert!(!island.accepts_quick_menu_click());
    }

    #[test]
    fn outside_click_region_keeps_animated_island_interactive() {
        // Match the island's input rounding, translated from its centred
        // 980px surface to the full 1504px logical output, not pixel scale.
        let hole = dismiss_hole(Rect { x: 249.4, y: 0.0, w: 480.2, h: 377.1 }, 1504, 980);
        assert_eq!(hole, (511, 0, 482, 379));
        let compact = dismiss_hole(Rect { x: 390.0, y: 0.0, w: 200.0, h: 32.0 }, 1504, 980);
        assert_eq!(compact, (652, 0, 201, 33));
    }
}
