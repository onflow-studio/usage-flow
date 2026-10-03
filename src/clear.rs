//! Keeps other apps' windows out from under the panel. macOS gives an app no way to reserve a
//! strip of the screen, so this moves and narrows the windows that overlap it instead, which
//! takes the Accessibility permission.

use crate::window::Side;
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::{CFNumber, CFNumberRef};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
};
use std::ffi::c_void;
use std::sync::Mutex;
use std::time::Duration;

type AXUIElementRef = CFTypeRef;

const AX_VALUE_POINT: u32 = 1;
const AX_VALUE_SIZE: u32 = 2;
const SETTINGS_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const CHECK_INTERVAL: Duration = Duration::from_millis(700);
/// A window is narrowed to make room only down to this width; past it, it is moved instead.
const MIN_WIDTH: f64 = 480.0;
/// An app that does not answer in this long is skipped until the next check.
const APP_TIMEOUT_SECS: f32 = 0.3;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> u8;
    fn CGEventSourceButtonState(state: i32, button: u32) -> u8;
}

/// The panel's place on screen, in top-left coordinates, and the edge it is docked against.
static PANEL: Mutex<Option<(CGRect, Side)>> = Mutex::new(None);

/// Whether macOS lets this app move other apps' windows.
pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Asks for the Accessibility permission and opens the settings pane where it is granted.
pub fn request() {
    let options = CFDictionary::from_CFType_pairs(&[(
        CFString::new("AXTrustedCheckOptionPrompt").as_CFType(),
        CFBoolean::true_value().as_CFType(),
    )]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) };
    let _ = std::process::Command::new("open").arg(SETTINGS_PANE).spawn();
}

/// Where the panel is, `None` while windows should be left alone.
pub fn set_panel(panel: Option<((f64, f64, f64, f64), Side)>) {
    *PANEL.lock().unwrap() = panel.map(|((x, y, width, height), side)| {
        (CGRect::new(&CGPoint::new(x, y), &CGSize::new(width, height)), side)
    });
}

pub fn spawn() {
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(CHECK_INTERVAL);
            let Some((panel, side)) = *PANEL.lock().unwrap() else { continue };
            // A pressed button means a window may be mid-drag: it settles first.
            let dragging = unsafe { CGEventSourceButtonState(0, 0) != 0 };
            if dragging || !trusted() {
                continue;
            }
            for pid in overlapping_apps(&panel) {
                clear_app(pid, &panel, side);
            }
        }
    });
}

fn overlaps(window: &CGRect, panel: &CGRect) -> bool {
    window.origin.x < panel.origin.x + panel.size.width
        && window.origin.x + window.size.width > panel.origin.x
        && window.origin.y < panel.origin.y + panel.size.height
        && window.origin.y + window.size.height > panel.origin.y
}

/// The apps with an ordinary window on screen that overlaps the panel. The window list is cheap
/// to read, so the apps that need no change are never asked anything.
fn overlapping_apps(panel: &CGRect) -> Vec<i32> {
    let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let Some(windows) = copy_window_info(options, kCGNullWindowID) else { return Vec::new() };
    let own = std::process::id() as i32;
    let mut pids = Vec::new();
    for window in windows.iter() {
        let window: CFDictionary = unsafe { CFDictionary::wrap_under_get_rule(*window as CFDictionaryRef) };
        let number = |key: CFStringRef| {
            let value = window.find(key as *const c_void)?;
            unsafe { CFNumber::wrap_under_get_rule(*value as CFNumberRef) }.to_i32()
        };
        let (Some(pid), Some(0)) = (number(unsafe { kCGWindowOwnerPID }), number(unsafe { kCGWindowLayer })) else {
            continue;
        };
        let bounds = window.find(unsafe { kCGWindowBounds } as *const c_void).and_then(|bounds| {
            CGRect::from_dict_representation(&unsafe { CFDictionary::wrap_under_get_rule(*bounds as CFDictionaryRef) })
        });
        if pid != own && !pids.contains(&pid) && bounds.is_some_and(|b| overlaps(&b, panel)) {
            pids.push(pid);
        }
    }
    pids
}

fn attribute(element: AXUIElementRef, name: &str) -> Option<CFType> {
    let mut value: CFTypeRef = std::ptr::null();
    let error = unsafe { AXUIElementCopyAttributeValue(element, CFString::new(name).as_concrete_TypeRef(), &mut value) };
    (error == 0 && !value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
}

fn flag(element: AXUIElementRef, name: &str) -> bool {
    attribute(element, name).and_then(|v| v.downcast::<CFBoolean>()).is_some_and(bool::from)
}

fn frame(window: AXUIElementRef) -> Option<CGRect> {
    let mut origin = CGPoint::new(0.0, 0.0);
    let mut size = CGSize::new(0.0, 0.0);
    let (position, extent) = (attribute(window, "AXPosition")?, attribute(window, "AXSize")?);
    let read = unsafe {
        AXValueGetValue(position.as_CFTypeRef(), AX_VALUE_POINT, &mut origin as *mut _ as *mut c_void) != 0
            && AXValueGetValue(extent.as_CFTypeRef(), AX_VALUE_SIZE, &mut size as *mut _ as *mut c_void) != 0
    };
    read.then(|| CGRect::new(&origin, &size))
}

fn set(window: AXUIElementRef, name: &str, kind: u32, value: *const c_void) {
    let value = unsafe { AXValueCreate(kind, value) };
    if value.is_null() {
        return;
    }
    let value = unsafe { CFType::wrap_under_create_rule(value) };
    unsafe { AXUIElementSetAttributeValue(window, CFString::new(name).as_concrete_TypeRef(), value.as_CFTypeRef()) };
}

/// Moves each of the app's ordinary windows that overlaps the panel to start at the panel's inner
/// edge, keeping its far edge where it was while that leaves it wide enough.
fn clear_app(pid: i32, panel: &CGRect, side: Side) {
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return;
    }
    let app = unsafe { CFType::wrap_under_create_rule(app) };
    unsafe { AXUIElementSetMessagingTimeout(app.as_CFTypeRef(), APP_TIMEOUT_SECS) };
    let Some(windows) = attribute(app.as_CFTypeRef(), "AXWindows") else { return };
    let windows: CFArray = unsafe { CFArray::wrap_under_get_rule(windows.as_CFTypeRef() as CFArrayRef) };
    for window in windows.iter() {
        let window: AXUIElementRef = *window;
        let standard = attribute(window, "AXSubrole")
            .and_then(|s| s.downcast::<CFString>())
            .is_some_and(|s| s == "AXStandardWindow");
        if !standard || flag(window, "AXFullScreen") || flag(window, "AXMinimized") {
            continue;
        }
        let Some(current) = frame(window).filter(|f| overlaps(f, panel)) else { continue };
        let far = match side {
            Side::Left => current.origin.x + current.size.width,
            Side::Right => current.origin.x,
        };
        let inner = match side {
            Side::Left => panel.origin.x + panel.size.width,
            Side::Right => panel.origin.x,
        };
        let width = if (far - inner).abs() >= MIN_WIDTH { (far - inner).abs() } else { current.size.width };
        let origin = CGPoint::new(if side == Side::Left { inner } else { inner - width }, current.origin.y);
        let size = CGSize::new(width, current.size.height);
        set(window, "AXPosition", AX_VALUE_POINT, &origin as *const _ as *const c_void);
        set(window, "AXSize", AX_VALUE_SIZE, &size as *const _ as *const c_void);
    }
}
