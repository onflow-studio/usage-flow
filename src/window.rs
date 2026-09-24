//! Native NSWindow tweaks eframe doesn't expose.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSScreen, NSView, NSWindow, NSWindowCollectionBehavior};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::PathBuf;

fn ns_window(frame: &eframe::Frame) -> Option<Retained<NSWindow>> {
    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return None };
    // SAFETY: eframe hands us the live NSView of our window; this runs on the main thread.
    let view: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
    view.window()
}

/// Full height on the left edge of the display it was last on (or its current one),
/// between the menu bar and the Dock.
pub fn dock_left(frame: &eframe::Frame, width: f32) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(window) = ns_window(frame) else { return };
    let saved = std::fs::read_to_string(settings_dir().join("display")).ok();
    let screens = NSScreen::screens(mtm);
    let remembered = saved.and_then(|name| {
        (0..screens.count())
            .map(|i| screens.objectAtIndex(i))
            .find(|s| s.localizedName().to_string() == name)
    });
    let Some(screen) = remembered.or_else(|| window.screen()) else { return };
    dock_on(&window, &screen, width);
}

pub fn current_display(frame: &eframe::Frame) -> Option<String> {
    Some(ns_window(frame)?.screen()?.localizedName().to_string())
}

pub fn save_display(name: &str) {
    let dir = settings_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("display"), name);
}

/// Docks the window on the next display, wrapping around.
pub fn dock_next_screen(frame: &eframe::Frame, width: f32) {
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
    dock_on(&window, &next, width);
}

pub fn screen_count() -> usize {
    MainThreadMarker::new().map_or(1, |mtm| NSScreen::screens(mtm).count())
}

fn dock_on(window: &NSWindow, screen: &NSScreen, width: f32) {
    let area = screen.visibleFrame();
    let rect = NSRect::new(
        NSPoint::new(area.origin.x, area.origin.y),
        NSSize::new(width as f64, area.size.height),
    );
    window.setFrame_display(rect, true);
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

fn settings_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support/claude-usage")
}

fn settings_path() -> PathBuf {
    settings_dir().join("pinned")
}

pub fn load_pinned() -> bool {
    settings_path().exists()
}

pub fn save_pinned(pinned: bool) {
    let path = settings_path();
    if pinned {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = std::fs::write(path, "");
    } else {
        let _ = std::fs::remove_file(path);
    }
}
