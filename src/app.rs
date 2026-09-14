//! The Wayland side: one layer-shell surface on the overlay layer, wl_shm
//! buffers drawn with cairo at the output's fractional scale, frame
//! callbacks driving the animations, pointer hover, and the event loop
//! sources (control socket, sway IPC, the clock timer).

use std::time::{Duration, Instant};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            EventLoop,
            generic::Generic,
            timer::{TimeoutAction, Timer},
            Interest, Mode, PostAction,
        },
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Dispatch, QueueHandle,
            globals::{GlobalList, registry_queue_init},
            protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
        },
        protocols::wp::{
            fractional_scale::v1::client::{
                wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
                wp_fractional_scale_v1::{self, WpFractionalScaleV1},
            },
            viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};

use crate::ctl::{Command, Server};
use crate::island::{BOX_H, BoxRect, FILLET, Island, Renderer};
use crate::sway;

/// Logical size of the (mostly transparent) surface: wide enough for the
/// dodge and the spread, tall enough for the box and its fillets.
const SURFACE_W: u32 = 980;
const SURFACE_H: u32 = (BOX_H + FILLET) as u32 + 2;

pub struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    viewport: WpViewport,
    _fractional: Option<WpFractionalScaleV1>,
    qh: QueueHandle<App>,

    /// output pixels per logical pixel (wp_fractional_scale)
    scale: f64,
    width: u32,
    height: u32,
    configured: bool,
    frame_pending: bool,
    pointer: Option<wl_pointer::WlPointer>,
    input_rect: Option<BoxRect>,
    canvas: Option<cairo::ImageSurface>,

    pub island: Island,
    renderer: Renderer,
    sway_query: Option<sway::Query>,
    pub exit: bool,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<App>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
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

    let mut island = Island::new();
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
        pool,
        layer,
        viewport,
        _fractional: fractional,
        qh: qh.clone(),
        scale: 1.0,
        width: SURFACE_W,
        height: SURFACE_H,
        configured: false,
        frame_pending: false,
        pointer: None,
        input_rect: None,
        canvas: None,
        island,
        renderer: Renderer::new(1.0),
        sway_query,
        exit: false,
    };

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

fn until_next_minute() -> Duration {
    let t = crate::island::libc_localtime();
    Duration::from_secs(60 - t.sec as u64) + Duration::from_millis(20)
}

fn bind_global<I>(globals: &GlobalList, qh: &QueueHandle<App>, version: u32) -> Result<I, Box<dyn std::error::Error>>
where
    I: smithay_client_toolkit::reexports::client::Proxy + 'static,
    App: Dispatch<I, ()>,
{
    Ok(globals.bind::<I, App, ()>(qh, version..=version, ())?)
}

impl App {
    fn handle_command(&mut self, cmd: Command) -> String {
        let now = Instant::now();
        match cmd {
            Command::Toggle => self.island.hidden.user = !self.island.hidden.user,
            Command::Show => self.island.hidden.user = false,
            Command::Hide => self.island.hidden.user = true,
            Command::Dodge => self.island.dodged = true,
            Command::Undodge => self.island.dodged = false,
            Command::DodgeToggle => self.island.dodged = !self.island.dodged,
            Command::Status => {
                return format!(
                    "{} {} scale={} {}x{}",
                    if self.island.is_hidden() { "hidden" } else { "shown" },
                    if self.island.dodged { "dodged" } else { "centred" },
                    self.scale,
                    self.width,
                    self.height
                );
            }
            Command::Quit => {
                self.exit = true;
                return "bye".into();
            }
        }
        self.island.sync(now);
        self.redraw();
        "ok".into()
    }

    fn check_fullscreen(&mut self) {
        let Some(q) = self.sway_query.as_mut() else { return };
        match q.fullscreen_focused() {
            Ok(fs) => {
                if fs != self.island.hidden.fullscreen {
                    self.island.hidden.fullscreen = fs;
                    self.island.sync(Instant::now());
                    self.redraw();
                }
            }
            Err(e) => {
                eprintln!("island-shell: sway query: {e}");
                self.sway_query = None;
            }
        }
    }

    /// Present a new frame now unless one is already on its way.
    pub fn redraw(&mut self) {
        if self.configured && !self.frame_pending {
            self.draw();
        }
    }

    fn draw(&mut self) {
        let now = Instant::now();
        let bw = (self.width as f64 * self.scale).round() as i32;
        let bh = (self.height as f64 * self.scale).round() as i32;
        let stride = bw * 4;

        let canvas = match &self.canvas {
            Some(c) if c.width() == bw && c.height() == bh => self.canvas.as_ref().unwrap(),
            _ => {
                self.canvas = Some(cairo::ImageSurface::create(cairo::Format::ARgb32, bw, bh).expect("cairo surface"));
                self.canvas.as_ref().unwrap()
            }
        };
        let rect = {
            let cr = cairo::Context::new(canvas).expect("cairo context");
            cr.set_operator(cairo::Operator::Source);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = cr.paint();
            cr.set_operator(cairo::Operator::Over);
            cr.scale(self.scale, self.scale);
            self.island.draw(&cr, &self.renderer, self.width as f64 / 2.0, now)
        };
        canvas.flush();

        let (buffer, shm_canvas) = self
            .pool
            .create_buffer(bw, bh, stride, wl_shm::Format::Argb8888)
            .expect("shm buffer");
        let cairo_stride = canvas.stride() as usize;
        canvas
            .with_data(|src| {
                for y in 0..bh as usize {
                    let row = &src[y * cairo_stride..y * cairo_stride + stride as usize];
                    shm_canvas[y * stride as usize..(y + 1) * stride as usize].copy_from_slice(row);
                }
            })
            .expect("cairo surface data");

        // input only over the box (the rest of the surface is empty air)
        if rect != self.input_rect {
            let region = Region::new(&self.compositor).expect("wl_region");
            if let Some(r) = rect {
                region.add(r.x.floor() as i32, r.y.floor() as i32, r.w.ceil() as i32 + 1, r.h.ceil() as i32 + 1);
            }
            self.layer.set_input_region(Some(region.wl_region()));
            self.input_rect = rect;
        }

        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, bw, bh);
        self.viewport.set_source(0.0, 0.0, bw as f64, bh as f64);
        self.viewport.set_destination(self.width as i32, self.height as i32);
        if self.island.is_animating(now) {
            surface.frame(&self.qh, FrameCallbackData(surface.clone()));
            self.frame_pending = true;
        }
        buffer.attach_to(surface).expect("attach");
        self.layer.commit();
    }

    fn set_scale(&mut self, scale: f64) {
        if (scale - self.scale).abs() < 1e-6 {
            return;
        }
        self.scale = scale;
        self.renderer = Renderer::new(scale);
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
        self.draw();
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
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
        // draw straight away (the first frame, or a new size); a pending
        // frame callback will draw again, which is harmless
        self.draw();
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
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
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
                    changed = true;
                }
                PointerEventKind::Leave { .. } => {
                    self.island.hovered = false;
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            self.island.sync(Instant::now());
            self.redraw();
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch<WpViewporter, ()> for App {
    fn event(_: &mut Self, _: &WpViewporter, _: <WpViewporter as smithay_client_toolkit::reexports::client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpViewport, ()> for App {
    fn event(_: &mut Self, _: &WpViewport, _: <WpViewport as smithay_client_toolkit::reexports::client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpFractionalScaleManagerV1, ()> for App {
    fn event(_: &mut Self, _: &WpFractionalScaleManagerV1, _: <WpFractionalScaleManagerV1 as smithay_client_toolkit::reexports::client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
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
