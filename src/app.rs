//! The window runtime: a Wayland client that paints with the software canvas.
//!
//! A program implements [`App`] and calls [`run`]. The runtime owns everything else: the window, the shared-memory
//! buffers, HiDPI scale, pointer and keyboard (with repeat), and the redraw policy. It draws a frame only after
//! input and while [`ui::State::animating`](crate::ui::State::animating) says something moves, then goes back to
//! sleep, so an idle program costs no CPU.
//!
//! ```ignore
//! struct Hello;
//! impl chebrium::app::App for Hello {
//!     fn draw(&mut self, ui: &mut Ui, size: [f32; 2], host: &mut Host) {
//!         if ui.button([20.0, 20.0, 120.0, 32.0], "Quit", true) { host.quit = true; }
//!     }
//! }
//! chebrium::app::run(Config::new("Hello", "hello"), Hello);
//! ```

use wayland_protocols::wp::{
    fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        wp_fractional_scale_v1::{self, WpFractionalScaleV1},
    },
    viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};
use crate::input::{Key, KeyMods};
use crate::record::{Frame, Recorder};
use crate::soft::Gfx;
use crate::style::Style;
use crate::theme::Appearance;
use crate::ui::{State, Ui};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    data_device_manager::{
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::{CopyPasteSource, DataSourceHandler},
        DataDeviceManagerState, WritePipe,
    },
    delegate_compositor, delegate_data_device, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry, delegate_seat, delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::calloop::{EventLoop, LoopHandle},
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::KeyboardHandler,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use smithay_client_toolkit::reexports::csd_frame::WindowState;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::time::Duration;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_data_device::WlDataDevice, wl_data_device_manager::DndAction, wl_data_source::WlDataSource, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

const BTN_LEFT: u32 = 0x110;
const COPY_MIMES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

/// What a program that handles the keyboard itself (a terminal) gets: the key as the keyboard layout produced it.
pub use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym, Modifiers};

/// A pointer event as it came: all buttons, motion, the wheel in both directions.
#[derive(Clone, Copy, Debug)]
pub enum PointerKind {
    Enter,
    Leave,
    Motion,
    Press { button: u32, serial: u32 },
    Release { button: u32 },
    /// scrolling: smooth amounts and whole wheel notches, vertical and horizontal
    Axis { dx: f32, dy: f32, discrete_x: i32, discrete_y: i32 },
}

#[derive(Clone, Copy, Debug)]
pub struct PointerEvt {
    /// position in logical pixels
    pub pos: [f32; 2],
    pub kind: PointerKind,
}

/// A message box from other threads to the program that wakes it when something arrives: a terminal's reader thread, a
/// decoder. The program calls `connect` with the waker it gets in `App::start`, then reads with `drain`.
pub struct Inbox<T> {
    rx: std::sync::mpsc::Receiver<T>,
    link: std::sync::Arc<std::sync::Mutex<Option<Waker>>>,
}

pub struct Outbox<T> {
    tx: std::sync::mpsc::Sender<T>,
    link: std::sync::Arc<std::sync::Mutex<Option<Waker>>>,
}

impl<T> Clone for Outbox<T> {
    fn clone(&self) -> Self {
        Outbox { tx: self.tx.clone(), link: self.link.clone() }
    }
}

pub fn channel<T>() -> (Outbox<T>, Inbox<T>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let link = std::sync::Arc::new(std::sync::Mutex::new(None));
    (Outbox { tx, link: link.clone() }, Inbox { rx, link })
}

impl<T> Outbox<T> {
    /// Send, and wake the program (once it has connected).
    pub fn send(&self, msg: T) {
        let _ = self.tx.send(msg);
        if let Ok(g) = self.link.lock() {
            if let Some(w) = g.as_ref() {
                w.wake();
            }
        }
    }
}

impl<T> Inbox<T> {
    pub fn connect(&self, waker: Waker) {
        if let Ok(mut g) = self.link.lock() {
            *g = Some(waker);
        }
    }

    /// Everything that has arrived.
    pub fn drain(&self) -> Vec<T> {
        self.rx.try_iter().collect()
    }
}

/// Who draws the header (and border) of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Decorations {
    /// the shell draws it, in the theme's style (the default: nothing to do)
    #[default]
    Server,
    /// the program draws its own (see `chrome`); the shell still rounds the corners and casts the shadow
    Client,
    /// no header at all: the program is its content (a viewer, a player, a splash screen)
    None,
}

pub struct Config {
    pub title: String,
    /// the desktop file / icon name of the program
    pub app_id: String,
    pub size: (u32, u32),
    pub min_size: (u32, u32),
    pub decorations: Decorations,
    /// the user may resize the window, maximise it or take it fullscreen (a calculator may not)
    pub resizable: bool,
}

