//! Where the panel sits on screen. Each platform places the window with its own window system.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;

/// The edge of the display the panel sticks to.
#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Left,
    Right,
}
