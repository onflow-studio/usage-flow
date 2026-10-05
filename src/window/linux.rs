//! X11 window tweaks eframe doesn't expose. A Wayland client can neither place its own window
//! nor reserve a strip of the screen, so under Wayland the app runs through XWayland.

use super::Side;
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, EventMask, MapState, PropMode,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_CURRENT_DESKTOP,
        _NET_WORKAREA,
        _NET_WM_STATE,
        _NET_WM_STATE_STICKY,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_STRUT,
        _NET_WM_STRUT_PARTIAL,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DOCK,
        _NET_WM_WINDOW_TYPE_NORMAL,
    }
}

/// How long the window manager is given to act on a request before the window is read again.
const SETTLE: Duration = Duration::from_millis(400);
/// How long a reading of the displays is good for.
const MONITORS_TTL: Duration = Duration::from_secs(1);
/// The left, middle and right mouse buttons in a pointer's state.
const BUTTONS: u16 = 0x0700;

struct X {
    conn: RustConnection,
    root: u32,
    atoms: Atoms,
}

/// A connection of the app's own, beside the one winit keeps to itself.
fn x() -> Option<&'static X> {
    static X: OnceLock<Option<X>> = OnceLock::new();
    X.get_or_init(|| {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
        Some(X { conn, root, atoms })
    })
    .as_ref()
}

/// In root window pixels, counted from the top left.
#[derive(Clone, Copy, PartialEq)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.width
    }

    fn bottom(&self) -> i32 {
        self.y + self.height
    }

    fn holds(&self, other: &Rect) -> bool {
        other.width > 0
            && other.height > 0
            && other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right() && self.right() > other.x && self.y < other.bottom() && self.bottom() > other.y
    }

    fn intersection(&self, other: &Rect) -> Option<Rect> {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let (width, height) = (self.right().min(other.right()) - x, self.bottom().min(other.bottom()) - y);
        (width > 0 && height > 0).then_some(Rect { x, y, width, height })
    }
}

#[derive(Clone)]
struct Monitor {
    name: String,
    rect: Rect,
}

/// The panel's window, for the calls that come without a frame.
static WINDOW: AtomicU32 = AtomicU32::new(0);
/// Whether the panel is asked to show on every workspace.
static PINNED: AtomicBool = AtomicBool::new(false);
/// Window frame and display area as of the last fit, so an unchanged window is left alone.
static FITTED: Mutex<Option<(Rect, Rect)>> = Mutex::new(None);
/// The display area the window was last docked for. The window manager acts on it in its own
/// time, and may settle on another frame than the one asked for.
static ASKED: Mutex<Option<Rect>> = Mutex::new(None);
/// When the window manager was last asked for something.
static CHANGED: Mutex<Option<Instant>> = Mutex::new(None);
/// The strip reserved for the panel: its frame when it was reserved, and its side.
static RESERVED: Mutex<Option<(Rect, Side)>> = Mutex::new(None);

fn window_id(frame: &eframe::Frame) -> Option<u32> {
    let id = match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(handle) => handle.window as u32,
        RawWindowHandle::Xcb(handle) => handle.window.get(),
        _ => return None,
    };
    WINDOW.store(id, Ordering::SeqCst);
    Some(id)
}

fn changed() {
    *CHANGED.lock().unwrap() = Some(Instant::now());
}

fn settling() -> bool {
    CHANGED.lock().unwrap().is_some_and(|at| at.elapsed() < SETTLE)
}

fn monitors(x: &X) -> Vec<Monitor> {
    static CACHE: Mutex<Option<(Instant, Vec<Monitor>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, monitors)) = &*cache {
        if at.elapsed() < MONITORS_TTL {
            return monitors.clone();
        }
    }
    let found = read_monitors(x).unwrap_or_default();
    *cache = Some((Instant::now(), found.clone()));
    found
}

/// The displays, the primary one first.
fn read_monitors(x: &X) -> Option<Vec<Monitor>> {
    let mut found = x.conn.randr_get_monitors(x.root, true).ok()?.reply().ok()?.monitors;
    found.sort_by_key(|m| !m.primary);
    let monitors = found.iter().map(|m| {
        let name = x.conn.get_atom_name(m.name).ok().and_then(|cookie| cookie.reply().ok());
        Monitor {
            name: name.map(|n| String::from_utf8_lossy(&n.name).into_owned()).unwrap_or_default(),
            rect: Rect { x: m.x as i32, y: m.y as i32, width: m.width as i32, height: m.height as i32 },
        }
    });
    Some(monitors.collect())
}