impl Config {
    pub fn new(title: &str, app_id: &str) -> Config {
        Config { title: title.into(), app_id: app_id.into(), size: (480, 360), min_size: (200, 160), decorations: Decorations::Server, resizable: true }
    }

    pub fn decorations(mut self, d: Decorations) -> Config {
        self.decorations = d;
        self
    }

    pub fn size(mut self, w: u32, h: u32) -> Config {
        self.size = (w, h);
        self
    }

    /// A window of one size, that cannot be resized, maximised or made fullscreen (the program changes its size itself).
    pub fn resizable(mut self, on: bool) -> Config {
        self.resizable = on;
        self
    }

    pub fn min_size(mut self, w: u32, h: u32) -> Config {
        self.min_size = (w, h);
        self
    }
}

/// What the program can ask of the runtime.
#[derive(Default)]
pub struct Host {
    /// close the window
    pub quit: bool,
    /// change the window title
    pub title: Option<String>,
    /// draw one more frame even though nothing is animating
    pub redraw: bool,
    /// go fullscreen (`Some(true)`) or leave it (`Some(false)`)
    pub fullscreen: Option<bool>,
    /// physical pixels per logical pixel (the runtime sets it before every frame): a program that renders its own
    /// pictures wants to know how big they have to be
    pub scale: f32,
    /// the look of the desktop (kept up to date: a program follows a theme switch while it is open)
    pub appearance: Appearance,
    /// the window is maximised right now (set by the runtime)
    pub maximized: bool,
    /// ask the shell to start moving the window (call it on the press that begins a drag of the header)
    pub drag: bool,
    /// ask the shell to start resizing the window from an edge or corner
    pub resize: Option<Edge>,
    pub minimize: bool,
    /// make the window this size (logical px), and stop it being made smaller than the other size: a program whose
    /// content changes shape (a calculator with more keys) asks for the room it needs
    pub set_size: Option<(u32, u32)>,
    /// the window cannot be resized by the user (set by the runtime from the config)
    pub fixed: bool,
    pub set_min_size: Option<(u32, u32)>,
    /// maximise, or bring the window back when it is maximised
    pub toggle_maximize: bool,
    /// the desktop's menu bar shows this program's `App::menus` (set by the runtime): the program can leave its own
    /// mode pickers and option buttons out of the window
    pub menu_bar: bool,
    /// the window has the keyboard focus (set by the runtime)
    pub activated: bool,
    /// put this text on the clipboard
    pub copy: Option<String>,
    /// put this picture (PNG bytes) on the clipboard
    pub copy_image: Option<Vec<u8>>,
    /// ask for the clipboard's text: it arrives in `App::clipboard`
    pub paste: bool,
    /// the pointer shape to show (`None`: whatever the widgets under the pointer asked for)
    pub cursor: Option<crate::ui::Cursor>,
}

/// An edge or corner of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Edge {
    fn raw(self) -> ResizeEdge {
        match self {
            Edge::Top => ResizeEdge::Top,
            Edge::Bottom => ResizeEdge::Bottom,
            Edge::Left => ResizeEdge::Left,
            Edge::Right => ResizeEdge::Right,
            Edge::TopLeft => ResizeEdge::TopLeft,
            Edge::TopRight => ResizeEdge::TopRight,
            Edge::BottomLeft => ResizeEdge::BottomLeft,
            Edge::BottomRight => ResizeEdge::BottomRight,
        }
    }
}

/// Wakes the program from another thread: a decoder that finished a picture, a player with a new video frame. The
/// runtime then draws a frame. Cheap to clone and to call.
#[derive(Clone)]
pub struct Waker(smithay_client_toolkit::reexports::calloop::ping::Ping);

impl Waker {
    pub fn wake(&self) {
        self.0.ping();
    }
}

pub trait App {
    /// Called once when the window is set up: keep the waker if other threads will need it.
    fn start(&mut self, _waker: Waker) {}

    /// A key press, raw: the keysym, the text it produced and the modifiers. Return true when the program used it, and
    /// the widgets will not see it. A terminal, a text editor and games want this; most programs use `key`.
    fn raw_key(&mut self, _event: &KeyEvent, _mods: &Modifiers, _serial: u32, _host: &mut Host) -> bool {
        false
    }

    /// A pointer event, raw (all buttons, motion, both wheels). Return true when the program used it.
    fn raw_pointer(&mut self, _event: &PointerEvt, _mods: &Modifiers, _host: &mut Host) -> bool {
        false
    }

    /// The text of the clipboard, after `Host::paste`.
    fn clipboard(&mut self, _text: String, _host: &mut Host) {}

