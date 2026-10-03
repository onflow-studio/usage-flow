//! Native NSWindow tweaks eframe doesn't expose.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSEvent, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn ns_window(frame: &eframe::Frame) -> Option<Retained<NSWindow>> {
    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return None };
    // SAFETY: eframe hands us the live NSView of our window; this runs on the main thread.
    let view: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
    view.window()
}

/// The edge of the display the panel sticks to.
#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Left,
    Right,
}

/// Full height on `side` of the display it was last on (or its current one), between the menu
/// bar and the Dock.
pub fn dock(frame: &eframe::Frame, width: f32, side: Side) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(window) = ns_window(frame) else { return };
    let saved = crate::settings::display();
    let screens = NSScreen::screens(mtm);
    let remembered = saved.and_then(|name| {
        (0..screens.count())
            .map(|i| screens.objectAtIndex(i))
            .find(|s| s.localizedName().to_string() == name)
    });
    let Some(screen) = remembered.or_else(|| window.screen()) else { return };
    dock_on(&window, &screen, width, side);
}

/// The window's place on screen in top-left coordinates, as the window server counts them:
/// `(x, y, width, height)`.
pub fn screen_rect(frame: &eframe::Frame) -> Option<(f64, f64, f64, f64)> {
    let window = ns_window(frame)?;
    // AppKit counts up from the bottom of the primary display, which is the first one.
    let primary = NSScreen::screens(MainThreadMarker::new()?).firstObject()?.frame();
    let rect = window.frame();
    Some((rect.origin.x, primary.size.height - rect.origin.y - rect.size.height, rect.size.width, rect.size.height))
}

pub fn current_display(frame: &eframe::Frame) -> Option<String> {
    Some(ns_window(frame)?.screen()?.localizedName().to_string())
}

/// Docks the window on the next display, wrapping around.
pub fn dock_next_screen(frame: &eframe::Frame, width: f32, side: Side) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(window) = ns_window(frame) else { return };
    let screens = NSScreen::screens(mtm);
    if screens.count() < 2 {
        return;
    }
    let current = window.screen();
    let index = (0..screens.count())
        .find(|&i| current.as_deref().is_some_and(|c| std::ptr::eq(c, &*screens.objectAtIndex(i))))
        .unwrap_or(0);
    let next = screens.objectAtIndex((index + 1) % screens.count());
    dock_on(&window, &next, width, side);
}

pub fn screen_count() -> usize {
    MainThreadMarker::new().map_or(1, |mtm| NSScreen::screens(mtm).count())
}

/// Full height of `screen` between the menu bar and the Dock, against `side`.
fn dock_on(window: &NSWindow, screen: &NSScreen, width: f32, side: Side) {
    let area = screen.visibleFrame();
    let x = match side {
        Side::Left => area.origin.x,
        Side::Right => area.origin.x + area.size.width - width as f64,
    };
    let rect = NSRect::new(
        NSPoint::new(x, area.origin.y),
        NSSize::new(width as f64, area.size.height),
    );
    window.setFrame_display(rect, true);
}

thread_local! {
    /// Window frame and display area as of the last fit, so an unchanged window is left alone.
    static FITTED: std::cell::Cell<Option<(NSRect, NSRect)>> = const { std::cell::Cell::new(None) };
}

/// Snaps the window back to an edge of the display it sits on whenever either changed, e.g.
/// after being dragged: full height, against whichever side it was dropped nearer to. Returns
/// that side when it had to move the window.
pub fn keep_docked(frame: &eframe::Frame, width: f32) -> Option<Side> {
    // Mid-drag the window still belongs to the mouse.
    if NSEvent::pressedMouseButtons() != 0 {
        return None;
    }
    let window = ns_window(frame)?;
    let screen = window.screen()?;
    let area = screen.visibleFrame();
    let current = window.frame();
    if FITTED.get() == Some((current, area)) {
        return None;
    }
    let middle = current.origin.x + current.size.width / 2.0;
    let side = if middle > area.origin.x + area.size.width / 2.0 { Side::Right } else { Side::Left };
    dock_on(&window, &screen, width, side);
    // macOS may constrain the frame we asked for, so remember the one it settled on.
    FITTED.set(Some((window.frame(), area)));
    Some(side)
}

/// "Pinned" = shown on every desktop (Space) and over full-screen apps.
pub fn set_pinned(frame: &eframe::Frame, pinned: bool) {
    let Some(window) = ns_window(frame) else { return };
    let behavior = if pinned {
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
    } else {
        NSWindowCollectionBehavior::Default
    };
    window.setCollectionBehavior(behavior);
}
