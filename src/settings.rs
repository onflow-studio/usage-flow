//! Settings are marker files in the data folder: there or not, with no format to migrate.

use std::path::PathBuf;

pub const PINNED: &str = "pinned";
pub const ALWAYS_ON_TOP_OFF: &str = "always-on-top-disabled";
pub const LOGIN_OFF: &str = "login-disabled";
pub const MENU_BAR_USAGE_OFF: &str = "menu-bar-usage-hidden";
pub const ALERTS_OFF: &str = "alerts-disabled";

fn app_support() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Library/Application Support")
}

pub fn dir() -> PathBuf {
    app_support().join("Usage Flow")
}

/// The app was called claude-usage, and its data folder carries the name: bring settings and the
/// last reading across once.
pub fn migrate() {
    let (dir, legacy) = (dir(), app_support().join("claude-usage"));
    if dir.exists() || !legacy.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    for entry in std::fs::read_dir(legacy).into_iter().flatten().flatten() {
        let _ = std::fs::copy(entry.path(), dir.join(entry.file_name()));
    }
}

pub fn flag(name: &str) -> bool {
    dir().join(name).exists()
}

pub fn set_flag(name: &str, on: bool) {
    let path = dir().join(name);
    if on {
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(path, "");
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// Name of the display the panel was last on.
pub fn display() -> Option<String> {
    std::fs::read_to_string(dir().join("display")).ok()
}

pub fn save_display(name: &str) {
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(dir().join("display"), name);
}