    /// Files or text dropped on the window (from a drag of the Files app, or of selected text).
    fn dropped(&mut self, _files: Vec<std::path::PathBuf>, _text: String, _host: &mut Host) {}

    /// The menus for the desktop's menu bar (View, Edit, ...). Called before every frame and sent only when they changed,
    /// so it is the place to say what is checked or disabled right now. Empty: the program has no menus there.
    fn menus(&self) -> Vec<crate::appmenu::Menu> {
        Vec::new()
    }

    /// An entry of `menus` was chosen in the menu bar.
    fn menu_action(&mut self, _id: u32, _host: &mut Host) {}

    /// Called before every frame: the place to read what other threads sent.
    fn tick(&mut self, _host: &mut Host) {}

    /// Build this frame's widgets. `size` is the window in logical pixels.
    fn draw(&mut self, ui: &mut Ui<'_>, size: [f32; 2], host: &mut Host);

    /// A key press the widgets did not use (a text field being edited takes the typing first).
    fn key(&mut self, _key: Key, _mods: KeyMods, _host: &mut Host) {}

    /// The colours of the widgets. By default they follow the desktop's look.
    fn style(&self, look: &Appearance) -> Style {
        look.style()
    }

    /// The window background (straight RGB) and its opacity. By default the glass of the desktop's theme: translucent, so
    /// the shell frosts what is behind the window.
    fn background(&self, look: &Appearance) -> ([f32; 3], f32) {
        look.background()
    }
}

/// `CHEBRIUM_PERF=1`: once a second, where the frames of this program went (milliseconds a frame, on average).
#[derive(Default)]
struct Perf {
    on: bool,
    since: Option<std::time::Instant>,
    frames: u32,
    skipped: u32,
    build: f64,
    update: f64,
    copy: f64,
    pixels: u64,
}

impl Perf {
    fn report(&mut self) {
        let since = *self.since.get_or_insert_with(std::time::Instant::now);
        if since.elapsed() < Duration::from_secs(1) || self.frames + self.skipped == 0 {
            return;
        }
        let n = self.frames.max(1) as f64;
        eprintln!(
            "chebrium: {} frames, {} unchanged | per frame: build {:.2} ms, paint {:.2} ms, copy {:.2} ms | {:.0}k px painted",
            self.frames,
            self.skipped,
            self.build / (n + self.skipped as f64),
            self.update / (n + self.skipped as f64),
            self.copy / n,
            self.pixels as f64 / n / 1000.0
        );
        *self = Perf { on: true, since: Some(std::time::Instant::now()), ..Default::default() };
    }
}

struct Runtime<A: App> {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    qh: QueueHandle<Runtime<A>>,
    loop_handle: LoopHandle<'static, Runtime<A>>,
    width: u32,
    height: u32,
    scale: i32,
    /// the exact scale the desktop asked for (`wp_fractional_scale`), when it speaks it and there is a viewport to use it with
    frac: Option<f32>,
    viewport: Option<WpViewport>,
    /// kept alive: the desktop sends the scale through it
    _fractional: Option<WpFractionalScaleV1>,
    configured: bool,
    dirty: bool,
    frame_pending: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /// `wp_cursor_shape_v1`: the compositor draws the cursor, the program only names it
    shape_manager: Option<smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager>,
    shape_device: Option<smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    enter_serial: u32,
    shown_cursor: Option<crate::ui::Cursor>,
    mods: Modifiers,
    state: State,
    gfx: Gfx,
    /// the pixels and commands of the last frame: a frame that changed nothing is not sent, and a small change
    /// repaints only its own rectangle
    frame: Frame,
    host: Host,
    app: A,
    seat: Option<wl_seat::WlSeat>,
    /// serial of the last pointer press: moving and resizing need it
    serial: u32,
    look_stamp: Option<std::time::SystemTime>,
    data_device_manager: DataDeviceManagerState,
    data_device: Option<DataDevice>,
    copy_source: Option<CopyPasteSource>,
    clip_text: String,
    /// the picture on the clipboard when we copied one (then `clip_text` is empty)
    clip_png: Vec<u8>,
    /// serial of the last key press or click: setting the clipboard needs one
    input_serial: u32,
    paste_out: Outbox<String>,
    paste_in: Inbox<String>,
    /// what was dropped on the window: (files, text)
    drop_out: Outbox<(Vec<std::path::PathBuf>, String)>,
    drop_in: Inbox<(Vec<std::path::PathBuf>, String)>,
    perf: Perf,
    last_trim: std::time::Instant,
    menu: Option<crate::appmenu::Client>,
}

impl<A: App + 'static> Runtime<A> {
    /// Name the pointer shape to the compositor when it changed (or the pointer just came in).
    fn cursor_ops(&mut self) {
        use crate::ui::Cursor;
        use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;
        let want = self.host.cursor.unwrap_or(self.state.cursor);
        if self.shown_cursor == Some(want) || self.enter_serial == 0 {
            return;
        }
        let Some(dev) = &self.shape_device else { return };
        dev.set_shape(
            self.enter_serial,
            match want {
                Cursor::Default => Shape::Default,
                Cursor::Text => Shape::Text,
                Cursor::Pointer => Shape::Pointer,
                Cursor::Grab => Shape::Grab,
                Cursor::Grabbing => Shape::Grabbing,
                Cursor::Move => Shape::Move,
                Cursor::ResizeH => Shape::EwResize,
                Cursor::ResizeV => Shape::NsResize,
                Cursor::ResizeNwse => Shape::NwseResize,
                Cursor::ResizeNesw => Shape::NeswResize,
                Cursor::NotAllowed => Shape::NotAllowed,
                Cursor::Wait => Shape::Wait,
                Cursor::Crosshair => Shape::Crosshair,
            },
        );
        self.shown_cursor = Some(want);
    }