fn monitor_of<'a>(monitors: &'a [Monitor], window: &Rect) -> Option<&'a Monitor> {
    let (x, y) = (window.x + window.width / 2, window.y + window.height / 2);
    let holds = |m: &&Monitor| x >= m.rect.x && x < m.rect.right() && y >= m.rect.y && y < m.rect.bottom();
    monitors.iter().find(holds).or(monitors.first())
}

fn values(x: &X, window: u32, property: u32, kind: AtomEnum) -> Vec<u32> {
    x.conn
        .get_property(false, window, property, kind, 0, 1024)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().map(|values| values.collect()))
        .unwrap_or_default()
}

fn rects(values: &[u32]) -> Vec<Rect> {
    values
        .chunks_exact(4)
        .map(|v| Rect { x: v[0] as i32, y: v[1] as i32, width: v[2] as i32, height: v[3] as i32 })
        .collect()
}

/// The part of `monitor` the desktop's own panels and docks leave free.
fn work_area(x: &X, monitor: Rect) -> Rect {
    let desktop = values(x, x.root, x.atoms._NET_CURRENT_DESKTOP, AtomEnum::CARDINAL).first().copied().unwrap_or(0);
    // Mutter, GNOME's window manager, publishes a work area per display.
    let per_monitor = x
        .conn
        .intern_atom(true, format!("_GTK_WORKAREAS_D{desktop}").as_bytes())
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .filter(|reply| reply.atom != 0)
        .map(|reply| values(x, x.root, reply.atom, AtomEnum::CARDINAL))
        .unwrap_or_default();
    if let Some(area) = rects(&per_monitor).into_iter().find(|area| monitor.holds(area)) {
        return area;
    }
    // The standard has one work area for the whole desktop: right for a single display, and
    // for any other only as far as it reaches into it.
    rects(&values(x, x.root, x.atoms._NET_WORKAREA, AtomEnum::CARDINAL))
        .get(desktop as usize)
        .and_then(|area| area.intersection(&monitor))
        .unwrap_or(monitor)
}

/// The room the panel has on `monitor`: its work area, plus the strip reserved for the panel
/// itself once the work area has shrunk by it.
fn area(x: &X, monitor: Rect) -> Rect {
    let mut area = work_area(x, monitor);
    if let Some((panel, side)) = *RESERVED.lock().unwrap() {
        if monitor.holds(&panel) && side == Side::Left && area.x == panel.right() {
            area.x = panel.x;
            area.width += panel.width;
        } else if monitor.holds(&panel) && side == Side::Right && area.right() == panel.x {
            area.width += panel.width;
        }
    }
    area
}

fn frame_of(x: &X, window: u32) -> Option<Rect> {
    let size = x.conn.get_geometry(window).ok()?.reply().ok()?;
    let at = x.conn.translate_coordinates(window, x.root, 0, 0).ok()?.reply().ok()?;
    Some(Rect { x: at.dst_x as i32, y: at.dst_y as i32, width: size.width as i32, height: size.height as i32 })
}

fn viewable(x: &X, window: u32) -> bool {
    let attributes = x.conn.get_window_attributes(window).ok().and_then(|cookie| cookie.reply().ok());
    attributes.is_some_and(|a| a.map_state == MapState::VIEWABLE)
}

/// egui counts in points and X11 in pixels.
pub fn units_per_point(ctx: &egui::Context) -> f32 {
    ctx.native_pixels_per_point().unwrap_or(1.0)
}

/// Full height on `side` of the display it was last on (or its current one), between the
/// desktop's own panels.
pub fn dock(frame: &eframe::Frame, width: f32, side: Side) {
    let (Some(x), Some(window)) = (x(), window_id(frame)) else { return };
    let monitors = monitors(x);
    let saved = crate::settings::display();
    let remembered = saved.and_then(|name| monitors.iter().find(|m| m.name == name));
    let current = || frame_of(x, window).and_then(|frame| monitor_of(&monitors, &frame));
    let Some(monitor) = remembered.or_else(current) else { return };
    dock_on(x, window, monitor.rect, width, side);
}

