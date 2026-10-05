//! Keeps other apps' windows out from under the panel. X11 lets a window reserve a strip along
//! an edge of the screen, as the desktop's own panels and docks do, so the window manager keeps
//! the others clear and no permission is asked for.

use crate::window::{self, Side};

/// Nothing to allow on Linux: reserving the strip is the panel's own business.
pub fn trusted() -> bool {
    true
}

pub fn request() {}

/// Where the panel is, `None` while windows should be left alone.
pub fn set_panel(panel: Option<((f64, f64, f64, f64), Side)>) {
    window::reserve(panel);
}

pub fn spawn() {}