    /// Do what the program asked of the window during the frame.
    fn window_ops(&mut self) {
        let (drag, resize) = (std::mem::take(&mut self.host.drag), self.host.resize.take());
        if let Some(seat) = self.seat.clone() {
            if drag {
                self.window.move_(&seat, self.serial);
            } else if let Some(edge) = resize {
                self.window.resize(&seat, self.serial, edge.raw());
            }
        }
        if std::mem::take(&mut self.host.minimize) {
            self.window.set_minimized();
        }
        if std::mem::take(&mut self.host.toggle_maximize) {
            if self.host.maximized {
                self.window.unset_maximized();
            } else {
                self.window.set_maximized();
            }
        }
    }

    /// The clipboard, as the program asked during the frame.
    fn clipboard_ops(&mut self) {
        if let Some(text) = self.host.copy.take() {
            if let Some(dd) = &self.data_device {
                let source = self.data_device_manager.create_copy_paste_source(&self.qh, COPY_MIMES);
                source.set_selection(dd, self.input_serial);
                self.clip_text = text;
                self.clip_png.clear();
                self.copy_source = Some(source);
            }
        }
        if let Some(png) = self.host.copy_image.take() {
            if let Some(dd) = &self.data_device {
                let source = self.data_device_manager.create_copy_paste_source(&self.qh, ["image/png"]);
                source.set_selection(dd, self.input_serial);
                self.clip_text.clear();
                self.clip_png = png;
                self.copy_source = Some(source);
            }
        }
        if std::mem::take(&mut self.host.paste) {
            if self.copy_source.is_some() {
                // our own selection: no round trip through the compositor (a picture is no text to paste)
                if self.clip_png.is_empty() {
                    let text = self.clip_text.clone();
                    self.paste_out.send(text);
                }
            } else if let Some(dd) = &self.data_device {
                if let Some(offer) = dd.data().selection_offer() {
                    if let Some(mime) = offer.with_mime_types(|m| COPY_MIMES.iter().find(|c| m.iter().any(|x| x == *c)).map(|s| s.to_string())) {
                        if let Ok(mut pipe) = offer.receive(mime) {
                            let out = self.paste_out.clone();
                            std::thread::spawn(move || {
                                let mut buf = Vec::new();
                                if pipe.read_to_end(&mut buf).is_ok() {
                                    out.send(String::from_utf8_lossy(&buf).into_owned());
                                }
                            });
                        }
                    }
                }
            }
        }
    }

    /// The shell changed its look (or its language): follow it.
    fn follow_look(&mut self) {
        // free memory goes back to the system now and then (what a decoder or an image load left behind)
        if self.last_trim.elapsed() > Duration::from_secs(5) {
            crate::mem::trim();
            self.last_trim = std::time::Instant::now();
        }
        let stamp = Appearance::stamp();
        if stamp != self.look_stamp {
            self.look_stamp = stamp;
            let look = Appearance::load();
            crate::i18n::set_language(&look.language);
            if look != self.host.appearance {
                self.host.appearance = look;
                self.frame.invalidate();
                self.schedule();
            }
        }
    }

    fn schedule(&mut self) {
        self.dirty = true;
        if self.configured && !self.frame_pending {
            self.draw();
        }
    }

    /// Device pixels per logical pixel: the desktop's exact scale when it gave one, else the whole one.
    fn fscale(&self) -> f32 {
        match (&self.viewport, self.frac) {
            (Some(_), Some(f)) => f.max(0.5),
            _ => self.scale as f32,
        }
    }