/// The window's place on screen in top-left coordinates: `(x, y, width, height)`.
pub fn screen_rect(frame: &eframe::Frame) -> Option<(f64, f64, f64, f64)> {
    let rect = frame_of(x()?, window_id(frame)?)?;
    Some((rect.x as f64, rect.y as f64, rect.width as f64, rect.height as f64))
}

pub fn current_display(frame: &eframe::Frame) -> Option<String> {
    let x = x()?;
    let rect = frame_of(x, window_id(frame)?)?;
    Some(monitor_of(&monitors(x), &rect)?.name.clone())
}

/// Docks the window on the next display, wrapping around.
pub fn dock_next_screen(frame: &eframe::Frame, width: f32, side: Side) {
    let (Some(x), Some(window)) = (x(), window_id(frame)) else { return };
    let monitors = monitors(x);
    if monitors.len() < 2 {
        return;
    }
    let current = frame_of(x, window).and_then(|frame| monitor_of(&monitors, &frame).map(|m| m.rect));
    let index = monitors.iter().position(|m| Some(m.rect) == current).unwrap_or(0);
    dock_on(x, window, monitors[(index + 1) % monitors.len()].rect, width, side);
}

pub fn screen_count() -> usize {
    x().map_or(1, |x| monitors(x).len().max(1))
}

/// Full height of `monitor` between the desktop's own panels, against `side`.
fn dock_on(x: &X, window: u32, monitor: Rect, width: f32, side: Side) {
    let area = area(x, monitor);
    let width = width.round() as i32;
    let left = match side {
        Side::Left => area.x,
        Side::Right => area.right() - width,
    };
    // The window is not resizable, which the window manager holds it to: the new size is
    // declared before it is asked for. The position goes with it, so the window comes back to
    // its place after being hidden.
    let mut hints = WmSizeHints::new();
    hints.position = Some((WmSizeHintsSpecification::UserSpecified, left, area.y));
    hints.size = Some((WmSizeHintsSpecification::UserSpecified, width, area.height));
    hints.min_size = Some((width, area.height));
    hints.max_size = Some((width, area.height));
    let _ = hints.set_normal_hints(&x.conn, window);
    let place = ConfigureWindowAux::new().x(left).y(area.y).width(width as u32).height(area.height as u32);
    let _ = x.conn.configure_window(window, &place);
    let _ = x.conn.flush();
    *ASKED.lock().unwrap() = Some(area);
    changed();
}

/// Snaps the window back to an edge of the display it sits on whenever either changed, e.g.
/// after being dragged: full height, against whichever side it was dropped nearer to. Returns
/// that side when it had to move the window.
pub fn keep_docked(frame: &eframe::Frame, width: f32) -> Option<Side> {
    let x = x()?;
    let window = window_id(frame)?;
    if !viewable(x, window) || settling() {
        return None;
    }
    // Whatever was asked for has been acted on by now, so it vouches for this reading alone.
    let asked = ASKED.lock().unwrap().take();
    apply_state(x, window);
    // Mid-drag the window still belongs to the mouse.
    let pointer = x.conn.query_pointer(x.root).ok()?.reply().ok()?;
    if u16::from(pointer.mask) & BUTTONS != 0 {
        return None;
    }
    let current = frame_of(x, window)?;
    let monitor = monitor_of(&monitors(x), &current)?.rect;
    let area = area(x, monitor);
    let mut fitted = FITTED.lock().unwrap();
    if *fitted == Some((current, area)) {
        return None;
    }
    // Already docked for this area: the frame the window manager settled on is the one to keep.
    if asked == Some(area) {
        *fitted = Some((current, area));
        return None;
    }
    let middle = current.x + current.width / 2;
    let side = if middle > area.x + area.width / 2 { Side::Right } else { Side::Left };
    dock_on(x, window, monitor, width, side);
    Some(side)
}

/// "Pinned" = shown on every workspace.
pub fn set_pinned(frame: &eframe::Frame, pinned: bool) {
    PINNED.store(pinned, Ordering::SeqCst);
    if let (Some(x), Some(window)) = (x(), window_id(frame)) {
        if viewable(x, window) {
            apply_state(x, window);
        }
    }
}

