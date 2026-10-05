//! "Open at login": a per-user LaunchAgent on macOS, an autostart entry on Linux. On by default;
//! turning it off leaves a marker so the default doesn't re-enable it on the next launch.

use crate::settings;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
const LABEL: &str = "dev.f3r.usage-flow";
/// The agent the app installed when it was called claude-usage.
#[cfg(target_os = "macos")]
const LEGACY_LABEL: &str = "com.claude-usage.app";

#[cfg(target_os = "macos")]
fn plist_path(label: &str) -> PathBuf {
    settings::home().join(format!("Library/LaunchAgents/{label}.plist"))
}

#[cfg(target_os = "macos")]
fn path() -> PathBuf {
    plist_path(LABEL)
}

/// `$XDG_CONFIG_HOME/autostart`, where the desktop looks for what to open with the session.
#[cfg(target_os = "linux")]
fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| settings::home().join(".config"))
        .join("autostart/usage-flow.desktop")
}

pub fn enabled() -> bool {
    !settings::flag(settings::LOGIN_OFF)
}

/// Writes (or refreshes) the login item so it points at the running binary.
pub fn sync() {
    if !enabled() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    // A build run from the source tree never claims the login item.
    if exe.components().any(|c| c.as_os_str() == "target") {
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = std::fs::remove_file(plist_path(LEGACY_LABEL));
    let entry = entry(&exe.to_string_lossy());
    let path = path();
    if std::fs::read_to_string(&path).ok().as_deref() == Some(entry.as_str()) {
        return;
    }
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(path, entry);
}

#[cfg(target_os = "linux")]
fn entry(exe: &str) -> String {
    let exe = exe.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "[Desktop Entry]\nType=Application\nName=Usage Flow\nComment=Claude Code and Codex limits at a glance\nExec=\"{exe}\"\nIcon=usage-flow\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(target_os = "macos")]
fn entry(exe: &str) -> String {
    let exe = exe.replace('&', "&amp;").replace('<', "&lt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    )
}

pub fn set_enabled(on: bool) {
    settings::set_flag(settings::LOGIN_OFF, !on);
    if on {
        sync();
    } else {
        let _ = std::fs::remove_file(path());
    }
}