    fn draw(&mut self) {
        self.dirty = false;
        let s = self.scale;
        let fs = self.fscale();
        let (w, h) = ((self.width as f32 * fs).round() as usize, (self.height as f32 * fs).round() as usize);
        self.frame.resize(w, h);
        for text in self.paste_in.drain() {
            // a field being edited takes the pasted text; otherwise the program does
            if !self.state.insert_text(&text) {
                self.app.clipboard(text, &mut self.host);
            }
        }
        for (files, text) in self.drop_in.drain() {
            self.app.dropped(files, text, &mut self.host);
        }
        if let Some(client) = &mut self.menu {
            for id in client.take() {
                self.app.menu_action(id, &mut self.host);
            }
            client.set(&self.app.menus());
            self.host.menu_bar = client.available() && self.host.appearance.menubar;
        }
        self.app.tick(&mut self.host);
        let (bg, bg_a) = self.app.background(&self.host.appearance);
        let t_build = std::time::Instant::now();
        let dt = self.state.begin_frame();
        let style = self.app.style(&self.host.appearance);
        self.host.scale = fs;
        // the widgets draw into a recording; nothing is painted yet
        let mut recorder = Recorder::new(&mut self.gfx, fs);
        let mut ui = Ui::new(&mut recorder, &mut self.state, style, dt);
        self.app.draw(&mut ui, [self.width as f32, self.height as f32], &mut self.host);
        let seen = ui.edit_seen;
        let cmds = recorder.cmds;
        self.window_ops();
        if let Some(m) = self.host.set_min_size.take() {
            if !self.host.fixed {
                self.window.set_min_size(Some(m));
            }
        }
        if let Some(sz) = self.host.set_size.take() {
            if self.host.fixed {
                // a window of one size: it is the new size that is the only one
                self.window.set_min_size(Some(sz));
                self.window.set_max_size(Some(sz));
            }
            if sz != (self.width, self.height) {
                self.width = sz.0;
                self.height = sz.1;
                self.dirty = true;
            }
        }
        self.clipboard_ops();
        self.cursor_ops();
        self.state.end_frame(seen);
        // moving things need another frame; the frame callback below delivers it
        let more = self.state.animating() || std::mem::take(&mut self.host.redraw);
        if more {
            self.dirty = true;
        }
        if let Some(t) = self.host.title.take() {
            self.window.set_title(t);
        }
        match self.host.fullscreen.take() {
            Some(true) => self.window.set_fullscreen(None),
            Some(false) => self.window.unset_fullscreen(),
            None => {}
        }
        let surface = self.window.wl_surface().clone();
        let build_ms = t_build.elapsed().as_secs_f64() * 1000.0;
        let t_update = std::time::Instant::now();
        let damage = self.frame.update(cmds, &mut self.gfx, (bg, bg_a), fs);
        if self.perf.on {
            self.perf.build += build_ms;
            self.perf.update += t_update.elapsed().as_secs_f64() * 1000.0;
        }
        let Some(damage) = damage else {
            if self.perf.on {
                self.perf.skipped += 1;
                self.perf.report();
            }
            // nothing on screen changed: no buffer is sent, but an animation still needs its next callback
            if more {
                surface.frame(&self.qh, surface.clone());
                self.window.commit();
                self.frame_pending = true;
            }
            return;
        };
        let (buffer, data) = match self.pool.create_buffer(w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("chebrium: buffer: {e}");
                self.frame.invalidate();
                return;
            }
        };
        // a fresh buffer holds nothing of the last frame, so it gets all the pixels; only the changed box is damaged
        // (the pool may hand out a buffer a little larger than the frame: it is rounded up to its alignment)
        let t_copy = std::time::Instant::now();
        let n = self.frame.data.len().min(data.len());
        data[..n].copy_from_slice(&self.frame.data[..n]);
        if self.perf.on {
            self.perf.frames += 1;
            self.perf.copy += t_copy.elapsed().as_secs_f64() * 1000.0;
            self.perf.pixels += ((damage[2] - damage[0]).max(0) as u64) * ((damage[3] - damage[1]).max(0) as u64);
            self.perf.report();
        }
        match (&self.viewport, self.frac) {
            // the buffer has the exact scale's pixels: the viewport says how big it is on the screen
            (Some(v), Some(_)) => {
                surface.set_buffer_scale(1);
                v.set_destination(self.width as i32, self.height as i32);
            }
            _ => surface.set_buffer_scale(s),
        }
        surface.damage_buffer(damage[0], damage[1], damage[2] - damage[0], damage[3] - damage[1]);
        surface.frame(&self.qh, surface.clone());
        if buffer.attach_to(&surface).is_ok() {
            self.window.commit();
            self.frame_pending = true;
        }
    }

    fn on_key(&mut self, event: KeyEvent) {
        // a program that reads the keyboard itself gets the key as it is
        let serial = self.input_serial;
        if self.app.raw_key(&event, &self.mods, serial, &mut self.host) {
            self.schedule();
            return;
        }
        let mods = KeyMods { ctrl: self.mods.ctrl, alt: self.mods.alt, shift: self.mods.shift };
        let key = match event.keysym {
            Keysym::Return | Keysym::KP_Enter => Some(Key::Enter),
            Keysym::Escape => Some(Key::Esc),
            Keysym::BackSpace => Some(Key::Backspace),
            Keysym::Delete => Some(Key::Delete),
            Keysym::Up => Some(Key::Up),
            Keysym::Down => Some(Key::Down),
            Keysym::Left => Some(Key::Left),
            Keysym::Right => Some(Key::Right),
            Keysym::Home => Some(Key::Home),
            Keysym::End => Some(Key::End),
            Keysym::Page_Up => Some(Key::PageUp),
            Keysym::Page_Down => Some(Key::PageDown),
            Keysym::F2 => Some(Key::F2),
            Keysym::F5 => Some(Key::F5),
            // with Ctrl held the text is a control character; shortcuts want the plain letter
            _ if mods.ctrl => event.keysym.key_char().map(|c| Key::Char(c.to_ascii_lowercase())),
            _ => event.utf8.as_deref().and_then(|t| t.chars().find(|c| !c.is_control())).map(Key::Char),
        };
        let Some(key) = key else { return };
        if !self.state.key(key, mods) {
            self.app.key(key, mods, &mut self.host);
        }
        // Ctrl+C / X / V in a text field: the clipboard is the runtime's to reach
        if let Some(t) = self.state.clip_out.take() {
            self.host.copy = Some(t);
        }
        if std::mem::take(&mut self.state.want_paste) {
            self.host.paste = true;
        }
        self.schedule();
    }
}