/// Keeps the panel out of the taskbar and the window switcher, and on every workspace while
/// pinned. The window manager drops these states when the window is hidden, so they are checked
/// against the ones it holds.
fn apply_state(x: &X, window: u32) {
    let held = values(x, window, x.atoms._NET_WM_STATE, AtomEnum::ATOM);
    let ask = |state: u32, on: bool| {
        if held.contains(&state) != on {
            let message = ClientMessageEvent::new(32, window, x.atoms._NET_WM_STATE, [on as u32, state, 0, 1, 0]);
            let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
            let _ = x.conn.send_event(false, x.root, mask, message);
        }
    };
    ask(x.atoms._NET_WM_STATE_SKIP_TASKBAR, true);
    ask(x.atoms._NET_WM_STATE_SKIP_PAGER, true);
    // A dock is on every workspace as it is.
    if RESERVED.lock().unwrap().is_none() {
        ask(x.atoms._NET_WM_STATE_STICKY, PINNED.load(Ordering::SeqCst));
    }
    let _ = x.conn.flush();
}

/// Reserves the strip of screen the panel is docked on, so the window manager keeps other
/// windows out of it, or gives it back with `None`. While it holds the strip the panel is a
/// dock to the window manager: on every workspace, and not to be dragged.
pub fn reserve(panel: Option<((f64, f64, f64, f64), Side)>) {
    let (Some(x), window) = (x(), WINDOW.load(Ordering::SeqCst)) else { return };
    if window == 0 {
        return;
    }
    let wanted = match panel {
        Some(((left, top, width, height), side)) => {
            let rect = Rect { x: left as i32, y: top as i32, width: width as i32, height: height as i32 };
            // Only a panel at rest in its docked place has a strip to reserve.
            let docked = FITTED.lock().unwrap().is_some_and(|(frame, _)| frame == rect);
            if settling() || !docked {
                return;
            }
            strut(x, &rect, side).map(|strut| (rect, side, strut))
        }
        None => None,
    };
    let mut reserved = RESERVED.lock().unwrap();
    if *reserved == wanted.map(|(rect, side, _)| (rect, side)) {
        return;
    }
    let atoms = &x.atoms;
    let kind = match wanted {
        Some((_, _, strut)) => {
            let _ = x.conn.change_property32(PropMode::REPLACE, window, atoms._NET_WM_STRUT_PARTIAL, AtomEnum::CARDINAL, &strut);
            let _ = x.conn.change_property32(PropMode::REPLACE, window, atoms._NET_WM_STRUT, AtomEnum::CARDINAL, &strut[..4]);
            atoms._NET_WM_WINDOW_TYPE_DOCK
        }
        None => {
            let _ = x.conn.delete_property(window, atoms._NET_WM_STRUT_PARTIAL);
            let _ = x.conn.delete_property(window, atoms._NET_WM_STRUT);
            atoms._NET_WM_WINDOW_TYPE_NORMAL
        }
    };
    let _ = x.conn.change_property32(PropMode::REPLACE, window, atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM, &[kind]);
    let _ = x.conn.flush();
    *reserved = wanted.map(|(rect, side, _)| (rect, side));
    changed();
}

/// The strip from the screen's edge to the panel's inner one, as `_NET_WM_STRUT_PARTIAL` counts
/// it. X11 measures it from the edge of the whole screen, every display together, so a panel
/// with another display beyond it has no strip to reserve: it would take that display with it.
fn strut(x: &X, panel: &Rect, side: Side) -> Option<[u32; 12]> {
    let screen = x.conn.get_geometry(x.root).ok()?.reply().ok()?;
    let monitors = monitors(x);
    let own = monitor_of(&monitors, panel)?.rect;
    let strip = match side {
        Side::Left => Rect { x: 0, width: panel.right(), ..*panel },
        Side::Right => Rect { width: screen.width as i32 - panel.x, ..*panel },
    };
    if monitors.iter().any(|m| m.rect != own && m.rect.overlaps(&strip)) {
        return None;
    }
    let mut strut = [0; 12];
    let (thickness, start) = match side {
        Side::Left => (0, 4),
        Side::Right => (1, 6),
    };
    strut[thickness] = strip.width as u32;
    strut[start] = panel.y as u32;
    strut[start + 1] = (panel.bottom() - 1) as u32;
    Some(strut)
}