impl<A: App + 'static> CompositorHandler for Runtime<A> {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, new_factor: i32) {
        if new_factor != self.scale {
            self.scale = new_factor.max(1);
            self.schedule();
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_pending = false;
        if self.dirty {
            self.draw();
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl<A: App + 'static> OutputHandler for Runtime<A> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl<A: App + 'static> WindowHandler for Runtime<A> {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.host.quit = true;
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _: u32) {
        let (w, h) = (configure.new_size.0.map(|v| v.get()).unwrap_or(self.width), configure.new_size.1.map(|v| v.get()).unwrap_or(self.height));
        let changed = (w, h) != (self.width, self.height);
        self.width = w;
        self.height = h;
        self.host.maximized = configure.state.contains(WindowState::MAXIMIZED);
        self.host.activated = configure.is_activated();
        let first = !self.configured;
        self.configured = true;
        if first || changed {
            self.schedule();
        }
    }
}

impl<A: App + 'static> SeatHandler for Runtime<A> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if self.data_device.is_none() {
            self.data_device = Some(self.data_device_manager.get_data_device(qh, &seat));
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let kb = self.seat_state.get_keyboard_with_repeat(qh, &seat, None, self.loop_handle.clone(), Box::new(|rt: &mut Runtime<A>, _kb, event| rt.on_key(event))).expect("keyboard");
            self.keyboard = Some(kb);
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            let ptr = self.seat_state.get_pointer(qh, &seat).expect("pointer");
            self.shape_device = self.shape_manager.as_ref().map(|m| m.get_shape_device(&ptr, qh));
            self.pointer = Some(ptr);
            self.seat = Some(seat.clone());
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl<A: App + 'static> KeyboardHandler for Runtime<A> {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, serial: u32, event: KeyEvent) {
        self.input_serial = serial;
        self.on_key(event);
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: u32) {
        self.mods = modifiers;
        self.state.mods = KeyMods { ctrl: modifiers.ctrl, alt: modifiers.alt, shift: modifiers.shift };
    }
}

impl<A: App + 'static> PointerHandler for Runtime<A> {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for ev in events {
            if &ev.surface != self.window.wl_surface() {
                continue;
            }
            let p = [ev.position.0 as f32, ev.position.1 as f32];
            if let PointerEventKind::Enter { serial } = ev.kind {
                self.enter_serial = serial;
                // a new enter forgets the shape: it has to be named again
                self.shown_cursor = None;
            }
            if let PointerEventKind::Press { serial, .. } = ev.kind {
                self.serial = serial;
                self.input_serial = serial;
            }
            // a program that reads the pointer itself gets every event first
            let kind = match ev.kind {
                PointerEventKind::Enter { .. } => PointerKind::Enter,
                PointerEventKind::Leave { .. } => PointerKind::Leave,
                PointerEventKind::Motion { .. } => PointerKind::Motion,
                PointerEventKind::Press { button, serial, .. } => PointerKind::Press { button, serial },
                PointerEventKind::Release { button, .. } => PointerKind::Release { button },
                PointerEventKind::Axis { horizontal, vertical, .. } => PointerKind::Axis {
                    dx: horizontal.absolute as f32,
                    dy: vertical.absolute as f32,
                    discrete_x: horizontal.discrete,
                    discrete_y: vertical.discrete,
                },
            };
            if self.app.raw_pointer(&PointerEvt { pos: p, kind }, &self.mods, &mut self.host) {
                continue;
            }
            match ev.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => self.state.pointer_move(p),
                PointerEventKind::Leave { .. } => self.state.pointer_move([-1000.0; 2]),
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => self.state.pointer_down(p),
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => self.state.pointer_up(),
                PointerEventKind::Axis { vertical, .. } => {
                    let dy = if vertical.discrete != 0 { vertical.discrete as f32 * 40.0 } else { vertical.absolute as f32 };
                    self.state.pointer_scroll(dy);
                }
                _ => {}
            }
        }
        // a move or resize the press asked for goes out with this press's serial now, not at the next frame (which may be
        // far off while one is pending), when the pointer may be elsewhere and the serial is stale
        self.window_ops();
        self.schedule();
    }
}

impl<A: App + 'static> DataDeviceHandler for Runtime<A> {
    // a drag came over the window: take files (or text) as a copy
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64, _: &wl_surface::WlSurface) {
        let Some(offer) = self.data_device.as_ref().and_then(|dd| dd.data().drag_offer()) else { return };
        match drop_mime(&offer) {
            Some(mime) => {
                offer.set_actions(DndAction::Copy, DndAction::Copy);
                offer.accept_mime_type(offer.serial, Some(mime));
            }
            None => offer.accept_mime_type(offer.serial, None),
        }
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
    // the compositor also reports our own `set_selection` back here, so this cannot tell that someone else took the
    // clipboard (dropping the source then destroyed our own copy: a paste found nothing); `cancelled` says that
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        let Some(offer) = self.data_device.as_ref().and_then(|dd| dd.data().drag_offer()) else { return };
        let Some(mime) = drop_mime(&offer) else { return };
        let Ok(mut pipe) = offer.receive(mime.clone()) else { return };
        let out = self.drop_out.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if pipe.read_to_end(&mut buf).is_ok() {
                let text = String::from_utf8_lossy(&buf).into_owned();
                if mime == "text/uri-list" {
                    out.send((text.lines().filter_map(file_from_uri).collect(), String::new()));
                } else {
                    out.send((Vec::new(), text));
                }
            }
            offer.finish();
        });
    }
}

/// What to ask a drag for: the files when it has them, else its text.
fn drop_mime(offer: &DragOffer) -> Option<String> {
    offer.with_mime_types(|m| {
        ["text/uri-list"].iter().chain(COPY_MIMES.iter()).find(|c| m.iter().any(|x| x == **c)).map(|s| s.to_string())
    })
}

/// The path in a `file://` line of a `text/uri-list` (percent escapes decoded).
fn file_from_uri(line: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = line.trim().strip_prefix("file://")?;
    let rest = &rest[rest.find('/')?..];
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(out)))
}

impl<A: App + 'static> DataOfferHandler for Runtime<A> {
    fn source_actions(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
    fn selected_action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
}

impl<A: App + 'static> DataSourceHandler for Runtime<A> {
    fn accept_mime(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: Option<String>) {}

    fn send_request(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, mime: String, fd: WritePipe) {
        let mut f = std::fs::File::from(OwnedFd::from(fd));
        let _ = if mime == "image/png" { f.write_all(&self.clip_png) } else { f.write_all(self.clip_text.as_bytes()) };
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self.copy_source.as_ref().is_some_and(|s| s.inner() == source) {
            self.copy_source = None;
        }
        source.destroy();
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

impl<A: App + 'static> ShmHandler for Runtime<A> {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl<A: App + 'static> ProvidesRegistryState for Runtime<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(@<A: App + 'static> Runtime<A>);
wayland_client::delegate_noop!(@<A: App + 'static> Runtime<A>: ignore WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(@<A: App + 'static> Runtime<A>: ignore WpViewporter);
wayland_client::delegate_noop!(@<A: App + 'static> Runtime<A>: ignore WpViewport);

impl<A: App + 'static> wayland_client::Dispatch<WpFractionalScaleV1, ()> for Runtime<A> {
    fn event(state: &mut Self, _: &WpFractionalScaleV1, event: wp_fractional_scale_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // the scale the desktop wants, in 120ths
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let s = scale as f32 / 120.0;
            if state.frac != Some(s) {
                state.frac = Some(s);
                state.schedule();
            }
        }
    }
}
delegate_output!(@<A: App + 'static> Runtime<A>);
delegate_shm!(@<A: App + 'static> Runtime<A>);
delegate_seat!(@<A: App + 'static> Runtime<A>);
delegate_data_device!(@<A: App + 'static> Runtime<A>);
delegate_keyboard!(@<A: App + 'static> Runtime<A>);
delegate_pointer!(@<A: App + 'static> Runtime<A>);
delegate_xdg_shell!(@<A: App + 'static> Runtime<A>);
delegate_xdg_window!(@<A: App + 'static> Runtime<A>);
delegate_registry!(@<A: App + 'static> Runtime<A>);

/// Open the window and run until it is closed. Panics when there is no Wayland display.
pub fn run<A: App + 'static>(config: Config, app: A) {
    crate::mem::tame_allocator();
    let conn = Connection::connect_to_env().expect("no Wayland display (WAYLAND_DISPLAY)");
    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<Runtime<A>> = EventLoop::try_new().unwrap();
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue).insert(loop_handle.clone()).unwrap();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg_shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let data_device_manager = DataDeviceManagerState::bind(&globals, &qh).expect("wl_data_device_manager");
    let (paste_out, paste_in) = channel::<String>();
    let (drop_out, drop_in) = channel::<(Vec<std::path::PathBuf>, String)>();
    let surface = compositor.create_surface(&qh);
    // the compositor draws the frame, so the program only paints its content
    let decorations = match config.decorations {
        Decorations::Server => WindowDecorations::RequestServer,
        Decorations::Client => WindowDecorations::RequestClient,
        Decorations::None => WindowDecorations::None,
    };
    let window = xdg_shell.create_window(surface, decorations, &qh);
    // a desktop that speaks the fractional scale lets the program draw for exactly that
    let viewport = globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok().map(|v| v.get_viewport(window.wl_surface(), &qh, ()));
    let fractional = globals.bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ()).ok().map(|m| m.get_fractional_scale(window.wl_surface(), &qh, ()));
    window.set_title(config.title.clone());
    window.set_app_id(config.app_id.clone());
    window.set_min_size(Some(config.min_size));
    if !config.resizable {
        window.set_min_size(Some(config.size));
        window.set_max_size(Some(config.size));
    }
    window.commit();

    let look = Appearance::load();
    crate::i18n::set_language(&look.language);
    let (width, height) = config.size;
    let pool = SlotPool::new((width * height * 4) as usize, &shm).expect("shm pool");
    let mut rt = Runtime {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        window,
        qh: qh.clone(),
        loop_handle: loop_handle.clone(),
        width,
        height,
        scale: 1,
        frac: None,
        viewport,
        _fractional: fractional,
        configured: false,
        dirty: false,
        frame_pending: false,
        keyboard: None,
        pointer: None,
        shape_manager: smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager::bind(&globals, &qh).ok(),
        shape_device: None,
        enter_serial: 0,
        shown_cursor: None,
        mods: Modifiers::default(),
        state: State::default(),
        gfx: Gfx::new(),
        frame: Frame::default(),
        host: Host { appearance: look, fixed: !config.resizable, ..Host::default() },
        app,
        seat: None,
        serial: 0,
        look_stamp: Appearance::stamp(),
        data_device_manager,
        data_device: None,
        copy_source: None,
        clip_text: String::new(),
        clip_png: Vec::new(),
        input_serial: 0,
        paste_out,
        paste_in,
        drop_out,
        drop_in,
        perf: Perf { on: std::env::var_os("CHEBRIUM_PERF").is_some(), ..Default::default() },
        last_trim: std::time::Instant::now(),
        menu: None,
    };
    let (ping, source) = smithay_client_toolkit::reexports::calloop::ping::make_ping().expect("wake-up channel");
    loop_handle.insert_source(source, |_, _, rt: &mut Runtime<A>| rt.schedule()).expect("wake-up source");
    rt.paste_in.connect(Waker(ping.clone()));
    rt.drop_in.connect(Waker(ping.clone()));
    if !rt.app.menus().is_empty() {
        let waker = Waker(ping.clone());
        rt.menu = crate::appmenu::Client::connect(&config.app_id, move || waker.wake());
    }
    rt.app.start(Waker(ping));
    while !rt.host.quit {
        if event_loop.dispatch(Duration::from_millis(250), &mut rt).is_err() {
            break;
        }
        rt.follow_look();
    }
}
